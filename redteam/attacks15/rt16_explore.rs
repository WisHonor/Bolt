//! RT16 exploration (decision 024), sandbox only: hostile zstd frames through both decoders.
//! Not a proof: it prints what each path does.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::cast_possible_truncation
)]

use super::*;
use crate::id::{Flavor, hash_content, hash_record};
use std::time::Instant;
use zstd::stream::raw::CParameter;

const MAGIC: [u8; 4] = [0x28, 0xB5, 0x2F, 0xFD];
const RAW: u32 = 0;
const RLE: u32 = 1;

pub(super) fn wd(log: u32) -> u8 {
    ((log - 10) << 3) as u8
}

pub(super) fn header(fhd: u8, window: Option<u8>, fcs: &[u8]) -> Vec<u8> {
    let mut f = MAGIC.to_vec();
    f.push(fhd);
    f.extend(window);
    f.extend_from_slice(fcs);
    f
}

pub(super) fn block(f: &mut Vec<u8>, kind: u32, size: usize, last: bool) {
    let v = ((size as u32) << 3) | (kind << 1) | u32::from(last);
    f.extend_from_slice(&v.to_le_bytes()[..3]);
}

pub(super) fn raw_blocks(f: &mut Vec<u8>, data: &[u8], per: usize) {
    if data.is_empty() {
        block(f, RAW, 0, true);
        return;
    }
    let n = data.len().div_ceil(per);
    for (i, c) in data.chunks(per).enumerate() {
        block(f, RAW, c.len(), i + 1 == n);
        f.extend_from_slice(c);
    }
}

pub(super) fn rle_blocks(f: &mut Vec<u8>, byte: u8, total: usize, per: usize) {
    if total == 0 {
        block(f, RAW, 0, true);
        return;
    }
    let mut left = total;
    while left > 0 {
        let n = left.min(per);
        left -= n;
        block(f, RLE, n, left == 0);
        f.push(byte);
    }
}

pub(super) fn random(n: usize, seed: u64) -> Vec<u8> {
    let mut x = seed | 1;
    (0..n)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            (x >> 24) as u8
        })
        .collect()
}

/// A frame of `data` from zstd's streaming encoder: no content size, a window byte at 5.
pub(super) fn streamed(data: &[u8], level: i32, params: &[CParameter]) -> Vec<u8> {
    let mut enc = zstd::stream::write::Encoder::new(Vec::new(), level).unwrap();
    for p in params {
        enc.set_parameter(*p).unwrap();
    }
    std::io::Write::write_all(&mut enc, data).unwrap();
    enc.finish().unwrap()
}

fn dirs() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let root = tempfile::tempdir().unwrap();
    let tmp = root.path().join("tmp");
    let packs = root.path().join("packs");
    std::fs::create_dir(&tmp).unwrap();
    std::fs::create_dir(&packs).unwrap();
    (root, tmp, packs)
}

pub(super) fn plant(
    kind: ObjectKind,
    id: Id,
    size: u64,
    frame: &[u8],
) -> (tempfile::TempDir, PackReader, IndexEntry) {
    let (root, tmp, packs) = dirs();
    let p = Prepared {
        id,
        size,
        zstd: true,
        stored: frame.to_vec(),
    };
    let mut w = PackWriter::create(&tmp).unwrap();
    w.add_prepared(kind, &p).unwrap();
    let path = w.finish(&packs).unwrap().unwrap();
    let r = PackReader::open(&path).unwrap();
    let e = r.entries().next().unwrap().unwrap();
    (root, r, e)
}

fn told(e: &PackError, id: &Id) -> String {
    let s = match e {
        PackError::Corrupt { reason, .. } => format!("damaged: {reason}"),
        e => e.to_string(),
    };
    s.replace(&id.to_string(), "<id>")
}

