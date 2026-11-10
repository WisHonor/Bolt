# Milestone 5 ("Big files"): rulings

The owner asked for milestone 5 to be built start to finish while they were away, with every call made by Claude and written down. This file collects those calls. It has three parts:
- **Please confirm:** the calls that most change what Bolt does or what the owner signs up for. Each can be reversed.
- **Every ruling, verbatim:** each ruling as it was written in the build's ledger when it was made, task by task.
- **Deferred:** what this milestone left for later.

The decisions themselves are records [082](../decisions/082-how-big-files-are-cut-into-chunks.md) to [098](../decisions/098-a-blob-is-also-judged-in-its-crlf-form.md). Each says "Decided by Claude, at the owner's request while the owner was away. The owner can reverse any part."

## Please confirm

### Decisions that set how big files work

1. **How files are cut** ([082](../decisions/082-how-big-files-are-cut-into-chunks.md)):
   - FastCDC, with chunks of at least 16 KiB, about 64 KiB, and at most 256 KiB.
   - Only files from 1 MiB to 128 GiB are cut. Bigger files are stored whole, as before.
   - These numbers freeze with format 1 at v0.1.
2. **How chunks are stored** ([083](../decisions/083-a-chunk-list-is-stored-under-its-files-id.md)):
   - A file's chunk list is a new record, kind 18, kept under the file's own ID, so no existing ID changes.
   - `.bolt/format` gains `feature chunks`. An older Bolt refuses a project that has it, rather than misread it.
3. **Several threads** ([084](../decisions/084-files-are-read-once-on-several-threads.md)):
   - 1 to 8 worker threads read and hash files.
   - One writer adds what they make to the packs, always in the same order.
   - At most 64 MiB of work is out at once.
4. **Git LFS import** ([085](../decisions/085-importing-git-lfs-files.md), [089](../decisions/089-git-lfs-objects-kept-elsewhere.md), [090](../decisions/090-the-first-save-looks-at-the-whole-folder-for-pointers.md)):
   - Bolt brings in Git LFS files only from `.git/lfs/objects`. It never fetches them and never runs Git LFS.
   - An import stops when a needed object is missing, damaged or stored elsewhere.
   - A pointer with extensions stops it too.
   - A file that only looks like a pointer comes in as text, with a warning.
   - The first save after an import stops at any pointer file left in the folder, and names `bolt discard` as the way out.
5. **Git's deltas rebuilt once** ([086](../decisions/086-rebuilding-gits-deltas-once.md), [091](../decisions/091-the-work-stop-names-a-way-out-for-long-histories.md) to [093](../decisions/093-a-family-rebuild-builds-every-member.md)):
   - The import rebuilds each big file's delta family once, keeping decision 044's work allowance.
   - A long, honest history of small edits to one big file can still pass the allowance. The stop now says so, and names a Git step that gets past it: `git -c core.bigFileThreshold=1m repack -a -d -f`.
   - Raising the allowance, or a "trust this repository" flag, are the alternatives if that stop turns out to be common.
6. **No write limit for imports** ([087](../decisions/087-what-an-import-writes-once-files-are-chunked.md)):
   - Chunking ends the disk blow-up for file contents.
   - It doesn't end it for commit messages and folder listings. RT4-1's repository still imports about 98 times bigger than Git keeps it.
   - Storing records as deltas, later, is what would end it.
7. **Benchmarks** ([088](../decisions/088-benchmarks-and-the-speed-gates.md), [results](../benchmarks.md)):
   - Both speed gates pass with a wide margin. Saving 10 GiB peaked at 43.5 MB on Linux and 48.7 MB on the laptop, against 500 MB. `status` on an unchanged 100,000-file project took 0.70 s and 1.16 s, against 2 s.
   - Git still wins `status` and small saves on small projects: 62 ms against 13 ms on git/git.
   - No stronger zstd level earns its time, so merging and `tidy` keep stored bytes as they are.
   - Confirm the fairness choices in §2: waiting out Git's background repack, a `sync` before every timed run, and files aged 3 s before `status`.

### Red-team rulings

8. **The user's Git settings are read where Git reads them** ([094](../decisions/094-red-team-m5-rulings.md) to [097](../decisions/097-red-team-m5-session-2-rulings.md)):
   - the `GIT_CONFIG_*` variables, `GIT_CONFIG_SYSTEM` tidied as Git tidies it, and `/dev/null` naming no file;
   - the home folder found as Git finds it.

   Still not read, each with a follow-up row:
   - `[includeIf]` sections;
   - the system's own settings file when no variable names it;
   - `-c` options passed through `GIT_CONFIG_PARAMETERS`;
   - `GIT_CONFIG`.

   Where one of those may matter, the stop for missing Git LFS files says so.
9. **Files that Git LFS may check out as something else** ([094](../decisions/094-red-team-m5-rulings.md), [097](../decisions/097-red-team-m5-session-2-rulings.md), [098](../decisions/098-a-blob-is-also-judged-in-its-crlf-form.md)):
   - A blob that doesn't come in as a Git LFS file is named in a warning whenever Git LFS may read it as a pointer. That covers its own first 1,024 bytes, those bytes with CRLF line ends, and anything that looks like a pointer.
   - Bolt keeps the blob's own bytes.
   - The other way round stays a known limit: a padded pointer whose CRLF form Git LFS reads as text.

### Other calls

10. **A test-only dependency.** `bolt-vcs` gained `blake3 = "1.8.7"` as a dev-dependency. It is the same crate and version the engine already uses, so `Cargo.lock` gains no package.
11. **Red-team checks kept as tests.** The red teams' Windows checks (`redteam8_windows.rs`, `redteam9_windows.rs`) passed on the fixed code, and stay in the suite as ordinary tests.
12. **Git LFS in the red-team sandbox.** The sandbox image gained `git-lfs` (one word in `redteam/Dockerfile`), so attacks can build Git LFS repositories.
13. **Known costs left as they are, each with a follow-up row:**
    - A switch stopped by damage holds the project until the store is fixed (since milestone 3).
    - A valid list of 8,388,608 one-byte chunks takes about 82 s to write out an 8 MiB file. That could change the format before v0.1 freezes it.
    - The git/git import peaks about 100 MB higher than before, most likely the Git reader's cache staying near its 256 MiB budget.
14. **Edits to accepted decision records:** only "Refined by" lines were added, to link the new records both ways, as decision 081 does. The records changed are 047, 060, 085, 086, 089, 092, 094, 095 and 097.
15. **Milestone 6's terminal UI:** the owner's design canvas was reviewed ([review](2026-11-09-terminal-ui-canvas-review.md)), with a prompt to redo it. It isn't yet known whether the owner has re-prompted.
16. **A sentence in decision 097 that the code narrows:**
    - Decision 097 says that with no home folder, "a setting that starts with `~/`" stops the command.
    - The code stops only on an include path, or a `core.excludesFile`, that starts with `~/`. It never expands `lfs.storage`, so that setting never stops it.
    - design.md §5.16 and storage-format.md §12 say exactly this. Accepted records aren't rewritten, so this note records the narrowing.
17. **One slip, already fixed.** The controller committed the second red team's proof file without a Windows lint check, so CI failed on `1b31dc6`. The fix (`f00e688`) made it build on every system.

