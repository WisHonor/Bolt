# Milestone 4 ("Undo everything") Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The diary drives `undo`, `redo`, `undo --list` and `undo --to`; every new command survives a crash and a lost `head` can be picked up again; `tidy` frees what nothing needs; `shelf`, `unshelf` and `save --amend` work; on Windows, Windows and macOS and Linux alike.

**Architecture:** Every step records at most one change to the folder, as a switch between two saves; what `discard` and `shelf` throw away becomes a hidden "kept" save. Undo and redo work out the done and undone steps by walking back from `head`, put back a whole state, and make the step's switch backwards or forwards through `goto`'s planner and journal, so every rule for unsaved work, names and links applies unchanged. `tidy` marks what lines, shelves, the diary and the undo window reach and rewrites only the packs that hold anything else.

**Tech Stack:** Rust 1.99.1 and the crates already in the workspace. No new dependency.

**Spec:** decisions [070](../decisions/070-undo-and-redo-step-through-the-diary.md) to [077](../decisions/077-tidy-and-the-undo-window.md), with 003, 009, 010, 017, 021, 048 to 051, 054 to 057, 062 and 065; [design.md](../design.md) §5.1, §5.7, §5.8, §5.9, §6 and §7; [storage-format.md](../storage-format.md) §9 and §11; [security.md](../security.md) T26 to T29; the milestone 4 rows of [the follow-ups](2026-11-01-milestone-1-follow-ups.md).

**Rulings made while building:** `2026-11-06-milestone-4-rulings.md` (copied from the progress log before the milestone is merged).

## Global Constraints

- Everything in CLAUDE.md applies.
  - `#![forbid(unsafe_code)]` in the engine. **No task adds `unsafe`.** Task 9 changes only the options passed through the platform crate's existing safe functions; the number of `unsafe` blocks in `bolt-vcs-platform` must be the same before and after it (a step checks). If any task finds it needs new `unsafe`, it stops and asks: the milestone's pull request then stays open for the owner's line-by-line review, with a walkthrough written as `docs/plans/2026-11-05-milestone-3-unsafe-walkthrough.md` is.
  - No `unwrap`, `expect` or `panic` outside tests. One place per rule. Spec before code: storage-format.md §9 and §11 already say the bytes; the diary's test vectors change in Task 2. Tests pass on Windows, macOS and Linux.
- Code comments are short and human: the why in a line or two, no process words, no task or decision numbers in code.
- Modules that parse bytes from disk keep `#![deny(clippy::indexing_slicing, clippy::arithmetic_side_effects)]`; nothing recurses once per folder level or once per diary entry (walks are loops with explicit stacks).
- **Every read and write inside the project folder goes through `bolt_platform::Dir`** (decision 048), and every write through `goto`'s planner and write-out (`writeout::plan`, `Journal`, `writeout::carry_out`). No new code writes into the project any other way.
- Untrusted text is escaped before printing (`text::escape`): paths, messages, line names, shelf messages, entries' commands.
- Messages say what happened, why, and what to do next (decision 017); no questions without a terminal. `--json` documents follow `docs/json.md`, which each task that adds or changes a document updates in the same commit.
- Every new changing command takes the lock with `Repository::lock_for_change` and finishes a stopped switch first (`writeout::finish_interrupted`), as `switch::locked` does.
- **Values** (from the decisions):

  | What | Value |
  |---|---|
  | A folder change | two different save IDs, `from` then `to` (071) |
  | Shelf numbers | the smallest whole number from 1 no shelf has, in decimal; newest first in the state (073) |
  | Default shelf message | `Shelved`; a shelf `--shelve` makes: `Set aside by undo` or `Set aside by redo` (070, 073) |
  | A kept save's message | `discard` or `shelf`, then the paths given, one space apart, cut to 1 KiB; a shelf's own message when given (071) |
  | `undo --list` | at most 20 undo steps, then "and more"; entry IDs shown as 12 hex digits (072) |
  | `undo --to` | at least 8 hex digits, matching exactly one entry from `head` back (072) |
  | `undo N`, `redo N` | N from 1; one entry per step (070) |
  | The undo window | `undo-window-days`, a whole number from 1 to 36,500, 30 when unset, at the top of the user's own `config.toml` (077) |
  | Tidy's grace period | packs modified less than 24 hours before `tidy` starts are left untouched (077) |
  | A small pack (as for merging) | under 64 MiB on disk, its index recording at most 256 MiB uncompressed (057, 062) |

- **New warning kinds** (each in json.md §4 when its task adds it): `links_left` (Task 1), `head_not_flushed` (Task 8), `pack_not_tidied` (Task 10), `pack_unreadable` (Task 11).
- **The diary records the command's name** (`save`, `goto`, `discard`, `undo`, `redo`, `shelf`, `unshelf`, `tidy` is not recorded, `save` for an amend), as milestone 1 does; descriptions in lists and messages come from what the step changed.
- **Dependencies:** none new. Any other needs the owner's approval.
- **Local runs on this machine:** `export PATH="$HOME/.cargo/bin:$PATH"`, `export BOLT_CONFIG_DIR="$(mktemp -d)"`, build with `-j 2`, test with `-- --test-threads=2`. Run `cargo fmt` after every edit. Unix-only code can't be clippy-checked here: watch CI for Unix-only lints.
- Push at task boundaries to `milestone-4`; CI runs on Windows, macOS and Linux; the next task starts only once they pass. One commit per task plus fix commits, with no Claude attribution.

## Review Focus

1. **Undo after editing again.** `bolt discard a.txt`, edit `a.txt` again, `bolt undo`. Expected: the undo stops before changing anything, naming `a.txt`; the new edit is intact. Test: Task 4 `undo_of_a_discard_stops_when_the_file_was_edited_again`.
2. **A crash in the middle of an undo.** Power cut while undoing a `goto` that rewrites many files. Expected: every file wholly old or new; the next changing command finishes the undo and says so. Test: Task 4 `an_undo_stopped_at_every_write_is_finished_by_the_next_command`.
3. **Unshelving after the line moved on.** `bolt shelf`, then a save that changes a file the shelf changed, then `bolt unshelf`. Expected: nothing changes, the file is named, the shelf stays. Test: Task 6 `unshelf_onto_a_save_that_changed_a_shelved_file_stops`.
4. **`tidy` with undone saves inside and outside the window.** Expected: what an entry within the window needs stays and undo still works; what only older entries need goes; shelves always stay. Test: Task 10 `tidy_keeps_what_the_window_reaches_and_removes_older`.
5. **Shelving a 300 MiB file.** Expected: streamed into the store; memory stays far below the file's size. Test: Task 6 `shelving_a_big_file_uses_little_memory` (Linux, peak memory through `/proc`).

---

## File structure

