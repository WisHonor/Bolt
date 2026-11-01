# 008 — Repository layout and where settings live

- **Status:** Accepted
- **Date:** 2026-10-31
- **Agenda topic:** 8

## Context

The project's rules (what is chunked, lockable, ignored, how conflicts are shown) must reach everyone who copies the project. They arrive from strangers, so they may only describe things, never run anything. Personal settings must never be shared. The `.bolt/` folder needs a clear structure, and older copies of Bolt must never damage repositories made by newer ones.

## Options

- **One shared file** (`bolt.toml`) holding all project rules, including the ignore list. One place to look, but ignore lists get long and Git users expect a separate ignore file.
- **A settings file plus an ignore file** (`bolt.toml` + `.boltignore` in `.gitignore` syntax). Familiar, and ready-made ignore lists work as-is. (Git: `.gitattributes` + `.gitignore`; Mercurial: `.hgignore`.)
- **Rules inside `.bolt/`, shared through a server.** Clean project folder, but rules aren't part of history and sharing needs a server. (Perforce's typemap; Plastic's lock rules.)

## Decision

**`bolt.toml` + `.boltignore` at the project root, versioned like any other file.**
- `bolt.toml` (TOML) holds the project rules: preset, chunking, lockable files, conflict-marker rules, large-save threshold. It is purely declarative and can never name a program to run.
- `.boltignore` uses exactly the `.gitignore` pattern syntax. Git import copies an existing `.gitignore` into it.
- Unknown keys in `bolt.toml` (from a newer Bolt) produce a warning and are otherwise ignored.

**Two kinds of settings, never mixed**
- *Project rules* live only in the two shared files.
- *Personal settings* (identity, editor, merge tools, display) live in the user's config folder or `.bolt/config.toml` and are never shared.
- Personal settings cannot change project rules. Anything that runs a program can only be a personal setting.

**Layout of `.bolt/`**
- `format`: the format version and the list of features the repository uses;
- `store/`: all saved data (precious; internals in topic 9);
- `state/`: lines and the operation log (precious; internals in topic 10);
- `cache/`: speed-ups only; safe to delete at any time;
- `tmp/`: files being written, on the same drive as the project so write-then-rename is atomic; cleaned on start;
- `config.toml`: personal settings for this project.

**Format and upgrades**
- A Bolt that finds a feature it doesn't understand in `format` refuses to open the repository and asks to be updated.
- A newer Bolt always opens older repositories but never upgrades their format by itself; it suggests upgrades, and `bolt setup` performs them with crash-safe writes and a verification step.
- On Windows, `.bolt/` is marked hidden, like Git's `.git/`.

## Consequences

- The `bolt start` screen writes `bolt.toml` and `.boltignore`, and `bolt setup` edits them. Both appear in status and history like normal files, and older saves carry their own rules.
- Project rules are untrusted input: parsing them gets fuzz tests and malicious-repository cases (topic 18).
- Everything in `cache/` must be rebuildable; tests check this by deleting it.
- Which rules apply when you `goto` an old save or combine lines with different rules is settled with presets in topic 16.
