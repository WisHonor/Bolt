# Milestone 1 ("First save") Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `bolt start`, `bolt save`, `bolt history` and `bolt show` work end to end on real projects, on Bolt's real storage format, on Windows, macOS and Linux.

**Architecture:** A Cargo workspace with three crates. `bolt-vcs-core` is the engine library: fingerprints, the canonical encoding, records, pack files, the object store, the repository, saving a folder, and history. `bolt-vcs-platform` is the only crate allowed `unsafe`, for operating-system calls (in this milestone, only hiding `.bolt/` on Windows). `bolt-vcs` builds the `bolt` command. Every write to disk goes through one module, `fsio`, which also carries the fault-injection hooks the crash tests use.

**Tech Stack:** Rust (edition 2024, stable, pinned in `rust-toolchain.toml`), blake3, zstd, thiserror, getrandom, ignore, unicode-normalization, jiff, toml + serde, directories, clap, serde_json, windows-sys (platform crate only). Tests: proptest, tempfile, assert_cmd, insta. Fuzzing: cargo-fuzz.

**Spec:** [storage-format.md](../storage-format.md) (byte layouts), [design.md](../design.md) §5 (behaviour), [security.md](../security.md), [testing.md](../testing.md), [milestones.md](../milestones.md) (milestone 1), and decisions 004–012, 017, 019 and 022.

## Global Constraints

- Edition 2024; toolchain pinned to the exact stable version installed in Task 1 (Rust 1.89 or newer, for `std::fs::File::lock`).
- `#![forbid(unsafe_code)]` in `bolt-vcs-core` and `bolt-vcs`; `unsafe` only in `bolt-vcs-platform` (decision 022).
- Clippy's `unwrap_used`, `expect_used` and `panic` are denied outside tests; CI runs clippy with `-D warnings`.
- IDs are BLAKE3, 32 bytes; context strings exactly as storage-format.md §2 (for example `bolt-vcs 2026-10-31 folder-listing v1`).
- Encoding exactly as storage-format.md §1, with the limits in §3, §4, §9 and §11.
- `.bolt/format` is exactly `bolt-repository 1\nhash blake3\n`.
- Change IDs display as the letters `k`–`z` (half-byte n → `'k' + n`, high half first); screens show the shortest unique prefix, at least 4 letters. Fingerprints display as lowercase hex, at least 8 characters.
- Every write to disk goes through `fsio` (decision 022: one place per rule).
- Messages say what happened, why, and what to do next (decision 017).
- Tests pass on `windows-latest`, `macos-latest` and `ubuntu-latest`.
- One commit per task; no Claude attribution in commit messages.
- Anything on disk that storage-format.md doesn't describe gets a spec edit first; the tasks below list the known edits.

## Review Focus

1. **A file changes while `bolt save` reads it** (an editor or Unity saving at that moment). Expected: Bolt never records a half-written file; the save stops, names the file and asks to save again; nothing is written. Tests: Task 7 `content_that_changes_while_streaming_is_rejected`, Task 11 `save_stops_when_a_file_changes_during_the_save`.
2. **A file Bolt can't read** (no permission, or held open by another program on Windows). Expected: the save fails naming the file, and the repository stays exactly as before. Test: Task 11 `save_names_an_unreadable_file_and_changes_nothing`.
3. **File names that aren't valid UTF-8** (possible on Linux). Expected: the save refuses, shows the name readably, and never crashes. Test: Task 11 `save_refuses_non_utf8_names`.
4. **Symlinks that point outside the project or at their own folder.** Expected: the link itself is saved and never followed. Test: Task 11 `symlinks_are_saved_not_followed`.
5. **Running a command outside a project, or from a subfolder.** Expected: outside a project, a clear "not inside a Bolt project" message saying what to do; in a subfolder, the command works on the enclosing project. Tests: Task 12 `commands_outside_a_project_explain_what_to_do`, `commands_work_from_a_subfolder`.

---

## File structure

```text
Cargo.toml                           workspace, shared lints
rust-toolchain.toml                  pinned toolchain
clippy.toml                          unwrap/expect/panic allowed in tests
deny.toml                            cargo-deny rules
.gitattributes                       no line-ending conversion
.github/workflows/ci.yml             every push: fmt, clippy, tests on 3 OSes, cargo-deny
.github/workflows/nightly.yml        nightly fuzzing (Task 13)
crates/bolt-vcs-core/
  src/lib.rs                         module list, #![forbid(unsafe_code)]
  src/id.rs                          Id, ChangeId, Flavor, hashing, display
  src/encoding.rs                    Writer, Reader, DecodeError
  src/names.rs                       name rules and portability checks
  src/listing.rs                     folder listings
  src/save.rs                        saves
  src/diary.rs                       diary entries, line-name rules
  src/fsio.rs                        safe writes, retries, fault injection
  src/pack.rs                        pack writer and reader
  src/store.rs                       object store over every pack
  src/verify.rs                      integrity check
  src/repo.rs                        format file, open, head, lock, commit, init
  src/config.rs                      the user's name and email
  src/snapshot.rs                    saving the folder
  src/diff.rs                        what changed between two saves
  src/history.rs                     history and resolving saves
  tests/vectors/*.json               test vectors (storage-format.md §13)
  tests/vectors.rs                   checks every vector
crates/bolt-vcs-platform/src/lib.rs  hide_path, is_hidden
crates/bolt-vcs/
  src/main.rs                        command line (clap)
  src/output.rs                      human and --json output
  tests/cli.rs                       end-to-end tests
  tests/corpus.rs                    malicious-name cases end to end
fuzz/                                cargo-fuzz targets (not a workspace member)
```

Tests use small helpers `hex(&[u8]) -> String` and `unhex(&str) -> Vec<u8>`, defined once in a test-support module.

---

### Task 1: Workspace, rules and CI

**Files:**
- Create: `Cargo.toml`, `rust-toolchain.toml`, `clippy.toml`, `deny.toml`, `.gitattributes`, `.github/workflows/ci.yml`
- Create: `crates/bolt-vcs-core/{Cargo.toml,src/lib.rs}`, `crates/bolt-vcs-platform/{Cargo.toml,src/lib.rs}`, `crates/bolt-vcs/{Cargo.toml,src/main.rs}`
- Modify: `.gitignore` (add `/target/`)

**Interfaces:**
- Produces: packages `bolt-vcs-core` (lib name `bolt_core`), `bolt-vcs-platform` (lib name `bolt_platform`), `bolt-vcs` (binary `bolt`); a cargo feature `fault-injection` on `bolt-vcs-core`, which `bolt-vcs` forwards with its own `fault-injection` feature.

- [ ] **Step 1: Confirm the toolchain.** Run `rustc --version` and `cargo --version`. Expected: both print a version, 1.89 or newer. If either is missing, stop and ask the owner to install Rust. Write the exact version into `rust-toolchain.toml`:

```toml
[toolchain]
channel = "<the exact version printed>"
components = ["rustfmt", "clippy"]
```

- [ ] **Step 2: Write `.gitattributes`** containing the single line `* -text`, and add `/target/` to `.gitignore`.

- [ ] **Step 3: Write the workspace `Cargo.toml`** with `members = ["crates/*"]`, `resolver = "3"`, `[workspace.package] edition = "2024"`, and shared lints:

```toml
[workspace.lints.rust]
unsafe_code = "forbid"

[workspace.lints.clippy]
unwrap_used = "deny"
expect_used = "deny"
panic = "deny"
```

`bolt-vcs-core` and `bolt-vcs` use `[lints] workspace = true`. `bolt-vcs-platform` does not: it sets `unsafe_code = "allow"`, `unsafe_op_in_unsafe_fn = "deny"` and the same clippy lints. `clippy.toml` holds `allow-unwrap-in-tests = true`, `allow-expect-in-tests = true` and `allow-panic-in-tests = true`; if the installed clippy rejects the last key, remove it and start each test file with `#![allow(clippy::panic)]`.

- [ ] **Step 4: Write the three crates.** Each gets a crate doc comment. `bolt-vcs-core/src/lib.rs` starts with `#![forbid(unsafe_code)]` and holds one placeholder test:

```rust
#[test]
fn workspace_builds() {
    assert_eq!(2 + 2, 4);
}
```

`bolt-vcs/src/main.rs` prints `bolt: milestone 1 in progress`. The doc comment of `bolt-vcs-platform/src/lib.rs` says it is the only place `unsafe` is allowed, for operating-system calls, and that the owner reviews it line by line.

- [ ] **Step 5: Check locally.** Run `cargo fmt --check`, `cargo clippy --workspace --all-targets --all-features -- -D warnings` and `cargo test --workspace --all-features`. Expected: all pass.

- [ ] **Step 6: Write `deny.toml` and `ci.yml`.**
  - `deny.toml`: allow the licenses MIT, Apache-2.0, Apache-2.0 WITH LLVM-exception, BSD-2-Clause, BSD-3-Clause, ISC, Unicode-3.0, Zlib, CC0-1.0, MPL-2.0 and Unlicense; deny vulnerability advisories; warn on duplicate crate versions.
  - `ci.yml`: runs on `push` and `pull_request`.
    - A `test` job with matrix `os: [windows-latest, macos-latest, ubuntu-latest]` runs checkout, `Swatinem/rust-cache@v2`, `cargo fmt --check`, `cargo clippy --workspace --all-targets --all-features -- -D warnings` and `cargo test --workspace --all-features`.
    - A `deny` job on ubuntu runs `EmbarkStudios/cargo-deny-action@v2`.
    - The pinned toolchain installs itself from `rust-toolchain.toml`.

