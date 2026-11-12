# 129 — `goto`'s limit until v0.2 is three times Git's time, and the status gate fails its job

- **Status:** Accepted
- **Date:** 2026-11-11
- **Agenda topic:** none (getting v0.1 ready: clean code and the gate)
- **Decided by:** Claude, at the owner's request while the owner was away. The owner can reverse any part.
- **Refines:** [decision 128](128-v0.1-ships-without-goto-within-twice-gits-time.md) (the limit it left to be picked from the runs on record); [decision 023](023-v0.1-speed-bar.md) (its status gate, printed and read until v0.1)
- **Partly superseded by:** [decision 132](132-gotos-limit-is-three-and-a-half-times-gits-time.md) (the limit is 3.5 times Git's time, after a run on unchanged code passed 3)

## Context

Decision 128 lets v0.1 ship with `goto` over twice Git's time, and asks for a limit, picked from the runs on record, past which the job still fails. The ratios on record, each the median of ten runs of going back to the first version on Git's source, by the `speed-gate` job:

| Runner | The ten runs on record, oldest first | Worst |
|---|---|---|
| GitHub's Linux runner | 2.12, 1.79, 2.02, 1.99, 2.40, 2.08, 2.50, 2.019, 2.425, 2.651 | 2.651 |
| GitHub's Windows runner | 1.62, 1.72, 1.62, 2.19, 1.70, 1.72, 1.72, 1.703, 1.763, 1.661 | 2.19 |

The first nine are in [benchmarks.md](../benchmarks.md) §8 and §9; the tenth is the Nightly run on `main` of 2026-11-11 ([42966403404](https://github.com/WisHonor/Bolt/actions/runs/42966403404)). Before milestone 6's speed work the same step took 4.9 times Git's time on Linux and 2.8 on Windows (§1), and 4.5 turn by turn on one Linux runner (§8). What moves the ratio from run to run is the runner: its disk (a `goto` flushes ten times, Git none) and its processor.

Decision 023's second gate, `status` on an unchanged 100,000-file project in under 2 seconds, is timed nightly but "printed and read, not enforced, until v0.1" ([testing.md](../testing.md) §5). v0.1 is here.

## Options

For the limit:
- **A. 2.5.** Below the worst run on record: the job would have failed today on a runner that changed nothing.
- **B. 3.** 13% above the worst run on record. Fitting a log-normal spread to the ten Linux ratios, a run on code that changed nothing goes past 3 about once in two hundred. A change that brings back a third of what milestone 6 took off (from about 4.7 times Git's time to about 2.2) fails most runs.
- **C. 3.5.** A false alarm less than once in ten thousand runs, but a change that brings back half of what milestone 6 took off passes about half the runs.

For the status gate:
- **A. Keep printing it.**
- **B. Fail its job above 2 seconds,** as the memory gate fails above its limit.

## Decision

**B and B.**

- **`goto` fails the job past 3 times Git's time**, the medians of ten runs as for the other two steps, on both systems. Up to 3 the job passes, and when the ratio is over 2 the line says it is over the v0.2 goal. `status` and the small save keep their limit of 2.
- **The status gate fails its job** when either of its two timed runs of `status` on the unchanged 100,000-file project takes 2 seconds or more. It runs nightly on GitHub's Linux runner, where it took 0.17 to 0.20 s after milestone 6.

## Consequences

- If a run on code that changed nothing goes past 3, the cause is found and written down in [benchmarks.md](../benchmarks.md), and the limit is loosened only by a new record.
- The limit says nothing about how a person's own disk treats ten flushes: [benchmarks.md](../benchmarks.md) publishes the losses, as before.
- `bench/run.sh gate`, `bench/README.md`, `crates/bolt-vcs/tests/speed.rs`, [testing.md](../testing.md) §4 and §5, [benchmarks.md](../benchmarks.md) §1 and "v0.1 ships when" in [milestones.md](../milestones.md) follow.
