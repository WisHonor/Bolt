//! Git LFS repositories, end to end: a Git LFS file comes in as what it stands for, read from
//! `.git/lfs/objects`, which these tests fill by hand; the last two tests use Git LFS itself,
//! where it is installed.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use bolt_core::git::craft::lfs_pointer_to;
use bolt_core::id::hash_content;
use bolt_core::pack::{ObjectKind, PackReader};
use bolt_core::repo::Repository;
use bolt_core::text::bytes;
use support::{
    Env, copies_of, files, git_commit, git_with_input, link_file, link_folder, main_tip,
    saved_files, text, write,
};

/// A `.gitattributes` that has Git LFS keep these files.
const TRACKED: &str = "*.psd filter=lfs diff=lfs merge=lfs -text\n\
                       *.bin filter=lfs diff=lfs merge=lfs -text\n";

/// `len` bytes that look random, made from `seed`, so they don't compress.
fn noise(seed: u64, len: usize) -> Vec<u8> {
    let mut state = seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1;
    (0..len)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state >> 24) as u8
        })
        .collect()
}

/// The pointer to `contents`, whose object nobody fetched.
fn pointer(contents: &[u8]) -> String {
    lfs_pointer_to(contents).0
}

/// Where the object of `contents` goes in the repository.
fn object_path(env: &Env, contents: &[u8]) -> PathBuf {
    env.root().join(".git").join(lfs_pointer_to(contents).1)
}

/// Keeps `contents` in the repository's `.git/lfs/objects`, as `git lfs fetch` does; the pointer
/// to them.
fn keep(env: &Env, contents: &[u8]) -> String {
    let at = object_path(env, contents);
    std::fs::create_dir_all(at.parent().unwrap()).unwrap();
    std::fs::write(at, contents).unwrap();
    pointer(contents)
}

/// A repository whose `.gitattributes` has Git LFS keep `*.psd` and `*.bin`.
fn tracked() -> Env {
    let env = Env::new();
    env.commit(".gitattributes", TRACKED, "Track art with Git LFS");
    env
}

/// Runs `bolt import --json`, which must work, and returns its document.
fn imported_json(env: &Env) -> serde_json::Value {
    let out = env.bolt().args(["--json", "import"]).output().unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    serde_json::from_slice(&out.stdout).unwrap()
}

