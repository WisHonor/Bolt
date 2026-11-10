# 091 — The import's work stop names a way out for long histories of small edits

- **Status:** Accepted
- **Date:** 2026-11-09
- **Agenda topic:** none (milestone 5 build)
- **Decided by:** Claude, at the owner's request while the owner was away. The owner can reverse any part.
- **Refines:** [decision 086](086-rebuilding-gits-deltas-once.md) (the stop past the import's work allowance: what it says, and its hint)

## Context

Decision 044 gives an import an allowance for rebuilding Git's deltas: the larger of 4 GiB and 500 bytes for each compressed byte it inflates. Decision 086 rebuilds each big file's delta family once and keeps that allowance. It expected honest histories of big files to fit, so that the stop past the allowance would point only at repositories built to make importing slow. Its hint says so: "If you trust this repository, have Git make its deltas anew with `git repack -a -d -f`".

Building it showed that this holds for 086's own example (a 200 MiB file in 50 versions, each changing 1%), but not for every honest history. Each version is now built once, so the work is the size of all the versions together, while a small edit adds a delta of only a few hundred bytes to what is inflated. So the longer such a history, the more work each inflated byte must buy:

- a file of 8 MiB that doesn't compress, edited a few bytes at a time, passes the allowance after about 500 versions;
- a 20 MB text file that Git compresses tenfold, such as a game scene, edited a few KB at a time, passes it after about 200.

Only a repository whose history is mostly such a file gets there: every other file the import inflates adds 500 times its compressed size to the allowance. But then the stop calls an honest repository hostile, and its hint doesn't help, since Git's new deltas are as small as the old ones. The import can't tell the two apart: many tiny deltas that each build a big file are exactly what a hostile pack sends.

## Options

- **A. Raise the 500.** Honest histories up to the new ratio fit, but a hostile pack buys that much more work for each byte, and decision 044's red-team bounds would need measuring again.
- **B. A flag that lifts the allowance** for a repository the user trusts. It needs no Git step, but it is a new option, and a stop that can be passed by habit.
- **C. Keep the allowance; say what the stop may be, and name a way out that works for both.** Have Git store each file over 1 MiB whole and make its other deltas anew: `git -c core.bigFileThreshold=1m repack -a -d -f`. A file Git stores whole needs no rebuilding at all, and Bolt cuts its versions into chunks and shares them anyway.

## Decision

**C.**

- **The message:** rebuilding objects from Git's deltas would take more than the N GiB of work Bolt allows: the repository holds a long history of small edits to a big file, or it was built to make importing slow.
- **The hint:** "Nothing was changed. If you trust this repository, have Git store its files over 1 MiB whole and make its other deltas anew, with `git -c core.bigFileThreshold=1m repack -a -d -f`, then run bolt import again. Git's packs stay bigger until its next `git gc`."
- **The allowance stays** as decision 044 set it, and its red-team bounds with it.
- **Known limit:** files of 1 MiB or less keep their deltas after that repack. So a history whose small files alone need more than the allowance still stops. That takes thousands of tiny edits to each, which hasn't been seen in a real repository; `git repack -a -d -f --window=0`, which stores every object whole, gets past it.

## Consequences

- An honest history that passes the allowance imports after one Git step. That step makes Git's packs bigger, with every version of each file over 1 MiB stored whole and compressed, until Git's next `git gc` makes deltas again.
- The stop no longer calls such a repository hostile.
- If real repositories hit it often, option A or B can come later, with the benchmarks of [decision 088](088-benchmarks-and-the-speed-gates.md) to measure them by.
- design.md §5.16 describes the stop.
