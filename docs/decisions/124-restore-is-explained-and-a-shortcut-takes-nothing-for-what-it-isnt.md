# 124 — `bolt restore` is explained, and a shortcut takes no argument for what it isn't

- **Status:** Accepted
- **Date:** 2026-11-10
- **Agenda topic:** none (milestone 6 build, the review of Git's words)
- **Decided by:** Claude, at the owner's request while the owner was away. The owner can reverse any part.
- **Refines:** [decision 116](116-gits-words-as-shortcuts.md) (the `restore` row, which Git forms run a Bolt command, and the consequence "a shortcut never does more than its Git form asked")

## Context

Decision 116 lets a few Git words run the Bolt command they stand for. Its last consequence says a shortcut never does more than its Git form asked, and names `restore` without a path as the one place where the Bolt command would. The review of the code found that the rule reaches further.

- **`restore <paths>`.** Git's `restore` never touches a file Git doesn't track: `git restore .` leaves new files where they are. Bolt has no such difference: a new file is unsaved work (decision 050), so `bolt discard .` removes it too. It keeps a safety copy and `bolt undo` brings it back, but only inside the undo window. So `bolt restore .`, typed from habit, would remove files the habit expects to stay.
- **Arguments read as something else.** Arguments Bolt doesn't know are passed on as typed, and some then mean another thing:
  - `bolt commit src/a.rs` would save every change with the message "src/a.rs" (Git would commit that one file and open an editor);
  - `bolt stash create` would shelve everything under the name "create";
  - `bolt commit -m "--amend"` would amend, since the message became an option;
  - `bolt branch -d x` was told how to start a line, which is not what was asked.

## Options

- **A. Keep running `restore`, and say in its hint that new files go too.** Convenient, and nothing is lost for good, but the one Bolt command that throws work away would be reachable through a habit that expects less.
- **B. Explain `restore`, always.** One more command to type, once, and no surprise.
- **C. Run `restore` only when it reaches no new file.** It needs a way to discard changed files but keep new ones, which Bolt doesn't have.

## Decision

**B, and each shortcut runs only the forms that mean in Bolt what they mean in Git.**

- **`restore`, with or without paths, is explained, never run:** `bolt discard <paths>` throws away the unsaved changes in those paths, new files included, and `bolt undo` brings them back. The command that throws work away is reached only by its own name.
- **`commit`** runs `save` in two forms only:
  - with its message given by `-m` or `--message` (also `-am`, or with `-a`): the message becomes `save`'s message, and every other argument keeps its place, so paths still make a partial save;
  - with no argument but options: `commit`, `commit --amend`.

  `commit <something>` with no message option is explained: Git would take it for a path and open an editor, and Bolt would take it for the message.
- **A message is always passed as a message:** the translation puts `--` before it, after the options, so a message that starts with `-` is never read as an option.
- **`stash`** runs `shelf` as `stash` and `stash push`, `unshelf` as `stash pop` (with a number, `unshelf <number>`), and `shelf --list` as `stash list`. `stash` with anything else is explained.
- **`branch`** alone lists the lines. `branch <name>` is explained, as decision 116 says. `branch` with an option (`-d`, `-a`) is passed to `bolt line`, which says in its own words that it takes no such option.
- **The shortcut words are ten:** `init`, `commit`, `diff`, `log`, `checkout`, `switch`, `stash`, `branch`, `fsck`, `gc`.
- **What still differs from Git, by Bolt's design** (new files are part of the project, decision 003): `bolt commit` saves new files too, and `bolt stash` sets them aside too, to come back with `stash pop`. The hints for `commit` and `stash` say so. Neither takes anything out of reach: a save and a shelf both keep what they took.

## Consequences

- Decision 116's consequence now holds as written for everything that leaves the folder: no shortcut removes a file its Git form would have left.
- A Git user's `bolt restore file` gets two lines and then types `bolt discard file`.
- [storage-format.md](../storage-format.md) §10's list of words that `git-words` can hold loses `restore`; design.md §5.15 lists it among the explained words.
- The user guide's "coming from Git" table says what `commit` and `stash` do with new files.
