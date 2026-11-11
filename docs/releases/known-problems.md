# Known problems in Bolt 0.1

What is known not to work, or not to work well yet, and what to do about each. The user guide's section 13, What v0.1 doesn't do yet, has the full list of what is still to come.

## Not there yet: a server, merging, locking, repair

Bolt 0.1 is for one person on one computer. It can't share a project, combine two lines, lock files for a team, or repair a damaged project: `bolt verify` finds damage but can't fix it.

**What to do:** keep another copy of anything that matters, such as a copy of the whole project folder, `.bolt` included. If `bolt verify` reports damage, go back to that copy, and please tell us.

## Going back to another save is slower than Git on Linux

`bolt goto` takes about twice Git's time on Linux. Bolt's aim is to stay within twice Git's time everywhere; this is a goal for the next version.

**What to do:** nothing; it is only slower.

## A project built to be refused can take minutes to be refused

A project someone else made on purpose, with a very deep, branching folder tree packed into a few kilobytes, is refused by `bolt goto` and `bolt changes --save`, but only after several minutes and hundreds of megabytes of memory. Nothing is written to your folder.

**What to do:** in a project someone gave you, run `bolt verify` first. It finds such a save in a moment and says so.

## Files another program has open, on Windows

On Windows, a program that holds a file open (an editor, a game engine, a preview) stops Bolt from writing over it. `bolt goto`, `bolt discard`, `bolt shelf` and `bolt unshelf` write everything else, name the file, and stop.

**What to do:** close that program, then run any command that changes the project; it finishes what was left.

## Windows and macOS warn about Bolt

The programs aren't signed yet, so either system may warn the first time Bolt runs. The warning means only that nobody it knows vouched for the program.

**What to do:** check the download against `SHA256SUMS`, then follow the user guide's section 2, Installing, part "Windows and macOS warn about Bolt".

## Online-only files of sync tools other than OneDrive

Bolt works in a OneDrive folder. Other sync tools may show Bolt a placeholder, a file kept online with nothing on your disk yet. Bolt can't read one, so `bolt save` and `bolt status` stop at it.

**What to do:** make the project's files available offline in the sync tool (often "Always keep on this device" or "Make available offline"), then run the command again.

## USB sticks, SD cards and other FAT or exFAT drives

These drives don't record who owns a folder, so Bolt refuses a project on them until you trust its folder. On them, Bolt's last check before writing over a file also compares fewer things than it can elsewhere.

**What to do:** keep projects on your computer's own drive where you can. For a project on such a drive that is yours, add its folder to `safe-folders` in your personal settings, copying the line Bolt's hint prints; the user guide's section 11, Personal settings, says how.
