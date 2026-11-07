//! `bolt undo` and `bolt redo`, end to end.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use bolt_core::diary::{Action, DiaryEntry, Position, State};
use bolt_core::id::Id;
use bolt_core::listing::EntryKind;
use bolt_core::pack::ObjectKind;
use bolt_core::repo::Repository;

mod support;
use support::{Env, text};

/// Every file below `dir` but `.bolt`, by path, with what it holds.
fn files(dir: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut found = BTreeMap::new();
    let mut folders = vec![(dir.to_path_buf(), String::new())];
    while let Some((at, rel)) = folders.pop() {
        for e in fs::read_dir(&at).unwrap() {
            let e = e.unwrap();
            let name = e.file_name().to_string_lossy().into_owned();
            if rel.is_empty() && name == ".bolt" {
                continue;
            }
            let path = if rel.is_empty() {
                name
            } else {
                format!("{rel}/{name}")
            };
            if e.file_type().unwrap().is_dir() {
                folders.push((e.path(), path));
            } else {
                found.insert(path, fs::read(e.path()).unwrap());
            }
        }
    }
    found
}

fn holding(entries: &[(&str, &str)]) -> BTreeMap<String, Vec<u8>> {
    entries
        .iter()
        .map(|(p, b)| ((*p).to_owned(), b.as_bytes().to_vec()))
        .collect()
}

fn write(env: &Env, path: &str, bytes: &str) {
    let at = env.root().join(path);
    fs::create_dir_all(at.parent().unwrap()).unwrap();
    fs::write(at, bytes).unwrap();
}

/// A project with `entries` saved on `main`.
fn project(entries: &[(&str, &str)]) -> Env {
    let env = Env::empty();
    env.bolt().arg("start").assert().success();
    for (path, bytes) in entries {
        write(&env, path, bytes);
    }
    ok(&env, &["save", "Files"]);
    env
}

/// `bolt <args>`, which must succeed; what it printed.
fn ok(env: &Env, args: &[&str]) -> String {
    let out = env.bolt().args(args).output().unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "{args:?}: {}",
        text(&out.stderr)
    );
    text(&out.stdout)
}

/// `bolt <args>`, which must fail; what it said, on stdout then stderr.
fn refused(env: &Env, args: &[&str]) -> (String, String) {
    let out = env.bolt().args(args).output().unwrap();
    assert_eq!(
        out.status.code(),
        Some(1),
        "{args:?}: {}",
        text(&out.stdout)
    );
    (text(&out.stdout), text(&out.stderr))
}

fn json(env: &Env, args: &[&str]) -> serde_json::Value {
    let out = env.bolt().args(args).arg("--json").output().unwrap();
    serde_json::from_slice(&out.stdout).unwrap()
}

