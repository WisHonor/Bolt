#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use super::*;
use crate::journal::Journal;
use crate::lines::Target;
use crate::listing::EntryKind;
use crate::repo::RepoError;
use crate::snapshot::{SaveOutcome, SaveRequest, save_folder};
use crate::switch::{discard, goto};
use crate::test_support::{plant_save, symlink};
use crate::warning;
use std::collections::{BTreeMap, HashMap};
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
        message: Some("next"),
        who: &who,
        command: "save",
        accept_crlf: false,
        git_excludes: None,
        new_line: None,
        only: None,
        amend: false,
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

/// A project on line `old`, its folder holding `files` as that line's save has them, and `main`
/// moved on to a save of `entries`. Gives the two saves.
fn on_old_line(
    files: &[(&str, &[u8])],
    entries: &[(&str, EntryKind, &[u8])],
) -> (tempfile::TempDir, Repository, Id, Id) {
    let (dir, mut repo) = saved(files);
    let old = tip_of(&repo.current().unwrap().1).unwrap();
    plant_save(&mut repo, entries);
    let main = tip_of(&repo.current().unwrap().1).unwrap();
    let lock = repo.lock_for_change("test", &mut |_| {}).unwrap();
    let (head, mut entry) = repo.current().unwrap();
    entry.parent = Some(head);
    entry.state.lines.insert("old".into(), old);
    entry.state.position = Position::OnLine("old".into());
    let packs = repo.new_packs().unwrap();
    repo.commit(&lock, packs, &entry).unwrap();
    drop(lock);
    (dir, repo, old, main)
}

fn goto_main(repo: &mut Repository) {
    let main = Target::Line("main".into());
    goto(repo, &main, &who(), "goto", None, &mut |_| {}).unwrap();
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
    match &*failed.error {
        SwitchError::StepGone { step: named, .. } => assert_eq!(*named, step),
        other => panic!("{other:?}"),
    }
    assert!(failed.error.to_string().contains(&step.short_hex()));
    assert_eq!(repo.current().unwrap().0, step);
}

#[test]
fn a_journal_left_after_its_switch_counts_as_a_stop_finished() {
    let (_dir, mut repo) = saved(&[("a.txt", b"a")]);
    let before = repo.head().unwrap();
    let step = plant(&mut repo, "save", |_| {});
    // The switch took effect (`head` names its entry), but its journal was left behind.
    {
        let lock = repo.lock_for_change("test", &mut |_| {}).unwrap();
        let left = Journal {
            target: step,
            from: before,
            actions: vec![],
        };
        left.write(&repo, &lock).unwrap();
    }
    let who = who();
    let done = undo(&mut repo, &one_step(&who), &mut |_| {}).unwrap();
    assert!(done.finished);
    assert!(done.taken.is_empty());
    assert_eq!(repo.head().unwrap(), step);
    assert_eq!(Journal::read(&repo).unwrap(), None);
    // Run again, it takes the step.
    let done = undo(&mut repo, &one_step(&who), &mut |_| {}).unwrap();
    assert!(!done.finished);
    assert_eq!(done.taken[0].step, step);
}

