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

  the label padded to 24 characters (cut with the cut mark when longer), a space, 24 cells, a space, the percentage right-aligned in 4 characters, two spaces, then done and total, as in the example. A progress line takes at most one column less than the terminal's width, so it never wraps, and it never shows part of a number or part of the bar: when the whole line doesn't fit, the counts go whole; if it still doesn't fit, the cells go whole too. The label and the percentage always show, the label cut at its end with the cut mark only when even they don't fit.
- **A turning mark,** when it isn't: the label, a space, the cut mark, a space and one of `⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏` in turn (`|/-\` without symbols). The label is cut at its end, the same way, when the line wouldn't fit.
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

### `bolt start`

| State | Outcome line | Details | Next step |
|---|---|---|---|
| Asked (terminal, no `--preset`, no `bolt.toml`) | stderr first: `Looks like a game project: found Unity files and 2.3 GiB of files.` (a code project; or `Bolt can't tell what kind of project this is: it found no game engine's or language's project files, and …`; `more than` when the look stopped early), then the three choices with `← suggested` (dim) and `Choice [N]:` | | |
| Started with Code or Game | `Started a Bolt project in <folder>` | `Using Game settings (Unity):` then bullets: chunked size, what the preset's lines leave out, the large-save numbers, and for Game what is lockable; then `The rules are in bolt.toml and .boltignore. See them any time with: bolt setup`; then `Your first save is <id> on line main.` | `Next: change some files, then run: bolt save "describe your change"` |
| Started with Custom | the same | `Using Custom settings: Bolt's plain defaults, with no rule files:` then the chunked size and the large-save numbers; `There are no rule files. See the rules any time with: bolt setup` | the same |
| The folder has a `bolt.toml` | the same | `Using the rules in this folder's bolt.toml, which stay as they are:` then its chunked size and large-save numbers; `See them any time with: bolt setup` | the same |
| No terminal or `--json`, no `--preset` | `error: Nothing was started: Bolt can't ask what kind of project this is without a terminal.` With `--json`, the document's `error` says it instead, ending `… this is with --json.` | `hint:` names `--preset code`, `--preset game`, `--preset custom`, and the suggestion with what the look found, in fixed words | |
| A wrong answer three times, or the input ended | `error: Nothing was started: no kind of project was chosen.` | the same hint | |
| `--preset` where a `bolt.toml` is | `error: Nothing was started: this folder already has a bolt.toml…` | `hint:` start without `--preset`, then `bolt setup --preset <name>` | |
| Made, rule files not written | the warnings found so far, then `error: The project was started, but its rule files weren't written: …` | `hint: Run bolt setup --preset <name> to write them.` | |

A wrong answer says `Answer 1, 2 or 3, or a preset's name: code, game or custom.` and asks again, three times in all. Sizes of settings are as `bolt.toml` writes them (§9); the folder's size is measured.

### `bolt setup`

| State | Outcome line | Details | Next step |
|---|---|---|---|
| No flag, with a `bolt.toml` | `Using <label> settings, from bolt.toml:` (`Using the rules in bolt.toml:` without a label) | rows: `Files stored in chunks`, `A save asks first`, `Lockable`, `No conflict markers`, `Left out by the preset` (`N lines in .boltignore`); `none` where there are none | `Next: edit bolt.toml or .boltignore to change a rule, or switch presets with: bolt setup --preset <name>` |
| No flag, no `bolt.toml` | `No bolt.toml here, so Bolt's own values apply:` | the same rows | the same |
| No flag, a `bolt.toml` Bolt can't use (not TOML, a folder, over 1 MiB) | `bolt.toml couldn't be used, so Bolt's own values apply:`, and a `warning:` says why | the same rows | the same |
| `--preset`, nothing to change | `Nothing to change: the rule files already are what the Code preset writes.` | | |
| `--preset`, a preview | `Switching to Code settings changes these files:` | each file's change as `bolt changes` shows it, one empty line between files; then `Apply these changes? [y/N]` on stderr | |
| Answered no | `Nothing was changed.` | | |
| Applied | `Switched to Code settings: made bolt.toml, changed .boltignore.` | | `Changed your mind? bolt undo` |
| No terminal or `--json`, no `--yes` | the preview, then `error: Nothing was changed: Bolt can't ask whether to apply these changes without a terminal.` With `--json`, there is no preview, and the document's `error` says it instead, ending `… these changes with --json.` | `hint: If they are what you want, run: bolt setup --preset <name> --yes` | |

