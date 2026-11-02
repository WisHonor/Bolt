# 032 — Imported commits are kept exactly, and importing is repeatable

- **Status:** Accepted
- **Date:** 2026-11-02
- **Agenda topic:** none (milestone 2 planning, question 2)
- **Refines:** [decision 006](006-save-object.md) (change IDs of imported saves) and [decision 019](019-review-clarifications.md) #6–7 (what an imported save holds)

## Context

A Git commit holds a tree, parents, an author line, a committer line and a message, and sometimes more: a signature (`gpgsig`), a signed merge (`mergetag`), an `encoding` line, other headers, or lines in unusual forms (a `-0000` time zone, say). A Bolt save holds nearly the same fields (decision 006), but its change ID is random. Decision 002 promises export before 1.0, and principle 1 says nothing a user had is lost.

## Options

- **A. The meaningful fields only,** with random change IDs (Mercurial's `hg convert`, Fossil's importer). Signatures, signed merges, extra headers and unusual lines are lost; importing twice gives different saves; an export gives every commit a new Git ID.
- **B. Lossless and repeatable** (hg-git keeps extra data so Git IDs come back identical; Jujutsu derives change IDs from commit IDs). Keep a commit's original header whenever the save's own fields can't rebuild it, and derive change IDs from the commit.
- **C. Keep each original commit whole beside its save** (Jujutsu's and Sapling's Git backends keep the Git commit itself). Exact by construction, but every message is stored twice, and it needs a format change: an extra field holds at most 64 KiB, and cleanup would have to keep an object that only an extra field points at.

## Decision

**B.**

- **Fields.**
  - The commit's tree becomes the root listing.
  - The parents keep Git's order, up to 1,024 (decision 019).
  - The author line becomes the author and the committer line the saver. Times become milliseconds (whole seconds) and time zones minutes.
  - The message is kept as decision 019 says.
  - Imported saves get no `line` note: Git never recorded which branch a commit was made on.
- **Exactness.** A save rebuilds its commit's bytes in a standard form ([storage-format.md](../storage-format.md) §4). When that form doesn't give back the original bytes (a signature, a signed merge, an `encoding` line, other headers, lines Git wrote unusually), the save keeps the commit's original header in the extra field `git-header`. Import rebuilds every commit and checks that the bytes hash to the commit's Git ID; a mismatch is a bug and stops the import.
- **Limits.** A commit whose message is over 1 MiB, or whose header is over 64 KiB, can't be kept exactly, so it stops the import, naming the commit.
- **Old tree forms.** Some old tools wrote Git trees in a non-standard form (zero-padded modes, say). Those trees import with the right files, but a later export can't recreate their commits' Git IDs, so import reports how many commits have one.
- **Change IDs.** An imported save's change ID comes from its commit: the `change-id` header Jujutsu writes, when the commit has one that decodes to 16 bytes; otherwise Jujutsu's rule for commits without it (the last 16 bytes of the commit ID, in reverse order, each byte's bits reversed). Importing the same repository twice, on any computer, gives identical saves.

## Consequences

- [storage-format.md](../storage-format.md) §4 defines `git-header` and the standard form, and §5 how imported saves get their change IDs. Both are format-1 rules and get test vectors.
- Export (before 1.0) can give back the original Git ID of every imported commit whose trees are in the standard form.
- Re-importing (milestone 2 planning, question 9) can rely on imports being repeatable.
- Bolt spells change IDs with the letters mirrored from Jujutsu's (0 is `k` in Bolt and `z` in Jujutsu), so one ID looks different in the two tools. That is display only and can be aligned before v0.1 ([design.md](../design.md) §7, deferred).
