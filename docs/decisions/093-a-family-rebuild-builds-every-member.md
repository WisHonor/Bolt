# 093 — A family rebuild builds every member, and lets the small ones go

- **Status:** Accepted
- **Date:** 2026-11-09
- **Agenda topic:** none (milestone 5 build)
- **Decided by:** Claude, at the owner's request while the owner was away. The owner can reverse any part.
- **Refines:** [decision 092](092-a-family-rebuild-stores-only-files-big-enough-to-cut.md) (when a member under 1 MiB is built)

## Context

Decision 092 stores and maps only the members of a delta family that are at least 1 MiB. It says a smaller one is built only when something in its family is built on it. Review showed that the rebuild in fact builds every member as it walks the family, including small ones with nothing built on them. It checks each against its name, then lets go of the small ones. Not building those would mean learning each one's size first, from the start of its delta, and for the few-byte deltas a hostile pack would use, that costs as much as building it.

## Options

- **A. Learn the size of each member with nothing built on it first,** and skip the small ones.
- **B. Build every member as the walk meets it,** as the rebuild does, and store only those of 1 MiB or more.

## Decision

**B.** Every member is built and checked as the walk meets it. One under 1 MiB is let go as soon as nothing more in its family is built on it, and is never stored or mapped. Decision 092's bound holds: what is stored and mapped is unchanged, and building a small member costs its own work, within the allowance.

## Consequences

- A hostile family of tiny members costs time within the allowance, as decision 092 says.
- design.md §5.16 and security.md T11 say that a small member is built, checked and let go.
