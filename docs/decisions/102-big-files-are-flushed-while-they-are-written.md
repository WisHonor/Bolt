# 102 — Big files are flushed while they are written

- **Status:** Accepted
- **Date:** 2026-11-10
- **Agenda topic:** none (milestone 6 planning, speed and space)
- **Decided by:** Claude, at the owner's request while the owner was away. The owner can reverse any part.
- **Refines:** [decision 009](009-storage-engine.md) (how a pack is made durable) and [decision 049](049-writing-files-one-at-a-time.md) (how a file a switch writes is made durable)

## Context

Bolt flushes a pack to the disk before it moves it into the store, and a file a switch writes before it renames it into place, so a crash can't leave either half written. Both flushes come at the very end. For a big file that is the largest cost Bolt has ([benchmarks.md](../benchmarks.md) §7, on GitHub's runners):

- **First save of 1,000 MiB that doesn't compress:** 4.2 s. Reading, cutting, hashing and writing take 1.9 s, and then the one flush takes 2.3 s. Git LFS takes 2.5 s, and never flushes.
- **Going back to an old version of a 1 GiB file:** 3.9 s. Copying the chunks out takes 1.3 s, then the flush 2.3 s. Git takes 3.8 s on average, without a flush.

While Bolt reads and hashes, the disk has nothing to do: the system keeps the written bytes in memory until the flush asks for them. So the two costs add up, where they could overlap.

## Options

- **A. Keep one flush at the end.** Nothing changes; Bolt stays behind Git LFS on a first save and behind Git on going back.
- **B. Don't flush, as Git LFS and Git's checkout don't.** Fast, and against principle 1: after a power cut a pack in the store, or a file in the folder, could be empty or cut short.
- **C. Flush as the file is written, and once more at the end.** A helper thread asks the system to write out what has been written so far, every 16 MiB, while the writing goes on. The last flush then has little left to do. What reaches the store or the folder is exactly as durable as today.
- **D. Write past the system's memory altogether** (unbuffered, or synced, writes). Each write waits for the disk, which is slower than C for the same safety, and Windows asks for aligned buffers.

## Decision

**C.**

- **When.** Once a pack, or a file a switch is writing, has grown past 16 MiB, a helper thread flushes its data every further 16 MiB written, through a second handle to the same open file, while the writer goes on. Smaller files never start one.
- **The end is unchanged.** When the file is complete the helper stops, then the whole file is flushed as today, and only then is it renamed into place. Nothing is renamed sooner, and a file in `tmp/` still means nothing to anyone.
- **An error is never lost.** If one of the helper's flushes fails, the command fails with that error when the file is finished, as it would if the last flush failed. (A system may report a failed flush once only, so the helper's error can't be left for the last flush to find.)
- **What is written doesn't change.** The same bytes go into the same packs in the same order; only when the disk receives them changes. Crash tests stop at the same points.

## Consequences

- A first save of big files that don't compress, and going back to an old version of a big file, should take about as long as the slower of the two halves (reading and hashing, or the disk), not their sum. The numbers are published either way ([benchmarks.md](../benchmarks.md)).
- A command that fails halfway may have flushed part of a file in `tmp/`, which the next command clears, as it clears any leftover.
- One more thread runs while a big file is written. It holds no memory beyond its handle.
- On a drive where a flush is very slow (some network drives), the flushes along the way cost what the last one would have cost; the total doesn't grow, but progress pauses more often.
- [storage-format.md](../storage-format.md) §11 and [design.md](../design.md) §5.7 and §5.9 say when a pack and a written file are flushed.
- Tests: a helper's flush that fails stops the command with nothing renamed into place; a save writes the same packs with and without the helper; the crash sweeps of big files stop at the same places.
