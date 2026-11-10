//! `bolt discard`, end to end.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;
use std::path::Path;

use bolt_core::diary::Position;
use bolt_core::link::ProjectLink;
use bolt_core::listing::{Entry, EntryKind, Listing};
use bolt_core::pack::ObjectKind;
use bolt_core::repo::Repository;

mod support;
use support::{Env, changed, files, holding, plant_save, saved_files, text, write};

/// A project with `entries` saved.
fn project(entries: &[(&str, &str)]) -> Env {
    let env = Env::empty();
    env.bolt()
        .args(["start", "--preset", "custom"])
        .assert()
        .success();
    for (path, bytes) in entries {
        write(&env, path, bytes.as_bytes());
    }
    env.bolt().args(["save", "Files"]).assert().success();
    env
}

/// `bolt discard <args>` in `dir`, which must succeed; what it printed.
fn discard_in(env: &Env, dir: &Path, args: &[&str]) -> String {
    let out = env.bolt_in(dir).arg("discard").args(args).output().unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    text(&out.stdout)
}

fn discard(env: &Env, args: &[&str]) -> String {
    discard_in(env, env.root(), args)
}

fn discard_json(env: &Env, args: &[&str]) -> serde_json::Value {
    let out = env
        .bolt()
        .arg("discard")
        .args(args)
        .arg("--json")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    serde_json::from_slice(&out.stdout).unwrap()
}

/// `bolt discard <args>`, which must fail; what it said.
fn refused(env: &Env, args: &[&str]) -> String {
    let out = env.bolt().arg("discard").args(args).output().unwrap();
    assert_eq!(out.status.code(), Some(1), "{}", text(&out.stdout));
    text(&out.stderr)
}

fn strings(v: &serde_json::Value) -> Vec<String> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|s| s.as_str().unwrap().to_owned())
        .collect()
}

const SAVED: &[(&str, &str)] = &[("a.txt", "a\n"), ("b.txt", "b\n"), ("dir/c.txt", "c\n")];

#[test]
fn discard_restores_changed_and_deleted_files_and_removes_new_ones() {
    let env = project(SAVED);
    let (_, before) = env.state();
    write(&env, "a.txt", b"mine\n");
    fs::remove_file(env.root().join("b.txt")).unwrap();
    write(&env, "n.txt", b"new\n");
    write(&env, "newdir/deep/x.txt", b"x\n");
    let said = discard(&env, &[]);
    // The deleted file lost nothing, so it needs no copy.
    assert!(
        said.contains(
            "Threw away your unsaved changes to 4 files.\n\n    Safety copies of the 3 that held \
             unsaved work were kept.\n"
        ),
        "{said}"
    );
    assert_eq!(files(env.root()), holding(SAVED));
    assert!(!env.root().join("newdir").exists());
    assert!(changed(&env).is_empty());
    assert!(!env.root().join(".bolt/state/journal").exists());
    // The project stays on its save; the diary records the discard.
    let (repo, state) = env.state();
    assert_eq!(state.lines, before.lines);
    assert_eq!(state.position, Position::OnLine("main".into()));
    assert_eq!(repo.current().unwrap().1.command, "discard");
}

#[test]
fn discard_leaves_ignored_files_alone() {
    let env = project(&[(".boltignore", "*.log\nbuild/\n"), ("a.txt", "a\n")]);
    write(&env, "a.txt", b"mine\n");
    write(&env, "x.log", b"noise");
    write(&env, "build/out.bin", b"\0built");
    // A new folder goes, but not the ignored file in it.
    write(&env, "tmp/n.txt", b"new");
    write(&env, "tmp/t.log", b"kept");
    discard(&env, &[]);
    let mut wanted = holding(&[(".boltignore", "*.log\nbuild/\n"), ("a.txt", "a\n")]);
    wanted.insert("x.log".into(), b"noise".to_vec());
    wanted.insert("build/out.bin".into(), b"\0built".to_vec());
    wanted.insert("tmp/t.log".into(), b"kept".to_vec());
    assert_eq!(files(env.root()), wanted);
}

