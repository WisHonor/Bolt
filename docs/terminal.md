# Opalro in the terminal

> **Status:** written on 2026-11-09 from [decision 120](decisions/120-the-terminal-ui.md), before the code that follows it (milestone 6). The look is the owner's design canvas; every fact is Opalro's own ([the canvas review](plans/2026-11-09-terminal-ui-canvas-review.md)). §12 is filled in as each command is restyled.

**In plain words.** This page is the rulebook for what Opalro prints for people: where each line goes, how it is laid out, which words get which colour, which symbols are used and what stands in for them, and how all of it changes when nobody is looking at a terminal. The `--json` formats have their own page ([json.md](json.md)).

## 1. Streams

| Stream | What goes there |
|---|---|
| stdout | The command's answer: the outcome line, its details, lists and tables. |
| stderr | What talks to the person: `error:`, `warning:`, `hint:`, questions and their choices, progress, `Next:` and "Changed your mind?" lines. |

Order: the answer, then the warnings, then the next step. With `--json`, stdout holds one document and none of this page applies to it.

## 2. What Opalro finds out first

Each is found out once, when the command starts, for stdout and stderr separately.

- **A terminal:** the stream is one (`std::io::IsTerminal`).
- **Colour** on a stream: it is a terminal, `NO_COLOR` is unset or empty, `TERM` isn't `dumb`, `--json` wasn't given, and on Windows either `WT_SESSION` is set or the console agreed to read colour codes (`anstyle_query::windows::enable_ansi_colors()` returned `Some(true)`; it asks for stdout and stderr together and fails unless both are the console, so on the older console redirecting either leaves the other without colour).
- **Symbols** on a stream: it is a terminal and, on macOS and Linux, the first of `LC_ALL`, `LC_CTYPE` and `LANG` that is set and not empty names UTF-8 (`UTF-8` or `utf8`, in any letter case); on Windows, `WT_SESSION` or `TERM_PROGRAM` is set. `OPALRO_ASCII=1` turns symbols off everywhere. `OPALRO_ASCII=0` turns them on for any terminal. No other value of `OPALRO_ASCII` means anything.
- **Width** of a stream that is a terminal: what the system says (the `terminal_size` crate); 80 when it won't say. A stream that isn't a terminal has no width: nothing on it is wrapped or cut.

Tests run without a terminal, so they see no colour, no symbols, no wrapping and no progress.

## 3. Styles

Opalro writes the terminal's own colours (SGR codes), never RGB, and ends each styled piece with `ESC[0m`.

| Role | Code | Carrier without colour |
|---|---|---|
| The outcome's first word (`Saved`, `Undid`…) | `1;32` green, bold | the word, first on the first line |
| `error:` | `1;31` red, bold | the word |
| `warning:` | `1;33` yellow, bold | the word |
| `hint:`, `Next:` | `1;36` cyan, bold | the word |
| A command to type | `36` cyan | starts with `opalro `, after a colon or on a line of its own |
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
| `✓` `✗` | `+` `x` | `opalro verify`'s checks, and nowhere else |
| `─` | `-` | the separator between files in `opalro changes` |
| `…` | `...` | something cut short; work in progress |
| `█` `░` | `#` `.` | the cells of a progress bar |

Opalro draws no boxes, banners or rules beyond that separator.

## 5. Layout

- **The outcome line** is first, starts at the left edge with what happened in the past tense, and is one line. A command that only reports (`status`, `history`, `line`, `show`) starts with where you are instead. "Nothing to …" lines are plain, and the command ends with exit code 0.
- **Details** come after one empty line, indented 4 spaces. Bullets are the bullet symbol and one space. On a terminal a detail that is a sentence wraps as §6 says, its later lines starting under its text: 4 spaces in, or under the text after a bullet or a check mark when the line starts with one. The outcome line is one line and is never wrapped; the rows of lists and tables are cut as §7 says, never wrapped.
- **Text from the project or a Git repository** (a name, a path, a message) is never broken by wrapping and never starts a line 4 spaces in, where it could pass for one of Opalro's details. Where it doesn't fit beside the words that introduce it, it goes on a row of its own, 4 spaces further in than the text of its detail (8 in for a plain detail, 10 under a bullet or a check mark), as the rows of a list are; a row still too long runs long. A command Opalro names inside a sentence (`opalro status`) is never broken either.
- **Lists with a place marker** put the marker at the left edge and start every other row with two spaces.
- **Columns** are at least two spaces apart; text is left-aligned, numbers and sizes right-aligned. A table is as wide as what it holds and is never padded out to the terminal's width.
- **Empty lines:** one after an outcome line that has details; one before the first `warning:` when anything was printed before it (the answer counts only when stdout and stderr are both a terminal), so stderr never starts with an empty line; one before a `hint:`, a `Next:` or a "Changed your mind?" line. Never two in a row, never one at the end.

## 6. Errors, warnings and the next step

