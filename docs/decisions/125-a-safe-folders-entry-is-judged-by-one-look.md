# 125 — A `safe-folders` entry is judged by one look, and a dropped switch is told even when the pick-up fails

- **Status:** Accepted
- **Date:** 2026-11-10
- **Agenda topic:** none (milestone 6 build, the review of the follow-ups)
- **Decided by:** Claude, at the owner's request while the owner was away. The owner can reverse any part.
- **Refines:** [decision 123](123-follow-ups-done-or-moved-past-v0.1.md) (items 5 and 13), and through it decisions [026](026-ownership-check-details.md) and [042](042-trust-hint-skips-folders-anyone-can-write-to.md)
- **Refined by:** [decision 149](149-a-drive-that-cant-name-its-folders-is-told-not-offered-safe-folders.md) (where Bolt can't read a folder's identity, the entry is named as passed over and no `safe-folders` line is offered)

## Context

Decision 123 said two things that the review of their code showed were not yet whole.

- **Item 13** says a `safe-folders` entry that is itself a link (a symbolic link, or a junction on Windows) no longer counts, so that whoever can replace a trusted folder with a link can't move the trust to the link's target. As first built, Bolt looked at the entry twice: once following it, to see whether it is the project's folder, and once more to see whether it is a link. Someone who can replace the trusted folder can swap it back and forth between a link to their own project and a real folder, and is trusted whenever the first look meets the link and the second the folder. Three more things came up:
  - an entry written with `..` in it (`/s/trusted/x/..`) is resolved by the system through a link at `trusted`, so it got past the rule;
  - on Windows a link can be more than a symbolic link or a junction: a mounted volume and other reparse points behave the same;
  - once folders are told apart by the identity the drive gives them, a drive that gives none (some WebDAV and SMB shares report a file number of 0 for everything) makes every folder look like every other.
- **Item 5** says that picking up a lost `head` drops a stopped switch's journal and says so in a `switch_dropped` warning. As first built, the warning was part of the pick-up's result. When the pick-up failed after it had removed the journal (a disk that filled up, a flush that failed), the journal was gone and nothing said so; the second try found no journal and never mentioned the switch.

## Options

For the entry:

- **A. Two looks, as first built.** Small, and wrong against exactly the person the rule is for.
- **B. One look.** Open the entry once without following its last name; refuse a link there; compare the folder the handle holds with the project's folder by the identity the drive gives.
- **C. Keep comparing paths, but hold the entry open so it can't be swapped.** It works on Windows, where an open folder can be kept from being renamed; on Unix a folder can always be renamed away from under a handle, so it would be two rules.

## Decision

**B, with these rules.**

- **One open, not following the last name.** If what is there is a link, the entry doesn't count: a symbolic link on any system; on Windows also a junction, a mounted volume or any other reparse point. Whether it is a link is read from that same handle, never from the path again.
- **Compared by identity.** The folder the handle holds is the project's folder when the two have the same identity: device and file number on Linux and macOS, volume serial number and file ID on Windows. Bolt never looks the entry's path up a second time to decide.
- **No identity, no trust.** A file number of 0 means the drive doesn't tell its folders apart, and never counts as a match. On such a drive `safe-folders` can't vouch for a folder, and the refusal says so. The way on is the usual one for a folder Bolt won't trust: work on a copy on a drive of your own, or as the account that owns the folder.
- **An entry with `..` in it doesn't count,** whatever it points at. Before, such an entry counted when it led to the project (a test pinned `sibling/../project`); it no longer does.
- **A link in a part above the entry's last name is still followed,** as before: the rule is about the entry itself. Someone who can replace a folder above a trusted folder could always put a real folder of their own at the trusted path, which counts by design.
- **The refusal names each entry it passed over and why** (a link, with the words that fit the system; `..`; no identity), escaped like any text from the settings.
- **`switch_dropped` is told as soon as the journal has been removed,** and is one of the warnings shown before an error (with `head_not_flushed`), so it reaches the person whether the pick-up then succeeds or fails. It is told once.

## Consequences

- No new `unsafe`: the platform crate already opened folders without following links and already read a file's identity; the engine now uses both for this check.
- On a drive that gives no file identity, a project owned by someone else, or whose owner can't be read, can't be opened through `safe-folders`. This is new: the path comparison it replaces worked there. It fails toward refusing.
- A `safe-folders` entry the user can search but not read no longer counts, since it can't be opened. Bolt couldn't work in such a folder anyway.
- [security.md](../security.md) T23 and the user guide's `safe-folders` section say all of this; [json.md](../json.md)'s list of warnings shown before an error gains `switch_dropped`.
