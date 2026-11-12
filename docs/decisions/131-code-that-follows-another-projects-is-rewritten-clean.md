# 131 — Code that follows another project's code is rewritten clean, and copied test material is replaced

- **Status:** Accepted
- **Date:** 2026-11-11
- **Agenda topic:** none (getting v0.1 ready: clean code and the gate)
- **Decided by:** Claude, at the owner's request while the owner was away. The owner can reverse any part.
- **Refined by:** [decision 133](133-red-team-v0.1-rulings.md) (two findings in the rewritten readers fixed)
- **Refines:** the owner's license decision for v0.1 (closed: all rights reserved, the source private, builds downloaded under a user agreement), which the package session records as decision 140

## Context

Bolt v0.1 ships as a closed product. Git's license (GPL version 2) doesn't allow code that follows Git's code in a closed program, and the owner asked for everything in the repository to be checked: every module written from Git's source, any code ported from another project, the tests, the fuzz targets, the test vectors, the benchmark and red-team folders. Three auditors compared the code with Git's source at `v2.56.0` and with the other projects named in it. What they found, item by item, is in [the code audit](../releases/v0.1.0-code-audit.md). In short:

- **Production code that follows Git's code:** in `ignore.rs`, the reading of ignore lines and the matching of a pattern against a name and a path (Git's `wildmatch.c` and `dir.c`); in `names.rs`, the test for Windows' hashed short names of `.git` and `.gitmodules` (Git's `path.c`); in `config.rs`, how a path Git's settings give is tidied (Git's `path.c` and its Windows path code); in `git/config.rs`, the reader of Git's config files and of the repository format's settings (Git's `config.c` and `setup.c`); in `git/object.rs`, the reading of a commit's author and committer lines (Git's `ident.c`).
- **Production code that follows another project's code:** the Git LFS pointer reader in `git/object.rs` follows Git LFS's own decoder (MIT); the Windows half of the platform crate's folder handle follows Rust's standard library in places (MIT or Apache-2.0).
- **Test material copied from Git's tests:** a line-for-line test port of Git's matcher, and rows from Git's tests t3070, t1014, t0060, t1300, t1402 and t0008; from Git LFS's tests, about twenty pointer cases (MIT); and a few names, times and sample lines from Git's test library, Git's own ignore file and Linux's history.
- **Everything else** implements a documented format or Git's observable behaviour in Bolt's own way (the pack, index, delta and object readers, among others), uses a library through its API, or is Bolt's own. Some of it carries comments that name Git's source files and functions.

## Options

- **A. Rewrite the production code that follows another project's code from that project's documentation and observable behaviour, under clean-room rules; replace the copied test material with comparisons against the real programs and with cases of Bolt's own; delete the old code.**
- **B. Keep the code under permissive licenses (Git LFS's, Rust's) with their notices, and rewrite only what follows Git's.** Lawful, but the release rule is that no code following another project's code remains.
- **C. Stop matching Git's behaviour** (ignore rules, ref names, config files) where the code followed Git. It would break decisions 038, 047, 054 and the import.

## Decision

**A, except the platform crate.**

- **Rewritten under clean-room rules:** `ignore.rs` (all of it), the look-alike tests of `names.rs` (all of them, the ones the audit judged Bolt's own included, so the group is clean as a whole), the tidying of Git's paths in `config.rs`, `git/config.rs` (all of it), and in `git/object.rs` the person lines and the Git LFS pointer reader.
  - **The rewriter** works from a behaviour brief, the other project's documentation, and the program itself (`git`, `git lfs`) run on cases it makes up. It doesn't open the other project's source, the old module, the old module's tests or history, or the audit's working files, and its report lists every file it read.
  - **A comparer** runs the old and the new code side by side from the outside, on generated and hand-made inputs, refusals included, until they decide every case the same way. Where they differ the new code is changed, so no project's saves, ignore verdicts or refusals change.
  - **A reviewer** with the other project's source open says, part by part, whether the new code still follows it. A part that does is rewritten again by a fresh rewriter, from a description that names no code of the other project.
  - Then the old code and the comparison against it are deleted. The fuzz targets drive the new code.
- **Test material copied from another project is replaced:** with comparisons against the program itself where it has a command that shows the behaviour (`git check-ignore`, `git check-ref-format`, `git config`, `git lfs pointer`), and with cases Bolt's own authors write. The names, times and sample lines from Git's test library, Git's own ignore file and Linux's history are replaced too, though each is tiny.
- **Comments and living documents** stop naming another project's source files, functions and tests as the model for Bolt's code. Accepted decision records, old plans, rulings and red-team reports stay as written: they are the record of how the old code came about, and the audit points to them.
- **Kept:**
  - **The Windows half of the platform crate's folder handle** follows Rust's standard library in about 300 lines: how a name is opened relative to a held folder, deleted and renamed by handle, listed, and how a link's target is shown. Its license (MIT or Apache-2.0) allows it in a closed product with its notice, which every Rust program owes anyway, since the standard library is built into it. A rewrite would be new `unsafe` code, in the code that defends against a folder swapped under a command, for the owner to review line by line before v0.1. It stays unchanged, the notices ship Rust's license, and the owner decides whether it is ever rewritten.
  - **Data under licenses that ask nothing or that the notices cover:** the presets' rule lists from GitHub's `gitignore` templates (CC0-1.0, a public-domain dedication), the Unicode ranges in `text.rs` (facts; the Unicode license ships for `unicode-normalization`), the ten frames of the turning mark (common property), and the 320-byte SHAttered prefix that tests SHA-1 collision detection (a published research sample, used only by a test, and no other sample of a real SHA-1 collision exists to use instead).
  - What the audit judged to be Bolt's own expression of a documented format or of Git's behaviour, with any comment that names Git's internals reworded.

## Consequences

- No stored format, `bolt.toml` key or command output changes, and the rewritten readers decide every case as before.
- Because one part of the platform crate follows another project's code, [the code audit](../releases/v0.1.0-code-audit.md) can't say that no code following another project's code remains; it says what remains and why. The owner's rule is that no release build is made until it says so, so the release build waits for the owner's word on that part.
- The old code stays in the repository's history: history is never rewritten. The audit says what a lawyer should look at, the history and the time the repository was public among them.
- The notices file must carry Rust's license (for the standard library and the platform code that follows it) and the duties the audit lists; that file is the package session's, and the follow-ups list has the row.
