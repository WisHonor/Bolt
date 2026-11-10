# 120 — The terminal UI

- **Status:** Accepted
- **Date:** 2026-11-09
- **Agenda topic:** none (milestone 6 planning)
- **Decided by:** Claude, at the owner's request while the owner was away. The owner can reverse any part.
- **Refines:** [decision 017](017-commands-and-output.md) (output and error style; colours and progress only in a terminal; help shows examples first); [decision 058](058-changes-line-by-line.md) (colour in `bolt changes`, and colour on Windows)

## Context

The owner designed what Bolt's terminal should look like: a canvas with a style guide and a screen for every command, in dark, light and plain form ([reviewed on 2026-11-08](../plans/2026-11-09-terminal-ui-canvas-review.md)). The review found the look good and many facts wrong, since the canvas was drawn without Bolt's rules: invented flags, wrong numbers, commands that work another way. The canvas was not redone, so milestone 6 builds from it under one rule the owner set:

> The design sets the look: layout, colour, symbols, widths and the style of the words. Bolt's decisions, json.md and the code set the facts: commands, flags, numbers, exit codes, warnings and recovery hints. Where they disagree, the decision wins.

Today Bolt prints plain text. Only `bolt changes` has colour, and not on Windows: the older Windows console shows colour codes as text until a program asks it to read them, and that request is an operating-system call. Decision 058 expected it to be new `unsafe` code in Bolt's platform crate, which the owner reviews line by line.

Bolt knows nothing about the terminal's width, and writes `→` and `…` whatever the console can show.

## Options

**Colour on Windows**
- **A. Bolt's own console call,** about fifteen lines of `unsafe` in the platform crate. Milestone 6 would then wait for the owner's line-by-line review before it could merge.
- **B. The call that is already inside Bolt.** `clap`, which reads Bolt's command line, brings the `anstyle-query` crate, whose `enable_ansi_colors` makes exactly that request; clap runs it today to colour its own help. Calling it adds no code of Bolt's that is `unsafe`, and no package to `Cargo.lock`.
- **C. No colour on Windows,** Bolt's main platform.

**The terminal's width**
- **A. A new small dependency,** `terminal_size` (0.4.4), the crate clap itself uses to wrap help. It asks the system; the two crates it is built on, `rustix` and `windows-sys`, are already in Bolt at the versions it asks for.
- **B. Bolt's own call in the platform crate:** safe on macOS and Linux through `rustix`, `unsafe` on Windows.
- **C. Always assume 80 columns.** No dependency, and none of the canvas's width rules.

**Lines meant for people, when nobody is looking** (the canvas prints its "Next:" lines, phase lines and wrapped text in logs too)
- **A. As the canvas draws them.** Every script and log gets coaching lines on stderr, and a long error breaks over several lines there.
- **B. Only in a terminal.** Without one, stderr stays as it is today: empty unless something went wrong, and every message on one line, so scripts and logs can match it.

## Decision

**Colour on Windows through the call already in Bolt (B). Width from `terminal_size` (A). Lines for people only in a terminal (B).**

### Streams

- **stdout** carries the answer: the outcome line, its details, lists and tables.
- **stderr** carries what talks to the person: `error:`, `warning:`, `hint:`, questions, progress, and the "next step" lines.
- Order: the answer, then warnings, then the next step. Warnings are never dropped and never merged into a count.

### Layout

- **The outcome comes first:** one line at the left edge that starts with what happened, in the past tense: `Saved`, `Updated` (an amend), `Switched to`, `Threw away`, `Undid`, `Redid`, `Shelved`, `Unshelved`, `Started`, `Imported`, `Freed`, `Verified`. A command that only reports (`status`, `history`, `line`, `show`) opens with where you are. "Nothing to save: …" and its like are plain lines, exit code 0.
- **Details** follow after one blank line, indented four spaces; bullets are `•` and a space. Lists with a place marker (`history`, `line`, `undo --list`) put the marker `▸` at the left edge and indent the other rows two spaces.
- **`error:`** says what happened; **`hint:`** follows after a blank line and says what to do. The words of every error, warning and hint stay Bolt's own, from the code: the canvas changes how they are laid out, not what they say.
- **`warning:`** lines come after the answer, one blank line before the first, one line each.
- **`Next:`** names the most likely next command, as the last line, at most one per command. **`Changed your mind? bolt undo`** ends the commands whose result may surprise (`discard`, `goto`, `setup --preset`, `unshelf`).
- Never two blank lines in a row, and none at the end. No boxes, banners or rules; the one separator is the dim line between files in `bolt changes`.

