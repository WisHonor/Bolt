#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use super::*;
use crate::drive;
use crate::history::{current_save, tip_of};
use crate::journal::{Change, Content, Expected};
use crate::listing::EntryKind;
use crate::save::Person;
use crate::test_support::{Written, names, noise, plant_save, written};
use crate::warning;
use std::fs;

fn who() -> Person {
    Person {
        name: "Ada".into(),
        email: "ada@example.com".into(),
        time_ms: 1_700_000_000_000,
        tz_offset_min: 0,
    }
}

fn project() -> (tempfile::TempDir, Repository) {
    let dir = tempfile::tempdir().unwrap();
    let repo = Repository::init(dir.path(), &who()).unwrap();
    (dir, repo)
}

/// A save is the save's ID and its diary entry's.
type Saved = (Id, Id);

/// Commits a save holding `entries`, whatever the folder holds.
fn planted(repo: &mut Repository, entries: &[(&str, EntryKind, &[u8])]) -> Saved {
    plant_save(repo, entries);
    let (head, entry) = repo.current().unwrap();
    (tip_of(&entry).unwrap(), head)
}

fn files(
    entries: &[(&'static str, &'static [u8])],
) -> Vec<(&'static str, EntryKind, &'static [u8])> {
    entries
        .iter()
        .map(|&(path, bytes)| (path, EntryKind::File, bytes))
        .collect()
}

fn journal(target: Id, plan: &Plan) -> Journal {
    let journal = Journal {
        target,
        from: Id([0; 32]),
        actions: plan.actions.clone(),
    };
    // Every plan must be a journal that reads back.
    assert_eq!(Journal::decode(&journal.encode()).unwrap(), journal);
    journal
}

fn planned(repo: &mut Repository, from: Id, to: Id, remove_new: bool) -> Plan {
    let _lock = repo.lock_for_change("goto", &mut |_| {}).unwrap();
    let drive = drive::probe(repo.tmp_folder()).unwrap();
    plan(repo, from, to, None, remove_new, drive, None).unwrap()
}

fn applied(repo: &mut Repository, journal: &Journal) -> Applied {
    let lock = repo.lock_for_change("goto", &mut |_| {}).unwrap();
    let drive = drive::probe(repo.tmp_folder()).unwrap();
    apply(repo, &lock, journal, drive).unwrap()
}

/// Plans and carries out a switch from the save `from` to `to`.
fn switch(repo: &mut Repository, from: Id, to: Saved) -> (Plan, Applied) {
    let plan = planned(repo, from, to.0, false);
    let done = applied(repo, &journal(to.1, &plan));
    (plan, done)
}

fn this_drive(repo: &mut Repository) -> Drive {
    let _lock = repo.lock_for_change("probe", &mut |_| {}).unwrap();
    drive::probe(repo.tmp_folder()).unwrap()
}

/// The names in the folder `dir`, `.bolt` aside, sorted.
fn names_in(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n != ".bolt")
        .collect();
    names.sort();
    names
}

fn kinds(warnings: &[Warning]) -> Vec<&'static str> {
    warnings.iter().map(|w| w.kind).collect()
}

fn content(bytes: &[u8]) -> Content {
    Content {
        id: crate::id::hash_content(bytes),
        size: bytes.len() as u64,
    }
}

#[test]
fn a_planted_save_naming_git_or_bolt_spellings_writes_none_of_them() {
    let (dir, mut repo) = project();
    let empty = current_save(&repo).unwrap();
    let to = planted(
        &mut repo,
        &files(&[
            (".git/config", b"evil"),
            (".GIT/hooks/post-checkout", b"evil"),
            ("git~1/x", b"evil"),
            (".git. /x", b"evil"),
            (".g\u{200c}it/x", b"evil"),
            ("sub/.Git/hooks/x", b"evil"),
            ("ok.txt", b"fine"),
        ]),
    );
    let (plan, done) = switch(&mut repo, empty, to);
    let mut refused = vec![
        ".GIT",
        ".g\u{200c}it",
        ".git",
        ".git. ",
        "git~1",
        "sub/.Git",
    ];
    refused.sort_unstable();
    assert_eq!(plan.not_written, refused);
    assert_eq!(names_in(dir.path()), ["ok.txt", "sub"]);
    assert!(names_in(&dir.path().join("sub")).is_empty());
    assert!(done.changed_since_plan.is_empty());

    // A journal made by hand, never decoded, gets the same checks when it is carried out.
    let evil = content(b"fine");
    let by_hand = Journal {
        target: to.1,
        from: to.1,
        actions: [
            ".BOLT/x",
            ".bolt/tmp/x",
            "bolt~1",
            ".git",
            "a/.Git",
            "a/git~1/hooks/x",
            "..",
            "a/../b",
        ]
        .iter()
        .map(|path| Action {
            path: (*path).into(),
            change: Change::File(evil),
            expected: None,
        })
        .collect(),
    };
    let done = applied(&mut repo, &by_hand);
    assert_eq!(done.written, 0);
    assert!(kinds(&done.warnings).contains(&warning::NOT_WRITTEN_HERE));
    assert_eq!(names_in(dir.path()), ["ok.txt", "sub"]);
    assert!(!dir.path().join(".bolt/x").exists());
    assert!(!dir.path().join(".bolt/tmp/x").exists());
}

/// Makes `link` a link to the folder `target` that this account can make: a symlink on Unix,
/// a junction on Windows.
fn folder_link(target: &Path, link: &Path) {
    #[cfg(unix)]
    std::os::unix::fs::symlink(target, link).unwrap();
    #[cfg(windows)]
    crate::test_support::junction(link, target);
}

#[test]
fn a_link_planted_as_a_folder_name_cant_redirect_a_later_file() {
    let (dir, mut repo) = project();
    let outside = tempfile::tempdir().unwrap();
    let empty = current_save(&repo).unwrap();
    let target = outside.path().to_string_lossy().replace('\\', "/");
    let to = planted(
        &mut repo,
        &[
            ("A", EntryKind::Symlink, target.as_bytes()),
            ("a/b", EntryKind::File, b"payload"),
        ],
    );
    let drive = this_drive(&mut repo);
    let (plan, _) = switch(&mut repo, empty, to);
    assert!(names_in(outside.path()).is_empty());
    if drive.ignores_case {
        assert_eq!(plan.not_written, ["A", "a"]);
        assert!(names_in(dir.path()).is_empty());
    } else {
        // The link leads out of the project, so it is a file holding its target.
        assert_eq!(plan.placeholders, ["A"]);
        assert_eq!(fs::read(dir.path().join("A")).unwrap(), target.as_bytes());
        assert!(fs::symlink_metadata(dir.path().join("a")).unwrap().is_dir());
        assert_eq!(fs::read(dir.path().join("a/b")).unwrap(), b"payload");
    }

    // A link put on disk where a folder is to be written is never written through.
    folder_link(outside.path(), &dir.path().join("c"));
    let by_hand = Journal {
        target: to.1,
        from: to.1,
        actions: vec![
            Action {
                path: "c".into(),
                change: Change::MakeFolder,
                expected: None,
            },
            Action {
                path: "c/b".into(),
                change: Change::File(content(b"payload")),
                expected: None,
            },
        ],
    };
    let done = applied(&mut repo, &by_hand);
    assert_eq!(done.changed_since_plan, ["c"]);
    assert_eq!(done.written, 0);
    assert!(names_in(outside.path()).is_empty());
    assert!(
        fs::symlink_metadata(dir.path().join("c"))
            .unwrap()
            .file_type()
            .is_symlink()
            || cfg!(windows)
    );
}

#[test]
fn a_case_only_rename_is_written_on_a_case_blind_drive() {
    let (dir, mut repo) = project();
    let empty = current_save(&repo).unwrap();
    let a = planted(
        &mut repo,
        &files(&[
            ("Readme.md", b"old"),
            ("Same.txt", b"same"),
            ("Docs/x.txt", b"x"),
        ]),
    );
    switch(&mut repo, empty, a);
    let b = planted(
        &mut repo,
        &files(&[
            ("README.md", b"new"),
            ("SAME.txt", b"same"),
            ("docs/X.txt", b"x2"),
        ]),
    );
    let drive = this_drive(&mut repo);
    let (plan, done) = switch(&mut repo, a.0, b);
    assert!(plan.collisions.is_empty() && plan.not_written.is_empty());
    assert!(done.changed_since_plan.is_empty(), "{done:?}");
    let top = names_in(dir.path());
    if drive.ignores_case {
        // A folder keeps the spelling it had; files take the new one.
        assert_eq!(top, ["Docs", "README.md", "SAME.txt"]);
    } else {
        assert_eq!(top, ["README.md", "SAME.txt", "docs"]);
    }
    assert_eq!(fs::read(dir.path().join("README.md")).unwrap(), b"new");
    assert_eq!(fs::read(dir.path().join("SAME.txt")).unwrap(), b"same");
    // A file in a folder renamed with it takes its new spelling too.
    let folder = if drive.ignores_case { "Docs" } else { "docs" };
    assert_eq!(names_in(&dir.path().join(folder)), ["X.txt"]);
    assert_eq!(
        fs::read(dir.path().join(folder).join("X.txt")).unwrap(),
        b"x2"
    );
}

