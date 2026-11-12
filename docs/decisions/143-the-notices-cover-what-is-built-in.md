# 143 — The notices cover everything built into the program

- **Status:** Accepted
- **Date:** 2026-11-11
- **Agenda topic:** none (the v0.1 release: the package)
- **Decided by:** Claude, at the owner's request while the owner was away. The owner can reverse any part.
- **Refines:** [decision 119](119-packaging-v0.1.md) (`THIRD-PARTY-LICENSES.html` in each archive); [decision 140](140-bolt-is-closed-source.md) (what the dependencies' licenses ask of a closed product)

## Context

**In plain words.** Most of the code inside Bolt's program was written by other people, who let anyone use it on one condition: their notice travels with every copy. The notices file is where Bolt keeps that condition. It has to cover everything inside the program, not just the parts a tool happened to list.

**What the file covered.** `THIRD-PARTY-LICENSES.html` is written by cargo-about for every crate built into `bolt` (the run that made the last draft, 2026-11-10, was read on 2026-11-11). It covers:
- every crate in the program, with its license text, and the procedural macros that run only while it is built (harmless extras);
- C code inside a crate: `zstd-sys` builds the zstd C library into the program, and the file carries that library's own BSD-3-Clause text (Meta Platforms); `blake3`'s C and assembly are the crate's own code, under its own licenses;
- data inside a crate: `encoding_rs`'s WHATWG tables (BSD-3-Clause), the Unicode tables of `unicode-ident` and `unicode-normalization` (Unicode-3.0), and `jiff-tzdb`'s time-zone database (public domain).

**What it missed.**
- **Rust's standard library,** which is in every Rust program but in no crate graph, so cargo-about never sees it. It is MIT or Apache-2.0, and holds code from other projects (LLVM's compiler-rt, `hashbrown`, `gimli` and more). The toolchain installs the Rust project's own notices for it, `share/doc/rust/COPYRIGHT-library.html`, with the `rustc` component, so every toolchain that builds Bolt has the file that matches it.
- **Microsoft's C runtime** in the Windows program, linked in so that nothing needs installing (decision 119). It comes with the Visual Studio build tools on GitHub's Windows runner, under Microsoft's license for them.
- On Linux and macOS the C library is the system's own (glibc, libSystem), loaded when the program starts and never shipped; the small start-up files linked in from glibc and GCC carry exceptions that let any program use them without a notice.

**MPL-2.0.** `deny.toml` allows MPL-2.0 for every crate. One crate uses it: `option-ext` 0.2.0, which `directories` 6.0.0 brings through `dirs-sys` 0.5.0 to find the user's settings folder.

## Options

- **A. Leave the file as it is.** The standard library's notices are missing from every copy.
- **B. Write the standard library's licenses into the template by hand.** They would go stale with each new toolchain.
- **C. Ship the Rust project's own notices beside ours, from the toolchain that built the program, and say in ours what isn't a crate.**

## Decision

**C.**

- **Each archive holds two notices files:** `THIRD-PARTY-LICENSES.html` (the crates, from cargo-about) and `THIRD-PARTY-LICENSES-RUST.html`, a copy of `COPYRIGHT-library.html` from the toolchain of the release build, whose version is pinned in `rust-toolchain.toml`.
- **`THIRD-PARTY-LICENSES.html` opens by saying what it covers,** names the second file for the standard library, and says that the Windows program also holds Microsoft's C runtime, linked in by Microsoft's tools under their license, and that on Linux and macOS the system's own C library is used and not shipped.
- **Each crate it lists names where its source is:** its page for that version on crates.io. That is what MPL-2.0 asks, and it costs the other crates nothing.
- **MPL-2.0 is allowed for `option-ext` alone.** `deny.toml` moves it from the general allow list to an exception for that crate, so a second MPL-2.0 crate stops CI until someone rules on it. What MPL-2.0 asks of the owner, for `option-ext`:
  - its license text goes with the program (it is in the notices);
  - people are told where its source is (crates.io, in the notices) and can get it at no charge;
  - the user agreement doesn't limit their rights in that source (it says so, [decision 141](141-the-user-agreement.md));
  - any change to its files would have to be shared under MPL-2.0 (Bolt changes none);
  - nothing else in Bolt is affected: MPL-2.0 works file by file.
- **What each license asks of a closed product** is the table in [decision 140](140-bolt-is-closed-source.md). The notices meet every row: the texts and notices are in the files, there are no `NOTICE` files to pass on, and Bolt changes no one else's files.
- **The C runtime's terms are a question for the lawyer.** Microsoft's license for the Visual Studio tools lets a program carry the runtime linked in, on conditions (among them that the program's own license protects that code at least as much as Microsoft's does, which the agreement's ban on taking the program apart helps with), and whether the copy on GitHub's runner may be used for a paid product depends on who holds which Visual Studio license. The page for the owner lists it.

## Consequences

- Each archive gains about 1.5 MB of HTML before compression.
- The release workflow fails when either notices file is missing from an archive ([decision 145](145-one-release-workflow-for-every-version.md)), or when cargo-about lists one of Bolt's own crates (as before).
- A dependency with a license not on the allow list stops CI; adding it needs a ruling on what it asks of a closed product.
- If Bolt's Windows build moves off Microsoft's runtime (a GNU toolchain, or the runtime as a separate DLL), the lawyer's question goes with it.
