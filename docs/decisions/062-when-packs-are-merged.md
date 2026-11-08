# 062 — When packs are merged, and which count as small

- **Status:** Accepted
- **Date:** 2026-11-05
- **Agenda topic:** none (milestone 3, review of the merge's code)
- **Decided by:** Claude, at the owner's request while the owner was away. The owner can reverse any part.
- **Refines:** [decision 057](057-pack-index-on-disk-and-merging.md) (when a merge runs, and which packs it takes)
- **Refined by:** [decision 077](077-tidy-and-the-undo-window.md) (`tidy` rewrites the packs merging leaves, and version 1 ones); [decision 083](083-a-chunk-list-is-stored-under-its-files-id.md) (a pack counts as small by its chunk lists' stored sizes)

## Context

Decision 057 merges once the store holds more than 32 packs, taking every pack under 64 MiB. Two problems follow:
- **Big packs make every commit merge.** With 31 or more big packs, each commit pushes the count past 32, so the small packs, which by then are one merged pack, get rewritten again. A project with big assets reaches that often. For code-sized commits that is about 20 GiB written per cycle.
- **Size on disk says little about the work.** A pack under 64 MiB can hold a zstd entry that claims up to 1 TiB, which the merge would decompress to check, on every merge. Reads already defend against such copies (decisions 029 and 061); the merge mustn't reopen the hole.

## Options

- **A. Count only small packs**, and call a pack small by what its index records as well as its size on disk. Git does much the same: `gc.autoPackLimit` doesn't count packs it keeps.
- **B. Count every pack, but skip a merge that would only rewrite one merged pack.** This doesn't deal with the big claims.
- **C. Merge only when asked** (`tidy`, milestone 4). Then risk 17 comes back for anyone who never runs it.

## Decision

**A.**
- A pack is **small** when it is under 64 MiB on disk **and** the uncompressed sizes its index records total at most 256 MiB. Only the index is read to decide; nothing is decompressed.
- A merge runs after a command has committed, when **more than 32 packs are small**. It takes only the small ones.
- Every other pack is left as it is, for a later cleanup or repack (`tidy`, milestone 4).
- A small pack that can't be read (an I/O error, too little memory, no permission) is left as it is and named in `pack_not_merged`, as a damaged one is, and the merge carries on with the rest. Only a failure to write the new packs stops the merge.

## Consequences

- With many big packs, a commit no longer rewrites the merged pack: the next merge waits until more than 32 small packs have built up again.
- A merge decompresses at most 256 MiB per pack, at most about 8.25 GiB per merge (33 packs). A damaged pack that is kept is read again at each merge, and the same bound limits that cost.
- The store can hold more than 32 packs in all. Reads still keep at most 16 open (decision 061).
- [storage-format.md](../storage-format.md) §11 and [design.md](../design.md) §5.7 describe the rule.
