# Milestone 5 attack surface (red-team session, decision 024)

Milestone 5 handles big files.
- **Chunks.** A file of 1 MiB or more is cut into chunks of about 64 KiB, by a cutter that looks at the bytes, so equal stretches of different files and versions are stored once. The file is stored as those chunks plus a chunk list: a record of kind 18, kept under the file's own ID.
- **Workers.** Saving, writing files out and importing read each file once, on several worker threads. One writer adds what they make to the packs, in a fixed order.
- **Git LFS import.** `bolt import` brings in Git LFS files whose contents are on disk in `.git/lfs/objects`, checking each against its pointer.
- **The first save after an import** stops where the folder holds a Git LFS pointer instead of the real file.
- **Delta families.** The import rebuilds each Git delta family of a big file once, bottom up, instead of walking its chain for every version.

Threat list: `docs/security.md` §4 (T1–T31; T31 is new). Decisions 082–093 in `docs/decisions/` give the rules this code follows. `docs/design.md` §5.16 summarises the import, and `docs/storage-format.md` §6, §11 and §13 give the bytes and the test vectors.

Your goals:
- **Data:** lose or corrupt data: unsaved work, saved history, a big file's versions, files outside Bolt.
- **Wrong bytes:** make wrong bytes reach the folder, or a command report a file as something it isn't.
- **Escapes:** read, write, remove or follow a link outside the project, or inside `.git` (including `.git/lfs`) or `.bolt`.
- **`tidy`:** make it remove a chunk, or a chunk list, that is still needed.
- **Resources:** crash, hang, or use memory, disk or time past the bounds the docs state.
- **The person at the terminal:** trick them with spoofed output, misleading messages, or a stop that names the wrong file or the wrong way out.

## Where the code is

- **Chunks:**
  - `crates/bolt-vcs-core/src/chunks.rs`: the cutter; the chunk-list record, decoded as it is read; the limits.
  - `ingest.rs`: how bytes enter the store (`store_bytes`, `store_held`, `ready`, `add_ready`).
  - `store.rs`: `copy_content` and `copy_chunks`; `Wanted`, which decides what a lookup may take; the set of copies already checked; the `NEAR` rule.
  - `pack.rs` and `pack/`: kind-18 entries, `add_chunk_list`; `add_record` refuses lists.
  - `verify.rs`: list checks.
  - `repo.rs`: `feature chunks` in `.bolt/format`.
- **Workers:** `workers.rs`: the pool, the 64 MiB window, at most 4 jobs a worker, the stop flag, `Lost`.
- **Merging and `tidy`:**
  - `merge.rs`: lists are copied last, in a second pass.
  - `tidy.rs` and `tidy/`: `survey`, `check_index`, `chunks()`, `cant_tidy`. Every chunk a kept file's list names is kept, and so is every chunk a list in a young pack names. A pack it can't open stops it.
- **Saving:**
  - `snapshot.rs`: `Job`, `Done` and `Jobs`; `read_small`; `build_with`; the whole-folder pointer look on the first save after an import.
  - `snapshot/endings.rs`: `pointers()` and `stands_in`.
