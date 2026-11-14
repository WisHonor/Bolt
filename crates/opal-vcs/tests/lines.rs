//! Saving away from a line, and `opalro line`.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use opal_core::diary::Position;
use opal_core::id::Id;
use opal_core::repo::Repository;

mod support;
use support::{Env, text};

/// A started project with two saves on `main`, at its first save: returns that save.
fn at_first_save() -> (Env, Id) {
    let env = Env::empty();
    env.opalro()
        .args(["start", "--preset", "custom"])
        .assert()
        .success();
    env.overwrite("a.txt", "a");
    env.opalro().args(["save", "A"]).assert().success();
    let first = env.state().1.lines["main"];
    env.overwrite("a.txt", "b");
    env.opalro().args(["save", "B"]).assert().success();
    env.opalro()
        .args(["goto", &first.to_hex()])
        .assert()
        .success();
    assert_eq!(env.state().1.position, Position::AtSave(first));
    (env, first)
}

/// Runs `opalro save args`, which must fail without changing anything; returns its error output.
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
    assert!(err.contains("opalro save --line <name>"), "{err}");
    assert_eq!(env.state().1.position, Position::AtSave(first));
}

#[test]
fn at_a_save_a_large_save_asks_for_the_line_name_first() {
    let (env, _) = at_first_save();
    let huge = std::fs::File::create(env.root().join("huge.bin")).unwrap();
    huge.set_len(100_000_001).unwrap();
    let err = refused_save(&env, &["C"]);
    assert!(err.contains("opalro save --line <name>"), "{err}");
    let err = refused_save(&env, &["--line", "art", "C"]);
    assert!(err.contains("--yes"), "{err}");
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
    env.opalro().args(["save", "D"]).assert().success();
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

/// `opalro save --json args`, which must fail without changing anything; returns its document.
fn refused_save_json(env: &Env, args: &[&str]) -> serde_json::Value {
    let before = env.standing();
    let out = env
        .opalro()
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
    assert!(hint.contains("opalro save --line <name>"), "{doc}");
}

#[test]
fn save_json_with_line_gives_the_line() {
    let (env, _) = at_first_save();
    env.overwrite("a.txt", "c");
    let out = env
        .opalro()
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
fn save_line_with_nothing_to_save_makes_no_line_and_names_line_new() {
    let (env, first) = at_first_save();
    let before = env.standing();
    let out = env.save(&["--line", "feature", "C"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    let said = text(&out.stdout);
    assert!(said.contains("Nothing to save"), "{said}");
    assert!(
        said.contains("No line was made: opalro line new <name> makes one without saving."),
        "{said}"
    );
    assert_eq!(env.standing(), before);
    let state = env.state().1;
    assert_eq!(state.position, Position::AtSave(first));
    assert!(!state.lines.contains_key("feature"));
}

#[test]
fn save_line_on_a_line_starts_a_new_line_and_leaves_the_old_one_where_it_was() {
    let env = Env::empty();
    env.opalro()
        .args(["start", "--preset", "custom"])
        .assert()
        .success();
    let main = env.state().1.lines["main"];
    let before = Repository::open(env.root()).unwrap().head().unwrap();
    env.overwrite("a.txt", "a");
    let out = env.save(&["--line", "wall-run", "Try wall running"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    assert!(text(&out.stdout).contains("You're on the new line wall-run."));
    let out = env.opalro().args(["line", "--json"]).output().unwrap();
    let doc: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(doc["lines"], serde_json::json!(["main", "wall-run"]));
    assert_eq!(doc["position"], serde_json::json!({"line": "wall-run"}));
    let (repo, state) = env.state();
    assert_eq!(state.lines["main"], main);
    let save = repo.store().save(&state.lines["wall-run"], false).unwrap();
    assert_eq!(save.parents, vec![main]);
    // One diary entry, the save's, makes the line too.
    let (_, entry) = repo.current().unwrap();
    assert_eq!(entry.parent, Some(before));
    assert_eq!(entry.command, "save");
}

#[test]
fn save_line_on_a_line_with_nothing_to_save_makes_no_line() {
    let env = Env::empty();
    env.opalro()
        .args(["start", "--preset", "custom"])
        .assert()
        .success();
    let before = env.standing();
    let out = env.save(&["--line", "x", "msg"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    let said = text(&out.stdout);
    assert!(said.contains("Nothing to save"), "{said}");
    assert!(said.contains("No line was made"), "{said}");
    assert_eq!(env.standing(), before);
    let out = env.opalro().args(["line", "--json"]).output().unwrap();
    let doc: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(doc["lines"], serde_json::json!(["main"]));
    assert_eq!(doc["position"], serde_json::json!({"line": "main"}));
}

#[test]
fn save_line_on_a_line_refuses_a_tags_name() {
    let env = Env::new();
    env.commit("a.txt", "a\n", "First");
    env.git(&["tag", "v1"]);
    env.import();
    env.overwrite("a.txt", "b");
    let err = refused_save(&env, &["--line", "v1", "B"]);
    assert!(err.contains("is a tag's name already"), "{err}");
    let err = refused_save(&env, &["--line", "MAIN", "B"]);
    assert!(err.contains("There is already a line main"), "{err}");
}

#[test]
fn line_lists_lines_and_server_lines() {
    let env = Env::new();
    env.commit("a.txt", "a\n", "First");
    env.git(&["branch", "zz"]);
    env.git(&["branch", "b"]);
    env.git(&["update-ref", "refs/remotes/origin/x", "HEAD"]);
    env.import();
    let out = env.opalro().arg("line").output().unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    let name = env.root().file_name().unwrap().to_string_lossy();
    assert_eq!(
        text(&out.stdout),
        format!("Lines in {name}:\n\n  b\n> main\n  zz\n\nOn the server (origin):\n  x@origin\n")
    );
    let out = env.opalro().args(["line", "--json"]).output().unwrap();
    let doc: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(doc["position"], serde_json::json!({"line": "main"}));
    assert_eq!(doc["lines"], serde_json::json!(["b", "main", "zz"]));
    assert_eq!(doc["server_lines"], serde_json::json!(["x@origin"]));
    // At a save, no line is current.
    let first = env.state().1.lines["main"];
    env.opalro()
        .args(["goto", &first.to_hex()])
        .assert()
        .success();
    let out = env.opalro().arg("line").output().unwrap();
    assert_eq!(
        text(&out.stdout),
        format!(
            "Lines in {name}. You are at save {}, not on a line.\n\n  b\n  main\n  zz\n\nOn the \
             server (origin):\n  x@origin\n",
            &first.to_hex()[..12]
        )
    );
    let out = env.opalro().args(["line", "--json"]).output().unwrap();
    let doc: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(doc["position"], serde_json::json!({"save": first.to_hex()}));
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
            .opalro()
            .args(["save", "--line", "feature", "C"])
            .env("OPALRO_FAULT_AT", n.to_string())
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
    assert!(err.contains("opalro save --line <name>"), "{err}");
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

#[test]
fn line_json_matches_its_snapshots() {
    let (env, first) = at_first_save();
    let line = |env: &Env| -> serde_json::Value {
        let out = env.opalro().args(["line", "--json"]).output().unwrap();
        assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
        serde_json::from_slice(&out.stdout).unwrap()
    };
    let mut doc = line(&env);
    assert_eq!(doc["position"]["save"], first.to_hex(), "{doc}");
    doc["position"]["save"] = "[redacted]".into();
    insta::assert_json_snapshot!("line_at_a_save", doc);
    env.opalro().args(["goto", "main"]).assert().success();
    insta::assert_json_snapshot!("line_on_a_line", line(&env));
}

#[test]
fn history_json_at_a_save_gives_that_save_as_position() {
    let (env, first) = at_first_save();
    let out = env.opalro().args(["history", "--json"]).output().unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    let doc: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(
        doc["position"],
        serde_json::json!({"save": first.to_hex()}),
        "{doc}"
    );
}
