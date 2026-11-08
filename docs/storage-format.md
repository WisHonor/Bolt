# Bolt Storage Format — Draft

> **Status:** draft of repository format 1, written during the design phase (2026-10-31). The byte layouts follow decisions 004–016 and 019. Test vectors (§13) are added milestone by milestone, and format 1 freezes at the v0.1 release. Until then anything here may change, and repositories made by development builds are disposable (decision 019).

**In plain words.** This document says exactly which bytes Bolt writes, so any program, or a future version of Bolt, can read a Bolt repository without guessing. Everything Bolt stores is either a piece of file content or a small structured record (a folder listing, a save, a diary entry…). Each one is named by its fingerprint, packed into pack files, and checked against its fingerprint every time it is read.

The words "must", "must not" and "should" are used as in RFC 2119.

## 1. Building blocks

Every structured record is made of these pieces. A reader must reject any input that doesn't follow them exactly, so each record has exactly one valid encoding (decision 005).

| Piece | Encoding | A reader must reject |
|---|---|---|
| `u8` | 1 byte | — |
| `u32`, `u64` | fixed width, little-endian | — |
| `i16`, `i64` | fixed width, little-endian, two's complement | values outside the field's stated range |
| `varint` | unsigned LEB128, at most 10 bytes, value at most 2^64 − 1 | non-minimal forms (a final `0x00` byte after the first byte), overflow |
| `bool` | 1 byte: `0x00` or `0x01` | any other value |
| `bytes` | a `varint` length, then that many bytes | a length beyond the input or beyond the field's limit |
| `string` | `bytes` holding valid UTF-8 | invalid UTF-8 |
| `id` | 32 raw bytes (a BLAKE3 output) | — |
| `change-id` | 16 raw bytes | — |

Sorted lists are sorted by the raw UTF-8 bytes of their key, strictly ascending, so duplicates are impossible. A record must use all of its input: trailing bytes are an error.

## 2. Fingerprints (IDs)

- **Algorithm:** BLAKE3 with a 32-byte output, declared once per repository in `.bolt/format` (§10).
- **Content** (a whole small file, or one chunk of a big file): the ID is the plain BLAKE3 hash of the raw bytes. For a whole file this is exactly what `b3sum` prints.
- **Structured records:** the ID is BLAKE3 in key-derivation mode, `derive_key(context, encoded record)`, with the record type's context string:

| Record | Context string |
|---|---|
| Folder listing | `bolt-vcs 2026-10-31 folder-listing v1` |
| Save | `bolt-vcs 2026-10-31 save v1` |
| Chunk list (reserved, unused: a chunk list is stored under its file's ID, §6) | `bolt-vcs 2026-10-31 chunk-list v1` |
| Project link | `bolt-vcs 2026-10-31 project-link v1` |
| Conflict record | `bolt-vcs 2026-10-31 conflict v1` |
| Diary entry | `bolt-vcs 2026-10-31 diary-entry v1` |
| Tag list | `bolt-vcs 2026-11-02 tag-list v1` |
| Server lines | `bolt-vcs 2026-11-02 server-lines v1` |

Context strings are constants forever, even if the product is renamed.

- **Display:** fingerprints are shown in lowercase hex; screens show the shortest unique prefix, at least 8 characters. Change IDs (§5) use their own letters.

## 3. Folder listing

One per folder (decision 005). An empty folder has a listing with zero entries.

```text
u8      version = 1
varint  entry count (at most 1,000,000)
entries, sorted by name:
  string  name
  u8      kind
  id      id
  varint  size
```

| Kind | Value | `id` points to | `size` |
|---|---|---|---|
| File | 1 | the file's content ID (big files: see §6) | file length in bytes |
| Executable file | 2 | as for File | file length in bytes |
| Symlink | 3 | content holding the link target's bytes | target length in bytes |
| Folder | 4 | a folder listing | 0 |
| Project link | 5 | a project link record (§7) | 0 |
| Conflict | 6 | a conflict record (§8) | 0 |

- A symlink's target is stored with `/` between its parts. On Windows, Bolt turns each `\` it reads in a target into `/` before storing it, as Git for Windows does, and each `/` back into `\` when it makes the link; elsewhere a target is stored exactly as read ([decision 045](decisions/045-link-targets-stored-with-slashes.md)).

- `name` must follow decision 004's always-on rules: valid UTF-8, not empty, not `.` or `..`, no `/`, no NUL, at most 765 bytes (Windows' 255 UTF-16 units; [decision 051](decisions/051-names-this-computer-cant-write-and-length-limits.md)), and not `.bolt` under any spelling Windows or macOS would treat as `.bolt` (any letter case, trailing dots or spaces, the short names `BOLT~1` to `BOLT~4`, a `:stream` suffix).
- The portability rules (differences only in letter case or Unicode normalization, reserved device names, trailing dots or spaces, characters Windows forbids) are checked when saving, not when reading: imported histories may contain such names (decision 004).
- **Limits for a whole save** ([decision 025](decisions/025-red-team-m1-rulings.md)): every path, counted from the root with `/` between names, may be at most **4,096** bytes ([decision 051](decisions/051-names-this-computer-cant-write-and-length-limits.md)); the folders reachable from a save's root listing may nest at most **4,096** levels below the root, and describe at most **10,000,000** entries in total, counting every path (a listing reached through two paths counts twice). A reader walking a save must stop with an error as soon as either limit is passed, and must not recurse once per folder level. The one exception is a save's search through folders of the save before it that are gone from disk, for entries it must keep ([design.md](design.md) §5.2). Past that earlier save's limits, that search ends without an error and keeps what it found, so a save that is over the limits can't stop every save after it. A reader that compares two saves counts each of them against the limits on its own, not the two together.

## 4. Save

```text
u8         version = 1
id         root folder listing
varint     parent count (0 to 1024)
id × n     parents, in order (the first parent is the line the user was on)
person     author
person     saver
string     message (at most 1 MiB)
change-id  change ID
varint     move-note count (at most 100,000)
move notes, sorted by `to`:
  string   from  (full path, `/`-separated)
  string   to    (full path, `/`-separated)
bytes      signature (empty = unsigned; must be empty unless `.bolt/format` lists `feature signatures`)
varint     extra-field count (at most 64)
extra fields, sorted by key:
  string   key
  bytes    value (at most 64 KiB)

person:
  string   name
  string   email
  i64      time, in milliseconds since 1970-01-01 UTC
  i16      time-zone offset in minutes (any value; imported Git commits can carry unusual offsets)
```

