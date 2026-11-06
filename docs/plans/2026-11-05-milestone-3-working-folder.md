# Milestone 3 ("Working folder") Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `status` (with the file-state cache), `changes`, `goto`, `discard`, partial saves, safety copies and basic lines work end to end on Windows, macOS and Linux, writing into the project folder without ever going through a link, outside the project, or into `.git` or `.bolt`; packs are searched on disk, opened when needed and merged.

**Architecture:** A folder-handle type in the platform crate (`Dir`) does every read and write inside the project relative to a held folder, refusing links at each name (rustix on Unix, Windows' own calls). The save's walk moves onto it. A keyed file-state cache lets `status` and `save` skip unchanged files. A write-out module plans a switch, journals it, and applies it one file at a time; `goto` and `discard` are built on it, with safety copies in the diary entry. Packs get a fixed-width index searched on disk, a few open at a time, merged when there are many.

**Tech Stack:** Rust 1.99.1, the existing crates, and two new ones, each reviewed under security.md T18: `rustix` (platform crate, Unix; decision 048) and `similar` (engine; decision 058). Tests use `git` only where milestone 2's tests already do.

**Spec:** decisions [045](../decisions/045-link-targets-stored-with-slashes.md) to [059](../decisions/059-bolt-spellings-left-out-like-git-spellings.md), with 003, 004, 009 to 012, 019, 022 to 026, 028, 039 and 043; [design.md](../design.md) §5.1, §5.2, §5.7, §5.9, §5.10, §5.15; [storage-format.md](../storage-format.md) §3, §9 to §12; [security.md](../security.md) §4 and §7; [testing.md](../testing.md); the milestone 3 rows of [the follow-ups](2026-11-01-milestone-1-follow-ups.md).

**Rulings made while building:** `2026-11-05-milestone-3-rulings.md` (copied from the progress log before it is deleted).

## Global Constraints

- Everything in CLAUDE.md applies.
  - `#![forbid(unsafe_code)]` in the engine. `unsafe` only in `bolt-vcs-platform`, each block with a `// SAFETY:` comment. **Task 1 adds `unsafe` (Windows calls): it gets a line-by-line walkthrough for the owner, and the milestone isn't merged until the owner has reviewed it.** No other task may add `unsafe`.
  - No `unwrap`, `expect` or `panic` outside tests. One place per rule. Spec before code: the journal (§10) and pack version 2 (§11) get their bytes and test vectors before their code. Tests pass on Windows, macOS and Linux.
- Code comments are short and human: the why in a line or two, no process words, no task numbers.
- Modules that parse bytes from disk (`journal.rs`, `filecache.rs`, `changeids.rs`, pack version 2 in `pack.rs`) start with `#![deny(clippy::indexing_slicing, clippy::arithmetic_side_effects)]`; sizes and counts read from a file are checked against its length before anything is allocated; nothing recurses once per folder level.
- **Every read and write inside the project folder goes through `bolt_platform::Dir`** (decision 048). No `std::fs` call on a path inside the project, except opening the top folder itself. Files in `.bolt` keep going through `safe_fs`, whose fixed folders are opened through `Dir` too (Task 3).
- Untrusted text is escaped before printing (`text::escape`, T21): paths, names, link targets, lines of files in `changes`.
- Messages say what happened, why, and what to do next (decision 017); no questions without a terminal. `--json` documents follow `docs/json.md`, which each task that adds or changes a document updates in the same commit.
- **Limits and fixed values** (from the decisions):

  | What | Value |
  |---|---|
  | A name | at most 765 bytes in the format; a new name a save adds at most 255 bytes; fits this system when Windows' 255 UTF-16 units or another system's 255 bytes hold it (051) |
  | A path | at most 4,096 bytes from the top, `/` between names (051) |
  | Folder handles a walk holds | at most 256; deeper folders are opened again from the deepest held one, a name at a time |
  | Racy files | a file not at least 2 s older than "now", taken from a file written in `.bolt/tmp` at the scan's start, is read again next time (053) |
  | Cache key | 32 random bytes, file `cache-key` in the user's own settings folder (the folder of `config::user_config_path()`) (053) |
  | Big saves | ask when a save adds more than 100,000,000 bytes or 10,000 files not in the save it builds on; `--yes` skips (056) |
  | Packs | version 2 written; at most 16 open; merge after a commit when more than 32 packs, every pack under 64 MiB (057) |
  | Line diffs | binary: a zero byte in the first 8,000 bytes; no lines for files over 64 MiB; 2 s per file (058) |
  | A file in use on Windows | tried again for about 1 s (10 tries, 100 ms apart), then left for the next run (049) |
  | Modes on Unix | 0644, or 0755 for executables, exactly (052) |

- **New warning kinds** (each in json.md §4 when its task adds it): `not_written_here`, `placeholders_written`, `files_in_use`, `changed_since_plan`, `switch_unfinished`, `switch_finished`, `pack_not_merged`, `diff_timed_out`. `git_names_left_out` also names `.bolt` spellings (059).
- **Fixed paths:** `.bolt/state/journal`; `.bolt/cache/files` (file-state cache); `.bolt/cache/change-ids` (change-ID cache).
- **Dependencies:** only `rustix` and `similar`. Any other needs the owner's approval.
- **CI costs minutes** (the repository is private since 2026-11-05): run the whole suite locally before pushing, push at task boundaries, and keep the milestone's pull request closed until Task 24, so each push runs CI once.
- One commit per task unless a step says otherwise, and no Claude attribution.

## Review Focus

1. **A case-only rename between two saves on a drive that ignores letter case** (`Readme.md` in one, `README.md` in the other). Expected: `goto` ends with the new spelling holding the new content, and never deletes the file it just wrote. Test: Task 12 `a_case_only_rename_is_written_on_a_case_blind_drive`.
2. **An editor or antivirus holding a file open on Windows during `goto`.** Expected: the rest is written, the journal stays, the message names the file, and running `goto` again finishes. Test: Task 12 `a_file_held_open_is_left_and_the_next_run_finishes` (Windows; a helper process holds the file).
3. **`goto` stopped halfway (Ctrl+C, power cut).** Expected: every file is wholly old or wholly new, and the next changing command finishes the switch and says so. Test: Task 13 `a_switch_stopped_at_every_write_is_finished_by_the_next_command`.
4. **Unchanged files `status` can't or shouldn't read** (OneDrive files online only, files without read permission). Expected: an unchanged file whose cache entry matches is never opened. Test: Task 9 `status_does_not_open_unchanged_files` (Unix: a file made unreadable after it was cached).
5. **A 300 MiB file written by `goto`.** Expected: streamed, memory well under the file's size. Test: Task 12 `writing_a_big_file_uses_little_memory` (Linux, peak memory through `/proc`).

---

## File structure

```text
crates/bolt-vcs-platform/
  Cargo.toml              rustix (cfg unix); windows-sys features for NT calls
  src/dir.rs              Dir, DirEntry, EntryMeta, EntryType; the shared API (Task 1)
  src/dir/unix.rs         on rustix, no unsafe (Task 1)
  src/dir/windows.rs      NtCreateFile, SetFileInformationByHandle, GetFileInformationByHandleEx (Task 1)
crates/bolt-vcs-core/src/
  names.rs                own_folder, is_bolt_alias, writable_here, length rules (Task 2)
  snapshot/walk.rs        the walk through Dir; metadata per file (Task 3)
  repo.rs                 owner-check races (Task 4); merge after commit (Task 6); finishing a switch (Task 13)
  pack.rs                 pack version 2 (Task 5)
  store.rs                lazy packs, at most 16 open (Task 5)
  merge.rs                merging small packs (Task 6)
  keyed.rs                the user's cache key and keyed checksums (Task 7)
  changeids.rs            change-ID cache (Task 7)
  filecache.rs            file-state cache (Task 8)
  status.rs               what changed, from the walk and the cache (Task 9)
  snapshot.rs             save uses the cache (Task 10); partial saves (Task 17); the big-save question (Task 18)
  journal.rs              state/journal (Task 11)
  writeout.rs             plan and apply a switch (Task 12); finish an interrupted one (Task 13)
  switch.rs               goto (Task 14), discard (Task 15)
  lines.rs                goto's targets, the line list, saving away from a line (Tasks 14, 16)
  changes.rs              line-by-line differences (Task 19)
crates/bolt-vcs/src/main.rs, output.rs   status, changes, goto, discard, line; save's new flags
crates/bolt-vcs/tests/
  status.rs, goto.rs, discard.rs, lines.rs, changes.rs   end to end
  corpus_writeout.rs      hostile saves written out (Task 20)
  model.rs                random edits, saves, goto and discard against a model (Task 21)
crates/bolt-vcs-core/tests/vectors/   journal.json, pack_v2.json (Tasks 5, 11)
fuzz/fuzz_targets/        decode_journal, file_cache, change_ids, decode_project_link, open_pack (v2)
docs/plans/2026-11-05-milestone-3-unsafe-walkthrough.md   the owner's line-by-line review of Task 1
```

---

### Task 1: Folder handles in the platform crate

Decision 048. The only task with `unsafe`.

**Files:**
- Create: `crates/bolt-vcs-platform/src/dir.rs`, `src/dir/unix.rs`, `src/dir/windows.rs`; `docs/plans/2026-11-05-milestone-3-unsafe-walkthrough.md`.
- Modify: `crates/bolt-vcs-platform/Cargo.toml` (`rustix = { version = "1.1.6", features = ["fs"] }` for `cfg(unix)`; windows-sys features `Wdk_Foundation`, `Wdk_Storage_FileSystem`, `Win32_System_IO`, `Win32_System_Kernel` as needed), `src/lib.rs` (`pub mod dir`), `deny.toml` if a licence needs allowing.

**Interfaces:**
```rust
pub struct Dir { /* an open folder: OwnedFd on Unix, OwnedHandle on Windows */ }
pub enum EntryType { File, Dir, Symlink, Other }   // Other: a reparse point that isn't a link, a pipe, a device
pub struct DirEntry { pub name: OsString, pub kind: EntryType, pub short_name: Option<OsString> }   // short name: Windows 8.3 only
pub struct EntryMeta { pub kind: EntryType, pub size: u64, pub modified_ns: i128, pub executable: Option<bool>, pub file_id: (u64, u128) }
impl Dir {
    pub fn open_top(path: &Path) -> io::Result<Dir>;                 // a resolved path; refuses a link at its last name
    pub fn open_dir(&self, name: &OsStr) -> io::Result<Dir>;
    pub fn entries(&self) -> io::Result<Vec<DirEntry>>;              // no "." or ".."
    pub fn metadata(&self, name: &OsStr) -> io::Result<EntryMeta>;   // never follows
    pub fn file_meta(file: &File) -> io::Result<EntryMeta>;          // of an open file
    pub fn open_file(&self, name: &OsStr) -> io::Result<File>;       // read-only; a regular file only
    pub fn create_new(&self, name: &OsStr, executable: bool) -> io::Result<File>;   // exclusive; 0644/0755 on Unix
    pub fn rename_into(&self, from: &OsStr, to: &Dir, to_name: &OsStr) -> io::Result<()>;   // replaces a file at to_name
    pub fn create_dir(&self, name: &OsStr) -> io::Result<()>;
    pub fn remove_file(&self, name: &OsStr) -> io::Result<()>;      // a file or a link itself
    pub fn remove_dir(&self, name: &OsStr) -> io::Result<()>;       // empty only
    pub fn symlink(&self, name: &OsStr, target: &OsStr, to_dir: bool) -> io::Result<()>;
    pub fn read_link(&self, name: &OsStr) -> io::Result<OsString>;
    pub fn set_executable(&self, name: &OsStr, executable: bool) -> io::Result<()>;   // Unix: 0644/0755; Windows: does nothing
    pub fn sync(&self) -> io::Result<()>;
}
pub fn is_link_refusal(e: &io::Error) -> bool;   // the error a link or reparse point at a name gives
```
- A name holding `/` (or `\` on Windows), `.` or `..` is refused with `InvalidInput` before any call.
- **Unix:** `openat` with `O_NOFOLLOW | O_CLOEXEC` (`O_DIRECTORY` for folders), `statat` with `SYMLINK_NOFOLLOW`, `rustix::fs::Dir`, `mkdirat`, `renameat`, `unlinkat`, `symlinkat`, `readlinkat`, `fchmod` on a file opened with `O_NOFOLLOW`.
- **Windows:** `NtCreateFile` with `RootDirectory = self`, `FILE_OPEN_REPARSE_POINT`, `OBJ_DONT_REPARSE` (retried without it where the system refuses the flag, as std's `remove_dir_all` does), then a reparse point found on the opened handle is closed and refused; `GetFileInformationByHandleEx(FileIdBothDirectoryInfo)` for `entries` (with short names), `FileBasicInfo`, `FileStandardInfo`, `FileIdInfo` for metadata; `SetFileInformationByHandle(FileRenameInfoEx)` with `RootDirectory` and `FILE_RENAME_FLAG_REPLACE_IF_EXISTS | FILE_RENAME_FLAG_POSIX_SEMANTICS` for `rename_into`; `FileDispositionInfoEx` with `FILE_DISPOSITION_FLAG_DELETE | FILE_DISPOSITION_FLAG_POSIX_SEMANTICS` for removal; `CreateSymbolicLinkW` with `SYMBOLIC_LINK_FLAG_ALLOW_UNPRIVILEGED_CREATE` by the folder's final path (`GetFinalPathNameByHandleW`), then the new entry is opened through the handle and checked to be that link.

- [ ] **Step 1: Write the failing tests** (`dir.rs` tests, every system):
```rust
#[test] fn a_link_at_a_name_is_refused_at_every_call() {
    // folder `real/`, link `l -> real` (a junction too on Windows), file link `f -> real/x`
    for e in [d.open_dir("l".as_ref()).err(), d.open_file("f".as_ref()).err()] { assert!(is_link_refusal(&e.unwrap())); }
    assert_eq!(d.metadata("l".as_ref()).unwrap().kind, EntryType::Symlink);
}
#[test] fn a_folder_swapped_for_a_link_after_opening_stays_the_folder() {
    // open `a/` as a Dir; rename `a` away and put a link `a -> elsewhere`; create_new through the held Dir
    // lands in the old folder, never in `elsewhere`
}
#[test] fn rename_into_replaces_a_file_atomically() {}
#[test] fn names_with_separators_or_dots_are_refused_before_any_call() {
    for n in ["a/b", ".", "..", ""] { assert_eq!(d.open_dir(n.as_ref()).unwrap_err().kind(), io::ErrorKind::InvalidInput); }
    #[cfg(windows)] assert_eq!(d.open_dir("a\\b".as_ref()).unwrap_err().kind(), io::ErrorKind::InvalidInput);
}
#[test] fn entries_and_metadata_describe_what_is_there() { /* file, folder, link, sizes, executable on Unix */ }
#[cfg(unix)] #[test] fn new_files_get_exact_modes() { /* umask 0o077 or 0o000: 0644 and 0755 exactly */ }
#[cfg(windows)] #[test] fn short_names_come_with_entries() { /* `.git` folder in an NTFS temp dir: some entry has short_name GIT~1 or a hashed one */ }
#[cfg(windows)] #[test] fn links_are_made_for_folders_and_files() { /* skips with a printed reason without the right */ }
```
- [ ] **Step 2:** Run `cargo test -p bolt-vcs-platform`. Expected: compile errors.
- [ ] **Step 3:** Implement. Every `unsafe` block gets a `// SAFETY:` comment; no other crate changes.
- [ ] **Step 4:** Run fmt, clippy and the tests on all three systems (CI).
- [ ] **Step 5:** Write the walkthrough: each `unsafe` block quoted, with what it calls, why it is sound, and what Microsoft's documentation says of each flag (linked). Plain words first.
- [ ] **Step 6:** Commit: `Hold folders open and work inside them, refusing links at every name`.

