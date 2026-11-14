# 159 — Tags: making, listing, moving and deleting them

- **Status:** Accepted
- **Date:** 2026-11-14
- **Agenda topic:** none (milestone 7 planning)
- **Decided by:** Claude, at the owner's request while the owner was away. The owner can reverse any part.
- **Refines:** [decision 012](012-lines.md) (tags are fixed names; moving or deleting one asks first); [decision 017](017-commands-and-output.md) (`tag`); [decision 034](034-tags-kept-and-shared.md) (the tag list); [decision 055](055-lines-in-v0.1.md) (what `goto` takes, and in which order); [decision 116](116-gits-words-as-shortcuts.md) (Git's word `tag`)

## Context

Tags are part of format 1: the diary points to a tag list (decision 034), and a Git import fills it. But no command makes, lists or deletes a tag, and `opalro goto v1.0` doesn't find one. Decision 012 says what a tag is: a fixed name for an important save, which never moves by itself, and which Opalro asks about before moving or deleting.

Left open: the command's forms, which names a new tag may have, what a tag's name means where a save is asked for, and where tags are shown.

## Options

**Names for tags made in Opalro**

- **A. The line-name rules** (letters, digits, `-`, `_`, `.` and `/`). `v1.0` and `release/2026-11` fit, and nothing needs escaping on screen or quoting in a shell.
- **B. Any text,** as an imported tag may have (Git allows most characters).

An imported tag keeps the name it came with either way.

**Naming the save to tag**

- **A. `--save <save>`,** as `changes --save` does, so a message is never taken for a save.
- **B. A second bare word,** as in `git tag <name> <commit>`.

## Decision

**A, and A.**

- **`opalro tag`** lists the tags: each name, its save (12 hex digits) and the first line of its message. It reads the tag list only, never the saves.
- **`opalro tag <name> ["message"] [--save <save>]`** tags the save the project is on, or the one named.
  - The name follows the line-name rules. It may not be another tag's name in a different letter case, and may not be exactly a line's name.
  - **A name that is a tag already:** at the same save, there is nothing to do. At another save, the tag would move, so Opalro asks first (`--yes` for scripts). A moved tag keeps its message unless a new one is given. It loses the Git tag objects it was imported with, since those describe the old place.
- **`opalro tag --delete <name>`** asks first (`--yes` for scripts), as decision 012 says. `opalro undo` brings the tag back.
- **Each is one diary step,** named `tag`, with no change to the folder.
- **A tag's name works where a save is asked for:** in `goto`, `show`, `changes --save` and `tag --save`. The order is: a line, a server line, a tag, the start of a change ID, the start of a fingerprint. `opalro goto <tag>` puts the project at the tag's save, not on a line, and says so. A tag's name that is also the start of another save's ID is refused, listing both, as a line's name is (decision 068).
- **`history` and `show` name the tags at each save.**
- **Git's word** `tag` is Opalro's too. `tag -d <name>` runs `tag --delete <name>`. Git's other options for tags are refused as unknown, as on any command.

## Consequences

- A tagged save is kept by `tidy` and can't be amended. Both were already so.
- `history` reads the tag list once per run. The 100,000 tags an import may bring make a table of that size for the run.
- `undo --list` still says only that "the tags changed" for a tag step.
- A tag made in Opalro holds a name, a save and a message. Who made it, and when, isn't in the tag list's format; adding that would need a feature (decision 121).
- Imported tags with names outside the line-name rules can be listed, gone to, moved and deleted, but not made again under that name.
