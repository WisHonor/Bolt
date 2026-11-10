//! End-to-end tests of `bolt status`.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::time::{Duration, SystemTime};

use bolt_core::link::ProjectLink;
use bolt_core::listing::{Entry, EntryKind, Listing};
use bolt_core::pack::ObjectKind;

mod support;
use support::{Env, plant_save, text};

/// A project with `files` saved, each last changed an hour ago so the cache can vouch for it.
fn project(files: &[(&str, &str)]) -> Env {
    let env = Env::empty();
    env.bolt()
        .args(["start", "--preset", "custom"])
        .assert()
        .success();
    for (name, content) in files {
        write_old(&env.root().join(name), content.as_bytes());
    }
    env.bolt().args(["save", "Files"]).assert().success();
    env
}

/// Writes `path`, then sets its time an hour back.
fn write_old(path: &Path, bytes: &[u8]) {
    if let Some(up) = path.parent() {
        fs::create_dir_all(up).unwrap();
    }
    fs::write(path, bytes).unwrap();
    set_time(path, SystemTime::now() - Duration::from_secs(3600));
}

fn set_time(path: &Path, time: SystemTime) {
    let file = fs::File::options().write(true).open(path).unwrap();
    file.set_modified(time).unwrap();
}

/// `bolt --json status` with `args`, which must succeed.
fn status_json(env: &Env, args: &[&str]) -> serde_json::Value {
    let out = env
        .bolt()
        .args(["--json", "status"])
        .args(args)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    serde_json::from_slice(&out.stdout).unwrap()
}

/// Each change as `(path, change, kind)`.
fn changes(doc: &serde_json::Value) -> Vec<(String, String, String)> {
    let s = |v: &serde_json::Value| v.as_str().unwrap().to_owned();
    doc["changes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| (s(&c["path"]), s(&c["change"]), s(&c["kind"])))
        .collect()
}

fn change(path: &str, change: &str, kind: &str) -> (String, String, String) {
    (path.into(), change.into(), kind.into())
}

/// Makes `c` another kind of file than it was: executable on Unix, a link where Windows allows
/// one. False where neither can be done.
fn change_kind(c: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(c, fs::Permissions::from_mode(0o755)).unwrap();
        true
    }
    #[cfg(windows)]
    {
        fs::remove_file(c).unwrap();
        if std::os::windows::fs::symlink_file("b.txt", c).is_ok() {
            return true;
        }
        fs::write(c, "c").unwrap();
        false
    }
}

#[test]
fn status_lists_added_modified_removed_and_kind_changes() {
    let env = project(&[
        ("a.txt", "a"),
        ("b.txt", "b"),
        ("c", "c"),
        ("sub/s.txt", "s"),
    ]);
    fs::write(env.root().join("a.txt"), "changed").unwrap();
    fs::remove_file(env.root().join("b.txt")).unwrap();
    fs::remove_dir_all(env.root().join("sub")).unwrap();
    // A name that reads backwards in a terminal unless escaped.
    fs::write(env.root().join("new\u{202e}.txt"), "new").unwrap();
    let kind_changed = change_kind(&env.root().join("c"));
    let doc = status_json(&env, &[]);
    let mut expected = vec![
        change("a.txt", "modified", "file"),
        change("b.txt", "removed", "file"),
    ];
    if kind_changed {
        let kind = if cfg!(unix) { "executable" } else { "symlink" };
        expected.push(change("c", "kind-changed", kind));
    }
    expected.push(change("new\u{202e}.txt", "added", "file"));
    expected.push(change("sub/s.txt", "removed", "file"));
    assert_eq!(changes(&doc), expected, "{doc}");
    assert_eq!(doc["changes"][0]["old_size"], 1);
    assert_eq!(doc["changes"][0]["new_size"], 7);
    assert_eq!(doc["position"], serde_json::json!({"line": "main"}));
    // For people: the position, then a line per change, the name escaped.
    let out = env.bolt().arg("status").output().unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    let said = text(&out.stdout);
    assert!(said.starts_with("On line main at save "), "{said}");
    // The kind column is as wide as its widest word: "kind changed" where a kind changed.
    let width = if kind_changed { 12 } else { 8 };
    for (kind, path) in [
        ("modified", "a.txt"),
        ("removed", "b.txt"),
        ("added", "new\\u{202e}.txt"),
        ("removed", "sub/s.txt"),
    ] {
        let line = format!("    {kind:<width$}  {path} ");
        assert!(said.contains(&line), "{line:?} in {said}");
    }
    assert!(!said.contains('\u{202e}'), "{said}");
    // Saving makes it clean again.
    env.bolt().args(["save", "More"]).assert().success();
    let said = text(&env.bolt().arg("status").output().unwrap().stdout);
    assert!(said.contains("Nothing changed since "), "{said}");
    assert!(changes(&status_json(&env, &[])).is_empty());
}

