//! Red-team proof tests for milestone 1, session 2 (decision 024): each asserts the safe
//! behaviour and fails without the fix for the finding it is named after.
//! See docs/redteam/2026-11-02-milestone-1-session-2.md.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

use bolt_core::diary::{Action, DiaryEntry};
use bolt_core::id::ChangeId;
use bolt_core::pack::ObjectKind;
use bolt_core::repo::Repository;
use bolt_core::save::Save;
use tempfile::TempDir;

mod support;
#[cfg(unix)]
use support::bolt_capped;
use support::run_limited;

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

    fn bolt_in(&self, dir: &Path) -> Command {
        let mut cmd = Command::new(assert_cmd::cargo::cargo_bin("bolt"));
        cmd.current_dir(dir)
            .env_remove("BOLT_FAULT_AT")
            .env("BOLT_CONFIG_DIR", self.config.path())
            .env("HOME", self.config.path())
            .env("XDG_CONFIG_HOME", self.config.path())
            .env("BOLT_USER_NAME", "Test")
            .env("BOLT_USER_EMAIL", "test@example.com");
        cmd
    }

    fn bolt(&self) -> Command {
        self.bolt_in(self.root())
    }

    fn ok(&self, args: &[&str]) -> String {
        let out = self.bolt().args(args).output().unwrap();
        assert_eq!(
            out.status.code(),
            Some(0),
            "bolt {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).into_owned()
    }
}

fn pack_bytes(root: &Path) -> u64 {
    std::fs::read_dir(root.join(".bolt/store/packs"))
        .unwrap()
        .map(|e| e.unwrap().metadata().unwrap().len())
        .sum()
}

/// Commits, through Bolt's own library, `n` more saves on `main`, each the child of the last and
/// shaped by `shape`, plus `lines` more lines that all point at the newest one.
fn plant_chain(root: &Path, n: usize, lines: usize, shape: impl FnOnce(&mut Save)) {
    let mut repo = Repository::open(root).unwrap();
    let lock = repo.lock_for_change("save", &mut |_| {}).unwrap();
    let (head, entry) = repo.current().unwrap();
    let mut tip = entry.state.lines["main"];
    let mut save = repo.store().save(&tip, false).unwrap();
    shape(&mut save);
    let mut pack = repo.new_packs().unwrap();
    for _ in 0..n {
        save.parents = vec![tip];
        save.change_id = ChangeId::random().unwrap();
        tip = pack.add_record(ObjectKind::Save, &save.encode()).unwrap();
    }
    let mut state = entry.state.clone();
    state.lines.insert("main".into(), tip);
    for i in 0..lines {
        state.lines.insert(format!("l{i:06}"), tip);
    }
    let diary = DiaryEntry {
        parent: Some(head),
        action: Action::Command,
        command: "save".into(),
        who: save.author.clone(),
        bolt_version: "0.1.0".into(),
        state,
        safety_copies: vec![],
    };
    repo.commit(&lock, pack, &diary).unwrap();
}

/// Not a finding: builds a hostile repository for the scripts in redteam/attacks2, in the
/// folder `RT2_DIR`. `RT2_KIND=history`: `RT2_N` saves with 1 MiB messages. `RT2_KIND=lines`:
/// `RT2_N` small saves and `RT2_LINES` more lines pointing at the newest.
#[test]
#[ignore = "run by redteam/attacks2 scripts"]
fn plant_for_script() {
    let dir = std::path::PathBuf::from(std::env::var("RT2_DIR").unwrap());
    let var = |k: &str| std::env::var(k).unwrap_or_default();
    let n: usize = var("RT2_N").parse().unwrap();
    let st = Command::new(assert_cmd::cargo::cargo_bin("bolt"))
        .arg("start")
        .current_dir(&dir)
        .env("BOLT_USER_NAME", "Test")
        .env("BOLT_USER_EMAIL", "test@example.com")
        .stdout(Stdio::null())
        .status()
        .unwrap();
    assert!(st.success());
    match var("RT2_KIND").as_str() {
        "history" => {
            let message = largest_message();
            plant_chain(&dir, n, 0, |s| s.message = message);
        }
        "lines" => plant_chain(&dir, n, var("RT2_LINES").parse().unwrap(), |_| {}),
        other => panic!("unknown RT2_KIND {other}"),
    }
    eprintln!("planted; packs: {} bytes", pack_bytes(&dir));
}

