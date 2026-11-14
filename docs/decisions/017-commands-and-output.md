# 017 — Commands, names and output style

- **Status:** Accepted
- **Date:** 2026-10-31
- **Agenda topic:** 17
- **Refined by:** [decision 021](021-v0.1-command-scope.md) (the v0.1 command set); [decision 054](054-partial-saves.md) (partial saves name files, folders and patterns); [decision 058](058-changes-line-by-line.md) (`bolt changes` with the `similar` crate); [decision 072](072-undo-list-and-undo-to.md) (`undo --list` and `undo --to`); [decision 116](116-gits-words-as-shortcuts.md) (Git's words as shortcuts); [decision 117](117-what-json-covers-at-v0.1.md) (what `--json` covers at v0.1); [decision 120](120-the-terminal-ui.md) (the terminal UI); [decision 123](123-follow-ups-done-or-moved-past-v0.1.md) (`--debug` waits until after v0.1); [decision 158](158-making-renaming-and-deleting-lines.md) (`line new`, `line rename` and `line delete`); [decision 159](159-tags.md) (`tag`)

## Context

Commands and messages are what users actually see. Names are cheap to change before 1.0 but shape how friendly Bolt feels from day one, and many users will arrive knowing Git.

## Options

- **Bolt's words only;** Git words stop with a hint. One vocabulary, but Git users stumble at first.
- **Bolt's words, with Git's words as hidden shortcuts.** Friendly for newcomers and Git users; many commands have two names.
- **Git's words.** Nothing new for Git users, but it drops the "friendly names" goal, and familiar words would mislead because Bolt behaves differently.

## Decision

**Bolt's words, with Git's words accepted as hidden shortcuts.**
- Help and documentation show only Bolt's words. The first time a Git word is used, Bolt names its own word once. Where a Git word has no Bolt equivalent (for example `add`, since Bolt has no staging), Bolt explains instead of guessing.

**Command set** (Git equivalent in brackets):
- Starting: `start` (init), `setup`, `import <git repo>`, `export` (before 1.0)
- Every day: `status`, `changes` (diff), `save "msg" [files]` (commit), `discard <file>` (restore), `undo` / `redo`, `shelf` / `unshelf` (stash)
- History: `history` (log), `show <save>`, `who <file>` (blame), `goto <save or line>` (checkout/switch), `reverse <save>` (revert)
- Lines (v0.2): `line`, `line new`, `line rename`, `line delete` (branch), `combine <line>` (merge), `resolve <file>`, `tag`
- Team (v0.3): `copy <url>` (clone), `get` (pull), `send` (push), `lock` / `unlock` / `locks`, `login`, `pin`
- Upkeep: `verify` (fsck), `tidy` (gc). The name is `tidy`, not `clean`, because Git's `clean` deletes untracked files.

**Output and errors**
- Every error states what happened, why, and what to do next, in plain English; technical details only with `--debug`.
- Every command accepts `--json`, with a documented, versioned format, for scripts, editor plugins and the future desktop app.
- Colors and progress bars appear only in a real terminal; `NO_COLOR` is respected.
- Without a terminal, Bolt never asks questions: it stops and names the flag to use (such as `--yes`).
- Typos get suggestions, and help shows examples first.
- `bolt undo --list` numbers count steps back from now; `bolt undo 3` undoes the last three steps.

## Consequences

- When a Bolt word collides with a Git word, Bolt's meaning wins; collisions with destructive Git meanings are avoided altogether (hence `tidy`).
- The `--json` formats are a public interface: versioned and covered by snapshot tests.
- What `get` does with unsent local saves (download only, or also combine) is decided with the sync design before v0.3.
- Example messages throughout the docs are drafts until each command is built.
