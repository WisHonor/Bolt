//! Exploration: `bolt import` of a hostile delta family, stopped at every abort point and run
//! again. The family: a 2 MiB blob stored whole; two honest members built on it, one with two
//! members of its own; a member listed under another blob's name; a delta whose bytes are
//! damaged; a delta that builds on itself; and a REF_DELTA onto a blob only another pack holds.
//! The commits name only the honest blobs. Each run must end with the files a clean import gives,
//! and `.bolt/tmp` empty.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![cfg(feature = "fault-injection")]
mod support;

use std::fs;

use bolt_core::git::craft::{Craft, PackEntry, delta};
use bolt_core::git::{HashKind, ObjectType, object_id};
use support::{Env, main_tip, noise, saved_files, text};

fn edit(base: &[u8], at: usize, with: &[u8]) -> Vec<u8> {
    let mut v = base.to_vec();
    v[at..at + with.len()].copy_from_slice(with);
    v
}

#[test]
fn sweep_import_of_a_hostile_family() {
    let t = Env::empty();
    let git = Craft::new(t.root(), HashKind::Sha1);
    let id = |data: &[u8]| object_id(ObjectType::Blob, data, HashKind::Sha1).unwrap();
    let base = noise(31, 2 << 20);
    let m1 = edit(&base, 100, b"MEMBER-1");
    let m2 = edit(&base, 200, b"MEMBER-2");
    let m3 = edit(&m1, 300, b"MEMBER-3");
    let m4 = edit(&m1, 400, b"MEMBER-4");
    let other = noise(32, 2 << 20);
    let m6 = edit(&other, 600, b"MEMBER-6");
    // Another pack holds `other`, which entry 6 builds on by name.
    git.pack(&[PackEntry::Whole(ObjectType::Blob, other.clone())]);
    let mut damaged = delta(&base, &m2);
    let n = damaged.len();
    damaged[n - 3] ^= 0xff;
    git.pack_renamed(
        &[
            PackEntry::Whole(ObjectType::Blob, base.clone()),
            PackEntry::OfsDelta { base: 0, delta: delta(&base, &m1) },
            PackEntry::OfsDelta { base: 0, delta: delta(&base, &m2) },
            PackEntry::OfsDelta { base: 1, delta: delta(&m1, &m3) },
            PackEntry::OfsDelta { base: 1, delta: delta(&m1, &m4) },
            PackEntry::OfsDelta { base: 0, delta: damaged },
            PackEntry::RefDelta { base: id(&other), delta: delta(&other, &m6) },
            PackEntry::OfsDelta { base: 7, delta: delta(&base, &m1) },
        ],
        // Entry 2 builds m2 but is listed as `other`'s twin name: a member that isn't its name.
        &[(2, id(b"a name m2 never hashes to"))],
    );
    let tree1 = git.tree(&[(b"100644", b"a.bin", id(&m1)), (b"100644", b"base.bin", id(&base))]);
    let tree2 = git.tree(&[
        (b"100644", b"a.bin", id(&m3)),
        (b"100644", b"b.bin", id(&m4)),
        (b"100644", b"base.bin", id(&base)),
        (b"100644", b"other.bin", id(&other)),
    ]);
    // Neither Git nor Bolt reads a REF_DELTA onto another pack's object, so no tree names m6.
    let _ = m6;
    let who = "A <a@b> 1700000000 +0000";
    let c1 = git.commit(format!("tree {}\nauthor {who}\ncommitter {who}\n\none\n", tree1.to_hex()).as_bytes());
    let c2 = git.commit(
        format!("tree {}\nparent {}\nauthor {who}\ncommitter {who}\n\ntwo\n", tree2.to_hex(), c1.to_hex())
            .as_bytes(),
    );
    git.loose_ref("refs/heads/main", &format!("{}\n", c2.to_hex()));

    let expected = {
        let probe = Env::empty();
        support::copy_dir(t.root(), probe.root());
        let out = probe.bolt().arg("import").output().unwrap();
        eprintln!("clean import: {:?} {}", out.status.code(), text(&out.stderr));
        assert_eq!(out.status.code(), Some(0));
        saved_files(&probe, main_tip(&probe))
    };
    assert!(expected["a.bin"] == m3 && expected["b.bin"] == m4 && expected["other.bin"] == other);
    for n in 1..200 {
        let env = Env::empty();
        support::copy_dir(t.root(), env.root());
        let out = env
            .bolt()
            .arg("import")
            .env("BOLT_FAULT_AT", n.to_string())
            .output()
            .unwrap();
        let said = text(&out.stderr);
        let at = said
            .split_once("injected crash at ")
            .map(|(_, r)| r.lines().next().unwrap().to_owned());
        let litter: Vec<_> = fs::read_dir(env.root().join(".bolt/tmp"))
            .map(|d| d.map(|e| e.unwrap().file_name()).collect())
            .unwrap_or_default();
        let st = env.bolt().arg("status").output().unwrap();
        let again = env.bolt().arg("import").output().unwrap();
        let log = format!(
            "n={n} at={at:?} tmp after the stop: {} | status={:?} {} | import again={:?} {}",
            litter.len(),
            st.status.code(),
            text(&st.stderr).lines().next().unwrap_or(""),
            again.status.code(),
            text(&again.stderr).lines().next().unwrap_or("")
        );
        eprintln!("{log}");
        if at.is_none() {
            assert_eq!(out.status.code(), Some(0), "{said}");
            break;
        }
        assert!(env.root().join(".bolt/format").exists(), "not finished: {log}");
        let saved = saved_files(&env, main_tip(&env));
        assert!(saved == expected, "wrong files after {log}");
        let tmp: Vec<_> = fs::read_dir(env.root().join(".bolt/tmp"))
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert!(tmp.is_empty(), "litter in .bolt/tmp: {tmp:?} after {log}");
    }
}
