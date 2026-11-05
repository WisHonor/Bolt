# 044 — Rulings after milestone 2's fixes and second red-team session

- **Status:** Accepted
- **Date:** 2026-11-03
- **Agenda topic:** none (follows [decision 024](024-red-team-gate.md) and [decision 043](043-red-team-m2-rulings.md))
- **Refines:** [decision 043](043-red-team-m2-rulings.md) (how far a change-id header is refused; which saved names are left out)
- **Reviewed:** 2026-11-04; the owner left the call to Claude, who kept it whole. Of the questions it left, rebuilding each delta chain once is scheduled for milestone 5, with big files; and no write limit is added for imports before chunked storage arrives, in the same milestone, when the question comes back.

## Context

The fixes for milestone 2's first red-team session (decision 043, report [2026-11-03-milestone-2.md](../redteam/2026-11-03-milestone-2.md)) were reviewed one by one, and a second session attacked them ([2026-11-03-milestone-2-session-2.md](../redteam/2026-11-03-milestone-2-session-2.md)). The fixes held where they were aimed. A few things were decided along the way, and these are written down here:

- **RT3-4:** the import limits how much work rebuilding Git's compressed objects may take. The first version of the limit grew with the packs' size on disk, which a sparse file supplies for free.
- **RT3-10:** decision 043 refuses a change-id header that an earlier commit already used. A hostile commit could still copy the ID that a plain commit (one without a header) gets from its own commit ID.
- **RT3-5:** decision 043 leaves every name Git treats as `.git` out of saves. A folder this computer can't hold is kept whole, as decision 039 says, so anything already inside it stays.
- **RT4-1:** Bolt stores each save's message and each distinct file whole, compressed. Git stores similar ones as small deltas. So a small Git repository can import into a very large `.bolt`: 976 KB became 1.46 GB in the second session.

## Options

- **RT3-4 allowance.** A: base it on the compressed bytes Bolt actually inflates from packs. B: a fixed ceiling. That refuses real repositories with big files sooner. C: rebuild each chain of deltas once, from its bottom base, as Git's `index-pack` does. That is the real fix, but a larger change.
- **RT3-10.** A: also refuse a header equal to the ID any commit gets from its own commit ID. B: keep "earlier" only.
- **RT3-5.** A: also open folders kept whole and drop names inside them. That changes decision 039's promise that such entries are kept exactly. B: keep them whole, and rely on writing files out (milestone 3) to check every name again.
- **RT4-1.** A: an import-wide limit on what it writes, relative to what it read. That refuses real repositories whose history Git compresses well, such as a large file edited thousands of times. B: accept whole records until the storage engine's chunks and deltas arrive ([decision 009](009-storage-engine.md); chunking comes in milestone 5), and say so.

## Decision

- **RT3-4: A for now.**
  - The allowance is the larger of 4 GiB and 500 times the compressed bytes Bolt inflated from pack entries, each entry counted once.
  - Zeros inside an entry's stream are still inflated and counted. So a hostile archive can buy roughly 35 to 1,000 times the time Bolt spends inflating, and can make an import slow.
  - The user can stop it at any time. Nothing is committed, and a re-run starts over.
  - C, rebuilding each chain once, keeping rebuilt objects in the cache, and building a big file once for both of its reads, is left for the owner to schedule.
  - Until then, an honest history of a big file (roughly 80 MiB or more) in a long delta chain can be refused, with a hint to repack.
- **RT3-10: A.** A change-id header is used only if no earlier commit used it and no commit's own ID gives it. The import stays repeatable.
- **RT3-5: B.** A folder kept whole keeps what the last save holds there, names Git treats as `.git` included. Only a save made before decision 043 or a planted `.bolt` can hold one. Writing files out (milestone 3) checks every name again.
- **RT4-1: B.**
  - An import can need far more disk than the Git repository it reads.
  - A full disk stops the import with nothing committed, and the hint says the unfinished `.bolt` can be deleted.
  - Whether to add a write limit before chunks and deltas arrive is the owner's question.

## Consequences

- [design.md](../design.md) §5.16 and [security.md](../security.md) (T7, T11) state the allowance, what it doesn't stop, and the disk use, as above.
- The follow-ups gain two questions for the owner: the chain-at-a-time rebuild, and a write limit for imports until chunks and deltas arrive.
