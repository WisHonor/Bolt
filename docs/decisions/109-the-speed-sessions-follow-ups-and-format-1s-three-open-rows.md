# 109 — The speed session's follow-ups: done, measured, or moved past v0.1; and format 1's three open rows

- **Status:** Accepted
- **Date:** 2026-11-10
- **Agenda topic:** none (milestone 6, speed and space)
- **Decided by:** Claude, at the owner's request while the owner was away. The owner can reverse any part.
- **Refines:** [decision 123](123-follow-ups-done-or-moved-past-v0.1.md) (the rows it left to the speed session); [decision 121](121-format-1-freezes-at-v0.1.md) (the three format questions it left open); [decision 105](105-the-change-id-cache-keeps-each-lines-saves-in-order.md) (how a save is found by its whole ID, and what a wrong list may do); [decision 044](044-red-team-m2-final-rulings.md) and [decision 087](087-what-an-import-writes-once-files-are-chunked.md) (what an import costs)

## Context

[The follow-ups list](../plans/2026-11-01-milestone-1-follow-ups.md) holds what five milestones put off. Decision 123 settled most of its "before v0.1" rows and left the rest to the speed session: every row about speed, memory or space. Decision 121 froze format 1 at v0.1 and named three of those rows as questions about the format itself, to be answered before the freeze. The owner's rule for each row: do it, or move it past v0.1 in a decision record that says why.

Each row's premise was checked in the code, or measured, before it was ruled on. The numbers are in [benchmarks.md](../benchmarks.md) §8.

## Decision

### Done in milestone 6

1. **Where Git wins: `status`, a small save and going back, on a small project.** The third speed gate ([decision 099](099-a-third-speed-gate-within-twice-gits-time.md), measured by medians, [decision 108](108-the-third-gate-compares-medians.md)) and the work behind it. What still loses is under "Still losing" below.
2. **Where Git LFS wins: the first save of big files that don't compress, and going back to an old version of a big file.** A pack or a file is flushed while it is written ([decision 102](102-big-files-are-flushed-while-they-are-written.md)), and a switch reads each chunk whole.
3. **`undo --list`, its `--all` form, and the amend check, on a long history** (three "measure" rows of milestone 4). They read the whole line for every command. `cache/change-ids` now keeps each line's saves in order (decision 105), and they answer from it.
4. **A hostile list of tiny chunks.** A switch keeps the chunks it has checked while it writes one file, up to 8 MiB of them, so a list that repeats a chunk reads it once. The list of 8,388,608 one-byte chunks that took 82 s to write out takes 22 s.
5. **The import's cache of rebuilt objects** is 96 MiB, Git's own default, where it was 256 MiB ([decision 104](104-an-import-keeps-96-mib-of-rebuilt-objects.md)).
6. **Folders deeper than the walk holds open.** Past the levels the walk holds open (at most 128 folders at once), every 48th level now stays open too, so a deep folder is opened again from the nearest one kept, at most twice, where the opens grew with the square of the depth. The deepest chain the limits allow (2,048 levels with a file in each) saves in 2.4 s where it took 108 s, and a `status` that must read every file takes 1.3 s where it took 106 s (the Windows laptop, a release build). What a save reads, the order it reads in and the packs it writes are unchanged; the row's own idea, reading files top down, would have changed the order objects enter packs.
7. **Write-out's cost on the deepest tree** (a "not measured yet" row since milestone 3) is measured: on that same chain a `goto` that removes the 2,048 files takes 335 s, and one that writes them back 529 s (the Windows laptop, a release build). Write-out opens every folder again from the top for each file and keeps no checked folder open, which is the safe design [decision 048](048-handle-relative-file-access.md) chose against a folder swapped under a switch. The cost grows with the square of the depth, so a tree a tenth as deep costs a hundredth. It stays as it is for v0.1, a known limit: after v0.1.

### Format 1's three open rows: none changes the format

