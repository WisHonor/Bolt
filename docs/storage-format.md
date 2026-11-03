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

- `name` must follow decision 004's always-on rules: valid UTF-8, not empty, not `.` or `..`, no `/`, no NUL, at most 255 bytes, and not `.bolt` under any spelling Windows or macOS would treat as `.bolt` (any letter case, trailing dots or spaces, the short names `BOLT~1` to `BOLT~4`, a `:stream` suffix).
- The portability rules (differences only in letter case or Unicode normalization, reserved device names, trailing dots or spaces, characters Windows forbids) are checked when saving, not when reading: imported histories may contain such names (decision 004).
- **Limits for a whole save** ([decision 025](decisions/025-red-team-m1-rulings.md)): the folders reachable from a save's root listing may nest at most **4,096** levels below the root, and describe at most **10,000,000** entries in total, counting every path (a listing reached through two paths counts twice). A reader walking a save must stop with an error as soon as either limit is passed, and must not recurse once per folder level. The one exception is a save's search through folders of the save before it that are gone from disk, for entries it must keep ([design.md](design.md) §5.2). Past that earlier save's limits, that search ends without an error and keeps what it found, so a save that is over the limits can't stop every save after it. A reader that compares two saves counts each of them against the limits on its own, not the two together.

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
string  command, as typed (values that look like secrets are redacted)
person  who and when (as in §4)
string  Bolt version
state:
  varint  line count;    lines, sorted by name:   string name, id save
  bool    has tags;      if true: id of the tag list
  u8      position (1 = on a line, 2 = at a save), then string line name or id save
  varint  shelf count;   shelves, newest first:   string label (unique), id save
  bool    has server lines; if true: id of the server-lines list
varint  safety-copy count
safety copies, sorted by path:
  string  path
  u8      kind (as in §3)
  id      id
  varint  size
```

Line names follow decision 012's rules: Unicode letters and digits, `-`, `_`, `.` and `/` only; not empty; no leading or trailing `/`, no `//`, and no `.` or `..` part. Two line names may not differ only by letter case, and a position "on a line" must name one of the lines. Shelf labels are unique.

- **Limits:** line and shelf names at most 255 bytes; the recorded command at most 64 KiB; the Bolt version at most 256 bytes; safety-copy paths at most 4 KiB. At most 100,000 lines, 10,000 shelves and 1,000,000 safety copies. Safety copies must be strictly sorted by path; readers reject anything else.

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
| `cache/` | Rebuildable speed-ups | No |
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

**`state/lock`** is held with an operating-system lock (`LockFileEx` on Windows, `flock` elsewhere) by every command that changes the repository. Its contents mean nothing. Once a command holds the lock, it replaces `state/lock-info` (through `tmp/` and a rename) with `pid <n>\ncommand <command>\nstarted <unix ms>\n`; a command that finds the lock busy shows that text in its "waiting for another command" message. The text lives in a separate file because Windows doesn't let other processes read a locked file; it is only informational and may be stale.

**`state/journal`** is written (whole, through `tmp/` and a rename) before a command starts rewriting files in the folder, and removed after the command commits. It records the target diary entry, each planned file action (path, expected current ID, new ID) and the safety copies taken, followed by a BLAKE3 checksum of the bytes before it. If it exists when Bolt starts, Bolt finishes or rolls back the interrupted command (decision 010).

## 11. Pack files

Each command that stores data writes one or more packs (decisions 009 and 041): each is built in `tmp/`, flushed to disk, then renamed to `store/packs/<hex of the index checksum>.pack`. The diary entry goes in the command's last pack, and `state/head` moves only after all of them are in the store.

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
| Tag list | 22 |
| Server lines | 23 |

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

**Index limits.** A pack holds at most 16,777,216 (2^24) objects, and a writer must not put more in one pack ([decision 028](decisions/028-red-team-m1-session-2-rulings.md)). A command that needs more writes several; Bolt's writer starts a new pack once one holds 2^20 objects, which keeps the index it holds in memory near 100 MB ([decision 041](decisions/041-several-packs-per-command.md)). An index line is 43 to 61 bytes (its two sizes are varints of 1 to 10 bytes), so the index of a pack with `count` objects is the count's own varint followed by 43 × `count` to 61 × `count` bytes, with nothing after the last line.

## 12. Project files (outside `.bolt/`)

- **`bolt.toml`** (TOML; decisions 008 and 016). Draft keys: `preset`, `chunk-files-over`, `ask-before-saving-over`, and `[[rule]]` tables with `files`, `lockable`, `conflict-markers` and `moves-with-asset`. Unknown keys produce a warning. Final key names are settled when presets are built (milestone 6).
- **`.boltignore`** files work like `.gitignore` files: one at the project root, and optionally more in subfolders, each using the `.gitignore` pattern syntax relative to its own folder. `.bolt/` and `.git/` are always ignored (decision 019); where the drive ignores letter case (as Windows' and macOS's do by default) that includes `.git` in any letter case (`.GIT`), because there it is Git's folder too. A `.boltignore` must be a regular file of at most 1 MiB; a link, pipe or larger file stops the save with an error (decision 025). Decisions 028, 029 and 038 add:
  - **`.gitignore` files are read too** (decision 038), as if each were a `.boltignore` in its folder, with that folder's `.boltignore` lines after its `.gitignore` lines, so they can add to Git's rules or undo them. The limits below count both kinds. A `.gitignore` that isn't a regular file (a link, folder or special file) is skipped with a `gitignore_skipped` warning and its rules don't apply, as Git skips a linked one; any other problem with it (over 1 MiB, past a limit, or unreadable) stops the save, as for a `.boltignore`. Git's personal lists (`.git/info/exclude`, the user's global Git ignore file) are not read.
  - **Rules keep out only what isn't saved yet** (decision 038). A file or link in the save the new one builds on is saved again even when a rule matches it, as in Git; it leaves history only when it is deleted from the folder. The name must match exactly, and the entry must still be a folder if it was saved as one and not a folder if it wasn't. So a save looks inside an ignored folder where that save holds the folder, keeps only the names saved there, at every depth, and reads no ignore file inside it. Before any rule applies, a save keeps exactly as they were the project links, names this system can't hold and names that are one file on the drive, without looking at what the folder holds there, and a link Git wrote as a file holding exactly its target ([design.md](design.md) §5.2, decisions 036 and 039).
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

## 14. Open points

- Final `bolt.toml` key names (milestone 6).
- Conflict record details (before v0.2).
- Delta storage for small files (a later `feature deltas`).
- Splitting giant folder listings (a later listing version).
- The signature format (later).
- A limit on path length (decided with milestone 3's write-out, before format 1 freezes; decision 026).