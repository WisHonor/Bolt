# Bolt — Design Spec

> **Living document.** It always describes the *current* design; the reasons behind each choice live in [`decisions/`](decisions/).
> **Phase:** building; milestone 1 done 2026-11-01, with the fixes from its two red-team sessions and its final review ([decisions 024](decisions/024-red-team-gate.md) to [029](decisions/029-red-team-m1-final-review-rulings.md)); milestone 2 ("Coming from Git") built on 2026-11-03, with its red-team session, review and merge next (design signed off by the owner on 2026-10-31) · **Started:** 2026-10-31 · **Decisions recorded:** 42
> **Companion documents:** [storage-format.md](storage-format.md) · [security.md](security.md) · [testing.md](testing.md) · [milestones.md](milestones.md)

## 1. Vision

**In plain words.** Bolt keeps the history of a project: you save versions, go back in time, try ideas on the side, and work with other people. It is meant to be one tool for the *whole* project: the source code **and** the multi-gigabyte textures, models, datasets and videos next to it.

**Why it exists.** Bolt is a personal project, but it is designed as if it would be marketed and sold: the bar is "best on the market", and decisions are judged that way.

To the user, every save is a full **photo** of the project. How photos are stored (whole copies, differences, shared chunks) is the storage engine's private business and can change without changing what users see.

**What makes it different**
- **Own engine**: its own object model, storage format and network protocol. Not a Git wrapper, not Git with renamed commands.
- **Friendly**: clear command names, human error messages, undo for everything.
- **Great with big files**: big files are split into chunks so only changed pieces are stored and sent; big files and old versions download on demand; built-in, opt-in file locking for files that can't be merged.
- **Hybrid**: small text files handled like Git (but faster), big binaries like Perforce (but friendlier), in the same project, with one engine and one storage format.
- **Adapts to the project**: `bolt start` detects the project type and suggests a preset; presets only change defaults, and per-file rules always apply.
- **Open source** (likely MIT/Apache-2.0 for the tool, possibly AGPL for the server), with its own hosting platform later: asset previews, a locks dashboard, cheap deduplicated storage.

**Signature experience.** The owner's target for `bolt start` (exact wording is settled in topics 16–17):

```text
$ bolt start

Looks like a game project (found Unity files and 2.3 GB of assets).

What kind of project is this?
  1. Code (apps, websites, libraries)
  2. Game  ← suggested
  3. AI / data
  4. Design / video
  5. Custom

Using "Game" settings:
  • Big files (over 1 MB) are chunked and downloaded on demand
  • .psd .fbx .blend .png files can be locked
  • Code files are stored and merged normally

Change anytime with: bolt setup
```