- Parents must be distinct.
- **Limits:** person name and email at most 4 KiB each; move-note paths (`from`, `to`) at most 4 KiB each; extra-field keys at most 256 bytes; signature at most 64 KiB. Move notes must be strictly sorted by `to` and extra-field keys strictly sorted (so no duplicates); readers reject anything else.
- **Defined extra fields:** `line` holds the name of the line the save was made on, as UTF-8 (decision 012). `git-commit` holds, as hex, the Git commit an imported save came from; `git-original-message` holds the original bytes of a Git message that couldn't be converted to UTF-8 exactly (decision 019); `git-header` holds the original header of a commit that the standard form below can't rebuild: every byte before the empty line that ends the header (decision 032). Readers keep extra fields they don't understand.
- **Rebuilding a Git commit** (decision 032). A save imported from Git gives back its commit's exact bytes:
  - With `git-header`: that field's bytes, an LF, then the message bytes.
  - Otherwise, the standard form, each line ending in an LF: `tree <hex>`; one `parent <hex>` per parent, in order; `author <name> <<email>> <seconds> <zone>`; `committer` the same way, from the saver; an empty line; then the message bytes. `<hex>` is the Git ID of the commit's tree or of the parent's commit, `<seconds>` the time in whole seconds, and `<zone>` a `+` (a `-` for a negative offset) followed by the offset's hours and minutes, two digits each (`+0530`).
  - The message bytes are `git-original-message` if the save has it, else the save's message.
  - Import keeps `git-header` exactly when the standard form doesn't give back the original bytes, and checks that the rebuilt bytes hash to the commit's ID. A tree in Git's standard form gets the same Git ID when it is written again from the save's listings; import counts the commits whose trees aren't in that form.
- **Signatures (later):** the signed bytes will be the save encoded with an empty signature field (decisions 006 and 019).

## 5. Change IDs

- A save made in Bolt: 16 random bytes from the operating system's secure random source, created with the save (decision 006).
- A save imported from Git takes its change ID from its commit, so importing a repository again gives the same saves (decision 032): the commit's first `change-id` header, written by Jujutsu, if it decodes to 16 bytes in Jujutsu's letters (`z` for 0 down to `k` for 15, in either letter case, as Jujutsu reads them); otherwise the last 16 bytes of the commit ID, in reverse order, with the bits of each byte reversed (Jujutsu's rule for commits without that header).
- A header's change ID goes to one save of an import only, so no two imported saves share one (decision 043): to the first commit in the import's fixed order with that header, and to none if the rule for commits without the header gives that ID to any commit of the import. Every other commit with the header gets the change ID that rule gives it.
- The import's fixed order is a depth-first walk from the commits the imported branches, server branches and tags and a detached `HEAD` point to, taken smallest Git ID first. From each commit it goes into its parents smallest Git ID first, skipping commits it has met already, and it puts each commit after its parents. The same repository always gives the same order.
- **Display:** each half-byte (value 0–15) becomes the letter `k` plus that value, so change IDs use only the letters `k` to `z` and never look like hex fingerprints. Screens show the shortest unique prefix, at least 4 letters.

## 6. Big files and chunk lists

A file of at least 1 MiB (1,048,576 bytes) and at most 128 GiB is cut into chunks and stored as its chunks plus a chunk list ([decision 082](decisions/082-how-big-files-are-cut-into-chunks.md)); any other file is stored as a single content object. Either way, the folder listing refers to the **file ID**, the plain BLAKE3 hash of the whole file, so IDs never depend on chunking (decision 005). Each chunk is a content object (§11's kind 1), its ID the plain BLAKE3 hash of its bytes.

```text
u8      version = 1
id      file ID
varint  file size
varint  chunk count (1 to 16,777,216)
chunks, in file order:
  id      chunk ID
  varint  chunk size (at least 1)
```

- The chunk sizes must add up to the file size.
- **Where it is stored** ([decision 083](decisions/083-a-chunk-list-is-stored-under-its-files-id.md)): in a pack, as an entry of kind 18 whose ID is the file ID (not a hash of the record), whose uncompressed size is the file size, and whose stored size is the record's length, at most 512 MiB, with compression 0. A lookup of a file's ID so finds either a whole copy (kind 1) or a chunk list (kind 18), and both are copies of the file. The context string for chunk lists (§2) is reserved and unused.
- **A reader must** refuse a chunk list entry whose compression isn't 0 or whose stored size is over 512 MiB; refuse its record unless it decodes by these rules exactly (version 1; a chunk count from 1 to 16,777,216, and no more chunks than the record's bytes can hold; no chunk size of 0; the sizes adding up to the file size; nothing after the last chunk) and names its entry's file ID and size; look for each chunk as content stored whole (kind 1), passing over a chunk list stored under a chunk's ID, so that reading a file never goes deeper than one list; check each chunk against its ID and its size in the list as it reads it; and check that the BLAKE3 hash of the bytes it rebuilt is the file ID before anything uses them.
- **Before relying on a stored chunk list** instead of storing its own, a writer must have cut the same file into exactly the chunks the list names, in order, each read and checked, or stored, by that writer in the same pass (§11); otherwise it stores its own list beside it.
- Chunking parameters are not part of the format: any chunk list that rebuilds the file is valid.

**How Bolt cuts a file** (not normative: any cut is valid, but Bolt's own is pinned by §13's vectors, so that files cut on any computer share their chunks; [decision 082](decisions/082-how-big-files-are-cut-into-chunks.md)). The cutter takes the bytes not yet cut, from the first:
- `MIN` is 16,384, `AVG` 65,536 and `MAX` 262,144. `MASK_S` is the top 17 bits of a 64-bit number (`0xFFFF800000000000`) and `MASK_L` the top 15 (`0xFFFE000000000000`).
- `GEAR` is 256 numbers of 64 bits: BLAKE3 in key-derivation mode with the context string `bolt-vcs 2026-11-08 chunk-cutter gear v1` and no key material, read through its extendable output: the first 2,048 bytes, as little-endian 64-bit numbers in order. `GEAR[0]` is `0x6c62c35066ded124` and `GEAR[255]` is `0x305bf3edcb5c381d`.
- With `n` bytes left: if `n` is at most `MIN`, the chunk is all of them. Otherwise let `end` be the smaller of `n` and `MAX`, and `normal` the smaller of `end` and `AVG`. A hash `h` starts at 0; for each position `i` from `MIN` to `end − 1`, counted from the chunk's first byte as 0, `h` becomes `(h << 1) + GEAR[the byte at i]`, both wrapping at 2^64. The chunk ends after byte `i`, so it is `i + 1` bytes long, at the first `i` where `h & MASK_S` is 0 while `i` is below `normal`, or `h & MASK_L` is 0 from `normal` on; if there is none, the chunk is `end` bytes long.
- Only files of 1 MiB up to 128 GiB (137,438,953,472 bytes) are cut; a bigger one is stored whole. Every chunk but the last is longer than `MIN`, so a cut file has at most 8,388,097 chunks, and its list stays under 300 MB, inside the limits above.

## 7. Project link

Records a Git submodule (or, later, another project) without fetching it (decision 004).

```text
u8      version = 1
string  system ("git")
string  url
bytes   commit ID in that system (20 or 32 bytes for Git)
```

A Git submodule's link has an empty `url`: its address stays in `.gitmodules`, which comes in as an ordinary file (decision 036). Readers reject any system but `git`, and a commit ID of any other length.

## 8. Conflict record (reserved for v0.2)

Drafted now so the format can hold it (decision 007); finalised before v0.2.

```text
u8      version = 1
varint  term count (odd, at least 3)
terms, alternating side, ancestor, side, …, side:
  bool    present (false = absent: deleted, or never existed)
  if present:
    u8      kind (as in §3)
    id      id
    varint  size
bool    has an in-progress version
if true:
  id      file ID
  varint  size
```

A repository that contains any conflict record lists the `conflicts` feature in `.bolt/format` (§10).

## 9. Diary entry

One per changing command (decision 010). Each entry records the complete state after the command. Tags and server lines change rarely, so they live in records of their own that the entry points to (decision 034); an entry that changes neither points to the same records as its parent, and they are stored once.

```text
u8      version = 1
varint  parent count (0 for the first entry, otherwise 1)
id      parent entry (if any)
u8      action (1 = command, 2 = undo, 3 = redo)
id      target entry (only for undo and redo: the command entry being undone or redone)
string  command: its name, such as `save` (what else was typed is not recorded, so nothing secret is)
person  who and when (as in §4)
string  Bolt version
state:
  varint  line count;    lines, sorted by name:   string name, id save
  bool    has tags;      if true: id of the tag list
  u8      position (1 = on a line, 2 = at a save), then string line name or id save
  varint  shelf count;   shelves, newest first:   string label (unique), id save
  bool    has server lines; if true: id of the server-lines list
bool    has a folder change; if true:
  id      the save the folder was switched from
  id      the save it was switched to (not the same save)
```

**The folder change** ([decision 071](decisions/071-a-step-records-its-change-to-the-folder.md)) says what the command did to the project folder, as a switch between two saves. Both start from the save the folder was on before the command, so they agree everywhere it changed nothing. The command switches the folder between two saves (for `goto`, the save it left and the one it goes to); at each path where those two differ and the command writes, or finds the folder already holding the target's version, the "to" save holds that version, which is what the folder holds there after the command, and the "from" save holds what the folder held there just before (what the command wrote over: a file or link, a folder, or nothing; or the target's version where the folder already held it). Where one of them is exactly a save that exists already (the save the folder was on, or the save it was switched to), that save is named; otherwise it is a kept save (below). Paths the command leaves unwritten (names this computer can't hold, `.git` and `.bolt` spellings, project links) keep the base's version on both sides, except inside a folder the command makes where the folder held a file, a link or nothing: there the "to" save holds only what the command writes or finds already there. A folder above a path the command writes, which it neither makes nor removes, is a folder on both sides, as the project folder held it before and after: where the base save holds no folder there (the user made it, perhaps in place of a file), it holds on each side just that side's entries below it, so undo and redo never remove or replace a folder the command only wrote inside. A command can be planned from a save other than the one the folder was on: an `undo --to --shelve` jump plans from that save with the set-aside work on it, and an `unshelf` onto another save plans from the save shelved on. Wherever such a command writes, the "from" side holds that save's version, since the folder matched it there or the command would have stopped, and so does the "to" side below a path it writes and leaves unwritten. Write-out judges a link it writes against the "to" save, and writes a placeholder for a link whose target passes through a name it wouldn't write. `goto` records the save it left and the save it went to (where unsaved work already matched the target, or it wrote inside a folder the save it left doesn't hold, a kept save holding that work or that folder instead of the save it left); `discard` and `shelf` record the kept save of what they throw away, then the current save (a kept save where they make a folder the folder didn't hold, or leave one the current save doesn't hold); `unshelf` records the current save with what the folder held at the paths the shelf changes, then the current save with the shelf's version at them; `undo` and `redo` record the folder change of the step they undo or redo, backwards for an undo; `undo --to`, as a step of its own, records what it wrote, as `goto` does. Commands that change no file (`start`, `import`, `save`, `save --amend`) record none. Undo switches from the "to" save back to the "from" save and redo from the "from" save to the "to" save, carrying all other unsaved work along, as `goto` does ([decision 070](decisions/070-undo-and-redo-step-through-the-diary.md)).

