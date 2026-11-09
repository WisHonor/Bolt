# 095 — A Git setting that names /dev/null names no file

- **Status:** Accepted
- **Date:** 2026-11-09
- **Agenda topic:** none (milestone 5's red-team fixes)
- **Decided by:** Claude, at the owner's request while the owner was away. The owner can reverse any part.
- **Refines:** [decision 047](047-git-personal-ignore-lists-read.md) (a `core.excludesFile` Bolt can't read stops the save) and [decision 094](094-red-team-m5-rulings.md) (`GIT_CONFIG_SYSTEM` and `GIT_CONFIG_GLOBAL` follow Git's rules)
- **Refined by:** [decision 096](096-git-config-system-is-tidied-as-git-tidies-it.md) (`GIT_CONFIG_SYSTEM` is tidied as Git tidies it before it is judged; and Git documents `/dev/null` only for the two variables, not for `core.excludesFile`)

## Context

Git's documentation gives `/dev/null` as the way to switch a settings file off: `GIT_CONFIG_GLOBAL=/dev/null`, say, or `core.excludesFile = /dev/null` for no personal ignore list. Git reads it as an empty file. On Windows, Git turns the exact text `/dev/null` into `nul`, Windows' own name for it.

Bolt reads only regular files from the user's settings (decisions 047 and 060), since a pipe or device could hang it. So:
- `core.excludesFile = /dev/null` stops every save, on every system. This has been so since milestone 3, and only came to light now.
- The review of decision 094's fixes found that on Windows, Bolt joins `/dev/null` to the project's drive before it checks it. It then reads `C:\dev\null` when that exists. Any local user can make that file, and Bolt would apply settings Git ignores.

## Options

- **A. Keep stopping** on a device, as now. A user who has switched their list off can't save.
- **B. Treat `/dev/null` as naming no file,** wherever Bolt reads a path from Git's settings or Git's variables. This looks at the value as written, before it is joined to any folder. On Windows, `nul` names no file too, in any case.
- **C. Read devices as Git does.** But a pipe or another device can hang a read, which decisions 047 and 060 exist to prevent.

## Decision

**B.**
- **The values covered:** `core.excludesFile`, `GIT_CONFIG_SYSTEM`, `GIT_CONFIG_GLOBAL`, and the path of a plain include.
- **What counts as naming no file:** the exact text `/dev/null`, and on Windows also `nul`, in any case. Each means no file: no personal ignore list, no settings file, no include.
- **When it is checked:** on the value as written, before `~/` is expanded or the value is joined to any folder.
- **Anything else** that names a device or another non-regular file still stops the command, as decision 047 says.

## Consequences

- One rule in crates/bolt-vcs-core/src/config.rs, with unit tests on every system, the Windows spellings included.
- storage-format.md §12 says it.
