# 107 — A saved file is matched against the ignore rules only when it changed

- **Status:** Accepted
- **Date:** 2026-11-10
- **Agenda topic:** none (milestone 6, speed and space)
- **Decided by:** Claude, at the owner's request while the owner was away. The owner can reverse any part.
- **Refines:** [decision 038](038-gitignore-and-saved-files.md) (what an ignore rule means for a file the last save already holds)

## Context

Decision 038 says what is saved stays saved: an ignore rule never drops a file the save a command builds on already holds. For such a file the rules decide only two warnings, and only when the file changed: "a change to a saved file that a rule matches" and "Bolt saves a file Git's rules alone would leave out".

Yet the walk matches every name against every rule in force, saved or not, on every `status`, `save` and `goto`. Once the wildcard lines were made cheap (milestone 6's Tasks 2 and 2b), a function-level profile of `status` on Git's source put the matcher at 42% of what the walk's thread still does: 31 of 73 million instructions, for 5,076 names of which 4,850 are files the last save holds and none has changed. On the same profile Git's own `status` takes about half of Bolt's time, and the third speed gate ([decision 099](099-a-third-speed-gate-within-twice-gits-time.md)) asks for at most twice.

## Options

- **A. Keep matching every name.** The gate's `goto` step on Linux then stays out of reach by measurement.
- **B. Match a saved file only when it turns out changed.** During the walk, a file or link the save holds under that name is kept without a look at the rules. If the scan then finds it changed, it is matched, by the same rules, read at the same moment of the walk, and the two warnings come out as before.
- **C. Make the matcher cheaper still** (sort the remaining wildcard lines by the last letter they can match). It helps new files too, but takes off about half of what B does for a project where nearly everything is saved, and can still be done later.

## Decision

**B.**

- **During the walk,** a name in a folder is kept without matching it against the rules when all of these hold: the save the command builds on holds that name in that folder as a file or a link (not a folder); the entry met is not a folder; and the folder isn't one that is kept only for what was saved in it (inside an ignored folder nothing is matched anyway). The names Git treats as `.git` and Bolt as `.bolt` are still checked first, as before, though a save never holds one.
- **Everything else is matched as before:** every folder, every name the save doesn't hold, every name whose kind changed between a folder and a file. So what a save takes in or leaves out is decided exactly as before.
- **When such a file turns out changed** (its kind or contents differ from the save's), it is matched then, against the rules that were in force in its folder: the walk keeps, for each folder it went into, the ignore files it had read on the way there, and whether the folder lies inside one Git's rules ignore or inside a repository of its own. The answer is the one the walk would have given: the rules are the same text, read once, during the walk.
- **The two warnings are unchanged,** word for word and path for path: they name the files they named before.

## Consequences

- An unchanged scan of a saved project matches only its folders and its new files. On Git's source that is about 230 names where it was 5,076.
- Nothing a user sees changes. A new ignore line shows its effect on saved files exactly when it did before: when one of them changes.
- The walk keeps a few shared pointers per folder (the ignore files in force there) until the scan is over.
- [design.md](../design.md) §5.9 says so. Tests: on random folders, rule files and changes, the two warnings name the same files as when every name is matched during the walk; the existing warning tests pass unchanged; a counter shows an unchanged scan matches no saved file.