- **`error:`** then the message; **`hint:`** after one empty line. Their words are Opalro's own, from the code that found the problem; this page only lays them out.
- **`warning:`** lines come after the answer, one per warning, in the order met, never merged into a count.
- **`Next:`** names the most likely next command: at most one per command, last. **`Changed your mind? opalro undo`** ends `discard`, `goto`, `unshelf` and `setup --preset`. Both, and the hint the first time a Git word is used ([decision 116](decisions/116-gits-words-as-shortcuts.md)), are printed **only when stderr is a terminal** and `--json` wasn't given. Without a terminal, stderr holds only errors, their hints and warnings, as before milestone 6.
- **Wrapping** happens only on a terminal: a message longer than the smaller of the width and 100 columns breaks at a space, the spaces at the break dropped, and the lines after the first start under the message's text: 7 spaces in for `error:`, 9 for `warning:`, 6 for `hint:` and `Next:`, and as §5 says for a detail. A line with no space to break at is left long. A message that already has several lines keeps its own line breaks, each line wrapped the same way. Without a terminal nothing wraps.
- **Exit codes:** 0 done, or nothing to do; 1 failed; 2 a command line Opalro couldn't use. Nothing else.

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
- A question, or the line that says Opalro is waiting for another command, takes the progress line off the screen first. So does any line told on stderr while the turning mark turns, such as `tidy`'s "Tidying: about … to free"; the mark comes back below it. The line is erased with `ESC[2K` where stderr shows colour, and with spaces over it otherwise.

## 9. Numbers, times and names

- **Sizes:** 1,024 bytes to the KiB: `512 B`, `31 KiB`, `240 MiB`, `1.2 GiB`. One decimal below 10 of a unit, none from 10 up; bytes always whole. A comma between thousands in every unit (`1,023 B`, `1,023 KiB`). **A size that is a setting** (`opalro setup`, and what `opalro start` says about a preset) is shown as `opalro.toml` would write it: the largest unit that gives the number of bytes exactly, the 1,024 family tried first, so 100,000,000 bytes is `100 MB` and 1,073,741,824 is `1 GiB`.
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
| Which preset (`opalro start`) | `Choice [N]:` | `--preset <name>` | stops, starting nothing |
| A preset switch (`opalro setup --preset`) | `Apply these changes? [y/N]` | `--yes` | stops after showing the changes |

Every stop is an `error:` whose `hint:` names the flag.

## 11. Help

- `opalro` alone, `opalro help` and `opalro --help` print one screen: a sentence about Opalro, the usage line, and the commands in three groups, one line each: every day (`status`, `save`, `changes`, `history`, `goto`, `undo`); lines, shelves and details (`line`, `shelf`, `discard`, `show`); the project (`start`, `setup`, `import`, `tidy`, `verify`). `redo` and `unshelf` are named beside `undo` and `shelf`, as `(and: redo)`. The last line says `Help on one command: opalro help <command>    Version: opalro --version`. Each command's name is padded to 10.
- `opalro help` and `opalro --help` end with exit code 0. `opalro` alone prints the same screen, also on stdout, and ends with 2, as before milestone 6: a command line with no command is one Opalro couldn't use.
- `opalro help <command>` and `opalro <command> --help` print the same page: `opalro <command>: ` and the command's one-line description, then `Examples` (each a command line, then what it does, the lines padded to the longest), `Usage`, `Arguments` (when the command takes any) and `Options`, each a title on a line of its own, its lines under it flush left, one line each. The usage, the arguments and the options are read from the command line's own definitions, so only options the command really takes are listed. Every example is a command line Opalro takes.
- Git's words appear nowhere in help. A command line Opalro can't use (an unknown command or option) gets the command-line reader's own message on stderr and exit code 2, as before.
- Help is plain text on stdout, with section titles in bold on a terminal, and nothing else styled. It is never wrapped or cut: it reads the same at any width and in a pipe.

## 12. Each command

Filled in while milestone 6 restyles each command: its outcome line in each state, its details, and its next step.

### `opalro start`

