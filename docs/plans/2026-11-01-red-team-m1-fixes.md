# Red-team fixes for milestone 1 — Implementation Plan

> **Status: done (2026-11-02).** Tasks 1–8 are done, and so are the extra tasks added on the way: 5b (the comparison of two saves streams, so very long paths can't exhaust memory), 9 (Windows reserved names follow Microsoft's list), 10 (decisions 026 and 027 in code: drives that record no owner, `bolt start` below a `.bolt` it may not use, no link followed while searching for a project, and the project folder must be the user's too), 11 (shorter code comments), 12 (read-only CI permissions, actions pinned to commits, Dependabot), 13 (red-team session 2's proof tests), and 14–16 (fixes for session 2's findings RT2-1 to RT2-9). Rulings made on the way: decisions [026](../decisions/026-ownership-check-details.md), [027](../decisions/027-project-folder-must-be-yours.md) and [028](../decisions/028-red-team-m1-session-2-rulings.md). Red-team session 2 attacked the fixes on 2026-11-02 ([report](../redteam/2026-11-02-milestone-1-session-2.md)); its findings are fixed too. A final review of the whole branch led to one more round of fixes, ruled in [decision 029](../decisions/029-red-team-m1-final-review-rulings.md). The task text below is kept as planned; a note marks each rule that changed later.

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close every proven finding of milestone 1's red-team session (RT-1 to RT-11) and add the ownership check decided in decision 025, before milestone 2 starts.

**Architecture:** One new engine module, `safe_fs`, becomes the only way Bolt opens files and checks folders inside `.bolt/` (and the `.boltignore` and settings readers): no link following, regular files only, capped reads. The platform crate gains a flag helper for no-follow opening (safe code) and an ownership check (operating-system calls). Save-wide limits move into one place used by every walk over a save. The attacker's proof tests are brought into the repository first, ignored, and each task un-ignores the tests it fixes. *(Ruled during Task 3: the user's own `config.toml` may be a link, as dotfile managers make it, but must still be a regular file of at most 1 MiB; only files a project can bring are never followed.)*

**Tech Stack:** Rust 1.99.1 (edition 2024), existing crates; new direct dependency `libc` (unix only, platform crate).

**Spec:** [decision 025](../decisions/025-red-team-m1-rulings.md), [security.md](../security.md) §4 T7, T11, T19–T23, [storage-format.md](../storage-format.md) §3 (save-wide limits), §10 (shape of `.bolt/`), §12 (`.boltignore`). The red-team report: `docs/redteam/2026-11-01-milestone-1.md` (added by Task 1).

## Global Constraints

- Everything in CLAUDE.md applies: `#![forbid(unsafe_code)]` in the engine; `unsafe` only in `bolt-vcs-platform`, each block with a `// SAFETY:` comment; no `unwrap`/`expect`/`panic` outside tests; one place per rule; tests pass on Windows, macOS and Linux.
- Limits, verbatim from the spec: at most **4,096** folder levels below a save's root; at most **10,000,000** entries per save, counting every path; `format` at most **4 KiB**; `state/head` at most **65 bytes**; `state/lock-info` at most **4 KiB**; `config.toml` (user's and project's) at most **1 MiB**; `.boltignore` at most **1 MiB**.
- Temporary files Bolt owns in `tmp/` are named `tmp-` + 16 lowercase hex digits (fsio::create_temp); only those are ever deleted.
- The fixed folders that must be real folders: `.bolt/`, `.bolt/store/`, `.bolt/store/packs/`, `.bolt/state/`, `.bolt/cache/`, `.bolt/tmp/`.
- Every proof test in `crates/bolt-vcs/tests/redteam.rs` keeps its name and its assertion; a task may adjust setup only if the fix changes a message the test matches, and says so in its report.
- Commit messages carry no Claude attribution.

## Review Focus

1. **A missing optional file must still read as missing.** `state/head` absent before the first save and an absent `config.toml` are normal; `safe_fs` must return `io::ErrorKind::NotFound` unchanged so callers keep treating them as absent (Task 2 test `missing_file_stays_not_found`).
2. **Ordinary projects must keep working on Windows.** No-follow opening must not refuse plain files on NTFS, and `.bolt` hidden by `hide_path` must still pass `check_real_dir` (Task 3 test `hidden_bolt_folder_is_a_real_folder`; the existing Windows CLI tests).
3. **A symlinked `.boltignore` is now an error, not ignored.** The message must name the file and say it must be a regular file (Task 3 test `symlinked_boltignore_stops_the_save`).
4. **Saves at exactly the limits must still work** (4,096 levels, and a count that reaches but does not pass the entry limit), so the limit is "more than", not "at" (Task 5 tests `depth_at_the_limit_is_accepted` and `entry_limit_counts_every_path`).
5. **The ownership check must not lock users out of their own projects**, including folders created by an elevated administrator on Windows and the GitHub CI runners' temp folders (Task 8 tests `own_temp_folder_is_owned`, and the full CLI suite on all three systems).

