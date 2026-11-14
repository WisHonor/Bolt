//! Git's words: the shortcuts run Opalro's commands, the rest are explained, and the first use of
//! each shortcut names Opalro's word once.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;
use std::process::Output;

use serde_json::json;

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
    env.opalro().args(args).output().unwrap()
}

/// `opalro <args>`, which must succeed; what it printed.
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

/// `opalro <args>`, which must be explained and change nothing: what it said on stderr.
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
fn each_shortcut_runs_its_opal_command() {
    let fresh = Env::empty();
    ok(&fresh, &["init", "--preset", "custom"]);
    assert!(fresh.root().join(".opalro").is_dir());

    let env = project();
    write(&env, "a.txt", "changed\n");
    for (git, opalro) in [
        (&["diff"][..], &["changes"][..]),
        (&["diff", "a.txt"], &["changes", "a.txt"]),
        (&["log"], &["history"]),
        (&["branch"], &["line"]),
        (&["stash", "list"], &["shelf", "--list"]),
    ] {
        let typed = run(&env, &[git, &["--json"]].concat());
        let meant = run(&env, &[opalro, &["--json"]].concat());
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
    assert!(said.contains("opalro save \"message\" <paths>"), "{said}");
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
        assert!(said.contains("opalro shelf --list"), "{said}");
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
        assert!(said.contains("opalro discard <paths>"), "{said}");
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
        assert!(said.contains("opalro undo"), "{said}");
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
    let out = run(&env, &["branch", "-a"]);
    assert_eq!(out.status.code(), Some(2));
    let said = text(&out.stderr);
    assert!(!said.contains("--line <name>"), "{said}");
    assert!(said.contains("line"), "{said}");
}

#[test]
fn commit_whose_line_is_given_no_name_is_refused_and_saves_nothing() {
    let env = project();
    write(&env, "a.txt", "changed\n");
    let before = head(&env);
    let out = run(&env, &["commit", "-m", "x", "--line", "--", "y"]);
    let said = text(&out.stderr);
    assert_eq!(out.status.code(), Some(2), "{said}");
    assert!(said.contains("--line"), "{said}");
    assert_eq!(text(&out.stdout), "");
    assert_eq!(head(&env), before);
    assert_eq!(changed(&env), ["a.txt"]);
}

#[test]
fn add_explains_and_changes_nothing() {
    let env = project();
    write(&env, "new.txt", "new\n");
    let said = explained(&env, &["add", "new.txt"]);
    assert!(said.contains("opalro save"), "{said}");
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
        assert!(said.contains("opalro "), "{word}: {said}");
    }
    for sub in ["apply", "drop", "clear", "show"] {
        let said = explained(&env, &["stash", sub]);
        assert!(said.contains("opalro unshelf"), "{said}");
    }
}

/// The project's lines, and the one it is on.
fn lines(env: &Env) -> (serde_json::Value, serde_json::Value) {
    let doc = doc(&run(env, &["line", "--json"]).stdout);
    (doc["lines"].clone(), doc["position"]["line"].clone())
}

#[test]
fn checkout_b_and_switch_c_start_a_line() {
    let env = project();
    let started = ok(&env, &["checkout", "-b", "wall-run"]);
    assert!(
        started.starts_with("Started line wall-run at save "),
        "{started}"
    );
    assert_eq!(
        lines(&env),
        (json!(["main", "wall-run"]), json!("wall-run"))
    );
    let made = doc(&run(&env, &["switch", "--json", "-c", "side"]).stdout);
    assert_eq!(made["line"], "side", "{made}");
    ok(&env, &["switch", "--create", "side-2"]);
    assert_eq!(
        lines(&env),
        (
            json!(["main", "side", "side-2", "wall-run"]),
            json!("side-2")
        )
    );
}

