# 111 — Rulings after the first red-team session on milestone 6's speed work

- **Status:** Accepted
- **Date:** 2026-11-10
- **Agenda topic:** none (milestone 6's red-team gate, [decision 024](024-red-team-gate.md))
- **Decided by:** Claude, at the owner's request while the owner was away. The owner can reverse any part.
- **Refines:** [decision 105](105-the-change-id-cache-keeps-each-lines-saves-in-order.md) (which lines get a list, and what happens when the lists outgrow the file) and [decision 109](109-the-speed-sessions-follow-ups-and-format-1s-three-open-rows.md) (what a list may be trusted for: a whole ID too); it also writes down the pack reader's block window, which no record had
- **Refined by:** [decision 112](112-rulings-after-the-second-red-team-session-on-the-speed-work.md) (the block window reads a block only while blocks pay for themselves; the check before an amend keeps what it read of the other lines; RT12-1's proof runs on Linux only)

## Context

The first red-team session on the speed work ([report](../redteam/2026-11-10-milestone-6-speed.md)) attacked a copy of the branch from the no-network sandbox and compared it throughout with `main`.

**What held.** Nothing a command says depended on who won the race between the walk and the helper that reads listings ahead ([decision 110](110-the-last-saves-listings-are-read-ahead-of-the-walk.md)). Over 1,050 random projects run through both builds side by side, every exit code, output, warning and file agreed. Damaged second copies of a listing, planted saves, packs that aren't packs, a 2,000-level chain under 256 open files, packs merged under a reader and a folder swapped mid-walk all gave `main`'s answers. No change was missed, no wrong tree or wrong bytes reached the store or the folder, and no repository was swallowed.

**What it proved:** three findings, none of them a loss of data. Each was checked in the code before it was ruled on.

- **RT12-1, Medium.** In a project with many lines over a long history, `save --amend` gave every line its own full copy of the shared history in the change-ID cache: 1.4 GB for 10,000 lines over 1,000 saves, where `main` took 0.47 GB. The cache was then too big for its file, so it was built again by the next amend. Under a memory cap the command died after the amend was made, so the person was told it failed when it hadn't.
- **RT12-2, Low.** A file whose tiny chunks are stored scattered in one pack, which only a planted project has, cost a 64 KiB read for each small read: about 200 KB for each 8-byte chunk, and twice `main`'s time.
- **RT12-3, Low.** `goto` with all 64 digits of an ID believed a list that held it. A list that lies under a good seal could then give a store error for a folder listing's ID, or send `goto` to a real save that no line reaches, which a command without the cache refuses to find.

## Decision

All three are fixed. None is ruled away.

1. **A list is made only for a line the project is on (RT12-1).**
   - The amend check asks each other line whether it holds the save or reaches its change ID by walking that line from its newest save, until it meets the amended line's own list or a save an earlier walk passed. No list is built for it. Memory no longer grows with lines times history; the walk costs a read only for a save no list holds, and `main` paid the same in time.
   - **When the lists outgrow the file** (256 MiB), the position's own list is kept, then the other lines in order of name while they fit. Before, a cache one line too big was never written at all, and every command rebuilt it.
   - The file's size is worked out before anything is encoded, the buffer is asked for in a way that can fail, and a cache that can't be kept is skipped: keeping a cache can never abort a command or fail it.
2. **A small read away from the block held costs a small read (RT12-2).** This is also where the pack reader's block window is written down:
   - A read of at most 4 KiB from a pack's entries, or from its index, may be answered from the last 64 KiB block read of each. A pack never changes once it is named, so a block is only ever a copy of what the file holds.
   - **A block is read only at the second read in a row that falls in it.** The first read of a block the reader doesn't hold reads just the bytes asked for. So scattered reads each cost what they ask for, and a run of reads side by side costs one small read, then the block, then nothing.
   - The single probes of a search in the index read the file directly: several probes of one search fall in one block by nature, and would look like a run.
3. **A whole ID is checked against its line's saves before a list is believed (RT12-3).** As for the start of a change ID ([decision 109](109-the-speed-sessions-follow-ups-and-format-1s-three-open-rows.md)): the list is checked link by link from the line's newest save down to the ID. If a link is wrong, the lists are dropped and the lines are walked, so the answer is the one a command without the cache gives.
4. **What the session didn't reach** goes to the second session: the other races in the walk, crash sweeps through a big save and a big `goto`, the memory of the widest folders, ten thousand small packs, a line of a million saves.
5. **What the sandbox can't run** is a row in the [follow-ups](../plans/2026-11-01-milestone-1-follow-ups.md): the speed work attacked on Windows and macOS (short names, the 256-file limit, composed and decomposed names), on a drive that folds letter case, and across a power cut (whether an unflushed cache or an early flush can leave a torn file that is trusted). The build's own tests run on all three systems on every push; a power cut needs a machine that can replay one.

## Consequences

- The three proof tests run on every push (`crates/bolt-vcs/tests/redteam12.rs`): RT12-1's on Unix, where an address-space cap can be set; RT12-2's on Linux, where the bytes a process read can be counted; RT12-3's everywhere.
- An amend in a project where another line has a long history of its own, which no list holds, reads that history on every amend.
- A scattered 8-byte chunk costs about 3 KB of reads where it cost 192 KB (its proof test allows 4,096 bytes), and listing all 24,498 saves of Git's own line takes 0.30 s on Linux where it took 0.49 s, since a lookup in a big index no longer reads a block each time. The third gate's steps measure the same as before the fixes, within the difference between two runners ([benchmarks.md](../benchmarks.md) §8).
- [design.md](../design.md) §5.4 and §5.7 and [security.md](../security.md) T32 say so. Decisions 105 and 109 get a back-link line.
