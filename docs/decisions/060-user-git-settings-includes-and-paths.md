# 060 — Git's personal lists: includes in the user's settings, and lists inside the project

- **Status:** Accepted
- **Date:** 2026-11-05
- **Agenda topic:** none (the review of the code that builds decision 047)
- **Decided by:** Claude, at the owner's request ("do whatever you think is best", 2026-11-05). The owner can reverse any part.
- **Refines:** [decision 047](047-git-personal-ignore-lists-read.md) (Bolt reads Git's personal ignore lists; includes in the user's settings aren't followed)
- **Refined by:** [decision 085](085-importing-git-lfs-files.md) (the user's own Git settings are read for `lfs.storage` too, only to refuse) and [decision 094](094-red-team-m5-rulings.md) (the user's settings are read where Git reads them: `GIT_CONFIG_GLOBAL`, `GIT_CONFIG_SYSTEM` and `GIT_CONFIG_COUNT`)

## Context

The review of decision 047's code, run against real Git, found three gaps:

- `core.excludesFile` set in a file that `~/.gitconfig` includes (`[include] path = ~/.gitconfig.local`, a common way to keep settings across machines) was silently missed, so files Git keeps out were saved without a word.
- A relative `core.excludesFile`, or one that otherwise lands inside the project, was read following links, so a project holding a link at that path could make Bolt read a file outside it.
- Any error looking at `.git` or `.git/info` (a link, a refused permission) silently skipped `.git/info/exclude`.

## Options

For includes:
- **A. Follow plain `[include]` sections in the user's own settings, as Git does,** but not `[includeIf]`, whose conditions need the repository, its branch or its remotes.
- **B. Follow neither, as decision 047 says, and warn** whenever the settings have includes. Many people keep their work email in an `[includeIf]`, so every save would warn.
- **C. Follow both,** evaluating Git's conditions.

## Decision

**A**, and:

- **Includes.** A plain `[include] path` in the user's own Git settings is read where it appears, as Git reads it: a relative path from the including file's folder, `~/` expanded, at most 10 deep, each file at most 1 MiB, a link followed (these are the user's own files). `[includeIf]` sections aren't followed; [storage-format.md](../storage-format.md) §12 says so.
- **A list inside the project.** A `core.excludesFile` that names a path inside the project is read there as a `.gitignore` is: without following a link, and skipped with a `gitignore_skipped` warning when it is one.
- **`~user/`** paths aren't expanded: such a setting stops the save, naming it and asking for the full path.
- **`.git/info`.** A missing `.git` folder or `info` folder means no list; one that is a link is skipped with a `gitignore_skipped` warning; any other problem stops the save, as for a `.gitignore`. A `.git` that is a file (a worktree's or submodule's pointer) also means no list: Bolt doesn't follow it out of the project (decision 036), so in a Git worktree the main repository's `.git/info/exclude` doesn't apply.

## Consequences

- [storage-format.md](../storage-format.md) §12 states the rules.
- The tests cover a list set in an included file, a relative list that is a link inside the project, `~user/`, and a linked `.git/info`.