- [ ] **Step 7: Commit, push, check CI.** Run `git add -A`, `git commit -m "Set up the workspace, engineering rules and CI"`, then `git push`. Expected: every job is green on GitHub. If cargo-deny flags a license that isn't on the list, add it only if it is permissive, and say so in the commit message.

---

### Task 2: Fingerprints and change IDs

**Files:**
- Create: `crates/bolt-vcs-core/src/id.rs`
- Modify: `crates/bolt-vcs-core/src/lib.rs`, `crates/bolt-vcs-core/Cargo.toml` (blake3, getrandom, thiserror; dev: proptest)

**Interfaces:**
- Produces:
  - `pub struct Id(pub [u8; 32])` (Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug); `Id::to_hex(&self) -> String`; `Id::from_hex(s: &str) -> Option<Id>` (exactly 64 hex characters, either case); `impl Display` (lowercase hex).
  - `pub enum Flavor { FolderListing, Save, ChunkList, ProjectLink, Conflict, DiaryEntry }` with `Flavor::context(self) -> &'static str`.
  - `pub fn hash_content(bytes: &[u8]) -> Id`; `pub fn hash_record(flavor: Flavor, encoded: &[u8]) -> Id`; `pub struct ContentHasher` with `new() -> Self`, `update(&mut self, bytes: &[u8])`, `finish(self) -> Id`.
  - `pub struct ChangeId(pub [u8; 16])` (same derives as `Id`); `ChangeId::random() -> Result<ChangeId, getrandom::Error>`; `ChangeId::to_letters(&self) -> String` (32 letters); `ChangeId::from_letters(s: &str) -> Option<ChangeId>` (exactly 32 letters `k`–`z`).

- [ ] **Step 1: Write the failing tests** in `id.rs`:

```rust
#[test]
fn plain_hash_matches_the_official_blake3_empty_vector() {
    assert_eq!(hash_content(b"").to_hex(),
        "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262");
}

#[test]
fn streaming_hash_equals_one_shot() {
    let mut h = ContentHasher::new();
    h.update(b"ab");
    h.update(b"c");
    assert_eq!(h.finish(), hash_content(b"abc"));
}

#[test]
fn context_strings_are_exact() {
    assert_eq!(Flavor::FolderListing.context(), "bolt-vcs 2026-10-31 folder-listing v1");
    assert_eq!(Flavor::Save.context(), "bolt-vcs 2026-10-31 save v1");
    assert_eq!(Flavor::ChunkList.context(), "bolt-vcs 2026-10-31 chunk-list v1");
    assert_eq!(Flavor::ProjectLink.context(), "bolt-vcs 2026-10-31 project-link v1");
    assert_eq!(Flavor::Conflict.context(), "bolt-vcs 2026-10-31 conflict v1");
    assert_eq!(Flavor::DiaryEntry.context(), "bolt-vcs 2026-10-31 diary-entry v1");
}

#[test]
fn flavors_never_collide_with_each_other_or_plain() {
    let b = b"same bytes";
    let all = [hash_content(b), hash_record(Flavor::FolderListing, b),
               hash_record(Flavor::Save, b), hash_record(Flavor::DiaryEntry, b)];
    for i in 0..all.len() {
        for j in i + 1..all.len() {
            assert_ne!(all[i], all[j]);
        }
    }
}

#[test]
fn change_id_letters() {
    assert_eq!(ChangeId([0x00; 16]).to_letters(), "k".repeat(32));
    assert_eq!(ChangeId([0xff; 16]).to_letters(), "z".repeat(32));
    let mut b = [0u8; 16];
    b[0] = 0x0f;
    b[1] = 0xf0;
    assert!(ChangeId(b).to_letters().starts_with("kzzk"));
    assert_eq!(ChangeId::from_letters("kka"), None);
}

proptest! {
    #[test]
    fn hex_round_trips(bytes in any::<[u8; 32]>()) {
        prop_assert_eq!(Id::from_hex(&Id(bytes).to_hex()), Some(Id(bytes)));
    }

    #[test]
    fn letters_round_trip(bytes in any::<[u8; 16]>()) {
        prop_assert_eq!(ChangeId::from_letters(&ChangeId(bytes).to_letters()), Some(ChangeId(bytes)));
    }
}
```

- [ ] **Step 2: Run** `cargo test -p bolt-vcs-core id::`. Expected: FAIL, items not defined.

- [ ] **Step 3: Implement `id.rs`.** `hash_record` is `Id(blake3::derive_key(flavor.context(), encoded))`. `ChangeId::random` fills 16 bytes from the OS secure random source with the `getrandom` crate. Each byte becomes two letters, high half-byte first: `char::from(b'k' + half)`.

- [ ] **Step 4: Run** `cargo test -p bolt-vcs-core id::`. Expected: PASS.

- [ ] **Step 5: Commit:** `git commit -m "Add fingerprints and change IDs"`.

---

### Task 3: Encoding building blocks

**Files:**
- Create: `crates/bolt-vcs-core/src/encoding.rs`
- Modify: `crates/bolt-vcs-core/src/lib.rs`

**Interfaces:**
- Consumes: `Id`, `ChangeId` (Task 2).
- Produces:
  - `pub struct Writer` with `new()`, `u8`, `u32`, `u64`, `i16`, `i64`, `varint(u64)`, `bool`, `bytes(&[u8])`, `string(&str)`, `id(&Id)`, `change_id(&ChangeId)` and `into_bytes(self) -> Vec<u8>`.
  - `pub struct Reader<'a>` with `new(&'a [u8])` and readers returning `Result<_, DecodeError>`:
    - fixed width: `u8`, `u32`, `u64`, `i16`, `i64`;
    - `varint()`, and `count(limit: u64, field: &'static str)` (a varint that must be at most `limit`);
    - `bool`, `bytes(limit: usize, field) -> &'a [u8]`, `string(limit: usize, field) -> &'a str`;
    - `id`, `change_id`;
    - `finish(self) -> Result<(), DecodeError>`.
  - `pub enum DecodeError { UnexpectedEnd, TrailingBytes, NonMinimalVarint, VarintOverflow, InvalidBool(u8), InvalidUtf8, LimitExceeded { field: &'static str, limit: u64 }, Invalid(&'static str) }`, deriving thiserror, Debug, PartialEq and Eq, with plain-English messages.

- [ ] **Step 1: Write the failing tests:**

```rust
fn enc(v: u64) -> Vec<u8> {
    let mut w = Writer::new();
    w.varint(v);
    w.into_bytes()
}

#[test]
fn varint_known_encodings() {
    assert_eq!(enc(0), [0x00]);
    assert_eq!(enc(127), [0x7f]);
    assert_eq!(enc(128), [0x80, 0x01]);
    assert_eq!(enc(300), [0xac, 0x02]);
    assert_eq!(enc(u64::MAX), [0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x01]);
}

#[test]
fn varint_rejects_non_minimal_forms_and_overflow() {
    assert_eq!(Reader::new(&[0x80, 0x00]).varint(), Err(DecodeError::NonMinimalVarint));
    assert_eq!(Reader::new(&[0xff, 0x00]).varint(), Err(DecodeError::NonMinimalVarint));
    let mut too_big = vec![0xff; 9];
    too_big.push(0x02);
    assert_eq!(Reader::new(&too_big).varint(), Err(DecodeError::VarintOverflow));
    assert_eq!(Reader::new(&[0xff; 10]).varint(), Err(DecodeError::VarintOverflow));
    assert_eq!(Reader::new(&[0x80]).varint(), Err(DecodeError::UnexpectedEnd));
}

#[test]
fn fixed_width_is_little_endian() {
    let mut w = Writer::new();
    w.u32(1);
    w.i16(-2);
    w.i64(-1);
    assert_eq!(w.into_bytes(), [1, 0, 0, 0, 0xfe, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff]);
}

#[test]
fn readers_are_strict() {
    assert_eq!(Reader::new(&[0x02]).bool(), Err(DecodeError::InvalidBool(2)));
    assert_eq!(Reader::new(&[0x01, 0xff]).string(10, "name"), Err(DecodeError::InvalidUtf8));
    assert_eq!(Reader::new(&[0x05, b'a']).bytes(10, "value"), Err(DecodeError::UnexpectedEnd));
    assert_eq!(Reader::new(&[0x03, b'a', b'b', b'c']).string(2, "name"),
               Err(DecodeError::LimitExceeded { field: "name", limit: 2 }));
    assert_eq!(Reader::new(&[0x0b]).count(10, "entries"),
               Err(DecodeError::LimitExceeded { field: "entries", limit: 10 }));
    let mut r = Reader::new(&[0x00, 0x00]);
    r.u8().unwrap();
    assert_eq!(r.finish(), Err(DecodeError::TrailingBytes));
}

proptest! {
    #[test]
    fn varints_round_trip(v in any::<u64>()) {
        let bytes = enc(v);
        let mut r = Reader::new(&bytes);
        prop_assert_eq!(r.varint(), Ok(v));
        prop_assert_eq!(r.finish(), Ok(()));
    }

    #[test]
    fn strings_round_trip(s in ".{0,64}") {
        let mut w = Writer::new();
        w.string(&s);
        let bytes = w.into_bytes();
        prop_assert_eq!(Reader::new(&bytes).string(1024, "s"), Ok(s.as_str()));
    }
}
```

