# Bolt Test Plan — Draft

> **Status:** drafted in the design phase (2026-10-31); §3 and §4 describe the tests and CI as they run after milestone 1 (2026-11-02).

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
| Differential tests | Importing a real Git repository gives exactly the same files, for every commit, as Git itself | `git` as the reference |
| Fuzz tests | Decoders and parsers never crash, hang or over-allocate on garbage input | `cargo-fuzz` |
| Malicious-repository corpus | Each attack in [security.md](security.md) is refused or handled safely | crafted repositories |
| Red-team sessions | A determined attacker can't break what the other tests didn't think of; once per milestone ([decision 024](decisions/024-red-team-gate.md)) | a fresh agent in a no-network Docker sandbox |
| CLI tests | Commands print what they should, both for people and as `--json` | `assert_cmd`, `insta` snapshots |
| Benchmarks | Speed and size claims are true, and stay true | `criterion`, `hyperfine` |

## 3. Details

**Property tests.** An in-memory model tracks what the folder and history should contain. Each random action (create, change, move or delete files; save; goto; discard; undo; redo) is applied to both Bolt and the model. After every action: `status` matches the model, every save restores byte-identically, `undo` returns to the previous state, and nothing the model says exists is missing.

**Crash tests.** Every disk write goes through one small layer that tests can make fail (return an error, or abort the process) at the N-th operation. For each N, a test runs a command, fails it at step N, then checks that the repository opens, `bolt verify` passes, no saved data is missing, and the next command finishes or rolls back the interrupted one. Separately, a loop kills real Bolt processes at random moments.

**Differential tests against Git.** Start with small repositories built by scripts, then real ones: a medium project such as Git's own repository, then larger ones. Bolt imports the repository, and for every commit the files Bolt restores are compared byte for byte with what `git` checks out. Larger repositories run nightly, not on every push.

**Fuzz targets.** The decoders for folder listings, saves, chunk lists, conflict records and diary entries; pack indexes and trailers; `.bolt/format`; `state/journal`; `bolt.toml`; `.boltignore`; name and path validation; the Git import reader.

**Malicious-repository corpus.** At least one case per threat in [security.md](security.md) §4: path tricks, `.bolt` spellings, symlink escapes, case and Unicode collisions, device names, bombs, non-canonical encodings, wrong fingerprints, truncated packs and corrupted indexes. Cases inspired by Git's published vulnerabilities are added as they are reviewed.

**Red-team proof tests.** Every proven red-team finding keeps its proof as a permanent test that runs on every push: `crates/bolt-vcs/tests/redteam.rs` (session 1, 18 tests) and `crates/bolt-vcs/tests/redteam2.rs` (session 2 and the final review of its fixes, 12 tests). Each runs on the systems its attack needs; the only ignored test is a helper that plants files for the session's attack scripts.

**Windows-specific tests.** A helper process holds a file open, to test retries and the "file in use by" message; reserved names; long paths; read-only files; the hidden `.bolt/` folder; junctions inside and around `.bolt/`; a project nested 4,096 folders deep (§4).

## 4. CI on GitHub

- **Every push and pull request:** formatting check, clippy with warnings denied, every test on Windows, macOS and Linux, a build without features, and `cargo-deny` (which also checks the RustSec advisory database and that every crate comes from crates.io). On Windows, one more step runs the slow tests that save a project nested 4,096 folders deep and refuse one at 4,097; they are marked ignored, so only that step runs them.
- **Nightly, and on every push to `main`:** fuzzing of every target, 5 minutes each nightly and 1 minute on pushes. Differential tests on larger repositories join with milestone 2, and benchmarks, with results kept for comparison, once they exist.
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
