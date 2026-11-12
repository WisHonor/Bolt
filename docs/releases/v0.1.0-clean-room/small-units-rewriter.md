# Task 9 report: four small units, written new (clean room)

## What I implemented

Each home now holds my own code at the brief's interface. The old re-exports are gone, and `#[cfg(test)] mod tests;` stays in each.

- `names/lookalike.rs`: `is_git_alias`, `is_bolt_alias`, `is_hashed_short_name`, `is_plain_short_name`.
  - NTFS form: a part is cut at the first `:`, then trailing dots and spaces are dropped, and it is compared ignoring ASCII case with `.git`, or with `git~1` (only `~1`, as Git does).
  - For `.git`, every `\`-separated part is checked, as Git does. For `.bolt`, only the first part is checked, with `bolt~1` to `bolt~4` (storage-format.md).
  - HFS+ form: the ignorables U+200C–U+200F, U+202A–U+202E, U+206A–U+206F and U+FEFF are skipped anywhere. Then `.git` or `.bolt` must match in any ASCII case, followed by the end of the name, `\`, U+FFFE or U+FFFF. This is checked from the name's start only.
  - Short names of `.gitmodules`: the plain form takes digits 1–4 and the hashed form 1–9. Both are checked in any `\` part, with any stream suffix and trailing dots and spaces.
  - Fast path: the first byte must be `.`, the letter, 0xE2 or 0xEF; otherwise the name is refused at once. For `is_git_alias` and the short-name checks, a name that contains a `\` still gets the full check. The functions never allocate.
- `config/git_path.rs`: `tidy_path`.
  - The root is kept as written, with `\` turned into `/`. Without `windows`, the root is one leading `/`.
  - On Windows the root is one of three things. A drive is the first character (its length taken from the UTF-8 lead byte, any character) followed by `:` and an optional separator. A network root is two separators, the server, a separator, the share and the separator after the share; when no separator follows the server, there is no root at all. Otherwise the root is one leading separator.
  - After the root, runs of separators collapse to one, `.` parts go, and `..` takes away the part before it. When there is no part to take away, the result is `None`.
  - A trailing separator is kept: `/a/b/..` becomes `/a/`.
- `git/person.rs`: `parse_person`.
  - The name runs to the first `<`, with trailing Git whitespace trimmed. The email runs to the first `>` after that `<`. A line with no `<` is all name; with no `>` after the `<`, the email is empty and the time and zone are zero.
  - The date is read after the last `>`: whitespace, digits, whitespace, then `+` or `-` followed by a digit. Without all of these, both the time and the zone are 0.
  - A time that doesn't fit in milliseconds as an i64 becomes 0 by itself. A zone that isn't exactly 4 digits becomes 0 by itself, and minutes past 59 count as they are.
  - Text goes through `String::from_utf8_lossy` and is cut at `floor_char_boundary(MAX_PERSON_FIELD_BYTES)`.
- `git/lfs.rs`: the constants, `LfsPointer` with `oid_hex`, and `lfs_pointer`.
  - `LFS_SPECS` holds git-lfs v1 and hawser v1, which object.rs and the docs need. The alpha `http://git-media.io/v/2` is accepted as a third version by `lfs_pointer` only.
  - Steps: a blob over 1024 bytes gives `None`. Unicode whitespace is trimmed from both ends; a byte that isn't UTF-8 stops the trimming. Lines split at LF, one CR is dropped from each line's end, and empty lines are skipped. Each line splits into key and value at its first space.
  - The keys must be `version`, `oid` and `size`, in that order. `ext-<0-9>-<name>` lines may come anywhere before the size; a name starts with `[A-Za-z0-9_]` and anything may follow. At most one name per digit; the same line twice is fine.
  - Values: the oid and every extension value are `sha256:` followed by 64 lowercase hex digits. The size is a decimal i64 with an optional sign and must not be negative (`-0` is zero). Nothing may follow the size line.

