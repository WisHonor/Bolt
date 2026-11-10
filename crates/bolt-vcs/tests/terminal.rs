//! What `bolt` prints on a terminal, and what it prints when nobody is looking.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::Path;

use bolt_core::pack::PackWriter;

mod support;
use support::{Env, text};

/// The symbols Bolt draws on a terminal that shows them.
const SYMBOLS: [char; 10] = ['•', '▸', '→', '←', '✓', '✗', '─', '…', '█', '░'];

/// `bolt` in `dir` where everything asks for colour and symbols, but no terminal.
fn bolt_in_a_log(env: &Env, dir: &Path) -> assert_cmd::Command {
    let mut cmd = env.bolt_in(dir);
    cmd.env("TERM", "xterm-256color")
        .env("LC_ALL", "en_US.UTF-8")
        .env("TERM_PROGRAM", "bolt-test")
        .env("BOLT_ASCII", "0")
        .env_remove("NO_COLOR")
        .env_remove("BOLT_FAKE_TERMINAL");
    cmd
}

/// `bolt` in `dir` on a terminal 80 columns wide that shows colour and symbols.
#[cfg(feature = "fault-injection")]
fn bolt_on_a_terminal(env: &Env, dir: &Path) -> assert_cmd::Command {
    let mut cmd = bolt_in_a_log(env, dir);
    cmd.env("BOLT_FAKE_TERMINAL", "80").env_remove("BOLT_ASCII");
    cmd
}

/// A project with `a.txt` saved as "a", then changed to "abc".
#[cfg(feature = "fault-injection")]
fn project_with_a_change() -> Env {
    let env = Env::empty();
    env.bolt().arg("start").assert().success();
    std::fs::write(env.root().join("a.txt"), "a\n").unwrap();
    env.bolt().args(["save", "first"]).assert().success();
    std::fs::write(env.root().join("a.txt"), "abc\n").unwrap();
    env
}

#[cfg(feature = "fault-injection")]
#[test]
fn an_error_is_red_and_its_hint_cyan_after_an_empty_line() {
    let env = Env::empty();
    let out = bolt_on_a_terminal(&env, env.root())
        .arg("status")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(
        text(&out.stderr),
        "\x1b[1;31merror:\x1b[0m Not inside a Bolt project.\n\n\x1b[1;36mhint:\x1b[0m Run `bolt \
         start` to make this folder a Bolt project.\n"
    );
}

#[cfg(feature = "fault-injection")]
#[test]
fn no_color_turns_colour_off_and_keeps_symbols() {
    let env = project_with_a_change();
    let out = bolt_on_a_terminal(&env, env.root())
        .env("NO_COLOR", "1")
        .arg("changes")
        .output()
        .unwrap();
    let shown = text(&out.stdout);
    assert!(
        shown.starts_with("modified  a.txt  (2 B → 4 B)\n"),
        "{shown:?}"
    );
    assert!(!shown.contains('\x1b'), "{shown:?}");
}

#[cfg(feature = "fault-injection")]
#[test]
fn bolt_ascii_turns_symbols_off_and_keeps_colour() {
    let env = project_with_a_change();
    let out = bolt_on_a_terminal(&env, env.root())
        .env("BOLT_ASCII", "1")
        .arg("changes")
        .output()
        .unwrap();
    let shown = text(&out.stdout);
    assert!(
        shown.starts_with("modified  a.txt  (2 B -> 4 B)\n"),
        "{shown:?}"
    );
    assert!(
        shown.contains("\x1b[36m@@ -1,1 +1,1 @@\x1b[0m"),
        "{shown:?}"
    );
    assert!(shown.contains("\x1b[32m+abc\x1b[0m"), "{shown:?}");
}

