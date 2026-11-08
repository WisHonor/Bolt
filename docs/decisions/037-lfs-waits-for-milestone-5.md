# 037 — Until milestone 5, an import stops at Git LFS files

- **Status:** Accepted
- **Date:** 2026-11-02
- **Agenda topic:** none (milestone 2 planning, question 7)
- **Refines:** [decision 020](020-lfs-import.md) (how LFS files are imported, which milestone 5 builds)
- **Superseded by:** [decision 085](085-importing-git-lfs-files.md) (milestone 5 imports Git LFS files)

## Context

Git LFS keeps a small pointer file in Git for each big file: the real file's SHA-256 and size. The real file lives in `.git/lfs/objects` and on the server. Decision 020 says how Bolt imports LFS files, and [milestones.md](../milestones.md) puts that in milestone 5 with chunking and streaming. Milestone 2's import arrives first.

## Options

- **A. Stop** at any repository whose history holds an LFS pointer, until milestone 5.
- **B. Import the pointer files as ordinary small files,** with a warning. This is how Jujutsu, which doesn't support LFS, sees them. The history matches Git's storage but not the user's folder, so the first save records every big file as changed. Milestone 5 can't repair an import already done.
- **C. Bring decision 020 forward into milestone 2.** LFS projects become importable two milestones earlier, but the milestone grows: reading Git LFS's store, checking every file, a pre-check that every version is on disk, and differential tests that need Git LFS installed.

## Decision

**A.**

- **Stopping.** The import stops at the first Git LFS pointer it finds, naming the file and the commit, and nothing is committed. The message says this version of Bolt can't import Git LFS files yet.
- **The test** is the one Git LFS itself uses. A blob counts as a pointer when all of these hold:
  - it is at most 1,024 bytes;
  - it begins with `version https://git-lfs.github.com/spec/v1` (or the older `https://hawser.github.com/spec/v1`);
  - it has an `oid sha256:` line of 64 hex digits and a `size` line.
- **Lookalikes.** A small file that merely looks like a pointer also stops the import. The message names it.

## Consequences

- Milestone 2's differential tests use repositories without LFS. Projects stored with Git LFS, Unity projects among them, become importable with milestone 5.
- Milestone 5 replaces this stop with decision 020's import, and keeps the same test for finding pointers.
