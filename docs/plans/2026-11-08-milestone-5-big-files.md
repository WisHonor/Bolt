# Milestone 5 ("Big files") Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Big files are cut into chunks that versions share, read once and stored as they are read, with hashing and compression on several threads; `tidy` follows chunk lists; Git LFS repositories import; an import rebuilds each of Git's delta families once; the two speed gates are measured and Bolt is benchmarked against Git and Git LFS, results published; on Windows, macOS and Linux alike.

**Architecture:** A chunk list is stored in a pack under its file's own ID (kind 18), so a lookup of a file finds its whole copy or its chunk list, with no cache. `chunks.rs` holds the cutter and the chunk-list record; `ingest.rs` is the one place a file's bytes enter the store (one read: small files whole, big ones cut as read); `workers.rs` runs reading, hashing, checking and compressing on worker threads while one thread writes every pack in a fixed order. `Store::copy_content` rebuilds chunked files, checking each chunk and the whole. The import reads Git LFS files from `.git/lfs/objects`, and rebuilds a big file's whole delta family once, from its bottom.

**Tech Stack:** Rust 1.99.1 and the crates already in the workspace (`blake3` for the cutter's table and its extendable output, `sha2` for Git LFS's SHA-256, `zstd`, std threads). No new dependency. `hyperfine` is installed only on CI runners, by the benchmark workflow.

**Spec:** decisions [082](../decisions/082-how-big-files-are-cut-into-chunks.md) to [088](../decisions/088-benchmarks-and-the-speed-gates.md), with 005, 009, 020, 023, 028, 031, 044, 053, 057 and 077; [design.md](../design.md) §5.3, §5.7, §5.9, §5.16, §6 and §7; [storage-format.md](../storage-format.md) §6, §11 and §13 (vectors already in `crates/bolt-vcs-core/tests/vectors/`: `chunking.json`, `chunk_list.json`, `pack_chunked.json`); [security.md](../security.md) T3, T6, T7, T9, T11, T27 and T31; the milestone 5 rows of [the follow-ups](2026-11-01-milestone-1-follow-ups.md) (rows on Git LFS write-out, rebuilding Git's deltas, an import's disk use, compressing again, and `tidy` with chunk lists).

**Rulings made while building:** `2026-11-08-milestone-5-rulings.md`, kept as the build goes, verbatim.

## Global Constraints

- Everything in CLAUDE.md applies.
  - `#![forbid(unsafe_code)]` in the engine. **No task adds `unsafe`.** The number of `unsafe` blocks in `bolt-vcs-platform` must be the same at the end as at the start (Task 15 checks). A task that finds it needs new `unsafe` stops and asks: the milestone's pull request then stays open for the owner, with a walkthrough written as `docs/plans/2026-11-05-milestone-3-unsafe-walkthrough.md` is.
  - No `unwrap`, `expect` or `panic` outside tests. One place per rule: the cutting rule and the chunk-list record live only in `chunks.rs`; how bytes enter the store only in `ingest.rs`; the worker pool only in `workers.rs`; Git LFS's pointer rules only in `git/object.rs`. Spec before code: storage-format.md §6, §11 and §13 already say the bytes and give the vectors. Tests pass on Windows, macOS and Linux.
- Code comments are short and human: the why in a line or two, no process words, no task or decision numbers in code.
- Modules that parse bytes from disk keep `#![deny(clippy::indexing_slicing, clippy::arithmetic_side_effects)]` (`chunks.rs` among them); nothing recurses once per chunk, folder level or delta (walks are loops with explicit stacks).
- **Chunking changes no ID.** Folder listings, saves and diary entries keep their bytes; every existing vector stays byte for byte. A file's ID is the plain BLAKE3 of its whole contents, however it is stored.
- **Every read and write inside the project folder goes through `bolt_platform::Dir`** (decision 048), and every file Bolt reads in `.git` through the Git reader's own openers, which never follow a link.
- **Outputs are untrusted until checked:** a chunked file's bytes may reach a caller's writer before its whole-file check; every caller already treats `copy_content`'s output as untrusted until it returns `Ok` (write-out writes into `.bolt/tmp` and renames only after). Keep it so.
- **One writer.** Only one thread adds objects to packs, in a fixed order, and every pack fault point is met on that thread, so a command writes the same packs whatever the timing and crash tests stop at the same place.
- Untrusted text is escaped before printing (`text::escape`): paths, Git LFS object names, messages.
- Messages say what happened, why, and what to do next (decision 017); no questions without a terminal. `--json` documents follow `docs/json.md`, which each task that adds or changes a document updates in the same commit.
- **Values** (from the decisions):

  | What | Value |
  |---|---|
  | Files cut into chunks | 1,048,576 to 137,438,953,472 bytes (1 MiB to 128 GiB); bigger files stored whole as today (082) |
  | Chunk sizes | at least 16,384, about 65,536, at most 262,144 bytes; only the last chunk shorter than 16,384 (082) |
  | Masks | `MASK_S` = top 17 bits (`0xFFFF800000000000`) before 65,536; `MASK_L` = top 15 (`0xFFFE000000000000`) from there (082) |
  | The cutter's table | BLAKE3 derive-key, context `bolt-vcs 2026-11-08 chunk-cutter gear v1`, no key material, first 2,048 bytes of its extendable output as 256 little-endian `u64` (082) |
  | A chunk list in a pack | kind 18, ID = the file's ID, uncompressed size = the file's size, stored size = the record's length (at most 512 MiB), compression 0 (083) |
  | Chunk count | 1 to 8,388,608 per list; Bolt's cutter makes at most 8,388,097; a list is decoded as it is read, never held as stored bytes and decoded list at once (083) |
  | Merging and `tidy` copy chunk lists last | a list whose ID the pack being written holds as whole content is left out; a different list for the same ID starts a new pack; an identical one is left out (083) |
  | `.bolt/format` | `feature chunks`: written by `start` and `import`; added by every changing command right after the lock when missing, before it stores anything (083) |
  | Worker threads | `std::thread::available_parallelism()`, clamped to 1..=8 (084) |
  | Work handed to workers | each job counted from hand-out until the writer takes its result, at what it reads plus the most its result can be; no more handed out while that passes 64 MiB or 4 jobs a worker are out; workers never wait on the writer (084) |
  | A Git LFS pointer | at most 1,024 bytes, not empty, accepted by Git LFS's parser (085) |
  | A Git LFS object | `.git/lfs/objects/<oid[0..2]>/<oid[2..4]>/<oid>`, a regular file of the pointer's size (085) |
  | Missing objects listed | the first 20 (path, commit, size), then the count and total size (085) |
  | A family rebuilt once | the first deltified blob met that builds at least 1,048,576 bytes; its family's headers scanned once per pack, at most 8 bytes an entry (086) |
  | The memory gate | saving 10 GiB of incompressible bytes peaks at no more than 500,000,000 bytes (088) |
  | Compressing again | only if level 9 or 19 saves ≥ 10% of the code and uncompressed sets' stored bytes at ≤ 5× level 3's time (088) |

