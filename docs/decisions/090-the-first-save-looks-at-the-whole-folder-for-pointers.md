# 090 — The first save after an import looks for Git LFS pointers in the whole folder

- **Status:** Accepted
- **Date:** 2026-11-09
- **Agenda topic:** none (milestone 5 build)
- **Decided by:** Claude, at the owner's request while the owner was away. The owner can reverse any part.
- **Refines:** [decision 054](054-partial-saves.md) (a partial save applies every other rule to what it names: this stop is an exception) and [decision 085](085-importing-git-lfs-files.md) (the first save's stop at Git LFS pointers: where it looks, and a way out that needs no Git)

## Context

Decision 085 makes the first save after an import stop where the folder holds a Git LFS pointer instead of the file the import brought in. That happens when Git LFS wasn't installed, or was told to skip, when Git checked the folder out. The save says to run `git lfs checkout`, then save again, and has no flag to save the pointers anyway, since saving pointer text over every big file is never what anyone means. Decision 054 lets a save name paths, and applies every other rule of a save only to what it names.

Building it showed two gaps:

- A first save that names other paths goes through, and its save becomes the base. That save carries no Git commit, so no later save looks, and the next full save records the pointer text over the big files: the very thing 085 stops. A partial amend has the same gap, since an amend drops the Git fields.
- Once `.git` or its Git LFS objects are gone (someone moved to Bolt and deleted `.git`, or ran `git lfs prune`), `git lfs checkout` can't help, and there is no flag. Yet Bolt already holds the real files.

## Options

- **A. Keep 054's rule for this stop too,** and accept that a partial first save ends the check.
- **B. Look at the whole folder on the first save after an import, partial or not, and name a way out that needs no Git.**
- **C. Remember in the repository that the pointers haven't been checked,** until some save has looked. It needs a new record in the store's format.

## Decision

**B.**

- **Where it looks:** on the first save after an import, and on an amend of the save an import made, the pointer check looks at every file in the folder of at most 1,024 bytes that the file-state cache doesn't vouch for, with 085's rules, even when the save names paths. A partial save stops at a pointer outside its paths, naming it. This one stop is an exception to 054's rule that every other rule applies to what is named. The line-ending stop of decision 039 keeps to the named paths: it records real differences, and it asks first.
- **The way out:** after `git lfs checkout`, the hint says that if Git LFS can't, `bolt discard` with those paths puts back the files the import brought in. Discard keeps what it throws away, the pointer text, as a safety copy.
- **Known limit:** the stop names five files and counts the rest. A folder checked out without Git LFS can hold thousands of pointers. `git lfs checkout` clears them all at once; `bolt discard` clears them a few paths at a time, and `bolt status` lists them all.

## Consequences

- A partial first save after an import reads, once, the small files in the whole folder that the cache doesn't vouch for. Like a full first save, it can stop at one of them that is locked or changing, outside the paths it names.
- A later speed-up that lets a partial save's walk skip what no named path reaches (design.md §7) must keep this look on the first save after an import.
- design.md §5.16 and json.md describe the stop.
