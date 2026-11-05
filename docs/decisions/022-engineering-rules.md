# 022 — Engineering rules

- **Status:** Accepted
- **Date:** 2026-10-31
- **Agenda topic:** 18 (design review); first proposed as risk 16 at kickoff
- **Refined by:** [decision 048](048-handle-relative-file-access.md) (folders are held open and worked inside, never through a link)

## Context

Bolt is a data-integrity tool written mostly by Claude Code while the owner learns Rust, so subtle bugs can look fine in review. The kickoff proposed rules to guard against that. The design review added one practical snag: some Windows features (naming the program that holds a file, hiding `.bolt/`, file IDs for the fast `status`) need operating-system calls that Rust only allows inside `unsafe` code.

## Decision

- **No `unsafe` in the engine:** the engine library uses `#![forbid(unsafe_code)]`.
- **One exception:** a tiny, separate platform crate may use `unsafe`, for operating-system calls only. It stays as small as possible, and the owner reviews it line by line.
- **No panics in library code:** errors are returned; clippy's `unwrap_used`, `expect_used` and `panic` lints are denied outside tests.
- **Spec before code** for anything stored on disk, with test vectors for every format.
- **One place per rule:** name validation, path building and write-out each live in a single module that everything else uses.
- **Review:** the owner reviews all code that touches disk or network before it is merged, and changes are kept small.
- **Done means tested:** a task counts as done only when its tests pass on Windows, macOS and Linux.

## Consequences

- These rules are copied into CLAUDE.md so every coding session follows them.
- Milestone 1, step 0 sets them up as compiler and clippy settings and creates the platform crate.
- Memory-mapping files stays out of v0.1: it needs `unsafe`, and a file changed underneath can crash the process.
