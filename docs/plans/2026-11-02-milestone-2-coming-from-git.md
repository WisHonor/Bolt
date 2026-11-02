# Milestone 2 ("Coming from Git") Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `bolt import` turns a Git repository's whole history (every branch and tag) into Bolt saves, in place next to `.git/`, exactly and repeatably, and differential tests compare every imported commit with Git, on Windows, macOS and Linux.

**Architecture:** A read-only Git reader (module `git` in `bolt-vcs-core`: the repository's format and config, refs, loose objects, packs and deltas, object parsing) feeds an importer (`import.rs`) that turns blobs, trees, commits and tags into Bolt objects. A command may now spread its objects over several packs. The diary keeps tags and server lines in two shared records. `save` learns Git's ignore rules exactly (a hand-written matcher reading `.gitignore` and `.boltignore`), keeps files that are already saved, and keeps what this computer can't hold. The CLI gets `bolt import`.

**Tech Stack:** Rust 1.99.1 (`rust-toolchain.toml`), the existing crates, and four new ones, each reviewed under security.md T18: `miniz_oxide` (zlib; forbids `unsafe` itself), `sha1-checked` (SHA-1 with collision detection, as Git's sha1dc), `sha2` (SHA-256 repositories), `encoding_rs` (commit messages in a declared legacy encoding, decision 019 #7). Tests use the `git` program as the reference.

**Spec:** decisions [030](../decisions/030-administrators-only-when-elevated.md) to [041](../decisions/041-several-packs-per-command.md), with 002, 004, 019, 020 and 022–029; [design.md](../design.md) §5.16; [storage-format.md](../storage-format.md) §2–§5, §7, §9, §11, §12; [security.md](../security.md) §4 and §7; [testing.md](../testing.md); the milestone 2 rows of [the follow-ups](2026-11-01-milestone-1-follow-ups.md).

## Global Constraints

- Everything in CLAUDE.md applies.
  - `#![forbid(unsafe_code)]` in the engine. `unsafe` only in `bolt-vcs-platform`, each block with a `// SAFETY:` comment. **If a task needs new `unsafe` code, stop and walk the owner through it line by line before going on.** No task below is expected to need any.
  - No `unwrap`, `expect` or `panic` outside tests. One place per rule. Spec before code. Tests pass on Windows, macOS and Linux.
- Code comments are short and human: the why in a line or two, no process words.
- Every module that parses Git data (`git/*.rs`) and the wildmatch code in `ignore.rs` start with `#![deny(clippy::indexing_slicing, clippy::arithmetic_side_effects)]`.
  - Sizes, offsets and counts from a file are checked against the file's length before anything is allocated.
  - Nothing recurses once per tree level, per delta in a chain, or per wildcard.
- Every file under `.git/` is opened through `safe_fs`: no link is followed, only regular files are read, and every read is bounded. Nothing is memory-mapped.
- Product code never runs `git` or `git-lfs` and never fetches. Tests may run `git` as the reference, only on repositories the tests build themselves. The hostile corpus is read only by Bolt; Git runs on it only inside the red-team sandbox.
- Untrusted text is escaped before printing (`text::escape`, T21): ref names, paths, author names, messages.
- Messages say what happened, why, and what to do next (decision 017). `--json` documents follow `docs/json.md` (Task 2).
- **Limits** (from the decisions):

  | What | Limit |
  |---|---|
  | A deltified object, and its base | 512 MiB each; the delta's declared sizes are checked before anything is allocated |
  | Delta chain | 4,096 deep; Git's own cap is 4,095 |
  | Delta base cache | 256 MiB |
  | Blobs read whole | up to 64 MiB; bigger ones are streamed twice: once to check, once into the pack |
  | Commit message | 1 MiB |
  | A message that isn't UTF-8 | 64 KiB, since its original bytes must fit an extra field |
  | `git-header` | 64 KiB |
  | Extra fields | 64 KiB a value, 64 fields |
  | Parents | 1,024 |
  | Snapshot | 4,096 folder levels; 10,000,000 entries, counting every path |
  | Names | 255 bytes |
  | Lines; servers; lines per server | 100,000; 64; 100,000 |
  | Tags | 100,000; a tag list at most 512 MiB; a Git tag object at most 1 MiB; at most 16 tag objects per chain |
  | Packs | a writer starts a new one at 2^20 objects; the format's limit stays 2^24 |
  | An LFS pointer | at most 1,024 bytes |

- **Fixed values:**
  - **Context strings:** `bolt-vcs 2026-11-02 tag-list v1` and `bolt-vcs 2026-11-02 server-lines v1`. **Object kinds:** tag list 22, server lines 23.
  - **Extra fields:** `git-commit` (hex), `git-header` (raw bytes), `git-original-message` (raw bytes).
  - **Change IDs of imported saves:** the commit's first `change-id` header, if it decodes to 16 bytes in Jujutsu's letters (`z` = 0 … `k` = 15). Otherwise the last 16 bytes of the commit ID, in reverse order, with each byte's bits reversed.
  - **Renaming rule** (decision 033), applied to line, server-line and server names:
    1. each character Bolt doesn't allow becomes `-` (bytes that aren't UTF-8 become U+FFFD, and so `-`);
    2. a name over 255 bytes is cut to 255 at a character boundary;
    3. a name equal to an earlier one, or differing only by letter case (`to_lowercase`), gets `-2`, `-3`… in the byte order of the original names; a suffixed name must itself be free.

    Tag names use the same rule, except that any valid UTF-8 is allowed.
- **Dependencies:** only the four above. Any other needs the owner's approval.
- One commit per task unless a step says otherwise, and no Claude attribution in commit messages.

## Review Focus

1. **Git changes the repository during an import** (a `git gc` or a commit in another window). Expected: the import stops with "Git changed the repository while Bolt read it; run bolt import again", and nothing is committed. Test: Task 13 `import_stops_cleanly_when_git_removes_a_pack`.
2. **A Linux history holding names Windows can't hold** (`aux.c`, `a:b`, `Readme` beside `README`), imported and then saved on Windows. Expected: the import keeps the names, and the first save keeps them exactly as they were. Test: Task 15 `windows_cant_hold_some_names_and_saving_keeps_them`, on all three systems.
3. **Huge declared sizes:** a loose object or delta claiming terabytes. Expected: refused before allocating, with a message naming the object. Tests: Task 10 `a_huge_declared_size_is_refused_before_allocating` and `a_delta_claiming_a_huge_result_is_refused_before_allocating`.
4. **An interrupted import** (Ctrl+C or a crash after some packs). Expected: no project yet. Running `bolt import` again finishes, reusing what was stored after checking it, and gives the same saves. Test: Task 13 `an_interrupted_import_can_be_run_again`.
5. **Git's optional files** (commit-graph, multi-pack index, bitmaps, `.rev` files, worktree config, only packed refs). Expected: ignored, and the import matches Git. Test: Task 15 `optional_git_files_are_ignored`.

---

## File structure

```text
crates/bolt-vcs-platform/src/lib.rs   Owner::Administrators { elevated } (Task 1)
crates/bolt-vcs-core/src/
  warning.rs     Warning: one type for every warning, printed and in --json (Task 2)
  diary.rs       TagList and ServerLines records; State points to them (Task 3)
  pack.rs        PackSet: one command's packs, a new one every 2^20 objects (Task 4)
  names.rs       is_git_alias, holdable_here (Task 5)
  drive.rs       Drive::probe: this drive's letter case and Unicode form (Task 5)
  ignore.rs      Git's wildmatch and ignore rules, nothing compiled (Task 6)
  snapshot.rs    .gitignore and .boltignore; kept entries; the line-ending stop (Tasks 7, 8, 14)
  git/mod.rs     Repo::open: the repository checks and refusals; GitId, GitError (Task 9)
  git/config.rs  the allow-listed keys of .git/config (Task 9)
  git/refs.rs    HEAD, loose refs, packed-refs, ref-name rules (Task 9)
  git/odb.rs     finding and reading objects, loose and packed (Task 10)
  git/pack.rs    .idx version 2 and .pack reading, deltas (Task 10)
  git/object.rs  commits, trees, tags; the LFS pointer test; rebuilding a commit (Task 11)
  git/craft.rs   writes Git repositories byte by byte, for tests (feature `test-support`; Tasks 9, 10)
  import.rs      Git objects and refs to Bolt (Tasks 12, 13)
crates/bolt-vcs/src/{main.rs,output.rs}   bolt import; save --accept-crlf; warnings in --json
crates/bolt-vcs/tests/
  support/mod.rs     shared helpers: bolt(), git() with a clean environment, planting saves
  import.rs          bolt import end to end (Task 13)
  import_vs_git.rs   differential tests (Task 15)
  ignore_vs_git.rs   the matcher against git check-ignore (Task 6)
  corpus_git.rs      hostile Git repositories (Task 16)
docs/json.md        the --json formats (Task 2)
fuzz/fuzz_targets/  decode_tag_list, decode_server_lines (Task 3), ignore_rules (Task 6),
                    git_loose, git_pack, git_delta, git_objects, git_refs, git_config (Task 16)
```

---

### Task 1: The Administrators group counts only in an elevated terminal

Decision 030.

**Files:**
- Modify: `crates/bolt-vcs-platform/src/lib.rs` (`Owner`, `windows_owner::owner_of`)
- Modify: `crates/bolt-vcs-core/src/repo.rs` (`owner_allows`, `RepoError`), `crates/bolt-vcs/src/main.rs` (`start`)
- Test: unit tests in both crates; `crates/bolt-vcs/tests/cli.rs`

**Interfaces:**
- Produces (platform):
  - `Owner::Administrators { elevated: bool }`, made on Windows only. The owner is the Administrators group, and `elevated` says whether that group is enabled on the user's token, as `CheckTokenMembership` counts it.
  - `Owner::User` now means the user's own account only.
  - A private, safe `fn group_counts(attributes: u32) -> bool`: `SE_GROUP_ENABLED` is set and `SE_GROUP_USE_FOR_DENY_ONLY` is clear.
- Produces (core):
  - `RepoError::AdminsNotElevated { path: PathBuf }`;
  - `pub fn elevated_only_note(root: &Path) -> Option<String>` and a testable `elevated_only_note_with(root, owner_of)`.

- [ ] **Step 1: Write the failing tests.**

```rust
// bolt-vcs-platform, windows_owner tests
#[test]
fn the_group_counts_only_when_enabled() {
    assert!(group_counts(SE_GROUP_ENABLED | SE_GROUP_ENABLED_BY_DEFAULT | SE_GROUP_MANDATORY));
    assert!(!group_counts(SE_GROUP_USE_FOR_DENY_ONLY));
    assert!(!group_counts(0));
}

// bolt-vcs-core, repo tests
#[test]
fn administrators_count_only_when_elevated() {
    let elevated = Owner::Administrators { elevated: true };
    let normal = Owner::Administrators { elevated: false };
    assert!(owner_allows([Ok(elevated)], root, &[]).is_ok());
    assert!(matches!(owner_allows([Ok(normal)], root, &[]), Err(RepoError::AdminsNotElevated { .. })));
    assert!(owner_allows([Ok(normal)], root, &[root.to_owned()]).is_ok());
}

#[test]
fn the_refusal_names_the_group_and_both_ways_on() {
    let e = RepoError::AdminsNotElevated { path: root.to_owned() };
    assert!(e.to_string().contains("Administrators group"));
    let hint = e.hint().unwrap();
    assert!(hint.contains("administrator") && hint.contains("safe-folders"));
}

#[test]
fn start_notes_a_project_that_opens_only_elevated() {
    let note = elevated_only_note_with(root, &|_| Ok(Owner::Administrators { elevated: true }));
    assert!(note.unwrap().contains("administrator terminal"));
    assert!(elevated_only_note_with(root, &|_| Ok(Owner::User)).is_none());
}
```

- [ ] **Step 2:** Run `cargo test --workspace --all-features`. Expected: the new tests fail to compile (no `Owner::Administrators`).
- [ ] **Step 3: Implement.**
  - In `windows_owner::owner_of`, an owner that is the Administrators SID becomes `Administrators { elevated }`, where `elevated` means the group is in the token with `group_counts(group.Attributes)`. That field comes from the slice the code already builds, so no new `unsafe` is needed.
  - Update `Owner`'s doc comment.
  - In `owner_allows`, `elevated: true` passes and `elevated: false` gives `AdminsNotElevated`.
    - Message: "`<path>` belongs to the Administrators group, and this terminal isn't running as administrator."
    - Hint: open an administrator terminal, or add the folder to `safe-folders` (reuse `config::trust_hint`).
  - When the project folder or its `.bolt` is `Administrators { elevated: true }`, `elevated_only_note` says: "This project belongs to the Administrators group, so it opens only from an administrator terminal unless you add it to safe-folders:" followed by the trust line.
  - `bolt start` prints the note on stderr. Task 2 turns it into a warning.
- [ ] **Step 4:** Run fmt, clippy (`-D warnings`) and the tests. Expected: all pass.
- [ ] **Step 5:** Commit: `Count the Administrators group as the user only in an elevated terminal`.

### Task 2: Warnings, and the `--json` contract

These are milestone-2 follow-ups: document the `--json` formats, and carry warnings in them.

**Files:**
- Create: `crates/bolt-vcs-core/src/warning.rs`, `docs/json.md`
- Modify:
  - `crates/bolt-vcs-core/src/{lib.rs,snapshot.rs,store.rs}`;
  - `crates/bolt-vcs/src/{main.rs,output.rs}`;
  - the insta snapshots;
  - `docs/design.md` §5.15 (one line linking `json.md`).

**Interfaces:**
- Produces: `pub struct Warning { pub kind: &'static str, pub message: String }`. `kind` is a stable snake_case code; `message` is escaped text for people.
- Kinds now:
  - `damaged_pack`: a pack whose copy failed a check was passed over;
  - `copy_unreadable`: a stored copy couldn't be read for a reason that isn't damage, such as an I/O error, even though another copy worked;
  - `change_id_not_shortened`;
  - `changed_files_not_counted`, with why;
  - `elevated_only` (Task 1's note).
- `SaveOutcome::Saved` gains `warnings: Vec<Warning>`, and so does every later outcome type.
- Every command's `--json` document has `"warnings": [{"kind": …, "message": …}]`, always present. People see each warning as `warning: <message>` on stderr.
- `history --json` and `show --json`: when an error happens after output began, Bolt closes the document with `"error": {"message": …, "hint": …}` and exits 1. The output is always valid JSON.

- [ ] **Step 1: Write the failing tests** (`crates/bolt-vcs/tests/cli.rs`):

```rust
#[test]
fn save_json_carries_a_damaged_pack_warning() {
    // plant a damaged copy of a file's content (helpers in the redteam2 tests), then save that file
    let doc = json_of(bolt(&dir).args(["save", "--json", "m"]));
    let kinds: Vec<_> = doc["warnings"].as_array().unwrap().iter().map(|w| w["kind"].clone()).collect();
    assert!(kinds.contains(&json!("damaged_pack")));
}

#[test]
fn history_json_stays_valid_when_a_save_is_unreadable() {
    // three saves; damage the middle save's only copy
    let out = bolt(&dir).args(["history", "--json"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    let doc: Value = serde_json::from_slice(&out.stdout).unwrap();   // still parses
    assert!(doc.get("error").is_some());
}

#[test]
fn every_json_document_has_warnings() {
    for args in [vec!["start", "--json"], vec!["save", "--json", "m"], vec!["history", "--json"], vec!["show", "--json"]] {
        let doc = json_of(bolt(&dir).args(&args));
        assert!(doc["warnings"].is_array(), "{args:?}");
    }
}
```

- [ ] **Step 2:** Run. Expected: these fail.
- [ ] **Step 3: Implement.**
  - Collect warnings in `store.rs`: the damage already noted (`damaged_packs`), plus a new list of copies that couldn't be read for other reasons.
  - Move today's stderr notes into `Warning`s.
  - Write `docs/json.md`, documenting every command's document:
    - fields, types and what `null` means (`changed_files: null` means it couldn't be counted);
    - `ignored_project_identity`;
    - `warnings` and `error`;
    - `bolt_json` versioning: fields may be added, never removed or changed within one version.
- [ ] **Step 4:** Update the insta snapshots (`cargo insta review`, accepting only the new `warnings` arrays), then run fmt, clippy and the tests.
- [ ] **Step 5:** Commit: `Carry warnings in --json, keep JSON streams valid on errors, and document the --json formats`.

### Task 3: Tags and server lines in records of their own

Decisions 033 and 034.

**Files:**
- Modify:
  - `crates/bolt-vcs-core/src/id.rs`: `Flavor::TagList`, `Flavor::ServerLines`, with the context strings;
  - `pack.rs`: `ObjectKind::TagList = 22`, `ObjectKind::ServerLines = 23`;
  - `diary.rs`, `repo.rs`, `verify.rs`;
  - the commands that build a state (`start` in `repo.rs`, `save` in `snapshot.rs`).
- Test: unit tests; `tests/vectors.rs` with new `vectors/tag_list.json` and `vectors/server_lines.json`, and an updated `vectors/diary.json`.
- Create: `fuzz/fuzz_targets/decode_tag_list.rs`, `fuzz/fuzz_targets/decode_server_lines.rs`. Modify: `fuzz/Cargo.toml`, `.github/workflows/nightly.yml` (the target list).

**Interfaces:**
```rust
pub struct Tag { pub save: Id, pub message: String, pub git_tags: Vec<Vec<u8>> }
pub struct TagList { pub tags: BTreeMap<String, Tag> }                         // encode, decode, id
pub struct ServerLines { pub servers: BTreeMap<String, BTreeMap<String, Id>> } // encode, decode, id
pub struct State {
    pub lines: BTreeMap<String, Id>,
    pub tags: Option<Id>,           // the tag list, if there are tags
    pub position: Position,
    pub shelves: Vec<(String, Id)>,
    pub server_lines: Option<Id>,   // the server-lines list, if there are servers
}
pub const MAX_GIT_TAG_OBJECTS: u64 = 16;
pub const MAX_GIT_TAG_OBJECT_BYTES: usize = 1 << 20;
impl Repository {
    pub fn tag_list(&self, state: &State) -> Result<TagList, RepoError>;       // empty if none
    pub fn server_lines(&self, state: &State) -> Result<ServerLines, RepoError>;
}
```
The decoding rules are exactly [storage-format.md](../storage-format.md) §9:
- **Tag list:** 1 to 100,000 tags, in strictly sorted order. Names are 1 to 255 bytes, messages at most 64 KiB, and each tag has 0 to 16 Git tag objects of at most 1 MiB each.
- **Server lines:** 1 to 64 servers. Server names pass `validate_line_name`, and no two differ only by letter case. Each server's lines follow `read_lines`, with 1 to 100,000 lines each.
- **Neither record may be empty.** The diary entry's `bool` says whether there is one.

- [ ] **Step 1: Write the failing tests.**

```rust
#[test] fn tag_lists_round_trip()              // proptest over valid lists
#[test] fn server_lines_round_trip()
#[test] fn tag_list_rejections() {
    for bad in [empty_list(), unsorted(), duplicate(), name_of(0), name_of(256), seventeen_git_tags(), git_tag_of(MAX_GIT_TAG_OBJECT_BYTES + 1)] {
        assert!(TagList::decode(&bad).is_err());
    }
}
#[test] fn server_lines_rejections() {
    for bad in [empty(), server_named("a b"), servers_differing_by_case("Origin", "origin"), server_without_lines()] {
        assert!(ServerLines::decode(&bad).is_err());
    }
}
#[test] fn start_writes_neither_record() { let s = first_entry_state(); assert!(s.tags.is_none() && s.server_lines.is_none()); }
#[test] fn a_save_keeps_the_records_of_the_entry_before_it() {
    // commit a state with tags = Some(t), server_lines = Some(r) by hand; save a change
    assert_eq!((after.tags, after.server_lines), (Some(t), Some(r)));
}
```

Add vectors: one lightweight tag plus one annotated tag with one Git tag object; one server with two lines; a diary entry with both pointers.

- [ ] **Step 2:** Run. Expected: compile errors, then failures.
- [ ] **Step 3:** Implement the records, the state fields and the readers. Teach `verify_store` the two new kinds. Add both fuzz targets to `fuzz/Cargo.toml` and to the nightly loop.
- [ ] **Step 4:** Run fmt, clippy, the tests, and `cargo +nightly fuzz build` in `fuzz/` (Linux).
- [ ] **Step 5:** Commit: `Keep tags and server lines in records of their own that diary entries share`.

### Task 4: A command may write several packs

Decision 041.

**Files:**
- Modify: `crates/bolt-vcs-core/src/{pack.rs,repo.rs,snapshot.rs}`; `fsio.rs` gets a fault point label `packs:rolled-over`.
- Test: unit tests in `pack.rs` and `repo.rs`.

**Interfaces:**
```rust
pub const WRITER_PACK_OBJECTS: u64 = 1 << 20;
pub struct PackSet { /* the pack being built, the finished packs' paths, the limit */ }
impl PackSet {
    pub fn add_record(&mut self, kind: ObjectKind, encoded: &[u8]) -> Result<Id, PackError>;
    pub fn add_content(&mut self, bytes: &[u8]) -> Result<Id, PackError>;
    pub fn add_content_from(&mut self, reader: &mut dyn Read, expected: Id) -> Result<u64, PackError>;
    pub fn contains(&self, id: &Id) -> bool;     // in the pack being built
    pub fn is_empty(&self) -> bool;
}
#[cfg(any(test, feature = "fault-injection"))]
impl PackSet { pub fn with_limit(tmp_dir: &Path, packs_dir: &Path, limit: u64) -> Result<PackSet, PackError>; }
#[cfg(any(test, feature = "fault-injection"))]
pub fn set_test_pack_limit(limit: Option<u64>);   // process-wide, for tests of whole commands (Task 13)
impl Repository {
    pub fn new_packs(&self) -> Result<PackSet, RepoError>;   // replaces new_pack for commands
    pub fn commit(&mut self, lock: &ChangeLock, packs: PackSet, entry: &DiaryEntry) -> Result<Id, RepoError>;
}
```
- When the pack being built reaches the limit, it is finished: moved into `store/packs/`, with its path kept. The next add starts a new pack.
- `commit` adds the diary entry to the last pack, finishes it, registers every finished pack with the store, then replaces `state/head`.
- An object stored twice in two packs of one command is harmless (decision 028).

- [ ] **Step 1: Write the failing tests.**

```rust
#[test]
fn a_command_past_the_limit_writes_several_packs_and_takes_effect_once() {
    let mut packs = PackSet::with_limit(tmp, packs_dir, 3).unwrap();
    let ids: Vec<Id> = (0..7u8).map(|i| packs.add_content(&[i]).unwrap()).collect();
    assert_eq!(repo.head().unwrap(), head_before);          // nothing visible yet
    repo.commit(&lock, packs, &entry).unwrap();
    assert_eq!(count_packs(packs_dir), packs_before + 3);   // 3 + 3 + 1 objects, then the entry
    for id in ids { assert!(repo.store().read_record_or_content(&id).is_ok()); }
}
#[test]
fn the_diary_entry_is_in_the_last_pack() { /* the newest pack holds the entry's id */ }
#[test]
fn a_crash_between_packs_leaves_the_repository_as_it_was() {
    fault::arm_at("packs:rolled-over", 1);
    assert!(run_command_past_the_limit().is_err());
    let repo = Repository::open(root).unwrap();
    assert_eq!(repo.head().unwrap(), head_before);
    assert!(verify_store(packs_dir).is_ok());
    fault::disarm();
    run_command_past_the_limit().unwrap();                 // a re-run works and reuses objects
}
#[test]
fn ordinary_commands_still_write_one_pack() { /* start and a small save add one pack each */ }
```

- [ ] **Step 2:** Run. Expected: compile errors.
- [ ] **Step 3:** Implement `PackSet`, and switch `start` and `save` to it.
- [ ] **Step 4:** Run fmt, clippy and the tests, including the existing crash tests.
- [ ] **Step 5:** Commit: `Let one command write several packs, a new one every 2^20 objects`.

### Task 5: Names Git treats as `.git`, names this system can hold, and the drive's letter case

Decisions 035 and 039, plus the follow-up "decide letter case by the drive, as Git does".

**Files:**
- Modify: `crates/bolt-vcs-core/src/{names.rs,fsio.rs,lib.rs}`. Create: `crates/bolt-vcs-core/src/drive.rs`.
- Modify: `fuzz/fuzz_targets/name_rules.rs` (it also calls `is_git_alias`).

**Interfaces:**
```rust
pub fn is_git_alias(name: &str) -> bool;    // names.rs: Git's is_ntfs_dotgit || is_hfs_dotgit, on every system
pub fn holdable_here(name: &str) -> bool;   // names.rs: decision 004's portability rules that apply on this system
pub struct Drive { pub ignores_case: bool, pub composes_unicode: bool }
pub fn probe(tmp_dir: &Path) -> io::Result<Drive>;   // drive.rs
```
- **`is_git_alias`** follows Git's `path.c`: any letter case; trailing spaces and dots; `git~1`; an NTFS stream suffix (`:` and anything after it); and the code points HFS+ ignores anywhere (U+200C–U+200F, U+202A–U+202E, U+206A–U+206F, U+FEFF).
- **`holdable_here`:**
  - On Windows it refuses device names (M1's list, including `COM¹`–`COM³` and `LPT¹`–`LPT³`), trailing dots or spaces, and `<>:"\|?*` and control characters.
  - On macOS and Linux every valid name is holdable.
- **`probe`:**
  - It creates a file through `fsio::create_temp` in `tmp_dir` and checks whether its letter-case variant exists (`ignores_case`).
  - It then creates `tmp-<16 hex>-e\u{301}` and checks whether the composed `é` form exists (`composes_unicode`).
  - It removes what it made. `fsio::is_temp_name` also accepts that second form.

- [ ] **Step 1: Write the failing tests.**

```rust
#[test]
fn git_aliases_match_gits_own_lists() {
    // copied from Git's t/t0060-path-utils.sh (is_ntfs_dotgit, is_hfs_dotgit), plus these
    for name in [".git", ".GIT", "git~1", "GIT~1", ".git.", ".git ", ".git. .", ".git::$INDEX_ALLOCATION", ".g\u{200c}it", "\u{feff}.git"] {
        assert!(is_git_alias(name), "{name:?}");
    }
    for name in [".gitignore", ".gitmodules", "git", ".git2", "xgit~1", ".bolt"] {
        assert!(!is_git_alias(name), "{name:?}");
    }
}
#[test]
fn holdable_here_follows_this_systems_rules() {
    #[cfg(windows)] for name in ["aux.txt", "con", "name.", "a:b", "a\\b"] { assert!(!holdable_here(name)); }
    #[cfg(not(windows))] for name in ["aux.txt", "con", "name.", "a:b"] { assert!(holdable_here(name)); }
}
#[test]
fn the_drive_probe_matches_the_ci_drives() {
    let d = probe(tmp).unwrap();
    assert_eq!(d.ignores_case, cfg!(any(windows, target_os = "macos")));
    assert_eq!(d.composes_unicode, cfg!(target_os = "macos"));
}
#[test]
fn the_probe_leaves_nothing_behind() { probe(tmp).unwrap(); assert_eq!(fs::read_dir(tmp).unwrap().count(), 0); }
```

- [ ] **Step 2:** Run. Expected: compile errors.
- [ ] **Step 3:** Implement. Copy Git's exact cases from its source. Don't invent cases.
- [ ] **Step 4:** Run fmt, clippy and the tests.
- [ ] **Step 5:** Commit: `Recognise every name Git treats as .git, and test the drive's letter case and Unicode form`.

### Task 6: An ignore matcher that follows Git's own rules

This is the milestone-2 follow-up. It lifts storage-format.md §12's refusals.

**Files:**
- Create: `crates/bolt-vcs-core/src/ignore.rs`, `crates/bolt-vcs/tests/ignore_vs_git.rs`, `crates/bolt-vcs/tests/support/mod.rs` (the `git()` helper with a clean environment; Task 15 extends it), `fuzz/fuzz_targets/ignore_rules.rs`.
- Modify: `crates/bolt-vcs-core/src/snapshot.rs`.
  - It replaces `GitignoreBuilder` and deletes the refusal code: `like_git`, `simple_class`, `literal_braces` and their tests.
  - `ignore::WalkBuilder` stays, for walking only.
- Modify: `docs/storage-format.md` §12 (the refusal bullets and the "limit on size" bullet go), `docs/security.md` T12.

**Interfaces:**
```rust
pub fn wildmatch(pattern: &[u8], text: &[u8], casefold: bool) -> bool;   // Git's wildmatch.c with WM_PATHNAME
pub struct Rules { /* one ignore file's lines, for the folder it sits in */ }
impl Rules { pub fn parse(text: &[u8], folder: &str) -> Rules; }         // Git's line rules; refuses no line
pub enum Decision { Ignored, Kept, NoRule }
pub struct Matcher { /* the rules of the folders the walk is in */ }
impl Matcher {
    pub fn new(drive: Drive) -> Matcher;
    pub fn enter(&mut self, folder: &str, rules: Vec<Rules>);    // in reading order
    pub fn leave_to(&mut self, folder: &str);
    pub fn decide(&self, path: &str, is_dir: bool) -> Decision;
}
```
- **Semantics** are Git's `dir.c` and `wildmatch.c`, ported exactly, letter-case quirks included:
  - A byte order mark at the start is skipped. Lines end at LF, and one CR before it is dropped.
  - A line starting with `#` is a comment. Trailing spaces are trimmed unless escaped.
  - `!` negates. A leading `/` anchors the pattern; a trailing `/` matches folders only.
  - A pattern without `/` matches a name at any depth. `**` and `\` work as in Git, as do classes with `!`, `^` and `[:class:]`.
  - `?` and `*` never match `/`.
  - The last matching line wins.
- **Letter case:** `casefold` comes from `Drive::ignores_case`, as Git's `core.ignorecase` does.
- **macOS:** where `Drive::composes_unicode` holds, names are compared in NFC, as Git does with `core.precomposeunicode`. Patterns are used as written.
- **wildmatch is iterative.** Its time is at most proportional to the pattern's length times the text's length.
- **Limits** stay: 1 MiB per file, 1 MiB in force at a folder, 16 MiB per save.

- [ ] **Step 1: Write the failing tests.**

```rust
// ignore.rs
#[test]
fn gits_wildmatch_table_passes() {
    // every row of Git's t/t3070-wildmatch.sh: (expected wildmatch, expected iwildmatch, text, pattern)
    for (wm, iwm, text, pattern) in WILDMATCH_CASES {
        assert_eq!(wildmatch(pattern, text, false), *wm, "{text:?} {pattern:?}");
        assert_eq!(wildmatch(pattern, text, true), *iwm, "{text:?} {pattern:?} (casefold)");
    }
}
#[test]
fn matching_takes_time_proportional_to_its_input() {
    let pattern = "*a".repeat(2000) + "b";
    let text = "a".repeat(4000);
    let start = Instant::now();
    assert!(!wildmatch(pattern.as_bytes(), text.as_bytes(), false));
    assert!(start.elapsed() < Duration::from_secs(2));
}

// crates/bolt-vcs/tests/ignore_vs_git.rs
proptest! {
    #![proptest_config(ProptestConfig { cases: 256, ..ProptestConfig::default() })]
    #[test]
    fn bolt_keeps_exactly_the_files_git_keeps(tree in ignore_tree()) {
        // writes each rule file twice, as .gitignore for Git and .boltignore for Bolt, plus the paths,
        // into a fresh git repository; Git's real walk is the reference:
        // `git -c core.excludesFile= -c core.ignorecase=<probe> ls-files --others --exclude-standard -z`;
        // the files a Bolt save keeps must be exactly those
    }
    #[test]
    fn single_path_verdicts_match_check_ignore(tree in ignore_tree()) {
        // `git ... check-ignore --no-index -v -n --stdin` against Matcher::decide, path by path
    }
}
#[test]
fn gits_own_ignore_cases_match() { /* the cases of Git's t/t0008-ignores.sh */ }
```

`ignore_tree()` draws patterns from these pieces: letters of both cases, `é` in NFC and NFD, `*`, `**`, `?`, `[`, `]`, `!`, `^`, `-`, `\`, `/`, spaces, `#`, `{`, `}`. It also draws nested folders, and paths that match some patterns but not others.
- [ ] **Step 2:** Run. Expected: failures.
- [ ] **Step 3:** Implement. Then replace the old refusal tests with tests showing those lines now behave as in Git: `[!…]`, `[[:alpha:]]`, `a[/]b`, a trailing `\`, decomposed lines, and capitals after `\`.
- [ ] **Step 4:** Run fmt, clippy and the tests on all three systems (CI), plus `cargo +nightly fuzz build`.
- [ ] **Step 5:** Commit: `Match ignore rules exactly as Git does, and stop refusing lines Bolt couldn't read before`.

### Task 7: `save` reads `.gitignore` too, and saved files stay saved

Decision 038.

**Files:**
- Modify: `crates/bolt-vcs-core/src/snapshot.rs`.
- Test: unit tests in `snapshot.rs`.

**Behaviour:**
- **The rules at each folder:**
  - First `.gitignore`: a regular file of at most 1 MiB, read through `safe_fs`. A link or special file is skipped with warning `gitignore_skipped`, and its rules don't apply.
  - Then `.boltignore`, as today: a link stops the save.
  - Both count toward the limits.
- **The walk knows the base save's entries.**
  - An ignored entry is kept if the base save has an entry of that name in that folder.
  - An ignored folder is entered if the base save has a folder of that name. Inside it, only names the base save has are kept, by the same rule all the way down, so new files there stay ignored.
- `.git` and `.bolt` folders stay skipped always (decision 019).

- [ ] **Step 1: Write the failing tests.**

```rust
#[test] fn gitignore_rules_apply_like_boltignore_rules()
#[test] fn boltignore_lines_come_after_gitignore_lines() {
    write(root, ".gitignore", b"*.log\n"); write(root, ".boltignore", b"!keep.log\n");
    // keep.log saved, other.log not
}
#[cfg(unix)] #[test] fn a_linked_gitignore_is_skipped_with_a_warning() {
    // .gitignore -> /etc/passwd: warnings has kind "gitignore_skipped"; its rules don't apply
}
#[test] fn both_files_count_toward_the_limits() { /* 600 KiB + 600 KiB in one folder: the save stops, naming the file that went over */ }
#[test] fn a_saved_file_stays_saved_when_a_rule_matches_it() {
    write(root, "a.dll", b"1"); save(); write(root, ".gitignore", b"*.dll\n"); write(root, "b.dll", b"2"); save();
    assert!(saved_paths().contains(&"a.dll".into()) && !saved_paths().contains(&"b.dll".into()));
}
#[test] fn a_saved_file_in_an_ignored_folder_stays_and_new_ones_stay_out() {
    // Library/keep.txt saved; then .gitignore "Library/"; Library/new.txt added: keep.txt saved again, new.txt not
}
#[test] fn deleting_a_saved_ignored_file_removes_it() { /* then it stays out */ }
```

- [ ] **Step 2:** Run. Expected: failures.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run fmt, clippy and the tests.
- [ ] **Step 5:** Commit: `Read .gitignore files too, and keep saved files that an ignore rule matches`.

### Task 8: `save` keeps what this computer can't hold, and leaves project links alone

Decisions 036 and 039 (part 1).

**Files:**
- Modify: `crates/bolt-vcs-core/src/snapshot.rs`. Use `test_support` helpers to plant base saves.

**Behaviour:** for each entry of the base save in a folder, the save keeps it exactly as it was, and ignores whatever the folder holds at that name, when any of these is true:
- (a) `!holdable_here(name)`;
- (b) another entry of the same base folder is the same name on this drive: equal after `to_lowercase` when `drive.ignores_case`, or after NFC when `drive.composes_unicode`;
- (c) it is a symlink, and the folder holds a regular file at that name whose bytes are exactly the link's stored target;
- (d) it is a project link. The walk never enters that folder.

The save's warnings get one `kept_as_they_were` warning, with the count, when the count is above zero.

- [ ] **Step 1: Write the failing tests.**

```rust
#[cfg(windows)] #[test] fn a_name_this_system_cant_hold_is_kept() {
    let base = plant_save(&repo, &[("aux.txt", File, b"x")]);
    let saved = save_now(&mut repo).unwrap();
    assert_eq!(entry(&saved, "aux.txt"), entry(&base, "aux.txt"));
}
#[test] fn names_that_are_one_file_on_this_drive_are_kept() {
    // base: "README" and "readme"; disk: whatever the drive holds. Where the drive ignores
    // case both stay as they were; on Linux they're saved from disk as usual
}
#[test] fn a_link_git_wrote_as_a_file_stays_a_link() {
    // base: symlink "l" -> "target"; disk: regular file "l" holding b"target"; saved kind is Symlink
}
#[test] fn a_file_that_isnt_the_links_target_replaces_the_link() { /* disk "l" holds b"other": saved as File */ }
#[test] fn a_project_link_is_kept_and_its_folder_never_read() {
    // base: ProjectLink "sub"; disk: sub/.git (a file "gitdir: /etc") and sub/a.txt
    // saved: "sub" is the same ProjectLink; no path under "sub/"
}
#[test] fn kept_entries_are_counted_in_one_warning() {}
```

- [ ] **Step 2:** Run. Expected: failures.
- [ ] **Step 3:** Implement. Probe the drive once per command (`drive::probe(repo.tmp_dir())`).
- [ ] **Step 4:** Run fmt, clippy and the tests.
- [ ] **Step 5:** Commit: `Keep entries this computer can't hold, links Git wrote as files, and project links as they were`.

### Task 9: Git reader, part 1: the repository, its config and its refs

Decisions 031, 033 and 039.

**Files:**
- Create: `crates/bolt-vcs-core/src/git/{mod.rs,config.rs,refs.rs,craft.rs}`.
- Modify: `crates/bolt-vcs-core/src/lib.rs` (`pub mod git;`), `crates/bolt-vcs-core/Cargo.toml` (feature `test-support = []`; `git::craft` is `#[cfg(any(test, feature = "test-support"))]`; the dependencies `miniz_oxide`, `sha1-checked` and `sha2`, which `craft` needs to write and hash objects), `crates/bolt-vcs/Cargo.toml` (dev-dependency enables `test-support`).

**Interfaces:**
```rust
pub enum ObjectType { Commit, Tree, Blob, Tag }
pub fn object_id(kind: ObjectType, data: &[u8], hash: HashKind) -> GitId;   // "<type> <len>\0" + data; SHA-1 with collision detection, or SHA-256
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub enum HashKind { Sha1, Sha256 }                         // len(): 20 or 32
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct GitId { kind: HashKind, bytes: [u8; 32] }       // as_bytes(), to_hex(), from_hex(kind, &str)
pub enum Refusal { FormatVersion(u64), UnknownExtension(String), Reftable, PartialClone, Shallow,
                   Alternates, GitFile, SparseCheckout, OldPackIndex }
pub enum GitError {
    NotAGitFolder,                                  // no .git folder here
    Refused(Refusal),                               // each with a message and a hint saying what to run
    NotYours { path: PathBuf },                     // .git belongs to another account
    Broken { what: String },                        // damaged or hostile data; names the file or object
    Changed { what: String },                       // Git changed the repository while Bolt read it
    Io { path: PathBuf, source: io::Error },
}
pub struct Repo { /* .git path, hash kind, objects (Task 10) */ }
impl Repo {
    pub fn open(work_tree: &Path) -> Result<Repo, GitError>;
    pub fn hash(&self) -> HashKind;
    pub fn refs(&self) -> Result<Refs, GitError>;
}
pub enum Head { Branch(Vec<u8>), Detached(GitId) }          // the branch name after refs/heads/
pub struct Refs {
    pub head: Head,
    pub branches: Vec<(Vec<u8>, GitId)>,                    // names after refs/heads/
    pub servers: Vec<(Vec<u8>, Vec<u8>, GitId)>,            // (server, name) after refs/remotes/
    pub tags: Vec<(Vec<u8>, GitId)>,                        // names after refs/tags/
    pub other: Vec<Vec<u8>>,                                // whole ref names, for the summary
}
```

**`Repo::open` checks, in order:**
1. `work_tree/.git` exists. Otherwise `NotAGitFolder`: "No Git repository here: run bolt import at the top of a Git project, where its .git folder is."
2. If it is a file, `Refused(GitFile)`. If it is a link or junction, `Broken`. It must pass `safe_fs::check_real_dir`.
3. Its owner is the user's: `Owner::User` or `Administrators { elevated: true }`, unless the work tree is in `safe-folders`. This reuses `repo.rs`'s `owner_allows`, made `pub(crate)`.
4. `.git/config`:
   - It is a regular file of at most 1 MiB.
   - It is parsed by `git::config`, a strict subset of Git's syntax: sections, subsections, `[a.b]`, quoted values with `\"`, `\\`, `\n`, `\t` and `\b`, continuation lines, `#` and `;` comments, and booleans. A line it can't read refuses with the line number.
   - It keeps only `core.repositoryformatversion`, `core.sparsecheckout` and `extensions.*`. Includes aren't followed, as Git's own format check doesn't follow them.
   - Format version 0 or 1. With 1, every extension must be known:
     - `objectformat` is `sha1` or `sha256`; `refstorage` is `files` (`reftable` gives `Reftable`);
     - `partialclone` gives `PartialClone`;
     - `compatobjectformat`, `preciousobjects`, `worktreeconfig`, `relativeworktrees`, `submodulepathconfig`, `noop` and `noop-v1` are accepted;
     - anything else gives `UnknownExtension`.
   - With `worktreeconfig`, `.git/config.worktree` is read too, for `core.sparsecheckout` only. `true` gives `SparseCheckout`.
5. Files and folders that refuse:
   - `.git/reftable/` gives `Reftable`;
   - a non-empty `.git/shallow` gives `Shallow`;
   - `.git/objects/info/alternates` gives `Alternates`;
   - `.git/commondir` gives `GitFile`;
   - any `objects/pack/*.promisor` gives `PartialClone`.

**Refs:**
- `HEAD` is at most 4 KiB, holding either `ref: refs/heads/<name>\n` or a full hex ID plus `\n`. Anything else is `Broken`: "Git's HEAD points at `<x>`, which isn't a branch".
- Loose refs: `.git/refs/` is walked iteratively. Every folder must pass `check_real_dir`, and every file is opened with `open_regular` and read up to 4 KiB.
  - Names follow Git's `check-ref-format` rules. A bad name is `Broken`.
  - On Windows, a ref whose last part Windows treats as a device is refused before it is opened, using `names` (gitoxide's CVE-2024-35197).
  - Symbolic refs under `refs/` are skipped. `refs/remotes/<s>/HEAD` is skipped silently; others are listed in `other`.
- `packed-refs`:
  - An optional `# pack-refs with:` header, then `<hex> <name>` lines.
  - `^<hex>` lines are ignored, since Bolt peels tags itself.
  - Any other line, or a duplicate name, is `Broken`.
  - A loose ref overrides a packed one, as in Git.

**`craft.rs`** (test-only) writes repositories byte by byte, malformed ones included, with these functions:
```rust
pub fn new(dir: &Path, hash: HashKind) -> Craft;  // writes .git/objects, .git/refs, a minimal config, HEAD → main
pub fn config(&self, text: &str);
pub fn head(&self, text: &str);
pub fn file(&self, rel: &str, bytes: &[u8]);      // any file under .git
pub fn loose_ref(&self, name: &str, text: &str);
pub fn packed_refs(&self, text: &str);
pub fn object(&self, kind: ObjectType, bytes: &[u8]) -> GitId;   // loose, compressed with miniz_oxide
pub fn raw_loose(&self, id_hex: &str, compressed: &[u8]);
pub fn tree(&self, entries: &[(&[u8], &[u8], GitId)]) -> GitId;  // (mode, name, id), written as given
pub fn commit(&self, raw: &[u8]) -> GitId;
```

- [ ] **Step 1: Write the failing tests.** Build each case with `Craft`.

```rust
#[test] fn a_plain_repository_opens() { assert_eq!(Repo::open(dir).unwrap().hash(), HashKind::Sha1); }
#[test] fn sha256_repositories_open() { /* version 1, objectformat = sha256 */ }
#[test] fn each_refusal_says_what_to_do() {
    for (setup, refusal) in [(reftable_folder, Refusal::Reftable), (shallow_file, Refusal::Shallow),
        (alternates_file, Refusal::Alternates), (partial_clone_ext, Refusal::PartialClone),
        (sparse_checkout_true, Refusal::SparseCheckout), (unknown_ext, Refusal::UnknownExtension("x".into())),
        (format_version_2, Refusal::FormatVersion(2)), (git_is_a_file, Refusal::GitFile)] {
        let err = Repo::open(setup()).err().unwrap();
        assert!(matches!(&err, GitError::Refused(r) if *r == refusal));
        assert!(err.hint().is_some());
    }
}
#[test] fn a_dot_git_link_is_refused() {}
#[test] fn config_lines_bolt_cant_read_refuse_with_the_line_number() {}
#[test] fn config_reading_follows_git_on_a_table_of_lines() { /* quotes, escapes, continuation, case, booleans */ }
#[test] fn ref_names_follow_check_ref_format() { /* table from Git's t/t1402-check-ref-format.sh */ }
#[test] fn packed_refs_are_read_and_loose_refs_win() {}
#[test] fn malformed_packed_refs_are_broken() {}
#[test] fn head_forms() { /* branch, detached, pointing outside refs/heads */ }
#[cfg(windows)] #[test] fn a_ref_named_like_a_device_is_refused_before_opening() {}
#[test] fn a_dot_git_owned_by_another_account_is_refused() { /* through a stubbed owner check */ }
```

- [ ] **Step 2:** Run. Expected: compile errors.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run fmt, clippy and the tests.
- [ ] **Step 5:** Commit: `Read a Git repository's format, config and refs, refusing what Bolt can't import`.

### Task 10: Git reader, part 2: loose objects, packs and deltas

Decision 031.

**Files:**
- Create: `crates/bolt-vcs-core/src/git/{odb.rs,pack.rs}`.
- Modify: `git/craft.rs` (packs). The dependencies arrived with Task 9.
- Run `cargo deny check` locally.

**Interfaces:**
```rust
pub struct Object { pub kind: ObjectType, pub data: Vec<u8> }   // ObjectType comes from Task 9
pub const MAX_IN_MEMORY_OBJECT: u64 = 512 << 20;
pub const MAX_DELTA_DEPTH: u32 = 4096;
pub const DELTA_BASE_CACHE: u64 = 256 << 20;
impl Repo {
    pub fn header(&self, id: &GitId) -> Result<(ObjectType, u64), GitError>;     // without reading the data
    pub fn read(&self, id: &GitId) -> Result<Object, GitError>;                  // up to MAX_IN_MEMORY_OBJECT, ID checked
    pub fn blob_reader(&self, id: &GitId) -> Result<BlobReader<'_>, GitError>;   // impl Read: objects stored whole stream; deltified ones read from memory
}
pub struct BlobReader<'r> { /* … */ }   // checks the ID as the data ends; a mismatch fails that last read
// craft.rs
pub enum PackEntry { Whole(ObjectType, Vec<u8>), OfsDelta { base: usize, delta: Vec<u8> },
                     RefDelta { base: GitId, delta: Vec<u8> }, Raw(Vec<u8>) }
pub fn pack(&self, entries: &[PackEntry]) -> (PathBuf, PathBuf);   // a .pack and its .idx; Raw writes bytes as given
pub fn delta(base: &[u8], result: &[u8]) -> Vec<u8>;               // a valid delta, for tests
```

**Loose objects:**
- Opened with `open_regular` and inflated by streaming.
- The header `<type> <size>\0` is parsed strictly before anything is allocated: a decimal size, no leading zeros, at most 2^40.
- The data must be exactly `size` bytes, and nothing may follow the zlib stream.

**Pack indexes and packs:**
- Every `objects/pack/pack-*.idx` with its `.pack`.
- `.idx` must be version 2. A version 1 index gives `Refused(OldPackIndex)`, with the hint `git repack -a -d`.
- The fan-out must not decrease. Names must be strictly sorted, and their count must be the fan-out's last value. Offsets must stay inside the pack, and the 64-bit offset table must be in bounds.
- The index checksum is checked while the index is read in bounded steps. The name and offset tables stay in memory, about 28–40 bytes an object, after their size is checked against the file.
- The `.pack` header is `PACK`, version 2 or 3, and its object count must equal the index's.

**Entries:**
- The type and size varint is at most 10 bytes and can't overflow.
- An `OFS_DELTA` distance (Git's encoding) must be at least 1 and point inside the pack, before the entry. A `REF_DELTA` names its base by ID.
- The zlib data must inflate to exactly the declared size.

**Deltas:**
- The base size must equal the base object's size. The declared result size must be at most 512 MiB, checked before allocating.
- Copy instructions must stay inside the base (size 0 means 0x10000). Insert instructions are 1–127 bytes. Opcode 0 is refused.
- The output must be exactly the result size.
- Chains are resolved iteratively, at most `MAX_DELTA_DEPTH` deep, and a cycle is refused.
- Bases are cached in an LRU of at most `DELTA_BASE_CACHE` bytes.

**IDs:**
- Every object read is checked against its ID: SHA-1 through `sha1-checked`, where a detected collision attack is `Broken`, or SHA-256.
- `blob_reader` checks at the end of the stream, so a reader that fails stores nothing in `PackSet::add_content_from`.

**Errors:**
- An object found nowhere is `Broken`: "missing object `<hex>`".
- A pack or object file that disappears while Bolt reads it is `Changed`.

- [ ] **Step 1: Write the failing tests.**

```rust
#[test] fn loose_objects_read_back() {}
#[test] fn packed_objects_read_back_whole_and_through_both_kinds_of_delta() {}
#[test] fn sha256_objects_read_back() {}
#[test] fn large_offsets_are_followed() { /* a crafted idx using the 64-bit table */ }
#[test] fn blobs_stored_whole_are_streamed() { /* read blob_reader to the end in small reads: bytes and size match */ }
#[test] fn a_streamed_blob_with_the_wrong_id_fails_its_last_read() {}
#[test] fn a_huge_declared_size_is_refused_before_allocating() {
    craft.raw_loose(&hex, &deflate(b"blob 99999999999\0x"));
    assert!(matches!(repo.read(&id), Err(GitError::Broken { .. })));
}
#[test] fn a_delta_claiming_a_huge_result_is_refused_before_allocating() { /* result size 2^62 */ }
#[test] fn bad_deltas_are_refused() {
    for delta in [copy_past_base_end(), opcode_zero(), result_size_mismatch(), base_size_mismatch()] { /* Broken */ }
}
#[test] fn delta_chains_are_bounded_and_cycles_refused() { /* chain 4,097 deep; REF_DELTA A↔B; OFS distance 0 */ }
#[test] fn a_zlib_bomb_and_trailing_garbage_are_refused() {}
#[test] fn an_object_whose_id_doesnt_match_is_refused() {}
#[test] fn a_sha1_collision_attack_is_refused() { /* a collision block from sha1-checked's test vectors */ }
#[test] fn every_truncation_of_a_pack_or_index_is_an_error() {
    // proptest: cut a small valid pack (or idx) at any length; reading every object returns Err, never panics
}
#[test] fn a_pack_claiming_four_billion_objects_is_refused() {}
#[test] fn a_pack_removed_while_reading_is_reported_as_changed() {}
```

- [ ] **Step 2:** Run. Expected: compile errors.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run fmt, clippy, the tests and `cargo deny check`.
- [ ] **Step 5:** Commit: `Read Git's loose objects and packs, with every size checked and every object verified`.

### Task 11: Git objects: commits, trees, tags, and rebuilding a commit

Decisions 032, 035 and 037.

**Files:**
- Create: `crates/bolt-vcs-core/src/git/object.rs`.
- Modify: `crates/bolt-vcs-core/src/save.rs` (the extra-field names as constants).

**Interfaces:**
```rust
pub struct Commit<'a> { pub tree: GitId, pub parents: Vec<GitId>, pub author: &'a [u8], pub committer: &'a [u8],
                        pub encoding: Option<&'a [u8]>, pub change_id: Option<&'a [u8]>,
                        pub header: &'a [u8], pub message: &'a [u8] }
pub fn parse_commit(data: &[u8], hash: HashKind) -> Result<Commit<'_>, GitError>;
pub struct TreeEntry<'a> { pub kind: EntryKind, pub standard_mode: bool, pub name: &'a [u8], pub id: GitId }
pub fn parse_tree(data: &[u8], hash: HashKind) -> Result<Vec<TreeEntry<'_>>, GitError>;
pub struct TagObject<'a> { pub object: GitId, pub kind: ObjectType, pub name: &'a [u8], pub message: &'a [u8] }
pub fn parse_tag(data: &[u8], hash: HashKind) -> Result<TagObject<'_>, GitError>;
pub fn is_lfs_pointer(data: &[u8]) -> bool;
pub fn parse_person(line: &[u8]) -> Person;      // best effort, never fails; exactness comes from git-header
pub fn rebuild_commit(save: &Save, tree: &GitId, parents: &[GitId]) -> Vec<u8>;   // storage-format §4
// object_id comes from Task 9
// save.rs
pub const EXTRA_GIT_COMMIT: &str = "git-commit";
pub const EXTRA_GIT_HEADER: &str = "git-header";
pub const EXTRA_GIT_ORIGINAL_MESSAGE: &str = "git-original-message";
```

**Commits:**
- `tree` comes first, exactly once. Then any number of `parent` lines, up to 1,024; more stops, naming the count. Then exactly one `author` and one `committer`, then any other headers (`key SP value`, with continuation lines that start with a space).
- A NUL in the header is refused, and so is a missing empty line.
- `encoding` and `change-id` are taken from their first occurrence.
- `header` is every byte before the empty line; `message` is everything after it.

**Trees:**
- Entries are `mode SP name NUL id`.
- Standard modes are 100644, 100755, 120000, 40000 and 160000.
- Accepted with `standard_mode = false`:
  - zero-padded forms of the standard modes;
  - other `100…` regular modes, whose kind comes from the owner-execute bit, as Git's `canon_mode` decides.
- Any other mode is `Broken`.
- Entries must be in Git's order (a folder compares as if it ended in `/`) and strictly increasing. Duplicates are `Broken`, including a file and a folder of one name.
- Name policy is the importer's (Task 12).

**Tags:**
- `object`, `type` and `tag` lines, an optional `tagger` and other headers, then an empty line and the message, signature included.

**LFS pointers:**
- At most 1,024 bytes, beginning with `version https://git-lfs.github.com/spec/v1\n` or `version https://hawser.github.com/spec/v1\n`.
- They hold an `oid sha256:` line with 64 lowercase hex digits and a `size` line of digits, as Git LFS's own pointer parser requires.

**`rebuild_commit`:** exactly storage-format.md §4's standard form, or `git-header` + LF + the message bytes. The message bytes are `git-original-message` if present, otherwise the message.

- [ ] **Step 1: Write the failing tests.**

```rust
#[test] fn good_commits_parse() { /* a table: gpgsig and mergetag with continuation lines, -0000, encoding, change-id, many parents */ }
#[test] fn bad_commits_are_refused() { /* two trees, author before parent, missing committer, NUL, bad continuation, 1,025 parents */ }
#[test] fn trees_parse_and_order_is_checked() { /* standard and accepted modes; out of order; duplicates; a file and a folder of one name */ }
#[test] fn tags_parse() {}
#[test] fn lfs_pointers_and_lookalikes() { /* a table of true and false cases */ }
#[test] fn a_standard_commit_rebuilds_without_git_header() {
    let save = save_from_commit_fields(&c);              // test helper doing what Task 12 does, minus storage
    assert_eq!(rebuild_commit(&save, &c.tree, &c.parents), raw);
}
#[test] fn an_unusual_commit_rebuilds_from_git_header() { /* gpgsig, -0000, odd author line */ }
#[test] fn people_parse_best_effort() { /* missing email, huge timestamp, bad zone: fields defaulted, never an error */ }
```

- [ ] **Step 2:** Run. Expected: compile errors.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run fmt, clippy and the tests.
- [ ] **Step 5:** Commit: `Parse Git commits, trees and tags, and rebuild a commit from its save`.

### Task 12: Turning Git objects into Bolt objects

Decisions 032, 035, 036 and 037.

**Files:**
- Create: `crates/bolt-vcs-core/src/import.rs`.
- Modify: `crates/bolt-vcs-core/src/id.rs` (`ChangeId::from_git_commit`), `Cargo.toml` (`encoding_rs`).

**Interfaces:**
```rust
impl ChangeId { pub fn from_git_commit(commit_id: &[u8], change_id_header: Option<&[u8]>) -> ChangeId; }
pub struct Converter<'r> { /* the Git repo, the store, the packs, memo tables, counters */ }
impl<'r> Converter<'r> {
    pub fn new(git: &'r git::Repo, store: &'r Store, packs: &'r mut PackSet) -> Converter<'r>;
    pub fn commit(&mut self, id: &GitId, parent_saves: &[Id]) -> Result<Id, ImportError>;   // stores the save, its trees and blobs
    pub fn tag(&mut self, target: &GitId) -> Result<TagOutcome, ImportError>;
    pub fn counts(&self) -> &Counts;    // inexact commits; skipped entries (first 100, plus a count)
}
pub enum TagOutcome { Tag(diary::Tag), Skipped { reason: String } }
pub enum ImportError { Git(GitError), Repo(RepoError), Store(StoreError), Pack(PackError),
                       Stop { what: String, hint: Option<String> },
                       NotUtf8Names { first: Vec<(String, String)>, count: u64 },   // (commit hex, escaped path)
                       LfsPointer { commit: String, path: String } }
```

**Blobs:**
- A memo maps each Git ID to (content ID, size).
- Blobs of at most 1,024 bytes are read and tested with `is_lfs_pointer`. A pointer stops the import, naming the commit and path (decision 037).
- Blobs up to 64 MiB are read whole: Bolt computes its ID, checks the store with `has_good_copy`, and stores with `add_content`.
- Bigger blobs are streamed twice: one `blob_reader` into a hasher, then a second `blob_reader` into `PackSet::add_content_from` with the expected ID.

**Trees:**
- Converted with an explicit stack.
- A memo maps each Git tree ID to { listing ID, entries counting every path, depth, exact }.
- **Names:**
  - names that aren't valid UTF-8 are collected, and the import stops with the list (decision 019);
  - `.bolt` spellings are skipped with a warning (decision 019);
  - `is_git_alias` names, `.`, `..`, empty names and names containing `/` are skipped with a warning (decision 035);
  - names over 255 bytes stop the import, listing where they are.
- **Kinds:**
  - a submodule becomes a project-link record `{ system: "git", url: "", commit: the bytes }` (decision 036);
  - a symlink's target is its content.
- **Limits:** the root's totals are checked against 4,096 levels and 10,000,000 entries.
- **Exactness:** a tree is exact only if every mode in it, and in everything below it, is standard and no entry was skipped.

**Commits:**
- The save:
  - `root`, the given parents;
  - `author` and `saver` from `parse_person`, each field cut to 4 KiB at a character boundary;
  - the message: UTF-8 as it is, or decoded with `encoding_rs` from the declared encoding (UTF-8 lossy for an unknown label). The original bytes go into `git-original-message` whenever the save's message bytes differ;
  - `change_id = ChangeId::from_git_commit(id, commit.change_id)`;
  - `git-commit` = the hex ID.
- If `rebuild_commit` doesn't give back the original bytes, Bolt adds `git-header` and rebuilds again. A second mismatch stops the import: "Bolt couldn't keep commit `<hex>` exactly; please report this".
- The import stops, naming the commit, at:
  - a message over 1 MiB;
  - a message that isn't UTF-8 and is over 64 KiB;
  - a header over 64 KiB.
- A commit whose root tree isn't exact counts as inexact.

**Tags:**
- A ref straight to a commit is a lightweight tag: no Git tag objects and an empty message.
- An annotated tag is followed through at most 16 tag objects, each at most 1 MiB, to a commit. Its message is the outermost tag's, converted like a commit's and cut at a character boundary to 64 KiB. `git_tags` holds every tag object's raw bytes, outermost first.
- A tag ending at a tree or blob is `Skipped`, with the reason.

- [ ] **Step 1: Write the failing tests.** Build each case with `Craft` and a fresh repository.

```rust
#[test] fn change_ids_follow_jujutsus_rule() {
    let id = [0x01u8; 20];
    let expected: Vec<u8> = id[4..].iter().rev().map(|b| b.reverse_bits()).collect();
    assert_eq!(ChangeId::from_git_commit(&id, None).0.to_vec(), expected);
    assert_eq!(ChangeId::from_git_commit(&id, Some(b"zyxwvutsrqponmlkzyxwvutsrqponmlk")).0,
               [0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, 0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef]);
}
#[test] fn blobs_are_stored_and_big_ones_streamed() {}
#[test] fn every_kind_of_entry_converts() { /* file, executable, symlink, folder, submodule (empty url) */ }
#[test] fn dangerous_names_are_skipped_and_listed() { /* .GIT, git~1, .g\u{200c}it, ".." and "a/b" as single raw names */ }
#[test] fn bolt_names_are_skipped_with_a_warning() {}
#[test] fn a_long_name_stops_the_import() {}
#[test] fn names_that_arent_utf8_stop_with_a_list_of_all() {}
#[test] fn a_tree_bomb_stops_at_the_entry_limit_quickly() { /* 4 levels of 1,000 shared entries */ }
#[test] fn a_deep_tree_stops_at_4096_without_running_out_of_stack() {}
#[test] fn commit_fields_carry_over() { /* author, saver, times, zones, message, parents */ }
#[test] fn git_header_is_kept_only_when_needed() {}
#[test] fn every_commit_rebuilds_to_its_own_id() { /* a table of unusual commits */ }
#[test] fn a_latin1_message_is_decoded_and_its_bytes_kept() {}
#[test] fn an_oversized_message_stops_the_import() {}
#[test] fn an_lfs_pointer_stops_the_import_naming_it() {}
#[test] fn tags_convert() { /* lightweight; annotated with a signature; a chain of 2; a tag of a tree (skipped); a chain of 17 (stops) */ }
#[test] fn converting_twice_gives_identical_saves() { /* two fresh stores: identical save IDs */ }
```

- [ ] **Step 2:** Run. Expected: compile errors.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run fmt, clippy and the tests.
- [ ] **Step 5:** Commit: `Turn Git blobs, trees, commits and tags into Bolt objects, exactly and repeatably`.

### Task 13: `bolt import`

Decisions 033, 034, 037, 039 and 040.

**Files:**
- Modify:
  - `crates/bolt-vcs-core/src/{import.rs,repo.rs}`;
  - `crates/bolt-vcs/src/{main.rs,output.rs}`;
  - `docs/json.md` (the import document).
- Create: `crates/bolt-vcs/tests/import.rs`. The two tests that need fault points or a progress hook run in-process, in `import.rs`'s own tests.

**Interfaces:**
```rust
pub struct ImportRequest<'a> { pub who: &'a Person, pub command: &'a str, pub progress: &'a mut dyn FnMut(u64, u64) }
pub struct ImportReport { pub commits: u64, pub lines: u64, pub servers: u64, pub server_lines: u64, pub tags: u64,
    pub position: Position, pub renamed: Vec<Renaming>, pub skipped_refs: Vec<String>,
    pub skipped_tags: Vec<(String, String)>, pub skipped_entries: Vec<(String, String)>,
    pub skipped_entry_count: u64, pub inexact_commits: u64, pub warnings: Vec<Warning> }
pub struct Renaming { pub kind: &'static str, pub git: String, pub bolt: String }   // kind: line, server, server_line, tag
pub fn import_git(root: &Path, req: &mut ImportRequest) -> Result<ImportReport, ImportError>;
impl Repository {
    pub fn begin(root: &Path) -> Result<(Repository, ChangeLock), RepoError>;    // init's checks and folders; no entry, no format
    pub fn commit_first(&mut self, lock: &ChangeLock, packs: PackSet, entry: &DiaryEntry) -> Result<Id, RepoError>; // format last
}
```
`Repository::init` becomes `begin` + its empty save + `commit_first`. Its one change in behaviour is deliberate: `begin` refuses a `.bolt` folder another tool made (step 1 below), so `bolt start` no longer settles into bolt.new's folder either. Add a `cli.rs` test, `start_refuses_another_tools_bolt_folder`.

**The flow:**
1. **What already exists.**
   - A `.bolt` with `format` refuses: "This folder is already a Bolt project. Picking up new Git commits comes in a later version" (decision 040).
   - A `.bolt` without `format` is an interrupted start or import, and this run finishes it. Exception: if it holds anything besides Bolt's own entries (`format`, `store`, `state`, `cache`, `tmp`, `config.toml`), it is another tool's folder, such as bolt.new's. Then Bolt refuses: "There's a .bolt folder here that Bolt didn't make (bolt.new and other tools use that name); move or rename it first."
   - An import inside another Bolt project refuses, as `start` does.
2. `git::Repo::open(root)`, then `refs()`.
3. **Names and limits.** The renaming rule is applied to lines, servers, server lines and tags. The limits are checked (Global Constraints).
4. **Where to start.**
   - The roots are branches, server lines, tags (peeled) and a detached `HEAD`.
   - A `HEAD` branch with no commit stops: "Git's current branch `<name>` has no commits yet; switch to a branch with commits, or commit first."
   - No commits at all stops: "There's nothing to import: this Git repository has no commits. bolt start makes a new project here."
5. **Pass 1** reads every commit's parents iteratively. They are ordered parents first, with ties broken by Git ID.
6. **Pass 2** converts the commits in that order (Task 12), calling `progress(done, total)` after each.
7. **Tags:** `Converter::tag` for each one.
8. **The state:**
   - the lines;
   - a `TagList` and a `ServerLines` record, if any, stored through the packs;
   - the position: `OnLine` for the `HEAD` branch's renamed line, or `AtSave` for a detached `HEAD`.
9. **The diary entry** records `import` as its command. Then `commit_first`.
10. **Warnings:**
    - each `.bolt` folder Git checked out below the top (found in `HEAD`'s tree): "Git checked out a folder named .bolt at `<path>`. Bolt left it out of history, but commands run inside it would use it as a Bolt project; delete it unless you made it.";
    - `.git/info/grafts` exists: "Git's grafts file was ignored; Bolt imported the commits as they are."

On any error, nothing is committed. Running `bolt import` again finishes the job, reusing stored objects after checking them (decision 041).

**People see** (stdout):
```text
Imported 1,234 commits from Git: 5 lines, 12 server lines from 1 server, 8 tags.
You're on line main.
Renamed 1 name that Bolt's names don't allow: feature+x → feature-x
```
Then come skipped refs, skipped tags, the first 100 skipped entries and a count, and inexact commits ("N commits can't be recreated exactly by a later export"). Warnings go to stderr. Progress, "Importing commits: n of total", goes to stderr only in a terminal, at most 4 times a second.

**The JSON document** (`docs/json.md`): `bolt_json`, `commits`, `lines`, `servers`, `server_lines`, `tags`, and `position` as `{"line": …}` or `{"save": …}`. Then `renamed` as `[{kind, git, bolt}]`, `skipped_refs`, `skipped_tags` as `[{git, reason}]`, `skipped_entries` as `{count, first: [{commit, path}]}`, `inexact_commits` and `warnings`.

- [ ] **Step 1: Write the failing tests** (`crates/bolt-vcs/tests/import.rs`, building repos with `support::git`):

```rust
#[test] fn a_simple_repository_imports() {
    // two branches, one tag, HEAD on main; then `bolt history` lists main's commits newest first,
    // with Git's authors and messages, and `bolt show` shows the latest
}
#[test] fn server_branches_become_server_lines() { /* refs/remotes/origin/x → x@origin, read through Repository::server_lines */ }
#[test] fn names_bolt_refuses_are_renamed_and_listed() { /* feature+x; Feature and feature in packed-refs */ }
#[test] fn a_detached_head_puts_you_at_its_save() {}
#[test] fn an_unborn_branch_stops_the_import() {}
#[test] fn an_empty_repository_stops_the_import() {}
#[test] fn a_second_import_is_refused() {}
#[test] fn another_tools_bolt_folder_is_refused() { /* .bolt/config.json */ }
#[test] fn import_inside_another_project_is_refused() {}
// in its own test binary, crates/bolt-vcs-core/tests/import_interrupted.rs, because
// set_test_pack_limit is process-wide (as crates/bolt-vcs-core/tests/pack_limit.rs does):
#[test] fn an_interrupted_import_can_be_run_again() {
    // set_test_pack_limit(Some(3)); stop the import at the packs:rolled-over fault point (sweep the fault
    // positions as pack_limit.rs does, or add a small arm_at(label, nth) to fsio::fault); no .bolt/format;
    // import_git again succeeds; its save IDs equal those of a clean import in another folder
}
// in crates/bolt-vcs-core/src/import.rs's tests (in-process):
#[test] fn import_stops_cleanly_when_git_removes_a_pack() {
    // the progress callback deletes objects/pack/*.pack after the first commit:
    // Err(ImportError::Git(GitError::Changed { .. })); no .bolt/format
}
// in crates/bolt-vcs/tests/import.rs:
#[test] fn the_same_repository_imports_to_the_same_saves_anywhere() {}
#[test] fn nested_bolt_folders_are_warned_about() {}
#[test] fn progress_is_not_printed_without_a_terminal() {}
#[test] fn import_json_output() { /* insta snapshot of the document */ }
```

- [ ] **Step 2:** Run. Expected: failures.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run fmt, clippy and the tests.
- [ ] **Step 5:** Commit: `Add bolt import`.

### Task 14: The first save after an import stops at converted line endings

Decision 039, part 2.

**Files:**
- Modify: `crates/bolt-vcs-core/src/snapshot.rs` (`SaveRequest.accept_crlf`, `SaveError::LineEndingsChanged`), `crates/bolt-vcs/src/main.rs` (`save --accept-crlf`).
- Test: `crates/bolt-vcs/tests/import.rs`.

**Interfaces:** `SaveRequest { …, pub accept_crlf: bool }`; `SaveError::LineEndingsChanged { count: u64, first: Vec<String> }`.

**The rule:**
- It applies when the base save has `git-commit` and `accept_crlf` is false.
- Each file or executable whose content changed, and whose size on disk is between the base size + 1 and twice the base size, is compared as a stream. It is flagged when its bytes are exactly the base bytes with each LF turned into CRLF, and the base holds no CR.
- If any file is flagged, the save stops and writes nothing.
- The message names up to 5 files and the count: "N files differ from what Git stored only in line endings: Git turned LF into CRLF when it wrote them out (core.autocrlf)."
- The hint, verbatim: "To keep history as Git stored it, commit or stash your changes in Git, then run `git config core.autocrlf false` and `git checkout -- .`, and save again. To save the files with CRLF anyway: bolt save --accept-crlf "message"".

- [ ] **Step 1: Write the failing tests.**

```rust
#[test] fn converted_line_endings_stop_the_first_save() {
    // import a repo; rewrite two text files with CRLF; `bolt save m` exits 1, names both, history unchanged
}
#[test] fn accept_crlf_saves_them() {}
#[test] fn a_real_edit_is_not_mistaken_for_a_conversion() {}
#[test] fn later_saves_dont_check() { /* after one save on top, CRLF changes are saved normally */ }
```

- [ ] **Step 2:** Run. Expected: failures.
- [ ] **Step 3:** Implement. The comparison streams, so it never holds a file whole.
- [ ] **Step 4:** Run fmt, clippy and the tests.
- [ ] **Step 5:** Commit: `Stop the first save after an import when Git converted line endings`.

### Task 15: Differential tests against Git

This is decision 002's test infrastructure, as testing.md describes it.

**Files:**
- Create: `crates/bolt-vcs/tests/import_vs_git.rs`.
- Modify: `crates/bolt-vcs/tests/support/mod.rs`, `.github/workflows/nightly.yml` (a `differential` job), `docs/testing.md` §3 and §4.

**`support::git(dir, args)`** runs Git with a clean environment:
- the environment is cleared except `PATH`, `SystemRoot` and `TEMP`;
- `HOME` points to a temp folder;
- `GIT_CONFIG_NOSYSTEM=1`, `GIT_CONFIG_GLOBAL` = an empty file, `GIT_NO_REPLACE_OBJECTS=1`, `LC_ALL=C`;
- `-c core.autocrlf=false -c user.name=Test -c user.email=test@example.com -c commit.gpgsign=false -c init.defaultBranch=main`.

**`support::assert_matches_git(project)`:**
- Every commit from `git rev-list --all` and `HEAD` has exactly one save with that `git-commit`.
- Its `rebuild_commit` bytes equal `git cat-file commit <id>`.
- Every distinct Git tree, read once through `git cat-file --batch`, maps to its listing:
  - the same names, after Bolt's skip rules;
  - kinds from modes;
  - file contents whose Git blob ID (SHA-1 of `blob <len>\0` + bytes, computed in the test) equals the entry's ID.
- Lines, server lines and tags equal `git for-each-ref` after the renaming rule.
- Annotated tags' bytes equal `git cat-file tag`.

- [ ] **Step 1: Write the tests.** These run on every push, on all three systems.

```rust
#[test] fn everyday_history_matches_git() {
    // files, folders, `git update-index --chmod=+x`, symlinks and case twins and a submodule via
    // `update-index --cacheinfo`, renames, deletions, empty commits, a merge and an octopus merge,
    // Unicode names, branches including feature+x, refs/remotes/origin/*, lightweight, annotated
    // and nested tags, a tag of a tree and of a blob, then the same with a detached HEAD
}
#[test] fn unusual_commits_match_git() { /* `git hash-object -t commit --literally -w`: gpgsig, mergetag, encoding ISO-8859-1, change-id, -0000, odd author line, unknown header */ }
#[test] fn packed_and_loose_objects_match_git() { /* `git repack -adf --depth=50`, then some loose objects, `git pack-refs --all` */ }
#[test] fn sha256_repositories_match_git() { /* `git init --object-format=sha256`; skipped with a printed reason if git is older than 2.29 */ }
#[test] fn optional_git_files_are_ignored() { /* commit-graph, multi-pack-index, a bitmap, worktree config */ }
#[test] fn windows_cant_hold_some_names_and_saving_keeps_them() {
    // `update-index --cacheinfo` adds aux.c, a:b, README and Readme; import; save; the names
    // are still in the new save on every system
}
#[test] #[ignore = "nightly: needs BOLT_DIFF_REPO, a full clone of a real repository"]
fn a_real_repository_matches_git() { /* prints the import's time and peak memory */ }
```

- [ ] **Step 2:** Run. Expected: pass. Any mismatch is a bug in Tasks 9–13, and it is fixed there with its own test.
- [ ] **Step 3:** Add the nightly job. It clones `https://github.com/git/git` with full history into a temp folder, then runs `cargo test --release -p bolt-vcs --test import_vs_git -- --ignored a_real_repository_matches_git` with `BOLT_DIFF_REPO` set. Update testing.md: what runs on every push, what runs nightly, and the measured time and memory.
- [ ] **Step 4:** Run fmt, clippy and the tests. Run the nightly test once locally on a clone of `git/git`, and record its time and peak memory in testing.md §3.
- [ ] **Step 5:** Commit: `Compare every imported commit with Git`.

### Task 16: Hostile Git repositories: the corpus and fuzz targets

This is security.md §7's review of Git's published vulnerabilities, done during planning on 2026-11-02.

**Files:**
- Create: `crates/bolt-vcs/tests/corpus_git.rs`; `fuzz/fuzz_targets/{git_loose,git_pack,git_delta,git_objects,git_refs,git_config}.rs`.
- Modify: `fuzz/Cargo.toml`, `.github/workflows/nightly.yml`, `docs/security.md` (§4 T1, T2, T3, T6, T7, T11, T19, T24; §6; §7), `docs/testing.md` §3, `docs/plans/2026-11-01-milestone-1-follow-ups.md`.

**The corpus.** One test each, built with `Craft`. Each asserts the exact refusal, or the skip plus its warning, and that nothing is committed when the import stops.

| Case | Input | Expected |
|---|---|---|
| `.git` spellings (CVE-2014-9390, CVE-2019-1349, CVE-2019-1352) | `.git`, `.GIT`, `git~1`, `.git.`, `.git ` , `.git::$INDEX_ALLOCATION`, `.g\u{200c}it` and each other HFS-ignorable code point, at the root and nested, each holding `hooks/post-checkout` | skipped with a warning; nothing under them imported |
| Path parts | single raw names `..`, `.`, empty, `a/b`, `.git\hooks\x` | the first four skipped; `.git\hooks\x` imported as one name and never written out on Windows (milestone 3) |
| `.bolt` spellings | `.bolt`, `.BOLT`, `bolt~1` | skipped with a warning (decision 019) |
| Long and bad names | a 256-byte name; names that aren't UTF-8 in two commits | stops; the second lists both |
| Broken trees | duplicate names; a file and a folder of one name; wrong order; mode `160001`; an entry naming a missing object | stops, naming the commit and folder |
| Accepted modes | `040000`, `100664` | imported; counted as inexact |
| Depth and size (CVE-2016-2324) | a tree 100,000 levels deep; 4 levels of 1,000 shared entries | stops at 4,096 levels and at 10,000,000 entries, quickly, with no stack overflow |
| Ambiguous commits (go-git 2026) | two `tree` lines, two `author` lines, `parent` before `tree`, a truncated `gpgsig`, a NUL in the header, 1,025 parents | stops |
| Unusual commits | `-0000`, an odd author line, `encoding`, `gpgsig`, `mergetag`, an unknown header, a `change-id` | imported exactly; each rebuilds to its own ID |
| Tags | a tag of a tree; a chain of 17; a tag object of 1 MiB + 1 | skipped; stops; stops |
| Refs (go-git CVE-2026-71557; gitoxide CVE-2024-35197) | `refs/heads/../../config` in packed-refs; a loose ref that is a link; `refs/heads/CON` on Windows; a malformed packed-refs line | stops before opening anything outside `refs/` |
| Names of lines | `Feature` and `feature`; `feature+x`; 65 servers | renamed `-2`; renamed `feature-x`; stops |
| `HEAD` | an unborn branch; a missing commit; `ref: refs/remotes/x` | stops with the matching message |
| Repository shape (CVE-2022-39253, CVE-2024-32021) | `.git` a link or junction; `gitdir:` file; `objects/` a link; a pack that is a link; `alternates`; `commondir`; `shallow`; sparse checkout; reftable; `partialclone`; an unknown extension | refused, saying what to do; nothing read through a link |
| Ownership (CVE-2024-32004, CVE-2024-32465) | `.git` owned by another account (stubbed owner check) | refused unless the folder is in `safe-folders` |
| Loose objects (libgit2 CVE-2026-53585) | header `blob 99999999999`; a zlib bomb; trailing garbage | refused before allocating |
| Packs and deltas (gitoxide GHSA-x494, GHSA-x862, GHSA-mpf5, GHSA-633h; dulwich CVE-2026-114095; libgit2 CVE-2018-10887/10888) | every truncation point; object count 0xFFFFFFFF; `OFS_DELTA` distance 0 and past the start; `REF_DELTA` A↔B; a chain 4,097 deep; a delta claiming 2^62 bytes; a copy past the base; pack version 3 | an error, never a panic or an abort |
| Hashes (gitoxide CVE-2025-31130) | a SHA-1 collision block; an object whose ID doesn't match | refused |
| Speed-up files (gitoxide GHSA-989c) | a commit-graph with wrong parent positions; a garbage multi-pack index | ignored; the import is right |
| LFS (decision 037) | a pointer; a lookalike | stops, naming the file and commit |
| Submodules (CVE-2018-11235, CVE-2018-17456, CVE-2019-1387, CVE-2025-48384; T24) | `.gitmodules` with `../` names, `-` URLs, `update = !cmd`, a trailing CR; a linked `.gitmodules` | imported as an ordinary entry, never read; links hold an empty URL; a later save doesn't enter the submodule's folder, whose `.git` file points to `/etc` |
| Symlinks (CVE-2021-21300, CVE-2024-32002, GHSA-xw69) | `A -> .git/hooks` beside `a/post-checkout`; targets `\\attacker\s\x`, `//attacker/s`, `/etc/passwd` | imported as links; no target is opened or inspected |
| Nested `.bolt` (save) | `HEAD`'s tree holds `sub/.bolt/format` | warned about by the import |
| Terminal (T21; CVE-2024-52005) | ref names, author names and messages with bidi controls and escapes | escaped in every message and summary |

**Fuzz targets.** Each one must never panic, hang or allocate past the limits:
- `git_loose`: bytes as a loose object file;
- `git_pack`: input split into an `.idx` and a `.pack`, then every object read;
- `git_delta`: a base and a delta;
- `git_objects`: `parse_commit`, `parse_tree` and `parse_tag`;
- `git_refs`: `packed-refs` and ref names;
- `git_config`: the config parser.

Each gets 5 minutes nightly and 1 minute on pushes, like the others.

**Docs:**
- security.md §7: the review item is done (2026-11-02, cases in `corpus_git.rs`).
- §4: the rows mention the import's defences.
- §6: corpus counts.

**Follow-ups for milestone 3.** These are the review's write-out cases, added to the follow-ups table:
- never write a `.git` spelling on any system, including NTFS or vfat drives mounted on Linux (CVE-2019-1353);
- compare against the real short names of `.bolt` and `.git` (`GetShortPathNameW`), since NTFS switches to hashed short names after four clashes;
- refuse a link whose name collides with a folder's on a drive that ignores case (CVE-2021-21300, CVE-2024-32002), and create links last;
- never inspect a link's target (Git for Windows' NTLM leak, gitoxide GHSA-xw69);
- refuse `\` and drive letters (CVE-2019-1354, CVE-2019-1351);
- refuse device names, including `COM¹`–`COM³` and `LPT¹`–`LPT³`;
- write explicit 0644 and 0755 modes (gitoxide CVE-2025-22620);
- keep no cache of checked folder prefixes (gitoxide 2026);
- for milestone 5, apply Git LFS's write-out lessons (CVE-2025-26625).

- [ ] **Step 1:** Write the corpus tests. Expected: all pass, since Tasks 9–13 built the defences. A failing one is a bug, fixed with its test.
- [ ] **Step 2:** Add the fuzz targets and the nightly entries. Run each locally for 60 seconds (Linux or WSL).
- [ ] **Step 3:** Update the docs and the follow-ups.
- [ ] **Step 4:** Run fmt, clippy and the tests.
- [ ] **Step 5:** Commit: `Add the hostile Git repository corpus and fuzz targets for the Git reader`.

### Task 17: Milestone 1 follow-ups due now: behaviour

**Files:**
- Modify: `crates/bolt-vcs-core/src/{repo.rs,config.rs,safe_fs.rs}`, `crates/bolt-vcs/src/main.rs`.
- Test: unit tests and `cli.rs`.

Each item comes with its test:

1. A missing `.bolt/tmp` or `.bolt/cache` is recreated, not reported as damage. A missing `store/packs` or `state` stays damage.
   - Test: `a_missing_tmp_or_cache_is_recreated`.
2. `ignored_identity_note` shows at most 256 characters of a name or email, then `…`. When `BOLT_USER_NAME` or `BOLT_USER_EMAIL` is set, its hint says the settings line won't take effect while they are.
   - Test: `the_identity_note_is_short_and_honest`.
3. `start` reports success, with a warning, when reading its save back after committing fails, since the project exists.
   - Test: through a fault point `start:read-back`.
4. A `safe-folders` line below a table (such as `[user]`) is warned about: "safe-folders must be at the top of `<file>`, before any [section]".
   - Test: `safe_folders_in_a_table_is_warned_about`.
5. The message that names the settings file twice names it once.
6. The trust hint never offers `safe-folders` for a folder other accounts can write to: on Unix a folder with the other-write bit (`/tmp`), on Windows a drive's root folder. It explains the risk instead.
   - Tests: `the_trust_hint_never_offers_tmp` (Unix) and `the_trust_hint_never_offers_a_drive_root` (Windows).
7. `open_regular`'s doc says `truncate` would act before a file is refused, and `read_capped_following_links` becomes `pub(crate)`.

- [ ] Steps: write the failing tests; run them; implement; run fmt, clippy and the tests; commit `Fix milestone 1's small follow-ups: missing tmp and cache, identity notes and trust hints`.

### Task 18: Milestone 1 follow-ups due now: tests and CI

**Files:**
- Modify: `crates/bolt-vcs/tests/{support/mod.rs,cli.rs,redteam.rs,redteam2.rs,corpus.rs}`, `crates/bolt-vcs-core/src/{snapshot.rs,store.rs,diff.rs,repo.rs}` tests, `.github/workflows/ci.yml`.

1. Shared test helpers: `add_listing`, `plant_save` and the other helpers copied between test files move into `support/mod.rs`, and the copies go.
2. New tests:
   - anchored rules in a nested ignore file;
   - one bad copy read again through streaming (decision 028's "damage on every read");
   - `start`'s search for an enclosing project;
   - decision 027's project-folder owner rule;
   - the diff test asserts `SaveTooLarge` rather than `is_err`.
3. JSON checks use `get(..) == Some(&Value::Null)` instead of `is_null`.
4. CI: the `deny` job also runs `cargo deny --manifest-path fuzz/Cargo.toml check`.

- [ ] Steps: make the changes; run fmt, clippy and the tests; push and check the `deny` job; commit `Close milestone 1's test gaps and check the fuzz crate's dependencies`.

### Task 19: The docs match the milestone

- [ ] **Step 1:** Update the docs:
  - [storage-format.md](../storage-format.md) §13 lists the new vectors.
  - [testing.md](../testing.md) gets the differential tests, the corpus, the fuzz targets and the CI jobs.
  - [security.md](../security.md) gets every row the milestone changed.
  - [design.md](../design.md): the phase line.
  - [milestones.md](../milestones.md): milestone 2's steps are ticked.
  - `docs/json.md` is complete.
- [ ] **Step 2:** Update [the follow-ups](2026-11-01-milestone-1-follow-ups.md):
  - Strike every item this milestone finished, with the date.
  - Add the items it defers:
    - catching up with Git (decision 040);
    - reftable (031);
    - the name-length limit (035);
    - line-ending conversion (039);
    - the measured memory of an import's memo tables, and a disk-backed map if a real repository needs one;
    - Task 16's write-out cases.
- [ ] **Step 3:** Add the owner's manual checks to the milestone's done-when in milestones.md:
  - import one of the owner's own repositories;
  - try a project inside a OneDrive folder (decision 026 #4).
- [ ] **Step 4:** Commit: `Bring the docs up to date with milestone 2`.

### Task 20: Red-team session

Decision 024.

- [ ] **Step 1:** Write the attack-surface notes the attacker gets:
  - hostile Git repositories: `.git` shape, config, refs and packed-refs, loose objects, packs, indexes and deltas, commits, trees and tags, LFS pointers, names;
  - the save after an import: `.gitignore`, saved files staying saved, kept entries, project links, the line-ending stop;
  - the new diary records;
  - commands that write several packs;
  - `bolt import`'s output and its spoofing risks;
  - resources: the memo tables, decompression and delta bombs, deep chains, tree bombs.
- [ ] **Step 2:** Run the session:
  - A fresh agent on the most capable model attacks a copy of the branch in `redteam/sandbox.sh up <copy>`: no network, working only in the copy and the container.
  - Windows-only attacks become tests on a `redteam/m2` branch run by CI, and that branch is deleted afterwards.
  - Proof tests go in `crates/bolt-vcs/tests/redteam3.rs`, ignored until fixed. The report goes in `docs/redteam/<date>-milestone-2.md`.
- [ ] **Step 3:** Fix every proven finding, each with its proof test un-ignored, or rule on it in writing (a decision record). New threats join security.md §4, and deferred items join the follow-ups.
- [ ] **Step 4:** If the session proved anything critical or high, run a short second session on the fixes, as milestone 1 did.
- [ ] **Step 5:** Commit each fix separately, with the finding's ID in the message.

### Task 21: Whole-branch review and merge

- [ ] **Step 1:** A fresh reviewer on the most capable model reads the whole branch against `main`. It checks:
  - correctness;
  - the engineering rules;
  - security;
  - that every document in `docs/` matches the code and today's state.
- [ ] **Step 2:** Fix the findings, or rule on them in writing.
- [ ] **Step 3:** Run fmt, clippy and the tests locally, and push.
- [ ] **Step 4:** Open the pull request `milestone-2` → `main`, with no Claude attribution.
- [ ] **Step 5:** Wait for CI on all three systems, `deny` and CodeQL to go green. Check that the code-scanning, Dependabot and secret-scanning alerts are clean (`gh api repos/WisHonor/Bolt/code-scanning/alerts`, and the same for `dependabot` and `secret-scanning`).
- [ ] **Step 6:** Merge the pull request with a merge commit.
