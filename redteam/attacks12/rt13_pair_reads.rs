//! RT13 exploration (RT13-1): distinct tiny chunks stored in one pack in pairs, each pair in
//! another 64 KiB block than the one before (and shuffled, RT12-2's shape); `discard` writing it
//! out, timed, with the bytes the process read (`rchar`), on this build's release `bolt` and on
//! main's.
//! Run in the sandbox only: redteam/attacks12/run.sh explore rt13_pair_reads
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use std::process::Command;
use std::time::Instant;

use bolt_core::chunks::{Chunk, ChunkList};
use bolt_core::id::hash_content;
use support::{Env, copy_dir, text};

const NEW: &str = "/work/target/release/bolt";
const MAIN: &str = "/work/_main/target/release/bolt";

/// `bolt discard` in `dir`: seconds, and the bytes it read.
fn discard(env: &Env, bin: &str, dir: &std::path::Path) -> (f64, u64, String) {
    let t = Instant::now();
    let out = Command::new("sh")
        .arg("-c")
        .arg("\"$0\" discard >/tmp/rt13-said 2>&1; grep rchar /proc/$$/io; cat /tmp/rt13-said")
        .arg(bin)
        .current_dir(dir)
        .env("BOLT_USER_NAME", "Test")
        .env("BOLT_USER_EMAIL", "test@example.com")
        .env("BOLT_CONFIG_DIR", env.config())
        .env("HOME", env.config())
        .output()
        .unwrap();
    let said = text(&out.stdout);
    let rchar = said
        .lines()
        .find_map(|l| l.strip_prefix("rchar: "))
        .map_or(0, |n| n.trim().parse().unwrap());
    (t.elapsed().as_secs_f64(), rchar, said.lines().last().unwrap_or("").to_owned())
}

fn planted(chunks: usize, shape: &str) -> Env {
    let env = Env::empty();
    env.bolt().arg("start").assert().success();
    let bytes: Vec<Vec<u8>> = (0..chunks).map(|j| (j as u64).to_le_bytes().to_vec()).collect();
    let order: Vec<usize> = match shape {
        // A fixed scramble: every 7919th, wrapping.
        "shuffled" => (0..chunks).map(|j| (j * 7919) % chunks).collect(),
        // Pairs in columns of over 64 KiB: pair k+1 is a column (a block) away from pair k.
        "pairs" => {
            let columns = chunks / 2 / 600;
            (0..columns)
                .flat_map(|c| (c..chunks / 2).step_by(columns))
                .flat_map(|p| [2 * p, 2 * p + 1])
                .collect()
        }
        _ => (0..chunks).collect(),
    };
    assert_eq!(order.len(), chunks);
    support::add_pack(env.root(), |pack| {
        for &j in &order {
            pack.add_content(&bytes[j]).unwrap();
        }
    });
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
fn tiny_chunks_from_one_pack() {
    for (chunks, shuffled) in [
        (100_000, "pairs"),
        (100_000, "shuffled"),
        (400_000, "pairs"),
    ] {
        let env = planted(chunks, shuffled);
        for _ in 0..2 {
            for (name, bin) in [("new", NEW), ("main", MAIN)] {
                let dir = tempfile::tempdir().unwrap();
                copy_dir(env.root(), dir.path());
                let (secs, rchar, said) = discard(&env, bin, dir.path());
                println!(
                    "{chunks} chunks shuffled {shuffled}: {name} {secs:.2} s, read {} MB ({} B a chunk); {said}",
                    rchar / 1_000_000,
                    rchar / chunks as u64
                );
            }
        }
    }
}
