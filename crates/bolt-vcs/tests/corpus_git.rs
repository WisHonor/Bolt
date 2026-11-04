//! Hostile Git repositories (security.md §7): each case is written byte by byte with `Craft` and
//! read only by Bolt. `bolt import` must refuse it, or skip and list what it must, and commit
//! nothing when it stops. A case that can only be built below the import names the test that
//! holds it.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use bolt_core::diff::for_each_change;
use bolt_core::git::craft::{Craft, PackEntry, delta, entry_header, zlib};
use bolt_core::git::{GitError, GitId, HashKind, ObjectType, object_id};
use bolt_core::history::current_save;
use bolt_core::id::Id;
use bolt_core::import::{ImportError, ImportReport, ImportRequest, import_git};
use bolt_core::link::ProjectLink;
use bolt_core::listing::EntryKind;
use bolt_core::pack::ObjectKind;
use bolt_core::repo::Repository;
use bolt_core::save::Person;
use bolt_core::store::StoreError;
use bolt_core::text::escape_all;
use support::{Env, git_commit, text, top_folder};

use ObjectType::{Blob, Commit, Tag, Tree};

const PEOPLE: &str = "author Ada <ada@example.com> 1700000000 +0000\n\
                      committer Ada <ada@example.com> 1700000000 +0000\n";

/// An empty folder with a Git repository in it, on `main`, which has no commits yet.
fn repo() -> (Env, Craft) {
    let env = Env::empty();
    let git = Craft::new(env.root(), HashKind::Sha1);
    (env, git)
}

fn blob_id(bytes: &[u8]) -> GitId {
    object_id(Blob, bytes, HashKind::Sha1).unwrap()
}

/// A tree of `entries` in Git's order: by name, a folder's as if it ended in `/`.
fn tree(git: &Craft, entries: &[(&[u8], &[u8], GitId)]) -> GitId {
    let mut entries = entries.to_vec();
    entries.sort_by_key(|&(mode, name, _)| {
        let mut key = name.to_vec();
        if mode == b"40000" || mode == b"040000" {
            key.push(b'/');
        }
        key
    });
    git.tree(&entries)
}

fn commit(git: &Craft, tree: GitId, parents: &[GitId], message: &str) -> GitId {
    let mut raw = format!("tree {}\n", tree.to_hex());
    for parent in parents {
        raw += &format!("parent {}\n", parent.to_hex());
    }
    git.commit(format!("{raw}{PEOPLE}\n{message}\n").as_bytes())
}

fn on_main(git: &Craft, commit: GitId) {
    git.loose_ref("refs/heads/main", &format!("{}\n", commit.to_hex()));
}

/// `main` at a commit of a tree of `entries`, which is returned.
fn main_of(git: &Craft, entries: &[(&[u8], &[u8], GitId)]) -> GitId {
    let id = commit(git, tree(git, entries), &[], "Hostile");
    on_main(git, id);
    id
}

/// Runs `bolt import`, which must stop before it makes `.bolt`, and returns its error output.
fn refused(env: &Env) -> String {
    let err = env.refused_import();
    assert!(!env.root().join(".bolt").exists(), "{err}");
    err
}

/// Runs `bolt import`, which must stop once it has begun `.bolt`, committing nothing, and returns
/// its error output.
fn stopped(env: &Env) -> String {
    committed_nothing(env, env.refused_import())
}

/// Checks that an import that stopped after it began `.bolt` said so and committed nothing.
fn committed_nothing(env: &Env, err: String) -> String {
    assert!(err.contains("Nothing was committed."), "{err}");
    assert!(!env.root().join(".bolt/format").exists(), "{err}");
    err
}

/// `bolt import`'s work in this process, for sweeps that run it hundreds of times.
fn import_here(dir: &Path) -> Result<ImportReport, ImportError> {
    let who = Person {
        name: "Ada".into(),
        email: "ada@example.com".into(),
        time_ms: 0,
        tz_offset_min: 0,
    };
    let progress = &mut |_, _| {};
    let mut req = ImportRequest {
        who: &who,
        command: "import",
        progress,
    };
    import_git(dir, &mut req, &mut None)
}

/// Every file, link and project link of the save the import left the project on, as
/// `<kind> <path>`.
fn saved(env: &Env) -> Vec<String> {
    let (repo, _) = env.state();
    let save = current_save(&repo).unwrap();
    files_of(&repo, &save)
}

/// `saved`, for the save `save`.
fn files_of(repo: &Repository, save: &Id) -> Vec<String> {
    let root = repo.store().save(save, false).unwrap().root;
    let mut paths = Vec::new();
    for_each_change(repo.store(), None, root, |c| {
        paths.push(format!("{} {}", c.kind.as_str(), c.path));
        Ok::<_, StoreError>(())
    })
    .unwrap();
    paths
}

/// The bytes of the top-level entry `name` of the save the import left the project on.
fn content(env: &Env, name: &str) -> Vec<u8> {
    let entry = top_folder(env.root()).get(name).cloned().unwrap();
    let (repo, _) = env.state();
    let mut out = Vec::new();
    repo.store().copy_content(&entry.id, &mut out).unwrap();
    out
}

/// The line `bolt import` lists an entry it left out with, every hidden character shown.
fn left_out(path: &str, commit: &GitId) -> String {
    let path = escape_all(path);
    format!("\n  `{path}` in commit {}\n", &commit.to_hex()[..12])
}

/// The heading of the list of entries an import left out.
fn left_out_heading(count: usize) -> String {
    format!(
        "Left out {count} entries whose names Bolt never writes to disk (such as .git and .bolt), \
         with everything in them:\n"
    )
}

/// `parts` inside `.git`, joined as this system joins them.
fn in_git(parts: &[&str]) -> PathBuf {
    parts
        .iter()
        .fold(PathBuf::from(".git"), |path, part| path.join(part))
}

/// Fails unless `err` has the line `error: <before><a path ending in path><after>`. Bolt names a
/// file in the repository by its whole path, which starts with the project folder's as this
/// system gives it (macOS puts `/private` before a temporary one), so only that part is open.
#[track_caller]
fn assert_names(err: &str, before: &str, path: &Path, after: &str) {
    let before = format!("error: {before}");
    let after = format!("{}{}{after}", std::path::MAIN_SEPARATOR, path.display());
    let named = |line: &str| {
        line.len() > before.len() + after.len()
            && line.starts_with(&before)
            && line.ends_with(&after)
    };
    assert!(err.lines().any(named), "{before}…{after}\nin {err}");
}

const BROKEN: &str = "Bolt can't import this Git repository: ";

/// The hint for what isn't the plain file or real folder Git keeps in a repository.
const NOT_PLAIN: &str = "\nhint: Nothing was changed. Bolt reads only plain files and real \
                         folders in a Git repository, and never follows links there: put what a \
                         link points to in its place, or import a fresh clone.\n";

/// Makes `link` a link to the folder `target`: on Windows a junction, which needs no rights.
fn link_folder(target: &Path, link: &Path) {
    #[cfg(unix)]
    std::os::unix::fs::symlink(target, link).unwrap();
    #[cfg(windows)]
    {
        let made = std::process::Command::new("cmd")
            .args(["/D", "/C", "mklink", "/J"])
            .arg(link)
            .arg(target)
            .stdout(std::process::Stdio::null())
            .status()
            .unwrap();
        assert!(made.success(), "mklink /J {}", link.display());
    }
}

