# Bolt `--json` Formats

> **Status:** written in milestone 2 (2026-11-02) for `start`, `save`, `history` and `show`, as [decision 017](decisions/017-commands-and-output.md) promised; `import` added 2026-11-03. It holds every document and every kind of warning these five commands print (checked against the code on 2026-11-03). Each command that comes later adds its document here in the milestone that builds it.

**In plain words.** Every Bolt command takes `--json`. Instead of text for people, it then prints one JSON document that a script, an editor plugin or the desktop app can read. This page says what is in each document, what each field means, and what `null` means where it can appear. The formats are a public interface: they are versioned, and the CLI tests keep them from changing by accident.

## 1. Rules for every document

- **One document, on stdout, in one line.** Nothing else goes to stdout: warnings and progress lines go to stderr, as text. `history` and `show` print their documents while they read, but what comes out is still one object. Don't depend on the order of its keys.
- **`bolt_json` is the version of the format,** a number, `1` now. It is in every document.
- **The versioning rule.** Within one `bolt_json` number, fields may be added, and kinds of warnings may be added. Fields are never removed, renamed, or given another type or meaning. Programs must ignore fields and warning kinds they don't know. A change that breaks the rule gets the next number. A field that can be `null` says so in its table, and says when: `start`'s `save`, for one, is `null` if the project was made but its first save couldn't be read back (a `first_save_not_read` warning says so). Check for `null` before you use such a field.
- **Exit codes.** `0`: the command did its work, though it may have warnings. `1`: it failed, and the document has `error`. `2`: the command line was wrong; the usage message is text, not JSON.
- **Text from the project is data.** Names, messages and paths are given exactly as they are stored, so a program can use them. They can hold anything, escape sequences included, so escape them before showing them in a terminal. Bolt's own messages (`warnings[].message`, `error.message` and `error.hint`) are already escaped.
- **Messages are for people.** Programs should go by `kind`, `result` and the other fields, because the words of a message can change.

## 2. Pieces used by several commands

**A save** (`save` in the documents below):

| Field | Type | Meaning |
|---|---|---|
| `id` | string | The save's fingerprint: 64 hex digits. |
| `change_id` | string | The save's change ID: 32 letters from `k` to `z`. |
| `change_id_short` | string | The shortest start of `change_id` (at least 4 letters) that no other save on the line starts with. The whole ID if the line couldn't be read to work it out; a `change_id_not_shortened` warning says so. |
| `message` | string | The message, as written. It can have several lines. |
| `author` | person | Who wrote the change. |
| `saver` | person | Who made the save. |
| `line` | string or `null` | The line the save was made on. `null` if the save doesn't say. |