/// What `bolt status` finds changed.
fn changed(env: &Env) -> Vec<String> {
    let doc = json(env, &["status"]);
    doc["changes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["path"].as_str().unwrap().to_owned())
        .collect()
}

fn state(env: &Env) -> State {
    env.state().1
}

fn main_tip(env: &Env) -> Id {
    state(env).lines["main"]
}

/// The newest diary entry, with its ID.
fn head(env: &Env) -> (Id, DiaryEntry) {
    Repository::open(env.root()).unwrap().current().unwrap()
}

fn entry(env: &Env, id: Id) -> DiaryEntry {
    let repo = Repository::open(env.root()).unwrap();
    let bytes = repo
        .store()
        .read_record(&id, ObjectKind::DiaryEntry)
        .unwrap();
    DiaryEntry::decode(&bytes).unwrap()
}

/// Every file of the save `save`, by path, with what it holds.
fn saved_files(env: &Env, save: Id) -> BTreeMap<String, Vec<u8>> {
    let repo = Repository::open(env.root()).unwrap();
    let store = repo.store();
    let mut found = BTreeMap::new();
    let mut folders = vec![(store.save(&save, false).unwrap().root, String::new())];
    while let Some((id, rel)) = folders.pop() {
        for e in store.listing(&id).unwrap().entries() {
            let path = if rel.is_empty() {
                e.name.clone()
            } else {
                format!("{rel}/{}", e.name)
            };
            if e.kind == EntryKind::Folder {
                folders.push((e.id, path));
            } else {
                let mut bytes = Vec::new();
                store.copy_content(&e.id, &mut bytes).unwrap();
                found.insert(path, bytes);
            }
        }
    }
    found
}

fn journal_exists(env: &Env) -> bool {
    env.root().join(".bolt/state/journal").exists()
}

const ONE: &[(&str, &str)] = &[("a.txt", "a1\n"), ("b.txt", "b1\n"), ("dir/c.txt", "c1\n")];
const TWO: &[(&str, &str)] = &[("a.txt", "a2\n"), ("b.txt", "b2\n"), ("dir/c.txt", "c2\n")];

/// A project whose `main` holds `TWO`, saved over `ONE`, at the save of `ONE` after a goto that
/// wrote three files. Gives that save and the goto's entry.
fn after_a_goto() -> (Env, Id, Id) {
    let env = project(ONE);
    let one = main_tip(&env);
    for (path, bytes) in TWO {
        write(&env, path, bytes);
    }
    ok(&env, &["save", "Two"]);
    ok(&env, &["goto", &one.to_hex()]);
    assert_eq!(files(env.root()), holding(ONE));
    let goto = head(&env).0;
    (env, one, goto)
}

#[test]
fn undo_of_a_save_moves_the_line_back_and_keeps_the_files() {
    let env = project(&[("a.txt", "a\n")]);
    let first = main_tip(&env);
    write(&env, "a.txt", "A\n");
    ok(&env, &["save", "Change a"]);
    let said = ok(&env, &["undo"]);
    assert!(said.contains("Undid save"), "{said}");
    assert!(said.contains("\"Change a\""), "{said}");
    assert!(said.contains("main is back at"), "{said}");
    assert!(said.contains("its changes show as unsaved"), "{said}");
    assert_eq!(main_tip(&env), first);
    assert_eq!(files(env.root()), holding(&[("a.txt", "A\n")]));
    assert_eq!(changed(&env), ["a.txt"]);
}

#[test]
fn redo_of_a_save_moves_it_forward_again() {
    let env = project(&[("a.txt", "a\n")]);
    write(&env, "a.txt", "A\n");
    ok(&env, &["save", "Change a"]);
    let second = main_tip(&env);
    ok(&env, &["undo"]);
    let said = ok(&env, &["redo"]);
    assert!(said.contains("Redid save"), "{said}");
    assert!(said.contains("\"Change a\""), "{said}");
    assert_eq!(main_tip(&env), second);
    assert!(changed(&env).is_empty());
    let (_, newest) = head(&env);
    assert_eq!(newest.command, "redo");
}

#[test]
fn undo_of_a_goto_switches_back_carrying_unsaved_work() {
    let (env, _, _) = after_a_goto();
    write(&env, "n.txt", "new\n");
    let said = ok(&env, &["undo"]);
    assert!(said.contains("Undid goto"), "{said}");
    assert!(said.contains("back on main"), "{said}");
    assert!(said.contains("3 files written"), "{said}");
    assert_eq!(state(&env).position, Position::OnLine("main".into()));
    let mut wanted = holding(TWO);
    wanted.insert("n.txt".into(), b"new\n".to_vec());
    assert_eq!(files(env.root()), wanted);
    assert_eq!(changed(&env), ["n.txt"]);
}

const SAVED: &[(&str, &str)] = &[
    ("a.txt", "a\n"),
    ("b.txt", "b\n"),
    ("dir/c.txt", "c\n"),
    ("gone/d.txt", "d\n"),
];

/// A project holding `SAVED`, then a changed, a new and a deleted file, a new folder and a deleted
/// one. Gives the folder as it then is, and what status found changed.
fn with_unsaved_work() -> (Env, BTreeMap<String, Vec<u8>>, Vec<String>) {
    let env = project(SAVED);
    write(&env, "a.txt", "mine\n");
    write(&env, "n.txt", "new\n");
    write(&env, "newdir/x.txt", "x\n");
    fs::remove_file(env.root().join("b.txt")).unwrap();
    fs::remove_dir_all(env.root().join("gone")).unwrap();
    let changes = changed(&env);
    assert_eq!(changes.len(), 5, "{changes:?}");
    let now = files(env.root());
    (env, now, changes)
}

#[test]
fn undo_of_a_discard_brings_back_changed_new_and_deleted_files_and_folders() {
    let (env, before, changes) = with_unsaved_work();
    ok(&env, &["discard"]);
    assert_eq!(files(env.root()), holding(SAVED));
    let said = ok(&env, &["undo"]);
    assert!(said.contains("Undid discard"), "{said}");
    assert_eq!(files(env.root()), before);
    assert!(!env.root().join("gone").exists());
    assert!(env.root().join("newdir").is_dir());
    assert_eq!(changed(&env), changes);
}

#[test]
fn redo_of_a_discard_throws_them_away_again() {
    let (env, _, _) = with_unsaved_work();
    ok(&env, &["discard"]);
    ok(&env, &["undo"]);
    let said = ok(&env, &["redo"]);
    assert!(said.contains("Redid discard"), "{said}");
    assert_eq!(files(env.root()), holding(SAVED));
    assert!(!env.root().join("newdir").exists());
    assert!(changed(&env).is_empty());
}

#[test]
fn undo_of_a_discard_stops_when_the_file_was_edited_again() {
    let env = project(SAVED);
    write(&env, "a.txt", "mine\n");
    ok(&env, &["discard", "a.txt"]);
    write(&env, "a.txt", "again\n");
    let (at, _) = head(&env);
    let (_, said) = refused(&env, &["undo"]);
    assert!(said.contains("a.txt"), "{said}");
    assert!(said.contains("bolt undo --shelve"), "{said}");
    assert_eq!(fs::read(env.root().join("a.txt")).unwrap(), b"again\n");
    assert_eq!(head(&env).0, at);
}

#[test]
fn undo_shelve_sets_the_work_in_the_way_aside_and_undoes_in_one_step() {
    let (env, one, goto) = after_a_goto();
    write(&env, "a.txt", "mine\n");
    // Work out of the step's way stays where it is, and off the shelf.
    write(&env, "n.txt", "new\n");
    let (_, said) = refused(&env, &["undo"]);
    assert!(said.contains("a.txt"), "{said}");
    let said = ok(&env, &["undo", "--shelve"]);
    assert!(said.contains("Undid goto"), "{said}");
    assert!(said.contains("shelf 1"), "{said}");
    // Back on the save left, the edit on a shelf of its own.
    assert_eq!(state(&env).position, Position::OnLine("main".into()));
    let with_new = |files: &[(&str, &str)]| {
        let mut wanted = holding(files);
        wanted.insert("n.txt".into(), b"new\n".to_vec());
        wanted
    };
    assert_eq!(files(env.root()), with_new(TWO));
    assert_eq!(changed(&env), ["n.txt"]);
    let shelves = state(&env).shelves;
    assert_eq!(shelves.len(), 1);
    let (label, shelf) = shelves[0].clone();
    assert_eq!(label, "1");
    let repo = Repository::open(env.root()).unwrap();
    let kept = repo.store().save(&shelf, false).unwrap();
    assert_eq!(kept.parents, [one]);
    assert_eq!(kept.message, "Set aside by undo");
    let mut shelved = holding(ONE);
    shelved.insert("a.txt".into(), b"mine\n".to_vec());
    assert_eq!(saved_files(&env, shelf), shelved);
    // One entry: the goto undone, and no shelf step of its own.
    let (_, newest) = head(&env);
    assert_eq!(newest.action, Action::Undo(goto));
    assert_eq!(newest.parent, Some(goto));
    // Redo brings the goto back; the shelf stays.
    ok(&env, &["redo"]);
    assert_eq!(files(env.root()), with_new(ONE));
    assert_eq!(state(&env).shelves, shelves);
}

#[test]
fn undo_shelve_stops_at_an_ignored_file_in_the_way_and_stores_nothing() {
    let env = project(&[("a.txt", "a1\n")]);
    let one = main_tip(&env);
    write(&env, "a.txt", "a2\n");
    write(&env, "out.txt", "saved\n");
    ok(&env, &["save", "Two"]);
    ok(&env, &["goto", &one.to_hex()]);
    // The undo would write out.txt back, where the folder now has a file Bolt ignores; a.txt,
    // which it writes too, has an edit of its own.
    write(&env, ".gitignore", "out.txt\n");
    write(&env, "out.txt", "build output\n");
    write(&env, "a.txt", "mine\n");
    let folder = files(env.root());
    let (at, packs) = env.standing();
    let (_, said) = refused(&env, &["undo", "--shelve"]);
    assert!(said.contains("out.txt"), "{said}");
    assert!(said.contains("can't set them aside"), "{said}");
    assert!(said.contains("then run bolt undo --shelve again"), "{said}");
    assert_eq!(files(env.root()), folder);
    assert_eq!(env.standing(), (at, packs));
    assert!(state(&env).shelves.is_empty());
}

#[test]
fn undo_and_redo_keep_shelves_but_their_own() {
    let (env, _, _) = after_a_goto();
    write(&env, "a.txt", "mine\n");
    ok(&env, &["undo", "--shelve"]);
    let shelves = state(&env).shelves;
    assert_eq!(shelves.len(), 1);
    ok(&env, &["redo"]);
    assert_eq!(state(&env).shelves, shelves);
    ok(&env, &["undo"]);
    assert_eq!(state(&env).shelves, shelves);
    ok(&env, &["undo"]);
    assert_eq!(state(&env).shelves, shelves);
}

#[cfg(feature = "fault-injection")]
#[test]
fn running_undo_again_after_a_stop_finishes_it_and_takes_no_further_step() {
    let (env, _, goto) = after_a_goto();
    let out = env
        .bolt()
        .arg("undo")
        .env("BOLT_FAULT_FAIL", "switch:changed")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1), "{}", text(&out.stdout));
    assert!(journal_exists(&env));
    assert_eq!(head(&env).0, goto);
    let said = ok(&env, &["undo"]);
    assert!(said.contains("Finished"), "{said}");
    assert!(said.contains("bolt undo again"), "{said}");
    let (_, newest) = head(&env);
    assert_eq!(newest.action, Action::Undo(goto));
    assert_eq!(newest.parent, Some(goto));
    assert_eq!(files(env.root()), holding(TWO));
    assert!(!journal_exists(&env));
}

