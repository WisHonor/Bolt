# 011 — Fast status and safe write-out

- **Status:** Accepted
- **Date:** 2026-10-31
- **Agenda topic:** 11
- **Refined by:** [decision 048](048-handle-relative-file-access.md) (folders are held open and worked inside, never through a link); [decision 049](049-writing-files-one-at-a-time.md) (files are written one at a time from a journal a crash leaves to finish); [decision 053](053-file-state-cache.md) (the file-state cache); [decision 110](110-the-last-saves-listings-are-read-ahead-of-the-walk.md) (the last save's listings are read ahead of the walk)

## Context

`status` must tell what changed since the last save without reading every byte of a project that can hold tens of gigabytes. Every command that writes files (`goto`, `discard`, `undo`, `combine`) must never overwrite unsaved work, even when a game engine or image editor changes files at the same moment, and must cope with Windows refusing to replace files that are open.

## Options

- **Read every file every time.** Always right, but far too slow for big projects.
- **Remember each file's size and last-modified time** (Git's index, Mercurial's dirstate, jj). Only files that look changed are re-read; files changed "too recently" are always re-read to avoid the same-clock-tick trap.
- **The above plus a background watcher** (Git's fsmonitor, Watchman). Close to instant, but adds a helper process to manage, and watchers can lose track.

## Decision

**A file-state cache now; a watcher later as an optional speed-up.**
- **Cache:** for each tracked file, the size, last-modified time, file ID (where the OS provides one) and fingerprint from the last time Bolt read it, stored in `.bolt/cache/` and replaced atomically. Files whose recorded details match are trusted; the others are re-read and re-fingerprinted.
- **Same-tick trap:** files modified too close to the moment the cache was written are always re-read (Git's "racy" rule).
- **Escape hatch:** `bolt status --full` re-reads everything. Deleting the cache is always safe.
- **Big files** are re-read in parallel, with progress shown.
- **Watcher (later):** it may only narrow which files are checked; if it loses events or restarts, Bolt falls back to a full check. Correctness never depends on it.

**Safe write-out** (`goto` and every other command that writes files):
- **Plan first:** compute every file change before touching anything; stop and ask if unsaved work would be overwritten (decision 003).
- **Re-check before replacing:** each file is compared with what the plan expected just before it is replaced; unexpected changes are never overwritten.
- **Files in use:** retry briefly; then finish everything else, report which program holds each remaining file (via Windows' Restart Manager), and let one more command finish the job.
- **Per-file atomicity:** write to a temporary file, then rename it into place; the whole switch is journaled (decision 010).

## Consequences

- `status` writes only to `cache/`, never to the store (decision 003). Two `status` runs may race on the cache, which is safe because cache writes are atomic and the cache is disposable.
- A tool that changes a file while preserving its size and timestamp can go unnoticed until `status --full`; accepted as rare.
- Performance claims for `status` must be benchmarked on real projects (topic 18).
- Write-out safety from decision 004 (no writing through symlinks or outside the project) lives in the same write path and is tested with it.
