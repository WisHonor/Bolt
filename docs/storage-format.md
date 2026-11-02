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
| Chunk list | `bolt-vcs 2026-10-31 chunk-list v1` |
| Project link | `bolt-vcs 2026-10-31 project-link v1` |
| Conflict record | `bolt-vcs 2026-10-31 conflict v1` |
| Diary entry | `bolt-vcs 2026-10-31 diary-entry v1` |

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

- `name` must follow decision 004's always-on rules: valid UTF-8, not empty, not `.` or `..`, no `/`, no NUL, at most 255 bytes, and not `.bolt` under any spelling Windows or macOS would treat as `.bolt` (any letter case, trailing dots or spaces, the short names `BOLT~1` to `BOLT~4`, a `:stream` suffix).
- The portability rules (differences only in letter case or Unicode normalization, reserved device names, trailing dots or spaces, characters Windows forbids) are checked when saving, not when reading: imported histories may contain such names (decision 004).
- **Limits for a whole save** ([decision 025](decisions/025-red-team-m1-rulings.md)): the folders reachable from a save's root listing may nest at most **4,096** levels below the root, and describe at most **10,000,000** entries in total, counting every path (a listing reached through two paths counts twice). A reader walking a save must stop with an error as soon as either limit is passed, and must not recurse once per folder level. A reader that compares two saves counts each of them against the limits on its own, not the two together.

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
- **Defined extra fields:** `line` holds the name of the line the save was made on, as UTF-8 (decision 012). `git-commit` holds, as hex, the Git commit an imported save came from; `git-original-message` holds the original bytes of a Git message that couldn't be converted to UTF-8 exactly (decision 019). Readers keep extra fields they don't understand.
- **Signatures (later):** the signed bytes will be the save encoded with an empty signature field (decisions 006 and 019).

## 5. Change IDs

- 16 random bytes from the operating system's secure random source, created with the save (decision 006).
- **Display:** each half-byte (value 0–15) becomes the letter `k` plus that value, so change IDs use only the letters `k` to `z` and never look like hex fingerprints. Screens show the shortest unique prefix, at least 4 letters.

## 6. Big files and chunk lists

Files at or above the chunking threshold (in `bolt.toml`; 1 MB by default) are cut with FastCDC (decision 009) and stored as chunks plus a chunk list. Smaller files are stored as a single content object. Either way, the folder listing refers to the **file ID**, the plain BLAKE3 hash of the whole file, so IDs never depend on chunking (decision 005).

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
- After reassembling a file, a reader must check that its BLAKE3 hash equals the file ID before using it.
- Chunking parameters are not part of the format: any chunk list that reassembles to the right bytes is valid.
- Bolt finds a file's chunk list through an index from file ID to chunk list, kept in `cache/` and rebuilt from the chunk lists themselves, which contain the file ID (decision 019).

## 7. Project link

Records a Git submodule (or, later, another project) without fetching it (decision 004).

```text
u8      version = 1
string  system ("git")
string  url
bytes   commit ID in that system (20 or 32 bytes for Git)
```

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

One per changing command (decision 010). Each entry records the complete state after the command.

```text
u8      version = 1
varint  parent count (0 for the first entry, otherwise 1)
id      parent entry (if any)
u8      action (1 = command, 2 = undo, 3 = redo)
id      target entry (only for undo and redo: the command entry being undone or redone)
string  command, as typed (values that look like secrets are redacted)
person  who and when (as in §4)
string  Bolt version
state:
  varint  line count;    lines, sorted by name:   string name, id save
  varint  tag count;     tags, sorted by name:    string name, id save, string message
  u8      position (1 = on a line, 2 = at a save), then string line name or id save
  varint  shelf count;   shelves, newest first:   string label (unique), id save
  varint  remote count;  remotes, sorted by name: string remote name,
                           varint line count, lines sorted by name: string name, id save
varint  safety-copy count
safety copies, sorted by path:
  string  path
  u8      kind (as in §3)
  id      id
  varint  size
```

