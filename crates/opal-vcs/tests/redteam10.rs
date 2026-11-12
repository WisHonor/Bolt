//! Red-team proof tests for milestone 6, presets and polish (decision 024): each asserts the safe
//! behaviour and fails on the code the session attacked. See
//! docs/redteam/2026-11-10-milestone-6.md.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use opal_core::chunks::{Chunk, ChunkList};
use opal_core::id::hash_content;
use support::{Env, main_tip, noise, plant_save, text, top_listing};

/// A big file's first chunk is in no pack as content: a pack holds a chunk list under the chunk's
/// ID instead, whose two pieces build the chunk. Reading the file takes chunks as content only,
/// so `goto` can't write it; `verify` must not say the project is fine.
#[test]
fn rt10_1_verify_finds_a_chunk_stored_only_as_a_chunk_list() {
    let env = Env::empty();
    env.opal()
        .args(["start", "--preset", "custom"])
        .assert()
        .success();
    let first = main_tip(&env).to_hex();
    env.opal().args(["goto", &first]).assert().success();
    let big = noise(10, 3 << 20);
    let (list, chunks) = support::cut(&big);
    let first_chunk = chunks[0];
    let (a, b) = first_chunk.split_at(first_chunk.len() / 2);
    let as_list = ChunkList {
        file: hash_content(first_chunk),
        size: first_chunk.len() as u64,
        chunks: vec![
            Chunk {
                id: hash_content(a),
                size: a.len() as u64,
            },
            Chunk {
                id: hash_content(b),
                size: b.len() as u64,
            },
        ],
    };
    plant_save(env.root(), |p| {
        p.add_content(a).unwrap();
        p.add_content(b).unwrap();
        p.add_chunk_list(&as_list).unwrap();
        for chunk in &chunks[1..] {
            p.add_content(chunk).unwrap();
        }
        p.add_chunk_list(&list).unwrap();
        top_listing(p, &[("big.bin", list.file, big.len() as u64)])
    });

    let goto = env.opal().args(["goto", "main"]).output().unwrap();
    eprintln!(
        "opal goto main: {:?} {}",
        goto.status.code(),
        text(&goto.stderr).trim()
    );
    let verify = env.opal().arg("verify").output().unwrap();
    eprintln!(
        "opal verify: {:?} {}{}",
        verify.status.code(),
        text(&verify.stdout).trim(),
        text(&verify.stderr).trim()
    );
    assert!(
        goto.status.code() == Some(0) || verify.status.code() != Some(0),
        "goto can't write big.bin, and verify says the project is fine"
    );
}

/// A save whose listing gives a file a size its content doesn't have: `goto` can't write it.
#[test]
fn rt10_2_verify_finds_a_listing_whose_size_isnt_its_files() {
    let env = Env::empty();
    env.opal()
        .args(["start", "--preset", "custom"])
        .assert()
        .success();
    let first = main_tip(&env).to_hex();
    env.opal().args(["goto", &first]).assert().success();
    let small = b"hello\n";
    let big = noise(11, 3 << 20);
    plant_save(env.root(), |p| {
        let small_id = p.add_content(small).unwrap();
        let big_id = support::add_chunked(p, &big);
        top_listing(
            p,
            &[
                ("a.txt", small_id, 999),
                ("big.bin", big_id, big.len() as u64 + 1),
            ],
        )
    });
    let goto = env.opal().args(["goto", "main"]).output().unwrap();
    eprintln!(
        "opal goto main: {:?} {}",
        goto.status.code(),
        text(&goto.stderr).trim()
    );
    let verify = env.opal().arg("verify").output().unwrap();
    eprintln!(
        "opal verify: {:?} {}{}",
        verify.status.code(),
        text(&verify.stdout).trim(),
        text(&verify.stderr).trim()
    );
    assert!(
        goto.status.code() == Some(0) || verify.status.code() != Some(0),
        "goto can't write the save, and verify says the project is fine"
    );
}

/// A save whose folders describe more than 10,000,000 entries, through one listing reached by
/// many paths: every reader must refuse it, so `goto` does; `verify` reads each listing once.
#[test]
fn rt10_3_verify_finds_a_save_past_the_entry_limit() {
    use opal_core::listing::{Entry, EntryKind, Listing};
    use opal_core::pack::ObjectKind;
    let env = Env::empty();
    env.opal()
        .args(["start", "--preset", "custom"])
        .assert()
        .success();
    let first = main_tip(&env).to_hex();
    env.opal().args(["goto", &first]).assert().success();
    plant_save(env.root(), |p| {
        let x = p.add_content(b"x").unwrap();
        let mut listing = |n: usize, kind, id, size| {
            let entries = (0..n)
                .map(|i| Entry {
                    name: format!("{i:05}"),
                    kind,
                    id,
                    size,
                })
                .collect();
            let bytes = Listing::new(entries).unwrap().encode();
            p.add_record(ObjectKind::FolderListing, &bytes).unwrap()
        };
        // 10,001 folders of 1,000 files each: 10,011,001 entries.
        let folder = listing(1_000, EntryKind::File, x, 1);
        listing(10_001, EntryKind::Folder, folder, 0)
    });
    let goto = env.opal().args(["goto", "main"]).output().unwrap();
    eprintln!(
        "opal goto main: {:?} {}",
        goto.status.code(),
        text(&goto.stderr).trim()
    );
    let verify = env.opal().arg("verify").output().unwrap();
    eprintln!(
        "opal verify: {:?} {}{}",
        verify.status.code(),
        text(&verify.stdout).trim(),
        text(&verify.stderr).trim()
    );
    assert!(
        goto.status.code() == Some(0) || verify.status.code() != Some(0),
        "goto refuses the save, and verify says the project is fine"
    );
}
