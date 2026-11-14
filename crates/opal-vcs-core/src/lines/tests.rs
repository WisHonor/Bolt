#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use super::*;
use crate::diary::{Action, DiaryEntry, ServerLines, Tag, TagList};
use crate::id::ChangeId;
use crate::pack::ObjectKind;
use crate::save::Save;
use crate::undo::{StepRequest, undo};
use std::collections::BTreeMap;

fn who() -> Person {
    Person {
        name: "Ada".into(),
        email: "ada@example.com".into(),
        time_ms: 1_700_000_000_000,
        tz_offset_min: 0,
    }
}

/// A new project, on `main` at its first save, which this gives.
fn project() -> (tempfile::TempDir, Repository, Id) {
    let dir = tempfile::tempdir().unwrap();
    let repo = Repository::init(dir.path(), &who()).unwrap();
    let start = repo.current().unwrap().1.state.lines["main"];
    (dir, repo, start)
}

/// Stores a save with `parents`, a copy of the first one otherwise, in a step that changes no
/// line.
fn plant(repo: &mut Repository, parents: &[Id]) -> Id {
    let lock = repo.lock_for_change("save", &mut |_| {}).unwrap();
    let (head, mut entry) = repo.current().unwrap();
    let first = repo.store().save(&parents[0], false).unwrap();
    let save = Save {
        parents: parents.to_vec(),
        message: format!("after {}", parents[0].short_hex()),
        change_id: ChangeId::random().unwrap(),
        ..first
    };
    let mut packs = repo.new_packs().unwrap();
    let id = packs.add_record(ObjectKind::Save, &save.encode()).unwrap();
    entry.parent = Some(head);
    entry.command = "save".into();
    entry.folder_change = None;
    repo.commit(&lock, packs, &entry).unwrap();
    id
}

/// Commits a step whose state is the current one as `change` leaves it, with `tags` if any.
fn set(repo: &mut Repository, tags: Option<&TagList>, change: impl FnOnce(&mut State)) {
    let lock = repo.lock_for_change("test", &mut |_| {}).unwrap();
    let (head, mut entry) = repo.current().unwrap();
    let mut packs = repo.new_packs().unwrap();
    if let Some(tags) = tags {
        let id = packs
            .add_record(ObjectKind::TagList, &tags.encode())
            .unwrap();
        entry.state.tags = Some(id);
    }
    change(&mut entry.state);
    let entry = DiaryEntry {
        parent: Some(head),
        action: Action::Command,
        command: "test".into(),
        folder_change: None,
        ..entry
    };
    repo.commit(&lock, packs, &entry).unwrap();
}

fn tag_at(save: Id) -> TagList {
    let tag = Tag {
        save,
        message: "First playable".into(),
        git_tags: vec![],
    };
    TagList {
        tags: BTreeMap::from([("v1".to_owned(), tag)]),
    }
}

fn state(repo: &Repository) -> State {
    repo.current().unwrap().1.state
}

fn made(repo: &mut Repository, name: &str) -> Result<Made, LineError> {
    new_line(repo, name, &who(), &mut |_| {})
}

fn renamed(repo: &mut Repository, old: Option<&str>, new: &str) -> Result<Renamed, LineError> {
    rename_line(repo, old, new, &who(), &mut |_| {})
}

/// Deletes `name`, saying yes or `answer`, and gives what `confirm` was asked.
fn deleted(
    repo: &mut Repository,
    name: &str,
    answer: bool,
) -> (Result<Deleted, LineError>, Vec<(String, u64)>) {
    let mut asked = Vec::new();
    let done = delete_line(repo, name, &who(), &mut |_| {}, &mut |line, n| {
        asked.push((line.to_owned(), n));
        answer
    });
    (done, asked)
}

/// A project on `main` at its first save, with line `b` two saves past it.
fn two_of_its_own() -> (tempfile::TempDir, Repository, Id, Id) {
    let (dir, mut repo, start) = project();
    let one = plant(&mut repo, &[start]);
    let two = plant(&mut repo, &[one]);
    set(&mut repo, None, |s| {
        s.lines.insert("b".into(), two);
    });
    (dir, repo, start, two)
}