**Kept saves.** A save made only for a folder change is an ordinary save (§4). What `discard` and `shelf` throw away (the safety copies of decision 050) is one: its parent is the save the folder was on, its folder listings are that save's with the folder's own version of each path thrown away (a changed, new or deleted file or link, or a new folder). Every kept save's parent is the save the folder was on; its message names the command and the paths or shelf it was given (`discard art/hero.psd`), or is the shelf's message; its author and saver are the user, its change ID is new, and it has a `line` field when it was made on a line. Only the entry's folder change names it, or the state's shelves for a shelf.

**Undo and redo** (decision 070): an entry with action 2 or 3 names its target, a command entry (action 1) earlier on the diary. Its lines, position, tags and server lines are the target's parent's for an undo and the target's own for a redo. Its shelves are the ones before it, except that undoing a step takes off a shelf that step added (a `shelf`, or a jump made with `--shelve`) and puts back one it removed (an `unshelf`) (keeping its label unless another shelf has it, else the smallest number free), redoing them does the opposite, and `--shelve` adds the shelf it made, first. An entry made by `undo --to` for an entry outside the undo order is a command entry (action 1): its lines, position, tags and server lines are that entry's, and its shelves the ones before it, with the shelf `--shelve` made first if it set work aside.

Line names follow decision 012's rules: Unicode letters and digits, `-`, `_`, `.` and `/` only; not empty; no leading or trailing `/`, no `//`, and no `.` or `..` part. Two line names may not differ only by letter case: a reader compares the two names after Unicode's lowercase mapping (Rust's `str::to_lowercase`, which also lowercases a final capital sigma as `ς`), not after full case folding, so `ß` and `SS` are two different names, and so are `ΑΣ` and `ασ`. A position "on a line" must name one of the lines. Shelf labels are unique.

- **Limits:** line and shelf names at most 255 bytes; the recorded command at most 64 KiB; the Bolt version at most 256 bytes. At most 100,000 lines and 10,000 shelves. A folder change whose two saves are the same is rejected.
- **Shelf labels** are the numbers Bolt gives shelves, written in decimal: the smallest from 1 that no other shelf has ([decision 073](decisions/073-shelves.md)). A reader accepts any unique label.

**Tag list** (decision 034). Never empty: an entry without tags has none.

```text
u8      version = 1
varint  tag count (1 to 100,000)
tags, sorted by name:
  string  name (1 to 255 bytes)
  id      save
  string  message (at most 64 KiB)
  varint  Git tag object count (0 to 16)
  bytes × n  the original bytes of each Git tag object, from the tag's ref to the commit (each at most 1 MiB)
```

A tag made in Bolt, or imported from a lightweight Git tag, has no Git tag objects. An imported annotated tag keeps every tag object on its way to the commit, signature included, so export can recreate them exactly. Like every record, a tag list is at most 512 MiB (§11).

**Server lines** (decision 033, 034). Each server's lines as last seen; until v0.3 these are only the server branches a Git import brought in. Never empty: a server without lines is left out, and an entry without servers has no list.

```text
u8      version = 1
varint  server count (1 to 64)
servers, sorted by name:
  string  server name
  varint  line count (1 to 100,000)
  lines, sorted by name: string name, id save
```

Server names and their line names follow the line-name rules, and no two server names, nor two line names of one server, differ only by letter case.

## 10. Repository files in `.bolt/`

| Path | What it is | Normative |
|---|---|---|
| `format` | Format version and features | Yes |
| `store/packs/*.pack` | All stored objects (§11) | Yes |
| `state/head` | ID of the newest diary entry | Yes |
| `state/lock` | Lock file for changing commands | Yes |
| `state/lock-info` | Who holds the lock, for the waiting message | No |
| `state/journal` | Present only while a command rewrites the folder | Yes |
| `cache/` | Rebuildable speed-ups, sealed with a key kept in the user's own settings so that a cache Bolt didn't write for this account is ignored ([decision 053](decisions/053-file-state-cache.md)): `files`, the file-state cache (a file's kind, size, modified time, change time, file ID and content ID, below), and `change-ids`, each line's change IDs (below) | No |
| `tmp/` | Files being written | No |
| `config.toml` | Personal settings | No |

**Shape of `.bolt/` itself** ([decision 025](decisions/025-red-team-m1-rulings.md)). A shared project can arrive with links planted inside `.bolt/`, so a reader must refuse a repository unless `.bolt/`, `store/`, `store/packs/`, `state/`, `cache/` and `tmp/` are real folders (never symbolic links, Windows junctions or any other Windows reparse point), and report a repository whose `store/packs/` or `state/` is missing as damaged. A missing `tmp/` or `cache/` is made again, since neither holds anything worth keeping. Every file Bolt reads or writes in `.bolt/` is opened without following links and must be a regular file (never a pipe, device, folder or Windows reparse point), and reads are capped: `format` at most 4 KiB, `state/head` at most 65 bytes, `state/lock-info` and `config.toml` at most 4 KiB and 1 MiB. Clearing `tmp/` removes only Bolt's own temporary names (`tmp-` followed by 16 lowercase hex digits, optionally followed by `-e` and U+0301, the name under which a command briefly tests whether the drive treats `é` written as one character and as `e` plus an accent as one file).

**`format`** is UTF-8 text, one item per line:

```text
bolt-repository 1
hash blake3
```

Further lines of the form `feature <name>` list features the repository uses (for example `feature conflicts` or `feature deltas`). A reader must refuse to open a repository whose first line, `hash` line or any `feature` line it doesn't understand, and say that a newer Bolt is needed (decision 008).

**`state/head`** holds the lowercase hex ID of the newest diary entry followed by a newline. It is always replaced whole: written to `tmp/`, flushed to disk, then renamed.

**`state/lock`** is held with an operating-system lock (`LockFileEx` on Windows, `flock` elsewhere) by every command that changes the repository. Its contents mean nothing. A command that only reads never makes it: it opens it only to read, to clear `tmp/` while no command holds the lock, and skips that when it can't open it, so a project the user may read but not write still reads (the one exception, a command that finds a `.bolt` with no `format`, opens an existing one to read and write to see whether a `start` or `import` holds it, as some drives lock only a file open for writing, and a refusal there means no lock is held). Clearing `tmp/` is best effort, there and once a command that changes the repository holds the lock: a leftover that can't be removed waits for a later command, and fails nothing (only `start` and `import` stop when they can't clear it, before they store anything). An error opening or locking `state/lock` names it. Once a command holds the lock, it replaces `state/lock-info` (through `tmp/` and a rename) with `pid <n>\ncommand <command>\nstarted <unix ms>\n`; a command that finds the lock busy shows that text in its "waiting for another command" message. The text lives in a separate file because Windows doesn't let other processes read a locked file; it is only informational and may be stale.

**`state/journal`** ([decision 049](decisions/049-writing-files-one-at-a-time.md)) is written under the change lock, whole, through `tmp/`, flushed to disk and renamed, before a command starts rewriting files in the folder, and removed (the removal flushed too) after the command moves `state/head`. A switch that a crash stopped is finished by the next command that changes anything, never rolled back. A journal whose target is the entry `state/head` already names took effect: it is only removed, and a command that moves `state/head` without writing a journal of its own removes such a journal first, failing before it stores anything if it can't, so that no journal outlives the head it was finished at. The safety copies are not repeated in it: they are stored, and listed in the diary entry the journal names, before it is written. The actions are sorted to make the file one fixed sequence of bytes, not to give the order they are carried out in: `a` sorts before `a/x`, so turning folder `a` into a file means removing what is under it first, and a folder is made before what goes in it. The command that carries out a journal orders the actions itself, and checks each one against the folder as it finds it.

```text
bytes[8] "BOLTJRNL"
u8       version = 1
id       the diary entry the switch commits
id       the diary entry `state/head` named when it began
varint   action count (at most 10,000,000, and refused if more than the bytes left hold at 4 bytes each)
actions, sorted by path in byte order, no path twice:
  string   project path, `/` between names (at most 4,096 bytes)
  u8       action: 1 write file, 2 write executable, 3 write link, 4 make folder, 5 remove file,
           6 remove folder
  bool     has an expected current entry; if true:
    u8       kind: 1 file, 2 executable, 3 link, 4 folder (§3's numbers; nothing else)
    id       its content ID (32 zero bytes for a folder)
    varint   its size (0 for a folder)
  if the action is 1, 2 or 3:
    id       the new content ID (a link's: of its target)
    varint   the new size
bytes[32] BLAKE3 of everything before it
```

A journal drives writes into the user's folder, so a reader checks the checksum before anything else and refuses the whole journal if any rule here is broken. Every name in a path follows §3's always-on name rules (so a path has no empty name, no `.` or `..`, and no `.bolt` under any spelling, and doesn't start or end with `/`). Removing a file needs an expected entry of kind 1, 2 or 3, and removing a folder one of kind 4, so that what is removed is re-checked first. An expected link is still there where the path holds that link, or a regular file holding exactly its target (the same content ID and size): the placeholder Bolt writes for a link it can't make there, which a save reads as that link too ([decision 039](decisions/039-first-save-after-import.md), [052](decisions/052-links-and-executable-bit-on-windows.md)). The other way round, a link where a file was expected is a change. A command that changes anything reads it whole, with a cap of 4 GiB, checksum included; a bigger journal is not written and the command stops before changing anything. A command that only reads reads just its first 73 bytes (magic, version, target and `from`), enough to say a switch is unfinished and between which entries, and checks nothing past them ([decision 079](decisions/079-red-team-m4-rulings.md)).

**The cache key and the seal** ([decision 053](decisions/053-file-state-cache.md)). Every file in `cache/` is sealed: its body followed by 32 bytes, the keyed BLAKE3 hash of the body under the user's cache key. A file whose seal doesn't match (another account's, a copied project's from someone else, or edited) is ignored and rebuilt, never trusted and never an error. The key is the file `cache-key` in the folder of the user's own `config.toml` (`BOLT_CONFIG_DIR` if set): exactly 32 random bytes, nothing else. Bolt makes it on first use, writing a file of its own name there, readable only by the user on Unix (0600), and linking it in as `cache-key`, which never replaces a key already there, so two Bolts making it at once end with one key; only on a drive that has no hard links (the link fails with "not supported", or EPERM as Linux's FAT gives it) is it made in place, and any other failure counts as a key Bolt can't use. A key file that can't be read or isn't 32 bytes long is never replaced, since it is the user's own and replacing it silently could hide that someone changed it: the command goes on without its caches, none trusted or written, and a `cache_key_unusable` warning names the file and says to delete it. It is the user's own, so a link there is followed. With no settings folder at all, the command goes without its caches too, without a warning. Only the command line reads the key; the engine is handed it, and reads and writes no cache when it isn't, so a program using the engine never touches the user's settings, and no cache sealed with a key for one run (which no later run trusts) replaces a good one.

**`cache/files`** (not normative) is the file-state cache: what each file in the folder was when Bolt last read it, so that a file whose kind, size, modified time, change time and file ID still match needn't be read again. Sealed as above; read with a 256 MiB cap, seal included, and a bigger one is not written (the command warns instead).

```text
bytes[8] "BOLTFILE"
u8       version = 3 (a cache of another version is ignored and rebuilt)
varint   file count (refused if more than the bytes left hold at 84 bytes each)
files, sorted by project path in byte order, no path twice:
  string   project path, `/` between names (at most 4,096 bytes)
  u8       kind: 1 file, 2 executable, 3 symbolic link (§3's numbers; nothing else)
  varint   size in bytes
  i64      modified time: whole seconds since 1970-01-01 UTC, rounded down
  u32      and nanoseconds past them (below 1,000,000,000)
  i64      change time (`ctime` on Unix, `ChangeTime` on Windows): whole seconds, rounded down
  u32      and nanoseconds past them (below 1,000,000,000)
  u64      the drive's number (the device on Unix, the volume serial on Windows)
  u64      the file's number on it (the inode, or Windows' file ID): its low 64 bits
  u64      and its high 64 bits (0 but on drives with 128-bit file IDs, such as ReFS)
  bool     racy
  id       content ID
bytes[32] seal
```

An entry is trusted only if it isn't racy, the file's kind, size, modified time, change time and both numbers are the ones recorded ([decision 069](decisions/069-the-file-cache-matches-the-change-time.md): a file written over in place keeps its number but not its change time), and the file's number isn't 0, which some network and FUSE drives give every file ([decision 067](decisions/067-the-file-cache-matches-file-identity.md)): a file moved or copied onto the path is another file, so it is read. At the start of a scan Bolt writes a file in `tmp/`, takes its modified time as "now" and removes it, so the drive's own clock and resolution count. When the cache is written, an entry whose modified time isn't at least 2 seconds older than that "now" (FAT's resolution) is marked racy, and stays racy until the file is read again ([decision 053](decisions/053-file-state-cache.md)). The file is replaced whole, through `tmp/` and a rename.

**`cache/change-ids`** (not normative) lists the change IDs of each line's saves, so that a save and `history` can shorten change IDs (§5) without reading the line. Sealed as above; at most 256 MiB with the seal (a bigger one is not written).

```text
bytes[8] "BOLTCHID"
u8       version = 1
varint   line count (at most 100,000)
lines, sorted by name in byte order, no name twice:
  string   name (at most 255 bytes)
  id       the line's newest save when its IDs were read
  varint   change-ID count (refused if more than the bytes left hold)
  change-id × count   the change IDs of the saves from that save back along first parents, sorted, repeats kept
bytes[32] seal
```

A line's IDs are used only for a line whose newest save is the one recorded, or one whose first parents lead back to it: Bolt reads the saves from the newest back to the recorded one and adds their IDs. If the walk reaches the line's first save without meeting it (an undo moved the line back, say), the IDs read replace the line's. Lines the current diary entry doesn't have are dropped when the file is next written. Commands write it through `tmp/` and a rename, without the lock; a write that fails is just skipped.

## 11. Pack files

Writers write pack version 2, whose index has a fan-out table and fixed-width lines, so a reader can search it on disk; readers read versions 2 and 1 ([decision 057](decisions/057-pack-index-on-disk-and-merging.md)). The two versions differ only in the index. Each command that stores data writes one or more packs (decisions 009 and 041): each is built in `tmp/`, flushed to disk, then renamed to `store/packs/<hex of the index checksum>.pack`. The diary entry goes in the command's last pack, and `state/head` moves only after all of them are in the store.

```text
header:
  [8]     magic "BOLTPACK"
  u32     pack version = 2 (1 in packs written before milestone 3)
  u32     flags = 0
entries, one after another:
  u8      object kind
  id      id
  u64     uncompressed size
  u64     stored size
  u8      compression (0 = none, 1 = zstd)
  stored bytes
index, version 2:
  u64     entry count
  u32 × 256  fan-out: number i is how many IDs have a first byte ≤ i
  id × count  the IDs, strictly ascending
  lines, one per ID in the same order, 25 bytes each:
    u8      object kind
    u64     offset of the entry's header
    u64     uncompressed size
    u64     stored size
index, version 1:
  varint  entry count
  entries, sorted by id:
    id      id
    u8      object kind
    u64     offset of the entry's header
    varint  uncompressed size
    varint  stored size
trailer:
  u64     offset of the index
  [32]    BLAKE3 hash of the index bytes
  [8]     magic "BOLTPEND"
```

| Object kind | Value |
|---|---|
| Content (a whole small file, or a chunk) | 1 |
| Folder listing | 16 |
| Save | 17 |
| Chunk list, stored under its file's ID (§6) | 18 |
| Conflict record | 19 |
| Diary entry | 20 |
| Project link | 21 |
| Tag list | 22 |
| Server lines | 23 |

In version 2, ID `i` is at `index + 1,032 + 32 × i` and its line at `index + 1,032 + 32 × count + 25 × i`, where `index` is the offset of the index, so a reader finds an object by a binary search of the IDs from `fan-out[b − 1]` (0 when `b` is 0) up to `fan-out[b]`, `b` being the ID's first byte.

A reader must:
- check both magic values, a pack version of 1 or 2, flags of 0, and that every offset and size it uses stays inside the file;
- in version 2, refuse an entry count above the limit below, an index that isn't exactly `8 + 1,024 + 57 × count` bytes, or a fan-out that decreases or doesn't end at the count, before reading any ID; then read only the IDs and lines it needs, by position, and check each line it reads (a known kind, a size within the limits below, its entry inside the entry area), so an open pack costs the same memory whatever its count. Reading every line in order (as `verify` does) also checks that the IDs ascend and that each sits in its fan-out range. The index checksum is checked by `verify`, not on every open: every object read is checked against its own ID anyway;
- in version 1, check the index checksum, and read the index a piece at a time, never whole: refuse an entry count above the limit below, or an index whose length doesn't fit its count, before reading any line, then check each line as it is read (a known kind, IDs strictly ascending, its entry inside the entry area), and that no two entries overlap;
- check that each entry's header matches its index line;
- decompress zstd data without ever producing more than the recorded uncompressed size, and require the zstd frame's own content size, if present, to match;
- check the fingerprint of every object it reads: plain BLAKE3 for content, the record's context string for the other records, and for a chunk list §6's checks, the file it rebuilds checked against the file ID;
- treat the same ID found in two packs as the same object (packs can overlap while being merged), and when one pack's copy fails for any reason (a failed check, a read error, too little memory), try the other packs' copies before giving up, so a damaged or planted pack never hides a good copy; a pack whose copy failed a check is reported as damaged;
- search the packs one at a time ([decisions 061](decisions/061-pack-copy-order-and-lazy-opening.md), [066](decisions/066-a-pack-bolt-cant-open-for-another-reason.md) and [078](decisions/078-a-pack-bolt-cant-open-gets-its-own-warning.md)): first the pack that held the last object read if it is open, then the other open packs, then the closed ones in order of name; open a pack only when first searching it, and keep at most 16 open, closing the least recently used first, however many packs hold the object; pass over a pack that can't be opened or searched and keep its error, reporting the pack as damaged when the error is damage, and in a `pack_unreadable` warning naming it and why, once per command, when it fails for another reason (permission denied, say), so a lookup that reads a good copy elsewhere succeeds and one that finds nothing, or no copy that works, fails with that error, which names the pack; a lookup that passed over several keeps the error that matters most: one that isn't damage (the object may be fine there), then damage, then a pack gone; a pack that failed to open isn't opened again until the packs are listed again, so a junk pack costs one try, not one each lookup; a pack listed but gone when it is opened (merged into another) sends the reader to list the packs again, once per lookup;
- read the first copy found when it records at most 1 MiB, holding only that copy whole; when it records more, or fails, search every pack and try the copies smallest recorded size first, streamed (content several packs hold is checked before it is streamed out), ties in pack order (a copy already checked in this process may go first), so a planted copy that claims a huge size is read last, and one found first costs at most a 1 MiB read; only when no copy works does the read fail, naming a read error or lack of memory if there was one (the object may still be fine), else the damage.

Before skipping an object because the store already holds it, a writer must read a stored copy of the same kind in full and check its fingerprint (once per process is enough). The writer knows the object's size (a file's length, a link's target, an encoded record), so it passes over a copy that records another size without reading it, and reports that copy's pack as damaged. If no copy passes, it stores its own; a pack it can't search because it is damaged doesn't stop it either (a second copy is harmless), while one it can't read for another reason does, with that error, since it may hold a good copy. For a file, a whole copy (kind 1) and a chunk list (kind 18) are both copies, and a chunk list counts as good only as §6 says.

Each entry's header is a fixed 50 bytes, so a writer that streams an object can fill the header in after writing its data.

Uncompressed sizes are at most 2^40 bytes (1 TiB); structured records are at most 512 MiB uncompressed. A chunk list's stored size is at most 512 MiB, and its uncompressed size is its file's.

**Merging.** A pack is small when it is a regular file under 64 MiB on disk and the uncompressed sizes its index records total at most 256 MiB, counting each chunk list at its stored size ([decision 083](decisions/083-a-chunk-list-is-stored-under-its-files-id.md)), which is judged from the index alone, without decompressing anything. Once a command has committed (moved `state/head`, and for `start` and `import` written `.bolt/format`) and more than 32 packs are small, it merges the small ones, still under the change lock ([decisions 057](decisions/057-pack-index-on-disk-and-merging.md) and [062](decisions/062-when-packs-are-merged.md)). Every other pack is left as it is, and isn't counted. The packs are taken in order of name, and each object in order of ID; each is read and checked as a reader checks it (header, size, fingerprint; a chunk list by its structure, §6, without rebuilding its file), and then its stored bytes and compression are copied as they are into new version 2 packs, written as any command writes its packs (so a merge rewrites version 1 packs as version 2). A pack with a copy that fails its check, or that can't be opened or read (damage, an I/O error, too little memory), is left as it is, a `pack_not_merged` warning names it, and the merge carries on; only a failure to write the new packs stops it. Once more than 32 packs are under 64 MiB on disk, each one's index is read to judge whether it is small, and one that can't be judged is named so too, even when 32 or fewer of the rest are small and nothing is merged. A damaged pack that is kept is read again at each merge, and the 256 MiB bound limits that cost. Only once every new pack is durable and renamed into the store are the old packs removed, one at a time, never one whose name is a new pack's, compared ignoring letter case. A pack that can't be removed stays as a second copy of its objects, with a warning. A stop at any point leaves every object readable: before the rename, the old packs are untouched; after it, objects in two packs are one object. A merge that fails doesn't fail the command, which has happened; it is a warning, and the next command that commits tries again.

**Tidying** ([decision 077](decisions/077-tidy-and-the-undo-window.md)). `bolt tidy`, under the change lock, first marks what must be kept: everything the current diary entry's state reaches (lines, the save the project is at, shelves, tags and server lines, each save with all its parents, its folder listings, contents, project links and the records those name), every diary entry from `state/head` back along parents and, for every diary end (an entry no other names as its parent) whose `who` time is within the undo window, that end and its diary back along parents ([decision 079](decisions/079-red-team-m4-rulings.md)), and, for every such entry whose `who` time is within the undo window, everything the states of that entry and of its parent reach and both saves of its folder change, and for an undo or redo entry the same for its target (the states of the target and of its parent, and the target's folder change). A pack whose file was modified less than 24 hours ago is left as it is, though its index is read too. Every pack's index is checked against its checksum before tidying judges what it holds (a version 2 reader skips that check, since every object it reads is checked anyway, but tidying decides what to delete from the index alone); a pack that can't be opened, or whose index can't be read or fails its checksum, stops tidying before it removes anything, since it could hold the only chunk list that names chunks in other packs ([decision 083](decisions/083-a-chunk-list-is-stored-under-its-files-id.md)). If any pack holds a conflict record, which this version doesn't follow, tidying stops too. Marking follows chunk lists: for every file it keeps, each chunk list stored under the file's ID is read and checked by its structure (§6), and every chunk it names is kept; so is every chunk that a chunk list in a pack modified less than 24 hours ago names. Of the others, a pack with nothing to keep is removed without being read, first, before anything is written, so tidying frees space even on a full disk; a version 2 pack that isn't small and holds only what is kept stays; every other pack (one holding something not kept, a small one, or one in version 1) is rewritten: each kept object in it is read and checked as merging checks it and its stored bytes copied as they are into new version 2 packs, written as any command writes its packs. The copy reads the index again, so a pack whose copy doesn't take as many kept objects as its checked index named (a failing disk can read it differently) is left as it is, with a `pack_not_tidied` warning. Only once every new pack is durable and renamed into the store are the old packs it rewrote removed, one at a time, never one whose name is a new pack's, compared ignoring letter case. If anything that must be kept can't be read, tidying stops before it removes anything.

**Index limits.** A pack holds at most 16,777,216 (2^24) objects, and a writer must not put more in one pack ([decision 028](decisions/028-red-team-m1-session-2-rulings.md)). A command that needs more writes several; Bolt's writer starts a new pack once one holds 2^20 objects, which keeps the index it holds in memory near 100 MB ([decision 041](decisions/041-several-packs-per-command.md)). In version 2 the index of a pack with `count` objects is exactly `1,032 + 57 × count` bytes. In version 1 an index line is 43 to 61 bytes (its two sizes are varints of 1 to 10 bytes), so the index is the count's own varint followed by 43 × `count` to 61 × `count` bytes, with nothing after the last line.

## 12. Project files (outside `.bolt/`)

- **`bolt.toml`** (TOML; decisions 008 and 016). Draft keys: `preset`, `chunk-files-over`, `ask-before-saving-over`, and `[[rule]]` tables with `files`, `lockable`, `conflict-markers` and `moves-with-asset`. Unknown keys produce a warning. Final key names are settled when presets are built (milestone 6).
- **`.boltignore`** files work like `.gitignore` files: one at the project root, and optionally more in subfolders, each using the `.gitignore` pattern syntax relative to its own folder. `.bolt/` and `.git/` are always ignored (decision 019); where the drive ignores letter case (as Windows' and macOS's do by default) that includes `.git` in any letter case (`.GIT`), because there it is Git's folder too. Any `.bolt` spelling other than the project's own folder is left out the same way, named in the same warning ([decision 059](decisions/059-bolt-spellings-left-out-like-git-spellings.md)). Any other entry whose name Git treats as `.git`, as the import judges it (a `.git` file, `.GIT` where letter case counts, `git~1`, spellings with a character macOS ignores), is left out too, in every folder and on every system. A `git_names_left_out` warning names what was left out: the ones in the folder that no ignore rule keeps out, and those the save it builds on held, each once (decision 043). A folder below the top that holds an entry named `.git` (a folder, or a worktree's or submodule's file; `.GIT` too where the drive ignores letter case) or a folder named `.bolt` is a repository or project of its own, and is left out as if an ignore rule kept it out: only what the save it builds on holds there is kept, and a `nested_projects_left_out` warning names those no ignore rule keeps out (decision 046). A `.boltignore` must be a regular file of at most 1 MiB; a link, pipe or larger file stops the save with an error (decision 025). Decisions 028, 029 and 038 add:
  - **`.gitignore` files are read too** (decision 038), as if each were a `.boltignore` in its folder, with that folder's `.boltignore` lines after its `.gitignore` lines, so they can add to Git's rules or undo them. The limits below count both kinds. A `.gitignore` that isn't a regular file (a link, folder or special file) is skipped with a `gitignore_skipped` warning and its rules don't apply, as Git skips a linked one; any other problem with it (over 1 MiB, past a limit, or unreadable) stops the save, as for a `.boltignore`. Git's personal lists apply too (decision 047), as if their lines came first in the top folder's `.gitignore`: the user's own list, then `.git/info/exclude`, so the project's own lines win, as in Git, and both count as Git's rules wherever a save asks what Git alone would leave out. The user's own list is the file `core.excludesFile` names in the user's own Git settings (`$XDG_CONFIG_HOME/git/config`, then `~/.gitconfig`, which wins; `~/` expanded; a relative path from the project's top; set to nothing, it names no list, as in Git), else `$XDG_CONFIG_HOME/git/ignore` (`~/.config/git/ignore` when that variable isn't set); a link to it is followed, since it is the user's own file, and `core.excludesFile` is never taken from the repository's `.git/config`. Plain `[include]` sections of the user's settings are followed as Git follows them (at most 10 deep); `[includeIf]` sections aren't; a list that lands inside the project is read there like a `.gitignore`, without following a link; a path starting `~user/` stops the save ([decision 060](decisions/060-user-git-settings-includes-and-paths.md)). `.git/info/exclude` is read only where the top folder holds a real `.git` folder, and like a `.gitignore`: a link or special file is skipped with a `gitignore_skipped` warning, and so is the list when `.git` or `.git/info` is a link (decision 060). Both count toward the limits below; a list that isn't there is none, and any other problem reading one stops the save.
  - **Rules keep out only what isn't saved yet** (decision 038). A file or link in the save the new one builds on is saved again even when a rule matches it, as in Git; it leaves history only when it is deleted from the folder. A save that takes a change to one, to its contents or its kind, warns (`ignored_changes_saved`, decision 043). The name must match exactly, and the entry must still be a folder if it was saved as one and not a folder if it wasn't. So a save looks inside an ignored folder where that save holds the folder, keeps only the names saved there, at every depth, and reads no ignore file inside it. Before any rule applies, a save keeps exactly as they were the project links, names this system can't hold and names that are one file on the drive, without looking at what the folder holds there, and a link Git wrote as a file holding exactly its target ([design.md](design.md) §5.2, decisions 036 and 039). Neither rule keeps a name Git treats as `.git`: a save drops any the save it builds on holds (decision 043), except inside a folder kept as it was, which keeps what that save holds in it; writing files out (milestone 3) checks every name again.
  - **Read and matched exactly as Git does** (decisions 028 and 029). Bolt follows Git's own code for this (`dir.c` and `wildmatch.c`), quirks and all, as Git 2.52 and later have it, so the files Bolt keeps out are the ones Git would. No line is refused: a line Git can't use matches nothing, as in Git.
    - A file is read as bytes, and needn't be UTF-8. A UTF-8 byte order mark at its start is skipped. Lines end at each LF, and one CR before it is dropped, on the last line too; a NUL ends a line early. A line starting with `#` is a comment, and blank lines are skipped. Spaces at the end of a line are removed unless a `\` escapes them: `\\ ` loses its space and `\ ` keeps it, and after a `\` that ends the line none are removed.
    - `!` at the start takes back what an earlier line ignored. A `/` at the start ties the pattern to its file's folder, and a `/` at the end makes it match folders only. A pattern with no other `/` matches a name at any depth below the folder; one with a `/` matches the path from the folder.
    - Git matches bytes. `*` matches any run of bytes but `/`, and `?` one byte but `/`, so never a character beyond ASCII, which takes two or more. `[…]` matches one byte but `/`: its members are bytes, ranges such as `a-z`, and classes such as `[:alpha:]`; `[!…]` and `[^…]` match a byte that isn't a member. `**` as a whole folder name matches any run of bytes, `/` included: `**/x` matches `x` at any depth, `a/**/b` matches `a/b` and `a/x/y/b`, and `a/**` everything inside `a`; any other `**` is a `*`. `\` makes the next byte plain, a `[`, `*` or `\` included, and `{` and `}` are always plain.
    - The last line that matches decides, and the lines of the nearest folder's file come first, so a folder can take back what a folder above it ignores. A line can't take back a file inside an ignored folder, whose contents stay out as in Git (apart from what the save before holds; see above).
    - Some lines match nothing: one ending in a `\` that escapes nothing, one with a `[` that never closes or names a class Git doesn't know (`[[:foo:]]`), and one whose class would have to take a `/` (`a[/]b`). A range that runs backwards (`[z-a]`) is empty.
    - Git before 2.52 read a `**` right after the literal start of a pattern with a `/` (`foo**/bar`) as a whole folder name; Bolt reads it as 2.52 and later do, as a `*`.
    - Matching never recurses: a line is matched against a path in time at most proportional to the line's length times the path's.
  - **Letter case.** Where the drive ignores letter case (as Windows' and macOS's do by default), patterns match without regard to it, as Git does with `core.ignorecase`, which Git sets by testing the drive just as Bolt does: `*.env` ignores `PROD.ENV`, and `secrets/` ignores `Secrets/`. Elsewhere patterns match exactly, as in Git. As in Git, only ASCII letters are folded, and a capital letter that a `\` escapes or that stands alone in a class (`[B]`, unlike `[A-Z]`) keeps its case, so there it matches nothing.
  - **Unicode form.** Where the drive takes an accented letter written as one character and as a letter plus a separate accent for the same name (as macOS's does), names are compared in composed form (NFC), as Git does with `core.precomposeunicode`. Patterns are used as written, so there a line in decomposed form matches nothing.
  - **Braces.** `{` and `}` are ordinary characters, as in Git: `{a,b}` is not a choice, and `\{` means the same as `{`.
  - **Limits.** The ignore files in force at one folder (its `.gitignore` and `.boltignore`, and those of the folders above it) total at most 1 MiB, and the files one save takes rules from total at most 16 MiB, counting each file whose bytes Bolt has read. Bolt checks both before it reads any rule from a file. Past either limit the save stops, saves nothing, and the error names the file that went over. A file refused before it is counted (one over 1 MiB, say) still costs a read of up to 1 MiB, but the save stops there, so no later file is read. The scan stops at the first ignore file that stops the save, whatever the problem.

## 13. Test vectors

The first implementation generates, and the owner reviews, vectors in `crates/bolt-vcs-core/tests/vectors/`:
- for each record type, a few hand-checkable inputs with their exact encoded bytes (hex) and their IDs;
- plain BLAKE3 checked against the official BLAKE3 test vectors;
- a small reference pack and a `.bolt/format` file;
- invalid encodings that every reader must reject.

Vectors are added milestone by milestone, for the records each milestone builds; format 1 freezes at the v0.1 release (decision 019).

**The files so far.** All are in `crates/bolt-vcs-core/tests/vectors/`.
- **Milestone 1:**
  - `listing.json`: folder listings (§3), three vectors;
  - `save.json`: saves (§4), three vectors;
  - `diary.json`: diary entries (§9), a `minimal` one and a `full` one;
  - `pack.json`: packs (§11), two vectors.
- **Milestone 2:**
  - `tag_list.json`: a tag list (§9) with a lightweight tag and an annotated one that keeps a Git tag object;
  - `server_lines.json`: server lines (§9), one server with two lines;
  - `project_link.json`: project links (§7), a SHA-1 submodule and a SHA-256 one;
  - `diary.json`: the `full` entry now points at the tag list and the server lines of the two files above, so it changed; the `minimal` entry is byte for byte what milestone 1 wrote;
  - `shattered-prefix.bin`: not a record. It is the first 320 bytes of one of the two PDFs of SHAttered, the first public SHA-1 collision. The Git reader's SHA-1 must refuse it as a collision attack, and must accept it with one bit changed.
- **Milestone 3:**
  - `listing.json`: a fourth vector, `long-name`, a file whose name is 600 bytes (200 characters of three bytes each), past the 255 bytes names had until [decision 051](decisions/051-names-this-computer-cant-write-and-length-limits.md); its length is a two-byte varint. The first three vectors are byte for byte what milestone 1 wrote.
  - `pack_v2.json`: a version 2 pack (§11) of the same two objects as `pack.json`'s `two-objects`, which the writer must write byte for byte. `pack.json` stays as milestone 1 wrote it, in version 1, which readers must still read.
  - `change_ids.json`: a sealed `cache/change-ids` (§10) with two lines, its key, its body and the sealed bytes.
  - `file_cache.json`: a sealed `cache/files` (§10) with two files, one racy and one from before 1970, its key, its body and the sealed bytes.
  - `journal.json`: a `state/journal` (§10) with one action of each kind but "write executable", and its checksum.
- **Milestone 4:**
  - `diary.json`: the safety-copy list became the folder change ([decision 071](decisions/071-a-step-records-its-change-to-the-folder.md)), so the `full` entry (an undo), which held two safety copies, now records a folder change, and its bytes and ID changed; the `minimal` entry, which records none, is byte for byte what milestone 1 wrote. The diary's tests reject a folder change from a save to itself.
- **Milestone 5** (written from §6's text before the code, [decisions 082](decisions/082-how-big-files-are-cut-into-chunks.md) and [083](decisions/083-a-chunk-list-is-stored-under-its-files-id.md)):
  - `chunking.json`: Bolt's cutter (§6). `GEAR[0]`, `GEAR[1]` and `GEAR[255]`, and the chunk sizes it gives for four inputs, the "noise" being BLAKE3's extendable output in key-derivation mode with the context `bolt-vcs 2026-11-08 chunk-cutter test v1`: 3 MiB of noise (40 chunks); the same with one byte, `0x42`, inserted at offset 1,000,000 (40 chunks, the 12th one byte longer and every other the same); 1 MiB of zeros (four chunks of 262,144); and 16,385 bytes of noise (one chunk).
  - `chunk_list.json`: chunk lists (§6) with their file IDs: `hello-world` (the 11-byte file `hello world` cut into `hello ` and `world`) and the 3 MiB of noise above; and eight that every reader must refuse (version 2, no chunks, a chunk of 0 bytes, sizes that don't add up, a trailing byte, a non-minimal varint, a count past the record's bytes, a count over 16,777,216).
  - `pack_chunked.json`: a version 2 pack holding the two chunks of `hello world`, stored raw, and its chunk list, stored under the file's ID, which the writer must write byte for byte.

## 14. Open points

- Final `bolt.toml` key names (milestone 6).
- Conflict record details (before v0.2).
- Delta storage for small files (a later `feature deltas`).
- Splitting giant folder listings (a later listing version).
- The signature format (later).
- ~~A limit on path length~~ (decided: 4,096 bytes, and names at most 765 bytes; [decision 051](decisions/051-names-this-computer-cant-write-and-length-limits.md)).