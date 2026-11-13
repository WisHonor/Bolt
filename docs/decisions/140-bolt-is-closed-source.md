# 140 — Bolt is closed source

- **Status:** Accepted
- **Date:** 2026-11-11
- **Agenda topic:** none (the v0.1 release: the package)
- **Decided by:** the owner.
- **Supersedes:** [decision 119](119-packaging-v0.1.md) in part (its "License" point, which prepared v0.1 with no license, and its consequence that anyone can build a release again from the public lock file and workflow)
- **Refined by:** [decision 141](141-the-user-agreement.md) (the user agreement); [decision 143](143-the-notices-cover-what-is-built-in.md) (the notices, and what MPL-2.0 asks); [decision 156](156-the-user-agreements-blanks-filled-by-the-owner.md) (the license's rights holder, filled by the owner)

## Context

**In plain words.** A license is the permission slip that says what other people may do with something you made. Without one, the law keeps every right with the maker: nobody else may copy, change or pass it on. Until now Bolt was prepared with no license ([decision 119](119-packaging-v0.1.md)): the repository was public, so anyone could read the code, but nobody had the right to use it, and the README said "Bolt has no license yet". The owner has chosen.

**Technically.** The workspace's three crates are `publish = false`. `deny.toml` passes over them in its license check (`private = { ignore = true }`) because they had no license. [design.md](../design.md) §1 still describes Bolt as "open source (likely MIT/Apache-2.0 for the tool …)". Bolt is built from about 80 open-source crates and Rust's standard library, and their licenses ask things of whoever passes the program on.

## Options

- **A. Open source** (MIT or Apache-2.0, as design.md's vision had it). Anyone may use, change and pass on Bolt. Git and Mercurial (GPL-2.0), Jujutsu (Apache-2.0) and Sapling (GPL-2.0) work this way.
- **B. Source available.** The code is public, under a license that limits what others may do with it for some years (the Business Source License, the Functional Source License). No version control system of note works this way.
- **C. Closed source.** All rights reserved, the source private, and builds downloaded from the owner's website under a user agreement. Perforce's Helix Core and Unity Version Control (formerly Plastic SCM) work this way.

## Decision

**C, decided by the owner.** All rights reserved; the source stays private; people download builds from the owner's website under a user agreement.

In the repository:
- **`LICENSE`**, at its top, says that Bolt is all rights reserved and closed source, and that the user agreement shipped with each build is what lets a person use it. Who holds the rights is one of the owner's blanks: the agreement's "who grants it".
- **The manifests:** `license-file = "LICENSE"` in `[workspace.package]`, inherited by the three crates, and `publish = false` stays, so `cargo publish` refuses every one of them.
- **`deny.toml` keeps `private = { ignore = true }`.** Its allow list says what a closed product may be built from, so it judges the crates of others, never Bolt's own, which are closed and never published. The follow-ups row that planned to drop the exemption was written for an open-source license; this record closes it.
- The README, the guide and the release notes say that Bolt is closed source and used under the user agreement, where they said "no license yet".
- Making the repository private is the owner's own step.

**What it means for the licenses of the code Bolt is built from.** A closed product may be built from code under the licenses below, as long as it does what each asks. Every one of them stays in force for its own code: the user agreement can't take away what they give.

| License | What it asks of a closed product | In Bolt's build (2026-11-11) |
|---|---|---|
| MIT, ISC | Keep the copyright notice and the permission notice with every copy. | most crates |
| BSD-2-Clause, BSD-3-Clause | The same, in the documentation that goes with the program; BSD-3-Clause adds: don't use the authors' names to promote Bolt. | `zstd`, `zstd-safe`, `zstd-sys` (and the zstd C library inside it), `encoding_rs`'s data |
| Apache-2.0 | Give a copy of the license, keep their notices, pass on any `NOTICE` file, and mark files you changed. Whoever sues claiming the code infringes a patent loses the license. | `similar`, and `blake3` and `constant_time_eq` where it is chosen; none has a `NOTICE` file, and Bolt changes none |
| Apache-2.0 WITH LLVM-exception | As Apache-2.0, but a compiled program needn't carry the notices. | `rustix`, `linux-raw-sys` (also MIT) |
| Unicode-3.0 | Keep Unicode's copyright and permission notice with the data. | `unicode-ident`, `unicode-normalization` |
| Zlib | Nothing for a program; don't claim you wrote it. | `miniz_oxide`, `tinyvec` (also MIT) |
| CC0-1.0, Unlicense | Nothing. | `blake3`, `constant_time_eq`, `jiff`, `memchr` (each also offered under Apache-2.0 or MIT) |
| MPL-2.0 | File by file: its own files' source must stay available under MPL-2.0, and people told where; changes to those files must be shared; the rest of the program is untouched. | `option-ext` only ([decision 143](143-the-notices-cover-what-is-built-in.md)) |

- **Not allowed:** licenses that would oblige Bolt to publish its own source or to let users relink it, or that would limit what the user agreement may say: the GPL, LGPL, AGPL, SSPL, EUPL and their like. None is on the allow list, so cargo-deny refuses any crate that brings one, on every push.
- **The allow list changes only with a ruling** on what the new license asks of a closed product.
- **Code taken from other projects** (written from another project's code rather than used as a dependency) is the engine session's audit, in its own decisions; this record doesn't rule on it.

## Consequences

- The repository becomes the owner's private workshop: nothing that ships in a build or goes on the website may point into it ([decision 144](144-where-people-get-bolt-and-report-problems.md)).
- Nobody outside can read the code, build it, open issues or see releases. Decision 119's consequence that anyone can build a release again from the public lock file and workflow no longer holds. What an outside reader can still check is the program itself, and the storage format and the benchmarks if the owner publishes them.
- design.md §1's vision line changes, and §7's "Contribution licensing: CLA or DCO" is moot while Bolt takes no outside contributions.
- The user agreement ([decision 141](141-the-user-agreement.md)) and the notices ([decision 143](143-the-notices-cover-what-is-built-in.md)) ship beside each build.
- The package names (`bolt-vcs`, `bolt-vcs-core`, `bolt-vcs-platform`) were chosen in milestone 1 so they could be published to crates.io later; they now never are.
