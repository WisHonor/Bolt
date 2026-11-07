#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use super::*;
use crate::snapshot::{SaveOutcome, SaveRequest, save_folder};
use crate::switch::discard;
use crate::test_support::symlink;
use std::collections::BTreeMap;
use std::fs;

fn who() -> Person {
    Person {
        name: "Ada".into(),
        email: "ada@example.com".into(),
        time_ms: 1_700_000_000_000,
        tz_offset_min: 0,
    }
}

fn one_step(who: &Person) -> StepRequest<'_> {
    StepRequest {
        n: 1,
        shelve: false,
        who,
        git_excludes: None,
    }
}

fn write_all(dir: &Path, files: &[(&str, &[u8])]) {
    for (path, bytes) in files {
        let at = dir.join(path);
        fs::create_dir_all(at.parent().unwrap()).unwrap();
        fs::write(at, bytes).unwrap();
    }
}

/// A project whose folder holds `files`, saved on `main`.
fn saved(files: &[(&str, &[u8])]) -> (tempfile::TempDir, Repository) {
    let dir = tempfile::tempdir().unwrap();
    let mut repo = Repository::init(dir.path(), &who()).unwrap();
    write_all(dir.path(), files);
    let who = who();
    let req = SaveRequest {
        message: "next",
        who: &who,
        command: "save",
        accept_crlf: false,
        git_excludes: None,
        new_line: None,
        only: None,
    };
    let saved = save_folder(&mut repo, &req, &mut |_| {}, &mut |_| true).unwrap();
    assert!(matches!(saved, SaveOutcome::Saved { .. }));
    (dir, repo)
}

/// Commits a step after the current entry that changed no file, its state the current one as
/// `change` leaves it.
fn plant(repo: &mut Repository, command: &str, change: impl FnOnce(&mut State)) -> Id {
    plant_with(repo, command, None, change)
}

/// [`plant`], with `folder` as the step's folder change.
fn plant_with(
    repo: &mut Repository,
    command: &str,
    folder: Option<FolderChange>,
    change: impl FnOnce(&mut State),
) -> Id {
    let lock = repo.lock_for_change("test", &mut |_| {}).unwrap();
    let (head, mut entry) = repo.current().unwrap();
    entry.parent = Some(head);
    entry.action = Action::Command;
    entry.command = command.into();
    entry.folder_change = folder;
    change(&mut entry.state);
    let packs = repo.new_packs().unwrap();
    repo.commit(&lock, packs, &entry).unwrap()
}

fn shelf(label: &str, save: Id) -> (String, Id) {
    (label.to_owned(), save)
}

#[test]
fn effects_name_what_changed() {
    let (a, b, c) = (Id([1; 32]), Id([2; 32]), Id([3; 32]));
    let before = State {
        lines: BTreeMap::from([("main".into(), a), ("old".into(), a)]),
        tags: None,
        position: Position::OnLine("main".into()),
        shelves: vec![shelf("1", a)],
        server_lines: Some(c),
    };
    let after = State {
        lines: BTreeMap::from([("main".into(), b), ("new".into(), b)]),
        tags: Some(c),
        position: Position::AtSave(b),
        shelves: vec![shelf("2", b)],
        server_lines: Some(c),
    };
    let line = |name: &str, from, to| Effect::Line {
        name: name.into(),
        from,
        to,
    };
    let found = effects(&before, &after, Some(FolderChange { from: a, to: b }));
    assert_eq!(
        found,
        [
            line("main", Some(a), Some(b)),
            line("new", None, Some(b)),
            line("old", Some(a), None),
            Effect::Position {
                from: before.position.clone(),
                to: after.position.clone(),
            },
            Effect::ShelfAdded {
                label: "2".into(),
                save: b,
            },
            Effect::ShelfGone {
                label: "1".into(),
                save: a,
            },
            Effect::Tags,
            Effect::Files { from: a, to: b },
        ]
    );
    assert!(effects(&before, &before, None).is_empty());
}

