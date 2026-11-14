# 163 — Red team on milestone 7, second session: six findings, all fixed

- **Status:** Accepted
- **Date:** 2026-11-14
- **Agenda topic:** none (milestone 7, its second red-team session, RT18)
- **Decided by:** Claude, at the owner's request while the owner was away. The owner can reverse any part.
- **Refines:** [decision 162](162-red-team-m7-rulings.md) (the refusal's hints; what `line delete` names; how an odd tag name is shown; what a hint may carry); [decision 055](055-lines-in-v0.1.md) (where a save is looked for by the start of its fingerprint)

## Context

The second session on milestone 7 (RT18, [report](../redteam/2026-11-14-milestone-7-session-2.md)) attacked the fixes of the first ([decision 162](162-red-team-m7-rulings.md)) and what the first had left undone, on a copy of `milestone-7` at d149d80 in the no-network sandbox. It found six things: one medium, five low. None loses data, skips a question, or takes a name for the wrong save. The fixes hold where they were aimed. What broke was the advice around them: Opalro told a person to type something it then refused.

What held: all four commands read a name through one place; the refusal can't be dodged and fails closed when the tag list can't be read; `line delete` writes nothing before it stops; odd names never ended a question early or made a second row. Two commands at once, tidying after a deleted line, and a planted diary step were checked by reading the code, and held.

- **RT18-1 (Medium).** The refusal of decision 162 shows each save's ID and says to type it. But a save was found by its fingerprint only when a line reached it. A server line's save is often ahead of every line, and an imported tag can sit on a save no line reaches. For those the ID shown was refused, and for a server line nothing at all reached its save while the tag was there.
- **RT18-2 (Low).** `line delete` named a tag whose own save can't be read, but not one whose save is there while an older save behind it is missing: that still gave `object … is not in the store` with no hint.
- **RT18-3 (Low).** A quoted tag name could still hold `]  [main]  [` inside the quotes, so its row in `history` still showed a `[main]`.
- **RT18-4 (Low).** After `opalro show` of a save that isn't on the line the project is on, the next step it suggested (`opalro changes --save <ID>`) used a short ID that only works on the line, and failed. This is older than milestone 7; tags and server lines made it common.
- **RT18-5 (Low).** Following the refusal's advice (go to the line's save by its ID) worked, and then said `To keep saving on v1: opalro goto v1`, which is the refusal again.
- **RT18-6 (Low).** When a shelf can't be brought back, the hint put the shelf's stored label inside a command. Opalro only makes labels that are numbers, but a planted project can hold `1; touch pwned`. This too is older than milestone 7, and it is the same mistake decision 162 fixed for tags.
- **Suspicions, without proof.** Reading a line's name now holds the whole tag list twice: with a tag list just under the largest a record can be (7,900 tags with 64 KiB messages), `goto main` peaked at 1.0 GB in a debug build. A tag list that can't be read now stops `goto` by any name. Wide characters can run a little past the width (known since the first session).

## Options

**The IDs the refusal shows (RT18-1)**

- **A. Make them work:** a fingerprint also finds a save that a server line or a tag reaches. Those saves are held already, by `tidy` and by `line delete`.
- **B. Show only IDs that work, and otherwise name another way out.** For a server line whose save no line reaches there is no ID to show.

**A save the walk can't read, behind a holder's own save (RT18-2)**

- **A. Name the holder whose history holds it,** found by one more look after the walk has failed.
- **B. The plain damage message,** without the holder's name.

**A third session on these fixes**

- **A. None:** the fixes are small, each has its proof test, and the whole-branch review reads them. Milestone 8's session attacks names and hints again, since `combine` prints both.
- **B. A third session now.**

## Decision

**A, A and A; the rest fixed as the report suggests.**

- **RT18-1.** A save is found by the start of its fingerprint among the saves that lines, server lines and tags reach along first parents. (Reaching saves through every parent, as a combine will need, is milestone 8's.) The refusal's hints also name the way to end the double meaning:
  - for a line: `To reach one of them, type its ID as shown here. To end the double meaning, rename the line (opalro line rename <old> <new>) or delete the tag (opalro tag --delete <name>).`
  - for a server line: `To reach one of them, type its ID as shown here. To end the double meaning, delete the tag: opalro tag --delete <name>`
- **RT18-2.** When the walk can't read a save, `line delete` looks for the first line, tag or server line whose history holds it and names it as damage: `the tag ghost names a save whose history can't be read: …`. When only the deleted line's own history holds it: `line feat's history can't be read: …`. Nothing is deleted either way.
- **RT18-3.** Inside the quotes of an odd name, `[` and `]` are written `\[` and `\]`, as `"` and `\` already are, so no line's mark can appear inside it.
- **RT18-4.** A save's ID inside a hint's command is one the command will take from where the project is: its short change ID when the save is on the line, otherwise the first 12 hex digits of its fingerprint.
- **RT18-5.** `goto` says `To keep saving on <line>: opalro goto <line>` only when that command would go onto the line. When the line's name is also a tag at another save it says: `Line v1 is here too, but v1 is also a tag at another save. To keep saving on the line, rename it first: opalro line rename <old> <new>`
- **RT18-6. No hint puts text from the project inside a command unless it is plain:** only the characters a line's name may have, not starting with `-` (and `line@server` when both parts are so). Anything else is left out: the hint then says to run the same command again, or shows a placeholder. This is one rule in one place, for names, labels and paths alike.
- **The tag list is read once for one name,** not twice.
- **No third session.**

## Consequences

- RT18-4 and RT18-6 are in v0.1's draft release too, since both are older than milestone 7. Neither loses data: one is a suggestion that fails, the other needs a planted project and a person who pastes a hint. They are fixed on `v0.2`; whether v0.1 gets them before it is published is the owner's call ([rulings](../plans/2026-11-14-milestone-7-rulings.md), "Please confirm").
- A fingerprint now also reaches a tag's save and a server line's, and the saves behind them along first parents. A line or tag named with 8 or more hex digits is checked against those too (decision 068).
- A tag list near the largest a record can be still costs about half a gigabyte to read for one name. Looking a name up needs only the names; reading the list in pieces is on the [follow-ups](../plans/2026-11-01-milestone-1-follow-ups.md).
- The damage message for a tag list that can't be read doesn't say that `opalro undo` may bring the project back; also on the follow-ups.
- The six proof tests (`crates/opal-vcs/tests/redteam18.rs`) run on every push once each fix is in.
