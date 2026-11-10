# 040 — Importing is one-time in milestone 2; catching up with Git comes later

- **Status:** Accepted
- **Date:** 2026-11-02
- **Agenda topic:** none (milestone 2 planning, question 10)
- **Refined by:** [decision 123](123-follow-ups-done-or-moved-past-v0.1.md) (catching up with Git moves past v0.1)
- **Decided by:** Claude, at the owner's request to keep going with Claude's recommendations.
- **Confirms:** [decision 019](019-review-clarifications.md) #6 (import is one-time in v0.1; picking up new commits comes later)

## Context

After an in-place import, Git and Bolt share the folder, and people may keep committing in Git while they try Bolt. Re-running the import would bring their new commits in. Imports are repeatable ([decision 032](032-exact-repeatable-commit-import.md)): the same commit always becomes the same save, so a second import only adds what is new.

## Options

- **A. One-time,** as decision 019 says. A second `bolt import` in the project refuses.
- **B. Catch up.** A second import adds new commits and replaces the server lines with Git's current view. It moves a line to its branch's new commit only if the line hasn't moved in Bolt since the last import; otherwise it leaves the line and reports it, since combining arrives in v0.2.
- **C. Mirror.** Lines follow Git exactly, even backwards, which can take saves made in Bolt off a line.

## Decision

**A for milestone 2.**

- A second `bolt import` in a project says the project was already imported and that catching up with Git comes later.
- Until then, importing again means importing into a fresh folder.

**B is the planned next step.** Decision 032 already makes it small: unchanged commits become the same saves, and only the line rules are new. It is built before v0.1 if the milestones leave room, otherwise after.

## Consequences

- Milestone 2 stays its planned size.
- Catching up with Git is listed as deferred ([design.md](../design.md) §7).
