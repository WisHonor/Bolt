//! Git's words: the few Opalro runs as its own commands, the ones it explains instead, and the hint
//! that names Opalro's word the first time one is used.

use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::hash::{BuildHasher, RandomState};
use std::io::{self, Read, Write};
use std::path::Path;

use crate::output::tell;
use crate::style::{self, Role};

/// What the command line means.
pub enum Typed {
    /// Nothing of Git's.
    Opalro(Vec<OsString>),
    /// Run `args`; `word` is the Git word that was typed.
    Shortcut {
        word: &'static str,
        args: Vec<OsString>,
    },
    /// Say this on stderr, and end with exit code 2.
    Explain(&'static str),
}

/// The words that run an Opalro command, which are the only ones `git-words` may list.
const SHORTCUTS: [&str; 10] = [
    "init", "commit", "diff", "log", "checkout", "switch", "stash", "branch", "fsck", "gc",
];

/// `save`'s options that take a value, which travels with them when `commit` is translated.
pub const SAVE_VALUE_OPTIONS: [&str; 1] = ["--line"];

/// The user's file of words already explained, beside their `config.toml`.
const FILE: &str = "git-words";
/// The most of `git-words` read.
const MAX_BYTES: u64 = 4096;

const ADD: &str = "Opalro has no staging step: `opalro save` saves every change, new files too.\n\
                   To save only some files, name them: `opalro save \"message\" <paths>`";
const RM_MV: &str = "Delete or move the file itself, as you would any file.\n\
                     The next `opalro save` records it.";
const RESET: &str = "To take back the last command: `opalro undo`\n\
                     To throw away unsaved changes: `opalro discard`\n\
                     To go to an older save: `opalro goto <save>`";
const RESTORE: &str = "`opalro discard <paths>` throws away the unsaved changes in those paths,\n\
                       new files included, and `opalro undo` brings them back.";
const COMMIT: &str = "To save every change: `opalro save \"message\"`\n\
                      To save only some files and folders: `opalro save \"message\" <paths>`";
const NEW_LINE: &str = "`opalro line new <name>` makes a new line where you are and moves onto it.\n\
                        To start it somewhere else, go there first: `opalro goto <save>`";
const STASH: &str = "`opalro shelf` sets your unsaved changes aside, new files too.\n\
                     `opalro unshelf` brings the newest back, `opalro unshelf <number>` another.\n\
                     `opalro shelf --list` lists them by that number, which stays with its shelf.";
const LATER: &str = "Combining lines and seeing who changed each line arrive later in v0.2.\n\
                     Until then, `opalro history` lists the saves on your line.";
const SYNC: &str = "Syncing with a server arrives in Opalro v0.3.\n\
                    Until then, `opalro import` brings a Git repository here into Opalro.";
const REVERT: &str = "Opalro's word for this is `opalro reverse <save>`, which arrives in v0.3.\n\
                      Until then, `opalro undo` takes back your own last step.";
const CLEAN: &str = "Opalro never deletes files it doesn't keep.\n\
                     `opalro tidy` frees space inside .opalro only.";

/// What the command line (without the program's name) means. Only the first argument that
/// isn't `--json` is looked at as a word; a shortcut runs only in the forms that mean in Opalro
/// what they mean in Git.
pub fn translate(mut args: Vec<OsString>) -> Typed {
    let Some(at) = args.iter().position(|a| a != "--json") else {
        return Typed::Opalro(args);
    };
    let rest = args.split_off(at + 1);
    let Some(word) = args.pop() else {
        return Typed::Opalro(rest);
    };
    let before = args;
    let run = |word: &'static str, opalro: &[&str], rest: Vec<OsString>| Typed::Shortcut {
        word,
        args: before
            .iter()
            .cloned()
            .chain(opalro.iter().map(OsString::from))
            .chain(rest)
            .collect(),
    };
    let given = || rest.iter().filter(|a| *a != "--json");
    match word.to_str().unwrap_or_default() {
        "init" => run("init", &["start"], rest),
        "commit" => match commit_args(rest) {
            Some(rest) => run("commit", &["save"], rest),
            None => Typed::Explain(COMMIT),
        },
        "diff" => run("diff", &["changes"], rest),
        "log" => run("log", &["history"], rest),
        "checkout" if given().any(|a| a.to_string_lossy().starts_with("-b")) => {
            new_line("checkout", &["-b"], rest, run)
        }
        "switch"
            if given().any(|a| {
                let a = a.to_string_lossy();
                a.starts_with("-c") || a.starts_with("--create")
            }) =>
        {
            new_line("switch", &["-c", "--create"], rest, run)
        }
        "checkout" => run("checkout", &["goto"], rest),
        "switch" => run("switch", &["goto"], rest),
        "stash" => stash(rest, run),
        // An option goes to `line`, which answers in its own words.
        "branch" if given().next().is_some_and(|a| !is_option(a)) => Typed::Explain(NEW_LINE),
        "branch" => branch(rest, run),
        "tag" => {
            let mut rest = rest;
            // Git's `-d` is Opalro's `--delete`; the rest is Opalro's own word as typed.
            if let Some(d) = rest.iter_mut().find(|a| *a != "--json")
                && d == "-d"
            {
                *d = OsString::from("--delete");
            }
            Typed::Opalro(before.into_iter().chain([word]).chain(rest).collect())
        }
        "fsck" => run("fsck", &["verify"], rest),
        "gc" => run("gc", &["tidy"], rest),
        "add" => Typed::Explain(ADD),
        "restore" => Typed::Explain(RESTORE),
        "rm" | "mv" => Typed::Explain(RM_MV),
        "reset" => Typed::Explain(RESET),
        "merge" | "rebase" | "cherry-pick" | "blame" => Typed::Explain(LATER),
        "clone" | "fetch" | "pull" | "push" | "remote" => Typed::Explain(SYNC),
        "revert" => Typed::Explain(REVERT),
        "clean" => Typed::Explain(CLEAN),
        _ => Typed::Opalro(before.into_iter().chain([word]).chain(rest).collect()),
    }
}

