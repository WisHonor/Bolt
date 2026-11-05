# 043 — Rulings after milestone 2's red-team session

- **Status:** Accepted
- **Date:** 2026-11-03
- **Agenda topic:** none (follows [decision 024](024-red-team-gate.md))
- **Refines:** [decision 019](019-review-clarifications.md) (which names a save leaves out); [decision 032](032-exact-repeatable-commit-import.md) (when a commit's change-id header is used); [decision 038](038-gitignore-and-saved-files.md) (saved files that an ignore rule matches); [decision 039](039-first-save-after-import.md) (sparse checkouts set up through an include)
- **Refined by:** [decision 044](044-red-team-m2-final-rulings.md) (a change-id header equal to any commit's own ID is refused too; folders kept whole keep what the last save holds)
- **Partly superseded by:** [decision 046](046-nested-projects-left-out.md) (a folder that holds a repository or project of its own is left out whole, not saved without its `.git`)
- **Reviewed:** 2026-11-04; the owner left the call to Claude, who kept every ruling except the last sentence of RT3-5's, which decision 046 replaces.

## Context

The red-team session on milestone 2 (report: [docs/redteam/2026-11-03-milestone-2.md](../redteam/2026-11-03-milestone-2.md)) found 2 high, 4 medium and 4 low problems. Most are fixed by code alone, within decisions already made: memory and time limits in the Git reader and the importer. These need a ruling because they change a decided behaviour or are left as they are:

- **RT3-5:** `bolt save` stores names Git treats as `.git` (`.GIT`, `git~1`, `.g‌it` with an invisible character, a `.git` file), which the import leaves out and Git refuses to track. Decision 019 only skips folders named `.git`.
- **RT3-2:** each save stores its parents whole. A chain of merges with 1,024 parents each, a few hundred bytes in Git, becomes about 33 KB a save in Bolt.
- **RT3-10:** two Git commits can carry the same Jujutsu change ID, so two saves share one. Then neither can be picked by it.
- **RT3-6:** a file the repository tracks stays saved even when `.gitignore` lists it (decision 038), and `bolt save` doesn't say so. A hostile repository can track `.env` and get the user's real secrets saved.
- **RT3-7:** Bolt refuses sparse checkouts by reading `core.sparseCheckout` in `.git/config`, but Git also reads that setting from files the config includes.
- **RT3-9:** the tag list keeps a copy of every tag object's bytes for every tag (decision 034). Many tags through one big tag object fill its 512 MiB limit from a tiny repository.

## Options

- **RT3-5.** A: leave out every name Git treats as `.git` in saves too, files included. B: leave out only folders, as now, and refuse such names when milestone 3 writes files out.
- **RT3-2.** A: a limit on the parents an import may add beyond each commit's first. B: store parents in a shared record. That changes the save format, and is a bigger change than the risk needs.
- **RT3-10.** A: take a change-id header only for the first commit in import order that carries it. B: keep duplicates, and mark saves that share an ID wherever IDs are shown, as Jujutsu does.
- **RT3-6.** A: warn when a save records a change to a file an ignore rule matches. B: stop saving such files. That undoes decision 038.
- **RT3-7.** A: follow the config's includes. They can name any file on the computer, and Bolt never reads outside the project. B: refuse whenever `.git/info/sparse-checkout` exists. That file stays after sparse checkout is turned off. C: leave it, and say so.
- **RT3-9.** A: keep the format, and fix how much the import holds at once. B: store each Git tag object once and have tags refer to it. That is a format change.

## Decision

- **RT3-5: A.** A save leaves out every name Git treats as `.git` (the same check the import uses), in every folder and on every system, files included. It lists them in a warning, as the import does. The rest of a nested Git repository is saved as before.
- **RT3-2: A.** An import may add at most 1,000,000 parents beyond each commit's first. It counts them while reading the history, before writing anything, and stops past the limit. Real repositories need far fewer: Linux's history has about 80,000 merges. The disk a save's parents take is accepted: within the limit it stays under about 35 MB.
- **RT3-10: A.** A commit's change-id header is used only if no earlier commit in the import (in its fixed order) used the same one. A later commit gets the ID from its commit ID instead, and the import counts such commits in a warning. The import stays repeatable.
- **RT3-6: A.** When a save records a change to a file an ignore rule matches, it warns. The warning names the first such files and gives a count, and says Git would leave such changes out unless they were added. The warning about files that look like secrets (security.md T12) stays for later.
- **RT3-7: C.** Bolt doesn't follow includes. When `.git/config` has an `[include]` or `[includeIf]` section, the import warns that Bolt read only `.git/config` itself, so it can't tell whether sparse checkout is turned on elsewhere. If it is, the first save would record the files Git left out as deleted. Those files stay in history and can be brought back.
- **RT3-9: A.** The import encodes the tag list once at its exact size, and checks it without keeping a second copy. Storing each Git tag object once is left for a later format change.

## Consequences

- The save's walk uses the import's check for `.git` names. Milestone 3's write-out still needs its own check, since a project shared as a `.bolt` folder can carry any name.
- [design.md](../design.md) §5.16 and [security.md](../security.md) list the new limit and the new warnings. [json.md](../json.md) lists the new warning kinds.
- The tag-list format change and the sparse-checkout question join the follow-ups.
