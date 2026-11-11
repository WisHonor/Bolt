//! A planted deep tree of shared folder listings: 24 listings each naming the next twice, 8 KiB
//! that hold 2^25 files by path. Every command that walks it refuses it at once, writing nothing.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use bolt_core::diary::{Action, DiaryEntry, FolderChange, State};
use bolt_core::id::Id;
use bolt_core::listing::{Entry, EntryKind, Listing};
use bolt_core::pack::{ObjectKind, PackSet};
use bolt_core::repo::Repository;
use support::{Env, files, main_tip, plant_save, run_limited};

/// The top listing of a save holding the deep tree in the folder `deep`.
fn deep_tree(pack: &mut PackSet) -> Id {
    let x = pack.add_content(b"x").unwrap();
    let two = |kind, id, size| {
        let entries = ["a", "b"]
            .iter()
            .map(|n| Entry {
                name: (*n).to_owned(),
                kind,
                id,
                size,
            })
            .collect();
        Listing::new(entries).unwrap().encode()
    };
    let mut at = pack
        .add_record(ObjectKind::FolderListing, &two(EntryKind::File, x, 1))
        .unwrap();
    for _ in 0..24 {
        at = pack
            .add_record(ObjectKind::FolderListing, &two(EntryKind::Folder, at, 0))
            .unwrap();
    }
    let top = Listing::new(vec![Entry {
        name: "deep".into(),
        kind: EntryKind::Folder,
        id: at,
        size: 0,
    }])
    .unwrap();
    pack.add_record(ObjectKind::FolderListing, &top.encode())
        .unwrap()
}

/// A started project on `main`, whose first save is the one given, with the deep tree saved on
/// `main` after it. The folder is still the first save's, so it lacks `deep`.
fn on_the_tree() -> (Env, Id, Id) {
    let env = Env::empty();
    env.bolt()
        .args(["start", "--preset", "custom"])
        .assert()
        .success();
    let first = main_tip(&env);
    plant_save(env.root(), deep_tree);
    let tree = main_tip(&env);
    (env, first, tree)
}

/// The same, with the project at its first save rather than on `main`.
fn beside_the_tree() -> Env {
    let env = Env::empty();
    env.bolt()
        .args(["start", "--preset", "custom"])
        .assert()
        .success();
    let first = main_tip(&env).to_hex();
    env.bolt().args(["goto", &first]).assert().success();
    plant_save(env.root(), deep_tree);
    env
}

/// Commits a diary entry after the current one, holding its state as `edit` leaves it; its ID.
fn plant_entry(
    env: &Env,
    command: &str,
    action: Action,
    folder_change: Option<FolderChange>,
    edit: impl FnOnce(&mut State),
) -> Id {
    let mut repo = Repository::open(env.root()).unwrap();
    let lock = repo.lock_for_change(command, &mut |_| {}).unwrap();
    let (head, entry) = repo.current().unwrap();
    let mut state = entry.state.clone();
    edit(&mut state);
    let diary = DiaryEntry {
        parent: Some(head),
        action,
        command: command.into(),
        who: entry.who,
        bolt_version: "0.1.0".into(),
        state,
        folder_change,
    };
    let packs = repo.new_packs().unwrap();
    repo.commit(&lock, packs, &diary).unwrap();
    repo.head().unwrap()
}

/// Fails unless `bolt <args>` refuses the deep tree within 10 seconds, writing nothing.
fn refused_at_once(env: &Env, args: &[&str]) {
    let (standing, folder) = (env.standing(), files(env.root()));
    let mut bolt = env.bolt_process_in(env.root());
    bolt.args(args);
    let ended = run_limited(bolt, 120);
    assert_eq!(ended.code, Some(1), "{args:?}: {ended:?}");
    assert!(
        ended
            .stderr
            .contains("more than 10000000 files and folders in total"),
        "{args:?}: {}",
        ended.stderr
    );
    assert!(ended.secs < 10.0, "{args:?} took {:.1} s", ended.secs);
    assert_eq!(env.standing(), standing, "{args:?}");
    assert_eq!(files(env.root()), folder, "{args:?}");
    assert!(!env.root().join(".bolt/state/journal").exists(), "{args:?}");
}

#[test]
fn reading_the_tree_is_refused_at_once() {
    let env = beside_the_tree();
    refused_at_once(&env, &["goto", "main"]);
    refused_at_once(&env, &["changes", "--save", "main"]);
    refused_at_once(&env, &["show", "main"]);
}

#[test]
fn a_folder_on_the_tree_without_it_is_refused_at_once() {
    let (env, ..) = on_the_tree();
    refused_at_once(&env, &["status"]);
    refused_at_once(&env, &["discard"]);
    refused_at_once(&env, &["shelf"]);
}

#[test]
fn a_shelf_of_the_tree_is_refused_at_once() {
    let (env, first, tree) = on_the_tree();
    // The tree's save, made on the first, set aside on a shelf, and `main` back at the first.
    plant_entry(&env, "shelf", Action::Command, None, |state| {
        state.lines.insert("main".into(), first);
        state.shelves.insert(0, ("1".into(), tree));
    });
    refused_at_once(&env, &["unshelf"]);
}

#[test]
fn undo_and_redo_across_the_tree_are_refused_at_once() {
    let (env, first, tree) = on_the_tree();
    // A step that took the folder from the first save to the tree's.
    let onto = FolderChange {
        from: first,
        to: tree,
    };
    let step = plant_entry(&env, "goto", Action::Command, Some(onto), |_| {});
    refused_at_once(&env, &["undo"]);
    // That step undone, back to the state before it, so redo would take it again.
    let back = FolderChange {
        from: tree,
        to: first,
    };
    plant_entry(&env, "undo", Action::Undo(step), Some(back), |_| {});
    refused_at_once(&env, &["redo"]);
}