/// A message of the most bytes a save may hold (1 MiB), with a short first line.
fn largest_message() -> String {
    let mut m = "Looks harmless\n".to_owned();
    m.push_str(&"A".repeat((1 << 20) - m.len()));
    m
}

// RT2-1: `line_history` keeps every save on the line in memory, and zstd shrinks a 1 MiB
// message to a few dozen bytes. A sub-megabyte repository makes `bolt history` need gigabytes.

#[cfg(unix)]
#[test]
fn rt2_1_history_of_a_tiny_repository_does_not_need_gigabytes() {
    let env = Env::new();
    env.ok(&["start"]);
    let message = largest_message();
    plant_chain(env.root(), 2000, 0, |s| s.message = message);
    let packs = pack_bytes(env.root());
    eprintln!("all packs together: {packs} bytes");
    let end = run_limited(
        bolt_capped(env.config.path(), env.root(), &["history"]),
        120,
    );
    assert!(
        end.code == Some(0),
        "`bolt history` on a {packs}-byte repository failed under a 1.5 GB cap: {end:?}"
    );
}

/// The same history bomb hits the victim's own `bolt save`: `short_of` walks the whole line
/// after the save is committed, so the save lands and then the command dies.
#[cfg(unix)]
#[test]
fn rt2_1_save_in_a_repository_with_a_long_history_does_not_die_after_committing() {
    let env = Env::new();
    env.ok(&["start"]);
    let message = largest_message();
    plant_chain(env.root(), 2000, 0, |s| s.message = message);
    let before = Repository::open(env.root()).unwrap().head().unwrap();
    std::fs::write(env.root().join("mine.txt"), "my work").unwrap();
    let end = run_limited(
        bolt_capped(env.config.path(), env.root(), &["save", "my work"]),
        120,
    );
    let after = Repository::open(env.root()).unwrap().head().unwrap();
    assert!(
        end.code == Some(0),
        "`bolt save` failed (committed anyway: {}): {end:?}",
        before != after
    );
}

// RT2-2: `resolve` with hex digits walks the whole history once for EVERY line, and a diary
// entry may hold 100,000 lines that all point at the same long chain.

#[test]
fn rt2_2_show_by_fingerprint_does_not_walk_the_history_once_per_line() {
    let env = Env::new();
    env.ok(&["start"]);
    plant_chain(env.root(), 300, 99_999, |_| {});
    let packs = pack_bytes(env.root());
    let mut cmd = env.bolt();
    cmd.args(["show", "0123abcd"]);
    let end = run_limited(cmd, 60);
    assert!(
        end.code == Some(1) && end.stderr.contains("No save, line or ID matches"),
        "`bolt show 0123abcd` hung or failed otherwise on a {packs}-byte repository: {end:?}"
    );
}

// RT2-3: `text::escape` lets invisible format characters through (zero-width space, word
// joiner, the deprecated bidi-shaping controls U+206A-206F, tag characters), so two different
// names print identically.

