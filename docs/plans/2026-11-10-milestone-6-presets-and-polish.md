# Milestone 6 ("Presets and polish → v0.1"), presets and polish: Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task by task. Steps use checkbox (`- [ ]`) syntax for tracking. Every subagent runs on Opus.

**Goal:** `bolt start` asks what kind of project this is and writes a preset's rules; `bolt setup` shows and switches them, undoably; `bolt.toml` is read; Git's words work as shortcuts; `bolt verify` checks everything; every command prints in the owner's design, with colour on Windows too; the follow-ups due before v0.1 are done or moved; and v0.1 is packaged up to, but not including, publishing.

**Architecture:** Presets are data plus two pure functions that give the text of `bolt.toml` and of a block of `.boltignore`; one new diary step, `setup`, writes those two files through the write-out that `goto` and `unshelf` already use, so undo, redo and crash recovery come with it. `bolt.toml` is read by one module (`rules.rs`) right after the change lock is taken, and its chunk threshold lives on the store, so the walk and the save loop keep their signatures. `bolt verify` is the existing pack check, made to go on past a bad object and to decode records, plus `tidy`'s own marking run without removing anything. Everything a person sees goes through one small module in the command-line crate (`style.rs`) that knows, per stream, whether it has colour, symbols and a width.

**Tech Stack:** Rust 1.99.1 and the crates already in the workspace (`toml` for `bolt.toml`, `similar` for the preview of a preset switch, `clap`), plus two for the command-line crate only: `anstyle-query` (already in `Cargo.lock` through clap; asks the Windows console to read colour codes) and `terminal_size` (new; the terminal's width). GitHub Actions for the release builds.

**Spec:** decisions [113](../decisions/113-presets-what-each-writes-and-how-start-picks-one.md) to [123](../decisions/123-follow-ups-done-or-moved-past-v0.1.md); [storage-format.md](../storage-format.md) §5, §9, §10, §11 and §12 with the vectors `rules.json` and `presets.json`; [json.md](../json.md); [terminal.md](../terminal.md). The look comes from the owner's design canvas, read through [its review](2026-11-09-terminal-ui-canvas-review.md): the canvas sets layout, colour, symbols, widths and the style of the words; Bolt's decisions, json.md and the code set every fact.

**Rulings made while building:** `2026-11-10-milestone-6-rulings.md`, kept as the build goes, verbatim. Every ruling's premise is checked in the code before it is made.

**The other session.** A speed session builds milestone 6's other half at the same time, in its own worktree, on `milestone-6-speed`. It owns the speed of `status`, `save` and `goto`, the walk behind them, the file cache, `benchmarks.md`, the speed gates, and decisions 099 to 112. This plan never optimizes those paths, and where a task must touch `snapshot.rs`, `snapshot/walk.rs`, `writeout.rs`, `store.rs`, `pack.rs`, `filecache.rs` or `import/convert.rs`, the change is the smallest that works and the task says so in its report. Nobody here checks out `milestone-6-speed` or touches `C:\Users\elmas\Desktop\Bolt-speed`.

## Global Constraints

- Everything in CLAUDE.md applies.
  - `#![forbid(unsafe_code)]` in the engine and the command-line crate. **No task adds `unsafe`.** The number of `unsafe` blocks in `bolt-vcs-platform` must be the same at the end as at the start (Task 20 checks). A task that finds it needs new `unsafe` stops and says so.
  - No `unwrap`, `expect` or `panic` outside tests. One place per rule: `bolt.toml`'s rules only in `rules.rs`; what a preset writes and the `.boltignore` block only in `presets.rs`; the `setup` step only in `setup.rs`; colour, symbols, width, wrapping and sizes only in `style.rs` (and `text.rs` for the size formatter the engine's messages share); Git's words only in `git_words.rs`.
  - Spec before code: storage-format.md §12 and the two vector files are written; the code matches them, and a task that finds the spec wrong stops and reports rather than changing a vector.
- Code comments are short and human: the why in a line or two, no process words, no task or decision numbers in code.
- **Untrusted text is escaped exactly as today** (`text::escape`, per piece, at the edge; a second escape in `Warning::new` and when an error is told). Colour is added around escaped text when it is printed and is never stored in an error's or a warning's message. A new piece of project text on screen (a `preset` label, a `bolt.toml` key, a line of `.boltignore` in a preview) is escaped like the rest.
- **`--json` never carries colour, symbols picked for a terminal, progress, "Next:" lines or questions.** Every question keeps its stop without a terminal, and the stop's hint names the flag.
- **Without a terminal, stderr holds only errors, their hints and warnings,** as before this milestone: no "Next:" line, no Git-word hint, no progress, nothing wrapped or cut. Tests run without a terminal.
- **A project with no `bolt.toml` behaves byte for byte as before:** the same IDs, the same packs, the same question numbers.
- Every read and write inside the project folder goes through `bolt_platform::Dir` (decision 048). The rule files are written only by the `setup` step, through write-out.
- Messages say what happened, why, and what to do next (decision 017). The words of existing errors, warnings and hints stay as they are unless a task here names the change: the design changes how they are laid out, not what they say.
- **Values** (from the decisions):

  | What | Value |
  |---|---|
  | `chunk-files-over` | a size from 1,048,576 to 137,438,953,472 bytes; 1 MiB when absent; a file of that size or more is cut (114) |
  | `ask-before-saving-over` | a size; 100,000,000 bytes (`"100 MB"`) when absent; Game writes `"1 GiB"` (113, 114) |
  | `ask-before-saving-files-over` | a whole number from 0; 10,000 when absent (114) |
  | A size | digits, optional spaces, then `B`, `KB`, `MB`, `GB`, `TB` (×1,000) or `KiB`, `MiB`, `GiB`, `TiB` (×1,024); must fit 64 bits (114) |
  | `preset` | text of at most 64 bytes (114) |
  | `bolt.toml` | TOML 1.1.0; a regular file of at most 1,048,576 bytes, read through the top folder's handle, never through a link (114) |
  | Warnings for `bolt.toml` | `rules_file_skipped` (the whole file), `rules_ignored` (keys; top-level in byte order, then `rule[N]` or `rule[N].key`, N from 1) (114) |
  | The `.boltignore` block | first line starts with `# >>> Bolt preset`, last line is exactly `# <<< Bolt preset`; one CR at a line's end is overlooked (113, storage-format.md §12) |
  | Detection | names in the top folder; sizes through handles, never through a link; stops after 20,000 entries (113) |
  | `bolt verify`'s problems kept | the first 1,000, and the full count; 20 a kind on screen (118) |
  | Exit codes | 0 done, 1 failed, 2 a command line Bolt couldn't use (120) |
  | Wrapping | only on a terminal, at the smaller of its width and 100 columns; under-text indents 7 (`error:`), 9 (`warning:`), 6 (`hint:`, `Next:`) (120) |
  | Progress | only on a terminal; after 400 ms; redrawn at most every 100 ms; label padded to 24, 24 cells, percentage in 4, then done / total; counts go under 70 columns, cells under 50 (120) |
  | Sizes on screen | 1,024-based: `B`, `KiB`, `MiB`, `GiB`, `TiB`; one decimal below 10, none from 10 up (120) |
  | Change-ID letters | half-byte `v` is the letter `z` minus `v` (122) |
  | Version | `0.1.0` (119) |

- **New warning kinds** (each in json.md §4 already): `rules_file_skipped`, `rules_ignored` (Task 4), `diary_end_unreadable` (Task 9), `switch_dropped` (Task 13).
- **One new file in `.bolt/state/`:** `tidied` (Task 9; storage-format.md §10): a number of milliseconds in digits and an LF, read with a cap of 32 bytes, written through `tmp/` and a rename.
- **A size that is a setting** is shown as `bolt.toml` would write it: the largest unit that gives it exactly, the 1,024 family first (`100 MB`, `1 GiB`); every measured size is 1,024-based (terminal.md §9).
- **Dependencies:** `anstyle-query` as a direct dependency of `bolt-vcs` (no new package), and `terminal_size` (one new package), both for the command-line crate only. Any other needs a ruling first.
- **Local runs on this machine** (memory is short; two sessions share it): `export PATH="$HOME/.cargo/bin:/c/Program Files/GitHub CLI:$PATH"`, `export BOLT_CONFIG_DIR="$(mktemp -d)"` before any test, build with `-j 1`, test with `-- --test-threads=1`, and run one test file at a time (`cargo test -j 1 -p bolt-vcs --test cli -- --test-threads=1`). Run `cargo fmt` after every edit. Unix-only code can't be clippy-checked here: watch CI. Test files live in a temporary folder, never in the repository.
- Docs are edited with the Edit tool or `sed`, never Python in text mode; every edited doc is checked with `tr -cd '\r' < file | wc -c`, which must print 0.
- The untracked `.codex/` and `AGENTS.md` are not ours: never staged, edited or deleted.
- Push at task boundaries to `milestone-6`; CI runs on Windows, macOS and Linux; a task is done only once they pass. One commit per task plus fix commits, with no Claude attribution. `main` comes in by merge, never by rebase.

