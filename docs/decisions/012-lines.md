# 012 — Lines: movable names, with a "made on" note

- **Status:** Accepted
- **Date:** 2026-10-31
- **Agenda topic:** 12
- **Refined by:** [decision 033](033-git-branches-to-lines.md) (server lines exist from v0.1, filled by a Git import; server names follow the line-name rules); [decision 034](034-tags-kept-and-shared.md) (tags and server lines live in shared records; imported annotated tags keep their Git bytes); [decision 055](055-lines-in-v0.1.md) (lines in v0.1); [decision 121](121-format-1-freezes-at-v0.1.md) (line names stay compared by Unicode's lowercase mapping when format 1 freezes)

## Context

A line is a named line of work (Git: branch). Lines exist from v0.1 (`main`); combining arrives in v0.2 and server lines in v0.3. This decision fixes what a line is and the rules v0.1 must respect; the full design of combining comes before v0.2.

## Options

- **Movable name tags** (Git branches, Mercurial bookmarks). Saving on a line moves it forward; you can step off lines and back. Familiar and simple, but history forgets which line a save was made on.
- **No current line** (jj). Work happens on unnamed saves, and names are moved by hand. Flexible, but confusing: the line doesn't move when you save.
- **Line name stamped into every save** (Mercurial's named branches; Plastic). History always shows where work happened, but names become permanent.

## Decision

**Movable name tags, plus a "made on" note in each save.**
- A line is a name pointing at a save; saving while on a line moves it forward.
- Each save records, as an extra field (decision 006), the name of the line it was made on. It is information for history and the future website only; renaming a line later never rewrites old saves.
- Lines and tags live in diary entries (decision 010), never as files. Renaming or deleting a line is an undoable step, and filesystem case rules can't affect lines.
- **Names:** letters, digits, `-`, `_`, `.` and `/` for grouping; no spaces; two names may not differ only by letter case. `bolt start` creates `main`.
- **Not on a line:** after `goto` to a save rather than a line, Bolt says the user is looking at an older save; saving there asks for a new line name (or takes one from a flag, for scripts).
- **Tags:** fixed names for important saves. They never move; moving or deleting one asks for confirmation.
- **Combining (v0.2), direction only:** per-file three-way merge from the common starting point; text merged line by line; binary files never merged automatically; move notes followed; several common starting points are recorded through decision 007's multi-ancestor conflict record.
- **Server lines (v0.3):** the diary remembers where each server's lines were, shown as `main@origin`.

## Consequences

- The diary state must hold local lines, tags, the user's position (a line or a bare save), and per-server line positions (empty until v0.3).
- History views can show where each save was made, even after lines are combined or deleted.
- Fast-forward behaviour, moving saves between lines (rebase) and the combine commands are designed before v0.2; command names in topic 17.
- Line names received from servers (v0.3) are untrusted input and get the same validation as local ones.
