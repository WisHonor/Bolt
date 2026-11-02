# 031 — Bolt reads Git repositories with its own reader

- **Status:** Accepted
- **Date:** 2026-11-02
- **Agenda topic:** none (milestone 2 planning, question 1)

## Context

`bolt import` (milestone 2) reads a whole Git history: refs, commits, trees and blobs, from loose objects and pack files. The repository is untrusted ([security.md](../security.md) §3). Decision 022 keeps memory-mapping out of v0.1, and v0.1 promises that big files are streamed, never held whole in memory.

Research on 2026-11-02 (gix's source and releases, Git's documentation and source, and the security advisories of Git, gitoxide, libgit2, go-git and dulwich) found that most advisories for Git readers in 2026 were crashes and huge allocations on crafted packs.

## Options

- **A. gix (gitoxide).** Used by Jujutsu (its only Git library), Cargo, GitButler and Helix. It memory-maps every pack and index with no way to turn that off, reads every object whole into memory, follows alternates and replace refs by default, and can't read reftable. A minimal setup pulls in 124 crates, and a breaking release comes about monthly. Its pack reader had crash and allocation bugs on crafted packs, fixed in 2026 and published only as GitHub advisories, which `cargo-deny` doesn't see.
- **B. Our own reader.** About 2,000–4,000 lines plus tests and fuzzing; every Bolt rule holds. go-git, dulwich, JGit and isomorphic-git each have their own.
- **C. The `git` program** (`git cat-file --batch` or `git fast-export`). Reads every format Git can, with the least code. Needs Git 2.45 or newer installed; running Git on a stranger's repository is what several published attacks use; `fast-export` drops some commit headers. Fossil's importer and reposurgeon work this way.

## Decision

**B.** The engine library gets a read-only Git reader.

- **What it reads:** loose objects; pack indexes (version 2) and packs, with offset and reference deltas; loose refs, `packed-refs` and `HEAD`; and from `.git/config` only `core.repositoryformatversion` and the `extensions.*` keys. SHA-1 and SHA-256 repositories.
- **How it reads:**
  - nothing is memory-mapped, and every read is bounded;
  - files inside `.git` are opened as regular files without following links, as in `.bolt/` (T19);
  - an object Git stored whole streams straight into Bolt's pack;
  - a deltified object is rebuilt in memory, and both it and its base may be at most 512 MiB, past which the import stops; Git never deltifies files above 512 MiB by default (`core.bigFileThreshold`);
  - delta chains are followed without recursion, with a depth limit and a cycle check;
  - every Git object's ID is checked as it is read, with collision-detecting SHA-1, as Git checks its own (or SHA-256).
- **What it never does:** run Git or Git LFS, fetch anything, or use `refs/replace`, grafts, the commit-graph, the multi-pack index, bitmaps or any other file that only speeds Git up.
- **What it refuses,** before writing anything, saying what to do instead: an unknown repository format version or extension; reftable; partial clones; shallow clones; alternates; a `.git` that is a file (linked worktrees, submodule checkouts); and a `.git` folder that another account owns, unless the project folder is in `safe-folders`, as for `.bolt` (decisions 025 to 027 and 030).
- **Dependencies:** `miniz_oxide` (zlib, which forbids `unsafe` itself), `sha1-checked` and `sha2`, each reviewed under T18.
- **Git itself** runs only in tests, as the reference. Tests that run Git on hostile repositories run only in the red-team sandbox.

## Consequences

- Import is the biggest single piece of milestone 2. The differential tests compare every imported commit with Git, and the reader gets its own fuzz targets and malicious-repository cases (testing.md).
- [security.md](../security.md) T6 says import uses Bolt's own reader and never runs Git.
- Reftable repositories can't be imported until the reader learns that format, which it needs before Git 3.0 makes reftable common ([design.md](../design.md) §7, deferred).
