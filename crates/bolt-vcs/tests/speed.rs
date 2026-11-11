//! The two speed gates for v0.1, and the deepest chain of folders a save takes. Slow, so ignored;
//! run nightly and by hand, one at a time:
//! `cargo test --release -p bolt-vcs --test speed -- --ignored --nocapture --exact <test>`
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

mod support;
use support::{Env, main_tip, run_limited, text};

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

/// The deepest chain of folders a save takes, 2,048 levels of `d`, with a file in each but the
/// last, whose path would pass 4,096 bytes: a save, a status that reads every file again, and a
/// `goto` that empties the folder and one that writes the chain out again, each timed. Run by
/// hand only, not nightly. The chain is made by absolute paths, so only Windows gets to 2,048
/// levels; elsewhere it is skipped where the system's path limit stops it.
#[test]
#[ignore = "slow: saves the deepest chain of folders, a file in each"]
fn the_deepest_chain_of_folders_with_a_file_in_each() {
    let env = Env::empty();
    env.bolt()
        .args(["start", "--preset", "custom"])
        .assert()
        .success();
    let first = main_tip(&env).to_hex();
    let Some(_chain) = Chain::make(env.root(), 2048) else {
        return;
    };
    let timed = |args: &[&str]| {
        let started = Instant::now();
        let out = env.bolt().args(args).output().unwrap();
        assert!(out.status.success(), "{args:?}: {}", text(&out.stderr));
        started.elapsed().as_secs_f64()
    };
    println!("save: {:.2} s", timed(&["save", "Chain", "--yes"]));
    let second = main_tip(&env).to_hex();
    fs::remove_file(env.root().join(".bolt/cache/files")).unwrap();
    println!("status, nothing vouched for: {:.2} s", timed(&["status"]));
    println!("goto the save before: {:.2} s", timed(&["goto", &first]));
    assert!(!env.root().join("d").exists());
    println!("goto the chain again: {:.2} s", timed(&["goto", &second]));
}

/// `levels` nested folders named `d`, each but the deepest holding a file `x`, removed from the
/// bottom up when dropped, as a path that long can't be removed whole everywhere.
struct Chain {
    top: PathBuf,
    bottom: PathBuf,
}

impl Chain {
    /// `None`, with the reason printed, where the system can't make it.
    fn make(top: &Path, levels: usize) -> Option<Chain> {
        let mut chain = Chain {
            top: top.to_path_buf(),
            bottom: top.to_path_buf(),
        };
        for level in 0..levels {
            let made = if level == 0 {
                Ok(())
            } else {
                fs::write(chain.bottom.join("x"), b"x")
            };
            chain.bottom.push("d");
            if let Err(e) = made.and_then(|()| fs::create_dir(&chain.bottom)) {
                chain.bottom.pop();
                println!("Skipped: this system can't make {levels} nested folders: {e}");
                return None;
            }
        }
        Some(chain)
    }
}

impl Drop for Chain {
    fn drop(&mut self) {
        while self.bottom != self.top {
            let _ = fs::remove_file(self.bottom.join("x"));
            if fs::remove_dir(&self.bottom).is_err() {
                break;
            }
            self.bottom.pop();
        }
    }
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
