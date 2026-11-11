# Red-team report: milestone 6, presets and polish, second session (decision 024, session RT11, 2026-11-10)

Code attacked: a copy of `milestone-6` at 4102eea ("Make bolt verify report a chunk held only as another kind, a listing that gives a file another size, and a save too big for any reader, as decision 126 says"), with the three fixes of the first session (RT10) in.
Proof tests: `crates/bolt-vcs/tests/redteam11.rs`. 1 test, marked `#[ignore = "RT11-1: …"]`, asserting the SAFE behaviour. It was run in the no-network sandbox and fails as claimed. It runs on Linux only, because it caps the address space with `ulimit -v` and reads peak memory from `/proc`.
Run it: `cargo test -p bolt-vcs --all-features --test redteam11 -- --ignored` (or `redteam/attacks10/run.sh proofs`, which runs it in the sandbox). In the sandbox, on the code attacked, it ended `test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 497.64s`. Without `--ignored` the file passes (1 ignored). The file is formatted with `rustfmt --edition 2024` and passes `cargo clippy -p bolt-vcs --all-targets --all-features -- -D warnings`.
Windows checks: `crates/bolt-vcs/tests/redteam11_windows.rs` (`#[cfg(windows)]`, not ignored): a junction named like a pack, and a junction at `.bolt/state/tidied`, each leading out of the project, through `verify` and `tidy`. **They have not run**: they need a Windows machine (CI on a `redteam/*` branch). They were formatted in the sandbox, but not compiled or linted for Windows there.
Attack code: `redteam/attacks10/`. `run.sh explore <name>` copies one Rust exploration into the tests and runs it in the sandbox; `run.sh script <file>` runs a shell probe against the debug build with fault injection; `run.sh proofs` runs the proof file.

Counts: Critical 0, High 0, Medium 1, Low 0.

What held: the three fixes are right. I couldn't fool them (sizes, kinds and the save limits are checked the way the readers check them, at the limits and one past), couldn't make them cry wolf on any honest project Bolt's own commands built, and `tidy` never runs them. `verify` running beside a loop of saves, undos, shelves, `goto` and tidies that rewrite every pack found no false problem in 558 runs. Crash sweeps of `bolt start --preset` and of `bolt undo --to` after a lost `head` lost nothing. The one finding is about cost, not correctness: **the new size check keeps the name of every file it finds with a wrong size, in each of its two looks, though `verify` reports at most 1,000 problems**, so a small planted store makes `verify` run out of memory where `tidy` and `goto` read the same listings in a constant 91 MiB.

## Findings

