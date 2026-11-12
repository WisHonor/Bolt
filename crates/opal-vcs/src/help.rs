//! What `opalro help` prints: one screen naming every command, and each command's own page of
//! examples, usage and options. The options come from the command line's own definitions, so a
//! page never lists one the command doesn't take.

use clap::Command;

use crate::style::{self, Role};

/// A command in the overview: its name, what it does, and the command named beside it.
pub type Row = (&'static str, &'static str, Option<&'static str>);

/// The overview's groups, each a title and its rows.
pub const GROUPS: [(&str, &[Row]); 3] = [
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
            ("start", "Make this folder an Opalro project", None),
            ("setup", "See or change the project's rules", None),
            ("import", "Bring in the Git repository in this folder", None),
            ("tidy", "Free the space nothing needs any more", None),
            (
                "verify",
                "Check that everything Opalro stored is intact",
                None,
            ),
        ],
    ),
];

/// The screen `opalro`, `opalro help` and `opalro --help` print.
pub fn overview() -> String {
    let s = style::out();
    let mut text = "Opalro keeps every version of your project: code and big files together.\n\n\
                    Usage: opalro <command> [options]\n"
        .to_owned();
    for (title, commands) in GROUPS {
        text += &format!("\n{}\n", s.paint(Role::Bold, title));
        for (name, does, beside) in commands {
            let beside = beside.map(|b| format!("   (and: {b})")).unwrap_or_default();
            text += &format!("{name:<10}{does}{beside}\n");
        }
    }
    text + "\nHelp on one command: opalro help <command>    Version: opalro --version\n"
}

/// The examples a command's help leads with: each a command line and what it does.
pub fn examples(command: &str) -> &'static [(&'static str, &'static str)] {
    match command {
        "status" => &[
            ("opalro status", "See what changed since your last save"),
            ("opalro status --full", "Read every file to be sure"),
        ],
        "save" => &[
            (
                "opalro save \"Double jump\"",
                "Save every change, new files too",
            ),
            (
                "opalro save \"Fix aim\" Assets/Scripts/Enemy.cs",
                "Save only these files and folders",
            ),
            (
                "opalro save --amend \"Better message\"",
                "Replace your line's newest save, keeping its change ID",
            ),
            (
                "opalro save \"Big import\" --yes",
                "Save without asking, however much it adds",
            ),
            (
                "opalro save --line wall-run \"Try wall running\"",
                "At a save, start the new line wall-run",
            ),
        ],
        "changes" => &[
            ("opalro changes", "Show every unsaved change, line by line"),
            (
                "opalro changes Assets/Scripts",
                "Only in these files and folders",
            ),
            (
                "opalro changes --save main",
                "What the newest save on main changed",
            ),
        ],
        "history" => &[(
            "opalro history",
            "List the saves on your line, newest first",
        )],
        "goto" => &[
            ("opalro goto main", "Go to the line main"),
            ("opalro goto kxqmz", "Go to a save, by its change ID"),
            ("opalro goto main@origin", "Go to a server's line"),
        ],
        "undo" => &[
            ("opalro undo", "Take back your last step"),
            ("opalro undo 3", "Take back the last three steps"),
            (
                "opalro undo --shelve",
                "Set unsaved changes in the way aside first",
            ),
            (
                "opalro undo --list",
                "List the steps you can take back or bring back",
            ),
            (
                "opalro undo --to 3f9a2c7e",
                "Make things as they were after this entry",
            ),
        ],
        "redo" => &[
            ("opalro redo", "Bring back the step undone last"),
            ("opalro redo 2", "Bring back the last two"),
        ],
        "line" => &[("opalro line", "List the lines, marking the one you're on")],
        "shelf" => &[
            ("opalro shelf", "Set your unsaved changes aside"),
            (
                "opalro shelf \"Half-done wall run\"",
                "The same, saying what they are",
            ),
            (
                "opalro shelf --list",
                "List the shelves by number, newest first",
            ),
        ],
        "unshelf" => &[
            ("opalro unshelf", "Bring back the newest shelf"),
            ("opalro unshelf 2", "Bring back shelf 2"),
        ],
        "discard" => &[
            ("opalro discard", "Throw away every unsaved change"),
            (
                "opalro discard Assets/Scripts/Player.cs",
                "Only in these files and folders",
            ),
        ],
        "show" => &[
            ("opalro show", "Show the save you're at"),
            ("opalro show kxqmz", "Show a save, by its change ID"),
        ],
        "start" => &[
            (
                "opalro start",
                "Make this folder a project, asking what kind",
            ),
            (
                "opalro start --preset game",
                "Start it with the Game settings",
            ),
        ],
        "setup" => &[
            ("opalro setup", "Show the project's rules"),
            (
                "opalro setup --preset code",
                "Show what the Code preset changes, then ask",
            ),
            (
                "opalro setup --preset code --yes",
                "Make those changes without asking",
            ),
        ],
        "import" => &[(
            "opalro import",
            "Bring in the Git repository in this folder, with all its history",
        )],
        "tidy" => &[("opalro tidy", "Free the space nothing needs any more")],
        "verify" => &[
            ("opalro verify", "Check everything Opalro has stored"),
            (
                "opalro verify --json",
                "The same, every problem listed for a script",
            ),
        ],
        _ => &[],
    }
}

/// The help page of the command `name`: what it does, its examples, its usage, then what it
/// takes. `None` when Opalro has no such command, as for the command-line reader's own `help`.
pub fn page(root: &mut Command, name: &str) -> Option<String> {
    let examples = examples(name);
    if examples.is_empty() {
        return None;
    }
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
    let mut text = format!("opalro {name}: {about}\n");
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
