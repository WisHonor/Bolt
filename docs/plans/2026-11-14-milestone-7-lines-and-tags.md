# Milestone 7 ("Lines and tags"): Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task by task. Steps use checkbox (`- [ ]`) syntax for tracking. Every subagent runs on Opus.

**Goal:** `opalro line new`, `line rename` and `line delete` work, `opalro save --line` works from a line, `opalro tag` makes, lists, moves and deletes tags, a tag's name is taken wherever a save is asked for, and Opalro can tell which saves a line has that nothing else has, through every parent.

**Architecture:** Every new command is one diary step that changes only the state (lines, position, tags) and never the folder, so undo, redo, `undo --list`, `tidy` and crash recovery already cover it. One new engine module, `ancestry.rs`, owns every question about which save is in which save's history; `line delete` is its first user and `combine` (milestone 8) the next. Line operations live in `lines.rs`, tag operations in a new `tags.rs`, and both check names through the rules `diary.rs` already holds. Nothing new is stored on disk: format 1 holds all of it, and a project used with this build still opens in v0.1.

**Tech Stack:** Rust 1.99.1 and the crates already in the workspace. No new dependency.

**Spec:** decisions [157](../decisions/157-v0.2-in-four-milestones.md) to [160](../decisions/160-whether-a-save-is-in-anothers-history.md); [design.md](../design.md) §5.10 and §5.15; [storage-format.md](../storage-format.md) §9 (the diary entry, the tag list); [terminal.md](../terminal.md) §6, §10 and §12; [json.md](../json.md) §1 to §3. Each task that adds words a person sees writes them into terminal.md, json.md and the guide in the same commit.

**Rulings made while building:** `2026-11-14-milestone-7-rulings.md`, kept as the build goes, verbatim. Every ruling's premise is checked in the code before it is made.

**Branches:** tasks are built on `milestone-7`, made from `v0.2`. It is merged into `v0.2` after the red-team session (Task 14). `main` is never touched: it stays what v0.1 is released from.

## Global Constraints

- Everything in CLAUDE.md applies.
  - `#![forbid(unsafe_code)]` in the engine and the command-line crate. **No task adds `unsafe`.**
  - No `unwrap`, `expect` or `panic` outside tests.
  - One place per rule: line and tag name rules only in `diary.rs` (`validate_line_name`, `fold_line_name`) and the two checks built on them (`lines::check_new_name`, `tags.rs`); ancestry only in `ancestry.rs`; Git's words only in `git_words.rs`; colour, symbols and widths only in `style.rs`.