- **RT11-1 Medium (a bounded resource blow-up; T7, T11 and T35, decision 126).**
  - **What.** Think of a clerk who must report every wrongly labelled box in a warehouse but may hand in only the first thousand notes: this one writes a full note for every box anyway, keeps all of them, and then copies the whole pile twice more. A small planted store with many wrong labels fills the clerk's desk before the report is written.

    In detail: `Noted::listing_read` (`tidy.rs`) pushes `(listing, name.clone(), listed, stored)` into `noted.sizes` for every file entry whose size isn't the stored one, with no cap. `look_at` (`verify.rs`) moves them all into a `BTreeSet<Finding>`; `agreed_needs` keeps each look's verdict in `verdicts`, so a first look that found anything triggers a second that builds a second full set while the first is held, and `Verdict::with` then clones their intersection. `told_needs` keeps only `PROBLEMS_KEPT` (1,000) of them. So `verify`'s memory grows with the number of wrong-sized *entries* summed over every listing, about three copies of each name (up to 765 bytes), not with the number of objects (the known limit). Listings are read one at a time and freed, which is why the same store with right sizes stays flat. Measured with the debug build in the sandbox, 50,000 files of 765-byte names per listing, all naming one 1-byte content:

    | listings | `verify`, right sizes | `verify`, wrong sizes | `tidy`, wrong sizes |
    |---|---|---|---|
    | 1 | 91 MiB, 7.9 s | 179 MiB, 16.6 s | 90 MiB, 4.1 s |
    | 2 | 91 MiB, 17.9 s | 269 MiB, 35.2 s | 91 MiB, 9.0 s |
    | 8 | 91 MiB, 59.6 s | 1,043 MiB, 154.5 s | 91 MiB, 37.9 s |

    About 119 MiB more per 50,000 wrong entries, about 2.4 KiB per entry, and no ceiling. The store is a handful of objects and compresses to almost nothing; 30 such listings would take a 4 GB machine down.
  - **Who is hit.** The person who runs `verify` on a project they were given (a copied `.bolt`, later a server's), which is the case decision 126 was written for. `verify` is killed or aborts instead of saying the project is damaged; on a machine without a cap it can push everything else into swap first. `tidy`, `goto` and the rest read the same store in constant memory.
  - **Proof (test).** `rt11_1_verify_reports_many_wrong_sizes_within_bounded_memory`: 12 listings of 50,000 wrong-sized files, run with the address space capped at 1,000,000 KB. The same store with right sizes passes `verify` under the cap, and `tidy` on the planted one finishes under it; `verify` on the planted one aborts.
    ```
    bolt verify, address space capped at 1000000 KB: code Some(0) in 135s, peak 92 MiB:
    bolt verify, address space capped at 1000000 KB: code None in 177s, peak 962 MiB: note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
    bolt tidy, address space capped at 1000000 KB: code Some(0) in 57s, peak 91 MiB:
    assertion `left == right` failed: verify should report the wrong sizes, not run out of memory
      left: None
     right: Some(1)
    ```
  - **Fix.** Keep what is told bounded and what is counted cheap: in `Noted`, keep the full `(listing, name, listed, stored)` for at most `PROBLEMS_KEPT` wrong sizes and count the rest (with the listing ID only, 32 bytes, if the two looks must agree on them; or a running hash of them). Make `Verdict::Found` compare looks by those bounded sets and counts, and drop each look's set once it has been compared rather than holding all of them in `verdicts`. The same applies, more mildly, to `noted.unread`'s one `(id, OtherKind)` per entry, and to `noted.shapes`, which keeps a `(usize, Id)` for every folder entry of every listing until the look ends (about 40 bytes per folder entry, not bounded by the number of objects either). A test like the proof's, with the count of wrong entries raised until the old code passed a cap, would pin it.

## Held up

Each of these was tried in the sandbox unless marked as code reading, and Bolt behaved correctly:

- **The three new checks, fooled** (code reading of `verify.rs`, `tidy.rs` `Noted`, `Held`, `chunks`, `store.rs` `Wanted`, `listing.rs` `SaveLimits`, the readers in `snapshot/walk.rs`, `snapshot/only.rs` and `diff.rs`):
  - A chunk held only as another kind (a chunk list, a record): `chunks` counts it present only with a `Content` line; a record can't share a content ID, since records are hashed with BLAKE3's `derive_key` and content with plain BLAKE3.
  - A wrong size reached only through a shelf, a tag list, server lines, an undo step, a cut-off diary end or a save's parent: every one of them goes through `Marks::follow`, which calls `listing_read` on every listing it reads.
  - An index line that lies about a size: `read_chunk_list` and `copy_content` check it against the object (`pack.rs` lines 943 and 980), so a lying line is told as damage; a chunk list's sizes must add up (`chunks.rs`); a chunk named with another size than its content fails the rebuild.
  - The limits: `too_big` counts entries, depth (the top listing is level 0) and path length (names joined by `/`) exactly as `snapshot/walk.rs` does, so a save at exactly 10,000,000 entries or 4,096 bytes passes and one more fails in both. It is memoised per listing and walked with its own stack, so a chain as deep as a save can hold costs one pass.
- **No false alarms** (`attacks10/honest-verify.sh`, `honest-lfs.sh`): `verify` after every step of: an import from Git with links (to a file, to a folder, out of the project, a 200-byte target), an executable whose bit was later removed, an empty file and a 3 MB file; saves of a 1.5 MB file with `chunk-files-over` at its default, at 1 MiB and at 64 MiB, a file of exactly 1 MiB and then 1 MiB + 1; shelves, unshelve, undo and redo of a shelf, a second line, `discard` and its undo, `save --amend`, `setup --preset code` and its undo, `tidy` before and after every pack was aged three days, `start --preset code` beside a link and an executable, `setup --preset game`, a 5 MB save, its undo and a tidy; an import with Git LFS files (present, and a submodule, and a `text=auto` CRLF file). No step was reported as a problem.
- **`verify` beside changes** (`attacks10/verify-race.sh`, 240 s): one loop saving, undoing, shelving, going to `main`, aging every pack and tidying (so each tidy rewrote them all), 472 rounds; beside it, `verify` 558 times. Not one run reported a problem, and the last one was clean.
- **`tidy` unchanged on planted stores** (code reading: `Noted` is `None` for `tidy`, so `listing_read` and the new checks never run in it; and RT11-1's store): `tidy` finished without error in constant memory.
- **`state/tidied`** (code reading of `tidied`, `record_window`, `safe_fs::read_capped_in` and the platform crate's `open_file`): a pipe there is opened with `O_NONBLOCK|O_NOFOLLOW` and refused, never waited on; junk, a huge number (fails to parse) and a future time count as absent; a past time earlier than the window changes nothing (`max`). A time just before now shortens the window to what that tidy says it freed, which is what decision 123 asks; only someone who can already write `.bolt` can plant it.
- **Diary ends with missing steps** (code reading of `Marks::diary`, `step_entry`, `mark`): a missing undo target or its parent on a cut-off end is passed over with the warning; on the current diary it stops `tidy`. `walked` ends every loop, and ends only add to what is kept.
- **Crash sweep of `bolt start --preset code`** (`attacks10/start-crash-sweep.sh`) in a folder with a 2 MB file and a `.boltignore` with a byte order mark, CRLF, an old preset block and no final line feed: stopped at each of its 22 abort points, then `status`, `start` again, `status`, `verify` and a save. Every time the files were untouched, the user's `.boltignore` lines were still there, and the project ended usable with `verify` clean.
- **Crash sweep of `bolt undo --to` after a lost `head`** (`attacks10/pickup-crash-sweep.sh`): a `discard` stopped at `switch:changed` (the saved file written over unsaved work), `head` deleted, then `undo --to` that entry stopped at each of its 8 abort points, run again, `status`, `verify` and `tidy`: every later command finished, `verify` was clean, and the unsaved work was still in the store.
- **Progress over a question** (code reading of `main.rs`): every question (`ask_large`, `ask_line_name`) pauses the turning mark first and resumes it after; `setup` and `start` ask before their mark starts.
- **Preset detection** (code reading of `presets::look`): names in the top folder only, `open_dir` without following, no file opened, at most 20,000 names met, at most 128 levels; planted links named like engines' files are seen as links and count for nothing (its own test covers that).
- **`git-words`** (code reading): looked at before it is opened, so a pipe or a folder is passed over; read with a 4 KiB cap; only whole lines that are shortcut words count, so escape codes in it are never printed.

## Suspicions without proof

None of these counts as a finding.

- **A deep, narrow tree bomb, measured.** A chain of 24 listings of two folders each, ending in a listing of two files (2^25 entries by path, 27 objects), release build in the sandbox (`attacks10/rt11_deep_chain.rs`, `RT11_BOLT=/work/target/release/bolt`):

  | command | result | time | peak memory |
  |---|---|---|---|
  | `bolt verify` | exit 1, `save_too_big` | 0.1 s | 4 MiB |
  | `bolt changes --save main` | exit 1, refused | 201 s | 5 MiB |
  | `bolt goto main` | exit 1, refused | 517 s | 551 MiB |
  | `bolt tidy` | exit 0 | 0.2 s | 5 MiB |

  So `verify`'s new count is cheap, and the cost the first session saw in a debug `goto` is real in a release build too: almost nine minutes and half a gigabyte for an 8 KiB store, and over three minutes for `changes`. Each walk reads a listing once per path that reaches it until the 10,000,000-entry count trips. Decision 126 lists this as a known cost; the numbers are worse than the debug figure written there. A walk that first measures the save by listing, as `too_big` now does, would refuse it at once.
- **`verify`'s baseline on wide listings.** With right sizes, 5 listings of 200,000 files of 765-byte names (each listing about 160 MB uncompressed, within the 512 MiB record limit), `verify` in the debug build was still on its first look after about 25 minutes and its resident memory had reached 1.7 GB when I stopped it. Listings that size cost every reader a lot; I didn't measure `tidy` or `goto` on them, nor a release build.
- **`safe-folders` judged by path, then the project opened by path** (code reading, `repo.rs`). `check_owner_with` asks who owns `root` and `root/.bolt` by path (lines 537 to 539), and `owner_allows` takes the project's identity from another open of `root` (line 599); `open_untouched` then opens `root/.bolt` by path once more (line 1290) and holds that handle from there on. Someone who can rename entries in a folder above the project (a group-writable parent, say) could swap the project between the check and the open. Whoever can do that controls the project anyway, as with Git's `safe.directory`; it can't be tried here (one account, no capabilities). Opening `root` once, judging the owner and identity from that handle, and opening `.bolt` through it would close it.
- **`git-words` replaced by a file when it was a link** (code reading, `git_words.rs` `remember`): the rename writes a regular file over a dotfile manager's link. The user's own file, not a project's; harmless but surprising.

## Not attacked

- `safe-folders` with another account's folders, bind mounts or drives without file numbers (one account, no capabilities in the sandbox).
- Preset detection at a million names (read, not run; the cap of 20,000 names met bounds it, but `top.entries()` lists the top folder whole first, a known limit).
- zstd bombs in packs against `verify` (the first session's suspicion stands).
- Wide and combining characters in the new rows, and the terminal outputs at widths 1 to 500 (the first session covered them; nothing in this commit changed them but the problem lines, which use the same escaping).
- The release workflow (unchanged since the first session's review).
