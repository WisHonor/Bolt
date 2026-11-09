//! Big files, end to end: cut into chunks as they are read, and back byte for byte.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::io::{self, Seek, SeekFrom, Write};
use std::path::Path;

use bolt_core::chunks::set_test_cut_sizes;
use bolt_core::git::set_test_delta_limits;
use bolt_core::history::current_save;
use bolt_core::id::{Flavor, Id, hash_record};
use bolt_core::import::{ImportRequest, import_git};
use bolt_core::listing::EntryKind;
use bolt_core::pack::ObjectKind;
use bolt_core::repo::Repository;

mod support;
use support::{
    Env, ada, copies_of, copy_dir, id_of, main_tip, save_here, text, top_folder, write, write_noise,
};

const MIB: u64 = 1 << 20;

/// `bolt <args>`, which must succeed.
fn ok(env: &Env, args: &[&str]) {
    let out = env.bolt().args(args).output().unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "{args:?}: {}",
        text(&out.stderr)
    );
}

/// A project that has saved `big.bin`, `size` bytes made from `seed`, and the file's ID.
fn with_a_big_file(seed: u64, size: u64) -> (Env, Id) {
    let env = Env::empty();
    ok(&env, &["start"]);
    let id = write_noise(&env.root().join("big.bin"), seed, size);
    write(&env, "small.txt", "small\n");
    ok(&env, &["save", "Big"]);
    (env, id)
}

/// Writes a few bytes over the middle of the file at `path`; its new ID.
fn edit(path: &Path) -> Id {
    let mut file = std::fs::OpenOptions::new().write(true).open(path).unwrap();
    let middle = file.metadata().unwrap().len() / 2;
    file.seek(SeekFrom::Start(middle)).unwrap();
    file.write_all(b"edited").unwrap();
    drop(file);
    id_of(path)
}

#[test]
fn chunking_changes_no_ids() {
    let off = tempfile::tempdir().unwrap();
    Repository::init(off.path(), &ada()).unwrap();
    std::fs::create_dir(off.path().join("sub")).unwrap();
    let files = [
        ("one.bin", MIB),
        ("three.bin", 3 * MIB),
        ("sub/forty.bin", 40 * MIB),
    ];
    let ids: Vec<Id> = (1..)
        .zip(files)
        .map(|(seed, (path, size))| write_noise(&off.path().join(path), seed, size))
        .collect();
    let on = tempfile::tempdir().unwrap();
    copy_dir(off.path(), on.path());
    // The same project in two folders, saved with cutting turned off, then on. Off, every file is
    // too long to cut, so it is stored whole as it is streamed.
    set_test_cut_sizes(Some((u64::MAX, 0)));
    save_here(off.path(), "Big files").unwrap();
    set_test_cut_sizes(None);
    save_here(on.path(), "Big files").unwrap();

    let off_repo = Repository::open(off.path()).unwrap();
    let on_repo = Repository::open(on.path()).unwrap();
    let off_id = current_save(&off_repo).unwrap();
    let off_save = off_repo.store().save(&off_id, false).unwrap();
    let on_id = current_save(&on_repo).unwrap();
    let mut on_save = on_repo.store().save(&on_id, false).unwrap();
    // A save draws its change ID at random: given the same one, it is the same save.
    on_save.change_id = off_save.change_id;
    assert_eq!(hash_record(Flavor::Save, &on_save.encode()), off_id);
    // The same listings, held by both stores...
    let mut folders = vec![off_save.root];
    while let Some(id) = folders.pop() {
        let listing = off_repo.store().listing(&id).unwrap();
        assert_eq!(on_repo.store().listing(&id).unwrap(), listing);
        let below = listing
            .entries()
            .iter()
            .filter(|e| e.kind == EntryKind::Folder);
        folders.extend(below.map(|e| e.id));
    }
    // ...name each file by the same ID, stored whole in one and cut in the other.
    for id in &ids {
        assert_eq!(copies_of(off.path(), id), [ObjectKind::Content]);
        assert_eq!(copies_of(on.path(), id), [ObjectKind::ChunkList]);
        for repo in [&off_repo, &on_repo] {
            repo.store().copy_content(id, &mut io::sink()).unwrap();
        }
    }
}

#[test]
fn big_files_go_to_and_come_back_byte_for_byte() {
    let (env, first) = with_a_big_file(1, 3 * MIB);
    let one = main_tip(&env);
    let big = env.root().join("big.bin");
    let second = edit(&big);
    ok(&env, &["save", "Edited"]);
    let two = main_tip(&env);
    for id in [first, second] {
        assert_eq!(copies_of(env.root(), &id), [ObjectKind::ChunkList]);
    }
    ok(&env, &["goto", &one.to_hex()]);
    assert_eq!(id_of(&big), first);
    ok(&env, &["goto", &two.to_hex()]);
    assert_eq!(id_of(&big), second);
}