### Task 2: One module for names, and the new length limits

Decisions 051 and 059; the follow-ups on write-out's name checks.

**Files:**
- Modify: `crates/bolt-vcs-core/src/{names.rs,listing.rs,snapshot/walk.rs,import/convert.rs}`, `fuzz/fuzz_targets/name_rules.rs`, `crates/bolt-vcs-core/tests/vectors/listing.json` (a vector with a 600-byte name), `docs/storage-format.md` §13.

**Interfaces:**
```rust
pub const MAX_NAME_BYTES: usize = 765;
pub const MAX_NEW_NAME_BYTES: usize = 255;
pub const MAX_PATH_BYTES: usize = 4096;
pub fn is_bolt_alias(name: &str) -> bool;                  // every spelling Windows or macOS treats as .bolt
pub fn own_folder(name: &str, drive: Drive) -> bool;       // the top's .bolt or .git as this drive spells them
pub fn fits_here(name: &str) -> bool;                      // Windows: ≤ 255 UTF-16 units; elsewhere ≤ 255 bytes
pub enum NotWritable { GitName, BoltName, NotHoldable(PortabilityProblem), TooLong, ShortNameOf(&'static str) }
pub fn writable_here(name: &str, short_names_here: &[(&str, &str)]) -> Result<(), NotWritable>;
```
- `holdable_here` keeps its meaning (decision 004's rules for this system) and adds `fits_here`. `writable_here` refuses `.git` and `.bolt` spellings on every system (CVE-2019-1353: NTFS and vfat drives mounted on Linux), whatever `holdable_here` refuses here (`\`, `:`, device names including `COM¹`–`COM³` and `LPT¹`–`LPT³`, trailing dots and spaces on Windows), names `fits_here` refuses, and a name equal, ignoring case, to the real short name of a `.git` or `.bolt` in the same folder (`short_names_here`: pairs of long name and short name from `Dir::entries`).
- `is_internal_folder` in the walk goes; it asks `own_folder`. A `.bolt` spelling other than the project's own is left out with the `git_names_left_out` warning, whose text now says "names Git or Bolt keep for themselves".
- Paths: a save refuses a new path over 4,096 bytes; the listing walkers stop past it as past the depth limit; the import stops at one, naming it. A new name over 255 bytes stays refused by the portability check; the import keeps names up to 765 bytes.

- [ ] **Step 1: Write the failing tests:**
```rust
#[test] fn writable_here_refuses_every_spelling_of_git_and_bolt_on_every_system() {
    for n in [".git", ".GIT", "git~1", ".git.", ".bolt", ".BOLT", "bolt~1", ".bolt::$INDEX_ALLOCATION", ".g\u{200c}it"] {
        assert!(writable_here(n, &[]).is_err(), "{n:?}");
    }
}
#[test] fn a_real_short_name_of_git_is_refused() { assert!(writable_here("GI7EBA~1", &[(".git", "GI7EBA~1")]).is_err()); }
#[cfg(windows)] #[test] fn windows_refuses_separators_devices_and_trailing_dots() {
    for n in ["a\\b", "c:d", "con", "aux.txt", "COM\u{b9}", "LPT\u{b3}.log", "name.", "name "] { assert!(writable_here(n, &[]).is_err(), "{n:?}"); }
}
#[cfg(windows)] #[test] fn com0_and_lpt0_are_ordinary_names_on_windows() {
    // the follow-up: Microsoft's list doesn't name them; save and write them
    assert!(writable_here("COM0", &[]).is_ok() && writable_here("LPT0.txt", &[]).is_ok());
}
#[test] fn names_up_to_765_bytes_decode_and_fit_by_this_systems_rule() {
    let cjk = "語".repeat(200); // 600 bytes, 200 UTF-16 units
    assert!(validate_name(&cjk).is_ok());
    assert_eq!(fits_here(&cjk), cfg!(windows));
    assert!(validate_name(&"a".repeat(766)).is_err());
}
#[test] fn a_bolt_spelling_is_left_out_of_a_save_with_a_warning() { /* walk test: `bolt~1/x` and `.bolt./y` left out, named */ }
#[test] fn a_bolt_folder_spelled_by_the_drives_rule_is_skipped_silently_at_the_top() {}
#[test] fn a_path_over_4096_bytes_is_refused_by_save_and_by_readers() {}
```
- [ ] **Step 2:** Run. Expected: failures.
- [ ] **Step 3:** Implement, and add the new vector to `listing.json` and storage-format.md §13.
- [ ] **Step 4:** Run fmt, clippy, the tests and `cargo +nightly fuzz build`.
- [ ] **Step 5:** Commit: `Check names for writing in one place, allow Windows-length names, and limit paths`.

### Task 3: The walk through folder handles

Decisions 045 (the save side) and 048; the follow-up on handle-relative walking; security.md T20's known limit.

**Files:**
- Modify: `crates/bolt-vcs-core/src/snapshot/walk.rs` (the traversal), `snapshot.rs` (`store_file` reads through the folder's `Dir`), `safe_fs.rs` and `repo.rs` (`.bolt`'s fixed folders opened through `Dir`), `Cargo.toml` (drop `ignore` if nothing else uses it).

**Interfaces:**
- `walk` keeps its signature. `Child` gains `meta: Option<EntryMeta>` (kind, size, modified time, file ID), taken from the listing where the system gives it, else from `Dir::metadata`.
- The walk opens each folder from its parent's `Dir` (`open_dir`), never by path. It holds at most 128 folder handles; past that, a folder is opened again from the deepest held one, a name at a time.
- Files are read from their folder's `Dir` (`open_file`); the opened file's `file_meta` must match the scan's file ID, size and time, else `ChangedDuringSave`.
- A link's target read on Windows has each `\` turned into `/` before it is stored (decision 045); `a_link_git_wrote_keeps_its_target_when_saved` runs on Windows from now on, and the nightly job "Links Git for Windows writes keep their target when saved" is removed from `nightly.yml`.

- [ ] **Step 1: Write the failing tests:**
```rust
#[cfg(unix)] #[test] fn a_folder_swapped_for_a_link_mid_walk_is_refused_not_followed() {
    // a fault point between scanning `a/` and reading `a/x` swaps `a` for a link to a folder outside;
    // the save stops with ChangedDuringSave and stores nothing from outside
}
#[test] fn store_packs_swapped_for_a_link_after_the_shape_check_is_refused() {}
#[test] fn a_deep_tree_holds_at_most_256_folders_open() { /* 600 levels: the save works; a counter in Dir (test-only) sees ≤ 257 open */ }
#[cfg(windows)] #[test] fn link_targets_read_on_windows_are_stored_with_slashes() {}
```
Keep every existing walk test passing unchanged (ignore rules, kept entries, nested projects, `.git` names).
- [ ] **Step 2:** Run. Expected: the new tests fail.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run fmt, clippy and the whole suite, including the ignored real-depth tests on Windows (`cargo test -p bolt-vcs-core --all-features --lib -- --ignored real_folders`).
- [ ] **Step 5:** Update security.md T20 (the folder-swap limit is gone) and T19. Commit: `Walk the project through folder handles, and store link targets read on Windows with slashes`.

### Task 4: The two owner-check races

The follow-ups from milestone 1's red-team fixes, task 10.

**Files:** Modify `crates/bolt-vcs-core/src/repo.rs`; tests in `repo.rs` and `crates/bolt-vcs/tests/cli.rs`.

- **A project reached through a link:** `discover` resolves the starting folder (`std::fs::canonicalize`) before it looks for `.bolt` or asks an owner, so the owner check judges the folder `.bolt` is in, not the link.
- **A `.bolt` that vanishes and comes back:** a `.bolt` seen during the search whose owner lookup then says "not found" refuses the command ("the .bolt folder changed while Bolt looked at it; run the command again") instead of counting as no project.

- [ ] Steps: failing tests `the_owner_check_judges_the_resolved_folder` (Unix: the project reached through a symlink owned by root in a test that can make one; Windows: a junction) and `a_bolt_that_vanishes_during_the_owner_check_is_refused` (a fault point removes `.bolt` after it is found and puts it back after the lookup); implement; run; commit `Resolve the starting folder before the owner check, and refuse a .bolt that vanishes mid-check`.

### Task 5: Pack version 2, searched on disk, opened when needed

Decision 057.

**Files:**
- Modify: `docs/storage-format.md` §11 (bytes first), `crates/bolt-vcs-core/src/{pack.rs,store.rs,verify.rs}`, `crates/bolt-vcs-core/tests/vectors.rs`; create `tests/vectors/pack_v2.json`; `fuzz/fuzz_targets/open_pack.rs`.

**Interfaces:**
- **Bytes** (storage-format §11, version 2): header as version 1 with `pack version = 2`; entries as version 1; then the index:
  ```text
  u64       count (at most 2^24)
  u32 × 256 fan-out: entry i is how many IDs have a first byte ≤ i
  [32] × count   IDs, strictly ascending
  per ID, in the same order: u8 kind, u64 offset, u64 uncompressed size, u64 stored size
  ```
  then the trailer as version 1. The index is exactly `8 + 1024 + 57 × count` bytes; the fan-out must be non-decreasing and end at `count`.
- `pub const PACK_VERSION: u32 = 2;` written; `PackReader::open` accepts 1 (index read whole, as today) and 2 (header, trailer and fan-out read; lines read by position on demand).
- `PackReader::find(&self, id: &Id) -> Result<Option<IndexEntry>, PackError>`: binary search within the fan-out's range.
- `Store` lists packs at `open`, opens a pack when first searched, keeps at most 16 open (least recently used closed first), and searches first the pack that found the last object. A pack listed but gone when opened sends the store to list again once.

- [ ] **Step 1: Write the failing tests:**
```rust
#[test] fn pack_v2_matches_its_vector() { /* tests/vectors/pack_v2.json: two objects, exact bytes and IDs */ }
#[test] fn a_v2_index_line_is_found_by_position() {}
#[test] fn version_1_packs_still_read() { /* the milestone 1 vector */ }
#[test] fn a_planted_pack_claiming_2_pow_24_objects_costs_reads_not_memory() {
    // a sparse v2 pack whose index claims 2^24 entries: opening the store and a lookup stay under 64 MiB more memory (Linux, VmHWM)
}
#[test] fn at_most_16_packs_are_open_at_once() { /* 40 packs, 40 lookups */ }
#[test] fn a_fan_out_that_decreases_or_overshoots_is_refused() {}
```
- [ ] **Step 2:** Run. Expected: failures.
- [ ] **Step 3:** Implement the writer, the reader and the store.
- [ ] **Step 4:** Run fmt, clippy, the tests, and the fuzz target build.
- [ ] **Step 5:** Commit: `Write pack version 2, search its index on disk, and open packs only when needed`.

### Task 6: Merging small packs

Decision 057.

**Files:** Create `crates/bolt-vcs-core/src/merge.rs`; modify `repo.rs` (`commit` merges after `head` moves), `warning.rs` (`pack_not_merged`), `docs/json.md`.

**Interfaces:**
```rust
pub struct MergeReport { pub merged: usize, pub kept_damaged: Vec<PathBuf> }
pub(crate) fn merge_small_packs(repo: &mut Repository, lock: &ChangeLock) -> Result<Option<MergeReport>, RepoError>;
```
- Runs when the store holds more than 32 packs; takes every pack under 64 MiB, copies each object's stored bytes as they are after checking it (decompress, hash), writes one version 2 pack through `PackWriter`, makes it durable and renames it in, and only then removes the old packs. A pack with a copy that fails its check is left and named in `pack_not_merged`. Fault points `merge:written` and `merge:removing`.
- `commit` calls it after `head` moves; any error there is a warning, since the command has happened.

- [ ] Steps: failing tests `thirty_three_saves_leave_at_most_a_few_packs`, `a_merge_stopped_at_every_step_leaves_every_object_readable`, `a_damaged_pack_is_left_and_named`, `big_packs_are_not_merged`; implement; run; commit `Merge small packs once a store holds many`.

### Task 7: The cache key, and a change-ID cache

Decision 053's key; the follow-up on change IDs.

**Files:** Create `crates/bolt-vcs-core/src/{keyed.rs,changeids.rs}`; modify `history.rs` (`short_change_id`, `history`), `snapshot.rs`; `fuzz/fuzz_targets/change_ids.rs`.

**Interfaces:**
```rust
pub fn cache_key() -> Result<[u8; 32], RepoError>;   // keyed.rs: made on first use, atomically, in the user's settings folder; tests set BOLT_CONFIG_DIR
pub fn seal(key: &[u8; 32], body: &[u8]) -> Vec<u8>;            // body ‖ blake3::keyed_hash(key, body)
pub fn open_sealed<'a>(key: &[u8; 32], bytes: &'a [u8]) -> Option<&'a [u8]>;
pub struct ChangeIds { /* sorted change IDs of the saves on each line, and the diary entry they were read for */ }
pub fn short_change_id_cached(repo: &Repository, line: &str, id: &ChangeId) -> Result<String, RepoError>;
```
- `.bolt/cache/change-ids`, sealed; rebuilt when its diary entry isn't the head's ancestor or the seal fails. A save adds its own ID without reading its line; `history` reads its line once.

- [ ] Steps: failing tests `a_save_does_not_read_its_line_once_the_cache_is_built` (a counting store hook), `a_cache_from_another_account_is_ignored`, `an_edited_cache_is_ignored`, `short_ids_match_the_uncached_ones` (property test); implement; run; commit `Seal caches with a key of the user's own, and cache change IDs for shortening`.

### Task 8: The file-state cache

Decision 053.

**Files:** Create `crates/bolt-vcs-core/src/filecache.rs`; `fuzz/fuzz_targets/file_cache.rs`; storage-format.md §10 describes the file (not normative).

**Interfaces:**
```rust
pub struct FileState { pub kind: EntryKind, pub size: u64, pub modified_ns: i128, pub id: Id }
pub struct FileCache { /* by project path */ }
impl FileCache {
    pub fn load(repo: &Repository) -> FileCache;                                   // empty when missing, unsealed or damaged
    pub fn matches(&self, path: &str, meta: &EntryMeta) -> Option<&FileState>;     // kind, size and time equal, and not racy
    pub fn record(&mut self, path: &str, meta: &EntryMeta, id: Id);
    pub fn store(self, repo: &Repository, now_ns: i128) -> Result<(), RepoError>; // atomic; entries within 2 s of now marked racy
}
pub fn drive_now(repo: &Repository) -> Result<i128, RepoError>;                    // modified time of a file written in .bolt/tmp
```
- Encoding: `u8 version = 1`, `varint count`, then by path: `string path`, `u8 kind`, `varint size`, `i64 seconds`, `u32 nanoseconds`, `bool racy`, `id`; sealed with `keyed::seal`. Read with a 256 MiB cap.

- [ ] Steps: failing tests `a_matching_entry_is_trusted`, `a_file_changed_within_two_seconds_of_now_is_read_again`, `a_cache_sealed_by_another_key_is_ignored`, `a_truncated_or_edited_cache_is_ignored`, `size_or_time_or_kind_changes_miss`; implement; run, with the fuzz target; commit `Add the file-state cache, sealed, with Git's racy rule against the drive's own clock`.

### Task 9: `bolt status`

Decisions 011 and 053.

**Files:** Create `crates/bolt-vcs-core/src/status.rs`; modify `crates/bolt-vcs/src/{main.rs,output.rs}`, `docs/json.md`; create `crates/bolt-vcs/tests/status.rs`.

**Interfaces:**
```rust
pub struct Status { pub position: Position, pub changes: Vec<PathChange>, pub not_written_here: Vec<String>, pub unfinished_switch: Option<String>, pub warnings: Vec<Warning> }
pub struct PathChange { pub path: String, pub change: ChangeKind, pub kind: EntryKind, pub old_size: Option<u64>, pub new_size: Option<u64> }
pub fn status(repo: &Repository, full: bool) -> Result<Status, SaveError>;
```
- Walks as a save does (same rules, same kept entries), reads only files the cache doesn't vouch for (all with `full`), hashes them, compares with the current save, and writes the cache back. Never writes to the store. Lists entries this computer can't hold as not written here.
- CLI: `bolt status [--full]` prints the position, then `added`/`modified`/`removed` lines with paths escaped, then the not-written list; "Nothing changed since <short>." when clean. `--json` per json.md.

- [ ] Steps: failing tests `status_lists_added_modified_removed_and_kind_changes`, `status_does_not_open_unchanged_files` (Review Focus 4), `status_full_reads_everything`, `status_never_writes_to_the_store`, `entries_this_computer_cant_hold_are_listed_as_not_written_here`, `status_json_matches_its_snapshot`; implement; run; commit `Add bolt status`.

### Task 10: `save` uses the cache, and the speed bar

The follow-up on reading stored copies twice; decision 023.

**Files:** Modify `crates/bolt-vcs-core/src/snapshot.rs`; create `crates/bolt-vcs/tests/speed.rs` (ignored; run by hand and nightly); `testing.md` §5.

- A file the cache vouches for whose cached ID equals the base save's entry is taken from the base save, unread and not read back; every other file is read as today. The cache is written back after the save commits.
- `speed.rs`: `status_on_an_unchanged_100_000_file_project` builds the project, runs `status` twice, and prints the second run's time; nightly records it (decision 023's gate is checked against it before v0.1).