- **New warning kinds** (each in json.md §4 when its task adds it): `lfs_lookalikes` (Task 6).
- **Dependencies:** none new. Any other needs the owner's approval.
- **Local runs on this machine:** `export PATH="$HOME/.cargo/bin:$PATH"`, `export BOLT_CONFIG_DIR="$(mktemp -d)"`, build with `-j 2`, test with `-- --test-threads=2`. Run `cargo fmt` after every edit. Unix-only code can't be clippy-checked here: watch CI for Unix-only lints. Big test files go in a temporary folder, never in the repository, and are deleted afterwards; nothing loads a big file whole into memory.
- Git LFS is installed on this machine (git-lfs 3.7.0) and on GitHub's runners. Tests that need it run `git lfs install --local` in their own repository (never the user's or the global config, which tests keep empty) and skip, saying so, when `git lfs` isn't on the PATH.
- Push at task boundaries to `milestone-5`; CI runs on Windows, macOS and Linux; a task is done only once they pass. One commit per task plus fix commits, with no Claude attribution.

## Review Focus

1. **Chunking changes no ID.** Save a project holding files of 1 MiB, 3 MiB and 40 MiB with chunking turned off by the test hook, then the same project in another folder with it on. Expected: the same save ID, the same listings; only the packs differ. Test: Task 3 `chunking_changes_no_ids`.
2. **A planted chunk list never reaches the folder.** A `.bolt` whose save names a big file through a chunk list that lists another file's chunks (each chunk good). `bolt goto` onto it. Expected: refused as damage naming the file; the folder is untouched; nothing half-written remains. Test: Task 9 `a_chunk_list_naming_another_files_chunks_is_never_written_out`.
3. **`tidy` never removes a chunk a kept file needs.** Two versions of a big file sharing most chunks; the old one only in an undone save older than the window; a pack with chunks of both. Expected: after `tidy`, the new version reads back; the old version's own chunks are gone. And with one pack made unreadable, `tidy` stops before removing anything. Tests: Task 5 `tidy_keeps_every_chunk_a_kept_file_needs` and `tidy_stops_at_a_pack_it_cannot_read`.
4. **A Git LFS repository cloned without `git lfs fetch --all`.** Expected: `bolt import` stops before `.bolt` exists, listing the missing objects (at most 20), their count and size, and the command that fetches them. Test: Task 6 `an_import_stops_before_writing_when_lfs_objects_are_missing`.
5. **A long history of a big file.** 50 versions of a file Git deltifies against each other. Expected: the import finishes inside the allowance, each object built once. Test: Task 8 `a_long_history_of_a_big_file_imports_within_the_allowance`.
6. **Saving a 10 GiB file.** Expected: under 500 MB peak. Test: Task 10 `saving_a_10_gib_file_stays_under_500_mb`.

---

## File structure

```text
crates/bolt-vcs-core/src/
  chunks.rs        the cutter and the chunk-list record (Task 1)
  pack.rs, pack/v2.rs   kind 18 entries: written raw under the file's ID, read and checked (Task 2)
  store.rs         copy_content through chunk lists; holds_chunk_list (Task 2)
  merge.rs, verify.rs   chunk lists checked by structure; small packs by stored size (Task 2)
  ingest.rs        one read: small files whole, big files cut as read (Task 3)
  workers.rs       worker threads, results taken in order by one writer (Task 4)
  snapshot.rs, snapshot/walk.rs   save and status through ingest and the workers (Tasks 3, 4)
  writeout.rs      safety copies through ingest (Task 3)
  tidy.rs          follows chunk lists; stops at a pack it can't read (Task 5)
  git/object.rs    Git LFS pointers parsed (oid, size, extensions), the exact form (Task 6)
  git/mod.rs, git/config.rs   opening a Git LFS object; lfs.storage refused (Task 6)
  import.rs, import/convert.rs   the first pass's Git LFS check; LFS files brought in; families rebuilt (Tasks 6, 8)
  snapshot/endings.rs   the first save after an import stops at pointer files (Task 7)
  git/odb.rs, git/pack.rs   the base cache keeps what it builds; family rebuilds (Task 8)
crates/bolt-vcs/src/main.rs, output.rs   import summary, Git LFS errors, the first-save stop (Tasks 6, 7)
crates/bolt-vcs/tests/
  chunks.rs        end to end: big files saved, edited, switched, shelved (Tasks 3, 4)
  import_lfs.rs    Git LFS imports, against git-lfs where installed (Task 6)
  corpus_chunks.rs hostile chunk lists (Task 9)
  speed.rs         the memory gate (Task 10)
crates/bolt-vcs-core/tests/throughput.rs   cutter, BLAKE3 and zstd speeds; compression levels (Task 11)
bench/             scripts that time Bolt, Git and Git LFS (Task 11)
.github/workflows/bench.yml, nightly.yml   the benchmarks and the memory gate (Tasks 10, 11)
fuzz/fuzz_targets/decode_chunk_list.rs, cut_chunks.rs   (Task 1)
docs/benchmarks.md   the published results (Tasks 10, 11)
```

---

### Task 1: Chunk lists and the cutter

Decisions 082 and 083; storage-format.md §6 and §13.

**Files:**
- Create: `crates/bolt-vcs-core/src/chunks.rs` (with its unit tests); `fuzz/fuzz_targets/decode_chunk_list.rs`, `fuzz/fuzz_targets/cut_chunks.rs`.
- Modify: `crates/bolt-vcs-core/src/lib.rs`; `crates/bolt-vcs-core/tests/vectors.rs` (read `chunking.json` and `chunk_list.json`); `fuzz/Cargo.toml`; `.github/workflows/nightly.yml` (the fuzz list).