/// Makes `link` a link to the file `target`; false, with a note, where Windows lacks the right.
fn link_file(target: &Path, link: &Path) -> bool {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, link).unwrap();
        true
    }
    #[cfg(windows)]
    match std::os::windows::fs::symlink_file(target, link) {
        Ok(()) => true,
        // ERROR_PRIVILEGE_NOT_HELD: links need Developer Mode or admin rights.
        Err(e) if e.raw_os_error() == Some(1314) => {
            eprintln!("note: skipped, this account can't make links");
            false
        }
        Err(e) => panic!("can't make a link: {e}"),
    }
}

/// The characters HFS+ ignores in names, Git's `next_hfs_char` list.
fn hfs_ignorable() -> impl Iterator<Item = char> {
    ('\u{200c}'..='\u{200f}')
        .chain('\u{202a}'..='\u{202e}')
        .chain('\u{206a}'..='\u{206f}')
        .chain(['\u{feff}'])
}

// CVE-2014-9390, CVE-2019-1349, CVE-2019-1352
#[test]
fn git_spellings_are_skipped_with_everything_in_them() {
    let (env, git) = repo();
    let hook = git.object(Blob, b"#!/bin/sh\necho owned\n");
    let hooks = git.tree(&[(b"100755", b"post-checkout", hook)]);
    let planted = git.tree(&[(b"40000", b"hooks", hooks)]);
    let readme = git.object(Blob, b"readme\n");
    let mut names: Vec<String> = [
        ".git",
        ".GIT",
        "git~1",
        ".git.",
        ".git ",
        ".git::$INDEX_ALLOCATION",
    ]
    .map(String::from)
    .to_vec();
    names.extend(hfs_ignorable().map(|c| format!(".g{c}it")));
    let mut entries: Vec<(&[u8], &[u8], GitId)> = names
        .iter()
        .map(|name| (&b"40000"[..], name.as_bytes(), planted))
        .collect();
    entries.push((b"100644", b"README", readme));
    let sub = tree(&git, &entries);
    entries.push((b"40000", b"sub", sub));
    let id = main_of(&git, &entries);
    let (out, _) = env.import();
    assert!(out.contains(&left_out_heading(names.len() * 2)), "{out}");
    for name in &names {
        assert!(out.contains(&left_out(name, &id)), "{name:?}: {out}");
        let nested = format!("sub/{name}");
        assert!(out.contains(&left_out(&nested, &id)), "{nested:?}: {out}");
    }
    assert_eq!(saved(&env), ["file README", "file sub/README"]);
    // Nothing went into Git's own folder.
    assert!(!env.root().join(".git/hooks").exists());
}

/// A backslash ends the `.git` part on every system, as in Git (decision 035).
#[test]
fn path_parts_are_skipped() {
    let (env, git) = repo();
    let file = git.object(Blob, b"x\n");
    let up = git.tree(&[(b"100644", b"escaped.txt", file)]);
    let id = main_of(
        &git,
        &[
            (b"40000", b"..", up),
            (b"100644", b".", file),
            (b"100644", b"", file),
            (b"100644", b"a/b", file),
            (b"100755", b".git\\hooks\\x", file),
            (b"100644", b"README", file),
        ],
    );
    let (out, _) = env.import();
    assert!(out.contains(&left_out_heading(5)), "{out}");
    for path in ["..", ".", "a/b", ".git\\hooks\\x"] {
        assert!(out.contains(&left_out(path, &id)), "{path}: {out}");
    }
    let empty = format!("\n  an empty name in commit {}\n", &id.to_hex()[..12]);
    assert!(out.contains(&empty), "{out}");
    assert_eq!(saved(&env), ["file README"]);
}

/// Decision 019. The nested ones are the nested `.bolt` case too: Git checks them out, and a
/// command run inside one would take it for a project.
#[test]
fn bolt_spellings_are_skipped_and_warned_about() {
    let spellings = [".bolt", ".BOLT", "bolt~1"];
    let (env, git) = repo();
    let file = git.object(Blob, b"x\n");
    let project = git.tree(&[(b"100644", b"format", file)]);
    let mut entries: Vec<(&[u8], &[u8], GitId)> = spellings
        .iter()
        .map(|name| (&b"40000"[..], name.as_bytes(), project))
        .collect();
    entries.push((b"100644", b"README", file));
    let sub = tree(&git, &entries);
    let id = main_of(
        &git,
        &[(b"40000", b"sub", sub), (b"100644", b"README", file)],
    );
    let (out, err) = env.import();
    assert!(out.contains(&left_out_heading(3)), "{out}");
    for name in spellings {
        let path = format!("sub/{name}");
        assert!(out.contains(&left_out(&path, &id)), "{out}");
        let warning = format!(
            "warning: Git checked out a folder named .bolt at `{path}`. Bolt left it out of \
             history, but commands run inside it would use it as a Bolt project; delete it \
             unless you made it.\n"
        );
        assert!(err.contains(&warning), "{err}");
    }
    assert_eq!(saved(&env), ["file README", "file sub/README"]);

    // At the top, Git checks it out where Bolt keeps the project.
    for name in spellings {
        let (env, git) = repo();
        let project = git.tree(&[(b"100644", b"format", git.object(Blob, b"x\n"))]);
        main_of(&git, &[(b"40000", name.as_bytes(), project)]);
        let err = refused(&env);
        let stop = format!(
            "error: Git tracks `{name}` at the top of this project, and Bolt can't use it as its \
             own .bolt folder.\n"
        );
        assert!(err.contains(&stop), "{err}");
    }
}

#[test]
fn a_name_longer_than_255_bytes_stops_the_import() {
    let (env, git) = repo();
    let long = "x".repeat(256);
    let id = main_of(
        &git,
        &[(b"100644", long.as_bytes(), git.object(Blob, b"x\n"))],
    );
    let err = stopped(&env);
    let stop = format!(
        "error: Git reads this object, but Bolt can't import it as it is: commit `{}` has a name \
         longer than 255 bytes, more than Bolt allows: `{}...`.\n",
        id.to_hex(),
        "x".repeat(255)
    );
    assert!(err.contains(&stop), "{err}");
}

#[test]
fn names_that_arent_utf8_in_two_commits_stop_listing_both() {
    let (env, git) = repo();
    let file = git.object(Blob, b"x\n");
    let first = commit(&git, git.tree(&[(b"100644", b"caf\xe9", file)]), &[], "One");
    let second = git.tree(&[(b"100644", b"na\xefve", file)]);
    let second = commit(&git, second, &[first], "Two");
    on_main(&git, second);
    let err = stopped(&env);
    let stop = format!(
        "error: 2 names in the Git history aren't valid UTF-8, and Bolt keeps only UTF-8 names:\n  \
         `caf\u{fffd}` in commit `{}`\n  `na\u{fffd}ve` in commit `{}`\n",
        first.to_hex(),
        second.to_hex()
    );
    assert!(err.contains(&stop), "{err}");
}

