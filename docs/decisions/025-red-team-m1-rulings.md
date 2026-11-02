# 025 — Rulings after milestone 1's red-team session

- **Status:** Accepted
- **Date:** 2026-11-01
- **Agenda topic:** none (follows [decision 024](024-red-team-gate.md))
- **Refined by:** [decision 026](026-ownership-check-details.md) (details of the ownership check, and limits left for later)

## Context

The first red-team session (decision 024) found 3 critical, 4 high, 2 medium and 2 low problems in milestone 1. Most are fixed by code alone. Three need the owner's decision, because they change a decided behaviour or the storage format:

- **RT-9:** a project's `.bolt/config.toml` can set the name and email on the user's saves. Decision 008 counts `.bolt/config.toml` as personal, but a project shared as a zip carries it, so a stranger can make the user's saves claim to be from someone else.
- **RT-6 and RT-10:** the format limits how wide one folder listing may be, but not how deep folders nest or how many files one save describes in total. A 100,000-deep save overflows the stack; three shared 1,000-entry listings describe a billion files in 3.6 KB.
- **Unproven:** a `.bolt` folder planted in a parent folder (such as `C:\` or `/tmp`) by another account would capture every command run in folders below it. Git had this bug (CVE-2022-24765) and fixed it with an ownership check.

## Options

**Identity (RT-9).**
- A. Ignore identities in project settings unless the user approves them for that project.
- B. Keep them, but print "saving as X (from this project's settings)" on every save.
- C. Remove per-project identities.

**Limits.** Depth and total-size limits on one save, or limits only in the reader's code.

**Planted parent repository.** Copy Git's ownership check now, or defer it.

## Decision

**Identity: A.** Bolt never takes the user's name or email from a project's settings.
- The order is: `BOLT_USER_NAME` and `BOLT_USER_EMAIL`; then a `[[project]]` entry in the user's own `config.toml` whose `folder` is this project's folder; then the `[user]` section of the user's own `config.toml`.
- Approving a per-project identity means adding it to the user's own settings, which no project can change:

  ```toml
  [[project]]
  folder = 'C:\Users\me\Projects\Work'
  name = "Work Name"
  email = "me@work.example"
  ```

- When a project's `.bolt/config.toml` names an identity, Bolt ignores it and says so on `save`, showing the lines above filled in, so the user can approve it on purpose.
- Decision 008 is otherwise unchanged: `.bolt/config.toml` stays a home for personal settings that cannot affect anyone else.

**Limits: yes**, added to [storage-format.md](../storage-format.md) §3 as rules for a whole save:
- at most **4,096** folder levels below the root (Git's default limit is 2,048);
- at most **10,000,000** entries in total, counting every path (a listing reached twice counts twice).

Readers that walk a save stop with an error as soon as either limit is passed, and walks never recurse once per folder level.

**Planted parent repository: yes.**
- Bolt refuses to use a repository whose `.bolt` folder is owned by another user account, as Git does.
- On Windows, a folder owned by the Administrators group counts as the user's own when the user is an administrator.
- The user can trust a specific folder anyway by listing it in their own settings (`safe-folders = ['…']`).
- The ownership checks are operating-system calls, so they live in the platform crate and the owner reviews them line by line.

## Consequences

- Five threats are added to [security.md](../security.md) §4: T19 (links and special files inside `.bolt/`), T20 (files swapped during a save), T21 (terminal spoofing), T22 (identity claimed by a project), T23 (a repository planted in a parent folder). Tree bombs and deep nesting join T7 and T11.
- The fixes are planned in [plans/2026-11-01-red-team-m1-fixes.md](../plans/2026-11-01-red-team-m1-fixes.md). Every red-team proof test becomes a permanent test, and a short second session checks the fixes before milestone 2 starts.
- These limits are part of format 1, so they freeze at v0.1 with the rest of the format.