#[test]
fn checkout_b_with_a_starting_point_is_explained() {
    let env = project();
    for args in [
        &["checkout", "-b", "side", "main"][..],
        &["switch", "-c", "side", "main"],
        &["switch", "--create", "side", "main"],
        &["checkout", "-bside"],
        &["checkout", "side", "-b"],
        &["checkout", "-b"],
    ] {
        let said = explained(&env, args);
        assert!(said.contains("opalro line new <name>"), "{said}");
        assert!(said.contains("opalro goto <save>"), "{said}");
    }
    assert_eq!(lines(&env), (json!(["main"]), json!("main")));
}

#[test]
fn branch_m_renames() {
    let env = project();
    ok(&env, &["branch", "-m", "wall-run"]);
    assert_eq!(lines(&env), (json!(["wall-run"]), json!("wall-run")));
    ok(&env, &["branch", "--move", "wall-run", "side"]);
    assert_eq!(lines(&env), (json!(["side"]), json!("side")));
    ok(&env, &["branch", "-m", "side", "wall-run", "--json"]);
    assert_eq!(lines(&env), (json!(["wall-run"]), json!("wall-run")));
}

#[test]
fn branch_d_deletes_and_capital_d_says_yes() {
    let env = project();
    ok(&env, &["line", "new", "side"]);
    ok(&env, &["goto", "main"]);
    ok(&env, &["branch", "-d", "side"]);
    assert_eq!(lines(&env).0, json!(["main"]));

    // A line with a save of its own: without a terminal, -d stops and names --yes.
    ok(&env, &["line", "new", "side"]);
    write(&env, "a.txt", "side\n");
    ok(&env, &["save", "On side"]);
    ok(&env, &["goto", "main"]);
    for flag in ["-d", "--delete"] {
        let out = run(&env, &["branch", flag, "side"]);
        assert_eq!(out.status.code(), Some(1));
        assert!(text(&out.stderr).contains("--yes"), "{}", text(&out.stderr));
    }
    assert_eq!(lines(&env).0, json!(["main", "side"]));
    ok(&env, &["branch", "-D", "side"]);
    assert_eq!(lines(&env).0, json!(["main"]));
}

#[test]
fn branch_with_a_name_is_still_explained() {
    let env = project();
    let said = explained(&env, &["branch", "side"]);
    assert!(said.contains("opalro line new <name>"), "{said}");
    assert_eq!(lines(&env), (json!(["main"]), json!("main")));
}

