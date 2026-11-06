#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use super::*;
use crate::drive;
use crate::history::{current_save, tip_of};
use crate::journal::{Change, Content, Expected};
use crate::listing::EntryKind;
use crate::save::Person;
use crate::test_support::plant_save;
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
        message: "next",
        who: &who,
        command: "save",
        accept_crlf: false,
        git_excludes: None,
    };
    crate::snapshot::save_folder(repo, &req, &mut |_| {})
}

#[cfg(feature = "fault-injection")]
#[test]
fn a_switch_stopped_at_every_write_is_finished_by_the_next_command() {
    use crate::fsio::fault;
    use crate::snapshot::SaveOutcome;
    let mut stops_before_head = 0;
    for n in 1.. {
        assert!(n < 100, "the switch never finished");
        let (dir, mut repo, journal) = stopped_switch();
        let lock = repo.lock_for_change("goto", &mut |_| {}).unwrap();
        let drive = drive::probe(repo.tmp_folder()).unwrap();
        fault::arm(n);
        let result = carry_out(&mut repo, &lock, &journal, drive);
        fault::disarm();
        drop(lock);
        drop(repo);
        // Every file wholly old or wholly new, and nothing else in the folder.
        for (path, _) in OLD.iter().chain(NEW) {
            let now = held(dir.path(), path);
            let old = OLD.iter().find(|(p, _)| p == path).map(|(_, b)| b.to_vec());
            let new = NEW.iter().find(|(p, _)| p == path).map(|(_, b)| b.to_vec());
            assert!(now == old || now == new, "fault {n}: {path} holds {now:?}");
        }
        for name in names_in(dir.path()) {
            assert!(
                ["a.txt", "b.txt", "dir", "gone", "gone.txt", "new"].contains(&name.as_str()),
                "fault {n}: {name}"
            );
        }

        let mut repo = Repository::open(dir.path()).unwrap();
        let took_effect = repo.head().unwrap() == journal.target;
        let outcome = save_now(&mut repo).unwrap();
        let SaveOutcome::NothingChanged { current, warnings } = outcome else {
            panic!("fault {n}: the save saved the folder before the switch finished");
        };
        assert_eq!(current, tip_of(&repo.current().unwrap().1).unwrap());
        assert_eq!(repo.head().unwrap(), journal.target, "fault {n}");
        assert_eq!(Journal::read(&repo).unwrap(), None, "fault {n}");
        assert_holds(dir.path(), NEW);
        assert_eq!(names_in(dir.path()), ["a.txt", "b.txt", "dir", "new"]);
        // Said when the save had a switch to finish; only the journal's removal is silent.
        let finished = kinds(&warnings).contains(&warning::SWITCH_FINISHED);
        assert_eq!(finished, result.is_err() && !took_effect, "fault {n}");
        if result.is_err() && !took_effect {
            stops_before_head += 1;
        }
        if let Ok(done) = result {
            assert!(done.in_use.is_empty());
            // A stop after each change, before the head, and in removing the journal.
            assert!(stops_before_head >= 8, "only {stops_before_head} stops");
            assert!(n > stops_before_head + 1, "no stop after the head moved");
            break;
        }
    }
}

#[test]
fn a_journal_whose_target_is_the_head_is_only_removed() {
    let (dir, mut repo, journal) = stopped_switch();
    let lock = repo.lock_for_change("save", &mut |_| {}).unwrap();
    repo.move_head_to(&lock, journal.target).unwrap();
    assert_eq!(finish_interrupted(&mut repo, &lock).unwrap(), None);
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
    let last = bytes.len() - 1;
    bytes[last] ^= 1;
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
