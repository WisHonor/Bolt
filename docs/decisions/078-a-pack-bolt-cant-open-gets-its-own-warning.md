# 078 — A pack Bolt can't open for another reason gets a warning of its own

- **Status:** Accepted
- **Date:** 2026-11-07
- **Agenda topic:** none (milestone 4, the store's follow-ups)
- **Decided by:** Claude, at the owner's request while the owner was away. The owner can reverse any part.
- **Supersedes:** [decision 066](066-a-pack-bolt-cant-open-for-another-reason.md) in part (its option C is taken now: such a pack is named in a warning)

## Context

Decision 066 kept the `damaged_pack` warning for packs whose bytes are wrong, and passed over without a word a pack that can't be opened for another reason (no permission to read it, an I/O error, too many open files). It chose that because a lookup needing the pack fails with its real error anyway, and left everything else to `bolt verify` (milestone 6).

Milestone 4 brings two reasons to say more. `tidy` judges every pack and must never quietly pass one over while it frees space, and the lost-head commands read every pack's index looking for diary entries; in both, a pack that is silently skipped is a pack the user doesn't know about. And the milestone 1 follow-ups asked for it: warn for such a pack as `copy_unreadable` warns for a copy that can't be read, so a failing disk or a wrong permission is noticed even while other packs answer every lookup.

## Options

- **A. Keep decision 066:** no warning; a lookup that needs the pack fails with its error. A pack nothing happens to need stays unnoticed until `bolt verify`.
- **B. A warning of its own,** `pack_unreadable`, naming the pack and why, once per command, beside `damaged_pack` for real damage. Like Git, which names a pack it can't open (`error: packfile … cannot be accessed`) and goes on with the others.
- **C. Call it damage,** as decision 061 first read. Wrong about the cause.

## Decision

**B.** A pack Bolt can't open or search for a reason other than damage is passed over as before, its error kept, and a `pack_unreadable` warning names it and the reason, once per command. Damage keeps its own `damaged_pack` warning. The store remembers, for its lifetime, the packs that failed to open, so a pack that keeps failing is tried once per command, not once per lookup.

## Consequences

- A command answered by other packs now mentions a pack it couldn't read. `--json` documents carry the warning ([json.md](../json.md) §4).
- With a lost `head`, `bolt undo --list` and `bolt undo --to` pass over such a pack with the warning instead of stopping; a pack too damaged to open still stops them, since an entry could be in it ([decision 075](075-crash-recovery.md)).
- [storage-format.md](../storage-format.md) §11 and [security.md](../security.md) T9 describe it.