### Colour

Bolt uses the terminal's own sixteen colours, never fixed RGB, and every colour has a carrier that survives without it: a word, a symbol or a place.

| What | Style |
|---|---|
| The outcome's first word | green, bold |
| `error:` | red, bold |
| `warning:` | yellow, bold |
| `hint:`, `Next:` | cyan, bold |
| A command to type | cyan |
| A change ID | magenta, bold |
| A fingerprint, a line's name, a path | blue |
| Added and removed (words, counts, diff lines) | green and red |
| `modified` and `kind changed` | yellow |
| A diff's `@@` line | cyan |
| What can be skipped (times, authors, counts in brackets) | dim (SGR 2) |
| The number a question turns on; the place marker | bold |

- **When:** a stream gets colour when it is a terminal, `NO_COLOR` is unset or empty, and `TERM` isn't `dumb`. stdout and stderr are judged each on its own. `--json` never has any.
- **On Windows** Bolt first asks the console to read colour codes (`anstyle_query::windows::enable_ansi_colors`, option B). Inside Windows Terminal that is always so. On the older console the request covers stdout and stderr together and fails unless both are the console, so when either is redirected there, the other stays plain.
- Nothing essential is ever said in dim text or by colour alone.

### Symbols

| Symbol | Without it | Used for |
|---|---|---|
| `•` | `*` | bullets |
| `▸` | `>` | the place marker |
| `→` `←` | `->` `<-` | was and now; the suggested choice |
| `✓` `✗` | `+` `x` | `bolt verify`'s checks, nowhere else |
| `─` | `-` | the separator in `bolt changes` |
| `…` | `...` | something cut short; work in progress |
| `█` `░` | `#` `.` | a progress bar's cells |

Bolt writes the symbols only to a terminal known to show them: on macOS and Linux one whose locale is UTF-8 (`LC_ALL`, `LC_CTYPE` or `LANG`); on Windows, Windows Terminal (`WT_SESSION` is set) or a terminal that names itself (`TERM_PROGRAM`). Everywhere else, the older Windows console and anything that isn't a terminal included, it writes the plain forms. `BOLT_ASCII=1` asks for the plain forms anywhere; `BOLT_ASCII=0` asks for the symbols in any terminal.

### Widths

- Bolt asks the terminal for its width. In a terminal, an error, warning or hint longer than the smaller of that width and 100 columns is wrapped at a space, and the lines after the first start under its text (7 spaces for `error:`, 9 for `warning:`, 6 for `hint:` and `Next:`).
- A table is as wide as what it holds, never padded to fill the terminal. When a row would pass the width, the path gives way first: whole folders are cut from its middle, keeping its first folder and the file's name (`Assets/…/Rock_Albedo.psd`); a message in a list is cut at its end.
- **When stdout or stderr isn't a terminal, nothing on it is wrapped or cut.** Each message is one line and each path is whole.
- Nothing in an error or a hint is ever cut.

### Progress

- A bar shows on stderr, only in a terminal, only after 400 ms of work, redrawn at most ten times a second on one line, and erased when the work ends; the outcome line is the summary. Its shape: a label padded to 24 characters, a bar of 24 cells, the percentage, then done and total (`Importing commits        ██████░░░░░░░░░░░░░░░░░░  26%  334 / 1,284`). Under 70 columns the counts go; under 50 the bar goes too.
- Work with no known total shows the label, `…` and a turning mark instead.
- Bars: an import's three passes (reading Git's history, checking Git LFS files, importing commits) and `bolt verify`. `save`, `goto`, `discard`, `shelf`, `unshelf`, `undo`, `redo`, `tidy` and `setup --preset` show the turning mark with what they are doing, since the engine doesn't yet report how far a save is.
- Without a terminal nothing is printed while a command works, as today.

