# 030 — The Administrators group counts only in an elevated terminal, as in Git

- **Status:** Accepted
- **Date:** 2026-11-02
- **Agenda topic:** none (the owner's review of decisions 026 to 029, before milestone 2)
- **Supersedes:** [decision 026](026-ownership-check-details.md) #1
- **Decided by:** Claude, at the owner's request ("Do what you think is best"). The same review kept decisions 027, 028 and 029 unchanged.
- **Refined by:** [decision 042](042-trust-hint-skips-folders-anyone-can-write-to.md) (the hint never offers `safe-folders` for a folder other accounts can write to)

## Context

Decision 026 #1 counts a folder owned by the Administrators group as the user's whenever the user is an administrator, elevated or not: in a normal terminal the group is on the user's token "for deny only", and Bolt counted that too. Git for Windows counts the group only when it is enabled, that is, in an elevated terminal (`CheckTokenMembership`).

With decision 027 the laxer rule widens the planted-repository gap (security.md T23). Every signed-in user can rename folders inside the folders made directly under `C:\`, and at the root of many second drives. If an administrator made such a folder in an elevated terminal, another account can rename a folder inside it to `.bolt` and fill it, and the administrator's commands below it, outside their own projects, use that repository even in a normal terminal. The attack needs a second account on the same computer.

## Options

- **A. Keep decision 026 #1.** A project started in an elevated terminal keeps working in a normal one. Laxer than Git.
- **B. Git's rule.** The group counts only while it is enabled on the token. A project whose folders the Administrators group owns then opens in a normal terminal only once it is listed in `safe-folders` (Git users meet this as "detected dubious ownership").
- **C. B, and an elevated `bolt start` makes the user the owner of the new `.bolt`.** Smoother for someone who runs only `bolt start` as administrator, but it needs new `unsafe` code in the platform crate to set an owner, and a project folder itself made in an elevated terminal still needs `safe-folders`.

## Decision

**B.**

- On Windows, a folder owned by the Administrators group counts as the user's only while that group is enabled on the user's token, as Git checks it. In a normal terminal such a folder is refused, with a message that names the Administrators group and the two ways on: an administrator terminal, or the folder in `safe-folders`.
- When an elevated `bolt start` makes a project whose folder or `.bolt` the Administrators group owns, it says that the project will open only from an administrator terminal unless the folder is listed in `safe-folders`.
- C is left out: it covers only `.bolt`, and adds `unsafe` code for a convenience nobody has asked for yet.

## Consequences

- [security.md](../security.md) T23 and [design.md](../design.md) §5.6 state the rule; the remaining-risk note no longer has an administrator case of its own.
- A project made in an elevated terminal before this change needs an administrator terminal or a `safe-folders` entry afterwards. Repositories made by development builds are disposable anyway ([decision 019](019-review-clarifications.md)).
- The code change is a task in milestone 2's plan.
