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

/// `bolt` in `dir` on a terminal 80 columns wide that shows colour and symbols. Progress waits
/// ten minutes, so it never comes in first on a slow machine; a test about progress sets its own.
#[cfg(feature = "fault-injection")]
fn bolt_on_a_terminal(env: &Env, dir: &Path) -> assert_cmd::Command {
    let mut cmd = bolt_in_a_log(env, dir);
    cmd.env("BOLT_FAKE_TERMINAL", "80")
        .env("BOLT_PROGRESS_QUIET_MS", "600000")
        .env_remove("BOLT_ASCII");
    cmd
}

/// A project with `a.txt` saved as "a", then changed to "abc".
#[cfg(feature = "fault-injection")]
fn project_with_a_change() -> Env {
    let env = Env::empty();
    env.bolt()
        .args(["start", "--preset", "custom"])
        .assert()
        .success();
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
        shown.starts_with("modified  a.txt  +1 -1  (2 B → 4 B)\n"),
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
        shown.starts_with(
            "\x1b[33mmodified\x1b[0m  \x1b[34ma.txt\x1b[0m  \x1b[32m+1\x1b[0m \x1b[31m-1\x1b[0m  \
             \x1b[2m(2 B -> 4 B)\x1b[0m\n"
        ),
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
    for args in [
        &["--json", "changes"][..],
        &["--json", "status"],
        &["--json", "help"],
        &["--json", "save", "--help"],
    ] {
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
    run(&["start", "--preset", "custom"]);
    std::fs::write(env.root().join("a.txt"), "a\n").unwrap();
    run(&["save", "first"]);
    std::fs::write(env.root().join("a.txt"), "abc\n").unwrap();
    assert!(run(&["status"]).contains("  2 B -> 4 B"));
    assert!(run(&["changes"]).contains("  (2 B -> 4 B)"));
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
    assert!(run(&["verify"]).contains("    + "));
    run(&["help"]);
    run(&["help", "import"]);
    // An import, in a Git repository of its own.
    let git = Env::new();
    git.commit("a.txt", "a\n", "First");
    let out = bolt_in_a_log(&git, git.root())
        .arg("import")
        .output()
        .unwrap();
    let (shown, said) = (text(&out.stdout), text(&out.stderr));
    assert_eq!(out.status.code(), Some(0), "{said}");
    assert_eq!(said, "");
    assert_eq!(
        shown,
        "Imported 1 commit from Git: 1 line, 0 tags.\n\nYou're on line main.\n"
    );
}

/// A project whose store holds a damaged copy of the licence it is about to save.
fn project_with_a_damaged_copy_of_its_licence() -> Env {
    let env = Env::empty();
    env.bolt()
        .args(["start", "--preset", "custom"])
        .assert()
        .success();
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
    assert!(text(&out.stdout).starts_with("\x1b[1;32mSaved\x1b[0m "));
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
        env.bolt()
            .args(["start", "--preset", "custom"])
            .assert()
            .success();
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

/// Progress is for people at a terminal: a script sees none, whatever its environment asks for,
/// even with no quiet time; the same commands on a terminal do show it.
#[cfg(feature = "fault-injection")]
#[test]
fn no_progress_reaches_stderr_without_a_terminal() {
    for terminal in [true, false] {
        let env = Env::new();
        env.commit("a.txt", "a\n", "First");
        env.commit("b.txt", "b\n", "Second");
        // A sign of Git LFS, so the import looks for its files too.
        std::fs::create_dir(env.root().join(".git/lfs")).unwrap();
        for (args, label) in [
            (&["import"][..], "Reading Git's history"),
            (&["save", "Third"], "Saving"),
        ] {
            if args[0] == "save" {
                std::fs::write(env.root().join("c.txt"), "c\n").unwrap();
            }
            let mut cmd = if terminal {
                bolt_on_a_terminal(&env, env.root())
            } else {
                bolt_in_a_log(&env, env.root())
            };
            let out = cmd
                .env("BOLT_PROGRESS_QUIET_MS", "0")
                .args(args)
                .output()
                .unwrap();
            let said = text(&out.stderr);
            assert_eq!(out.status.code(), Some(0), "{said}");
            if terminal {
                assert!(
                    said.starts_with(&format!("\r{label}")),
                    "{args:?}: {said:?}"
                );
            } else {
                assert!(said.is_empty(), "{args:?}: {said:?}");
            }
        }
    }
}

/// `bolt` in `dir` on a terminal `width` columns wide, without colour or symbols, so that a test
/// can read the layout.
#[cfg(feature = "fault-injection")]
fn bolt_at_width(env: &Env, dir: &Path, width: usize) -> assert_cmd::Command {
    let mut cmd = bolt_on_a_terminal(env, dir);
    cmd.env("BOLT_FAKE_TERMINAL", width.to_string())
        .env("NO_COLOR", "1")
        .env("BOLT_ASCII", "1");
    cmd
}

/// A project just started, with no rule files.
fn started() -> Env {
    let env = Env::empty();
    env.bolt()
        .args(["start", "--preset", "custom"])
        .assert()
        .success();
    env
}

/// `bolt <args>` run in a log or on a terminal `width` columns wide: its exit code, then what it
/// printed on stdout and stderr.
fn output_at(env: &Env, width: Option<usize>, args: &[&str]) -> (Option<i32>, String, String) {
    let mut cmd = match width {
        None => bolt_in_a_log(env, env.root()),
        #[cfg(feature = "fault-injection")]
        Some(width) => bolt_at_width(env, env.root(), width),
        #[cfg(not(feature = "fault-injection"))]
        Some(_) => unreachable!("a terminal is faked only with fault injection"),
    };
    let out = cmd.args(args).output().unwrap();
    (out.status.code(), text(&out.stdout), text(&out.stderr))
}

/// What `args` print on stdout and stderr, in a log or on a terminal `width` columns wide.
fn run_at(env: &Env, width: Option<usize>, args: &[&str]) -> (String, String) {
    let (code, shown, said) = output_at(env, width, args);
    assert_eq!(code, Some(0), "{args:?}: {said}");
    (shown, said)
}

/// [`run_at`], for a command that must fail.
fn failing_at(env: &Env, width: Option<usize>, args: &[&str]) -> (String, String) {
    let (code, shown, said) = output_at(env, width, args);
    assert_eq!(code, Some(1), "{args:?}: {shown}{said}");
    (shown, said)
}

/// The name of the project's folder, as `line` and `undo --list` give it.
fn folder_name(env: &Env) -> String {
    env.root()
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into_owned()
}

/// Writes `contents` to the project's `path`.
fn put(env: &Env, path: &str, contents: &str) {
    std::fs::write(env.root().join(path), contents).unwrap();
}

/// A project with `a.txt` saved as "First", then `a.txt` changed and `b.txt` added as "Second":
/// the two saves' change IDs.
fn two_saves() -> (Env, String, String) {
    let env = started();
    put(&env, "a.txt", "a\n");
    let (first, _) = run_at(&env, None, &["save", "First"]);
    put(&env, "a.txt", "A\n");
    put(&env, "b.txt", "b\n");
    let (second, _) = run_at(&env, None, &["save", "Second"]);
    let (first, second) = (saved_id(&first).to_owned(), saved_id(&second).to_owned());
    (env, first, second)
}

/// Whether "Changed your mind? bolt undo" ends what was said, as it does on a terminal only.
fn offers_undo(said: &str) -> bool {
    said.ends_with("\nChanged your mind? bolt undo\n")
}

/// The widths a command is tried at: none, then 60, 80 and 120 columns where a terminal can be
/// faked.
fn widths() -> Vec<Option<usize>> {
    let mut widths = vec![None];
    if cfg!(feature = "fault-injection") {
        widths.extend([Some(60), Some(80), Some(120)]);
    }
    widths
}

/// The change ID a save's outcome line names.
fn saved_id(said: &str) -> &str {
    said.split(' ').nth(1).unwrap()
}

#[test]
fn save_says_saved_then_the_id_the_message_and_the_line() {
    let env = started();
    for (n, width) in widths().into_iter().enumerate() {
        std::fs::write(env.root().join("a.txt"), format!("{n}\n")).unwrap();
        let (shown, _) = run_at(&env, width, &["save", "Add a"]);
        let id = saved_id(&shown);
        assert!(id.len() >= 4 && id.bytes().all(|b| (b'k'..=b'z').contains(&b)));
        assert_eq!(
            shown,
            format!("Saved {id} \"Add a\" on main  (1 file)\n"),
            "{width:?}"
        );
    }
}

#[test]
fn an_amend_says_updated_and_that_the_id_stays() {
    let env = started();
    std::fs::write(env.root().join("a.txt"), "a\n").unwrap();
    let (saved, _) = run_at(&env, None, &["save", "Add a"]);
    std::fs::write(env.root().join("b.txt"), "b\n").unwrap();
    let (updated, _) = run_at(&env, None, &["save", "--amend", "Add a and b"]);
    assert_eq!(
        updated,
        format!(
            "Updated save {} \"Add a and b\" on main  (2 files)\n\n    The ID stays the same.\n",
            saved_id(&saved)
        )
    );
}

#[test]
fn nothing_to_save_names_the_last_save_and_exits_0() {
    let env = started();
    std::fs::write(env.root().join("a.txt"), "a\n").unwrap();
    let (saved, _) = run_at(&env, None, &["save", "Add a"]);
    let (again, said) = run_at(&env, None, &["save", "Again"]);
    assert_eq!(
        again,
        format!(
            "Nothing to save: no changes since your last save, {} \"Add a\".\n",
            saved_id(&saved)
        )
    );
    assert_eq!(said, "");
}

#[test]
fn a_partial_save_says_what_it_left_out() {
    let env = started();
    std::fs::write(env.root().join("a.txt"), "a\n").unwrap();
    std::fs::write(env.root().join("b.txt"), "b\n").unwrap();
    let (shown, _) = run_at(&env, None, &["save", "Only a", "a.txt"]);
    assert_eq!(
        shown,
        format!(
            "Saved {} \"Only a\" on main  (1 file)\n\n    Only what you named was saved: bolt \
             status lists the changes left out.\n",
            saved_id(&shown)
        )
    );
}

#[test]
fn status_opens_with_where_you_are_then_lists_changes_in_columns() {
    let env = started();
    std::fs::write(env.root().join("a.txt"), "a\n").unwrap();
    std::fs::write(env.root().join("c.txt"), "c\n").unwrap();
    let (saved, _) = run_at(&env, None, &["save", "First"]);
    let id = saved_id(&saved);
    let (shown, _) = run_at(&env, None, &["status"]);
    assert_eq!(
        shown,
        format!("On line main at save {id} \"First\"  (just now)\nNothing changed since then.\n")
    );
    std::fs::write(env.root().join("a.txt"), "abc\n").unwrap();
    std::fs::remove_file(env.root().join("c.txt")).unwrap();
    std::fs::create_dir(env.root().join("sub")).unwrap();
    std::fs::write(env.root().join("sub/new.txt"), "n\n").unwrap();
    let (shown, said) = run_at(&env, None, &["status"]);
    let row =
        |kind: &str, path: &str, size: &str| format!("    {kind:<8}  {path:<11}  {size:>10}\n");
    assert_eq!(
        shown,
        format!(
            "On line main at save {id} \"First\"  (just now)\n\nChanged since then (3 files):\n\n{}{}{}",
            row("modified", "a.txt", "2 B -> 4 B"),
            row("removed", "c.txt", "2 B"),
            row("added", "sub/new.txt", "2 B"),
        )
    );
    assert_eq!(said, "");
}

#[test]
fn status_cuts_a_long_path_from_its_middle_only_on_a_terminal() {
    let env = started();
    let path = "Assets/Textures/Environment/Cliffs/Rocks/Cliff_Large_01_Albedo_4K.psd";
    std::fs::create_dir_all(env.root().join("Assets/Textures/Environment/Cliffs/Rocks")).unwrap();
    std::fs::write(env.root().join(path), "p\n").unwrap();
    for width in widths() {
        let (shown, said) = run_at(&env, width, &["status"]);
        let row = shown.lines().last().unwrap();
        match width {
            Some(w) if w < 100 => {
                assert!(row.starts_with("    added  Assets/.../"), "{w}: {shown}");
                assert!(
                    row.ends_with("/Cliff_Large_01_Albedo_4K.psd  2 B"),
                    "{w}: {shown}"
                );
                assert!(shown.lines().all(|l| l.len() <= w), "{w}: {shown}");
            }
            _ => assert_eq!(row, format!("    added  {path}  2 B"), "{width:?}"),
        }
        // A person at a terminal is told what to do next; a log isn't.
        assert_eq!(said.contains("Next: "), width.is_some(), "{said}");
    }
}

#[test]
fn history_marks_the_current_save_and_cuts_long_messages_at_the_end() {
    let env = started();
    std::fs::write(env.root().join("a.txt"), "a\n").unwrap();
    run_at(&env, None, &["save", "First"]);
    std::fs::write(env.root().join("a.txt"), "b\n").unwrap();
    let long = "word ".repeat(30);
    let long = long.trim_end();
    run_at(&env, None, &["save", long]);
    for width in widths() {
        let (shown, _) = run_at(&env, width, &["history"]);
        let lines: Vec<&str> = shown.lines().collect();
        assert_eq!(
            lines[..2],
            ["Saves on line main, newest first:", ""],
            "{shown}"
        );
        assert!(lines[2].starts_with("> "), "{shown}");
        assert!(lines[2].ends_with("  [main]"), "{shown}");
        assert!(
            lines[3].starts_with("  ") && lines[3].ends_with("  First"),
            "{shown}"
        );
        // The first save, made by `bolt start`, has no message.
        assert_eq!(lines.len(), 5, "{shown}");
        match width {
            Some(w) => {
                assert!(lines[2].ends_with("...  [main]"), "{w}: {shown}");
                assert!(lines.iter().all(|l| l.len() <= w), "{w}: {shown}");
            }
            None => assert!(lines[2].contains(long), "{shown}"),
        }
    }
}

#[test]
fn show_lists_what_a_save_changed_with_sizes_in_1024_units() {
    let env = started();
    std::fs::write(env.root().join("big.bin"), vec![b'x'; 2048]).unwrap();
    let (saved, _) = run_at(&env, None, &["save", "Big\n\nA body line."]);
    let id = saved_id(&saved);
    for width in widths() {
        let (shown, said) = run_at(&env, width, &["show"]);
        let lines: Vec<&str> = shown.lines().collect();
        assert_eq!(
            lines[..2],
            [format!("Save {id} \"Big\"").as_str(), ""],
            "{shown}"
        );
        assert!(lines[2].starts_with("    Author       Test <"), "{shown}");
        assert!(lines[3].starts_with("    Saved by     Test <"), "{shown}");
        assert_eq!(lines[4], "    Line         main", "{shown}");
        assert!(lines[5].starts_with("    After        "), "{shown}");
        let fingerprint = lines[6].strip_prefix("    Fingerprint  ").unwrap();
        assert!(fingerprint.len() == 64 && fingerprint.bytes().all(|b| b.is_ascii_hexdigit()));
        assert_eq!(
            lines[7..],
            [
                "",
                "        A body line.",
                "",
                "1 file changed:",
                "",
                "    added  big.bin  2.0 KiB"
            ],
            "{shown}"
        );
        assert_eq!(
            said.contains(&format!(
                "Next: see its lines with: bolt changes --save {id}"
            )),
            width.is_some(),
            "{said}"
        );
    }
}

#[cfg(feature = "fault-injection")]
#[test]
fn changes_colours_only_added_removed_and_hunk_lines() {
    let env = started();
    std::fs::write(env.root().join("a.txt"), "a\nb\nc\n").unwrap();
    std::fs::write(env.root().join("b.txt"), "b\n").unwrap();
    env.bolt().args(["save", "First"]).assert().success();
    std::fs::write(env.root().join("a.txt"), "a\nB\nc\n").unwrap();
    std::fs::write(env.root().join("b.txt"), "bb\n").unwrap();
    let out = bolt_on_a_terminal(&env, env.root())
        .arg("changes")
        .output()
        .unwrap();
    let shown = text(&out.stdout);
    let lines: Vec<&str> = shown.lines().collect();
    assert_eq!(
        lines[..7],
        [
            "\x1b[33mmodified\x1b[0m  \x1b[34ma.txt\x1b[0m  \x1b[32m+1\x1b[0m \x1b[31m-1\x1b[0m  \
             \x1b[2m(6 B → 6 B)\x1b[0m",
            "\x1b[36m@@ -1,3 +1,3 @@\x1b[0m",
            " a",
            "\x1b[31m-b\x1b[0m",
            "\x1b[32m+B\x1b[0m",
            " c",
            &format!("\x1b[2m{}\x1b[0m", "─".repeat(79)),
        ],
        "{shown}"
    );
    assert!(
        lines[7].starts_with("\x1b[33mmodified\x1b[0m  \x1b[34mb.txt"),
        "{shown}"
    );
    // At other widths the separator spans the width, up to 100 columns.
    for (width, rule) in [(60, 59), (120, 100)] {
        let (shown, _) = run_at(&env, Some(width), &["changes"]);
        assert!(
            shown.contains(&format!("\n{}\n", "-".repeat(rule))),
            "{shown}"
        );
    }
    // Without a terminal files are parted by an empty line, as no line of a file can be.
    let (shown, _) = run_at(&env, None, &["changes"]);
    assert!(
        shown.contains(" c\n\nmodified  b.txt  +1 -1  (2 B -> 3 B)\n"),
        "{shown}"
    );
}

/// The lines meant only for people (`Next:`) go to a terminal: a log sees the answer alone.
#[test]
fn each_of_these_commands_prints_no_new_line_without_a_terminal() {
    let env = started();
    std::fs::write(env.root().join("a.txt"), "a\n").unwrap();
    for args in [
        &["save", "First"][..],
        &["status"],
        &["changes"],
        &["history"],
        &["show"],
    ] {
        let (shown, said) = run_at(&env, None, args);
        assert_eq!(said, "", "{args:?}");
        assert!(!shown.contains("Next:"), "{args:?}: {shown}");
        assert!(!shown.contains('\x1b'), "{args:?}: {shown}");
        assert!(!shown.contains(SYMBOLS), "{args:?}: {shown}");
    }
    std::fs::write(env.root().join("a.txt"), "b\n").unwrap();
    let (_, said) = run_at(&env, None, &["status"]);
    assert_eq!(said, "");
    // The commands that end with "Changed your mind?" or a `Next:` on a terminal.
    let first = support::main_tip(&env).to_hex();
    run_at(&env, None, &["save", "Second"]);
    put(&env, "a.txt", "c\n");
    for args in [
        &["discard"][..],
        &["goto", &first[..12]],
        &["line"],
        &["goto", "main"],
        &["undo"],
        &["redo"],
        &["undo", "--list"],
        &["undo", "--list", "--all"],
        &["tidy"],
    ] {
        let (shown, said) = run_at(&env, None, args);
        assert_eq!(said, "", "{args:?}");
        assert!(!shown.contains('\x1b'), "{args:?}: {shown}");
        assert!(!shown.contains(SYMBOLS), "{args:?}: {shown}");
        assert!(!shown.contains("Changed your mind"), "{args:?}: {shown}");
    }
    put(&env, "a.txt", "d\n");
    for args in [&["shelf", "Work"][..], &["shelf", "--list"], &["unshelf"]] {
        let (shown, said) = run_at(&env, None, args);
        assert_eq!(said, "", "{args:?}");
        assert!(!shown.contains("Next:"), "{args:?}: {shown}");
        assert!(!shown.contains("Changed your mind"), "{args:?}: {shown}");
    }
    // A step `--shelve` sets work aside for: no `Next:` either.
    run_at(&env, None, &["discard"]);
    put(&env, "a.txt", "e\n");
    let (shown, said) = run_at(&env, None, &["undo", "--shelve"]);
    assert!(shown.starts_with("Shelved "), "{shown}");
    assert_eq!(said, "");
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

#[test]
fn goto_says_switched_to_and_how_many_files_it_wrote() {
    let (env, first, second) = two_saves();
    for width in widths() {
        let (shown, said) = run_at(&env, width, &["goto", &first]);
        let lines: Vec<&str> = shown.lines().collect();
        assert!(lines[0].starts_with("Switched to save "), "{shown}");
        assert!(
            lines[0].ends_with(" \"First\". You are not on a line."),
            "{shown}"
        );
        assert_eq!(
            lines[1..],
            [
                "",
                "    1 file written, 1 removed.",
                "    Saving here asks for a new line's name."
            ],
            "{shown}"
        );
        assert_eq!(offers_undo(&said), width.is_some(), "{said:?}");
        let (shown, said) = run_at(&env, width, &["goto", "main"]);
        assert_eq!(
            shown,
            format!("Switched to line main at save {second} \"Second\".\n\n    2 files written.\n")
        );
        assert_eq!(offers_undo(&said), width.is_some(), "{said:?}");
    }
    // Unsaved work that doesn't collide comes along; at a line's newest save, the line is named.
    put(&env, "c.txt", "mine\n");
    let tip = support::main_tip(&env).to_hex();
    run_at(&env, None, &["goto", &first]);
    let (shown, _) = run_at(&env, None, &["goto", &tip[..12]]);
    assert!(
        shown.ends_with(
            " \"Second\". You are not on a line.\n\n    2 files written.\n    Your unsaved changes \
             in 1 file came along.\n    Saving here asks for a new line's name.\n    To keep \
             saving on main: bolt goto main\n"
        ),
        "{shown}"
    );
}

#[test]
fn goto_stopped_by_unsaved_work_lists_the_files_and_changes_nothing() {
    let (env, first, _) = two_saves();
    put(&env, "b.txt", "mine\n");
    for width in widths() {
        let (shown, said) = failing_at(&env, width, &["goto", &first]);
        assert_eq!(shown, "", "{width:?}");
        assert!(
            said.starts_with("error: Your unsaved changes, or files Bolt ignores,"),
            "{said}"
        );
        assert!(said.lines().any(|l| l.trim() == "b.txt"), "{said}");
        assert!(said.contains("\n\nhint: Save them first"), "{said}");
        assert!(!said.contains("Changed your mind"), "{said}");
        if let Some(w) = width {
            assert!(said.lines().all(|l| l.len() <= w), "{said}");
        }
        let b = std::fs::read_to_string(env.root().join("b.txt")).unwrap();
        assert_eq!(b, "mine\n");
    }
}

#[test]
fn discard_says_threw_away_and_that_undo_brings_it_back() {
    let (env, _, _) = two_saves();
    for (n, width) in widths().into_iter().enumerate() {
        put(&env, "a.txt", &format!("mine {n}\n"));
        let (shown, said) = run_at(&env, width, &["discard"]);
        assert_eq!(
            shown,
            "Threw away your unsaved changes to a.txt.\n\n    A safety copy of it was kept.\n",
            "{width:?}"
        );
        assert_eq!(offers_undo(&said), width.is_some(), "{said:?}");
        let (shown, _) = run_at(&env, None, &["undo"]);
        assert_eq!(shown, "Undid discard: 1 file back.\n");
        let a = std::fs::read_to_string(env.root().join("a.txt")).unwrap();
        assert_eq!(a, format!("mine {n}\n"));
        run_at(&env, None, &["discard"]);
    }
    // Several files are counted.
    put(&env, "a.txt", "mine\n");
    put(&env, "n.txt", "new\n");
    let (shown, _) = run_at(&env, None, &["discard"]);
    assert_eq!(
        shown,
        "Threw away your unsaved changes to 2 files.\n\n    A safety copy of each was kept.\n"
    );
}

#[test]
fn line_marks_the_current_line_with_the_marker() {
    let (env, first, _) = two_saves();
    run_at(&env, None, &["goto", &first]);
    put(&env, "c.txt", "c\n");
    run_at(&env, None, &["save", "--line", "feature", "Feature"]);
    let name = folder_name(&env);
    for width in widths() {
        let (shown, said) = run_at(&env, width, &["line"]);
        assert_eq!(
            shown,
            format!("Lines in {name}:\n\n> feature\n  main\n"),
            "{width:?}"
        );
        assert_eq!(said, "");
    }
    // At a save no line is marked, and the outcome says where you are.
    let tip = support::main_tip(&env).to_hex();
    run_at(&env, None, &["goto", &tip[..12]]);
    let (shown, _) = run_at(&env, None, &["line"]);
    assert_eq!(
        shown,
        format!(
            "Lines in {name}. You are at save {}, not on a line.\n\n  feature\n  main\n",
            &tip[..12]
        )
    );
    #[cfg(feature = "fault-injection")]
    {
        run_at(&env, None, &["goto", "feature"]);
        let out = bolt_on_a_terminal(&env, env.root())
            .arg("line")
            .output()
            .unwrap();
        let shown = text(&out.stdout);
        assert!(
            shown.contains("\n\x1b[1m▸\x1b[0m \x1b[34mfeature\x1b[0m\n  \x1b[34mmain\x1b[0m\n"),
            "{shown:?}"
        );
    }
}

#[test]
fn undo_and_redo_say_what_they_took_back_or_brought_back() {
    let (env, _, _) = two_saves();
    for width in widths() {
        let (shown, said) = run_at(&env, width, &["undo"]);
        assert!(shown.starts_with("Undid save "), "{shown}");
        assert!(
            shown.contains(" \"Second\": main is back at ")
                && shown.ends_with("; its changes show as unsaved.\n"),
            "{shown}"
        );
        assert_eq!(shown.lines().count(), 1, "{shown}");
        assert_eq!(said, "");
        let (shown, said) = run_at(&env, width, &["redo"]);
        assert!(shown.starts_with("Redid save "), "{shown}");
        assert!(shown.contains(" \"Second\": main is at "), "{shown}");
        assert_eq!(said, "");
    }
    // Several steps are counted, then listed without their verb.
    let (shown, _) = run_at(&env, None, &["undo", "2"]);
    let lines: Vec<&str> = shown.lines().collect();
    assert_eq!(lines[..2], ["Undid 2 steps:", ""], "{shown}");
    assert!(
        lines[2].starts_with("    save ") && lines[2].contains(" \"Second\": "),
        "{shown}"
    );
    assert!(
        lines[3].starts_with("    save ") && lines[3].contains(" \"First\": "),
        "{shown}"
    );
    assert_eq!(lines.len(), 4, "{shown}");
    // Fewer than asked for: all there were.
    let (shown, _) = run_at(&env, None, &["redo", "5"]);
    let lines: Vec<&str> = shown.lines().collect();
    assert_eq!(
        lines[..2],
        ["Redid 2 steps (all there were):", ""],
        "{shown}"
    );
    assert_eq!(lines.len(), 4, "{shown}");
    // Unsaved work in the way stops it; `--shelve` sets the work aside, and says so first.
    put(&env, "a.txt", "mine\n");
    run_at(&env, None, &["discard"]);
    put(&env, "a.txt", "again\n");
    let (shown, said) = failing_at(&env, None, &["undo"]);
    assert_eq!(shown, "");
    assert!(said.contains("bolt undo --shelve"), "{said}");
    let width = *widths().last().unwrap();
    let (shown, said) = run_at(&env, width, &["undo", "--shelve"]);
    assert_eq!(
        shown,
        "Shelved your unsaved changes in the way on shelf 1.\nUndid discard: 1 file back.\n"
    );
    assert_eq!(
        said.ends_with("\nNext: bring your changes back with: bolt unshelf\n"),
        width.is_some(),
        "{said:?}"
    );
}

/// A row of a list as a log gets it (`whole`), and as a terminal `width` columns wide does
/// (`shown`): the same, or cut at its end to the width.
fn cut_to(whole: &str, shown: &str, width: usize) {
    if whole.chars().count() <= width {
        assert_eq!(shown, whole, "{width}");
    } else {
        assert_eq!(shown.chars().count(), width, "{whole}\n{shown}");
        let kept = shown
            .strip_suffix("...")
            .unwrap_or_else(|| panic!("{shown}"));
        assert!(whole.starts_with(kept), "{whole}\n{shown}");
    }
}

#[test]
fn undo_list_numbers_the_steps_and_marks_where_you_are() {
    let env = started();
    for name in ["b", "c", "d"] {
        put(&env, &format!("{name}.txt"), "x\n");
        run_at(&env, None, &["save", &format!("Add {name}")]);
    }
    run_at(&env, None, &["undo", "2"]);
    let (shown, said) = run_at(&env, None, &["undo", "--list"]);
    assert_eq!(said, "");
    let lines: Vec<&str> = shown.lines().collect();
    let heading = format!(
        "Undo diary for {}, newest first. Steps you can redo come first.",
        folder_name(&env)
    );
    assert_eq!(lines[..2], [heading.as_str(), ""], "{shown}");
    // Newest first: the steps redo can bring back, where you are, then those undo can take.
    let numbered = [
        (lines[2], "redo 2", "\"Add d\""),
        (lines[3], "redo 1", "\"Add c\""),
        (lines[5], "undo 1", "\"Add b\""),
    ];
    for (line, label, message) in numbered {
        let rest = line.strip_prefix(&format!("  {label}  ")).unwrap();
        let (entry, rest) = rest.split_once("  ").unwrap();
        assert!(
            entry.len() == 12 && entry.bytes().all(|b| b.is_ascii_hexdigit()),
            "{shown}"
        );
        assert!(
            rest.starts_with("just now  save: main moved to "),
            "{shown}"
        );
        assert!(rest.ends_with(message), "{shown}");
    }
    assert_eq!(lines[4], "> you are here", "{shown}");
    // The diary's first entry has no number: undo can't take it.
    assert!(
        lines[6].starts_with(&format!("{}  ", " ".repeat(8))),
        "{shown}"
    );
    assert!(
        lines[6].ends_with("  start: the diary's first entry, which can't be undone"),
        "{shown}"
    );
    assert_eq!(lines.len(), 7, "{shown}");
    // On a terminal what a step did is cut at its end to fit; nothing else changes.
    for width in widths().into_iter().flatten() {
        let (cut, said) = run_at(&env, Some(width), &["undo", "--list"]);
        assert_eq!(said, "");
        let rows: Vec<&str> = cut.lines().collect();
        assert_eq!(rows.len(), lines.len(), "{cut}");
        for (whole, shown) in lines.iter().zip(rows).skip(2) {
            cut_to(whole, shown, width);
        }
    }
}

#[test]
fn shelf_and_unshelf_say_the_shelfs_number() {
    let (env, _, _) = two_saves();
    for width in widths() {
        put(&env, "a.txt", "mine\n");
        let (shown, said) = run_at(&env, width, &["shelf", "Work"]);
        assert_eq!(
            shown,
            "Shelved your unsaved changes in 1 file on shelf 1.\n"
        );
        assert_eq!(
            said.ends_with("\nNext: bring them back with: bolt unshelf\n"),
            width.is_some(),
            "{said:?}"
        );
        let (shown, said) = run_at(&env, width, &["unshelf"]);
        assert_eq!(shown, "Unshelved shelf 1 \"Work\": 1 file written.\n");
        assert_eq!(offers_undo(&said), width.is_some(), "{said:?}");
        run_at(&env, None, &["discard"]);
    }
    // A new shelf takes the smallest free number, and the others keep theirs.
    for n in ["1", "2", "3"] {
        put(&env, "a.txt", n);
        run_at(&env, None, &["shelf"]);
    }
    let (shown, _) = run_at(&env, None, &["unshelf", "2"]);
    assert_eq!(shown, "Unshelved shelf 2 \"Shelved\": 1 file written.\n");
    run_at(&env, None, &["discard"]);
    put(&env, "a.txt", "4");
    let (shown, _) = run_at(&env, None, &["shelf"]);
    assert_eq!(
        shown,
        "Shelved your unsaved changes in 1 file on shelf 2.\n"
    );
}

#[test]
fn shelf_list_aligns_its_columns() {
    let (env, _, _) = two_saves();
    put(&env, "a.txt", "mine\n");
    run_at(&env, None, &["shelf", "Work on login\nwith details"]);
    put(&env, "a.txt", "x\n");
    put(&env, "n.txt", "new\n");
    run_at(&env, None, &["shelf"]);
    let long = "word ".repeat(16);
    let long = long.trim_end();
    put(&env, "a.txt", "y\n");
    run_at(&env, None, &["shelf", long]);
    let (shown, said) = run_at(&env, None, &["shelf", "--list"]);
    assert_eq!(said, "");
    let row = |n: &str, message: &str, files: &str| {
        let quoted = format!("\"{message}\"");
        format!(
            "    {n}  {quoted:<w$}  {files:<7}  just now  on main",
            w = long.len() + 2
        )
    };
    let rows = [
        row("3", long, "1 file"),
        row("2", "Shelved", "2 files"),
        row("1", "Work on login", "1 file"),
    ];
    assert_eq!(
        shown,
        format!("3 shelves, newest first:\n\n{}\n", rows.join("\n"))
    );
    // On a terminal the message gives way, and the rows still line up.
    for width in widths().into_iter().flatten() {
        let (cut, _) = run_at(&env, Some(width), &["shelf", "--list"]);
        let lines: Vec<&str> = cut.lines().skip(2).collect();
        assert_eq!(lines.len(), 3, "{cut}");
        for line in &lines {
            assert!(line.len() <= width, "{width}: {cut}");
            assert!(line.ends_with("  just now  on main"), "{cut}");
        }
        let facts: Vec<usize> = lines.iter().map(|l| l.len()).collect();
        assert!(facts.iter().all(|&len| len == facts[0]), "{cut}");
        assert_eq!(lines[0].contains("...\""), rows[0].len() > width, "{cut}");
    }
}

#[cfg(feature = "fault-injection")]
#[test]
fn changed_your_mind_shows_only_on_a_terminal() {
    let (env, first, _) = two_saves();
    let on_terminal = |args: &[&str]| {
        let out = bolt_on_a_terminal(&env, env.root())
            .args(args)
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
        text(&out.stderr)
    };
    let offered = "\nChanged your mind? \x1b[36mbolt undo\x1b[0m\n";
    put(&env, "a.txt", "mine\n");
    assert!(on_terminal(&["discard"]).ends_with(offered));
    assert!(on_terminal(&["goto", &first]).ends_with(offered));
    assert!(on_terminal(&["goto", "main"]).ends_with(offered));
    put(&env, "a.txt", "mine\n");
    run_at(&env, None, &["shelf"]);
    assert!(on_terminal(&["unshelf"]).ends_with(offered));
    // Nothing thrown away is nothing to change your mind about; other commands don't offer it.
    run_at(&env, None, &["discard"]);
    assert_eq!(on_terminal(&["discard"]), "");
    put(&env, "a.txt", "third\n");
    for args in [&["save", "Third"][..], &["undo"], &["redo"], &["tidy"]] {
        assert!(!on_terminal(args).contains("Changed your mind"), "{args:?}");
    }
    // Never beside --json, nor in a log.
    put(&env, "a.txt", "mine\n");
    assert_eq!(on_terminal(&["--json", "discard"]), "");
    put(&env, "a.txt", "mine\n");
    assert_eq!(run_at(&env, None, &["discard"]).1, "");
}

#[cfg(feature = "fault-injection")]
#[test]
fn each_of_these_commands_fits_60_80_and_120_columns() {
    let env = started();
    let deep = "Assets/Textures/Environment/Cliffs/Rocks/Cliff_Large_01_Albedo_4K.psd";
    std::fs::create_dir_all(env.root().join("Assets/Textures/Environment/Cliffs/Rocks")).unwrap();
    let long = "Add the large cliff textures for the second level, and the rocks around it";
    put(&env, deep, "a");
    run_at(&env, None, &["save", long]);
    let first = support::main_tip(&env).to_hex();
    put(&env, deep, "b");
    run_at(&env, None, &["save", long]);
    for width in [60, 80, 120] {
        let run = |args: &[&str]| {
            let (shown, said) = run_at(&env, Some(width), args);
            for line in said.lines() {
                assert!(line.len() <= width, "{args:?} at {width}: {said}");
            }
            // The rows of a list are cut to the width.
            if args[1..].contains(&"--list") {
                for row in shown.lines().skip(2) {
                    assert!(row.len() <= width, "{args:?} at {width}: {shown}");
                }
            }
        };
        put(&env, deep, "mine");
        run(&["discard"]);
        run(&["goto", &first[..12]]);
        run(&["line"]);
        run(&["goto", "main"]);
        run(&["undo"]);
        run(&["redo"]);
        run(&["undo", "--list"]);
        put(&env, deep, "mine");
        run(&["shelf", long]);
        run(&["shelf", "--list"]);
        run(&["unshelf"]);
        run_at(&env, None, &["discard"]);
        run(&["tidy"]);
    }
}

/// A Git repository whose import renames a line, leaves a ref out, and warns of a file that only
/// looks like a Git LFS pointer.
#[cfg(feature = "fault-injection")]
fn a_repository_with_much_to_say() -> Env {
    let env = Env::new();
    let tracked = "*.psd filter=lfs diff=lfs merge=lfs -text\n";
    env.commit(".gitattributes", tracked, "Track art with Git LFS");
    let looks = format!(
        "version https://git-lfs.github.com/spec/v1\noid sha256:{}\nsize 1\nand more\n",
        "a".repeat(64)
    );
    env.commit("art/a.psd", &looks, "Add a look-alike");
    env.git(&["branch", "feature+x"]);
    env.git(&["update-ref", "refs/notes/commits", "HEAD"]);
    env
}

/// What `text` says, without the spaces and line breaks that lay it out.
#[cfg(feature = "fault-injection")]
fn words(text: &str) -> Vec<&str> {
    text.split_whitespace().collect()
}

#[cfg(feature = "fault-injection")]
#[test]
fn the_import_summary_fits_60_80_and_120_columns() {
    let (logged, warned) = run_at(&a_repository_with_much_to_say(), None, &["import"]);
    assert!(
        logged.starts_with("Imported 2 commits from Git: 2 lines, 0 tags.\n\n    Renamed "),
        "{logged}"
    );
    assert!(logged.ends_with("\n\nYou're on line main.\n"), "{logged}");
    assert!(warned.starts_with("warning: A file looks like"), "{warned}");
    assert!(!warned.contains("Next:"), "{warned}");
    for width in [60, 80, 120] {
        let env = a_repository_with_much_to_say();
        let (shown, said) = run_at(&env, Some(width), &["import"]);
        for line in shown.lines().chain(said.lines()) {
            assert!(line.chars().count() <= width, "{width}: {line:?}");
        }
        // Only laid out anew: every word is there, in its place.
        assert_eq!(words(&shown), words(&logged), "{width}: {shown}");
        assert_eq!(shown.lines().next(), logged.lines().next(), "{width}");
        for line in shown.lines().skip(2).take_while(|l| !l.is_empty()) {
            assert!(line.starts_with("    "), "{width}: {shown}");
        }
        assert!(said.starts_with("\nwarning: "), "{width}: {said}");
        assert!(
            said.ends_with("\n\nNext: look around with: bolt status\n"),
            "{width}: {said}"
        );
    }
}

/// Help is the same text on a terminal of any width as in a log; on a terminal that shows colour
/// its titles are bold, and nothing else is styled.
#[test]
fn help_reads_the_same_at_every_width_with_only_its_titles_bold() {
    let env = Env::empty();
    for (args, titles) in [
        (
            &["help"][..],
            &["Every day", "Lines, shelves and details", "The project"][..],
        ),
        (
            &["help", "save"],
            &["Examples", "Usage", "Arguments", "Options"],
        ),
    ] {
        let (logged, said) = run_at(&env, None, args);
        assert_eq!(said, "", "{args:?}");
        assert!(!logged.contains('\x1b'), "{args:?}");
        for width in widths() {
            assert_eq!(
                run_at(&env, width, args),
                (logged.clone(), String::new()),
                "{args:?} at {width:?}"
            );
        }
        #[cfg(feature = "fault-injection")]
        {
            let out = bolt_on_a_terminal(&env, env.root())
                .args(args)
                .output()
                .unwrap();
            let shown = text(&out.stdout);
            for title in titles {
                assert!(
                    shown.contains(&format!("\n\x1b[1m{title}\x1b[0m\n")),
                    "{title}: {shown:?}"
                );
            }
            assert_eq!(shown.matches('\x1b').count(), titles.len() * 2, "{shown:?}");
            assert_eq!(shown.replace("\x1b[1m", "").replace("\x1b[0m", ""), logged);
        }
        let _ = titles;
    }
}

/// The project's `state/head`, which changes when a command records a step.
fn head_file(env: &Env) -> Vec<u8> {
    std::fs::read(env.root().join(".bolt/state/head")).unwrap()
}

#[test]
fn goto_where_the_project_already_is_says_so_and_offers_no_undo() {
    let (env, _, second) = two_saves();
    let before = head_file(&env);
    for width in widths() {
        let (shown, said) = run_at(&env, width, &["goto", "main"]);
        assert_eq!(
            shown,
            format!("Already on line main, at save {second} \"Second\".\n")
        );
        assert!(!said.contains("Changed your mind"), "{said:?}");
        // No step was recorded, so there was nothing to offer to undo.
        assert_eq!(head_file(&env), before);
    }
    let width = *widths().last().unwrap();
    // At a save: the same, at the save.
    let tip = support::main_tip(&env).to_hex();
    run_at(&env, None, &["goto", &tip[..12]]);
    let at = head_file(&env);
    let (shown, said) = run_at(&env, width, &["goto", &tip[..12]]);
    assert!(
        shown.starts_with("Already at save ") && shown.ends_with(" \"Second\".\n"),
        "{shown}"
    );
    assert!(!said.contains("Changed your mind"), "{said:?}");
    assert_eq!(head_file(&env), at);
    // From the save to the line whose newest it is, only the position changes: a step all the
    // same, which undo takes back.
    let (shown, said) = run_at(&env, width, &["goto", "main"]);
    assert_eq!(
        shown,
        format!(
            "Switched to line main at save {second} \"Second\".\n\n    No file needed changing.\n"
        )
    );
    assert_eq!(offers_undo(&said), width.is_some(), "{said:?}");
    assert_ne!(head_file(&env), at);
    run_at(&env, None, &["undo"]);
    let (shown, _) = run_at(&env, None, &["goto", &tip[..12]]);
    assert!(shown.starts_with("Already at save "), "{shown}");
}

#[test]
fn undo_that_shelves_more_than_once_points_at_the_shelf_list() {
    let (env, _, _) = two_saves();
    // Two discards, then unsaved work: undoing each discard writes over work in the way.
    put(&env, "a.txt", "mine 1\n");
    run_at(&env, None, &["discard"]);
    put(&env, "a.txt", "mine 2\n");
    run_at(&env, None, &["discard"]);
    put(&env, "a.txt", "mine 3\n");
    let width = *widths().last().unwrap();
    let (shown, said) = run_at(&env, width, &["undo", "2", "--shelve"]);
    assert!(
        shown.starts_with(
            "Shelved your unsaved changes in the way on shelf 1.\nShelved your unsaved changes in \
             the way on shelf 2.\nUndid 2 steps:\n"
        ),
        "{shown}"
    );
    assert_eq!(
        said.ends_with("\nNext: see the shelves with: bolt shelf --list\n"),
        width.is_some(),
        "{said:?}"
    );
    assert!(!said.contains("bolt unshelf"), "{said:?}");
}

#[cfg(feature = "fault-injection")]
#[test]
fn unshelf_that_only_finishes_a_stopped_switch_offers_no_undo() {
    let (env, _, _) = two_saves();
    put(&env, "a.txt", "mine\n");
    run_at(&env, None, &["shelf"]);
    let stopped = env
        .bolt()
        .arg("unshelf")
        .env("BOLT_FAULT_FAIL", "switch:changed")
        .output()
        .unwrap();
    assert_eq!(stopped.status.code(), Some(1), "{}", text(&stopped.stderr));
    let (before, _) = support::head(&env);
    let (shown, said) = run_at(&env, Some(80), &["unshelf"]);
    assert!(shown.starts_with("Finished the unshelf"), "{shown}");
    assert!(!said.contains("Changed your mind"), "{said:?}");
    // It recorded no step of its own: `head` moved only to the entry the stopped unshelf wrote.
    let (_, now) = support::head(&env);
    assert_eq!(
        (now.command.as_str(), now.parent),
        ("unshelf", Some(before))
    );
}

/// A left-out entry whose name reads like one of the summary's own details never starts a line
/// of the summary: it isn't broken, and a name that doesn't fit beside its heading gets a row of
/// its own, further in.
#[cfg(feature = "fault-injection")]
#[test]
fn a_name_from_the_repository_never_passes_for_a_detail_line() {
    let path = "x Brought in 41 Git LFS files (11 GiB) from .git/lfs/objects. y";
    for width in [60, 80, 120] {
        let env = Env::new();
        let stream = format!(
            "blob\nmark :1\ndata 2\na\n\ncommit refs/heads/main\nmark :2\nauthor Ada Lovelace \
             <ada@example.com> 1700000000 +0000\ncommitter Ada Lovelace <ada@example.com> \
             1700000000 +0000\ndata 6\nFirst\n\nM 100644 :1 a.txt\nM 100644 :1 \
             {path}/.g\u{200c}it/x\n\n"
        );
        let args = ["-c", "core.protectHFS=false", "fast-import", "--quiet"];
        let made = support::git_with_input(env.root(), &args, stream.as_bytes());
        assert!(made.status.success(), "{}", text(&made.stderr));
        let (shown, _) = run_at(&env, Some(width), &["import"]);
        let whole = format!("`{path}/.g\\u{{200c}}it` in commit ");
        let named: Vec<&str> = shown.lines().filter(|l| l.contains("Brought")).collect();
        assert_eq!(named.len(), 1, "{width}: {shown}");
        assert!(
            named[0].starts_with(&format!("        {whole}")),
            "{width}: {shown}"
        );
        for line in shown.lines() {
            if let Some(rest) = line.strip_prefix("    ") {
                assert!(!rest.starts_with(['`', 'x']), "{width}: {shown}");
            }
        }
    }
}

/// `sentence`, which `logged` holds whole on a line `under` columns in, is broken in `shown` (60
/// columns) with every later line `under` columns in.
#[cfg(feature = "fault-injection")]
fn wraps_under(logged: &str, shown: &str, start: &str, under: usize) {
    let whole = logged.lines().find(|l| l.contains(start)).unwrap();
    let sentence = &whole[under..];
    let lines: Vec<&str> = shown.lines().collect();
    let at = lines.iter().position(|l| l.contains(start)).unwrap();
    let mut joined = lines[at][under..].to_owned();
    assert_ne!(joined, sentence, "not wrapped: {shown}");
    let pad = " ".repeat(under);
    for line in &lines[at + 1..] {
        if joined.len() >= sentence.len() {
            break;
        }
        let rest = line.strip_prefix(&pad).unwrap_or_else(|| panic!("{shown}"));
        assert!(!rest.starts_with(' '), "{shown}");
        joined = format!("{joined} {rest}");
    }
    assert_eq!(joined, sentence, "{shown}");
    // The outcome line, which may name a long folder, is never wrapped; details are.
    for line in lines.iter().filter(|l| l.starts_with("    ")) {
        assert!(line.chars().count() <= 60, "{line:?}");
    }
}

#[cfg(feature = "fault-injection")]
#[test]
fn detail_sentences_wrap_under_their_text_at_60_columns() {
    let start = ["start", "--preset", "custom"];
    let (logged, _) = run_at(&Env::empty(), None, &start);
    let (shown, _) = run_at(&Env::empty(), Some(60), &start);
    // A bullet's later lines start under its text.
    wraps_under(&logged, &shown, "Files of ", 6);
    wraps_under(&logged, &shown, "There are no rule files.", 4);

    let env = started();
    put(&env, "a.txt", "1\n");
    put(&env, "b.txt", "1\n");
    let (logged, _) = run_at(&env, None, &["save", "One", "a.txt"]);
    put(&env, "a.txt", "2\n");
    let (shown, _) = run_at(&env, Some(60), &["save", "Two", "a.txt"]);
    wraps_under(&logged, &shown, "Only what you named", 4);

    for (args, start, under) in [
        (&["verify"][..], " stored objects in ", 6),
        (&["tidy"], " changed in the last 24 hours ", 4),
    ] {
        let (logged, _) = run_at(&env, None, args);
        let (shown, _) = run_at(&env, Some(60), args);
        wraps_under(&logged, &shown, start, under);
    }
}

/// On a terminal an import shows each of its passes; the summary names none, and a log sees no
/// progress at all.
#[cfg(feature = "fault-injection")]
#[test]
fn an_import_shows_each_pass_on_a_terminal_and_nothing_in_a_log() {
    let labels = [
        "Reading Git's history",
        "Checking Git LFS files",
        "Importing commits",
    ];
    for terminal in [true, false] {
        let env = Env::new();
        env.commit("a.txt", "a\n", "First");
        env.commit("b.txt", "b\n", "Second");
        // A sign of Git LFS, so the import checks for its files too.
        std::fs::create_dir(env.root().join(".git/lfs")).unwrap();
        let mut cmd = if terminal {
            bolt_at_width(&env, env.root(), 120)
        } else {
            bolt_in_a_log(&env, env.root())
        };
        let out = cmd
            .env("BOLT_PROGRESS_QUIET_MS", "0")
            .arg("import")
            .output()
            .unwrap();
        let (shown, said) = (text(&out.stdout), text(&out.stderr));
        assert_eq!(out.status.code(), Some(0), "{said}");
        for label in labels {
            assert!(!shown.contains(label), "{shown}");
            assert_eq!(said.contains(label), terminal, "{label}: {said:?}");
        }
        if !terminal {
            assert_eq!(said, "");
        }
    }
}
