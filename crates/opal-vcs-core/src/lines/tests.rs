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
fn a_new_name_cant_start_with_a_dash_but_a_stored_one_still_works() {
    let (_dir, mut repo, s) = project();
    for name in ["-", "-x", "--yes"] {
        let bad = made(&mut repo, name).unwrap_err();
        assert!(
            matches!(&bad, LineError::Name(NameError::Bad { reason, .. })
                if reason == "a line name cannot start with -"),
            "{bad:?}"
        );
        let bad = renamed(&mut repo, Some("main"), name).unwrap_err();
        assert!(
            matches!(bad, LineError::Name(NameError::Bad { .. })),
            "{bad:?}"
        );
    }
    // One an import brought in still renames and deletes.
    set(&mut repo, None, |st| {
        st.lines.insert("-old".into(), s);
        st.lines.insert("-gone".into(), s);
    });
    renamed(&mut repo, Some("-old"), "side").unwrap();
    let (done, _) = deleted(&mut repo, "-gone", true);
    done.unwrap();
    assert_eq!(
        state(&repo).lines.keys().collect::<Vec<_>>(),
        ["main", "side"]
    );
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
    let head = repo.head().unwrap();
    let taken = renamed(&mut repo, Some("main"), "Other").unwrap_err();
    assert!(
        matches!(&taken, LineError::Name(NameError::Taken { taken, .. }) if taken == "other"),
        "{taken:?}"
    );
    let bad = renamed(&mut repo, Some("main"), "wall run").unwrap_err();
    assert!(
        matches!(bad, LineError::Name(NameError::Bad { .. })),
        "{bad:?}"
    );
    assert_eq!(repo.head().unwrap(), head);
}

