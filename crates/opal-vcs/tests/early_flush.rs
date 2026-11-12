//! A big pack, or a big file a switch writes, flushed while it is written. Nothing here lowers
//! the pack limit, which reaches the whole process, so a save of 40 MiB writes a pack past 16 MiB.
#![cfg(feature = "fault-injection")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::ffi::OsString;
use std::path::Path;
use std::sync::{Mutex, MutexGuard, PoisonError};

use opal_core::fsio::fault::{self, Early};
use opal_core::id::Id;
use opal_core::lines::Target;
use opal_core::pack::ObjectKind;
use opal_core::repo::Repository;
use opal_core::snapshot::SaveOutcome;
use opal_core::switch::goto;
use tempfile::TempDir;

mod support;
use support::{ada, entries_with_bytes, id_of, save_here, write_noise};

/// Memory and disk are short: one 40 MiB project at a time.
fn one_at_a_time() -> MutexGuard<'static, ()> {
    static BIG: Mutex<()> = Mutex::new(());
    BIG.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A new project holding `big.bin`, 40 MiB that don't compress; the file's ID.
fn with_40_mib() -> (TempDir, Id) {
    let dir = tempfile::tempdir().unwrap();
    Repository::init(dir.path(), &ada()).unwrap();
    let id = write_noise(&dir.path().join("big.bin"), 11, 40 << 20);
    (dir, id)
}

fn head(root: &Path) -> Id {
    Repository::open(root).unwrap().head().unwrap()
}

fn pack_files(root: &Path) -> Vec<OsString> {
    let mut names: Vec<OsString> = std::fs::read_dir(root.join(".opalro/store/packs"))
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    names.sort();
    names
}

#[test]
fn an_early_flush_that_fails_stops_the_save() {
    let _one = one_at_a_time();
    let (dir, _) = with_40_mib();
    let (before, packs) = (head(dir.path()), pack_files(dir.path()));
    fault::early_flush(Early::Fails);
    let saved = save_here(dir.path(), "Big");
    fault::early_flush(Early::On);
    let said = format!("{:?}", saved.map(|_| ()));
    assert!(said.contains("injected fault at flush:early"), "{said}");
    assert_eq!(head(dir.path()), before);
    assert_eq!(pack_files(dir.path()), packs);
}

#[test]
fn an_early_flush_that_fails_stops_a_goto() {
    let _one = one_at_a_time();
    let (dir, id) = with_40_mib();
    let Ok(SaveOutcome::Saved { save_id, .. }) = save_here(dir.path(), "Big") else {
        panic!("not saved");
    };
    let big = dir.path().join("big.bin");
    std::fs::remove_file(&big).unwrap();
    save_here(dir.path(), "Gone").unwrap();
    let mut repo = Repository::open(dir.path()).unwrap();
    fault::early_flush(Early::Fails);
    let done = goto(
        &mut repo,
        &Target::Save(save_id),
        &ada(),
        "goto",
        None,
        &mut |_| {},
    );
    fault::early_flush(Early::On);
    drop(repo);
    let said = format!("{:?}", done.map(|_| ()));
    assert!(said.contains("injected fault at flush:early"), "{said}");
    assert!(!big.exists());
    assert!(dir.path().join(".opalro/state/journal").exists());
    // The next command finishes the switch.
    save_here(dir.path(), "Next").unwrap();
    assert_eq!(id_of(&big), id);
    assert!(!dir.path().join(".opalro/state/journal").exists());
}

/// One pack as stored: each entry's ID, kind, offset, stored size and a fingerprint of its
/// bytes, then every entry's offset, in order of offset.
type PackAsStored = (Vec<(Id, ObjectKind, u64, u64, Id)>, Vec<u64>);

/// The packs in the store of the project at `root`, as stored.
fn packs_as_stored(root: &Path) -> Vec<PackAsStored> {
    let mut packs: Vec<PackAsStored> = std::fs::read_dir(root.join(".opalro/store/packs"))
        .unwrap()
        .map(|pack| {
            let entries = entries_with_bytes(&pack.unwrap().path());
            // A save record and a diary entry hold a random change ID and the time, so their
            // stored sizes differ from run to run and what follows them moves: not compared.
            let kept: Vec<_> = entries
                .into_iter()
                .filter(|(e, _)| !matches!(e.kind, ObjectKind::Save | ObjectKind::DiaryEntry))
                .map(|(e, bytes)| (e.id, e.kind, e.offset, e.stored_size, bytes))
                .collect();
            // In order of offset too: the index is in order of ID.
            let mut offsets: Vec<u64> = kept.iter().map(|entry| entry.2).collect();
            offsets.sort_unstable();
            (kept, offsets)
        })
        .collect();
    packs.sort_by(|a, b| format!("{a:?}").cmp(&format!("{b:?}")));
    packs
}

#[test]
fn a_save_writes_the_same_packs_with_and_without_the_helper() {
    let _one = one_at_a_time();
    let saved = |how| {
        let (dir, _) = with_40_mib();
        fault::early_flush(how);
        let before = fault::early_flushes();
        save_here(dir.path(), "Big").unwrap();
        fault::early_flush(Early::On);
        let flushed = fault::early_flushes() - before;
        (packs_as_stored(dir.path()), flushed)
    };
    let (with, flushed) = saved(Early::On);
    let (without, none) = saved(Early::Off);
    assert!(flushed >= 1, "{flushed}");
    assert_eq!(none, 0);
    assert_eq!(with, without);
}
