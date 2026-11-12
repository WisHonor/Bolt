//! `opalro goto`, end to end, and the helpers its tests share.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use opal_core::journal::Journal;
use opal_core::repo::Repository;

mod support;
use support::{Env, text};

/// Holds the file at `path` open, shared with nobody, as an editor or virus scanner may hold
/// one, until the handle is dropped. Another process's commands then can't replace or remove it.
#[cfg(windows)]
pub fn hold_open(path: &std::path::Path) -> std::fs::File {
    use std::os::windows::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(path)
        .unwrap()
}

#[cfg(windows)]
#[test]
fn a_held_file_cant_be_replaced_or_removed_until_let_go() {
    let dir = tempfile::tempdir().unwrap();
    let (held, other) = (dir.path().join("held"), dir.path().join("other"));
    std::fs::write(&held, b"old").unwrap();
    std::fs::write(&other, b"new").unwrap();
    let handle = hold_open(&held);
    const SHARING_VIOLATION: i32 = 32;
    // Windows' own rename says "access denied" here; Opalro's, by handle, a sharing violation.
    assert!(std::fs::rename(&other, &held).is_err());
    let removed = std::fs::remove_file(&held).unwrap_err();
    assert_eq!(removed.raw_os_error(), Some(SHARING_VIOLATION));
    drop(handle);
    std::fs::rename(&other, &held).unwrap();
    assert_eq!(std::fs::read(&held).unwrap(), b"new");
}