#[test]
fn broken_trees_stop_naming_the_commit_and_folder() {
    type Sub = fn(&Craft) -> GitId;
    let cases: [(Sub, &str); 4] = [
        (
            |git| {
                let x = git.object(Blob, b"x\n");
                git.tree(&[(b"100644", b"x", x), (b"100644", b"x", x)])
            },
            "Bolt can't import this Git repository: a tree has two entries named `x`",
        ),
        // A file and a folder of one name needn't be neighbours: `a`, `a.b`, then `a/`.
        (
            |git| {
                let x = git.object(Blob, b"x\n");
                let a = git.tree(&[(b"100644", b"x", x)]);
                git.tree(&[
                    (b"100644", b"a", x),
                    (b"100644", b"a.b", x),
                    (b"40000", b"a", a),
                ])
            },
            "Bolt can't import this Git repository: a tree has two entries named `a`",
        ),
        (
            |git| {
                let x = git.object(Blob, b"x\n");
                git.tree(&[(b"100644", b"b", x), (b"100644", b"a", x)])
            },
            "Bolt can't import this Git repository: a tree's entries `b` and `a` are out of Git's \
             order",
        ),
        (
            |git| git.tree(&[(b"160001", b"x", git.object(Blob, b"x\n"))]),
            "Git reads this object, but Bolt can't import it as it is: a tree entry `x` has the \
             mode `160001`, which Git doesn't write and Bolt doesn't take",
        ),
    ];
    for (make, why) in cases {
        let (env, git) = repo();
        let sub = make(&git);
        let id = main_of(&git, &[(b"40000", b"sub", sub)]);
        let err = stopped(&env);
        let (sub, id) = (sub.to_hex(), id.to_hex());
        let stop = format!("error: {why}, in the folder `sub` (tree `{sub}`) of commit `{id}`.\n");
        assert!(err.contains(&stop), "{err}");
    }

    // An entry naming an object that isn't there.
    let (env, git) = repo();
    let missing = blob_id(b"never written\n");
    let sub = git.tree(&[(b"100644", b"x", missing)]);
    let id = main_of(&git, &[(b"40000", b"sub", sub)]);
    let err = stopped(&env);
    let stop = format!(
        "error: Bolt can't import this Git repository: missing object `{}`, in the file `sub/x` of \
         commit `{}`.\n",
        missing.to_hex(),
        id.to_hex()
    );
    assert!(err.contains(&stop), "{err}");
}

/// One mode to a commit, so each is counted on its own.
#[test]
fn modes_git_no_longer_writes_are_imported_as_inexact() {
    let (env, git) = repo();
    let file = git.object(Blob, b"x\n");
    let inner = git.tree(&[(b"100644", b"inner.txt", file)]);
    let padded = commit(&git, tree(&git, &[(b"040000", b"old", inner)]), &[], "One");
    let shared = tree(&git, &[(b"100664", b"shared.txt", file)]);
    on_main(&git, commit(&git, shared, &[padded], "Two"));
    let (out, _) = env.import();
    assert!(
        out.contains(
            "2 commits can't be recreated exactly by a later export: they hold folders in a form \
             Git no longer writes, or entries Bolt left out.\n"
        ),
        "{out}"
    );
    assert_eq!(saved(&env), ["file shared.txt"]);
    let (repo, state) = env.state();
    let two = repo.store().save(&state.lines["main"], false).unwrap();
    assert_eq!(files_of(&repo, &two.parents[0]), ["file old/inner.txt"]);
}

/// A pack of `levels` trees, each holding the one below as `a`; returns the top one.
fn deep_tree(git: &Craft, levels: usize) -> GitId {
    let mut below = object_id(Tree, b"", HashKind::Sha1).unwrap();
    let mut entries = vec![PackEntry::Whole(Tree, Vec::new())];
    for _ in 0..levels {
        let bytes = [b"40000 a\0".as_slice(), below.as_bytes()].concat();
        below = object_id(Tree, &bytes, HashKind::Sha1).unwrap();
        entries.push(PackEntry::Whole(Tree, bytes));
    }
    git.pack(&entries);
    below
}

/// `stopped`, for an import that must stop within 30 seconds: it is killed then, so a limit gone
/// fails the test instead of hanging it. A stack overflow would end it with another exit code.
fn stopped_quickly(env: &Env) -> String {
    let limit = Duration::from_secs(30);
    let started = Instant::now();
    let out = env.bolt().arg("import").timeout(limit).output().unwrap();
    let took = started.elapsed();
    assert!(took < limit, "{took:?}");
    assert_eq!(out.status.code(), Some(1), "{}", text(&out.stdout));
    committed_nothing(env, text(&out.stderr))
}

// CVE-2016-2324
#[test]
fn deep_and_wide_trees_stop_at_bolts_limits_quickly() {
    let (env, git) = repo();
    let id = commit(&git, deep_tree(&git, 100_000), &[], "Deep");
    on_main(&git, id);
    let err = stopped_quickly(&env);
    let stop = format!(
        "error: Git reads this object, but Bolt can't import it as it is: commit `{}` can't \
         become one save, because its folders nest more than 4096 levels deep.\n",
        id.to_hex()
    );
    assert!(err.contains(&stop), "{err}");

    // Four levels of 1,000 entries, each the same as its neighbours: 10^12 paths.
    let (env, git) = repo();
    let names: Vec<String> = (0..1000).map(|i| format!("{i:04}")).collect();
    let level = |mode: &[u8], id: GitId| {
        let entries: Vec<(&[u8], &[u8], GitId)> =
            names.iter().map(|n| (mode, n.as_bytes(), id)).collect();
        git.tree(&entries)
    };
    let mut wide = level(b"100644", git.object(Blob, b"x\n"));
    for _ in 0..3 {
        wide = level(b"40000", wide);
    }
    let id = commit(&git, wide, &[], "Wide");
    on_main(&git, id);
    let err = stopped_quickly(&env);
    let stop = format!(
        "error: Git reads this object, but Bolt can't import it as it is: commit `{}` can't \
         become one save, because it has more than 10000000 files and folders in total.\n",
        id.to_hex()
    );
    assert!(err.contains(&stop), "{err}");
}

// go-git 2026: parsers that disagree on what a commit says
#[test]
fn ambiguous_commits_stop_the_import() {
    // Never read: each commit is refused first.
    let tree = blob_id(b"any tree").to_hex();
    let parents: String = (1..=1025u32)
        .map(|n| format!("parent {n:040x}\n"))
        .collect();
    let broken = "Bolt can't import this Git repository";
    let unimportable = "Git reads this object, but Bolt can't import it as it is";
    for (raw, why) in [
        (
            format!("tree {tree}\n{PEOPLE}tree {tree}\n\nm\n"),
            format!("{unimportable}: a commit's header has the key `tree` out of place"),
        ),
        (
            format!("tree {tree}\n{PEOPLE}author Eve <eve@example.com> 0 +0000\n\nm\n"),
            format!("{unimportable}: a commit's header has the key `author` out of place"),
        ),
        (
            format!("parent {tree}\ntree {tree}\n{PEOPLE}\nm\n"),
            format!("{broken}: a commit doesn't start with a `tree` line"),
        ),
        (
            format!("tree {tree}\n{PEOPLE}gpgsig -----BEGIN PGP SIGNATURE-----\n iQEz"),
            format!("{broken}: a commit ends inside its header"),
        ),
        (
            format!("tree {tree}\n{}\nm\n", PEOPLE.replacen("Ada ", "Ada\0 ", 1)),
            format!("{broken}: a commit's header holds a NUL byte"),
        ),
        (
            format!("tree {tree}\n{parents}{PEOPLE}\nm\n"),
            format!("{unimportable}: a commit has 1025 parents, more than the 1024 Bolt keeps"),
        ),
    ] {
        let (env, git) = repo();
        let id = git.commit(raw.as_bytes());
        on_main(&git, id);
        let err = refused(&env);
        let stop = format!("error: {why}, in commit `{}`.\n", id.to_hex());
        assert!(err.contains(&stop), "{err}");
    }
}

