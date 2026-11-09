# 092 — A family rebuild stores only the files big enough to be cut

- **Status:** Accepted
- **Date:** 2026-11-09
- **Agenda topic:** none (milestone 5 build)
- **Decided by:** Claude, at the owner's request while the owner was away. The owner can reverse any part.
- **Refines:** [decision 086](086-rebuilding-gits-deltas-once.md) (which objects a family rebuild stores and maps)

## Context

Decision 086 rebuilds a big file's whole delta family once. It stores and maps every object in the family, so that no later request rebuilds one. Building it showed that the map costs memory for each object it holds: about 100 to 200 bytes in the import's memo, kept until the import ends.

A family's small objects are cheap within the work allowance. A delta of a few bytes that builds a few bytes adds about 10 KB to the allowance (500 bytes for each byte inflated), and costs about 1 KiB. So a hostile pack can make one family of millions of tiny objects, up to the 16,777,216 objects a repository's packs may list. The memo would then grow to about 3 GB, and 4.5 GB for a moment while its table grows: far past the reader's own tables, which decision 086 keeps under 1 GiB.

## Options

- **A. Keep storing every member,** and state the bound: about 3 GB at 2^24 objects.
- **B. Store and map only the members big enough to be cut** (at least 1 MiB, the trigger's own test). Build the smaller ones only as bases for what is built on them, and leave them to the path every file under 1 MiB takes, through the cache. Each member stored then costs at least 1 MiB of rebuilding, so the allowance bounds how many there are.
- **C. A cap on the members stored per import.** It is an arbitrary number, and an honest import past it would fall back to the slow path without saying so.

## Decision

**B.**

- **What is stored:** a member of at least 1 MiB that passes its check is stored and mapped, as decision 086 says. A smaller one is built only when something in its family is built on it, and is let go once that is done. A commit that needs it reads it the way it reads any file under 1 MiB, through the cache, which now keeps what it builds (decision 086).
- That covers Git LFS pointers too, which are at most 1,024 bytes: the family rebuild no longer looks for them.
- **The bound:** each member stored has cost at least 1 MiB of the allowance, so a pack can make Bolt store and map at most one for each MiB of it: 4,096 at its 4 GiB floor. Past the floor, that is one for each 2 KiB or so that Bolt inflates, so the memo grows with what the repository makes Bolt read, as the rest of the import's memory does.
- **Honest histories:** Git builds a delta only from an object of similar size, never from one 32 times bigger, so a big file's family rarely holds much smaller versions. A file whose versions hover around 1 MiB reads its smaller ones through the cache, as it did before decision 086.

## Consequences

- A hostile family of tiny objects costs time within the allowance, and no memory beyond the rebuild's own.
- security.md T11 states the bound, and design.md §5.16 says which members are stored.
- The family rebuild needs no Git LFS check of its own.
