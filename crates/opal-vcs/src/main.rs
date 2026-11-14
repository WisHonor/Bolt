//! The `opalro` command-line tool.

mod git_words;
mod goto_output;
mod help;
mod history_output;
mod import_output;
mod output;
mod progress;
mod setup_output;
mod shelf_output;
mod show_output;
mod style;
mod undo_output;

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::ffi::OsString;
use std::io::{BufRead, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{CommandFactory, Parser, Subcommand};
use git_words::Typed;
use opal_core::changes::{Lines, save_changes, unsaved_changes};
use opal_core::config::{
    ConfigError, Identity, git_excludes_file, git_import_settings, ignored_identity_note,
    ignored_project_identity, load_identity, now_person, undo_window_days,
};
use opal_core::diary::{Action, DiaryEntry, Position};
use opal_core::diff::{count_changes, for_each_change};
use opal_core::fsio::fault_point;
use opal_core::history::{
    LineIds, ResolveError, current_save, for_each_on_line, resolve, short_change_id,
};
use opal_core::id::{ChangeId, Id};
use opal_core::import::{ImportError, ImportPass, ImportRequest, import_git};
use opal_core::lines::resolve_target;
use opal_core::names::{GivenPathError, project_given, project_path};
use opal_core::presets::{Looked, Preset, suggested};
use opal_core::repo::{RepoError, Repository, elevated_only_note, resolved};
use opal_core::save::Save;
use opal_core::setup::{SetupError, apply_setup, look_at, plan_setup};
use opal_core::shelf::{self, Shelved};
use opal_core::snapshot::{LargeSave, SaveError, SaveOutcome, SaveRequest, save_folder};
use opal_core::status::{PathChange, status, why_stopped};
use opal_core::store::StoreError;
use opal_core::switch::{SwitchError, discard, goto};
use opal_core::text::escape;
use opal_core::tidy;
use opal_core::undo::{self, StepInfo, StepRequest};
use opal_core::verify;
use opal_core::warning::{self, Warning};
use opal_core::writeout::unfinished_warning;
use output::{Failure, OPALRO_JSON};
use serde_json::json;

#[derive(Parser)]
#[command(
    name = "opalro",
    version,
    about = "Version control for code and big files."
)]
struct Cli {
    /// Print machine-readable JSON instead of text.
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Make this folder an Opalro project.
    Start {
        /// What kind of project: code, game, or custom (Opalro's plain defaults, no rule files).
        #[arg(long, value_enum, value_name = "PRESET")]
        preset: Option<PresetArg>,
    },
    /// Show the project's rules as Opalro reads them, or change them to a preset's.
    Setup {
        /// Show the lines this preset would change, then change them.
        #[arg(long, value_enum, value_name = "PRESET")]
        preset: Option<PresetArg>,
        /// Make the changes without asking.
        #[arg(long, requires = "preset")]
        yes: bool,
    },
    /// Bring the Git repository in this folder into Opalro, with all its history.
    Import,
    /// Save the current state of your files: all of them, or only these files, folders and
    /// patterns (such as "*.png" or "art/**/*.png").
    Save(SaveArgs),
    /// Show what changed since your last save.
    Status {
        /// Read every file, not only those that look changed.
        #[arg(long)]
        full: bool,
    },
    /// Show your unsaved changes line by line, in these files and folders or everywhere; or what
    /// a save changed.
    Changes {
        /// The save to show, by line name, change ID or fingerprint, against its first parent.
        #[arg(long, value_name = "SAVE")]
        save: Option<String>,
        /// Only these files and folders.
        #[arg(value_name = "PATH")]
        paths: Vec<PathBuf>,
    },
    /// List the saves on this line.
    History,
    /// Show one save.
    Show {
        /// The save, by line name, change ID or fingerprint; the one you're at if left out.
        save: Option<String>,
    },
    /// List the lines, marking the one you're on, then the server lines.
    Line,
    /// Go to a line, or to a save: by line name, line@server, change ID or fingerprint.
    Goto {
        /// The line or save to go to.
        target: String,
    },
    /// Throw away your unsaved changes: in these files and folders, or everywhere. A safety copy
    /// of each changed or new file is kept.
    Discard {
        /// Only in these files and folders.
        #[arg(value_name = "PATH")]
        paths: Vec<PathBuf>,
    },
    /// Take back the last step, or the last N, one at a time: the lines, where you are and what
    /// the step did to your files go back as they were.
    Undo {
        #[command(flatten)]
        steps: Steps,
        /// List the steps redo can bring back and undo can take, numbered for N.
        #[arg(long, conflicts_with_all = ["n", "shelve", "to"])]
        list: bool,
        /// With --list: every entry of the diary, undos and redos included.
        #[arg(long, requires = "list")]
        all: bool,
        /// Make things as they were right after this diary entry, named by at least 8 hex digits
        /// of its ID, as opalro undo --list shows them.
        #[arg(long, value_name = "ENTRY", conflicts_with = "n")]
        to: Option<String>,
    },
    /// Bring back the step undone last, or the last N.
    Redo {
        #[command(flatten)]
        steps: Steps,
    },
    /// Set your unsaved changes aside on a new shelf, and put your files back as your last save
    /// has them.
    Shelf {
        /// What the shelf holds, in a few words.
        message: Option<String>,
        /// List the shelves, newest first.
        #[arg(long, conflicts_with = "message")]
        list: bool,
    },
    /// Bring back the newest shelf, or the one with this number, onto the save you're on.
    Unshelf {
        /// The shelf's number, as opalro shelf --list shows it.
        #[arg(value_name = "NUMBER")]
        number: Option<String>,
    },
    /// Free the space nothing needs any more: saves no line, shelf or tag reaches, and what only
    /// steps older than the undo window (undo-window-days in your settings, 30 by default) needed.
    Tidy,
    /// Check everything Opalro has stored, and that the project has everything it needs. Changes
    /// nothing.
    Verify,
}

#[derive(Clone, Copy, clap::ValueEnum)]
enum PresetArg {
    Code,
    Game,
    Custom,
}

impl From<PresetArg> for Preset {
    fn from(p: PresetArg) -> Preset {
        match p {
            PresetArg::Code => Preset::Code,
            PresetArg::Game => Preset::Game,
            PresetArg::Custom => Preset::Custom,
        }
    }
}

#[derive(clap::Args)]
struct SaveArgs {
    /// What the save holds, in a few words.
    message: Option<String>,
    /// Save only these files, folders and patterns.
    #[arg(value_name = "PATH")]
    paths: Vec<PathBuf>,
    /// After an import, save files Git wrote with CRLF line endings as they are.
    #[arg(long)]
    accept_crlf: bool,
    /// The name of a new line this save starts.
    #[arg(long, value_name = "NAME")]
    line: Option<String>,
    /// Save without asking, however much the save adds.
    #[arg(long)]
    yes: bool,
    /// Replace your line's newest save with one holding your changes too, keeping its change ID;
    /// leave out the message, or give "", to keep its message.
    #[arg(long, conflicts_with = "line")]
    amend: bool,
}

#[derive(clap::Args)]
struct Steps {
    /// How many steps.
    #[arg(value_name = "N", default_value_t = 1, value_parser = clap::value_parser!(u64).range(1..))]
    n: u64,
    /// Set your unsaved changes in the way aside on a new shelf, as part of the same step.
    #[arg(long)]
    shelve: bool,
}

impl From<RepoError> for Failure {
    fn from(e: RepoError) -> Failure {
        Failure {
            hint: e.hint(),
            message: e.to_string(),
            printed: false,
        }
    }
}

impl From<ConfigError> for Failure {
    fn from(e: ConfigError) -> Failure {
        Failure {
            hint: e.hint(),
            message: e.to_string(),
            printed: false,
        }
    }
}

impl From<StoreError> for Failure {
    fn from(e: StoreError) -> Failure {
        RepoError::from(e).into()
    }
}

impl From<SaveError> for Failure {
    fn from(e: SaveError) -> Failure {
        Failure {
            hint: e.hint(),
            message: e.to_string(),
            printed: false,
        }
    }
}

impl From<ResolveError> for Failure {
    fn from(e: ResolveError) -> Failure {
        Failure {
            hint: e.hint(),
            message: e.to_string(),
            printed: false,
        }
    }
}

impl From<SwitchError> for Failure {
    fn from(e: SwitchError) -> Failure {
        Failure {
            hint: e.hint(),
            message: e.to_string(),
            printed: false,
        }
    }
}

impl From<GivenPathError> for Failure {
    fn from(e: GivenPathError) -> Failure {
        Failure {
            hint: Some(
                "Name files and folders inside the project, from the folder you are in.".into(),
            ),
            message: e.to_string(),
            printed: false,
        }
    }
}

impl From<std::io::Error> for Failure {
    fn from(e: std::io::Error) -> Failure {
        RepoError::from(e).into()
    }
}

impl From<SetupError> for Failure {
    fn from(e: SetupError) -> Failure {
        Failure {
            hint: e.hint(),
            message: e.to_string(),
            printed: false,
        }
    }
}

impl From<ImportError> for Failure {
    fn from(e: ImportError) -> Failure {
        Failure {
            hint: e.hint(),
            message: e.to_string(),
            printed: false,
        }
    }
}

/// The first save of the project `start` has just made, read back: its ID, the save, and its
/// change ID at its shortest.
fn read_first_save(repo: &Repository) -> Result<(Id, Save, String), Failure> {
    fault_point("start:read-back")?;
    let (_, entry) = repo.current()?;
    let id = *entry.state.lines.get("main").ok_or_else(|| Failure {
        message: "The new project has no line main.".into(),
        hint: None,
        printed: false,
    })?;
    let save = repo.store().save(&id, repo.signatures_allowed())?;
    let short = short_change_id(repo, &id, &save.change_id)?;
    Ok((id, save, short))
}

/// `repo`, sealing its caches with the user's own key. A key Opalro can't use is a warning, and
/// the caches go unused this time: they only make Opalro faster.
fn with_users_key(mut repo: Repository) -> Repository {
    match opal_core::keyed::cache_key() {
        Ok(Some(key)) => repo.set_cache_key(key),
        Ok(None) => {}
        Err(e) => repo.store().note(Warning::new(
            warning::CACHE_KEY_UNUSABLE,
            format!(
                "{e} Opalro worked without its caches this time. {}",
                e.hint().unwrap_or_default()
            ),
        )),
    }
    repo
}

