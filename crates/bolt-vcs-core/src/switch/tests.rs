#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use super::*;
use crate::diary::ServerLines;
use crate::id::hash_content;
use crate::listing::EntryKind;
use crate::snapshot::{SaveOutcome, SaveRequest, save_folder};
use crate::test_support::{commit_records, plant_save, sample_records, symlink};
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
        message: Some("next"),
        who: &who,
        command: "save",
        accept_crlf: false,
        git_excludes: None,
        new_line: None,
        only: None,
        amend: false,
    };
    save_folder(repo, &req, &mut |_| {}, &mut |_| true).unwrap()
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
fn goto_records_the_save_it_left_and_the_one_it_went_to() {
    let (_dir, mut repo) = two_lines();
    let lines = repo.current().unwrap().1.state.lines;
    let target = Target::Line("main".into());
    goto(&mut repo, &target, &who(), "goto", None, &mut |_| {}).unwrap();
    let change = repo.current().unwrap().1.folder_change;
    let switched = FolderChange {
        from: lines["old"],
        to: lines["main"],
    };
    assert_eq!(change, Some(switched));
}

#[test]
fn a_goto_writes_a_link_through_a_folder_the_folder_already_held() {
    let (dir, mut repo) = saved(&[("a.txt", b"a")]);
    let root = dir.path();
    if !symlink(Path::new("a.txt"), &root.join("probe")) {
        return;
    }
    fs::remove_file(root.join("probe")).unwrap();
    let old = repo.current().unwrap().1.state.lines["main"];
    plant_save(
        &mut repo,
        &[
            ("a.txt", EntryKind::File, b"a"),
            ("d/x.txt", EntryKind::File, b"x"),
            ("l", EntryKind::Symlink, b"d/x.txt"),
        ],
    );
    let main = repo.current().unwrap().1.state.lines["main"];
    let lock = repo.lock_for_change("test", &mut |_| {}).unwrap();
    let (head, mut entry) = repo.current().unwrap();
    entry.parent = Some(head);
    entry.state.lines.insert("old".into(), old);
    entry.state.position = Position::OnLine("old".into());
    let packs = repo.new_packs().unwrap();
    repo.commit(&lock, packs, &entry).unwrap();
    drop(lock);
    // The folder already holds the target's d, so the goto writes only the link through it.
    write_all(root, &[("d/x.txt", b"x")]);
    let target = Target::Line("main".into());
    let done = goto(&mut repo, &target, &who(), "goto", None, &mut |_| {}).unwrap();
    assert!(done.applied.placeholders.is_empty(), "{done:?}");
    let link = fs::symlink_metadata(root.join("l")).unwrap();
    assert!(link.file_type().is_symlink(), "written as a plain file");
    assert_eq!(fs::read(root.join("l")).unwrap(), b"x");
    let change = repo.current().unwrap().1.folder_change.unwrap();
    assert_eq!(change.to, main);
}

#[test]
fn a_goto_that_writes_nothing_records_no_folder_change() {
    let (_dir, mut repo) = two_lines();
    let old = repo.current().unwrap().1.state.lines["old"];
    let done = goto(
        &mut repo,
        &Target::Save(old),
        &who(),
        "goto",
        None,
        &mut |_| {},
    )
    .unwrap();
    assert_eq!(done.position, Position::AtSave(old));
    let (_, entry) = repo.current().unwrap();
    assert_eq!(
        (entry.command.as_str(), entry.folder_change),
        ("goto", None)
    );
}

#[test]
fn discard_records_its_kept_save_then_the_current_one() {
    let (dir, mut repo) = saved(&[("a.txt", b"a"), ("same.txt", b"s")]);
    let save = tip_of(&repo.current().unwrap().1).unwrap();
    write_all(dir.path(), &[("a.txt", b"mine"), ("n.txt", b"new")]);
    let done = discard(&mut repo, None, &who(), "discard", None, &mut |_| {}).unwrap();
    assert_eq!(done.safety_copies, 2);
    let (_, entry) = repo.current().unwrap();
    assert_eq!(entry.folder_change, done.folder_change);
    let change = entry.folder_change.unwrap();
    assert_eq!(change.to, save);
    let store = repo.store();
    let kept = store.save(&change.from, false).unwrap();
    assert_eq!(kept.parents, [save]);
    let top = store.listing(&kept.root).unwrap();
    let held: Vec<(&str, Id)> = top
        .entries()
        .iter()
        .map(|e| (e.name.as_str(), e.id))
        .collect();
    let thrown_away = [
        ("a.txt", hash_content(b"mine")),
        ("n.txt", hash_content(b"new")),
        ("same.txt", hash_content(b"s")),
    ];
    assert_eq!(held, thrown_away);
    let mut mine = Vec::new();
    store
        .copy_content(&hash_content(b"mine"), &mut mine)
        .unwrap();
    assert_eq!(mine, b"mine");
    assert_eq!(
        files(dir.path()),
        holding(&[("a.txt", b"a"), ("same.txt", b"s")])
    );
}

