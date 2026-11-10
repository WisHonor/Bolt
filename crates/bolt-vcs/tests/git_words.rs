//! Git's words: the shortcuts run Bolt's commands, the rest are explained, and the first use of
//! each shortcut names Bolt's word once.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;
use std::process::Output;

mod support;
use support::{Env, changed, head, text, write};

/// A project with `a.txt` and `b.txt` saved on `main`.
fn project() -> Env {
    let env = Env::empty();
    ok(&env, &["start", "--preset", "custom"]);
    write(&env, "a.txt", "a\n");
    write(&env, "b.txt", "b\n");
    ok(&env, &["save", "Files"]);
    env
}

fn run(env: &Env, args: &[&str]) -> Output {
    env.bolt().args(args).output().unwrap()
}

/// `bolt <args>`, which must succeed; what it printed.
fn ok(env: &Env, args: &[&str]) -> String {
    let out = run(env, args);
    assert_eq!(
        out.status.code(),
        Some(0),
        "{args:?}: {}",
        text(&out.stderr)
    );
    text(&out.stdout)
}

/// `bolt <args>`, which must be explained and change nothing: what it said on stderr.
fn explained(env: &Env, args: &[&str]) -> String {
    let before = head(env);
    let out = run(env, args);
    assert_eq!(out.status.code(), Some(2), "{args:?}");
    assert_eq!(text(&out.stdout), "", "{args:?}");
    assert_eq!(head(env), before, "{args:?} wrote a diary entry");
    let said = text(&out.stderr);
    assert!((2..=3).contains(&said.lines().count()), "{args:?}: {said}");
    assert!(!said.starts_with("error:"), "{said}");
    said
}

fn doc(out: &[u8]) -> serde_json::Value {
    serde_json::from_slice(out).unwrap()
}

/// The message of the newest save on the line.
fn last_message(env: &Env) -> String {
    let history = doc(&run(env, &["history", "--json"]).stdout);
    history["saves"][0]["message"].as_str().unwrap().to_owned()
}

#[test]
fn each_shortcut_runs_its_bolt_command() {
    let fresh = Env::empty();
    ok(&fresh, &["init", "--preset", "custom"]);
    assert!(fresh.root().join(".bolt").is_dir());

    let env = project();
    write(&env, "a.txt", "changed\n");
    for (git, bolt) in [
        (&["diff"][..], &["changes"][..]),
        (&["diff", "a.txt"], &["changes", "a.txt"]),
        (&["log"], &["history"]),
        (&["branch"], &["line"]),
        (&["stash", "list"], &["shelf", "--list"]),
    ] {
        let typed = run(&env, &[git, &["--json"]].concat());
        let meant = run(&env, &[bolt, &["--json"]].concat());
        assert_eq!(typed.status.code(), Some(0), "{git:?}");
        assert_eq!(text(&typed.stdout), text(&meant.stdout), "{git:?}");
    }
    for word in ["checkout", "switch"] {
        ok(&env, &[word, "main"]);
        let typed = run(&env, &[word, "nowhere"]);
        let meant = run(&env, &["goto", "nowhere"]);
        assert_eq!(typed.status.code(), Some(1));
        assert_eq!(text(&typed.stderr), text(&meant.stderr));
    }
    ok(&env, &["stash"]);
    assert!(changed(&env).is_empty());
    write(&env, "b.txt", "changed\n");
    ok(&env, &["stash", "push"]);
    let shelves = doc(&run(&env, &["shelf", "--list", "--json"]).stdout);
    assert_eq!(shelves["shelves"].as_array().unwrap().len(), 2);
    let tidied = doc(&run(&env, &["gc", "--json"]).stdout);
    let meant = doc(&run(&env, &["tidy", "--json"]).stdout);
    let keys = |d: &serde_json::Value| d.as_object().unwrap().keys().cloned().collect::<Vec<_>>();
    assert_eq!(keys(&tidied), keys(&meant));
}