#[test]
fn discard_of_a_path_touches_nothing_else() {
    let env = project(SAVED);
    write(&env, "a.txt", b"mine\n");
    write(&env, "m.txt", b"mine too\n");
    write(&env, "dir/c.txt", b"changed\n");
    write(&env, "dir/n.txt", b"new\n");
    let said = discard(&env, &["dir"]);
    assert!(
        said.contains(
            "Threw away your unsaved changes to 2 files.\n\n    A safety copy of each was kept.\n"
        ),
        "{said}"
    );
    let mut wanted = holding(SAVED);
    wanted.insert("a.txt".into(), b"mine\n".to_vec());
    wanted.insert("m.txt".into(), b"mine too\n".to_vec());
    assert_eq!(files(env.root()), wanted);
    assert_eq!(changed(&env), ["a.txt", "m.txt"]);
}

#[test]
fn discard_paths_are_relative_to_the_current_folder() {
    let env = project(SAVED);
    write(&env, "a.txt", b"mine\n");
    write(&env, "dir/c.txt", b"changed\n");
    let said = discard_in(&env, &env.root().join("dir"), &["c.txt"]);
    assert!(said.contains("to dir/c.txt.\n"), "{said}");
    assert_eq!(changed(&env), ["a.txt"]);
    discard_in(&env, &env.root().join("dir"), &["../a.txt"]);
    assert!(changed(&env).is_empty());
}

#[test]
fn discard_keeps_a_safety_copy_of_everything_it_throws_away() {
    let env = project(SAVED);
    let mine: &[(&str, &[u8])] = &[
        ("a.txt", b"mine\n"),
        ("deep/x.txt", b"x\n"),
        ("n.bin", b"\0\x01\xffbinary"),
    ];
    for (path, bytes) in mine {
        write(&env, path, bytes);
    }
    fs::remove_file(env.root().join("b.txt")).unwrap();
    let doc = discard_json(&env, &[]);
    assert_eq!(doc["safety_copies"], 3, "{doc}");
    assert_eq!(strings(&doc["restored"]), ["a.txt", "b.txt"], "{doc}");
    assert_eq!(strings(&doc["removed"]), ["deep/x.txt", "n.bin"], "{doc}");
    // The entry records a switch from a save of what the folder held to the save it is on.
    let repo = Repository::open(env.root()).unwrap();
    let (_, entry) = repo.current().unwrap();
    let change = entry.folder_change.unwrap();
    assert_eq!(change.to, entry.state.lines["main"]);
    let mut held = holding(&[("dir/c.txt", "c\n")]);
    for (path, bytes) in mine {
        held.insert((*path).to_owned(), bytes.to_vec());
    }
    assert_eq!(saved_files(&env, change.from), held);
    assert_eq!(files(env.root()), holding(SAVED));
}

#[test]
fn discard_with_nothing_to_discard_says_so() {
    let env = project(SAVED);
    let before = env.standing();
    let said = discard(&env, &[]);
    assert!(said.contains("Nothing to discard"), "{said}");
    let said = discard(&env, &["dir"]);
    assert!(said.contains("Nothing to discard"), "{said}");
    let doc = discard_json(&env, &[]);
    assert_eq!(doc["restored"], serde_json::json!([]), "{doc}");
    assert_eq!(doc["removed"], serde_json::json!([]), "{doc}");
    assert_eq!(doc["safety_copies"], 0, "{doc}");
    // Nothing was recorded.
    assert_eq!(env.standing(), before);
}

#[test]
fn discard_refuses_paths_outside_the_project_or_in_bolt() {
    let env = project(SAVED);
    write(&env, "a.txt", b"mine\n");
    let before = env.standing();
    let said = refused(&env, &["../elsewhere.txt"]);
    assert!(said.contains("outside the project"), "{said}");
    let said = refused(&env, &[".bolt/state"]);
    assert!(said.contains(".bolt"), "{said}");
    let said = refused(&env, &["a.txt", "nothing-here.txt"]);
    assert!(said.contains("nothing-here.txt"), "{said}");
    assert!(said.contains("bolt status"), "{said}");
    // Nothing was thrown away.
    assert_eq!(env.standing(), before);
    assert_eq!(changed(&env), ["a.txt"]);
}

