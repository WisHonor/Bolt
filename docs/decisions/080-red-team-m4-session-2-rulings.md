# 080 — Rulings after milestone 4's second red-team session

- **Status:** Accepted
- **Date:** 2026-11-08
- **Agenda topic:** none (milestone 4's red-team gate, [decision 024](024-red-team-gate.md))
- **Decided by:** Claude, at the owner's request while the owner was away. The owner can reverse any part.
- **Refines:** [decision 079](079-red-team-m4-rulings.md) (the second look before write-out changes a file)

## Context

The second session ([report](../redteam/2026-11-08-milestone-4-session-2.md)) attacked the fixes decision 079 made and proved two findings:
- **RT7-1, Medium.** Before write-out removes a file, it reads the whole file to check it is the saved version, then removes it with no further look. A program that writes into the part already read loses its edit, and no safety copy is kept. For a big file the gap lasts the whole read. A folder or a link put where a file was has the same gap.
- **RT7-2, Low.** `tidy` frees what only steps older than the undo window need. If the window is then made longer, those steps count as inside it again. `tidy` looks for what it already removed, and stops with a message that the project is damaged. It keeps stopping until those steps age out.

## Decision

- **RT7-1.** The second look of decision 079 comes before every change write-out makes to a path that holds something: a removal, and a folder or link put in a file's place, as well as a replacement. It uses the same check, `same_file`. A file that differs from the plan's look is left and reported (`changed_since_plan`). What remains open is the instant between the look and the change, as decision 079 accepts.
- **RT7-2 waits.** Nothing is lost: `tidy` only refuses to run, and every other command works. The fix needs a rule for telling "removed by an earlier `tidy`" apart from "lost". One way is to record the window `tidy` last used and treat a longer window as starting from that `tidy`. It is a follow-up for before v0.1, and the proof test stays ignored until then. Until it is fixed, the message is wrong only in this case, after the user has made the window longer.

## Consequences

- Removing a file during a switch costs one more look at it.
- A user who lengthens the undo window after a `tidy` can't `tidy` again until the older steps age out.