/// What each path makes of `frame` stored for `content`: as content and as a folder listing.
pub(super) fn verdict(name: &str, content: &[u8], frame: &[u8]) -> (String, String) {
    let n = content.len() as u64;
    let cid = hash_content(content);
    let (_d, r, e) = plant(ObjectKind::Content, cid, n, frame);
    let t = Instant::now();
    let small = match r.read_small(&e) {
        Ok(Some(b)) if b == content => "ACCEPT".to_string(),
        Ok(Some(_)) => "ACCEPT-WRONG-BYTES".to_string(),
        Ok(None) => "none".to_string(),
        Err(e) => format!("err {}", told(&e, &cid)),
    };
    let t_small = t.elapsed();
    let mut out = Vec::new();
    let t = Instant::now();
    let streamed = match r.decode_into(&e, &mut out) {
        Ok(()) if out == content => "ACCEPT".to_string(),
        Ok(()) => "ACCEPT-WRONG-BYTES".to_string(),
        Err(e) => told(&e, &cid),
    };
    let t_stream = t.elapsed();
    drop(out);
    let mut out = Vec::new();
    let copy = match r.copy_content(&e, &mut out) {
        Ok(()) => "ACCEPT".to_string(),
        Err(e) => told(&e, &cid),
    };
    drop(out);
    let rec = if n <= MAX_RECORD_SIZE {
        let rid = hash_record(Flavor::FolderListing, content);
        let (_d2, r2, e2) = plant(ObjectKind::FolderListing, rid, n, frame);
        match r2.read_record(&e2) {
            Ok(_) => "ACCEPT".to_string(),
            Err(e) => told(&e, &rid),
        }
    } else {
        "-".to_string()
    };
    println!(
        "{name}\n    stored={} size={n} one_plain={} read_small={small} [{t_small:.1?}] \
         decode_into={streamed} [{t_stream:.1?}]\n    copy_content={copy}\n    read_record={rec}",
        frame.len(),
        one_plain_frame(frame, n),
    );
    (copy, rec)
}

#[test]
fn rt16_frames() {
    let k = 1024usize;
    let fcs4 = |n: usize| (n as u32).to_le_bytes();
    let mut cases: Vec<(String, Vec<u8>, Vec<u8>)> = Vec::new();

    // Blocks over Block_Maximum_Size (the smaller of the window and 128 KiB).
    for (n, fcs) in [
        (200 * k, true),
        (200 * k, false),
        (900 * k, true),
        (1200 * k, true),
    ] {
        let content = vec![0x41u8; n];
        let mut f = if fcs {
            header(0x80, Some(wd(21)), &fcs4(n))
        } else {
            header(0x00, Some(wd(21)), &[])
        };
        rle_blocks(&mut f, 0x41, n, 300 * k);
        cases.push((
            format!("RLE blocks of 300 KiB, window 2^21, fcs={fcs}, {n} B"),
            content,
            f,
        ));
    }
    {
        let n = 200 * k;
        let content = vec![0x41u8; n];
        let mut f = header(0xA0, None, &fcs4(n));
        rle_blocks(&mut f, 0x41, n, n);
        cases.push((
            "RLE one block of 200 KiB, single segment".into(),
            content,
            f,
        ));
    }
    for n in [200 * k, 1200 * k] {
        let content = random(n, 7);
        let mut f = header(0x80, Some(wd(21)), &fcs4(n));
        raw_blocks(&mut f, &content, 200 * k);
        cases.push((
            format!("raw blocks of 200 KiB, window 2^21, {n} B"),
            content,
            f,
        ));
    }
    for n in [64 * k, 1100 * k] {
        let content = random(n, 9);
        let mut f = header(0x80, Some(wd(10)), &fcs4(n));
        raw_blocks(&mut f, &content, 64 * k);
        cases.push((
            format!("raw blocks of 64 KiB, window 1 KiB, {n} B"),
            content,
            f,
        ));
    }
    {
        let n = 100 * k;
        let content = random(n, 11);
        let mut f = header(0x00, Some(wd(10)), &[]);
        raw_blocks(&mut f, &content, 2 * k);
        cases.push((
            "raw blocks of 2 KiB, window 1 KiB, no fcs".into(),
            content,
            f,
        ));
    }
    // Compressed blocks decoding past the window: zstd's frame, its window byte made smaller.
    for n in [256 * k, 2048 * k] {
        let content = b"0123456789abcdef".repeat(n / 16);
        let mut f = streamed(&content, 3, &[]);
        assert_eq!(f[4] & 0x20, 0);
        f[5] = wd(16);
        cases.push((
            format!("compressed 128 KiB blocks, window byte -> 64 KiB, {n} B"),
            content,
            f,
        ));
    }
    // Offsets past the window: R ++ R, the window byte made 128 KiB.
    for half in [200 * k, 450 * k, 700 * k] {
        let r = random(half, 13);
        let mut content = r.clone();
        content.extend_from_slice(&r);
        let mut f = streamed(
            &content,
            19,
            &[
                CParameter::WindowLog(23),
                CParameter::EnableLongDistanceMatching(true),
            ],
        );
        let ok_len = f.len();
        assert_eq!(f[4] & 0x20, 0);
        f[5] = wd(17);
        cases.push((
            format!(
                "offsets of {half} B, window byte -> 128 KiB, {} B (frame {ok_len})",
                2 * half
            ),
            content,
            f,
        ));
    }
    // A content size of all ones, which zstd reads as none.
    for n in [1000usize, 1200 * k] {
        let content = random(n, 17);
        let mut f = header(0xC0, Some(wd(21)), &[0xFF; 8]);
        raw_blocks(&mut f, &content, 128 * k);
        cases.push((
            format!("content size 2^64-1 (8-byte field), {n} B"),
            content.clone(),
            f,
        ));
        let mut f = header(
            0xC0,
            Some(wd(21)),
            &[0xFE, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF],
        );
        raw_blocks(&mut f, &content, 128 * k);
        cases.push((
            format!("content size 2^64-2 (8-byte field), {n} B"),
            content,
            f,
        ));
    }
    // Good frames: Opalro's own two ways, and the spec's boundaries.
    for n in [0usize, 1, 1000, 1 << 20, (1 << 20) + 1] {
        let content = b"plain frame ".repeat(n / 12 + 1)[..n].to_vec();
        cases.push((
            format!("GOOD bulk::compress, {n} B"),
            content.clone(),
            zstd::bulk::compress(&content, 3).unwrap(),
        ));
        cases.push((
            format!("GOOD streamed, {n} B"),
            content.clone(),
            streamed(&content, 3, &[]),
        ));
    }
    {
        let n = 300 * k;
        let content = random(n, 19);
        let mut f = header(0x80, Some(wd(27)), &fcs4(n));
        raw_blocks(&mut f, &content, 128 * k);
        cases.push((
            "GOOD window exactly 2^27, raw 128 KiB blocks".into(),
            content.clone(),
            f,
        ));
        let mut f = header(0x00, Some(wd(27)), &[]);
        raw_blocks(&mut f, &content, 128 * k);
        cases.push(("GOOD window exactly 2^27, no fcs".into(), content, f));
        let content = random(1000, 23);
        let mut f = header(0xC0, Some(wd(21)), &1000u64.to_le_bytes());
        raw_blocks(&mut f, &content, 128 * k);
        cases.push(("GOOD 8-byte content size 1000".into(), content.clone(), f));
        let mut f = header(0x40, Some(wd(21)), &(1000u16 - 256).to_le_bytes());
        raw_blocks(&mut f, &content, 128 * k);
        cases.push(("GOOD 2-byte content size 1000".into(), content, f));
    }

    for (name, content, frame) in &cases {
        verdict(name, content, frame);
    }
}

