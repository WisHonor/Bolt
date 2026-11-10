# Bolt in the terminal

> **Status:** written on 2026-11-09 from [decision 120](decisions/120-the-terminal-ui.md), before the code that follows it (milestone 6). The look is the owner's design canvas; every fact is Bolt's own ([the canvas review](plans/2026-11-09-terminal-ui-canvas-review.md)). §12 is filled in as each command is restyled.

**In plain words.** This page is the rulebook for what Bolt prints for people: where each line goes, how it is laid out, which words get which colour, which symbols are used and what stands in for them, and how all of it changes when nobody is looking at a terminal. The `--json` formats have their own page ([json.md](json.md)).

## 1. Streams

| Stream | What goes there |
|---|---|
| stdout | The command's answer: the outcome line, its details, lists and tables. |
| stderr | What talks to the person: `error:`, `warning:`, `hint:`, questions and their choices, progress, `Next:` and "Changed your mind?" lines. |

Order: the answer, then the warnings, then the next step. With `--json`, stdout holds one document and none of this page applies to it.

## 2. What Bolt finds out first

Each is found out once, when the command starts, for stdout and stderr separately.

- **A terminal:** the stream is one (`std::io::IsTerminal`).
- **Colour** on a stream: it is a terminal, `NO_COLOR` is unset or empty, `TERM` isn't `dumb`, `--json` wasn't given, and on Windows either `WT_SESSION` is set or the console agreed to read colour codes (`anstyle_query::windows::enable_ansi_colors()` returned `Some(true)`; it asks for stdout and stderr together and fails unless both are the console, so on the older console redirecting either leaves the other without colour).
- **Symbols** on a stream: it is a terminal and, on macOS and Linux, the first of `LC_ALL`, `LC_CTYPE` and `LANG` that is set and not empty names UTF-8 (`UTF-8` or `utf8`, in any letter case); on Windows, `WT_SESSION` or `TERM_PROGRAM` is set. `BOLT_ASCII=1` turns symbols off everywhere. `BOLT_ASCII=0` turns them on for any terminal. No other value of `BOLT_ASCII` means anything.
- **Width** of a stream that is a terminal: what the system says (the `terminal_size` crate); 80 when it won't say. A stream that isn't a terminal has no width: nothing on it is wrapped or cut.

Tests run without a terminal, so they see no colour, no symbols, no wrapping and no progress.

## 3. Styles

Bolt writes the terminal's own colours (SGR codes), never RGB, and ends each styled piece with `ESC[0m`.

| Role | Code | Carrier without colour |
|---|---|---|
| The outcome's first word (`Saved`, `Undid`…) | `1;32` green, bold | the word, first on the first line |
| `error:` | `1;31` red, bold | the word |
| `warning:` | `1;33` yellow, bold | the word |
| `hint:`, `Next:` | `1;36` cyan, bold | the word |
| A command to type | `36` cyan | starts with `bolt `, after a colon or on a line of its own |
| A change ID | `1;35` magenta, bold | letters `k` to `z` only |
| A fingerprint, a line's name, a path | `34` blue | a label before it, square brackets, or its `/` |
| `added`, `+` lines and counts | `32` green | the word or the `+` |
| `removed`, `-` lines and counts | `31` red | the word or the `-` |
| `modified`, `kind changed` | `33` yellow | the word |
| A diff's `@@` line | `36` cyan | the `@@` |
| What can be skipped: times, authors, counts in brackets | `2` dim | its place in the line |
| The number a question turns on; the place marker | `1` bold | its place in the sentence; the marker itself |

- Text from a project is escaped first, exactly as before milestone 6 ([security.md](security.md) T21), and the style goes around the escaped text. A message kept in an error or a warning never holds a code: styles are added when it is printed.
- Nothing a person must act on is dim, and nothing is said by colour alone.

## 4. Symbols

| Symbol | Without symbols | Used for |
|---|---|---|
| `•` | `*` | bullets in details |
| `▸` | `>` | the place marker in `history`, `line` and `undo --list` |
| `→` | `->` | was and now |
| `←` | `<-` | the suggested choice in a question |
| `✓` `✗` | `+` `x` | `bolt verify`'s checks, and nowhere else |
| `─` | `-` | the separator between files in `bolt changes` |
| `…` | `...` | something cut short; work in progress |
| `█` `░` | `#` `.` | the cells of a progress bar |

Bolt draws no boxes, banners or rules beyond that separator.

## 5. Layout

- **The outcome line** is first, starts at the left edge with what happened in the past tense, and is one line. A command that only reports (`status`, `history`, `line`, `show`) starts with where you are instead. "Nothing to …" lines are plain, and the command ends with exit code 0.
- **Details** come after one empty line, indented 4 spaces. Bullets are the bullet symbol and one space.
- **Lists with a place marker** put the marker at the left edge and start every other row with two spaces.
- **Columns** are at least two spaces apart; text is left-aligned, numbers and sizes right-aligned. A table is as wide as what it holds and is never padded out to the terminal's width.
- **Empty lines:** one after an outcome line that has details; one before the first `warning:` when anything was printed before it (the answer counts only when stdout and stderr are both a terminal), so stderr never starts with an empty line; one before a `hint:`, a `Next:` or a "Changed your mind?" line. Never two in a row, never one at the end.

## 6. Errors, warnings and the next step

