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
- **A message** in a list is cut at its end, with the cut mark, never inside an escape (`\x1b`, `\u{202e}`): a cut that would fall there falls before its backslash.
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

- **The three checks,** in this order, each a `✓` line when it found nothing, else its `✗` lines: the stored objects (`pack_damaged`, `pack_unreadable` and `object_damaged` problems; its `✓` line says `N of M` when some objects failed, and shows only while some matched), what the saves, lines and shelves need (`object_missing`), and the diary.
- **When what the project needs couldn't be worked out,** there is no line for the second and third checks, which never ran: in their place one `✗` line gives the problem's message. A lost `head` (`head_lost`) says why it is lost; a check that couldn't be made for another reason, such as a pack Bolt couldn't read while it looked, says `What the project needs couldn't be checked: <why>.` (`pack_unreadable`).
- **A kind's `✗` line** counts every problem of that kind found, kept or not (`1 pack is damaged:`, `2 problems reading packs:`, `1 object is damaged:`, `3 objects the project needs are missing:`), one line per kind, and its rows follow, indented 8: the object's fingerprint (12 hex digits) or the pack's file name, two spaces, then the problem's message, then, for something the project needs, `Save <12 hex digits> needs it at <path>.` (`… at its top folder.` for the top folder). At most 20 rows a kind, then `… and N more (bolt verify --json lists up to 1,000)`, N counting the rest of that kind.
- **Something missing** says `It isn't in any pack.` when every pack was read, and `It isn't in any pack Bolt could read.` when some pack couldn't be opened or read. One stored as another kind of object than what needs it says `It is stored, but as another kind of object than what needs it.` All three are `object_missing`.
- **Colour:** `Verified` green and bold; `✓` green; `✗` red; fingerprints and pack names blue, paths blue. Nothing else.
- **Without symbols** (§4) the marks are `+` and `x`, and the cut mark is `...`. Without a terminal it is the same text, plain, and stderr holds only the warnings and the hint, with no bar.
- **The hint** says the same however the problems were found. When `head` is lost it first gives that error's own hint (`bolt undo --list`, then `bolt undo --to`). There is no `error:` line: the outcome line on stdout is the error, and `--json`'s `error` holds it, with the hint.
- A switch left unfinished is a `warning:` (`switch_unfinished`), as for `bolt status`, and doesn't change the exit code.

### `bolt save`

While it works, a terminal's stderr shows the turning mark `Saving` (§8).

| State | Outcome line | Details | Next step |
|---|---|---|---|
| Saved | `Saved <id> "<message's first line>" on <line>  (2 files)` | | |
| Saved on a new line (`--line`, or the name asked for) | the same, `on` the new line | `You're on the new line <line>.` | |
| Saved what paths name | the same | `Only what you named was saved: bolt status lists the changes left out.` | |
| Updated (`--amend`) | `Updated save <id> "<message's first line>" on <line>  (2 files)` | `The ID stays the same.` | |
| The files couldn't be counted | the same, ending `(couldn't count the changed files)` | | |
| Nothing to save | `Nothing to save: no changes since your last save, <id> "<message>".` (`Nothing to save there: it matches your last save, …` with paths) | `No line was made: …` when `--line` was given | |
| Nothing to amend | `Nothing to amend: your files match your last save, <id> "<message>", and so does the message.` (`what you named matches` with paths) | | |

- The count is files, never lines: counting lines would slow every save. `on <line>` is left out for a save that names no line.
- **Colour:** `Saved` and `Updated` green and bold, the ID magenta and bold, the line blue, the count dim, the `bolt status` of the partial-save line cyan.
- The questions and stops (a large save, a save away from a line, the first save's line endings and Git LFS pointers) keep their words and flags (§10); only their layout follows §6.

### `bolt status`

| State | Outcome line | Details | Next step |
|---|---|---|---|
| On a line | `On line <line> at save <id> "<message>"  (2 h ago)` | | |
| At a save | `At save <id> "<message>", not on a line  (2 h ago)` | | |
| Nothing changed | the outcome line, then at once `Nothing changed since then.` | | |
| Changes | the outcome line; after an empty line `Changed since then (3 files):` | after an empty line, a row a change | `Next: save them with: bolt save "describe your change"` |
| Entries not written here | after the rest, `Not written here, because this computer can't hold them as they are (saves keep them as they were):` | a path a row | |

