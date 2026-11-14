# 160 — Whether a save is in another's history: one walk, through every parent

- **Status:** Accepted
- **Date:** 2026-11-14
- **Agenda topic:** none (milestone 7 planning)
- **Decided by:** Claude, at the owner's request while the owner was away. The owner can reverse any part.
- **Refines:** [decision 012](012-lines.md) (combining starts from the common starting point); [decision 158](158-making-renaming-and-deleting-lines.md) (`line delete` asks when nothing else holds the line's saves)

## Context

**In plain words.** History is a family tree of saves. Several commands need to ask family questions: "is this save an ancestor of that one?", "which saves does this line have that no other line has?", and, for combining, "where did these two lines part?".

Every walk over saves in v0.1 follows first parents only: `history`, finding a save by its ID, and the check before an amend. Only `tidy` and `verify` follow every parent, and only to collect what to keep. A save with two parents comes only from a Git import in v0.1, so that was enough. In v0.2 `combine` makes such saves every day.

Nothing stored says how deep in history a save is, so a question about ancestry can only be answered by reading saves.

## Options

- **A. Read everything first.** Collect every save each side reaches, then compare the two sets. Always exact and very simple, but it reads the whole history every time.
- **B. Walk both sides at once, newest first, and stop where they meet.** This is how Git finds where two branches parted when it has no commit-graph file. It reads the saves the two sides don't share, and a little more.
- **C. Keep an index of how deep each save is** (Git's commit-graph file; Jujutsu and Mercurial keep such an index always). Fastest, but it is a new cache with its own format, sealing and rules for when it is stale.

## Decision

**B now. C only if measurements ask for it.**

- **One module owns every question about ancestry** (`ancestry`). It reads each save once per walk, never recurses, and remembers only a few marks per save.
- **The walk.** Each side marks the saves it reaches. The walk always looks next at the newest save it hasn't finished, by the time it was saved. A save marked from both sides is shared, and so is everything below it. The walk ends when only shared saves are left to look at.
- **Exact whatever the clocks say:** whether a save is in another's history, and (from milestone 8) where two lines parted. Times decide only how soon the walk notices shared history. Wrong times make it slower, never wrong.
- **Exact only when no save is older than its parent:** the count of saves one side has alone, which `line delete` shows in its question. In a history with such clocks the count can be too high, never too low, so it errs toward asking.
- **No limit on a walk's length.** It is bounded by the saves stored, as `tidy`'s walk is.

## Consequences

- The cost is measured on Git's own history before milestone 8 builds on it, and written into [benchmarks.md](../benchmarks.md). An index of depths is a decision of its own if the numbers ask for it.
- `history`, finding a save by its ID, and the amend check keep following first parents until milestones 8 and 10 change them.
- A planted history can make a walk long, never wrong: every save is checked against its fingerprint when read, so a history can't loop.