**A person** (`author`, `saver`): `name` (string), `email` (string), `time_ms` (integer: milliseconds since 1970-01-01 UTC) and `tz_offset_min` (integer: minutes east of UTC in the person's time zone; negative is west).

**A warning:** `{"kind": …, "message": …}`, described in §4.

## 3. The documents

### `bolt start --json`

| Field | Type | Meaning |
|---|---|---|
| `bolt_json` | number | `1` |
| `root` | string | The project's folder. |
| `line` | string | The line the project starts on: always `"main"`. |
| `save` | save or `null` | The first save, "Start project". **`null` means Bolt made the project but couldn't read the save back.** The command still succeeds, and a `first_save_not_read` warning says why. |
| `warnings` | list | See §4. |

### `bolt import --json`

`bolt import` brings the Git repository in the current folder into Bolt, whole ([design.md](design.md) §5.16). Names from Git can hold bytes that aren't UTF-8; in this document each such byte sequence is U+FFFD.

| Field | Type | Meaning |
|---|---|---|
| `bolt_json` | number | `1` |
| `commits` | number | How many commits became saves. |
| `lines` | number | How many lines the branches became. |
| `servers` | number | How many servers have server lines. |
| `server_lines` | number | How many server lines there are, all servers together. |
| `tags` | number | How many tags were imported. |
| `position` | object | Where the project is now: `{"line": …}`, the line Git's current branch became, or `{"save": …}`, the fingerprint (64 hex digits) of the save Git's detached `HEAD` became. |
| `renamed` | list | Each name Bolt's rules don't allow, and the one it was given: `{"kind", "git", "bolt"}`. `kind` is `"line"`, `"server"`, `"server_line"` or `"tag"`. A server line's `git` is `<server>/<branch>` and its `bolt` is `<line>@<server>`. `[]` if nothing was renamed. |
| `skipped_refs` | object | The refs that weren't imported, such as `refs/stash`, `refs/notes/commits` and `refs/pull/…` (only branches, server branches and tags are): `count`, how many in all, and `first`, the whole names of the first 100 in byte order. |
| `skipped_tags` | list | Each tag that wasn't imported: `{"git": …, "reason": …}`, its Git name and why, such as a tag of a folder. |
| `skipped_entries` | object | Entries left out of saves because Bolt never writes their names to disk, such as spellings of `.git` and `.bolt`, with everything in them: `count`, how many in all, and `first`, the first 100 met, each `{"commit": …, "path": …}` (the Git commit in hex, and the path in it). A path can end in `/`, or be `""`, where the name left out is empty. |
| `inexact_commits` | number | How many commits a later export can't recreate with their Git IDs: their folders are in a form Git no longer writes, or entries were left out. |
| `warnings` | list | See §4. |

### `bolt save --json`

Saving gives one of two documents, told apart by `result`.

`"result": "saved"`: a save was made.

| Field | Type | Meaning |
|---|---|---|
| `bolt_json`, `result` | number, string | `1`, `"saved"` |
| `save` | save | The new save. |
| `changed_files` | number or `null` | How many files were added, removed or changed since the save before. It can be `0` when only folders changed. **`null` means Bolt couldn't count them.** The save was made all the same, and a `changed_files_not_counted` warning says why. |
| `warnings` | list | See §4. |
| `ignored_project_identity` | object | Only when present: see below. |

`"result": "nothing-changed"`: the folder is as the line's last save has it, so nothing was saved. The fields are `bolt_json`, `result`, `warnings` and, if it applies, `ignored_project_identity`.

**`ignored_project_identity`** is there only when the project's own settings (`.bolt/config.toml`) name a person other than the user. Bolt never takes who the user is from a project, since a project that was shared could name anyone, so the save was made as the user. The field says who was named, `{"name": …, "email": …}`, and is left out otherwise. For people, `bolt save` prints a note instead of the field. The note shows at most 256 characters of a name or email, and gives the lines to add to their own settings, if they do want that identity in this folder, only when nothing was cut: lines with a cut name would save as someone else. The field always has both in full.

**A save that stops.** Like any failed command, a save that fails prints an error document (§5) and exits with 1. One case is worth knowing after an import: a save made on top of an imported save stops, and saves nothing, when files differ from the imported version only in line endings, because Git turned LF into CRLF when it wrote them out. The `error.message` says how many files, and names the first few. The `error.hint` gives the two ways on: have Git write the files back as it stored them, or save them as they are with `bolt save --accept-crlf "message"`. Once a save has been made, later ones don't check.

### `bolt history --json`

| Field | Type | Meaning |
|---|---|---|
| `bolt_json` | number | `1` |
| `line` | string or `null` | The line the project is on. `null` if it is at a save instead of on a line. |
| `saves` | list of save | The saves of the line, newest first, following first parents back to the start. |
| `warnings` | list | See §4. |

### `bolt show --json`

`bolt show` takes a line name, the start of a change ID, or the start of a fingerprint (at least 8 hex digits), and shows the save the project is on when given none.

| Field | Type | Meaning |
|---|---|---|
| `bolt_json` | number | `1` |
| `save` | save | The save shown. |
| `changes` | list of change | What the save changed compared with its first parent (with all of its files for a save that has none), in the order of the paths. |
| `warnings` | list | See §4. |

**A change:**

| Field | Type | Meaning |
|---|---|---|
| `path` | string | The path from the project folder, with `/` between names. |
| `change` | string | `"added"`, `"removed"`, `"modified"`, or `"kind-changed"`: the same path became another kind of file, such as a file that became a link. |
| `kind` | string | `"file"`, `"executable"`, `"symlink"`, `"project-link"` or `"conflict"`: the kind after the change, or before it for a removed file. |
| `old_size`, `new_size` | number or `null` | Size in bytes before and after. **`null` means the path wasn't there on that side:** `old_size` for an added file, `new_size` for a removed one. A project link or a conflict has size `0`. |

Folders are not listed; the files in them are.

## 4. Warnings

`warnings` is in every document, the one for a failed command included, and is `[]` when there is nothing to say. A warning is something that went wrong without stopping the command. When a command finishes, people see each one on stderr as `warning: <message>`. A command that fails lists the warnings it had found before it failed in its document, and people are shown only the error.

| Field | Type | Meaning |
|---|---|---|
| `kind` | string | A code in snake_case that stays the same. |
| `message` | string | What happened, for people. Escaped. It can have several lines. |

| `kind` | Where | What it says |
|---|---|---|
| `damaged_pack` | any command that reads | A pack held a copy of something that failed its check. If another copy was good, Bolt went on with that one. If none was, the command fails and `error` says so, and the warning still names the pack. Ask for help before trying to repair the project. |
| `copy_unreadable` | any command that reads | A stored copy couldn't be read for a reason other than damage, such as a disk error, although another copy could. A failing disk can cause it. |
| `change_id_not_shortened` | `save`, `show` | Bolt couldn't read the line's older saves to work out the short form of a change ID, so the whole ID is given as `change_id_short`. |
| `changed_files_not_counted` | `save` | The save was made, but `changed_files` is `null`. The message says why. |
| `gitignore_skipped` | `save`, unless the save stops | A `.gitignore` isn't a regular file (it is a link, folder or special file), so Bolt skipped it and its rules didn't apply, as Git skips a linked one. The same goes for Git's personal lists where Bolt reads them without following links: `.git/info/exclude`, and the user's own list when it is inside the project; one of those is skipped too when a folder on the way to it, such as `.git`, is a link ([decision 060](decisions/060-user-git-settings-includes-and-paths.md)). The message names the file, and the linked folder if there is one. |
| `kept_as_they_were` | `save`, unless the save stops | The save kept entries of the last save exactly as they were, because this computer can't hold them as they are: project links, names this system can't create and names that are one file on this drive, whatever the folder holds at their paths, and links Git wrote as files holding exactly their targets. The message counts them. |
| `git_names_left_out` | `save`, unless the save stops | The save left out entries whose names Git treats as `.git`, the ones `bolt import` leaves out too (`.GIT` where letter case counts, `git~1`, spellings with characters macOS ignores, a `.git` file), with everything in them, wherever they were: those in the folder, and those the last save held, which the new save no longer has. The one exception is a folder kept as it was (see `kept_as_they_were`), which keeps everything the last save holds in it. Git's own `.git` folder isn't counted, nor is an entry an ignore rule keeps out that the last save didn't hold. Each is named once; the message names the first 5 by path, with every invisible character shown, and counts the rest. |
| `nested_projects_left_out` | `save`, unless the save stops | Folders below the top hold a Git repository or Bolt project of their own (an entry named `.git`, such as a worktree's or submodule's file, or a `.bolt` folder), so the save took nothing new from them, as Git and Jujutsu leave such folders out ([decision 046](decisions/046-nested-projects-left-out.md)). What the last save holds in such a folder stays saved, as in an ignored folder. A folder an ignore rule keeps out isn't counted. The message names the first 5 by path and counts the rest; it comes with every save while the folders are there. |
| `ignored_changes_saved` | `save`, when it makes a save | The save took changes to files or links that an ignore rule matches, which stay saved because the last save holds them: their contents or their kind changed. Git would leave such changes out unless the files were added (with `git add -f` if Git doesn't track them yet). Unchanged ones aren't counted, nor are new ones, which the rule keeps out, or deleted ones. The message names the first 5 by path and counts the rest. |
| `gitignored_files_saved` | `save`, when it makes a save | The save took files or links that the project's `.gitignore` files keep out, because a `.boltignore` line takes them back, or takes back a folder they are in: new ones, and ones whose contents or kind changed. Git would leave such changes out unless the files were added (with `git add -f` if Git doesn't track them yet), and a repository can track a `.boltignore` like any other file. Unchanged ones aren't counted, nor are deleted ones. The message names the first 5 by path and counts the rest. |
| `elevated_only` | `start`, `import`, on Windows | The project belongs to the Administrators group, so it opens only from an administrator terminal unless its folder is added to `safe-folders`. |
| `first_save_not_read` | `start` | The project was made, but Bolt couldn't read its first save back, so `save` is `null`. The message says why, and to run `bolt history` to check that the project reads. |
| `nested_bolt_folder` | `import` | Git checked out a folder named `.bolt` below the top of the project (one in its current commit). Bolt left it out of history, but Bolt commands run inside it would take it for a project. Each of the first 100 gets a warning naming it; one more warning counts the rest. A folder Git holds at two paths (one tree used twice) is named at the first only. |
| `grafts_ignored` | `import` | The Git repository has a grafts file (`.git/info/grafts`), which changes the parents Git shows for some commits. Bolt ignored it and imported the commits as they are. |
| `duplicate_change_id` | `import` | Some commits have a `change-id` header (Jujutsu's) whose change ID another commit in the history has too, from the same header or from its own commit ID. Two saves never share a change ID, so such a header's ID went to one save at most, and each of the other commits took the change ID its own commit ID gives, as a commit without the header does. The same repository always gets the same IDs. The message counts them. |
| `config_includes_ignored` | `import` | The Git repository's config (`.git/config`, or `config.worktree` beside it) has an `[include]` or `[includeIf]` section. Bolt doesn't read the files they name, so it can't tell whether they turn on sparse checkout, which Bolt otherwise refuses. If one does, Git left some files out of the folder, and the first save would record them as deleted; they stay in history and can be brought back. The message says how to check (`git config core.sparseCheckout`) and how to turn it off before saving. |
| `info_exclude_ignored` | no longer given | Until 2026-11-04, `import` said that Git's personal exclude list, `.git/info/exclude`, had patterns Bolt didn't use. Since [decision 047](decisions/047-git-personal-ignore-lists-read.md) a save reads it, and the user's own Git ignore list too, so there is nothing to say. |

## 5. Errors

A command that fails before it has printed anything gives a document of just this, with exit code 1:

```json
{"bolt_json":1,"error":{"hint":"Run `bolt start` to make this folder a Bolt project.","message":"Not inside a Bolt project."},"warnings":[]}
```

| Field | Type | Meaning |
|---|---|---|
| `error.message` | string | What happened and why. |
| `error.hint` | string or `null` | What to do next, if there is something to say. |
| `warnings` | list | What the command had found before it failed, such as a damaged pack its store noted while reading: §4. `[]` if nothing. |

`history` and `show` print their documents as they read, so they can fail after output has begun. They then end the document instead of cutting it off: the list that was open is closed with what was read so far, and `error` (the same object) and `warnings` are added, with `save` too for `show`. The exit code is 1, and the output is still one valid document. **A document with `error` is incomplete:** check for `error` before using `saves` or `changes`.

```text
{"bolt_json":1,"line":"main","saves":[ …the saves read so far… ],"error":{"hint":null,"message":"…"},"warnings":[]}
```