#[test]
fn rt2_3_invisible_characters_in_names_and_messages_are_not_printed_raw() {
    let env = Env::new();
    env.ok(&["start"]);
    let invisible = [
        '\u{200b}',
        '\u{2060}',
        '\u{2063}',
        '\u{206a}',
        '\u{206e}',
        '\u{feff}',
        '\u{e0041}',
        '\u{180e}',
        '\u{00ad}',
    ];
    std::fs::write(env.root().join("setup.py"), "real").unwrap();
    for (i, c) in invisible.iter().enumerate() {
        std::fs::write(env.root().join(format!("setup{c}.py")), format!("evil {i}")).unwrap();
    }
    let message: String = format!("Fix typo{}", invisible.iter().collect::<String>());
    env.ok(&["save", &message]);
    let shown = env.ok(&["show"]) + &env.ok(&["history"]);
    let raw: Vec<String> = invisible
        .iter()
        .filter(|c| shown.contains(**c))
        .map(|c| format!("U+{:04X}", u32::from(*c)))
        .collect();
    assert!(raw.is_empty(), "printed raw: {raw:?}\n{shown}");
}

/// The paths `bolt show --json` lists for the newest save.
fn shown_paths(env: &Env) -> Vec<String> {
    let out = env.ok(&["--json", "show"]);
    out.split("\"path\":\"")
        .skip(1)
        .map(|s| s.split('"').next().unwrap().to_owned())
        .collect()
}

// RT2-4: `.boltignore` is meant to use "exactly the .gitignore pattern syntax" (storage-format.md
// §12), but `ignore`/`globset` read `{a,b}` as a choice where Git reads it literally.

#[test]
fn rt2_4_braces_in_boltignore_are_literal_as_in_git() {
    let env = Env::new();
    env.ok(&["start"]);
    std::fs::write(env.root().join(".boltignore"), "{prod,dev}.env\n").unwrap();
    std::fs::write(env.root().join("{prod,dev}.env"), "API_KEY=hunter2").unwrap();
    std::fs::write(env.root().join("prod.env"), "kept by Git").unwrap();
    env.ok(&["save", "settings"]);
    let saved = shown_paths(&env);
    assert!(
        !saved.contains(&"{prod,dev}.env".to_owned()) && saved.contains(&"prod.env".to_owned()),
        "Git ignores `{{prod,dev}}.env` and keeps `prod.env`; Bolt saved {saved:?}"
    );
}

/// RT2-9, Windows and macOS: there Git sets core.ignorecase, so `*.env` also ignores `PROD.ENV`.
/// The `ignore` matcher is case-sensitive unless told otherwise.
#[cfg(any(windows, target_os = "macos"))]
#[test]
fn win_rt2_9_boltignore_ignores_case_like_git_on_case_insensitive_systems() {
    let env = Env::new();
    env.ok(&["start"]);
    std::fs::write(env.root().join(".boltignore"), "*.env\nsecrets/\n").unwrap();
    std::fs::write(env.root().join("PROD.ENV"), "API_KEY=hunter2").unwrap();
    std::fs::create_dir(env.root().join("Secrets")).unwrap();
    std::fs::write(env.root().join("Secrets").join("key.pem"), "-----BEGIN").unwrap();
    std::fs::write(env.root().join("readme.txt"), "kept").unwrap();
    env.ok(&["save", "settings"]);
    let saved = shown_paths(&env);
    assert_eq!(
        saved,
        [".boltignore", "readme.txt"],
        "Git ignores the rest here"
    );
}

// RT2-5: a pack's index is read whole into memory, limited only by the file's size. A sparse
// file costs nothing on disk and survives `tar --sparse`.

/// Writes a pack of `size` bytes that is all hole except its header and trailer; the trailer
/// says the index starts right after the header.
#[cfg(target_os = "linux")]
fn plant_sparse_pack(root: &Path, size: u64) {
    use std::io::{Seek, SeekFrom, Write};
    let mut f = std::fs::File::create(root.join(".bolt/store/packs/zz.pack")).unwrap();
    f.write_all(b"BOLTPACK\x01\0\0\0\0\0\0\0").unwrap();
    f.set_len(size).unwrap();
    f.seek(SeekFrom::Start(size - 48)).unwrap();
    f.write_all(&16u64.to_le_bytes()).unwrap();
    f.write_all(&[0; 32]).unwrap();
    f.write_all(b"BOLTPEND").unwrap();
}

