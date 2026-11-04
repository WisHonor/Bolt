# 019 — Clarifications from the design review

- **Status:** Accepted (Claude's calls; the owner raised no objections)
- **Date:** 2026-10-31
- **Agenda topic:** 18
- **Refined by:** [decision 031](031-own-git-reader.md) (#6: how import reads Git); [decision 032](032-exact-repeatable-commit-import.md) (#6–7: imported commits are kept exactly, with change IDs from the commit); [decision 033](033-git-branches-to-lines.md) (#6: which branches become lines, and how names are renamed); [decision 035](035-git-names-bolt-cant-take.md) (#7: other names in Git trees that Bolt can't take as they are); [decision 039](039-first-save-after-import.md) (#2: built in milestone 2, with Git's plain-file links); [decision 040](040-import-once-catch-up-later.md) (#6: one-time import confirmed for milestone 2); [decision 043](043-red-team-m2-rulings.md) (a save leaves out every name Git treats as `.git`, files included)

## Context

A fresh reviewer read every design document as if it had to build Bolt from them, and reported open questions and three contradictions between documents. Questions only the owner can answer are asked separately. This record lists the smaller points Claude settled so that the documents agree with each other and with earlier decisions.

## Decision

1. **When format 1 freezes:** at the v0.1 release, not at milestone 1. Repositories made by development builds before then are disposable and are not migrated. Each milestone adds the test vectors for the records it builds.
2. **Entries this computer can't hold:** names Windows can't create, files whose names collide on this filesystem, and symlinks written as placeholders are kept in history untouched. Bolt works them out again from the current save and the OS's rules whenever needed, so no extra file is stored. `status` lists them as "kept in history, not written on this computer", and `save` never records them as deleted or changed.
3. **Names containing `\` or `:`** are valid in storage (Linux projects and Git histories can have them) and are never written out on Windows (decision 004's write-out safety). security.md T1 now says so, which resolves the contradiction between T1 and storage-format §3.
4. **Signatures:** a save's signature field must be empty unless the repository lists `feature signatures`. The signed bytes are the save encoded with an empty signature field. This is what decision 006's "no format change" means: the layout stays the same, and the feature flag tells older readers to refuse.
5. **Lines in v0.1:** as decision 012 says, saving after `goto` to a save asks for a new line name, so v0.1 can create lines, list them and switch between them. v0.2 adds combining, renaming and deleting lines. milestones.md and the roadmap now say so.
6. **Git import (milestone 2):** runs in place. `bolt import` inside a Git project creates `.bolt/` next to `.git/` and never changes the Git repository; another folder can be chosen instead. Every branch becomes a line and every tag a tag. Each imported save records its Git commit in the extra field `git-commit`, so the commit mapping (decision 002) can always be rebuilt; the lookup table itself lives in `cache/`. Import is one-time in v0.1; re-running it to pick up new commits comes later.
7. **Git data Bolt can't hold as-is:** paths that aren't valid UTF-8 stop the import with a list of where they occur. Commit messages in another declared encoding are converted to UTF-8; invalid bytes are replaced, and the original bytes are kept in the extra field `git-original-message`. `.bolt` entries inside Git trees are skipped with a warning. The parent limit becomes 1024, because real Git histories contain merges with dozens of parents. Time-zone offsets may use the full `i16` range, because some Git commits carry unusual offsets.
8. **`.git/` is always ignored**, like `.bolt/`, so Bolt never saves Git's internals.
9. **Integrity checking from milestone 1:** the check behind `bolt verify` exists as an internal function from milestone 1, so crash tests can use it; the `bolt verify` command itself ships in milestone 6.
10. **Finding a big file's chunk list:** through an index from file ID to chunk list, kept in `cache/` and rebuilt from the chunk lists themselves, which contain the file ID.
11. **Test data for games:** candidates, still to be checked, are Unity's open-source "Open Project 1" (with Git LFS history), Godot's demo projects, and scripted series of versions of real assets. The owner's own code projects cover everyday use.

## Consequences

- storage-format.md, security.md, milestones.md and design.md are updated to match.
- If the owner disagrees with any point, it is changed before the design phase ends.
