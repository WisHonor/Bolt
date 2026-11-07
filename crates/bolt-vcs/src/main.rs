//! The `bolt` command-line tool.

mod import_output;
mod output;

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::io::{BufRead, IsTerminal, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use bolt_core::changes::{Lines, save_changes, unsaved_changes};
use bolt_core::config::{
    ConfigError, Identity, git_excludes_file, ignored_identity_note, ignored_project_identity,
    load_identity, now_person, undo_window_days,
};
use bolt_core::diary::{Action, DiaryEntry, Position};
use bolt_core::diff::{count_changes, for_each_change};
use bolt_core::fsio::fault_point;
use bolt_core::history::{
    LineIds, ResolveError, current_save, for_each_on_line, resolve, short_change_id,
};
use bolt_core::id::{ChangeId, Id};
use bolt_core::import::{ImportError, ImportRequest, import_git};
use bolt_core::lines::resolve_target;
use bolt_core::names::{GivenPathError, project_given, project_path};
use bolt_core::repo::{RepoError, Repository, elevated_only_note, resolved};
use bolt_core::save::Save;
use bolt_core::shelf::{self, Shelved};
use bolt_core::snapshot::{LargeSave, SaveError, SaveOutcome, SaveRequest, save_folder};
use bolt_core::status::{PathChange, status, why_stopped};
use bolt_core::store::StoreError;
use bolt_core::switch::{SwitchError, discard, goto};
use bolt_core::text::escape;
use bolt_core::tidy;
use bolt_core::undo::{self, StepInfo, StepRequest};
use bolt_core::warning::{self, Warning};
use bolt_core::writeout::unfinished_warning;
use clap::{Parser, Subcommand};
use output::{BOLT_JSON, Failure};
use serde_json::json;

#[derive(Parser)]
#[command(
    name = "bolt",
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
    /// Make this folder a Bolt project.
    Start,
    /// Bring the Git repository in this folder into Bolt, with all its history.
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
        #[arg(value_name = "PATH")]
        paths: Vec<PathBuf>,
    },
    /// List the saves on this line.
    History,
    /// Show one save.
    Show { save: Option<String> },
    /// List the lines, marking the one you're on, then the server lines.
    Line,
    /// Go to a line, or to a save: by line name, line@server, change ID or fingerprint.
    Goto { target: String },
    /// Throw away your unsaved changes: in these files and folders, or everywhere. A safety copy
    /// of each changed or new file is kept.
    Discard {
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
        /// of its ID, as bolt undo --list shows them.
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
        /// The shelf's number, as bolt shelf --list shows it.
        #[arg(value_name = "NUMBER")]
        number: Option<String>,
    },
    /// Free the space nothing needs any more: saves no line, shelf or tag reaches, and what only
    /// steps older than the undo window (undo-window-days in your settings, 30 by default) needed.
    Tidy,
}

#[derive(clap::Args)]
struct SaveArgs {
    message: Option<String>,
    #[arg(value_name = "PATH")]
    paths: Vec<PathBuf>,
    /// After an import, save files Git wrote with CRLF line endings as they are.
    #[arg(long)]
    accept_crlf: bool,
    /// At a save rather than on a line, the name of the new line this save starts.
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

/// `repo`, sealing its caches with the user's own key. A key Bolt can't use is a warning, and
/// the caches go unused this time: they only make Bolt faster.
fn with_users_key(mut repo: Repository) -> Repository {
    match bolt_core::keyed::cache_key() {
        Ok(Some(key)) => repo.set_cache_key(key),
        Ok(None) => {}
        Err(e) => repo.store().note(Warning::new(
            warning::CACHE_KEY_UNUSABLE,
            format!(
                "{e} Bolt worked without its caches this time. {}",
                e.hint().unwrap_or_default()
            ),
        )),
    }
    repo
}

fn start(json: bool, opened: &mut Option<Repository>) -> Result<(), Failure> {
    let root = resolved(&std::env::current_dir()?);
    // The user's own settings may name this folder, so the first save is theirs too.
    let who = now_person(&load_identity(Some(&root))?);
    let repo: &Repository = opened.insert(with_users_key(Repository::init(&root, &who)?));
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
                    "The project was made, but Bolt couldn't read its first save back. {}\n\
                     Run `bolt history` to check that the project reads.",
                    problem.message
                ),
            ));
            None
        }
    };
    warnings.extend(repo.store().warnings());
    let text = if json {
        let save = first.as_ref().map_or(json!(null), |(id, save, short)| {
            output::save_json(id, save, short)
        });
        let document = json!({
            "bolt_json": BOLT_JSON,
            "root": root.display().to_string(),
            "line": "main",
            "save": save,
            "warnings": output::warnings_json(&warnings),
        });
        format!("{document}\n")
    } else {
        // The folder's name came from outside Bolt, like any name in a project.
        let shown = escape(&root.display().to_string(), false);
        let mut text = format!("Started a Bolt project in {shown}.\n");
        if let Some((_, _, short)) = &first {
            text += &format!("Your first save is {short}, on line main.\n");
            text += "Next: change some files, then run: bolt save \"describe your change\"\n";
        }
        text
    };
    output::print_done(&text, &warnings)
}

