//! End-to-end proof that the shortcuts which make `status`, `save` and `goto` fast are never
//! taken when they would be wrong: the caches, the 2-second rule, the walk that lists a folder
//! before it looks at its files, and the files a switch has just written.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use std::ffi::OsStr;
use std::fs;
use std::path::Path;
use std::time::{Duration, SystemTime};

use bolt_core::filecache::{FileCache, drive_now};
use bolt_core::id::{Id, hash_content};
use bolt_core::keyed::{self, KEY_LEN};
use bolt_core::repo::Repository;
use bolt_platform::dir::{Dir, EntryMeta};
use support::{Env, changed, copy_dir, main_tip, text};

/// A started project with `files` saved, each last changed an hour ago so the cache vouches for
/// it.
fn project(files: &[(&str, &str)]) -> Env {
    let env = Env::empty();
    env.bolt().arg("start").assert().success();
    for (path, bytes) in files {
        let at = env.root().join(path);
        fs::create_dir_all(at.parent().unwrap()).unwrap();
        fs::write(&at, bytes).unwrap();
        set_time(&at, SystemTime::now() - Duration::from_secs(3600));
    }
    env.bolt().args(["save", "Files"]).assert().success();
    env
}

fn set_time(path: &Path, time: SystemTime) {
    let file = fs::File::options().write(true).open(path).unwrap();
    file.set_modified(time).unwrap();
}

fn modified(path: &Path) -> SystemTime {
    fs::metadata(path).unwrap().modified().unwrap()
}

/// Writes `bytes` over the file at `path` in place, then puts its modified time back.
fn rewrite_keeping_time(path: &Path, bytes: &str) {
    let when = modified(path);
    fs::write(path, bytes).unwrap();
    set_time(path, when);
}

/// What Bolt's own look at the file `name` in the project's top folder finds.
fn look(env: &Env, name: &str) -> EntryMeta {
    Dir::open_top(env.root())
        .unwrap()
        .metadata(OsStr::new(name))
        .unwrap()
}

/// The cache key `bolt` made in the test user's settings folder.
fn user_key(env: &Env) -> [u8; KEY_LEN] {
    fs::read(env.config().join(keyed::KEY_FILE))
        .unwrap()
        .try_into()
        .unwrap()
}

/// Replaces `cache/files` with one sealed by `key` that holds only `path`, with the details
/// `meta` and the content `id`, its time compared with `now`.
fn plant_file_cache(
    env: &Env,
    key: [u8; KEY_LEN],
    path: &str,
    meta: &EntryMeta,
    id: Id,
    now: i128,
) {
    let mut repo = Repository::open(env.root()).unwrap();
    repo.set_cache_key(key);
    let mut cache = FileCache::default();
    cache.record(path, meta, id);
    cache.store(&repo, now).unwrap();
}

/// The drive's own time now, as a scan reads it.
fn drive_time(env: &Env) -> i128 {
    drive_now(&Repository::open(env.root()).unwrap()).unwrap()
}

fn change_ids_cache(root: &Path) -> std::path::PathBuf {
    root.join(".bolt/cache/change-ids")
}

/// A started project with `count` saves on `main`, each adding a file; their IDs and change IDs,
/// oldest first. `bolt history` has filled the change-ID cache.
fn saves(count: usize) -> (Env, Vec<(Id, String)>) {
    let env = Env::empty();
    env.bolt().arg("start").assert().success();
    let mut made = Vec::new();
    for n in 0..count {
        add_save(&env, n, &mut made);
    }
    env.bolt().arg("history").assert().success();
    assert!(change_ids_cache(env.root()).is_file());
    (env, made)
}

fn add_save(env: &Env, n: usize, made: &mut Vec<(Id, String)>) {
    support::write(env, &format!("f{n}.txt"), format!("file {n}"));
    env.bolt()
        .args(["save", &format!("Save {n}")])
        .assert()
        .success();
    let id = main_tip(env);
    let repo = Repository::open(env.root()).unwrap();
    let letters = repo
        .store()
        .save(&id, false)
        .unwrap()
        .change_id
        .to_letters();
    made.push((id, letters));
}

