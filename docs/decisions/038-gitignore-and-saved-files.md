# 038 — Bolt reads `.gitignore` too, and ignore rules keep out only what isn't saved yet

- **Status:** Accepted
- **Date:** 2026-11-02
- **Refined by:** [decision 043](043-red-team-m2-rulings.md) (a save warns when it keeps a change to a file an ignore rule matches); [decision 046](046-nested-projects-left-out.md) (a folder that holds a repository or project of its own is left out as if ignored); [decision 107](107-a-saved-files-ignore-rules-are-read-only-when-it-changed.md) (a file the last save holds is matched against the rules only when it changed; the warnings are the same)
- **Partly superseded by:** [decision 047](047-git-personal-ignore-lists-read.md) (Git's personal lists are read too)
- **Agenda topic:** none (milestone 2 planning, question 8)
- **Partly supersedes:** [decision 008](008-layout-and-settings.md) ("Git import copies an existing `.gitignore` into it")
- **Refines:** [decision 003](003-working-copy-model.md) (which files a save tracks)

## Context

Git projects list what shouldn't be saved in `.gitignore` files. Bolt reads `.boltignore` files in the same syntax, and milestone 2 makes it read them exactly as Git does. After an in-place import, Git and Bolt share the folder, and the next `bolt save` has to keep out what Git keeps out; otherwise it saves build output, caches or secrets (risk 7).

Git's ignore rules only affect files Git isn't tracking yet. Real repositories hold tracked files that match a rule, such as a DLL force-added despite a `*.dll` rule. Bolt's save applied its rules to every file, so the first save after an import would have recorded such files as deleted.

## Options

**Where the rules come from**
- **A. Copy them,** as decision 008 says: import writes a `.boltignore` beside every `.gitignore`. The copies are new files that drift apart from the originals as people keep editing `.gitignore` in Git.
- **B. Read both:** every `.gitignore` is read as if it were a `.boltignore` in its folder. Jujutsu and Sapling read `.gitignore` this way.

**Files already saved that match a rule**
- **X. As in Git, Mercurial and Jujutsu:** a rule only keeps out files that aren't in the last save.
- **Y. As Bolt did:** rules apply to every file, so a saved file that matches one is recorded as deleted.

## Decision

**B and X.**

- **Reading `.gitignore`.** A `.gitignore` is read as if it were a `.boltignore` in its folder, in every project, imported or not.
  - Its lines come first and that folder's `.boltignore` lines after, so Bolt's lines can add to Git's rules or undo them.
  - The `.boltignore` limits count both kinds of file.
  - A `.gitignore` that isn't a regular file (a link, say) is skipped with a warning, as Git skips a linked one.
- **Git's personal lists are not read.** That means `.git/info/exclude` and the user's global Git ignore file: they are personal Git settings, and the first lives inside a repository Bolt treats as untrusted.
- **Import copies nothing** and writes nothing into the folder.
- **Saved files stay saved.** A file or link that is in the save the new one builds on is saved again even when a rule matches it. It leaves history only when it is deleted from the folder.
  - A save therefore looks inside an ignored folder wherever that save holds entries, and keeps only those.
  - There is no command yet to stop saving a file while keeping it on disk (Git's `git rm --cached`); one can come later.

## Consequences

- [storage-format.md](../storage-format.md) §12 states both rules.
- Every project changes: a rule added later no longer drops files that are already saved; they are deleted once instead.
- The Git-exact matcher must arrive before Bolt reads `.gitignore` files. Otherwise a `.gitignore` line Bolt can't yet read exactly as Git does (decision 029) would stop every save after an import.
- The differential test of the matcher compares Bolt's choices with `git check-ignore` for the same `.gitignore` files, with Git's personal lists switched off.
