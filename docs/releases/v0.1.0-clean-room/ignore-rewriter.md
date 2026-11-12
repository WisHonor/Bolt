# Task 5 report: `ignore.rs`, written new (clean room)

## What I implemented

`crates/bolt-vcs-core/src/ignore.rs` now holds the whole module; it no longer re-exports `legacy::ignore`. Same interface: `wildmatch`, `Rules::parse`, `Decision`, `Verdict`, `Matcher::{new, enter, leave_to, in_force_bytes, decide, verdict, in_force}`, `pub(crate) InForce` (`Clone`) with `verdict`. `#![deny(clippy::indexing_slicing, clippy::arithmetic_side_effects)]` on the module; no unwrap/expect/panic outside tests.

Shape:
- A line is read (BOM, LF, one CR, NUL, `#`, trailing spaces with `\` escapes, `!`, trailing `/`, leading `/`) and compiled once. A line that can't be used is dropped (matches nothing).
- A pattern compiles to one of: `Plain` (bytes compared whole), `Star` (plain head and tail around one `*`, compared at the name's ends, no allocation), or `Glob` (pieces: byte, `?`, set, `*`, trailing `**`, `**/`).
- `Glob` runs as a set of positions stepped once per path byte (two bool vectors), so time is at most pattern length x path length; no backtracking.
- `**` is a whole-folder `**` only when it starts the pattern or follows a `/` (or another `**/`) and ends the pattern or comes before a `/`; any other run of stars is `*` (Git 2.52 reading of `foo**/bar`). A run of three or more stars as a whole name acts like `**` (git 2.50 does this for `a/***/b`, `***/b`).
- Sets precompute two 256-entry tables: exact, and for a path whose letters are lowered (letter case ignored). `/` is never in a set.
- Each rule file indexes its plain lines (basename or path) by a case-folded hash, so plain lines are found without trying each; only the non-plain lines after the best plain hit are tried, newest first.
- The rules in force are a linked list of `Arc<Level>`; `InForce` is one `Arc` clone plus the `Drive`. `leave_to` drops levels whose folder doesn't hold the given folder. `in_force_bytes` is kept cumulative per level.
- Verdict: levels from nearest up, files in reverse, last matching line first; Bolt's verdict from any file, Git's from `.gitignore`/personal lists only; stops when both are known. A rule file applies only to paths strictly below its folder (folder compared case-insensitively where the drive ignores case, and in NFC where it composes). Paths are NFC'd only where the drive composes and the path is non-ASCII and not already NFC.

## Behaviour learnt from `git check-ignore` (git 2.50.x, `-c core.excludesFile= --no-index -v -n`)

All run in temp folders under `%TEMP%` via a scratch script; each case: one `.gitignore`, the paths listed (dirs created where a trailing `/` was used).
- `abc/**` matches `abc/x`, `abc/x/y`, not `abc`. `**` matches `a`, `a/b`. `a/***/b` matches `a/b`, `a/x/b`, `a/x/y/b`, not `a/xb`. `***/b` matches `b`, `x/b`, `x/y/b`. `a/**/**/b` matches `a/b`. `a/**b` matches `a/xb`, `a/b`, not `a/x/b`. `**a/b` matches `xa/b`, `a/b`, not `x/a/b`. `x/**y/z` matches `x/ay/z`, `x/y/z`, not `x/a/y/z`. `*/**/b` matches `a/b`, `a/x/b`, not `b`. `x/**/` matches dir `x/b`, not `x`. `**/` matches dirs only. `/**/b` matches `b`, `x/y/b`.
- `a**/b` matched `ax/y/b` and `a/b**` matched `a/bc/d` on 2.50: the pre-2.52 reading, which the brief says not to follow; my tests use the 2.52 answers for these two.
- Classes: `[]a]x` takes `]`,`a`; `[!]a]x` takes neither; `[^]x` matches nothing (unclosed); `[a-]`, `[-a]` take `a`,`-`; `[a\-c]` takes `a`,`-`,`c` not `b`; `[\a-\c]` takes `b`; `[z-a]y` matches `zy` but not `ay` (a backwards range is empty, but its first byte still counts on its own); `[a-c-e]` takes a-c, `-`, `e`, not `d`; `[--0]` takes `-`,`.`,`0`; `[a-\]]` takes `a` only; `[[:digit:]-a]` takes digits, `-`, `a`; `[[:alpha]x` takes `[`, `a`, `l`, `:` (tested as `qq[[:b]y` vs `qq:y`); `[[:]y` takes `[` and `:`; `[[:al]pha:]]x` matches `apha:]]x`; `[[:foo:]]x` matches nothing; `a[b` matches nothing; `a[/]b` matches nothing; negated classes never take `/` and nothing aborts (`**/a[!b]c` matches `a/c/adc`).
- Letter case (`-c core.ignorecase=true`): `[A-Z]`,`[a-z]` take either case; `[B]` takes neither `b` nor `B`, `[b]` both; `[[:upper:]]` and `[[:lower:]]` take either; `\B` matches neither, `\b` both; `[!B]` takes `b`,`B`; `[!b]` neither; `[B-B]` both; `[Z-a]` takes `z`,`Z`,`_`,`A`, not `y`; `[Z-A]` takes neither `z` nor `Z` (case kept: `Z` only); `[^A-Z]` takes `_` not `x`/`X`.
- Lines: trailing tab kept; `!`, `/`, `!/` match nothing; `\!bang` matches `!bang`; leading space kept; `d\/` and `c//` match nothing; `a\/b` matches `a/b`; `!q/` on dir `q` is a kept verdict.
- Windows note: paths with `:` after one letter, or starting with `:`, are taken by git as drive or pathspec syntax, so `:` was tested inside longer names only.

## Where the brief/spec text and Git differ

- Spec §12 and the brief say "a range that runs backwards (`[z-a]`) is empty". Git takes the range's first byte on its own (`[z-a]` matches `z`). I followed Git, as the spec's binding rule is "read and matched exactly as Git does". Worth a word in `storage-format.md` §12 if the owner agrees.
- `***` as a whole folder name is read as `**` (Git does this); the spec only says "any other `**` is a `*`".
- The brief says comparison tests should pass with a note where git isn't installed; the file's existing helpers panic instead ("this test compares Bolt with Git, and git isn't installed"). My new tests use the same helpers, so they behave as the existing tests do.

## Tests

Own unit tests in `crates/bolt-vcs-core/src/ignore/tests.rs` (existing 8 kept unchanged except the first line's `allow`, which now also allows `indexing_slicing`/`arithmetic_side_effects` because the module's deny reaches its test submodule):
- `double_stars_cross_folders_only_as_whole_folder_names`, `classes_read_as_git_reads_them`, `letter_case_is_folded_as_git_folds_it` (all from the check-ignore runs above), `lines_git_cant_use_match_nothing`, `plain_names_and_other_lines_keep_their_order` (the plain-line index doesn't change which line wins, basename and path, both drives), `rules_apply_only_below_their_folder`, `matching_takes_time_in_proportion_to_pattern_and_path` (thirty `**/` against a 6,000-byte path, `*a` x40 against 5,000 `a`s).

`crates/bolt-vcs/tests/ignore_vs_git.rs`:
- `single_patterns_match_check_ignore`: 96 single patterns (classes, ranges, escapes, `?`, `*`, `**` in every position, case, NFC/NFD, spaces, `!`, `#`, braces), each in its own folder's `.gitignore`, judged on ~30 files and their folders against one `git check-ignore` run; reports every difference. Lines Git 2.52 changed are left out on older git, as the file's proptests do.
- `scenarios_match_check_ignore`: five scenarios (nested negations and anchored lines; both personal lists before the top `.gitignore`; folder-only lines and `**`; nothing taking back a name inside an ignored folder vs `dir/*`; letter case), each checked path by path against `git check-ignore` with `core.excludesFile` and `.git/info/exclude`.
- Added `walk_verdict_with` (personal lists) and `check_ignore_with` (own list); the old functions call them.

Results (Windows, git 2.50):
- `cargo test -p bolt-vcs-core --lib -- --test-threads=1 ignore:: presets::` : ok, 33 passed.
- `cargo test -p bolt-vcs --test ignore_vs_git -- --test-threads=1` : 6 passed (incl. the three proptests at 256 cases).
- `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -j 1 -- -D warnings`: clean.
- Whole workspace (`cargo test --workspace -j 1 -- --test-threads=1`): NOT RUN TO THE END. Claude Code stopped it while it was still compiling because the laptop ran critically low on memory, and I was told not to start it again on my own. The walk's tests (`snapshot::walk`) and the rest of the workspace have not been run against the new module; the controller should run them, or CI will.

## Files changed

- `crates/bolt-vcs-core/src/ignore.rs` (rewritten)
- `crates/bolt-vcs-core/src/ignore/tests.rs` (new tests; first-line allow widened)
- `crates/bolt-vcs/tests/ignore_vs_git.rs` (two new comparisons, two helpers generalised)

## Files opened, commands, web pages

Files opened: `.superpowers/sdd/2026-11-11-v0.1-engine/clean/common-clean-rules.md`, `clean/ignore-brief.md`, `implementer-contract.md`; `docs/storage-format.md` (the `.boltignore` item of §12 and its sub-items, plus grep hits for "boltignore"); `C:\Program Files\Git\mingw64\share\doc\git-doc\gitignore.adoc` (PATTERN FORMAT section); `crates/bolt-vcs-core/src/ignore.rs` (the old two-line re-export), `ignore/tests.rs`, `drive.rs`, `fuzz/fuzz_targets/ignore_rules.rs`, `crates/bolt-vcs/tests/ignore_vs_git.rs`, `crates/bolt-vcs/tests/support/mod.rs` (first ~100 lines, for the git helpers), `crates/bolt-vcs-core/src/snapshot/walk.rs` (lines 195-240, 905-980, 1220-1260, 2180-2195), `presets.rs` (585-630), `scope.rs` (80-110), `names.rs` (grep of fn signatures), `lib.rs` (first 30 lines), `Cargo.toml` of bolt-vcs-core (dependencies). Directory listing of `crates/bolt-vcs-core/src/` and of `.superpowers/sdd/2026-11-11-v0.1-engine/` (names only; none of the other files there opened). Not opened: anything in `legacy/`, git or library source, history of ignore.rs/legacy, `docs/plans/`, `docs/redteam/`, `redteam/`, the code audit. `git log --oneline -8` (subjects only) was read.
Commands that taught behaviour: the `git check-ignore` runs listed above (git 2.50, scratch script in my scratchpad folder, repos in `%TEMP%`, deleted after each run).
Web pages: none.

## Self-review

- Interface kept exactly; callers (`walk.rs`, `scope.rs`, `presets.rs` tests, fuzz target, ignore_vs_git) compile unchanged.
- Cost: plain lines by hash, one-`*` lines by ends, no allocation on those paths; `Glob` allocates two small vectors per call. Snapshot = one `Arc` clone. `status` timing on Git's source is for the controller.
- Concerns: the `[z-a]` note above; `**` behaviour after a literal start can only be checked against git 2.50 here, which reads it the old way (CI's 2.56 will check the comparison tests including those lines).
Commit: 49b6eab

## Fix round 1 (comparer cases)

1. `**` before an escaped `/`. Checked with git 2.50 `check-ignore --no-index`: `**\/b` ignores `a/x/b` and `x/b` but not `b`; `x\/**\/y` ignores `x/k/y` but not `x/y`; `a/**\x` ignores `a/bx`, `a/x`, not `a/b/x`. So a whole-name `**` followed by `\/` takes any run (including `/`, but it can't stand for nothing, since the `/` must still follow); followed by `\` and anything else it is a `*`. Change in `compile`: such a `**` becomes the any-run piece and the `\/` is read as a plain `/`. This gives the comparer's cases: `wildmatch("**\/", "///")` true; `**\/*` on `a/a/a` Ignored; `**\/b` on `a/x/b` Ignored/Ignored.
2. NUL in `wildmatch`'s arguments: the pattern and the text now end at the first NUL (`wildmatch("\0", "")` true). Rule-file lines already ended at a NUL.

Tests added: cases in `double_stars_cross_folders_only_as_whole_folder_names` (incl. a `.boltignore`-style matcher check of `**\/*` on `a/a/a`), new `a_nul_ends_what_wildmatch_is_given`, and `**\/b`, `x\/**\/y`, `a/**\x` added to `ignore_vs_git`'s `PATTERNS` (compared with check-ignore).
Runs (free memory checked first, 3.4 GB): `cargo test -p bolt-vcs-core -j 1 --lib -- --test-threads=1 ignore::` ok, 27 passed, 2 ignored (the comparer's ignored tests in legacy/, not opened or touched); `cargo test -j 1 -p bolt-vcs --test ignore_vs_git -- --test-threads=1` ok, 6 passed; fmt and clippy (workspace, all targets, -D warnings) clean. The comparer's two ignored tests could now be un-ignored if the ruling agrees.
Commands that taught behaviour this round: two `git check-ignore` runs in `%TEMP%` with `**\/b`, `x\/**\/y`, `z/**\/`, `a/**\x`, `c/**\/`, `e/**\*`. Files opened: `ignore.rs`, `ignore/tests.rs`, `ignore_vs_git.rs` (mine). Web pages: none.
Commit: 8ca582f
