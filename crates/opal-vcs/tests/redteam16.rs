//! Red-team session RT16 (decision 024): format 1's one-frame zstd rule, attacked in the sandbox.
//! Each test asserts the safe behaviour, so it fails until its finding is fixed. See
//! docs/redteam/2026-11-12-v0.1-final.md.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use std::path::Path;
use std::time::{Duration, SystemTime};

use opal_core::id::{Flavor, Id, hash_content, hash_record};
use opal_core::pack::{ENTRY_HEADER_LEN, ObjectKind, PackReader, PackWriter, Prepared, prepare};
use support::{Env, add_pack, text};

const MAGIC: [u8; 4] = [0x28, 0xB5, 0x2F, 0xFD];

/// A window byte: 2^`log` bytes.
fn window(log: u8) -> u8 {
    (log - 10) << 3
}

/// A 3-byte block header: `kind` 0 raw, 1 RLE, 2 compressed.
fn block(frame: &mut Vec<u8>, kind: u32, size: usize, last: bool) {
    let size = u32::try_from(size).unwrap();
    frame.extend_from_slice(&(size << 3 | kind << 1 | u32::from(last)).to_le_bytes()[..3]);
}

/// A frame with a window of 2^21 and its content size, made of one RLE block of `size` bytes.
fn one_rle_block(byte: u8, size: usize) -> Vec<u8> {
    let mut frame = MAGIC.to_vec();
    frame.extend([0x80, window(21)]);
    frame.extend_from_slice(&u32::try_from(size).unwrap().to_le_bytes());
    block(&mut frame, 1, size, true);
    frame.push(byte);
    frame
}

/// Opalro's own frame of `bytes` (one segment), its header rewritten to give a window of
/// 2^`log` bytes and the content size; the blocks are zstd's, as they are.
fn with_window(bytes: &[u8], log: u8) -> Vec<u8> {
    let made = prepare(hash_content(bytes), bytes).unwrap();
    assert!(made.zstd);
    let frame = made.stored;
    let descriptor = frame[4];
    assert_ne!(descriptor & 0x20, 0, "one segment");
    let size_field = [1, 2, 4, 8][usize::from(descriptor >> 6)];
    let mut out = MAGIC.to_vec();
    out.extend([0x80 | (descriptor & 0x04), window(log)]);
    out.extend_from_slice(&u32::try_from(bytes.len()).unwrap().to_le_bytes());
    out.extend_from_slice(&frame[5 + size_field..]);
    out
}

fn noise(len: usize, seed: u64) -> Vec<u8> {
    let mut x = seed | 1;
    (0..len)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            (x >> 24) as u8
        })
        .collect()
}

/// 10 KiB of letters, then their last KiB again up to 70 KiB: zstd makes it one compressed
/// block, literals then matches 1 KiB back.
fn seventy_kib() -> Vec<u8> {
    let alphabet = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut v: Vec<u8> = noise(10 << 10, 29)
        .into_iter()
        .map(|b| alphabet[usize::from(b & 63)])
        .collect();
    while v.len() < 70 << 10 {
        let at = v.len() - 1024;
        let piece = v[at..at + 1024].to_vec();
        v.extend_from_slice(&piece);
    }
    v
}

/// The frame of `seventy_kib`, its window made 64 KiB: its one block, compressed in 7.5 KiB,
/// decodes to 70 KiB, more than RFC 8878 lets a block hold in that frame.
fn last_block_past_its_limit() -> (Vec<u8>, Vec<u8>) {
    let content = seventy_kib();
    let frame = with_window(&content, 16);
    let header = u32::from_le_bytes([frame[10], frame[11], frame[12], 0]);
    assert_eq!((header >> 1) & 3, 2, "a compressed block");
    assert_eq!(header & 1, 1, "the last block");
    assert!(header >> 3 < 64 << 10, "stored within the limit");
    (content, frame)
}

fn project() -> Env {
    let env = Env::empty();
    env.opalro()
        .args(["start", "--preset", "custom"])
        .assert()
        .success();
    env
}

