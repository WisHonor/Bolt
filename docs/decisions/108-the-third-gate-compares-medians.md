# 108 — The third speed gate compares medians

- **Status:** Accepted
- **Date:** 2026-11-10
- **Agenda topic:** none (milestone 6, speed and space)
- **Decided by:** Claude, at the owner's request while the owner was away. The owner can reverse any part.
- **Refines:** [decision 099](099-a-third-speed-gate-within-twice-gits-time.md) (how the third gate's times are compared)

## Context

Decision 099 times ten runs of each step and compares Bolt's mean with Git's. While the speed work was measured, gate run after gate run, the mean turned out to measure something else on GitHub's Linux runner.

Bolt flushes what a command's result rests on to the disk: once in a `status` (the file it reads the drive's clock from), and ten times in a `goto` that writes one file (the clock's file, the diary's pack and its folder, the journal and its folder, the file and its folder, `head` and its folder, the folder again once the journal is removed). Git flushes nothing in these three steps. On the runner's disk a flush usually takes 0.2 to 0.5 ms, and all ten together 3.6 ms. But now and then one takes 60 ms or more:

| Gate run | Bolt's ten `goto` runs | Their mean | Git's ten runs |
|---|---|---|---|
| 42796523139 | 36.3 to 38.3 ms | 37.4 ms | 14.4 to 15.0 ms |
| 42793107729 | 29.5 to 180.2 ms | 99.2 ms | 9.7 to 11.4 ms |
| 42801136909 | 27.9 to 94.5 ms | 43.5 ms | 13.7 to 14.4 ms |

Two of three consecutive runs had such stalls, in `goto` and in the small save alike; the first profile of this milestone saw the same in `status` (one flush of 50 ms and one of 134 ms in seven runs). With them, the mean of ten runs says the same code took 37 ms on one machine and 99 ms on the next. A gate on that number passes or fails by the disk the job happened to get, and a gate that a re-run turns green teaches people to re-run it.

The stalls are real, and a person whose disk flushes slowly waits for Bolt's flushes where Git makes none. That is a cost of Bolt's promise about crashes, and it has to be published. But it is not what the gate is for: the gate asks whether Bolt's own work on an everyday project stays within twice Git's.

## Options

- **A. Keep the mean.** Simple, and on a quiet disk it equals the median. On GitHub's Linux runner it fails or passes by the disk.
- **B. Compare medians.** Half of each tool's runs are at or below its median, so up to four stalled runs in ten don't move it. Print each tool's mean, best and worst run beside it, so a stall shows as a mean far above the median.
- **C. Compare best runs.** Hides every cost that isn't there in the luckiest run. Too kind.
- **D. Flush less.** It would change what a crash can leave behind, which is the owner's to decide, not a measuring question.

## Decision

**B.**

- **The gate compares the medians** of each step's ten timed runs: Bolt's median may be at most twice Git's, on each of the two systems. Everything else in decision 099 stands: the steps, the data, the runners, the untimed preparation, ten runs after a warm-up, the job that fails.
- **Beside each ratio the job prints** both tools' mean, best run and worst run. [benchmarks.md](../benchmarks.md) publishes the same columns.
- **What a flush costs is published too:** how many times each of the three steps flushes, what a flush takes on a quiet disk, and what the stalled runs looked like.

## Consequences

- A run on a machine whose disk stalls now gives the same verdict as one on a quiet machine, as long as fewer than half of a step's runs stall. If more do, the gate still fails, and the printed best and worst runs say why.
- The published means can be worse than the gate's medians, and benchmarks.md says so where it shows them.
- `bench/run.sh gate`, `bench/README.md`, [benchmarks.md](../benchmarks.md) §1, [testing.md](../testing.md) §5 and decision 099 (a back-link line) follow.
- Flushing less in a `goto` stays open, as a question for the owner: [the follow-ups list](../plans/2026-11-01-milestone-1-follow-ups.md) has the row.
