//! Exploration, not a proof: work a dropped switch wrote over, after `undo --to` and `tidy`.
//! It held (the stopped discard's entry stays a diary end inside the undo window).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use std::fs;
use std::time::{Duration, SystemTime};

use bolt_core::id::hash_content;
use support::{Env, copies_of, text, write};

fn age(env: &Env, hours: u64) {
    for entry in fs::read_dir(env.root().join(".bolt/store/packs")).unwrap() {
        let file = fs::File::options()
            .write(true)
            .open(entry.unwrap().path())
            .unwrap();
        file.set_modified(SystemTime::now() - Duration::from_secs(hours * 3600))
            .unwrap();
    }
}

/// `discard` stops after writing the saved file over unsaved work, and `head` is then lost. The
/// pick-up drops the switch and says the work it wrote over "is kept in that switch's own saves".
/// Nothing names those saves any more: no command shows them, and the next `tidy` frees them.
#[test]
fn rt10_2_work_a_dropped_switch_wrote_over_stays_reachable() {
    let precious = b"unsaved work, written over by the discard\n";
    let mut stopped = None;
    for n in 1..40 {
        let env = Env::empty();
        env.bolt()
            .args(["start", "--preset", "custom"])
            .assert()
            .success();
        write(&env, "a.txt", "saved\n");
        env.bolt().args(["save", "First"]).assert().success();
        write(&env, "a.txt", precious);
        let out = env
            .bolt()
            .env("BOLT_FAULT_AT", n.to_string())
            .arg("discard")
            .output()
            .unwrap();
        let said = text(&out.stderr);
        if out.status.success() {
            break;
        }
        if said.contains("injected crash at switch:changed")
            && fs::read(env.root().join("a.txt")).unwrap() == b"saved\n"
        {
            stopped = Some(env);
            break;
        }
    }
    let env = stopped.expect("no stop after discard wrote a.txt");
    let (entry, _) = support::head(&env);
    fs::remove_file(env.root().join(".bolt/state/head")).unwrap();
    let out = env
        .bolt()
        .args(["undo", "--to", &entry.to_hex()])
        .output()
        .unwrap();
    eprintln!(
        "bolt undo --to: {:?} {}",
        out.status.code(),
        text(&out.stderr).trim()
    );
    assert_eq!(out.status.code(), Some(0));
    let listed = env.bolt().args(["undo", "--list", "--all"]).output().unwrap();
    eprintln!("bolt undo --list --all:\n{}", text(&listed.stdout));
    let work = hash_content(precious);
    eprintln!("copies of the unsaved work before tidy: {:?}", copies_of(env.root(), &work));
    age(&env, 48);
    let out = env.bolt().arg("tidy").output().unwrap();
    eprintln!("bolt tidy: {:?} {}", out.status.code(), text(&out.stdout).trim());
    let left = copies_of(env.root(), &work);
    eprintln!("copies of the unsaved work after tidy: {left:?}");
    assert!(!left.is_empty(), "tidy freed the work the warning says is kept");
}
