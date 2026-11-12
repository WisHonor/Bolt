# 127 — Red team, milestone 6 (session 2): what `verify` holds is bounded, and two costs are written down

- **Status:** Accepted
- **Date:** 2026-11-10
- **Agenda topic:** none (milestone 6, the red team's second session on presets and polish)
- **Decided by:** Claude, at the owner's request while the owner was away. The owner can reverse any part.
- **Refines:** [decision 126](126-verify-also-finds-a-save-that-cant-be-written-out.md) (the size check it added) and [decision 118](118-bolt-verify.md)
- **Refined by:** [decision 130](130-a-walk-measures-a-listing-it-meets-again-before-walking-it-again.md) (the deep tree is refused at once: a walk measures a listing it meets again before it walks it again)

## Context

The red team's second session attacked the code with the first session's fixes in ([report](../redteam/2026-11-10-milestone-6-session-2.md)). It could not fool the three new checks of decision 126, nor make them report a problem on an honest project, and 558 runs of `verify` beside saves, switches and a `tidy` raised no false alarm. It found one problem, and measured two costs.

- **RT11-1 (the finding).** The size check of decision 126 keeps, for every file whose listed size is wrong, the file's name and both sizes, in each of its looks, with no limit, though `verify` tells at most 1,000 problems. A small planted store with many wrong sizes makes `verify` hold about 2.4 KiB for each: eight listings of 50,000 wrong entries took 1,043 MiB, where the same store with right sizes took 91 MiB and `tidy` 91 MiB. Under a cap of a million kilobytes, `verify` died.
- **A deep, narrow tree of shared listings, measured in a release build.** A chain of 24 listings of two folders each (27 objects, 8 KiB) holds 2^25 files by path. `bolt verify` tells it in a tenth of a second (`save_too_big`, decision 126). `bolt changes --save` refuses it after 201 seconds, and `bolt goto` after 517 seconds and 551 MiB: both walk it path by path until the 10,000,000-entry limit trips. Nothing is written.
- **Very wide honest listings.** With five listings of 200,000 files with 765-byte names (160 MB each before compression), a debug `verify` was still on its first look after 25 minutes at 1.7 GB.
- **By reading only:** who owns the project folder is asked by its path, and the folder is then opened by its path again; and a `git-words` that was a link is replaced by a plain file.

## Options

For RT11-1:
- **A. Keep everything, as built.** Then what a planted store repeats decides how much memory `verify` needs.
- **B. Keep what will be told, count the rest.** Details for at most 1,000 problems; a count beyond that; each look's working data dropped once the look is over.

For the deep tree:
- **A. Have every reader measure a save by its listings before walking it,** as `verify` now does: such a save is then refused at once. It adds a pass over the save's listings to every `goto` and `changes --save`, in the walk the speed session tuned against its gate.
- **B. Leave the readers as they are for v0.1, and write the cost down.**

## Decision

1. **RT11-1: B.** `verify` keeps the details of a problem (names, paths, sizes) for at most the 1,000 problems it tells, and counts the rest. Two looks agree on the problems kept and on the counts; where the counts differ, the smaller is told. While a look works, what it holds is bounded by the number of objects the store holds and of the links from one folder listing to another, never by how many entries or paths name them, and it is let go when the look ends. So `verify`'s memory still grows with the number of objects (the known limit), and no longer with what a planted listing repeats.
2. **The deep tree: B, for v0.1.** `goto` and `changes --save` still take minutes to refuse such a save, and `goto` half a gigabyte. It is written down as a known cost with the measured numbers, and it goes to the owner on the please-confirm list, because the fix (measure by listing first) is small to state and sits in the path the third speed gate measures. Until then `bolt verify` is the way to check a project one was given before going to its saves: it tells such a save at once.
3. **Very wide listings.** The fix for RT11-1 also looks for anything in `verify` whose time grows faster than the number of entries, and removes it. What is left is the cost every reader pays for a listing it must hold whole; it goes to the follow-ups with the numbers.
4. **The project folder judged by path, then opened by path.** A known limit, the same as Git's `safe.directory`: whoever can rename entries in a folder above the project controls the project already. Opening the folder once and judging owner and identity from that handle would close it; it goes to the follow-ups.
5. **`git-words` replacing a link** is as [storage-format.md](../storage-format.md) §10 says: the user's own file, read through a link and written as a plain file.

## Consequences

- The proof test joins the tests (Linux only: it caps the process's memory). `--json` is unchanged: `problems_total` is still the true count, and `problems` still holds at most 1,000.
- [security.md](../security.md)'s row for `verify` on a hostile store and the follow-ups list gain the measured costs.
- Both of this milestone's red-team sessions found problems only in `bolt verify`, the newest reader. Its checks are now the strictest of any reader's; the other readers' cost on a hostile deep tree is the open item.