fn import(json: bool, opened: &mut Option<Repository>) -> Result<(), Failure> {
    let root = resolved(&std::env::current_dir()?);
    let who = now_person(&load_identity(Some(&root))?);
    let mut reading = output::Progress::new("Reading Git's history", "commits");
    let mut importing = output::Progress::new("Importing commits", "commits");
    let imported = import_git(
        &root,
        &mut ImportRequest {
            who: &who,
            command: "import",
            // The first pass can't say how many commits there are; the second can.
            progress: &mut |done, total| match total {
                None => reading.update(done, None),
                Some(_) => {
                    reading.finish();
                    importing.update(done, total);
                }
            },
        },
        opened,
    );
    reading.finish();
    importing.finish();
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
        let standing = match &report.position {
            Position::OnLine(line) => format!("You're on line {}.", escape(line, false)),
            Position::AtSave(id) => {
                // By change ID, as everywhere else; by fingerprint if the save can't be read now.
                let short = opened.as_ref().and_then(|repo| {
                    let save = repo.store().save(id, repo.signatures_allowed()).ok()?;
                    Some(short_or_whole(repo, id, &save.change_id, &mut warnings))
                });
                let short = short.unwrap_or_else(|| id.to_hex());
                format!(
                    "You're at save {short}, where Git's HEAD was detached: not on a line. Saving \
                     here asks for a new line's name."
                )
            }
        };
        import_output::import_text(&report, &standing)
    };
    output::print_done(&text, &warnings)
}

/// Asks in the terminal for the new line's name; `None` if there is no terminal to ask in.
fn ask_line_name() -> Result<Option<String>, Failure> {
    if !std::io::stdin().is_terminal() {
        return Ok(None);
    }
    let _ = write!(std::io::stderr(), "Name a new line for this save: ");
    let mut answer = String::new();
    std::io::stdin().lock().read_line(&mut answer)?;
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
    if !std::io::stdin().is_terminal() {
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
    // With --json nothing is asked, so a large save stops and names --yes.
    let mut said_no = false;
    let mut confirm = |large: &LargeSave| {
        if yes || json {
            return yes;
        }
        let answer = ask_large(large);
        said_no = answer == Some(false);
        answer.unwrap_or(false)
    };
    let asked;
    let saved = match save_folder(repo, &req, &mut on_wait, &mut confirm) {
        // Only now is it known that the save is at a save: a stopped switch may have moved it.
        Err(SaveError::NeedsLineName) if !json && let Some(name) = ask_line_name()? => {
            asked = name;
            req.new_line = Some(&asked);
            save_folder(repo, &req, &mut on_wait, &mut confirm)
        }
        done => done,
    };
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
                    "bolt_json": BOLT_JSON,
                    "result": "saved",
                    "paths": paths,
                    "save": output::save_json(&save_id, &save, &short),
                    "changed_files": changed_files,
                    "replaced": replaced.map(|id| id.to_hex()),
                    "warnings": output::warnings_json(&warnings),
                });
                format!("{}\n", with_ignored(document, ignored.as_ref()))
            } else {
                let title = escape(save.message.lines().next().unwrap_or_default(), false);
                let counted = match changed_files {
                    Some(1) => "(1 file)".to_owned(),
                    Some(n) => format!("({n} files)"),
                    None => "(couldn't count the changed files)".to_owned(),
                };
                let made = if replaced.is_some() {
                    "Amended"
                } else {
                    "Saved"
                };
                let mut text = format!("{made} {short}  \"{title}\"  {counted}\n");
                if let Some(line) = req.new_line {
                    text += &format!("You're on the new line {}.\n", escape(line, false));
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
                    "bolt_json": BOLT_JSON,
                    "result": "nothing-changed",
                    "paths": paths,
                    "warnings": output::warnings_json(&warnings),
                });
                format!("{}\n", with_ignored(document, ignored.as_ref()))
            } else {
                let last = repo.store().save(&current, repo.signatures_allowed())?;
                let short = short_or_whole(repo, &current, &last.change_id, &mut warnings);
                warnings.extend(repo.store().warnings());
                let mut text = if amend {
                    let what = if paths.is_empty() {
                        "your files match"
                    } else {
                        "what you named matches"
                    };
                    format!(
                        "Nothing to amend: {what} your last save ({short}), and so does the \
                         message.\n"
                    )
                } else if paths.is_empty() {
                    format!("Nothing to save: no changes since your last save ({short}).\n")
                } else {
                    format!("Nothing to save there: it matches your last save ({short}).\n")
                };
                if req.new_line.is_some() {
                    text += "No line was made: a new line starts with a save, so change a file \
                             first.\n";
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
        format!("Bolt couldn't read this line's older saves to shorten the change ID. {problem}"),
    )
}