#[test]
fn a_shelf_a_step_added_or_removed_comes_off_or_back_by_its_save() {
    let (s, t, u) = (Id([1; 32]), Id([2; 32]), Id([3; 32]));
    // Undoing a step that added shelf 1 takes it off; a shelf of the user's own stays.
    let mut now = vec![shelf("2", t), shelf("1", s)];
    restore_shelves(&mut now, &[], &[shelf("1", s)], false);
    assert_eq!(now, [shelf("2", t)]);
    // Redone, it comes back first, as the newest, with its label.
    restore_shelves(&mut now, &[], &[shelf("1", s)], true);
    assert_eq!(now, [shelf("1", s), shelf("2", t)]);
    // Undoing a step that removed one puts it back; with its label taken, it gets the smallest
    // number free.
    let mut now = vec![shelf("1", u), shelf("3", t)];
    let (parent, step) = ([shelf("1", s), shelf("3", t)], [shelf("3", t)]);
    restore_shelves(&mut now, &parent, &step, false);
    assert_eq!(now, [shelf("2", s), shelf("1", u), shelf("3", t)]);
    // Redone, it goes again, whatever its label now.
    restore_shelves(&mut now, &parent, &step, true);
    assert_eq!(now, [shelf("1", u), shelf("3", t)]);
    // One already there isn't put back twice.
    let mut now = vec![shelf("1", s)];
    restore_shelves(&mut now, &[], &[shelf("1", s)], true);
    assert_eq!(now, [shelf("1", s)]);
}

#[test]
fn undo_and_redo_of_shelf_steps_take_their_shelves_off_and_put_them_back() {
    let (_dir, mut repo) = saved(&[("a.txt", b"a")]);
    let save = tip_of(&repo.current().unwrap().1).unwrap();
    let who = who();
    let shelved = plant(&mut repo, "shelf", |s| {
        s.shelves.insert(0, shelf("1", save))
    });
    let done = undo(&mut repo, &one_step(&who), &mut |_| {}).unwrap();
    assert_eq!(done.taken.len(), 1);
    assert_eq!(done.taken[0].step, shelved);
    let gone = Effect::ShelfGone {
        label: "1".into(),
        save,
    };
    assert_eq!(done.taken[0].effects, [gone]);
    let (_, entry) = repo.current().unwrap();
    assert_eq!(entry.action, Action::Undo(shelved));
    assert!(entry.state.shelves.is_empty());

    redo(&mut repo, &one_step(&who), &mut |_| {}).unwrap();
    assert_eq!(repo.current().unwrap().1.state.shelves, [shelf("1", save)]);

    let unshelved = plant(&mut repo, "unshelf", |s| s.shelves.clear());
    let done = undo(&mut repo, &one_step(&who), &mut |_| {}).unwrap();
    assert_eq!(done.taken[0].step, unshelved);
    assert_eq!(repo.current().unwrap().1.state.shelves, [shelf("1", save)]);
    redo(&mut repo, &one_step(&who), &mut |_| {}).unwrap();
    assert!(repo.current().unwrap().1.state.shelves.is_empty());
}

#[test]
fn a_step_whose_saves_are_gone_is_named_and_nothing_changes() {
    let (_dir, mut repo) = saved(&[("a.txt", b"a")]);
    let save = tip_of(&repo.current().unwrap().1).unwrap();
    // What the folder held before the step is a save the store doesn't have.
    let lost = FolderChange {
        from: Id([7; 32]),
        to: save,
    };
    let step = plant_with(&mut repo, "discard", Some(lost), |_| {});
    let who = who();
    let failed = undo(&mut repo, &one_step(&who), &mut |_| {}).unwrap_err();
    assert!(failed.taken.is_empty());
    match &failed.error {
        SwitchError::StepGone { step: named, .. } => assert_eq!(*named, step),
        other => panic!("{other:?}"),
    }
    assert!(failed.error.to_string().contains(&short_entry(&step)));
    assert_eq!(repo.current().unwrap().0, step);
}

#[test]
fn undo_writes_a_link_through_a_folder_only_the_save_it_writes_has() {
    let (dir, mut repo) = saved(&[("a.txt", b"a")]);
    let root = dir.path();
    write_all(root, &[("d/x.txt", b"x")]);
    if !symlink(&Path::new("d").join("x.txt"), &root.join("l")) {
        return;
    }
    let who = who();
    let done = discard(&mut repo, None, &who, "discard", None, &mut |_| {}).unwrap();
    assert!(done.links_left.is_empty(), "{done:?}");
    assert!(fs::symlink_metadata(root.join("l")).is_err());
    undo(&mut repo, &one_step(&who), &mut |_| {}).unwrap();
    let link = fs::symlink_metadata(root.join("l")).unwrap();
    assert!(link.file_type().is_symlink(), "written as a plain file");
    assert_eq!(fs::read(root.join("l")).unwrap(), b"x");
}
