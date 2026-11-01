# 020 — Importing Git LFS files

- **Status:** Accepted
- **Date:** 2026-10-31
- **Agenda topic:** 18 (design review)
- **Supersedes:** the last consequence of [decision 002](002-git-compatibility.md) ("LFS files whose content isn't available locally must import as 'known but not downloaded'")

## Context

Bolt identifies files by the BLAKE3 hash of their contents (decision 005). A Git LFS pointer only gives a SHA-256 hash and a size, and a normal Git clone only downloads LFS contents for the checked-out version. Importing a full history therefore needs the old contents, and v0.1 has no server to fill gaps later.

## Options

- **Require the contents on disk.** The user runs `git lfs fetch --all`; Bolt reads the local LFS storage and stops before writing anything if contents are missing. Simple, no network code, no format change; the whole LFS history must be downloaded first.
- **Bolt downloads missing contents itself.** Convenient, but adds network code and Git-hosting logins to a local-only v0.1.
- **Allow "contents unknown" entries** keyed by SHA-256. Imports anything, but adds a permanent entry kind that Bolt can never verify itself.

## Decision

**Require the contents on disk.**
- `bolt import` reads LFS contents from the Git repository's local LFS storage and checks each one against its pointer's SHA-256 hash and size before storing it.
- Before writing anything, import checks that every LFS file in the history being imported is present. If any are missing, it stops, lists them, and explains how to fetch them (`git lfs fetch --all`).
- Imported LFS files become normal Bolt files, identified by their BLAKE3 hash (decision 005).

## Consequences

- No format change and no network code in v0.1.
- Importing a large LFS history needs that history downloaded first; the message says so and estimates the size when it can.
- Letting Bolt fetch missing contents itself can be added later as a convenience.
- Histories whose LFS contents are gone for good (deleted from the server) can't be fully imported in v0.1; a partial import can be designed if needed.