#[cfg(target_os = "linux")]
#[test]
fn rt2_5_a_sparse_pack_does_not_make_every_command_allocate_its_size() {
    let env = Env::new();
    env.ok(&["start"]);
    plant_sparse_pack(env.root(), 2 << 30);
    let mut cmd = env.bolt();
    cmd.arg("history");
    let end = run_limited(cmd, 120);
    assert!(
        end.peak_mb < 512,
        "`bolt history` on a pack that takes 8 KB of disk peaked at {} MB: {end:?}",
        end.peak_mb
    );
}

// RT2-6: the scan keeps the compiled rules of every folder on its way down, and one 1 MiB
// `.boltignore` compiles to ~17 MB; nested ones add up (4,096 levels allowed).

#[cfg(unix)]
#[test]
fn rt2_6_nested_boltignore_files_do_not_exhaust_memory_in_save() {
    let env = Env::new();
    env.ok(&["start"]);
    let mut rules = String::new();
    for i in 0.. {
        if rules.len() > (1 << 20) - 300 {
            break;
        }
        rules += &format!("{}{i}\n", "[a-zA-Z0-9_]".repeat(20));
    }
    let mut dir = env.root().to_path_buf();
    for _ in 0..100 {
        std::fs::write(dir.join(".boltignore"), &rules).unwrap();
        dir.push("d");
        std::fs::create_dir(&dir).unwrap();
    }
    let end = run_limited(
        bolt_capped(env.config.path(), env.root(), &["save", "x"]),
        300,
    );
    // The files in force at one folder may total 1 MiB, so the second one is refused: an ordinary
    // error that names it, not an abort for want of memory.
    assert!(
        end.code == Some(1)
            && end.stderr.contains("d/.boltignore: ")
            && end.stderr.contains("larger than 1048576 bytes"),
        "`bolt save` of 100 nested .boltignore files did not stop cleanly under a 1.5 GB cap: {end:?}"
    );
}

// RT2-7: `save` skips storing content (and listings) whose ID the store already knows, but no
// object in a pack is checked against its fingerprint until it is read. A planted pack that
// claims the ID of a file the victim adds later makes Bolt keep garbage instead of the file.

/// Writes a structurally valid pack `00.pack` (it sorts before Bolt's own packs, so its entries
/// win) with one uncompressed entry of `kind`: `id`, holding `data`.
fn plant_lying_pack(root: &Path, kind: ObjectKind, id: &bolt_core::id::Id, data: &[u8]) {
    use bolt_core::encoding::Writer;
    use bolt_core::pack::{PACK_END_MAGIC, PACK_MAGIC};
    let mut entry = Writer::new();
    entry.u8(kind as u8);
    entry.id(id);
    entry.u64(data.len() as u64);
    entry.u64(data.len() as u64);
    entry.u8(0); // stored as is
    let mut index = Writer::new();
    index.varint(1);
    index.id(id);
    index.u8(kind as u8);
    index.u64(16);
    index.varint(data.len() as u64);
    index.varint(data.len() as u64);
    let index = index.into_bytes();
    let mut out = PACK_MAGIC.to_vec();
    out.extend_from_slice(&1u32.to_le_bytes()); // version 1, which readers still read
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&entry.into_bytes());
    out.extend_from_slice(data);
    let index_offset = out.len() as u64;
    out.extend_from_slice(&index);
    out.extend_from_slice(&index_offset.to_le_bytes());
    let mut checksum = Writer::new(); // plain BLAKE3 of the index, which `hash_content` is
    checksum.id(&bolt_core::id::hash_content(&index));
    out.extend_from_slice(&checksum.into_bytes());
    out.extend_from_slice(&PACK_END_MAGIC);
    std::fs::write(root.join(".bolt/store/packs/00.pack"), out).unwrap();
}

