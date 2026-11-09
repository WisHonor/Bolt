//! Exploration: RT8-3's warning on each way the import reads a blob over 1,024 bytes that starts
//! with a pointer: streamed (over 64 MiB, stored whole by Git), read whole (under 64 MiB), and
//! rebuilt in a big file's delta family. Each must be named in the lfs_lookalikes warning.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod support;

use std::fs;

use bolt_core::git::craft::lfs_pointer_to;
use support::{Env, noise, text, write};

fn padded(pointer: &str, tail: &[u8]) -> Vec<u8> {
    let mut blob = pointer.as_bytes().to_vec();
    blob.resize(1024, b' ');
    blob.extend_from_slice(tail);
    blob
}

#[test]
fn every_read_path_warns() {
    let env = Env::new();
    env.commit(
        ".gitattributes",
        "*.bin filter=lfs diff=lfs merge=lfs -text\n",
        "Track",
    );
    let (pointer, at) = lfs_pointer_to(b"the picture\n");
    let at = env.root().join(".git").join(at);
    fs::create_dir_all(at.parent().unwrap()).unwrap();
    fs::write(at, b"the picture\n").unwrap();
    // Streamed: 65 MiB, stored whole by Git (random tail, so no delta).
    write(&env, "streamed.bin", padded(&pointer, &noise(1, 65 << 20)));
    // Read whole: 3 MiB.
    write(&env, "whole.bin", padded(&pointer, &noise(2, 3 << 20)));
    // A family: two versions of a 4 MiB look-alike, packed so Git keeps one as a delta.
    let mut v = padded(&pointer, &noise(3, 4 << 20));
    write(&env, "family.bin", &v);
    env.commit("readme.txt", "one\n", "One");
    v[2_000_000..2_000_008].copy_from_slice(b"EDIT-ONE");
    write(&env, "family.bin", &v);
    env.commit("readme.txt", "two\n", "Two");
    env.git(&["repack", "-a", "-d", "-f", "-q"]);
    let idx = fs::read_dir(env.root().join(".git/objects/pack"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| p.extension().is_some_and(|x| x == "idx"))
        .unwrap();
    let verify = env.git(&["verify-pack", "-v", idx.to_str().unwrap()]);
    let deltas = verify.lines().filter(|l| l.split_whitespace().count() >= 7).count();
    eprintln!("objects stored as deltas: {deltas}");

    let out = env.bolt().args(["--json", "import"]).output().unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    let doc: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let said: Vec<String> = doc["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| w["message"].as_str().unwrap_or("").to_owned())
        .collect();
    eprintln!("warnings: {said:#?}");
    for name in ["streamed.bin", "whole.bin", "family.bin"] {
        let n = said.iter().map(|m| m.matches(&format!("`{name}`")).count()).sum::<usize>();
        eprintln!("{name}: named {n} times");
    }
}
