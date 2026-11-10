# Milestone 6, speed and space: Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** On an everyday code project Bolt stays within twice Git's time for `status`, a small save and `goto` (the third v0.1 gate), on GitHub's Linux and Windows runners; the first save of big files that don't compress and going back to an old version of a big file stop losing to Git LFS and Git; every follow-ups row about speed, memory or space is done or ruled on; and every number is published, before and after, losses included.

**Architecture:** Nothing a user sees changes, and no stored ID or format changes. The work removes what the profile ([benchmarks.md](../benchmarks.md) §7) shows the walk doing for nothing: the ignore matcher tries every line against every name (an index finds plain-name lines by lookup); every folder is asked for four names before it is listed (the listing answers first, decision 101); the file cache is copied path by path, looked up twice a file and written and flushed on every scan (decision 100); a record costs seven reads; every folder's listing is rebuilt and hashed even when nothing in it changed. Big files are flushed while they are written (decision 102), and a switch reads each chunk whole. The gate is a job (`speed-gate`) that fails above twice Git's time (decision 099).

**Tech Stack:** Rust 1.99.1 and the crates already in the workspace (`blake3`, `zstd`, std threads). No new dependency. `hyperfine` is installed only on CI runners.

**Spec:** decisions [099](../decisions/099-a-third-speed-gate-within-twice-gits-time.md) to [105](../decisions/105-the-change-id-cache-keeps-each-lines-saves-in-order.md), with 006, 009, 011, 023, 028, 038, 046, 048, 049, 053, 057, 061, 067, 069, 083, 084 and 088; [design.md](../design.md) §5.7 and §5.9; [storage-format.md](../storage-format.md) §10 and §11; [security.md](../security.md) T9, T20, T25 and T32; [testing.md](../testing.md) §4 and §5; [benchmarks.md](../benchmarks.md) §1 and §7; the speed session's rows of [the follow-ups](2026-11-01-milestone-1-follow-ups.md).

**Rulings made while building:** `2026-11-10-milestone-6-speed-rulings.md`, kept as the build goes, verbatim.

## Global Constraints

- Everything in CLAUDE.md applies.
  - `#![forbid(unsafe_code)]` in the engine. **No task adds `unsafe`, and no task edits `crates/bolt-vcs-platform`.** A task that finds it needs either stops and reports: new `unsafe` leaves the milestone's pull request open for the owner.
  - No `unwrap`, `expect` or `panic` outside tests. One place per rule. Tests pass on Windows, macOS and Linux.
- Code comments are short and human: the why in a line or two, no process words, no task or decision numbers in code.
- **No safety rule is weakened.** These stay exactly as they are, and each task's review checks them:
  - the file cache's 2-second rule, read from the drive's own clock at the start of every scan, before any file is looked at (`filecache::drive_now`, which keeps its flush);
  - an entry is trusted only if it isn't racy and the file has the recorded kind, size, modified time, change time and file ID, and its number isn't 0;
  - every object read from the store is checked against its fingerprint, every chunk as it is read and the rebuilt file against its ID;
  - no link is followed at any name; a folder that holds a `.git` or a `.bolt` of its own is left out; `.git` and `.bolt` spellings are left out with their warning;
  - each file's details come from a look at the file itself, never from the folder listing;
  - a folder swapped between its listing and the look at its files is refused, and a file read must still be the file the walk saw;
  - everything a command's result rests on is flushed as before: packs, `state/head`, `state/journal` and its removal, `format`, each file a switch writes and the folders it changes.
- **No stored ID changes, and no existing project means anything else.** Folder listings, saves, diary entries, chunk lists and packs keep their bytes: a save writes byte for byte the packs it wrote before. `cache/files` stays version 3. Every existing test vector stays byte for byte.
- **Same outcome, less work.** Unless a task says otherwise, a command gives exactly the result, warnings and errors it gave before; a task that would change one stops and reports.
- **One writer.** Only one thread adds objects to packs, in a fixed order; crash tests stop at the same places.
- No `bolt.toml` setting and no preset rule is added here: the other session owns them. `--json` documents don't change.
- **Values** (from the decisions):

  | What | Value |
  |---|---|
  | The gate (099) | on Git's source at `v2.56.0`: `status` with nothing changed, a small change saved, back to the first version; Bolt's mean at most 2 times Git's; 10 timed runs after a warm-up; Linux and Windows runners, each its own check |
  | Caches (100) | `cache/files` and `cache/change-ids`: written through `tmp/` and a rename, never flushed (neither the file nor `cache/`) |
  | When `cache/files` is written (100) | only if the bytes to write differ from the bytes read |
  | `state/lock-info`, the drive test's rename (100) | not flushed |
  | A folder is asked for `.git` and `.bolt` by name (101) | only if a name in its listing (or a Windows short name) is `.git` or `.bolt` in any ASCII letter case, or any name holds a byte outside ASCII |
  | A folder's ignore file is opened (101) | only if a name in its listing is `.gitignore` (or `.boltignore`) in any ASCII letter case, or any name holds a byte outside ASCII |
  | A file flushed while written (102) | past 16 MiB (16,777,216 bytes), every further 16 MiB, by a helper thread through a second handle; then the whole file once more before the rename |
  | zstd level, the 95% rule, chunk sizes (103) | unchanged |

- **Local runs on this machine:** `export PATH="$HOME/.cargo/bin:$PATH"`, `export BOLT_CONFIG_DIR="$(mktemp -d)"`, build with `-j 1`, test with `-- --test-threads=2` (two sessions share the laptop, and memory is short). Run `cargo fmt` after every edit, and `cargo clippy -j 1 --workspace --all-targets -- -D warnings` before every commit. Unix-only code can't be clippy-checked here: watch CI. Big test files go in a temporary folder, never in the repository, and are deleted afterwards. Never run `git stash` (the stash is shared with the other session's folder). Work only in `C:\Users\elmas\Desktop\Bolt-speed`.
- Push at task boundaries to `milestone-6-speed`; CI runs on Windows, macOS and Linux; a task is done only once they pass. One commit per task plus fix commits, with no Claude attribution.
- **Measuring:** a push to a `gate/` branch runs the `speed-gate` job alone (Task 1). Numbers go into the ledger with their run.

