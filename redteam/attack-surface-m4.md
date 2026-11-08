# Milestone 4 attack surface (red-team session, decision 024)

Milestone 4 makes every step undoable. The diary (the record of every command, kept in the store and named by `.bolt/state/head`) now drives `undo`, `redo`, `undo --list` and `undo --to`. `shelf` and `unshelf` set unsaved work aside and bring it back. `save --amend` replaces the last save. `tidy` removes what nothing in the undo window needs any more. And every command stops when `state/head` is lost, until `undo --to` picks the diary up again.

Threat list: `docs/security.md` §4 (T1–T29; T26–T29 are new). Decisions 070–078 in `docs/decisions/` give the rules this code follows; `docs/design.md` §5.1, §5.7, §5.8 and §5.9 summarise them, and `docs/storage-format.md` §9–§11 give the bytes. Your goals:
- lose or corrupt data: unsaved work, saved history, a shelf, anything a step in the undo window needs, files outside Bolt;
- make a step that can't be undone, an undo that doesn't put back what the step changed, or a redo that doesn't do it again;
- read, write, remove or follow a link outside the project, or inside `.git` or `.bolt`;
- make `tidy` remove what is still needed;
- crash, hang, or exhaust memory or disk;
- trick the person at the terminal (spoofed output, misleading messages, a planted entry offered as where the diary stopped).

## Where the code is

