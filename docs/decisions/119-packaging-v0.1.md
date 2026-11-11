# 119 — Packaging v0.1

- **Status:** Accepted
- **Date:** 2026-11-09
- **Agenda topic:** none (milestone 6 planning)
- **Decided by:** Claude, at the owner's request while the owner was away. The owner can reverse any part.
- **Refines:** [decision 018](018-milestone-plan.md) (milestone 6's "packaging"); [decision 024](024-red-team-gate.md) (`SECURITY.md` before the first public release)
- **Superseded in part by:** [decision 140](140-bolt-is-closed-source.md) (the license: Bolt is closed source); [decision 144](144-where-people-get-bolt-and-report-problems.md) (people get Bolt from the owner's website and report problems at the owner's address)
- **Refined by:** [decision 143](143-the-notices-cover-what-is-built-in.md) (the notices cover the standard library and the C runtime); [decision 145](145-one-release-workflow-for-every-version.md) (one workflow for every version, six files in each archive); [decision 146](146-the-clean-machine-check-and-which-systems-each-build-needs.md) (the clean-machine check)

## Context

Milestone 6 ends with "packaging", and no document says what that is. Until now Bolt is built from source: the workspace version is `0.0.0`, there is no README, no user guide, no `SECURITY.md`, and no license. The owner publishes: pushing the `v0.1` tag, publishing a release, signing, and claiming a name on any package registry are the owner's own steps. Everything before them can be made ready.

The owner left the license blank, so v0.1 is prepared without one, and the choice heads milestone 6's please-confirm list.

One follow-up row belongs here: Bolt works inside OneDrive because Windows hides its placeholders from programs running in the default mode, and a package format that changes that mode (MSIX, say) would need the check again.

## Options

**How people get Bolt**
- **A. One archive per system, on a GitHub release:** the program, a short README, the user guide and the notices for the code Bolt is built from, with a checksum file.
- **B. Installers and package managers** (MSI or MSIX, Homebrew, winget, `cargo install`). Friendlier, but each is a name to claim, a format to keep working and, for MSIX, a different mode on Windows.

**How the builds are made**
- **A. On the owner's computer.** Nobody else could repeat it.
- **B. By a GitHub Actions workflow, started by hand,** from the commit the release will name.

## Decision

**Archives on a GitHub release (A), built by a workflow started by hand (B).**

- **Version.** The workspace becomes `0.1.0`; `bolt --version` says `bolt 0.1.0`. The packages stay `bolt-vcs`, `bolt-vcs-core` and `bolt-vcs-platform`, and stay `publish = false`: nothing goes to crates.io.
- **Four builds:** Windows (x86-64, MSVC, with the C runtime linked in, so it runs on a Windows with nothing else installed), Linux (x86-64, glibc, built on the oldest Ubuntu GitHub offers, so it runs on older systems too), and macOS for Apple silicon and for Intel. Each is built with `--locked` from the commit the workflow runs on.
- **Each archive** (`bolt-0.1.0-<target>.zip` on Windows, `.tar.gz` elsewhere) holds `bolt` (or `bolt.exe`), `README.md`, the user guide, and `THIRD-PARTY-LICENSES.html`: the license texts of every crate built into Bolt, which their licenses ask for whenever the program is passed on. `SHA256SUMS` lists the four archives.
- **The workflow** (`.github/workflows/release.yml`) starts only when someone starts it (`workflow_dispatch`); no push and no tag starts it. Before packing, it runs each program it built: `bolt --version`, then start, save, history and verify on a small project. Its build jobs can only read the repository. One last job, the only one that may write, makes a **draft** release named `v0.1.0` with the archives, the checksums and the notes. A draft is invisible to everyone but the owner, and makes no tag: GitHub creates the tag when the owner publishes it. The draft names the exact commit the archives were built from as its target, so the tag lands on that commit and no other.
- **Not done, on purpose:** no tag is pushed, nothing is published, nothing is signed or notarized, no installer is built, and no name is claimed on crates.io, Homebrew, winget or anywhere else. Those are the owner's.
- **No MSIX,** so Bolt keeps running in Windows' default mode and milestone 2's OneDrive check stays good.
- **License.** v0.1 is prepared with none: no `LICENSE` file, no `license` in the manifests, and `deny.toml` keeps its exemption for Bolt's own crates. The README says so in one line. Published like that, nobody else has the right to use or share Bolt, so the owner's choice comes first on the please-confirm list; adding it later is a `LICENSE` file, three manifest lines and one line of `deny.toml`.
- **Documents that ship with it:**
  - `README.md`: what Bolt is, how to install it, the first five minutes, where the guide is;
  - `docs/guide.md`: the user guide (install, every command, presets, `safe-folders`, coming from Git);
  - `SECURITY.md`: how to report a vulnerability in private, through GitHub's private reporting for the repository, which the owner turns on.
- **Unsigned programs.** Windows SmartScreen and macOS Gatekeeper warn about a program nobody signed. The guide says how to run it anyway, and that signing is planned.

## Consequences

- Anyone can build the same commit with the same lock file and workflow, which are public. The result isn't promised to be the same bytes: Rust builds aren't byte-for-byte repeatable without more work. The checksums say that a download is the file the workflow uploaded and wasn't damaged on the way; they don't say who made it, since nothing is signed.
- The release workflow is a new way for a mistake or an attacker to reach users, so it is on the red team's list, its actions are pinned to exact commits like the others, and the one tool it downloads (the notices generator) is pinned to a version and checked against a checksum.
- The follow-up "Packaging: … if Bolt is packaged (MSIX, say) … try a project inside OneDrive again" is closed: the archives don't change how Bolt runs.
- Package managers and installers can come after v0.1, each with its own decision.
