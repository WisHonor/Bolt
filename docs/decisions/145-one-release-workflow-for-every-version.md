# 145 — One release workflow for every version

- **Status:** Accepted
- **Date:** 2026-11-11
- **Agenda topic:** none (the v0.1 release: the package)
- **Decided by:** Claude, at the owner's request while the owner was away. The owner can reverse any part.
- **Refines:** [decision 119](119-packaging-v0.1.md) (the release workflow and what each archive holds)
- **Superseded in part by:** [decision 150](150-the-name-is-opalro.md) (the names: Bolt is now Opalro, and `bolt` is now `opal`)

## Context

**In plain words.** The release workflow is the machine that turns a commit into the files people download. It was built for one release, v0.1.0, and its last step names that version. The first tester to find a bug will need a v0.1.1, and the machine should make it without being rebuilt: change the version number, write the notes, start it.

**Technically.** `.github/workflows/release.yml` reads the version from `Cargo.toml` for the archives' names, but its `release` job sets `TAG: v0.1.0` and refuses any other version. Each archive holds four files; the user agreement ([decision 141](141-the-user-agreement.md)) and the standard library's notices ([decision 143](143-the-notices-cover-what-is-built-in.md)) must join them. `SHA256SUMS` is written only in the last job, so nothing before it can check a download the way a person will. The builds of a pull request stay among the run's artifacts for the repository's default 90 days, though they are tests of the workflow, built from a branch.

## Options

- **A. A workflow per release,** copied and edited each time. Every copy is a new thing to review.
- **B. One workflow whose every name comes from `Cargo.toml`,** so a release is: change the version, add the notes, start the workflow from `main`.

## Decision

**B.**

- **The version** is read once, from the `bolt-vcs` package in `Cargo.toml` (as today), and everything is named from it: the archives (`bolt-<version>-<target>.zip` or `.tar.gz`), the draft's tag name (`v<version>`) and title (`Bolt <version>`), and the notes, `docs/releases/v<version>.md`. The first job fails when that notes file is missing, so a pull request that bumps the version without notes fails at once.
- **Each archive holds exactly six files:** the program, `README.md`, `docs/guide.md`, `USER-AGREEMENT.md`, `THIRD-PARTY-LICENSES.html` and `THIRD-PARTY-LICENSES-RUST.html`. The packing step fails on any other set, and on any file in it that holds the repository's address ([decision 144](144-where-people-get-bolt-and-report-problems.md)).
- **Each program is checked for networking** right after it is built ([decision 142](142-bolt-sends-nothing-and-each-build-proves-it.md)), before its smoke test.
- **`SHA256SUMS` is written in a job of its own** right after the four builds, so the clean-machine check ([decision 146](146-the-clean-machine-check-and-which-systems-each-build-needs.md)) checks every download against it as a person would.
- **The website copies** are made in the same run: `bolt-<version>-website.zip` holds the documents meant for the owner's website (the download page's text, the welcome note, how to report a problem, the known problems, the guide, the user agreement, `SECURITY.md` and the release notes, as they are, and the `--json` reference, the storage format and the benchmarks with their links into the repository made plain text). The job fails when any of them still holds the repository's address or a link into it.
- **The draft release** is made only on a start by hand from `main`, and only after every build, the checksums, the website copies and the clean-machine check have passed. It refuses when a tag or release named `v<version>` exists, drafts included (as today). Its files: the four archives, `SHA256SUMS` and the website copies.
- **A pull request's builds are tests of the workflow,** not releases: the artifacts of a run that a pull request started are kept for 3 days, not the repository's default.
- **What doesn't change:** only a start by hand makes a draft, only the last job can write, nothing is published, signed or tagged, every action is pinned to a commit, and cargo-about is downloaded at a pinned version and checked against its checksum.
- **A fix release** (v0.1.1 after a tester's bug) follows the steps in [testing.md](../testing.md) §4: the fix with a test that fails without it, through a pull request; the version raised in `Cargo.toml` with `Cargo.lock` updated in the same commit, and `docs/releases/v0.1.1.md`; then the workflow started by hand from `main`, and the owner publishes the draft and puts its files on the website.

## Consequences

- The workflow file needs no edit for a new version; a reviewer of a release reads only the version change and the notes.
- A pull request that touches the workflow, `about.toml`, `about.hbs`, `USER-AGREEMENT.md` or anything under `docs/releases/` still runs every job but the draft, the clean-machine check included.
- The website copies are made from the same commit as the program, so the guide on the website is the guide in the archive.
- A draft left half-made by a failed last job still blocks the next start until it is deleted by hand, as testing.md says.
