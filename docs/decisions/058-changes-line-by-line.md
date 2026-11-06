# 058 — `bolt changes` shows line-by-line differences, with the `similar` crate

- **Status:** Accepted
- **Date:** 2026-11-05
- **Agenda topic:** none (milestone 3 planning, question 11)
- **Decided by:** Claude, at the owner's request ("do whatever you think is best", 2026-11-05). The owner can reverse any part.
- **Refines:** [decision 017](017-commands-and-output.md) (`changes`, Git's `diff`)
- **Refined by:** [decision 063](063-changes-takes-the-save-with-a-flag.md) (the save is named with `--save`)

## Context

`bolt changes` shows what changed inside text files, line by line; milestone 1 left line-by-line differences for milestone 3. It needs a diff algorithm, a rule for which files are text, and limits, since a project can hold huge or hostile files (security.md T11: "time limits on diffs"). A new dependency needs the owner's approval (decision 022's review rule).

## Options

- **A. `imara-diff`** 0.2 (Myers and histogram, fast; used by gitoxide and Helix; Apache-2.0; last released 2025-06). No time limit.
- **B. `similar`** (Myers and patience; Apache-2.0; released 2026-10; already in `Cargo.lock` through `insta`). It has a deadline: past it, the diff it gives is coarser, not unbounded.
- **C. Our own Myers.**

## Decision

**B.**

- **`bolt changes [paths]`** shows unsaved changes against the current save; **`bolt changes <save>`** shows what that save changed against its first parent. `show` stays the summary of files.
- **Binary files:** a file with a zero byte in its first 8,000 bytes is binary, as Git decides; it shows as changed, with its sizes, and no lines. So is a file over 64 MiB.
- **Limits:** each file gets at most 2 seconds; past that `similar` gives a coarser diff, and Bolt says so.
- **Output:** the unified form with 3 lines of context, colored only in a terminal; every line from a file is escaped before printing (T21). `--json` gives each file's hunks, documented in [json.md](../json.md).

## Consequences

- `similar` becomes a dependency of the engine, under security.md T18's review.
- Line endings show as they are; a file that only changed CRLF to LF shows every line changed, and the summary says so.