---

### Task 1: Bring the red-team proof into the repository

**Files:**
- Create: `crates/bolt-vcs/tests/redteam.rs` (copied from the session copy), `docs/redteam/2026-11-01-milestone-1.md` (the report), `redteam/attacks/{rt4-dev-zero-head.sh,crash-sweep.sh,save-probes.sh}`
- Modify: `docs/security.md` §6 (one line linking the report)

Source: `.superpowers/redteam-m1/` in the repository folder (git-ignored): `redteam.rs`, `report-m1.md`, `attacks/*.sh`. Read them; never run the attack scripts on the host.

- [ ] **Step 1:** Copy them into the repository: `redteam.rs` → `crates/bolt-vcs/tests/redteam.rs`, `report-m1.md` → `docs/redteam/2026-11-01-milestone-1.md`, `attacks/*.sh` → `redteam/attacks/`. In the report, change the "Proof tests" line to point at `crates/bolt-vcs/tests/redteam.rs` and the run command to `cargo test -p bolt-vcs --test redteam`.
- [ ] **Step 2:** Mark every test that fails today `#[ignore = "RT-<n>: fixed by Task <k> of plans/2026-11-01-red-team-m1-fixes.md"]` using this map: RT-1, RT-2, RT-3, RT-4 → Task 3; RT-5 and `win_junction_in_project_is_not_walked_into` → Task 4; RT-6, RT-10 → Task 5; RT-7, RT-8 → Task 6; RT-9 → Task 7. The Windows tests (`rt1_windows_…`, `win_junction_…`) are marked the same way.
- [ ] **Step 3:** Verify: `cargo test -p bolt-vcs --test redteam` passes with every failing test ignored (on Windows: the 3 Windows tests ignored, the RT-7 and RT-9 tests ignored, RT-6 ignored). Linux proof that they fail already exists in the report; don't run attack code on the host. Then fmt, clippy, full tests.
- [ ] **Step 4:** Commit: `Add the milestone 1 red-team proof tests, ignored until fixed`.

### Task 2: `safe_fs`, the one way to open files Bolt doesn't trust

**Files:**
- Modify: `crates/bolt-vcs-platform/src/lib.rs`, `crates/bolt-vcs-platform/Cargo.toml` (unix: `libc`)
- Create: `crates/bolt-vcs-core/src/safe_fs.rs`; Modify: `crates/bolt-vcs-core/src/lib.rs`

**Interfaces:**
- Produces (platform): `pub fn no_follow(options: &mut std::fs::OpenOptions) -> &mut std::fs::OpenOptions` — unix: `custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)`; Windows: `custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)`. Safe code, no `unsafe`.
- Produces (core, `bolt_core::safe_fs`):
  - `pub fn open_regular(path: &Path, options: &OpenOptions) -> io::Result<File>` — applies `no_follow` to a clone of `options`, opens, then checks the open handle's metadata: must be a regular file, and on Windows must not carry `FILE_ATTRIBUTE_REPARSE_POINT` (0x400). A link (including unix `ELOOP` from `O_NOFOLLOW`), folder, pipe or device gives `io::ErrorKind::InvalidInput` with message `"<path> is not a regular file (it is a link, folder or special file)"`. `NotFound` is returned unchanged.
  - `pub fn read_capped(path: &Path, max: u64) -> io::Result<Vec<u8>>` — `open_regular` for reading, reads at most `max + 1` bytes; more than `max` gives `io::ErrorKind::InvalidData` with `"<path> is larger than <max> bytes"`.
  - `pub fn check_real_dir(path: &Path) -> io::Result<()>` — `symlink_metadata`: must be a folder, not a symlink, and on Windows not a reparse point; otherwise `InvalidInput` with `"<path> is not a real folder (it is a link, junction or file)"`. `NotFound` unchanged.
  - The limits as constants: `MAX_FORMAT_BYTES: u64 = 4096`, `MAX_HEAD_BYTES: u64 = 65`, `MAX_LOCK_INFO_BYTES: u64 = 4096`, `MAX_CONFIG_BYTES: u64 = 1 << 20`, `MAX_BOLTIGNORE_BYTES: u64 = 1 << 20`.

