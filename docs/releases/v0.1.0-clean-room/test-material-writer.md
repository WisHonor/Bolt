# Task 11 report: the remaining test material, replaced

Status: DONE_WITH_CONCERNS (one scope note, below). Commit `be98331` on `v0.1-engine` (not pushed).

## What I did

### 1. Ref names (`crates/bolt-vcs-core/src/git/refs.rs`, test module)
- `ref_names_follow_each_rule`: hand-written cases of my own, grouped by the rules in
  `git-check-ref-format.adoc` (dot-first level, `.lock`, one level, `..`, control/DEL/space/`~^:`,
  bytes past ASCII and invalid UTF-8 allowed, `?*[`, slash first/last/doubled, trailing dot, `@{`,
  `@` alone, backslash, empty name).
- `ref_names_are_checked_as_git_checks_them`: 3000 names from a fixed-seed xorshift over pieces
  (letters, `/`, `.`, `..`, `@`, `@{`, `{}`, `*?[]`, `\`, `~^:`, space, tab, 0x01, 0x1f, DEL, `é`, `日`,
  `.lock`, `lock`, `HEAD`; half prefixed `refs/heads/`) plus a few fixed one-level/edge names. Each
  is run through `git check-ref-format <name>` with **no options**: Bolt's rule is
  `check_refname_format` with no options (its doc comment; callers are HEAD's target, loose refs and
  packed-refs, all requiring two levels). Exit 0 = valid, 1 = invalid, anything else panics (guards
  against a name read as an option). Children run 32 at a time in an empty temp folder (no repo, and
  nothing for a `*` to glob-match). Skips with a note when `git` isn't installed. Takes ~14 s here.
- The `check_ref_format` helper the brief mentions was not in the module at HEAD (already deleted),
  so the option mapping came from `is_valid_ref_name`'s doc and its callers.
- `--normalize`: no helper written. Bolt never normalizes; it refuses doubled/leading slashes as
  plain `check-ref-format` does, so the comparison needs no normalizing.
- NUL and invalid UTF-8 can't be passed as a Windows command-line argument, so those are covered
  only by hand cases (the existing NUL test plus `\xff\x80`).
- Mutation check: removing `0..=0x1f` from Bolt's rule made the comparison fail on
  `"refs/heads/\u{1}locka-b}"` (proves control bytes reach git intact on Windows); restored.

### 2. Test identities and sample tag
Replacements (all made up): `A U Thor <author@example.com>` -> `Ida Lark <ida.lark@example.com>`,
`C O Mitter <committer@example.com>` -> `Omar Fenn <omar.fenn@example.com>`, `T Agger
<tagger@example.com>` -> `Vera Toll <vera.toll@example.com>`; times 1112912053/…113/…100/…000 ->
1601234567/…627/…614/…500; zones on those lines -0700 -> -0400 (author), +0530 -> +0545 (committer),
keeping a negative zone and a non-whole-hour positive one. Explicit `-0000` zone cases kept as such.
`Person` expectations updated (`-420`→`-240`, `330`→`345`, ms times).
Sample tag: object `c39ae07f…` -> made-up `5e1f0a7d9c3b2486e0d4a1b7c9f3e5d2a8b6c4e0`, `v2.6.11-tree` ->
`snapshot-3-tree`, message -> "The tree as it stood at snapshot 3.".

`object.rs` `a_standard_commit_rebuilds_without_git_header` checks IDs "as Git made them", so I
recomputed them with `git commit-tree` (temp repos, SHA-1 and `--object-format=sha256`, empty tree,
GIT_AUTHOR_*/GIT_COMMITTER_* set to the new values): SHA-1 first `cc9c6586…`, second `9bb66b4c…`;
SHA-256 parent (the SHA-256 "first") `6230c9b9…`, second `acf2103a…`.

## Scope note (concern)
The same copied identities were also in `crates/bolt-vcs/tests/amend.rs` and
`crates/bolt-vcs/tests/import_vs_git.rs`, which the brief doesn't name. I replaced them there too
(same values; both tests compute against Git or Bolt at run time, no fixed IDs) and ran both. Revert
those two hunks if that was out of scope.

## Tests run (Bash, `-j 1`, `--test-threads=1`, BOLT_CONFIG_DIR set; free memory checked ≥5.7 GB each time)
- `cargo test -j 1 -p bolt-vcs-core --lib -- --test-threads=1`: first run 1 failed (the recomputed-ID
  test above); after fixing: `ok. 1083 passed; 0 failed; 5 ignored`.
- `cargo test -j 1 -p bolt-vcs --test import --test corpus_git --test amend --test import_vs_git --
  --test-threads=1`: amend 12 ok, corpus_git 25 ok, import 33 ok, import_vs_git 7 ok (1 ignored).
- `cargo fmt --all --check` clean; `cargo clippy --workspace --all-targets -j 1 -- -D warnings` clean.
- No `#[cfg(unix)]` tests added.

## Files opened
- `.superpowers/sdd/2026-11-11-v0.1-engine/clean/common-clean-rules.md`, `clean/test-material-brief.md`,
  `implementer-contract.md`
- `crates/bolt-vcs-core/src/git/refs.rs` (lines 1–420 + tests), `crates/bolt-vcs-core/src/git/person/tests.rs`
  (1–60), `crates/bolt-vcs-core/src/git/object.rs` (test regions ~480–530, 1055–1110, 1436–1510, 1540–1570),
  `crates/bolt-vcs-core/src/import/convert.rs` (1455–1470, 2250–2290, 2405–2450),
  `crates/bolt-vcs-core/src/git/config/tests.rs` (225–300, for the git-skip pattern),
  `crates/bolt-vcs-core/Cargo.toml` (dev-dependencies), grep hits in `crates/bolt-vcs/tests/amend.rs`
  and `crates/bolt-vcs/tests/import_vs_git.rs`.
- `C:\Program Files\Git\mingw64\share\doc\git-doc\git-check-ref-format.adoc` (documentation).
- Grep over `crates` excluded `legacy/`; `docs/releases/v0.1.0-code-audit.md` showed up only as a
  file name in a grep `-l` listing and was not opened. No history commands, no other project's code or tests, no web pages.

## Commands that taught behaviour
- `git check-ref-format` on `refs/heads/a<n>` (timing, ~35 ms each), and on `refs/heads./a`,
  `refs/heads/a{@`, `refs/heads/@`, `a/b` (all exit 0).
- `git commit-tree` in temp SHA-1 and SHA-256 repos with the new identities, to get the IDs above;
  `git cat-file commit` to confirm the byte form matches the test's `commit()` helper.
