#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use super::*;
use crate::chunks::{Chunk, ChunkList};
use crate::diary::{Action, FolderChange};
use crate::history::tip_of;
use crate::id::hash_content;
use crate::listing::Entry;
use crate::pack::{PackSet, PackWriter};
use crate::save::Person;
use crate::snapshot::{SaveOutcome, SaveRequest, save_folder};
use crate::switch::discard;
use crate::test_support::{add_listing, lock_pack, plant_damaged_copies, plant_root, plant_save};
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

const DAY: i64 = 86_400_000;

fn make(
    n: usize,
) -> (
    tempfile::TempDir,
    std::path::PathBuf,
    Vec<std::path::PathBuf>,
) {
    let root = tempfile::tempdir().unwrap();
    let tmp = root.path().join("tmp");
    let packs = root.path().join("packs");
    std::fs::create_dir(&tmp).unwrap();
    std::fs::create_dir(&packs).unwrap();
    let paths = (0..n)
        .map(|i| {
            let mut w = PackWriter::create(&tmp).unwrap();
            w.add_content(format!("content number {i}").as_bytes())
                .unwrap();
            w.add_content(&vec![i as u8; 500]).unwrap();
            w.finish(&packs).unwrap().unwrap()
        })
        .collect();
    (root, packs, paths)
}

#[test]
fn verify_catches_corruption() {
    let (_root, packs, paths) = make(3);
    let report = verify_store(&packs);
    assert!(report.is_ok(), "{:?}", report.problems);
    assert_eq!(report.objects_checked, 6);

    let entry = PackReader::open(&paths[1])
        .unwrap()
        .entries()
        .next()
        .unwrap()
        .unwrap();
    let mut bytes = std::fs::read(&paths[1]).unwrap();
    bytes[(entry.offset + ENTRY_HEADER_LEN) as usize] ^= 1;
    std::fs::write(&paths[1], bytes).unwrap();

    let report = verify_store(&packs);
    assert!(!report.is_ok());
    let name = paths[1].file_name().unwrap().to_string_lossy().into_owned();
    assert_eq!(report.problems.len(), 1, "{:?}", report.problems);
    assert!(report.problems[0].contains(&name));
}

#[test]
fn verify_checks_tag_lists_and_server_lines() {
    let (root, packs, _) = make(0);
    let (tags, servers) = crate::test_support::sample_records();
    let mut w = PackWriter::create(&root.path().join("tmp")).unwrap();
    let tags_id = w.add_record(ObjectKind::TagList, &tags.encode()).unwrap();
    w.add_record(ObjectKind::ServerLines, &servers.encode())
        .unwrap();
    let path = w.finish(&packs).unwrap().unwrap();
    let report = verify_store(&packs);
    assert!(report.is_ok(), "{:?}", report.problems);
    assert_eq!(report.objects_checked, 2);

    let entry = PackReader::open(&path)
        .unwrap()
        .find(&tags_id)
        .unwrap()
        .unwrap();
    let mut bytes = std::fs::read(&path).unwrap();
    bytes[(entry.offset + ENTRY_HEADER_LEN) as usize] ^= 1;
    std::fs::write(&path, bytes).unwrap();
    let report = verify_store(&packs);
    assert_eq!(report.problems.len(), 1, "{:?}", report.problems);
    assert!(report.problems[0].contains(&tags_id.to_hex()));
}

#[test]
fn verify_finds_a_chunk_list_that_builds_the_wrong_file() {
    let (root, packs, _) = make(0);
    let tmp = root.path().join("tmp");
    let pieces = [&b"hello "[..], b"world"];
    let good = hello_world();
    let mut w = PackWriter::create(&tmp).unwrap();
    for p in pieces {
        w.add_content(p).unwrap();
    }
    w.add_chunk_list(&good).unwrap();
    w.finish(&packs).unwrap();
    let report = verify_store(&packs);
    assert!(report.is_ok(), "{:?}", report.problems);
    assert_eq!(report.objects_checked, 3);

    // Each chunk is good, and so is the list's structure, but they build another file.
    let wrong = ChunkList {
        file: hash_content(b"HELLO WORLD"),
        ..good
    };
    let mut w = PackWriter::create(&tmp).unwrap();
    w.add_chunk_list(&wrong).unwrap();
    let path = w.finish(&packs).unwrap().unwrap();
    let report = verify_store(&packs);
    assert_eq!(report.problems.len(), 1, "{:?}", report.problems);
    let name = path.file_name().unwrap().to_string_lossy().into_owned();
    let problem = &report.problems[0];
    assert!(
        problem.starts_with(&format!("{name}: object {}: ", wrong.file)),
        "{problem}"
    );
    assert!(problem.contains("build another file"), "{problem}");
}

#[test]
fn problems_show_pack_names_escaped() {
    let (_root, packs, _paths) = make(0);
    std::fs::write(packs.join("a\u{202e}b.pack"), b"short").unwrap();
    let report = verify_store(&packs);
    assert_eq!(report.problems.len(), 1, "{:?}", report.problems);
    let problem = &report.problems[0];
    assert!(!problem.contains('\u{202e}'), "{problem}");
    assert!(problem.starts_with("a\\u{202e}b.pack: "), "{problem}");
}

#[test]
fn verify_reports_a_broken_pack_instead_of_failing() {
    let (_root, packs, paths) = make(3);
    let len = std::fs::metadata(&paths[0]).unwrap().len();
    std::fs::OpenOptions::new()
        .write(true)
        .open(&paths[0])
        .unwrap()
        .set_len(len - 10)
        .unwrap();
    let report = verify_store(&packs);
    assert_eq!(report.problems.len(), 1, "{:?}", report.problems);
    assert_eq!(report.objects_checked, 4);
}

