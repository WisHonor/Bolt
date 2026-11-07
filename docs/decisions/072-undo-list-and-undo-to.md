# 072 — `undo --list` shows the steps, and `undo --to` goes to any entry

- **Status:** Accepted
- **Date:** 2026-11-06
- **Agenda topic:** none (milestone 4 planning)
- **Decided by:** Claude, at the owner's request while the owner was away. The owner can reverse any part.
- **Refines:** [decision 010](010-operation-log.md) (undo can jump to any entry) and [decision 017](017-commands-and-output.md) (`undo --list` numbers count steps back from now)

## Context

Decision 017 says the numbers `bolt undo --list` shows count steps back from now, and that `bolt undo 3` undoes the last three steps. Decision 010 says undo can jump to any entry, and [decision 070](070-undo-and-redo-step-through-the-diary.md) makes undo and redo editor-style. Left open: what the list shows, how an entry is named for a jump, and what a jump to an entry outside the undo order does.

## Options

- **A. List the steps, jump by entry ID** (Photoshop's History panel, where undone states stay listed until a new action clears them; Jujutsu's `jj op log` with `jj op restore <id>`). The list shows the steps undo can take, numbered back from now, and above them the steps redo can bring back; each shows the start of its entry's ID. `undo --to <entry>` makes the state what it was right after that entry.
- **B. List the whole diary, numbered** (Jujutsu's `jj op log`, Git's `reflog` with `HEAD@{n}`). Every entry, undos and redos included, numbered back from now, and `undo --to <n>`. The numbers then don't match `undo N`, which counts steps, and they shift whenever another command runs between the listing and the jump.
- **C. No jump**, only `undo N`. Simple, but a step that a later command cleared from the redo list could never come back, which decision 010 promises.

## Decision

**A**, with the whole diary on request.

- **`bolt undo --list`** shows, newest first: the steps redo can bring back, marked as undone and numbered for `bolt redo N` (1 is what `bolt redo` brings back), then the steps undo can take, numbered from 1 (what `bolt undo` takes) for `bolt undo N`. At most 20 undo steps are shown, and the list says when there are more (counting them would read the whole diary). Each line has the number, the first 12 hex digits of the entry's ID, when it was made, the command's name, and what it changed (the save it made and its message, where it went, which shelf, how many files). The first entry of the diary says it can't be undone.
- **`bolt undo --list --all`** shows every entry from `head` back to the first, undo and redo entries and cleared steps included, each with what it did.
- **`bolt undo --to <entry>`** takes the start of an entry's ID (at least 8 hex digits) that matches exactly one entry from `head` back, and makes the state the one right after that entry:
  - a step on the undo or redo list is reached by undoing or redoing steps one at a time, as `undo N` or `redo N` would, so redo still works afterwards;
  - any other entry (an undo or redo entry, or a step a later command cleared) is reached by one new step that puts back that entry's lines, position, tags and server lines (the shelves stay as they are, [decision 073](073-shelves.md)) and switches the folder from the current save to that state's save, carrying unsaved work as `goto` does ([decision 071](071-a-step-records-its-change-to-the-folder.md) records it). `bolt undo` takes that step back;
  - the current entry changes nothing.
  - Unsaved work in the way stops it as it stops `undo`, and `--shelve` sets that work aside the same way ([decision 070](070-undo-and-redo-step-through-the-diary.md)).
- Entry IDs are fingerprints in hex, so they never look like change IDs, which use the letters k to z.

## Consequences

- Each of these commands has a `--json` document ([json.md](../json.md)).
- The same two commands help when `state/head` is lost ([decision 075](075-crash-recovery.md)).
- A step reached by `--to` through the undo order behaves exactly like `undo N`, so there is one way steps are undone.
