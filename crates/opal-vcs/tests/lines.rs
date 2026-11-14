//! Saving away from a line, and `opalro line`.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use opal_core::diary::{Position, ServerLines};
use opal_core::id::Id;
use opal_core::pack::{ObjectKind, PackSet};
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
fn line_new_and_save_line_refuse_a_tags_name() {
    let env = project();
    run(&env, &["tag", "v1"], 0);
    let before = env.standing();
    let (_, said) = run(&env, &["line", "new", "v1"], 1);
    assert!(
        said.contains("\"v1\" is a tag's name already, and a line can't share it."),
        "{said}"
    );
    env.overwrite("a.txt", "b");
    let err = refused_save(&env, &["--line", "v1", "B"]);
    assert!(err.contains("is a tag's name already"), "{err}");
    assert_eq!(lines_now(&env)["lines"], serde_json::json!(["main"]));
    assert_eq!(env.standing().0, before.0);
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

/// A started project on `main`, with `a.txt` saved.
fn project() -> Env {
    let env = Env::empty();
    env.opalro()
        .args(["start", "--preset", "custom"])
        .assert()
        .success();
    env.overwrite("a.txt", "a");
    env.opalro().args(["save", "A"]).assert().success();
    env
}

/// `opalro args`, which must exit with `code`; its stdout and stderr.
fn run(env: &Env, args: &[&str], code: i32) -> (String, String) {
    let out = env.opalro().args(args).output().unwrap();
    let (shown, said) = (text(&out.stdout), text(&out.stderr));
    assert_eq!(out.status.code(), Some(code), "{args:?}: {shown}{said}");
    (shown, said)
}

/// `opalro line --json`.
fn lines_now(env: &Env) -> serde_json::Value {
    let (shown, _) = run(env, &["line", "--json"], 0);
    serde_json::from_str(&shown).unwrap()
}

/// A project on `main` with a line `side` holding one save no other line has.
fn with_a_line_of_its_own() -> Env {
    let env = project();
    run(&env, &["line", "new", "side"], 0);
    env.overwrite("a.txt", "side");
    run(&env, &["save", "X"], 0);
    run(&env, &["goto", "main"], 0);
    env
}

#[test]
fn line_new_starts_a_line_and_moves_onto_it() {
    let env = project();
    let main = env.state().1.lines["main"];
    let (shown, _) = run(&env, &["line", "new", "wall-run"], 0);
    assert!(
        shown.starts_with("Started line wall-run at save "),
        "{shown}"
    );
    assert!(
        shown.ends_with(" \"A\".\n\n    You're on it: your next save goes on wall-run.\n"),
        "{shown}"
    );
    let doc = lines_now(&env);
    assert_eq!(doc["position"], serde_json::json!({"line": "wall-run"}));
    assert_eq!(doc["lines"], serde_json::json!(["main", "wall-run"]));
    let (repo, state) = env.state();
    assert_eq!(state.lines["wall-run"], main);
    let (_, entry) = repo.current().unwrap();
    assert_eq!(entry.command, "line new");
    assert_eq!(entry.folder_change, None);
}

#[test]
fn line_new_keeps_unsaved_changes() {
    let env = project();
    env.overwrite("a.txt", "changed");
    run(&env, &["line", "new", "wall-run"], 0);
    let (status, _) = run(&env, &["status"], 0);
    assert!(status.contains("a.txt"), "{status}");
    let main = env.state().1.lines["main"];
    run(&env, &["save", "On the new line"], 0);
    let state = env.state().1;
    assert_eq!(state.lines["main"], main);
    assert_ne!(state.lines["wall-run"], main);
    assert_eq!(state.position, Position::OnLine("wall-run".into()));
}

#[test]
fn line_rename_renames_and_says_what_stays() {
    let env = project();
    run(&env, &["line", "new", "wall-run"], 0);
    run(&env, &["goto", "main"], 0);
    let (shown, _) = run(&env, &["line", "rename", "wall-run", "wall-running"], 0);
    assert_eq!(
        shown,
        "Renamed line wall-run to wall-running.\n\n    Saves already made keep wall-run as the line \
         they were made on.\n"
    );
    assert_eq!(
        lines_now(&env)["lines"],
        serde_json::json!(["main", "wall-running"])
    );
    assert_eq!(
        Repository::open(env.root())
            .unwrap()
            .current()
            .unwrap()
            .1
            .command,
        "line rename"
    );
    // The name it has already: nothing to do, and no step.
    let before = env.standing();
    let (shown, _) = run(&env, &["line", "rename", "main", "main"], 0);
    assert_eq!(shown, "Line main already has that name.\n");
    assert_eq!(env.standing(), before);
}

#[test]
fn line_rename_with_one_name_renames_the_current_line() {
    let env = project();
    run(&env, &["line", "rename", "trunk"], 0);
    let doc = lines_now(&env);
    assert_eq!(doc["position"], serde_json::json!({"line": "trunk"}));
    assert_eq!(doc["lines"], serde_json::json!(["trunk"]));
}

/// Writes the current state as `change` leaves it, as one step through Opalro's own library.
fn plant_state(env: &Env, change: impl FnOnce(&mut opal_core::diary::State, &mut PackSet)) {
    use opal_core::diary::{Action, DiaryEntry};
    let mut repo = Repository::open(env.root()).unwrap();
    let lock = repo.lock_for_change("test", &mut |_| {}).unwrap();
    let (head, entry) = repo.current().unwrap();
    let mut packs = repo.new_packs().unwrap();
    let mut state = entry.state.clone();
    change(&mut state, &mut packs);
    let diary = DiaryEntry {
        parent: Some(head),
        action: Action::Command,
        command: "test".into(),
        who: entry.who,
        opal_version: "0.1.0".into(),
        state,
        folder_change: None,
    };
    repo.commit(&lock, packs, &diary).unwrap();
}

/// `line delete side`, with and without `--yes`, must stop on damage naming `holder` and change
/// nothing.
fn delete_stops_naming(env: &Env, holder: &str) {
    delete_stops_saying(
        env,
        &format!("{holder} names a save that can't be read: object "),
    );
}

/// `line delete side`, with and without `--yes`, must stop with an error holding `says` and
/// change nothing: neither the diary nor the packs.
fn delete_stops_saying(env: &Env, says: &str) {
    let before = env.standing();
    for args in [
        &["line", "delete", "side"][..],
        &["line", "delete", "side", "--yes"],
    ] {
        let (shown, said) = run(env, args, 1);
        assert_eq!(shown, "", "{args:?}");
        assert!(said.contains(says), "{args:?}: {said}");
        assert!(
            said.contains("hint: Nothing was changed. Ask for help before trying to repair it."),
            "{args:?}: {said}"
        );
        assert_eq!(env.standing(), before, "{args:?}");
    }
    assert!(lines_now(env)["lines"].to_string().contains("side"));
}

#[test]
fn line_delete_names_a_tag_whose_save_cant_be_read() {
    let env = with_a_line_of_its_own();
    let missing = Id::from_hex(&"7".repeat(64)).unwrap();
    env.plant_tags(["ghost".to_owned()].into_iter(), missing);
    delete_stops_naming(&env, "the tag ghost");
    // A tag at an object that isn't a save: the folder listing of main's save.
    let (repo, state) = env.state();
    let listing = repo.store().save(&state.lines["main"], false).unwrap().root;
    drop(repo);
    env.plant_tags(["ghost".to_owned()].into_iter(), listing);
    delete_stops_naming(&env, "the tag ghost");
}

#[test]
fn line_delete_names_a_line_or_server_line_whose_save_cant_be_read() {
    let missing = Id::from_hex(&"7".repeat(64)).unwrap();
    let env = with_a_line_of_its_own();
    plant_state(&env, |state, _| {
        state.lines.insert("broken".into(), missing);
    });
    delete_stops_naming(&env, "the line broken");

    let env = with_a_line_of_its_own();
    plant_state(&env, |state, packs| {
        let servers = ServerLines {
            servers: [("origin".to_owned(), [("main".to_owned(), missing)].into())].into(),
        };
        let id = packs
            .add_record(ObjectKind::ServerLines, &servers.encode())
            .unwrap();
        state.server_lines = Some(id);
    });
    delete_stops_naming(&env, "the server line main@origin");
}

/// A save in the store whose parent isn't, a copy of main's newest otherwise; and that parent.
fn orphan_save(env: &Env) -> (Id, Id) {
    use opal_core::id::ChangeId;
    use opal_core::save::Save;
    let (repo, state) = env.state();
    let tip = repo.store().save(&state.lines["main"], false).unwrap();
    drop(repo);
    let missing = Id::from_hex(&"5".repeat(64)).unwrap();
    let mut orphan = None;
    support::add_pack(env.root(), |pack| {
        let save = Save {
            parents: vec![missing],
            change_id: ChangeId::random().unwrap(),
            message: "orphan".into(),
            ..tip
        };
        orphan = Some(pack.add_record(ObjectKind::Save, &save.encode()).unwrap());
    });
    (orphan.unwrap(), missing)
}

/// The damage message whose detail is `detail`, then the store's words for `missing`.
fn history_damage(detail: &str, missing: Id) -> String {
    format!(
        "error: This project's Opalro data looks damaged: {detail}: object {missing} is not in \
         the store.\n"
    )
}

#[test]
fn line_delete_names_a_tag_whose_older_saves_cant_be_read() {
    let env = with_a_line_of_its_own();
    let (orphan, missing) = orphan_save(&env);
    env.plant_tags(["ghost".to_owned()].into_iter(), orphan);
    let says = history_damage(
        "the tag ghost names a save whose history can't be read",
        missing,
    );
    delete_stops_saying(&env, &says);
    // The same two saves further back: a save on top of the orphan.
    let env = with_a_line_of_its_own();
    let (orphan, _) = orphan_save(&env);
    let mut later = None;
    let (repo, _) = env.state();
    let above = repo.store().save(&orphan, false).unwrap();
    drop(repo);
    support::add_pack(env.root(), |pack| {
        let save = opal_core::save::Save {
            parents: vec![orphan],
            message: "later".into(),
            ..above
        };
        later = Some(pack.add_record(ObjectKind::Save, &save.encode()).unwrap());
    });
    env.plant_tags(["ghost".to_owned()].into_iter(), later.unwrap());
    delete_stops_saying(&env, &says);
}

#[test]
fn line_delete_names_a_line_or_server_line_whose_older_saves_cant_be_read() {
    let env = with_a_line_of_its_own();
    let (orphan, missing) = orphan_save(&env);
    plant_state(&env, |state, _| {
        state.lines.insert("broken".into(), orphan);
    });
    delete_stops_saying(
        &env,
        &history_damage(
            "the line broken names a save whose history can't be read",
            missing,
        ),
    );

    let env = with_a_line_of_its_own();
    let (orphan, missing) = orphan_save(&env);
    plant_state(&env, |state, packs| {
        let servers = ServerLines {
            servers: [("origin".to_owned(), [("main".to_owned(), orphan)].into())].into(),
        };
        let id = packs
            .add_record(ObjectKind::ServerLines, &servers.encode())
            .unwrap();
        state.server_lines = Some(id);
    });
    delete_stops_saying(
        &env,
        &history_damage(
            "the server line main@origin names a save whose history can't be read",
            missing,
        ),
    );
}

#[test]
fn line_delete_names_the_deleted_lines_own_history_when_no_holders_holds_it() {
    let env = with_a_line_of_its_own();
    let (orphan, missing) = orphan_save(&env);
    plant_state(&env, |state, _| {
        state.lines.insert("side".into(), orphan);
    });
    delete_stops_saying(
        &env,
        &history_damage("line side's history can't be read", missing),
    );
}

#[test]
fn a_line_another_line_holds_is_deleted_though_a_tag_cant_be_read() {
    // The walk answers from main before it reads the tag's save.
    let env = project();
    run(&env, &["line", "new", "side"], 0);
    run(&env, &["goto", "main"], 0);
    let missing = Id::from_hex(&"7".repeat(64)).unwrap();
    env.plant_tags(["ghost".to_owned()].into_iter(), missing);
    run(&env, &["line", "delete", "side"], 0);
    assert_eq!(lines_now(&env)["lines"], serde_json::json!(["main"]));
}

#[test]
fn line_delete_deletes_a_line_another_holds_without_asking() {
    let env = project();
    run(&env, &["line", "new", "side"], 0);
    run(&env, &["goto", "main"], 0);
    let (shown, said) = run(&env, &["line", "delete", "side"], 0);
    assert!(
        shown.starts_with("Deleted line side, which was at save "),
        "{shown}"
    );
    assert!(shown.ends_with(" \"A\".\n"), "{shown}");
    assert!(!said.contains("error"), "{said}");
    assert_eq!(lines_now(&env)["lines"], serde_json::json!(["main"]));
    let (_, entry) = Repository::open(env.root()).unwrap().current().unwrap();
    assert_eq!(entry.command, "line delete");
    assert_eq!(entry.folder_change, None);
}

#[test]
fn line_delete_without_a_terminal_stops_and_names_yes() {
    let env = with_a_line_of_its_own();
    let before = env.standing();
    let (shown, said) = run(&env, &["line", "delete", "side"], 1);
    assert_eq!(shown, "");
    assert!(
        said.contains(
            "error: Line side holds 1 save that no other line or tag has, and Opalro can't ask \
             whether to delete it without a terminal."
        ),
        "{said}"
    );
    assert!(
        said.contains("hint: To delete it: opalro line delete side --yes"),
        "{said}"
    );
    assert_eq!(env.standing(), before);
    assert_eq!(
        lines_now(&env)["lines"],
        serde_json::json!(["main", "side"])
    );
    let (shown, _) = run(&env, &["line", "delete", "side", "--json"], 1);
    let doc: serde_json::Value = serde_json::from_str(&shown).unwrap();
    assert_eq!(
        doc["error"]["message"],
        "Line side holds 1 save that no other line or tag has, and Opalro can't ask whether to \
         delete it with --json."
    );
    assert_eq!(
        doc["error"]["hint"],
        "To delete it: opalro line delete side --yes"
    );
    assert_eq!(env.standing(), before);
}

#[cfg(feature = "fault-injection")]
#[test]
fn line_delete_asks_in_a_terminal() {
    let env = with_a_line_of_its_own();
    let asked_with = |extra: &[&str], input: &str| {
        let out = env
            .opalro()
            .args(["line", "delete", "side"])
            .args(extra)
            .env("OPALRO_FAKE_TERMINAL", "80")
            .env("OPALRO_ASCII", "1")
            .env("NO_COLOR", "1")
            .write_stdin(input)
            .output()
            .unwrap();
        (out.status.code(), text(&out.stdout), text(&out.stderr))
    };
    let asked = |input: &str| asked_with(&[], input);
    let before = env.standing();
    // With --json, even at a terminal, nothing is asked.
    let (code, shown, said) = asked_with(&["--json"], "y\n");
    assert_eq!(code, Some(1), "{said}");
    assert!(!said.contains("Delete it?"), "{said}");
    let doc: serde_json::Value = serde_json::from_str(&shown).unwrap();
    assert!(
        doc["error"]["message"]
            .as_str()
            .unwrap()
            .ends_with("with --json."),
        "{doc}"
    );
    assert_eq!(env.standing(), before);
    let (code, shown, said) = asked("n\n");
    assert_eq!(code, Some(0), "{said}");
    assert_eq!(shown, "Nothing was deleted.\n");
    assert!(
        said.starts_with(
            "Line side holds 1 save that no other line or tag has.\nDelete it? [y/N] "
        ),
        "{said}"
    );
    assert_eq!(env.standing(), before);
    let (code, shown, said) = asked("y\n");
    assert_eq!(code, Some(0), "{said}");
    assert!(shown.starts_with("Deleted line side, "), "{shown}");
    assert_eq!(lines_now(&env)["lines"], serde_json::json!(["main"]));
}

#[test]
fn line_delete_yes_deletes_and_undo_brings_it_back() {
    let env = with_a_line_of_its_own();
    let x = env.state().1.lines["side"];
    let (shown, _) = run(&env, &["line", "delete", "side", "--yes"], 0);
    assert!(
        shown.starts_with("Deleted line side, which was at save "),
        "{shown}"
    );
    assert!(
        shown.ends_with(
            " \"X\".\n\n    No other line or tag has its 1 save. opalro tidy frees them once the \
             undo window has passed.\n"
        ),
        "{shown}"
    );
    assert!(!env.state().1.lines.contains_key("side"));
    run(&env, &["undo"], 0);
    assert_eq!(env.state().1.lines["side"], x);
}

/// Makes every pack of the project two days old, so that `tidy` may write them again.
fn age_packs(env: &Env) {
    let packs = env.root().join(".opalro/store/packs");
    let then = std::time::SystemTime::now() - std::time::Duration::from_secs(48 * 3600);
    for entry in std::fs::read_dir(packs).unwrap() {
        let file = std::fs::File::options()
            .write(true)
            .open(entry.unwrap().path())
            .unwrap();
        file.set_modified(then).unwrap();
    }
}

#[test]
fn a_deleted_lines_saves_survive_tidy_inside_the_undo_window() {
    let env = with_a_line_of_its_own();
    run(&env, &["line", "delete", "side", "--yes"], 0);
    age_packs(&env);
    let (shown, _) = run(&env, &["tidy", "--json"], 0);
    let doc: serde_json::Value = serde_json::from_str(&shown).unwrap();
    assert_eq!(doc["packs_young"], 0, "{doc}");
    // `tidy` records no step, so one undo takes back the delete.
    run(&env, &["undo"], 0);
    assert!(env.state().1.lines.contains_key("side"));
    run(&env, &["goto", "side"], 0);
    assert_eq!(
        std::fs::read_to_string(env.root().join("a.txt")).unwrap(),
        "side"
    );
    run(&env, &["verify"], 0);
}

#[test]
fn after_a_rename_history_goto_and_short_ids_still_work() {
    let env = project();
    env.overwrite("a.txt", "b");
    run(&env, &["save", "B"], 0);
    env.overwrite("a.txt", "wip");
    run(&env, &["shelf", "Half done"], 0);
    let (history, _) = run(&env, &["history", "--json"], 0);
    let history: serde_json::Value = serde_json::from_str(&history).unwrap();
    let older = history["saves"][1]["change_id_short"]
        .as_str()
        .unwrap()
        .to_owned();
    run(&env, &["line", "rename", "main", "trunk-line"], 0);
    let (shown, _) = run(&env, &["history"], 0);
    assert!(
        shown.starts_with("Saves on line trunk-line, newest first:"),
        "{shown}"
    );
    assert!(shown.contains(&older), "{shown}");
    let (after, _) = run(&env, &["history", "--json"], 0);
    let after: serde_json::Value = serde_json::from_str(&after).unwrap();
    assert_eq!(after["saves"], history["saves"]);
    let (shown, _) = run(&env, &["goto", "trunk-line"], 0);
    assert!(shown.starts_with("Already on line trunk-line"), "{shown}");
    let (shown, _) = run(&env, &["show", &older, "--json"], 0);
    let shown: serde_json::Value = serde_json::from_str(&shown).unwrap();
    assert_eq!(shown["save"]["message"], "A");
    let (shown, _) = run(&env, &["shelf", "--list"], 0);
    assert!(shown.contains("on main"), "{shown}");
    run(&env, &["undo"], 0);
    let doc = lines_now(&env);
    assert_eq!(doc["lines"], serde_json::json!(["main"]));
    assert_eq!(doc["position"], serde_json::json!({"line": "main"}));
}

/// `opalro args --json`, which must succeed; its document, masked.
fn json_of(env: &Env, args: &[&str]) -> serde_json::Value {
    let mut args = args.to_vec();
    args.push("--json");
    let (shown, _) = run(env, &args, 0);
    support::masked(serde_json::from_str(&shown).unwrap())
}

#[test]
fn line_new_json_matches_its_snapshot() {
    let env = project();
    insta::assert_json_snapshot!(json_of(&env, &["line", "new", "wall-run"]));
}

#[test]
fn line_rename_json_matches_its_snapshot() {
    let env = project();
    insta::assert_json_snapshot!(json_of(&env, &["line", "rename", "trunk"]));
}

#[test]
fn line_delete_json_matches_its_snapshot() {
    let env = with_a_line_of_its_own();
    insta::assert_json_snapshot!(json_of(&env, &["line", "delete", "side", "--yes"]));
}

#[test]
fn help_line_lists_its_forms() {
    let (shown, _) = run(&Env::empty(), &["help", "line"], 0);
    let section = |title: &str| -> Vec<String> {
        (shown.lines())
            .skip_while(|l| *l != title)
            .skip(1)
            .take_while(|l| !l.is_empty())
            .map(|l| l.split_whitespace().collect::<Vec<_>>().join(" "))
            .collect()
    };
    assert_eq!(
        section("Forms"),
        [
            "opalro line new <NAME> Make a new line at the save you're on, and move onto it",
            "opalro line rename <NAME>... Rename a line: the one you're on, or the one named first",
            "opalro line delete [--yes] <NAME> Delete a line. Its saves stay stored for the undo \
             window",
            "--yes Delete without asking, even when no other line or tag has its saves",
        ],
        "{shown}"
    );
    let options = section("Options");
    assert!(!options.is_empty(), "{shown}");
    assert!(options.iter().all(|l| !l.contains("--yes")), "{shown}");
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