## Review Focus

1. **A hostile `bolt.toml` never stops or slows a command.** A link to a file outside the project, a folder, a 2 MiB file, a file of 100,000 nested brackets, a key given twice, `chunk-files-over = "1 B"`. Expected: `bolt save` and `bolt discard` work with Bolt's own values, one warning says why, and nothing outside the project is read. Tests: Task 4 `a_rules_file_bolt_cannot_use_never_stops_a_save`, Task 15 `corpus_rules.rs`.
2. **Text from a project stays escaped with colour on.** A file name, a save message, a line's name and a `preset` label that each hold `ESC[31m` and U+202E. Expected: with a terminal faked, the bytes Bolt writes hold its own colour codes and the project's text only in escaped form. Test: Task 15 `project_text_is_escaped_with_colour_on`.
3. **A start can be taken back.** A folder with a `.boltignore` of the user's own; `bolt start --preset game`; `bolt undo`. Expected: `bolt.toml` is gone and `.boltignore` is byte for byte what it was. Test: Task 7 `undo_after_a_start_gives_the_folder_back`.
4. **A preset switch keeps what was never saved.** `bolt.toml` edited by hand and not saved; `bolt setup --preset code --yes`; `bolt undo`. Expected: the edited file is back. Test: Task 6 `undo_puts_back_edits_that_were_never_saved`.
5. **`bolt verify` beside a running command reports nothing that isn't there.** A save, a merge of small packs or a `tidy` running at the same time. Expected: no problem reported for a pack that went away. Test: Task 9 `a_pack_that_goes_away_mid_check_is_no_problem`.
6. **A script sees what it saw before.** Every command run without a terminal: stderr empty when nothing went wrong, no escape codes, no symbols outside ASCII in Bolt's own words. Test: Task 1 `without_a_terminal_bolts_own_words_are_plain`, kept true by Tasks 10 to 12.
7. **No `bolt.toml`, no change.** The same folder saved by the build before this milestone and by this one. Expected: the same save ID and the same pack names. Test: Task 4 `a_project_without_a_rules_file_saves_as_before`.
8. **Git's words never do something else.** `bolt stash pop` brings a shelf back, and never makes a shelf called "pop"; `bolt commit -m "x"` saves with the message `x`; `bolt add .` changes nothing. Test: Task 8 `stash_pop_brings_a_shelf_back`, `add_explains_and_changes_nothing`.

---

## File structure

| File | What it is for |
|---|---|
| `crates/bolt-vcs/src/style.rs` (new) | What each stream can show (colour, symbols, width); roles and their codes; symbols and their plain forms; wrapping; cutting a path to fit |
| `crates/bolt-vcs/src/progress.rs` (new) | The progress bar and the turning mark; replaces `output::Progress` |
| `crates/bolt-vcs/src/git_words.rs` (new) | Git's words: the table, the forms turned into Bolt's, the explanations, the once-only hint |
| `crates/bolt-vcs/src/help.rs` (new) | The overview screen and each command's examples |
| `crates/bolt-vcs/src/main.rs`, `output.rs`, `import_output.rs` | The commands `setup` and `verify`, `start --preset`; every command's text in the new layout |
| `crates/bolt-vcs-core/src/rules.rs` (new) | `bolt.toml`: sizes, keys, what is passed over and why |
| `crates/bolt-vcs-core/src/presets.rs` and `src/presets/*` (new) | The presets' text as data; what was found in a folder; the `.boltignore` block |
| `crates/bolt-vcs-core/src/setup.rs` (new) | Working out a preset switch, and writing it as one step |
| `crates/bolt-vcs-core/src/verify.rs` | The full check: packs and objects, then what the project needs |
| `crates/bolt-vcs-core/src/tidy.rs` | Marking shared with `verify`; the two things it now passes over |
| `crates/bolt-vcs-core/src/text.rs` | The one size formatter |
| `crates/bolt-vcs/tests/setup.rs`, `verify.rs`, `git_words.rs`, `terminal.rs`, `corpus_rules.rs` (new) | CLI tests of the new commands, the look with a terminal faked, and hostile rule files |
| `docs/guide.md`, `README.md`, `SECURITY.md` (new) | The user guide and what ships beside the program |
| `.github/workflows/release.yml`, `about.toml`, `about.hbs` (new) | The release builds, their checksums, the notices, the draft release |
| `redteam/attack-surface-m6.md`, `redteam/attacks9`, `redteam/attacks10`, `crates/bolt-vcs/tests/redteam10*.rs`, `redteam11*.rs` (new) | The red team's two sessions |

---

### Task 1: What a terminal can show, and the shared lines

Decision 120; terminal.md §1 to §7 and §9.

**Files:**
- Create: `crates/bolt-vcs/src/style.rs`; `crates/bolt-vcs/tests/terminal.rs`.
- Modify: `crates/bolt-vcs/Cargo.toml` (`anstyle-query`, `terminal_size`); `crates/bolt-vcs/src/main.rs` (find out once, at the start; `bolt changes` takes its colour from it, on Windows too); `crates/bolt-vcs/src/output.rs` (`tell_failure`, `print_warnings`, `format_size`, the `→` in `change_line`); `crates/bolt-vcs-core/src/text.rs` (`bytes` becomes 1,024-based); the tests whose text this changes.

**Interfaces:**
```rust
// style.rs
#[derive(Clone, Copy, Debug)]
pub struct Screen { pub color: bool, pub symbols: bool, pub width: Option<usize> }   // width: None when not a terminal
pub fn out() -> Screen;                       // stdout's, found out once
pub fn err() -> Screen;                       // stderr's
pub fn find_out(json: bool);                  // called first thing in main; also asks the Windows console for colour
#[derive(Clone, Copy)]
pub enum Role { Ok, Error, Warning, Hint, Command, ChangeId, Fingerprint, Line, Path, Added, Removed, Changed, Hunk, Dim, Bold }
#[derive(Clone, Copy)]
pub enum Sym { Bullet, Marker, Arrow, Back, Check, Cross, Rule, Cut, Full, Empty }
impl Screen {
    pub fn paint(self, role: Role, escaped: &str) -> String;       // the text unchanged when color is false
    pub fn sym(self, sym: Sym) -> &'static str;
    /// `prefix` then `message`, wrapped under the message's text when this is a terminal.
    pub fn prefixed(self, prefix_role: Role, prefix: &str, message: &str) -> String;
    /// `path` cut from its middle to fit `room` columns, or whole when it can't be or this is no terminal.
    pub fn fit_path(self, path: &str, room: usize) -> String;
    pub fn cut_end(self, text: &str, room: usize) -> String;
}
pub fn next(line: &str);                      // "Next: …" on stderr, only when stderr is a terminal
pub fn changed_your_mind();                   // "Changed your mind? bolt undo", the same
// text.rs (engine): the one formatter, 1,024-based
pub fn bytes(n: u64) -> String;               // "512 B", "31 KiB", "1.2 GiB"
```
A test hook, compiled only with the `fault-injection` feature: `BOLT_FAKE_TERMINAL=<width>` makes both streams count as terminals of that width (colour and symbols then follow `NO_COLOR`, `TERM` and `BOLT_ASCII` as usual, and on Windows the console is not asked).

- [ ] **Step 1: Write the failing tests** (unit tests in `style.rs`; CLI tests in `tests/terminal.rs`):

