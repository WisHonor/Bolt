# 041 — A command may write several packs

- **Status:** Accepted
- **Date:** 2026-11-02
- **Agenda topic:** none (milestone 2 planning, question 11; answers decision 028's open question)
- **Decided by:** Claude, at the owner's request to keep going with Claude's recommendations.
- **Refines:** [decision 009](009-storage-engine.md) (packs) and [decision 028](028-red-team-m1-session-2-rulings.md) #2 (its open question)

## Context

A pack holds at most 2^24 objects ([decision 028](028-red-team-m1-session-2-rulings.md)), and [storage-format.md](../storage-format.md) §11 said each command writes one pack. A large Git import needs more objects than that, and so does a file of millions of chunks (milestone 5). Decision 009 already allowed "very large writes may be split across several packs".

A writer also holds the index of the pack it is building in memory, about 100 bytes an object, so a pack near the limit needs one to two gigabytes just for that.

## Options

- **A. Several packs per command.** Git does this with `pack.packSizeLimit` and `git fast-import --max-pack-size`; restic and borg write many packs per backup.
- **B. Raise the per-pack limit.** A planted pack's index already costs a reader 3–4 GB at 2^24 objects, so raising it makes that worse.
- **C. Refuse any command that needs more.** Imports of the largest repositories, and files of more than about 16 million chunks, could never be stored.

## Decision

**A.**

- **Filling packs.** A command fills a pack until it holds 2^20 (1,048,576) objects, moves it into the store, and starts the next one.
  - The 2^20 is a writer's choice, well under the format's 2^24, so the index a writer holds stays near 100 MB. Readers still accept up to 2^24.
- **The diary entry** goes in the command's last pack. `state/head` moves only after every pack of the command is in the store, so the command still takes effect at one moment ([decision 010](010-operation-log.md)).
- **After a crash** there may be complete packs that nothing points to.
  - A re-run reuses their objects, after checking each one as decision 028 requires.
  - Cleanup (milestone 4) removes what stays unused.

## Consequences

- [storage-format.md](../storage-format.md) §11 says a command writes one or more packs, and drops the open point.
- Large imports add many packs at once. Milestone 3's merging of small packs and lazy opening (risk 17) apply to them too.
- Tests force a split with a small test-only threshold. A crash between packs must leave `state/head` unchanged and the repository usable.
