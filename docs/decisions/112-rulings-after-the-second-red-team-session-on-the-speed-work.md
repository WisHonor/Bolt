# 112 — Rulings after the second red-team session on milestone 6's speed work

- **Status:** Accepted
- **Date:** 2026-11-10
- **Agenda topic:** none (milestone 6's red-team gate, [decision 024](024-red-team-gate.md))
- **Decided by:** Claude, at the owner's request while the owner was away. The owner can reverse any part.
- **Refines:** [decision 111](111-rulings-after-the-first-red-team-session-on-the-speed-work.md) (the block window reads a block only while blocks pay for themselves; the check before an amend keeps what it read of the other lines; the first session's memory proof runs on Linux only) and [decision 100](100-caches-are-written-without-a-flush-and-only-when-they-changed.md) (a third cache, written the same way)

## Context

The second red-team session ([report](../redteam/2026-11-10-milestone-6-speed-session-2.md)) attacked the branch with the first session's three fixes in, again from the no-network sandbox and beside `main`.

**What held.**

- **The three fixes.** Their proof tests pass. 150 random runs of lines, amends, `undo` and `redo`, with `goto` by every kind of ID and with `cache/change-ids` removed or put back stale between commands, gave `main`'s exit code, output and files at every step.
- **A `goto` stopped half way.** Stopped at each of its 17 fault points while writing a 40 MiB file, then finished by the next command, it was always reported and always recovered.
- **Deep and wide projects.** 3,000 saves over a 300-level tree under 256 open files ran as fast as `main` or faster. The widest save the limits allow took the same memory on both builds.

**What it proved:** two findings, both Low, neither a loss of data, and both a cost that a fix of the first session moved instead of removing. Each was checked in the code before it was ruled on.

- **RT13-1, Low.** The block window's rule ([decision 111](111-rulings-after-the-first-red-team-session-on-the-speed-work.md): a block is read at the second read in a row that falls in it) is beaten by reads that come in pairs. A planted project whose tiny chunks are stored two by two, each pair in another block than the pair before, makes every pair read 64 KiB: about 36 KB read for each 8-byte chunk where `main` reads about 400 bytes. The time is no worse than `main`'s while the pack is in the system's cache; on a drive where it isn't, the bytes are the time.
- **RT13-2, Low.** Decision 111 stopped the check before an amend from making a list for each other line, and walks each instead. Nothing the walk reads is kept. A line that split off long ago and has a long run of saves of its own is read from the store on every amend: with 300 such lines of 2,000 saves each, every amend took 19 to 21 s, where `main` took 34 s once and 0.3 s after. No planted project is needed: an imported history with old release branches pays it. Decision 111 had written this cost down as a consequence; the session showed how much it is.

## Options

For RT13-2:

- **A. Leave it, as decision 111's consequence.** Nothing is wrong, only slow. But the people it is slow for are the ones Bolt most wants: those who import a real Git history, with its maintenance branches, and amend their newest save.
- **B. Make a list for each other line again, within a budget.** A list holds a line's whole history, so lines that share a long history each cost a copy of it: the first session's finding. A budget keeps memory bounded but leaves most lines of a big project walked on every amend.
- **C. Keep only what the walk read** (chosen): for each other line, the saves from its newest down to where the walk stopped, and where that is. Each save a walk reads is kept once, so the cost is the lines' own saves, not lines times history. It needs a place to keep them: a new cache file, so that `cache/change-ids`, which nearly every command loads, doesn't grow with it.

## Decision

Both are fixed. Neither is ruled away.

1. **The block window reads a block only while blocks pay for themselves (RT13-1).**
   - For a pack's entries and for its index, the reader counts the bytes of the small reads asked of the window, and the bytes it has read as blocks. A block is read only if, with it, the blocks read come to at most four times the bytes asked for.
   - Decision 111's rule stays as well: a block is read only at the second read in a row that falls in it.
   - So whatever the order of the reads, a pack reader reads at most five times what it was asked for: what a read asks for itself, and at most four times that in blocks. A reader's first reads are plain small reads, as they were on `main`, until 16 KiB has been asked of it.
2. **The check before an amend keeps what it read of the other lines (RT13-2).**
   - A new cache, `cache/other-lines` ([storage-format.md](../storage-format.md) §10), holds runs. A run is the saves a walk read from a line's newest save back along first parents, each with its change ID, and the ID of the save the walk stopped at, if it stopped at one.
   - The check uses a run for a line whose newest save is the run's first. It looks for the change ID in the run; if it isn't there, it goes on from where the run stopped, which is usually a save of the amended line's own list, so nothing is read. Whatever it does read is added to the run.
   - **It is a cache like the other two:** sealed with the user's key, written through `tmp/` and a rename without a flush or the lock, skipped if the write fails, rebuilt if deleted ([decision 100](100-caches-are-written-without-a-flush-and-only-when-they-changed.md)). At most 64 MiB: when the runs would pass that, what was read last isn't kept, and those saves are read again by the next amend.
   - **Only the check before an amend reads it.** Other commands don't load it, so a project with long side lines pays for them only when it amends.
   - **What it is trusted for:** whether another line shows the change being amended, which is what `main` trusted its lists for. A run that is wrong under a good seal can make the check miss a line (the amend goes ahead and that line keeps the old save; `undo` takes the amend back) or name a line that doesn't show it (the amend is refused, and saving as a new save still works). It can't change a stored save or what a command shows or goes to.
3. **RT12-1's proof test runs on Linux only.** It rests on an address-space cap, which macOS doesn't enforce, so there it proved nothing and cost a 10,000-branch import on every push.
4. **What neither session reached** stays in the [follow-ups](../plans/2026-11-01-milestone-1-follow-ups.md): a planted store of ten thousand small packs, the races in the walk that `main` had too, and what the sandbox can't run (Windows, macOS, a drive that folds letter case, a power cut, a drive whose pages aren't cached).

## Consequences

- The two proof tests run on every push on Linux (`crates/bolt-vcs/tests/redteam13.rs`); both count the bytes a process read, which only Linux tells.
- After the first amend in a project, a later amend reads no save of a line that hasn't moved. The first still reads every other line's own saves once, as `main` did.
- `cache/other-lines` takes 48 bytes for each save of the other lines' own runs: 29 MB for the session's 300 lines of 2,000 saves.
- A pack reader's first reads, until 16 KiB has been asked of it, cost a read each where a block could answer them before. The third gate's steps were measured again with the fixes in ([benchmarks.md](../benchmarks.md) §8).
- These two fixes were not attacked by a third session. They were reviewed, and the proof tests and the whole suite run on all three systems.
- [storage-format.md](../storage-format.md) §10 and §13, [design.md](../design.md) §5.4 and §5.7, [security.md](../security.md) T32 and [testing.md](../testing.md) say so. Decisions 100 and 111 get a back-link line.
