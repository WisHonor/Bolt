# 004 — What a project can contain

- **Status:** Accepted
- **Date:** 2026-10-31
- **Agenda topic:** 4
- **Refined by:** [decision 035](035-git-names-bolt-cant-take.md) (a Git import skips dangerous names and stops at names Bolt can't hold); [decision 036](036-submodules-as-bare-links.md) (a submodule's link holds just the commit, and saves leave it alone); [decision 039](039-first-save-after-import.md) (the first save after an import stops at line endings Git converted); [decision 045](045-link-targets-stored-with-slashes.md) (link targets are stored with `/`, and written with `\` on Windows); [decision 046](046-nested-projects-left-out.md) (a folder holding a `.git` or `.bolt` of its own is left out of saves); [decision 051](051-names-this-computer-cant-write-and-length-limits.md) (names this computer can't write; names at most 765 bytes, paths 4,096); [decision 052](052-links-and-executable-bit-on-windows.md) (links and the executable bit when files are written); [decision 059](059-bolt-spellings-left-out-like-git-spellings.md) (`.bolt` spellings left out like `.git` spellings)
- **Superseded in part by:** [decision 150](150-the-name-is-opalro.md) (the names: Bolt is now Opalro, and `bolt` is now `opalro`)

## Context

These rules are baked into the storage format: what kinds of entries Bolt stores, which names are allowed, and whether contents are ever changed. Two goals pull against each other: store exactly what is there (so Git imports are exact, decision 002), and keep projects portable and safe (Windows is the tier-1 platform, decision 001, and every repository is untrusted input).

## Options

- **Like Git.** Files, executable flag, symlinks; optional line-ending conversion; no empty folders; any name the filesystem allows. Familiar and maps one to one to Git, but inherits Git's line-ending surprises, Unity's vanishing-empty-folder problem, and names that only break on Windows or macOS.
- **Faithful and portable.** Store exactly what is there, never change bytes, check names when saving, refuse dangerous names when writing files out.
- **Minimal.** Only regular files and folders. Simplest, but Git imports would lose information, contradicting decision 002.

## Decision

**Faithful and portable.**
- **Bytes in = bytes out.** Bolt never converts line endings or changes file contents by default. An opt-in per-file rule may come later if real projects need it.
- **Entry kinds:** regular file, executable file, symlink, folder (empty folders included), and a link to another project (for Git submodules; not fetched).
- **Not stored:** timestamps, Windows attributes, other permissions. On Windows, where the executable flag isn't visible, Bolt keeps what history says.
- **Name rules, always enforced:** valid UTF-8; `/` separates folders; no empty, `.` or `..` components; no absolute paths; no component that Windows or macOS would treat as `.bolt` (any letter case, trailing dots or spaces, short-name forms like `BOLT~1`).
- **Portability check on save:** new names that would break on Windows or macOS are refused: names differing only by letter case or Unicode normalization, reserved device names (`CON`, `AUX`, `NUL`, `COM1`…), trailing dots or spaces, and characters Windows forbids (`<>:"\|?*` and control characters). A project setting can relax this. Imported histories keep such names unchanged.
- **Safety on write-out:** Bolt never writes through a symlink, outside the project, or to a name the OS would misread (device names, short-name aliases, `name:stream`); it reports those instead. Symlinks pointing outside the project, or that the OS can't create, are written as placeholder files with a warning.
- **Nested projects:** a folder with its own `.bolt/` is left alone by the outer project. Full nested-project support is deferred.

## Consequences

- Git export (decision 002) must add a placeholder file for each empty folder and turn project links back into submodules.
- Teams that relied on Git's line-ending conversion set their editors instead; Bolt shows line-ending changes in diffs honestly.
- Checking out a Linux project with case-only name differences on Windows or macOS reports the colliding files instead of silently overwriting one with the other.
- Name checks and write-out safety are a core security surface: they get their own fuzz tests and malicious-repository cases (topic 18).
- The exact Windows rules (reserved names, short names, stream syntax) are verified against Microsoft's documentation when the security plan is written (topic 18).