#[cfg(windows)]
#[test]
fn a_file_held_open_is_left_and_the_next_run_finishes() {
    use std::os::windows::fs::OpenOptionsExt;
    let (dir, mut repo) = project();
    let empty = current_save(&repo).unwrap();
    let a = planted(&mut repo, &files(&[("x", b"old x"), ("y", b"old y")]));
    switch(&mut repo, empty, a);
    let b = planted(&mut repo, &files(&[("x", b"new x"), ("y", b"new y")]));
    let plan = planned(&mut repo, a.0, b.0, false);
    let journal = journal(b.1, &plan);
    // Shared with nobody, as an editor or virus scanner may hold a file.
    let held = fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(dir.path().join("x"))
        .unwrap();
    let done = applied(&mut repo, &journal);
    assert_eq!(done.in_use, ["x"]);
    assert!(kinds(&done.warnings).contains(&warning::FILES_IN_USE));
    assert_eq!(fs::read(dir.path().join("y")).unwrap(), b"new y");
    drop(held);
    assert_eq!(fs::read(dir.path().join("x")).unwrap(), b"old x");

    let again = applied(&mut repo, &journal);
    assert!(again.in_use.is_empty(), "{again:?}");
    assert_eq!(again.written, 1);
    assert_eq!(fs::read(dir.path().join("x")).unwrap(), b"new x");
}

#[test]
fn twins_on_a_case_blind_drive_are_both_left_unwritten() {
    let (dir, mut repo) = project();
    let empty = current_save(&repo).unwrap();
    let to = planted(
        &mut repo,
        &files(&[
            ("README", b"1"),
            ("readme", b"2"),
            ("Dir/a", b"3"),
            ("dir/b", b"4"),
            ("other", b"5"),
        ]),
    );
    let drive = this_drive(&mut repo);
    let (plan, done) = switch(&mut repo, empty, to);
    if drive.ignores_case {
        assert_eq!(plan.not_written, ["Dir", "README", "dir", "readme"]);
        assert_eq!(names_in(dir.path()), ["other"]);
        assert!(kinds(&done.warnings).is_empty());
    } else {
        assert!(plan.not_written.is_empty());
        assert_eq!(
            names_in(dir.path()),
            ["Dir", "README", "dir", "other", "readme"]
        );
    }
}

#[test]
fn links_leaving_the_project_become_placeholders() {
    let (dir, mut repo) = project();
    let empty = current_save(&repo).unwrap();
    let link = |path, target: &'static [u8]| (path, EntryKind::Symlink, target);
    let to = planted(
        &mut repo,
        &[
            ("inside", EntryKind::File, b"i"),
            link("up", b"../outside"),
            link("abs", b"/etc/passwd"),
            link("drive", b"C:/Windows"),
            link("unc", b"//server/share"),
            link("deep/ok", b"../inside"),
            link("deep/esc", b"../../x"),
            // Through a link, `..` could lead anywhere.
            link("via", b"deep/ok/../x"),
        ],
    );
    let (plan, done) = switch(&mut repo, empty, to);
    assert_eq!(
        plan.placeholders,
        ["abs", "deep/esc", "drive", "unc", "up", "via"]
    );
    for (path, target) in [
        ("up", "../outside"),
        ("abs", "/etc/passwd"),
        ("drive", "C:/Windows"),
        ("unc", "//server/share"),
        ("deep/esc", "../../x"),
        ("via", "deep/ok/../x"),
    ] {
        let at = dir.path().join(path);
        assert!(fs::symlink_metadata(&at).unwrap().is_file(), "{path}");
        assert_eq!(fs::read(&at).unwrap(), target.as_bytes(), "{path}");
    }
    let ok = fs::symlink_metadata(dir.path().join("deep/ok")).unwrap();
    if ok.file_type().is_symlink() {
        let read = fs::read_link(dir.path().join("deep/ok")).unwrap();
        let want = if cfg!(windows) {
            "..\\inside"
        } else {
            "../inside"
        };
        assert_eq!(read, Path::new(want));
    } else {
        // Only where this account can't make links.
        assert!(cfg!(windows) && kinds(&done.warnings).contains(&warning::PLACEHOLDERS_WRITTEN));
    }
}

#[cfg(unix)]
#[test]
fn modes_are_exactly_0644_and_0755() {
    use std::os::unix::fs::PermissionsExt;
    let (dir, mut repo) = project();
    let mode = |path: &str| {
        fs::symlink_metadata(dir.path().join(path))
            .unwrap()
            .permissions()
            .mode()
            & 0o7777
    };
    let empty = current_save(&repo).unwrap();
    let a = planted(
        &mut repo,
        &[
            ("plain", EntryKind::File, b"p"),
            ("tool", EntryKind::Executable, b"t"),
            ("flip", EntryKind::File, b"f"),
        ],
    );
    switch(&mut repo, empty, a);
    assert_eq!(
        (mode("plain"), mode("tool"), mode("flip")),
        (0o644, 0o755, 0o644)
    );
    // What the file had before doesn't count.
    fs::set_permissions(dir.path().join("plain"), fs::Permissions::from_mode(0o666)).unwrap();
    // A mode change writes the file anew, so another name linked to it keeps its mode.
    let outside = tempfile::tempdir().unwrap();
    let linked = outside.path().join("flip");
    fs::hard_link(dir.path().join("flip"), &linked).unwrap();
    let b = planted(
        &mut repo,
        &[
            ("plain", EntryKind::File, b"p2"),
            ("tool", EntryKind::Executable, b"t"),
            ("flip", EntryKind::Executable, b"f"),
        ],
    );
    let (_, done) = switch(&mut repo, a.0, b);
    assert!(done.changed_since_plan.is_empty(), "{done:?}");
    assert_eq!(
        (mode("plain"), mode("tool"), mode("flip")),
        (0o644, 0o755, 0o755)
    );
    let theirs = fs::metadata(&linked).unwrap().permissions().mode() & 0o7777;
    assert_eq!(theirs, 0o644);
}

#[test]
fn a_file_changed_after_the_plan_is_left_alone() {
    let (dir, mut repo) = project();
    let empty = current_save(&repo).unwrap();
    let a = planted(
        &mut repo,
        &files(&[("x", b"old"), ("gone", b"bye"), ("same", b"s")]),
    );
    switch(&mut repo, empty, a);
    let b = planted(&mut repo, &files(&[("x", b"new"), ("same", b"s")]));
    let plan = planned(&mut repo, a.0, b.0, false);
    fs::write(dir.path().join("x"), b"mine!").unwrap();
    fs::write(dir.path().join("gone"), b"mine too").unwrap();
    let done = applied(&mut repo, &journal(b.1, &plan));
    assert_eq!(done.changed_since_plan, ["gone", "x"]);
    assert!(kinds(&done.warnings).contains(&warning::CHANGED_SINCE_PLAN));
    assert_eq!(fs::read(dir.path().join("x")).unwrap(), b"mine!");
    assert_eq!(fs::read(dir.path().join("gone")).unwrap(), b"mine too");
}

#[test]
fn only_a_folder_gone_or_made_a_file_counts_as_changed_since_the_plan() {
    use io::ErrorKind::{NotADirectory, NotFound, PermissionDenied};
    assert!(folder_changed(&io::Error::from(NotFound)));
    assert!(folder_changed(&io::Error::from(NotADirectory)));
    // A passing failure stops the command, so no copy is skipped for a file still there.
    assert!(!folder_changed(&io::Error::from(PermissionDenied)));
    assert!(!folder_changed(&io::Error::other("too many open files")));
}