/// Puts `frame` in the store as the only copy of `content`.
fn plant(root: &Path, content: &[u8], frame: &[u8]) -> Vec<std::path::PathBuf> {
    let p = Prepared {
        id: hash_content(content),
        size: content.len() as u64,
        zstd: true,
        stored: frame.to_vec(),
    };
    add_pack(root, |pack| {
        pack.add_prepared(ObjectKind::Content, &p).unwrap()
    })
}

/// Saves `content` as `name` once `frame` is its only copy, then runs `opalro verify`.
fn verify_with(content: &[u8], frame: &[u8]) -> (Option<i32>, String) {
    let env = project();
    plant(env.root(), content, frame);
    std::fs::write(env.root().join("f.bin"), content).unwrap();
    env.opalro().args(["save", "One file"]).assert().success();
    let out = env.opalro().arg("verify").output().unwrap();
    (out.status.code(), text(&out.stdout))
}

fn assert_damaged(what: &str, content: &[u8], frame: &[u8]) {
    let (code, shown) = verify_with(content, frame);
    let short = &hash_content(content).to_string()[..12];
    assert!(
        code == Some(1) && shown.contains(short) && shown.contains("damaged"),
        "{what}: format 1 refuses this frame, yet the save took it as the file's copy and \
         `opalro verify` passed it ({code:?}):\n{shown}"
    );
}

/// Each copy of `id` in the store, with its stored bytes.
fn stored_copies(root: &Path, id: &Id) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    for f in std::fs::read_dir(root.join(".opalro/store/packs")).unwrap() {
        let path = f.unwrap().path();
        let bytes = std::fs::read(&path).unwrap();
        for e in PackReader::open(&path).unwrap().entries() {
            let e = e.unwrap();
            if e.id == *id {
                let at = usize::try_from(e.offset + ENTRY_HEADER_LEN).unwrap();
                out.push(bytes[at..at + usize::try_from(e.stored_size).unwrap()].to_vec());
            }
        }
    }
    out
}

/// RT16-1, as briefed (1eaa3b1): an RLE block of 200 KiB, over the 128 KiB any block may hold.
/// The code review's walk of the blocks (69b72d0) refuses it.
#[test]
fn rt16_1_a_small_file_whose_rle_block_is_over_its_limit_is_damaged() {
    let content = vec![b'A'; 200 << 10];
    assert_damaged(
        "an RLE block of 200 KiB",
        &content,
        &one_rle_block(b'A', content.len()),
    );
}

/// RT16-1: a compressed block is walked by its stored size only. Damage by the spec as briefed;
/// the code review's ruling 10 (69b72d0) lets a reader read it, so this goes if that is confirmed.
#[test]
#[ignore = "RT16-1, ruling 10: a compressed block that unpacks past its limit is read in one call; ruled on, not fixed"]
fn rt16_1_a_small_file_whose_last_block_decodes_past_its_limit_is_damaged() {
    let (content, frame) = last_block_past_its_limit();
    assert_damaged(
        "a last block of 70 KiB in a 64 KiB window",
        &content,
        &frame,
    );
}