/// How many objects of `kind` the project at `root` stores, in all its packs.
fn stored(root: &Path, kind: ObjectKind) -> usize {
    std::fs::read_dir(root.join(".bolt/store/packs"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|path| path.extension().is_some_and(|e| e == "pack"))
        .map(|path| {
            let pack = PackReader::open(&path).unwrap();
            pack.entries()
                .filter(|e| e.as_ref().unwrap().kind == kind)
                .count()
        })
        .sum()
}

/// Checks that an import that stopped after it began `.bolt` committed nothing.
#[track_caller]
fn committed_nothing(env: &Env, err: &str) {
    assert!(err.contains("Nothing was committed."), "{err}");
    assert!(env.root().join(".bolt").is_dir(), "{err}");
    assert!(!env.root().join(".bolt/format").exists(), "{err}");
}

#[test]
fn an_import_brings_in_git_lfs_files_as_their_contents() {
    let env = tracked();
    let small = b"a small picture\n".to_vec();
    // Big enough to be cut into chunks, as any big file is.
    let big = noise(1, 3 << 20);
    write(&env, "art/small.psd", keep(&env, &small));
    write(&env, "art/big.bin", keep(&env, &big));
    env.commit("readme.txt", "hello\n", "Add art");
    let (out, _) = env.import();
    let total = bytes((small.len() + big.len()) as u64);
    assert!(
        out.contains(&format!(
            "Brought in 2 Git LFS files ({total}) from .git/lfs/objects."
        )),
        "{out}"
    );
    let saved = saved_files(&env, main_tip(&env));
    assert_eq!(saved["art/small.psd"], small);
    assert!(saved["art/big.bin"] == big);
    assert_eq!(saved["readme.txt"], b"hello\n");
    assert_eq!(
        copies_of(env.root(), &hash_content(&big)),
        [ObjectKind::ChunkList]
    );
}

/// A repository cloned without `git lfs fetch --all` stops before `.bolt` is made, listing the
/// first 20 missing, with their count and size, and the command that fetches them.
#[test]
fn an_import_stops_before_writing_when_lfs_objects_are_missing() {
    let env = tracked();
    let contents = |n: u8| vec![n; 1000];
    for n in 0..12 {
        write(&env, &format!("art/a{n:02}.psd"), pointer(&contents(n)));
    }
    let first = env.commit("readme.txt", "first\n", "Twelve pictures");
    // One that is there isn't listed.
    write(&env, "art/kept.psd", keep(&env, b"a kept picture\n"));
    for n in 12..25 {
        write(&env, &format!("art/b{n:02}.psd"), pointer(&contents(n)));
    }
    let second = env.commit("readme.txt", "second\n", "Thirteen more");
    let err = env.refused_import();
    assert!(!env.root().join(".bolt").exists(), "{err}");
    let total = bytes(25_000);
    assert!(
        err.contains(&format!(
            "error: 25 Git LFS files in this history, {total} in all, aren't in .git/lfs/objects:\n"
        )),
        "{err}"
    );
    let size = bytes(1000);
    assert!(
        err.contains(&format!("\n  `art/a00.psd` in commit `{first}` ({size})\n")),
        "{err}"
    );
    assert!(
        err.contains(&format!(
            "\n  `art/b19.psd` in commit `{second}` ({size})\n"
        )),
        "{err}"
    );
    assert!(!err.contains("art/b20.psd"), "{err}");
    assert!(!err.contains("kept.psd"), "{err}");
    assert_eq!(err.matches(" in commit `").count(), 20, "{err}");
    assert!(err.contains("\n  and 5 more\n"), "{err}");
    assert!(
        err.contains(
            "hint: Nothing was changed. Download every version Git LFS keeps with `git lfs fetch \
             --all`, then run bolt import again."
        ),
        "{err}"
    );
    // Once they are there, the import goes through.
    for n in 0..25 {
        keep(&env, &contents(n));
    }
    env.import();
    assert_eq!(
        saved_files(&env, main_tip(&env))["art/b24.psd"],
        contents(24)
    );
}

/// With no `.git/lfs` and no `filter=lfs`, only the converter meets a pointer, once `.bolt` is
/// begun; nothing is committed, and the next run finishes from what was stored.
#[test]
fn without_a_sign_a_missing_object_stops_the_second_pass() {
    let env = Env::new();
    env.commit("a.txt", "a\n", "First");
    let commit = env.commit(
        "art/big.psd",
        &pointer(b"a big picture\n"),
        "Add a big file",
    );
    let err = env.refused_import();
    assert!(
        err.contains(&format!(
            "error: A Git LFS file in this history isn't in .git/lfs/objects:\n  `art/big.psd` in \
             commit `{commit}` (14 bytes)\n"
        )),
        "{err}"
    );
    assert!(err.contains("git lfs fetch --all"), "{err}");
    committed_nothing(&env, &err);
    keep(&env, b"a big picture\n");
    env.import();
    assert_eq!(
        saved_files(&env, main_tip(&env))["art/big.psd"],
        b"a big picture\n"
    );
}

#[test]
fn an_object_whose_sha256_differs_stops_the_import() {
    // One read whole, and one cut into chunks as it is read.
    for len in [100, 2 << 20] {
        let env = tracked();
        let real = noise(3, len);
        let commit = env.commit("art/big.psd", &keep(&env, &real), "Add a picture");
        // Other bytes of the same length where its object goes.
        std::fs::write(object_path(&env, &real), noise(4, len)).unwrap();
        let err = env.refused_import();
        let (_, path) = lfs_pointer_to(&real);
        let oid = path.rsplit('/').next().unwrap();
        assert!(
            err.contains(&format!(
                "error: The Git LFS file `art/big.psd` in commit `{commit}` doesn't match its \
                 pointer: what .git/lfs/objects holds for object `{oid}` has another SHA-256.\n"
            )),
            "{len}: {err}"
        );
        // Fetching skips an object that is there, so `fsck` moves the bad copy aside first; it
        // looks at `HEAD` only unless given the commit.
        let fsck = err.find(&format!("run `git lfs fsck {commit}`, which moves"));
        let fetch = err.find("then `git lfs fetch --all`");
        assert!(
            fsck.zip(fetch).is_some_and(|(fsck, fetch)| fsck < fetch),
            "{len}: {err}"
        );
        committed_nothing(&env, &err);
    }
}

/// An object counts only as a plain file of its pointer's size, so a folder, a link, or a file
/// shorter or longer than its pointer says, by a byte or by far, is missing, and stops the import
/// before `.bolt` is made.
#[test]
fn an_object_of_the_wrong_size_or_kind_is_missing() {
    let real = b"the real picture\n";
    for case in [
        "folder",
        "link",
        "shorter file",
        "a byte longer",
        "longer file",
    ] {
        let env = tracked();
        let at = object_path(&env, real);
        std::fs::create_dir_all(at.parent().unwrap()).unwrap();
        match case {
            "folder" => std::fs::create_dir(&at).unwrap(),
            "link" => {
                let target = env.config().join("real");
                std::fs::write(&target, real).unwrap();
                if !link_file(&target, &at) {
                    continue;
                }
            }
            "shorter file" => std::fs::write(&at, &real[1..]).unwrap(),
            "a byte longer" => std::fs::write(&at, [&real[..], b"!"].concat()).unwrap(),
            _ => std::fs::write(&at, noise(5, 1 << 20)).unwrap(),
        }
        env.commit("art/big.psd", &pointer(real), "Add a picture");
        let err = env.refused_import();
        assert!(
            err.contains("error: A Git LFS file in this history isn't in .git/lfs/objects:\n"),
            "{case}: {err}"
        );
        assert!(err.contains("hint: Nothing was changed."), "{case}: {err}");
        assert!(!env.root().join(".bolt").exists(), "{case}");
    }
}

/// A link in `.git/lfs` is never followed, even to the right object. One at `.git/lfs` or
/// `.git/lfs/objects` keeps the objects elsewhere, so the import stops naming it; behind one at a
/// folder below, the object is missing.
#[test]
fn a_link_inside_git_lfs_is_never_followed() {
    let real = b"the real picture\n";
    let (_, path) = lfs_pointer_to(real);
    let parts: Vec<&str> = path.split('/').collect();
    for (linked, kept_by) in [
        (0, Some(".git/lfs")),
        (1, Some(".git/lfs/objects")),
        (2, None),
    ] {
        let env = tracked();
        // The object, kept right, in a folder outside the repository.
        let elsewhere = env.config().join("elsewhere");
        let file = parts[linked + 1..]
            .iter()
            .fold(elsewhere.clone(), |at, part| at.join(part));
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, real).unwrap();
        let link = parts[..=linked]
            .iter()
            .fold(env.root().join(".git"), |at, part| at.join(part));
        std::fs::create_dir_all(link.parent().unwrap()).unwrap();
        link_folder(&elsewhere, &link);
        let commit = env.commit("art/big.psd", &pointer(real), "Add a picture");
        let err = env.refused_import();
        let says = match kept_by {
            Some(folder) => format!(
                "error: Commit `{commit}` holds a Git LFS file at `art/big.psd`, but Git LFS's \
                 files are kept outside .git/lfs/objects, where Bolt reads them: {folder} is a \
                 link, or isn't a folder, and Bolt never follows a link in .git.\n"
            ),
            None => "error: A Git LFS file in this history isn't in .git/lfs/objects:\n".to_owned(),
        };
        assert!(err.contains(&says), "{}: {err}", parts[linked]);
        assert!(!env.root().join(".bolt").exists(), "{}", parts[linked]);
    }
}