/// Carries out the switch to `to`, while another program writes `bytes` to `path` as soon as
/// the switch reaches `point`; what it left.
#[cfg(feature = "fault-injection")]
fn raced(
    repo: &mut Repository,
    from: Id,
    to: Saved,
    point: &'static str,
    path: &'static str,
    bytes: &'static [u8],
) -> Applied {
    let plan = planned(repo, from, to.0, false);
    let at = repo.root().join(path);
    // A modified time far from now, so a same-size save can't look unchanged within one tick.
    crate::fsio::fault::act_at(point, move || {
        fs::write(&at, bytes).unwrap();
        let old = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000_000);
        fs::File::options()
            .write(true)
            .open(&at)
            .unwrap()
            .set_modified(old)
            .unwrap();
    });
    applied(repo, &journal(to.1, &plan))
}

#[cfg(feature = "fault-injection")]
#[test]
fn a_file_saved_while_its_new_contents_are_written_is_left() {
    let (dir, mut repo) = project();
    let empty = current_save(&repo).unwrap();
    // Made at a path the plan saw empty.
    let a = planted(&mut repo, &files(&[("x", b"old")]));
    let done = raced(&mut repo, empty, a, "switch:written", "x", b"theirs");
    assert_eq!(done.changed_since_plan, ["x"]);
    assert_eq!(fs::read(dir.path().join("x")).unwrap(), b"theirs");
    // Saved over a file the plan saw, keeping its size.
    fs::write(dir.path().join("x"), b"old").unwrap();
    let b = planted(&mut repo, &files(&[("x", b"new")]));
    let done = raced(&mut repo, a.0, b, "switch:written", "x", b"mxn");
    assert_eq!(done.changed_since_plan, ["x"]);
    assert_eq!(fs::read(dir.path().join("x")).unwrap(), b"mxn");
    assert!(names_in(&dir.path().join(".bolt/tmp")).is_empty());
}

#[cfg(feature = "fault-injection")]
#[test]
fn a_file_saved_after_the_check_before_its_removal_is_left() {
    let (dir, mut repo) = project();
    let empty = current_save(&repo).unwrap();
    let a = planted(&mut repo, &files(&[("x", b"old"), ("f", b"a file")]));
    switch(&mut repo, empty, a);
    // Removed.
    let b = planted(&mut repo, &files(&[("f", b"a file")]));
    let done = raced(&mut repo, a.0, b, "switch:removing", "x", b"mine, edited");
    assert_eq!(done.changed_since_plan, ["x"]);
    assert_eq!(done.removed, 0);
    assert_eq!(fs::read(dir.path().join("x")).unwrap(), b"mine, edited");
    // Given a folder's place.
    let c = planted(&mut repo, &files(&[("f/in", b"in")]));
    let done = raced(&mut repo, b.0, c, "switch:removing", "f", b"mine, edited");
    assert_eq!(done.changed_since_plan, ["f"]);
    assert_eq!(fs::read(dir.path().join("f")).unwrap(), b"mine, edited");
}

#[cfg(all(unix, feature = "fault-injection"))]
#[test]
fn a_link_changed_after_the_check_before_its_removal_is_left() {
    use std::os::unix::fs::symlink;
    let (dir, mut repo) = project();
    let at = dir.path().join("l");
    let empty = current_save(&repo).unwrap();
    let a = planted(&mut repo, &[("l", EntryKind::Symlink, b"t1")]);
    switch(&mut repo, empty, a);
    assert_eq!(fs::read_link(&at).unwrap(), Path::new("t1"));
    // Another link put in its place: re-pointed by another program just before.
    let b = planted(&mut repo, &[("l", EntryKind::Symlink, b"t2")]);
    let plan = planned(&mut repo, a.0, b.0, false);
    let link = at.clone();
    crate::fsio::fault::act_at("switch:removing", move || {
        fs::remove_file(&link).unwrap();
        symlink("theirs", &link).unwrap();
    });
    let done = applied(&mut repo, &journal(b.1, &plan));
    assert_eq!(done.changed_since_plan, ["l"]);
    assert_eq!(fs::read_link(&at).unwrap(), Path::new("theirs"));
    // A file put in its place: swapped for a file just before.
    fs::remove_file(&at).unwrap();
    symlink("t2", &at).unwrap();
    let c = planted(&mut repo, &files(&[("l", b"a file")]));
    let plan = planned(&mut repo, b.0, c.0, false);
    let link = at.clone();
    crate::fsio::fault::act_at("switch:removing", move || {
        fs::remove_file(&link).unwrap();
        fs::write(&link, b"mine").unwrap();
    });
    let done = applied(&mut repo, &journal(c.1, &plan));
    assert_eq!(done.changed_since_plan, ["l"]);
    assert_eq!(fs::read(&at).unwrap(), b"mine");
}

#[cfg(windows)]
#[test]
fn a_stored_link_target_holding_a_backslash_is_not_written_on_windows() {
    let (dir, mut repo) = project();
    let empty = current_save(&repo).unwrap();
    let to = planted(
        &mut repo,
        &[
            ("l", EntryKind::Symlink, b"a\\b"),
            ("m", EntryKind::Symlink, b"..\\..\\x"),
        ],
    );
    let (plan, _) = switch(&mut repo, empty, to);
    assert_eq!(plan.placeholders, ["l", "m"]);
    for (path, target) in [("l", "a\\b"), ("m", "..\\..\\x")] {
        let at = dir.path().join(path);
        assert!(fs::symlink_metadata(&at).unwrap().is_file());
        assert_eq!(fs::read(&at).unwrap(), target.as_bytes());
    }
}

#[test]
fn a_folder_name_windows_would_misread_is_not_written_there() {
    let (dir, mut repo) = project();
    let empty = current_save(&repo).unwrap();
    let to = planted(
        &mut repo,
        &files(&[("a\\b/c", b"1"), ("x:y/z", b"2"), ("ok/c", b"3")]),
    );
    let (plan, _) = switch(&mut repo, empty, to);
    if cfg!(windows) {
        assert_eq!(plan.not_written, ["a\\b", "x:y"]);
        assert_eq!(names_in(dir.path()), ["ok"]);
    } else {
        assert!(plan.not_written.is_empty());
        assert_eq!(names_in(dir.path()), ["a\\b", "ok", "x:y"]);
    }
    // Nor from a journal made by hand.
    let by_hand = Journal {
        target: to.1,
        from: to.1,
        actions: vec![
            Action {
                path: "n\\m".into(),
                change: Change::MakeFolder,
                expected: None,
            },
            Action {
                path: "n\\m/c".into(),
                change: Change::File(content(b"1")),
                expected: None,
            },
        ],
    };
    let done = applied(&mut repo, &by_hand);
    assert_eq!(done.written, usize::from(!cfg!(windows)));
    assert_eq!(dir.path().join("n\\m").exists(), !cfg!(windows));
}

#[test]
fn a_folder_becomes_a_file_and_back_and_a_journal_carried_out_twice_changes_nothing_more() {
    let (dir, mut repo) = project();
    let empty = current_save(&repo).unwrap();
    let a = planted(
        &mut repo,
        &files(&[("a/x", b"1"), ("a/y/z", b"2"), ("f", b"3")]),
    );
    switch(&mut repo, empty, a);
    let b = planted(
        &mut repo,
        &files(&[("a", b"a file"), ("f/inside", b"a folder")]),
    );
    let (plan, done) = switch(&mut repo, a.0, b);
    assert!(done.changed_since_plan.is_empty(), "{done:?}");
    assert_eq!(fs::read(dir.path().join("a")).unwrap(), b"a file");
    assert_eq!(fs::read(dir.path().join("f/inside")).unwrap(), b"a folder");
    assert_eq!((done.written, done.removed), (2, 3));
    let again = applied(&mut repo, &journal(b.1, &plan));
    assert_eq!((again.written, again.removed), (0, 0));
    assert!(again.changed_since_plan.is_empty() && again.in_use.is_empty());

    let (_, back) = switch(&mut repo, b.0, a);
    assert!(back.changed_since_plan.is_empty(), "{back:?}");
    assert_eq!(fs::read(dir.path().join("a/y/z")).unwrap(), b"2");
    assert_eq!(fs::read(dir.path().join("f")).unwrap(), b"3");
    assert_eq!(names_in(dir.path()), ["a", "f"]);
}

