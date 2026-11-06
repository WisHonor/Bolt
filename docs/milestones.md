# Bolt Milestones

> **Status:** design-phase plan (2026-10-31), [decision 018](decisions/018-milestone-plan.md). Milestone 1 is broken into small steps below; each later milestone gets its own step list when it starts. **Milestone 1 is done** (2026-11-01; plan: [plans/2026-10-31-milestone-1-first-save.md](plans/2026-10-31-milestone-1-first-save.md)). The fixes from its two red-team sessions (2026-11-01 and 2026-11-02) are done too: plan [plans/2026-11-01-red-team-m1-fixes.md](plans/2026-11-01-red-team-m1-fixes.md), rulings in decisions [025](decisions/025-red-team-m1-rulings.md) to [029](decisions/029-red-team-m1-final-review-rulings.md). **Milestone 2 is built** (2026-11-03), and both its red-team sessions are done (2026-11-03, rulings in decisions [043](decisions/043-red-team-m2-rulings.md) and [044](decisions/044-red-team-m2-final-rulings.md)); its final review is done too, and it was merged into `main` on 2026-11-04. **Milestone 2 is done** (2026-11-04): the owner imported one of their own repositories and tried a project inside a OneDrive folder; the results are below its steps. It was planned on 2026-11-02 after decisions [030](decisions/030-administrators-only-when-elevated.md) to [041](decisions/041-several-packs-per-command.md), and [decision 042](decisions/042-trust-hint-skips-folders-anyone-can-write-to.md) settled one more question; its steps are below and its plan is [plans/2026-11-02-milestone-2-coming-from-git.md](plans/2026-11-02-milestone-2-coming-from-git.md). **Milestone 3 was planned on 2026-11-05** after decisions [048](decisions/048-handle-relative-file-access.md) to [059](decisions/059-bolt-spellings-left-out-like-git-spellings.md); its steps are below and its plan is [plans/2026-11-05-milestone-3-working-folder.md](plans/2026-11-05-milestone-3-working-folder.md). **Milestone 3 is done** (2026-11-06): both its red-team sessions and its whole-branch review are done, and it was merged into `main` after the owner approved its `unsafe` code. The OneDrive "Free up space" check of decision 069 is still to be tried. Items milestone 1's reviews deferred, and the ones milestone 2 deferred, are in [plans/2026-11-01-milestone-1-follow-ups.md](plans/2026-11-01-milestone-1-follow-ups.md); each later plan pulls in the ones that apply.

## v0.1: solo, local, games first

| # | Milestone | Done when |
|---|---|---|
| 1 | **First save** ✅ | `bolt start`, `bolt save`, `bolt history` and `bolt show` work end to end on real projects, on the real formats, on all three systems |
| 2 | **Coming from Git** ✅ | `bolt import` brings in a Git repository's history (every branch and tag, in place next to `.git/`), and differential tests compare every commit with Git; the owner has imported one of their own repositories and tried a project inside a OneDrive folder ([decision 026](decisions/026-ownership-check-details.md) #4) |
| 3 | **Working folder** ✅ | `status` (with the file-state cache), `changes`, `goto`, `discard`, partial saves, safety copies, and basic lines: saving after `goto` creates one, `line` lists them, `goto <line>` switches; merge small packs and open packs lazily before real use (each save adds a pack, and every pack stays open) |
| 4 | **Undo everything** | The diary drives `undo`, `redo`, `undo --list` and `undo --to`; crash recovery; `tidy`; `shelf`/`unshelf` and `save --amend` ([decision 021](decisions/021-v0.1-command-scope.md)) |
| 5 | **Big files** | Chunking, streaming, parallel hashing, Git LFS import (contents must be on disk, [decision 020](decisions/020-lfs-import.md)), and benchmarks against Git LFS |
| 6 | **Presets and polish → v0.1** | The `bolt start` screen with the Code, Game (Unity, Unreal, Godot) and Custom presets; `bolt setup`; Git-word shortcuts; `--json`; `verify`; packaging |

**Every milestone also ends with a red-team session** ([decision 024](decisions/024-red-team-gate.md)): a fresh agent attacks the milestone's code from an isolated sandbox, and every proven finding is fixed (or ruled on in writing) before the milestone is merged. Milestone 1 had two sessions, on 2026-11-01 and (on the fixes) 2026-11-02; every finding is fixed. Milestone 2 had two as well, both on 2026-11-03, the second on the fixes; every proven finding is fixed or ruled on in writing (decisions 043 and 044).

**v0.1 ships when** every test passes on the three systems and both speed gates pass ([decision 023](decisions/023-v0.1-speed-bar.md)): saving a 10 GB file uses at most about 500 MB of memory, and `status` on an unchanged 100,000-file project takes under 2 seconds on an SSD. Benchmark results against Git and Git LFS are published either way.

After v0.1: v0.2 adds combining lines (plus renaming and deleting them); v0.3 adds syncing and locking ([design.md](design.md) §1).

## Milestone 1: First save

