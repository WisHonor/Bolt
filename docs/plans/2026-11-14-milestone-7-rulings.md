# Milestone 7 ("Lines and tags"): rulings

The owner asked for v0.2 to be started while they were away, with Claude working alone. This file collects the calls made while planning and building milestone 7. It has three parts:

- **Please confirm:** the calls that most change what Opalro does. Each can be reversed.
- **Every ruling, as made:** each ruling as it was written in the build's ledger when it was made, task by task, with its reason and what it costs if it is wrong.
- **Deferred:** what this milestone left for later, and where each item went.

The decisions themselves are records [157](../decisions/157-v0.2-in-four-milestones.md) to [163](../decisions/163-red-team-m7-session-2-rulings.md). Each says "Decided by Claude, at the owner's request while the owner was away. The owner can reverse any part."

**Branch:** `milestone-7`, from `v0.2`. Nothing was merged into `main`, which stays what v0.1 is released from.

## Please confirm

1. **v0.2 is four milestones, built on a branch of its own** ([157](../decisions/157-v0.2-in-four-milestones.md)): lines and tags, then combining where nothing conflicts, then conflicts, then `who` and the release. `main` isn't touched until you say so. Left out of v0.2: moving saves between lines (Git's rebase and cherry-pick), guessing that a changed file was also moved, merge programs, and a combined save where moving the line forward would do.
2. **`opalro line new <name>` moves you onto the new line** ([158](../decisions/158-making-renaming-and-deleting-lines.md)). Git's `branch <name>` stays where it is; that is the classic way to save on the wrong branch, so Opalro doesn't copy it, and doesn't run `branch <name>` as a shortcut either.
3. **`opalro line delete` asks only when nothing else holds the line's saves** (158, [161](../decisions/161-what-building-milestone-7-refined.md)). "Holds" means another line, a tag or a server line has the line's newest save in its history. Where you stand doesn't count, and shelves don't. Without a terminal it stops and names `--yes`. Undo brings the line back for the 30 days of the undo window.
4. **Tags** ([159](../decisions/159-tags.md)): a new tag's name follows the line-name rules; another save is tagged with `--save <save>`, not a second bare word; moving and deleting a tag ask first; a tag's name works wherever a save is asked for, after lines and server lines and before IDs.
5. **A new line's or tag's name can't start with `-`** (161). It would read as an option ever after. Names that came in from Git still work.
6. **A message typed for a tag that is already there is left as it was** (161), and Opalro says so. Changing a tag's message means deleting it and making it again.
7. **When the tag list can't be read, the commands that need it stop** (161), `history` and `show` included, with the damage message every other damage has. The alternative was to go on without tags and warn.
8. **How Opalro tells whether a save is in another's history** ([160](../decisions/160-whether-a-save-is-in-anothers-history.md), 161): one walk, no index. Whether a line is held is exact whatever the saves' clocks say. The count shown in `line delete`'s question is exact unless some save is older than its parent, and then it can only be too high. The walk can read a long way back when a long-lived line was combined in late; an index of depths would fix that and is left until measurements ask for it.
9. **Not changed, for you to weigh:** a line or tag named only with the letters k to z (`x`, `try`, `tmp`) can also be the start of a save's change ID, and then `goto` refuses the name as meaning both ([decision 068](../decisions/068-a-goto-target-that-names-both.md)). `line new` makes such names easy now. Counting text as the start of a change ID only from 4 letters on would make short names safe.
10. **A name that is both a line and a tag, at two saves, is refused** ([162](../decisions/162-red-team-m7-rulings.md)), in `goto`, `show`, `changes --save` and `tag --save`, naming both saves. Only a Git import (a branch and a tag both called `v1`) or a planted project brings such a pair. v0.1 took the line without a word. The way out is to rename the line. The alternatives were to take the line and warn, or to take the tag as Git mostly does.
11. **The speed gate for `status` is at its limit on Linux, with `main`'s own code too** ([benchmarks.md](../benchmarks.md) §11). Opalro's `status` takes what it took; Git's got about a millisecond quicker on the test machines after their image changed on 2026-11-11, so the ratio there is now 1.84 to 2.15 against your limit of 2. Milestone 7 changed nothing `status` runs. I left the limit alone: it is yours (decisions 128 and 132). `main`'s nightly run can now fail the same way. The choices: speed `status` up (I would do this before v0.2 is released), judge the gate on several runs instead of one, or fix the Git version the gate compares with.
12. **Two findings of the second red-team session are in v0.1's draft too** ([163](../decisions/163-red-team-m7-session-2-rulings.md)), because they are older than milestone 7. After `opalro show` of a save on another line, the next step it suggests fails. And when a shelf can't be brought back, the hint puts the shelf's stored label inside a command, so a planted project could make a hint that runs something else if pasted. Neither loses data. Both are fixed on `v0.2`. Say if you want them in v0.1 before you publish it.

