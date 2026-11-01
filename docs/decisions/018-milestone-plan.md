# 018 — Milestone plan for v0.1

- **Status:** Accepted
- **Date:** 2026-10-31
- **Agenda topic:** 18

## Context

The design decisions are made; coding starts once the design documents are complete and reviewed. Milestone 1 has to balance a solid foundation against an early working result, because this is a personal project where motivation matters (risk 1).

## Options

- **Foundation only:** fingerprints, encoding, packs, `.bolt/` and `bolt start`. Rock-solid, but weeks with nothing visible.
- **First save:** the same foundation plus `start`, `save`, `history` and `show` working end to end on the real formats.
- **The whole core loop:** first save plus `status`, `changes`, `goto`, `discard` and undo. Usable, but too big for a first milestone.

## Decision

**Milestone 1 is "First save".** The rest of v0.1 follows in this order: Coming from Git, Working folder, Undo everything, Big files, Presets and polish. Details are in [milestones.md](../milestones.md).

## Consequences

- Git import (milestone 2) arrives early, so later milestones are tested against real histories.
- Milestone 1 stores every file whole; chunking (milestone 5) changes no IDs (decision 005).
- Each later milestone gets its own step list when it starts.