fn now() -> i64 {
    let since = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
    i64::try_from(since.as_millis()).unwrap()
}

fn days_ago(days: i64) -> Person {
    Person {
        name: "Ada".into(),
        email: "ada@example.com".into(),
        time_ms: now() - days * DAY,
        tz_offset_min: 0,
    }
}

/// A project started `days` ago.
fn project(days: i64) -> (tempfile::TempDir, Repository) {
    let dir = tempfile::tempdir().unwrap();
    let repo = Repository::init(dir.path(), &days_ago(days)).unwrap();
    (dir, repo)
}

fn tip(repo: &Repository) -> Id {
    tip_of(&repo.current().unwrap().1).unwrap()
}

/// `verify` with a 30-day window, the store's own packs closed first so that tests may change
/// them on any system.
fn checked(repo: &Repository) -> Verified {
    repo.store().close_all();
    verify(repo, 30, now(), &mut |_, _| {})
}

fn kinds(v: &Verified) -> Vec<ProblemKind> {
    v.problems.iter().map(|p| p.kind).collect()
}

fn pack_paths(repo: &Repository) -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = fs::read_dir(repo.packs_dir())
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "pack"))
        .collect();
    paths.sort();
    paths
}

/// Every object of every pack, with its pack.
fn objects(repo: &Repository) -> Vec<(PathBuf, IndexEntry)> {
    let mut all = Vec::new();
    for path in pack_paths(repo) {
        for e in PackReader::open(&path).unwrap().entries() {
            all.push((path.clone(), e.unwrap()));
        }
    }
    all
}

fn flip(path: &Path, at: u64) {
    let mut bytes = fs::read(path).unwrap();
    bytes[usize::try_from(at).unwrap()] ^= 1;
    fs::write(path, bytes).unwrap();
}

/// Stores what `add` adds in packs of their own, nothing naming them yet; gives their paths.
fn own_pack(repo: &mut Repository, add: impl FnOnce(&mut PackSet)) -> Vec<PathBuf> {
    let _lock = repo.lock_for_change("test", &mut |_| {}).unwrap();
    let mut packs = repo.new_packs().unwrap();
    add(&mut packs);
    packs.finish().unwrap()
}

/// Commits a save whose top folder holds `name`, a file `id` of `size` bytes.
fn plant_file(repo: &mut Repository, name: &str, id: Id, size: u64) -> Id {
    plant_root(repo, |packs| {
        let file = Entry {
            name: name.into(),
            kind: EntryKind::File,
            id,
            size,
        };
        add_listing(packs, vec![file]).0
    });
    tip(repo)
}

/// "hello world" cut into two chunks.
fn hello_world() -> ChunkList {
    let pieces = [&b"hello "[..], b"world"];
    ChunkList {
        file: hash_content(b"hello world"),
        size: 11,
        chunks: pieces
            .iter()
            .map(|p| Chunk {
                id: hash_content(p),
                size: p.len() as u64,
            })
            .collect(),
    }
}

#[test]
fn an_intact_project_has_no_problems_and_counts_what_it_checked() {
    let (_dir, mut repo) = project(0);
    plant_save(
        &mut repo,
        &[
            ("a.txt", EntryKind::File, b"one"),
            ("src/b.txt", EntryKind::File, b"two"),
        ],
    );
    plant_save(&mut repo, &[("a.txt", EntryKind::File, b"three")]);
    let v = checked(&repo);
    assert_eq!(v.problems_total, 0, "{:?}", v.problems);
    let packs = pack_paths(&repo);
    assert_eq!(v.packs, packs.len() as u64);
    assert_eq!(v.objects, objects(&repo).len() as u64);
    let bytes: u64 = packs.iter().map(|p| fs::metadata(p).unwrap().len()).sum();
    assert_eq!(v.bytes, bytes);
    assert_eq!((v.saves, v.entries, v.lines, v.shelves), (3, 3, 1, 0));
    assert!(v.warnings.is_empty(), "{:?}", v.warnings);
}

#[test]
fn a_flipped_byte_in_any_object_is_found_and_the_rest_is_still_checked() {
    let (_dir, mut repo) = project(0);
    plant_save(
        &mut repo,
        &[
            ("a.txt", EntryKind::File, b"one"),
            ("src/b.txt", EntryKind::File, b"two"),
        ],
    );
    let all = objects(&repo);
    let total = all.len() as u64;
    for (path, entry) in all {
        repo.store().close_all();
        flip(&path, entry.offset + ENTRY_HEADER_LEN);
        let v = checked(&repo);
        flip(&path, entry.offset + ENTRY_HEADER_LEN);
        assert_eq!(v.objects, total, "{entry:?}");
        let found = v
            .problems
            .iter()
            .any(|p| p.kind == ProblemKind::ObjectDamaged && p.object == Some(entry.id));
        assert!(found, "{entry:?}: {:?}", v.problems);
    }
    assert_eq!(checked(&repo).problems_total, 0);
}

#[test]
fn a_record_with_a_good_fingerprint_that_does_not_decode_is_damaged() {
    let (_dir, repo) = project(0);
    let mut w = PackWriter::create(repo.tmp_dir()).unwrap();
    let id = w.add_record(ObjectKind::Save, b"not a save").unwrap();
    w.finish(repo.packs_dir()).unwrap();
    let v = checked(&repo);
    assert_eq!(kinds(&v), [ProblemKind::ObjectDamaged], "{:?}", v.problems);
    assert_eq!(v.problems[0].object, Some(id));
    assert!(v.problems[0].message.contains("doesn't read as a save"));
}