/// The preset `opalro start` begins with: `None` where the folder has an `opalro.toml` of its own,
/// which stays. Asks when it may and `asked` is `None`; stops before anything is made otherwise.
fn preset_to_start(
    json: bool,
    asked: Option<Preset>,
    looked: &Looked,
    has_rules_file: bool,
) -> Result<Option<Preset>, Failure> {
    if has_rules_file {
        return match asked {
            None => Ok(None),
            Some(preset) => Err(Failure {
                message:
                    "Nothing was started: this folder already has an opalro.toml, and a start \
                          keeps the rules it holds, so --preset can't be used here."
                        .into(),
                hint: Some(format!(
                    "Run opalro start without --preset to keep them; to switch to the preset's \
                     rules afterwards, run: opalro setup --preset {}",
                    preset.name()
                )),
                printed: false,
            }),
        };
    }
    if let Some(preset) = asked {
        return Ok(Some(preset));
    }
    let suggestion = suggested(&looked.found);
    let stop = |message: &str| Failure {
        message: message.to_owned(),
        hint: Some(format!(
            "Name the kind of project: opalro start --preset code, opalro start --preset game or opalro \
             start --preset custom. Opalro would suggest --preset {}: it {}.",
            suggestion.name(),
            setup_output::found_words(looked)
        )),
        printed: false,
    };
    if !style::may_ask() {
        let why = if json {
            "Nothing was started: Opalro can't ask what kind of project this is with --json."
        } else {
            "Nothing was started: Opalro can't ask what kind of project this is without a terminal."
        };
        return Err(stop(why));
    }
    match ask_preset(looked, suggestion) {
        Some(preset) => Ok(Some(preset)),
        None => Err(stop("Nothing was started: no kind of project was chosen.")),
    }
}

/// Asks in the terminal which preset to start with, at most three times; `None` if no answer
/// named one, or the input ended.
fn ask_preset(looked: &Looked, suggestion: Preset) -> Option<Preset> {
    const CHOICES: [(Preset, &str); 3] = [
        (Preset::Code, "apps, websites, libraries"),
        (Preset::Game, "Unity, Unreal, Godot, big assets"),
        (Preset::Custom, "Opalro's plain defaults, no rule files"),
    ];
    let e = style::err();
    output::tell(&setup_output::look_line(looked));
    output::tell("");
    output::tell("What kind of project is this?");
    output::tell("");
    let mut default = 1;
    for (n, (preset, gloss)) in (1..).zip(CHOICES) {
        let mut line = format!("    {n}. {:<8} {gloss}", setup_output::title(preset));
        if preset == suggestion {
            default = n;
            let mark = format!("{} suggested", e.sym(style::Sym::Back));
            line += &format!("   {}", e.paint(style::Role::Dim, &mark));
        }
        output::tell(&line);
    }
    output::tell("");
    for _ in 0..3 {
        let _ = write!(std::io::stderr(), "Choice [{default}]: ");
        let mut answer = String::new();
        if !matches!(std::io::stdin().lock().read_line(&mut answer), Ok(1..)) {
            return None;
        }
        let answer = answer.trim().to_ascii_lowercase();
        let picked = match answer.as_str() {
            "" => Some(suggestion),
            "1" => Some(Preset::Code),
            "2" => Some(Preset::Game),
            "3" => Some(Preset::Custom),
            name => Preset::from_name(name),
        };
        if picked.is_some() {
            output::tell("");
            return picked;
        }
        output::tell("Answer 1, 2 or 3, or a preset's name: code, game or custom.");
    }
    None
}

fn start(
    json: bool,
    asked: Option<Preset>,
    opened: &mut Option<Repository>,
) -> Result<(), Failure> {
    let root = resolved(&std::env::current_dir()?);
    // Before anything is asked: a folder that can't be a project is refused at once.
    Repository::check_new(&root)?;
    // The user's own settings may name this folder, so the first save is theirs too. Read before
    // the question, so nobody answers it only to be refused.
    let who = now_person(&load_identity(Some(&root))?);
    let (looked, has_rules_file) = look_at(&root)?;
    let preset = preset_to_start(json, asked, &looked, has_rules_file)?;
    let repo = opened.insert(with_users_key(Repository::init(&root, &who)?));
    let mut warnings: Vec<Warning> = elevated_only_note(&root)
        .map(|note| Warning::new(warning::ELEVATED_ONLY, note))
        .into_iter()
        .collect();
    // The project exists now, so a save that can't be read back is a warning, not a failure.
    let first = match read_first_save(repo) {
        Ok(first) => Some(first),
        Err(problem) => {
            warnings.push(Warning::new(
                warning::FIRST_SAVE_NOT_READ,
                format!(
                    "The project was made, but Opalro couldn't read its first save back. {}\n\
                     Run `opalro history` to check that the project reads.",
                    problem.message
                ),
            ));
            None
        }
    };
    // The rule files are a second step, which `opalro undo` takes back.
    let mut rules_written = Vec::new();
    let mut unwritten = None;
    if let Some(preset) = preset.filter(|p| *p != Preset::Custom) {
        let wrote = git_excludes_file(&root)
            .map_err(Failure::from)
            .and_then(|excludes| {
                let plan = plan_setup(&root, preset)?;
                let _working = progress::Working::start("Writing rule files");
                Ok(apply_setup(
                    repo,
                    &plan,
                    &who,
                    excludes.as_deref(),
                    &mut waiting,
                )?)
            });
        match wrote {
            Ok(done) => {
                rules_written = done.written;
                warnings.extend(done.warnings);
            }
            Err(problem) => {
                unwritten = Some(Failure {
                    message: format!(
                        "The project was started, but its rule files weren't written: {}",
                        problem.message
                    ),
                    hint: Some(format!(
                        "Run opalro setup --preset {} to write them.",
                        preset.name()
                    )),
                    printed: false,
                });
            }
        }
    }
    let rules = repo.read_rules();
    warnings.extend(rules.warnings());
    warnings.extend(repo.store().warnings());
    if json {
        let save = first.as_ref().map_or(json!(null), |(id, save, short)| {
            output::save_json(id, save, short)
        });
        let document = json!({
            "opalro_json": OPALRO_JSON,
            "root": root.display().to_string(),
            "position": output::position_json(&Position::OnLine("main".into())),
            "save": save,
            "preset": preset.map(Preset::name),
            "found": setup_output::found_json(&looked.found),
            "rules_written": rules_written,
            "warnings": output::warnings_json(&warnings),
        });
        let Some(failure) = unwritten else {
            return output::print_done(&format!("{document}\n"), &warnings);
        };
        let document = output::failed_document(document, &failure, &warnings);
        output::print_out(&format!("{document}\n"));
        let in_document = Failure {
            printed: true,
            ..failure
        };
        return output::ended(Some(in_document), String::new, &warnings, None);
    }
    if let Some(failure) = unwritten {
        // The warnings it found, then the failure.
        return output::ended(Some(failure), String::new, &warnings, None);
    }
    // The folder's name came from outside Opalro, like any name in a project.
    let shown = escape(&root.display().to_string(), false);
    let short = first.as_ref().map(|(_, _, short)| short.as_str());
    let text = setup_output::started(&shown, preset, &looked.found, &rules.rules, short);
    output::print_done(&text, &warnings)?;
    if short.is_some() {
        let e = style::err();
        style::next(&format!(
            "change some files, then run: {}",
            e.command("opalro save \"describe your change\"")
        ));
    }
    Ok(())
}

fn setup_command(
    json: bool,
    preset: Option<Preset>,
    yes: bool,
    opened: &mut Option<Repository>,
) -> Result<(), Failure> {
    let repo = opened.insert(with_users_key(Repository::discover(
        &std::env::current_dir()?,
    )?));
    let Some(preset) = preset else {
        let read = repo.read_rules();
        let block = opal_core::setup::opalroignore_block_lines(repo.root())?;
        let mut warnings = read.warnings();
        warnings.extend(repo.store().warnings());
        if json {
            let mut document = setup_output::rules_json(&read, block);
            document["opalro_json"] = OPALRO_JSON.into();
            document["warnings"] = output::warnings_json(&warnings);
            return output::print_quietly(&format!("{document}\n"), &warnings);
        }
        output::print_quietly(&setup_output::rules_shown(&read, block), &warnings)?;
        let e = style::err();
        style::next(&format!(
            "edit opalro.toml or .opalroignore to change a rule, or switch presets with: {}",
            e.command("opalro setup --preset <name>")
        ));
        return Ok(());
    };
    let plan = plan_setup(repo.root(), preset)?;
    let title = setup_output::title(preset);
    let mut document = json!({
        "opalro_json": OPALRO_JSON,
        "preset": preset.name(),
        "found": setup_output::found_json(&plan.found),
        "files": setup_output::changes_json(&plan.changes),
        "applied": false,
    });
    if plan.changes.is_empty() {
        let warnings = repo.store().warnings();
        let text = if json {
            document["warnings"] = output::warnings_json(&warnings);
            format!("{document}\n")
        } else {
            format!(
                "Nothing to change: the rule files already are what the {title} preset writes.\n"
            )
        };
        return output::print_quietly(&text, &warnings);
    }
    if !json {
        let mut preview =
            format!("Switching to {title} settings changes these files:\n\n").into_bytes();
        setup_output::write_preview(&mut preview, &plan.changes)?;
        output::print_out(&String::from_utf8_lossy(&preview));
    }
    if !yes && style::may_ask() && !ask_apply() {
        return output::print_quietly("\nNothing was changed.\n", &repo.store().warnings());
    }
    if !yes && !style::may_ask() {
        let failure = Failure {
            message: if json {
                "Nothing was changed: Opalro can't ask whether to apply these changes with --json."
                    .into()
            } else {
                "Nothing was changed: Opalro can't ask whether to apply these changes without a \
                 terminal."
                    .into()
            },
            hint: Some(format!(
                "If they are what you want, run: opalro setup --preset {} --yes",
                preset.name()
            )),
            printed: false,
        };
        if !json {
            return Err(failure);
        }
        let warnings = repo.store().warnings();
        let document = output::failed_document(document, &failure, &warnings);
        output::print_out(&format!("{document}\n"));
        let in_document = Failure {
            printed: true,
            ..failure
        };
        return output::ended(Some(in_document), String::new, &warnings, None);
    }
    let who = now_person(&load_identity(Some(repo.root()))?);
    let git_excludes = git_excludes_file(repo.root())?;
    let done = {
        let _working = progress::Working::start("Writing rule files");
        apply_setup(repo, &plan, &who, git_excludes.as_deref(), &mut waiting)
    }?;
    let mut warnings = done.warnings;
    warnings.extend(repo.store().warnings());
    // Every file changed while Opalro worked: nothing was written.
    let applied = !done.written.is_empty();
    let text = if json {
        document["applied"] = applied.into();
        document["warnings"] = output::warnings_json(&warnings);
        format!("{document}\n")
    } else {
        // After the preview.
        setup_output::switched(preset, &plan.changes, &done.written)
    };
    output::print_done(&text, &warnings)?;
    if applied {
        style::changed_your_mind();
    }
    Ok(())
}