#[test]
fn discard_leaves_project_links_alone() {
    let env = Env::empty();
    env.bolt()
        .args(["start", "--preset", "custom"])
        .assert()
        .success();
    plant_save(env.root(), |pack| {
        let a = pack.add_content(b"a").unwrap();
        let link = ProjectLink::submodule(&[1; 20]);
        let link_id = pack
            .add_record(ObjectKind::ProjectLink, &link.encode())
            .unwrap();
        let top = Listing::new(vec![
            entry("a.txt", EntryKind::File, a, 1),
            entry("lib", EntryKind::ProjectLink, link_id, 0),
        ])
        .unwrap();
        pack.add_record(ObjectKind::FolderListing, &top.encode())
            .unwrap()
    });
    write(&env, "a.txt", b"mine");
    // What the link's folder holds belongs to the other project.
    write(&env, "lib/f.txt", b"theirs");
    let doc = discard_json(&env, &[]);
    assert_eq!(strings(&doc["restored"]), ["a.txt"], "{doc}");
    assert_eq!(doc["removed"], serde_json::json!([]), "{doc}");
    assert_eq!(
        files(env.root()),
        holding(&[("a.txt", "a"), ("lib/f.txt", "theirs")])
    );
    let said = discard(&env, &["lib"]);
    assert!(said.contains("Nothing to discard"), "{said}");
    assert!(env.root().join("lib/f.txt").exists());
}

#[test]
fn discard_leaves_nested_projects_alone() {
    let env = project(SAVED);
    fs::create_dir_all(env.root().join("vendor/.git")).unwrap();
    write(&env, "vendor/v.txt", b"v");
    write(&env, "n.txt", b"new");
    discard(&env, &[]);
    assert!(env.root().join("vendor/.git").is_dir());
    assert_eq!(fs::read(env.root().join("vendor/v.txt")).unwrap(), b"v");
    assert!(!env.root().join("n.txt").exists());
}

#[test]
fn discard_leaves_entries_this_computer_cant_hold_unwritten() {
    let env = Env::empty();
    env.bolt()
        .args(["start", "--preset", "custom"])
        .assert()
        .success();
    plant_save(env.root(), |pack| {
        let a = pack.add_content(b"a").unwrap();
        let sub = Listing::new(vec![
            // A spelling of `.git`, which no system writes.
            entry(".GIT", EntryKind::File, a, 1),
            entry("y.txt", EntryKind::File, a, 1),
        ])
        .unwrap();
        let sub_id = pack
            .add_record(ObjectKind::FolderListing, &sub.encode())
            .unwrap();
        let top = Listing::new(vec![
            entry("a.txt", EntryKind::File, a, 1),
            entry("sub", EntryKind::Folder, sub_id, 0),
        ])
        .unwrap();
        pack.add_record(ObjectKind::FolderListing, &top.encode())
            .unwrap()
    });
    write(&env, "a.txt", b"mine");
    let doc = discard_json(&env, &[]);
    assert_eq!(strings(&doc["restored"]), ["a.txt", "sub/y.txt"], "{doc}");
    assert_eq!(strings(&doc["not_written"]), ["sub/.GIT"], "{doc}");
    assert_eq!(
        files(env.root()),
        holding(&[("a.txt", "a"), ("sub/y.txt", "a")])
    );
}

fn entry(name: &str, kind: EntryKind, id: bolt_core::id::Id, size: u64) -> Entry {
    Entry {
        name: name.into(),
        kind,
        id,
        size,
    }
}

#[test]
fn discard_json_matches_its_snapshot() {
    let env = project(SAVED);
    write(&env, "a.txt", b"mine\n");
    write(&env, "n.txt", b"new\n");
    insta::assert_json_snapshot!(discard_json(&env, &[]));
}

