# Task 7 report: `git/config.rs` rewritten clean

## What I implemented

`crates/bolt-vcs-core/src/git/config.rs` is now a new reader. It no longer re-exports `legacy::git_config`, and the interface the brief gives is unchanged (`Format`, `read_format`, `Worktree`, `read_worktree`, `UserSetting`, `read_user_settings`, `read_user_variable`, `read_bool`, `read_count`). One streaming parser (`read`) gives every entry, with its full name, value and the line it ends on, to a callback. Three layers sit on top of it: the repository's format, the user's settings file, and one environment pair.

### Syntax, as `git config` 2.50 reads it (every rule checked against git)
- A byte order mark is skipped only at the very start. CRLF is read as LF. A lone CR is an ordinary byte, but counts as whitespace between items and at either end of a value. VT and FF aren't whitespace.
- **Headers:** section names may hold letters, digits, `-` and `.`, and are lowercased. A space, tab or CR, then `"subsection"`; even an empty section name may take one (`[ "x"]` is `.x`). In a subsection, `\x` stands for `x`. A newline, or the end of the file, inside it is refused. After the closing quote only `]` may come. Before any section, keys belong to no section.
- **Line Git names for a bad header:** the header's own line, except at the end of the file right after a name (`[a`) or after the closing quote, and at a line end after the closing quote. There Git names the next line. I measured each case.
- **Keys:** start with a letter, then letters, digits or `-`, lowercased. Then only spaces or tabs, followed by `=` or the line end. `k # c`, `k\r = v` and `k \` are refused. A key with no `=` has no value.
- **Values:** whitespace at the start is dropped while the value is empty. Inside, whitespace is kept as written. At the end, unquoted whitespace is dropped, back to the last character, quote or line continuation (`a ""` gives `a `, `a \<LF>  ` gives `a `). The escapes are `\n`, `\t`, `\b`, `\"`, `\\`; a backslash before a line end, or at the end of the file, continues the line. Any other escape is refused. `#` and `;` start a comment outside quotes. A line end or end of file inside quotes is refused at that line.
- **NUL:** in names Git ends the name at the NUL, so `[core "excludesfile\0x"] foo = bar` sets `core.excludesfile`, and Bolt reads it that way. A NUL in a value Bolt *takes* is refused (`Broken`, or `Err(line)` in the user's settings) rather than read short. `git/mod.rs`'s existing test already wanted that for the version.

### Numbers and booleans
- `parse_int` reads like `strtoimax` in base 0: leading C whitespace, a sign, `0x` hex or a leading `0` for octal, and a 64-bit limit. Then one optional `k`/`m`/`g` unit in either case, and the result must fit 32 bits.
- `read_bool` takes `true/yes/on` and `false/no/off` in any letter case, empty as false, or else that integer, true unless 0.
- `read_count` reads like `strtoul` base 10 in a C `unsigned long`; a negative number wraps round. The result must fit an int, and empty is 0. Its error text is now "isn't a number Git reads as a count", which `crates/bolt-vcs/tests/cli.rs` checks word for word.

### `read_format` (Git 2.50's repository checks, observed with `git rev-parse` on made-up repos)
- Each `core.repositoryformatversion` is parsed as it is read; a bad one is `Broken`, and the last one counts. With no version at all, the extensions' values are still checked, but none of them takes effect. A negative version is `Broken`: the existing test wants that, though Git reads it as no version.
- Extension values are checked in file order, under any version:
  - `objectformat` and `compatobjectformat` must be `sha1` or `sha256`. A missing value is refused. A second `compatobjectformat` is refused; so is one equal to the repository's own hash, where Git itself hits a `BUG`.
  - `refstorage` may be `files` or `reftable`. `files://…` gives `RefsElsewhere` and `reftable://…` gives `Reftable`: these are newer Git's URI forms, which the existing tests want and Git 2.50 refuses. Anything else is `Broken`, and the last value counts.
  - `partialclone` needs a value.
  - `worktreeconfig`, `preciousobjects`, `relativeworktrees` and `submodulepathconfig` are booleans.
  - `noop` and `noop-v1` aren't checked.
- After reading:
  - version > 1 gives `FormatVersion`;
  - under version 1, the first unknown extension gives `UnknownExtension`. A subsection makes a different extension: `[extensions "x"] y` is `x.y`, as Git says;
  - under version 0, an extension only version 1 knows is `Broken`, with a message naming `extensions.<name>` and its line. Unknown extensions are ignored, and `partialclone` and `worktreeconfig` still count;
  - then `PartialClone`, then `Reftable` or `RefsElsewhere`.
- `core.sparsecheckout`: every occurrence is read as a boolean, so a bad one is `Broken` even when a later line sets a good one. With no value it is true. `lfs.storage`: the last one counts, empty when it has no value.
- **Includes:** a key named exactly `include.path` with no value or an empty one is `Broken`, because Git fails on it. With a value it is noted. Any `includeIf.<anything>.path` is noted, with or without a value. `[include "x"]`, `[include.x]`, `[includeIf]` with no subsection, and `[include]` with only other keys are not noted, since Git doesn't follow them.
- Errors read `line N of <file>: …`.

### `read_worktree`
Reads only `core.sparsecheckout`, `lfs.storage` and the includes, checked as above. The version and extensions in `config.worktree` are ignored.

### The user's settings
- **`read_user_settings`:**
  - `core.excludesfile` and `include.path` with no value give `Err(line)`; Git refuses a path with no value.
  - `lfs.storage` with no value is empty.
  - `core.sparsecheckout` with no value is `Some(true)`, and a bad value is `SparseCheckout(None)`.
  - A NUL in any of these values gives `Err(line)`.
  - Conditional includes and includes with a subsection are skipped.
- **`read_user_variable`:**
  - The key is split at its first and last dots. A last dot at position 0 or none, or a newline in the subsection, is refused.
  - The section may hold only letters, digits or `-`, and may be empty when there is a subsection.
  - The name must start with a letter and hold only letters, digits or `-`.
  - The section and name are lowercased; the subsection is kept as written.

## Tests (all in `git/config/tests.rs`; the old ones are unchanged)
- `files_are_read_as_git_reads_them`: about 125 hand-written files and 150 generated ones (a fixed xorshift over 29 pieces that often go wrong). Each is compared with `git config --file f --list -z`: the same names and values, or the same line in Git's "bad config line N". `BOLT_GIT_CONFIG_CASES=<n>` sets how many are generated. I ran 3,000, 12,000 and 14,000. The 12,000 run found one difference, whitespace kept before a line continuation at the end; I fixed it, and 14,000 then passed.
- `booleans_are_read_as_git_reads_them`: 52 values against `git config --type=bool`.
- `format_versions_are_read_as_git_reads_them`, `extension_values_are_checked_as_git_checks_them`, `a_nul_ends_names_and_refuses_values`, `includes_git_cant_follow_are_refused`, `only_the_settings_named_are_taken`: expected results taken from the git runs listed below. Each `Refusal` case is in these and in `git/mod.rs`'s existing tests.
- When `git` isn't installed, the comparison tests print a note and pass, as the brief asks. They use `--file`, so the user's and system settings can't affect them.

### Commands and results
- `cargo fmt --all --check`: clean.
- `cargo clippy --workspace --all-targets -j 1 -- -D warnings`: clean.
- `cargo test -p bolt-vcs-core --lib -- --test-threads=1 git::config`: 12 passed (14,000 generated cases: 307 s).
- `cargo test -p bolt-vcs-core --lib git:: config::`: 138 passed.
- First `cargo test --workspace -j 1 -- --test-threads=1` run: one failure, `cli.rs` `a_git_settings_variable_git_refuses_stops_a_save_and_is_named`, which wanted the exact `GIT_CONFIG_COUNT` wording. After the fix, `--test cli` passed 80/80.
- `cargo test --workspace -j 1 --no-fail-fast -- --test-threads=1`: exit 0; 64 test binaries, 1709 passed, 0 failed (this includes `import`, `import_lfs` and `corpus_git`). fmt and clippy were run again after the last edit: clean.

## Files changed
- `crates/bolt-vcs-core/src/git/config.rs` (rewritten)
- `crates/bolt-vcs-core/src/git/config/tests.rs` (new tests appended; allow attributes for panic, indexing and arithmetic in tests)

## Clean-room record
**Files opened:**
- `clean/common-clean-rules.md`, `clean/git-config-brief.md`, `implementer-contract.md`.
- `crates/bolt-vcs-core/src/git/config.rs` (the old re-export only), `git/config/tests.rs`, `fuzz/fuzz_targets/git_config.rs`.
- `crates/bolt-vcs-core/src/git/mod.rs` (lines 25–1000: errors, `Repo::open`, `read_format`, tests), `crates/bolt-vcs-core/src/config.rs` (lines 240–659).
- `crates/bolt-vcs-core/src/lib.rs` (lines 15–30), `git/refs.rs` (lines 1–20, for the lint style), `crates/bolt-vcs/tests/ignore_vs_git.rs` (lines 160–190), `crates/bolt-vcs/tests/cli.rs` (lines 88–133).
- grep hits in `crates/bolt-vcs-core/Cargo.toml`, `git/craft.rs`, `crates/bolt-vcs/tests/{corpus_git,import_lfs,import_vs_git,redteam3}.rs` (config strings only).
- Decision 031 in full; grep lines from decisions 033, 060, 089, 094 and 097.
- Git's documentation:
  - `C:\Program Files\Git\mingw64\share\doc\git-doc\config.adoc` (CONFIGURATION FILE: Syntax, Includes, Values);
  - `git-config.adoc` (ENVIRONMENT);
  - `git-config.html` (the `extensions.*` and `core.repositoryFormatVersion` entries);
  - `gitrepository-layout.html` (the repository format section).

**Never opened:** Git's source, `legacy/`, the history of these files, other projects' tests. No web pages read.

**Commands that taught behaviour**, all in my scratch folder:
- `git config --file f --list -z`, `--get`, `--get-regexp` on about 200 hand-made files (scripts `probe.py`, `c1.txt`–`c7.txt`): headers, keys, values, escapes, CR and CRLF, NUL, BOM, line numbers;
- `git config --type=bool` and `--type=int` on 63 values (`vals.py`);
- `git init` with a made-up `.git/config`, then `git rev-parse --show-object-format` and `git config --get-all core.sparsecheckout` and `lfs.storage` (`repo.py`, `r1.txt`–`r3.txt`), about 115 repositories: versions, the extensions under v0, v1, v2 and none, values, includes, sparseCheckout;
- `GIT_CONFIG_COUNT=1 GIT_CONFIG_KEY_0=<key> git config --list` for `.a.b`, `a..b`, `-.x`, `a.b.c.d`, `A.B.C`;
- the comparison tests themselves.

## Self-review and concerns
- **Where I follow the existing tests over Git 2.50:**
  - a negative version is `Broken`;
  - `refStorage` URI forms are refused as `RefsElsewhere` or `Reftable`, though Git 2.50 calls them invalid;
  - `submodulePathConfig` is known, though Git 2.50 doesn't know it.
- **Choices I made where the brief gives no answer:**
  - A NUL in a value Bolt takes is refused, where Git reads only the part before it. The existing test wanted this for the version, and I apply it to every value Bolt takes, the user's settings files included.
  - An empty `include.path` in the repository is `Broken`. On Windows Git fails trying to read the folder; I haven't checked other systems.
  - `includeIf.*.path` with no value is noted, not refused. Git refuses it only when the condition matches, which Bolt can't evaluate.
  - `partialClone =` (empty) is refused as a partial clone; Git 2.50 opens such a repository.
  - The extensions only version 1 knows include `submodulepathconfig`, which is a guess.
- The comparer may find differences from the old code in these choices, or in error wording (all messages are new).
- No `#[cfg(unix)]` tests added.

---

# Fix round 1 (comparer's task-8 report, seven families)

Commit `f868e2e`, on top of `6d3d407`; the comparer's module and its line in `git/mod.rs` are left as they were. I read only `task-8-report.md`, never `legacy/` or `compare_git_config.rs`. I ran the comparer's tests and its ignored `census` test as commands.

## What changed in `git/config.rs`
1. **`read_count`:** a count that wraps negative, or is past an int or an unsigned long, now gives "is more than Git takes". Text that isn't a count still gives "isn't a number Git reads as a count".
2. **`include.path` with no value or an empty one:** `read_format` and `read_worktree` no longer refuse it; the include is noted. `read_user_settings` is unchanged (`Err(line)` with no value, `Include([])` when empty).
3. **`includes`:** any setting whose section is `include` or `includeif` is noted, whatever its subsection, key or value: `[includeIf] x`, `[include "x"] a`, `[include.x] path`. `[include]` alone, `[includes]` and `[x "include"]` are still not noted.
4. **NUL:** a NUL in a subsection, or anywhere in a value, quoted, escaped or not, makes the line unreadable. That is `Broken` "it can't read line N", or `Err(N)` in the user's settings. N is the header's line, or the line the NUL is on. NUL in a comment is still read; in a key it was already refused. Names are no longer cut short, since no NUL can reach one.
5. **A file that is only part of a byte order mark:** `\xef` or `\xef\xbb` is refused on line 2 when the file or the first line ends right after it (Git does the same; also checked for `\xef\n`), and on line 1 otherwise.
6. **Order of refusals:** after the version and the extensions themselves, `Reftable` and `RefsElsewhere` now come before `PartialClone`.
7. **Wording:**
   - Every per-line problem (syntax, a bad version number, a bad boolean, a bad or missing extension value, a second `compatObjectFormat`, `compatObjectFormat` equal to the hash) gives `it can't read line N of <file>`.
   - A negative version gives `<file> gives the format version -1, which Git never writes`.
   - An extension that needs version 1 gives `<file> sets extensions.<name>, which needs format version 1, but the version is 0`. `<name>` is always one of Git's own fixed names, so no value from the file is quoted in any message any more.

## Tests
- My tests were updated to the rules above. The NUL cases and the NUL piece left the comparisons with git, since Bolt now refuses where Git cuts short. Added:
  - `a_nul_in_a_subsection_or_value_is_refused`;
  - `every_include_setting_is_noted`;
  - `a_byte_order_mark_cut_short_is_refused_where_git_says`;
  - `a_count_is_refused_with_its_reason`;
  - the refs-before-partial-clone order and the exact messages in `extension_values_are_checked_as_git_checks_them`.
- `cargo test -j 1 -p bolt-vcs-core --lib -- --test-threads=1`: `ok. 1066 passed; 0 failed; 4 ignored` (190.97 s). The comparer's four tests now pass.
- Comparer's census (`BOLT_COMPARE_CASES=50000` and `=400000`, debug build): no disagreement groups printed.
- `BOLT_GIT_CONFIG_CASES=3000` comparisons with git: pass.
- `cargo test -p bolt-vcs --test cli --test import --test import_lfs --test corpus_git`: 80, 25, 33 and 29 passed, 0 failed.
- `cargo fmt --all --check` and `cargo clippy --workspace --all-targets -j 1 -- -D warnings`: clean.

## Concerns
- Two cases fall outside the comparer's families and keep my earlier choice: `compatObjectFormat` equal to the repository's hash refuses with "it can't read line N", and so does a second `compatObjectFormat`. I can't tell whether the old reader refused these, and the census found no difference.

---

# Fix round 2 (comparer's commit 51eeb7b: compatObjectFormat equal to the hash)

Commit `f63f027`. `extensions.compatObjectFormat` equal to the repository's own hash is no longer refused: the check after reading is gone. So `compatObjectFormat = sha1` with no `objectformat`, or `sha256` with `objectformat = sha256`, in any order, reads as Ok with that hash, under any version form that reads as 1. Later lines still give their own refusal: `Reftable`, `RefsElsewhere`, `PartialClone`. A bad value or a second `compatObjectFormat` is still "it can't read line N"; the comparer found no difference there. `Seen.compat` is now a bool.

Tests: I took the two equal-hash cases out of the refusal list and added them to the accepted ones. A new assertion checks that `compatObjectFormat = sha1` followed by `refStorage = reftable` gives `Reftable`.
- `cargo test -j 1 -p bolt-vcs-core --lib -- --test-threads=1`: `ok. 1066 passed; 0 failed; 4 ignored` (194.89 s), and `hand_made_texts_agree` passes.
- Census, release, `BOLT_COMPARE_CASES=100000`: `ok`, with no disagreement groups printed (2.49 s).
- fmt and workspace clippy: clean.

---

# Fix round 3 (reviewer: extension list, comments)

Commit `4453b54`, on top of `e619596`.
1. `EXTENSIONS` is now one alphabetical list of Git's extension names. `VERSION_0_EXTENSIONS` lists the four a format version 0 repository takes too: noop, partialclone, preciousobjects and worktreeconfig. The check still walks the extensions in file order and stops at the first unknown one (version 1) or the first that needs version 1 (version 0), so every answer, and which refusal comes first, is unchanged.
2. The comments on `is_c_space`, `parse_int` and `read_count`, and those at the count test in tests.rs, now state the rule instead of naming C functions. For example: decimal, `0x` hex or leading-`0` octal digits up to 64 bits, then an optional `k`, `m` or `g`; and a negative count wraps round in the unsigned long.
3. The file's header now says that a file whose syntax Git refuses is refused, and so is a value Bolt takes that it can't read as Git does. It also says that the values of settings Bolt doesn't take aren't checked, though Git may refuse some of them (`core.bare = maybe`). The behaviour is unchanged.

Results:
- Free memory was 3.1 to 3.5 GB before the runs.
- `cargo test -j 1 -p bolt-vcs-core --lib -- --test-threads=1`: `ok. 1066 passed; 0 failed; 4 ignored`.
- `BOLT_COMPARE_CASES=100000 cargo test -j 1 --release -p bolt-vcs-core --lib legacy::compare_git_config -- --include-ignored --test-threads=1`: `ok. 7 passed; 0 failed` (census included).
- `cargo clippy --workspace --all-targets -j 1 -- -D warnings` and `cargo fmt --all --check`: clean.

---

# RT14-2: NEEDS_CONTEXT (no code changed)

The Git installed here, `git version 2.50.1.windows.1`, does **not** refuse -2^31 as a boolean:
- **`git config --type=bool --file f --get s.k`:** these read as `true`:
  - `-2147483648`, `-2g`, `-2097152k`, `-2048m`, `-0x80000000`, `-2147483647`, `-1g`, `2147483647`.

  These are refused with "bad boolean config value":
  - `-2147483649`, `2147483648`, `2g`.
- **A repository with `core.repositoryformatversion = 1` and `extensions.preciousObjects`** set to each of those values, checked with `git rev-parse --show-object-format`, opens for -2^31 in every form. It refuses only -2147483649, 2147483648 and 2g.
- The same holds for `extensions.worktreeConfig = -2g` and `core.sparseCheckout = -2147483648`.

So on this machine the proof `rt14_2` would fail at its own first assertion ("Git reads it after all"), whatever Bolt does. My `booleans_are_read_as_git_reads_them` test also compares `-2147483648` and `-2g` with this Git, which reads them as true.

Bolt reads booleans as this Git does today. To refuse -2^31 I need to know which Git the finding was made with, and whether Bolt should follow that Git where it differs from 2.50. If so, I would make the range -(2^31-1) to 2^31-1 for booleans only, leaving `core.repositoryformatversion`'s range as it is unless told otherwise. I would also take the -2^31 forms out of the comparison with the installed git, or make them depend on its version.
