//! `bolt tidy`, end to end.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;
use std::time::{Duration, SystemTime};

use bolt_core::repo::Repository;
use serde_json::{Value, json};

mod support;
use support::{Env, text};

/// `bolt <args>`, which must succeed; what it printed, and what it said on stderr.
fn ok(env: &Env, args: &[&str]) -> (String, String) {
    let out = env.bolt().args(args).output().unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "{args:?}: {}",
        text(&out.stderr)
    );
    (text(&out.stdout), text(&out.stderr))
}

/// A project with a file saved, and 64 KiB that don't compress in a pack of their own, which
/// nothing reaches, as a crash can leave them.
fn with_garbage() -> Env {
    let env = Env::empty();
    env.bolt().arg("start").assert().success();
    fs::write(env.root().join("a.txt"), "one").unwrap();
    ok(&env, &["save", "One"]);
    let mut repo = Repository::open(env.root()).unwrap();
    let lock = repo.lock_for_change("test", &mut |_| {}).unwrap();
    let mut packs = repo.new_packs().unwrap();
    let mut x: u64 = 0x9e37_79b9_7f4a_7c15;
    let lost: Vec<u8> = (0..64 << 10)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x.to_le_bytes()[3]
        })
        .collect();
    packs.add_content(&lost).unwrap();
    packs.finish().unwrap();
    drop(lock);
    env
}

/// Makes every pack of the project `hours` old, so that `tidy` may change it.
fn age(env: &Env, hours: u64) {
    let packs = env.root().join(".bolt").join("store").join("packs");
    for entry in fs::read_dir(packs).unwrap() {
        let file = fs::File::options()
            .write(true)
            .open(entry.unwrap().path())
            .unwrap();
        let then = SystemTime::now() - Duration::from_secs(hours * 3600);
        file.set_modified(then).unwrap();
    }
}

#[test]
fn tidy_says_what_it_will_free_and_what_it_freed() {
    let env = with_garbage();
    age(&env, 48);
    let (head, packs) = env.standing();
    let out = env.bolt().args(["tidy", "--json"]).output().unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    let doc: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(doc["bolt_json"], 1);
    let (before, after) = (doc["bytes_before"].as_u64(), doc["bytes_after"].as_u64());
    let freed = before.unwrap() - after.unwrap();
    assert!(freed >= 64 << 10, "{doc}");
    // Nothing kept is in two packs here, so the estimate is exact.
    assert_eq!(doc["to_free"], json!(freed), "{doc}");
    assert_eq!(doc["packs_removed"], json!(packs.len()), "{doc}");
    assert_eq!(doc["packs_written"], 1, "{doc}");
    assert_eq!(doc["packs_young"], 0, "{doc}");
    assert_eq!(doc["warnings"], json!([]), "{doc}");
    // It makes no diary entry, and the project reads as before.
    assert_eq!(env.standing().0, head);
    let (status, _) = ok(&env, &["status"]);
    assert!(status.contains("Nothing changed since"), "{status}");

    // In text, what it freed. Not in a terminal, so no estimate first.
    let env = with_garbage();
    age(&env, 48);
    let (said, told) = ok(&env, &["tidy"]);
    assert!(
        said.starts_with("Freed ") && said.contains(" KB ("),
        "{said}"
    );
    assert!(said.ends_with(" packs removed, 1 written).\n"), "{said}");
    assert_eq!(said.lines().count(), 1, "{said}");
    assert_eq!(told, "");
}

#[test]
fn tidy_leaves_packs_changed_in_the_last_day_untouched() {
    let env = with_garbage();
    let before = env.standing();
    let (said, _) = ok(&env, &["tidy"]);
    let young = before.1.len();
    assert_eq!(
        said,
        format!(
            "Freed 0 B (0 packs removed, 0 written).\nLeft {young} packs changed in the last 24 \
             hours as they are: a later bolt tidy frees what they hold that nothing needs.\n"
        )
    );
    assert_eq!(env.standing(), before);
}

#[test]
fn a_bad_undo_window_stops_tidy_naming_the_settings_file() {
    let env = with_garbage();
    age(&env, 48);
    let config = env.config().join("config.toml");
    for (setting, said) in [
        (
            "undo-window-days = 0\n",
            "is not a valid Bolt settings file: undo-window-days must be a whole number of days \
             from 1 to 36500, not 0",
        ),
        (
            "[editor]\nundo-window-days = 30\n",
            "undo-window-days must be at the top of",
        ),
    ] {
        fs::write(&config, setting).unwrap();
        let before = env.standing();
        let out = env.bolt().arg("tidy").output().unwrap();
        assert_eq!(out.status.code(), Some(1), "{setting}");
        let err = text(&out.stderr);
        assert!(err.contains(said), "{err}");
        assert!(err.contains(&config.display().to_string()), "{err}");
        assert_eq!(env.standing(), before, "{setting}");
    }
    fs::write(&config, "undo-window-days = 1\n").unwrap();
    let (said, _) = ok(&env, &["tidy"]);
    assert!(said.contains("written)."), "{said}");
}
