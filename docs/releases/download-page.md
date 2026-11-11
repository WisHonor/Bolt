# Download Bolt 0.1.0

Bolt keeps every version of your project, the code and the big files together: textures, models, sounds, videos.
It has its own engine and its own way of storing things; it is not built on Git, though it can bring in a Git repository.
Almost every command that changes something can be taken back with `bolt undo`.

**This is an early version.** Bolt 0.1 is for one person on one computer: there is no server, so no sharing or locking yet, and no merging yet either. Keep another copy of anything that matters to you; Bolt is not a backup yet.

## The downloads

Take the archive for your system:

| Your system | What it needs | The archive |
|---|---|---|
| Windows | Windows 10 or 11, or Windows Server 2016 or later, on a 64-bit Intel or AMD processor | `bolt-0.1.0-x86_64-pc-windows-msvc.zip` |
| Linux | A 64-bit Intel or AMD processor, and glibc 2.34 or newer (Ubuntu 22.04, Debian 12, Fedora 35, RHEL, AlmaLinux and Rocky Linux 9, and later) | `bolt-0.1.0-x86_64-unknown-linux-gnu.tar.gz` |
| macOS on Apple silicon (M1 and later) | macOS 11 or later | `bolt-0.1.0-aarch64-apple-darwin.tar.gz` |
| macOS on Intel | macOS 11 or later | `bolt-0.1.0-x86_64-apple-darwin.tar.gz` |

Nothing else needs to be installed: not Git, not a runtime, not a library. On Linux, `ldd --version` prints your glibc version on its first line.

**By downloading you accept the user agreement,** `USER-AGREEMENT.md`, offered here beside the downloads. It is also inside every archive. Please read it first: it says what you may do with Bolt.

## Check the download

`SHA256SUMS`, offered here beside the archives, lists a fingerprint (a SHA-256 checksum) for each archive. If your file's fingerprint is the one listed, the file arrived whole and is the one put on this page. It doesn't say who made it: Bolt 0.1 is not signed yet.

The fingerprints:

```text
<checksums: filled in from the draft's SHA256SUMS>
```

The user guide's section 2, Installing, has the command that checks a file on each system.

## What is inside

Each archive holds six files:

- the program, `bolt.exe` on Windows and `bolt` elsewhere;
- `README.md`, a first look at Bolt;
- the user guide, `docs/guide.md`;
- `USER-AGREEMENT.md`, the user agreement;
- `THIRD-PARTY-LICENSES.html` and `THIRD-PARTY-LICENSES-RUST.html`, the notices for the code of other people that is built into Bolt.

## First steps

1. Download the archive for your system, and `SHA256SUMS`, into the same folder.
2. Read the user agreement, `USER-AGREEMENT.md`.
3. Then follow the user guide's section 2, Installing: it checks the download, unpacks it, puts `bolt` on your PATH and runs `bolt --version`. The guide is `docs/guide.md` in the archive, and is published on this page too.

The guide's section 3, The first five minutes, comes next. Windows and macOS may warn about Bolt the first time it runs, because it isn't signed; the guide's section 2 says what to do.

## Reporting a problem

Write to the address on the page you downloaded Bolt from. Say which version `bolt --version` prints, your system, and what you typed and saw. A security problem goes to the same address, privately, with "security" in the subject. "Reporting a problem", published on this page, says what to send.