/// Asks in the terminal whether to apply a preset's changes. Only a yes is one.
fn ask_apply() -> bool {
    let _ = write!(std::io::stderr(), "Apply these changes? [y/N] ");
    let mut answer = String::new();
    if std::io::stdin().lock().read_line(&mut answer).is_err() {
        return false;
    }
    let answer = answer.trim();
    answer.eq_ignore_ascii_case("y") || answer.eq_ignore_ascii_case("yes")
}

fn import(json: bool, opened: &mut Option<Repository>) -> Result<(), Failure> {
    let root = resolved(&std::env::current_dir()?);
    let who = now_person(&load_identity(Some(&root))?);
    let git_settings = git_import_settings(&root)?;
    let mut bar = progress::Bar::new("Reading Git's history");
    let imported = import_git(
        &root,
        &mut ImportRequest {
            who: &who,
            command: "import",
            git_settings: Some(&git_settings),
            progress: &mut |pass, done, total| {
                bar.set_label(match pass {
                    ImportPass::ReadingHistory => "Reading Git's history",
                    ImportPass::CheckingLfs => "Checking Git LFS files",
                    ImportPass::Importing => "Importing commits",
                });
                bar.update(done, total, false);
            },
        },
        opened,
    );
    bar.finish();
    let report = imported?;
    // A project made from an import opens only where one made by `start` would.
    let mut warnings: Vec<Warning> = elevated_only_note(&root)
        .map(|note| Warning::new(warning::ELEVATED_ONLY, note))
        .into_iter()
        .collect();
    warnings.extend(report.warnings.iter().cloned());
    let text = if json {
        format!("{}\n", import_output::import_json(&report, &warnings))
    } else {
        let s = style::out();
        let standing = match &report.position {
            Position::OnLine(line) => {
                format!(
                    "You're on line {}.",
                    s.paint(style::Role::Line, &escape(line, false))
                )
            }
            Position::AtSave(id) => {
                // By change ID, as everywhere else; by fingerprint if the save can't be read now.
                let short = opened.as_ref().and_then(|repo| {
                    let save = repo.store().save(id, repo.signatures_allowed()).ok()?;
                    Some(short_or_whole(repo, id, &save.change_id, &mut warnings))
                });
                let short = match short {
                    Some(short) => s.paint(style::Role::ChangeId, &short),
                    None => s.paint(style::Role::Fingerprint, &id.to_hex()),
                };
                format!(
                    "You're at save {short}, where Git's HEAD was detached: not on a line. Saving \
                     here asks for a new line's name."
                )
            }
        };
        import_output::import_text(&report, &standing)
    };
    output::print_done(&text, &warnings)?;
    style::next(&format!(
        "look around with: {}",
        style::err().command("opalro status")
    ));
    Ok(())
}

/// Asks in the terminal for the new line's name, with `working` off the screen meanwhile; `None`
/// if there is no terminal to ask in.
fn ask_line_name(working: &progress::Working) -> Result<Option<String>, Failure> {
    if !style::may_ask() {
        return Ok(None);
    }
    working.pause();
    let _ = write!(std::io::stderr(), "Name a new line for this save: ");
    let mut answer = String::new();
    let read = std::io::stdin().lock().read_line(&mut answer);
    working.resume();
    read?;
    let answer = answer.trim();
    if answer.is_empty() {
        return Err(Failure {
            message: "Nothing was saved: no name was given for the new line.".into(),
            hint: None,
            printed: false,
        });
    }
    Ok(Some(answer.to_owned()))
}

/// Asks in the terminal whether to make a save that adds this much; `None` if there is no
/// terminal to ask in. An answer that can't be read is a no.
fn ask_large(large: &LargeSave) -> Option<bool> {
    if !style::may_ask() {
        return None;
    }
    let _ = write!(std::io::stderr(), "{large} Save them? [y/N] ");
    let mut answer = String::new();
    if std::io::stdin().lock().read_line(&mut answer).is_err() {
        return Some(false);
    }
    let answer = answer.trim();
    Some(answer.eq_ignore_ascii_case("y") || answer.eq_ignore_ascii_case("yes"))
}

fn save(json: bool, args: SaveArgs, opened: &mut Option<Repository>) -> Result<(), Failure> {
    let SaveArgs {
        message,
        paths: given,
        accept_crlf,
        line,
        yes,
        amend,
    } = args;
    // A blank message is none: a save needs one, and an amend then keeps the one it has.
    let message = message.filter(|m| !m.trim().is_empty());
    // Before the project is looked for, so it is said first.
    if message.is_none() && !amend {
        return Err(SaveError::NeedsMessage.into());
    }
    let here = resolved(&std::env::current_dir()?);
    let repo = opened.insert(with_users_key(Repository::discover(&here)?));
    let only = if given.is_empty() {
        None
    } else {
        let drive = repo.drive()?;
        let only = given
            .iter()
            .map(|p| project_given(repo.root(), &here, p, drive))
            .collect::<Result<Vec<_>, _>>()?;
        Some(only)
    };
    let paths: Vec<String> = only.iter().flatten().map(|g| g.shown()).collect();
    let who = now_person(&load_identity(Some(repo.root()))?);
    // A project's own settings never say who the user is; if they try, the user is told.
    let ignored = ignored_project_identity(repo);
    let git_excludes = git_excludes_file(repo.root())?;
    let mut req = SaveRequest {
        message: message.as_deref(),
        who: &who,
        command: "save",
        accept_crlf,
        git_excludes: git_excludes.as_deref(),
        new_line: line.as_deref(),
        only: only.as_deref(),
        amend,
    };
    let mut on_wait = waiting;
    let working = progress::Working::start("Saving");
    // With --json nothing is asked, so a large save stops and names --yes.
    let mut said_no = false;
    let mut confirm = |large: &LargeSave| {
        if yes || json {
            return yes;
        }
        working.pause();
        let answer = ask_large(large);
        working.resume();
        said_no = answer == Some(false);
        answer.unwrap_or(false)
    };
    let asked;
    let saved = match save_folder(repo, &req, &mut on_wait, &mut confirm) {
        // Only now is it known that the save is at a save: a stopped switch may have moved it.
        Err(SaveError::NeedsLineName) if !json && let Some(name) = ask_line_name(&working)? => {
            asked = name;
            req.new_line = Some(&asked);
            save_folder(repo, &req, &mut on_wait, &mut confirm)
        }
        done => done,
    };
    drop(working);
    let outcome = match saved {
        Err(SaveError::LargeSave(_)) if said_no => {
            return Err(Failure {
                message: "Nothing was saved: you answered no.".into(),
                hint: None,
                printed: false,
            });
        }
        done => done?,
    };
    let (mut text, warnings) = match outcome {
        SaveOutcome::Saved {
            save_id,
            save,
            changed_files,
            short_change_id: short,
            mut warnings,
            replaced,
        } => {
            // The save is made, so nothing from here on may fail the command.
            warnings.extend(repo.store().warnings());
            let text = if json {
                let document = json!({
                    "opalro_json": OPALRO_JSON,
                    "result": "saved",
                    "paths": paths,
                    "save": output::save_json(&save_id, &save, &short),
                    "changed_files": changed_files,
                    "replaced": replaced.map(|id| id.to_hex()),
                    "warnings": output::warnings_json(&warnings),
                });
                format!("{}\n", with_ignored(document, ignored.as_ref()))
            } else {
                let s = style::out();
                let counted = match changed_files {
                    Some(n) => format!("({})", output::files(n)),
                    None => "(couldn't count the changed files)".to_owned(),
                };
                let made = match replaced {
                    Some(_) => format!("{} save", s.paint(style::Role::Ok, "Updated")),
                    None => s.paint(style::Role::Ok, "Saved"),
                };
                let on = save.line().map_or(String::new(), |line| {
                    format!(" on {}", s.paint(style::Role::Line, &escape(line, false)))
                });
                let mut text = format!(
                    "{made} {} {}{on}  {}\n",
                    s.paint(style::Role::ChangeId, &short),
                    output::quoted_title(&save),
                    s.paint(style::Role::Dim, &counted)
                );
                let mut details = Vec::new();
                if replaced.is_some() {
                    details.push("The ID stays the same.".to_owned());
                }
                if let Some(line) = req.new_line {
                    let line = style::whole(&escape(line, false));
                    details.push(format!("You're on the new line {line}."));
                }
                if only.is_some() {
                    details.push(format!(
                        "Only what you named was saved: {} lists the changes left out.",
                        s.command("opalro status")
                    ));
                }
                if !details.is_empty() {
                    text += "\n";
                }
                for detail in details {
                    text += &s.detail(None, &detail);
                    text.push('\n');
                }
                text
            };
            (text, warnings)
        }
        SaveOutcome::NothingChanged {
            current,
            mut warnings,
        } => {
            let text = if json {
                warnings.extend(repo.store().warnings());
                let document = json!({
                    "opalro_json": OPALRO_JSON,
                    "result": "nothing_changed",
                    "paths": paths,
                    "warnings": output::warnings_json(&warnings),
                });
                format!("{}\n", with_ignored(document, ignored.as_ref()))
            } else {
                let last = repo.store().save(&current, repo.signatures_allowed())?;
                let short = short_or_whole(repo, &current, &last.change_id, &mut warnings);
                warnings.extend(repo.store().warnings());
                let s = style::out();
                let last_save = format!(
                    "your last save, {} {}",
                    s.paint(style::Role::ChangeId, &short),
                    output::quoted_title(&last)
                );
                let mut text = if amend {
                    let what = if paths.is_empty() {
                        "your files match"
                    } else {
                        "what you named matches"
                    };
                    format!("Nothing to amend: {what} {last_save}, and so does the message.\n")
                } else if paths.is_empty() {
                    format!("Nothing to save: no changes since {last_save}.\n")
                } else {
                    format!("Nothing to save there: it matches {last_save}.\n")
                };
                if req.new_line.is_some() {
                    text += "No line was made: opalro line new <name> makes one without saving.\n";
                }
                text
            };
            (text, warnings)
        }
    };
    if !json && let Some(ignored) = &ignored {
        text += &format!("{}\n", ignored_identity_note(ignored, repo.root()));
    }
    output::print_done(&text, &warnings)
}

