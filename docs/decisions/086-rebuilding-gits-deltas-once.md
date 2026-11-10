# 086 — An import rebuilds each of Git's delta families once

- **Status:** Accepted
- **Date:** 2026-11-08
- **Agenda topic:** none (milestone 5 planning; scheduled by decision 044's review)
- **Decided by:** Claude, at the owner's request while the owner was away. The owner can reverse any part.
- **Refines:** [decision 044](044-red-team-m2-final-rulings.md) (RT3-4: option C, rebuilding each chain once, which its review scheduled for milestone 5) and [decision 031](031-own-git-reader.md) (how the Git reader rebuilds objects)
- **Refined by:** [decision 091](091-the-work-stop-names-a-way-out-for-long-histories.md) (the stop past the allowance says it may be a long history of small edits to a big file, and its hint has Git store files over 1 MiB whole) and [decision 092](092-a-family-rebuild-stores-only-files-big-enough-to-cut.md) (a family rebuild stores only members of at least 1 MiB)

## Context

Git stores most versions of a file as a delta, a short recipe that rebuilds it from another version, and those recipes chain: version 1 from version 2, version 2 from version 3, and so on, down to one version stored whole. Git puts the newest version at the bottom, stored whole. Bolt's import converts commits oldest first, so its first request for a file walks the whole chain from the newest version back, and the next request, one version newer, walks nearly all of it again. Its cache doesn't save it: rebuilding takes the base it builds on out of the cache, never puts back what it built, never keeps anything over 256 MiB, and a file over 64 MiB is read twice in a row. So the work grows with the square of the number of versions, and the import's work allowance (decision 044) stops honest histories of big files: a 200 MiB file in 50 versions needs about 251 GiB of rebuilding against an allowance of about 146 GiB. Decision 044's review scheduled the fix: rebuild each chain once from its base, as Git's own `index-pack` does; keep rebuilt objects in the cache; and rebuild a big file once for both reads.

## Options

- **A. Fix the cache only:** keep what was built, don't take a base out, keep bigger objects. It helps, but the order is still wrong: a request walks down from the newest version, and with versions of hundreds of MiB a cache holds only a few, so most requests still start at the bottom.
- **B. Rebuild every pack whole before converting,** as `index-pack` does: every object once, bases first. But it builds objects the import never needs (unreachable ones, and every tree and commit, which are small and cheap anyway), and holds tables for the whole pack at once.
- **C. One family at a time:** the first time the import needs a big file that Git stored as a delta, it rebuilds that file's whole family in its pack (the object stored whole at the bottom and every object built from it, directly or through others), each object once, from the bottom up, storing each file as it is built and remembering its ID. Later versions are then found already stored. Everything else keeps today's path, with the cache fixed.

## Decision

**C, with the cache fixed and one read per file.**

- **When:** the first time the import needs a file Git stored as a delta and that delta builds at least 1 MiB (big enough to be cut into chunks), it finds the bottom of its chain, then every entry of that pack built on it, and rebuilds them all, each from the one it is built on, depth first. Each file built is stored as any file is (chunked from 1 MiB) and its Git ID mapped to its Bolt ID, so no later request rebuilds it. Every object in a family has the bottom's type, so a family is all files.
- **Finding a family:** a pass over the pack's entry headers, in pack order, notes each delta's base. It runs once per pack, only for a pack where a family is rebuilt, and keeps at most 8 bytes an entry: about 128 MiB at the 2^24 objects a repository's packs may list, which keeps the reader's tables under 1 GiB.
- **Memory:** as today, rebuilding holds the base and what it builds, each at most 512 MiB, and nothing else whole. An object whose family has more objects built from it still to come is kept in the base cache's 256 MiB while it fits, and dropped after its last; one that doesn't fit is written to a file in `.bolt/tmp` and read back, whole, when the next comes, so it isn't built twice. At most 4 GiB of such files exist at once; past that, an object that doesn't fit is built again from the bottom when it is needed, counted against the allowance. A chain where each version is built from the next, Git's usual shape, needs no such keeping. The import's rebuilding so holds at most about 1.3 GiB (two objects of 512 MiB and the cache), with the chunk work of decision 084 on top.
- **Objects no imported commit names** in a family (versions only reflogs, the stash, notes or `refs/original` keep, and unreachable ones `git gc` hasn't pruned yet) are built and stored too, costing their time and disk; `tidy` removes them later.
- **A member that fails:** an object of the family whose delta can't be applied, or whose result isn't the object its pack index names, is neither stored nor mapped; one that was built still serves as a base for the objects built from it, each checked against its own name. The import stops on such an object only if a commit it brings in needs it, as it would today.
- **The cache, for everything else** (trees, commits, small files): a base it uses stays in the cache, the object it builds goes in too, a hit counts as recent use, and an object up to the 256 MiB budget is kept.
- **One read:** a file over 64 MiB that Git stored whole is still streamed; it is now read once, hashed and cut in the same pass ([decision 084](084-files-are-read-once-on-several-threads.md)), not twice.
- **The allowance stays** as decision 044 set it: a few bytes of delta can still build 512 MiB once, and a hostile pack can have millions of such deltas. Now each object is built once, so honest histories fit: the 200 MiB file in 50 versions needs about 10 GiB of rebuilding.

## Consequences

- Honest histories of big files, up to the 512 MiB Git deltifies, import without the repack hint; the stop past the allowance now points only at repositories built to make importing slow, and its hint says so.
- A hostile pack can no longer buy repeated rebuilding of one chain with zeros in its streams; it can still buy one rebuild per delta, within the allowance, and, past 4 GiB of kept objects, rebuilding of a branching family of objects too big for the cache, also within the allowance.
- An import that stops and is run again rebuilds the families it meets again, and finds what it stored already there.
- design.md §5.16 and security.md T7 and T11 describe the new behaviour; the red-team tests that time a hostile chain keep their bounds.