#[test]
fn a_missing_diary_entry_is_damage_that_names_it() {
    let (_dir, mut repo) = saved(&[("a.txt", b"a")]);
    let missing = Id([9; 32]);
    let lock = repo.lock_for_change("test", &mut |_| {}).unwrap();
    let (_, mut entry) = repo.current().unwrap();
    entry.parent = Some(missing);
    let packs = repo.new_packs().unwrap();
    repo.commit(&lock, packs, &entry).unwrap();
    drop(lock);
    let who = who();
    let failed = undo(&mut repo, &one_step(&who), &mut |_| {}).unwrap_err();
    match *failed.error {
        SwitchError::Repo(RepoError::Damaged { detail }) => {
            assert!(detail.contains(&missing.to_hex()), "{detail}");
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn the_steps_warnings_are_told_as_one_commands() {
    let state = State {
        lines: BTreeMap::new(),
        tags: None,
        position: Position::AtSave(Id([0; 32])),
        shelves: vec![],
        server_lines: None,
    };
    let entry = DiaryEntry {
        parent: None,
        action: Action::Command,
        command: "goto".into(),
        who: who(),
        bolt_version: String::new(),
        state: state.clone(),
        folder_change: None,
    };
    let skipped = Warning::new(warning::GITIGNORE_SKIPPED, "skipped");
    let step = |placeholder: &str| {
        let placeholders = vec![placeholder.to_owned()];
        let applied = Applied {
            warnings: vec![
                writeout::placeholders_warning(&placeholders),
                skipped.clone(),
            ],
            placeholders,
            ..Applied::default()
        };
        Taken {
            step: Id([1; 32]),
            entry: entry.clone(),
            action: Action::Undo(Id([1; 32])),
            before: state.clone(),
            after: state.clone(),
            effects: vec![],
            applied,
        }
    };
    let both = ["a".to_owned(), "b".to_owned(), "c".to_owned()];
    let one_warning = writeout::placeholders_warning(&both);
    // The step a file in use stopped wrote some too.
    let stopped = Earlier {
        placeholders: vec!["c".to_owned()],
        not_written: vec![],
    };
    assert_eq!(
        warnings(&[step("a"), step("b")], stopped),
        [skipped.clone(), one_warning]
    );
}

#[cfg(windows)]
#[test]
fn a_step_stopped_by_a_file_in_use_shares_the_placeholders_warning_of_the_steps_before() {
    use std::os::windows::fs::OpenOptionsExt;
    use std::time::{Duration, SystemTime};
    const FILE: EntryKind = EntryKind::File;
    const LINK: EntryKind = EntryKind::Symlink;
    let (dir, mut repo) = saved(&[("x", b"x0")]);
    repo.set_cache_key([7; 32]);
    let start = tip_of(&repo.current().unwrap().1).unwrap();
    // Links that lead out of the project, so each is written as a placeholder.
    let mut tips = Vec::new();
    for entries in [
        &[("l1", LINK, b"/out1" as &[u8]), ("x", FILE, b"x0")][..],
        &[
            ("l1", LINK, b"/out1"),
            ("l2", LINK, b"/out2"),
            ("x", FILE, b"xb"),
        ],
    ] {
        plant_save(&mut repo, entries);
        tips.push(tip_of(&repo.current().unwrap().1).unwrap());
    }
    plant(&mut repo, "test", |state| {
        state.lines.insert("main".into(), start);
        state.lines.insert("a".into(), tips[0]);
        state.lines.insert("b".into(), tips[1]);
    });
    for line in ["a", "b"] {
        let to = Target::Line(line.into());
        goto(&mut repo, &to, &who(), "goto", None, &mut |_| {}).unwrap();
    }
    let who = who();
    let two = StepRequest {
        n: 2,
        ..one_step(&who)
    };
    undo(&mut repo, &two, &mut |_| {}).unwrap();
    // Undoing the gotos took both placeholders away.
    for link in ["l1", "l2"] {
        assert!(!dir.path().join(link).exists(), "{link}");
    }
    // Old enough for the file-state cache to vouch for, so nothing has to read it while held.
    let x = dir.path().join("x");
    let file = fs::File::options().write(true).open(&x).unwrap();
    file.set_modified(SystemTime::now() - Duration::from_secs(3600))
        .unwrap();
    drop(file);
    crate::status::status(&repo, false, None).unwrap();
    let held = fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(&x)
        .unwrap();
    // The redo of `goto a` writes l1; the redo of `goto b` writes l2, then stops at x.
    let failed = redo(&mut repo, &two, &mut |_| {}).unwrap_err();
    drop(held);
    assert_eq!(failed.taken.len(), 1, "{:?}", failed.error);
    let mut told = failed.warnings;
    told.extend(repo.store().warnings());
    let placeholders: Vec<&Warning> = told
        .iter()
        .filter(|w| w.kind == warning::PLACEHOLDERS_WRITTEN)
        .collect();
    assert_eq!(placeholders.len(), 1, "{told:?}");
    assert!(
        placeholders[0].message.ends_with("\n  l1\n  l2"),
        "{told:?}"
    );
}

#[test]
fn undo_and_redo_record_their_steps_own_folder_change() {
    const FILE: EntryKind = EntryKind::File;
    let main_files = [("a.txt", FILE, b"A" as &[u8]), ("c.txt", FILE, b"c")];
    let (dir, mut repo, _, _) = on_old_line(&[("a.txt", b"a")], &main_files);
    let root = dir.path();
    // Unsaved work that already matches main: the goto finds it there, and writes only a.txt.
    write_all(root, &[("c.txt", b"c")]);
    goto_main(&mut repo);
    let change = repo.current().unwrap().1.folder_change.unwrap();
    let who = who();
    undo(&mut repo, &one_step(&who), &mut |_| {}).unwrap();
    let backwards = FolderChange {
        from: change.to,
        to: change.from,
    };
    assert_eq!(repo.current().unwrap().1.folder_change, Some(backwards));
    assert_eq!(fs::read(root.join("a.txt")).unwrap(), b"a");
    assert_eq!(fs::read(root.join("c.txt")).unwrap(), b"c");
    redo(&mut repo, &one_step(&who), &mut |_| {}).unwrap();
    assert_eq!(repo.current().unwrap().1.folder_change, Some(change));
    assert_eq!(fs::read(root.join("a.txt")).unwrap(), b"A");
}

/// Every entry of a save by path: files and links with their IDs, folders without.
fn tree_of(repo: &Repository, save: Id) -> BTreeMap<String, (EntryKind, Option<Id>)> {
    let store = repo.store();
    let mut found = BTreeMap::new();
    let mut todo = vec![(String::new(), store.save(&save, false).unwrap().root)];
    while let Some((at, listing)) = todo.pop() {
        for e in store.listing(&listing).unwrap().entries() {
            let path = if at.is_empty() {
                e.name.clone()
            } else {
                format!("{at}/{}", e.name)
            };
            let id = (e.kind != EntryKind::Folder).then_some(e.id);
            if e.kind == EntryKind::Folder {
                todo.push((path.clone(), e.id));
            }
            found.insert(path, (e.kind, id));
        }
    }
    found
}

#[test]
fn a_folder_the_user_removed_stays_removed_through_an_undo_and_a_redo() {
    const FILE: EntryKind = EntryKind::File;
    let old: &[(&str, &[u8])] = &[
        ("a.txt", b"a"),
        ("assets/a.png", b"A"),
        ("assets/b.png", b"B"),
    ];
    let main = [
        ("a.txt", FILE, b"a" as &[u8]),
        ("assets/a.png", FILE, b"A"),
        ("assets/b.png", FILE, b"B"),
        ("assets/new.png", FILE, b"N"),
    ];
    let (dir, mut repo, _, _) = on_old_line(old, &main);
    let root = dir.path();
    let in_assets = || {
        let mut names: Vec<String> = fs::read_dir(root.join("assets"))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    };
    // Removed without saving: the goto makes the folder again for new.png alone.
    fs::remove_dir_all(root.join("assets")).unwrap();
    goto_main(&mut repo);
    assert_eq!(in_assets(), ["new.png"]);
    let change = repo.current().unwrap().1.folder_change.unwrap();
    let (from, to) = (tree_of(&repo, change.from), tree_of(&repo, change.to));
    let differ: BTreeSet<&str> = from
        .keys()
        .chain(to.keys())
        .filter(|p| from.get(*p) != to.get(*p))
        .map(String::as_str)
        .collect();
    assert_eq!(differ, BTreeSet::from(["assets", "assets/new.png"]));
    let who = who();
    undo(&mut repo, &one_step(&who), &mut |_| {}).unwrap();
    assert!(!root.join("assets").exists());
    redo(&mut repo, &one_step(&who), &mut |_| {}).unwrap();
    assert_eq!(in_assets(), ["new.png"]);
}

#[test]
fn a_file_deleted_with_a_folder_stays_gone_when_a_discard_of_part_of_it_is_redone() {
    let (dir, mut repo) = saved(&[("assets/a.png", b"A"), ("assets/b.png", b"B")]);
    let root = dir.path();
    // A file where the folder was: discarding a.png makes the folder again for a.png alone.
    fs::remove_dir_all(root.join("assets")).unwrap();
    fs::write(root.join("assets"), b"F").unwrap();
    let who = who();
    let only = ["assets/a.png".to_owned()];
    discard(&mut repo, Some(&only), &who, "discard", None, &mut |_| {}).unwrap();
    assert!(!root.join("assets/b.png").exists());
    undo(&mut repo, &one_step(&who), &mut |_| {}).unwrap();
    assert_eq!(fs::read(root.join("assets")).unwrap(), b"F");
    redo(&mut repo, &one_step(&who), &mut |_| {}).unwrap();
    assert_eq!(fs::read(root.join("assets/a.png")).unwrap(), b"A");
    assert!(
        !root.join("assets/b.png").exists(),
        "the deleted b.png came back"
    );
}

#[test]
fn a_redo_writes_a_link_through_a_folder_the_folder_already_held() {
    let entries = [
        ("a.txt", EntryKind::File, b"a" as &[u8]),
        ("d/x.txt", EntryKind::File, b"x"),
        ("l", EntryKind::Symlink, b"d/x.txt"),
    ];
    let (dir, mut repo, _, main) = on_old_line(&[("a.txt", b"a")], &entries);
    let root = dir.path();
    if !symlink(Path::new("a.txt"), &root.join("probe")) {
        return;
    }
    fs::remove_file(root.join("probe")).unwrap();
    // The folder already holds main's d, so the goto writes only the link through it.
    write_all(root, &[("d/x.txt", b"x")]);
    goto_main(&mut repo);
    let is_link = |root: &Path| {
        let found = fs::symlink_metadata(root.join("l")).unwrap();
        found.file_type().is_symlink()
    };
    assert!(is_link(root));
    let change = repo.current().unwrap().1.folder_change.unwrap();
    assert_eq!(change.to, main);
    let who = who();
    undo(&mut repo, &one_step(&who), &mut |_| {}).unwrap();
    assert!(fs::symlink_metadata(root.join("l")).is_err());
    assert_eq!(fs::read(root.join("d/x.txt")).unwrap(), b"x");
    redo(&mut repo, &one_step(&who), &mut |_| {}).unwrap();
    assert!(is_link(root), "written as a plain file");
    assert_eq!(repo.current().unwrap().1.folder_change, Some(change));
}

#[test]
fn shelve_stores_nothing_when_a_link_it_must_leave_is_in_the_way() {
    let entries = [
        ("a.txt", EntryKind::File, b"a" as &[u8]),
        ("w/f.txt", EntryKind::File, b"f"),
    ];
    let (dir, mut repo, _, _) = on_old_line(&[("a.txt", b"a")], &entries);
    let root = dir.path();
    goto_main(&mut repo);
    // New in w since: a file --shelve could set aside, and a link out of the project it can't.
    write_all(root, &[("w/mine.txt", b"mine")]);
    let out = Path::new("..").join("..").join("outside");
    if !symlink(&out, &root.join("w").join("out")) {
        return;
    }
    let (head, packs) = (repo.head().unwrap(), repo.store().pack_count());
    let who = who();
    let shelve = StepRequest {
        shelve: true,
        ..one_step(&who)
    };
    let failed = undo(&mut repo, &shelve, &mut |_| {}).unwrap_err();
    match *failed.error {
        SwitchError::InTheWayOfStep {
            paths,
            shelve: true,
            ..
        } => assert_eq!(paths, ["w"]),
        other => panic!("{other:?}"),
    }
    assert_eq!(
        (repo.head().unwrap(), repo.store().pack_count()),
        (head, packs)
    );
    assert_eq!(fs::read(root.join("w/mine.txt")).unwrap(), b"mine");
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

#[test]
fn an_entry_is_named_by_digits_no_other_entry_starts_with() {
    // Two entries whose IDs share their first 8 hex digits, the newer one after the older.
    let older = Id([0xab; 32]);
    let mut bytes = [0xab; 32];
    bytes[4] = 0;
    let newer = Id(bytes);
    let entry = |parent| DiaryEntry {
        parent,
        action: Action::Command,
        command: "save".into(),
        who: who(),
        bolt_version: String::new(),
        state: State {
            lines: BTreeMap::new(),
            tags: None,
            position: Position::AtSave(Id([0; 32])),
            shelves: vec![],
            server_lines: None,
        },
        folder_change: None,
    };
    let diary = HashMap::from([(newer, entry(Some(older))), (older, entry(None))]);
    let mut read = |id: &Id| {
        let found = diary.get(id).cloned();
        found.ok_or_else(|| RepoError::Damaged {
            detail: id.to_hex(),
        })
    };
    match entry_named("abababab", newer, &mut read) {
        Err(SwitchError::EntryAmbiguous { entries, .. }) => assert_eq!(entries, [newer, older]),
        other => panic!("{other:?}"),
    }
    // One digit more tells them apart.
    assert_eq!(
        entry_named("ababababab", newer, &mut read).unwrap().0,
        older
    );
    assert_eq!(
        entry_named("abababab00", newer, &mut read).unwrap().0,
        newer
    );
    let none = entry_named("abababac", newer, &mut read);
    assert!(matches!(none, Err(SwitchError::NoEntry { .. })), "{none:?}");
}

/// The packs in the store, by path.
fn pack_paths(repo: &Repository) -> HashSet<std::path::PathBuf> {
    let listed = fs::read_dir(repo.packs_dir()).unwrap();
    listed.map(|e| e.unwrap().path()).collect()
}

/// A pack whose index fails part way for a reason other than damage, here cut short while open,
/// is passed over whole with a lost `head`: the entries read before the failure aren't offered.
#[test]
fn a_lost_head_passes_over_a_pack_whose_index_fails_part_way() {
    let (dir, mut repo) = saved(&[("a.txt", b"a")]);
    let (head, entry) = repo.current().unwrap();
    // Two crashed commands' entries, one after the other, alone in a pack.
    let before = pack_paths(&repo);
    let lock = repo.lock_for_change("test", &mut |_| {}).unwrap();
    let mut packs = repo.new_packs().unwrap();
    let first = DiaryEntry {
        parent: Some(head),
        ..entry
    };
    let one = packs
        .add_record(ObjectKind::DiaryEntry, &first.encode())
        .unwrap();
    let second = DiaryEntry {
        parent: Some(one),
        ..first
    };
    repo.store_entry(&lock, packs, &second).unwrap();
    drop(lock);
    let [cut]: [_; 1] = pack_paths(&repo)
        .difference(&before)
        .cloned()
        .collect::<Vec<_>>()
        .try_into()
        .unwrap();
    fs::remove_file(dir.path().join(".bolt/state/head")).unwrap();
    let mut repo = Repository::open(dir.path()).unwrap();
    // Opened now, and kept open; then cut after its first index line: the trailer is 48 bytes,
    // and an index line 25.
    repo.entry(&one).unwrap();
    let len = fs::metadata(&cut).unwrap().len();
    let file = fs::OpenOptions::new().write(true).open(&cut).unwrap();
    file.set_len(len - 48 - 25).unwrap();
    drop(file);
    let ends: Vec<Id> = stopped_at(&repo).unwrap().iter().map(|e| e.entry).collect();
    assert_eq!(ends, [head]);
    let warnings = repo.store().warnings();
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert_eq!(warnings[0].kind, warning::PACK_UNREADABLE);
    let name = cut.file_name().unwrap().to_str().unwrap();
    assert!(warnings[0].message.contains(name), "{warnings:?}");
    // And it is picked up from the others.
    let who = who();
    let done = undo_to(&mut repo, &head.to_hex()[..8], &one_step(&who), &mut |_| {}).unwrap();
    assert!(done.head_lost);
    assert_eq!(repo.current().unwrap().1.parent, Some(head));
}