/// Bytes from a 64-letter alphabet: little for LZ to find, but Huffman packs them.
fn letters(n: usize, seed: u64) -> Vec<u8> {
    random(n, seed)
        .into_iter()
        .map(|b| {
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/"[usize::from(b & 63)]
        })
        .collect()
}

/// `lit` bytes of letters, then the last KiB of them again until `total`: zstd makes one
/// compressed block (literals, then matches at 1 KiB) when `total` is at most 128 KiB.
fn lits_then_match(lit: usize, total: usize, seed: u64) -> Vec<u8> {
    let mut v = letters(lit, seed);
    while v.len() < total {
        let at = v.len() - 1024;
        let piece: Vec<u8> = v[at..at + 1024.min(total - v.len())].to_vec();
        v.extend_from_slice(&piece);
    }
    v
}

#[test]
fn rt16_fix() {
    let k = 1024usize;
    let mut cases: Vec<(String, Vec<u8>, Vec<u8>)> = Vec::new();
    // A compressed block that decodes past the limit: the walk sees only its stored size.
    for (lit, total, log) in [
        (20 * k, 70 * k, 16),
        (40 * k, 100 * k, 16),
        (10 * k, 70 * k, 16),
        (60 * k, 128 * k, 16),
        (5 * k, 40 * k, 15),
        (20 * k, 120 * k, 16),
    ] {
        let content = lits_then_match(lit, total, 29);
        let mut f = streamed(&content, 3, &[]);
        assert_eq!(f[4] & 0x20, 0);
        let blocks_from_zstd = f[5];
        f[5] = wd(log);
        // The one block's header, after the 6-byte frame header.
        let h = u32::from_le_bytes([f[6], f[7], f[8], 0]);
        cases.push((
            format!(
                "compressed last block: {lit} B literals, decodes {total} B, window byte {blocks_from_zstd:#x} -> 2^{log}; block type {} stored {} last {}",
                (h >> 1) & 3,
                h >> 3,
                h & 1
            ),
            content,
            f,
        ));
    }
    // A block header across the decoder's 131,075-byte read, and blocks exactly at the limit.
    {
        let content = random(1100 * k, 31);
        let mut f = header(0x80, Some(wd(21)), &(content.len() as u32).to_le_bytes());
        block(&mut f, RAW, 131_064, false);
        f.extend_from_slice(&content[..131_064]);
        raw_blocks(&mut f, &content[131_064..], 128 * k);
        cases.push(("GOOD raw blocks, a header across 131075".into(), content, f));
        let content = vec![7u8; 128 * k];
        let mut f = header(0x00, Some(wd(17)), &[]);
        rle_blocks(&mut f, 7, content.len(), 128 * k);
        cases.push(("GOOD one RLE block of exactly 128 KiB".into(), content, f));
        let content = vec![7u8; 128 * k + 1];
        let mut f = header(0x00, Some(wd(21)), &[]);
        rle_blocks(&mut f, 7, content.len(), 128 * k + 1);
        cases.push(("RLE block of 128 KiB + 1".into(), content, f));
        let content = vec![7u8; 1000];
        let mut f = header(0x20, None, &[232]);
        rle_blocks(&mut f, 7, 232, 232);
        cases.push((
            "GOOD single segment 1-byte size 232, RLE 232".into(),
            vec![7u8; 232],
            f,
        ));
        let mut f = header(0x20, None, &[200]);
        rle_blocks(&mut f, 7, 232, 232);
        cases.push((
            "single segment size 200, RLE 232 (over its window)".into(),
            vec![7u8; 232],
            f,
        ));
        drop(content);
    }
    for (name, content, frame) in &cases {
        verdict(name, content, frame);
    }
}

