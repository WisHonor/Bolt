# v0.1 release: attack surface (red-team session RT14, decision 024)

Bolt v0.1 is about to ship as a closed product. Before the release, the code that followed Git's code was written again from Git's documentation and from what the `git` program does, by people who never saw the old code, then compared with the old code on millions of inputs (decision 131). Every command that walks a save now refuses a planted deep tree of shared listings at once (decision 130). This session attacks those, and a few things no earlier session has attacked.

- **The rewritten readers of what a stranger wrote.** `ignore.rs` (`.gitignore`, `.boltignore`, Git's personal lists: pattern reading and matching, the two verdicts, letter case and Unicode form); `git/config.rs` (`.git/config`, `config.worktree`, the user's own Git settings and `GIT_CONFIG_*`); `names/lookalike.rs` (names Windows or macOS take for `.git` or `.bolt`); `config/git_path.rs` (tidying `GIT_CONFIG_SYSTEM`); `git/person.rs` (a commit's author and committer lines); `git/lfs.rs` (Git LFS pointers). The old versions are gone; the rewrites were made to give exactly the old answers, and the old ones were Git's. Where they differ from Git, the docs say so.
- **The deep tree.** `listing.rs` (`measure`, `Measure::add_folder`, `SaveLimits`), and its callers: `diff.rs` (the two-save walk), `snapshot/walk.rs` (the scan against the last save, folders gone from disk, the read-ahead thread), `snapshot/only.rs` (partial saves), `verify.rs` (`too_big`). A walk keeps the listings it has entered; one met again is measured and counted whole, then walked without counting.
- **The gate** (`bench/run.sh gate`) isn't attack surface; leave it.

Threat list: `docs/security.md` §4. Decisions 128 to 132 in `docs/decisions/` give the rules; [docs/releases/v0.1.0-code-audit.md](../docs/releases/v0.1.0-code-audit.md) says what was rewritten.

Your goals:
- **A verdict changed:** an ignore file, a Git config file, a ref name, a person line or a pointer that the rewritten reader reads differently from Git (where the docs don't say Bolt differs on purpose), or that makes Bolt take a file Git's rules keep out, or keep out one they take.
- **A repository swallowed:** a name the rewritten look-alike checks miss that Windows or macOS opens as `.git` or `.bolt`.
- **The deep tree, again:** a planted save that still makes any command walk for long, hold much memory, or be refused only after work in proportion to the paths; or an honest save (one within the format's limits, shared folders included) that a command now refuses or answers differently.
- **Resources:** a hostile ignore file, config file, pointer or name that costs time or memory out of proportion to its size (the bracket parser was quadratic once and is linear now: check).
- **The two halves of milestone 6 together** (never attacked as one): preset rules (`bolt setup`, `bolt.toml`, the preset block of `.boltignore`) against the walk's late ignore verdict (decision 107: a saved file is matched only when it changed) and its listing shortcut (a folder whose entries match the last save takes its listing); `bolt verify` against the three caches (`cache/files`, `cache/change-ids`, `cache/other-lines`) planted, stale or torn; `--json` on hostile names (control characters, invalid UTF-8 shown, very long names, names that look like JSON).
- **The speed session's second-round fixes:** the pack reader's block budget (it reads a block only while the blocks read come to at most four times what was asked, decision 112) and `cache/other-lines` (the third sealed cache, read only by the check before an amend).
- **A release build,** not a debug build: build with `cargo build --release` and attack that binary (overflow checks are off there; timing differs).

## Where the code is

- **Ignore rules:** `crates/bolt-vcs-core/src/ignore.rs` (`wildmatch`, `Rules::parse`, the bracket parser and its bitsets, the plain-name index and the one-star fast path, `Matcher`, `InForce`); its caller `snapshot/walk.rs` (`judged`, the late verdict) and `snapshot.rs` (`build_with`); `scope.rs` (command-line patterns of a partial save).
- **Git's config:** `git/config.rs` (`read_format`, `read_worktree`, `read_user_settings`, `read_user_variable`, `read_bool`, `read_count`); callers `git/mod.rs` (`Repo::open`) and `config.rs` (the user's settings, includes, `GIT_CONFIG_*`).
- **Names:** `names/lookalike.rs` (`is_git_alias`, `is_bolt_alias`, the short-name checks), called from `names.rs` (`validate_name`, `writable_here`), the walk, write-out and the import.
- **Paths and people and pointers:** `config/git_path.rs`, `git/person.rs`, `git/lfs.rs` (and `git/object.rs`'s look-alike pointer checks around it).
- **The deep tree:** listed above; tests in `crates/bolt-vcs/tests/deep_tree.rs`.
- **Milestone 6's halves:** `setup.rs`, `presets.rs`, `rules.rs`; `verify.rs`; `filecache.rs`, `changeids.rs`, `otherlines.rs`; `crates/bolt-vcs/src/output.rs` and `*_output.rs` for `--json`; `pack.rs` (`Window`, the block budget).

## Places worth attacking

- Patterns at the edges `gitignore(5)` describes and `git check-ignore` decides: brackets with `]` first, `!`/`^`, ranges backwards and across letter case, unknown classes, escapes everywhere, `**` glued to text and repeated, trailing spaces with backslashes, NUL, CR, a byte order mark, invalid UTF-8, letter case on a drive that ignores it, NFD names on a drive that composes. Compare each with `git check-ignore` inside the sandbox.
- Config files `git config --list` reads one way and Bolt another: quoting, escapes, continuations, comments in values, subsections, booleans and numbers with units; extensions in any order and combination; `include`/`includeIf` forms; `GIT_CONFIG_COUNT` edge values.
- Look-alikes: every spelling of `.git` and `.bolt` with ignorable characters, streams, trailing dots and spaces, short names (`git~1` … `git~9`, hashed forms), mixed case, separators.
- Deep trees: chains, diamonds, two saves both planted (a `goto` from one to the other), a deep tree under a folder deleted from disk (`status`, `save`), in a partial save's scope, in a shelf, in the target of an undo or redo; honest projects with many identical folders (empty folders, folders holding one identical file).
- Windows and macOS on `redteam/v01-*` CI branches (attack code runs only there and in the sandbox): the items the follow-ups list as not attacked for the speed work — on Windows, the short names a listing shows and a drive that folds letter case; on macOS, a drive that folds case and composes Unicode.

## Limits already known (not findings unless you show more)

- The follow-ups list's "Known limit" and "Known cost" rows, including this session's own section ("Release v0.1: clean code and the gate").
- Bolt reads some Git config values Git refuses (`core.bare = maybe`, a valueless `core.worktree`), refuses a negative format version, and refuses a repeated `ext-` key in a pointer where Git LFS takes the last: all as before, on purpose.
- The platform crate is unchanged since milestone 6's sessions attacked it.
- `verify`'s memory grows with the number of objects (decision 127).