/// Two copies of the project: the first with no change-ID cache, the second with `planted` as
/// its cache.
fn with_and_without(env: &Env, planted: &[u8]) -> (tempfile::TempDir, tempfile::TempDir) {
    let (without, with) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    copy_dir(env.root(), without.path());
    copy_dir(env.root(), with.path());
    fs::remove_file(change_ids_cache(without.path())).unwrap();
    fs::write(change_ids_cache(with.path()), planted).unwrap();
    (without, with)
}

/// `undo --list`'s lines without the diary entries' own IDs, which hold the time each was made.
fn without_entry_ids(said: &str) -> String {
    said.lines()
        .map(|line| match line.splitn(3, "  ").collect::<Vec<_>>()[..] {
            [step, id, rest] if id.len() == 12 && id.bytes().all(|b| b.is_ascii_hexdigit()) => {
                format!("{step}  {rest}\n")
            }
            _ => format!("{line}\n"),
        })
        .collect()
}

/// Runs each command in both projects, which must say the same.
fn same_in_both(env: &Env, without: &Path, with: &Path, commands: &[&[&str]]) {
    for args in commands {
        let run = |dir: &Path| {
            let out = env.bolt_in(dir).args(*args).output().unwrap();
            (
                out.status.code(),
                without_entry_ids(&text(&out.stdout)),
                text(&out.stderr),
            )
        };
        let (expected, got) = (run(without), run(with));
        assert_eq!(got, expected, "bolt {args:?}");
    }
}

fn current_save(dir: &Path) -> Id {
    bolt_core::history::current_save(&Repository::open(dir).unwrap()).unwrap()
}

#[test]
fn a_planted_cache_is_never_trusted() {
    // A file changed in place, with its size and modified time kept, and a cache that vouches for
    // it as saved: sealed by another key, it is ignored.
    let env = project(&[("a.txt", "one"), ("b.txt", "bee")]);
    rewrite_keeping_time(&env.root().join("a.txt"), "two");
    let now = look(&env, "a.txt");
    let saved = hash_content(b"one");
    plant_file_cache(
        &env,
        [0x5a; KEY_LEN],
        "a.txt",
        &now,
        saved,
        drive_time(&env),
    );
    assert_eq!(changed(&env), ["a.txt"]);
    // Sealed by this user's key, the same cache is believed: the seal alone refused it.
    plant_file_cache(&env, user_key(&env), "a.txt", &now, saved, drive_time(&env));
    assert!(changed(&env).is_empty());

    // A change-ID cache that gives the newest save the oldest one's change ID, and the oldest the
    // newest's, sealed by another key: every command says what it says with no cache.
    let (env, made) = saves(4);
    let sealed = fs::read(change_ids_cache(env.root())).unwrap();
    let body = keyed::open_sealed(&user_key(&env), &sealed).unwrap();
    let repo = Repository::open(env.root()).unwrap();
    let change_id = |id: &Id| repo.store().save(id, false).unwrap().change_id.0;
    let (newest, oldest) = (change_id(&made[3].0), change_id(&made[0].0));
    let mut lying = body.to_vec();
    let at = |bytes: &[u8], id: &[u8; 16]| bytes.windows(16).position(|w| w == id).unwrap();
    let (n, o) = (at(&lying, &newest), at(&lying, &oldest));
    lying[n..n + 16].copy_from_slice(&oldest);
    lying[o..o + 16].copy_from_slice(&newest);
    let planted = keyed::seal(&[0x5a; KEY_LEN], &lying);
    let (without, with) = with_and_without(&env, &planted);
    goes_to_each(
        &env,
        without.path(),
        with.path(),
        &[&made[0], &made[3], &made[1]],
    );
}

