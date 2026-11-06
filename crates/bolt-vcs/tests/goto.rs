//! `bolt goto`, end to end. For now its helpers, and a check that they do what tests need.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use bolt_core::journal::Journal;
use bolt_core::repo::Repository;

mod support;
use support::{Env, text};

/// Holds the file at `path` open, shared with nobody, as an editor or virus scanner may hold
/// one, until the handle is dropped. Another process's commands then can't replace or remove it.
#[cfg(windows)]
pub fn hold_open(path: &std::path::Path) -> std::fs::File {
    use std::os::windows::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(path)
        .unwrap()
}

#[cfg(windows)]
#[test]
fn a_held_file_cant_be_replaced_or_removed_until_let_go() {
    let dir = tempfile::tempdir().unwrap();
    let (held, other) = (dir.path().join("held"), dir.path().join("other"));
    std::fs::write(&held, b"old").unwrap();
    std::fs::write(&other, b"new").unwrap();
    let handle = hold_open(&held);
    const SHARING_VIOLATION: i32 = 32;
    // Windows' own rename says "access denied" here; Bolt's, by handle, a sharing violation.
    assert!(std::fs::rename(&other, &held).is_err());
    let removed = std::fs::remove_file(&held).unwrap_err();
    assert_eq!(removed.raw_os_error(), Some(SHARING_VIOLATION));
    drop(handle);
    std::fs::rename(&other, &held).unwrap();
    assert_eq!(std::fs::read(&held).unwrap(), b"new");
}

/// The kinds of the warnings in `bolt <args> --json`'s document.
fn warning_kinds(env: &Env, args: &[&str]) -> (Vec<String>, serde_json::Value) {
    let out = env.bolt().args(args).arg("--json").output().unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    let doc: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let kinds = doc["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| w["kind"].as_str().unwrap().to_owned())
        .collect();
    (kinds, doc)
}

/// A project with a save holding `a.txt`, put back on its first save as a switch to it that
/// stopped before `head` moved would leave it.
fn stopped_switch() -> Env {
    let env = Env::empty();
    env.bolt().arg("start").assert().success();
    let first = Repository::open(env.root()).unwrap().head().unwrap();
    env.overwrite("a.txt", "a");
    env.bolt().args(["save", "A"]).assert().success();
    let mut repo = Repository::open(env.root()).unwrap();
    let second = repo.head().unwrap();
    let lock = repo.lock_for_change("goto", &mut |_| {}).unwrap();
    repo.move_head_to(&lock, first).unwrap();
    let journal = Journal {
        target: second,
        from: first,
        actions: vec![],
    };
    journal.write(&repo, &lock).unwrap();
    env
}

#[test]
fn read_only_commands_say_a_switch_is_unfinished() {
    let env = stopped_switch();
    for command in ["status", "history", "show"] {
        let (kinds, doc) = warning_kinds(&env, &[command]);
        assert!(
            kinds.contains(&"switch_unfinished".to_owned()),
            "{command}: {doc}"
        );
        let out = env.bolt().arg(command).output().unwrap();
        let said = text(&out.stderr);
        assert!(
            said.contains("warning: A switch between saves was stopped"),
            "{said}"
        );
    }
    let (_, doc) = warning_kinds(&env, &["status"]);
    assert!(doc["unfinished_switch"].is_string(), "{doc}");

    // The next command that changes the project finishes it, and says so.
    let (kinds, doc) = warning_kinds(&env, &["save", "B"]);
    assert_eq!(doc["result"], "nothing-changed", "{doc}");
    assert_eq!(kinds, ["switch_finished"]);
    let (kinds, doc) = warning_kinds(&env, &["status"]);
    assert!(kinds.is_empty(), "{doc}");
    assert!(doc["unfinished_switch"].is_null());
    assert!(!env.root().join(".bolt/state/journal").exists());
}

#[test]
fn a_damaged_journal_stops_a_save_and_is_reported() {
    let env = stopped_switch();
    let journal = env.root().join(".bolt/state/journal");
    std::fs::write(&journal, b"BOLTJRNL garbage").unwrap();
    let out = env.save(&["B"]);
    assert_eq!(out.status.code(), Some(1));
    let said = text(&out.stderr);
    assert!(said.contains(".bolt/state/journal"), "{said}");
    assert!(said.contains("bolt status"), "{said}");
    let (kinds, doc) = warning_kinds(&env, &["status"]);
    assert!(kinds.contains(&"switch_unfinished".to_owned()), "{doc}");
    assert!(journal.exists());
}
