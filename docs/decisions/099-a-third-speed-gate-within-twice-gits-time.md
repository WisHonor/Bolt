# 099 — A third speed gate: within twice Git's time on a code project

- **Status:** Accepted
- **Date:** 2026-11-10
- **Agenda topic:** none (milestone 6 planning, speed and space)
- **Decided by:** Claude, at the owner's request while the owner was away. The owner can reverse any part.
- **Refines:** [decision 023](023-v0.1-speed-bar.md) (a third gate beside its two) and [decision 088](088-benchmarks-and-the-speed-gates.md) (how it is measured and enforced)
- **Refined by:** [decision 108](108-the-third-gate-compares-medians.md) (the gate compares the medians of each step's runs, not the means: a flush that stalls on the runner's disk no longer decides it)

## Context

Decision 023 gives v0.1 two gates: memory when saving a 10 GB file, and `status` on 100,000 unchanged files in under 2 seconds. Neither compares Bolt with Git. The benchmarks do ([benchmarks.md](../benchmarks.md) §2, 2026-11-09), and on a small code project Git wins every everyday step. On Git's own source (about 4,900 files), on GitHub's Linux runner:

- `status` with nothing changed takes Bolt 62 ms against Git's 13;
- a small change, saved, takes 69 ms against 30;
- going back to the first version takes 73 ms against 15.

On the Windows runner the same steps take 135 ms against 45, 149 against 127, and 152 against 55. A project of 20 files takes Bolt 3 to 4 ms, so the time grows with the files, not with start-up; the profile in [benchmarks.md](../benchmarks.md) §7 says where it goes. The owner asked for a gate that holds Bolt to Git on these three steps (2026-11-09).

## Options

- **A. A ratio against Git, measured in the same job.** Bolt may take at most twice Git's time, each step compared with Git's on the same machine in the same run. The ratio holds whatever machine GitHub hands out.
- **B. A time in milliseconds for each step.** Simple to state, but GitHub's machines differ by up to 2 times from run to run ([benchmarks.md](../benchmarks.md) §6), so the machine would decide.
- **C. Publish the numbers, with no gate,** which is decision 023's rule for everything but its two gates. The owner asked for a gate.

## Decision

**A.**

- **The gate.** On Git's source at tag `v2.56.0` (the code set of benchmarks.md §2), each of three steps takes Bolt at most twice the time it takes Git:
  - `status` with nothing changed (`bolt status` against `git status`);
  - a small change, saved (`bolt save` against `git add -A && git commit`);
  - going back to the first version (`bolt goto` against `git checkout`).
- **Where.** On GitHub's Linux runner and on its Windows runner. Each system is its own check, Bolt and Git in one job on one machine.
- **Measured as benchmarks.md §2 measures,** with the same commands, the same untimed preparation and waits, `hyperfine`, and warm caches. The means are compared. It times 10 runs of each step where §2 times 5: these steps take tens of milliseconds, and a mean of 5 moves more.
- **Enforced.** `bench/run.sh gate` runs the code set alone, for Bolt and for Git, prints the three ratios and fails when one is over 2. A `speed-gate` job runs it on both systems, in the `Bench` workflow and nightly. Unlike decision 023's status time, which is printed and read, this one fails its job.
- **Git's version** is the runner's own (2.56.0 today). The published results name it.
- **v0.1 ships** when all three gates pass.

## Consequences

- Before any change the gate fails five of its six checks: only the small save on Windows passes, at 1.2 times Git's time. Milestone 6's "speed and space" work is what it takes to pass.
- The gate follows Git: when a later Git is faster, Bolt must be too.
- Only GitHub's runners are gated. On a computer whose virus scanner checks every file as it is opened, such as the owner's laptop, Bolt's own look at each file costs more than there ([benchmarks.md](../benchmarks.md) §7), so its ratio is worse; those numbers are published, not gated.
- [benchmarks.md](../benchmarks.md) §1 lists three gates, [testing.md](../testing.md) §4 and §5 describe the job and the gate, and [milestones.md](../milestones.md) says v0.1 ships when all three pass.
