//! Red-team proof tests for milestone 2's fixes, session 2 (decision 024): each asserts the safe
//! behaviour and fails on the code the session attacked. See
//! docs/redteam/2026-11-03-milestone-2-session-2.md.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

use opal_core::git::craft::{Craft, digest, entry_header, zlib};
use opal_core::git::{GitId, HashKind, ObjectType, object_id};
use opal_core::history::current_save;
use opal_core::listing::EntryKind;
use opal_core::repo::Repository;
use support::run_limited;

use ObjectType::{Blob, Commit, Tree};

struct Env(support::Env);

impl std::ops::Deref for Env {
    type Target = support::Env;
    fn deref(&self) -> &support::Env {
        &self.0
    }
}

impl Env {
    fn new() -> Env {
        Env(support::Env::empty())
    }

    fn opal(&self, args: &[&str]) -> Command {
        let mut cmd = self.opal_process_in(self.root());
        cmd.args(args);
        cmd
    }

    fn ok(&self, args: &[&str]) -> Output {
        let out = self.opal(args).output().unwrap();
        assert_eq!(
            out.status.code(),
            Some(0),
            "opal {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        out
    }

    /// Writes `file` in the work tree, as Git would have checked it out.
    fn write(&self, file: &str, bytes: &[u8]) {
        let path = self.root().join(file);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// The save the project is on: its paths, folders included, and a reader for one file's bytes.
fn saved(root: &Path) -> (Vec<String>, impl Fn(&str) -> Option<Vec<u8>>) {
    let repo = Repository::open(root).unwrap();
    let save = repo
        .store()
        .save(&current_save(&repo).unwrap(), false)
        .unwrap();
    let mut paths = Vec::new();
    let mut files = Vec::new();
    let mut todo = vec![(String::new(), save.root)];
    while let Some((prefix, id)) = todo.pop() {
        for e in repo.store().listing(&id).unwrap().entries() {
            let path = format!("{prefix}{}", e.name);
            if e.kind == EntryKind::Folder {
                todo.push((format!("{path}/"), e.id));
            } else {
                let mut bytes = Vec::new();
                repo.store().copy_content(&e.id, &mut bytes).unwrap();
                files.push((path.clone(), bytes));
            }
            paths.push(path);
        }
    }
    paths.sort();
    let read = move |path: &str| {
        files
            .iter()
            .find(|(p, _)| p == path)
            .map(|(_, b)| b.clone())
    };
    (paths, read)
}

const PEOPLE: &str = "author Ada <ada@example.com> 1700000000 +0000\n\
                      committer Ada <ada@example.com> 1700000000 +0000\n";

fn commit_bytes(tree: &GitId, parents: &[GitId], message: &str) -> Vec<u8> {
    let mut raw = format!("tree {}\n", tree.to_hex());
    for parent in parents {
        raw += &format!("parent {}\n", parent.to_hex());
    }
    format!("{raw}{PEOPLE}\n{message}").into_bytes()
}

fn sha1(kind: ObjectType, data: &[u8]) -> GitId {
    object_id(kind, data, HashKind::Sha1).unwrap()
}

fn dir_size(dir: &Path) -> u64 {
    let mut total = 0;
    let mut todo = vec![dir.to_owned()];
    while let Some(dir) = todo.pop() {
        for e in fs::read_dir(dir).unwrap() {
            let e = e.unwrap();
            let meta = e.metadata().unwrap();
            if meta.is_dir() {
                todo.push(e.path());
            } else {
                total += meta.len();
            }
        }
    }
    total
}

// A pack and its version 2 index, written as Git writes them (redteam3.rs's writer).

fn varint(mut n: u64) -> Vec<u8> {
    let mut out = Vec::new();
    while n >= 0x80 {
        out.push((n & 0x7f) as u8 | 0x80);
        n >>= 7;
    }
    out.push(n as u8);
    out
}

fn ofs_distance(mut distance: u64) -> Vec<u8> {
    let mut out = vec![(distance & 0x7f) as u8];
    distance >>= 7;
    while distance > 0 {
        distance -= 1;
        out.push(0x80 | (distance & 0x7f) as u8);
        distance >>= 7;
    }
    out.reverse();
    out
}

/// A delta instruction copying `len` bytes (at most 0xFFFFFF) from `offset` in the base.
fn copy(offset: u64, len: u64) -> Vec<u8> {
    let (mut op, mut args) = (0x80u8, Vec::new());
    for (i, bit) in [0x01u8, 0x02, 0x04, 0x08].into_iter().enumerate() {
        let b = (offset >> (8 * i)) as u8;
        if b != 0 {
            op |= bit;
            args.push(b);
        }
    }
    for (i, bit) in [0x10u8, 0x20, 0x40].into_iter().enumerate() {
        let b = (len >> (8 * i)) as u8;
        if b != 0 {
            op |= bit;
            args.push(b);
        }
    }
    [vec![op], args].concat()
}

fn insert(bytes: &[u8]) -> Vec<u8> {
    bytes
        .chunks(127)
        .flat_map(|c| [vec![c.len() as u8], c.to_vec()].concat())
        .collect()
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &byte in bytes {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb8_8320 & (crc & 1).wrapping_neg());
        }
    }
    !crc
}

struct PackOut {
    pack: Vec<u8>,
    rows: Vec<(Vec<u8>, u32, u64)>,
}

impl PackOut {
    fn new(count: u32) -> PackOut {
        let mut pack = b"PACK".to_vec();
        pack.extend_from_slice(&2u32.to_be_bytes());
        pack.extend_from_slice(&count.to_be_bytes());
        PackOut {
            pack,
            rows: Vec::new(),
        }
    }

