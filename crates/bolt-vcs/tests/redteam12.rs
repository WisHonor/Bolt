//! Red-team session RT12 (milestone 6, speed and space): proof tests. Each asserts the safe
//! behaviour, so it fails on the code attacked and passes once its finding is fixed. The report
//! is docs/redteam/2026-11-10-milestone-6-speed.md.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use support::{Env, text};

/// A Git repository whose `main` holds `commits` commits in a row, with `branches` more branches
/// all at the commit before `main`'s newest, imported into Bolt: a line for each branch.
#[cfg(target_os = "linux")]
fn imported_with_branches(commits: usize, branches: usize) -> Env {
    let env = Env::new();
    let mut stream = Vec::new();
    for i in 0..commits {
        let data = format!("v{i}\n");
        stream.extend_from_slice(
            format!(
                "commit refs/heads/main\nmark :{}\ncommitter a <a@b> {} +0000\ndata 2\nc\n",
                i + 1,
                1_700_000_000 + i
            )
            .as_bytes(),
        );
        if i > 0 {
            stream.extend_from_slice(format!("from :{i}\n").as_bytes());
        }
        stream.extend_from_slice(
            format!("M 100644 inline f\ndata {}\n{data}\n", data.len()).as_bytes(),
        );
    }
    env.git_input(&["fast-import", "--quiet"], &stream);
    let parent = env.git(&["rev-parse", "main~1"]);
    let refs: String = (0..branches)
        .map(|j| format!("create refs/heads/b{j:06} {parent}\n"))
        .collect();
    env.git_input(&["update-ref", "--stdin"], refs.as_bytes());
    env.git(&["checkout", "-q", "-f", "main"]);
    env.import();
    env
}

/// RT12-1. An amend asks every other line whether it reaches the save, and the change-ID cache
/// then holds each line's whole list of saves, a copy per line, and encodes and seals all of it
/// before its 256 MiB cap is checked. 10,000 branches over 1,000 commits (10 million pairs) take
/// an amend to 1.4 GB; main's took 0.47 GB. Under the 1.5 GB cap `cli.rs` uses, it aborts, after
/// the amend is committed. Linux only: macOS doesn't enforce the address-space cap.
#[cfg(target_os = "linux")]
#[test]
fn rt12_1_an_amend_among_many_lines_stays_within_memory() {
    let env = imported_with_branches(1_000, 10_000);
    std::fs::write(env.root().join("f"), "changed\n").unwrap();
    let out = support::bolt_capped(env.config(), env.root(), &["save", "--amend", "m"])
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}{}",
        text(&out.stdout),
        text(&out.stderr)
    );
}

/// RT12-2. A small read of a pack is answered from a 64 KiB block of it, and a read away from the
/// block held reads a new one. A file whose tiny chunks sit scattered in one pack then costs three
/// such blocks a chunk (the index's IDs, its lines, the entry): about 200 KB read for each 8-byte
/// chunk, where main read under 500 bytes, and twice main's time. Measured as the bytes `bolt`
/// read (`rchar`), which the shell that waited for it counts.
#[cfg(target_os = "linux")]
#[test]
fn rt12_2_a_scattered_small_read_costs_a_small_read() {
    use bolt_core::chunks::{Chunk, ChunkList};
    use bolt_core::id::hash_content;

    const CHUNKS: usize = 20_000;
    let env = Env::empty();
    env.bolt().arg("start").assert().success();
    let bytes: Vec<Vec<u8>> = (0..CHUNKS as u64)
        .map(|j| j.to_le_bytes().to_vec())
        .collect();
    // Stored out of the file's order, as a list planted over an old pack can name them.
    support::add_pack(env.root(), |pack| {
        for j in 0..CHUNKS {
            pack.add_content(&bytes[(j * 7919) % CHUNKS]).unwrap();
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
    let read: u64 = said
        .lines()
        .find_map(|l| l.strip_prefix("rchar:"))
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    let per_chunk = read / CHUNKS as u64;
    assert!(
        per_chunk <= 4096,
        "read {read} bytes to write {} bytes: {per_chunk} a chunk",
        whole.len()
    );
}

/// RT12-3. `goto` with all 64 digits of an ID takes the ID as a save the line reaches when the
/// line's list in `cache/change-ids` holds it, without asking the store whether it is one. A list
/// sealed with the user's key that starts at the line's newest save, as a list should, but holds
/// a folder listing's ID makes `goto` of that ID fail with "object … is FolderListing, not Save"
/// (or "is not in the store" for an ID of nothing), where main says no save matches.
#[test]
fn rt12_3_goto_of_a_full_id_no_save_has_says_no_save_matches() {
    use bolt_core::keyed::{self, KEY_LEN};
    use bolt_core::repo::Repository;

    let env = Env::empty();
    env.bolt().arg("start").assert().success();
    for n in 0..2 {
        std::fs::write(env.root().join("f"), format!("v{n}")).unwrap();
        env.bolt()
            .args(["save", &format!("s{n}")])
            .assert()
            .success();
    }
    let repo = Repository::open(env.root()).unwrap();
    let mut line = Vec::new();
    let mut next = Some(support::main_tip(&env));
    while let Some(id) = next {
        let save = repo.store().save(&id, false).unwrap();
        line.push((id, save.change_id.0));
        next = save.parents.first().copied();
    }
    let stranger = repo.store().save(&line[0].0, false).unwrap().root;
    drop(repo);
    line.insert(1, (stranger, [0x11; 16]));
    // Version 2 of storage-format.md §10: one list, `main`'s.
    let mut body = b"BOLTCHID\x02\x01\x04main".to_vec();
    body.push(u8::try_from(line.len()).unwrap());
    for (id, change_id) in &line {
        body.extend_from_slice(&id.0);
        body.extend_from_slice(change_id);
    }
    let key: [u8; KEY_LEN] = std::fs::read(env.config().join(keyed::KEY_FILE))
        .unwrap()
        .try_into()
        .unwrap();
    std::fs::write(
        env.root().join(".bolt/cache/change-ids"),
        keyed::seal(&key, &body),
    )
    .unwrap();
    let out = env
        .bolt()
        .args(["goto", &stranger.to_hex()])
        .output()
        .unwrap();
    let said = text(&out.stderr);
    assert!(
        said.contains("No save, line or ID matches"),
        "{:?}: {said}",
        out.status.code()
    );
}
