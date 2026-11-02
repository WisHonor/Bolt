# 033 — Git branches become lines, and the server's branches server lines

- **Status:** Accepted
- **Date:** 2026-11-02
- **Agenda topic:** none (milestone 2 planning, question 3)
- **Refines:** [decision 012](012-lines.md) (server lines exist from v0.1, filled by an import) and [decision 019](019-review-clarifications.md) #6 (which branches an import brings in)

## Context

A Git repository holds the user's own branches (`refs/heads/`), a copy of each server's branches as Git last saw them (`refs/remotes/<server>/`), tags, `HEAD`, and other refs: the stash, notes, replace refs, `git filter-branch` backups, pull-request refs. A fresh clone has one branch of its own and every branch of the server.

Bolt line names allow only letters, digits, `-`, `_`, `.` and `/`, and no two may differ only by letter case (decision 012). Git allows more, such as `+`, `#` and `@`.

## Options

**The server's branches**
- **A. Own branches only** (Mercurial's `hg convert`). Commits only the server's branches reach aren't imported; a fresh clone gives one line.
- **B. Server branches as ordinary lines,** `origin/feature-x` (Fossil's importer fed `git fast-export --all`). The user's line list grows by one line per server branch, and those lines never move.
- **C. Server branches as server lines,** `feature-x@origin`, in the part of the diary that decision 012 set aside for servers (Jujutsu does this).

**Names Bolt's rules refuse**
- **X. Stop the import and list them,** to be renamed in Git first. The server's branches can't be renamed by the user.
- **Y. Rename them by a fixed rule,** and list every renaming.

Widening decision 012's rules to Git's was left out: `@` already means "on a server" in `main@origin`, and `#` needs quoting in shells.

## Decision

**C and Y.**

- **Lines.** The user's own branches become lines.
- **Server lines.** Each server's branches become that server's lines (`feature-x@origin`).
  - The server's name is the first part after `refs/remotes/`, and the server's symbolic `HEAD` is skipped.
  - They record where Git last saw the server. Bolt can't reach a Git server, so they move only with another import.
- **Renaming.** Names Bolt's rules refuse are renamed by a fixed rule, and the import's summary lists every renaming. This covers line names, server-line names and server names. The rule:
  - each character Bolt doesn't allow becomes `-`;
  - a name over 255 bytes is shortened;
  - a name that then equals another, or differs from it only by letter case, gets `-2`, `-3` and so on, taken in the byte order of the original names.

  So the same repository always gets the same names.
- **Server names** follow the line-name rules from now on. They were unchecked while the server section stayed empty.
- **Where the user stands.**
  - Git on a branch: on that line.
  - A detached `HEAD`: at that commit's save.
  - A current branch with no commits yet: the import stops, saying to switch branches or commit first.
- **Not imported.** The stash, notes, replace refs, `refs/original/` backups, pull-request refs and every other ref outside `refs/heads/`, `refs/remotes/` and `refs/tags/` aren't imported, and the summary lists them. Commits that only those reach, or that nothing reaches, aren't imported either.
- **Limits.** A repository with no commits stops the import, pointing to `bolt start`. So does one past Bolt's limits (100,000 lines, 64 servers, 100,000 lines per server), naming the limit.

## Consequences

- [storage-format.md](../storage-format.md) §9: the server section holds the server branches an import brought in, and server names follow the line-name rules.
- v0.3 must accept a server Bolt can't reach (Git's `origin`) and let the user remove it.
- Milestone 3's `line` command lists server lines too.
- Every diary entry carries every server line, so a repository with thousands of server branches makes every entry large. Tags have the same problem; the fix is decided with the tags question (milestone 2 planning).
