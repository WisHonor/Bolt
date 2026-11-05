# 056 — A save that adds unusually much asks first, from milestone 3

- **Status:** Accepted
- **Date:** 2026-11-05
- **Agenda topic:** none (milestone 3 planning, question 9; raised by the owner's import check)
- **Decided by:** Claude, at the owner's request ("do whatever you think is best", 2026-11-05). The owner can reverse any part.
- **Refines:** [decision 003](003-working-copy-model.md) (a save that adds unusually much asks first) and [decision 016](016-presets.md) (presets set the threshold)

## Context

Decision 003 makes a save that adds unusually much ask first, with a threshold each preset sets and a flag that skips the question for scripts. Presets arrive in milestone 6, so until then nothing asks. The owner's import check (2026-11-04) showed the cost: the first save after importing Tailrd would have added about 190,000 files that Git's personal lists hide, without a word.

## Options

- **A. Wait for presets.**
- **B. A default now:** ask when a save adds more than 100 MB of new files (the Code preset's number) or more than 10,000 new files. Jujutsu, for comparison, refuses to snapshot a new file over 1 MiB unless told.
- **C. Warn only,** after saving.

## Decision

**B.**

- **New** means not in the save the new one builds on: a changed file doesn't count.
- **In a terminal** the save says how much it adds, in how many files, names the three biggest folders by size, and asks "Save them? [y/N]". Answering no saves nothing.
- **Without a terminal** it stops, saves nothing, and names the flag `--yes`, which also skips the question in a terminal.
- Presets (milestone 6) change the numbers through `bolt.toml`.

## Consequences

- A first save of a big game project asks once.
- [json.md](../json.md) documents the stop for scripts.