## How behaviour was learned (clean-room log)

**Files opened**
- In the dispatch's folder: common-clean-rules.md, small-units-brief.md and implementer-contract.md.
- The four homes and their `tests.rs`, plus `names.rs` (validate_name, writable_here, tests), `config.rs` (around system_file_named) and `save.rs` (Person, MAX_PERSON_FIELD_BYTES).
- In `git/mod.rs`: is_git_space, trim_end and trim_start. In `git/object.rs`: split_at_byte, the lfs look-alike functions and their tests.
- `import/convert.rs` (tag_message and message_text), `lib.rs`, the workspace and crate `Cargo.toml`.
- Grep hits on docs: storage-format.md lines 73 and 640, design.md lines 277, 522 and 540, decisions 004, 035, 037, 043, 051, 059 and 106, json.md line 501, and milestone-2 plan and rulings lines found by grepping for "hawser". These were single lines from grep output, not the plans as a whole.
- Git's `fsck-msgids.adoc` (grep for dotgit and gitmodules) and `git lfs pointer --help`.
- No source of Git or Git LFS, nothing in `legacy/`, and no history was opened. No web pages were read.

**Commands that taught behaviour.** All ran in my scratchpad, outside the repository. The scripts and data are in `C:\Users\elmas\AppData\Local\Temp\claude\4167566f4b6edf19\fd447068-700a-479c-b89f-0478aae16b7b\scratchpad\t9\`.

- `git update-index --index-info` with `core.protectNTFS` and `core.protectHFS` (probe1.py, probe2.py). Git for Windows refuses every Windows-invalid name with these settings: any `\`, `:`, a trailing dot or space, device names and control characters. That made it useless for telling `.git` spellings apart, so I switched to the next method.
- `git fsck --unreachable` on one-entry trees written with `git hash-object -t tree --literally` (fsckcheck.py).
  - `hasDotgit` on file entries gave Git's `.git` rule (probe4.py, probe5.py).
  - `gitmodulesSymlink` on link entries gave the short names of `.gitmodules` (probe6.py).
  - gen_names.py generated 6,000 mutated `.git` spellings and 3,000 mutated short names in two seeds. A temporary test compared them with my functions and has since been removed. Every `.git` verdict agreed. For the short names, the only difference is deliberate: Bolt also refuses `gitmod~1\`, `GITMOD~4\` and `Gi7EbA~9\` (a short name followed by `\`), which Git accepts.
- `GIT_CONFIG_SYSTEM=<path> git config --system --list`, reading the path in Git's error message (tidy.py with tidycases.txt and tidycases2.txt). gen_paths.py then compared 1,200 random Windows paths in a temporary test. All agreed: each difference my checker reported was a `<drive>..` failure that Git prints untidied, which my checker didn't recognise as a failure.
- `git hash-object -t commit --literally` followed by `git log -1 --date=raw --format='%an|%ae|%at|%ad'` on odd author lines (person.py, personcases.txt).
- `git lfs pointer --check --file` (lfs1.py, lfs2.py and lfs3.py by hand, then gen_lfs.py and gen_lfs_gentle.py on 2,648 random near-pointers). Every verdict on blobs of at most 1024 bytes agreed.

## Tests

New tests:
- lookalike: `git_spellings_match_what_git_fsck_flags`, `short_names_of_gitmodules_match_what_git_fsck_flags` and `ordinary_names_are_answered_at_once`.
- git_path: `paths_are_tidied_part_by_part`.
- person: `people_read_as_git_log_shows_them`.
- lfs: `the_lines_go_in_git_lfs_order`, and `lfs_pointer_agrees_with_git_lfs`. The second builds 68 near-pointers, one piece changed at a time plus 40 mixtures, runs `git lfs pointer --check --stdin` on 8 threads, and is skipped with a message where `git lfs` isn't installed. It takes about 3 seconds.

Results:
- `cargo test -p bolt-vcs-core --lib -- lookalike git_path person lfs names::`: 51 passed.
- `cargo fmt --all --check`: clean.
- `cargo clippy --workspace --all-targets -j 1 -- -D warnings`: clean.
- `cargo test --workspace -j 1 -- --test-threads=1`: see the end of this report.

## Differences from Git, and choices I made

1. **Person lines.** Bolt's own test decides here, and it is stricter than Git. Git shows `1 +5` as `+0005`, `+01000` as `+1000` and `+01:00` as `+0001`; Bolt makes each zone 0. Git drops the zone with a time too large; Bolt keeps the zone (60 for `+0100`). Git can't split a line without `<`; Bolt keeps the whole line as the name, untrimmed. That last one is my choice, since it can't be observed.
2. **Files over 1024 bytes.** `git lfs pointer --check` reads only the first 1024 bytes of a file. So it accepts a pointer followed by more than 1024 bytes of newlines, or followed by an `x` at byte 1025. Bolt's test says `None` for anything over 1024 bytes, and the brief limits the comparison to blobs of at most 1024 bytes, so I followed Bolt's test.
3. **The hawser version.** `LFS_SPECS` has to hold `[&str; 2]`, but Git LFS accepts three versions: git-lfs v1, hawser v1 and the alpha. `LFS_SPECS` holds git-lfs and hawser (what object.rs and the docs use for look-alikes), and `lfs_pointer` also accepts the alpha.
4. **Short names followed by `\`.** `is_plain_short_name` and `is_hashed_short_name` refuse a short name followed by `\` (`gitmod~1\`), which Git accepts. This is stricter and only affects `writable_here`.
5. **`.bolt`.**
   - Bolt checks only the first `\` part, as Bolt's test requires (`x\.bolt` is accepted).
   - HFS+ treats `\`, U+FFFE and U+FFFF as the end of the name, as for `.git`. So `\u{200c}.bolt\x` is refused, which is stricter.
   - Bolt doesn't recognise `.bolt`'s hashed short names by themselves. `writable_here` refuses them only where the folder's listing gives them.
6. **`tidy_path`, cases Git doesn't show.**
   - A drive's character length comes from the UTF-8 lead byte, so a byte that isn't UTF-8 counts as one byte. Git for Windows can't be given such bytes through the environment.
   - Without `windows`, `//x` becomes `/x`: one leading `/` is the root. I couldn't run Git on Linux here.
