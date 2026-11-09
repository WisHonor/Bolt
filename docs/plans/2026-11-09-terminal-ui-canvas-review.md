# Terminal-UI canvas review

The owner's [terminal-UI design canvas](https://claude.ai/artifact/7EsY5fiSavfbM1NWcnYW3h), reviewed on 2026-11-08 screen by screen against Bolt's decisions, [json.md](../json.md) and the code of that day. The correction prompt at the end also takes in decisions [089](../decisions/089-git-lfs-objects-kept-elsewhere.md) and [090](../decisions/090-the-first-save-looks-at-the-whole-folder-for-pointers.md), made the next day.

## Verdict

Not yet. The visual system is good and nearly complete, but too many facts are wrong to build from. The prompt that made the canvas described the screens without giving Bolt's actual rules, so the design invented details wherever the prompt was silent.

## What is good

Every state asked for has dark, light and plain versions, with stdout and stderr marked and complete ASCII fallbacks. The error-then-hint shape is consistent, every question has a no-terminal stop that names its flag, untrusted text is escaped clearly, long paths are cut from the middle, and the undo list and verify screens are strong.

## What is wrong, most serious first

1. **Import works the wrong way.** It shows `bolt import <repo>` with a source folder. Bolt's `import` takes no argument and works in place, next to `.git/`. The design also refuses uncommitted Git work (Bolt carries it into the next save), and says a stopped import "leaves nothing behind" (Bolt keeps the started `.bolt`, so a rerun can finish).
2. **Recovery hints that would fail:**
   - `bolt goto --continue` doesn't exist; the next changing command finishes a stopped switch.
   - Files open in another program: Bolt tries 10 times during the write, writes the rest, then fails with exit 1. The design says it checks everything first and changes nothing.
   - Undo stopped by unsaved work: "save first" doesn't help (undo would take that save back). `--shelve` is the way.
   - Shelves: a new shelf takes the smallest free number, and numbers never shift. So the suggested `unshelf 2` would bring back the wrong shelf.
3. **Decided numbers contradicted:**
   - The large-save question: the design says 1 GB / 5,000 files; Bolt uses 100 MB / 10,000 and names three folders.
   - The undo list: the design shows 5 steps; Bolt shows up to 20, then "and more".
   - `undo --to`: the design accepts 4 hex digits; Bolt needs at least 8.
   - Diff limits: the design says 4 MB; Bolt's rules are binary content, over 64 MiB, or over 1,000,000 lines.
   - Exit codes: Bolt uses 0, 1 and 2 only.
4. **Behaviour reversed:**
   - A bare `bolt discard` is refused behind an invented `--all`; in Bolt it discards everything, undoably.
   - `tidy` claims every undo step stays complete; it removes what only steps older than 30 days need.
5. **About 25 invented flags**, some contradicting decisions:
   - `--allow-lfs-pointers` and `--skip-missing-lfs` (decision 085 has no such flags);
   - `--wait`, `bolt unlock`, `--check`, `--full`, `--repair` fetching from a server;
   - `-m`, `line new`, `--git-words`.
6. **Things from later versions:** "locked by…", "merged line by line", `line new`, and network repair are v0.2 and v0.3 features.
7. **Smaller wording mismatches:**
   - Default shelf messages: Bolt uses "Shelved" and "Set aside by undo".
   - Status shows a "renamed" kind that doesn't exist.
   - Show calls a shortened fingerprint "full" (the full one is 64 hex digits).
   - Git-word hints should appear once, the first time only.

## What is missing

- **Bolt's real warnings**, about 30 kinds in `json.md`. None appear; four invented ones do.
- **Several states:**
  - not inside a project; a lost `head` and its recovery; a command finishing a stopped switch first;
  - ambiguous `goto` targets; amend refusals; a second import refused;
  - the other Git LFS stops; `unshelf` conflicts; `status --full`.
- **Widths:** 60 and 120 columns are shown only for `status`.
- **"What changed from today"** is mostly wrong: it calls existing screens new.

## Two terminal notes

The design should take these on. Windows has no colour in Bolt yet, so the plain version is the Windows default for now. And legacy Windows consoles lack ✓ ✗ ▸ in their fonts, so default to ASCII unless the terminal is known to handle them.

## The correction prompt

To paste into the same Claude Design canvas:

