//! Helpers the integration tests share.
// Each test file uses only some of them.
#![allow(dead_code)]

use std::io::{self, Write};
use std::path::Path;
use std::process::{Command, Output, Stdio};

/// Runs `git args` in `dir` with `input` on its standard input, and nothing from this computer's
/// Git set-up: no system or user config, no replace objects, and messages in plain C. A test that
/// needs Git fails, saying so, where Git isn't installed.
pub fn git_with_input(dir: &Path, args: &[&str], input: &[u8]) -> Output {
    let home = tempfile::tempdir().unwrap();
    let empty_config = home.path().join("gitconfig");
    std::fs::write(&empty_config, b"").unwrap();
    let mut cmd = Command::new("git");
    cmd.current_dir(dir).env_clear();
    for key in ["PATH", "SystemRoot", "TEMP", "TMP"] {
        if let Some(value) = std::env::var_os(key) {
            cmd.env(key, value);
        }
    }
    cmd.env("HOME", home.path());
    if cfg!(windows) {
        cmd.env("USERPROFILE", home.path());
    }
    cmd.env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", &empty_config)
        .env("GIT_NO_REPLACE_OBJECTS", "1")
        .env("LC_ALL", "C")
        .args(["-c", "core.autocrlf=false", "-c", "user.name=Test"])
        .args([
            "-c",
            "user.email=test@example.com",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(["-c", "init.defaultBranch=main"])
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = match cmd.spawn() {
        Ok(child) => child,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            panic!("this test compares Bolt with Git, and git isn't installed or isn't on PATH")
        }
        Err(e) => panic!("can't run git: {e}"),
    };
    let mut stdin = child.stdin.take().unwrap();
    let input = input.to_vec();
    // Fed from a thread of its own, so a full output pipe can't stall Git while it reads.
    let feeder = std::thread::spawn(move || stdin.write_all(&input));
    let out = child.wait_with_output().unwrap();
    // A Git that stopped early shows that in its status and messages, not here.
    let _ = feeder.join();
    out
}

/// Runs `git args` in `dir` as `git_with_input` does, and returns what it printed. The test fails
/// if Git does.
pub fn git(dir: &Path, args: &[&str]) -> Vec<u8> {
    let out = git_with_input(dir, args, b"");
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    out.stdout
}