```rust
#[test] fn each_role_has_its_code_and_ends_with_a_reset() {}
#[test] fn without_colour_paint_changes_nothing() {}
#[test] fn every_symbol_has_a_plain_form_in_ascii() {}
#[test] fn a_long_error_wraps_under_its_text_at_the_width_or_100() { /* width 60 and width 140 */ }
#[test] fn a_message_with_its_own_lines_keeps_them() {}
#[test] fn a_word_longer_than_the_width_is_left_long() {}
#[test] fn nothing_wraps_without_a_terminal() {}
#[test] fn a_path_loses_folders_from_its_middle_and_keeps_its_first_folder_and_name() { /* Assets/…/Rock_Albedo.psd */ }
#[test] fn a_path_that_cannot_fit_is_left_whole() {}
#[test] fn widths_count_escaped_text() {}
#[test] fn sizes_are_1024_based_with_one_decimal_below_ten() { /* 0 B, 1023 B, 1.0 KiB, 9.9 MiB, 10 MiB, 1.2 GiB */ }
#[test] fn utf8_locales_are_recognised() { /* en_US.UTF-8, C.utf8, POSIX; LC_ALL before LC_CTYPE before LANG */ }
#[test] fn bolt_ascii_wins_either_way() {}
// tests/terminal.rs, with BOLT_FAKE_TERMINAL
#[test] fn an_error_is_red_and_its_hint_cyan_after_an_empty_line() {}
#[test] fn no_color_turns_colour_off_and_keeps_symbols() {}
#[test] fn bolt_ascii_turns_symbols_off_and_keeps_colour() {}
#[test] fn json_has_no_colour_even_on_a_terminal() {}
#[test] fn without_a_terminal_bolts_own_words_are_plain() { /* Review Focus 6: every command; stderr empty on success; no ESC; ASCII symbols */ }
#[test] fn warnings_come_after_an_empty_line_each_with_its_prefix() {}
```
- [ ] **Step 2:** Run them. Expected: FAIL (no `style` module).
- [ ] **Step 3:** Implement. `find_out` reads the environment once into a `OnceLock`. The Windows request is `anstyle_query::windows::enable_ansi_colors()`; inside Windows Terminal (`WT_SESSION`) it isn't needed. `tell_failure` prints `error:` and, after an empty line, `hint:`; `print_warnings` puts one empty line before the first warning. Messages are escaped as today before `paint` or `prefixed` sees them.
- [ ] **Step 4:** Update the tests whose text changed (sizes, `->` for `→` without a terminal, the empty line before a hint). Run `tests/terminal.rs`, then each changed test file, then the workspace. Expected: PASS. `cargo deny check` passes with the new package.
- [ ] **Step 5:** Commit: `Know what the terminal can show, and lay out errors, hints and warnings by it`.

### Task 2: Progress

Decision 120; terminal.md §8; follow-up "Progress during an import's first pass".

