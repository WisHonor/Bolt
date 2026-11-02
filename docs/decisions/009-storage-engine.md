# 009 — Storage engine: pack files

- **Status:** Accepted
- **Date:** 2026-10-31
- **Agenda topic:** 9
- **Refined by:** [decision 041](041-several-packs-per-command.md) (a command writes several packs when it needs more objects than one may hold)

## Context

Decision 005 fixed what every piece of data is called; this decision fixes how the bytes sit on disk in `.bolt/store/`: grouping, compression, chunking and cleanup. None of it changes any ID, so all of it can be improved later, but the starting design must be fast on Windows with big files and impossible to corrupt.

## Options

- **One file per object** (Git's loose objects). Simplest and trivially crash-safe, but a 10 GB game project becomes about 160,000 files, which is slow on Windows (antivirus scans each new file).
- **Pack files** (Git packfiles, restic, borg, Hugging Face Xet). All new data from one command goes into one pack with an index. Few files, fast on Windows, efficient with millions of chunks, and a natural unit for sync and cloud storage; more code (indexes, merging, cleanup by rewriting packs).
- **An embedded database** (SQLite, redb; Fossil uses SQLite). Transactions for free, but a poor fit for tens of GB: the file doesn't shrink without a rewrite, one damaged file can take everything with it, and the format depends on a third party.

## Decision

**Pack files.**
- **Packs:** each command that stores data writes one new pack (data plus an index at its end, with its own checksum) into `.bolt/tmp/`, makes it durable, then renames it into the store. A pack becomes visible only when complete, so a crash never leaves half a pack. Very large writes may be split across several packs. Small packs are merged later.
- **Chunking:** files of 1 MB and up are cut with FastCDC. Starting parameters, taken from casync: 16 KB minimum, 64 KB average, 256 KB maximum. The threshold and parameters can be tuned at any time (decision 005).
- **Compression:** zstd, at a fast level when saving and a stronger level when merging packs. Each chunk is compressed on its own; data that barely compresses is stored raw. Every stored piece records its uncompressed size, and decompression never goes past it.
- **Deltas** for small text files come later, as a storage feature listed in `.bolt/format`: zstd compression against an older version, with regular full copies to keep chains short. v0.1 stores small files whole and compressed.
- **Cleanup:** keeps everything reachable from lines, shelves and the undo window (topic 10); never deletes anything younger than a grace period; writes new packs before deleting old ones. Cheap cleanup (dropping packs that are entirely garbage) runs automatically; repacking runs only when the user asks, and Bolt estimates the space it would free.
- **Checking:** every object is verified against its fingerprint when read.

## Consequences

- Fast lookups need an index across all packs, kept in `cache/` and rebuildable from the packs.
- Dedup savings depend on file formats (risk 4); the chunking parameters are a starting point to tune with real game assets (topic 18).
- Packs are the natural unit for syncing and server storage later (topic 15), which may use a different pack size.
- Until deltas exist, imports of very large Git histories use more disk than Git; differential testing starts with smaller repositories.
- Rules for commands that write or merge packs at the same time are settled with concurrency in topic 10.
- casync's exact default chunk sizes were quoted from memory; they only seed the starting parameters.