#[test]
fn an_index_that_fails_its_checksum_is_a_damaged_pack() {
    let (_dir, repo) = project(0);
    let path = pack_paths(&repo).remove(0);
    let len = fs::metadata(&path).unwrap().len();
    // The first byte of the checksum, before the end magic.
    flip(&path, len - 8 - 32);
    let v = checked(&repo);
    assert_eq!(kinds(&v), [ProblemKind::PackDamaged], "{:?}", v.problems);
    let name = path.file_name().unwrap().to_string_lossy().into_owned();
    assert_eq!(v.problems[0].pack.as_deref(), Some(name.as_str()));
    // Its objects are still read back.
    assert_eq!(v.objects, objects(&repo).len() as u64);
}

#[test]
fn a_pack_bolt_cannot_open_for_another_reason_is_unreadable_not_damaged() {
    let (_dir, repo) = project(0);
    let mut w = PackWriter::create(repo.tmp_dir()).unwrap();
    w.add_content(b"spare").unwrap();
    let path = w.finish(repo.packs_dir()).unwrap().unwrap();
    // Root reads everything, so there it can't be tried.
    let Some(_held) = lock_pack(&path) else {
        return;
    };
    let v = checked(&repo);
    assert_eq!(kinds(&v), [ProblemKind::PackUnreadable], "{:?}", v.problems);
    let name = path.file_name().unwrap().to_string_lossy().into_owned();
    assert_eq!(v.problems[0].pack.as_deref(), Some(name.as_str()));
}

#[test]
fn a_deleted_pack_shows_as_objects_the_project_needs() {
    let (_dir, mut repo) = project(0);
    let mut content = None;
    let gone = own_pack(&mut repo, |p| {
        content = Some(p.add_content(b"only here").unwrap())
    });
    let content = content.unwrap();
    plant_root(&mut repo, |packs| {
        let file = Entry {
            name: "a.txt".into(),
            kind: EntryKind::File,
            id: content,
            size: 9,
        };
        let (docs, _) = add_listing(packs, vec![file]);
        let folder = Entry {
            name: "docs".into(),
            kind: EntryKind::Folder,
            id: docs,
            size: 0,
        };
        add_listing(packs, vec![folder]).0
    });
    let save = tip(&repo);
    repo.store().close_all();
    fs::remove_file(&gone[0]).unwrap();
    let v = checked(&repo);
    assert_eq!(kinds(&v), [ProblemKind::ObjectMissing], "{:?}", v.problems);
    assert_eq!(v.problems[0].object, Some(content));
    assert_eq!(
        v.problems[0].needed_by,
        Some((save, "docs/a.txt".to_owned()))
    );
}

#[test]
fn a_missing_chunk_of_a_needed_file_is_missing() {
    let (_dir, mut repo) = project(0);
    let list = hello_world();
    let gone = own_pack(&mut repo, |p| {
        p.add_content(b"world").unwrap();
    });
    own_pack(&mut repo, |p| {
        p.add_content(b"hello ").unwrap();
        p.add_chunk_list(&list).unwrap();
    });
    let save = plant_file(&mut repo, "big.bin", list.file, 11);
    repo.store().close_all();
    fs::remove_file(&gone[0]).unwrap();
    let v = checked(&repo);
    assert_eq!(kinds(&v), [ProblemKind::ObjectMissing], "{:?}", v.problems);
    assert_eq!(v.problems[0].object, Some(list.chunks[1].id));
    assert_eq!(v.problems[0].needed_by, Some((save, "big.bin".to_owned())));
}

#[test]
fn a_chunk_list_that_builds_another_file_is_damaged() {
    let (_dir, mut repo) = project(0);
    let wrong = ChunkList {
        file: hash_content(b"HELLO WORLD"),
        ..hello_world()
    };
    own_pack(&mut repo, |p| {
        p.add_content(b"hello ").unwrap();
        p.add_content(b"world").unwrap();
        p.add_chunk_list(&wrong).unwrap();
    });
    let save = plant_file(&mut repo, "big.bin", wrong.file, 11);
    let v = checked(&repo);
    assert_eq!(kinds(&v), [ProblemKind::ObjectDamaged], "{:?}", v.problems);
    assert_eq!(v.problems[0].object, Some(wrong.file));
    assert!(v.problems[0].message.contains("build another file"));
    assert_eq!(v.problems[0].needed_by, Some((save, "big.bin".to_owned())));
}

#[test]
fn a_lost_head_is_one_problem_and_the_packs_are_still_checked() {
    let (dir, mut repo) = project(0);
    plant_save(&mut repo, &[("a.txt", EntryKind::File, b"one")]);
    fs::write(dir.path().join(".bolt/state/head"), "not an entry\n").unwrap();
    let v = checked(&repo);
    assert_eq!(kinds(&v), [ProblemKind::HeadLost], "{:?}", v.problems);
    assert!(v.problems[0].message.contains(".bolt/state/head"));
    assert_eq!(v.objects, objects(&repo).len() as u64);
    assert_eq!(v.packs, pack_paths(&repo).len() as u64);
}

fn save_text(dir: &Path, repo: &mut Repository, text: &str, days: i64) {
    fs::write(dir.join("a.txt"), text).unwrap();
    let who = days_ago(days);
    let req = SaveRequest {
        message: Some("Change"),
        who: &who,
        command: "save",
        accept_crlf: false,
        git_excludes: None,
        new_line: None,
        only: None,
        amend: false,
    };
    let saved = save_folder(repo, &req, &mut |_| {}, &mut |_| true).unwrap();
    assert!(matches!(saved, SaveOutcome::Saved { .. }));
}

