# Milestone 2 attack surface (red-team session, decision 024)

Milestone 2 adds `bolt import`. It reads a Git repository already on disk, next to `.git/`, with Bolt's own read-only reader, and writes its whole history into a new `.bolt/`. It also changes `bolt save` so the first save after an import behaves like Git with tracked files.

Threat list: `docs/security.md` §4 (T1–T24). Decisions 031–041 in `docs/decisions/` give the rules this code follows, and `docs/design.md` §5.16 summarises them. Your goals:
- lose or corrupt data;
- read or write outside the project;
- make Bolt store what it shouldn't, or miss what it should;
- make an import's history differ from Git's without saying so;
- crash, hang, or exhaust memory or disk;
- trick the person at the terminal.

## Where the code is

- **Git reader:** `crates/bolt-vcs-core/src/git/`.
  - `mod.rs`: opening, refusals, `GitError`.
  - `config.rs`, `refs.rs`.
  - `id.rs`: IDs and SHA-1 with collision detection.
  - `odb.rs`: object lookup, loose objects, delta chains, base cache, `blob_reader`.
  - `pack.rs`: `.idx` and `.pack` parsing, deltas.
  - `object.rs`: commits, trees, tags, people, LFS pointer tests, `rebuild_commit`.
  - `craft.rs`: a byte-exact test writer you can use.
- **Import:** `crates/bolt-vcs-core/src/import.rs` holds the flow, refusals and the read-only first pass. `import/convert.rs` turns Git objects into Bolt objects, with memo tables and the tag budget. `import/names.rs` holds the renaming rule and limits.
- **Save after an import:** `snapshot.rs`, `snapshot/walk.rs` and `snapshot/endings.rs`.
  - `walk.rs` covers ignore rules, saved files staying saved, kept entries (names this computer can't hold, case or Unicode twins, links Git wrote as files, project links) and folders gone from disk.
  - `endings.rs` is the line-ending stop.
  - Also `ignore.rs` (a port of Git's wildmatch and dir.c), `names.rs` (`.git`/`.bolt` spellings) and `drive.rs` (the case and Unicode probe).
- **Store and diary:**
  - `pack.rs` (core): `PackSet` writes several packs per command, and a writer rolls over at 2^20 objects.
  - `diary.rs`: tag-list and server-lines records, kinds 22 and 23.
  - `link.rs`: project-link records.
  - `repo.rs`: `begin`/`commit_first`; `format` is written last.
- **CLI:** `crates/bolt-vcs/src/main.rs` and `import_output.rs`.

## Limits as built (check each one holds before allocating)

| What | Limit |
|---|---|
| `.git/config`, `config.worktree` | 1 MiB |
| `HEAD`, each loose ref | 4 KiB |
| `packed-refs` | 512 MiB, read whole |
| Ref entries | 7,600,000 entries, and 512 MiB of names and paths |
| Open packs kept between reads | 16 |
| Deltified object and its base | 512 MiB each |
| Delta chain | 4,096 |
| Base cache | 256 MiB |
| Blobs read whole | up to 64 MiB; bigger ones are streamed twice |
| One tree | 1,000,000 entries |
| Snapshot | 4,096 levels and 10,000,000 entries, counting every path |
| Names | 255 bytes |
| Commit | at most 64 KiB of header + 1 + 1 MiB of message, checked from the header before reading; a non-UTF-8 message is limited to 64 KiB |
| Parents | 1,024 |
| Tags | 100,000 refs, skipped ones included |
| Tag chains | 16 objects of at most 1 MiB each |
| Tag list | 512 MiB, budgeted in the read-only pass |
| Server-lines record | 512 MiB |
| Lines | 100,000 |
| Servers | 64 |
| Lines per server | 100,000 |
| LFS pointer | non-empty, at most 1,024 bytes; Git LFS's own parser, plus a looks-like rule |

## Areas the builders flagged; probe them first

1. **Delta-base amplification.** N small objects deltified against one huge base (≤ 512 MiB, bigger than the 256 MiB cache) each rebuild that base, so N × 512 MiB is inflated. This applies to commits read in pass 1 and pass 2, and to distinct blobs. Git behaves the same; Git's `pack-objects` never makes a delta whose base is more than 32× its result. Measure it.
2. **Tag chains.** A kept tag chain (ending at a commit) is re-read once per ref. It is bounded only by the 512 MiB tag-list budget. Measure reads and time at the limits.
3. **Memo tables.** These take about 100 bytes per distinct Git object. git/git (86k commits) peaks at about 583 MiB. Find a small repository that makes them large.
4. **A tracked `.env`.** Saved files stay saved and there's no add step. So a hostile repository that tracks `.env` (or any secret's name) makes the user's later local `.env` get saved despite `.gitignore`. Git needs `git add` or `commit -a` for that. Prove the impact, and judge whether it is a finding or design.
5. **A `.git` file in a subfolder** that isn't a project link (a submodule added after the import, or a nested worktree) is saved as an ordinary file. Git refuses to track any `.git` path. Think about what that means once Bolt writes files (milestone 3) and for a later export to Git.
6. **Windows link targets.** Git for Windows writes a link's `/` as `\`, so the first save after an import with real links stores `dir\target.txt`. This is known and awaiting the owner's decision; don't re-prove it, but look for worse consequences.
7. **Name checks.**
   - The `.bolt` check looks at the first backslash part only, while the `.git` check looks at every part.
   - `is_hfs_dotgit` checks the whole name, not each backslash part.
   - Names reach Windows only from milestone 3, but check what the import and save do with them now.
8. **Races during the walk.** A folder swapped for a link mid-walk is security.md T20's known limit. It stops with "a fault in Bolt"; check nothing worse can happen.
9. **Interrupted imports.**
   - A re-run reuses stored objects after checking them (decision 041).
   - Another tool's `.bolt` is refused. So is a `.bolt` that Git tracks at the top.
   - Try planted packs or records in an unfinished `.bolt`.
10. **Output.**
    - Ref names, paths, author names and messages go through `text::escape`.
    - The renaming summary, skipped refs, tags and entries, warnings, progress and `--json`.
    - Try bidi controls, escapes, very long names, and names chosen so that renaming makes them collide or mislead.
11. **Exactness.**
    - Make an import whose saves don't rebuild to Git's bytes while Bolt says nothing.
    - Make two different Git histories import to the same Bolt saves.
    - Make the "inexact commits" count wrong.

## Rules

- Work **only** in your copy and the container: `bash redteam/sandbox.sh run <cmd>` runs in the copy at `/work`, with no network. Never touch the owner's repository folder or any other folder.
- **Proof is required.** Each finding comes with a test that fails because of the bug, or a script whose output shows it.
  - Put proof tests in `crates/bolt-vcs/tests/redteam3.rs` in your copy, and scripts in `redteam/attacks3/`.
  - Each test asserts the **safe** behaviour, so it fails on today's code.
  - Suspicions without proof go in a separate list and never count as findings.
- **Windows-only or macOS-only attacks** (junctions, device names, short names, case tricks, real links) are written as tests. The controller runs them on GitHub's machines from a throwaway `redteam/m2` branch. Mark them clearly.
- **Report:** `docs/redteam/2026-11-03-milestone-2.md` in your copy, in the format of `docs/redteam/2026-11-02-milestone-1-session-2.md`.
  - For each finding: an ID (RT3-n), its severity, the threat it maps to (or a new one), what happens, who is hit, the proof (with output) and a suggested fix.
  - Then the suspicions, and what held up.