#[test]
fn unsaved_work_in_the_way_collides_and_the_rest_comes_along() {
    let (dir, mut repo) = project();
    let empty = current_save(&repo).unwrap();
    let a = planted(
        &mut repo,
        &files(&[("keep", b"k"), ("both", b"1"), ("dir/x", b"x")]),
    );
    switch(&mut repo, empty, a);
    let b = planted(
        &mut repo,
        &files(&[("keep", b"k"), ("both", b"2"), ("new", b"n")]),
    );
    fs::write(dir.path().join("both"), b"mine").unwrap();
    fs::write(dir.path().join("dir/mine"), b"mine").unwrap();
    fs::write(dir.path().join("carried"), b"mine").unwrap();
    let plan = planned(&mut repo, a.0, b.0, false);
    assert_eq!(plan.collisions, ["both", "dir"]);
    assert!(plan.actions.iter().all(|a| a.path != "carried"));
    // Removing new files too, as discard does: the folder comes to match the save exactly.
    let discard = planned(&mut repo, a.0, a.0, true);
    let paths: Vec<&str> = discard.actions.iter().map(|a| a.path.as_str()).collect();
    assert_eq!(paths, ["both", "carried", "dir/mine"]);
    assert!(discard.collisions.is_empty());
    assert_eq!(
        discard.actions[0].expected,
        Some(Expected {
            kind: EntryKind::File,
            id: crate::id::hash_content(b"mine"),
            size: 4
        })
    );
}

#[test]
fn nothing_below_a_folder_in_the_way_is_planned() {
    let (dir, mut repo) = project();
    let empty = current_save(&repo).unwrap();
    let a = planted(&mut repo, &files(&[(".boltignore", b"build/\n")]));
    switch(&mut repo, empty, a);
    fs::create_dir(dir.path().join("build")).unwrap();
    fs::write(dir.path().join("build/junk"), b"ignored").unwrap();
    let b = planted(
        &mut repo,
        &files(&[
            (".boltignore", b"build/\n"),
            ("build/x", b"x"),
            ("build/deeper/y", b"y"),
            ("GI7EBA~1/hooks/post-checkout", b"evil"),
        ]),
    );
    let plan = planned(&mut repo, a.0, b.0, false);
    assert_eq!(plan.collisions, ["build"]);
    assert!(plan.actions.is_empty(), "{:?}", plan.actions);
    assert_eq!(plan.not_written, ["GI7EBA~1"]);
}

#[test]
fn what_the_plan_never_read_in_a_folder_that_goes_is_in_the_way_or_kept() {
    let (dir, mut repo) = project();
    let empty = current_save(&repo).unwrap();
    let ignore: (&str, &[u8]) = (".boltignore", b"*.o\nout/\n");
    let a = planted(
        &mut repo,
        &files(&[ignore, ("d/f", b"f"), ("d/sub/g", b"g"), ("e/s/h", b"h")]),
    );
    switch(&mut repo, empty, a);
    // The target has a file where `d` is a folder, and no `e` at all.
    let b = planted(&mut repo, &files(&[ignore, ("d", b"file d")]));
    let clean = planned(&mut repo, a.0, b.0, false);
    assert!(clean.collisions.is_empty(), "{:?}", clean.collisions);
    fs::write(dir.path().join("d/sub/x.o"), b"build").unwrap();
    fs::create_dir(dir.path().join("e/s/out")).unwrap();
    for remove_new in [false, true] {
        let plan = planned(&mut repo, a.0, b.0, remove_new);
        assert_eq!(plan.collisions, ["d/sub/x.o"]);
        let paths: Vec<&str> = plan.actions.iter().map(|a| a.path.as_str()).collect();
        // `e` and `e/s` stay for what is left in them; the saved file goes.
        assert_eq!(paths, ["e/s/h"]);
    }
}

/// Needs a drive that ignores case.
#[cfg(any(windows, target_os = "macos"))]
#[test]
fn a_folder_becoming_a_file_in_another_case_is_judged_as_the_drive_names_it() {
    let (dir, mut repo) = project();
    let empty = current_save(&repo).unwrap();
    let ignore: (&str, &[u8]) = (".boltignore", b"*.o\n");
    let a = planted(&mut repo, &files(&[ignore, ("d/f", b"f"), ("d/s/g", b"g")]));
    switch(&mut repo, empty, a);
    let b = planted(&mut repo, &files(&[ignore, ("D", b"file D")]));
    // Its own saved files are not in its way.
    let clean = planned(&mut repo, a.0, b.0, false);
    assert!(clean.collisions.is_empty(), "{:?}", clean.collisions);
    // An ignored file still is.
    fs::write(dir.path().join("d/s/x.o"), b"build").unwrap();
    let plan = planned(&mut repo, a.0, b.0, false);
    assert_eq!(plan.collisions, ["D/s/x.o"]);
}

#[test]
fn every_name_of_a_path_is_checked_against_its_folders_real_short_names() {
    let own = |folder: &str| -> io::Result<Vec<(String, String)>> {
        Ok(match folder {
            "" => vec![(".git".into(), "ZZ7EBA~1".into())],
            "a" => vec![(".bolt".into(), "BO1234~1".into())],
            _ => vec![],
        })
    };
    assert_eq!(
        first_refused("ZZ7EBA~1/hooks/x", own).unwrap(),
        Some("ZZ7EBA~1")
    );
    assert_eq!(
        first_refused("a/bo1234~1/x", own).unwrap(),
        Some("a/bo1234~1")
    );
    assert_eq!(first_refused("a/ZZ7EBA~1/x", own).unwrap(), None);
    assert_eq!(first_refused("b/c", own).unwrap(), None);
}

#[cfg(windows)]
#[test]
fn the_real_short_name_of_git_leads_nowhere() {
    let (dir, mut repo) = project();
    // These take `GIT~1` to `GIT~4`, so `.git` gets a hashed short name (`GI2837~1` here).
    for taken in ["g it", "gi t", "g i t", "g  it"] {
        fs::create_dir(dir.path().join(taken)).unwrap();
    }
    fs::create_dir_all(dir.path().join(".git/hooks")).unwrap();
    let top = bolt_platform::dir::Dir::open_top(dir.path()).unwrap();
    let short = top
        .entries()
        .unwrap()
        .into_iter()
        .find(|e| e.name == ".git")
        .and_then(|e| e.short_name)
        .map(|s| s.to_string_lossy().into_owned());
    let Some(short) = short else {
        println!("Skipped: this drive gives no 8.3 names; the short-name check is tested above.");
        return;
    };
    // No rule refuses the name by itself: only the folder's own short names catch it.
    assert_eq!(crate::names::writable_here(&short, &[]), Ok(()), "{short}");
    let to = planted(&mut repo, &files(&[("x", b"x")]));
    let by_hand = Journal {
        target: to.1,
        from: to.1,
        actions: vec![Action {
            path: format!("{short}/hooks/post-checkout"),
            change: Change::File(content(b"x")),
            expected: None,
        }],
    };
    let done = applied(&mut repo, &by_hand);
    assert_eq!(done.written, 0);
    assert!(names_in(&dir.path().join(".git/hooks")).is_empty());
}

#[test]
fn a_folder_where_a_project_link_is_gets_nothing_written() {
    let (dir, mut repo) = project();
    let empty = current_save(&repo).unwrap();
    let a = planted(&mut repo, &[("vendor", EntryKind::ProjectLink, &[7; 20])]);
    switch(&mut repo, empty, a);
    // The other project's own checkout.
    fs::create_dir(dir.path().join("vendor")).unwrap();
    let b = planted(
        &mut repo,
        &files(&[("vendor/x", b"x"), ("vendor/y/z", b"z")]),
    );
    let plan = planned(&mut repo, a.0, b.0, false);
    assert!(plan.actions.is_empty(), "{:?}", plan.actions);
    let discard = planned(&mut repo, a.0, b.0, true);
    assert!(discard.actions.is_empty(), "{:?}", discard.actions);
}

#[test]
fn a_link_whose_size_is_understated_is_a_placeholder() {
    use crate::listing::Entry;
    use crate::test_support::{add_listing, plant_root};
    let (_dir, mut repo) = project();
    let empty = current_save(&repo).unwrap();
    let long = "x/".repeat(3000);
    plant_root(&mut repo, |packs| {
        let id = packs.add_content(long.as_bytes()).unwrap();
        let link = Entry {
            name: "l".into(),
            kind: EntryKind::Symlink,
            id,
            size: 1,
        };
        add_listing(packs, vec![link]).0
    });
    let to = current_save(&repo).unwrap();
    let plan = planned(&mut repo, empty, to, false);
    assert_eq!(plan.placeholders, ["l"]);
}

#[test]
fn a_removal_below_a_folder_swapped_for_a_link_is_reported() {
    let (dir, mut repo) = project();
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("x"), b"keep").unwrap();
    let to = planted(&mut repo, &files(&[("y", b"y")]));
    folder_link(outside.path(), &dir.path().join("c"));
    let by_hand = Journal {
        target: to.1,
        from: to.1,
        actions: vec![Action {
            path: "c/x".into(),
            change: Change::RemoveFile,
            expected: Some(Expected {
                kind: EntryKind::File,
                id: crate::id::hash_content(b"keep"),
                size: 4,
            }),
        }],
    };
    let done = applied(&mut repo, &by_hand);
    assert_eq!(done.changed_since_plan, ["c/x"]);
    assert_eq!(fs::read(outside.path().join("x")).unwrap(), b"keep");
}

