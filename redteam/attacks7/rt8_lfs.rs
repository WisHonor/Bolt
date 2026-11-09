//! Exploration: Git LFS import edges (red-team milestone 5).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod support;

use std::fs;
use std::path::Path;

use bolt_core::git::craft::lfs_pointer_to;
use support::{Env, main_tip, saved_files, text, write};

const TRACKED: &str = "*.bin filter=lfs diff=lfs merge=lfs -text\n";

fn keep(env: &Env, contents: &[u8]) -> String {
    let (pointer, at) = lfs_pointer_to(contents);
    let at = env.root().join(".git").join(at);
    fs::create_dir_all(at.parent().unwrap()).unwrap();
    fs::write(at, contents).unwrap();
    pointer
}

fn import(env: &Env, extra_env: &[(&str, &str)]) -> (Option<i32>, String, String) {
    let mut cmd = env.bolt();
    cmd.arg("import");
    for (k, v) in extra_env {
        cmd.env(k, v);
    }
    let out = cmd.output().unwrap();
    (out.status.code(), text(&out.stdout), text(&out.stderr))
}

fn lfs_env(env: &Env, extra: &[(&str, &str)]) -> String {
    let mut cmd = std::process::Command::new("git");
    cmd.current_dir(env.root())
        .env("HOME", env.config())
        .env("XDG_CONFIG_HOME", env.config())
        .args(["lfs", "env"]);
    for (k, v) in extra {
        cmd.env(k, v);
    }
    let out = cmd.output().unwrap();
    text(&out.stdout)
        .lines()
        .filter(|l| l.starts_with("LocalMediaDir"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn repo_with_object_elsewhere(elsewhere: &Path) -> Env {
    let env = Env::new();
    env.commit(".gitattributes", TRACKED, "Track");
    let contents = b"the real picture\n";
    let (pointer, at) = lfs_pointer_to(contents);
    // Kept where lfs.storage says, as Git LFS keeps it.
    let kept = elsewhere.join(at.strip_prefix("lfs/").unwrap());
    fs::create_dir_all(kept.parent().unwrap()).unwrap();
    fs::write(kept, contents).unwrap();
    write(&env, "a.bin", pointer);
    env.commit("readme.txt", "hi\n", "Add a picture");
    env
}

#[test]
fn explore_lfs_storage_in_a_users_include_if() {
    let elsewhere = tempfile::tempdir().unwrap();
    let env = repo_with_object_elsewhere(elsewhere.path());
    let inc = env.config().join("work.inc");
    fs::write(
        &inc,
        format!("[lfs]\n\tstorage = {}\n", elsewhere.path().display()),
    )
    .unwrap();
    fs::write(
        env.config().join(".gitconfig"),
        format!(
            "[includeIf \"gitdir:{}/\"]\n\tpath = {}\n",
            env.root().display(),
            inc.display()
        ),
    )
    .unwrap();
    eprintln!("git lfs env: {}", lfs_env(&env, &[]));
    let (code, out, err) = import(&env, &[]);
    eprintln!("bolt import: {code:?}\n{out}\n{err}");
}

#[test]
fn explore_lfs_storage_in_the_system_config() {
    let elsewhere = tempfile::tempdir().unwrap();
    let env = repo_with_object_elsewhere(elsewhere.path());
    let sys = env.config().join("system.gitconfig");
    fs::write(
        &sys,
        format!("[lfs]\n\tstorage = {}\n", elsewhere.path().display()),
    )
    .unwrap();
    let s = sys.display().to_string();
    let extra = [("GIT_CONFIG_SYSTEM", s.as_str())];
    eprintln!("git lfs env: {}", lfs_env(&env, &extra));
    let (code, out, err) = import(&env, &extra);
    eprintln!("bolt import: {code:?}\n{out}\n{err}");
}

#[test]
fn explore_git_config_global_hides_the_home_setting() {
    let env = Env::new();
    env.commit(".gitattributes", TRACKED, "Track");
    let contents = b"the real picture\n";
    write(&env, "a.bin", keep(&env, contents));
    env.commit("readme.txt", "hi\n", "Add a picture");
    // ~/.gitconfig sets lfs.storage, but Git reads GIT_CONFIG_GLOBAL instead of it.
    fs::write(
        env.config().join(".gitconfig"),
        "[lfs]\n\tstorage = /nowhere/shared-lfs\n",
    )
    .unwrap();
    let real = env.config().join("real.gitconfig");
    fs::write(&real, "[user]\n\tname = Someone\n").unwrap();
    let r = real.display().to_string();
    let extra = [("GIT_CONFIG_GLOBAL", r.as_str())];
    eprintln!("git lfs env: {}", lfs_env(&env, &extra));
    let (code, out, err) = import(&env, &extra);
    eprintln!("bolt import: {code:?}\n{out}\n{err}");
}

#[test]
fn explore_a_pointer_padded_past_1024_bytes() {
    let env = Env::new();
    env.commit(".gitattributes", TRACKED, "Track");
    let contents = b"what Git LFS checks out\n";
    let mut padded = keep(&env, contents).into_bytes();
    padded.extend(std::iter::repeat_n(b' ', 1100));
    padded.extend_from_slice(b"\nanything at all after the first 1,024 bytes\n");
    write(&env, "a.bin", &padded);
    env.commit("readme.txt", "hi\n", "Add a picture");
    let (code, out, err) = import(&env, &[]);
    eprintln!("bolt import: {code:?}\n{out}\n{err}");
    let saved = saved_files(&env, main_tip(&env));
    eprintln!(
        "saved a.bin: {} bytes, is the object: {}",
        saved["a.bin"].len(),
        saved["a.bin"] == contents
    );
}
