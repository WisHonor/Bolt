# 036 — Submodules become links that hold just the commit, and saves leave them alone

- **Status:** Accepted
- **Date:** 2026-11-02
- **Agenda topic:** none (milestone 2 planning, question 6)
- **Refines:** [decision 004](004-project-contents.md) (submodules become project links)

## Context

A Git submodule is a tree entry (mode 160000) holding the ID of a commit in another repository. Its address sits in `.gitmodules`, a Git settings file inside the project.

Decision 004 makes a submodule a project link that isn't fetched, and the project-link record has an address field ([storage-format.md](../storage-format.md) §7).

The working folder usually holds a checkout of the submodule, with its own `.git` pointer file. Today's `bolt save` would store those as ordinary files and replace the link with a folder.

## Options

- **A. The link keeps its address,** read from that commit's `.gitmodules`. Mercurial's `hg convert` makes subrepositories with their addresses. It needs a careful reader for Git's settings format, which hostile repositories have abused (CVE-2025-48384, CVE-2023-29007).
- **B. The link holds just the commit.** Jujutsu keeps submodule entries this way, without managing them. `.gitmodules` comes in as an ordinary file, so nothing is lost.
- **C. Bring the submodules' contents in as ordinary folders,** as `git subtree` gives you. It needs every submodule's history, loses the link, and makes the project's history claim files it never held.

## Decision

**B.**

- **The link.** A submodule becomes a project link with system `git`, the commit ID and an empty address.
- **`.gitmodules`.** Import never reads it. It comes in as an ordinary entry, or as a symlink if it is one.
- **Saving.** A later `bolt save` keeps every project link of the save it builds on exactly as it is. It never looks inside the link's folder, whatever is there: a checkout, an empty folder, or nothing. In particular it never reads a submodule's `.git` pointer file. This is decision 019's promise for entries this computer doesn't hold.
- **Changes.** Bolt v0.1 can't move a submodule to another commit, or add or remove one; that is done in Git.

## Consequences

- [storage-format.md](../storage-format.md) §7: an imported submodule's address is empty, and the address lives in `.gitmodules`.
- [security.md](../security.md) gets a submodule row (T24): nothing about a submodule is fetched, run or parsed.
- If Bolt ever fetches submodules, it reads `.gitmodules` from the same save at that point, with a hardened reader.
- Export puts submodules back exactly: the link becomes the submodule entry again, and `.gitmodules` is the ordinary file it was.
