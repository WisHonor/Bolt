# 156 — The user agreement's blanks, filled by the owner

- **Status:** Accepted
- **Date:** 2026-11-12
- **Agenda topic:** none (the v0.1 release: the final builds)
- **Decided by:** the owner
- **Refines:** [decision 141](141-the-user-agreement.md) (its four blanks, and the box that kept it a draft) and [decision 140](140-bolt-is-closed-source.md) (the license's rights holder)

## Context

Decision 141 wrote the user agreement in plain words, as a draft for a lawyer. It left four facts only the owner could give as marked blanks: who grants it, who may use Opalro and for what, the price, and whose law applies. A box at its top said it wasn't in force until a lawyer had read it and the blanks were filled. Decision 140's `LICENSE` left its rights holder blank. The owner has no lawyer, and gave the facts in answer to one question at a time.

## Options

- **A. Wait for a lawyer** before filling the blanks. v0.1 would wait too, since an archive with blanks has no agreement in force.
- **B. Fill the blanks with the owner's own answers,** remove the box, and keep the plain-language text as decision 141 wrote it.

## Decision

**B.** The answers:

- **Who "we" is, and the rights holder in `LICENSE`:** Wissam Elmasry, a person.
- **Who may use Opalro, and for what:** "your own use as one person, for any purpose, personal or commercial".
- **The price:** nothing. Opalro 0.1 is free, so the limit of liability, what the person paid, is nothing. What the law doesn't let an agreement limit stays unlimited, as section 7 already says.
- **The law:** the Province of Quebec, Canada. A person keeps every right their own consumer law gives them.
- **The "Draft for a lawyer" box** is removed, and the agreement is in force as written.

## Consequences

- `grep -n "\[BLANK:" USER-AGREEMENT.md LICENSE` prints nothing, and the next Release run's archives carry the filled agreement and license.
- No lawyer has read the agreement. It is the plain-language text decision 141 wrote, with these answers.
- **Quebec** (not legal advice). The Charter of the French Language, as Bill 96 amended it, generally expects a standard-form contract offered in Quebec, as this agreement is, to be given in French first. Quebec's Consumer Protection Act also limits what an agreement may exclude for a consumer; the agreement already keeps what the law doesn't let it limit. Before Opalro is sold in Quebec, a French version of the agreement is the owner's to add (the follow-ups).
- A later version that costs money comes with its own agreement (section 10).
