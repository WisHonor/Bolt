# 100 — Caches are written without a flush, and the file cache only when it changed

- **Status:** Accepted
- **Date:** 2026-11-10
- **Agenda topic:** none (milestone 6 planning, speed and space)
- **Decided by:** Claude, at the owner's request while the owner was away. The owner can reverse any part.
- **Refines:** [decision 053](053-file-state-cache.md) (how and when the file-state cache is written) and [decision 011](011-status-and-write-out.md) (how Bolt's own files in `.bolt` are replaced)

## Context

Every `status`, every `save` and the plan of every `goto` ends by writing `cache/files` again, whole, even when it holds exactly what it held before. The write goes through a file in `tmp/`, which is flushed to the disk, then a rename, then a flush of `cache/`. So on Linux and macOS an unchanged `status` flushes the disk four times: twice here, once for the file that gives the drive's clock, and once for the drive test's rename. A `save` flushes twelve times and a `goto` sixteen (Windows has no flush for a folder, so it makes about half as many). On GitHub's Linux runner a flush takes 0.2 to 0.7 ms, and writing the cache 2.4 to 4 ms of a 63 ms `status` ([benchmarks.md](../benchmarks.md) §7). Git writes its index without a flush.

A flush buys one thing: after a power cut, the file holds what was written. For a cache that is worth nothing. A cache is only a hint, and every cache file is sealed with a keyed checksum ([storage-format.md](../storage-format.md) §10): one that a crash left empty, cut short or mixed fails the seal and is rebuilt. An older cache that survived whole is as good as it was before the command ran: each entry is still trusted only if the file matches it.

Two more files are flushed for no gain. `state/lock-info` only feeds the "waiting for another command" message, and the spec already says it may be stale. The drive test renames a throwaway file in `tmp/` and flushes the folder, then deletes the file.

## Options

- **A. Keep every flush.** Nothing to argue, and four flushes on every `status`.
- **B. Write caches without a flush, and the file cache only when it changed.** The same bytes are written, less often and less expensively. The seal already covers what a crash can leave.
- **C. Also drop the flush of the file that gives the drive's clock.** On a local drive a file's time is set when it is written, so the flush changes nothing there. But decision 053 reads "now" from that file so that the drive's own clock counts, and some network drives set a file's time only when its bytes reach them. It saves about 0.5 ms.

## Decision

**B.** Not C: the 2-second rule's clock stays exactly as it is, since the gain is small and "nil risk" can't be shown for every network drive.

- **Caches aren't flushed.** `cache/files` and `cache/change-ids` are still written whole into `tmp/` and renamed into place, so a reader never sees half a file. Neither the file nor `cache/` is flushed.
- **The file cache is written only when it changed.** A scan writes `cache/files` only if what it would write differs from what it read: an entry added, dropped or changed, or an entry that becomes racy or stops being racy against this scan's "now". Entries are marked racy exactly as before, so the 2-second rule decides the same things; when the result is the cache already on disk, nothing is written.
- **"Now" is still read at the start of every scan,** from a file written and flushed in `tmp/`, before any file is looked at.
- **`state/lock-info` isn't flushed.** It is replaced through `tmp/` and a rename as before.
- **The drive test's rename isn't flushed.**
- **Everything a command's result rests on is still flushed** as before: packs, `state/head`, `state/journal` and its removal, `format`, each file a switch writes and the folders it changes, and the cache key.

## Consequences

- An unchanged `status` flushes the disk once, not four times, and writes nothing but its two throwaway files in `tmp/`.
- After a power cut a cache can be missing, older or torn. A torn one fails its seal and is rebuilt; the next `status` then reads every file once. No command's result changes.
- [storage-format.md](../storage-format.md) §10 and [design.md](../design.md) §5.9 say what is flushed and when the cache is written. The cache's format stays version 3.
- Tests: an unchanged `status` leaves `cache/files` untouched (the same file, not a new one with the same bytes); a cache cut short or zeroed is ignored and rebuilt; an entry that turns racy or stops being racy is written.