```text
crates/bolt-vcs-platform/src/dir/windows.rs   write-through renames, removals and new folders; read-only files on FAT (Task 9)
crates/bolt-vcs-core/src/
  kept.rs         what discard and shelf throw away, kept as a save (Task 1)
  diary.rs        the folder change replaces the safety-copy list (Task 2)
  timeline.rs     the done and undone steps, from the diary (Task 3)
  undo.rs         undo, redo, what a step changed (Task 4); the list and undo --to (Task 5); a lost head (Task 8)
  shelf.rs        shelf, unshelf, the shelf list (Task 6)
  switch.rs       one switch for every step: action, state, folder change (Tasks 1, 2, 4)
  writeout.rs     safety copies feed the kept save (Task 1)
  snapshot.rs     save --amend (Task 7)
  repo.rs, fsio.rs   a lost head; a failed flush after head moves (Task 8)
  tidy.rs         bolt tidy (Task 10); merge.rs shares its copying
  config.rs       undo-window-days (Task 10)
  store.rs, changeids.rs, merge.rs   store follow-ups (Task 11)
  snapshot/walk.rs, snapshot/endings.rs   walk follow-ups (Task 12)
crates/bolt-vcs/src/main.rs, output.rs   undo, redo, shelf, unshelf, tidy, save --amend (Tasks 4 to 10); Stdout for every command (Task 13)
crates/bolt-vcs/tests/
  undo.rs, shelf.rs, amend.rs, tidy.rs, recovery.rs   end to end
  corpus_diary.rs   hostile diaries (Task 14)
  model.rs          undo, redo, shelves and amend against the model (Task 15)
crates/bolt-vcs-core/tests/vectors/diary.json   the `full` entry (Task 2)
```

---

### Task 1: The two saves of a switch, and what `discard` throws away kept as a save

Decision 071. The format doesn't change yet: `discard` still lists its safety copies, and also works out its folder change.

