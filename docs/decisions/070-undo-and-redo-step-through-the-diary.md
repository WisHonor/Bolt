# 070 — Undo and redo step through the diary like an editor's history

- **Status:** Accepted
- **Date:** 2026-11-06
- **Agenda topic:** none (milestone 4 planning)
- **Decided by:** Claude, at the owner's request while the owner was away. The owner can reverse any part.
- **Refines:** [decision 010](010-operation-log.md) (how undo and redo walk the diary, and what they put back) and [decision 017](017-commands-and-output.md) (`bolt undo 3`)

## Context

Decision 010 set the shape: every changing command adds a diary entry holding the whole state after it; undo and redo add entries that copy an earlier state and restore files from safety copies; they behave like an editor's Ctrl+Z and Ctrl+Y, a new command clears what redo could bring back, and undo can jump to any entry. Decision 017 adds that `bolt undo 3` undoes the last three steps, and design.md §5.8 shows an undone save leaving its changes as unsaved work. Building it (milestone 4) needs answers to:
- after an undo, what the next undo takes back, and what redo brings back;
- what an undo or a redo does to the project folder, for each kind of command;
- what happens when unsaved work is in the way, and how the user gets past it;
- what can't be undone.

## Options

- **A. Step back one entry at a time, the undo itself included** (Git's `git reset --hard HEAD@{1}`; Jujutsu's `jj undo` before version 0.33). Undo goes back to the state before the newest entry, so a second undo undoes the first. Simple, but it can't go back more than one step, which is the first thing anyone tries with an editor's undo.
- **B. Editor-style undo and redo, worked out from the diary** (every editor; Sapling's `sl undo` and `sl redo`; Jujutsu's `jj undo` and `jj redo` since 0.33). Each command is a step. Undo takes back the newest step not yet undone, redo brings back the step undone last, and a new command clears what redo could bring back. Both lists follow from the undo and redo entries the diary already holds, so nothing else is stored.
- **C. Restore only** (Jujutsu's `jj op restore`; Git's reflog). There is no undo order: the user picks an entry, and its state comes back as a new step. Powerful, but every undo needs a lookup first.

## Decision

**B**, with C's reach through `undo --to` ([decision 072](072-undo-list-and-undo-to.md)).

- **Steps.** Every entry a command made (action "command") is a step. Undo and redo entries move between steps; they are not steps themselves.
- **The two lists.** At any entry there are the steps done (undo takes the newest) and the steps undone (redo takes the one undone last). They follow from the diary: a command entry adds its step to the done list and empties the undone list; an undo entry moves its target from the done list to the undone list; a redo entry moves it back. Bolt works them out by walking back from `head`, never reading the whole diary: the newest done step is the entry itself for a command entry, the target for a redo entry, and for an undo entry the newest done step of the entry before its target. The undone list is collected walking back to the newest command entry.
- **The state.** Undoing a step puts back the lines, where you are, the tags and the server lines as they were just before the step (the state of the step's parent entry); redoing it puts back the step's own. A step can only be undone once every later step is undone, so this never loses a later change.
- **Shelves are the user's own,** as Git's stash is: undo and redo leave the list of shelves as it is, except that undoing a step takes off a shelf the step itself added (a `shelf`, or a jump made with `--shelve`) and puts back one it removed (an `unshelf`), and redoing it does the opposite ([decision 073](073-shelves.md)). A shelf put back keeps its number unless another shelf has taken it, and then gets the smallest number free.
- **The files.** A step that changed files records what the folder held and what it wrote, at exactly the paths it wrote ([decision 071](071-a-step-records-its-change-to-the-folder.md)). Undo writes back what the folder held there and redo writes again what the step wrote, both through `goto`'s planner and write-out, so unsaved work anywhere else comes along untouched. A step that changed no file, such as `save` or `save --amend`, changes none when it is undone or redone: its saved changes simply show as unsaved again, as design.md §5.8 shows.
- **Unsaved work in the way.** Where a path the step wrote has changed since (unsaved work, or something Bolt ignores, in the way), the undo or redo stops before anything changes and names the paths, as `goto` stops (decisions [050](050-unsaved-work-in-goto-and-discard.md) and [065](065-matching-work-doesnt-collide.md)). Saving, shelving or discarding that work first doesn't help here, since each is a step of its own that the next undo would take back first. So the hint names **`--shelve`**: `bolt undo --shelve` (and `bolt redo --shelve`) sets the unsaved changes in the way aside on a new shelf, as part of the same step, and then makes it; files Bolt ignores that are in the way still stop it. Undo and redo never overwrite unsaved work.
- **Several steps.** `bolt undo N` and `bolt redo N` take N steps one after another, each its own diary entry, so each can be redone or undone again on its own. The first step that can't be taken stops the command, which says how many were taken.
- **What can't be undone.** The diary's first entry (made by `start` or `import`): `bolt undo` then says there is nothing to undo. A step whose saves `tidy` has removed ([decision 077](077-tidy-and-the-undo-window.md)) can't be undone or redone, and the error names the step. `tidy` itself is not a step.
- **After a stopped switch.** An undo or redo that a crash or a file in use stopped is finished by the next changing command ([decision 049](049-writing-files-one-at-a-time.md)). `undo` and `redo` themselves take no further step in a run that finished one, so running the same command again finishes it and stops ([decision 075](075-crash-recovery.md)).
- **Only this repository** (decision 010): nothing here changes for anyone else. Undoing what others have received comes with syncing (v0.3).

## Consequences

- `bolt undo` with nothing to undo, and `bolt redo` with nothing to redo, succeed and say so; scripts read the lists from `--json`.
- An undo or redo that changes files is a switch like `goto`'s: the journal is written first, `head` moves last, and after a crash the next changing command finishes it.
- A shelf made by `--shelve` holds work made on the save undone from, so it comes back onto that save; elsewhere `unshelf` stops where that save's files differ (decision 073).
- The order of steps is a property of the diary alone, so it can be tested against a model: random commands, undos and redos must leave exactly the state and the files the model predicts (testing.md §3).
