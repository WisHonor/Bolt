# 123 — Follow-ups before v0.1: done in milestone 6, or moved past it

- **Status:** Accepted
- **Date:** 2026-11-09
- **Agenda topic:** none (milestone 6 planning)
- **Decided by:** Claude, at the owner's request while the owner was away. The owner can reverse any part.
- **Refined by:** [decision 125](125-a-safe-folders-entry-is-judged-by-one-look.md) (items 5 and 13: a `safe-folders` entry is judged by one look, and `switch_dropped` is told even when the pick-up fails)
- **Refines:** [decision 039](039-first-save-after-import.md) (the line-ending hint); [decision 043](043-red-team-m2-rulings.md) (sparse checkouts Bolt can't see); [decision 049](049-writing-files-one-at-a-time.md) (which Windows errors mean a file is in use); [decision 060](060-user-git-settings-includes-and-paths.md) (paths in the user's Git settings); [decision 075](075-crash-recovery.md) (picking up after a lost `head`; the hint of a switch that stopped); [decision 077](077-tidy-and-the-undo-window.md) and [decision 079](079-red-team-m4-rulings.md) and [decision 080](080-red-team-m4-session-2-rulings.md) (what `tidy` does when something it would keep can't be read); decisions [026](026-ownership-check-details.md) and [042](042-trust-hint-skips-folders-anyone-can-write-to.md) (`safe-folders`); [decision 040](040-import-once-catch-up-later.md) (catching up with Git); [decision 017](017-commands-and-output.md) (`--debug` waits)
- **Refined by:** [decision 109](109-the-speed-sessions-follow-ups-and-format-1s-three-open-rows.md) (the rows it left to the speed session); [decision 147](147-the-package-sessions-follow-ups-before-v0.1.md) (the package session's rows)

## Context

[The follow-ups list](../plans/2026-11-01-milestone-1-follow-ups.md) holds what five milestones put off. Many rows say "Before v0.1" or "Milestone 6". Milestone 6 is built by two sessions at once: the speed session owns every row about speed, memory or space (a row is its own if fixing it changes the walk, the store, or the import's speed or memory), and this record covers the rest. The owner's rule for them: do it, or move it past v0.1 in a decision record that says why.

Each row's premise was checked against the code on 2026-11-09 before it was ruled on.

## Decision

### Done in milestone 6

Each line says what changes. None changes what a repository stores; item 6 adds one small file to `.bolt/state/`, which storage-format.md §10 describes.

1. **The line-ending hint** (decision 039). Its Git step becomes `git rm --cached -r -q :/`, which means the whole repository from any folder, and it gains a sentence: files `.gitattributes` marks `eol=crlf` stay CRLF after those steps, and `--accept-crlf` then saves just those.
2. **A sparse checkout turned on in the user's own Git settings** stops an import as one in the repository's settings does (decision 043): the import reads `core.sparseCheckout` there too, where it already reads `lfs.storage`. One turned on through an include in `.git/config` keeps its warning; see "moved" below.
3. **`bolt shelf --json` gains `changed_again`** ([decision 117](117-what-json-covers-at-v0.1.md)).
4. **Windows error 303** (a file deleted while another program still holds it, on drives such as FAT and exFAT) counts as a file in use during write-out, like errors 32 and 33 (decision 049): the file is left for the next command, not a failed switch.
5. **A lost `head` after a stopped switch** (RT6-3, decision 079). Picking an entry up with `bolt undo --to` now removes the stopped switch's journal, which no command could carry out any more. What that switch had already written shows as unsaved changes against the entry picked, and what it would have written over is in its kept save; the pick-up says so, in a `switch_dropped` warning. It still writes no file (decision 075).
6. **A longer undo window after a `tidy`** (RT7-2, decision 080). Decision 080 asked for a rule that tells "removed by an earlier `tidy`" from "lost", and suggested recording the window `tidy` last used. That is what is built: once `tidy` has worked out what to keep, and before it removes anything, it writes the start of the undo window it used into `.bolt/state/tidied`. The next `tidy` counts its window from the later of that time and its own start, so a window made longer never asks for what the earlier `tidy` freed. A save that is missing inside that window is still damage, and still stops `tidy`. The file is small text, read with a cap; one Bolt can't read, or one that names a time in the future, counts as absent, which only brings the old stop back. A planted one can at worst let a `tidy` free undo data older than the time it names, which can't be later than now.
7. **A diary end whose diary can't be read back** (a planted end whose parent is missing, say) no longer stops `tidy` for good (decision 079). `tidy` keeps what it could reach from that end, names the end in a `diary_end_unreadable` warning, and goes on. An end dated in the future is still kept as long as its date says; that costs only space.
8. **Picking up after a lost `head`** also reads the entry's tag list and server lines before making it current (decision 075), and refuses an entry whose records can't be read.
9. **A switch stopped by damage** (decision 075). Its hint now names the way on without the damaged data: deleting `.bolt/state/journal` leaves the files as they are, and the next command goes on. It says so for an object that is missing as well as for one that is damaged.
10. **Git 2.56's path prefixes** (decision 060). `:(optional)` before a `core.excludesFile` path is read as Git reads it: a file that isn't there counts as no setting. `%(prefix)/` stops the command wherever it appears, as `~user/` does, since Bolt can't know where Git is installed.
11. **`~\x` on Windows** (decision 060). Only `~/` is the home folder; `~\x` stops, as in Git. Checked against Git for Windows 2.50.1 on 2026-11-09, which says "failed to expand user dir in: '~\ign'".
12. **Two hints in the Git reader** that blamed a damaged repository, or permissions, when the computer ran out of memory now say it ran out of memory.
13. **A `safe-folders` entry that is itself a link** (a symbolic link, or a junction on Windows) no longer counts, so whoever can replace a trusted folder with a link can't move the trust to the link's target (decisions 026 and 042). When Bolt then refuses the project, its message names the entry it passed over and why.
14. **Colour on Windows, the terminal UI, and progress in an import's first pass:** [decision 120](120-the-terminal-ui.md).
15. **Packaging and OneDrive, the user guide with `safe-folders`, `SECURITY.md`:** [decision 119](119-packaging-v0.1.md).
16. **Line names** and **change-ID letters:** decisions [121](121-format-1-freezes-at-v0.1.md) and [122](122-change-ids-in-jujutsus-letters.md).

### Moved past v0.1

1. **Catching up with Git** (a second `bolt import` that brings in new commits; decision 040). It was "before v0.1 if the milestones leave room". They don't: it is a command of its own, with rules to settle for lines that moved on both sides, and v0.1's promise is a one-time import. A second import still refuses.
2. **The first save after an import stopping when imported files are missing from disk** (a sparse checkout set up through an include; decision 043's row). The stop would also catch files the user deleted on purpose and hasn't committed, which decision 039 says come along. It needs a flag, and a rule for partial saves (decision 090). Until then the include is named in a warning at import, and nothing is lost: the files stay in the imported save.
3. **A project whose `.bolt/tmp` can't be written** still fails `status` and `changes`. They test there how the drive treats letter case and Unicode forms; without the test Bolt would have to guess, and a wrong guess is a wrong status. The failure names the folder.
4. **`GIT_CONFIG_PARAMETERS`** (Git's `-c` options, passed to programs Git starts). It reaches Bolt only when Git itself runs it, from an alias or a hook. It is written in Git's own quoting, and a reader that gets that wrong is worse than none. The limit stays documented.
5. **Emoji flags written with tag characters** stay escaped. The owner's rule for milestone 6 is that text from a project is escaped as it is today, and decision 028 chose safety: tag characters can carry hidden text.
6. **A warning about files that look like secrets** (security.md T12). Decision 043 left it "for later"; the Code preset keeps `.env` files out ([decision 113](113-presets-what-each-writes-and-how-start-picks-one.md)).
7. **`--debug`** (decision 017 promised technical details behind it). Bolt's errors already carry the system's own message where it helps, and nothing more is collected that a flag could show.
8. **A progress bar for `save`** and the other commands that store or write files. They show that they are working, not how far they are: that needs the engine to report progress from its storing loop.

### The speed session's rows

These stay with the speed session, which decides each in its own records: the memory a hostile save can cost a reader; many planted packs; a tighter bound on a stored copy's size; the import's memo tables, its header memo and its repeated reads; Git tag objects stored once; folders deeper than the walk holds open; the four "measure" rows of milestone 4; hostile lists of tiny chunks; where Git wins; and the set of checked copies.

### Still the owner's

The license; the cloud-placeholder row ("the owner's call"); and milestone 5's please-confirm list.

## Consequences

- The follow-ups list is updated row by row as each fix lands, and the moved rows change their "By" to "After v0.1" with a pointer here.
- RT6-3's and RT7-2's tests (`rt6_3`, `rt7_2`), ignored since milestone 4, run again.
- [json.md](../json.md) gains `diary_end_unreadable` and `switch_dropped`; [security.md](../security.md)'s rows for T27 and T28 drop their "before v0.1".