#[test]
fn a_link_through_a_name_that_isnt_a_folder_is_a_placeholder() {
    let (_dir, mut repo) = project();
    let empty = current_save(&repo).unwrap();
    let link = |path, target: &'static [u8]| (path, EntryKind::Symlink, target);
    let to = planted(
        &mut repo,
        &[
            ("f", EntryKind::File, b"f"),
            ("d/inside", EntryKind::File, b"i"),
            link("through_a_file", b"f/../d/inside"),
            link("through_nothing", b"nowhere/../d/inside"),
            link("fine", b"d/inside"),
        ],
    );
    let plan = planned(&mut repo, empty, to.0, false);
    assert_eq!(plan.placeholders, ["through_a_file", "through_nothing"]);
}

#[test]
fn a_link_is_written_as_one_only_inside_the_project_through_folders_of_the_save() {
    let (_dir, mut repo) = project();
    let save = planted(&mut repo, &files(&[("d/f", b"f")])).0;
    let root = repo.store().save(&save, false).unwrap().root;
    let stored = HashMap::new();
    let exact = Drive {
        ignores_case: false,
        composes_unicode: false,
    };
    let mut tree = Tree::new(repo.store(), &stored, root, exact);
    let too_long = format!("{}d/f", "./".repeat(2100));
    for (path, target) in [
        ("out", "/tmp"),
        ("unc", "//host/share"),
        ("drive", "C:/x"),
        ("up", "../x"),
        ("d/up", "../../x"),
        ("through_a_file", "d/f/../f"),
        ("through_nothing", "e/../d/f"),
        ("long", too_long.as_str()),
    ] {
        let as_link = writes_as_link(&mut tree, path, target.as_bytes()).unwrap();
        assert!(!as_link, "{path}");
    }
    for (path, target) in [("in", "d/f"), ("d/back", "../d"), ("dot", "./d")] {
        let as_link = writes_as_link(&mut tree, path, target.as_bytes()).unwrap();
        assert!(as_link, "{path}");
    }
}

#[test]
fn a_link_through_a_name_write_out_wont_write_is_a_placeholder() {
    let (_dir, mut repo) = project();
    let tree_files = files(&[
        (".git/hooks/pre-commit", b"run"),
        ("git~1/hooks/x", b"run"),
        ("d/f", b"f"),
    ]);
    let save = planted(&mut repo, &tree_files).0;
    let root = repo.store().save(&save, false).unwrap().root;
    let stored = HashMap::new();
    let exact = Drive {
        ignores_case: false,
        composes_unicode: false,
    };
    let mut tree = Tree::new(repo.store(), &stored, root, exact);
    // The save's own `.git` folder is never written, so a link into it can't be one either.
    for (path, target) in [
        ("hook", ".git/hooks/pre-commit"),
        ("hooks", ".git/hooks"),
        ("git", ".git"),
        ("short", "git~1/hooks/x"),
        ("bolt", ".bolt/x"),
        ("d/up", "../.git/hooks"),
    ] {
        let as_link = writes_as_link(&mut tree, path, target.as_bytes()).unwrap();
        assert!(!as_link, "{path}");
    }
    assert!(writes_as_link(&mut tree, "in", b"d/f").unwrap());
}

#[test]
fn whether_links_can_be_made_is_tried_in_bolts_own_folder() {
    let (_dir, mut repo) = project();
    let _lock = repo.lock_for_change("discard", &mut |_| {}).unwrap();
    let probe = tempfile::tempdir().unwrap();
    let link = probe.path().join("l");
    #[cfg(unix)]
    let made = std::os::unix::fs::symlink("x", &link);
    #[cfg(windows)]
    let made = std::os::windows::fs::symlink_file("x", &link);
    assert_eq!(can_make_links(&repo).unwrap(), made.is_ok());
}

/// Makes `dir` keep letter case apart, as WSL folders do on a drive that otherwise ignores it.
/// False, with a note, where Windows can't.
#[cfg(windows)]
fn keep_case_apart(dir: &Path) -> bool {
    let done = std::process::Command::new("fsutil")
        .args(["file", "setCaseSensitiveInfo"])
        .arg(dir)
        .arg("enable")
        .stdout(std::process::Stdio::null())
        .status();
    if done.is_ok_and(|s| s.success()) {
        return true;
    }
    println!("Skipped: this Windows can't make a folder that keeps letter case apart.");
    false
}

#[cfg(windows)]
#[test]
fn a_case_rename_in_a_folder_that_keeps_case_apart_moves_no_unchecked_file() {
    let (dir, mut repo) = project();
    let drive = this_drive(&mut repo);
    assert!(drive.ignores_case);
    fs::create_dir(dir.path().join("docs")).unwrap();
    if !keep_case_apart(&dir.path().join("docs")) {
        return;
    }
    let empty = current_save(&repo).unwrap();
    let a = planted(&mut repo, &files(&[("docs/Readme.md", b"old")]));
    switch(&mut repo, empty, a);
    let b = planted(&mut repo, &files(&[("docs/README.md", b"new")]));
    let plan = planned(&mut repo, a.0, b.0, false);
    // After the plan: the old spelling edited, and a separate file under the new one.
    fs::write(dir.path().join("docs/Readme.md"), b"mine").unwrap();
    fs::write(dir.path().join("docs/README.md"), b"old").unwrap();
    let done = applied(&mut repo, &journal(b.1, &plan));
    assert_eq!(
        fs::read(dir.path().join("docs/Readme.md")).unwrap(),
        b"mine"
    );
    assert_eq!(fs::read(dir.path().join("docs/README.md")).unwrap(), b"new");
    assert_eq!(done.changed_since_plan, ["docs/Readme.md"]);
}

#[test]
fn a_link_replaced_by_a_file_named_in_other_case_is_written() {
    let (dir, mut repo) = project();
    let outside = tempfile::tempdir().unwrap();
    folder_link(outside.path(), &dir.path().join("readme"));
    let read = fs::read_link(dir.path().join("readme")).unwrap();
    let target = read.to_string_lossy().into_owned();
    let target = if cfg!(windows) {
        target.replace('\\', "/")
    } else {
        target
    };
    let a = planted(
        &mut repo,
        &[("readme", EntryKind::Symlink, target.as_bytes())],
    );
    let b = planted(&mut repo, &files(&[("README", b"new")]));
    let (_, done) = switch(&mut repo, a.0, b);
    assert!(done.changed_since_plan.is_empty(), "{done:?}");
    assert_eq!(names_in(dir.path()), ["README"]);
    assert_eq!(fs::read(dir.path().join("README")).unwrap(), b"new");
    assert!(outside.path().exists());
}

const OLD: &[(&str, &[u8])] = &[
    ("a.txt", b"old a"),
    ("b.txt", b"same"),
    ("dir/c.txt", b"old c"),
    ("gone/x.txt", b"bye"),
    ("gone.txt", b"bye"),
];
const NEW: &[(&str, &[u8])] = &[
    ("a.txt", b"new a"),
    ("b.txt", b"same"),
    ("dir/c.txt", b"new c"),
    ("new/d.txt", b"d"),
    ("new/e.txt", b"e"),
];

/// A project on the save `OLD`, its folder holding it, with the save `NEW` stored and the
/// journal of a switch to it written, but nothing switched yet.
fn stopped_switch() -> (tempfile::TempDir, Repository, Journal) {
    let (dir, mut repo) = project();
    let old = planted(&mut repo, &files(OLD));
    let new = planted(&mut repo, &files(NEW));
    for (path, bytes) in OLD {
        let at = dir.path().join(path);
        fs::create_dir_all(at.parent().unwrap()).unwrap();
        fs::write(at, bytes).unwrap();
    }
    let lock = repo.lock_for_change("goto", &mut |_| {}).unwrap();
    repo.move_head_to(&lock, old.1).unwrap();
    let drive = drive::probe(repo.tmp_folder()).unwrap();
    let plan = plan(&repo, old.0, new.0, None, false, drive, None).unwrap();
    let journal = Journal {
        target: new.1,
        from: old.1,
        actions: plan.actions,
    };
    journal.write(&repo, &lock).unwrap();
    (dir, repo, journal)
}

/// What a file of the switch holds now, if it is there.
fn held(dir: &Path, path: &str) -> Option<Vec<u8>> {
    fs::read(dir.join(path)).ok()
}

