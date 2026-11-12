//! What the parts of a save cost, and what a stronger zstd level would save if merging and `tidy`
//! compressed stored bytes again. Slow, so ignored; run in a release build, one test at a time:
//! `cargo test --release -p opal-vcs-core --test throughput -- --ignored --nocapture --test-threads=1`
//! OPAL_BENCH_CODE names a code project's folder, and OPAL_BENCH_GODOT a folder whose PNG and OGG
//! files are the compressed assets; a set whose variable isn't set is left out.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use opal_core::chunks::{Cutter, is_cut};
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Instant;

const MIB: usize = 1 << 20;
const PIECE: usize = 64 << 10;
const LEVELS: [i32; 3] = [3, 9, 19];

#[test]
#[ignore = "slow: times the cutter, BLAKE3 and zstd"]
fn speeds_of_the_parts() {
    let noise = noise(256 * MIB);
    let started = Instant::now();
    let mut cutter = Cutter::new(&noise[..]);
    let mut chunks = 0;
    while cutter.next_chunk().unwrap().is_some() {
        chunks += 1;
    }
    report(&format!("cutter ({chunks} chunks)"), noise.len(), started);

    let started = Instant::now();
    let mut hasher = blake3::Hasher::new();
    for piece in noise.chunks(PIECE) {
        hasher.update(piece);
    }
    std::hint::black_box(hasher.finalize());
    report("BLAKE3", noise.len(), started);

    let sound = sound(64 * MIB);
    for (name, data) in [("noise", &noise[..]), ("sound", &sound[..])] {
        for level in LEVELS {
            let started = Instant::now();
            let mut out = 0;
            for piece in data.chunks(PIECE) {
                out += zstd::bulk::compress(piece, level).unwrap().len();
            }
            report(
                &format!("zstd {level} on {name} ({out} bytes out)"),
                data.len(),
                started,
            );
        }
    }
}

/// Merging and `tidy` compress again only if level 9 or 19 saves at least 10% of the code and
/// uncompressed sets' stored bytes, in at most 5 times level 3's time.
#[test]
#[ignore = "slow: compresses each set at three levels"]
fn what_compressing_again_would_save() {
    let mut sets = Vec::new();
    if let Some(dir) = std::env::var_os("OPAL_BENCH_CODE") {
        sets.push(("code", files_under(Path::new(&dir), |_| true)));
    }
    sets.push((
        "uncompressed",
        vec![
            Source::Made(sound(32 * MIB)),
            Source::Made(picture(32 * MIB)),
        ],
    ));
    if let Some(dir) = std::env::var_os("OPAL_BENCH_GODOT") {
        let assets = files_under(Path::new(&dir), |path| {
            path.extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("png") || e.eq_ignore_ascii_case("ogg"))
        });
        sets.push(("compressed", assets));
    }

    // Per level: the bytes stored and the seconds spent, for the sets the rule counts.
    let mut ruled = [(0u64, 0f64); LEVELS.len()];
    for (name, sources) in &sets {
        let (mut pieces, mut bytes) = (0, 0);
        each_piece(sources, |piece| {
            pieces += 1;
            bytes += piece.len() as u64;
        });
        println!(
            "{name}: {} sources, {pieces} pieces, {bytes} bytes",
            sources.len()
        );
        for (i, level) in LEVELS.into_iter().enumerate() {
            let (mut stored, mut secs) = (0, 0.0);
            each_piece(sources, |piece| {
                let started = Instant::now();
                let compressed = zstd::bulk::compress(piece, level).unwrap();
                secs += started.elapsed().as_secs_f64();
                stored += kept(compressed.len(), piece.len());
            });
            println!(
                "  level {level:>2}: {stored} bytes stored ({:.1}% of the bytes), {secs:.2} s, \
                 {:.0} MB/s",
                100.0 * stored as f64 / bytes as f64,
                bytes as f64 / 1e6 / secs
            );
            if *name != "compressed" {
                ruled[i].0 += stored;
                ruled[i].1 += secs;
            }
        }
    }
    // The rule counts the code set too, so without it there is no verdict to give.
    if !sets.iter().any(|(name, _)| *name == "code") {
        println!("No verdict: set OPAL_BENCH_CODE to count the code set.");
        return;
    }
    let (base_bytes, base_secs) = ruled[0];
    for (level, (stored, secs)) in LEVELS.into_iter().zip(ruled).skip(1) {
        let saves = 1.0 - stored as f64 / base_bytes as f64;
        let times = secs / base_secs;
        println!(
            "code and uncompressed, level {level}: saves {:.1}% of level 3's {base_bytes} bytes \
             in {times:.1} times its time: {}",
            100.0 * saves,
            if saves >= 0.10 && times <= 5.0 {
                "qualifies"
            } else {
                "does not qualify"
            }
        );
    }
}

