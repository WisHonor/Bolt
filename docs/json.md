# Opalro `--json` Formats

> **Status:** written in milestone 2 (2026-11-02) for `start`, `save`, `history` and `show`, as [decision 017](decisions/017-commands-and-output.md) promised; `import` added 2026-11-03, `status` 2026-11-05, `goto`, `discard`, `changes`, `line`, `undo` and `redo` 2026-11-06, `save --amend`, `undo --to`, `undo --list`, `shelf`, `shelf --list`, `unshelf` and `tidy` 2026-11-07, and milestone 5's changes on 2026-11-08 and 2026-11-09: `tidy`'s stop at a pack it can't read, `import`'s Git LFS fields, stops and warning and its stop at a fault in Opalro, and `save`'s stop at Git LFS pointers. It holds every document and every kind of warning the seventeen commands print (checked against the code on 2026-11-03, the documents and warnings milestone 4 added or changed on 2026-11-07, and milestone 5's on 2026-11-09). Each command that comes later adds its document here in the milestone that builds it. Milestone 6 (decided 2026-11-09, [decision 117](decisions/117-what-json-covers-at-v0.1.md)) adds `setup` and `verify`, `start`'s preset fields, `shelf`'s `changed_again` and four kinds of warning, and freezes `opalro_json` 1 when v0.1 is released; before the freeze, every document was read once more against the others and the code (2026-11-10), and the names that disagreed were mended.

**In plain words.** Every Opalro command takes `--json`. Instead of text for people, it then prints one JSON document that a script, an editor plugin or the desktop app can read. This page says what is in each document, what each field means, and what `null` means where it can appear. The formats are a public interface: they are versioned, and the CLI tests keep them from changing by accident.

## 1. Rules for every document

- **One document, on stdout, in one line.** Nothing else goes to stdout. When the command succeeds, its warnings also go to stderr, as text; after a failure they are only in the document (§4 names the few that are told before the error). `history`, `show`, `changes` and `undo --list --all` print their documents while they read, but what comes out is still one object. Don't depend on the order of its keys.
- **`opalro_json` is the version of the format,** a number, `1` now. It is in every document.
- **The versioning rule.** Within one `opalro_json` number, fields may be added, and kinds of warnings may be added. Fields are never removed, renamed, or given another type or meaning. Programs must ignore fields and warning kinds they don't know. A change that breaks the rule gets the next number. A field that can be `null` says so in its table, and says when: `start`'s `save`, for one, is `null` if the project was made but its first save couldn't be read back (a `first_save_not_read` warning says so). Check for `null` before you use such a field.
- **Frozen at v0.1.** When v0.1 is released, `opalro_json` 1 is fixed for good under the rule above ([decision 117](decisions/117-what-json-covers-at-v0.1.md)): the field names, their types and meanings, the `kind` of each warning and the exit codes. The wording of `message` and `hint` texts isn't part of it: they are for people, and may change.
- **Never in a document, and never because of `--json`:** colour or any other terminal code, symbols picked for the terminal, progress, and the lines Opalro adds for a person at a terminal ("Next:", the hint the first time a Git word is used). With `--json` Opalro never asks a question: where it would ask a person, it fails and its hint names the flag (`--yes`, `--line`, `--preset`).
- **Exit codes.** `0`: the command did its work, though it may have warnings. `1`: it failed, and the document has `error`. `2`: the command line was wrong; the usage message is text, not JSON, and so is what Opalro says about a Git word it doesn't run ([decision 116](decisions/116-gits-words-as-shortcuts.md)).
- **Text from the project is data.** Names, messages and paths are given exactly as they are stored, so a program can use them. They can hold anything, escape sequences included, so escape them before showing them in a terminal. Opalro's own messages (`warnings[].message`, `error.message`, `error.hint` and `verify`'s `problems[].message`) are already escaped, exactly as they are shown on screen: a control character is written as `\x` and two hex digits, and a character that hides or reorders text as `\u{…}`, with only the newlines Opalro writes kept.
- **Messages are for people.** Programs should go by `kind`, `result` and the other fields, because the words of a message can change.
- **An error has no code in v0.1.** A script goes by the exit code and by which command it ran, and must not match on `error.message`. A code is planned, and will come as a new field of `error`.
- **A warning names its paths only in its `message` in v0.1.** That is a known gap: fields may be added to a warning later, as to any object.
- **Numbers are whole numbers.** Every number in a document is an integer. Sizes, counts and times stay far below 2^53; only `setup`'s `ask_before_saving_over` and `ask_before_saving_files_over` can go past it, since they are whatever the user wrote in `opalro.toml`, and a reader that keeps numbers as doubles (JavaScript's) then rounds them.
- **Paths are project paths,** with `/` between names, from the project folder. The one exception is `start`'s `root`.
- **What a name means.** A field name means one thing at a document's top level. Inside the objects a name can mean something else: `kind`, wherever it is, says which variant an object is (an entry's kind in a change, a warning's kind in `warnings`), and in `goto` the top-level `save` is a save while `position`'s `save` is a fingerprint. Across documents a count and a list can share a name: `lines` is a count in `import` and a list in `line`, and `saves` a count in `verify` and a list in `history`.
- **Every value Opalro chooses is written in snake_case:** the kinds of warnings, effects and problems, and the values of `result`, `change`, `kind` and `content` (`nothing_changed`, `kind_changed`, `project_link`, `too_big`). Text from the project, such as a path or a line name, is given as it is.

## 2. Pieces used by several commands

**A save** (`save` in the documents below):

| Field | Type | Meaning |
|---|---|---|
| `id` | string | The save's fingerprint: 64 hex digits. |
| `change_id` | string | The save's change ID: 32 letters from `k` to `z`, written as Jujutsu writes them (`z` for 0, `k` for 15; [decision 122](decisions/122-change-ids-in-jujutsus-letters.md)). |
| `change_id_short` | string | The shortest start of `change_id` (at least 4 letters) that no other save on the line starts with. The whole ID if the line couldn't be read to work it out; a `change_id_not_shortened` warning says so. |
| `message` | string | The message, as written. It can have several lines. |
| `author` | person | Who wrote the change. |
| `saver` | person | Who made the save. |
| `line` | string or `null` | The line the save was made on. `null` if the save doesn't say. |

