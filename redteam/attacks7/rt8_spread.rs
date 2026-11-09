//! Exploration: reading a big file whose chunks are spread over more packs than stay open.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod support;

use std::time::Instant;

use bolt_core::repo::Repository;
use support::{Env, add_pack, cut, noise};

fn read_time(env: &Env, id: &bolt_core::id::Id) -> f64 {
    let repo = Repository::open(env.root()).unwrap();
    let start = Instant::now();
    repo.store().copy_content(id, &mut std::io::sink()).unwrap();
    start.elapsed().as_secs_f64()
}

fn time_with(packs: usize, big: &[u8]) -> f64 {
    let env = Env::empty();
    env.bolt().arg("start").assert().success();
    let (list, chunks) = cut(big);
    for p in 0..packs {
        add_pack(env.root(), |pack| {
            for (i, c) in chunks.iter().enumerate() {
                if i % packs == p {
                    pack.add_content(c).unwrap();
                }
            }
        });
    }
    add_pack(env.root(), |pack| pack.add_chunk_list(&list).unwrap());
    // Twice: the first read checks every copy, the second finds them known good.
    let first = read_time(&env, &list.file);
    let second = read_time(&env, &list.file);
    eprintln!(
        "{} chunks over {packs} packs: {first:.2} s, again {second:.2} s",
        list.chunks.len()
    );
    first
}

#[test]
fn explore_chunks_spread_over_many_packs() {
    let big = noise(9, 128 << 20);
    for packs in [1, 16, 17, 40, 80] {
        time_with(packs, &big);
    }
}
