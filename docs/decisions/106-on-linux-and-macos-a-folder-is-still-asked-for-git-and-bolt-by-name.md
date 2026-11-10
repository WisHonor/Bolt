# 106 — On Linux and macOS a folder is still asked for `.git` and `.bolt` by name

- **Status:** Accepted
- **Date:** 2026-11-10
- **Agenda topic:** none (milestone 6, speed and space)
- **Decided by:** Claude, at the owner's request while the owner was away. The owner can reverse any part.
- **Refines:** [decision 101](101-the-walk-lists-a-folder-before-it-looks-inside.md) (when the walk asks a folder for `.git` and `.bolt` by name)

## Context

Decision 101 lets the walk list a folder first and ask it for `.git` and `.bolt` by name only when the listing could hold them: when a name in it is `.git` or `.bolt` in any ASCII letter case, or holds a character outside ASCII. It rests on one claim: a drive takes an all-ASCII name for `.git` only by letter case. The record said so itself: "If a drive ever matched an all-ASCII name to `.git` by some other rule, a nested repository there would be missed and its files saved. Bolt knows of no such drive; the red team is asked to look for one."

The review of the code that builds decision 101 found one, before the red team started.

FAT and NTFS drives keep two names for a file: the long one people see, and a short one in the old eight-dot-three form (`PROGRA~1`). Either name opens the file. On Windows a folder's listing gives both, and decision 101 looks at both. On Linux and macOS the listing gives only the long name, but the drive still answers to the short one: Linux's FAT and NTFS drivers and macOS's FAT driver all compare a lookup with the short name too.

A short name has at most eight characters, a dot and three more. `.GIT` fits that shape, with nothing before the dot. No system writes such a short name on its own (Windows gives `.git` the short name `GIT~1`), but a drive image made by hand can hold one. So on a FAT or NTFS drive plugged into a Linux or macOS machine, a folder can hold an entry that is listed as `notes` and also answers to `.git`:

- Git, and Bolt before decision 101, ask for `.git` by name, get a yes, and leave the folder out as a repository of its own.
- With decision 101's rule the listing shows only `notes`, so nothing is asked, and the folder's files are saved.

Nothing is lost and nothing is run. But a save takes a folder it would have left out, which is the one thing the nested-project check is there to stop ([decision 046](046-nested-projects-left-out.md)), and exactly the gap decision 101 asked to be looked for.

The finding comes from reading the drivers' sources, not from a run: it needs a drive image mounted, which the red-team sandbox can't do.

`.bolt`, `.gitignore` and `.boltignore` can't be short names: each has more than three characters after its dot.

## Options

- **A. Accept it.** It takes a drive image built by hand, and a drive that lies can do worse. But the rule this work follows is to keep the safe way unless the gain is large and the risk is shown to be nil, and neither holds here.
- **B. On Linux and macOS, ask every folder for `.git` and `.bolt` by name, as before decision 101.** The listing still decides whether the ignore files are opened. Two lookups a folder, each a microsecond or two there: about half a millisecond of a `status` on Git's source (226 folders).
- **C. Ask the system what kind of drive it is, and ask by name only on FAT and NTFS.** A list of drive kinds to keep right on two systems, and network drives and user-space drives hide what is underneath them.

## Decision

**B.**

- **Linux and macOS.** Every folder the walk goes into is asked for `.git` and `.bolt` by name, whatever its listing shows, exactly as before decision 101. Both names are asked, though only `.git` can be a short name, so that the check there is simply the old one.
- **Windows.** Decision 101 stands as written: the listing gives each entry's short name, and the walk looks at it.
- **Ignore files, on every system.** Decision 101 stands: `.gitignore` and `.boltignore` are opened only when the listing could hold them. Their names can't be short names.
- **The folder is still listed first,** on every system, and the walk still goes on from that one listing.

### Also recorded: which problem is reported first

Decision 101 named one case where a folder with two problems now reports the other one. The code as built has more of the same kind, all for a folder that can be opened but not listed: its listing's failure is now reported before its name, the length of its path and its depth are judged, where those came first before. The save stops at the same folder either way. A folder deeper than Bolt allows is listed once before it is refused.

## Consequences

- On Linux and macOS decision 101 saves two calls a folder (the ignore files), not four. On Windows, where a failed lookup costs most, it saves four.
- The gap is closed on the systems where a listing can't show a short name. [security.md](../security.md) T32 says so, and the red team is still asked for any other drive that takes an all-ASCII name for `.gitignore`, `.boltignore`, or (on Windows) `.git` or `.bolt` by a rule other than letter case.
- [design.md](../design.md) §5.9 and decision 101 (a back-link line) point here.
- Tests: on Linux and macOS, a walk asks each folder for `.git` and `.bolt` whatever it holds, and opens no ignore file in a folder whose listing can't hold one; on Windows the counts are decision 101's.
