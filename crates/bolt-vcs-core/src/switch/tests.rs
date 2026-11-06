#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use super::*;
use crate::diary::ServerLines;
use crate::id::hash_content;
use crate::listing::EntryKind;
use crate::snapshot::{SaveOutcome, SaveRequest, save_folder};
use crate::test_support::{commit_records, plant_save, sample_records};
use crate::warning::Warning;
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

fn save_now(repo: &mut Repository) -> SaveOutcome {
    let who = who();
    let req = SaveRequest {
        message: "next",
        who: &who,
        command: "save",
        accept_crlf: false,
        git_excludes: None,
    };
    save_folder(repo, &req, &mut |_| {}).unwrap()
}

fn write_all(dir: &Path, files: &[(&str, &[u8])]) {
    for (path, bytes) in files {
        let at = dir.join(path);
        fs::create_dir_all(at.parent().unwrap()).unwrap();
        fs::write(at, bytes).unwrap();
    }
}

/// Every file below `dir` but `.bolt`, with what it holds.
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

fn holding(entries: &[(&str, &[u8])]) -> BTreeMap<String, Vec<u8>> {
    entries
        .iter()
        .map(|(p, b)| ((*p).to_owned(), b.to_vec()))
        .collect()
}

/// A project whose folder holds `files`, saved on `main`.
fn saved(files: &[(&str, &[u8])]) -> (tempfile::TempDir, Repository) {
    let dir = tempfile::tempdir().unwrap();
    let mut repo = Repository::init(dir.path(), &who()).unwrap();
    write_all(dir.path(), files);
    assert!(matches!(save_now(&mut repo), SaveOutcome::Saved { .. }));
    (dir, repo)
}

const OLD: &[(&str, &[u8])] = &[("a.txt", b"a"), ("gone/x.txt", b"x")];
const NEW: &[(&str, &[u8])] = &[("a.txt", b"A"), ("new/b.txt", b"b"), ("new/c.txt", b"c")];

/// A project on line `old`, its folder holding `OLD`, with line `main` holding `NEW`.
fn two_lines() -> (tempfile::TempDir, Repository) {
    let (dir, mut repo) = saved(OLD);
    let old = repo.current().unwrap().1.state.lines["main"];
    let entries: Vec<(&str, EntryKind, &[u8])> =
        NEW.iter().map(|&(p, b)| (p, EntryKind::File, b)).collect();
    plant_save(&mut repo, &entries);
    let lock = repo.lock_for_change("test", &mut |_| {}).unwrap();
    let (head, mut entry) = repo.current().unwrap();
    entry.parent = Some(head);
    entry.state.lines.insert("old".into(), old);
    entry.state.position = Position::OnLine("old".into());
    let packs = repo.new_packs().unwrap();
    repo.commit(&lock, packs, &entry).unwrap();
    (dir, repo)
}

#[test]
fn goto_a_line_moves_the_folder_and_the_head() {
    let (dir, mut repo) = two_lines();
    let target = Target::Line("main".into());
    let done = goto(&mut repo, &target, &who(), "goto", None, &mut |_| {}).unwrap();
    assert_eq!(done.position, Position::OnLine("main".into()));
    assert_eq!((done.applied.written, done.carried), (3, 0));
    assert_eq!(files(dir.path()), holding(NEW));
    assert!(!dir.path().join("gone").exists());
    assert_eq!(repo.current().unwrap().1.state.position, done.position);
    assert_eq!(Journal::read(&repo).unwrap(), None);
}

#[test]
fn goto_where_the_project_is_records_nothing() {
    let (_dir, mut repo) = two_lines();
    let head = repo.head().unwrap();
    let target = Target::Line("old".into());
    let done = goto(&mut repo, &target, &who(), "goto", None, &mut |_| {}).unwrap();
    assert_eq!(done.applied, Applied::default());
    assert_eq!(repo.head().unwrap(), head);
}

#[test]
fn a_switch_records_safety_copies_in_its_diary_entry() {
    let (dir, mut repo) = saved(&[("a.txt", b"a"), ("same.txt", b"s")]);
    write_all(dir.path(), &[("a.txt", b"mine"), ("n.txt", b"new")]);
    let lock = repo.lock_for_change("discard", &mut |_| {}).unwrap();
    let (head, entry) = repo.current().unwrap();
    let from = tip_of(&entry).unwrap();
    let drive = drive::probe(repo.tmp_folder()).unwrap();
    // A switch to the save the folder is on that throws the unsaved work away.
    let plan = writeout::plan(&repo, from, from, None, true, drive, None).unwrap();
    let s = Switch {
        head,
        from,
        state: entry.state.clone(),
        plan,
        drive,
    };
    let earlier = &mut Earlier::default();
    let (_, kept) = switch(&mut repo, &lock, s, &who(), "discard", earlier).unwrap();
    assert_eq!(kept, 2);
    let (_, entry) = repo.current().unwrap();
    let copies: Vec<(&str, Id)> = entry
        .safety_copies
        .iter()
        .map(|c| (c.path.as_str(), c.id))
        .collect();
    assert_eq!(
        copies,
        [
            ("a.txt", hash_content(b"mine")),
            ("n.txt", hash_content(b"new"))
        ]
    );
    for (copy, bytes) in entry.safety_copies.iter().zip([&b"mine"[..], b"new"]) {
        let mut out = Vec::new();
        repo.store().copy_content(&copy.id, &mut out).unwrap();
        assert_eq!(out, bytes);
    }
    assert_eq!(
        files(dir.path()),
        holding(&[("a.txt", b"a"), ("same.txt", b"s")])
    );
}

