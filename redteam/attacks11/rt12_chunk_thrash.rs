//! RT12 exploration: a file whose chunk list takes its small chunks from more packs than a store
//! keeps open, in turn, so every chunk read opens a pack again; `discard` writing it, timed on
//! this build's release `bolt` and on main's. Also the same chunks from fewer packs, in order.
//! Run in the sandbox only: redteam/attacks11/run.sh explore rt12_chunk_thrash
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use std::process::Command;
use std::time::Instant;

use bolt_core::chunks::{Chunk, ChunkList};
use bolt_core::id::hash_content;
use support::{Env, copy_dir, text};

const NEW: &str = "/work/target/release/bolt";
const MAIN: &str = "/work/_main/target/release/bolt";

fn run(env: &Env, bin: &str, dir: &std::path::Path, args: &[&str]) -> (f64, String) {
    let t = Instant::now();
    let out = Command::new(bin)
        .args(args)
        .current_dir(dir)
        .env("BOLT_USER_NAME", "Test")
        .env("BOLT_USER_EMAIL", "test@example.com")
        .env("BOLT_CONFIG_DIR", env.config())
        .env("HOME", env.config())
        .output()
        .unwrap();
    let said = format!("{:?} {} {}", out.status.code(), text(&out.stdout), text(&out.stderr));
    (t.elapsed().as_secs_f64(), said.chars().take(200).collect())
}

fn planted(packs: usize, chunks: usize, chunk_len: usize, interleave: bool) -> Env {
    let env = Env::empty();
    env.bolt().arg("start").assert().success();
    let bytes: Vec<Vec<u8>> = (0..chunks)
        .map(|j| {
            let mut b = (j as u64).to_le_bytes().to_vec();
            b.resize(chunk_len, (j % 251) as u8);
            b
        })
        .collect();
    let per = chunks.div_ceil(packs);
    for p in 0..packs {
        support::add_pack(env.root(), |pack| {
            for (j, b) in bytes.iter().enumerate() {
                let here = if interleave { j % packs == p } else { j / per == p };
                if here {
                    pack.add_content(b).unwrap();
                }
            }
        });
    }
    let whole: Vec<u8> = bytes.concat();
    let list = ChunkList {
        file: hash_content(&whole),
        size: whole.len() as u64,
        chunks: bytes
            .iter()
            .map(|b| Chunk {
                id: hash_content(b),
                size: b.len() as u64,
            })
            .collect(),
    };
    support::plant_save(env.root(), |pack| {
        pack.add_chunk_list(&list).unwrap();
        support::top_listing(pack, &[("big", list.file, list.size)])
    });
    env
}

#[test]
fn chunks_from_many_packs_in_turn() {
    for (packs, chunks, len, interleave) in [
        (20, 100_000, 8, true),
        (20, 100_000, 8, false),
        (40, 100_000, 8, true),
        (20, 20_000, 3000, true),
        (20, 20_000, 3000, false),
    ] {
        let env = planted(packs, chunks, len, interleave);
        for (name, bin) in [("new", NEW), ("main", MAIN)] {
            let dir = tempfile::tempdir().unwrap();
            copy_dir(env.root(), dir.path());
            let (secs, said) = run(&env, bin, dir.path(), &["discard"]);
            let ok = std::fs::metadata(dir.path().join("big")).map(|m| m.len()).unwrap_or(0);
            println!(
                "packs {packs} chunks {chunks} x {len} B interleaved {interleave}: {name} {secs:.2} s, wrote {ok} B; {said:?}"
            );
        }
    }
}
