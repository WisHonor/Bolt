# 152 — Red team on the name: one finding fixed, and more short names left to the owner before the freeze

- **Status:** Accepted
- **Date:** 2026-11-12
- **Agenda topic:** none (the name: the red team's session on the rename)
- **Decided by:** Claude, at the owner's request. The owner can reverse any part.
- **Refines:** [decision 150](150-the-name-is-opalro.md) (what the guard on the project's own folder covers)
- **Refined by:** [decision 155](155-more-short-names-of-opalro-are-refused.md) (the owner chose B)

## Context

The red team's session on the rename (RT15, [report](../redteam/2026-11-12-the-name.md)) attacked a release build in the no-network sandbox. Its Windows and macOS probes ran on GitHub's machines from a `redteam/*` branch, deleted afterwards. It tried 441,650 names, 78,289 of which a drive would take for `.opalro`, and every one was refused. Old names, old variables, old settings folders, old magic bytes and format lines, and records stored under the old context strings were all ignored, or refused as damage. The probes held on Windows and macOS: every one-character change of `.opalro` and of its short name, a planted save naming the hashed short names of `.opalro` and of a subfolder's `.opalroignore`, and another tool's `.OPALRO`.

- **RT15-1 (Low).** Two of the import's messages printed a zero-width joiner as it is, so the name read as `.opalro`: the stop for a spelling of `.opalro` that Git tracks at the top, and the warning for one below the top. The same was true of `.bolt`.
- **Two suspicions, without proof:**
  - On Linux, a drive formatted for Windows (WSL's `/mnt/c`, ntfs-3g, a FAT stick) can give `.opalro` a short name other than `OPALRO~1` to `~4`: up to `~9` on FAT, a hashed one on NTFS. Only Windows compares a name with the real short names in its folder, so on Linux such a name could reach `.opalro` through a later `goto`. Not new: `.bolt` had `BOLT~5`. What the rename changed is that `.opalroignore` starts with the same six letters, so it can take one of the four.
  - The hashed short name NTFS gives `.opalro` follows from the name, as Git's `gi7eba` does for `.gitmodules`, so a save could refuse it on every system.

## Options (for the suspicions)

- **A. Leave the rule as it is:** `.opalro`, and `opalro~1` to `opalro~4`, on every system, and on Windows the real short names in the folder.
- **B. Also refuse `opalro~5` to `opalro~9` and the NTFS hashed form, on every system.** A few more names refused, none an honest project is likely to hold. But the rule is part of format 1 ([storage-format.md](../storage-format.md) §3): once v0.1 is published, a later Opalro can't refuse a name a v0.1 save holds, so it is now or never.

## Decision

- **RT15-1: fixed.** Both messages show the joiner (`escape_all`), as the save's warning and the import's own list already did.
- **The suspicions: no finding,** since neither was shown and neither is new. B is the owner's to choose before publishing (the page for the owner, part 2, item 1); until then the rule stays as it is.

## Consequences

- RT15-1's proof runs on every push. The probes stay in `redteam15_platforms.rs`, switched off, for the next branch that needs them.
- The follow-ups and the page for the owner carry the choice between A and B.
