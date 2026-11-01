# 007 — Conflicts as data: keep every version

- **Status:** Accepted
- **Date:** 2026-10-31
- **Agenda topic:** 7

## Context

Decision 003 lets a save contain unresolved conflicts, so users can keep working and fix them later. This decision sets what a stored conflict contains. Conflicts are built in v0.2, but their shape is part of the storage format, so it is designed now. Game projects add two constraints: binary files can't hold conflict markers, and some text files (Unity scenes, prefabs, `.meta` files) break when markers are written into them.

## Options

- **Markers only.** Store the file with markers plus a list of conflicted paths, which is what committing markers in Git gives. Simple, but loses the original versions, can't represent binary files, and can't tell user edits from markers.
- **Keep every version.** A conflict entry points to a record with the common ancestor(s), each side (or "absent") and the user's half-fixed version. Lossless, and works for binaries. (jj stores conflicts this way.)
- **Keep every version, plus conflict algebra.** jj-style automatic simplification when saves are rebuilt on top of each other. Most powerful; much more complex.

## Decision

**Keep every version now; add conflict algebra on top later.**
- A folder-listing entry can be of kind "conflict", pointing to a conflict record with its own fingerprint flavor (decision 005).
- The record holds a general list of terms: one or more sides and the ancestors between them (two sides and one ancestor in the common case, more after multi-way combines). Any term can be "absent" (deleted, or never existed), and terms can be of different kinds (file, folder, symlink).
- The record can also hold the user's half-fixed version, so saving mid-resolution never loses work.
- **Text files:** standard markers with the ancestor section (`<<<<<<<`, `|||||||`, `=======`, `>>>>>>>`), labelled with line names and change IDs.
- **Binary files:** the version from the user's own line stays in place, so the project keeps opening in the game engine; the other versions stay inside `.bolt/`, never as loose files in the project. A command opens any version in its usual app or keeps it.
- **Per-file rule:** presets decide which files get markers; the Unity preset treats scenes, prefabs and `.meta` files like binaries.
- **Fixed** means the markers are gone, or the user marks the file as fixed.
- Move notes (decision 006) let Bolt apply an edit to a moved file instead of reporting a conflict.
- A conflicted save is a normal save: it can be checked out, built on and combined; its conflicts stay listed until fixed.

## Consequences

- **Conflict algebra is planned** (owner's decision): jj-style automatic simplification when saves are rebuilt on top of each other. The general term list was chosen so it can be added without a format change.
- Bolt must read the folder back to detect fixed files, including parsing its own markers (topic 11).
- Git export (decision 002) needs a rule for conflicted saves, for example markers for text files and the user's side for binaries.
- Markers and side versions are written out under the same safety rules as everything else (decision 004).
- Marker labels and the commands for choosing versions are settled in topic 17; merge behaviour itself in topic 12.