- [ ] **Step 2: Run** `cargo test -p bolt-vcs-core encoding::`. Expected: FAIL.

- [ ] **Step 3: Implement `encoding.rs`.** A varint reads at most 10 bytes. It rejects a final `0x00` byte that isn't the first byte, and a 10th byte that is above `0x01` or has the continuation bit set. Lengths are checked against the limit, then against the remaining input, before anything is allocated.

- [ ] **Step 4: Run** `cargo test -p bolt-vcs-core encoding::`. Expected: PASS.

- [ ] **Step 5: Commit:** `git commit -m "Add the strict encoding building blocks"`.

---

### Task 4: Names and paths

**Files:**
- Create: `crates/bolt-vcs-core/src/names.rs`
- Modify: `crates/bolt-vcs-core/src/lib.rs`, `crates/bolt-vcs-core/Cargo.toml` (unicode-normalization)

**Interfaces:**
- Produces:
  - `pub const MAX_NAME_BYTES: usize = 255;`
  - `pub enum NameError { Empty, DotOrDotDot, Slash, Nul, TooLong, BoltSpelling }` (thiserror, Debug, PartialEq, Eq).
  - `pub fn validate_name(name: &str) -> Result<(), NameError>`: the always-on rules (decision 004).
  - `pub enum PortabilityProblem { CaseCollision { a: String, b: String }, NormalizationCollision { a: String, b: String }, ReservedName(String), TrailingDotOrSpace(String), ForbiddenCharacter { name: String, ch: char } }` (Debug, PartialEq, Eq, Display in plain English).
  - `pub fn name_problems(name: &str) -> Vec<PortabilityProblem>`: checks on one name.
  - `pub fn folder_problems(names: &[&str]) -> Vec<PortabilityProblem>`: collisions among names in one folder, plus each name's own problems.

- [ ] **Step 1: Check the sources.** Before writing tests, read Microsoft's "Naming Files, Paths, and Namespaces" page (reserved names and forbidden characters) and Git's `is_ntfs_dotgit` / `is_hfs_dotgit` (`.git` spellings, including the characters macOS ignores). Wherever they differ from the lists below, follow the sources, adjust the tests, and record it in the commit message.

- [ ] **Step 2: Write the failing tests:**

```rust
#[test]
fn always_on_rules() {
    for ok in ["Player.cs", "README", "a b", "aux.c", "a:b", "a\\b", "\u{e9}", "名前"] {
        assert_eq!(validate_name(ok), Ok(()), "{ok}");
    }
    assert_eq!(validate_name(""), Err(NameError::Empty));
    assert_eq!(validate_name("."), Err(NameError::DotOrDotDot));
    assert_eq!(validate_name(".."), Err(NameError::DotOrDotDot));
    assert_eq!(validate_name("a/b"), Err(NameError::Slash));
    assert_eq!(validate_name("a\0b"), Err(NameError::Nul));
    assert_eq!(validate_name(&"x".repeat(256)), Err(NameError::TooLong));
    assert_eq!(validate_name(&"x".repeat(255)), Ok(()));
}

#[test]
fn every_bolt_spelling_is_refused() {
    for bad in [".bolt", ".BOLT", ".Bolt", ".bolt.", ".bolt ", ".bolt. .", "bolt~1", "BOLT~1",
                ".bolt::$INDEX_ALLOCATION", ".bolt:stream", ".b\u{200C}olt", "\u{FEFF}.bolt"] {
        assert_eq!(validate_name(bad), Err(NameError::BoltSpelling), "{bad:?}");
    }
    for ok in [".boltignore", "bolt.toml", ".bolts", "bolt"] {
        assert_eq!(validate_name(ok), Ok(()), "{ok}");
    }
}

#[test]
fn single_name_portability() {
    for reserved in ["CON", "con", "con.txt", "NUL", "AUX", "PRN", "COM1", "com9.log", "LPT1", "CONIN$", "CONOUT$"] {
        assert!(matches!(name_problems(reserved).as_slice(), [PortabilityProblem::ReservedName(_)]), "{reserved}");
    }
    assert!(matches!(name_problems("name.").as_slice(), [PortabilityProblem::TrailingDotOrSpace(_)]));
    assert!(matches!(name_problems("name ").as_slice(), [PortabilityProblem::TrailingDotOrSpace(_)]));
    for (name, ch) in [("a:b", ':'), ("a\\b", '\\'), ("a?b", '?'), ("a*b", '*'), ("a|b", '|'),
                       ("a<b", '<'), ("a>b", '>'), ("a\"b", '"'), ("a\u{1}b", '\u{1}')] {
        assert!(name_problems(name).contains(&PortabilityProblem::ForbiddenCharacter { name: name.into(), ch }), "{name}");
    }
    for fine in ["Player.cs", "console.txt", "COM10", "aux_data", "nul2"] {
        assert!(name_problems(fine).is_empty(), "{fine}");
    }
}

#[test]
fn sibling_collisions() {
    assert!(folder_problems(&["README", "readme"]).iter()
        .any(|p| matches!(p, PortabilityProblem::CaseCollision { .. })));
    assert!(folder_problems(&["\u{e9}", "e\u{301}"]).iter()
        .any(|p| matches!(p, PortabilityProblem::NormalizationCollision { .. })));
    assert!(folder_problems(&["a", "b", "A.txt"]).is_empty());
}
```

- [ ] **Step 3: Run** `cargo test -p bolt-vcs-core names::`. Expected: FAIL.

- [ ] **Step 4: Implement `names.rs`.**
  - **`.bolt` spellings:** remove the characters macOS ignores (U+200C–U+200F, U+202A–U+202E, U+206A–U+206F, U+FEFF), cut at the first `:`, trim trailing dots and spaces, lowercase the ASCII letters, then compare with `.bolt` and `bolt~1`.
  - **Reserved names:** take the text before the first `.`, trim trailing spaces, uppercase the ASCII letters, and compare with CON, PRN, AUX, NUL, COM0–COM9, LPT0–LPT9, COM¹, COM², COM³, LPT¹, LPT², LPT³, CONIN$ and CONOUT$, as corrected by Step 1.
  - **Case collisions:** compare the lowercase form of each NFC-normalized name. This is an approximation; NTFS uses its own uppercase table.
  - **Normalization collisions:** names that are equal after NFC but different as written.

- [ ] **Step 5: Run** `cargo test -p bolt-vcs-core names::`. Expected: PASS.

- [ ] **Step 6: Commit:** `git commit -m "Add name rules and portability checks"`.

---

### Task 5: Folder listings

**Files:**
- Create: `crates/bolt-vcs-core/src/listing.rs`, `crates/bolt-vcs-core/tests/vectors.rs`, `crates/bolt-vcs-core/tests/vectors/listing.json`
- Modify: `crates/bolt-vcs-core/src/encoding.rs` (add `DecodeError::InvalidName(NameError)`), `crates/bolt-vcs-core/src/lib.rs`, `crates/bolt-vcs-core/Cargo.toml` (dev: serde_json, serde)

**Interfaces:**
- Consumes: `Writer`, `Reader`, `DecodeError` (Task 3); `validate_name`, `NameError` (Task 4); `Id`, `Flavor`, `hash_record` (Task 2).
- Produces:
  - `#[repr(u8)] pub enum EntryKind { File = 1, Executable = 2, Symlink = 3, Folder = 4, ProjectLink = 5, Conflict = 6 }`, with `from_u8(u8) -> Option<EntryKind>` and `as_str(self) -> &'static str` (`file`, `executable`, `symlink`, `folder`, `project-link`, `conflict`).
  - `pub struct Entry { pub name: String, pub kind: EntryKind, pub id: Id, pub size: u64 }`.
  - `pub struct Listing` (private, always sorted by name bytes, unique and valid); `pub const MAX_ENTRIES: u64 = 1_000_000;`.
  - Constructors: `Listing::empty() -> Listing`; `Listing::new(entries: Vec<Entry>) -> Result<Listing, DecodeError>`, which sorts and rejects duplicates, invalid names, and a non-zero size on folders, project links and conflicts.
  - Accessors: `entries(&self) -> &[Entry]`; `get(&self, name: &str) -> Option<&Entry>`.
  - Encoding: `encode(&self) -> Vec<u8>`; `decode(bytes: &[u8]) -> Result<Listing, DecodeError>`, which rejects unsorted input rather than sorting it; `id(&self) -> Id`.

- [ ] **Step 1: Write the failing tests** in `listing.rs`:

