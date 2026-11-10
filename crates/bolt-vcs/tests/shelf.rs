//! `bolt shelf`, `bolt shelf --list` and `bolt unshelf`, end to end.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;
use std::path::Path;

use bolt_core::diary::{Action, Position, State};
use bolt_core::id::{Id, hash_content};
use bolt_core::journal::{self, Change, Expected, Journal};
use bolt_core::listing::EntryKind;
use bolt_core::repo::Repository;

mod support;
use support::{Env, Files, changed, files, head, holding, main_tip, saved_files, text, write};

/// `files` with `changes` made: a path set to what it holds, or removed with `None`.
fn with(files: &Files, changes: &[(&str, Option<&str>)]) -> Files {
    let mut changed = files.clone();
    for (path, bytes) in changes {
        match bytes {
            Some(b) => changed.insert((*path).to_owned(), b.as_bytes().to_vec()),
            None => changed.remove(*path),
        };
    }
    changed
}

/// A project with `entries` saved on `main`.
fn project(entries: &[(&str, &str)]) -> Env {
    let env = Env::empty();
    env.bolt()
        .args(["start", "--preset", "custom"])
        .assert()
        .success();
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

fn state(env: &Env) -> State {
    env.state().1
}

/// The shelves' numbers, newest first.
fn labels(env: &Env) -> Vec<String> {
    state(env).shelves.into_iter().map(|(l, _)| l).collect()
}

#[cfg(feature = "fault-injection")]
fn journal_exists(env: &Env) -> bool {
    env.root().join(".bolt/state/journal").exists()
}

const SAVED: &[(&str, &str)] = &[
    (".gitignore", "*.log\n"),
    ("a.txt", "a\n"),
    ("b.txt", "b\n"),
    ("dir/c.txt", "c\n"),
    ("gone/d.txt", "d\n"),
];

/// A project holding `SAVED`, then a changed, a new and a deleted file, a new folder and a deleted
/// one. Gives the folder as it then is, and what status found changed.
fn with_unsaved_work() -> (Env, Files, Vec<String>) {
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
fn shelf_sets_every_unsaved_change_aside_and_cleans_the_folder() {
    let (env, before, _) = with_unsaved_work();
    write(&env, "build.log", "log\n");
    let said = ok(&env, &["shelf"]);
    assert_eq!(
        said,
        "Set your unsaved changes in 5 files aside on shelf 1: bolt unshelf brings them back.\n"
    );
    // The folder is the save again, but for what Bolt ignores.
    let clean = with(&holding(SAVED), &[("build.log", Some("log\n"))]);
    assert_eq!(files(env.root()), clean);
    assert!(!env.root().join("newdir").exists());
    assert!(changed(&env).is_empty());
    // The shelf is a save on the current one, holding the folder as it was.
    let shelves = state(&env).shelves;
    assert_eq!(shelves.len(), 1);
    let (label, shelf) = shelves[0].clone();
    assert_eq!(label, "1");
    let repo = Repository::open(env.root()).unwrap();
    let kept = repo.store().save(&shelf, false).unwrap();
    assert_eq!(kept.parents, [main_tip(&env)]);
    assert_eq!(kept.message, "Shelved");
    assert_eq!(kept.line(), Some("main"));
    assert_eq!(saved_files(&env, shelf), before);
    // One step, which switched the folder from the shelf to the save.
    let (_, newest) = head(&env);
    assert_eq!(
        (newest.action, newest.command.as_str()),
        (Action::Command, "shelf")
    );
    let change = newest.folder_change.unwrap();
    assert_eq!((change.from, change.to), (shelf, main_tip(&env)));
}

#[test]
fn something_in_the_way_of_a_saved_file_stops_shelf() {
    let env = project(SAVED);
    write(&env, ".boltignore", "dir\n");
    ok(&env, &["save", "Ignore dir"]);
    // The saved dir/c.txt would go back where a file Bolt ignores now is.
    fs::remove_dir_all(env.root().join("dir")).unwrap();
    write(&env, "dir", "ignored\n");
    write(&env, "a.txt", "mine\n");
    let before = (env.standing(), files(env.root()));
    let (_, said) = refused(&env, &["shelf"]);
    assert!(
        said.contains("in the way") && said.contains("dir"),
        "{said}"
    );
    assert!(said.contains("then run bolt shelf again"), "{said}");
    assert_eq!((env.standing(), files(env.root())), before);
    assert!(labels(&env).is_empty());
}

#[test]
fn unshelf_brings_the_newest_back_and_drops_it() {
    let (env, before, changes) = with_unsaved_work();
    ok(&env, &["shelf", "First"]);
    write(&env, "b.txt", "other\n");
    ok(&env, &["shelf", "Second"]);
    assert_eq!(labels(&env), ["2", "1"]);
    let first = state(&env).shelves[1].clone();
    let said = ok(&env, &["unshelf"]);
    assert_eq!(said, "Brought back shelf 2 \"Second\": 1 file written.\n");
    assert_eq!(
        files(env.root()),
        with(&holding(SAVED), &[("b.txt", Some("other\n"))])
    );
    assert_eq!(state(&env).shelves, [first]);
    assert_eq!(head(&env).1.command, "unshelf");
    // The other comes back where the folder is the save again.
    ok(&env, &["discard"]);
    ok(&env, &["unshelf"]);
    assert_eq!(files(env.root()), before);
    assert_eq!(changed(&env), changes);
    assert!(labels(&env).is_empty());
}

#[test]
fn unshelf_by_number() {
    let env = project(SAVED);
    write(&env, "a.txt", "mine\n");
    ok(&env, &["shelf", "Change a"]);
    write(&env, "b.txt", "mine\n");
    ok(&env, &["shelf", "Change b"]);
    let doc = json(&env, &["unshelf", "1"]);
    assert_eq!(doc["bolt_json"], 1);
    assert_eq!(doc["finished"], false);
    assert_eq!(doc["shelf"]["label"], "1", "{doc}");
    assert_eq!(
        (doc["written"].as_u64(), doc["removed"].as_u64()),
        (Some(1), Some(0))
    );
    assert_eq!(doc["carried"], 0);
    assert_eq!(doc["warnings"], serde_json::json!([]));
    assert_eq!(changed(&env), ["a.txt"]);
    assert_eq!(labels(&env), ["2"]);
    // A number no shelf has changes nothing.
    let standing = env.standing();
    let (_, said) = refused(&env, &["unshelf", "1"]);
    assert!(said.contains("There's no shelf 1."), "{said}");
    assert!(said.contains("bolt shelf --list"), "{said}");
    assert_eq!(env.standing(), standing);
}

#[test]
fn numbers_take_the_smallest_free() {
    let env = project(SAVED);
    for (path, label) in [("a.txt", "1"), ("b.txt", "2")] {
        write(&env, path, "mine\n");
        let doc = json(&env, &["shelf"]);
        assert_eq!(doc["shelf"]["label"], label, "{doc}");
        assert_eq!(doc["shelf"]["files"], 1, "{doc}");
    }
    ok(&env, &["unshelf", "1"]);
    ok(&env, &["discard"]);
    write(&env, "dir/c.txt", "mine\n");
    assert!(ok(&env, &["shelf"]).contains("on shelf 1"));
    // The new one is the newest.
    assert_eq!(labels(&env), ["1", "2"]);
}

#[test]
fn unshelf_onto_another_save_applies_where_nothing_collides() {
    let env = project(SAVED);
    write(&env, "a.txt", "mine\n");
    write(&env, "n.txt", "new\n");
    ok(&env, &["shelf"]);
    write(&env, "b.txt", "B\n");
    ok(&env, &["save", "Change b"]);
    let saved = files(env.root());
    // Work of its own stays as it is.
    write(&env, "dir/c.txt", "mine\n");
    let doc = json(&env, &["unshelf"]);
    assert_eq!(doc["shelf"]["label"], "1", "{doc}");
    assert_eq!(doc["written"], 2, "{doc}");
    assert_eq!(doc["carried"], 1, "{doc}");
    let wanted = with(
        &saved,
        &[
            ("a.txt", Some("mine\n")),
            ("n.txt", Some("new\n")),
            ("dir/c.txt", Some("mine\n")),
        ],
    );
    assert_eq!(files(env.root()), wanted);
    assert_eq!(changed(&env), ["a.txt", "dir/c.txt", "n.txt"]);
    assert!(labels(&env).is_empty());
    ok(&env, &["undo"]);
    assert_eq!(
        files(env.root()),
        with(&saved, &[("dir/c.txt", Some("mine\n"))])
    );
    assert_eq!(labels(&env), ["1"]);
    assert!(ok(&env, &["redo"]).contains("Redid unshelf"));
    assert_eq!(files(env.root()), wanted);
}

/// The text of `bolt goto` to the save `save`, as a hint names it.
fn goto_hint(save: Id) -> String {
    format!("bolt goto {}", &save.to_hex()[..12])
}

#[test]
fn unshelf_onto_a_save_that_changed_a_shelved_file_stops() {
    let env = project(SAVED);
    let shelved_on = main_tip(&env);
    write(&env, "a.txt", "mine\n");
    ok(&env, &["shelf"]);
    write(&env, "a.txt", "theirs\n");
    ok(&env, &["save", "Change a"]);
    let before = (env.standing(), files(env.root()), state(&env).shelves);
    let (_, said) = refused(&env, &["unshelf"]);
    assert!(
        said.contains("changed what is there since the shelf was made:\n  a.txt"),
        "{said}"
    );
    assert!(!said.contains("Your unsaved changes"), "{said}");
    // Saving, shelving or discarding wouldn't clear the way: going to that save does.
    assert!(!said.contains("Save, shelve or discard"), "{said}");
    let goto = goto_hint(shelved_on);
    let way = format!("{goto}, then run bolt unshelf 1 there");
    assert!(said.contains(&way), "{said}");
    assert_eq!(
        (env.standing(), files(env.root()), state(&env).shelves),
        before
    );
    let words: Vec<&str> = goto.split(' ').collect();
    ok(&env, &words[1..]);
    assert_eq!(state(&env).position, Position::AtSave(shelved_on));
    ok(&env, &["unshelf", "1"]);
    assert_eq!(
        files(env.root()),
        with(&holding(SAVED), &[("a.txt", Some("mine\n"))])
    );
    assert!(labels(&env).is_empty());
}

#[test]
fn unsaved_work_in_the_way_stops_unshelf() {
    let env = project(SAVED);
    write(&env, "a.txt", "mine\n");
    ok(&env, &["shelf"]);
    write(&env, "a.txt", "again\n");
    let before = (env.standing(), files(env.root()), state(&env).shelves);
    let out = env.bolt().args(["unshelf", "--json"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    let doc: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let message = doc["error"]["message"].as_str().unwrap();
    assert!(
        message.contains("files Bolt ignores, are where it would write:\n  a.txt"),
        "{doc}"
    );
    assert!(!message.contains("since the shelf was made"), "{doc}");
    let hint = doc["error"]["hint"].as_str().unwrap();
    assert!(hint.contains("Save, shelve or discard"), "{doc}");
    assert!(hint.contains("bolt unshelf 1"), "{doc}");
    assert!(!hint.contains("bolt goto"), "{doc}");
    assert_eq!(
        (env.standing(), files(env.root()), state(&env).shelves),
        before
    );
}

#[test]
fn unshelf_tells_unsaved_work_from_what_the_save_changed() {
    let env = project(SAVED);
    let shelved_on = main_tip(&env);
    write(&env, "a.txt", "mine\n");
    write(&env, "b.txt", "mine\n");
    ok(&env, &["shelf"]);
    write(&env, "a.txt", "theirs\n");
    ok(&env, &["save", "Change a"]);
    write(&env, "b.txt", "again\n");
    let (_, said) = refused(&env, &["unshelf"]);
    assert!(
        said.contains("files Bolt ignores, are where it would write:\n  b.txt\n"),
        "{said}"
    );
    assert!(
        said.contains("changed what is there since the shelf was made:\n  a.txt\n"),
        "{said}"
    );
    assert!(
        said.contains("Save, shelve or discard your unsaved changes"),
        "{said}"
    );
    let goto = goto_hint(shelved_on);
    assert!(
        said.contains(&format!("{goto}, then run bolt unshelf 1 there")),
        "{said}"
    );
    // As the hint says: the work set aside, then the shelf brought back on its own save.
    ok(&env, &["shelf"]);
    let words: Vec<&str> = goto.split(' ').collect();
    ok(&env, &words[1..]);
    ok(&env, &["unshelf", "1"]);
    let both = with(
        &holding(SAVED),
        &[("a.txt", Some("mine\n")), ("b.txt", Some("mine\n"))],
    );
    assert_eq!(files(env.root()), both);
    assert_eq!(labels(&env), ["2"]);
}

#[test]
fn a_folder_the_save_made_a_file_counts_as_changed_by_the_save() {
    let env = project(SAVED);
    let shelved_on = main_tip(&env);
    write(&env, "dir/new.txt", "new\n");
    ok(&env, &["shelf"]);
    // The save it comes back onto has a file where the shelf's folder is.
    fs::remove_dir_all(env.root().join("dir")).unwrap();
    write(&env, "dir", "a file\n");
    ok(&env, &["save", "dir is a file"]);
    assert!(changed(&env).is_empty());
    let (_, said) = refused(&env, &["unshelf"]);
    assert!(
        said.contains("changed what is there since the shelf was made:\n  dir/new.txt"),
        "{said}"
    );
    assert!(!said.contains("Your unsaved changes"), "{said}");
    let goto = goto_hint(shelved_on);
    assert!(
        said.contains(&format!("{goto}, then run bolt unshelf 1 there")),
        "{said}"
    );
    let words: Vec<&str> = goto.split(' ').collect();
    ok(&env, &words[1..]);
    ok(&env, &["unshelf", "1"]);
    assert_eq!(
        files(env.root()),
        with(&holding(SAVED), &[("dir/new.txt", Some("new\n"))])
    );
}

#[test]
fn work_of_your_own_below_a_folder_the_save_removed_or_made_is_unsaved_work() {
    let env = project(SAVED);
    write(&env, "dir/new.txt", "shelved\n");
    write(&env, "fresh/new.txt", "shelved\n");
    ok(&env, &["shelf"]);
    // The save it comes back onto removed `dir`, and made `fresh` with a file of its own.
    fs::remove_dir_all(env.root().join("dir")).unwrap();
    write(&env, "fresh/other.txt", "theirs\n");
    ok(&env, &["save", "Remove dir, make fresh"]);
    write(&env, "dir/new.txt", "mine\n");
    write(&env, "fresh/new.txt", "mine\n");
    let (_, said) = refused(&env, &["unshelf"]);
    assert!(
        said.contains("are where it would write:\n  dir/new.txt\n  fresh/new.txt\n"),
        "{said}"
    );
    assert!(!said.contains("since the shelf was made"), "{said}");
}

#[test]
fn a_file_the_save_changed_where_the_shelf_made_a_folder_counts_as_changed_by_the_save() {
    let env = project(SAVED);
    fs::remove_file(env.root().join("b.txt")).unwrap();
    write(&env, "b.txt/inside.txt", "shelved\n");
    ok(&env, &["shelf"]);
    write(&env, "b.txt", "theirs\n");
    ok(&env, &["save", "Change b"]);
    let (_, said) = refused(&env, &["unshelf"]);
    assert!(
        said.contains(
            "changed what is there since the shelf was made:\n  b.txt\n  b.txt/inside.txt\n"
        ),
        "{said}"
    );
    assert!(!said.contains("Your unsaved changes"), "{said}");
}

#[test]
fn a_save_no_line_reaches_any_more_is_not_offered_to_goto() {
    let env = project(SAVED);
    write(&env, "b.txt", "B\n");
    ok(&env, &["save", "Change b"]);
    let changed_b = head(&env).0;
    ok(&env, &["undo"]);
    write(&env, "c.txt", "C\n");
    ok(&env, &["save", "Change b and add c"]);
    let left_behind = main_tip(&env);
    // Going back to the save of b alone takes c.txt away: the edit there goes on a shelf made
    // on a save the jump leaves no line on.
    write(&env, "c.txt", "mine\n");
    ok(&env, &["undo", "--to", &changed_b.to_hex(), "--shelve"]);
    assert_eq!(labels(&env), ["1"]);
    let goto = goto_hint(left_behind);
    let words: Vec<&str> = goto.split(' ').collect();
    refused(&env, &words[1..]);
    let (_, said) = refused(&env, &["unshelf"]);
    assert!(
        said.contains("changed what is there since the shelf was made:\n  c.txt"),
        "{said}"
    );
    assert!(
        said.contains("bolt goto can't reach the save it was made on"),
        "{said}"
    );
    assert!(said.contains("bolt undo --list"), "{said}");
    assert!(!said.contains(&goto), "{said}");
}

#[test]
fn a_change_a_finished_switch_met_is_not_told_as_one_the_shelf_met() {
    let env = project(SAVED);
    // A stopped switch whose journal would remove a file Bolt ignores, which changed since.
    write(&env, "build.log", "changed\n");
    {
        let mut repo = Repository::open(env.root()).unwrap();
        let lock = repo.lock_for_change("test", &mut |_| {}).unwrap();
        let (head, mut entry) = repo.current().unwrap();
        entry.parent = Some(head);
        let packs = repo.new_packs().unwrap();
        let target = repo.store_entry(&lock, packs, &entry).unwrap();
        let planned = b"planned\n";
        let remove = journal::Action {
            path: "build.log".into(),
            change: Change::RemoveFile,
            expected: Some(Expected {
                kind: EntryKind::File,
                id: hash_content(planned),
                size: planned.len() as u64,
            }),
        };
        let journal = Journal {
            target,
            from: head,
            actions: vec![remove],
        };
        journal.write(&repo, &lock).unwrap();
    }
    let out = env.bolt().arg("shelf").output().unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    assert_eq!(
        text(&out.stdout),
        "Nothing to shelve: your files match your last save.\n"
    );
    let warned = text(&out.stderr);
    assert!(
        warned.contains("changed while Bolt was switching"),
        "{warned}"
    );
    assert_eq!(
        fs::read(env.root().join("build.log")).unwrap(),
        b"changed\n"
    );
}

#[test]
fn shelf_with_nothing_unsaved_says_so() {
    let env = project(SAVED);
    let standing = env.standing();
    let said = ok(&env, &["shelf"]);
    assert_eq!(
        said,
        "Nothing to shelve: your files match your last save.\n"
    );
    let doc = json(&env, &["shelf"]);
    assert_eq!(doc["shelf"], serde_json::Value::Null, "{doc}");
    assert_eq!(doc["warnings"], serde_json::json!([]));
    assert_eq!(env.standing(), standing);
    // And with no shelf, there is nothing to bring back.
    let (_, said) = refused(&env, &["unshelf"]);
    assert!(said.contains("There's no shelf to bring back."), "{said}");
    assert_eq!(env.standing(), standing);
}

#[test]
fn undo_of_shelf_brings_the_changes_back_and_drops_the_shelf() {
    let (env, before, changes) = with_unsaved_work();
    ok(&env, &["shelf"]);
    let shelves = state(&env).shelves;
    let said = ok(&env, &["undo"]);
    assert!(said.contains("Undid shelf: shelf 1 is gone"), "{said}");
    assert_eq!(files(env.root()), before);
    assert!(env.root().join("newdir").is_dir());
    assert!(!env.root().join("gone").exists());
    assert_eq!(changed(&env), changes);
    assert!(labels(&env).is_empty());
    // Redo sets them aside again, on the same shelf.
    ok(&env, &["redo"]);
    assert_eq!(files(env.root()), holding(SAVED));
    assert_eq!(state(&env).shelves, shelves);
}

#[test]
fn undo_of_unshelf_takes_them_out_and_puts_the_shelf_back() {
    let (env, before, _) = with_unsaved_work();
    ok(&env, &["shelf", "Work"]);
    let shelves = state(&env).shelves;
    ok(&env, &["unshelf"]);
    assert_eq!(files(env.root()), before);
    let said = ok(&env, &["undo"]);
    assert!(said.contains("Undid unshelf: shelf 1 is back"), "{said}");
    assert_eq!(files(env.root()), holding(SAVED));
    assert!(!env.root().join("newdir").exists());
    assert!(changed(&env).is_empty());
    assert_eq!(state(&env).shelves, shelves);
}

#[test]
fn undo_of_an_unshelf_onto_another_save_touches_only_what_it_wrote() {
    let env = project(SAVED);
    write(&env, "a.txt", "mine\n");
    write(&env, "newdir/x.txt", "x\n");
    ok(&env, &["shelf"]);
    // The save it comes back onto already holds its a.txt, and a file in the folder it adds.
    write(&env, "a.txt", "mine\n");
    write(&env, "newdir/saved.txt", "s\n");
    ok(&env, &["save", "Other"]);
    let other = files(env.root());
    ok(&env, &["unshelf"]);
    assert_eq!(
        files(env.root()),
        with(&other, &[("newdir/x.txt", Some("x\n"))])
    );
    assert_eq!(changed(&env), ["newdir/x.txt"]);
    ok(&env, &["undo"]);
    assert_eq!(files(env.root()), other);
    assert!(changed(&env).is_empty());
    assert_eq!(labels(&env), ["1"]);
}

#[cfg(feature = "fault-injection")]
#[test]
fn running_unshelf_again_after_a_stop_finishes_it_and_brings_back_no_other_shelf() {
    let env = project(SAVED);
    write(&env, "a.txt", "mine\n");
    ok(&env, &["shelf", "First"]);
    write(&env, "b.txt", "mine\n");
    write(&env, "n.txt", "new\n");
    ok(&env, &["shelf", "Second"]);
    // `bolt <args>`, stopped after its first write.
    let stopped = |env: &Env, args: &[&str]| {
        let out = env
            .bolt()
            .args(args)
            .env("BOLT_FAULT_FAIL", "switch:changed")
            .output()
            .unwrap();
        assert_eq!(
            out.status.code(),
            Some(1),
            "{args:?}: {}",
            text(&out.stdout)
        );
        assert!(journal_exists(env));
    };
    // `bolt <args>`, which finishes the stop and brings no shelf back.
    let finishes = |env: &Env, args: &[&str]| -> String {
        let said = ok(env, args);
        let done = " that was stopped, so no shelf was brought back this time.";
        assert!(
            said.starts_with("Finished the ") && said.contains(done),
            "{said}"
        );
        assert!(!journal_exists(env));
        said
    };
    stopped(&env, &["unshelf"]);
    let said = finishes(&env, &["unshelf"]);
    assert!(said.starts_with("Finished the unshelf"), "{said}");
    let again = "Running bolt unshelf again brings back the newest shelf left.";
    assert!(said.contains(again), "{said}");
    // The newest is back whole, and the other is still on the list.
    let second = with(
        &holding(SAVED),
        &[("b.txt", Some("mine\n")), ("n.txt", Some("new\n"))],
    );
    assert_eq!(files(env.root()), second);
    assert_eq!(labels(&env), ["1"]);
    // Named, the shelf the stopped switch was bringing back is off the list.
    stopped(&env, &["unshelf", "1"]);
    let said = finishes(&env, &["unshelf", "1"]);
    assert!(
        said.contains("There's no shelf 1 on the list now"),
        "{said}"
    );
    assert_eq!(
        files(env.root()),
        with(&second, &[("a.txt", Some("mine\n"))])
    );
    assert!(labels(&env).is_empty());
    // Another command's stop, finished with no shelf left, then with one named that is there.
    stopped(&env, &["discard"]);
    let said = finishes(&env, &["unshelf"]);
    assert!(said.starts_with("Finished the discard"), "{said}");
    assert!(said.contains("There are no shelves left."), "{said}");
    write(&env, "a.txt", "third\n");
    ok(&env, &["shelf"]);
    write(&env, "b.txt", "fourth\n");
    stopped(&env, &["discard"]);
    let said = finishes(&env, &["unshelf", "1"]);
    assert!(
        said.contains("Running bolt unshelf 1 again brings it back."),
        "{said}"
    );
    // And told in JSON.
    write(&env, "b.txt", "fifth\n");
    stopped(&env, &["discard"]);
    let doc = json(&env, &["unshelf"]);
    assert_eq!(doc["finished"], true, "{doc}");
    assert_eq!(doc["shelf"], serde_json::Value::Null, "{doc}");
    assert_eq!(labels(&env), ["1"]);
}

#[test]
fn undo_redo_and_jumps_never_drop_a_shelf() {
    let env = project(SAVED);
    write(&env, "a.txt", "mine\n");
    ok(&env, &["shelf"]);
    let shelves = state(&env).shelves;
    write(&env, "b.txt", "B\n");
    ok(&env, &["save", "Change b"]);
    let step = head(&env).0;
    for args in [&["undo"][..], &["redo"], &["undo"]] {
        ok(&env, args);
        assert_eq!(state(&env).shelves, shelves, "{args:?}");
    }
    // A new step clears the save from what redo can bring back, so going to it is a jump.
    write(&env, "c.txt", "C\n");
    ok(&env, &["save", "Change b and c"]);
    ok(&env, &["undo", "--to", &step.to_hex()]);
    let (_, jump) = head(&env);
    assert_eq!(
        (jump.action, jump.command.as_str()),
        (Action::Command, "undo")
    );
    assert_eq!(state(&env).shelves, shelves);
    for args in [&["undo"][..], &["redo"]] {
        ok(&env, args);
        assert_eq!(state(&env).shelves, shelves, "{args:?}");
    }
}

#[test]
fn shelf_list_shows_number_message_line_time_and_files() {
    let env = project(SAVED);
    assert_eq!(
        ok(&env, &["shelf", "--list"]),
        "No shelves: bolt shelf sets your unsaved changes aside on one.\n"
    );
    write(&env, "a.txt", "mine\n");
    write(&env, "n.txt", "new\n");
    ok(&env, &["shelf", "Work on login\nwith details"]);
    // One made at a save, not on a line.
    ok(&env, &["goto", &main_tip(&env).to_hex()]);
    write(&env, "b.txt", "mine\n");
    ok(&env, &["shelf"]);
    let said = ok(&env, &["shelf", "--list"]);
    assert_eq!(
        said,
        "2  just now  at a save  1 file  \"Shelved\"\n\
         1  just now  on main  2 files  \"Work on login\"\n"
    );
    let doc = json(&env, &["shelf", "--list"]);
    assert_eq!(doc["bolt_json"], 1);
    assert_eq!(doc["warnings"], serde_json::json!([]));
    let listed = doc["shelves"].as_array().unwrap();
    assert_eq!(listed.len(), 2, "{doc}");
    let shelves = state(&env).shelves;
    let first = &listed[1];
    assert_eq!(first["label"], "1");
    assert_eq!(first["save"], shelves[1].1.to_hex());
    assert_eq!(first["message"], "Work on login\nwith details");
    assert_eq!(first["line"], "main");
    assert_eq!(first["files"], 2);
    assert_eq!(first["saver"]["name"], "Test");
    assert!(first["saver"]["time_ms"].as_i64().unwrap() > 0);
    assert_eq!(listed[0]["line"], serde_json::Value::Null);
    // --list takes no message.
    let out = env.bolt().args(["shelf", "--list", "x"]).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
}

#[test]
fn gotos_hint_names_shelf() {
    let env = project(SAVED);
    let one = main_tip(&env);
    write(&env, "a.txt", "two\n");
    ok(&env, &["save", "Two"]);
    write(&env, "a.txt", "mine\n");
    let (_, said) = refused(&env, &["goto", &one.to_hex()]);
    assert!(said.contains("shelve them (bolt shelf)"), "{said}");
    // Shelving them clears the way.
    ok(&env, &["shelf"]);
    ok(&env, &["goto", &one.to_hex()]);
    assert_eq!(state(&env).position, Position::AtSave(one));
}

/// Makes `link` lead to the folder `target`: a link, or on Windows a junction, which needs no
/// rights and which Bolt reads as a link too.
fn folder_link(target: &Path, link: &Path) {
    #[cfg(unix)]
    std::os::unix::fs::symlink(target, link).unwrap();
    #[cfg(windows)]
    support::junction(link, target);
}

#[test]
fn shelf_leaves_a_link_that_could_come_back_only_as_a_file() {
    let env = project(SAVED);
    let outside = tempfile::tempdir().unwrap();
    folder_link(outside.path(), &env.root().join("out"));
    let out = env.bolt().arg("shelf").output().unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    assert_eq!(text(&out.stdout), "Nothing was shelved.\n");
    let warned = text(&out.stderr);
    assert!(warned.contains("Didn't shelve 1 unsaved link"), "{warned}");
    assert!(warned.contains("  out\n"), "{warned}");
    assert!(labels(&env).is_empty());
    // Other work is shelved, and the link stays.
    write(&env, "a.txt", "mine\n");
    let doc = json(&env, &["shelf"]);
    assert_eq!(doc["shelf"]["files"], 1, "{doc}");
    let kinds: Vec<&str> = doc["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| w["kind"].as_str().unwrap())
        .collect();
    assert_eq!(kinds, ["links_left"], "{doc}");
    assert!(fs::read_link(env.root().join("out")).is_ok());
    assert_eq!(changed(&env), ["out"]);
}

/// Writes `size` bytes that don't compress at `path`.
#[cfg(target_os = "linux")]
fn write_noise(path: &Path, size: u64) {
    use std::io::Write;
    let mut file = fs::File::create(path).unwrap();
    let mut state = 0x9e37_79b9_7f4a_7c15_u64;
    let mut chunk = vec![0; 1 << 20];
    for _ in 0..(size >> 20) {
        for word in chunk.chunks_mut(8) {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            word.copy_from_slice(&state.to_le_bytes());
        }
        file.write_all(&chunk).unwrap();
    }
}

#[cfg(target_os = "linux")]
#[test]
fn shelving_a_big_file_uses_little_memory() {
    const SIZE: u64 = 300 << 20;
    let env = project(&[("a.txt", "a\n")]);
    let big = env.root().join("big");
    write_noise(&big, SIZE);
    // Set aside, then back.
    for (command, there) in [("shelf", false), ("unshelf", true)] {
        let mut cmd = env.bolt_process_in(env.root());
        cmd.arg(command);
        let ended = support::run_limited(cmd, 600);
        assert_eq!(ended.code, Some(0), "{command}: {ended:?}");
        assert!(
            ended.peak_mb < 100,
            "{command} of a 300 MiB file peaked at {} MB",
            ended.peak_mb
        );
        assert_eq!(big.exists(), there, "{command}");
    }
    assert_eq!(fs::metadata(&big).unwrap().len(), SIZE);
}

/// Whether the `--json` document `doc` has a warning of `kind`.
#[cfg(feature = "fault-injection")]
fn warned(doc: &serde_json::Value, kind: &str) -> bool {
    let warnings = doc["warnings"].as_array().unwrap();
    warnings.iter().any(|w| w["kind"] == kind)
}

/// Runs `bolt <command>` on copies of `template`, stopped at the first write, then the second,
/// and so on until it runs to the end. Every time, each file is wholly `before` or `after`, and
/// the next command that changes the project finishes it. `done` checks a copy where the step
/// was made, and `not_begun` one where it wasn't.
#[cfg(feature = "fault-injection")]
fn stopped_anywhere(
    template: &Env,
    command: &str,
    (before, after): (&Files, &Files),
    done: impl Fn(&Env, u32),
    not_begun: impl Fn(&Env, u32),
) {
    let at = head(template).0;
    let mut finished_later = 0;
    for n in 1.. {
        assert!(n < 300, "{command} never finished");
        let env = Env::empty();
        support::copy_dir(template.root(), env.root());
        let out = env
            .bolt()
            .arg(command)
            .env("BOLT_FAULT_AT", n.to_string())
            .output()
            .unwrap();
        let crashed = text(&out.stderr).contains("injected crash");
        for (path, bytes) in files(env.root()) {
            let known = [before, after].map(|f| f.get(&path) == Some(&bytes));
            assert!(known.contains(&true), "{command} {n}: {path} is neither");
        }
        let unfinished = warned(&json(&env, &["status"]), "switch_unfinished");
        // Stopped once its head moved, it left only its journal, which goes without a word.
        let only_journal = journal_exists(&env) && head(&env).1.command == command;
        // A command that changes nothing of its own finishes it.
        let next = json(&env, &["discard", "dir/c.txt"]);
        assert_eq!(
            next["error"],
            serde_json::Value::Null,
            "{command} {n}: {next}"
        );
        let finished = unfinished && !only_journal;
        assert_eq!(warned(&next, "switch_finished"), finished, "{command} {n}");
        assert!(!journal_exists(&env), "{command} {n}");
        let (now, newest) = head(&env);
        if newest.command == command {
            assert_eq!(files(env.root()), *after, "{command} {n}");
            done(&env, n);
        } else {
            assert_eq!(now, at, "{command} {n}");
            assert_eq!(files(env.root()), *before, "{command} {n}");
            not_begun(&env, n);
        }
        if finished {
            finished_later += 1;
        }
        if !crashed {
            assert!(n > 1, "no fault point was reached");
            assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
            assert_eq!(newest.command, command);
            assert!(finished_later >= 3, "only {finished_later} finished later");
            break;
        }
    }
}

#[cfg(feature = "fault-injection")]
#[test]
fn a_shelf_or_unshelf_stopped_at_every_write_is_finished_by_the_next_command() {
    let (template, edited, _) = with_unsaved_work();
    let clean = holding(SAVED);
    stopped_anywhere(
        &template,
        "shelf",
        (&edited, &clean),
        |env, n| {
            let shelves = state(env).shelves;
            assert_eq!(shelves.len(), 1, "shelf {n}");
            assert_eq!(saved_files(env, shelves[0].1), edited, "shelf {n}");
        },
        |env, n| assert!(labels(env).is_empty(), "shelf {n}"),
    );
    ok(&template, &["shelf"]);
    stopped_anywhere(
        &template,
        "unshelf",
        (&clean, &edited),
        |env, n| assert!(labels(env).is_empty(), "unshelf {n}"),
        |env, n| assert_eq!(labels(env), ["1"], "unshelf {n}"),
    );
}
