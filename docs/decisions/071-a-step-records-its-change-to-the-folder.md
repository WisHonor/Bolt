# 071 — A step records its change to the folder as a switch between two saves

- **Status:** Accepted
- **Date:** 2026-11-06
- **Agenda topic:** none (milestone 4 planning)
- **Decided by:** Claude, at the owner's request while the owner was away. The owner can reverse any part.
- **Refines:** [decision 010](010-operation-log.md) (what a diary entry records) and [decision 050](050-unsaved-work-in-goto-and-discard.md) (how safety copies are kept, and what `discard` leaves alone)

## Context

Undo has to put back what a step did to the folder, and redo has to do it again ([decision 070](070-undo-and-redo-step-through-the-diary.md)). A diary entry records the state after its command and the safety copies it made: the contents of the unsaved files `discard` threw away (decision 050). That leaves gaps:
- a file the user had deleted, which `discard` brings back, has no copy, so an undo couldn't delete it again; nor is an empty folder `discard` removed listed anywhere;
- an entry doesn't say whether its command changed the folder at all. After `save` and after `goto` the project is on another save, but only `goto` rewrote files;
- a switch between whole saves doesn't say which paths a command actually wrote. `goto` leaves alone unsaved work that already matches its target (decision 065), and `unshelf` can land on a save other than the one shelved on: undoing either as a switch between two whole saves would write over files the step never touched.

## Options

- **A. A switch between two saves holding exactly what the step changed.** An entry may record one change to the folder: from one save to another, both built on the save the folder was on, holding what the folder held and what the step wrote, at exactly the paths it wrote. Undo switches backwards and redo forwards through `goto`'s planner, so its rules for unsaved work, ignored files, project links and names apply unchanged, and the planner leaves alone every path where the two saves agree. Git keeps set-aside work the same way (a stash is a commit), and Jujutsu keeps the whole working copy as a commit at every command.
- **B. The writes themselves.** An entry lists every path its command wrote, with what was there before and after, as the journal does (decision 049). The same information as A, but as a new kind of list in the diary rather than saves, and undo then needs a planner of its own for whatever it finds in the way.
- **C. Infer it from the command.** Keep the format and decide from the command's name what undo does. The diary records only the command's name, and Git's words arrive as shortcuts in milestone 6, so the name can't be relied on; and deletions still couldn't be undone.

## Decision

**A.**
- **The format** ([storage-format.md](../storage-format.md) §9): the list of safety copies at the end of an entry is replaced by an optional folder change: a `bool`, then, if it is true, the ID of the save the folder was switched from and the ID of the save it was switched to, which must differ. An entry without one encodes as before (one zero byte where the empty list was), so the `minimal` test vector of milestone 1 keeps its bytes.
- **What the two saves hold.** Both start from the save the folder was on before the step, so they agree everywhere the step changed nothing. A step switches the folder between two saves (for `goto`, the save it left and the one it goes to); at each path where those two differ and the step writes, or finds the folder already holding the target's version (decision 065), the "to" save holds that version, which is what the folder holds there after the step, and the "from" save holds what the folder held there before: what the step wrote over (a file or link, a folder, or nothing), or the same target's version where the folder already held it. Where one of them is exactly a save that already exists (the save the folder was on, or the save switched to), that save is recorded; otherwise it is a kept save made for the step. So undo writes back what the folder held, at exactly the paths the step wrote, and leaves alone what the folder already held; redo writes again what the step wrote; a path changed since stops them (decision 070). Paths the step leaves unwritten (names this computer can't hold, `.git` and `.bolt` spellings, project links) keep the base's version on both sides, except inside a folder the step makes where the folder held a file, a link or nothing: there the "to" save holds only what the step writes, or finds already there, and the "from" save what the folder held. So the "to" save is a true picture of the folder at every path the step touched, which write-out uses to judge links.
- **Safety copies become kept saves.** What `discard` and `shelf` throw away is their "from" save: the current save with the folder's own version of each changed, new or deleted file or link they put back and each new folder they remove; the file contents are stored as safety copies are stored today. Their "to" save is the current save, or a kept save where they make a folder the folder didn't hold (above). A kept save is hidden: only its entry names it (and the state's shelves, for a shelf), which keeps it for the undo window ([decision 077](077-tidy-and-the-undo-window.md)).
- **What each command records:**
  - `goto`: from the save it left to the save it went to; where unsaved work already matched the target, the "from" save is a kept save holding that work, since it was already there;
  - `discard` and `shelf`: from the kept save to the current save (or a kept save, where they make a folder the folder didn't hold);
  - `unshelf`: from the current save, with what the folder held at the paths the shelf changes, to the current save with the shelf's version at them ([decision 073](073-shelves.md));
  - `undo` and `redo`: the folder change of the step they undo or redo, backwards for an undo, since that step's two saves already picture the folder before and after it; `undo --to`, when it is a step of its own: what it wrote, as `goto`;
  - `save`, `save --amend`, `start` and `import` change no file, so they record none.
- **Links that can't be put back.** `discard` and `shelf` leave where it is an unsaved link (new, or changed) that writing out could only bring back as a placeholder file: one that leads out of the project, or through a name Bolt never writes (a `.git` or `.bolt` spelling, or a name this system can't hold), or any link where this system can't make links. A warning names it. Bringing it back as a link would mean trusting a link read from the diary, which hostile diaries rule out (security.md T26); bringing it back as a file would quietly turn the user's link into a file at the next save.
- **A kept save** is a save like any other: its message names the command and the paths it was given (`discard art/hero.psd`), or is the shelf's message; its author and saver are the user; it has a new random change ID, and a `line` field when it was made on a line. `bolt show` shows it by its fingerprint.
- **Names.** Undo and redo read only saves, so every path they write comes from folder listings, whose names are checked when they are read and again just before each is written (decisions [049](049-writing-files-one-at-a-time.md) and [051](051-names-this-computer-cant-write-and-length-limits.md)). That meets the follow-up asking undo to check safety-copy paths from the diary before writing anything; a test plants a hostile diary to prove it.

## Consequences

- `discard`'s messages and `--json` don't change: it still says a safety copy was kept, and `safety_copies` still counts the files kept.
- A kept save costs a few folder listings: one for each folder above a path it holds. Most `goto`s record two saves that already exist, and make none.
- `tidy` keeps both saves of the folder change of every entry within the undo window.
- The `full` test vector in `diary.json` changes, since it held safety copies; `minimal` keeps its bytes.