Line names follow decision 012's rules: Unicode letters and digits, `-`, `_`, `.` and `/` only; not empty; no leading or trailing `/`, no `//`, and no `.` or `..` part. Two line names may not differ only by letter case, and a position "on a line" must name one of the lines. Shelf labels are unique. The remote list stays empty until v0.3.

- **Limits:** line, tag and shelf names at most 255 bytes (remote names too); tag messages at most 64 KiB; the recorded command at most 64 KiB; the Bolt version at most 256 bytes; safety-copy paths at most 4 KiB. At most 100,000 lines (in the state and per remote), 100,000 tags, 10,000 shelves, 64 remotes and 1,000,000 safety copies. Safety copies must be strictly sorted by path; readers reject anything else.

## 10. Repository files in `.bolt/`

| Path | What it is | Normative |
|---|---|---|
| `format` | Format version and features | Yes |
| `store/packs/*.pack` | All stored objects (§11) | Yes |
| `state/head` | ID of the newest diary entry | Yes |
| `state/lock` | Lock file for changing commands | Yes |
| `state/lock-info` | Who holds the lock, for the waiting message | No |
| `state/journal` | Present only while a command rewrites the folder | Yes |
| `cache/` | Rebuildable speed-ups | No |
| `tmp/` | Files being written | No |
| `config.toml` | Personal settings | No |

**Shape of `.bolt/` itself** ([decision 025](decisions/025-red-team-m1-rulings.md)). A shared project can arrive with links planted inside `.bolt/`, so a reader must refuse a repository unless `.bolt/`, `store/`, `store/packs/`, `state/`, `cache/` and `tmp/` are real folders (never symbolic links, Windows junctions or any other Windows reparse point), and report a repository whose `store/packs/`, `state/` or `tmp/` is missing as damaged. Every file Bolt reads or writes in `.bolt/` is opened without following links and must be a regular file (never a pipe, device, folder or Windows reparse point), and reads are capped: `format` at most 4 KiB, `state/head` at most 65 bytes, `state/lock-info` and `config.toml` at most 4 KiB and 1 MiB. Clearing `tmp/` removes only Bolt's own temporary names (`tmp-` followed by 16 lowercase hex digits).

**`format`** is UTF-8 text, one item per line:

```text
bolt-repository 1
hash blake3
```

Further lines of the form `feature <name>` list features the repository uses (for example `feature conflicts` or `feature deltas`). A reader must refuse to open a repository whose first line, `hash` line or any `feature` line it doesn't understand, and say that a newer Bolt is needed (decision 008).

**`state/head`** holds the lowercase hex ID of the newest diary entry followed by a newline. It is always replaced whole: written to `tmp/`, flushed to disk, then renamed.

**`state/lock`** is held with an operating-system lock (`LockFileEx` on Windows, `flock` elsewhere) by every command that changes the repository. Its contents mean nothing. Once a command holds the lock, it replaces `state/lock-info` (through `tmp/` and a rename) with `pid <n>\ncommand <command>\nstarted <unix ms>\n`; a command that finds the lock busy shows that text in its "waiting for another command" message. The text lives in a separate file because Windows doesn't let other processes read a locked file; it is only informational and may be stale.

**`state/journal`** is written (whole, through `tmp/` and a rename) before a command starts rewriting files in the folder, and removed after the command commits. It records the target diary entry, each planned file action (path, expected current ID, new ID) and the safety copies taken, followed by a BLAKE3 checksum of the bytes before it. If it exists when Bolt starts, Bolt finishes or rolls back the interrupted command (decision 010).

## 11. Pack files

Each command that stores data writes one pack (decision 009): it is built in `tmp/`, flushed to disk, then renamed to `store/packs/<hex of the index checksum>.pack`.