#[test]
fn tag_d_deletes() {
    let env = project();
    ok(&env, &["tag", "v1.0"]);
    ok(&env, &["tag", "v2.0"]);
    ok(&env, &["tag", "-d", "v1.0", "--yes"]);
    ok(&env, &["tag", "--delete", "v2.0", "--yes"]);
    let tags = doc(&run(&env, &["tag", "--json"]).stdout);
    assert_eq!(tags["tags"], json!([]), "{tags}");
    // Never `--yes` on its own: without a terminal it stops and names it.
    ok(&env, &["tag", "v1.0"]);
    let out = run(&env, &["tag", "-d", "v1.0"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(text(&out.stderr).contains("--yes"), "{}", text(&out.stderr));
}

#[test]
fn tag_goes_to_opalro_as_typed() {
    let env = project();
    let typed = ok(&env, &["tag", "v1.0", "First"]);
    assert!(typed.starts_with("Tagged save "), "{typed}");
    let listed = doc(&run(&env, &["tag", "--json"]).stdout);
    assert_eq!(listed["tags"][0]["name"], "v1.0", "{listed}");
}

/// `opalro` on an 80-column terminal without colour.
#[cfg(feature = "fault-injection")]
fn on_a_terminal(env: &Env, args: &[&str]) -> Output {
    env.opalro()
        .env("OPALRO_FAKE_TERMINAL", "80")
        .env("OPALRO_PROGRESS_QUIET_MS", "600000")
        .env("NO_COLOR", "1")
        .args(args)
        .output()
        .unwrap()
}

#[cfg(feature = "fault-injection")]
const COMMIT_HINT: &str =
    "\nhint: in Opalro, commit is save; it takes new files too: opalro save \"message\"\n";

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
    assert!(!text(&second.stderr).contains("hint: in Opalro"));
    assert_eq!(
        fs::read_to_string(env.config().join("git-words")).unwrap(),
        "commit\n"
    );
    // Another word is new, and a failed command still teaches it.
    let failed = on_a_terminal(&env, &["checkout", "nowhere"]);
    assert_eq!(failed.status.code(), Some(1));
    assert!(
        text(&failed.stderr).ends_with(
            "\nhint: in Opalro, checkout is goto: opalro goto <line or save>\n      With -b, it \
             is opalro line new <name>.\n"
        ),
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
fn a_git_words_file_opal_cannot_use_is_never_an_error() {
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

/// An explanation wraps at words on a terminal narrower than its lines, and stays whole in a log.
#[cfg(feature = "fault-injection")]
#[test]
fn an_explanation_wraps_on_a_narrow_terminal() {
    let env = project();
    let logged = explained(&env, &["stash", "apply"]);
    let out = env
        .opalro()
        .env("OPALRO_FAKE_TERMINAL", "40")
        .env("NO_COLOR", "1")
        .args(["stash", "apply"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    let said = text(&out.stderr);
    assert!(said.lines().count() > logged.lines().count(), "{said}");
    for line in said.lines() {
        assert!(line.chars().count() <= 40, "{line:?}");
    }
    let words = |text: &str| text.split_whitespace().collect::<Vec<_>>().join(" ");
    assert_eq!(words(&said), words(&logged));
}

/// Opalro numbers shelves from 1 and a number stays with its shelf, so both say where to find it.
#[cfg(feature = "fault-injection")]
#[test]
fn stash_says_a_shelfs_number_is_the_one_shelf_list_shows() {
    let env = project();
    let said = explained(&env, &["stash", "drop"]);
    assert!(
        said.contains(
            "`opalro shelf --list` lists them by that number, which stays with its shelf."
        ),
        "{said}"
    );
    write(&env, "a.txt", "changed\n");
    let out = on_a_terminal(&env, &["stash"]);
    assert_eq!(out.status.code(), Some(0));
    assert!(
        text(&out.stderr).ends_with(
            "\nhint: in Opalro, stash is shelf, which takes new files too: opalro shelf\n      A \
             shelf keeps the number opalro shelf --list shows, counted from 1.\n"
        ),
        "{}",
        text(&out.stderr)
    );
}

/// Both of the stash hint's commands are painted as commands.
#[cfg(feature = "fault-injection")]
#[test]
fn the_stash_hints_commands_are_both_coloured() {
    let env = project();
    write(&env, "a.txt", "changed\n");
    let out = env
        .opalro()
        .env("OPALRO_FAKE_TERMINAL", "80")
        .env("OPALRO_PROGRESS_QUIET_MS", "600000")
        .env_remove("NO_COLOR")
        .arg("stash")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    let said = text(&out.stderr);
    for command in ["opalro shelf", "opalro shelf --list"] {
        assert!(
            said.contains(&format!("\x1b[36m{command}\x1b[0m")),
            "{command}: {said:?}"
        );
    }
}

#[test]
fn the_diary_names_opals_command() {
    let env = project();
    write(&env, "a.txt", "changed\n");
    ok(&env, &["commit", "-m", "x"]);
    let list = doc(&run(&env, &["undo", "--list", "--json"]).stdout);
    assert_eq!(list["undo"][0]["command"], "save", "{list}");
    assert!(ok(&env, &["undo", "--list"]).contains("save"));
}

#[test]
fn help_never_names_a_git_word() {
    // `tag` is left out: it is Opalro's own word for the tags an import brings.
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
        text(&run(&env, &[]).stdout),
    ];
    for command in [
        "start", "setup", "import", "save", "status", "changes", "history", "show", "line", "goto",
        "discard", "undo", "redo", "shelf", "unshelf", "tidy", "verify",
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
