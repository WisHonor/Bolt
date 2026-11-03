//! The `bolt` command-line tool.

mod import_output;
mod output;

use std::process::ExitCode;

use bolt_core::config::{
    ConfigError, Identity, ignored_identity_note, ignored_project_identity, load_identity,
    now_person,
};
use bolt_core::diary::Position;
use bolt_core::diff::{ChangeKind, count_changes, for_each_change};
use bolt_core::fsio::fault_point;
use bolt_core::history::{ResolveError, current_save, for_each_on_line, resolve, short_change_id};
use bolt_core::id::{ChangeId, Id};
use bolt_core::import::{ImportError, ImportRequest, import_git};
use bolt_core::repo::{RepoError, Repository, elevated_only_note};
use bolt_core::save::Save;
use bolt_core::snapshot::{SaveError, SaveOutcome, SaveRequest, save_folder};
use bolt_core::store::StoreError;
use bolt_core::text::escape;
use bolt_core::warning::{self, Warning};
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
    /// Save the current state of your files.
    Save {
        message: Option<String>,
        /// After an import, save files Git wrote with CRLF line endings as they are.
        #[arg(long)]
        accept_crlf: bool,
    },
    /// List the saves on this line.
    History,
    /// Show one save.
    Show { save: Option<String> },
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
        let hint = match &e {
            ResolveError::NotFound(_) => Some("See your saves with: bolt history".to_owned()),
            ResolveError::Ambiguous { .. } => Some("Type more of the ID to pick one.".to_owned()),
            ResolveError::Repo(r) => r.hint(),
        };
        Failure {
            hint,
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

fn start(json: bool, opened: &mut Option<Repository>) -> Result<(), Failure> {
    let root = std::env::current_dir()?;
    // The user's own settings may name this folder, so the first save is theirs too.
    let who = now_person(&load_identity(Some(&root))?);
    let repo: &Repository = opened.insert(Repository::init(&root, &who)?);
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
    if json {
        let save = first.as_ref().map_or(json!(null), |(id, save, short)| {
            output::save_json(id, save, short)
        });
        println!(
            "{}",
            json!({
                "bolt_json": BOLT_JSON,
                "root": root.display().to_string(),
                "line": "main",
                "save": save,
                "warnings": output::warnings_json(&warnings),
            })
        );
    } else {
        // The folder's name came from outside Bolt, like any name in a project.
        let shown = escape(&root.display().to_string(), false);
        println!("Started a Bolt project in {shown}.");
        if let Some((_, _, short)) = &first {
            println!("Your first save is {short}, on line main.");
            println!("Next: change some files, then run: bolt save \"describe your change\"");
        }
    }
    output::print_warnings(&warnings);
    Ok(())
}

fn import(json: bool, opened: &mut Option<Repository>) -> Result<(), Failure> {
    let root = std::env::current_dir()?;
    let who = now_person(&load_identity(Some(&root))?);
    let mut progress = output::Progress::new("Importing commits");
    let imported = import_git(
        &root,
        &mut ImportRequest {
            who: &who,
            command: "import",
            progress: &mut |done, total| progress.update(done, total),
        },
        opened,
    );
    progress.finish();
    let report = imported?;
    // A project made from an import opens only where one made by `start` would.
    let mut warnings: Vec<Warning> = elevated_only_note(&root)
        .map(|note| Warning::new(warning::ELEVATED_ONLY, note))
        .into_iter()
        .collect();
    warnings.extend(report.warnings.iter().cloned());
    if json {
        println!("{}", import_output::import_json(&report, &warnings));
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
                format!("You're at save {short}, where Git's HEAD was detached.")
            }
        };
        import_output::print_import(&report, &standing);
    }
    output::print_warnings(&warnings);
    Ok(())
}

