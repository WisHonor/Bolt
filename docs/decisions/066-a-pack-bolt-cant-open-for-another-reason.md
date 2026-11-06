# 066 — A pack Bolt can't open for a reason other than damage

- **Status:** Accepted
- **Date:** 2026-11-06
- **Agenda topic:** none (milestone 3, the docs pass)
- **Decided by:** Claude, at the owner's request while the owner was away. The owner can reverse any part.
- **Refines:** [decision 061](061-pack-copy-order-and-lazy-opening.md) (a pack that can't be opened or searched is reported as damaged)

## Context

Decision 061 says a pack that can't be opened or searched is passed over, reported as a damaged pack, and its error kept. Bolt does that for a pack whose bytes are wrong. A pack can also fail to open for another reason: no permission to read it, a drive that gives an I/O error, too many open files. Calling that pack damaged would send the user looking for corruption that isn't there.

## Options

- **A. Report only real damage as damage.** A pack that fails for another reason is passed over the same way and its error kept, but with no `damaged_pack` warning. A lookup that finds a good copy elsewhere succeeds; one that finds nothing fails with that pack's own error (permission denied, say), naming the pack.
- **B. Report every such pack as damaged,** as 061 reads. Simple, but wrong about the cause.
- **C. A separate warning** for packs passed over for other reasons. More precise when a lookup succeeds, but a lookup that needs the pack already fails with its real error, so this adds little before `bolt verify` (milestone 6).

## Decision

**A.** Only a pack whose bytes are wrong is reported as damaged. Any other failure to open or search a pack is passed over without a warning; its error is kept, and a lookup that can't be answered without that pack fails with it.

## Consequences

- A lookup answered by another pack doesn't mention a pack that couldn't be read for a reason other than damage. `bolt verify` (milestone 6) is where every pack gets checked.
- [storage-format.md](../storage-format.md) §11 and [security.md](../security.md) T9 describe this.