#[test]
fn what_an_earlier_tidy_freed_is_no_problem() {
    let (dir, mut repo) = project(50);
    save_text(dir.path(), &mut repo, "one", 45);
    fs::write(dir.path().join("a.txt"), "thrown away").unwrap();
    discard(&mut repo, None, &days_ago(10), "discard", None, &mut |_| {}).unwrap();
    // Two days on, with a five-day window: what the discard kept goes.
    let later = now() + 2 * DAY;
    crate::tidy::tidy(&mut repo, 5, later, &mut |_| {}, &mut |_| {}).unwrap();
    // The window made longer again: what that tidy freed isn't asked for.
    let v = checked(&repo);
    assert_eq!(v.problems_total, 0, "{:?}", v.problems);
    // Without the record of the window it used, it would be.
    fs::remove_file(dir.path().join(".bolt/state/tidied")).unwrap();
    let v = checked(&repo);
    assert!(kinds(&v).contains(&ProblemKind::ObjectMissing), "{v:?}");
}

#[test]
fn a_save_missing_inside_the_window_is_a_problem() {
    let (_dir, mut repo) = project(0);
    let missing = Id([9; 32]);
    let lock = repo.lock_for_change("test", &mut |_| {}).unwrap();
    let (head, mut entry) = repo.current().unwrap();
    entry.parent = Some(head);
    entry.action = Action::Command;
    entry.command = "test".into();
    entry.folder_change = Some(FolderChange {
        from: missing,
        to: tip_of(&entry).unwrap(),
    });
    let packs = repo.new_packs().unwrap();
    repo.commit(&lock, packs, &entry).unwrap();
    drop(lock);
    let v = checked(&repo);
    assert_eq!(kinds(&v), [ProblemKind::ObjectMissing], "{:?}", v.problems);
    assert_eq!(v.problems[0].object, Some(missing));
    assert_eq!(v.problems[0].needed_by, None);
}

#[test]
fn a_diary_end_that_cannot_be_read_back_is_a_warning_told_once() {
    let (_dir, mut repo) = project(0);
    let gone = Id([7; 32]);
    let lock = repo.lock_for_change("test", &mut |_| {}).unwrap();
    let (_, mut entry) = repo.current().unwrap();
    entry.parent = Some(gone);
    entry.command = "test".into();
    entry.folder_change = None;
    let packs = repo.new_packs().unwrap();
    let end = repo.store_entry(&lock, packs, &entry).unwrap();
    drop(lock);
    let v = checked(&repo);
    assert_eq!(v.problems_total, 0, "{:?}", v.problems);
    let warned: Vec<&Warning> = v
        .warnings
        .iter()
        .filter(|w| w.kind == crate::warning::DIARY_END_UNREADABLE)
        .collect();
    assert_eq!(warned.len(), 1, "{:?}", v.warnings);
    assert!(warned[0].message.contains(&end.short_hex()), "{warned:?}");
    assert!(warned[0].message.contains(&gone.short_hex()), "{warned:?}");
}

#[test]
fn a_pack_that_goes_away_mid_check_is_no_problem() {
    let (_dir, mut repo) = project(0);
    plant_save(&mut repo, &[("a.txt", EntryKind::File, b"one")]);
    let packs = pack_paths(&repo);
    assert!(packs.len() > 1);
    // Merged into a new pack once the check has begun, before its turn.
    let last = packs.last().unwrap().clone();
    let mut merge = Some((last.clone(), last.with_file_name("zz-merged.pack")));
    repo.store().close_all();
    let v = verify(&repo, 30, now(), &mut |_, _| {
        if let Some((from, to)) = merge.take() {
            fs::rename(from, to).unwrap();
        }
    });
    assert!(merge.is_none());
    assert_eq!(v.problems_total, 0, "{:?}", v.problems);
    assert_eq!(v.packs, packs.len() as u64);
}

#[test]
fn a_pack_that_arrives_mid_check_is_checked() {
    let (_dir, mut repo) = project(0);
    plant_save(&mut repo, &[("a.txt", EntryKind::File, b"one")]);
    let count = pack_paths(&repo).len() as u64;
    let packs = repo.packs_dir().to_owned();
    let mut arrive = Some(());
    repo.store().close_all();
    let v = verify(&repo, 30, now(), &mut |_, _| {
        if arrive.take().is_some() {
            plant_damaged_copies(&packs, &[(ObjectKind::Content, b"late")]);
        }
    });
    assert_eq!(v.packs, count + 1);
    assert_eq!(kinds(&v), [ProblemKind::ObjectDamaged], "{:?}", v.problems);
    assert_eq!(v.problems[0].pack.as_deref(), Some("00.pack"));
}

#[test]
fn more_than_1000_problems_are_counted_and_1000_kept() {
    let (_dir, repo) = project(0);
    let contents: Vec<Vec<u8>> = (0..1005)
        .map(|i| format!("object {i}").into_bytes())
        .collect();
    let objects: Vec<(ObjectKind, &[u8])> = contents
        .iter()
        .map(|c| (ObjectKind::Content, c.as_slice()))
        .collect();
    plant_damaged_copies(repo.packs_dir(), &objects);
    let v = checked(&repo);
    assert_eq!(v.problems_total, 1005);
    assert_eq!(v.problems.len(), PROBLEMS_KEPT);
    assert_eq!(v.count(ProblemKind::ObjectDamaged), 1005);
}

#[cfg(unix)]
#[test]
fn a_fifo_named_like_a_pack_is_refused_within_the_limit() {
    let (dir, repo) = project(0);
    drop(repo);
    let fifo = dir.path().join(".bolt/store/packs/zz.pack");
    let made = std::process::Command::new("mkfifo")
        .arg(&fifo)
        .status()
        .unwrap();
    assert!(made.success());
    let root = dir.path().to_owned();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let repo = Repository::open(&root).unwrap();
        let _ = tx.send(verify(&repo, 30, now(), &mut |_, _| {}));
    });
    let v = rx
        .recv_timeout(std::time::Duration::from_secs(10))
        .expect("verify hung on a FIFO named like a pack");
    assert_eq!(kinds(&v), [ProblemKind::PackDamaged], "{:?}", v.problems);
    assert_eq!(v.problems[0].pack.as_deref(), Some("zz.pack"));
}

