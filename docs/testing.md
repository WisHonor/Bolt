# Bolt Test Plan — Draft

> **Status:** drafted in the design phase (2026-10-31); §3 and §4 describe the tests and CI as they run after milestone 1 (2026-11-02), with milestone 2's differential tests, ignore-rule tests, hostile Git repositories and fuzz targets (2026-11-03).

**In plain words.** Bolt's first promise is "never lose or corrupt your data". Tests are how that promise survives every change to the code. Each kind of test below proves something specific, and together they run on Windows, macOS and Linux every time code is pushed.

## 1. Rules

- A task is done only when its tests pass on all three systems.
- Every on-disk format has test vectors ([storage-format.md](storage-format.md) §13) before it holds real data.
- Every bug fix comes with a test that fails without the fix.
- Speed and size claims come only from the benchmarks in §5.

## 2. Kinds of tests

| Kind | What it proves | Tools |
|---|---|---|
| Unit tests | Each small piece does what it says | Rust's built-in tests |
| Format tests | Every record encodes to exactly the bytes in the test vectors and decodes back; every invalid variant is rejected | golden files in `crates/bolt-vcs-core/tests/vectors/` |
| Round-trip tests | Saving and then restoring gives byte-identical files, names, executable flags, symlinks and empty folders | `assert_fs` |
| Property tests | Random sequences of edits and commands never lose data and always match a simple model of what should happen | `proptest` |
| Crash tests | Stopping Bolt at any point while it writes leaves the repository usable, and the next command recovers | fault injection, killed processes |
| Differential tests | Importing a real Git repository gives exactly the same files, for every commit, as Git itself; Bolt's ignore rules keep the same files as Git's | `git` as the reference |
| Fuzz tests | Decoders and parsers never crash, hang or over-allocate on garbage input | `cargo-fuzz` |
| Malicious-repository corpus | Each attack in [security.md](security.md) is refused or handled safely | crafted repositories |
| Red-team sessions | A determined attacker can't break what the other tests didn't think of; once per milestone ([decision 024](decisions/024-red-team-gate.md)) | a fresh agent in a no-network Docker sandbox |
| CLI tests | Commands print what they should, both for people and as `--json` | `assert_cmd`, `insta` snapshots |
| Benchmarks | Speed and size claims are true, and stay true | `criterion`, `hyperfine` |

## 3. Details

**Property tests.** An in-memory model tracks what the folder and history should contain. Each random action (create, change, move or delete files; save; goto; discard; undo; redo) is applied to both Bolt and the model. After every action: `status` matches the model, every save restores byte-identically, `undo` returns to the previous state, and nothing the model says exists is missing.

**Crash tests.** Every disk write goes through one small layer that tests can make fail (return an error, or abort the process) at the N-th operation, or fail at one named point (`BOLT_FAULT_FAIL=start:read-back`) and let the command carry on. For each N, a test runs a command, fails it at step N, then checks that the repository opens, `bolt verify` passes, no saved data is missing, and the next command finishes or rolls back the interrupted one. Separately, a loop kills real Bolt processes at random moments.

A command can now write several packs, so milestone 2 added crash tests for the gap between two of them. A named point, `packs:rolled-over`, sits right after a full pack goes into the store.
- `a_crash_between_packs_leaves_the_repository_as_it_was` (in `crates/bolt-vcs-core/src/repo.rs`) fails a command that needs three packs at every step. It checks that `state/head` still names the old entry, that the store checks out, and that running the command again works.
- `crates/bolt-vcs-core/tests/import_interrupted.rs` does the same to `bolt import`, with a test-only limit of three objects a pack. After every stop there is no project yet (`.bolt/format` is missing), and running the import again gives the saves and state of a clean import. At least one stop must fall between two packs.

Beside them, `crates/bolt-vcs-core/tests/pack_limit.rs` is not a crash test: it checks the test-only limit itself, that it splits a command's packs until it is cleared. The limit applies to every command in the process, so that file and `import_interrupted.rs` each hold a single test.

**Differential tests against Git.** `crates/bolt-vcs/tests/import_vs_git.rs` makes repositories with Git itself, runs `bolt import`, and then `assert_matches_git` (in `tests/support/`) compares the project with what Git holds, reading Git's objects through `git cat-file --batch`:

- each commit `git rev-list --all` lists has exactly one save, which rebuilds the commit's exact bytes, and commits Git writes itself rebuild from their saves' own fields, without their Git header;
- each author and committer line in Git's own form names the save's person, time and zone, so a kept Git header can't hide a wrong one;
- each Git tree, read once, is a listing with the same names after Bolt's skip rules, the kinds its modes make, and files whose Git blob IDs, computed in the test, are the entries' IDs;
- the lines, server lines and tags are the branches, server branches and tags `git for-each-ref` lists, under the names Bolt's renaming rule gives them (the everyday test also checks a few names as written, so a wrong rule can't pass on both sides), and annotated tags keep their tag objects' exact bytes.

Git runs with nothing from the computer's set-up: no system or user config, no templates, no replace objects.