```text
header:
  [8]     magic "BOLTPACK"
  u32     pack version = 1
  u32     flags = 0
entries, one after another:
  u8      object kind
  id      id
  u64     uncompressed size
  u64     stored size
  u8      compression (0 = none, 1 = zstd)
  stored bytes
index:
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
| Chunk list | 18 |
| Conflict record | 19 |
| Diary entry | 20 |
| Project link | 21 |

A reader must:
- check both magic values, the index checksum, and that every offset and size stays inside the file;
- read the index a piece at a time, never whole: refuse an entry count above the limit below, or an index whose length doesn't fit its count, before reading any line, then check each line as it is read (a known kind, IDs strictly ascending, its entry inside the entry area);
- check that each entry's header matches its index line;
- decompress zstd data without ever producing more than the recorded uncompressed size, and require the zstd frame's own content size, if present, to match;
- check the fingerprint of every object it reads: plain BLAKE3 for content, the record's context string otherwise;
- treat the same ID found in two packs as the same object (packs can overlap while being merged), and when one pack's copy fails for any reason (a failed check, a read error, too little memory), try the other packs' copies before giving up, so a damaged or planted pack never hides a good copy; a pack whose copy failed a check is reported as damaged;
- try the copies smallest recorded size first, ties in pack order (a copy already checked in this process may go first), so a planted copy that claims a huge size is read last; only when no copy works does the read fail, naming a read error or lack of memory if there was one (the object may still be fine), else the damage.

Before skipping an object because the store already holds it, a writer must read a stored copy of the same kind in full and check its fingerprint (once per process is enough). The writer knows the object's size (a file's length, a link's target, an encoded record), so it passes over a copy that records another size without reading it, and reports that copy's pack as damaged. If no copy passes, it stores its own.

Each entry's header is a fixed 50 bytes, so a writer that streams an object can fill the header in after writing its data.

Uncompressed sizes are at most 2^40 bytes (1 TiB); structured records are at most 512 MiB uncompressed.

**Index limits.** A pack holds at most 16,777,216 (2^24) objects, and a writer must not put more in one pack; what a command does when it needs more objects than one pack may hold is open, to be decided while planning milestone 2 ([decision 028](decisions/028-red-team-m1-session-2-rulings.md), [design.md §7](design.md#7-open-questions-and-deferred-decisions)). An index line is 43 to 61 bytes (its two sizes are varints of 1 to 10 bytes), so the index of a pack with `count` objects is the count's own varint followed by 43 × `count` to 61 × `count` bytes, with nothing after the last line.

## 12. Project files (outside `.bolt/`)

- **`bolt.toml`** (TOML; decisions 008 and 016). Draft keys: `preset`, `chunk-files-over`, `ask-before-saving-over`, and `[[rule]]` tables with `files`, `lockable`, `conflict-markers` and `moves-with-asset`. Unknown keys produce a warning. Final key names are settled when presets are built (milestone 6).
- **`.boltignore`** files work like `.gitignore` files: one at the project root, and optionally more in subfolders, each using the `.gitignore` pattern syntax relative to its own folder. `.bolt/` and `.git/` are always ignored (decision 019); on Windows and macOS that includes `.git` in any letter case (`.GIT`), because there it is Git's folder too. A `.boltignore` must be a regular file of at most 1 MiB; a link, pipe or larger file stops the save with an error (decision 025). Decisions 028 and 029 add:
  - **Letter case.** On Windows and macOS patterns match without regard to letter case, as Git does there by default (`core.ignorecase`): `*.env` ignores `PROD.ENV`, and `secrets/` ignores `Secrets/`. On Linux patterns match exactly, as in Git.
  - **Braces.** `{` and `}` are ordinary characters, as in Git: `{a,b}` is not a choice, and `\{` means the same as `{`.
  - **Limits.** The `.boltignore` files in force at one folder (its own and those of the folders above it) total at most 1 MiB, and the files one save takes rules from total at most 16 MiB, counting each file whose bytes Bolt has read and sized, even if its patterns then fail. Bolt checks both before it builds any rule from a file. Past either limit the save stops, saves nothing, and the error names the file that went over. A file refused before it is counted (one over 1 MiB, say) still costs a read of up to 1 MiB, but the save stops there, so no later file is read.
  - **Lines Bolt can't yet read like Git.** Until milestone 2 brings a matcher that follows Git's own rules, Bolt accepts only lines it reads exactly as Git does. Any other line stops the save, which saves nothing; the error names the file and the line, quotes the line and says what to change. The scan stops at the first `.boltignore` problem of any kind.
    - Lines are read as Git reads them. A UTF-8 byte order mark at the start of the file is skipped. Lines end at each LF, and one CR before it is dropped, on the last line too. A line starting with `#` is a comment, and blank lines are skipped. Spaces at the end of a line are removed unless a `\` escapes them: `\\ ` loses its space, and `\ ` keeps it.
    - A line then stops the save if it holds a control character anywhere (U+0000 to U+001F, or U+007F; a tab, say): Git ends a pattern at a NUL. So does a line that ends in any other blank character (a no-break space, say), or in a `\` that escapes nothing, even before a final `/` (`foo\`, `foo\/`).
    - So does a `**` that touches a `\` on either side (`**\*.md`, `x/**\y`, `a\\**`): on Windows the matcher would take that `\` for a folder separator, and the rule is the same on every system.
    - So does a line that isn't in Unicode's composed form (NFC), with an accented letter written as a letter plus a separate accent: Git on macOS compares names in composed form. This rule too is the same on every system.
    - It also stops the save if it has a `[`, not written `\[`, that doesn't begin a simple class: `[`, then one or more members, then `]`. A member is an ASCII letter or digit, `.` or `_`, or a range between two digits, two small letters or two capitals, lower end first (`[0-9]`, `[a-rt-v]`). So these stop the save: a `[` that is never closed; `[]`; `[!…]` and `[^…]`; and a class holding `/`, `\`, `[`, a `]` right after the `[`, `:` (as in `[[:alpha:]]`), a brace, other punctuation, a character beyond ASCII, or a range that runs backwards (`[z-a]`).
    - On Windows and macOS, where letter case is ignored, Git lowers the letters of each name it compares, but not a capital letter after a `\` or in a class without its small letter, so that capital matches nothing. Those lines stop the save too: write `B` for `\B`, and `[Bb]` for `[B]`.
    - Common patterns work: `*.py[cod]`, `[Bb]in/`, `[Dd]ebug/`, `*.sw[a-p]`, `[._]*.s[a-v][a-z]`, `*.[oa]`, `[0-9]*.log`, `foo\ `, `\#notes`, `!keep.txt`.
  - **A limit on size.** A file whose patterns are too many or too complex for the matcher stops the save, with an error that says so. Ignoring letter case on Windows and macOS lowers that limit to a few thousand path patterns (about 150 KB) per file.

## 13. Test vectors

The first implementation generates, and the owner reviews, vectors in `crates/bolt-vcs-core/tests/vectors/`:
- for each record type, a few hand-checkable inputs with their exact encoded bytes (hex) and their IDs;
- plain BLAKE3 checked against the official BLAKE3 test vectors;
- a small reference pack and a `.bolt/format` file;
- invalid encodings that every reader must reject.

Vectors are added milestone by milestone, for the records each milestone builds; format 1 freezes at the v0.1 release (decision 019).

## 14. Open points

- Final `bolt.toml` key names (milestone 6).
- Conflict record details (before v0.2).
- Delta storage for small files (a later `feature deltas`).
- Splitting giant folder listings (a later listing version).
- The signature format (later).
- What a command does when it needs more objects than one pack may hold (§11; decided while planning milestone 2).
- A limit on path length (decided with milestone 3's write-out, before format 1 freezes; decision 026).