/// Every file and folder under `dir`: a file's bytes, and when each was last changed.
fn every_file(dir: &Path) -> BTreeMap<PathBuf, (Option<Vec<u8>>, SystemTime)> {
    let mut found = BTreeMap::new();
    let mut folders = vec![dir.to_owned()];
    while let Some(folder) = folders.pop() {
        let changed = fs::symlink_metadata(&folder).unwrap().modified().unwrap();
        found.insert(folder.clone(), (None, changed));
        for e in fs::read_dir(&folder).unwrap() {
            let path = e.unwrap().path();
            let meta = fs::symlink_metadata(&path).unwrap();
            if meta.is_dir() {
                folders.push(path);
            } else {
                let bytes = fs::read(&path).unwrap();
                found.insert(path, (Some(bytes), meta.modified().unwrap()));
            }
        }
    }
    found
}

#[test]
fn verify_writes_nothing() {
    let (dir, mut repo) = project(0);
    let list = hello_world();
    let gone = own_pack(&mut repo, |p| {
        p.add_content(b"only here").unwrap();
    });
    own_pack(&mut repo, |p| {
        p.add_content(b"hello ").unwrap();
        p.add_content(b"world").unwrap();
        p.add_chunk_list(&list).unwrap();
    });
    plant_root(&mut repo, |packs| {
        let files = [
            ("big.bin", list.file, 11),
            ("a.txt", hash_content(b"only here"), 9),
        ];
        let entries = files.map(|(name, id, size)| Entry {
            name: name.into(),
            kind: EntryKind::File,
            id,
            size,
        });
        add_listing(packs, entries.to_vec()).0
    });
    let bolt = dir.path().join(".bolt");
    assert!(bolt.join("format").is_file());
    let before = every_file(&bolt);
    let v = checked(&repo);
    assert_eq!(v.problems_total, 0, "{:?}", v.problems);
    assert_eq!(every_file(&bolt), before);

    // Again with a needed chunk damaged and a needed file missing, so naming what needs them
    // runs too.
    repo.store().close_all();
    fs::remove_file(&gone[0]).unwrap();
    let (path, entry) = objects(&repo)
        .into_iter()
        .find(|(_, e)| e.id == list.chunks[0].id)
        .unwrap();
    flip(&path, entry.offset + ENTRY_HEADER_LEN);
    let before = every_file(&bolt);
    let v = checked(&repo);
    let mut found = kinds(&v);
    found.sort_by_key(|k| k.name());
    assert_eq!(
        found,
        [ProblemKind::ObjectDamaged, ProblemKind::ObjectMissing],
        "{:?}",
        v.problems
    );
    assert!(v.problems.iter().all(|p| p.needed_by.is_some()), "{v:?}");
    assert_eq!(every_file(&bolt), before);
}

fn found(ids: &[u8]) -> Verdict {
    let kept: BTreeSet<Finding> = ids.iter().map(|b| Finding::Missing(Id([*b; 32]))).collect();
    Verdict::Found(Findings {
        counts: [kept.len() as u64, 0, 0, 0, 0],
        diary_gap: false,
        kept,
    })
}

fn lost(why: &str) -> Verdict {
    Verdict::Lost(why.into())
}

fn failed(why: &str) -> Verdict {
    Verdict::Failed(ProblemKind::PackUnreadable, why.into())
}

#[test]
fn two_looks_that_found_the_head_report_only_what_both_found_missing() {
    assert_eq!(agreed(&[found(&[1, 2]), found(&[2, 3])]), Some(found(&[2])));
}

#[test]
fn two_looks_that_lost_the_head_report_it_lost() {
    assert_eq!(agreed(&[lost("one"), lost("two")]), Some(lost("two")));
}

#[test]
fn a_lost_head_between_two_that_found_it_is_outvoted() {
    assert_eq!(agreed(&[found(&[1, 2]), lost("raced")]), None);
    let three = [found(&[1, 2]), lost("raced"), found(&[2, 3])];
    assert_eq!(agreed(&three), Some(found(&[2])));
}

#[test]
fn a_found_head_between_two_that_lost_it_is_outvoted() {
    assert_eq!(
        agreed(&[lost("one"), found(&[1]), lost("three")]),
        Some(lost("three"))
    );
}

#[test]
fn two_looks_that_failed_report_the_failure() {
    assert_eq!(agreed(&[failed("one"), failed("two")]), Some(failed("two")));
}

#[test]
fn three_looks_that_all_differ_report_the_last() {
    assert_eq!(
        agreed(&[found(&[1]), lost("two"), failed("three")]),
        Some(failed("three"))
    );
    assert_eq!(
        agreed(&[failed("one"), found(&[1]), lost("three")]),
        Some(lost("three"))
    );
}

#[test]
fn a_save_made_while_verify_reads_the_packs_never_reads_as_a_lost_head() {
    let (dir, mut repo) = project(0);
    let mut content = None;
    let gone = own_pack(&mut repo, |p| {
        content = Some(p.add_content(b"only here").unwrap())
    });
    let content = content.unwrap();
    plant_file(&mut repo, "a.txt", content, 9);
    repo.store().close_all();
    fs::remove_file(&gone[0]).unwrap();
    let root = dir.path().to_owned();
    let mut saved = false;
    let v = verify(&repo, 30, now(), &mut |_, _| {
        if !saved {
            saved = true;
            let mut other = Repository::open(&root).unwrap();
            plant_save(&mut other, &[("b.txt", EntryKind::File, b"while it reads")]);
        }
    });
    assert!(saved);
    assert_eq!(kinds(&v), [ProblemKind::ObjectMissing], "{:?}", v.problems);
    assert_eq!(v.problems[0].object, Some(content));
}

