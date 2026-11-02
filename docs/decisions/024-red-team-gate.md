# 024 — A red-team session at the end of every milestone

- **Status:** Accepted
- **Date:** 2026-11-01
- **Agenda topic:** none (process change after milestone 1)
- **Followed by:** [decision 025](025-red-team-m1-rulings.md) (rulings after milestone 1's first session), [decision 028](028-red-team-m1-session-2-rulings.md) (after its second) and [decision 029](029-red-team-m1-final-review-rulings.md) (after the final review of the fixes)

## Context

Principle 2 says to treat every repository and server as potentially malicious. Bolt's defenses so far were designed up front ([security.md](../security.md)) and checked by the people and agents who built them: fuzzing, a malicious-repository corpus and task reviews. Nobody has yet attacked Bolt with the single goal of breaking it.

Bolt has no network code before v0.3, but untrusted input already reaches it: a shared project folder (with a `.bolt/` inside), the files and symlinks in a working folder, and the text shown in a terminal. Milestones 2 (importing Git repositories made by strangers) and 3 (writing files to disk) add the areas where version control systems have historically had most of their serious vulnerabilities.

The owner asked whether to run an agent that tries to break in non-stop.

## Options

- **A. Wait until networking (v0.3).** Cheapest now; misses the milestone 2–3 bugs, which are the most common kind.
- **B. A time-boxed red-team session at the end of every milestone, starting now.** A fresh attacker, a specific target list, proof required for each finding. This mirrors per-release security audits, with fuzzing as the continuous part.
- **C. An agent attacking non-stop.** Costs grow without limit and the findings thin out after the first hours (repeats and false alarms). Fuzzing already does the "never stops" part cheaply.

## Decision

**B.**

- **When.** Every milestone ends with a red-team session before it is merged into `main`. Milestone 1 was merged before this decision, so its session runs now, before milestone 2 starts.
- **Who.** A fresh agent on the most capable model, not one that built the milestone. It gets the threat list ([security.md](../security.md) §4), the attack-surface notes for the milestone, and the code. Its goal is to break Bolt: lose or corrupt data, read or write outside the project, store files it should not, crash, hang, exhaust memory or disk, or trick the person at the terminal.
- **Proof required.** Every finding comes with a test that fails because of the bug (or a script that shows it). Suspicions without proof are listed separately and never count as findings.
- **Isolation.** Attack code never runs on the owner's real machine environment:
  - the attacker works on a **copy** of the repository, never the owner's folders;
  - everything it builds or runs (Bolt and its attack programs) runs in a **Docker container** with no network, no credentials, no mounted personal folders, and memory, CPU and process limits;
  - Windows-only attacks (junctions, device names, short names, case tricks) are written as tests and run on GitHub's throwaway Windows machines from a `redteam/*` branch, which is deleted afterwards.
  - Limit: the agent itself runs on the owner's computer, so its instructions confine it to the copy. The container contains what it runs, not the agent.
- **Time box.** One session per milestone, ending with a report: each finding with its severity, the threat it maps to (or a new threat), and its proof.
- **After the session.** Every confirmed finding is fixed with its test before the milestone counts as done, and its test joins the malicious-repository corpus. Anything not fixed now gets a written ruling and an entry in the milestone's follow-ups.

## Consequences

- Each milestone's "done when" now includes a red-team session with its findings fixed or ruled.
- New threats found by the attacker are added to [security.md](../security.md) §4.
- The sandbox (container definition and run script) lives in the repository, so every session uses the same setup.
- Security issues found this way become public when their tests are pushed. This is acceptable while Bolt has no users; before the first public release, `SECURITY.md` defines private reporting (security.md §7).
