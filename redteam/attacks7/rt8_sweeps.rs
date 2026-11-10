//! Milestone 5 crash sweeps: commands that store big files stopped at every abort point, then run
//! on; the unsaved work and every save must stay readable. Run in the sandbox with
//! `cargo test -p bolt-vcs --features fault-injection --test rt8_sweeps -- --test-threads=2`.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![cfg(feature = "fault-injection")]
mod support;

use std::fs;
use std::time::{Duration, SystemTime};

use bolt_core::id::{Id, hash_content};
use support::{Env, id_of, noise, text};

const MIB: usize = 1 << 20;

fn ok(env: &Env, args: &[&str]) -> String {
    let out = env.bolt().args(args).output().unwrap();
    assert_eq!(out.status.code(), Some(0), "{args:?}: {}", text(&out.stderr));
    text(&out.stdout)
}

/// Whether the bytes with ID `id` are at `name` on disk or read back whole from the store.
fn kept(env: &Env, name: &str, id: Id) -> bool {
    let at = env.root().join(name);
    if at.is_file() && id_of(&at) == id {
        return true;
    }
    let repo = bolt_core::repo::Repository::open(env.root()).unwrap();
    let mut out = Vec::new();
    repo.store().copy_content(&id, &mut out).is_ok() && hash_content(&out) == id
}

fn age(env: &Env) {
    for entry in fs::read_dir(env.root().join(".bolt/store/packs")).unwrap() {
        let f = fs::File::options()
            .write(true)
            .open(entry.unwrap().path())
            .unwrap();
        f.set_modified(SystemTime::now() - Duration::from_secs(72 * 3600))
            .unwrap();
    }
}

/// Sweeps `args` over every abort point; after each, `next` runs, then the precious bytes must be
/// somewhere.
fn sweep(template: &Env, args: &[&str], next: &[&[&str]], precious: &[(&str, Id)], aged: bool) {
    for n in 1..400 {
        let env = Env::empty();
        support::copy_dir(template.root(), env.root());
        if aged {
            age(&env);
        }
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
        let mut log = format!(
            "{args:?} n={n} at={at:?} status={:?} {}\n",
            st.status.code(),
            text(&st.stderr).lines().next().unwrap_or("")
        );
        for a in next {
            let o = env.bolt().args(*a).output().unwrap();
            log += &format!(
                "  next {a:?}: {:?} {}{}\n",
                o.status.code(),
                text(&o.stdout).lines().next().unwrap_or(""),
                text(&o.stderr).lines().next().unwrap_or("")
            );
        }
        for (name, id) in precious {
            if !kept(&env, name, *id) {
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

fn put(env: &Env, name: &str, bytes: &[u8]) -> Id {
    let at = env.root().join(name);
    fs::create_dir_all(at.parent().unwrap()).unwrap();
    fs::write(at, bytes).unwrap();
    hash_content(bytes)
}

#[test]
fn sweep_discard_big() {
    let t = Env::empty();
    ok(&t, &["start"]);
    let v1 = put(&t, "big.bin", &noise(1, 3 * MIB));
    put(&t, "small.txt", b"one");
    ok(&t, &["save", "A"]);
    let mut edited = noise(1, 3 * MIB);
    edited[1_000_000..1_000_010].copy_from_slice(b"EDITEDWORK");
    let v2 = put(&t, "big.bin", &edited);
    let n2 = put(&t, "new.bin", &noise(3, 2 * MIB));
    sweep(
        &t,
        &["discard"],
        &[&["discard"], &["undo"], &["redo"], &["undo"]],
        &[("big.bin", v1), ("big.bin", v2), ("new.bin", n2)],
        false,
    );
}

#[test]
fn sweep_shelf_big() {
    let t = Env::empty();
    ok(&t, &["start"]);
    let v1 = put(&t, "big.bin", &noise(1, 3 * MIB));
    ok(&t, &["save", "A"]);
    let v2 = put(&t, "big.bin", &noise(2, 3 * MIB));
    let n2 = put(&t, "d/new.bin", &noise(4, 2 * MIB));
    sweep(
        &t,
        &["shelf"],
        &[&["shelf"], &["unshelf"], &["undo"], &["redo"]],
        &[("big.bin", v1), ("big.bin", v2), ("d/new.bin", n2)],
        false,
    );
}

#[test]
fn sweep_unshelf_big() {
    let t = Env::empty();
    ok(&t, &["start"]);
    let v1 = put(&t, "big.bin", &noise(1, 3 * MIB));
    ok(&t, &["save", "A"]);
    let v2 = put(&t, "big.bin", &noise(2, 3 * MIB));
    ok(&t, &["shelf"]);
    let w = put(&t, "other.bin", &noise(5, 2 * MIB));
    sweep(
        &t,
        &["unshelf"],
        &[&["unshelf"], &["undo"], &["redo"]],
        &[("big.bin", v1), ("big.bin", v2), ("other.bin", w)],
        false,
    );
}

#[test]
fn sweep_save_big() {
    let t = Env::empty();
    ok(&t, &["start"]);
    let v1 = put(&t, "big.bin", &noise(1, 3 * MIB));
    ok(&t, &["save", "A"]);
    let mut edited = noise(1, 3 * MIB);
    edited[2_000_000..2_000_010].copy_from_slice(b"EDITEDWORK");
    let v2 = put(&t, "big.bin", &edited);
    let n2 = put(&t, "new.bin", &noise(6, 5 * MIB / 2));
    sweep(
        &t,
        &["save", "B"],
        &[&["save", "B"], &["undo"], &["redo"]],
        &[("big.bin", v1), ("big.bin", v2), ("new.bin", n2)],
        false,
    );
}

#[test]
fn sweep_tidy_big() {
    let t = Env::empty();
    ok(&t, &["start"]);
    let mut ids = Vec::new();
    for i in 0..4u64 {
        let mut bytes = noise(10, 3 * MIB);
        bytes[(i as usize) * 500_000..(i as usize) * 500_000 + 8].copy_from_slice(b"VERSION!");
        ids.push(put(&t, "big.bin", &bytes));
        ok(&t, &["save", &format!("S{i}")]);
    }
    let work = put(&t, "big.bin", &noise(11, 3 * MIB));
    ok(&t, &["discard"]);
    let shelved = put(&t, "s.bin", &noise(12, 2 * MIB));
    ok(&t, &["shelf"]);
    let mut precious = vec![("big.bin", work), ("s.bin", shelved)];
    for id in &ids {
        precious.push(("big.bin", *id));
    }
    sweep(&t, &["tidy"], &[&["tidy"], &["undo"]], &precious, true);
}