```rust
fn entry_hex(name: &str, kind: u8, size: u8) -> String {
    format!("{:02x}{}{:02x}{}{:02x}", name.len(), hex(name.as_bytes()), kind, "00".repeat(32), size)
}

#[test]
fn empty_listing_bytes() {
    assert_eq!(hex(&Listing::empty().encode()), "0100");
}

#[test]
fn one_entry_bytes() {
    let l = Listing::new(vec![Entry { name: "a".into(), kind: EntryKind::File, id: Id([0; 32]), size: 5 }]).unwrap();
    assert_eq!(hex(&l.encode()), format!("0101{}", entry_hex("a", 1, 5)));
}

#[test]
fn decode_rejects_bad_listings() {
    let two = |a: &str, b: &str| unhex(&format!("0102{}{}", entry_hex(a, 1, 0), entry_hex(b, 1, 0)));
    assert!(Listing::decode(&two("b", "a")).is_err());                                   // unsorted
    assert!(Listing::decode(&two("a", "a")).is_err());                                   // duplicate
    assert!(Listing::decode(&unhex(&format!("0101{}", entry_hex("..", 1, 0)))).is_err()); // bad name
    assert!(Listing::decode(&unhex(&format!("0101{}", entry_hex("a", 7, 0)))).is_err());  // bad kind
    assert!(Listing::decode(&unhex(&format!("0101{}", entry_hex("a", 4, 5)))).is_err());  // folder with a size
    assert!(matches!(Listing::decode(&unhex("01c1843d")), Err(DecodeError::LimitExceeded { .. }))); // 1,000,001 entries
    assert!(Listing::decode(&unhex("0100ff")).is_err());                                 // trailing byte
}

proptest! {
    #[test]
    fn listings_round_trip(names in prop::collection::btree_set("[a-zA-Z0-9_.-]{1,12}", 0..20),
                           size in any::<u64>()) {
        let entries: Vec<Entry> = names.into_iter()
            .filter(|n| validate_name(n).is_ok())
            .map(|n| Entry { name: n, kind: EntryKind::File, id: Id([7; 32]), size })
            .collect();
        let l = Listing::new(entries).unwrap();
        prop_assert_eq!(Listing::decode(&l.encode()).unwrap().encode(), l.encode());
    }
}
```

- [ ] **Step 2: Run** `cargo test -p bolt-vcs-core listing::`. Expected: FAIL.

- [ ] **Step 3: Implement `listing.rs`** (layout: storage-format.md §3). `id()` is `hash_record(Flavor::FolderListing, &self.encode())`.

- [ ] **Step 4: Run** `cargo test -p bolt-vcs-core listing::`. Expected: PASS.

- [ ] **Step 5: Create the test vectors.**
  - `tests/vectors.rs` holds an ignored test, `print_listing_vectors`, which prints JSON for the vectors `empty-listing` (`Listing::empty()`) and `one-file` (the listing from `one_entry_bytes`): `[{"name", "hex", "id"}]`.
  - Run it with `cargo test -p bolt-vcs-core --test vectors -- --ignored --nocapture` and save the output as `tests/vectors/listing.json`. **The owner checks the bytes against storage-format.md §3 before this step is committed.**
  - The non-ignored test `listing_vectors_hold` decodes every vector, re-encodes it to the same hex, and checks the ID.

- [ ] **Step 6: Run** `cargo test -p bolt-vcs-core`. Expected: PASS.

- [ ] **Step 7: Commit:** `git commit -m "Add folder listings and their test vectors"`.

---

### Task 6: Saves

**Files:**
- Create: `crates/bolt-vcs-core/src/save.rs`, `crates/bolt-vcs-core/tests/vectors/save.json`
- Modify: `docs/storage-format.md` §4, `crates/bolt-vcs-core/src/lib.rs`, `crates/bolt-vcs-core/tests/vectors.rs`

**Interfaces:**
- Consumes: Tasks 2, 3, 5.
- Produces:
  - `pub struct Person { pub name: String, pub email: String, pub time_ms: i64, pub tz_offset_min: i16 }`
  - `pub struct MoveNote { pub from: String, pub to: String }`
  - `pub struct Save { pub root: Id, pub parents: Vec<Id>, pub author: Person, pub saver: Person, pub message: String, pub change_id: ChangeId, pub move_notes: Vec<MoveNote>, pub signature: Vec<u8>, pub extra: BTreeMap<String, Vec<u8>> }`
  - Constants: `MAX_PARENTS: u64 = 1024`, `MAX_MESSAGE_BYTES: usize = 1 << 20`, `MAX_MOVE_NOTES: u64 = 100_000`, `MAX_EXTRA_FIELDS: u64 = 64`, `MAX_EXTRA_VALUE_BYTES: usize = 64 * 1024`, `MAX_PERSON_FIELD_BYTES: usize = 4096`, `MAX_PATH_BYTES: usize = 4096`, `MAX_EXTRA_KEY_BYTES: usize = 256`, `EXTRA_LINE: &str = "line"`.
  - Methods:
    - `Save::encode(&self) -> Vec<u8>`: move notes are written sorted by `to`.
    - `Save::decode(bytes: &[u8], signatures_allowed: bool) -> Result<Save, DecodeError>`: rejects duplicate parents, unsorted or duplicate keys, and a non-empty signature unless allowed.
    - `Save::id(&self) -> Id`.
    - `Save::line(&self) -> Option<&str>`: the `line` extra field, if it is UTF-8.

- [ ] **Step 1: Edit the spec.** In storage-format.md §4, add the limits: person name and email at most 4 KiB each, move-note paths at most 4 KiB, extra-field keys at most 256 bytes.

- [ ] **Step 2: Write the failing tests:**

```rust
fn person() -> Person {
    Person { name: "A".into(), email: "a@b".into(), time_ms: 0, tz_offset_min: 0 }
}

fn minimal() -> Save {
    Save { root: Id([0; 32]), parents: vec![], author: person(), saver: person(), message: "m".into(),
           change_id: ChangeId([0; 16]), move_notes: vec![], signature: vec![], extra: BTreeMap::new() }
}

#[test]
fn minimal_save_bytes() {
    let p = format!("0141{}{}{}", "03614062", "00".repeat(8), "0000");
    let expected = format!("01{}00{}{}016d{}000000", "00".repeat(32), p, p, "00".repeat(16));
    assert_eq!(hex(&minimal().encode()), expected);
}

#[test]
fn decode_rejects_bad_saves() {
    let mut s = minimal();
    s.parents = vec![Id([1; 32]), Id([1; 32])];
    assert!(Save::decode(&s.encode(), false).is_err());                      // duplicate parents
    let mut s = minimal();
    s.signature = vec![1, 2, 3];
    assert!(Save::decode(&s.encode(), false).is_err());                      // signature without the feature
    assert!(Save::decode(&s.encode(), true).is_ok());
    let mut s = minimal();
    s.message = "x".repeat(MAX_MESSAGE_BYTES + 1);
    assert!(matches!(Save::decode(&s.encode(), false), Err(DecodeError::LimitExceeded { .. })));
    let mut s = minimal();
    s.parents = vec![Id([2; 32]); 1025];
    assert!(matches!(Save::decode(&s.encode(), false), Err(DecodeError::LimitExceeded { .. })));
    let mut bytes = minimal().encode();
    bytes.pop();                                                             // drop "0 extra fields"
    bytes.extend(unhex("02016200016100"));                                   // keys "b" then "a"
    assert!(Save::decode(&bytes, false).is_err());
}

#[test]
fn line_extra_field() {
    let mut s = minimal();
    s.extra.insert(EXTRA_LINE.into(), b"main".to_vec());
    assert_eq!(Save::decode(&s.encode(), false).unwrap().line(), Some("main"));
}
```

Also add a proptest, `saves_round_trip`, with random parents (distinct), messages, change IDs and extra fields within the limits.

- [ ] **Step 3: Run** `cargo test -p bolt-vcs-core save::`. Expected: FAIL.

- [ ] **Step 4: Implement `save.rs`** (layout: storage-format.md §4). `id()` is `hash_record(Flavor::Save, &self.encode())`.

- [ ] **Step 5: Run** `cargo test -p bolt-vcs-core save::`. Expected: PASS.

- [ ] **Step 6: Create the test vectors** `save.json`, the same way as Task 5 Step 5. Use two vectors: `minimal` (from the test above) and `with-parent-line-and-move` (one parent `Id([1; 32])`, extra `line` = `main`, one move note `a.txt` → `b.txt`). **The owner checks the bytes against §4 before committing.** Add `save_vectors_hold`.

- [ ] **Step 7: Run** `cargo test -p bolt-vcs-core`, then commit: `git commit -m "Add saves and their test vectors"`.

---

### Task 7: Safe writes and pack files

**Files:**
- Create: `crates/bolt-vcs-core/src/fsio.rs`, `crates/bolt-vcs-core/src/pack.rs`, `crates/bolt-vcs-core/tests/vectors/pack.json`
- Modify: `docs/storage-format.md` §11, `crates/bolt-vcs-core/src/lib.rs`, `crates/bolt-vcs-core/Cargo.toml` (zstd; dev: tempfile)

