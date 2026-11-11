# The Bolt user guide

This guide is for Bolt v0.1. It assumes you have never used version control before. If you have used Git, the most widely used version control system, read it anyway, then see [Coming from Git](#9-coming-from-git).

The commands you can type along with in the practice project (from [section 3](#3-the-first-five-minutes) on) are run by a test, in order, in one practice project, so they are commands Bolt really takes; the other examples, which need something the practice project doesn't have, are not. What Bolt prints is shown as a terminal shows it, without the colours; the change IDs (the short words like `kxqm`), times, names, sizes and counts will be your own. Where a terminal can't show symbols, Bolt prints plain ones instead: `->` for `→`, `<-` for `←`, `>` for `▸`, `*` for `•`, `...` for `…`, and `+` and `x` for `✓` and `✗`. The lines that start with `Next:` or `Changed your mind?` appear only in a terminal, not when a script runs Bolt. In a terminal, Bolt also wraps its longer sentences onto more lines, at 100 columns or at the window's edge if that comes first; here they are shown unwrapped.

## Contents

1. [What Bolt is](#1-what-bolt-is)
2. [Installing](#2-installing)
3. [The first five minutes](#3-the-first-five-minutes)
4. [Every command](#4-every-command)
5. [Presets, `bolt.toml` and `.boltignore`](#5-presets-bolttoml-and-boltignore)
6. [Undo, in depth](#6-undo-in-depth)
7. [Shelves](#7-shelves)
8. [Lines](#8-lines)
9. [Coming from Git](#9-coming-from-git)
10. [Looking after a project](#10-looking-after-a-project)
11. [Personal settings](#11-personal-settings)
12. [Scripts](#12-scripts)
13. [What v0.1 doesn't do yet](#13-what-v01-doesnt-do-yet)
14. [Stopping](#14-stopping)

## 1. What Bolt is

Bolt keeps every version of your project: the code and the big files together, such as textures, models, sounds and videos.

- **Saves.** When you run `bolt save`, Bolt takes a photo of your whole project folder and keeps it. You can look at any save again, or go back to it.
- **Lines.** A line is a name, like `main`, for a row of saves, one after the other. You can start a second line to try an idea without touching the first.
- **Undo.** Bolt keeps a diary of every command that changed something. `bolt undo` takes back the last one, like Ctrl+Z in an editor. Before a command throws away work you haven't saved, Bolt keeps a copy of it, so undo can bring it back.
- **Big files.** Files of 1 MiB or more are cut into pieces, called chunks. When you change part of a big file, only the pieces that changed are stored again.

**What v0.1 is.** Bolt v0.1 is for one person on one computer. Everything stays in your project folder, in a hidden folder named `.bolt`. There is no server yet, so you can't share a project through Bolt or work on one with other people. Combining two lines into one is planned for v0.2, and syncing with a server and locking files for v0.3 (see [section 13](#13-what-v01-doesnt-do-yet)).

## 2. Installing

Everything starts from the page you downloaded Bolt from: it has the archives, the checksums and the user agreement. Bolt sends nothing over the network and never checks for updates, so new versions are announced on that page too.

### What each system needs

| Your system | What Bolt needs |
|---|---|
| Windows | Windows 10 or 11, or Windows Server 2016 or later, on a 64-bit Intel or AMD processor |
| Linux | A 64-bit Intel or AMD processor, and glibc 2.34 or newer (Ubuntu 22.04, Debian 12, Fedora 35, RHEL, AlmaLinux and Rocky Linux 9, and later) |
| macOS | macOS 11 or later, on Apple silicon or Intel |

glibc is the library almost every Linux program is built on; `ldd --version` prints its version on the first line. Nothing else needs to be installed: not Git, not a runtime, not a library.

### Download

On the page you downloaded Bolt from, take the archive for your system:

| Your system | The archive |
|---|---|
| Windows | `bolt-0.1.0-x86_64-pc-windows-msvc.zip` |
| Linux | `bolt-0.1.0-x86_64-unknown-linux-gnu.tar.gz` |
| macOS on Apple silicon (M1 and later) | `bolt-0.1.0-aarch64-apple-darwin.tar.gz` |
| macOS on Intel | `bolt-0.1.0-x86_64-apple-darwin.tar.gz` |

Each archive holds six files:

- the program, `bolt.exe` on Windows and `bolt` elsewhere;
- `README.md`, a first look at Bolt;
- this guide, `docs/guide.md`;
- `USER-AGREEMENT.md`, the user agreement: what you may do with Bolt;
- `THIRD-PARTY-LICENSES.html` and `THIRD-PARTY-LICENSES-RUST.html`, the notices for the code of other people that is built into Bolt.

### Check the download

The same page has a file named `SHA256SUMS`. It lists a fingerprint (a checksum) for each archive. If the fingerprint of the file you downloaded is the same as the one listed, the file arrived whole and is the file that was put on the page. Download `SHA256SUMS` into the same folder as the archive, then, in a terminal in that folder:

On Windows, in PowerShell:

```text
(Get-FileHash .\bolt-0.1.0-x86_64-pc-windows-msvc.zip -Algorithm SHA256).Hash
```

It prints a long string of letters and digits. Open `SHA256SUMS` in Notepad and compare it with the line that ends with the same file name (upper or lower case doesn't matter). `certutil -hashfile bolt-0.1.0-x86_64-pc-windows-msvc.zip SHA256` prints the same fingerprint, if you prefer the older Command Prompt.

On macOS, in Terminal (this is the Apple silicon archive; for an Intel Mac, use `bolt-0.1.0-x86_64-apple-darwin.tar.gz` in its place):

```text
grep bolt-0.1.0-aarch64-apple-darwin.tar.gz SHA256SUMS | shasum -a 256 -c
```

On Linux:

```text
grep bolt-0.1.0-x86_64-unknown-linux-gnu.tar.gz SHA256SUMS | sha256sum -c
```

`grep` picks the one line for your archive out of `SHA256SUMS`, and the other command checks the file against it. It prints the file's name and `OK`, such as `bolt-0.1.0-x86_64-unknown-linux-gnu.tar.gz: OK`. If it prints `FAILED` instead (or, on Windows, the two fingerprints differ), the file isn't the one that was put on the page: don't run the program, delete it and download it again. Anything other than the file's name followed by `OK` means the same, whatever else it says (a complaint that it found no checksum lines, for example, means the name you typed isn't in `SHA256SUMS`).

The fingerprints prove that the download wasn't damaged and matches what was put on the page. They don't prove who made it, because v0.1 isn't signed (below).

### Put `bolt` on your PATH

Your PATH is the list of folders your terminal looks in when you type a command's name. Unpack the archive and put the program in one of those folders.

- **Windows:** unpack the `.zip` (right-click it, then "Extract All"). Move `bolt.exe` to a folder of your own, such as `C:\Users\<you>\bin` (make the folder first if it isn't there). Open Start, search for "environment variables", open "Edit environment variables for your account", choose `Path`, then "Edit", "New", and add that folder. Open a new terminal.
- **macOS and Linux:** in a terminal, in the folder that holds the archive, unpack it, make a folder for your own programs, `~/.local/bin` (`~` is your home folder), and move `bolt` there. On Linux, for example:

  ```text
  tar -xzf bolt-0.1.0-x86_64-unknown-linux-gnu.tar.gz
  mkdir -p ~/.local/bin
  mv bolt ~/.local/bin/
  ```

  (Use your archive's name in the first line.) Then add that folder to your PATH, by adding a line to the file your terminal reads when it opens: `~/.zshrc` on macOS, whose terminal uses the zsh shell, and `~/.bashrc` on Linux, whose terminals mostly use bash. If you use another shell, it has its own file.

  ```text
  echo 'export PATH="$HOME/.local/bin:$PATH"' >> ~/.zshrc
  ```

  That is macOS; on Linux, write `~/.bashrc` in its place. Then close the terminal and open a new one. The other way, if you have administrator rights, is to put it in `/usr/local/bin`, a folder that is already on the PATH: `sudo mkdir -p /usr/local/bin`, then `sudo mv bolt /usr/local/bin/`. On Linux, don't take this way if `/usr/local/bin/bolt` is already there: another program named bolt (below) puts one there, and `mv` would replace it.

Check that it works, in a new terminal:

```text
bolt --version
```

It prints:

```text
bolt 0.1.0
```

**If it prints only a number,** such as `5.2.1`, another program named bolt comes first on your PATH: Puppet Bolt, a tool for running tasks on servers, installs a `bolt` command too. Put your folder ahead of it:

- **Windows:** in PowerShell, `Get-Command bolt -All` (or `where.exe bolt`) lists every `bolt` in the order Windows tries them. Windows searches the computer's Path (the system one) before your account's, so moving your folder up in your account's Path helps only when Puppet's folder (`C:\Program Files\Puppet Labs\Bolt\bin`) is in your account's Path too: then select yours, choose "Move Up" until it is above Puppet's, and open a new terminal. Two ways always work: run Bolt by its full path, such as `C:\Users\<you>\bin\bolt.exe`, or, with administrator rights, add your folder to the system Path ("Edit the system environment variables", then "Environment Variables", `Path` under "System variables") and move it above Puppet's there.
- **macOS and Linux:** the line you added above puts `~/.local/bin` first, ahead of Puppet's folders (`/opt/puppetlabs/bin`, and `/usr/local/bin` on Linux). If it still loses, a later line in the same file puts one of them in front: move your line below it, and open a new terminal.

### Windows and macOS warn about Bolt

v0.1 is not signed. Signing means a company or a developer buys a certificate and stamps the program with it, so that Windows and macOS can say who made it. Signing is planned for a later version. Until then, either system may warn you the first time.

The warning means only "nobody we know vouched for this program". It doesn't mean something bad was found. Check the download against `SHA256SUMS` first, as above; that is the check you can make yourself.

- **Windows.** A program unpacked with "Extract All" keeps the mark Windows puts on downloads. Started from a terminal, as this guide does, it usually runs with no warning. Started from Explorer, SmartScreen shows "Windows protected your PC": choose "More info", then "Run anyway". If Smart App Control is on (Windows 11), Windows blocks Bolt from Explorer and from a terminal alike, with no "Run anyway"; it can be turned off only in Windows Security. To remove the mark, in PowerShell, in the folder that holds it: `Unblock-File .\bolt.exe`.
- **macOS.** Unpacking with `tar` in Terminal, as above, leaves no mark, so nothing is blocked. If you unpacked by double-clicking the `.tar.gz` in Finder instead, `bolt` carries the download mark, and macOS blocks its first run, even from Terminal. To allow it, within about an hour of the block: open System Settings (System Preferences before macOS 13), then Privacy & Security (Security & Privacy in System Preferences); near the bottom it names `bolt` and offers "Open Anyway". Choose it and enter your password, then run `bolt --version` again. Control-clicking and choosing Open no longer works since macOS 15. Or, in Terminal, in the folder that holds it, remove the mark: `xattr -d com.apple.quarantine bolt`.

### Folders that sync (OneDrive and the like)

Bolt works in a project inside OneDrive: Windows keeps OneDrive's online-only files (placeholders, which stand in for files kept online) out of view of programs like Bolt, and the v0.1 archive doesn't change that. This was tried on a real OneDrive folder.

If another sync tool lets Bolt see a placeholder, Bolt can't read it yet, so `bolt save` and `bolt status` stop at it, as at any file Bolt can't read.

## 3. The first five minutes

### Tell Bolt who you are

Every save records who made it, so before your first project Bolt needs your name and email. They go in your personal settings file, `config.toml`, which is in a different folder on each system:

| System | The file |
|---|---|
| Windows | `%APPDATA%\bolt\config\config.toml`, such as `C:\Users\Ana\AppData\Roaming\bolt\config\config.toml` |
| macOS | `~/Library/Application Support/bolt/config.toml` |
| Linux | `~/.config/bolt/config.toml` (or `$XDG_CONFIG_HOME/bolt/config.toml`, if you have set that variable) |

Make the folder and the file if they aren't there, then open the file in a plain text editor. These lines do that, in a terminal:

- **Windows,** in PowerShell. Notepad asks whether to make the file: say yes.

  ```text
  mkdir -Force $env:APPDATA\bolt\config
  notepad $env:APPDATA\bolt\config\config.toml
  ```

- **macOS,** in Terminal. The `Library` folder is hidden in Finder, so these lines go there for you. If TextEdit shows a ruler above the text, choose Format, then Make Plain Text, so it saves plain text.

  ```text
  mkdir -p ~/Library/Application\ Support/bolt
  touch ~/Library/Application\ Support/bolt/config.toml
  open -e ~/Library/Application\ Support/bolt/config.toml
  ```

- **Linux:**

  ```text
  mkdir -p ~/.config/bolt
  nano ~/.config/bolt/config.toml
  ```

  `nano` shows its keys along the bottom: Ctrl+O then Enter saves, Ctrl+X leaves. Any other plain-text editor does as well.

Put these three lines in the file, with your own name and email, and save it:

```toml
[user]
name = "Ana Lima"
email = "ana@example.com"
```

The file is written in TOML, a plain text format of `key = value` lines, with a name in square brackets, such as `[user]`, starting a section. Text goes in quotes. If you ever write a Windows path in this file, put it in single quotes (`'C:\Users\Ana\work'`) or double every backslash (`"C:\\Users\\Ana\\work"`): inside double quotes a single backslash starts a special character, and Bolt can't read the file. Every command that needs your name then stops with a message such as:

```text
error: C:\Users\Ana\AppData\Roaming\bolt\config\config.toml is not a valid Bolt settings file: too few unicode value digits, expected unicode hexadecimal value
```

If you skip this step, `bolt start` stops and tells you, naming the exact file on your computer:

```text
error: Bolt needs your name and email to record who made each save.

hint: Add this to C:\Users\Ana\AppData\Roaming\bolt\config\config.toml (make the file, and its folder, if they aren't there):
      [user]
      name = "Your Name"
      email = "you@example.com"
```

[Section 11](#11-personal-settings) says more about this file.

### Start a project

Make an empty folder for practice, open a terminal there, and run `bolt start`. Bolt looks at the folder's file names and sizes (it never opens a file or runs anything in it) and asks what kind of project it is:

```text
Bolt can't tell what kind of project this is: it found no game engine's or language's project files, and 0 B of files.

What kind of project is this?

    1. Code     apps, websites, libraries
    2. Game     Unity, Unreal, Godot, big assets
    3. Custom   Bolt's plain defaults, no rule files   ← suggested

Choice [3]:
```

In a Unity project, for example, it says `Looks like a game project: found Unity files and 2.3 GiB of files.` and suggests Game. Type `1`, `2` or `3`, or the preset's name, then Enter. Enter alone takes the suggestion.

A **preset** is a starter set of rules for your project: which files to leave out, from what size files are cut into chunks, and when a save should ask before going ahead. [Section 5](#5-presets-bolttoml-and-boltignore) says what each one writes.

You can also name the preset yourself, which is what this guide does so that the test can run it:

```sh
bolt start --preset code
```

```text
Started a Bolt project in /home/ana/practice

Using Code settings:

    • Files of 1 MiB or more are stored in chunks, so a changed part is stored once
    • .env files are left out
    • A save that adds over 100 MB or 10,000 new files asks first

    The rules are in bolt.toml and .boltignore. See them any time with: bolt setup

Your first save is wxyn on line main.

Next: change some files, then run: bolt save "describe your change"
```

Bolt made three things:

- a hidden folder `.bolt`, where it keeps everything it stores;
- a first, empty save named "Start project", on a line named `main`;
- the two rule files of the Code preset, `bolt.toml` and `.boltignore`, which are ordinary files you can open and edit.

`wxyn` is the save's **change ID**: a short name made of the letters k to z. Yours will be different.

### Save

Make a file, then save. The words in quotes are the save's message: a few words that say what changed.

```sh
echo "hello" > notes.txt
bolt save "First notes"
```

```text
Saved kxqm "First notes" on main  (3 files)
```

Three files: `notes.txt`, and the two rule files `bolt start` wrote. There is no step before saving: `bolt save` takes every change in the folder, new files included.

### See where you are

```sh
bolt status
```

```text
On line main at save kxqm "First notes"  (just now)
Nothing changed since then.
```

### Change something

```sh
echo "hello again" > notes.txt
bolt status
```

```text
On line main at save kxqm "First notes"  (just now)

Changed since then (1 file):

    modified  notes.txt  6 B → 12 B

Next: save them with: bolt save "describe your change"
```

To see exactly which lines changed:

```sh
bolt changes
```

```text
modified  notes.txt  +1 -1  (6 B → 12 B)
@@ -1,1 +1,1 @@
-hello
+hello again
```

A line starting with `-` was taken out, and one starting with `+` was put in.

### Save again, and look back

```sh
bolt save "More notes"
bolt history
```

```text
Saved ptvz "More notes" on main  (1 file)
```

```text
Saves on line main, newest first:

▸ ptvz  just now  Ana Lima  More notes  [main]
  kxqm  just now  Ana Lima  First notes
  wxyn  just now  Ana Lima  Start project
```

The `▸` marks the save you are at. `[main]` says it is the newest save on the line `main`.

### Undo

Changed your mind about that save?

```sh
bolt undo
```

```text
Undid save ptvz "More notes": main is back at kxqm; its changes show as unsaved.
```

The save is gone from the line, but your file still says "hello again": the change is unsaved again, ready to save another way or throw away. `bolt redo` would bring the save back. Every command that changes something can be undone this way.

## 4. Every command

`bolt help` lists them all, in three groups:

```sh
bolt help
```

```text
Bolt keeps every version of your project: code and big files together.

Usage: bolt <command> [options]

Every day
status    See what changed since your last save, and where you are
save      Save your changes, new files too
changes   Show your unsaved changes line by line
history   List the saves on your line, newest first
goto      Go to another line or save
undo      Take back your last step   (and: redo)

Lines, shelves and details
line      List your lines
shelf     Set your unsaved changes aside   (and: unshelf)
discard   Throw away your unsaved changes; undo brings them back
show      Show everything about one save

The project
start     Make this folder a Bolt project
setup     See or change the project's rules
import    Bring in the Git repository in this folder
tidy      Free the space nothing needs any more
verify    Check that everything Bolt stored is intact

Help on one command: bolt help <command>    Version: bolt --version
```

`bolt help <command>` shows one command's examples and every option it takes:

```sh
bolt help save
```

A few words come up again and again:

- **Unsaved changes:** whatever in your folder differs from the save you are at.
- **A step:** one command that changed something, as the undo diary records it.
- **A path:** a file or folder named from where you are, with `/` between folders, such as `Assets/Rock.psd`.

### `bolt status`

See what changed since your last save, and where you are.

```sh
bolt status
```

```text
On line main at save kxqm "First notes"  (just now)

Changed since then (1 file):

    modified  notes.txt  6 B → 12 B

Next: save them with: bolt save "describe your change"
```

Each row is the kind of change (`added`, `removed`, `modified`, or `kind changed` when a file became a folder or the other way round), the path, and the size: before and after for a modified file. If you are at an older save rather than on a line, the first line says `At save <id> "<message>", not on a line` instead.

To be fast, `status` re-reads only files whose size or time changed. `--full` reads every file, to be sure:

```sh
bolt status --full
```

### `bolt save`

Save your changes, new files too.

```sh
bolt save "More notes"
```

```text
Saved lnqt "More notes" on main  (1 file)
```

The save you undid at the end of the last section was taken off the line, so this is a new save, with a new change ID.

**Saving only some files.** Name them after the message: files, folders, or patterns such as `"*.png"` or `"art/**/*.png"` (`*` stays within a folder, `**` crosses folders). Everything else stays unsaved.

```sh
echo "buy milk" > todo.txt
echo "a game about bees" > idea.txt
bolt save "Add a to-do list" todo.txt
```

```text
Saved rmns "Add a to-do list" on main  (1 file)

    Only what you named was saved: bolt status lists the changes left out.
```

**Fixing the last save.** `--amend` replaces your line's newest save with one that also holds your changes, and keeps its change ID. Give a new message, or leave it out (or give `""`) to keep the old one. Here it takes `idea.txt`, which the save above left out:

```sh
bolt save --amend "Add a to-do list and an idea"
```

```text
Updated save rmns "Add a to-do list and an idea" on main  (2 files)

    The ID stays the same.
```

`bolt undo` puts the save back as it was before the amend.

**When there is nothing to save,** Bolt says so and changes nothing:

```text
Nothing to save: no changes since your last save, rmns "Add a to-do list and an idea".
```

**Big saves ask first.** A save that adds more than the project allows at once (100 MB or 10,000 new files with the Code preset) names the folders that hold the most and asks `Save them? [y/N]`. It is there to catch a cache or a render folder you didn't mean to keep. `--yes` saves without asking, for scripts; without a terminal and without `--yes`, nothing is saved.

**A message is needed.** `bolt save` with no message stops and saves nothing. Only `--amend` may leave it out.

**At an older save,** a save starts a new line, and `--line <name>` names it: see [Lines](#8-lines).

### `bolt changes`

Show your unsaved changes line by line.

```sh
echo "buy milk and eggs" > todo.txt
bolt changes
```

```text
modified  todo.txt  +1 -1  (9 B → 18 B)
@@ -1,1 +1,1 @@
-buy milk
+buy milk and eggs
```

Each file starts with a header: the kind of change, the path, how many lines were added and removed, and the size. Then come the changed parts, each with three lines around it for context. A file that isn't text (an image, say), a file over 64 MiB, or one of more than 1,000,000 lines shows its header only, with the reason, such as `(binary: no lines shown)`.

Name files or folders to look only there:

```sh
bolt changes todo.txt
```

`--save` shows what a save changed instead, compared with the save before it. Name the save by its line (for the line's newest save), its change ID, or 8 or more digits of its fingerprint. Here it shows the save made with `--amend` above, which added both files:

```sh
bolt changes --save main
```

Now save the eggs:

```sh
bolt save "Eggs too"
```

### `bolt history`

List the saves on your line, newest first.

```sh
bolt history
```

In a game project with a longer history, for example:

```text
Saves on line main, newest first:

▸ ptvz   2 h ago     Ana Lima    Add double jump  [main]
  kxqmz  3 days ago  Ana Lima    First playable
  kxqmr  5 days ago  Ben Okafor  Add rock textures
```

Each row is the change ID, when the save was made, who made it, and the first line of its message. The `▸` marks the save you are at.

### `bolt goto`

Go to another line or save. Name a line, a save's change ID as `bolt history` shows it (Bolt looks for change IDs among the saves of the line you're on), or 8 or more digits of a save's fingerprint.

```sh
bolt goto main
```

Here you are already there, so nothing changes, and there is nothing to undo:

```text
Already on line main, at save tqwl "Eggs too".
```

Going to an older save by its change ID writes that save's files into your folder. In the practice project, going back to the first notes would write `notes.txt` as it was then and remove `todo.txt` and `idea.txt`, which that save doesn't have. (If you are following along, you don't need to type this one: the examples after it assume you are still on `main`.)

```text
bolt goto kxqm
```

```text
Switched to save kxqm "First notes". You are not on a line.

    1 file written, 2 removed.
    Saving here asks for a new line's name.

Changed your mind? bolt undo
```

When the save you go to is the newest on a line, Bolt adds how to keep saving on that line, such as `To keep saving on main: bolt goto main`.

Your unsaved changes come along when they don't clash with what the other save holds. When they do, Bolt changes nothing and lists them: save them, set them aside with `bolt shelf`, or throw them away with `bolt discard`, then run `bolt goto` again. `bolt goto` never writes over unsaved work.

On Windows, if another program has a file open, Bolt writes the rest, names the file, and stops; close that program, and the next command that changes the project finishes the switch.

### `bolt undo` and `bolt redo`

Take back your last step; bring back the step you undid.

```sh
bolt undo
bolt redo
```

```text
Undid save tqwl "Eggs too": main is back at rmns; its changes show as unsaved.
```

```text
Redid save tqwl "Eggs too": main is at tqwl again.
```

`bolt undo 3` takes back the last three steps, and `bolt redo 2` brings back the last two.

**The diary.** `--list` shows the steps you can bring back, then the steps you can take back, numbered for `undo N` and `redo N`:

```sh
bolt undo --list
```

In a project named Skyrunner, for example:

```text
Undo diary for Skyrunner, newest first. Steps you can redo come first.

  redo 2  3f9a2c7e01b4  3 min ago   discard: 1 file changed
  redo 1  81c0de4a77f2  4 min ago   goto: on main at ptvz, 48 files changed
▸ you are here
  undo 1  c42a9b7e5510  2 h ago     save: main moved to ptvz "Add double jump"
  undo 2  7d0e34fa19c8  3 days ago  save: main moved to kxqmz "First playable"
          0a1b2c3d4e5f  3 days ago  start: the diary's first entry, which can't be undone
  and more: bolt undo --list --all shows the whole diary
```

The second column is the diary entry's ID. `--to` takes 8 or more of its digits and makes things as they were right after that entry:

```text
bolt undo --to 3f9a2c7e
```

**Unsaved work in the way.** Undo and redo never write over unsaved changes: if a step would, they stop and name the files. `--shelve` sets those changes aside on a new shelf first, as part of the same step:

```text
bolt undo --shelve
```

```text
Shelved your unsaved changes in the way on shelf 1.
Undid discard: 1 file back.

Next: bring your changes back with: bolt unshelf
```

[Section 6](#6-undo-in-depth) says what counts as a step and what undo can't bring back.

### `bolt line`

List your lines, marking the one you're on.

```sh
bolt line
```

In a project named Skyrunner, with two lines and a Git import behind it, for example:

```text
Lines in Skyrunner:

▸ feature/double-jump
  main

On the server (origin):
  main@origin
```

`Skyrunner` is the name of the project's folder. Lines "on the server" come only from a Git import in v0.1: they are the server branches Git last saw. See [Lines](#8-lines) for how a new line starts.

### `bolt shelf` and `bolt unshelf`

Set your unsaved changes aside, and bring them back later. Think of a shelf as a drawer: your folder goes back to how the last save has it, and your half-done work waits in the drawer.

```sh
echo "half an idea" > draft.txt
bolt shelf "Draft idea"
```

```text
Shelved your unsaved changes in 1 file on shelf 1.

Next: bring them back with: bolt unshelf
```

```sh
bolt shelf --list
```

```text
1 shelf, newest first:

    1  "Draft idea"  1 file  just now  on main
```

```sh
bolt unshelf
```

```text
Unshelved shelf 1 "Draft idea": 1 file written.

Changed your mind? bolt undo
```

`bolt unshelf` brings back the newest shelf; `bolt unshelf 2` brings back shelf 2. See [Shelves](#7-shelves).

### `bolt discard`

Throw away your unsaved changes. Changed files go back to how the last save has them, deleted files come back, and new files are removed. Bolt keeps a safety copy of everything it throws away, and `bolt undo` brings it all back.

```sh
bolt discard draft.txt
```

```text
Threw away your unsaved changes to draft.txt.

    A safety copy of it was kept.

Changed your mind? bolt undo
```

```sh
bolt undo
```

```text
Undid discard: 1 file back.
```

With no paths, `bolt discard` throws away every unsaved change in the project. Let's keep the draft this time:

```sh
bolt save "Add a draft"
```

### `bolt show`

Show everything about one save: the one you're at, or one you name by line, change ID or fingerprint.

```sh
bolt show
```

For a save in a game project, for example:

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

Next: see its lines with: bolt changes --save ptvz
```

The **fingerprint** is a save's full, permanent name, worked out from everything in it. `After` names the save before this one by the first 12 digits of its fingerprint, and its message. `bolt show` and `bolt goto` take those digits back whenever the history of one of your lines leads back to that save, which is nearly always. Only some saves brought in from Git are out of reach: the other side of a Git merge whose branch was gone before the import, or the save before one where the import left you away from every line.

### `bolt start`

Make this folder a Bolt project. See [the first five minutes](#3-the-first-five-minutes).

```text
bolt start
bolt start --preset game
```

- In a terminal, without `--preset`, it asks which preset to use. Without a terminal (in a script), it needs `--preset code`, `--preset game` or `--preset custom`, and starts nothing otherwise.
- In a folder that already has a `bolt.toml` (a copy of a teammate's project, say), it asks nothing and keeps those rules: `Using the rules in this folder's bolt.toml, which stay as they are:`. `--preset` is refused there; switch afterwards with `bolt setup --preset`.
- With Code or Game, a start is two entries in the undo diary: the start itself, which can't be undone, then the writing of the rule files, as `bolt setup --preset` writes them. So `bolt undo` right after a start takes the rule files away again (and gives back a `.boltignore` exactly as it was), and leaves the project.

### `bolt setup`

See or change the project's rules.

```sh
bolt setup
```

```text
Using code settings, from bolt.toml:

    Files stored in chunks  1 MiB or more
    A save asks first       over 100 MB or 10,000 new files
    Lockable                none
    No conflict markers     none
    Left out by the preset  4 lines in .boltignore

Next: edit bolt.toml or .boltignore to change a rule, or switch presets with: bolt setup --preset <name>
```

`bolt setup` alone changes nothing. `bolt setup --preset <name>` switches to another preset: it shows the exact lines it would change in both files, asks `Apply these changes? [y/N]`, and changes them only on a yes. `--yes` skips the question. See [section 5](#5-presets-bolttoml-and-boltignore).

### `bolt import`

Bring in the Git repository in this folder, with all its history. It runs inside the Git repository's own folder and takes no argument:

```text
bolt import
```

See [Coming from Git](#9-coming-from-git).

### `bolt tidy`

Free the space nothing needs any more.

```sh
bolt tidy
```

The practice project is new, so there is nothing to free yet:

```text
Nothing to free: storage holds 29 KiB.

    Left 18 packs changed in the last 24 hours as they are: a later bolt tidy frees what they hold that nothing needs.
```

A **pack** is one of the files in `.bolt` where Bolt stores your saves; each command that stores something writes one. `tidy` leaves every pack from the last 24 hours alone, which is why a new project has nothing to free.

In a project with something to free, it says how much it freed, such as `Freed 1.2 GiB  (3 packs removed, 1 written)`. See [Looking after a project](#10-looking-after-a-project).

### `bolt verify`

Check that everything Bolt stored is intact.

```sh
bolt verify
```

In a project named Skyrunner, for example:

```text
Verified Skyrunner: no problems found.

    ✓ 12,480 stored objects in 3 packs (4.8 GiB) match their fingerprints
    ✓ 1,290 saves, 3 lines and 2 shelves point to objects that exist
    ✓ the undo diary (24 entries) is intact
```

See [Looking after a project](#10-looking-after-a-project).

## 5. Presets, `bolt.toml` and `.boltignore`

A project's rules live in two text files at the top of the project:

- **`bolt.toml`** holds the settings: from what size files are cut into chunks, when a save asks first, and rules for some kinds of file.
- **`.boltignore`** lists the files and folders Bolt leaves out of saves, such as caches that a game engine makes again by itself.

They are ordinary files: they go into your saves like any other file, so a copy of the project carries its rules. Bolt also reads files named `.gitignore`, which projects that once used Git often hold: they list files to leave out in the same way, so such a project needs nothing more.

### What each preset writes

| | Code | Game | Custom |
|---|---|---|---|
| For | apps, websites, libraries, scripts | Unity, Unreal, Godot, big assets | Bolt's plain defaults |
| `bolt.toml` | written | written | none |
| Files stored in chunks from | 1 MiB | 1 MiB | 1 MiB |
| A save asks first over | 100 MB or 10,000 new files | 1 GiB or 10,000 new files | 100 MB or 10,000 new files |
| Marked lockable | nothing | art and audio, and each found engine's scenes and assets | nothing |
| `.boltignore` | build output and caches of each language found, and `.env` files | each found engine's caches, generated files, build output and crash files | none |

**Code** recognises Node, Python, Rust, Go, Dart, .NET, Maven and Gradle by their project files, and adds each one's lines. It always leaves out `.env` and `.env.*` files, which often hold passwords, but keeps `.env.example`.

**Game** recognises Unity, Unreal and Godot. It marks as lockable the art and audio that can't be merged (`*.psd *.png *.tga *.tif *.tiff *.exr *.jpg *.jpeg *.fbx *.blend *.max *.mb *.wav *.ogg *.mp3 *.mp4 *.mov`), then, for Unity, `*.unity` and `*.prefab` (lockable, and no conflict markers) and `*.meta` (no conflict markers); for Unreal, `*.uasset` and `*.umap`; for Godot, `*.scn` and `*.res`. Its lists follow one rule: when in doubt, the file is saved. They leave out what the engine makes again (Unity's `Library/`, `Temp/` and `Logs/`, Unreal's `Binaries/`, `Intermediate/` and `Saved/`, Godot's `.godot/` and `.import/`, and the like). They leave nothing out only for an extension that an asset can have: a `*.obj` model, a `*.exe` tool or a `*.log` file is saved. (They do leave out files that only a tool makes, by extension, such as the project files `*.sln` and `*.csproj` that Unity writes at the top of the project.) If you start with Game before the engine has made its project, no engine is found: run `bolt setup --preset game` once it has.

**Custom** writes no file. A project with no `bolt.toml` uses Bolt's own values. That includes an imported project, unless its Git history held a `bolt.toml` of its own.

"Lockable" and "conflict markers" are only recorded in v0.1: locking a file arrives with syncing, and conflict markers with combining lines. The presets write them now so a project has them from its first save.

### `bolt.toml`, key by key

This is the file the Code preset writes:

```toml
# Bolt project rules, written by the Code preset (version 1).
# This file is the project's rules: edit it freely. `bolt setup` shows how Bolt reads it.
preset = "code"

# A file of this size or more is stored in chunks, so a changed part is stored once.
chunk-files-over = "1 MiB"

# A save that adds more than this, in new bytes or in new files, asks first.
ask-before-saving-over = "100 MB"
ask-before-saving-files-over = 10000
```

| Key | Value | When it's missing | What it does |
|---|---|---|---|
| `preset` | text, at most 64 bytes | none | A label saying which preset wrote the file. Bolt shows it and never acts on it. |
| `chunk-files-over` | a size, from `"1 MiB"` to `"128 GiB"` | `"1 MiB"` | A file of this size or more is stored in chunks. It changes only how files are stored, never what a save holds. |
| `ask-before-saving-over` | a size | `"100 MB"` | A save that adds more new bytes than this asks first. |
| `ask-before-saving-files-over` | a whole number, 0 or more | `10000` | A save that adds more new files than this asks first. |
| `[[rule]]` | a table; as many as you like | none | Settings for the files its patterns match. |

Each `[[rule]]` has `files`, a list of one or more patterns written as in `.boltignore`, and either or both of these, each `true` or `false`:

- `lockable` (`false` when no rule says): the file can be locked, once locking arrives.
- `conflict-markers` (`true` when no rule says): whether a conflict inside the file is shown with markers, once combining arrives. Scene files that an engine must read whole are better off without them.

```toml
[[rule]]
files = ["*.unity", "*.prefab"]
lockable = true
conflict-markers = false
```

Rules are read from top to bottom, and for each setting the last rule that matches a file wins.

**Sizes** are a whole number, then a unit, in quotes: `B`, `KB`, `MB`, `GB` and `TB` count 1,000 bytes to the kilobyte, and `KiB`, `MiB`, `GiB` and `TiB` count 1,024. So `"100 MB"` is 100,000,000 bytes and `"1 GiB"` is 1,073,741,824. Spell the unit exactly so, with no fractions: `"1.5 GiB"` isn't a size, but `"1536 MiB"` is.

**When Bolt can't use the file, or a key in it,** it never stops: it warns, and uses its own value for whatever it couldn't read.

- If `bolt.toml` can't be used at all (it isn't valid TOML, it is a folder or a link, or it is over 1 MiB), Bolt passes over the whole file, uses its own values, and a warning says why.
- If one key can't be used (a value of the wrong kind or out of range, a rule without usable `files`, or a key this version doesn't know), Bolt passes over that key, uses the rest, and one warning names each such key.

`bolt save`, `bolt setup` and `bolt start` show these warnings; `bolt save` repeats them until the file is fixed. `bolt setup` always shows the rules as Bolt read them, so run it after you edit the file.

### `.boltignore`

A `.boltignore` is a list of patterns: names, with wildcards, of files and folders a save leaves out. (If you know Git's `.gitignore`, it works exactly the same way.)

- One pattern per line, such as `*.tmp` for every file whose name ends in `.tmp`. A line starting with `#` is a comment.
- `*` matches anything within one folder name; `**` matches across folders: `**/build/` is a folder named `build` at any depth.
- A `/` at the end matches folders only (`Library/`). A `/` at the start ties the pattern to the folder the file is in (`/Builds/`).
- A `!` at the start takes back a file an earlier line left out (`!.env.example`).
- A `.boltignore` in a subfolder applies to that folder.

Ignore rules keep out only files that **aren't saved yet**. A file that is already in a save stays in later saves even when a line matches it, until you delete it.

Unlike `bolt.toml`, a `.boltignore` Bolt can't read (a link, or a file over 1 MiB) stops a save with an error, rather than being passed over.

**The preset's block.** A preset writes its lines between two marker lines, at the top of `.boltignore`:

```text
# >>> Bolt preset: code, version 1. `bolt setup` rewrites the lines down to the end mark.
# Settings that hold secrets.
.env
.env.*
!.env.example
# <<< Bolt preset
```

Bolt owns the lines between the markers: switching presets rewrites them. Lines you add **outside** the block, below the end marker, are yours: Bolt never changes them, and since they come later they win over the preset's lines. If a `.boltignore` was already there when the preset was written, every line of it stays, below the block.

### Switching presets

`bolt setup --preset` shows each line it would change, in both files, then asks. With `--yes` it doesn't ask:

```sh
bolt setup --preset game --yes
```

It prints `Switching to Game settings changes these files:`, then each file's changes, as `bolt changes` shows them, then:

```text
Switched to Game settings: changed bolt.toml, changed .boltignore.

Changed your mind? bolt undo
```

Switching writes `bolt.toml` whole, so a rule you added there by hand is gone unless you add it again; the preview lists every line that goes, before it asks. In `.boltignore`, only the block changes. `--preset custom` removes `bolt.toml` and the block, and removes `.boltignore` if nothing is left in it.

The new files show as unsaved changes until you save them. `bolt setup` now shows the Game rules. A switch is one step, so `bolt undo` puts both files back exactly as they were, even edits you never saved:

```sh
bolt setup
bolt undo
```

## 6. Undo, in depth

**The diary.** Bolt keeps a diary of every command that changed something: each save, goto, discard, shelf, unshelf, undo, redo, preset switch, start and import is one entry (a start with Code or Game adds a second, for writing its rule files). Each entry records the whole state afterwards: every line, where you are, the shelves, and what the command wrote in your folder. Undo goes back to an earlier entry.

**What counts as a step.** One command that changed something is one step. `bolt undo` takes back the newest step not yet undone; `bolt redo` brings back the one you undid last. A new command clears what redo could bring back, as in an editor. Undo and redo are steps too, so they appear in the diary (`bolt undo --list --all` shows every entry), but each is the other's way back.

```sh
bolt undo --list --all
```

Commands that only look (`status`, `changes`, `history`, `show`, `line`, `setup` without `--preset`, `verify`) record nothing. `bolt tidy` isn't a step either: undo doesn't take it back.

**What undoing does.** Undoing a step puts the lines and your position back as they were before it, and writes back what the step changed in your folder, and only that. Other unsaved work comes along, and undo never writes over it: if it would, undo stops and names the files, and `bolt undo --shelve` sets them aside first. A step that changed no file, such as a save, changes none when undone: its changes show as unsaved again.

**Safety copies.** Before `bolt discard` or `bolt shelf` throws anything away, Bolt stores it, every changed, new or deleted file, as a hidden save of its own. That is how `bolt undo` brings back a file you discarded, even a new one that was never saved. `goto`, `undo`, `redo` and `unshelf` keep no copies because they never write over unsaved work: they stop instead. Stored copies cost little: content Bolt already holds isn't stored twice.

**The undo window.** Old steps stay undoable for 30 days, or as many days as your `undo-window-days` setting says ([section 11](#11-personal-settings)). Nothing is removed on its own: only `bolt tidy` frees the space, and it keeps everything that undoing or redoing a step inside the window needs. After a `tidy`, steps older than the window may no longer be undone.

**What undo can't bring back:**

- The diary's first entry, the start: it can't be undone.
- A step whose saves `bolt tidy` has removed, because it was older than the undo window.
- Anything Bolt never stored: a file it left out because an ignore rule matched it and it was never saved. `discard` and `goto` never touch such files.
- An unsaved link (a shortcut-like file that points elsewhere) that Bolt couldn't write back as a link: `discard` and `shelf` leave it alone and warn.
- Shelves you made or brought back by hand stay as they are when you undo other steps; only a step that itself added or removed a shelf changes the list.
- Anything that isn't in this project: undo changes only your copy.

**When something went wrong.** If a crash stops a command halfway, the next command that changes the project finishes it first, and says so. If Bolt can't tell which diary entry is the current one (a damaged `.bolt/state/head`), every command stops, saying your saves are safe; then `bolt undo --list` lists the entries it could have stopped at, and `bolt undo --to <entry>` makes one current without touching any file.

## 7. Shelves

A shelf holds unsaved changes you want out of the way for a while: to try something else, or to go to another save without carrying them along.

- `bolt shelf` sets every unsaved change aside, new and deleted files included, and puts your folder back to how your last save has it. Give it a message to remember what it holds: `bolt shelf "Half-done wall run"`. Without one, its message is `Shelved`.
- Shelves are numbered from 1. A new shelf takes the smallest number no shelf has, and a shelf keeps its number: numbers never shift when another shelf comes back.
- `bolt shelf --list` lists them, newest first, with the number, message, how many files, when, and whether it was made on a line or at a save.
- `bolt unshelf` brings back the newest shelf, and `bolt unshelf <number>` that one, onto whatever save you're on now. The shelf then leaves the list. Your other unsaved changes stay as they are.

```sh
bolt shelf --list
```

```text
No shelves: bolt shelf sets your unsaved changes aside on one.
```

If your folder already differs from the shelf's save at a path the shelf changes, `bolt unshelf` stops and changes nothing, since combining two versions of a file comes later. Save, shelve or discard what is there first.

Making a shelf and bringing one back are steps that `bolt undo` takes back. Shelving only some files, keeping a shelf while bringing it back, and dropping a shelf aren't built yet; to drop one, bring it back and then `bolt discard`.

## 8. Lines

A line is a name, like `main` or `feature/double-jump`, that points at the newest save made on it and moves forward each time you save. Every project starts with `main`.

**How a line starts.** You start a new line by saving somewhere that isn't the newest save of a line:

1. Find a save in `bolt history`, and go to it with `bolt goto` and its change ID. Bolt says you are not on a line.
2. Change some files.
3. Save with `--line` and the new line's name. In a terminal, a save without `--line` asks for the name (`Name a new line for this save:`).

```text
bolt goto kxqm
bolt save --line wall-run "Try wall running"
```

```text
Saved vzqk "Try wall running" on wall-run  (1 file)

    You're on the new line wall-run.
```

Now `bolt line` lists both lines, and `bolt goto main` and `bolt goto wall-run` move between them:

```sh
bolt line
```

**Names** may use letters, digits, `-`, `_`, `.` and `/`, with no spaces, and two names may not differ only in letter case.

**In v0.1** a line can be made, listed and switched to. There is no merging yet: two lines can't be combined into one, and lines can't be renamed or deleted. Those come in v0.2. While you are on a line, `--line` is refused.

## 9. Coming from Git

### Importing a Git repository

`bolt import` turns a Git repository's whole history into Bolt saves. Run it in the repository's folder, the one that holds `.git`:

```text
cd my-game
bolt import
```

```text
Imported 1,284 commits from Git: 3 lines, 2 server lines from 1 server, 12 tags.

    Renamed 2 names that Bolt's names don't allow:
        feature+x -> feature-x
        tag v1:0 -> v1-0
    Didn't import 1 ref (only branches, server branches and tags come in): refs/notes/commits
    Brought in 41 Git LFS files (11 GiB) from .git/lfs/objects.

You're on line main.

Next: look around with: bolt status
```

- **In place.** It makes `.bolt` next to `.git`, in the same folder. It never changes the Git repository, never runs Git, and never goes online.
- **What comes along:** every commit, exactly, as a save; your branches as lines; each server's branches, as Git last saw them, as server lines (`main@origin`); and tags. Names Bolt doesn't allow are renamed by a fixed rule, and the summary lists them. You end up where Git was: on its current branch's line, or at the save of a detached `HEAD`, where saving asks for a new line's name.
- **Work you hadn't committed** isn't refused: it shows as unsaved changes and goes into your next save.
- **Ignore rules** carry over by themselves, since Bolt reads `.gitignore` files.
- **Git LFS files** come in as ordinary files, read from `.git/lfs/objects`, which must hold every version the history names. Bolt never downloads them: first run `git lfs fetch --all`.
- **An import happens once.** Running it again is refused; bringing in Git's newer commits later comes after v0.1.
- **A project has no preset after an import.** Run `bolt setup --preset game` (or `code`) to give it one.

**What stops an import, and why.** Bolt treats every repository it is given as possibly built to cause harm, so it stops, saying why and what to do, rather than guess:

- Git LFS files it can't find: missing from `.git/lfs/objects` (it lists the first 20, and says to run `git lfs fetch --all`), kept somewhere else by Git LFS's settings, or damaged (it says to run `git lfs fsck` on the commit, then `git lfs fetch --all`).
- Git LFS files that need Git LFS to run a program to rebuild them.
- A repository that would take too much work to rebuild: a long history of small edits to one big file, or one built to make importing slow. The hint gives a Git command that packs it so Bolt can read it, if you trust it.
- Kinds of repository Bolt refuses, before writing anything: shallow or partial clones, sparse checkouts, the reftable format, and a `.git` that another account owns.
- A folder that isn't a Git repository, or one that is already a Bolt project.

If an import stops partway, the `.bolt` folder it began stays, and its message says how far it got. When nothing was committed yet, running `bolt import` again can finish from what it stored, or you can delete that folder. When the history was already committed, running `bolt import` again finishes the project.

**The first save after an import** may stop once:

- If Git changed line endings when it wrote your files (common on Windows), Bolt says how to have Git write them back as they are stored, or saves them as they are with `bolt save --accept-crlf "message"`.
- If Git LFS wasn't installed when the files were checked out, some files are only small pointer files. Bolt names them and says to run `git lfs checkout`, or `bolt discard` with those paths, which puts back the real files the import brought in.

### Git's words in Bolt

Bolt has its own words, but it accepts the few Git words people type most, so you aren't stuck: type them after `bolt`, as in `bolt commit -m "message"`. The first time you use one, Bolt runs its own command and then tells you its word, once (only in a terminal):

```text
hint: in Bolt, commit is save, which takes new files too: bolt save "message"
```

| In Git | In Bolt | `bolt` with the Git word |
|---|---|---|
| `git init` | `bolt start` | runs |
| `git status` | `bolt status` | the same word |
| `git commit -m "message"` | `bolt save "message"` | runs; also with `-a`, `-am` or `--message` |
| `git commit --amend` | `bolt save --amend` | runs |
| `git diff` | `bolt changes` | runs |
| `git log` | `bolt history` | runs |
| `git show` | `bolt show` | the same word |
| `git checkout <target>`, `git switch <target>` | `bolt goto <target>` | runs |
| `git stash`, `git stash push` | `bolt shelf` | runs |
| `git stash pop` | `bolt unshelf` | runs; `stash pop 2` runs `unshelf 2` |
| `git stash list` | `bolt shelf --list` | runs |
| `git branch` | `bolt line` | runs |
| `git fsck` | `bolt verify` | runs |
| `git gc` | `bolt tidy` | runs |
| `git restore` | `bolt discard <paths>` | explained, never run |
| `git add` | nothing: `bolt save` takes every change | explained |
| `git rm`, `git mv` | delete or move the file itself | explained |
| `git reset` | `bolt undo`, `bolt discard` or `bolt goto <save>` | explained |
| `git checkout -b`, `git switch -c`, `git branch <name>` | `bolt goto <save>`, then `bolt save --line <name> "message"` | explained |
| `git revert` | `bolt reverse`, in v0.3; until then `bolt undo` | explained |
| `git merge`, `rebase`, `cherry-pick`, `tag`, `blame` | coming in v0.2 | explained |
| `git clone`, `fetch`, `pull`, `push`, `remote` | coming in v0.3 | explained |
| `git clean` | none: Bolt never deletes files it doesn't keep | explained |

"Runs" means `bolt commit -m "message"` really saves. "Explained" means Bolt prints two or three lines saying what to type instead, does nothing else, and ends with exit code 2 (a number every command hands back when it ends, which scripts read: 0 means it worked; see [Scripts](#12-scripts)). For example, `bolt add` prints:

```text
Bolt has no staging step: `bolt save` saves every change, new files too.
To save only some files, name them: `bolt save "message" <paths>`
```

A Git word runs only in the forms that mean in Bolt what they mean in Git; any other form is explained, rather than run as something you didn't mean. So `commit` runs only with `-m` or `--message`, or with options alone (`bolt commit --amend`); `bolt commit "message"` without `-m`, or a path without a message, is explained. `stash` runs only as `stash`, `stash push`, `stash pop`, `stash pop <number>` and `stash list`; `stash apply` and the rest are explained.

**What is different from Git, on purpose:**

- **New files are part of the project.** There is no staging step. `bolt commit` (like `bolt save`) saves new files too, without an `add`.
- **`bolt stash` sets new files aside too,** and `stash pop` brings them back. Shelves are numbered Bolt's way, as `bolt shelf --list` shows them: from 1, and a shelf keeps its number. `stash pop 2` brings back shelf 2, which isn't Git's `stash@{2}`. The first `bolt stash` says so:

  ```text
  hint: in Bolt, stash is shelf, which takes new files too: bolt shelf
        A shelf keeps the number bolt shelf --list shows, counted from 1.
  ```
- **`restore` is explained, never run.** The command is `bolt discard`, and it removes new files too, which Git's `restore` never does. It keeps a safety copy first, and `bolt undo` brings everything back. Because it reaches further than Git's word, Bolt runs it only by its own name.
- **Undo covers everything,** including goto, discard and shelf, with one `bolt undo`.

## 10. Looking after a project

### `bolt verify`

`bolt verify` checks everything Bolt has stored, and that the project has everything it needs.

- **Stored objects:** every pack (the files in `.bolt/store/packs` that hold your saves) opens and reads in full, and every object in it matches its fingerprint. Each big file is rebuilt from its chunks and checked.
- **What the project needs:** every save, line, shelf and tag points to things that exist, down to every file's contents, and so does everything undo needs within the undo window.
- **The undo diary:** every entry back to the first is there and reads.

It doesn't check the files in your folder (`bolt status --full` reads those), or Bolt's caches, which it can rebuild.

When everything is fine it says `Verified`, as in [section 4](#bolt-verify), and ends with exit code 0. When it finds a problem it says `Checked` instead, lists each one, and ends with exit code 1:

```text
Checked Skyrunner: 3 problems found.

    ✓ 12,479 of 12,480 stored objects in 3 packs (4.8 GiB) match their fingerprints
    ✗ 1 object is damaged:
        4c2e9a01f7b3  It doesn't match its fingerprint. Save 1a2b3c4d5e6f needs it at Assets/Rock.psd.
    ✗ 2 objects the project needs are missing:
        b17f0d9e2a64  It isn't in any pack. Save 1a2b3c4d5e6f needs it at Assets/Scripts/OldInput.cs.
        9d0e44a1c3b2  It isn't in any pack.
    ✓ the undo diary (24 entries) is intact

hint: Bolt can't repair a project in this version. Saves that don't use the damaged or missing data still work; keep any other copy of this project safe.
```

Each problem names the object's fingerprint and, when a save needs it, which save and which file. At most 20 are listed of each kind; `bolt verify --json` lists up to 1,000.

**v0.1 can't repair.** With no server, there is nowhere to fetch a good copy from. Saves that don't use the damaged data still work, so you can still go to them and copy files out. Keep any other copy of the project (a backup) safe.

`bolt verify` changes nothing Bolt stored and takes no lock, so other commands can run while it checks. Like every command, it makes the two empty working folders `.bolt/tmp` and `.bolt/cache` if a copy of the project lost them. In a terminal, a bar shows how far it has read.

### `bolt tidy`

Bolt never deletes stored data on its own: undone saves, amended saves and the safety copies of old discards and shelves stay in `.bolt` until you run `bolt tidy`.

`bolt tidy` keeps:

- everything your lines, shelves and tags reach, with all their history, and the save you're at;
- every diary entry;
- everything undo and redo need for every step inside the undo window (30 days unless you set `undo-window-days`).

It frees the rest: saves nothing reaches any more, and what only steps older than the undo window needed. It says how much it will free while it works, then what it freed:

```text
Freed 1.2 GiB  (3 packs removed, 1 written)

    Left 2 packs changed in the last 24 hours as they are: a later bolt tidy frees what they hold that nothing needs.
```

**Recent packs.** As a grace period, `tidy` leaves every pack changed in the last 24 hours whole and untouched. What they hold that nothing needs is freed by a `tidy` a day later.

It writes every new pack before it removes an old one, so stopping it at any point loses nothing. If anything it must keep can't be read, it removes nothing and says what it found. `tidy` isn't a step: `bolt undo` doesn't take it back, and steps older than the undo window may not be undoable after it.

## 11. Personal settings

Your personal settings are yours alone: they stay on your computer, they never go into a project, and a project can't change them.

### Where the file is

The file is `config.toml`, in Bolt's settings folder:

| System | Folder |
|---|---|
| Windows | `%APPDATA%\bolt\config` (`%APPDATA%` is your roaming application data folder, usually `C:\Users\<you>\AppData\Roaming`) |
| macOS | `~/Library/Application Support/bolt` |
| Linux | `~/.config/bolt`, or `$XDG_CONFIG_HOME/bolt` when that variable is set to a full path |

If the environment variable `BOLT_CONFIG_DIR` names a folder, Bolt uses `config.toml` in that folder instead. When Bolt needs your name and can't find it, its message names the exact file it looked in.

The same folder holds two files Bolt manages itself: `cache-key`, a random key that seals Bolt's speed-up caches so that a cache someone else made is never trusted, and `git-words`, the Git words whose hint you have already seen.

### The keys

```toml
safe-folders = ["/home/ana/shared/skyrunner"]
undo-window-days = 60

[user]
name = "Ana Lima"
email = "ana@example.com"

[[project]]
folder = "/home/ana/work/client-game"
name = "Ana Lima"
email = "ana@studio.example"
```

- **`[user]`**, `name` and `email`: who you are. Every save records it.
- **`[[project]]`**: a different name and email for one project. `folder` is that project's folder, written as a full path. You can have several.
- **`undo-window-days`**: how many days of steps `bolt tidy` keeps undoable, a whole number from 1 to 36,500. Without it, 30.
- **`safe-folders`**: folders whose projects you trust although another account owns them (below).

`safe-folders` and `undo-window-days` must come at the top of the file, before `[user]` or any other `[section]`; TOML would otherwise read them as part of that section, and Bolt says so.

The environment variables `BOLT_USER_NAME` and `BOLT_USER_EMAIL`, when both are set, come before the file. A name or email written inside a project (in its `.bolt/config.toml`) is ignored, with a note, since a project you copied from someone else could otherwise make your saves claim to be theirs.

### `safe-folders`

**What it is for.** Bolt works in a project only when both the project's folder and its `.bolt` folder belong to you, the account you're logged in as. This protects you from a project someone else planted where you'd run Bolt: in a shared folder, or a folder above your own. (Git has the same rule.)

**When Bolt asks for it.** When a project's folder or its `.bolt` belongs to another account, or when the drive doesn't record who owns folders (USB sticks and SD cards often don't), Bolt refuses to use it:

```text
error: The project at /srv/work belongs to another user account.
```

and its hint offers the line that trusts that folder, and names your settings file:

```text
hint: If you trust it, add its folder to safe-folders at the top of /home/ana/.config/bolt/config.toml:
      safe-folders = ["/srv/work"]
```

Add the line only if you know the project is yours or someone's you trust. Each entry must be a full path, and names one exact folder: the project's own, not a folder above it. Copy the line Bolt's hint prints, rather than typing it: it is already written the right way, which matters on Windows, where a path in double quotes needs every backslash doubled (`"D:\\Shared\\game"`), or the whole path in single quotes (`'D:\Shared\game'`); otherwise Bolt can't read the settings file at all.

On Windows, a few more cases need it: a project directly in your home folder (`C:\Users\<you>`, which Windows gives to the SYSTEM account), and, outside a terminal run as administrator, a folder the Administrators group owns.

**Never list a folder that other people can write to.** Such as `/tmp` on macOS and Linux, or the top of a drive or of a network share on Windows. Anyone who can write there can put a project there, and Bolt would then trust it as yours. (A real attack on Git worked that way: CVE-2022-24765.) For such a folder Bolt doesn't offer the line at all: it says that other accounts can write there, and asks you to move the project into a folder of your own. Do that, rather than adding the line by hand.

**Some entries never count.** Bolt passes over these, and when it then refuses the project, its message names the entry and why:

- **An entry that is itself a link:** a symbolic link, or on Windows also a junction, a mounted drive or anything else Windows treats as a link to another place. Otherwise whoever could replace a trusted folder with a link could move your trust wherever the link leads. List the real folder. (A link in a folder *above* the entry is still followed: the rule is about the entry itself.)
- **An entry with `..` in it,** such as `/home/ana/shared/x/..`, whatever it leads to. Write the folder's path out in full.
- **Any entry, on a drive that doesn't tell its folders apart.** Some network shares give every folder the same identity, so Bolt can't be sure the entry is the project's folder, and `safe-folders` can't vouch for a folder there. Work on a copy on a drive of your own, or as the account that owns the folder.

## 12. Scripts

Every command takes `--json`: it then prints one JSON document on stdout instead of text, asks no question, and shows no colour, progress or `Next:` line.

```sh
bolt status --json
bolt history --json
```

A command that would ask (`bolt start`, a big `bolt save`, a save away from a line, `bolt setup --preset`) stops instead, and its error names the flag that answers the question: `--preset`, `--yes` or `--line`. Without a terminal, the same commands stop the same way even without `--json`.

Every command ends with one of three exit codes:

| Code | Means |
|---|---|
| 0 | Done, or nothing to do. |
| 1 | It failed, or `bolt verify` found a problem. |
| 2 | Bolt couldn't use the command line: an unknown command or option, `bolt` with no command, or a Git word Bolt explains instead of running. |

The `--json` reference, published on the page you downloaded Bolt from, describes every document, field and warning. Every document says its format's version in `bolt_json`, `1` in v0.1. Within one version, a field is never removed, renamed or given another meaning, though new ones may be added; a change that would break that gets the next version number. The words of messages and hints, which are for people, may change at any time.

## 13. What v0.1 doesn't do yet

- **No server, no sharing.** v0.1 is one person on one computer. Bolt can't yet copy a project from a server, or send and get changes (syncing is planned for v0.3).
- **No merging.** Two lines can't be combined, and lines can't be renamed or deleted (v0.2).
- **No locking.** Rules can mark files lockable, but nothing locks them until syncing arrives (v0.3).
- **No repair.** `bolt verify` finds damage but can't fix it.
- **No export to Git yet,** and an import happens once: bringing in Git's newer commits later comes after v0.1.
- **Not yet:** shelving only some files, dropping a shelf, a warning about files that look like secrets, and installers or package managers.
- **Not signed.** The programs aren't signed yet, so Windows and macOS warn about them ([section 2](#windows-and-macos-warn-about-bolt)).
- **Closed source.** Bolt's source code isn't published. You may use Bolt under the user agreement, `USER-AGREEMENT.md`, which comes in the same archive as this guide.

### Where Git and Git LFS still do better

Bolt is measured against Git and Git LFS on GitHub's Linux and Windows machines, doing the same steps on the same data. On Git's own source code, Bolt's first save takes a fifth of Git's time on Linux and a tenth on Windows, and a small save is level with Git on Linux and quicker on Windows. Git is ahead elsewhere:

- **`bolt status`** takes 1.6 to 1.8 times Git's time.
- **Going back to another save** (`bolt goto`) takes about twice Git's time on Linux.
- **On a slow disk,** Git LFS is quicker at the first save of big files that don't compress.
- **A long history takes more space** than Git's: 369 MB against 314 MB for Git's own history, since Bolt doesn't yet store a version as its difference from the one before.

Bolt's own bar is to stay within twice Git's time. v0.1 ships without meeting it for `bolt goto`: that is a goal for v0.2. `bolt status` with nothing changed and a small save stay held to it: Bolt's speed check fails when either takes more than twice Git's time.

### Reporting a problem

To report a bug or ask a question, write to the address on the page you downloaded Bolt from. Say which version `bolt --version` prints, your system, and what you typed and saw.

A security problem (anything that lets a project or repository harm the person using Bolt) goes to the same address, privately, with "security" in the subject. Please keep it to yourself until it is fixed or ruled on. `SECURITY.md`, published on the same page, says what to include and what counts.

## 14. Stopping

You can stop using Bolt at any time, and keep every file.

### Removing Bolt from a computer

Removing Bolt doesn't touch your projects: each keeps its files and its `.bolt` folder. To remove it, delete the program, take its folder off your PATH, and delete the settings folder of [section 11](#11-personal-settings), which holds your name and email and two files Bolt made for itself. (If you set `BOLT_CONFIG_DIR`, the settings are in that folder instead.)

- **Windows,** in PowerShell, if you put `bolt.exe` in `C:\Users\<you>\bin` as in section 2:

  ```text
  Remove-Item $HOME\bin\bolt.exe
  Remove-Item -Recurse $env:APPDATA\bolt
  ```

  Then take the folder off your PATH: open Start, search for "environment variables", open "Edit environment variables for your account", choose `Path`, then "Edit", select the folder and choose "Delete". Leave it if other programs live in it.

- **macOS,** in Terminal:

  ```text
  rm ~/.local/bin/bolt
  rm -r ~/Library/Application\ Support/bolt
  ```

- **Linux** (`$XDG_CONFIG_HOME/bolt` in place of `~/.config/bolt`, if you have set that variable):

  ```text
  rm ~/.local/bin/bolt
  rm -r ~/.config/bolt
  ```

  On macOS and Linux, if you put `bolt` in `/usr/local/bin` instead, the first line is `sudo rm /usr/local/bin/bolt`. Then take `~/.local/bin` off your PATH by deleting the `export PATH="$HOME/.local/bin:$PATH"` line from `~/.zshrc` or `~/.bashrc`, but only if you added that line yourself for Bolt and no other program lives in that folder.

### Leaving a project without losing its files

A project's files are ordinary files in an ordinary folder. Bolt keeps everything else in the hidden `.bolt` folder beside them.

**The folder holds only the files of the save you are on,** with your unsaved changes and the files Bolt leaves out (those stay where they are). Everything else lives only in `.bolt`, and goes when `.bolt` goes: every earlier save, the other lines and their saves, the newest saves of your line if you went back to an older one, your shelves, and everything `bolt undo` could take back. v0.1 can't combine lines or export to Git, so nothing carries that work out for you. Before you delete anything:

1. **Go to the files you want to keep.** `bolt line` lists your lines, and `bolt status` says where you are. Go to the line or save whose files you want in the folder, such as `bolt goto main`.
2. **Bring back your shelves.** `bolt shelf --list` shows them: bring each back with `bolt unshelf` and save it. If `bolt unshelf` stops, the shelf was made on another save and changes a file that differs here; it changes nothing, and the shelf stays safe in the copy of step 3, which you then must make. Then `bolt status` should say "Nothing changed since then"; if not, save.
3. **Keep the history.** Copy the whole project folder, `.bolt` included, somewhere safe. Do this whenever anything in the list above (an earlier save, another line, a newer save, a shelf) isn't in the folder now: after step 4, the copy is the only place it is. A later Bolt reads it: what v0.1 writes on disk never changes meaning.
4. **Delete `.bolt`.** The folder is then an ordinary folder, and `bolt.toml` and `.boltignore`, if the project has them, are ordinary files you may keep or delete. In the project's folder:

   ```text
   Remove-Item -Recurse -Force .bolt
   ```

   in PowerShell on Windows, or in a terminal on macOS and Linux:

   ```text
   rm -rf .bolt
   ```

There is no export to Git yet: to carry on in Git, start a new Git repository in the folder; its history starts there.
