//! Decision 023's speed bar: `status` on an unchanged 100,000-file project. Slow, so it is
//! ignored; run by hand and nightly:
//! `cargo test --release -p bolt-vcs --test speed -- --ignored --nocapture`
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;
use std::io::Write;
use std::time::{Duration, Instant, SystemTime};

mod support;
use support::{Env, text};

const FILES: usize = 100_000;
const PER_FOLDER: usize = 100;

#[test]
#[ignore = "slow: builds a 100,000-file project"]
fn status_on_an_unchanged_100_000_file_project() {
    let env = Env::empty();
    env.bolt().arg("start").assert().success();
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
    env.bolt().args(["save", "Files", "--yes"]).assert().success();
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
