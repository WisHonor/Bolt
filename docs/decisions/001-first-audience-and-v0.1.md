# 001 — First audience, main platform and v0.1 scope

- **Status:** Accepted
- **Date:** 2026-10-31
- **Agenda topic:** 1
- **Refined by:** [decision 157](157-v0.2-in-four-milestones.md) (v0.2 in four milestones, and what it leaves out)

## Context

Bolt is a personal project, but it is designed as if it would be marketed and sold, so the bar is "best on the market". It can't be best for everyone on day one, so we pick who it must delight first. That choice sets the main platform, what goes into the first usable version, which presets and benchmarks come first, and the build order (locking, for example, needs a server).

## Options

**First audience**
- **Game developers, Windows first.** The most pain: Git + LFS is clumsy, Perforce is expensive and unfriendly, Unity Version Control and Diversion are proprietary. Needs big binaries, locking once there are two people, and tools for artists.
- **AI/ML, Linux and macOS first.** Needs dedup and partial download; Hugging Face (Xet storage), DVC, Oxen and lakeFS already serve it.
- **Code, "a friendlier Git".** The biggest audience and the hardest to win; jj already offers much of the same UX while staying Git-compatible.
- Design and video teams were set aside for now: they need previews and a GUI from day one.

**First usable version**
- **Code first, big files later** (the brief's original build order). A gentle start, but big files get bolted on late, which is one reason Git struggles with them.
- **Big files in v0.1, branches and merging in v0.2.** The distinctive feature comes first and streaming is designed in from the start; chunking must be settled sooner.
- **Everything in v0.1.** Feels complete, but the longest wait before anything is usable.

## Decision

- **First audience: game developers.** The engine stays general (one format for every kind of project); games set the priorities, first presets and benchmarks.
- **Main platform: Windows (tier 1).** macOS and Linux are supported and tested in CI from the start.
- **v0.1, solo and local-only:** `start`, `save`, `status`, `changes`, `history`, `show`, `goto`, `discard`, `undo`/`redo`, Git import, and chunked big files. Big files are streamed, never loaded whole into memory.
- **v0.2:** lines and combine (branches, merging, conflicts).
- **v0.3:** sync and locking for small teams; big files and old versions download on demand.

## Consequences

- Windows behaviour (files held open by other programs, reserved names, path limits) is a core requirement, not polish.
- Big-file identity and chunking (topics 5 and 9) must be settled before v0.1, earlier than the brief planned.
- Build order: test infrastructure is built with the object store; a minimal Git importer follows saves and history; chunked big files come before lines and combine.
- Locking and on-demand download need a server, so they wait for v0.3. Earlier versions record lock rules but can't enforce them.
- Game-engine presets come first, guided by the owner's target `bolt start` screen (design.md §1); which presets ship in which version is decided in topic 16.
- Real game projects are needed as test data for benchmarks and dedup measurements.