/// Makes `path` something Bolt can't open, and gives what keeps it so; `None` where nothing
/// can (root reads everything).
fn lock(path: &Path) -> Option<Box<dyn std::any::Any>> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        struct Unlock(std::path::PathBuf);
        impl Drop for Unlock {
            fn drop(&mut self) {
                let _ = fs::set_permissions(&self.0, fs::Permissions::from_mode(0o644));
            }
        }
        fs::set_permissions(path, fs::Permissions::from_mode(0o000)).unwrap();
        let unlock = Unlock(path.to_owned());
        fs::read(path)
            .is_err()
            .then(|| Box::new(unlock) as Box<dyn std::any::Any>)
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        let file = fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(path)
            .unwrap();
        Some(Box::new(file))
    }
}

#[cfg(any(windows, feature = "fault-injection"))]
#[test]
fn status_does_not_open_unchanged_files() {
    let env = project(&[("a.txt", "a"), ("locked.txt", "secret")]);
    // The first status reads every file and caches what it saw.
    assert!(changes(&status_json(&env, &[])).is_empty());
    // Bolt can't open it now. Not by taking the right to read it on Unix: that moves its change
    // time, and the cache stops vouching for it.
    #[cfg(windows)]
    let _held = lock(&env.root().join("locked.txt"));
    fs::write(env.root().join("a.txt"), "changed").unwrap();
    let status = |args: &[&str]| {
        env.bolt()
            .env("BOLT_FAULT_UNREADABLE", "locked.txt")
            .args(args)
            .output()
            .unwrap()
    };
    let out = status(&["--json", "status"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    let doc: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(
        changes(&doc),
        [change("a.txt", "modified", "file")],
        "{doc}"
    );
    // And a status that must open it can't.
    let out = status(&["status", "--full"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(text(&out.stderr).contains("Can't read locked.txt"));
}

#[test]
fn status_full_reads_everything() {
    let env = project(&[("a.txt", "aaa"), ("b.txt", "b")]);
    assert!(changes(&status_json(&env, &[])).is_empty());
    // Same size, same time, written over in place: its change time tells (decision 069).
    let a = env.root().join("a.txt");
    let time = fs::metadata(&a).unwrap().modified().unwrap();
    fs::write(&a, "bbb").unwrap();
    set_time(&a, time);
    let doc = status_json(&env, &[]);
    assert_eq!(
        changes(&doc),
        [change("a.txt", "modified", "file")],
        "{doc}"
    );
    let doc = status_json(&env, &["--full"]);
    assert_eq!(
        changes(&doc),
        [change("a.txt", "modified", "file")],
        "{doc}"
    );
    // And --full opens the files the cache vouches for.
    let Some(_held) = lock(&env.root().join("b.txt")) else {
        return;
    };
    let out = env.bolt().args(["status", "--full"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    // Status only reads, so it says nothing of saving.
    let said = text(&out.stderr);
    assert!(said.contains("Can't read b.txt"), "{said}");
    assert!(
        said.contains("hint: Let Bolt read it, or list it in .boltignore."),
        "{said}"
    );
    assert!(!said.contains("save"), "{said}");
}

/// Every file in `.bolt` but the caches and temporary files, with its contents.
fn store_files(root: &Path) -> BTreeMap<String, Vec<u8>> {
    let bolt = root.join(".bolt");
    let mut found = BTreeMap::new();
    let mut todo = vec![bolt.clone()];
    while let Some(dir) = todo.pop() {
        for entry in fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            let rel = path
                .strip_prefix(&bolt)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            if rel == "cache" || rel == "tmp" {
                continue;
            }
            if path.is_dir() {
                todo.push(path);
            } else {
                found.insert(rel, fs::read(&path).unwrap());
            }
        }
    }
    found
}

#[test]
fn status_never_writes_to_the_store() {
    let env = project(&[("a.txt", "a"), ("sub/b.txt", "b")]);
    fs::write(env.root().join("a.txt"), "changed").unwrap();
    fs::write(env.root().join("sub/new.txt"), "new").unwrap();
    fs::remove_file(env.root().join("sub/b.txt")).unwrap();
    let before = store_files(env.root());
    for args in [&[][..], &["--full"]] {
        assert_eq!(changes(&status_json(&env, args)).len(), 3);
    }
    assert_eq!(store_files(env.root()), before);
    // The file-state cache is all it writes.
    assert!(env.root().join(".bolt/cache/files").is_file());
}

#[test]
fn a_cache_that_cant_be_written_is_a_warning() {
    let env = project(&[("a.txt", "a")]);
    // The save wrote the cache; a folder takes its place.
    fs::remove_file(env.root().join(".bolt/cache/files")).unwrap();
    fs::create_dir_all(env.root().join(".bolt/cache/files/in-the-way")).unwrap();
    let doc = status_json(&env, &[]);
    let kinds: Vec<&str> = doc["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| w["kind"].as_str().unwrap())
        .collect();
    assert_eq!(kinds, ["file_cache_not_stored"], "{doc}");
    let said = doc["warnings"][0]["message"].as_str().unwrap();
    assert!(
        said.starts_with(
            "Bolt couldn't keep what it learned about your files, so the next status or save \
             reads them all again: "
        ),
        "{said}"
    );
    assert!(changes(&doc).is_empty());
}

#[test]
fn ignored_files_and_nested_projects_are_not_listed() {
    let env = project(&[
        (
            ".boltignore",
            "*.log
",
        ),
        ("a.txt", "a"),
    ]);
    fs::write(env.root().join("x.log"), "noise").unwrap();
    fs::create_dir_all(env.root().join("vendor/.git")).unwrap();
    fs::write(env.root().join("vendor/v.txt"), "v").unwrap();
    let doc = status_json(&env, &[]);
    assert!(changes(&doc).is_empty(), "{doc}");
    let kinds: Vec<&str> = doc["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| w["kind"].as_str().unwrap())
        .collect();
    assert_eq!(kinds, ["nested_projects_left_out"], "{doc}");
    assert!(
        doc["warnings"][0]["message"]
            .as_str()
            .unwrap()
            .contains("vendor")
    );
}

#[test]
fn a_file_changed_too_close_to_the_cache_is_read_again() {
    let env = project(&[("a.txt", "aaa")]);
    // A time the cache can't trust whenever it is written: past the drive's "now".
    let a = env.root().join("a.txt");
    let soon = SystemTime::now() + Duration::from_secs(600);
    set_time(&a, soon);
    assert!(changes(&status_json(&env, &[])).is_empty());
    // Same size, same time: only reading the file tells.
    fs::write(&a, "bbb").unwrap();
    set_time(&a, soon);
    let doc = status_json(&env, &[]);
    assert_eq!(
        changes(&doc),
        [change("a.txt", "modified", "file")],
        "{doc}"
    );
}

/// A save whose top folder holds `a.txt`, the project link `lib`, which no computer holds as it
/// is, and the folder `deps` holding another and `x.txt`.
fn plant_links(root: &Path) {
    plant_save(root, |pack| {
        let a = pack.add_content(b"a").unwrap();
        let link = ProjectLink::submodule(&[1; 20]);
        let link_id = pack
            .add_record(ObjectKind::ProjectLink, &link.encode())
            .unwrap();
        let entry = |name: &str, kind, id, size| Entry {
            name: name.into(),
            kind,
            id,
            size,
        };
        let deps = Listing::new(vec![
            entry("lib", EntryKind::ProjectLink, link_id, 0),
            entry("x.txt", EntryKind::File, a, 1),
        ])
        .unwrap();
        let deps_id = pack
            .add_record(ObjectKind::FolderListing, &deps.encode())
            .unwrap();
        let top = Listing::new(vec![
            entry("a.txt", EntryKind::File, a, 1),
            entry("deps", EntryKind::Folder, deps_id, 0),
            entry("lib", EntryKind::ProjectLink, link_id, 0),
        ])
        .unwrap();
        pack.add_record(ObjectKind::FolderListing, &top.encode())
            .unwrap()
    });
}

#[test]
fn entries_this_computer_cant_hold_are_listed_as_not_written() {
    let env = Env::empty();
    env.bolt()
        .args(["start", "--preset", "custom"])
        .assert()
        .success();
    write_old(&env.root().join("a.txt"), b"a");
    plant_links(env.root());
    // `deps` is gone from disk, and its link stays where the save has it.
    let doc = status_json(&env, &[]);
    assert_eq!(
        changes(&doc),
        [change("deps/x.txt", "removed", "file")],
        "{doc}"
    );
    assert_eq!(doc["not_written"], serde_json::json!(["deps/lib", "lib"]));
    let said = text(&env.bolt().arg("status").output().unwrap().stdout);
    assert!(
        said.ends_with(
            "Not written here, because this computer can't hold them as they are (saves keep \
             them as they were):\n\n    deps/lib\n    lib\n"
        ),
        "{said}"
    );
    // With the file back, nothing changed, and both stay listed.
    write_old(&env.root().join("deps/x.txt"), b"a");
    let doc = status_json(&env, &[]);
    assert!(changes(&doc).is_empty(), "{doc}");
    assert_eq!(doc["not_written"], serde_json::json!(["deps/lib", "lib"]));
    let said = text(&env.bolt().arg("status").output().unwrap().stdout);
    assert!(said.contains("Nothing changed since "), "{said}");
}

#[test]
fn status_says_in_its_own_words_why_it_stops() {
    let env = Env::empty();
    env.bolt()
        .args(["start", "--preset", "custom"])
        .assert()
        .success();
    write_old(&env.root().join("a.txt"), b"a");
    plant_links(env.root());
    // The save keeps deps/lib as it is, so deps can't be a file.
    write_old(&env.root().join("deps"), b"a file");
    let out = env.bolt().arg("status").output().unwrap();
    assert_eq!(out.status.code(), Some(1), "{}", text(&out.stdout));
    assert_eq!(
        text(&out.stderr),
        "error: Bolt can't show the status: deps: the last save has a folder there holding \
         deps/lib, which every save keeps as it was because this computer can't hold it as it \
         is, and a save can't hold both under one name.\n\nhint: Rename or move deps.\n"
    );
}

#[test]
fn a_save_holding_a_spelling_of_bolt_is_named() {
    let env = Env::empty();
    env.bolt()
        .args(["start", "--preset", "custom"])
        .assert()
        .success();
    // No listing Bolt makes holds `.BOLT`, so it is spelled `-BOLT`, then the bytes are changed.
    plant_save(env.root(), |pack| {
        let id = pack.add_content(b"x").unwrap();
        let top = Listing::new(vec![Entry {
            name: "-BOLT".into(),
            kind: EntryKind::File,
            id,
            size: 1,
        }])
        .unwrap()
        .encode();
        let at = top.windows(5).position(|w| w == b"-BOLT").unwrap();
        let mut hostile = top.clone();
        hostile[at] = b'.';
        pack.add_record(ObjectKind::FolderListing, &hostile)
            .unwrap()
    });
    let planted = env.state().1.lines["main"];
    let out = env.bolt().arg("status").output().unwrap();
    assert_eq!(out.status.code(), Some(1), "{}", text(&out.stdout));
    assert_eq!(
        text(&out.stderr),
        format!(
            "error: Save {} holds a name spelled like Bolt's own folder (.bolt), which Bolt never \
             writes, so it can't be used.\n\nhint: Bolt never makes such a save, so another program \
             put it there. Nothing was changed: ask for help before trying to repair it.\n",
            planted.short_hex()
        )
    );
}

#[test]
fn status_json_matches_its_snapshot() {
    let env = project(&[("a.txt", "a"), ("b.txt", "b")]);
    fs::write(env.root().join("a.txt"), "changed").unwrap();
    fs::remove_file(env.root().join("b.txt")).unwrap();
    fs::write(env.root().join("c.txt"), "c").unwrap();
    let mut doc = status_json(&env, &[]);
    let redact = |v: &mut serde_json::Value| *v = "[redacted]".into();
    let save = &mut doc["save"];
    for key in ["id", "change_id", "change_id_short"] {
        redact(&mut save[key]);
    }
    for person in ["author", "saver"] {
        for key in ["time_ms", "tz_offset_min"] {
            redact(&mut save[person][key]);
        }
    }
    insta::assert_json_snapshot!(doc);
}

#[test]
fn a_reader_that_goes_away_ends_it_quietly() {
    use std::io::Read;
    use std::process::Stdio;
    let env = project(&[]);
    // More than any pipe holds, so status is still printing when the reader leaves.
    for i in 0..4000 {
        fs::write(env.root().join(format!("{i:0>60}.txt")), "x").unwrap();
    }
    let mut child = env
        .bolt_process_in(env.root())
        .arg("status")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdout = child.stdout.take().unwrap();
    let mut first = [0; 16];
    stdout.read_exact(&mut first).unwrap();
    drop(stdout);
    let out = child.wait_with_output().unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    assert_eq!(text(&out.stderr), "");
}