/// `change_id` at its shortest or, if the line can't be read to work that out, whole, with a
/// warning: a damaged older save mustn't fail a command that otherwise worked.
fn short_or_whole(
    repo: &Repository,
    id: &Id,
    change_id: &ChangeId,
    warnings: &mut Vec<Warning>,
) -> String {
    short_change_id(repo, id, change_id).unwrap_or_else(|problem| {
        warnings.push(not_shortened(&problem));
        change_id.to_letters()
    })
}

/// Says the line couldn't be read to shorten change IDs, so they are given whole.
fn not_shortened(problem: &RepoError) -> Warning {
    Warning::new(
        warning::CHANGE_ID_NOT_SHORTENED,
        format!("Opalro couldn't read this line's older saves to shorten the change ID. {problem}"),
    )
}

/// `document`, plus the identity a project's settings named and Opalro ignored, if there was one.
fn with_ignored(mut document: serde_json::Value, ignored: Option<&Identity>) -> serde_json::Value {
    if let Some(identity) = ignored {
        document["ignored_project_identity"] =
            json!({"name": identity.name, "email": identity.email});
    }
    document
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_millis()).ok())
        .unwrap_or(0)
}

/// A change `opalro status` found, as the diff walk hands one out.
fn file_change(c: &PathChange) -> opal_core::diff::FileChange<'_> {
    opal_core::diff::FileChange {
        path: &c.path,
        change: c.change,
        kind: c.kind,
        old_kind: c.old_kind,
        old_size: c.old_size,
        new_size: c.new_size,
        old_id: c.old_id,
        new_id: c.new_id,
    }
}

fn status_command(json: bool, full: bool, opened: &mut Option<Repository>) -> Result<(), Failure> {
    let repo: &Repository = opened.insert(with_users_key(Repository::discover(
        &std::env::current_dir()?,
    )?));
    let git_excludes = git_excludes_file(repo.root())?;
    // Status only reads, so its hints never say that nothing was saved.
    let found = status(repo, full, git_excludes.as_deref()).map_err(|e| Failure {
        hint: e.hint_when_reading(),
        message: why_stopped(&e),
        printed: false,
    })?;
    let mut warnings = found.warnings.clone();
    let (id, save) = (&found.save_id, &found.save);
    let short = short_or_whole(repo, id, &save.change_id, &mut warnings);
    warnings.extend(repo.store().warnings());
    let mut out = output::Stdout::new();
    // What stdout can't take is remembered, for `ended` to tell.
    let _ = (|| -> std::io::Result<()> {
        if json {
            let changes: Vec<serde_json::Value> = found
                .changes
                .iter()
                .map(|c| output::change_json(&file_change(c)))
                .collect();
            return writeln!(
                out,
                "{}",
                json!({
                    "opalro_json": OPALRO_JSON,
                    "position": output::position_json(&found.position),
                    "save": output::save_json(id, save, &short),
                    "changes": changes,
                    "not_written": found.not_written_here,
                    "unfinished_switch": found.unfinished_switch.is_some(),
                    "warnings": output::warnings_json(&warnings),
                })
            );
        }
        let s = style::out();
        let at = format!(
            "{} {}",
            s.paint(style::Role::ChangeId, &short),
            output::quoted_title(save)
        );
        let when = output::relative_time(save.author.time_ms, save.author.tz_offset_min, now_ms());
        let when = s.paint(style::Role::Dim, &format!("({when})"));
        match &found.position {
            Position::OnLine(line) => {
                let line = s.paint(style::Role::Line, &escape(line, false));
                writeln!(out, "On line {line} at save {at}  {when}")?;
            }
            Position::AtSave(_) => writeln!(out, "At save {at}, not on a line  {when}")?,
        }
        if found.changes.is_empty() {
            writeln!(out, "Nothing changed since then.")?;
        } else {
            let count = output::files(found.changes.len());
            let count = s.paint(style::Role::Dim, &format!("({count})"));
            writeln!(out, "\nChanged since then {count}:\n")?;
            let rows: Vec<output::Row> = found
                .changes
                .iter()
                .map(|c| output::Row::of(&file_change(c)))
                .collect();
            let mut columns = output::Columns::default();
            rows.iter().for_each(|row| columns.fit(row));
            columns.settle(s, &rows);
            for row in &rows {
                writeln!(out, "{}", columns.line(s, row))?;
            }
        }
        if !found.not_written_here.is_empty() {
            writeln!(
                out,
                "\nNot written here, because this computer can't hold them as they are (saves keep \
                 them as they were):\n"
            )?;
            for path in &found.not_written_here {
                let room = s.width.map_or(usize::MAX, |w| w.saturating_sub(4));
                let path = s.fit_path(&escape(path, false), room);
                writeln!(out, "    {}", s.paint(style::Role::Path, &path))?;
            }
        }
        Ok(())
    })();
    output::ended(None, String::new, &warnings, None)?;
    if !json && !found.changes.is_empty() {
        style::next(&format!(
            "save them with: {}",
            style::err().command("opalro save \"describe your change\"")
        ));
    }
    Ok(())
}

/// Why `opalro changes` stopped: a file couldn't be read, or what it shows couldn't be written.
enum Stop {
    Read(SaveError),
    Write(Failure),
}

impl From<SaveError> for Stop {
    fn from(e: SaveError) -> Stop {
        Stop::Read(e)
    }
}

impl From<StoreError> for Stop {
    fn from(e: StoreError) -> Stop {
        Stop::Read(e.into())
    }
}

impl From<Stop> for Failure {
    fn from(stop: Stop) -> Failure {
        match stop {
            // Only reading, so the hints never say that nothing was saved.
            Stop::Read(e) => Failure {
                hint: e.hint_when_reading(),
                message: e.to_string(),
                printed: false,
            },
            Stop::Write(failure) => failure,
        }
    }
}

fn changes_command(
    json: bool,
    save: Option<&str>,
    given: &[PathBuf],
    opened: &mut Option<Repository>,
) -> Result<(), Failure> {
    let here = resolved(&std::env::current_dir()?);
    let repo: &Repository = opened.insert(with_users_key(Repository::discover(&here)?));
    let drive = repo.drive()?;
    let paths = given
        .iter()
        .map(|p| project_path(repo.root(), &here, p, drive))
        .collect::<Result<Vec<_>, _>>()?;
    let only = (!paths.is_empty()).then_some(&paths[..]);
    let id = match save {
        Some(t) => resolve(repo, t)?,
        None => current_save(repo)?,
    };
    let saved = repo.store().save(&id, repo.signatures_allowed())?;
    let git_excludes = match save {
        Some(_) => None,
        None => git_excludes_file(repo.root())?,
    };
    let timed_out = RefCell::new(Vec::new());
    let shown = Cell::new(0_usize);
    let note = |path: &str, lines: &Lines| {
        if matches!(
            lines,
            Lines::Hunks {
                timed_out: true,
                ..
            }
        ) {
            timed_out.borrow_mut().push(path.to_owned());
        }
        shown.set(shown.get() + 1);
    };
    let walk = |each: &mut dyn FnMut(&str, &PathChange, Lines) -> Result<(), Stop>| {
        let walked = match save {
            Some(_) => save_changes(repo, id, only, each),
            None => unsaved_changes(repo, only, git_excludes.as_deref(), each),
        };
        walked.map_err(Failure::from)
    };
    let warnings = || {
        let mut warnings: Vec<Warning> = timed_out
            .borrow()
            .iter()
            .map(|path| {
                Warning::new(
                    warning::DIFF_TIMED_OUT,
                    format!(
                        "Opalro stopped comparing {} line by line after 2 seconds, so its changes \
                         show in bigger pieces than they could.",
                        escape(path, false)
                    ),
                )
            })
            .collect();
        let short = short_or_whole(repo, &id, &saved.change_id, &mut warnings);
        warnings.extend(repo.store().warnings());
        (warnings, short)
    };
    let wrote = |done: std::io::Result<()>| done.map_err(|e| Stop::Write(e.into()));
    let mut out = output::Stdout::new();
    let result = if json {
        output::stream_document(
            &mut out,
            "{\"changes\":[",
            |item| {
                walk(&mut |path, c, lines| {
                    note(path, &lines);
                    let file = output::LinesJson {
                        change: &file_change(c),
                        lines: &lines,
                    };
                    item(&file).map_err(Stop::Write)
                })
            },
            || {
                let (warnings, short) = warnings();
                (
                    json!({"opalro_json": OPALRO_JSON, "save": output::save_json(&id, &saved, &short)}),
                    warnings,
                )
            },
        )
    } else {
        walk(&mut |path, c, lines| {
            note(path, &lines);
            if shown.get() > 1 {
                wrote(writeln!(out, "{}", output::between_files()))?;
            }
            wrote(output::write_lines(&mut out, &file_change(c), &lines))
        })
        .and_then(|()| {
            let (warnings, short) = warnings();
            let s = style::out();
            let short = format!(
                "{} {}",
                s.paint(style::Role::ChangeId, &short),
                output::quoted_title(&saved)
            );
            if shown.get() == 0 {
                let said = match (save, only) {
                    (Some(_), None) => writeln!(out, "Save {short} changed no files."),
                    (Some(_), Some(_)) => {
                        writeln!(out, "Save {short} changed no files in those paths.")
                    }
                    (None, None) => writeln!(out, "Nothing changed since {short}."),
                    (None, Some(_)) => {
                        writeln!(out, "Nothing changed in those paths since {short}.")
                    }
                };
                said?;
            }
            Ok(warnings)
        })
    };
    output::listed(result, || warnings().0)
}

fn history(json: bool, opened: &mut Option<Repository>) -> Result<(), Failure> {
    let repo: &Repository = opened.insert(with_users_key(Repository::discover(
        &std::env::current_dir()?,
    )?));
    let unfinished = unfinished_warning(repo);
    let mut out = output::Stdout::new();
    let done = history_to(&mut out, repo, json, unfinished.clone());
    output::listed(done, || so_far(unfinished, repo))
}

/// The warnings of a listing stdout stopped: `unfinished`, found before it began, and the store's.
fn so_far(unfinished: Option<Warning>, repo: &Repository) -> Vec<Warning> {
    let mut warnings: Vec<Warning> = unfinished.into_iter().collect();
    warnings.extend(repo.store().warnings());
    warnings
}