**Interfaces:**
- Consumes: Tasks 2, 3.
- Produces (fsio):
  - `pub fn create_temp(tmp_dir: &Path) -> io::Result<(File, PathBuf)>`: a new file named `tmp-<16 random hex>`, created with `create_new`.
  - `pub fn write_atomic(tmp_dir: &Path, dest: &Path, bytes: &[u8]) -> io::Result<()>`: temp file → write → `sync_all` → `fault_point("write:before-rename")` → `rename_durable`.
  - `pub fn rename_durable(from: &Path, to: &Path) -> io::Result<()>`: rename, retrying up to 10 times 50 ms apart when Windows reports a sharing violation (raw OS error 32) or access denied (5); then `sync_dir` on the parent folder.
  - `pub fn sync_dir(dir: &Path) -> io::Result<()>`: on Unix, open the folder and `sync_all`; on Windows, a no-op.
  - `pub fn fault_point(label: &'static str) -> io::Result<()>`.
    - Without the `fault-injection` feature, it always returns `Ok(())`.
    - With the feature, it counts calls. If `fault::arm(n)` was called on this thread and the thread's count reaches `n`, it returns `Err(io::Error::other(format!("injected fault at {label}")))`.
    - Otherwise, if the environment variable `BOLT_FAULT_AT=N` is set and the process-wide count reaches `N`, it prints `injected crash at {label}` to stderr and calls `std::process::abort()`.
  - `#[cfg(feature = "fault-injection")] pub mod fault { pub fn arm(n: u64); pub fn disarm(); }`
- Produces (pack):
  - `#[repr(u8)] pub enum ObjectKind { Content = 1, FolderListing = 16, Save = 17, ChunkList = 18, Conflict = 19, DiaryEntry = 20, ProjectLink = 21 }`, with `from_u8` and `flavor(self) -> Option<Flavor>` (None for `Content`).
  - Constants: `PACK_MAGIC = *b"BOLTPACK"`, `PACK_END_MAGIC = *b"BOLTPEND"`, `PACK_VERSION: u32 = 1`, `MAX_OBJECT_SIZE: u64 = 1 << 40`, `MAX_RECORD_SIZE: u64 = 512 << 20`, `ENTRY_HEADER_LEN: u64 = 50`.
  - `pub struct IndexEntry { pub id: Id, pub kind: ObjectKind, pub offset: u64, pub uncompressed_size: u64, pub stored_size: u64 }`
  - `pub struct PackWriter`:
    - `create(tmp_dir: &Path) -> Result<PackWriter, PackError>`;
    - `add_record(&mut self, kind: ObjectKind, encoded: &[u8]) -> Result<Id, PackError>` (any kind but `Content`);
    - `add_content(&mut self, bytes: &[u8]) -> Result<Id, PackError>`;
    - `add_content_from(&mut self, reader: &mut dyn Read, expected: Id) -> Result<u64, PackError>`: streams the content and returns its size. If the streamed bytes' fingerprint isn't `expected`, it returns `ContentChanged` and the partial entry is removed.
    - `contains(&self, id: &Id) -> bool`; `is_empty(&self) -> bool`;
    - `finish(self, packs_dir: &Path) -> Result<Option<PathBuf>, PackError>`: `None` when empty, with the temp file deleted.
  - `pub struct PackReader`:
    - `open(path: &Path) -> Result<PackReader, PackError>`;
    - `entries(&self) -> &[IndexEntry]` (sorted by id); `find(&self, id: &Id) -> Option<&IndexEntry>`;
    - `read_record(&self, entry: &IndexEntry) -> Result<Vec<u8>, PackError>`;
    - `copy_content(&self, entry: &IndexEntry, out: &mut dyn Write) -> Result<(), PackError>`: verifies at the end, so callers treat the output as untrusted until it returns `Ok`.
  - `pub enum PackError { Io(io::Error), Corrupt { path: PathBuf, reason: String }, FingerprintMismatch { id: Id }, ContentChanged, TooLarge }` (thiserror).

- [ ] **Step 1: Edit the spec.** In storage-format.md §11, the entry header's two sizes become fixed-width `u64` (a 50-byte header), so a streaming writer can fill the header in after writing the data. Add: "structured records are at most 512 MiB uncompressed".

- [ ] **Step 2: Write the failing tests** in `pack.rs`. Name each test and check what it states:
  - `round_trip_records_and_content`: add one listing record, one save record, empty content, 1 MiB of zeros and 1 MiB of random bytes; `finish`; `open`; every entry reads back equal, under its own ID.
  - `identical_objects_are_stored_once`: the same content added twice gives one index entry.
  - `empty_writer_writes_no_pack`: `finish` returns `None` and the temp folder is empty.
  - `pack_name_is_the_index_checksum`: the file name equals the hex BLAKE3 of the index bytes, plus `.pack`.
  - `every_single_byte_flip_is_detected`: for a pack holding two records and one content, flip bit 0 of each byte in turn. Opening and reading every object must then fail somewhere and never return different data.
  - `truncated_pack_is_refused`: without its last byte, `open` fails.
  - `decompression_bomb_is_refused`: a hand-built pack whose entry claims an uncompressed size of 10 but holds a zstd frame of 1 MiB of zeros. Reading fails with `Corrupt`, after producing at most 11 bytes.
  - `content_that_changes_while_streaming_is_rejected`: `add_content_from(&mut &b"new"[..], hash_content(b"old"))` returns `ContentChanged`. After `add_content(b"x")` and `finish`, the pack opens with exactly one entry.
  - `#[cfg(feature = "fault-injection")] crash_inside_finish_leaves_no_pack`: arm each fault point in `finish` in turn. The packs folder stays empty after every injected fault.

- [ ] **Step 3: Run** `cargo test -p bolt-vcs-core --features fault-injection pack::`. Expected: FAIL.

- [ ] **Step 4: Implement `fsio.rs` and `pack.rs`.**
  - **Header:** each entry's header is written as zeros first, then filled in by seeking back once the data is written.
  - **Compression:** zstd level 3.
    - In-memory data keeps its compressed form only if that is smaller than 95% of the original.
    - Streams decide from their first 64 KiB.
  - **Finishing:** `finish` writes the index and trailer, calls `sync_all`, `fault_point("pack:before-rename")`, `rename_durable`, then `fault_point("pack:after-rename")`.
  - **Opening:** `open` checks everything listed in storage-format.md §11, and checks that entries don't overlap.

- [ ] **Step 5: Run** `cargo test -p bolt-vcs-core --features fault-injection pack::`. Expected: PASS.

- [ ] **Step 6: Create the pack test vector.** Make a pack holding only `Listing::empty()` as a record; it is stored uncompressed, because it is too small to shrink. Save its hex and file name in `pack.json` (owner reviews against §11), and add `pack_vector_holds`, which writes the hex to a file and opens it.

- [ ] **Step 7: Commit:** `git commit -m "Add safe writes and pack files"`.

---

### Task 8: Object store and integrity check

**Files:**
- Create: `crates/bolt-vcs-core/src/store.rs`, `crates/bolt-vcs-core/src/verify.rs`
- Modify: `crates/bolt-vcs-core/src/lib.rs`, `docs/milestones.md` (milestone 1, step 7: the lookup index lives in memory for now; storing it in `cache/` waits until measurements show a need)

**Interfaces:**
- Consumes: Tasks 5–7.
- Produces:
  - `pub struct Store`:
    - `open(packs_dir: &Path) -> Result<Store, StoreError>`: opens every `*.pack` and ignores other files.
    - Lookup: `contains(&self, id: &Id) -> bool`; `kind(&self, id: &Id) -> Option<ObjectKind>`.
    - Reading: `read_record(&self, id: &Id, kind: ObjectKind) -> Result<Vec<u8>, StoreError>`; `copy_content(&self, id: &Id, out: &mut dyn Write) -> Result<(), StoreError>`.
    - Decoded reads: `listing(&self, id: &Id) -> Result<Listing, StoreError>`; `save(&self, id: &Id, signatures_allowed: bool) -> Result<Save, StoreError>`.
    - `register(&mut self, pack_path: &Path) -> Result<(), StoreError>`; `pack_count(&self) -> usize`.
  - `pub enum StoreError { Pack(PackError), Decode(DecodeError), Missing(Id), WrongKind { id: Id, expected: ObjectKind, found: ObjectKind } }`
  - `pub struct VerifyReport { pub objects_checked: u64, pub problems: Vec<String> }` with `is_ok(&self) -> bool`.
  - `pub fn verify_store(packs_dir: &Path) -> VerifyReport`: opens each pack on its own, so a broken pack becomes a listed problem rather than a crash. Every object is read; contents go to `io::sink()`.

- [ ] **Step 1: Write the failing tests:**
  - `random_stores_and_fetches` (proptest, 32 cases): up to 30 random contents (0–2048 bytes) spread over 1–3 packs, each finished and registered. A freshly opened `Store` reads every content back equal, and `contains` is true for each.
  - `overlapping_packs_are_fine`: the same object in two packs reads correctly.
  - `missing_object_is_an_error`: an unknown ID gives `Missing`.
  - `wrong_kind_is_an_error`: reading a content ID as `FolderListing` gives `WrongKind`.
  - `verify_catches_corruption`: an untouched store gives `is_ok()`. After a flipped byte in an entry's data, the report names exactly that pack.
  - `verify_reports_a_broken_pack_instead_of_failing`: with one pack truncated, its problem is listed and the other packs are still checked.

- [ ] **Step 2: Run** `cargo test -p bolt-vcs-core store:: verify::`. Expected: FAIL.

- [ ] **Step 3: Implement `store.rs` and `verify.rs`.** The in-memory map goes from `Id` to (pack number, entry number).

