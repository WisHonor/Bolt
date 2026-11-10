//! Exploration: the import's work allowance against a hostile pack. A 64 KiB blob stored whole,
//! and 40 REF_DELTAs on it of about 8 KiB each (a few dozen bytes compressed), each building a
//! different file of just under 512 MiB by copying the blob 8,191 times: 20 GiB of rebuilding
//! from a pack of a few kilobytes. The deltas are listed under names of their own (so the test
//! needn't build them to name them); a tree names one, which starts the family's rebuild. The
//! import must stop at its allowance (4 GiB), in bounded time and memory, committing nothing.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod support;

use bolt_core::git::craft::{Craft, PackEntry, entry_header, zlib};
use bolt_core::git::{HashKind, ObjectType, object_id};
use support::{Env, noise, run_limited};

/// A delta from a `base_len`-byte base: `copies` copies of the whole base (each one byte, `0x80`:
/// offset 0, size 0x10000), then `tail` inserted.
fn bomb(base_len: usize, copies: usize, tail: &[u8]) -> Vec<u8> {
    let built = base_len * copies + tail.len();
    let mut delta = Vec::new();
    for mut n in [base_len, built] {
        loop {
            let byte = (n & 0x7f) as u8;
            n >>= 7;
            if n == 0 {
                delta.push(byte);
                break;
            }
            delta.push(byte | 0x80);
        }
    }
    delta.extend(std::iter::repeat_n(0x80u8, copies));
    delta.push(tail.len() as u8);
    delta.extend_from_slice(tail);
    delta
}

#[test]
fn a_pack_of_delta_bombs_stops_at_the_allowance() {
    let t = Env::empty();
    let git = Craft::new(t.root(), HashKind::Sha1);
    let base = noise(41, 64 << 10);
    let base_id = object_id(ObjectType::Blob, &base, HashKind::Sha1).unwrap();
    let mut entries = vec![PackEntry::Whole(ObjectType::Blob, base.clone())];
    for i in 0..40u32 {
        let delta = bomb(base.len(), 8191, format!("member {i}\n").as_bytes());
        let mut raw = entry_header(7, delta.len() as u64);
        raw.extend_from_slice(base_id.as_bytes());
        raw.extend_from_slice(&zlib(&delta));
        entries.push(PackEntry::Raw(raw));
    }
    // Craft lists a raw entry under the ID of a blob of its bytes.
    let first = match &entries[1] {
        PackEntry::Raw(raw) => object_id(ObjectType::Blob, raw, HashKind::Sha1).unwrap(),
        _ => unreachable!(),
    };
    let (pack, _) = git.pack(&entries);
    eprintln!("pack: {} bytes", std::fs::metadata(&pack).unwrap().len());
    let tree = git.tree(&[(b"100644", b"bomb.bin", first)]);
    let who = "A <a@b> 1700000000 +0000";
    let c = git.commit(
        format!("tree {}\nauthor {who}\ncommitter {who}\n\nbombs\n", tree.to_hex()).as_bytes(),
    );
    git.loose_ref("refs/heads/main", &format!("{}\n", c.to_hex()));

    let mut cmd = t.bolt_process_in(t.root());
    cmd.arg("import");
    let ended = run_limited(cmd, 900);
    eprintln!(
        "bolt import: code {:?}, timed out {}, {:.1} s, peak {} MiB\n{}",
        ended.code, ended.timed_out, ended.secs, ended.peak_mb, ended.stderr
    );
    assert_eq!(ended.code, Some(1));
    assert!(ended.stderr.contains("of work Bolt allows"), "{}", ended.stderr);
}
