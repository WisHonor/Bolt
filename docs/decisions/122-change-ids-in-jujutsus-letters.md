# 122 — Change IDs are written in Jujutsu's letters

- **Status:** Accepted
- **Date:** 2026-11-09
- **Agenda topic:** none (milestone 6 planning)
- **Decided by:** Claude, at the owner's request while the owner was away. The owner can reverse any part.
- **Refines:** [decision 006](006-save-object.md) (change IDs are shown in the letters k–z)
- **Supersedes in part:** [decision 032](032-exact-repeatable-commit-import.md), the sentence "Bolt spells change IDs with the letters mirrored from Jujutsu's (0 is `k` in Bolt and `z` in Jujutsu), so one ID looks different in the two tools"

## Context

A change ID is 16 bytes, shown as 32 letters from `k` to `z`, one per half-byte, so it never looks like a hex fingerprint (decision 006). Bolt writes the value 0 as `k` and 15 as `z`. Jujutsu, where the idea comes from, writes 0 as `z` and 15 as `k`.

That mattered little until imports. An imported save takes its change ID from its Git commit by Jujutsu's own rules (decision 032): the `change-id` header Jujutsu writes, or else Jujutsu's rule for a commit without one. So Bolt and Jujutsu hold the same 16 bytes for the same commit, and show them as different letters. Decision 032 said this "can be aligned before v0.1", and design.md §7 asked for the decision before v0.1.

It is display only: saves store the 16 bytes, and so do the caches. But once people have change IDs in notes, scripts and messages, the letters can't change any more.

## Options

- **A. Keep Bolt's letters.** Nothing to do. A person who uses both tools on one repository sees two names for each change, for good.
- **B. Write them as Jujutsu does.** A few lines of code and the docs' examples, now, while nobody has a change ID written down.

## Decision

**B.** Each half-byte with the value `v` is written as the letter `z` minus `v`: 0 is `z`, 15 is `k`.

- Change IDs still use only the letters `k` to `z`, still show as the shortest unique start of at least 4 letters, and are read back the same way wherever a command takes one.
- Nothing stored changes: not a save, not a fingerprint, not a cache.
- For a commit Jujutsu knows, `bolt history` and `jj log` now show the same letters (Jujutsu usually shows more of them). They still differ where decisions 043 and 044 give a save another change ID than its header's, so that no two saves share one.

## Consequences

- [storage-format.md](../storage-format.md) §5's display rule, [json.md](../json.md)'s description of `change_id`, and the examples in design.md change with the code. The examples in older decision records stay as they were written.
- The line in design.md §7 "Change-ID letters like Jujutsu's" is done.
- A development build and v0.1 show different letters for the same save. Development repositories are disposable (storage-format.md), and the bytes are the same.
