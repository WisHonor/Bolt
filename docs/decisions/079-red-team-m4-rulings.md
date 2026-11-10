# 079 — Rulings after milestone 4's red-team session

- **Status:** Accepted
- **Date:** 2026-11-08
- **Agenda topic:** none (milestone 4's red-team gate, [decision 024](024-red-team-gate.md))
- **Decided by:** Claude, at the owner's request while the owner was away. The owner can reverse any part.
- **Refines:** [decision 077](077-tidy-and-the-undo-window.md) (what `tidy` keeps)
- **Refined by:** [decision 080](080-red-team-m4-session-2-rulings.md) (the second look before every change); [decision 123](123-follow-ups-done-or-moved-past-v0.1.md) (RT6-3 is fixed, and a diary end that can't be read back no longer stops `tidy`)

## Context

The first red-team session on milestone 4 ([report](../redteam/2026-11-08-milestone-4.md)) proved four findings:
- **RT6-1, Medium.** When `state/head` is put back to an older entry that still reads, `tidy` keeps only the diary from that entry back. It then deletes every save made after it. A sync tool or a restore of `.bolt/state` can do this, and nothing warns.
- **RT6-2, Medium.** Write-out checks a file and then writes the new contents in `.bolt/tmp`. It renames them into place with no second look. A program that saves into the file during that gap loses its save, with no safety copy. For a big file the gap is a second or more.
- **RT6-3, Low.** When `head` is lost after a stopped switch, `undo --list` offers only the stopped switch's own entry. Picking it leaves a journal that every changing command refuses.
- **RT6-4, Low.** A planted 4 GiB `state/journal` is read whole by every command, read-only ones such as `status` included, and can exhaust memory.

## Decision

- **RT6-1.** `tidy` also keeps every diary end: an entry no other entry names as its parent, which is what `undo --list` offers when `head` is lost (decision 075). For every end whose time is within the undo window, it keeps the end and its diary back along parents, as it keeps the diary from `head`. It also keeps everything the states of those entries reach, as it does for entries in the window. A diary cut off from `head` then lives at least as long as the undo window, and `undo --to` can still reach it.
- **RT6-2.** This is no new rule. Decision 049 already says each file is checked again just before it is replaced, and the code didn't do so at the rename. Write-out now looks at the file again right before it renames the new contents into place. If its identity, size or change time differ from the plan's look, it leaves the file and reports it (`changed_since_plan`). What is left of the gap is the instant row 70 of the follow-ups already accepts, until a no-replace rename exists.
- **RT6-4.** A read-only command reads only the journal's front: enough to say a switch is unfinished and between which entries. Changing commands still read and check it whole, which is the known cost in the follow-ups' row 69.
- **RT6-3 waits.** Nothing is lost: the saves are intact, and the hint's route (delete the journal, then `discard`) gives a working folder back. Finishing the switch on pick-up, or offering its `from` entry, is a follow-up for before v0.1. The proof test stays ignored until then.

## Consequences

- `tidy` keeps more after a rollback or a crash's leftover entry, by at most the undo window's worth of entries.
- A rename during write-out waits for one more look at the file.
- A planted journal costs a read-only command a few bytes of reading.
