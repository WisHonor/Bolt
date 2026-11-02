# Bolt Security Plan — Draft

> **Status:** design-phase draft (2026-10-31). Every defense here has a test in [testing.md](testing.md). The Windows naming rules were checked against Microsoft's documentation on 2026-11-01 (see §6 and decision 004).

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
| T1 | Path tricks: `..`, absolute paths, drive letters (`C:`), network paths (`\\server`), backslashes | Name rules on every listing entry (no empty, `.` or `..` parts, no `/` or NUL); names containing `\` or `:` may be stored but are never written out on Windows; write-out builds paths only from validated names | 004, 019 |
| T2 | Writing into `.bolt/` through spellings Windows or macOS treat as `.bolt` (`.BOLT`, `.bolt.`, `BOLT~1`, `.bolt::$INDEX_ALLOCATION`) | Such names are invalid everywhere | 004 |
| T3 | Writing through a symlink to reach outside the project | Write-out never follows symlinks and checks every path step; symlinks pointing outside become placeholder files | 004, 011 |
| T4 | Two names that become one file on Windows or macOS (`README` / `readme`, Unicode normalization) | Refused on save; reported, never overwritten, on write-out | 004 |
| T5 | Windows device names (`CON`, `NUL`, `AUX`, `COM1`, `CONIN$`…) and `name:stream` | Never written; reported | 004 |
| T6 | Running code from a project (hooks, filters, merge drivers) | No hooks run on copy or import; `bolt.toml` is declarative; programs are named only in personal settings; Git import reads repositories through a library and never runs Git hooks or filters | 008, 017 |
| T7 | Decompression bombs and size bombs, including tree bombs (shared listings describing billions of files) | Every stored object records its real size and decompression stops there; hard limits on names, listings, messages, parents and chunk counts; at most 10,000,000 entries per save | 009, 025, storage spec |
| T8 | Malformed or ambiguous records (two encodings of one record) | Strict readers accept exactly one encoding; fuzz-tested | 005 |
| T9 | Tampered data on disk or from a server | Every object is checked against its fingerprint on every read and on receipt; packs carry index checksums | 005, 009, 015 |
| T10 | Fingerprint collisions | BLAKE3-256 (no known attacks); the algorithm is a per-repository setting, so it can be replaced | 005 |
| T11 | Resource exhaustion (deep nesting, huge entry counts, pathological diffs) | Limits in the storage spec (at most 4,096 folder levels per save); no recursion per folder level; time limits on diffs | storage spec, 025 |
| T12 | Accidentally saving secrets (`.env`, private keys, tokens) | Presets ignore `.env`; `bolt save` warns about files that look like secrets (key files, token formats such as `github_pat_…`, `ghp_…`, `AKIA…`) | 016 |
| T13 | Token theft from disk or logs | Tokens live only in the OS credential store, never in config files or history; redacted in `--debug` output and in the commands recorded in diary entries | 015 |
| T14 | Reading another customer's data by guessing fingerprints (v0.3+) | Dedup only within a project or organization; referencing an existing chunk requires proof of possession | 015 |
| T15 | Network tampering | HTTPS only, and fingerprints are verified anyway | 015 |
| T16 | Lock abuse (v0.3) | Only maintainers break locks; the holder is notified; every lock action is logged on the server | 014 |
| T17 | Unsafe asset previews on the website (later) | Previews made in isolated workers; scripts stripped from SVG; strict content security policy | later |
| T18 | Malicious or vulnerable dependencies | Few dependencies; `Cargo.lock` committed; `cargo-deny` in CI (licenses, bans, and the RustSec advisory database that `cargo-audit` uses); every new dependency reviewed | testing |
| T19 | Links and special files inside `.bolt/` (a symlinked `tmp/` makes cleanup delete files elsewhere; a symlinked `store/packs` or `state/lock` writes elsewhere; a pipe or `/dev/zero` hangs or exhausts memory) | `.bolt/` and every fixed folder in it must be real folders, never links or junctions; its files must be regular files, opened without following links and read with a size cap; all of this in one module; cleanup removes only Bolt's own temporary names | 025 (red team) |
| T20 | A file swapped for a link while `save` reads it, so the link's target is saved | A save opens each file without following a link at the file's own name, and the file it opened (checked on the open file, not on its path) must be a regular file; otherwise the save stops, saves nothing and reports the file as changed during the save. Bolt reads each `.boltignore` once itself, through the same checks. Known limits: a folder swapped for a link while the save walks it waits for milestone 3's handle-relative walking; hard links planted in a shared folder before the save are saved like ordinary files, as in Git | 025 (red team) |
| T21 | Terminal spoofing: bidi overrides, line separators and control characters in names, messages, paths and errors | Every untrusted piece of text is escaped before printing: control characters, bidi controls, line and paragraph separators | 025 (red team) |
| T22 | A project claiming to be the user (its settings set the name and email on the user's saves) | Identity comes only from the environment and the user's own settings; a project's identity is ignored, with a note | 025 (red team) |
| T23 | A repository planted in a parent folder (`C:\.bolt`, `/tmp/.bolt`) capturing commands run below it | Refuse a repository unless both the project folder and its `.bolt` folder belong to the user, as Git requires; listing that exact folder in the user's own `safe-folders` trusts both. On Windows, a folder owned by the Administrators group counts as the user's when the user is an administrator, elevated or not. A folder whose owner can't be told (FAT, exFAT, some network shares) is refused with a message that says so. `bolt start` ignores an enclosing `.bolt` that isn't the user's. Nothing inside a `.bolt` is looked at before its owner is known. Remaining risk: on a drive whose root folder the Administrators group owns, a standard user who can rename a folder that group owns could plant it there for every administrator | 025, 026, 027 (red team) |

## 5. Engineering rules (approved, decision 022)

- The engine forbids `unsafe` code (`#![forbid(unsafe_code)]`). Memory-mapping files is avoided in v0.1: it needs `unsafe`, and a file changed underneath can crash the process.
- The one exception is a tiny, separate platform crate, which may use `unsafe` for operating-system calls only (for example Windows' Restart Manager, hiding `.bolt/`, file IDs). The owner reviews it line by line.
- No panics in library code: errors are returned, and clippy's `unwrap_used`, `expect_used` and `panic` lints are denied outside tests.
- One place per rule: name validation, path building and write-out each live in a single module that everything else must use.
- The owner reviews anything that touches disk or network before it is merged.
- Spec before code for anything on disk, and test vectors for every format.

## 6. Security tests

See [testing.md](testing.md) §3: fuzz targets for every decoder and parser, a malicious-repository corpus with at least one case per threat above, Windows-specific tests, and crash tests.

**Red-team sessions** ([decision 024](decisions/024-red-team-gate.md)). Every milestone ends with a time-boxed session in which a fresh agent attacks Bolt from an isolated sandbox and must prove each finding with a failing test. Findings are fixed before the milestone counts as done; their tests join the corpus, and new threats are added to §4.

**Session reports.** Milestone 1, session 1 (2026-11-01): [findings](redteam/2026-11-01-milestone-1.md); proof tests in `crates/bolt-vcs/tests/redteam.rs`.

**Windows name rules.** Checked on 2026-11-01 against Microsoft's "Naming Files, Paths, and Namespaces" page (dated 2024-08-28): Bolt refuses everything on that list, plus the console devices `CONIN$` and `CONOUT$`, while `COM0` and `LPT0`, which are not on it, stay ordinary names. Bolt also refuses `.bolt`'s short names (`bolt~1` to `bolt~4`) and its stream syntax (`.bolt::$INDEX_ALLOCATION`), because Windows treats both as `.bolt`.

## 7. Open items

- Review Git's published vulnerabilities (path, symlink, case and submodule bugs) and add every relevant case to the corpus.
- Write the server's threat model in detail before v0.3 (authentication, permissions, rate limits, audit log).
- Add a `SECURITY.md` explaining how to report vulnerabilities before any public release.
- Windows: Bolt refuses every reparse point, including non-link ones such as OneDrive and Dropbox placeholders, both inside `.bolt/` and when saving. It is untested whether synced folders expose these to Bolt. If they do, projects in synced folders can't be used. The safe way to support them is to reopen without `FILE_FLAG_OPEN_REPARSE_POINT` and check that it is the same file (an operating-system call, so it goes in the platform crate). This needs a check on a real synced folder first.