fn assert_holds(dir: &Path, wanted: &[(&str, &[u8])]) {
    for (path, bytes) in wanted {
        assert_eq!(held(dir, path).as_deref(), Some(*bytes), "{path}");
    }
}

fn save_now(repo: &mut Repository) -> Result<crate::snapshot::SaveOutcome, SaveError> {
    let who = who();
    let req = crate::snapshot::SaveRequest {
        message: Some("next"),
        who: &who,
        command: "save",
        accept_crlf: false,
        git_excludes: None,
        new_line: None,
        only: None,
        amend: false,
    };
    crate::snapshot::save_folder(repo, &req, &mut |_| {}, &mut |_| true)
}

#[cfg(feature = "fault-injection")]
#[test]
fn a_switch_stopped_at_every_write_is_finished_by_the_next_command() {
    use crate::fsio::fault;
    use crate::snapshot::SaveOutcome;
    use crate::test_support::after_head;
    let mut stops_before_head = 0;
    for n in 1.. {
        assert!(n < 100, "the switch never finished");
        let (dir, mut repo, journal) = stopped_switch();
        let lock = repo.lock_for_change("goto", &mut |_| {}).unwrap();
        let drive = drive::probe(repo.tmp_folder()).unwrap();
        fault::arm(n);
        let result = carry_out(&mut repo, &lock, &journal, drive);
        fault::disarm();
        let went_on = after_head(&repo);
        drop(lock);
        drop(repo);
        assert_wholly_old_or_new(dir.path(), n);

        let mut repo = Repository::open(dir.path()).unwrap();
        let took_effect = repo.head().unwrap() == journal.target;
        let outcome = save_now(&mut repo).unwrap();
        let SaveOutcome::NothingChanged { current, .. } = outcome else {
            panic!("fault {n}: the save saved the folder before the switch finished");
        };
        assert_eq!(current, tip_of(&repo.current().unwrap().1).unwrap());
        assert_switched(&repo, dir.path(), &journal, n);
        // Said when the save had a switch to finish; only the journal's removal is silent.
        let finished = kinds(&repo.store().warnings()).contains(&warning::SWITCH_FINISHED);
        assert_eq!(finished, result.is_err() && !took_effect, "fault {n}");
        if result.is_err() && !took_effect {
            stops_before_head += 1;
        }
        assert!(
            result.is_ok() || !took_effect,
            "fault {n}: failed once its head moved"
        );
        if let Ok(done) = result
            && !went_on
        {
            assert!(done.in_use.is_empty());
            // A stop after each change and before the head, then three after it: in flushing
            // the head, in removing the journal and in flushing that.
            assert!(stops_before_head >= 8, "only {stops_before_head} stops");
            assert!(
                n > stops_before_head + 3,
                "too few stops after the head moved"
            );
            break;
        }
    }
}

/// Every path below `dir`, `.bolt` aside, `/` between names.
#[cfg(feature = "fault-injection")]
fn all_paths(dir: &Path) -> Vec<String> {
    let mut found = Vec::new();
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
                folders.push((e.path(), path.clone()));
            }
            found.push(path);
        }
    }
    found.sort();
    found
}

/// Every file of the switch wholly old or wholly new, and nothing else anywhere in the folder.
#[cfg(feature = "fault-injection")]
fn assert_wholly_old_or_new(dir: &Path, n: u64) {
    for (path, _) in OLD.iter().chain(NEW) {
        let now = held(dir, path);
        let old = OLD.iter().find(|(p, _)| p == path).map(|(_, b)| b.to_vec());
        let new = NEW.iter().find(|(p, _)| p == path).map(|(_, b)| b.to_vec());
        assert!(now == old || now == new, "fault {n}: {path} holds {now:?}");
    }
    let known: Vec<&str> = OLD
        .iter()
        .chain(NEW)
        .flat_map(|(p, _)| [*p, parent(p)])
        .collect();
    for path in all_paths(dir) {
        assert!(known.contains(&path.as_str()), "fault {n}: {path}");
    }
}

/// The switch done: `head` on its entry, no journal, the folder exactly the new save.
#[cfg(feature = "fault-injection")]
fn assert_switched(repo: &Repository, dir: &Path, journal: &Journal, n: u64) {
    assert_eq!(repo.head().unwrap(), journal.target, "fault {n}");
    assert_eq!(Journal::read(repo).unwrap(), None, "fault {n}");
    assert_holds(dir, NEW);
    let mut wanted: Vec<&str> = NEW.iter().flat_map(|(p, _)| [*p, parent(p)]).collect();
    wanted.retain(|p| !p.is_empty());
    wanted.sort_unstable();
    wanted.dedup();
    assert_eq!(all_paths(dir), wanted, "fault {n}");
}

#[cfg(feature = "fault-injection")]
#[test]
fn a_finish_stopped_in_turn_is_finished_by_the_command_after() {
    use crate::fsio::fault;
    use crate::test_support::after_head;
    for n in [1, 4, 9] {
        for m in 1.. {
            assert!(m < 60, "the save never got through");
            let (dir, mut repo, journal) = stopped_switch();
            let lock = repo.lock_for_change("goto", &mut |_| {}).unwrap();
            let drive = drive::probe(repo.tmp_folder()).unwrap();
            fault::arm(n);
            assert!(carry_out(&mut repo, &lock, &journal, drive).is_err());
            fault::disarm();
            drop(lock);
            fault::arm(m);
            let finishing = save_now(&mut repo);
            fault::disarm();
            let went_on = after_head(&repo);
            drop(repo);
            assert_wholly_old_or_new(dir.path(), m);
            let mut repo = Repository::open(dir.path()).unwrap();
            save_now(&mut repo).unwrap();
            assert_switched(&repo, dir.path(), &journal, m);
            if finishing.is_ok() && !went_on {
                break;
            }
        }
    }
}

#[test]
fn a_journal_whose_target_is_the_head_is_only_removed() {
    let (dir, mut repo, journal) = stopped_switch();
    let lock = repo.lock_for_change("save", &mut |_| {}).unwrap();
    repo.move_head_to(&lock, journal.target).unwrap();
    // A stop finished all the same, with nothing left to write.
    let done = finish_interrupted(&mut repo, &lock).unwrap();
    assert_eq!(done, Some(Applied::default()));
    assert_eq!(Journal::read(&repo).unwrap(), None);
    // Nothing of it was carried out again.
    assert_holds(dir.path(), OLD);
}

#[test]
fn status_says_a_switch_is_unfinished() {
    let (_dir, repo, _) = stopped_switch();
    let found = crate::status::status(&repo, false, None).unwrap();
    assert!(found.unfinished_switch.is_some());
    assert!(kinds(&found.warnings).contains(&warning::SWITCH_UNFINISHED));
}

#[test]
fn a_switch_that_cant_write_a_file_names_it_and_waits_for_the_next_command() {
    let (dir, mut repo, mut journal) = stopped_switch();
    // A journal whose size for a file isn't its content's, as damage would leave it.
    let a = journal
        .actions
        .iter_mut()
        .find(|a| a.path == "dir/c.txt")
        .unwrap();
    let Change::File(c) = &mut a.change else {
        panic!("{a:?}")
    };
    c.size += 1;
    let lock = repo.lock_for_change("save", &mut |_| {}).unwrap();
    journal.write(&repo, &lock).unwrap();
    for _ in 0..2 {
        let e = finish_interrupted(&mut repo, &lock).unwrap_err();
        assert!(
            matches!(&e, RepoError::SwitchStopped { path, .. } if path == "dir/c.txt"),
            "{e:?}"
        );
        assert!(e.to_string().contains("dir/c.txt"), "{e}");
        let hint = e.hint().unwrap();
        assert!(
            hint.contains("next command") && hint.contains("damaged"),
            "{hint}"
        );
        // Not taken back, and not passed over either.
        assert_eq!(Journal::read(&repo).unwrap(), Some(journal.clone()));
        assert_eq!(repo.head().unwrap(), journal.from);
        assert_eq!(held(dir.path(), "dir/c.txt").unwrap(), b"old c");
        assert_eq!(held(dir.path(), "a.txt").unwrap(), b"new a");
    }
    drop(lock);
    assert!(matches!(
        save_now(&mut repo),
        Err(SaveError::Repo(RepoError::SwitchStopped { .. }))
    ));
}