#[cfg(windows)]
#[test]
fn a_discard_with_a_file_in_use_fails_and_the_next_one_finishes() {
    use std::os::windows::fs::OpenOptionsExt;
    use std::time::{Duration, SystemTime};
    let env = project(SAVED);
    // What b.txt holds, so the store has its safety copy already; old enough for the file-state
    // cache to vouch for, so nothing has to read the held file.
    write(&env, "a.txt", b"b\n");
    let file = fs::File::options()
        .write(true)
        .open(env.root().join("a.txt"))
        .unwrap();
    file.set_modified(SystemTime::now() - Duration::from_secs(3600))
        .unwrap();
    drop(file);
    assert_eq!(changed(&env), ["a.txt"]);
    let held = fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(env.root().join("a.txt"))
        .unwrap();
    let out = env.bolt().arg("discard").output().unwrap();
    assert_eq!(out.status.code(), Some(1), "{}", text(&out.stdout));
    let said = text(&out.stderr);
    assert!(said.contains("error:") && said.contains("a.txt"), "{said}");
    assert!(env.root().join(".bolt/state/journal").exists());
    drop(held);
    discard(&env, &[]);
    assert_eq!(files(env.root()), holding(SAVED));
    assert!(!env.root().join(".bolt/state/journal").exists());
}

#[test]
fn a_file_where_a_saved_empty_folder_was_is_reported_removed() {
    let env = Env::empty();
    env.bolt()
        .args(["start", "--preset", "custom"])
        .assert()
        .success();
    plant_save(env.root(), |pack| {
        let a = pack.add_content(b"a").unwrap();
        let empty = Listing::new(vec![]).unwrap();
        let empty_id = pack
            .add_record(ObjectKind::FolderListing, &empty.encode())
            .unwrap();
        let top = Listing::new(vec![
            entry("a.txt", EntryKind::File, a, 1),
            entry("e", EntryKind::Folder, empty_id, 0),
        ])
        .unwrap();
        pack.add_record(ObjectKind::FolderListing, &top.encode())
            .unwrap()
    });
    write(&env, "a.txt", b"a");
    write(&env, "e", b"mine");
    let said = discard(&env, &[]);
    assert!(
        said.contains(
            "Threw away your unsaved changes to e.\n\n    A safety copy of it was kept.\n"
        ),
        "{said}"
    );
    assert!(env.root().join("e").is_dir());
    let repo = Repository::open(env.root()).unwrap();
    let kept = repo.current().unwrap().1.folder_change.unwrap().from;
    assert_eq!(
        saved_files(&env, kept),
        holding(&[("a.txt", "a"), ("e", "mine")])
    );
}

#[test]
fn a_file_where_a_saved_folder_was_is_counted_with_what_comes_back() {
    let env = project(SAVED);
    fs::remove_dir_all(env.root().join("dir")).unwrap();
    write(&env, "dir", b"mine");
    let doc = discard_json(&env, &[]);
    assert_eq!(strings(&doc["removed"]), ["dir"], "{doc}");
    assert_eq!(strings(&doc["restored"]), ["dir/c.txt"], "{doc}");
    assert_eq!(doc["safety_copies"], 1, "{doc}");
    assert_eq!(files(env.root()), holding(SAVED));
}

#[test]
fn discard_refuses_when_an_ignored_file_is_in_the_way() {
    let env = project(SAVED);
    write(&env, ".boltignore", b"dir\n");
    env.bolt().args(["save", "Ignore"]).assert().success();
    fs::remove_dir_all(env.root().join("dir")).unwrap();
    write(&env, "dir", b"ignored");
    write(&env, "a.txt", b"mine\n");
    let before = (env.standing(), files(env.root()));
    let said = refused(&env, &[]);
    assert!(
        said.contains("in the way") && said.contains("dir"),
        "{said}"
    );
    assert_eq!((env.standing(), files(env.root())), before);
}