#[test]
fn undo_with_nothing_to_undo_says_so_and_succeeds() {
    let env = Env::empty();
    env.bolt().arg("start").assert().success();
    let (at, _) = head(&env);
    assert!(ok(&env, &["undo"]).contains("Nothing to undo"));
    assert!(ok(&env, &["redo"]).contains("Nothing to redo"));
    assert_eq!(head(&env).0, at);
}

#[test]
fn undo_three_takes_three_steps_one_entry_each() {
    let env = project(&[("a.txt", "a\n")]);
    let mut saves = vec![head(&env).0];
    for name in ["b.txt", "c.txt"] {
        write(&env, name, "x\n");
        ok(&env, &["save", name]);
        saves.push(head(&env).0);
    }
    let said = ok(&env, &["undo", "3"]);
    assert_eq!(said.matches("Undid save").count(), 3, "{said}");
    // The newest step was undone first, so walking back meets its undo last.
    let mut at = head(&env).0;
    for step in &saves {
        let undo = entry(&env, at);
        assert_eq!(undo.action, Action::Undo(*step));
        at = undo.parent.unwrap();
    }
    assert_eq!(at, saves[2]);
    // Only the project's first entry is left, and that can't be undone.
    assert!(ok(&env, &["undo"]).contains("Nothing to undo"));
    assert_eq!(ok(&env, &["redo", "3"]).matches("Redid save").count(), 3);
}