- [ ] **Step 1: Write the failing tests** in `safe_fs.rs` (unit tests; unix-only ones `#[cfg(unix)]`, Windows-only `#[cfg(windows)]`):
  - `regular_file_reads_back` — a 3-byte file reads back with `read_capped(.., 3)`.
  - `missing_file_stays_not_found` — `read_capped` and `open_regular` on a missing path give `ErrorKind::NotFound`.
  - `over_the_cap_is_refused` — a 4-byte file with `max = 3` gives `InvalidData`.
  - `symlink_is_refused` (unix: `std::os::unix::fs::symlink`; Windows: `std::os::windows::fs::symlink_file`, skip if it fails for lack of privilege) — `InvalidInput`, and the link's target is untouched.
  - `fifo_is_refused_without_hanging` (unix; create with `mkfifo` via `std::process::Command`) — returns `InvalidInput` within the test (no hang).
  - `folder_is_refused` — `open_regular` on a folder gives `InvalidInput`.
  - `check_real_dir_refuses_links_and_files` — a real folder passes; a symlink to a folder and a regular file give `InvalidInput`; Windows: a junction made with `cmd /c mklink /J` gives `InvalidInput`.
- [ ] **Step 2:** Run `cargo test -p bolt-vcs-core safe_fs` — fails (module missing).
- [ ] **Step 3:** Implement the platform helper and the module as specified.
- [ ] **Step 4:** Run the tests — pass. fmt, clippy, full tests.
- [ ] **Step 5:** Commit: `Add safe_fs: no-follow, regular-file-only, capped reads`.

### Task 3: Every `.bolt/` access goes through `safe_fs` (RT-1 to RT-4)

**Files:**
- Modify: `crates/bolt-vcs-core/src/repo.rs` (`init`, `open`, `discover`, `read_head`, `head`, `clean_tmp`, lock-info read in `lock_for_change`), `fsio.rs` (`open_lock_file`), `store.rs` (`Store::open`/`refresh`), `pack.rs` (`PackReader::open`), `config.rs` (`from_file`), `snapshot.rs` (`.boltignore` reading in `walk`)
- Test: `crates/bolt-vcs/tests/redteam.rs` (un-ignore RT-1 to RT-4, including `rt1_windows_…`), unit tests in `repo.rs` and `snapshot.rs`

**Interfaces:**
- Consumes: Task 2's `safe_fs` functions and constants.
- Produces: `RepoError::Damaged` is used for a repository whose shape breaks §10 (message names the path and says it is a link or not a regular file; hint: the existing damaged hint).