/// From `main`, `goto` each save by its change ID, in both projects, which must say the same
/// before and after each, and must get to it.
fn goes_to_each(env: &Env, without: &Path, with: &Path, saves: &[&(Id, String)]) {
    let looks: &[&[&str]] = &[&["status"], &["history"], &["undo", "--list"]];
    same_in_both(env, without, with, looks);
    for (id, letters) in saves {
        same_in_both(env, without, with, &[&["goto", "main"], &["goto", letters]]);
        assert_eq!(current_save(with), *id);
        same_in_both(env, without, with, looks);
    }
}

#[test]
fn a_stale_change_id_cache_gives_what_the_store_gives() {
    // Taken before two more saves were made, and sealed by the right key.
    let (env, mut made) = saves(3);
    let old = fs::read(change_ids_cache(env.root())).unwrap();
    add_save(&env, 3, &mut made);
    add_save(&env, 4, &mut made);
    let (without, with) = with_and_without(&env, &old);
    goes_to_each(
        &env,
        without.path(),
        with.path(),
        &[&made[4], &made[0], &made[3]],
    );
}

#[test]
fn a_stale_cache_is_only_trusted_file_by_file() {
    // Save, copy the cache away, change the file (same size, new time), put the old cache back.
    let env = project(&[("a.txt", "one"), ("b.txt", "bee")]);
    let old = fs::read(cache_of(&env)).unwrap();
    fs::write(env.root().join("a.txt"), "two").unwrap();
    fs::write(cache_of(&env), &old).unwrap();
    assert_eq!(changed(&env), ["a.txt"]);

    // The same file written over in place with its modified time kept: its change time tells.
    let env = project(&[("a.txt", "one"), ("b.txt", "bee")]);
    let old = fs::read(cache_of(&env)).unwrap();
    rewrite_keeping_time(&env.root().join("a.txt"), "two");
    fs::write(cache_of(&env), &old).unwrap();
    assert_eq!(changed(&env), ["a.txt"]);

    // Another file moved onto the path, with the same size and times: its file ID tells.
    let env = project(&[("a.txt", "one"), ("b.txt", "bee")]);
    let old = fs::read(cache_of(&env)).unwrap();
    let a = env.root().join("a.txt");
    let other = env.root().join("other.tmp");
    fs::write(&other, "two").unwrap();
    set_time(&other, modified(&a));
    fs::rename(&other, &a).unwrap();
    fs::write(cache_of(&env), &old).unwrap();
    assert_eq!(changed(&env), ["a.txt"]);

    // On disk a change moves more than one detail at once: the change time can't be set back.
    // So each detail is tried alone with a cache that differs from the file in just that one,
    // sealed by the right key and vouching for the saved content.
    let env = project(&[("a.txt", "one"), ("b.txt", "bee")]);
    rewrite_keeping_time(&env.root().join("a.txt"), "two");
    let now = look(&env, "a.txt");
    let saved = hash_content(b"one");
    let key = user_key(&env);
    let later = drive_time(&env);
    let but = |change: fn(&mut EntryMeta)| {
        let mut meta = now;
        change(&mut meta);
        meta
    };
    let mut cases = vec![
        ("the same", now, later, true),
        ("size", but(|m| m.size += 1), later, false),
        ("modified time", but(|m| m.modified_ns -= 1), later, false),
        ("change time", but(|m| m.changed_ns -= 1), later, false),
        ("drive", but(|m| m.file_id.0 ^= 1), later, false),
        ("file number", but(|m| m.file_id.1 ^= 1), later, false),
        // Recorded less than 2 s after the file's time: racy, never trusted.
        ("2-second rule", now, now.modified_ns + 1_000_000_000, false),
    ];
    // Windows keeps no executable bit.
    if cfg!(unix) {
        let kind = but(|m| m.executable = Some(!m.executable.unwrap_or(false)));
        cases.push(("kind", kind, later, false));
    }
    for (detail, recorded, at, believed) in cases {
        plant_file_cache(&env, key, "a.txt", &recorded, saved, at);
        let expected: &[&str] = if believed { &[] } else { &["a.txt"] };
        assert_eq!(changed(&env), expected, "{detail}");
    }
}

