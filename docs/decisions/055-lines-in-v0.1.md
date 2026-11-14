# 055 — Lines in v0.1: saving away from a line, listing them, switching

- **Status:** Accepted
- **Date:** 2026-11-05
- **Agenda topic:** none (milestone 3 planning, question 8)
- **Decided by:** Claude, at the owner's request ("do whatever you think is best", 2026-11-05). The owner can reverse any part.
- **Refines:** [decision 012](012-lines.md) (saving away from a line asks for a new line's name, or takes one from a flag) and [decision 021](021-v0.1-command-scope.md) (lines are created by saving away from a line)
- **Refined by:** [decision 068](068-a-goto-target-that-names-both.md) (a line name that is also the start of another save's ID is refused); [decision 158](158-making-renaming-and-deleting-lines.md) (`save --line` works from a line, and `line new` makes one without saving); [decision 159](159-tags.md) (a tag's name comes third in what `goto` takes)

## Context

Milestone 3 brings basic lines: saving after `goto` to a save makes a line, `line` lists them, and `goto <line>` switches. Decision 012 settled the shape. Left open: the flag's name, what `goto` to the newest save of a line means, how `goto` reads its argument, and what `line` shows.

## Options

For `goto` to a save that is a line's newest:
- **A. At that save, not on the line,** with a hint to `goto` the line instead. Git detaches on `git checkout <commit>`; Mercurial makes a bookmark active only when you update to it by name.
- **B. On the line, since it is its newest save.** Convenient, but the same command then means two things.

## Decision

**A**, and:

- **Saving at a save** asks for the new line's name in a terminal; without one it stops and names the flag `--line <name>`, which works in a terminal too. The name follows decision 012's rules and may not be a line's name already, in any letter case. The new line points at the new save, and the project is on it. On a line, `--line` is refused: making a line from there comes in v0.2.
- **`goto <name>`** takes, in this order: a line's name (on that line), a server line `line@server` (at its save; saving there makes a line), the start of a change ID, the start of a fingerprint (at least 8 hex digits). An ambiguous start is refused, listing the saves, as `show` does.
- **`bolt line`** lists the lines in name order, marking the current one, then the server lines; with `--json`, a document [json.md](../json.md) describes.

## Consequences

- Each save made on a line keeps recording the line in its `line` extra field (decision 012); a save made at a save records the new line.
- `history` shows the line or save the project is on.