- **Nothing new on disk.** No task changes an encoding, a limit, `.opalro/format` or a cache's format. A task that finds it needs to stops and says so.
- **Written clean.** Nothing is written from another project's source (decision 131). The ancestry walk is written from this plan's description and decision 160, and checked from the outside against `git rev-list`.
- Code comments are short and human: the why in a line or two, no process words, no task or decision numbers in code.
- **Untrusted text is escaped exactly as today** (`opal_core::text::escape`, per piece, at the edge). A tag's name and message can be any text an import brought in: both are escaped wherever they are printed, in lists, outcome lines, questions, errors and hints.
- **`--json` never asks and never carries colour, symbols, progress or "Next:" lines.** Where a person would be asked, it stops and names the flag. `opalro_json` stays `1`: documents and fields are added, none is changed (decision 117).
- **Without a terminal, nothing is asked:** the command stops with an `error:` whose `hint:` names `--yes`, and changes nothing.
- **Each new command records one diary entry** whose command is exactly `line new`, `line rename`, `line delete` or `tag`, with no folder change. What was typed after the command (names, messages) is never recorded in the entry's command (storage-format.md §9).
- **Each new command finishes a stopped switch first,** right after taking the lock, exactly as `shelf` does, and passes on the warnings that finishing made.
- **Exit codes:** 0 done (also "nothing to do", and a question answered no), 1 failed or stopped, 2 a command line Opalro couldn't use.
- **Values:**

  | What | Value |
  |---|---|
  | A line's or a new tag's name | `diary::validate_line_name`: letters, digits, `-`, `_`, `.`, `/`; at most 255 bytes; no empty, `.` or `..` part |
  | Two names "alike" | equal after `diary::fold_line_name` (Unicode lowercase mapping) |
  | A tag's message | at most 64 KiB (`MAX_TAG_MESSAGE_BYTES`) |
  | Tags in a project | at most 100,000 (`MAX_TAGS`) |
  | A save named on screen | its short change ID with its message's first line in quotes, as `goto` names one; 12 hex digits where the save isn't read (`opalro tag`'s list) |
  | A fingerprint in JSON | 64 hex digits |
  | The saves a line has alone | `ancestry::only_in(tip, holders)`, where the holders are every other line's newest save, every tag's save and every server line's save |

- **Local runs on this machine:** `export PATH="$HOME/.cargo/bin:/c/Program Files/GitHub CLI:$PATH"` and `export OPALRO_CONFIG_DIR="$(mktemp -d)"` before any test (the older `BOLT_CONFIG_DIR` protects nothing). Build with `-j 2`, test with `-- --test-threads=2`, one test file at a time (`cargo test -j 2 -p opal-vcs --all-features --test lines -- --test-threads=2`; engine tests: `cargo test -j 2 -p opal-vcs-core --all-features <filter> -- --test-threads=2`). Run `cargo fmt` after every edit and `cargo clippy -j 2 --workspace --all-targets --all-features -- -D warnings` before every commit. Unix-only code can't be checked here: watch CI. Test files live in a temporary folder, never in the repository.
- Docs are edited with the Edit tool or `sed`, never Python in text mode; every edited doc is checked with `tr -cd '\r' < file | wc -c`, which must print 0.
- The untracked `.codex/` and `AGENTS.md` are not ours: never staged, edited or deleted.
- One commit per task plus fix commits, with no Claude attribution, on `milestone-7`. The controller pushes at task boundaries; CI runs on Windows, macOS and Linux, and a task is done only once they pass. `v0.2` comes in by merge, never by rebase.

## Review Focus

1. **A line step after a stopped switch.** A `goto` stopped half-way (a crash, or a file held open), then `opalro line new x` or `opalro tag v1`. Expected: the switch is finished first, a `switch_finished` warning says so, and the line or tag is made where the finished switch leaves the project. Test: Task 2 `a_line_step_after_a_stopped_goto_finishes_it_first`.
2. **A deleted line's saves and `tidy`.** `opalro line delete x --yes` on a line with saves of its own, then `opalro tidy`, then `opalro undo` twice. Expected: inside the undo window `tidy` frees none of them, the line comes back, and `opalro goto x` works. Test: Task 4 `a_deleted_lines_saves_survive_tidy_inside_the_undo_window`.
3. **A renamed line everywhere it was known by its old name.** After `opalro line rename main trunk`: `history`, `goto trunk`, a short change ID typed to `show`, `shelf --list` (which keeps the old name for an old shelf) and `undo`. Expected: all work at once, with no stale name acted on. Test: Task 4 `after_a_rename_history_goto_and_short_ids_still_work`.
4. **Hostile tag names and messages.** An imported tag named with an escape sequence (`ESC[31m`) and U+202E, with a message holding the same. Expected: `opalro tag`, the move and delete questions, `history`, `show` and every error print them only in escaped form, with colour on too. Tests: Task 7 `an_imported_tags_name_and_message_are_escaped`, Task 9's row in `project_text_is_escaped_with_colour_on`.
5. **A hundred thousand tags.** A project whose tag list holds 100,000 tags (an import's limit). Expected: `opalro tag`, `opalro goto <tag>`, `opalro history` and `opalro tag new-one` each end within the test's time limit, and memory stays near the list's own size. Test: Task 8 `a_hundred_thousand_tags_are_listed_and_searched_in_time`.
6. **A line only the place you stand holds.** The project at a save that is line `x`'s newest (not on the line), no other line or tag reaching it; `opalro line delete x`. Expected: the question is asked: where the project stands is not a holder. Test: Task 2 `standing_at_a_lines_newest_save_does_not_hold_it`.

---

## File structure

| File | Responsibility |
|---|---|
| `crates/opal-vcs-core/src/ancestry.rs` (new) | Which saves are in which save's history, through every parent |
| `crates/opal-vcs-core/src/lines.rs` | What `goto` takes (as today), plus making, renaming and deleting lines and the one check of a new line's name |
| `crates/opal-vcs-core/src/tags.rs` (new) | Making, moving and deleting tags; tags by save for `history` and `show` |
| `crates/opal-vcs-core/src/history.rs` | `resolve` learns tags |
| `crates/opal-vcs-core/src/snapshot.rs` | `save --line` from a line |
| `crates/opal-vcs/src/main.rs` | The `line` forms and `tag` on the command line, their questions |
| `crates/opal-vcs/src/line_output.rs`, `tag_output.rs` (new) | Their words for people |
| `crates/opal-vcs/src/output.rs` | What `undo` says about a step that made no save |
| `crates/opal-vcs/src/help.rs`, `git_words.rs` | Help for a command with forms; Git's words |
| `crates/opal-vcs/tests/lines.rs`, `tags.rs` (new), `undo.rs`, `git_words.rs`, `model.rs`, `json_formats.rs`, `cli.rs`, `terminal.rs`, `import_vs_git.rs` | Tests |

---

### Task 1: The ancestry walk

**Files:**
- Create: `crates/opal-vcs-core/src/ancestry.rs`
- Modify: `crates/opal-vcs-core/src/lib.rs` (add `pub mod ancestry;`)

**Interfaces:**
- Consumes: `repo.store().save(&id, repo.signatures_allowed())` to read a save (its `parents` and `saver.time_ms`).
- Produces:

```rust
/// How many saves `tip` has in its history, itself included, that no save of `others` has in
/// its own. 0 exactly when one of `others` is `tip` or has `tip` in its history.
pub fn only_in(repo: &Repository, tip: Id, others: &[Id]) -> Result<u64, RepoError>
```

**The walk** (decision 160). The signature and tests don't determine it, so here it is:

- A node is kept per save met: `marks` (bit 1 = reached from `tip`, bit 2 = reached from `others`), `spread` (the marks it had when it last gave them to its parents; 0 = never), and its saver time and parents, read once when the save is first met.
- `tip` starts with bit 1 and each of `others` with bit 2; a save that is both starts with both.
- **As soon as `tip` has bit 2, the answer is 0:** something else holds it, so it has nothing of its own. That is checked at the start and whenever a mark is given.
- A node is **waiting** when `marks != spread`: so a save that gains a mark after it spread waits again, and passes the new mark on. A max-heap of `(saver time, id)` holds the waiting nodes. Each turn pops the newest waiting node, whatever its marks, gives its marks to each parent, and sets its `spread = marks`.
- **The walk goes on while any of these holds:** some waiting node has exactly one mark; some waiting node has spread before (it carries a correction); or the newest waiting node is not earlier than the earliest of the saves counted so far (the nodes whose marks are exactly bit 1).
- The answer is the number of nodes whose marks are exactly bit 1.
- No recursion. A save that can't be read is the store's error, passed on.

Why shared saves spread too: the other side's mark can run on alone below the place the two sides met (through an older side line that was combined in, say); the shared mark, spreading in the same newest-first order, catches up with such a run. Why the early answer: every save between one of the others and a held `tip` is a descendant of `tip`, never gets `tip`'s mark, and so always passes the others' mark on; the mark therefore reaches a held `tip` whatever the saves' times are, and the commonest question (a line already combined into another) costs a handful of reads. Why the last clause: at the end every waiting save is shared, has never spread, and is strictly older than every counted save; when no save is older than its parent, such a save can't be a descendant of a counted one, so every save between the others and a counted save has passed on all it will ever have, and the count is exact, saves dated alike included (Git keeps whole seconds, and a rebased series shares one). When some save is older than its parent the count can be too high, never too low (decision 160).

(Reworded on 2026-11-14 during Task 1's review rounds: the first wording had no early answer and left a shared save that had never spread out of the walk for good; a second wording spread each save once, which a tie where two lines parted turns into a walk to the project's first save. The pinned tests in `ancestry.rs` hold each of these mistakes.)

- [ ] **Step 1: Write the failing tests** in `ancestry.rs` (`#[cfg(test)] mod tests`), building saves with chosen parents and saver times the way `history.rs`'s tests build theirs:

  - `a_line_another_holds_has_nothing_of_its_own`: a chain `a ← b ← c`. `only_in(b, [c]) == 0`, `only_in(c, [b]) == 1`, `only_in(c, []) == 3`, `only_in(c, [c]) == 0`.
  - `a_save_reached_only_through_a_second_parent_is_held`: `m` has parents `[x, y]`. `only_in(y, [m]) == 0`.
  - `both_sides_of_a_fork_are_counted_apart`: `base ← a1 ← a2` and `base ← b1`. `only_in(a2, [b1]) == 2`, `only_in(b1, [a2]) == 1`.
  - `shared_history_is_not_read`: 200 saves in a chain, then 3 more on one side and 2 on the other, each later than its parent. `only_in(side_a, [side_b]) == 3`, and the store's count of saves read (`saves_read`, the test counter beside `Store::save`) grew by at most 7.
  - `only_in_agrees_with_a_full_walk` (proptest, 256 cases): up to 40 saves, each with 0 to 3 parents among the earlier ones; `tip` and 0 to 3 `others` picked among them. The oracle collects every ancestor of each side from the parents table and counts the difference. With every save later than its parents: `only_in == oracle`. With times drawn at random: `(only_in == 0) == (oracle == 0)` and `only_in >= oracle`.

- [ ] **Step 2: Run them and see them fail**

Run: `cargo test -j 2 -p opal-vcs-core --all-features ancestry -- --test-threads=2`
Expected: does not compile (`only_in` not found).

- [ ] **Step 3: Implement `only_in`** as described above. A `BinaryHeap<(i64, Id)>` and a `HashMap<Id, Node>`; count the nodes that need looking at as their marks change rather than scanning the heap.

- [ ] **Step 4: Run the tests and see them pass**

Run: `cargo test -j 2 -p opal-vcs-core --all-features ancestry -- --test-threads=2`
Expected: 5 passed.

- [ ] **Step 5: Commit**

```bash
git add crates/opal-vcs-core/src/ancestry.rs crates/opal-vcs-core/src/lib.rs
git commit -m "Tell which saves a line has alone: one walk through every parent, newest first (decision 160)"
```

### Task 2: Lines made, renamed and deleted (engine)

**Files:**
- Modify: `crates/opal-vcs-core/src/lines.rs` (tests in `crates/opal-vcs-core/src/lines/tests.rs` if they outgrow the file)

**Interfaces:**
- Consumes: `ancestry::only_in` (Task 1); `diary::{validate_line_name, line_named_alike}`; `Repository::{current, commit, new_packs, tag_list, server_lines}`; the way `shelf` takes the lock, finishes a stopped switch and passes on its warnings.
- Produces:

```rust
pub struct Made { pub line: String, pub save: Id, pub warnings: Vec<Warning> }
pub struct Renamed { pub from: String, pub to: String, pub renamed: bool, pub warnings: Vec<Warning> }
pub struct Deleted { pub line: String, pub save: Id, pub only_here: u64, pub warnings: Vec<Warning> }

pub fn new_line(repo: &mut Repository, name: &str, who: &Person,
    on_wait: &mut dyn FnMut(&str)) -> Result<Made, LineError>
pub fn rename_line(repo: &mut Repository, old: Option<&str>, new: &str, who: &Person,
    on_wait: &mut dyn FnMut(&str)) -> Result<Renamed, LineError>
/// `confirm(line, only_here)` is called only when no other line, tag or server line holds
/// the line's newest save; a `false` keeps the line.
pub fn delete_line(repo: &mut Repository, name: &str, who: &Person,
    on_wait: &mut dyn FnMut(&str), confirm: &mut dyn FnMut(&str, u64) -> bool)
    -> Result<Deleted, LineError>

/// The one check of a new line's name. `renaming` is the line being renamed, left out of the
/// "alike" check so a line can take another spelling of its own name.
pub(crate) fn check_new_name(repo: &Repository, state: &State, name: &str,
    renaming: Option<&str>) -> Result<(), NameError>

pub enum NameError {
    Bad { name: String, reason: String },   // the words `SaveError::BadLineName` has today
    Taken { name: String, taken: String },  // the words `SaveError::LineTaken` has today
    Tag { name: String },
}
pub enum LineError {
    Name(NameError), NoSuchLine(String), NotOnALine, OnIt(String),
    Kept { line: String, only_here: u64 }, Repo(RepoError),
}
```

- Each error has `hint()`, like the engine's others. The words:

  | Error | Message | Hint |
  |---|---|---|
  | `NameError::Tag` | `"<name>" is a tag's name already, and a line can't share it.` | `Choose another name.` |
  | `NoSuchLine` | `There is no line named "<name>".` | `See the lines with: opalro line` |
  | `NotOnALine` | `You're at a save, not on a line, so there is no current line to rename.` | `Name the line too: opalro line rename <old> <new>` |
  | `OnIt` | `You're on line <line>, and Opalro doesn't delete the line you're on.` | `Go to another line first: opalro goto <line>` |
  | `Kept` | `Line <line> holds <N> saves that no other line or tag has, so it wasn't deleted.` (`1 save`) | `To delete it anyway: opalro line delete <line> --yes` |

- Behaviour, each under the change lock after finishing a stopped switch:
  - `new_line`: `check_new_name(.., None)`; the line points at the save the project is on (its line's newest, or the save it is at); the position becomes the new line. Entry command `line new`.
  - `rename_line`: `old` is the name given, or the line the project is on (`NotOnALine` at a save); unknown is `NoSuchLine`; `old == new` returns `renamed: false` and records nothing; else `check_new_name(.., Some(old))`, the name is swapped, and a position on the line follows it. Entry command `line rename`.
  - `delete_line`: unknown is `NoSuchLine`; the line the project is on is `OnIt`; `only_here = ancestry::only_in(tip, holders)` with the holders of Global Constraints; when it isn't 0 and `confirm` says no, `Kept`; else the name is removed. Entry command `line delete`.
  - No entry has a folder change. The caches need nothing: both heal themselves.

- [ ] **Step 1: Write the failing tests** (engine tests, as `shelf`'s and `undo`'s are written):

  - `a_new_line_starts_at_the_save_the_project_is_on_and_is_moved_onto`: on `main` at save `s`: afterwards `state.lines["wall-run"] == s`, the position is `OnLine("wall-run")`, the entry's command is `line new` and it has no folder change. The same at a save.
  - `a_new_lines_name_is_checked`: `wall run` is `Name(Bad)`; `Main` beside `main` is `Name(Taken)`; a tag's name is `Name(Tag)`; `state/head` is unchanged after each.
  - `a_rename_keeps_the_project_on_the_line`: on `main`, `rename_line(None, "trunk")`: `lines` holds `trunk` at the same save and no `main`; the position is `OnLine("trunk")`; command `line rename`.
  - `a_line_can_take_another_spelling_of_its_own_name`: `Main` to `main` works; `main` to `Other` beside a line `other` is `Name(Taken)`.
  - `renaming_to_the_same_name_makes_no_step`: `renamed == false`, head unchanged.
  - `renaming_the_current_line_at_a_save_is_refused`: `NotOnALine`.
  - `the_line_the_project_is_on_is_not_deleted`: `OnIt`.
  - `deleting_asks_only_when_nothing_else_holds_the_saves`: a line with 2 saves of its own: `confirm` is called with `("b", 2)`. Not called for: a line at `main`'s newest save; a line only a tag holds; a line `main` holds through a second parent (a save on `main` with parents `[main's, b's]`, built by hand).
  - `standing_at_a_lines_newest_save_does_not_hold_it`: the project at the save that is `x`'s newest: `confirm` is called.
  - `a_no_keeps_the_line`: `Kept { line, only_here: 2 }`, head unchanged.
  - `undo_takes_each_line_step_back`: after each of the three, one undo gives back the lines and the position as they were.
  - `a_line_step_after_a_stopped_goto_finishes_it_first` (`#[cfg(feature = "fault-injection")]`, as `a_goto_stopped_at_every_step_is_finished_or_never_happened` arms a fault): a `goto` stopped part-way, then `new_line`: its warnings hold `switch_finished`, and the new line points at the save the finished switch went to.

- [ ] **Step 2: Run them and see them fail**

Run: `cargo test -j 2 -p opal-vcs-core --all-features lines -- --test-threads=2`
Expected: does not compile (`new_line` not found).

- [ ] **Step 3: Implement** `check_new_name`, `new_line`, `rename_line`, `delete_line` and the two error types.

- [ ] **Step 4: Run the tests and see them pass**

Run: `cargo test -j 2 -p opal-vcs-core --all-features lines -- --test-threads=2`
Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git add crates/opal-vcs-core/src/lines.rs crates/opal-vcs-core/src/lines
git commit -m "Lines can be made, renamed and deleted: each a diary step that undo takes back (decision 158)"
```

### Task 3: `save --line` from a line

**Files:**
- Modify: `crates/opal-vcs-core/src/snapshot.rs` (`save_goes_on`, the errors `BadLineName`, `LineTaken`, `LineFromLine` and their hints)
- Modify: `crates/opal-vcs/src/main.rs` (`SaveArgs::line`'s help text; the mapping of the save errors), `crates/opal-vcs/src/help.rs` (the `save --line` example)
- Modify: `docs/guide.md` (`opalro save`), `docs/terminal.md` (`opalro save`: the "nothing to save" detail)
- Test: `crates/opal-vcs/tests/lines.rs`

**Interfaces:**
- Consumes: `lines::check_new_name`, `NameError` (Task 2).
- Produces: `SaveError::Name(NameError)`, which replaces `BadLineName` and `LineTaken` with the same words; `SaveError::LineFromLine` is gone.

- On a line, `--line <name>` checks the name with `check_new_name(.., None)`; the save's parent is the line's newest save; the new line points at the new save, the line the project was on stays where it was, and the project is on the new line. It is one diary entry, command `save`, as at a save.
- `--line`'s help: `The name of a new line this save starts.`; the example's text: `Start the new line wall-run with this save`.
- With nothing to save, no line is made (as today); the detail line now ends: `No line was made: opalro line new <name> makes one without saving.`

- [ ] **Step 1: Write the failing tests** in `tests/lines.rs`:

  - `save_line_on_a_line_starts_a_new_line_and_leaves_the_old_one_where_it_was`: on `main` with a changed file, `opalro save --line wall-run "Try wall running"` exits 0; `opalro line --json` lists `main` and `wall-run` with the position on `wall-run`; `main`'s newest save is what it was before.
  - `save_line_with_nothing_to_save_makes_no_line_and_names_line_new`: stdout holds `No line was made: opalro line new <name> makes one without saving.`
  - Replace the v0.1 test that pins "making a new line from a line comes in v0.2" with the first test above.

- [ ] **Step 2: Run them and see them fail**

Run: `cargo test -j 2 -p opal-vcs --all-features --test lines -- --test-threads=2`
Expected: the new test fails with the v0.1 refusal.

- [ ] **Step 3: Implement** the change in `save_goes_on` and the error types; update the help text, the example and the two docs.

- [ ] **Step 4: Run the tests and see them pass**

Run: `cargo test -j 2 -p opal-vcs --all-features --test lines -- --test-threads=2`, then `--test cli` and `--test guide`
Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git add -A crates docs
git commit -m "save --line starts a new line from a line too, leaving the line you were on where it was (decision 158)"
```

### Task 4: `opalro line new`, `rename` and `delete` on the command line

**Files:**
- Modify: `crates/opal-vcs/src/main.rs` (the `Line` command's forms, their handlers, the question)
- Create: `crates/opal-vcs/src/line_output.rs` (the words for people)
- Modify: `crates/opal-vcs/src/help.rs` (`page` lists a command's forms; `examples("line")`; the overview row)
- Modify: `docs/terminal.md` (§10's table, `opalro line`), `docs/json.md` (three sections), `docs/guide.md` (`opalro line`, §8 Lines)
- Test: `crates/opal-vcs/tests/lines.rs` (and its snapshots), `json_formats.rs`, `cli.rs`, `terminal.rs`

**Interfaces:**
- Consumes: `lines::{new_line, rename_line, delete_line, Made, Renamed, Deleted, LineError}` (Task 2).
- Produces: the command line below, three `--json` documents, and `impl From<LineError> for Failure`.

```rust
/// List the lines, marking the one you're on, then the server lines; or make, rename or
/// delete one.
Line { #[command(subcommand)] action: Option<LineAction> },

#[derive(Subcommand)]
enum LineAction {
    /// Make a new line at the save you're on, and move onto it.
    New { name: String },
    /// Rename a line: the one you're on, or the one named first.
    Rename { #[arg(value_name = "NAME", num_args = 1..=2, required = true)] names: Vec<String> },
    /// Delete a line. Its saves stay stored for the undo window.
    Delete {
        name: String,
        /// Delete without asking, even when no other line or tag has its saves.
        #[arg(long)] yes: bool,
    },
}
```

**What people see** (terminal.md, `opalro line`, after the list's rules):

| State | Outcome line | Details | Next step |
|---|---|---|---|
| A new line | `Started line <line> at save <id> "<message>".` | `You're on it: your next save goes on <line>.` | `Changed your mind? opalro undo` |
| Renamed | `Renamed line <old> to <new>.` | `Saves already made keep <old> as the line they were made on.` | `Changed your mind? opalro undo` |
| The same name | `Line <line> already has that name.` | | |
| Deleted | `Deleted line <line>, which was at save <id> "<message>".` | when nothing else had its saves: `No other line or tag has its 3 saves. opalro tidy frees them once the undo window has passed.` (`its 1 save`) | `Changed your mind? opalro undo` |
| The question, answered no | `Nothing was deleted.` | | |
| No terminal and no `--yes` | stdout gets nothing; `error: Line <line> holds 3 saves that no other line or tag has, and Opalro can't ask whether to delete it without a terminal.` (`… with --json.`) | `hint: To delete it: opalro line delete <line> --yes` | |

- The question, on stderr, as §10 asks one: `Line <line> holds 3 saves that no other line or tag has.` then `Delete it? [y/N] `. §10's table gains the row: a line no other line or tag holds, `Delete it? [y/N]`, `--yes`, "stops, deleting nothing".
- **Colour:** `Started`, `Renamed` and `Deleted` green and bold, line names blue, IDs magenta and bold.
- `opalro help line` shows the three forms under its usage, each with its one-line description; `examples("line")`: `opalro line` "List your lines", `opalro line new wall-run` "Start a new line where you are, and move onto it", `opalro line rename wall-run wall-running` "Rename a line", `opalro line delete wall-run` "Delete a line; undo brings it back". The overview's row: `List, make, rename and delete lines`.

**The documents** (json.md, each with `opalro_json` and `warnings`):

| Section | Fields |
|---|---|
| `opalro line new --json` | `line` (string), `save` (save: the save the new line starts at) |
| `opalro line rename --json` | `from`, `to` (strings), `renamed` (boolean: `false` when the line already had that name) |
| `opalro line delete --json` | `line` (string), `save` (save: the line's newest), `only_here` (number: the saves no other line or tag has) |

- [ ] **Step 1: Write the failing tests** in `tests/lines.rs`:

  - `line_new_starts_a_line_and_moves_onto_it`: stdout starts `Started line wall-run at save `; `line --json` gives the position `wall-run`.
  - `line_new_keeps_unsaved_changes`: a changed file stays changed in `status`, and the next `save` lands on the new line.
  - `line_rename_renames_and_says_what_stays`; `line_rename_with_one_name_renames_the_current_line`.
  - `line_delete_deletes_a_line_another_holds_without_asking`: a line at `main`'s newest save, deleted from `main` with no `--yes`: exit 0.
  - `line_delete_without_a_terminal_stops_and_names_yes`: a line with a save of its own: exit 1, stderr holds `opalro line delete x --yes`, and `line --json` still lists it. With `--json`: the document's `error`, exit 1.
  - `line_delete_asks_in_a_terminal` (`--all-features`, the faked terminal as `setup`'s question is tested): answer `n` gives `Nothing was deleted.` and exit 0; answer `y` deletes.
  - `line_delete_yes_deletes_and_undo_brings_it_back`.
  - `a_deleted_lines_saves_survive_tidy_inside_the_undo_window`: delete with `--yes`, `opalro tidy`, `opalro undo` twice (the tidy step, then the delete), `opalro goto x` exits 0.
  - `after_a_rename_history_goto_and_short_ids_still_work`: on `main` with two saves and a shelf: `line rename main trunk`; `history` lists both saves under `trunk`; `goto trunk` says already there; `show <short change ID>` finds the older save; `shelf --list` still says `on main`; `undo` gives `main` back.
  - `line_new_json_matches_its_snapshot`, `line_rename_json_matches_its_snapshot`, `line_delete_json_matches_its_snapshot` (masked as the others are).
  - `help_line_lists_its_forms`: `opalro help line` holds `opalro line new`, `opalro line rename` and `opalro line delete`.
  - Add the three documents to `json_formats.rs`'s `every_document`, `line`'s option to `cli.rs`'s `COMMANDS`, and the three forms to the commands `terminal.rs` runs without a terminal.

- [ ] **Step 2: Run them and see them fail**

Run: `cargo test -j 2 -p opal-vcs --all-features --test lines -- --test-threads=2`
Expected: the new tests fail (clap refuses `line new`).

- [ ] **Step 3: Write terminal.md, json.md and the guide** as above (the guide's `opalro line` section and §8 show each form in a runnable `sh` block).

- [ ] **Step 4: Implement** the forms, the handlers, `line_output.rs`, the question (`ask_delete_line`, as `ask_apply` asks), the help changes and `From<LineError> for Failure`.

- [ ] **Step 5: Run the tests and see them pass**

Run: the `lines`, `json_formats`, `cli`, `terminal` and `guide` test files, each as in Global Constraints, with `cargo insta` snapshots reviewed and accepted.
Expected: all pass.

- [ ] **Step 6: Commit**

```bash
git add -A crates docs
git commit -m "opalro line new, rename and delete: on the command line, in the guide, and in --json (decision 158)"
```

### Task 5: What undo says about a step that made no save

**Files:**
- Modify: `crates/opal-vcs/src/output.rs` (`step_line`)
- Modify: `docs/terminal.md` (`opalro undo` and `opalro redo`: the bullet on how a step is named), `docs/storage-format.md` §9 (the commands that record no folder change gain `line new`, `line rename`, `line delete` and `tag`)
- Test: `crates/opal-vcs/tests/undo.rs`

**Interfaces:**
- Consumes: the diary entries Tasks 2 and 3 record.
- Produces: `step_line` names "the save the step made", and adds `; its changes show as unsaved`, only for a step whose command is `save`.

- [ ] **Step 1: Write the failing tests** in `tests/undo.rs`:

  - `undoing_a_new_line_names_no_save`: `line new wall-run`, then `undo`: stdout is `Undid line new: line wall-run is gone, back on main at <id>.` (no `unsaved` in it); `redo`: `Redid line new: wall-run is at <id> again, on wall-run at <id> again.`
  - `undoing_a_rename_and_a_delete_say_what_came_back`: undo of a rename holds `main is back at` and `line trunk is gone`; undo of a delete is `Undid line delete: x is back at <id>.`
  - `undo_list_names_the_line_steps`: `undo --list` rows hold `line new`, `line rename` and `line delete`.
  - `undoing_a_save_still_says_its_changes_show_as_unsaved` (the existing words, pinned if no test does yet).

- [ ] **Step 2: Run them and see them fail**

Run: `cargo test -j 2 -p opal-vcs --all-features --test undo -- --test-threads=2`
Expected: the first fails: the line names a save and says its changes show as unsaved.

- [ ] **Step 3: Implement** the one condition in `step_line`, and edit the two docs.

- [ ] **Step 4: Run the tests and see them pass**

Run: `cargo test -j 2 -p opal-vcs --all-features --test undo -- --test-threads=2`
Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git add -A crates docs
git commit -m "Undo names a save, and says its changes show as unsaved, only for a step that made one"
```

### Task 6: Tags (engine)

**Files:**
- Create: `crates/opal-vcs-core/src/tags.rs`
- Modify: `crates/opal-vcs-core/src/lib.rs` (add `pub mod tags;`)

**Interfaces:**
- Consumes: `Repository::{current, tag_list, commit, new_packs}`; `diary::{Tag, TagList, validate_line_name, fold_line_name}`; `packs.add_record(ObjectKind::TagList, ..)`; the lock-and-finish way of Task 2.
- Produces:

```rust
/// What `tag` and `delete_tag` ask before they change a tag that exists.
pub enum Ask<'a> {
    Move { name: &'a str, from: Id, to: Id },
    Delete { name: &'a str, save: Id },
}
pub enum Tagged { Made { save: Id }, Already { save: Id }, Moved { from: Id, to: Id } }

/// Tags `save`, or the save the project is on. `confirm` is asked before a tag moves.
pub fn tag(repo: &mut Repository, name: &str, message: Option<&str>, save: Option<Id>,
    who: &Person, on_wait: &mut dyn FnMut(&str), confirm: &mut dyn FnMut(&Ask<'_>) -> bool)
    -> Result<(Tagged, Vec<Warning>), TagError>
/// Deletes a tag, once `confirm` says yes. Gives the save it was at.
pub fn delete_tag(repo: &mut Repository, name: &str, who: &Person,
    on_wait: &mut dyn FnMut(&str), confirm: &mut dyn FnMut(&Ask<'_>) -> bool)
    -> Result<(Id, Vec<Warning>), TagError>
/// The tags at each save, names in order, for `history` and `show`.
pub fn by_save(repo: &Repository, state: &State) -> Result<HashMap<Id, Vec<String>>, RepoError>

pub enum TagError {
    BadName { name: String, reason: String }, Alike { name: String, taken: String },
    Line { name: String }, NoSuchTag(String), MessageTooLong, TooMany,
    NotMoved { name: String, from: Id, to: Id }, NotDeleted { name: String },
    Repo(RepoError),
}
```

- The words:

  | Error | Message | Hint |
  |---|---|---|
  | `BadName` | `"<name>" can't be a tag's name: <reason>.` (the reasons `validate_line_name` gives) | `A tag's name takes letters, digits, - _ . and /.` |
  | `Alike` | `There is already a tag named "<taken>", and two tags can't differ only by letter case.` | `Choose another name.` |
  | `Line` | `"<name>" is a line's name already, and a tag can't share it.` | `Choose another name.` |
  | `NoSuchTag` | `There is no tag named "<name>".` | `See the tags with: opalro tag` |
  | `MessageTooLong` | `A tag's message can be at most 64 KiB.` | |
  | `TooMany` | `This project already has 100,000 tags, the most it can hold.` | |

- Rules (decision 159):
  - A name that is no tag yet: `validate_line_name`; no other tag equal after `fold_line_name`; not exactly a line's name. The tag holds the save, the message (empty when none) and no Git tag objects.
  - A name that is a tag (exact): at the same save, `Already`, and nothing is recorded, whatever message was given. At another save, `confirm(Move)`: a no is `NotMoved`; a yes moves it, keeps its message unless one was given, and drops its Git tag objects.
  - `delete_tag`: unknown is `NoSuchTag`; `confirm(Delete)`: a no is `NotDeleted`. When the last tag goes, the state has no tag list.
  - A tag an import brought in, with any name, can be moved and deleted: the name rules are for new names.
  - One diary entry, command `tag`, no folder change; the new tag list is stored in the step's packs.

- [ ] **Step 1: Write the failing tests** in `tags.rs`:

  - `a_tag_names_the_save_the_project_is_on`: `Made`; the list holds it with its message; the entry's command is `tag`; no folder change.
  - `a_tag_can_name_another_save`: `save: Some(older)`.
  - `a_new_tags_name_follows_the_line_rules`: `v 1` is `BadName`; `V1.0` beside `v1.0` is `Alike`; `main` is `Line`; head unchanged after each.
  - `tagging_the_same_save_again_is_nothing_to_do`: `Already`, head unchanged, even with a new message.
  - `moving_a_tag_asks_and_keeps_its_message`: `confirm` gets `Move { from, to }`; a yes gives `Moved`; the message stays when none is given and changes when one is; a tag planted with Git tag objects has none afterwards.
  - `a_no_leaves_the_tag_where_it_was`: `NotMoved`, head unchanged.
  - `deleting_asks_first_and_the_last_tag_leaves_no_list`: `confirm` gets `Delete`; afterwards `state.tags` is `None`.
  - `undo_brings_a_deleted_tag_back`.
  - `an_imported_tag_with_any_name_can_be_moved_and_deleted`: a tag named `weird name` planted in the list.
  - `by_save_gives_each_saves_tags_in_order`.

- [ ] **Step 2: Run them and see them fail**

Run: `cargo test -j 2 -p opal-vcs-core --all-features tags -- --test-threads=2`
Expected: does not compile.

- [ ] **Step 3: Implement** `tags.rs`.

- [ ] **Step 4: Run the tests and see them pass**

Run: `cargo test -j 2 -p opal-vcs-core --all-features tags -- --test-threads=2`
Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git add crates/opal-vcs-core/src/tags.rs crates/opal-vcs-core/src/lib.rs
git commit -m "Tags can be made, moved and deleted: each a diary step, moving and deleting only once asked (decision 159)"
```

### Task 7: `opalro tag` on the command line

**Files:**
- Modify: `crates/opal-vcs/src/main.rs` (the `Tag` command, its handler and questions)
- Create: `crates/opal-vcs/src/tag_output.rs`
- Modify: `crates/opal-vcs/src/help.rs` (the overview's row, `examples("tag")`)
- Modify: `docs/terminal.md` (§10's table, a new `opalro tag` section), `docs/json.md` (three sections), `docs/guide.md` (a new `opalro tag` section in §4, tags in §8)
- Test: `crates/opal-vcs/tests/tags.rs` (new, with snapshots), `lines.rs`, `json_formats.rs`, `cli.rs`, `terminal.rs`

**Interfaces:**
- Consumes: `tags::{tag, delete_tag, Ask, Tagged, TagError}` (Task 6); `history::resolve` for `--save`; `Repository::tag_list` for the list.
- Produces:

```rust
/// Name a save with a tag, list the tags, or delete one.
Tag {
    /// The tag's name. Without one, the tags are listed.
    name: Option<String>,
    /// What the tag marks, in a few words.
    message: Option<String>,
    /// The save to tag, by line name, tag, change ID or fingerprint; the one you're at if left out.
    #[arg(long, value_name = "SAVE", requires = "name")]
    save: Option<String>,
    /// Delete this tag.
    #[arg(long, value_name = "NAME", conflicts_with_all = ["name", "message", "save"])]
    delete: Option<String>,
    /// Move or delete the tag without asking.
    #[arg(long)]
    yes: bool,
},
```

**What people see** (terminal.md, a new `opalro tag` section after `opalro line`):

```text
3 tags:

  release/2026-11  4f9c2e81a7b3  "November build"
  v1.0             9d02c3aa41f0  "First playable"
  v1.1             1b7e55d0c9a2
```

- **The list:** `3 tags:` (`1 tag:`), then a row a tag in name order: two spaces, the name, the first 12 hex digits of its save's fingerprint, and its message's first line in quotes when it has one. No save is read. On a terminal the message is cut at its end to fit. With no tags: `No tags: opalro tag <name> names the save you're at.`

| State | Outcome line | Next step |
|---|---|---|
| Made | `Tagged save <id> "<message>" as <tag>.` | `Changed your mind? opalro undo` |
| Already there | `Tag <tag> is already at save <id> "<message>".` | |
| Moved | `Moved tag <tag> to save <id> "<message>". It was at save <old id>.` | `Changed your mind? opalro undo` |
| Deleted | `Deleted tag <tag>, which was at save <id> "<message>".` | `Changed your mind? opalro undo` |
| A question answered no | `Nothing was changed.` | |
| A move with no terminal and no `--yes` | stdout gets nothing; `error: Tag <tag> is at another save, <old id>, and Opalro can't ask whether to move it without a terminal.` (`… with --json.`); `hint: To move it, run the same command with --yes.` | |
| A delete with no terminal and no `--yes` | `error: Opalro asks before it deletes a tag, and can't without a terminal.` (`… with --json.`); `hint: To delete it: opalro tag --delete <tag> --yes` | |

- The questions, on stderr: `Tag <tag> is at save <old id> "<message>".` then `Move it to save <id> "<message>"? [y/N] `; and `Delete tag <tag>, at save <id> "<message>"? [y/N] `. §10's table gains both rows, each answered by `--yes`.
- **Colour:** `Tagged`, `Moved` and `Deleted` green and bold, tag names blue like line names, IDs magenta and bold, the list's fingerprints blue.
- The overview's row (under "Lines, shelves and details"): `tag` — `Name a save, like v1.0, or list your tags`. `examples("tag")`: `opalro tag` "List your tags", `opalro tag v1.0 "First playable"` "Tag the save you're at", `opalro tag v1.0 --save main` "Tag another save", `opalro tag --delete v1.0` "Delete a tag; undo brings it back".

**The documents** (json.md, each with `opalro_json` and `warnings`):

| Section | Fields |
|---|---|
| `opalro tag --json` | `tags` (list of `{"name": string, "save": 64 hex digits, "message": string}`, in name order) |
| `opalro tag <name> --json` | `tag` (string), `save` (save), `moved_from` (64 hex digits, or `null`), `changed` (boolean: `false` when the tag was already there) |
| `opalro tag --delete --json` | `tag` (string), `save` (save: where it was) |

- [ ] **Step 1: Write the failing tests** in `tests/tags.rs`:

  - `tag_names_the_save_you_are_at_and_the_list_shows_it`; `tag_takes_a_message`; `tag_save_tags_another_save` (by line name and by short change ID).
  - `tag_refuses_a_bad_name_and_a_lines_name`: exit 1 with the engine's words.
  - `tagging_the_same_save_twice_says_already`: exit 0, `is already at save`.
  - `moving_a_tag_without_a_terminal_stops_and_names_yes`; `tag_yes_moves_it`; `moving_a_tag_asks_in_a_terminal` (faked terminal; `n` gives `Nothing was changed.`).
  - `tag_delete_without_a_terminal_stops`; `tag_delete_yes_deletes_and_undo_brings_it_back`.
  - `no_tags_says_how_to_make_one`.
  - `an_imported_tags_name_and_message_are_escaped`: a Git repository with an annotated tag whose name holds U+202E and whose message holds `ESC[31m`, imported: `opalro tag`'s output holds neither raw (`assert_escaped`).
  - `tag_list_json_matches_its_snapshot`, `tag_json_matches_its_snapshot`, `tag_delete_json_matches_its_snapshot`.
  - In `tests/lines.rs`: `line_new_and_save_line_refuse_a_tags_name`.
  - Add the three documents to `every_document`, `tag` to `cli.rs`'s `COMMANDS` (its length grows by one), and `tag` to the commands `terminal.rs` runs.

- [ ] **Step 2: Run them and see them fail**

Run: `cargo test -j 2 -p opal-vcs --all-features --test tags -- --test-threads=2`
Expected: fails: `tag` is explained as a Git word.

- [ ] **Step 3: Write terminal.md, json.md and the guide** as above; the guide shows `opalro tag v1.0 "First playable"`, `opalro tag` and `opalro tag --delete v1.0 --yes` in runnable blocks.

- [ ] **Step 4: Implement** the command, `tag_output.rs`, the two questions, the help rows and `From<TagError> for Failure`. `git_words.rs` stops explaining `tag` here (its other words wait for Task 10): `tag` goes to Opalro's command as typed.

- [ ] **Step 5: Run the tests and see them pass**

Run: the `tags`, `lines`, `json_formats`, `cli`, `terminal`, `git_words` and `guide` test files.
Expected: all pass.

- [ ] **Step 6: Commit**

```bash
git add -A crates docs
git commit -m "opalro tag: make, list, move and delete tags, on the command line, in the guide, and in --json (decision 159)"
```

### Task 8: A tag's name where a save is asked for

**Files:**
- Modify: `crates/opal-vcs-core/src/history.rs` (`resolve`, a new `tag_named`, `ResolveError`), `crates/opal-vcs-core/src/lines.rs` (`resolve_target`)
- Modify: `crates/opal-vcs/src/main.rs` (the help text of `goto`, `show` and `changes --save`)
- Modify: `docs/guide.md` (`opalro goto`, `opalro show`), `docs/json.md` (`opalro show --json`'s first paragraph)
- Test: engine tests in `history.rs`; `crates/opal-vcs/tests/tags.rs`

**Interfaces:**
- Consumes: `Repository::tag_list`.
- Produces: `resolve` and `resolve_target` take, in this order: a line, a server line (`resolve_target` only), a tag, the start of a change ID, the start of a fingerprint. A tag gives `Target::Save(its save)`.

```rust
/// The save of the tag named `text`, if there is one. A name that is also the start of
/// another save's change ID or fingerprint is refused, as a line's is.
pub(crate) fn tag_named(repo: &Repository, entry: &DiaryEntry, text: &str)
    -> Result<Option<Id>, ResolveError>
// new variant:
ResolveError::TagOrSave { text: String, saves: Vec<String> }
```

- Words: `"<text>" is the name of a tag and also the start of another save's ID (<saves>), so Opalro didn't pick either.` Hint: `To reach the save, type more of its ID; opalro history --json gives the whole change ID and fingerprint. To reach the tag's save, take its ID from: opalro tag`
- `LineOrSave`'s hint changes its last sentence to: `To reach the line by another name: opalro line rename <old> <new>`
- In `resolve_target`, a text holding `@` that names no server line is no longer "not found" at once: it goes on to tags and IDs (an imported tag may hold `@`).
- Help: `goto`: `Go to a line, or to a save: by line name, line@server, tag, change ID or fingerprint.`; `show` and `changes --save`: `by line name, tag, change ID or fingerprint`.

- [ ] **Step 1: Write the failing tests:**

  - Engine: `a_line_wins_over_a_tag_of_the_same_name` (a state built by hand holding both); `a_tag_named_like_the_start_of_another_saves_id_is_refused` (a tag named with the first 8 hex digits of another save's fingerprint: `TagOrSave`); `a_tag_holding_an_at_sign_is_found_when_no_server_line_matches`.
  - `tests/tags.rs`: `goto_takes_a_tags_name_and_lands_at_its_save_not_on_a_line` (stdout holds `You are not on a line.`); `show_and_changes_save_take_a_tags_name`; `tag_save_takes_a_tags_name`.
  - `a_hundred_thousand_tags_are_listed_and_searched_in_time` (`run_limited`, 20 s each): a tag list of 100,000 tags planted through the support helpers; `opalro tag`, `opalro goto <one of them>`, `opalro history` and `opalro tag one-more` each exit 0 in time.

- [ ] **Step 2: Run them and see them fail**

Run: `cargo test -j 2 -p opal-vcs --all-features --test tags -- --test-threads=2`
Expected: `goto v1.0` fails with "No save, line or ID matches".

- [ ] **Step 3: Implement** `tag_named` and the new order, the texts and the docs.

- [ ] **Step 4: Run the tests and see them pass**

Run: the engine's `history` tests, then the `tags`, `goto`, `cli` and `guide` test files.
Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git add -A crates docs
git commit -m "A tag's name is taken where a save is asked for: after lines and server lines, before IDs (decision 159)"
```

### Task 9: `history` and `show` name the tags at each save

**Files:**
- Modify: `crates/opal-vcs/src/main.rs` (`history`, `show`), `crates/opal-vcs/src/history_output.rs`, `crates/opal-vcs/src/show_output.rs`
- Modify: `docs/terminal.md` (`opalro history`, `opalro show`), `docs/json.md` (both sections), `docs/guide.md` (§8)
- Test: `crates/opal-vcs/tests/tags.rs`, `terminal.rs`

**Interfaces:**
- Consumes: `tags::by_save` (Task 6), read once per command.
- Produces:
  - `history`: a row ends with `[tag <name>]` for each tag at its save, after the `[<line>]` marks, names in order. Its document gains `tags`: a list of `{"name": string, "save": 64 hex digits}` for every tag at a listed save, in name order.
  - `show`: a row `Tags         v1.0, release/2026-11` after `Line`, only when the save has tags. Its document gains `tags`: the names at the save shown, in order (an empty list when none).
  - **Colour:** tag names blue, like line names.

- [ ] **Step 1: Write the failing tests:**

  - `history_and_show_name_a_saves_tags`: after `opalro tag v1.0`: `history`'s first row ends `[main] [tag v1.0]`; `show` holds a `Tags` row with `v1.0`; both documents hold the tag.
  - `a_save_without_tags_shows_no_tags_row`.
  - In `terminal.rs`, `project_text_is_escaped_with_colour_on` gains `history` and `show` over a save with a hostile tag name.

- [ ] **Step 2: Run them and see them fail**

Run: `cargo test -j 2 -p opal-vcs --all-features --test tags -- --test-threads=2`
Expected: the row holds no `[tag v1.0]`.

- [ ] **Step 3: Implement**, and edit the three docs.

- [ ] **Step 4: Run the tests and see them pass**

Run: the `tags`, `terminal`, `json_formats`, `cli` and `guide` test files; snapshots of `history` and `show` reviewed.
Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git add -A crates docs
git commit -m "history and show name the tags at each save"
```

### Task 10: Git's words for lines and tags

**Files:**
- Modify: `crates/opal-vcs/src/git_words.rs`
- Modify: `docs/guide.md` (§9, "Git's words in Opalro")
- Test: `crates/opal-vcs/src/git_words.rs` (unit tests), `crates/opal-vcs/tests/git_words.rs`, `crates/opal-vcs/tests/cli.rs` (its list of Git words)

**Interfaces:**
- Consumes: the commands of Tasks 4 and 7.
- Produces (decisions 158 and 159):

  | Typed | Runs |
  |---|---|
  | `checkout -b <name>`, `switch -c <name>`, `switch --create <name>` (exactly those two arguments) | `line new <name>` |
  | `branch -m <new>`, `branch -m <old> <new>`, the same with `--move` | `line rename …` |
  | `branch -d <name>`, `branch --delete <name>` | `line delete <name>` |
  | `branch -D <name>` | `line delete <name> --yes` |
  | `tag -d <name>`, `tag --delete <name>` | `tag --delete <name>` |
  | `tag` in any other form | `tag`, as typed (the word is Opalro's too, so no hint) |

- Still explained, never run: `branch <name>` (Git makes the branch and stays; `line new` moves onto it), and `checkout -b` or `switch -c` with a starting point. Their text (`NEW_LINE`) becomes: `` `opalro line new <name>` makes a new line where you are and moves onto it.\nTo start it somewhere else, go there first: `opalro goto <save>` ``
- `LATER` becomes: `` Combining lines and seeing who changed each line arrive later in v0.2.\nUntil then, `opalro history` lists the saves on your line. `` It no longer covers `tag`.
- The first-use hint stays one fixed text per Git word; `branch`'s and `checkout`'s don't change.

- [ ] **Step 1: Write the failing tests:** `checkout_b_and_switch_c_start_a_line`, `checkout_b_with_a_starting_point_is_explained`, `branch_m_renames`, `branch_d_deletes_and_capital_d_says_yes`, `branch_with_a_name_is_still_explained`, `tag_d_deletes`, `tag_goes_to_opalro_as_typed`; the explained words' list loses `tag`.

- [ ] **Step 2: Run them and see them fail**

Run: `cargo test -j 2 -p opal-vcs --all-features --test git_words -- --test-threads=2`
Expected: `checkout -b x` is explained, not run.

- [ ] **Step 3: Implement** in `translate`, and update the guide's table.

- [ ] **Step 4: Run the tests and see them pass**

Run: `cargo test -j 2 -p opal-vcs --all-features git_words -- --test-threads=2`, then the `git_words`, `cli` and `guide` test files.
Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git add -A crates docs
git commit -m "Git's words for making, renaming and deleting a line, and for deleting a tag"
```

### Task 11: The model test learns lines and tags

**Files:**
- Modify: `crates/opal-vcs/tests/model.rs`
- Modify: `docs/testing.md` (the model test's list of actions)

**Interfaces:**
- Consumes: `lines::{new_line, rename_line, delete_line}`, `tags::{tag, delete_tag}`, `save --line` from a line (Tasks 2, 3 and 6).
- Produces: new actions `LineNew`, `LineRename`, `LineDelete { yes }`, `SaveOnNewLine`, `Tag { move_it }` and `TagDelete { yes }`, each with a weight of the size `Shelf`'s has. The model's state gains its tags (name to save). The model expects: a refused name (taken, or the other kind's); `OnIt` for the line the project is on; `Kept` when the model's own walk over its saves finds no holder and `yes` is false; and after every step the diary's lines, position and tags equal the model's, through undo, redo and `undo --to` as well.

- [ ] **Step 1: Add the actions and the model's expectations; run the model test**

Run: `PROPTEST_CASES=64 cargo test -j 2 -p opal-vcs --all-features --test model -- --test-threads=2`
Expected: passes. A failure is a difference between the engine and the model: find which is wrong before changing either.

- [ ] **Step 2: Run it longer once**

Run: `PROPTEST_CASES=512 cargo test -j 2 -p opal-vcs --all-features --release --test model -- --test-threads=2`
Expected: passes.

- [ ] **Step 3: Update testing.md and commit**

```bash
git add crates/opal-vcs/tests/model.rs docs/testing.md
git commit -m "The model test makes, renames and deletes lines and tags, and undoes them"
```

### Task 12: The walk against Git, on real histories

**Files:**
- Modify: `crates/opal-vcs/tests/support/vs_git.rs`, `crates/opal-vcs/tests/import_vs_git.rs`
- Modify: `docs/testing.md` (the differential tests)

**Interfaces:**
- Consumes: `opal_core::ancestry::only_in` (Task 1).
- Produces: `assert_ancestry_matches_git(dir: &Path) -> usize` in `vs_git.rs`: for the imported lines whose names came in unchanged, in name order, up to 64 ordered pairs `(a, b)`: `only_in(newest of a, [newest of b])` equals what `git rev-list --count a ^b` prints. It returns the pairs compared, and prints the pairs and the time the walks took (for benchmarks.md).

- [ ] **Step 1: Write the check and call it** from each repository `import_vs_git.rs` imports on every push, and from the ignored test that imports Git's own repository.

- [ ] **Step 2: Run the per-push tests**

Run: `cargo test -j 2 -p opal-vcs --all-features --test import_vs_git -- --test-threads=2`
Expected: passes, with at least one repository comparing more than one pair (a repository with a merge among them; add a branch-and-merge to a generated one if none has).

- [ ] **Step 3: Update testing.md and commit**

```bash
git add -A crates docs/testing.md
git commit -m "The ancestry walk is compared with git rev-list on every imported repository"
```

### Task 13: The documents

**Files:**
- Modify: `docs/design.md` (§5.15's "Built so far", the phase line), `docs/milestones.md` (milestone 7's steps ticked), `docs/testing.md` (the new test files), `docs/security.md` (below), `docs/guide.md` (§13, what Opalro doesn't do yet), `README.md` if it lists commands, `CLAUDE.md` (the phase line)
- Modify: `docs/benchmarks.md` (the ancestry walk's cost on Git's own history, from the nightly run of Task 12's check; the controller supplies the numbers)

- security.md gains, in the house form: a planted history can make an ancestry walk long, never wrong, and `line delete`'s count can only be too high (decision 160); names and messages of imported tags are shown escaped everywhere (Review Focus 4); a line or tag name typed on the command line is checked before anything is stored.
- The guide's "doesn't do yet" list loses making, renaming and deleting lines, and tags; it keeps combining and `who`.

- [ ] **Step 1: Edit the documents; check each with `tr -cd '\r' < file | wc -c`**
- [ ] **Step 2: Run the `guide` test file** (links, commands, "nothing points into the repository")
- [ ] **Step 3: Commit**

```bash
git add -A docs README.md CLAUDE.md
git commit -m "Milestone 7's documents: what is built, how it is tested, and what an attacker could try"
```

### Task 14: Red team (the controller's)

- [ ] Write `redteam/attack-surface-m7.md`: the new commands, the names they take, the ancestry walk, the tag list at its limits, undo across the new steps.
- [ ] One session from the no-network sandbox (decision 024), attack code in `redteam/attacks16`, report in `docs/redteam/`. Windows and macOS through a `redteam/` CI branch.
- [ ] Every proven finding fixed, with its proof in `crates/opal-vcs/tests/redteam17.rs`, or ruled on in a decision record. A second session on the fixes when a finding changed engine code.

### Task 15: Review, rulings and the merge into `v0.2` (the controller's)

- [ ] A whole-branch review by a fresh reviewer; its findings fixed or ruled on.
- [ ] `docs/plans/2026-11-14-milestone-7-rulings.md`: the please-confirm list first, then every ruling as made.
- [ ] The nightly workflow run once on the branch (model test at 1,024 cases, the fuzz targets, Git's own repository through Task 12's check); benchmarks.md gets the walk's numbers.
- [ ] CI green on Windows, macOS and Linux; `milestone-7` merged into `v0.2`; milestones.md and CLAUDE.md say milestone 7 is done.