| State | Outcome line | Details | Next step |
|---|---|---|---|
| Asked (terminal, no `--preset`, no `opalro.toml`) | stderr first: `Looks like a game project: found Unity files and 2.3 GiB of files.` (a code project; or `Opalro can't tell what kind of project this is: it found no game engine's or language's project files, and …`; `more than` when the look stopped early), then the three choices with `← suggested` (dim) and `Choice [N]:` | | |
| Started with Code or Game | `Started an Opalro project in <folder>` | `Using Game settings (Unity):` then bullets: chunked size, what the preset's lines leave out, the large-save numbers, and for Game what is lockable; then `The rules are in opalro.toml and .opalroignore. See them any time with: opalro setup`; then `Your first save is <id> on line main.` | `Next: change some files, then run: opalro save "describe your change"` |
| Started with Custom | the same | `Using Custom settings: Opalro's plain defaults, with no rule files:` then the chunked size and the large-save numbers; `There are no rule files. See the rules any time with: opalro setup` | the same |
| The folder has an `opalro.toml` | the same | `Using the rules in this folder's opalro.toml, which stay as they are:` then its chunked size and large-save numbers; `See them any time with: opalro setup` | the same |
| No terminal or `--json`, no `--preset` | `error: Nothing was started: Opalro can't ask what kind of project this is without a terminal.` With `--json`, the document's `error` says it instead, ending `… this is with --json.` | `hint:` names `--preset code`, `--preset game`, `--preset custom`, and the suggestion with what the look found, in fixed words | |
| A wrong answer three times, or the input ended | `error: Nothing was started: no kind of project was chosen.` | the same hint | |
| `--preset` where an `opalro.toml` is | `error: Nothing was started: this folder already has an opalro.toml…` | `hint:` start without `--preset`, then `opalro setup --preset <name>` | |
| Made, rule files not written | the warnings found so far, then `error: The project was started, but its rule files weren't written: …` | `hint: Run opalro setup --preset <name> to write them.` | |

A wrong answer says `Answer 1, 2 or 3, or a preset's name: code, game or custom.` and asks again, three times in all. Sizes of settings are as `opalro.toml` writes them (§9); the folder's size is measured.

### `opalro setup`

| State | Outcome line | Details | Next step |
|---|---|---|---|
| No flag, with an `opalro.toml` | `Using <label> settings, from opalro.toml:` (`Using the rules in opalro.toml:` without a label) | rows: `Files stored in chunks`, `A save asks first`, `Lockable`, `No conflict markers` (`none` where there are none), `Left out by the preset` (`N lines in .opalroignore`, or `no preset lines in .opalroignore`) | `Next: edit opalro.toml or .opalroignore to change a rule, or switch presets with: opalro setup --preset <name>` |
| No flag, no `opalro.toml` | `No opalro.toml here, so Opalro's own values apply:` | the same rows | the same |
| No flag, an `opalro.toml` Opalro can't use (not TOML, a folder, over 1 MiB) | `opalro.toml couldn't be used, so Opalro's own values apply:`, and a `warning:` says why | the same rows | the same |
| `--preset`, nothing to change | `Nothing to change: the rule files already are what the Code preset writes.` | | |
| `--preset`, a preview | `Switching to Code settings changes these files:` | each file's change as `opalro changes` shows it, one empty line between files; then `Apply these changes? [y/N]` on stderr | |
| Answered no | `Nothing was changed.` | | |
| Applied | `Switched to Code settings: made opalro.toml, changed .opalroignore.` | | `Changed your mind? opalro undo` |
| Every file changed between the plan and the write | `Nothing was written: the rule files changed while Opalro worked. Run opalro setup --preset code again.` (exit code 0) | the `changed_since_plan` warning names the files; the step is in the undo diary all the same, as for any switch that left files alone, and an `opalro undo` of it stops rather than write over the files as they are now | |
| No terminal or `--json`, no `--yes` | the preview, then `error: Nothing was changed: Opalro can't ask whether to apply these changes without a terminal.` With `--json`, there is no preview, and the document's `error` says it instead, ending `… these changes with --json.` | `hint: If they are what you want, run: opalro setup --preset <name> --yes` | |

The patterns of the `Lockable` and `No conflict markers` rows are separated by spaces; one that holds a space of any kind or a double quote is between double quotes, as `opalro.toml` writes it, with `\"` and `\\` for a double quote and a backslash inside them (`"My Art/*.psd" *.png`). Problems in `opalro.toml` come as `warning:` lines after the list. The engine's refusals (a rule file that is a link or a folder, one over 1 MiB, one changed since the preview) are errors with their own hints.

### `opalro import`

It takes no argument: run inside a Git repository's folder, it makes `.opalro/` there, next to `.git/`, and never changes the Git repository. Uncommitted Git work isn't refused: it shows as unsaved changes and goes into the next save. While it works, a terminal's stderr shows each pass in turn (§8): `Reading Git's history` with the turning mark, as how many commits there are isn't known yet; then the bars `Checking Git LFS files`, when the repository shows a sign of Git LFS, and `Importing commits`, both by commits.

```text
Imported 1,284 commits from Git: 3 lines, 2 server lines from 1 server, 12 tags.

    Renamed 2 names that Opalro's names don't allow:
        feature+x -> feature-x
        tag v1:0 -> v1-0
    Didn't import 1 ref (only branches, server branches and tags come in): refs/notes/commits
    Brought in 41 Git LFS files (11 GiB) from .git/lfs/objects.

