# Bolt Milestones

> **Status:** design-phase plan (2026-10-31), [decision 018](decisions/018-milestone-plan.md). Milestone 1 is broken into small steps below; each later milestone gets its own step list when it starts. **Milestone 1 is done** (2026-11-01; plan: [plans/2026-10-31-milestone-1-first-save.md](plans/2026-10-31-milestone-1-first-save.md)). The fixes from its two red-team sessions (2026-11-01 and 2026-11-02) are done too: plan [plans/2026-11-01-red-team-m1-fixes.md](plans/2026-11-01-red-team-m1-fixes.md), rulings in decisions [025](decisions/025-red-team-m1-rulings.md) to [029](decisions/029-red-team-m1-final-review-rulings.md). **Milestone 2 is built** (2026-11-03), and both its red-team sessions are done (2026-11-03, rulings in decisions [043](decisions/043-red-team-m2-rulings.md) and [044](decisions/044-red-team-m2-final-rulings.md)); its final review is done too, and it was merged into `main` on 2026-11-04. **Milestone 2 is done** (2026-11-04): the owner imported one of their own repositories and tried a project inside a OneDrive folder; the results are below its steps. It was planned on 2026-11-02 after decisions [030](decisions/030-administrators-only-when-elevated.md) to [041](decisions/041-several-packs-per-command.md), and [decision 042](decisions/042-trust-hint-skips-folders-anyone-can-write-to.md) settled one more question; its steps are below and its plan is [plans/2026-11-02-milestone-2-coming-from-git.md](plans/2026-11-02-milestone-2-coming-from-git.md). **Milestone 3 was planned on 2026-11-05** after decisions [048](decisions/048-handle-relative-file-access.md) to [059](decisions/059-bolt-spellings-left-out-like-git-spellings.md); its steps are below and its plan is [plans/2026-11-05-milestone-3-working-folder.md](plans/2026-11-05-milestone-3-working-folder.md). **Milestone 3 is done** (2026-11-06): both its red-team sessions and its whole-branch review are done, and it was merged into `main` after the owner approved its `unsafe` code. **Milestone 4 was planned on 2026-11-06** after decisions [070](decisions/070-undo-and-redo-step-through-the-diary.md) to [077](decisions/077-tidy-and-the-undo-window.md); its steps are below and its plan is [plans/2026-11-06-milestone-4-undo-everything.md](plans/2026-11-06-milestone-4-undo-everything.md). **Milestone 4 is done** (2026-11-08): built on 2026-11-07, both its red-team sessions and its whole-branch review are done, and it was merged into `main` (pull request #15); rulings: [plans/2026-11-06-milestone-4-rulings.md](plans/2026-11-06-milestone-4-rulings.md). The OneDrive "Free up space" check of decision 069 is still to be tried. **Milestone 5 was planned on 2026-11-08** after decisions [082](decisions/082-how-big-files-are-cut-into-chunks.md) to [088](decisions/088-benchmarks-and-the-speed-gates.md); its steps are below and its plan is [plans/2026-11-08-milestone-5-big-files.md](plans/2026-11-08-milestone-5-big-files.md). **Milestone 5 is done** (2026-11-09): built on 2026-11-08 and 2026-11-09, with decisions [089](decisions/089-git-lfs-objects-kept-elsewhere.md) to [093](decisions/093-a-family-rebuild-builds-every-member.md) made while building it; both its red-team sessions are done and their findings fixed (decisions [094](decisions/094-red-team-m5-rulings.md) to [098](decisions/098-a-blob-is-also-judged-in-its-crlf-form.md)), its whole-branch review is done, and it was merged into `main` (pull request #21); rulings: [plans/2026-11-08-milestone-5-rulings.md](plans/2026-11-08-milestone-5-rulings.md). **Milestone 6 was planned on 2026-11-09 and 2026-11-10**: two sessions build it at once, one for presets and polish (decided on 2026-11-09, decisions [113](decisions/113-presets-what-each-writes-and-how-start-picks-one.md) to [123](decisions/123-follow-ups-done-or-moved-past-v0.1.md), plan [plans/2026-11-10-milestone-6-presets-and-polish.md](plans/2026-11-10-milestone-6-presets-and-polish.md)) and one for speed and space; each lists its steps below. **Milestone 6 is done** (2026-11-10): both halves were built on 2026-11-10, with decisions [104](decisions/104-an-import-keeps-96-mib-of-rebuilt-objects.md) to [110](decisions/110-the-last-saves-listings-are-read-ahead-of-the-walk.md), [124](decisions/124-restore-is-explained-and-a-shortcut-takes-nothing-for-what-it-isnt.md) and [125](decisions/125-a-safe-folders-entry-is-judged-by-one-look.md) made while building them; each half's two red-team sessions are done and their findings fixed (decisions [111](decisions/111-rulings-after-the-first-red-team-session-on-the-speed-work.md), [112](decisions/112-rulings-after-the-second-red-team-session-on-the-speed-work.md), [126](decisions/126-verify-also-finds-a-save-that-cant-be-written-out.md) and [127](decisions/127-what-verify-holds-is-bounded-and-two-costs-are-written-down.md)), each half's whole-branch review is done, and both were merged into `main`, speed and space first (pull request #27) and presets and polish second (#28), once the two had been measured together ([benchmarks.md](benchmarks.md) §9); rulings: [plans/2026-11-10-milestone-6-rulings.md](plans/2026-11-10-milestone-6-rulings.md) and [plans/2026-11-10-milestone-6-speed-rulings.md](plans/2026-11-10-milestone-6-speed-rulings.md). **v0.1 itself has not shipped:** the third speed gate misses on Linux, Bolt has no license yet, and publishing the release is the owner's. Items milestone 1's reviews deferred, and the ones milestones 2 to 5 deferred, are in [plans/2026-11-01-milestone-1-follow-ups.md](plans/2026-11-01-milestone-1-follow-ups.md); each later plan pulls in the ones that apply.

## v0.1: solo, local, games first

| # | Milestone | Done when |
|---|---|---|
| 1 | **First save** ✅ | `bolt start`, `bolt save`, `bolt history` and `bolt show` work end to end on real projects, on the real formats, on all three systems |
| 2 | **Coming from Git** ✅ | `bolt import` brings in a Git repository's history (every branch and tag, in place next to `.git/`), and differential tests compare every commit with Git; the owner has imported one of their own repositories and tried a project inside a OneDrive folder ([decision 026](decisions/026-ownership-check-details.md) #4) |
| 3 | **Working folder** ✅ | `status` (with the file-state cache), `changes`, `goto`, `discard`, partial saves, safety copies, and basic lines: saving after `goto` creates one, `line` lists them, `goto <line>` switches; merge small packs and open packs lazily before real use (each save adds a pack, and every pack stays open) |
| 4 | **Undo everything** ✅ | The diary drives `undo`, `redo`, `undo --list` and `undo --to`; crash recovery; `tidy`; `shelf`/`unshelf` and `save --amend` ([decision 021](decisions/021-v0.1-command-scope.md)) |
| 5 | **Big files** ✅ | Chunking, streaming, parallel hashing, Git LFS import (contents must be on disk, [decision 020](decisions/020-lfs-import.md)), and benchmarks against Git LFS |
| 6 | **Presets and polish → v0.1** | The `bolt start` screen with the Code, Game (Unity, Unreal, Godot) and Custom presets; `bolt setup`; Git-word shortcuts; `--json`; `verify`; packaging |

**Every milestone also ends with a red-team session** ([decision 024](decisions/024-red-team-gate.md)): a fresh agent attacks the milestone's code from an isolated sandbox, and every proven finding is fixed (or ruled on in writing) before the milestone is merged. Milestone 1 had two sessions, on 2026-11-01 and (on the fixes) 2026-11-02; every finding is fixed. Milestone 2 had two as well, both on 2026-11-03, the second on the fixes; every proven finding is fixed or ruled on in writing (decisions 043 and 044). Milestone 3 had two, both on 2026-11-06 (decisions 067 to 069), milestone 4 two, both on 2026-11-08 (decisions 079 and 080), and milestone 5 two, both on 2026-11-09 (decisions 094 to 098). Milestone 6 had two for each of its halves, all on 2026-11-10 (decisions 111, 112, 126 and 127).

**v0.1 ships when** every test passes on the three systems, the speed gates below pass, no code that follows another project's code is left in Bolt ([the code audit](releases/v0.1.0-code-audit.md)), and the owner publishes it. The speed gates, which the nightly jobs check exactly as written here:

- **Memory** ([decision 023](decisions/023-v0.1-speed-bar.md)): saving a 10 GB file peaks at no more than 500,000,000 bytes, on GitHub's Linux runner.
- **Status** (decisions [023](decisions/023-v0.1-speed-bar.md) and [129](decisions/129-gotos-limit-until-v0.2-and-the-status-gate-enforced.md)): `status` on an unchanged 100,000-file project takes under 2 seconds, both times it is run, on GitHub's Linux runner.
- **Against Git** (decisions [099](decisions/099-a-third-speed-gate-within-twice-gits-time.md), [108](decisions/108-the-third-gate-compares-medians.md), [128](decisions/128-v0.1-ships-without-goto-within-twice-gits-time.md) and [132](decisions/132-gotos-limit-is-three-and-a-half-times-gits-time.md)): on Git's own source, `status` with nothing changed and a small change saved each take at most twice Git's time, and going back to the first version at most three and a half times, comparing the medians of ten runs, on GitHub's Linux and Windows runners. Going back within twice Git's time is a goal for v0.2: the job prints its ratio and says when it is over.

Benchmark results against Git and Git LFS are published either way ([benchmarks.md](benchmarks.md)). The first two gates passed when measured on 2026-11-09 and every time since. The third was added on 2026-11-10 and failed five of its six checks before milestone 6's speed work; after it, going back on Linux took 1.79 to 2.65 times Git's time over ten runs, so on 2026-11-11 the owner ruled that v0.1 ships without it, holding `status` and the small save to twice Git's time (decision 128).

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

Planned on 2026-11-05, after decisions [048](decisions/048-handle-relative-file-access.md) to [059](decisions/059-bolt-spellings-left-out-like-git-spellings.md) (the owner decided 048; Claude decided the rest at the owner's request). Plan: [plans/2026-11-05-milestone-3-working-folder.md](plans/2026-11-05-milestone-3-working-folder.md). Each step ends with tests passing on Windows, macOS and Linux; the plan's task numbers are in brackets. A ✅ marks a step that is done (2026-11-06). Decisions [060](decisions/060-user-git-settings-includes-and-paths.md) to [069](decisions/069-the-file-cache-matches-the-change-time.md) were made while building it.

**0. Folder handles** ✅ (Task 1)
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

**11. Red team, review and merge** ✅ (Tasks 23–24)
- A red-team session from the sandbox (decision 024), a whole-branch review, the owner's review of the new `unsafe` code, then a pull request whose checks all pass.
- *Done when:* every proven finding is fixed or ruled on, and the owner has reviewed the `unsafe` code.

## Milestone 4: Undo everything

Planned on 2026-11-06, after decisions [070](decisions/070-undo-and-redo-step-through-the-diary.md) to [077](decisions/077-tidy-and-the-undo-window.md) (Claude decided them at the owner's request, while the owner was away; each can be reversed). Plan: [plans/2026-11-06-milestone-4-undo-everything.md](plans/2026-11-06-milestone-4-undo-everything.md). Each step ends with tests passing on Windows, macOS and Linux; the plan's task numbers are in brackets. A ✅ marks a step that is done (2026-11-07; step 7 on 2026-11-08). [Decision 078](decisions/078-a-pack-bolt-cant-open-gets-its-own-warning.md) was made while building it.

**0. Kept saves and the folder change** ✅ (Tasks 1–2)
- What `discard` throws away is kept as a save of its own, and each diary entry records what its command did to the folder as a switch between two saves (decision 071).
- *Done when:* a discard's kept save holds exactly what was thrown away, deletions and folders included, and the diary's test vectors match storage-format.md §9.

**1. Undo and redo** ✅ (Tasks 3–5)
- The done and undone steps worked out from the diary; `bolt undo [N]`, `bolt redo [N]`, `bolt undo --list [--all]` and `bolt undo --to <entry>` (decisions 070, 072).
- *Done when:* the step lists match a replay of the diary, undo and redo of every command put back exactly what it did, and unsaved work in the way stops them before anything changes.

**2. Shelves and amend** ✅ (Tasks 6–7)
- `bolt shelf`, `bolt unshelf`, `bolt shelf --list` and `bolt save --amend` (decisions 073, 074).
- *Done when:* each case of the decisions has a test, and undo takes each of them back.

**3. Crash recovery** ✅ (Tasks 8–9)
- A lost `head` stops every command and is picked up from the diary; a failure after `head` moves is a warning; renames, removals and new folders are written through on Windows (decisions 075, 076).
- *Done when:* each new command stopped at every write is finished or left as it was by the next one.

**4. `tidy`** ✅ (Task 10)
- What lines, shelves, the diary and the undo window reach is kept, and nothing else; the `undo-window-days` setting (decision 077).
- *Done when:* each kind of reference is kept, and a `tidy` stopped at any step loses nothing.

**5. Follow-ups** ✅ (Tasks 11–13)
- The milestone 4 rows of the follow-ups: the store, the walk, output and messages.
- *Done when:* each has a test or a ruling.

**6. Proof** ✅ (Tasks 14–15)
- Hostile diaries written out, and undo, redo, shelves and amend in the model test.
- *Done when:* both pass on all three systems.

**7. Docs, red team, review and merge** ✅ (Tasks 16–18)
- The docs brought up to date, two red-team sessions from the sandbox (decision 024), a whole-branch review, then a pull request whose checks all pass.
- *Done when:* every proven finding is fixed or ruled on, and every document matches the code.

## Milestone 5: Big files

Planned on 2026-11-08, after decisions [082](decisions/082-how-big-files-are-cut-into-chunks.md) to [088](decisions/088-benchmarks-and-the-speed-gates.md) (Claude decided them at the owner's request, while the owner was away; each can be reversed). Plan: [plans/2026-11-08-milestone-5-big-files.md](plans/2026-11-08-milestone-5-big-files.md). Each step ends with tests passing on Windows, macOS and Linux; the plan's task numbers are in brackets. A ✅ marks a step that is done (steps 0 to 3 on 2026-11-08, steps 4 to 8 on 2026-11-09). Decisions [089](decisions/089-git-lfs-objects-kept-elsewhere.md) to [093](decisions/093-a-family-rebuild-builds-every-member.md) were made while building it.

**0. Chunk lists and the cutter** ✅ (Task 1)
- Bolt's own FastCDC cuts files of 1 MiB to 128 GiB into chunks of 16 KiB to 256 KiB; the chunk-list record is read and written (decision 082).
- *Done when:* the vectors of storage-format.md §13 pass, and the record and the cutter have fuzz targets.

**1. Chunked files in the store** ✅ (Task 2)
- A chunk list is stored under its file's ID; reads rebuild the file, checking each chunk and the whole; merging and the store's check follow lists (decision 083).
- *Done when:* a chunked file reads back byte for byte from one pack or several, and a damaged or planted list is refused, never trusted.

**2. One read, several threads** ✅ (Tasks 3–4)
- Every writer reads a file once, a big one cut as it is read; reading, hashing, checking and compressing run on worker threads, and one thread writes the packs (decision 084).
- *Done when:* chunking changes no ID, and a save writes the same packs with one thread or eight.

**3. `tidy` follows chunk lists** ✅ (Task 5)
- *Done when:* every chunk a kept file needs survives `tidy`, and a pack it can't read stops it.

**4. Git LFS import** ✅ (Tasks 6–7)
- Git LFS files come in from `.git/lfs/objects`, checked against their pointers; missing ones stop the import before anything is written; the first save stops at pointer files (decision 085).
- *Done when:* a Git LFS repository imports as Git LFS checks it out, on all three systems.

**5. Delta families rebuilt once** ✅ (Task 8)
- *Done when:* a long history of a big file imports within the allowance, each object built once (decision 086).

**6. Proof** ✅ (Task 9)
- Hostile chunk lists, Git LFS's write-out lessons (CVE-2025-26625), big files in the model test, crash tests.
- *Done when:* each passes on all three systems.

**7. Gates and benchmarks** ✅ (Tasks 10–11)
- *Done when:* both speed gates are measured, and `docs/benchmarks.md` compares Bolt with Git and Git LFS, whatever it shows (decisions 023, 087, 088).

**8. Follow-ups and docs** ✅ (Tasks 12–13)
- *Done when:* the follow-ups list and the docs match the code.

**9. Red team, review and merge** ✅ (Tasks 14–15)
- Two red-team sessions from the sandbox (decision 024), a whole-branch review, then a pull request whose checks all pass.
- *Done when:* every proven finding is fixed or ruled on, and every document matches the code.

## Milestone 6: Presets and polish → v0.1

Two sessions built it at once, each with its own steps, decisions and red-team sessions: presets and polish (decisions from 113), and speed and space (decisions [099](decisions/099-a-third-speed-gate-within-twice-gits-time.md) to 112). Each step ends with tests passing on Windows, macOS and Linux; a plan's task numbers are in brackets. A ✅ marks a step that is done.

### Presets and polish

Decided on 2026-11-09, in decisions [113](decisions/113-presets-what-each-writes-and-how-start-picks-one.md) to [123](decisions/123-follow-ups-done-or-moved-past-v0.1.md) (Claude decided them at the owner's request, while the owner was away; each can be reversed), and its plan written on 2026-11-10: [plans/2026-11-10-milestone-6-presets-and-polish.md](plans/2026-11-10-milestone-6-presets-and-polish.md).

**0. What a terminal can show** ✅ (Tasks 1–2)
- Bolt finds out, for stdout and stderr each, whether it has colour, symbols and a width; errors, hints and warnings are laid out by it; progress is a bar or a turning mark (decision 120). Colour works on Windows.
- *Done when:* with a terminal faked, each role has its colour and each symbol its plain form; without one, Bolt's own words are as plain as before.

**1. Change IDs in Jujutsu's letters** ✅ (Task 3)
- *Done when:* an imported save shows the letters its commit's `change-id` header has (decision 122).

**2. `bolt.toml`** ✅ (Task 4)
- The chunk threshold and the large-save question's numbers are read from the folder's `bolt.toml`; what Bolt can't use is passed over with a warning (decision 114).
- *Done when:* the vectors of storage-format.md §12 pass, a hostile file never stops a save, and a project without the file saves byte for byte as before.

**3. Presets** ✅ (Tasks 5–7)
- The presets' rules as data, a look at the folder to suggest one, the `setup` step that writes the two rule files, `bolt start`'s question and `--preset`, and `bolt setup` (decisions 113 and 115).
- *Done when:* each preset writes its vector byte for byte, `bolt undo` after a start gives the folder back, and a preset switch can be undone and redone.

**4. Git's words** ✅ (Task 8)
- *Done when:* each shortcut runs its Bolt command, each explained word changes nothing, and the hint shows once (decision 116).

**5. `bolt verify`** ✅ (Task 9)
- Every pack and object, then everything the project needs; `tidy` records the window it used (decisions 118 and 123).
- *Done when:* every kind of damage the pack tests make is found, a deleted pack shows as what the project lacks, and a command running beside it never makes it report damage.

**6. Every command in the new design** ✅ (Tasks 10–12)
- *Done when:* each command's output follows terminal.md at 60, 80 and 120 columns, and every place the canvas and Bolt's facts disagree is written down.

**7. Follow-ups** ✅ (Task 13)
- The rows decision 123 marks "done in milestone 6".
- *Done when:* each has a test, and the tests of RT6-3 and RT7-2 run again.

**8. The JSON formats, before they freeze** ✅ (Task 14)
- *Done when:* every document has been read against json.md once, and a test fails on a field json.md doesn't list (decision 117).

**9. Proof** ✅ (Task 15)
- Hostile rule files, and project text with colour on.
- *Done when:* both pass on all three systems.

**10. The guide and packaging** ✅ (Tasks 16–17)
- The user guide, the README, `SECURITY.md`, version 0.1.0 and the release workflow (decision 119).
- *Done when:* every command in the guide runs in a test, and the workflow builds four archives that each pass their smoke test.

**11. Docs, red team, review and merge** ✅ (Tasks 18–20)
- The docs brought up to date, two red-team sessions from the sandbox (decision 024), a whole-branch review, then a pull request whose checks all pass.
- *Done when:* every proven finding is fixed or ruled on, and every document matches the code.

### Speed and space

Planned on 2026-11-10, after decisions [099](decisions/099-a-third-speed-gate-within-twice-gits-time.md) to [103](decisions/103-where-git-keeps-less-zstd-level-3-and-the-chunk-sizes-stay.md) (Claude decided them at the owner's request, while the owner was away; each can be reversed). It starts from a profile of where the time goes ([benchmarks.md](benchmarks.md) §7). The plan's task numbers are added in brackets when the plan is written.

**0. Measure** ✅ (2026-11-10)
- `status`, a small save and `goto` timed from the inside on Git's source and on Godot's demos, on GitHub's Linux and Windows runners and on the laptop; the big-file steps Git or Git LFS wins; where Godot's space goes.
- *Done when:* [benchmarks.md](benchmarks.md) §7 says where the time and the space go.

**1. The gate against Git** ✅ (2026-11-10)
- `bench/run.sh gate` and the `speed-gate` job ([decision 099](decisions/099-a-third-speed-gate-within-twice-gits-time.md)).
- *Done when:* the job runs on both systems and fails, as it must before the work below. It compares medians since [decision 108](decisions/108-the-third-gate-compares-medians.md).

**2. Caches and flushes** ✅ (2026-11-10)
- The file cache is loaded without a copy of each path, asked once for each file, written only when it changed and never flushed; `state/lock-info` and the drive test lose their flush ([decision 100](decisions/100-caches-are-written-without-a-flush-and-only-when-they-changed.md)).
- *Done when:* an unchanged `status` leaves `cache/files` untouched, and a torn cache is ignored and rebuilt.

**3. The walk** ✅ (2026-11-10)
- Ignore lines that are plain names are found by lookup, not by trying each; a folder is listed before it is asked for `.git`, `.bolt` and ignore files ([decision 101](decisions/101-the-walk-lists-a-folder-before-it-looks-inside.md)); each name is handled without building its path three times.
- *Done when:* the matcher agrees with Git and with its old self on random rules, and every walk test passes unchanged.

**4. The store and the listings** ✅ (2026-11-10)
- A record is found and read in fewer reads; a folder whose entries all match the last save's takes that save's listing instead of building its own.
- *Done when:* every object is still checked against its fingerprint on every read, and a save writes the same packs as before.

**5. The commands' own work** ✅ (2026-11-10)
- A command tests the drive once, loads each cache once, reads the diary entry once and lists the packs once; a switch plans from one scan.
- *Done when:* the first measurement of the gate after these steps is published, and what is still over is named.

**6. Big files** ✅ (2026-11-10)
- A big file is flushed while it is written ([decision 102](decisions/102-big-files-are-flushed-while-they-are-written.md)); a switch reads each chunk whole and keeps the chunks it has checked while it writes a file.
- *Done when:* the first save of big files that don't compress and going back to an old version of a big file are measured again, beside Git and Git LFS.

**7. Follow-ups** ✅ (2026-11-10)
- The rows of the [follow-ups](plans/2026-11-01-milestone-1-follow-ups.md) about speed, memory or space: each is done, or moved past v0.1 in a decision record that says why.
- Also built here: `cache/change-ids` keeps each line's saves in order, so commands on a long history don't read the line ([decision 105](decisions/105-the-change-id-cache-keeps-each-lines-saves-in-order.md)); the last save's listings are read ahead of the walk ([decision 110](decisions/110-the-last-saves-listings-are-read-ahead-of-the-walk.md)); folders deeper than the walk holds open no longer cost the square of their depth.
- *Done when:* each has a test, a measurement or a ruling ([decision 109](decisions/109-the-speed-sessions-follow-ups-and-format-1s-three-open-rows.md)).

**8. Proof** ✅ (2026-11-10)
- Tests for what speed puts at risk: a torn, stale or planted cache; files changed during a walk or within the cache's 2 seconds; folders swapped mid-walk; a flush that fails halfway through a big file.
- *Done when:* each passes on all three systems, and the model test passes with the cache on.

**9. Docs, red team, review, benchmarks and merge** ✅ (2026-11-10)
- The docs brought up to date, two red-team sessions from the sandbox (decision 024), a whole-branch review, the whole benchmark and all three gates run again, then a pull request whose checks all pass.
- *Done when:* every proven finding is fixed or ruled on, every document matches the code, and [benchmarks.md](benchmarks.md) gives the numbers before and after, losses included.
- **The third gate is not met yet.** `status` and the small save pass; going back on Linux is at twice Git's time or over, where it was 4.5 times, and missed in five runs of seven ([benchmarks.md](benchmarks.md) §8). What would settle it is the owner's call: the first row of this session's section of the [follow-ups](plans/2026-11-01-milestone-1-follow-ups.md).
