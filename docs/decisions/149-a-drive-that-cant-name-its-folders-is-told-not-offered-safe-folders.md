# 149 — A drive that can't name its folders is told so, not offered `safe-folders`

- **Status:** Accepted
- **Date:** 2026-11-11
- **Agenda topic:** none (the v0.1 release: the package)
- **Decided by:** Claude, at the owner's request while the owner was away. The owner can reverse any part.
- **Refines:** [decision 125](125-a-safe-folders-entry-is-judged-by-one-look.md) (an entry that can't count is named in the refusal, with why)

## Context

**In plain words.** Bolt only works in a project it can tell is yours. A USB stick formatted FAT32 or exFAT records no owners, so Bolt asks you to vouch for the folder by listing it in `safe-folders`, and the guide says that is what such drives need. The package session tried it on GitHub's Windows runner (run 43053321602): on a FAT32 and an exFAT drive, `bolt start` refused, offered the line `safe-folders = ['R:\proj']`, and with that exact line added refused again with the same words and the same offer. Nothing said why the line didn't count.

**Technically.** Since decision 125 an entry counts only when the folder it opens has the same identity (volume and file number) as the project's folder. When Bolt can't read that identity at all, `owner_allows` (`repo.rs`) passes the entry over with no note (`_ => continue`), though decision 125 says every entry passed over is named with why; and the hint that offers the line doesn't know the drive can't take it. The refusal itself is right: it fails toward refusing.

## Options

- **A. Make such drives usable** (trust a FAT folder by its path, say). That widens what Bolt trusts, in code that decides whose project Bolt opens: it wants its own design and a red-team round.
- **B. Keep refusing, and say so truthfully:** name the entry and why it can't count, and offer no `safe-folders` line where none can work; tell people to work on a copy on an internal drive.

## Decision

**B for v0.1; A comes later, with its own design and red-team round.**

- When Bolt can't read a folder's identity on the drive (no file number, or none the drive will report), a `safe-folders` entry that leads to the project is named as passed over, with the words decision 125 already gives: "its drive doesn't tell its folders apart, so safe-folders can't vouch for a folder on it".
- On such a drive the refusal's hint offers no `safe-folders` line; it says to work on a copy on a drive that records owners (the computer's own drive).
- What Bolt trusts doesn't change: every case refused before is refused now. So this needs no red-team session of its own; the owner may ask for one.
- The guide's "Personal settings" section and the known problems say that on Windows a project on a FAT32 or exFAT drive (most USB sticks and SD cards) can't be used in v0.1, and how to work around it.

## Consequences

- People with a project on a USB stick get a true answer instead of a line that does nothing.
- Making those drives usable is a row of the follow-ups for a later release, not v0.1.
- How Bolt behaves on FAT and exFAT on Linux and macOS isn't changed here; the package session's try on Linux didn't run (the runner couldn't reach Ubuntu's package mirror), and is on the owner's page.
