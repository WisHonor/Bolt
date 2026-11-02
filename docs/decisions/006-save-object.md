# 006 — The save object: change IDs and recorded moves

- **Status:** Accepted
- **Date:** 2026-10-31
- **Agenda topic:** 6
- **Refined by:** [decision 032](032-exact-repeatable-commit-import.md) (a save imported from Git takes its change ID from its commit)

## Context

A save is a small record that points at the project's top folder listing and says who, when and why. Decisions 002 and 005 already fix most of its fields. Two real choices remained: how change IDs (stable names that survive edits) are generated and shown, and whether saves remember moved files. Moves matter most for big binary files that are moved and edited, for merges, and for locks.

## Options

**Moved files**
- **Never record; guess when needed** (Git). Simple, but guesses fail for moved-and-edited files, especially binaries; merges and locks lose track of them.
- **Record moves Bolt is sure about; guess the rest.** Works with moves made in Explorer or a game engine; detection must be conservative. (Plastic / Unity Version Control also detects moves automatically.)
- **Only explicit moves count** (Perforce's `p4 move`). Precise, but moves made in other tools look like delete + add.

**Change IDs** had few real alternatives (jj's design is the reference), so a single proposal was accepted.

## Decision

**Fields of a save**, all covered by its fingerprint ("save" flavor, decision 005):
- the top folder listing's ID;
- parent saves, in order: none for the first save, one normally, two or more after `combine` (the first parent is the line you were on);
- author and saver (who wrote the change, and who last saved this version), each with name, email, time (milliseconds since 1970, UTC) and time-zone offset;
- the message, stored exactly as written (UTF-8);
- the change ID;
- optional move notes;
- a reserved signature slot;
- a list of extra fields, sorted by name, that older versions of Bolt keep but ignore.

**Change IDs**
- 128 random bits, created with the save. Kept when the save is edited or moved to another line; a duplicated save gets a new one.
- Displayed only in the letters k–z (16 letters, one per hex digit, as in jj), so they can never be confused with fingerprints, which use 0–9 and a–f.
- Screens show the shortest unique prefix, at least 4 letters.
- Two visible saves sharing a change ID are marked "divergent" until the user keeps one.

**Moved files**
- When saving, Bolt records a move only when it is certain: the same contents disappeared at one path and appeared at another, or a game-engine signal such as a Unity `.meta` file moving with its asset.
- Diffs, merges and locks use move notes when present and fall back to guessing otherwise.
- Move notes never change what a save contains: the snapshot stays a full photo.

## Consequences

- Change IDs only pay off with commands that edit past saves (amend, reword, move between lines); which ones exist is settled in topic 17.
- Divergence mostly arises once syncing exists (v0.3); v0.1 only needs to detect and show it.
- Git export (decision 002) needs rules for change IDs and move notes, which Git can't hold natively.
- Move detection is engine-aware through presets (topic 16) and must be conservative: a wrong move note is worse than none.
- Signatures can be added later without a format change; the signed bytes are the save without its signature slot.
