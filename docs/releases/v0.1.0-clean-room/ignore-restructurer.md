# Task 12a fix report: restructuring parts of `ignore.rs`

**Status:** DONE_WITH_CONCERNS. Commit `b3cf781` (on `v0.1-engine`, not pushed).

## What changed (`crates/bolt-vcs-core/src/ignore.rs`)

1. **Bracket parser, now two passes.** `members(text)` reads the body into a list of `Member`s (`Byte`, `Range`, `Class`). It works element by element and reads ahead: a class `[:name:]` (helper `class_at`), or else one byte (helper `member_byte`, which handles the `\` escape), followed by `-x` when the next byte isn't `]`, in which case it reads a range. There is no "previous byte" state and nothing is cleared. A leading `]` is a member, and a leading `!` or `^` turns the set over. `set_of` then builds the set from the members. A range's first byte also counts on its own, which is what makes a backwards range like `[z-a]` still take `z` (an existing test describes this).
2. **Trailing spaces.** `without_trailing_spaces` counts the spaces at the end of the line, then the `\` just before them, and keeps the first space only when that count is odd. The branch for a `\` that ends the line, its doc line, and the test that only described that branch are gone.
3. **Set memory.** `Set` is now two `Bits([u64; 4])` (the exact and folded forms), 64 bytes in all instead of 512. `Bits::add(lo, hi)` sets a range with one mask per 64-bit word. Classes are still filled byte by byte over 0..=0x7f. The folded form is the exact form plus the lower-case copy of every A–Z that a range or class takes.
4. **Unicode form of folders.** New `InForce::composed_name` (NFC where the drive composes, borrowed otherwise). `composed` now uses it. `enter` applies it to each rule file's folder and also to the level's own folder, and `leave_to` applies it to its argument, so all three use the same form.

## Tests (`crates/bolt-vcs-core/src/ignore/tests.rs`)
- Replaced the dropped branch test with cases for the new rule: `c\\\   ` matches `c\ ` and not `c\   `; `d\\\\  ` matches `d\\` and not `d\\ `.
- Added range cases across the whole ASCII range, a negated range, and byte cases above ASCII (`[\x7f-\xc1]`, `[!a]` against `\xff`) to cover more than one bitset word.
- Added `going_back_up_compares_folders_composed_where_the_drive_composes_them`: enter a folder in NFD or NFC form, `leave_to` the other form's subfolder, and that folder's rules stay in force.

## Results
- `cargo fmt --all --check`: clean (after running `cargo fmt`).
- `cargo clippy --workspace --all-targets -j 1 -- -D warnings`: clean.
- `cargo test -j 1 -p bolt-vcs-core --lib ignore::tests -- --test-threads=1`: 24 passed.
- `cargo test -j 1 -p bolt-vcs --test ignore_vs_git -- --test-threads=1`: 6 passed.
- `cargo test -j 1 -p bolt-vcs-core --lib snapshot:: -- --test-threads=1`: 194 passed, 2 ignored. `presets`: 11 passed.
- Old-against-new comparison (`BOLT_COMPARE_CASES=100000 cargo test -j 1 --release -p bolt-vcs-core --lib legacy::compare_ignore -- --test-threads=1`):
  - **With item 4 temporarily reverted:** all 6 pass, so items 1–3 keep every answer.
  - **With item 4 in place (as committed):** 5 pass. `walks_agree` fails: `bytes after leave_to("*/é")`, left 0, right 1. The shrunk input enters a folder written `e\u{301}` on a composing drive and then calls `leave_to("é/a")`. The old code drops that folder's rules because the two spellings differ as bytes. The new code keeps them because they name the same folder. This is the very inconsistency item 4 asked me to remove.

## Concern
Items 2 ("keep every answer") and 4 conflict: item 4 changes an answer by design. On a composing drive, mixed-form `leave_to` calls only happen in the comparison's random walks; the real walk passes names in one form. I can't adjust the comparison because it lives in `legacy/`. Either the controller or someone allowed to open `legacy/` must update `walks_agree` (for example, compare with folder names in a single form, or accept the new answer), or rule that item 4 should not change `leave_to`'s answers.

An earlier run with only `ignore::` as the filter also picked up `legacy::compare_ignore::walks_agree`, which failed in the same way. That was the first sign of this difference.

## Files opened
- `.superpowers/sdd/2026-11-11-v0.1-engine/clean/common-clean-rules.md`
- `.superpowers/sdd/2026-11-11-v0.1-engine/clean/ignore-restructure-brief.md`
- `.superpowers/sdd/2026-11-11-v0.1-engine/clean/ignore-brief.md`
- `.superpowers/sdd/2026-11-11-v0.1-engine/implementer-contract.md`
- `crates/bolt-vcs-core/src/ignore.rs`
- `crates/bolt-vcs-core/src/ignore/tests.rs`
- `crates/bolt-vcs-core/src/snapshot/walk.rs` (only a grep for `leave_to` and `.enter(`)

Nothing in `legacy/` was opened (see the fix note below for the files opened since).

## Fix note: item 4 reverted, per the coordinator's ruling

- `enter` stores the level's own folder as given again, and `leave_to` compares its argument as given again, both as before item 4. The rule files' folders are still composed in `enter`, as they were before this task, now through the shared helper `composed_name`, which gives the same answers. Items 1–3 are unchanged.
- I removed the test `going_back_up_compares_folders_composed_where_the_drive_composes_them`.
- **Not yet verified or committed.** Free memory was 0.6 GB, so I waited until it reached 2.67 GB. `cargo fmt --all --check` passed. Claude Code then stopped the comparison run (`BOLT_COMPARE_CASES=100000 … legacy::compare_ignore`) because the system was critically low on memory, and I may not restart it on my own. The revert is in the working tree, not committed, as the diff `ignore.rs` +1 −3, `ignore/tests.rs` −15.
- Before item 4 was first reverted (a temporary revert, the same answers as now), the same comparison passed 6 of 6 at 100000 cases. To finish: when memory allows, run the comparison, `ignore::tests`, `ignore_vs_git` and `snapshot::`, then commit.
- Files opened since the first report: the background task output files for the memory waits and the test runs (in this session's temp `tasks` folder). The comparison was run as a command, and only its panic message was read: the input and both answers, as the brief allows. No `git` history was read, no web pages were read, and I ran no `git` cases (the behaviour came from the existing tests and the brief).

(The coordinator then committed the item-4 revert as `e619596`.)

## Fix note 2: linear bracket reading, and a cheaper `composed`

Commit `95f22f3` (on top of `4453b54`).

1. **Linear bracket reading.** `members` now remembers where the next `]` is (as the length of text left there) and searches again only once the reader has gone past it. Each search starts after the previous `]`, so no byte is searched twice, and no search goes past the bracket's closing `]`. When no `]` remains, it returns `None` (matches nothing) at once; the old answer was the same, because such a bracket can never close. `class_at(text, close)` now takes that position instead of searching for it. The answers are unchanged.
   New test `reading_a_bracket_takes_time_in_proportion_to_its_length` uses shapes of 500,000 repeats: `[` + `[:`×n + `x]`, `[` + `[:a`×n + `]`, `[` + `\]`×n + `]`, `[a` + `-[:`×n + `]` (each checked for one path it matches and one it doesn't), and two that match nothing: never closed, `[` + `[:`×n + `x`, and a class name running to the end, `[` + `[:`×n + `:]` (checked through `wildmatch` and through a rule file with two such lines). It must all finish within 60 s. It takes 4.4 s in a debug build here; the old reader would need about 10^11 steps.
2. **`composed`** returns the path as is when the drive doesn't compose Unicode or the path is ASCII, before any UTF-8 check.

### Results
- Free memory was checked before each run (2.6–6.2 GB). The first comparison run was stopped by Claude Code for low memory; after a wait, the retry finished.
- `BOLT_COMPARE_CASES=100000 cargo test -j 1 --release -p bolt-vcs-core --lib legacy::compare_ignore -- --test-threads=1`: 6 passed, 0 failed.
- `cargo test -j 1 -p bolt-vcs-core --lib -- --test-threads=1 ignore:: snapshot:: presets::`: 235 passed, 2 ignored.
- `cargo test -j 1 -p bolt-vcs --test ignore_vs_git -- --test-threads=1`: 6 passed.
- `cargo fmt --all --check` and `cargo clippy --workspace --all-targets -j 1 -- -D warnings`: clean.

### Files opened and commands
- `crates/bolt-vcs-core/src/ignore.rs`, `crates/bolt-vcs-core/src/ignore/tests.rs` (grep and read), and the background-task output files in this session's temp `tasks` folder.
- To compare test times, I ran `git stash` and then `git stash pop` once, which set aside my own uncommitted changes and brought them back. I didn't list or show any stash or history.
- I opened nothing in `legacy/`, read no web pages, and ran no `git` behaviour cases.

## Fix note 3: RT14-3, the cost of glob lines

Commit `193c017`.

1. **No allocation in matching.** `run` used to allocate four `Vec<bool>` for every line it tried on every path. It now steps positions as bit sets (`u64` words). A pattern of up to 63 pieces runs in a `[u64; 4]` on the stack. A longer one runs in a `Vec<u64>` kept per thread (`thread_local!` + `RefCell`); it is cleared and resized, so it doesn't allocate once its capacity is reached. If the `RefCell` is somehow already borrowed, a fresh `Vec` is used. The stepping is unchanged: for each byte, `inside` and `at` are visited in the same order, now iterating over the set bits. The ordinary path allocates nothing per path: plain lines (lookup by hash, then a compare), one-star lines (comparing the two ends), short glob lines (stack), and `composed` (borrowed on drives that don't compose and on ASCII names).
2. **A quick test per line (`Sieve`).** Removing the allocation alone took the proof from 14.3 s to 5.1 s (Git: 1.27 s); what remained was reading every line's boxed pieces from memory, about 10 MB per path. Each glob line now also keeps, inline, the bytes its first byte can be (from a leading byte or `[…]`, in both letter cases; any byte otherwise) and the bytes every match must contain (its plain bytes). `last` works out once per rule file per path which bytes the path holds (both cases where case is ignored), and only if the file has non-plain lines. A line is stepped only if it passes the quick test. The test only turns a path away when the line can't match it, so every answer stays the same. New test `the_quick_test_never_turns_away_a_match` checks every 3-piece pattern built from 14 pieces (bytes, `\b`, `\B`, `?`, `*`, `**`, `**/`, `/**`, ranges, negated sets, classes, `[B]`) against every text of up to 3 bytes over `a B c /`, with and without folding: whenever the pattern matches, the quick test passes.
3. Removed the `#[ignore]` from `rt14_3`'s test in `crates/bolt-vcs/tests/redteam14.rs`; nothing else in it changed.

### Results (free memory 5.5 GB or more before each run)
- `cargo test -j 1 --release -p bolt-vcs --test redteam14 -- --test-threads=1`: passes. `git 1.23 s, bolt 0.23 s` (before this fix: 14.3 s; after step 1 only: 5.09 s).
- `cargo test -j 1 -p bolt-vcs-core --lib -- --test-threads=1 ignore:: snapshot:: presets::`: 223 passed, 2 ignored. `ignore::tests` alone: 18 passed.
- `cargo test -j 1 -p bolt-vcs --test ignore_vs_git -- --test-threads=1`: 6 passed.
- `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -j 1 -- -D warnings`: clean.

### Notes
- `legacy/` no longer exists at this HEAD, so the old-against-new comparison couldn't be run. "Every answer the same" rests on the unchanged stepping, the quick-test property test, the unit tests and `ignore_vs_git`.
- The `**/z1/**/q*` shape the red team also timed isn't in the proof. The quick test turns those lines away for paths without `z`, `1` and `q`, but I didn't time that shape. I also didn't measure `status` on Git's own source.
- `wildmatch` (now `glob_match`) still compiles its pattern on every call. That is the command-line path, not the ignore walk, so I left it.
- Files opened: `crates/bolt-vcs-core/src/ignore.rs`, `crates/bolt-vcs-core/src/ignore/tests.rs`, `crates/bolt-vcs/tests/redteam14.rs` (lines 100–169), and this session's task output files. I ran `git log --oneline` on the two ignore files to see why the test count changed; it lists commit titles only, without diffs. No web pages were read.
