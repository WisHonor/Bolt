# 029 — Rulings after milestone 1's final review

- **Status:** Accepted
- **Date:** 2026-11-02
- **Agenda topic:** none (follows [decision 024](024-red-team-gate.md); refines [decision 028](028-red-team-m1-session-2-rulings.md))
- **Decided by:** Claude, while the owner was away, under the owner's instruction of 2026-11-01 to finish the milestone and merge. The owner can reverse any part.

## Context

Before the red-team fixes were merged, two reviews read the whole branch. They found two problems in rules that decision 028 set:

- **Checking stored copies (028 #1).** A save reads a stored copy in full before it trusts it. A planted pack can hold a copy that claims a huge size (up to 1 TiB) while taking little disk, so every save spent minutes reading it. And running out of memory on one copy stopped the read, which could hide a good copy in another pack.
- **`.boltignore` (028 #4).** Bolt's matcher still read a few kinds of line differently from Git. For most of them Bolt silently left out a file that Git keeps, which breaks principle 1, while storage-format.md said the save stopped.

## Decision

1. **Stored copies are tried cheapest first** (refines 028 #1).
   - A save knows the size of what it stores: a file's length, a link's target, an encoded listing. A stored copy that records another size can't be that object, so it is passed over without being read, and its pack is reported as damaged.
   - Reads try the copies smallest recorded size first, ties in the packs' order, so a planted copy claiming a huge size is read last.
   - Any failure of one copy, running out of memory included, moves on to the next copy. Only when no copy works does the read fail, with an error that names the problem.
2. **A `.boltignore` line that Bolt can't yet read like Git stops the save** (supersedes the last bullet of 028 #4).
   - Until milestone 2 brings a matcher that follows Git's own rules, Bolt accepts only lines in forms where it and Git agree. Any other line stops the save, and the error names the file and line and says what to change.
   - Lines are read as Git reads them. A line stops the save if it ends in a blank that Git keeps (a tab, say) or in a `\` that escapes nothing, or if it has a `[` that doesn't begin a simple class of letters, digits, `.`, `_` and ranges. On Windows and macOS, so does a capital letter after a `\` or alone in a class, where Git doesn't ignore letter case. [storage-format.md](../storage-format.md) §12 has the details.
   - The scan stops at the first `.boltignore` problem of any kind.
   - This reverses a choice made while fixing session 2's findings: an unclosed `[` was read as a plain `[`, which made Bolt leave out a file that Git keeps.

## Consequences

- [storage-format.md](../storage-format.md) §11 and §12, and threats T9, T11 and T12 in [security.md](../security.md), state these rules.
- Common patterns keep working: `*.py[cod]`, `[Bb]in/`, `*.sw[a-p]`, `[0-9]*.log`, `foo\ `, `\#notes`, `!keep.txt`. Rarer ones, such as `[!…]`, `[[:alpha:]]` or `a[/]b`, must be rewritten until milestone 2.
- A planted copy that records the right size still costs one read of that size per save, and each planted pack's index costs memory. Both are follow-ups before v0.1 ([plans/2026-11-01-milestone-1-follow-ups.md](../plans/2026-11-01-milestone-1-follow-ups.md)).
