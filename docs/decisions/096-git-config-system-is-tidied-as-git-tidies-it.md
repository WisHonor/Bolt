# 096 — `GIT_CONFIG_SYSTEM` is tidied as Git tidies it

- **Status:** Accepted
- **Date:** 2026-11-09
- **Agenda topic:** none (milestone 5's red-team fixes)
- **Decided by:** Claude, at the owner's request while the owner was away. The owner can reverse any part.
- **Refines:** [decision 095](095-a-git-setting-naming-dev-null-names-no-file.md) (which value is judged against `/dev/null`) and [decision 094](094-red-team-m5-rulings.md) (`GIT_CONFIG_SYSTEM` follows Git's rules)

## Context

Decision 095 judges a value against `/dev/null` exactly as it is written. The review of its code found that Git tidies one of these values first: `GIT_CONFIG_SYSTEM`. It runs the value through `normalize_path_copy` before comparing it with `/dev/null` and before opening the file (Git v2.56.0, config.c:1513). That tidying:
- turns `\` into `/` on Windows;
- folds repeated separators into one;
- drops `.` parts, and folds `..` parts into the part before them.

So on Windows, `\dev\null`, `/dev//null` and `/dev/./null` are all the null device to Git. Bolt took them as paths, joined them to the project's drive, and read `C:\dev\null` if it existed. That is the harm decision 095 removed for the plain spelling. Git doesn't tidy `GIT_CONFIG_GLOBAL`, so Bolt already matched Git there.

Decision 095's context also says that Git's documentation gives `core.excludesFile = /dev/null` as a way to switch the list off. It doesn't: Git documents `/dev/null` only for `GIT_CONFIG_GLOBAL` and `GIT_CONFIG_SYSTEM` (git.adoc:772). Git still reads `core.excludesFile = /dev/null` as an empty list, so 095's rule stands.

## Options

- **A. Tidy `GIT_CONFIG_SYSTEM` as Git does,** then judge and open the tidied value.
- **B. Leave it,** and name it in the follow-ups. The harm takes an odd spelling of `/dev/null` and a planted `C:\dev\null`.

## Decision

**A.**
- Bolt tidies `GIT_CONFIG_SYSTEM` as Git v2.56.0's `normalize_path_copy` does, before it judges the value against `/dev/null` (decision 095) and before it opens the file.
- Where the tidying fails, as on a `..` that climbs above the value's start, Git ignores the failure and goes on with a half-tidied path (config.c:1513 doesn't check the result). Bolt stops instead, naming the variable.
- No other value is tidied, since Git tidies no other.

## Consequences

- The tidying is ported once, into crates/bolt-vcs-core/src/config.rs, with Git's own cases as unit tests on every system, the Windows ones included.
- storage-format.md §12 says it.