/// `document`, plus the identity a project's settings named and Bolt ignored, if there was one.
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

/// A change `bolt status` found, as the diff walk hands one out.
fn file_change(c: &PathChange) -> bolt_core::diff::FileChange<'_> {
    bolt_core::diff::FileChange {
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
                    "bolt_json": BOLT_JSON,
                    "position": output::position_json(&found.position),
                    "save": output::save_json(id, save, &short),
                    "changes": changes,
                    "not_written_here": found.not_written_here,
                    "unfinished_switch": found.unfinished_switch,
                    "warnings": output::warnings_json(&warnings),
                })
            );
        }
        match &found.position {
            Position::OnLine(line) => writeln!(out, "On line {}.", escape(line, false))?,
            Position::AtSave(_) => writeln!(out, "At save {short}, not on a line.")?,
        }
        if found.changes.is_empty() {
            writeln!(out, "Nothing changed since {short}.")?;
        }
        for c in &found.changes {
            writeln!(out, "{}", output::change_line(&file_change(c)))?;
        }
        if !found.not_written_here.is_empty() {
            writeln!(
                out,
                "Not written here, because this computer can't hold them as they are (saves keep \
                 them as they were):"
            )?;
            for path in &found.not_written_here {
                writeln!(out, "  {}", escape(path, false))?;
            }
        }
        Ok(())
    })();
    output::ended(None, String::new, &warnings, None)
}

