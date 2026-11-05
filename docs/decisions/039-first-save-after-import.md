# 039 — The first save after an import keeps what this computer can't hold, and stops at converted line endings

- **Status:** Accepted
- **Date:** 2026-11-02
- **Refined by:** [decision 043](043-red-team-m2-rulings.md) (sparse checkouts set up through a config include: a warning, not a refusal); [decision 051](051-names-this-computer-cant-write-and-length-limits.md) (names this computer can't write; names at most 765 bytes, paths 4,096)
- **Reviewed:** 2026-11-04; the owner left the call to Claude. The line-ending stop offers its two ways on in its hint and never asks, in a terminal too: Bolt never runs Git, so a question could offer only the way that saves the CRLF files, which the hint names already. Kept.
- **Agenda topic:** none (milestone 2 planning, question 9)
- **Refines:** [decision 019](019-review-clarifications.md) #2 (entries this computer can't hold), built now rather than in milestone 3; [decision 004](004-project-contents.md) (bytes in = bytes out); [decision 031](031-own-git-reader.md) (the reader also takes `core.sparseCheckout`, only to refuse a sparse checkout)

## Context

After an in-place import the folder holds what Git last wrote there, and Bolt's history holds what Git stored. Git sometimes writes a file differently from how it stores it, or can't write it at all:
- **Names.** On Windows it can't write names such as `aux.txt` or `name.`. On a drive that ignores letter case, `README` and `readme` are one file.
- **Symlinks.** Where it can't make a link (Windows without the right permission), it writes a small file holding the link's target.
- **Line endings.** On Windows it usually turns LF line endings into CRLF as it writes files out (`core.autocrlf`, which Git for Windows turns on), while storing LF.

A save that took the folder at face value would record all of this as changes the user made.

## Options

For entries this computer can't hold there is no real alternative: decision 019 #2 already promises they stay in history untouched. For line endings:

- **A. Record files as they are.** The first save records a change to every converted file, often thousands.
- **B. Stop and explain.** The first save on top of an imported save looks for files that differ from it only in line endings, stops, and lets the user choose. Fossil's `commit` similarly stops on CR/LF line endings and asks.
- **C. Line-ending conversion in Bolt now,** as an opt-in project rule. This is like Git's `core.autocrlf` and Jujutsu's end-of-line setting. It is a sizeable feature that decision 004 deferred until real projects need it.

## Decision

**B, with decision 019 #2 built in milestone 2.**

- **Kept as they are.** A save keeps an entry of the save it builds on exactly as it is, whatever the folder holds at that path, when any of these is true:
  - its name can't exist on this system (decision 004's portability rules for this system);
  - it is one file on this drive with another entry of the same folder, because their names differ only in letter case or Unicode form; Bolt finds out by testing the drive itself, as Git does when it sets `core.ignorecase`;
  - it is a symlink, and its path now holds a regular file whose bytes are exactly the link's target, which is how Git writes a link it can't create. Milestone 3's placeholders take the same form.

  The save's summary counts these entries. A change made on this computer to such a file isn't saved; milestone 3's `status` lists them as not written here.
- **Line endings.** A save whose base is an imported save first looks for files that differ from it only in line endings: the folder's bytes equal the saved bytes with every LF turned into CRLF.
  - If there are any, the save stops and saves nothing. It says Git converted these files when it wrote them out, names a few and counts the rest.
  - It offers two ways on: have Git write the files back as stored, or save them as they are with a flag.
  - Without a terminal it names the flag.
  - Later saves don't check.
- **Sparse checkouts are refused by the import,** pointing to `git sparse-checkout disable`. There Git leaves files out of the folder on purpose, so a first save would record each as deleted. To see it, the reader also takes `core.sparseCheckout` from the repository's config (and from `config.worktree` when `extensions.worktreeConfig` is set). That key can only make Bolt refuse.
- **Uncommitted work comes along.** Changes not committed in Git, and new files Git doesn't ignore, are part of the next save.
- **Rarer conversions aren't detected.** Git can also convert files as it writes them out (`working-tree-encoding`, `ident`, custom filters). These are rare, and the first save records such files as they are.

## Consequences

- Save gets three rules: entries this computer can't hold, Git's plain-file links, and the line-ending stop. It also needs a test of the drive's letter-case and Unicode behaviour, which the Git-exact ignore matcher needs too (decision 028's follow-up).
- [design.md](../design.md) §5.2's "entries this computer can't hold" now applies from milestone 2.
- Line-ending conversion stays deferred ([design.md](../design.md) §7).