#[test]
fn a_pointer_with_extensions_stops_the_import() {
    let real = b"the real picture\n";
    let version = "version https://git-lfs.github.com/spec/v1\n";
    let extended = |pointer: &str| {
        let rest = pointer.strip_prefix(version).unwrap();
        format!("{version}ext-0-foo sha256:{}\n{rest}", "f".repeat(64))
    };
    let says = |commit: &str| {
        format!(
            "error: Commit `{commit}` holds a Git LFS pointer at `art/big.psd` with extensions \
             (`ext-` lines): only Git LFS can rebuild its file, by running a program, which Bolt \
             never does.\n"
        )
    };
    // With a sign of Git LFS, the first pass stops at it before `.bolt` is made.
    let env = tracked();
    let commit = env.commit("art/big.psd", &extended(&keep(&env, real)), "Add a picture");
    let err = env.refused_import();
    assert!(err.contains(&says(&commit)), "{err}");
    assert!(err.contains("git lfs migrate export"), "{err}");
    assert!(!env.root().join(".bolt").exists(), "{err}");
    // Without one, the second pass does.
    let env = Env::new();
    let commit = env.commit("art/big.psd", &extended(&pointer(real)), "Add a picture");
    let err = env.refused_import();
    assert!(err.contains(&says(&commit)), "{err}");
    committed_nothing(&env, &err);
}

/// What an import that stops at `art/big.psd` in `commit` says when `lfs.storage` is set to
/// `/mnt/shared-lfs` in `file`.
fn kept_by_setting(commit: &str, file: &str) -> String {
    format!(
        "error: Commit `{commit}` holds a Git LFS file at `art/big.psd`, but `lfs.storage` keeps \
         Git LFS's files outside .git/lfs/objects, where Bolt reads them: it is set to \
         `/mnt/shared-lfs` in {file}.\n"
    )
}

/// `lfs.storage` naming Git LFS's own place, `lfs` or nothing, keeps nothing elsewhere.
#[test]
fn lfs_storage_set_to_git_lfss_own_place_imports_as_usual() {
    for value in ["lfs", ""] {
        let env = tracked();
        env.git(&["config", "lfs.storage", value]);
        env.commit("art/big.psd", &keep(&env, b"a picture\n"), "Add a picture");
        env.import();
        let saved = saved_files(&env, main_tip(&env));
        assert_eq!(saved["art/big.psd"], b"a picture\n", "{value:?}");
    }
}

