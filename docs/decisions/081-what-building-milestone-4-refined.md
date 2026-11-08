# 081 — What building milestone 4 changed in its accepted decisions

- **Status:** Accepted
- **Date:** 2026-11-08
- **Agenda topic:** none (milestone 4's final review)
- **Decided by:** Claude, at the owner's request while the owner was away. The owner can reverse any part.
- **Refines:** [decision 070](070-undo-and-redo-step-through-the-diary.md), [decision 071](071-a-step-records-its-change-to-the-folder.md), [decision 073](073-shelves.md), [decision 074](074-save-amend.md), [decision 075](075-crash-recovery.md) and [decision 077](077-tidy-and-the-undo-window.md) (edits made to them while milestone 4 was built)

## Context

Decisions 070 to 077 were accepted with milestone 4's design, before any of it was built. Building it showed gaps and wrong details in six of them, and each was fixed by editing the record in place, the spec before the code. CLAUDE.md asks instead for a new record that refines or supersedes the old one, so the record keeps what the owner was first asked to accept. The final review found the edits. Nothing was hidden: each was ruled on in the milestone's progress log. But the records no longer show their first text, except through Git.

## Decision

The edits stand: the code, the tests and storage-format.md match them. This record lists them, so the owner can review each against the text first accepted (`git show origin/main:docs/decisions/<file>`). Each edited record gets a "Refined by" line pointing here, and no other edit.

- **070, undo and redo** (c69927d). Before: undoing a `shelf` takes its shelf off the list and undoing an `unshelf` puts it back. After: undoing any step takes off a shelf the step itself added (a `shelf`, or a jump made with `--shelve`) and puts back one it removed (an `unshelf`). Why: a jump made with `--shelve` adds a shelf too, and undoing a step should undo all it did.
- **071, a step's change to the folder.** Four edits, each found while building the record and checked by the model test:
  - 2b37de9. Before: the two saves differ only where the step wrote, and a `goto` onto unsaved work that already matched the target keeps the left save's version in its "to" save. After: the "to" save is the folder after the step at every path the switch's two saves differ, matched work included, and the "from" side there is a kept save holding that work; `unshelf`'s "from" save holds what the folder held at the shelf's paths. Why: a "to" save that didn't picture the folder after the step judged links wrongly and made redo write the wrong version.
  - 207d569. Before: `undo`, `redo` and `undo --to` record what they wrote. After: `undo` and `redo` record the folder change of the step they undo or redo, backwards for an undo; `undo --to`, when it is a step of its own, records what it wrote, as `goto`; paths the step leaves unwritten (names this computer can't hold, `.git` and `.bolt` spellings, project links) keep the base's version on both sides. Why: that step's two saves already picture the folder before and after it, at exactly its paths, so redo's "to" side is the original target.
  - 00b528c. Before: `discard` and `shelf` leave a link that leads out of the project, or any link where links can't be made. After: also one that leads through a name Bolt never writes (a `.git` or `.bolt` spelling, or a name this system can't hold). Why: write-out would bring such a link back only as a placeholder file, the case the rule exists for.
  - 690f304. Before: the two saves agree everywhere the step changed nothing, and `discard` and `shelf` go to the current save. After: inside a folder the step makes where the folder held a file, a link or nothing, the "to" save holds only what the step writes or finds there; `discard` and `shelf` go to a kept save where they make a folder the folder didn't hold. Why: a `goto` that makes again a folder the user deleted, to write into it, would otherwise give its "to" save the base's whole folder, so a redo would bring back files the user deleted.
- **073, shelves.** 2b37de9 describes `unshelf`'s folder change as 071 does; c69927d widens the shelf rule as in 070; f6e1781 makes the hint for a shelf in the way name where to go when no line reaches the save it was shelved on (`bolt goto` and its ID, or else `bolt undo --list`). Why: the same reasons as 070 and 071, and an error must say what to do next (decision 017).
- **074, `save --amend`.** f851a5f: an empty message keeps the old one, so `bolt save --amend "" <paths>` amends only those paths and keeps the message. Why: the message comes before the paths on the command line, so this was the only way to name paths and keep it. 51e2cef: a save a server line reaches through its history isn't seen by the check, joining the second-parent limit, and the comparison with Jujutsu is corrected (it refuses more: anything a remote branch, a tag or the main branch reaches). Why: walking every server line's history on each amend costs too much in a big import; nothing is lost, as with second parents.
- **075, crash recovery** (0bf667e). Before: `undo --to` after a lost head picks any entry the store holds. After: only one whose state's saves it can read; one it can't is refused, naming it. Why: making such a state current would strand the project, with no command able to read it.
- **077, `tidy`** (969b40b). Added: each pack's index is checked against its checksum before `tidy` judges it, and one that fails is left with a warning; a pack holding nothing to keep is removed first, so `tidy` frees space on a full disk; a store holding a chunk list or a conflict record stops `tidy` before it removes anything. Why: `tidy` decides what to delete from the index alone, unlike reads, which check each object; and it must not delete what a kind of record it doesn't yet follow names.

Commit c67083e, which the final review also named, edited only storage-format.md §9, not decision 071, whose text already said what it records.

From milestone 5, an accepted decision that building shows to be wrong or short is refined by a new record, written before the code, as CLAUDE.md says. Whether to allow in-place edits before a milestone merges instead, recorded once in CLAUDE.md, is for the owner to decide.

## Consequences

- Each of the six records reads as it does now, and this one says what changed and why.
- The owner reviews these refinements with milestone 4's other rulings, and can reverse any of them; a reversal means changing the code to match.
