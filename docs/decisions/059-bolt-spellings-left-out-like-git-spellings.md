# 059 — `.bolt` spellings are left out of saves like `.git` spellings, from one module

- **Status:** Accepted
- **Date:** 2026-11-05
- **Agenda topic:** none (milestone 3 planning, question 12; a follow-up from milestone 2)
- **Decided by:** Claude, at the owner's request ("do whatever you think is best", 2026-11-05). The owner can reverse any part.
- **Refines:** [decision 004](004-project-contents.md) (no part that Windows or macOS would treat as `.bolt`) and [decision 043](043-red-team-m2-rulings.md) (names Git treats as `.git` are left out of saves)
- **Superseded in part by:** [decision 150](150-the-name-is-opalro.md) (the names: Bolt is now Opalro, and `bolt` is now `opal`)

## Context

Decision 004 makes every `.bolt` spelling (`.BOLT`, `.bolt.`, `bolt~1`, `.bolt::$INDEX_ALLOCATION`) an invalid name, so a save that meets one below the top stops. The walk skips only a folder named exactly `.bolt`, so a project whose folder is spelled `.BOLT` on Windows opens but refuses every save. Decision 043 leaves names Git treats as `.git` out of saves with a warning instead. The rules for both live partly in the save's walk (`is_internal_folder`), against the rule of one place per rule.

## Options

- **A. Leave `.bolt` spellings out with a warning, as `.git` spellings are,** and skip the project's own `.bolt` in any spelling the drive treats as `.bolt`.
- **B. Keep stopping the save,** fixing only the `.BOLT` project.

## Decision

**A.**

- **The project's own folder:** at the top, `.bolt` and `.git` are skipped without a word in any spelling the drive treats as them (letter case where the drive ignores it).
- **Any other `.bolt` spelling** is left out of a save, with everything in it, and named in the warning that names `.git` spellings, which now covers both. Names stay invalid in listings, so no save can hold one.
- **One module:** the names module holds both tests and the walk asks it.

## Consequences

- The `git_names_left_out` warning keeps its kind and also names `.bolt` spellings; [json.md](../json.md) says so.
- A folder that holds a `.bolt` folder is a project of its own, left out whole (decision 046), so this mostly concerns spellings at the top and names like `bolt~1`.