#[test]
fn discard_and_shelf_keep_a_big_file_as_chunks() {
    let (env, saved) = with_a_big_file(2, 3 * MIB);
    let big = env.root().join("big.bin");
    // A discarded edit is kept as chunks, and undo brings it back.
    let edited = edit(&big);
    ok(&env, &["discard", "big.bin"]);
    assert_eq!(id_of(&big), saved);
    assert_eq!(copies_of(env.root(), &edited), [ObjectKind::ChunkList]);
    ok(&env, &["undo"]);
    assert_eq!(id_of(&big), edited);
    // Shelved, it needs nothing the store doesn't hold already.
    ok(&env, &["shelf"]);
    assert_eq!(id_of(&big), saved);
    assert_eq!(copies_of(env.root(), &edited), [ObjectKind::ChunkList]);
    ok(&env, &["unshelf"]);
    assert_eq!(id_of(&big), edited);
}

/// What this thread has read from files so far, in bytes.
#[cfg(target_os = "linux")]
fn bytes_read() -> u64 {
    let io = std::fs::read_to_string("/proc/thread-self/io").unwrap();
    let line = io.lines().find_map(|l| l.strip_prefix("rchar:")).unwrap();
    line.trim().parse().unwrap()
}

#[test]
fn an_import_reads_each_big_blob_once() {
    // One blob Git's reader hands over whole, and one too big for that, which streams.
    const STREAMED: u64 = 66 * MIB;
    let env = Env::new();
    let held = write_noise(&env.root().join("held.bin"), 3, 3 * MIB);
    let streamed = write_noise(&env.root().join("streamed.bin"), 4, STREAMED);
    env.git(&["add", "-A"]);
    env.git(&["commit", "-q", "-m", "Big files"]);
    #[cfg(target_os = "linux")]
    let before = bytes_read();
    let who = ada();
    let mut req = ImportRequest {
        who: &who,
        command: "import",
        lfs_storage: None,
        progress: &mut |_, _| {},
    };
    import_git(env.root(), &mut req, &mut None).unwrap();
    // Git's copy of each once, and a little besides: reading one twice passes 130 MiB.
    #[cfg(target_os = "linux")]
    {
        let read = bytes_read() - before;
        assert!(
            read < STREAMED + STREAMED / 2,
            "the import read {read} bytes"
        );
    }
    let top = top_folder(env.root());
    let repo = Repository::open(env.root()).unwrap();
    for (name, id) in [("held.bin", held), ("streamed.bin", streamed)] {
        assert_eq!(top.get(name).unwrap().id, id);
        assert_eq!(
            copies_of(env.root(), &id),
            [ObjectKind::ChunkList],
            "{name}"
        );
        repo.store().copy_content(&id, &mut io::sink()).unwrap();
    }
}

#[test]
fn a_long_history_of_a_big_file_imports_within_the_allowance() {
    // 50 versions of an 8 MiB file, each a few bytes on from the last, which Git stores as deltas
    // built on a few versions stored whole.
    let env = Env::new();
    let path = env.root().join("big.bin");
    write_noise(&path, 5, 8 * MIB);
    let mut versions = Vec::new();
    for v in 0..50u64 {
        let mut file = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
        file.seek(SeekFrom::Start(v * 100_000)).unwrap();
        file.write_all(format!("version {v:02}").as_bytes())
            .unwrap();
        drop(file);
        versions.push(id_of(&path));
        env.git(&["add", "big.bin"]);
        env.git(&["commit", "-q", "-m", &format!("Version {v}")]);
    }
    env.git(&["repack", "-a", "-d", "-f", "--depth=50"]);
    // With a cache of one version, as 256 MiB holds one of a 200 MiB file, rebuilding the
    // versions one by one took 1.2 GB when measured; each once takes 420 MB.
    set_test_delta_limits(Some((640 * MIB, 9 * MIB)));
    let who = ada();
    let mut req = ImportRequest {
        who: &who,
        command: "import",
        lfs_storage: None,
        progress: &mut |_, _| {},
    };
    let imported = import_git(env.root(), &mut req, &mut None);
    set_test_delta_limits(None);
    imported.unwrap();
    let repo = Repository::open(env.root()).unwrap();
    for id in &versions {
        repo.store().copy_content(id, &mut io::sink()).unwrap();
    }
    assert_eq!(
        top_folder(env.root()).get("big.bin").unwrap().id,
        versions[49]
    );
}
