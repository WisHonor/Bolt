# 028 — Rulings after milestone 1's second red-team session

- **Status:** Accepted
- **Date:** 2026-11-02
- **Agenda topic:** none (follows [decision 024](024-red-team-gate.md); report: [redteam/2026-11-02-milestone-1-session-2.md](../redteam/2026-11-02-milestone-1-session-2.md))
- **Decided by:** Claude, while the owner was away, under the owner's instruction of 2026-11-01 to finish the milestone and merge. The owner can reverse any part.

## Context

The second red-team session attacked the fixes from the first. Every one of those fixes held. It found 3 high, 4 medium and 2 low problems in code the fixes hadn't touched. Some of the fixes change rules the specs state, so they are recorded here:

- **Storage (RT2-7, RT2-5).** `save` trusted an object already in the store without checking it, so a planted pack could swallow the user's content. And a pack's whole index was read into memory, sized by the file.
- **`.boltignore` (RT2-6, RT2-9, RT2-4).** Rules from nested files piled up in memory. Patterns ignored letter case nowhere, while Git ignores it on Windows and macOS. Braces meant a choice, while Git reads them literally.
- **Terminal (RT2-3).** Invisible format characters printed raw.

## Decision

1. **Verify before trusting.**
   - An object already in the store may let a save skip writing it only after Bolt has read it back and checked its fingerprint. Each object is checked once per command.
   - If the stored copy is bad, the save stores the user's own copy and warns about the damaged pack.
   - Reads try every pack that holds an object, so a lying pack can never hide a good copy.
   - A record that an object was checked is never kept in `.bolt/cache/`, because a copied project carries that folder.
2. **Pack indexes have a limit, and are read in bounded steps.**
   - A pack holds at most 2^24 (16,777,216) objects, and its index length must fit that count (storage-format.md §11).
   - Readers refuse a bigger index before allocating, and check each entry as they read it.
   - **Open question for milestone 2's planning:** what one command does when it needs more objects than one pack may hold. Examples are a file with millions of chunks (milestone 5) or a very large Git import. Writing several packs per command would change decision 009, so it needs its own decision.
3. **`.boltignore` limits** (storage-format.md §12).
   - The files in force at any folder may total at most 1 MiB.
   - One save reads at most 16 MiB of them.
   - Past either limit, the save stops and names the file.
4. **`.boltignore` matches like Git.**
   - On Windows and macOS, patterns ignore letter case, as Git does by default on those systems. On Linux they match exactly.
   - Braces are literal.
   - Bolt's matcher still differs from Git in a few edge cases: a trailing tab, an unclosed `[`, `a[/]b` and a trailing `\`. A matcher that follows Git's own rules, with a test against `git check-ignore`, is planned for milestone 2.
5. **Escaping covers every invisible format character** (Unicode general category Cf), plus the Hangul fillers. The exception is the zero-width joiner and non-joiner, which some scripts and emoji need.
6. **History is never held in memory.**
   - `history` prints as it reads.
   - Short change IDs come from a pass that keeps only IDs.
   - Looking a save up by fingerprint visits each save once.
   - After a save is committed, Bolt never reports that nothing changed; a later problem is a warning.

## Consequences

- The security threats T9, T11, T12, T19 and T21 in [security.md](../security.md) describe these defences.
- [storage-format.md](../storage-format.md) §11 and §12 state the limits and matching rules.
- Checking stored copies costs one extra read of content already stored during a save. Milestone 3's file-state cache removes it for unchanged files, and the speed bar of [decision 023](023-v0.1-speed-bar.md) is measured after that.
- [design.md](../design.md) §7 lists the pack-size question as open, for milestone 2's planning.
