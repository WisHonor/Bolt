# 158 — Making, renaming and deleting lines

- **Status:** Accepted
- **Date:** 2026-11-14
- **Agenda topic:** none (milestone 7 planning)
- **Decided by:** Claude, at the owner's request while the owner was away. The owner can reverse any part.
- **Refines:** [decision 012](012-lines.md) (lines are movable names in the diary; renaming or deleting one is an undoable step); [decision 017](017-commands-and-output.md) (`line new`, `line rename`, `line delete`); [decision 055](055-lines-in-v0.1.md) (on a line, `save --line` was refused until v0.2); [decision 116](116-gits-words-as-shortcuts.md) (Git's words for these)
- **Refined by:** [decision 161](161-what-building-milestone-7-refined.md) (a new name can't start with `-`; who holds a line's saves; `save --line` with nothing to save; how `help line` lists its forms)

## Context

v0.1 makes a line only by saving away from one (decision 055), and can't rename or delete a line. Decision 012 fixed what a line is: a movable name kept in the diary, with rules for its name, and renaming or deleting as steps `undo` takes back. Decision 017 fixed the command words. Left open: whether making a line also moves you onto it, what renaming and deleting refuse, and whether deleting asks first.

## Options

**A new line: move onto it, or stay**

- **A. Make the line and move onto it** (Git's `switch -c`; Mercurial's `hg bookmark <name>` makes the new bookmark the active one).
- **B. Make the line and stay where you are** (Git's `branch <name>`; Jujutsu's `jj bookmark create`).

B needs a second command before the next save lands on the new line. Forgetting it puts the save on the wrong line, which is the classic Git mistake.

**Deleting a line that holds saves nothing else shows**

- **A. Ask first** in a terminal, or take `--yes` (Git refuses `branch -d` on an unmerged branch and wants `-D`).
- **B. Never ask,** since `undo` brings the line back (Mercurial deletes a bookmark without a word; `opalro discard` asks nothing for the same reason).
- **C. Refuse** until the line has been combined into another.

B is tempting, because every step can be undone. But `discard` shows at once what is gone. A deleted line's saves leave quietly, when the undo window ends 30 days later.

## Decision

**A, and A.**

- **`opalro line new <name>`** makes a line at the save the project is on and puts the project on it. The folder isn't touched, so unsaved work stays as it is and the next save goes on the new line. It works on a line and at a save.
- **`opalro save --line <name>` on a line** now works: the save is made on the new line, and the line the project was on stays where it was. It is `line new` and `save` as one step.
- **`opalro line rename <old> <new>`**, or `opalro line rename <new>` for the line the project is on. A project on the line stays on it, under the new name. Saves already made keep the old name as the line they were made on (decision 012): that note says where work happened at the time, and is never rewritten.
- **`opalro line delete <name>`** takes the name away. The saves stay stored, and `opalro undo` brings the line back for as long as the undo window lasts (decision 077).
  - Refused: the line the project is on. The hint says to go to another line first.
  - When no other line, no tag and no server line has the line's newest save in its history, deleting it leaves saves that nothing else shows. Then Opalro asks first, saying how many such saves there are. Without a terminal it stops and names `--yes`. When something else holds every save of the line, it asks nothing.
  - "In its history" means through every parent, so a line that was combined into another is held by it ([decision 160](160-whether-a-save-is-in-anothers-history.md)).
- **Names** follow decision 012's rules, checked where they already are. A new name may not be another line's in any letter case, and may not be exactly a tag's ([decision 159](159-tags.md)). Renaming a line to another spelling of its own name (`Main` to `main`) is allowed.
- **Nothing to do makes no step:** renaming a line to the name it already has.
- **Each is one diary step,** named `line new`, `line rename` or `line delete`, with no change to the folder. `undo` and `redo` need nothing new for them.
- **Git's words.** `checkout -b <name>` and `switch -c <name>` run `line new`. `branch -m` runs `line rename`. `branch -d` runs `line delete`, and `branch -D` runs it with `--yes`. `branch <name>` is still explained and never run: in Git it makes the branch and stays, while `line new` moves onto it ([decision 124](124-restore-is-explained-and-a-shortcut-takes-nothing-for-what-it-isnt.md)'s rule).

## Consequences

- `opalro undo` after a `line new` or a rename must not say that a save's changes "show as unsaved": no save was made. What `undo` prints learns the difference.
- `opalro help line` lists the three forms.
- A deleted line's saves are freed by `opalro tidy` once the step is past the undo window and nothing else holds them.
- The count in `line delete`'s question can be too high, never too low, in a history where a save is older than its parent (decision 160).
- A project can end with no line at all (at a save, its last line deleted). The next save there asks for a new line's name, as it always has.
- The model test gains the three steps, and `save --line` from a line.