/// The kinds of the warnings in `opalro <args> --json`'s document.
fn warning_kinds(env: &Env, args: &[&str]) -> (Vec<String>, serde_json::Value) {
    let out = env.opalro().args(args).arg("--json").output().unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    let doc: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let kinds = doc["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| w["kind"].as_str().unwrap().to_owned())
        .collect();
    (kinds, doc)
}

/// A project with a save holding `a.txt`, put back on its first save as a switch to it that
/// stopped before `head` moved would leave it.
fn stopped_switch() -> Env {
    let env = Env::empty();
    env.opalro()
        .args(["start", "--preset", "custom"])
        .assert()
        .success();
    let first = Repository::open(env.root()).unwrap().head().unwrap();
    env.overwrite("a.txt", "a");
    env.opalro().args(["save", "A"]).assert().success();
    let mut repo = Repository::open(env.root()).unwrap();
    let second = repo.head().unwrap();
    let lock = repo.lock_for_change("goto", &mut |_| {}).unwrap();
    repo.move_head_to(&lock, first).unwrap();
    let journal = Journal {
        target: second,
        from: first,
        actions: vec![],
    };
    journal.write(&repo, &lock).unwrap();
    env
}

#[test]
fn read_only_commands_say_a_switch_is_unfinished() {
    let env = stopped_switch();
    for command in ["status", "history", "show", "line"] {
        let (kinds, doc) = warning_kinds(&env, &[command]);
        assert!(
            kinds.contains(&"switch_unfinished".to_owned()),
            "{command}: {doc}"
        );
        let out = env.opalro().arg(command).output().unwrap();
        let said = text(&out.stderr);
        assert!(
            said.contains("warning: A switch between saves was stopped"),
            "{said}"
        );
    }
    let (_, doc) = warning_kinds(&env, &["status"]);
    assert_eq!(doc["unfinished_switch"], true, "{doc}");

    // The next command that changes the project finishes it, and says so.
    let (kinds, doc) = warning_kinds(&env, &["save", "B"]);
    assert_eq!(doc["result"], "nothing_changed", "{doc}");
    assert_eq!(kinds, ["switch_finished"]);
    let (kinds, doc) = warning_kinds(&env, &["status"]);
    assert!(kinds.is_empty(), "{doc}");
    assert_eq!(doc["unfinished_switch"], false, "{doc}");
    assert!(!env.root().join(".opalro/state/journal").exists());
}

#[test]
fn a_damaged_journal_stops_a_save_and_is_reported() {
    let env = stopped_switch();
    let journal = env.root().join(".opalro/state/journal");
    std::fs::write(&journal, b"OPALJRNL garbage").unwrap();
    let out = env.save(&["B"]);
    assert_eq!(out.status.code(), Some(1));
    let said = text(&out.stderr);
    assert!(said.contains(".opalro/state/journal"), "{said}");
    assert!(said.contains("opalro status"), "{said}");
    let (kinds, doc) = warning_kinds(&env, &["status"]);
    assert!(kinds.contains(&"switch_unfinished".to_owned()), "{doc}");
    assert!(journal.exists());
}

#[cfg(feature = "fault-injection")]
#[test]
fn a_save_that_fails_after_finishing_a_switch_still_says_it_finished() {
    for json in [false, true] {
        let env = stopped_switch();
        let mut cmd = env.opalro();
        cmd.args(["save", "B"])
            .env("OPALRO_FAULT_FAIL", "save:walked");
        if json {
            cmd.arg("--json");
        }
        let out = cmd.output().unwrap();
        assert_eq!(out.status.code(), Some(1));
        if json {
            let doc: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
            assert_eq!(doc["warnings"][0]["kind"], "switch_finished", "{doc}");
        } else {
            let said = text(&out.stderr);
            assert!(said.contains("warning: Finished a switch"), "{said}");
            assert!(said.contains("error:"), "{said}");
        }
        assert!(!env.root().join(".opalro/state/journal").exists());
    }
}

// `opalro goto` itself.

use std::path::Path;

use opal_core::diary::{Action, DiaryEntry, FolderChange, Position};
use opal_core::id::ChangeId;
use opal_core::pack::ObjectKind;
use opal_core::save::Save;
use support::{changed, files, holding};

const MAIN: &[(&str, &str)] = &[
    ("a.txt", "a\n"),
    ("b.txt", "b\n"),
    ("dir/c.bin", "\0\u{1}\r\n\u{ff}binary"),
];
const FEATURE: &[(&str, &str)] = &[
    ("a.txt", "a\n"),
    ("b.txt", "b on feature\r\n"),
    ("new/deep/d.txt", "d\n"),
];

fn write_all(env: &Env, entries: &[(&str, &str)]) {
    for (path, bytes) in entries {
        let at = env.root().join(path);
        std::fs::create_dir_all(at.parent().unwrap()).unwrap();
        std::fs::write(at, bytes).unwrap();
    }
}

/// An imported Git repository with two lines: `main`, holding `MAIN`, which the project is on,
/// and `feature`, holding `FEATURE`. The line `zz` and the server line `x@origin` are at
/// `feature`'s save.
fn two_lines() -> Env {
    let env = Env::new();
    write_all(&env, MAIN);
    env.git(&["add", "-A"]);
    env.git(&["commit", "-q", "-m", "Main"]);
    env.git(&["checkout", "-q", "-b", "feature"]);
    std::fs::remove_dir_all(env.root().join("dir")).unwrap();
    write_all(&env, FEATURE);
    env.git(&["add", "-A"]);
    env.git(&["commit", "-q", "-m", "Feature"]);
    env.git(&["update-ref", "refs/remotes/origin/x", "HEAD"]);
    env.git(&["branch", "zz", "HEAD"]);
    env.git(&["checkout", "-q", "main"]);
    env.import();
    assert_eq!(files(env.root()), holding(MAIN));
    env
}

fn goto(env: &Env, target: &str) -> String {
    let out = env.opalro().args(["goto", target]).output().unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    text(&out.stdout)
}

fn goto_json(env: &Env, target: &str) -> serde_json::Value {
    let out = env
        .opalro()
        .args(["goto", target, "--json"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    serde_json::from_slice(&out.stdout).unwrap()
}

#[test]
fn goto_a_line_and_back_restores_every_file_byte_for_byte() {
    let env = two_lines();
    let said = goto(&env, "feature");
    assert!(
        said.starts_with("Switched to line feature at save "),
        "{said}"
    );
    assert_eq!(files(env.root()), holding(FEATURE));
    assert!(!env.root().join("dir").exists());
    assert_eq!(env.state().1.position, Position::OnLine("feature".into()));
    assert!(changed(&env).is_empty());

    let said = goto(&env, "main");
    assert!(said.starts_with("Switched to line main at save "), "{said}");
    assert_eq!(files(env.root()), holding(MAIN));
    assert!(!env.root().join("new").exists());
    assert!(changed(&env).is_empty());
    assert!(!env.root().join(".opalro/state/journal").exists());
}

#[test]
fn goto_carries_unsaved_changes_that_dont_collide() {
    let env = two_lines();
    env.overwrite("a.txt", "mine\n");
    env.overwrite("n.txt", "new\n");
    let said = goto(&env, "feature");
    assert!(
        said.starts_with("Switched to line feature at save "),
        "{said}"
    );
    assert!(said.contains("2 files"), "{said}");
    let mut wanted = holding(FEATURE);
    wanted.insert("a.txt".into(), b"mine\n".to_vec());
    wanted.insert("n.txt".into(), b"new\n".to_vec());
    assert_eq!(files(env.root()), wanted);
    assert_eq!(changed(&env), ["a.txt", "n.txt"]);
}

#[test]
fn goto_stops_at_colliding_changes_and_changes_nothing() {
    let env = two_lines();
    env.overwrite("b.txt", "mine\n");
    env.overwrite("dir/c.bin", "mine too");
    let before = (env.standing(), files(env.root()));
    let out = env.opalro().args(["goto", "feature"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    let said = text(&out.stderr);
    assert!(
        said.contains("b.txt") && said.contains("dir/c.bin"),
        "{said}"
    );
    assert!(
        said.contains("hint:") && said.contains("opalro save"),
        "{said}"
    );
    assert_eq!((env.standing(), files(env.root())), before);
    assert!(!env.root().join(".opalro/state/journal").exists());
    assert_eq!(env.state().1.position, Position::OnLine("main".into()));
}

#[test]
fn goto_a_save_at_a_lines_tip_is_at_the_save() {
    let env = two_lines();
    let feature = env.state().1.lines["feature"];
    let said = goto(&env, &feature.to_hex()[..12]);
    assert!(said.contains("not on a line"), "{said}");
    assert!(
        said.contains("Saving here asks for a new line's name."),
        "{said}"
    );
    assert!(
        said.contains("To keep saving on feature: opalro goto feature"),
        "{said}"
    );
    // Of the lines there, the first by name.
    assert!(!said.contains("opalro goto zz"), "{said}");
    assert_eq!(env.state().1.position, Position::AtSave(feature));
    assert_eq!(files(env.root()), holding(FEATURE));
}

#[test]
fn goto_a_server_line_is_at_its_save() {
    let env = two_lines();
    let (repo, state) = env.state();
    let at = repo.server_lines(&state).unwrap().servers["origin"]["x"];
    drop(repo);
    let said = goto(&env, "x@origin");
    assert!(said.contains("not on a line"), "{said}");
    assert_eq!(env.state().1.position, Position::AtSave(at));
    assert_eq!(files(env.root()), holding(FEATURE));
}

/// Commits a save on `main` with the change ID `change_id` and the same files as the last.
fn plant_with_change_id(root: &Path, change_id: ChangeId) {
    let mut repo = Repository::open(root).unwrap();
    let lock = repo.lock_for_change("save", &mut |_| {}).unwrap();
    let (head, entry) = repo.current().unwrap();
    let parent_id = entry.state.lines["main"];
    let parent = repo.store().save(&parent_id, false).unwrap();
    let mut pack = repo.new_packs().unwrap();
    let save = Save {
        parents: vec![parent_id],
        message: "Planted".into(),
        change_id,
        ..parent
    };
    let save_id = pack.add_record(ObjectKind::Save, &save.encode()).unwrap();
    let mut state = entry.state.clone();
    state.lines.insert("main".into(), save_id);
    let diary = DiaryEntry {
        parent: Some(head),
        command: "save".into(),
        state,
        folder_change: None,
        ..entry
    };
    repo.commit(&lock, pack, &diary).unwrap();
}

#[test]
fn an_ambiguous_target_lists_the_saves() {
    let env = Env::empty();
    env.opalro()
        .args(["start", "--preset", "custom"])
        .assert()
        .success();
    let mut second = [0; 16];
    second[15] = 1;
    plant_with_change_id(env.root(), ChangeId([0; 16]));
    plant_with_change_id(env.root(), ChangeId(second));
    let before = env.standing();
    let out = env.opalro().args(["goto", "zzzz"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    let said = text(&out.stderr);
    assert!(said.contains("matches more than one save"), "{said}");
    assert!(said.contains(&ChangeId([0; 16]).to_letters()), "{said}");
    assert!(said.contains(&ChangeId(second).to_letters()), "{said}");
    assert_eq!(env.standing(), before);
}

#[test]
fn goto_records_its_switch_in_its_diary_entry() {
    let env = two_lines();
    let (repo, state) = env.state();
    let before = repo.head().unwrap();
    drop(repo);
    env.overwrite("a.txt", "mine\n");
    let doc = goto_json(&env, "feature");
    let repo = Repository::open(env.root()).unwrap();
    let (_, entry) = repo.current().unwrap();
    assert_eq!(entry.parent, Some(before));
    assert_eq!(entry.action, Action::Command);
    assert_eq!(entry.command, "goto");
    assert_eq!(entry.state.position, Position::OnLine("feature".into()));
    // The unsaved a.txt is the same in both saves, so the switch leaves it out.
    let switched = FolderChange {
        from: state.lines["main"],
        to: state.lines["feature"],
    };
    assert_eq!(entry.folder_change, Some(switched));
    // Goto never overwrites unsaved work, so what it carries along needs no copy.
    assert_eq!(doc["safety_copies"], 0, "{doc}");
    assert_eq!(std::fs::read(env.root().join("a.txt")).unwrap(), b"mine\n");
}

#[test]
fn goto_json_matches_its_snapshot() {
    let env = two_lines();
    let mut doc = goto_json(&env, "feature");
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
    // Going where the project already is records no step.
    assert_eq!(goto_json(&env, "feature")["recorded"], false);
}

/// A project whose line `main` holds `second`, on the save before, which holds `first`.
fn on_first_of_two(first: &[(&str, &str)], second: &[(&str, &str)]) -> Env {
    let env = Env::empty();
    env.opalro()
        .args(["start", "--preset", "custom"])
        .assert()
        .success();
    write_all(&env, first);
    env.opalro().args(["save", "First"]).assert().success();
    let saved = env.state().1.lines["main"];
    for e in std::fs::read_dir(env.root()).unwrap() {
        let e = e.unwrap();
        if e.file_name() != ".opalro" {
            let path = e.path();
            if path.is_dir() {
                std::fs::remove_dir_all(path).unwrap();
            } else {
                std::fs::remove_file(path).unwrap();
            }
        }
    }
    write_all(&env, second);
    env.opalro().args(["save", "Second"]).assert().success();
    goto(&env, &saved.to_hex());
    assert_eq!(files(env.root()), holding(first));
    env
}

#[test]
fn a_deleted_file_the_target_lacks_too_comes_along() {
    let env = on_first_of_two(&[("a", "a"), ("b", "b")], &[("a", "a")]);
    std::fs::remove_file(env.root().join("b")).unwrap();
    let said = goto(&env, "main");
    // Matching the target, the deletion is no longer an unsaved change to carry.
    assert!(!said.contains("came along"), "{said}");
    assert_eq!(files(env.root()), holding(&[("a", "a")]));
    assert!(changed(&env).is_empty());
}

#[test]
fn a_deleted_folder_the_target_lacks_too_comes_along() {
    let env = on_first_of_two(&[("a", "a"), ("d/x", "x")], &[("a", "a")]);
    std::fs::remove_dir_all(env.root().join("d")).unwrap();
    goto(&env, "main");
    assert_eq!(files(env.root()), holding(&[("a", "a")]));
    assert!(!env.root().join("d").exists());
    assert!(changed(&env).is_empty());
}

#[test]
fn a_file_deleted_from_a_folder_the_target_lacks_comes_along() {
    let env = on_first_of_two(&[("a", "a"), ("d/x", "x")], &[("a", "a")]);
    std::fs::remove_file(env.root().join("d/x")).unwrap();
    goto(&env, "main");
    assert_eq!(files(env.root()), holding(&[("a", "a")]));
    assert!(!env.root().join("d").exists());
    assert!(changed(&env).is_empty());
}

#[test]
fn a_file_changed_to_the_targets_contents_comes_along() {
    let env = on_first_of_two(&[("a", "old")], &[("a", "new")]);
    env.overwrite("a", "new");
    goto(&env, "main");
    assert_eq!(files(env.root()), holding(&[("a", "new")]));
    assert!(changed(&env).is_empty());
}

#[test]
fn a_file_the_target_adds_in_a_deleted_folder_is_written() {
    let env = on_first_of_two(&[("d/x", "x")], &[("d/x", "x"), ("d/e/b", "b")]);
    std::fs::remove_dir_all(env.root().join("d")).unwrap();
    goto(&env, "main");
    assert_eq!(files(env.root()), holding(&[("d/e/b", "b")]));
    assert_eq!(changed(&env), ["d/x"]);
}

#[test]
fn a_deleted_folder_stays_deleted_when_the_target_only_removes_below_it() {
    let env = on_first_of_two(&[("d/x", "x"), ("d/e/y", "y")], &[("d/x", "x")]);
    std::fs::remove_dir_all(env.root().join("d")).unwrap();
    goto(&env, "main");
    assert!(!env.root().join("d").exists());
    assert_eq!(changed(&env), ["d/x"]);
}

#[test]
fn a_new_file_in_a_folder_the_target_lacks_still_collides() {
    let env = on_first_of_two(&[("a", "a"), ("d/x", "x")], &[("a", "a")]);
    std::fs::remove_file(env.root().join("d/x")).unwrap();
    env.overwrite("d/y", "mine");
    let before = (env.standing(), files(env.root()));
    let out = env.opalro().args(["goto", "main"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(
        text(&out.stderr).contains("in the way"),
        "{}",
        text(&out.stderr)
    );
    assert_eq!((env.standing(), files(env.root())), before);
}

#[test]
fn goto_where_the_project_is_carries_nothing_along() {
    let env = two_lines();
    env.overwrite(
        "a.txt", "mine
",
    );
    let before = env.standing();
    let said = goto(&env, "main");
    assert!(said.starts_with("Already on line main, at save "), "{said}");
    assert!(!said.contains("came along"), "{said}");
    assert_eq!(env.standing(), before);
}

#[cfg(windows)]
#[test]
fn a_goto_with_a_file_in_use_fails_and_the_next_one_finishes() {
    use std::time::{Duration, SystemTime};
    let env = two_lines();
    // Old enough for the file-state cache to vouch for, so neither the plan nor the switch has to
    // read the held file.
    let then = SystemTime::now() - Duration::from_secs(3600);
    for path in files(env.root()).keys() {
        let file = std::fs::File::options()
            .write(true)
            .open(env.root().join(path))
            .unwrap();
        file.set_modified(then).unwrap();
    }
    assert!(changed(&env).is_empty());
    let held = hold_open(&env.root().join("b.txt"));
    let out = env.opalro().args(["goto", "feature"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1), "{}", text(&out.stdout));
    let said = text(&out.stderr);
    assert!(said.contains("error:") && said.contains("b.txt"), "{said}");
    assert!(text(&out.stdout).is_empty());
    let out = env
        .opalro()
        .args(["goto", "feature", "--json"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let doc: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(
        doc["error"]["message"].as_str().unwrap().contains("b.txt"),
        "{doc}"
    );
    let (kinds, _) = warning_kinds(&env, &["status"]);
    assert!(kinds.contains(&"switch_unfinished".to_owned()), "{kinds:?}");

    drop(held);
    let (kinds, doc) = warning_kinds(&env, &["goto", "feature"]);
    assert!(kinds.contains(&"switch_finished".to_owned()), "{doc}");
    assert_eq!(doc["position"]["line"], "feature", "{doc}");
    assert_eq!(files(env.root()), holding(FEATURE));
    assert!(!env.root().join(".opalro/state/journal").exists());
}

// A link leading out of the project is written as a plain file holding its target, on every
// system; a switch takes that file for the link, as a save does.

use opal_core::id::Id;
use opal_core::listing::EntryKind::{File, Symlink};
use support::planted_saves;

/// Where `at_a_placeholder`'s saves are among the IDs it gives.
const HOLDING_OUT: usize = 0;
const WITHOUT: usize = 1;
const AS_FILE: usize = 2;
const AS_FOLDER: usize = 3;
const AS_OTHER_LINK: usize = 4;
#[cfg(feature = "fault-injection")]
const KEEP_CHANGED: usize = 5;

/// A project at a save holding `out`, a link leading out of the project written as a
/// placeholder, and saves to go to where `out` is gone, a file of other bytes, a folder, or
/// another link, and one where it is gone and `keep.txt` changed. Their IDs, as indexed above.
fn at_a_placeholder() -> (Env, Vec<Id>) {
    let keep = ("keep.txt", File, b"k\n" as &[u8]);
    let (env, ids) = planted_saves(&[
        &[("out", Symlink, b"../outside"), keep],
        &[keep],
        &[("out", File, b"other\n"), keep],
        &[("out/inside.txt", File, b"in\n"), keep],
        &[("out", Symlink, b"../elsewhere"), keep],
        &[("keep.txt", File, b"K\n")],
    ]);
    let doc = goto_json(&env, &ids[HOLDING_OUT].to_hex());
    assert_eq!(doc["placeholders"], serde_json::json!(["out"]), "{doc}");
    assert_eq!(
        files(env.root()),
        holding(&[("keep.txt", "k\n"), ("out", "../outside")])
    );
    assert!(changed(&env).is_empty());
    (env, ids)
}

#[test]
fn a_placeholder_is_removed_as_the_link_it_stands_for() {
    let (env, ids) = at_a_placeholder();
    let (kinds, doc) = warning_kinds(&env, &["goto", &ids[WITHOUT].to_hex()]);
    assert!(kinds.is_empty(), "{doc}");
    assert_eq!(doc["removed"], 1, "{doc}");
    assert_eq!(files(env.root()), holding(&[("keep.txt", "k\n")]));
    assert!(changed(&env).is_empty());
}

#[test]
fn a_placeholder_is_replaced_as_the_link_it_stands_for() {
    let (env, ids) = at_a_placeholder();
    for (to, then, warned) in [
        (AS_FILE, &[("out", "other\n")], &[][..]),
        (AS_FOLDER, &[("out/inside.txt", "in\n")], &[]),
        (
            AS_OTHER_LINK,
            &[("out", "../elsewhere")],
            &["placeholders_written"],
        ),
    ] {
        let (kinds, doc) = warning_kinds(&env, &["goto", &ids[to].to_hex()]);
        assert_eq!(kinds, warned, "{doc}");
        let mut wanted = holding(then);
        wanted.insert("keep.txt".into(), b"k\n".to_vec());
        assert_eq!(files(env.root()), wanted);
        assert!(changed(&env).is_empty());
        goto(&env, &ids[HOLDING_OUT].to_hex());
    }
}

#[cfg(feature = "fault-injection")]
#[test]
fn a_stopped_switch_finishes_by_removing_a_placeholder_as_its_link() {
    let (env, ids) = at_a_placeholder();
    let to = ids[KEEP_CHANGED].to_hex();
    // Stopped once `keep.txt` is written, before `out` goes.
    let out = env
        .opalro()
        .args(["goto", &to])
        .env("OPALRO_FAULT_FAIL", "switch:changed")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1), "{}", text(&out.stdout));
    assert!(env.root().join(".opalro/state/journal").exists());
    let (kinds, doc) = warning_kinds(&env, &["goto", &to]);
    assert_eq!(kinds, ["switch_finished"], "{doc}");
    assert_eq!(files(env.root()), holding(&[("keep.txt", "K\n")]));
    assert!(changed(&env).is_empty());
}

#[test]
fn a_placeholder_you_changed_is_unsaved_work() {
    let (env, ids) = at_a_placeholder();
    std::fs::write(env.root().join("out"), "mine\n").unwrap();
    assert_eq!(changed(&env), ["out"]);
    let out = env
        .opalro()
        .args(["goto", &ids[WITHOUT].to_hex()])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1), "{}", text(&out.stdout));
    let said = text(&out.stderr);
    assert!(
        said.contains("in the way") && said.contains("\n  out"),
        "{said}"
    );
    assert_eq!(std::fs::read(env.root().join("out")).unwrap(), b"mine\n");
}

/// A goto to a save whose files' pack is gone stops at the file it can't write and leaves that
/// file as it was. Where it stops partway, its hint names the way on, and that way works.
#[test]
fn a_goto_that_needs_a_pack_that_is_gone_stops_and_names_the_way_on() {
    let env = Env::empty();
    env.opalro()
        .args(["start", "--preset", "custom"])
        .assert()
        .success();
    let packs = |env: &Env| -> Vec<std::path::PathBuf> {
        let folder = env.root().join(".opalro/store/packs");
        std::fs::read_dir(folder)
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect()
    };
    let started = packs(&env);
    support::write(&env, "f.txt", "x\n");
    support::write(&env, "g.txt", "a\n");
    env.opalro().args(["save", "One"]).assert().success();
    let first: Vec<_> = packs(&env)
        .into_iter()
        .filter(|p| !started.contains(p))
        .collect();
    support::write(&env, "g.txt", "b\n");
    env.opalro().args(["save", "Two"]).assert().success();
    let two = support::main_tip(&env).to_hex();
    support::write(&env, "f.txt", "z\n");
    env.opalro().args(["save", "Three"]).assert().success();
    for pack in first {
        std::fs::remove_file(pack).unwrap();
    }
    let out = env.opalro().args(["goto", &two]).output().unwrap();
    let said = text(&out.stderr);
    assert_eq!(out.status.code(), Some(1), "{}", text(&out.stdout));
    assert!(said.contains("is not in the store"), "{said}");
    assert_eq!(std::fs::read(env.root().join("f.txt")).unwrap(), b"z\n");
    let journal = env.root().join(".opalro/state/journal");
    if journal.exists() {
        assert!(said.contains("delete .opalro/state/journal"), "{said}");
        std::fs::remove_file(&journal).unwrap();
    }
    support::write(&env, "h.txt", "new\n");
    env.opalro().args(["save", "Four"]).assert().success();
    assert_eq!(std::fs::read(env.root().join("f.txt")).unwrap(), b"z\n");
}