// The unusual commits a save keeps exactly (`-0000`, an odd author line, `encoding`, `gpgsig`,
// `mergetag`, an unknown header, a `change-id`) are import_vs_git.rs's
// `unusual_commits_match_git`, which checks each rebuilds its commit's bytes against Git.

/// What a tag object of `target`, a `kind`, holds before its message.
fn tag_head(target: &GitId, kind: &str) -> String {
    format!(
        "object {}\ntype {kind}\ntag t\ntagger Ada <ada@example.com> 1700000000 +0000\n\n",
        target.to_hex()
    )
}

fn tag_object(git: &Craft, target: &GitId, kind: &str, message: &[u8]) -> GitId {
    git.object(Tag, &[tag_head(target, kind).as_bytes(), message].concat())
}

#[test]
fn tags_bolt_cant_keep_are_left_out_or_stop_the_import() {
    // A tag of a folder is left out and listed.
    let (env, git) = repo();
    let file = git.object(Blob, b"x\n");
    let folder = tree(&git, &[(b"100644", b"x", file)]);
    on_main(&git, commit(&git, folder, &[], "One"));
    let tag = tag_object(&git, &folder, "tree", b"A folder\n");
    git.loose_ref("refs/tags/folder", &format!("{}\n", tag.to_hex()));
    let (out, _) = env.import();
    assert!(
        out.contains(
            "Didn't import 1 tag: folder, as it tags a folder (a Git tree), not a commit\n"
        ),
        "{out}"
    );
    assert!(out.contains(", 0 tags."), "{out}");

    // Chains of tag objects, each tagging the one before, and the size of the first, which tags
    // the commit.
    for (objects, size, why) in [
        (
            17,
            200,
            "tag `t` reaches its commit through more than 16 tag objects, more than Bolt keeps",
        ),
        (
            1,
            1_048_577,
            "tag `t` has a tag object of 1048577 bytes (`{tag}`), more than the 1 MiB Bolt keeps \
             of one",
        ),
    ] {
        let (env, git) = repo();
        let id = one_file(&git);
        let message = vec![b'x'; size - tag_head(&id, "commit").len()];
        let mut tag = tag_object(&git, &id, "commit", &message);
        for _ in 1..objects {
            tag = tag_object(&git, &tag, "tag", b"Again\n");
        }
        git.loose_ref("refs/tags/t", &format!("{}\n", tag.to_hex()));
        let err = refused(&env);
        let why = why.replace("{tag}", &tag.to_hex());
        let stop =
            format!("error: Git reads this object, but Bolt can't import it as it is: {why}.\n");
        assert!(err.contains(&stop), "{err}");
    }
}

/// `main` at a commit of one small file; returns the commit.
fn one_file(git: &Craft) -> GitId {
    main_of(git, &[(b"100644", b"x", git.object(Blob, b"x\n"))])
}

// go-git CVE-2026-71557, gitoxide CVE-2024-35197: nothing outside `refs/` is opened.
#[test]
fn refs_bolt_cant_read_stop_the_import() {
    for (packed, why) in [
        (
            "{id} refs/heads/../../config\n",
            "the ref `refs/heads/../../config` in .git/packed-refs has a name Git doesn't allow",
        ),
        (
            "# pack-refs with: peeled fully-peeled sorted \n{id} refs/heads/b\nnot a ref\n",
            "line 3 of .git/packed-refs isn't a ref",
        ),
    ] {
        let (env, git) = repo();
        let id = one_file(&git);
        git.packed_refs(&packed.replace("{id}", &id.to_hex()));
        let err = refused(&env);
        let stop = format!("error: Bolt can't import this Git repository: {why}.\n");
        assert!(err.contains(&stop), "{err}");
    }

    // Loose refs that are links to a folder and to a file outside, which hold a good ref.
    let linked = in_git(&["refs", "heads", "linked"]);
    let why = " is not a regular file (it is a link, folder or special file).";
    let outside = tempfile::tempdir().unwrap();
    let (env, git) = repo();
    let id = one_file(&git);
    fs::write(outside.path().join("main"), format!("{}\n", id.to_hex())).unwrap();
    link_folder(outside.path(), &env.root().join(&linked));
    let err = refused(&env);
    assert_names(&err, BROKEN, &linked, why);
    assert!(err.contains(NOT_PLAIN), "{err}");
    let (env, git) = repo();
    one_file(&git);
    if link_file(&outside.path().join("main"), &env.root().join(&linked)) {
        let err = refused(&env);
        assert_names(&err, BROKEN, &linked, why);
        assert!(err.contains(NOT_PLAIN), "{err}");
    }
}

/// Windows opens a device for a name like `CON`, so the walk refuses it without opening it.
/// A `\\?\` path makes such a file without Windows reading the name.
#[cfg(windows)]
#[test]
fn a_ref_named_like_a_windows_device_is_refused_unopened() {
    let (env, git) = repo();
    let id = one_file(&git);
    let heads = env.root().join(".git").join("refs").join("heads");
    let con = std::path::PathBuf::from(format!(r"\\?\{}", heads.display())).join("CON");
    fs::write(&con, format!("{}\n", id.to_hex())).unwrap();
    let err = refused(&env);
    let stop = "error: Bolt can't import this Git repository: `refs/heads/CON` has a name Windows \
                treats specially, so Bolt didn't open it.\n";
    assert!(err.contains(stop), "{err}");
    fs::remove_file(&con).unwrap();
}

#[test]
fn line_names_bolt_refuses_are_renamed_and_too_many_servers_stop() {
    let (env, git) = repo();
    let id = one_file(&git).to_hex();
    git.packed_refs(&format!(
        "{id} refs/heads/Feature\n{id} refs/heads/feature\n{id} refs/heads/feature+x\n"
    ));
    let (out, _) = env.import();
    let renamed = "Renamed 2 names that Bolt's names don't allow:\n  feature → feature-2\n  \
                   feature+x → feature-x\n";
    assert!(out.contains(renamed), "{out}");
    let (_, state) = env.state();
    let lines: Vec<&str> = state.lines.keys().map(String::as_str).collect();
    assert_eq!(lines, ["Feature", "feature-2", "feature-x", "main"]);

    let (env, git) = repo();
    let id = one_file(&git).to_hex();
    let servers: String = (0..65)
        .map(|n| format!("{id} refs/remotes/s{n:02}/main\n"))
        .collect();
    git.packed_refs(&servers);
    let err = refused(&env);
    let stop = "error: Bolt can't import this Git repository: it has branches from more than 64 \
                servers, and Bolt holds at most 64.\n";
    assert!(err.contains(stop), "{err}");
}