/// Bytes a pack keeps for a piece: compressed only when that is under 95% of the piece.
fn kept(compressed: usize, original: usize) -> u64 {
    if compressed * 20 < original * 19 {
        compressed as u64
    } else {
        original as u64
    }
}

enum Source {
    File(PathBuf),
    Made(Vec<u8>),
}

/// Calls `each` with every piece a save stores: a file under 1 MiB whole, a bigger one chunk by
/// chunk.
fn each_piece(sources: &[Source], mut each: impl FnMut(&[u8])) {
    for source in sources {
        let (mut reader, len): (Box<dyn Read + '_>, u64) = match source {
            Source::File(path) => (
                Box::new(File::open(path).unwrap()),
                fs::metadata(path).unwrap().len(),
            ),
            Source::Made(bytes) => (Box::new(&bytes[..]), bytes.len() as u64),
        };
        if is_cut(len) {
            let mut cutter = Cutter::new(reader);
            while let Some(chunk) = cutter.next_chunk().unwrap() {
                each(chunk);
            }
        } else {
            let mut bytes = Vec::new();
            reader.read_to_end(&mut bytes).unwrap();
            each(&bytes);
        }
    }
}

/// The files under `dir` that `keep` accepts, `.git` left out, in a fixed order.
fn files_under(dir: &Path, keep: impl Fn(&Path) -> bool) -> Vec<Source> {
    let mut found = Vec::new();
    let mut folders = vec![dir.to_path_buf()];
    while let Some(folder) = folders.pop() {
        for entry in fs::read_dir(&folder).unwrap() {
            let entry = entry.unwrap();
            let kind = entry.file_type().unwrap();
            if kind.is_dir() && entry.file_name() != ".git" {
                folders.push(entry.path());
            } else if kind.is_file() && keep(&entry.path()) {
                found.push(entry.path());
            }
        }
    }
    found.sort();
    assert!(!found.is_empty(), "no files under {}", dir.display());
    found.into_iter().map(Source::File).collect()
}

/// Bytes from BLAKE3's extendable output: nothing in them compresses.
fn noise(len: usize) -> Vec<u8> {
    let mut out = vec![0; len];
    blake3::Hasher::new()
        .update(b"opal throughput")
        .finalize_xof()
        .fill(&mut out);
    out
}

/// A fixed stream of pseudo-random numbers.
struct Xorshift(u64);

impl Xorshift {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    /// A number from -spread to spread.
    fn around(&mut self, spread: i64) -> i64 {
        (self.next() % (2 * spread as u64 + 1)) as i64 - spread
    }
}

/// Like a WAV file's samples: 16-bit stereo at 44.1 kHz, three tones that swell and fade, under
/// a recording's noise of about 6 bits.
fn sound(len: usize) -> Vec<u8> {
    let mut rng = Xorshift(0x5eed_0001);
    let mut out = Vec::with_capacity(len);
    let mut t = 0u64;
    while out.len() < len {
        let s = t as f64 / 44_100.0;
        let swell = 0.5 + 0.5 * (s * 0.7).sin();
        let tone = 6000.0 * (s * 2.0 * std::f64::consts::PI * 220.0).sin()
            + 3000.0 * swell * (s * 2.0 * std::f64::consts::PI * 330.0).sin()
            + 1500.0 * (s * 2.0 * std::f64::consts::PI * 1234.5).sin();
        for channel in [1.0, 0.8] {
            let sample = (tone * channel) as i64 + rng.around(32);
            out.extend_from_slice(&(sample as i16).to_le_bytes());
        }
        t += 1;
    }
    out.truncate(len);
    out
}

/// Like a BMP file's pixels: 24-bit rows 4,096 wide, smooth shading with a camera's noise of
/// about 3 bits, and a quarter of it flat colour, as drawn shapes are.
fn picture(len: usize) -> Vec<u8> {
    const WIDTH: usize = 4096;
    let mut rng = Xorshift(0x5eed_0002);
    let mut out = Vec::with_capacity(len);
    let mut y = 0;
    while out.len() < len {
        for x in 0..WIDTH {
            let flat = (x / 512 + y / 512) % 4 == 0;
            for channel in 0..3 {
                let shade = (x * (channel + 1) / 32 + y / 16) % 256;
                let value = if flat {
                    (channel * 80 + 40) as i64
                } else {
                    shade as i64 + rng.around(4)
                };
                out.push(value.clamp(0, 255) as u8);
            }
        }
        y += 1;
    }
    out.truncate(len);
    out
}

fn report(what: &str, bytes: usize, started: Instant) {
    let secs = started.elapsed().as_secs_f64();
    println!("{what}: {:.0} MB/s", bytes as f64 / 1e6 / secs);
}