You're on line main.
```

| State | Outcome line | Details | Next step |
|---|---|---|---|
| Imported, on a line | `Imported <N> commits from Git: <N> lines, <N> tags.` (`, <N> server lines from <N> servers` after the lines when Git had any) | what was renamed or left out, and what a later export can't recreate, a detail each; then, after an empty line, `You're on line <line>.` | `Next: look around with: opalro status` |
| Imported, Git's `HEAD` detached | the same | the same, ending `You're at save <id>, where Git's HEAD was detached: not on a line. Saving here asks for a new line's name.` | the same |
| Refused (not a Git repository's folder, a second import, a project already here, a repository Opalro can't read) | stdout gets nothing; `error:` says why, `hint:` what to do | | |
| A Git LFS stop: objects missing (the first 20, with the commit and size, then `and N more`), kept elsewhere (`lfs.storage`, or a link at `.git/lfs`), a pointer with `ext-` lines, an object whose SHA-256 differs | stdout gets nothing; `error:` in the import's own words, `hint:` as decisions 085, 089 and 091 word it, naming Git's commands and none of Opalro's flags | | |
| The work stop | stdout gets nothing; `error: Opalro can't import this Git repository: rebuilding objects from Git's deltas would take more than the <N> GiB of work Opalro allows: the repository holds a long history of small edits to a big file, or it was built to make importing slow.` | ``hint: Nothing was changed. If you trust this repository, have Git store its files over 1 MiB whole and make its other deltas anew, with `git -c core.bigFileThreshold=1m repack -a -d -f`, then run opalro import again. Git's packs stay bigger until its next `git gc`.`` (when it stopped after `.opalro` was begun, the hint of the last row, with this advice) | |
| Stopped after `.opalro` was begun | stdout gets nothing; `error:` as the stop says it | `hint:` the stop's own advice without its `Nothing was changed.`, then, on a line of its own: when nothing was committed yet, both that nothing was committed and that Opalro kept the `.opalro` folder it began, so that `opalro import` can finish from what it stored (or stops the same way until the cause is fixed), or the folder may be deleted; when the history was committed, that it is stored and that `opalro import` run again finishes the project | |

- **The details,** in this order, each indented 4 and only when there is something to say: the names renamed because Opalro's rules refuse them (bad characters become `-`, clashes get `-2`); the refs not imported (only branches, server branches and tags come in); the tags not imported, with why; the entries left out, which are only `.git` and `.opalro` spellings, every hidden character shown; how many commits a later export can't recreate exactly; the Git LFS files brought in (count and size), the commits holding them, and the pointers not in Git LFS's exact form.
- **A list** of one is on its detail's line, after a colon; of more, its rows are indented 8 under it: the first 100, then `and N more`. Rows are names, never cut, so a long one runs long.
- Details wrap on a terminal as §5 says. A list of one sits on its detail's line only when it fits there; otherwise its item takes a row of its own, 8 in, as a longer list's rows do, since it is the repository's text.
- The import's warnings (such as `lfs_lookalikes` and `elevated_only`) come on stderr after the summary, one `warning:` each, before the `Next:` line.
- **Colour:** `Imported` green and bold, the line blue, the save's ID magenta and bold.

### `opalro verify`

It changes nothing and takes no lock. Like every command, it makes the two empty working folders `.opalro/tmp` and `.opalro/cache` if they are missing. `<project>` is the name of the project's folder. While it reads the packs, a terminal's stderr shows the bar `Checking stored objects`, by bytes of pack read out of bytes in all (§8); the total grows when a pack arrives during the check.

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

hint: Opalro can't repair a project in this version. Saves that don't use the damaged or missing data still work; keep any other copy of this project safe.
```