fn is_option(a: &OsString) -> bool {
    a.to_string_lossy().starts_with('-') && a != "-"
}

/// `checkout -b <name>` and `switch -c <name>` run `line new`, with `--json` anywhere; any other
/// form of them, as with a starting point, is explained.
fn new_line(
    word: &'static str,
    flags: &[&str],
    mut rest: Vec<OsString>,
    run: impl Fn(&'static str, &[&str], Vec<OsString>) -> Typed,
) -> Typed {
    let words: Vec<&OsString> = rest.iter().filter(|a| *a != "--json").collect();
    let [flag, name] = words.as_slice() else {
        return Typed::Explain(NEW_LINE);
    };
    if !flags.iter().any(|f| *flag == f) || is_option(name) {
        return Typed::Explain(NEW_LINE);
    }
    rest.retain(|a| !flags.iter().any(|f| a == f));
    run(word, &["line", "new"], rest)
}

/// `branch -m [<old>] <new>`, `branch -d <name>` and `branch -D <name>` run `line rename` and
/// `line delete`, with `--json` anywhere; any other option goes to `line` as typed.
fn branch(
    mut rest: Vec<OsString>,
    run: impl Fn(&'static str, &[&str], Vec<OsString>) -> Typed,
) -> Typed {
    let words: Vec<&OsString> = rest.iter().filter(|a| *a != "--json").collect();
    let Some((flag, names)) = words.split_first() else {
        return run("branch", &["line"], rest);
    };
    let names_only = names.iter().all(|n| !is_option(n));
    let (opalro, yes): (&[&str], bool) = match flag.to_str().unwrap_or_default() {
        "-m" | "--move" if (1..=2).contains(&names.len()) && names_only => {
            (&["line", "rename"], false)
        }
        "-d" | "--delete" if names.len() == 1 && names_only => (&["line", "delete"], false),
        "-D" if names.len() == 1 && names_only => (&["line", "delete"], true),
        _ => return run("branch", &["line"], rest),
    };
    if let Some(at) = rest.iter().position(|a| a != "--json") {
        rest.remove(at);
    }
    rest.extend(yes.then(|| OsString::from("--yes")));
    run("branch", opalro, rest)
}

/// `stash`, `stash push`, `stash pop [number]` and `stash list` run Opalro's; anything else is
/// explained. `--json` anywhere comes along.
fn stash(
    rest: Vec<OsString>,
    run: impl Fn(&'static str, &[&str], Vec<OsString>) -> Typed,
) -> Typed {
    let (json, words): (Vec<OsString>, Vec<OsString>) =
        rest.into_iter().partition(|a| a == "--json");
    let words: Vec<&str> = words.iter().map(|w| w.to_str().unwrap_or("-")).collect();
    let (opalro, number): (&[&str], _) = match words.as_slice() {
        [] | ["push"] => (&["shelf"], None),
        ["pop"] => (&["unshelf"], None),
        ["pop", n] if !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()) => {
            (&["unshelf"], Some(OsString::from(*n)))
        }
        ["list"] => (&["shelf", "--list"], None),
        _ => return Typed::Explain(STASH),
    };
    run("stash", opalro, number.into_iter().chain(json).collect())
}

/// `commit`'s arguments as `save` takes them, or `None` where `commit` is explained instead.
/// With a message (`-m`, `--message`, `--message=`, `-am`): the options, then `--`, the message
/// and the other positionals as typed; `-a` and `--all` go, as save saves every change anyway.
/// Without one, only options may be given, and they go on as typed.
fn commit_args(rest: Vec<OsString>) -> Option<Vec<OsString>> {
    let mut message = None;
    let (mut options, mut positionals) = (Vec::new(), Vec::new());
    let mut paths_only = false;
    let mut args = rest.into_iter();
    while let Some(a) = args.next() {
        if paths_only {
            positionals.push(a);
        } else if a == "--" {
            paths_only = true;
        } else if message.is_none() && (a == "-m" || a == "--message" || a == "-am") {
            match args.next() {
                Some(m) => message = Some(m),
                None => options.push(a),
            }
        } else if let Some(m) = a
            .to_str()
            .and_then(|s| s.strip_prefix("--message="))
            .filter(|_| message.is_none())
        {
            message = Some(OsString::from(m));
        } else if SAVE_VALUE_OPTIONS.iter().any(|o| a == *o) {
            options.push(a);
            options.extend(args.next());
        } else if is_option(&a) {
            options.push(a);
        } else {
            positionals.push(a);
        }
    }
    let Some(message) = message else {
        return positionals.is_empty().then_some(options);
    };
    let options = options.into_iter().filter(|a| a != "-a" && a != "--all");
    Some(
        options
            .chain([OsString::from("--"), message])
            .chain(positionals)
            .collect(),
    )
}

/// The hint the first use of a shortcut word gets: Opalro's word, and an example.
pub fn hint(word: &str) -> Option<&'static str> {
    Some(match word {
        "init" => "in Opalro, init is start: opalro start",
        "commit" => "in Opalro, commit is save; it takes new files too: opalro save \"message\"",
        "diff" => "in Opalro, diff is changes: opalro changes",
        "log" => "in Opalro, log is history: opalro history",
        "checkout" => {
            "in Opalro, checkout is goto: opalro goto <line or save>\n\
             With -b, it is `opalro line new <name>`."
        }
        "switch" => {
            "in Opalro, switch is goto: opalro goto <line>\n\
             With -c, it is `opalro line new <name>`."
        }
        "stash" => {
            "in Opalro, stash is shelf, which takes new files too: opalro shelf\n\
             A shelf keeps the number `opalro shelf --list` shows, counted from 1."
        }
        "branch" => {
            "in Opalro, branch is line: opalro line\n\
             With -d, it is `opalro line delete`; with -m, `opalro line rename`."
        }
        "fsck" => "in Opalro, fsck is verify: opalro verify",
        "gc" => "in Opalro, gc is tidy: opalro tidy",
        _ => return None,
    })
}

/// Prints an explanation on stderr, wrapped at words on a narrow terminal, its commands in the
/// command colour.
pub fn explain(text: &str) {
    let e = style::err();
    // Commands are between backquotes: every second piece, each kept whole by the wrap.
    let painted: String = text
        .split('`')
        .enumerate()
        .map(|(n, piece)| match n % 2 {
            1 => format!("`{}`", e.command(piece)),
            _ => piece.to_owned(),
        })
        .collect();
    tell(&e.indented(0, &painted));
}

/// After a shortcut ran: its hint, once per person, only when lines for people are printed.
pub fn teach(word: &str) {
    if !style::for_people() {
        return;
    }
    let Some(hint) = hint(word) else {
        return;
    };
    let config = opal_core::config::user_config_path();
    if !first_time(word, config.as_deref().and_then(Path::parent)) {
        return;
    }
    let e = style::err();
    // The command ends the first line; a line after it says more, its commands between
    // backquotes, which are painted instead of printed.
    let (first, more) = hint.split_once('\n').unwrap_or((hint, ""));
    let (said, command) = first.split_once(": ").unwrap_or((first, ""));
    let mut message = format!("{said}: {}", e.command(command));
    if !more.is_empty() {
        let more: String = more
            .split('`')
            .enumerate()
            .map(|(n, piece)| match n % 2 {
                1 => e.command(piece),
                _ => piece.to_owned(),
            })
            .collect();
        message = format!("{message}\n{more}");
    }
    tell("");
    tell(&e.prefixed(Role::Hint, "hint:", &message));
}

/// True the first time `word` is met for this user; then remembers it in `git-words`. With no
/// settings folder, or a file Opalro can't read or write, it is the first time again.
pub fn first_time(word: &str, settings_folder: Option<&Path>) -> bool {
    let Some(folder) = settings_folder else {
        return true;
    };
    let path = folder.join(FILE);
    let mut known = known_words(&path);
    if known.contains(&word) {
        return false;
    }
    let Some(word) = SHORTCUTS.iter().find(|w| **w == word) else {
        return true;
    };
    known.push(word);
    // Not being able to remember only means the hint shows again.
    let _ = remember(folder, &path, &known);
    true
}

/// The shortcut words `git-words` lists, each once. Only whole lines count.
fn known_words(path: &Path) -> Vec<&'static str> {
    let Ok(bytes) = read_capped(path) else {
        return Vec::new();
    };
    let mut lines: Vec<&[u8]> = bytes.split(|b| *b == b'\n').collect();
    // What follows the last LF is no whole line.
    lines.pop();
    let mut known = Vec::new();
    for line in lines {
        if let Some(word) = SHORTCUTS.iter().find(|w| w.as_bytes() == line)
            && !known.contains(word)
        {
            known.push(*word);
        }
    }
    known
}

