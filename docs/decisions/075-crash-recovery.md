# 075 — Crash recovery: every step survives a crash, and a lost head is picked up from the diary

- **Status:** Accepted
- **Date:** 2026-11-06
- **Agenda topic:** none (milestone 4 planning)
- **Decided by:** Claude, at the owner's request while the owner was away. The owner can reverse any part.
- **Refines:** [decision 010](010-operation-log.md) (a command takes effect when `head` moves) and [decision 049](049-writing-files-one-at-a-time.md) (finishing a switch a crash stopped)
- **Refined by:** [decision 081](081-what-building-milestone-4-refined.md) (edits made while milestone 4 was built)

## Context

Milestone 3 finishes a switch that a crash stopped, on the next command (decision 049). Milestone 4 lists crash recovery as one of its goals, and three things are left:
- the new commands (`undo`, `redo`, `shelf`, `unshelf`, `save --amend`, `tidy`) must survive a crash at any point;
- a command whose `head` has moved can still report failure: flushing the `state` folder after the rename can fail, and the command then exits 1 although it happened (a follow-up from milestone 1's red-team fixes);
- `state/head` itself can be lost: deleted, damaged, or naming an entry the store doesn't hold (a failing disk, a sync tool, a restore from a backup). Every command stops then, though every save is intact, and nothing helps the user go on.

## Options

For a lost `head`:
- **A. Pick it up from the diary.** Commands say what is wrong and that the saves are safe. `bolt undo --list` lists the diary entries the store holds that no other entry follows (the places the diary may have stopped), newest first, and `bolt undo --to <entry>` makes one current. Nothing is guessed. Like Git, where a lost branch is found again through the reflog or `git fsck --lost-found` and set by hand.
- **B. Repair it without asking:** take the newest entry by its time. An entry a crashed command stored but never made current can be newer than the real `head`, and a planted pack can claim any time, so the guess can make a crashed command look done, or put a stranger's state in place.
- **C. Leave it to `verify`** (milestone 6), as Git leaves a damaged `HEAD` to `git fsck` and hand edits.

## Decision

**A**, and:
- **Every new command is crash-safe**, and crash tests stop it at each of its fault points. It takes effect when `head` moves; one that changes files does so through the journal, so the next changing command finishes it (decision 049); `tidy` writes its new packs before it removes any old one, as merging does ([decision 077](077-tidy-and-the-undo-window.md)).
- **Running a command again finishes it, and no more.** Bolt's hint after files in use, or after a crash, is to run the command again; the next changing command then finishes the stopped switch before doing its own work (decision 049). For `goto`, `discard`, `shelf` and `save` that is harmless, since doing their own work again changes nothing more. `undo`, `redo` and `unshelf` take the next step, or the next shelf, each time, so in a run that finished a stopped switch they stop there, say what was finished, and say that running them again takes one more.
- **Once `head` has moved, nothing fails the command.** A flush of `state/` that fails after the rename is a `head_not_flushed` warning: the command happened, but a power cut soon after could undo it.
- **A lost `head`** is a missing `state/head`, one that isn't an entry's ID, or one naming an entry the store doesn't hold or can't read. Every command, read-only ones included, stops with an error that says so, that the saves are safe, and which two commands help. Then:
  - `bolt undo --list` lists the diary entries in the store that no other entry has as its parent, newest first by the time they record, with their commands;
  - `bolt undo --to <entry>` (any entry the store holds whose state's saves it can read; one it can't is refused, naming it, so the pick never strands the project) adds a step after that entry, holding its state, and makes the new step current. It changes no file: `status` then shows how the folder differs from that state's save.
  - Every other `undo` and `redo` waits until then.
- Windows renames: [decision 076](076-renames-reach-the-disk-on-windows.md).

## Consequences

- Listing a lost `head`'s candidates reads the index of every pack; on a huge store that takes a while, and it only runs when `head` is lost.
- A `head` naming an older entry that reads well is still trusted (decision 010): Bolt never second-guesses a `head` it can use.
- An entry left by a crash stays out of the diary until `tidy` removes it, unless the user picks it here.
