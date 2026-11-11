# 110 — The last save's folder listings are read ahead of the walk, through a second handle on the store

- **Status:** Accepted
- **Date:** 2026-11-10
- **Agenda topic:** none (milestone 6, speed and space)
- **Decided by:** Claude, at the owner's request while the owner was away. The owner can reverse any part.
- **Refines:** [decision 011](011-status-and-write-out.md) (how `status`, `save` and `goto` compare the folder with the last save) and [decision 061](061-pack-copy-order-and-lazy-opening.md) (which copy is read first, and at most 16 packs open)

## Context

For every folder it goes into, the walk reads the last save's listing of that folder from the store, to know what the save holds there. Each read is a lookup in a pack's index, a read of the record, a decompression, a hash and a decode: about 20 microseconds. After the rest of milestone 6's work those reads were the largest thing left on the walking thread that Bolt's own code decides: 4.5 of the 16.4 ms the walk of Git's source takes on GitHub's Linux runner ([benchmarks.md](../benchmarks.md) §8), and the walking thread is what a `status` waits for.

Which listings the walk will ask for is known before it starts: the save names its top listing, and each listing names the listings of the folders in it, in the order the walk meets them. So another thread can read them while the walk looks at the folder.

Two things went wrong on the way, and both shaped the decision.

- **The first build had the helper read through the walk's own store.** The store remembers which pack it read last and which packs it has open, and it searches them in that order ([decision 061](061-pack-copy-order-and-lazy-opening.md)). A helper reading through the same store changes that order at moments that depend on which thread runs first, and a listing handed over ready-made skipped what a read notes (the last pack, the copy found good). On a healthy store nothing showed. On a store where a listing has two copies and one is damaged, which copy the walk met first, and so whether the command warned about the damaged pack, depended on timing. A review found it before it was merged.
- **A memory test failed on Linux.** `save_does_not_hold_every_long_path_of_the_save_before_it` runs `bolt save` with its address space capped. One more thread was enough to fail it, and measuring why showed the test itself stood on an edge: see the last part of the decision.

## Options

- **A. Leave the reads on the walking thread.**
- **B. A helper reads ahead through the walk's own store.** The least code. Warnings on a damaged store can then come and go from run to run.
- **C. A helper reads ahead through a second handle on the store**, and the walk's store still runs its own read for each listing, spared only what the helper has already done.
- **D. Keep the last save's listings in a cache file,** as Git's index keeps its tree cache. One more file to seal, to keep in step with `head`, and to attack; and reading it is itself a read the walk would wait for.

## Decision

**C.**

- **The helper.** `status`, `save` and the plan of a `goto` start one more thread beside the walk. It reads the last save's listings depth first, each folder's subfolders in the listing's own order, which is the order the walk goes into them, and puts each into a buffer. It knows nothing of the folder on disk and decides nothing.
- **It reads through a second handle on the store:** its own list of the packs (copied from the store's, not read from the folder again), its own open packs, its own notes of good and bad copies. The two handles share the folder they open packs from and nothing else. Whatever the helper's reads do, they do to the second handle, which is gone when the walk ends.
- **The walk never waits for it.** When the walk comes to a folder it looks in the buffer for that listing. If it is there, the walk takes it, and whatever lay before it in the buffer is dropped (folders the walk passed by: deleted, turned into files, left out by a rule). If it isn't there yet, the walk reads the listing itself, at once, as before.
- **A listing taken from the buffer still goes through the store's own read.** The walk's store finds its first copy in its own order, passes over a copy it knows is bad, notes damage it meets, and remembers the copy as good and its pack as the last one read: the same code as with no helper. It is spared two things, and only these:
  - the lookup in the index, when the pack it asks first is the pack the helper read the listing from;
  - the read of the record, when the copy it comes to read is the very copy the helper read (the same pack, the same index line).
  So everything the store remembers after a walk, and every warning that follows from it, is what it would have been with no helper.
- **What the two shortcuts rest on:** a pack is named by its index's checksum and never changes once it is named (the pack reader's block window already rests on this), so the pack of one name holds the same lines and the same bytes through either handle. The listing itself is always one whose bytes were read whole and checked against the ID asked for, by the same reading code. The rule that every read is checked against its fingerprint stands: the walk's store doesn't read the record a second time, so there is no second read to check.
- **Bounds.** The buffer holds at most 256 listings and at most 1 MiB of them as stored (a listing takes a few times that once decoded). The helper stops for good at the first listing it can't read cleanly (missing, damaged, not a listing, in a pack it can't open), at one stored in more than 1 MiB, past 2,048 levels, or with more than 65,536 folders still to read. Stopping tells nothing and costs nothing but speed: the walk reads the rest itself, with every warning it would have given.
- **Open files.** [Decision 061](061-pack-copy-order-and-lazy-opening.md)'s limit of 16 open packs holds for each handle. While a walk runs there can be 16 more, the second handle's.
- **The memory tests' cap.** The three tests in `crates/bolt-vcs/tests/cli.rs` that run `bolt` with its address space capped used 100,000 KB. Measured on GitHub's Linux runner with the cap lowered step by step, `main` passes the save test at 100 MB, fails it in 4 to 7 runs of 15 at 90 MB, and passes again at 80, 64, 50 and 40 MB. The failures are not about what Bolt holds: just over 64 MiB, the C library's allocator sometimes manages to reserve a whole 64 MiB arena for a thread, and then everything else starves. With one more thread the edge moved up to 100 MB. The cap is now 64,000 KB, under one arena, where none can ever be reserved: the cap then measures what Bolt holds (under 40 MB in all three tests), and it is tighter than it was.

## Consequences

- `status` on Git's source, on one GitHub runner, the builds timed turn by turn: 27.1 ms before, 23.5 ms after on Linux (the first build, through the walk's own store, measured 23.2 ms); 117.9 ms before, 114.4 ms after on Windows. On a single processor it is no slower than before (34.8 against 35.7 ms).
- One more thread for each scan, and up to 16 more open packs while it runs.
- **Known limits, all of them speed only.** A saved folder with more than 256 folders under it that the walk no longer goes into fills the buffer with listings nobody takes, and reading ahead ends for that scan. A store with a damaged or unreadable pack that sorts before the pack a listing is in ends it at once. A project with many small packs makes the helper open them as the walk does (`bolt tidy` merges them).
- A pack replaced under its name while a command runs may be read through one handle as the old file and through the other as the new: the listing handed over is still checked against its ID, but its copy may be noted good through a handle that would have read the old file. Bolt never rewrites a named pack; `tidy` replaces one only with the same objects.
- [design.md](../design.md) §5.9 says so, and [security.md](../security.md) T32. Tests: for five cases (one copy in the pack asked first; one copy elsewhere; two copies with the walk's first one damaged; the helper's copy known bad; the first copy not a listing), a store that takes a listing read ahead remembers exactly what a store that read it itself remembers; reading through a second handle leaves the first store as it was; the walk's scan, warnings and store are the same with the helper on and off on random trees, on trees whose folders were deleted, turned into files or left out, and on stores with a damaged second copy; the buffer never passes its bounds; a walk that ends early wakes a helper waiting for room, and no thread outlives the walk.
