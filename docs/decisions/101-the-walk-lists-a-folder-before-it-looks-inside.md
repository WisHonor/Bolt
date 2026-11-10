# 101 — The walk lists a folder before it looks for `.git`, `.bolt` and ignore files in it

- **Status:** Accepted
- **Date:** 2026-11-10
- **Agenda topic:** none (milestone 6 planning, speed and space)
- **Decided by:** Claude, at the owner's request while the owner was away. The owner can reverse any part.
- **Refines:** [decision 046](046-nested-projects-left-out.md) (how the walk finds a folder that holds a repository or project of its own) and [decision 038](038-gitignore-and-saved-files.md) (when a folder's ignore files are opened)

## Context

For every folder it goes into, the walk asks the drive four questions by name before it lists the folder: is there a `.git` here, is there a `.bolt`, and can `.gitignore` and `.boltignore` be opened. In almost every folder all four answers are no. Then it lists the folder, which already says which names are there.

A failed lookup costs little on Linux (a microsecond or two) and a lot on Windows (about 10 µs on GitHub's runner, about 35 µs on the owner's laptop with its virus scanner). On Git's source the four cost about 2 ms of a 63 ms `status` on Linux and about 9 ms of 102 ms on Windows; on Godot's demos, with 569 folders, about 5 ms and 17 ms ([benchmarks.md](../benchmarks.md) §7).

The lookups go to the drive on purpose. Whether a folder holds a `.git` is the drive's call: where the drive ignores letter case, `.GIT` is found as `.git` ([decision 046](046-nested-projects-left-out.md)), and a drive may fold names in ways Bolt doesn't model. So the listing can't simply replace them.

## Options

- **A. Keep asking by name in every folder.** Exact, and four lookups a folder.
- **B. Decide from the listing alone,** comparing names as Bolt thinks the drive compares them. No lookups, but wrong on any drive whose rules Bolt doesn't model, and a missed `.git` means a nested repository's files are saved.
- **C. List first, and ask by name only when the listing could hold the name.** The listing can only rule a lookup out; whenever it can't, the drive is asked exactly as today.

## Decision

**C.**

- **The order.** Once the ignore rules of the folders above have kept a folder, the walk opens it and lists it, then decides the rest from that one listing: whether it is a project of its own, which ignore files to read, and what it holds.
- **`.git` and `.bolt`.** The walk asks the drive for `.git` and `.bolt` by name, as before, if any name in the listing (on Windows, any short name too) either
  - is `.git` or `.bolt` in any ASCII letter case, or
  - holds a character outside ASCII.

  Otherwise it doesn't ask: the folder holds neither.
- **`.gitignore` and `.boltignore`.** The walk tries to open each, as before, if a name in the listing is that name in any ASCII letter case, or any name holds a character outside ASCII. Otherwise the file isn't there.
- **Why that is enough.** A lookup by name can succeed only if the drive takes some entry of the folder for that name. Every drive Bolt runs on compares a name made only of ASCII characters with another by ASCII letter case at most; the wider rules (case tables beyond ASCII, Unicode forms, the characters HFS+ ignores) all need a character outside ASCII in the entry's name. So with only ASCII names in the folder, and none of them the name in any letter case, the lookup would fail. Whenever a name leaves ASCII, Bolt doesn't reason: it asks.
- **What doesn't change.** The answer still comes from the drive whenever the listing can't rule it out. No link is followed at any name. An ignore file is read by the same rules (a regular file only, the same limits, the same warnings and stops). A name Git treats as `.git`, or Bolt as `.bolt`, is still left out of the save with its warning. The project's top folder is handled as before.
- **One view.** The listing that decides is the listing the walk then walks, so the check and the walk see the folder at one moment. Before, a `.git` made between the lookup and the listing was missed until the next command; now it is seen.

## Consequences

- A folder with plain ASCII names costs an open and a listing, with no failed lookups: four fewer calls a folder.
- A folder with any name outside ASCII costs what it costs today.
- If a drive ever matched an all-ASCII name to `.git` by some other rule, a nested repository there would be missed and its files saved. Bolt knows of no such drive; the red team is asked to look for one ([security.md](../security.md) T32).
- A folder that can't be listed stops the save where it stopped before, at that folder; when a folder has two problems at once (an unreadable ignore file and a listing that fails), the one reported may now be the listing's.
- Tests: on every system, a folder with `.git` or `.bolt` in each letter case, with a name outside ASCII beside it, and with neither, is judged as the drive judges it; the ignore files likewise; a folder swapped between its listing and its use is still refused.