fn history_to(
    out: &mut output::Stdout,
    repo: &Repository,
    json: bool,
    unfinished: Option<Warning>,
) -> Result<Vec<Warning>, Failure> {
    if json {
        let position = output::position_json(&repo.current()?.1.state.position);
        // The document `json!` would make, in pieces and in alphabetical key order, as in `show`.
        return output::stream_document(
            out,
            &format!("{{\"opalro_json\":{OPALRO_JSON},\"position\":{position},\"saves\":["),
            |each| {
                for_each_on_line(repo, |id, save, short| {
                    each(&output::save_json(id, save, short))
                })
            },
            || {
                let mut warnings: Vec<Warning> = unfinished.into_iter().collect();
                warnings.extend(repo.store().warnings());
                (json!({}), warnings)
            },
        );
    }
    let now = now_ms();
    let (_, entry) = repo.current()?;
    let mut listed = history_output::History::new(match &entry.state.position {
        Position::OnLine(line) => Some(escape(line, false)),
        Position::AtSave(_) => None,
    });
    // Each save's lines, found once: an import can bring thousands.
    let mut heads: HashMap<Id, Vec<String>> = HashMap::new();
    for (name, tip) in &entry.state.lines {
        heads.entry(*tip).or_default().push(escape(name, false));
    }
    let walked = for_each_on_line(repo, |id, s, short| {
        let row = history_output::SaveRow {
            short: short.to_owned(),
            when: output::relative_time(s.author.time_ms, s.author.tz_offset_min, now),
            author: escape(&s.author.name, false),
            title: escape(s.message.lines().next().unwrap_or_default(), false),
            lines: heads.remove(id).unwrap_or_default(),
        };
        listed.add(out, row)?;
        Ok::<(), Failure>(())
    });
    // What was read before a damaged save is still shown, before the error, which is the one
    // told even if that writing fails too.
    let finished = listed.finish(out);
    walked?;
    finished?;
    let mut warnings: Vec<Warning> = unfinished.into_iter().collect();
    warnings.extend(repo.store().warnings());
    Ok(warnings)
}

fn show(json: bool, text: Option<&str>, opened: &mut Option<Repository>) -> Result<(), Failure> {
    let repo: &Repository = opened.insert(with_users_key(Repository::discover(
        &std::env::current_dir()?,
    )?));
    let id = match text {
        Some(t) => resolve(repo, t)?,
        None => current_save(repo)?,
    };
    let unfinished = unfinished_warning(repo);
    let early = unfinished.clone();
    let store = repo.store();
    let save = store.save(&id, repo.signatures_allowed())?;
    // Of the parent, only what is shown and the root the walk compares with.
    let (old_root, parent_title) = match save.parents.first() {
        Some(p) => {
            let parent = store.save(p, repo.signatures_allowed())?;
            (Some(parent.root), output::quoted_title(&parent))
        }
        None => (None, String::new()),
    };
    // Counting first holds the saves to the limits and reads them for damage before anything is
    // printed, so a save that can't be listed prints only the error. It also sets the table's
    // columns. The changes are then printed as the walk finds them, and none is kept.
    let (mut count, mut columns) = (0_usize, output::Columns::default());
    for_each_change(store, old_root, save.root, |c| {
        count += 1;
        if !json {
            columns.fit(&output::Row::of(&c));
        }
        Ok::<(), StoreError>(())
    })?;
    let mut out = output::Stdout::new();
    let mut shown_as = String::new();
    let done = (|| -> Result<Vec<Warning>, Failure> {
        if json {
            // The document `json!` would make, in pieces: serde_json writes an object's keys in
            // alphabetical order, so `changes` comes first, then `opalro_json` and `save`.
            return output::stream_document(
                &mut out,
                "{\"changes\":[",
                |each| {
                    for_each_change(store, old_root, save.root, |c| {
                        each(&output::change_json(&c))
                    })
                },
                || {
                    // Shortened after the walk: if it fails first, only the store has warnings to
                    // give.
                    let mut warnings: Vec<Warning> = unfinished.into_iter().collect();
                    let short = short_or_whole(repo, &id, &save.change_id, &mut warnings);
                    warnings.extend(store.warnings());
                    (
                        json!({"opalro_json": OPALRO_JSON, "save": output::save_json(&id, &save, &short)}),
                        warnings,
                    )
                },
            );
        }
        let mut warnings: Vec<Warning> = unfinished.into_iter().collect();
        let short = short_or_whole(repo, &id, &save.change_id, &mut warnings);
        shown_as.clone_from(&short);
        show_output::write_head(&mut out, &id, &save, &short, &parent_title, count, now_ms())?;
        let s = style::out();
        for_each_change(store, old_root, save.root, |c| {
            writeln!(out, "{}", columns.line(s, &output::Row::of(&c)))?;
            Ok::<(), Failure>(())
        })?;
        warnings.extend(store.warnings());
        Ok(warnings)
    })();
    output::listed(done, || so_far(early, repo))?;
    if !json && count > 0 {
        style::next(&format!(
            "see its lines with: {}",
            style::err().command(&format!("opalro changes --save {shown_as}"))
        ));
    }
    Ok(())
}

fn line_command(json: bool, opened: &mut Option<Repository>) -> Result<(), Failure> {
    let repo: &Repository = opened.insert(with_users_key(Repository::discover(
        &std::env::current_dir()?,
    )?));
    let (_, entry) = repo.current()?;
    let current = match &entry.state.position {
        Position::OnLine(line) => Some(line.as_str()),
        Position::AtSave(_) => None,
    };
    let servers = repo.server_lines(&entry.state)?;
    let server_lines: Vec<String> = servers
        .servers
        .iter()
        .flat_map(|(server, lines)| lines.keys().map(move |line| format!("{line}@{server}")))
        .collect();
    let mut warnings: Vec<Warning> = unfinished_warning(repo).into_iter().collect();
    warnings.extend(repo.store().warnings());
    if json {
        let lines: Vec<&String> = entry.state.lines.keys().collect();
        let document = json!({
            "opalro_json": OPALRO_JSON,
            "position": output::position_json(&entry.state.position),
            "lines": lines,
            "server_lines": server_lines,
            "warnings": output::warnings_json(&warnings),
        });
        return output::print_quietly(&format!("{document}\n"), &warnings);
    }
    let s = style::out();
    let project = escape(&project_name(repo), false);
    let mut shown = match &entry.state.position {
        Position::OnLine(_) => format!("Lines in {project}:\n"),
        Position::AtSave(id) => format!(
            "Lines in {project}. You are at save {}, not on a line.\n",
            s.paint(style::Role::Fingerprint, &id.short_hex())
        ),
    };
    if !entry.state.lines.is_empty() {
        shown.push('\n');
    }
    for line in entry.state.lines.keys() {
        let mark = if current == Some(line.as_str()) {
            s.paint(style::Role::Bold, s.sym(style::Sym::Marker))
        } else {
            " ".to_owned()
        };
        let line = s.paint(style::Role::Line, &escape(line, false));
        shown += &format!("{mark} {line}\n");
    }
    for (server, lines) in &servers.servers {
        let server = escape(server, false);
        let named = s.paint(style::Role::Dim, &format!("({server})"));
        shown += &format!("\nOn the server {named}:\n");
        for line in lines.keys() {
            let line = format!("{}@{server}", escape(line, false));
            shown += &format!("  {}\n", s.paint(style::Role::Line, &line));
        }
    }
    output::print_quietly(&shown, &warnings)
}

/// The name of the project's folder, as the system writes it.
fn project_name(repo: &Repository) -> String {
    let root = repo.root();
    root.file_name().map_or_else(
        || root.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    )
}

fn goto_command(json: bool, text: &str, opened: &mut Option<Repository>) -> Result<(), Failure> {
    let repo = opened.insert(with_users_key(Repository::discover(
        &std::env::current_dir()?,
    )?));
    let who = now_person(&load_identity(Some(repo.root()))?);
    let git_excludes = git_excludes_file(repo.root())?;
    let target = resolve_target(repo, text)?;
    let done = {
        let _working = progress::Working::start("Writing files");
        goto(
            repo,
            &target,
            &who,
            "goto",
            git_excludes.as_deref(),
            &mut waiting,
        )
    }?;
    let mut warnings = done.applied.warnings.clone();
    // The switch has happened, so a save that can't be read back only goes unnamed.
    let save = repo
        .store()
        .save(&done.save, repo.signatures_allowed())
        .ok();
    let short = match &save {
        Some(save) => short_or_whole(repo, &done.save, &save.change_id, &mut warnings),
        None => done.save.short_hex(),
    };
    // Likewise, this only leaves out a line of advice if it fails.
    let lines = repo
        .current()
        .map(|(_, e)| e.state.lines)
        .unwrap_or_default();
    warnings.extend(repo.store().warnings());
    let text = if json {
        let document = json!({
            "opalro_json": OPALRO_JSON,
            "position": output::position_json(&done.position),
            "save": save.map(|s| output::save_json(&done.save, &s, &short)),
            "written": done.applied.written,
            "removed": done.applied.removed,
            "not_written": done.applied.not_written,
            "placeholders": done.applied.placeholders,
            "safety_copies": done.safety_copies,
            "recorded": done.recorded,
            "warnings": output::warnings_json(&warnings),
        });
        format!("{document}\n")
    } else {
        goto_output::switched(&done, &short, save.as_ref(), &lines)
    };
    output::print_done(&text, &warnings)?;
    // A goto to where the project already is records no step, so has nothing to undo.
    if !json && done.recorded {
        style::changed_your_mind();
    }
    Ok(())
}

