# 082 — How big files are cut into chunks

- **Status:** Accepted
- **Date:** 2026-11-08
- **Agenda topic:** none (milestone 5 planning)
- **Refined by:** [decision 114](114-bolt-toml-keys-and-how-bolt-reads-it.md) (the threshold is `bolt.toml`'s `chunk-files-over`); [decision 103](103-where-git-keeps-less-zstd-level-3-and-the-chunk-sizes-stay.md) (the chunk sizes stay)
- **Decided by:** Claude, at the owner's request while the owner was away. The owner can reverse any part.
- **Refines:** [decision 009](009-storage-engine.md) (chunking: FastCDC from 1 MB, 16 KB / 64 KB / 256 KB to start); decisions [013](013-local-data-and-offline.md) and [016](016-presets.md) (the threshold is fixed at 1 MiB until the presets make it a `bolt.toml` setting)
- **Superseded in part by:** [decision 150](150-the-name-is-opalro.md) (the names: Bolt is now Opalro, and `bolt` is now `opal`)

## Context

A big file is stored as pieces (chunks) cut where its contents say, not every so many bytes, so that an edit in the middle of a file changes only the pieces around it and every other piece is stored once, however many versions share it. Decision 009 chose FastCDC, for files of 1 MB and up, with pieces of 16 KB at least, 64 KB on average and 256 KB at most, taken from casync and quoted from memory. Milestone 5 builds it. Left open: which form of FastCDC, the exact numbers, and how the cut points are pinned. Pieces are shared between versions, and from v0.3 between computers, only when everyone cuts the same file at the same places, so the cutting rule matters even though it changes no ID (decision 005).

## Options

- **A. Bolt's own cutter,** FastCDC as its 2020 paper gives it (normalized chunking at level 1: a harder test before the average size, an easier one after), with its table of 256 random numbers taken from BLAKE3 so the storage spec can state it, testing the top bits of the rolling hash (as Hugging Face Xet's gearhash does), and test vectors for the cut points. About 80 lines.
- **B. The `fastcdc` crate** (its 2020 form). Well tested and MIT-licensed, but the cut points then depend on that crate's own table and version, which Bolt's spec can't state and an update could change.
- **C. Fixed-size pieces.** Simplest, but one byte inserted near the start moves every later boundary, so two versions share almost nothing. Git LFS and Perforce don't cut files at all.

And the sizes: a smaller average (16 KB) finds more shared pieces but makes four times the objects; a larger one (restic's 1 MB, borg's 2 MB) makes fewer objects but stores more for each small edit. casync and Xet use 64 KB.

## Decision

**A, with decision 009's sizes in binary units.**

- **What is cut:** a file of at least 1 MiB (1,048,576 bytes) is cut into chunks, up to the limit below; a smaller one is stored whole. The 1 MiB is fixed until milestone 6's presets make it the `bolt.toml` setting decisions 013 and 016 describe (`chunk-files-over`); the setting will change which files are cut, never an ID.
- **Sizes:** at least 16 KiB (16,384 bytes), about 64 KiB (65,536) on average, at most 256 KiB (262,144). Only a file's last chunk can be shorter than the minimum.
- **The rule** (storage-format.md §6 gives it exactly): from the start of the bytes not yet cut, the first 16,384 are skipped; then a 64-bit hash takes in one byte at a time (shift left by one, add the table's number for the byte, wrapping), and the chunk ends after the first byte at which the hash's top 17 bits are all zero, before the 64 KiB mark, or its top 15 bits, after it, or at 256 KiB.
- **The table:** 256 numbers of 64 bits, little-endian, the first 2,048 bytes of BLAKE3's extendable output in key-derivation mode with the context string `bolt-vcs 2026-11-08 chunk-cutter gear v1`.
- **Pinned, not part of the format:** any chunk list that rebuilds the file is valid (decision 005), but Bolt's cutter is pinned by test vectors, so changing it is a decision of its own. A change costs sharing with what was cut before, never correctness.
- **A limit:** a file of more than 128 GiB (137,438,953,472 bytes) is stored whole, streamed, as every file is today. The table is public, so someone can make a file whose every chunk comes out at the minimum; below 128 GiB such a file still has at most 8,388,097 chunks, a list of under 300 MB, inside a chunk list's limits (8,388,608 chunks, 512 MiB; [decision 083](083-a-chunk-list-is-stored-under-its-files-id.md)). Files that big are rare, and chunking them waits for a list of lists.
- **Tuning** waits for milestone 5's benchmarks on real assets ([decision 088](088-benchmarks-and-the-speed-gates.md); risk 4).

## Consequences

- One byte inserted in the middle of a 3 MiB file changes one of its 40 chunks (storage-format.md §13's vector); the others are shared.
- How much versions share depends on the file's format: much for uncompressed assets and appended data, little for compressed ones (PNG, MP4, ZIP-based files) or a file rewritten whole (risk 4).
- The cutter reads one byte at a time on one thread; its speed is measured before anything faster (FastCDC's two-bytes-at-a-time form, say) is tried.
- storage-format.md §6 and §13 state the rule and its vectors before any code.
