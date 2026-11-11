# Milestone 6 attack surface: presets and polish (red-team sessions RT10 and RT11, decision 024)

Milestone 6 makes Bolt ready for v0.1. This file covers the half called "presets and polish". The other half, speed and space, has its own sessions (RT12 and RT13) and its own file.

- **Presets.** `bolt start` asks what kind of project this is (Code, Game, Custom) and suggests one from what it sees in the folder. A preset is written out in full: rules in `bolt.toml` and a marked block of lines in `.boltignore`. Custom writes nothing. `bolt setup` shows the rules, and `bolt setup --preset` switches preset, as a step `bolt undo` takes back.
- **`bolt.toml`.** A file in the project's top folder, read by every command that needs the rules: where files are cut into chunks (`chunk-files-over`), when a save asks first (`ask-before-saving-over`, `ask-before-saving-files-over`), and rules per file pattern. A file or a key Bolt can't use never stops a command: Bolt warns and uses its own values.
- **`bolt verify`.** Checks every pack and every object in it, then that the project has everything it needs. It takes no lock and changes nothing Bolt stored.
- **The terminal.** Colour, symbols with ASCII fallbacks, the terminal's width, paths cut from the middle, messages cut at the end, sentences wrapped, progress bars and a turning mark. Text from a project is escaped as before, colour or not.
- **Git's words.** `bolt commit`, `bolt stash` and eight more words run the Bolt command they stand for, in the forms that mean the same; others are explained. A first-time hint is remembered in a `git-words` file in the user's settings folder.
- **`--json`** was read through once and frozen: names and shapes can't change after v0.1.
- **Follow-ups:** `tidy` records the undo window it used in `.bolt/state/tidied`; a diary end it can't read back no longer stops it; picking up a lost `head` drops a stopped switch's journal; a `safe-folders` entry is judged by one look at the folder it holds; hints and Git-settings details.
- **Packaging.** A workflow started by hand builds four archives, tests them, and makes a draft release.

Threat list: `docs/security.md` §4. Decisions 113 to 125 in `docs/decisions/` give the rules this code follows. `docs/terminal.md` says what every command prints for people, `docs/json.md` what it prints for programs, `docs/storage-format.md` §10 to §12 the files on disk (`state/tidied`, the `setup` step, `bolt.toml`, `.boltignore`), and `docs/guide.md` what users are told.

Your goals:
- **Data:** lose or corrupt data: unsaved work, saved history, undo history, files outside Bolt.
- **Wrong rules:** make a project that arrives with rule files (copied, unpacked, imported from Git) change what Bolt does beyond what those rules may: where it writes, what it reads, what it runs, how much memory, disk or time it uses.
- **Escapes:** read, write, remove or follow a link outside the project, or inside `.git` or `.bolt`, through a rule file, a preset, `setup`, `verify` or `tidy`.
- **Trust:** make Bolt work in a folder it should refuse (`safe-folders`), or make a release that nobody with the right to do so asked for.
- **`verify`:** make it say a damaged or incomplete project is fine, say a good one is damaged, write anything, or hang.
- **`tidy`:** make it free something still needed, through `state/tidied` or a diary end.
- **Resources:** crash, hang, or use memory, disk or time past the bounds the docs state.
- **The person at the terminal:** forge or recolour a line of output, hide text, move the cursor, make a line that isn't Bolt's read like one of Bolt's (a detail, a row, a hint, a `Next:`), or get a raw control character into `--json`.

## Where the code is

