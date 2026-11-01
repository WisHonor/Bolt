# Bolt

A new version control system with its own engine (not a Git wrapper), written in Rust. One tool for code and large binary assets: friendly commands, undo for everything, great big-file handling.

## Phase: DESIGN — no implementation code
- Don't write implementation code, create Cargo projects, or add dependencies until the owner explicitly ends the design phase.
- The design phase ends when these exist and the owner approves them: a complete `docs/design.md`, a storage format spec draft, a testing and security plan, and a milestone-1 task list of small, testable steps.

## Where things live
- `docs/design.md`: the living design spec (plain language + technical detail). Read it first: it holds the agenda, risks, open questions and deferred decisions.
- `docs/decisions/NNN-short-title.md`: one record per decision (Context, Options, Decision, Consequences), numbered from 001. Never rewrite an accepted decision; supersede it with a new record and link both ways.

## Working with the owner
- The owner is learning Rust and VCS internals and wants to understand and own every core decision.
- Bolt is a personal project, but design it as if it would be marketed and sold: the bar is "best on the market". Keep the technical goals ambitious.
- Plain language first (everyday analogies welcome), then the technical detail.
- One topic at a time, in agenda order: explain it, give 2–3 realistic options with trade-offs and which existing VCS does each, recommend one, then wait for the owner's decision.
- Be honest: push back on bad ideas, call out over-ambition, say when something is hard or already solved elsewhere.
- After each decision: write the decision record, then update `docs/design.md` (the relevant section, the agenda checkbox, risks, open and deferred lists).

## Non-negotiable principles
1. Never lose or corrupt user data: atomic writes, repository locking, hash verification on every read and every received object, crash safety.
2. Security by design: treat every repository and server as potentially malicious.
3. Testability: unit, round-trip, property-based, differential vs. Git, crash, fuzz, malicious-repo corpus; CI on Windows, macOS and Linux.
4. Provable claims: reproducible benchmarks vs. Git, `bolt verify`, a documented storage format spec.
5. Correct first, fast second: measure before optimizing.