- **`error:`** then the message; **`hint:`** after one empty line. Their words are Bolt's own, from the code that found the problem; this page only lays them out.
- **`warning:`** lines come after the answer, one per warning, in the order met, never merged into a count.
- **`Next:`** names the most likely next command: at most one per command, last. **`Changed your mind? bolt undo`** ends `discard`, `goto`, `unshelf` and `setup --preset`. Both, and the hint the first time a Git word is used ([decision 116](decisions/116-gits-words-as-shortcuts.md)), are printed **only when stderr is a terminal** and `--json` wasn't given. Without a terminal, stderr holds only errors, their hints and warnings, as before milestone 6.
- **Wrapping** happens only on a terminal: a message longer than the smaller of the width and 100 columns breaks at a space, and the lines after the first start under the message's text: 7 spaces in for `error:`, 9 for `warning:`, 6 for `hint:` and `Next:`. A line with no space to break at is left long. A message that already has several lines keeps its own line breaks, each line wrapped the same way.
- **Exit codes:** 0 done, or nothing to do; 1 failed; 2 a command line Bolt couldn't use. Nothing else.

## 7. Cutting what doesn't fit

Only on a terminal, and only in lists and tables:

- **A path** that would make its row pass the width loses whole folders from its middle, replaced by one cut mark, keeping its first folder and its file's name: `Assets/…/Rock_Albedo.psd`. If that still doesn't fit, the path is left as it is and the row runs long.
- **A message** in a list is cut at its end, with the cut mark.
- **IDs, numbers, errors and hints** are never cut.
- Widths count what is printed: the escaped form of a project's text, and the plain form of a symbol.

## 8. Progress

- On stderr, only when it is a terminal and `--json` wasn't given; nothing is printed while a command works otherwise.
- Nothing shows for the first 400 ms. Then one line, redrawn in place at most every 100 ms, and erased when the work ends: the outcome line is the summary.
- **A bar,** when the total is known:

```text
Importing commits        ██████░░░░░░░░░░░░░░░░░░  26%  334 / 1,284
```

  the label padded to 24 characters (cut with the cut mark when longer), a space, 24 cells, a space, the percentage right-aligned in 4 characters, two spaces, then done and total, as in the example. Under 70 columns the last part goes; under 50 the cells go too, leaving the label and the percentage. A line is kept short of the terminal's last column, so it never wraps.
- **A turning mark,** when it isn't: the label, a space, the cut mark, a space and one of `⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏` in turn (`|/-\` without symbols).
- **Who shows what:** `import` shows a bar for each of its passes (reading Git's history, checking Git LFS files, importing commits); `verify` shows a bar by bytes read; `save`, `goto`, `discard`, `shelf`, `unshelf`, `undo`, `redo`, `tidy` and `setup --preset` show the turning mark with what they are doing ("Saving", "Writing files", "Tidying").
- A question, or the line that says Bolt is waiting for another command, takes the progress line off the screen first. So does any line told on stderr while the turning mark turns, such as `tidy`'s "Tidying: about … to free"; the mark comes back below it. The line is erased with `ESC[2K` where stderr shows colour, and with spaces over it otherwise.

## 9. Numbers, times and names

- **Sizes:** 1,024 bytes to the KiB: `512 B`, `31 KiB`, `240 MiB`, `1.2 GiB`. One decimal below 10 of a unit, none from 10 up; bytes always whole. A comma between thousands in every unit (`1,023 B`, `1,023 KiB`). **A size that is a setting** (`bolt setup`, and what `bolt start` says about a preset) is shown as `bolt.toml` would write it: the largest unit that gives the number of bytes exactly, the 1,024 family tried first, so 100,000,000 bytes is `100 MB` and 1,073,741,824 is `1 GiB`.
- **Counts:** a comma between thousands (`12,000 files`); real plurals (`1 file`, `2 files`).
- **Times:** `just now`, `5 min ago`, `2 h ago`, `1 day ago`, `3 days ago` up to 30 days, then the date, in the save's own time zone.
- **Change IDs:** the shortest start no other save on the line shares, at least 4 letters. **Fingerprints and diary entries:** 12 hex digits in lists.
- **Paths** are relative to the project's top with `/` on every system; the project's own folder is shown as the system writes it.
- **Messages** are in straight double quotes, their first line only in lists.

## 10. Questions

A question is asked only when stdin and stderr are both terminals and `--json` wasn't given. It says the situation first, then asks, with the default in square brackets:

| Question | Prompt | Flag that answers it | Without a terminal |
|---|---|---|---|
| A save that adds a lot | `Save them? [y/N]` | `--yes` | stops, saving nothing |
| A save away from a line | `Name a new line for this save:` | `--line <name>` | stops, saving nothing |
| Which preset (`bolt start`) | `Choice [N]:` | `--preset <name>` | stops, starting nothing |
| A preset switch (`bolt setup --preset`) | `Apply these changes? [y/N]` | `--yes` | stops after showing the changes |

Every stop is an `error:` whose `hint:` names the flag.

## 11. Help

- `bolt` alone, `bolt help` and `bolt --help` print one screen: a sentence about Bolt, the usage line, and the commands in three groups, one line each: every day (`status`, `save`, `changes`, `history`, `goto`, `undo`); lines, shelves and details (`line`, `shelf`, `discard`, `show`); the project (`start`, `setup`, `import`, `tidy`, `verify`). `redo` and `unshelf` are named beside `undo` and `shelf`.
- `bolt help <command>` and `bolt <command> --help` print the command's one-line description, then examples, then usage, then its options, each on one line. Only options the command really takes are listed.
- Git's words appear nowhere in help.
- Help is plain text on stdout, with section titles in bold on a terminal.

## 12. Each command

Filled in while milestone 6 restyles each command: its outcome line in each state, its details, and its next step.