- **Rules** (decision 114): `crates/bolt-vcs-core/src/rules.rs`: `parse`, `read` (it opens the file through the project's top folder handle, without following a link), `parse_size`, the limits (file size, number of rules, pattern length), `RulesRead::warnings`. `Repository::rules()` and `read_rules()` in `repo.rs`. The fuzz target `fuzz/fuzz_targets/parse_rules.rs`. How the cut threshold reaches the store: `Store::set_chunk_from`, `is_cut`, `is_small`, `is_streamed` in `store.rs` (only a file under 1 MiB is ever read whole, whatever the setting).
- **Presets and detection** (decision 113): `crates/bolt-vcs-core/src/presets.rs` and `presets/`: `look` (what it reads of the folder: names in the top folder only, at most `LOOK_AT_MOST`), `suggested`, `bolt_toml`, `boltignore_block`, `edit_boltignore`, `block_lines`.
- **The `setup` step** (decision 115): `crates/bolt-vcs-core/src/setup.rs`: `plan_setup`, `apply_setup` (it looks at both rule files again under the lock), the two kept saves of the step, the refusals when something that isn't a regular file stands at a rule file's name.
- **`verify`** (decision 118): `crates/bolt-vcs-core/src/verify.rs`: `verify`, `check_packs` and `check_pack`, `rebuild` (a chunked file rebuilt against its ID), `look_at` (it reads `head` before it lists the packs), `agreed` and `agreed_needs` (a problem with what the project needs is told only when two looks agree; at most three looks), `told_needs`. It shares `tidy`'s marking: `mark` in `tidy.rs`. `Repository::discover_untouched` and `open_untouched` in `repo.rs` open a project without clearing `.bolt/tmp` or trying the lock.
- **`tidy`** (decisions 123 and 125): `tidy.rs`: `window_start`, `record_window` (`state/tidied`: digits and a line feed, read with a cap, a time later than now counts as absent, a link is refused), `step_entry` (a walk from a diary end carries on past an undo step the store lacks), the `diary_end_unreadable` warning.
- **Picking up a lost `head`** (decisions 123 and 125): `undo.rs` `pick_up`: it reads the entry's tag list and server lines first, removes a stopped switch's journal, and tells `switch_dropped` at once.
- **`safe-folders`** (decision 125): `repo.rs` `owner_allows` and `same_identity`; `safe_fs.rs` `folder_id`; the platform crate's no-follow open and identity (`crates/bolt-vcs-platform/src/dir.rs`, `dir/unix.rs`, `dir/windows.rs`).
- **The command line:** `crates/bolt-vcs/src/main.rs` (every command; `start`, `setup`, `verify_command`), `setup_output.rs`, `output.rs` (change rows, `Columns`, the error document, `verified`), `history_output.rs` (it holds at most `HELD_ROWS` rows or `HELD_BYTES` of text to size its columns), `show_output.rs`, `goto_output.rs`, `undo_output.rs`, `shelf_output.rs`, `import_output.rs`, `help.rs`.
- **The terminal** (decision 120): `crates/bolt-vcs/src/style.rs`: `Screen` (colour, symbols, width), `paint`, `fit_path` (cuts a path at `/`), `cut_end` (never inside an escape Bolt wrote), `wrap`, `indented`, `prefixed`, `Screen::detail`, `Screen::command`, and the marks that keep a piece of text whole while wrapping: U+0001, U+0002 and U+0003, taken out before anything is printed. `progress.rs`: `Bar`, `Working`. Two test hooks exist only when Bolt is built with the `fault-injection` feature: `BOLT_FAKE_TERMINAL=<width>` and `BOLT_PROGRESS_QUIET_MS=<ms>`.
- **Git's words** (decisions 116 and 124): `crates/bolt-vcs/src/git_words.rs`: `translate`, `commit_args` (`save <options> -- <message> <paths>`; `SAVE_VALUE_OPTIONS`), `stash`, the explanations, `first_time` and `remember` (the `git-words` file: read with a cap, lines that are no shortcut skipped).
- **`--json`** (decision 117): every `*_json` function in the files above; `output.rs` `error_json` (messages and hints get the screen's escaping pass); `crates/bolt-vcs/tests/json_formats.rs` checks every document against the tables in `docs/json.md`.
- **Packaging** (decision 119): `.github/workflows/release.yml`, `about.toml`, `about.hbs`, `docs/releases/v0.1.0.md`.
- **Tests that already pin the basics** (go past them): `crates/bolt-vcs/tests/corpus_rules.rs` (hostile rule files), `terminal.rs` (escaping, cutting and marks with colour on), `setup.rs`, `verify.rs`, `git_words.rs`, `guide.rs` (every command in the guide runs), and the engine's `rules`, `presets`, `setup`, `verify`, `tidy` and `repo` test modules.

## Places worth attacking

- **A project that arrives with rule files** (T-rows for `bolt.toml` and `.boltignore` in `docs/security.md`). The project may come from a zip, a copy, a `git clone` followed by `bolt import`, or a `.bolt` someone else made.
  - `bolt.toml` as a link (to a file outside, to a device, to itself), a folder, a pipe, a file of 1 MiB + 1 bytes, of 2 GiB, a sparse file;
  - deep nesting, huge arrays, keys repeated a million times, a million rules, patterns that take long to match (`**/**/**…`, long alternations), a pattern list of long strings;
  - numbers at the edges: sizes of 0, of 1 MiB − 1, of 128 GiB + 1, past 2^63, with every unit and with none, negative, fractional;
  - a `chunk-files-over` that makes `save`, `goto` or `import` hold a whole large file in memory, or cut one file two ways in one project;
  - `ask-before-saving-over` set so the question never comes, or always comes, or overflows the sum;
  - a `preset` label of escape codes, of 1 MiB, with a newline;
  - a `Bolt.toml` or `BOLT.TOML` beside or in place of `bolt.toml`, on drives that ignore case and on drives that don't;
  - a rule file that changes between the read and its use, or between `setup`'s plan and its apply;
  - rule files inside the imported Git history versus in the folder: only the folder's may count.
- **`.boltignore` and the preset's block.** Ten thousand start marks; an end mark with no start; marks with trailing spaces, CRLF, a BOM, bytes that aren't UTF-8; a block of 100 MiB; the file as a link or a folder. `setup --preset` must touch only the first block and keep every other byte; `undo` must put every byte back.
- **Preset detection fooled by planted files.** Files and folders named like every engine's at once; links and junctions with those names pointing outside or at huge folders; a folder of a million names; names that differ only by case or by Unicode form. The planted files may change the suggestion and nothing else: `--preset code` writes Code's rules whatever is planted. Find a way for detection to read a file's contents, follow a link, or take unbounded time.
- **The `setup` step.** Stop `bolt setup --preset` and `bolt start --preset` at every fault point, then run any command: nothing lost, both rule files either as before or as after, `undo` and `redo` exact. Race it against `save`, `goto`, another `setup`. Put a link, a folder or a read-only file at a rule file's name between the plan and the apply. Make it write through a link. Make `undo` of a setup restore the wrong bytes or the wrong mode.
- **`bolt verify` on hostile packs.**
  - packs with indexes that lie (wrong counts, offsets past the end, entries out of order, two entries for one ID), records that match their fingerprint but don't decode, chunk lists that build another file, a list of millions of one-byte chunks, a pack that is a pipe, a link, a 100 GiB sparse file;
  - make it report no problems for a project missing an object some save needs, or whose `head` names an entry no pack holds;
  - make it report damage that isn't there, by running `save`, `tidy`, `goto` and merges beside it in a loop (it takes no lock: the two-looks rule is the defence);
  - make it write anything under `.bolt` (it may only make `.bolt/tmp` and `.bolt/cache` when they are missing), take the lock, or follow a link out;
  - its memory: it holds the IDs it reads; find input that makes it hold far more than that;
  - `--json`: at most 1,000 problems kept and the true count; names escaped.
- **`tidy` and `state/tidied`.** Plant `state/tidied` with every kind of content (a future time, a past time, a huge number, a link, a folder) and make `tidy` free undo data newer than the time it names, or anything the current state needs. Plant a diary end whose parent, or whose undo step's target, is missing, and make `tidy` free what a readable part of that diary still needs, or loop.
- **Picking up a lost `head`.** After a switch stopped halfway, lose `head` and pick up with `undo --to`: anything the stopped switch wrote must show as unsaved changes, what it would have overwritten must be in its kept save, and no file may be written by the pick-up. Stop the pick-up at every fault point.
- **`safe-folders`.** Get Bolt to work in a folder owned by someone else: an entry that is a symbolic link, a junction, a mounted volume; an entry with `.`, `..`, a trailing separator, another letter case, a `\\?\` or UNC form; a link above the entry (followed, by design: is that enough to cheat?); a folder swapped for a link while Bolt looks; two folders with one identity (hard-linked folders don't exist, but bind mounts and drives that give no file numbers do).
- **The terminal, with colour on and off, at widths from 1 to 500.**
  - escape sequences, bidi controls, zero-width characters, newlines and carriage returns in every text a project controls: file names (Unix), save messages, line names, shelf messages, author and saver names and emails, tag and branch names from Git, `bolt.toml` keys and values, pack file names, paths in errors and warnings;
  - text shaped like Bolt's own output: a save message that looks like a change row, a line name that looks like the place marker, a path with spaces crafted so that wrapping starts a line with it, a message ending in `  [main]`;
  - cutting: make `fit_path` or `cut_end` cut through an escape or a multi-byte character, leave colour switched on, or panic at tiny widths;
  - the wrapping marks U+0001 to U+0003: get one into a project's text unescaped, or get one printed;
  - wide and combining characters: rows are counted one column per character, a known limit; find more than a ragged row;
  - progress: make a bar or the turning mark draw over a message, a question or `--json`, or appear when stderr isn't a terminal.
- **`--json`.** A raw control character anywhere in any document; a document that doesn't parse; a field `docs/json.md` doesn't list; colour codes; an error whose `message` isn't escaped; numbers past 2^53 where the doc says none can be.
- **Git's words.** Make a shortcut do more than its Git form asks: `commit` forms that amend or save paths the user didn't mean, a message read as an option, `stash` forms that shelve under a name, `branch` forms that change a line, a word hidden after `--json` or other options. The `git-words` file as a link, a folder, a pipe, 1 GiB, or holding escape codes.
- **Help.** A page that names a flag or command that doesn't exist; `bolt help <something hostile>`.
- **The release workflow.** Read `.github/workflows/release.yml` as an attacker with a pull request, and as one with write access to a branch that isn't `main`: can either make a release, change what an archive holds, run code with a token that can write, or get a value they control into a script? Is every action pinned, the one download checked before use, the smoke test run on the very file that is packed? You can't run workflows from the sandbox: this part is a review, and a finding is a line of the workflow with the event that reaches it.
- **Crashes.** Stop every new or changed command (`start`, `setup`, `verify`, `tidy`, `undo --to` with `head` lost) at every fault point (`BOLT_FAULT_FAIL` and the fault-point list in `fsio.rs`), then run any command. Nothing may be lost, and the next changing command must finish or refuse cleanly.
- **Windows.** Windows-only attacks (junctions and mounted volumes at rule files and `safe-folders` entries, reserved names in presets' patterns, console colour, files in use during `setup`, error 303) become tests on a `redteam/m6a-*` branch, run by CI and deleted afterwards.

## Limits already known (not findings unless you show more)

`docs/security.md` lists the known limits, and `docs/plans/2026-11-01-milestone-1-follow-ups.md` the deferred items. Among them:
- **Wide characters** count as one column, so a row of them can pass the terminal's width.
- **Lines that aren't lists or tables** (an outcome line with a long title) are never cut, so they can pass the width.
- **A save message that ends like a line marker** (`  [main]`) can pass for one in `bolt history` when colour is off; `--json` can't be mistaken.
- **`verify`'s memory** grows with the number of objects, like `tidy`'s.
- **A clock that ran ahead during a `tidy`** and was then set back makes `tidy` stop, and `verify` report missing undo data, until the clock passes the recorded time, if the undo window was also raised. Nothing is lost.
- **A link planted at `.bolt/state/tidied`** stops `tidy` until it is removed by hand.
- **`safe-folders` on a drive that gives no file numbers** can't vouch for a folder (decision 125).
- **Errors have no code in `--json`** in v0.1.
- **The archives are not signed**, and `SHA256SUMS` says only that a download is the file the workflow uploaded.
- **The cap on rules** bounds how many are kept, not the work of reading a file full of rules that are passed over (the file's size cap bounds that).
