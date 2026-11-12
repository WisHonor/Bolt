# The name: attack surface (red-team session RT15, decision 024)

Bolt was renamed Opalro ([decision 150](../docs/decisions/150-the-name-is-opalro.md)). Whatever a person types says `opalro`: the command, the project's own folder `.opalro/`, its rules in `opalro.toml` and `.opalroignore`, the variables `OPALRO_…`, the settings folder `opalro`, the `--json` field `opalro_json`. What only the program uses says `opal`: the magic bytes `OPAL…` at the start of its own files, the fingerprints' context strings `opal-vcs …`, the first line of `.opalro/format` (`opal-repository 1`), the crates. Two script passes made most of the change, by rewriting names in the code, the tests and the documents. A script misses what it can't see, and several misses were already caught:

- the folder guard checked for a first letter `b` before anything else, so it would have let the folder's short name through;
- test data spelled the folder's name in pieces (`.b\u{200C}olt`), in other letter cases (`.Bolt`, `Bolt.toml`), or in folder names chosen to take the short names `BOLT~1` to `BOLT~4`;
- a test that keeps the repository's address out of the shipped documents, the release job's list of archives for the draft, and a bench script's reader of its own results all lost their match when one side of a name changed and the other didn't.

Your job is to find the ones not caught. Threat list: `docs/security.md` §4. The rules: decisions 004, 008, 059, 106, 121, 148 and 150 in `docs/decisions/`.

Your goals:
- **A repository swallowed:** a name the look-alike checks miss that Windows or macOS opens as `.opalro` (or `.git`), so that a save holding it writes into the project's own data, or the walk takes it for the project's folder. The new name is six letters, two more than the old one: check every place that may have assumed four, the short names above all (`OPALRO~1` is a full eight characters).
- **Two names that start alike:** `.opalro` and `.opalroignore` share their first six letters, so on Windows both get a short name `OPALRO~` and a number, whichever was made first taking `~1`. Find a save, an import or a write-out where that lets one stand for the other.
- **The old name still special, or the new one not:** anything that still treats `.bolt`, `.opal`, `bolt.toml`, `opal.toml`, `.boltignore`, `.opalignore`, a `BOLT…` magic or a `BOLT_…` or `OPAL_…` variable as the program's own; or a guard the old name had that the new one lost (a check against a name, a first letter, a length, a byte string, a case-folded form).
- **A stored byte read two ways:** a file in `.opalro/` with an old magic (`BOLTPACK`, `BOLTPEND`, `BOLTJRNL`, `BOLTFILE`, `BOLTCHID`, `BOLTLINE`), the old format line (`bolt-repository 1`), or an object fingerprinted with an old context string. Each must be refused as damage, or rebuilt where it is a cache, never taken as valid and never a crash. Also two kinds of object whose context strings the rename made equal, so that one can pass for the other.
- **Settings and identity:** the settings folder `opalro`, its override `OPALRO_CONFIG_DIR`, the identity variables `OPALRO_USER_NAME` and `OPALRO_USER_EMAIL`. Old `BOLT_…` and `OPAL_…` variables and the old folders must change nothing. A name or email inside a project's `.opalro/config.toml` must still never be used (decision 025).
- **Another tool's `.opalro` folder:** `opalro start` and `opalro import` must refuse to settle into a `.opalro` folder holding names the program doesn't put there, in every spelling a drive takes for `.opalro`.
- **A release build,** not a debug build: build with `cargo build --release -p opal-vcs` (the program is `target/release/opalro`) and attack that binary.

## Where the code is

- **Names:** `crates/opal-vcs-core/src/names/lookalike.rs` (`is_opal_alias`, which checks for `.opalro`; `is_git_alias`; `may_spell`; `short_name`; `is_hashed_short_name`); `names.rs` (`validate_name`, `writable_here`, `own_folder`, `listed_hints`); their callers in `snapshot/walk.rs` (asking a folder for `.git` and `.opalro` by name, decision 106), write-out, and the import (`import/names.rs`, `import/convert.rs`).
- **The project's folder:** `repo.rs` (`FORMAT_TEXT`, `parse_format`, the check for another tool's folder `holds_only_opals_own`, `OPALRO_ENTRIES`).
- **Magic bytes and context strings:** `pack.rs` (`PACK_MAGIC`, `PACK_END_MAGIC`), `journal.rs`, `filecache.rs`, `changeids.rs`, `otherlines.rs`; the context strings are found with `grep -rn "opal-vcs 20" crates/opal-vcs-core/src`.
- **Settings:** `config.rs` (the settings folder, `OPALRO_CONFIG_DIR`, the identity variables).
- **The release:** `.github/release/website.sh` (now also refuses a website copy that says Bolt) and `.github/workflows/release.yml` (archives named `opalro-<version>-<system>`). Read only: don't run the workflow.

## Places worth attacking

- Every spelling of `.opalro`: each character HFS+ ignores, at each place in the name (`.o\u{200C}palro`, `\u{FEFF}.opalro`, `.opalr\u{206F}o` …); NTFS forms (`.opalro.`, `.opalro `, `.opalro::$INDEX_ALLOCATION`, `.opalro:x`); `opalro~1` to `opalro~9`, `OPALRO~1.`, with `\` and `/` after; mixed letter case; the first character of a name that NTFS or HFS+ drops or folds, against `may_spell`'s fast path.
- A project that holds `.bolt/` or `.opal/` (left by a pre-release build), or a save that plants `.bolt`, `.opal`, `bolt.toml`, `opal.toml`, `.boltignore` or `.opalignore`: these are now ordinary files and folders, saved and written out like any other.
- The old magic bytes and format line planted in `.opalro/`; a pack, journal or cache made by a pre-release build copied into `.opalro/`.
- On Windows, only on a `redteam/name-*` CI branch (attack code runs only there and in the sandbox): a folder whose `.opalro` gets a hashed short name because `o palro`, `op alro`, `opa lro` and `opal ro` took `OPALRO~1` to `OPALRO~4` first, then a save naming that hashed short name (RT14's `redteam14_platforms.rs` has the pattern); and a folder where `.opalroignore` was made before `.opalro`.

## Limits already known (not findings unless you show more)

- The follow-ups list's "Known limit" and "Known cost" rows, and RT14's known costs (docs/redteam/2026-11-11-v0.1-release.md).
- Decision records, plans, rulings, red-team reports, the clean-room records and the code audit keep the old name on purpose (decision 150). So do the GitHub repository's name and the build path the Windows program carries (`D:\a\Bolt\Bolt\…`). Rust names inside the code keep `opal` (`is_opal_alias`, `OpalName`).
- A project saved by a pre-release build isn't an Opalro project, and its old settings folder isn't read (decision 150).

## Corrected after the session

- The release builds hold no "bolt" in any letter case: the final review checked pull request #33's builds. The note above about the build path the Windows program carries was wrong.
- The first line of `.opalro/format` is `opalro-repository 1` since the final review, not `opal-repository 1`: a person can read it, and the program prints it when it meets a format it doesn't know ([decision 150](../docs/decisions/150-the-name-is-opalro.md)).
