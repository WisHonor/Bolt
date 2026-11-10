# 105 — The change-ID cache keeps each line's saves in order

- **Status:** Accepted
- **Date:** 2026-11-10
- **Agenda topic:** none (milestone 6, speed and space)
- **Decided by:** Claude, at the owner's request while the owner was away. The owner can reverse any part.
- **Refines:** [decision 006](006-save-object.md) (how a change ID is shortened without reading the line) and [decision 053](053-file-state-cache.md) (the seal every cache carries); it gives `cache/change-ids` its version 2
- **Refined by:** [decision 109](109-the-speed-sessions-follow-ups-and-format-1s-three-open-rows.md) (how a save is found by its whole ID as built, and that a list is never the reason a save is shown or gone to unchecked)

## Context

`cache/change-ids` holds, for each line, the change IDs of its saves, sorted. That is enough to shorten a change ID while the project is on the line's newest save. Everywhere else Bolt still reads the whole line from the store, a save at a time:

- **at a save rather than on a line** (after `bolt goto <save>`), every command that prints the save's short change ID reads every save back from it: `status`, `goto`, `save`;
- **`bolt goto <ID>`** reads every save every line reaches to find the one the ID names, then reads the line again for the message;
- **`bolt undo --list`** reads the whole line to shorten the IDs it shows (a follow-ups row since milestone 4).

On a short history nobody notices. On Git's own repository, imported (24,497 saves on its line), GitHub's Linux runner measured `bolt goto` to the save before the newest, which rewrites one file, at 1.3 s against 13 ms for `git checkout`: 0.84 s before the lock is even taken and 0.43 s after the switch, both spent reading saves. `status` at a save took 0.52 s against Git's 0.16 s, and `undo --list` 0.56 s. On the Windows runner the same took 2.4 s, 1.9 s and 1.3 s ([benchmarks.md](../benchmarks.md) §7).

A sorted list of change IDs can't answer these: they ask which saves come before a given one, and which save a change ID belongs to.

## Options

- **A. Leave it,** and say that commands away from a line's newest save grow with the history.
- **B. Keep each line's saves in order in the cache:** for every save from the line's newest back along first parents, its ID and its change ID. Everything above becomes a scan of a list in memory.
- **C. Keep shortened IDs themselves, or an index by save.** More to keep right for the same answers.

## Decision

**B.** `cache/change-ids` goes to version 2; a version 1 file is ignored and rebuilt, like any cache Bolt can't use.

- **What it holds.** For each line it knows: the line's saves from the newest back along first parents, each as its ID and its change ID, in that order. The project's position, when it is at a save rather than on a line, is kept the same way under the empty name, which no line can have.
- **When a list is used.** A line's list is used when its first save is the line's newest save now. If the line has moved forward, Bolt reads the new saves back to the list's first and puts them in front, as it does today. If the line has moved back to a save the list holds, the saves in front of it are dropped, with no read. Otherwise the line is read once and the list replaced.
- **At a save.** The saves back from a save the project is at are the tail of any line's list that holds that save; only when no list does are they read, once, and kept.
- **What asks the list instead of the store:** the short form of a change ID, on a line or at a save, worked out exactly as reading the line works it out (the longest start it shares with the change ID of any other save in the list); `undo --list`; finding a save by the start of its change ID; and finding a save by all 64 digits of its ID, which looks in the list of the project's own position first and stops at the first save found.
- **Trust.** The file is sealed like every cache ([storage-format.md](../storage-format.md) §10), so one Bolt didn't write for this account is ignored. A list is a fact about saves that never change: the saves behind a given save are always the same. So a list that starts at the right save can't have gone stale. It never stands in for a save's contents: a command that goes to a save still reads that save, checked against its ID.
- **Size.** 48 bytes a save for each line the cache knows; the 256 MiB cap stays, and a cache that would pass it isn't written.

## Consequences

- `goto`, `status` at a save and `undo --list` no longer grow with the history once the cache knows the line; the first command after the cache is lost reads the line once.
- The short IDs shown are the ones shown today.
- A short hex prefix (fewer than 64 digits) is still looked for by reading every line, as today; only the full ID is quick.
- [storage-format.md](../storage-format.md) §10 describes version 2, with its test vector (`change_ids.json`), and the `change_ids` fuzz target reads it.
- Tests: each answer from the list equals the answer from reading the line, on random histories with amended saves and lines moved back and forward; a list that doesn't start at the line's newest save is never used as it is; a version 1 file is ignored.