- [ ] **Step 4: Run** the same command. Expected: PASS.

- [ ] **Step 5: Commit:** `git commit -m "Add the object store and the integrity check"`.

---

### Task 9: Repository core: format file, diary, head, lock, commit

**Files:**
- Create: `crates/bolt-vcs-core/src/diary.rs`, `crates/bolt-vcs-core/src/repo.rs`, `crates/bolt-vcs-core/tests/vectors/diary.json`
- Modify: `docs/storage-format.md` §9 and §10, `crates/bolt-vcs-core/src/lib.rs`, `crates/bolt-vcs-core/tests/vectors.rs`

**Interfaces:**
- Consumes: Tasks 2–8.
- Produces (diary):
  - `pub enum Action { Command, Undo(Id), Redo(Id) }` (bytes 1, 2, 3; undo and redo carry their target entry).
  - `pub enum Position { OnLine(String), AtSave(Id) }`
  - `pub struct Tag { pub save: Id, pub message: String }`
  - `pub struct State { pub lines: BTreeMap<String, Id>, pub tags: BTreeMap<String, Tag>, pub position: Position, pub shelves: Vec<(String, Id)>, pub remotes: BTreeMap<String, BTreeMap<String, Id>> }`
  - `pub struct SafetyCopy { pub path: String, pub kind: EntryKind, pub id: Id, pub size: u64 }`
  - `pub struct DiaryEntry { pub parent: Option<Id>, pub action: Action, pub command: String, pub who: Person, pub bolt_version: String, pub state: State, pub safety_copies: Vec<SafetyCopy> }`, with `encode`, `decode(bytes: &[u8]) -> Result<DiaryEntry, DecodeError>` and `id`.
  - `pub fn validate_line_name(name: &str) -> Result<(), DecodeError>`:
    - allowed: Unicode letters and digits, `-`, `_`, `.`, `/`;
    - not empty, at most 255 bytes;
    - no leading or trailing `/`, no `//`, no `.` or `..` part.

    Decoding also requires line names to be unique ignoring letter case, and an `OnLine` position to name an existing line.
- Produces (repo):
  - `pub const FORMAT_TEXT: &str = "bolt-repository 1\nhash blake3\n";`
  - `pub struct FormatInfo { pub features: BTreeSet<String> }`; `pub fn parse_format(text: &str) -> Result<FormatInfo, RepoError>`. The first line must be `bolt-repository 1` and the second `hash blake3`; after that only `feature <name>` lines may follow, each ending in `\n`. This milestone knows no features, so any `feature` line gives `NewerFormat`; anything else malformed gives `Damaged`.
  - `pub struct Repository`:
    - Opening: `open(root: &Path) -> Result<Repository, RepoError>`. It reads and parses `format`, opens the store and deletes leftover files in `tmp/`.
    - Paths: `root(&self) -> &Path`; `tmp_dir(&self) -> &Path`; `packs_dir(&self) -> &Path`.
    - Store access: `store(&self) -> &Store`; `signatures_allowed(&self) -> bool`; `new_pack(&self) -> Result<PackWriter, RepoError>`.
    - Current state: `head(&self) -> Result<Id, RepoError>` (`state/head` must be exactly 64 lowercase hex characters and `\n`); `current(&self) -> Result<(Id, DiaryEntry), RepoError>`.
    - `lock_for_change(&self, command: &str, on_wait: &mut dyn FnMut(&str)) -> Result<ChangeLock, RepoError>`:
      - Take the lock on `state/lock` with `File::try_lock`.
      - If it is busy, read `state/lock-info` (best effort), call `on_wait` once with it, then block on `File::lock`.
      - Once the lock is held, write `state/lock-info` (`pid <n>\ncommand <c>\nstarted <unix ms>\n`) with `write_atomic`.
    - `commit(&mut self, lock: &ChangeLock, pack: PackWriter, entry: &DiaryEntry) -> Result<Id, RepoError>`:
      - Add the entry as a record, then `finish` and `register` the pack.
      - Call `fault_point("commit:before-head")`, then write `state/head` with `write_atomic`.
  - `pub enum RepoError { NotAProject, AlreadyAProject { root: PathBuf }, InsideAnotherProject { root: PathBuf }, NewerFormat { detail: String }, Damaged { detail: String }, Io(io::Error), Store(StoreError), Pack(PackError), Decode(DecodeError) }`, plus `hint(&self) -> Option<String>`. Messages:
    - `NotAProject`: "Not inside a Bolt project." Hint: "Run `bolt start` to make this folder a Bolt project."
    - `AlreadyAProject`: "This folder is already a Bolt project."
    - `InsideAnotherProject`: "This folder is inside a Bolt project at {root}." Hint: "Run Bolt commands there, or pick a folder outside it."
    - `NewerFormat`: "This project uses {detail}, which this version of Bolt doesn't understand." Hint: "Update Bolt to open it."
    - `Damaged`: "This project's Bolt data looks damaged: {detail}." Hint: "Nothing was changed. Ask for help before trying to repair it."

- [ ] **Step 1: Edit the spec.**
  - §9 limits:
    - line, tag and shelf names at most 255 bytes;
    - tag messages at most 64 KiB, the recorded command at most 64 KiB, the Bolt version at most 256 bytes;
    - at most 100,000 lines, 100,000 tags, 10,000 shelves, 64 remotes and 1,000,000 safety copies.
  - §10: the lock's information moves to a separate `state/lock-info`, because Windows doesn't let other processes read a locked file.

- [ ] **Step 2: Write the failing tests:**

```rust
#[test]
fn format_file() {
    assert!(parse_format(FORMAT_TEXT).unwrap().features.is_empty());
    for (text, needle) in [("bolt-repository 2\nhash blake3\n", "bolt-repository 2"),
                           ("bolt-repository 1\nhash sha256\n", "sha256"),
                           ("bolt-repository 1\nhash blake3\nfeature teleport\n", "teleport")] {
        let err = parse_format(text).unwrap_err();
        assert!(matches!(err, RepoError::NewerFormat { .. }) && err.to_string().contains(needle), "{text:?}");
    }
    for damaged in ["", "bolt-repository 1\n", "bolt-repository 1\r\nhash blake3\r\n", "hello\n"] {
        assert!(matches!(parse_format(damaged), Err(RepoError::Damaged { .. })), "{damaged:?}");
    }
}
```

Also write:
  - Diary tests in `diary.rs`:
    - `minimal_entry_bytes`: `parent: None`, `Command`, command `start`, `who` = `person()`, version `0.1.0`, lines `{main → Id([0; 32])}`, `OnLine("main")`, nothing else. The hex is `"01" "00" "01" "057374617274"`, then the person, then `"05302e312e30" "01" "046d61696e"`, then 32 zero bytes, then `"00" "01046d61696e" "00" "00" "00"`.
    - `diary_rejects_bad_entries`: an action byte of 4; two lines that differ only by case (`Main`, `main`); `OnLine("dev")` with no `dev` line; the line name `has space`; a parent count of 2.
    - `entries_round_trip` (proptest).
  - Repository tests in `repo.rs`, using a test helper that creates the folder layout and the `format` file by hand:
    - `head_round_trips` and `head_rejects_garbage` (`xyz\n` gives `Damaged`).
    - `#[cfg(feature = "fault-injection")] commit_is_all_or_nothing`: commit entry A, then for n = 1, 2, … arm n and try to commit entry B. After every injected fault, reopen: `head()` is A and `verify_store` is ok. Stop at the first n where the commit succeeds; then `head()` is B.
    - `second_changer_waits_and_is_told_why`: thread A holds the lock for 300 ms with command `save`. Thread B's `on_wait` is called exactly once with text containing `save`, and B gets the lock only after A releases it.
    - `readers_never_wait`: while A holds the lock, `head()` returns at once.

- [ ] **Step 3: Run** `cargo test -p bolt-vcs-core --features fault-injection diary:: repo::`. Expected: FAIL.

- [ ] **Step 4: Implement `diary.rs` and `repo.rs`** (layout: storage-format.md §9 and §10).

- [ ] **Step 5: Run** the same command. Expected: PASS.

- [ ] **Step 6: Create the test vector `diary.json`** from `minimal_entry_bytes`, the same way as Task 5 (the owner reviews it against §9), and add `diary_vectors_hold`.

- [ ] **Step 7: Commit:** `git commit -m "Add the diary, the head pointer, the change lock and commits"`.

---

### Task 10: `bolt start`

**Files:**
- Modify: `crates/bolt-vcs-core/src/repo.rs` (`init`, `discover`)
- Create: `crates/bolt-vcs-core/src/config.rs`, `crates/bolt-vcs/src/output.rs`, `crates/bolt-vcs/tests/cli.rs`
- Modify: `crates/bolt-vcs-platform/src/lib.rs`, `crates/bolt-vcs/src/main.rs`
- Cargo:
  - platform: `windows-sys` (Windows only; features `Win32_Storage_FileSystem`, `Win32_Foundation`)
  - core: `jiff`, `toml`, `serde`, `directories`
  - cli: `clap` (derive), `serde_json`; dev: `assert_cmd`, `insta` (json, filters), `tempfile`