fn discard_command(
    json: bool,
    given: &[PathBuf],
    opened: &mut Option<Repository>,
) -> Result<(), Failure> {
    let here = resolved(&std::env::current_dir()?);
    let repo = opened.insert(with_users_key(Repository::discover(&here)?));
    let drive = repo.drive()?;
    let paths = given
        .iter()
        .map(|p| project_path(repo.root(), &here, p, drive))
        .collect::<Result<Vec<_>, _>>()?;
    let who = now_person(&load_identity(Some(repo.root()))?);
    let git_excludes = git_excludes_file(repo.root())?;
    let done = {
        let _working = progress::Working::start("Writing files");
        discard(
            repo,
            (!paths.is_empty()).then_some(&paths[..]),
            &who,
            "discard",
            git_excludes.as_deref(),
            &mut waiting,
        )
    }?;
    let mut warnings = done.applied.warnings.clone();
    warnings.extend(repo.store().warnings());
    let text = if json {
        let document = json!({
            "opalro_json": OPALRO_JSON,
            "restored_paths": done.restored,
            "removed_paths": done.removed,
            "folders": done.folders,
            "not_written": done.applied.not_written,
            "placeholders": done.applied.placeholders,
            "safety_copies": done.safety_copies,
            "warnings": output::warnings_json(&warnings),
        });
        format!("{document}\n")
    } else {
        output::discarded(&done, paths.is_empty())
    };
    output::print_done(&text, &warnings)?;
    if !json && done.restored.len() + done.removed.len() + done.folders > 0 {
        style::changed_your_mind();
    }
    Ok(())
}

/// `opalro undo`, `opalro redo`, and `opalro undo --to <entry>` (`to`).
fn undo_command(
    json: bool,
    redo: bool,
    steps: &Steps,
    to: Option<&str>,
    opened: &mut Option<Repository>,
) -> Result<(), Failure> {
    let repo = opened.insert(with_users_key(Repository::discover(
        &std::env::current_dir()?,
    )?));
    let who = now_person(&load_identity(Some(repo.root()))?);
    let git_excludes = git_excludes_file(repo.root())?;
    let req = StepRequest {
        n: usize::try_from(steps.n).unwrap_or(usize::MAX),
        shelve: steps.shelve,
        who: &who,
        git_excludes: git_excludes.as_deref(),
    };
    let on_wait = &mut waiting;
    let working = progress::Working::start("Writing files");
    let done = match to {
        Some(to) => undo::undo_to(repo, to, &req, on_wait),
        None if redo => undo::redo(repo, &req, on_wait),
        None => undo::undo(repo, &req, on_wait),
    };
    drop(working);
    let (taken, shelved, finished, head_lost, mut warnings, failure) = match done {
        Ok(done) => (
            done.taken,
            done.shelved,
            done.finished,
            done.head_lost,
            done.warnings,
            None,
        ),
        Err(e) => (
            e.taken,
            e.shelved,
            false,
            false,
            e.warnings,
            Some(Failure::from(*e.error)),
        ),
    };
    let (verb, list, command) = if redo {
        ("Redid", "redone", "redo")
    } else {
        ("Undid", "undone", "undo")
    };
    let failed = failure.is_some();
    let to = to.map(|to| escape(to, false));
    // The text output; problems met naming its saves go to `into`.
    let text_of = |into: &mut Vec<Warning>| {
        let s = style::out();
        let mut text = String::new();
        // The shelves were made first, as part of the first step.
        for (label, _) in &shelved {
            text += &format!(
                "{} your unsaved changes in the way on shelf {}.\n",
                s.paint(style::Role::Ok, "Shelved"),
                escape(label, false)
            );
        }
        let mut named = |id: &Id| named_save(repo, id, into);
        let mut rows = Vec::new();
        for t in &taken {
            if head_lost {
                text += &output::picked_up_line(t, &mut named);
                text.push('\n');
            } else {
                // A step `undo --to` took records only `undo`: say where it went.
                let went = undo::went_to(repo, &t.entry).ok().flatten();
                rows.push(output::step_line(t, went.as_ref(), &shelved, &mut named));
            }
        }
        // On the way to an entry the steps are all undos, or all redos.
        let verb = match (&to, taken.first().map(|t| &t.action)) {
            (Some(_), Some(Action::Redo(_))) => "Redid",
            (Some(_), _) => "Undid",
            (None, _) => verb,
        };
        let k = rows.len();
        let some = |k: usize| if k == 1 { "step" } else { "steps" };
        // Several steps, or fewer than asked for, are counted first, then listed.
        let counted = match (failed, &to) {
            _ if k == 0 => None,
            (true, Some(to)) => Some(format!(
                "{k} {} on the way to entry {to} before one stopped:",
                some(k)
            )),
            (true, None) => Some(format!("{k} of the {} steps asked for:", steps.n)),
            (false, None) if k < req.n => Some(format!("{k} {} (all there were):", some(k))),
            _ if k > 1 => Some(format!("{k} steps:")),
            _ => None,
        };
        match counted {
            Some(counted) => {
                text += &format!("{} {counted}\n\n", s.paint(style::Role::Ok, verb));
                for (_, row) in &rows {
                    text += &format!("    {row}\n");
                }
            }
            None => {
                for (verb, row) in &rows {
                    text += &format!("{} {row}\n", s.paint(style::Role::Ok, verb));
                }
            }
        }
        if finished {
            let what = finished_switch(repo).unwrap_or_else(|| "switch".to_owned());
            let done = s.paint(style::Role::Ok, "Finished");
            text += &match &to {
                Some(to) => format!(
                    "{done} the {what} that was stopped, so nothing more was done. Running \
                     opalro undo --to {to} again goes on from there.\n"
                ),
                None => format!(
                    "{done} the {what} that was stopped, so nothing more was {list}. Running \
                     opalro {command} again takes one more step.\n"
                ),
            };
        }
        match &to {
            _ if failed || finished || !taken.is_empty() => {}
            Some(to) => {
                text += &format!(
                    "Nothing changed: things are already as they were right after entry {to}.\n"
                );
            }
            None => text += &format!("Nothing to {command}.\n"),
        }
        text
    };
    if json {
        warnings.extend(repo.store().warnings());
        let warnings = told_once(warnings);
        let shelves: Vec<serde_json::Value> = shelved
            .iter()
            .map(|(label, save)| json!({"label": label, "save": save.to_hex()}))
            .collect();
        let mut document = json!({ "shelved": shelves });
        let key = if to.is_some() { "steps" } else { list };
        document[key] = taken.iter().map(output::step_json).collect();
        if head_lost {
            document["head_lost"] = true.into();
        }
        let Some(failure) = failure else {
            let position = match taken.last() {
                Some(t) => Some(t.after.position.clone()),
                None => repo.current().ok().map(|(_, e)| e.state.position),
            };
            document["opalro_json"] = OPALRO_JSON.into();
            document["finished"] = finished.into();
            document["position"] = position.map_or(json!(null), |p| output::position_json(&p));
            document["warnings"] = output::warnings_json(&warnings);
            return output::print_done(&format!("{document}\n"), &warnings);
        };
        let document = output::failed_document(document, &failure, &warnings);
        output::print_out(&format!("{document}\n"));
        let in_document = Failure {
            printed: true,
            ..failure
        };
        // Made only for stderr, if stdout lost the document. That gives saves whole, so naming them
        // adds nothing to its warnings.
        let text = || text_of(&mut Vec::new());
        return output::ended(Some(in_document), text, &warnings, None);
    }
    let text = text_of(&mut warnings);
    match failure {
        None => {
            warnings.extend(repo.store().warnings());
            output::print_done(&text, &told_once(warnings))?;
            // One shelf made is the newest, which `opalro unshelf` brings back; each step of
            // several can make one.
            let e = style::err();
            match shelved.len() {
                0 => {}
                1 => style::next(&format!(
                    "bring your changes back with: {}",
                    e.command("opalro unshelf")
                )),
                _ => style::next(&format!(
                    "see the shelves with: {}",
                    e.command("opalro shelf --list")
                )),
            }
            Ok(())
        }
        Some(failure) => {
            output::print_out(&text);
            // Plus the store's warnings that a failure tells, such as a head move not flushed.
            let kept = repo.store().warnings().into_iter();
            warnings.extend(kept.filter(|w| warning::told_on_failure(w.kind)));
            output::ended(Some(failure), || text, &told_once(warnings), None)
        }
    }
}

/// `opalro shelf`.
fn shelf_command(
    json: bool,
    message: Option<&str>,
    opened: &mut Option<Repository>,
) -> Result<(), Failure> {
    let repo = opened.insert(with_users_key(Repository::discover(
        &std::env::current_dir()?,
    )?));
    let who = now_person(&load_identity(Some(repo.root()))?);
    let git_excludes = git_excludes_file(repo.root())?;
    let working = progress::Working::start("Writing files");
    let done = shelf::shelf(repo, message, &who, git_excludes.as_deref(), &mut waiting);
    drop(working);
    let done = done?;
    let made = match &done {
        Shelved::Made(made) => Some(made),
        Shelved::Nothing { .. } => None,
    };
    let mut warnings = made.map(|d| d.applied.warnings.clone()).unwrap_or_default();
    warnings.extend(repo.store().warnings());
    let text = if json {
        format!("{}\n", output::shelved_json(&done, &warnings))
    } else {
        match &done {
            Shelved::Made(d) => shelf_output::shelved(d),
            // The warnings that follow name what stayed, and say why.
            Shelved::Nothing {
                changed_again: true,
                ..
            } => {
                "Nothing was shelved: your files changed while Opalro was reading them. Run opalro \
                  shelf again.\n"
                    .to_owned()
            }
            Shelved::Nothing { links_left, .. } if !links_left.is_empty() => {
                "Nothing was shelved.\n".to_owned()
            }
            Shelved::Nothing { .. } => {
                "Nothing to shelve: your files match your last save.\n".to_owned()
            }
        }
    };
    output::print_done(&text, &warnings)?;
    // The new shelf is the newest, which `opalro unshelf` brings back.
    if !json && made.is_some() {
        let unshelf = style::err().command("opalro unshelf");
        style::next(&format!("bring them back with: {unshelf}"));
    }
    Ok(())
}

/// `opalro shelf --list`.
fn shelf_list_command(json: bool, opened: &mut Option<Repository>) -> Result<(), Failure> {
    let repo: &Repository = opened.insert(with_users_key(Repository::discover(
        &std::env::current_dir()?,
    )?));
    let mut warnings: Vec<Warning> = unfinished_warning(repo).into_iter().collect();
    let list = shelf::shelves(repo)?;
    warnings.extend(repo.store().warnings());
    let text = if json {
        let shelves: Vec<serde_json::Value> = list.iter().map(output::shelf_json).collect();
        let document = json!({
            "opalro_json": OPALRO_JSON,
            "shelves": shelves,
            "warnings": output::warnings_json(&warnings),
        });
        format!("{document}\n")
    } else {
        shelf_output::listed(&list, now_ms())
    };
    output::print_quietly(&text, &warnings)
}

