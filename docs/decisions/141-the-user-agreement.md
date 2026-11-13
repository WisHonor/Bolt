# 141 — The user agreement

- **Status:** Accepted
- **Date:** 2026-11-11
- **Agenda topic:** none (the v0.1 release: the package)
- **Decided by:** Claude, at the owner's request while the owner was away. The owner can reverse any part.
- **Refines:** [decision 140](140-bolt-is-closed-source.md) (people use Bolt under a user agreement)
- **Refined by:** [decision 156](156-the-user-agreements-blanks-filled-by-the-owner.md) (the owner's four facts filled, and the draft box removed)

## Context

**In plain words.** With Bolt closed ([decision 140](140-bolt-is-closed-source.md)), the law lets nobody use it until they are given permission. The user agreement is that permission: what a person may do with Bolt, what they may not, and what the owner does and doesn't promise. Bolt holds people's work, so the promises about data matter as much as the permissions.

**The owner's facts.** The owner gave four facts as blanks to fill: who grants the agreement (`<NAME OR COMPANY>`), the price (`<PRICE>`), who may use Bolt and for what (`<WHO, AND FOR WHAT>`), and whose law governs it (`<COUNTRY>`). All four were still unfilled on 2026-11-11.

## Options

- **A. A standard long-form license** (an end-user license agreement from a template, in legal language). Familiar to lawyers, read by nobody else.
- **B. A plain-language agreement written for Bolt,** short sections a person can read in five minutes, marked as a draft for a lawyer, with the owner's four facts left as marked blanks.
- **C. No agreement:** "all rights reserved" and a download link. A person who downloads Bolt then has no written permission to use it, and the owner makes no limit on what they promise.

## Decision

**B.** The draft is `USER-AGREEMENT.md`, at the repository's top, and it ships in every archive beside the program; the download page offers it before the download ([decision 144](144-where-people-get-bolt-and-report-problems.md)).

- **Marked a draft.** Its first lines say it is a draft for a lawyer to review, and not in force until a lawyer has read it and the blanks are filled. Each blank is written `[BLANK: …]` with what goes there, so a search finds every one.
- **What a person may do:** install and run Bolt on computers they use, for [BLANK: who, and for what], and keep copies for their own backup.
- **What they may not do:** pass Bolt on (they send people to the download page instead), sell, rent or lend it, change it, or take it apart (decompile, disassemble, reverse engineer), except as far as the law allows that whatever an agreement says; nor remove its notices.
- **Their work stays theirs.** The agreement claims nothing in the projects a person keeps with Bolt.
- **What Bolt sends: nothing.** No networking code is in the program or its dependencies, and every build is checked for it ([decision 142](142-bolt-sends-nothing-and-each-build-proves-it.md)): no telemetry, no update check, no license check over the network, no crash reports. Bolt reads and writes only on the computer it runs on. The operating system or a sync tool may move a project's files over a network (a network drive, OneDrive); Bolt doesn't, and the agreement says so.
- **No warranty, and keep another copy.** v0.1 is early software, given as it is, without promises that it fits a purpose or is free of faults, as far as the law allows; the agreement asks people to keep another copy of anything that matters.
- **The limits of the owner's liability,** for a tool that holds people's work: no liability for lost data, lost profits or indirect damage, and at most what the person paid for Bolt ([BLANK: price]); nothing in it limits what the law doesn't let an agreement limit (harm caused on purpose or by gross negligence, injury, and the rights a consumer's own law gives).
- **Other people's code inside Bolt** is under its own licenses, listed in the notices that ship with it ([decision 143](143-the-notices-cover-what-is-built-in.md)); nothing in the agreement limits a person's rights under them.
- **Stopping:** a person may stop any time (the guide's "Stopping" section says how, without losing files); the agreement ends if its terms are broken, and the person then deletes their copies. What it says about their work, warranty and liability goes on.
- **Changes:** a later version of Bolt may come with a new agreement; the versions a person got under this one stay under it.
- **Reports and ideas** a person sends may be used to improve Bolt, with nothing owed; what a report holds is used only for that.
- **Law:** [BLANK: country]'s, without taking away rights a person's own consumer law gives them.
- **Plain language first:** "you" is the person or organisation using Bolt, "we" is [BLANK: name or company]; sentences are short; every section says what it means for the person in its first line.

## Consequences

- A lawyer must read it, and the owner fill the four blanks, before the owner publishes; the page for the owner lists both.
- The release archives hold `USER-AGREEMENT.md` ([decision 145](145-one-release-workflow-for-every-version.md)), and the archive check fails without it.
- The agreement's promise that Bolt sends nothing binds the code: a feature that sends anything (syncing, planned for v0.3) needs a new decision first, a new agreement, and a change to the check of decision 142.
- No translations: the agreement is in English only.