- [ ] Steps: failing tests `an_unchanged_file_is_not_read_by_save` (Unix: made unreadable after caching), `a_file_the_cache_misses_is_read_and_saved`; implement; run; measure once on this machine and record the figure in testing.md §5; commit `Let save skip files the cache vouches for, and measure status on 100,000 files`.

### Task 11: The journal

Decision 049; storage-format §10.

**Files:** Modify `docs/storage-format.md` §10 (bytes first); create `crates/bolt-vcs-core/src/journal.rs`, `tests/vectors/journal.json`, `fuzz/fuzz_targets/decode_journal.rs`.

**Interfaces:**
- **Bytes:**
  ```text
  u8      version = 1
  id      the diary entry the switch commits
  id      the diary entry head named when it began
  varint  action count (at most 10,000,000)
  actions, sorted by path:
    string  path (at most 4,096 bytes)
    u8      action (1 = write file, 2 = write executable, 3 = write link, 4 = make folder, 5 = remove file, 6 = remove folder)
    bool    has an expected current entry; if true: u8 kind, id, varint size
    if writing: id, varint size
  [32]    BLAKE3 of everything before it
  ```
- `pub struct Journal { pub target: Id, pub from: Id, pub actions: Vec<Action> }` with `encode`, `decode`, `write(repo)`, `read(repo) -> Result<Option<Journal>, RepoError>`, `remove(repo)`.

