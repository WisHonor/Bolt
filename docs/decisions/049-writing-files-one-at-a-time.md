# 049 — `goto` and `discard` write one file at a time, from a journal that a crash leaves to finish

- **Status:** Accepted
- **Date:** 2026-11-05
- **Agenda topic:** none (milestone 3 planning, question 2)
- **Decided by:** Claude, at the owner's request ("do whatever you think is best", 2026-11-05). The owner can reverse any part.
- **Refines:** [decision 010](010-operation-log.md) (folder rewrites are journaled, and finished or rolled back) and [decision 011](011-status-and-write-out.md) (plan, re-check, temp file and rename)
- **Refined by:** [decision 075](075-crash-recovery.md) (every milestone 4 command survives a crash; a lost `head`); [decision 076](076-renames-reach-the-disk-on-windows.md) (renames and removals written through on Windows); [decision 123](123-follow-ups-done-or-moved-past-v0.1.md) (Windows error 303 counts as a file in use)

## Context

`goto` swaps the files in the folder for another save's; `discard` puts back the current save's. No operating system changes thousands of files in one instant, so a power cut halfway leaves some files new and some old. Decisions 010 and 011 already say each file is written to a temporary file and renamed into place, and that the whole switch is journaled first. Left open: in what order, and what the next command does after a crash.

## Options

- **A. One file at a time, journal first; the next command finishes the switch.** Mercurial records an interrupted update and refuses to go on until it is run again; Git keeps no journal.
- **B. Every new file into `.bolt/tmp` first, then a quick pass of renames.** The window is shorter but the switch still isn't one step, and a big file needs its size free twice.
- **C. Roll back after a crash,** putting the old files back.

## Decision

**A.**

- **Order:**
  1. Under the lock, plan every change by comparing the folder with the current save and the target save, and stop before anything changes if unsaved work would be overwritten (decision 050).
  2. Store the safety copies and the new diary entry, without moving `state/head`.
  3. Write `state/journal`: the target entry, each planned change and the safety copies.
  4. Make each change: folders, then files, through a temporary file and a rename; removals; links last (decision 052). Each file is re-checked just before it is replaced, and one that isn't what the plan expected is left alone and reported.
  5. Move `state/head` to the new entry: the moment the command takes effect.
  6. Remove the journal.
- **After a crash** everything the switch needs is already stored, so it is finished, never rolled back. The next command that changes anything finishes it first and says so; read-only commands say a switch is unfinished and that running `bolt goto` again finishes it. A journal whose entry `head` already names is only removed.
- **A file in use on Windows** (an editor or antivirus holding it) is tried again for about a second; then the rest is finished, the journal stays, and Bolt names the file and says that another program has it open and that running the command again finishes. Naming the program (Windows' Restart Manager, more `unsafe`) waits for a later milestone.

## Consequences

- [storage-format.md](../storage-format.md) §10 gets the journal's exact bytes, with test vectors, before code.
- Crash tests stop `goto` and `discard` at every write and check that the next command finishes them, with every file whole.
- Decision 011's "name the program holding a file" joins the follow-ups.