**Interfaces:**
- Produces (platform):
  - `pub fn hide_path(path: &Path) -> std::io::Result<()>`: on Windows, adds `FILE_ATTRIBUTE_HIDDEN` with `GetFileAttributesW` and `SetFileAttributesW`; elsewhere, `Ok(())`.
  - `pub fn is_hidden(path: &Path) -> std::io::Result<bool>`.
- Produces (config):
  - `pub struct Identity { pub name: String, pub email: String }`
  - `pub fn load_identity(project_root: Option<&Path>) -> Result<Identity, ConfigError>`. It tries these in order:
    1. the environment variables `BOLT_USER_NAME` and `BOLT_USER_EMAIL`;
    2. `<root>/.bolt/config.toml`;
    3. `<user config dir>/config.toml`, where the user config dir is `BOLT_CONFIG_DIR` if set, else `directories::ProjectDirs::from("", "", "bolt")`.

    Each file holds `[user]` with `name` and `email`.
  - `pub fn user_config_path() -> Option<PathBuf>`
  - `pub enum ConfigError { MissingIdentity { path: Option<PathBuf> }, Invalid { path: PathBuf, detail: String } }`. `MissingIdentity` says "Bolt needs your name and email to record who made each save." Its hint shows the path and the snippet `[user]\nname = "Your Name"\nemail = "you@example.com"`.
  - `pub fn now_person(identity: &Identity) -> Person`: the current time in ms, and the local offset in minutes from `jiff::Zoned::now()`.
- Produces (repo):
  - `Repository::init(root: &Path, who: &Person) -> Result<Repository, RepoError>`:
    - **Refusals:** `AlreadyAProject` if `root/.bolt/format` exists; `InsideAnotherProject` if any ancestor folder has `.bolt/format`.
    - **Interrupted start:** if `root/.bolt` exists without `format`, an earlier start was interrupted; continue, reusing or overwriting what is there.
    - **Folders:** create `.bolt`, `store/packs`, `state`, `cache` and `tmp`; call `hide_path(.bolt)`; take the change lock.
    - **First pack:** the empty listing, plus the first save:
      - `root` = the empty listing's ID, no parents;
      - author = saver = `who`;
      - message `Start project`, a random change ID, extra `line` = `main`.
    - **Diary entry:** `Command`, command `start`, `who`, `env!("CARGO_PKG_VERSION")`, lines `{main → save}`, `OnLine("main")`.
    - **Finish:** commit, then write `.bolt/format` last with `write_atomic`.
  - `Repository::discover(start: &Path) -> Result<Repository, RepoError>`: walks up from `start` to the first folder with `.bolt/format`; else `NotAProject`.
