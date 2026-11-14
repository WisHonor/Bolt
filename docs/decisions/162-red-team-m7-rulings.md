# 162 — Red team on milestone 7: five findings, all fixed

- **Status:** Accepted
- **Date:** 2026-11-14
- **Agenda topic:** none (milestone 7, its red-team session, RT17)
- **Decided by:** Claude, at the owner's request while the owner was away. The owner can reverse any part.
- **Refines:** [decision 159](159-tags.md) (one order for a name in all four commands; a name that is both a line and a tag; how a tag's name is shown, and what a hint may carry); [decision 068](068-a-goto-target-that-names-both.md) (the refusal of text that names two things now covers a line and a tag); [decision 158](158-making-renaming-and-deleting-lines.md) (`line delete` names a holder it can't read)

## Context

The red team's session on milestone 7 (RT17, [report](../redteam/2026-11-14-milestone-7.md)) attacked a copy of `milestone-7` at 26e910a in the no-network sandbox. It found five things: none critical, none high, two medium, three low. None loses data and none skips a question. Each is about a name that means the wrong save, or about what a person is shown.

What held: the walk that `line delete` relies on ([decision 160](160-whether-a-save-is-in-anothers-history.md)) gave the right answer on 15,000 questions over 3,000 random histories, with clocks running backwards and times at the ends of 64 bits. Deleting a line and then tidying, amending a tagged save, Git's words and `save --line` held too.

- **RT17-1 (Medium).** Decision 159 gives one order for a name where a save is asked for: a line, a server line, a tag, an ID. Only `goto` followed it. `show`, `changes --save` and `tag --save` never looked at server lines. Git allows a tag named `main@origin`, and the import keeps it, so `show main@origin` showed the tag's save while `goto main@origin` went to the server line's.
- **RT17-2 (Medium).** Git allows a branch and a tag of one name (a release branch and a tag both named `v1`, say), and the import keeps both. `goto v1` and `show v1` took the line without a word, while `opalro tag` listed `v1` at another save. Git warns that such a name is ambiguous, and most of its commands then take the tag.
- **RT17-3 (Low).** A planted tag naming a save the project doesn't have stopped `line delete`, even with `--yes`, with `object … is not in the store`: no hint, and no word on which tag.
- **RT17-4 (Low).** Where Opalro can't ask, `tag --delete` printed a hint with the tag's name inside a command. An imported or planted tag can be named `v1; touch pwned`, and pasting that hint runs the second command.
- **RT17-5 (Low).** A planted tag named `v9]  [main` made its row in `history` read as if line `main` were there too, and a name holding `, ` read as two tags in `show`.
- **Two suspicions, without proof.** The questions before moving or deleting a tag don't say how many saves only that tag was holding, as `line delete`'s question does. And deleting a line that parted long ago reads the other lines back to that point (the cost decision 161 accepted), which nobody has timed on a large project.

## Options

**A name that is both a line and a tag, at two saves (RT17-2)**

- **A. Refuse it, naming both saves,** as decision 068 refuses a line's name that is also the start of another save's ID. Neither passes for the other. The line can't be reached by that name until it is renamed.
- **B. Take the line and warn.** What Git does, with the other winner. A script that doesn't read warnings still gets a save its author may not have meant.
- **C. Take the tag,** as `git rev-parse` does. It goes against decision 159's order, and lets a planted tag take a line's name.

**A holder that `line delete` can't read (RT17-3)**

- **A. Report it as damage, naming the holder.** Nothing is deleted.
- **B. Leave that holder out, with a warning,** since a save that isn't there holds nothing. But if the save is missing because the project lost part of its data, the tag did hold those saves, and deleting the line would let `tidy` free them later.

**A tag's name in a hint (RT17-4)**

- **A. The hint doesn't carry the name:** "run the same command with --yes", as the hint for moving a tag already says.
- **B. Quote the name for the shell.** No one way of quoting is right in every shell: cmd, PowerShell and sh each read quotes differently.

**A tag's name on screen (RT17-5)**

- **A. Show a name that doesn't fit the line-name rules between double quotes,** with any `"` or `\` inside it marked, so that its end can't be forged.
- **B. Refuse such names when a tag list is read.** Format 1 is frozen ([decision 121](121-format-1-freezes-at-v0.1.md)) and v0.1 reads them, and Git's own tags use many of these characters.

## Decision

**A in each, and RT17-1 fixed as decision 159 already says.**

- **RT17-1: one order, in one place.** `goto`, `show`, `changes --save` and `tag --save` read a name through the same code: a line, a server line, a tag, the start of a change ID, the start of a fingerprint. `opalro show main@origin` shows the server line's save.
- **RT17-2: a name that is a line (or a server line) and also a tag at another save is refused,** in all four commands. The message gives the first 12 hex digits of each save's fingerprint, which `goto` and `show` take:
  - `"v1" is the name of a line and also of a tag at another save (line 3f9a1c2b4d5e, tag 9b8c7d6e5f4a), so Opalro didn't pick either.`
  - The hint: `To reach one of them, type its ID as shown here. To reach the line by another name: opalro line rename <old> <new>`. For a server line the hint is its first sentence alone.
  - When the line and the tag are at the same save there is nothing to choose, and the line is taken.
  - `line rename`, `line delete`, `tag <name>` and `tag --delete` take the name of their own kind and never ask what else it could mean, so they are the way out.
- **RT17-3: damage, naming the holder.** When `line delete` can't read the save a tag, a line or a server line names, it stops, changes nothing, and says which: `the tag ghost names a save that can't be read: …`, with the hint all damage gets.
- **RT17-4: no stored name inside a command in a hint.** The hint is `To delete it, run the same command with --yes.` The hints that carry a line's name stay, because a stored line name always fits the line-name rules.
- **RT17-5: an odd name is shown between double quotes.** Wherever Opalro writes a tag's name as text (the list, `history`'s marks, `show`'s row, the questions, the outcome lines and the messages), a name that fits the line-name rules is written as it is, and any other between double quotes, with `"` and `\` inside it written as `\"` and `\\`. `--json` carries the name as it is stored.
- **The suspicions: no finding.** Whether the tag questions should count the saves a tag alone holds is the owner's to say ([follow-ups](../plans/2026-11-01-milestone-1-follow-ups.md)). The walk's cost is timed before milestone 8 builds on it, as decision 160 already asks.

## Consequences

- **A change from v0.1 for one rare case.** A project imported with a branch and a tag of one name at two saves: v0.1's `goto <name>` took the line, since tags weren't names yet. Now it is refused until the line is renamed.
- `show` and `changes --save` take `line@server`, which v0.1 refused as unknown.
- Reading a line's name now reads the tag list too: one more record per `goto`, `show` or `changes --save`.
- The import doesn't yet say, when it runs, that a branch and a tag share a name; the refusal says it on first use.
- The five proof tests (`crates/opal-vcs/tests/redteam17.rs`) run on every push once each fix is in, and a second session attacks the fixes.
- [security.md](../security.md), [json.md](../json.md), [terminal.md](../terminal.md) and the [guide](../guide.md) describe all of this.