#[test]
fn a_head_bolt_cant_follow_stops_the_import() {
    let (env, git) = repo();
    one_file(&git);
    git.head("ref: refs/heads/new\n");
    let err = refused(&env);
    let stop = "error: Git's current branch `new` has no commits yet; switch to a branch with \
                commits, or commit first.\n";
    assert!(err.contains(stop), "{err}");

    let (env, git) = repo();
    let missing = GitId::from_hex(HashKind::Sha1, &"c0".repeat(20)).unwrap();
    on_main(&git, missing);
    let err = refused(&env);
    let hex = missing.to_hex();
    let stop = format!(
        "error: Bolt can't import this Git repository: missing object `{hex}`, in commit `{hex}`.\n"
    );
    assert!(err.contains(&stop), "{err}");

    let (env, git) = repo();
    one_file(&git);
    git.head("ref: refs/remotes/x\n");
    let err = refused(&env);
    let stop = "error: Bolt can't import this Git repository: Git's HEAD points at \
                `refs/remotes/x`, which isn't a branch.\n";
    assert!(err.contains(stop), "{err}");
}

const GIT_FILE: &str = "This .git points to a repository kept elsewhere (a linked worktree or a \
                        submodule), which Bolt doesn't follow.\nhint: Run bolt import where the \
                        repository's own .git folder is, or in a fresh clone of it.\n";

// CVE-2022-39253, CVE-2024-32021
#[test]
fn repository_shapes_bolt_doesnt_follow_are_refused_saying_what_to_do() {
    type Shape = fn(&Craft);
    let cases: [(Shape, &str); 7] = [
        (
            |git| git.file("objects/info/alternates", b"/elsewhere/objects\n"),
            "This Git repository borrows objects from another one (.git/objects/info/alternates), \
             which Bolt doesn't follow.\nhint: Copy those objects in with `git repack -a -d`, \
             delete .git/objects/info/alternates, then run bolt import again.\n",
        ),
        (|git| git.file("commondir", b"../..\n"), GIT_FILE),
        (
            |git| git.file("shallow", format!("{}\n", "ab".repeat(20)).as_bytes()),
            "This Git repository is a shallow clone: its older history was never downloaded, and \
             Bolt needs all of it.\nhint: Fetch the rest with `git fetch --unshallow`, then run \
             bolt import again.\n",
        ),
        (
            |git| git.config("[core]\n\trepositoryformatversion = 0\n\tsparseCheckout = true\n"),
            "Git leaves some files out of this folder on purpose (a sparse checkout), so Bolt \
             would record them as deleted.\nhint: Turn it off with `git sparse-checkout disable`, \
             then run bolt import again.\n",
        ),
        (
            |git| git.file("reftable/tables.list", b""),
            "This Git repository keeps its refs in the reftable format, which Bolt can't read \
             yet.\nhint: Convert it with `git refs migrate --ref-storage-format=files`, then run \
             bolt import again.\n",
        ),
        (
            |git| {
                git.config(
                    "[core]\n\trepositoryformatversion = 1\n[extensions]\n\tpartialClone = origin\n",
                )
            },
            "This Git repository is a partial clone: some of its history was never downloaded, \
             and Bolt needs all of it.\nhint: Clone the repository again without `--filter`, and \
             run bolt import in the new clone.\n",
        ),
        (
            |git| git.config("[core]\n\trepositoryformatversion = 1\n[extensions]\n\tx = y\n"),
            "This Git repository uses extensions.x, a Git feature Bolt doesn't know.\nhint: \
             Import a fresh clone made with `git clone`, which usually doesn't use it, or update \
             Bolt.\n",
        ),
    ];
    for (shape, refusal) in cases {
        let (env, git) = repo();
        one_file(&git);
        shape(&git);
        let err = refused(&env);
        assert!(err.contains(&format!("error: {refusal}")), "{err}");
    }
    // A `.git` file: a linked worktree's, or a submodule's checkout.
    let env = Env::empty();
    fs::write(env.root().join(".git"), "gitdir: ../elsewhere/.git\n").unwrap();
    let err = refused(&env);
    assert!(err.contains(&format!("error: {GIT_FILE}")), "{err}");
}

/// Each link leads to a good repository's own part, so following it would have imported.
#[test]
fn links_in_the_repositorys_shape_are_refused_unfollowed() {
    let not_a_folder = " is not a real folder (it is a link, junction or file).";
    let elsewhere = tempfile::tempdir().unwrap();
    one_file(&Craft::new(elsewhere.path(), HashKind::Sha1));
    let env = Env::empty();
    link_folder(&elsewhere.path().join(".git"), &env.root().join(".git"));
    let err = refused(&env);
    assert_names(&err, BROKEN, Path::new(".git"), not_a_folder);
    assert!(err.contains(NOT_PLAIN), "{err}");

    let outside = tempfile::tempdir().unwrap();
    let (env, git) = repo();
    one_file(&git);
    let objects = env.root().join(in_git(&["objects"]));
    fs::rename(&objects, outside.path().join("objects")).unwrap();
    link_folder(&outside.path().join("objects"), &objects);
    let err = refused(&env);
    assert_names(&err, BROKEN, &in_git(&["objects"]), not_a_folder);
    assert!(err.contains(NOT_PLAIN), "{err}");

    // A pack holding the whole history.
    let (env, git) = repo();
    let tree = [b"100644 x\0".as_slice(), blob_id(b"x\n").as_bytes()].concat();
    let tree_id = object_id(Tree, &tree, HashKind::Sha1).unwrap();
    let raw = format!("tree {}\n{PEOPLE}\nPacked\n", tree_id.to_hex()).into_bytes();
    let (pack, _) = git.pack(&[
        PackEntry::Whole(Blob, b"x\n".to_vec()),
        PackEntry::Whole(Tree, tree),
        PackEntry::Whole(Commit, raw.clone()),
    ]);
    on_main(&git, object_id(Commit, &raw, HashKind::Sha1).unwrap());
    let moved = outside.path().join("moved.pack");
    fs::rename(&pack, &moved).unwrap();
    if link_file(&moved, &pack) {
        let err = refused(&env);
        let name = pack.file_name().unwrap().to_str().unwrap();
        let why = " is not a regular file (it is a link, folder or special file).";
        assert_names(&err, BROKEN, &in_git(&["objects", "pack", name]), why);
        assert!(err.contains(NOT_PLAIN), "{err}");
    }
}

// CVE-2024-32004, CVE-2024-32465: a `.git` another account owns. Its owner can only be stubbed
// at `Repo::open_with`, so git/mod.rs's `a_dot_git_owned_by_another_account_is_refused` holds
// this case: refused unless safe-folders lists the work tree.

// libgit2 CVE-2026-53585
#[test]
fn loose_objects_that_lie_about_their_size_are_refused_before_allocating() {
    // Where Git keeps the loose object `id`, which the messages name.
    let loose = |id: &GitId| {
        let hex = id.to_hex();
        in_git(&["objects", &hex[..2], &hex[2..]])
    };
    let x = blob_id(b"x\n");
    // 8 MiB of zeros, a few kilobytes compressed, in an object that says it holds 10 bytes.
    let bomb = zlib(&[b"blob 10\0".as_slice(), &vec![0; 8 << 20]].concat());
    let trailing = [zlib(b"blob 5\0hello"), b"garbage".to_vec()].concat();
    for (stored, why) in [
        (
            zlib(b"blob 99999999999\0x"),
            "ended after 1 of the 99999999999 bytes it says it holds",
        ),
        (bomb, "holds more than the 10 bytes it says"),
        (trailing, "has bytes after its compressed data"),
    ] {
        let (env, git) = repo();
        git.raw_loose(&x.to_hex(), &stored);
        let id = main_of(&git, &[(b"100644", b"x", x)]);
        let err = stopped(&env);
        let after = format!(" {why}, in the file `x` of commit `{}`.", id.to_hex());
        assert_names(&err, BROKEN, &loose(&x), &after);
    }
    // A folder is read whole, so its size is refused before memory is asked for.
    let (env, git) = repo();
    let top = blob_id(b"a folder\n");
    git.raw_loose(&top.to_hex(), &zlib(b"tree 99999999999\0"));
    on_main(&git, commit(&git, top, &[], "Huge"));
    let err = refused(&env);
    let after = format!(
        " says it holds 99999999999 bytes, more than the 536870912 Bolt reads into memory, in the \
         top folder (tree `{}`) of `HEAD`.",
        top.to_hex()
    );
    assert_names(&err, BROKEN, &loose(&top), &after);
}

