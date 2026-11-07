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
use bolt_core::text::escape;

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
        // Stopped once its head moved, it left only its journal, which goes without a word.
        let only_journal = journal_exists(&env) && head(&env).1.action == Action::Undo(goto);
        // The next command that changes the project finishes it, and then it is done.
        let next = json(&env, &["discard"]);
        assert_eq!(next["error"], serde_json::Value::Null, "fault {n}: {next}");
        let finished = unfinished && !only_journal;
        assert_eq!(warned(&next, "switch_finished"), finished, "fault {n}");
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
        if finished {
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
        // Stopped once its head moved, it left only its journal, which goes without a word.
        let only_journal = journal_exists(&env) && head(&env).1.action == Action::Undo(goto);
        // A command that changes nothing of its own finishes it.
        let next = json(&env, &["discard", "dir/c.txt"]);
        assert_eq!(next["error"], serde_json::Value::Null, "fault {n}: {next}");
        let finished = unfinished && !only_journal;
        assert_eq!(warned(&next, "switch_finished"), finished, "fault {n}");
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
        if finished {
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

/// Every diary entry from the newest back.
fn diary(env: &Env) -> Vec<Id> {
    let mut ids = Vec::new();
    let mut at = Some(head(env).0);
    while let Some(id) = at {
        ids.push(id);
        at = entry(env, id).parent;
    }
    ids
}

/// The first `digits` hex digits of `id`, as `bolt undo --to` takes them.
fn start_of(id: Id, digits: usize) -> String {
    id.to_hex()[..digits].to_owned()
}

#[test]
fn the_list_shows_redo_steps_then_undo_steps_numbered_from_now() {
    let env = project(&[("a.txt", "a\n")]);
    let mut steps = diary(&env);
    steps.reverse();
    for name in ["b", "c", "d"] {
        write(&env, &format!("{name}.txt"), "x\n");
        ok(&env, &["save", &format!("Add {name}")]);
        steps.push(head(&env).0);
    }
    ok(&env, &["undo", "2"]);
    let said = ok(&env, &["undo", "--list"]);
    let lines: Vec<&str> = said.lines().collect();
    assert_eq!(lines.len(), 5, "{said}");
    // Newest first: the steps undone, numbered for redo N, then those undo can take.
    let shown = [
        ("undone 2", 4, "\"Add d\""),
        ("undone 1", 3, "\"Add c\""),
        ("1", 2, "\"Add b\""),
        ("2", 1, "\"Files\""),
    ];
    for (line, (label, step, message)) in lines.iter().zip(shown) {
        let rest = line.trim_start().strip_prefix(label).unwrap();
        assert!(
            rest.starts_with(&format!("  {}  ", steps[step].short_hex())),
            "{said}"
        );
        assert!(rest.contains("  save: main moved to "), "{said}");
        assert!(rest.ends_with(message), "{said}");
    }
    // The diary's first entry has no number: undo can't take it.
    let first = lines[4].trim_start();
    assert!(first.starts_with(&steps[0].short_hex()), "{said}");
    assert!(
        first.ends_with("  start: the diary's first entry, which can't be undone"),
        "{said}"
    );
    // `bolt redo` brings back the step numbered undone 1, and `bolt undo` takes the one numbered 1.
    assert!(ok(&env, &["redo"]).contains("\"Add c\""));
    ok(&env, &["undo"]);
    assert!(ok(&env, &["undo"]).contains("\"Add b\""));
}

#[test]
fn the_list_shows_at_most_20_undo_steps_and_says_when_there_are_more() {
    // The first entry and 19 saves: 20 steps, every one shown.
    let env = project(&[("a.txt", "0\n")]);
    for i in 1..19 {
        write(&env, "a.txt", &format!("{i}\n"));
        ok(&env, &["save", &format!("Save {i}")]);
    }
    let said = ok(&env, &["undo", "--list"]);
    let lines: Vec<&str> = said.lines().collect();
    assert_eq!(lines.len(), 20, "{said}");
    assert!(lines[18].trim_start().starts_with("19  "), "{said}");
    assert!(lines[19].ends_with("can't be undone"), "{said}");
    assert!(!said.contains("and more"), "{said}");
    // One more: 20 steps undo can take, all shown, and past them only the first entry, which it
    // can't.
    write(&env, "a.txt", "19\n");
    ok(&env, &["save", "Save 19"]);
    let said = ok(&env, &["undo", "--list"]);
    let lines: Vec<&str> = said.lines().collect();
    assert_eq!(lines.len(), 20, "{said}");
    assert!(lines[19].trim_start().starts_with("20  "), "{said}");
    assert!(!said.contains("can't be undone"), "{said}");
    assert!(!said.contains("and more"), "{said}");
    assert_eq!(json(&env, &["undo", "--list"])["more"], false);
    // One more again: 20 are shown, and the list says there are more.
    write(&env, "a.txt", "20\n");
    ok(&env, &["save", "Save 20"]);
    let said = ok(&env, &["undo", "--list"]);
    let lines: Vec<&str> = said.lines().collect();
    assert_eq!(lines.len(), 21, "{said}");
    assert!(lines[19].trim_start().starts_with("20  "), "{said}");
    assert_eq!(
        lines[20],
        "and more: bolt undo --list --all shows the whole diary"
    );
    let doc = json(&env, &["undo", "--list"]);
    assert_eq!(doc["undo"].as_array().unwrap().len(), 20, "{doc}");
    assert_eq!(doc["more"], true);
}

#[test]
fn undo_to_a_step_on_the_list_undoes_one_at_a_time_and_redo_still_works() {
    let env = project(&[("a.txt", "a\n")]);
    let (files_step, files_save) = (head(&env).0, main_tip(&env));
    let mut steps = Vec::new();
    for name in ["b", "c"] {
        write(&env, &format!("{name}.txt"), "x\n");
        ok(&env, &["save", &format!("Add {name}")]);
        steps.push(head(&env).0);
    }
    let said = ok(&env, &["undo", "--to", &start_of(files_step, 12)]);
    assert_eq!(said.matches("Undid save").count(), 2, "{said}");
    assert_eq!(main_tip(&env), files_save);
    // An undo entry each, the newest step undone first.
    let (_, newest) = head(&env);
    assert_eq!(newest.action, Action::Undo(steps[0]));
    let before = entry(&env, newest.parent.unwrap());
    assert_eq!(before.action, Action::Undo(steps[1]));
    assert_eq!(before.parent, Some(steps[1]));
    // Redo brings them back as ever, and --to a step redo can bring back redoes up to it.
    assert!(ok(&env, &["redo"]).contains("\"Add b\""));
    let doc = json(&env, &["undo", "--to", &start_of(steps[1], 8)]);
    let taken = doc["steps"].as_array().unwrap();
    assert_eq!(taken.len(), 1, "{doc}");
    assert_eq!(taken[0]["action"], "redo");
    assert_eq!(taken[0]["entry"], steps[1].to_hex());
    assert_eq!(doc["finished"], false);
    assert_eq!(doc["position"], serde_json::json!({"line": "main"}));
    assert_eq!(head(&env).1.action, Action::Redo(steps[1]));
    assert!(ok(&env, &["redo"]).contains("Nothing to redo"));
}

#[test]
fn undo_to_a_cleared_step_is_one_new_step_that_undo_takes_back() {
    let (env, one, goto) = after_a_goto();
    let two = main_tip(&env);
    ok(&env, &["undo"]);
    // A new step clears the goto from what redo can bring back.
    write(&env, "n.txt", "new\n");
    ok(&env, &["save", "Three"]);
    let three = main_tip(&env);
    let (before, _) = head(&env);
    let said = ok(&env, &["undo", "--to", &start_of(goto, 8)]);
    let went = format!("Went back to entry {} (goto)", goto.short_hex());
    assert!(said.contains(&went), "{said}");
    // One new step, with the goto's lines and position, and its save's files.
    let (jump, made) = head(&env);
    assert_eq!(made.parent, Some(before));
    assert_eq!(made.action, Action::Command);
    assert_eq!(made.command, "undo");
    assert_eq!(made.state.lines["main"], two);
    assert_eq!(made.state.position, Position::AtSave(one));
    assert_eq!(files(env.root()), holding(ONE));
    assert!(changed(&env).is_empty());
    // Undo takes it back.
    let said = ok(&env, &["undo"]);
    let undid = format!("Undid undo --to {}:", goto.short_hex());
    assert!(said.contains(&undid), "{said}");
    assert_eq!(head(&env).1.action, Action::Undo(jump));
    assert_eq!(main_tip(&env), three);
    assert_eq!(state(&env).position, Position::OnLine("main".into()));
    let mut wanted = holding(TWO);
    wanted.insert("n.txt".into(), b"new\n".to_vec());
    assert_eq!(files(env.root()), wanted);
}

#[test]
fn undo_to_a_cleared_step_keeps_the_shelves() {
    let (env, _, goto) = after_a_goto();
    write(&env, "a.txt", "mine\n");
    ok(&env, &["undo", "--shelve"]);
    let shelves = state(&env).shelves;
    assert_eq!(shelves.len(), 1);
    write(&env, "n.txt", "new\n");
    ok(&env, &["save", "Three"]);
    // Unsaved work where the step would write stops it, and --shelve sets it aside.
    write(&env, "b.txt", "edit\n");
    let to = start_of(goto, 8);
    let standing = env.standing();
    let (_, said) = refused(&env, &["undo", "--to", &to]);
    assert!(said.contains("b.txt"), "{said}");
    assert!(
        said.contains(&format!("bolt undo --to {to} --shelve")),
        "{said}"
    );
    assert_eq!(env.standing(), standing);
    let doc = json(&env, &["undo", "--to", &to, "--shelve"]);
    assert_eq!(doc["steps"][0]["action"], "jump", "{doc}");
    assert_eq!(doc["steps"][0]["entry"], goto.to_hex());
    assert_eq!(doc["shelved"][0]["label"], "2");
    assert_eq!(files(env.root()), holding(ONE));
    // The shelf there already stays, behind the one --shelve made.
    let now = state(&env).shelves;
    assert_eq!(now.len(), 2, "{now:?}");
    assert_eq!(now[0].0, "2");
    assert_eq!(now[1], shelves[0]);
    assert_eq!(saved_files(&env, now[0].1)["b.txt"], b"edit\n");
    // Undoing the jump writes the edit back, and takes the shelf the jump made off the list.
    let said = ok(&env, &["undo"]);
    let undid = format!("Undid undo --to {}:", goto.short_hex());
    assert!(said.contains(&undid), "{said}");
    assert!(said.contains("shelf 2 is gone"), "{said}");
    assert_eq!(fs::read(env.root().join("b.txt")).unwrap(), b"edit\n");
    assert_eq!(state(&env).shelves, shelves);
}

#[test]
fn undo_to_the_current_entry_changes_nothing() {
    let (env, _, goto) = after_a_goto();
    let standing = env.standing();
    let said = ok(&env, &["undo", "--to", &goto.to_hex()]);
    assert!(said.contains("Nothing changed"), "{said}");
    assert_eq!(env.standing(), standing);
    // After an undo, going to its own entry, or to the step undo would take next, changes nothing.
    ok(&env, &["undo"]);
    let standing = env.standing();
    let saved = entry(&env, goto).parent.unwrap();
    for to in [standing.0, saved] {
        let said = ok(&env, &["undo", "--to", &start_of(to, 8)]);
        assert!(said.contains("Nothing changed"), "{said}");
    }
    assert_eq!(env.standing(), standing);
}

#[test]
fn undo_to_needs_at_least_8_hex_digits_and_exactly_one_match() {
    let (env, _, goto) = after_a_goto();
    let standing = env.standing();
    let hex = goto.to_hex();
    let hex = hex.as_str();
    for text in [&hex[..7], "zzzzzzzz"] {
        let (_, said) = refused(&env, &["undo", "--to", text]);
        assert!(said.contains("at least 8 hex digits"), "{said}");
    }
    // Eight digits no entry starts with.
    let ids: Vec<String> = diary(&env).iter().map(Id::to_hex).collect();
    let none = (0..16)
        .map(|d| format!("{d:x}{}", &hex[1..8]))
        .find(|p| !ids.iter().any(|id| id.starts_with(p.as_str())))
        .unwrap();
    let (_, said) = refused(&env, &["undo", "--to", &none]);
    assert!(said.contains("No diary entry"), "{said}");
    // Either case will do.
    let upper = hex[..8].to_uppercase();
    assert!(ok(&env, &["undo", "--to", &upper]).contains("Nothing changed"));
    assert_eq!(env.standing(), standing);
    // --to and --list each go without N, and without the other.
    for args in [
        &["undo", "--to", hex, "2"][..],
        &["undo", "--list", "--to", hex],
        &["undo", "--list", "2"],
        &["undo", "--list", "--shelve"],
        &["undo", "--all"],
        &["redo", "--to", hex],
    ] {
        let out = env.bolt().args(args).output().unwrap();
        assert_eq!(out.status.code(), Some(2), "{args:?}");
    }
}

#[test]
fn undo_list_all_shows_every_entry_with_undos_and_redos() {
    let (env, _, goto) = after_a_goto();
    ok(&env, &["undo"]);
    ok(&env, &["redo"]);
    let entries = diary(&env);
    let said = ok(&env, &["undo", "--list", "--all"]);
    let lines: Vec<&str> = said.lines().collect();
    assert_eq!(lines.len(), 6, "{said}");
    for (line, id) in lines.iter().zip(&entries) {
        assert!(line.starts_with(&id.short_hex()), "{said}");
    }
    let g = goto.short_hex();
    assert!(
        lines[0].contains(&format!("  redo of {g}: at save ")),
        "{said}"
    );
    assert!(
        lines[1].contains(&format!("  undo of {g}: on main at ")),
        "{said}"
    );
    assert!(lines[2].contains("  goto: at save "), "{said}");
    for line in &lines[..3] {
        assert!(line.ends_with(", 3 files changed"), "{said}");
    }
    assert!(lines[3].contains("  save: main moved to "), "{said}");
    assert!(lines[3].ends_with("\"Two\""), "{said}");
    assert!(
        lines[5].ends_with("  start: the diary's first entry, which can't be undone"),
        "{said}"
    );
    // The list itself holds only the steps.
    let listed = ok(&env, &["undo", "--list"]);
    assert_eq!(listed.lines().count(), 4, "{listed}");
}

#[test]
fn undo_list_json_has_both_lists() {
    let env = project(&[("a.txt", "a\n")]);
    let mut steps = diary(&env);
    for name in ["b", "c"] {
        write(&env, &format!("{name}.txt"), "x\n");
        ok(&env, &["save", &format!("Add {name}")]);
        steps.insert(0, head(&env).0);
    }
    ok(&env, &["undo"]);
    let doc = json(&env, &["undo", "--list"]);
    assert_eq!(doc["bolt_json"], 1);
    assert_eq!(doc["more"], false);
    assert_eq!(doc["warnings"], serde_json::json!([]));
    let listed = |list: &serde_json::Value| -> Vec<String> {
        let list = list.as_array().unwrap();
        list.iter()
            .map(|e| e["entry"].as_str().unwrap().to_owned())
            .collect()
    };
    let hex = |ids: &[Id]| -> Vec<String> { ids.iter().map(Id::to_hex).collect() };
    // Redo's next first, and undo's newest first.
    assert_eq!(listed(&doc["redo"]), hex(&steps[..1]));
    assert_eq!(listed(&doc["undo"]), hex(&steps[1..]));
    let redo = &doc["redo"][0];
    assert_eq!(redo["action"], "command");
    assert_eq!(redo["target"], serde_json::Value::Null);
    assert_eq!(redo["command"], "save");
    assert_eq!(redo["first"], false);
    assert!(redo["time_ms"].as_i64().unwrap() > 0);
    let line = &redo["effects"][0];
    assert_eq!(line["kind"], "line");
    assert_eq!(line["name"], "main");
    let made = entry(&env, steps[0]).state.lines["main"];
    assert_eq!(line["to"], made.to_hex());
    let first = &doc["undo"][2];
    assert_eq!(first["first"], true);
    assert_eq!(first["effects"], serde_json::json!([]));
    // --all lists every entry, the undo included.
    let doc = json(&env, &["undo", "--list", "--all"]);
    let all = doc["entries"].as_array().unwrap();
    assert_eq!(all.len(), 5, "{doc}");
    assert_eq!(all[0]["action"], "undo");
    assert_eq!(all[0]["target"], steps[0].to_hex());
    assert_eq!(listed(&doc["entries"])[1..], hex(&steps)[..]);
    assert_eq!(doc["warnings"], serde_json::json!([]));
}

#[test]
fn undo_to_that_stops_on_the_way_says_how_far_it_got() {
    let (env, _, goto) = after_a_goto();
    let saved = entry(&env, goto).parent.unwrap();
    write(&env, "n.txt", "new\n");
    ok(&env, &["discard"]);
    write(&env, "a.txt", "mine\n");
    // The discard is undone; the goto before it would write over a.txt.
    let to = start_of(saved, 8);
    let (out, said) = refused(&env, &["undo", "--to", &to]);
    assert!(out.contains("Undid discard"), "{out}");
    let how_far = format!("Undid 1 step on the way to entry {to} before one stopped.");
    assert!(out.contains(&how_far), "{out}");
    assert!(said.contains("a.txt"), "{said}");
    assert!(
        said.contains(&format!("bolt undo --to {to} --shelve")),
        "{said}"
    );
    assert_eq!(fs::read(env.root().join("a.txt")).unwrap(), b"mine\n");
    assert_eq!(fs::read(env.root().join("n.txt")).unwrap(), b"new\n");
}

#[cfg(feature = "fault-injection")]
#[test]
fn undo_to_after_a_stop_finishes_it_and_takes_no_step() {
    let (env, one, goto) = after_a_goto();
    let files_step = diary(&env)[2];
    let out = env
        .bolt()
        .arg("undo")
        .env("BOLT_FAULT_FAIL", "switch:changed")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1), "{}", text(&out.stdout));
    assert!(journal_exists(&env));
    let to = start_of(files_step, 8);
    let said = ok(&env, &["undo", "--to", &to]);
    assert!(said.contains("Finished the undo of goto"), "{said}");
    let again = format!("Running bolt undo --to {to} again goes on from there.");
    assert!(said.contains(&again), "{said}");
    assert_eq!(head(&env).1.action, Action::Undo(goto));
    assert_eq!(files(env.root()), holding(TWO));
    assert!(!journal_exists(&env));
    // Run again, it goes the rest of the way.
    assert!(ok(&env, &["undo", "--to", &to]).contains("Undid save"));
    assert_eq!(main_tip(&env), one);
}

#[test]
fn the_lists_escape_what_the_diary_holds() {
    let env = project(&[("a.txt", "a\n")]);
    let (command, label) = ("save\u{1b}[2J", "1\u{1b}]0;x\u{7}");
    {
        let mut repo = Repository::open(env.root()).unwrap();
        let lock = repo.lock_for_change("test", &mut |_| {}).unwrap();
        let (head, mut planted) = repo.current().unwrap();
        planted.parent = Some(head);
        planted.command = command.into();
        let save = planted.state.lines["main"];
        planted.state.shelves.insert(0, (label.into(), save));
        let packs = repo.new_packs().unwrap();
        repo.commit(&lock, packs, &planted).unwrap();
    }
    for args in [&["undo", "--list"][..], &["undo", "--list", "--all"]] {
        let said = ok(&env, args);
        assert!(!said.contains(['\u{1b}', '\u{7}']), "{said:?}");
        assert!(said.contains(&escape(command, false)), "{said}");
        let shelf = format!("shelf {} added", escape(label, false));
        assert!(said.contains(&shelf), "{said}");
    }
}
