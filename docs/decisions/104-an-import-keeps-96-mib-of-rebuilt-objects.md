# 104 — An import keeps 96 MiB of rebuilt objects, as Git does

- **Status:** Accepted
- **Date:** 2026-11-10
- **Agenda topic:** none (milestone 6, speed and space)
- **Decided by:** Claude, at the owner's request while the owner was away. The owner can reverse any part.
- **Refines:** [decision 086](086-rebuilding-gits-deltas-once.md) (how many rebuilt objects the Git reader keeps for the next delta built on them: 96 MiB, where it kept 256 MiB)

## Context

Importing Git's own repository peaked at about 573 MB of memory before milestone 5 and at 669 to 680 MB with it ([benchmarks.md](../benchmarks.md) §3). The 100 MB came from the Git reader's cache of rebuilt objects: since [decision 086](086-rebuilding-gits-deltas-once.md) it keeps the bases it uses and the objects it builds, so it stays near its limit of 256 MiB, where before it drained. The follow-ups asked for the import's memory to be looked at before v0.1.

The limit was measured ([run 42757172187](https://github.com/WisHonor/Bolt/actions/runs/42757172187) and [run 42759080455](https://github.com/WisHonor/Bolt/actions/runs/42759080455), 2026-11-10, GitHub's Linux runner, one import each):

| The cache's limit | Importing git/git (86,091 commits) | Peak memory |
|---|---|---|
| 256 MiB | 138.5 s, 141.3 s, 145.6 s | 657 MB |
| 128 MiB | 137.9 s | 500 MB |
| 64 MiB | 143.7 s | 401 MB |
| 32 MiB | 143.8 s | 352 MB |
| 16 MiB | 153.8 s | 319 MB |

The three runs at 256 MiB differ by 5%, so nothing down to 32 MiB is measurably slower; 16 MiB is. A long history of one big file (a 200 MiB file in 50 versions) imports in 39.7 to 40.2 s and peaks at 426 MB at 256, 128 and 64 MiB alike: there the objects are too big for the cache at any of these sizes, and wait in a file in `.bolt/tmp` as decision 086 has it.

Git's own cache of the same objects (`core.deltaBaseCacheLimit`) holds 96 MiB unless set otherwise.

## Options

- **A. Keep 256 MiB.** Nothing to do, and 100 MB more than before milestone 5 for no measured gain.
- **B. 96 MiB, Git's own default.** No slower in the measurements, about 200 MB less at the peak, and a size Git has used for the same delta chains for years.
- **C. 32 MiB,** the smallest that measured no slower. It saves another 100 MB, on the evidence of one repository.

## Decision

**B.** The Git reader keeps at most 96 MiB of rebuilt objects. Nothing else about the cache changes: what goes in, what is let go first, and the files in `.bolt/tmp` for objects that don't fit.

## Consequences

- An import of git/git should peak near 450 MB, below what it took before milestone 5. The final benchmark measures it, with the time, against `main` ([benchmarks.md](../benchmarks.md) §3).
- A repository whose delta chains reuse many objects of 1 MiB or more at once may rebuild more than before; each rebuild still counts against the import's allowance, so nothing becomes unbounded.
- The memory an import's own tables take (the Git objects it has converted) still grows with the repository; that row stays in the [follow-ups](../plans/2026-11-01-milestone-1-follow-ups.md), with what the import of Linux's history measured.
- [design.md](../design.md) §5.16 and [security.md](../security.md) T11 say 96 MiB.