#[test]
fn a_new_line_starts_at_the_save_the_project_is_on_and_is_moved_onto() {
    let (_dir, mut repo, s) = project();
    let done = made(&mut repo, "wall-run").unwrap();
    assert_eq!((done.line.as_str(), done.save), ("wall-run", s));
    let (_, entry) = repo.current().unwrap();
    assert_eq!(entry.state.lines["wall-run"], s);
    assert_eq!(entry.state.lines["main"], s);
    assert_eq!(entry.state.position, Position::OnLine("wall-run".into()));
    assert_eq!(entry.command, "line new");
    assert_eq!(entry.folder_change, None);

    set(&mut repo, None, |st| st.position = Position::AtSave(s));
    made(&mut repo, "from-a-save").unwrap();
    let st = state(&repo);
    assert_eq!(st.lines["from-a-save"], s);
    assert_eq!(st.position, Position::OnLine("from-a-save".into()));
}

#[test]
fn a_new_lines_name_is_checked() {
    let (_dir, mut repo, s) = project();
    set(&mut repo, Some(&tag_at(s)), |_| {});
    let head = repo.head().unwrap();
    let bad = made(&mut repo, "wall run").unwrap_err();
    assert!(
        matches!(bad, LineError::Name(NameError::Bad { .. })),
        "{bad:?}"
    );
    let taken = made(&mut repo, "Main").unwrap_err();
    assert!(
        matches!(&taken, LineError::Name(NameError::Taken { taken, .. }) if taken == "main"),
        "{taken:?}"
    );
    let tag = made(&mut repo, "v1").unwrap_err();
    assert!(
        matches!(tag, LineError::Name(NameError::Tag { .. })),
        "{tag:?}"
    );
    assert_eq!(
        tag.to_string(),
        "\"v1\" is a tag's name already, and a line can't share it."
    );
    assert_eq!(repo.head().unwrap(), head);
}

#[test]
fn a_rename_keeps_the_project_on_the_line() {
    let (_dir, mut repo, s) = project();
    let done = renamed(&mut repo, None, "trunk").unwrap();
    assert!(done.renamed);
    assert_eq!((done.from.as_str(), done.to.as_str()), ("main", "trunk"));
    let (_, entry) = repo.current().unwrap();
    assert_eq!(entry.state.lines, BTreeMap::from([("trunk".to_owned(), s)]));
    assert_eq!(entry.state.position, Position::OnLine("trunk".into()));
    assert_eq!(entry.command, "line rename");
    assert_eq!(entry.folder_change, None);
}

#[test]
fn a_line_can_take_another_spelling_of_its_own_name() {
    let (_dir, mut repo, _) = project();
    renamed(&mut repo, Some("main"), "Main").unwrap();
    renamed(&mut repo, Some("Main"), "main").unwrap();
    assert!(state(&repo).lines.contains_key("main"));
    made(&mut repo, "other").unwrap();
    let taken = renamed(&mut repo, Some("main"), "Other").unwrap_err();
    assert!(
        matches!(&taken, LineError::Name(NameError::Taken { taken, .. }) if taken == "other"),
        "{taken:?}"
    );
}

#[test]
fn renaming_to_the_same_name_makes_no_step() {
    let (_dir, mut repo, _) = project();
    let head = repo.head().unwrap();
    let done = renamed(&mut repo, Some("main"), "main").unwrap();
    assert!(!done.renamed);
    assert_eq!(repo.head().unwrap(), head);
}

#[test]
fn renaming_the_current_line_at_a_save_is_refused() {
    let (_dir, mut repo, s) = project();
    set(&mut repo, None, |st| st.position = Position::AtSave(s));
    let refused = renamed(&mut repo, None, "trunk").unwrap_err();
    assert!(matches!(refused, LineError::NotOnALine), "{refused:?}");
    let unknown = renamed(&mut repo, Some("nope"), "trunk").unwrap_err();
    assert!(matches!(unknown, LineError::NoSuchLine(_)), "{unknown:?}");
}

