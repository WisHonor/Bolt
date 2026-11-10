//! RT12 exploration: planted last saves the helper reads ahead of the walk, each command run 25
//! times on this build and once on main's; all must say the same. Folders naming a file's
//! content, a save or nothing as their listing; folders sharing one listing; a `.git` spelling in
//! a subfolder's listing; thousands of saved folders gone from disk; a deep chain.
//! Run in the sandbox only: redteam/attacks11/run.sh explore rt12_ahead_planted
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::process::Command;

use bolt_core::id::Id;
use bolt_core::listing::{Entry, EntryKind, Listing};
use bolt_core::pack::{ObjectKind, PackSet};
use support::{Env, copy_dir, text};

const MAIN: &str = "/work/_main/target/release/bolt";

fn said(cmd: &mut Command, root: &Path, args: &[&str]) -> String {
    let out = cmd.args(args).output().unwrap();
    let shown = root.display().to_string();
    let clean = |s: String| {
        // Change IDs are drawn at random: keep only the words that aren't one.
        s.replace(&shown, "<root>")
            .split_whitespace()
            .map(|w| {
                if w.len() >= 4 && w.bytes().all(|b| (b'k'..=b'z').contains(&b)) {
                    "<cid>"
                } else {
                    w
                }
            })
            .collect::<Vec<_>>()
            .join(" ")
    };
    format!(
        "{:?} | {} | {}",
        out.status.code(),
        clean(text(&out.stdout)),
        clean(text(&out.stderr))
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

fn listing(pack: &mut PackSet, entries: Vec<Entry>) -> Id {
    let mut entries = entries;
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    pack.add_record(ObjectKind::FolderListing, &Listing::new(entries).unwrap().encode())
        .unwrap()
}

fn file(pack: &mut PackSet, name: &str, bytes: &[u8]) -> Entry {
    Entry {
        name: name.into(),
        kind: EntryKind::File,
        id: pack.add_content(bytes).unwrap(),
        size: bytes.len() as u64,
    }
}

fn folder(name: &str, id: Id) -> Entry {
    Entry {
        name: name.into(),
        kind: EntryKind::Folder,
        id,
        size: 0,
    }
}

type Build = Box<dyn Fn(&mut PackSet, Id) -> Id>;

fn cases() -> Vec<(&'static str, Build, Vec<&'static str>)> {
    vec![
        (
            "folder names content",
            Box::new(|p: &mut PackSet, _| {
                let c = p.add_content(b"not a listing").unwrap();
                let f = file(p, "f", b"f");
                listing(p, vec![folder("s", c), f])
            }),
            vec!["s/x", "f"],
        ),
        (
            "folder names a save",
            Box::new(|p: &mut PackSet, parent| {
                let f = file(p, "f", b"f");
                listing(p, vec![folder("s", parent), f])
            }),
            vec!["s/x", "f"],
        ),
        (
            "folder names nothing",
            Box::new(|p: &mut PackSet, _| {
                let f = file(p, "f", b"f");
                listing(p, vec![folder("s", Id([7; 32])), f])
            }),
            vec!["s/x", "f"],
        ),
        (
            "folder names nothing, gone from disk",
            Box::new(|p: &mut PackSet, _| {
                let f = file(p, "f", b"f");
                listing(p, vec![folder("s", Id([7; 32])), f])
            }),
            vec!["f"],
        ),
        (
            "shared listing",
            Box::new(|p: &mut PackSet, _| {
                let x = file(p, "x", b"x");
                let inner = listing(p, vec![x]);
                listing(p, vec![folder("a", inner), folder("b", inner), folder("c", inner)])
            }),
            vec!["a/x", "b/x", "c/y"],
        ),
        (
            "git spelling below",
            Box::new(|p: &mut PackSet, _| {
                let x = file(p, "x", b"x");
                let inner = listing(p, vec![x.clone()]);
                let s = listing(p, vec![x, folder(".GIT", inner)]);
                listing(p, vec![folder("s", s)])
            }),
            vec!["s/x"],
        ),
        (
            "thousands gone",
            Box::new(|p: &mut PackSet, _| {
                let x = file(p, "x", b"x");
                let mut top = vec![];
                for i in 0..3000 {
                    let n = file(p, "n", i.to_string().as_bytes());
                    let inner = listing(p, vec![x.clone(), n]);
                    top.push(folder(&format!("d{i:05}"), inner));
                }
                top.push(file(p, "z", b"z"));
                listing(p, top)
            }),
            vec!["z", "d00007/x", "d02999/x"],
        ),
    ]
}

#[test]
fn planted_last_saves_say_what_main_says() {
    let mut differ = Vec::new();
    for (name, build, disk) in cases() {
        let env = Env::empty();
        env.bolt().arg("start").assert().success();
        let parent = support::main_tip(&env);
        support::plant_save(env.root(), |p| build(p, parent));
        for path in &disk {
            let at = env.root().join(path);
            fs::create_dir_all(at.parent().unwrap()).unwrap();
            fs::write(&at, path.as_bytes().last().map(|b| vec![*b]).unwrap()).unwrap();
        }
        for args in [&["status"][..], &["changes"], &["history"], &["save", "again"]] {
            let (n, o) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
            copy_dir(env.root(), n.path());
            copy_dir(env.root(), o.path());
            let old = said(&mut main_bolt(&env, o.path()), o.path(), args);
            let mut runs = BTreeSet::new();
            let times = if args[0] == "save" { 1 } else { 25 };
            for _ in 0..times {
                runs.insert(said(&mut env.bolt_process_in(n.path()), n.path(), args));
            }
            let new = runs.iter().next().unwrap().clone();
            println!("{name} {args:?}: {}", new.chars().take(300).collect::<String>());
            if runs.len() > 1 || new != old {
                differ.push(format!("{name} {args:?}\nNEW: {runs:#?}\nOLD: {old}"));
            }
        }
    }
    for d in &differ {
        println!("DIFFER {d}\n");
    }
    assert!(differ.is_empty(), "{} differ", differ.len());
}