fn save(
    json: bool,
    message: Option<String>,
    accept_crlf: bool,
    opened: &mut Option<Repository>,
) -> Result<(), Failure> {
    let message = message.unwrap_or_default();
    if message.trim().is_empty() {
        return Err(Failure {
            message: "A save needs a message.".into(),
            hint: Some("Say what you changed: bolt save \"describe your change\"".into()),
            printed: false,
        });
    }
    let repo = opened.insert(Repository::discover(&std::env::current_dir()?)?);
    let who = now_person(&load_identity(Some(repo.root()))?);
    // A project's own settings never say who the user is; if they try, the user is told.
    let ignored = ignored_project_identity(repo.root());
    let req = SaveRequest {
        message: &message,
        who: &who,
        command: "save",
        accept_crlf,
    };
    let outcome = save_folder(repo, &req, &mut |_| {
        eprintln!("Waiting for another Bolt command to finish...");
    })?;
    let warnings = match outcome {
        SaveOutcome::Saved {
            save_id,
            save,
            changed_files,
            short_change_id: short,
            mut warnings,
        } => {
            // The save is made, so nothing from here on may fail the command.
            warnings.extend(repo.store().warnings());
            if json {
                let document = json!({
                    "bolt_json": BOLT_JSON,
                    "result": "saved",
                    "save": output::save_json(&save_id, &save, &short),
                    "changed_files": changed_files,
                    "warnings": output::warnings_json(&warnings),
                });
                println!("{}", with_ignored(document, ignored.as_ref()));
            } else {
                let title = escape(message.lines().next().unwrap_or_default(), false);
                let counted = match changed_files {
                    Some(1) => "(1 file)".to_owned(),
                    Some(n) => format!("({n} files)"),
                    None => "(couldn't count the changed files)".to_owned(),
                };
                println!("Saved {short}  \"{title}\"  {counted}");
            }
            warnings
        }
        SaveOutcome::NothingChanged {
            current,
            mut warnings,
        } => {
            if json {
                warnings.extend(repo.store().warnings());
                let document = json!({
                    "bolt_json": BOLT_JSON,
                    "result": "nothing-changed",
                    "warnings": output::warnings_json(&warnings),
                });
                println!("{}", with_ignored(document, ignored.as_ref()));
            } else {
                let last = repo.store().save(&current, repo.signatures_allowed())?;
                let short = short_or_whole(repo, &current, &last.change_id, &mut warnings);
                warnings.extend(repo.store().warnings());
                println!("Nothing to save: no changes since your last save ({short}).");
            }
            warnings
        }
    };
    if !json && let Some(ignored) = &ignored {
        println!("{}", ignored_identity_note(ignored, repo.root()));
    }
    output::print_warnings(&warnings);
    Ok(())
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
        warnings.push(Warning::new(
            warning::CHANGE_ID_NOT_SHORTENED,
            format!(
                "Bolt couldn't read this line's older saves to shorten the change ID. {problem}"
            ),
        ));
        change_id.to_letters()
    })
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

fn history(json: bool, opened: &mut Option<Repository>) -> Result<(), Failure> {
    let repo: &Repository = opened.insert(Repository::discover(&std::env::current_dir()?)?);
    if json {
        let line = match &repo.current()?.1.state.position {
            bolt_core::diary::Position::OnLine(name) => json!(name),
            bolt_core::diary::Position::AtSave(_) => json!(null),
        };
        // The document `json!` would make, in pieces and in alphabetical key order, as in `show`.
        let warnings = output::stream_document(
            &mut std::io::stdout(),
            &format!("{{\"bolt_json\":{BOLT_JSON},\"line\":{line},\"saves\":["),
            |each| {
                for_each_on_line(repo, |id, save, short| {
                    each(&output::save_json(id, save, short))
                })
            },
            || (json!({}), repo.store().warnings()),
        )?;
        output::print_warnings(&warnings);
        return Ok(());
    }
    let now = now_ms();
    for_each_on_line(repo, |_, s, short| {
        let when = output::relative_time(s.author.time_ms, s.author.tz_offset_min, now);
        let title = s.message.lines().next().unwrap_or_default();
        println!(
            "{short}  {when}  {}  {}",
            escape(&s.author.name, false),
            escape(title, false)
        );
        Ok::<(), Failure>(())
    })?;
    output::print_warnings(&repo.store().warnings());
    Ok(())
}