#[test]
fn the_line_the_project_is_on_is_not_deleted() {
    let (_dir, mut repo, _) = project();
    let (refused, asked) = deleted(&mut repo, "main", true);
    assert!(
        matches!(refused, Err(LineError::OnIt(ref line)) if line == "main"),
        "{refused:?}"
    );
    assert!(asked.is_empty());
    let (unknown, _) = deleted(&mut repo, "nope", true);
    assert!(
        matches!(unknown, Err(LineError::NoSuchLine(_))),
        "{unknown:?}"
    );
}

#[test]
fn deleting_asks_only_when_nothing_else_holds_the_saves() {
    let (_dir, mut repo, _, two) = two_of_its_own();
    let (done, asked) = deleted(&mut repo, "b", true);
    assert_eq!(asked, [("b".to_owned(), 2)]);
    let done = done.unwrap();
    assert_eq!(
        (done.line.as_str(), done.save, done.only_here),
        ("b", two, 2)
    );
    let (_, entry) = repo.current().unwrap();
    assert!(!entry.state.lines.contains_key("b"));
    assert_eq!(entry.command, "line delete");
    assert_eq!(entry.folder_change, None);

    // At main's newest save.
    let (_dir, mut repo, start) = project();
    set(&mut repo, None, |st| {
        st.lines.insert("c".into(), start);
    });
    let (done, asked) = deleted(&mut repo, "c", true);
    assert_eq!(done.unwrap().only_here, 0);
    assert!(asked.is_empty());

    // Held only by a tag.
    let (_dir, mut repo, _, two) = two_of_its_own();
    set(&mut repo, Some(&tag_at(two)), |_| {});
    let (done, asked) = deleted(&mut repo, "b", true);
    assert_eq!(done.unwrap().only_here, 0);
    assert!(asked.is_empty());

    // Held by main through a second parent.
    let (_dir, mut repo, start, two) = two_of_its_own();
    let combined = plant(&mut repo, &[start, two]);
    set(&mut repo, None, |st| {
        st.lines.insert("main".into(), combined);
    });
    let (done, asked) = deleted(&mut repo, "b", true);
    assert_eq!(done.unwrap().only_here, 0);
    assert!(asked.is_empty());
}

#[test]
fn a_server_line_holds_a_line_too() {
    let (_dir, mut repo, _, two) = two_of_its_own();
    let servers = ServerLines {
        servers: BTreeMap::from([("origin".to_owned(), BTreeMap::from([("b".to_owned(), two)]))]),
    };
    let lock = repo.lock_for_change("test", &mut |_| {}).unwrap();
    let (head, mut entry) = repo.current().unwrap();
    let mut packs = repo.new_packs().unwrap();
    let id = packs
        .add_record(ObjectKind::ServerLines, &servers.encode())
        .unwrap();
    entry.parent = Some(head);
    entry.state.server_lines = Some(id);
    repo.commit(&lock, packs, &entry).unwrap();
    drop(lock);
    let (done, asked) = deleted(&mut repo, "b", true);
    assert_eq!(done.unwrap().only_here, 0);
    assert!(asked.is_empty());
}

#[test]
fn standing_at_a_lines_newest_save_does_not_hold_it() {
    let (_dir, mut repo, _, two) = two_of_its_own();
    set(&mut repo, None, |st| st.position = Position::AtSave(two));
    let (done, asked) = deleted(&mut repo, "b", true);
    assert_eq!(asked, [("b".to_owned(), 2)]);
    assert_eq!(done.unwrap().only_here, 2);
}