/// `lfs.storage` set elsewhere stops only an import that has a Git LFS file to bring in.
#[test]
fn lfs_storage_elsewhere_never_stops_a_repository_without_lfs_files() {
    // Set for other repositories, in the user's own Git settings.
    let env = Env::new();
    let storage = "[lfs]\n\tstorage = /mnt/shared-lfs\n";
    std::fs::write(env.config().join(".gitconfig"), storage).unwrap();
    env.commit("a.txt", "a\n", "First");
    env.import();
    // Set in the repository, whose `.gitattributes` names Git LFS's filter for files it hasn't.
    let env = tracked();
    env.git(&["config", "lfs.storage", "/mnt/shared-lfs"]);
    env.commit("a.txt", "a\n", "First");
    env.import();
}

/// With a sign of Git LFS, `lfs.storage` set elsewhere stops the import at the first Git LFS file,
/// before `.bolt` is made, even with its object at hand, saying where it is set: `.git/config`,
/// or `config.worktree`, which Git reads after it.
#[test]
fn lfs_storage_elsewhere_stops_an_import_of_lfs_files_before_writing() {
    for file in [".git/config", ".git/config.worktree"] {
        let env = tracked();
        if file == ".git/config" {
            env.git(&["config", "lfs.storage", "/mnt/shared-lfs"]);
        } else {
            env.git(&["config", "lfs.storage", "lfs"]);
            env.git(&["config", "extensions.worktreeConfig", "true"]);
            env.git(&["config", "--worktree", "lfs.storage", "/mnt/shared-lfs"]);
        }
        let commit = env.commit("art/big.psd", &keep(&env, b"a picture\n"), "Add a picture");
        let err = env.refused_import();
        assert!(
            err.contains(&kept_by_setting(&commit, file)),
            "{file}: {err}"
        );
        assert!(
            err.contains(
                "hint: Nothing was changed. Remove that setting, or set it to lfs, download the \
                 files into .git/lfs/objects with `git lfs fetch --all`, then run bolt import \
                 again."
            ),
            "{file}: {err}"
        );
        assert!(!env.root().join(".bolt").exists(), "{file}: {err}");
    }
}

/// Without a sign, the import stops at the first Git LFS file the second pass meets.
#[test]
fn without_a_sign_lfs_storage_elsewhere_stops_at_the_first_lfs_file() {
    let env = Env::new();
    env.git(&["config", "lfs.storage", "/mnt/shared-lfs"]);
    env.commit("a.txt", "a\n", "First");
    let commit = env.commit("art/big.psd", &pointer(b"a picture\n"), "Add a picture");
    let err = env.refused_import();
    assert!(
        err.contains(&kept_by_setting(&commit, ".git/config")),
        "{err}"
    );
    committed_nothing(&env, &err);
}

/// The user's own Git settings, as Bolt finds them (here in the test's own home folder, never the
/// user's), come first, so the repository's own setting wins over them, as in Git.
#[test]
fn lfs_storage_in_the_users_own_git_settings_stops_unless_the_repository_sets_its_own() {
    let storage = "[lfs]\n\tstorage = /mnt/shared-lfs\n";
    for included in [true, false] {
        let env = tracked();
        let home = env.config();
        let set_in = if included {
            // In a file `~/.gitconfig` includes.
            std::fs::write(home.join(".gitconfig"), "[include]\n\tpath = lfs.inc\n").unwrap();
            home.join("lfs.inc")
        } else {
            std::fs::create_dir_all(home.join("git")).unwrap();
            home.join("git").join("config")
        };
        std::fs::write(&set_in, storage).unwrap();
        let commit = env.commit("art/big.psd", &keep(&env, b"a picture\n"), "Add a picture");
        let err = env.refused_import();
        let says = kept_by_setting(&commit, &set_in.display().to_string());
        assert!(err.contains(&says), "{included}: {err}");
        assert!(!env.root().join(".bolt").exists(), "{included}");
    }
    let env = tracked();
    std::fs::write(env.config().join(".gitconfig"), storage).unwrap();
    env.git(&["config", "lfs.storage", "lfs"]);
    env.commit("art/big.psd", &keep(&env, b"a picture\n"), "Add a picture");
    env.import();
    let saved = saved_files(&env, main_tip(&env));
    assert_eq!(saved["art/big.psd"], b"a picture\n");
}

/// A save keeps a file's contents, not that it was a pointer, so the summary counts the commits
/// whose Git IDs an export can give back only by turning such files back into pointers.
#[test]
fn the_summary_counts_commits_holding_lfs_files() {
    let history = || {
        let env = Env::new();
        env.commit("readme.txt", "hello\n", "First");
        env.commit(".gitattributes", TRACKED, "Track art with Git LFS");
        let picture = keep(&env, b"picture one\n");
        env.commit("art/a.psd", &picture, "Add a picture");
        // The `art` folder again, as it was.
        env.commit("readme.txt", "hello again\n", "Edit the readme");
        env
    };
    let (out, _) = history().import();
    assert!(
        out.contains(
            "2 commits hold Git LFS files, so a later export can recreate their Git IDs only by \
             turning those files back into pointers.\n"
        ),
        "{out}"
    );
    assert!(
        out.contains("Brought in 1 Git LFS file (12 bytes) from .git/lfs/objects.\n"),
        "{out}"
    );
    let json = imported_json(&history());
    assert_eq!(json["lfs_commits"], 2);
    assert_eq!(json["lfs_files"]["count"], 1);
    assert_eq!(json["lfs_files"]["bytes"], 12);
    assert_eq!(json["inexact_lfs_pointers"], 0);
}

