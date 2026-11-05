# 051 — Names this computer can't write, and how long names and paths may be

- **Status:** Accepted
- **Date:** 2026-11-05
- **Agenda topic:** none (milestone 3 planning, question 4; the path-length limit deferred by decision 026 and the name limit deferred by decision 035)
- **Decided by:** Claude, at the owner's request ("do whatever you think is best", 2026-11-05). The owner can reverse any part.
- **Refines:** [decision 004](004-project-contents.md), [decision 019](019-review-clarifications.md) #2 and [decision 039](039-first-save-after-import.md) (entries this computer can't hold); [decision 026](026-ownership-check-details.md) #5 and [decision 035](035-git-names-bolt-cant-take.md) (the deferred limits)

## Context

A save can hold names this computer can't create: `aux.txt` or `name.` on Windows, two names that are one file on a drive that ignores letter case (`README` and `readme`), names too long for the system. Decisions 019 and 039 settled the save side: such entries stay in history untouched, and saves keep them. Write-out (milestone 3) needs the other side.

Two limits were left for this milestone. The format bounds a save's depth (4,096 levels) and entries (10,000,000), but not a path's length, which can reach about 1 MB. And a name may be at most 255 bytes, while Windows allows 255 UTF-16 units, up to 765 bytes in UTF-8: a name of 90 Chinese characters is 270 bytes, so a history made on Windows can stop an import.

## Options

- **Twins on a drive that folds them.** A: write neither, report both. B: write the first in byte order (Git writes both, the second over the first, and then warns that "paths have collided").
- **Path length.** A: no limit in the format; write what the system takes. B: at most 4,096 bytes from the top, Linux's `PATH_MAX`.
- **Name length.** A: keep 255 bytes. B: allow Windows' own limit, up to 765 bytes, so a history made on Windows imports whole; where a name doesn't fit, it is an entry this computer can't hold.

## Decision

**A, B and B.**

- **Not written.** Write-out never writes an entry this computer can't hold (decision 004's portability rules for this system, and write-out's own checks of security.md: `.git` and `.bolt` spellings, `\` and `:` on Windows, device names, real 8.3 short names). Such an entry stays in history, `status` lists it as not written on this computer, and saves keep it as it was.
- **Twins** that are one file on this drive are both left unwritten and both reported, as saves keep both as they were (decision 039).
- **Paths** may be at most 4,096 bytes, counted from the top with `/` between names. Readers stop at a longer one as they stop past the depth limit; a save refuses one; an import stops at one, naming it. Bolt writes long paths through folder handles (decision 048), so Windows' 260-character limit doesn't apply to Bolt itself.
- **Names** may be at most 765 bytes in the format. A name fits this system when Windows' 255 UTF-16 units or another system's 255 bytes hold it; one that doesn't is an entry this computer can't hold. A new name a save would add must still fit 255 bytes, since macOS and Linux can't hold more (decision 004's portability check); an import keeps longer ones.

## Consequences

- [storage-format.md](../storage-format.md) §3 states both limits; the names module, the listing decoder, the import and their tests change, and the test vectors gain a long name.
- Decision 026's deferred path-length question and decision 035's name-length question are settled.
