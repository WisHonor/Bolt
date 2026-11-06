# Bolt

A new version control system with its own engine (not a Git wrapper), written in Rust. One tool for code and large binary assets: friendly commands, undo for everything, great big-file handling.

## Phase: BUILDING — milestone 1 ("First save") done 2026-11-01; milestone 2 ("Coming from Git") done 2026-11-04; milestone 3 ("Working folder") done 2026-11-06; milestone 4 next
- The owner signed off the design on 2026-10-31. Build milestone by milestone (`docs/milestones.md`), task by task from the plan in `docs/plans/`.
- The design docs stay the source of truth. Changing a decided behaviour or format needs a new decision record that supersedes the old one, and spec edits come before code.

## Where things live
- `docs/design.md`: the living design spec (plain language + technical detail). Read it first: it holds the agenda, risks, open questions and deferred decisions.
- `docs/plans/`: implementation plans, one per milestone.
- `docs/decisions/NNN-short-title.md`: one record per decision (Context, Options, Decision, Consequences), numbered from 001. Never rewrite an accepted decision; supersede it with a new record and link both ways.

## Working with the owner
- The owner is learning Rust and VCS internals and wants to understand and own every core decision.
- Bolt is a personal project, but design it as if it would be marketed and sold: the bar is "best on the market". Keep the technical goals ambitious.
- Plain language first (everyday analogies welcome), then the technical detail.
- One topic at a time, in agenda order: explain it, give 2–3 realistic options with trade-offs and which existing VCS does each, recommend one, then wait for the owner's decision.
- Be honest: push back on bad ideas, call out over-ambition, say when something is hard or already solved elsewhere.
- After each decision: write the decision record, update `docs/design.md` (the relevant section, the agenda checkbox, risks, open and deferred lists), then commit and push.
- Commits and PRs carry no Claude attribution: no `Co-Authored-By: Claude` trailer and no "Generated with Claude Code" line (owner's request; overrides default attribution).

## Non-negotiable principles
1. Never lose or corrupt user data: atomic writes, repository locking, hash verification on every read and every received object, crash safety.
2. Security by design: treat every repository and server as potentially malicious.
3. Testability: unit, round-trip, property-based, differential vs. Git, crash, fuzz, malicious-repo corpus; CI on Windows, macOS and Linux.
4. Provable claims: reproducible benchmarks vs. Git, `bolt verify`, a documented storage format spec.
5. Correct first, fast second: measure before optimizing.

## Engineering rules (decision 022)
- The engine library uses `#![forbid(unsafe_code)]`. `unsafe` is allowed only in a tiny, separate platform crate, for operating-system calls; the owner reviews it line by line.
- No panics in library code: return errors; deny clippy's `unwrap_used`, `expect_used` and `panic` outside tests.
- Spec before code for anything stored on disk (`docs/storage-format.md`), with test vectors for every format.
- One place per rule: name validation, path building and write-out each live in a single module that everything else uses.
- Keep changes small; the owner reviews all code that touches disk or network before it is merged.
- A task is done only when its tests pass on Windows, macOS and Linux.
- A milestone is merged only after its red-team session (decision 024): a fresh agent attacks it from the no-network Docker sandbox, and every proven finding is fixed or ruled on in writing. Attack code never runs outside the sandbox or a `redteam/*` CI branch.