// gitoxide GHSA-x494, GHSA-x862, GHSA-mpf5, GHSA-633h; dulwich CVE-2026-114095; libgit2
// CVE-2018-10887 and CVE-2018-10888
#[test]
fn every_cut_of_a_pack_or_its_index_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let git = Craft::new(dir.path(), HashKind::Sha1);
    let base = b"hello, hello, hello\n".to_vec();
    let edited = b"hello, hello, goodbye\n".to_vec();
    let tree = [
        b"100644 a\0".as_slice(),
        blob_id(&base).as_bytes(),
        b"100644 b\0",
        blob_id(&edited).as_bytes(),
    ]
    .concat();
    let tree_id = object_id(Tree, &tree, HashKind::Sha1).unwrap();
    let raw = format!("tree {}\n{PEOPLE}\nPacked\n", tree_id.to_hex()).into_bytes();
    let (pack, idx) = git.pack(&[
        PackEntry::Whole(Blob, base.clone()),
        PackEntry::OfsDelta {
            base: 0,
            delta: delta(&base, &edited),
        },
        PackEntry::Whole(Tree, tree),
        PackEntry::Whole(Commit, raw.clone()),
    ]);
    on_main(&git, object_id(Commit, &raw, HashKind::Sha1).unwrap());
    // In this process: the sweep runs the import more than a thousand times.
    for path in [&pack, &idx] {
        let good = fs::read(path).unwrap();
        for len in 0..good.len() {
            fs::write(path, &good[..len]).unwrap();
            let result = import_here(dir.path());
            let what = format!("{} cut to {len} bytes", path.display());
            assert!(
                matches!(result, Err(ImportError::Git(GitError::Broken { .. }))),
                "{what}: {result:?}"
            );
            assert!(!dir.path().join(".bolt").exists(), "{what}");
        }
        fs::write(path, &good).unwrap();
    }
    // Whole, they import.
    assert_eq!(import_here(dir.path()).unwrap().commits, 1);
}

/// A repository whose one file, `x`, is the object `file`, packed with `entries` while its commit
/// and tree are loose; `damage` changes the pack's bytes first. Returns the commit, and the pack
/// as Bolt names it inside the repository.
fn packed(
    entries: &[PackEntry],
    file: GitId,
    damage: impl FnOnce(&mut [u8]),
) -> (Env, GitId, PathBuf) {
    let (env, git) = repo();
    let (pack, _) = git.pack(entries);
    let mut bytes = fs::read(&pack).unwrap();
    damage(&mut bytes);
    fs::write(&pack, bytes).unwrap();
    let id = main_of(&git, &[(b"100644", b"x", file)]);
    let name = pack.file_name().unwrap().to_str().unwrap();
    (env, id, in_git(&["objects", "pack", name]))
}

/// Where the index of `pack` (inside `env`'s repository) puts `id`: Git's version 2 layout, with
/// every offset under 2 GiB.
fn offset_in(env: &Env, pack: &Path, id: &GitId) -> u32 {
    let idx = fs::read(env.root().join(pack).with_extension("idx")).unwrap();
    let count = u32::from_be_bytes(idx[1028..1032].try_into().unwrap()) as usize;
    let names = &idx[1032..1032 + 20 * count];
    let n = names
        .chunks(20)
        .position(|name| name == id.as_bytes())
        .unwrap();
    let at = 1032 + 24 * count + 4 * n;
    u32::from_be_bytes(idx[at..at + 4].try_into().unwrap())
}

#[test]
fn bad_packs_and_deltas_are_errors_never_a_crash() {
    use PackEntry::{OfsDelta, Raw, RefDelta, Whole};
    let x = || Whole(Blob, b"x".to_vec());
    type Damage = fn(&mut [u8]);
    let cases: [(Damage, &str); 2] = [
        (
            |pack| pack[8..12].copy_from_slice(&u32::MAX.to_be_bytes()),
            "holds 4294967295 objects, but its index lists 1",
        ),
        (
            |pack| pack[4..8].copy_from_slice(&4u32.to_be_bytes()),
            "is a version 4 pack, which Git doesn't write",
        ),
    ];
    for (damage, why) in cases {
        let (env, id, pack) = packed(&[x()], blob_id(b"x"), damage);
        let err = refused(&env);
        // Found when the import reads its first object, the commit.
        let after = format!(" {why}, in commit `{}`.", id.to_hex());
        assert_names(&err, BROKEN, &pack, &after);
    }
    // Git reads version 3 as it reads version 2, and so does Bolt.
    let (env, _, _) = packed(&[x()], blob_id(b"x"), |pack| pack[7] = 3);
    env.import();
    assert_eq!(saved(&env), ["file x"]);

    let on_itself = delta(b"x", b"y");
    // An `OFS_DELTA` 100 bytes back from the first entry, at offset 12.
    let before_start = [entry_header(6, 3), vec![100]].concat();
    let (to_a, to_b) = (delta(b"x", b"a"), delta(b"x", b"b"));
    let versions: Vec<Vec<u8>> = (0..=4097).map(|i| format!("v{i}").into_bytes()).collect();
    let mut chain = vec![Whole(Blob, versions[0].clone())];
    for (i, pair) in versions.windows(2).enumerate() {
        let delta = delta(&pair[0], &pair[1]);
        chain.push(OfsDelta { base: i, delta });
    }
    // A delta on 1 byte that says it builds 2^62, in Git's size encoding.
    let huge = [
        &[1u8, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x40][..],
        b"\x01x",
    ]
    .concat();
    let past_base = vec![10, 5, 0x91, 8, 5];
    // Each names the entry of `x` by where it is in the pack: `{at}`.
    let (entry, object, built) = (
        "the entry at offset {at} in",
        "the object at offset {at} in",
        "the delta at offset {at} in",
    );
    for (entries, file, what, why) in [
        (
            vec![
                x(),
                OfsDelta {
                    base: 1,
                    delta: on_itself.clone(),
                },
            ],
            blob_id(&on_itself),
            entry,
            "names itself as its base",
        ),
        (
            vec![Raw(before_start.clone())],
            blob_id(&before_start),
            entry,
            "names a base 100 bytes back, before the pack's first entry",
        ),
        (
            vec![
                RefDelta {
                    base: blob_id(&to_b),
                    delta: to_a.clone(),
                },
                RefDelta {
                    base: blob_id(&to_a),
                    delta: to_b,
                },
            ],
            blob_id(&to_a),
            "the deltas that build the object at offset {at} in",
            "go round in a loop",
        ),
        (
            chain,
            blob_id(&versions[4097]),
            object,
            "is more than 4096 deltas deep",
        ),
        (
            vec![
                x(),
                OfsDelta {
                    base: 0,
                    delta: huge.clone(),
                },
            ],
            blob_id(&huge),
            built,
            "would build 4611686018427387904 bytes, more than the 536870912 Bolt rebuilds in \
             memory",
        ),
        (
            vec![
                Whole(Blob, b"0123456789".to_vec()),
                OfsDelta {
                    base: 0,
                    delta: past_base.clone(),
                },
            ],
            blob_id(&past_base),
            built,
            "copies from past the end of its base",
        ),
    ] {
        let (env, id, pack) = packed(&entries, file, |_| {});
        let err = stopped(&env);
        let at = offset_in(&env, &pack, &file).to_string();
        let before = format!("{BROKEN}{} ", what.replace("{at}", &at));
        let after = format!(" {why}, in the file `x` of commit `{}`.", id.to_hex());
        assert_names(&err, &before, &pack, &after);
    }
}

