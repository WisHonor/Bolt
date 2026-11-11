//! Milestone 6's second red-team session on presets and polish (decision 024): attacks only
//! Windows can run, as tests of the safe behaviour. See
//! docs/redteam/2026-11-10-milestone-6-session-2.md.
#![cfg(windows)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use std::fs;

use support::{Env, junction, text};

fn project() -> Env {
    let env = Env::empty();
    fs::write(env.root().join("a.txt"), "hello\n").unwrap();
    env.bolt()
        .args(["start", "--preset", "custom"])
        .assert()
        .success();
    env
}

/// A junction named like a pack, leading to a folder outside the project: `verify` reports it
/// and never goes through it, and `tidy`, which stops at a pack it can't open, leaves what it
/// leads to alone.
#[test]
fn a_junction_named_like_a_pack_is_reported_and_what_it_leads_to_is_left_alone() {
    let env = project();
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("keep.txt"), "outside\n").unwrap();
    let name = format!("{}.pack", "ab".repeat(32));
    junction(
        &env.root()
            .join(".bolt")
            .join("store")
            .join("packs")
            .join(name),
        outside.path(),
    );

    let verify = env.bolt().arg("verify").output().unwrap();
    assert_eq!(
        verify.status.code(),
        Some(1),
        "verify: {}{}",
        text(&verify.stdout),
        text(&verify.stderr)
    );
    let _ = env.bolt().arg("tidy").output().unwrap();
    assert_eq!(
        fs::read_to_string(outside.path().join("keep.txt")).unwrap(),
        "outside\n"
    );
}

/// A junction at `.bolt/state/tidied` counts as no record at all: `verify` still vouches for an
/// intact project, and nothing goes through the junction.
#[test]
fn a_junction_at_state_tidied_is_no_record_and_is_never_gone_through() {
    let env = project();
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("keep.txt"), "outside\n").unwrap();
    junction(
        &env.root().join(".bolt").join("state").join("tidied"),
        outside.path(),
    );

    let verify = env.bolt().arg("verify").output().unwrap();
    assert_eq!(
        verify.status.code(),
        Some(0),
        "verify: {}{}",
        text(&verify.stdout),
        text(&verify.stderr)
    );
    let _ = env.bolt().arg("tidy").output().unwrap();
    assert_eq!(
        fs::read_to_string(outside.path().join("keep.txt")).unwrap(),
        "outside\n"
    );
    assert_eq!(fs::read_dir(outside.path()).unwrap().count(), 1);
}