    fn push(&mut self, entry: Vec<u8>, id: &[u8]) -> u64 {
        let at = self.pack.len() as u64;
        self.rows.push((id.to_vec(), crc32(&entry), at));
        self.pack.extend_from_slice(&entry);
        at
    }

    fn whole(&mut self, code: u8, data: &[u8], id: &[u8]) -> u64 {
        let entry = [entry_header(code, data.len() as u64), zlib(data)].concat();
        self.push(entry, id)
    }

    fn ofs_delta(&mut self, base_at: u64, delta: &[u8], id: &[u8]) -> u64 {
        let at = self.pack.len() as u64;
        let entry = [
            entry_header(6, delta.len() as u64),
            ofs_distance(at - base_at),
            zlib(delta),
        ]
        .concat();
        self.push(entry, id)
    }

    fn finish(mut self, root: &Path) {
        let checksum = digest(HashKind::Sha1, &self.pack);
        self.pack.extend_from_slice(&checksum);
        self.rows.sort();
        let mut idx = b"\xfftOc".to_vec();
        idx.extend_from_slice(&2u32.to_be_bytes());
        let mut below = 0u32;
        for byte in 0..=255u8 {
            below += self.rows.iter().filter(|r| r.0[0] == byte).count() as u32;
            idx.extend_from_slice(&below.to_be_bytes());
        }
        for row in &self.rows {
            idx.extend_from_slice(&row.0);
        }
        for row in &self.rows {
            idx.extend_from_slice(&row.1.to_be_bytes());
        }
        for row in &self.rows {
            idx.extend_from_slice(&(row.2 as u32).to_be_bytes());
        }
        idx.extend_from_slice(&checksum);
        let sum = digest(HashKind::Sha1, &idx);
        idx.extend_from_slice(&sum);
        let name: String = checksum.iter().map(|b| format!("{b:02x}")).collect();
        let dir = root.join(".git/objects/pack");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(format!("pack-{name}.pack")), &self.pack).unwrap();
        fs::write(dir.join(format!("pack-{name}.idx")), &idx).unwrap();
    }
}

// RT4-1: decision 043 limits the parents an import may add, because each save keeps its parents
// whole while Git stores a merge as a small delta. Every other part of a commit is kept whole too:
// here N commits share one random 1 MB message, which Git's pack holds once (each commit is a
// ~80-byte delta on the first), and each save holds a copy. No merge, so the limit never applies.

/// `n` bytes of letters and digits, the same every run, ending in a newline.
fn random_text(n: usize) -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
    let mut state = 0x9e37_79b9_7f4a_7c15u64;
    let mut out: String = (1..n)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            CHARS[(state % CHARS.len() as u64) as usize] as char
        })
        .collect();
    out.push('\n');
    out
}

/// `n` commits in a chain, each holding the same random 1 MB message, all but the first stored
/// as a delta on the first. Returns the size of `.git`.
fn commits_sharing_a_big_message(root: &Path, n: usize) -> u64 {
    let git = Craft::new(root, HashKind::Sha1);
    let message = random_text(1_000_000);
    let tree = sha1(Tree, b"");
    let mut pack = PackOut::new(1 + n as u32);
    pack.whole(2, b"", tree.as_bytes());
    let base = commit_bytes(&tree, &[], &message);
    let mut tip = sha1(Commit, &base);
    let base_at = pack.whole(1, &base, tip.as_bytes());
    // "tree <hex>\n", then each later commit's "parent <hex>\n".
    let head = 46;
    let len = base.len() as u64;
    for _ in 1..n {
        let line = format!("parent {}\n", tip.to_hex());
        let data = [&base[..head], line.as_bytes(), &base[head..]].concat();
        let delta = [
            varint(len),
            varint(data.len() as u64),
            copy(0, head as u64),
            insert(line.as_bytes()),
            copy(head as u64, len - head as u64),
        ]
        .concat();
        tip = sha1(Commit, &data);
        pack.ofs_delta(base_at, &delta, tip.as_bytes());
    }
    pack.finish(root);
    git.loose_ref("refs/heads/main", &format!("{}\n", tip.to_hex()));
    dir_size(&root.join(".git"))
}

