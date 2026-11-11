# 132 — `goto`'s limit until v0.2 is three and a half times Git's time

- **Status:** Accepted
- **Date:** 2026-11-11
- **Agenda topic:** none (getting v0.1 ready: clean code and the gate)
- **Decided by:** Claude, at the owner's request while the owner was away. The owner can reverse any part.
- **Supersedes:** the limit of [decision 129](129-gotos-limit-until-v0.2-and-the-status-gate-enforced.md) (3 times Git's time). The rest of 129 stands: the status gate fails its job.

## Context

Decision 129 picked 3 from the ten runs on record, whose worst was 2.651, expecting a run on unchanged code to pass 3 about once in two hundred. The first run under the new rule, on the branch at `f888f2a`, whose engine is `main`'s ([run 42970694870](https://github.com/WisHonor/Bolt/actions/runs/42970694870)), gave 3.012 on GitHub's Linux runner. Bolt's times there were ordinary (`goto` 25.8 ms; it was 22.9 to 32.4 ms in the earlier runs) but Git's were the fastest on record (`goto` 8.6 ms against 11.0 to 17.1; `status` 7.7 ms against 8.4 to 13.4). The runner was quick at what Git spends its time on, and a `goto`'s ten flushes don't get quicker with the processor. So the ratio moves with the runner more than ten runs showed: with the eleventh run the log-normal fit puts a run past 3 at about one in thirty-five, past 3.5 at about one in five hundred.

The same run had `status` at 2.207 times Git's time (17.0 ms against 7.7). That limit is the owner's (decision 128) and is not changed here; [benchmarks.md](../benchmarks.md) §10 records the run and why.

## Options

- **A. Keep 3.** About one nightly in thirty-five would fail on code that changed nothing.
- **B. 3.5.** About one in five hundred. A change that brings back half of what milestone 6 took off `goto` still fails about half the runs, and the ratio is printed every night.
- **C. Hold `goto` to Bolt's own past times in milliseconds.** Runners differ by up to twice in speed (benchmarks.md §6), so the runner would decide.

## Decision

**B.** `bench/run.sh gate` fails when going back to the first version takes more than 3.5 times Git's median; over 2 it still says it is over the v0.2 goal. Picked before the gate's ten runs on the finished code, from the eleven runs on record.

## Consequences

- `bench/run.sh`, `bench/README.md`, [testing.md](../testing.md), [benchmarks.md](../benchmarks.md) §1, "v0.1 ships when" in [milestones.md](../milestones.md), [design.md](../design.md) §1 and the plan follow; decision 129 gets a back-link line.
