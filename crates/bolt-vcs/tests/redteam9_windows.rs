//! Milestone 5's second red-team session (decision 024): attacks only Windows can run, as tests
//! of the safe behaviour.
#![cfg(windows)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use std::fs;
use std::path::Path;
use std::process::Command;

use support::{Env, main_tip, saved_files, text, write};

/// `path` as a Git setting's value: Git reads a `\` there as an escape, so it is written `/`.
fn in_git_settings(path: &Path) -> String {
    path.display().to_string().replace('\\', "/")
}

/// `cmd` as a user runs it on Windows, where `HOME` is usually not set: Git for Windows then makes
/// one from `HOMEDRIVE` and `HOMEPATH` when they name a folder (a domain's home drive, say), else
/// from `USERPROFILE`. Here they name `home`.
fn with_home_drive(cmd: &mut Command, home: &Path) {
    let home = home.to_str().unwrap();
    let (drive, path) = home.split_at(2);
    cmd.env_remove("HOME")
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("GIT_CONFIG_GLOBAL")
        .env_remove("GIT_CONFIG_COUNT")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("HOMEDRIVE", drive)
        .env("HOMEPATH", path);
}

/// Bolt takes the home folder from `HOME`, else from the system's profile folder. Git for Windows,
/// finding no `HOME`, takes `HOMEDRIVE` and `HOMEPATH` first, so where a domain maps a home drive
/// Git reads that drive's `.gitconfig` and Bolt the profile's: their personal ignore lists differ.
/// Bolt must keep out exactly what Git keeps out.
#[test]
fn without_home_a_save_applies_the_ignore_list_git_for_windows_applies() {
    let env = Env::new();
    let home = tempfile::tempdir().unwrap();
    let list = home.path().join("real.ignore");
    fs::write(&list, "*.key\n").unwrap();
    let excludes = format!("[core]\n\texcludesFile = {}\n", in_git_settings(&list));
    fs::write(home.path().join(".gitconfig"), excludes).unwrap();
    write(&env, "deploy.key", "-----BEGIN PRIVATE KEY-----\n");
    write(&env, "readme.txt", "hello\n");

    let mut origin = Command::new("git");
    origin
        .current_dir(env.root())
        .args(["config", "--show-origin", "--get", "core.excludesfile"]);
    with_home_drive(&mut origin, home.path());
    let origin = origin.output().unwrap();
    let mut status = Command::new("git");
    status
        .current_dir(env.root())
        .args(["status", "--porcelain", "--untracked-files=all"]);
    with_home_drive(&mut status, home.path());
    let status = status.output().unwrap();
    let git_keeps_out_key = !text(&status.stdout).contains("deploy.key");
    eprintln!(
        "HOMEDRIVE+HOMEPATH = {}; USERPROFILE = {:?}\ngit config --show-origin: {}git status: {}",
        home.path().display(),
        std::env::var_os("USERPROFILE"),
        text(&origin.stdout),
        text(&status.stdout).replace('\n', " ")
    );

    env.bolt().arg("start").assert().success();
    let mut save = env.bolt_process_in(env.root());
    save.args(["save", "First"]);
    with_home_drive(&mut save, home.path());
    let out = save.output().unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    let saved = saved_files(&env, main_tip(&env));
    let names: Vec<&String> = saved.keys().collect();
    eprintln!("bolt save: {}saved: {names:?}", text(&out.stdout));
    assert_eq!(
        saved.contains_key("deploy.key"),
        !git_keeps_out_key,
        "saved {names:?}, where Git's status was {}",
        text(&status.stdout).replace('\n', " ")
    );
}
