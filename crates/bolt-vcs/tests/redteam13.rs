//! Red-team session RT13 (milestone 6, speed and space, second session): proof tests. Each
//! asserts the safe behaviour, so it fails on the code attacked and passes once its finding is
//! fixed. The report is docs/redteam/2026-11-10-milestone-6-speed-session-2.md.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

#[cfg(target_os = "linux")]
use support::{Env, text};

/// The bytes `bolt <args>` read (`rchar`) in `env`, which the shell that waited for it counts;
/// "failed" if it did.
#[cfg(target_os = "linux")]
fn reads(env: &Env, args: &str) -> String {
    let mut cmd = std::process::Command::new("sh");
    cmd.arg("-c")
        .arg(format!(
            "\"$0\" {args} >/dev/null 2>&1 || echo failed; grep rchar /proc/$$/io"
        ))
        .arg(assert_cmd::cargo::cargo_bin("bolt"))
        .current_dir(env.root())
        .env("BOLT_USER_NAME", "Test")
        .env("BOLT_USER_EMAIL", "test@example.com")
        .env("BOLT_CONFIG_DIR", env.config())
        .env("HOME", env.config());
    text(&cmd.output().unwrap().stdout)
}

#[cfg(target_os = "linux")]
fn rchar(said: &str) -> u64 {
    assert!(!said.contains("failed"), "{said}");
    said.lines()
        .find_map(|l| l.strip_prefix("rchar:"))
        .unwrap()
        .trim()
        .parse()
        .unwrap()
}

/// The bytes `bolt discard` read (`rchar`) in `env`, checked to have written `whole` to `big`.
#[cfg(target_os = "linux")]
fn discard_reads(env: &Env, whole: &[u8]) -> u64 {
    let mut cmd = std::process::Command::new("sh");
    cmd.arg("-c")
        .arg("\"$0\" discard >/dev/null 2>&1 || echo failed; grep rchar /proc/$$/io")
        .arg(assert_cmd::cargo::cargo_bin("bolt"))
        .current_dir(env.root())
        .env("BOLT_USER_NAME", "Test")
        .env("BOLT_USER_EMAIL", "test@example.com")
        .env("BOLT_CONFIG_DIR", env.config())
        .env("HOME", env.config());
    let said = text(&cmd.output().unwrap().stdout);
    assert!(!said.contains("failed"), "{said}");
    assert_eq!(std::fs::read(env.root().join("big")).unwrap(), whole);
    said.lines()
        .find_map(|l| l.strip_prefix("rchar:"))
        .unwrap()
        .trim()
        .parse()
        .unwrap()
}

/// RT13-1. RT12-2's fix reads a 64 KiB block at the second read in a row that falls in it. A
/// file whose tiny chunks are stored two by two, each pair in another block than the pair before
/// it, makes every pair read a whole block: about 32 KiB read for each 8-byte chunk, where main
/// read under 500 bytes.
#[cfg(target_os = "linux")]
#[test]
fn rt13_1_small_reads_in_pairs_cost_small_reads() {
    use bolt_core::chunks::{Chunk, ChunkList};
    use bolt_core::id::hash_content;

    const CHUNKS: usize = 20_000;
    // Columns of pairs, each over 64 KiB, so the next pair is always in another block.
    const COLUMNS: usize = 15;
    let env = Env::empty();
    env.bolt().arg("start").assert().success();
    let bytes: Vec<Vec<u8>> = (0..CHUNKS as u64)
        .map(|j| j.to_le_bytes().to_vec())
        .collect();
    support::add_pack(env.root(), |pack| {
        for column in 0..COLUMNS {
            for pair in (column..CHUNKS / 2).step_by(COLUMNS) {
                pack.add_content(&bytes[2 * pair]).unwrap();
                pack.add_content(&bytes[2 * pair + 1]).unwrap();
            }
        }
    });
    let whole = bytes.concat();
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
    let read = discard_reads(&env, &whole);
    let per_chunk = read / CHUNKS as u64;
    assert!(
        per_chunk <= 4096,
        "read {read} bytes to write {} bytes: {per_chunk} a chunk",
        whole.len()
    );
}

/// RT13-2. RT12-1's fix walks every other line, on every amend, until it meets the amended line's
/// list; nothing it learns is kept. Lines that branched off long ago, each with its own long run of
/// saves, are read from the store whole on every amend. Main kept their lists in
/// `cache/change-ids`, so only its first amend read them: 300 such branches of 2,000 commits took
/// this build 19 to 21 s an amend, every time, and main 34 s once, then 0.3 s.
#[cfg(target_os = "linux")]
#[test]
#[ignore = "RT13-2: every amend walks every line that branched off long ago, where main read them once"]
fn rt13_2_a_second_amend_does_not_walk_the_other_lines_again() {
    const BRANCHES: usize = 40;
    const DEPTH: usize = 500;
    let env = Env::new();
    let mut stream = b"commit refs/heads/main\nmark :1\ncommitter a <a@b> 1700000000 +0000\n\
data 1\nr\nM 100644 inline f\ndata 2\nr\n\n\
commit refs/heads/main\nmark :2\ncommitter a <a@b> 1700000001 +0000\ndata 1\nm\nfrom :1\n\
M 100644 inline f\ndata 2\nm\n\n"
        .to_vec();
    let mut mark = 2;
    for b in 0..BRANCHES {
        let mut prev = 1;
        for i in 0..DEPTH {
            mark += 1;
            let data = format!("b{b}.{i}\n");
            stream.extend_from_slice(
                format!(
                    "commit refs/heads/b{b:03}\nmark :{mark}\ncommitter a <a@b> {} +0000\n\
data 1\nc\nfrom :{prev}\nM 100644 inline g\ndata {}\n{data}\n",
                    1_700_000_002 + mark,
                    data.len()
                )
                .as_bytes(),
            );
            prev = mark;
        }
    }
    env.git_input(&["fast-import", "--quiet"], &stream);
    env.git(&["checkout", "-q", "-f", "main"]);
    env.import();
    let mut read = Vec::new();
    for n in 0..2 {
        std::fs::write(env.root().join("f"), format!("more {n}\n")).unwrap();
        read.push(rchar(&reads(&env, &format!("save --amend m{n}"))));
    }
    assert!(
        read[1] * 4 <= read[0],
        "the first amend read {} bytes, the second {}",
        read[0],
        read[1]
    );
}
