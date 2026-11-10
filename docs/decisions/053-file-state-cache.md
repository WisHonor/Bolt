# 053 — The file-state cache: what it records, racy files, and a cache Bolt didn't write

- **Status:** Accepted
- **Date:** 2026-11-05
- **Agenda topic:** none (milestone 3 planning, question 6)
- **Decided by:** Claude, at the owner's request ("do whatever you think is best", 2026-11-05). The owner can reverse any part.
- **Refines:** [decision 011](011-status-and-write-out.md) (the cache and the racy rule) and [decision 028](028-red-team-m1-session-2-rulings.md) #1 (nothing in `cache/` vouches for an object)
- **Refined by:** [decision 067](067-the-file-cache-matches-file-identity.md) (an entry matches the file's ID too) and [decision 069](069-the-file-cache-matches-the-change-time.md) (and its change time); [decision 084](084-files-are-read-once-on-several-threads.md) (the files the cache doesn't vouch for are read once, on several threads); [decision 100](100-caches-are-written-without-a-flush-and-only-when-they-changed.md) (the caches are written without a flush, and the file cache only when it changed); [decision 105](105-the-change-id-cache-keeps-each-lines-saves-in-order.md) (`cache/change-ids` keeps each line's saves in order, under the same seal)

## Context

`status` must say what changed without reading every byte (decision 023: under 2 seconds for an unchanged 100,000-file project). Decision 011 settled the idea: remember each file's size and last-modified time, trust files whose details match, and re-read files changed too close to when the cache was written (Git's "racy" files). Three things were open:

- **What makes a planted cache harmless.** `.bolt/cache/` travels with a copied project. A cache that lies could make a save treat a changed file as unchanged, so the user's change is silently not saved.
- **The racy rule's details.** A file changed twice within one tick of the drive's clock keeps the same time.
- **Using the cache in `save`.** Then a save stops reading back unchanged files, and stops downloading OneDrive files that are online only.

## Options

For a planted cache:
- **A. Details a copy can't reproduce:** size, time, change time, inode and device, as Git's index records. On Windows Rust gives no inode or change time without new `unsafe`, and an unzipper can set a file's creation time.
- **B. A keyed checksum over the cache,** with a random key kept in the user's own settings folder, outside every project. A cache Bolt didn't write for this account fails the check and is rebuilt. BLAKE3, already used, has a keyed mode. Mercurial keeps size, time and mode, and trusts its dirstate.
- **C. Don't trust the cache in `save`:** a save reads everything, which fails the speed bar and downloads online-only files.

## Decision

**B.**

- **Per file** the cache records its path, kind (file, executable, link), size, last-modified time in nanoseconds, and content ID.
- **The key** is 32 random bytes in the user's own settings folder, made on first use. The cache is written atomically; one whose keyed checksum fails is ignored and rebuilt.
- **Racy rule.** At the start of a scan Bolt writes a file in `.bolt/tmp` and takes its modified time as "now", so the drive's own clock and resolution count. A file whose time is not at least 2 seconds older than that (FAT's resolution) is read again next time.
- **`save` uses it only to decide** that a file is unchanged since the save it builds on; it then reuses that save's entry, unread. The cache never names content to a save. `status` uses its content IDs to show what changed. `bolt status --full` and deleting `cache/` read everything again.

## Consequences

- The cache file is described in [storage-format.md](../storage-format.md) §10, not normative; a test checks that a cache from another account, or edited, is not used.
- Milestone 3 measures decision 023's speed bar for `status` once this is built.