## Review Focus

1. **A changed file is never missed.** Change a file's bytes keeping its size, with a new time, an old time, and the same time; move another file onto its path; write over it in place. `status` trusting the cache lists it wherever it did before these tasks. Tests: the cache's own (`filecache.rs`), the model test (`edits_saves_switches_shelves_and_undos_match_the_model`), `redteam5.rs`.
2. **A cache a crash tore is never trusted.** Cut `cache/files` short, zero it, mix two caches. Expected: ignored, rebuilt, the right status. Test: Task 3 `a_torn_file_cache_is_ignored_and_rebuilt`.
3. **An unchanged scan writes nothing.** Two `status` runs on an unchanged project. Expected: `cache/files` is the same file (same file ID, same bytes) after the second. Test: Task 3 `an_unchanged_status_leaves_the_file_cache_untouched`.
4. **A nested repository is still left out.** A folder below the top with a `.git` file, a `.git` folder, `.GIT`, a `.bolt` folder, alone and beside names outside ASCII. Expected: judged exactly as a lookup by name judges it on this drive. Tests: Task 4, and the existing nested-project tests unchanged.
5. **The matcher says what it said.** Random rule files and paths: the indexed matcher's last matching line is the linear scan's. Test: Task 2 `the_index_finds_the_line_a_scan_finds`; `ignore_vs_git.rs` unchanged.
6. **A listing taken from the last save is the listing a build would make.** For a folder that didn't change, the ID the new save's tree holds equals the ID of a listing the test builds by hand from the folder's files. Test: Task 7 `an_unchanged_folders_listing_is_the_one_a_build_would_make`; the vectors of storage-format.md §13 unchanged.
7. **A flush that fails is never lost.** The helper's flush fails while a big file is written. Expected: the command fails, nothing is renamed into place, `head` stays. Test: Task 9.
8. **Every read is still checked.** Flip a byte in a record, a chunk, a chunk list, an index line, on the new read paths. Expected: refused as damage, as before. Tests: the pack and store tests unchanged, Task 6's additions.

---

## Task 1: The gate's job

**Files:** modify `bench/run.sh`, `bench/README.md`, `.github/workflows/bench.yml`, `.github/workflows/nightly.yml`; create `.github/workflows/speed-gate.yml`.

**What:** `bench/run.sh gate <data folder> <results folder>` runs §2's steps on the code set alone, for Bolt and for Git, with 10 timed runs, then compares the means of three steps and fails when Bolt's is over twice Git's.

- [ ] **Step 1: `run.sh`.** The timed runs become `runs=${BENCH_RUNS:-5}` in `step` (still 3 when the warm-up took over a minute). Add the mode:

  ```sh
  gate)
  	# The third speed gate (decision 099): the code set alone, Bolt and Git, 10 timed runs.
  	work=$2 results=$3
  	export BENCH_RUNS=10
  	mkdir -p "$work" "$results"
  	[ -d "$work/code" ] || bash "$here/data.sh" code "$work/code"
  	find "$work/code" -type f -exec touch -d '1 hour ago' {} +
  	for t in bolt git; do steps code "$t"; done
  	rm -rf "${work:?}/code"
  	gate_check "$results"
  	;;
  ```

  and `gate_check`, which reads hyperfine's JSON:

  ```sh
  # gate_check <results>: Bolt's mean against Git's for the gate's three steps; fails over 2 times.
  gate_check() {
  	local failed=0 step bolt git
  	echo "== The gate: Bolt's time, Git's time, times Git's (at most 2)"
  	for step in status small-change back-to-first; do
  		bolt=$(jq -r '.results[0].mean' "$1"/*-code-bolt-"$step".json)
  		git=$(jq -r '.results[0].mean' "$1"/*-code-git-"$step".json)
  		awk -v s="$step" -v b="$bolt" -v g="$git" 'BEGIN {
  			r = b / g
  			printf "%-14s %8.1f ms %8.1f ms %5.2f %s\n", s, b * 1000, g * 1000, r, (r > 2 ? "OVER" : "ok")
  			exit (r > 2)
  		}' || failed=1
  	done
  	return "$failed"
  }
  ```

  Update the usage text and the header comment.
