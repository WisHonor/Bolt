# 045 — Link targets are stored with `/`, and written with `\` on Windows

- **Status:** Accepted
- **Date:** 2026-11-04
- **Agenda topic:** none (milestone 3 planning; a question milestone 2's differential tests raised)
- **Decided by:** the owner.
- **Refines:** [decision 004](004-project-contents.md) (symlinks are stored as they are)

## Context

A symlink stores its target as text: `dir/target.txt`. Linux and macOS separate its parts with `/`. Windows needs `\`: it doesn't follow a link whose target uses `/` (Perl issue 20506, Jujutsu issue 6934).

Git always stores `/`. With real links turned on (`core.symlinks=true`), Git for Windows turns each `/` into `\` when it makes a link, and each `\` back into `/` when it reads one. Bolt stores a target exactly as it reads it, so on Windows it stores `dir\target.txt`:
- the first save after an import records every link Git made as changed (the nightly job "Links Git for Windows writes keep their target when saved", run 41644016857);
- a link saved on Windows points, on Linux and macOS, at a file whose name holds a `\`.

Milestone 3 writes links out, so what history holds must be settled first.

## Options

- **A. Store `/`, write `\` on Windows** (Git for Windows; Jujutsu, whose `symlink_target_convert_to_store` converts so that "the symlink remains valid on Unix").
- **B. Store targets exactly as read**, as now. No tool that makes links on Windows does this; Mercurial doesn't make links on Windows at all.
- **C. Keep the bytes, but don't count `\` against `/` as a change** when the last save holds the target with `/`. Imports stay unchanged, but links made on Windows still break elsewhere.

## Decision

**A.**

- **Saving.** On Windows, each `\` in a link's target becomes `/` before the target is stored. On macOS and Linux a target is stored exactly as read. Nothing else changes: a drive letter, `..` or an absolute path stays as it is, as in Git.
- **Writing out** (milestone 3). On Windows, each `/` of a stored target becomes `\` when the link is made.
  - A stored target that holds a `\` (a Linux name, where `\` is an ordinary character) can't be written on Windows as meant: Windows would read it as a separator, so `..\..\x` would leave the folder. Write-out keeps such a link as it was and reports it, as it does a name Windows can't hold.
- **One place.** The conversion lives in one module, used by `save` and write-out alike.

## Consequences

- [storage-format.md](../storage-format.md) §3: a link's target is stored with `/` between its parts.
- [design.md](../design.md) §5.2 says so, and §7 no longer lists the known limitation.
- [security.md](../security.md) T3: a link whose stored target holds a `\` is never written on Windows.
- Milestone 3 builds both sides. The test `a_link_git_wrote_keeps_its_target_when_saved` then runs on Windows too, and the nightly job that gathered the evidence goes.
- A link saved on Windows before this is built holds `\`, so the next save records it as changed once. Development repositories are disposable (decision 019).
- Whether Windows can make links at all (it needs Developer Mode or administrator rights), and links to folders, are milestone 3's question about links on Windows.
