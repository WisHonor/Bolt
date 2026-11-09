# 089 — When Git LFS objects kept elsewhere stop an import

- **Status:** Accepted
- **Date:** 2026-11-08
- **Agenda topic:** none (milestone 5 build)
- **Decided by:** Claude, at the owner's request while the owner was away. The owner can reverse any part.
- **Refines:** [decision 085](085-importing-git-lfs-files.md) (when `lfs.storage` stops an import, and the hint for an object that doesn't match its pointer)

## Context

Decision 085 reads Git LFS files only from `.git/lfs/objects`. It refuses a repository whose `.git/config`, or the user's own Git settings, set `lfs.storage`, before writing anything: the objects are kept elsewhere, so every one would look missing, and the hint to fetch them would never help. Building it showed that the rule stops too much in two cases, misses two others, and gives one wrong hint:

- Git LFS documents `lfs.storage` for sharing one store between clones, and people set it in their own Git settings. Read literally, 085 then refuses every import on that computer, even of a repository with no Git LFS files.
- `lfs.storage` set to `lfs`, or left empty, names Git LFS's default place, `.git/lfs` (Git LFS reads a relative path inside `.git`). Nothing is kept elsewhere.
- Git also takes settings from `.git/config.worktree` when `extensions.worktreeConfig` is set. Bolt already reads that file for `core.sparseCheckout` (decision 039), but not for `lfs.storage`.
- A `.git/lfs` or `.git/lfs/objects` that is a link, a common way to keep big files on another drive, keeps the objects elsewhere too. Bolt never follows a link in `.git`, so every object looks missing: the same trap.
- For an object whose SHA-256 isn't its pointer's, 085's hint says to run `git lfs fetch --all`, then `git lfs fsck`. But fetching skips an object that is already on disk. It is `git lfs fsck` that moves a bad copy aside (to `.git/lfs/bad`), and it checks only `HEAD` unless given a commit, so it has to run first, on the commit named.

## Options

- **A. Keep 085 as written.** Simple, but a setting meant for other repositories blocks every import, and the two other ways of keeping objects elsewhere still end in a hint that can't help.
- **B. Stop only when the Git LFS files an import must bring in are kept elsewhere,** and say where in each case.

## Decision

**B.**

- **What counts as kept elsewhere:**
  - `lfs.storage` set to anything but an empty value or `lfs`. The value is taken as Git takes it: the user's own Git settings (the files and plain includes decision 060 reads), then `.git/config`, then `.git/config.worktree` when `extensions.worktreeConfig` is set, the last one found winning. Bolt doesn't resolve a path it never reads, so a value that leads back to `.git/lfs` another way still counts.
  - A `.git/lfs` or `.git/lfs/objects` that is a link, or not a folder.
- **When it stops an import:** only when the import has a Git LFS file to bring in.
  - With a sign of Git LFS, the first pass stops once it has found one, before anything is written.
  - Without a sign, the import stops at the first one the second pass meets, with nothing committed.
  - Either way, it stops in place of naming objects missing, and says where the setting is set, or which folder is a link. A repository with no Git LFS files comes in whatever the setting says.
- **The hint for an object whose SHA-256 isn't its pointer's:** run `git lfs fsck <commit>`, which moves the bad copy to `.git/lfs/bad`, then `git lfs fetch --all`, and import again.
- **Unchanged:** Bolt reads Git LFS files only from `.git/lfs/objects`, never from where `lfs.storage` or a link points, and never fetches.

## Consequences

- Setting `lfs.storage` for other repositories no longer blocks importing this one. A repository whose Git LFS objects sit behind a link is told why, instead of being sent to fetch what is already there.
- Importing a repository whose objects are kept elsewhere still needs them in `.git/lfs/objects`: the setting removed and the objects fetched again, or the link replaced by the folder. Reading them where they are waits for a real need (design.md §7, a store on another drive).
- design.md §5.16, security.md T31 and json.md describe the new stops, and tests cover each.
