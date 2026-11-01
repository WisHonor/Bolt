//! The `bolt` command-line tool.

mod output;

use std::process::ExitCode;

use bolt_core::config::{ConfigError, load_identity, now_person};
use bolt_core::repo::{RepoError, Repository};
use bolt_core::save::Save;
use bolt_core::snapshot::{SaveError, SaveOutcome, SaveRequest, save_folder};
use bolt_core::store::StoreError;
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

impl From<std::io::Error> for Failure {
    fn from(e: std::io::Error) -> Failure {
        RepoError::from(e).into()
    }
}

/// The short form of a save's change ID.
fn short_id(save: &Save) -> String {
    // shortcut: 4 letters may not be unique; Task 12 adds the shortest-unique-prefix rule.
    save.change_id.to_letters().chars().take(4).collect()
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
    let short = &short_id(&save);
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
        println!("Started a Bolt project in {}.", root.display());
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
            let short = short_id(&save);
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
                let title = message.lines().next().unwrap_or_default();
                let files = if changed_files == 1 { "file" } else { "files" };
                println!("Saved {short}  \"{title}\"  ({changed_files} {files})");
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
                    short_id(&last)
                );
            }
        }
    }
    Ok(())
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.command {
        Command::Start => start(cli.json),
        Command::Save { message } => save(cli.json, message),
        Command::History | Command::Show { .. } => Err(Failure {
            message: "This command is not ready yet.".into(),
            hint: None,
        }),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(failure) => {
            output::print_error(cli.json, &failure);
            ExitCode::from(1)
        }
    }
}