- **The diary:** `crates/bolt-vcs-core/src/diary.rs` (entries, the folder change), `timeline.rs` (the done and undone steps, worked out walking back from `head`), `undo.rs` and `undo/` (`undo`, `redo`, `undo --list`, `--all`, `undo --to`, the lost-head list and pick-up `diary_ends`).
- **Steps that write into the project:** `kept.rs` (`switch_saves`: the two saves a step records, kept saves, links that can't be put back), `switch.rs` (`goto`, `discard`, undo, redo and jumps through `goto`'s planner, `--shelve`, finishing a stopped switch), `writeout.rs` (`plan`, `carry_out`, `Plan.matched`), `journal.rs`.
- **Shelves:** `shelf.rs` and `shelf/` (labels, the 10,000 limit, collisions, `unshelf` onto another save).
- **Amend:** `snapshot.rs` (`SaveRequest.amend`, the refusals when another line, a server line, a tag or another line's history names the save), `changeids.rs` (`replace`).
- **Crash recovery:** `repo.rs` (`HeadLost`, the lost-head paths, `move_head` removing a finished switch's journal, the best-effort clean-up of `tmp/` when the lock is free), `journal.rs` (`remove_before_head`).
- **`tidy`:** `tidy.rs` and `tidy/` (what it keeps, the 24-hour grace by a pack's time, the index checked before judging, packs holding only garbage removed first, others rewritten through `merge::copy_pack` with a filter, the stop at chunk lists and conflict records), `config.rs` (`undo-window-days`).
- **The store:** `store.rs` (storing again past a damaged copy, `pack_unreadable`, packs remembered as failed until the next listing).
- **Windows durability:** `crates/bolt-vcs-platform/src/dir/windows.rs` (write-through renames, removals and new folders; the read-only fallbacks; the only `unsafe`).
- **The walk:** `snapshot/walk.rs` (`.git` opened search-only for `info/exclude`, the first problem by path), `drive.rs` (the probe tries again when its file vanishes).
- **CLI:** `crates/bolt-vcs/src/main.rs`, `output.rs` (the new lists, step lines and `--json` documents in `docs/json.md`).

## Places worth attacking

- **A planted diary** (T26): entries naming saves whose listings hold `.git` and `.bolt` spellings, names Windows can't hold, links with `..` or absolute targets, a link named like a folder with files below it; folder changes whose two saves differ outside the step; undo and redo targets that aren't command entries; a redo that doesn't cancel the undo it meets; parent chains that loop or never end; states naming saves, tags or lines that don't exist. Make `undo`, `redo`, `unshelf` or `undo --to` write any of it.
- **Steps as writers:** make an undo or redo overwrite unsaved work it should stop at, write at a path its step never wrote, or bring back a file the user deleted after the step; make `--shelve` lose work in the way; make an undone jump made with `--shelve` lose its shelf; make finishing a stopped switch take a step it shouldn't. A step's two saves must agree wherever it wrote nothing, folders above what it wrote included (storage-format §9; the model test found and Task 15 fixed undo removing a folder the user made that a step only wrote inside): find another place they don't, through `goto`, `discard <path>`, `unshelf`, `undo --to` jumps, folders the user made in place of a saved file, names in other letter case, and ignored files left in a folder a step empties.
- **Placeholders:** a link Bolt writes as a plain file holding its target (one leading out, or any link where links can't be made) is that link to every command, so a switch removes or replaces it like the link. Make a switch remove or overwrite a file of the user's that way: one swapped in after the plan, an executable one, one whose bytes match another link's target, a placeholder renamed or hard-linked.
- **`undo --to <entry>`:** prefixes that match several entries, or something that isn't an entry, or (with `head` lost) a planted entry in a pack; an entry whose saves can't be read.
- **A lost head** (T28): remove, garble, truncate or replace `state/head`; make `undo --list` offer a planted entry as the likely end, or `undo --to` pick one up and write it; packs that can't be opened during the search; a diary of millions of entries (time and memory for `undo --list`, `undo`, `tidy`).
- **Shelves:** the 10,000th shelf, labels freed and reused, messages with escape sequences, `unshelf` onto another save, collisions with unsaved work, a shelf whose saves are gone.
- **Amend** (T29): make two saves people can see share one change ID (amend a save a tag, a server line or another line names; race another command); amend an imported save; empty messages.
- **`tidy`** (T27): make it remove anything an entry in the window needs (its state, its parent's state, its folder change's saves, a shelf's saves, a kept save, an undo or redo target's state, a tag's or a server line's history); lying or damaged indexes, version 1 packs, packs whose names differ only in letter case, a pack Bolt can't open; a crash at every fault point while it rewrites and removes; a pack's time in the future or far past; `undo-window-days` at 1 and 36,500; chunk lists and conflict records (kinds 18 and 19) must stop it.
- **Crashes:** stop every command at every new fault point (`journal:removing`, `shelf:planned`, the tidy points, `walk:sending`, `drive:case`, `drive:unicode`), then run any command: nothing lost, and the next changing command finishes or refuses cleanly.
- **Windows renames and removals:** read-only files, FAT and exFAT, a file in use. Windows-only attacks become tests on a `redteam/m4` branch, run by CI.
- **Terminal output:** escape sequences and bidi characters in entry messages, command names, shelf messages and line names, in `undo --list`, `--all`, the step lines and every `--json` document, and in the text stderr gets when stdout can't take a command's output (json.md §5: a reader gone, a write failing). Make a command panic, hang or give the wrong exit code with a closed or failing stdout or stderr.
- **Left for a red team** (`docs/plans/2026-11-01-milestone-1-follow-ups.md` rows 69–71): a planted journal that makes each changing command read and hash up to 4 GiB before refusing it; a file a switch makes replacing one another program made an instant before (there's no no-replace rename yet); two spellings that are hard links to one file in a case-sensitive folder (traced safe, untested).

## Limits already known (not findings unless you show more)

`docs/security.md` lists known limits for T1–T29, and `docs/plans/2026-11-01-milestone-1-follow-ups.md` the deferred items. Among them:
- folders past the 126 held levels are opened again one at a time: the deepest tree the path limit allows, with a file in every folder, takes about 84 s to save on Windows;
- undo and redo keep no safety copies: what they overwrite is in the step's saves, so it matters only when the store is damaged;
- `tidy` doesn't compress again (milestone 5), and rewrites a lone small pack every time;
- amend doesn't check a server line whose history is ahead of a line (decision 074);
- `shelf --json` can't tell its own race from a finished switch's `changed_since_plan`.
