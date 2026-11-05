# 010 — Operation log, undo and concurrency

- **Status:** Accepted
- **Date:** 2026-10-31
- **Agenda topic:** 10
- **Refined by:** [decision 034](034-tags-kept-and-shared.md) (tags and server lines live in records of their own that diary entries point to); [decision 049](049-writing-files-one-at-a-time.md) (files are written one at a time from a journal a crash leaves to finish)

## Context

"Undo for everything" is a core promise. Every command that changes the repository must be recorded so it can be undone, including commands that touch the working folder (decision 003's safety copies). This decision sets how that record works, what happens when two commands run at once, how crashes are survived, and how far back undo reaches.

## Options

- **A diary per line** (Git's reflog). Simple, but undo is manual, and nothing records discarded files, shelves or commands that touch several lines.
- **One diary for the whole repository, one changing command at a time.** Full undo/redo like an editor; changing commands take turns behind an OS-managed lock; readers never wait. (Sapling's undo is similar.)
- **One diary, lock-free** (jj). Concurrent changing commands both succeed and are merged on the next command. Never waits, but much more complex.

## Decision

**One diary for the whole repository; one changing command at a time.**
- **Diary entries** are objects in the store with their own fingerprint flavor (decision 005). Each records the complete state after the command (where every line points, where the user is, shelves), the safety copies it made, its parent entry, and metadata (command, time, user, Bolt version).
- **Committing a command:** the command writes one pack containing its new data and its diary entry (decision 009), then atomically replaces `.bolt/state/head` to point at the entry. That replacement is the moment the command takes effect.
- **Folder changes are journaled:** before a command rewrites files in the working folder, it records what it is about to do; after a crash, the next Bolt run finishes or rolls back the interrupted command.
- **Undo and redo** create new diary entries whose state copies an earlier one, and restore folder files from safety copies where the undone command changed them. They behave like an editor's Ctrl+Z / Ctrl+Y: a new command after an undo clears the redo path, but every entry stays in the diary, and undo can jump to any entry.
- **Things that reached others** (sent saves, locks): undo restores only the local repository; Bolt says so and offers the opposite action (v0.3).
- **Concurrency:** changing commands take an exclusive lock held through the operating system, released automatically if the process dies; a second changing command waits and says why. Read-only commands never take the lock: they read `head` once and see a consistent state.
- **Undo window:** 30 days by default (a personal setting). Safety copies can also have a size budget; when one is cleaned up, undo reports what it can no longer restore. Diary entries are small and kept longer than the data they point to.

## Consequences

- `state/` holds little more than the `head` pointer and the lock. Lines live inside diary entries, so there are no separate ref files to keep consistent.
- Cleanup (decision 009) treats everything reachable from diary entries inside the undo window as alive.
- Pack merging and cleanup also run under the lock, so they never race a command that is writing.
- Exact wording, the list view and step numbering are settled in topic 17; the example flow in design.md §5.8 shows the intended feel.
- Undo behaviour for sync and locks is designed with them in topics 14–15.
