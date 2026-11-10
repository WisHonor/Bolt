# 087 — What an import writes, once files are chunked

- **Status:** Accepted
- **Date:** 2026-11-08
- **Agenda topic:** none (milestone 5 planning; the question decision 044's review left for when chunks arrive)
- **Decided by:** Claude, at the owner's request while the owner was away. The owner can reverse any part.
- **Refines:** [decision 044](044-red-team-m2-final-rulings.md) (RT4-1: no write limit for imports until chunked storage arrives)
- **Refined by:** [decision 109](109-the-speed-sessions-follow-ups-and-format-1s-three-open-rows.md) (what an import costs in memory: measured, and left for after v0.1)

## Context

Git stores similar things as small deltas; Bolt stores each save's message, each folder's listing and each distinct file whole, compressed. So a small Git repository can import into a very large `.bolt`: in milestone 2's second red-team session, 2,000 commits sharing one 1 MB message took under 1 MB in Git and about 1.5 GB in Bolt (RT4-1). Decision 044 accepted that until chunked storage arrived, with a clean stop on a full disk, and its review asked to look again then: does chunking end it, or does an import still need a limit on what it writes?

Chunking ([decisions 082](082-how-big-files-are-cut-into-chunks.md) and [083](083-a-chunk-list-is-stored-under-its-files-id.md)) covers file contents from 1 MiB up. It ends the case decision 044 gave for not limiting: a large file edited thousands of times now costs its changed chunks and a chunk list per version (about 35 bytes a chunk), not a whole copy per version. It does nothing for records: a message is part of its save, and a listing is one record, so 2,000 saves with one 1 MB message are still 2,000 records of 1 MB, and a folder of 100,000 entries changed in each of 10,000 commits is still 10,000 listings.

## Options

- **A. A limit on what an import writes, relative to what it read** (say, 50 times the Git objects it inflated). It stops RT4-1, but refuses honest repositories whose big folders change in many commits, which Git stores as tiny deltas.
- **B. A limit on records only** (on message bytes, say). An attacker moves to listings, which amplify more; a limit on listings refuses the same honest repositories as A.
- **C. No limit:** keep decision 044's clean stop on a full disk. The real fix is storing records as deltas (decision 009's later `feature deltas`), which is the format change that would make 2,000 near-identical records cost one.

## Decision

**C.** Chunking ends the big-file part of the problem; records stay whole, and an import has no write limit. A full disk stops the import with nothing committed, and the hint says the unfinished `.bolt` can be deleted or the import run again once there is room, as today. Storing records as deltas stays where decision 009 put it, a later storage feature, now named in the follow-ups as what ends RT4-1.

The red team's RT4-1 repository and an honest history of one big file (200 MiB, 50 versions changing 1% each) are measured with decision 088's benchmarks, Git's size against Bolt's, and published.

## Consequences

- A hostile repository can still make an import fill a disk; it can't make it commit anything, or corrupt what is there.
- An honest history of a big file now imports into about one whole copy plus each version's changed chunks, near what Git's deltas take; Git LFS keeps every version whole.
- security.md T7 says what remains.
