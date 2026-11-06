//! Saving away from a line, and `bolt line`.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use bolt_core::diary::Position;
use bolt_core::id::Id;
use bolt_core::repo::Repository;

mod support;
use support::{Env, text};

/// A started project with two saves on `main`, at its first save: returns that save.
fn at_first_save() -> (Env, Id) {
    let env = Env::empty();
    env.bolt().arg("start").assert().success();
    env.overwrite("a.txt", "a");
    env.bolt().args(["save", "A"]).assert().success();
    let first = env.state().1.lines["main"];
    env.overwrite("a.txt", "b");
    env.bolt().args(["save", "B"]).assert().success();
    env.bolt()
        .args(["goto", &first.to_hex()])
        .assert()
        .success();
    assert_eq!(env.state().1.position, Position::AtSave(first));
    (env, first)
}

/// Runs `bolt save args`, which must fail without changing anything; returns its error output.
fn refused_save(env: &Env, args: &[&str]) -> String {
    let before = env.standing();
    let out = env.save(args);
    assert_eq!(out.status.code(), Some(1), "{}", text(&out.stdout));
    assert_eq!(env.standing(), before);
    text(&out.stderr)
}

#[test]
fn saving_at_a_save_without_a_terminal_names_the_flag() {
    let (env, first) = at_first_save();
    env.overwrite("a.txt", "c");
    let err = refused_save(&env, &["C"]);
    assert!(err.contains("bolt save --line <name>"), "{err}");
    assert_eq!(env.state().1.position, Position::AtSave(first));
}

#[test]
fn save_with_line_makes_the_line_and_moves_onto_it() {
    let (env, first) = at_first_save();
    let main = env.state().1.lines["main"];
    let before = Repository::open(env.root()).unwrap().head().unwrap();
    env.overwrite("a.txt", "c");
    let out = env.save(&["--line", "feature/x", "C"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    assert!(text(&out.stdout).contains("on the new line feature/x"));
    let (repo, state) = env.state();
    assert_eq!(state.position, Position::OnLine("feature/x".into()));
    assert_eq!(state.lines["main"], main);
    let save = repo.store().save(&state.lines["feature/x"], false).unwrap();
    assert_eq!(save.parents, vec![first]);
    assert_eq!(save.line(), Some("feature/x"));
    // The line and the save are one diary entry, so a crash can't keep one without the other.
    let (head, entry) = repo.current().unwrap();
    assert_eq!(entry.parent, Some(before));
    assert_ne!(head, before);
    drop(repo);
    // Saving again stays on it.
    env.overwrite("a.txt", "d");
    env.bolt().args(["save", "D"]).assert().success();
    assert_eq!(env.state().1.position, Position::OnLine("feature/x".into()));
}

#[test]
fn a_line_name_taken_in_another_case_is_refused() {
    let (env, _) = at_first_save();
    env.overwrite("a.txt", "c");
    let err = refused_save(&env, &["--line", "MAIN", "C"]);
    assert!(err.contains("There is already a line main"), "{err}");
    let err = refused_save(&env, &["--line", "has space", "C"]);
    assert!(
        err.contains(
            "`has space` can't be a line's name: a line name can only hold letters, digits, -, \
             _, . and /"
        ),
        "{err}"
    );
}

/// `bolt save --json args`, which must fail without changing anything; returns its document.
fn refused_save_json(env: &Env, args: &[&str]) -> serde_json::Value {
    let before = env.standing();
    let out = env
        .bolt()
        .args(["save", "--json"])
        .args(args)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1), "{}", text(&out.stderr));
    assert_eq!(env.standing(), before);
    serde_json::from_slice(&out.stdout).unwrap()
}

#[test]
fn bad_line_names_give_an_error_document() {
    let (env, _) = at_first_save();
    env.overwrite("a.txt", "c");
    let doc = refused_save_json(&env, &["--line", "Main", "C"]);
    let message = doc["error"]["message"].as_str().unwrap();
    assert!(message.starts_with("There is already a line main"), "{doc}");
    assert_eq!(
        doc["error"]["hint"],
        "Nothing was saved. Choose another name."
    );
    let doc = refused_save_json(&env, &["--line", "a//b", "C"]);
    let message = doc["error"]["message"].as_str().unwrap();
    assert!(
        message.starts_with("`a//b` can't be a line's name"),
        "{doc}"
    );
    // Without a name, `--json` never asks.
    let doc = refused_save_json(&env, &["C"]);
    let hint = doc["error"]["hint"].as_str().unwrap();
    assert!(hint.contains("bolt save --line <name>"), "{doc}");
}

