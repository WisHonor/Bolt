# 050 — What `goto` and `discard` do with unsaved work

- **Status:** Accepted
- **Date:** 2026-11-05
- **Agenda topic:** none (milestone 3 planning, question 3)
- **Decided by:** Claude, at the owner's request ("do whatever you think is best", 2026-11-05). The owner can reverse any part.
- **Refines:** [decision 003](003-working-copy-model.md) (safety copies; `goto` carries unsaved changes along unless they collide)
- **Refined by:** [decision 065](065-matching-work-doesnt-collide.md) (unsaved work that already matches the target doesn't collide)

## Context

Decision 003 decided the shape: before a command overwrites or removes unsaved work it stores a safety copy and records it in the diary, for `undo` (milestone 4) to bring back; `goto` carries unsaved changes along when nothing collides, and otherwise stops. Bolt tracks new files by itself, so a new file is unsaved work too. Left open: what counts as a collision, what `discard` does with new files, and whether either asks first.

## Options

For new files under `discard`:
- **A. Remove them, with a safety copy,** so the folder matches the save. Jujutsu's `restore` does this, since it tracks new files itself too.
- **B. Leave them,** as Git's `restore` does (`git clean` removes untracked files).
- **C. Ask each time.**

## Decision

**A**, and:

- **Unsaved work** is a difference between the folder and the current save: a changed file (contents, kind or executable bit), a new file no ignore rule keeps out, or a deleted file. An ignored file that the save doesn't hold is never touched by `goto` or `discard`.
- **Safety copies** are made of what the command would overwrite or remove and the current save doesn't hold: a changed file's contents, a new file. A deleted file needs none. They go into the store, are listed in the diary entry, and are kept for the undo window; `undo` arrives in milestone 4.
- **`goto` collisions.** Unsaved work collides when the target save differs from the current save at the same path, or at a folder above or below it, or when the target would write over a file an ignore rule keeps out. Then `goto` changes nothing, lists the paths, and says to save or discard them first (shelving comes in milestone 4). Other unsaved work comes along.
- **`discard [paths]`** makes those paths, or the whole folder without any, match the current save: changed files restored, deleted ones back, new ones removed. It asks nothing, since every step can be undone, and says what it threw away and that a safety copy was kept.
- **Never touched:** entries this computer can't hold (decision 051), project links and what their folders hold, folders that hold a repository or project of their own (decision 046), and `.git` and `.bolt`.

## Consequences

- The diary entry's safety-copy list (storage-format.md §9) is filled from milestone 3.
- Until milestone 4 there is no command that brings a safety copy back; nothing deletes them before then either.
