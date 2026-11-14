# 161 — What building milestone 7 refined

- **Status:** Accepted
- **Date:** 2026-11-14
- **Agenda topic:** none (milestone 7 build)
- **Decided by:** Claude, at the owner's request while the owner was away. The owner can reverse any part.
- **Refines:** [decision 158](158-making-renaming-and-deleting-lines.md) (lines made, renamed and deleted); [decision 159](159-tags.md) (tags); [decision 160](160-whether-a-save-is-in-anothers-history.md) (the walk through every parent); [decision 116](116-gits-words-as-shortcuts.md) (Git's words)

## Context

Decisions 158 to 160 were made before milestone 7 was built. Building it, and the review of each task, settled details those records left open and corrected two things they got wrong. This record collects them, as [decision 081](081-what-building-milestone-4-refined.md) did for milestone 4. Each was written down in the build's ledger when it was made, with its reason; the rulings file ([plans/2026-11-14-milestone-7-rulings.md](../plans/2026-11-14-milestone-7-rulings.md)) has them word for word.

## Decision

**The walk (decision 160).** The record's words stand; three details were found while building it.

- **The answer is 0 as soon as the save asked about is reached from the other side.** Something else holds it, so it has nothing of its own. This is what keeps "0 exactly when held" true whatever the saves' clocks say, and it makes the commonest question (a line already combined into another) cost a handful of reads.
- **Saves dated alike are counted right.** Git keeps whole seconds, and a series of commits made again by one command shares a second, so a save and its parent with the same time are common in an imported history. The walk therefore also goes on while a shared save it hasn't looked at could still sit above a counted one, which is whenever that shared save isn't older than the earliest save counted. The promise is as decision 160 words it: the count is exact when no save is older than its parent, equal times included.
- **What it costs.** The walk reads the saves the two sides don't share, plus the shared history back to where the oldest line that only the others have parted from it. A long-lived line combined in late makes that a long way back. Git's own search for where two branches parted has the same cost when it has no index of depths. Decision 160 already says such an index comes only if measurements ask for it; the walk is measured on Git's own history at the end of this milestone.

**Names (decisions 158 and 159).**

- **A new line's or tag's name can't start with `-`.** Such a name reads as an option, to Opalro and to every script, forever after. Git refuses such branch names too. The rule is for names made here; a name that arrives another way (a Git import) still loads, and can be listed, renamed, moved and deleted (after `--`, as any option-like word).

**Lines (decision 158).**

- **Who holds a line's saves:** every other line's newest save, every tag's save, and every server line's save. Where the project stands is not a holder, and shelves aren't either, so a line whose newest save you are standing at, off the line, still asks before it is deleted.
- **`opalro tidy` records no step,** so one `opalro undo` brings a deleted line back, tidy or not, for as long as the undo window lasts.
- **`save --line` with nothing to save makes no line,** as before; its message now names `opalro line new`, which makes one without saving. An amend can't start a line: the engine refuses the pair, not only the command line.
- **`opalro help line` lists the three forms,** each with its own options. An option of one form isn't shown as an option of `opalro line` itself.

**Tags (decision 159).**

- **A message given for a tag that is already at that save is left as it was,** and Opalro says so. Changing a tag's message means deleting the tag and making it again.
- **A tag list that can't be read is damage, said by name:** "the project's tag list can't be read", in the words every damage error uses. `history` and `show` stop at it, as `goto` and the others do: the tag list is the project's own data, and `opalro verify` is the command for damage.
- **Git's `tag <name> <commit>` is not Opalro's form.** A second word is the tag's message; another save is tagged with `--save`. The guide says so.

**Undo.** `opalro undo` names a save, and says that its changes show as unsaved, only for a step that made one. For `line new`, `line rename`, `line delete` and `tag` it says what came back.

**Git's words (decision 116).** The first-use hints for `checkout`, `switch` and `branch` gained a second line each, since each word now runs more than one Opalro command.

## Consequences

- Left for the owner, each in the follow-ups list:
  - **Short names made only of the letters k to z.** A line or tag named `x`, `try` or `tmp` can also be the start of a save's change ID, and then `goto` refuses the name as meaning both ([decision 068](068-a-goto-target-that-names-both.md)). `line new` makes such names easy. Counting text as the start of a change ID only from 4 letters on, which is what screens show, would make names of 1 to 3 letters safe.
  - **A save with a great many tags** makes one very long `history` row. No cap was added.
  - **A damaged tag list stops every command that resolves a name.** A way to go on without tags, or to drop a damaged tag list, belongs with repair, later.
  - **`opalro tag v1.0 3f9a0c2e` takes the second word as the message.** A hint when that word itself names a save would catch the Git habit.
- The tests this milestone added never give a line or tag a name that could read as the start of an ID, unless the test is about that clash.