#[test]
fn what_no_readable_pack_holds_is_told_as_such() {
    let (_dir, mut repo) = project(0);
    let mut content = None;
    let gone = own_pack(&mut repo, |p| {
        content = Some(p.add_content(b"only here").unwrap())
    });
    plant_file(&mut repo, "a.txt", content.unwrap(), 9);
    repo.store().close_all();
    fs::remove_file(&gone[0]).unwrap();
    let v = checked(&repo);
    assert_eq!(v.problems[0].message, "It isn't in any pack.");
    fs::write(repo.packs_dir().join("zz.pack"), b"not a pack").unwrap();
    let v = checked(&repo);
    let missing: Vec<&Problem> = v
        .problems
        .iter()
        .filter(|p| p.kind == ProblemKind::ObjectMissing)
        .collect();
    assert_eq!(missing.len(), 1, "{:?}", v.problems);
    assert_eq!(missing[0].message, "It isn't in any pack Bolt could read.");
}

#[test]
fn an_object_stored_as_another_kind_is_missing_for_what_needs_it() {
    let (_dir, mut repo) = project(0);
    let mut content = None;
    own_pack(&mut repo, |p| {
        content = Some(p.add_content(b"not a folder").unwrap())
    });
    let content = content.unwrap();
    plant_root(&mut repo, |_| content);
    let save = tip(&repo);
    let v = checked(&repo);
    assert_eq!(kinds(&v), [ProblemKind::ObjectMissing], "{:?}", v.problems);
    assert_eq!(v.problems[0].object, Some(content));
    assert_eq!(
        v.problems[0].message,
        "It is stored, but as another kind of object than what needs it."
    );
    assert_eq!(v.problems[0].needed_by, Some((save, String::new())));
}

#[test]
fn a_lost_head_and_one_that_could_not_be_read_agree() {
    let unreadable = Verdict::Failed(ProblemKind::HeadLost, "can't read it".into());
    assert_eq!(
        agreed(&[lost("one"), unreadable.clone()]),
        Some(unreadable.clone())
    );
    assert_eq!(agreed(&[unreadable, lost("two")]), Some(lost("two")));
}

#[test]
fn an_unreadable_head_and_an_unreadable_pack_do_not_agree() {
    let head = Verdict::Failed(ProblemKind::HeadLost, "can't read it".into());
    assert_eq!(agreed(&[head.clone(), failed("pack")]), None);
    assert_eq!(agreed(&[failed("pack"), head]), None);
}

#[test]
fn a_chunk_held_only_as_a_chunk_list_is_missing() {
    let (_dir, mut repo) = project(0);
    let list = hello_world();
    // "world" is in no pack as content: a chunk list of its two halves stands under its ID.
    let world = ChunkList {
        file: hash_content(b"world"),
        size: 5,
        chunks: [&b"wor"[..], b"ld"]
            .iter()
            .map(|p| Chunk {
                id: hash_content(p),
                size: p.len() as u64,
            })
            .collect(),
    };
    own_pack(&mut repo, |p| {
        p.add_content(b"hello ").unwrap();
        p.add_content(b"wor").unwrap();
        p.add_content(b"ld").unwrap();
        p.add_chunk_list(&world).unwrap();
        p.add_chunk_list(&list).unwrap();
    });
    let save = plant_file(&mut repo, "big.bin", list.file, 11);
    let v = checked(&repo);
    assert_eq!(kinds(&v), [ProblemKind::ObjectMissing], "{:?}", v.problems);
    assert_eq!(v.problems[0].object, Some(world.file));
    assert_eq!(
        v.problems[0].message,
        "It is stored, but as another kind of object than what needs it."
    );
    assert_eq!(v.problems[0].needed_by, Some((save, "big.bin".to_owned())));
}

#[test]
fn a_listing_that_gives_a_file_another_size_is_damaged() {
    let (_dir, mut repo) = project(0);
    let list = hello_world();
    let mut small = None;
    own_pack(&mut repo, |p| {
        small = Some(p.add_content(b"hello\n").unwrap());
        p.add_content(b"hello ").unwrap();
        p.add_content(b"world").unwrap();
        p.add_chunk_list(&list).unwrap();
    });
    let small = small.unwrap();
    plant_root(&mut repo, |packs| {
        let files = [("a.txt", small, 999), ("big.bin", list.file, 12)];
        let entries = files.map(|(name, id, size)| Entry {
            name: name.into(),
            kind: EntryKind::File,
            id,
            size,
        });
        let (docs, _) = add_listing(packs, entries.to_vec());
        let folder = Entry {
            name: "docs".into(),
            kind: EntryKind::Folder,
            id: docs,
            size: 0,
        };
        add_listing(packs, vec![folder]).0
    });
    let save = tip(&repo);
    let v = checked(&repo);
    assert_eq!(
        kinds(&v),
        [ProblemKind::ObjectDamaged, ProblemKind::ObjectDamaged],
        "{:?}",
        v.problems
    );
    let docs = repo
        .store()
        .listing(&repo.store().save(&save, false).unwrap().root);
    let docs = docs.unwrap().entries()[0].id;
    for (p, (path, listed, stored)) in v
        .problems
        .iter()
        .zip([("docs/a.txt", 999, 6), ("docs/big.bin", 12, 11)])
    {
        assert_eq!(p.object, Some(docs), "{p:?}");
        assert!(p.message.contains(&format!("{listed} bytes")), "{p:?}");
        assert!(p.message.contains(&format!("{stored} bytes")), "{p:?}");
        assert_eq!(p.needed_by, Some((save, path.to_owned())), "{p:?}");
    }
}

