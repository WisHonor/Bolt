# 077 — `tidy` keeps what lines, shelves and the undo window reach, and nothing else

- **Status:** Accepted
- **Date:** 2026-11-06
- **Agenda topic:** none (milestone 4 planning)
- **Decided by:** Claude, at the owner's request while the owner was away. The owner can reverse any part.
- **Refines:** [decision 009](009-storage-engine.md) (cleanup), [decision 010](010-operation-log.md) (the undo window), [decision 057](057-pack-index-on-disk-and-merging.md) and [decision 062](062-when-packs-are-merged.md) (packs left for `tidy`)
- **Refined by:** [decision 079](079-red-team-m4-rulings.md) (diary ends in the window are kept); [decision 081](081-what-building-milestone-4-refined.md) (edits made while milestone 4 was built); [decision 083](083-a-chunk-list-is-stored-under-its-files-id.md) (`tidy` follows chunk lists, and stops at a pack it can't read); [decision 088](088-benchmarks-and-the-speed-gates.md) (compressing again waits for its numbers)

## Context

Decision 009: cleanup keeps everything reachable from lines, shelves and the undo window; never deletes anything younger than a grace period; writes new packs before deleting old ones; cheap cleanup runs on its own, repacking only when the user asks, with an estimate of the space it would free. Decision 010: the undo window is 30 days, a personal setting, with an optional size budget for safety copies. Decisions 057 and 062 merge only small packs after a command and leave the rest, and compressing again at a stronger level, to `tidy`. Milestone 4 builds `bolt tidy`. Left open: exactly what is kept, how long the grace period is, which packs are rewritten, what stops it, and which parts of decision 009 wait.

## Options

- **A. Mark what is kept, then rewrite only the packs that hold anything else** (Git's `git gc`, with `gc.pruneExpire`; Jujutsu's `jj util gc --expire`). A pack holding nothing to keep is deleted unread, one holding only what is kept stays, and the others are rewritten with what is kept.
- **B. Delete only packs that hold nothing to keep.** Cheap, but once small packs have been merged (decision 057) most garbage shares a pack with something kept, so it frees little.
- **C. Rewrite everything** (`git repack -a -d -f`). Frees the most and could compress everything again, but rewrites every byte of a big store each time.

## Decision

**A.**
- **`bolt tidy`** takes the lock, finishes a switch a crash stopped, works out what to keep and how much it will free (and says so), then removes the rest, and says how much space it freed.
- **What is kept:**
  - everything the current state reaches: each line's save, the save you are at, every shelf, every tag and server line, with all their history (every parent), folder listings, file contents and project links;
  - every diary entry from `head` back to the first: entries are small, and the diary is the record;
  - for each diary entry made within the undo window (by the time it records), everything an undo or redo of it, or of what it undid or redid, needs: the states of the entry and of the entry before it, both saves of its folder change ([decision 071](071-a-step-records-its-change-to-the-folder.md)), and, for an undo or redo entry, the states of its target and of the entry before the target and both saves of the target's folder change; each with everything it reaches and all its history. So a step made, undone or redone within the window can always be undone and redone again, however old the state it goes back to;
  - every pack changed in the last 24 hours, whole and untouched: the grace period.
- **What is removed:** everything else: saves that nothing above reaches (undone, cleared or amended saves, and the kept saves of old discards and shelves), whatever only they hold, and diary entries that aren't on the diary from `head` back, which a crash can leave.
- **How:** each pack's index is checked against its checksum before `tidy` judges it, since it decides what to delete from the index alone; one that fails is left as it is, with a warning. A pack holding nothing to keep is deleted unread, first, so `tidy` frees space even on a full disk. A pack holding only what is kept stays as it is, unless it is small or still version 1: those are rewritten as version 2, so the merge after a command stops reading old indexes. The others are rewritten with what they hold that is kept: each object is checked as merging checks it, and its stored bytes are copied unchanged. Every new pack is on disk before any pack it rewrites is removed, so a stop at any point loses nothing. A store holding a chunk list or a conflict record (kinds this version doesn't follow) stops `tidy` before it removes anything.
- **Damage stops it.** If anything that must be kept can't be read (a missing or damaged save, listing, entry or other record), `tidy` removes nothing, says what it found, and exits 1. A pack it can't read is left as it is, with a warning, as merging leaves one.
- **The undo window** is the personal setting `undo-window-days`, at the top of the user's own settings: whole days, 1 to 36,500, and 30 when it isn't set. Undo can still reach an older entry until `tidy` removes what it needs.
- **Waiting:**
  - cleanup on its own (decision 009's cheap cleanup): working out what to keep reads every record still needed, too slow to run behind a save. Jujutsu doesn't collect on its own either. Until it comes, only `tidy` frees space;
  - the size budget for safety copies (decision 010);
  - compressing again at a stronger level (decisions 009 and 057) waits for milestone 5, where chunked files change what is stored and its benchmarks can measure the levels first.

## Consequences

- `tidy`'s memory grows with what it keeps: a set of fingerprints, about 100 bytes an object, so about 1 GB at 10 million objects. It is measured with the memory review before v0.1.
- Amended, undone and cleared saves stay in the store for at least the undo window; `bolt show` reaches them by fingerprint until `tidy` removes them.
- A project that never runs `tidy` keeps everything, as before.
- Entries kept in a young pack can point at what `tidy` removed from older ones; they are garbage too, and the next `tidy` a day later removes them.