**Files:**
- Create: `crates/bolt-vcs/src/progress.rs`.
- Modify: `crates/bolt-vcs/src/output.rs` (the old `Progress` goes); `crates/bolt-vcs/src/main.rs` (import's three passes; the turning mark around `save`, `goto`, `discard`, `shelf`, `unshelf`, `undo`, `redo`, `tidy`; questions and the waiting line take it off the screen first); `crates/bolt-vcs-core/src/import.rs` (the first pass reports how far it is).

**Interfaces:**
```rust
// engine: which pass an import is in
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImportPass { ReadingHistory, CheckingLfs, Importing }
// ImportRequest.progress becomes: &'a mut dyn FnMut(ImportPass, u64, Option<u64>)
// progress.rs
pub struct Bar { /* … */ }
impl Bar {
    pub fn new(label: &str) -> Bar;                      // draws nothing for 400 ms
    pub fn set_label(&mut self, label: &str);
    pub fn update(&mut self, done: u64, total: Option<u64>, as_bytes: bool);
    pub fn finish(self);                                 // erases the line
}
pub struct Working { /* a thread that draws the turning mark */ }
impl Working {
    pub fn start(label: &'static str) -> Working;        // nothing when stderr is no terminal
    pub fn pause(&self);                                 // before a question or the waiting line
    pub fn resume(&self);
}                                                        // dropping it erases the line
pub fn bar_line(label: &str, done: u64, total: u64, as_bytes: bool, screen: Screen) -> String;   // pure, for tests
```

- [ ] **Step 1: Write the failing tests:**

```rust
#[test] fn a_bar_line_has_the_label_padded_to_24_then_24_cells_the_percentage_and_the_counts() {}
#[test] fn under_70_columns_the_counts_go_and_under_50_the_cells() {}
#[test] fn a_bar_uses_plain_cells_without_symbols() {}
#[test] fn a_long_label_is_cut_to_24() {}
#[test] fn nothing_is_drawn_before_400_ms_or_more_often_than_every_100() { /* an injected clock */ }
#[test] fn finishing_erases_the_line() {}
#[test] fn an_import_reports_its_first_pass() { /* engine: CheckingLfs heard with a rising count, in a repository with a Git LFS sign */ }
#[test] fn no_progress_reaches_stderr_without_a_terminal() { /* CLI: import and save */ }
```
- [ ] **Step 2:** Run them. Expected: FAIL.
- [ ] **Step 3:** Implement. The bar writes to a `Write` it is handed, so tests read its bytes. The turning mark is the only thread the command-line crate starts; it holds nothing but a flag and the label.
- [ ] **Step 4:** Run the tests, `--test import`, `--test import_lfs`, then the workspace. Expected: PASS.
- [ ] **Step 5:** Commit: `Show progress as a bar or a turning mark, and during an import's first pass`.

### Task 3: Change IDs in Jujutsu's letters

Decision 122; storage-format.md §5.

**Files:**
- Modify: `crates/bolt-vcs-core/src/id.rs` (`to_letters`, `from_letters`, `from_jujutsu`), `history.rs` (the prefix parser's comment), the tests that pin letters (`id.rs`, `import.rs`), doc comments that say `k` is 0.

- [ ] **Step 1: Write the failing tests:**

```rust
#[test] fn zero_is_z_and_fifteen_is_k() { /* [0x00; 16] -> "z"×32; [0xff; 16] -> "k"×32; [0x0f, 0xf0, …] -> "zkkz…" */ }
#[test] fn letters_read_back_to_the_same_bytes() { /* proptest */ }
#[test] fn an_imported_save_shows_the_letters_its_change_id_header_has() { /* header "zyxw…": history prints the same */ }
```
- [ ] **Step 2:** Run them. Expected: FAIL.
- [ ] **Step 3:** Implement: `b'z' - half` and its inverse. `from_jujutsu` keeps accepting capitals. Nothing stored changes: `change_ids.json` and every other vector still pass untouched.
- [ ] **Step 4:** Run `-p bolt-vcs-core`, `--test import`, `--test cli`. Expected: PASS.
- [ ] **Step 5:** Commit: `Write change IDs in Jujutsu's letters`.

### Task 4: `bolt.toml` is read

Decision 114; storage-format.md §12; `rules.json`.

**Files:**
- Create: `crates/bolt-vcs-core/src/rules.rs`; `fuzz/fuzz_targets/parse_rules.rs`.
- Modify: `crates/bolt-vcs-core/src/lib.rs`, `repo.rs` (read after the lock; keep what was read), `store.rs` (the threshold), `ingest.rs`, `snapshot.rs` (the two question numbers; the warnings; `is_small`), `writeout.rs` and `import/convert.rs` (their `is_cut` and `is_small` calls), `warning.rs` (two kinds), `crates/bolt-vcs-core/tests/vectors.rs`, `fuzz/Cargo.toml`, `.github/workflows/nightly.yml` (the fuzz list). These last five engine files belong to the speed session's paths: change only the calls named here.

**Interfaces:**
```rust
// rules.rs
pub const MAX_RULES_FILE_BYTES: u64 = 1 << 20;
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rule { pub files: Vec<String>, pub lockable: Option<bool>, pub conflict_markers: Option<bool> }
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rules { pub preset: Option<String>, pub chunk_files_over: u64, pub ask_over_bytes: u64, pub ask_over_files: u64, pub rules: Vec<Rule> }
impl Default for Rules { /* 1 MiB, 100,000,000, 10,000, no rules */ }
pub fn parse_size(text: &str) -> Option<u64>;
/// A setting's size as `bolt.toml` would write it: the largest unit that gives it exactly, the 1,024 family first.
pub fn size_text(bytes: u64) -> String;       // 1048576 -> "1 MiB", 100000000 -> "100 MB", 1500 -> "1500 B"
/// What a `bolt.toml`'s text means, and each name passed over with why. `Err` says why the text isn't TOML.
pub fn parse(text: &[u8]) -> Result<(Rules, Vec<(String, String)>), String>;
#[derive(Clone, Debug, Default)]
pub struct RulesRead { pub rules: Rules, pub file: bool, pub skipped: Option<String>, pub ignored: Vec<(String, String)> }
/// Reads `bolt.toml` in `top`. Never fails: what can't be used is in `skipped` or `ignored`.
pub fn read(top: &Dir) -> RulesRead;
impl RulesRead { pub fn warnings(&self) -> Vec<Warning>; }       // rules_file_skipped, rules_ignored
// repo.rs
impl Repository {
    pub fn rules(&self) -> &RulesRead;        // what the last lock_for_change read; Bolt's own values before that
    pub fn read_rules(&self) -> RulesRead;    // reads the folder now, for a command that takes no lock
}
// store.rs
impl Store {
    pub(crate) fn set_chunk_from(&self, bytes: u64);
    pub(crate) fn is_cut(&self, size: u64) -> bool;      // the tests' override of chunks::cut_sizes still wins
    pub(crate) fn is_small(&self, size: u64) -> bool;
}
```

- [ ] **Step 1: Write the failing tests:**

```rust
// rules.rs
#[test] fn every_vector_reads_as_it_says() { /* rules.json: values, kept rules, ignored names in order, skipped */ }
#[test] fn sizes_read_in_both_families() {}
#[test] fn a_size_that_does_not_fit_64_bits_is_no_size() {}
#[test] fn a_settings_size_is_written_in_the_largest_exact_unit_and_reads_back() { /* 1 MiB, 100 MB, 1 GiB, 5 GB, 1500 B, 0 B; parse_size(size_text(n)) == n */ }
#[test] fn a_file_of_nested_brackets_is_passed_over_without_overflowing_the_stack() { /* 100,000 `[` */ }
#[test] fn a_link_a_folder_and_a_file_over_1_mib_are_each_passed_over() { /* read(), through a real folder */ }
#[test] fn the_file_is_never_read_through_a_link() { /* a link to a file outside: skipped, and the target's text appears in no warning */ }
#[test] fn warnings_escape_what_the_file_says() { /* a key holding ESC */ }
// snapshot / CLI
#[test] fn a_project_without_a_rules_file_saves_as_before() { /* Review Focus 7: fixed folder, fixed identity and time: the save ID and pack names pinned */ }
#[test] fn chunk_files_over_decides_which_files_are_cut() { /* "4 MiB": a 2 MiB file stored whole, a 5 MiB one in chunks; same file IDs either way */ }
#[test] fn safety_copies_and_imports_use_the_threshold_too() {}
#[test] fn the_question_uses_the_files_numbers() { /* ask-before-saving-over = "1 KiB": a 2 KiB save asks; "1 TiB" and files 1000000: it doesn't */ }
#[test] fn a_rules_file_bolt_cannot_use_never_stops_a_save() { /* Review Focus 1: each case saves, with one warning */ }
#[test] fn only_save_shows_the_rules_warnings() { /* goto, discard, status: none */ }
#[test] fn the_rules_are_read_from_the_folder_not_the_save() {}
```
- [ ] **Step 2:** Run them. Expected: FAIL.
- [ ] **Step 3:** Implement. `parse` walks a `toml::Table` key by key, so one bad key never fails the rest; it never uses a derive on the whole file. `read` uses `safe_fs::read_capped_in` through `Dir::open_top`. `lock_for_change` calls `read` once it holds the lock and hands the threshold to the store; `save_folder` adds `rules().warnings()` to its own and gives the two numbers to `large_save`. Check how the `toml` crate bounds nesting before relying on it; if it doesn't, bound the text's bracket depth before parsing.
- [ ] **Step 4:** Run `-p bolt-vcs-core rules`, `--test vectors`, `--test chunks`, `--test cli`, then the workspace. Expected: PASS, with every existing vector untouched.
- [ ] **Step 5:** Commit: `Read bolt.toml: the chunk threshold and the large-save question's numbers`.

### Task 5: What a preset writes, and what a folder looks like

Decision 113; storage-format.md §12; `presets.json`.

**Files:**
- Create: `crates/bolt-vcs-core/src/presets.rs`; `crates/bolt-vcs-core/src/presets/` (one text file per piece, included with `include_str!`).
- Modify: `crates/bolt-vcs-core/src/lib.rs`, `crates/bolt-vcs-core/tests/vectors.rs`.

**Interfaces:**
```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)] pub enum Preset { Code, Game, Custom }
impl Preset { pub fn name(self) -> &'static str; pub fn from_name(name: &str) -> Option<Preset>; }
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)] pub enum Engine { Unity, Unreal, Godot }
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)] pub enum Language { Node, Python, Rust, Go, Dart, Dotnet, Maven, Gradle }
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Found { pub engines: Vec<Engine>, pub languages: Vec<Language> }       // each in the enum's order, once
pub const LOOK_AT_MOST: u64 = 20_000;
#[derive(Clone, Debug)]
pub struct Looked { pub found: Found, pub bytes: u64, pub counted_all: bool, pub has_rules_file: bool }
/// Names in the top folder, and the sizes of the files below it. Reads no file's contents.
pub fn look(top: &Dir) -> io::Result<Looked>;
pub fn suggested(found: &Found) -> Preset;
pub fn bolt_toml(preset: Preset, found: &Found) -> Option<String>;            // None for Custom
pub fn boltignore_block(preset: Preset, found: &Found) -> Option<String>;     // None for Custom
/// A `.boltignore` with the first block replaced by `block`, added at the top, or removed. `None`: no file is left.
pub fn edit_boltignore(before: Option<&[u8]>, block: Option<&str>) -> Option<Vec<u8>>;
pub fn block_lines(boltignore: &[u8]) -> Option<usize>;                       // the first block's lines, marks not counted
```

- [ ] **Step 1: Write the failing tests:**

```rust
#[test] fn every_preset_vector_is_written_byte_for_byte() { /* presets.json: presets */ }
#[test] fn every_boltignore_edit_gives_its_vector() { /* presets.json: boltignore_edits */ }
#[test] fn what_a_preset_writes_reads_back_as_its_rules() { /* bolt_toml -> rules::parse: nothing ignored */ }
#[test] fn every_ignore_line_is_one_gits_matcher_can_use() { /* each block through ignore::Rules::parse */ }
#[test] fn no_preset_line_keeps_out_an_asset_by_its_extension() { /* *.obj, *.exe, *.dll, *.png, *.log, *.tmp match nothing in any block */ }
#[test] fn unity_unreal_and_godot_are_each_found_by_their_files() {}
#[test] fn each_language_is_found_by_its_manifest() {}
#[test] fn a_folder_named_like_a_project_file_is_not_one() { /* a folder called project.godot */ }
#[test] fn sizes_leave_out_git_bolt_and_the_engines_caches() {}
#[test] fn looking_stops_after_20_000_entries() {}
#[test] fn looking_never_goes_through_a_link() { /* a link to a big folder outside: not counted */ }
#[test] fn the_suggestion_is_game_then_code_then_custom() {}
```
- [ ] **Step 2:** Run them. Expected: FAIL.
- [ ] **Step 3:** Implement. The pieces are the texts of storage-format.md §12; `bolt_toml` and `boltignore_block` only put them together. `look` walks with an explicit stack of folder handles, at most 128 deep, counting an entry for each name it meets.
- [ ] **Step 4:** Run `-p bolt-vcs-core presets` and `--test vectors`. Expected: PASS.
- [ ] **Step 5:** Commit: `Add the presets' rules as data, and look at a folder to suggest one`.

### Task 6: The `setup` step

Decision 115; storage-format.md §9.

**Files:**
- Create: `crates/bolt-vcs-core/src/setup.rs` (with its tests).
- Modify: `crates/bolt-vcs-core/src/lib.rs`, `kept.rs` (`kept_save` and `overlay` reachable from `setup.rs`), `repo.rs` or `switch.rs` only as far as a new error needs.

**Interfaces:**
```rust
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuleFileChange { pub path: &'static str, pub before: Option<Vec<u8>>, pub after: Option<Vec<u8>> }   // "bolt.toml", ".boltignore"
#[derive(Clone, Debug)]
pub struct SetupPlan { pub preset: Preset, pub found: Found, pub changes: Vec<RuleFileChange> }   // only files that would change
/// What switching to `preset` would do to the two rule files. Changes nothing.
pub fn plan_setup(root: &Path, preset: Preset) -> Result<SetupPlan, SetupError>;
#[derive(Debug)]
pub struct SetupOutcome { pub written: Vec<&'static str>, pub warnings: Vec<Warning> }
/// Writes the plan as one `setup` step. Refuses if either file changed since the plan was made.
pub fn apply_setup(repo: &mut Repository, plan: &SetupPlan, who: &Identity, git_excludes: Option<&Path>,
                   on_wait: &mut dyn FnMut(&str)) -> Result<SetupOutcome, SetupError>;
pub enum SetupError { NotAFile { path: &'static str }, TooLarge { path: &'static str }, ChangedSincePlan { path: &'static str }, InTheWay(/* as unshelf's */), Repo(RepoError), /* … */ }
```
The step follows `unshelf` (`shelf.rs`): the "from" save is the current save's listing with the two paths as the folder holds them now (`kept::overlay`, then a kept save), the "to" save the same with the plan's `after`; both are stored, then `writeout::plan(from, to)`, `kept::switch_saves` and `switch::switch` with the command name `setup` and the state unchanged.

- [ ] **Step 1: Write the failing tests:**

```rust
#[test] fn a_plan_lists_only_the_files_that_would_change() {}
#[test] fn a_plan_for_the_same_preset_twice_is_empty() {}
#[test] fn applying_writes_both_files_and_records_one_setup_entry() {}
#[test] fn undo_removes_a_file_the_step_made_and_redo_writes_it_again() {}
#[test] fn undo_puts_back_edits_that_were_never_saved() { /* Review Focus 4 */ }
#[test] fn a_boltignore_of_the_users_own_keeps_every_line() {}
#[test] fn custom_removes_the_rules_file_and_the_block() {}
#[test] fn other_unsaved_work_is_left_alone() {}
#[test] fn the_state_does_not_change() { /* lines, position, shelves as before; status shows the two files as unsaved */ }
#[test] fn a_link_or_a_folder_named_like_a_rule_file_refuses() { /* nothing changes; the link's target is never read */ }
#[test] fn a_file_edited_after_the_plan_refuses() {}
#[test] fn a_setup_stopped_at_any_write_is_finished_by_the_next_command() { /* the fault sweep the other steps use */ }
#[test] fn the_entrys_two_saves_hold_the_two_files_before_and_after() { /* storage-format.md §9 */ }
```
- [ ] **Step 2:** Run them. Expected: FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run `-p bolt-vcs-core setup`, `-p bolt-vcs-core undo`, `--test undo`, `--test recovery`. Expected: PASS.
- [ ] **Step 5:** Commit: `Write a preset's rule files as one step that undo takes back`.

### Task 7: `bolt start` asks, and `bolt setup`

Decisions 113 and 115; json.md (`start`, `setup`); terminal.md §10.

**Files:**
- Create: `crates/bolt-vcs/tests/setup.rs`.
- Modify: `crates/bolt-vcs/src/main.rs` (`Start { preset }`, `Setup { preset, yes }`), `output.rs`; every test and helper that runs `bolt start` (`--preset custom`); `bench/run.sh` (its one `bolt start`).

**Interfaces:** the command line is `bolt start [--preset code|game|custom]` and `bolt setup [--preset code|game|custom] [--yes]`. What they print is decision 113's two screens and decision 115's list, laid out by terminal.md; the JSON is json.md's.

- [ ] **Step 1: Write the failing tests** (`tests/setup.rs`, `tests/cli.rs`):

```rust
#[test] fn start_without_a_terminal_needs_a_preset_and_names_the_flag() { /* exit 1; nothing made; the hint names --preset and the suggestion */ }
#[test] fn start_with_json_never_asks() {}
#[test] fn start_preset_custom_writes_no_file_and_is_as_before() {}
#[test] fn start_preset_game_in_a_unity_folder_writes_unitys_rules() { /* the two files equal presets.json's game-unity */ }
#[test] fn start_preset_code_writes_the_lists_of_the_languages_found() {}
#[test] fn start_says_what_the_rules_are_in_words() {}
#[test] fn undo_after_a_start_gives_the_folder_back() { /* Review Focus 3 */ }
#[test] fn a_folder_with_its_own_rules_file_keeps_it_and_is_asked_nothing() { /* without a terminal and without --preset: it starts; JSON preset is null */ }
#[test] fn preset_with_a_rules_file_already_there_is_refused_before_anything_is_made() {}
#[test] fn what_start_says_about_the_folder_names_nothing_from_it() { /* a file called "\x1b[31mAssets": only fixed words */ }
#[test] fn start_json_has_preset_found_and_rules_written() { /* snapshot */ }
#[test] fn setup_shows_the_rules_as_bolt_reads_them() {}
#[test] fn setup_without_a_rules_file_shows_bolts_own_values() {}
#[test] fn setup_shows_what_in_the_rules_file_was_passed_over() {}
#[test] fn setup_preset_shows_the_line_changes_then_stops_without_yes() { /* exit 1, names --yes, nothing written */ }
#[test] fn setup_preset_yes_applies_and_undo_takes_it_back() {}
#[test] fn setup_preset_with_nothing_to_change_says_so() { /* exit 0, no entry */ }
#[test] fn setup_json_matches_json_md() { /* both forms; snapshots */ }
#[test] fn a_preview_escapes_the_lines_it_shows() { /* a .boltignore line holding ESC */ }
```
- [ ] **Step 2:** Run them. Expected: FAIL.
- [ ] **Step 3:** Implement. The question goes to stderr and reads stdin, asked only when both are terminals and `--json` isn't given; the preview of a switch reuses `bolt changes`' line writer on the two texts. Then, as its own commit, add `--preset custom` to every `bolt start` in the tests and helpers and in `bench/run.sh`.
- [ ] **Step 4:** Run `--test setup`, `--test cli`, then every test file in turn. Expected: PASS.
- [ ] **Step 5:** Commit: `Ask what kind of project bolt start begins, and add bolt setup`.

### Task 8: Git's words

Decision 116; storage-format.md §10.

**Files:**
- Create: `crates/bolt-vcs/src/git_words.rs`; `crates/bolt-vcs/tests/git_words.rs`.
- Modify: `crates/bolt-vcs/src/main.rs` (the command line passes through `translate` before clap reads it).

**Interfaces:**
```rust
pub enum Typed {
    Bolt(Vec<OsString>),                                   // nothing of Git's
    Shortcut { word: &'static str, args: Vec<OsString> },  // run `args`; `word` is what was typed
    Explain(&'static str),                                 // say this on stderr, exit code 2
}
/// What the command line (without the program's name) means.
pub fn translate(args: Vec<OsString>) -> Typed;
pub fn hint(word: &str) -> Option<&'static str>;           // "in Bolt, commit is save: bolt save \"message\""
/// True the first time `word` is met for this user; then remembers it in `git-words`.
pub fn first_time(word: &str, settings_folder: Option<&Path>) -> bool;
```

- [ ] **Step 1: Write the failing tests:**

```rust
#[test] fn each_shortcut_runs_its_bolt_command() { /* the table of decision 116, row by row */ }
#[test] fn commit_takes_gits_message_forms() { /* -m x, --message x, --message=x, -am x, -a -m x */ }
#[test] fn save_itself_still_refuses_dash_m() {}
#[test] fn stash_pop_brings_a_shelf_back() { /* Review Focus 8: and no shelf is called "pop" */ }
#[test] fn stash_list_lists() {}
#[test] fn restore_without_a_path_explains_and_discards_nothing() { /* unsaved changes still there; exit 2 */ }
#[test] fn restore_with_paths_discards_only_those() {}
#[test] fn add_explains_and_changes_nothing() { /* exit 2; stderr explains; the diary has no new entry */ }
#[test] fn each_explained_word_explains() { /* rm, mv, reset, merge, rebase, push, pull, clone, clean, … */ }
#[test] fn checkout_dash_b_and_branch_with_a_name_explain() {}
#[test] fn the_hint_shows_once_per_word_on_a_terminal() { /* BOLT_FAKE_TERMINAL: first run hints, second doesn't; git-words holds the word */ }
#[test] fn no_hint_without_a_terminal_or_with_json() { /* and nothing is remembered */ }
#[test] fn a_git_words_file_bolt_cannot_use_is_never_an_error() { /* a folder in its place; 1 MiB of junk */ }
#[test] fn the_diary_names_bolts_command() { /* bolt commit: undo --list says save */ }
#[test] fn help_never_names_a_git_word() {}
#[test] fn json_comes_through_a_shortcut() { /* bolt --json commit -m x, and bolt commit --json -m x */ }
```
- [ ] **Step 2:** Run them. Expected: FAIL.
- [ ] **Step 3:** Implement. `translate` looks only at the first argument that isn't `--json`; everything it doesn't know stays as typed for clap.
- [ ] **Step 4:** Run `--test git_words`, `--test cli`. Expected: PASS.
- [ ] **Step 5:** Commit: `Accept Git's words as shortcuts, and explain the ones Bolt doesn't run`.

### Task 9: `bolt verify`, and two things that no longer stop `tidy`

Decisions 118 and 123 (items 6 and 7); storage-format.md §11; json.md (`verify`).

**Files:**
- Create: `crates/bolt-vcs/tests/verify.rs`.
- Modify: `crates/bolt-vcs-core/src/tidy.rs` (marking usable by `verify`; `state/tidied` written before anything is removed and read as the earliest the window may start; a diary end that can't be read back is passed over with a warning), `verify.rs`, `warning.rs`; `crates/bolt-vcs/src/main.rs`, `output.rs`; `crates/bolt-vcs/tests/redteam7.rs` (`rt7_2` runs again).

**Interfaces:**
```rust
// verify.rs
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProblemKind { PackDamaged, PackUnreadable, ObjectDamaged, ObjectMissing, HeadLost }
#[derive(Clone, Debug)]
pub struct Problem { pub kind: ProblemKind, pub pack: Option<String>, pub object: Option<Id>,
                     pub needed_by: Option<(Id, String)>, pub message: String }
pub const PROBLEMS_KEPT: usize = 1_000;
#[derive(Clone, Debug, Default)]
pub struct Verified { pub packs: u64, pub objects: u64, pub bytes: u64, pub saves: u64, pub entries: u64,
                      pub lines: u64, pub shelves: u64, pub problems_total: u64, pub problems: Vec<Problem>,
                      pub warnings: Vec<Warning> }
/// Checks every pack, then that the project has all it needs. Takes no lock and changes nothing.
/// `progress` hears bytes of pack read and bytes in all.
pub fn verify(repo: &Repository, window_days: u32, now_ms: i64, progress: &mut dyn FnMut(u64, u64)) -> Verified;
// verify_store(packs_dir) stays for the tests that use it, sharing the per-pack check.
```

- [ ] **Step 1: Write the failing tests:**

```rust
// engine
#[test] fn an_intact_project_has_no_problems_and_counts_what_it_checked() {}
#[test] fn a_flipped_byte_in_any_object_is_found_and_the_rest_is_still_checked() {}
#[test] fn a_record_with_a_good_fingerprint_that_does_not_decode_is_damaged() {}
#[test] fn an_index_that_fails_its_checksum_is_a_damaged_pack() {}
#[test] fn a_pack_bolt_cannot_open_for_another_reason_is_unreadable_not_damaged() {}
#[test] fn a_deleted_pack_shows_as_objects_the_project_needs() { /* object_missing, naming a save and path */ }
#[test] fn a_missing_chunk_of_a_needed_file_is_missing() {}
#[test] fn a_chunk_list_that_builds_another_file_is_damaged() {}
#[test] fn a_lost_head_is_one_problem_and_the_packs_are_still_checked() {}
#[test] fn what_an_earlier_tidy_freed_is_no_problem() { /* the window made longer after a tidy: state/tidied bounds it */ }
#[test] fn a_save_missing_inside_the_window_is_a_problem() {}
#[test] fn a_pack_that_goes_away_mid_check_is_no_problem() { /* Review Focus 5: a pack merged between the listing and its turn */ }
#[test] fn a_pack_that_arrives_mid_check_is_checked() {}
#[test] fn more_than_1000_problems_are_counted_and_1000_kept() {}
#[test] fn a_fifo_named_like_a_pack_is_refused_within_the_limit() { /* as the existing no-hang test */ }
#[test] fn verify_writes_nothing() { /* every file in .bolt byte for byte and with its times, format included */ }
// tidy
#[test] fn rt7_2_raising_the_undo_window_after_a_tidy_leaves_tidy_working() { /* un-ignored */ }
#[test] fn tidy_writes_the_window_it_used_before_it_removes_anything() { /* state/tidied: digits and an LF; a tidy stopped right after still left it */ }
#[test] fn a_save_missing_inside_the_window_still_stops_tidy() {}
#[test] fn a_missing_save_the_current_state_needs_still_stops_tidy() {}
#[test] fn a_tidied_file_of_junk_or_from_the_future_counts_as_absent() { /* a folder, 1 MiB, letters, now + 1 day */ }
#[test] fn a_planted_tidied_file_cannot_free_undo_data_newer_than_it_names() {}
#[test] fn a_diary_end_whose_parent_is_missing_is_passed_over_with_a_warning() {}
#[test] fn tidy_keeps_what_it_reached_from_such_an_end() {}
// CLI
#[test] fn verify_says_verified_and_exits_0() {}
#[test] fn verify_lists_problems_and_exits_1() { /* at most 20 a kind, then how many more */ }
#[test] fn verify_json_matches_json_md() { /* snapshot; error present only with problems */ }
#[test] fn verify_names_a_pack_with_an_escaped_name() {}
```
- [ ] **Step 2:** Run them. Expected: FAIL.
- [ ] **Step 3:** Implement. `verify` lists the packs through the repository's own folder handle, checks each, lists again, and checks any new one until a listing brings nothing new (at most 8 rounds); a pack gone when opened is skipped. Marking is `tidy`'s own, with `tidy`'s window (bounded by `state/tidied`), called so that it collects what it can't read instead of stopping; an ID it wants that no index holds is a candidate for `object_missing`. Before anything is reported missing, `verify` looks once more from the start (`head`, the pack listing, the needs), and reports only what is still missing; only then are the saves walked once more to name one save and path that need it. Records are decoded with the decoders their kinds already have.
- [ ] **Step 4:** Run `-p bolt-vcs-core verify`, `-p bolt-vcs-core tidy`, `--test verify`, `--test tidy`, `--test redteam7`, then the workspace. Expected: PASS.
- [ ] **Step 5:** Commit: `Add bolt verify, and let tidy pass over what an earlier tidy freed`.

### Task 10: `save`, `status`, `changes`, `history` and `show` in the new design

Decision 120; terminal.md; the canvas's screens for these commands; the review's fact list.

**Files:** `crates/bolt-vcs/src/main.rs`, `output.rs`; `docs/terminal.md` (§12); the tests of these commands (`cli.rs`, `status.rs`, `changes.rs`, `amend.rs`, `lines.rs`, `terminal.rs`).

**How a restyle task works** (the same for Tasks 10, 11 and 12):
1. Read the canvas's screens for the group, and the review. For every line of every screen, either build it or write down why not: the fact it gets wrong, and Bolt's fact. That list goes into the task's report, for the rulings file.
2. The outcome line first, details indented four, lists as terminal.md §5 and §7, a `Next:` or "Changed your mind?" line where terminal.md §6 gives one. Colours by role. Errors, warnings and hints keep their words.
3. Update the tests that match the old text. Add one test per command that runs it with a terminal faked at 60, 80 and 120 columns, and one that it prints nothing new without a terminal.
4. Fill in terminal.md §12 for the group: each state's outcome line.

**Facts to keep:** no line counts per file in `save` or `status`; `Updated` for an amend; added, removed, modified and kind changed are the only kinds; `status --full`; `changes --save`; the first-save stops (line endings, Git LFS pointers) and their hints as they are; `show` gives author and saver, and the full fingerprint is 64 hex digits.

- [ ] **Step 1: Write the failing tests:**

```rust
#[test] fn save_says_saved_then_the_id_the_message_and_the_line() {}
#[test] fn an_amend_says_updated_and_that_the_id_stays() {}
#[test] fn nothing_to_save_names_the_last_save_and_exits_0() {}
#[test] fn a_partial_save_says_what_it_left_out() {}
#[test] fn status_opens_with_where_you_are_then_lists_changes_in_columns() {}
#[test] fn status_cuts_a_long_path_from_its_middle_only_on_a_terminal() { /* 60, 80, 120 columns, and none */ }
#[test] fn history_marks_the_current_save_and_cuts_long_messages_at_the_end() {}
#[test] fn show_lists_what_a_save_changed_with_sizes_in_1024_units() {}
#[test] fn changes_colours_only_added_removed_and_hunk_lines() {}
#[test] fn each_of_these_commands_prints_no_new_line_without_a_terminal() {}
```
- [ ] **Step 2:** Run them. Expected: FAIL.
- [ ] **Step 3:** Implement, following the method above.
- [ ] **Step 4:** Run each touched test file, then the workspace. Expected: PASS.
- [ ] **Step 5:** Commit: `Lay out save, status, changes, history and show in the new design`.

### Task 11: `goto`, `discard`, lines, undo, shelves and `tidy` in the new design

Decision 120; terminal.md; the canvas's screens for these commands; the review's fact list.

**Files:** `crates/bolt-vcs/src/main.rs`, `output.rs`; `docs/terminal.md` (§12); the tests of these commands (`goto.rs`, `discard.rs`, `lines.rs`, `undo.rs`, `shelf.rs`, `tidy.rs`, `recovery.rs`, `terminal.rs`).

**How a restyle task works:** as in Task 10 (its four numbered points apply here word for word: read the screens and write down every fact the canvas gets wrong; outcome first; update and add tests at 60, 80 and 120 columns and without a terminal; fill in terminal.md §12).

**Facts to keep:** there is no `goto --continue`: the next changing command finishes a stopped switch and says so; files in use fail with exit code 1 after the rest is written; an undo or redo stopped by unsaved work names only `--shelve`; a bare `bolt discard` throws away every unsaved change, undoably, with no `--all`; a new shelf takes the smallest free number and numbers never shift; `bolt unshelf` brings back the newest; default shelf messages are `Shelved`, `Set aside by undo` and `Set aside by redo`; `undo --list` shows the steps redo can bring back, then up to 20 undo steps, then "and more"; entry IDs are 12 hex digits and `undo --to` needs at least 8; `tidy` keeps the undo window and leaves packs changed in the last 24 hours; its "about X to free" line stays on stderr on a terminal; a lost `head` stops every command and says the saves are safe.

- [ ] **Step 1: Write the failing tests:**

```rust
#[test] fn goto_says_switched_to_and_how_many_files_it_wrote() {}
#[test] fn goto_stopped_by_unsaved_work_lists_the_files_and_changes_nothing() {}
#[test] fn discard_says_threw_away_and_that_undo_brings_it_back() {}
#[test] fn line_marks_the_current_line_with_the_marker() {}
#[test] fn undo_and_redo_say_what_they_took_back_or_brought_back() {}
#[test] fn undo_list_numbers_the_steps_and_marks_where_you_are() {}
#[test] fn shelf_and_unshelf_say_the_shelfs_number() {}
#[test] fn shelf_list_aligns_its_columns() {}
#[test] fn tidy_says_freed_and_how_many_packs() {}
#[test] fn changed_your_mind_shows_only_on_a_terminal() {}
#[test] fn each_of_these_commands_fits_60_80_and_120_columns() {}
#[test] fn each_of_these_commands_prints_no_new_line_without_a_terminal() {}
```
- [ ] **Step 2:** Run them. Expected: FAIL.
- [ ] **Step 3:** Implement, following the method above.
- [ ] **Step 4:** Run each touched test file, then the workspace. Expected: PASS.
- [ ] **Step 5:** Commit: `Lay out goto, discard, lines, undo, shelves and tidy in the new design`.

### Task 12: `import` and help in the new design

Decision 120; terminal.md §11; the canvas's screens for `import` and help; the review's fact list; decisions 085, 089 and 091 for the stops.

**Files:**
- Create: `crates/bolt-vcs/src/help.rs`.
- Modify: `crates/bolt-vcs/src/main.rs`, `import_output.rs`; `docs/terminal.md` (§12); `crates/bolt-vcs/tests/import.rs`, `import_lfs.rs`, `cli.rs`, `terminal.rs`.

**How a restyle task works:** as in Task 10 (its four numbered points apply here word for word).

**Facts to keep:** `bolt import` takes no argument and works in place, next to `.git/`; a second import is refused; uncommitted Git work comes along; an import that stops keeps the started `.bolt`; the summary's counts are those `import_output.rs` prints today; every Git LFS stop and its hint are worded as decisions 085, 089 and 091 word them, and none takes a flag. The canvas doesn't cover `tidy` stopping at a pack it can't read, or the import's work stop with decision 091's hint: lay those two out by the same rules as the others. Help lists only what each command really takes, and no Git word.

**Interfaces:**
```rust
// help.rs
pub fn overview() -> String;                          // terminal.md §11: the three groups
pub fn examples(command: &str) -> &'static [(&'static str, &'static str)];   // (command line, what it does)
```

- [ ] **Step 1: Write the failing tests:**

```rust
#[test] fn the_import_summary_leads_with_imported_and_its_counts() {}
#[test] fn each_git_lfs_stop_keeps_its_words_and_names_no_flag() {}
#[test] fn the_work_stop_is_laid_out_like_every_other_error() { /* decision 091's hint, whole */ }
#[test] fn bolt_alone_help_and_dash_dash_help_print_the_overview() {}
#[test] fn the_overview_lists_every_command_once_and_no_git_word() {}
#[test] fn each_commands_help_leads_with_examples_then_usage_then_options() {}
#[test] fn each_commands_help_leads_with_examples_that_parse() { /* every example line is run through clap and must parse */ }
#[test] fn help_lists_only_options_the_command_takes() { /* no -m, --wait, --repair, --allow-lfs-pointers */ }
#[test] fn the_import_summary_fits_60_80_and_120_columns() {}
```
- [ ] **Step 2:** Run them. Expected: FAIL.
- [ ] **Step 3:** Implement, following the method above.
- [ ] **Step 4:** Run each touched test file, then the workspace. Expected: PASS.
- [ ] **Step 5:** Commit: `Lay out import and help in the new design`.

### Task 13: The follow-ups

Decision 123, "Done in milestone 6", items 1 to 5 and 8 to 13. Each is small and has its own test; the premise of each was checked on 2026-11-09 and is checked again before the change.

**Files:** `crates/bolt-vcs-core/src/snapshot.rs` (the line-ending hint only), `import.rs` and `config.rs` (the user's `core.sparseCheckout`; `:(optional)`; `%(prefix)/`; `~\x`), `git/pack.rs` and `git/odb.rs` (the two hints), `writeout.rs` (`in_use` only), `undo.rs` (the pick-up), `repo.rs` (`SwitchStopped`'s hint; `owner_allows`), `crates/bolt-vcs/src/main.rs` (`changed_again`), `warning.rs` (`switch_dropped`), `crates/bolt-vcs/tests/redteam6.rs` (`rt6_3` runs again).

- [ ] **Step 1: Write the failing tests:**

```rust
#[test] fn the_line_ending_hint_names_the_whole_repository_and_eol_crlf_files() { /* `git rm --cached -r -q :/`; the sentence on eol=crlf */ }
#[test] fn a_sparse_checkout_in_the_users_git_settings_stops_an_import() { /* as the repository's own does */ }
#[test] fn an_optional_excludes_file_that_is_not_there_counts_as_none() { /* :(optional)~/nope */ }
#[test] fn an_optional_excludes_file_that_is_there_is_read() {}
#[test] fn a_prefix_path_stops_wherever_it_appears() { /* %(prefix)/x in core.excludesFile and in a file's include */ }
#[test] fn tilde_backslash_stops_on_windows() { /* ~\x: as Git for Windows 2.50.1 */ }
#[test] fn running_out_of_memory_building_a_delta_says_so() { /* not the git fsck hint */ }
#[test] fn a_raw_delta_read_that_ran_out_of_memory_says_so() {}
#[test] fn error_303_counts_as_a_file_in_use() { /* Windows only */ }
#[test] fn rt6_3_picking_up_a_lost_head_after_a_stopped_switch_lets_the_project_go_on() { /* un-ignored; a switch_dropped warning; the next save works */ }
#[test] fn a_pick_up_refuses_an_entry_whose_tag_list_or_server_lines_cannot_be_read() {}
#[test] fn a_switch_stopped_by_damage_or_a_missing_object_names_the_way_on() { /* deleting .bolt/state/journal */ }
#[test] fn a_safe_folders_entry_that_is_a_link_does_not_count() { /* a symlink on Unix, a junction on Windows; the refusal names the entry */ }
#[test] fn a_safe_folders_entry_that_is_a_real_folder_still_counts() {}
#[test] fn shelf_json_tells_its_own_race_from_nothing_to_shelve() { /* changed_again */ }
```
- [ ] **Step 2:** Run them. Expected: FAIL.
- [ ] **Step 3:** Implement, one item a commit. Strike each row in the follow-ups list as it lands.
- [ ] **Step 4:** Run each touched test file, then the workspace. Expected: PASS.
- [ ] **Step 5:** Commits, one per item, each named for what changed.

### Task 14: The JSON formats, read once before they freeze

Decision 117; json.md.

**Files:** `docs/json.md`; `crates/bolt-vcs/src/main.rs`, `output.rs`, `import_output.rs`; the snapshot tests.

- [ ] **Step 1:** Read every document in json.md against the code that prints it, and list every field name or shape that disagrees with another document's (one thing under two names, a count that is a string in one place and a number in another, a field a document should have and doesn't). Report the list; each change is a ruling made by the controller before anything is edited.
- [ ] **Step 2:** For each ruled change: the failing snapshot first, then the code, then json.md.
- [ ] **Step 3:** Add a test that walks every command's document on a small project and fails on a top-level field json.md doesn't list (`every_document_has_only_the_fields_json_md_lists`), and one that every document parses and holds `bolt_json: 1` and `warnings`.
- [ ] **Step 4:** Run every test file. Expected: PASS.
- [ ] **Step 5:** Commit: `Read the JSON formats once before they freeze`.

### Task 15: Proof

**Files:**
- Create: `crates/bolt-vcs/tests/corpus_rules.rs`.
- Modify: `crates/bolt-vcs/tests/terminal.rs`, `crates/bolt-vcs/tests/model.rs` (`setup` among the model's steps), `docs/testing.md`.

- [ ] **Step 1: Write the tests** (they should pass; one that fails is a bug to fix in the task that owns the code):

```rust
// corpus_rules.rs: a project that arrives with hostile rule files
#[test] fn a_rules_file_that_is_a_link_out_of_the_project() {}
#[test] fn a_rules_file_that_is_a_folder_a_pipe_or_2_mib() {}
#[test] fn a_rules_file_of_nested_tables_arrays_and_brackets() {}
#[test] fn a_rules_file_with_numbers_at_every_limit() {}
#[test] fn a_rules_file_whose_keys_and_values_hold_escape_codes() { /* save, setup and start: escaped everywhere, JSON included */ }
#[test] fn a_rules_file_with_a_million_rules_under_the_size_limit() { /* bounded time: the command ends within the limit */ }
#[test] fn a_boltignore_with_ten_thousand_start_marks() { /* setup --preset ends within the limit and touches only the first block */ }
#[test] fn a_boltignore_that_is_a_link_stops_setup_and_start() {}
#[test] fn planted_engine_files_only_change_the_suggestion() { /* --preset code in a folder with every engine's files writes Code's rules */ }
#[test] fn a_folder_of_a_million_names_is_looked_at_in_bounded_time() {}
#[test] fn rule_files_inside_an_imported_repository_are_data_like_any_other() { /* import, then save: the checked-out bolt.toml counts from the folder, never from Git's objects */ }
// terminal.rs
#[test] fn project_text_is_escaped_with_colour_on() { /* Review Focus 2: status, history, show, changes, goto, undo --list, shelf --list, setup, verify */ }
#[test] fn a_line_name_that_spells_an_escape_code_cannot_recolour_the_row() {}
#[test] fn cutting_a_path_never_cuts_through_an_escaped_character() {}
```
- [ ] **Step 2:** Run them; fix what fails where it belongs.
- [ ] **Step 3:** Add `setup --preset` (each preset) to the model test's random steps, with undo and redo after it.
- [ ] **Step 4:** Run `--test corpus_rules`, `--test terminal`, `--test model`. Expected: PASS.
- [ ] **Step 5:** Commit: `Prove hostile rule files and escaped text with colour on`.

### Task 16: The user guide, the README and `SECURITY.md`

Decision 119.

**Files:** Create `docs/guide.md`, `README.md`, `SECURITY.md`.

- [ ] **Step 1:** Write `docs/guide.md` for someone who has never used version control: installing (the archive, the checksum, the unsigned-program warnings on Windows and macOS); the first five minutes; every command with one example and what it prints; presets and editing `bolt.toml` and `.boltignore`; undo; shelves; lines; importing from Git, with a "coming from Git" table of words; `bolt verify` and `bolt tidy`; personal settings, with `safe-folders` (what it is for, when Bolt asks for it, why never to list a folder others can write to, and that an entry that is a link doesn't count); what v0.1 doesn't do yet.
- [ ] **Step 2:** Every command line in the guide is run by a test (`crates/bolt-vcs/tests/guide.rs`: `every_command_in_the_guide_runs`), so the guide can't drift: the test reads the fenced blocks marked `sh` and runs each `bolt …` line in a scratch project.
- [ ] **Step 3:** Write `README.md` (what Bolt is, install, five commands, the guide's link, "no license yet") and `SECURITY.md` (report through GitHub's private reporting for the repository; what to include; what to expect).
- [ ] **Step 4:** Run `--test guide`. Expected: PASS. Check every link.
- [ ] **Step 5:** Commit: `Add the user guide, the README and how to report a vulnerability`.

### Task 17: Packaging

Decision 119.

**Files:** `Cargo.toml` (version `0.1.0`), `Cargo.lock`; create `.github/workflows/release.yml`, `about.toml`, `about.hbs`; `docs/testing.md` (§4).

- [ ] **Step 1:** Set the version; `bolt --version` prints `bolt 0.1.0` (test: `version_is_0_1_0`).
- [ ] **Step 2:** Write `release.yml`:
  - starts on `workflow_dispatch` only, plus on a pull request that changes the workflow itself, where it builds and tests the archives and stops before the release;
  - `permissions: contents: read` at the top; four build jobs (`x86_64-pc-windows-msvc` with the C runtime linked in, `x86_64-unknown-linux-gnu` on the oldest Ubuntu runner, `aarch64-apple-darwin`, `x86_64-apple-darwin`), each `cargo build --release --locked -p bolt-vcs`, then a smoke test of the program it built (`--version`; in a scratch folder with its own `BOLT_CONFIG_DIR`: `start --preset custom`, a save, `history`, `verify`), then the archive (`bolt-0.1.0-<target>.zip` or `.tar.gz`: the program, `README.md`, `docs/guide.md`, `THIRD-PARTY-LICENSES.html`), uploaded as an artifact;
  - the notices come from `cargo-about`, one pinned release downloaded from its GitHub release and checked against a SHA-256 written in the workflow;
  - one last job, only on `workflow_dispatch`, with `contents: write`: downloads the four archives with `gh run download`, writes `SHA256SUMS`, and runs `gh release create v0.1.0 --draft` with the notes file and the five files. It refuses if a release or a tag named `v0.1.0` exists.
  - every action pinned to a commit, as in `ci.yml`.
- [ ] **Step 3:** Push; the pull-request run builds the four archives. Read its log: each smoke test passed, each archive holds its four files.
- [ ] **Step 4:** Commit: `Package v0.1: four builds, their checksums and a draft release, from a workflow started by hand`.

### Task 18: The docs match the code

**Files:** `docs/design.md`, `storage-format.md`, `json.md`, `terminal.md`, `security.md`, `testing.md`, `milestones.md`, `plans/2026-11-01-milestone-1-follow-ups.md`, `CLAUDE.md`.

- [ ] **Step 1:** design.md: §5.1, §5.6 to §5.9, §5.14 to §5.16 say what was built, in the present tense; the "built so far" list has `setup`, `verify` and `start --preset`; §7's list is current.
- [ ] **Step 2:** security.md: threat rows for a hostile `bolt.toml` and `.boltignore`, preset detection fooled by planted files, `verify` on hostile packs, escape codes with colour on and through `--json`, the `git-words` file, and the release workflow (what it can write, what it downloads, what is pinned); T27 and T28 lose their "before v0.1".
- [ ] **Step 3:** testing.md: the new test files, the fuzz target, the faked terminal, the guide test, the release workflow's pull-request run.
- [ ] **Step 4:** The follow-ups list: every row done is struck with its date; a "Deferred by milestone 6" section holds what this milestone put off, each with why. milestones.md: milestone 6's steps under "Presets and polish", each marked when done. CLAUDE.md's phase line.
- [ ] **Step 5:** Every doc checked for CR bytes and broken links. Commit: `Bring the docs up to date with milestone 6's presets and polish`.

### Task 19: Red team

Decision 024. Two sessions, each a fresh agent on the most capable model, in the no-network Docker sandbox (`redteam/sandbox.sh`), one sandbox on the machine at a time.

- [ ] **Step 1:** Write `redteam/attack-surface-m6.md`: what milestone 6 added, where the code is, the threat list, and the goals. New surfaces: a hostile `bolt.toml` or `.boltignore` in an imported or copied project; preset detection fooled by planted files; the `setup` step (links at the rule files' names, races, crashes); `bolt verify` on hostile packs and beside other commands; escape sequences on screen with colour on and through `--json`; Git's words (`git-words`, argument forms); the terminal's width and the cutting of paths; the release workflow.
- [ ] **Step 2:** Session 1 (RT10): attacks in `redteam/attacks9`, proofs as `crates/bolt-vcs/tests/redteam10*.rs`, Windows-only attacks on a `redteam/m6a-*` branch in CI, the report in `docs/redteam/<date>-milestone-6.md`. Each finding has a test that fails because of the bug.
- [ ] **Step 3:** Fix every proven finding, or rule on it in a decision record. A proof test runs on this machine only once its fix is in; before it is committed, clippy runs on it here.
- [ ] **Step 4:** Session 2 (RT11), on the fixes: `redteam/attacks10`, `redteam11*.rs`, `docs/redteam/<date>-milestone-6-session-2.md`. Fix or rule again.
- [ ] **Step 5:** Docker is shut down. security.md gains any new threat. Commit and push.

### Task 20: Final review, rulings, merge, and the draft release

- [ ] **Step 1:** Merge `main` in (the speed session's work, if it has merged). Read any follow-ups rows that session added for this one.
- [ ] **Step 2:** A whole-branch review by a fresh agent: every doc against the code; the `unsafe` count in `bolt-vcs-platform` unchanged; no attribution in any commit; no CR bytes; `.codex/` and `AGENTS.md` untouched.
- [ ] **Step 3:** Write `docs/plans/<date>-milestone-6-rulings.md`: the please-confirm list first (the license at its top, then milestone 5's list carried unchanged, then this milestone's calls), then every ruling verbatim, then what was deferred and where it went.
- [ ] **Step 4:** A pull request to `main`, with CI green on all three systems. If the speed session has merged first: rerun the whole benchmark and the speed gates on the combined code and publish the numbers in `benchmarks.md`; then merge with a merge commit and mark milestone 6 done in `milestones.md` and `design.md` through a small pull request. If it hasn't: merge with a merge commit and leave the rest of this step to that session.
- [ ] **Step 5:** Start the release workflow on the merged commit: four archives, `SHA256SUMS`, and a draft release `v0.1.0` with the notes. Nothing is published, tagged or signed.