#[test]
#[ignore = "RT4-1: ruled in decision 044 (whole records accepted until chunked storage)"]
fn rt4_1_a_small_history_without_merges_does_not_import_into_a_huge_opal() {
    let env = Env::new();
    let git_bytes = commits_sharing_a_big_message(env.root(), 100);
    let end = run_limited(env.opal(&["import"]), 600);
    let opal_bytes = if env.root().join(".opal").exists() {
        dir_size(&env.root().join(".opal"))
    } else {
        0
    };
    // Before the bound is checked, so a run records the sizes either way.
    println!(
        "RT4-1 sizes: .git {git_bytes} bytes, .opal {opal_bytes} bytes; the import took {:.1} s, \
         peaked at {} bytes, and exited with {:?}",
        end.secs, end.peak_bytes, end.code
    );
    // The bound redteam3.rs's RT3-2 test holds a ~1 MB repository to: decision 043's "under
    // about 35 MB" for a history at the parent limit. A stop at a limit is safe too; a stop that
    // calls this valid repository damaged is not.
    let stopped_at_a_limit = end.code == Some(1) && !end.stderr.contains("git fsck");
    assert!(
        (end.code == Some(0) || stopped_at_a_limit) && opal_bytes < 40 << 20,
        "`opal import` of a {git_bytes}-byte Git repository (100 commits, no merges) wrote a \
         {opal_bytes}-byte .opal: {end:?}"
    );
}

// RT4-2: `Repo::header` of a deltified object walks its whole delta chain, without the base cache
// and without counting toward the import's work allowance (RT3-4's fix). Pass 1 asks for the
// header of every tag's target, and a lightweight tag's target is a commit, which `roots` never
// remembers: 100,000 tags on a commit 4,096 deltas deep make it read 409,600,000 entry headers.

/// A commit at the end of a chain of `depth` deltas, and `tags` lightweight tags on it.
fn tags_on_a_deep_commit(root: &Path, tags: usize, depth: usize) {
    let git = Craft::new(root, HashKind::Sha1);
    let tree = sha1(Tree, b"");
    let mut pack = PackOut::new(2 + depth as u32);
    pack.whole(2, b"", tree.as_bytes());
    let mut prev = commit_bytes(&tree, &[], "chain 0\n");
    let mut at = pack.whole(1, &prev, sha1(Commit, &prev).as_bytes());
    let keep = (prev.len() - "chain 0\n".len()) as u64;
    for i in 1..=depth {
        let data = commit_bytes(&tree, &[], &format!("chain {i}\n"));
        let delta = [
            varint(prev.len() as u64),
            varint(data.len() as u64),
            copy(0, keep),
            insert(&data[keep as usize..]),
        ]
        .concat();
        at = pack.ofs_delta(at, &delta, sha1(Commit, &data).as_bytes());
        prev = data;
    }
    pack.finish(root);
    let tip = sha1(Commit, &prev).to_hex();
    git.loose_ref("refs/heads/main", &format!("{tip}\n"));
    let mut packed = "# pack-refs with: peeled fully-peeled sorted \n".to_owned();
    for i in 0..tags {
        packed += &format!("{tip} refs/tags/t{i:06}\n");
    }
    git.packed_refs(&packed);
}

#[test]
fn rt4_2_tags_on_a_commit_deep_in_deltas_import_in_reasonable_time() {
    // The same tags on a commit stored whole import at once (0.3 s in a release build).
    let whole = Env::new();
    tags_on_a_deep_commit(whole.root(), 100_000, 0);
    let end = run_limited(whole.opal(&["import"]), 20);
    assert!(end.code == Some(0), "{end:?}");
    let env = Env::new();
    tags_on_a_deep_commit(env.root(), 100_000, 4096);
    let git_bytes = dir_size(&env.root().join(".git"));
    let end = run_limited(env.opal(&["import"]), 20);
    // A stop at a limit is safe; a stop that calls this valid repository damaged is not.
    assert!(
        !end.timed_out && !end.stderr.contains("git fsck"),
        "`opal import` of 100,000 tags on one commit 4,096 deltas deep (a {git_bytes}-byte Git \
         repository) was still running after 20 s: {end:?}"
    );
}

