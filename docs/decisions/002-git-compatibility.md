# 002 — Git compatibility scope

- **Status:** Accepted
- **Date:** 2026-10-31
- **Agenda topic:** 2

## Context

Almost every project today lives in Git. Compatibility decides how easily teams can move in (bring their history), whether they can move out ("what if Bolt dies?" is the first question any team asks about a new tool), and whether Bolt users can work next to Git users. It is decided early because it sets what saves must remember: turning saves back into Git commits needs Git's details, such as author vs. committer, time zones and file permissions.

## Options

- **Import only (Git → Bolt).** A repository's whole history comes in; Git LFS files become normal Bolt big files. Simplest, and the on-ramp for teams on Git + LFS, but there is no way back. (Mercurial's `hg convert`.)
- **Import + export, as one-off conversions both ways.** Adds `bolt export`, which writes a normal Git repository: "you can always leave". More work, and exported commits get new Git IDs. (Fossil.)
- **Live two-way sync** with a Git remote, so programmers stay on Git while artists use Bolt. The best story for mixed teams and by far the hardest: exact round-trips, Git forms for Bolt-only features (change IDs, saved conflicts, locks), ongoing upkeep as Git changes. (jj, Sapling, Unity Version Control's GitSync, Diversion.)

## Decision

**Import + export, in two steps.**
- **v0.1:** import a Git repository's full history, including Git LFS files, which become normal Bolt big files.
- **Before 1.0:** `bolt export` writes a normal Git repository.
- **Live two-way sync:** not promised, and not blocked.

## Consequences

- Saves keep author and committer separately, each with a time zone (topic 6).
- The project model must hold what Git content needs: the executable bit and symlinks; submodules need an explicit import rule (topic 4).
- The importer keeps a table mapping each Git commit to its Bolt save, outside the save objects; a future live sync can build on it.
- Bolt-only things Git can't hold (saved conflicts, change IDs, lock rules, and empty folders if Bolt allows them) need export rules, settled when export is designed. So does the form Bolt big files take in Git (Git LFS or plain files).
- Import doubles as test infrastructure: differential tests import real Git histories and check that every snapshot matches (principle 3).
- Imported repositories are untrusted input (malicious paths, oversized objects), like anything else Bolt reads.
- LFS files whose content isn't available locally must import as "known but not downloaded" (topic 5).