#[test]
fn save_json_with_line_gives_the_line() {
    let (env, _) = at_first_save();
    env.overwrite("a.txt", "c");
    let out = env
        .bolt()
        .args(["save", "--json", "--line", "feature", "C"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    let doc: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(doc["result"], "saved");
    assert_eq!(doc["save"]["line"], "feature");
}

#[test]
fn nothing_changed_at_a_save_needs_no_line_name() {
    let (env, first) = at_first_save();
    let before = env.standing();
    let out = env.save(&["C"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    assert!(
        text(&out.stdout).contains("Nothing to save"),
        "{}",
        text(&out.stdout)
    );
    assert!(!text(&out.stdout).contains("No line was made"));
    assert_eq!(env.standing(), before);
    assert_eq!(env.state().1.position, Position::AtSave(first));
}

#[test]
fn line_with_nothing_changed_says_no_line_was_made() {
    let (env, first) = at_first_save();
    let before = env.standing();
    let out = env.save(&["--line", "feature", "C"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    let said = text(&out.stdout);
    assert!(said.contains("Nothing to save"), "{said}");
    assert!(
        said.contains("No line was made: a new line starts with a save, so change a file first."),
        "{said}"
    );
    assert_eq!(env.standing(), before);
    let state = env.state().1;
    assert_eq!(state.position, Position::AtSave(first));
    assert!(!state.lines.contains_key("feature"));
}

#[test]
fn line_is_refused_on_a_line() {
    let env = Env::empty();
    env.bolt().arg("start").assert().success();
    env.overwrite("a.txt", "a");
    let err = refused_save(&env, &["--line", "feature", "A"]);
    assert!(err.contains("comes in v0.2"), "{err}");
}

#[test]
fn line_lists_lines_and_server_lines() {
    let env = Env::new();
    env.commit("a.txt", "a\n", "First");
    env.git(&["branch", "zz"]);
    env.git(&["branch", "b"]);
    env.git(&["update-ref", "refs/remotes/origin/x", "HEAD"]);
    env.import();
    let out = env.bolt().arg("line").output().unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    assert_eq!(text(&out.stdout), "  b\n* main\n  zz\n  x@origin\n");
    let out = env.bolt().args(["line", "--json"]).output().unwrap();
    let doc: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(doc["current"], "main");
    assert_eq!(doc["lines"], serde_json::json!(["b", "main", "zz"]));
    assert_eq!(doc["server_lines"], serde_json::json!(["x@origin"]));
    // At a save, no line is current.
    let first = env.state().1.lines["main"];
    env.bolt()
        .args(["goto", &first.to_hex()])
        .assert()
        .success();
    let out = env.bolt().arg("line").output().unwrap();
    assert_eq!(text(&out.stdout), "  b\n  main\n  zz\n  x@origin\n");
    let out = env.bolt().args(["line", "--json"]).output().unwrap();
    let doc: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(doc["current"], serde_json::Value::Null);
}

/// Wherever a save away from a line stops, the project is either where it was or on the new
/// line at the new save: never with the save and without the line.
#[cfg(feature = "fault-injection")]
#[test]
fn a_crash_never_keeps_the_save_without_its_line() {
    for n in 1.. {
        assert!(n < 100, "the save never finished");
        let (env, first) = at_first_save();
        env.overwrite("a.txt", "c");
        let out = env
            .bolt()
            .args(["save", "--line", "feature", "C"])
            .env("BOLT_FAULT_AT", n.to_string())
            .output()
            .unwrap();
        let crashed = text(&out.stderr).contains("injected crash");
        let state = env.state().1;
        match &state.position {
            Position::AtSave(at) => {
                assert!(crashed, "fault {n}: {}", text(&out.stderr));
                assert_eq!(*at, first, "fault {n}");
                assert!(!state.lines.contains_key("feature"), "fault {n}");
            }
            Position::OnLine(line) => {
                assert_eq!(line, "feature", "fault {n}");
                let repo = Repository::open(env.root()).unwrap();
                let save = repo.store().save(&state.lines["feature"], false).unwrap();
                assert_eq!(save.parents, vec![first], "fault {n}");
            }
        }
        if !crashed {
            assert!(n > 1, "no fault point was reached");
            assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
            break;
        }
    }
}

#[test]
fn a_partial_save_at_a_save_needs_a_line_name_too() {
    let (env, first) = at_first_save();
    env.overwrite("a.txt", "c");
    env.overwrite("b.txt", "new");
    let err = refused_save(&env, &["C", "a.txt"]);
    assert!(err.contains("bolt save --line <name>"), "{err}");
    let out = env.save(&["--line", "part", "C", "a.txt"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    let (repo, state) = env.state();
    let save = repo.store().save(&state.lines["part"], false).unwrap();
    assert_eq!(save.parents, vec![first]);
    let names: Vec<String> = repo
        .store()
        .listing(&save.root)
        .unwrap()
        .entries()
        .iter()
        .map(|e| e.name.clone())
        .collect();
    assert_eq!(names, ["a.txt"]);
}