- [ ] Steps: failing tests `the_journal_matches_its_vectors`, `a_journal_with_a_wrong_checksum_or_unsorted_paths_is_refused`, `round_trips` (property test); implement; run, with the fuzz target; commit `Specify and add the switch journal`.

### Task 12: Writing a switch

Decisions 049 to 052 and 045 (the write side); security.md §4's write-out defences; the review of Git's published vulnerabilities.

**Files:** Create `crates/bolt-vcs-core/src/writeout.rs`; modify `warning.rs` (`not_written_here`, `placeholders_written`, `files_in_use`, `changed_since_plan`), `docs/json.md`; create `crates/bolt-vcs/tests/goto.rs` (helpers) and a holder helper for Windows tests.

**Interfaces:**
```rust
pub struct Plan { pub actions: Vec<journal::Action>, pub not_written: Vec<String>, pub placeholders: Vec<String>, pub collisions: Vec<String> }
// `remove_new`: discard removes new files; goto carries them along
pub fn plan(repo: &Repository, from: Id, to: Id, only: Option<&[String]>, remove_new: bool, drive: Drive) -> Result<Plan, SaveError>;
pub struct Applied { pub written: usize, pub removed: usize, pub in_use: Vec<String>, pub changed_since_plan: Vec<String>, pub warnings: Vec<Warning> }
pub fn apply(repo: &Repository, lock: &ChangeLock, journal: &journal::Journal) -> Result<Applied, RepoError>;
```
- **Plan:** compare the folder (walked as `status` walks it) with the current save `from` and the target `to`. An entry `writable_here` refuses, or one of two names that are one file on this drive, goes in `not_written` and is never touched. A link whose target leaves the project, judged from the target save's paths alone (absolute, a drive letter, `\\`, or `..` past the top), or that holds a `\` on Windows, is a placeholder. Unsaved work in the way goes in `collisions` (decision 050's rule).
- **Apply, in order:** folders top down; files (new contents from the store, streamed into a file made in `.bolt/tmp` through `Dir::create_new`, synced, then `rename_into` the target folder); executable bits; removals deepest first; on a drive that ignores case, a removal whose name equals a written name but for case is skipped; links last. Before replacing or removing anything, its current entry must equal the journal's expected one (kind, size, then content ID when size and time don't settle it), else it is left and listed in `changed_since_plan`. A Windows sharing violation is tried 10 times, 100 ms apart, then listed in `in_use`.
- **Links:** on Windows, `/` becomes `\`; a folder link when the target, resolved within `to`, is a folder; a link the system won't make becomes a placeholder file holding the target's bytes. Never read a target on disk.

- [ ] **Step 1: Write the failing tests** (`writeout.rs` and `crates/bolt-vcs/tests/goto.rs`):
```rust
#[test] fn a_planted_save_naming_git_or_bolt_spellings_writes_none_of_them() {}
#[test] fn a_link_planted_as_a_folder_name_cant_redirect_a_later_file() { /* save: link `a -> /tmp/x` and file `a/b`: neither escapes; CVE-2021-21300 shape */ }
#[test] fn a_case_only_rename_is_written_on_a_case_blind_drive() {}             // Review Focus 1
#[cfg(windows)] #[test] fn a_file_held_open_is_left_and_the_next_run_finishes() {} // Review Focus 2
#[cfg(target_os = "linux")] #[test] fn writing_a_big_file_uses_little_memory() {} // Review Focus 5: 300 MiB, peak < 100 MB more
#[test] fn twins_on_a_case_blind_drive_are_both_left_unwritten() {}
#[test] fn links_leaving_the_project_become_placeholders() {}
#[cfg(unix)] #[test] fn modes_are_exactly_0644_and_0755() {}
#[test] fn a_file_changed_after_the_plan_is_left_alone() {}
#[cfg(windows)] #[test] fn a_stored_link_target_holding_a_backslash_is_not_written_on_windows() {}
```
- [ ] **Step 2:** Run. Expected: failures.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run fmt, clippy and the tests on all three systems.
- [ ] **Step 5:** Commit: `Plan and write a switch one file at a time, never through a link or onto a name the system misreads`.

### Task 13: Finishing an interrupted switch

Decision 049.

**Files:** Modify `crates/bolt-vcs-core/src/{writeout.rs,repo.rs,snapshot.rs,status.rs}`, `warning.rs` (`switch_unfinished`, `switch_finished`).

**Interfaces:**
```rust
pub fn finish_interrupted(repo: &mut Repository, lock: &ChangeLock) -> Result<Option<Applied>, RepoError>;
```
- Called by `save`, `goto` and `discard` right after they lock. A journal whose target `head` already names is removed. Otherwise the actions are applied again (each re-checked, so finished ones are skipped), `head` moves, the journal goes, and a `switch_finished` warning says what was finished. Read-only commands give `switch_unfinished` when a journal is there.

- [ ] Steps: failing tests `a_switch_stopped_at_every_write_is_finished_by_the_next_command` (Review Focus 3: fault points at every step, every file wholly old or new, then a `save` finishes it), `a_journal_whose_target_is_the_head_is_only_removed`, `read_only_commands_say_a_switch_is_unfinished`; implement; run; commit `Finish an interrupted switch before the next change`.

### Task 14: `bolt goto`

Decisions 050 and 055.

**Files:** Create `crates/bolt-vcs-core/src/{switch.rs,lines.rs}`; modify `crates/bolt-vcs/src/{main.rs,output.rs}`, `docs/json.md`; tests in `crates/bolt-vcs/tests/goto.rs`.

**Interfaces:**
```rust
pub enum Target { Line(String), ServerLine { server: String, line: String, save: Id }, Save(Id) }
pub fn resolve_target(repo: &Repository, text: &str) -> Result<Target, ResolveError>;   // lines.rs: line, line@server, change ID, fingerprint
pub struct GotoOutcome { pub position: Position, pub save: Id, pub applied: Applied, pub safety_copies: usize, pub carried: usize }
pub enum SwitchError { Collisions { paths: Vec<String> }, Resolve(ResolveError), Save(SaveError), Repo(RepoError) }   // with hint()
pub fn goto(repo: &mut Repository, target: &Target, who: &Person, command: &str, on_wait: &mut dyn FnMut(&str)) -> Result<GotoOutcome, SwitchError>;
```
- Lock, finish an interrupted switch, plan; on collisions stop with `SwitchError::Collisions { paths }` (hint: save or discard them first). Otherwise store safety copies (decision 050), the diary entry (new position; `Action::Command`; safety copies), write the journal, apply, move `head`, remove the journal. Unsaved work that doesn't collide stays in the folder. On a line: "You're on line <line>." At a save: "You're at save <short>, not on a line. Saving here asks for a new line's name." plus, at a line's newest save, "To keep saving on <line>: bolt goto <line>".
- CLI `bolt goto <target>`; `--json` per json.md: `position`, `save`, `written`, `removed`, `not_written`, `placeholders`, `files_in_use`, `safety_copies`, `warnings`.

- [ ] Steps: failing tests `goto_a_line_and_back_restores_every_file_byte_for_byte`, `goto_carries_unsaved_changes_that_dont_collide`, `goto_stops_at_colliding_changes_and_changes_nothing`, `goto_a_save_at_a_lines_tip_is_at_the_save`, `goto_a_server_line_is_at_its_save`, `an_ambiguous_target_lists_the_saves`, `goto_records_safety_copies_in_its_diary_entry`, `goto_json_matches_its_snapshot`; implement; run; commit `Add bolt goto`.

### Task 15: `bolt discard`

Decision 050.

**Files:** Modify `crates/bolt-vcs-core/src/switch.rs`, `crates/bolt-vcs/src/{main.rs,output.rs}`, `docs/json.md`; create `crates/bolt-vcs/tests/discard.rs`.

**Interfaces:**
```rust
pub struct DiscardOutcome { pub restored: Vec<String>, pub removed: Vec<String>, pub applied: Applied, pub safety_copies: usize }
pub fn discard(repo: &mut Repository, paths: Option<&[String]>, who: &Person, command: &str, on_wait: &mut dyn FnMut(&str)) -> Result<DiscardOutcome, SwitchError>;
```
- A switch from the current save to itself, limited to `paths` (project paths, the CLI turns arguments relative to the current folder into them; a folder means everything in it), that also removes new files. Ignored files the save doesn't hold, kept entries, project links and nested projects are never touched. Prints "Threw away your unsaved changes in N files (a safety copy of each was kept)."; nothing to discard says so.

- [ ] Steps: failing tests `discard_restores_changed_and_deleted_files_and_removes_new_ones`, `discard_leaves_ignored_files_alone`, `discard_of_a_path_touches_nothing_else`, `discard_keeps_a_safety_copy_of_everything_it_throws_away`, `discard_with_nothing_to_discard_says_so`; implement; run; commit `Add bolt discard`.

### Task 16: Saving away from a line, and `bolt line`

Decision 055.

**Files:** Modify `crates/bolt-vcs-core/src/{lines.rs,snapshot.rs}` (`SaveRequest::new_line: Option<&str>`), `crates/bolt-vcs/src/main.rs` (`save --line`, `line`), `docs/json.md`; create `crates/bolt-vcs/tests/lines.rs`.

- A save at a save needs `new_line`: in a terminal the CLI asks "Name a new line for this save:"; without one it stops naming `--line <name>`. A name that breaks decision 012's rules, or is a line's already in any letter case, is refused. The new line points at the new save, and the project is on it; the save records it in its `line` field. On a line, `--line` is refused ("making a line from here comes in v0.2").
- `bolt line` lists lines in name order, `*` on the current one, then server lines as `line@server`; `--json`: `current`, `lines`, `server_lines`.

- [ ] Steps: failing tests `saving_at_a_save_without_a_terminal_names_the_flag`, `save_with_line_makes_the_line_and_moves_onto_it`, `a_line_name_taken_in_another_case_is_refused`, `line_lists_lines_and_server_lines`, `line_is_refused_on_a_line`; implement; run; commit `Save away from a line into a new one, and list lines`.

### Task 17: Partial saves

Decision 054.

**Files:** Modify `crates/bolt-vcs-core/src/snapshot.rs` (`SaveRequest::only: Option<&[String]>`), `crates/bolt-vcs/src/main.rs` (paths after the message), `docs/json.md` (`paths`); tests in `snapshot.rs` and `cli.rs`.

- Project paths and patterns; a pattern (holding `*`, `?` or `[`) is matched with `ignore::wildmatch` (Git's rules, `**` included) against project paths below the current folder. The new save takes the folder's version of every matched path, deletions included, and the base save's version of everything else.
- Refused, saving nothing: an argument that matches nothing in the folder or the base save, a path outside the project, a new file an ignore rule keeps out named outright (hint: change the rule to save it).

- [ ] Steps: failing tests `a_partial_save_takes_only_the_named_file`, `a_named_folder_takes_everything_in_it`, `a_pattern_takes_the_files_it_matches`, `a_named_deleted_file_is_saved_as_deleted`, `an_argument_matching_nothing_is_refused`, `an_ignored_new_file_named_outright_is_refused`, `paths_are_relative_to_the_current_folder`; implement; run; commit `Save only the named files, folders or patterns`.

### Task 18: The question before a large save

Decision 056.

**Files:** Modify `crates/bolt-vcs-core/src/snapshot.rs` (`save_folder` gains `confirm_large: &mut dyn FnMut(&LargeSave) -> bool`), `crates/bolt-vcs/src/main.rs` (`--yes`), `docs/json.md`.

**Interfaces:**
```rust
pub struct LargeSave { pub new_files: u64, pub new_bytes: u64, pub biggest_folders: Vec<(String, u64)> }   // three biggest by bytes
```
- Counted from the scan's sizes, before anything is read or stored: files not in the base save (after a partial save's selection). Over 100,000,000 bytes or 10,000 files: in a terminal "This save adds 2.3 GB in 1,234 new files, most in: ... Save them? [y/N]"; without one, the save stops with `SaveError::LargeSave(LargeSave)`, whose hint names `--yes`.

- [ ] Steps: failing tests `a_save_adding_more_than_10000_files_asks`, `a_save_adding_more_than_100_mb_asks`, `without_a_terminal_it_stops_and_names_yes`, `yes_skips_the_question`, `changed_files_dont_count`; implement; run; commit `Ask before a save that adds unusually much`.

### Task 19: `bolt changes`

Decision 058.

**Files:** Create `crates/bolt-vcs-core/src/changes.rs`, `crates/bolt-vcs/tests/changes.rs`; modify `crates/bolt-vcs-core/Cargo.toml` (`similar`, the version already in `Cargo.lock` if it serves), `crates/bolt-vcs/src/{main.rs,output.rs}`, `docs/json.md`.

**Interfaces:**
```rust
pub enum Lines { Binary, TooBig, Hunks { hunks: Vec<Hunk>, timed_out: bool } }
pub struct Hunk { pub old_start: u64, pub old_lines: u64, pub new_start: u64, pub new_lines: u64, pub lines: Vec<(Tag, Vec<u8>)> }
pub fn file_lines(old: &[u8], new: &[u8], deadline: Duration) -> Lines;
pub fn unsaved_changes(repo: &Repository, paths: Option<&[String]>, each: &mut dyn FnMut(&str, &PathChange, Lines) -> Result<(), SaveError>) -> Result<(), SaveError>;
pub fn save_changes(repo: &Repository, save: Id, paths: Option<&[String]>, each: ...) -> Result<(), SaveError>;
```
- CLI `bolt changes [--save <save>] [paths…]`: the save is given with `--save`, so a path is never taken for one. Unified output, 3 lines of context, color only in a terminal, each line escaped. A file whose only change is CRLF to LF is said so in its header.

- [ ] Steps: failing tests `a_changed_line_shows_with_context`, `a_binary_file_shows_without_lines`, `a_file_over_64_mib_shows_without_lines`, `a_slow_diff_is_cut_at_its_deadline_and_said`, `control_characters_in_lines_are_escaped`, `changes_of_a_save_compare_with_its_first_parent`, `changes_json_matches_its_snapshot`; implement; run; commit `Add bolt changes, line by line`.

### Task 20: Hostile saves written out

Security.md §4 (T1 to T5) and §7; the write-out cases of the review of Git's published vulnerabilities.

**Files:** Create `crates/bolt-vcs/tests/corpus_writeout.rs`; modify security.md §6.

Each case plants a save (`test_support::plant_save`, written as a hostile `.bolt` would hold it), runs `bolt goto` onto it, and checks the exact result, nothing written outside the project or into `.git` or `.bolt`, and the message:
- `.git` spellings, `.bolt` spellings, `git~1` and a real hashed short name (Windows);
- `\` and drive letters in names (CVE-2019-1354, CVE-2019-1351), device names with superscript digits;
- a link named like a folder, then files below it (CVE-2021-21300, CVE-2024-32002), on a case-blind drive too;
- links to absolute paths, to `..` past the top, to UNC paths (Git for Windows' NTLM leak): placeholders, never looked at;
- modes 0644/0755 (CVE-2025-22620); no cache of checked folders (gitoxide, 2026): a folder made in the middle of a switch is checked again;
- names over the system's limit, paths over 4,096 bytes, 4,096 levels deep.

- [ ] Steps: write the cases (each must fail on a deliberately broken write-out before it passes on the real one); run on all three systems; commit `Add hostile saves to the write-out corpus`.

### Task 21: Edits, saves and switches against a model

testing.md §3's property test.

**Files:** Create `crates/bolt-vcs/tests/model.rs`.

- A proptest of random actions (write, change, delete, rename a file; make or delete a folder; `save`; `goto` a random earlier save or line; `discard` all or a path; `status`) applied to Bolt and to an in-memory model of the folder and its saves. After each: `status` agrees with the model, every save restores byte for byte, and nothing the model holds is missing. 64 cases of 30 actions on every push; more nightly.

- [ ] Steps: write it; run; fix what it finds (each fix with its own test); commit `Check random edits, saves and switches against a model`.

### Task 22: Follow-ups due now, and the docs

**Files:** as each needs; the docs.

1. The project link decoder gets a fuzz target (`decode_project_link`), and the per-file test `Env` structs move onto the shared one (the follow-ups' low-priority items).
2. `json.md` documents every new document and warning; `testing.md` §3 and §4 the new tests, fuzz targets and the removed nightly link job; `security.md` every row the milestone changed (T1 to T5, T9, T11, T19, T20); `storage-format.md` §13 the new vectors; `design.md` the phase line; `milestones.md` milestone 3's steps ticked.
3. The follow-ups: strike what this milestone finished, with the date; add what it defers (naming the program that holds a file; a way to mark a file executable on Windows; compressing again when merging).

- [ ] Steps: make the changes; run fmt, clippy and the tests; commit `Bring the docs up to date with milestone 3`.

### Task 23: Red-team session

Decision 024.

- [ ] **Step 1:** Write `redteam/attack-surface-m3.md`: write-out (names, links, case and Unicode tricks, the journal, crashes), the walk and its handles, the caches (planted, racy), `goto`/`discard`/partial saves, pack version 2 and merging, `changes` on hostile files, output spoofing.
- [ ] **Step 2:** Start Docker Desktop if it isn't running. A fresh agent on the most capable model attacks a copy of the branch in `redteam/sandbox.sh up <copy>` (no network, working only in the copy and the container). Windows-only attacks become tests on a `redteam/m3` branch run by CI, deleted afterwards. Proof tests go in `crates/bolt-vcs/tests/redteam5.rs`, ignored until fixed; the report in `docs/redteam/<date>-milestone-3.md`.
- [ ] **Step 3:** Fix every proven finding, each with its proof test un-ignored, or rule on it in writing (a decision record). New threats join security.md §4; deferred items join the follow-ups.
- [ ] **Step 4:** If anything critical or high was proven, run a short second session on the fixes.
- [ ] **Step 5:** Commit each fix separately, with the finding's ID in the message.

### Task 24: Whole-branch review, the owner's review of the `unsafe` code, and merge

- [ ] **Step 1:** A fresh reviewer on the most capable model reads the whole branch against `main` for correctness, the engineering rules, security, and that every document in `docs/` matches the code and today's state.
- [ ] **Step 2:** Fix the findings, or rule on them in writing.
- [ ] **Step 3:** Copy the progress log's rulings into `docs/plans/2026-11-05-milestone-3-rulings.md`.
- [ ] **Step 4:** Run fmt, clippy and the tests locally, and push. Open the pull request `milestone-3` → `main`, with no Claude attribution.
- [ ] **Step 5:** Wait for CI on all three systems, `deny` and CodeQL. Check the Dependabot alerts (and code-scanning and secret-scanning ones where the repository has them).
- [ ] **Step 6:** The owner walks through Task 1's `unsafe` code line by line (decision 022), using the walkthrough. Then merge the pull request with a merge commit.
