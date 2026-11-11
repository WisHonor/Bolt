//! RT12 exploration: listings with a damaged second copy in a planted pack, the pack sorting
//! before or after the good one's, and holding the listing read just before; every read-only
//! command run 30 times on this build and once on main's, which must all say the same.
//! Run in the sandbox only: redteam/attacks11/run.sh explore rt12_ahead_damaged
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use std::fs;
use std::path::Path;
use std::process::Command;
use std::time::{Duration, SystemTime};

use bolt_core::pack::{ENTRY_HEADER_LEN, ObjectKind, PackReader};
use bolt_core::repo::Repository;
use support::{Env, copy_dir, text};

const MAIN: &str = "/work/_main/target/release/bolt";

fn project() -> Env {
    let env = Env::empty();
    env.bolt().arg("start").assert().success();
    for (path, bytes) in [("a/x", "x"), ("b/y/z", "z"), ("c/w", "w"), ("b/q", "q")] {
        let at = env.root().join(path);
        fs::create_dir_all(at.parent().unwrap()).unwrap();
        fs::write(&at, bytes).unwrap();
        let f = fs::File::options().write(true).open(&at).unwrap();
        f.set_modified(SystemTime::now() - Duration::from_secs(3600)).unwrap();
    }
    env.bolt().args(["save", "Files"]).assert().success();
    env
}

#[derive(Clone, Copy, Debug)]
enum Damage {
    Byte,
    Header,
    Cut,
    Gone,
}

/// Plants a pack holding good copies of `good` listings and a damaged copy of `bad`, padded with
/// `pad` so its name changes; the pack's path and whether it sorts first.
fn plant(env: &Env, good: &[bolt_core::id::Id], bad: bolt_core::id::Id, pad: u32, how: Damage) -> (std::path::PathBuf, bool) {
    let repo = Repository::open(env.root()).unwrap();
    let records: Vec<Vec<u8>> = good.iter().map(|id| repo.store().listing(id).unwrap().encode()).collect();
    let bad_bytes = repo.store().listing(&bad).unwrap().encode();
    drop(repo);
    let made = support::add_pack(env.root(), |pack| {
        for r in &records {
            pack.add_record(ObjectKind::FolderListing, r).unwrap();
        }
        pack.add_record(ObjectKind::FolderListing, &bad_bytes).unwrap();
        pack.add_content(&pad.to_le_bytes()).unwrap();
    });
    let path = made[0].clone();
    let entry = PackReader::open(&path).unwrap().find(&bad).unwrap().unwrap();
    let mut bytes = fs::read(&path).unwrap();
    match how {
        Damage::Byte => bytes[(entry.offset + ENTRY_HEADER_LEN) as usize] ^= 1,
        Damage::Header => bytes[(entry.offset + 3) as usize] ^= 1,
        Damage::Cut => bytes.truncate((entry.offset + ENTRY_HEADER_LEN + 1) as usize),
        Damage::Gone => {}
    }
    fs::write(&path, &bytes).unwrap();
    let packs = env.root().join(".bolt/store/packs");
    let mut names: Vec<_> = fs::read_dir(&packs).unwrap().map(|e| e.unwrap().file_name()).collect();
    names.sort();
    let first = names.iter().find(|n| n.to_string_lossy().ends_with(".pack")).unwrap() == path.file_name().unwrap();
    (path, first)
}

fn said(bolt: &mut Command, root: &Path, args: &[&str]) -> String {
    let out = bolt.args(args).output().unwrap();
    let shown = root.display().to_string();
    format!(
        "{:?}\n{}\n{}",
        out.status.code(),
        text(&out.stdout).replace(&shown, "<root>"),
        text(&out.stderr).replace(&shown, "<root>")
    )
}

fn main_bolt(env: &Env, dir: &Path) -> Command {
    let mut cmd = Command::new(MAIN);
    cmd.current_dir(dir)
        .env("BOLT_USER_NAME", "Test")
        .env("BOLT_USER_EMAIL", "test@example.com")
        .env("BOLT_CONFIG_DIR", env.config())
        .env("HOME", env.config());
    cmd
}

#[test]
fn damaged_copies_say_what_main_says() {
    let mut differ = Vec::new();
    for how in [Damage::Byte, Damage::Header, Damage::Cut, Damage::Gone] {
        for pad in 0..6u32 {
            let env = project();
            let top = support::top_folder(env.root());
            let (a, b) = (top.get("a").unwrap().id, top.get("b").unwrap().id);
            // `a` read just before `b`, from the planted pack when it sorts first.
            let (_, first) = plant(&env, &[a], b, pad, how);
            for args in [&["status"][..], &["changes"], &["history"], &["save", "again"], &["status"]] {
                let new_dir = tempfile::tempdir().unwrap();
                let old_dir = tempfile::tempdir().unwrap();
                copy_dir(env.root(), new_dir.path());
                copy_dir(env.root(), old_dir.path());
                let old = said(&mut main_bolt(&env, old_dir.path()), old_dir.path(), args);
                let mut runs = std::collections::BTreeSet::new();
                for _ in 0..(if args[0] == "save" { 1 } else { 30 }) {
                    runs.insert(said(&mut env.bolt_process_in(new_dir.path()), new_dir.path(), args));
                }
                let new = runs.iter().next().unwrap().clone();
                if runs.len() > 1 || new != old {
                    differ.push(format!("{how:?} pad {pad} first {first} {args:?}\nNEW({}): {runs:#?}\nOLD: {old}", runs.len()));
                }
            }
            println!("{how:?} pad {pad} planted pack first: {first}");
        }
    }
    for d in &differ {
        println!("DIFFER {d}\n");
    }
    assert!(differ.is_empty(), "{} differ", differ.len());
}