/// `opalro unshelf`, of the shelf `number` or the newest.
fn unshelf_command(
    json: bool,
    number: Option<&str>,
    opened: &mut Option<Repository>,
) -> Result<(), Failure> {
    let repo = opened.insert(with_users_key(Repository::discover(
        &std::env::current_dir()?,
    )?));
    let who = now_person(&load_identity(Some(repo.root()))?);
    let git_excludes = git_excludes_file(repo.root())?;
    let working = progress::Working::start("Writing files");
    let done = shelf::unshelf(repo, number, &who, git_excludes.as_deref(), &mut waiting);
    drop(working);
    let done = done?;
    let mut warnings = done.applied.warnings.clone();
    warnings.extend(repo.store().warnings());
    let text = if json {
        let shelf =
            (!done.finished).then(|| json!({"label": done.label, "save": done.save.to_hex()}));
        let document = json!({
            "opalro_json": OPALRO_JSON,
            "finished": done.finished,
            "shelf": shelf,
            "written": done.applied.written,
            "removed": done.applied.removed,
            "carried": done.carried,
            "not_written": done.applied.not_written,
            "placeholders": done.applied.placeholders,
            "warnings": output::warnings_json(&warnings),
        });
        format!("{document}\n")
    } else if done.finished {
        let what = finished_switch(repo).unwrap_or_else(|| "switch".to_owned());
        // What running it again would do; nothing is said if the list can't be read.
        let left = repo.current().map(|(_, e)| e.state.shelves);
        let next = match (number, left) {
            (_, Err(_)) => String::new(),
            (None, Ok(left)) if left.is_empty() => " There are no shelves left.".to_owned(),
            (None, Ok(_)) => {
                " Running opalro unshelf again brings back the newest shelf left.".to_owned()
            }
            (Some(n), Ok(left)) if left.iter().any(|(label, _)| label == n) => {
                format!(
                    " Running opalro unshelf {} again brings it back.",
                    escape(n, false)
                )
            }
            // The stopped switch may have been the one bringing it back.
            (Some(n), Ok(_)) => format!(
                " There's no shelf {} on the list now: opalro shelf --list shows the shelves there \
                 are.",
                escape(n, false)
            ),
        };
        format!(
            "{} the {what} that was stopped, so no shelf was brought back this time.{next}\n",
            style::out().paint(style::Role::Ok, "Finished")
        )
    } else {
        let title = repo
            .store()
            .save(&done.save, repo.signatures_allowed())
            .map(|s| format!(" {}", output::quoted_title(&s)))
            .unwrap_or_default();
        shelf_output::unshelved(&done, &title)
    };
    output::print_done(&text, &warnings)?;
    if !json && !done.finished {
        style::changed_your_mind();
    }
    Ok(())
}

/// `opalro tidy`.
fn tidy_command(json: bool, opened: &mut Option<Repository>) -> Result<(), Failure> {
    let repo = opened.insert(with_users_key(Repository::discover(
        &std::env::current_dir()?,
    )?));
    let window = undo_window_days()?;
    let working = progress::Working::start("Tidying");
    let done = tidy::tidy(repo, window, now_ms(), &mut waiting, &mut |to_free| {
        if style::for_people() {
            output::tell(&format!(
                "Tidying: about {} to free",
                opal_core::text::bytes(to_free)
            ));
        }
    });
    drop(working);
    let done = done?;
    let warnings = repo.store().warnings();
    let text = if json {
        let document = json!({
            "opalro_json": OPALRO_JSON,
            "to_free": done.to_free,
            "bytes_before": done.bytes_before,
            "bytes_after": done.bytes_after,
            "packs_removed": done.packs_removed,
            "packs_written": done.packs_written,
            "packs_young": done.packs_young,
            "warnings": output::warnings_json(&warnings),
        });
        format!("{document}\n")
    } else {
        output::tidied(&done)
    };
    output::print_done(&text, &warnings)
}

/// `opalro verify`. Problems found end it with exit code 1, the outcome line being the error.
fn verify_command(json: bool, opened: &mut Option<Repository>) -> Result<(), Failure> {
    let repo: &Repository =
        opened.insert(Repository::discover_untouched(&std::env::current_dir()?)?);
    let window = undo_window_days()?;
    let mut bar = progress::Bar::new("Checking stored objects");
    let done = verify::verify(repo, window, now_ms(), &mut |read, total| {
        bar.update(read, Some(total), true);
    });
    bar.finish();
    let warnings = done.warnings.clone();
    // The folder's name came from outside Opalro, like any name in a project.
    let project = escape(&project_name(repo), false);
    let failure = (done.problems_total > 0).then(|| Failure {
        message: output::verify_outcome(&done, &project),
        hint: Some(output::verify_hint(&done)),
        printed: true,
    });
    if json {
        let document = output::verified_json(&done);
        let Some(failure) = failure else {
            return output::print_quietly(&format!("{document}\n"), &warnings);
        };
        let document = output::failed_document(document, &failure, &warnings);
        output::print_out(&format!("{document}\n"));
        return output::ended(Some(failure), String::new, &warnings, None);
    }
    output::print_out(&output::verified(&done, &project));
    output::ended(None, String::new, &warnings, None)?;
    let Some(failure) = failure else {
        return Ok(());
    };
    if let Some(hint) = &failure.hint {
        output::tell_hint(hint);
    }
    Err(failure)
}

/// How many of the steps undo can take `opalro undo --list` shows.
const UNDO_LIST: usize = 20;

/// `opalro undo --list`, and with `--all` (`all`) the whole diary.
fn undo_list_command(
    json: bool,
    all: bool,
    opened: &mut Option<Repository>,
) -> Result<(), Failure> {
    let repo: &Repository = opened.insert(with_users_key(Repository::discover(
        &std::env::current_dir()?,
    )?));
    let unfinished = unfinished_warning(repo);
    let warnings: Vec<Warning> = unfinished.clone().into_iter().collect();
    let mut out = output::Stdout::new();
    // With `head` lost, either list gives where the diary may have stopped instead.
    let done = if matches!(repo.current(), Err(RepoError::HeadLost { .. })) {
        stopped_at_to(&mut out, repo, json, warnings)
    } else if all {
        whole_diary_to(&mut out, repo, json, warnings)
    } else {
        undo_list_to(&mut out, repo, json, warnings)
    };
    output::listed(done, || so_far(unfinished, repo))
}

/// The steps redo can bring back, newest first and numbered for `opalro redo N`, then those undo
/// can take, numbered for `opalro undo N`.
fn undo_list_to(
    out: &mut output::Stdout,
    repo: &Repository,
    json: bool,
    mut warnings: Vec<Warning>,
) -> Result<Vec<Warning>, Failure> {
    let list = undo::undo_list(repo, UNDO_LIST)?;
    if json {
        let entries = |steps: &[StepInfo]| -> Vec<serde_json::Value> {
            let json = |s: &StepInfo| output::entry_json(&s.entry, &s.diary, &s.effects);
            steps.iter().map(json).collect()
        };
        warnings.extend(repo.store().warnings());
        let warnings = told_once(warnings);
        let document = json!({
            "opalro_json": OPALRO_JSON,
            "redo": entries(&list.redo),
            "undo": entries(&list.undo),
            "more": list.more,
            "warnings": output::warnings_json(&warnings),
        });
        writeln!(out, "{document}")?;
        return Ok(warnings);
    }
    let now = now_ms();
    let (mut named, unshortened) = names(repo);
    warnings.extend(unshortened);
    let files = &mut |from, to| files_between(repo, from, to);
    let mut step = |label: String, s: &StepInfo| undo_output::Step {
        label,
        entry: s.entry.short_hex(),
        when: when(&s.diary, now),
        what: output::entry_line(&s.diary, &s.effects, &mut named, files),
    };
    let redo: Vec<undo_output::Step> = (list.redo.iter().enumerate().rev())
        .map(|(i, s)| step(format!("redo {}", i + 1), s))
        .collect();
    // Undo can't take the diary's first entry, so it gets no number.
    let undo: Vec<undo_output::Step> = (list.undo.iter().enumerate())
        .map(|(i, s)| match s.diary.parent {
            Some(_) => step(format!("undo {}", i + 1), s),
            None => step(String::new(), s),
        })
        .collect();
    let project = escape(&project_name(repo), false);
    undo_output::write_list(out, &project, &redo, &undo, list.more)?;
    warnings.extend(repo.store().warnings());
    Ok(told_once(warnings))
}

/// Every entry of the diary, newest first, printed as it is read.
fn whole_diary_to(
    out: &mut output::Stdout,
    repo: &Repository,
    json: bool,
    mut warnings: Vec<Warning>,
) -> Result<Vec<Warning>, Failure> {
    if json {
        // The document `json!` would make, in pieces and in alphabetical key order, as in
        // `history`.
        return output::stream_document(
            out,
            "{\"entries\":[",
            |each| {
                undo::whole_diary(repo, |id, entry, effects| {
                    each(&output::entry_json(&id, entry, effects))
                })
            },
            || {
                warnings.extend(repo.store().warnings());
                (json!({"opalro_json": OPALRO_JSON}), told_once(warnings))
            },
        );
    }
    let now = now_ms();
    let (mut named, unshortened) = names(repo);
    warnings.extend(unshortened);
    let files = &mut |from, to| files_between(repo, from, to);
    let s = style::out();
    undo::whole_diary(repo, |id, entry, effects| {
        let what = output::entry_line(entry, effects, &mut named, files);
        writeln!(
            out,
            "{}  {}  {what}",
            s.paint(style::Role::Fingerprint, &id.short_hex()),
            s.paint(style::Role::Dim, &when(entry, now))
        )?;
        Ok::<(), Failure>(())
    })?;
    warnings.extend(repo.store().warnings());
    Ok(told_once(warnings))
}

