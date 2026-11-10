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

/// What `args` print on stdout and stderr, in a log or on a terminal `width` columns wide.
fn run_at(env: &Env, width: Option<usize>, args: &[&str]) -> (String, String) {
    let mut cmd = match width {
        None => bolt_in_a_log(env, env.root()),
        #[cfg(feature = "fault-injection")]
        Some(width) => bolt_at_width(env, env.root(), width),
        #[cfg(not(feature = "fault-injection"))]
        Some(_) => unreachable!("a terminal is faked only with fault injection"),
    };
    let out = cmd.args(args).output().unwrap();
    let said = text(&out.stderr);
    assert_eq!(out.status.code(), Some(0), "{args:?}: {said}");
    (text(&out.stdout), said)
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
