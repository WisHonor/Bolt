# Bolt Security Plan — Draft

> **Status:** design-phase draft (2026-10-31). Every defense here has a test in [testing.md](testing.md). The exact Windows naming rules are checked against Microsoft's documentation before milestone 1 implements name checking (decision 004).

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
| T7 | Decompression bombs and size bombs | Every stored object records its real size and decompression stops there; hard limits on names, listings, messages, parents and chunk counts | 009, storage spec |
| T8 | Malformed or ambiguous records (two encodings of one record) | Strict readers accept exactly one encoding; fuzz-tested | 005 |
| T9 | Tampered data on disk or from a server | Every object is checked against its fingerprint on every read and on receipt; packs carry index checksums | 005, 009, 015 |
| T10 | Fingerprint collisions | BLAKE3-256 (no known attacks); the algorithm is a per-repository setting, so it can be replaced | 005 |
| T11 | Resource exhaustion (deep nesting, huge entry counts, pathological diffs) | Limits in the storage spec; no unbounded recursion; time limits on diffs | storage spec |
| T12 | Accidentally saving secrets (`.env`, private keys, tokens) | Presets ignore `.env`; `bolt save` warns about files that look like secrets (key files, token formats such as `github_pat_…`, `ghp_…`, `AKIA…`) | 016 |
| T13 | Token theft from disk or logs | Tokens live only in the OS credential store, never in config files or history; redacted in `--debug` output and in the commands recorded in diary entries | 015 |
| T14 | Reading another customer's data by guessing fingerprints (v0.3+) | Dedup only within a project or organization; referencing an existing chunk requires proof of possession | 015 |
| T15 | Network tampering | HTTPS only, and fingerprints are verified anyway | 015 |
| T16 | Lock abuse (v0.3) | Only maintainers break locks; the holder is notified; every lock action is logged on the server | 014 |
| T17 | Unsafe asset previews on the website (later) | Previews made in isolated workers; scripts stripped from SVG; strict content security policy | later |
| T18 | Malicious or vulnerable dependencies | Few dependencies; `Cargo.lock` committed; `cargo-deny` and `cargo-audit` in CI; every new dependency reviewed | testing |

## 5. Engineering rules (approved, decision 022)

- The engine forbids `unsafe` code (`#![forbid(unsafe_code)]`). Memory-mapping files is avoided in v0.1: it needs `unsafe`, and a file changed underneath can crash the process.
- The one exception is a tiny, separate platform crate, which may use `unsafe` for operating-system calls only (for example Windows' Restart Manager, hiding `.bolt/`, file IDs). The owner reviews it line by line.
- No panics in library code: errors are returned, and clippy's `unwrap_used`, `expect_used` and `panic` lints are denied outside tests.
- One place per rule: name validation, path building and write-out each live in a single module that everything else must use.
- The owner reviews anything that touches disk or network before it is merged.
- Spec before code for anything on disk, and test vectors for every format.

## 6. Security tests

See [testing.md](testing.md) §3: fuzz targets for every decoder and parser, a malicious-repository corpus with at least one case per threat above, Windows-specific tests, and crash tests.

## 7. Open items

- Check the Windows naming rules (reserved names, short names, stream syntax) against Microsoft's documentation before implementing name checks.
- Review Git's published vulnerabilities (path, symlink, case and submodule bugs) and add every relevant case to the corpus.
- Write the server's threat model in detail before v0.3 (authentication, permissions, rate limits, audit log).
- Add a `SECURITY.md` explaining how to report vulnerabilities before any public release.