**Files:**
- Create: `crates/bolt-vcs-core/src/kept.rs` (with its tests).
- Modify: `crates/bolt-vcs-core/src/lib.rs` (`mod kept`), `diary.rs` (the `FolderChange` type, not yet encoded), `switch.rs` (`throw_away` works out its folder change and leaves links it couldn't write back; `DiscardOutcome` gains `folder_change`), `writeout.rs` (a helper telling whether a link could be written back as a link).

**Interfaces:**
- Consumes: `writeout::plan`, `writeout::keep_safety_copies` (stores what the store lacks; returns the copies kept, by path), `journal::{Action, Change, Expected}`, `listing::{Listing, Entry, EntryKind}`, `PackSet::add_record`, `ChangeId::random`.
- Produces:
```rust
// diary.rs
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FolderChange { pub from: Id, pub to: Id }
// kept.rs
/// Who makes a kept save, and what it says.
pub(crate) struct KeptFrom<'a> {
    pub base: Id,                 // the save the folder is on; every kept save's only parent
    pub message: String,
    pub who: &'a Person,
    pub line: Option<&'a str>,    // its `line` field, when made on a line
}
/// The folder change of a switch planned as `actions` on a folder on `from.base`: the save
/// holding what the folder held at each path the actions write, and the one holding what they
/// write there, both `from.base` elsewhere. Held contents the store lacks are stored first
/// (`keep_safety_copies`); a path whose content changed after the plan is left out of both. Each
/// side is `from.base` or `target` when its folders are exactly that save's, otherwise a kept save
/// stored in `packs`. `None` when the two sides are the same.
pub(crate) fn switch_saves(repo: &Repository, plan: &Plan, from: Id /* the plan's own from save */, target: Id, kept: &KeptFrom, drive: Drive, packs: &mut PackSet) -> Result<(Option<FolderChange>, usize /* safety copies kept */), SaveError>;
/// The tree of `base_root` with each path in `changes` set to that entry (`None`: removed), its
/// new folder listings stored in `packs`; returns the new root listing's ID. A folder entry's
/// listing is built from the changes below it, on the base's listing there if the base has a
/// folder there. Loops, never recursion per level.
pub(crate) fn overlay(store: &Store, base_root: Id, changes: &BTreeMap<String, Option<(EntryKind, Id, u64)>>, packs: &mut PackSet) -> Result<Id, SaveError>;
// writeout.rs
/// The paths of the unsaved links `actions` would throw away that write-out could only bring
/// back as placeholders: judged with `link_shape` against the tree the kept save would have,
/// again until no more are left; whether links can be made here is probed once in `.bolt/tmp`.
pub(crate) fn links_to_leave(repo: &Repository, actions: &[Action], base: Id, drive: Drive) -> Result<Vec<String>, SaveError>;
// switch.rs
pub struct DiscardOutcome { /* as today */ pub folder_change: Option<FolderChange>, pub links_left: Vec<String> }
```
- `discard` drops from its plan every action that would throw away a link `links_to_leave` names, and the removal of each folder above one; a warning `links_left` names them (add the kind to `warning.rs` and json.md §4).

- [ ] **Step 1: Write the failing tests** (in `kept.rs`; projects made with `Repository::init` in a temp folder, discards through `switch::discard`, gotos through `switch::goto`):
```rust
#[test] fn the_kept_save_holds_what_discard_threw_away() {
    // saved: a.txt, b.txt, dir/c.txt. Folder: a.txt changed, b.txt deleted, new.txt added,
    // newdir/x.txt and the empty folder newdir/y added (Unix: also a file's executable bit flipped).
    // discard everything. folder_change.to == the saved save; folder_change.from: parent the saved
    // save, message "discard", saver who, tree exactly {a.txt (changed bytes), dir/c.txt, new.txt,
    // newdir/x.txt, newdir/y/} with no b.txt; every content in it reads back byte for byte.
}
#[test] fn a_partial_discard_keeps_only_what_it_names() { /* discard a.txt: from = saved tree with a.txt changed; message "discard a.txt" */ }
#[test] fn a_kind_change_is_kept_both_ways() { /* saved file f, folder f/x on disk; saved folder g/z, file g on disk */ }
#[test] fn a_file_changed_after_the_plan_is_left_out_of_both_sides() {
    // change a.txt again at the act point "discard:planned": both sides hold the saved a.txt; discard leaves the file (changed_since_plan)
}
#[test] fn nothing_to_discard_keeps_nothing() { /* folder_change == None, nothing stored */ }
#[test] fn a_plain_gotos_two_sides_are_the_two_saves() { /* switch_saves of goto's plan: from == the left save, to == the target; no new save stored */ }
#[test] fn work_that_matched_the_target_is_left_out_of_the_switch() {
    // decision 065: a.txt changed to the target's bytes before the goto; `to` is a kept save with the left save's a.txt
}
#[cfg(unix)] #[test] fn discard_leaves_a_link_it_could_not_write_back() {
    // new link `out -> /tmp` and newdir/inner -> /etc: both stay, newdir stays, a `links_left` warning names both; the rest is discarded
}
proptest! { #[test] fn overlay_sets_exactly_the_paths_given(base in tree(), changes in changes()) {
    // read the result back as a path -> entry map; equals base with changes applied (and what lay below a removed or replaced folder gone)
} }
```
- [ ] **Step 2:** Run `cargo test -p bolt-vcs-core kept -j 2 -- --test-threads=2`. Expected: compile errors (`kept` doesn't exist).
- [ ] **Step 3:** Implement `overlay`, `switch_saves` and `links_to_leave`; in `switch::throw_away`, after planning, drop the links that can't come back, then call `switch_saves` (target = the current save; `KeptFrom` message per the values table, `line` from the position) with the same `packs`, before the entry is stored. A folder's `Expected` has a zero ID: its listing comes from the changes below it.
- [ ] **Step 4:** Run the tests above and `cargo test -p bolt-vcs-core switch -j 2 -- --test-threads=2`. Expected: PASS.
- [ ] **Step 5:** `cargo fmt`, `cargo clippy --workspace --all-targets --all-features -j 2 -- -D warnings`. Commit: `Work out what a switch changes as two saves, and keep what discard throws away`.

### Task 2: The diary records the folder change

Decision 071 (format); storage-format.md §9 already says the bytes.

**Files:**
- Modify: `crates/bolt-vcs-core/src/diary.rs` (format), `switch.rs` (goto and discard record their switch), `writeout.rs` (`SafetyCopy` moves here from `diary.rs`; it is no longer part of the format), every place that builds a `DiaryEntry` (`repo.rs`, `snapshot.rs`, `import.rs`, `test_support.rs`, tests), `crates/bolt-vcs-core/tests/vectors/diary.json` and `tests/vectors.rs`.

**Interfaces:**
- Consumes: `FolderChange`, `switch_saves` (Task 1).
- Produces:
```rust
pub struct DiaryEntry {
    pub parent: Option<Id>, pub action: Action, pub command: String, pub who: Person,
    pub bolt_version: String, pub state: State,
    pub folder_change: Option<FolderChange>,   // replaces safety_copies
}
```
- `goto` records `switch_saves` of its plan (target = the save it goes to); a goto that writes nothing (only the position changes) records none. `discard` records its folder change from Task 1. `DiscardOutcome.safety_copies` still counts the files kept.

- [ ] **Step 1: Write the failing tests** (`diary.rs` and `vectors.rs`):
```rust
#[test] fn a_folder_change_round_trips() { /* Some(FolderChange{from: a, to: b}) encodes and decodes to itself */ }
#[test] fn a_folder_change_from_a_save_to_itself_is_refused() { /* decode -> Err(Invalid("a folder change from a save to itself")) */ }
#[test] fn an_entry_without_a_folder_change_has_milestone_1s_bytes() { /* the `minimal` vector's hex is unchanged */ }
#[test] fn goto_records_the_save_it_left_and_the_one_it_went_to() {}
#[test] fn discard_records_its_kept_save_then_the_current_one() {}
```
- [ ] **Step 2:** Run `cargo test -p bolt-vcs-core diary -- --test-threads=2`. Expected: compile errors.
- [ ] **Step 3:** Change the encoder and decoder (a `bool`, then two IDs; `from == to` refused). Update the `full` vector (it is an undo entry with two safety copies; it now records a folder change from 32 bytes of `0x5a` to 32 bytes of `0x6b`), its hex and ID regenerated by the encoder and checked by hand against §9; `minimal` stays byte for byte. `fuzz/fuzz_targets/decode_diary.rs` keeps building.
- [ ] **Step 4:** Run `cargo test --workspace -j 2 -- --test-threads=2`. Expected: PASS.
- [ ] **Step 5:** Commit: `Record what each step did to the folder as a switch between two saves`.

### Task 3: The done and undone steps

Decision 070 (the two lists).

**Files:**
- Create: `crates/bolt-vcs-core/src/timeline.rs` (with tests); modify `lib.rs`.

**Interfaces:**
- Produces:
```rust
pub(crate) type Read<'a> = dyn FnMut(&Id) -> Result<DiaryEntry, RepoError> + 'a;
/// The steps at `head`: done, newest first (at most `limit`, and whether there are more), and
/// undone, the one redo takes first first.
pub(crate) struct Steps { pub done: Vec<(Id, DiaryEntry)>, pub more_done: bool, pub undone: Vec<(Id, DiaryEntry)> }
pub(crate) fn steps(head: Id, read: &mut Read, limit: usize) -> Result<Steps, RepoError>;
/// The newest done step: what `undo` takes. `None` past the first entry.
pub(crate) fn newest_done(head: Id, read: &mut Read) -> Result<Option<(Id, DiaryEntry)>, RepoError>;
```
- Walking back: a command entry is done and its parent comes next; a redo's target is done and the target's parent comes next; an undo's target's parent comes next. The undone list counts redo entries walking back from `head` and stops at the first command entry. A target that isn't a command entry is damage (`RepoError::Damaged`, naming the entry).

- [ ] **Step 1: Write the failing tests:**
```rust
proptest! { #[test] fn the_lists_match_a_replay_of_the_diary(ops in vec(op(), 1..60)) {
    // Replay ops (Command, Undo when something is done, Redo when something is undone) on two
    // stacks, and build the matching diary entries in a HashMap. After each op, `steps(head, read, 1000)`
    // equals the stacks: done newest first, undone next-redo first.
} }
#[test] fn the_first_entry_is_done_and_nothing_comes_before_it() {}
#[test] fn a_target_that_is_not_a_command_is_damage() {}
#[test] fn the_done_list_stops_at_the_limit_and_says_there_is_more() {}
```
- [ ] **Step 2:** Run `cargo test -p bolt-vcs-core timeline`. Expected: compile errors.
- [ ] **Step 3:** Implement `steps` and `newest_done` as loops (no recursion).
- [ ] **Step 4:** Run the tests. Expected: PASS.
- [ ] **Step 5:** Commit: `Work out the done and undone steps from the diary`.

### Task 4: `bolt undo` and `bolt redo`

Decisions 070 and 071.

**Files:**
- Create: `crates/bolt-vcs-core/src/undo.rs`; `crates/bolt-vcs/tests/undo.rs`.
- Modify: `switch.rs` (`Switch` carries the entry's action and folder change; one `switch` makes every step that changes files), `lib.rs`, `crates/bolt-vcs/src/main.rs`, `output.rs`, `docs/json.md`.

**Interfaces:**
- Consumes: `timeline::{newest_done, steps}`, `writeout::plan`, `switch::locked`.
- Produces:
```rust
/// What one step taken did.
pub struct Taken {
    pub step: Id,                 // the command entry undone or redone
    pub entry: DiaryEntry,        // that step's own entry
    pub before: State, pub after: State,
    pub effects: Vec<Effect>,     // what changed, for messages and lists
    pub applied: Applied,         // what the switch wrote and removed; empty when it changed no file
}
pub enum Effect {
    Line { name: String, from: Option<Id>, to: Option<Id> },   // made, moved or gone
    Position { from: Position, to: Position },
    ShelfAdded { label: String, save: Id }, ShelfGone { label: String, save: Id },
    Tags, ServerLines,
    Files { from: Id, to: Id },
}
/// What changed between two states and through a folder change.
pub fn effects(before: &State, after: &State, folder: Option<FolderChange>) -> Vec<Effect>;
pub struct UndoOutcome {
    pub taken: Vec<Taken>, pub asked: usize,
    pub finished: bool,                   // a stopped switch was finished, so no step was taken
    pub shelved: Option<(String, Id)>,    // the shelf `--shelve` made, if any
}
pub struct UndoError { pub taken: Vec<Taken>, pub error: SwitchError }   // steps taken before the one that stopped it
pub struct StepRequest<'a> { pub n: usize, pub shelve: bool, pub who: &'a Person, pub git_excludes: Option<&'a Path> }
pub fn undo(repo: &mut Repository, req: &StepRequest, on_wait: &mut dyn FnMut(&str)) -> Result<UndoOutcome, UndoError>;
pub fn redo(repo: &mut Repository, req: &StepRequest, on_wait: &mut dyn FnMut(&str)) -> Result<UndoOutcome, UndoError>;
// switch.rs
pub enum SwitchError { /* ... */
    InTheWayOfStep { paths: Vec<String>, redo: bool },   // hint: `bolt undo --shelve` (or redo), and why a separate save, shelf or discard won't do
    StepGone { step: Id, detail: String },               // its saves can't be read: removed by tidy, or lost
}
/// `locked` also says whether it finished a stopped switch.
pub(crate) fn locked<T>(repo: &mut Repository, command: &str, on_wait: &mut dyn FnMut(&str), f: impl FnOnce(&mut Repository, &ChangeLock, &mut Earlier, bool /* finished */) -> Result<T, SwitchError>) -> Result<T, SwitchError>;
```
- **Taking one step**, under the lock (once for all N): if the lock's run finished a stopped switch, take none: `finished: true` (decision 075). Else take `newest_done` (undo) or the first undone (redo). The new entry's lines, position, tags and server lines are the step's parent's (undo) or the step's own (redo); its shelves are the current ones, except that a step that added a shelf (compare its state's shelves with its parent's, by save) has it taken off on undo and put back on redo, and a step that removed one the opposite; a shelf put back keeps its label unless taken, else the smallest free number. With a folder change `{from, to}`, undo plans `writeout::plan(repo, to, from, None, false, …)` and redo `plan(repo, from, to, …)`; collisions are `InTheWayOfStep`. The entry (`Action::Undo(step)` or `Redo(step)`, command `undo` or `redo`, its folder change from `switch_saves` of the plan) goes through `switch`. Without a folder change, the entry is committed with `repo.commit`. A step with no parent (the first entry) ends the undo: `taken` holds what was taken, and that is success.
- **`--shelve`:** the collisions that are unsaved work (not ignored files, not links `links_to_leave` names: those still stop it) are set aside first, in the same step: a kept save on the current save holding the folder's version of those paths (`switch_saves` of `plan(repo, current, current, Some(paths), true, …)`'s actions, its `from` side) becomes a new shelf, first, with the smallest free number and the message "Set aside by undo" (or redo). The step's switch is then planned again from `overlay(from-side, the folder's entries at those paths)`, stored as a kept save, so those paths are overwritten; one entry, one journal.
- CLI: `bolt undo [N] [--shelve]` and `bolt redo [N] [--shelve]` (`N` from 1, clap's range). Text: one line per step taken, from its effects ("Undid save kxqm "Add double jump": main is back at ptvz; its changes show as unsaved"; "Undid goto: back on main at ptvz, 3 files written"; "Undid discard: 2 files back, 1 removed"), the shelf `--shelve` made, then "Nothing (more) to undo/redo" where it ran out; after finishing a stopped switch, what was finished and that running it again takes one more. Exit 0 unless a step failed. JSON (json.md): `{bolt_json, undone|redone: [step], finished, shelved, position, warnings}` with `step = {entry, command, time_ms, effects, written, removed}`; a failure is an error document that still lists `undone`/`redone`.

- [ ] **Step 1: Write the failing tests** (`crates/bolt-vcs/tests/undo.rs`, through the `bolt` binary with `support::Env`):
```rust
#[test] fn undo_of_a_save_moves_the_line_back_and_keeps_the_files() { /* status then lists the save's changes as unsaved */ }
#[test] fn redo_of_a_save_moves_it_forward_again() {}
#[test] fn undo_of_a_goto_switches_back_carrying_unsaved_work() {}
#[test] fn undo_of_a_discard_brings_back_changed_new_and_deleted_files_and_folders() {}
#[test] fn redo_of_a_discard_throws_them_away_again() {}
#[test] fn undo_of_a_discard_stops_when_the_file_was_edited_again() { /* exit 1; names a.txt; a.txt keeps the new edit; head unchanged; the hint names --shelve */ }
#[test] fn undo_shelve_sets_the_work_in_the_way_aside_and_undoes_in_one_step() {
    // goto X, edit a file X wrote, `bolt undo --shelve`: back on the left save; a new shelf holds the edit;
    // `bolt undo --list` shows the goto as undone (redo brings it back), and no shelf step; redo, then unshelf, brings the edit back
}
#[test] fn undo_and_redo_keep_shelves_but_their_own() { /* a shelf made by --shelve survives redo and a further undo (engine test with planted entries until Task 6) */ }
#[test] fn running_undo_again_after_a_stop_finishes_it_and_takes_no_further_step() { /* files in use or BOLT_FAULT_FAIL; then `bolt undo`: finished, head is the stopped undo's entry */ }
#[test] fn undo_with_nothing_to_undo_says_so_and_succeeds() {}
#[test] fn undo_three_takes_three_steps_one_entry_each() {}
#[test] fn undo_two_stops_at_the_first_it_cannot_take_and_says_how_many_were_taken() {}
#[test] fn a_new_command_clears_what_redo_could_bring_back() {}
#[test] fn an_undo_stopped_at_every_write_is_finished_by_the_next_command() {
    // fault-injection: BOLT_FAULT_AT=n for n = 1.. until the undo completes; after each, `bolt status` says the
    // switch is unfinished or done, the next changing command finishes it, and every file is wholly old or new
}
#[cfg(windows)] #[test] fn an_undo_with_a_file_held_open_is_finished_by_the_next_command() {}
#[test] fn undo_json_lists_the_steps_taken() {}
```
- [ ] **Step 2:** Run `cargo test -p bolt-vcs --test undo -j 2 -- --test-threads=2`. Expected: compile errors (`undo` isn't a command).
- [ ] **Step 3:** Implement `undo.rs`, the `Switch` change in `switch.rs`, the CLI and json.md.
- [ ] **Step 4:** Run the tests, then `cargo test --workspace -j 2 -- --test-threads=2`. Expected: PASS.
- [ ] **Step 5:** Commit: `Undo and redo steps, putting back what each did to the folder`.

### Task 5: `bolt undo --list`, `--all` and `--to`

Decision 072.

**Files:**
- Modify: `undo.rs`, `main.rs`, `output.rs`, `docs/json.md`; tests in `crates/bolt-vcs/tests/undo.rs`.

**Interfaces:**
- Consumes: `timeline::steps`, `undo::{effects, undo, redo}`.
- Produces:
```rust
pub struct StepInfo { pub entry: Id, pub diary: DiaryEntry, pub effects: Vec<Effect> }
pub struct UndoList { pub redo: Vec<StepInfo>, pub undo: Vec<StepInfo>, pub more: bool }   // redo: next first; undo: newest first
pub fn undo_list(repo: &Repository, limit: usize) -> Result<UndoList, RepoError>;          // limit 20 from the CLI
/// Every entry from `head` back, newest first.
pub fn whole_diary(repo: &Repository, each: &mut dyn FnMut(Id, &DiaryEntry, &[Effect]) -> Result<(), RepoError>) -> Result<(), RepoError>;
pub fn undo_to(repo: &mut Repository, entry: &str, req: &StepRequest, on_wait: &mut dyn FnMut(&str)) -> Result<UndoOutcome, UndoError>;   // req.n is unused
```
- `undo_to`: the text must be at least 8 hex digits matching exactly one entry from `head` back (else an error naming the matches, or saying none matched). On the redo list: redo up to and including it; on the undo list: undo the steps newer than it, one at a time; the current entry: nothing. Any other entry: one new step, `Action::Command`, command `undo`, with that entry's lines, position, tags and server lines and the current shelves, and the folder change `switch_saves` gives for `plan(repo, current save, that state's save, None, false, …)`, if any. Unsaved work in the way stops it, and `--shelve` sets it aside, as for `undo`.
- CLI: `bolt undo --list [--all]`, `bolt undo --to <entry> [--shelve]`; `--list` and `--to` exclude `N` and each other. Text per decision 072: redo steps marked `undone` and numbered for `redo N`, then undo steps numbered from 1, each with `entry[..12]`, relative time, the command's name and its effects; the diary's first entry says it can't be undone; "and more: bolt undo --list --all shows the whole diary". JSON documents in json.md.

- [ ] **Step 1: Write the failing tests:**
```rust
#[test] fn the_list_shows_redo_steps_then_undo_steps_numbered_from_now() {}
#[test] fn the_list_shows_at_most_20_undo_steps_and_says_when_there_are_more() {}
#[test] fn undo_to_a_step_on_the_list_undoes_one_at_a_time_and_redo_still_works() {}
#[test] fn undo_to_a_cleared_step_is_one_new_step_that_undo_takes_back() {}
#[test] fn undo_to_a_cleared_step_keeps_the_shelves() {}
#[test] fn undo_to_the_current_entry_changes_nothing() {}
#[test] fn undo_to_needs_at_least_8_hex_digits_and_exactly_one_match() {}
#[test] fn undo_list_all_shows_every_entry_with_undos_and_redos() {}
#[test] fn undo_list_json_has_both_lists() {}
```
- [ ] **Step 2:** Run them. Expected: FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run `cargo test -p bolt-vcs --test undo -j 2 -- --test-threads=2`. Expected: PASS.
- [ ] **Step 5:** Commit: `List the steps undo and redo can take, and go to any entry`.

### Task 6: Shelves

Decision 073.

**Files:**
- Create: `crates/bolt-vcs-core/src/shelf.rs`; `crates/bolt-vcs/tests/shelf.rs`.
- Modify: `switch.rs` (discard's kept-save path shared with shelf; `SwitchError::Collisions`' hint names `bolt shelf`), `lib.rs`, `main.rs`, `output.rs`, `docs/json.md`.

**Interfaces:**
- Consumes: `kept::switch_saves`, `writeout::{plan, links_to_leave}`, `switch::locked` and `switch` (Task 4), `diff::count_changes`.
- Produces:
```rust
pub struct ShelfOutcome { pub label: String, pub save: Id, pub files: usize, pub applied: Applied, pub links_left: Vec<String> }
pub fn shelf(repo: &mut Repository, message: Option<&str>, who: &Person, git_excludes: Option<&Path>, on_wait: &mut dyn FnMut(&str)) -> Result<Option<ShelfOutcome>, SwitchError>;   // None: nothing unsaved
pub struct UnshelfOutcome { pub label: String, pub save: Id, pub applied: Applied, pub carried: usize, pub finished: bool /* a stopped switch was finished instead */ }
pub fn unshelf(repo: &mut Repository, label: Option<&str>, who: &Person, git_excludes: Option<&Path>, on_wait: &mut dyn FnMut(&str)) -> Result<UnshelfOutcome, SwitchError>;
pub struct ShelfInfo { pub label: String, pub save: Id, pub message: String, pub line: Option<String>, pub saver: Person, pub files: usize }
pub fn shelves(repo: &Repository) -> Result<Vec<ShelfInfo>, RepoError>;   // newest first
pub enum SwitchError { /* ... */ NoShelf { label: Option<String> }, ShelfInTheWay { paths: Vec<String>, label: String } }
```
- `shelf`: as `discard` with no paths (the same plan, links left as Task 1 leaves them, and its folder change: `{ from: the kept save, to: the current save }`, message per the values table); the state's shelves gain `(label, kept save)` first; command `shelf`.
- `unshelf`: if the lock's run finished a stopped switch, bring nothing back (`finished: true`). Else the shelf's base is its save's first parent; `plan(repo, base, shelf, None, false, …)`; collisions are `ShelfInTheWay` (hint: save, shelve or discard that work, then `bolt unshelf <its number>`, since a new shelf would be the newest; where your save changed them, go to the save you shelved on and unshelf there); the state's shelves lose it; the folder change is `switch_saves` of the plan on the current save (from the current save to the current save with what it wrote); command `unshelf`.
- CLI: `bolt shelf [message]`, `bolt shelf --list`, `bolt unshelf [number]`. JSON documents in json.md.

- [ ] **Step 1: Write the failing tests** (`crates/bolt-vcs/tests/shelf.rs`):
```rust
#[test] fn shelf_sets_every_unsaved_change_aside_and_cleans_the_folder() { /* changed, new, deleted, new folder; an ignored file stays */ }
#[test] fn unshelf_brings_the_newest_back_and_drops_it() {}
#[test] fn unshelf_by_number() {}
#[test] fn numbers_take_the_smallest_free() { /* 1, 2, unshelf 1, shelf -> 1 */ }
#[test] fn unshelf_onto_another_save_applies_where_nothing_collides() {}
#[test] fn unshelf_onto_a_save_that_changed_a_shelved_file_stops() { /* exit 1, names it, nothing changed, shelf kept */ }
#[test] fn unsaved_work_in_the_way_stops_unshelf() {}
#[test] fn shelf_with_nothing_unsaved_says_so() {}
#[test] fn undo_of_shelf_brings_the_changes_back_and_drops_the_shelf() {}
#[test] fn undo_of_unshelf_takes_them_out_and_puts_the_shelf_back() {}
#[test] fn undo_of_an_unshelf_onto_another_save_touches_only_what_it_wrote() {
    // the current save already holds the shelf's version of one file and has a saved file inside a folder the shelf adds:
    // undo leaves both alone, and status shows no change afterwards
}
#[test] fn running_unshelf_again_after_a_stop_finishes_it_and_brings_back_no_other_shelf() {}
#[test] fn undo_redo_and_jumps_never_drop_a_shelf() {}
#[test] fn shelf_list_shows_number_message_line_time_and_files() {}
#[test] fn gotos_hint_names_shelf() {}
#[cfg(target_os = "linux")] #[test] fn shelving_a_big_file_uses_little_memory() { /* 300 MiB file; peak RSS well under 100 MiB, as writeout_memory.rs measures */ }
#[test] fn a_shelf_or_unshelf_stopped_at_every_write_is_finished_by_the_next_command() { /* fault sweep as in undo.rs */ }
```
- [ ] **Step 2:** Run `cargo test -p bolt-vcs --test shelf -j 2 -- --test-threads=2`. Expected: compile errors.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run the tests and the workspace. Expected: PASS.
- [ ] **Step 5:** Commit: `Set unsaved changes aside on numbered shelves and bring them back`.

### Task 7: `bolt save --amend`

Decision 074.

**Files:**
- Modify: `crates/bolt-vcs-core/src/snapshot.rs` (`SaveRequest::amend`, `SaveOutcome::Saved::replaced`, two errors), `changeids.rs` (a line's IDs replaced whole), `main.rs` (`--amend`; the message may be left out with it), `output.rs`, `docs/json.md`; create `crates/bolt-vcs/tests/amend.rs`.

**Interfaces:**
- Produces:
```rust
pub struct SaveRequest<'a> { /* ... */ pub amend: bool, pub message: Option<&'a str> /* None: the old message, only with amend */ }
pub enum SaveOutcome { Saved { /* ... */ replaced: Option<Id> }, NothingChanged { .. } }
pub enum SaveError { /* ... */ AmendAtSave, AmendShared { by: Vec<String> /* "line x", "server line y@origin", "tag v1" */ } }
```
- The walk's base is the line's newest save `S`. The new save: `S`'s parents, author, change ID and move notes; saver `who`; the new message or `S`'s; `S`'s extra fields without `git-commit`, `git-header` and `git-original-message`. Nothing to amend when the root and message are `S`'s. Refused before the walk: at a save (`AmendAtSave`); when another line, a server line or a tag names `S`, or `S`'s change ID is in another line's first-parent change IDs (`ChangeIds::line`) (`AmendShared`). `--line` with `--amend` is refused. The diary's command is `save`; no folder change.

- [ ] **Step 1: Write the failing tests** (`crates/bolt-vcs/tests/amend.rs`):
```rust
#[test] fn amend_replaces_the_newest_save_keeping_change_id_parents_and_author() {}
#[test] fn amend_with_only_a_message_rewords() {}
#[test] fn amend_with_paths_takes_only_those() {}
#[test] fn amend_with_nothing_new_says_so() {}
#[test] fn amend_at_a_save_is_refused() {}
#[test] fn amend_of_a_save_another_line_holds_is_refused() { /* goto ptvz; save --line b; goto main; amend -> refused naming line b */ }
#[test] fn amend_of_a_save_a_server_line_or_tag_names_is_refused() { /* import a repo where main == origin/main */ }
#[test] fn amend_drops_git_fields() {}
#[test] fn undo_of_an_amend_puts_the_old_save_back_and_keeps_the_files() {}
#[test] fn amend_json_says_what_it_replaced() {}
#[test] fn an_amend_stopped_at_every_write_leaves_the_old_save_on_the_line() { /* fault sweep: before head moves nothing changed */ }
```
- [ ] **Step 2:** Run them. Expected: FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run `cargo test -p bolt-vcs --test amend -j 2 -- --test-threads=2` and the workspace. Expected: PASS.
- [ ] **Step 5:** Commit: `Amend the newest save of a line, keeping its change ID`.

### Task 8: A lost `head`, and failures after `head` moves

Decision 075; the follow-up "a committed save can still exit 1 when flushing `state/` after the head rename fails".

**Files:**
- Modify: `crates/bolt-vcs-core/src/repo.rs` (`RepoError::HeadLost`; `move_head_to` warns instead of failing after the rename), `fsio.rs` (a rename that happened but wasn't flushed is told apart), `warning.rs` (`HEAD_NOT_FLUSHED`), `undo.rs` (the list and `--to` without a usable head), `main.rs`, `docs/json.md`; create `crates/bolt-vcs/tests/recovery.rs`.

**Interfaces:**
- Produces:
```rust
pub enum RepoError { /* ... */ HeadLost { detail: String } }   // hint: the saves are safe; bolt undo --list, then bolt undo --to <entry>
pub(crate) enum Renamed { Flushed, NotFlushed(io::Error) }
pub fn rename_reported(from_dir: &Dir, from: &OsStr, to_dir: &Dir, to: &OsStr) -> io::Result<Renamed>;   // rename_durable stays for every other caller
/// The diary entries in the store that no other entry has as its parent, newest first by `who` time.
pub fn diary_ends(repo: &Repository) -> Result<Vec<(Id, DiaryEntry)>, RepoError>;
```
- `head` is lost when `state/head` is missing, isn't an entry's ID and a newline, or names an entry the store doesn't hold or can't decode. Every command meets it through `Repository::current`, and stops with `HeadLost`. `bolt undo --list` then prints `diary_ends`; `bolt undo --to <entry>` accepts any entry the store holds (8+ hex digits, one match among all diary entries in the packs), commits a new step (`Action::Command`, command `undo`, parent the chosen entry, its state, no folder change) and writes `head`. `lock_for_change` must not fail on a lost head for these two.
- After the rename of `head`, a failed flush becomes a `head_not_flushed` warning in the store's notes; the command succeeds. A fault point `head:renamed` sits between the rename and the flush.

- [ ] **Step 1: Write the failing tests** (`crates/bolt-vcs/tests/recovery.rs`):
```rust
#[test] fn a_missing_head_stops_every_command_saying_the_saves_are_safe() { /* status, history, save, goto, undo */ }
#[test] fn a_garbled_head_and_a_head_naming_a_missing_entry_are_lost_too() {}
#[test] fn undo_list_with_a_lost_head_lists_the_entries_no_other_follows_newest_first() {}
#[test] fn undo_to_with_a_lost_head_makes_that_entry_current_without_touching_files() {}
#[test] fn a_failed_flush_after_head_moves_is_a_warning_and_the_save_stands() { /* BOLT_FAULT_FAIL=head:renamed */ }
```
- [ ] **Step 2:** Run them. Expected: FAIL.
- [ ] **Step 3:** Implement. `diary_ends` lists diary entries from the pack indexes (`PackReader::entries`, kind 20) without reading other objects.
- [ ] **Step 4:** Run `cargo test -p bolt-vcs --test recovery -j 2 -- --test-threads=2` and the workspace. Expected: PASS.
- [ ] **Step 5:** Commit: `Say when head is lost and pick it up from the diary, and never fail a command once head moved`.

### Task 9: Renames, removals and new folders reach the disk on Windows

Decision 076; the follow-ups "Windows rename durability" and "Windows renames: clearing the read-only flag…".

**Files:**
- Modify: `crates/bolt-vcs-platform/src/dir/windows.rs`, `crates/bolt-vcs-core/src/fsio.rs` (the `sync_dir` comment; `create_dir_durable` goes), `crates/bolt-vcs-core/src/repo.rs` (`begin` makes `.bolt` and its fixed folders through folder handles, after the shape check: the follow-up "`begin` makes `.bolt` and its fixed folders by path before the shape check").

**Interfaces:** none new. `rename_into` opens its source, `remove_file` and `remove_dir` their entry, with `FILE_WRITE_THROUGH` in the `NtCreateFile` create options; `create_dir` makes the folder with it. Where the plain rename or delete (the FAT fallback) is refused because the file there is read-only, its `FILE_ATTRIBUTE_READONLY` is cleared through a handle opened with `FILE_WRITE_ATTRIBUTES` (`set_info(FileBasicInfo)`), and the call is made again. `sync` stays empty on Windows with a comment saying each change was written through. `begin` opens the project folder with `Dir::open_top`, makes `.bolt` with `Dir::create_dir` (refusing what is already there unless it is the unfinished project's own, as today), then each fixed folder inside it the same way, flushing each parent (`Dir::sync`).

- [ ] **Step 1:** Count `unsafe` in the platform crate: `grep -c "unsafe" crates/bolt-vcs-platform/src/dir/windows.rs crates/bolt-vcs-platform/src/lib.rs`. Note the numbers.
- [ ] **Step 2: Write the failing tests** (`dir.rs` tests, Windows):
```rust
#[cfg(windows)] #[test] fn a_read_only_file_is_replaced_on_the_plain_rename() { /* a test-only switch skips the POSIX rename */ }
#[cfg(windows)] #[test] fn a_read_only_file_is_removed_on_the_plain_delete() {}
#[cfg(windows)] #[test] fn renames_removals_and_new_folders_still_work_written_through() {}
#[test] fn start_never_makes_a_fixed_folder_through_a_link() { /* repo.rs: a link planted at .bolt/state before begin makes it is refused, and nothing is made where it points */ }
```
- [ ] **Step 3:** Run `cargo test -p bolt-vcs-platform -- --test-threads=2`. Expected: the read-only tests FAIL.
- [ ] **Step 4:** Implement through the existing safe functions (`nt_open`, `open_entry`, `set_info`).
- [ ] **Step 5:** Run the platform and core tests. Count `unsafe` again: the numbers must match Step 1.
- [ ] **Step 6:** Commit: `Write renames, removals and new folders through to the disk on Windows`.

### Task 10: `bolt tidy` and the undo window

Decision 077; storage-format.md §11 "Tidying"; the follow-ups "merge.rs drops the damage notes…" (tidy rewrites version 1 packs) and "Names and limits" (Task 12 has the rest).

**Files:**
- Create: `crates/bolt-vcs-core/src/tidy.rs`; `crates/bolt-vcs/tests/tidy.rs`.
- Modify: `merge.rs` (`copy_pack` takes a filter and is shared), `config.rs` (`undo-window-days`), `warning.rs` (`PACK_NOT_TIDIED`), `lib.rs`, `main.rs`, `output.rs`, `docs/json.md`.

**Interfaces:**
- Produces:
```rust
pub fn undo_window_days() -> Result<u32, ConfigError>;   // config.rs: 30 when unset; 1..=36_500 else an error naming the file
pub struct TidyReport {
    pub to_free: u64,                  // the estimate, before anything was removed
    pub bytes_before: u64, pub bytes_after: u64,   // the packs' sizes on disk
    pub packs_removed: usize, pub packs_written: usize, pub packs_young: usize,
}
pub fn tidy(repo: &mut Repository, window_days: u32, now_ms: i64, on_wait: &mut dyn FnMut(&str), on_estimate: &mut dyn FnMut(u64)) -> Result<TidyReport, RepoError>;
/// What must be kept: every object reached (fingerprints).
pub(crate) fn kept(repo: &Repository, window_days: u32, now_ms: i64) -> Result<HashSet<Id>, RepoError>;
```
- `kept` walks with explicit stacks: the diary from `head` back (every entry kept); the state of the current entry; for every entry whose `who.time_ms >= now_ms - window_days * 86_400_000`: its state, its parent's state and its folder change, and for an undo or redo entry also its target's state, the target's parent's state and the target's folder change; from a state: each line's save, the position's save, each shelf's save, the tag list (its record and each tag's save) and the server lines (their record and each line's save); from a save: its record, every parent, its root listing; from a listing: each entry's object by kind (file, executable, link: the content; folder: the listing; project link: the record; a conflict, which v0.1 never writes, stops `tidy` as something it can't follow yet). Every record is read and checked; content is marked, not read. Anything that can't be read stops `tidy` (`RepoError::Damaged`, naming it) before anything is removed.
- Then each `.pack` in `store/packs`: modified within 24 hours of the start → untouched; else by its index: nothing kept → removed unread; only kept, version 2 and not small → stays; otherwise its kept objects are copied (`merge::copy_pack` with the filter) into new packs. New packs finish (durable, renamed) before any old pack is removed, never one with a new pack's name. A pack that can't be read is left, with a `pack_not_tidied` warning. Fault points `tidy:marked`, `tidy:written`, `tidy:removing`. `tidy` makes no diary entry.
- CLI: `bolt tidy`: in a terminal, "Tidying: about X to free" before removing; then "Freed X (N packs removed, M written)". JSON in json.md.

- [ ] **Step 1: Write the failing tests** (`tidy.rs` unit tests with planted stores; `crates/bolt-vcs/tests/tidy.rs` end to end):
```rust
#[test] fn tidy_keeps_every_line_tag_server_line_and_shelf_with_all_their_history() {}
#[test] fn tidy_keeps_what_the_window_reaches_and_removes_older() {
    // entries made with who.time 40 days ago (an undone save, a discard's kept save) and 2 days ago;
    // after tidy with a 30-day window: the recent ones read back and undo still works; the old ones are gone
}
#[test] fn an_amend_made_after_a_month_away_can_still_be_undone_after_tidy() { /* the replaced save is reached only by the state before the amend */ }
#[test] fn a_step_undone_within_the_window_can_be_redone_after_tidy_however_old() {}
#[test] fn tidy_removes_entries_a_crash_left() {}
#[test] fn tidy_leaves_packs_changed_in_the_last_day_untouched() {}
#[test] fn tidy_stops_before_removing_anything_when_something_to_keep_is_missing() {}
#[test] fn tidy_stopped_at_every_step_loses_nothing() { /* fault sweep; every kept object reads back after each stop */ }
#[test] fn tidy_rewrites_version_1_and_small_packs_as_version_2() {}
#[test] fn tidy_leaves_a_pack_it_cannot_read_with_a_warning() {}
#[test] fn each_kind_of_reference_is_followed() { /* take each away from a planted store in turn: the object it names goes */ }
#[test] fn undo_window_days_is_read_from_the_users_settings_and_checked() { /* unset 30; 1 and 36500 ok; 0, 36501, "30" refused */ }
#[test] fn tidy_says_what_it_will_free_and_what_it_freed() {}
```
- [ ] **Step 2:** Run them. Expected: FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run `cargo test -p bolt-vcs --test tidy -j 2 -- --test-threads=2` and the workspace. Expected: PASS.
- [ ] **Step 5:** Commit: `Tidy the store: keep what lines, shelves and the undo window reach`.

### Task 11: The store's follow-ups

The milestone 4 rows of the follow-ups about the store.

**Files:** `crates/bolt-vcs-core/src/{store.rs,pack.rs,merge.rs,changeids.rs,repo.rs,warning.rs}`, `docs/json.md`, with tests beside each.

1. **A damaged pack no longer stops a save.** Where the only stored copy of an object a save, discard, shelf or amend needs is damaged, the object is stored again (a second copy is harmless) and the `damaged_pack` warning still names the pack. Test: `a_save_stores_again_past_a_damaged_only_copy`.
2. **A pack that can't be opened for another reason** (permission denied) gets a `pack_unreadable` warning naming it and why, once per command, as `copy_unreadable` does for a copy. Test: `a_pack_that_cannot_be_opened_is_named` (Unix: mode 000).
3. **A junk pack isn't opened again on every lookup:** the store remembers, for its lifetime, packs that failed to open, and a lookup keeps the error that matters most (damage over not found). Test: `a_junk_pack_is_opened_once_per_command`.
4. **Merge's damage notes stay** as `pack_not_merged` warnings when 32 or fewer packs stay small. Test: `a_damaged_small_pack_is_told_even_when_nothing_merges`.
5. **No cache is written with a key for one run only** (an unusable cache key, or the engine used without one), so a good cache isn't overwritten. Test: `a_run_only_key_writes_no_cache`.
6. **A read-only command never opens `state/lock` for writing:** where it can't, it skips the clean-up of `tmp/`; errors name the file. Test: `a_project_the_user_cannot_write_still_reads` (Unix: `state/` mode 0555).

- [ ] Steps: write each test, see it fail, fix, see it pass; `cargo test --workspace -j 2 -- --test-threads=2`; commit `Store follow-ups: store again past damage, name packs that can't be opened`.

### Task 12: The walk's follow-ups

The milestone 4 rows of the follow-ups about the walk, names and limits.

**Files:** `crates/bolt-vcs-core/src/{repo.rs,snapshot/walk.rs,snapshot/endings.rs,diff.rs,import/convert.rs}` and their tests.

1. (`begin` making `.bolt` through folder handles moved to Task 9, with the write-through it needs.)
2. A `.git` folder with mode 0711 no longer stops every save: Bolt reads what it needs in it (`info/exclude`) without listing it. Test (Unix): `a_git_folder_that_cannot_be_listed_does_not_stop_a_save`.
3. A link swapped for a file mid-save is reported as changed during the save. Test: `a_link_swapped_for_a_file_is_changed_during_the_save`.
4. The walk holds fewer handles than the system's limit on open files leaves room for: on macOS (256 by default) the walk's handles and the packs stay under it. Test (macOS): `a_deep_tree_saves_within_macos_open_file_limit`.
5. Which error a save reports first no longer depends on timing: the first problem by path. Test: `the_first_problem_by_path_is_the_one_reported`.
6. The line-ending check of the first save after an import matches names as the drive does. Test: `the_line_ending_check_matches_names_in_any_case_where_the_drive_does`.
7. Tests the follow-ups list as missing: a folder swapped above level 254; on Unix, a link target holding `\` stored as read; the path check reached on an untouched entry; the deep-tree test filling the job queue with distinct deep folders.
8. The depth checks in `diff::walk` and `Converter::enter`, which the 4,096-byte path limit reaches first, get a comment saying they are backstops; folders deeper than 254 are opened again from the deepest held one (O(depth²) at worst): measured on the deepest tree the limits allow and ruled on in the progress log.
9. On Unix, `create_new`'s clean-up removes a file it made by name: only ever in `.bolt/tmp`, which only Bolt writes under the lock; ruled on in the progress log, with a comment where it happens.

- [ ] Steps: as Task 11; commit `Walk follow-ups: handles from the start, no timing in errors`.

### Task 13: Output and messages

The milestone 4 rows of the follow-ups about output.

**Files:** `crates/bolt-vcs/src/{main.rs,output.rs,import_output.rs}`, `crates/bolt-vcs-core/src/{switch.rs,status.rs,changes.rs,listing.rs}`, `docs/json.md`, tests in `crates/bolt-vcs/tests/`.

1. `start`, `save`, `import`, `goto` and `discard` print through `output::Stdout`, so a reader that has gone ends them quietly (exit 0), as for `status`. Test: `every_command_ends_quietly_when_its_reader_has_gone`.
2. Wording: `discard` doesn't say "Nothing to discard" beside `changed_since_plan`; a store note isn't given twice when the CLI tries a save again; `status` says why it stops in its own words ("Bolt can't show the status: …"); a listing with a `.bolt` spelling names the save and what to do. Tests assert each message.
3. `bolt changes` marks a pair of lines that differ only in their line ending (`(CRLF)` after the line, in text; `"eol_only": true` in JSON). Lines that aren't UTF-8 stay U+FFFD in JSON, as json.md says (ruled in the progress log). A failed write to stdout mid-document ends with exit 1 and the error on stderr (ruled: a document can't be closed on a stdout that fails).

- [ ] Steps: as Task 11; commit `Every command prints through Stdout, and clearer messages`.

### Task 14: Hostile diaries written out

security.md T26; the follow-up "undo must validate safety-copy paths from the diary".

**Files:** create `crates/bolt-vcs/tests/corpus_diary.rs` (helpers from `tests/support/`).

Each case plants records into a project's store, as a hostile `.bolt` could hold them, makes them current with `state/head`, runs the command, and checks the exit code, the message, what was written, and that nothing reached a folder beside the project or went into `.git` or `.bolt`:
- a folder change to a save holding `.git` and `.bolt` spellings, `..`-shaped link targets, a link named like a folder with files below it, and names Windows can't hold → `bolt undo` / `bolt redo`;
- a shelf whose save holds the same → `bolt unshelf`;
- `undo --to` an entry whose state names a save with them;
- an undo whose target isn't a command entry; a state naming saves that don't exist; a folder change naming a missing save → refused, nothing changed.

- [ ] Steps: write the cases (each seen to fail with its defence taken out, noted in the commit); run `cargo test -p bolt-vcs --test corpus_diary -j 2 -- --test-threads=2`; commit `Hostile diaries: undo, redo and unshelf write nothing they shouldn't`.

### Task 15: Undo, redo, shelves and amend against the model

testing.md §3.

**Files:** `crates/bolt-vcs/tests/model.rs`.

- New actions: `Undo(n)` and `Redo(n)` with n from 1 to 3, each with or without `--shelve`, `UndoTo` a random entry, `Shelf`, `Unshelf(Option<label>)`, `Amend`. The model keeps the done and undone steps (each step the state and folder before and after it) and the shelves; its undo puts back the state and makes the folder change backwards; a stop for unsaved work in the way, which the model predicts with the same rule as `goto`, changes nothing.
- After every action, besides today's checks: the done and undone lists (`undo --list --json`) match the model's.

- [ ] Steps: extend, run `cargo test -p bolt-vcs --test model -j 2 -- --test-threads=2` with 256 cases locally (the nightly runs 1,024); commit `The model test undoes, redoes, shelves and amends`.

### Task 16: Docs and the follow-ups

**Files:** the docs.

1. `json.md`: every new document and warning (checked against the code); `testing.md` §3 and §4: the new tests and crash tests, the model's new actions; `security.md`: T26 to T29 as built, and the milestone 4 session reports once Task 17 is done; `storage-format.md` §13; `design.md`: the phase line, §5.15's commands; `milestones.md`: milestone 4's steps ticked.
2. The follow-ups: strike what this milestone finished, with the date; add what it defers (cleanup on its own, the size budget for safety copies, compressing again at a stronger level in milestone 5, divergent change IDs, shelving only some paths).

- [ ] Steps: make the changes; check each doc for CRLF (`tr -cd '\r' < file | wc -c` prints 0); commit `Bring the docs up to date with milestone 4`.

### Task 17: Red-team session

Decision 024.

- [ ] **Step 1:** Write `redteam/attack-surface-m4.md`: the diary (planted entries, folder changes, undo targets, shelves, a lost or planted `head`), undo/redo/unshelf as writers into the project, `undo --to` and its prefix, `tidy` (making it remove what is needed, a crash halfway, planted and damaged packs, the grace period), `save --amend` (shared change IDs), kept saves, crashes at every new fault point, Windows renames, output spoofing in the new lists; and the follow-ups left for a red team: a planted journal that makes each changing command hash up to 4 GiB before refusing it, a file a switch makes replacing one another program made an instant before, and two spellings that are hard links to one file in a case-sensitive folder.
- [ ] **Step 2:** Start Docker Desktop. A fresh agent on the most capable model attacks a copy of the branch in `redteam/sandbox.sh up <copy>` (no network, working only in the copy and the container). Windows-only attacks become tests on a `redteam/m4` branch run by CI, deleted afterwards. Proof tests go in `crates/bolt-vcs/tests/redteam6.rs`, ignored until fixed; the report in `docs/redteam/<date>-milestone-4.md`.
- [ ] **Step 3:** Fix every proven finding, each with its proof test un-ignored, or rule on it in writing (a decision record). New threats join security.md §4; deferred items join the follow-ups.
- [ ] **Step 4:** Run a second session on the fixes, its report in `docs/redteam/`. Shut Docker Desktop down afterwards.
- [ ] **Step 5:** Commit each fix separately, with the finding's ID in the message.

### Task 18: Whole-branch review and merge

- [ ] **Step 1:** A fresh reviewer on the most capable model reads the whole branch against `main` for correctness, the engineering rules, security, and that every document in `docs/` matches the code and today's state.
- [ ] **Step 2:** Fix the findings, or rule on them in writing.
- [ ] **Step 3:** Copy the progress log's rulings into `docs/plans/2026-11-06-milestone-4-rulings.md`: a "please confirm" list first, then every ruling verbatim, then the deferred items.
- [ ] **Step 4:** Run fmt, clippy and the tests locally, push, and open the pull request `milestone-4` → `main`, with no Claude attribution.
- [ ] **Step 5:** Wait for CI on all three systems, `deny` and CodeQL; check the code-scanning, secret-scanning and Dependabot alerts.
- [ ] **Step 6:** No new `unsafe`: merge with a merge commit, then mark milestone 4 done in CLAUDE.md and `milestones.md` through a small pull request.