- [ ] **Step 1: Write the failing tests:**
  - un-ignore the RT-1 to RT-4 tests in `redteam.rs`;
  - `repo.rs`: `open_refuses_a_linked_fixed_folder` (each of the six fixed folders replaced by a symlink in turn → `open` returns `RepoError::Damaged`, and a file in the link's target survives); `clean_tmp_removes_only_bolts_own_names` (`tmp-0123456789abcdef` removed, `notes.txt` and `tmp-xyz` kept); `hidden_bolt_folder_is_a_real_folder` (after `init`, `check_real_dir(.bolt)` is `Ok` on every platform);
  - `snapshot.rs`: `symlinked_boltignore_stops_the_save` (unix) — error mentions `.boltignore` and "regular file"; `oversized_boltignore_stops_the_save` — 1 MiB + 1 byte.
- [ ] **Step 2:** Run them — fail.
- [ ] **Step 3: Implement:**
  - `open`: `check_real_dir` on the six fixed folders before reading anything; `format` via `read_capped(.., MAX_FORMAT_BYTES)`; head via `read_capped(.., MAX_HEAD_BYTES)` (NotFound → `None` as now). `discover` keeps finding `format` by `is_file()` but `open` does the checks. *(Changed by Task 10, decision 026: the searches never follow a link, and Bolt asks who owns the folder and `.bolt` before it looks inside `.bolt`, so the owner check comes before these shape checks.)*
  - `init`: before creating anything, if `.bolt` exists it must pass `check_real_dir` (a link → `Damaged`); after creating the folders, `check_real_dir` on all six.
  - `clean_tmp`: delete only entries whose name is `tmp-` + 16 lowercase hex and whose `symlink_metadata` is a regular file.
  - `fsio::open_lock_file`: open through `safe_fs::open_regular` with the same options (create, read, write, no truncate).
  - lock-info read: `read_capped(.., MAX_LOCK_INFO_BYTES)`, any error → empty text as now.
  - `Store::open`/`refresh`: skip entries that aren't `*.pack`; open each pack with `open_regular` (pass the `File` to `PackReader`, e.g. `PackReader::from_file(file, path)` — keep `PackReader::open(path)` as a thin wrapper that uses `open_regular`).
  - `config.rs::from_file`: `read_capped(.., MAX_CONFIG_BYTES)`; NotFound → `Ok(None)`.
  - `.boltignore` in `walk`: `read_capped(.., MAX_BOLTIGNORE_BYTES)`; errors stop the save with the existing ignore-problem error naming the file.
- [ ] **Step 4:** Run the tests — pass (Linux RT tests in CI or the sandbox; the Windows twin in CI). fmt, clippy, full tests.
- [ ] **Step 5:** Commit: `Refuse links and special files inside .bolt and cap every read (RT-1 to RT-4)`.

### Task 4: `save` never follows a file swapped for a link (RT-5)

**Files:**
- Modify: `crates/bolt-vcs-core/src/snapshot.rs` (`store_file`, walker settings if the junction test needs it)
- Test: `redteam.rs` (un-ignore `rt5_…` and `win_junction_in_project_is_not_walked_into`), `snapshot.rs` unit test

**Interfaces:**
- Consumes: `safe_fs::open_regular`.

- [ ] **Step 1: Write the failing tests:** un-ignore the two tests; add `snapshot.rs` test `file_replaced_by_a_link_between_scan_and_read_is_refused` (unix): call `store_file` on a path that is a symlink to a file outside → `SaveError::Unreadable` (or `ChangedDuringSave`), and nothing is added to the pack.
- [ ] **Step 2:** Run — fail.
- [ ] **Step 3: Implement:** `store_file` opens with `safe_fs::open_regular(path, OpenOptions::new().read(true))`. A refusal maps to `SaveError::ChangedDuringSave { path }` (the scan saw a regular file, so a link or special file now means it changed during the save). If the junction test shows the walker descending into junctions on Windows, make the walker treat reparse-point folders as links (record them as symlinks, never walk into them).
- [ ] **Step 4:** Run — pass. fmt, clippy, full tests.
- [ ] **Step 5:** Commit: `Open saved files without following links (RT-5)`.

Out of scope, recorded in the follow-ups: a *folder* swapped for a link while the walk reads it needs handle-relative walking (`openat`-style); milestone 3's write-out needs the same machinery, so both are built there.

### Task 5: Save-wide limits on depth and size (RT-6, RT-10)

**Files:**
- Create: `crates/bolt-vcs-core/src/limits.rs` (or constants in `listing.rs` if that's where §3 limits already live — use that file)
- Modify: `crates/bolt-vcs-core/src/diff.rs` (iterative walk with counting), `snapshot.rs` (`walk` refuses trees past the limits)
- Test: `redteam.rs` (un-ignore RT-6, RT-10), `diff.rs` and `snapshot.rs` unit tests

**Interfaces:**
- Produces: `pub const MAX_SAVE_DEPTH: usize = 4096;` `pub const MAX_SAVE_ENTRIES: u64 = 10_000_000;` and `StoreError::SaveTooLarge { detail: String }` (Display: `"this save is too large to read safely: <detail>"`), mapped by the CLI like other damage. `SaveError::TooLarge { detail: String }` for a working folder past the limits (hint: ignore the deep or huge folder in `.boltignore`).

- [ ] **Step 1: Write the failing tests:**
  - un-ignore RT-6 and RT-10;
  - `diff.rs`: `depth_at_the_limit_is_accepted` (a chain of exactly 4,096 nested folders below the root, one file at the bottom: `diff_saves` from `None` lists that one file) and `depth_past_the_limit_is_refused` (4,097 → `SaveTooLarge`); `entry_limit_counts_every_path` — build listings where a 2-entry listing is shared by 3 parents, call the walk with a test-only limit (expose `diff_saves_with_limit(store, old, new, max_entries)` as `pub(crate)`) set to 8 → ok, 7 → `SaveTooLarge`.
  - `snapshot.rs`: `folder_deeper_than_the_limit_is_refused` (unix: create 4,097 nested folders on disk; use short names so paths stay under OS limits — skip on platforms where creation fails with a path-length error, and say so in the test).
- [ ] **Step 2:** Run — fail.
- [ ] **Step 3: Implement:** `diff_folders` becomes a loop over an explicit stack of `(old, new, prefix, depth)`; count every entry visited (both sides) against `MAX_SAVE_ENTRIES` and depth against `MAX_SAVE_DEPTH`; output order stays sorted by path as now. `walk` counts depth and entries the same way while scanning. *(Ruled during Task 5: each side is counted against the limit on its own, not the two together, as storage-format.md §3 now says.)*
- [ ] **Step 4:** Run — pass. fmt, clippy, full tests.
- [ ] **Step 5:** Commit: `Limit how deep and how large one save may be (RT-6, RT-10)`.

### Task 6: Terminal text can't lie (RT-7, RT-8, RT-11)

**Files:**
- Create: `crates/bolt-vcs-core/src/text.rs` (move `sanitize` here as `pub fn escape(text: &str, keep_newlines: bool) -> String`)
- Modify: `crates/bolt-vcs/src/output.rs` (use `bolt_core::text::escape`; `print_error` keeps newlines), `main.rs` (`start` prints the path escaped), `snapshot.rs` (`describe`: an unreadable folder is reported as unreadable, not as a `.boltignore` problem; every name or path placed in a `SaveError` message goes through `escape(.., false)`), other core error messages that embed a path or name.
- Test: `redteam.rs` (un-ignore RT-7, RT-8), unit tests in `text.rs`, CLI test for RT-11.

**Interfaces:**
- Produces: `bolt_core::text::escape(text: &str, keep_newlines: bool) -> String` — escapes as `\x{XX}`/`\u{XXXX}` every char that is `char::is_control()` (except `\n` when `keep_newlines`), every bidi control (U+061C, U+200E, U+200F, U+202A–U+202E, U+2066–U+2069) and U+2028, U+2029. *(Decision 028 later widened this to every invisible format character, Unicode category Cf, except the zero-width joiner and non-joiner, plus the Hangul fillers.)*

- [ ] **Step 1: Write the failing tests:** un-ignore RT-7 and RT-8; `text.rs`: `bidi_and_line_separators_are_escaped` (`"a\u{202e}b\u{2028}c"` → `"a\\u{202e}b\\u{2028}c"`), `newlines_kept_only_when_asked`, `plain_text_unchanged` (incl. `"café ✓"`); `cli.rs`: `unreadable_folder_is_reported_as_unreadable` (unix: chmod 000 a subfolder; stderr contains "can't read" or "unreadable" and not ".boltignore"), `multi_line_error_prints_lines` (a save refused for two non-portable names prints them on separate lines, no literal `\x0a`).
- [ ] **Step 2:** Run — fail.
- [ ] **Step 3:** Implement as specified. Untrusted pieces are escaped where they enter a message; `print_error` then prints with newlines kept.
- [ ] **Step 4:** Run — pass; update `insta` snapshots only where the change is intended, and list them in the report. fmt, clippy, full tests.
- [ ] **Step 5:** Commit: `Escape bidi and separator characters; clearer multi-line errors (RT-7, RT-8, RT-11)`.

### Task 7: Your identity comes only from you (RT-9)

**Files:**
- Modify: `crates/bolt-vcs-core/src/config.rs`, `crates/bolt-vcs/src/main.rs` (`save` prints the note)
- Test: `redteam.rs` (un-ignore RT-9), `config.rs` unit tests, `cli.rs`

**Interfaces:**
- Produces: `pub fn load_identity(project_root: Option<&Path>) -> Result<Identity, ConfigError>` keeps its signature with the new order: env vars; a `[[project]]` entry in the user's `config.toml` whose `folder` equals `project_root` (compare canonicalized paths exactly, on every system; Task 8 dropped the case-insensitive match on Windows, so names that only look alike are different folders); the user's `[user]`. Plus `pub fn ignored_project_identity(project_root: &Path) -> Option<Identity>`: the identity named in `.bolt/config.toml`, if any, when it differs from what `load_identity` returned (read with `read_capped`; any error → `None`).
- User `config.toml` shape (decision 025): `[[project]]` tables with `folder`, `name`, `email`.

- [ ] **Step 1: Write the failing tests:** un-ignore RT-9; `config.rs`: `project_entry_in_user_config_wins_for_that_folder`, `project_entry_for_another_folder_is_ignored`, `bolt_config_identity_is_never_used`; `cli.rs`: `save_notes_an_ignored_project_identity` — stdout (not `--json`) contains "This project's settings name Mallory <mallory@evil.example>; Bolt ignores identities from project settings." followed by the `[[project]]` snippet with this folder; `--json` output gains `"ignored_project_identity": {"name":…, "email":…}`.
- [ ] **Step 2:** Run — fail.
- [ ] **Step 3:** Implement as specified. The note's text is exactly: `note: This project's settings name <name> <<email>>; Bolt ignores identities from project settings.` then `hint: To save as them in this project, add this to <user config path>:` and the snippet (escaped with `text::escape`).
- [ ] **Step 4:** Run — pass. fmt, clippy, full tests.
- [ ] **Step 5:** Commit: `Never take the user's identity from a project's settings (RT-9)`.

### Task 8: Refuse a repository owned by someone else (T23)

**Files:**
- Modify: `crates/bolt-vcs-platform/src/lib.rs`, `crates/bolt-vcs-platform/Cargo.toml` (Windows features `Win32_Security`, `Win32_Security_Authorization`, `Win32_System_Threading` as needed), `crates/bolt-vcs-core/src/repo.rs` (`open`, `init`'s ancestor check), `config.rs` (`safe-folders`)
- Test: platform unit tests, `repo.rs` unit tests, `cli.rs`

**Interfaces:**
- Produces (platform): `pub fn is_owned_by_current_user(path: &Path) -> io::Result<bool>` — unix: `MetadataExt::uid()` of `symlink_metadata(path)` equals `libc::geteuid()`; Windows: the owner SID of `path` (`GetNamedSecurityInfoW`, `OWNER_SECURITY_INFORMATION`) equals the current process token's user SID (`OpenProcessToken`, `GetTokenInformation(TokenUser)`, `EqualSid`), or the owner is BUILTIN\Administrators (`IsWellKnownSid(WinBuiltinAdministratorsSid)`) and the token is a member (`CheckTokenMembership`). Every handle and buffer freed on every path (`CloseHandle`, `LocalFree`). Each `unsafe` block with `// SAFETY:`. *(Changed by decision 026 and Task 10: it became `owner_of(path) -> io::Result<Owner>`, which answers the user's, another account's, or can't tell; the Administrators group counts when it is on the token at all, enabled or for deny only, not through `CheckTokenMembership`.)*
- Produces (core): `RepoError::NotYours { path: PathBuf }` — Display: `"the project at <path> belongs to another user account"`; hint: `"If you trust it, add its folder to safe-folders in <user config path>: safe-folders = ['<path>']"`. `config.rs`: `pub fn safe_folders() -> Result<Vec<PathBuf>, ConfigError>` reading top-level `safe-folders` (array of strings) from the user's `config.toml`.

- [ ] **Step 1: Write the failing tests:**
  - platform: `own_temp_folder_is_owned` (a fresh `tempdir` → `true`); `system_folder_is_not_owned` (unix: `/` → `false`, skipped when `geteuid() == 0`; Windows: `C:\Windows` → `false`); `missing_path_is_an_error`.
  - core: `owner_rule` — a pure function `fn owner_allows(is_own: bool, root: &Path, safe: &[PathBuf]) -> bool` with cases: own → allowed; not own and not listed → refused; not own and listed (canonical match) → allowed.
  - cli: `start_and_save_work_in_a_normal_folder` already exists — keep it passing on all three systems (review focus 5).
- [ ] **Step 2:** Run — fail.
- [ ] **Step 3:** Implement. `open` checks the owner of `.bolt` after the shape checks and before reading anything else; `init`'s "inside another project" ancestor check reports `NotYours` for an ancestor `.bolt` owned by someone else (so a planted `C:\.bolt` gives a clear message, not a silent capture). *(Changed by decisions 026 and 027: Bolt asks who owns the project folder and its `.bolt` before it looks inside `.bolt`, both must be the user's, and `bolt start` looks past an enclosing `.bolt` it may not use instead of reporting it.)*
- [ ] **Step 4:** Run — pass on Windows locally, Linux in the sandbox; push and check macOS in CI. fmt, clippy, full tests.
- [ ] **Step 5:** Commit: `Refuse repositories owned by another user account (T23)`.

### After Task 8 (controller, not a task)

- Confirm `cargo test -p bolt-vcs --test redteam` runs every test un-ignored and green on all three systems.
- Add the folder-swap race (handle-relative walking) to `plans/2026-11-01-milestone-1-follow-ups.md` under milestone 3.
- Run red-team session 2 (decision 024) on the fixed code: same sandbox, same rules, told what session 1 found and what was fixed, asked to break the fixes. Then the whole-branch review, then merge.
