# Security

Opalro treats every project it is given, and every Git repository it imports, as possibly built to cause harm. If you find a way around that, please tell us in private.

## How to report a vulnerability

Write to support@opalro.com, with "security" in the subject. Only the people who look after Opalro see it.

Please don't describe a vulnerability anywhere public, such as a forum or social media, before it is fixed or ruled on.

## What to include

- The version: what `opal --version` prints.
- Your system: Windows, macOS or Linux, and its version.
- The steps that show the problem, from the start: what you ran, and what happened.
- If you can share one, a small repository or project that shows it. Strip anything private from it first.

## What to expect

- An acknowledgement that the report arrived.
- Then either a fix, or a written ruling that explains why something is not treated as a vulnerability, or how it is handled instead.
- Credit in the fix's notes, if you want it, under the name you give.

Opalro is a small project, so replies may take days. Please keep the problem private until it is fixed or ruled on.

## What counts

A vulnerability is anything a repository or project Opalro was given can make Opalro do to the person using it, such as:

- lose or corrupt their data;
- read or write anything outside the project;
- run a program;
- hang, or exhaust their memory or disk without limit;
- mislead the person at the terminal, for example by text in a file name or a message that passes for Opalro's own output.

"Given" covers a project copied from someone else (its files, its `opal.toml` and `.opalignore`, and everything in its `.opal` folder) and a Git repository passed to `opal import`. Opalro's own threat model (`docs/security.md`, kept with Opalro's source code) lists the threats Opalro defends against, and how.

## What doesn't count

Problems that need the attacker to already control your user account, such as malware running as you, or someone who can edit your personal Opalro settings. Opalro trusts its own program and your personal settings.
