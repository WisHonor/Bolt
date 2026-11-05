# 054 — A partial save names files, folders and patterns

- **Status:** Accepted
- **Date:** 2026-11-05
- **Agenda topic:** none (milestone 3 planning, question 7)
- **Decided by:** Claude, at the owner's request ("do whatever you think is best", 2026-11-05). The owner can reverse any part.
- **Refines:** [decision 003](003-working-copy-model.md) (`bolt save <files or folders>`) and [decision 017](017-commands-and-output.md) (`save "msg" [files]`)

## Context

`bolt save "msg"` saves every change; decision 003 adds `bolt save "msg" <files or folders>`, which saves only those, with choosing parts of a file left for later. Left open: what can be named, and what happens to names that match nothing or that an ignore rule keeps out.

## Options

- **A. Paths only:** files and folders, relative to the current folder. Git (`git commit <paths>`), Mercurial (`hg commit FILES`) and Jujutsu (`jj commit PATHS`).
- **B. Paths and patterns** such as `*.cs` or `Assets/**/*.png`, matched by Bolt. Windows' shells pass `*` to a program as it is, so on Windows only the program can expand it; Git matches its own pathspecs, and Mercurial takes `glob:` patterns.
- **C. Interactive picking,** later, with parts of files.

## Decision

**B.**

- **Arguments:** `bolt save "message" <path or pattern>…`. A path is relative to the current folder; a folder means everything in it. An argument holding `*`, `?` or `[` is a pattern, matched against project paths with Bolt's Git-exact wildmatch (`**` included), relative to the current folder.
- **What is saved:** every change at the named paths, deletions included; everything else stays as the save it builds on has it, and the folder keeps it as unsaved work.
- **Refused, saving nothing:** an argument that names nothing (no file in the folder or in the last save), a path outside the project, and a new file an ignore rule keeps out, named outright (as `git add` refuses one); the hint says to change the rule.
- Every other rule of a save applies to what is named: ignore rules inside a named folder, the question before a large save (decision 056), entries kept as they were.

## Consequences

- The CLI's `save` takes paths after the message; `--json` lists them.
- Tests cover a file, a folder, a pattern, a deletion, a pattern matching nothing, and an ignored file named outright.
