# 084 — Files are read once, and hashed and compressed on several threads

- **Status:** Accepted
- **Date:** 2026-11-08
- **Agenda topic:** none (milestone 5 planning)
- **Decided by:** Claude, at the owner's request while the owner was away. The owner can reverse any part.
- **Refines:** [decision 053](053-file-state-cache.md) (how `save` and `status` read the files the cache doesn't vouch for)

## Context

Today `bolt save` reads each new or changed file twice, on one thread: once to learn its ID, then, if the store doesn't hold it, again to store it. `bolt status` reads such files once, also on one thread. Only the look at each file's size and times runs on several threads (up to 6). Milestone 5 cuts big files into chunks ([decision 082](082-how-big-files-are-cut-into-chunks.md)) and asks for parallel hashing. Two facts shape it: a pack's name is the hash of its index, which records where each object sits, so a command writes the same packs only if it writes in the same order; and a save reports its first problem in a fixed order (decision 053's walk, and the order it stores in), whatever the timing.

## Options

- **A. Read once; hash, check and compress on worker threads; write on one.** Each file is read once: a small one whole, a big one cut into chunks as it is read. Workers read files and chunks, hash them, check the store for a copy and compress what is new; one thread takes their results in the save's own order and writes the packs.
- **B. One pack writer per worker,** packs finished separately. Faster still on many cores, but which objects land in which pack, and the packs' names, would depend on timing, and crash tests that stop at the Nth write would no longer stop at the same place.
- **C. Only BLAKE3's own threads** (its `rayon` feature) for big files. A new dependency, and it speeds up hashing alone, while compressing is the slower step (zstd's fast level manages a few hundred MB/s on one core).
- **D. Stay on one thread** until measurements ask for more.

## Decision

**A.**

- **One read.** Every writer of file contents (`save`, the safety copies of `goto`, `discard`, `shelf` and undo, an import) stores a file of up to 128 GiB in one read. A file under 1 MiB is read whole, hashed, and stored unless the store already holds a good copy. A bigger one is cut as it is read: the whole file is hashed as it passes, each chunk is hashed and stored unless the store holds a good copy of it, and then the file's chunk list is stored unless the store already holds that same list ([decision 083](083-a-chunk-list-is-stored-under-its-files-id.md)). A file over 128 GiB, stored whole ([decision 082](082-how-big-files-are-cut-into-chunks.md)), keeps today's two reads. What is stored is exactly what was hashed, so the second read that today also compares a file's contents isn't needed: a file that changes while it is read is caught by its size, modified time and change time after the read (`ChangedDuringSave`, decision 069), as today's first read is, and chunks already stored for it are left for `tidy`. Known limit: on a drive with no change time (FAT, exFAT), a write of the same size that puts the modified time back is not caught, where today's second read would see the bytes differ.
- **Workers.** `save`, `status` and the safety copies read, hash and check files, and hash, check and compress chunks, on worker threads: as many as the machine runs at once, at most 8. A worker checks the store itself, since the store is safe to share, so checking stored copies (decision 028) runs in parallel too. Status only hashes.
- **One writer, a fixed order.** One thread takes the workers' results in the order the command stores in today and writes every pack, so a command writes the same packs and reports the same first problem whatever the timing, and every pack fault point is met on that one thread. The writer counts each job from when it hands it out until it has taken the result, at the most that job can hold (what it reads plus the most its result can be), and hands out no more while that count is over about 64 MiB or 4 jobs a worker are out; workers never wait on the writer, so memory stays bounded and nothing can deadlock.
- **On the reading thread:** the whole-file hash and the cutter, which are sequential by nature. BLAKE3 already uses the processor's vector units on one thread, and decision 088's benchmarks measure the cutter before anything is split further.
- **Test hooks** a test sets on its own thread (a file made unreadable, a fault point armed) reach the workers it starts.
- **Not parallel:** write-out still writes one file at a time (decision 049), and an import reads Git's objects one at a time; its chunks are hashed and compressed on the workers.

## Consequences

- A new or changed file is read once instead of twice; a big one is stored as it is read.
- A save of many new files, or a status after many files changed, uses the machine's cores; the gain is measured, and published, with decision 088's benchmarks.
- Memory per worker is about one file (under 1 MiB) or one chunk (at most 256 KiB) and its compressed copy, all counted in the 64 MiB window: far under decision 023's 500 MB for a 10 GB file.