#[cfg(feature = "fault-injection")]
#[test]
fn json_has_no_colour_even_on_a_terminal() {
    let env = project_with_a_change();
    for args in [&["--json", "changes"][..], &["--json", "status"]] {
        let out = bolt_on_a_terminal(&env, env.root())
            .args(args)
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(0), "{args:?}");
        assert!(!out.stdout.contains(&0x1b), "{}", text(&out.stdout));
        assert!(out.stderr.is_empty(), "{}", text(&out.stderr));
    }
    let outside = Env::empty();
    let out = bolt_on_a_terminal(&outside, outside.root())
        .args(["--json", "status"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(!out.stdout.contains(&0x1b), "{}", text(&out.stdout));
    assert!(out.stderr.is_empty(), "{}", text(&out.stderr));
}

/// Every command, run as a script would: stderr stays empty when it works, and stdout has no
/// colour and no symbols, whatever the environment asks for.
#[test]
fn without_a_terminal_bolts_own_words_are_plain() {
    let env = Env::empty();
    let run = |args: &[&str]| {
        let out = bolt_in_a_log(&env, env.root()).args(args).output().unwrap();
        let (shown, said) = (text(&out.stdout), text(&out.stderr));
        assert_eq!(out.status.code(), Some(0), "{args:?}: {said}");
        assert!(said.is_empty(), "{args:?}: {said:?}");
        assert!(!shown.contains('\x1b'), "{args:?}: {shown:?}");
        assert!(!shown.contains(SYMBOLS), "{args:?}: {shown:?}");
        shown
    };
    run(&["start"]);
    std::fs::write(env.root().join("a.txt"), "a\n").unwrap();
    run(&["save", "first"]);
    std::fs::write(env.root().join("a.txt"), "abc\n").unwrap();
    assert!(run(&["status"]).contains("(2 B -> 4 B)"));
    assert!(run(&["changes"]).contains("(2 B -> 4 B)"));
    run(&["save", "second"]);
    for args in [
        &["history"][..],
        &["show"],
        &["line"],
        &["changes", "main"],
        &["goto", "main"],
        &["undo", "--list"],
        &["undo"],
        &["redo"],
    ] {
        run(args);
    }
    std::fs::write(env.root().join("a.txt"), "abcd\n").unwrap();
    run(&["shelf", "wip"]);
    run(&["shelf", "--list"]);
    run(&["unshelf"]);
    run(&["discard"]);
    run(&["tidy"]);
}

/// A project whose store holds a damaged copy of the licence it is about to save.
fn project_with_a_damaged_copy_of_its_licence() -> Env {
    let env = Env::empty();
    env.bolt().arg("start").assert().success();
    let licence = b"Permission is hereby granted, free of charge, to any person\n";
    let made = tempfile::tempdir().unwrap();
    let mut w = PackWriter::create(made.path()).unwrap();
    w.add_content(licence).unwrap();
    let mut pack = std::fs::read(w.finish(made.path()).unwrap().unwrap()).unwrap();
    pack[16 + 50] ^= 1; // the first byte after the pack header and the entry header
    std::fs::write(env.root().join(".bolt/store/packs/00.pack"), pack).unwrap();
    std::fs::write(env.root().join("LICENSE"), licence).unwrap();
    env
}

/// Without a terminal the answer goes elsewhere, so stderr starts with the warning itself.
#[test]
fn warnings_each_have_their_prefix_and_stderr_never_starts_with_an_empty_line() {
    let env = project_with_a_damaged_copy_of_its_licence();
    let out = bolt_in_a_log(&env, env.root())
        .args(["save", "Add licence"])
        .output()
        .unwrap();
    let said = text(&out.stderr);
    assert_eq!(out.status.code(), Some(0), "{said}");
    let lines: Vec<&str> = said.lines().collect();
    assert_eq!(lines.len(), 3, "{said:?}");
    assert!(lines[0].starts_with("warning: pack "), "{said:?}");
    assert_eq!(lines[1], "", "{said:?}");
    assert_eq!(
        lines[2],
        "hint: Bolt didn't use the damaged copy. Ask for help before trying to repair it."
    );
}

/// On a terminal the answer and the warning share the screen, so an empty line parts them.
#[cfg(feature = "fault-injection")]
#[test]
fn a_warning_after_the_answer_on_a_terminal_comes_after_one_empty_line() {
    let env = project_with_a_damaged_copy_of_its_licence();
    let out = bolt_on_a_terminal(&env, env.root())
        .args(["save", "Add licence"])
        .output()
        .unwrap();
    let said = text(&out.stderr);
    assert_eq!(out.status.code(), Some(0), "{said}");
    assert!(text(&out.stdout).starts_with("Saved "));
    assert!(
        said.starts_with("\n\x1b[1;33mwarning:\x1b[0m pack"),
        "{said:?}"
    );
    assert!(!said.starts_with("\n\n"), "{said:?}");
}

/// A failing command whose first words are a warning starts with it, on a terminal or not.
#[cfg(feature = "fault-injection")]
#[test]
fn a_failing_command_that_warns_first_starts_with_the_warning() {
    for terminal in [false, true] {
        let env = Env::empty();
        env.bolt().arg("start").assert().success();
        let first = support::main_tip(&env).to_hex();
        std::fs::write(env.root().join("a.txt"), "a\n").unwrap();
        std::fs::write(env.root().join("b.txt"), "b\n").unwrap();
        env.bolt().args(["save", "Two files"]).assert().success();
        // A goto stopped after removing a.txt, then a.txt written again, in the way of finishing
        // it.
        let stopped = env
            .bolt()
            .args(["goto", &first])
            .env("BOLT_FAULT_FAIL", "switch:changed")
            .output()
            .unwrap();
        assert_eq!(stopped.status.code(), Some(1), "{}", text(&stopped.stderr));
        std::fs::write(env.root().join("a.txt"), "mine\n").unwrap();
        let mut cmd = if terminal {
            bolt_on_a_terminal(&env, env.root())
        } else {
            bolt_in_a_log(&env, env.root())
        };
        let out = cmd.args(["goto", "main"]).output().unwrap();
        let said = text(&out.stderr);
        assert_eq!(out.status.code(), Some(1), "{said}");
        assert!(out.stdout.is_empty(), "{said}");
        assert!(said.contains("warning:"), "{said}");
        assert!(!said.starts_with('\n'), "{said:?}");
    }
}

/// Progress is for people at a terminal: a script sees none, whatever its environment asks for.
#[test]
fn no_progress_reaches_stderr_without_a_terminal() {
    let env = Env::new();
    env.commit("a.txt", "a\n", "First");
    env.commit("b.txt", "b\n", "Second");
    // A sign of Git LFS, so the import looks for its files too.
    std::fs::create_dir(env.root().join(".git/lfs")).unwrap();
    for args in [&["import"][..], &["save", "Third"]] {
        if args[0] == "save" {
            std::fs::write(env.root().join("c.txt"), "c\n").unwrap();
        }
        let out = bolt_in_a_log(&env, env.root()).args(args).output().unwrap();
        assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
        assert!(out.stderr.is_empty(), "{args:?}: {}", text(&out.stderr));
    }
}

/// `tidy` tells how much it is about to free only to a person, never beside `--json`.
#[cfg(feature = "fault-injection")]
#[test]
fn tidy_tells_its_estimate_only_to_a_person() {
    let env = project_with_a_change();
    let said = |args: &[&str]| {
        let out = bolt_on_a_terminal(&env, env.root())
            .args(args)
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
        text(&out.stderr)
    };
    assert!(said(&["tidy"]).starts_with("Tidying: about "));
    assert_eq!(said(&["--json", "tidy"]), "");
}
