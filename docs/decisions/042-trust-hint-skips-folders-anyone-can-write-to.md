# 042 — The trust hint never offers a folder that other accounts can write to

- **Status:** Accepted
- **Date:** 2026-11-02
- **Agenda topic:** none (a question left by milestone 1's second red-team session, settled with the milestone 2 plan)
- **Decided by:** the owner, by approving the milestone 2 plan on 2026-11-02, which settled the question. Built in milestone 2 (2026-11-03).
- **Refines:** [decision 026](026-ownership-check-details.md), [decision 027](027-project-folder-must-be-yours.md) and [decision 030](030-administrators-only-when-elevated.md) (their refusals end with a hint that offers `safe-folders`)

## Context

When Bolt refuses a project because another account owns its folder or its `.bolt`, or because it can't tell who does (decisions 025 to 027 and 030), the hint ends with a line to paste into the user's own settings: `safe-folders = ["<the folder>"]`. Listing a folder there trusts it.

Milestone 1's second red-team session saw what that does for a `.bolt` that another account planted in `/tmp`. The hint offers `safe-folders = ["/tmp"]`, and one paste later every Bolt command run below `/tmp` uses a project that anyone could have planted. That is the shape of CVE-2022-24765, the Git bug that the ownership check exists to prevent. Git's own message offers the same line.

It is not only `/tmp`. Any folder where other accounts can add a folder of their own is the same trap: a folder anyone may write to on Unix (the sticky bit of `/tmp` only stops them removing each other's files), and the top of a drive or of a network share on Windows.

## Options

- **A. Keep offering the line,** as Git does. It is one paste from the attack, and the people who paste are the ones the check protects.
- **B. Never offer the line for such a folder, and explain the risk instead.** The user can still write the entry by hand; Bolt only stops suggesting it.
- **C. Refuse such `safe-folders` entries.** The safest, but a project at the top of a drive that only its owner uses could then never be opened, and Git doesn't do it.

## Decision

**B.**

- A folder counts as one that other accounts can write to when:
  - on Unix, the folder itself has the other-write bit set (`/tmp` does);
  - on Windows, it is the root of a drive or of a network share, or a link that leads to one.
- For such a folder the hint offers no line. It says that Bolt doesn't suggest adding the folder, because anyone could have put a project in it that Bolt would then trust, and asks the user to move the project into a folder of their own.
  - A `bolt start` or `bolt import` that stopped after making `.bolt` would add "then run it again to finish"; for such a folder it leaves that off, since the advice is to move.
- Every other folder gets the hint as before.
- Nothing else changes. A `safe-folders` entry the user writes by hand still works, and so does the rule about who must own the folder and its `.bolt`.

## Consequences

- [security.md](../security.md) T23 and [design.md](../design.md) §5.6 state the rule.
- A Git import that refuses a `.git` another account owns gives the same hint, so the same rule applies to it.
- A drive mounted so that every folder looks writable to everyone (WSL's `/mnt/c` can be) always gets the advice to move. The user can still list the folder by hand.
- Tests cover a folder with the other-write bit on Unix, and drive and share roots on Windows.
