# 027 — The project folder must be yours too, as Git requires

- **Status:** Accepted
- **Date:** 2026-11-01
- **Agenda topic:** none (refines [decision 025](025-red-team-m1-rulings.md) and [decision 026](026-ownership-check-details.md))
- **Decided by:** Claude, while the owner was away, under the owner's instruction of 2026-11-01 to finish the milestone and merge. The owner can reverse it.

## Context

Decision 025 refuses "a repository whose `.bolt` folder is owned by another user account, as Git does". A review of the fix branch found that checking only `.bolt` falls short of Git's rule on Windows.

On a default Windows install, every signed-in user may create folders directly under `C:\` and may change folders other users create there. Another account can therefore rename a folder the *victim* made into `C:\.bolt` and fill it with a repository. Its owner is the victim, so the `.bolt` check passes, and every command the victim runs outside a project on that drive uses the planted repository.

Git checks the folder that holds the repository as well as the repository data, so a `C:\.git` is refused whoever owns it.

## Options

- **A.** Also require the project folder (the folder that contains `.bolt`) to be the user's, by the same rules. This is Git's rule.
- **B.** Keep checking `.bolt` only, and document the gap.

## Decision

**A.**

- Bolt uses a project only when both the project folder and its `.bolt` folder pass the ownership rules of decisions 025 and 026: the user's own account, or the Administrators group for an administrator, elevated or not. Listing the project folder in `safe-folders` trusts both.
- Both owners are known before Bolt looks inside `.bolt`.
- `bolt start` refuses at once in a folder that isn't the user's, with the same hint.

## Consequences

- `C:\.bolt` is refused whoever owns it, because `C:\` belongs to the system, not the user. The same goes for any other folder the user doesn't own.
- Projects in folders the user doesn't own, such as a shared team folder or a drive root owned by an administrator, need a `safe-folders` entry, as with Git's `safe.directory`.
- [security.md](../security.md) T23 states the rule.