#[test]
fn a_link_renamed_only_in_letter_case_is_left_as_it_is() {
    let (dir, mut repo) = saved(&[("a.txt", b"a")]);
    let root = dir.path();
    // A target outside the project, which writing out could only put back as a plain file.
    if !symlink(&std::env::temp_dir(), &root.join("node")) {
        return;
    }
    assert!(matches!(save_now(&mut repo), SaveOutcome::Saved { .. }));
    fs::rename(root.join("node"), root.join("Node")).unwrap();
    write_all(root, &[("a.txt", b"mine")]);
    let ignores_case = {
        let _lock = repo.lock_for_change("probe", &mut |_| {}).unwrap();
        repo.drive().unwrap().ignores_case
    };
    let done = discard(&mut repo, None, &who(), "discard", None, &mut |_| {}).unwrap();
    assert_eq!(done.links_left, ["Node"]);
    let left = fs::symlink_metadata(root.join("Node")).unwrap();
    assert!(left.file_type().is_symlink());
    assert_eq!(fs::read(root.join("a.txt")).unwrap(), b"a");
    if ignores_case {
        // The saved `node` is `Node` to this drive: writing it back would replace the link left.
        assert!(done.applied.placeholders.is_empty(), "{done:?}");
        let mut names: Vec<String> = fs::read_dir(root)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(names, [".bolt", "Node", "a.txt"]);
    } else {
        assert_eq!(done.applied.placeholders, ["node"]);
    }
}

/// Makes `link` lead to the folder `target`: a link, or on Windows a junction, which needs no
/// rights and which Bolt reads as a link too.
fn folder_link(target: &Path, link: &Path) {
    #[cfg(unix)]
    std::os::unix::fs::symlink(target, link).unwrap();
    #[cfg(windows)]
    crate::test_support::junction(link, target);
}

fn remove_link(link: &Path) {
    #[cfg(unix)]
    fs::remove_file(link).unwrap();
    // A junction is a folder to Windows.
    #[cfg(windows)]
    fs::remove_dir(link).unwrap();
}

#[test]
fn a_saved_folder_replaced_by_a_link_spelled_otherwise_is_left_once() {
    let (dir, mut repo) = saved(&[("a.txt", b"a"), ("node/x", b"x")]);
    let root = dir.path();
    let ignores_case = {
        let _lock = repo.lock_for_change("probe", &mut |_| {}).unwrap();
        repo.drive().unwrap().ignores_case
    };
    if !ignores_case {
        return;
    }
    fs::remove_dir_all(root.join("node")).unwrap();
    folder_link(&std::env::temp_dir(), &root.join("Node"));
    let done = discard(&mut repo, None, &who(), "discard", None, &mut |_| {}).unwrap();
    // The saved folder would go back where the link is, so both stay as they are.
    assert_eq!(done.links_left, ["Node"]);
    assert_eq!(done.folder_change, None);
    let told: Vec<&Warning> = done
        .applied
        .warnings
        .iter()
        .filter(|w| w.kind == warning::LINKS_LEFT)
        .collect();
    assert_eq!(told.len(), 1, "{told:?}");
    assert!(told[0].message.contains("1 unsaved link"), "{told:?}");
    assert!(fs::read_link(root.join("Node")).is_ok());
}

#[test]
fn a_saved_link_pointed_elsewhere_outside_is_left_and_no_placeholder_is_told() {
    let (dir, mut repo) = saved(&[("a.txt", b"a")]);
    let root = dir.path();
    let (one, two) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    folder_link(one.path(), &root.join("out"));
    assert!(matches!(save_now(&mut repo), SaveOutcome::Saved { .. }));
    remove_link(&root.join("out"));
    folder_link(two.path(), &root.join("out"));
    // The saved link, written back, would be a placeholder: it leads out too.
    let done = discard(&mut repo, None, &who(), "discard", None, &mut |_| {}).unwrap();
    assert_eq!(done.links_left, ["out"]);
    assert!(done.applied.placeholders.is_empty(), "{done:?}");
    let warned = |kind: &str| done.applied.warnings.iter().any(|w| w.kind == kind);
    assert!(!warned(warning::PLACEHOLDERS_WRITTEN), "{done:?}");
    let now = fs::read_link(root.join("out")).unwrap();
    assert!(now.ends_with(two.path().file_name().unwrap()), "{now:?}");
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
        // A fault in flushing the moved head is only a warning, and the goto goes on.
        let warnings = repo.store().warnings();
        let unflushed = warnings.iter().any(|w| w.kind == warning::HEAD_NOT_FLUSHED);
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
        if result.is_ok() && !unflushed {
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

#[test]
fn the_links_left_warning_reads_right_for_one_link_and_for_several() {
    let one = links_left_warning(&["out".to_owned()]).message;
    assert!(
        one.starts_with(
            "Didn't throw away 1 unsaved link: an undo could bring it back only as a plain file \
             holding its target, since it may lead out"
        ),
        "{one}"
    );
    assert!(one.contains("\n  out\n"), "{one}");
    assert!(one.contains("through a name Bolt never writes"), "{one}");
    assert!(
        one.ends_with("Delete it yourself if you don't want it."),
        "{one}"
    );
    let two = links_left_warning(&["a".to_owned(), "b".to_owned()]).message;
    assert!(
        two.starts_with(
            "Didn't throw away 2 unsaved links: an undo could bring them back only as plain files \
             holding their targets, since they may lead out"
        ),
        "{two}"
    );
    assert!(
        two.ends_with("Delete them yourself if you don't want them."),
        "{two}"
    );
}
