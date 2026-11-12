//! Milestone 5's red-team session (decision 024): attacks only Windows can run, as tests of the
//! safe behaviour. Big files are cut into chunks from 1 MiB, so each test's files are bigger.
#![cfg(windows)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use std::fs;
use std::path::{Path, PathBuf};

use opal_core::id::{Id, hash_content};
use support::{Env, git, id_of, noise, text};

const MIB: usize = 1 << 20;

fn ok(env: &Env, dir: &Path, args: &[&str]) -> String {
    let out = env.opal_in(dir).args(args).output().unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "{args:?}: {}{}",
        text(&out.stdout),
        text(&out.stderr)
    );
    text(&out.stdout)
}

fn put(path: &Path, bytes: &[u8]) -> Id {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, bytes).unwrap();
    hash_content(bytes)
}

/// A project holding `big.bin`, cut into chunks, saved as `saved`.
fn project(saved: &[u8]) -> Env {
    let env = Env::empty();
    ok(&env, env.root(), &["start", "--preset", "custom"]);
    put(&env.root().join("big.bin"), saved);
    ok(&env, env.root(), &["save", "Big"]);
    env
}

/// CVE-2025-26625 for chunked files: a hard link at a path `discard`, `undo`, `unshelf` and
/// `redo` write, to a file outside the project, must leave that file as it was.
#[test]
fn a_hard_link_at_a_big_file_a_switch_writes_is_never_written_through() {
    let saved = noise(1, 2 * MIB + 7);
    let env = project(&saved);
    let outside = tempfile::tempdir().unwrap();
    let victim = outside.path().join("victim.bin");
    let precious = noise(2, 2 * MIB + 3);
    let precious_id = put(&victim, &precious);
    let big = env.root().join("big.bin");

    // The folder's big.bin is the outside file, so its bytes are unsaved work there.
    fs::remove_file(&big).unwrap();
    fs::hard_link(&victim, &big).unwrap();
    ok(&env, env.root(), &["discard"]);
    assert_eq!(
        id_of(&victim),
        precious_id,
        "discard wrote through the link"
    );
    assert_eq!(id_of(&big), hash_content(&saved));
    ok(&env, env.root(), &["undo"]);
    assert_eq!(id_of(&victim), precious_id, "undo wrote through the link");
    assert_eq!(id_of(&big), precious_id);

    // A shelf brought back onto a hard link that holds what the save expects.
    ok(&env, env.root(), &["discard"]);
    let mut edited = saved.clone();
    edited[1_500_000..1_500_008].copy_from_slice(b"SHELVED!");
    put(&big, &edited);
    ok(&env, env.root(), &["shelf"]);
    assert_eq!(id_of(&big), hash_content(&saved));
    let same = outside.path().join("same.bin");
    let same_id = put(&same, &saved);
    fs::remove_file(&big).unwrap();
    fs::hard_link(&same, &big).unwrap();
    ok(&env, env.root(), &["unshelf"]);
    assert_eq!(id_of(&same), same_id, "unshelf wrote through the link");
    assert_eq!(id_of(&big), hash_content(&edited));
    ok(&env, env.root(), &["undo"]);
    ok(&env, env.root(), &["redo"]);
    assert_eq!(id_of(&same), same_id, "redo wrote through the link");
    assert_eq!(id_of(&victim), precious_id);
    assert_eq!(id_of(&big), hash_content(&edited));
}

/// A big file made read-only (as Perforce and some art tools leave files) with unsaved work in
/// it: `discard` either replaces it and keeps the work, or refuses and leaves it as it was.
#[test]
fn a_read_only_big_file_with_unsaved_work_is_never_lost() {
    let saved = noise(3, 3 * MIB);
    let env = project(&saved);
    let big = env.root().join("big.bin");
    let mut edited = saved.clone();
    edited[2_000_000..2_000_008].copy_from_slice(b"MY WORK!");
    let work = put(&big, &edited);
    let mut perms = fs::metadata(&big).unwrap().permissions();
    perms.set_readonly(true);
    fs::set_permissions(&big, perms).unwrap();

    let out = env.opalro().arg("discard").output().unwrap();
    if out.status.code() == Some(0) {
        assert_eq!(id_of(&big), hash_content(&saved));
        ok(&env, env.root(), &["undo"]);
        assert_eq!(id_of(&big), work, "the work thrown away didn't come back");
    } else {
        let said = text(&out.stderr);
        assert!(said.contains("big.bin"), "{said}");
        assert_eq!(id_of(&big), work, "a refused discard changed the file");
    }
}

/// A folder name as long as Windows lets a test make in one step.
const LONG: &str = "a-folder-name-long-enough-to-pass-max-path";

/// A project folder's path of 245 characters, so that what Opalro writes in `.opalro/tmp`, the files
/// in it, and Git LFS's objects in `.git/lfs/objects` are over 260. `.opalro` itself stays under,
/// since it is hidden by its path, even where Windows spells the temp folder out longer (GitHub's
/// `RUNNER~1`). Its parent is made.
fn deep_folder(base: &Path) -> PathBuf {
    let mut deep = base.to_path_buf();
    while deep.as_os_str().len() + LONG.len() + 1 < 245 {
        deep.push(LONG);
    }
    let left = 245 - deep.as_os_str().len() - 1;
    deep.push("x".repeat(left.max(1)));
    fs::create_dir_all(deep.parent().unwrap()).unwrap();
    deep
}

/// Paths past MAX_PATH below the project: an import of a big Git LFS file, the first save's stop
/// at its pointer, and the `opalro discard` it names, which writes the file back through `.opalro/tmp`.
/// Git itself can't work that deep, so the repository is made at a short path, then moved there.
#[test]
fn a_project_whose_files_pass_max_path_imports_and_writes_back_git_lfs_files() {
    let env = Env::empty();
    let deep = deep_folder(env.root());
    let staging = tempfile::tempdir().unwrap();
    let short = staging.path().join("r");
    fs::create_dir(&short).unwrap();
    git(&short, &["init", "-q"]);
    let name = "art/a-picture-whose-path-is-long-too.bin";
    let contents = noise(4, 2 * MIB + 11);
    let (pointer, object) = opal_core::git::craft::lfs_pointer_to(&contents);
    put(&short.join(".git").join(&object), &contents);
    put(&short.join(name), pointer.as_bytes());
    put(
        &short.join(".gitattributes"),
        b"*.bin filter=lfs diff=lfs merge=lfs -text\n",
    );
    git(&short, &["add", "-A"]);
    git(&short, &["commit", "-q", "-m", "Add a picture"]);
    fs::rename(&short, &deep).unwrap();
    assert!(deep.join(name).as_os_str().len() > 260);
    assert!(deep.join(".git").join(&object).as_os_str().len() > 260);

    ok(&env, &deep, &["import"]);
    let out = env.opal_in(&deep).args(["save", "Next"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1), "{}", text(&out.stdout));
    assert!(
        text(&out.stderr).contains("Git LFS pointer"),
        "{}",
        text(&out.stderr)
    );
    ok(&env, &deep, &["discard", name]);
    assert_eq!(id_of(&deep.join(name)), hash_content(&contents));
    let out = env.opal_in(&deep).args(["status"]).output().unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    assert!(
        !text(&out.stdout).contains("modified"),
        "{}",
        text(&out.stdout)
    );
}
