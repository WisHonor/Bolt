# 147 — The package session's follow-ups before v0.1

- **Status:** Accepted
- **Date:** 2026-11-11
- **Agenda topic:** none (the v0.1 release: the package)
- **Decided by:** Claude, at the owner's request while the owner was away. The owner can reverse any part.
- **Refines:** [decision 123](123-follow-ups-done-or-moved-past-v0.1.md) and [decision 109](109-the-speed-sessions-follow-ups-and-format-1s-three-open-rows.md) (the follow-ups due before v0.1)

## Context

**In plain words.** The follow-ups list is Bolt's to-do list of things found along the way. Before v0.1 is published, no row may still say "before v0.1" or "before the first release": each is done, or moved later with the reason written down. Two sessions prepare v0.1 at once; the engine session rules on the rows about code taken from other projects, the slow refusal of a planted deep tree and the third speed gate. These four are this session's:

1. **The license** (row "Before v0.1 (the owner's)"): choose one, add `license` to the manifests and drop the `private` exemption in `deny.toml`.
2. **`bolt.exe` on a clean Windows** (row "Before the first release"): the release program never ran on a Windows without the build tools.
3. **The walk** (row "Milestone 4, then before v0.1"): its open part, folders past the 126 held levels opened again for each folder with files, was done on 2026-11-10 ([decision 109](109-the-speed-sessions-follow-ups-and-format-1s-three-open-rows.md); the row in "Deferred by milestone 4"), but the row's label still says "before v0.1". What is left: a kept closed folder Git ignores is counted among Git's, with no effect.
4. **The store's memory of checked copies** (a "Should do" item labelled "before v0.1, with the memory review of decision 026"): decision 109 moved it past v0.1 on 2026-11-10, but its label wasn't changed.

## Options

For each row: done now, or moved past v0.1 with the reason.

## Decision

1. **The license: done** by [decision 140](140-bolt-is-closed-source.md): `LICENSE`, `license-file` in the manifests, and the README, the guide and the release notes say what is true. The `private` exemption stays, since Bolt's crates are closed and never published (decision 140 says why); the row's plan for an open-source license doesn't apply.
2. **The clean Windows: done** by the clean-machine check ([decision 146](146-the-clean-machine-check-and-which-systems-each-build-needs.md)), once the release program has passed on Windows Server Core, a Windows with no build tools, no Visual C++ runtime and no Git. A Windows 10 or 11 desktop without the build tools is still worth one run by hand; the page for the owner lists it.
3. **The walk: closed.** Its one open part is done; the count it leaves changes no file saved, ignored or reported, so it isn't worth a change to the walk the speed session tuned. It moves past v0.1 as a note, not a fix.
4. **The store's memory: moved past v0.1** by decision 109, as the row already says; its label now says so.

## Consequences

- With the engine session's rows, no row of the follow-ups says "before v0.1" or "before the first release" once both sessions are merged.
- New rows this session finds while preparing the package are written to its own section of the follow-ups, each done or moved past v0.1 with its reason.
