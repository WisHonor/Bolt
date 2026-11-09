# 047 — Bolt reads Git's personal ignore lists too

- **Status:** Accepted
- **Date:** 2026-11-04
- **Agenda topic:** none (milestone 3 planning: a question milestone 2's final review left, and the owner's import check)
- **Decided by:** Claude, at the owner's request ("do what you think is best", 2026-11-04). The owner can reverse any part.
- **Partly supersedes:** [decision 038](038-gitignore-and-saved-files.md) ("Git's personal lists are not read")
- **Refined by:** [decision 060](060-user-git-settings-includes-and-paths.md) (plain includes in the user's settings are followed; a list inside the project is read without following links) and [decision 094](094-red-team-m5-rulings.md) (the user's settings are read where Git reads them: `GIT_CONFIG_GLOBAL`, `GIT_CONFIG_SYSTEM` and `GIT_CONFIG_COUNT`)

## Context

Bolt reads every `.gitignore` so that it keeps out what Git keeps out (decision 038). Git has two more lists, both personal: `.git/info/exclude`, for one repository, and the user's own list, the file `core.excludesFile` names, by default `~/.config/git/ignore`. Decision 038 left both out, as personal Git settings, the first inside a repository Bolt treats as untrusted. `bolt import` warns when `.git/info/exclude` has patterns; nothing mentions the user's own list.

The owner's import check (2026-11-04, [milestones.md](../milestones.md)) showed what that costs. In Tailrd, `.git/info/exclude` hides 44 Git worktrees and Claude Code's working files, and the user's own list, where Claude Code adds `**/.claude/settings.local.json`, hides that file, which the first save took without a word.

## Options

- **A. Keep them out,** and warn about the user's own list too.
- **B. Read both, as Git does.** Jujutsu does the same: its `base_ignores` reads `core.excludesFile` (or `~/.config/git/ignore`) and `.git/info/exclude`.
- **C. Read `.git/info/exclude` only.**

## Decision

**B.**

- **Where their lines go.** Both lists are read in every project, as if their lines came first in the top folder's `.gitignore`: the user's own list, then `.git/info/exclude`. The project's own `.gitignore` and `.boltignore` lines come after them and win, as in Git. They count as Git's rules wherever a save asks what Git alone would leave out (decisions 038 and 043).
- **`.git/info/exclude`** is read only where the top folder holds a real `.git` folder, through `safe_fs` like a `.gitignore`: a link or special file is skipped with a warning. It can keep out nothing a `.gitignore` couldn't.
- **The user's own list** is the file `core.excludesFile` names in the user's own Git settings (`$XDG_CONFIG_HOME/git/config`, then `~/.gitconfig`, which wins, as Git reads them; `~/` expanded; a relative path from the project's top, as in Git), else `$XDG_CONFIG_HOME/git/ignore`, which is `~/.config/git/ignore` when that variable isn't set.
  - It is the user's own file, so a link to it is followed.
  - Bolt never takes `core.excludesFile` from the repository's `.git/config`, which could name any file on the computer. Includes in the user's Git settings aren't followed, as in the repository's config.
- **Limits and problems.** Both count toward the ignore files' limits (storage-format.md §12). A list that isn't there is none; any other problem reading one stops the save and names the file, as for a `.gitignore`.
- **The import** no longer warns about `.git/info/exclude`.

## Consequences

- [storage-format.md](../storage-format.md) §12, [design.md](../design.md) §5.6 and §5.16, [security.md](../security.md) T12 and [json.md](../json.md) say so.
- What a save keeps out can now depend on the user's own Git settings, as what Git commits does.
- Tests keep Bolt from the real user's Git settings, and the differential test against Git covers both lists.
