//! Red-team session RT14 (the v0.1 release): proof tests. Each asserts the safe behaviour, so it
//! fails on the code attacked and passes once its finding is fixed. The report is
//! docs/redteam/2026-11-11-v0.1-release.md.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use std::io::Write;
use std::process::{Command, Stdio};
use std::time::Instant;

use bolt_core::drive::Drive;
use bolt_core::ignore::{Decision, Matcher, Rules};
use support::Env;
#[cfg(target_os = "linux")]
use {
    bolt_core::id::Id,
    bolt_core::listing::{Entry, EntryKind, Listing},
    bolt_core::pack::{ObjectKind, PackSet},
    support::{plant_save, run_limited},
};

/// A save's top listing holding `deep`: a chain of 21 listings each naming the next twice, about
/// 1 KiB stored and 8,388,606 files and folders by path, within the format's 10,000,000.
#[cfg(target_os = "linux")]
fn shared_tree(pack: &mut PackSet) -> Id {
    let x = pack.add_content(b"x").unwrap();
    let two = |kind, id, size| {
        let entries = ["a", "b"]
            .iter()
            .map(|n| Entry {
                name: (*n).to_owned(),
                kind,
                id,
                size,
            })
            .collect();
        Listing::new(entries).unwrap().encode()
    };
    let mut at = pack
        .add_record(ObjectKind::FolderListing, &two(EntryKind::File, x, 1))
        .unwrap();
    for _ in 0..21 {
        at = pack
            .add_record(ObjectKind::FolderListing, &two(EntryKind::Folder, at, 0))
            .unwrap();
    }
    let top = Listing::new(vec![Entry {
        name: "deep".into(),
        kind: EntryKind::Folder,
        id: at,
        size: 0,
    }])
    .unwrap();
    pack.add_record(ObjectKind::FolderListing, &top.encode())
        .unwrap()
}

/// RT14-1: a planted save of about 1 KiB, within the format's limits through shared listings,
/// costs `status` seconds and gigabytes: 17 s and 1.4 GiB in a release build, path by path.
/// The safe behaviour: what a command spends on a save stays in proportion to what the save
/// stores, or the save is refused at once.
#[cfg(target_os = "linux")]
#[test]
#[ignore = "RT14-1: a planted save within the limits costs status gigabytes; until its fix is in"]
fn a_small_planted_save_within_the_limits_costs_status_little() {
    let env = Env::empty();
    env.bolt()
        .args(["start", "--preset", "custom"])
        .assert()
        .success();
    plant_save(env.root(), shared_tree);
    let mut bolt = env.bolt_process_in(env.root());
    bolt.arg("status");
    let ended = run_limited(bolt, 120);
    println!("{ended:?}");
    assert!(
        ended.peak_mb < 256,
        "status held {} MiB for a save of about 1 KiB",
        ended.peak_mb
    );
    assert!(ended.secs < 5.0, "status took {:.1} s", ended.secs);
}

/// RT14-2: Git refuses a boolean config value of exactly -2^31 (`-2147483648`, `-2g`,
/// `-2097152k`): its range check stops at -(2^31 - 1). Bolt reads it as true, so it imports a
/// repository whose `extensions.preciousObjects` Git refuses ("fatal: bad boolean config value"),
/// and would read `config.worktree` for an `extensions.worktreeConfig` Git refuses.
#[test]
#[ignore = "RT14-2: a boolean of -2^31 is read where Git refuses it; until its fix is in"]
fn import_refuses_a_boolean_extension_git_refuses() {
    let env = Env::new();
    env.commit("a.txt", "a\n", "First");
    env.git(&["config", "core.repositoryformatversion", "1"]);
    env.git(&["config", "extensions.preciousObjects", "-2147483648"]);
    let refused = Command::new("git")
        .arg("status")
        .current_dir(env.root())
        .output()
        .unwrap();
    assert!(!refused.status.success(), "Git reads it after all");
    let out = env.bolt().arg("import").output().unwrap();
    assert!(
        !out.status.success(),
        "Bolt imported a repository Git refuses: {}",
        String::from_utf8_lossy(&out.stdout)
    );
}

/// RT14-3: an ignore file of glob lines costs Bolt many times Git's time. 47,368 lines like
/// `[!a-z]17*[xy]?` (under 1 MiB) against 2,000 paths that none matches: in a release build
/// `bolt status` takes 14.3 s where `git status` takes 1.1 s, and for `**/z1/**/q*` lines 26 s
/// against 3.6 s. Each line tried allocates the matcher's state afresh. Run with --release.
#[test]
#[ignore = "RT14-3: glob ignore lines cost Bolt over ten times Git's time; until its fix is in"]
fn hostile_glob_ignore_lines_cost_bolt_no_more_than_twice_git() {
    let mut text = String::new();
    for i in 0..47_368 {
        text.push_str(&format!("[!a-z]{i}*[xy]?\n"));
    }
    let paths: Vec<String> = (1..=2000).map(|i| format!("d/f{i}.txt")).collect();

    let dir = tempfile::tempdir().unwrap();
    assert!(
        Command::new("git")
            .args(["init", "-q"])
            .current_dir(dir.path())
            .status()
            .unwrap()
            .success()
    );
    std::fs::write(dir.path().join(".gitignore"), &text).unwrap();
    let mut input = paths.join("\n");
    input.push('\n');
    let started = Instant::now();
    let mut git = Command::new("git")
        .args(["check-ignore", "--no-index", "--stdin"])
        .current_dir(dir.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .spawn()
        .unwrap();
    git.stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    git.wait().unwrap();
    let git_secs = started.elapsed().as_secs_f64();

    let started = Instant::now();
    let mut matcher = Matcher::new(Drive {
        ignores_case: false,
        composes_unicode: false,
    });
    matcher.enter("", vec![Rules::parse(text.as_bytes(), "", true)]);
    let mut kept = 0;
    for path in &paths {
        kept += usize::from(matcher.verdict(path, false).git != Decision::Ignored);
    }
    let bolt_secs = started.elapsed().as_secs_f64();
    assert_eq!(kept, paths.len());
    println!("git {git_secs:.2} s, bolt {bolt_secs:.2} s");
    assert!(
        bolt_secs <= 2.0 * git_secs.max(0.1),
        "Bolt took {bolt_secs:.2} s where Git took {git_secs:.2} s"
    );
}
