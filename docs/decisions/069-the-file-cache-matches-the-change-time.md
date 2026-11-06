# 069 — The file-state cache matches the change time too

- **Status:** Accepted
- **Date:** 2026-11-06
- **Agenda topic:** none (milestone 3, its second red-team session, RT5-5)
- **Decided by:** Claude, at the owner's request while the owner was away. The owner can reverse any part.
- **Refines:** [decision 053](053-file-state-cache.md) (what the cache records) and [decision 067](067-the-file-cache-matches-file-identity.md) (its option B, left for later, is taken now)

## Context

Decision 067 made a cache entry match the file's ID, so a file moved or copied onto a saved path is read. A file overwritten in place keeps its ID: `cp -p`, `unzip -o`, `tar --overwrite` and `rsync --inplace -t` empty the existing file, write the new bytes and give it the source's modified time. With the same size and the shared times decision 067 describes (npm, archives, FAT), the cache still vouched for content that wasn't there, and `goto` replaced it without a safety copy. The second red-team session proved it (RT5-5).

Every system keeps a second time that no ordinary call sets: the change time, moved to "now" whenever the file's contents or details change, setting its modified time included. Git's index matches it (`ctime`) for this reason.

## Options

- **A. Match the change time too.** On Unix it is `st_ctime`, which the platform crate's `stat` already returns. On Windows it is `ChangeTime` from `FILE_BASIC_INFO`, read through the platform crate's existing `info` call; that needs one more `unsafe impl` of its `Plain` promise, for the owner's review with the rest. It also closes decision 067's known limit: a file made again in a freed inode has a new change time.
- **B. Hash anything `goto` or `discard` is about to overwrite without a safety copy.** Safe whatever the cache says, but every switch then reads every file it replaces, and `status` stays wrong.
- **C. Both.** B as a backstop costs the reads above for a case A already covers.

## Decision

**A.**

- **Each entry records the file's change time** to the nanosecond, and is trusted only when the file has that change time as well as its kind, size, modified time and ID. The cache's version goes to 3; an older cache is ignored and rebuilt.
- **The racy rule stays on the modified time alone,** as Git's does. Counting the change time too would read again every file just unpacked with an old time, which is what the cache is for.
- **Write-out** asks the cache the same question through the same call before it replaces a file, so it is covered with no code of its own.

## Consequences

- An in-place overwrite that keeps the size and modified time is seen by `status` and protected by `goto` and `discard`.
- **Known limit:** a program that sets the change time back. On Unix only the kernel or a privileged tool that changes the clock can; on Windows any program allowed to write the file can, through `SetFileInformationByHandle`. A drive that keeps no change time of its own (FAT and exFAT, where systems report one that follows the modified time) gives no protection beyond decision 067's. [security.md](../security.md) T25 lists both.
- **Known limit:** a file written over in place within one tick of the change-time clock after Bolt read it (milliseconds on Linux, 100 ns on Windows), keeping its size and an old modified time, still matches.
- [storage-format.md](../storage-format.md) §10 describes version 3 of `cache/files`, with its test vector.
