# Bolt

Bolt is a version control system for code and large binary files together, with its own engine and storage format: it is not built on Git. You save versions of your project, go back to any of them, try ideas on lines of their own, and undo any command with `bolt undo`. Big files are cut into chunks, so a change to part of a file stores only the part that changed.

## State

Bolt v0.1 is an early release, for one person on one computer: there is no server, so there is no sharing, merging or locking yet. It can import a Git repository, Git LFS files included.

What Bolt writes on disk is documented in [docs/storage-format.md](docs/storage-format.md), and that format is frozen from v0.1: every later Bolt reads what v0.1 wrote.

## Install

Download the archive for your system from the GitHub release, check it against `SHA256SUMS`, and put `bolt` on your PATH. <!-- CHECK: the release page's address -->

| System | Archive |
|---|---|
| Windows | `bolt-0.1.0-x86_64-pc-windows-msvc.zip` |
| Linux | `bolt-0.1.0-x86_64-unknown-linux-gnu.tar.gz` |
| macOS, Apple silicon | `bolt-0.1.0-aarch64-apple-darwin.tar.gz` |
| macOS, Intel | `bolt-0.1.0-x86_64-apple-darwin.tar.gz` |

v0.1 isn't signed, so Windows and macOS warn the first time you run it. The guide's [Installing](docs/guide.md#2-installing) section has the steps for each system, and how to get past the warning.

## Five commands

```text
bolt start                   Make this folder a project, asking what kind
bolt save "Double jump"      Save every change, new files too
bolt status                  See what changed since your last save
bolt history                 List the saves on your line, newest first
bolt undo                    Take back your last step
```

`bolt help` lists every command, and `bolt help <command>` shows one command's examples and options.

## Documents

- [docs/guide.md](docs/guide.md): the user guide, for people new to version control and people coming from Git.
- [docs/design.md](docs/design.md): how Bolt works and why.
- [docs/storage-format.md](docs/storage-format.md): everything Bolt writes on disk, byte by byte.
- [docs/json.md](docs/json.md): the `--json` output every command gives, for scripts.
- [docs/security.md](docs/security.md): what Bolt defends against, and how.
- [docs/benchmarks.md](docs/benchmarks.md): measurements against Git and Git LFS.

## Build from source

Bolt is written in Rust. With a Rust toolchain installed:

```text
cargo build --release -p bolt-vcs
```

The program is `target/release/bolt` (`bolt.exe` on Windows).

## Reporting a vulnerability

Don't open a public issue for a security problem. [SECURITY.md](SECURITY.md) says how to report one in private.

## License

Bolt has no license yet; one will be chosen before v0.1 is published.
