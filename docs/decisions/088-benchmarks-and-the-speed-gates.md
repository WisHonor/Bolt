# 088 — Benchmarks, the speed gates, and compressing again

- **Status:** Accepted
- **Date:** 2026-11-08
- **Agenda topic:** none (milestone 5 planning)
- **Decided by:** Claude, at the owner's request while the owner was away. The owner can reverse any part.
- **Refines:** [decision 023](023-v0.1-speed-bar.md) (how the two gates are measured and the results published); decisions [009](009-storage-engine.md), [057](057-pack-index-on-disk-and-merging.md) and [077](077-tidy-and-the-undo-window.md) (compressing again at a stronger level waits for these numbers)

## Context

Decision 023 sets two gates for v0.1 (saving a 10 GB file uses at most about 500 MB of memory; `status` on an unchanged 100,000-file project takes under 2 seconds on an SSD) and asks for benchmark results against Git and Git LFS, published either way; Bolt needn't beat them. testing.md §5 lists what to measure, but nothing beyond the status gate exists yet: no harness, no data, no published numbers. Decisions 009, 057 and 077 also leave compressing again, at a stronger level when packs are merged or rewritten, to these benchmarks. Claims must be reproducible (principle 4).

## Options

- **A. Scripts and generated data, run in CI and on the owner's laptop.** Shell scripts drive `bolt`, `git` and `git lfs` through the same steps, timed with `hyperfine`; the data is generated from fixed seeds or taken from public repositories at fixed tags; a manual workflow runs them on GitHub's Linux and Windows machines, and the results go into a document with the machine and versions.
- **B. A Rust benchmark harness** (`criterion` for the parts, a runner for the whole). Finer statistics, but a large new dependency tree, and the end-to-end steps would still shell out to `git`.
- **C. Only the two gates.** Cheapest, but decision 023 asks for the comparison.

## Decision

**A.**

- **The memory gate:** a test, ignored and run by a nightly job on Linux in a release build, writes 10 GiB of pseudo-random bytes (every chunk different, nothing compresses) into a temporary folder, runs `bolt save` on it, samples the process's peak memory (`VmHWM`) while it runs, and fails above 500 MB (10^6 bytes each). The job frees disk on the runner first; the files are deleted afterwards. It also runs once on the owner's Windows laptop through the command, its peak working set sampled from outside, and both results are published.
- **The status gate:** the existing timing of `status` on 100,000 unchanged files (testing.md §5), run again after this milestone's changes, nightly and on the laptop, and published.
- **Against Git and Git LFS:** scripts in `bench/` build each data set in a temporary folder and time each step for Bolt, Git, and Git with Git LFS tracking the big files: the first save or commit; saving a small change; `status` on nothing changed; `status` after touching one big file; going back to an old version; and the size on disk afterwards (`.bolt` against `.git`, Git LFS's objects included). Data sets: a real code project at a fixed tag; a folder of big binary files; one big file in 10 versions, each with a small edit and a small insertion; Godot's demo projects at a fixed tag, as a real game project; and an import of a Git LFS repository built by the script. Each step runs 5 times after a warm-up (3 for steps over a minute), with the mean, minimum and maximum shown; caches are warm, and the document says so.
- **Where:** a `bench` workflow started by hand runs the scripts on GitHub's Linux and Windows machines, with the `git-lfs` they come with; results go into `docs/benchmarks.md` with the machine, the versions of Bolt, Git and Git LFS, and the exact commands, whatever they show.
- **Parts:** the speeds of the cutter, BLAKE3 and zstd (levels 3, 9 and 19, on chunk-sized pieces) come from ignored tests that print MB/s; no `criterion`.
- **Compressing again:** the same tests measure, on the code set, on uncompressed assets and on compressed ones (from the Godot demos), the bytes zstd's levels 9 and 19 save against level 3, and the time they take. Merging and `tidy` compress again at a stronger level only if one saves at least 10% of the stored bytes of the code and uncompressed sets at no more than 5 times level 3's time; otherwise they go on copying stored bytes as they are. Compressing again would only ever touch content: a chunk list stays uncompressed, as storage-format.md §6 requires. The outcome is ruled on with the numbers, and published.
- **Also measured** (decision 087): the red team's RT4-1 repository and a 200 MiB file in 50 versions, each importing into Bolt, sizes against Git's.

## Consequences

- `bench/`, a `bench` workflow, a nightly job for the memory gate and `docs/benchmarks.md` are new; `hyperfine` is installed on the runners by the workflow, never on the owner's laptop.
- Every speed or size claim Bolt makes comes from `docs/benchmarks.md`, including the ones where Git or Git LFS wins.
- Numbers from GitHub's shared machines vary from run to run; the document says how much, and the laptop's numbers sit beside them.
