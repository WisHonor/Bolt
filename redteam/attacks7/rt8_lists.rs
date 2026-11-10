//! Exploration: two chunk lists for one file through merging and tidy (red-team milestone 5).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod support;

use std::fs;
use std::time::{Duration, SystemTime};

use bolt_core::chunks::{Chunk, ChunkList};
use bolt_core::id::hash_content;
use support::{Env, add_list_record, add_pack, cut, noise, text};

fn ok(env: &Env, args: &[&str]) -> String {
    let out = env.bolt().args(args).output().unwrap();
    assert_eq!(out.status.code(), Some(0), "{args:?}: {}", text(&out.stderr));
    text(&out.stdout) + &text(&out.stderr)
}

fn reads_back(env: &Env, bytes: &[u8]) -> bool {
    let repo = bolt_core::repo::Repository::open(env.root()).unwrap();
    let mut out = Vec::new();
    repo.store()
        .copy_content(&hash_content(bytes), &mut out)
        .is_ok()
        && out == bytes
}

fn packs(env: &Env) -> Vec<String> {
    let mut v: Vec<String> = fs::read_dir(env.root().join(".bolt/store/packs"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    v.sort();
    v
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

/// A list for `big` naming its chunks in another order: it decodes, its sizes add up, and it
/// builds another file.
fn shuffled(big: &[u8]) -> ChunkList {
    let (mut list, _) = cut(big);
    list.chunks.reverse();
    list
}

/// A list for `big` naming a chunk no pack holds.
fn missing(big: &[u8]) -> ChunkList {
    let (mut list, _) = cut(big);
    let last = list.chunks.len() - 1;
    list.chunks[last] = Chunk {
        id: hash_content(b"nowhere"),
        size: list.chunks[last].size,
    };
    list
}

fn run(bad: ChunkList, big: &[u8]) {
    let env = Env::empty();
    ok(&env, &["start"]);
    fs::write(env.root().join("big.bin"), big).unwrap();
    ok(&env, &["save", "Big"]);
    // The bad list in a pack named to come first.
    let planted = add_pack(env.root(), |pack| {
        add_list_record(pack, bad.file, bad.size, bad.encode());
    });
    let first = env.root().join(".bolt/store/packs/0000-first.pack");
    fs::rename(&planted[0], &first).unwrap();
    eprintln!("before merge: reads back {}", reads_back(&env, big));
    // Enough small packs for a merge.
    for i in 0..40 {
        fs::write(env.root().join("n.txt"), format!("{i}")).unwrap();
        ok(&env, &["save", &format!("S{i}")]);
    }
    eprintln!("packs after merges: {}", packs(&env).len());
    eprintln!("after merge: reads back {}", reads_back(&env, big));
    fs::remove_file(env.root().join("big.bin")).unwrap();
    let said = ok(&env, &["discard"]);
    eprintln!("discard: {}", said.lines().next().unwrap_or(""));
    let back = fs::read(env.root().join("big.bin")).unwrap() == big;
    eprintln!("discard wrote the right bytes: {back}");
    age(&env);
    let said = ok(&env, &["tidy"]);
    eprintln!("tidy: {}", said.replace('\n', " | "));
    eprintln!("after tidy: reads back {}", reads_back(&env, big));
    fs::remove_file(env.root().join("big.bin")).unwrap();
    let said = ok(&env, &["discard"]);
    let back = fs::read(env.root().join("big.bin")).unwrap() == big;
    eprintln!(
        "discard after tidy wrote the right bytes: {back} ({})",
        said.lines().next().unwrap_or("")
    );
    assert!(back);
}

#[test]
fn explore_a_shuffled_list_first() {
    let big = noise(5, 3 << 20);
    run(shuffled(&big), &big);
}

#[test]
fn explore_a_list_naming_a_missing_chunk_first() {
    let big = noise(6, 3 << 20);
    run(missing(&big), &big);
}
