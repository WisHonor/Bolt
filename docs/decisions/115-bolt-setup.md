# 115 — `bolt setup`

- **Status:** Accepted
- **Date:** 2026-11-09
- **Agenda topic:** none (milestone 6 planning)
- **Decided by:** Claude, at the owner's request while the owner was away. The owner can reverse any part.
- **Refines:** [decision 016](016-presets.md) (changing presets shows the exact lines and is an undoable change); [decision 008](008-layout-and-settings.md) (`bolt setup` is where format upgrades happen); [decision 071](071-a-step-records-its-change-to-the-folder.md) (what the `setup` step records)

## Context

Decision 016 gave `bolt setup` one job: show exactly which lines of `bolt.toml` and `.boltignore` a preset switch would change, and apply them "as a normal, undoable change". Decision 008 also made it the only place a repository's format is upgraded. Nothing was built. Open: what `bolt setup` does with no flag, how the change becomes undoable, whether single rules get flags of their own, and what a script does.

The two files are ordinary files in the folder. They are often unsaved: `bolt start` has just written them. And they are untrusted: a copied project can hold a link or a folder under either name.

## Options

**How a switch is shown**
- **A. A "was → now" table of rules in words,** as the design canvas draws it.
- **B. The exact lines,** as `bolt changes` shows a file, which is what decision 016 says.

**How it is undone**
- **A. A diary step** that records the two files before and after as two saves, like `discard` and `unshelf` (decision 071). `bolt undo` puts back exactly what was there, even a file that didn't exist.
- **B. Plain file writes,** with a hint that `bolt discard` puts the saved versions back. Less code, but it loses edits that were never saved, and decision 016 says "undoable".

**Changing one rule**
- **A. Flags** (`--ignore <folder>`, `--lock <pattern>`, `--chunk-over <size>`), as the canvas suggests.
- **B. Edit the file.** `bolt.toml` and `.boltignore` are text files made to be edited (decision 016), and `bolt setup` shows how Bolt reads them.

## Decision

**The exact lines (B), as a diary step (A), with no single-rule flags (B).**

- **`bolt setup`** with no flag changes nothing. It shows the rules as Bolt reads them from the folder: the preset's label, from what size files are stored in chunks, the two numbers of the large-save question, the patterns marked lockable and those without conflict markers, and how many lines the preset's block of `.boltignore` holds. With no `bolt.toml` it says Bolt's defaults apply. Problems in `bolt.toml` show as warnings ([decision 114](114-bolt-toml-keys-and-how-bolt-reads-it.md)).
- **`bolt setup --preset code|game|custom`** switches:
  1. It works out the new text of both files ([decision 113](113-presets-what-each-writes-and-how-start-picks-one.md)): `bolt.toml` whole, and in `.boltignore` only the block between the preset's two marker lines, added at the top when there is none. Lines outside the block are never touched. `custom` removes `bolt.toml` and the block; a `.boltignore` left with nothing but empty lines is removed.
  2. It prints the change to each file, line by line, in the form `bolt changes` uses. When nothing would change it says so and ends, exit code 0.
  3. In a terminal it asks `Apply these changes? [y/N]` on stderr. No, or an empty answer, changes nothing. `--yes` skips the question. Without a terminal, or with `--json`, and without `--yes`, it stops after the preview, exit code 1, and names `--yes`.
  4. It writes both files as one step.
- **The step.** `setup` is a changing command: it takes the lock, finishes a stopped switch first, and records one diary entry whose folder change is two saves, the two files as the folder held them and as it wrote them (decision 071). So:
  - `bolt undo` puts both files back exactly, edits that were never saved included, and removes a file the step made; `bolt redo` writes them again;
  - other unsaved work is left alone;
  - a crash is finished by the next changing command, as for any switch (decision 049);
  - the state (lines, position, shelves) doesn't change, and the new files show as unsaved changes until a save takes them.
- **Refused, changing nothing:** `bolt.toml` or `.boltignore` in the folder that isn't a regular file (a link, a folder, a special file), or one over 1 MiB.
- **No flags for single rules.** To change a rule, edit `bolt.toml` or `.boltignore`; `bolt setup` then shows what Bolt makes of it.
- **Newer presets.** A preset's text carries a version. Running `bolt setup --preset` with the project's own preset shows, as line changes, what a newer Bolt's version of it would change, and whatever was edited by hand since.
- **Format upgrades.** Format 1 has none ([decision 121](121-format-1-freezes-at-v0.1.md)). When a later format or feature exists, `bolt setup` is where it is offered and done (decision 008).

## Consequences

- `bolt start` uses the same step to write a preset's files (decision 113), so there is one piece of code that writes rule files, and it is the one write-out already protects.
- Switching presets replaces `bolt.toml` whole, so rules added by hand are lost unless they are added again; the preview shows each such line before the question, and `bolt undo` brings them back.
- [storage-format.md](../storage-format.md) §9 lists `setup` among the commands a diary entry names, and [json.md](../json.md) gains the documents of both forms.
- The design canvas's rule table and its `--lock`, `--ignore`, `--chunk-over` and `--ask-over` flags are not built; milestone 6's rulings list them.