**Interfaces:**
```rust
pub const CHUNK_FROM: u64 = 1 << 20;        // a file this long or longer is cut...
pub const CHUNK_UP_TO: u64 = 1 << 37;       // ...up to this long; a longer one is stored whole
pub const MIN_CHUNK: usize = 16 << 10;
pub const AVG_CHUNK: usize = 64 << 10;
pub const MAX_CHUNK: usize = 256 << 10;
pub const MAX_CHUNKS: u64 = 1 << 23;        // per chunk list, as the format allows
pub fn is_cut(size: u64) -> bool;           // CHUNK_FROM..=CHUNK_UP_TO
/// Length of the chunk that starts `data`, which holds every byte left or at least MAX_CHUNK.
pub fn chunk_len(data: &[u8]) -> usize;
/// Cuts what a reader gives into chunks, holding at most 2 × MAX_CHUNK bytes.
pub struct Cutter<R: Read> { /* … */ }
impl<R: Read> Cutter<R> {
    pub fn new(reader: R) -> Self;
    pub fn next_chunk(&mut self) -> io::Result<Option<&[u8]>>;   // retries Interrupted
    pub fn into_inner(self) -> R;
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)] pub struct Chunk { pub id: Id, pub size: u64 }
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChunkList { pub file: Id, pub size: u64, pub chunks: Vec<Chunk> }
impl ChunkList {
    pub fn encode(&self) -> Vec<u8>;
    pub fn decode(bytes: &[u8]) -> Result<ChunkList, DecodeError>;   // §6's rules exactly
    /// The same rules, decoding `len` bytes as they are read, never holding them whole.
    pub fn read_from(reader: &mut dyn Read, len: u64) -> Result<ChunkList, DecodeError>;
}
```
- The table is computed once (`std::sync::OnceLock`) from `blake3::Hasher::new_derive_key(…).finalize_xof()`.
- `decode` refuses: a version other than 1; a count of 0, over `MAX_CHUNKS`, or over what the bytes left can hold at 33 bytes a chunk (checked before allocating); a chunk size of 0; sizes that overflow or don't add up to the file size; a non-minimal varint; bytes after the last chunk. It reuses `encoding.rs`'s reader.

