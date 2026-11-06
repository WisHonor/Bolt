# 064 — `bolt changes` shows no lines for a file of more than 1,000,000 lines

- **Status:** Accepted
- **Date:** 2026-11-06
- **Agenda topic:** none (milestone 3, review of `bolt changes`)
- **Decided by:** Claude, at the owner's request while the owner was away. The owner can reverse any part.
- **Refines:** [decision 058](058-changes-line-by-line.md) (which files show without lines)

## Context

Decision 058 diffs any file up to 64 MiB line by line. The size isn't what costs memory, though: the number of lines is. A diff keeps a little for every line (where it starts, which side it is on, the diff's own tables), several dozen bytes in all. A 64 MiB file made only of empty lines has 67 million lines, and a file like that would take gigabytes, many times its own size. A hostile project can hold such a file (security.md T11).

## Options

- **A. A limit on lines per side**, like the 64 MiB size limit: past it, the file shows as changed with its sizes and no lines. Counting the line breaks is one pass over bytes already read.
- **B. A lower size limit.** That bounds lines only through the size, so it would have to drop to a few MiB, shutting out ordinary big text files.
- **C. Leave it to the 2-second deadline.** The deadline bounds the diff's time, not what it holds before it starts.

## Decision

**A, at 1,000,000 lines per side.** Past it, on either side, a file shows without lines, as one over 64 MiB does, and says it has over 1,000,000 lines.

Why that figure: at 1,000,000 lines, what the diff holds is tens of MB per side, in the same range as the 64 MiB a file may already take, and Myers' tables grow with the number of lines too. Hand-written source files stay far below it (the biggest are around 100,000 lines). Past it, a line-by-line listing is too long to read anyway: lock files, generated code and data dumps.

## Consequences

- A file is diffed only if it is at most 64 MiB and at most 1,000,000 lines on each side, so what one file's diff holds stays bounded.
- `--json` gives such a file `"lines": "too-many-lines"`; [json.md](../json.md) says so, and security.md T11 lists the limit.
