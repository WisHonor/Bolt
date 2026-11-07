# 073 — Shelves are kept saves, numbered, the user's own, and come back onto any save

- **Status:** Accepted
- **Date:** 2026-11-06
- **Agenda topic:** none (milestone 4 planning)
- **Decided by:** Claude, at the owner's request while the owner was away. The owner can reverse any part.
- **Refines:** [decision 003](003-working-copy-model.md) (`bolt shelf` sets unsaved changes aside as a hidden save) and [decision 021](021-v0.1-command-scope.md) (`shelf` and `unshelf` in v0.1)

## Context

Decision 003 makes `bolt shelf` set unsaved changes aside as a hidden save, and shelving undoable; decision 021 puts `shelf` and `unshelf` in v0.1; decision 050 left `goto`'s "shelve them" for milestone 4. A diary entry's state already holds the shelves, newest first, each a unique label and a save. Left open: what a shelf takes, how shelves are named, how one comes back (onto another save, too), how undo treats them, and how to see them.

## Options

- **A. Kept saves listed in the state** (Git's stash keeps commits; Mercurial's and Sapling's `shelve` keep bundles of commits). A shelf is a kept save ([decision 071](071-a-step-records-its-change-to-the-folder.md)) named in the state, so `tidy` and every rule for saves apply to it unchanged.
- **B. Patch files** (Mercurial's `shelve` before it used bundles). Text patches can't hold binary files or links byte for byte, which "bytes in = bytes out" rules out (decision 004).
- **C. No shelf** (Jujutsu, whose working copy is a commit: you just start another). Not Bolt's model, where the folder is separate from history (decision 003).

## Decision

**A.**
- **`bolt shelf ["message"]`** sets every unsaved change aside: the folder's version of each changed, new or deleted file or link and each new folder is kept as a save on top of the current one, exactly as `discard` keeps what it throws away; then the folder is put back to the current save as `bolt discard` puts it back, with the same rules (ignored files, project links, entries this computer can't hold, nested projects and links that couldn't be written back as links are never touched, and something in the way stops it before anything changes). The message is "Shelved" when none is given. With nothing unsaved it says so, and makes no step.
- **Numbers.** A new shelf takes the smallest number from 1 that no shelf has, and goes first in the list, as the newest.
- **`bolt unshelf [number]`** brings back the newest shelf, or the one with that number. Its changes are made as a switch from the save it was shelved on to the shelf, carrying all other unsaved work along, so each change lands where the folder still holds what that save held. It works on any save, not only the one shelved on, as long as nothing collides. Where the folder differs from that save at a path the shelf changes (unsaved work there, or the current save changed it since), nothing is changed: the paths are named, and the hint says to save, shelve or discard that work and then bring the shelf back by its number (a new shelf would be the newest), or, where the current save changed them, to go to the save shelved on (`bolt goto` and its ID, when a line still reaches it, else `bolt undo --list`, whose steps can bring it back) and unshelf there (combining comes in v0.2). Otherwise the shelf leaves the list, and the step records its switch as decision 071 says: from the current save with what the folder held at the shelf's paths, to the current save with the shelf's version there.
- **Shelves are the user's own,** like Git's stash: undo, redo and `undo --to` leave the list as it is, except that undoing a step takes off a shelf the step itself added (a `shelf`, or a jump made with `--shelve`) and puts back one it removed (an `unshelf`), and redoing it does the opposite ([decision 070](070-undo-and-redo-step-through-the-diary.md)). A shelf put back keeps its number unless another shelf has it, and then gets the smallest number free. `bolt undo --shelve` adds a shelf of its own (decision 070).
- **`bolt shelf --list`** shows each shelf's number, message, the line it was shelved on, when, and how many files it changed.
- **After a stopped switch,** `unshelf` brings no shelf back in a run that finished a stopped one, so running it again after files in use or a crash doesn't bring back a second shelf ([decision 075](075-crash-recovery.md)).
- **Not in v0.1:** shelving only some paths (beyond what `undo --shelve` sets aside), keeping a shelf in the list while bringing it back, and dropping a shelf (bring it back, then discard: both can be undone).
- `goto`'s hint for unsaved work in the way names `bolt shelf` beside saving and discarding.

## Consequences

- A shelf's files stay stored until it is brought back, and then for the undo window.
- Undoing `shelf` brings its changes back and takes it off the list; undoing `unshelf` takes its changes out of the folder and puts it back on the list.
- Numbers are short to type and stay put while a shelf exists; a number can come back after its shelf is gone.
- A jump with `undo --to` never makes a shelf disappear.