/// A file Git LFS's parser refuses is an ordinary file to Git LFS, which checks it out as it is.
#[test]
fn a_lookalike_comes_in_as_it_is_with_a_warning() {
    let history = || {
        let env = tracked();
        // A line after `size`, which Git LFS doesn't read.
        for n in 0..6u8 {
            let looks = format!("{}and more\n", pointer(&[n]));
            write(&env, &format!("art/{n}.psd"), looks);
        }
        let commit = env.commit("readme.txt", "hello\n", "Add lookalikes");
        (env, commit)
    };
    let (env, commit) = history();
    let (_, err) = env.import();
    assert!(
        err.contains(
            "warning: 6 files look like Git LFS pointers, though Git LFS doesn't read them as \
             such, so Bolt imported them as they are, as Git LFS checks them out:\n"
        ),
        "{err}"
    );
    assert!(
        err.contains(&format!(
            "\n  `art/4.psd` in commit `{commit}`\n  and 1 more"
        )),
        "{err}"
    );
    assert!(!err.contains("art/5.psd"), "{err}");
    let saved = saved_files(&env, main_tip(&env));
    assert_eq!(
        text(&saved["art/5.psd"]),
        format!("{}and more\n", pointer(&[5]))
    );
    let (env, _) = history();
    let json = imported_json(&env);
    let kinds: Vec<&str> = json["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| w["kind"].as_str().unwrap())
        .collect();
    assert!(kinds.contains(&"lfs_lookalikes"), "{json}");
    assert_eq!(json["lfs_files"]["count"], 0);
}

/// A pointer of size 0 is an empty file, and its object, which Git LFS never keeps, isn't read:
/// so where the objects are kept doesn't matter to it, in either pass. It counts as a Git LFS
/// file, and as a pointer not in Git LFS's form, which writes an empty file as an empty blob.
#[test]
fn a_pointer_of_size_zero_is_an_empty_file() {
    for sign in [true, false] {
        let env = if sign { tracked() } else { Env::new() };
        env.git(&["config", "lfs.storage", "/mnt/shared-lfs"]);
        env.commit("art/empty.psd", &pointer(b""), "Add an empty picture");
        assert!(!object_path(&env, b"").exists());
        let json = imported_json(&env);
        assert_eq!(json["lfs_files"]["count"], 1, "{sign}");
        assert_eq!(json["lfs_files"]["bytes"], 0, "{sign}");
        assert_eq!(json["inexact_lfs_pointers"], 1, "{sign}");
        let saved = saved_files(&env, main_tip(&env));
        assert_eq!(saved["art/empty.psd"], b"", "{sign}");
    }
}

/// Bolt doesn't read a config's `[include]` or `[includeIf]` sections, so when the repository's
/// config has one, the hint for missing objects says that one setting `lfs.storage` changes
/// nothing for Bolt: in either pass.
#[test]
fn the_hint_for_missing_objects_names_config_includes() {
    let note = "This repository's Git config has [include] or [includeIf] sections, which Bolt \
                doesn't read: if one sets lfs.storage, the files must be in .git/lfs/objects all \
                the same.";
    for sign in [true, false] {
        let env = if sign { tracked() } else { Env::new() };
        env.git(&["config", "include.path", "lfs.inc"]);
        env.commit("art/big.psd", &pointer(b"a picture\n"), "Add a picture");
        let err = env.refused_import();
        assert!(err.contains("isn't in .git/lfs/objects"), "{sign}: {err}");
        assert!(err.contains(note), "{sign}: {err}");
    }
    let env = tracked();
    env.commit("art/big.psd", &pointer(b"a picture\n"), "Add a picture");
    let err = env.refused_import();
    assert!(!err.contains("[include]"), "{err}");
}

/// A `.git/lfs` Bolt can't look into is looked at only once an import has a Git LFS file to bring
/// in, so an import with none comes in.
#[cfg(unix)]
#[test]
fn an_unreadable_git_lfs_folder_stops_only_an_import_with_lfs_files() {
    use std::os::unix::fs::PermissionsExt;
    let lock = |env: &Env, mode| {
        let lfs = env.root().join(".git/lfs");
        std::fs::set_permissions(lfs, std::fs::Permissions::from_mode(mode)).unwrap();
    };
    let import = |env: &Env| {
        std::fs::create_dir_all(env.root().join(".git/lfs/objects")).unwrap();
        lock(env, 0o000);
        let readable = std::fs::read_dir(env.root().join(".git/lfs")).is_ok();
        let out = env.bolt().arg("import").output().unwrap();
        lock(env, 0o755);
        (readable, out)
    };
    let env = tracked();
    env.commit("a.txt", "a\n", "First");
    let (readable, out) = import(&env);
    if readable {
        eprintln!("note: skipped, this account can read any folder");
        return;
    }
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    let env = tracked();
    env.commit("art/big.psd", &pointer(b"a picture\n"), "Add a picture");
    let (_, out) = import(&env);
    assert_eq!(out.status.code(), Some(1), "{}", text(&out.stdout));
    let err = text(&out.stderr);
    assert!(err.contains("error: Bolt couldn't read "), "{err}");
    assert!(!env.root().join(".bolt").exists(), "{err}");
}

/// A later export can give a pointer back only in the form Git LFS writes, so the others are
/// counted.
#[test]
fn pointers_not_in_the_exact_form_are_counted() {
    let history = || {
        let env = tracked();
        let (a, b, c) = (b"one\n", b"two\n", b"three\n");
        write(&env, "art/a.psd", keep(&env, a));
        write(&env, "art/b.psd", keep(&env, b).replace('\n', "\r\n"));
        let pre_release = keep(&env, c).replace("git-lfs.github.com", "hawser.github.com");
        write(&env, "art/c.psd", pre_release);
        env.commit("readme.txt", "hello\n", "Add pictures");
        env
    };
    let env = history();
    let (out, _) = env.import();
    assert!(
        out.contains(
            "2 Git LFS pointers aren't in the exact form Git LFS writes, so a later export can't \
             recreate them as they were.\n"
        ),
        "{out}"
    );
    let saved = saved_files(&env, main_tip(&env));
    assert_eq!(saved["art/a.psd"], b"one\n");
    assert_eq!(saved["art/b.psd"], b"two\n");
    assert_eq!(saved["art/c.psd"], b"three\n");
    let json = imported_json(&history());
    assert_eq!(json["inexact_lfs_pointers"], 2);
    assert_eq!(json["lfs_files"]["count"], 3);
}

/// Git LFS keeps every version of a file whole; Bolt keeps one version's chunks and the few the
/// next one changed.
#[test]
fn versions_of_a_big_lfs_file_share_their_chunks() {
    let first = noise(6, 3 << 20);
    let mut second = first.clone();
    second[1_500_000..1_500_006].copy_from_slice(b"edited");
    let one = tracked();
    one.commit("big.bin", &keep(&one, &first), "A big file");
    one.import();
    let two = tracked();
    two.commit("big.bin", &keep(&two, &first), "A big file");
    two.commit("big.bin", &keep(&two, &second), "Edit it");
    two.import();
    let (chunks_of_one, chunks_of_two) = (
        stored(one.root(), ObjectKind::Content),
        stored(two.root(), ObjectKind::Content),
    );
    assert!(chunks_of_one > 20, "{chunks_of_one}");
    // The chunk the edit is in, and perhaps one either side of it.
    assert!(
        chunks_of_two - chunks_of_one <= 3,
        "{chunks_of_one} then {chunks_of_two}"
    );
    assert!(saved_files(&two, main_tip(&two))["big.bin"] == second);
}

const POINTERS_HINT: &str = "Nothing was saved. Run `git lfs checkout` to have Git LFS write the \
                             files, then save again. If Git LFS can't, `bolt discard` with those \
                             paths puts back the files the import brought in.";

/// The folder holds the pointers Git checked out, as it does where Git LFS wasn't installed.
#[test]
fn the_first_save_after_an_import_stops_at_lfs_pointers_in_the_folder() {
    let env = tracked();
    for n in 1..=7 {
        write(&env, &format!("art/p{n}.psd"), keep(&env, &noise(n, 50)));
    }
    env.commit("readme.txt", "hello\n", "Add pictures");
    env.import();
    // Line endings Git converted too: the pointers come first, and --accept-crlf can't help.
    env.overwrite("readme.txt", "hello\r\n");
    let before = env.standing();
    for args in [&["Save it"][..], &["--accept-crlf", "Save it"]] {
        let out = env.save(args);
        assert_eq!(out.status.code(), Some(1), "{}", text(&out.stdout));
        let err = text(&out.stderr);
        assert!(
            err.contains(
                "error: 7 files are Git LFS pointers, not the files they stand for: Git LFS \
                 wasn't installed when the folder was checked out, or was told to skip them.\n  \
                 art/p1.psd\n  art/p2.psd\n  art/p3.psd\n  art/p4.psd\n  art/p5.psd\n  \
                 and 2 more\n"
            ),
            "{err}"
        );
        assert!(err.contains(&format!("hint: {POINTERS_HINT}\n")), "{err}");
        assert_eq!(env.standing(), before);
    }

    let out = env
        .bolt()
        .args(["--json", "save", "Save it"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(json["error"]["hint"], POINTERS_HINT);
    let message = json["error"]["message"].as_str().unwrap();
    assert!(
        message.starts_with("7 files are Git LFS pointers"),
        "{message}"
    );
    assert_eq!(env.standing(), before);
}

/// Only a pointer to a file of the imported version's size stands in for it.
#[test]
fn a_pointer_of_another_size_is_saved_as_a_change() {
    let env = tracked();
    let a = b"a picture\n";
    write(&env, "art/a.psd", keep(&env, a));
    write(&env, "art/b.psd", keep(&env, b"b picture\n"));
    env.commit("readme.txt", "hello\n", "Add pictures");
    env.import();
    let other = pointer(b"another, bigger picture\n");
    write(&env, "art/b.psd", &other);
    let out = env.save(&["Point elsewhere"]);
    assert_eq!(out.status.code(), Some(1), "{}", text(&out.stdout));
    let err = text(&out.stderr);
    assert!(
        err.contains(
            "error: 1 file is a Git LFS pointer, not the file it stands for: Git LFS wasn't \
             installed when the folder was checked out, or was told to skip it.\n  art/a.psd\n"
        ),
        "{err}"
    );
    assert!(!err.contains("b.psd"), "{err}");

    // The picture as Git LFS writes it out.
    write(&env, "art/a.psd", a);
    let out = env.save(&["Point elsewhere"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    assert!(
        text(&out.stdout).contains("(1 file)"),
        "{}",
        text(&out.stdout)
    );
    let saved = saved_files(&env, main_tip(&env));
    assert_eq!(saved["art/a.psd"], a);
    assert_eq!(saved["art/b.psd"], other.as_bytes());
}

#[test]
fn only_the_first_save_after_an_import_looks() {
    let env = tracked();
    let picture = b"a picture\n";
    write(&env, "art/a.psd", keep(&env, picture));
    env.commit("readme.txt", "hello\n", "Add a picture");
    env.import();
    env.overwrite("readme.txt", "hello again\n");
    let out = env.save(&["First"]);
    assert_eq!(out.status.code(), Some(1), "{}", text(&out.stdout));
    write(&env, "art/a.psd", picture);
    let out = env.save(&["First"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    // The save before the next one is Bolt's, not Git's: a pointer is a change like any other.
    write(&env, "art/a.psd", pointer(picture));
    let out = env.save(&["Back to the pointer"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    let saved = saved_files(&env, main_tip(&env));
    assert_eq!(saved["art/a.psd"], pointer(picture).as_bytes());
}

/// An imported save with one Git LFS file, whose pointer the folder still holds, and an edit.
fn imported_with_a_pointer() -> Env {
    let env = tracked();
    write(&env, "art/a.psd", keep(&env, b"a picture\n"));
    env.commit("readme.txt", "hello\n", "Add a picture");
    env.import();
    env.overwrite("readme.txt", "hello again\n");
    env
}

#[test]
fn an_amend_of_the_imported_save_stops_at_lfs_pointers_too() {
    let env = imported_with_a_pointer();
    let before = env.standing();
    let out = env.save(&["--amend"]);
    assert_eq!(out.status.code(), Some(1), "{}", text(&out.stdout));
    let err = text(&out.stderr);
    assert!(err.contains("Git LFS pointer"), "{err}");
    assert!(err.contains("\n  art/a.psd\n"), "{err}");
    assert_eq!(env.standing(), before);
    write(&env, "art/a.psd", b"a picture\n");
    let out = env.save(&["--amend"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    let saved = saved_files(&env, main_tip(&env));
    assert_eq!(saved["art/a.psd"], b"a picture\n");
    assert_eq!(saved["readme.txt"], b"hello again\n");
}

/// A save of some paths looks only at those, and saves only those.
#[test]
fn a_save_of_some_paths_stops_only_at_pointers_among_them() {
    let env = imported_with_a_pointer();
    let before = env.standing();
    let out = env.save(&["Some", "art/a.psd", "readme.txt"]);
    assert_eq!(out.status.code(), Some(1), "{}", text(&out.stdout));
    let err = text(&out.stderr);
    assert!(err.contains("\n  art/a.psd\n"), "{err}");
    assert_eq!(env.standing(), before);
    let out = env.save(&["Some", "readme.txt"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    let saved = saved_files(&env, main_tip(&env));
    assert_eq!(saved["art/a.psd"], b"a picture\n");
    assert_eq!(saved["readme.txt"], b"hello again\n");
}

/// The hint's second way, for when Git LFS can't write the files any more.
#[test]
fn discarding_the_pointers_puts_back_what_the_import_brought_in() {
    let env = tracked();
    let (a, b) = (noise(1, 50), noise(2, 60));
    write(&env, "art/a.psd", keep(&env, &a));
    write(&env, "art/b.psd", keep(&env, &b));
    env.commit("readme.txt", "hello\n", "Add pictures");
    env.import();
    // Git LFS's own copies gone, so `git lfs checkout` can't help.
    std::fs::remove_dir_all(env.root().join(".git/lfs")).unwrap();
    env.overwrite("readme.txt", "hello again\n");
    let out = env.save(&["Save it"]);
    assert_eq!(out.status.code(), Some(1), "{}", text(&out.stdout));
    let err = text(&out.stderr);
    assert!(err.contains("\n  art/a.psd\n  art/b.psd\n"), "{err}");
    assert!(err.contains(&format!("hint: {POINTERS_HINT}\n")), "{err}");

    let out = env
        .bolt()
        .args(["discard", "art/a.psd", "art/b.psd"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    assert!(std::fs::read(env.root().join("art/a.psd")).unwrap() == a);
    let out = env.save(&["Save it"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    assert!(
        text(&out.stdout).contains("(1 file)"),
        "{}",
        text(&out.stdout)
    );
    let saved = saved_files(&env, main_tip(&env));
    assert_eq!(saved["art/a.psd"], a);
    assert_eq!(saved["art/b.psd"], b);
    assert_eq!(saved["readme.txt"], b"hello again\n");
}

/// Whether `git lfs` runs here.
fn has_git_lfs(dir: &Path) -> bool {
    git_with_input(dir, &["lfs", "version"], b"")
        .status
        .success()
}

#[test]
fn an_lfs_repository_matches_what_git_lfs_checks_out() {
    let env = Env::new();
    if !has_git_lfs(env.root()) {
        eprintln!("note: skipped, git lfs isn't installed");
        return;
    }
    // Into this repository's own config: the tests' Git has no other settings to change.
    env.git(&["lfs", "install", "--local"]);
    env.git(&["lfs", "track", "*.bin", "*.psd"]);
    let commit = |message: &str| {
        env.git(&["add", "-A"]);
        env.git(&["commit", "-q", "-m", message]);
        (env.git(&["rev-parse", "HEAD"]), files(env.root()))
    };
    let mut big = noise(7, 3 << 20);
    write(&env, "big.bin", &big);
    write(&env, "art/small.psd", "a small picture\n");
    write(&env, "readme.txt", "hello\n");
    let mut committed = vec![commit("First")];
    // Git holds the pointers, Git LFS the files.
    let held = env.git(&["cat-file", "-p", "HEAD:big.bin"]);
    assert!(
        held.starts_with("version https://git-lfs.github.com/spec/v1\n"),
        "{held}"
    );
    big[1_500_000..1_500_006].copy_from_slice(b"edited");
    write(&env, "big.bin", &big);
    write(&env, "art/new.psd", noise(8, 5000));
    committed.push(commit("Edit the big file"));
    std::fs::remove_file(env.root().join("art/small.psd")).unwrap();
    committed.push(commit("Remove a picture"));
    env.import();
    let repo = Repository::open(env.root()).unwrap();
    let mut saves = BTreeMap::new();
    let mut next = Some(main_tip(&env));
    while let Some(save) = next {
        saves.insert(git_commit(&repo, &save), save);
        next = repo
            .store()
            .save(&save, false)
            .unwrap()
            .parents
            .first()
            .copied();
    }
    for (commit, files) in committed {
        assert!(saved_files(&env, saves[&commit]) == files, "{commit}");
    }
}

#[test]
fn after_git_lfs_checkout_the_save_goes_through() {
    let env = Env::new();
    if !has_git_lfs(env.root()) {
        eprintln!("note: skipped, git lfs isn't installed");
        return;
    }
    env.git(&["lfs", "install", "--local"]);
    env.git(&["lfs", "track", "*.bin"]);
    let big = noise(9, 5000);
    write(&env, "big.bin", &big);
    write(&env, "readme.txt", "hello\n");
    env.git(&["add", "-A"]);
    env.git(&["commit", "-q", "-m", "First"]);
    // Checked out again with Git LFS told to skip, which leaves the pointer.
    env.git(&["lfs", "install", "--local", "--skip-smudge"]);
    std::fs::remove_file(env.root().join("big.bin")).unwrap();
    env.git(&["checkout", "--", "big.bin"]);
    let held = std::fs::read(env.root().join("big.bin")).unwrap();
    assert!(held.starts_with(b"version https://git-lfs.github.com/spec/v1\n"));
    env.import();
    let out = env.save(&["Save it"]);
    assert_eq!(out.status.code(), Some(1), "{}", text(&out.stdout));
    let err = text(&out.stderr);
    assert!(err.contains("Git LFS pointer"), "{err}");
    assert!(err.contains("\n  big.bin\n"), "{err}");

    env.git(&["lfs", "checkout"]);
    assert!(std::fs::read(env.root().join("big.bin")).unwrap() == big);
    write(&env, "readme.txt", "hello again\n");
    let out = env.save(&["Save it"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    let saved = saved_files(&env, main_tip(&env));
    assert!(saved["big.bin"] == big);
    assert_eq!(saved["readme.txt"], b"hello again\n");
}