// gitoxide CVE-2025-31130. A SHA-1 collision can't be built as a Git object, since Git's header
// shifts the attack's blocks: git/id.rs's `a_sha1_collision_attack_is_refused` feeds the hasher
// SHAttered's own (tests/vectors/shattered-prefix.bin).
#[test]
fn an_object_whose_id_doesnt_match_is_refused() {
    let (env, git) = repo();
    let claimed = blob_id(b"x\n");
    git.raw_loose(&claimed.to_hex(), &zlib(b"blob 6\0hello\n"));
    let id = main_of(&git, &[(b"100644", b"x", claimed)]);
    let err = stopped(&env);
    let stop = format!(
        "error: Bolt can't import this Git repository: object `{}` doesn't match its ID: what Git \
         stored under it hashes to `{}`, in the file `x` of commit `{}`.\n",
        claimed.to_hex(),
        blob_id(b"hello\n").to_hex(),
        id.to_hex()
    );
    assert!(err.contains(&stop), "{err}");
}

/// A commit-graph file as Git lays one out, whose every commit names as its parent a position
/// past the end. Its checksum is zeros.
fn commit_graph(commits: &[GitId]) -> Vec<u8> {
    let mut ids = commits.to_vec();
    ids.sort();
    let n = ids.len() as u64;
    let fanout = 8 + 4 * 12;
    let (names, data) = (fanout + 1024, fanout + 1024 + 20 * n);
    let mut out = b"CGPH\x01\x01\x03\x00".to_vec();
    for (chunk, at) in [
        (*b"OIDF", fanout),
        (*b"OIDL", names),
        (*b"CDAT", data),
        ([0; 4], data + 36 * n),
    ] {
        out.extend(chunk);
        out.extend(at.to_be_bytes());
    }
    for byte in 0..=255u8 {
        let below = ids.iter().filter(|id| id.as_bytes()[0] <= byte).count() as u32;
        out.extend(below.to_be_bytes());
    }
    for id in &ids {
        out.extend(id.as_bytes());
    }
    for _ in &ids {
        // The root tree, both parents' positions, then the generation and time.
        out.extend([0; 20]);
        out.extend((n as u32 + 1).to_be_bytes());
        out.extend(0x7000_0000u32.to_be_bytes());
        out.extend([0; 8]);
    }
    out.extend([0; 20]);
    out
}

// gitoxide GHSA-989c: Bolt reads the commits themselves, never Git's caches of them.
#[test]
fn speed_up_files_are_ignored() {
    let (env, git) = repo();
    let folder = tree(&git, &[(b"100644", b"x", git.object(Blob, b"x\n"))]);
    let first = commit(&git, folder, &[], "One");
    let second = commit(&git, folder, &[first], "Two");
    on_main(&git, second);
    git.file("objects/info/commit-graph", &commit_graph(&[first, second]));
    git.file(
        "objects/pack/multi-pack-index",
        b"MIDX\x01\x01\x00\x01garbage",
    );
    env.import();
    let (repo, state) = env.state();
    let two = repo.store().save(&state.lines["main"], false).unwrap();
    assert_eq!(git_commit(&repo, &two.parents[0]), first.to_hex());
    let one = repo.store().save(&two.parents[0], false).unwrap();
    assert!(one.parents.is_empty());
}

// Decision 037
#[test]
fn lfs_pointers_and_files_that_look_like_one_stop_the_import() {
    let pointer = "version https://git-lfs.github.com/spec/v1\noid \
                   sha256:4d7a214614ab2935c943f9e0ff69d22eadbb8f32b1258daaa5e2ca24d17e2393\n\
                   size 12345\n";
    for (file, stop) in [
        // Git LFS reads an extension line before `version`, so this is a pointer.
        (
            format!("ext-0-foo sha256:{}\n{pointer}", "f".repeat(64)),
            "holds a Git LFS pointer at `art/big.psd`, and this version of Bolt can't import Git \
             LFS files yet: Git LFS repositories come in milestone 5.",
        ),
        // Git LFS rejects a line after `size`, but the file still looks like a pointer.
        (
            format!("{pointer}and then some\n"),
            "holds a file at `art/big.psd` that looks like a Git LFS pointer, though Git LFS \
             wouldn't read it as one; Bolt stops rather than guess whether it is a Git LFS file.",
        ),
    ] {
        let (env, git) = repo();
        let art = git.tree(&[(b"100644", b"big.psd", git.object(Blob, file.as_bytes()))]);
        let id = main_of(&git, &[(b"40000", b"art", art)]);
        let err = stopped(&env);
        let stop = format!("error: Commit `{}` {stop}\n", id.to_hex());
        assert!(err.contains(&stop), "{err}");
    }

    // A file that only quotes a pointer is an ordinary file.
    let (env, git) = repo();
    let notes = format!("Our big files use Git LFS:\n{pointer}");
    let notes_id = git.object(Blob, notes.as_bytes());
    main_of(&git, &[(b"100644", b"notes.txt", notes_id)]);
    env.import();
    assert_eq!(content(&env, "notes.txt"), notes.as_bytes());
}

/// A name that climbs into `.git`, an address that is an option, a command to run on update, and
/// values that end in a carriage return.
const GITMODULES: &[u8] = b"[submodule \"../../.git/hooks\"]\n\tpath = sub\n\
    \turl = -uupload-pack=touch /tmp/pwned\n\tupdate = !touch /tmp/pwned\n\
    [submodule \"sub\r\"]\n\tpath = sub\r\n\turl = ./sub\r\n";