#[test]
fn a_no_keeps_the_line() {
    let (_dir, mut repo, _, _) = two_of_its_own();
    let head = repo.head().unwrap();
    let (kept, _) = deleted(&mut repo, "b", false);
    let kept = kept.unwrap_err();
    assert!(
        matches!(&kept, LineError::Kept { line, only_here: 2 } if line == "b"),
        "{kept:?}"
    );
    assert_eq!(
        kept.to_string(),
        "Line b holds 2 saves that no other line or tag has, so it wasn't deleted."
    );
    assert_eq!(
        kept.hint().as_deref(),
        Some("To delete it anyway: opalro line delete b --yes")
    );
    assert_eq!(repo.head().unwrap(), head);
}

#[test]
fn undo_takes_each_line_step_back() {
    let (_dir, mut repo, start) = project();
    set(&mut repo, None, |st| {
        st.lines.insert("b".into(), start);
    });
    let steps: [&dyn Fn(&mut Repository); 3] = [
        &|r| drop(made(r, "c").unwrap()),
        &|r| drop(renamed(r, None, "trunk").unwrap()),
        &|r| drop(deleted(r, "b", true).0.unwrap()),
    ];
    for step in steps {
        let before = state(&repo);
        step(&mut repo);
        assert_ne!(state(&repo), before);
        let req = StepRequest {
            n: 1,
            shelve: false,
            who: &who(),
            git_excludes: None,
        };
        let done = undo(&mut repo, &req, &mut |_| {}).unwrap();
        assert_eq!(done.taken.len(), 1);
        let after = state(&repo);
        assert_eq!(
            (after.lines, after.position),
            (before.lines, before.position)
        );
    }
}

#[cfg(feature = "fault-injection")]
#[test]
fn a_line_step_after_a_stopped_goto_finishes_it_first() {
    use crate::fsio::fault;
    use crate::journal::Journal;
    use crate::listing::EntryKind;
    use crate::switch::goto;
    use crate::test_support::plant_save;
    use crate::warning::SWITCH_FINISHED;
    for n in 1.. {
        assert!(n < 100, "no goto stopped between its journal and its head");
        // On line `old`, the folder holding its file, with `main` a save further on.
        let (dir, mut repo, start) = project();
        std::fs::write(dir.path().join("a.txt"), b"a").unwrap();
        let first = crate::snapshot::save_folder(
            &mut repo,
            &crate::snapshot::SaveRequest {
                message: Some("first"),
                who: &who(),
                command: "save",
                accept_crlf: false,
                git_excludes: None,
                new_line: None,
                only: None,
                amend: false,
            },
            &mut |_| {},
            &mut |_| true,
        );
        assert!(first.is_ok(), "{first:?}");
        let old = state(&repo).lines["main"];
        assert_ne!(old, start);
        plant_save(&mut repo, &[("a.txt", EntryKind::File, b"A")]);
        let main = state(&repo).lines["main"];
        set(&mut repo, None, |st| {
            st.lines.insert("old".into(), old);
            st.position = Position::OnLine("old".into());
        });

        fault::arm(n);
        let stopped = goto(
            &mut repo,
            &Target::Line("main".into()),
            &who(),
            "goto",
            None,
            &mut |_| {},
        );
        fault::disarm();
        drop(repo);
        let mut repo = Repository::open(dir.path()).unwrap();
        let journal = Journal::read(&repo).unwrap();
        let unfinished = journal.is_some_and(|j| j.target != repo.head().unwrap());
        // Only a stop between the journal and the head leaves a switch to finish.
        if !unfinished {
            continue;
        }
        assert!(stopped.is_err(), "fault {n}");
        let done = made(&mut repo, "next").unwrap();
        let mut told = done.warnings.clone();
        told.extend(repo.store().warnings());
        assert!(
            told.iter().any(|w| w.kind == SWITCH_FINISHED),
            "fault {n}: {told:?}"
        );
        assert_eq!(done.save, main, "fault {n}");
        assert_eq!(state(&repo).lines["next"], main);
        assert_eq!(std::fs::read(dir.path().join("a.txt")).unwrap(), b"A");
        break;
    }
}
