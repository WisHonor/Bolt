//! What `bolt help` prints: one screen naming every command, and each command's own page of
//! examples, usage and options. The options come from the command line's own definitions, so a
//! page never lists one the command doesn't take.

use clap::Command;

use crate::style::{self, Role};

/// A command in the overview: its name, what it does, and the command named beside it.
type Row = (&'static str, &'static str, Option<&'static str>);

/// The overview's groups, each a title and its rows.
const GROUPS: [(&str, &[Row]); 3] = [
    (
        "Every day",
        &[
            (
                "status",
                "See what changed since your last save, and where you are",
                None,
            ),
            ("save", "Save your changes, new files too", None),
            ("changes", "Show your unsaved changes line by line", None),
            ("history", "List the saves on your line, newest first", None),
            ("goto", "Go to another line or save", None),
            ("undo", "Take back your last step", Some("redo")),
        ],
    ),
    (
        "Lines, shelves and details",
        &[
            ("line", "List your lines", None),
            ("shelf", "Set your unsaved changes aside", Some("unshelf")),
            (
                "discard",
                "Throw away your unsaved changes; undo brings them back",
                None,
            ),
            ("show", "Show everything about one save", None),
        ],
    ),
    (
        "The project",
        &[
            ("start", "Make this folder a Bolt project", None),
            ("setup", "See or change the project's rules", None),
            ("import", "Bring in the Git repository in this folder", None),
            ("tidy", "Free the space nothing needs any more", None),
            (
                "verify",
                "Check that everything Bolt stored is intact",
                None,
            ),
        ],
    ),
];

/// The screen `bolt`, `bolt help` and `bolt --help` print.
pub fn overview() -> String {
    let s = style::out();
    let mut text = "Bolt keeps every version of your project: code and big files together.\n\n\
                    Usage: bolt <command> [options]\n"
        .to_owned();
    for (title, commands) in GROUPS {
        text += &format!("\n{}\n", s.paint(Role::Bold, title));
        for (name, does, beside) in commands {
            let beside = beside.map(|b| format!("   (and: {b})")).unwrap_or_default();
            text += &format!("{name:<10}{does}{beside}\n");
        }
    }
    text + "\nHelp on one command: bolt help <command>    Version: bolt --version\n"
}

/// The examples a command's help leads with: each a command line and what it does.
pub fn examples(command: &str) -> &'static [(&'static str, &'static str)] {
    match command {
        "status" => &[
            ("bolt status", "See what changed since your last save"),
            ("bolt status --full", "Read every file to be sure"),
        ],
        "save" => &[
            (
                "bolt save \"Double jump\"",
                "Save every change, new files too",
            ),
            (
                "bolt save \"Fix aim\" Assets/Scripts/Enemy.cs",
                "Save only these files and folders",
            ),
            (
                "bolt save --amend \"Better message\"",
                "Replace your line's newest save, keeping its change ID",
            ),
            (
                "bolt save \"Big import\" --yes",
                "Save without asking, however much it adds",
            ),
            (
                "bolt save --line wall-run \"Try wall running\"",
                "At a save, start the new line wall-run",
            ),
        ],
        "changes" => &[
            ("bolt changes", "Show every unsaved change, line by line"),
            (
                "bolt changes Assets/Scripts",
                "Only in these files and folders",
            ),
            (
                "bolt changes --save main",
                "What the newest save on main changed",
            ),
        ],
        "history" => &[("bolt history", "List the saves on your line, newest first")],
        "goto" => &[
            ("bolt goto main", "Go to the line main"),
            ("bolt goto kxqmz", "Go to a save, by its change ID"),
            ("bolt goto main@origin", "Go to a server's line"),
        ],
        "undo" => &[
            ("bolt undo", "Take back your last step"),
            ("bolt undo 3", "Take back the last three steps"),
            (
                "bolt undo --shelve",
                "Set unsaved changes in the way aside first",
            ),
            (
                "bolt undo --list",
                "List the steps you can take back or bring back",
            ),
            (
                "bolt undo --to 3f9a2c7e",
                "Make things as they were after this entry",
            ),
        ],
        "redo" => &[
            ("bolt redo", "Bring back the step undone last"),
            ("bolt redo 2", "Bring back the last two"),
        ],
        "line" => &[("bolt line", "List the lines, marking the one you're on")],
        "shelf" => &[
            ("bolt shelf", "Set your unsaved changes aside"),
            (
                "bolt shelf \"Half-done wall run\"",
                "The same, saying what they are",
            ),
            (
                "bolt shelf --list",
                "List the shelves by number, newest first",
            ),
        ],
        "unshelf" => &[
            ("bolt unshelf", "Bring back the newest shelf"),
            ("bolt unshelf 2", "Bring back shelf 2"),
        ],
        "discard" => &[
            ("bolt discard", "Throw away every unsaved change"),
            (
                "bolt discard Assets/Scripts/Player.cs",
                "Only in these files and folders",
            ),
        ],
        "show" => &[
            ("bolt show", "Show the save you're at"),
            ("bolt show kxqmz", "Show a save, by its change ID"),
        ],
        "start" => &[
            ("bolt start", "Make this folder a project, asking what kind"),
            (
                "bolt start --preset game",
                "Start it with the Game settings",
            ),
        ],
        "setup" => &[
            ("bolt setup", "Show the project's rules"),
            (
                "bolt setup --preset code",
                "Show what the Code preset changes, then ask",
            ),
            (
                "bolt setup --preset code --yes",
                "Make those changes without asking",
            ),
        ],
        "import" => &[(
            "bolt import",
            "Bring in the Git repository in this folder, with all its history",
        )],
        "tidy" => &[("bolt tidy", "Free the space nothing needs any more")],
        "verify" => &[
            ("bolt verify", "Check everything Bolt has stored"),
            (
                "bolt verify --json",
                "The same, every problem listed for a script",
            ),
        ],
        _ => &[],
    }
}

/// The help page of the command `name`: what it does, its examples, its usage, then what it
/// takes. `None` when Bolt has no such command.
pub fn page(root: &mut Command, name: &str) -> Option<String> {
    root.build();
    let command = root.find_subcommand(name)?;
    let s = style::out();
    let about = command
        .get_about()
        .map(ToString::to_string)
        .unwrap_or_default();
    let mut first = about.chars();
    let about: String = first
        .next()
        .map(|c| c.to_lowercase().chain(first).collect())
        .unwrap_or_default();
    let mut text = format!("bolt {name}: {about}\n");
    let examples = examples(name);
    let wide = examples.iter().map(|(line, _)| style::columns(line)).max();
    text += &format!("\n{}\n", s.paint(Role::Bold, "Examples"));
    for (line, does) in examples {
        text += &format!("{line:<0$}  {does}\n", wide.unwrap_or(0));
    }
    let usage = command.clone().render_usage().to_string();
    let usage = usage.strip_prefix("Usage: ").unwrap_or(&usage);
    text += &format!("\n{}\n{usage}\n", s.paint(Role::Bold, "Usage"));
    for (title, tag) in [("Arguments", "{positionals}"), ("Options", "{options}")] {
        let rendered = command
            .clone()
            .help_template(tag)
            .term_width(0)
            .render_help()
            .to_string();
        let lines: Vec<&str> = rendered
            .lines()
            .map(|l| l.strip_prefix("  ").unwrap_or(l).trim_end())
            .filter(|l| !l.is_empty())
            .collect();
        if !lines.is_empty() {
            text += &format!("\n{}\n{}\n", s.paint(Role::Bold, title), lines.join("\n"));
        }
    }
    Some(text)
}
