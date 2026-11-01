# 014 — File locking: whole-project locks now, smart locks later

- **Status:** Accepted
- **Date:** 2026-10-31
- **Agenda topic:** 14

## Context

Binary assets such as textures and engine scenes can't be merged: if two people edit one, someone's work is lost. Locks prevent this by letting one person claim a file. Locks need a server, so they are built in v0.3; v0.1 only stores the rules. The main question is how far a lock reaches when a project has several lines.

## Options

- **A lock covers one line** (Perforce, where lines are separate folders). Never blocks unrelated work, but conflicts return when lines are combined.
- **A lock covers the whole project** (Git LFS by default). Simple, and truly prevents lost work, but can block unrelated work on other lines.
- **A lock follows the change** (Unity Version Control's smart locks). Holds on every line until the change reaches the integration line, then releases itself. The strongest option, but more complex, and abandoned changes leave locks behind.

## Decision

**Whole-project locks in v0.3; locks that follow the change are planned later.**
- Lock records hold the file (followed across moves via move notes, decision 006), the line it was taken on, the holder, the time and a message, so smart locks can grow out of them.
- **Lockable files** are listed as patterns in `bolt.toml` (decision 008); presets fill them in.
- **Read-only until locked:** lockable files stay read-only on disk until the user holds the lock, so editors complain at the first save. Users can turn this off in personal settings.
- **Visibility:** `bolt lock <file> "<message>"` takes a lock; `bolt status` shows who holds which lock, since when, and why.
- **Release:** automatic when the holder sends the change; a flag keeps the lock.
- **Enforcement:** the server refuses changes to files locked by someone else; `bolt save` warns earlier.
- **Forgotten locks:** project maintainers can break a lock; the holder is notified, and their unsent changes stay safe locally.
- **Offline:** locks need the server. On request, Bolt lets the user edit anyway, warns that they don't hold the lock, and tries to take it when back online.
- **v0.1:** patterns only; nothing is enforced without a server.

## Consequences

- The server (topic 15) needs lock storage, permission checks for taking and breaking locks, and enforcement when receiving saves.
- Undoing `lock` or `unlock` performs the matching server action rather than a local rewind (decision 010).
- Making files read-only is a working-folder behaviour, not something stored in history (decision 004).
- Smart locks need the server to know whether a change has reached the integration line (an ancestry check) and a rule for changes that never get there; both are designed later.
- The exact behaviour of Unity Version Control's smart locks should be checked before ours are designed.
