//! Measures the process's peak memory, so this file holds a single test: any other test running
//! beside it would add its own. Only Linux tells the peak (VmHWM), and lets a process reset it.
#![cfg(target_os = "linux")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use bolt_core::drive::Drive;
use bolt_core::history::current_save;
use bolt_core::journal::Journal;
use bolt_core::repo::Repository;
use bolt_core::save::Person;
use bolt_core::snapshot::{SaveRequest, save_folder};
use bolt_core::writeout::{apply, plan};
use std::io::Write;

const SIZE: u64 = 300 << 20;

/// The process's peak resident memory since it was last reset, in bytes.
fn peak() -> u64 {
    let status = std::fs::read_to_string("/proc/self/status").unwrap();
    let line = status.lines().find(|l| l.starts_with("VmHWM:")).unwrap();
    line.split_whitespace()
        .nth(1)
        .unwrap()
        .parse::<u64>()
        .unwrap()
        * 1024
}

fn save(repo: &mut Repository) {
    let who = Person {
        name: "Ada".into(),
        email: "ada@example.com".into(),
        time_ms: 1_700_000_000_000,
        tz_offset_min: 0,
    };
    let req = SaveRequest {
        message: "Big",
        who: &who,
        command: "save",
        accept_crlf: false,
        git_excludes: None,
    };
    save_folder(repo, &req, &mut |_| {}).unwrap();
}

#[test]
fn writing_a_big_file_uses_little_memory() {
    let dir = tempfile::tempdir().unwrap();
    let mut repo = Repository::init(
        dir.path(),
        &Person {
            name: "Ada".into(),
            email: "ada@example.com".into(),
            time_ms: 1_700_000_000_000,
            tz_offset_min: 0,
        },
    )
    .unwrap();
    // Bytes that don't compress, so the store holds all 300 MiB.
    let big = dir.path().join("big");
    let mut file = std::fs::File::create(&big).unwrap();
    let mut state = 0x9e37_79b9_7f4a_7c15_u64;
    let mut chunk = vec![0; 1 << 20];
    for _ in 0..(SIZE >> 20) {
        for word in chunk.chunks_mut(8) {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            word.copy_from_slice(&state.to_le_bytes());
        }
        file.write_all(&chunk).unwrap();
    }
    drop(file);
    save(&mut repo);
    let with = current_save(&repo).unwrap();
    let with_entry = repo.head().unwrap();
    std::fs::remove_file(&big).unwrap();
    save(&mut repo);
    let without = current_save(&repo).unwrap();

    std::fs::write("/proc/self/clear_refs", "5").unwrap();
    let before = peak();
    let lock = repo.lock_for_change("goto", &mut |_| {}).unwrap();
    let drive = Drive {
        ignores_case: false,
        composes_unicode: false,
    };
    let plan = plan(&repo, without, with, None, false, drive, None).unwrap();
    let journal = Journal {
        target: with_entry,
        from: repo.head().unwrap(),
        actions: plan.actions,
    };
    let done = apply(&repo, &lock, &journal, drive).unwrap();
    let grew = peak().saturating_sub(before);
    assert_eq!(done.written, 1);
    assert_eq!(std::fs::metadata(&big).unwrap().len(), SIZE);
    assert!(
        grew < 100 << 20,
        "writing a 300 MiB file took {grew} bytes more"
    );
}