### Numbers, times and names

- **Sizes** are 1024-based with their true names: `512 B`, `31 KiB`, `240 MiB`, `1.2 GiB`; one decimal below 10, none from 10 up. Today's two formatters (one 1024-based but labelled "MB", one 1000-based) become this one. A size that is a setting is shown as `bolt.toml` would write it, in the largest unit that gives it exactly (`100 MB`, `1 GiB`), so the default of 100,000,000 bytes reads `100 MB`, not `95 MiB`.
- **Counts** take thousands separators: `12,000 files`. Plurals are real: `1 file`, `2 files`.
- **Times** stay as they are: relative up to 30 days (`2 h ago`), then the date.
- **Change IDs** are the shortest unique start of at least 4 letters; fingerprints show 12 hex digits in lists.
- **Text from a project** (file names, messages, line names, authors) is escaped exactly as today, colour or not (decision 028). Colour is put around the escaped text when it is printed, never stored in a message, and a column's width counts the escaped form.

### Questions

A question is asked only when stdin and stderr are both terminals and `--json` isn't given: the situation, the question, then the prompt with its default in brackets (`[y/N]`, `Choice [2]:`). Otherwise the command stops and names the flag that answers it. This holds for the three questions Bolt has: before a large save (`--yes`), the name of a new line (`--line`), and the preset (`--preset`), and for `bolt setup --preset` (`--yes`).

### Help

- `bolt` alone and `bolt help` print one screen: a sentence about Bolt, then the commands in three groups (every day; lines, shelves and details; the project), one line each.
- `bolt help <command>` and `bolt <command> --help` lead with examples, then usage, then options, and list only what the command really takes.
- Git's words never appear in help ([decision 116](116-gits-words-as-shortcuts.md)).

### Where the canvas and Bolt's facts disagree

The facts win, each time. The review's list is the checklist; these are its headings, and milestone 6's rulings file lists every case met while building:

- **Exit codes** are 0, 1 and 2 only (the canvas has four).
- **No invented flags or commands:** no `--color`, `--ascii`, `--utc`, `--wait`, `--repair`, `--report`, `--full-paths`, `--allow-lfs-pointers`, `--git-words`, `bolt unlock`, `history -n`, and no `-m` on `bolt save`.
- **Numbers** are Bolt's: the large-save question's, 20 steps in `undo --list`, 8 hex digits for `undo --to`, the limits of `bolt changes`.
- **No "renamed" kind**, no line counts per file in `save` or `status` (they would slow `status`), no "stored in N chunks" line, no "in N saves" column in `verify`.
- **Waiting for the lock** has no timeout and no flag: Bolt says it is waiting and waits.
- **Nothing from later versions:** no locks, no merging, no fetching.
- **The canvas's lines for people in logs** (`Next:`, phase lines, wrapped text) are for terminals only, by this decision.

## Consequences

- Bolt has colour on Windows without new `unsafe` code of its own, so milestone 6 doesn't wait on a line-by-line review for it. `anstyle-query` becomes a direct dependency of the command-line crate; `Cargo.lock` gains nothing for it.
- `terminal_size` 0.4.4 is one new package in `Cargo.lock`, for the command-line crate only: it brings no second copy of anything.
- Almost every message's layout changes, and the CLI tests that match text change with it. Tests run without a terminal, so they see the plain forms: no colour, no symbols, nothing wrapped.
- [docs/terminal.md](../terminal.md) holds these rules as the reference the code is checked against.
- `save` and its neighbours show a turning mark, not a bar; a real bar waits for the engine to report progress, a row in the follow-ups.
- The follow-up "Colour in `bolt changes` on Windows" is done by this decision; design.md's deferred item goes.