/// Why `bolt changes` stopped: a file couldn't be read, or what it shows couldn't be written.
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
    // The older Windows console shows colour codes as text until a console call turns them on,
    // and that call is unsafe code for the platform crate: no colour on Windows for now.
    let color = !cfg!(windows)
        && !json
        && std::io::stdout().is_terminal()
        && std::env::var_os("NO_COLOR").is_none_or(|v| v.is_empty());
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
                        "Bolt stopped comparing {} line by line after 2 seconds, so its changes \
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
            &format!("{{\"bolt_json\":{BOLT_JSON},\"changes\":["),
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
                    json!({"save": output::save_json(&id, &saved, &short)}),
                    warnings,
                )
            },
        )
    } else {
        walk(&mut |path, c, lines| {
            note(path, &lines);
            if shown.get() > 1 {
                wrote(writeln!(out))?;
            }
            wrote(output::write_lines(
                &mut out,
                &file_change(c),
                &lines,
                color,
            ))
        })
        .and_then(|()| {
            let (warnings, short) = warnings();
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
        let line = match &repo.current()?.1.state.position {
            bolt_core::diary::Position::OnLine(name) => json!(name),
            bolt_core::diary::Position::AtSave(_) => json!(null),
        };
        // The document `json!` would make, in pieces and in alphabetical key order, as in `show`.
        return output::stream_document(
            out,
            &format!("{{\"bolt_json\":{BOLT_JSON},\"line\":{line},\"saves\":["),
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
    for_each_on_line(repo, |_, s, short| {
        let when = output::relative_time(s.author.time_ms, s.author.tz_offset_min, now);
        let title = s.message.lines().next().unwrap_or_default();
        writeln!(
            out,
            "{short}  {when}  {}  {}",
            escape(&s.author.name, false),
            escape(title, false)
        )?;
        Ok::<(), Failure>(())
    })?;
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
    let old_root = match save.parents.first() {
        Some(p) => Some(store.save(p, repo.signatures_allowed())?.root),
        None => None,
    };
    // Counting first holds the saves to the limits and reads them for damage before anything is
    // printed, so a save that can't be listed prints only the error. The changes are then
    // printed as the walk finds them, and none is kept.
    let count = count_changes(store, old_root, save.root)?;
    let mut out = output::Stdout::new();
    let done = (|| -> Result<Vec<Warning>, Failure> {
        if json {
            // The document `json!` would make, in pieces: serde_json writes an object's keys in
            // alphabetical order, so `changes` comes between `bolt_json` and `save`.
            return output::stream_document(
                &mut out,
                &format!("{{\"bolt_json\":{BOLT_JSON},\"changes\":["),
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
                        json!({"save": output::save_json(&id, &save, &short)}),
                        warnings,
                    )
                },
            );
        }
        let mut warnings: Vec<Warning> = unfinished.into_iter().collect();
        let short = short_or_whole(repo, &id, &save.change_id, &mut warnings);
        let when = output::relative_time(save.author.time_ms, save.author.tz_offset_min, now_ms());
        writeln!(
            out,
            "Save {short} · {} · {when}",
            escape(&save.author.name, false)
        )?;
        // Indented, as Git does, so that no line of the message can pass for a change below it.
        for line in escape(save.message.trim_end(), true).lines() {
            if line.trim().is_empty() {
                writeln!(out)?;
            } else {
                writeln!(out, "    {line}")?;
            }
        }
        writeln!(out)?;
        for_each_change(store, old_root, save.root, |c| {
            writeln!(out, "{}", output::change_line(&c))?;
            Ok::<(), Failure>(())
        })?;
        if count == 0 {
            writeln!(out, "  (no file changes)")?;
        }
        warnings.extend(store.warnings());
        Ok(warnings)
    })();
    output::listed(done, || so_far(early, repo))
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
            "bolt_json": BOLT_JSON,
            "current": current,
            "lines": lines,
            "server_lines": server_lines,
            "warnings": output::warnings_json(&warnings),
        });
        return output::print_quietly(&format!("{document}\n"), &warnings);
    }
    let mut shown = String::new();
    for line in entry.state.lines.keys() {
        let mark = if current == Some(line.as_str()) {
            '*'
        } else {
            ' '
        };
        shown += &format!("{mark} {}\n", escape(line, false));
    }
    for line in &server_lines {
        shown += &format!("  {}\n", escape(line, false));
    }
    output::print_quietly(&shown, &warnings)
}

fn goto_command(json: bool, text: &str, opened: &mut Option<Repository>) -> Result<(), Failure> {
    let repo = opened.insert(with_users_key(Repository::discover(
        &std::env::current_dir()?,
    )?));
    let who = now_person(&load_identity(Some(repo.root()))?);
    let git_excludes = git_excludes_file(repo.root())?;
    let target = resolve_target(repo, text)?;
    let done = goto(
        repo,
        &target,
        &who,
        "goto",
        git_excludes.as_deref(),
        &mut waiting,
    )?;
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
            "bolt_json": BOLT_JSON,
            "position": output::position_json(&done.position),
            "save": save.map(|s| output::save_json(&done.save, &s, &short)),
            "written": done.applied.written,
            "removed": done.applied.removed,
            "not_written": done.applied.not_written,
            "placeholders": done.applied.placeholders,
            "files_in_use": done.applied.in_use,
            "safety_copies": done.safety_copies,
            "warnings": output::warnings_json(&warnings),
        });
        format!("{document}\n")
    } else {
        let mut text = match done.carried {
            0 => String::new(),
            1 => "Your unsaved changes in 1 file came along.\n".to_owned(),
            n => format!("Your unsaved changes in {n} files came along.\n"),
        };
        match &done.position {
            Position::OnLine(line) => text += &format!("You're on line {}.\n", escape(line, false)),
            Position::AtSave(id) => {
                text += &format!(
                    "You're at save {short}, not on a line. Saving here asks for a new line's \
                     name.\n"
                );
                if let Some((line, _)) = lines.iter().find(|(_, tip)| *tip == id) {
                    let line = escape(line, false);
                    text += &format!("To keep saving on {line}: bolt goto {line}\n");
                }
            }
        }
        text
    };
    output::print_done(&text, &warnings)
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
    let done = discard(
        repo,
        (!paths.is_empty()).then_some(&paths[..]),
        &who,
        "discard",
        git_excludes.as_deref(),
        &mut waiting,
    )?;
    let mut warnings = done.applied.warnings.clone();
    warnings.extend(repo.store().warnings());
    let text = if json {
        let document = json!({
            "bolt_json": BOLT_JSON,
            "restored": done.restored,
            "removed": done.removed,
            "folders": done.folders,
            "not_written": done.applied.not_written,
            "placeholders": done.applied.placeholders,
            "files_in_use": done.applied.in_use,
            "safety_copies": done.safety_copies,
            "warnings": output::warnings_json(&warnings),
        });
        format!("{document}\n")
    } else {
        output::discarded(&done, paths.is_empty())
    };
    output::print_done(&text, &warnings)
}