/// RT16-5: the same stored bytes, read as a folder listing (one call: stored in 8 KiB) and refused
/// as a file (streamed: recorded over 1 MiB), where the spec said v0.1 reads such a block in any
/// entry stored in at most 1 MiB. 848f7a3's wording allows this, so this goes if that stays.
#[test]
#[ignore = "RT16-5, ruling 20: the spec says a file's one-call read goes by its unpacked size too; kept as the record"]
fn rt16_5_the_same_frame_is_read_as_a_listing_and_refused_as_a_file() {
    let (tail, small) = last_block_past_its_limit();
    let mut content = vec![b'Z'; 1 << 20];
    content.extend_from_slice(&tail);
    // 16 RLE blocks of 64 KiB, then the 70 KiB block as the last.
    let mut frame = MAGIC.to_vec();
    frame.extend([0x80, window(16)]);
    frame.extend_from_slice(&u32::try_from(content.len()).unwrap().to_le_bytes());
    for _ in 0..16 {
        block(&mut frame, 1, 64 << 10, false);
        frame.push(b'Z');
    }
    frame.extend_from_slice(&small[10..]);
    let dir = tempfile::tempdir().unwrap();
    let mut w = PackWriter::create(dir.path()).unwrap();
    let as_listing = hash_record(Flavor::FolderListing, &content);
    for (kind, id) in [
        (ObjectKind::FolderListing, as_listing),
        (ObjectKind::Content, hash_content(&content)),
    ] {
        let p = Prepared {
            id,
            size: content.len() as u64,
            zstd: true,
            stored: frame.clone(),
        };
        w.add_prepared(kind, &p).unwrap();
    }
    let path = w.finish(dir.path()).unwrap().unwrap();
    let r = PackReader::open(&path).unwrap();
    let mut listing = None;
    let mut file = None;
    for e in r.entries() {
        let e = e.unwrap();
        if e.id == as_listing {
            listing = Some(
                r.read_record(&e)
                    .map(|b| b.len())
                    .map_err(|e| e.to_string()),
            );
        } else {
            let mut out = Vec::new();
            file = Some(
                r.copy_content(&e, &mut out)
                    .map(|()| out.len())
                    .map_err(|e| e.to_string()),
            );
        }
    }
    let (listing, file) = (listing.unwrap(), file.unwrap());
    assert_eq!(
        listing.is_ok(),
        file.is_ok(),
        "one frame ({} B stored), two answers:\n  as a listing: {listing:?}\n  as a file: {file:?}",
        frame.len()
    );
}

/// RT16-4: with the person's own copy in the store, `tidy` keeps a planted one that the streamed
/// decoder refuses (its pack's name sorts first) and removes theirs.
#[test]
#[ignore = "RT16-4, ruling 19: tidy may keep a planted copy over the person's own; ruled on, not fixed in v0.1"]
fn rt16_4_tidy_keeps_the_persons_own_copy_over_a_planted_one() {
    let env = project();
    let (content, frame) = last_block_past_its_limit();
    std::fs::write(env.root().join("f.bin"), &content).unwrap();
    env.opalro().args(["save", "Mine"]).assert().success();
    let packs = env.root().join(".opalro/store/packs");
    for made in plant(env.root(), &content, &frame) {
        std::fs::rename(made, packs.join("00.pack")).unwrap();
    }
    let old = SystemTime::now() - Duration::from_secs(2 * 24 * 3600);
    for f in std::fs::read_dir(&packs).unwrap() {
        let file = std::fs::File::options()
            .write(true)
            .open(f.unwrap().path())
            .unwrap();
        file.set_modified(old).unwrap();
    }
    env.opalro().arg("tidy").assert().success();
    let copies = stored_copies(env.root(), &hash_content(&content));
    let verify = env.opalro().arg("verify").output().unwrap();
    assert!(
        copies.iter().any(|c| *c != frame),
        "after tidy, the file's only copy is the planted frame ({} copies); verify says:\n{}",
        copies.len(),
        text(&verify.stdout)
    );
}

/// RT16-2, as briefed (1eaa3b1): an 8-byte content size of 2^64 - 1, which zstd reads as none.
/// The code review's fix (69b72d0) refuses it.
#[test]
fn rt16_2_a_content_size_of_all_ones_is_damage() {
    let content = noise(1000, 17);
    let mut frame = MAGIC.to_vec();
    frame.extend([0xC0, window(21)]);
    frame.extend_from_slice(&[0xFF; 8]);
    block(&mut frame, 0, content.len(), true);
    frame.extend_from_slice(&content);
    assert_damaged("a content size of 2^64 - 1", &content, &frame);
}

/// RT16-3: matches 450 KiB back in a frame whose window is 128 KiB.
#[test]
#[ignore = "RT16-3, ruling 18: a match past the window is read where zstd reads it; ruled on, not fixed"]
fn rt16_3_matches_past_the_window_are_damage() {
    let half = noise(450 << 10, 13);
    let mut content = half.clone();
    content.extend_from_slice(&half);
    let frame = with_window(&content, 17);
    assert!(
        frame.len() < content.len() * 6 / 10,
        "zstd found the far match"
    );
    assert_damaged("matches 450 KiB back, window 128 KiB", &content, &frame);
}
