# 067 — The file-state cache matches a file's identity too

- **Status:** Accepted
- **Date:** 2026-11-06
- **Agenda topic:** none (milestone 3, its red-team session, RT5-1)
- **Decided by:** Claude, at the owner's request while the owner was away. The owner can reverse any part.
- **Refines:** [decision 053](053-file-state-cache.md) (what the cache records and when an entry is trusted)

## Context

Decision 053 trusts a cache entry when the file at its path has the recorded kind, size and modified time. Nothing ties the entry to the file itself. Moving one file onto another's path keeps the moved file's own time, so if the two share a size and a time, the cache vouches for content that isn't there. Shared times are common: npm gives every file in a package the same time, archive and reproducible-build tools give a whole tree one, and FAT and HFS+ give one time to everything written in the same tick. `status` then shows nothing, and `goto` or `discard` overwrite the file without a safety copy, so its bytes are lost. The red-team session proved it (RT5-1).

Decision 053 set aside the drive's file ID because Rust gave none on Windows without new `unsafe`. Milestone 3's platform crate now reads it everywhere: the inode and device on Unix, the file ID and volume serial on Windows, already used to notice a file that changed while it was read.

## Options

- **A. Match the file ID too.** A file moved or copied onto the path is another file, with another ID, so the cache misses and the file is read. One more field per entry, no new `unsafe`.
- **B. Also match the change time** where the system gives one (Unix `ctime`, as Git does). It catches a file deleted and made again in the same inode with the same size and time. Windows has it only through a call the platform crate doesn't make yet, which is new `unsafe`.
- **C. Never let the cache decide what is overwritten:** hash every file `goto` or `discard` is about to replace. Safe whatever the cache says, but a switch then reads every file it changes, and `status` stays wrong.

## Decision

**A.**

- **Each entry records the file's ID** (the drive's number and the file's number on it), and is trusted only when the file at the path has that ID as well as the recorded kind, size and time.
- **A file whose number is 0 is never trusted.** Some network and FUSE drives report 0 for every file, which says nothing about which file it is. Trusting nothing there costs speed only: every such file is read each time, as with no cache. The cache's version goes to 2, so a version 1 cache is simply ignored and rebuilt.
- **A drive whose IDs aren't stable** (FAT numbers a file by where its entry sits, and Linux makes up new ones each time the drive is mounted) mostly just misses more often: a file renumbered is read again, which is never wrong, only slower. Such IDs are used like any other; telling such a drive apart needs a call the platform crate doesn't make yet.

## Consequences

- Moving, copying or unpacking a file onto a saved path is seen by `status` and protected by `goto` and `discard`, even with the same size and time.
- **Known limit:** on Unix a file deleted and made again can be given the freed inode. If it also has the old size and modified time (an unpacker that sets the archive's time over an older copy of the same archive, say), the cache still trusts it. FAT can do the same when a renamed file's entry lands in the slot the replaced file's entry left. Option B closes this on Unix and is left for later; [security.md](../security.md) §4 lists it.
- [storage-format.md](../storage-format.md) §10 describes version 2 of `cache/files`, with its test vector.