#[test]
fn a_rename_elsewhere_leaves_the_position_as_it_is() {
    let (_dir, mut repo, s) = project();
    set(&mut repo, None, |st| {
        st.lines.insert("b".into(), s);
    });
    renamed(&mut repo, Some("b"), "c").unwrap();
    let st = state(&repo);
    assert_eq!(st.position, Position::OnLine("main".into()));
    assert_eq!(st.lines.keys().collect::<Vec<_>>(), ["c", "main"]);

    set(&mut repo, None, |st| st.position = Position::AtSave(s));
    renamed(&mut repo, Some("main"), "trunk").unwrap();
    let st = state(&repo);
    assert_eq!(st.position, Position::AtSave(s));
    assert_eq!(st.lines.keys().collect::<Vec<_>>(), ["c", "trunk"]);
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
    let head = repo.head().unwrap();
    let refused = renamed(&mut repo, None, "trunk").unwrap_err();
    assert!(matches!(refused, LineError::NotOnALine), "{refused:?}");
    let unknown = renamed(&mut repo, Some("nope"), "trunk").unwrap_err();
    assert!(matches!(unknown, LineError::NoSuchLine(_)), "{unknown:?}");
    assert_eq!(repo.head().unwrap(), head);
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

/// Gives server `origin` the line `line` at `save`, and no other.
fn server_line(repo: &mut Repository, line: &str, save: Id) {
    let servers = ServerLines {
        servers: BTreeMap::from([(
            "origin".to_owned(),
            BTreeMap::from([(line.to_owned(), save)]),
        )]),
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
}

/// A tag list with each of `tags`.
fn tags(tags: &[(&str, Id)]) -> TagList {
    let tag = |save| Tag {
        save,
        message: String::new(),
        git_tags: vec![],
    };
    TagList {
        tags: tags
            .iter()
            .map(|(n, s)| ((*n).to_owned(), tag(*s)))
            .collect(),
    }
}

#[test]
fn a_server_line_holds_a_line_too() {
    let (_dir, mut repo, _, two) = two_of_its_own();
    server_line(&mut repo, "b", two);
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

fn hex12(id: Id) -> String {
    id.to_hex()[..12].to_owned()
}

#[test]
fn a_server_line_is_read_in_the_same_order_by_resolve() {
    use crate::history::resolve;
    let (_dir, mut repo, start, two) = two_of_its_own();
    server_line(&mut repo, "main", two);
    assert_eq!(resolve(&repo, "main@origin").unwrap(), two);
    let on_server = Target::ServerLine {
        server: "origin".into(),
        line: "main".into(),
        save: two,
    };
    assert_eq!(resolve_target(&repo, "main@origin").unwrap(), on_server);
    // A tag of the same name at the same save: the server line.
    set(&mut repo, Some(&tags(&[("main@origin", two)])), |_| {});
    assert_eq!(resolve_target(&repo, "main@origin").unwrap(), on_server);
    // A tag holding `@` that no server line has is just the tag.
    set(&mut repo, Some(&tags(&[("b@origin", start)])), |_| {});
    assert_eq!(resolve(&repo, "b@origin").unwrap(), start);
    assert_eq!(
        resolve_target(&repo, "b@origin").unwrap(),
        Target::Save(start)
    );
}

#[test]
fn a_name_found_at_several_levels_is_the_first_in_order() {
    use crate::history::resolve;
    let (_dir, mut repo, start, two) = two_of_its_own();
    // `b` is a line and a tag at the line's save, `rel-1` a tag alone.
    set(
        &mut repo,
        Some(&tags(&[("b", two), ("rel-1", start)])),
        |_| {},
    );
    assert_eq!(
        resolve_target(&repo, "b").unwrap(),
        Target::Line("b".into())
    );
    assert_eq!(resolve(&repo, "b").unwrap(), two);
    assert_eq!(resolve_target(&repo, "rel-1").unwrap(), Target::Save(start));
}

#[test]
fn a_line_and_a_tag_of_one_name_at_two_saves_are_refused() {
    use crate::history::resolve;
    let (_dir, mut repo, start, two) = two_of_its_own();
    set(&mut repo, Some(&tags(&[("b", start)])), |_| {});
    let refused = resolve(&repo, "b").unwrap_err();
    assert!(
        matches!(&refused, ResolveError::LineOrTag { line, tag, .. }
            if *line == hex12(two) && *tag == hex12(start)),
        "{refused:?}"
    );
    assert_eq!(
        refused.to_string(),
        format!(
            "\"b\" is the name of a line and also of a tag at another save (line {}, tag {}), so \
             Opalro didn't pick either.",
            hex12(two),
            hex12(start)
        )
    );
    assert_eq!(
        refused.hint().unwrap(),
        "To reach one of them, type its ID as shown here. To end the double meaning, rename the \
         line (opalro line rename <old> <new>) or delete the tag (opalro tag --delete <name>)."
    );
    // Each ID the refusal shows reaches its save.
    assert_eq!(resolve(&repo, &hex12(two)).unwrap(), two);
    assert_eq!(resolve(&repo, &hex12(start)).unwrap(), start);
    assert!(matches!(
        resolve_target(&repo, "b"),
        Err(ResolveError::LineOrTag { .. })
    ));
    // Renaming the line is the way out; its own commands take its name as a line's.
    renamed(&mut repo, Some("b"), "b-line").unwrap();
    assert_eq!(resolve(&repo, "b").unwrap(), start);
    assert_eq!(resolve(&repo, "b-line").unwrap(), two);
}

#[test]
fn a_server_line_and_a_tag_of_one_name_at_two_saves_are_refused() {
    use crate::history::resolve;
    let (_dir, mut repo, start, two) = two_of_its_own();
    server_line(&mut repo, "main", two);
    set(&mut repo, Some(&tags(&[("main@origin", start)])), |_| {});
    let refused = resolve(&repo, "main@origin").unwrap_err();
    assert_eq!(
        refused.to_string(),
        format!(
            "\"main@origin\" is the name of a server line and also of a tag at another save \
             (server line {}, tag {}), so Opalro didn't pick either.",
            hex12(two),
            hex12(start)
        )
    );
    assert_eq!(
        refused.hint().unwrap(),
        "To reach one of them, type its ID as shown here. To end the double meaning, delete the \
         tag: opalro tag --delete <name>"
    );
    assert!(matches!(
        resolve_target(&repo, "main@origin"),
        Err(ResolveError::ServerLineOrTag { .. })
    ));
    // `two` is on line b, but the ID shown reaches the server line's save even with b gone.
    set(&mut repo, None, |st| {
        st.lines.remove("b");
    });
    assert_eq!(resolve(&repo, &hex12(two)).unwrap(), two);
}

/// A save no line reaches, after `parent`, and one after it.
fn off_the_lines(repo: &mut Repository, parent: Id) -> (Id, Id) {
    let behind = plant(repo, &[parent]);
    (behind, plant(repo, &[behind]))
}

#[test]
fn a_fingerprint_finds_a_save_a_tag_or_server_line_reaches() {
    use crate::history::resolve;
    let (_dir, mut repo, start) = project();
    let (behind_tag, tagged) = off_the_lines(&mut repo, start);
    let (behind_server, on_server) = off_the_lines(&mut repo, start);
    let (behind_none, unheld) = off_the_lines(&mut repo, start);
    for id in [behind_tag, tagged, behind_server, on_server] {
        assert!(matches!(
            resolve(&repo, &id.to_hex()[..8]),
            Err(ResolveError::NotFound(_))
        ));
    }
    set(&mut repo, Some(&tags(&[("rel-1", tagged)])), |_| {});
    server_line(&mut repo, "main", on_server);
    for id in [behind_tag, tagged, behind_server, on_server] {
        assert_eq!(resolve(&repo, &id.to_hex()[..8]).unwrap(), id);
        assert_eq!(resolve(&repo, &id.to_hex()).unwrap(), id);
    }
    for id in [behind_none, unheld] {
        assert!(matches!(
            resolve(&repo, &id.to_hex()[..8]),
            Err(ResolveError::NotFound(_))
        ));
    }
    // Fewer than 8 hex digits are still no ID.
    assert!(matches!(
        resolve(&repo, &tagged.to_hex()[..7]),
        Err(ResolveError::NotFound(_))
    ));
}

#[test]
fn a_line_named_like_the_start_of_a_tag_only_saves_id_is_refused() {
    use crate::history::resolve;
    let (_dir, mut repo, start) = project();
    let (_, tagged) = off_the_lines(&mut repo, start);
    let hex = tagged.to_hex()[..8].to_owned();
    set(&mut repo, Some(&tags(&[("rel-1", tagged)])), |st| {
        st.lines.insert(hex.clone(), start);
    });
    match resolve(&repo, &hex) {
        Err(ResolveError::LineOrSave { saves, .. }) => assert_eq!(saves, [tagged.to_hex()]),
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_tag_at_a_save_that_cant_be_read_breaks_no_lookup() {
    use crate::history::resolve;
    let (_dir, mut repo, start, two) = two_of_its_own();
    let missing = Id::from_hex(&"7".repeat(64)).unwrap();
    set(&mut repo, Some(&tags(&[("ghost", missing)])), |_| {});
    server_line(&mut repo, "main", missing);
    // The lines' saves are found as before; the missing save by its own fingerprint only.
    assert_eq!(resolve(&repo, &hex12(start)).unwrap(), start);
    assert_eq!(resolve(&repo, &hex12(two)).unwrap(), two);
    assert_eq!(resolve(&repo, &hex12(missing)).unwrap(), missing);
}

#[test]
fn a_line_named_like_another_saves_id_and_a_tag_is_refused_for_the_id_first() {
    use crate::history::resolve;
    let (_dir, mut repo, start, two) = two_of_its_own();
    let hex = start.to_hex()[..8].to_owned();
    set(&mut repo, Some(&tags(&[(&hex, start)])), |st| {
        st.lines.insert(hex.clone(), two);
    });
    let refused = resolve(&repo, &hex).unwrap_err();
    assert!(
        matches!(refused, ResolveError::LineOrSave { .. }),
        "{refused:?}"
    );
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

/// A project on line `old` whose `goto main` stopped between its journal and its head, so the
/// next command has the switch to finish. Gives `main`'s save too.
#[cfg(feature = "fault-injection")]
fn stopped_goto() -> (tempfile::TempDir, Repository, Id) {
    use crate::fsio::fault;
    use crate::journal::Journal;
    use crate::listing::EntryKind;
    use crate::switch::goto;
    use crate::test_support::plant_save;
    for n in 1..100 {
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
        let repo = Repository::open(dir.path()).unwrap();
        let journal = Journal::read(&repo).unwrap();
        // Only a stop between the journal and the head leaves a switch to finish.
        if journal.is_some_and(|j| j.target != repo.head().unwrap()) {
            assert!(stopped.is_err(), "fault {n}");
            return (dir, repo, main);
        }
    }
    panic!("no goto stopped between its journal and its head");
}

#[cfg(feature = "fault-injection")]
#[test]
fn a_line_step_after_a_stopped_goto_finishes_it_first() {
    use crate::warning::SWITCH_FINISHED;
    let (dir, mut repo, main) = stopped_goto();
    let done = made(&mut repo, "next").unwrap();
    let mut told = done.warnings.clone();
    told.extend(repo.store().warnings());
    assert!(told.iter().any(|w| w.kind == SWITCH_FINISHED), "{told:?}");
    assert_eq!(done.save, main);
    assert_eq!(state(&repo).lines["next"], main);
    assert_eq!(std::fs::read(dir.path().join("a.txt")).unwrap(), b"A");
}

#[cfg(feature = "fault-injection")]
#[test]
fn a_refused_line_step_still_tells_what_finishing_a_stopped_goto_left() {
    use crate::journal::{Action, Journal};
    use crate::warning::{NOT_WRITTEN_HERE, SWITCH_FINISHED};
    let (_dir, mut repo, _) = stopped_goto();
    // A journal can hold a name no system may write (one made elsewhere, say): finishing leaves
    // it unwritten, and must say so.
    let lock = repo.lock_for_change("test", &mut |_| {}).unwrap();
    let mut journal = Journal::read(&repo).unwrap().unwrap();
    let change = journal.actions[0].change;
    journal.actions.insert(
        0,
        Action {
            path: ".GIT".into(),
            change,
            expected: None,
        },
    );
    journal.write(&repo, &lock).unwrap();
    drop(lock);

    let head = repo.head().unwrap();
    let refused = made(&mut repo, "main").unwrap_err();
    assert!(
        matches!(refused, LineError::Name(NameError::Taken { .. })),
        "{refused:?}"
    );
    assert_ne!(repo.head().unwrap(), head, "the switch wasn't finished");
    let told = repo.store().warnings();
    let kinds: Vec<&str> = told.iter().map(|w| w.kind).collect();
    assert!(kinds.contains(&SWITCH_FINISHED), "{told:?}");
    assert!(kinds.contains(&NOT_WRITTEN_HERE), "{told:?}");
}

/// Stores a copy of `like` whose parent isn't in the store, and gives it.
fn orphan(repo: &mut Repository, like: Id) -> Id {
    let lock = repo.lock_for_change("save", &mut |_| {}).unwrap();
    let (head, mut entry) = repo.current().unwrap();
    let save = Save {
        parents: vec![Id::from_hex(&"5".repeat(64)).unwrap()],
        change_id: ChangeId::random().unwrap(),
        message: "orphan".into(),
        ..repo.store().save(&like, false).unwrap()
    };
    let mut packs = repo.new_packs().unwrap();
    let id = packs.add_record(ObjectKind::Save, &save.encode()).unwrap();
    entry.parent = Some(head);
    entry.command = "save".into();
    entry.folder_change = None;
    repo.commit(&lock, packs, &entry).unwrap();
    id
}

#[test]
fn a_thousand_tags_whose_history_cant_be_read_are_looked_through_once() {
    let (_dir, mut repo, _, two) = two_of_its_own();
    let lost = orphan(&mut repo, two);
    let names: Vec<String> = (0..1000).map(|i| format!("t{i:04}")).collect();
    let list = tags(&names.iter().map(|n| (n.as_str(), lost)).collect::<Vec<_>>());
    set(&mut repo, Some(&list), |_| {});
    let head = repo.head().unwrap();
    let before = repo.store().saves_read();
    let (done, asked) = deleted(&mut repo, "b", true);
    let read = repo.store().saves_read() - before;
    let e = done.unwrap_err();
    assert_eq!(
        e.to_string(),
        format!(
            "This project's Opalro data looks damaged: the tag t0000 names a save whose history \
             can't be read: object {} is not in the store.",
            "5".repeat(64)
        )
    );
    assert!(asked.is_empty());
    assert_eq!(repo.head().unwrap(), head);
    // The walk and the look read the four saves there are, each at most twice.
    assert!(read <= 8, "{read}");
}