## Every ruling, verbatim

Each line below is copied as it was written in the build's ledger, in the order the rulings were made, under the task it belongs to. A ruling names what was decided, why, and what it costs if it is wrong. Commit IDs and agent names in them refer to the build; the decisions they mention are in `docs/decisions/`.

### Before Task 1

- Ruling: housekeeping goes in its own small PR (#17, docs only) based on `main`; the design PR branch (`milestone-5-design`) is based on the housekeeping branch so they merge cleanly in order — the owner asked for housekeeping first — if wrong: none.
- Ruling: the design PR carries decisions 082–088, the back-links on the records they refine or supersede (one line each, nothing else edited), design.md, storage-format.md, security.md, milestones.md, the plan, and the three vector files (`chunking.json`, `chunk_list.json`, `pack_chunked.json`), computed by a scratch program written from storage-format.md §6's text, independent of the product code, so the spec and vectors come before code — milestone 4's design PR carried decisions, docs and plan the same way — if wrong: the vectors could share a misreading with the spec; the implementation's tests will catch any disagreement with the text.
- Ruling: a chunk list is stored under its file's ID (kind 18), superseding decision 019 #10's cache — a cache can't vouch, needs a scan on every miss, and tidy can't rely on it; a list's own ID would prove nothing since nothing trusted names it — if wrong: a format change before format 1 freezes.
- Ruling: Bolt's own FastCDC (2020 form, normalized level 1), top-bit masks (17 then 15), gear table from BLAKE3 XOF with context `bolt-vcs 2026-11-08 chunk-cutter gear v1`, 16/64/256 KiB, from 1 MiB — no new dependency, cut points pinned by the spec — if wrong: costs dedup with earlier cuts, never correctness.
- Ruling: files over 128 GiB are stored whole — the gear table is public, so a crafted file can force minimum-size chunks; under 128 GiB a list stays under 300 MB (8,388,097 chunks), inside 2^24 and 512 MiB, without a list of lists — if wrong: such files get no dedup; a list of lists later.
- Ruling: a stored chunk list counts as a good copy only when the writer has just cut the same file into exactly its chunks (each checked or stored in that pass); otherwise the writer stores its own list beside it, and `has_good_copy` never counts a list — avoids reading a big file back to prove a list — if wrong: an extra small list per file in rare cases.
- Ruling: `tidy` stops at any pack it can't open, read or check (refines 077's "left with a warning") — such a pack could hold the only list naming chunks in other packs — if wrong: a damaged pack blocks tidy until moved aside; the hint says how.
- Ruling: one writer thread in a fixed order, workers for reading, hashing, checking and compressing, at most 8, window ~64 MiB, lookahead stops at the next big file — pack names hash the index (offsets), so order must be fixed for identical packs and repeatable crash sweeps; stopping lookahead at a big file avoids nested windows and deadlock — if wrong: less parallelism than possible.
- Ruling: Git LFS files are recognised by content (Git LFS's parser), not by `.gitattributes` — the same set `git lfs fetch --all` downloads, and conversion stays a function of the blob so the per-Git-ID memo holds — if wrong: a pointer-text file at a non-LFS path imports as its object (known limit, deferred).
- Ruling: the "nothing written" Git LFS check runs in the first pass only when the repository shows a sign of Git LFS; without one, a missing object stops the second pass at the first — the full-history walk costs time, and repositories with pointers but no `.git/lfs` have no objects anyway — if wrong: one stop per run in a rare case.
- Ruling: look-alikes import as they are with an `lfs_lookalikes` warning (decision 037 stopped at them) — Git LFS checks them out as they are — if wrong: a parser bug would import a real pointer as text, which the differential test against git-lfs guards.
- Ruling: pointers with `ext-` lines stop the import; `lfs.storage` in `.git/config` is refused (read only to refuse); a `size 0` pointer is an empty file — Bolt never runs programs and reads only `.git/lfs/objects` — if wrong: rare repositories can't import.
- Ruling: the first save after an import stops at Git LFS pointer files whose size line is the imported version's size, with no flag — saving pointer text over real files is never meant — if wrong: a user who means it edits once.
- Ruling: delta families are rebuilt once, triggered by the first deltified blob that builds ≥ 1 MiB; family headers scanned once per pack, ≤ 8 bytes an entry, lazily; objects with children still to come stay in the 256 MiB base cache while they fit — keeps rt3_8/rt4_4's 750 MB bound — if wrong: branching families of huge objects rebuild some bases twice.
- Ruling: RT4-1 stays without a write limit (decision 087) — chunking ends the big-file part; records aren't chunked; any limit refuses honest repositories; records as deltas is the real fix — if wrong: a hostile repository can still fill a disk, cleanly.
- Ruling: benchmarks by shell scripts + hyperfine on CI runners (never installed on the laptop), data generated or from public repos at fixed tags; no criterion; the bench workflow also runs on pushes to `bench/**` because a manual workflow can't run before it's on `main` — if wrong: coarser statistics than criterion.
- Ruling: compressing again only if level 9 or 19 saves ≥ 10% at ≤ 5× level 3's time on the code and uncompressed sets — tidy and merging must stay usable on big stores — if wrong: a few % of disk.
- Ruling: every subagent runs on Opus (`model: "opus"`), implementers and reviewers alike — the owner's instruction — costs tokens only.
- Ruling: the controller (me) pushes; implementers commit locally and stage files by name; the untracked `.codex/` and `AGENTS.md` are never staged — outward actions in one place — if wrong: none.
- Ruling R-S1: Task 3's test hook may move only CHUNK_FROM and CHUNK_UP_TO (and only under test-support), never the cutting rule — the cut points are pinned by vectors — if wrong: none.
- Ruling R-S2: Task 10 creates `.github/workflows/bench.yml` with only the memory-gate job (on `workflow_dispatch` and pushes to `bench/**`), plus the nightly job; Task 11 adds the benchmark jobs to the same file — Task 10 needs a way to run on Linux before the workflow exists on main — if wrong: none.
- Ruling R-S3: the cutter's hot loop may index the 256-entry table with a byte under a narrow `#[allow(clippy::indexing_slicing)]` with a one-line reason, and uses `wrapping_shl`/`wrapping_add`; everything else in chunks.rs keeps the module's denies — a `get()` per byte would slow the cutter for no safety gain (a u8 can't pass 255) — if wrong: one allow to remove.
- Ruling (finding 1, high): merging and `tidy` copy chunk lists last; a list whose ID the output pack holds as whole content is left out, a different list for the same ID starts a new pack, an identical one is left out — a pack holds one entry per ID, and a planted structurally-valid list could otherwise displace a checked chunk or whole copy and lose data — if wrong: an extra pack in a rare case.
- Ruling (finding 2, high): repositories list `feature chunks` in `.bolt/format`; `start`/`import` write it, every changing command adds it right after the lock before storing anything; it is the one format change made without `bolt setup` (refines 008) — a pre-milestone-5 build refuses a kind-18 index line over 512 MiB as damage and its tidy then goes on without that pack's lists, so it could delete chunks — if wrong: old dev builds refuse repositories M5 touched (intended). Replaces the withdrawn "no feature line" ruling.
- Ruling (finding 3a): a family member whose delta fails or whose result isn't its index name is neither stored nor mapped, still serves as a base, and stops the import only if a commit needs it — rt3_8/rt4_4 build members under wrong names and expect success — if wrong: none.
- Ruling (finding 3b): a member too big for the cache with children still to come is spilled to `.bolt/tmp` (at most 4 GiB at once) and read back, else rebuilt from the bottom within the allowance; T11's claim says so — keeps honest branching families linear — if wrong: disk use in tmp during an import.
- Ruling (finding 3c/3d): the "rare after gc" claim is replaced (reflogs, stash, notes, refs/original keep such objects); the import's rebuild memory ceiling (~1.3 GiB + chunk window) is stated in 086 — honesty — if wrong: none.
- Ruling (finding 4): the worker window counts each job from hand-out until its result is taken, at its input plus the most its result can be, capped at 64 MiB and 4 jobs a worker — counting only finished results lets jobs pile up unseen — if wrong: less parallel look-ahead.
- Ruling (finding 5): 085 refines 032: the import counts commits holding Git LFS files, since an export can rebuild their Git IDs only by turning those files back into pointers — nothing records which entries were pointers, and recording paths per save would pass the extra-field limits — if wrong: export may need a record added then.
- Ruling (finding 6): the 1 MiB threshold is fixed until milestone 6's presets make it the `bolt.toml` setting decisions 013 and 016 describe; §6 now says the format allows any file whole or chunked and Bolt cuts 1 MiB–128 GiB — if wrong: none.
- Ruling (finding 7): `lfs.storage` is also refused when the user's own Git settings set it (060's reader); pointer text kept as data without objects is a known limit (deferred); pointer-size sums saturate — if wrong: a rare repository can't import.
- Ruling (finding 8): a chunk list holds at most 8,388,608 chunks (was 2^24) and is decoded as it is read, so a hostile list costs at most ~336 MB — Bolt's cutter never makes more than 8,388,097 — if wrong: none before format 1 freezes.
- Ruling (finding 9): compressing again never touches a chunk list (kind 18 stays uncompressed) — else every list becomes damage — if wrong: none.
- Ruling (finding 10): back-links added for 062 (and 008, 013, 016, 032, 060); Task 5 counts lists at stored size in tidy's sums; 084 says "up to 128 GiB" and states the FAT/exFAT change-detection limit honestly — if wrong: none.
- Ruling (finding 11): invalid vectors fixed (`no-chunks` is an empty file's list; `count-over-the-limit` is 8,388,609 and also breaks "past the bytes", tests only assert refusal); §13 says the noise has no key material — if wrong: none.

### Task 1

- Ruling: Task 1's `ChunkList::read_from` returns `ReadListError { Damaged(DecodeError), Io(io::Error), OutOfMemory }`, reserving with `try_reserve`; `DecodeError`/encoding.rs unchanged — the store must tell damage from a read error or too little memory, as it does for other records (028/061), or a failing disk would mark good packs damaged — if wrong: a small enum to fold back. Sent to the implementer before review.

### Task 2

- Ruling: this is a milestone 4 edge (unshelf's recorded folder change), not Task 2's code (Task 2 touched no shelf/undo/kept code); it is debugged by its own subagent to the root cause, fixed as its own commit with an engine regression test plus the proptest regression line, and reviewed like a task; Task 2 counts as done only once CI is green on a head that holds both — if wrong: none.
- Model-test fix: review Approved (c1c47ac). Important (spec only, predates the fix): storage-format.md §9's planned-from sentence contradicted the made-folder rule and the unshelf sentence. Ruling: reword it now to the reviewer's sentence ("Below a path such a command writes, both sides hold that save's version wherever it differs from the base, … except inside a folder the command makes where the folder held a file, a link or nothing.") — no decision record, since 071/073/081 already say the fixed behaviour; the code is the authority's match — if wrong: one sentence.
- Ruling: the re-review's out-of-scope observation 1 (a ≤ 1 MiB first copy — a chunk list whose chunk's pack can't be opened for a non-damage reason — is re-read in the final one-try pass after the check pass proved another copy good, so the read fails though a good copy exists; breaks §11 :402/:404) is promoted to a Task 2 finding for fix round 2: it lives in Task 2's read path and Task 3's write-out callers rely on copy_content — fix: the check pass hands the copy that passed to the one-try read, which starts from it; probe test first — if wrong: one more round.
- Task 2: parked — a known-good first copy, now possibly a chunk list proven earlier in the process, is streamed with no fallback; a passing non-damage failure re-reading its chunks fails that read though a whole copy exists — Ruling: a known-good list is streamed like a known-good whole copy, whose transient re-read failure already fails the read (milestone 3's ruling, 2026-11-05 rulings :74); the next command reads again; the final review sees it — if wrong: a rare transient read failure.

### Task 3

- Task 3: review Approved, spec ✅ (d039a51..de599ce). ⚠️ resolved: Linux CI green (memory tests, import read count); in-process stop sweep via fault::arm — Ruling: accepted, as repo.rs:3641 and import_interrupted.rs do (a real abort sweep would need a pack-limit hook the binary reads) — if wrong: a crash-between-chunks case covered in-process only; separate test file justified (process-wide pack limit).

### Task 4

- Ruling: concern 8 (safety copies still read small files on the calling thread, as the plan's Task 4 text says) — decision 084 binds ("`save`, `status` and the safety copies read, hash and check files … on worker threads"); the plan narrowed it without a record, so keep_safety_copies hands small files to the workers too, same window and one writer, before review — if wrong: a little extra code in writeout.rs.
- Ruling: concern 7 (when two workers find bad copies in one pack, which one the pack's damage warning quotes can vary) — accepted: decision 084 fixes the packs and the first problem, not which of several damaged objects a warning quotes; such a store is already damaged and `bolt verify` lists every bad object — if wrong: one tie-break (lowest offset) in the store's damage note.
- Ruling: concerns 1-6 accepted as observations (per-call scoped threads avoid `unsafe`; a big file's inner workers; repeated content compressed twice, stored once; a small file's change check right after its read stores the consistent copy read, as decision 084 says; status waits for jobs out) — the reviewer sees them.
- Ruling: Minor 4 (with workers the store can hold up to 24 packs open, and two workers missing one pack both insert it; store.rs:851's comment is false) is fixed now — decision 061 and storage-format §11 say at most 16 packs open at a time, and the docs must match the code — fix: never close a pack a lookup is reading, count opens in flight, reuse a pack another thread opened meanwhile; a multi-thread test — if wrong: a few lines in store.rs.
- Ruling: Minor 5 (which pack a store error names can vary with timing when two copies fail alike) is fixed now with a fixed tie-break among errors of equal rank — decision 084 promises the same first problem whatever the timing, and the pack named is part of the error — if wrong: one comparison.
- Ruling: Minor 1 (status waits for files already being hashed after its first problem) is fixed now — a stop flag checked between reads restores today's prompt stop — if wrong: a few lines.
- Ruling: Minor 2 (safety copies: a later action's store error now beats an earlier action's copy error) is fixed now — the entry is looked up as each job is pulled and its error carried in order, as save's Job::Failed does — if wrong: a few lines.
- Ruling: Minor 3 — the writer-side `packs.contains` short-circuit comes back (store_held, store_bytes' small path); repeated content on the workers stays, and decision 088's benchmarks add a repetitive big file — if wrong: CPU on repeated chunks.
- Ruling: Minor 6 — add: a panicking worker ends in a panic, never a hang; jobs stop being pulled after the first error; the crash sweep compares pack names; `lend` with many jobs in different folders if it fits the deep-tree test cheaply. The no-thread fallback test is deferred — if wrong: a rare path untested.
- Ruling: Minors 7, 9, 10, 11 fixed now (shared test helpers in test_support; `Lost` before the hooks; hash before the store check in read_copy; no decision number in snapshot.rs's link_target doc).
- Ruling: Minor 8 (snapshot.rs grew about 250 lines of reading glue) deferred to the final review — moving it now is churn on a reviewed diff; snapshot/reading.rs is its home — if wrong: one move later.
- Ruling: fix round 2 = comment wording only (store.rs:2, :878-879, :413, :185-191, workers.rs:330); also, among errors of equal rank, prefer one that names its pack before ordering by text if the error type tells them apart cheaply (the re-review saw a raw I/O error naming no pack beat "Bolt couldn't read pack X…" by alphabet; spec §11 says the error names the pack) — if wrong: a comparison key.
- Task 4: out-of-scope (deferred to the final review): `Packs::list`'s retain and `close_all` drop open packs without checking a lookup still reads them, so after another process merges a pack mid-lookup a pack can stay open uncounted (at most the lookups in flight over); close_all runs only in merge and tidy — Ruling: deferred — needs a concurrent merge during a status; bounded by 9 — if wrong: a few handles over 16 for a moment.
- Task 4: minor (deferred to the final review): a copy's own read error names no pack, so among equal-rank errors it can be told first by alphabet; spec §11 says the failing lookup's error names the pack — Ruling: deferred — pre-existing (the same class as Task 2's copy_unreadable naming the wrong pack, store.rs:678); deterministic now; the fix is building that error with its pack's name — if wrong: a less helpful message in a double failure.

### Task 5

- Ruling: Task 5 concern 1 (a folder, a link or an unreadable entry named `*.pack` now stops tidy) — accepted: the store lists every `*.pack` entry as a pack, and §11 says a pack tidy can't open stops it — if wrong: one filter.
- Ruling: Task 5 concerns 2, 3, 5 (each old pack's index checked twice; the hint leads with permissions even for damage, as the brief words it; a damaged list under a kept file stops tidy even beside a good whole copy, and such a file's list chunks are freed one tidy later) — accepted as observations; strict stops are what decision 083 says — the reviewer sees them.
- Ruling: Task 5 Minor 1 (what counts as a pack is decided in store.rs:262-264 and again in tidy.rs:202-204; tidy's no-loss guarantee depends on the two agreeing) fixed now with one shared helper — CLAUDE.md's one place per rule, and a drift would lose chunks — if wrong: one function.
- Ruling: Task 5 Minors 2, 3, 5 fixed now — chunks()'s open failure becomes CantTidy naming the pack, and "which must be kept" isn't said of a young pack's list; json.md:320 adds that an unreadable list in a young pack stops tidy too; the hint (code and json.md) warns that moving a pack out also loses the chunks only its chunk lists name (the plan's wording undersold it; json.md is a living doc, no decision record holds the wording) — if wrong: wording.
- Ruling: Task 5 Minor 4 — add the cheap tests: an old folder named like a pack; tests.rs:658 checks `lists_of(...).is_empty()`; `two_versions` asserts its premise (shared chunks only in the dropped version's pack); a young list through the fault sweep if it fits cheaply. The "details can't be read" stop stays untested (no portable way to make an entry's details unreadable) — if wrong: one rare stop untested.
- Task 5: out-of-scope → Task 13 (docs): json.md:320 and storage-format.md §11 say "if anything that must be kept can't be read … the command fails", but tidy reads only records and chunk lists while marking, never kept file contents (a damaged kept copy gives pack_not_tidied when its pack is copied); decision 077:31 is the accurate version — Ruling: reword both to records and chunk lists in Task 13 — if wrong: one sentence.

### Task 6

- Ruling: Task 6 concerns 1, 4, 5 — decision 089 (refines 085; commit 932d15f, spec before code): `lfs.storage` counts only when set to anything but empty or `lfs`, taken as Git takes it (user's settings, then .git/config, then config.worktree when extensions.worktreeConfig is set, last wins); a `.git/lfs` or `.git/lfs/objects` that is a link or not a folder counts as kept elsewhere too; either stops an import only when it has a Git LFS file to bring in (with a sign: first pass, before writing; without: the first pointer the second pass meets), in place of naming objects missing; the mismatch hint is `git lfs fsck <commit>`, then `git lfs fetch --all` (checked against git-lfs 3.7.0's own help: fsck moves bad copies to .git/lfs/bad and checks only HEAD unless given a commit; lfs.storage's default is `lfs` inside .git) — a global lfs.storage would otherwise block every import on the computer — if wrong: one record and a narrower refusal to undo.
- Ruling: Task 6 concern 2 (a link holding pointer text comes in as a link) — accepted, matches 085 ("a file") and Git LFS; design.md §5.16 now says a file, never a link (932d15f) — if wrong: none.
- Ruling: Task 6 concern 3 (a size-0 pointer counts as not in the exact form) — accepted: Git LFS writes an empty file as an empty blob, never a pointer, so an export couldn't give one back — if wrong: one count.
- Ruling: Task 6 concern 6 — no progress during the first pass's check: deferred to milestone 6 (terminal output); design.md's "Reading Git" bullet naming lfs.storage → Task 13; Repo::lfs_object_path and the import_output.rs placement → the reviewer judges.
- Ruling: an `lfs.storage` line with no value counts as empty (Git LFS's own place) — narrow; a refusal never loses data either way — if wrong: one more stop. Links checked only when the setting doesn't already point elsewhere — accepted. design.md's "Reading Git" bullet (lfs.storage, config.worktree) → Task 13.
- Ruling: Task 6 Minor 1 (the first pass re-reads the header of every blob over 1 KiB in each new tree that holds it: O(files × commits) on a flat folder with a long history) fixed now — one mark per blob seen, dropped before the second pass, whose own memo already holds one entry per blob, so peak memory doesn't rise; decision 085's consequence ("a mark per distinct folder and small file") becomes per folder and file — the rulings file says so — if wrong: some first-pass memory.
- Ruling: Task 6 Minor 2 (a pointer's stop rules written in both passes) fixed now — one helper both passes call (one place per rule; a drift would let the first pass approve what the second stops at after .bolt exists) — if wrong: a refactor.
- Ruling: Task 6 Minor 3 (an I/O error checking `.git/lfs` stops an import with no Git LFS files) fixed now — the error waits for the first Git LFS file, as decision 089 says such an import comes in — if wrong: a rare error shown later.
- Ruling: Task 6 Minor 4 (security.md T31 names only `.git/lfs`; import.rs:1081-1083's doc comment overstates the limits; git/mod.rs:491 garbled) fixed now.
- Ruling: Task 6 Minor 5 — add the four tests (a link holding a pointer's text with its object missing passes the first pass; a size-0 pointer counts and never stops under lfs.storage elsewhere; limits across commits with a skipped subtree; an object one byte longer).
- Ruling: Task 6 spec gap (an `[include]` in .git/config that sets lfs.storage is never read, so objects look missing and the fetch hint can't help) — the missing-objects stop's hint also says, when the repository's config has [include]/[includeIf] sections, that Bolt doesn't read them and that if one sets lfs.storage the objects must be in .git/lfs/objects — decision 043's warning names only sparse checkout and shows only on success — if wrong: one hint line.
- Task 6: deferred (final review / follow-ups): `has_lfs_folder()?` on every import can still fail on a disk-level fault (not permissions); `[includeIf]` in the user's own Git settings isn't followed, so an lfs.storage set there is never seen and the includes hint fires only for the repository's config — Ruling: deferred — the same limit decision 060 already has for every setting read from the user's files — if wrong: one more hint case.
- Ruling (Task 7 ambiguity): when the first save after an import meets both pointer files and line endings Git converted, the pointer stop comes first and nothing is asked — the line-ending question offers a flag to save as is, which can't help while pointers stand in for files — if wrong: two stops in another order.

### Task 7

- Ruling: Task 7 concern 1 (the first save after an import reads each small file the cache doesn't vouch for twice: the pointer check, then the save) — accepted: decision 039's line-ending check already reads those files in its own pass, it happens once per import, and decision 084's one read is about storing; Task 11's benchmarks time a first save after import — if wrong: a slower first save on projects of many small files.
- Ruling: Task 7 concern 2 ("then save again", as 085 and design.md say, not the brief's "`bolt save`") — accepted — if wrong: wording.
- Ruling: Task 7 Minor 5 (the stop names only `git lfs checkout`; once `.git` or its Git LFS objects are gone — say the user deleted `.git` after moving to Bolt — there is no way on, and there is no flag) — the hint adds that `bolt discard` with those paths puts back the files the import brought in (Bolt already holds them; discard keeps the pointer text as a safety copy); json.md and design.md §5.16 say so (Task 13 for design.md) — an additive hint, not a change to decision 085's stop; goes on the please-confirm list — if wrong: one hint sentence.
- Ruling: Task 7 Minors 1-4 fixed now: tests at exactly 1,024 bytes and for a file over 1,024 bytes that must pass unopened; CLI cases for `save --amend` and a partial save with pointers present; drop the hint's unreachable branch; endings.rs's module doc says what a pointer stop means.
- Ruling: Task 7 fix-round concern 1 (a partial first save that leaves the pointer files out goes through, and its save is then the base, so no later save checks and the next full save records pointer text) — fix round 2: on the first save after an import the pointer check looks at the whole folder (every file of at most 1,024 bytes the cache doesn't vouch for), even when the save names paths — decision 085 says the save stops "where the folder holds a pointer", and saving pointer text is never what a user meant — if wrong: a partial save stopped by pointers outside its paths, cleared by `git lfs checkout` or `bolt discard`. The line-ending stop keeps decision 039's narrowing (it records real differences, and it asks); its gap goes to the follow-ups.
- Ruling: a partial amend looks at the whole folder too (an amend drops the save's Git fields, so after a partial amend no later save would check — the same gap) — accepted — if wrong: one condition. A partial first save can now fail on a locked or changing small file outside its paths, as a full first save would — accepted.
- Ruling: the whole-folder look is an exception to decision 054 ("every other rule of a save applies to what is named"), and the discard way out adds to 085's stop — so decision 090 records both (refines 054 and 085; commit 125bcc3, with design.md §5.16, the §7 partial-save note, the agenda and the count; back-links on 054 and 085) rather than a ruling alone — the owner's rule: a decided behaviour changes only through a new record — if wrong: one record to reverse.

### Task 8

- Ruling: `rebuild_family<E: From<GitError>>(id, tmp: &Dir, each)` instead of the brief's ConvertError — ConvertError doesn't exist and ImportError meets From<GitError>; kept files need a folder, and the pack set's tmp is the one that exists — if wrong: a signature change in two files.
- Ruling: family members that are pointer-sized or look like pointers are left to `blob` — what a blob is gets decided in one place; Git doesn't deltify a pointer against a big file, and a hostile pack that does pays a charged chain walk each — if wrong: some extra, bounded work.
- Ruling: a kept file in .bolt/tmp that can't be written or read back means building that object again from the bottom, charged — 086 already builds again past 4 GiB of kept files; a failing disk stops the import at its next store anyway — if wrong: a slow import on a failing disk instead of a stop.
- Ruling: concern 2 (the allowance still stops honest long histories of tiny edits: ~500 versions of an incompressible 8 MiB file, ~200 of a 20 MB text file compressed tenfold, in a repository that is mostly that file; `repack -a -d -f` doesn't help) → decision 091 (commit below): keep the allowance (a security bound with red-team tests), make the message say it may be a long history of small edits or a hostile pack, and make the hint `git -c core.bigFileThreshold=1m repack -a -d -f` (a whole object costs no rebuild work: odb.rs `build` returns early for Bottom::Whole when stored_whole) — please-confirm item: the owner may prefer a higher multiplier or a trust flag — if wrong: one message, one hint, one test line.
- Ruling: the under-1-GiB "0 GiB" display under the test hook stays — only tests reach it.
- Ruling: fix round 1 takes minors 1-6 — cheap while the implementer has the context; hostile-shape tests follow the malicious-repo corpus principle; (5) becomes one key (file) plus a T11 sentence stating the bound, not a size filter (storing only members ≥ 1 MiB would send an honest file hovering near 1 MiB back to chain walks and could stop it at the allowance) — if wrong: one round of churn.
- Ruling: minor 7 deferred to the final review list — shrinking the test safely needs the base code re-measured; 40 s per CI platform is acceptable — if wrong: CI time.
- Ruling: GitError::Bug added (listed in json.md) — no existing variant said "a bug in Bolt"; lost() stays in odb.rs (build() uses it); a family's bottom walk now stops at 4,096 deltas (the read cap) — all accepted — if wrong: small.
- Ruling: the memo bound (~3 GB at 2^24 members, 4.5 GB while the table doubles: tiny members add more allowance than they cost) is not acceptable as a stated bound — it is 3-4x the reader's own 1 GiB tables and a red-team finding in waiting → decision 092 (commit 8620778): a family rebuild stores and maps only members of at least 1 MiB (is_cut), smaller ones are built only as bases and left to the cache path; each stored member costs ≥ 1 MiB of allowance, so at most one per MiB of it; the closure's LFS check goes (pointers are < 1 MiB). T11 and design.md updated by the controller — if wrong: honest files hovering near 1 MiB read their smaller versions through the cache (as before 086).
- Ruling: keep the code, correct the record → decision 093 (commit 484e6ca; design.md and T11 updated) — skipping childless small members needs each one's size from the start of its delta, which for tiny hostile deltas costs as much as building it; the owner's rule forbids editing 092 — if wrong: one record.

### Task 9

- Ruling: concern 1 (a valid list of 8,388,608 one-byte chunks takes 82 s release / 142 s debug on Windows to write its 8 MiB file; ~0.3 s per MB of list) — accepted as a bounded cost of a planted .bolt (lists from imports and saves come from Bolt's own cutter, ≥ 16 KiB chunks) → T11 states it (Task 13); follow-up row (Task 12) with the two cheap speed-ups the implementer named (buffer write-out's temporary file; reuse a chunk's copy when the next chunk is the same) and a format-1 option to bound a list's count by its file's size; attack-surface item for Task 14 — measure first: no decision asks for speed here — if wrong: a red-team finding.
- Ruling: concern 2 (a switch refused by damage holds the project until the store is fixed: the journal stays, every later changing command meets the same damage, SwitchStopped's hint offers no way out) — pre-existing since milestone 3 (decision 049), not caused by chunk lists → follow-up row (Task 12) + please-confirm item: check every chunk list a switch needs before its journal is written (structure only), and a SwitchStopped hint that names the way out; attack-surface note for Task 14 — if wrong: owner's call later.
- Ruling: concerns 3-5 accepted (renamed milestone 3 test → reviewer checks it still covers its old property; listings stores files from 1 MiB as chunks; testing.md §3/§4 → Task 13).
- Ruling: fix round 1 takes minors 2-5 (test-only, cheap; 2 makes a security property an assertion instead of a crash); minor 1 is report wording — skip.

### Task 10

- Task 10: dispatched (BASE bca0558). Ruling: the implementer may push its HEAD to bench/m5 (and only there, never force) to run bench.yml's memory gate on Linux, then delete bench/m5 once the numbers are copied; Task 11 pushes it again — the plan mandates the branch and the controller's "don't push" rule exists to keep milestone-5 tidy — if wrong: a stray branch to delete.
- Ruling: `blake3 = "1.8.7"` in bolt-vcs's dev-dependencies is not a new dependency — Cargo.lock gains no package (one line in bolt-vcs's dependency list), same crate, version and default features bolt-vcs-core already uses; decision 088 asks for BLAKE3's extendable output — please-confirm item — if wrong: swap to a writer in core's test-support.
- Ruling: fix round 1 takes the Important and minors 1, 2, 4, 5; the duplicated job becomes one reusable workflow (workflow_call) in Task 11, which rewrites bench.yml — carry → Task 11.

### Task 11

- Task 11: dispatched (BASE ba513aa). Rulings: (R1) RT4-1's repository is attack code (redteam4.rs builder; ruled, not fixed) — it is built only on a `redteam/m5-sizes` CI branch, never on bench/m5 or the laptop; (R2) the implementer may push HEAD only to bench/m5 and redteam/m5-sizes, never force, and deletes both after copying numbers; (R3) the memory gate becomes one reusable workflow (workflow_call) used by bench.yml and nightly.yml; (R4) if a compression level qualifies under 088's rule, stop before code and report — the controller writes the decision record and storage-format change first; (R5) carried: a repetitive big file, a first save after an import, git/git main vs milestone-5 on one runner alternating 3 runs each (>10% slower is a finding), benchmarks.md:29 wording.
- Ruling (Task 11 concern 1): the git/git import's peak rose ~100 MB (560 → 660 MB). DELTA_BASE_CACHE is 256 MiB on both main and milestone-5; since decision 086 the cache also keeps the objects it builds (put_copy), so it fills closer to its budget. Within the documented 256 MiB (design.md §5.16), not a gate; benchmarks.md names that as the likely cause once the reviewer confirms it from the code — cost if wrong: an unexplained ~100 MB the owner still sees published.
- Ruling (Task 11 concern 2): where Git wins (status 62 vs 13 ms and small saves ~2× on small projects; going back to an old 1 GiB version 15–40% slower on Linux; Godot 3% and the 200 MiB × 50 history 18% more space) is published as measured; Task 12 adds one follow-up row (start-up cost of status/small saves; the space gaps as chunking's trade-off against deltas, decision 082) — cost if wrong: none, a row.
- Ruling (Task 11 concern 3): the fairness choices (wait out Git's background repack, sync before each timed run, files aged 3 s before the status steps, syncing differences stated) refine decision 088's method without changing it (warm caches, 5 runs after a warm-up, 3 over a minute); no new decision; please-confirm item — cost if wrong: a re-run with other choices.
- Ruling (Task 11, compressing again): no level qualifies under decision 088's rule (level 9: 2.6% at 4.4×; level 19: 8.8% at 53.5×; both runners agree). Merging and tidy keep copying stored bytes; follow-ups row 91 is closed by Task 12 with a pointer to benchmarks.md §5. No decision record: applying 088's rule decides nothing new — cost if wrong: one record later.
- Ruling (Task 11, bench.yml trigger): Bench must not fire on every redteam/** push (Task 14 and later red teams push redteam/m5 for Windows attack tests); narrow the push trigger to redteam/*-sizes, keep the job gates — fixed in the fix round.
- Ruling (Task 11 review): fix all ten, plus two checks the reviewer couldn't make: cite or soften §2's outside claims (Git's core.fsync default, Git LFS not syncing, Git for Windows' foreground repack), and verify §3's "reads uncached files twice" against snapshot.rs/endings.rs — accuracy of a published page beats its length — cost if wrong: a docs-only round.
- Ruling: fix all of them in round 2, Godot's space included (one clause, same paragraph) — cost if wrong: none.

### Task 12

- Task 12: dispatched (BASE ed32f27), implementer ac88b9db2ec3022d6 (Opus). Rulings carried: strike rows 29, 48, 50, 91, 94, 109 (each checked first) and the write-limit owner line; old TooMuchWork hint → 091's; row 44 gets 573 → 669–680 MB; row 43 gets git/git's three .bat files; row 26 padding: ruling only (stays before v0.1; a reading bound needs its own decision and spec first); line 142 checked-copy memory: ruling with the memory gate's 43.4–43.5 MB at ~164k chunks; new "Deferred by milestone 5" section (lists of lists over 128 GiB, .gitattributes-exact LFS and fetching, records as deltas, canvas review → docs/plans/2026-11-09-terminal-ui-canvas-review.md, first-pass progress, partial first save's line-ending check, tiny-chunk lists, a switch held by damage, the allowance vs long histories, where Git wins, two should-dos); code: endings.rs:25 comment, a partial-amend whole-folder pointer test (fix in snapshot.rs if it fails), convert.rs:997 comment.
- Ruling (Task 12 concern 1): the controller's ruling 6 was wrong — the memory gate saves only new chunks, so it doesn't measure the checked-copy set; accepted the implementer's row: 40 bytes an entry since 2026-11-05, about 690 MB at 10 million objects, an estimate, row stays open — cost if wrong: none, a row.
- Ruling (Task 12 concern 2): the controller's ruling 5 premise was wrong — files streamed whole (every saved file before milestone 5, files over 128 GiB now) choose compression from their first 64 KiB, so a copy can be slightly bigger than its recorded size; "stored under recorded" would break good copies. Row 26 stays open as written — cost if wrong: none.
- Ruling (Task 12 concern 3): the canvas review's prompt facts updated to decisions 089/090 accepted; the review file names the milestone 5 stops the prompt doesn't yet cover (tidy at a pack it can't read; the import's work stop) — fixed in the fix round if the reviewer finds anything else, else by the controller.
- Ruling: the controller fixes the 7 Minor doc points itself (docs only, no code) in one commit — cheaper than a round, and each is a checked fact from the review — cost if wrong: a wording slip the final review catches.

### Task 13

- Ruling (Task 13 concern 1): tidy.rs:67-69's comment carries the old "anything that must be kept" wording — fixed by the implementer in the fix round (a comment, no behaviour) — cost if wrong: none.
- Ruling (Task 13 concern 2): storage-format.md §13 has promised a `.bolt/format` vector since milestone 1, and none exists in tests/vectors/ (repo.rs unit tests pin the bytes). Milestone 5 changed that file (`feature chunks`), so add tests/vectors/format.json and a check in tests/vectors.rs in the fix round, keeping the spec's promise rather than weakening it — cost if wrong: one small vector file.
- Ruling (Task 13 concern 3): the extra "Hostile lists of tiny chunks" item in design.md §7 stays — one of its options changes the format, and §7 is where format-touching deferrals live — cost if wrong: one line.
- Ruling: the red-team sandbox gains git-lfs (Debian's package; one word in redteam/Dockerfile, commit 8744283) — milestone 5's attack surface is largely Git LFS import, and import_lfs's tests skip without it; Bolt itself never runs Git LFS — cost if wrong: one word.
- Ruling: the red team starts while Task 13's docs re-review runs — it attacks code, which Task 13's fix round left unchanged apart from a comment and a test vector — cost if wrong: none. Docker Desktop started for Task 14.

### Task 14

- Ruling: redteam8_windows.rs joins the suite as ordinary tests — they assert behaviour that already holds, like the corpus tests — cost if wrong: three Windows tests in CI.
- Ruling (fixes concern 1): Git LFS reads settings through `git config -l`, which reads only the file `GIT_CONFIG` names when that is set; Bolt doesn't read GIT_CONFIG and 094 doesn't name it — deferred to a follow-up row (GIT_CONFIG is a legacy variable for `git config` alone, rarely set) — cost if wrong: a misleading stop in a rare setup, the same class as RT8-1.
- Ruling (fixes concern 2): pre-existing gaps — Git 2.56's `:(optional)` path prefix, and `core.excludesFile = /dev/null` stopping a save (decision 047 refuses a non-regular list) — follow-up rows, Before v0.1; not red-team findings, and the /dev/null one needs its own small decision (it changes 047's "any other problem stops the save") — cost if wrong: a user with that setting can't save until it's fixed, as since milestone 3.
- Ruling (fixes concern 3): rare mismatches (`..` in GIT_CONFIG_SYSTEM, relative paths in these variables, no home folder) — one follow-up row — cost if wrong: none.
- Ruling: decision 095 (41d66c3, written before the code) widens Important 1 — the exact text `/dev/null` (and `nul` on Windows, any case) names no file, judged before any join, for core.excludesFile (which has stopped every save with that setting since milestone 3), GIT_CONFIG_SYSTEM/GLOBAL and includes; §12 updated (Minor 8 done by the controller). The follow-up row for core.excludesFile=/dev/null is dropped: 095 fixes it — cost if wrong: one rule.
- Ruling (round 1 concern: unreadable system file): Bolt stops there, as Git v2.56.0's status and `git config -l` paths die (config.c:2299-2328); only the user's own files are skipped on EACCES (config.c:1592, :1596) — matches Git — cost if wrong: none.
- Ruling (round 1 concern: blob too long to cut): no new test — that path needs a blob over 64 MiB and no hook lowers it; the re-review confirms it uses the same check — cost if wrong: an untested branch of one call.
- Ruling (round 1 concern: Windows permissions): Git for Windows dies on a settings file whose ACL denies reading (its access() sees no ACLs), so Bolt must stop there too on Windows rather than skip — a silent skip applies a different personal list, RT8-2's harm; Unix keeps the skip, as Git — sent as a small fix before the re-review — cost if wrong: a stop on a locked ~/.gitconfig, which Git gives too.
- Ruling: decision 096 (4ba1778, before the code): GIT_CONFIG_SYSTEM tidied as Git's normalize_path_copy does before judging and opening; where Git's tidying fails Git goes on half-tidied (config.c:1513 ignores the result, checked in fetched source), Bolt stops; 096 also corrects 095's context (Git documents /dev/null only for the two variables; 095's rule stands). §12 now names which unreadable files are skipped and the %(prefix)/ stop. EPERM → stop (skip raw EACCES 13 only). Item 3 left (harmless) — cost if wrong: one more small port.
- Ruling (round 2 concern): on Windows a GIT_CONFIG_SYSTEM that isn't valid Unicode stops the command — pathological, accepted (fail closed) — cost if wrong: none.
- Ruling: §12 and row 146 fixed by the controller (8475ef3, docs); the config.rs message wording and comment nit are parked for the final review's single fix dispatch — cost if wrong: one message's wording until then.
- Ruling: rt9_1 must never write the real home (it wrote the account's ~/.gitconfig in the sandbox); it proves the rule with a no-HOME run plus the lookup's unit tests. redteam9_windows.rs joins the suite with the fix. — cost if wrong: a weaker end-to-end proof on Unix, covered by unit tests.
- Ruling (RT9 concerns): rt9_1 weaker than the red team's proof — accepted per the earlier ruling (unit tests cover the account-home case); RT9-2 keeps Git LFS's exact test alongside the looser one (union) — right; the warning's "may" — right; an empty HOME read as `/` (as Git for `~/x`) and a bare `~` stopping where Git reads nothing — accepted, fail closed — cost if wrong: none.
- Ruling: decision 098 (32bf9cb, before the code) — also run Git LFS's own parser (lfs_pointer) on the first 1,024 bytes of the blob's CRLF form, for every blob not imported as a Git LFS file, any size; union with 094's and 097's tests; ident and working-tree encodings stay a limit. 097's "a conversion only pushes bytes out" was wrong for these shapes. Important 2 is a bug against 097 (stop at the first `~`-led path value, excludesFile and include.path) — no new decision — cost if wrong: extra warnings only.
- Ruling (round 1 concern: `~\x` on Windows): upstream Git v2.56.0 accepts only `~/` (path.c:710) and stops on `~\x`; Bolt has expanded `~\x` since milestone 3 (a0dc290); Git for Windows' own patches unchecked — a follow-up row (Before v0.1), not changed now — cost if wrong: Bolt applies a list where upstream Git stops, a rare spelling.
- Ruling (round 1 concern: a bad line after a `~/` value): reported instead of the `~/` stop; both stop — accepted — cost if wrong: a different message.
- Ruling (out of scope: a repository's own `~/` core.excludesFile with no home makes Git die, Bolt saves): as decided — decision 047 never reads core.excludesFile from the repository (a security rule) — no row — cost if wrong: none.

### Task 15

- Rulings (final review): fix I1, I2, M1–M5, M7–M9; M6 only if it stays one ingest.rs function and the four call sites, else a "Should do" row; carried lines 135, 142, 145 become "Should do" rows; 108 and 195 stand by earlier rulings; 239 (a 40 s test) accepted, CI time only; 271 already a row; the reviewer's "declined to judge" list stands (all decided limits or deferred rows; blake3 dev-dependency = no new crate; decision-number comments follow main's convention) — cost if wrong: one more round.
- Ruling (final fixes concern 1): `build_delta` (git/pack.rs, on main since RT4-4) reports running out of memory as damage with the `git fsck` hint — pre-existing, not in the review — a follow-up row (Before v0.1), added with the rulings file — cost if wrong: a misleading hint on an out-of-memory import.
- Ruling (concerns 2–4): the T11 placement and extra figures (re-review checks them), the phase-line update, and one redundant index lookup in copy_in — accepted — cost if wrong: one lookup per too-long file.

## Deferred

What milestone 5 left for later, each a row of [the follow-ups list](2026-11-01-milestone-1-follow-ups.md#deferred-by-milestone-5-2026-11-09), given here by its first sentence. Rows that would change the storage format are also in design.md §7.

- **When real projects need it:** Chunking files over 128 GiB.
- **When real repositories need it:** Git LFS files judged by `.gitattributes`, as a Git checkout judges them.
- **Known limit:** RT9-2 the other way round: Git converts line endings before Git LFS's smudge filter reads a blob's first 1,024 bytes, so a pointer of at most 1,024 bytes whose size line comes after enough blank lines is, under `eol=crlf` or `core.autocrlf=true`, checked out by Git LFS as its text (a 1,017-byte blob in the red team's case), while the import brings in the file it names, checked against its SHA-256.
- **Known limit:** Git's other conversions before Git LFS's smudge filter: `ident`, which grows each `$Id$`, and a working-tree encoding (`working-tree-encoding`) change a file's first 1,024 bytes too, but the `lfs_lookalikes` warning models only the CRLF form, the one Git for Windows turns on by default ([decision 098](../decisions/098-a-blob-is-also-judged-in-its-crlf-form.md)).
- **With network code:** Bolt fetching Git LFS files itself.
- **A later storage feature:** Records stored as deltas (decision 009's later `feature deltas`).
- **Milestone 6:** The terminal-UI design.
- **Milestone 6:** Progress during an import's first pass.
- **Known limit:** The line-ending check of the first save after an import keeps to the paths a partial save names, while its pointer check looks at the whole folder ([decision 090](../decisions/090-the-first-save-looks-at-the-whole-folder-for-pointers.md)).
- **Before format 1 freezes at v0.1:** Hostile lists of tiny chunks.
- **Before v0.1:** A switch stopped by damage holds the project (so since milestone 3).
- **If real repositories hit it:** The import's work allowance against honest long histories of small edits to one big file.
- **Before v0.1, measure:** Where Git wins ([benchmarks.md](../benchmarks.md) §2, and §3 for the imports).
- **Should do (tests):** The corpus tests' other `bolt --json` runners have no timeout, so a read that looped would hang the test rather than fail it: `run` in `corpus_diary.rs`, and three `goto` runs in `corpus_writeout.rs`.
- **Watch (Windows):** The undo test `a_redo_that_fails_after_a_step_lists_that_step_in_its_document` failed once on the Windows laptop with "Access is denied" on `.bolt/state/head`, and passed three times when run again; never in CI.
- **Measure:** A big file whose chunks are spread over many packs reads slowly: at most 16 packs stay open, and a chunk in a closed pack opens the closed ones by name until found.
- **Red team:** Several bad chunk lists for one file multiply its read: each is tried once per command, and one whose chunks all exist but build another file is read to its end before its final check, so N planted lists cost N reads of the file on every `goto`.
- **Red team:** A link below `.git/lfs/objects` (at `objects/ab`, say) makes its objects missing, so the import's stop says to fetch, where [decision 089](../decisions/089-git-lfs-objects-kept-elsewhere.md) names only `.git/lfs` and `.git/lfs/objects`.
- **Red team:** Invalid UTF-8 in two extension keys: `lfs_pointer` reads a pointer's text lossily, so `ext-0-a\xff` and `ext-0-a\xfe` become one key, where Git LFS sees two keys with one priority and refuses the pointer.
- **Red team:** `tidy`'s memory grows by about 40 bytes for each distinct chunk ID it keeps, on top of one decoded list at a time: a planted young list naming 8,388,608 distinct IDs adds about 320 MB until `tidy` ends.
- **Red team (Windows):** A program can set a file's change time back: a file changed during a save, then given back all its old times, isn't seen.
- **If real users hit it:** `GIT_CONFIG` isn't read.
- **Before v0.1:** Git 2.56's path prefixes aren't read.
- **Known limit:** Rare differences from Git in where the user's settings are read ([decision 094](../decisions/094-red-team-m5-rulings.md)); the path `GIT_CONFIG_SYSTEM` names is now tidied as Git tidies it, `..` included ([decision 096](../decisions/096-git-config-system-is-tidied-as-git-tidies-it.md)).
- **Before v0.1:** `GIT_CONFIG_PARAMETERS` isn't read.
- **Known limit:** Git LFS reads Git's settings by lines: it runs `git config --includes -l` without `-z` and splits at newlines, so a planted value holding an escaped newline (`core.foo = "x\nlfs.storage=/elsewhere"`) gives Git LFS an `lfs.storage` neither Git nor Bolt sees (`git lfs env` said `LocalMediaDir=/elsewhere/objects`).
- **Known limit:** Odd spellings of the null device stop Bolt where Git reads an empty file: `GIT_CONFIG_GLOBAL=/dev//null`, or `core.excludesFile` set to `/dev//null` or `/dev/zero` (`config-diff.sh`).
- **Known limit:** The first save after an import doesn't see a pointer whose checked-out CRLF text passes 1,024 bytes: its pointer look reads only files of at most 1,024 bytes, so it takes the pointer text Git LFS checked out (RT9-2's other way round) as a change over the file the import brought in, which stays in the history.…
- **Before v0.1:** On Windows, Bolt reads `~\x` in a `core.excludesFile` or an `include.path` as the home folder's `x`, taking `\` as a separator, as it has since milestone 3 (`a0dc290`).
- **Before v0.1:** Two places in the Git reader still give the wrong hint when the computer runs out of memory: `build_delta` (git/pack.rs) reports a failed allocation for a delta's result as damage, with the `git fsck` hint, as it has since RT4-4; and `odb.rs` maps a raw delta read error straight to an I/O error, with the permissions hint.
- **Should do:** `snapshot.rs` grew about 250 lines of reading glue in milestone 5 (`At`, `Job`, `Done`, `Jobs`, `read_small`, `read_scanned`, `build_reading` and `store_read`).
- **Should do:** `Packs::list`'s `retain` and `close_all`, in `store.rs`, drop open packs without checking that no lookup still reads them.
- **Should do:** A copy's own read error names no pack.

Also left as they are, by ruling:
- **A chunk list already proven good is streamed with no fallback.** A passing read error fails that one read, nothing is written, and the next command reads again. This is the same rule milestone 3 set for a whole copy.
- **Every import checks for a `.git/lfs` folder,** which can fail on a disk fault.
- **The long-history import test takes about 40 s** in a debug build on each CI system.