#[test]
fn a_journal_that_cant_be_used_stops_changes_and_is_reported() {
    let (dir, mut repo, journal) = stopped_switch();
    let on_disk = dir.path().join(".bolt/state/journal");
    let mut bytes = fs::read(&on_disk).unwrap();
    // At its front, where commands that only read look too.
    bytes[0] ^= 1;
    fs::write(&on_disk, &bytes).unwrap();
    let lock = repo.lock_for_change("save", &mut |_| {}).unwrap();
    let e = finish_interrupted(&mut repo, &lock).unwrap_err();
    assert!(matches!(e, RepoError::JournalUnusable { .. }), "{e:?}");
    assert!(e.to_string().contains(".bolt/state/journal"), "{e}");
    assert!(e.hint().unwrap().contains("delete .bolt/state/journal"));
    assert_holds(dir.path(), OLD);
    drop(lock);
    let found = crate::status::status(&repo, false, None).unwrap();
    assert!(kinds(&found.warnings).contains(&warning::SWITCH_UNFINISHED));
    assert!(
        found
            .unfinished_switch
            .unwrap()
            .contains(".bolt/state/journal")
    );

    // One made for another save than the head's is no switch of this folder.
    let lock = repo.lock_for_change("save", &mut |_| {}).unwrap();
    let stray = Journal {
        from: journal.target,
        target: Id([7; 32]),
        ..journal
    };
    stray.write(&repo, &lock).unwrap();
    let e = finish_interrupted(&mut repo, &lock).unwrap_err();
    assert!(matches!(e, RepoError::JournalUnusable { .. }), "{e:?}");
    assert_holds(dir.path(), OLD);
    drop(lock);
    // Read-only commands say the same, not that the next command finishes it.
    let found = crate::status::status(&repo, false, None).unwrap();
    let said = found.unfinished_switch.unwrap();
    assert!(said.contains(".bolt/state/journal"), "{said}");
}

#[cfg(windows)]
#[test]
fn a_file_held_open_keeps_the_switch_unfinished_until_let_go() {
    use std::os::windows::fs::OpenOptionsExt;
    let (dir, mut repo, journal) = stopped_switch();
    let handle = fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(dir.path().join("a.txt"))
        .unwrap();
    let lock = repo.lock_for_change("save", &mut |_| {}).unwrap();
    let e = finish_interrupted(&mut repo, &lock).unwrap_err();
    assert!(matches!(e, RepoError::SwitchInUse { .. }), "{e:?}");
    assert!(e.to_string().contains("a.txt"), "{e}");
    assert_eq!(repo.head().unwrap(), journal.from);
    assert!(Journal::read(&repo).unwrap().is_some());
    drop(handle);
    let done = finish_interrupted(&mut repo, &lock).unwrap().unwrap();
    assert!(kinds(&done.warnings).contains(&warning::SWITCH_FINISHED));
    assert_eq!(repo.head().unwrap(), journal.target);
    assert_holds(dir.path(), NEW);
}

/// Commits a save whose top listing holds `.BOLT`, and gives its ID.
fn planted_hostile(repo: &mut Repository) -> Id {
    // No listing Bolt makes holds `.BOLT`, so it is spelled `-BOLT`, then the bytes are changed.
    crate::test_support::plant_root(repo, |packs| {
        let id = packs.add_content(b"x").unwrap();
        let named = |name: &str| Entry {
            name: name.into(),
            kind: EntryKind::File,
            id,
            size: 1,
        };
        let listing = Listing::new(vec![named("-BOLT")]).unwrap().encode();
        let at = listing.windows(5).position(|w| w == b"-BOLT").unwrap();
        let mut hostile = listing;
        hostile[at] = b'.';
        packs
            .add_record(ObjectKind::FolderListing, &hostile)
            .unwrap()
    });
    current_save(repo).unwrap()
}