/// `bolt undo`, `bolt redo`, and `bolt undo --to <entry>` (`to`).
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
    let done = match to {
        Some(to) => undo::undo_to(repo, to, &req, on_wait),
        None if redo => undo::redo(repo, &req, on_wait),
        None => undo::undo(repo, &req, on_wait),
    };
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
        let mut text = String::new();
        let mut named = |id: &Id| named_save(repo, id, into);
        for t in &taken {
            if head_lost {
                text += &output::picked_up_line(t, &mut named);
            } else {
                // A step `undo --to` took records only `undo`: say where it went.
                let went = undo::went_to(repo, &t.entry).ok().flatten();
                text += &output::step_line(t, went.as_ref(), &shelved, &mut named);
            }
            text.push('\n');
        }
        for (label, _) in &shelved {
            let label = escape(label, false);
            text += &format!("Set your unsaved changes in the way aside on shelf {label}.\n");
        }
        if finished {
            let what = finished_switch(repo).unwrap_or_else(|| "switch".to_owned());
            text += &match &to {
                Some(to) => format!(
                    "Finished the {what} that was stopped, so nothing more was done. Running \
                     bolt undo --to {to} again goes on from there.\n"
                ),
                None => format!(
                    "Finished the {what} that was stopped, so nothing more was {list}. Running \
                     bolt {command} again takes one more step.\n"
                ),
            };
        }
        match (failed, &to) {
            (true, Some(to)) if !taken.is_empty() => {
                // On the way to an entry the steps are all undos, or all redos.
                let verb = match taken.first().map(|t| &t.action) {
                    Some(Action::Redo(_)) => "Redid",
                    _ => "Undid",
                };
                let k = taken.len();
                let steps = if k == 1 { "step" } else { "steps" };
                text +=
                    &format!("{verb} {k} {steps} on the way to entry {to} before one stopped.\n");
            }
            (true, None) if !taken.is_empty() => {
                let (k, n) = (taken.len(), steps.n);
                text += &format!("{verb} {k} of the {n} steps asked for.\n");
            }
            (false, Some(to)) if !finished && taken.is_empty() => {
                text += &format!(
                    "Nothing changed: things are already as they were right after entry {to}.\n"
                );
            }
            (false, None) if !finished && taken.len() < req.n => {
                let more = if taken.is_empty() { "" } else { " more" };
                text += &format!("Nothing{more} to {command}.\n");
            }
            _ => {}
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
            document["bolt_json"] = BOLT_JSON.into();
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
            output::print_done(&text, &told_once(warnings))
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

/// `bolt shelf`.
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
    let done = shelf::shelf(repo, message, &who, git_excludes.as_deref(), &mut waiting)?;
    let made = match &done {
        Shelved::Made(made) => Some(made),
        Shelved::Nothing { .. } => None,
    };
    let applied = made.map(|d| d.applied.clone()).unwrap_or_default();
    let mut warnings = applied.warnings.clone();
    warnings.extend(repo.store().warnings());
    let text = if json {
        let shelf =
            made.map(|d| json!({"label": d.label, "save": d.save.to_hex(), "files": d.files}));
        let document = json!({
            "bolt_json": BOLT_JSON,
            "shelf": shelf,
            "written": applied.written,
            "removed": applied.removed,
            "not_written": applied.not_written,
            "placeholders": applied.placeholders,
            "files_in_use": applied.in_use,
            "warnings": output::warnings_json(&warnings),
        });
        format!("{document}\n")
    } else {
        match &done {
            Shelved::Made(d) => {
                let files = match d.files {
                    0 => String::new(),
                    1 => " in 1 file".to_owned(),
                    n => format!(" in {n} files"),
                };
                let label = escape(&d.label, false);
                format!(
                    "Set your unsaved changes{files} aside on shelf {label}: bolt unshelf brings \
                     them back.\n"
                )
            }
            // The warnings that follow name what stayed, and say why.
            Shelved::Nothing {
                changed_again: true,
                ..
            } => "Nothing was shelved: your files changed while Bolt was reading them. Run bolt \
                  shelf again.\n"
                .to_owned(),
            Shelved::Nothing { links_left, .. } if !links_left.is_empty() => {
                "Nothing was shelved.\n".to_owned()
            }
            Shelved::Nothing { .. } => {
                "Nothing to shelve: your files match your last save.\n".to_owned()
            }
        }
    };
    output::print_done(&text, &warnings)
}