- **A row:** the kind (`added`, `removed`, `modified`, `kind changed`), the path, then what it holds, right-aligned: the size, `2 B → 4 B` for a modified file, `file → folder` for a kind changed. Rows are in path order; each column is as wide as what it holds. On a terminal a path that would pass the width loses folders from its middle (§7), and the column is as wide as the widest path shown.
- The time is when the save was written (its author's). Sizes are measured; no line counts.
- **Colour:** the ID magenta and bold, the line and the paths blue, `added` green, `removed` red, `modified` and `kind changed` yellow, the time and the count dim.
- `--full` changes how hard status looks, not what it prints. An unfinished switch is a `warning:` (`switch_unfinished`) after the answer.

### `bolt changes`

| State | Outcome line | Details | Next step |
|---|---|---|---|
| Changes | none: each file's header, then its hunks | | |
| Nothing changed | `Nothing changed since <id> "<message>".` (`Nothing changed in those paths since …` with paths) | | |
| `--save`, nothing | `Save <id> "<message>" changed no files.` (`… no files in those paths.`) | | |

- **A file's header:** the kind, the path, `+N -M` (lines added and removed) for a text file, then what it holds in brackets, as in `bolt status`: `modified  Assets/Player.cs  +2 -1  (6 B → 7 B)`. A file shown without lines says why after it (`(binary: no lines shown)`, `(over 64 MiB: no lines shown)`, `(over 1,000,000 lines: no lines shown)`), and a line-ending-only change says so (`(only line endings changed: LF to CRLF)`).
- **Hunks** are standard: `@@ -a,b +c,d @@`, three lines of context, `(CRLF)` after a line of a pair that differs only in its ending.
- **Between files:** on a terminal the dim `─` rule, one column short of the width and at most 100 long; without a terminal an empty line, as no line of a file can be one and a rule of `-` could pass for a removed line.
- **Colour:** the kind as in `bolt status`, the path blue, `+N` and `+` lines green, `-M` and `-` lines red, `@@` lines cyan, the bracket dim. Context lines have none.
- There is no summary line first: `bolt changes` prints each file as it compares it, and knows the totals only at the end. A diff that took over 2 seconds is a `warning:` (`diff_timed_out`).

### `bolt history`

```text
Saves on line main, newest first:

▸ ptvz   2 h ago     Ana Lima    Add double jump  [main]
  kxqmz  3 days ago  Ana Lima    First playable
  kxqmr  5 days ago  Ben Okafor  Add rock textures
```

- **The heading:** `Saves on line <line>, newest first:`; at a save, `At save <id>, not on a line. Saves back from it, newest first:`. Every save is listed, from where you are back. When the first save can't be read, stdout gets nothing, not even the heading; when a later one can't, the rows before it are printed, then the error.
- **A row:** the place marker on the save you're at (the first row), the ID, when it was written, its author, its message's first line, then `[<line>]` for each line whose newest save it is. Columns are as wide as the first rows need, up to 1,000 rows or 1 MiB of their text (escaped), whichever comes first: `history` never holds a whole line, so a wider value after them runs long. On a terminal the message is cut at its end to fit, never the ID or the names.
- **Colour:** the marker bold, IDs magenta and bold, the time and the author dim, line names blue.

### `bolt show`

```text
Save ptvz "Add double jump"

    Author       Ana Lima <ana@example.com>, 2026-11-08 12:14 +01:00  (2 h ago)
    Saved by     Ana Lima <ana@example.com>, 2026-11-08 12:14 +01:00  (2 h ago)
    Line         main
    After        3f9a0c2e7b04 "First playable"
    Fingerprint  d77a81f736da0b96a75602567a49fa760764e22213b6852cda7b6660f926f3ed

        Second jump while airborne, plus 0.1 s of coyote time off ledges.

2 files changed:

    added     Assets/Scripts/DoubleJump.cs    1.2 KiB
    modified  Assets/Scripts/Player.cs      6 B → 7 B
```

then, on a terminal, `Next: see its lines with: bolt changes --save <id>` when it changed files.

- **The facts:** the author and the saver, each with the date and time in the save's own zone and how long ago; `Line` is the line the save was made on, `none` for an imported save; `After` names the save before it by the first 12 hex digits of its fingerprint, which `bolt show` takes back, and its message (a merge brought in from Git adds the other parents' fingerprints); no change ID is shortened for it, as that could read the whole line; the fingerprint is whole.
- **The message** after its first line is indented 8, so no line of it can pass for a change row, which is indented 4; its blank lines stay blank.
- **The changes** are rows as in `bolt status`, after `N files changed:`; `No files changed.` when there are none. The columns are measured by the walk that checks the save before anything is printed; a path cut on a terminal leaves its column as wide as the terminal lets it be.
- **Colour:** IDs magenta and bold, the line blue, the fingerprints blue, the rows as in `bolt status`, the times in brackets dim.
