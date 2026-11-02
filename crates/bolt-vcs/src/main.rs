//! The `bolt` command-line tool.

mod output;

use std::process::ExitCode;

use bolt_core::config::{ConfigError, load_identity, now_person};
use bolt_core::diff::{ChangeKind, count_changes, for_each_change};
use bolt_core::history::{ResolveError, line_history, resolve, short_change_ids};
use bolt_core::id::Id;
use bolt_core::repo::{RepoError, Repository};
use bolt_core::save::Save;
use bolt_core::snapshot::{SaveError, SaveOutcome, SaveRequest, save_folder};
use bolt_core::store::StoreError;
use bolt_core::text::escape;
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
    /// Save the current state of your files.
    Save { message: Option<String> },
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
        }
    }
}

impl From<ConfigError> for Failure {
    fn from(e: ConfigError) -> Failure {
        Failure {
            hint: e.hint(),
            message: e.to_string(),
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
        }
    }
}

impl From<std::io::Error> for Failure {
    fn from(e: std::io::Error) -> Failure {
        RepoError::from(e).into()
    }
}

/// The shortest unique form of a save's change ID among the line's saves (and this one).
fn short_of(repo: &Repository, id: &Id, save: &Save) -> Result<String, Failure> {
    let mut saves = line_history(repo)?;
    if !saves.iter().any(|(i, _)| i == id) {
        saves.push((*id, save.clone()));
    }
    Ok(short_change_ids(&saves).remove(id).unwrap_or_default())
}

fn start(json: bool) -> Result<(), Failure> {
    let root = std::env::current_dir()?;
    let who = now_person(&load_identity(None)?);
    let repo = Repository::init(&root, &who)?;
    let (_, entry) = repo.current()?;
    let id = entry.state.lines.get("main").ok_or_else(|| Failure {
        message: "The new project has no line main.".into(),
        hint: None,
    })?;
    let save = repo.store().save(id, repo.signatures_allowed())?;
    let short = &short_of(&repo, id, &save)?;
    if json {
        println!(
            "{}",
            json!({
                "bolt_json": BOLT_JSON,
                "root": root.display().to_string(),
                "line": "main",
                "save": output::save_json(id, &save, short),
            })
        );
    } else {
        // The folder's name came from outside Bolt, like any name in a project (RT-8).
        let shown = escape(&root.display().to_string(), false);
        println!("Started a Bolt project in {shown}.");
        println!("Your first save is {short}, on line main.");
        println!("Next: change some files, then run: bolt save \"describe your change\"");
    }
    Ok(())
}

fn save(json: bool, message: Option<String>) -> Result<(), Failure> {
    let message = message.unwrap_or_default();
    if message.trim().is_empty() {
        return Err(Failure {
            message: "A save needs a message.".into(),
            hint: Some("Say what you changed: bolt save \"describe your change\"".into()),
        });
    }
    let mut repo = Repository::discover(&std::env::current_dir()?)?;
    let who = now_person(&load_identity(Some(repo.root()))?);
    let req = SaveRequest {
        message: &message,
        who: &who,
        command: "save",
    };
    let outcome = save_folder(&mut repo, &req, &mut |_| {
        eprintln!("Waiting for another Bolt command to finish...");
    })?;
    match outcome {
        SaveOutcome::Saved {
            save_id,
            save,
            changed_files,
        } => {
            let short = short_of(&repo, &save_id, &save)?;
            if json {
                println!(
                    "{}",
                    json!({
                        "bolt_json": BOLT_JSON,
                        "result": "saved",
                        "save": output::save_json(&save_id, &save, &short),
                        "changed_files": changed_files,
                    })
                );
            } else {
                let title = escape(message.lines().next().unwrap_or_default(), false);
                let counted = match changed_files {
                    Some(1) => "(1 file)".to_owned(),
                    Some(n) => format!("({n} files)"),
                    None => "(couldn't count the changed files)".to_owned(),
                };
                println!("Saved {short}  \"{title}\"  {counted}");
            }
        }
        SaveOutcome::NothingChanged { current } => {
            if json {
                println!(
                    "{}",
                    json!({"bolt_json": BOLT_JSON, "result": "nothing-changed"})
                );
            } else {
                let last = repo.store().save(&current, repo.signatures_allowed())?;
                println!(
                    "Nothing to save: no changes since your last save ({}).",
                    short_of(&repo, &current, &last)?
                );
            }
        }
    }
    Ok(())
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_millis()).ok())
        .unwrap_or(0)
}

fn history(json: bool) -> Result<(), Failure> {
    let repo = Repository::discover(&std::env::current_dir()?)?;
    let saves = line_history(&repo)?;
    let shorts = short_change_ids(&saves);
    if json {
        let line = match &repo.current()?.1.state.position {
            bolt_core::diary::Position::OnLine(name) => json!(name),
            bolt_core::diary::Position::AtSave(_) => json!(null),
        };
        let list: Vec<_> = saves
            .iter()
            .map(|(id, s)| output::save_json(id, s, &shorts[id]))
            .collect();
        println!(
            "{}",
            json!({"bolt_json": BOLT_JSON, "line": line, "saves": list})
        );
    } else {
        let now = now_ms();
        for (id, s) in &saves {
            let when = output::relative_time(s.author.time_ms, s.author.tz_offset_min, now);
            let title = s.message.lines().next().unwrap_or_default();
            println!(
                "{}  {when}  {}  {}",
                shorts[id],
                escape(&s.author.name, false),
                escape(title, false)
            );
        }
    }
    Ok(())
}

fn show(json: bool, text: Option<&str>) -> Result<(), Failure> {
    let repo = Repository::discover(&std::env::current_dir()?)?;
    let id = match text {
        Some(t) => resolve(&repo, t)?,
        None => line_history(&repo)?
            .first()
            .map(|(id, _)| *id)
            .ok_or_else(|| Failure {
                message: "This line has no saves.".into(),
                hint: None,
            })?,
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
    let short = short_of(&repo, &id, &save)?;
    if json {
        // The document `json!` would make, in pieces: serde_json writes an object's keys in
        // alphabetical order, so `changes` comes between `bolt_json` and `save`.
        print!("{{\"bolt_json\":{BOLT_JSON},\"changes\":[");
        let mut comma = "";
        for_each_change(store, old_root, save.root, |c| {
            print!(
                "{comma}{}",
                json!({
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
                })
            );
            comma = ",";
            Ok::<(), Failure>(())
        })?;
        println!("],\"save\":{}}}", output::save_json(&id, &save, &short));
        return Ok(());
    }
    let when = output::relative_time(save.author.time_ms, save.author.tz_offset_min, now_ms());
    println!(
        "Save {short} · {} · {when}",
        escape(&save.author.name, false)
    );
    println!("{}\n", escape(save.message.trim_end(), true));
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
    Ok(())
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.command {
        Command::Start => start(cli.json),
        Command::Save { message } => save(cli.json, message),
        Command::History => history(cli.json),
        Command::Show { save } => show(cli.json, save.as_deref()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(failure) => {
            output::print_error(cli.json, &failure);
            ExitCode::from(1)
        }
    }
}
