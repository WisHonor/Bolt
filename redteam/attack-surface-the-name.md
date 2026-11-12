# The name: attack surface (red-team session RT15, decision 024)

Bolt was renamed Opalro, with `opal` for short ([decision 150](../docs/decisions/150-the-name-is-opalro.md)): the command is `opal`, a project keeps its own data in `.opal/`, its rules in `opal.toml` and `.opalignore`, the program's own files start with `OPAL…` magic bytes, and every fingerprint's context string says `opal-vcs`. A script made most of the change, by rewriting names in the code, the tests and the documents. A script misses what it can't see, and three such misses were already caught:

- `is_opal_alias` checked for a first letter `b` before anything else, so it would have let `opal~1` through;
- test data spelled the folder's name in pieces (`.b\u{200C}olt`), in other letter cases (`.Bolt`, `Bolt.toml`), or in folder names chosen to take the short names `BOLT~1` to `BOLT~4`;
- a test that keeps the repository's address out of the shipped documents looked for `wishonor/bolt` after the script had made it `wishonor/opal`.

Your job is to find the ones not caught. Threat list: `docs/security.md` §4. The rules: decisions 004, 008, 059, 106, 121, 148 and 150 in `docs/decisions/`.

Your goals:
- **A repository swallowed:** a name the look-alike checks miss that Windows or macOS opens as `.opal` (or `.git`), so that a save holding it writes into the project's own data, or the walk takes it for the project's folder.
- **The old name still special, or the new one not:** anything that still treats `.bolt`, `bolt.toml`, `.boltignore`, a `BOLT…` magic or a `BOLT_…` variable as the program's own; or a guard the old name had that the new one lost (a check against a name, a first letter, a length, a byte string, a case-folded form).
- **A stored byte read two ways:** a file in `.opal/` with an old magic (`BOLTPACK`, `BOLTPEND`, `BOLTJRNL`, `BOLTFILE`, `BOLTCHID`, `BOLTLINE`), the old format line (`bolt-repository 1`), or an object fingerprinted with an old context string. Each must be refused as damage, or rebuilt where it is a cache, never taken as valid and never a crash. Also two kinds of object whose context strings the rename made equal, so that one can pass for the other.
- **Settings and identity:** the settings folder is `opal`, its override `OPAL_CONFIG_DIR`, the identity variables `OPAL_USER_NAME` and `OPAL_USER_EMAIL`. Old `BOLT_…` variables and the old `bolt` folder must change nothing. A name or email inside a project's `.opal/config.toml` must still never be used (decision 025).
- **Another tool's `.opal` folder:** `opal start` and `opal import` must refuse to settle into a `.opal` folder holding names the program doesn't put there, in every spelling a drive takes for `.opal`.
- **A release build,** not a debug build: build with `cargo build --release -p opal-vcs` and attack that binary.

## Where the code is

- **Names:** `crates/opal-vcs-core/src/names/lookalike.rs` (`is_opal_alias`, `is_git_alias`, `may_spell`, `short_name`, `is_hashed_short_name`); `names.rs` (`validate_name`, `writable_here`, `own_folder`, `listed_hints`); their callers in `snapshot/walk.rs` (asking a folder for `.git` and `.opal` by name, decision 106), write-out (`checkout.rs`, `write_out` and friends), and the import (`import/names.rs`, `import/convert.rs`).
- **The project's folder:** `repo.rs` (`FORMAT_TEXT`, `parse_format`, the check for another tool's folder `holds_only_opals_own`, `OPAL_ENTRIES`).
- **Magic bytes and context strings:** `pack.rs` (`PACK_MAGIC`, `PACK_END_MAGIC`), `journal.rs`, `filecache.rs`, `changeids.rs`, `otherlines.rs`; the context strings are found with `grep -rn "opal-vcs 20" crates/opal-vcs-core/src`.
- **Settings:** `config.rs` (the settings folder, `OPAL_CONFIG_DIR`, the identity variables).
- **The release:** `.github/release/website.sh` (now also refuses a website copy that says Bolt) and `.github/workflows/release.yml` (archives named `opalro-<version>-<system>`). Read only: don't run the workflow.

## Places worth attacking

- Every spelling of `.opal`: each character HFS+ ignores, at each place in the name (`.o\u{200C}pal`, `\u{FEFF}.opal`, `.opa\u{206F}l` …); NTFS forms (`.opal.`, `.opal `, `.opal::$INDEX_ALLOCATION`, `.opal:x`); `opal~1` to `opal~9`, `OPAL~1.`, with `\` and `/` after; mixed letter case; the first character of a name that NTFS or HFS+ drops or folds, against `may_spell`'s fast path.
- A project that holds `.bolt/` (left by a pre-release build), or a save that plants `.bolt`, `bolt.toml` or `.boltignore`: these are now ordinary files and folders, saved and written out like any other.
- The old magic bytes and format line planted in `.opal/`; a pack, journal or cache made by a pre-release build copied into `.opal/`.
- On Windows, only on a `redteam/name-*` CI branch (attack code runs only there and in the sandbox): a folder whose `.opal` gets a hashed short name because `o pal`, `op al`, `opa l` and `o p al` took `OPAL~1` to `OPAL~4` first; then a save naming that hashed short name. RT14's `redteam14_platforms.rs` has the pattern.

## Limits already known (not findings unless you show more)

- The follow-ups list's "Known limit" and "Known cost" rows, and RT14's known costs (docs/redteam/2026-11-11-v0.1-release.md).
- Decision records, plans, rulings, red-team reports, the clean-room records and the code audit keep the old name on purpose (decision 150). So do the GitHub repository's name and the build path the Windows program carries (`D:\a\Bolt\Bolt\…`).
- A project saved by a pre-release build isn't an Opalro project, and its old settings folder isn't read (decision 150).