7. **`\0`** in a name is not treated as an end, since Bolt's names never contain one. Git's C strings would end there.
8. **Cost.** I found no Bolt test that counts full checks on ordinary names. Ordinary names are answered from their first byte; for `is_git_alias` and the short-name checks, a name with a later `\` also gets a scan for it.

## Self-review

- Each unit matches the brief's interface, the module docs and comments are short, and no other project's source is named.
- There is no `unwrap`, `expect` or `panic` in library code. The arithmetic follows `git/`'s `arithmetic_side_effects` lint (checked or saturating operations).
- No `#[cfg(unix)]` tests were added.
- Temporary comparison tests were removed. The scratch scripts and data stay in the scratchpad.

## Workspace run

Commit f204660. `cargo test --workspace -j 1 -- --test-threads=1` was stopped by Claude Code because the laptop ran short of memory, and it was stopped while still compiling. So these have not been run yet: the full workspace suite, the other `names.rs`, `config.rs`, `git/object.rs`, `import` and `snapshot` tests, and `-p bolt-vcs --test import --test import_lfs --test corpus_git --test redteam3`. The harness said not to restart it without being asked. Run them when memory allows.

## Fix round 1 (commit 036f2bf, on 3f0c982)

How I learned the old answers: by running the comparison only. That means the `legacy::compare_small` tests, and the ignored `census` test run with `--ignored --nocapture`, which lists every differing input with both answers. I parsed its output with scratchpad `cen.py`. I opened nothing in `legacy/`. I also ran `git fsck` once more on longer-counter short names (probe7.py): Git agrees with the old code that the hashed form takes longer counters and the plain form doesn't.