On every push, on all three systems, it compares six repositories: everyday history (renames, deletions, an executable, links, case twins, a submodule, Unicode names, a merge and an octopus merge, every kind of branch, server branch and tag, then the same with a detached `HEAD`); unusual commits written byte by byte (signatures, a merge tag, Latin-1, a change ID, `-0000`, an odd author line, an unknown header, a parent named twice); packed and loose objects with deltas; a SHA-256 repository; Git's optional files (commit-graph, multi-pack index, bitmap, reverse index, worktree config, only packed refs); and names Windows can't hold, saved again after the import. One more test checks that a link Git wrote keeps its target when saved, on all three systems: Git for Windows writes the `/` in a link's target as `\`, and Bolt stores `/` there, as Git does ([decision 045](decisions/045-link-targets-stored-with-slashes.md)). In CI the test fails rather than skip if Git can't make the link. Before milestone 3 built this, a nightly job ran the test on GitHub's Windows runner as the evidence the decision rests on: in [run 41644016857](https://github.com/WisHonor/Bolt/actions/runs/41644016857) it failed as it should, saving `dir\target.txt` where Git stores `dir/target.txt`.

Nightly and when started by hand (§4), `a_real_repository_matches_git` imports a full clone of Git's own repository (git/git; it checks the clone isn't shallow) and compares all of it. It imports in its own process and prints how long the import took and the process's peak memory, which Linux reports as `VmHWM`, read right after the import.

Measured on 2026-11-03 in release builds, on git/git as of 2026-11-01: 85,977 commits and 1,012 tags (Bolt leaves out the one that tags a file). Every commit, tree, file and ref matched Git; 98 commits have trees in forms Git no longer writes, and 219 keep their Git header.

- GitHub's Linux runner (`ubuntu-latest`, nightly run 41638396168): imported in 102.0 s, with a peak of 583 MiB (`VmHWM`).
- A Windows 11 laptop (AMD Ryzen 7 7730U with 8 cores, 16 GB of RAM, an NVMe SSD) with Git 2.50.1: imported in 165 s, with a peak working set of 503 MiB; the comparison took 124 s more. Windows tells a process its peak memory only through a system call, so there the test prints none: this figure is the test process's `PeakWorkingSet64`, read by PowerShell twice a second and taken as the import ended (the comparison never raised it).

**Ignore rules against Git.** Bolt's ignore matcher is a port of Git's own, and three kinds of test hold it to Git:
- Every row of Git's wildmatch table (`t/t3070-wildmatch.sh`, 190 rows) passes, with and without letter case folded. A port of Git's recursive matcher, kept in the tests, must agree with Bolt's loop on 20,000 random patterns and texts. A count of the steps the matcher takes (not a clock) shows that its work stays within the pattern's length times the text's.
- `crates/bolt-vcs/tests/ignore_vs_git.rs` writes random rule files in random projects (as `.gitignore` for Git and `.boltignore` for Bolt, in several folders, with names in both letter cases and in composed and decomposed Unicode). For 256 projects each, it checks that a save keeps exactly the files `git ls-files --others --exclude-standard` lists, and that the verdict on every path is the one `git check-ignore --no-index -v` gives. A third check lays a `.boltignore` of mostly `!` lines beside the `.gitignore` files and shows Git only the `.gitignore` files: Bolt's verdict from the `.gitignore` lines alone, which the warning about files a `.boltignore` takes back rests on, must still be Git's. Git is told what the drive is like (letter case, Unicode form) and kept from the user's own ignore list.
- The same file holds the cases of Git's own `t/t0008-ignores.sh` that need only a project's own ignore files, and a test of Git's two personal lists ([decision 047](decisions/047-git-personal-ignore-lists-read.md)): a user's own list, named to Git as `core.excludesFile`, and `.git/info/exclude`, with project rules that take back some of their lines, must leave a save exactly the files Git lists. The CLI tests point `HOME` and `XDG_CONFIG_HOME` at a folder of their own, so the real user's Git settings never reach them.

Git before 2.52 reads a `**` right after the literal start of a pattern with a `/` differently, and Bolt follows 2.52 and later; with an older Git such lines are left out of the random rules. The CI runners' Git is new enough that nothing is left out.

**Fuzz targets.** Fifteen cover the decoders and parsers of data from outside Bolt:
- Bolt's records: `decode_listing` (folder listings), `decode_save`, `decode_diary` (diary entries), `decode_tag_list` and `decode_server_lines`.
- Bolt's files: `open_pack` (a pack, with its index, trailer and every entry), `format_file` (`.bolt/format`) and `name_rules` (a list of names given to `validate_name`, `is_git_alias` and `folder_problems`: the always-on name rules, the `.git` spellings and the clashes among one folder's names).
- `ignore_rules`: ignore files, the matcher and wildmatch, on drives that do and don't ignore letter case or compose Unicode.
- The Git reader: `git_loose` (loose objects, as they are and compressed, to get past zlib), `git_pack` (a pack and its index, given the checksums no fuzzer finds by chance), `git_delta`, `git_objects` (commits, trees, tags, people, and the tests for Git LFS pointers), `git_refs` (`packed-refs` and `HEAD`, then the renaming of branch and tag names) and `git_config`.