What it implies for the design:
- **Presets change rules, never the engine.** They decide what's chunked, lockable and ignored, and how files merge; the storage format stays the same. That is what makes "change anytime" cheap: switching presets must not rewrite history, which requires that a file's ID never depends on how it is chunked (topic 5).
- **Settings reach teammates.** The rules go in `bolt.toml` and `.boltignore` at the project root, so everyone who copies the project gets the same behaviour (decision 008).
- **Presets know the engine.** Unity, Unreal and Godot differ in which files must be locked (Unity: scenes and prefabs; Unreal: `.uasset`, `.umap`) and which cache folders must be ignored (Unity's `Library/`, `Temp/` and `Logs/` can hold gigabytes and must never be saved). Detection applies ignore rules before reporting sizes.
- **Detection only reads names and sizes.** It never runs anything from the project.
- **Locking and on-demand download need a server**, so they arrive in v0.3; until then the rules are recorded but not enforced.
- **Scripts need a non-interactive form**, e.g. `bolt start --preset game`.

**First audience and roadmap** ([decision 001](decisions/001-first-audience-and-v0.1.md))
- **First audience:** game developers. The engine stays general; games set the priorities, first presets and benchmarks.
- **Main platform:** Windows. macOS and Linux are supported and tested in CI from the start.
- **v0.1, solo and local-only:** `start`, `save`, `status`, `changes`, `history`, `show`, `goto`, `discard`, `undo`/`redo`, `shelf`/`unshelf`, `save --amend` ([decision 021](decisions/021-v0.1-command-scope.md)), Git import including Git LFS files whose contents are on disk (decisions [002](decisions/002-git-compatibility.md) and [020](decisions/020-lfs-import.md)), chunked big files. Big files are streamed, never loaded whole into memory. Presets: Code, Game (Unity, Unreal, Godot) and Custom ([decision 016](decisions/016-presets.md)). Lines can be created (by saving away from a line), listed and switched (decision 012).
- **v0.1 milestones:** First save → Coming from Git → Working folder → Undo everything → Big files → Presets and polish ([decision 018](decisions/018-milestone-plan.md), [milestones.md](milestones.md)).
- **v0.1 release gates** ([decision 023](decisions/023-v0.1-speed-bar.md)): saving a 10 GB file uses at most about 500 MB of memory; `status` on an unchanged 100,000-file project takes under 2 seconds on an SSD; benchmark results against Git and Git LFS are published.
- **v0.2:** combining lines (merging, conflicts), plus renaming and deleting lines, and `who`.
- **v0.3:** sync and locking for small teams; big files and old versions download on demand.
- **Before 1.0:** Git export ([decision 002](decisions/002-git-compatibility.md)). Live two-way Git sync is not promised, but nothing may block it.
- **Later:** conflict algebra on top of decision 007, smart locks on top of decision 014, AI/data and Design/video presets (decision 016), hosting platform, GUI and editor plugins.

**Scope of this design phase.** The brief describes several products: the engine (with a Git bridge), a sync server, and a hosting platform. This phase designs the **engine** in detail, and the server and platform only as far as the engine must respect them. Each gets its own design pass before it is built.

## 2. Principles (non-negotiable)

1. **Never lose or corrupt user data.** Atomic writes (temp file + rename), repository locking, hash verification on every read and every received object, crash safety.
2. **Security by design.** Assume projects and servers can be malicious: path traversal (`..`, absolute paths, writes into `.bolt/`), symlink attacks, case-insensitive and Windows name tricks, never auto-running code (hooks) on copy, decompression and size bombs, tampered data, server permissions and locks, safe asset previews (e.g. scripts inside SVG), secret-detection warnings, signed saves later.
3. **Testability.** Unit tests; round-trip tests (save → restore → byte-identical); property-based random action sequences; differential testing against Git (import real histories, verify every snapshot); crash/kill tests; fuzzing; a malicious-repo corpus; CI on Windows, macOS and Linux; dependency auditing.
4. **Provable claims.** Reproducible benchmarks against Git, `bolt verify` for self-checking, a documented storage format spec.
5. **Correct first, fast second.** Measure, then optimize.

## 3. Tentative decisions from the kickoff brief

None of these is final until it has a decision record. "First review" is Claude's initial read; the owner decides in the listed topic.

| # | Tentative decision | First review | Topic |
|---|---|---|---|
| T1 | Rust | Keep | — |
| T2 | Users see snapshots; storage may use deltas with periodic full copies | **Confirmed**: identity by decision 005; storage by decision 009 (packs; deltas for small files come later) | 5, 9 |
| T3 | BLAKE3 hashing, zstd compression | **Confirmed**: BLAKE3 (decision 005), zstd (decision 009) | 5, 9 |
| T4 | Objects: blobs, trees, saves, refs, plus an operation log | Mostly keep; chunk lists added by decision 005; conflict entries in topic 7; mutable state (lines, operation log) in topics 8 and 10 | 5–7, 10 |
| T5 | A save holds author, timestamp, message, parents, root tree, stable change ID | **Confirmed** with additions by decision 006: author and saver with time zones, 128-bit random change ID, move notes, signature slot, extra fields | 6 |
| T6 | No staging area; `bolt save` saves everything, `bolt save <files>` saves some | **Confirmed** by decision 003: new files tracked automatically; unusually large saves ask first | 3 |
| T7 | Operation log; `undo`/`redo` work on any action | **Confirmed** by decision 010: one diary for the whole repository, editor-style undo/redo, 30-day undo window | 10 |
| T8 | Conflicts stored inside saves ("sticky notes") | **Confirmed** by decision 007: every version kept; conflict algebra planned on top later | 7 |
| T9 | Content-defined chunking above ~1 MB; big files and old versions on demand; full small-file history local; opt-in locking | Identity: decision 005. Chunking: FastCDC from 1 MB, 64 KB average chunks (decision 009). Dedup gains depend on file format. On-demand download: decision 013. Locking: decision 014 | 5, 9, 13, 14 |
| T10 | Presets at `bolt start`, written to `.bolt/config.toml`, changeable via `bolt setup` | **Confirmed**: rules in `bolt.toml` + `.boltignore` (decision 008); presets written out in full (decision 016) | 8, 16 |
| T11 | On-disk format version number from day one | **Confirmed**, with a feature list, by decision 008 | 8 |
| T12 | Crates: `bolt-core` (engine library) + `bolt` (CLI) | Keep the split; those package names are taken on crates.io, though the command can still be `bolt` | Deferred: name |

## 4. Design agenda

Ordered so the most foundational, hardest-to-change decisions come first. Stage 4 topics are decided only as far as the engine must respect them; each gets a full design pass before it is built. Every topic records its own security and testing notes.

**Stage 1 — Direction**
- [x] 1. First audience, tier-1 platform, and what v0.1 is → [decision 001](decisions/001-first-audience-and-v0.1.md)
- [x] 2. Git interoperability scope: none, import only, or a two-way bridge → [decision 002](decisions/002-git-compatibility.md)

**Stage 2 — Core model** (baked into every repository; hardest to change)
- [x] 3. Working-copy model: is your folder a live draft save (jj) or separate until you save (Git)? Tracking new files, partial saves, shelf → [decision 003](decisions/003-working-copy-model.md)
- [x] 4. What a project can contain: path rules, line endings, executable bit, symlinks, empty folders, nested projects → [decision 004](decisions/004-project-contents.md)
- [x] 5. Object identity: what is hashed and how, ID format, canonical byte encoding, big-file identity, objects known but not downloaded → [decision 005](decisions/005-object-identity.md)
- [x] 6. The save object: fields, change IDs (generation, display, divergence), room for signatures and move hints → [decision 006](decisions/006-save-object.md)
- [x] 7. Conflicts as data: how a conflict lives in a tree and in your folder, for text and binary files → [decision 007](decisions/007-conflicts.md)

**Stage 3 — Local engine**
- [x] 8. Repository layout in `.bolt/`; shared vs. local config; format version, feature list, upgrades → [decision 008](decisions/008-layout-and-settings.md)
- [x] 9. Storage engine: loose vs. packed objects, compression, deltas and full-copy cadence, chunking algorithm and thresholds, garbage collection → [decision 009](decisions/009-storage-engine.md)
- [x] 10. Operation log: undo/redo semantics per command, two commands at once, how long undo lasts → [decision 010](decisions/010-operation-log.md)
- [x] 11. Fast `status` and safe `goto`: file-state cache, timestamp traps, file watching, never overwriting unsaved work → [decision 011](decisions/011-status-and-write-out.md)

**Stage 4 — Collaboration** (constraints only, for now)
- [x] 12. Lines and combining: naming, remote lines, tags, three-way merge, renames → [decision 012](decisions/012-lines.md) (full combining design before v0.2)
- [x] 13. Partial download and offline behaviour → [decision 013](decisions/013-local-data-and-offline.md)
- [x] 14. Locking semantics → [decision 014](decisions/014-locking.md)
- [x] 15. Sync protocol, server storage model, permissions → [decision 015](decisions/015-sync.md) (full protocol design before v0.3)

**Stage 5 — Product surface and wrap-up**
- [x] 16. Presets and the shared project file: detection, preset contents, per-file rule syntax → [decision 016](decisions/016-presets.md)
- [x] 17. Commands, names and aliases; error and output style (human-readable and `--json`) → [decision 017](decisions/017-commands-and-output.md)
- [x] 18. Deliverables: storage format spec draft, threat model and security plan, test plan, milestone-1 task list; a fresh-eyes review of this document before sign-off → milestones in [decision 018](decisions/018-milestone-plan.md); review clarifications in [decision 019](decisions/019-review-clarifications.md); the owner's answers to the review in decisions [020](decisions/020-lfs-import.md), [021](decisions/021-v0.1-command-scope.md), [022](decisions/022-engineering-rules.md) and [023](decisions/023-v0.1-speed-bar.md). Signed off by the owner on 2026-10-31.

**Milestone 2 planning** (from 2026-11-02; each question is decided before the milestone's plan is written, except the last, which the owner settled by approving the plan)
- [x] Reading Git repositories → [decision 031](decisions/031-own-git-reader.md)
- [x] How a Git commit becomes a save → [decision 032](decisions/032-exact-repeatable-commit-import.md)
- [x] How branches become lines, and where you stand after an import → [decision 033](decisions/033-git-branches-to-lines.md)
- [x] How tags come in, and keeping big lists of tags and server lines out of every diary entry → [decision 034](decisions/034-tags-kept-and-shared.md)
- [x] Names in Git trees that Bolt can't store as they are → [decision 035](decisions/035-git-names-bolt-cant-take.md)
- [x] Submodules → [decision 036](decisions/036-submodules-as-bare-links.md)
- [x] Git LFS before milestone 5 → [decision 037](decisions/037-lfs-waits-for-milestone-5.md)
- [x] Ignore rules after an import → [decision 038](decisions/038-gitignore-and-saved-files.md)
- [x] The first save after an import → [decision 039](decisions/039-first-save-after-import.md)
- [x] Re-importing → [decision 040](decisions/040-import-once-catch-up-later.md)
- [x] More objects than one pack may hold (decision 028's open question) → [decision 041](decisions/041-several-packs-per-command.md)
- [x] Trusting a folder that other accounts can write to (a question from milestone 1's second red-team session) → [decision 042](decisions/042-trust-hint-skips-folders-anyone-can-write-to.md)

## 5. Design

The current design, by area. Each part links to the decision that set it.

### 5.1 Your project folder and saves ([decision 003](decisions/003-working-copy-model.md))

**In plain words.** Your project folder is separate from history: nothing enters history until you run `bolt save`. But Bolt never destroys unsaved work without keeping a copy first, so `undo` can always bring it back. It works like an IDE's local history, built into the version control system.

- **Saving.** `bolt save "msg"` records every change in the folder; `bolt save <files or folders> "msg"` records only those. Choosing parts of a file comes later.
- **Tracking.** New files are tracked automatically (there is no `add` step) unless ignore rules exclude them. Rules keep out only files that aren't saved yet: a saved file stays saved until it is deleted, as in Git ([decision 038](decisions/038-gitignore-and-saved-files.md)). A save that adds unusually much at once asks for confirmation first; the threshold is set per preset (topic 16), and a flag skips the question for scripts.
- **Safety copies.** Before any command overwrites or removes unsaved work (`discard`, `goto`, `combine`, `undo`, `redo`, `shelf`), Bolt stores the current content of the affected files and records it in the operation log. Only files about to be touched are copied. Safety copies live in the normal object store, so content that is already stored costs nothing extra.
- **Shelf.** `bolt shelf` sets unsaved changes aside as a hidden save and restores the folder; shelving is undoable.
- **`goto` with unsaved changes.** Changes come along when nothing collides; otherwise Bolt stops and offers to shelve, save or discard them.
- **Conflicts.** A save can contain unresolved conflicts, and the save records them (topic 7).
- **`status`** only reads; it never writes to the store.

### 5.2 What a project can contain ([decision 004](decisions/004-project-contents.md))

**In plain words.** Bolt stores exactly what's in your project and never changes a file's bytes. It checks new names when you save, so a project that works on Linux won't break on Windows or a Mac, and it refuses to write anything dangerous when it puts files back on disk.

- **Entry kinds:** regular file, executable file, symlink, folder (empty folders included), and a link to another project (from Git submodules; not fetched, and kept as it is by every save: [decision 036](decisions/036-submodules-as-bare-links.md)).
- **Bytes in = bytes out:** no line-ending conversion or any other change to contents by default.
- **Not stored:** timestamps, Windows attributes, other permissions. On Windows the executable flag keeps whatever history says.
- **Names (always):** valid UTF-8, `/` between folders, no empty, `.` or `..` parts, no absolute paths, and no part that Windows or macOS would treat as `.bolt`.
- **Portability check on save:** new names that would break on Windows or macOS are refused (differences only in letter case or Unicode normalization, reserved device names, trailing dots or spaces, forbidden characters), unless the project relaxes the rule. Imported histories keep their names.
- **Safety on write-out:** never through a symlink, never outside the project, never to a name the OS would misread; such entries are reported instead. Symlinks that leave the project, or that the OS can't create, become placeholder files with a warning.
- **Nested projects:** a folder with its own `.bolt/` is left alone by the outer project.
- **Entries this computer can't hold** (names Windows can't create, names that collide here, symlinks written as placeholders) stay in history untouched: `status` lists them as not written on this computer, and saves never record them as deleted or changed (decision 019). Saves keep them from milestone 2, when imports first bring them, and also keep a symlink that Git wrote as a plain file holding its target; a save's warnings say how many entries it kept ([decision 039](decisions/039-first-save-after-import.md)). A folder gone from disk that holds one of them comes back holding only what is kept; if a file or link now has that folder's name, the save stops and says to rename or move it.

### 5.3 How data is identified ([decision 005](decisions/005-object-identity.md))

**In plain words.** Every stored object gets an ID that is a fingerprint of its contents. A file is identified by its contents, never by how it was cut into chunks, so the same bytes always have the same ID whatever the settings.

- **Algorithm:** BLAKE3, 256-bit IDs shown as lowercase hex (short prefixes in the UI); declared once per repository, never mixed.
- **Flavors:** file contents and chunks use plain BLAKE3 (`b3sum` prints the same ID). Each structured object type (folder listings, saves, chunk lists…) uses BLAKE3's key-derivation mode with its own context string.
- **Big files:** a chunk list links the file's ID to its chunks; the reassembled file is checked against its ID before use.
- **Folder listings:** one per folder; entries hold name, kind, ID and size, sorted by name in byte order.
- **Encoding:** hand-written binary format, one valid encoding per object, strict readers, test vectors in the storage spec. Serde only for data that is never fingerprinted.
- **Known but not downloaded:** a normal state everywhere; `bolt verify` tells it apart from real loss.

### 5.4 What's inside a save ([decision 006](decisions/006-save-object.md))

**In plain words.** A save points at the photo of your project (its top folder listing) and records who, when and why. Each save also has a short, stable change ID made of letters (`kxqmz`) that survives edits, and it can note files Bolt is sure you moved.

- **Fields:** top folder listing ID; ordered parents; author and saver (name, email, time, time zone); message; change ID; optional move notes; reserved signature slot; extra fields that older versions keep but ignore.
- **Change IDs:** 128 random bits, shown in the letters k–z (never confused with hex fingerprints), as the shortest unique prefix of at least 4 letters; kept across edits; marked "divergent" when two visible saves share one. A save imported from Git takes its change ID from its commit instead ([decision 032](decisions/032-exact-repeatable-commit-import.md)).
- **Move notes:** recorded only when Bolt is certain (same contents moved, or a Unity `.meta` file moved with its asset); used by diffs, merges and locks; they never change the snapshot.

### 5.5 Conflicts ([decision 007](decisions/007-conflicts.md))

**In plain words.** When two lines of work change the same thing differently, Bolt keeps every version in the save, like a sticky note, so you can keep working and fix it later. Nothing is lost, and it works for binary files too.

- **Record:** a folder-listing entry of kind "conflict" points to a record with the sides and the ancestors between them (any of which can be "absent"), plus your half-fixed version if you started fixing it.
- **In your folder:** text files get standard markers with the ancestor section; binary files keep your version in place, with the other versions kept inside `.bolt/`. Presets decide per file type (the Unity preset treats scenes, prefabs and `.meta` files like binaries).
- **Fixed** when the markers are gone, or when you mark the file as fixed.
- **Planned later:** conflict algebra, jj-style automatic simplification of conflicts. The record's general form allows it without a format change.

### 5.6 Settings and the `.bolt/` folder ([decision 008](decisions/008-layout-and-settings.md))

**In plain words.** The project's rules live in two normal files at the top of your project, so they travel with it and are part of its history. Your personal settings stay on your machine and can never change the project's rules.

- **Project rules:** `bolt.toml` (preset, chunking, lockable files, conflict-marker rules, large-save threshold), purely declarative, and `.boltignore` (the `.gitignore` syntax, read and matched exactly as Git does; [storage-format.md](storage-format.md) §12 gives the details and its limits). Bolt reads a folder's `.gitignore` too, before its `.boltignore` ([decision 038](decisions/038-gitignore-and-saved-files.md)).
- **Personal settings:** identity, editor, merge tools, display; kept in your user config folder or `.bolt/config.toml` and never shared. Your name and email come only from you: `BOLT_USER_NAME` and `BOLT_USER_EMAIL`, else a `[[project]]` entry in your own settings for this exact folder (written as a full path), else `[user]`; a project's own settings are ignored, with a note ([decision 025](decisions/025-red-team-m1-rulings.md)). Bolt uses a project only when both its folder and its `.bolt/` belong to you, as Git requires ([decisions 025](decisions/025-red-team-m1-rulings.md), [026](decisions/026-ownership-check-details.md) and [027](decisions/027-project-folder-must-be-yours.md)). On Windows, folders the Administrators group owns count as yours only in an elevated terminal, as in Git ([decision 030](decisions/030-administrators-only-when-elevated.md)). To trust a folder anyway (a shared folder, a drive that records no owner, or your Windows home folder itself, which Windows gives to SYSTEM), list that exact folder in the top-level `safe-folders` list of your own settings (above any `[section]`; Bolt never suggests it for a folder other accounts can write to, such as `/tmp`, or the root of a drive or share on Windows, where anyone could have planted the project: [decision 042](decisions/042-trust-hint-skips-folders-anyone-can-write-to.md)). Anything that runs a program is personal only.
- **`.bolt/`:** `format` (version + feature list), `store/` (saved data), `state/` (lines, operation log), `cache/` (rebuildable), `tmp/` (files being written), `config.toml` (personal). Hidden on Windows.
- **Format safety:** if `format` lists a feature Bolt doesn't know, it refuses to open the repository and asks to be updated. Format upgrades only happen when you ask, through `bolt setup`.

### 5.7 How data is stored ([decision 009](decisions/009-storage-engine.md))

**In plain words.** Each command writes its new data into one big "pack" file, which only appears once it's complete, so a crash can never leave half-written data in the store. Big files are cut into chunks based on their content, so an edit in the middle of a file only adds the chunks around the change (when the file format keeps edits local).

- **Packs:** written to `tmp/`, made durable, then renamed into the store; index at the end with its own checksum; at most 2^24 objects each ([decision 028](decisions/028-red-team-m1-session-2-rulings.md)); a command that needs more writes several, and only its last holds the diary entry ([decision 041](decisions/041-several-packs-per-command.md)); small packs merged later.
- **Chunking:** FastCDC for files of 1 MB and up; 16 KB / 64 KB / 256 KB (minimum / average / maximum) to start, tuned later with real assets.
- **Compression:** zstd per chunk, fast on save and stronger on merge; incompressible data stored raw; sizes recorded so decompression bombs are caught.
- **Deltas:** later, as a format feature, for small text files.
- **Cleanup:** keeps everything reachable from lines, shelves and the undo window; a grace period protects anything recent; cheap cleanup is automatic, repacking happens on request with a space estimate.
- **Checking:** every object is verified against its fingerprint on read, and a stored copy is read and verified before a save relies on it instead of storing its own; a bad copy is passed over for a good one in another pack, with a warning ([decision 028](decisions/028-red-team-m1-session-2-rulings.md)). Copies are tried smallest first, and a save skips, unread, a copy whose recorded size isn't the object's, so a planted copy claiming a huge size can't slow it down ([decision 029](decisions/029-red-team-m1-final-review-rulings.md)).

### 5.8 Operation log and undo ([decision 010](decisions/010-operation-log.md))

**In plain words.** Bolt keeps a diary of every command that changed something. Each entry records the whole state of the repository afterwards, so `undo` simply goes back to an earlier entry, like Ctrl+Z in an editor. Changing commands take turns, and a crash can never leave a command half-done.

- **Entries:** objects in the store; each holds the full state (every line, where you are, shelves), the safety copies made, its parent, and metadata. Tags and server lines, which change rarely, sit in records of their own that entries point to and share while unchanged ([decision 034](decisions/034-tags-kept-and-shared.md)).
- **Committing:** one or more packs (the new data, with the entry in the last one), then `.bolt/state/head` is replaced atomically. Folder rewrites are journaled first, so an interrupted command is finished or rolled back on the next run.
- **Undo/redo:** new entries that copy an earlier state and restore files from safety copies; nothing is erased, and undo can jump to any entry.
- **Other people:** undo changes only your repository; for sent saves and locks, Bolt offers the opposite action (v0.3).
- **Concurrency:** one changing command at a time behind an OS-held lock; read-only commands never wait.
- **Undo window:** 30 days by default (personal setting), plus an optional size budget for safety copies.

**Example** (illustrative; commands and wording are settled in topic 17):

```text
$ bolt save "Add double jump"
Saved ptvz  "Add double jump"  (2 files)

$ bolt discard Assets/Scripts/Player.cs
Threw away your unsaved changes in Assets/Scripts/Player.cs (+31 −4 lines).
A safety copy was kept. Changed your mind? bolt undo

$ bolt undo
Undid: discard Assets/Scripts/Player.cs
  Player.cs is back, with your +31 −4 unsaved lines.

$ bolt undo
Undid: save ptvz "Add double jump"
  ptvz is off line main; its changes are unsaved work again.

$ bolt redo
Redid: save ptvz "Add double jump"
```

Behind the scenes, `head` points at the newest diary entry, and every entry records the full state:

```text
head ─► redo     main → ptvz
        undo     main → kxqm
        undo     main → ptvz   + Player.cs restored from its safety copy
        discard  main → ptvz   + safety copy of Player.cs
        save     main → ptvz
        save     main → kxqm
```

### 5.9 Fast status and safe writing ([decision 011](decisions/011-status-and-write-out.md))

**In plain words.** `status` remembers each file's size and last-modified time, so it only re-reads files that look changed. Anything that writes files plans first, re-checks each file right before replacing it, and never overwrites work you haven't saved.

- **Cache:** size, time, file ID and fingerprint per file, in `.bolt/cache/`; files changed "too recently" are always re-read; `bolt status --full` re-reads everything.
- **Watcher:** later, optional, and only ever a speed-up.
- **Writing files:** plan → stop if unsaved work would be overwritten → re-check each file just before replacing it → temp file + rename → the whole switch journaled.
- **Files in use (Windows):** retry briefly, finish the rest, name the program holding the file, then finish with one more command.

### 5.10 Lines ([decision 012](decisions/012-lines.md))

**In plain words.** A line is a name, like `main` or `feature/double-jump`, that points at the latest save made on it and moves forward when you save. Each save also notes which line it was made on, so history keeps that information even after lines are combined or deleted.

- **Stored in the diary**, not as files: renaming or deleting a line can be undone, and filesystem case rules never interfere.
- **Names:** letters, digits, `-`, `_`, `.`, `/`; no spaces; no two names differing only by letter case; `bolt start` creates `main`.
- **Not on a line:** after `goto` to a save, Bolt says you're looking at an older save; saving there asks for a new line name.
- **Tags:** fixed names for important saves; they never move, and changing one asks first.
- **In v0.1:** lines can be created (by saving away from a line), listed and switched; combining, renaming and deleting come in v0.2.
- **Combining (v0.2):** three-way merge per file; text merged line by line; binaries never merged automatically; move notes followed.
- **Server lines (v0.3):** shown as `main@origin`. Until then the only ones are the server branches a Git import brought in ([decision 033](decisions/033-git-branches-to-lines.md)).

### 5.11 What's on your computer ([decision 013](decisions/013-local-data-and-offline.md))

**In plain words.** Your computer always has the whole history (every save, folder listing and small-file version) plus the current version of every big file, as real files. Older versions of big files download when you need them, and pinning keeps chosen ones available offline.

- **Always local:** saves, folder listings, diary entries, every small-file version.
- **Big files:** current versions fully downloaded (never stubs); older versions on request; offline errors name exactly what's missing.
- **Pinning:** keep chosen saves or folders available offline.
- **Freeing space:** only versions the server has confirmed it holds can be dropped.
- **Later:** partial workspaces for huge projects.

### 5.12 Locking ([decision 014](decisions/014-locking.md))

**In plain words.** Files that can't be merged (textures, models, engine scenes) can be locked: while you hold the lock, nobody else can send changes to that file on any line. Locks arrive with the server in v0.3; locks that follow a change until it reaches `main` come later.

- **Which files:** patterns in `bolt.toml`, filled in by presets.
- **Early warning:** lockable files stay read-only until you lock them (a personal setting turns this off).
- **Locking:** `bolt lock <file> "<why>"`; `bolt status` shows who holds what.
- **Release:** automatic on send, unless you ask to keep the lock.
- **Safety net:** the server refuses changes to files someone else has locked.
- **Forgotten locks:** maintainers can break them; the holder is told and keeps their work.
- **Offline:** you can edit on request, with a warning; Bolt takes the lock when you're back online.
- **Planned later:** smart locks that release once the change reaches the integration line.

### 5.13 Syncing ([decision 015](decisions/015-sync.md))

**In plain words.** Your copy and the server first compare which saves they have, then move only the missing pieces, each identified by its fingerprint. Every piece is checked on arrival, transfers resume where they stopped, and a big file that changed in one spot uploads only the changed chunks.

- **Transport:** HTTPS; browser login; tokens in the OS credential store.
- **Trust nothing:** both sides verify every piece against its fingerprint.
- **Server storage:** chunk packs in object storage (S3 / R2), everything else in PostgreSQL; chunks are shared only within a project or organization, and only after proof of possession.
- **Permissions:** roles per project and protected lines; folder-level permissions later.
- **Folder remotes:** the same layout on a USB drive or NAS, for backups (no locks).
- **Versioned protocol** from the first release.

### 5.14 Presets ([decision 016](decisions/016-presets.md))

**In plain words.** A preset is a starter set of project rules. `bolt start` detects what kind of project you have, suggests a preset and writes all of its rules into `bolt.toml` and `.boltignore`, so everything Bolt does is visible and editable in your project.

- **Detection:** names and sizes only; cache folders skipped; stops early on huge folders.
- **Rules:** read top to bottom, later rules win; `.boltignore` pattern syntax; settings like `lockable` and `conflict-markers`.
- **v0.1 presets:** Code (apps, websites, libraries, scripts), Game (Unity, Unreal, Godot), Custom. AI/data and Design/video later.
- **Code preset:** no locks, markers for all text, ignore lists for every detected language, `.env` ignored, asks before saving more than about 100 MB at once, normal chunking for big files.
- **Changing presets:** `bolt setup` shows the exact changes and applies them as an undoable change.

### 5.15 Commands and messages ([decision 017](decisions/017-commands-and-output.md))

**In plain words.** Bolt uses its own friendly words (`save`, `goto`, `combine`…) but quietly accepts Git's words too, so Git users aren't stuck. Every error explains what happened, why, and what to do next.

| When | Commands (Git word) |
|---|---|
| Starting | `start` (init) · `setup` · `import` · `export` (before 1.0) |
| Every day | `status` · `changes` (diff) · `save` (commit) · `save --amend` (commit --amend) · `discard` (restore) · `undo` / `redo` · `shelf` / `unshelf` (stash) |
| History | `history` (log) · `show` · `who` (blame; v0.2) · `goto` (checkout/switch) · `reverse` (revert; v0.3) |
| Lines (v0.2) | `line` (branch) · `combine` (merge) · `resolve` · `tag` |
| Team (v0.3) | `copy` (clone) · `get` (pull) · `send` (push) · `lock` / `unlock` / `locks` · `login` · `pin` |
| Upkeep | `verify` (fsck) · `tidy` (gc) |

- **Output:** `--json` on every command; colors and progress only in a terminal; no questions without a terminal; examples first in help.
- **`--json` formats:** every document, field and kind of warning is in [json.md](json.md).

### 5.16 Importing from Git (milestone 2)

**In plain words.** `bolt import` turns a Git repository's whole history into Bolt saves, in place next to `.git/`, without changing the Git repository. Bolt reads Git's files itself, with the same care it takes with its own: nothing in them is trusted, every read is bounded, and big files flow straight through.

- **Already decided:** every branch becomes a line and every tag a tag; each save records its Git commit in the extra field `git-commit`; paths that aren't valid UTF-8 stop the import with a list of where they occur; messages in other encodings are converted to UTF-8, keeping the original bytes; `.bolt` entries in Git trees are skipped and the summary lists them, a `.bolt` folder Git checked out below the top gets a warning, and one Git tracks at the top stops the import before anything is written, since Bolt never takes files Git wrote for its own; up to 1,024 parents ([decision 019](decisions/019-review-clarifications.md)). Submodules become project links ([decision 004](decisions/004-project-contents.md)); LFS contents must be on disk ([decision 020](decisions/020-lfs-import.md), milestone 5).
- **Reading Git** ([decision 031](decisions/031-own-git-reader.md)): Bolt's own read-only reader, for loose objects, packs and their deltas, refs, and from `.git/config` only the format lines (plus `core.sparseCheckout`, only to refuse); SHA-1 and SHA-256 repositories.
  - No memory-mapping, and every read is bounded. Files in `.git` are opened without following links, objects Git stored whole stream straight into Bolt's pack, and every Git object's ID is checked as it is read (SHA-1 with collision detection).
  - A deltified object and its base may each be at most 512 MiB, which Git never exceeds by default; so may any object read whole into memory. A file Git stored whole streams through at any size, and a loose object may say it holds at most 1 TiB.
  - A delta chain may be at most 4,096 deep (Git's own cap is 4,095), and one that loops is refused. Bases rebuilt along the way are kept for the next delta built on them, at most 256 MiB of them.
  - Packs are read from `objects/pack`, each `pack-*.idx` with its `.pack`, which must match it. A file without its partner is a pack Git is still writing or removing, and is skipped, as Git skips it. An index's tables of names and offsets stay in memory (28–40 bytes an object), read a buffer at a time once its length fits its object count. `objects/pack` may hold at most 100,000 entries. Between reads, at most 16 packs stay open (the ones used last), few enough for any system's limit on open files.
  - A pack or loose object that Git removes or rewrites during the import stops it as a change, not as damage: run it again once Git has finished. A pack Bolt already has open goes on being read as it was.
  - `.git/config` and `.git/config.worktree` may each be at most 1 MiB.
  - `HEAD` and each loose ref may be at most 4 KiB.
  - `.git/packed-refs` may be at most 512 MiB, room for millions of refs.
  - It reads at most 7,600,000 entries of `.git/refs` and `packed-refs` (folders, files and refs): room for every ref decision 033's limits let an import bring in, and a million more it only lists.
  - The ref names and folder paths it keeps while reading refs may take at most 512 MiB. This is separate from the limit on `packed-refs`, which is read whole, so the two limits add up to 1 GiB of memory at the peak of reading refs, before the maps that hold the refs add their own overhead.
  - It never runs Git or Git LFS and never fetches. It ignores replace refs, grafts and Git's speed-up files, and warns when the repository has a grafts file.
  - It refuses, before writing anything and saying what to do: unknown formats or extensions, reftable, refs kept outside `.git`, partial and shallow clones, alternates, sparse checkouts, pack indexes in Git's old version 1 format, a `.git` that is a file, and a `.git` another account owns.
- **Commits become saves exactly** ([decision 032](decisions/032-exact-repeatable-commit-import.md)).
  - Tree, parents (in Git's order), author, committer (as the saver) and message carry over; imported saves get no `line` note, since Git never recorded where a commit was made.
  - Whatever the save's own fields can't rebuild (a signature, a signed merge, extra or unusual header lines) stays in the extra field `git-header`. Import checks that every commit rebuilds to its own Git ID, so a later export can give back the same IDs.
  - A save's change ID comes from its commit (Jujutsu's rule, or the `change-id` header Jujutsu writes), so importing the same repository again, on any computer, gives identical saves.
  - A commit with a message over 1 MiB or a header over 64 KiB stops the import; commits whose trees an old tool wrote in a non-standard form are counted, since export can't recreate their IDs.
- **Branches become lines** ([decision 033](decisions/033-git-branches-to-lines.md)).
  - Your own branches become lines.
  - Each server's branches, as Git last saw them, become server lines (`feature-x@origin`); they move only with another import.
  - Names Bolt's rules refuse are renamed by a fixed rule, which the summary lists: each character Bolt doesn't allow becomes `-`, a name over 255 bytes is shortened, and a clash gets `-2`, `-3` and so on. Names that need no change keep them, as does the first in byte order of names that differ only by letter case, so the suffixes go to renamed names and to the later names of such a group. The same repository always gets the same names.
  - You stand where Git did: on its current branch's line, or at the save of a detached `HEAD`.
  - The stash, notes, replace refs and other refs aren't imported; the summary lists them. A repository with no commits, or a current branch with none yet, stops the import.
- **Names Bolt can't take as they are** ([decision 035](decisions/035-git-names-bolt-cant-take.md)). Dangerous names are skipped, with everything under them, and the summary lists them: any spelling a system would treat as `.git` (Git's own list, on every system, as Git for Windows applies it: its check for macOS spellings also ends a name at a backslash), `.`, `..`, empty names and names holding `/`. Names over 255 bytes and broken folders (duplicate names, wrong order, unknown modes, contents that don't match their ID, missing objects) stop the import. A snapshot of more than 4,096 folder levels or 10,000,000 entries (counting every path) stops it too, and so does a single Git tree of more than 1,000,000 entries, which is more than a Bolt folder may hold. Commits with skipped entries are counted, since export can't recreate them exactly.
- **Submodules become bare links** ([decision 036](decisions/036-submodules-as-bare-links.md)): a project link holding only the commit. `.gitmodules` comes in as an ordinary file and is never read. A later save keeps every link as it is and never looks inside its folder; moving, adding or removing a submodule is done in Git.
- **The first save after an import** ([decision 039](decisions/039-first-save-after-import.md)) stops when files differ from the imported version only by line endings Git converted, offering to have Git write them back or to save them as they are with `bolt save --accept-crlf`. Like every save, it keeps what this computer can't hold exactly as it was (§5.2). Sparse checkouts are refused by the import. Uncommitted work in Git becomes part of the next save.
- **Import is one-time for now** ([decision 040](decisions/040-import-once-catch-up-later.md)): a second `bolt import` refuses; catching up with Git's new commits comes later.
- **Ignore rules carry over by themselves** ([decision 038](decisions/038-gitignore-and-saved-files.md)): Bolt reads `.gitignore` files as well as `.boltignore`, so an import copies nothing, and files Git tracks despite a rule stay saved.
- **Git LFS waits for milestone 5** ([decision 037](decisions/037-lfs-waits-for-milestone-5.md)): until then the import stops at the first LFS pointer (found with Git LFS's own test), naming it, and commits nothing. A blob counts as a pointer when Git LFS's own pointer parser accepts it (the rules of its `lfs/pointer.go`), and it is not empty and at most 1,024 bytes. A file that merely looks like a pointer stops the import too, and the message says so: it is not empty and at most 1,024 bytes, its first line starts with `version https://git-lfs.github.com/spec/v1` or `version https://hawser.github.com/spec/v1`, and it has a line that is `oid sha256:` and 64 hex digits and a line starting `size `, in any order, whatever else it holds.
- **Tags come in exactly** ([decision 034](decisions/034-tags-kept-and-shared.md)). An annotated tag keeps its Git bytes, signature included, beside its Bolt message. A tag of something other than a commit (a folder snapshot, a single file) is skipped and listed.

## 6. Risks and watch-outs

Raised at kickoff. Each is either answered by a topic or tracked here until it is.

1. **Scope and motivation.** The brief describes several products; the realistic failure mode is losing steam long before Bolt is usable. *Response:* a usable v0.1 early (roadmap in §1), then grow toward the full goals; design later layers only as far as the engine must respect them.
2. **Adoption.** Every independent engine of the Git era (Mercurial, Bazaar, Fossil, Darcs, Pijul) stayed niche or faded against Git and GitHub; the newcomers gaining traction (jj, Sapling) work with Git repositories. Bolt is designed as if it would be marketed, so this matters. *Response:* a deliberate first audience (decision 001); Git import in v0.1 and export before 1.0 (decision 002).
3. **Precise claims.** Git is heavily optimized for code repositories; big files are where Bolt can clearly win. Git LFS does have locking (`git lfs lock`), and Git has partial clone (`--filter=blob:none`), which skips old file contents. The real problem is that both are add-ons: awkward, easy to bypass, unevenly supported by hosts.
4. **Dedup depends on file format.** Chunking saves a lot when edits leave most bytes untouched (WAV, uncompressed source assets, appended datasets) and little on already-compressed formats (PNG, JPEG, MP4, ZIP-based files) or a retrained model where every weight changed. *Response:* measure on a real-asset corpus before claiming savings (starting parameters in decision 009; measurement in topic 18).
5. **Shared vs. local settings.** *Answered by decision 008:* project rules in `bolt.toml` + `.boltignore`, declarative only; personal settings are never shared and can never change project rules.
6. **Undo has edges.** *Answered by decisions 003 and 010:* safety copies capture unsaved work; undo restores only your own repository and offers the opposite action for things others received; a 30-day undo window, with an optional size budget for safety copies.
7. **Accidental huge or secret saves.** No staging + "save everything" + big-file support means one save can swallow a 30 GB render folder or a `.env` file. *Response:* new files are tracked automatically and unusually large saves ask first (decision 003); preset ignore rules (topic 16); secret warnings (topic 18).
8. **The working-copy model drives undo, conflicts and shelving.** *Answered by decision 003:* a separate folder with automatic safety copies.
9. **File-model basics** (line endings, executable bit, symlinks, empty folders, path encoding, case and Unicode collisions, nested projects). *Answered by decision 004.*
10. **Windows must be first-class.** Antivirus and editors hold files open, so temp-file-plus-rename fails intermittently and needs retries. Also: reserved names (`CON`, `NUL`, `AUX`, `COM1`…), trailing dots and spaces, 8.3 short names, alternate data streams, long paths, case-insensitivity, symlinks that need special permission. *Name rules and write-out safety: decision 004. Files held open and retries: decision 011.*
11. **Big-file identity.** *Answered by decision 005:* files are identified by their contents, never by their chunk list.
12. **Hashed objects need a hand-specified byte format.** *Answered by decision 005:* a hand-written format with exactly one valid encoding per object; serde only for data that is never fingerprinted.
13. **Renames are guessed** when changes come from comparing snapshots. *Answered by decision 006:* saves carry move notes for moves Bolt is certain about; how merges use them follows in topic 12.
14. **Change IDs.** *Answered by decision 006:* 128 random bits, shown in the letters k–z, with divergence shown explicitly.
15. **Build order.** Answered by decision 001: test infrastructure is built with the object store, a minimal Git importer follows saves and history, and chunked big files come before lines and combine. Locking waits for the server (v0.3).
16. **A data-integrity tool written mostly by an AI while the owner learns Rust.** Subtle bugs can look fine in review. *Answered by decision 022:* the rules are approved and copied into CLAUDE.md, with `unsafe` allowed only in a tiny platform crate for operating-system calls. *Decision 024* adds an attacker's view: a red-team session from an isolated sandbox at the end of every milestone.
17. **Too many open packs.** Every save adds a pack, a big import adds several at once ([decision 041](decisions/041-several-packs-per-command.md)), and the store keeps every pack open, so a long-lived project will hit the operating system's open-file limit. *Response:* merge small packs and open packs lazily before real use (milestone 3).
18. **Our own Git reader meets hostile repositories first.** Readers of Git's formats keep having crash and memory bugs on crafted packs (gitoxide, go-git, libgit2 and dulwich all published some in 2026). *Response:* decision 031's bounds; fuzzing; malicious-repository cases drawn from Git's published vulnerabilities (reviewed 2026-11-02); differential tests against Git; milestone 2's red-team session.

## 7. Open questions and deferred decisions

**Open**: every unchecked agenda topic above, plus:
- **Test data:** real game projects for benchmarks and dedup measurements. Candidates, still to be checked: Unity's open-source "Open Project 1" (with Git LFS history), Godot's demo projects, and scripted series of versions of real assets (decision 019).
- ~~**Objects per pack vs. one pack per command.**~~ *Decided while planning milestone 2* ([decision 041](decisions/041-several-packs-per-command.md)): a command writes several packs when it needs more than one may hold.
- ~~**CI**~~ *Done in milestone 1:* GitHub Actions on Windows, macOS and Linux, with a `.gitattributes` that turns off line-ending conversion; the repository is public, so Actions minutes aren't a constraint.
- **Known limitation: links Git wrote on Windows.** With real links on Windows (`core.symlinks=true`), Git for Windows writes a link's target with `\`. Bolt saves a target as it reads it, so the first save after an import records those links as changed. The owner will decide whether to store `/` in link targets read on Windows, as Git does. Evidence: the nightly job "Links Git for Windows writes keep their target when saved" ([testing.md](testing.md) §3, run 41644016857); its test is ignored on Windows until then.

**Deferred**: not needed for v0.1. The constraint to respect now is in brackets.
- **Name.** "Bolt" collides with bolt.new (a widely used AI app builder) and Puppet Bolt, which installs a `bolt` command. On crates.io, `bolt`, `bolt-core` and `bolt-cli` are taken and `bolt-vcs` is free (checked 2026-10-31); package names can differ from the command name. [Decide before format 1 freezes at the v0.1 release: the name is built into `.bolt/`, `bolt.toml`, `.boltignore` and the pack files' magic bytes. Until then, renaming only means changing constants (decision 019).]
- **Path-length limit** in the format. Depth and entries are limited, but a path can reach about 1 MB (255-byte names, 4,096 levels deep). With it, the 255-byte name limit: Windows allows 255 UTF-16 units, up to 765 bytes in UTF-8, so a Windows project can hold names Bolt refuses, and an import stops at them (decision 035). [Readers never hold every path at once. Decide with milestone 3's write-out, where operating-system path limits apply, and before format 1 freezes at v0.1 (decision 026).]
- **Memory bound for hostile saves.** Inside the format's limits, a hostile save can still make a reader hold about 1.5 to 4 GB. [Revisit with the speed-bar benchmarks before v0.1 (decisions 023, 026).]
- **Contribution licensing**: CLA or DCO. [Matters if the server is AGPL and ever dual-licensed; decide before accepting outside contributions.]
- **GUI and editor plugins** (Unreal, Unity, Godot, Blender). [All logic stays in the engine library so a GUI can embed it.]
- **Interactive save viewer** (`bolt show`, ratatui + crossterm). [After the core works.]
- **Signed saves.** [Room reserved in the save format (decision 006).]
- **Conflict algebra** (jj-style automatic simplification of conflicts). [Planned by the owner, after v0.2; decision 007's format already allows it.]
- **Smart locks** (locks that follow a change until it reaches the integration line). [Planned by the owner, after v0.3; decision 014's lock records allow it.]
- **Folder remotes** (sync with a USB drive or NAS). [May come before the server; no locks there (decision 015).]
- **Folder-level permissions.** [Later; hidden folders act like "not downloaded" (decision 015).]
- **Deltas for small text files.** [Added later as a storage feature through the `.bolt/format` feature list (decision 009).]
- **File watcher** for faster `status`. [After v0.1, optional; it may only narrow what gets checked, never decide correctness (decision 011).]
- **Store on another drive, or shared between projects** (like Git LFS's `lfs.storage` setting). [Later; keep the store's location a single setting internally.]
- **Structural diff and merge** (tree-sitter). [Later.]
- **Virtual filesystem for on-demand files** (ProjFS on Windows, FUSE, macOS File Provider). [Not planned (decision 013); revisit only if measurements of engine start-up reads show it would pay off.]
- **Partial workspaces** (choose which folders to download at all). [After v0.3 (decision 013).]
- **Web platform and asset previews.** [After the server.]
- **Live two-way Git sync.** [Not promised (decision 002); every imported save records its Git commit (`git-commit`), so the mapping between the two can always be rebuilt (decision 019).]
- **Perforce import.** Many game studios use Perforce; a natural second importer after Git. [Later.]
- **Nested projects.** [A folder with its own `.bolt/` is left alone, and Git submodules import as project links (decision 004).]
- **Opt-in line-ending conversion** for specific files. [Only if real projects need it; the default stays bytes in = bytes out (decision 004). Until then the first save after an import stops at files Git converted (decision 039).]
- **Catching up with Git.** A second import that adds new commits and moves lines that haven't moved in Bolt since the last import. [Imports are repeatable, so it is small (decision 032); before v0.1 if the milestones leave room, otherwise after (decision 040).]
- **Importing reftable repositories.** [The reader refuses them, saying how to make a copy it can read; teach it the format before Git 3.0 makes reftable common (decision 031).]
- **Change-ID letters like Jujutsu's.** Bolt writes 0 as `k` and 15 as `z`; Jujutsu, whose change IDs imported saves share, writes them the other way round. [Display only, no stored data changes; decide before v0.1 (decision 032).]

## 8. Prior art

Learn from these; don't copy them.

| System | What to learn |
|---|---|
| Git | Snapshots + packfiles with deltas; content addressing; the index; weak with big files |
| Mercurial | Revlog: deltas with periodic full snapshots; the `requires` file for format features |
| Jujutsu (jj) | Change IDs, operation log, first-class conflicts, working copy as a commit |
| Pijul | Patch theory: changes, not snapshots, as the primitive |
| Sapling (Meta) | Scalability; the Mononoke server; lazily fetched history |
| Perforce, Unity Version Control | Big files, locking, typemaps (per-file storage rules) |
| Diversion | Cloud VCS for games; syncs with Git and Perforce |
| Epic's Lore | Open-source engine for large projects (early stage; not studied yet) |
| Anchorpoint | Git + LFS made friendly for artists, auto-configured per engine |
| Hugging Face Xet | Content-defined chunking at scale for ML files, replacing Git LFS on the Hub |
| restic, borg | Content-defined chunking for backups (FastCDC and relatives) |
| Git partial clone, sparse checkout, Scalar | Git's own answer to very large repositories |
| Fossil | A whole repository in one SQLite file |

## 9. Glossary

Working names; command names can still change (topic 17).

- **Repository**: the history database, kept in `.bolt/`.
- **Working folder**: the files you actually edit.
- **Save**: one recorded version of the whole project (Git: commit). A photo of every file at one moment.
- **Change ID**: a stable name for a save that survives edits and rewrites, written in the letters k–z (`kxqmz`). Editing a save changes its fingerprint, not its change ID.
- **Conflict**: two lines of work changed the same thing differently. Bolt keeps every version in the save so you can fix it later.
- **Divergent**: two saves that share one change ID, usually because two people edited the same save; Bolt shows both until you keep one.
- **Move note**: a record inside a save that a file moved, kept only when Bolt is certain.
- **Pack**: a file in `.bolt/store/` holding many pieces of data plus an index, written in one go so a crash can't leave half of it.
- **Undo window**: how far back undo reaches; 30 days by default.
- **Cache**: remembered facts that speed Bolt up, like file sizes and times; always safe to delete.
- **Pinning**: marking saves or folders so they stay available offline.
- **Lock**: a "someone's working on this" claim on a file that can't be merged; while you hold it, nobody else can send changes to that file.
- **Remote**: another copy you sync with: a server, or a plain folder such as a USB drive.
- **Preset**: a starter set of project rules for one kind of project (Code, Game…), written into `bolt.toml` and `.boltignore` by `bolt start`.
- **Hash**: a fingerprint of data. Change one byte and the fingerprint changes (BLAKE3).
- **Blob**: the contents of one file, stored under its hash.
- **Tree**: a folder listing: names pointing to blobs or to other trees.
- **Chunk**: a piece of a big file. Identical pieces are stored once.
- **Chunk list**: the list of pieces a big file was cut into. Stored separately, so the file's ID never depends on it.
- **Line**: a named line of work (Git: branch); a movable name that points at the latest save made on it.
- **Tag**: a fixed name for an important save, like `v1.0`; it never moves.
- **Operation log**: a diary of every command that changed the repository; it is what makes undo possible.
- **Project rules**: settings the whole team shares, kept in `bolt.toml` and `.boltignore` and saved in history like normal files.
- **Personal settings**: your own preferences (name, editor, merge tools); never shared, and they can't change project rules.
- **Safety copy**: a copy of unsaved work that Bolt keeps automatically before a command overwrites it, so undo can bring it back.
- **Shelf**: a place to set unsaved changes aside and bring them back later (Git: stash).
- **Symlink**: a file that points to another file or folder, like a shortcut.
