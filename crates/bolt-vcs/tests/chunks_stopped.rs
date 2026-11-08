//! Saves of a big file that stop part way. The pack limit these tests lower, so that a save fills
//! several packs while it reads one file, reaches every command in their process, so they keep a
//! file of their own.
#![cfg(feature = "fault-injection")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::io::Write;

use bolt_core::fsio::fault;
use bolt_core::id::Id;
use bolt_core::pack::set_test_pack_limit;
use bolt_core::repo::Repository;
use bolt_core::snapshot::SaveError;
use bolt_core::verify::verify_store;

mod support;
use support::{ada, id_of, save_here, top_folder, write_noise};

/// A new project holding `big.bin`, 3 MiB or about 48 chunks, to be saved in packs of 16
/// objects; the file's ID.
fn with_a_big_file() -> (tempfile::TempDir, Id) {
    let dir = tempfile::tempdir().unwrap();
    Repository::init(dir.path(), &ada()).unwrap();
    let id = write_noise(&dir.path().join("big.bin"), 5, 3 << 20);
    set_test_pack_limit(Some(16));
    (dir, id)
}

fn head(dir: &tempfile::TempDir) -> Id {
    Repository::open(dir.path()).unwrap().head().unwrap()
}

#[test]
fn a_big_file_changed_during_the_save_stops_it() {
    let (dir, _) = with_a_big_file();
    let before = head(&dir);
    // It grows once the save has stored part of it.
    let big = dir.path().join("big.bin");
    let growing = big.clone();
    fault::act_at("packs:rolled-over", move || {
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&growing)
            .unwrap();
        file.write_all(b"more").unwrap();
    });
    match save_here(dir.path(), "Big") {
        Err(SaveError::ChangedDuringSave { path }) => assert_eq!(path, "big.bin"),
        other => panic!("{:?}", other.map(|_| ())),
    }
    assert_eq!(head(&dir), before);
    // Saved again, it is saved as it is now.
    save_here(dir.path(), "Big").unwrap();
    assert_eq!(
        top_folder(dir.path()).get("big.bin").unwrap().id,
        id_of(&big)
    );
}

#[test]
fn a_save_stopped_between_chunks_leaves_the_project_as_it_was() {
    let mut between_chunks = false;
    for n in 1.. {
        assert!(n < 300, "the save never finished");
        let (dir, id) = with_a_big_file();
        let before = head(&dir);
        fault::arm(n);
        let saved = save_here(dir.path(), "Big");
        fault::disarm();
        // Once a stop no longer stops it, it has saved.
        let Err(stop) = saved else {
            assert_eq!(top_folder(dir.path()).get("big.bin").unwrap().id, id);
            break;
        };
        between_chunks |= format!("{stop:?}").contains("packs:rolled-over");
        assert_eq!(head(&dir), before, "fault {n}");
        assert_eq!(id_of(&dir.path().join("big.bin")), id, "fault {n}");
        // The next save stores it, with what the stopped one left or without.
        save_here(dir.path(), "Big").unwrap();
        assert_eq!(
            top_folder(dir.path()).get("big.bin").unwrap().id,
            id,
            "fault {n}"
        );
        let report = verify_store(&dir.path().join(".bolt/store/packs"));
        assert!(report.is_ok(), "fault {n}: {:?}", report.problems);
    }
    assert!(
        between_chunks,
        "no stop fell between two of the file's chunks"
    );
}
