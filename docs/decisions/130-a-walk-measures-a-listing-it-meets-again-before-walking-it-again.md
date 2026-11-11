# 130 — A walk measures a listing it meets again before it walks it again

- **Status:** Accepted
- **Date:** 2026-11-11
- **Agenda topic:** none (getting v0.1 ready: clean code and the gate)
- **Decided by:** Claude, at the owner's request while the owner was away. The owner can reverse any part.
- **Refines:** [decision 127](127-what-verify-holds-is-bounded-and-two-costs-are-written-down.md) (its second ruling: the deep tree's cost in `goto` and `changes --save`, left for v0.1); [decision 126](126-verify-also-finds-a-save-that-cant-be-written-out.md) (a save past the readers' limits)

## Context

A folder's listing is stored once and named by its fingerprint, so two folders with the same contents share one listing. A planted save can use that: a chain of 24 listings, each naming the next one twice, is 27 objects and 8 KiB, and holds 2^25 files by path. The format allows at most 10,000,000 entries in a save, counting every path ([storage-format.md](../storage-format.md) §3), and every reader counts as it walks. `bolt verify` measures each listing once, however many paths reach it, and tells such a save in a tenth of a second. The other readers walk it path by path until the count passes the limit: `bolt changes --save` refused it after 201 seconds, and `bolt goto` after 517 seconds and 551 MiB (decision 127, measured in a release build). Nothing was written, but a project someone hands over can stall these commands for minutes. Decision 127 left it for v0.1, written down; the owner asked for it fixed, every command that reads a save refusing such a save at once.

The walks over a save by path are the comparison of two trees (`diff.rs`, behind `changes`, `show`, `goto`, `discard`, the shelves, undo and redo), the scan's walk of the folder against the last save (behind `status` and `save`), and the walk of a partial save. Each counts entries with the save's limits as it reads listings. Write-out looks paths up one at a time, from a plan those walks made, and `tidy` marks each object once.

## Options

- **A. Measure the whole save before every walk,** as `verify` does: each listing of the save read once before the walk starts. Every command then reads every listing of the saves it touches, `status` and a small save among them, where today they read only the listings of the folders they look at. That is hundreds of extra reads on an everyday project, in the two steps the third speed gate holds to twice Git's time.
- **B. Measure a listing when the walk meets it a second time.** A walk keeps the set of listings it has entered. Before it enters one again, reached by another path, it measures that listing as `verify` does (each listing beneath read once, however many paths reach it) and counts all of it at once, then walks it without counting it again. Only shared listings make a save bigger by path than what is stored, so this catches every such save at the first repeat that carries it past a limit, and costs a set lookup per folder where nothing repeats.
- **C. Leave it** (decision 127's ruling).

## Decision

**B.**

- **Every walk over a save keeps, for each save it walks, the listings it has entered.** On entering one a second time, it measures that listing first: the entries below it counted by path, the deepest folder and the longest path below it, each listing read once whatever reaches it, as `verify` measures. The whole of it is counted against the save's limits at once, and the walk then goes through it without counting it twice.
- **The count is still the count by path.** A save within the limits the format sets is never refused, and the saves refused are those past them, with the same words as before ("it has more than 10000000 files and folders in total"). The refusal comes before the repeated part is walked: for the planted chain, after one path down and a read of each listing.
- **A save whose listings are each reached by one path costs one set lookup for each folder a walk enters.** Two folders with the same contents (two empty folders holding the same file, say) cost one measure of that listing.
- **The measure keeps its results for the rest of the walk,** so a listing met a third time is not measured again.
- What it costs an honest `goto` and `changes --save` is measured before and after on one runner and published in [benchmarks.md](../benchmarks.md).

## Consequences

- `bolt goto`, `changes --save`, `show`, `discard`, `shelf`, `unshelf`, `undo` and `redo` refuse a planted deep tree at once, and so do `status` and `save` when the last save holds one in a folder the disk no longer has. A command that never walks the repeated part of a save doesn't walk it now either, so it is not refused for it; `bolt verify` names every save past the limits.
- The red team's test of decision 127's chain becomes a test that each command refuses it within seconds; [security.md](../security.md)'s row and the follow-ups row on the deep tree are brought up to date.
- Decision 127 gets a back-link line.
