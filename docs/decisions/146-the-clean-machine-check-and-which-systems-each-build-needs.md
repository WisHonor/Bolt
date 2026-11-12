# 146 — The clean-machine check, and which systems each build needs

- **Status:** Accepted
- **Date:** 2026-11-11
- **Agenda topic:** none (the v0.1 release: the package)
- **Decided by:** Claude, at the owner's request while the owner was away. The owner can reverse any part.
- **Refines:** [decision 119](119-packaging-v0.1.md) (each program is run before it is packed; the Windows program needs nothing installed; the Linux program runs on older systems)
- **Superseded in part by:** [decision 150](150-the-name-is-opalro.md) (the names: Bolt is now Opalro, and `bolt` is now `opalro`)

## Context

**In plain words.** A program that works on the computer that built it can still fail on a stranger's: the builder has compilers, libraries and settings a new person doesn't. The release workflow runs each program once, but on the runner that built it. The only time a release program ran away from its builder was on the owner's Windows laptop, which has the build tools too. And the guide says which systems Bolt runs on only for Linux.

**What each program declares** (checked 2026-11-11):
- **Windows:** built for x86-64 with Microsoft's tools, the C runtime linked in. Rust's `x86_64-pc-windows-msvc` target needs Windows 10 or Windows Server 2016 or later. The platform crate's newer calls (POSIX-style rename and delete, which need Windows 10 version 1607 or later) fall back to the plain ones where the system refuses them, as on FAT drives.
- **Linux:** built on Ubuntu 22.04; the newest glibc symbol it needs is `GLIBC_2.34` (the release run of 2026-11-10 prints it), so it needs glibc 2.34 or newer: Ubuntu 22.04, Debian 12, Fedora 35, and Red Hat Enterprise Linux, AlmaLinux and Rocky Linux 9, and later.
- **macOS:** Rust's defaults: macOS 11 for Apple silicon and 10.12 for Intel.

## Options

- **A. By hand,** on machines the owner borrows. Not repeatable, and not run again for v0.1.1.
- **B. On fresh GitHub runners only.** A runner is a new machine, but it holds compilers, Git and dozens of libraries a person's computer may not.
- **C. On fresh runners, and in bare containers on Linux and Windows,** with the oldest system the build claims wherever one can be had, as part of every run of the release workflow.

## Decision

**C.** A `clean-machine` job in the release workflow, after the checksums are written, runs each archive in each of these places:

| Archive | Fresh runner | Bare container |
|---|---|---|
| Windows | `windows-2025` and `windows-2022` | Windows Server Core (`ltsc2022`), on the `windows-2022` runner |
| Linux | `ubuntu-latest` | AlmaLinux 9 (glibc 2.34, the oldest the build claims), Ubuntu 22.04 and Debian 12 |
| macOS, Apple silicon | `macos-15`, and the oldest macOS runner GitHub offers for it | none: macOS has no containers |
| macOS, Intel | `macos-15-intel` | none |

- **On each runner,** the archive and `SHA256SUMS` are downloaded fresh and checked with the guide's own command for that system; the archive is unpacked as the guide says.
- **In each place,** with a settings folder of its own (`BOLT_CONFIG_DIR`) and nothing else set: `bolt --version` must print `bolt <version>`; `bolt start` with each preset, each in a new folder; in one project, a save, a change, `status`, a second save, `goto` to the first save, `undo`, `redo`, a change put aside with `shelf`, and `verify`; and an import of a small Git repository made on the runner beforehand (Bolt reads it without Git, so the container needs none). Each command must succeed, and the few whose results show it are checked (the version, `goto` leaving the first save's contents, `undo` and `redo` putting them back, `verify` finding nothing).
- **The project path** in each place holds a space and letters outside ASCII (`Projets d'été/日本`), and so does the settings folder's, as a user name would put them there.
- **The first run** follows the guide word for word on each runner (the install steps, then the first five minutes, from the guide inside the archive); the plan's task for it lists what a CI runner can't do the way a person does (a dialog, Notepad) and what it does instead.
- **What each program needs** is written in the guide, under "Download": Windows 10 or 11, or Windows Server 2016 or later, on a 64-bit Intel or AMD processor; Linux on a 64-bit Intel or AMD processor with glibc 2.34 or newer (with the distributions above); macOS 11 or later, on Apple silicon or Intel. The check prints what each program declares (its newest glibc symbol; its macOS minimum) and fails when that no longer matches the guide.
- **The oldest checked** is what GitHub offers: Windows Server 2022 for Windows, glibc 2.34 (AlmaLinux 9) for Linux, and the oldest macOS runner for each Mac. Windows 10 and the oldest macOS versions can't be had in CI; the page for the owner lists them among the things to try by hand.

## Consequences

- The follow-ups row "`bolt.exe` hasn't been run on a clean Windows" is done when the Server Core run passes: a Windows with no build tools, no Visual C++ runtime and no Git.
- The check runs on every release run, a pull request's included, so v0.1.1 gets it for nothing.
- The macOS claim for Intel is narrower than what the program declares (10.12): nothing older than macOS 14 can be tried in CI.
- It costs runner time on every release run: about ten jobs of a few minutes, of which the macOS ones count tenfold against a private repository's minutes.
