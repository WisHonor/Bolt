# 034 — Tags are kept exactly, and tags and server lines are shared between diary entries

- **Status:** Accepted
- **Date:** 2026-11-02
- **Agenda topic:** none (milestone 2 planning, question 4)
- **Refines:** [decision 010](010-operation-log.md) (what a diary entry holds) and [decision 012](012-lines.md) (tags)
- **Refined by:** [decision 159](159-tags.md) (tags made, moved and deleted in Opalro)
- **Corrected:** 2026-11-03. The last consequence is wrong: repositories made before this change still open. Milestone 1 could make no tags and no remotes, so each of its entries has a zero where the tag count was and another where the remote count was, and the new layout reads both as "none". The text below stays as it was written.

## Context

Git has two kinds of tag:
- **lightweight:** just a name for a commit;
- **annotated:** a small record of its own, holding the tagger, a date and a message, and often a signature.

A Bolt tag holds a name, a save and a message, inside the state of every diary entry. Server lines (decision 033) live there too.

Every command writes a diary entry with the complete state (decision 010). A repository with 2,000 tags and 3,000 server branches would copy about half a megabyte into every entry. That is still about 200 KB after compression, because fingerprints don't compress, so a busy year of work would add gigabytes.

## Options

- **A. As today.** Every tag sits in every diary entry, with name, save and message; the tagger, date and signature are dropped. Jujutsu's operation log works roughly this way, storing a view of every ref in each operation.
- **B. Tags kept exactly, and the big lists shared.** Tags and server lines move into records of their own, which diary entries point to by fingerprint and reuse while they are unchanged. Annotated tags keep their Git bytes. Git shares unchanged folders between commits the same way.

## Decision

**B.**

- **Shared records.** Tags and server lines live in two records of their own, a tag list and a server-lines list ([storage-format.md](../storage-format.md) §9).
  - A diary entry points to each by fingerprint, or records that there is none.
  - An entry that changes neither points to the same records, so they are stored once until they change.
- **Annotated tags.** An imported annotated tag keeps the original bytes of every Git tag object from its ref to the commit, signature included, so export can recreate them exactly. Its Bolt message is the Git message converted to UTF-8 as for commits (decision 019), up to 64 KiB.
- **Tags Bolt can't hold.** A tag that ends at something other than a commit, such as a tree or a single file, is skipped and listed in the import's summary. It stays in Git.
- **Names.** Tag names that aren't valid UTF-8, or are over 255 bytes, are renamed by decision 033's rule.
- **Limits.** The import stops, naming the limit, past any of these:
  - a Git tag object over 1 MiB;
  - more than 16 tag objects in one chain;
  - more than 100,000 tags;
  - a tag list over 512 MiB, the limit for every record.

## Consequences

- [storage-format.md](../storage-format.md) §2, §9 and §11 define the two records, their context strings and object kinds, and the diary's pointers to them. Test vectors follow (§13).
- Commands that don't need tags or server lines never read them.
- Milestone 1's diary entries hold tags and server lines inline, so repositories made before this change won't open. Repositories made by development builds are disposable ([decision 019](019-review-clarifications.md)).
