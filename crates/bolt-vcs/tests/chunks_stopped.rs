//! Saves of a big file that stop part way. The pack limit these tests lower, so that a save fills
//! several packs while it reads one file, reaches every command in their process, so they keep a
//! file of their own.
#![cfg(feature = "fault-injection")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::HashSet;
use std::io::{self, Write};
use std::path::Path;

use bolt_core::diary::Position;
use bolt_core::fsio::fault;
use bolt_core::id::Id;
use bolt_core::listing::EntryKind;
use bolt_core::pack::{IndexEntry, ObjectKind, PackReader, set_test_pack_limit};
use bolt_core::repo::Repository;
use bolt_core::shelf::shelf;
use bolt_core::snapshot::SaveError;
use bolt_core::switch::discard;
use bolt_core::verify::verify_store;
use bolt_core::warning::{HEAD_NOT_FLUSHED, SWITCH_UNFINISHED};
use bolt_core::workers::set_test_workers;

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

/// Every file of every save the diary names, from `head` back to the start, each read back whole:
/// the lines', the position's, the shelves' and both of each folder change.
fn files_named(root: &Path) -> HashSet<Id> {
    let repo = Repository::open(root).unwrap();
    let store = repo.store();
    let mut saves = Vec::new();
    let mut at = Some(repo.head().unwrap());
    while let Some(id) = at {
        let entry = repo.entry(&id).unwrap();
        saves.extend(entry.state.lines.values());
        saves.extend(entry.state.shelves.iter().map(|(_, save)| *save));
        if let Position::AtSave(save) = entry.state.position {
            saves.push(save);
        }
        if let Some(change) = entry.folder_change {
            saves.extend([change.from, change.to]);
        }
        at = entry.parent;
    }
    let mut files = HashSet::new();
    let mut folders: Vec<Id> = saves
        .iter()
        .map(|save| store.save(save, false).unwrap().root)
        .collect();
    while let Some(folder) = folders.pop() {
        for e in store.listing(&folder).unwrap().entries() {
            if e.kind == EntryKind::Folder {
                folders.push(e.id);
            } else if files.insert(e.id) {
                store.copy_content(&e.id, &mut io::sink()).unwrap();
            }
        }
    }
    files
}

#[test]
fn a_discard_or_shelf_of_a_big_file_stopped_anywhere_loses_nothing() {
    for shelving in [false, true] {
        let mut between_chunks = false;
        for n in 1.. {
            assert!(n < 300, "it never finished");
            // Saved, then replaced by 3 MiB that share no chunk with it, to throw away or shelve.
            let (dir, saved) = with_a_big_file();
            save_here(dir.path(), "Big").unwrap();
            let big = dir.path().join("big.bin");
            let unsaved = write_noise(&big, 6, 3 << 20);
            let mut repo = Repository::open(dir.path()).unwrap();
            fault::arm(n);
            let done = if shelving {
                shelf(&mut repo, None, &ada(), None, &mut |_| {}).map(drop)
            } else {
                discard(&mut repo, None, &ada(), "discard", None, &mut |_| {}).map(drop)
            };
            fault::disarm();
            // A stop once `head` moved is only a warning: the command went on to the end.
            let after_head = [HEAD_NOT_FLUSHED, SWITCH_UNFINISHED];
            let warned = repo
                .store()
                .warnings()
                .iter()
                .any(|w| after_head.contains(&w.kind));
            drop(repo);
            between_chunks |= format!("{done:?}").contains("packs:rolled-over");
            // The next command finishes what was stopped; the file is one version or the other,
            // and both are still in a save the diary names.
            save_here(dir.path(), "Next").unwrap();
            let held = id_of(&big);
            assert!([saved, unsaved].contains(&held), "fault {n}: half written");
            let named = files_named(dir.path());
            for (id, which) in [(saved, "saved"), (unsaved, "unsaved")] {
                assert!(
                    named.contains(&id),
                    "fault {n}: the {which} version is lost"
                );
            }
            let report = verify_store(&dir.path().join(".bolt/store/packs"));
            assert!(report.is_ok(), "fault {n}: {:?}", report.problems);
            if done.is_ok() && !warned {
                assert!(n > 10, "only {n} fault points");
                break;
            }
        }
        assert!(
            between_chunks,
            "no stop fell between two of the file's chunks"
        );
    }
}

/// Where a save of the big file and some small ones stops when its `n`th fault point fails, with
/// `count` workers, and the packs it leaves in the store; `None` once nothing stops it.
fn stopped_at(n: u64, count: usize) -> (Option<String>, Vec<String>) {
    let (dir, _) = with_a_big_file();
    for i in 0..40 {
        let text = format!("small file {i}\n").repeat(i + 1);
        std::fs::write(dir.path().join(format!("f{i:02}.txt")), text).unwrap();
    }
    set_test_workers(Some(count));
    fault::arm(n);
    let saved = save_here(dir.path(), "Big");
    fault::disarm();
    set_test_workers(None);
    // The point it stopped at, not the project's folder, which each run has its own of.
    let place = saved.err().map(|e| {
        let said = format!("{e:?}");
        match said.split_once("injected fault at ") {
            Some((_, at)) => at.split('"').next().unwrap_or(at).to_owned(),
            None => said,
        }
    });
    (place, packs_in(&dir.path().join(".bolt/store/packs")))
}

/// The packs in the folder `packs`, by name. One that holds a save, whose change ID is drawn at
/// random, or a record naming one, is told by the files and folders it holds.
fn packs_in(packs: &std::path::Path) -> Vec<String> {
    let mut found = Vec::new();
    for entry in std::fs::read_dir(packs).unwrap() {
        let path = entry.unwrap().path();
        let objects: Vec<IndexEntry> = PackReader::open(&path)
            .unwrap()
            .entries()
            .map(Result::unwrap)
            .collect();
        let made = |e: &&IndexEntry| {
            use ObjectKind::{ChunkList, Content, FolderListing};
            matches!(e.kind, Content | ChunkList | FolderListing)
        };
        found.push(if objects.iter().all(|e| made(&e)) {
            path.file_name().unwrap().to_string_lossy().into_owned()
        } else {
            let ids: Vec<String> = objects
                .iter()
                .filter(made)
                .map(|e| e.id.to_string())
                .collect();
            format!("with a save: {}", ids.join(","))
        });
    }
    found.sort();
    found
}

#[test]
fn a_crash_sweep_of_a_threaded_save_stops_at_the_same_places() {
    for n in 1.. {
        assert!(n < 300, "the save never finished");
        let one = stopped_at(n, 1);
        assert_eq!(one, stopped_at(n, 8), "fault {n}");
        if one.0.is_none() {
            assert!(n > 10, "only {n} fault points");
            break;
        }
    }
}
