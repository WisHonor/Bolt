# Bolt — Design Spec

> **Living document.** It always describes the *current* design; the reasons behind each choice live in [`decisions/`](decisions/).
> **Phase:** design, no implementation code yet · **Started:** 2026-10-31 · **Decisions recorded:** 7

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
- **Settings reach teammates.** The rules go in the shared project file, so everyone who copies the project gets the same behaviour (topics 8, 16).
- **Presets know the engine.** Unity, Unreal and Godot differ in which files must be locked (Unity: scenes and prefabs; Unreal: `.uasset`, `.umap`) and which cache folders must be ignored (Unity's `Library/`, `Temp/` and `Logs/` can hold gigabytes and must never be saved). Detection applies ignore rules before reporting sizes.
- **Detection only reads names and sizes.** It never runs anything from the project.
- **Locking and on-demand download need a server**, so they arrive in v0.3; until then the rules are recorded but not enforced.
- **Scripts need a non-interactive form**, e.g. `bolt start --preset game`.

**First audience and roadmap** ([decision 001](decisions/001-first-audience-and-v0.1.md))
- **First audience:** game developers. The engine stays general; games set the priorities, first presets and benchmarks.
- **Main platform:** Windows. macOS and Linux are supported and tested in CI from the start.
- **v0.1, solo and local-only:** `start`, `save`, `status`, `changes`, `history`, `show`, `goto`, `discard`, `undo`/`redo`, Git import including Git LFS files ([decision 002](decisions/002-git-compatibility.md)), chunked big files. Big files are streamed, never loaded whole into memory.
- **v0.2:** lines and combine (branches, merging, conflicts).
- **v0.3:** sync and locking for small teams; big files and old versions download on demand.
- **Before 1.0:** Git export ([decision 002](decisions/002-git-compatibility.md)). Live two-way Git sync is not promised, but nothing may block it.
- **Later:** conflict algebra on top of decision 007, hosting platform, GUI and editor plugins. Which presets ship in which version is decided in topic 16.

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
| T2 | Users see snapshots; storage may use deltas with periodic full copies | Identity **confirmed** by decision 005: IDs never depend on how data is stored; storage strategy in topic 9 | 5, 9 |
| T3 | BLAKE3 hashing, zstd compression | BLAKE3 **confirmed** by decision 005; zstd details in topic 9 | 5, 9 |
| T4 | Objects: blobs, trees, saves, refs, plus an operation log | Mostly keep; chunk lists added by decision 005; conflict entries in topic 7; mutable state (lines, operation log) in topics 8 and 10 | 5–7, 10 |
| T5 | A save holds author, timestamp, message, parents, root tree, stable change ID | **Confirmed** with additions by decision 006: author and saver with time zones, 128-bit random change ID, move notes, signature slot, extra fields | 6 |
| T6 | No staging area; `bolt save` saves everything, `bolt save <files>` saves some | **Confirmed** by decision 003: new files tracked automatically; unusually large saves ask first | 3 |
| T7 | Operation log; `undo`/`redo` work on any action | Keep; define the edges (data already sent, discarded files, how long undo lasts) | 10 |
| T8 | Conflicts stored inside saves ("sticky notes") | **Confirmed** by decision 007: every version kept; conflict algebra planned on top later | 7 |
| T9 | Content-defined chunking above ~1 MB; big files and old versions on demand; full small-file history local; opt-in locking | Big files are identified by their contents (decision 005); chunking details in topic 9; dedup gains depend on file format | 5, 9, 13, 14 |
| T10 | Presets at `bolt start`, written to `.bolt/config.toml`, changeable via `bolt setup` | Keep the idea; rules teammates need must live in a shared, versioned, declarative file, not in `.bolt/` | 8, 16 |
| T11 | On-disk format version number from day one | Keep; consider a feature list as well (like Mercurial's `requires` file) | 8 |
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
- [ ] 8. Repository layout in `.bolt/`; shared vs. local config; format version, feature list, upgrades
- [ ] 9. Storage engine: loose vs. packed objects, compression, deltas and full-copy cadence, chunking algorithm and thresholds, garbage collection
- [ ] 10. Operation log: undo/redo semantics per command, two commands at once, how long undo lasts
- [ ] 11. Fast `status` and safe `goto`: file-state cache, timestamp traps, file watching, never overwriting unsaved work

**Stage 4 — Collaboration** (constraints only, for now)
- [ ] 12. Lines and combining: naming, remote lines, tags, three-way merge, renames
- [ ] 13. Partial download and offline behaviour
- [ ] 14. Locking semantics
- [ ] 15. Sync protocol, server storage model, permissions

**Stage 5 — Product surface and wrap-up**
- [ ] 16. Presets and the shared project file: detection, preset contents, per-file rule syntax
- [ ] 17. Commands, names and aliases; error and output style (human-readable and `--json`)
- [ ] 18. Deliverables: storage format spec draft, threat model and security plan, test plan, milestone-1 task list; a fresh-eyes review of this document before sign-off

## 5. Design

The current design, by area. Each part links to the decision that set it.

### 5.1 Your project folder and saves ([decision 003](decisions/003-working-copy-model.md))

**In plain words.** Your project folder is separate from history: nothing enters history until you run `bolt save`. But Bolt never destroys unsaved work without keeping a copy first, so `undo` can always bring it back. It works like an IDE's local history, built into the version control system.

- **Saving.** `bolt save "msg"` records every change in the folder; `bolt save <files or folders> "msg"` records only those. Choosing parts of a file comes later.
- **Tracking.** New files are tracked automatically (there is no `add` step) unless ignore rules exclude them. A save that adds unusually much at once asks for confirmation first; the threshold is set per preset (topic 16), and a flag skips the question for scripts.
- **Safety copies.** Before any command overwrites or removes unsaved work (`discard`, `goto`, `combine`, `undo`, `redo`, `shelf`), Bolt stores the current content of the affected files and records it in the operation log. Only files about to be touched are copied. Safety copies live in the normal object store, so content that is already stored costs nothing extra.
- **Shelf.** `bolt shelf` sets unsaved changes aside as a hidden save and restores the folder; shelving is undoable.
- **`goto` with unsaved changes.** Changes come along when nothing collides; otherwise Bolt stops and offers to shelve, save or discard them.
- **Conflicts.** A save can contain unresolved conflicts, and the save records them (topic 7).
- **`status`** only reads; it never writes to the store.

### 5.2 What a project can contain ([decision 004](decisions/004-project-contents.md))

**In plain words.** Bolt stores exactly what's in your project and never changes a file's bytes. It checks new names when you save, so a project that works on Linux won't break on Windows or a Mac, and it refuses to write anything dangerous when it puts files back on disk.

- **Entry kinds:** regular file, executable file, symlink, folder (empty folders included), and a link to another project (from Git submodules; not fetched).
- **Bytes in = bytes out:** no line-ending conversion or any other change to contents by default.
- **Not stored:** timestamps, Windows attributes, other permissions. On Windows the executable flag keeps whatever history says.
- **Names (always):** valid UTF-8, `/` between folders, no empty, `.` or `..` parts, no absolute paths, and no part that Windows or macOS would treat as `.bolt`.
- **Portability check on save:** new names that would break on Windows or macOS are refused (differences only in letter case or Unicode normalization, reserved device names, trailing dots or spaces, forbidden characters), unless the project relaxes the rule. Imported histories keep their names.
- **Safety on write-out:** never through a symlink, never outside the project, never to a name the OS would misread; such entries are reported instead. Symlinks that leave the project, or that the OS can't create, become placeholder files with a warning.
- **Nested projects:** a folder with its own `.bolt/` is left alone by the outer project.

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
- **Change IDs:** 128 random bits, shown in the letters k–z (never confused with hex fingerprints), as the shortest unique prefix of at least 4 letters; kept across edits; marked "divergent" when two visible saves share one.
- **Move notes:** recorded only when Bolt is certain (same contents moved, or a Unity `.meta` file moved with its asset); used by diffs, merges and locks; they never change the snapshot.

### 5.5 Conflicts ([decision 007](decisions/007-conflicts.md))

**In plain words.** When two lines of work change the same thing differently, Bolt keeps every version in the save, like a sticky note, so you can keep working and fix it later. Nothing is lost, and it works for binary files too.

- **Record:** a folder-listing entry of kind "conflict" points to a record with the sides and the ancestors between them (any of which can be "absent"), plus your half-fixed version if you started fixing it.
- **In your folder:** text files get standard markers with the ancestor section; binary files keep your version in place, with the other versions kept inside `.bolt/`. Presets decide per file type (the Unity preset treats scenes, prefabs and `.meta` files like binaries).
- **Fixed** when the markers are gone, or when you mark the file as fixed.
- **Planned later:** conflict algebra, jj-style automatic simplification of conflicts. The record's general form allows it without a format change.

## 6. Risks and watch-outs

Raised at kickoff. Each is either answered by a topic or tracked here until it is.

1. **Scope and motivation.** The brief describes several products; the realistic failure mode is losing steam long before Bolt is usable. *Response:* a usable v0.1 early (roadmap in §1), then grow toward the full goals; design later layers only as far as the engine must respect them.
2. **Adoption.** Every independent engine of the Git era (Mercurial, Bazaar, Fossil, Darcs, Pijul) stayed niche or faded against Git and GitHub; the newcomers gaining traction (jj, Sapling) work with Git repositories. Bolt is designed as if it would be marketed, so this matters. *Response:* a deliberate first audience (decision 001); Git import in v0.1 and export before 1.0 (decision 002).
3. **Precise claims.** Git is heavily optimized for code repositories; big files are where Bolt can clearly win. Git LFS does have locking (`git lfs lock`), and Git has partial clone (`--filter=blob:none`), which skips old file contents. The real problem is that both are add-ons: awkward, easy to bypass, unevenly supported by hosts.
4. **Dedup depends on file format.** Chunking saves a lot when edits leave most bytes untouched (WAV, uncompressed source assets, appended datasets) and little on already-compressed formats (PNG, JPEG, MP4, ZIP-based files) or a retrained model where every weight changed. *Response:* measure on a real-asset corpus before claiming savings (topic 9).
5. **Shared vs. local settings.** `.bolt/` is private and never copied, but chunk, lock and ignore rules must reach every teammate. *Response:* a versioned, declarative project file plus private local config (topics 8, 16). The shared file comes from strangers on `copy`, so it must never run commands or point to programs; Git keeps hooks and filter commands local for this reason.
6. **Undo has edges.** Undo can't recall what others already received, and undo history keeps old data alive. Undoing `discard`/`goto` is answered by decision 003: safety copies capture unsaved work before it is overwritten. *Response:* explicit semantics per command, an undo window and a cleanup policy (topic 10).
7. **Accidental huge or secret saves.** No staging + "save everything" + big-file support means one save can swallow a 30 GB render folder or a `.env` file. *Response:* new files are tracked automatically and unusually large saves ask first (decision 003); preset ignore rules (topic 16); secret warnings (topic 18).
8. **The working-copy model drives undo, conflicts and shelving.** *Answered by decision 003:* a separate folder with automatic safety copies.
9. **File-model basics** (line endings, executable bit, symlinks, empty folders, path encoding, case and Unicode collisions, nested projects). *Answered by decision 004.*
10. **Windows must be first-class.** Antivirus and editors hold files open, so temp-file-plus-rename fails intermittently and needs retries. Also: reserved names (`CON`, `NUL`, `AUX`, `COM1`…), trailing dots and spaces, 8.3 short names, alternate data streams, long paths, case-insensitivity, symlinks that need special permission. *Name rules and write-out safety: decision 004. Files held open and retries: topic 11.*
11. **Big-file identity.** *Answered by decision 005:* files are identified by their contents, never by their chunk list.
12. **Hashed objects need a hand-specified byte format.** *Answered by decision 005:* a hand-written format with exactly one valid encoding per object; serde only for data that is never fingerprinted.
13. **Renames are guessed** when changes come from comparing snapshots. *Answered by decision 006:* saves carry move notes for moves Bolt is certain about; how merges use them follows in topic 12.
14. **Change IDs.** *Answered by decision 006:* 128 random bits, shown in the letters k–z, with divergence shown explicitly.
15. **Build order.** Answered by decision 001: test infrastructure is built with the object store, a minimal Git importer follows saves and history, and chunked big files come before lines and combine. Locking waits for the server (v0.3).
16. **A data-integrity tool written mostly by an AI while the owner learns Rust.** Subtle bugs can look fine in review. *Proposed rails, pending the owner's approval:* spec before code for anything on disk, test vectors for every format, `#![forbid(unsafe_code)]` in the engine, no panics in library code, small changes, and the owner reads every module that writes to disk.

## 7. Open questions and deferred decisions

**Open**: every unchecked agenda topic above, plus:
- **Test data:** real game projects (open-source Unity projects first) for benchmarks and dedup measurements (topics 9, 18).
- **CI:** GitHub Actions on Windows, macOS and Linux, set up as the first coding step (during design there is nothing to test). Private repositories get a limited free Actions allowance that Windows and macOS runners use up faster; revisit repository visibility then. In the same step, add a `.gitattributes` that stops Git converting line endings in this repo, so byte-exact test files stay byte-exact on Windows.
- **Very large safety copies:** cap their size, or expire them sooner than other undo data? (topic 10)

**Deferred**: not needed for v0.1. The constraint to respect now is in brackets.
- **Name.** "Bolt" collides with bolt.new (a widely used AI app builder) and Puppet Bolt, which installs a `bolt` command. On crates.io, `bolt`, `bolt-core` and `bolt-cli` are taken and `bolt-vcs` is free (checked 2026-10-31); package names can differ from the command name. [Decide before any public release: the `.bolt/` folder name gets baked into every repository.]
- **Contribution licensing**: CLA or DCO. [Matters if the server is AGPL and ever dual-licensed; decide before accepting outside contributions.]
- **GUI and editor plugins** (Unreal, Unity, Godot, Blender). [All logic stays in the engine library so a GUI can embed it.]
- **Interactive save viewer** (`bolt show`, ratatui + crossterm). [After the core works.]
- **Signed saves.** [Room reserved in the save format (decision 006).]
- **Conflict algebra** (jj-style automatic simplification of conflicts). [Planned by the owner, after v0.2; decision 007's format already allows it.]
- **Structural diff and merge** (tree-sitter). [Later.]
- **Virtual filesystem for on-demand files** (ProjFS on Windows, FUSE, macOS File Provider). [Very hard and OS-specific; not before v1.]
- **Web platform and asset previews.** [After the server.]
- **Live two-way Git sync.** [Not promised (decision 002); the importer's commit-mapping table keeps the door open.]
- **Perforce import.** Many game studios use Perforce; a natural second importer after Git. [Later.]
- **Nested projects.** [A folder with its own `.bolt/` is left alone, and Git submodules import as project links (decision 004).]
- **Opt-in line-ending conversion** for specific files. [Only if real projects need it; the default stays bytes in = bytes out (decision 004).]

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
- **Hash**: a fingerprint of data. Change one byte and the fingerprint changes (BLAKE3).
- **Blob**: the contents of one file, stored under its hash.
- **Tree**: a folder listing: names pointing to blobs or to other trees.
- **Chunk**: a piece of a big file. Identical pieces are stored once.
- **Chunk list**: the list of pieces a big file was cut into. Stored separately, so the file's ID never depends on it.
- **Line**: a named line of work (Git: branch).
- **Operation log**: a diary of every command that changed the repository; it is what makes undo possible.
- **Safety copy**: a copy of unsaved work that Bolt keeps automatically before a command overwrites it, so undo can bring it back.
- **Shelf**: a place to set unsaved changes aside and bring them back later (Git: stash).
- **Symlink**: a file that points to another file or folder, like a shortcut.
