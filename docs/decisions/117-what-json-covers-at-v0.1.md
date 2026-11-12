# 117 — What `--json` covers at v0.1

- **Status:** Accepted
- **Date:** 2026-11-09
- **Agenda topic:** none (milestone 6 planning)
- **Decided by:** Claude, at the owner's request while the owner was away. The owner can reverse any part.
- **Refines:** [decision 017](017-commands-and-output.md) (every command accepts `--json`, in a documented, versioned format)
- **Superseded in part by:** [decision 150](150-the-name-is-opalro.md) (the names: Bolt is now Opalro, and `bolt` is now `opal`)

## Context

Every command built so far prints one JSON document with `--json`, and [json.md](../json.md) describes each: fifteen commands, about thirty kinds of warning, and one rule for change ("within one `bolt_json` number, fields may be added … never removed, renamed, or given another type or meaning"). v0.1 is the first release other programs can build on, so three things need settling: what the new commands print, what is fixed from v0.1 on, and what stays out of JSON.

## Options

- **A. Freeze `bolt_json` 1 at v0.1 as it stands,** new commands included, after one review of the documents for names that disagree.
- **B. Call the formats unstable until v0.2.** Free to rename, but nothing a script or an editor plugin could rely on, which is what decision 017 promised them.

## Decision

**A.**

- **Every command has a document.** Milestone 6 adds:
  - `bolt setup`: the rules as Bolt reads them; with `--preset`, the line changes to each file and whether they were applied;
  - `bolt verify`: what was checked and every problem found ([decision 118](118-bolt-verify.md));
  - `bolt start` gains `preset`, the rule files it wrote, and what detection found;
  - `bolt shelf` gains `changed_again`, which tells its own race (every file changed again while Bolt read it) from a stopped switch a command finished first: both gave `shelf: null` and the same warning before.
- **`bolt_json` 1 freezes when v0.1 is released.** From then on json.md's rule holds for good: a field or a kind of warning can be added; a field is never removed, renamed, or given another type or meaning; a change that breaks this takes `bolt_json` 2.
- **What is frozen:** the field names, their types and meanings, the `kind` of each warning, and the exit codes (0 done, 1 failed with an `error` in the document, 2 a command line Bolt couldn't use). **What isn't:** the wording of `message` and `hint` texts, which are for people.
- **Before the freeze, one review:** milestone 6 reads every document once for field names or shapes that disagree with each other, and fixes them while that is still free. Each change is a ruling in the milestone's rulings file.
- **What `--json` never carries:** colour or any other terminal code, symbols chosen for the terminal, progress, hints and "next step" lines. It never asks a question: where a person would be asked, it stops and names the flag (`--yes`, `--line`, `--preset`). Milestone 6's new lines for people (the "next step" lines, the Git-word hint, progress bars) never appear with `--json`, on stdout or on stderr.
- **What is always text:** `--help`, `--version`, a command line Bolt can't use (exit code 2), and the explanation of a Git word Bolt doesn't run ([decision 116](116-gits-words-as-shortcuts.md)).

## Consequences

- json.md gains the two new documents, the new fields and the new kinds of warning (`rules_file_skipped`, `rules_ignored`, and those milestone 6's fixes add), and states the freeze.
- The CLI tests pin each new document with a snapshot, as they do the others (decision 017).
- A mistake found in a document after v0.1 can only be mended by adding a field beside the wrong one, or by `bolt_json` 2.