#[test]
fn commit_takes_gits_message_forms() {
    let env = project();
    for (n, form) in [
        &["-m", "x0"][..],
        &["--message", "x1"],
        &["--message=x2"],
        &["-am", "x3"],
        &["-a", "-m", "x4"],
        &["-m", "x5", "--all"],
    ]
    .iter()
    .enumerate()
    {
        write(&env, "a.txt", format!("{n}\n"));
        ok(&env, &[&["commit"], *form].concat());
        assert_eq!(last_message(&env), format!("x{n}"));
    }
    write(&env, "a.txt", "plain\n");
    write(&env, "b.txt", "left\n");
    ok(&env, &["commit", "-m", "only a", "a.txt"]);
    assert_eq!(last_message(&env), "only a");
    assert_eq!(changed(&env), ["b.txt"]);
}

#[test]
fn commit_with_a_path_and_no_message_explains_and_saves_nothing() {
    let env = project();
    write(&env, "a.txt", "changed\n");
    let said = explained(&env, &["commit", "a.txt"]);
    assert!(said.contains("bolt save \"message\" <paths>"), "{said}");
    assert_eq!(changed(&env), ["a.txt"]);
    assert_eq!(last_message(&env), "Files");
}

#[test]
fn a_message_that_starts_with_a_dash_is_a_message() {
    let env = project();
    write(&env, "a.txt", "changed\n");
    ok(&env, &["commit", "-m", "--amend"]);
    let history = doc(&run(&env, &["history", "--json"]).stdout);
    let saves = history["saves"].as_array().unwrap();
    // The start's save, "Files", and the new one: nothing was amended.
    assert_eq!(saves.len(), 3, "{history}");
    assert_eq!(saves[0]["message"], "--amend");
    assert_eq!(saves[1]["message"], "Files");
}

#[test]
fn save_itself_still_refuses_dash_m() {
    let env = project();
    write(&env, "a.txt", "changed\n");
    let out = run(&env, &["save", "-m", "x"]);
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(changed(&env), ["a.txt"]);
}

#[test]
fn stash_pop_brings_a_shelf_back() {
    let env = project();
    write(&env, "a.txt", "changed\n");
    ok(&env, &["stash"]);
    assert!(changed(&env).is_empty());
    ok(&env, &["stash", "pop"]);
    assert_eq!(
        fs::read_to_string(env.root().join("a.txt")).unwrap(),
        "changed\n"
    );
    let shelves = doc(&run(&env, &["shelf", "--list", "--json"]).stdout);
    assert!(
        !shelves.to_string().contains("\"pop\""),
        "no shelf is called pop: {shelves}"
    );
}

#[test]
fn stash_list_lists() {
    let env = project();
    write(&env, "a.txt", "changed\n");
    ok(&env, &["shelf", "kept"]);
    let listed = ok(&env, &["stash", "list"]);
    assert_eq!(listed, ok(&env, &["shelf", "--list"]));
    assert!(listed.contains("kept"), "{listed}");
}

#[test]
fn stash_with_anything_else_explains_and_shelves_nothing() {
    let env = project();
    write(&env, "a.txt", "changed\n");
    for args in [
        &["stash", "create"][..],
        &["stash", "push", "-m", "x"],
        &["stash", "-u"],
        &["stash", "apply"],
        &["stash", "drop"],
        &["stash", "pop", "stash@{1}"],
    ] {
        let said = explained(&env, args);
        assert!(said.contains("bolt shelf --list"), "{said}");
        assert_eq!(changed(&env), ["a.txt"]);
    }
    let shelves = doc(&run(&env, &["shelf", "--list", "--json"]).stdout);
    assert_eq!(shelves["shelves"], serde_json::json!([]));
}

#[test]
fn restore_without_a_path_explains_and_discards_nothing() {
    let env = project();
    write(&env, "a.txt", "changed\n");
    for args in [
        &["restore"][..],
        &["restore", "--"],
        &["restore", "--staged"],
    ] {
        let said = explained(&env, args);
        assert!(said.contains("bolt discard <paths>"), "{said}");
        assert_eq!(changed(&env), ["a.txt"]);
    }
}