#[test]
fn discard_of_a_name_leaves_longer_names_alone() {
    let env = project(&[("a", "a\n"), ("ab", "ab\n")]);
    write(&env, "a", b"mine\n");
    write(&env, "ab", b"mine too\n");
    discard(&env, &["a"]);
    assert_eq!(changed(&env), ["ab"]);
}

#[cfg(unix)]
#[test]
fn discard_removes_a_new_link() {
    let env = project(SAVED);
    std::os::unix::fs::symlink("a.txt", env.root().join("l")).unwrap();
    let doc = discard_json(&env, &[]);
    assert_eq!(strings(&doc["removed"]), ["l"], "{doc}");
    assert_eq!(doc["safety_copies"], 1, "{doc}");
    assert!(env.root().join("l").symlink_metadata().is_err());
}

#[cfg(unix)]
#[test]
fn discard_says_nothing_was_discarded_when_it_leaves_the_only_link() {
    let env = project(SAVED);
    std::os::unix::fs::symlink("/tmp", env.root().join("out")).unwrap();
    let out = env.bolt().arg("discard").output().unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    let (said, warned) = (text(&out.stdout), text(&out.stderr));
    assert_eq!(
        said.trim(),
        "Nothing was discarded: the unsaved link stays as it is."
    );
    assert!(
        warned.contains("Didn't throw away 1 unsaved link") && warned.contains("  out\n"),
        "{warned}"
    );
    assert_eq!(
        fs::read_link(env.root().join("out")).unwrap(),
        Path::new("/tmp")
    );
    let doc = discard_json(&env, &[]);
    let kinds: Vec<&str> = doc["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| w["kind"].as_str().unwrap())
        .collect();
    assert!(kinds.contains(&"links_left"), "{doc}");
}

#[cfg(unix)]
#[test]
fn discard_puts_back_an_executable_bit() {
    use std::os::unix::fs::PermissionsExt;
    let env = project(SAVED);
    let a = env.root().join("a.txt");
    fs::set_permissions(&a, fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(changed(&env), ["a.txt"]);
    let doc = discard_json(&env, &[]);
    assert_eq!(strings(&doc["restored"]), ["a.txt"], "{doc}");
    assert_eq!(
        fs::metadata(&a).unwrap().permissions().mode() & 0o777,
        0o644
    );
    assert!(changed(&env).is_empty());
}

/// The names in `dir`, as the drive spells them.
#[cfg(any(windows, target_os = "macos"))]
fn names_in(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n != ".bolt")
        .collect();
    names.sort();
    names
}

#[cfg(any(windows, target_os = "macos"))]
#[test]
fn on_a_drive_that_ignores_case_paths_match_in_any_case() {
    let env = project(&[("Readme.md", "r\n"), ("b.txt", "b\n")]);
    write(&env, "Readme.md", b"mine\n");
    discard(&env, &["README.MD"]);
    assert!(changed(&env).is_empty());
    // Renamed only in case, and discarded by the name on disk: the saved name comes back.
    fs::rename(env.root().join("Readme.md"), env.root().join("README.md")).unwrap();
    discard(&env, &["README.md"]);
    assert_eq!(names_in(env.root()), ["Readme.md", "b.txt"]);
}

#[cfg(any(windows, target_os = "macos"))]
#[test]
fn a_path_two_saved_names_fold_to_is_refused_with_both() {
    let env = Env::empty();
    env.bolt()
        .args(["start", "--preset", "custom"])
        .assert()
        .success();
    plant_save(env.root(), |pack| {
        let a = pack.add_content(b"a").unwrap();
        let top = Listing::new(vec![
            entry("A.txt", EntryKind::File, a, 1),
            entry("a.txt", EntryKind::File, a, 1),
        ])
        .unwrap();
        pack.add_record(ObjectKind::FolderListing, &top.encode())
            .unwrap()
    });
    let before = env.standing();
    let said = refused(&env, &["A.TXT"]);
    assert!(said.contains("A.txt") && said.contains("a.txt"), "{said}");
    assert_eq!(env.standing(), before);
}