/// `bolt shelf --list`.
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
            "bolt_json": BOLT_JSON,
            "shelves": shelves,
            "warnings": output::warnings_json(&warnings),
        });
        format!("{document}\n")
    } else if list.is_empty() {
        "No shelves: bolt shelf sets your unsaved changes aside on one.\n".to_owned()
    } else {
        let now = now_ms();
        let width = list
            .iter()
            .map(|s| escape(&s.label, false).chars().count())
            .max()
            .unwrap_or(0);
        list.iter()
            .map(|s| output::shelf_line(s, width, now) + "\n")
            .collect()
    };
    output::print_quietly(&text, &warnings)
}

/// `bolt unshelf`, of the shelf `number` or the newest.
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
    let done = shelf::unshelf(repo, number, &who, git_excludes.as_deref(), &mut waiting)?;
    let mut warnings = done.applied.warnings.clone();
    warnings.extend(repo.store().warnings());
    let text = if json {
        let shelf =
            (!done.finished).then(|| json!({"label": done.label, "save": done.save.to_hex()}));
        let document = json!({
            "bolt_json": BOLT_JSON,
            "finished": done.finished,
            "shelf": shelf,
            "written": done.applied.written,
            "removed": done.applied.removed,
            "carried": done.carried,
            "not_written": done.applied.not_written,
            "placeholders": done.applied.placeholders,
            "files_in_use": done.applied.in_use,
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
                " Running bolt unshelf again brings back the newest shelf left.".to_owned()
            }
            (Some(n), Ok(left)) if left.iter().any(|(label, _)| label == n) => {
                format!(
                    " Running bolt unshelf {} again brings it back.",
                    escape(n, false)
                )
            }
            // The stopped switch may have been the one bringing it back.
            (Some(n), Ok(_)) => format!(
                " There's no shelf {} on the list now: bolt shelf --list shows the shelves there \
                 are.",
                escape(n, false)
            ),
        };
        format!(
            "Finished the {what} that was stopped, so no shelf was brought back this time.{next}\n"
        )
    } else {
        let title = repo
            .store()
            .save(&done.save, repo.signatures_allowed())
            .map(|s| {
                let first = s.message.lines().next().unwrap_or_default().to_owned();
                format!(" \"{}\"", escape(&first, false))
            })
            .unwrap_or_default();
        let count = |n: usize| match n {
            1 => "1 file".to_owned(),
            n => format!("{n} files"),
        };
        let what = match (done.applied.written, done.applied.removed) {
            (0, 0) => "your files already held its changes".to_owned(),
            (0, removed) => format!("{} removed", count(removed)),
            (written, 0) => format!("{} written", count(written)),
            (written, removed) => format!("{} written, {removed} removed", count(written)),
        };
        let label = escape(&done.label, false);
        let mut text = format!("Brought back shelf {label}{title}: {what}.\n");
        match done.carried {
            0 => {}
            1 => text += "Your other unsaved changes in 1 file stayed as they are.\n",
            n => text += &format!("Your other unsaved changes in {n} files stayed as they are.\n"),
        }
        text
    };
    output::print_done(&text, &warnings)
}

