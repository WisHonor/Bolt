# 026 — Details of the ownership check, and limits left for later

- **Status:** Accepted
- **Date:** 2026-11-01
- **Agenda topic:** none (refines [decision 025](025-red-team-m1-rulings.md))
- **Refined by:** [decision 027](027-project-folder-must-be-yours.md) (the project folder must be yours too)
- **Partly superseded by:** [decision 030](030-administrators-only-when-elevated.md) (#1: the Administrators group counts only in an elevated terminal, as in Git)

## Context

Building decision 025's ownership check (`plans/2026-11-01-red-team-m1-fixes.md`, Task 8) and reviewing the fixes raised seven questions decision 025 doesn't answer:

1. **"When the user is an administrator" on Windows.** In a normal terminal, Windows gives an administrator a reduced badge (access token): the Administrators group is on it but greyed out ("for deny only"). Folders created in an elevated terminal are owned by the Administrators group.
2. **Drives that record no owner.** FAT and exFAT drives (USB sticks, SD cards) and some network shares have no owner, or one Bolt can't tie to an account. `bolt start` worked there, but every later command said the project "belongs to another user account", which is false.
3. **`bolt start` below a planted `.bolt`.** The plan refused with "belongs to another user account" and a hint to trust the folder. Trusting a planted `C:\.bolt` would hand it every command under `C:\`.
4. **Windows reparse points that are not links,** such as OneDrive and Dropbox placeholders. Bolt refuses them inside `.bolt/` and when saving. Nobody knows yet whether synced folders show them to Bolt.
5. **Path length.** The format limits a save's depth (4,096 levels) and entries (10,000,000), but not a path's length. 255-byte names nested 4,096 deep make paths of about 1 MB. Before the fixes, `bolt show` built every path at once and ran out of memory on a 729 KB repository. It now streams.
6. **Memory inside the limits.** Within both limits, a hostile save can still make a reader hold about 1.5 to 4 GB while walking it.
7. **`sudo bolt`.** In the user's own project, run with `sudo`, Bolt runs as root, so the project counts as someone else's.

## Options

1. **Administrator.** A: the account is an administrator, elevated or not (the group appears on the token, enabled or for deny only). B: only an elevated process counts (`CheckTokenMembership`, Git's choice).
2. **No owner.** A: refuse, with a message that says the drive records no owner and how to trust it, and have `start` say so at once. B: trust drives that record no owner.
3. **`start` below a planted `.bolt`.** A: `start` ignores an enclosing `.bolt` that isn't the user's and creates the project. B: refuse, as planned.
4. **Non-link reparse points.** A: keep refusing until tested on a real synced folder. B: support them now by reopening and checking it is the same file (more `unsafe`).
5. **Path length.** A: decide with milestone 3's write-out, where operating-system path limits apply. B: add a limit to format 1 now.
6. **Memory.** A: accept for now and revisit with the speed-bar benchmarks ([decision 023](023-v0.1-speed-bar.md)). B: lower the format limits now.
7. **`sudo`.** A: a follow-up. B: trust `SUDO_UID` now, as Git does.

## Decision

**A for all seven.**

1. On Windows, a folder owned by the Administrators group counts as the user's when that group is on the user's token, enabled or for deny only. A project started in an elevated terminal keeps working in a normal one. Unlike Git, Bolt doesn't make the user add it to `safe-folders`.
2. When Windows can't tell the owner (none recorded, the owner is "Everyone", or the drive can't answer), the folder is not the user's. Bolt says so plainly: this drive doesn't record who owns it, and listing the folder in `safe-folders` trusts it. `bolt start` checks the `.bolt` it just created and gives that message at once.
3. `bolt start` ignores an enclosing `.bolt` that belongs to another account and isn't in `safe-folders`, and creates the project. Inside the new project, its own `.bolt` is found first. Outside it, commands still refuse the planted one. An enclosing `.bolt` that is the user's still gives "This folder is inside a Bolt project".
4. Non-link reparse points stay refused. [security.md](../security.md) §7 keeps the item open until it is tested on a real synced folder.
5. Format 1 has no path-length limit for now. Readers must never hold every path at once (the streaming diff does this). The limit is decided with milestone 3's write-out, before format 1 freezes at v0.1.
6. The current memory bound is accepted for now and revisited with the speed-bar benchmarks before v0.1.
7. `sudo` trusting `SUDO_UID` is a follow-up for a later milestone.

## Consequences

- Code: the ownership messages and `bolt start` change (a follow-up task on the fix branch).
- [security.md](../security.md) T23 describes the administrator rule, drives without owners, and `start` below a planted folder. It names the remaining risk: a standard user who can rename a folder the Administrators group owns could plant it as `C:\.bolt` for every administrator.
- [design.md](../design.md) §7 lists the path-length and memory questions as deferred, to decide before format 1 freezes.
- [plans/2026-11-01-milestone-1-follow-ups.md](../plans/2026-11-01-milestone-1-follow-ups.md) lists `sudo`, the path-length decision and the memory benchmark.
