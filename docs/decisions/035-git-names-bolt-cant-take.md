# 035 — Names in Git folders that Bolt can't take as they are

- **Status:** Accepted
- **Date:** 2026-11-02
- **Agenda topic:** none (milestone 2 planning, question 5)
- **Refined by:** [decision 051](051-names-this-computer-cant-write-and-length-limits.md) (names this computer can't write; names at most 765 bytes, paths 4,096)
- **Refines:** [decision 004](004-project-contents.md) (imported histories keep their names) and [decision 019](019-review-clarifications.md) #7 (names in Git trees)

## Context

A few names in Git trees can't come into Bolt as they are, and Git itself treats each kind as a problem:

1. **Dangerous names.** These are never safe to write to disk, and Git refuses to; spellings of `.git` plant files in Git's own folder (CVE-2014-9390, CVE-2019-1349, CVE-2019-1352).
   - Any name Windows or macOS would treat as `.git`: other letter case, `git~1`, trailing dots or spaces, a `:stream` suffix, or invisible characters that macOS ignores.
   - `.`, `..`, an empty name, or a name containing `/`.
2. **Names over 255 bytes,** Bolt's limit. Windows allows 255 UTF-16 units, which can be up to 765 bytes in UTF-8.
3. **Broken folders:**
   - two entries with one name;
   - entries out of Git's order;
   - unknown modes;
   - an ID that doesn't match the contents.

Decision 019 already stops the import at names that aren't valid UTF-8, and skips names that spell `.bolt` with a warning.

## Options

- **A. Stop the import at any of them,** as Git's strict mode does (`transfer.fsckObjects`), and GitHub on a push.
- **B. Skip any of them with a warning,** as decision 019 does for `.bolt`.
- **C. Decide by kind:** skip dangerous names, and stop at long names and broken folders. Git is the closest model: it keeps such a history but refuses to write those names to disk, and its integrity checks reject broken folders.

## Decision

**C.**

- **Dangerous names** are skipped with a warning, along with everything under them.
  - The `.git` spellings are Git's own list (`is_ntfs_dotgit` and `is_hfs_dotgit`), applied on every system.
  - The check lives in Bolt's names module (one place per rule), so milestone 3's write-out can reuse it.
- **Names over 255 bytes** stop the import, listing where they are.
- **Broken folders** stop the import, naming the commit and the folder:
  - two entries with one name, a file and a folder included;
  - entries out of Git's order;
  - an unknown mode;
  - an ID that doesn't match the contents;
  - a missing object.
- **Modes that Git accepts with a note** import as the kind they mean: a zero-padded `040000` is a folder, and `100664` a file. Their trees count as non-standard (decision 032).
- **Format limits.** A snapshot deeper than 4,096 folders, or with more than 10,000,000 entries, stops the import ([storage-format.md](../storage-format.md) §3).
- **The summary** lists the first 100 skipped paths, then a count.
- **Export.** A commit with skipped entries can't be recreated exactly by a later export, so the import counts these commits together with decision 032's non-standard trees.
- **Symlinks** are kept as they are, whatever they point at. Writing them out safely is milestone 3's job.

## Consequences

- Bolt's names module gets the `.git` check, tested with cases from Git's own test suite and from Git's published vulnerabilities.
- [security.md](../security.md) T2 says that an import skips `.git` spellings.
- This settles the follow-up that the importer refuses every spelling of `.git`: such entries are skipped, with a warning.
- Bolt's 255-byte name limit is revisited together with the path-length limit, before format 1 freezes ([design.md](../design.md) §7, deferred).