/// The entries no other follows, newest first, for a lost `head`: where the diary may have
/// stopped, for `opalro undo --to` to pick one from.
fn stopped_at_to(
    out: &mut output::Stdout,
    repo: &Repository,
    json: bool,
    mut warnings: Vec<Warning>,
) -> Result<Vec<Warning>, Failure> {
    let ends = undo::stopped_at(repo)?;
    if json {
        let entry = |s: &StepInfo| output::entry_json(&s.entry, &s.diary, &s.effects);
        let ends: Vec<serde_json::Value> = ends.iter().map(entry).collect();
        warnings.extend(repo.store().warnings());
        let warnings = told_once(warnings);
        let document = json!({
            "opalro_json": OPALRO_JSON,
            "head_lost": true,
            "ends": ends,
            "warnings": output::warnings_json(&warnings),
        });
        writeln!(out, "{document}")?;
        return Ok(warnings);
    }
    if ends.is_empty() {
        writeln!(
            out,
            "The project's current diary entry is lost, and no diary entry in the store can be \
             read to go on from. Ask for help before trying to repair it."
        )?;
    } else {
        writeln!(
            out,
            "The project's current diary entry is lost. The diary may have stopped at any of \
             these, newest first:"
        )?;
        let now = now_ms();
        // There is no current line to shorten change IDs against.
        let mut named = |id: &Id| named_by(repo, id, ChangeId::to_letters);
        let files = &mut |from, to| files_between(repo, from, to);
        let screen = style::out();
        for s in &ends {
            let what = output::entry_line(&s.diary, &s.effects, &mut named, files);
            writeln!(
                out,
                "{}  {}  {what}",
                screen.paint(style::Role::Fingerprint, &s.entry.short_hex()),
                screen.paint(style::Role::Dim, &when(&s.diary, now))
            )?;
        }
        writeln!(
            out,
            "Run opalro undo --to <entry> to make one of them the current one; it changes no file."
        )?;
    }
    warnings.extend(repo.store().warnings());
    Ok(told_once(warnings))
}

/// Names saves as `named_save` does, but each once, their change IDs shortened from one walk of
/// the current line for them all; with the warning that it couldn't be read, if so.
fn names(
    repo: &Repository,
) -> (
    impl FnMut(&Id) -> (String, Option<String>) + '_,
    Option<Warning>,
) {
    let (line, unshortened) = match LineIds::read(repo) {
        Ok(line) => (Some(line), None),
        Err(problem) => (None, Some(not_shortened(&problem))),
    };
    let mut known = HashMap::new();
    let named = move |id: &Id| {
        let short = |change_id: &ChangeId| match &line {
            Some(line) if line.shared(id, change_id) => id.short_hex(),
            Some(line) => line.short(id, change_id),
            None => change_id.to_letters(),
        };
        let name = known.entry(*id);
        name.or_insert_with(|| named_by(repo, id, short)).clone()
    };
    (named, unshortened)
}

/// How many files two saves differ in, if both can be read.
fn files_between(repo: &Repository, from: Id, to: Id) -> Option<usize> {
    let root = |id: &Id| repo.store().save(id, repo.signatures_allowed()).ok();
    let (from, to) = (root(&from)?.root, root(&to)?.root);
    count_changes(repo.store(), Some(from), to).ok()
}

/// When an entry's command ran, as lists say it.
fn when(entry: &DiaryEntry, now: i64) -> String {
    output::relative_time(entry.who.time_ms, entry.who.tz_offset_min, now)
}

/// What a command says while another one holds the project.
fn waiting(_: &str) {
    output::tell("Waiting for another Opalro command to finish...");
}

/// `warnings` with each told once, in the order first met: naming several saves can meet the same
/// problem more than once.
fn told_once(warnings: Vec<Warning>) -> Vec<Warning> {
    let mut told: Vec<Warning> = Vec::new();
    for w in warnings {
        if !told.contains(&w) {
            told.push(w);
        }
    }
    told
}

/// A save as messages name it: its change ID at its shortest, and the first line of its
/// message; 12 hex digits of its fingerprint if it can't be read, or if another save on the line
/// has its change ID.
fn named_save(repo: &Repository, id: &Id, warnings: &mut Vec<Warning>) -> (String, Option<String>) {
    named_by(repo, id, |change_id| {
        let short = short_or_whole(repo, id, change_id, warnings);
        // Only a change ID given whole can be another save's too, so only then is the line read.
        let shared = short == change_id.to_letters()
            && LineIds::read(repo).is_ok_and(|line| line.shared(id, change_id));
        if shared { id.short_hex() } else { short }
    })
}

/// [`named_save`], with the change ID shortened by `short`.
fn named_by(
    repo: &Repository,
    id: &Id,
    short: impl FnOnce(&ChangeId) -> String,
) -> (String, Option<String>) {
    match repo.store().save(id, repo.signatures_allowed()) {
        Ok(save) => {
            let first = save.message.lines().next().unwrap_or_default().to_owned();
            (short(&save.change_id), Some(first))
        }
        Err(_) => (id.short_hex(), None),
    }
}

/// What the switch a command finished was, from the diary entry it took to: "goto", say, or
/// "undo of goto".
fn finished_switch(repo: &Repository) -> Option<String> {
    let (_, entry) = repo.current().ok()?;
    let (what, step) = match entry.action {
        Action::Command => return Some(escape(&entry.command, false)),
        Action::Undo(step) => ("undo", step),
        Action::Redo(step) => ("redo", step),
    };
    let step = repo.entry(&step).ok()?;
    Some(format!("{what} of {}", escape(&step.command, false)))
}

/// What a command line clap stopped at prints: Opalro's own help when help was asked for, or when
/// no command was given, which still ends with 2; clap's own message for anything else.
fn help_or_usage_error(e: &clap::Error, args: &[OsString], json: bool) -> ExitCode {
    use clap::error::ErrorKind;
    let code = match e.kind() {
        ErrorKind::DisplayHelp => 0,
        ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand => 2,
        _ => e.exit(),
    };
    style::find_out(json);
    // The command asked about: the first word that isn't an option, or the one after `help`.
    let mut words = args
        .iter()
        .filter(|a| !a.to_string_lossy().starts_with('-'));
    let first = words.next();
    let named = if first.is_some_and(|w| w == "help") {
        words.next()
    } else {
        first
    };
    let page = named.and_then(|name| help::page(&mut Cli::command(), &name.to_string_lossy()));
    output::print_out(&page.unwrap_or_else(help::overview));
    ExitCode::from(code)
}

fn main() -> ExitCode {
    let mut typed = std::env::args_os();
    let program = typed.next().unwrap_or_else(|| "opalro".into());
    let typed: Vec<OsString> = typed.collect();
    let json = typed.iter().any(|a| a == "--json");
    let (args, shortcut) = match git_words::translate(typed) {
        Typed::Opalro(args) => (args, None),
        Typed::Shortcut { word, args } => (args, Some(word)),
        Typed::Explain(text) => {
            style::find_out(json);
            git_words::explain(text);
            return ExitCode::from(2);
        }
    };
    let cli = match Cli::try_parse_from(std::iter::once(program).chain(args.iter().cloned())) {
        Ok(cli) => cli,
        Err(e) => return help_or_usage_error(&e, &args, json),
    };
    style::find_out(cli.json);
    // The project a command opens is kept here, so that if the command fails, the error can list
    // what the project's store had noticed by then.
    let mut opened = None;
    let result = match cli.command {
        Command::Start { preset } => start(cli.json, preset.map(Preset::from), &mut opened),
        Command::Setup { preset, yes } => {
            setup_command(cli.json, preset.map(Preset::from), yes, &mut opened)
        }
        Command::Import => import(cli.json, &mut opened),
        Command::Save(args) => save(cli.json, args, &mut opened),
        Command::Line => line_command(cli.json, &mut opened),
        Command::Status { full } => status_command(cli.json, full, &mut opened),
        Command::Changes { save, paths } => {
            changes_command(cli.json, save.as_deref(), &paths, &mut opened)
        }
        Command::History => history(cli.json, &mut opened),
        Command::Show { save } => show(cli.json, save.as_deref(), &mut opened),
        Command::Goto { target } => goto_command(cli.json, &target, &mut opened),
        Command::Discard { paths } => discard_command(cli.json, &paths, &mut opened),
        Command::Undo {
            list: true, all, ..
        } => undo_list_command(cli.json, all, &mut opened),
        Command::Undo { steps, to, .. } => {
            undo_command(cli.json, false, &steps, to.as_deref(), &mut opened)
        }
        Command::Redo { steps } => undo_command(cli.json, true, &steps, None, &mut opened),
        Command::Shelf { list: true, .. } => shelf_list_command(cli.json, &mut opened),
        Command::Shelf { message, .. } => shelf_command(cli.json, message.as_deref(), &mut opened),
        Command::Unshelf { number } => unshelf_command(cli.json, number.as_deref(), &mut opened),
        Command::Tidy => tidy_command(cli.json, &mut opened),
        Command::Verify => verify_command(cli.json, &mut opened),
    };
    let code = match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(failure) => {
            let noticed = opened.map_or_else(Vec::new, |repo| repo.store().warnings());
            output::print_error(cli.json, &failure, &noticed);
            ExitCode::from(1)
        }
    };
    if let Some(word) = shortcut {
        git_words::teach(word);
    }
    code
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;

    #[test]
    fn commit_knows_every_option_of_save_that_takes_a_value() {
        let cli = Cli::command();
        let save = cli.find_subcommand("save").unwrap();
        let mut taking: Vec<String> = save
            .get_opts()
            .filter(|o| o.get_action().takes_values())
            .map(|o| format!("--{}", o.get_long().unwrap()))
            .collect();
        taking.sort();
        assert_eq!(taking, git_words::SAVE_VALUE_OPTIONS);
    }

    /// `line` split as a shell would: at spaces, except inside double quotes.
    fn words(line: &str) -> Vec<String> {
        let mut words = vec![String::new()];
        let mut quoted = false;
        for c in line.chars() {
            match c {
                '"' => quoted = !quoted,
                ' ' if !quoted => words.push(String::new()),
                c => words.last_mut().unwrap().push(c),
            }
        }
        words
    }

    #[test]
    fn each_commands_help_leads_with_examples_that_parse() {
        let cli = Cli::command();
        for command in cli.get_subcommands() {
            let name = command.get_name();
            let examples = help::examples(name);
            assert!(!examples.is_empty(), "{name}");
            for (line, does) in examples {
                assert!(!does.is_empty(), "{line}");
                let words = words(line);
                assert_eq!(words[..2], ["opalro", name], "{line}");
                let parsed = Cli::try_parse_from(&words);
                assert!(parsed.is_ok(), "{line}: {:?}", parsed.err());
            }
        }
        assert!(help::examples("commit").is_empty());
    }

    #[test]
    fn the_overview_names_every_command_once_and_no_other() {
        let mut named: Vec<&str> = help::GROUPS
            .iter()
            .flat_map(|(_, rows)| rows.iter())
            .flat_map(|(name, _, beside)| std::iter::once(*name).chain(*beside))
            .collect();
        named.sort_unstable();
        let mut commands: Vec<String> = Cli::command()
            .get_subcommands()
            .map(|c| c.get_name().to_owned())
            .collect();
        commands.sort();
        assert_eq!(named, commands);
    }
}