Decision 121 asked for each to be decided before the freeze. Format 1 freezes without any of them.

1. **Storing each Git tag object once.** No. Each imported tag keeps its tag objects' bytes, as today. The waste needs many tags that all go through one big tag object, which no repository this session met has (Linux has 951 tags and Git 1,022, a kilobyte or so each), and the tag list's own cap of 512 MiB bounds it. A second kind of record to save space in a case built on purpose isn't worth a format rule every later reader must carry.
2. **A bound on a chunk list's chunk count by its file's size.** No. The cost the bound would have capped is gone where it hurt: a repeated chunk is read once. A list of distinct tiny chunks still costs a read for each, but the one who plants it pays over a hundred bytes of pack for every chunk, and the reader's time grows only in step with that.
3. **A tighter bound on a stored copy's size** (refusing, unread, a copy stored bigger than zstd could ever make it). No. A bound that is wrong by a byte makes good data unreadable, which is the worse failure; and the cost it would save is a read of bytes the planter had to put on disk, one for one.

### Moved past v0.1

1. **The memory a hostile save inside the limits can cost a reader** (about 1.5 to 4 GB, for a save with very wide folders, and for comparing two of them). Before networking (v0.3), with the memory review: that is when a save first reaches Bolt from someone else's computer without the user having taken the folder by hand. What costs the memory is holding one whole listing, or two side by side, when a folder has millions of entries; the limit that allows it (10 million entries a save) is the format's, and reading a listing a piece at a time is a change to every reader of listings. This session added nothing to it: the listings read ahead of the walk are held up to 1 MiB as stored ([decision 110](110-the-last-saves-listings-are-read-ahead-of-the-walk.md)), and the memory tests run under a tighter cap than before.
2. **Many planted packs.** A cap on the packs or index entries of a store would refuse stores that are honest today (every save adds a pack until `tidy` merges them), so it needs `tidy` to be able to keep under it always. Packs are searched on disk and at most 16 are open for each handle on the store; the planter pays the disk. Before networking (v0.3), with the memory review.
3. **The import's memo tables.** Measured on the largest history at hand: Linux, 1,506,211 commits, imports in 49 minutes with a peak of 3.4 GB. That is a lot, but it works on an ordinary computer, and an import happens once. A table kept on disk is the fix when a real repository needs one; after v0.1. [design.md](../design.md) §5.16 and benchmarks.md §8 give the numbers.
4. **The header memo's cap,** unmeasured on a repository the size of Linux's: that import ran inside its work allowance. Closed as measured. The other shape of the memo (a number for each entry of a pack) stays an idea for after v0.1.
5. **Two reads the Git reader repeats.** Left. They are two small reads for each object, from memory the system already holds: at most a few seconds of Linux's 49 minutes.
6. **`kept.rs`'s check that grows with changes times matched paths.** Left. Measured on a switch of 3,548 files (Git's history, 2,000 saves back): the whole plan takes 38 ms of a switch of about 2 s.
7. **The set of copies the store has checked** (about 47 to 94 bytes a copy). Left. A command checks the copies it reads: about 11 MB for a 10 GiB file. The 10-million-object case is a full check of a huge store, which v0.1's commands don't make through this set.
8. **A big file whose chunks are spread over many packs.** Left as measured by the red team of milestone 5: `tidy` merges packs, which is what brings such a file back together. An index over the whole store is a later storage feature.

### Still losing, and where each went

These are new rows in the follow-ups list, each with its numbers.

