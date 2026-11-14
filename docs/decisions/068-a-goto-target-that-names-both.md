# 068 — A target that names both a line and another save

- **Status:** Accepted
- **Date:** 2026-11-06
- **Agenda topic:** none (milestone 3, its red-team session, RT5-3)
- **Decided by:** Claude, at the owner's request while the owner was away. The owner can reverse any part.
- **Refines:** [decision 055](055-lines-in-v0.1.md) (how `goto` reads its argument)
- **Refined by:** [decision 159](159-tags.md) (a tag's name that is also the start of another save's ID is refused the same way); [decision 162](162-red-team-m7-rulings.md) (a name that is both a line and a tag, at two saves, is refused too)

## Context

Decision 055 reads a target as, in this order, a line's name, a server line, the start of a change ID, the start of a fingerprint. Line names may be any letters and digits, so a line can be named exactly like the short change ID `history` prints for another save. An imported save's change ID is fixed by its commit (decision 032), so whoever made a repository knows in advance what `history` will show, and can name a branch to match. Someone who imports it and types `bolt goto <the ID history shows>` then lands on the branch's save instead, told only "You're on line …". The red-team session proved it (RT5-3). `show` and `changes --save` read their argument the same way.

## Options

- **A. Refuse text that is both.** When the text is a line's name and also the start of the change ID or fingerprint of another save, refuse it and list those saves. If the line's own save is the only one the ID names, there is nothing to choose and the line is taken. Nothing new to learn, and it is how an ambiguous start is already refused. Git, by contrast, takes the branch.
- **B. Refuse such names for lines,** for `save --line` and on import (renamed by decision 033's rule, and listed). Stops it at the source, but a name like `work` or `tops` is all letters from k to z, so it would refuse ordinary names, and a later save can still come to match an existing line.
- **C. A syntax to pick one,** such as `line:<name>`. Git has `refs/heads/<name>`. It adds something to learn for a rare case, and a planted repository can still mislead whoever doesn't use it.

## Decision

**A.** `goto`, `show` and `changes --save` refuse a text that is a line's name and the start of another save's change ID (on the current line) or fingerprint (at least 8 hex digits). The message lists the change IDs or fingerprints it matched; the hint says to type more of the ID to reach the save. A text that names only the line's own save is the line.

## Consequences

- The save is reached by typing more of its change ID, or its fingerprint. The line can't be reached by that name until lines can be renamed (v0.2); a save's fingerprint still reaches its save.
- A line name made only of the letters k to z, or of 8 or more hex digits, now costs a walk of the current line's history (or of every line's, for hex) each time it is named, as a change ID does.
- [json.md](../json.md) (`goto`, `show` and `changes`), [design.md](../design.md) and [security.md](../security.md) T21 describe this.
