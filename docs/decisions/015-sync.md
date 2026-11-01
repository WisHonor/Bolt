# 015 — Syncing: moving pieces by fingerprint

- **Status:** Accepted
- **Date:** 2026-10-31
- **Agenda topic:** 15

## Context

Syncing is how a local copy and a server catch up: `send` uploads new saves, `get` downloads other people's. Game projects mean multi-gigabyte files over unreliable connections, many people, and (later) a hosting service that needs cheap storage. Sync is built in v0.3; this decision sets its direction and what the engine must respect, with a full design pass before v0.3.

## Options

- **The server builds a custom bundle per request** (Git's protocol). Compact on the wire, but heavy server work per request, hard to serve from a CDN, and interrupted transfers usually restart.
- **Move missing pieces by fingerprint** (restic, Hugging Face Xet). A small metadata exchange, then independent chunk transfers to and from cloud storage: resumable, parallel, CDN-friendly, cheap servers; more requests and more client-side bookkeeping.
- **The server tracks each copy** (Perforce). Simple clients and central control, but per-computer server state, and a poor fit for full local history and offline work.

## Decision

**Move missing pieces by fingerprint.**
- **Shape:** first exchange line positions and the missing saves, folder listings and chunk lists (all small); then transfer missing chunks individually, in batches and in parallel, resuming where interrupted. Chunk downloads may come straight from cloud storage.
- **Transport:** HTTPS only. `bolt login` opens the browser; tokens are kept in the OS credential store (Windows Credential Manager, macOS Keychain, Linux secret service), never in plain-text files.
- **Trust nothing:** the client verifies every received piece against its fingerprint before use; the server re-verifies everything uploaded and enforces size limits.
- **Upload dedup:** the client asks which chunks the server already has and sends only the rest.
- **Server storage:** chunks grouped into large packs in object storage (S3 or Cloudflare R2); saves, lines, locks and permissions in PostgreSQL.
- **Dedup boundaries:** identical chunks are shared only within a project or organization, and a client may reference an existing chunk only after proving it has the content, so nobody can guess a fingerprint to read someone else's data.
- **Permissions:** per-project roles (read, write, maintain) and protected lines. Folder-level permissions can come later: a hidden folder behaves like "not downloaded", so fingerprint checks still work, though its name stays visible in the parent listing.
- **Folder remotes:** the same layout can live in a plain folder (USB drive, NAS) for backups and moving between one's own computers; locks are unavailable there. This may ship before the server.
- **Versioning:** client and server announce supported protocol versions and features from the first release.

## Consequences

- The server confirms which data it holds, which makes freeing local space safe (decision 013).
- The server stores and enforces locks (decision 014) and checks permissions.
- Many small requests make batching, connection reuse and parallelism part of the design, not an afterthought.
- Proof of possession and dedup boundaries are part of the threat model (topic 18).
- The full protocol (messages, batching, resumption, errors) is designed before v0.3.
