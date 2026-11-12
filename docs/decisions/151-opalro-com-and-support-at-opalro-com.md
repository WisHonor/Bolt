# 151 — opalro.com, and support@opalro.com

- **Status:** Accepted
- **Date:** 2026-11-12
- **Agenda topic:** none (the v0.1 release)
- **Decided by:** the owner.
- **Refines:** [decision 144](144-where-people-get-bolt-and-report-problems.md) (gives its two addresses)

## Context

**In plain words.** Decision 144 sends people to the owner's download page and report address, and has the documents say "the page you downloaded Bolt from" until the owner gives both. On 2026-11-12 the owner bought opalro.com ([decision 150](150-the-name-is-opalro.md)). An address that goes into a build stays in it: an archive can't be called back. If the address later stops working, or passes to someone else, it keeps sending the people who hold that build to the wrong place.

## Options

- **A. The site's root, https://opalro.com, and one mailbox, support@opalro.com,** for problems and security problems alike, as decision 144 has it. The root keeps working however the site's pages change.
- **B. A page of the site,** such as https://opalro.com/download. More direct, but every build keeps the path after the site is rearranged.
- **C. A second mailbox for security problems,** security@opalro.com, as many projects have. Decision 144 chose one address, with "security" in the subject; a second is one more mailbox to watch.

## Decision

**A.** People get Opalro at **https://opalro.com**, and report problems, security problems included, to **support@opalro.com**. A security problem says "security" in its subject and stays private until it is fixed or ruled on, as decision 144 says.

- The documents give both wherever they said "the page you downloaded Bolt from" or "the address on the page you downloaded Bolt from". The download page gives the report address on the page itself.
- The address is plain text, never a `mailto:` link, which the guide's test refuses.

## Consequences

- The owner sets up the mailbox, or a forward at the domain's registrar, before anyone gets a build, and keeps the domain for as long as any build is in use.
- The page for the owner's list of what is still missing comes down to the user agreement's blanks and the rights holder's name in the license.
