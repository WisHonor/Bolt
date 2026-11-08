//! Milestone 4 crash sweeps (no finding): `undo --to --shelve`, `redo --shelve`, `tidy`, `unshelf`
//! onto another save and `discard <path>` stopped at every abort point, then run on; the unsaved
//! work must stay on disk or in the store. Copy into crates/bolt-vcs/tests/ and run in the sandbox
//! with `cargo test -p bolt-vcs --features fault-injection --test crash_sweeps`.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![cfg(feature = "fault-injection")]
mod support;

use std::fs;
use std::time::{Duration, SystemTime};

use support::{Env, text};

fn ok(env: &Env, args: &[&str]) -> String {
    let out = env.bolt().args(args).output().unwrap();
    assert_eq!(out.status.code(), Some(0), "{args:?}: {}", text(&out.stderr));
    text(&out.stdout)
}

fn kept(env: &Env, name: &str, bytes: &[u8]) -> bool {
    if fs::read(env.root().join(name)).is_ok_and(|b| b == bytes) {
        return true;
    }
    let repo = bolt_core::repo::Repository::open(env.root()).unwrap();
    let mut out = Vec::new();
    repo.store()
        .copy_content(&bolt_core::id::hash_content(bytes), &mut out)
        .is_ok()
}

fn entries(env: &Env) -> Vec<String> {
    text(&env.bolt().args(["undo", "--list", "--all"]).output().unwrap().stdout)
        .lines()
        .map(|l| l.split_whitespace().next().unwrap_or("").to_owned())
        .collect()
}

fn age(env: &Env) {
    for entry in fs::read_dir(env.root().join(".bolt/store/packs")).unwrap() {
        let f = fs::File::options().write(true).open(entry.unwrap().path()).unwrap();
        f.set_modified(SystemTime::now() - Duration::from_secs(72 * 3600)).unwrap();
    }
}

/// Sweeps `args` over every abort point; after each, `next` runs, then the precious bytes must
/// be somewhere.
fn sweep(template: &Env, args: &[&str], next: &[&[&str]], precious: &[(&str, &[u8])]) {
    for n in 1..300 {
        let env = Env::empty();
        support::copy_dir(template.root(), env.root());
        age(&env);
        let out = env
            .bolt()
            .args(args)
            .env("BOLT_FAULT_AT", n.to_string())
            .output()
            .unwrap();
        let said = text(&out.stderr);
        let at = said
            .split_once("injected crash at ")
            .map(|(_, r)| r.lines().next().unwrap().to_owned());
        let st = env.bolt().arg("status").output().unwrap();
        let mut log = format!("{args:?} n={n} at={at:?} status={:?}\n", st.status.code());
        for a in next {
            let o = env.bolt().args(*a).output().unwrap();
            log += &format!(
                "  next {a:?}: {:?} {}{}\n",
                o.status.code(),
                text(&o.stdout).lines().next().unwrap_or(""),
                text(&o.stderr).lines().next().unwrap_or("")
            );
        }
        for (name, bytes) in precious {
            if !kept(&env, name, bytes) {
                panic!("LOST {name}\n{log}{said}");
            }
        }
        eprintln!("{log}");
        if at.is_none() {
            assert_eq!(out.status.code(), Some(0), "{said}");
            return;
        }
    }
}

#[test]
fn sweep_jump_shelve() {
    let t = Env::empty();
    ok(&t, &["start"]);
    fs::write(t.root().join("a"), "one").unwrap();
    ok(&t, &["save", "A"]);
    fs::write(t.root().join("a"), "two").unwrap();
    ok(&t, &["save", "B"]);
    let e = entries(&t);
    let b_entry = e[0].clone();
    ok(&t, &["undo"]);
    fs::write(t.root().join("c"), "cee").unwrap();
    ok(&t, &["save", "C"]);
    fs::write(t.root().join("a"), "WORK-A").unwrap();
    sweep(
        &t,
        &["undo", "--to", &b_entry, "--shelve"],
        &[&["undo", "--to", &b_entry, "--shelve"], &["undo"], &["redo"]],
        &[("a", b"WORK-A")],
    );
}

#[test]
fn sweep_redo_shelve() {
    let t = Env::empty();
    ok(&t, &["start"]);
    fs::write(t.root().join("a"), "one").unwrap();
    ok(&t, &["save", "A"]);
    fs::write(t.root().join("a"), "WORK-1").unwrap();
    ok(&t, &["discard"]);
    ok(&t, &["undo"]);
    fs::write(t.root().join("a"), "WORK-2").unwrap();
    sweep(
        &t,
        &["redo", "--shelve"],
        &[&["redo", "--shelve"], &["undo"], &["undo"]],
        &[("a", b"WORK-1"), ("a", b"WORK-2")],
    );
}

#[test]
fn sweep_tidy() {
    let t = Env::empty();
    ok(&t, &["start"]);
    for i in 0..5 {
        fs::write(t.root().join("a"), format!("v{i}")).unwrap();
        ok(&t, &["save", &format!("S{i}")]);
    }
    fs::write(t.root().join("a"), "WORK-D").unwrap();
    ok(&t, &["discard"]);
    fs::write(t.root().join("b"), "WORK-S").unwrap();
    ok(&t, &["shelf"]);
    ok(&t, &["undo", "2"]);
    ok(&t, &["redo", "2"]);
    age(&t);
    sweep(
        &t,
        &["tidy"],
        &[&["tidy"], &["undo", "2"]],
        &[("a", b"WORK-D"), ("b", b"WORK-S"), ("a", b"v4"), ("a", b"v0")],
    );
}

#[test]
fn sweep_unshelf_other_save() {
    let t = Env::empty();
    ok(&t, &["start"]);
    fs::write(t.root().join("a"), "one").unwrap();
    fs::write(t.root().join("o"), "o").unwrap();
    ok(&t, &["save", "A"]);
    fs::write(t.root().join("a"), "WORK-S").unwrap();
    ok(&t, &["shelf"]);
    fs::write(t.root().join("o"), "o2").unwrap();
    ok(&t, &["save", "B"]);
    fs::write(t.root().join("n"), "WORK-N").unwrap();
    sweep(
        &t,
        &["unshelf"],
        &[&["unshelf"], &["undo"], &["redo"]],
        &[("a", b"WORK-S"), ("n", b"WORK-N")],
    );
}

#[test]
fn sweep_discard_path() {
    let t = Env::empty();
    ok(&t, &["start"]);
    fs::create_dir(t.root().join("d")).unwrap();
    fs::write(t.root().join("d/x"), "x").unwrap();
    ok(&t, &["save", "A"]);
    fs::remove_dir_all(t.root().join("d")).unwrap();
    fs::write(t.root().join("d"), "WORK-D").unwrap();
    sweep(
        &t,
        &["discard", "d/x"],
        &[&["discard", "d/x"], &["undo"], &["redo"], &["undo"]],
        &[("d", b"WORK-D")],
    );
}