#[test]
fn rt2_7_save_stores_a_file_even_if_a_planted_pack_claims_its_id() {
    let env = Env::new();
    env.ok(&["start"]);
    // Content the attacker can predict: a standard licence, a vendored library, an engine DLL.
    let licence = b"MIT License\n\nPermission is hereby granted, free of charge, to any person\n";
    let id = bolt_core::id::hash_content(licence);
    plant_lying_pack(
        env.root(),
        ObjectKind::Content,
        &id,
        &vec![b'X'; licence.len()],
    );
    env.ok(&["history"]); // the planted pack passes every check made when a project opens
    std::fs::write(env.root().join("LICENSE"), licence).unwrap();
    let said = env.ok(&["save", "Add licence"]);
    let repo = Repository::open(env.root()).unwrap();
    let mut back = Vec::new();
    let read = repo.store().copy_content(&id, &mut back);
    assert!(
        read.is_ok() && back == licence,
        "`bolt save` said {said:?}, but LICENSE can't be read back: {read:?}"
    );
}

// Checking a stored copy before trusting it means reading it, and a planted copy can claim a
// huge size while taking little disk: every save then spent minutes on it.

/// Writes `00.pack` (it sorts before Bolt's own packs) holding a copy of each of `ids`, of `kind`,
/// that records `claimed` bytes but stores a zstd frame of 128 KiB RLE blocks: 4 bytes on disk per
/// 128 KiB.
fn plant_huge_copies(root: &Path, kind: ObjectKind, ids: &[bolt_core::id::Id], claimed: u64) {
    use bolt_core::encoding::Writer;
    use bolt_core::pack::{PACK_END_MAGIC, PACK_MAGIC};
    const BLOCK: u64 = 128 << 10;
    assert!(claimed >= BLOCK && claimed.is_multiple_of(BLOCK));
    let blocks = usize::try_from(claimed / BLOCK).unwrap();
    // Magic, no content size or checksum, a 128 KiB window; then RLE blocks, the last one marked.
    let mut frame = vec![0x28, 0xB5, 0x2F, 0xFD, 0x00, 0x38];
    frame.extend([0x02, 0x00, 0x10, 0x00].repeat(blocks - 1));
    frame.extend([0x03, 0x00, 0x10, 0x00]);
    let stored = frame.len() as u64;
    let mut ids = ids.to_vec();
    ids.sort();
    let mut out = PACK_MAGIC.to_vec();
    out.extend_from_slice(&1u32.to_le_bytes()); // version 1, which readers still read
    out.extend_from_slice(&0u32.to_le_bytes());
    let mut index = Writer::new();
    index.varint(ids.len() as u64);
    for id in &ids {
        index.id(id);
        index.u8(kind as u8);
        index.u64(out.len() as u64);
        index.varint(claimed);
        index.varint(stored);
        let mut entry = Writer::new();
        entry.u8(kind as u8);
        entry.id(id);
        entry.u64(claimed);
        entry.u64(stored);
        entry.u8(1); // zstd
        out.extend_from_slice(&entry.into_bytes());
        out.extend_from_slice(&frame);
    }
    let index = index.into_bytes();
    let index_offset = out.len() as u64;
    out.extend_from_slice(&index);
    out.extend_from_slice(&index_offset.to_le_bytes());
    let mut checksum = Writer::new(); // plain BLAKE3 of the index, which `hash_content` is
    checksum.id(&bolt_core::id::hash_content(&index));
    out.extend_from_slice(&checksum.into_bytes());
    out.extend_from_slice(&PACK_END_MAGIC);
    std::fs::write(root.join(".bolt/store/packs/00.pack"), out).unwrap();
}

/// Runs `f` on a thread of its own, failing if it takes longer than `secs` seconds.
fn within<T: Send + 'static>(secs: u64, f: impl FnOnce() -> T + Send + 'static) -> T {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || tx.send(f()));
    rx.recv_timeout(Duration::from_secs(secs))
        .unwrap_or_else(|e| panic!("no result within {secs} s: {e}"))
}

