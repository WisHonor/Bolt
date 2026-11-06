# 065 — Unsaved work that already matches the target doesn't collide

- **Status:** Accepted
- **Date:** 2026-11-06
- **Agenda topic:** none (milestone 3, the model test of edits, saves and switches)
- **Decided by:** Claude, at the owner's request while the owner was away. The owner can reverse any part.
- **Refines:** [decision 050](050-unsaved-work-in-goto-and-discard.md) (when unsaved work collides in `goto`)

## Context

Decision 050 says unsaved work collides "when the target save differs from the current save at the same path, or at a folder above or below it". Taken word for word, that stops a `goto` even where the folder already holds what the target would put there: a file deleted that the target doesn't hold either, or a file changed to exactly the target's contents.

The model test (testing.md §3) found that `goto` judged this two ways:
- **For a single file it let the work through.** The user deletes `b` and goes to a save without `b`: nothing collides, and the goto goes ahead.
- **Below a folder it stopped.** The user deletes `e/a` and goes to a save without the folder `e`: the goto stops, naming `e`.

Neither way loses anything. They disagree, though, and the stop asks the user to save or discard work that the switch would leave exactly as it is.

## Options

- **A. Matching work never collides, at a file or a folder.** Only unsaved work that differs from the target can be overwritten or removed, so only it collides. Git's checkout does the same: it switches when the working tree already matches the target at a path, and refuses only where it would overwrite a local change.
- **B. Matching work always collides,** as 050 reads word for word. This is safe but strict: it stops switches that couldn't harm anything, and the single-file case would start stopping too.
- **C. Keep both ways.** The rule would then depend on whether a folder above the work changes, which no user could predict.

## Decision

**A.** An unsaved change collides when all three hold:
- the change itself differs from the target (the folder doesn't hold what the target holds at its path);
- at its path, or at a folder above or below it, the target differs from the current save (a file differs, or a folder is on one side only);
- at that same path, the folder doesn't already hold what the target holds: the same file, a folder where the target has a folder, or nothing where the target has nothing.

A deletion the target agrees with, a deleted folder the target doesn't hold, and a file changed to the target's contents come along and simply stop being unsaved changes. So do new files in a folder the target adds, since the folder is already there and nothing in it is overwritten. The rule is judged in one place, where a switch is planned, for files and folders alike.

## Consequences

- `goto` stops less often and never for work it would leave untouched; nothing it would overwrite or remove goes without a collision.
- `discard` doesn't use collisions, so it is unchanged.
- The model test holds `goto` to this rule exactly, and `goto.rs` has a test for each kind of matching work and one for a real collision.