fn show(json: bool, text: Option<&str>, opened: &mut Option<Repository>) -> Result<(), Failure> {
    let repo: &Repository = opened.insert(Repository::discover(&std::env::current_dir()?)?);
    let id = match text {
        Some(t) => resolve(repo, t)?,
        None => current_save(repo)?,
    };
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
    if json {
        // The document `json!` would make, in pieces: serde_json writes an object's keys in
        // alphabetical order, so `changes` comes between `bolt_json` and `save`.
        let warnings = output::stream_document(
            &mut std::io::stdout(),
            &format!("{{\"bolt_json\":{BOLT_JSON},\"changes\":["),
            |each| {
                for_each_change(store, old_root, save.root, |c| {
                    each(&json!({
                        "path": c.path,
                        "change": match c.change {
                            ChangeKind::Added => "added",
                            ChangeKind::Removed => "removed",
                            ChangeKind::Modified => "modified",
                            ChangeKind::KindChanged => "kind-changed",
                        },
                        "kind": c.kind.as_str(),
                        "old_size": c.old_size,
                        "new_size": c.new_size,
                    }))
                })
            },
            || {
                // Shortened after the walk: if it fails first, only the store has warnings to give.
                let mut warnings = Vec::new();
                let short = short_or_whole(repo, &id, &save.change_id, &mut warnings);
                warnings.extend(store.warnings());
                (
                    json!({"save": output::save_json(&id, &save, &short)}),
                    warnings,
                )
            },
        )?;
        output::print_warnings(&warnings);
        return Ok(());
    }
    let mut warnings = Vec::new();
    let short = short_or_whole(repo, &id, &save.change_id, &mut warnings);
    let when = output::relative_time(save.author.time_ms, save.author.tz_offset_min, now_ms());
    println!(
        "Save {short} · {} · {when}",
        escape(&save.author.name, false)
    );
    // Indented, as Git does, so that no line of the message can pass for a change below it.
    for line in escape(save.message.trim_end(), true).lines() {
        if line.trim().is_empty() {
            println!();
        } else {
            println!("    {line}");
        }
    }
    println!();
    let size = |s: Option<u64>| output::format_size(s.unwrap_or(0));
    for_each_change(store, old_root, save.root, |c| {
        let p = &escape(c.path, false);
        match c.change {
            ChangeKind::Added => println!("  added     {p}  ({})", size(c.new_size)),
            ChangeKind::Removed => println!("  removed   {p}  ({})", size(c.old_size)),
            ChangeKind::Modified => {
                println!(
                    "  modified  {p}  ({} → {})",
                    size(c.old_size),
                    size(c.new_size)
                );
            }
            ChangeKind::KindChanged => {
                let old = c.old_kind.map_or("none", |k| k.as_str());
                println!("  changed   {p}  ({old} → {})", c.kind.as_str());
            }
        }
        Ok::<(), Failure>(())
    })?;
    if count == 0 {
        println!("  (no file changes)");
    }
    warnings.extend(store.warnings());
    output::print_warnings(&warnings);
    Ok(())
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    // The project a command opens is kept here, so that if the command fails, the error can list
    // what the project's store had noticed by then.
    let mut opened = None;
    let result = match cli.command {
        Command::Start => start(cli.json, &mut opened),
        Command::Import => import(cli.json, &mut opened),
        Command::Save {
            message,
            accept_crlf,
        } => save(cli.json, message, accept_crlf, &mut opened),
        Command::History => history(cli.json, &mut opened),
        Command::Show { save } => show(cli.json, save.as_deref(), &mut opened),
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
