# 003 — Working-copy model: separate folder with automatic safety copies

- **Status:** Accepted
- **Date:** 2026-10-31
- **Agenda topic:** 3

## Context

The project folder holds the files being edited; history holds saves. This decision covers unsaved work: what Bolt knows about it, and whether it can ever be lost. It drives whether `undo` can bring back thrown-away work, where conflicts live while being fixed, whether a shelf is needed, and what `goto` does with unsaved changes. Big files make the cost of capturing unsaved work a first-class concern.

## Options

- **Separate folder** (Git, Mercurial, Perforce, Fossil). Nothing enters history until a save. Familiar and cheap, but `discard` destroys work for good, which breaks "undo anything"; shelving and switching with unsaved changes need their own rules.
- **The folder is a live draft save** (jj). Every command first snapshots the folder into a draft save. One mechanism gives undo-anything, saving with conflicts and no need for a shelf, but it is unfamiliar and expensive with big files: every command must capture every changed file. jj itself won't auto-snapshot new files over 1 MiB by default.
- **Separate folder + automatic safety copies** (Bolt's hybrid). Looks like the first option, but Bolt never destroys unsaved work without keeping a copy first, tied to the operation log. Comparable ideas: JetBrains' Local History; Mercurial's `revert` leaves `.orig` backups.

## Decision

**Separate folder + automatic safety copies.**
- Nothing enters history until `bolt save`.
- Before any command overwrites or removes unsaved work (`discard`, `goto`, `combine`, `undo`, `redo`, `shelf`), Bolt captures the affected files and records the copy in the operation log, so undo can always restore them. Only files about to be touched are copied.
- New files are tracked automatically unless ignored. A save that adds unusually much asks first; a flag skips the question for scripts.
- Partial saves: `bolt save <files or folders>`. Choosing parts of a file comes later.
- `bolt shelf` sets unsaved changes aside as a hidden save; shelving is undoable.
- `goto` carries unsaved changes along when nothing collides; otherwise it stops and offers to shelve, save or discard them.
- A save can contain unresolved conflicts, and the save records them.

## Consequences

- Two mechanisms instead of one: saves and safety copies. Both live in the same object store, so content that is already stored costs nothing extra to capture.
- `status` never writes to the store.
- Capturing a large modified file takes time and space; safety copies are cleaned up with the undo window (topics 9, 10).
- Bolt must re-check a file right before overwriting it, in case another program (an editor or game engine) changed it after the capture (topic 11).
- Conflicts live in the folder until saved, then in the save (topic 7).
- The confirmation threshold for large saves is set per preset, since game projects legitimately add gigabytes (topic 16).
- Command names for restoring a shelf and skipping confirmations are settled in topic 17.