```text
Thanks, the visual system is strong; keep its layout, colour roles, fallbacks and the dark/light/plain/stdout-stderr structure. But many facts are wrong because I didn't give you Bolt's rules. Please revise every artboard to match these facts exactly, remove anything invented, and add the missing states. When you want to change one of these behaviours, don't change it in the mockup: list it on OpenQuestions with your reason.

FACTS
1. Import: `bolt import` takes NO argument. You run it inside a Git repository's folder; it makes `.bolt/` next to `.git/`, in place, and never changes the Git repository. A second import is refused. Uncommitted Git work is not refused; it comes along into the next save. An import that stops keeps the started `.bolt` folder, so running it again finishes from what was stored.
   Summary shows: commits imported; lines (from branches) and server lines (`feature@origin`); tags; names renamed only when Bolt's rules refuse them (bad characters become `-`, clashes get `-2`); skipped entries, which are only `.git`/`.bolt` spellings (submodules become project links, symlinks are kept); counts of commits whose trees aren't in Git's standard form, of Git LFS pointers not in Git LFS's exact form, and of commits holding Git LFS files; and the Git LFS files brought in (count, size).
   Git LFS stops (no flags, no partial import):
   - objects missing: stops before `.bolt` is made; lists the first 20 with path, commit and size, then how many in all and their total size; hint: `git lfs fetch --all`, then `bolt import` again;
   - Git LFS files kept elsewhere: `lfs.storage` set to another place (in the repository's or the user's Git settings), or `.git/lfs` a link; it stops only when the import has a Git LFS file to bring in, and says where the setting is or which folder is the link;
   - a pointer with `ext-` lines: names it; hint: `git lfs migrate export` in a copy of the repository, then `bolt import` in that copy;
   - an object whose SHA-256 differs: names the file, commit and object; hint: `git lfs fsck <commit>`, which moves the bad copy aside, then `git lfs fetch --all`, then `bolt import` again.
   A file that only looks like a pointer comes in as it is, with the warning `lfs_lookalikes`.
2. Unfinished switch (after a crash): no `--continue`, never rolled back. `bolt status` says a switch is unfinished, between which entries; any command that changes the project finishes it first and says so (`switch_finished`).
3. Files open in another program (Windows): Bolt tries each 10 times, 100 ms apart, while writing, writes everything else, then fails with exit 1 naming the files (`files_in_use`). The switch stays unfinished; the next command finishes it once they're closed. Bolt can't name the program yet.
4. Undo/redo stopped by unsaved work: the only way through is `--shelve`. Don't suggest saving, shelving or discarding first: undo would take that step back next.
5. Shelves: a new shelf takes the smallest free number from 1; numbers never shift. `bolt unshelf` brings back the NEWEST; `bolt unshelf N` a named one. Default message: `Shelved`. A shelf made by `--shelve`: `Set aside by undo` or `Set aside by redo`.
6. `bolt discard` with no paths throws away every unsaved change. It keeps a safety copy and can be undone. There is no `--all`.
7. `bolt tidy`: keeps what lines, shelves, the diary and every step in the undo window (30 days, setting `undo-window-days`) need, and removes what only older steps needed. It leaves packs changed in the last 24 hours untouched and says so. "Tidying: about X to free" goes to stderr, only in a terminal; then "Freed X (N packs removed, M written)". No `--check`.
8. The large-save question: asked when a save adds more than 100 MB or 10,000 new files; names the three top-level folders with the most new bytes; prompt `Save them? [y/N]` on stderr; without a terminal it stops and names `--yes`.
9. `bolt undo --list`: the steps redo can bring back first, then the undo steps, at most 20, then "and more". Entry IDs are 12 hex digits. `--all` shows the whole diary, undo and redo entries and cleared steps included. `bolt undo --to` needs at least 8 hex digits. The diary's first entry (a `start` or an `import`, never both) can't be undone.
10. `bolt changes`: standard unified diff, `@@ -a,b +c,d @@` hunk headers, 3 lines of context (propose a new header on OpenQuestions if you want one).
    - Shown without lines: binary (a zero byte in the first 8,000 bytes), over 64 MiB, or over 1,000,000 lines on either side. Big chunked text files still show lines.
    - A line-ending-only change: a note in the file header, then the hunks with `(CRLF)` after lines that end in CRLF.
    - A file whose diff takes over 2 seconds: coarser hunks plus the warning `diff_timed_out`.
    - `--save <save>` shows what a save changed.
11. Exit codes: 0 done (warnings allowed), 1 failed, 2 the command line was wrong. Nothing else.
12. Not in v0.1, so remove them: locking ("locked by", `unlock`, lock waits), combining/merging, line subcommands (`line new`; a line is created by saving away from a line with `--line <name>`), anything over the network (`verify` can't fetch), `-m`, `--wait`, `--full` on changes, `--repair`, `--check`, `--allow-lfs-pointers`, `--skip-missing-lfs`, `--git-words`, `status --endings`, `--write-under`. Real flags: save `--amend` `--line` `--yes` `--accept-crlf`; status `--full`; changes `--save`; undo/redo `[N]` `--shelve`; undo `--list` `--all` `--to`; shelf `--list`; unshelf `[N]`; `--json` everywhere.
13. Waiting for the lock: one changing command at a time; the next one says which command holds it (and since when) and waits until it ends. No timeout, no flag.
14. Git words: accepted as hidden shortcuts. The FIRST time a Git word is used, Bolt names its own word, once. Git words with no Bolt equivalent (e.g. `add`) get an explanation instead.
15. Status/show change kinds: added, removed, modified, kind changed (file ↔ link ↔ folder). There is no "renamed". `show` gives author AND saver; a full fingerprint is 64 hex digits; a save's "line" is the line it was made on (none for imported saves).
16. The first save after an import stops at: files whose only difference is line endings Git converted (`--accept-crlf` saves them as they are; or have Git rewrite them); and Git LFS pointer files where the import brought in the real file, looked for in the whole folder even when the save names paths (hint: `git lfs checkout`, or, if Git LFS can't, `bolt discard` with those paths; NO flag). The pointer stop comes first.
17. Sizes: use 1024-based units (KiB, MiB, GiB), which is how Bolt states its limits. Times: relative ("2 h ago") up to 30 days, then the date.
18. Terminal: Bolt has no colour on Windows yet (it needs a console call the owner must review), so plain is the Windows default for now. Default to ASCII symbols unless the terminal is known to render ✓ ✗ ▸ (Windows Terminal, UTF-8 macOS/Linux). "Dim" is SGR 2, not bright black; never put essential information only in dim text. Don't make status or save compute line counts per file (it slows status, which must stay under 2 s for 100,000 files).

ADD
- The real warnings, each with a mockup (one line on stderr, wording in Bolt's voice): placeholders_written, not_written_here, files_in_use, changed_since_plan, links_left, switch_finished, switch_unfinished, kept_as_they_were, ignored_changes_saved, gitignore_skipped, git_names_left_out, nested_projects_left_out, diff_timed_out, head_not_flushed, damaged_pack, copy_unreadable, pack_unreadable, pack_not_merged, pack_not_tidied, file_cache_not_stored, cache_key_unusable, lfs_lookalikes, grafts_ignored, duplicate_change_id, config_includes_ignored, nested_bolt_folder, elevated_only, and the note when a project's own settings try to set your name (ignored_project_identity). Remove the four invented warnings.
- Missing states:
  - not inside a Bolt project;
  - a lost head (every command stops and says the saves are safe; `undo --list` lists the diary's ends; `undo --to` picks one);
  - a command finishing a stopped switch first;
  - undoing a step whose saves tidy removed;
  - `undo N` stopping part-way;
  - an ambiguous goto/show target (a line name that is also the start of a change ID);
  - amend refusals (at a save, not on a line; another line or a tag names the save) and nothing to amend;
  - partial-save refusals (a path matching nothing, outside the project, an ignored new file named outright);
  - a second import refused;
  - the Git LFS stops in item 1;
  - unshelf stopped because the folder differs at a path the shelf changes;
  - unshelf of an unknown shelf;
  - `status --full`.
- 60- and 120-column versions of save, status, history, changes, undo --list and import's summary.
- Redo "what changed from today" from Bolt's real current output. Status, history, show, goto, line, undo --list and help already print text today. Name the real changes ("Amended" → "Updated", the `*` line marker → ▸, decimal → 1024-based sizes…), each with a reason.
- On OpenQuestions, drop questions already settled (the large-save limits, how long safety copies last, the words for v0.2/v0.3 features). Keep only genuine choices.
```
