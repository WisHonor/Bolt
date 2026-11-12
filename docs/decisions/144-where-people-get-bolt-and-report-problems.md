# 144 — Where people get Bolt and report problems

- **Status:** Accepted
- **Date:** 2026-11-11
- **Agenda topic:** none (the v0.1 release: the package)
- **Decided by:** Claude, at the owner's request while the owner was away. The owner can reverse any part. The two addresses are the owner's.
- **Supersedes:** [decision 119](119-packaging-v0.1.md) in part (people got Bolt from the repository's release page, and `SECURITY.md` sent reports through GitHub's private reporting)
- **Refines:** [decision 024](024-red-team-gate.md) (`SECURITY.md` says how to report a problem in private)
- **Refined by:** [decision 151](151-opalro-com-and-support-at-opalro-com.md) (its two addresses: https://opalro.com and support@opalro.com)

## Context

**In plain words.** Once the repository is private ([decision 140](140-bolt-is-closed-source.md)), only the owner can open it. Every link into it then leads a reader to a "page not found". Today the README, the guide, `SECURITY.md` and the release notes send people there four ways: to the release page to download, to the issues page to report a bug, to the Security tab to report a vulnerability, and to files in the repository to read more.

**The owner's facts.** People will download Bolt from the owner's website, at an address the owner gave as `<DOWNLOAD ADDRESS>`, and report problems, security problems included, at `<REPORT ADDRESS>`. Neither was filled in on 2026-11-11.

## Options

- **A. Keep the links to GitHub.** They break the day the repository goes private.
- **B. Send people to the download page and the report address,** in words that still work while the addresses aren't filled in.
- **C. B, and a test that fails when something that ships in a build or is meant for the website points into the repository.**

## Decision

**C.**

- **Where people get Bolt:** the owner's download page. While `<DOWNLOAD ADDRESS>` is unfilled, the documents say "the page you downloaded Bolt from" and name no address. The repository's draft release stays the owner's private staging place for the files; nobody else sees it.
- **Where people report a problem, security problems included:** the report address. While `<REPORT ADDRESS>` is unfilled, the documents say "the address on the page you downloaded Bolt from". A security problem goes to the same address, privately, with "security" in the subject, and is kept private until it is fixed or ruled on, as `SECURITY.md` asked before.
- **What ships in an archive** points only at what is in that archive: the README links the guide and the user agreement, and names the notices files; nothing links a file that doesn't ship. The README is written for people who downloaded Bolt: building from source, which nobody outside can do now, leaves it.
- **What is meant for the website** (the download page's text, the welcome note, how to report, the known problems, the guide, the user agreement, `SECURITY.md` and the release notes) reads on its own, with no link into the repository. The `--json` reference (`docs/json.md`, which the guide sends script writers to), the storage format and the benchmarks link into the repository throughout; the release workflow makes copies of them with those links turned into plain text, and the page for the owner says what an outside reader loses in each ([decision 145](145-one-release-workflow-for-every-version.md)).
- **The checks:**
  - a test (`crates/bolt-vcs/tests/guide.rs`) fails when the README, the guide, `SECURITY.md`, `USER-AGREEMENT.md` or a file under `docs/releases/` holds the repository's address, or a relative link to a file that doesn't ship beside it;
  - the release workflow fails when anything in an archive, or among the website copies, holds the repository's address.
- **When the owner fills in the addresses,** each place to change is a line holding "the page you downloaded Bolt from" or "the address on the page you downloaded Bolt from"; the page for the owner lists the files.

## Consequences

- No one needs GitHub to get Bolt, read about it or report a problem.
- `SECURITY.md` stays in the repository as the security policy the website can publish; GitHub's private vulnerability reporting, which works only for public repositories, is no longer used.
- Until the owner fills in the two addresses, the documents are vaguer than they should be ("the page you downloaded Bolt from"); the page for the owner puts the report address first among what is missing.
- `docs/benchmarks.md` is the engine session's file and isn't changed here; its website copy loses its links to workflow runs and to `bench/run.sh`, which the page for the owner lists.