/// A listing of `n` entries named 00000 and up, each of `kind` naming `id` with `size`.
fn many(packs: &mut PackSet, n: usize, kind: EntryKind, id: Id, size: u64) -> Id {
    let entries = (0..n)
        .map(|i| Entry {
            name: format!("{i:05}"),
            kind,
            id,
            size,
        })
        .collect();
    add_listing(packs, entries).0
}

#[test]
fn a_save_past_the_entry_limit_is_too_big() {
    let (_dir, mut repo) = project(0);
    plant_root(&mut repo, |packs| {
        let x = packs.add_content(b"x").unwrap();
        let folder = many(packs, 1_000, EntryKind::File, x, 1);
        many(packs, 10_001, EntryKind::Folder, folder, 0)
    });
    let save = tip(&repo);
    let v = checked(&repo);
    assert_eq!(kinds(&v), [ProblemKind::SaveTooBig], "{:?}", v.problems);
    assert_eq!(v.problems[0].object, Some(save));
    assert!(
        v.problems[0].message.contains("files and folders in total"),
        "{v:?}"
    );
    assert_eq!(v.count(ProblemKind::SaveTooBig), 1);
}

#[test]
fn a_save_with_a_path_past_the_limit_is_too_big() {
    let (_dir, mut repo) = project(0);
    plant_root(&mut repo, |packs| {
        let mut at = packs.add_content(b"x").unwrap();
        let mut kind = EntryKind::File;
        // Twenty levels of 250-byte names: a path of more than 5,000 bytes.
        for level in 0..20 {
            let entry = Entry {
                name: format!("{level:02}{}", "n".repeat(248)),
                kind,
                id: at,
                size: u64::from(kind == EntryKind::File),
            };
            at = add_listing(packs, vec![entry]).0;
            kind = EntryKind::Folder;
        }
        at
    });
    let save = tip(&repo);
    let v = checked(&repo);
    assert_eq!(kinds(&v), [ProblemKind::SaveTooBig], "{:?}", v.problems);
    assert_eq!(v.problems[0].object, Some(save));
    assert!(
        v.problems[0].message.contains("longer than 4096 bytes"),
        "{v:?}"
    );
}

#[test]
fn a_listing_is_not_blamed_for_a_damaged_copy_of_its_file() {
    let (_dir, mut repo) = project(0);
    let text = b"hello\n";
    let id = hash_content(text);
    // First by name, a copy whose index line claims 999 bytes.
    let mut w = PackWriter::create(repo.tmp_dir()).unwrap();
    let false_size = crate::pack::Prepared {
        id,
        size: 999,
        zstd: false,
        stored: text.to_vec(),
    };
    w.add_prepared(ObjectKind::Content, &false_size).unwrap();
    let made = w.finish(repo.packs_dir()).unwrap().unwrap();
    fs::rename(&made, repo.packs_dir().join("00.pack")).unwrap();
    own_pack(&mut repo, |p| {
        p.add_content(text).unwrap();
    });
    plant_file(&mut repo, "a.txt", id, 6);
    let v = checked(&repo);
    assert_eq!(kinds(&v), [ProblemKind::ObjectDamaged], "{:?}", v.problems);
    assert_eq!(v.problems[0].pack.as_deref(), Some("00.pack"));
    assert_eq!(v.problems[0].object, Some(id));
}

#[test]
fn a_chunk_named_many_times_is_noted_and_told_once() {
    let (_dir, mut repo) = project(0);
    let absent = hash_content(b"in no pack");
    let list = ChunkList {
        file: hash_content(b"a file of 3,000 bytes"),
        size: 3_000,
        chunks: vec![
            Chunk {
                id: absent,
                size: 1
            };
            3_000
        ],
    };
    own_pack(&mut repo, |p| p.add_chunk_list(&list).unwrap());
    plant_file(&mut repo, "big.bin", list.file, 3_000);
    repo.store().close_all();
    let Ok(Ok((_, look))) = look_at(&repo, 30, now(), &Told::default()) else {
        panic!("the look wasn't made");
    };
    let notes = look.noted.unread.iter().filter(|(id, _)| *id == absent);
    assert_eq!(notes.count(), 1);
    drop(look);
    let v = checked(&repo);
    assert_eq!(kinds(&v), [ProblemKind::ObjectMissing], "{:?}", v.problems);
    assert_eq!(v.problems[0].object, Some(absent));
}

/// What `verify`'s marking notes for one look of `repo`, with a 30-day window.
fn needs_of(repo: &Repository) -> Noted {
    let store = Store::open_in(Arc::clone(repo.packs_folder()), repo.packs_dir()).unwrap();
    let (head, _) = head_entry(&store, repo.head().unwrap()).unwrap();
    let now_ns = i128::from(now()) * 1_000_000;
    let packs = tidy::listed_leniently(repo.packs_folder(), now_ns).unwrap();
    let src = Source {
        store: &store,
        signed: repo.signatures_allowed(),
        folder: repo.packs_folder(),
        packs_dir: repo.packs_dir(),
    };
    let since = now() - 30 * DAY;
    tidy::needs(&src, &packs, head, since, Noted::default())
        .unwrap()
        .1
}

#[test]
fn many_wrong_sizes_are_counted_and_only_what_is_told_is_held() {
    let (_dir, mut repo) = project(0);
    plant_root(&mut repo, |packs| {
        let x = packs.add_content(b"x").unwrap();
        many(packs, 3_000, EntryKind::File, x, 2)
    });
    repo.store().close_all();
    let noted = needs_of(&repo);
    assert!(noted.sizes.len() <= PROBLEMS_KEPT, "{}", noted.sizes.len());
    let v = checked(&repo);
    assert_eq!(v.problems_total, 3_000);
    assert_eq!(v.count(ProblemKind::ObjectDamaged), 3_000);
    assert_eq!(v.problems.len(), PROBLEMS_KEPT);
    // The first by name are the ones told.
    assert!(
        v.problems[0].message.contains("00000 "),
        "{:?}",
        v.problems[0]
    );
    assert!(
        v.problems[999].message.contains("00999 "),
        "{:?}",
        v.problems[999]
    );
}

