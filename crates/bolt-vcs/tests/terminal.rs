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

#[test]
fn warnings_come_after_an_empty_line_each_with_its_prefix() {
    let env = project_with_a_damaged_copy_of_its_licence();
    let out = bolt_in_a_log(&env, env.root())
        .args(["save", "Add licence"])
        .output()
        .unwrap();
    let said = text(&out.stderr);
    assert_eq!(out.status.code(), Some(0), "{said}");
    let lines: Vec<&str> = said.lines().collect();
    assert_eq!(lines.len(), 4, "{said:?}");
    assert_eq!(lines[0], "", "{said:?}");
    assert!(lines[1].starts_with("warning: pack "), "{said:?}");
    assert_eq!(lines[2], "", "{said:?}");
    assert_eq!(
        lines[3],
        "hint: Bolt didn't use the damaged copy. Ask for help before trying to repair it."
    );
}
