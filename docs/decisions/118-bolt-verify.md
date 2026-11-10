# 118 — `bolt verify`

- **Status:** Accepted
- **Date:** 2026-11-09
- **Agenda topic:** none (milestone 6 planning)
- **Decided by:** Claude, at the owner's request while the owner was away. The owner can reverse any part.
- **Refines:** [decision 019](019-review-clarifications.md) (the check behind `bolt verify` exists from milestone 1; the command ships in milestone 6); [decision 057](057-pack-index-on-disk-and-merging.md) (the index checksum is checked by `verify`); decisions [066](066-a-pack-bolt-cant-open-for-another-reason.md) and [078](078-a-pack-bolt-cant-open-gets-its-own-warning.md) (`verify` is where every pack gets checked); [decision 083](083-a-chunk-list-is-stored-under-its-files-id.md) (what checking the store does with chunk lists); [decision 005](005-object-identity.md) (telling "not downloaded" from lost)

## Context

`bolt verify` is Bolt checking itself (principle 4). An internal check has existed since milestone 1: it opens every pack, checks a version 2 index against its checksum, and reads every object against its fingerprint, rebuilding each chunked file. Only tests call it. It looks at bytes alone: a store with a pack deleted passes, since every pack that is left is intact. It reads records without decoding them, it calls a pack that a merge removed mid-run damaged, and it never looks at `head`.

Everyday commands open packs only when a lookup reaches them, skip the index checksum, and pass over a pack they can't read with a warning (decisions 057, 066, 078). So a pack nothing happens to need stays unchecked until `verify`.

## Options

**What it checks**
- **A. The bytes only:** every pack, index and object.
- **B. The bytes, and that the project has everything it needs:** from `head`, through the diary, the lines, shelves and tags, down to every file's contents. `bolt tidy` already works this list out to decide what to keep.

**While other commands run**
- **A. Take the lock.** Nothing moves under it, but a check of a big store can run for minutes with every save waiting.
- **B. No lock,** like every command that only reads; look again where another command may have moved a pack.

## Decision

**Bytes and needs (B), without the lock (B).** `bolt verify` changes nothing and repairs nothing.

### What is checked

1. **Every pack** in `.bolt/store/packs`: it opens as a pack; its index reads in full, in order, and matches its checksum (decision 057); and every object in it reads back as what its index line says:
   - file contents and chunks decompress to their recorded size and match their fingerprint;
   - every record matches its fingerprint and decodes as its kind (a folder listing, a save, a diary entry, a tag list, server lines, a project link). Conflict records, which v0.1 never writes, are checked by fingerprint only;
   - every chunk list has the structure storage-format.md §6 asks, and the file it builds from its chunks matches the file's ID (decision 083).

   A pack that can't be opened or read is a problem that names it and says why, damage and other reasons apart (decisions 066 and 078). One bad object doesn't end the check: the rest of the pack is still checked.
2. **What the project needs.** Starting from `state/head`: every diary entry back to the first; everything the current state reaches (each line, the save the project is at, shelves, tags, server lines, with all their history); and what undo needs for the steps inside the undo window. This is what `bolt tidy` keeps (decisions 077 and 079), worked out by the same code. Every record on the way must read and decode, and every file's contents, chunk list and chunks must be in some pack. Something missing is a problem that names the object and what needs it.
   - A save that only an old step needs, and that isn't there, is not a problem: an earlier `tidy` freed it, and that step just can't be undone any more ([decision 123](123-follow-ups-done-or-moved-past-v0.1.md), RT7-2).
   - **A lost `head`** is a problem of its own, with the hint decision 075 gives (`bolt undo --list`, then `bolt undo --to`); the first check still runs.
   - **A switch left unfinished** is told as it is to `status`, as a warning, not a problem.
3. **Not checked:** the caches (Bolt rebuilds them), `tmp/`, and the files in the folder (`bolt status --full` reads those).

### While other commands run

`verify` takes no lock. It lists the packs, checks them, lists them again, and checks any new ones, until a listing brings nothing new. A pack that is gone when its turn comes was merged or tidied away, which is not a problem; what the project needs is then looked for in the packs that are there at the end. So a save, a merge or a `tidy` running beside it can't make it report damage that isn't there.

### What it says

- **Nothing wrong:** `Verified <project>: no problems found.`, then one line per check with what it covered (objects, packs and bytes; saves, lines and shelves; diary entries). Exit code 0.
- **Problems:** `Checked <project>: N problems found.`, then each problem: the object's fingerprint or the pack's name, what is wrong, and for something the project needs, which save and path need it. At most 20 are listed per kind on screen, then how many more; `--json` holds up to 1,000 and the full count. Exit code 1.
- The hint is honest: v0.1 can't repair. It says that saves which don't use the damaged data still work, and to keep any other copy of the project safe.
- A progress bar shows on stderr in a terminal, by bytes of pack read ([decision 120](120-the-terminal-ui.md)).
- The check marks are `✓` and `✗` where the terminal shows them, `+` and `x` elsewhere.

### What it doesn't have

No `--repair`, `--check`, `--full` or `--report`: there is no server to fetch a good copy from before v0.3. "Known but not downloaded" (decision 005) can't happen in v0.1, where everything is local, so everything missing is a problem; `verify` learns the difference when syncing arrives.

## Consequences

- The internal check gains decoding of records, takes the repository rather than a path, and goes on past a bad object; tests keep using it.
- `tidy`'s marking is shared with `verify`, so the two agree on what a project needs.
- A full check reads every pack once, and the chunks of each chunked file once more to rebuild it. On a big store that takes minutes; the bar says how far it is.
- The canvas's "in N saves" column isn't built: naming one save and path that need a damaged object needs no extra index, and counting them all would.
- [json.md](../json.md) gains the document, [security.md](../security.md) a note that `verify` meets hostile packs like any reader, and the red team attacks it.
