# Bolt Milestones

> **Status:** design-phase plan (2026-10-31), [decision 018](decisions/018-milestone-plan.md). Milestone 1 is broken into small steps below; each later milestone gets its own step list when it starts.

## v0.1: solo, local, games first

| # | Milestone | Done when |
|---|---|---|
| 1 | **First save** | `bolt start`, `bolt save`, `bolt history` and `bolt show` work end to end on real projects, on the real formats, on all three systems |
| 2 | **Coming from Git** | `bolt import` brings in a Git repository's history (every branch and tag, in place next to `.git/`), and differential tests compare every commit with Git |
| 3 | **Working folder** | `status` (with the file-state cache), `changes`, `goto`, `discard`, partial saves, safety copies, and basic lines: saving after `goto` creates one, `line` lists them, `goto <line>` switches |
| 4 | **Undo everything** | The diary drives `undo`, `redo`, `undo --list` and `undo --to`; crash recovery; `tidy` |
| 5 | **Big files** | Chunking, streaming, parallel hashing, Git LFS import, and benchmarks against Git LFS |
| 6 | **Presets and polish → v0.1** | The `bolt start` screen with the Code, Game (Unity, Unreal, Godot) and Custom presets; `bolt setup`; Git-word shortcuts; `--json`; `verify`; packaging |

After v0.1: v0.2 adds combining lines (plus renaming and deleting them); v0.3 adds syncing and locking ([design.md](design.md) §1).

## Milestone 1: First save

Each step is small and ends with tests passing on Windows, macOS and Linux. Steps are done in order.

**0. Tools and project skeleton**
- Install Rust (`rustup`, stable toolchain; on Windows, the MSVC build tools).
- Add a `.gitattributes` that turns off line-ending conversion.
- Create a Cargo workspace with the engine library and the CLI. The command is `bolt`; package names are chosen so they can be published later, since `bolt` and `bolt-core` are taken on crates.io.
- Turn the engineering rules in [security.md](security.md) §5 (once approved) into compiler and clippy settings.
- Add the GitHub Actions workflow from [testing.md](testing.md) §4.
- *Done when:* a placeholder test passes in CI on all three systems.

**1. Fingerprints**
- The ID type, hex display and parsing, plain BLAKE3 and flavored hashing with the context strings ([storage-format.md](storage-format.md) §2).
- *Done when:* plain hashing matches the official BLAKE3 test vectors; flavored hashing has its own vectors; display and parsing round-trip.

**2. Encoding building blocks**
- A strict writer and reader for the pieces in storage-format.md §1.
- *Done when:* round-trip property tests pass; non-minimal varints, bad UTF-8, bad booleans and trailing bytes are rejected; a fuzz target runs.

**3. Names and paths**
- Decision 004's always-on name rules and the portability check, in one module. The Windows rules are checked against Microsoft's documentation first.
- *Done when:* table-driven tests cover every rule, including Windows device names, `.bolt` spellings and case or Unicode collisions; a fuzz target runs.

**4. Folder listings**
- Encode, decode and validate (storage-format.md §3).
- *Done when:* test vectors, round-trip tests, rejection tests (unsorted, duplicate, bad kind, bad name), and a fuzz target.

**5. Saves and change IDs**
- Encode and decode saves (§4); create change IDs and show them in the letters k–z (§5); find a save from a short prefix.
- *Done when:* test vectors, round-trip tests, display tests and ambiguous-prefix tests.

**6. Pack files**
- Write a pack in `tmp/`, flush it, rename it into the store; read it back with every check in storage-format.md §11; zstd with a hard size limit.
- *Done when:* round-trip tests pass; flipping any single byte of a pack is detected; a writer killed halfway leaves the store unchanged; a decompression bomb is refused.

**7. Object store**
- Store, fetch and check objects across many packs, skip objects that are already stored, and keep a rebuildable lookup cache in `cache/`.
- An internal integrity check that walks every pack and object: the core of the later `bolt verify` command (milestone 6), used by tests from now on.
- *Done when:* property tests with random stores and fetches pass; deleting `cache/` changes nothing but speed; the integrity check catches every corruption the pack tests introduce.

**8. Diary, head and the lock**
- Encode and decode diary entries (§9); replace `state/head` atomically; take the OS lock for changing commands, with the "waiting" message.
- *Done when:* a crash between writing the pack and moving `head` leaves the repository exactly as before; two changing commands run one after the other; read-only commands never wait.

**9. `bolt start`**
- Create `.bolt/` (the format file, the folders, hidden on Windows), the empty first save and the line `main`, recorded in the first diary entry; refuse to start inside another project. No presets yet (milestone 6).
- *Done when:* CLI tests pass on all three systems, and opening a repository with an unknown feature is refused with a clear message.

**10. `bolt save`**
- Walk the folder (respecting `.boltignore`, skipping `.bolt/`), store every file whole (streamed, never loaded fully into memory; chunking arrives in milestone 5 and changes no IDs), build folder listings from the bottom up, and write the save and its diary entry in one pack.
- Record executable flags, symlinks and empty folders; refuse non-portable new names (decision 004); say so when nothing changed.
- *Done when:* every saved file reads back byte-identically from the store; a property test with random folders passes; the malicious-name cases are refused.

**11. `bolt history` and `bolt show`**
- `history` lists the saves on the current line (change ID, time, author, message). `show` lists what a save changed: added, removed and modified files with their sizes. Line-by-line differences come in milestone 3.
- *Done when:* snapshot tests cover the output for people and the `--json` output.

**12. Check and review**
- Run the malicious-repository cases that apply so far and every fuzz target for a while; the owner reviews all code that writes to disk.
- *Done when:* everything passes on all three systems, and `start`, `save`, `history` and `show` work on one of the owner's code projects and on an open-source Unity project.
