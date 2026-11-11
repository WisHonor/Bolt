# 126 — Red team, milestone 6 (session 1): `verify` also finds a save that can't be written out

- **Status:** Accepted
- **Date:** 2026-11-10
- **Agenda topic:** none (milestone 6, the red team's first session on presets and polish)
- **Decided by:** Claude, at the owner's request while the owner was away. The owner can reverse any part.
- **Refined by:** [decision 127](127-what-verify-holds-is-bounded-and-two-costs-are-written-down.md) (what the size check keeps in memory is bounded; the cost of a deep tree in the other readers is written down)
- **Refines:** [decision 118](118-bolt-verify.md) (what `bolt verify` checks)

## Context

Decision 118 makes `bolt verify` the command that vouches for a project: every pack and object reads back as what it should be, and the project has everything it needs. The red team's first session on milestone 6 ([report](../redteam/2026-11-10-milestone-6.md)) found three planted stores that pass it with "no problems found" while `bolt goto` refuses to write the save:

- **RT10-1.** A big file's chunk is in no pack as a chunk. A pack holds a *chunk list* under that chunk's ID instead. `verify` counted an ID found in any index, whatever its kind, as present. Writing the file takes each chunk as content only, so `goto` stops: the chunk "isn't in the store".
- **RT10-2.** A save's folder listing gives a file a size its stored contents don't have. `verify` read the listing and found the contents, and never compared the two. `goto` stops: the content "isn't the size the switch expects".
- **RT10-3.** A save holds more than 10,000,000 files and folders, counted by path: one listing of 10,001 folders that all name the same listing of 1,000 files, 13 KiB stored. `verify` read each listing once and never counted by path. `goto`, like every reader, refuses such a save.

Bolt itself never writes any of the three. They come from a hostile or broken writer: a `.bolt` someone else made, a merge of someone else's packs, later a server. That is exactly the project `verify` is run on.

## Options

- **A. Fix each.** `verify` gains the three checks the readers already make.
- **B. Rule them out.** Say `verify` checks only that bytes match their fingerprints and that every ID is somewhere. True to the first build, and it leaves `verify` saying "no problems found" about a project nobody can check out.
- **C. Have `verify` write every save out to a scratch place.** It would find everything `goto` finds, and take hours and the project's whole size in disk.

## Decision

**A, under one rule: what would stop a reader from writing out a save the project needs, because of what is stored, is a problem `verify` reports.** For the three things found:

1. **A chunk is present only as a chunk.** A chunk that a file's chunk list names counts as present only when some pack holds it as file contents. One held only as another kind of object is reported as `object_missing`, with the message that it is stored, but as another kind of object than what needs it; the save and path that need it are named, as for any missing object.
2. **A listing's size is the file's size.** For every file a save's folder listing names, the size the listing gives must be the size of what is stored: the contents' length, or for a chunked file the length its chunk list gives, or for a link the length of its target. A difference is reported as `object_damaged`, naming the folder listing, the path, both sizes, and one save that needs it. The listing is the object at fault: its fingerprint covers the wrong size, so it was written wrong.
3. **A save within the limits every reader keeps.** A save with more entries than a reader takes (10,000,000, a folder listing reached by two paths counted twice), or deeper or with longer paths than a reader takes ([storage-format.md](../storage-format.md) §3), is reported with a new kind, **`save_too_big`**, naming the save and which limit it passes. Counting is by path without walking a shared listing twice: a listing's count is its entries plus the counts of the folders it names, remembered per listing, so the check costs one pass over each listing, as before.

- **Only `verify` changes.** `tidy` keeps by ID and is unchanged; it stops where it stopped before.
- **`--json`.** `problems[].kind` gains `save_too_big`; nothing else in the document changes. The formats freeze when v0.1 is published, and this is before it.
- **The two-looks rule holds for these too:** each is reported only when two looks agree, like anything else found while working out what the project needs.

## Consequences

- `verify` holds a size for each file it meets while walking saves, and a count for each folder listing: more memory per object than before, on top of the known limit that its memory grows with the number of objects.
- Three red-team proofs (`crates/bolt-vcs/tests/redteam10.rs`) join the tests, switched on.
- Still not checked, and written down as known costs rather than fixed here: how long a reader takes before it refuses a deep, narrow tree bomb (the red team stopped a debug `goto` after four and a half minutes on a chain of 24 two-entry listings; the entry limit bounds it, no document bounds the time), and how long `verify` takes on a pack of objects that each claim a huge size and decompress to it. Both go to the follow-ups list.
- [json.md](../json.md) and [terminal.md](../terminal.md) describe the new problem lines; [decision 118](118-bolt-verify.md) gets a back-link.