// CVE-2018-11235, CVE-2018-17456, CVE-2019-1387, CVE-2025-48384; security.md T24
#[test]
fn submodules_are_bare_links_and_gitmodules_an_ordinary_file() {
    let (env, git) = repo();
    // First as a link to Git's own config, then as a file beside the submodule.
    let linked = tree(
        &git,
        &[(b"120000", b".gitmodules", git.object(Blob, b".git/config"))],
    );
    let first = commit(&git, linked, &[], "Linked");
    let sub = GitId::from_hex(HashKind::Sha1, &"5a".repeat(20)).unwrap();
    let modules = git.object(Blob, GITMODULES);
    let top = tree(
        &git,
        &[
            (b"100644", b".gitmodules", modules),
            (b"160000", b"sub", sub),
        ],
    );
    on_main(&git, commit(&git, top, &[first], "Hostile"));
    env.import();
    assert_eq!(saved(&env), ["file .gitmodules", "project-link sub"]);
    assert_eq!(content(&env, ".gitmodules"), GITMODULES);
    {
        let (repo, _) = env.state();
        let store = repo.store();
        let link = top_folder(env.root()).get("sub").cloned().unwrap();
        let link = store
            .read_record(&link.id, ObjectKind::ProjectLink)
            .unwrap();
        let link = ProjectLink::decode(&link).unwrap();
        assert_eq!(link, ProjectLink::submodule(sub.as_bytes()));
        assert!(link.url.is_empty());
        let head = store.save(&current_save(&repo).unwrap(), false).unwrap();
        let first = store.save(&head.parents[0], false).unwrap();
        let listing = store.listing(&first.root).unwrap();
        let linked = listing.get(".gitmodules").cloned().unwrap();
        assert_eq!(linked.kind, EntryKind::Symlink);
        let mut target = Vec::new();
        store.copy_content(&linked.id, &mut target).unwrap();
        assert_eq!(target, b".git/config");
    }

    // A later save keeps the link and never looks in its folder, whose `.git` file points at
    // /etc and whose `.boltignore` would stop the save if it were read.
    fs::write(env.root().join(".gitmodules"), GITMODULES).unwrap();
    let inside = env.root().join("sub");
    fs::create_dir(&inside).unwrap();
    fs::write(inside.join(".git"), "gitdir: /etc\n").unwrap();
    fs::write(inside.join("a.txt"), "x").unwrap();
    let too_big = "#".repeat(bolt_core::safe_fs::MAX_IGNORE_FILE_BYTES as usize + 1);
    fs::write(inside.join(".boltignore"), too_big).unwrap();
    fs::write(env.root().join("new.txt"), "new\n").unwrap();
    let out = env.save(&["After the import"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    let show = env.bolt().args(["--json", "show"]).output().unwrap();
    let json: serde_json::Value = serde_json::from_slice(&show.stdout).unwrap();
    let changed: Vec<&str> = json["changes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["path"].as_str().unwrap())
        .collect();
    assert_eq!(changed, ["new.txt"]);
}

// CVE-2021-21300, CVE-2024-32002, gitoxide GHSA-xw69
#[test]
fn links_are_imported_as_links_and_never_followed() {
    let (env, git) = repo();
    let hook = git.object(Blob, b"#!/bin/sh\necho owned\n");
    let a = git.tree(&[(b"100755", b"post-checkout", hook)]);
    // `A` beside `a`: one folder where the drive ignores letter case.
    let targets: [(&str, &[u8]); 4] = [
        ("A", b".git/hooks"),
        ("passwd", b"/etc/passwd"),
        ("share", b"\\\\attacker\\s\\x"),
        ("slashes", b"//attacker/s"),
    ];
    let mut entries: Vec<(&[u8], &[u8], GitId)> = targets
        .iter()
        .map(|(name, target)| (&b"120000"[..], name.as_bytes(), git.object(Blob, target)))
        .collect();
    entries.push((b"40000", b"a", a));
    main_of(&git, &entries);
    env.import();
    assert_eq!(
        saved(&env),
        [
            "symlink A",
            "executable a/post-checkout",
            "symlink passwd",
            "symlink share",
            "symlink slashes"
        ]
    );
    for (name, target) in targets {
        assert_eq!(content(&env, name), target, "{name}");
    }
    // The import wrote nothing: no link, nothing in Git's hooks.
    assert!(fs::symlink_metadata(env.root().join("A")).is_err());
    assert!(!env.root().join(".git/hooks").exists());
}

/// Text no terminal may be shown as it is (an escape sequence, a right-to-left override and a line
/// separator), and as Bolt shows it. A ref name can't hold the escape, so refs use the rest.
const HOSTILE: &str = "a\u{1b}[2Jb\u{202e}c\u{2028}d";
const HOSTILE_SHOWN: &str = "a\\x1b[2Jb\\u{202e}c\\u{2028}d";
const HOSTILE_REF: &str = "b\u{202e}c\u{2028}d";
const HOSTILE_REF_SHOWN: &str = "b\\u{202e}c\\u{2028}d";

/// Fails unless `shown` holds `escaped` and none of `HOSTILE`'s characters as they are.
#[track_caller]
fn assert_escaped(shown: &str, escaped: &str) {
    assert!(shown.contains(escaped), "{escaped} in {shown}");
    assert!(
        !shown.contains(['\u{1b}', '\u{202e}', '\u{2028}']),
        "{shown:?}"
    );
}

// security.md T21, CVE-2024-52005
#[test]
fn text_from_the_repository_is_shown_escaped() {
    let (env, git) = repo();
    let folder = tree(&git, &[(b"100644", b"x", git.object(Blob, b"x\n"))]);
    let who = format!("{HOSTILE} <m@example.com> 1700000000 +0000");
    let raw = format!(
        "tree {}\nauthor {who}\ncommitter {who}\n\nFix {HOSTILE}\n",
        folder.to_hex()
    );
    let id = git.commit(raw.as_bytes()).to_hex();
    // The branch Git is on, a ref left out and a tag of a folder, each named with it.
    git.packed_refs(&format!(
        "{id} refs/heads/{HOSTILE_REF}\n{id} refs/notes/{HOSTILE_REF}\n{} refs/tags/{HOSTILE_REF}\n",
        folder.to_hex()
    ));
    git.head(&format!("ref: refs/heads/{HOSTILE_REF}\n"));
    let (out, err) = env.import();
    let shown = format!("{out}{err}");
    let renamed = format!("Renamed 1 name that Bolt's names don't allow: {HOSTILE_REF_SHOWN} → ");
    assert_escaped(&shown, &renamed);
    assert_escaped(&shown, &format!("refs/notes/{HOSTILE_REF_SHOWN}\n"));
    let tag = format!("Didn't import 1 tag: {HOSTILE_REF_SHOWN}, as it tags");
    assert_escaped(&shown, &tag);
    let run = |command: &str| {
        let out = env.bolt().arg(command).output().unwrap();
        assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
        text(&out.stdout) + &text(&out.stderr)
    };
    // The author's name, then the message.
    let history = run("history");
    assert_escaped(
        &history,
        &format!("  {HOSTILE_SHOWN}  Fix {HOSTILE_SHOWN}\n"),
    );
    let show = run("show");
    assert_escaped(&show, &format!(" · {HOSTILE_SHOWN} · "));
    assert_escaped(&show, &format!("\n    Fix {HOSTILE_SHOWN}\n"));

    // And in errors: a branch with no commits yet, and a ref name Git refuses.
    let (env, git) = repo();
    one_file(&git);
    git.head(&format!("ref: refs/heads/{HOSTILE_REF}\n"));
    let err = refused(&env);
    let stop = format!(
        "error: Git's current branch `{HOSTILE_REF_SHOWN}` has no commits yet; switch to a branch \
         with commits, or commit first.\n"
    );
    assert_escaped(&err, &stop);
    let (env, git) = repo();
    let id = one_file(&git).to_hex();
    git.packed_refs(&format!("{id} refs/heads/{HOSTILE}\n"));
    let err = refused(&env);
    let stop = format!(
        "error: {BROKEN}the ref `refs/heads/{HOSTILE_SHOWN}` in .git/packed-refs has a name Git \
         doesn't allow.\n"
    );
    assert_escaped(&err, &stop);
}
