# 155 — A save also refuses `opalro~5` to `opalro~9` and NTFS's hashed short name of `.opalro`

- **Status:** Accepted
- **Date:** 2026-11-12
- **Agenda topic:** none (the v0.1 release: the final builds)
- **Decided by:** the owner
- **Refines:** [decision 152](152-red-team-rulings-on-the-name.md) (its option B, which it left to the owner)
- **Supersedes in part:** [decision 148](148-what-format-1s-spec-says-before-it-freezes.md) (its bullet on §3: the short names checked were `opalro~1` to `opalro~4`)

## Context

A name in a save is refused, on every system, when Windows or macOS would take it for the project's own `.opalro` folder ([storage-format.md](../storage-format.md) §3). The short names on the list were `opalro~1` to `opalro~4`, the ones NTFS gives first. On Windows, Opalro also compares each name with the real short names in its folder.

The red team on the name (decision 152) raised two suspicions, without proof:
- A drive formatted for Windows but written from Linux (WSL's `/mnt/c`, ntfs-3g, a FAT stick) can give `.opalro` another short name: up to `~9` on FAT, a hashed one on NTFS. On Linux, no real short names are compared, so a name like that in a save could reach `.opalro` through a later `goto` on that drive.
- When `OPALRO~1` to `~4` are taken, NTFS names a file by the long name's first two letters, four hex digits of a hash of the long name, `~` and a counter. The hash follows from the name alone, as Git's `gi7eba` does for `.gitmodules`, so a save could refuse it on every system.

The rule is part of format 1, so it can be widened only before v0.1 is published.

For `.opalro` the hash is `9975`, and the name is `OP9975~1`. The hash is the one the Wikipedia article "8.3 filename" gives. It gives Git's four known values (`gi7eba` for `.gitmodules`, `gi250a` for `.gitignore`, `gi7d29` for `.gitattributes`, `maba30` for `.mailmap`), and `GI2837~1`, the short name a Windows runner gave `.git` ([security.md](../security.md), T2). A Windows test reads `OP9975~1` off a real drive.

## Options

- **A. Leave the rule as it is:** `.opalro` and `opalro~1` to `opalro~4` on every system, and on Windows the real short names in the folder.
- **B. Also refuse `opalro~5` to `opalro~9` and the NTFS hashed form, on every system.**

## Decision

**B.** On every system, a name is also taken for `.opalro` when what the spelling rule leaves is:
- `opalro~5` to `opalro~9`; or
- NTFS's hashed short name of `.opalro`, in the form `git fsck` refuses for `.gitmodules`: eight characters, made of the start of `op9975` (all six before a one-digit counter), `~`, and a counter that starts with 1 to 9 and goes on in digits. Examples: `op9975~1`, `op997~12`, `o~123456`, `~1234567`.

Such a name is treated as every other spelling of `.opalro` already is:
- a save leaves it out, with a warning ([decision 059](059-bolt-spellings-left-out-like-git-spellings.md));
- an import treats it as it treats `.opalro` itself;
- a write-out refuses it.

## Consequences

- A project holding a file named like `opalro~5` or `op9975~1`, in any folder, has it left out of a save with a warning. An honest project is unlikely to hold one.
- On Windows, each name is still compared with the real short names in its folder as well, which covers any other short name a drive gives.
- The rule freezes with format 1 (§3).
