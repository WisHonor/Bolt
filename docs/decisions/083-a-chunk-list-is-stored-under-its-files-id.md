# 083 — A chunk list is stored under its file's ID

- **Status:** Accepted
- **Date:** 2026-11-08
- **Agenda topic:** none (milestone 5 planning)
- **Decided by:** Claude, at the owner's request while the owner was away. The owner can reverse any part.
- **Supersedes:** [decision 019](019-review-clarifications.md) #10 (finding a chunk list through an index in `cache/`)
- **Refines:** [decision 005](005-object-identity.md) (how a chunk list is linked to its file's ID), [decision 028](028-red-team-m1-session-2-rulings.md) #1 (when a stored chunk list counts as a good copy), [decision 057](057-pack-index-on-disk-and-merging.md) (what merging checks, and which packs are small) and [decision 077](077-tidy-and-the-undo-window.md) (`tidy` follows chunk lists, and stops at a pack it can't read)

## Context

A folder listing names a file by its ID, the BLAKE3 hash of its whole contents (decision 005), however the file is stored. A big file is stored as chunks plus a chunk list: the record that says which chunks, in order, make the file ([storage-format.md](../storage-format.md) §6, drafted with kind 18 and its own context string). So reading a big file starts from its ID and must find its chunk list. Decision 019 #10 put that link in an index in `cache/`, rebuilt from the chunk lists. But a cache is sealed with the user's key and thrown away when it isn't trusted (decision 053): after a copy, a new key, or a lost cache, every read of a big file would need a scan of every pack for chunk lists. A cache can never vouch for an object (decision 028), so a miss must always end in that scan, and `tidy` could never rely on it: a stale cache would delete chunks still needed.

## Options

- **A. The cache of decision 019.** As above.
- **B. Store the chunk list under the file's own ID.** A lookup of a file's ID then finds either its whole copy (kind 1) or its chunk list (kind 18), with no cache and no scan. The list's pack line records the file's size, so every rule that judges copies by their size works unchanged. The cost: the list can't be checked against an ID of its own. It is checked when used: it must decode and name the right file and size, each chunk is checked as it is read, and the whole file against its ID. An ID of its own would prove nothing anyway, since nothing trusted names it: the listing names the file.
- **C. The list under its own ID, plus a small pointer record under the file's ID.** Two objects per big file, and the pointer can be checked no better than B's list.

## Decision

**B.**

- **In a pack** ([storage-format.md](../storage-format.md) §11): a chunk list is an entry of kind 18 whose ID is its file's ID, whose uncompressed size is the file's size and whose stored size is the list's own length, at most 512 MiB, stored without compression.
- **The record** is §6's, unchanged: version 1, the file's ID and size, the chunk count (1 to 16,777,216), then each chunk's ID and size (at least 1), in file order; the sizes must add up to the file's size, and the record's ID and size must be its entry's. Its context string stays reserved, unused.
- **Chunks are content stored whole** (kind 1). A reader looking for a chunk passes over a chunk list stored under that ID, so reading a file never goes deeper than one list.
- **Reading a file:** a whole copy and a chunk list are both copies of the file, tried in the order the store tries copies today (decisions 028 and 061). A list streams its chunks in order, each found and checked as content, a chunk's bad copy passed over for a good one as for any object. The bytes are hashed as they go and must give the file's ID at the end; until then they are untrusted, as a lone big copy streamed today is (write-out writes into `.bolt/tmp` and renames only after).
- **A good copy, before relying on it:** a writer about to skip storing a file must, as now, have read a copy and checked it. A whole copy is checked as today. A chunk list counts once the writer has itself cut the same file into exactly the chunks the list names, each checked or stored in that same pass; any other existing list is not read through: the writer stores its own list, which is small, beside it.
- **Merging** checks a chunk list by its structure (it decodes, and names its entry's file and size), not by rebuilding the file; and a pack counts as small by its chunk lists' stored sizes, not their files' sizes, so a save's last pack, which holds its big files' lists, still merges.
- **`tidy`** follows chunk lists: each chunk list of a file it keeps, and each chunk list in a pack it keeps whole (one changed in the last 24 hours), keeps every chunk it names. It also stops, removing nothing, when it can't open a pack or read and check its index: such a pack could hold the only list that names chunks in other packs. Today it leaves such a pack with a warning. Conflict records still stop it.
- **No feature line:** chunk lists are part of format 1 as drafted from the start (§6, kind 18). A development build from before milestone 5 reads one as damage: it reads no file stored that way, and its `tidy` stops at it.
- **Checking the store** (the internal check behind `bolt verify`, milestone 6) reads each chunk list, checks its structure, and checks that the file it builds hashes to its ID.

## Consequences

- No cache and no scan: a big file is found as fast as a small one.
- A whole copy and a chunked copy of one file can sit side by side (a copy saved before milestone 5, then the file saved again chunked); both are good, and reads take either.
- A damaged chunk list is found when its file is read, not when its pack is opened; a read then tries the file's other copies, as for any damaged object.
- A pack Bolt can't read now stops `tidy` until it is readable again or moved out of `.bolt/store/packs`.