The decoder for project links ([storage-format.md](storage-format.md) §7) has test vectors but no target yet. The decoders for chunk lists, conflict records, `state/journal` and `bolt.toml` get theirs in the milestones that build them.

**Malicious-repository corpus.** At least one case per threat in [security.md](security.md) §4: path tricks, `.bolt` spellings, symlink escapes, case and Unicode collisions, device names, bombs, non-canonical encodings, wrong fingerprints, truncated packs and corrupted indexes. The cases from the review of Git's published vulnerabilities (2026-11-02, security.md §6) are in `crates/bolt-vcs/tests/corpus_git.rs`: each is a Git repository written byte by byte with `Craft`, which only Bolt reads, run through `bolt import` (in the test's own process where a sweep imports a thousand times), and each test checks the exact refusal, or what was left out and how it was listed, and that nothing was committed when the import stopped.

**Big zlib streams.** Bolt inflates Git's objects with miniz_oxide, 16 KiB of input a call. Given no input, miniz_oxide can still write the rest of a copy and then say it made no progress, which once cost big, mostly-zero files their last bytes. A unit test in `git/odb.rs` inflates the smallest stream found to do that, 52 MiB of zeros, on every push. `big_mostly_zero_streams_inflate_whole` sweeps zeros and sparse data of 110 to 300 MiB at three compression levels, checking each byte against the data compressed; it takes a few minutes in a release build, so it is ignored in normal runs and runs nightly (§4).

**Red-team proof tests.** Every proven red-team finding keeps its proof as a permanent test that runs on every push: `crates/bolt-vcs/tests/redteam.rs` (session 1, 18 tests), `crates/bolt-vcs/tests/redteam2.rs` (session 2 and the final review of its fixes, 12 tests), `crates/bolt-vcs/tests/redteam3.rs` (milestone 2's session and the review of its fixes, 12 tests) and `crates/bolt-vcs/tests/redteam4.rs` (milestone 2's second session, on the fixes, 4 tests). Each runs on the systems its attack needs; six of milestone 2's run on Linux only: five measure peak memory through `/proc`, and one of RT3-5's needs a drive where letter case counts. Ignored are a helper that plants files for session 2's attack scripts, RT3-7's proof, which asserts a refusal where decision 043 ruled for a warning, and RT4-1's proof, which asserts a bound on the disk an import takes where decision 044 accepted whole records until chunked storage.

**Windows-specific tests.** A helper process holds a file open, to test retries and the "file in use by" message; reserved names; long paths; read-only files; the hidden `.bolt/` folder; junctions inside and around `.bolt/`; a project nested as deep as a path allows, 2,048 folders whose last path is 4,095 bytes (§4).

## 4. CI on GitHub

- **Every push and pull request:** formatting check, clippy with warnings denied, every test on Windows, macOS and Linux (the differential tests against Git on small repositories among them), run to the end even when one fails (`--no-fail-fast`), so one failing test file can't hide the others, a build without features, and `cargo-deny`, on the workspace and on the fuzz crate (it also checks the RustSec advisory database and that every crate comes from crates.io). On Windows, one more step runs the slow tests that save a project nested 2,048 folders deep (a last path of 4,095 bytes) and refuse one a level deeper (4,097 bytes); they are marked ignored, so only that step runs them.
- **Nightly, on every push to `main`, and when started by hand:** fuzzing of every target (§3), 5 minutes each nightly and 1 minute each otherwise.
- **Nightly, and when started by hand (not on pushes):** the differential test on a fresh full clone of git/git, on Linux in a release build, within an hour; its log must show the one test passed, and shows the import's time and peak memory (§3). The sweep of big zlib streams runs beside it, also in a release build (§3). Benchmarks join, with results kept for comparison, once they exist.
- The repository is public, so Actions minutes aren't a constraint ([design.md](design.md) §7).
- A `.gitattributes` file turns off Git's line-ending conversion, so test files stay byte-exact on Windows.
- Workflows run with read-only permissions, actions are pinned to exact commits, and Dependabot opens weekly update pull requests for Rust crates and actions.

## 5. Benchmarks and measurements

- **Micro:** speeds of hashing, chunking, compression, encoding and decoding (`criterion`).
- **End to end:** `bolt` against Git and Git LFS on fixed test projects (a code project, a Unity project, folders of large binary files): first save, saving a small change, `status` on an unchanged project, `status` after touching one big file, restoring an old version (`hyperfine`). Results are published together with the machine they ran on.
- **Release gates for v0.1** ([decision 023](decisions/023-v0.1-speed-bar.md)): saving a 10 GB file uses at most about 500 MB of memory, and `status` on an unchanged 100,000-file project takes under 2 seconds on an SSD. Both run as automated benchmarks; everything else is measured and published, not gated.
- **Dedup:** how much space chunking really saves on real assets (textures, models, audio, video, engine files), so claims stay honest (design.md risk 4).

## 6. Test data

Small fixtures live in the repository. Large corpora (open-source Unity, Unreal and Godot projects; Git repositories) are downloaded by scripts and never committed. The owner's own code projects are used for everyday testing.
