# 057 — Packs are searched on disk, opened when needed, and merged when there are many

- **Status:** Accepted
- **Date:** 2026-11-05
- **Agenda topic:** none (milestone 3 planning, question 10; risk 17)
- **Decided by:** Claude, at the owner's request ("do whatever you think is best", 2026-11-05). The owner can reverse any part.
- **Refines:** [decision 009](009-storage-engine.md) (packs, merged later; a lookup index across packs) and [decision 028](028-red-team-m1-session-2-rulings.md) #2 (a pack's index read in bounded steps)
- **Refined by:** [decision 061](061-pack-copy-order-and-lazy-opening.md) (which copy is read first, the order packs are searched in, and the 16-open bound for shared objects) and [decision 062](062-when-packs-are-merged.md) (a merge runs when more than 32 packs are small, and small is judged by the index too)

## Context

Every save adds a pack, and an import adds several. Today every command opens every pack and reads its whole index into memory, so a long-lived project runs into the system's limit on open files (risk 17), and a planted pack with 2^24 index lines costs every command about 3 to 4 GB. A pack's index lines hold their sizes as varints, so they can't be searched on disk; they must be read in order.

## Options

- **A. Fixed-width index lines,** with a fan-out table first, like Git's `.idx`: a lookup reads a few lines by position, so nothing is held in memory. Packs open only when searched, and a few stay open. A format change, made before format 1 freezes.
- **B. Keep the format,** and keep a lookup index across all packs in `cache/` (decision 009's plan), rebuilt when packs change.
- **C. Keep reading every index,** and only cap the number of packs.

And when to merge: after a changing command, once there are many packs (Git merges at 50, `gc.autoPackLimit`), or only when asked (`tidy`, milestone 4).

## Decision

**A, merging automatically.**

- **Pack version 2:** after the entries, a fan-out table (256 counts), then the IDs in order (32 bytes each), then for each a fixed-width line (kind, offset, uncompressed size, stored size), so that line `i` sits at a known place. The trailer and its checksum stay. Readers keep reading version 1 packs whole; writers write version 2 only.
- **Lookups** read the fan-out once a pack is opened and then search the IDs by position: memory doesn't grow with a pack's size, so a planted index costs reads, not memory. A pack is opened when first searched, and at most 16 stay open, the least recently used closed first, as the Git reader does. The index's checksum is checked by `verify` (milestone 6), not on every open: every object read is checked against its own ID anyway (decision 028).
- **Merging:** when a changing command has committed and the store holds more than 32 packs, it merges, under the lock, every pack smaller than 64 MiB into one new pack. Each object is checked as it is copied, its stored bytes kept as they are. The new pack is made durable and renamed into the store before any old pack is removed; a crash leaves copies in two packs, which reads already treat as one object. A pack with a copy that fails its check isn't removed, and a warning names it. A reader that finds a pack gone looks again at the store's list.
- **Compressing again** at a stronger level when merging (decision 009) waits for `tidy` (milestone 4).

## Consequences

- [storage-format.md](../storage-format.md) §11 gets version 2's bytes, with a test vector, before code.
- Risk 17 is answered, and the follow-up about planted indexes' memory closes.
- Crash tests cover every step of a merge.
