# 114 — `bolt.toml`: its keys, and how Bolt reads it

- **Status:** Accepted
- **Date:** 2026-11-09
- **Agenda topic:** none (milestone 6 planning)
- **Decided by:** Claude, at the owner's request while the owner was away. The owner can reverse any part.
- **Refines:** [decision 008](008-layout-and-settings.md) (what `bolt.toml` holds, and what an unknown key does); [decision 016](016-presets.md) (the rule format; "exact key names are finalised in the format spec"); [decision 056](056-question-before-large-saves.md) (the question's numbers become settings); [decision 082](082-how-big-files-are-cut-into-chunks.md) (the 1 MiB threshold becomes `chunk-files-over`)

## Context

`bolt.toml` holds a project's rules and travels with the project, so it reaches Bolt from strangers: a copied folder, an imported repository, an old save. Until now no code read it. [storage-format.md](../storage-format.md) §12 listed draft keys (`preset`, `chunk-files-over`, `ask-before-saving-over`, and `[[rule]]` tables with `files`, `lockable`, `conflict-markers` and `moves-with-asset`) and said the final names come with presets. Three things need settling: the names and values, what a file Bolt can't read does, and when it is read.

In v0.1 two settings change what Bolt does: the size from which files are stored in chunks, and the numbers of the question before a large save. Rules for locking and conflict markers are recorded for v0.3 and v0.2.

## Options

**A file, or a value, Bolt can't use**
- **A. Stop the command,** as a `.boltignore` Bolt can't read stops a save. Strict, but a typo in `bolt.toml` would then stop `bolt discard bolt.toml`, the command that puts the file back.
- **B. Warn and use Bolt's default for what can't be read.** Each default is the careful side: files are chunked from 1 MiB, and the question is asked from 100 MB. A file can make Bolt ask less only with a value Bolt can read, which is the project's own choice.

**How sizes are written**
- **A. A number of bytes** (`chunk-files-over = 1048576`). Nothing to parse, hard to read.
- **B. Text with a unit** (`"1 MiB"`, `"100 MB"`). What a person would write. Bolt states its own limits in 1024-based units, while decision 056's 100 MB is 100,000,000 bytes, so both families are needed.

## Decision

**Warn and use the default (B); sizes are text with a unit (B).**

### The keys

```toml
preset = "game"
chunk-files-over = "1 MiB"
ask-before-saving-over = "1 GiB"
ask-before-saving-files-over = 10000

[[rule]]
files = ["*.unity", "*.prefab"]
lockable = true
conflict-markers = false
```

| Key | Value | Default | What it does |
|---|---|---|---|
| `preset` | text, at most 64 bytes | none | A label: which preset wrote the file. Bolt shows it and never acts on it. |
| `chunk-files-over` | a size, from `"1 MiB"` to `"128 GiB"` | `"1 MiB"` | A file of this size or more is stored in chunks (up to 128 GiB, decision 082). |
| `ask-before-saving-over` | a size | `"100 MB"` | A save that adds more new bytes than this asks first (decision 056). |
| `ask-before-saving-files-over` | a whole number, 0 or more | `10000` | A save that adds more new files than this asks first. |
| `[[rule]]` | a table, any number of them | none | Settings for the files its patterns match. Rules are read top to bottom, and for each setting the last rule that matches a file and gives it wins (decision 016). |
| `files` (in a rule) | a list of one or more patterns, in `.boltignore` syntax | required | Which files the rule is about. |
| `lockable` (in a rule) | `true` or `false` | `false` | The file can be locked. Recorded only: locking arrives with syncing (v0.3, decision 014). |
| `conflict-markers` (in a rule) | `true` or `false` | `true` | Whether a conflict in the file is shown with markers inside it. Recorded only: conflicts arrive with combining (v0.2, decision 007). |

- **A size** is a whole number, then optional spaces, then a unit: `B`, `KB`, `MB`, `GB`, `TB` (1,000 bytes to the kilobyte) or `KiB`, `MiB`, `GiB`, `TiB` (1,024), spelled exactly so. No fractions.
- **`chunk-files-over` is the smallest size that is cut**, as decision 082's "1 MiB to 128 GiB" is: with `"1 MiB"`, a file of exactly 1,048,576 bytes is cut. It can't go below 1 MiB. It changes which files are stored as chunks, never an ID.
- **The draft's `moves-with-asset` is not a key yet.** It comes with move notes (v0.2). A v0.1 Bolt that meets it warns, as for any key it doesn't know.
- No key names a program, and nothing in the file is ever run (decision 008).

### How it is read

- **The file:** `bolt.toml` in the project's top folder, a regular file of at most 1 MiB, read through the folder Bolt holds open and never through a link (decision 048). The copy that counts is the one in the folder, saved or not (decision 016).
- **No file** means Bolt's defaults, without a word. That is what a Custom project has ([decision 113](113-presets-what-each-writes-and-how-start-picks-one.md)).
- **A file Bolt can't use at all** (a link, a folder or special file, over 1 MiB, unreadable, or not TOML) is passed over: Bolt's defaults apply, and a `rules_file_skipped` warning says why.
- **A key Bolt can't use** is passed over, the rest of the file still counts, and one `rules_ignored` warning names each such key and why: a value of the wrong kind or out of range, a rule without usable `files`, or a key this Bolt doesn't know, which a newer Bolt may have written (decision 008).
- **Nothing in `bolt.toml` stops a command.**
- **When:** a command that stores files reads the file once, right after it takes the lock, and uses its `chunk-files-over` for everything it stores: a save, the safety copies of `discard` and `shelf`, an import. `bolt save` also uses the two question numbers. Only `bolt save`, `bolt setup` and `bolt start` show the warnings, so a file a newer Bolt wrote doesn't warn on every command.
- Personal settings can't change any of it (decision 008).

## Consequences

- [storage-format.md](../storage-format.md) §12 gives the rules above as the format, with test vectors for the reader: each key, each kind of bad value, and the limits.
- A typo in a setting means Bolt's default for it, and `bolt save` says so each time until it is fixed. `bolt setup` shows the rules as Bolt reads them.
- A project file can turn the large-save question off for itself by naming a huge number. The question guards against the user's own slips, and whoever writes `bolt.toml` could as well write the files being saved.
- The reader meets hostile files: it gets a fuzz target and malicious-file cases (deep nesting, huge numbers, a link in its place), and the red team attacks it (decision 008's consequence).
- Lowering the chunk threshold below 1 MiB isn't possible. If measurements on real assets ask for it, a later decision can widen the range without a format change.
