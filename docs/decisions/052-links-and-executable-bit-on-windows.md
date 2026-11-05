# 052 — Links and the executable bit when files are written

- **Status:** Accepted
- **Date:** 2026-11-05
- **Agenda topic:** none (milestone 3 planning, question 5)
- **Decided by:** Claude, at the owner's request ("do whatever you think is best", 2026-11-05). The owner can reverse any part.
- **Refines:** [decision 004](004-project-contents.md) (placeholders for links the system can't make; Windows keeps history's executable flag) and [decision 045](045-link-targets-stored-with-slashes.md) (link targets)

## Context

Windows has no executable bit, and making a link there needs Developer Mode or an administrator; it also needs to know whether the link points at a folder or a file. Decision 004 already says links that leave the project, or that the system can't make, are written as placeholder files holding their target, with a warning, and that on Windows a file keeps the executable flag history gives it. Git's published bugs add rules: make links last, and never look at what a link points to on disk.

## Options

- **A. Real links where the system lets this account make them, else placeholders.** A Windows link is a folder link when its target is a folder in the save being written. Git for Windows makes real links when `core.symlinks` is on and decides the kind from the disk; Jujutsu makes only file links on Windows, so folder links there don't work.
- **B. Always placeholders on Windows,** as Git for Windows does by default (`core.symlinks=false`).
- **C. Real links only,** refusing when the system can't make them.

## Decision

**A.**

- **Links** are made after every file and folder (CVE-2021-21300 and CVE-2024-32002 used a link made first). A link whose target leaves the project, judged from the save alone (an absolute target, or `..` past the top), or that the system won't make, is written as a placeholder file holding its target, with one warning per command. On Windows the target is written with `\` (decision 045), and a stored target holding a `\` is never written as a link.
- **Folder or file link on Windows:** a folder link when the target, resolved within the save being written, is a folder; otherwise a file link. Bolt never looks at the target on disk (Git for Windows' NTLM leak; gitoxide GHSA-xw69).
- **The executable bit:** on macOS and Linux every file is written with mode 0644 or 0755 exactly (gitoxide CVE-2025-22620), whatever the old file had. On Windows nothing is set, and saves keep the flag history gives each file (decision 004). A new file made on Windows isn't executable; a way to mark one waits for a later milestone.

## Consequences

- The save side already keeps a placeholder Git wrote as a link (decision 039); placeholders Bolt writes take the same form.
- Tests on Windows run with and without the right to make links; CI's Windows runner has it.
