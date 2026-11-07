//! `bolt changes`, end to end.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;
use std::io::Read;
use std::process::Stdio;
use std::time::{Duration, Instant};

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
    assert_eq!(
        text(&out.stderr),
        "warning: Bolt stopped comparing a.txt line by line after 2 seconds, so its changes \
         show in bigger pieces than they could.\n"
    );
}

#[test]
fn a_file_of_too_many_lines_shows_without_lines_quickly() {
    let env = project(&[("a.txt", b"a\n")]);
    // 60 MiB of empty lines: under the size limit, but split into lines it would take gigabytes.
    fs::write(env.root().join("a.txt"), vec![b'\n'; 60 << 20]).unwrap();
    let started = Instant::now();
    let out = changes(&env, &[]);
    assert!(
        out.contains("a.txt") && out.contains("over 1,000,000 lines: no lines shown"),
        "{out}"
    );
    assert!(!out.contains("@@"), "{out}");
    assert!(started.elapsed() < Duration::from_secs(30));
    // Under a cap of 512 MB, where the cap is enforced: the file held whole is 60 MiB.
    #[cfg(unix)]
    {
        let cmd = support::bolt_capped_at(env.config(), env.root(), 512_000, &["changes"]);
        let ended = support::run_limited(cmd, 60);
        assert_eq!(ended.code, Some(0), "{}", ended.stderr);
    }
}

#[test]
fn nothing_matching_is_said_plainly() {
    // The first save, of an empty folder, changed no files.
    let env = project(&[]);
    let short = |out: &str| out.split_whitespace().nth(1).unwrap().to_owned();
    let first = env.state().1.lines["main"].to_hex();
    let save = changes(&env, &["--save", &first]);
    assert!(
        save.starts_with("Save ") && save.ends_with(" changed no files.\n"),
        "{save}"
    );
    env.overwrite("a.txt", "a\n");
    env.bolt().args(["save", "A"]).assert().success();
    assert!(changes(&env, &[]).starts_with("Nothing changed since "));
    env.overwrite("a.txt", "b\n");
    let none = changes(&env, &["nothere.txt"]);
    assert!(
        none.starts_with("Nothing changed in those paths since "),
        "{none}"
    );
    let saved = changes(&env, &["--save", &first, "nothere.txt"]);
    assert_eq!(
        saved,
        format!("Save {} changed no files in those paths.\n", short(&save))
    );
}

#[test]
fn a_reader_that_goes_away_ends_it_quietly() {
    let env = project(&[("a.txt", numbered(1..=200_000).as_bytes())]);
    env.overwrite("a.txt", &numbered(1..=200_000).replace("line ", "LINE "));
    let mut child = env
        .bolt_process_in(env.root())
        .arg("changes")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdout = child.stdout.take().unwrap();
    let mut first = [0; 16];
    stdout.read_exact(&mut first).unwrap();
    drop(stdout);
    let out = child.wait_with_output().unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    assert_eq!(text(&out.stderr), "");
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
    // Both ways at once names no way.
    env.overwrite("a.txt", "one\r\ntwo\n");
    env.bolt().args(["save", "Mixed"]).assert().success();
    env.overwrite("a.txt", "one\ntwo\r\n");
    let out = changes(&env, &[]);
    assert!(out.contains("(only line endings changed)\n"), "{out}");
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

#[test]
fn a_pair_of_lines_that_differ_only_in_their_ending_is_marked() {
    let env = project(&[("a.txt", b"one\r\ntwo\nthree\n")]);
    env.overwrite("a.txt", "one\ntwo\nTHREE\n");
    let out = changes(&env, &[]);
    assert!(
        out.contains("\n-one (CRLF)\n+one\n two\n-three\n+THREE\n"),
        "{out}"
    );
    // In JSON both lines of the pair say so, and no other line has the field.
    let out = env.bolt().args(["--json", "changes"]).output().unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    let doc: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let marked: Vec<(&str, &str, Option<bool>)> = doc["changes"][0]["hunks"][0]["lines"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| {
            let s = |k: &str| l[k].as_str().unwrap();
            (
                s("tag"),
                s("text"),
                l.get("eol_only").map(|v| v.as_bool().unwrap()),
            )
        })
        .collect();
    assert_eq!(
        marked,
        [
            ("removed", "one\r\n", Some(true)),
            ("added", "one\n", Some(true)),
            ("same", "two\n", None),
            ("removed", "three\n", None),
            ("added", "THREE\n", None),
        ]
    );
    assert_eq!(doc["changes"][0]["only_line_endings"], false);
}

#[cfg(any(windows, target_os = "linux"))]
#[test]
fn a_stdout_that_fails_mid_document_ends_it_with_the_error_on_stderr() {
    use support::failing_stdout;
    // A first change small enough to sit whole in stdout's buffer, then one that overflows it.
    let env = project(&[("a.txt", b"a\n"), ("b.txt", numbered(1..=2000).as_bytes())]);
    env.overwrite("a.txt", "A\n");
    env.overwrite("b.txt", &numbered(1..=2000).replace("line ", "LINE "));
    for args in [&["--json", "changes"][..], &["changes"]] {
        let out = env
            .bolt_process_in(env.root())
            .args(args)
            .stdout(failing_stdout(&env))
            .stderr(Stdio::piped())
            .output()
            .unwrap();
        let said = text(&out.stderr);
        assert_eq!(out.status.code(), Some(1), "{args:?}: {said}");
        assert!(
            said.starts_with("error: Bolt couldn't write its output: "),
            "{args:?}: {said}"
        );
        assert_eq!(said.lines().count(), 1, "{args:?}: {said}");
    }
}