1. **`goto` on Linux.** A `goto` that writes one file takes 1.8 to 2.1 times Git's time on GitHub's Linux runners, by the runner's disk: under the gate's limit on some machines and over it on others (25.5 ms against Git's 12.9 on one runner, turn by turn; it was 4.5 times). Two things would close it, and neither is taken here: walking sibling folders on several threads (it re-shapes the module that decides what is saved, ignored, nested or refused; it wants its own design and red-team round), and flushing less in a `goto` (ten flushes today; that changes what a crash can leave, which is the owner's call).
2. **A switch that rewrites thousands of files.** 3,548 files of Git's history: 1.9 to 3.1 s against Git's 0.4 to 0.8 s on Linux, and 2.0 to 2.7 s against 0.9 to 1.2 s on Windows (they were 2.9 to 3.8 s and 6.4 to 7.5 s). Bolt flushes each file before it renames it into place; Git flushes none. Flushing them in batches on helper threads would roughly halve it.
3. **Space, against Git's deltas.** Linux's history takes 64.6 GB in Bolt and 6.6 GB in Git. Records stored as deltas are decision 009's later feature.
4. **The look at each file on Windows** costs five system calls in the platform crate (25 microseconds a file on GitHub's runner). Fewer calls means new `unsafe` code there, for the owner to review.
5. **A flush for the drive's clock** at the start of every scan: under a millisecond on a quiet disk, 5 to 15 ms on a disk that flushes slowly. The 2-second rule rests on it.
6. **A compression rule for file types that are already compressed** (Godot's `.hdr` images are where zlib's level 6 beats zstd's level 3, [decision 103](103-where-git-keeps-less-zstd-level-3-and-the-chunk-sizes-stay.md)). It would be a `bolt.toml` setting, which is the other session's to add.
7. **A line whose name is made only of the letters k to z** (`work`, say) is also tried as the start of a change ID by `goto`, and "no save has it" is confirmed by reading the line, as before this milestone.
8. **The first save of big files that don't compress, wherever the disk is slower than the hashing.** Bolt flushes a pack to the disk before it names it; Git LFS's time doesn't follow the disk, so it isn't waiting for it. On a Linux runner whose disk takes a gigabyte in about two seconds, Bolt's 2.45 s is beside LFS's 2.26 s (main took 4.15 s). On one whose disk takes five, Bolt takes 4.9 s (main 6.5 s) against 1.85 s; and on the Windows runner flushing while writing changes nothing, 5.9 s against about 3 s. Not flushing would close it, and the first principle forbids that. Writing a big pack past the system's cache, so the disk works while Bolt hashes, is an idea for after v0.1.
9. **Reading ahead gives up easily** (decision 110): at the first listing it can't read, and when a saved folder with over 256 folders under it is no longer walked into. From then on that scan is as slow as before.

### Decision 105's wording, corrected

Decision 105 says a save named by all 64 digits of its ID is looked for "in the list of the project's own position first". As built, a whole ID counts as found only in a list that starts at a line's current newest save; otherwise the lines are walked as before. Looking in the position's own list would find saves that no line reaches (after `bolt goto line@server`), where the walk says "no such save"; the rule that binds is that every answer is the one given without the cache.

And decision 105 rested a list's truth on the seal alone. As built, a list is never the reason a save is handed out unchecked:

- **The history listing** follows each save's own first parent from the line's newest save, one save at a time, as it did before the cache held lists; the list gives only the short forms. Each save is compared with the list's entry at its place as it goes by, and the first difference drops the list.
- **`goto <the start of a change ID>`** takes the one save the list names only after checking the list link by link, from the line's newest save down to that save (each save's change ID, and its first parent being the next one listed). No match, or several, is confirmed from the store.
- **A list that doesn't match** is never an error: it is dropped, rebuilt from the store and written, and the answer is the store's.

What stays trusted is that a list doesn't hide a second save of the line whose change ID starts with the same letters as the one asked for: showing that would mean reading the whole line on every lookup, which is what the list is there to avoid. The short forms Bolt prints rest on the same trust, as they did with the version 1 file.

## Consequences

- The follow-ups list is updated row by row: the done rows are struck with their dates, the moved rows say "After v0.1" or "Before networking (v0.3)" with a pointer here, and the "Still losing" rows are added.
- Format 1 freezes at v0.1 with no change from this session.
- Decision 105 gets a back-link line.