- Produces (CLI):
  - Usage: `bolt [--json] <command>`.
  - Exit codes: 0 on success; 1 on a Bolt error; 2 on a usage error (clap's default).
  - Bolt errors go to stderr as `error: <message>` plus `hint: <hint>`. With `--json`, they go to stdout as `{"bolt_json":1,"error":{"message":…,"hint":…}}`.
  - `output::save_json(id: &Id, save: &Save, short: &str) -> serde_json::Value` gives `{"change_id","change_id_short","id","message","author":{"name","email","time_ms","tz_offset_min"},"saver":{…},"line"}`.
- Messages:
  - `bolt start` prints `Started a Bolt project in <root>.`, `Your first save is <short change id>, on line main.` and `Next: change some files, then run: bolt save "describe your change"`.
  - With `--json`: `{"bolt_json":1,"root":…,"line":"main","save":<save_json>}`.

- [ ] **Step 1: Write the failing tests** in `cli.rs`. Every command runs with `BOLT_USER_NAME=Test`, `BOLT_USER_EMAIL=test@example.com` and `BOLT_CONFIG_DIR=<empty temp dir>`, except where stated otherwise.
  - `start_creates_a_project`: exit 0; `.bolt/format` equals `FORMAT_TEXT`; `.bolt/state/head` exists; stdout contains `Started a Bolt project`.
  - `start_twice_is_refused`: exit 1; stderr contains `already a Bolt project`.
  - `start_inside_a_project_is_refused`: run in `sub/` of a project; exit 1; stderr contains `inside a Bolt project`.
  - `start_without_a_name_explains_how_to_set_it`: no `BOLT_USER_*` variables; exit 1; stderr contains `[user]` and `name =`.
  - `start_json_output`: an insta JSON snapshot, with the IDs, change IDs, times and root redacted.
  - `#[cfg(feature = "fault-injection")] interrupted_start_can_be_finished`: for n = 1, 2, …, run `bolt start` with `BOLT_FAULT_AT=n`. If the run aborted, a plain `bolt start` must then exit 0 and leave `verify_store` ok. Stop at the first n whose run doesn't abort.
  - `bolt_folder_is_hidden_on_windows` (`cfg(windows)`): `bolt_platform::is_hidden(.bolt)` is true.

- [ ] **Step 2: Run** `cargo test -p bolt-vcs --features fault-injection`. Expected: FAIL.

- [ ] **Step 3: Implement** the platform functions, `config.rs`, `init`, `discover`, `main.rs` (clap: `start`, `save`, `history`, `show`; for now `save`, `history` and `show` print "not ready yet" and exit 1) and `output.rs`. **The owner reviews `bolt-vcs-platform` line by line before this task is committed.**

- [ ] **Step 4: Run** `cargo test --workspace --all-features`. Expected: PASS.

- [ ] **Step 5: Commit:** `git commit -m "Add bolt start"`.

---

### Task 11: `bolt save`

**Files:**
- Create: `crates/bolt-vcs-core/src/snapshot.rs`, `crates/bolt-vcs-core/src/diff.rs`
- Modify: `docs/storage-format.md` §12, `crates/bolt-vcs-core/src/lib.rs`, `crates/bolt-vcs-core/Cargo.toml` (`ignore`), `crates/bolt-vcs/src/main.rs`, `crates/bolt-vcs/src/output.rs`, `crates/bolt-vcs/tests/cli.rs`

**Interfaces:**
- Consumes: everything above.
- Produces (diff):
  - `pub enum ChangeKind { Added, Removed, Modified, KindChanged }`
  - `pub struct FileChange { pub path: String, pub change: ChangeKind, pub kind: EntryKind, pub old_size: Option<u64>, pub new_size: Option<u64> }`
  - `pub fn diff_saves(store: &Store, old_root: Option<Id>, new_root: Id) -> Result<Vec<FileChange>, StoreError>`: sorted by path. Folders are walked into, not listed; files in an added or removed folder are listed one by one.
- Produces (snapshot):
  - `pub struct SaveRequest<'a> { pub message: &'a str, pub who: &'a Person, pub command: &'a str }`
  - `pub enum SaveOutcome { Saved { save_id: Id, save: Save, changed_files: usize }, NothingChanged { current: Id } }`
  - `pub fn save_folder(repo: &mut Repository, req: &SaveRequest, on_wait: &mut dyn FnMut(&str)) -> Result<SaveOutcome, SaveError>`
  - `pub enum SaveError { Repo(RepoError), Store(StoreError), Pack(PackError), Unreadable { path: String, source: io::Error }, ChangedDuringSave { path: String }, InvalidName { path: String, reason: String }, NotPortable { problems: Vec<(String, PortabilityProblem)> }, Ignore { detail: String }, NotOnALine }`
  - `pub(crate) fn store_file(pack: &mut PackWriter, store: &Store, path: &Path, between_passes: &mut dyn FnMut()) -> Result<(Id, u64), SaveError>`. It works in two passes:
    1. a hashing pass with `ContentHasher` and a 1 MiB buffer;
    2. a storing pass with `add_content_from` against the hash from pass 1, skipped if `store` or `pack` already holds the ID.

    `between_passes` runs between the two passes; production passes a no-op. If the size or modification time differs before and after, or pass 2 gets `ContentChanged`, the result is `ChangedDuringSave`.

How saving works:
- **Walking the folder:** `ignore::WalkBuilder` with `hidden(false)`, `git_ignore(false)`, `git_exclude(false)`, `git_global(false)`, `ignore(false)`, `parents(false)`, `follow_links(false)` and `add_custom_ignore_filename(".boltignore")`, plus a filter that drops `.bolt` and `.git` folders at any depth. Listings are built bottom-up, deepest folders first.
- **Entry kinds:**
  - On Unix, a file is executable if it has any `x` bit. On Windows, it is executable only if the parent save had that path as executable (decision 004).
  - Symlinks are detected with `symlink_metadata` and read with `read_link`. The target must be valid UTF-8 and is stored as content.
  - Empty folders are kept.
- **Name checks:** a name that isn't valid UTF-8 gives `InvalidName`, with the path shown lossily. Names not in the parent save run through `names::folder_problems` folder by folder; any problem gives `NotPortable`. Either way, nothing is written.
- **The save:**
  - Parents = [the line's current save]; author = saver = `who`; a random change ID; extra `line` = the current line.
  - The diary entry is `Command` with the line moved to the new save.
  - It is committed under the lock.
  - `AtSave` gives `NotOnALine`, which can't happen in this milestone.
  - `NothingChanged` when the new root equals the parent's root.
  - `changed_files` = `diff_saves(parent root, new root).len()`.
- **CLI:** `bolt save "message"`; an empty message is refused with a hint.
  - On success it prints `Saved <short>  "<first line of message>"  (<n> files)`.
  - With nothing changed, it prints `Nothing to save: no changes since your last save (<short>).`
  - Errors list one path per line.
  - JSON output is `{"bolt_json":1,"result":"saved","save":<save_json>,"changed_files":n}` or `{"bolt_json":1,"result":"nothing-changed"}`.

- [ ] **Step 1: Edit the spec.** storage-format.md §12: `.boltignore` files work like `.gitignore` files: one at the project root, and optionally more in subfolders.

- [ ] **Step 2: Write the failing tests** (core tests go in `snapshot.rs`, using a project made with `Repository::init`):
  - `saved_files_read_back_byte_identical`: nested folders, an empty folder, an empty file, a 3 MiB file and Unicode names. After saving, walk the save's listings and `copy_content` every file: the bytes are equal, and the empty folder is a `Folder` whose listing is empty.
  - `random_folders_round_trip` (proptest, 32 cases): random trees with names from `[a-z0-9_.-]{1,10}` that pass `validate_name`, with random contents.
  - `second_save_without_changes_is_nothing_changed`.
  - `boltignore_git_and_bolt_folders_are_skipped`: `.boltignore` holds `*.tmp` and `Library/`. Files `a.tmp`, `Library/x` and `.git/config` are absent from the save; `.boltignore` itself is present.
  - `#[cfg(unix)] executable_flag_is_saved`.
  - `#[cfg(unix)] symlinks_are_saved_not_followed`: `loop -> .` and `out -> /` are `Symlink` entries whose content is the target bytes, and the walk never entered them. On Windows, run the same test only when `std::os::windows::fs::symlink_dir` succeeds.
  - `#[cfg(unix)] save_refuses_non_portable_new_names`: `README` with `readme`, `a:b` and `CON` give `NotPortable` listing all three; `head()` is unchanged.
  - `#[cfg(unix)] save_refuses_non_utf8_names`: a file named `b"f\xff"` gives `InvalidName` with the path `f\u{fffd}`; `head()` is unchanged.
  - `save_names_an_unreadable_file_and_changes_nothing`: on Unix, `chmod 000` (skipped when running as root); on Windows, the file is held open with `share_mode(0)`. The result is `Unreadable` naming the file, and `head()` is unchanged.
  - `save_stops_when_a_file_changes_during_the_save`: `store_file` with a `between_passes` that appends to the file gives `ChangedDuringSave`.
  - CLI tests: `save_then_nothing_to_save`, `save_requires_a_message`, `save_json_output` (insta, redacted), and `#[cfg(feature = "fault-injection")] interrupted_save_changes_nothing`. The last runs `bolt save` with `BOLT_FAULT_AT=n` for n = 1, 2, …. After each aborted run, `Repository::current` still shows line `main` at the first save (`history` doesn't exist until Task 12), and `verify_store` is ok. After the loop, a plain `bolt save` succeeds.

- [ ] **Step 3: Run** `cargo test --workspace --all-features`. Expected: the new tests FAIL.

- [ ] **Step 4: Implement** `diff.rs`, `snapshot.rs` and the `save` command.

- [ ] **Step 5: Run** `cargo test --workspace --all-features`. Expected: PASS.

- [ ] **Step 6: Commit:** `git commit -m "Add bolt save"`.

---

### Task 12: `bolt history` and `bolt show`

**Files:**
- Create: `crates/bolt-vcs-core/src/history.rs`
- Modify: `crates/bolt-vcs-core/src/lib.rs`, `crates/bolt-vcs/src/main.rs`, `crates/bolt-vcs/src/output.rs`, `crates/bolt-vcs/tests/cli.rs`

**Interfaces:**
- Consumes: `Repository`, `Store`, `Save`, `diff_saves`, `FileChange`.
- Produces:
  - `pub fn line_history(repo: &Repository) -> Result<Vec<(Id, Save)>, RepoError>`: follows first parents from the current line's save, newest first.
  - `pub fn short_change_ids(saves: &[(Id, Save)]) -> HashMap<Id, String>`: the shortest unique prefix among these saves, at least 4 letters.
  - `pub fn resolve(repo: &Repository, text: &str) -> Result<Id, ResolveError>`: tried in this order:
    1. a line name gives that line's save;
    2. only the letters `k`–`z` are read as a change-ID prefix, which must be unique among the line's saves;
    3. hex of at least 8 characters is read as a fingerprint prefix.
  - `pub enum ResolveError { NotFound(String), Ambiguous { text: String, candidates: Vec<String> }, Repo(RepoError) }`
- CLI:
  - `bolt history` prints one line per save: `<short>  <relative time>  <author name>  <first line of message>`.
  - `bolt show [save]` defaults to the current save. It prints a header `Save <short> · <author> · <relative time>`, then the message, then each change:
    - `  added     <path>  (<size>)`
    - `  removed   <path>  (<size>)`
    - `  modified  <path>  (<old> → <new>)`
    - `  changed   <path>  (<old kind> → <new kind>)`
    - `  (no file changes)` when there are none
  - Relative times: `just now`, `N min ago`, `N h ago`, `N days ago`; after 30 days, the date (`2026-10-31`).
  - Sizes: bytes below 1 KiB, otherwise KB, MB or GB with one decimal (1024-based).
  - JSON:
    - history: `{"bolt_json":1,"line":"main","saves":[<save_json>…]}`
    - show: `{"bolt_json":1,"save":<save_json>,"changes":[{"path","change":"added|removed|modified|kind-changed","kind","old_size","new_size"}]}`

- [ ] **Step 1: Write the failing tests:**
  - `history_lists_newest_first`: after start, save `one` and save `two`, the order is `two`, `one`, `Start project`.
  - `show_lists_what_a_save_changed`: add `a.txt`, change `b.txt` and delete `c.txt`, then save. `show` lists exactly: added `a.txt`, modified `b.txt`, removed `c.txt`.
  - `show_resolves_lines_prefixes_and_fingerprints`: resolve by `main`, by a 4-letter change prefix, and by an 8-character hex prefix.
  - `ambiguous_prefix_lists_candidates` (core test): two saves whose change IDs share their first letters give `Ambiguous` with both candidates.
  - `history_and_show_json_output`: insta snapshots, redacted.
  - `commands_outside_a_project_explain_what_to_do`: `bolt history` in an empty temp folder exits 1; stderr contains `Not inside a Bolt project` and `bolt start`.
  - `commands_work_from_a_subfolder`: `bolt save` and `bolt history` run from `project/sub` work on the project.

- [ ] **Step 2: Run** `cargo test --workspace --all-features`. Expected: the new tests FAIL.

- [ ] **Step 3: Implement** `history.rs` and the two commands.

- [ ] **Step 4: Run** `cargo test --workspace --all-features`. Expected: PASS.

- [ ] **Step 5: Commit:** `git commit -m "Add bolt history and bolt show"`.

---

### Task 13: Fuzzing, the corpus and review

**Files:**
- Create: `fuzz/Cargo.toml`, `fuzz/fuzz_targets/{decode_listing,decode_save,decode_diary,open_pack,name_rules,format_file}.rs`, `.github/workflows/nightly.yml`, `crates/bolt-vcs/tests/corpus.rs`
- Modify: `docs/milestones.md`, `docs/design.md`

**Interfaces:**
- Consumes: `Listing::decode`, `Save::decode`, `DiaryEntry::decode`, `PackReader::open`, `validate_name`, `folder_problems`, `parse_format`.

- [ ] **Step 1: Write the fuzz targets.**
  - Each target feeds arbitrary bytes to one decoder or parser and must never panic or hang.
  - `open_pack` writes the bytes to a temp file, opens it, and reads every entry.
  - `name_rules` runs `validate_name` and `folder_problems` on arbitrary UTF-8 split at `\n`.

- [ ] **Step 2: Write `nightly.yml`.** It runs nightly on ubuntu with the nightly toolchain and `cargo install cargo-fuzz`. It runs each target with `-max_total_time=300` and uploads any crash files as artifacts. Trigger it once by hand (`workflow_dispatch`). Expected: every target runs to its time limit without a crash.

- [ ] **Step 3: Write `corpus.rs`.** Run every malicious-name case from Task 4 end to end through `bolt save`, using whichever names the OS can create (most on Linux and macOS, fewer on Windows). Every case exits 1 with the name in the message, and `bolt history` still shows the previous save.

- [ ] **Step 4: Run** `cargo test --workspace --all-features`, then push. Expected: CI is green on all three systems.

- [ ] **Step 5: End-to-end check.** Do this on a copy of one of the owner's code projects and on an open-source Unity project:
  1. `bolt start`
  2. `bolt save "first"`
  3. change one file
  4. `bolt save "second"`
  5. `bolt history`
  6. `bolt show`

  Record the output and timings in the commit message or PR description.

- [ ] **Step 6: Owner review.** The owner reads `fsio.rs`, `pack.rs`, `repo.rs`, `snapshot.rs` and all of `bolt-vcs-platform`, and approves them.

- [ ] **Step 7: Close the milestone.** Mark milestone 1 done in `docs/milestones.md` (with the date), set design.md's phase to "building milestone 2", and commit: `git commit -m "Finish milestone 1: first save"`.
