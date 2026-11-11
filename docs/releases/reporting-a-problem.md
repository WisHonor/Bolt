# Reporting a problem

Write to the address on the page you downloaded Bolt from. Every report helps, even "this felt wrong" or "I expected something else".

## What to send

- **The version:** what `bolt --version` prints, such as `bolt 0.1.0`.
- **Your system and its version:** Windows 11 24H2, macOS 15.1, Ubuntu 24.04, and so on. On Linux, also the first line `ldd --version` prints.
- **What you typed:** the command, exactly, and what you did just before if it matters.
- **What it printed:** all of it, copied as text rather than a screenshot when you can.
- **What `bolt verify` says,** run in the project's folder. It checks that everything Bolt stored is whole, and changes nothing.
- **If it helps, the same command with `--json`,** such as `bolt status --json`. It prints the same result as one block of text that is easy to read back exactly.
- What you expected to happen instead.

## What not to send

- **No private files.** Never send a project, its `.bolt` folder or a file from it unless you are sure nothing in it is private. A small made-up project that shows the same problem is best.
- **Strip names.** Bolt's output holds file names, folder paths, and the name and email of whoever saved. Replace any you would rather keep to yourself (`C:\Users\ana\clients\acme` can become `C:\Users\me\project`), and say that you did.

## A security problem

Anything that lets a project or a Git repository someone gave you make Bolt harm you: lose or damage your data, read or write outside the project, run a program, hang or fill your memory or disk, or print something that passes for Bolt's own words.

Send it to the same address, privately, with "security" in the subject, and please keep it to yourself until it is fixed or ruled on. `SECURITY.md`, published on the page you downloaded Bolt from, says what to include and what counts.

## What happens next

You get an answer saying the report arrived. Bolt is a small project, so a reply may take a few days.
