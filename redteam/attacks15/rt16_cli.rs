//! RT16 exploration (decision 024), sandbox only: planted frames through the program.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use std::path::Path;
use std::time::{Duration, SystemTime};

use opal_core::id::{Id, hash_content};
use opal_core::pack::{ENTRY_HEADER_LEN, ObjectKind, PackReader, Prepared};
use support::{Env, add_pack, text};

fn rle_frame(byte: u8, total: usize, per: usize, fcs: bool) -> Vec<u8> {
    let mut f = vec![0x28, 0xB5, 0x2F, 0xFD];
    if fcs {
        f.extend([0x80, 0x58]);
        f.extend_from_slice(&(total as u32).to_le_bytes());
    } else {
        f.extend([0x00, 0x58]);
    }
    let mut left = total;
    while left > 0 {
        let n = left.min(per);
        left -= n;
        let v = ((n as u32) << 3) | (1 << 1) | u32::from(left == 0);
        f.extend_from_slice(&v.to_le_bytes()[..3]);
        f.push(byte);
    }
    f
}

fn plant(env: &Env, content: &[u8], frame: &[u8]) -> Id {
    let id = hash_content(content);
    add_pack(env.root(), |p| {
        p.add_prepared(
            ObjectKind::Content,
            &Prepared {
                id,
                size: content.len() as u64,
                zstd: true,
                stored: frame.to_vec(),
            },
        )
        .unwrap();
    });
    id
}

fn run(env: &Env, args: &[&str]) -> String {
    let out = env.opalro().args(args).output().unwrap();
    format!(
        "$ opalro {} -> {:?}\n{}{}",
        args.join(" "),
        out.status.code(),
        text(&out.stdout),
        text(&out.stderr)
    )
}

/// Each pack in the store holding `id`, with its stored bytes.
fn stored_copies(root: &Path, id: &Id) -> Vec<(String, Vec<u8>)> {
    let packs = root.join(".opalro/store/packs");
    let mut out = Vec::new();
    for f in std::fs::read_dir(&packs).unwrap() {
        let path = f.unwrap().path();
        let Ok(r) = PackReader::open(&path) else {
            continue;
        };
        let bytes = std::fs::read(&path).unwrap();
        for e in r.entries() {
            let e = e.unwrap();
            if e.id == *id {
                let at = (e.offset + ENTRY_HEADER_LEN) as usize;
                out.push((
                    path.file_name().unwrap().to_string_lossy().into_owned(),
                    bytes[at..at + e.stored_size as usize].to_vec(),
                ));
            }
        }
    }
    out
}

#[test]
fn rt16_cli() {
    let env = Env::empty();
    env.opalro()
        .args(["start", "--preset", "custom"])
        .assert()
        .success();
    let small = vec![b'A'; 200 << 10];
    let large = vec![b'B'; 1200 << 10];
    // RLE blocks over Block_Maximum_Size (128 KiB): one of 200 KiB; four of 300 KiB.
    let small_frame = rle_frame(b'A', small.len(), 300 << 10, true);
    let large_frame = rle_frame(b'B', large.len(), 300 << 10, true);
    let small_id = plant(&env, &small, &small_frame);
    let large_id = plant(&env, &large, &large_frame);
    std::fs::write(env.root().join("small.bin"), &small).unwrap();
    std::fs::write(env.root().join("large.bin"), &large).unwrap();
    println!("{}", run(&env, &["save", "Two files"]));
    let show = |what: &str, id: &Id, frame: &[u8]| {
        for (pack, bytes) in stored_copies(env.root(), id) {
            println!(
                "  {what} in {pack}: {} B, planted frame: {}",
                bytes.len(),
                bytes == frame
            );
        }
    };
    show("small", &small_id, &small_frame);
    show("large", &large_id, &large_frame);
    println!("{}", run(&env, &["verify"]));
    std::fs::remove_file(env.root().join("small.bin")).unwrap();
    std::fs::remove_file(env.root().join("large.bin")).unwrap();
    println!("{}", run(&env, &["discard"]));
    for (name, want) in [("small.bin", &small), ("large.bin", &large)] {
        let got = std::fs::read(env.root().join(name)).ok();
        println!(
            "  {name} written back whole: {}",
            got.as_ref() == Some(want)
        );
    }
    // Every pack two days old, so tidy may rewrite the small ones.
    let old = SystemTime::now() - Duration::from_secs(2 * 24 * 3600);
    for f in std::fs::read_dir(env.root().join(".opalro/store/packs")).unwrap() {
        let path = f.unwrap().path();
        std::fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(old)
            .unwrap();
    }
    println!("{}", run(&env, &["tidy"]));
    show("small after tidy", &small_id, &small_frame);
    show("large after tidy", &large_id, &large_frame);
    println!("{}", run(&env, &["verify"]));
}

/// The user's own good copy first, then a planted copy of the same file under a name that sorts
/// first: which one does `tidy` keep?
#[test]
fn rt16_cli_tidy_keeps_which() {
    let env = Env::empty();
    env.opalro()
        .args(["start", "--preset", "custom"])
        .assert()
        .success();
    let small = vec![b'A'; 200 << 10];
    std::fs::write(env.root().join("small.bin"), &small).unwrap();
    println!("{}", run(&env, &["save", "Mine"]));
    let id = hash_content(&small);
    let frame = rle_frame(b'A', small.len(), 300 << 10, true);
    let made = add_pack(env.root(), |p| {
        p.add_prepared(
            ObjectKind::Content,
            &Prepared {
                id,
                size: small.len() as u64,
                zstd: true,
                stored: frame.clone(),
            },
        )
        .unwrap();
    });
    let packs = env.root().join(".opalro/store/packs");
    for p in made {
        std::fs::rename(&p, packs.join("00.pack")).unwrap();
    }
    for (pack, bytes) in stored_copies(env.root(), &id) {
        println!(
            "  before tidy: {pack}: {} B, planted frame: {}",
            bytes.len(),
            bytes == frame
        );
    }
    let old = SystemTime::now() - Duration::from_secs(2 * 24 * 3600);
    for f in std::fs::read_dir(&packs).unwrap() {
        let path = f.unwrap().path();
        std::fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(old)
            .unwrap();
    }
    println!("{}", run(&env, &["tidy"]));
    for (pack, bytes) in stored_copies(env.root(), &id) {
        println!(
            "  after tidy: {pack}: {} B, planted frame: {}",
            bytes.len(),
            bytes == frame
        );
    }
    println!("{}", run(&env, &["verify"]));
}
