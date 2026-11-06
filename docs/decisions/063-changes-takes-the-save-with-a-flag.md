# 063 — `bolt changes` takes the save with `--save`

- **Status:** Accepted
- **Date:** 2026-11-06
- **Agenda topic:** none (milestone 3, building `bolt changes`)
- **Decided by:** Claude, at the owner's request while the owner was away. The owner can reverse any part.
- **Refines:** [decision 058](058-changes-line-by-line.md) (how a save is named on the command line)

## Context

Decision 058 gives two forms: `bolt changes [paths]` for unsaved changes, and `bolt changes <save>` for what a save changed. Both take a bare word, so `bolt changes abc` could mean the file `abc` or the save whose change ID starts `abc`. A short change ID is a few letters, and file names like `main` or `a1b2` are ordinary.

## Options

- **A. A flag for the save:** `bolt changes --save <save> [paths]`. A bare word is always a path. Git does the reverse, and needs `--` to tell them apart (`git diff abc -- file`).
- **B. Guess:** a word that names a save and no file is a save. A file created later changes what the same command means.
- **C. Keep 058's form and refuse a word that could be both.** Users then learn the rule from an error.

## Decision

**A.** `bolt changes [paths]` shows unsaved changes; `bolt changes --save <save> [paths]` shows what that save changed against its first parent, limited to the paths if any are given. A bare word is never taken for a save.

## Consequences

- One more word to type for a save's changes, and no command whose meaning depends on which files exist.
- [design.md](../design.md) and [json.md](../json.md) describe the `--save` form.
