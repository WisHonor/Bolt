# 116 — Git's words as shortcuts

- **Status:** Accepted
- **Date:** 2026-11-09
- **Agenda topic:** none (milestone 6 planning)
- **Decided by:** Claude, at the owner's request while the owner was away. The owner can reverse any part.
- **Refines:** [decision 017](017-commands-and-output.md) (Git's words are hidden shortcuts; the first use names Bolt's word once; a Git word with no Bolt equivalent is explained)
- **Refined by:** [decision 124](124-restore-is-explained-and-a-shortcut-takes-nothing-for-what-it-isnt.md) (`restore` is explained, never run, and each shortcut runs only the Git forms that mean the same in Bolt); [decision 158](158-making-renaming-and-deleting-lines.md) (Git's words for making, renaming and deleting a line); [decision 159](159-tags.md) (`tag` runs, and `tag -d` deletes); [decision 161](161-what-building-milestone-7-refined.md) (a second line in the hints for `checkout`, `switch` and `branch`)

## Context

Decision 017 chose Bolt's own words, with Git's accepted quietly so that people arriving from Git aren't stuck. It left open which words, what happens to the Git options people type with them (`commit -m`, `stash pop`), where "once" is remembered, and what a script sees. The diary already records Bolt's name for a command, never what was typed (decision 071).

## Options

**Git's options**
- **A. Words only.** `bolt commit -m "x"` fails on `-m`, the first thing a Git user types.
- **B. The few forms everyone types,** turned into Bolt's: `commit -m`, `stash pop`, `stash list`. Anything else Git-shaped meets Bolt's own command and its help.
- **C. Every Git option that has a Bolt meaning.** A second command line to keep in step for good.

**Where "once" is remembered**
- **A. In the project.** A second project teaches again, and a copied project carries someone else's answer.
- **B. In the user's own settings folder,** beside `config.toml`: once per person per word.

## Decision

**The few forms everyone types (B), remembered in the user's settings folder (B).**

### Words that run a Bolt command

| Typed | Runs |
|---|---|
| `init` | `start` |
| `commit` | `save` |
| `commit -m <message>`, `commit --message <message>`, `commit --message=<message>`, each also with `-a` or as `-am` | `save <message>` |
| `diff` | `changes` |
| `log` | `history` |
| `checkout <target>`, `switch <target>` | `goto <target>` |
| `restore <paths>` (at least one path) | `discard <paths>` |
| `stash`, `stash push` | `shelf` |
| `stash pop` | `unshelf` |
| `stash list` | `shelf --list` |
| `branch` (alone) | `line` |
| `fsck` | `verify` |
| `gc` | `tidy` |

`status` and `show` are the same word in both. Every other argument is passed on as typed and judged by the Bolt command, so `bolt commit --amend "x"` amends and `bolt log --oneline` is refused as `bolt history --oneline` would be. `-m` is Git's option on Git's word only: `bolt save -m` stays an error.

### Words Bolt explains instead of running

Each prints two or three lines on stderr saying what Bolt does instead, and ends with exit code 2, as any command line Bolt can't use does:

- `add`: there is no staging step; `bolt save` saves every change, new files included, and `bolt save "message" <paths>` saves some.
- `rm`, `mv`: delete or move the file itself; the next save records it.
- `reset`: `bolt undo` takes back the last command, `bolt discard` throws away unsaved changes, `bolt goto <save>` goes to an older save.
- `restore` with no path: Git refuses that, and `bolt discard` with no path throws away every unsaved change, so Bolt doesn't guess: it says to name the files (`bolt discard <paths>`), or to run `bolt discard` itself for everything.
- `checkout -b`, `switch -c`, `branch <name>`: a line starts when you save away from a line, with `bolt save --line <name>`.
- `stash apply`, `stash drop`, `stash clear`, `stash show`: `bolt unshelf` brings a shelf back, and keeping or dropping one isn't built yet.
- `merge`, `rebase`, `cherry-pick`, `tag`, `blame`: combining lines, tags and `who` arrive in v0.2.
- `clone`, `fetch`, `pull`, `push`, `remote`: syncing arrives in v0.3.
- `revert`: Bolt's `reverse` arrives in v0.3; until then `bolt undo` takes back your own last command.
- `clean`: Bolt never deletes files it doesn't keep; `bolt tidy` frees space inside `.bolt` only.

Any other word is an unknown command, with clap's suggestion when one is close.

### The hint

- The first time a person uses one of the shortcuts, Bolt runs the command and then says, on stderr: `hint: in Bolt, commit is save: bolt save "message"`. The text is fixed per word; it never repeats what was typed.
- It shows only when stderr is a terminal and `--json` isn't given, so scripts and logs never see it, and it never changes the exit code.
- **Once** is per person and per word: Bolt keeps the words it has explained in `git-words`, a small text file in the user's settings folder (one word per line). If the file can't be read or written, the hint simply shows again.

### What doesn't change

- Help, the user guide's command list and every message name only Bolt's words. The guide has one "coming from Git" table.
- The diary, `undo --list` and `--json` name the Bolt command that ran.

## Consequences

- A Git user's first `bolt commit -m "…"` works and teaches `bolt save`.
- The shortcut table is small on purpose. Git forms outside it get Bolt's own error for the Bolt command, which names that command.
- [storage-format.md](../storage-format.md) §10 describes `git-words`. It is read with a cap and never trusted: a word it lists only silences a hint.
- No shortcut runs under a word Bolt uses or plans to use (decision 017): `tag` is only explained for now, and becomes Bolt's own command when tags arrive.
- A shortcut never does more than its Git form asked: `restore` without a path is the one place where the Bolt command would, and it is explained instead.