- **The three checks,** in this order, each a `✓` line when it found nothing, else its `✗` lines: the stored objects (`pack_damaged`, `pack_unreadable` and `object_damaged` problems; when the packs couldn't be listed, its line is a `✗` saying why; its `✓` line says `N of M` when some objects failed, and shows only while some matched), what the saves, lines and shelves need (`object_missing` and `save_too_big`), and the diary. When the diary met an entry that is missing or can't be read (told among the problems, as `object_missing` or among the others), its line is `✗ the undo diary has a gap: Opalro could read N entries of it` instead, and nothing says the diary is intact. The line is about the current diary: an entry that only a diary cut off from it can't read is told among the problems, or as the `diary_end_unreadable` warning when the store lacks it, and is no gap.
- **When what the project needs couldn't be worked out,** there is no line for the second and third checks, which never ran: in their place one `✗` line gives the problem's message. A lost `head` (`head_lost`) says why it is lost; a check that couldn't be made for another reason, such as a pack Opalro couldn't read while it looked, says `What the project needs couldn't be checked: <why>.` (`pack_unreadable`).
- **A kind's `✗` line** counts every problem of that kind found, kept or not (`1 pack is damaged:`, `2 problems reading packs:`, `1 object is damaged:`, `3 objects the project needs are missing:`, `1 save is too big for any reader:`), one line per kind, and its rows follow, indented 8: the object's fingerprint (12 hex digits) or the pack's file name, two spaces, then the problem's message, then, for something the project needs, `Save <12 hex digits> needs it at <path>.` (`… at its top folder.` for the top folder). At most 20 rows a kind, then `… and N more`, N counting the rest of that kind. After the groups, when anything wasn't shown, one closing line: `opalro verify --json lists them all.` when there are at most 1,000 problems, `opalro verify --json lists the first 1,000.` when there are more.
- **Something missing** says `It isn't in any pack.` when every pack was read, and `It isn't in any pack Opalro could read.` when some pack couldn't be opened or read. One stored as another kind of object than what needs it (a chunk held only as a chunk list, say) says `It is stored, but as another kind of object than what needs it.` All three are `object_missing`.
- **A folder listing that gives a file another size** than what is stored for it (the contents' length, the length a chunk list gives, a link's target's length) is `object_damaged` on the listing: `It gives <name> as 999 bytes, but what is stored for it is 6 bytes.`, then the save and the file's path that need it.
- **A save too big for any reader** (`save_too_big`) is named by its fingerprint, then `No reader takes it: ` and the readers' own words: `it has more than 10000000 files and folders in total.`, `its folders nest more than 4096 levels deep.` or `a path in it is longer than 4096 bytes.`, counted by path, a listing reached by two paths counted twice.
- **Colour:** `Verified` green and bold; `✓` green; `✗` red; fingerprints and pack names blue, paths blue. Nothing else.
- **Without symbols** (§4) the marks are `+` and `x`, and the cut mark is `...`. Without a terminal it is the same text, plain, and stderr holds only the warnings and the hint, with no bar.
- **The hint** says the same however the problems were found. When `head` is lost it first gives that error's own hint (`opalro undo --list`, then `opalro undo --to`). There is no `error:` line: the outcome line on stdout is the error, and `--json`'s `error` holds it, with the hint.
- A switch left unfinished is a `warning:` (`switch_unfinished`), as for `opalro status`, and doesn't change the exit code. So is a diary cut off from the current one that can't be read back to its start (`diary_end_unreadable`), told once, as for `opalro tidy`.

### `opalro save`

While it works, a terminal's stderr shows the turning mark `Saving` (§8).

| State | Outcome line | Details | Next step |
|---|---|---|---|
| Saved | `Saved <id> "<message's first line>" on <line>  (2 files)` | | |
| Saved on a new line (`--line`, or the name asked for) | the same, `on` the new line | `You're on the new line <line>.` | |
| Saved what paths name | the same | `Only what you named was saved: opalro status lists the changes left out.` | |
| Updated (`--amend`) | `Updated save <id> "<message's first line>" on <line>  (2 files)` | `The ID stays the same.` | |
| The files couldn't be counted | the same, ending `(couldn't count the changed files)` | | |
| Nothing to save | `Nothing to save: no changes since your last save, <id> "<message>".` (`Nothing to save there: it matches your last save, …` with paths) | `No line was made: opalro line new <name> makes one without saving.` when `--line` was given | |
| Nothing to amend | `Nothing to amend: your files match your last save, <id> "<message>", and so does the message.` (`what you named matches` with paths) | | |

- The count is files, never lines: counting lines would slow every save. `on <line>` is left out for a save that names no line.
- **Colour:** `Saved` and `Updated` green and bold, the ID magenta and bold, the line blue, the count dim, the `opalro status` of the partial-save line cyan.
- The questions and stops (a large save, a save away from a line, the first save's line endings and Git LFS pointers) keep their words and flags (§10); only their layout follows §6.

### `opalro status`

| State | Outcome line | Details | Next step |
|---|---|---|---|
| On a line | `On line <line> at save <id> "<message>"  (2 h ago)` | | |
| At a save | `At save <id> "<message>", not on a line  (2 h ago)` | | |
| Nothing changed | the outcome line, then at once `Nothing changed since then.` | | |
| Changes | the outcome line; after an empty line `Changed since then (3 files):` | after an empty line, a row a change | `Next: save them with: opalro save "describe your change"` |
| Entries not written here | after the rest, `Not written here, because this computer can't hold them as they are (saves keep them as they were):` | a path a row | |

- **A row:** the kind (`added`, `removed`, `modified`, `kind changed`), the path, then what it holds, right-aligned: the size, `2 B → 4 B` for a modified file, `file → folder` for a kind changed. Rows are in path order; each column is as wide as what it holds. On a terminal a path that would pass the width loses folders from its middle (§7), and the column is as wide as the widest path shown.
- The time is when the save was written (its author's). Sizes are measured; no line counts.
- **Colour:** the ID magenta and bold, the line and the paths blue, `added` green, `removed` red, `modified` and `kind changed` yellow, the time and the count dim.
- `--full` changes how hard status looks, not what it prints. An unfinished switch is a `warning:` (`switch_unfinished`) after the answer.

### `opalro changes`

| State | Outcome line | Details | Next step |
|---|---|---|---|
| Changes | none: each file's header, then its hunks | | |
| Nothing changed | `Nothing changed since <id> "<message>".` (`Nothing changed in those paths since …` with paths) | | |
| `--save`, nothing | `Save <id> "<message>" changed no files.` (`… no files in those paths.`) | | |

- **A file's header:** the kind, the path, `+N -M` (lines added and removed) for a text file, then what it holds in brackets, as in `opalro status`: `modified  Assets/Player.cs  +2 -1  (6 B → 7 B)`. A file shown without lines says why after it (`(binary: no lines shown)`, `(over 64 MiB: no lines shown)`, `(over 1,000,000 lines: no lines shown)`), and a line-ending-only change says so (`(only line endings changed: LF to CRLF)`).
- **Hunks** are standard: `@@ -a,b +c,d @@`, three lines of context, `(CRLF)` after a line of a pair that differs only in its ending.
- **Between files:** on a terminal the dim `─` rule, one column short of the width and at most 100 long; without a terminal an empty line, as no line of a file can be one and a rule of `-` could pass for a removed line.
- **Colour:** the kind as in `opalro status`, the path blue, `+N` and `+` lines green, `-M` and `-` lines red, `@@` lines cyan, the bracket dim. Context lines have none.
- There is no summary line first: `opalro changes` prints each file as it compares it, and knows the totals only at the end. A diff that took over 2 seconds is a `warning:` (`diff_timed_out`).

### `opalro history`

```text
Saves on line main, newest first:

▸ ptvz   2 h ago     Ana Lima    Add double jump  [main]
  kxqmz  3 days ago  Ana Lima    First playable
  kxqmr  5 days ago  Ben Okafor  Add rock textures
```

- **The heading:** `Saves on line <line>, newest first:`; at a save, `At save <id>, not on a line. Saves back from it, newest first:`. Every save is listed, from where you are back. When the first save can't be read, stdout gets nothing, not even the heading; when a later one can't, the rows before it are printed, then the error.
- **A row:** the place marker on the save you're at (the first row), the ID, when it was written, its author, its message's first line, then `[<line>]` for each line whose newest save it is. Columns are as wide as the first rows need, up to 1,000 rows or 1 MiB of their text (escaped), whichever comes first: `history` never holds a whole line, so a wider value after them runs long. On a terminal the message is cut at its end to fit, never the ID or the names.
- **Colour:** the marker bold, IDs magenta and bold, the time and the author dim, line names blue.

### `opalro show`

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

then, on a terminal, `Next: see its lines with: opalro changes --save <id>` when it changed files.

- **The facts:** the author and the saver, each with the date and time in the save's own zone and how long ago; `Line` is the line the save was made on, `none` for an imported save; `After` names the save before it by the first 12 hex digits of its fingerprint, and its message (a merge brought in from Git adds the other parents' fingerprints); `opalro show` takes such digits back exactly when some line's history, along first parents, reaches that save: not, say, for a merge's other parent whose Git branch was gone before the import, or for the parent of a save an import left you at past every line (a detached Git `HEAD`), unless another line reaches it; no change ID is shortened for it, as that could read the whole line; the fingerprint is whole.
- **The message** after its first line is indented 8, so no line of it can pass for a change row, which is indented 4; its blank lines stay blank.
- **The changes** are rows as in `opalro status`, after `N files changed:`; `No files changed.` when there are none. The columns are measured by the walk that checks the save before anything is printed; a path cut on a terminal leaves its column as wide as the terminal lets it be.
- **Colour:** IDs magenta and bold, the line blue, the fingerprints blue, the rows as in `opalro status`, the times in brackets dim.

### `opalro goto`

While it writes, a terminal's stderr shows the turning mark `Writing files` (§8).

| State | Outcome line | Details | Next step |
|---|---|---|---|
| On a line | `Switched to line <line> at save <id> "<message>".` | what was written: `2 files written, 1 removed.` (`No file needed changing.` when nothing was); `Your unsaved changes in 2 files came along.` when some did | `Changed your mind? opalro undo` |
| At a save | `Switched to save <id> "<message>". You are not on a line.` | the same, then `Saving here asks for a new line's name.`, and `To keep saving on <line>: opalro goto <line>` when the save is a line's newest | the same |
| Already on that line | `Already on line <line>, at save <id> "<message>".` | | |
| Already at that save | `Already at save <id> "<message>".` | | |
| Unsaved work in the way | stdout gets nothing; `error: Your unsaved changes, or files Opalro ignores, are in the way, so nothing was changed:` with the paths under it, the first 5 and how many more | `hint:` save them, shelve them (`opalro shelf`) or discard them, then run `opalro goto` again | |
| Files open in another program (Windows) | the rest is written; `error:` names the files, exit code 1; the switch stays unfinished and the next command that changes the project finishes it | | |

- The save is named by its change ID, or by the first 12 hex digits of its fingerprint when it can't be read. Counts are of what the switch wrote and removed; it doesn't say how many were added or changed, nor list the unsaved changes that came along.
- A switch a crash stopped is a `warning:` (`switch_unfinished`) until a command finishes it; there is no `--continue`.
- **Already there:** a goto to where the project already is changes nothing and records no step, so it says so plainly, names the save as `Switched to` would, and offers no `opalro undo`, which would take back the step before it. The engine says whether the goto recorded a step of its own. A goto that first finishes a stopped command and then finds the project already there says the same: the step it finished is the stopped command's, not its own. A goto from a save to the line whose newest it is changes only the position: that is a step, and says `Switched to`.
- **Colour:** `Switched` green and bold, the ID magenta and bold, the line blue, the `opalro goto` of the last detail cyan.

### `opalro discard`

While it writes, a terminal's stderr shows the turning mark `Writing files` (§8). With no paths it throws away every unsaved change; there is no `--all`.

| State | Outcome line | Details | Next step |
|---|---|---|---|
| One file | `Threw away your unsaved changes to <path>.` | `A safety copy of it was kept.` | `Changed your mind? opalro undo` |
| Several | `Threw away your unsaved changes to 4 files.` (`to 1 folder` when only folders changed) | `A safety copy of each was kept.`, or `Safety copies of the 3 that held unsaved work were kept.` (`A safety copy of the 1 that held unsaved work was kept.` when one did; a deleted file put back needs none) | the same |
| Nothing | `Nothing to discard: your files match your last save.` (`Nothing to discard there: it matches your last save.` with paths) | | |
| Nothing could be | `Nothing was discarded: …` (the files changed while Opalro worked, or only unsaved links were left), and a `warning:` says why | | |

- **The safety-copy line** comes when Opalro kept a copy of something thrown away: every changed or new file gets one. A deleted file put back loses nothing and gets none, so a discard that only puts deleted files back has no details.
- **Colour:** `Threw away` green and bold, the path blue.

### `opalro line`

```text
Lines in Skyrunner:

▸ feature/double-jump
  main

On the server (origin):
  main@origin
```

- **The outcome line:** `Lines in <project>:`; at a save, `Lines in <project>. You are at save <12 hex digits>, not on a line.`, and no row is marked. `<project>` is the name of the project's folder.
- **A row** is the line's name, the place marker on the one you are on; the others start with two spaces. Each server's lines follow after an empty line, under `On the server (<server>):`, named `<line>@<server>` as `opalro goto` takes them. Rows show only names: a line's newest save, its time and message would mean reading every line.
- **Colour:** the marker bold, line names blue, the server's name in brackets dim.

### `opalro undo` and `opalro redo`

While they write, a terminal's stderr shows the turning mark `Writing files` (§8).

| State | Outcome line | Details | Next step |
|---|---|---|---|
| One step | `Undid <step>: <what changed>.` (`Redid …`; `undo --to` a step neither reaches: `Went back to entry <entry> (<command>): …`) | | |
| Several steps | `Undid 3 steps:` | each step, without its verb, a row | |
| Fewer than asked for | `Undid 2 steps (all there were):` | the same | |
| `--shelve` set work aside | `Shelved your unsaved changes in the way on shelf <n>.` first, then the step's line | | `Next: bring your changes back with: opalro unshelf` |
| `--shelve` set work aside more than once (`undo N --shelve`, each step its own shelf) | a `Shelved …` line a shelf, then the steps | | `Next: see the shelves with: opalro shelf --list` |
| Stopped part-way | `Undid 1 of the 3 steps asked for:` (`Undid 1 step on the way to entry <entry> before one stopped:` for `--to`), then the rows, then the `error:` | | |
| Stopped by unsaved work | stdout gets nothing; `error:` names the files | `hint:` names only `--shelve` (saving, shelving or discarding first would be the next step undo takes) | |
| Finished a stopped switch | `Finished the <command> that was stopped, so nothing more was undone. Running opalro undo again takes one more step.` | | |
| Nothing to take | `Nothing to undo.` (`Nothing to redo.`; `Nothing changed: things are already as they were right after entry <entry>.` for `--to`) | | |
| `undo --to` with `head` lost | `Made entry <entry> (<command>) the current one: on <line> at <id> "<message>". No file was changed: …` | | |

- A step is named as the diary has it: its command, the save it made with its message, then what changed (`main is back at <id>`, `2 files back`, `shelf 1 is gone`; `its changes show as unsaved` for a save undone, which changes no file).
- `undo` and `redo` end with no "Changed your mind?" line: each is the other's way back.
- **Colour:** `Undid`, `Redid`, `Went`, `Shelved`, `Finished` and `Made` green and bold.

### `opalro undo --list`

```text
Undo diary for Skyrunner, newest first. Steps you can redo come first.

  redo 2  3f9a2c7e01b4  3 min ago   discard: 1 file changed
  redo 1  81c0de4a77f2  4 min ago   goto: on main at ptvz, 48 files changed
▸ you are here
  undo 1  c42a9b7e5510  2 h ago     save: main moved to ptvz "Add double jump"
  undo 2  7d0e34fa19c8  3 days ago  save: main moved to kxqmz "First playable"
          0a1b2c3d4e5f  3 days ago  start: the diary's first entry, which can't be undone
  and more: opalro undo --list --all shows the whole diary
```

- **The heading:** `Undo diary for <project>, newest first.`, then `Steps you can redo come first.` when there are any.
- **A row:** what to type to take the step (`redo N`, `undo N`; nothing for the diary's first entry, which can't be undone), the first 12 hex digits of its entry (`opalro undo --to` takes 8 or more), when it ran, and what it did. Every step redo can bring back is listed, then at most 20 undo can take, then `and more: …` when there are more. The place marker's row stands between them. On a terminal what a step did is cut at its end to fit.
- **`--all`** lists every entry, newest first, as it reads them: the entry, when, what it did, with no heading, numbers or marker. **With `head` lost** the list names the entries where the diary may have stopped, then `Run opalro undo --to <entry> …`.
- **Colour:** the marker bold, entries blue, times dim, `opalro undo --list --all` cyan.

### `opalro shelf` and `opalro unshelf`

While they write, a terminal's stderr shows the turning mark `Writing files` (§8).

| State | Outcome line | Details | Next step |
|---|---|---|---|
| Shelved | `Shelved your unsaved changes in 3 files on shelf <n>.` (`Shelved your unsaved changes on shelf <n>.` for a shelf of no files, only folders) | | `Next: bring them back with: opalro unshelf` |
| Nothing to shelve | `Nothing to shelve: your files match your last save.` (`Nothing was shelved: …` when files changed while Opalro read them, or only unsaved links were left) | | |
| Unshelved | `Unshelved shelf <n> "<message>": 2 files written.` | `Your other unsaved changes in 1 file stayed as they are.` when some did | `Changed your mind? opalro undo` |
| Finished a stopped switch | `Finished the <command> that was stopped, so no shelf was brought back this time.` and what running it again does | | |
| Stopped where the folder differs | stdout gets nothing; `error:` names the paths, `hint:` its way out | | |

- A new shelf takes the smallest free number and is the newest; numbers never shift. `opalro unshelf` brings back the newest, `opalro unshelf <n>` that one. Messages: yours, or `Shelved`, `Set aside by undo`, `Set aside by redo`.

```text
2 shelves, newest first:

    2  "Half-done wall run"    3 files  2 h ago    on main
    1  "Enemy aim experiment"  1 file   1 day ago  at a save
```

- **`opalro shelf --list`:** the number `opalro unshelf` takes, right-aligned; the message's first line in quotes; how many files (`only folders` for a shelf of none); when; `on <line>` or `at a save`. On a terminal the message is cut at its end to fit. No shelves: `No shelves: opalro shelf sets your unsaved changes aside on one.`
- **Colour:** `Shelved` and `Unshelved` green and bold; the list's last three columns dim.

### `opalro tidy`

While it works, a terminal's stderr shows the turning mark `Tidying`, and above it `Tidying: about <size> to free` once that is known (§8).

| State | Outcome line | Details | Next step |
|---|---|---|---|
| Freed | `Freed 1.2 GiB  (3 packs removed, 1 written)` | `Left 2 packs changed in the last 24 hours as they are: a later opalro tidy frees what they hold that nothing needs.` when it did | |
| Nothing to free (it found nothing to free, and freed nothing, whatever small packs it rewrote whole) | `Nothing to free: storage holds 4.8 GiB.` | the same | |
| Something to free, but no pack could be removed | `Nothing could be freed: storage still holds 4.8 GiB.`, then a `warning:` a pack saying why it stayed | the same | |
| A pack it can't read | stdout gets nothing; stderr has the error and its hint, below; nothing was removed | | |
| A pack holding a conflict record | stdout gets nothing; stderr has the error and its hint, below; nothing was removed | | |

A pack it can't read, on stderr (`can't be read (<why>)` in place of `is damaged (<why>)` when it couldn't be read; `<pack>` is the pack's whole path, as the system writes it):

```text
error: Opalro can't tidy this project: pack <pack> is damaged (the file does not start with OPALPACK).

hint: Nothing was removed. Check the pack's permissions. If it is damaged for good, moving it out of .opalro/store/packs lets `opalro tidy` run, losing what only it holds, and the chunks only its chunk lists name.
```

A pack holding a conflict record, which this version doesn't follow, on stderr:

```text
error: Opalro can't tidy this project: pack <pack> holds a conflict record, which this version of Opalro doesn't follow.

hint: Nothing was removed, and the project works as before. A version of Opalro that follows them can tidy it.
```

- What it freed is how much smaller storage got, measured before and after. Its figure beforehand is an estimate, so a `tidy` that found something to free, removed a pack and wrote one can still leave storage no smaller: then the outcome line reads `Freed 0 B  (1 pack removed, 1 written)`.
- It keeps what lines, shelves, the diary and every step in the undo window need, and leaves packs changed in the last 24 hours as they are. It isn't a step, so undo doesn't take it back.
- **Colour:** `Freed` green and bold, the bracket dim.
