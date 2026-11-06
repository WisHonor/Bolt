//! `bolt changes`, end to end.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;

mod support;
use support::{Env, text};

/// A started project with `files` saved.
fn project(files: &[(&str, &[u8])]) -> Env {
    let env = Env::empty();
    env.bolt().arg("start").assert().success();
    for (name, bytes) in files {
        fs::write(env.root().join(name), bytes).unwrap();
    }
    env.bolt().args(["save", "Files"]).assert().success();
    env
}

/// `bolt changes` with `args`, which must succeed; returns what it printed.
fn changes(env: &Env, args: &[&str]) -> String {
    let out = env.bolt().arg("changes").args(args).output().unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    text(&out.stdout)
}

fn numbered(lines: std::ops::RangeInclusive<u32>) -> String {
    lines.map(|n| format!("line {n}\n")).collect()
}

#[test]
fn a_changed_line_shows_with_context() {
    let env = project(&[("a.txt", numbered(1..=10).as_bytes())]);
    env.overwrite(
        "a.txt",
        &numbered(1..=10).replace("line 5\n", "LINE FIVE\n"),
    );
    let out = changes(&env, &[]);
    assert!(out.contains("modified  a.txt"), "{out}");
    assert!(out.contains("@@ -2,7 +2,7 @@\n line 2\n line 3\n line 4\n-line 5\n+LINE FIVE\n line 6\n line 7\n line 8\n"), "{out}");
    assert!(
        !out.contains("line 1\n") && !out.contains("line 9"),
        "{out}"
    );
    // Only the paths given.
    fs::write(env.root().join("b.txt"), "b\n").unwrap();
    let only = changes(&env, &["b.txt"]);
    assert!(only.contains("+b") && !only.contains("a.txt"), "{only}");
}

#[test]
fn a_binary_file_shows_without_lines() {
    let env = project(&[("a.bin", b"one\0two\n")]);
    fs::write(env.root().join("a.bin"), b"one\0three\n").unwrap();
    let out = changes(&env, &[]);
    assert!(out.contains("a.bin") && out.contains("binary"), "{out}");
    assert!(!out.contains("@@") && !out.contains("three"), "{out}");
}

#[test]
fn a_file_over_64_mib_shows_without_lines() {
    let env = project(&[("a.txt", b"a\n")]);
    let big = fs::File::create(env.root().join("big.txt")).unwrap();
    big.set_len((64 << 20) + 1).unwrap();
    drop(big);
    let out = changes(&env, &[]);
    assert!(
        out.contains("big.txt") && out.contains("over 64 MiB"),
        "{out}"
    );
    assert!(!out.contains("@@") && !out.contains("binary"), "{out}");
}

#[cfg(feature = "fault-injection")]
#[test]
fn a_slow_diff_is_cut_at_its_deadline_and_said() {
    // A deadline already passed stands for a diff that ran out of time.
    let env = project(&[("a.txt", numbered(1..=50).as_bytes())]);
    env.overwrite("a.txt", &numbered(1..=50).replace("line 25\n", "x\n"));
    let out = env
        .bolt()
        .arg("changes")
        .env("BOLT_DIFF_DEADLINE_MS", "0")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    assert!(text(&out.stdout).contains("+x"), "{}", text(&out.stdout));
    assert!(
        text(&out.stderr).contains("warning: Bolt stopped comparing a.txt"),
        "{}",
        text(&out.stderr)
    );
}

#[test]
fn control_characters_in_lines_are_escaped() {
    let env = project(&[("a.txt", b"plain\n")]);
    env.overwrite("a.txt", "\x1b[31mred\x07\tand a tab\r\n");
    let out = changes(&env, &[]);
    assert!(out.contains("+\\x1b[31mred\\x07\tand a tab\n"), "{out}");
    assert!(!out.contains('\x1b') && !out.contains('\x07'), "{out}");
}

#[test]
fn changes_of_a_save_compare_with_its_first_parent() {
    let env = project(&[("a.txt", b"first\n")]);
    env.overwrite("a.txt", "second\n");
    env.bolt().args(["save", "Second"]).assert().success();
    env.overwrite("a.txt", "unsaved\n");
    let out = changes(&env, &["--save", "main"]);
    assert!(out.contains("-first\n+second\n"), "{out}");
    assert!(!out.contains("unsaved"), "{out}");
    // A path is never taken for a save.
    let path = changes(&env, &["a.txt"]);
    assert!(path.contains("-second\n+unsaved\n"), "{path}");
}

#[test]
fn only_line_endings_changed_is_said() {
    let env = project(&[("a.txt", b"one\r\ntwo\r\n")]);
    env.overwrite("a.txt", "one\ntwo\n");
    let out = changes(&env, &[]);
    assert!(
        out.contains("only line endings changed: CRLF to LF"),
        "{out}"
    );
}

#[test]
fn changes_json_matches_its_snapshot() {
    let env = project(&[("a.txt", b"one\ntwo\nthree\n"), ("b.bin", b"\0")]);
    env.overwrite("a.txt", "one\n2\nthree\n");
    fs::write(env.root().join("b.bin"), b"\0\0").unwrap();
    fs::write(env.root().join("c.txt"), "new").unwrap();
    let out = env.bolt().args(["--json", "changes"]).output().unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    let mut doc: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let redact = |v: &mut serde_json::Value| *v = "[redacted]".into();
    let save = &mut doc["save"];
    for key in ["id", "change_id", "change_id_short"] {
        redact(&mut save[key]);
    }
    for person in ["author", "saver"] {
        for key in ["time_ms", "tz_offset_min"] {
            redact(&mut save[person][key]);
        }
    }
    insta::assert_json_snapshot!(doc);
}
