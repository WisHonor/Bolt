# 153 — The platform code that follows Rust's stays, under Rust's license

- **Status:** Accepted
- **Date:** 2026-11-12
- **Agenda topic:** none (the v0.1 release: the final builds)
- **Decided by:** the owner
- **Refines:** [decision 131](131-code-that-follows-another-projects-is-rewritten-clean.md) (the one piece of code it left to the owner)

## Context

Decision 131 had every piece of code that followed another project's code written again under clean-room rules, with one exception: about 300 lines of the platform crate's Windows folder code (`crates/opal-vcs-platform/src/dir/windows.rs`). They follow Rust's standard library: how a name is opened relative to a held folder, deleted and renamed by handle, listed, and how a link's target is shown. Rust's license (MIT or Apache-2.0) allows that in a closed product as long as its notice ships, and every archive ships it as `THIRD-PARTY-LICENSES-RUST.html`. Decision 131 left the choice to the owner, and no release build was to be made until the owner ruled ([the code audit](../releases/v0.1.0-code-audit.md); the page for the owner, part 2).

## Options

- **A. Keep it, under Rust's license,** with Rust's notice in every archive. This is what the audit recommends: every Rust program owes Rust's notice anyway, since the standard library is built into it.
- **B. Have it written again,** clean. That means new `unsafe` code, in the part that defends against a folder swapped under a command. The owner would review it line by line, and no user would see a change.

## Decision

**A.** The code stays as it is, under Rust's license, and Rust's notice ships in every archive. It is the one exception to the rule that no code following another project's code is in v0.1.

## Consequences

- The release build is no longer held up.
- Every archive must keep carrying `THIRD-PARTY-LICENSES-RUST.html`. The release workflow puts it in each archive, copied from the Rust that built it.
- A rewrite stays possible later. It would need a decision of its own and the owner's line-by-line review.
