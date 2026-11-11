# Known problems in Bolt 0.1

What is known not to work, or not to work well yet, and what to do about each. The user guide's section 13, What v0.1 doesn't do yet, has the full list of what is still to come.

## Not there yet: a server, merging, locking, repair, export to Git

Bolt 0.1 is for one person on one computer. It can't share a project, combine two lines (a line is Bolt's word for a branch), lock files for a team, or repair a damaged project: `bolt verify` finds damage but can't fix it. It can bring in a Git repository, but can't yet turn a project back into one; the import never changes the Git repository, so you can carry on in Git.

**What to do:** keep another copy of anything that matters, such as a copy of the whole project folder, `.bolt` included. If `bolt verify` reports damage, go back to that copy, and please tell us.

## Going back to another save is slower than Git on Linux

`bolt goto` takes about twice Git's time on Linux, which is over Bolt's own bar of staying within twice Git's time. Making it quicker is a goal for the next version.

**What to do:** nothing; it is only slower.

## A project made to cause harm can take minutes to be refused

A project someone else made on purpose, with a very deep, branching folder tree packed into a few kilobytes, is refused by `bolt goto` and `bolt changes --save`, but only after several minutes and hundreds of megabytes of memory. Nothing is written to your folder.

**What to do:** in a project someone gave you, run `bolt verify` first. It finds such a save in a moment and says so. On a project built to cause harm, `bolt verify` itself may take a while, but it changes none of your files or saves.

## Files another program has open, on Windows

On Windows, a program that holds a file open (an editor, a game engine, a preview) stops Bolt from writing over it. The commands that write files in your folder (`bolt goto`, `bolt discard`, `bolt shelf`, `bolt unshelf`, `bolt undo`, `bolt redo` and `bolt setup --preset`) write everything else, name the file, and stop.

`bolt save` stops too, when the program doesn't let others read the file (Bolt says "Can't read" and the file's name), or writes to it while Bolt reads it (Bolt says it "changed while Bolt was reading it").

**What to do:** close that program. After a `bolt save`, save again; after any other command, run any command that changes the project, and it finishes what was left.

## Windows and macOS warn about Bolt

The programs aren't signed yet, so either system may warn the first time Bolt runs. The warning means only that nobody it knows vouched for the program.

**What to do:** check the download against `SHA256SUMS`, then follow the user guide's section 2, Installing, part "Windows and macOS warn about Bolt".

## Online-only files of sync tools other than OneDrive

Bolt works in a OneDrive folder. Other sync tools may show Bolt a placeholder, a file kept online with nothing on your disk yet. Bolt can't read one, so `bolt save` and `bolt status` stop at it.

**What to do:** make the project's files available offline in the sync tool (often "Always keep on this device" or "Make available offline"), then run the command again.

## USB sticks, SD cards and other FAT or exFAT drives

On Windows, a project on these drives can't be used in this version. They don't record who owns a folder, and Bolt can't tell their folders apart, so `safe-folders` can't vouch for one either. Bolt says "Bolt can't tell who owns the project at" the folder, "this drive doesn't record who owns folders", and that safe-folders can't vouch for a project on it. On Linux, Bolt was tried on both and works there; on macOS, it hasn't been tried yet. On every system, Bolt's last check before writing over a file compares fewer things on these drives than it can elsewhere.

**What to do:** on Windows, work on a copy of the project on your computer's own drive. Elsewhere, keep projects on your computer's own drive where you can.

## Some network shares

Some network shares (some NAS drives, Windows shares and WebDAV folders) give every folder the same identity. Bolt can't be sure which folder a `safe-folders` entry names there, so the entry doesn't count, and a project Bolt refuses there stays refused.

**What to do:** work on a copy of the project on a drive of your own.