Each step is small and ends with tests passing on Windows, macOS and Linux. Steps are done in order.

**0. Tools and project skeleton**
- Install Rust (`rustup`, stable toolchain; on Windows, the MSVC build tools).
- Add a `.gitattributes` that turns off line-ending conversion.
- Create a Cargo workspace with the engine library and the CLI. The command is `bolt`; package names are chosen so they can be published later, since `bolt` and `bolt-core` are taken on crates.io.
- Turn the engineering rules ([decision 022](decisions/022-engineering-rules.md)) into compiler and clippy settings, and create the small platform crate: the only place `unsafe` is allowed.
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
- Store, fetch and check objects across many packs, skip objects that are already stored. The lookup index lives in memory for now (rebuilt when the store opens); storing it in `cache/` waits until measurements show a need.
- An internal integrity check that walks every pack and object: the core of the later `bolt verify` command (milestone 6), used by tests from now on.
- *Done when:* property tests with random stores and fetches pass; the integrity check catches every corruption the pack tests introduce.

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

## Milestone 2: Coming from Git

Each step ends with tests passing on Windows, macOS and Linux; the plan's task numbers are in brackets. A ✅ marks a step that is done (2026-11-03).

**0. Ownership and warnings** ✅ (Tasks 1–2)
- The Administrators group counts as the user only in an elevated terminal (decision 030).
- Warnings travel in `--json` too, JSON output stays valid when a command fails midway, and `docs/json.md` documents every format.
- *Done when:* the ownership rule and every JSON document have tests.

**1. Shared diary records, and several packs per command** ✅ (Tasks 3–4)
- Tags and server lines live in records of their own that diary entries share (decision 034).
- A command writes a new pack every 2^20 objects, and still takes effect at one moment (decision 041).
- *Done when:* test vectors and fuzz targets cover the two records, and a crash between packs leaves the repository as it was.

**2. Names, and Git's ignore rules** ✅ (Tasks 5–7)
- Every name Git treats as `.git`; which names this system can hold; this drive's letter case and Unicode form.
- A matcher that follows Git's own rules, tested against `git check-ignore`; the forms storage-format.md §12 refused now work.
- `save` reads `.gitignore` files too, and keeps saved files that a rule matches (decision 038).
- *Done when:* Git's own wildmatch table passes, and the matcher agrees with Git on random rules on all three systems.

**3. Saving keeps what it can't hold** ✅ (Task 8)
- Names this computer can't hold, names that are one file on this drive, links Git wrote as files, and project links stay as they were (decisions 036, 039).
- *Done when:* each case has a test.

**4. The Git reader** ✅ (Tasks 9–11)
- The repository's format and config, refs and packed refs, loose objects, packs and deltas, commits, trees and tags, with every read bounded and every object verified (decision 031).
- *Done when:* every refusal, limit and parser has a test, and the reader never panics on truncated or crafted input.

**5. Import** ✅ (Tasks 12–14)
- `bolt import`: commits become saves exactly and repeatably (decision 032), branches lines and server lines (033), tags kept exactly (034), dangerous names skipped (035), submodules bare links (036), LFS repositories refused until milestone 5 (037), one-time for now (040).
- The first save after an import stops at line endings Git converted (decision 039).
- *Done when:* importing a repository twice, anywhere, gives the same saves, and an interrupted import can be run again.

**6. Proof** ✅ (Tasks 15–16)
- Differential tests compare every imported commit with Git, on scripted repositories on every push and on Git's own repository nightly.
- The hostile-repository corpus covers Git's published vulnerabilities that apply to an import (reviewed 2026-11-02), and the Git reader has fuzz targets.
- *Done when:* both pass, and the nightly run on Git's own repository matches.

**7. Follow-ups and docs** ✅ (Tasks 17–19)
- The milestone 1 follow-ups due now, and every document brought up to date.
- *Done when:* the follow-ups list and the docs match the code.