- **Write-out:** `writeout.rs` (`copy_content` into `.bolt/tmp`, renamed into place only once it returns `Ok`; safety copies on the workers) and `fsio.rs` (temporary files and the fault points).
- **Git LFS import:**
  - `git/object.rs`: `LfsPointer`, `lfs_pointer` (Git LFS's own parser rules), `is_exact_lfs_pointer`, and the look-alike test.
  - `git/mod.rs`: `has_lfs_folder`, `lfs_object_path`, `lfs_object` (opened without following a link), `has_config_includes`.
  - `git/config.rs`.
  - `import.rs`: the first pass's `LfsCheck`, `lfs_storage_elsewhere`, and the stops `LfsMissing`, `LfsMismatch`, `LfsExtension` and `LfsStorageElsewhere`.
  - `import/convert.rs`: `LfsRules`, `Sha256Reading`, the memo of object IDs, the two reads of a file over 128 GiB, and the family closure.
- **The delta-family rebuild:**
  - `git/pack.rs`: the `Bases` scan of a pack's entry headers.
  - `git/odb.rs`: the base cache; `spend`, the allowance, and `TooMuchWork`'s message.
  - `git/odb/family.rs`: `rebuild_family`, `family_bottom`, `Family`, `Kept`, and `Spill`, the files kept in `.bolt/tmp`.
- **CLI:** `crates/bolt-vcs/src/main.rs` and `output.rs`. `docs/json.md` covers the `lfs_lookalikes` warning, the Git LFS stops, `GitError::Bug` and the pointer stop.

## Places worth attacking

- **Planted chunk lists** (T3, T7, T11). Plant lists in a pack a save names, for example:
  - sizes that lie, or don't add up;
  - a count past the record's bytes;
  - a 10 GiB claim over small chunks;
  - another file's chunks;
  - a list under a chunk's ID (a chunk must never be read through a list);
  - nested lists;
  - a damaged list beside a good whole copy;
  - two different lists for one ID in two packs.

  Then make wrong bytes reach the folder: `copy_content`'s output is untrusted until it returns `Ok`, so look for a caller that uses it sooner. Or make `status`, `changes`, `show` or `verify` misreport. Task 9's corpus (`crates/bolt-vcs/tests/corpus_chunks.rs`) pins the basic cases; go past them.
- **The cutter:**
  - inputs that make only minimal chunks;
  - files at the edges: 1 MiB − 1, 1 MiB, 128 GiB, 128 GiB + 1;
  - its memory;
  - whether two machines cut a file the same way (the vectors are in storage-format.md §13).
- **Merging and `tidy`** (T27):
  - make `tidy` remove a chunk that a kept file's list names, that a list in a young pack names, or that a list in a pack it can't read names;
  - make a merge drop, duplicate or mix lists for one ID;
  - make a pack unreadable halfway through `tidy`;
  - stop `tidy` and merges at every fault point.
- **The workers** (decision 084):
  - break the writer's fixed order;
  - pass the 64 MiB window;
  - change a file while workers read it (the save must refuse it);
  - make a worker panic;
  - find a test hook (cut sizes, worker count, delta limits) that a real `bolt` can reach.
- **Git LFS import** (T31, T7, T11):
  - pointers and look-alikes (a look-alike comes in as text, with a warning), and extensions (a stop);
  - `.git/lfs` or `.git/lfs/objects` as a link or a file (a stop, decision 089);
  - objects of the wrong size or hash;
  - `lfs.storage` in every place Git reads it: the user's own settings and their includes, `.git/config`, and `config.worktree` with `extensions.worktreeConfig`;
  - thousands of missing objects: the first pass's list, its memory and its time;
  - an object that changes between the size check and the stream;
  - an object over 128 GiB, which is read twice;
  - pointer files reached only as links, in odd trees, or in commits only refs outside branches name.
- **The first save after an import** (decisions 085 and 090):
  - make it save pointer text over a real file: partial saves, amends, files the file-state cache vouches for, a pointer edited so it no longer parses, a pointer at another size;
  - make it stop on an ordinary file;
  - check that `bolt discard`, the way out it names, puts back the right file.
- **The family rebuild** (decisions 086 and 091–093):
  - hostile families: loops, ref deltas into other packs, huge fan-out, chains near 4,096 deltas, members whose bytes don't match their names;
  - families of millions of tiny members: they cost time within the allowance, and decision 092 says they cost no memory in the memo;
  - the spill files in `.bolt/tmp`: the 4 GiB cap, removal on every path, and a file planted at their names;
  - the base cache's 256 MiB budget;
  - the work allowance itself.
- **Write-out against Git LFS's CVE-2025-26625:**
  - a hard link at a path a switch writes, and a link at a folder above it, for chunked files too (Task 9 pins the basic cases);
  - a folder swapped for a link between the plan and the write.
- **Crashes:** stop every command that stores big files (`save`, `discard`, `shelf`, `unshelf`, `import`) at every fault point, then run any command. Nothing may be lost, and the next changing command must finish or refuse cleanly.
- **Terminal output:** escape sequences and bidi characters in Git LFS object names, pointer paths, `lfs.storage` values, config paths and pack names, in every message and every `--json` document.
- **Windows:** Windows-only attacks (hard links, files in use, read-only files, long paths under `.bolt/tmp`) become tests on a `redteam/m5` branch, run by CI and deleted afterwards.

## Limits already known (not findings unless you show more)

`docs/security.md` lists the known limits for T1–T31, and `docs/plans/2026-11-01-milestone-1-follow-ups.md` the deferred items. Among them:
- **RT4-1** (decision 087): records aren't chunked, so importing a history whose commits share one big message still needs far more disk than Git.
- **Tiny chunks are slow.** A valid list of 8,388,608 one-byte chunks takes about 82 s (release) to write out its 8 MiB file on the Windows laptop. Nightly's `most-chunks` job runs it. This is a finding only with more: many such files, memory past about 336 MB a list, or the same cost from a smaller list.
- **A switch stopped by damage** leaves its journal, and every later changing command meets the same damage until the store is fixed. This has been so since milestone 3, and a follow-up looks at it.
- **The work allowance** can stop an honest long history of small edits to one big file, in a repository that is mostly that file. The stop says so and names a Git step that gets past it (decision 091).
- **Small family members.** A big file's family members under 1 MiB are built and checked, then let go (decision 093). A commit that needs one rebuilds it through the cache.
- **Bolt and Git LFS.** Bolt reads Git LFS objects only from `.git/lfs/objects`. It never fetches, and never runs Git LFS.
- **The first save's stop** names five pointer files and counts the rest; `bolt status` lists them all (decision 090).
