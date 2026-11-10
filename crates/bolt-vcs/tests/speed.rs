//! The two speed gates for v0.1. Slow, so ignored; run nightly and by hand, one at a time:
//! `cargo test --release -p bolt-vcs --test speed -- --ignored --nocapture --exact <test>`
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;
use std::io::Write;
use std::path::Path;
use std::time::{Duration, Instant, SystemTime};

mod support;
use support::{Env, run_limited, text};

const FILES: usize = 100_000;
const PER_FOLDER: usize = 100;

// The memory gate's file, and the most memory saving it may take.
const BIG: u64 = 10 << 30;
const MOST_BYTES: u64 = 500_000_000;

#[test]
#[ignore = "slow: builds a 100,000-file project"]
fn status_on_an_unchanged_100_000_file_project() {
    let env = Env::empty();
    env.bolt()
        .args(["start", "--preset", "custom"])
        .assert()
        .success();
    // An hour old, so the cache can vouch for every file.
    let then = SystemTime::now() - Duration::from_secs(3600);
    for i in 0..FILES {
        let folder = env.root().join(format!("d{:04}", i / PER_FOLDER));
        if i % PER_FOLDER == 0 {
            fs::create_dir(&folder).unwrap();
        }
        let mut file = fs::File::create(folder.join(format!("f{i:06}.txt"))).unwrap();
        writeln!(file, "file {i}").unwrap();
        file.set_modified(then).unwrap();
    }
    let started = Instant::now();
    env.bolt()
        .args(["save", "Files", "--yes"])
        .assert()
        .success();
    println!("first save: {:.2} s", started.elapsed().as_secs_f64());
    let mut times = Vec::new();
    for _ in 0..2 {
        let started = Instant::now();
        let out = env.bolt().arg("status").output().unwrap();
        times.push(started.elapsed().as_secs_f64());
        assert!(out.status.success(), "{}", text(&out.stderr));
        assert!(
            text(&out.stdout).contains("Nothing changed"),
            "{}",
            text(&out.stdout)
        );
    }
    println!("status: {:.2} s, then {:.2} s", times[0], times[1]);
}

#[test]
#[ignore = "slow: writes and saves a 10 GiB file"]
fn saving_a_10_gib_file_stays_under_500_mb() {
    if !cfg!(target_os = "linux") {
        println!("Skipped: the memory gate is measured on Linux, whose /proc tells a peak.");
        return;
    }
    // Both folders go when `env` is dropped, after a failed assert too.
    let env = Env::empty();
    env.bolt()
        .args(["start", "--preset", "custom"])
        .assert()
        .success();
    write_incompressible(&env.root().join("big.bin"));
    let mut save = env.bolt_process_in(env.root());
    save.args(["save", "A 10 GiB file", "--yes"]);
    let ended = run_limited(save, 1800);
    println!(
        "saving 10 GiB: peak {} bytes, {:.1} s",
        ended.peak_bytes, ended.secs
    );
    assert_eq!(ended.code, Some(0), "{ended:?}");
    // A save that left the file out would pass the gate too.
    let packs = env.root().join(".bolt").join("store").join("packs");
    let stored: u64 = fs::read_dir(packs)
        .unwrap()
        .map(|e| e.unwrap().metadata().unwrap().len())
        .sum();
    assert!(stored >= BIG, "only {stored} bytes stored");
    // A running bolt holds more than 1 MiB, so less means no peak was read.
    assert!(
        ended.peak_bytes >= 1 << 20,
        "read no peak from /proc: {ended:?}"
    );
    assert!(
        ended.peak_bytes <= MOST_BYTES,
        "peaked above {MOST_BYTES} bytes"
    );
}

/// Writes the memory gate's file where `BOLT_GATE_FILE` says, to measure a save by hand on a
/// system without /proc (docs/benchmarks.md).
#[test]
#[ignore = "writes 10 GiB where BOLT_GATE_FILE says"]
fn write_the_memory_gate_file() {
    let Some(path) = std::env::var_os("BOLT_GATE_FILE") else {
        println!("Set BOLT_GATE_FILE to the file to write.");
        return;
    };
    write_incompressible(Path::new(&path));
}

/// Writes `BIG` bytes of BLAKE3's extendable output at `path`, a mebibyte at a time: none of it
/// compresses, and no two chunks match.
fn write_incompressible(path: &Path) {
    let mut file = fs::File::create(path).unwrap();
    let mut xof = blake3::Hasher::new()
        .update(b"bolt memory gate")
        .finalize_xof();
    let mut block = vec![0; 1 << 20];
    for _ in 0..BIG >> 20 {
        xof.fill(&mut block);
        file.write_all(&block).unwrap();
    }
    // On disk before the save starts, so the save's time is its own.
    file.sync_all().unwrap();
}
