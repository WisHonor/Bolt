# Bolt Security Plan — Draft

> **Status:** drafted in the design phase (2026-10-31), updated after milestone 1's two red-team sessions (2026-11-02) and for milestone 2's Git import (2026-11-03). Every defense gets a test in [testing.md](testing.md) when it is built. The Windows naming rules were checked against Microsoft's documentation on 2026-11-01 (see §6 and decision 004).

**In plain words.** Bolt assumes that any project you copy, any repository you import and any server you talk to might be trying to hurt you: overwrite files outside the project, run code on your computer, fill your disk, or trick you with tampered history. Bolt's defenses are simple rules, each applied in one place, and every rule has a test.

## 1. What we protect

1. **Your files and history:** never lost, never silently corrupted.
2. **Your computer:** nothing in a project can make Bolt run a program or write outside the project.
3. **Your secrets:** login tokens and passwords never end up in files, logs or history by accident.
4. **Private projects** (from v0.3): nobody reads a project they have no access to.
5. **Availability:** malicious input can make a command fail, but not hang it, crash it, or exhaust memory or disk without limit.

## 2. Who might attack

| Attacker | Example |
|---|---|
| A malicious project | A repository someone shares, or a Git repository you import, crafted to exploit Bolt |
| A malicious or compromised server | Sends tampered saves, oversized data or fake lines |
| A network attacker | Tries to read or change traffic |
| A teammate, within their permissions (v0.3) | Abuses locks, or sends huge files |
| Another customer of a hosting service (later) | Tries to read other projects' data through shared storage |

Out of scope: malware already running as you on your own computer.

## 3. Trust boundaries

- **Untrusted:** everything inside a project (file names, `bolt.toml`, `.boltignore`); everything in `.bolt/store` and `.bolt/state`, since files on disk can be tampered with; everything received from a server; every imported Git repository.
- **Trusted:** your personal settings and the Bolt program itself.

## 4. Threats and defenses

