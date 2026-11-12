# 150 — The name is Opalro, and `opal` for short

- **Status:** Accepted
- **Date:** 2026-11-12
- **Agenda topic:** none ([design.md](../design.md) §7, the open question "Name", to be settled before format 1 freezes)
- **Decided by:** the owner: Opalro, with `opal` for short. The names nobody reads or types (the crates, the context strings, the magic bytes, the archives, the settings folder) are Claude's calls at the owner's request; the owner can reverse any of them.
- **Supersedes in part:** the names in decisions [004](004-project-contents.md), [008](008-layout-and-settings.md), [025](025-red-team-m1-rulings.md), [059](059-bolt-spellings-left-out-like-git-spellings.md), [082](082-how-big-files-are-cut-into-chunks.md), [114](114-bolt-toml-keys-and-how-bolt-reads-it.md), [117](117-what-json-covers-at-v0.1.md), [119](119-packaging-v0.1.md), [120](120-the-terminal-ui.md), [121](121-format-1-freezes-at-v0.1.md), [145](145-one-release-workflow-for-every-version.md), [146](146-the-clean-machine-check-and-which-systems-each-build-needs.md) and [148](148-what-format-1s-spec-says-before-it-freezes.md). What each of them decided stays; only the names change.

## Context

**In plain words.** The project was called Bolt while it was built, and the name had two problems. Puppet sells a tool called Bolt that installs a `bolt` command, and holds the BOLT trademark for downloadable server software; and StackBlitz's bolt.new comes first when people search for the name ([design.md](../design.md) §7, the page for the owner, parts 4 and 9). The name is also inside every project: the `.bolt/` folder, `bolt.toml`, `.boltignore`, and the first bytes of the program's own files. Those freeze with format 1 when v0.1 is published ([decision 121](121-format-1-freezes-at-v0.1.md)), so this was the last cheap moment to change it. Today it means changing names in the code and the documents; after publishing, it would need a new format.

On 2026-11-12 the owner bought the domain opalro.com and asked to change "everything from bolt to opal (shorter form)".

## Options

- **A. Keep Bolt.** Nothing to change, but the clash with Puppet's `bolt` command and trademark stays for good.
- **B. Opal for everything:** the product, the command and the folder, the way Git and Pijul use one name for all three. Short and easy to say, but "Opal" is busy in software. Google Labs launched Opal, a tool that builds small AI apps, in 2025, and applied for OPAL in class 42 on 2025-10-30. Opal Security sells a developer tool. Opal Labs (class 42) and ORTEC (classes 9 and 42, finance software) hold registered OPAL marks. These were looked up on 2026-11-12 on trademark listing sites, not at the USPTO itself. A dictionary word also makes a weak mark, and searches would find Google's first, as they found bolt.new before.
- **C. Opalro for the product, and `opal` for short** in the command and the files, the way Mercurial uses `hg`, Jujutsu `jj` and Subversion `svn`. The product's name matches opalro.com, a made-up word makes a strong mark, and searches find it alone. The command stays four letters. One cost: Ruby's Opal compiler also installs an `opal` command, so a person who has both has two programs of that name.

## Decision

**C.** People read "Opalro"; they type `opal`.

| What | Was | Now |
|---|---|---|
| The product, in words | Bolt | Opalro |
| The command | `bolt` (`bolt.exe` on Windows) | `opal` (`opal.exe`) |
| The project's own folder | `.bolt/` | `.opal/` |
| The project's rules | `bolt.toml` | `opal.toml` |
| The ignore file | `.boltignore` | `.opalignore` |
| The names a save leaves out ([decision 059](059-bolt-spellings-left-out-like-git-spellings.md), [148](148-what-format-1s-spec-says-before-it-freezes.md)) | `.bolt`, and `bolt~1` to `bolt~4`, in every spelling a drive takes for them | `.opal`, and `opal~1` to `opal~4`, the same way |
| The magic bytes at the start or end of the program's own files | `BOLTPACK`, `BOLTPEND`, `BOLTJRNL`, `BOLTFILE`, `BOLTCHID`, `BOLTLINE` | `OPALPACK`, `OPALPEND`, `OPALJRNL`, `OPALFILE`, `OPALCHID`, `OPALLINE` |
| The fingerprints' context strings | `bolt-vcs <date> <purpose>` | `opal-vcs <date> <purpose>`, with the same dates and purposes |
| The `--json` version field | `bolt_json` | `opal_json` |
| The environment variables | `BOLT_…` | `OPAL_…` |
| The personal settings folder (in the system's folder for settings) | `bolt` | `opal` |
| The crates | `bolt-vcs`, `bolt-vcs-core`, `bolt-vcs-platform` | `opal-vcs`, `opal-vcs-core`, `opal-vcs-platform` |
| The archives | `bolt-0.1.0-<system>.zip` or `.tar.gz`, and `bolt-0.1.0-website.zip` | `opalro-0.1.0-<system>…`, and `opalro-0.1.0-website.zip` |

For anything the table doesn't name: words people read say Opalro; names people type, and names inside the program, say `opal`. The new names are as long as the old ones, so no length or limit changes.

**What keeps the old name:** accepted decision records, plans and rulings, red-team reports and the attack code kept with them, the clean-room records, and the code audit of 2026-11-11, which gets a note at its top. They record what was decided and done while the project was called Bolt. The GitHub repository keeps its name unless the owner renames it (GitHub then forwards the old links), and so do the working folders on this computer.

## Consequences

- **Every fingerprint changes.** A context string is part of what each object's ID is made from, so every ID in the test vectors changes, and the vectors are printed again by their printer tests ([storage-format.md](../storage-format.md) §13). Nothing was published, so no kept project changes meaning. A project saved by a pre-release build (one with a `.bolt/` folder) isn't an Opalro project, and the personal settings in the old `bolt` settings folder aren't read: start such a project again with `opal start`, and set the name and email again.
- **`.bolt` becomes an ordinary name.** The rule that keeps a project's own folder out of saves now guards `.opal`. The code that spots its spellings changes with it, including the first letter it looks for before anything else, so this change gets a red-team session before it is merged ([decision 024](024-red-team-gate.md)).
- **Two programs called `opal`.** Ruby's Opal compiler installs one. The guide's part about Puppet's `bolt` is replaced by one about it: how to tell the two apart, and how to put Opalro's first.
- **The trademark question moves, but stays.** A lawyer should still check OPALRO before v0.1 is sold. The page for the owner (part 4) now names the OPAL marks above instead of Puppet's BOLT. None of this is legal advice.
- **No `bolt` left where people look.** A test fails when a document that ships in a build, or is meant for the website, still says Bolt.