fn cache_of(env: &Env) -> std::path::PathBuf {
    env.root().join(".bolt/cache/files")
}

#[test]
fn a_file_changed_within_two_seconds_is_read_again() {
    // Written and saved at once: the cache the save leaves doesn't vouch for it.
    let env = Env::empty();
    env.bolt().arg("start").assert().success();
    let a = env.root().join("a.txt");
    fs::write(&a, "one").unwrap();
    fs::write(env.root().join("old.txt"), "old").unwrap();
    set_time(
        &env.root().join("old.txt"),
        SystemTime::now() - Duration::from_secs(3600),
    );
    env.bolt().args(["save", "Fresh"]).assert().success();
    let sealed = fs::read(cache_of(&env)).unwrap();
    let body = keyed::open_sealed(&user_key(&env), &sealed).unwrap();
    let cache = FileCache::decode(body).unwrap();
    assert!(cache.matches("old.txt", &look(&env, "old.txt")).is_some());
    assert!(cache.matches("a.txt", &look(&env, "a.txt")).is_none());
    // Changed again at once, with the same size and time: the next status reads it.
    rewrite_keeping_time(&a, "two");
    assert_eq!(changed(&env), ["a.txt"]);
}

#[test]
fn status_after_goto_reads_what_goto_wrote() {
    let env = project(&[("a.txt", "one")]);
    let first = main_tip(&env);
    fs::write(env.root().join("a.txt"), "two").unwrap();
    env.bolt().args(["save", "Two"]).assert().success();
    env.bolt()
        .args(["goto", &first.to_hex()])
        .assert()
        .success();
    // Changed right after the switch wrote it, with the same size and time.
    rewrite_keeping_time(&env.root().join("a.txt"), "uno");
    assert_eq!(changed(&env), ["a.txt"]);
}