#[test]
fn restore_with_paths_explains_and_discards_nothing() {
    let env = project();
    write(&env, "a.txt", "changed\n");
    write(&env, "new.txt", "new\n");
    for args in [&["restore", "a.txt"][..], &["restore", "."]] {
        let said = explained(&env, args);
        assert!(said.contains("bolt undo"), "{said}");
    }
    assert_eq!(changed(&env), ["a.txt", "new.txt"]);
    assert_eq!(
        fs::read_to_string(env.root().join("a.txt")).unwrap(),
        "changed\n"
    );
}

#[test]
fn branch_with_an_option_gets_lines_own_answer() {
    let env = project();
    let out = run(&env, &["branch", "-d", "x"]);
    assert_eq!(out.status.code(), Some(2));
    let said = text(&out.stderr);
    assert!(!said.contains("--line <name>"), "{said}");
    assert!(said.contains("line"), "{said}");
}

#[test]
fn add_explains_and_changes_nothing() {
    let env = project();
    write(&env, "new.txt", "new\n");
    let said = explained(&env, &["add", "new.txt"]);
    assert!(said.contains("bolt save"), "{said}");
    // With --json it is still text, on stderr.
    let with_json = explained(&env, &["--json", "add", "."]);
    assert_eq!(with_json, said);
    assert_eq!(changed(&env), ["new.txt"]);
}

#[test]
fn each_explained_word_explains() {
    let env = project();
    for word in [
        "rm",
        "mv",
        "reset",
        "merge",
        "rebase",
        "cherry-pick",
        "tag",
        "blame",
        "clone",
        "fetch",
        "pull",
        "push",
        "remote",
        "revert",
        "clean",
    ] {
        let said = explained(&env, &[word, "x"]);
        assert!(said.contains("bolt "), "{word}: {said}");
    }
    for sub in ["apply", "drop", "clear", "show"] {
        let said = explained(&env, &["stash", sub]);
        assert!(said.contains("bolt unshelf"), "{said}");
    }
}

#[test]
fn checkout_dash_b_and_branch_with_a_name_explain() {
    let env = project();
    for args in [
        &["checkout", "-b", "x"][..],
        &["switch", "-c", "x"],
        &["branch", "x"],
    ] {
        let said = explained(&env, args);
        assert!(said.contains("bolt save --line <name>"), "{said}");
    }
    let lines = doc(&run(&env, &["line", "--json"]).stdout);
    assert_eq!(lines["lines"].as_array().unwrap().len(), 1, "{lines}");
}

/// `bolt` on an 80-column terminal without colour.
#[cfg(feature = "fault-injection")]
fn on_a_terminal(env: &Env, args: &[&str]) -> Output {
    env.bolt()
        .env("BOLT_FAKE_TERMINAL", "80")
        .env("BOLT_PROGRESS_QUIET_MS", "600000")
        .env("NO_COLOR", "1")
        .args(args)
        .output()
        .unwrap()
}

#[cfg(feature = "fault-injection")]
const COMMIT_HINT: &str =
    "\nhint: in Bolt, commit is save, which takes new files too: bolt save \"message\"\n";

#[cfg(feature = "fault-injection")]
#[test]
fn the_hint_shows_once_per_word_on_a_terminal() {
    let env = project();
    write(&env, "a.txt", "1\n");
    let first = on_a_terminal(&env, &["commit", "-m", "one"]);
    assert_eq!(first.status.code(), Some(0));
    assert!(
        text(&first.stderr).ends_with(COMMIT_HINT),
        "{}",
        text(&first.stderr)
    );
    write(&env, "a.txt", "2\n");
    let second = on_a_terminal(&env, &["commit", "-m", "two"]);
    assert_eq!(second.status.code(), Some(0));
    assert!(!text(&second.stderr).contains("hint: in Bolt"));
    assert_eq!(
        fs::read_to_string(env.config().join("git-words")).unwrap(),
        "commit\n"
    );
    // Another word is new, and a failed command still teaches it.
    let failed = on_a_terminal(&env, &["checkout", "nowhere"]);
    assert_eq!(failed.status.code(), Some(1));
    assert!(
        text(&failed.stderr)
            .ends_with("\nhint: in Bolt, checkout is goto: bolt goto <line or save>\n"),
        "{}",
        text(&failed.stderr)
    );
    assert_eq!(
        fs::read_to_string(env.config().join("git-words")).unwrap(),
        "commit\ncheckout\n"
    );
}