| # | Threat | Defense | Decided in |
|---|---|---|---|
| T1 | Path tricks: `..`, absolute paths, drive letters (`C:`), network paths (`\\server`), backslashes | Name rules on every listing entry (no empty, `.` or `..` parts, no `/` or NUL); names containing `\` or `:` may be stored but are never written out on Windows; write-out builds paths only from validated names; a Git import holds every tree entry to the same rules, takes only ref names Git's `check-ref-format` allows, and finds loose refs by walking `refs/`, never by a name from the repository | 004, 019, 031 |
| T2 | Writing into `.bolt/`, or into Git's `.git/` beside it after an import, through spellings Windows or macOS treat as `.bolt` or `.git` (`.BOLT`, `.bolt.`, `BOLT~1`, `.bolt::$INDEX_ALLOCATION`, `.GIT`, `git~1`, `.git` with an invisible character) | `.bolt` spellings are invalid everywhere; a Git import skips every name any system would treat as `.git` (Git's own list, on every system, as Git for Windows applies it: its checks end a name at a backslash too, so `.git\…` is included), and `.`, `..`, empty names and names holding `/`, listing what it skipped in its summary; a `.bolt` spelling Git tracks at the top of the commit it is on stops the import, and a folder spelled so below it is warned about | 004, 019, 035 |
| T3 | Writing through a symlink to reach outside the project | Write-out never follows symlinks and checks every path step; symlinks pointing outside become placeholder files; a Git import keeps links as links, their targets as bytes it never opens or looks at, and reads nothing in `.git` through a link: `.git` itself, `objects/`, a ref or a pack that is one is refused | 004, 011, 031 |
| T4 | Two names that become one file on Windows or macOS (`README` / `readme`, Unicode normalization) | Refused on save when new; a name the last save already holds that is one file with another on this drive (an import can bring such names) is kept exactly as it was; reported, never overwritten, on write-out | 004, 039 |
| T5 | Windows device names (`CON`, `NUL`, `AUX`, `COM1`, `CONIN$`…) and `name:stream` | Never written; reported; a name the last save already holds that this computer can't hold (an import can bring such names) is kept exactly as it was; a Git import never opens a ref file named like a device on Windows | 004, 031, 039 |
| T6 | Running code from a project (hooks, filters, merge drivers) | No hooks run on copy or import; `bolt.toml` is declarative; programs are named only in personal settings; Git import reads repositories with Bolt's own reader and never runs Git, Git LFS, hooks or filters, nor fetches anything; a Git LFS pointer, or a small file that merely looks like one, stops the import, and hooks planted under a `.git` spelling are left out with that folder | 008, 017, 031, 037 |
| T7 | Decompression bombs and size bombs, including tree bombs (shared listings describing billions of files) | Every stored object records its real size and decompression stops there; hard limits on names, listings, messages, parents and chunk counts; at most 10,000,000 entries per save; a Git import inflates objects through fixed buffers; what it reads whole, and what a delta builds, it holds to 512 MiB, refusing a bigger claim before allocating, while bigger files stream through, checked against their ID as they end; it refuses more than 1,024 parents, 4,096 deltas per object or 16 tag objects per tag, a tag object over 1 MiB, and packs listing more than 16,777,216 objects in all, before reading an index's tables | 009, 025, 031, 034, storage spec |
| T8 | Malformed or ambiguous records (two encodings of one record) | Strict readers accept exactly one encoding; fuzz-tested for folder listings, saves, diary entries, tag lists and server lines, but not yet for project links (a follow-up) | 005, 034 |
| T9 | Tampered data on disk or from a server | Every object is checked against its fingerprint on every read and on receipt; packs carry index checksums; a stored object is read and checked before Bolt relies on it instead of storing the user's copy; a copy that fails, for any reason, is passed over for a good copy in another pack, and a pack whose copy fails a check is reported as damaged | 005, 009, 015, 028, 029 |
| T10 | Fingerprint collisions | BLAKE3-256 (no known attacks); the algorithm is a per-repository setting, so it can be replaced; a Git import checks every Git object's ID as it reads it, with SHA-1 collision detection as Git has it (SHA-256 repositories are read too), so an object built to collide is refused | 005, 031 |
| T11 | Resource exhaustion (deep nesting, huge entry counts, pathological diffs) | Limits in the storage spec (at most 4,096 folder levels per save; the ignore files (`.gitignore` and `.boltignore`) in force at one folder total at most 1 MiB, those one save takes rules from at most 16 MiB, and the scan stops at the first ignore file that stops the save); an ignore line is matched without recursion, in time at most its length times the path's; no recursion per folder level in Bolt's own walks (the scan thread has stack for the one library clean-up that does); time limits on diffs; history is read one save at a time and never held, and a save is looked up by fingerprint in one pass over the saves, however many lines reach it; stored copies are read smallest recorded size first, and a save never reads a copy whose recorded size isn't the object's, so a planted copy claiming a huge size is read only when no smaller copy works; a Git import walks trees, commits and delta chains with stacks of its own, and stops a commit past a save's limits as soon as it gets there | storage spec, 025, 028, 029, 031, 038 |
| T12 | Accidentally saving secrets (`.env`, private keys, tokens) | Presets ignore `.env`; `bolt save` warns about files that look like secrets (key files, token formats such as `github_pat_…`, `ghp_…`, `AKIA…`); `.boltignore` lines are read and matched exactly as Git reads and matches them, with Git's own rules ported and no line refused (letter case ignored where the drive ignores it, so `*.env` also keeps out `PROD.ENV` there; braces literal; the details in [storage-format.md](storage-format.md) §12), so the files Bolt keeps out are the ones Git would, which tests check against Git itself; after a Git import the project's `.gitignore` files apply too, read with the same matcher, so a first save keeps out what Git kept out | 016, 028, 029, 038 |
| T13 | Token theft from disk or logs | Tokens live only in the OS credential store, never in config files or history; redacted in `--debug` output and in the commands recorded in diary entries | 015 |
| T14 | Reading another customer's data by guessing fingerprints (v0.3+) | Dedup only within a project or organization; referencing an existing chunk requires proof of possession | 015 |
| T15 | Network tampering | HTTPS only, and fingerprints are verified anyway | 015 |
| T16 | Lock abuse (v0.3) | Only maintainers break locks; the holder is notified; every lock action is logged on the server | 014 |
| T17 | Unsafe asset previews on the website (later) | Previews made in isolated workers; scripts stripped from SVG; strict content security policy | later |
| T18 | Malicious or vulnerable dependencies | Few dependencies; `Cargo.lock` committed; `cargo-deny` in CI, on the workspace and on the fuzz crate, which has its own lock file (licenses, bans, sources: crates.io only, and the RustSec advisory database that `cargo-audit` uses); Dependabot alerts and weekly update pull requests (§6); every new dependency reviewed (milestone 2 added `miniz_oxide`, `sha1-checked`, `sha2` and `encoding_rs`) | testing, 031 |
| T19 | Links and special files inside `.bolt/` (a symlinked `tmp/` makes cleanup delete files elsewhere; a symlinked `store/packs` or `state/lock` writes elsewhere; a pipe or `/dev/zero` hangs or exhausts memory) | `.bolt/` and every fixed folder in it must be real folders, never links, junctions or other Windows reparse points; its files must be regular files, opened without following links and read with a size cap; all of this in one module; a pack's index is read a piece at a time, with a capped entry count, never whole; cleanup removes only Bolt's own temporary names; a Git import reads `.git` by the same rules, through the same module: Git's pack indexes may list at most 16,777,216 objects across all of a repository's packs, and their tables grow only as names pass the order check, so a sparse index costs nothing | 025 (red team), 031 |
| T20 | A file swapped for a link while `save` reads it, so the link's target is saved | A save opens each file without following a link at the file's own name, and the file it opened (checked on the open file, not on its path) must be a regular file; otherwise the save stops, saves nothing and reports the file as changed during the save. Bolt reads each ignore file (`.gitignore` and `.boltignore`) once itself, through the same checks; one that is a link or special file stops the save when it is a `.boltignore`, and is skipped with a warning when it is a `.gitignore`, as Git skips a linked one. Known limits: a folder swapped for a link between the scan and the read waits for milestone 3's handle-relative walking; hard links planted in a shared folder before the save are saved like ordinary files, as in Git; on Windows, a file that is a reparse point but not a link (a cloud placeholder, say) stops the save too, with a message that Bolt can't save it yet (§7) | 025 (red team), 038 |
| T21 | Terminal spoofing: bidi overrides, line separators, control characters and invisible characters in names, messages, paths and errors | Every untrusted piece of text is escaped before printing: control characters, bidi controls, line and paragraph separators, every other invisible format character (Unicode category Cf, except the zero-width joiner and non-joiner, which emoji and scripts need) and the blank Hangul fillers; the import's summary, warnings and refusals escape ref names, paths and messages from the Git repository the same way | 025, 028 (red team), 031 |
| T22 | A project claiming to be the user (its settings set the name and email on the user's saves) | Identity comes only from the environment and the user's own settings; a project's identity is ignored, with a note | 025 (red team) |
| T23 | A repository planted in a parent folder (`C:\.bolt`, `/tmp/.bolt`) capturing commands run below it | Refuse a repository unless both the project folder and its `.bolt` folder belong to the user, as Git requires; listing that exact folder in the user's own `safe-folders` trusts both. A Git import applies the same rule to `.git`: one that another account owns is refused unless the project folder is in `safe-folders`. A hint that would offer that line never does for a folder other accounts can write to (on Unix a folder with the other-write bit, `/tmp` included; on Windows the root of a drive or share): it says why instead and asks the user to move the project into a folder of their own. A `safe-folders` line written below a `[section]` of the settings belongs to that section, so Bolt never sees it; where the list is needed, Bolt stops with an error that says the line must go above every `[section]`, and a project that is the user's own never reads the list. On Windows, a folder owned by the Administrators group counts as the user's only in an elevated terminal, as in Git; an elevated `bolt start` that makes such a project says it will open only there unless the folder is in `safe-folders`. A project directly in the user's Windows home folder (`C:\Users\<name>`, which Windows gives to SYSTEM) needs a `safe-folders` entry. A folder whose owner can't be told (FAT, exFAT, some network shares) is refused with a message that says so. `bolt start` ignores an enclosing `.bolt` that isn't the user's. Nothing inside a `.bolt` is looked at before its owner is known. Remaining risk: where another account may rename folders inside a folder that counts as the user's, it can rename one the user owns to `.bolt` and fill it; the user's commands below it, outside their own projects, then use it. On Windows that includes every folder made directly under `C:\`, where all signed-in users may change what is inside, and, in an elevated terminal, folders the Administrators group owns. Git has the same gap | 025 (red team), 026, 027, 030, 031, 042 |
| T24 | Submodule tricks: `.gitmodules` names holding `../`, paths into `.git`, addresses that start with `-` or run commands (`ext::`, `update = !cmd`), values with a trailing CR, a submodule's `.git` pointer file aimed elsewhere | Nothing about a submodule is fetched, run or parsed: an import turns it into a project link holding only the commit and never reads `.gitmodules`, which comes in as an ordinary file (or a link, if Git holds one); a save keeps every project link as it is and never looks inside its folder, so a submodule's `.git` pointer file is never read or stored | 004, 036 |

## 5. Engineering rules (approved, decision 022)

- The engine forbids `unsafe` code (`#![forbid(unsafe_code)]`). Memory-mapping files is avoided in v0.1: it needs `unsafe`, and a file changed underneath can crash the process.
- The one exception is a tiny, separate platform crate, which may use `unsafe` for operating-system calls only (for example Windows' Restart Manager, hiding `.bolt/`, file IDs). The owner reviews it line by line.
- No panics in library code: errors are returned, and clippy's `unwrap_used`, `expect_used` and `panic` lints are denied outside tests.
- One place per rule: name validation, path building and write-out each live in a single module that everything else must use.
- The owner reviews anything that touches disk or network before it is merged.
- Spec before code for anything on disk, and test vectors for every format.

## 6. Security tests

See [testing.md](testing.md) §3: fuzz targets for the decoders and parsers listed there (the project link decoder has none yet: a follow-up), a malicious-repository corpus with at least one case per threat above, Windows-specific tests, and crash tests.

**Hostile Git repositories** (the review of Git's published vulnerabilities, 2026-11-02): `crates/bolt-vcs/tests/corpus_git.rs` holds 25 tests, one of them only on Windows, for 22 of the review's 24 kinds of case. The unusual commits a save keeps exactly are in `import_vs_git.rs`: `Craft` could write them too, but there the test also compares each with Git. A `.git` another account owns is tested in `git/mod.rs`, the only place its owner can be stubbed. A SHA-1 collision can't be built as a Git object, so `git/id.rs` feeds SHAttered's blocks to the hasher. Six fuzz targets cover the Git reader: loose objects, packs and their indexes, deltas, commits, trees and tags, refs, and config.

**Red-team sessions** ([decision 024](decisions/024-red-team-gate.md)). Every milestone ends with a time-boxed session in which a fresh agent attacks Bolt from an isolated sandbox and must prove each finding with a failing test. Every proven finding is fixed, or ruled on in writing, before the milestone is merged; its proof test stays as a permanent test ([testing.md](testing.md) §3), and new threats are added to §4.

**Session reports:**
- Milestone 1, session 1 (2026-11-01): [findings](redteam/2026-11-01-milestone-1.md); proof tests in `crates/bolt-vcs/tests/redteam.rs`.
- Milestone 1, session 2, on the fixes (2026-11-02): [findings](redteam/2026-11-02-milestone-1-session-2.md); proof tests in `crates/bolt-vcs/tests/redteam2.rs`.

**Windows name rules.** Checked on 2026-11-01 against Microsoft's "Naming Files, Paths, and Namespaces" page (dated 2024-08-28): Bolt refuses everything on that list, plus the console devices `CONIN$` and `CONOUT$`, while `COM0` and `LPT0`, which are not on it, stay ordinary names. Bolt also refuses `.bolt`'s short names (`bolt~1` to `bolt~4`) and its stream syntax (`.bolt::$INDEX_ALLOCATION`), because Windows treats both as `.bolt`.

**GitHub settings** (turned on by the owner on 2026-11-01):
- **On:**
  - dependency graph;
  - Dependabot alerts, malware alerts and grouped security updates;
  - Dependabot version updates, configured in `.github/dependabot.yml`;
  - CodeQL code scanning (default setup, which scans the Rust code, the workflows and the red-team Python scripts) with Copilot Autofix suggestions;
  - AI scans of the repository and of pull requests;
  - secret scanning with push protection;
  - private vulnerability reporting.
- **Off on purpose:**
  - agentic autofix: it spends AI credits and opens pull requests on its own;
  - automatic dependency submission: GitHub reads `Cargo.lock` directly.
- CI itself runs with read-only permissions, and its actions are pinned to exact commits ([testing.md](testing.md)).
- **Branch protection** (on since 2026-11-02, the ruleset "Protect main"):
  - force pushes to `main` and its deletion are blocked;
  - every change needs a pull request;
  - the CI checks (`test` on all three systems, and `deny`) and CodeQL must pass.

  Nobody can bypass it, the owner included, so every change, an agent's too, reaches `main` only through a pull request whose checks are green.

## 7. Open items

- ~~Review Git's published vulnerabilities (path, symlink, case and submodule bugs) and add every relevant case to the corpus.~~ Done 2026-11-03 (the review itself, 2026-11-02): the cases are in `crates/bolt-vcs/tests/corpus_git.rs` (§6), and those about writing files out are milestone 3 follow-ups.
- Write the server's threat model in detail before v0.3 (authentication, permissions, rate limits, audit log).
- Add a `SECURITY.md` explaining how to report vulnerabilities before any public release.
- Windows: Bolt refuses every reparse point inside `.bolt/`. When saving, it refuses every file that is a reparse point but not a link, such as a OneDrive or Dropbox placeholder, saying it is a special file Bolt can't save yet; links and junctions are saved as links, and a folder that is a non-link reparse point is walked like any other. It is untested whether synced folders expose these to Bolt. If they do, projects in synced folders can't be used. The safe way to support them is to reopen without `FILE_FLAG_OPEN_REPARSE_POINT` and check that it is the same file (an operating-system call, so it goes in the platform crate). This needs a check on a real synced folder first: the owner tries a project in a OneDrive folder before milestone 2 is merged ([milestones.md](milestones.md)).
