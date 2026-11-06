# 076 — Renames, removals and new folders reach the disk on Windows too

- **Status:** Accepted
- **Date:** 2026-11-06
- **Agenda topic:** none (milestone 4 planning; a must-do follow-up from milestone 1)
- **Decided by:** Claude, at the owner's request while the owner was away. The owner can reverse any part.
- **Refines:** [decision 009](009-storage-engine.md) (packs made durable, then renamed into the store) and [decision 049](049-writing-files-one-at-a-time.md) (the journal removed once its switch is done)

## Context

A command takes effect when `state/head` is renamed into place (decision 010), and a switch's journal is removed once its files are written (decision 049). On macOS and Linux Bolt then flushes the folder (`fsync`), so the rename survives a power cut. On Windows it can't: a folder Bolt holds open may only be listed, and Windows flushes only what is open for writing. NTFS logs renames in order, so a power cut can't mix them up, but it can lose the newest ones: a save Bolt reported as made could be gone after the restart. The milestone 1 follow-up asks for `MOVEFILE_WRITE_THROUGH`. But Bolt renames through folder handles, never paths (decision 048), and `MoveFileExW`, which takes that flag, works on paths.

## Options

- **A. Write-through on the handle that changes the name.** Bolt opens the file it renames, and the entry it removes, with `FILE_WRITE_THROUGH`, and makes folders with it, through the calls it already makes. The flag asks the file system to finish what is done through the handle on the disk before the call returns; Microsoft documents it for data written through the handle, and it is how `MOVEFILE_WRITE_THROUGH` is understood to make a move durable. No new `unsafe` code: only the options passed to the existing call change.
- **B. Flush the renamed file.** Open it for writing after the rename and call `FlushFileBuffers`, relying on NTFS committing its whole log with it. What a flush does to a file's name is undocumented too, and it adds an open for each rename.
- **C. `MoveFileExW` by path.** Breaks decision 048: a path can lead through a link planted between two calls.

## Decision

**A.** On Windows the platform crate opens with `FILE_WRITE_THROUGH` every file it renames and every entry it removes, and makes every folder with it. That covers `head`, the journal, packs, caches and the files a switch writes into the project. `Dir::sync` stays empty on Windows, and its comment says why: each change was written through.

The two Windows rename gaps milestone 3 deferred go with it. On FAT and exFAT, which have no POSIX renames, a read-only file being replaced or removed gets its read-only flag cleared first, through a handle opened to write its attributes, as Bolt's POSIX rename already ignores the flag on NTFS.

## Consequences

- Each rename, removal and new folder waits for the disk, as `fsync` makes them wait on macOS and Linux. A switch that writes many files gets slower on Windows by about one flush a file; `status`, which writes nothing, is unchanged.
- Nothing short of a power cut proves what survives one. The tests check that every rename, removal and new folder still works on NTFS, and the read-only cases where a test can make them; the owner can pull the plug on a virtual machine mid-save to check the rest.
- The milestone 3 `unsafe` walkthrough's blocks keep their code; the flag is one more option passed through the safe function that calls `NtCreateFile`.