## Every ruling, as made

**Before the build (the plan's own check)**

1. `LineError::Kept` doesn't say whether the question was asked. The command line's confirm closure notes whether it asked: asked and answered no prints `Nothing was deleted.` (exit 0); not asked is the error naming `--yes`. Cost if wrong: one more field on `Kept`.
2. The plan's test for `tidy` said "undo twice (the tidy step, then the delete)" without checking that `tidy` records a step. The implementer undoes as many steps as it takes. (It takes one: `tidy` records none.)
3. The plan's expected undo lines assumed an order of their parts. The parts come in the order the code gives them; the test pins what is printed once the wrongly named save is gone.

**Task 1: the ancestry walk**

4. The implementer planted a mistake (each save's marks spread only once) and the plan's property test still passed. Keep its pinned test and add clocks that run backwards to the property test. The plan's generator was too weak to catch the one subtle mistake the walk can make.
5. That still caught the mistake in few cases, as the implementer measured and explained: in about half the cases the generator now puts the tip under a save of the others.
6. The first review found that the walk kept reading after the answer was settled when the tip is held. The finding was right and the plan's wording was the defect: the answer is 0 as soon as the tip gains the others' mark, and shared saves spread too while a one-mark save is waiting. Cost if wrong: one more round.
7. With the early answer, the second spreading seemed to guard nothing decision 160 promises, so each save was made to spread once. **Wrong, and reversed by the next ruling.**
8. The implementer's probe showed saves dated alike break the count, and that with one spreading a tie where two lines parted sends the tip's mark alone to the project's first save. The second spreading came back, with one more stop clause: the walk also goes on while the newest waiting save isn't earlier than the earliest counted one. The property test gained a clock of ties with exact equality. The implementer's simulation of 300,000 histories found no counterexample, and the re-review re-derived the proof against the code.
9. The re-review noted that a save only the others have keeps the walk going whatever its date, so a long-lived line combined in late makes the walk read back to where it parted. Accepted for now: it is the price of "0 exactly when held, whatever the clocks", Git's own search has the same cost without its index, and decision 160 leaves the index until measurements ask for it.

**Task 2: lines, in the engine**

10. `delete_line`'s holders are other lines' newest saves, tags' saves and server lines' saves; not where the project stands, not shelves.
11. The review found that a refused line step after a stopped switch lost the warnings about what finishing that switch left unwritten. Fixed: they reach the caller on every path.

**Task 3: `save --line` from a line**

12. Three small points from the review went to the implementer at once: an amend that names a new line is refused by the engine too ("An amend replaces the newest save of your line, so it can't start a new line."); the JSON spec's stale sentence; a missing test.

**Task 4: the line commands**

13. `opalro help line` listed `--yes` as an option of `opalro line`, which refuses it. A form's option is shown with its form; the page has a `Forms` section, and a test holds it.

**Task 5: undo's words**

14. The review doubted what `undo` prints after a save that started a line. Settled by a test, not by argument: the code names the save and says its changes show as unsaved, which is right.

**Task 6: tags, in the engine**

15. A refused tag name said "a line name can only hold…". It now speaks of a tag's name.

**Task 7: `opalro tag`**

16. The second, read-only look at the project that words a question is opened only when a question is really asked, under the lock, so it can't show an older view.
17. The `Moved` line names the old place by its ID alone.
18. A message given for a tag already at that save is left as it was (decision 159: nothing to do), and the command says so: `Its message was left as it is.` A silently dropped message is unfriendly. Cost if wrong: one line to remove.
19. A test failed at random: its line was named `x`, and one save in sixteen has a change ID starting with `x`, which makes `goto x` refuse the name. The milestone's tests were given names that can't read as the start of an ID. The rule itself (decision 068) wasn't changed: see "Please confirm" 9.

**Task 9: tags in `history` and `show`**

20. With a tag list that can't be read, `history` and `show` stop before printing anything, as `goto` and the rest already do through the resolver. Cost if wrong: with a damaged tag list no read-only view of the saves is left.
21. No cap on how many tags a row shows; the cost is bounded by the tag list itself.
22. The error for an unreadable tag list was `object … is not in the store`, with no hint. It is now told as damage, by name, from one place (`Repository::tag_list`); the hint every damage error shares stays as v0.1 has it.
23. `undo --to` then said the cause twice. It reads the tag list record itself at that place, so its line is as it was before this milestone.

**Task 10: Git's words**

24. `opalro checkout -b -` made a line named `-`. A new line's or tag's name can't start with `-`; the stored rules and imported names are untouched. Cost if wrong: one rule to drop.
25. The first-use hints for `checkout`, `switch` and `branch` gained a second line, since each word now runs more than one Opalro command.

**Task 12: the walk against Git**

26. Every test pins how many pairs of tips it compared, and the nightly one fails if it compared none. The nightly clone of Git's own repository is left as cloned: the tips come from server lines as well as lines. A small generated history with a commit dated before its parent runs the weaker form of the check on every push.

**The red team's first session (RT17), its fixes, and the speed gate**

27. RT17-1: `goto`, `show`, `changes --save` and `tag --save` read a name through one function, in decision 159's order: a line, a server line, a tag, a change ID, a fingerprint. Cost if wrong: `show` and `changes --save` take `line@server`, which v0.1 refused as unknown.
28. RT17-2: a name that is a line or a server line and also a tag at another save is refused, naming both saves by 12 hex digits; at one save the line is taken. Neither may pass for the other, as decision 068 says of a name that is also an ID; a warning would leave a script with a save nobody chose. Cost if wrong: an import with a branch and a tag of one name must rename the line before `goto <name>` works. ("Please confirm" 10.)
29. RT17-3: when `line delete` can't read the save a tag, a line or a server line names, it stops, deletes nothing, and names that holder as damage. Leaving the holder out could let `tidy` free saves that a damaged project's tag did hold. Cost if wrong: one planted tag blocks `line delete` until the tag is deleted, which the message now makes findable.
30. RT17-4: the hint after `tag --delete` carries no stored name: `To delete it, run the same command with --yes.` No way of quoting is right in cmd, PowerShell and sh alike. Cost if wrong: one less hint that can be pasted.
31. RT17-5: one function writes a stored tag's name as text: as it is when it fits the line-name rules, else between double quotes with `"` and `\` marked. Format 1 is frozen, so such names can't be refused. Cost if wrong: imported tags like `v1.0+build` show in quotes.
32. The fixing agent rewrote the red team's proof for RT17-1, because its scenario (a server line and a tag of one name at two saves) is the very clash ruling 28 refuses. The proof now checks that `show` and `goto` refuse alike, and the plain case (`show main@origin` with no clashing tag) has a test of its own. The reviewer agreed it would have failed before the fix.
33. The review of the fixes found nothing serious and five small points (a check that could never fail, three missing tests, one list built twice). They went into one more commit, which was not reviewed by itself: the second red-team session and the whole-branch review both read it. Cost if wrong: a slip in that commit waits one step to be seen.
34. Since the fixes, reading a line's name also reads the tag list, so a tag list that can't be read stops `goto <line>` too. That follows from ruling 28 and from "Please confirm" 7, and a test pins it.
35. The nightly run on milestone 7 failed one job: Linux's speed gate, `status` at 2.029 times Git's time. The red gate doesn't hold milestone 7 back from `v0.2`: on one processor type Opalro's time is the same with and without milestone 7 (13.1 to 13.9 ms against 13.3 to 13.4 ms), and milestone 7 changed nothing `status` runs. Cost if wrong: a real slowdown under a millisecond would hide in this noise. ("Please confirm" 11.)

**The red team's second session (RT18) and its fixes**

36. RT18-1: a save is found by the start of its fingerprint among the saves that lines, server lines and tags reach along first parents (before: lines only), so the IDs the refusal of ruling 28 shows are ones Opalro takes; and the refusal's hints also name the way out (rename the line, or delete the tag). Cost if wrong: a wider search for hex text, and a line named with 8 hex digits is checked against more saves.
37. RT18-2: when the walk can't read a save behind a holder's own, `line delete` makes one more pass to find the first line, tag or server line whose history holds it, and names it; the deleted line's own history is named when no holder's does. Cost if wrong: about 25 lines on a path only damage reaches.
38. RT18-3: inside a quoted tag name, `[` and `]` are marked too, so no line's mark can appear inside one.
39. RT18-4: a save's ID inside a hint's command is its short change ID when the save is on the line the project is on, else 12 hex digits of its fingerprint, decided by asking the resolver; a hint whose ID wouldn't work is left out.
40. RT18-5: `goto` advises `opalro goto <line>` only when that would go onto the line; otherwise it points at `line rename`.
41. RT18-6: one rule for every hint: text from the project goes inside a command only when it is plain (a line name's characters, not starting with `-`); anything else is left out or shown as a placeholder. This covers names, labels and paths.
42. The tag list is read once for one name, not twice (the report's suspicion). A tag list near the largest a record can be still costs about half a gigabyte to read; that is on the follow-ups.
43. No third red-team session: the fixes are small, each has its proof test, the whole-branch review reads them, and milestone 8's session attacks names and hints again. Cost if wrong: a flaw in these fixes waits for that session.

## Deferred

Each of these is a row in the [follow-ups](2026-11-01-milestone-1-follow-ups.md), under "Deferred by milestone 7".

- Short names made only of the letters k to z, against the start of a change ID (for the owner; "Please confirm" 9).
- A save with a great many tags makes one very long `history` row.
- A damaged tag list stops every command that resolves a name; going on without tags, or dropping a damaged tag list, belongs with repair.
- `opalro tag v1.0 3f9a0c2e` takes the second word as the message; a hint when that word names a save would catch the Git habit.
- An index of how deep each save is, if the walk's measured cost asks for it.
- The import doesn't say, when it runs, that a branch and a tag share a name; the refusal says it on first use.
- The questions before moving or deleting a tag don't count the saves only that tag holds, as `line delete`'s question does (for the owner).
- `status` needs room under its limit again on Linux: a speed pass before v0.2 is released ("Please confirm" 11).
- The walk on Git's own history was timed in all (17.9 s for 64 questions), not question by question, and not against Git: both before milestone 8 builds on it.
- A tag list near the largest a record can be costs about half a gigabyte to read for one name; looking a name up needs only the names.
- The damage message for a tag list that can't be read doesn't say that `opalro undo` may bring the project back.
- A line whose own history can't be read can't be deleted; that belongs with repair.
- A save's message can end in text that looks like a line's mark in `history`; with colour on, the real mark is painted and the message isn't (for the owner).
- The second session checked three attacks by reading the code only: two commands at once, tidying after a deleted line once the undo window has passed, and a planted diary step that also carries a folder change.
- Smaller points the reviews left: a stored name that starts with `-` needs `--` on the command line, which the guide doesn't show; the tag list is stored again when an undo brings back a list the store already holds; `opalro tag --yes` with no name quietly lists; the model test has no server lines.
