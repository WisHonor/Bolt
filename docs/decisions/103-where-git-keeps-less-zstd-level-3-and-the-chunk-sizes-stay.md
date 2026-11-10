# 103 — Where Git keeps less: zstd level 3 and the chunk sizes stay

- **Status:** Accepted
- **Date:** 2026-11-10
- **Agenda topic:** none (milestone 6 planning, speed and space)
- **Decided by:** Claude, at the owner's request while the owner was away. The owner can reverse any part.
- **Refines:** [decision 009](009-storage-engine.md) (compression), [decision 082](082-how-big-files-are-cut-into-chunks.md) (the chunk sizes) and [decision 088](088-benchmarks-and-the-speed-gates.md) (what the benchmarks rule on)

## Context

The benchmarks show two places where Git keeps a project in less space than Bolt ([benchmarks.md](../benchmarks.md) §2 and §3):

- **Godot's demo projects:** 226.7 MiB in Bolt against 219.3 MiB in Git, 3% more.
- **One 200 MiB file in 50 versions, each overwriting 1% of it:** 351.9 MiB against 299.2 MiB, 18% more.

Both were looked into before deciding ([benchmarks.md](../benchmarks.md) §7).

**Godot.** Each file was stored each way and added up by kind. Of the 9.1 MB Bolt keeps beyond what Git's compression would, 7.0 MB is six `.hdr` images: Git's zlib at level 6 simply shrinks them more than zstd at level 3 does, whether the file is compressed whole or in chunks. It is not the chunks' doing, and it is not the rule that keeps a piece compressed only when that saves 5%: that rule costs 0.4%.

**The 200 MiB file.** Each version overwrites eight runs of 256 KiB. Git stores a version as the 2 MiB that changed. Bolt stores every chunk a run touches, and a run's first and last chunks are mostly unchanged bytes: with chunks of about 64 KiB, each run costs about 380 KiB, so a version costs about 3 MiB.

## Options

- **A. Leave both as they are, and publish them as Git's wins.**
- **B. Compress harder.** zstd at level 9 would save 2.5% on Godot's demos and still leave Bolt 1.1% behind Git, at about 4 times the compressing time. Decision 088 already measured this trade for packs that are rewritten, and ruled that no level pays.
- **C. Keep a piece compressed whenever it is smaller at all.** 0.4% on Godot's demos, and every read of such a piece then decompresses it for almost nothing.
- **D. Smaller chunks.** At a quarter of the size, the 200 MiB case would lose about two thirds of its extra. But every big file would have four times the chunks to list, find and check, a chunk list four times as long is stored with every version, and decision 082's pinned cut points would move, so nothing new would share chunks with anything stored before.
- **E. Deltas,** as Git has: a chunk, or a file, stored as its difference from another. It would close the 200 MiB case and the red team's RT4-1 ([decision 087](087-what-an-import-writes-once-files-are-chunked.md)). It is a new way of storing, so a format feature of its own (decision 009 names it for later).

## Decision

**A.**

- **zstd stays at level 3,** and a piece stays compressed only when that saves 5%. Bolt chose speed here on purpose: its first save of Godot's demos takes 1.1 s where Git's takes 8.6 s, for 3% more space.
- **The chunk sizes of decision 082 stay.** The case they lose is a file between 1 MiB and Git's 512 MiB limit that gets many small edits in scattered places. Past that limit Git keeps every version whole, and Bolt keeps a tenth of what Git does (ten versions of a 1 GiB file).
- **Deltas stay a later format feature.** Nothing in v0.1's format stands in their way.
- **A setting isn't added here.** A stronger level for chosen kinds of file, or no compression for kinds already compressed, would be a rule in `bolt.toml`; the follow-ups carry it as a row for the presets work.

## Consequences

- [benchmarks.md](../benchmarks.md) goes on publishing both as Git's wins, with what causes them.
- A game project heavy in `.hdr` or `.exr` images takes a few percent more in Bolt than in Git.
- A history of small scattered edits to a big file takes more in Bolt than in Git by about one chunk for each run of changed bytes.
- The [follow-ups](../plans/2026-11-01-milestone-1-follow-ups.md) gain a row for a compression rule in `bolt.toml`, and the row on deltas names chunks as well as records.