- [ ] **Step 1: Write the failing tests:**
```rust
#[test] fn the_cutters_table_matches_the_spec() { /* GEAR[0], [1], [255] from chunking.json */ }
#[test] fn the_cutter_cuts_the_vector_inputs_where_the_spec_says() { /* all four inputs of chunking.json */ }
#[test] fn one_inserted_byte_changes_one_chunk() { /* noise-3mib vs its insert: 11 + 1 + 28 */ }
#[test] fn the_cutter_cuts_the_same_whatever_the_reads() { /* a reader giving 1, 7, 4096 and random byte counts per read */ }
#[test] fn every_chunk_but_the_last_is_longer_than_the_minimum() { /* proptest: random data and lengths */ }
#[test] fn chunk_lists_match_their_vectors() { /* chunk_list.json valid: encode == hex, decode == value */ }
#[test] fn chunk_lists_that_break_a_rule_are_refused() { /* chunk_list.json invalid */ }
#[test] fn a_count_past_the_bytes_is_refused_before_anything_is_allocated() {}
#[test] fn decode_then_encode_gives_the_same_bytes() { /* proptest */ }
#[test] fn read_from_agrees_with_decode_on_every_vector() { /* valid and invalid, through a reader giving odd read sizes */ }
```
- [ ] **Step 2:** Run them. Expected: FAIL (no `chunks` module).
- [ ] **Step 3:** Implement. Fuzz targets: `decode_chunk_list` (decodes, and re-encodes what decodes to the same bytes) and `cut_chunks` (cuts random data; every length within the rules; lengths add up; cutting through a `Cutter` with odd read sizes gives the same lengths).
- [ ] **Step 4:** Run `cargo test -p bolt-vcs-core -j 2 -- --test-threads=2 chunks` and `--test vectors`, then the workspace. Expected: PASS. Build the fuzz targets (`cargo +nightly fuzz build` only where nightly is installed; otherwise CI's nightly job builds them).
- [ ] **Step 5:** Commit: `Cut big files into chunks, and read and write chunk lists`.

### Task 2: Chunked files in packs and the store

Decision 083; storage-format.md §6 and §11 (kind 18, reading, merging).

**Files:**
- Modify: `pack.rs` (kind 18 entries; `prepare`/`add_prepared` for bytes compressed elsewhere; the index's size limits per kind, v1 and v2), `pack/v2.rs`, `store.rs`, `merge.rs` (`holds_little`, `copy_pack`'s check and order), `verify.rs`, `repo.rs` (`feature chunks`), `crates/bolt-vcs-core/tests/vectors.rs` (`pack_chunked.json`).

**Interfaces:**
```rust
// pack.rs
pub struct Prepared { pub id: Id, pub size: u64, pub zstd: bool, pub stored: Vec<u8> }
pub fn prepare(id: Id, bytes: &[u8]) -> Result<Prepared, PackError>;   // compress-or-raw, add_whole's rule
impl PackWriter / PackSet {
    pub fn add_prepared(&mut self, kind: ObjectKind, p: &Prepared) -> Result<(), PackError>;
    pub fn add_chunk_list(&mut self, list: &ChunkList) -> Result<(), PackError>;  // kind 18, raw, under list.file
}
impl PackReader { pub fn read_chunk_list(&self, entry: &IndexEntry) -> Result<ChunkList, PackError>; }
// store.rs
impl Store {
    pub fn copy_content(&self, id: &Id, out: &mut dyn Write) -> Result<(), StoreError>;  // now also through chunk lists
    /// Whether a chunk list stored under `list.file` is exactly `list`.
    pub fn holds_chunk_list(&self, list: &ChunkList) -> Result<bool, StoreError>;
}
```
- An index line of kind 18 may record an uncompressed size up to 2^40 and a stored size up to 512 MiB; every other kind keeps its limits. `read_chunk_list` refuses compression other than 0, and a record whose file or size isn't the entry's (damage, so the copy is passed over).
- `copy_content`: copies of kind 1 and kind 18 are both copies of the file, in today's order and with today's rule for several copies (each checked before one is streamed). A chunk list streams its chunks in order, each looked up as kind 1 only (a kind-18 copy under a chunk's ID is passed over), each checked against its ID and its size in the list, through the existing per-object path (so a chunk's bad copy is passed over for a good one, and the pack is reported as damaged). The whole is hashed as it goes; a hash that isn't the file's ID makes this copy damage and the call an error, with the pack named.
- `has_good_copy` is unchanged for whole copies, and never counts a chunk list as good: a writer proves one by cutting the file (Task 3).
- `read_chunk_list` decodes the list as it reads it (`ChunkList::read_from`), never holding the stored bytes and the decoded list at once.
- Merging checks a chunk list with `read_chunk_list` (structure only) and copies its stored bytes; `holds_little` counts it at its stored size. `copy_pack` copies every other object first and chunk lists last: a list whose ID the pack being written already holds as content is left out, a list whose ID it already holds as a different list starts a new pack (so both survive), an identical list is left out. `tidy` (Task 5) shares `copy_pack`, so it gets the same order.
- `feature chunks`: `bolt start` and `bolt import` write it in `.bolt/format`; `Repository::lock_for_change` adds it, when missing, right after taking the lock and before anything is stored (`format` replaced whole through `tmp/` and a rename, as today); a reader accepts it. Read-only commands never write `format`.
- `verify_store` checks every chunk list's structure and that its file rebuilds to its ID.

- [ ] **Step 1: Write the failing tests** (`pack.rs`, `store.rs`, `merge.rs`, `verify.rs` unit tests; `vectors.rs`):
```rust
#[test] fn the_writer_writes_the_chunked_pack_vector_byte_for_byte() { /* pack_chunked.json */ }
#[test] fn a_chunked_file_reads_back_from_one_pack_or_several() {}
#[test] fn a_chunk_with_a_bad_copy_is_read_from_its_good_one() { /* damaged_packs names the bad pack */ }
#[test] fn a_chunk_list_naming_a_missing_chunk_fails_the_read() {}
#[test] fn a_chunk_list_of_good_chunks_that_build_another_file_fails_the_read() {}
#[test] fn a_chunk_list_under_a_chunks_id_is_passed_over() { /* no nesting */ }
#[test] fn a_chunk_list_that_names_another_file_or_size_is_damage() {}
#[test] fn a_compressed_chunk_list_is_damage() {}
#[test] fn a_whole_copy_and_a_chunked_copy_are_both_copies() {}
#[test] fn has_good_copy_never_counts_a_chunk_list() {}
#[test] fn holds_chunk_list_matches_only_the_same_list() {}
#[test] fn merging_keeps_chunked_files_readable() {}
#[test] fn a_planted_list_under_a_chunks_id_never_takes_the_chunks_place_in_a_merge() { /* the planted pack sorts first by name */ }
#[test] fn a_list_never_takes_a_whole_copys_place_in_a_merge() {}
#[test] fn two_different_lists_for_one_file_both_survive_a_merge() {}
#[test] fn a_pack_holding_the_list_of_a_10_gib_file_still_counts_as_small() {}
#[test] fn verify_finds_a_chunk_list_that_builds_the_wrong_file() {}
#[test] fn start_and_import_write_feature_chunks() {}
#[test] fn a_changing_command_adds_feature_chunks_before_storing_anything() { /* an older repository's format */ }
#[test] fn a_command_that_only_reads_leaves_format_alone() {}
```
- [ ] **Step 2:** Run them. Expected: FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run `cargo test -p bolt-vcs-core -j 2 -- --test-threads=2` and the workspace (merge, tidy and pack crash tests included). Expected: PASS.
- [ ] **Step 5:** Commit: `Store chunk lists under their file's ID, and read chunked files back`.

### Task 3: One read for every writer

Decisions 083 and 084 (one read; the good-copy rule for chunk lists). Single-threaded in this task; Task 4 adds the workers.

**Files:**
- Create: `crates/bolt-vcs-core/src/ingest.rs` (with its tests); `crates/bolt-vcs/tests/chunks.rs`.
- Modify: `snapshot.rs` (`store_file` through `ingest`; `between_passes` becomes a hook before the after-read stamp check), `writeout.rs` (`copy_in` through `ingest`; an ID other than the planned one leaves the file alone, as `ContentChanged` does today), `import/convert.rs` (`put` and `stream` through `ingest`; `stream` reads once), `chunks.rs` (a test hook to lower `CHUNK_FROM`/`CHUNK_UP_TO`, under `test-support`).

**Interfaces:**
```rust
pub(crate) enum IngestError { Changed, Read(io::Error), Pack(PackError), Store(StoreError) }
/// Stores the `size` bytes `reader` gives, reading them once, and gives their ID. Fewer or more bytes
/// than `size` is `Changed`.
pub(crate) fn store_bytes(packs: &mut PackSet, store: &Store, reader: &mut dyn Read, size: u64) -> Result<Id, IngestError>;
```
- Under `CHUNK_FROM`: read exactly `size` bytes (then one more read must give 0), hash, and store unless `packs.contains` or `store.has_good_copy`.
- `is_cut(size)`: a `Cutter`; for each chunk, hash it, store it unless `packs.contains` or `store.has_good_copy(chunk, Content, len)`; hash the whole as it passes; at the end the total must be `size`; then the list is stored unless `packs.contains(file)` or `store.holds_chunk_list(&list)` (the chunks were just checked or stored, so an equal list is good). Never `has_good_copy` on the whole file: a whole copy from before chunking is left, and the new list is stored beside it.
- Over `CHUNK_UP_TO`, `store_bytes` isn't used: `store_file` and `convert.rs` keep today's two reads for such a file (hash, then stream it into one object unless the store holds it).
- Save: `ChangedDuringSave` when `store_bytes` says `Changed` or the stamps differ after the read (today's check). Chunks stored by then stay in the uncommitted packs.

- [ ] **Step 1: Write the failing tests:**
```rust
#[test] fn a_new_file_is_read_once() { /* a reader that counts bytes read */ }
#[test] fn a_big_file_is_stored_as_chunks_and_reads_back() {}
#[test] fn an_edited_big_file_adds_only_its_changed_chunks() { /* store grows by a few chunks and a list */ }
#[test] fn the_same_list_is_not_stored_twice() {}
#[test] fn a_whole_copy_from_before_chunking_gets_a_list_beside_it() {}
#[test] fn a_file_that_grows_or_shrinks_while_read_is_changed() {}
#[test] fn a_file_over_the_limit_is_stored_whole() { /* CHUNK_UP_TO lowered by the test hook */ }
// crates/bolt-vcs/tests/chunks.rs (end to end)
#[test] fn chunking_changes_no_ids() { /* Review Focus 1 */ }
#[test] fn big_files_go_to_and_come_back_byte_for_byte() { /* save, edit, save, goto both ways */ }
#[test] fn discard_and_shelf_keep_a_big_file_as_chunks() {}
#[test] fn a_big_file_changed_during_the_save_stops_it() {}
#[test] fn an_import_reads_each_big_blob_once() {}
#[test] fn a_save_stopped_between_chunks_leaves_the_project_as_it_was() { /* pack limit lowered so packs:rolled-over falls inside one file; BOLT_FAULT_AT sweep */ }
```
- [ ] **Step 2:** Run them. Expected: FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run the workspace, with `writeout_memory.rs` and `shelving_a_big_file_uses_little_memory` (Linux, on CI). Expected: PASS.
- [ ] **Step 5:** Commit: `Read each file once, storing big ones as chunks as they are read`.

### Task 4: Hashing and compressing on several threads

Decision 084.

**Files:**
- Create: `crates/bolt-vcs-core/src/workers.rs` (with its tests).
- Modify: `ingest.rs` (chunk jobs), `snapshot.rs` (save and status take files from workers), `writeout.rs` (safety copies), `import/convert.rs` (chunk jobs), `fsio.rs` (test hooks reach workers).

**Interfaces:**
```rust
/// Worker threads for one command: `available_parallelism()` clamped to 1..=8 (a test hook sets the count).
pub(crate) fn with_workers<R>(f: impl FnOnce(&Workers<'_>) -> R) -> R;
impl Workers<'_> {
    /// Runs `work` on each job on the workers and hands the results to `take` in the jobs' order.
    /// A job counts `cost(&job)` bytes (what it reads plus the most its result can be) from when it
    /// is handed out until its result is taken; none is handed out while that total passes `window`
    /// or 4 jobs a worker are out.
    pub fn in_order<J, T, E>(&self, jobs: impl IntoIterator<Item = J>, window: usize,
        cost: impl Fn(&J) -> usize, work: impl Fn(J) -> T + Sync,
        take: impl FnMut(T) -> Result<(), E>) -> Result<(), E> where J: Send, T: Send;
}
```
- **Workers never wait on the writer:** results go into a shared table, never a bounded channel, and only the writer decides when to hand out more, by the cost counted from hand-out (not by results alone, which would let jobs pile up unseen). A chunk job costs twice its length plus zstd's bound; a small file twice its size plus the bound. No worker touches a `PackSet`; workers may use the `Store` (it is safe to share).
- **Save:** after the walk, the files the cache doesn't vouch for are taken in `build_with`'s order. A small file is a worker job (read whole, hash, check the store, `pack::prepare`). The writer looks ahead over small files only up to the next big file; a big file is cut on the writer's thread while its chunks are hashed, checked and prepared on the workers, and the writer adds them in order. The first problem a save reports is the one it reports today.
- **Status:** every file the cache doesn't vouch for, big ones included, is hashed on a worker; results are taken in order.
- **Safety copies and the import:** their big files' chunks go to the workers.
- **Test hooks:** a thread's `fault::unreadable`, armed fault points and `open_point` settings are copied to the workers it starts; pack fault points are only ever met on the writer.

- [ ] **Step 1: Write the failing tests:**
```rust
#[test] fn results_come_back_in_the_jobs_order_whatever_the_timing() {}
#[test] fn no_more_than_the_window_is_handed_out_however_slow_the_workers() { /* slow workers, fast writer: in-flight cost never passes the window */ }
#[test] fn a_slow_writer_never_blocks_a_worker() {}
#[test] fn a_save_writes_the_same_packs_with_one_thread_or_eight() { /* identical pack names and bytes */ }
#[test] fn the_first_problem_is_the_same_with_one_thread_or_eight() {}
#[test] fn an_unchanged_file_is_still_not_read_by_save() { /* the hook reaches the workers */ }
#[test] fn status_hashes_many_files_on_several_threads() { /* same answers as one thread */ }
#[test] fn a_crash_sweep_of_a_threaded_save_stops_at_the_same_places() {}
```
- [ ] **Step 2:** Run them. Expected: FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run the workspace, the model test with 256 cases, and `status_on_an_unchanged_100_000_file_project` once in a release build (compare with testing.md §5's 1.3 s). Expected: PASS, and no slower.
- [ ] **Step 5:** Commit: `Hash, check and compress on worker threads, writing packs on one`.

### Task 5: `tidy` follows chunk lists

Decision 083; storage-format.md §11 "Tidying"; the follow-up on `tidy` and chunk lists.

**Files:** `tidy.rs`, `tidy/tests.rs`, `repo.rs` (the error and hint for a pack `tidy` can't read), `crates/bolt-vcs/tests/tidy.rs`, `docs/json.md` (if the error's document changes).

- First, every pack's index is read and checked against its checksum, young packs included; a pack that can't be opened, read or checked stops `tidy` (`RepoError::CantTidy` naming it, with a hint: check its permissions; if it is damaged for good, moving it out of `.bolt/store/packs` lets `tidy` run, losing what only it holds). A conflict record still stops it.
- The kind-18 entries seen are kept as a map from file ID to their packs. Marking a file (kind 1, 2 or 3 entry) also reads every chunk list stored under its ID (`read_chunk_list`) and marks each chunk; so does every chunk list in a young pack. A list that can't be read stops `tidy`, as any record it must keep.
- `pack_not_tidied` stays for a pack whose copy reads differently from its checked index.
- Every size `tidy` adds up from an index (the small-pack judgement, the estimate of what it frees) counts a chunk list at its stored size, not its file's (today `tidy.rs` counts every line's uncompressed size).
- Rewriting goes through `merge::copy_pack`, so chunk lists are copied last, as Task 2 made it.

- [ ] **Step 1: Write the failing tests:**
```rust
#[test] fn tidy_keeps_every_chunk_a_kept_file_needs() { /* Review Focus 3 */ }
#[test] fn tidy_removes_the_chunks_only_an_old_file_needed() {}
#[test] fn tidy_keeps_the_chunks_a_young_packs_list_names() {}
#[test] fn tidy_follows_every_list_stored_under_a_files_id() { /* two lists, different chunks */ }
#[test] fn tidy_stops_at_a_pack_it_cannot_read() { /* unopenable, index unreadable, checksum failing; young and old */ }
#[test] fn tidy_still_stops_at_a_conflict_record() {}
#[test] fn tidy_stopped_at_every_step_loses_no_chunk() { /* fault sweep with chunked files */ }
```
- [ ] **Step 2:** Run them. Expected: FAIL (today `tidy` stops at any chunk list, and leaves unreadable packs with a warning).
- [ ] **Step 3:** Implement; replace `tidy_stops_at_a_chunk_list_or_a_conflict_record`.
- [ ] **Step 4:** Run `cargo test -p bolt-vcs-core -j 2 -- --test-threads=2 tidy`, the `tidy` CLI tests and the workspace. Expected: PASS.
- [ ] **Step 5:** Commit: `Tidy follows chunk lists, and stops at a pack it can't read`.

### Task 6: Git LFS files come in

Decision 085; design.md §5.16; security.md T6 and T31.

**Files:**
- Create: `crates/bolt-vcs/tests/import_lfs.rs`.
- Modify: `git/object.rs` (a parser that returns the pointer; the exact-form check), `git/mod.rs` (`Repo::lfs_object`), `git/config.rs` (`lfs.storage`, only to refuse), `import.rs` (the first pass's check replaces `stop_at_lfs_in_head`; the errors; the counts), `import/convert.rs` (pointers stored from their objects; a memo by oid), `crates/bolt-vcs/src/main.rs`, `output.rs`, `docs/json.md`; the tests that used a pointer only to stop an import (`tests/import.rs`'s interrupted-import test needs another trigger), `corpus_git.rs`'s LFS case, `fuzz/fuzz_targets/git_objects.rs`.

**Interfaces:**
```rust
// git/object.rs
pub struct LfsPointer { pub oid: [u8; 32], pub size: u64, pub extensions: bool }
pub fn lfs_pointer(blob: &[u8]) -> Option<LfsPointer>;        // Git LFS's parser; today's is_lfs_pointer rules
pub fn looks_like_lfs_pointer(blob: &[u8]) -> bool;           // unchanged
pub fn is_exact_lfs_pointer(blob: &[u8], p: &LfsPointer) -> bool;   // the bytes Git LFS writes
// git/mod.rs
impl Repo { pub fn lfs_object(&self, p: &LfsPointer) -> Result<Option<File>, GitError>; }  // None: not there
```
- **The first pass** (only with a sign of Git LFS): walk the root tree of every commit in `order`, each distinct tree once (a set of Git IDs), within the import's limits; for each blob, its size from its header; each of at most 1,024 bytes is read (once, a set of Git IDs) and parsed. A pointer whose object is missing, or not a regular file of its size (a stat, through the reader's no-link opener), is noted (the first 20 with path and commit; all counted with their sizes). A pointer with extensions stops the import there. Any missing: `ImportError::LfsMissing { first, count, bytes }`, before `.bolt` exists, with the hint `git lfs fetch --all`, then `bolt import` again.
- **`lfs.storage`** in `.git/config`, or in the user's own Git settings (the files and plain includes `git/config.rs` already reads for decision 060): `ImportError::LfsStorageElsewhere`, naming where it is set, before anything is written.
- Sizes added up from pointers (the missing total, the summary's bytes) saturate rather than overflow: pointers are untrusted.
- **The second pass:** a blob of at most 1,024 bytes that parses is brought in from its object: opened with `lfs_object`, its length checked, streamed through `ingest::store_bytes` behind a reader that computes SHA-256 as it passes; a different SHA-256 is `ImportError::LfsMismatch` (path, commit, oid), nothing committed. A missing object here is `LfsMissing` with that one. Size 0: an empty file, no object read. A pointer with extensions: `ImportError::LfsExtension`. A memo from oid to Bolt ID skips objects already stored by this import.
- **Look-alikes** are stored as they are; an `lfs_lookalikes` warning names the first 5 (path, commit) and counts them all.
- **The summary** gains the Git LFS files brought in (count and bytes), the pointers not in Git LFS's exact form (count) and the commits that hold Git LFS files (count; an export can give back their Git IDs only by turning those files back into pointers), in text and in `--json`.

- [ ] **Step 1: Write the failing tests** (unit tests in `object.rs`, `import.rs`; end to end in `import_lfs.rs`, writing `.git/lfs/objects` by hand; the last test with real Git LFS):
```rust
#[test] fn pointers_parse_as_git_lfs_parses_them() { /* the spec's examples: valid, ext lines, wrong order, upper-case hex, no size, trailing bytes */ }
#[test] fn the_exact_form_is_what_git_lfs_writes() {}
#[test] fn an_import_brings_in_git_lfs_files_as_their_contents() {}
#[test] fn an_import_stops_before_writing_when_lfs_objects_are_missing() { /* Review Focus 4: no .bolt, 20 listed, count and size, hint */ }
#[test] fn without_a_sign_a_missing_object_stops_the_second_pass() {}
#[test] fn an_object_whose_sha256_differs_stops_the_import() {}
#[test] fn an_object_of_the_wrong_size_or_kind_is_missing() { /* a folder, a link, a shorter file */ }
#[test] fn a_link_inside_git_lfs_is_never_followed() {}
#[test] fn a_pointer_with_extensions_stops_the_import() {}
#[test] fn lfs_storage_in_the_repository_config_is_refused() {}
#[test] fn lfs_storage_in_the_users_own_git_settings_is_refused() {}
#[test] fn the_summary_counts_commits_holding_lfs_files() {}
#[test] fn a_lookalike_comes_in_as_it_is_with_a_warning() {}
#[test] fn a_pointer_of_size_zero_is_an_empty_file() {}
#[test] fn pointers_not_in_the_exact_form_are_counted() {}
#[test] fn versions_of_a_big_lfs_file_share_their_chunks() {}
#[test] fn an_lfs_repository_matches_what_git_lfs_checks_out() { /* git lfs track; commits; every imported file equals what was committed; skipped without git-lfs */ }
```
- [ ] **Step 2:** Run them. Expected: FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run `cargo test -p bolt-vcs --test import_lfs -j 2 -- --test-threads=2`, the import, corpus and differential tests, and the workspace. Expected: PASS.
- [ ] **Step 5:** Commit: `Import Git LFS files from the repository's own store`.

### Task 7: The first save after an import stops at pointer files

Decision 085; decision 039's stop.

**Files:** `snapshot/endings.rs` (beside the line-ending check, the same "first save after an import" condition), `snapshot.rs` (`SaveError::LfsPointersInFolder { paths, count }`), `crates/bolt-vcs/src/output.rs`, `docs/json.md`, `crates/bolt-vcs/tests/import_lfs.rs`.

- A file stops it when it differs from the imported version, it is at most 1,024 bytes, `lfs_pointer` accepts it, and its size line is the imported version's size. The message names the first 5, counts them, and says to run `git lfs checkout`, then `bolt save` again. No flag.

- [ ] **Step 1: Write the failing tests:**
```rust
#[test] fn the_first_save_after_an_import_stops_at_lfs_pointers_in_the_folder() {}
#[test] fn a_pointer_of_another_size_is_saved_as_a_change() {}
#[test] fn only_the_first_save_after_an_import_looks() {}
#[test] fn after_git_lfs_checkout_the_save_goes_through() { /* with real Git LFS; skipped without it */ }
```
- [ ] **Step 2:** Run them. Expected: FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run the import and save tests, and the workspace. Expected: PASS.
- [ ] **Step 5:** Commit: `Stop the first save after an import at Git LFS pointer files`.

### Task 8: An import rebuilds each delta family once

Decision 086; design.md §5.16; security.md T7 and T11; the follow-up on rebuilding Git's deltas.

**Files:** `git/odb.rs` (`BaseCache`: a use leaves a base in place and refreshes it; `unpack` puts what it builds in; the family rebuild), `git/pack.rs` (the per-pack scan of entry headers, built lazily), `import/convert.rs` (the first need of a deltified blob of at least 1 MiB rebuilds its family, storing each blob through `ingest` and filling the Git-ID memo).

**Interfaces:**
```rust
impl Repo {
    /// Rebuilds every object of `id`'s delta family in its pack, each once, bottom up, depth first,
    /// handing each with its Git ID to `each`. Charges the work allowance as `unpack` does.
    pub fn rebuild_family(&self, id: &GitId,
        each: &mut dyn FnMut(&GitId, &[u8]) -> Result<(), ConvertError>) -> Result<(), ConvertError>;
}
```
- The scan reads each entry's header in pack order once and keeps its base's position (offset deltas) or index (ref deltas), at most 8 bytes an entry; children lists come from it.
- Every object built is checked against the name the pack's index gives it (Git ID), as every read is. A member whose delta can't be applied, or whose result isn't its name, is neither stored nor mapped (and not handed to `each`); one that was built still serves as the base of its children, each checked against its own name; the import stops on such a member only if a commit needs it, through today's path. (The red-team tests `rt3_8` and `rt4_4` build members under names that don't match and expect the import to succeed.)
- Memory: at most the base and the object being built, each at most 512 MiB, plus the base cache; an object with children still to come is kept in the cache while it fits, else written to a file in `.bolt/tmp` (through `fsio`, at most 4 GiB of such files at once, each removed after its last child) and read back whole for its next child; past 4 GiB it is rebuilt from the bottom when needed, counted against the allowance. `rt3_8` and `rt4_4` bound the import's peak at 750 MB for a 300 MiB file: keep the chunk window inside that margin.
- The work allowance and its numbers are unchanged; its stop's hint no longer needs the repack advice for honest histories.

- [ ] **Step 1: Write the failing tests:**
```rust
#[test] fn a_long_history_of_a_big_file_imports_within_the_allowance() {
    // Git makes 50 versions of an 8 MiB file deltified against each other (git repack -a -d -f --depth=50);
    // the allowance lowered by a test hook so the old quadratic work fails and once-each passes
}
#[test] fn each_object_of_a_family_is_built_once() { /* a build counter */ }
#[test] fn a_family_with_branches_keeps_memory_bounded() { /* a member too big for the cache goes to .bolt/tmp and is built once */ }
#[test] fn a_family_member_no_commit_needs_may_fail_its_check() { /* the import still succeeds */ }
#[test] fn the_base_cache_keeps_what_it_builds() {}
#[test] fn the_family_scan_runs_only_for_a_pack_it_needs() { /* pack_memory's planted pack still costs reads, not memory */ }
```
  and the existing hostile-chain tests (`rt3_4`, `rt3_4b`, `rt4_2`, `rt3_8`, `rt4_4`) keep passing.
- [ ] **Step 2:** Run them. Expected: FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run the Git reader, import and red-team tests, and the workspace; nightly's differential test on git/git runs after the push. Expected: PASS.
- [ ] **Step 5:** Commit: `Rebuild each of Git's delta families once`.

### Task 9: Proof: hostile chunk lists, write-out lessons, the model, crashes

testing.md §3; security.md T3 and T9; follow-up row on Git LFS write-out (CVE-2025-26625).

**Files:** create `crates/bolt-vcs/tests/corpus_chunks.rs`; modify `crates/bolt-vcs/tests/corpus_writeout.rs`, `crates/bolt-vcs/tests/model.rs`, `crates/bolt-vcs/tests/support/` (planting chunk lists).

- **Hostile chunk lists** (each planted in a `.bolt` a save names; then `goto`, `discard`, `status`, `changes`, `show`): a list naming another file's chunks (Review Focus 2); a list naming a chunk of another size; a missing chunk; a list stored under a chunk's ID; a list claiming a 10 GiB file for 3 small chunks; a count far past its bytes; a damaged list beside a good whole copy (the whole copy is read, the pack named). Each refuses or reads the good copy; nothing wrong reaches the folder; nothing outside the project changes.
- **Write-out lessons:** `a_hard_link_at_a_written_path_keeps_the_other_names_file` (the path holds a hard link to a file outside the project, with the content the save expects; `goto` writes the new version; the outside file is unchanged), `a_link_above_a_written_file_is_never_followed` (check the milestone 3 case covers a chunked file).
- **The model test:** files of 1 to 3 MiB among the random ones, edits that insert or overwrite inside them; after every action, every switch writes back byte-identical files.
- **Crashes:** a save, a discard and a shelf of big files stopped at every fault point (pack limit lowered) leave the project usable and every committed file readable.

- [ ] Steps: write the tests (each seen failing with its defence taken out, as milestone 3's corpus was), run them, run the model test with 256 cases locally (nightly runs 1,024), commit `Hostile chunk lists, Git LFS's write-out lessons, and big files in the model`.

### Task 10: The speed gates

Decision 088.

**Files:** `crates/bolt-vcs/tests/speed.rs`, `.github/workflows/nightly.yml` (a `memory-gate` job), `docs/benchmarks.md` (created here).

- `saving_a_10_gib_file_stays_under_500_mb` (ignored): writes 10 GiB from BLAKE3's extendable output into a temporary folder in 1 MiB writes (never whole in memory), `bolt start`, `bolt save --yes`, sampling the child's `VmHWM` (Linux; the helpers in `tests/support/`), fails above 500,000,000 bytes, prints the peak and the time, deletes everything.
- The nightly job frees runner disk first (removing preinstalled toolchains the job doesn't use), runs it in a release build, and greps that one test passed.
- On this laptop: the same through `bolt.exe`, its peak working set sampled every 200 ms with PowerShell, in `%TEMP%`, deleted afterwards; and `status_on_an_unchanged_100_000_file_project` once in a release build.
- `docs/benchmarks.md`: both gates, the machines, the commands, the numbers.

- [ ] Steps: write the test and the job; run it locally on Windows (sampled) and on Linux through CI (a `bench/m5` push, below); record the results; commit `Measure the two speed gates`.

### Task 11: Benchmarks against Git and Git LFS, and compressing again

Decisions 087 and 088; testing.md §5.

**Files:** create `bench/` (`run.sh`, `data.sh`, a short `README.md`), `.github/workflows/bench.yml` (on `workflow_dispatch` and on pushes to `bench/**` branches, Linux and Windows; installs `hyperfine` with `cargo install --locked`; runs the memory gate too), `crates/bolt-vcs-core/tests/throughput.rs` (ignored); modify `docs/benchmarks.md`.

- **Data:** a real code project at a fixed tag; 20 files of 50 MiB of noise; one 1 GiB file in 10 versions, each with 1 MiB overwritten and 4 KiB inserted at fixed offsets; Godot's demo projects at a fixed tag; a Git LFS repository the script builds (10 versions of five 100 MiB files); the red team's RT4-1 repository and a 200 MiB file in 50 versions changing 1% each (for import sizes, decision 087).
- **Steps timed** for Bolt, Git, and Git with Git LFS tracking files over 1 MiB: first save/commit; a small change; `status` with nothing changed; `status` after touching one big file; going back to the first version; size on disk; and `bolt import` of the Git LFS repository. `hyperfine --warmup 1 --runs 5` (3 for steps over a minute); export markdown.
- **Parts:** `throughput.rs` prints MB/s for the cutter, BLAKE3 and zstd levels 3, 9 and 19 on 64 KiB pieces, and, for compressing again, the bytes and time of each level on the code set, on uncompressed assets (generated WAV- and BMP-like data) and on compressed assets (the Godot demos' PNG and OGG files, given by an environment variable).
- **Compressing again:** apply decision 088's rule to the numbers; write the ruling with them. If a level qualifies, a decision record and storage-format.md change come first, then the code in `merge.rs` and `tidy.rs` (content only: a chunk list always stays uncompressed); otherwise nothing changes.
- **Publish** every number in `docs/benchmarks.md`, with the machines, versions and commands, including where Git or Git LFS wins.

- [ ] Steps: write the scripts and tests; push the branch's head to `bench/m5` to run the workflow (a manual workflow can't be started before it is on `main`); copy the results; run the micro tests locally; rule on compressing again; delete the `bench/m5` branch; commit `Benchmark Bolt against Git and Git LFS`.

### Task 12: The follow-ups

**Files:** the code each names, `docs/plans/2026-11-01-milestone-1-follow-ups.md`.

- Strike the milestone 5 rows done (Git LFS write-out, rebuilding Git's deltas, an import's disk use, compressing again, `tidy` with chunk lists), and the known limit on the diary-end search passing over an unreadable pack (`tidy` now stops there).
- Pull in the "Before v0.1" rows this milestone touched and that cost little now (each gets a test or a ruling): the store's checked-copy memory (measured with the memory gate's numbers), the planted-copy padding row (copies of chunks are at most 256 KiB).
- Add what this milestone defers (a list of lists for files over 128 GiB; `.gitattributes`-exact Git LFS; Bolt fetching Git LFS files; records as deltas for RT4-1; any benchmark finding).

- [ ] Steps: make the changes; commit `Milestone 5's follow-ups`.

### Task 13: Docs

**Files:** the docs.

1. `json.md`: every new document, field and warning (checked against the code); `testing.md` §3, §4 and §5 (the new tests, the fuzz targets, the bench workflow and memory gate, the micro tests in place of `criterion`); `security.md`: T3, T6, T7, T9, T11, T27 and T31 as built, and the milestone 5 sessions once Task 14 is done; `storage-format.md` §13 as built; `design.md`: the phase line; `milestones.md`: milestone 5's steps ticked.
2. Check each doc for CRLF (`tr -cd '\r' < file | wc -c` prints 0).

- [ ] Steps: make the changes; commit `Bring the docs up to date with milestone 5`.

### Task 14: Red-team sessions

Decision 024.

- [ ] **Step 1:** Write `redteam/attack-surface-m5.md`: chunk lists (planted, damaged, nested, lying sizes, under a chunk's ID, a 10 GiB claim), the cutter (inputs that make minimal chunks, a file near 128 GiB, memory), `copy_content` through lists and the untrusted-until-Ok output, merging and `tidy` with lists (removing a chunk still needed; a pack made unreadable mid-`tidy`), the worker threads (ordering, the window, a file changing while workers read it, test hooks), Git LFS import (pointers, look-alikes, extensions, links in `.git/lfs`, wrong sizes and hashes, `lfs.storage`, thousands of missing objects, memory of the first pass), the first-save stop, the family rebuild (hostile families: loops, ref-deltas, huge fan-out, deep chains, the allowance, memory), and Git LFS's CVE-2025-26625 against write-out.
- [ ] **Step 2:** Start Docker Desktop. A fresh agent on the most capable model attacks a copy of the branch in `redteam/sandbox.sh up <copy>` (no network; working only in the copy and the container). Windows-only attacks become tests on a `redteam/m5` branch run by CI, deleted afterwards. Proof tests go in `crates/bolt-vcs/tests/redteam8.rs`, ignored until fixed; the report in `docs/redteam/<date>-milestone-5.md`.
- [ ] **Step 3:** Fix every proven finding, each with its proof test un-ignored, or rule on it in a decision record. New threats join security.md §4; deferred items join the follow-ups.
- [ ] **Step 4:** Run a second session on the fixes, its report in `docs/redteam/<date>-milestone-5-session-2.md`. Shut Docker Desktop down afterwards (`Stop-Process` on `com.docker.*`, then `wsl --shutdown`).
- [ ] **Step 5:** Commit each fix separately, with the finding's ID in the message.

### Task 15: Whole-branch review and merge

- [ ] **Step 1:** A fresh reviewer on the most capable model reads the whole branch against `main` for correctness, the engineering rules, security, and that every document in `docs/` matches the code and today's state.
- [ ] **Step 2:** Fix the findings, or rule on them in writing.
- [ ] **Step 3:** Write `docs/plans/2026-11-08-milestone-5-rulings.md` from the build's rulings: a "please confirm" list first, then every ruling verbatim, then the deferred items.
- [ ] **Step 4:** Count `unsafe` in `bolt-vcs-platform` against `main`. Run fmt, clippy and the tests locally, push, and open the pull request `milestone-5` → `main`, with no Claude attribution.
- [ ] **Step 5:** Wait for CI on all three systems, `deny` and CodeQL; check the code-scanning, secret-scanning and Dependabot alerts.
- [ ] **Step 6:** No new `unsafe`: merge with a merge commit, then mark milestone 5 done in CLAUDE.md, `milestones.md` and `design.md` through a small pull request. New `unsafe`: leave the pull request open with a line-by-line walkthrough.
