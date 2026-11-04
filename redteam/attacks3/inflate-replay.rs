// Scratch, not compiled: replays Bolt's Inflater loop (16 KiB of input a call, as odb.rs) on zlib
// streams, to see where output goes missing. To run it, copy it to crates/bolt-vcs-core/tests/
// (miniz_oxide is that crate's dependency) and, in the sandbox:
//   RT3_DIR=<folder of N_what.z files> cargo test --release -p bolt-vcs-core \
//     --test inflate-replay sweep -- --ignored --nocapture
// Output: inflate-sweep.txt.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use miniz_oxide::inflate::stream::{InflateState, inflate};
use miniz_oxide::{DataFormat, MZError, MZFlush, MZStatus};

/// Bolt's loop: 16 KiB of input at a time, `out_len` bytes of output asked each call.
fn total(data: &[u8], in_len: usize, out_len: usize) -> Result<u64, String> {
    let mut state = InflateState::new_boxed(DataFormat::Zlib);
    let mut pos = 0usize;
    let (mut start, mut end) = (0usize, 0usize);
    let mut buf = vec![0u8; in_len];
    let mut out = vec![0u8; out_len];
    let mut total = 0u64;
    loop {
        let input = &buf[start..end];
        let r = inflate(&mut state, input, &mut out, MZFlush::None);
        start += r.bytes_consumed;
        match r.status {
            Ok(MZStatus::StreamEnd) => return Ok(total + r.bytes_written as u64),
            Ok(_) if r.bytes_written > 0 => total += r.bytes_written as u64,
            Ok(_) | Err(MZError::Buf) if start >= end => {
                start = 0;
                let n = (data.len() - pos).min(buf.len());
                buf[..n].copy_from_slice(&data[pos..pos + n]);
                pos += n;
                end = n;
                if n == 0 {
                    return Err("input ended".into());
                }
            }
            other => return Err(format!("{other:?}")),
        }
    }
}

#[test]
#[ignore = "scratch"]
fn sweep() {
    let dir = std::env::var("RT3_DIR").unwrap();
    let mut names: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    let (mut bad, mut all) = (0, 0);
    for name in names {
        let want: u64 = name.split('_').next().unwrap().parse().unwrap();
        let data = std::fs::read(format!("{dir}/{name}")).unwrap();
        all += 1;
        for out in [16384usize, 8192] {
            let got = total(&data, 16384, out);
            if got != Ok(want) {
                bad += 1;
                eprintln!("{name} (out {out}): {got:?}, want {want}");
            }
        }
    }
    eprintln!("SWEEP {bad} short of {all} streams");
}

#[test]
#[ignore = "scratch"]
fn miniz_made() {
    for mib in [180usize, 200, 240, 300] {
        let n = mib << 20;
        for level in [1u8, 6, 9] {
            let z = miniz_oxide::deflate::compress_to_vec_zlib(&vec![0u8; n], level);
            let got = total(&z, 16384, 16384);
            eprintln!("miniz level {level} {mib} MiB: {got:?} want {n}");
        }
    }
}
