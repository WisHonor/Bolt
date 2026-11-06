# 074 — `save --amend` replaces the newest save of your line

- **Status:** Accepted
- **Date:** 2026-11-06
- **Agenda topic:** none (milestone 4 planning)
- **Decided by:** Claude, at the owner's request while the owner was away. The owner can reverse any part.
- **Refines:** [decision 021](021-v0.1-command-scope.md) (`save --amend` in v0.1)

## Context

Decision 021 puts `save --amend` in v0.1: it replaces the last save with the current changes, a new message, or both, and keeps the save's change ID. Left open: whose name the new save carries, what happens to an imported save's Git fields, which saves may be amended, and what undo does.

## Options

- **A. Replace the newest save of the line, keeping its author and change ID** (Git's `commit --amend` keeps the author and makes you the committer; Jujutsu's rewrites keep the change ID and the author). The new save has the old one's parents, author, change ID and `line`; its saver is whoever amends, now.
- **B. Save on top, and fold the two later.** History stays a straight line, but it isn't an amend, and folding needs combining (v0.2).
- **C. Amend any save, rewriting what came after it** (Jujutsu's `jj describe` or `jj squash` of any change, which rebases its descendants). Needs rebasing, which waits for combining (v0.2).

## Decision

**A.**
- **`bolt save --amend ["message"] [paths]`** makes a new save of the folder's changes on top of the newest save of your line (all changes, or only the paths and patterns named, as [decision 054](054-partial-saves.md) says for any save), with the new message, or the old one when none is given. It has the old save's parents, author, change ID and `line`; its saver is the user, now; it drops `git-commit`, `git-header` and `git-original-message`, which no longer describe it. The line then points at the new save; the old one leaves the line's history and stays stored for the undo window.
- **Nothing to amend:** when the folder holds the save's files and the message is the same, it says so and makes no step.
- **Refused, changing nothing:**
  - at a save rather than on a line: the hint says to go to a line;
  - when another line, a server line or a tag names the save, or another line's history holds it (along first parents, as the change-ID cache follows them): two saves people can see would share one change ID, which only v0.2 can show. The hint says to save a new change instead. Jujutsu refuses the same way, by default, to rewrite what a remote branch or a tag holds.
- **Like any save,** it asks before a large save ([decision 056](056-question-before-large-saves.md)) and stops at the line-ending check after an import ([decision 039](039-first-save-after-import.md)).
- **Undo** puts the old save back on the line and changes no file, so the amended changes show as unsaved again ([decision 070](070-undo-and-redo-step-through-the-diary.md)).

## Consequences

- A save another line reaches only through a second parent isn't seen by the check. Only an import makes such merges in v0.1, and amending one only leaves two saves sharing a change ID until v0.2 shows divergence; nothing is lost.
- `save`'s `--json` document gains `replaced`: the save an amend replaced, or `null`.