#[test]
fn a_listing_naming_one_folder_many_times_holds_one_link() {
    let (_dir, mut repo) = project(0);
    plant_root(&mut repo, |packs| {
        let x = packs.add_content(b"x").unwrap();
        let folder = many(packs, 2, EntryKind::File, x, 1);
        many(packs, 10_000, EntryKind::Folder, folder, 0)
    });
    let root = repo.store().save(&tip(&repo), false).unwrap().root;
    repo.store().close_all();
    let noted = needs_of(&repo);
    assert_eq!(noted.shapes[&root].folders.len(), 1);
    assert_eq!(checked(&repo).problems_total, 0);
}

/// How long `verify` takes, at best of three, on a save of one listing of `n` files with names of
/// 208 bytes.
fn time_wide(n: usize) -> std::time::Duration {
    let (_dir, mut repo) = project(0);
    plant_root(&mut repo, |packs| {
        let x = packs.add_content(b"x").unwrap();
        let entries = (0..n)
            .map(|i| Entry {
                name: format!("{i:08}{}", "a".repeat(200)),
                kind: EntryKind::File,
                id: x,
                size: 1,
            })
            .collect();
        add_listing(packs, entries).0
    });
    (0..3)
        .map(|_| {
            let began = std::time::Instant::now();
            assert_eq!(checked(&repo).problems_total, 0);
            began.elapsed()
        })
        .min()
        .unwrap()
}

/// Twice the entries take no more than about twice the time: nothing in `verify` searches or
/// copies per entry. The best of three runs, and a quarter second of slack, keep a busy machine
/// from failing it.
#[test]
fn verify_takes_about_twice_as_long_for_twice_the_entries() {
    let (small, large) = (time_wide(20_000), time_wide(40_000));
    eprintln!("verify: {small:?} for 20,000 entries, {large:?} for 40,000");
    let bound = small.mul_f64(2.5) + std::time::Duration::from_millis(250);
    assert!(large <= bound, "{small:?} for 20,000, {large:?} for 40,000");
}

#[test]
fn two_looks_past_what_is_kept_tell_the_smaller_count() {
    let keep = |n: u64| Findings {
        kept: (0..=255u8).map(|b| Finding::Missing(Id([b; 32]))).collect(),
        counts: [n, 0, 0, 0, 0],
        diary_gap: false,
    };
    let both = agreed(&[Verdict::Found(keep(1_500)), Verdict::Found(keep(1_200))]);
    assert_eq!(both, Some(Verdict::Found(keep(1_200))));
}

#[test]
fn two_looks_tell_only_the_unreadable_objects_and_the_diary_gap_both_found() {
    let looked = |ids: &[u8], gap: bool| {
        let damaged = ids.iter().map(|b| Finding::Damaged(Id([*b; 32])));
        let unreadable = ids.iter().map(|b| Finding::Unreadable(Id([*b; 32])));
        let kept: BTreeSet<Finding> = damaged.chain(unreadable).collect();
        let n = ids.len() as u64;
        Verdict::Found(Findings {
            kept,
            counts: [0, 0, 0, n, n],
            diary_gap: gap,
        })
    };
    let both = agreed(&[looked(&[1, 2], true), looked(&[2, 3], false)]);
    assert_eq!(both, Some(looked(&[2], false)));
    let both = agreed(&[looked(&[1], true), looked(&[1], true)]);
    assert_eq!(both, Some(looked(&[1], true)));
}

#[test]
fn a_diary_with_a_gap_is_not_intact() {
    let (_dir, mut repo) = project(0);
    assert!(!checked(&repo).diary_gap);
    let gone = Id([7; 32]);
    let lock = repo.lock_for_change("test", &mut |_| {}).unwrap();
    let (_, mut entry) = repo.current().unwrap();
    entry.parent = Some(gone);
    entry.command = "test".into();
    entry.folder_change = None;
    let packs = repo.new_packs().unwrap();
    repo.commit(&lock, packs, &entry).unwrap();
    drop(lock);
    let v = checked(&repo);
    assert!(v.diary_gap);
    assert_eq!(kinds(&v), [ProblemKind::ObjectMissing], "{:?}", v.problems);
    assert_eq!(v.problems[0].object, Some(gone));
}

#[test]
fn what_the_check_of_the_packs_told_as_unreadable_is_not_told_again() {
    let (_dir, mut repo) = project(0);
    let mut content = None;
    own_pack(&mut repo, |p| {
        content = Some(p.add_content(b"one").unwrap())
    });
    plant_file(&mut repo, "a.txt", content.unwrap(), 3);
    let root = repo.store().save(&tip(&repo), false).unwrap().root;
    let (path, entry) = objects(&repo)
        .into_iter()
        .find(|(_, e)| e.id == root)
        .unwrap();
    repo.store().close_all();
    flip(&path, entry.offset + ENTRY_HEADER_LEN);
    let damaged = |told: &Told| {
        let Ok(Ok((found, _))) = look_at(&repo, 30, now(), told) else {
            panic!("the look wasn't made");
        };
        found.kept.contains(&Finding::Damaged(root))
    };
    assert!(damaged(&Told::default()));
    let told = Told {
        unreadable: HashSet::from([root]),
        ..Told::default()
    };
    assert!(!damaged(&told));
}