#[test]
fn a_spelling_of_bolt_the_plan_meets_is_told_as_the_save_holding_it() {
    let (_dir, mut repo) = project();
    let (good, _) = planted(&mut repo, &files(&[("a.txt", b"a")]));
    let other = planted_hostile(&mut repo);
    let hostile = planted_hostile(&mut repo);
    let told = |repo: &Repository, from, to| {
        let met = SaveError::Repo(RepoError::BoltName {
            save: None,
            current: false,
        });
        match told_whose(repo, met, from, to) {
            SaveError::Repo(RepoError::BoltName { save, current }) => (save, current),
            other => panic!("{other:?}"),
        }
    };
    // On a save that holds it, every switch away meets it, whatever the target holds.
    assert_eq!(told(&repo, hostile, good), (Some(hostile), true));
    assert_eq!(told(&repo, hostile, other), (Some(hostile), true));
    // The save planned from is named if it holds it, else the target; the narrower hint only
    // for the save the project is on, which an unshelf's base or an undo's kept save isn't.
    planted(&mut repo, &files(&[("b.txt", b"b")]));
    assert_eq!(told(&repo, hostile, good), (Some(hostile), false));
    assert_eq!(told(&repo, good, hostile), (Some(hostile), false));
    assert_eq!(told(&repo, other, hostile), (Some(other), false));
    // Reading the folder against such a save meets it first.
    let drive = this_drive(&mut repo);
    match plan(&repo, hostile, good, None, false, drive, None) {
        Err(SaveError::Repo(RepoError::BoltName { save, current })) => {
            assert_eq!((save, current), (Some(hostile), false));
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_placeholder_swapped_in_for_an_unsaved_link_is_kept_before_it_goes() {
    let (dir, mut repo) = project();
    let from = current_save(&repo).unwrap();
    let target = b"a.txt";
    let id = hash_content(target);
    // What a discard plans for an unsaved link `l` -> a.txt, which the save doesn't hold.
    let actions = vec![Action {
        path: "l".into(),
        change: Change::RemoveFile,
        expected: Some(Expected {
            kind: EntryKind::Symlink,
            id,
            size: 5,
        }),
    }];
    // Another program swaps the link for a file holding exactly its target.
    fs::write(dir.path().join("l"), target).unwrap();
    let (head, _) = repo.current().unwrap();
    let lock = repo.lock_for_change("discard", &mut |_| {}).unwrap();
    let drive = drive::probe(repo.tmp_folder()).unwrap();
    let mut packs = repo.new_packs().unwrap();
    let copies = keep_safety_copies(&repo, &actions, from, drive, &mut packs).unwrap();
    let copy = SafetyCopy {
        path: "l".into(),
        kind: EntryKind::Symlink,
        id,
        size: 5,
    };
    assert_eq!(copies, [copy]);
    repo.store_packs(&lock, packs).unwrap();
    let journal = Journal {
        target: head,
        from: head,
        actions,
    };
    let done = apply(&repo, &lock, &journal, drive).unwrap();
    assert_eq!((done.removed, done.changed_since_plan.len()), (1, 0));
    let mut kept = Vec::new();
    repo.store().copy_content(&id, &mut kept).unwrap();
    assert_eq!(kept, target);
}

#[test]
fn unsaved_work_changed_to_as_many_bytes_after_the_plan_is_left_alone() {
    let mut big = noise(9, 3 << 20);
    let small = (b"mine".to_vec(), b"MINE".to_vec());
    let planned_big = big.clone();
    big[1 << 20..(1 << 20) + 4].copy_from_slice(b"MINE");
    for (name, mine, now) in [("small", small.0, small.1), ("big", planned_big, big)] {
        let (dir, mut repo) = project();
        let from = current_save(&repo).unwrap();
        // What a discard plans for the unsaved file: to remove it, holding `mine`.
        let actions = vec![Action {
            path: name.into(),
            change: Change::RemoveFile,
            expected: Some(Expected {
                kind: EntryKind::File,
                id: hash_content(&mine),
                size: mine.len() as u64,
            }),
        }];
        // Then the user writes as many bytes over it.
        fs::write(dir.path().join(name), &now).unwrap();
        let (head, _) = repo.current().unwrap();
        let lock = repo.lock_for_change("discard", &mut |_| {}).unwrap();
        let drive = drive::probe(repo.tmp_folder()).unwrap();
        let mut packs = repo.new_packs().unwrap();
        let copies = keep_safety_copies(&repo, &actions, from, drive, &mut packs).unwrap();
        assert!(copies.is_empty(), "{name}: {copies:?}");
        repo.store_packs(&lock, packs).unwrap();
        let journal = Journal {
            target: head,
            from: head,
            actions,
        };
        let done = apply(&repo, &lock, &journal, drive).unwrap();
        assert_eq!(done.changed_since_plan, [name]);
        assert_eq!(fs::read(dir.path().join(name)).unwrap(), now, "{name}");
    }
}

/// Unsaved work too long to cut is kept whole, compared with the plan's ID as it is read, so one
/// changed to as many bytes is left out.
#[test]
fn unsaved_work_too_long_to_cut_is_kept_whole_unless_it_changed() {
    // Files of 1 to 2 MiB are cut; a longer one is too long to cut.
    crate::chunks::set_test_cut_sizes(Some((1 << 20, 2 << 20)));
    let mine = noise(11, 3 << 20);
    let mut changed = mine.clone();
    changed[1 << 20..(1 << 20) + 4].copy_from_slice(b"MINE");
    let id = hash_content(&mine);
    for (now, kept) in [(&mine, true), (&changed, false)] {
        let (dir, mut repo) = project();
        let from = current_save(&repo).unwrap();
        let actions = vec![Action {
            path: "big".into(),
            change: Change::RemoveFile,
            expected: Some(Expected {
                kind: EntryKind::File,
                id,
                size: mine.len() as u64,
            }),
        }];
        fs::write(dir.path().join("big"), now).unwrap();
        let _lock = repo.lock_for_change("discard", &mut |_| {}).unwrap();
        let drive = drive::probe(repo.tmp_folder()).unwrap();
        let mut packs = repo.new_packs().unwrap();
        let copies = keep_safety_copies(&repo, &actions, from, drive, &mut packs).unwrap();
        assert_eq!(copies.len(), usize::from(kept), "{kept}");
        assert_eq!(packs.kind_of(&id), kept.then_some(ObjectKind::Content));
    }
    crate::chunks::set_test_cut_sizes(None);
}

/// A new project holding unsaved work a discard would remove, and the actions that remove it:
/// 120 small files in five folders, some of them alike, a big file and, where links can be made,
/// a link.
fn unsaved_work() -> (tempfile::TempDir, Repository, Vec<Action>) {
    let (dir, repo) = project();
    let mut actions = Vec::new();
    let mut plan = |path: &str, kind: EntryKind, bytes: &[u8]| {
        actions.push(Action {
            path: path.into(),
            change: Change::RemoveFile,
            expected: Some(Expected {
                kind,
                id: hash_content(bytes),
                size: bytes.len() as u64,
            }),
        });
    };
    for i in 0..120u64 {
        let bytes = match i % 3 {
            0 => format!("unsaved {i}\n").repeat(i as usize + 1).into_bytes(),
            1 => noise(i, 100 + (i as usize * 97) % 5000),
            _ => b"the same in many folders\n".to_vec(),
        };
        let path = format!("d{}/f{i:03}.txt", i % 5);
        fs::create_dir_all(dir.path().join(format!("d{}", i % 5))).unwrap();
        fs::write(dir.path().join(&path), &bytes).unwrap();
        plan(&path, EntryKind::File, &bytes);
    }
    let big = noise(500, 3 << 20);
    fs::write(dir.path().join("big.bin"), &big).unwrap();
    plan("big.bin", EntryKind::File, &big);
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink("d0/f000.txt", dir.path().join("link")).unwrap();
        plan("link", EntryKind::Symlink, b"d0/f000.txt");
    }
    (dir, repo, actions)
}

/// The safety copies `keep_safety_copies` keeps of `actions` with `count` workers, or the first
/// problem, and the packs it wrote, 7 objects a pack so that the set rolls over many times.
fn copied_with(
    count: usize,
    repo: &mut Repository,
    actions: &[Action],
) -> (Result<Vec<SafetyCopy>, String>, Written) {
    let from = current_save(repo).unwrap();
    let _lock = repo.lock_for_change("discard", &mut |_| {}).unwrap();
    let drive = drive::probe(repo.tmp_folder()).unwrap();
    let tmp = repo.root().join(".bolt/tmp");
    let mut packs = PackSet::with_limit(&tmp, repo.packs_dir(), 7).unwrap();
    crate::workers::set_test_workers(Some(count));
    let copies = keep_safety_copies(repo, actions, from, drive, &mut packs);
    crate::workers::set_test_workers(None);
    (copies.map_err(|e| format!("{e:?}")), written(packs))
}

#[test]
fn safety_copies_of_many_files_are_the_same_on_one_thread_or_eight() {
    let mut seen = Vec::new();
    for count in [1, 8] {
        let (dir, mut repo, actions) = unsaved_work();
        // Changed to as many bytes since the plan: neither kept nor stored.
        let changed = dir.path().join("d1/f001.txt");
        let mut now = fs::read(&changed).unwrap();
        now[50] ^= 1;
        fs::write(&changed, &now).unwrap();
        let (copies, written) = copied_with(count, &mut repo, &actions);
        let copies = copies.unwrap();
        assert_eq!(copies.len(), actions.len() - 1, "{count}");
        assert!(copies.iter().all(|c| c.path != "d1/f001.txt"));
        for (name, _) in &written {
            let pack = crate::pack::PackReader::open(&repo.packs_dir().join(name)).unwrap();
            let mut ids = pack.entries().map(|e| e.unwrap().id);
            assert!(!ids.any(|id| id == hash_content(&now)), "{count}: stored");
        }
        assert!(written.len() > 5, "{} packs", written.len());
        seen.push((copies, written));
    }
    assert_eq!(seen[0].0, seen[1].0);
    assert_eq!(names(&seen[0].1), names(&seen[1].1));
    assert!(seen[0].1 == seen[1].1, "the packs hold other bytes");
}

/// The copies run on workers; the hook this thread sets reaches them.
#[cfg(feature = "fault-injection")]
#[test]
fn a_file_the_hook_makes_unreadable_is_caught_when_copies_run_on_workers() {
    for count in [8, 1] {
        let (_dir, mut repo, actions) = unsaved_work();
        crate::fsio::fault::unreadable(Some("d2/f052.txt"));
        let (copies, _) = copied_with(count, &mut repo, &actions);
        crate::fsio::fault::unreadable(None);
        let said = copies.unwrap_err();
        assert!(
            said.contains("Unreadable") && said.contains("\"d2/f052.txt\""),
            "{count}: {said}"
        );
    }
}

/// The copies are kept in the actions' order, so an earlier entry's problem comes first: here a
/// file that can't be read before a folder of the save that the store can't read.
#[cfg(feature = "fault-injection")]
#[test]
fn an_earlier_entrys_problem_comes_before_a_later_ones() {
    let (dir, mut repo) = project();
    // The save's folder `b` names a listing the store doesn't hold.
    crate::test_support::plant_root(&mut repo, |packs| {
        let lost = Entry {
            name: "b".into(),
            kind: EntryKind::Folder,
            id: Id([9; 32]),
            size: 0,
        };
        crate::test_support::add_listing(packs, vec![lost]).0
    });
    fs::write(dir.path().join("a.txt"), b"mine").unwrap();
    let remove = |path: &str| Action {
        path: path.into(),
        change: Change::RemoveFile,
        expected: Some(Expected {
            kind: EntryKind::File,
            id: hash_content(b"mine"),
            size: 4,
        }),
    };
    let actions = [remove("a.txt"), remove("b/c.txt")];
    crate::fsio::fault::unreadable(Some("a.txt"));
    let (copies, _) = copied_with(8, &mut repo, &actions);
    crate::fsio::fault::unreadable(None);
    let said = copies.unwrap_err();
    assert!(
        said.contains("Unreadable") && said.contains("\"a.txt\""),
        "{said}"
    );
}

#[test]
fn a_plan_reads_no_listing_of_the_save_it_is_on_again() {
    let (dir, mut repo) = project();
    let on = files(&[
        ("a/b/two.txt", b"two"),
        ("a/one.txt", b"one"),
        ("c/four.txt", b"four"),
    ]);
    let from = planted(&mut repo, &on).0;
    let mut more = on.clone();
    more.push(("new.txt", EntryKind::File, b"new"));
    let to = planted(&mut repo, &more).0;
    // The folder holds the target, so every listing a comparison opens is one read already.
    for (path, _, bytes) in &more {
        let at = dir.path().join(path);
        fs::create_dir_all(at.parent().unwrap()).unwrap();
        fs::write(at, bytes).unwrap();
    }
    let _lock = repo.lock_for_change("goto", &mut |_| {}).unwrap();
    let drive = drive::probe(repo.tmp_folder()).unwrap();
    let reads = crate::pack::open_files::reads;
    let start = reads();
    repo.store().save(&from, true).unwrap();
    repo.store().save(&to, true).unwrap();
    let saves = reads() - start;
    let start = reads();
    plan(&repo, from, to, None, false, drive, None).unwrap();
    // The walk reads the listings on a thread of its own; this one reads only the two saves, or
    // less when the walk's reads left their block held.
    assert!(reads() - start <= saves, "{} reads", reads() - start);
}