#[test]
fn undo_two_stops_at_the_first_it_cannot_take_and_says_how_many_were_taken() {
    let (env, _, _) = after_a_goto();
    write(&env, "n.txt", "new\n");
    ok(&env, &["discard"]);
    write(&env, "a.txt", "mine\n");
    let (out, said) = refused(&env, &["undo", "2"]);
    assert!(out.contains("Undid discard"), "{out}");
    assert!(out.contains("Undid 1 of the 2 steps"), "{out}");
    assert!(said.contains("a.txt"), "{said}");
    assert!(said.contains("so that step changed nothing"), "{said}");
    assert_eq!(fs::read(env.root().join("a.txt")).unwrap(), b"mine\n");
    assert_eq!(fs::read(env.root().join("n.txt")).unwrap(), b"new\n");
}

#[test]
fn a_new_command_clears_what_redo_could_bring_back() {
    let env = project(&[("a.txt", "a\n")]);
    write(&env, "b.txt", "b\n");
    ok(&env, &["save", "Add b"]);
    ok(&env, &["undo"]);
    write(&env, "c.txt", "c\n");
    ok(&env, &["save", "Add b and c"]);
    let (at, _) = head(&env);
    assert!(ok(&env, &["redo"]).contains("Nothing to redo"));
    assert_eq!(head(&env).0, at);
}