#[test]
fn rt2_7_a_planted_copy_claiming_a_huge_size_does_not_slow_save() {
    let env = Env::new();
    env.ok(&["start"]);
    let licence = b"MIT License\n\nPermission is hereby granted, free of charge, to any person\n";
    let id = bolt_core::id::hash_content(licence);
    // Records 1 TiB, takes about 32 MiB on disk.
    plant_huge_copies(env.root(), ObjectKind::Content, &[id], 1 << 40);
    std::fs::write(env.root().join("LICENSE"), licence).unwrap();
    let mut save = env.bolt();
    save.args(["save", "Add licence"]);
    let end = run_limited(save, 60);
    assert!(
        end.code == Some(0),
        "`bolt save` read the planted copy: {end:?}"
    );
    // Read back without knowing the size: the smallest copy must be tried first.
    let root = env.root().to_path_buf();
    let back = within(60, move || {
        let repo = Repository::open(&root).unwrap();
        let mut back = Vec::new();
        let read = repo.store().copy_content(&id, &mut back);
        read.map(|()| back).map_err(|e| e.to_string())
    });
    assert_eq!(back.as_deref(), Ok(&licence[..]));
}

#[test]
fn rt2_7_planted_copies_claiming_huge_sizes_do_not_slow_history() {
    let env = Env::new();
    env.ok(&["start"]);
    plant_chain(env.root(), 1000, 0, |_| {});
    let mut ids = Vec::new();
    let repo = Repository::open(env.root()).unwrap();
    bolt_core::history::for_each_on_line(&repo, |id, _, _| {
        ids.push(*id);
        Ok::<(), bolt_core::repo::RepoError>(())
    })
    .unwrap();
    drop(repo);
    // 512 MiB claimed per save, 16 KiB on disk each.
    plant_huge_copies(env.root(), ObjectKind::Save, &ids, 512 << 20);
    let mut history = env.bolt();
    history.arg("history");
    let end = run_limited(history, 60);
    assert!(
        end.code == Some(0),
        "`bolt history` read the planted copies: {end:?}"
    );
}

// RT2-8: after `save` has committed, it walks the whole line again (`short_of`), and an error
// there is printed with the hint "Nothing was changed", although the save was made.

#[test]
fn rt2_8_a_committed_save_is_never_reported_as_nothing_changed() {
    let env = Env::new();
    env.ok(&["start"]);
    let first = Repository::open(env.root())
        .unwrap()
        .current()
        .unwrap()
        .1
        .state
        .lines["main"];
    plant_chain(env.root(), 1, 0, |_| {});
    // The oldest save is damaged; the newest, which the new save builds on, is fine. Bolt's own
    // copy is damaged too, or reads would just pass the planted one over.
    plant_lying_pack(env.root(), ObjectKind::Save, &first, b"not a save");
    for e in std::fs::read_dir(env.root().join(".bolt/store/packs")).unwrap() {
        let path = e.unwrap().path();
        let found = bolt_core::pack::PackReader::open(&path)
            .unwrap()
            .find(&first)
            .unwrap();
        if let Some(entry) = found {
            let mut bytes = std::fs::read(&path).unwrap();
            let data = entry.offset + bolt_core::pack::ENTRY_HEADER_LEN;
            bytes[usize::try_from(data).unwrap()] ^= 1;
            std::fs::write(&path, bytes).unwrap();
        }
    }
    // Without the change-ID cache, the save reads the line to shorten its ID.
    std::fs::remove_file(env.root().join(".bolt/cache/change-ids")).unwrap();
    let before = Repository::open(env.root()).unwrap().head().unwrap();
    std::fs::write(env.root().join("mine.txt"), "my work").unwrap();
    let out = env.bolt().args(["save", "my work"]).output().unwrap();
    let after = Repository::open(env.root()).unwrap().head().unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.code() == Some(0)
            && after != before
            && stderr.contains("The save was made")
            && !stderr.contains("Nothing was changed"),
        "the save was committed, but Bolt said:\n{stderr}"
    );
}