**A person** (`author`, `saver`): `name` (string), `email` (string), `time_ms` (integer: milliseconds since 1970-01-01 UTC) and `tz_offset_min` (integer: minutes east of UTC in the person's time zone; negative is west).

**A warning:** `{"kind": …, "message": …}`, described in §4.

**IDs.** A fingerprint, a diary entry's ID, and every other "64 hex digits" in these documents is written in lowercase.

## 3. The documents

### `opalro start --json`

| Field | Type | Meaning |
|---|---|---|
| `opalro_json` | number | `1` |
| `root` | string | The project's folder: an absolute path in the system's own form, with `\` between names on Windows (it can start with `\\?\`), and each byte sequence that isn't UTF-8 shown as U+FFFD. It is the only path in any document that isn't a project path with `/`. |
| `position` | object | Where the project is, as in `status`: always `{"line": "main"}`, the line it starts on. |
| `save` | save or `null` | The first save, "Start project". **`null` means Opalro made the project but couldn't read the save back.** The command still succeeds, and a `first_save_not_read` warning says why. |
| `preset` | string or `null` | The preset the project started with: `"code"`, `"game"` or `"custom"` ([decision 113](decisions/113-presets-what-each-writes-and-how-start-picks-one.md)). `null` when the folder already had an `opalro.toml`, which stays as it is. |
| `found` | object | What Opalro saw in the folder: `engines`, a list of `"unity"`, `"unreal"` and `"godot"`, and `languages`, a list of `"node"`, `"python"`, `"rust"`, `"go"`, `"dart"`, `"dotnet"`, `"maven"` and `"gradle"`. The preset's rule files are written for these. |
| `rules_written` | list of string | The rule files the start wrote, as one undoable `setup` step after the start: `"opalro.toml"`, `".opalroignore"`, or `[]` (Custom writes none; an `opalro.toml` that was there is kept). |
| `warnings` | list | See §4. |
| `error` | object | Only when the project was made but its rule files couldn't be written (below): the exit code is then 1. |

A folder that already has an `opalro.toml` is judged first: there `opalro start` needs no `--preset` and never asks, the file stays as it is, and `preset` is `null`; giving `--preset` there fails before anything is made. In any other folder, with `--json` or without a terminal, `opalro start` needs `--preset code`, `--preset game` or `--preset custom`: without it nothing is started, and the error's hint names the flag and the preset Opalro would have suggested. A start whose project was made but whose rule files couldn't be written (one of the two names is a link or a folder, say) exits with code 1: the document has every field above, `rules_written` is `[]`, and `error` says what stopped the files, with a hint naming `opalro setup --preset <name>`, which writes them. The project is there, with no rule files yet, and `preset` stays the preset that was asked for.

### `opalro setup --json`

`opalro setup` shows the project's rules as Opalro reads them from `opalro.toml` and `.opalroignore` in the folder, and changes nothing ([decisions 114](decisions/114-bolt-toml-keys-and-how-bolt-reads-it.md) and [115](decisions/115-bolt-setup.md)).

| Field | Type | Meaning |
|---|---|---|
| `opalro_json` | number | `1` |
| `has_rules_file` | boolean | Whether the folder has an `opalro.toml` Opalro could read. `false` means Opalro's own values below apply, and a `rules_file_skipped` warning says why when a file is there. |
| `preset` | string or `null` | The file's `preset` label. `null` when it has none. |
| `chunk_files_over` | number | A file of this many bytes or more is stored in chunks. |
| `ask_before_saving_over` | number | A save that adds more new bytes than this asks first. |
| `ask_before_saving_files_over` | number | A save that adds more new files than this asks first. |
| `rules` | list | The file's `[[rule]]` tables that Opalro could read, in order: `files` (list of string), and `lockable` and `conflict_markers`, each `true`, `false`, or `null` where the rule doesn't say. |
| `ignore_block_lines` | number or `null` | How many lines the preset's block of `.opalroignore` holds, its two mark lines not counted. `null` when the file has no block. |
| `warnings` | list | See §4: `rules_file_skipped` and `rules_ignored` tell what in `opalro.toml` Opalro passed over. |

`opalro setup --preset <name>` works out what the preset's rule files would be, and with `--yes` writes them as one step that `opalro undo` takes back:

| Field | Type | Meaning |
|---|---|---|
| `opalro_json` | number | `1` |
| `preset` | string | The preset asked for. |
| `found` | object | As in `start`. |
| `files` | list | One object for each of `opalro.toml` and `.opalroignore` that would change: `path`; `before`, the file's text in the folder now, or `null` when there is none; and `after`, its text once written, or `null` when the switch removes it. Bytes that aren't UTF-8 show as U+FFFD. `[]` when the files already are what the preset writes. |
| `applied` | boolean | Whether the changes were written. `false` too when every file changed between the plan and the write, so nothing was written: the `changed_since_plan` warning names them, and the exit code is 0, as for a setup with nothing to change. |
| `warnings` | list | See §4. |
| `error` | object | Only without `--yes`, when there is something to change: `--json` can't ask, so nothing is written, the exit code is 1, and the hint names `--yes`. The rest of the document is still there, so a script can read what would change. |

### `opalro import --json`

`opalro import` brings the Git repository in the current folder into Opalro, whole ([design.md](design.md) §5.16). Names from Git can hold bytes that aren't UTF-8; in this document each such byte sequence is U+FFFD.

| Field | Type | Meaning |
|---|---|---|
| `opalro_json` | number | `1` |
| `commits` | number | How many commits became saves. |
| `lines` | number | How many lines the branches became. |
| `servers` | number | How many servers have server lines. |
| `server_lines` | number | How many server lines there are, all servers together. |
| `tags` | number | How many tags were imported. |
| `position` | object | Where the project is now: `{"line": …}`, the line Git's current branch became, or `{"save": …}`, the fingerprint (64 hex digits) of the save Git's detached `HEAD` became. |
| `renamed` | list | Each name Opalro's rules don't allow, and the one it was given: `{"kind", "git", "opalro"}`. `kind` is `"line"`, `"server"`, `"server_line"` or `"tag"`. A server line's `git` is `<server>/<branch>` and its `opalro` is `<line>@<server>`. `[]` if nothing was renamed. |
| `skipped_refs` | object | The refs that weren't imported, such as `refs/stash`, `refs/notes/commits` and `refs/pull/…` (only branches, server branches and tags are): `count`, how many in all, and `first`, the whole names of the first 100 in byte order. |
| `skipped_tags` | list | Each tag that wasn't imported: `{"git": …, "reason": …}`, its Git name and why, such as a tag of a folder. `reason` is text for people, like a message: its wording isn't frozen. |
| `skipped_entries` | object | Entries left out of saves because Opalro never writes their names to disk, such as spellings of `.git` and `.opalro`, with everything in them: `count`, how many in all, and `first`, the first 100 met, each `{"commit": …, "path": …}` (the Git commit in hex, and the path in it). A path can end in `/`, or be `""`, where the name left out is empty. |
| `inexact_commits` | number | How many commits a later export can't recreate with their Git IDs: their folders are in a form Git no longer writes, or entries were left out. |
| `lfs_files` | object | The Git LFS files brought in, each read from `.git/lfs/objects` and checked against its pointer's SHA-256 ([decision 085](decisions/085-importing-git-lfs-files.md)): `count`, how many objects (each version of a file is one, and pointers to the same object count once), and `bytes`, their total size. A pointer of size 0 counts, as an empty file. |
| `lfs_commits` | number | How many commits hold Git LFS files. A save keeps a file's contents, not that Git held a pointer for it, so a later export can recreate these commits with their Git IDs only by turning those files back into pointers. |
| `inexact_lfs_pointers` | number | How many Git LFS pointers aren't in the exact form Git LFS writes: `version https://git-lfs.github.com/spec/v1`, `oid sha256:` and the oid, `size` and the size, each line ending in LF. Git LFS keeps an empty file as an empty blob, so no pointer of size 0 is in that form. A later export can give a pointer back only in that form. |
| `warnings` | list | See §4. |

**Git LFS files** ([decision 085](decisions/085-importing-git-lfs-files.md)). A file of at most 1,024 bytes that Git LFS's own parser takes for a pointer comes in as the file it stands for, read only from `.git/lfs/objects/<oid[0..2]>/<oid[2..4]>/<oid>`, as a plain file of the size the pointer gives, never through a link, and checked against its SHA-256; a link holding a pointer's text comes in as it is, as Git checks it out. The import fails (exit code 1, with the error document of §5) when:

- objects are missing: `error.message` says how many objects are missing in all and their total size, then lists the first 20, each by path and commit with the size its pointer gives, and how many more there are, and `error.hint` says to run `git lfs fetch --all`. When the repository's config (`.git/config`, or `config.worktree` beside it when `extensions.worktreeConfig` is set) has an `[include]` or `[includeIf]` section, which Opalro doesn't read, the hint adds that if one sets `lfs.storage`, the files must be in `.git/lfs/objects` all the same. The hint also says that a Git setting Opalro doesn't read (the system's Git settings, or an `[includeIf]` section) may keep Git LFS's files elsewhere, that `git lfs env` shows where on its `LocalMediaDir` line, and that if that isn't `.git/lfs/objects`, the `lfs.storage` setting must be removed, or set to `lfs`, before the files are downloaded ([decision 094](decisions/094-red-team-m5-rulings.md)). When the repository shows a sign of Git LFS (a `.git/lfs` folder, or `filter=lfs` on a line of the top `.gitattributes` of `HEAD` that isn't a comment), this is found before anything is written; otherwise the import stops at the first missing object it meets, naming it, and nothing is committed;
- an object's SHA-256 isn't its pointer's: `error.message` names the file, the commit and the object, `error.hint` says to run `git lfs fsck <commit>` with that commit (which moves the bad copy to `.git/lfs/bad`), then `git lfs fetch --all`, and nothing is committed;
- a pointer has extension lines (`ext-`), whose file only Git LFS can rebuild, by running a program: `error.message` names the file and the commit, and with a sign of Git LFS this too is found before anything is written;
- the objects are kept elsewhere, and the import has a Git LFS file to bring in (one of size 0 needs no object, so it doesn't count) ([decision 089](decisions/089-git-lfs-objects-kept-elsewhere.md)). That is so when `lfs.storage` is set to anything but nothing or `lfs`, Git LFS's own place, taken as Git takes it, the last one found winning: the user's own Git settings files (those the user's Git ignore list is read from, with their plain includes: the file `GIT_CONFIG_SYSTEM` names, then the one `GIT_CONFIG_GLOBAL` names, or else `$XDG_CONFIG_HOME/git/config` and `~/.gitconfig`; [storage-format.md](storage-format.md) §12), then `.git/config`, then `.git/config.worktree` when `extensions.worktreeConfig` is set, then the settings `GIT_CONFIG_COUNT` gives ([decision 094](decisions/094-red-team-m5-rulings.md)); or else when `.git/lfs` or `.git/lfs/objects` is a link, or isn't a folder, which Opalro looks at only once it meets such a file, so an error reading them stops only an import that has one. `error.message` names the first such file and its commit, and says where the setting is set (the file, or the `GIT_CONFIG_VALUE_<n>` that holds it, with its value cut to 255 bytes) or which folder it is; `error.hint` says to remove the setting (or set it to `lfs`) and run `git lfs fetch --all`, or to put a real folder in place of the link. With a sign of Git LFS this is found before anything is written; otherwise the import stops at the first such file, and nothing is committed. In place of naming objects missing, it says why they can't be read. An import with no such file comes in whatever the setting says.

A file that isn't a Git LFS file but whose first 1,024 bytes look like a pointer, or that Git LFS's parser reads as one as they are or with CR LF line ends, comes in as it is, with an `lfs_lookalikes` warning (§4).

**A fault in Opalro.** What only a bug in Opalro's Git reader causes, such as losing track of what it read of a pack, stops the import too (exit code 1, with the error document of §5), and nothing is committed: `error.message` ends by asking for a report, and `error.hint` says it is a bug in Opalro, not a problem with the repository.

### `opalro save --json`

Saving gives one of two documents, told apart by `result`.

`opalro save "message" [paths]` saves every change, or only those at the files, folders and patterns named after the message ([decision 054](decisions/054-partial-saves.md)). They are taken from the folder the command runs in; a folder means everything in it, and an argument holding `*`, `?` or `[` is a pattern, matched as Git matches globs (`**` included), a folder it matches meaning everything in it. The save takes the folder's version of what they match, deletions included, and the last save's version of everything else, which stays unsaved work. On a drive that ignores letter case they match whatever the case (patterns fold ASCII letters, as Git does). To name a literal path holding `[`, `*` or `?`, put the character in a class: `a[[]1]` names `a[1]` (`\[` can't work on Windows, where `\` is between names). An argument matching nothing in the folder or the last save, a path outside the project or in `.opalro`, and a new file an ignore rule keeps out, named outright, fail the save (§5), saving nothing; a pattern just leaves such a file out.

`opalro save --amend ["message"] [paths]` replaces the newest save of the line instead ([decision 074](decisions/074-save-amend.md)). The new save holds that save's files with the folder's changes on top (all of them, or what the paths match, as above), and the message given, or the old one when none is given or it is `""` (which lets paths follow without a new message). It keeps the old save's parents, author, change ID and `line`, with the user as its saver, now, and drops the Git fields an import gave it (`git-commit`, `git-header`, `git-original-message`), which no longer describe it. The line then points at the new save; the old one stays stored, and `opalro undo` puts it back on the line, changing no file, so the amended changes show as unsaved again. Like any save, an amend asks before a large save and stops at the line-ending check after an import.

`"result": "saved"`: a save was made.

| Field | Type | Meaning |
|---|---|---|
| `opalro_json`, `result` | number, string | `1`, `"saved"` |
| `paths` | list of string | What the save was limited to, as project paths (`/` between names, `""` for the whole project), patterns joined to the folder they are matched below. `[]` when every change was saved. |
| `save` | save | The new save. |
| `changed_files` | number or `null` | How many files were added, removed or changed since the save before: for an amend, since its first parent, so every file the new save changes. It can be `0` when only folders changed. **`null` means Opalro couldn't count them.** The save was made all the same, and a `changed_files_not_counted` warning says why. |
| `replaced` | string or `null` | The fingerprint (64 hex digits) of the save an amend replaced. `null` for any other save. |
| `warnings` | list | See §4. |
| `ignored_project_identity` | object | Only when present: see below. |

**At a save, not on a line,** a save needs `--line <name>`: the new line it starts, which points at the new save; the project is then on it, and the save's `line` is that name. With `--json` Opalro never asks for the name, so without the flag a save with changes fails (§5), saving nothing. A name that breaks the line-name rules, or that is a line's already in any letter case, fails the save too, and so does `--line` on a line ([decision 055](decisions/055-lines-in-v0.1.md)).

`"result": "nothing_changed"`: the folder is as the save it would build on has it (the line's newest, or the save the project is at), so nothing was saved. At a save no name is needed for that, and with `--line` no line is made: a new line starts with a save, so change a file first. For an amend, the message is the save's own too, so nothing was amended. The fields are `opalro_json`, `result`, `paths` (as above: with paths, nothing they match changed), `warnings` and, if it applies, `ignored_project_identity`.

**`ignored_project_identity`** is there only when the project's own settings (`.opalro/config.toml`) name a person other than the user. Opalro never takes who the user is from a project, since a project that was shared could name anyone, so the save was made as the user. The field says who was named, `{"name": …, "email": …}`, and is left out otherwise. For people, `opalro save` prints a note instead of the field. The note shows at most 256 characters of a name or email, and gives the lines to add to their own settings, if they do want that identity in this folder, only when nothing was cut: lines with a cut name would save as someone else. The field always has both in full.

**A save that stops.** Like any failed command, a save that fails prints an error document (§5) and exits with 1. One case is worth knowing after an import: a save made on top of an imported save, or an amend that would replace one, stops, and saves nothing, when files differ from the imported version only in line endings, because Git turned LF into CRLF when it wrote them out. The `error.message` says how many files, and names the first few. The `error.hint` gives the two ways on: have Git write the files back as it stored them, or save them as they are by running the same command again with `--accept-crlf`, so that an amend stays an amend and a save of some paths saves only those. Once a save has been made, later ones don't check.

**Git LFS pointers in the folder** ([decision 085](decisions/085-importing-git-lfs-files.md)). Such a save also stops, saving nothing, when the folder holds a Git LFS pointer in place of a file the import brought in, as it does when Git LFS wasn't installed when the folder was checked out, or was told to skip: a file of at most 1,024 bytes that isn't the imported version, that Git LFS's own parser reads as a pointer, and whose `size` is the imported version's. It looks at the whole folder, even when the save names paths (the line-ending stop looks only at those): a pointer left out would be saved as it is by the next save, which doesn't look. This is looked for before line endings, and no flag saves the pointers anyway, `--accept-crlf` included. The `error.message` says how many files and names the first 5, and the `error.hint` says to run `git lfs checkout`, then save again, and that if Git LFS can't, `opalro discard` with those paths puts back the files the import brought in. A pointer of another size is saved as a change, as any pointer is once a save has been made.

**A save that adds unusually much** stops too, saving nothing, when it would add more than 100,000,000 bytes or more than 10,000 files that the save it builds on doesn't hold ([decision 056](decisions/056-question-before-large-saves.md)). Only new files count, by the sizes the folder reports: a file the last save already holds, changed or not, doesn't, nor does one renamed only in letter case on a drive that ignores case, and a partial save counts only what it would save. In a terminal Opalro asks first; with `--json` it never asks. The `error.message` gives the bytes and files it would add and the three top-level folders holding the most of those bytes (`the top folder` for files directly in the project folder), and the `error.hint` names `--yes`, which saves without asking. At a save, not on a line, `--line` is needed first: a save without it fails for that before the size is looked at.

**An amend that stops** fails (§5), saving nothing, before the folder is read, in two cases. At a save rather than on a line, the `error.hint` says to go to a line first. When another line, a server line or a tag names the save, or another line's history holds it along first parents, two saves you can see would then share one change ID: the `error.message` names the first 5 of them (`line b`, `server line main@origin`, `tag v1`) and counts the rest, and the `error.hint` says to save a new change instead. `--line` with `--amend` is a usage error (exit code 2): an amend never makes a line.

### `opalro status --json`

`opalro status` compares the project folder with the save the project is on, walking it as a save would (the same ignore rules, the same entries kept as they were). It reads only the files its file-state cache doesn't vouch for, and every file with `--full`. It never changes the project; it may only rewrite its caches in `.opalro/cache/`.

| Field | Type | Meaning |
|---|---|---|
| `opalro_json` | number | `1` |
| `position` | object | Where the project is, as in `import`: `{"line": …}`, or `{"save": …}` (64 hex digits) when it is at a save instead of on a line. |
| `save` | save | The save the folder was compared with: the line's newest, or the one the project is at. |
| `changes` | list of change | Each file that differs from that save, as in `show` (a change, below), in the order of the paths. `[]` when nothing changed. |
| `not_written` | list of string | The paths of the save's entries this computer can't hold as they are (project links, names this system can't create, names that are one file on this drive, links Git wrote as files holding their targets). Saves keep them as they were, so they are never changes. A kept folder is named once, not each entry in it. `[]` if none. |
| `unfinished_switch` | boolean | `true` when a switch between saves (`goto`, `discard`) was stopped before it was done, or just after it was done with its journal, `.opalro/state/journal`, left behind, or its journal can't be used; the `switch_unfinished` warning (§4) in `warnings` then says which. `false` otherwise. The folder is compared with the save the project is on: the one it was on before the switch, unless the switch was done. |
| `warnings` | list | See §4. |

### `opalro changes --json`

`opalro changes` shows unsaved changes line by line: the folder against the save the project is on, read as `status` reads it (it never changes the project; it may only rewrite its caches in `.opalro/cache/`). With `--save <save>` (a line name, the start of a change ID, or the start of a fingerprint) it shows what that save changed against its first parent instead. Paths after it limit it to those files and folders, taken from the folder you are in; a path is never taken for a save ([decision 058](decisions/058-changes-line-by-line.md)).

| Field | Type | Meaning |
|---|---|---|
| `opalro_json` | number | `1` |
| `save` | save | The save the folder was compared with, or the one `--save` named. |
| `changes` | list of file | Each file that differs, in the order of the paths: a change (as in `show`) with the fields below. `[]` when nothing changed. |
| `warnings` | list | See §4. |

**A file** is a change (§3, `show`) with these fields too:

| Field | Type | Meaning |
|---|---|---|
| `content` | string | `"text"`; `"binary"`, when either side has a zero byte in its first 8,000 bytes; `"too_big"`, when either side is over 64 MiB, which isn't read; or `"too_many_lines"`, when either side has more than 1,000,000 lines ([decision 064](decisions/064-changes-line-limit.md)). Only `"text"` has hunks. A project link or a conflict counts as empty text. |
| `hunks` | list of hunk | The changed lines with 3 lines of context around them, as in a unified diff. `[]` unless `content` is `"text"`. |
| `timed_out` | boolean | The diff took more than 2 seconds, so Opalro stopped it: the hunks are right, but in bigger pieces than they could be. A `diff_timed_out` warning says so too. |
| `only_line_endings` | boolean | Every line the hunks remove or add is one of a pair that differs only in its line ending (see `eol_only` below): CRLF into LF, LF into CRLF, or some of each. |

**A hunk:** `old_start`, `old_lines`, `new_start`, `new_lines` (numbers: the first line's number on each side, from 1, or the line before it for a side with no lines, and how many lines of that side the hunk covers), and `lines`, a list of `{"tag": …, "text": …}`: `tag` is `"same"`, `"removed"` or `"added"`, and `text` is the line with its ending (`"\n"`, `"\r\n"`, or none for a last line without one). Bytes that aren't UTF-8 show as U+FFFD. A removed line and an added line that differ only in their ending, `"\r\n"` for `"\n"` or the other way round, both have `"eol_only": true`; the pairs are the diff's own, the first of a run of removed lines with the first of the added lines right after it, the second with the second, and so on. Other lines leave `eol_only` out. For people, the line of such a pair that ends in CRLF is shown with `(CRLF)` after it.

### `opalro goto --json`

`opalro goto <target>` moves the project to a line or a save. The target is, in this order, a line's name (the project is then on that line), a server line `line@server` (at its save, not on a line), the start of a change ID on the current line, or at least 8 hex digits of a fingerprint (at that save, not on a line, even when it is a line's newest; the text output then says how to keep saving on that line, naming the first such line by name when several are there) ([decision 055](decisions/055-lines-in-v0.1.md)). A line's name that is also the start of another save's change ID or fingerprint is refused (exit code 1), and `error.message` lists the IDs it matched, so a line named like a save's ID never passes for that save; type more of the ID to reach the save ([decision 068](decisions/068-a-goto-target-that-names-both.md)). Unsaved changes come along unless the switch would overwrite them; then nothing changes, and the command fails, naming them ([decision 050](decisions/050-unsaved-work-in-goto-and-discard.md)).

| Field | Type | Meaning |
|---|---|---|
| `opalro_json` | number | `1` |
| `position` | object | Where the project is now, as in `status`: `{"line": …}`, or `{"save": …}` (64 hex digits). |
| `save` | save or `null` | The save the folder is now on. `null` only if Opalro couldn't read it back after the switch, which happened all the same. |
| `written` | number | Files and links written, links written as files included. |
| `removed` | number | Files, links and folders removed. |
| `not_written` | list of string | The paths of the save's entries this computer can't hold, left unwritten (see `not_written_here` in §4). `[]` if none. |
| `placeholders` | list of string | The paths of links written as plain files holding their targets (see `placeholders_written` in §4). `[]` if none. |
| `safety_copies` | number | How many safety copies of unsaved work the switch kept. `goto` never overwrites unsaved work, so it is `0` for now. |
| `recorded` | boolean | Whether the command recorded a step, which `opalro undo` takes back. `false` when the project already was there and there was nothing to write, so nothing changed. |
| `warnings` | list | See §4. |

**Files in use.** When another program holds files the switch must write (Windows), `goto` writes the rest and then fails (exit code 1) with the files named in `error`, and the failure document's warnings include `files_in_use` (§4). The switch isn't finished then: `head` and `.opalro/state/journal` stay, `status` gives `switch_unfinished`, and the next command that changes the project finishes it. `discard`, `shelf` and `unshelf` fail the same way. A document of a switch that succeeded never names files in use.

### `opalro discard --json`

`opalro discard [paths]` throws away unsaved changes: in those files and folders (a folder means everything in it; paths are taken from the folder the command runs in), or everywhere without any. Changed and deleted files are put back as the current save has them and new ones are removed, each changed or new one with a safety copy kept in a save its diary entry names; the project stays on its save. Ignored files the save doesn't hold, entries this computer can't hold, project links and nested projects are never touched ([decision 050](decisions/050-unsaved-work-in-goto-and-discard.md)), and neither are unsaved links an undo could bring back only as plain files (see `links_left` in §4). With nothing to throw away it records nothing and both lists of paths are `[]`. On a drive that ignores letter case, paths match whatever their case, as the drive matches them. A path outside the project, in `.opalro`, matching nothing in the save or among the unsaved changes, or matching more than one saved name (a save made elsewhere can hold `a.txt` and `A.txt`) fails the command (exit code 1) before anything changes, and so does something in the way of a file going back (ignored, or new since Opalro looked).

| Field | Type | Meaning |
|---|---|---|
| `opalro_json` | number | `1` |
| `restored_paths` | list of string | The paths of the files and links put back: changed or deleted ones. `[]` if none. |
| `removed_paths` | list of string | The paths of the new files and links removed, and of files and links a saved folder went back in place of. `[]` if none. |
| `folders` | number | Folders made or removed (a deleted saved folder put back, a new empty folder removed) that no listed file counts for. |
| `not_written` | list of string | As in `goto`. |
| `placeholders` | list of string | As in `goto`. |
| `safety_copies` | number | How many safety copies were kept: one for each changed or new file thrown away (a deleted file needs none). |
| `warnings` | list | See §4. |

### `opalro undo --json` and `opalro redo --json`

`opalro undo [N]` takes back the newest step not yet undone, or the newest `N` one after another, and `opalro redo [N]` brings back the step undone last, or the last `N` ([decision 070](decisions/070-undo-and-redo-step-through-the-diary.md)). Every command that changed something is a step; undos and redos aren't, and a new step clears what redo can bring back. Each step taken is a diary entry of its own. Undoing a step puts back the lines, where the project is, the tags and the server lines as they were just before it, and redoing it puts back its own; both write back, or write again, what the step did to the files, at exactly the paths it wrote, and other unsaved work stays as it is ([decision 071](decisions/071-a-step-records-its-change-to-the-folder.md)). Shelves stay, except one a `shelf` or `unshelf` step added or removed. With nothing (more) to undo or redo, the command succeeds with fewer steps than asked for, or none; the diary's first entry can never be undone. Unsaved work or a file Opalro ignores where a step would write, or a step whose saves can't be read any more (`tidy` removed them, or they were lost), stops the command before that step changes anything (exit code 1). With `--shelve`, the unsaved work in a step's way is set aside on a new shelf as part of that step, which is then taken; ignored files, and unsaved links that could come back only as plain files, still stop it, and then nothing is set aside. After finishing a switch that a crash or a file in use stopped, `undo` and `redo` take no step (`finished` is `true`), so running the command again takes one more ([decision 075](decisions/075-crash-recovery.md)).

| Field | Type | Meaning |
|---|---|---|
| `opalro_json` | number | `1` |
| `undone` (`undo`) or `redone` (`redo`) | list of step | The steps taken, in the order they were taken. `[]` if none. |
| `finished` | boolean | `true` if the command finished a stopped switch instead of taking a step. A switch that had taken effect, with only its journal left to remove, counts as one. |
| `shelved` | list | The shelves `--shelve` made, in the order it made them, each `{"label": …, "save": …}`: the shelf's number (a string), and its save's fingerprint (64 hex digits). `[]` if none. |
| `position` | object or `null` | Where the project is now, as in `status`. `null` only if Opalro couldn't read the project's newest diary entry back after the command, which did its work all the same. |
| `warnings` | list | See §4. |

**A step:**

| Field | Type | Meaning |
|---|---|---|
| `entry` | string | The diary entry of the step undone or redone, or of the entry `undo --to` went to: 64 hex digits. It is never the new entry this command made, which `opalro undo --list` shows. |
| `action` | string | `"undo"` or `"redo"`: what the diary entry the step made records. `"jump"` for the one step `undo --to` takes to an entry neither undo nor redo can reach (below); `opalro undo --list` shows the entry a jump made as `"command"`, with `command` `"undo"`. |
| `command` | string | The command that made the step, such as `save`, `goto` or `discard`. |
| `time_ms` | integer | When that command ran: milliseconds since 1970-01-01 UTC. |
| `effects` | list | What this undo, redo or jump changed: lines first, then where the project is, shelves, tags, server lines and files. |
| `written` | number | Files and links written, links written as files included. |
| `removed` | number | Files, links and folders removed. |

**An effect** is an object with a `kind`, and the fields that kind has:

| `kind` | Fields | Meaning |
|---|---|---|
| `line` | `name`, `from`, `to` | A line moved from one save to another (each 64 hex digits). `from` is `null` for a line that came back, `to` for a line that went. |
| `position` | `from`, `to` | Where the project is changed; each as `position` above. |
| `shelf_added` | `label`, `save` | A shelf came onto the list: one put back, or one `--shelve` made. |
| `shelf_gone` | `label`, `save` | A shelf left the list. |
| `tags` | | The tags changed. |
| `server_lines` | | The server lines changed. |
| `files` | `from`, `to` | The folder was switched between two saves (each 64 hex digits). For an undo or redo they are those of the step undone or redone, the other way round for an undo; for a jump they are its own switch, from the save the project was on to that entry's save. They picture the folder before and after at every path touched, whether written or already holding what would be written there, and either can be a save made only for that step. |

A step that stops the command fails it with exit code 1. Its document is the error document of §5 with `undone` or `redone` and `shelved` added, which list the steps taken, and the shelves made, before it stopped.

### `opalro undo --to <entry> --json`

`opalro undo --to <entry>` makes things as they were right after a diary entry ([decision 072](decisions/072-undo-list-and-undo-to.md)). The entry is named by the start of its ID, at least 8 hex digits in either case, as `opalro undo --list` shows them, and must match exactly one entry from the current one back. Fewer digits fail the command (exit code 1) before it does anything at all; no match, or more than one, fails it without taking a step, and `error.message` says which, naming the entries that matched. A run that first finishes a stopped switch doesn't look for the entry (below). A step redo can bring back is reached by redoing the steps up to it, and a step undo can take by undoing the steps after it, one at a time, exactly as `opalro redo N` and `opalro undo N` would, so redo works afterwards. Any other entry (an undo or redo entry, or a step a new command cleared from what redo can bring back) is reached by one new step, a jump: it puts back that entry's lines, where the project was, its tags and its server lines, keeps the shelves as they are, and switches the folder from the save the project is on to that entry's save as `goto` does, carrying other unsaved work along. Its diary entry records the command `undo`, and `opalro undo` takes it back. The current entry changes nothing, and neither does an entry whose state is already the current one. Unsaved work, or a file Opalro ignores, where a step would write stops it as it stops `undo`, and `--shelve` sets that work aside the same way. A jump to an entry whose saves can't be read any more (`tidy` removed them, or they were lost) stops before it changes anything, and `error.message` names the entry. After finishing a stopped switch it takes no step (`finished` is `true`); running it again goes on from there.

| Field | Type | Meaning |
|---|---|---|
| `opalro_json` | number | `1` |
| `steps` | list of step | The steps taken, in the order they were taken, as in `undo` (a step, above), each with its `action`. `[]` if none. |
| `finished` | boolean | As in `undo`. |
| `shelved` | list | As in `undo`. |
| `position` | object or `null` | As in `undo`. |
| `head_lost` | boolean | Only there, and `true`, when `head` was lost (below). |
| `warnings` | list | See §4. |

A step that stops it fails it with exit code 1, and its document is the error document of §5 with `steps` and `shelved` added.

**With `head` lost** (§5), `undo --to <entry>` picks it up instead ([decision 075](decisions/075-crash-recovery.md)): the entry may be any the store holds, so the digits must match exactly one diary entry in the store (`opalro undo --list` shows the likely ones), and no match, or more than one, fails it as above. It adds one step after that entry holding its state (its lines, where the project is, its shelves, tags and server lines), recorded as the command `undo`, and makes that step current. It changes no file: `opalro status` then shows how the folder differs from that state's save. An entry whose state names a save that can't be read (a line's, a shelf's, or the save the project would be at) is refused, naming the entry, and nothing changes, so the pick never strands the project; `opalro undo --list` shows the others. A stopped switch's journal is left as it is, since it can't be judged without `head`: the next command that changes the project then refuses it as made for another save, and says how to go on. The document has `head_lost`, and `steps` holds that one step with `action` `"jump"`, `effects` `[]` (there is no state from before it to compare with), and `written` and `removed` both `0`.

### `opalro undo --list --json`

`opalro undo --list` lists the steps redo can bring back and the steps undo can take, numbered for `opalro redo N` and `opalro undo N` ([decision 072](decisions/072-undo-list-and-undo-to.md)). It changes nothing.

| Field | Type | Meaning |
|---|---|---|
| `opalro_json` | number | `1` |
| `redo` | list of entry | The steps redo can bring back, the next one first: `opalro redo N` brings back the first N. `[]` if none. |
| `undo` | list of entry | The steps undo can take, newest first: `opalro undo N` takes the first N. At most 20. The diary's first entry, when it is listed, is last and can't be undone (its `first` is `true`). |
| `more` | boolean | Undo can take more steps than `undo` lists. |
| `warnings` | list | See §4. |

`opalro undo --list --all --json` lists every entry of the diary instead, newest first, and prints its document as it reads (§1, §5):

| Field | Type | Meaning |
|---|---|---|
| `opalro_json` | number | `1` |
| `entries` | list of entry | Every entry from the current one back to the diary's first: steps, undo and redo entries, and steps a new command cleared. |
| `warnings` | list | See §4. |

**With `head` lost** (§5), `opalro undo --list`, with or without `--all`, lists where the diary may have stopped instead, for `opalro undo --to` to pick one from ([decision 075](decisions/075-crash-recovery.md)): every diary entry the store holds that no other entry has as its parent, newest first by the time it records. Nothing is guessed, and nothing changes. Finding them reads the index of every pack, and the entries themselves; one that can't be read is left out, since it couldn't be made current either. A pack Opalro can't read for a reason other than damage (no permission, say) is passed over too, named in a `pack_unreadable` warning, and `opalro undo --to` passes over it the same way; a pack too damaged to open or search stops both, naming it.

| Field | Type | Meaning |
|---|---|---|
| `opalro_json` | number | `1` |
| `head_lost` | boolean | `true`: this is the document of a lost `head`, which has `ends` in place of the lists. |
| `ends` | list of entry | The entries no other follows, newest first, each an entry as below; its `effects` are what it changed from its parent, `[]` when that parent can't be read. `[]` when no diary entry in the store can be read. |
| `warnings` | list | See §4. |

**An entry:**

| Field | Type | Meaning |
|---|---|---|
| `entry` | string | The entry's ID: 64 hex digits. |
| `action` | string | `"command"` for a step (an entry a command made, a jump of `undo --to` included), `"undo"` or `"redo"` for an entry that undid or redid one. |
| `target` | string or `null` | The step an `"undo"` or `"redo"` entry undid or redid: 64 hex digits. `null` for a step. |
| `command` | string | The command's name, as the diary records it: `undo` for a jump. |
| `time_ms` | integer | When the command ran: milliseconds since 1970-01-01 UTC. |
| `effects` | list | What the entry changed from the one before it, each an effect as in `undo` (above); a `files` effect gives the entry's own switch, from the save the folder was switched from to the one it was switched to. `[]` for the diary's first entry. |
| `first` | boolean | The diary's first entry, made by `start` or `import`, which can't be undone. |

### `opalro shelf --json`

`opalro shelf ["message"]` sets every unsaved change aside on a new shelf, then puts the folder back to the current save as `discard` does without paths ([decision 073](decisions/073-shelves.md)). The folder's version of each changed, new or deleted file or link and each new folder is kept as a save on top of the current one, the shelf's save; the shelf goes first on the list, as the newest, numbered with the smallest number from 1 that no shelf has. Its message is `Shelved` when none is given. `discard`'s rules apply: ignored files, project links, entries this computer can't hold and nested projects are never touched, nor are unsaved links that could come back only as plain files (see `links_left` in §4), and something in the way of a file going back fails the command (exit code 1) before anything changes. A project keeps at most 10,000 shelves: one more fails the command before anything changes. With nothing to set aside it records nothing, and `shelf` is `null`; so it is when every file changed again while Opalro was reading it (a `changed_since_plan` warning names them).

| Field | Type | Meaning |
|---|---|---|
| `opalro_json` | number | `1` |
| `shelf` | object or `null` | The new shelf: `label`, its number (a string); `save`, its save's fingerprint (64 hex digits); and `files`, how many files it changes. `null` if there was nothing to set aside. |
| `changed_again` | boolean | `true` when there were changes to set aside but every one of them changed again while Opalro read it, so nothing was shelved (`shelf` is `null`, and a `changed_since_plan` warning names the files): run the command again. `false` otherwise, also when `shelf` is `null` because the folder had nothing to set aside. |
| `written` | number | Files and links put back as the save has them, links written as files included. |
| `removed` | number | Files, links and folders removed. |
| `not_written` | list of string | As in `goto`. |
| `placeholders` | list of string | As in `goto`. |
| `warnings` | list | See §4. |

### `opalro shelf --list --json`

`opalro shelf --list` lists the shelves, newest first. It changes nothing.

| Field | Type | Meaning |
|---|---|---|
| `opalro_json` | number | `1` |
| `shelves` | list of shelf | Every shelf, newest first. `[]` if there are none. |
| `warnings` | list | See §4. |

**A shelf:**

| Field | Type | Meaning |
|---|---|---|
| `label` | string | Its number, as `opalro unshelf` takes it. |
| `save` | string | Its save's fingerprint: 64 hex digits. |
| `message` | string | Its message, as written; `Shelved` if it was given none. It can have several lines. |
| `line` | string or `null` | The line it was shelved on. `null` if the project was at a save, not on a line. |
| `saver` | person | Who shelved it, and when. |
| `files` | number | How many files it changes from the save it was shelved on. |

### `opalro unshelf --json`

`opalro unshelf [number]` brings back the newest shelf, or the one with that number, onto whatever save the project is on ([decision 073](decisions/073-shelves.md)). Its changes are made as a switch from the save it was shelved on to the shelf, carrying all other unsaved work along, and the shelf leaves the list. Where the folder differs from the save it was shelved on at a path the shelf changes (unsaved work there, a file Opalro ignores, or the current save changed it since), nothing changes and the command fails (exit code 1). `error.message` names the paths of each kind apart: unsaved work or files Opalro ignores, and what the current save changed since the shelf was made (the save shelved on and the current one differ there). The hint gives each kind its own way out: save, shelve or discard the work, then bring the shelf back by its number; and for what the current save changed, go to the save the shelf was made on with `opalro goto` and the first 12 hex digits of its ID, which the hint gives, and bring the shelf back there. A path counts as changed by the current save where the two saves differ at it, or where the current save holds a file or link at a folder above it that the save shelved on doesn't hold there (it made the folder a file, say, or changed a file the shelf made a folder); work of the user's own below a folder the current save made or removed counts as unsaved work. When `goto` can't reach the save shelved on by those digits (it finds only saves some line still reaches, and an `undo --to` that moved a line back can leave one behind), the hint says so instead, and points to `opalro undo --list`, whose steps can bring it back. A number no shelf has, or no shelf at all, fails it too. After finishing a switch that a crash or a file in use stopped, it brings back no shelf (`finished` is `true`), so running it again never brings back a second one ([decision 075](decisions/075-crash-recovery.md)).

| Field | Type | Meaning |
|---|---|---|
| `opalro_json` | number | `1` |
| `finished` | boolean | `true` if the command finished a stopped switch instead of bringing a shelf back. A switch that had taken effect, with only its journal left to remove, counts as one. |
| `shelf` | object or `null` | The shelf brought back: `label`, its number, and `save`, its save's fingerprint (64 hex digits). `null` when `finished` is `true`. |
| `written` | number | Files and links written, links written as files included. |
| `removed` | number | Files, links and folders removed. |
| `carried` | number | How many files held unsaved changes, against the current save, that stayed as they were. |
| `not_written` | list of string | As in `goto`. |
| `placeholders` | list of string | As in `goto`. |
| `warnings` | list | See §4. |

### `opalro tidy --json`

`opalro tidy` frees the space nothing needs any more ([decision 077](decisions/077-tidy-and-the-undo-window.md)). It keeps everything the lines, the save the project is at, the shelves, the tags and the server lines reach, with all their history; every diary entry from the current one back, every diary end (an entry no other follows) made within the undo window, with its diary back, so a diary cut off from `head` survives ([decision 079](decisions/079-red-team-m4-rulings.md)); and, for every entry made within the undo window, what undoing or redoing it needs: the states of that entry and of the one before it and both saves of its folder change, and for an undo or redo the same for the step it undid or redid. The window is `undo-window-days` at the top of the user's own settings file, a whole number of days from 1 to 36,500, and 30 when it isn't set; any other value, or the line written below a `[section]`, fails the command (exit code 1), naming the file. It follows chunk lists ([decision 083](decisions/083-a-chunk-list-is-stored-under-its-files-id.md)): every chunk that a chunk list of a file it keeps names is kept, and so is every chunk a list in a pack changed in the last 24 hours names. Everything else goes: saves nothing above reaches (undone, cleared or amended ones, and what old `discard`s and shelves kept), what only they hold, and diary entries a crash left off the diary. A pack changed in the 24 hours before the command started is left as it is. Every pack's index, young ones' too, is read and checked against its checksum first, since what goes is judged from the index alone; a pack that can't be opened, or whose index can't be read or fails its checksum, fails the command before anything is removed, since it could hold the only chunk list naming chunks in other packs: `error.message` names the pack and says why, and `error.hint` says to check its permissions, and that moving it out of `.opalro/store/packs`, if it is damaged for good, lets `tidy` run, losing what only it holds, and the chunks only its chunk lists name. Everything in that folder named like a pack counts as one. Of the rest, one holding nothing to keep is removed first, before anything is written, so `tidy` frees space even on a full disk; one holding something else, or small (as for merging, each chunk list counted at its own size), or in the old version 1, is written again with only what is kept, chunk lists last as merging writes them, and removed only once every new pack is on the disk; the others stay. One whose copy fails is left as it is (`pack_not_tidied` in §4). A stop at any point loses nothing that is kept. To work out what to keep, it reads only records (diary entries, saves, folder listings, tag lists, server lines and project links) and chunk lists, never a file's contents, which it keeps by their IDs. If one it must read can't be read, nothing is removed and the command fails, naming it; a chunk list is named by its pack, whether it is a file's it keeps or one in a pack changed in the last 24 hours. And so it does if any pack holds a conflict record, which this version doesn't follow (`error.message` names the pack). In a terminal it says about how much it will free, on stderr, before it removes anything. It records no diary entry, so `undo` doesn't see it. A switch a crash or a file in use stopped is finished first, as any command that changes the project finishes one.

| Field | Type | Meaning |
|---|---|---|
| `opalro_json` | number | `1` |
| `to_free` | number | About how many bytes it worked out it would free, before it removed anything. |
| `bytes_before` | number | The size on disk of the store's packs before, in bytes. |
| `bytes_after` | number | The same after: `bytes_before` minus `bytes_after` is what it freed. |
| `packs_removed` | number | Packs removed. |
| `packs_written` | number | New packs written, holding what was kept from the packs written again. |
| `packs_young` | number | Packs left as they are because they changed in the 24 hours before; a later `tidy` frees what they hold that nothing needs. |
| `warnings` | list | See §4. |

### `opalro verify --json`

`opalro verify` checks everything Opalro has stored, and that the project has everything it needs ([decision 118](decisions/118-bolt-verify.md)). It changes nothing and takes no lock. Like every command, it makes the two empty working folders `.opalro/tmp` and `.opalro/cache` if they are missing. First every pack: it opens, its index reads in full and matches its checksum, and every object in it reads back as what the index says (contents against their fingerprint, records decoded as their kind, each chunked file rebuilt from its chunks). Then, from the current diary entry, everything `opalro tidy` would keep: each record must read, and each file's contents, chunk list and chunks must be in some pack.

| Field | Type | Meaning |
|---|---|---|
| `opalro_json` | number | `1` |
| `packs` | number | Packs checked. |
| `objects` | number | Objects read and checked in them. |
| `bytes` | number | The packs' size on disk, in bytes. |
| `saves` | number | Saves the project needs that were read. |
| `entries` | number | Diary entries read, from the current one back to the first. |
| `problems_total` | number | How many problems were found. `0` means everything checked out. |
| `problems` | list | The problems, at most the first 1,000: `kind` (below); `pack`, the pack's file name, or `null`; `object`, the object's fingerprint (64 hex digits), or `null`; `needed_by`, for something the project needs, an object with the `save` (its fingerprint) and the `path` in it that needs the object, or `null`; and `message`, for people. |
| `warnings` | list | See §4. `switch_unfinished` says so when a switch is unfinished, as for `status`; `diary_end_unreadable` names a diary cut off from the current one that can't be read back to its start. |
| `error` | object | Only when `problems_total` isn't 0: the exit code is then 1. |

| `kind` of problem | What it means |
|---|---|
| `pack_damaged` | A pack isn't a pack any more, or its index can't be read or doesn't match its checksum. |
| `pack_unreadable` | A pack couldn't be opened or read for a reason that isn't damage (no permission, say). It may be fine. |
| `object_damaged` | An object doesn't read back as what its pack's index says: it fails its fingerprint, doesn't decode as its kind, or, for a chunked file, its chunks don't build the file. Also a folder listing that gives a file a size its stored contents don't have. |
| `object_missing` | Something the project needs isn't in any pack, or is stored only as another kind of object than what needs it (a chunk held only as a chunk list, say). |
| `save_too_big` | A save the project needs is larger than any reader takes: more than 10,000,000 files and folders counted by path, or deeper or with longer paths than [storage-format.md](storage-format.md) §3 allows. `opalro goto` refuses such a save. |
| `head_lost` | `.opalro/state/head` is missing, can't be read, or names nothing Opalro can read, so what the project needs couldn't be worked out; the packs were still checked. `opalro undo --list` then lists where to pick up. |

### `opalro line --json`

| Field | Type | Meaning |
|---|---|---|
| `opalro_json` | number | `1` |
| `position` | object | Where the project is, as in `status`: `{"line": …}`, or `{"save": …}` (64 hex digits) when it is at a save instead of on a line. |
| `lines` | list of string | Every line's name, in byte order. |
| `server_lines` | list of string | Every server line as `line@server`, by server and then line, in byte order. `[]` if none. |
| `warnings` | list | See §4. |

### `opalro history --json`

| Field | Type | Meaning |
|---|---|---|
| `opalro_json` | number | `1` |
| `position` | object | Where the project is, as in `status`: `{"line": …}`, or `{"save": …}` (64 hex digits) when it is at a save instead of on a line. |
| `saves` | list of save | The saves of the line, newest first, following first parents back to the start. |
| `warnings` | list | See §4. |

### `opalro show --json`

`opalro show` takes a line name, the start of a change ID, or the start of a fingerprint (at least 8 hex digits), and shows the save the project is on when given none. A text that is both a line's name and the start of another save's ID is refused, as for `goto` (and for `changes --save`).

| Field | Type | Meaning |
|---|---|---|
| `opalro_json` | number | `1` |
| `save` | save | The save shown. |
| `changes` | list of change | What the save changed compared with its first parent (with all of its files for a save that has none), in the order of the paths. |
| `warnings` | list | See §4. |

**A change:**

| Field | Type | Meaning |
|---|---|---|
| `path` | string | The path from the project folder, with `/` between names. |
| `change` | string | `"added"`, `"removed"`, `"modified"`, or `"kind_changed"`: the same path became another kind of file, such as a file that became a link. |
| `kind` | string | `"file"`, `"executable"`, `"symlink"`, `"project_link"` or `"conflict"`: the kind after the change, or before it for a removed file. |
| `old_size`, `new_size` | number or `null` | Size in bytes before and after. **`null` means the path wasn't there on that side:** `old_size` for an added file, `new_size` for a removed one. A project link or a conflict has size `0`. |

Folders are not listed; the files in them are.

## 4. Warnings

`warnings` is in every document, the one for a failed command included, and is `[]` when there is nothing to say. A warning is something that went wrong without stopping the command. When a command finishes, people see each one on stderr as `warning: <message>`. A command that fails lists the warnings it had found before it failed in its document, and people are shown only the error, except for the warnings of a stopped switch the command finished first (`switch_finished`, `changed_since_plan`, `placeholders_written`, `not_written_here`), `head_not_flushed`, of a step whose head had moved, and `switch_dropped`, of a stopped switch an `undo --to` dropped before it failed: each tells of what happened all the same, so they come before the error. When stdout can't take what a command prints, stderr gets it instead, in the order §5 gives.

| Field | Type | Meaning |
|---|---|---|
| `kind` | string | A code in snake_case that stays the same. |
| `message` | string | What happened, for people. Escaped. It can have several lines. |

| `kind` | Where | What it says |
|---|---|---|
| `pack_damaged` | any command that reads | A pack held a copy of something that failed its check. If another copy was good, Opalro went on with that one. If none was, the command fails and `error` says so, and the warning still names the pack. Ask for help before trying to repair the project. |
| `copy_unreadable` | any command that reads | A stored copy couldn't be read for a reason other than damage, such as a disk error, although another copy could. A failing disk can cause it. |
| `pack_unreadable` | any command that reads | A pack couldn't be opened or searched for a reason other than damage, such as no permission to read it, a disk error or too little memory, so Opalro passed over it, looking in the other packs. Each pack is named once, with why and what to do: check that the file can be read, or free some memory. If what the command needed could only be in that pack, the command fails, and `error` names the pack too. |
| `pack_not_merged` | any command that changes the project | The command happened, and then more than 32 of its store's packs were small, so Opalro merged them, but not all of it worked: a pack is damaged (it, or a copy in it, failed Opalro's checks), so it was left as it is (ask for help before trying to repair the project); a pack couldn't be read (a disk error, say), so it was left as it is too; a merged pack couldn't be removed, so its objects are in two packs, which does no harm; or the merge stopped, and the next command that changes the project tries again. A pack that couldn't be read is named whenever more than 32 packs are under 64 MiB on disk, since each one's index is then read to judge it, even when too few of the rest are small to merge. The message names the pack, or says why the merge stopped. |
| `pack_not_tidied` | `tidy` | A pack was left whole, as it is, once its index had been checked: a copy in it failed its check, which is damage (ask for help before trying to repair the project), Opalro couldn't read it while copying (a disk error, say), its index read differently when its objects were copied than when it was checked (a failing disk can cause it), or it couldn't be removed, which does no harm (what it holds that must be kept is in a new pack too, or it held nothing to keep). The message names the pack and says why; the next `tidy` tries again. A pack whose index can't be read or checked stops `tidy` instead (§3). |
| `head_not_flushed` | any command that changes the project | The command happened: its new `head`, `.opalro/state/head`, is in place. But flushing `.opalro/state` to the disk after that failed, so a power cut soon after could undo the command ([decision 075](decisions/075-crash-recovery.md)). A switch removes its journal after that, and when the flush after that removal fails, the message says so instead: the head is in place, and a power cut could only bring the journal back, which the next command that changes the project removes. The message says why. The command succeeds unless a later step of the same run fails (`undo 3` stopped at its second step, say); people are then shown this warning before the error, since that step happened. |
| `change_id_not_shortened` | `import`, `save`, `status`, `show`, `changes`, `goto`; for people, also `undo --list` and `undo --list --all` | Opalro couldn't read the line's older saves to work out the short form of a change ID, so the whole ID is given as `change_id_short`. |
| `cache_key_unusable` | `start`, `save`, `status`, `changes`, `history`, `show`, `line`, `goto`, `discard`, `undo`, `redo`, `undo --list`, `shelf`, `shelf --list`, `unshelf`, `tidy` | Opalro's cache key, the file `cache-key` in the user's own settings folder, can't be read, isn't 32 bytes long, or couldn't be made. Opalro never replaces it, so the command worked without its caches, which only make it faster. The message names the file and says to delete it if it is there (Opalro then makes a new key), or else to check that Opalro can write to its folder. |
| `changed_files_not_counted` | `save` | The save was made, but `changed_files` is `null`. The message says why. |
| `gitignore_skipped` | `save`, unless the save stops; `status`; `changes`; `goto`, `discard`, `undo`, `redo`, `shelf` and `unshelf`, unless they stop | A `.gitignore` isn't a regular file (it is a link, folder or special file), so Opalro skipped it and its rules didn't apply, as Git skips a linked one. The same goes for Git's personal lists where Opalro reads them without following links: `.git/info/exclude`, and the user's own list when it is inside the project; one of those is skipped too when a folder on the way to it, such as `.git`, is a link ([decision 060](decisions/060-user-git-settings-includes-and-paths.md)). The message names the file, and the linked folder if there is one. |
| `kept_as_they_were` | `save`, unless the save stops | The save kept entries of the last save exactly as they were, because this computer can't hold them as they are: project links, names this system can't create and names that are one file on this drive, whatever the folder holds at their paths, and links Git wrote as files holding exactly their targets. The message counts them. |
| `git_names_left_out` | `save`, unless the save stops; `status`; `changes`; `goto`, `discard`, `undo`, `redo`, `shelf` and `unshelf`, unless they stop | The save left out entries whose names Git treats as `.git` or Opalro as `.opalro`, the ones `opalro import` leaves out too (`.GIT` where letter case counts, `git~1`, `.opalro.`, `opalro~1`, spellings with characters macOS ignores, a `.git` file), with everything in them, wherever they were: those in the folder, and those the last save held, which the new save no longer has ([decision 059](decisions/059-bolt-spellings-left-out-like-git-spellings.md)). The one exception is a folder kept as it was (see `kept_as_they_were`), which keeps everything the last save holds in it. The project's own `.opalro` folder and Git's `.git` folder at the top aren't counted, in any spelling the drive takes for them (`.GIT` where it ignores letter case), nor is an entry an ignore rule keeps out that the last save didn't hold. Each is named once; the message names the first 5 by path, with every invisible character shown, and counts the rest. |
| `nested_projects_left_out` | `save`, unless the save stops; `status`; `changes`; `goto`, `discard`, `undo`, `redo`, `shelf` and `unshelf`, unless they stop | Folders below the top hold a Git repository or Opalro project of their own (an entry named `.git`, such as a worktree's or submodule's file, or a `.opalro` folder), so the save took nothing new from them, as Git and Jujutsu leave such folders out ([decision 046](decisions/046-nested-projects-left-out.md)). What the last save holds in such a folder stays saved, as in an ignored folder. A folder an ignore rule keeps out isn't counted. The message names the first 5 by path and counts the rest; it comes with every save while the folders are there. |
| `ignored_changes_saved` | `save`, when it makes a save | The save took changes to files or links that an ignore rule matches, which stay saved because the last save holds them: their contents or their kind changed. Git would leave such changes out unless the files were added (with `git add -f` if Git doesn't track them yet). Unchanged ones aren't counted, nor are new ones, which the rule keeps out, or deleted ones. The message names the first 5 by path and counts the rest. |
| `gitignored_files_saved` | `save`, when it makes a save | The save took files or links that the project's `.gitignore` files keep out, because a `.opalroignore` line takes them back, or takes back a folder they are in: new ones, and ones whose contents or kind changed. Git would leave such changes out unless the files were added (with `git add -f` if Git doesn't track them yet), and a repository can track a `.opalroignore` like any other file. Unchanged ones aren't counted, nor are deleted ones. The message names the first 5 by path and counts the rest. |
| `file_cache_not_stored` | `save`, `status`, `changes`, `goto`, `discard`, `undo`, `redo`, `shelf`, `unshelf` | Opalro couldn't write its file-state cache (`.opalro/cache/files`): it would be bigger than 256 MiB, or the write failed. The save or status is right all the same; the next one reads every file again, which is only slower. The message says why. |
| `not_written_here` | `goto`, `discard`, `undo`, `redo`, `shelf`, `unshelf`, and a command that finishes a switch a crash stopped | Entries of the save being written were left unwritten, because this computer can't hold them as they are: a name this system can't make (`aux.txt` or `a:b` on Windows, one too long), a spelling of `.git` or `.opalro` (on every system), a name inside a folder Windows would misread (`a\b/c`), or two names that are one file on this drive (`README` and `readme` where letter case is ignored), which are both left out ([decision 051](decisions/051-names-this-computer-cant-write-and-length-limits.md)). Nothing is written at or below them; they stay in history, and saves keep them as they are. The message names the first 5 by path (a folder stands for what is in it) and counts the rest. |
| `placeholders_written` | `goto`, `discard`, `undo`, `redo`, `shelf`, `unshelf`, and a command that finishes a switch a crash stopped | Links were written as plain files holding their targets: the target leads out of the project, judged from the save alone (an absolute path, a drive letter, `\\`, `..` past the top, a way through another link or through a folder that save doesn't hold, or a way through a name Opalro never writes: a spelling of `.git` or `.opalro`, or a name this system can't hold), or holds a `\` on Windows, or this computer can't make links (Windows without Developer Mode or an administrator) ([decision 052](decisions/052-links-and-executable-bit-on-windows.md)). For `undo` and `redo` the save is one the step's folder change names, where a folder the step made holds only what the step wrote in it; for `unshelf` it is the shelf's save, and then the one its folder change switches to. The message names the first 5 by path and counts the rest. |
| `files_in_use` | `goto`, `discard`, `undo`, `redo`, `shelf`, `unshelf`, and a command that finishes a switch a crash stopped | Another program (an editor or virus scanner, say) held files open on Windows for about a second, so Opalro left them as they were and wrote the rest. The switch isn't finished, so `goto`, `discard`, `undo`, `redo`, `shelf` and `unshelf` fail, naming the files: closing the program and running the command again finishes it ([decision 049](decisions/049-writing-files-one-at-a-time.md)). The message names the first 5 by path and counts the rest. |
| `changed_since_plan` | `goto`, `discard`, `undo`, `redo`, `shelf`, `unshelf`, and a command that finishes a switch a crash stopped | Entries weren't what Opalro found when it planned the switch, a moment before (another program changed or made them, or put a link where a folder was), so Opalro left them as they are rather than lose that work. Nothing below such a folder is written either. The message names the first 5 by path and counts the rest. |
| `links_left` | `discard`, `shelf` | Unsaved links (new, or changed) were left as they are rather than thrown away or shelved, with the folders that hold them, and nothing saved was put back in their place: an undo, or `unshelf`, could bring them back only as plain files holding their targets ([decision 071](decisions/071-a-step-records-its-change-to-the-folder.md)). That is so when this computer can't make links (Windows without Developer Mode or an administrator), and when write-out would write a link as a placeholder in the save of what was thrown away or shelved, judged from that save alone: its target is longer than 4,096 bytes, is absolute, starts with a drive letter or `\\`, climbs past the top with `..`, goes through a name that isn't a folder of that save (another link, or a new folder left out of it), or through a name Opalro never writes (a spelling of `.git` or `.opalro`, or a name this system can't hold); on Windows also when it holds a `\` or a `:`. They stay in the folder as they are: delete them by hand if they aren't wanted. The message names the first 5 by path and counts the rest. |
| `switch_unfinished` | `status`, `changes`, `history`, `show`, `line`, `undo --list`, `shelf --list`, `verify`; and a command that changes the project, when its own switch, or a stopped one it finished first, was done but couldn't remove its journal | A switch between saves was stopped (a crash, Ctrl+C, a file in use) before it was done, so some files may still be as they were. Running `goto` or `discard` again, or any command that changes the project, finishes it first; it is never rolled back ([decision 049](decisions/049-writing-files-one-at-a-time.md)). If the switch's journal, `.opalro/state/journal`, can't be used (it is damaged, was made for another save than the one the project is on, or names a diary entry that can't be read), the message says so instead, with the same words the refusal uses: commands that change the project refuse to run while it is there, since the folder may be half switched, and deleting it lets them go on with the files as they are (a later `save` records the folder as it is then). Commands that only read check just the journal's front, which says between which entries the switch runs ([decision 079](decisions/079-red-team-m4-rulings.md)), so damage past it shows only when a command that changes the project reads it whole and refuses it. A switch stopped just after it took effect, with only its journal left, gets a message of its own: its files are all written, and the next command that changes the project removes the journal first (`undo`, `redo` and `unshelf` then take no step of their own, [decision 075](decisions/075-crash-recovery.md)). A command whose own switch, or a stopped switch it finished first, was done but couldn't remove the journal says so in the same kind, with why, and goes on: once `head` has moved nothing fails a command. Before the same run moves `head` again (the `save` after a switch it finished, the next step of `undo 2`), that journal is replaced by the step's own or removed, and a step that can't do so stops before it changes anything. The journal is then left for the next command, unless only the flush after removing it failed: then it is gone, and only a power cut could bring it back, for the next command that changes the project to remove. A journal never outlives the head it was finished at. With `head` lost (§5) there is no such warning: the journal can't be judged. |
| `switch_finished` | `save`, `goto`, `discard`, `undo`, `redo`, `shelf`, `unshelf` and `tidy` | Before doing its own work, the command finished a switch between saves that was stopped. The message counts the files it wrote and the entries it removed. It is given even when the command then fails, since the switch is done. When only the switch's journal was left, it is removed without a warning. |
| `diff_timed_out` | `changes` | Comparing a file line by line took more than 2 seconds, so Opalro stopped and shows its changes in bigger pieces than they could be (its `timed_out` is `true`). One warning for each such file, naming it. |
| `elevated_only` | `start`, `import`, on Windows | The project belongs to the Administrators group, so it opens only from an administrator terminal unless its folder is added to `safe-folders`. |
| `first_save_not_read` | `start` | The project was made, but Opalro couldn't read its first save back, so `save` is `null`. The message says why, and to run `opalro history` to check that the project reads. |
| `nested_opal_folder` | `import` | Git checked out a folder named `.opalro` below the top of the project (one in its current commit). Opalro left it out of history, but Opalro commands run inside it would take it for a project. Each of the first 100 gets a warning naming it; one more warning counts the rest. A folder Git holds at two paths (one tree used twice) is named at the first only. |
| `grafts_ignored` | `import` | The Git repository has a grafts file (`.git/info/grafts`), which changes the parents Git shows for some commits. Opalro ignored it and imported the commits as they are. |
| `duplicate_change_id` | `import` | Some commits have a `change-id` header (Jujutsu's) whose change ID another commit in the history has too, from the same header or from its own commit ID. Two saves never share a change ID, so such a header's ID went to one save at most, and each of the other commits took the change ID its own commit ID gives, as a commit without the header does. The same repository always gets the same IDs. The message counts them. |
| `config_includes_ignored` | `import` | The Git repository's config (`.git/config`, or `config.worktree` beside it when `extensions.worktreeConfig` is set) has an `[include]` or `[includeIf]` section. Opalro doesn't read the files they name, so it can't tell whether they turn on sparse checkout, which Opalro otherwise refuses. If one does, Git left some files out of the folder, and the first save would record them as deleted; they stay in history and can be brought back. The message says how to check (`git config core.sparseCheckout`) and how to turn it off before saving. |
| `lfs_lookalikes` | `import` | Files look like Git LFS pointers but aren't Git LFS files, and Opalro imported them as they are ([decision 085](decisions/085-importing-git-lfs-files.md)): files of any size whose first 1,024 bytes (all of a shorter one) look like a pointer (not empty, a first line starting `version https://git-lfs.github.com/spec/v1` or its pre-release's `version https://hawser.github.com/spec/v1`, a line that is `oid sha256:` and 64 hex digits, and a line starting `size `), or that Git LFS's own parser reads as one, as they are or with each LF not already after a CR turned into CR LF, cut at 1,024 bytes. Git LFS's smudge filter reads only those bytes, after Git may have turned line ends into CR LF, which pushes bytes out of them, so Git LFS may check such a file out as the file a pointer there names, whatever its size ([decisions 094](decisions/094-red-team-m5-rulings.md), [097](decisions/097-red-team-m5-session-2-rulings.md) and [098](decisions/098-a-blob-is-also-judged-in-its-crlf-form.md)). The message names the first 5 by path and commit, and counts them all. |
| `rules_file_skipped` | `save`, `setup` without `--preset`, `start` | The folder's `opalro.toml` couldn't be used at all (it is a link or a folder, is over 1 MiB, can't be read, or isn't TOML), so Opalro's own values apply: files are chunked from 1 MiB, and a save asks first over 100 MB or 10,000 new files ([decision 114](decisions/114-bolt-toml-keys-and-how-bolt-reads-it.md)). The message says what is wrong with it. |
| `rules_ignored` | `save`, `setup` without `--preset`, `start` | `opalro.toml` holds keys Opalro passed over, each named with why: a value of the wrong type or out of range, a rule without usable `files`, or a key this Opalro doesn't know, which a newer Opalro may have written. Opalro's own value applies for each. The warning names the first 20 and says how many more there are. |
| `diary_end_unreadable` | `tidy`, `verify` | A diary other than the project's own (one cut off from the current entry) can't be read back to its start: an entry it names isn't in the store. What can be reached from it is kept, and the command went on: `tidy` kept it, and `verify` counted it among what the project needs, telling the warning once ([decision 123](decisions/123-follow-ups-done-or-moved-past-v0.1.md)). |
| `switch_dropped` | `undo --to` | The project's `head` was lost while a switch was unfinished. Picking an entry up removed that switch's journal, which no command could finish any more: the files it had already written show as unsaved changes, and what it would have written over is kept in the switch's own saves ([decision 123](decisions/123-follow-ups-done-or-moved-past-v0.1.md)). |
| `info_exclude_ignored` | no longer given | Until 2026-11-04, `import` said that Git's personal exclude list, `.git/info/exclude`, had patterns Opalro didn't use. Since [decision 047](decisions/047-git-personal-ignore-lists-read.md) a save reads it, and the user's own Git ignore list too, so there is nothing to say. |

## 5. Errors

A command that fails before it has printed anything gives a document of just this, with exit code 1:

```json
{"opalro_json":1,"error":{"hint":"Run `opalro start` to make this folder an Opalro project.","message":"Not inside an Opalro project."},"warnings":[]}
```

| Field | Type | Meaning |
|---|---|---|
| `error.message` | string | What happened and why. |
| `error.hint` | string or `null` | What to do next, if there is something to say. |
| `warnings` | list | What the command had found before it failed, such as a damaged pack its store noted while reading: §4. `[]` if nothing. |

**A lost `head`.** `.opalro/state/head` names the newest diary entry, and every command reads it. When it is missing, isn't a diary entry's ID and a newline, or names an entry the store doesn't hold or can't read, every command fails with this document, read-only ones included, and changes nothing ([decision 075](decisions/075-crash-recovery.md)): `error.message` starts `Opalro can't tell which diary entry is the project's current one: .opalro/state/head` and says which of these it is, and `error.hint` says that the saves are safe and names the two commands that help. Those two are the exceptions: `opalro undo --list` then lists the entries where the diary may have stopped, and `opalro undo --to <entry>` makes one of them current (§3). A link or special file at `.opalro/state/head` is damage instead, as anywhere in `.opalro`.

`history`, `show`, `changes` and `undo --list --all` print their documents as they read, so they can fail after output has begun. They then end the document instead of cutting it off: the list that was open is closed with what was read so far, and `error` (the same object) and `warnings` are added, with `save` too for `show` and `changes`. The exit code is 1, and the output is still one valid document. **A document with `error` is incomplete:** check for `error` before using `saves`, `changes` or `entries`.

Stdout itself may not take what a command prints there, its document included: a write fails (a full disk, say), or its reader has gone (`opalro history --json | head -c 100`). The command then stops printing, and what it had printed is cut off. What it meant to say goes to stderr instead, as text, in this order:

1. Its own error, if it failed: `error:` with the message, then `hint:`. An `undo N` or `redo N` that stopped part way then gives the steps it took before stopping, as its text output says them.
2. Its warnings, `warning:` with each message, each once: what it had found by then. A command that prints as it reads finds most of its warnings at the end, so one stopped early gives only those it had found, such as `history`'s `switch_unfinished`.
3. Only when a write failed, not when the reader has gone: `error: Opalro couldn't write its output:` and why. A command that finished and changed the project did its work all the same, and the hint says so.

The exit code is the command's own: 0 when it succeeded and only the reader went, 1 when it failed or a write failed. A line stderr can't take is lost, and never stops Opalro: its reader may have gone too (`opalro status 2>&1 | true`).

```text
{"opalro_json":1,"position":{"line":"main"},"saves":[ …the saves read so far… ],"error":{"hint":null,"message":"…"},"warnings":[]}
```

`undo` and `redo` can fail after taking some of the steps asked for, and `undo --to` after some of the steps on its way. Their error document then also has `undone`, `redone` or `steps`, and `shelved`, listing what was done before the failure.