#[cfg(feature = "fault-injection")]
#[test]
fn an_undo_stopped_at_every_write_is_finished_by_the_next_command() {
    use support::copy_dir;
    let (template, one, goto) = after_a_goto();
    let (old, new) = (holding(ONE), holding(TWO));
    let mut finished_later = 0;
    for n in 1.. {
        assert!(n < 200, "the undo never finished");
        let env = Env::empty();
        copy_dir(template.root(), env.root());
        let out = env
            .bolt()
            .arg("undo")
            .env("BOLT_FAULT_AT", n.to_string())
            .output()
            .unwrap();
        let crashed = text(&out.stderr).contains("injected crash");
        // Every file is wholly old or new.
        for (path, bytes) in files(env.root()) {
            assert!(
                old.get(&path) == Some(&bytes) || new.get(&path) == Some(&bytes),
                "fault {n}: {path} is neither"
            );
        }
        let unfinished = warned(&json(&env, &["status"]), "switch_unfinished");
        // The next command that changes the project finishes it, and then it is done.
        let next = json(&env, &["discard"]);
        assert_eq!(next["error"], serde_json::Value::Null, "fault {n}: {next}");
        assert_eq!(warned(&next, "switch_finished"), unfinished, "fault {n}");
        assert!(!journal_exists(&env), "fault {n}");
        let (at, newest) = head(&env);
        if newest.action == Action::Undo(goto) {
            assert_eq!(files(env.root()), new, "fault {n}");
            assert_eq!(newest.state.position, Position::OnLine("main".into()));
        } else {
            assert_eq!(at, goto, "fault {n}");
            assert_eq!(newest.state.position, Position::AtSave(one), "fault {n}");
            assert_eq!(files(env.root()), old, "fault {n}");
        }
        if unfinished {
            finished_later += 1;
        }
        if !crashed {
            assert!(n > 1, "no fault point was reached");
            assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
            assert_eq!(newest.action, Action::Undo(goto));
            // Stopped after each of the three writes, at least, and finished later.
            assert!(finished_later >= 3, "only {finished_later} finished later");
            break;
        }
    }
}

/// Whether the `--json` document `doc` has a warning of `kind`.
#[cfg(feature = "fault-injection")]
fn warned(doc: &serde_json::Value, kind: &str) -> bool {
    let warnings = doc["warnings"].as_array().unwrap();
    warnings.iter().any(|w| w["kind"] == kind)
}

#[cfg(feature = "fault-injection")]
#[test]
fn an_undo_shelve_stopped_anywhere_is_finished_by_the_next_command() {
    use support::copy_dir;
    let (template, _, goto) = after_a_goto();
    write(&template, "a.txt", "mine\n");
    let (old, new) = (holding(ONE), holding(TWO));
    let mut edited = old.clone();
    edited.insert("a.txt".into(), b"mine\n".to_vec());
    let mut finished_later = 0;
    for n in 1.. {
        assert!(n < 300, "the undo never finished");
        let env = Env::empty();
        copy_dir(template.root(), env.root());
        let out = env
            .bolt()
            .args(["undo", "--shelve"])
            .env("BOLT_FAULT_AT", n.to_string())
            .output()
            .unwrap();
        let crashed = text(&out.stderr).contains("injected crash");
        // Every file is wholly old or new, or the edit.
        for (path, bytes) in files(env.root()) {
            let known = [&old, &new, &edited].map(|f| f.get(&path) == Some(&bytes));
            assert!(known.contains(&true), "fault {n}: {path} is none of them");
        }
        let unfinished = warned(&json(&env, &["status"]), "switch_unfinished");
        // A command that changes nothing of its own finishes it.
        let next = json(&env, &["discard", "dir/c.txt"]);
        assert_eq!(next["error"], serde_json::Value::Null, "fault {n}: {next}");
        assert_eq!(warned(&next, "switch_finished"), unfinished, "fault {n}");
        assert!(!journal_exists(&env), "fault {n}");
        let (at, newest) = head(&env);
        let shelves = &newest.state.shelves;
        if newest.action == Action::Undo(goto) {
            // Undone, with the edit on its shelf.
            assert_eq!(files(env.root()), new, "fault {n}");
            assert_eq!(shelves.len(), 1, "fault {n}");
            assert_eq!(saved_files(&env, shelves[0].1), edited, "fault {n}");
        } else {
            // Not begun: the edit is where it was, and no shelf was made.
            assert_eq!(at, goto, "fault {n}");
            assert_eq!(files(env.root()), edited, "fault {n}");
            assert!(shelves.is_empty(), "fault {n}");
        }
        if unfinished {
            finished_later += 1;
        }
        if !crashed {
            assert!(n > 1, "no fault point was reached");
            assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
            assert_eq!(newest.action, Action::Undo(goto));
            assert!(finished_later >= 3, "only {finished_later} finished later");
            break;
        }
    }
}

