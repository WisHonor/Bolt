# The name: Opalro, and `opal` for short

**Goal:** everything that was called Bolt is called Opalro, with `opal` for short ([decision 150](../decisions/150-the-name-is-opalro.md)), and every document gives the owner's addresses ([decision 151](../decisions/151-opalro-com-and-support-at-opalro-com.md)), before format 1 freezes at v0.1.

**Branch:** `opalro`, from `main` at `bf6e651` (the package's merge), in the second working folder.

## Tasks

1. **The decisions and the spec first.** Decisions 150 and 151, a back-link in each record whose names change, `design.md` (the open question "Name" closed), `storage-format.md` and `json.md`.
2. **The mechanical rename.** The crates' folders moved with `git mv`; every living file renamed by one script with ordered rules: archives `opalro-…`, `boltignore`, names inside the code (`BoltName` → `OpalName`), articles ("a Bolt" → "an Opalro"), then `Bolt` → `Opalro`, `BOLT` → `OPAL`, `bolt` → `opal`. Kept: the repository's address and the file names of history records. History records aren't touched.
3. **What a script can't see.** Text about other products (Puppet's Bolt, bolt.new), names spelled in pieces or in hex, a check on a single letter, case variants of the folder's name in test data.
4. **The test vectors,** printed again from the renamed code.
5. **The documents' content:** the guide's part about another program with the command's name, the owner's page, the addresses, the code audit's note.
6. **A test that no shipped document says Bolt.**
7. **A red-team session** on the folder guard and the renamed format (decision 024).
8. **The final review,** CI on Windows, macOS and Linux, and the pull request, left for the owner: it changes what is stored.

## Rulings

1. **The names nobody types** are in decision 150's table: the crates `opal-vcs-*`; the context strings `opal-vcs <date> <purpose>`, same dates; the magic bytes `OPAL…`; `opal_json`; `OPAL_*`; the settings folder `opal`; the archives `opalro-<version>-<system>`. Words people read say Opalro; names people type, or that live inside the program, say `opal`. The new names are as long as the old ones, so no limit or offset changes.
2. **History keeps the old name:** decision records, plans and rulings, red-team reports, the attack code, the clean-room records and the code audit, which gets one note at its top.
3. **Lines inside living documents that record what was asked or planned under the old name keep their words:** the name question's "The question was" in `design.md`, row T12 of `design.md`'s first-review table, and milestone 1's workspace task in `milestones.md`. Their crates.io claims were checked for `bolt`, not `opal`.
4. **The message about a `.opal` folder another tool made names no tool.** bolt.new makes `.bolt` folders; no tool is known to make `.opal` ones. The rule stays: such a folder is never taken over.
5. **The guide's part about Puppet's `bolt` becomes one about Ruby's Opal compiler,** which installs an `opal` command. Same advice: how to tell which comes first and how to put Opalro's first. Ruby's `--version` output was not checked, so the guide says "anything else" instead of quoting it, and the `/usr/local/bin` warning now covers macOS too.
6. **What the script missed, found by the tests and by reading.** `is_opal_alias` checked for a first letter `b` before anything else, which would have let `opal~1` through. Test data had `.Bolt` and `Bolt.toml` (case variants of the names, turned into "Opalro" by the prose rule), `.b\u{200C}olt` (split by a character macOS ignores), and RT14's folders `b olt` … `b o lt` (named so their short names take `BOLT~1` to `BOLT~4`). Each now spells `opal` the same way. The test data's example of a hashed short name, `BOC2A3~1`, became `OPC2A3~1`.
7. **The addresses are plain text** in every shipped document, https://opalro.com and support@opalro.com, as decision 144 asked.
