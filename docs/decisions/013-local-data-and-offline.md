# 013 — What's on your computer, and offline behaviour

- **Status:** Accepted
- **Date:** 2026-10-31
- **Agenda topic:** 13
- **Refined by:** [decision 082](082-how-big-files-are-cut-into-chunks.md) (the chunking threshold is 1 MiB until the presets make it a `bolt.toml` setting)

## Context

Team game projects accumulate hundreds of gigabytes of history: every version of every asset. Nobody wants all of it on every computer. This decision sets what is always local, what downloads on demand, and how Bolt behaves offline. It is built with sync in v0.3; v0.1 only needs to respect it.

## Options

- **Everything on every computer** (Git, Mercurial). Simplest and fully offline, but impossible for big game projects.
- **All history, but only current big files** (Git partial clone with a size filter; Git LFS by default). History and status work offline; old big-file versions need the server.
- **Everything on demand through a virtual filesystem** (VFS for Git, EdenFS, OneDrive Files On-Demand). Instant, but needs OS-specific drivers, helps little when engines scan every asset on open, and network stalls freeze the editor.

## Decision

**All history, but only current big files.**
- **Always local:** every save, folder listing and diary entry, and every version of every small file.
- **Big files:** the versions the working folder uses are always fully downloaded and never written as stub files. Older versions download on request; offline, Bolt says exactly what is missing.
- **Pinning:** saves or folders can be marked to stay available offline.
- **Partial workspaces (later):** for huge projects, users choose which folders to download at all.
- **Freeing space:** Bolt may drop downloaded big-file versions only if the server has confirmed it holds them; data that exists only locally is never dropped.

## Consequences

- v0.1 needs nothing new: decision 005 already supports "listed, but contents not here".
- Sync (topic 15) must let the server confirm what it holds and must fetch individual file versions efficiently.
- Every command must handle missing contents gracefully and name exactly what is missing.
- A virtual filesystem stays off the roadmap unless measurements of engine start-up reads show it would pay off.
- "Small" versus "big" follows the chunking threshold in `bolt.toml` (decision 009), so it can change without affecting any ID (decision 005).
