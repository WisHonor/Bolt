# 046 — A folder that holds a repository or project of its own is left out of saves

- **Status:** Accepted
- **Date:** 2026-11-04
- **Agenda topic:** none (milestone 3 planning: the owner's review of decision 043, and the owner's import check)
- **Decided by:** Claude, at the owner's request ("do what you think is best", 2026-11-04). The owner can reverse any part.
- **Supersedes:** the last sentence of [decision 043](043-red-team-m2-rulings.md)'s RT3-5 ruling ("The rest of a nested Git repository is saved as before").
- **Refines:** [decision 004](004-project-contents.md) (a folder with its own `.bolt/` is left alone) and [decision 038](038-gitignore-and-saved-files.md) (an ignored folder keeps what the last save holds)
- **Refined by:** [decision 101](101-the-walk-lists-a-folder-before-it-looks-inside.md) (the walk lists a folder before it asks it for a repository or project by name)

## Context

Decision 043 leaves every name Git treats as `.git` out of saves, files included, but saves the rest of a nested Git repository. The owner's import check (2026-11-04, [milestones.md](../milestones.md)) showed what that costs: Tailrd holds 44 Git worktrees under `.worktrees/`, each a full checkout of the project with a `.git` file, and the first save would have taken them all, about 190,000 files and 11 GB, minus their `.git` files.

Decision 004 says a folder with its own `.bolt/` is left alone by the outer project. The save's walk skips only folders named `.bolt` or `.git` themselves, so a nested project's other files are saved too.

Git never takes a nested repository's files: `git status` lists the folder as one untracked entry, and `git add` records only a link to its commit (a gitlink), warning about an embedded repository. Jujutsu leaves out every folder that holds `.git` or `.jj` ("we ignore all nested repos entirely").

## Options

- **A. Keep decision 043:** leave out the `.git` entry, save the rest.
- **B. Leave the whole folder out,** as Jujutsu does, keeping what the last save holds there as for an ignored folder, and say so.
- **C. Record a project link to the nested repository's current commit,** as `git add` does. It needs reading that `.git`, and a worktree's or submodule's `.git` file points outside the folder, often outside the project; decision 036 rules that out.

## Decision

**B.**

- **Which folders.** A folder below the top that holds an entry named `.git` (a folder, or the file a worktree or submodule has) or a folder named `.bolt` is a repository or project of its own. Where the drive ignores letter case, `.GIT` counts too, as the drive treats it as `.git`. The top folder's own `.git` and `.bolt` stay as they are (decision 019).
- **What a save does.** It leaves such a folder out, with everything in it, as if an ignore rule kept it out: what the last save holds there stays saved, as in any ignored folder (decision 038), and nothing new is taken. Nothing inside it is read but what the last save holds.
- **The warning.** A `nested_projects_left_out` warning names the first five such folders that no ignore rule keeps out, counts them, and says that a folder's files are saved once its `.git` or `.bolt` is gone, and that listing the folder in `.boltignore` leaves it out without the warning.
- **Unchanged.** A project link (a submodule) stays as decision 036 keeps it, whatever its folder holds. The import reads Git's trees, not the folder, so it doesn't change.

## Consequences

- [design.md](../design.md) §5.2, [storage-format.md](../storage-format.md) §12, [security.md](../security.md) T24 and [json.md](../json.md) say so. The save's walk changes, and so does the test that a nested repository's other files are saved.
- Milestone 3's `status` walks the folder the same way, so it shows these folders as left out.
- Someone who copies a cloned library, `.git` and all, into a project gets it left out with the warning, and saves its files by deleting its `.git`.