#[cfg(windows)]
#[test]
fn an_undo_with_a_file_held_open_is_finished_by_the_next_command() {
    use std::os::windows::fs::OpenOptionsExt;
    use std::time::{Duration, SystemTime};
    let (env, _, goto) = after_a_goto();
    // Old enough for the file-state cache to vouch for, so nothing has to read the held file.
    let file = fs::File::options()
        .write(true)
        .open(env.root().join("a.txt"))
        .unwrap();
    file.set_modified(SystemTime::now() - Duration::from_secs(3600))
        .unwrap();
    drop(file);
    assert!(changed(&env).is_empty());
    let held = fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(env.root().join("a.txt"))
        .unwrap();
    let (_, said) = refused(&env, &["undo"]);
    assert!(said.contains("a.txt"), "{said}");
    assert!(journal_exists(&env));
    drop(held);
    let said = ok(&env, &["undo"]);
    assert!(said.contains("Finished"), "{said}");
    assert_eq!(files(env.root()), holding(TWO));
    let (_, newest) = head(&env);
    assert_eq!(newest.action, Action::Undo(goto));
    assert_eq!(newest.parent, Some(goto));
    assert!(!journal_exists(&env));
}

#[test]
fn undo_json_lists_the_steps_taken() {
    let (env, one, goto) = after_a_goto();
    let doc = json(&env, &["undo", "2"]);
    assert_eq!(doc["bolt_json"], 1);
    assert_eq!(doc["finished"], false);
    assert_eq!(doc["shelved"], serde_json::json!([]));
    assert_eq!(doc["position"], serde_json::json!({"line": "main"}));
    assert_eq!(doc["warnings"], serde_json::json!([]));
    let undone = doc["undone"].as_array().unwrap();
    assert_eq!(undone.len(), 2, "{doc}");
    assert_eq!(undone[0]["entry"], goto.to_hex());
    assert_eq!(undone[0]["command"], "goto");
    assert_eq!(undone[0]["written"], 3);
    assert_eq!(undone[0]["removed"], 0);
    let kinds: Vec<&str> = undone[0]["effects"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["kind"].as_str().unwrap())
        .collect();
    assert_eq!(kinds, ["position", "files"]);
    assert_eq!(undone[1]["command"], "save");
    assert!(undone[1]["time_ms"].as_i64().unwrap() > 0);
    let line = &undone[1]["effects"][0];
    assert_eq!(line["kind"], "line");
    assert_eq!(line["name"], "main");
    assert_eq!(line["to"], one.to_hex());
    assert_eq!(undone[1]["written"], 0);

    let doc = json(&env, &["redo"]);
    assert_eq!(doc["redone"].as_array().unwrap().len(), 1, "{doc}");

    // A failure is a document of its own that still lists the steps taken.
    write(&env, "b.txt", "mine\n");
    let out = env.bolt().args(["redo", "2", "--json"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    let doc: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(doc["redone"].as_array().unwrap().len(), 0, "{doc}");
    assert!(doc["error"]["message"].as_str().unwrap().contains("b.txt"));
    assert!(doc["error"]["hint"].as_str().unwrap().contains("--shelve"));
}

#[test]
fn a_redo_that_fails_after_a_step_lists_that_step_in_its_document() {
    let (env, _, _) = after_a_goto();
    ok(&env, &["undo", "2"]);
    write(&env, "b.txt", "mine\n");
    // The save comes back; the goto after it would write over b.txt.
    let out = env.bolt().args(["redo", "2", "--json"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1), "{}", text(&out.stderr));
    let doc: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let redone = doc["redone"].as_array().unwrap();
    assert_eq!(redone.len(), 1, "{doc}");
    assert_eq!(redone[0]["command"], "save");
    assert_eq!(doc["shelved"], serde_json::json!([]));
    assert!(doc["error"]["message"].as_str().unwrap().contains("b.txt"));
    // The step taken stands.
    assert!(matches!(head(&env).1.action, Action::Redo(_)));
    assert_eq!(fs::read(env.root().join("b.txt")).unwrap(), b"mine\n");
}