Changes:
- **`is_git_alias`:** a `/` ends the name (`.git/` and `GIT~1/` are now look-alikes).
- **`is_bolt_alias`:** it takes the part before the first `/` or `\` and removes every HFS+ ignorable from it. The NTFS rule then applies: stream suffix, trailing dots and spaces, `.bolt` or `bolt~1` to `bolt~4`. So `\u{206d}bolt~1` is now a look-alike. U+FFFE no longer ends a name, so `.bolt\u{fffe}` is not.
- **`is_hashed_short_name`:** the whole name is checked; parts after a `\` no longer count. The name is the prefix, `~` and a digit 1 to 9. A longer counter is also accepted: it starts with 1 to 9, and the prefix is cut so that start, `~` and counter make 8 characters (`gi7eb~10`, `~1234567`). Streams and trailing dots and spaces are dropped as before.
- **`is_plain_short_name`:** the whole name is checked, with a one-digit counter 1 to 4 only. The coordinator's example `git~1091` gave **false** in the old code when I ran the census (old=false, mine was true), so I followed the run. Git agrees.
- **`tidy_path`:**
  - A NUL ends the path. On Windows the root is still read from the full bytes: a NUL ends a network share, but not the server name, so `//.\0/` keeps its root as old does.
  - A Windows drive is one ASCII byte other than NUL, or 1 to 4 bytes outside ASCII, followed by `:`. Git itself takes `éé:` as a drive.
- **`parse_person`:** a line without `<` has its trailing whitespace trimmed.
- **Tests:** in lookalike/tests.rs, `x\gitmod~1` and `gitmod~1\` are no longer short names, and I added longer-counter cases. Note that `x\gitmod~1` is still a short name to Git, so here the old answers (and now Bolt's) are less strict than Git.

Runs:
- Census, debug, 20,000 cases: the only remaining groups are prefixes callers never pass (`""`, `.git`, `g`, `gi7eba~`).
- Census, release, 100,000 cases: the same, plus `bo`.
- `cargo test -j 1 -p bolt-vcs-core --lib legacy::compare_small -- --test-threads=1` (debug, 2,000 cases) and the same with `BOLT_COMPARE_CASES=100000 --release`: constants, paths, persons and pointers agree. `names_agree` and `hand_made_inputs_agree` fail only on `"gi7eba~1"` with the excluded prefix `gi7eba~`. The hand-made test stops at that first failure, so the hand-made inputs after it are unchecked until the generator limits its prefixes.
- `cargo test -j 1 -p bolt-vcs-core --lib -- --test-threads=1`: 1078 passed, 2 failed (the two above, on the excluded prefix), 5 ignored.
- fmt and clippy `-D warnings` are clean.
- Still not run: the workspace suite and the `bolt-vcs` integration tests (import, import_lfs, corpus_git, redteam3).

## Fix round 2 (commit after 1f3058b)

- **`is_git_alias`:** a `/` now only ends the part it is in. The HFS+ check reads up to the first `/`. The NTFS check covers the start and every part after a `\`, each ending at the next `/` or `\`. So `/\.git`, `./\GIT~1`, `/é\.GIT` and `\t/0\.Git` are look-alikes, while `x/.git` is not, as before.
- **New test:** `a_slash_ends_a_part_but_a_backslash_starts_another`.

Runs:
- `BOLT_COMPARE_CASES=1000000 cargo test -j 1 --release -p bolt-vcs-core --lib legacy::compare_small -- --test-threads=1`: 6 passed, 0 failed, 1 ignored (census).
- `cargo test -j 1 -p bolt-vcs-core --lib -- --test-threads=1`: 1081 passed, 0 failed, 5 ignored.
- fmt and clippy `-D warnings` are clean.
- Still not run: the workspace suite and the `bolt-vcs` integration tests (import, import_lfs, corpus_git, redteam3).