fn status(key: &str) -> u64 {
    let s = std::fs::read_to_string("/proc/self/status").unwrap();
    s.lines()
        .find(|l| l.starts_with(key))
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|v| v.parse().ok())
        .unwrap_or(0)
}

fn measured(what: &str, f: impl FnOnce() -> String) {
    std::fs::write("/proc/self/clear_refs", "5").unwrap();
    let (rss0, peak0) = (status("VmRSS:"), status("VmPeak:"));
    let t = Instant::now();
    let said = f();
    println!(
        "{what}: {said} in {:.1?}; peak RSS +{} MiB over {} MiB, peak address space {} MiB (from {})",
        t.elapsed(),
        (status("VmHWM:").saturating_sub(rss0)) >> 10,
        rss0 >> 10,
        status("VmPeak:") >> 10,
        peak0 >> 10
    );
}

#[test]
fn rt16_cost() {
    let k = 1024usize;
    for log in [21u32, 27] {
        let n = 2048 * k;
        let content = vec![9u8; n];
        let mut f = header(0x00, Some(wd(log)), &[]);
        rle_blocks(&mut f, 9, n, 128 * k);
        let (_d, r, e) = plant(ObjectKind::Content, hash_content(&content), n as u64, &f);
        measured(
            &format!("2 MiB file, window 2^{log}, no content size"),
            || {
                let mut out = Vec::new();
                format!("{:?}", r.copy_content(&e, &mut out).map(|()| out.len()))
            },
        );
    }
    for extra in [0usize, 1] {
        let n = 256 << 20;
        let content_len = n;
        let mut f = header(0x00, Some(wd(21)), &[]);
        rle_blocks(&mut f, 3, n + extra, 128 * k);
        let id = hash_record(Flavor::FolderListing, &vec![3u8; content_len]);
        let (_d, r, e) = plant(ObjectKind::FolderListing, id, n as u64, &f);
        measured(
            &format!(
                "record recorded 256 MiB, frame makes 256 MiB + {extra}, {} B stored",
                f.len()
            ),
            || {
                format!(
                    "{:?}",
                    r.read_record(&e)
                        .map(|b| b.len())
                        .map_err(|e| told(&e, &id))
                )
            },
        );
    }
}

#[test]
fn rt16_far_matches_large() {
    let k = 1024usize;
    let r = random(200 * k, 41);
    let content: Vec<u8> = r.repeat(7);
    let mut f = streamed(
        &content,
        19,
        &[
            CParameter::WindowLog(23),
            CParameter::EnableLongDistanceMatching(true),
        ],
    );
    let good = f.len();
    f[5] = wd(17);
    verdict(
        &format!(
            "offsets of 200 KiB, window byte -> 128 KiB, {} B (frame {good})",
            content.len()
        ),
        &content,
        &f,
    );
}