// RT4-3: RT3-6's warning covers a saved file "an ignore rule matches". A repository that tracks a
// placeholder `.env` and lists it in .gitignore can also track a .opalignore that takes it back
// (`!.env`): for Git the file stays ignored, for Opalro no rule matches it any more. The user's real
// `.env` is then saved without a word, and so is a new file Git ignores, such as a private key.

#[test]
fn rt4_3_a_opalignore_from_the_repository_does_not_save_what_git_ignores_silently() {
    let env = Env::new();
    let git = Craft::new(env.root(), HashKind::Sha1);
    let files: [(&str, &[u8]); 4] = [
        (".env", b"API_KEY=put-yours-here\n"),
        (".gitignore", b".env\n*.pem\n"),
        (".opalignore", b"!.env\n!*.pem\n"),
        ("app.py", b"print('hi')\n"),
    ];
    let entries: Vec<(&[u8], &[u8], GitId)> = files
        .iter()
        .map(|(name, bytes)| (&b"100644"[..], name.as_bytes(), git.object(Blob, bytes)))
        .collect();
    let tree = git.tree(&entries);
    let commit = git.commit(&commit_bytes(&tree, &[], "Start\n"));
    git.loose_ref("refs/heads/main", &format!("{}\n", commit.to_hex()));
    for (name, bytes) in files {
        env.write(name, bytes);
    }
    let imported = env.ok(&["import"]);
    env.write(".env", b"API_KEY=sk_live_REAL_SECRET\n");
    env.write("deploy.pem", b"-----BEGIN PRIVATE KEY-----\nMIIE...\n");
    env.write("app.py", b"print('hello')\n");
    let out = env.ok(&["save", "Say hello"]);
    let said = [&imported.stdout, &imported.stderr, &out.stdout, &out.stderr]
        .map(|bytes| text(bytes))
        .concat();
    let (paths, read) = saved(env.root());
    let env_kept = read(".env").unwrap_or_default();
    let quiet_env = env_kept != b"API_KEY=put-yours-here\n" && !said.contains(".env");
    let quiet_pem = paths.iter().any(|p| p == "deploy.pem") && !said.contains("deploy.pem");
    assert!(
        !quiet_env && !quiet_pem,
        "with .gitignore keeping out .env and *.pem, the save took the user's .env ({:?}) and \
         deploy.pem ({}), and Opalro said only: {said}",
        text(&env_kept),
        paths.iter().any(|p| p == "deploy.pem")
    );
}

// RT4-4: RT3-8's fix says the limits allow a deltified file to be held twice: the object and its
// base. The delta itself is read whole too, up to the same 512 MiB, and is held beside both while
// it is applied. A 300 MiB file built on a 300 MiB base by a 300 MiB delta of inserts (a few
// hundred KB in the pack) takes three times its size, past RT3-8's own bound.

#[cfg(target_os = "linux")]
#[test]
fn rt4_4_rebuilding_a_deltified_file_holds_it_at_most_twice() {
    let env = Env::new();
    let git = Craft::new(env.root(), HashKind::Sha1);
    let size: u64 = 300 << 20;
    let mut pack = PackOut::new(5);
    let fake = |k: u64| [&[0xfe][..], &k.to_be_bytes(), &[0; 11]].concat();
    let mut at = pack.whole(3, &vec![0; 1 << 20], &fake(0));
    let grow = [
        varint(1 << 20),
        varint(size),
        copy(0, 1 << 20).repeat((size >> 20) as usize),
    ]
    .concat();
    at = pack.ofs_delta(at, &grow, &fake(1));
    // The file, as RT3-8's: 300 MiB of zeros ending in its own 8 bytes, but built by inserts only.
    let mut data = vec![0u8; size as usize];
    data[size as usize - 8..].copy_from_slice(b"the end!");
    let delta = [varint(size), varint(size), insert(&data)].concat();
    let blob = sha1(Blob, &data);
    drop(data);
    pack.ofs_delta(at, &delta, blob.as_bytes());
    drop(delta);
    let tree_data = [b"100644 big.bin\0".as_slice(), blob.as_bytes()].concat();
    let tree = sha1(Tree, &tree_data);
    pack.whole(2, &tree_data, tree.as_bytes());
    let commit_data = commit_bytes(&tree, &[], "A big file\n");
    let commit = sha1(Commit, &commit_data);
    pack.whole(1, &commit_data, commit.as_bytes());
    pack.finish(env.root());
    git.loose_ref("refs/heads/main", &format!("{}\n", commit.to_hex()));
    let end = run_limited(env.opal(&["import"]), 600);
    assert!(
        end.code == Some(0) && end.peak_mb < 750,
        "importing one 300 MiB deltified file took {} MB: {end:?}",
        end.peak_mb
    );
}