/// Tests that change the disk at a point the walk reaches on a thread of its own. That point is
/// shared by the whole process, so these run the walk here, one at a time.
#[cfg(feature = "fault-injection")]
mod mid_walk {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};

    use bolt_core::fsio::fault;
    use bolt_core::snapshot::{SaveError, SaveOutcome, SaveRequest, save_folder};
    use bolt_core::status::{Status, status};

    use super::*;

    static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());

    /// Runs `command` on the project, in this process and with the test user's key if `keyed`,
    /// and `change` when its walk first starts looking at files.
    fn changing<T>(
        env: &Env,
        keyed: bool,
        change: impl FnOnce() + Send + 'static,
        command: impl FnOnce(&mut Repository) -> T,
    ) -> T {
        let _alone = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
        let done = Arc::new(AtomicBool::new(false));
        let ran = Arc::clone(&done);
        fault::act_at_on_any_thread("walk:looking", move || {
            change();
            ran.store(true, Ordering::SeqCst);
        });
        let mut repo = Repository::open(env.root()).unwrap();
        if keyed {
            repo.set_cache_key(user_key(env));
        }
        let result = command(&mut repo);
        assert!(
            done.load(Ordering::SeqCst),
            "the walk never looked at a file"
        );
        result
    }

    fn status_here(repo: &mut Repository) -> Result<Status, SaveError> {
        status(repo, false, None)
    }

    fn save_here(repo: &mut Repository) -> Result<SaveOutcome, SaveError> {
        let who = support::ada();
        let req = SaveRequest {
            message: Some("Swapped"),
            who: &who,
            command: "save",
            accept_crlf: false,
            git_excludes: None,
            new_line: None,
            only: None,
            amend: false,
        };
        save_folder(repo, &req, &mut |_| {}, &mut |_| true)
    }

    fn paths(status: &Status) -> Vec<String> {
        status.changes.iter().map(|c| c.path.clone()).collect()
    }

    #[test]
    fn a_file_changed_during_the_walk_is_seen() {
        let env = project(&[("a.txt", "one")]);
        let a = env.root().join("a.txt");
        let change = move || rewrite_keeping_time(&a, "two");
        let found = changing(&env, true, change, status_here).unwrap();
        assert_eq!(paths(&found), ["a.txt"]);
        assert_eq!(changed(&env), ["a.txt"]);
    }

    /// A project holding `sub/a.txt`, with `more` written after its save, and a change that swaps
    /// `sub` for a link to a folder elsewhere holding another `a.txt` of the same size and time.
    fn swap_for_a_link(more: &[(&str, &str)]) -> (Env, tempfile::TempDir, impl FnOnce() + Send) {
        let env = project(&[("sub/a.txt", "one")]);
        for (path, bytes) in more {
            support::write(&env, path, bytes);
        }
        let sub = env.root().join("sub");
        let elsewhere = tempfile::tempdir_in(env.root().parent().unwrap()).unwrap();
        let other = elsewhere.path().join("other");
        fs::create_dir(&other).unwrap();
        fs::write(other.join("a.txt"), "two").unwrap();
        set_time(&other.join("a.txt"), modified(&sub.join("a.txt")));
        let gone = elsewhere.path().join("gone");
        let swap = move || {
            fs::rename(&sub, &gone).unwrap();
            support::link_folder(&other, &sub);
        };
        (env, elsewhere, swap)
    }

    /// What the status after the swap lists: the link, and the saved file gone from under it.
    fn after_the_swap(env: &Env) {
        assert_eq!(changed(env), ["sub", "sub/a.txt"]);
    }

    #[test]
    fn a_folder_swapped_for_a_link_mid_walk_is_refused() {
        // A save that reads a file of the swapped folder, through the cache's key or without it.
        for (more, keyed) in [(&[("sub/new.txt", "new")][..], true), (&[][..], false)] {
            let (env, _elsewhere, swap) = swap_for_a_link(more);
            let head = Repository::open(env.root()).unwrap().head().unwrap();
            match changing(&env, keyed, swap, save_here) {
                Err(SaveError::ChangedDuringSave { path }) => assert_eq!(path, "sub"),
                other => panic!("{:?}", other.map(|_| ())),
            }
            assert_eq!(Repository::open(env.root()).unwrap().head().unwrap(), head);
            after_the_swap(&env);
        }
        // With every file of it vouched for, nothing is read there: the status answers for the
        // folder as it was listed, never a mix of the two, and the next one lists the link.
        let (env, _elsewhere, swap) = swap_for_a_link(&[]);
        match changing(&env, true, swap, status_here) {
            Ok(found) => assert!(found.changes.is_empty(), "{:?}", found.changes),
            Err(e) => assert!(matches!(e, SaveError::ChangedDuringSave { .. }), "{e:?}"),
        }
        after_the_swap(&env);
    }

    #[test]
    fn a_git_folder_made_mid_walk_is_seen_by_the_next_command() {
        let env = project(&[("sub/a.txt", "one"), ("b.txt", "bee")]);
        let git = env.root().join("sub/.git");
        let make = move || {
            fs::create_dir(&git).unwrap();
            fs::write(git.join("HEAD"), "ref: refs/heads/main\n").unwrap();
        };
        let found = changing(&env, true, make, status_here).unwrap();
        assert!(found.changes.is_empty(), "{:?}", found.changes);
        // Left out from now on, as a project of its own, with what was saved of it kept.
        let out = env.bolt().args(["--json", "status"]).output().unwrap();
        assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
        let doc: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
        assert_eq!(doc["changes"], serde_json::json!([]), "{doc}");
        assert_eq!(
            doc["warnings"][0]["kind"], "nested_projects_left_out",
            "{doc}"
        );
        let said = doc["warnings"][0]["message"].as_str().unwrap();
        assert!(said.contains("\n  sub\n"), "{said}");
    }
}
