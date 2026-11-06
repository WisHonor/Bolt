# 061 — Which copy is read first, now that packs are opened only when searched

- **Status:** Accepted
- **Date:** 2026-11-05
- **Agenda topic:** none (milestone 3, Task 5 review)
- **Decided by:** Claude, at the owner's request while the owner was away. The owner can reverse any part.
- **Refines:** [decision 029](029-red-team-m1-final-review-rulings.md) #1 (copies tried smallest first) and [decision 057](057-pack-index-on-disk-and-merging.md) (packs opened when searched, at most 16 open)

## Context

Decision 029 has reads try every copy of an object smallest recorded size first, so that a planted copy claiming a huge size is read last. To know every copy, a read has to search every pack. Decision 057 opens a pack only when it is searched and keeps at most 16 open, and it looks first in the pack that held the last object, as Git does. If every read searched every pack, that first look would gain nothing, and once there are more than 16 packs every read would open files again.

## Options

- **A. Search every pack on every read**, and keep 029's order exactly. This is correct, but it costs a file open per closed pack on every read, so reads slow down once there are more than 16 packs.
- **B. Read the first copy found**, and search further only when it fails. This is as fast as Git, but a planted copy found first can claim up to 1 TiB, which brings back 029's problem.
- **C. Read the first copy found only when it is small**, and otherwise fall back to 029's order.

## Decision

**C.**

- The first copy found is read at once if it records at most 1 MiB. Only that copy is ever held whole in memory: content is read into a buffer first, so a bad copy is passed over before any byte goes out.
- When the first copy records more, or fails, every pack is searched and the copies are tried smallest recorded size first, streamed, as decision 029 #1 says. Content held by several packs is checked before it is streamed out.
- Packs are searched in this order: the pack that held the last object read if it is open, then the other open packs, then the closed ones by name. A lookup that finds nothing then opens only the packs that don't fit among the 16 open ones.
- At most 16 packs are open at a time, however many packs hold the same object. Copies found don't keep their packs open.
- A version 1 pack's index is still read whole and held while the pack is open, so up to 16 are held at once. A planted version 1 index therefore still costs memory. That is no worse than in milestone 1, and the merge in decision 057 rewrites packs as version 2.

## Consequences

- A planted copy that claims a huge size is still read last. A planted copy found first and claiming at most 1 MiB costs a read of at most 1 MiB, and only once per process, because bad copies are remembered.
- With more than 16 packs, a lookup that finds nothing opens (packs − 16) files. Merging keeps the number of packs near 32.
- [storage-format.md](../storage-format.md) §11 and [design.md](../design.md) describe this order.
