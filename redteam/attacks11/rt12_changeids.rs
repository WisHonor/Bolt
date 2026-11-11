//! RT12 exploration: a `cache/change-ids` sealed with the user's own key whose list for `main`
//! starts at its newest save, as it should, but holds an ID no save of the line has: a folder
//! listing's, a file's content's, or nothing's. `goto <all 64 digits>` of that ID, `history`,
//! `undo --list`, and `goto <change ID>` on this build and on main's.
//! Run in the sandbox only: redteam/attacks11/run.sh explore rt12_changeids
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use std::fs;
use std::process::Command;

use bolt_core::id::Id;
use bolt_core::keyed::{self, KEY_LEN};
use bolt_core::repo::Repository;
use support::{Env, copy_dir, text};

const MAIN: &str = "/work/_main/target/release/bolt";

fn varint(out: &mut Vec<u8>, mut v: u64) {
    while v >= 0x80 {
        out.push((v as u8 & 0x7f) | 0x80);
        v >>= 7;
    }
    out.push(v as u8);
}

/// A version 2 body holding one list, `main`'s.
fn body(saves: &[(Id, [u8; 16])]) -> Vec<u8> {
    let mut b = b"BOLTCHID".to_vec();
    b.push(2);
    varint(&mut b, 1);
    varint(&mut b, 4);
    b.extend_from_slice(b"main");
    varint(&mut b, saves.len() as u64);
    for (id, change) in saves {
        b.extend_from_slice(&id.0);
        b.extend_from_slice(change);
    }
    b
}

fn said(cmd: &mut Command, args: &[&str]) -> String {
    let out = cmd.args(args).output().unwrap();
    format!(
        "{:?} | {} | {}",
        out.status.code(),
        text(&out.stdout).replace('\n', " / "),
        text(&out.stderr).replace('\n', " / ")
    )
}

#[test]
fn a_list_with_a_stranger() {
    let env = Env::empty();
    env.bolt().arg("start").assert().success();
    for n in 0..3 {
        fs::write(env.root().join("f"), format!("v{n}")).unwrap();
        env.bolt().args(["save", &format!("s{n}")]).assert().success();
    }
    let key: [u8; KEY_LEN] = fs::read(env.config().join(keyed::KEY_FILE)).unwrap().try_into().unwrap();
    let repo = Repository::open(env.root()).unwrap();
    let mut line = Vec::new();
    let mut next = Some(support::main_tip(&env));
    while let Some(id) = next {
        let save = repo.store().save(&id, false).unwrap();
        line.push((id, save.change_id.0));
        next = save.parents.first().copied();
    }
    let tip = repo.store().save(&line[0].0, false).unwrap();
    let listing = tip.root;
    let content = repo.store().listing(&listing).unwrap().get("f").unwrap().id;
    drop(repo);
    for (what, stranger) in [("a listing", listing), ("content", content), ("nothing", Id([9; 32]))] {
        let mut lie = line.clone();
        lie.insert(1, (stranger, [0x11; 16]));
        for (who, mk) in [("new", None), ("main", Some(MAIN))] {
            let dir = tempfile::tempdir().unwrap();
            copy_dir(env.root(), dir.path());
            fs::write(dir.path().join(".bolt/cache/change-ids"), keyed::seal(&key, &body(&lie))).unwrap();
            let run = |args: &[&str]| {
                let mut cmd = match mk {
                    None => env.bolt_process_in(dir.path()),
                    Some(bin) => {
                        let mut c = Command::new(bin);
                        c.current_dir(dir.path())
                            .env("BOLT_USER_NAME", "Test")
                            .env("BOLT_USER_EMAIL", "test@example.com")
                            .env("BOLT_CONFIG_DIR", env.config())
                            .env("HOME", env.config());
                        c
                    }
                };
                said(&mut cmd, args)
            };
            println!("{what} {who} goto hex: {}", run(&["goto", &stranger.to_hex()]));
            println!("{what} {who} history: {}", run(&["history"]));
            println!("{what} {who} undo --list: {}", run(&["undo", "--list"]));
        }
    }
}
