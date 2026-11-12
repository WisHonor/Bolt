# 121 — Format 1 freezes at v0.1

- **Status:** Accepted
- **Date:** 2026-11-09
- **Agenda topic:** none (milestone 6 planning)
- **Decided by:** Claude, at the owner's request while the owner was away. The owner can reverse any part.
- **Refines:** [decision 008](008-layout-and-settings.md) (the format version, the feature list, and upgrades only through `bolt setup`); [decision 019](019-review-clarifications.md) (format 1 freezes at the v0.1 release); [decision 083](083-a-chunk-list-is-stored-under-its-files-id.md) (whether `feature chunks` stays a feature line "is settled when format 1 freezes"); [decision 012](012-lines.md) (how line names are compared)
- **Refined by:** [decision 109](109-the-speed-sessions-follow-ups-and-format-1s-three-open-rows.md) (the three format questions it left open: none changes the format); [decision 148](148-what-format-1s-spec-says-before-it-freezes.md) (the conflict record's layout doesn't freeze, only its reservation and v0.1's behaviour on meeting it; the ignore rules keep Git's behaviour at the versions the spec names)
- **Superseded in part by:** [decision 150](150-the-name-is-opalro.md) (the names: Bolt is now Opalro, and `bolt` is now `opalro`)

## Context

[storage-format.md](../storage-format.md) has said since milestone 1 that format 1 freezes at the v0.1 release, and that until then anything in it may change and repositories made by development builds are disposable. v0.1 is now being prepared, so the freeze needs to be said exactly: when, what it covers, what it promises, and what happens to the questions the docs left "before format 1 freezes".

Those questions are: whether `feature chunks` stays a line in `.bolt/format`; how line names are compared (design.md §5.10); the letters of change IDs, which is display only ([decision 122](122-change-ids-in-jujutsus-letters.md)); the name "Bolt" itself, which is built into `.bolt/`, `bolt.toml`, `.boltignore` and the packs' first bytes; and three rows that the speed session of milestone 6 owns, each of which could change what is stored (a Git tag object stored once, a bound on a chunk list's chunk count, a tighter bound on a stored copy's size).

## Options

**The freeze**
- **A. Freeze at v0.1,** as the docs have said.
- **B. Call v0.1 a preview** whose repositories a later version may ask to be made again. Free to mend mistakes, but nobody should keep their history in a tool that may not read it next year.

**`feature chunks`**
- **A. It stays a feature line,** in every repository. No code changes. A development build from before milestone 5 refuses such a repository, as decision 083 wanted: its `tidy` would remove chunks that are still needed.
- **B. It joins format 1** and the line goes. A cleaner file, but the same old build would then open a v0.1 repository and could damage it, and the owner still has such builds.

**Line names**
- **A. Keep comparing by Unicode's lowercase mapping,** as built and as storage-format.md §9 states.
- **B. Compare by full case folding,** so `ß` and `SS` clash. It needs a folding table Rust's standard library doesn't have, and makes readers refuse some diaries they accept today.

## Decision

**Freeze at v0.1 (A). `feature chunks` stays (A). Line names stay as built (A).**

- **When.** Format 1 freezes when v0.1 is released: when the owner publishes it. Until that moment either session of milestone 6 may still change the format, each change with its own decision and its test vectors first.
- **What freezes:** everything storage-format.md marks as part of the format: how every object is encoded and fingerprinted (§1 to §9), chunk lists among them, the files in `.bolt/` it calls normative (§10), packs (§11; version 2 is written, versions 1 and 2 are read), the limits, and the project files (§12): `bolt.toml`'s keys, its values for an absent key, and how `.boltignore` is read. Their test vectors freeze with them.
- **What the freeze promises.** Every later Bolt opens any repository v0.1 wrote and reads everything in it. Anything new on disk arrives as a `feature` line or as a later format number; a repository gains one only when its user asks, through `bolt setup` (decision 008), and from then on a Bolt older than that feature refuses the repository cleanly, as decision 008 has it.
- **What doesn't freeze:** where Bolt cuts a file into chunks, which storage-format.md §6 and decision 082 keep out of the format on purpose (any chunk list that rebuilds its file is valid; Bolt's own cutter is pinned by vectors, and changing it stays a decision of its own); `cache/`, `tmp/`, `state/lock-info` and `state/tidied`, which Bolt rebuilds, or reads only for a message or to save itself work; what commands print (the `--json` formats have their own freeze, [decision 117](117-what-json-covers-at-v0.1.md)); and how fast or in what order Bolt does its work.
- **Repositories from development builds** (anything before v0.1) are outside the promise, as the spec has always said, though Bolt goes on reading them today.
- **`feature chunks` stays a line.** Every repository v0.1 makes carries `bolt-repository 1`, `hash blake3` and `feature chunks`, and a reader of format 1 must know that feature. The step that adds the line to a repository made before milestone 5 stays, and stays the one format change made without `bolt setup` (decision 083).
- **Line names** are compared as storage-format.md §9 says: after Unicode's lowercase mapping. Which characters are letters, and what their lowercase is, follows the Unicode version of the Bolt that reads the name (17.0 in v0.1). So a later Bolt may accept a name with a letter a newer Unicode added, which v0.1 refuses, saying the name isn't valid. Known limit: if a later Unicode gave two existing line names the same lowercase form, a Bolt built on it would have to keep the older answer for format 1; no such pair is known.
- **The name stays "Bolt"** in format 1: `.bolt/`, `bolt.toml`, `.boltignore`, and `BOLTPACK`. Renaming after the release would need a new format, so the owner's last chance to rename is before publishing; it is on milestone 6's please-confirm list.
- **The speed session's format rows** are that session's to decide before the release. Whatever isn't changed by then is format 1 as it stands, and a later fix must come without changing the format, or as a feature.

## Consequences

- storage-format.md's status line and §14 say what is frozen and when; design.md's deferred items on line names and on the name point here.
- From v0.1 on, a mistake in format 1 can be mended only by a feature or a new format, each turned on through `bolt setup`. That is the price of a history people can rely on.
- No code changes for this decision: `.bolt/format`, its test vector and the line-name rules are as they were.