#[cfg(feature = "fault-injection")]
#[test]
fn a_goto_stopped_at_every_step_is_finished_or_never_happened() {
    use crate::fsio::fault;
    let mut finished_later = 0;
    for n in 1.. {
        assert!(n < 100, "the goto never finished");
        let (dir, mut repo) = two_lines();
        let target = Target::Line("main".into());
        fault::arm(n);
        let result = goto(&mut repo, &target, &who(), "goto", None, &mut |_| {});
        fault::disarm();
        drop(repo);
        // The next command finishes what was started, so the folder is the save of the line
        // the project is on, whole.
        let mut repo = Repository::open(dir.path()).unwrap();
        let outcome = save_now(&mut repo);
        assert!(
            matches!(outcome, SaveOutcome::NothingChanged { .. }),
            "fault {n}: the folder was half switched"
        );
        let position = repo.current().unwrap().1.state.position;
        let wanted = if position == Position::OnLine("main".into()) {
            NEW
        } else {
            assert_eq!(position, Position::OnLine("old".into()), "fault {n}");
            OLD
        };
        assert_eq!(files(dir.path()), holding(wanted), "fault {n}");
        assert_eq!(Journal::read(&repo).unwrap(), None, "fault {n}");
        if result.is_err() && wanted == NEW {
            finished_later += 1;
        }
        if result.is_ok() {
            assert_eq!(wanted, NEW);
            // Stops after each write and in removing the journal, finished by the save.
            assert!(finished_later >= 3, "only {finished_later} finished later");
            break;
        }
    }
}

#[test]
fn a_server_line_is_read_again_under_the_lock() {
    let (dir, mut repo) = two_lines();
    let main = repo.current().unwrap().1.state.lines["main"];
    let (tags, _) = sample_records();
    let servers = ServerLines {
        servers: BTreeMap::from([(
            "origin".to_owned(),
            BTreeMap::from([("x".to_owned(), main)]),
        )]),
    };
    commit_records(&mut repo, &tags, &servers);
    // Resolved before another command moved it.
    let stale = |line: &str| Target::ServerLine {
        server: "origin".into(),
        line: line.into(),
        save: Id([9; 32]),
    };
    let gone = goto(&mut repo, &stale("gone"), &who(), "goto", None, &mut |_| {});
    assert!(matches!(
        gone,
        Err(SwitchError::Resolve(ResolveError::NotFound(_)))
    ));
    let done = goto(&mut repo, &stale("x"), &who(), "goto", None, &mut |_| {}).unwrap();
    assert_eq!(done.position, Position::AtSave(main));
    assert_eq!(files(dir.path()), holding(NEW));
}

#[test]
fn a_command_gives_one_warning_for_its_placeholders_and_one_for_what_it_left_unwritten() {
    let names = |n: &[&str]| n.iter().map(|s| (*s).to_owned()).collect::<Vec<String>>();
    let plan = Plan {
        placeholders: names(&["p1"]),
        not_written: names(&["n1"]),
        ..Plan::default()
    };
    let applied = Applied {
        placeholders: names(&["p2"]),
        not_written: names(&["n2"]),
        warnings: vec![
            writeout::placeholders_warning(&names(&["p2"])),
            writeout::not_written_warning(&names(&["n2"])),
        ],
        ..Applied::default()
    };
    let mut earlier = Earlier {
        placeholders: names(&["p0"]),
        not_written: names(&["n0"]),
    };
    let done = switch_warnings(plan, applied, &mut earlier);
    let of =
        |kind: &str| -> Vec<&Warning> { done.warnings.iter().filter(|w| w.kind == kind).collect() };
    for (kind, paths) in [
        (warning::PLACEHOLDERS_WRITTEN, ["p0", "p1", "p2"]),
        (warning::NOT_WRITTEN_HERE, ["n0", "n1", "n2"]),
    ] {
        let found = of(kind);
        assert_eq!(found.len(), 1, "{kind}: {found:?}");
        for path in paths {
            assert!(
                found[0].message.contains(path),
                "{kind}: {}",
                found[0].message
            );
        }
    }
    assert_eq!(done.placeholders, names(&["p0", "p1", "p2"]));
    assert_eq!(done.not_written, names(&["n0", "n1", "n2"]));
    assert_eq!(earlier, Earlier::default());
}

#[cfg(feature = "fault-injection")]
#[test]
fn a_file_edited_after_the_discard_planned_is_left_as_edited() {
    use crate::fsio::fault;
    let (dir, mut repo) = saved(&[("a.txt", b"a"), ("b.txt", b"b")]);
    write_all(dir.path(), &[("a.txt", b"mine"), ("b.txt", b"mine too")]);
    let at = dir.path().join("a.txt");
    fault::act_at("discard:planned", move || fs::write(at, b"edited").unwrap());
    let done = discard(&mut repo, None, &who(), "discard", None, &mut |_| {}).unwrap();
    assert_eq!(done.applied.changed_since_plan, ["a.txt"]);
    assert_eq!(done.restored, ["b.txt"]);
    assert_eq!(fs::read(dir.path().join("a.txt")).unwrap(), b"edited");
    assert_eq!(fs::read(dir.path().join("b.txt")).unwrap(), b"b");
}
