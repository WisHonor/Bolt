# 133 — Red team before v0.1: two findings fixed, and a planted save inside the limits stays a known cost

- **Status:** Accepted
- **Date:** 2026-11-11
- **Agenda topic:** none (getting v0.1 ready: the red team's session on the rewritten readers and the deep tree)
- **Decided by:** Claude, at the owner's request while the owner was away. The owner can reverse any part.
- **Refines:** [decision 130](130-a-walk-measures-a-listing-it-meets-again-before-walking-it-again.md) (what a save within the limits may cost); [decision 131](131-code-that-follows-another-projects-is-rewritten-clean.md) (the rewritten readers)

## Context

The red team's session before v0.1 (RT14, [report](../redteam/2026-11-11-v0.1-release.md)) attacked the readers written again under decision 131, the deep-tree refusal of decision 130, and the two halves of milestone 6 together, in a release build, from the no-network sandbox and on Windows and macOS through a `redteam/` CI branch. It found three problems, all Low, and nothing it tried got a name past the look-alike checks or a verdict away from Git's:

- **RT14-1.** A planted save of about 1 KiB holds 8.4 million entries by path through shared listings, under the 10,000,000 a save may have. Decision 130 never refuses a save within the limits, so the commands walk all of it: `status` took 17 s and 1,446 MiB, `changes --save` 26 s, `show` 16 s. `bolt verify` reports nothing, since the save is within the limits.
- **RT14-2.** Where a boolean is expected, Git refuses a value of exactly -2^31 (`-2147483648`, `-2g`, `-2097152k`) and Bolt read it as true, so Bolt imported a repository Git won't open.
- **RT14-3.** A `.gitignore` under 1 MiB made of many pattern lines (brackets, `**`) made `status` 4 to 13 times slower than Git's on the same files: the matcher made new working space for every line it tried on every path.

## Options

For RT14-1:
- **A. A second limit:** refuse a save whose size by path is many times what its listings hold. Every honest project with many identical folders (empty ones, ones holding the same file) would sit somewhere on that scale, and any number picked refuses some of them; it is a change to what a valid save is, which format 1, about to freeze, would carry for good.
- **B. Leave it as a known cost.** A save within the limits costs what its size by path costs, honest or planted: the format allows 10,000,000 entries, and a reader that walks them takes the time and memory they take. Decision 109 already moved "the memory a hostile save inside the limits can cost a reader" to before networking (v0.3), with the memory review.

## Decision

- **RT14-1: B.** It is the cost of a save as big as the format allows, and it is the row decision 109 moved to before networking; the red team's numbers are added to it. Nothing is written or lost.
- **RT14-2: fixed.** A value Git refuses as a boolean is refused by Bolt, as the reader's other refusals are.
- **RT14-3: fixed.** The matcher keeps its working space for the whole walk, so a line tried on a path costs no new memory.

## Consequences

- The proofs of RT14-2 and RT14-3 join the tests; RT14-1's stays switched off, as a record of the cost.
- The follow-ups row on hostile saves inside the limits gains RT14-1's numbers; [security.md](../security.md)'s row on the deep tree says what decision 130 refuses and what it leaves.