- [ ] **Step 2: `speed-gate.yml`,** modelled on `bench.yml`'s `against-git` job (same pinned actions, `permissions: contents: read`, `shell: bash`, the folder choice for Windows, the two "Describe the machine" steps, hyperfine 1.20.0 by `cargo install --locked`). Triggers: `workflow_call`, `workflow_dispatch`, and `push` to `gate/**`. One job `speed-gate`, a matrix of `ubuntu-latest` and `windows-latest` with `fail-fast: false`, `timeout-minutes: 45`. Steps: build `bolt` in release, print the versions (`git --version`, `hyperfine --version`, `jq --version`, the commit), run `bash bench/run.sh gate "$BENCH_DATA" "$BENCH_RESULTS"` with `target/release` on the PATH, then (always) `bash bench/run.sh summary "$BENCH_RESULTS"` and upload the results as `gate-${{ runner.os }}`.
- [ ] **Step 3:** `bench.yml` and `nightly.yml` each gain a job that calls it (`uses: ./.github/workflows/speed-gate.yml`). In `bench.yml` it carries the same `if` as `against-git`; in `nightly.yml` the condition the other nightly-only jobs carry.
- [ ] **Step 4:** `bench/README.md` says what `gate` does in two sentences.
- [ ] **Step 5: Check.** `bash -n bench/run.sh`. With `shellcheck` if it is installed (it isn't required). The job itself is checked by the controller, who pushes a `gate/` branch: expected today, it runs on both systems and FAILS, printing three ratios over 2 on Linux.
- [ ] **Step 6: Commit** `Add the gate against Git: bench/run.sh gate and the speed-gate job`.

## Task 2: Ignore lines that are plain names are found by lookup

**Files:** modify `crates/bolt-vcs-core/src/ignore.rs` only.

**Why:** `Rules::last_match` tries every line against every name, last line first. Git's own `.gitignore` has 263 lines, 232 of them plain names such as `/git-add`; that is 17.7 ms of a 63 ms `status`.

**What:** `Rules` keeps, beside `patterns`, an index built once in `parse`:

- `names: Vec<(u64, u32)>`: for every line with no `/` left and no wildcard (`any_depth && literal == text.len()`), a hash of its text with ASCII letters lowercased, and the line's position; sorted.
- `paths: Vec<(u64, u32)>`: for every line with a `/` and no wildcard (`!any_depth && literal == text.len()`), the same hash of its text without one leading `/`; sorted.
- `rest: Vec<u32>`: every other line's position, ascending.

`last_match` then finds the last line that matches without trying the lines that can't:

1. `fits(i)` is today's test of line `i`, unchanged: `(is_dir || !p.dir_only)` and `match_basename` or `match_pathname`.
2. Candidates among `names`: the lines whose hash is the hash of `name` lowercased, highest position first; the first that `fits` is the best so far. The same among `paths`, with the hash of `path` below `self.folder` (computed as `match_pathname` computes `name`: the folder and a `/` taken off the front, compared with `same`; when `path` isn't below the folder there are no candidates).
3. Then `rest` from its end, stopping at the first position below the best so far; the first that `fits` wins.

Every line `last_match` returns has passed `fits`, and a plain line can only fit a name or path whose lowercased bytes are its own, so the answer is the one the scan gave. The hash is `std::hash::DefaultHasher::new()` fed the lowercased bytes; a collision only costs one more `fits`.

- [ ] **Step 1: Write the failing test** `the_index_finds_the_line_a_scan_finds` in `ignore.rs`'s tests: a proptest over rule files (lines drawn from plain names, `/anchored/paths`, `dir/` lines, `!` lines, `*.ext`, `a*b`, `[ab]c`, with ASCII letters in both cases and a few bytes outside ASCII), a folder (`""`, `"a"`, `"a/b"`), paths below and beside it, `is_dir` and `casefold` both ways. It compares `Rules::last_match` with `last_match_by_scan`, the old one-line scan kept in the test module, by the position of the line returned (compare pointers with `std::ptr::eq`).
- [ ] **Step 2: Run it** (`cargo test -j 1 -p bolt-vcs-core ignore:: -- --test-threads=2`). Expected: it passes trivially before the change (both are the scan); it is the net for Step 3.
- [ ] **Step 3: Build the index and the new `last_match`.** Keep `#![deny(clippy::indexing_slicing, clippy::arithmetic_side_effects)]` if the module has it; use `get`.
- [ ] **Step 4: A test that the work is gone,** `plain_lines_are_not_tried_one_by_one`: 5,000 plain-name lines and 3 wildcard lines; a path none matches. A test-only counter (a `Cell<u64>` in a `thread_local!`, compiled only under `cfg(test)`) counts calls of `fits`; expected at most 3 plus the candidates (0 here).
- [ ] **Step 5: Unit tests by hand** for the orders that matter: a later wildcard line beats an earlier plain one and the other way round; `!name` after `*.ext`; a `dir/` plain line doesn't fit a file; `/a/b` in a nested folder's file fits only below that folder; `Makefile` against `MAKEFILE` with and without `casefold`.
- [ ] **Step 6: Run** the whole `ignore` module's tests, `ignore_vs_git` (`cargo test -j 1 -p bolt-vcs --test ignore_vs_git -- --test-threads=2`), and the fuzz crate's build if `cargo fuzz` is installed (it needn't be). Expected: all pass.
- [ ] **Step 7: Commit** `Find plain ignore lines by lookup instead of trying each one`.

## Task 3: Caches and flushes (decision 100)

**Files:** modify `crates/bolt-vcs-core/src/filecache.rs`, `fsio.rs`, `snapshot.rs` (`Reading`, `Storing`, `keep_file_cache`, `read_folder`, `save_folder`), `snapshot/endings.rs` (only if a signature it uses changes), `writeout.rs` (`Run.cache`), `drive.rs`, `repo.rs` (lock-info), and their tests.

**What, in the cache:**

- `FileCache` holds the body it was loaded from and an index over it, not a `BTreeMap<String, Entry>`: `body: Vec<u8>` and `slots: Vec<Slot>` in the file's order (paths ascending), where a `Slot` has the path's place in `body` (two `u32`s), its `FileState`, its `racy` flag, and `kept: bool`. Decoding checks everything it checked before (magic, version, count against the bytes left, paths in order and at most 4,096 bytes, kinds, nanoseconds, booleans, nothing left over) and allocates no string per entry.
- `matches(&self, path, meta) -> Option<&FileState>`: as today, by binary search over the slots, then among what this scan recorded.
- `vouch(&mut self, path, meta) -> Option<FileState>`: `matches`, and marks the entry as kept for the next write.
- `record(&mut self, path, meta, id)`: as today, into `added: BTreeMap<String, Entry>`; a path that can't be kept (its kind, a path too long, a time that doesn't fit) is dropped from both.
- `store(self, repo, now_ns)`: what is written is the kept slots and the recorded entries (a recorded one wins over a slot of the same path), each marked racy as today (`racy |= modified_ns > now_ns - 2 s`). **Nothing is written when that is byte for byte the body that was loaded:** first by the cheap test (every slot kept, nothing recorded, no slot's racy flag would change), otherwise by encoding and comparing. The 256 MiB cap stays.
- `encode`/`decode` stay public for the fuzz target and the vector; `decode(encode(x))` round-trips.
- `Reading` and `Storing` hold one `cache: FileCache` in place of `trusted` and `seen`: `vouched` asks `matches`; `file` calls `vouch` when the cache vouches, and `record` after a read, as it does today; `keep_file_cache` stores it. With `--full`, or without the drive's "now", the cache starts empty as today, and without "now" nothing is stored, as today.

**What, in the flushes:**

- `fsio::write_cache` writes the temp file and renames it into place, with the Windows retries, and flushes neither the file nor the folder.
- `state/lock-info` is written the same way, keeping its fault point (`write:before-rename`): a new `fsio::write_unflushed(tmp, to, name, bytes)` beside `write_atomic`.
- `drive::probe_once` renames its throwaway file with a plain rename (a new `pub(crate) fn rename_plain` in `fsio`, the existing private `rename` with its retries), not `rename_durable`.
- `filecache::drive_now` is not touched: it keeps `sync_all`.

- [ ] **Step 1: Write the failing tests.**
  - `filecache.rs`: `an_unchanged_cache_is_not_written_again` (load, vouch every entry, store with a later "now": the file's ID and bytes are the same; use `Dir::metadata` for the ID); `a_cache_is_written_when_an_entry_is_added_dropped_or_changed` (three cases); `an_entry_that_turns_racy_is_written` (store with a "now" earlier than an entry's time plus 2 s: it becomes racy, so the file changes) and `a_racy_entry_read_again_later_is_written`; `a_torn_file_cache_is_ignored_and_rebuilt` (cut the file to half, zero it, and join the first half of one cache to the second half of another: `load` gives an empty cache each time, and a store writes a good one).
  - `crates/bolt-vcs/tests/status.rs`: `an_unchanged_status_leaves_the_file_cache_untouched` (two `bolt status` runs 3 s after the save; `cache/files` has the same modified time, length and bytes after the second, and on Unix the same inode).
  - `fsio.rs`: `a_cache_write_flushes_nothing` and `lock_info_is_written_without_a_flush` need a way to see a flush. Add a test-only counter beside the fault layer (`fault.rs`'s feature `fault-injection` already wraps the write layer): count calls of `sync_all` and `Dir::sync` made through `fsio`, and route the three flushes this task removes, and the ones it keeps in `replace`, through two small `fsio` functions (`flush_file`, `flush_dir`) so the count is honest.
- [ ] **Step 2: Run them.** Expected: the "not written again", "flushes nothing" and status tests fail; the torn-cache test passes already (the seal).
- [ ] **Step 3: Rebuild `FileCache`** as above. Keep the module's `#![deny(clippy::indexing_slicing, clippy::arithmetic_side_effects)]`.
- [ ] **Step 4: Move `Reading` and `Storing` onto the one cache.** `endings::pointers` and write-out's `look` go on calling `matches`.
- [ ] **Step 5: The flushes** in `fsio.rs`, `repo.rs` and `drive.rs`.
- [ ] **Step 6: Run** `cargo test -j 1 -p bolt-vcs-core -- --test-threads=2` and `cargo test -j 1 -p bolt-vcs -- --test-threads=2` whole (the crash sweeps count fault points: they must still pass; if a sweep asserts a number of steps that the removed flushes change, the number is what changes, and the report says which). Expected: all pass.
- [ ] **Step 7: Commit** `Write caches without a flush, and the file cache only when it changed`.

## Task 4: The walk lists a folder before it looks inside (decision 101)

**Files:** modify `crates/bolt-vcs-core/src/snapshot/walk.rs`, `crates/bolt-vcs-core/src/names.rs` (the rule lives there), and their tests.

**What:**

- `names.rs` gains the one rule: `pub struct Listed { pub git: bool, pub bolt: bool, pub gitignore: bool, pub boltignore: bool }` and `pub fn listed_hints(names: impl Iterator<Item = (&OsStr, Option<&OsStr>)>) -> Listed` (each entry's name and its Windows short name). A field is true when some name or short name is that name in any ASCII letter case, **or any name or short name holds a byte outside ASCII** (then all four are true). Compare bytes (`as_encoded_bytes`, `eq_ignore_ascii_case`, `is_ascii`).
- In `WalkState::judged`, for a folder the walk opens (`opens`): after the alias check, open it (`self.handles.dir(rel)`, as today), then **list it** (`dir.entries()`), then:
  - `nested = (hints.git || hints.bolt) && holds_own_repository(dir)`. `holds_own_repository` itself doesn't change.
  - `self.enter(rel, None)` reads `.gitignore` only when `hints.gitignore` and `.boltignore` only when `hints.boltignore`; pass the hints in. An ignore file that is read is read exactly as today.
  - the listing is kept for `InFolder::listed`, which takes it instead of listing again (keep its sort). A folder that turns out nested, or that a problem stops, drops it.
- A listing that fails is `folder_problem(&shown(rel), e)`, where a failed open is today.
- The top folder is not changed: `WalkState::new` enters it and `walk_folders` lists it as today.
- Test-only counters (`cfg(test)`, thread-local) count the lookups `holds_own_repository` makes and the ignore-file opens `rules_in` tries, so tests can see a lookup that didn't happen.

- [ ] **Step 1: Write the failing tests** in `walk.rs`'s tests (they run on all three systems; where a drive's rules decide, the test asks the drive: `expected = dir.metadata(OsStr::new(".git")).is_ok()`):
  - `a_folder_of_plain_names_is_not_asked_for_git_bolt_or_ignore_files`: a subfolder with `a.txt`, `B.c`, `sub/`; after a save the counters for that folder are 0.
  - `a_nested_repository_is_found_from_the_listing` (`.git` as a folder, as a file; `.bolt` as a folder): left out with its warning, as today, and the lookup counter is above 0.
  - `git_in_another_letter_case_is_judged_as_the_drive_judges_it`: a subfolder holding `.GIT`: nested exactly when `metadata(".git")` succeeds there.
  - `a_name_outside_ascii_makes_the_walk_ask` : a subfolder holding only `é.txt`: the lookups are made (counter above 0) and the folder is saved.
  - `a_bolt_file_is_not_a_project`: `.bolt` as a file in a subfolder: not nested (today's rule), left out by the alias rule with its warning.
  - `an_ignore_file_is_read_when_the_listing_shows_it` and `an_ignore_file_in_another_letter_case_is_read_where_the_drive_would_open_it`.
  - `names.rs`: table tests for `listed_hints` (exact names, letter case, `.gitx`, `x.git`, a name outside ASCII, a short name `GIT~1` which is not `.git`, a short name that is).
- [ ] **Step 2: Run them.** Expected: the counter tests fail (the lookups are made today).
- [ ] **Step 3: Build it.**
- [ ] **Step 4: Run** `walk.rs`'s tests, `crates/bolt-vcs-core/tests/swapped_folder.rs`, `first_problem.rs`, `ignore_vs_git`, `corpus.rs`, and the whole core suite. Expected: all pass with no test changed, except where a test names which of two problems in one folder is reported first; such a change is reported, not made silently.
- [ ] **Step 5: Commit** `List a folder before asking it for .git, .bolt and ignore files`.

## Task 5: The walk handles each name once

**Files:** modify `crates/bolt-vcs-core/src/snapshot/walk.rs`, `crates/bolt-vcs-core/src/names.rs`, and tests. No behaviour changes: this task only removes repeated work, and its review is an equivalence review.

**What** (each from the profile's "rest: building paths, keeping the books", 9.5 ms on Linux, and "checking each name", 2.3 ms):

- `names::is_bolt_alias` and `is_git_alias` answer without allocating: walk the characters once, skipping the ones `is_ignorable` names, stopping where `ntfs_base` stops. Keep the old bodies in the test module and a proptest that old and new agree on random strings built from `.bolt`, `.git`, `bolt~1`, letter-case changes, ignorable characters, trailing dots and spaces, `:` and `\`.
- `walk_folders` builds an entry's project path once (`rel`, the folder's path, a `/`, the name) and hands `judged` that string, the folder's path and the name, instead of an absolute `PathBuf` that `judged` takes apart again (`strip_prefix`, three `shown` calls, `path_bytes`). A name that isn't UTF-8 (Unix) takes the old path, bytes and all, since the matcher must see its real bytes and `shown` would lose them.
- What `judged` asks about the folder an entry is in (`held`, `git_held`, whether anything in it is `untouched`) is asked once per folder, kept in `InFolder`, not once per entry by hashing the folder's path.
- A folder's children collect in its `InFolder` and go into `folders` once, when the walk leaves the folder, not by a map lookup per entry. A `Job` carries its folder's path once; a `Look` carries its place among the children and its name, and its path is built only for an error.
- After the walk, each job's looks are filled in with one map lookup per job. Children are sorted once; the second sort runs only for a folder that got a kept or returning entry added.
- `Listing` decoding and `validate_name` get faster by themselves through the alias functions; nothing else in them changes.

- [ ] **Step 1: The alias functions,** with the proptest first (it passes trivially until the bodies change).
- [ ] **Step 2: A net for the walk:** a test `the_walk_gives_the_same_scan` isn't possible against the old code, so the net is the existing suite. Before changing anything, run and record the names and count of passing tests in `walk.rs`, `snapshot.rs`, `first_problem.rs` and `swapped_folder.rs`; the same must pass afterwards.
- [ ] **Step 3: Restructure `walk_folders`, `judged`, `Look`, `Job` and the fill-in.** The order of every decision in `judged` stays as it is: untouched, held, the matcher's verdict, the alias check, the nested check, the ignore files.
- [ ] **Step 4: Run** the whole workspace's tests. Expected: all pass, none changed.
- [ ] **Step 5: Commit** `Build each name's path once in the walk, and check names without allocating`.

## Task 6: A record is found and read in fewer reads

**Files:** modify `crates/bolt-vcs-core/src/pack/v2.rs`, `pack.rs`, `store.rs`, and their tests.

**What** (the profile: 7 reads for each folder listing; 5.8 ms on Linux, 10.4 ms on Windows, 26.6 ms on Godot's demos on Windows):

- **`v2::Index::find`:** once the range left to search holds at most 128 IDs (4,096 bytes), read them in one read and search in memory; a longer range is narrowed on disk as today until it does. Then the line, as today. A planted index still costs reads, not memory: at most 4,096 bytes are held.
- **`PackReader::read_record`, and content or a chunk of at most 1 MiB stored (`NEAR`):** one read of the entry's header and stored bytes together; the header is checked against the index line exactly as `stored` checks it; for zstd the frame's content size is checked from the same bytes as today, and the bytes are decompressed in one call into a buffer of exactly the recorded size, so nothing past it can be produced (`zstd::bulk::Decompressor`, one kept per thread in a `thread_local!`, made on first use; making it failing is out-of-memory, as today). The fingerprint check that follows is unchanged. Anything larger takes today's streamed path.
- **`Store::each_pack`:** a lookup first asks the pack that held the last object, when it is open, without building the list of packs; only when that pack doesn't hold the ID does it build the order it builds today, leaving that pack out. The order of decision 061 is unchanged.

- [ ] **Step 1: Write the failing tests.** `pack/v2.rs`: `a_lookup_reads_a_short_range_once` (with the reader's test read counter: an ID in a pack of 5,000 objects is found in at most 3 reads; today's count is higher). `pack.rs`: on the one-read path, a header that disagrees with the index line, a zstd frame that states another size, bytes that decompress to another size and bytes whose fingerprint is wrong are each refused with the error the streamed path gives (table test over both paths: write an entry just under and just over 1 MiB and damage each the same way).
- [ ] **Step 2: Run them.** Expected: the read-count test fails.
- [ ] **Step 3: Build the three changes.** Keep `#![deny(clippy::indexing_slicing, clippy::arithmetic_side_effects)]` where the files have it.
- [ ] **Step 4: Run** the core suite, `pack_memory.rs` (Linux only measures; it still must compile), the corpus tests of the CLI crate. Expected: all pass; every test that flips bytes of a pack still catches every flip.
- [ ] **Step 5: Commit** `Find and read a small record in three reads`.

## Task 7: A folder that didn't change takes the last save's listing

**Files:** modify `crates/bolt-vcs-core/src/snapshot.rs` (`build_reading`, `build_with`, `read_folder`, `FolderNow`), `snapshot/walk.rs` (`Scan.old` keeps each listing's ID), `writeout.rs` (`planned` passes its drive and reuses listings), `status.rs`, and tests.

**What:**

- `Scan.old` keeps, with each base listing, the ID it was read under, when nothing was dropped from it (`without_git_names` returned no names). `build_with` compares a folder's children with that listing in order (both are sorted by name): the same count, and each child the base's entry of that name with the same kind, ID and size. Such a folder's listing **is** the base's, so its ID is the base's: no `Listing::new`, no encoding, no hash, and for a save no `store_listing` (the store holds it: the walk just read and checked it).
- `build_reading`'s first pass keeps each file's answer from the cache; the second pass uses it instead of asking again.
- `read_folder` takes the drive from a caller that already tested it (`planned`), and hands back the base listings it read, by ID, with the new ones, so the comparisons that follow (`status`'s, and the four in `planned`) find them in memory instead of reading and checking them again.
- A listing taken this way must be the one a build would make (Review Focus 6): `an_unchanged_folders_listing_is_the_one_a_build_would_make` saves a project of three folders, changes one file in one of them, saves again, and for each folder that didn't change compares the ID the new save's tree holds with the ID of a `Listing` the test builds itself from the files on disk (each file hashed by the test, each subfolder's ID taken the same way).

- [ ] **Step 1:** That test. It passes before the change (the build makes every listing); it is the net.
- [ ] **Step 2: Write the failing tests:** `an_unchanged_folder_is_not_rebuilt` (a test-only counter of listings built: a status of an unchanged project of 3 folders builds 0; after one file changes in `a/b`, exactly the folders on its path); `status_reads_no_listing_twice` (the store's test read counter: an unchanged status reads each base listing once).
- [ ] **Step 3: Build it.**
- [ ] **Step 4: Run** the whole workspace's tests, the model test with `PROPTEST_CASES=256`. Expected: all pass.
- [ ] **Step 5: Commit** `Take the last save's listing for a folder that didn't change`.

## Task 8: Measure, and take what is still over

**Files:** as the measurement says. This task is the controller's as much as an implementer's.

- [ ] **Step 1 (controller):** push `gate/m6b-1` from the branch; record the six ratios in the ledger with the run. Push a profiling branch (the timers of `bench/m6b-profile`, re-applied) if a step is over and the cause isn't plain.
- [ ] **Step 2:** For each step still over 2, take the next of these, in this order, each as its own commit with its tests, and measure again:
  1. **A saved file's ignore verdict only when it changed.** A file the last save holds stays whatever the rules say; its verdict only feeds two warnings, for a file that changed. `judged` skips the matcher for a file whose name the base listing of its folder holds as a file or link, and `build_with` asks for the verdict when such a file turns out changed, from the rules that were in force in its folder (the matcher's levels kept per folder, with the folder's `held`, `nested_held` and `git_held`). The warnings a save gives are byte for byte what they were (`a_change_to_a_saved_file_a_rule_matches_is_warned_about`, `what_boltignore_takes_back_from_gitignore_is_warned_about_when_new_or_changed`). It needs a decision record, which the controller writes first.
  2. **The last save's listings read ahead.** When the walk reads a base listing, helper threads read, check and decode the listings of its subfolders, bounded (at most 64 waiting), so the walk finds them ready. Same reads, same checks, same limits counted when the walk takes one.
  3. **The switch's own work:** the saves `kept::switch_saves` and `planned` read again, the second load of the file cache in `apply`, the reads `resolve_target` and the command line make after the switch.
  4. **The save's own work:** `check_names` and `large_save` only for folders that hold a name the base doesn't.
- [ ] **Step 3:** If a step is still over after those, the controller rules on the next structural change (walking sibling folders on several threads, or a per-folder memo in the cache), with a decision record and a plan amendment, before any code.

## Task 9: Big files are flushed while they are written (decision 102)

**Files:** modify `crates/bolt-vcs-core/src/fsio.rs`, `pack.rs` (`PackWriter`), `writeout.rs` (`write_file`), and tests.

**What:** `fsio::EarlyFlush`, the one place the rule lives:

```rust
/// Flushes a file's data from another thread while it is written, so the last flush has little
/// left to do. The last flush is still the caller's.
pub(crate) struct EarlyFlush { /* bytes since the last flush; the helper, once started */ }

impl EarlyFlush {
    pub(crate) fn new() -> EarlyFlush;
    /// Notes `n` more bytes written to `file`. Past 16 MiB it starts the helper, which flushes
    /// through its own handle (`try_clone`); every further 16 MiB wakes it.
    pub(crate) fn wrote(&mut self, file: &File, n: u64);
    /// Stops the helper and gives the first error one of its flushes met.
    pub(crate) fn finish(&mut self) -> io::Result<()>;
}
```

- The helper calls `sync_data` on its clone and keeps the first error. It never touches the file's position. If the clone or the thread can't be made, there is simply no helper, and the last flush does all the work as today.
- `Drop` stops and joins the helper (an abandoned pack or file must not leave a thread).
- `PackWriter`: `wrote` after each entry is added, with the entry's bytes; `finish_into` calls `finish()?` after `self.file.flush()?` and before `sync_all`. A `PackWriter` that is dropped unfinished drops its helper.
- `write_file`: the temp file is written through a small `Write` wrapper that counts bytes into an `EarlyFlush`; `finish()?` before `file.sync_all()?`.
- A test hook, compiled under the `fault-injection` feature only, makes the helper's flush fail at a named point (`flush:early`), beside the existing fault layer.

- [ ] **Step 1: Write the failing tests.** `fsio.rs`: `a_small_file_starts_no_helper`; `a_big_file_is_flushed_along_the_way` (write 40 MiB in 1 MiB pieces through `wrote`: the helper flushed at least once before `finish`; count through the test counter of Task 3); `a_failed_early_flush_fails_finish` (the hook). `crates/bolt-vcs/tests/chunks_stopped.rs`: `an_early_flush_that_fails_stops_the_save` (a 40 MiB file, `BOLT_FAULT_FAIL=flush:early`: the save fails, `head` hasn't moved, `store/packs` gained no pack) and the same for a `goto` that writes it (nothing renamed into the folder, the journal kept, the next command finishes it once the fault is off). `a_save_writes_the_same_packs_with_and_without_the_helper` (the pack names of a 40 MiB save equal those of the same save with the helper turned off by the test hook).
- [ ] **Step 2: Run them.** Expected: they fail to compile or fail.
- [ ] **Step 3: Build `EarlyFlush`, then the two callers.**
- [ ] **Step 4: Run** the core suite, `chunks.rs`, `chunks_stopped.rs`, `shelf.rs`, `writeout`'s tests. Expected: all pass; the crash sweeps stop at the same places.
- [ ] **Step 5: Commit** `Flush a big pack or file while it is written`.

## Task 10: A switch reads each chunk whole, and keeps the ones it has checked

**Files:** modify `crates/bolt-vcs-core/src/store.rs` (`copy`, `copy_chunks`, `copy_file`), `pack.rs`, and tests.

**What** (the profile: 151,653 steps of 8 KiB through two fingerprints for 13,546 chunks; a chunk already checked takes the slower, streamed path):

- A chunk is at most 256 KiB, so every chunk copy takes Task 6's one-read path: one read, one check of its fingerprint in one call, one write into the file's writer, which fingerprints the whole file as today. Known-good copies take the same path: being known good no longer sends a small copy down the streamed one.
- While `copy_chunks` writes one file, it keeps the chunks it has checked, by ID, up to 8 MiB of them (the oldest dropped first), and writes a chunk the list names again from memory. A chunk is still read and checked the first time, and the whole file is still checked at the end. This is what bounds the list of 8,388,608 one-byte chunks (a follow-ups row): after the first read of each of its 256 possible chunks, nothing is read.
- The file written is unchanged, and so is every error: a chunk missing, damaged, or of another size than the list says is the same damage, naming the same pack.

- [ ] **Step 1: Write the failing tests.** `store.rs`: `a_chunk_named_twice_is_read_once` (the read counter: a list naming one chunk 1,000 times reads it once); `a_repeated_chunk_is_still_checked_the_first_time` (damage the chunk: the copy fails as before); `the_kept_chunks_stay_within_their_budget` (a list of 200 distinct 256 KiB chunks: the memo never holds more than 8 MiB; assert through a test-only peak).
- [ ] **Step 2: Run them.** Expected: the first fails (1,000 reads).
- [ ] **Step 3: Build it.**
- [ ] **Step 4: Run** the core suite, `corpus_chunks.rs`, `chunks.rs`, `writeout_memory.rs` (it must compile here; Linux measures), and the ignored `most-chunks` test once in a release build, noting its time in the report (it took about 82 s on this laptop before).
- [ ] **Step 5: Commit** `Read each chunk whole when writing a file out, and keep the ones already checked`.

## Task 11: The line's saves, kept in order (decision 105)

**Files:** modify `crates/bolt-vcs-core/src/changeids.rs`, `history.rs`, `lines.rs`, `snapshot.rs` (the save's and the amend's use of the cache), `undo.rs` or `timeline.rs` (wherever `LineIds` is read), `crates/bolt-vcs-core/tests/vectors.rs` and `tests/vectors/change_ids.json`, `fuzz/fuzz_targets/change_ids.rs`; `docs/storage-format.md` §10.

**Why:** on the imported git/git (24,497 saves on its line), `bolt goto` to the save before the newest takes 1.3 s on Linux and 2.4 s on Windows against Git's 13 ms and 90 ms, nearly all of it reading every save of the line two or three times; `status` at a save and `undo --list` take over half a second.

**What:** `cache/change-ids` version 2, as storage-format.md §10 says (the controller updates the spec with the decision, before this task):

```text
bytes[8] "BOLTCHID"
u8       version = 2 (a cache of another version is ignored and rebuilt)
varint   line count (at most 100,000)
lines, sorted by name in byte order, no name twice; the empty name is the project's position at a save:
  string   name (at most 255 bytes)
  varint   save count (refused if more than the bytes left hold at 48 bytes each)
  save × count, from the newest back along first parents:
    id         the save
    change-id  its change ID
bytes[32] seal
```

- `ChangeIds` keeps `lines: BTreeMap<String, Vec<(Id, ChangeId)>>`. `chain(&mut self, repo, line: Option<&str>, tip: Id) -> Result<&[(Id, ChangeId)], RepoError>` gives the saves from `tip` back: the cached list when its first save is `tip`; the list with the saves in front of `tip` dropped when it holds `tip` further down; the list with the new saves read and put in front when reading back from `tip` meets its first save; otherwise the line read once. For `line == None` (at a save) it first looks for `tip` in every cached list, and uses that list's tail.
- `add` and `replace` (a save, an amend) put the new save in front, or in place of the first.
- `history::short_change_id`, `walk_short`'s callers, `LineIds::read`, `for_each_on_line`, `by_change_prefix` and the amend check take the chain from the cache. The short form is worked out as `walk_short` works it out, over the chain: the longest start `change_id` shares with the change ID of any save in it other than `id`.
- `history::by_fingerprint_prefix`, given all 64 digits, looks in the chain of the project's position first and stops at the first save found; a shorter prefix walks as today.
- The cache is still written only when it changed, without a flush, and only with a key.

- [ ] **Step 1:** the vector and the decoder's tests (version 1 ignored; a count past the bytes left, names out of order, the same name twice, bytes left over: each refused).
- [ ] **Step 2: Write the failing tests:** in `history.rs`, a proptest over random histories (saves, amends, a line moved back by `undo` and forward again, `goto` to a save): each of `short_change_id`, `LineIds`, `by_change_prefix` and a 64-digit lookup gives what it gives with the cache emptied before every call. And `a_goto_to_the_save_before_reads_no_line` (the store's test counter of saves read: after one warm-up command, `goto` to the first parent and back reads a bounded number of saves, not the line's length; today it reads the line three times).
- [ ] **Step 3: Build it.**
- [ ] **Step 4: Run** the whole workspace's tests (`history`, `undo`, `amend`, `lines`, `goto`, the model test). Expected: all pass, and no snapshot of printed output changes.
- [ ] **Step 5: Commit** `Keep each line's saves in order in the change-ID cache`.

## Task 12: A switch of many files

**Files:** `crates/bolt-vcs-core/src/writeout.rs` and its tests. Only after the controller has written its decision record (the rule changes how a switch's files are flushed) and measured that the gate's own steps are done.

**Why:** a switch that rewrites 3,051 files of git/git takes 1.7 s in carrying out alone on Linux (0.89 s of it one flush for each file, one after another) and 3.6 s on Windows, where Git's whole checkout takes 0.43 to 0.74 s and 1.4 s.

**What:** each file is still written into `.bolt/tmp`, flushed, looked at again, and only then renamed into place. The flushes run on a few helper threads while the next files are written: the thread carrying out the switch writes a file's bytes, hands the open temp file to a helper, and goes on; when a helper says a file is flushed, that file gets its second look and its rename. Nothing is renamed before its own flush has succeeded; a flush that fails is that file's error, as today. Folders, removals and links keep their order.

- [ ] Steps as the decision record gives them, with tests that a switch of 200 files stopped at every fault point is finished by the next command (the existing sweep, widened), that a flush failing on file 150 leaves the journal and every file wholly old or wholly new, and that the folder ends byte for byte as today's code leaves it.

## Task 13: The follow-ups rows

**Files:** as each row needs; `docs/plans/2026-11-01-milestone-1-follow-ups.md`.

The other session's decision 123 leaves ten rows to this one, each to be done or moved past v0.1 in a decision record that says why. The controller measured them on the profiling branch and writes the record; this task carries out what is code:

- [ ] **The import keeps 96 MiB of rebuilt objects** (decision 104): `DELTA_BASE_CACHE` in `git/odb.rs`, and the tests and docs that name 256 MiB.
- [ ] **Folders deeper than the walk holds open** (a save or status grows with the square of the depth past 126 levels): read files in the walk's order, top down, so the folder last opened is reused. Test: a chain of 200 folders with a file in each opens each folder a bounded number of times (a test-only counter of folder opens); the existing 2,048-level tests' times, before and after, in the report.
- [ ] **The other rows:** whatever the controller's record says is code; the rest are edits to the follow-ups list.
- [ ] **Commit** each row on its own, named for what it does.

## Task 14: Proof

**Files:** create `crates/bolt-vcs/tests/speed_shortcuts.rs`; modify `fuzz/fuzz_targets/file_cache.rs` and `change_ids.rs` if an API moved; `docs/testing.md`.

One test for each thing speed puts at risk ([security.md](../security.md) T32), end to end through the `bolt` command, on all three systems where the system allows:

- [ ] `a_planted_cache_is_never_trusted`: a `cache/files` sealed with another key that vouches for a changed file; `status` lists the change. The same for a planted `cache/change-ids` that names another save for a change ID: `goto` goes to the right save.
- [ ] `a_stale_cache_is_only_trusted_file_by_file`: save, copy `cache/files` away, change a file (same size, new time), put the old cache back; `status` lists the change.
- [ ] `a_file_changed_within_two_seconds_is_read_again`, through the command (the unit test exists).
- [ ] `a_file_changed_during_the_walk_is_seen` (the act point `walk:looking` exists: change a file's bytes and time there; the status that follows lists it).
- [ ] `a_folder_swapped_for_a_link_mid_walk_is_refused` and `a_git_folder_made_mid_walk_is_seen_by_the_next_command`, with the existing act points.
- [ ] `status_after_goto_reads_what_goto_wrote` (the files a switch wrote are too new to trust: a change made right after the switch, same size, is listed).
- [ ] The model test passes with `PROPTEST_CASES=1024` once, locally, in a release build; its time goes in the report.
- [ ] **Commit** `Test what the speed work puts at risk`.

## Task 15: Docs

**Files:** `docs/design.md`, `docs/storage-format.md`, `docs/testing.md`, `docs/security.md`, `docs/milestones.md`, `docs/benchmarks.md`, `docs/plans/2026-11-01-milestone-1-follow-ups.md`, `bench/README.md`, `CLAUDE.md` only if a rule changed.

- [ ] Every sentence about the walk, the caches, the store's reads, flushing and the gate matches the code as built; the milestone's steps in `milestones.md` get their task numbers and ticks; `testing.md` §3 names the new tests; `security.md` T32 names its tests.
- [ ] Every edited file: `tr -cd '\r' < file | wc -c` prints 0; every relative link resolves.
- [ ] **Commit** `Bring the docs in line with the speed work as built`.

## Task 16: Red team, review, benchmarks and the pull request

The controller's: the attack-surface file (`redteam/attack-surface-m6-speed.md`), two sessions in the no-network sandbox (RT12 and RT13; `redteam/attacks11` and `attacks12`; tests `redteam12*.rs` and `redteam13*.rs`; reports `docs/redteam/<date>-milestone-6-speed.md` and `-session-2.md`; CI branches `redteam/m6b-*`), every proven finding fixed or ruled on in a decision record; a whole-branch review; the whole benchmark and the three gates run again on the final code; `benchmarks.md` with the numbers before and after; the rulings file; the pull request.