/// The first `MAX_BYTES` of the user's own file, following a link as for their `config.toml`.
fn read_capped(path: &Path) -> io::Result<Vec<u8>> {
    // Looked at first: opening a named pipe would wait for a writer.
    if !fs::metadata(path)?.is_file() {
        return Err(io::ErrorKind::InvalidInput.into());
    }
    let mut bytes = Vec::new();
    File::open(path)?.take(MAX_BYTES).read_to_end(&mut bytes)?;
    Ok(bytes)
}

/// Writes `words` as the whole file, through a file of Opalro's own name and a rename.
fn remember(folder: &Path, path: &Path, words: &[&str]) -> io::Result<()> {
    fs::create_dir_all(folder)?;
    let temp = folder.join(format!("tmp-{:016x}", RandomState::new().hash_one(())));
    let written = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)
        .and_then(|mut file| {
            let lines: String = words.iter().map(|w| format!("{w}\n")).collect();
            file.write_all(lines.as_bytes())
        })
        .and_then(|()| fs::rename(&temp, path));
    if written.is_err() {
        let _ = fs::remove_file(&temp);
    }
    written
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;

    fn typed(line: &str) -> Typed {
        translate(line.split(' ').map(OsString::from).collect())
    }

    fn runs(line: &str) -> (String, String) {
        match typed(line) {
            Typed::Shortcut { word, args } => (
                word.to_owned(),
                args.iter()
                    .map(|a| a.to_string_lossy().into_owned())
                    .collect::<Vec<_>>()
                    .join(" "),
            ),
            _ => panic!("{line} is no shortcut"),
        }
    }

    fn explained(line: &str) -> &'static str {
        match typed(line) {
            Typed::Explain(text) => text,
            _ => panic!("{line} isn't explained"),
        }
    }

    const EXPLANATIONS: [&str; 11] = [
        ADD, RM_MV, RESET, RESTORE, COMMIT, NEW_LINE, STASH, LATER, SYNC, REVERT, CLEAN,
    ];

    #[test]
    fn each_row_of_the_table_runs_its_opal_command() {
        for (typed, word, opalro) in [
            ("init", "init", "start"),
            ("diff a.txt", "diff", "changes a.txt"),
            ("log", "log", "history"),
            ("checkout main", "checkout", "goto main"),
            ("switch main", "switch", "goto main"),
            ("stash", "stash", "shelf"),
            ("stash push", "stash", "shelf"),
            ("stash pop", "stash", "unshelf"),
            ("stash pop 2", "stash", "unshelf 2"),
            ("stash list", "stash", "shelf --list"),
            ("checkout -b side", "checkout", "line new side"),
            ("switch -c side", "switch", "line new side"),
            ("switch --create side", "switch", "line new side"),
            (
                "checkout --json -b side",
                "checkout",
                "line new --json side",
            ),
            ("branch", "branch", "line"),
            ("branch -m side", "branch", "line rename side"),
            ("branch -m main side", "branch", "line rename main side"),
            ("branch --move main side", "branch", "line rename main side"),
            ("branch -d side", "branch", "line delete side"),
            ("branch --delete side", "branch", "line delete side"),
            ("branch -D side", "branch", "line delete side --yes"),
            (
                "branch -D side --json",
                "branch",
                "line delete side --json --yes",
            ),
            // Any other form goes to `line` as typed, which answers in its own words.
            ("branch -a", "branch", "line -a"),
            ("branch -d", "branch", "line -d"),
            ("branch -d a b", "branch", "line -d a b"),
            ("branch -m a b c", "branch", "line -m a b c"),
            ("branch -D side --force", "branch", "line -D side --force"),
            ("fsck", "fsck", "verify"),
            ("gc", "gc", "tidy"),
        ] {
            assert_eq!(runs(typed), (word.to_owned(), opalro.to_owned()), "{typed}");
        }
    }

    #[test]
    fn commit_runs_save_only_with_a_message_or_with_options_alone() {
        for (typed, opalro) in [
            ("commit -m x", "save -- x"),
            ("commit --message x", "save -- x"),
            ("commit --message=x", "save -- x"),
            ("commit -am x", "save -- x"),
            ("commit -a -m x", "save -- x"),
            ("commit -m x --all", "save -- x"),
            ("commit -m x --amend", "save --amend -- x"),
            ("commit --amend -m x", "save --amend -- x"),
            ("commit -m x --line l", "save --line l -- x"),
            ("commit --line l -m x", "save --line l -- x"),
            ("commit --line=l -m x", "save --line=l -- x"),
            ("commit -m x a.txt b", "save -- x a.txt b"),
            ("commit -m x -- -odd-name", "save -- x -odd-name"),
            ("commit --json -m x", "save --json -- x"),
            ("commit -m --amend", "save -- --amend"),
            ("commit --amend", "save --amend"),
            ("commit", "save"),
            // Without a message, -a is passed on, for save to refuse.
            ("commit -a", "save -a"),
        ] {
            assert_eq!(runs(typed).1, opalro, "{typed}");
        }
        for typed in [
            "commit x",
            "commit a.txt",
            "commit --amend a.txt",
            "commit -- a",
        ] {
            assert_eq!(explained(typed), COMMIT, "{typed}");
        }
    }

    /// `tag` is Opalro's own word: it runs as typed, with no hint, but for Git's `-d`.
    #[test]
    fn tag_is_opalros_own_word_and_d_is_delete() {
        for (line, opalro) in [
            ("tag", "tag"),
            ("tag v1.0", "tag v1.0"),
            ("tag v1.0 -d", "tag v1.0 -d"),
            ("tag -d v1.0", "tag --delete v1.0"),
            (
                "--json tag --json -d v1.0",
                "--json tag --json --delete v1.0",
            ),
            ("tag --delete v1.0", "tag --delete v1.0"),
            ("tag -d v1.0 --yes", "tag --delete v1.0 --yes"),
        ] {
            let Typed::Opalro(args) = typed(line) else {
                panic!("{line}")
            };
            let back: Vec<String> = args
                .iter()
                .map(|a| a.to_string_lossy().into_owned())
                .collect();
            assert_eq!(back.join(" "), opalro, "{line}");
        }
        assert_eq!(hint("tag"), None);
    }

    #[test]
    fn json_stays_where_it_was_typed() {
        assert_eq!(runs("--json commit -m x").1, "--json save -- x");
        assert_eq!(runs("--json stash --json pop").1, "--json unshelf --json");
        assert_eq!(runs("stash pop --json 3").1, "unshelf 3 --json");
        assert!(matches!(typed("--json add"), Typed::Explain(ADD)));
    }

    #[test]
    fn only_the_first_word_is_looked_at() {
        for line in [
            "save commit",
            "--json",
            "history log",
            "save -m x",
            "help commit",
        ] {
            let Typed::Opalro(args) = typed(line) else {
                panic!("{line}")
            };
            let back: Vec<String> = args
                .iter()
                .map(|a| a.to_string_lossy().into_owned())
                .collect();
            assert_eq!(back.join(" "), line);
        }
        assert!(matches!(translate(vec![]), Typed::Opalro(a) if a.is_empty()));
    }

    #[test]
    fn words_opal_does_not_run_are_explained() {
        assert_eq!(explained("add a.txt"), ADD);
        assert_eq!(explained("rm a"), RM_MV);
        assert_eq!(explained("mv a b"), RM_MV);
        assert_eq!(explained("reset --hard"), RESET);
        for typed in ["restore", "restore --staged", "restore .", "restore -- a"] {
            assert_eq!(explained(typed), RESTORE, "{typed}");
        }
        for typed in [
            "checkout -b side main",
            "checkout -b",
            "checkout -bside",
            "checkout side -b",
            "checkout -b -f",
            "switch -c side main",
            "switch --create side main",
            "switch -cside",
            "switch --create=side",
            "branch side",
            "branch side -d",
        ] {
            assert_eq!(explained(typed), NEW_LINE, "{typed}");
        }
        for typed in [
            "stash apply",
            "stash drop",
            "stash clear",
            "stash show",
            "stash create",
            "stash push -m x",
            "stash -u",
            "stash pop stash@{1}",
            "stash pop 1 2",
            "stash list x",
        ] {
            assert_eq!(explained(typed), STASH, "{typed}");
        }
        for word in ["merge", "rebase", "cherry-pick", "blame"] {
            assert_eq!(explained(word), LATER);
        }
        for word in ["clone", "fetch", "pull", "push", "remote"] {
            assert_eq!(explained(word), SYNC);
        }
        assert_eq!(explained("revert"), REVERT);
        assert_eq!(explained("clean -fd"), CLEAN);
    }

    #[test]
    fn explanations_are_two_or_three_lines_and_name_no_git_word() {
        for text in EXPLANATIONS {
            assert!((2..=3).contains(&text.lines().count()), "{text}");
            for word in text.split(|c: char| !c.is_ascii_alphanumeric() && c != '-') {
                assert!(!SHORTCUTS.contains(&word), "{word} in {text}");
            }
        }
    }

    #[test]
    fn no_explanation_or_hint_line_passes_78_columns() {
        // As printed: a hint after `hint: `, and its lines after the first under its text.
        let hints = SHORTCUTS
            .iter()
            .flat_map(|w| hint(w).unwrap().lines().enumerate())
            .map(|(n, line)| format!("{}{line}", if n == 0 { "hint: " } else { "      " }));
        for line in EXPLANATIONS
            .iter()
            .flat_map(|t| t.lines())
            .map(str::to_owned)
            .chain(hints)
        {
            assert!(line.chars().count() <= 78, "{line}");
        }
    }

    #[test]
    fn every_shortcut_word_has_a_hint_naming_its_opal_command() {
        for word in SHORTCUTS {
            let hint = hint(word).unwrap();
            assert!(
                hint.starts_with(&format!("in Opalro, {word} is ")),
                "{hint}"
            );
            assert!(hint.split_once(": ").unwrap().1.starts_with("opalro "));
        }
        assert_eq!(hint("save"), None);
        assert_eq!(hint("restore"), None);
        assert!(hint("commit").unwrap().contains("new files too"));
        assert!(hint("stash").unwrap().contains("new files too"));
    }

    #[test]
    fn a_word_is_new_once_and_lines_that_are_no_shortcut_are_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join(FILE);
        fs::write(&file, "log\nrestore\nnot a word\n commit\ndiff").unwrap();
        assert!(!first_time("log", Some(dir.path())));
        // `diff` has no LF after it, so it isn't a whole line.
        assert!(first_time("diff", Some(dir.path())));
        assert!(first_time("commit", Some(dir.path())));
        assert!(!first_time("commit", Some(dir.path())));
        assert_eq!(fs::read_to_string(&file).unwrap(), "log\ndiff\ncommit\n");
        assert!(first_time("commit", None));
        // A word that runs nothing is never remembered.
        assert!(first_time("restore", Some(dir.path())));
        assert!(first_time("restore", Some(dir.path())));
        // Only what the cap reads counts.
        let mut big = "x".repeat(MAX_BYTES as usize);
        big.push_str("\nlog\n");
        fs::write(&file, big).unwrap();
        assert!(first_time("log", Some(dir.path())));
        let left: Vec<_> = fs::read_dir(dir.path()).unwrap().collect();
        assert_eq!(left.len(), 1, "no temporary file is left");
    }
}
