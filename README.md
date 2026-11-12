# Bolt

Bolt is a version control system for code and large binary files together, with its own engine and storage format: it is not built on Git. You save versions of your project, go back to any of them, try ideas on lines of their own, and take back any command that changed your project with `bolt undo` (a save, a switch to another save, a discard of unsaved work, a shelf, a change of rules; it doesn't take back `bolt tidy`, which frees space, or the starting of the project itself). Big files are cut into chunks, so a change to part of a file stores only the part that changed.

## State

Bolt v0.1 is an early release, for one person on one computer: there is no server, so there is no sharing, merging or locking yet. It can import a Git repository, Git LFS files included. Keep another copy of anything that matters.

What Bolt writes on disk is frozen from v0.1: every later Bolt reads what v0.1 wrote.

## What is in this archive

- `bolt` (`bolt.exe` on Windows): the program.
- `README.md`: this file.
- [docs/guide.md](docs/guide.md): the user guide, for people new to version control and people coming from Git.
- [USER-AGREEMENT.md](USER-AGREEMENT.md): the user agreement, what you may do with Bolt.
- `THIRD-PARTY-LICENSES.html` and `THIRD-PARTY-LICENSES-RUST.html`: the notices for the code of other people that is built into Bolt.

## What each system needs

| System | What Bolt needs |
|---|---|
| Windows | Windows 10 or 11, or Windows Server 2016 or later, on a 64-bit Intel or AMD processor |
| Linux | A 64-bit Intel or AMD processor, and glibc 2.34 or newer (Ubuntu 22.04, Debian 12, Fedora 35, RHEL, AlmaLinux and Rocky Linux 9, and later) |
| macOS | macOS 11 or later, on Apple silicon or Intel |

Nothing else needs to be installed. Put `bolt` on your PATH, the list of folders your terminal looks in for commands: the guide's [Installing](docs/guide.md#2-installing) section has the steps for each system, how to check the download against `SHA256SUMS` from the page you downloaded Bolt from, and what to do when Windows or macOS warns about a program that isn't signed.

## Five commands

Before the first one, tell Bolt your name and email: the guide's [first five minutes](docs/guide.md#tell-bolt-who-you-are) say where.

```text
bolt start                   Make this folder a project, asking what kind
bolt save "Double jump"      Save every change, new files too
bolt status                  See what changed since your last save
bolt history                 List the saves on your line, newest first
bolt undo                    Take back your last step
```

`bolt help` lists every command, and `bolt help <command>` shows one command's examples and options.

## Problems and questions

Write to the address on the page you downloaded Bolt from. A security problem goes to the same address, privately, with "security" in the subject: please keep it to yourself until it is fixed or ruled on.

## License

Bolt is closed source: you may use it under the [user agreement](USER-AGREEMENT.md).
