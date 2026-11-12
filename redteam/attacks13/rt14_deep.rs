// RT14 exploration: the deep tree of shared listings against the commands and corners
// deep_tree.rs doesn't cover. Prints each command's exit, time and peak memory. Sandbox only.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use bolt_core::diary::{Action, DiaryEntry, FolderChange, State};
use bolt_core::id::Id;
use bolt_core::listing::{Entry, EntryKind, Listing};
use bolt_core::pack::{ObjectKind, PackSet};
use bolt_core::repo::Repository;
use support::{Env, main_tip, plant_save, run_limited};

/// A chain of `levels` listings each naming the next twice, under `top_name`; `leaf` makes
/// trees with different listings.
fn tree(pack: &mut PackSet, levels: usize, top_name: &str, leaf: &[u8]) -> Id {
    let x = pack.add_content(leaf).unwrap();
    let two = |kind, id, size| {
        let entries = ["a", "b"]
            .iter()
            .map(|n| Entry { name: (*n).to_owned(), kind, id, size })
            .collect();
        Listing::new(entries).unwrap().encode()
    };
    let mut at = pack
        .add_record(ObjectKind::FolderListing, &two(EntryKind::File, x, leaf.len() as u64))
        .unwrap();
    for _ in 0..levels {
        at = pack
            .add_record(ObjectKind::FolderListing, &two(EntryKind::Folder, at, 0))
            .unwrap();
    }
    let top = Listing::new(vec![Entry {
        name: top_name.into(),
        kind: EntryKind::Folder,
        id: at,
        size: 0,
    }])
    .unwrap();
    pack.add_record(ObjectKind::FolderListing, &top.encode()).unwrap()
}

fn started() -> (Env, Id) {
    let env = Env::empty();
    env.bolt().args(["start", "--preset", "custom"]).assert().success();
    let first = main_tip(&env);
    (env, first)
}

fn plant_entry(env: &Env, command: &str, folder_change: Option<FolderChange>, edit: impl FnOnce(&mut State)) {
    let mut repo = Repository::open(env.root()).unwrap();
    let lock = repo.lock_for_change(command, &mut |_| {}).unwrap();
    let (head, entry) = repo.current().unwrap();
    let mut state = entry.state.clone();
    edit(&mut state);
    let diary = DiaryEntry {
        parent: Some(head),
        action: Action::Command,
        command: command.into(),
        who: entry.who,
        bolt_version: "0.1.0".into(),
        state,
        folder_change,
    };
    let packs = repo.new_packs().unwrap();
    repo.commit(&lock, packs, &diary).unwrap();
}

fn probe(env: &Env, label: &str, args: &[&str]) {
    let mut bolt = env.bolt_process_in(env.root());
    bolt.args(args);
    let e = run_limited(bolt, 90);
    let first: String = e.stderr.lines().next().unwrap_or("").chars().take(150).collect();
    println!(
        "PROBE {label} {args:?}: code={:?} timed_out={} secs={:.2} peak={}MiB err={first:?}",
        e.code, e.timed_out, e.secs, e.peak_mb
    );
}

#[test]
fn rt14_deep_other_commands() {
    // The folder at the first save; main at the 2^25 tree.
    let (env, _first) = started();
    plant_save(env.root(), |p| tree(p, 24, "deep", b"x"));
    for args in [
        &["history"][..],
        &["history", "--json"],
        &["line"],
        &["show"],
        &["changes"],
        &["undo", "--list"],
        &["save", "x", "deep", "--yes"],
        &["save", "x", "nothing-here", "--yes"],
        &["save", "x", "--yes"],
        &["save", "--amend", "--yes"],
        &["verify"],
        &["tidy"],
    ] {
        probe(&env, "tree-on-main", args);
    }
}

#[test]
fn rt14_deep_long_name() {
    // The tree under a 300-byte name, which Linux can't hold, so the walk keeps it as it was.
    let long = "n".repeat(300);
    let (env, _first) = started();
    plant_save(env.root(), |p| tree(p, 24, &long, b"x"));
    for args in [&["status"][..], &["save", "x", "--yes"], &["goto", "main"]] {
        probe(&env, "long-name", args);
    }
}

#[test]
fn rt14_deep_ignored_folder() {
    // The tree's folder is on disk but empty, and an ignore rule keeps it out.
    let (env, _first) = started();
    std::fs::create_dir(env.root().join("deep")).unwrap();
    std::fs::write(env.root().join(".boltignore"), "deep/\n").unwrap();
    plant_save(env.root(), |p| tree(p, 24, "deep", b"x"));
    for args in [&["status"][..], &["save", "x", "--yes"], &["status"]] {
        probe(&env, "ignored", args);
    }
}

#[test]
fn rt14_deep_two_trees() {
    // main at tree A, line `other` at tree B; the folder believed at A.
    let (env, _first) = started();
    plant_save(env.root(), |p| tree(p, 24, "deep", b"x"));
    let a = main_tip(&env);
    plant_save(env.root(), |p| tree(p, 24, "deep", b"y"));
    let b = main_tip(&env);
    plant_entry(&env, "goto", Some(FolderChange { from: b, to: a }), |s| {
        s.lines.insert("main".into(), a);
        s.lines.insert("other".into(), b);
    });
    for args in [&["goto", "other"][..], &["changes", "--save", "other"], &["undo"]] {
        probe(&env, "two-trees", args);
    }
}

#[test]
fn rt14_deep_within_limits() {
    // An honest-looking save within the limits: 2^22 files by path (21 levels), 1 KiB stored.
    let (env, first) = started();
    plant_save(env.root(), |p| tree(p, 21, "deep", b"x"));
    let _ = first;
    for args in [&["save", "x", "deep", "--yes"][..], &["save", "x", "--yes"], &["status"]] {
        probe(&env, "within", args);
    }
}
