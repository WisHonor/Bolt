# Welcome to Opalro

Thank you for trying Opalro.

## What Opalro is

Opalro keeps every version of your project, the code and the big files together: textures, models, sounds, videos. You save, look back, try ideas on a line of their own, and go back to any save. Almost every command that changes something can be taken back with `opal undo`. When a big file changes, only the parts that changed are stored again. It has its own engine; it is not built on Git, though it can bring in a Git repository.

## It is early

This is version 0.1, the first one people outside are given. It is for one person on one computer: there is no server yet, so no sharing or locking, and no merging yet either.

Opalro has been built to never lose or damage your files, and tested hard for it. Still, it is new: **keep another copy of anything that matters to you.** Opalro is not a backup yet. A copy of the project folder made before you start is enough.

"Known problems", published on https://opalro.com, lists what is known not to work well yet, and what to do about each.

## What to try

Anything you like. These four help most:

1. **A first save of a real project with big files.** A game, a film, a design project: whatever you have, as it is. How long did it take, and did `opal status` then say nothing changed?
2. **Undo after each kind of command that changes something.** After an `opal save`, an `opal goto`, an `opal discard`, an `opal shelf`, an `opal unshelf`, an `opal setup --preset`: run `opal undo` and check that everything is back as it was before that command (after a save, your files stay as they are and show as unsaved again). Then `opal redo`. Commands that only look, such as `opal status`, `opal history` or `opal line`, record nothing, so undo after them takes back the command before.
3. **An import of one of your Git repositories.** Run `opal import` in a repository you know well, best in a copy of it. Does the history look like Git's? Opalro adds its own `.opal` folder beside `.git` and never changes the Git repository. Git LFS files come in too, once `git lfs fetch --all` has fetched them.
4. **A project in a folder that syncs,** such as OneDrive, Dropbox or iCloud Drive. Save, change, go back, and see whether the sync tool and Opalro get along.

After each, `opal verify` in the project says whether everything Opalro stored is whole.

## How long it takes

Installing takes about ten minutes, with the user guide's section 2, Installing. The guide's section 3, The first five minutes, shows the everyday commands. Each of the four tries above takes from a few minutes to an hour, mostly waiting for a big project to save or import.

## Telling us what happened

Write to support@opalro.com, whether something broke or simply felt wrong. "Reporting a problem", published on the same page, says what to send. A security problem goes to the same address, privately, with "security" in the subject.

When something works well, that is worth hearing too.