Problems in `bolt.toml` come as `warning:` lines after the list. The engine's refusals (a rule file that is a link or a folder, one over 1 MiB, one changed since the preview) are errors with their own hints.

### `bolt verify`

`<project>` is the name of the project's folder. While it reads the packs, a terminal's stderr shows the bar `Checking stored objects`, by bytes of pack read out of bytes in all (§8); the total grows when a pack arrives during the check.

Nothing wrong (exit code 0), on stdout:

```text
Verified Skyrunner: no problems found.

    ✓ 12,480 stored objects in 3 packs (4.8 GiB) match their fingerprints
    ✓ 1,290 saves, 3 lines and 2 shelves point to objects that exist
    ✓ the undo diary (24 entries) is intact
```

Problems (exit code 1), on stdout:

```text
Checked Skyrunner: 3 problems found.

    ✓ 12,479 of 12,480 stored objects in 3 packs (4.8 GiB) match their fingerprints
    ✗ 1 object is damaged:
        4c2e9a01f7b3  It doesn't match its fingerprint. Save 1a2b3c4d5e6f needs it at Assets/Rock.psd.
    ✗ 2 objects the project needs are missing:
        b17f0d9e2a64  It isn't in any pack. Save 1a2b3c4d5e6f needs it at Assets/Scripts/OldInput.cs.
        9d0e44a1c3b2  It isn't in any pack.
    ✓ the undo diary (24 entries) is intact
```

then, after the warnings, on stderr:

```text

hint: Bolt can't repair a project in this version. Saves that don't use the damaged or missing data still work; keep any other copy of this project safe.
```

- **The three checks,** in this order, each a `✓` line when it found nothing, else its `✗` lines: the stored objects (`pack_damaged`, `pack_unreadable` and `object_damaged` problems; its `✓` line says `N of M` when some objects failed, and shows only while some matched), what the saves, lines and shelves need (`object_missing`), and the diary (`head_lost`, whose `✗` line is the problem's message, and then there is no line for the second check, which couldn't run).
- **A kind's `✗` line** counts its problems (`1 pack is damaged:`, `2 packs couldn't be read:`, `1 object is damaged:`, `3 objects the project needs are missing:`), and its rows follow, indented 8: the object's fingerprint (12 hex digits) or the pack's file name, two spaces, then the problem's message, then, for something the project needs, `Save <12 hex digits> needs it at <path>.` (`… at its top folder.` for the top folder). At most 20 rows a kind, then `… and N more (bolt verify --json lists up to 1,000)`. When more than 1,000 problems were found, a last line says how many more were counted but not kept.
- **Colour:** `Verified` green and bold; `✓` green; `✗` red; fingerprints and pack names blue, paths blue. Nothing else.
- **Without symbols** (§4) the marks are `+` and `x`, and the cut mark is `...`. Without a terminal it is the same text, plain, and stderr holds only the warnings and the hint, with no bar.
- **The hint** says the same however the problems were found. When `head` is lost it first gives that error's own hint (`bolt undo --list`, then `bolt undo --to`). There is no `error:` line: the outcome line on stdout is the error, and `--json`'s `error` holds it, with the hint.
- A switch left unfinished is a `warning:` (`switch_unfinished`), as for `bolt status`, and doesn't change the exit code.