#[cfg(feature = "fault-injection")]
#[test]
fn no_hint_without_a_terminal_or_with_json() {
    let env = project();
    write(&env, "a.txt", "1\n");
    let out = run(&env, &["commit", "-m", "one"]);
    assert_eq!(text(&out.stderr), "");
    write(&env, "a.txt", "2\n");
    let out = on_a_terminal(&env, &["commit", "--json", "-m", "two"]);
    assert_eq!(out.status.code(), Some(0));
    assert!(
        !text(&out.stderr).contains("hint:"),
        "{}",
        text(&out.stderr)
    );
    assert!(!env.config().join("git-words").exists());
}

#[cfg(feature = "fault-injection")]
#[test]
fn a_git_words_file_bolt_cannot_use_is_never_an_error() {
    let env = project();
    let file = env.config().join("git-words");
    fs::create_dir(&file).unwrap();
    for n in 0..2 {
        write(&env, "a.txt", format!("{n}\n"));
        let out = on_a_terminal(&env, &["commit", "-m", "x"]);
        assert_eq!(out.status.code(), Some(0));
        let said = text(&out.stderr);
        assert!(said.ends_with(COMMIT_HINT), "{said}");
        assert!(!said.contains("warning:"), "{said}");
    }
    fs::remove_dir(&file).unwrap();
    fs::write(&file, vec![b'z'; 1 << 20]).unwrap();
    write(&env, "a.txt", "junk\n");
    let out = on_a_terminal(&env, &["commit", "-m", "x"]);
    assert_eq!(out.status.code(), Some(0));
    let said = text(&out.stderr);
    assert!(said.ends_with(COMMIT_HINT), "{said}");
    assert!(!said.contains("warning:"), "{said}");
}

#[test]
fn the_diary_names_bolts_command() {
    let env = project();
    write(&env, "a.txt", "changed\n");
    ok(&env, &["commit", "-m", "x"]);
    let list = doc(&run(&env, &["undo", "--list", "--json"]).stdout);
    assert_eq!(list["undo"][0]["command"], "save", "{list}");
    assert!(ok(&env, &["undo", "--list"]).contains("save"));
}

#[test]
fn help_never_names_a_git_word() {
    // `tag` is left out: it is Bolt's own word for the tags an import brings.
    const GIT: [&str; 26] = [
        "init",
        "commit",
        "diff",
        "log",
        "checkout",
        "switch",
        "stash",
        "branch",
        "fsck",
        "gc",
        "add",
        "restore",
        "rm",
        "mv",
        "reset",
        "merge",
        "rebase",
        "cherry-pick",
        "blame",
        "clone",
        "fetch",
        "pull",
        "push",
        "remote",
        "revert",
        "clean",
    ];
    let env = Env::empty();
    let mut helps = vec![
        ok(&env, &["--help"]),
        ok(&env, &["help"]),
        text(&run(&env, &[]).stderr),
    ];
    for command in [
        "start", "setup", "import", "save", "status", "changes", "history", "show", "line", "goto",
        "discard", "undo", "redo", "shelf", "unshelf", "tidy",
    ] {
        helps.push(ok(&env, &[command, "--help"]));
        helps.push(ok(&env, &["help", command]));
    }
    for help in helps {
        let words: Vec<String> = help
            .split(|c: char| !c.is_ascii_alphanumeric() && c != '-')
            .map(str::to_ascii_lowercase)
            .collect();
        for git in GIT {
            assert!(!words.iter().any(|w| w == git), "{git} in:\n{help}");
        }
    }
}

#[test]
fn json_comes_through_a_shortcut() {
    let env = project();
    for (n, args) in [
        &["--json", "commit", "-m", "x0"][..],
        &["commit", "--json", "-m", "x1"],
    ]
    .iter()
    .enumerate()
    {
        write(&env, "a.txt", format!("{n}\n"));
        let out = run(&env, args);
        assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
        let saved = doc(&out.stdout);
        assert_eq!(saved["save"]["message"], format!("x{n}"), "{saved}");
    }
}