**8. Red team, review and merge** ✅ (Tasks 20–21)
- A red-team session from the sandbox (decision 024), then a whole-branch review, then a pull request whose checks all pass.
- *Done when:* every proven finding is fixed or ruled on, and the owner has imported one of their own repositories and tried a project inside a OneDrive folder (decision 026 #4).

**The owner's checks** (2026-11-04, with a release build of `main` at `aae2a08`)
- **Importing one of the owner's repositories** (a copy of Tailrd: 1,517 commits, 112 branches, 12 server branches, no tags). The import matched Git's counts, listed the 2 refs it leaves out, and took 3.5 s. Every file in `.git` was byte for byte the same afterwards. `history` and `show` matched `git log --first-parent` and `git show`, and a second import refused. Before it ran, Bolt asked for a name and email; its hint now says to make the settings file and its folder.
  - The first save stopped at line endings, as decision 039 says: Git for Windows had written 1,492 of the 1,925 files with CRLF. The hint's three Git commands fixed it.
  - It would then have taken what Git's personal ignore lists hide, since Bolt doesn't read them (decision 038). `.git/info/exclude` hides 44 Git worktrees under `.worktrees/`: 468,000 files and 14 GB on disk, of which the project's `.gitignore` files keep out only part (about 190,000 files and 11 GB stay, by a rough count). The import warned about that list. The global `~/.config/git/ignore` hides `.claude/settings.local.json`, which was saved without a word, since nothing warns about that list.
  - With the exclude lines copied into `.boltignore`, as the warning says, the first save took exactly the 6 files Git showed as untracked, the new `.boltignore` and that settings file; all 1,925 imported files matched. Git's personal lists, and saving the contents of nested repositories and worktrees (decision 043), are the owner's questions in [the follow-ups](plans/2026-11-01-milestone-1-follow-ups.md).
- **A project inside OneDrive** (Files On-Demand on). `start`, eleven saves in a row, `history` and `show` worked, and OneDrive never held a file Bolt needed. After "Free up space" on a file and then on `.bolt`, every command still worked. Windows hides OneDrive's placeholders from programs like Bolt, which run in its "disguise placeholders" mode, so Bolt sees ordinary files, and reading an online-only file downloads it first ([security.md](security.md) §7).

## Milestone 3: Working folder

Planned on 2026-11-05, after decisions [048](decisions/048-handle-relative-file-access.md) to [059](decisions/059-bolt-spellings-left-out-like-git-spellings.md) (the owner decided 048; Claude decided the rest at the owner's request). Plan: [plans/2026-11-05-milestone-3-working-folder.md](plans/2026-11-05-milestone-3-working-folder.md). Each step ends with tests passing on Windows, macOS and Linux; the plan's task numbers are in brackets. A ✅ marks a step that is done (2026-11-06); step 0 waits for the owner's review of the `unsafe` code. Decisions [060](decisions/060-user-git-settings-includes-and-paths.md) to [069](decisions/069-the-file-cache-matches-the-change-time.md) were made while building it.

**0. Folder handles** (Task 1)
- Every read and write inside the project goes through a folder Bolt holds open, refusing a link at each name: `rustix` on macOS and Linux, Windows' own calls in the platform crate (decision 048).
- *Done when:* the handle tests pass on all three systems, and the owner has reviewed the new `unsafe` code line by line.

**1. Names and limits** ✅ (Task 2)
- One module decides what may be written: `.git` and `.bolt` spellings everywhere, real short names, `\`, `:` and device names on Windows; names up to 765 bytes, paths up to 4,096 (decisions 051, 059).
- *Done when:* each rule has a test, `COM0` and `LPT0` save and write on Windows, and the name fuzz target runs.

**2. The walk through handles, and the owner check** ✅ (Tasks 3–4)
- `save` walks and reads the folder through handles; link targets read on Windows are stored with `/` (decision 045); the owner check resolves the starting folder and refuses a `.bolt` that vanishes mid-check.
- *Done when:* a folder swapped for a link mid-save is refused, not followed.

**3. Packs** ✅ (Tasks 5–6)
- Pack version 2's index is searched on disk, packs open only when needed, and small packs merge once there are many (decision 057).
- *Done when:* a planted index costs reads, not memory; a merge stopped at any step loses nothing.

**4. Caches and `status`** ✅ (Tasks 7–10)
- A keyed file-state cache and change-ID cache; `bolt status`; `save` skips files the cache vouches for (decision 053).
- *Done when:* a planted or edited cache is ignored, unchanged files aren't opened, and `status` on 100,000 unchanged files is measured.

**5. Writing files** ✅ (Tasks 11–13)
- The journal, the write-out (one file at a time, links last, never onto a name the system misreads), and finishing an interrupted switch (decisions 049 to 052).
- *Done when:* a switch stopped at any write is finished by the next command, with every file wholly old or new.

**6. `goto`, `discard` and lines** ✅ (Tasks 14–16)
- `goto` with safety copies and collisions, `discard`, saving away from a line, `bolt line` (decisions 050, 055).
- *Done when:* going to a line and back restores every file byte for byte.

**7. Saves** ✅ (Tasks 17–18)
- Partial saves, and the question before a large save (decisions 054, 056).
- *Done when:* each case of the decisions has a test.

**8. `changes`** ✅ (Task 19)
- Line-by-line differences with `similar` (decision 058).
- *Done when:* binary, huge and slow files show without lines, and every line is escaped.

**9. Proof** ✅ (Tasks 20–21)
- Hostile saves written out, including the write-out cases of Git's published vulnerabilities; random edits, saves and switches checked against a model.
- *Done when:* both pass on all three systems.

**10. Follow-ups and docs** ✅ (Task 22)
- *Done when:* the follow-ups list and the docs match the code.

**11. Red team, review and merge** (Tasks 23–24)
- A red-team session from the sandbox (decision 024), a whole-branch review, the owner's review of the new `unsafe` code, then a pull request whose checks all pass.
- *Done when:* every proven finding is fixed or ruled on, and the owner has reviewed the `unsafe` code.