/// `bolt tidy`.
fn tidy_command(json: bool, opened: &mut Option<Repository>) -> Result<(), Failure> {
    let repo = opened.insert(with_users_key(Repository::discover(
        &std::env::current_dir()?,
    )?));
    let window = undo_window_days()?;
    let terminal = std::io::stderr().is_terminal();
    let done = tidy::tidy(repo, window, now_ms(), &mut waiting, &mut |to_free| {
        if terminal {
            output::tell(&format!(
                "Tidying: about {} to free",
                output::format_size(to_free)
            ));
        }
    })?;
    let warnings = repo.store().warnings();
    let text = if json {
        let document = json!({
            "bolt_json": BOLT_JSON,
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

/// How many of the steps undo can take `bolt undo --list` shows.
const UNDO_LIST: usize = 20;

/// `bolt undo --list`, and with `--all` (`all`) the whole diary.
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

/// The steps redo can bring back, newest first and numbered for `bolt redo N`, then those undo
/// can take, numbered for `bolt undo N`.
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
            "bolt_json": BOLT_JSON,
            "redo": entries(&list.redo),
            "undo": entries(&list.undo),
            "more": list.more,
            "warnings": output::warnings_json(&warnings),
        });
        writeln!(out, "{document}")?;
        return Ok(warnings);
    }
    let redo = list.redo.iter().enumerate().rev();
    let redo = redo.map(|(i, s)| (format!("undone {}", i + 1), s));
    // Undo can't take the diary's first entry, so it gets no number.
    let undo = list
        .undo
        .iter()
        .enumerate()
        .map(|(i, s)| match s.diary.parent {
            Some(_) => ((i + 1).to_string(), s),
            None => (String::new(), s),
        });
    let shown: Vec<(String, &StepInfo)> = redo.chain(undo).collect();
    let width = shown.iter().map(|(n, _)| n.len()).max().unwrap_or(0);
    let now = now_ms();
    let (mut named, unshortened) = names(repo);
    warnings.extend(unshortened);
    let files = &mut |from, to| files_between(repo, from, to);
    for (n, s) in &shown {
        let what = output::entry_line(&s.diary, &s.effects, &mut named, files);
        let (entry, when) = (s.entry.short_hex(), when(&s.diary, now));
        writeln!(out, "{n:>width$}  {entry}  {when}  {what}")?;
    }
    if list.more {
        writeln!(
            out,
            "and more: bolt undo --list --all shows the whole diary"
        )?;
    }
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
            &format!("{{\"bolt_json\":{BOLT_JSON},\"entries\":["),
            |each| {
                undo::whole_diary(repo, |id, entry, effects| {
                    each(&output::entry_json(&id, entry, effects))
                })
            },
            || {
                warnings.extend(repo.store().warnings());
                (json!({}), told_once(warnings))
            },
        );
    }
    let now = now_ms();
    let (mut named, unshortened) = names(repo);
    warnings.extend(unshortened);
    let files = &mut |from, to| files_between(repo, from, to);
    undo::whole_diary(repo, |id, entry, effects| {
        let what = output::entry_line(entry, effects, &mut named, files);
        writeln!(out, "{}  {}  {what}", id.short_hex(), when(entry, now))?;
        Ok::<(), Failure>(())
    })?;
    warnings.extend(repo.store().warnings());
    Ok(told_once(warnings))
}

/// The entries no other follows, newest first, for a lost `head`: where the diary may have
/// stopped, for `bolt undo --to` to pick one from.
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
            "bolt_json": BOLT_JSON,
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
        for s in &ends {
            let what = output::entry_line(&s.diary, &s.effects, &mut named, files);
            writeln!(
                out,
                "{}  {}  {what}",
                s.entry.short_hex(),
                when(&s.diary, now)
            )?;
        }
        writeln!(
            out,
            "Run bolt undo --to <entry> to make one of them the current one; it changes no file."
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
    output::tell("Waiting for another Bolt command to finish...");
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

fn main() -> ExitCode {
    let cli = Cli::parse();
    // The project a command opens is kept here, so that if the command fails, the error can list
    // what the project's store had noticed by then.
    let mut opened = None;
    let result = match cli.command {
        Command::Start => start(cli.json, &mut opened),
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
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(failure) => {
            let noticed = opened.map_or_else(Vec::new, |repo| repo.store().warnings());
            output::print_error(cli.json, &failure, &noticed);
            ExitCode::from(1)
        }
    }
}
