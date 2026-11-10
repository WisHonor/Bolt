//! Exploration: memory and time of the big-file commands (red-team milestone 5). Linux only.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![cfg(target_os = "linux")]
mod support;

use std::fs;
use std::process::Command;

use support::{Env, run_limited, text, write_noise};

fn sh(dir: &std::path::Path, script: &str) {
    let out = Command::new("sh")
        .current_dir(dir)
        .arg("-c")
        .arg(script)
        .output()
        .unwrap();
    assert!(out.status.success(), "{script}: {}", text(&out.stderr));
}

fn timed(env: &Env, args: &[&str]) -> String {
    let mut cmd = env.bolt_process_in(env.root());
    cmd.args(args);
    let e = run_limited(cmd, 1200);
    format!(
        "{args:?}: code={:?} secs={:.1} peak={} MiB {}",
        e.code,
        e.secs,
        e.peak_mb,
        e.stderr.lines().next().unwrap_or("")
    )
}

#[test]
fn explore_save_and_write_out_of_a_1_gib_file() {
    let env = Env::empty();
    env.bolt().arg("start").assert().success();
    write_noise(&env.root().join("big.bin"), 1, 256 << 20);
    eprintln!("{}", timed(&env, &["save", "--yes", "Big"]));
    write_noise(&env.root().join("big.bin"), 2, 256 << 20);
    eprintln!("{}", timed(&env, &["status"]));
    eprintln!("{}", timed(&env, &["discard"]));
    eprintln!("{}", timed(&env, &["undo"]));
    fs::remove_file(env.root().join("big.bin")).unwrap();
}

/// A big file Git stored as a delta on another version: the import rebuilds its family.
#[test]
fn explore_import_of_a_delta_family() {
    let env = Env::new();
    let size = 120 << 20;
    write_noise(&env.root().join("big.bin"), 3, size);
    env.git(&["add", "-A"]);
    env.git(&["commit", "-q", "-m", "one"]);
    for i in 0..3u8 {
        sh(
            env.root(),
            &format!("printf 'edit {i}' | dd of=big.bin bs=1 seek={} conv=notrunc 2>/dev/null", 1_000_000 * (i as u64 + 1)),
        );
        env.git(&["add", "-A"]);
        env.git(&["commit", "-q", "-m", &format!("edit {i}")]);
    }
    env.git(&["repack", "-a", "-d", "-f", "-q"]);
    eprintln!("{}", env.git(&["count-objects", "-v"]));
    eprintln!("{}", timed(&env, &["import"]));
    fs::remove_file(env.root().join("big.bin")).unwrap();
}
