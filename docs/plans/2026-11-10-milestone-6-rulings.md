# Milestone 6 ("Presets and polish"): rulings

The owner asked for milestone 6's presets-and-polish half to be built start to finish while they were away, with every call made by Claude and written down. This file collects those calls. A second session built the other half (speed and space) at the same time and has [its own rulings file](2026-11-10-milestone-6-speed-rulings.md). This file has four parts:
- **Please confirm:** the calls that most change what Bolt does or what the owner signs up for. Each can be reversed. The license comes first, then milestone 5's list, which the owner hasn't read yet, then this milestone's calls.
- **Every ruling, verbatim:** each ruling as it was written in the build's ledger when it was made, task by task.
- **Where the design lost to a fact:** every line of the terminal-design canvas that was not built as drawn, with the fact that decided it.
- **Deferred:** what this half left for later, and where each item went.

The decisions themselves are records [113](../decisions/113-presets-what-each-writes-and-how-start-picks-one.md) to [127](../decisions/127-what-verify-holds-is-bounded-and-two-costs-are-written-down.md); 126 and 127 are the rulings after the red team's two sessions. Each says "Decided by Claude, at the owner's request while the owner was away. The owner can reverse any part."

## Please confirm

### First: the license

1. **Bolt has no license.** The owner's instruction left the license blank, so v0.1 is prepared with none: no `LICENSE` file, no `license` in the manifests, and the README and the release notes say "Bolt has no license yet." Published like that, nobody else has the right to use, copy or change Bolt. Choosing one is a `LICENSE` file, three manifest lines and one line of `deny.toml` ([decision 119](../decisions/119-packaging-v0.1.md)). **Please choose before publishing the draft release.**

### Carried from milestone 5, not yet read

The owner's instruction left "milestone 5's please-confirm list" unanswered, so it counts as not read yet. Its seventeen items are carried here unchanged, with their own numbers (this milestone's list goes on at 2 after them), and nothing in them was reversed. They are copied word for word from [milestone 5's rulings file](2026-11-08-milestone-5-rulings.md#please-confirm).

#### Decisions that set how big files work

1. **How files are cut** ([082](../decisions/082-how-big-files-are-cut-into-chunks.md)):
   - FastCDC, with chunks of at least 16 KiB, about 64 KiB, and at most 256 KiB.
   - Only files from 1 MiB to 128 GiB are cut. Bigger files are stored whole, as before.
   - These numbers freeze with format 1 at v0.1.
2. **How chunks are stored** ([083](../decisions/083-a-chunk-list-is-stored-under-its-files-id.md)):
   - A file's chunk list is a new record, kind 18, kept under the file's own ID, so no existing ID changes.
   - `.bolt/format` gains `feature chunks`. An older Bolt refuses a project that has it, rather than misread it.
3. **Several threads** ([084](../decisions/084-files-are-read-once-on-several-threads.md)):
   - 1 to 8 worker threads read and hash files.
   - One writer adds what they make to the packs, always in the same order.
   - At most 64 MiB of work is out at once.
4. **Git LFS import** ([085](../decisions/085-importing-git-lfs-files.md), [089](../decisions/089-git-lfs-objects-kept-elsewhere.md), [090](../decisions/090-the-first-save-looks-at-the-whole-folder-for-pointers.md)):
   - Bolt brings in Git LFS files only from `.git/lfs/objects`. It never fetches them and never runs Git LFS.
   - An import stops when a needed object is missing, damaged or stored elsewhere.
   - A pointer with extensions stops it too.
   - A file that only looks like a pointer comes in as text, with a warning.
   - The first save after an import stops at any pointer file left in the folder, and names `bolt discard` as the way out.
5. **Git's deltas rebuilt once** ([086](../decisions/086-rebuilding-gits-deltas-once.md), [091](../decisions/091-the-work-stop-names-a-way-out-for-long-histories.md) to [093](../decisions/093-a-family-rebuild-builds-every-member.md)):
   - The import rebuilds each big file's delta family once, keeping decision 044's work allowance.
   - A long, honest history of small edits to one big file can still pass the allowance. The stop now says so, and names a Git step that gets past it: `git -c core.bigFileThreshold=1m repack -a -d -f`.
   - Raising the allowance, or a "trust this repository" flag, are the alternatives if that stop turns out to be common.
6. **No write limit for imports** ([087](../decisions/087-what-an-import-writes-once-files-are-chunked.md)):
   - Chunking ends the disk blow-up for file contents.
   - It doesn't end it for commit messages and folder listings. RT4-1's repository still imports about 98 times bigger than Git keeps it.
   - Storing records as deltas, later, is what would end it.
7. **Benchmarks** ([088](../decisions/088-benchmarks-and-the-speed-gates.md), [results](../benchmarks.md)):
   - Both speed gates pass with a wide margin. Saving 10 GiB peaked at 43.5 MB on Linux and 48.7 MB on the laptop, against 500 MB. `status` on an unchanged 100,000-file project took 0.70 s and 1.16 s, against 2 s.
   - Git still wins `status` and small saves on small projects: 62 ms against 13 ms on git/git.
   - No stronger zstd level earns its time, so merging and `tidy` keep stored bytes as they are.
   - Confirm the fairness choices in §2: waiting out Git's background repack, a `sync` before every timed run, and files aged 3 s before `status`.

#### Red-team rulings

8. **The user's Git settings are read where Git reads them** ([094](../decisions/094-red-team-m5-rulings.md) to [097](../decisions/097-red-team-m5-session-2-rulings.md)):
   - the `GIT_CONFIG_*` variables, `GIT_CONFIG_SYSTEM` tidied as Git tidies it, and `/dev/null` naming no file;
   - the home folder found as Git finds it.

   Still not read, each with a follow-up row:
   - `[includeIf]` sections;
   - the system's own settings file when no variable names it;
   - `-c` options passed through `GIT_CONFIG_PARAMETERS`;
   - `GIT_CONFIG`.

   Where one of those may matter, the stop for missing Git LFS files says so.
9. **Files that Git LFS may check out as something else** ([094](../decisions/094-red-team-m5-rulings.md), [097](../decisions/097-red-team-m5-session-2-rulings.md), [098](../decisions/098-a-blob-is-also-judged-in-its-crlf-form.md)):
   - A blob that doesn't come in as a Git LFS file is named in a warning whenever Git LFS may read it as a pointer. That covers its own first 1,024 bytes, those bytes with CRLF line ends, and anything that looks like a pointer.
   - Bolt keeps the blob's own bytes.
   - The other way round stays a known limit: a padded pointer whose CRLF form Git LFS reads as text.

#### Other calls

10. **A test-only dependency.** `bolt-vcs` gained `blake3 = "1.8.7"` as a dev-dependency. It is the same crate and version the engine already uses, so `Cargo.lock` gains no package.
11. **Red-team checks kept as tests.** The red teams' Windows checks (`redteam8_windows.rs`, `redteam9_windows.rs`) passed on the fixed code, and stay in the suite as ordinary tests.
12. **Git LFS in the red-team sandbox.** The sandbox image gained `git-lfs` (one word in `redteam/Dockerfile`), so attacks can build Git LFS repositories.
13. **Known costs left as they are, each with a follow-up row:**
    - A switch stopped by damage holds the project until the store is fixed (since milestone 3).
    - A valid list of 8,388,608 one-byte chunks takes about 82 s to write out an 8 MiB file. That could change the format before v0.1 freezes it.
    - The git/git import peaks about 100 MB higher than before, most likely the Git reader's cache staying near its 256 MiB budget.
14. **Edits to accepted decision records:** only "Refined by" lines were added, to link the new records both ways, as decision 081 does. The records changed are 047, 060, 085, 086, 089, 092, 094, 095 and 097.
15. **Milestone 6's terminal UI:** the owner's design canvas was reviewed ([review](2026-11-09-terminal-ui-canvas-review.md)), with a prompt to redo it. It isn't yet known whether the owner has re-prompted.
16. **A sentence in decision 097 that the code narrows:**
    - Decision 097 says that with no home folder, "a setting that starts with `~/`" stops the command.
    - The code stops only on an include path, or a `core.excludesFile`, that starts with `~/`. It never expands `lfs.storage`, so that setting never stops it.
    - design.md §5.16 and storage-format.md §12 say exactly this. Accepted records aren't rewritten, so this note records the narrowing.
17. **One slip, already fixed.** The controller committed the second red team's proof file without a Windows lint check, so CI failed on `1b31dc6`. The fix (`f00e688`) made it build on every system.

### Decisions that set how presets and the polish work

2. **Presets** ([113](../decisions/113-presets-what-each-writes-and-how-start-picks-one.md)):
   - `bolt start` asks what kind of project this is (Code, Game, Custom) and suggests one from the names it sees in the folder; planted files can change the suggestion and nothing else. Without a terminal it stops unless `--preset` is given.
   - A preset is written out in full: rules in `bolt.toml`, and a marked block of lines in `.boltignore`. **Custom writes no file at all.**
   - The lines each preset leaves out come from GitHub's ignore templates, reviewed under "when in doubt, save it": no line that matches by an extension an asset can have, and not Unreal's `Build/`, plugin `Binaries/` or Node's `dist/`.
3. **`bolt.toml`** ([114](../decisions/114-bolt-toml-keys-and-how-bolt-reads-it.md)): the key names are final (`preset`, `chunk-files-over`, `ask-before-saving-over`, `ask-before-saving-files-over`, `[[rule]]`). Sizes are text with a unit. **A file or a key Bolt can't use never stops a command:** Bolt warns and uses its own value, so a typo is noticed only through the warning.
4. **`bolt setup`** ([115](../decisions/115-bolt-setup.md)): it shows the rules, and `--preset` switches preset as a step `bolt undo` takes back. A `bolt start` with a preset therefore makes two diary entries: the start, then the rule files.
5. **Git's words** ([116](../decisions/116-gits-words-as-shortcuts.md), [124](../decisions/124-restore-is-explained-and-a-shortcut-takes-nothing-for-what-it-isnt.md)):
   - Ten words run the Bolt command they stand for, only in the forms that mean the same (`commit -m`, `stash`, `stash pop`, `checkout <line>` and so on); the others are explained and exit with 2.
   - **`bolt restore` is always explained, never run:** Git's `restore .` leaves new files alone, and Bolt's `discard .` removes them (undoably).
   - `bolt commit` saves new files too and `bolt stash` sets them aside too, which Git doesn't; the hints say so.
   - **`bolt stash pop 2` uses Bolt's own shelf numbers** (the ones `bolt shelf --list` shows), which are not Git's positions.
6. **`--json` is frozen from v0.1** ([117](../decisions/117-what-json-covers-at-v0.1.md)): a field can be added later, never renamed or removed. Before freezing, one read of all the formats changed twenty things a script could notice so that one thing has one name: sixteen names or values renamed, one field's type, one field removed, one added, and error text escaped as on screen (the release notes list them all). **Errors have no machine-readable code in v0.1:** scripts go by the exit code.
7. **`bolt verify`** ([118](../decisions/118-bolt-verify.md)): it checks every pack and that the project has all it needs, takes no lock, and tells a problem with what the project needs only when two looks agree. **It can't repair anything in v0.1.** Like every command it makes the two empty working folders `.bolt/tmp` and `.bolt/cache` when a copy lost them; apart from that it changes nothing. The red team made it stricter (items 13 and 14).
8. **Packaging** ([119](../decisions/119-packaging-v0.1.md)): four archives (Windows, Linux, two for macOS) built by a workflow that only starts by hand and, for the release itself, only from `main`; a draft release, no tag, nothing published, **nothing signed** (Windows and macOS warn; the guide says what to do), no installer, no package-manager name claimed.
9. **The terminal** ([120](../decisions/120-the-terminal-ui.md)):
   - Colour on Windows comes through a crate Bolt already had (`anstyle-query`), so **no new `unsafe`** was needed. The terminal's width comes from **one new dependency, `terminal_size` 0.4.4**.
   - `Next:` lines, "Changed your mind?", hints for Git's words, progress, wrapping and cutting appear **only on a terminal**; a pipe or a log gets what it got before, one line per message.
   - Where the design and a decision disagreed, the decision won; the third part of this file lists every such place.
10. **The format freezes at v0.1** ([121](../decisions/121-format-1-freezes-at-v0.1.md)); `feature chunks` stays a feature line.
11. **Change IDs are written in Jujutsu's letters** ([122](../decisions/122-change-ids-in-jujutsus-letters.md)): display only, but every `change_id` a script kept from an earlier build reads differently now.
12. **The follow-ups** ([123](../decisions/123-follow-ups-done-or-moved-past-v0.1.md), [125](../decisions/125-a-safe-folders-entry-is-judged-by-one-look.md)): sixteen done, eight moved past v0.1 with reasons. Two that change behaviour:
    - **A `safe-folders` entry is judged by one look at the folder it holds.** A link doesn't count, an entry with `..` in it doesn't count, and **on a drive that gives no file numbers (some network shares) `safe-folders` can no longer vouch for a folder**, where comparing paths used to work. It fails toward refusing.
    - Picking up a lost `head` now drops a stopped switch's journal and says so, even when the pick-up then fails.

### Red-team rulings

Two sessions, each a fresh agent in the no-network sandbox ([first report](../redteam/2026-11-10-milestone-6.md), [second report](../redteam/2026-11-10-milestone-6-session-2.md)). Four proven findings, all Medium, all in `bolt verify`, all fixed; nothing Critical or High.

13. **First session: `verify` said "no problems found" about three planted projects that `bolt goto` refuses to write out. Fixed** ([126](../decisions/126-verify-also-finds-a-save-that-cant-be-written-out.md)). They were a chunk held only as a chunk list, a folder listing that gives a file a size it doesn't have, and a save past the 10,000,000-entry limit (13 KiB stored). The rule now: **what would stop a reader from writing out a save the project needs, because of what is stored, is a problem `verify` reports.** `--json` gains one problem kind, `save_too_big`. For a link, `verify` is now stricter than `goto` (it compares the listed size with the target's length); no project Bolt wrote can differ there.
14. **Second session: a small planted project could make `verify` take a gigabyte of memory. Fixed** ([127](../decisions/127-what-verify-holds-is-bounded-and-two-costs-are-written-down.md)). `verify` now keeps the details of at most the 1,000 problems of a kind that it tells, and counts the rest. Its memory still grows with the number of objects, as `tidy`'s does (a known limit), and no longer with what a planted listing repeats.
15. **Not fixed, and the one to look at: a planted project can keep `goto` busy for minutes.** A chain of 24 folder listings of two folders each (27 objects, 8 KiB) holds 2^25 files by path. `bolt changes --save` refuses it after 201 seconds, and `bolt goto` after 517 seconds and 551 MiB; nothing is written, and `bolt verify` tells it in a tenth of a second. Refusing at once means measuring a save by its listings before walking it, in every reader: a pass added to the walk the speed session tuned against its gate, so it was left for a design of its own after v0.1 ([127](../decisions/127-what-verify-holds-is-bounded-and-two-costs-are-written-down.md), [security.md](../security.md) T7). Until then, `bolt verify` is the way to check a project one was given. **Please confirm that v0.1 can ship with it.**
16. **A known limit, as Git's `safe.directory` has:** the project folder is judged by its path (its owner, and its identity for `safe-folders`) and then opened by its path again. Whoever can rename entries in a folder above the project controls it already ([security.md](../security.md) T23).
17. **Two costs written down, not bounded:** `verify` on a pack of objects that each claim a huge size and decompress to it (about 20 seconds for each megabyte planted), and very wide listings, which every reader holds whole. Both are rows of the follow-ups.
18. **What the sessions did not attack:** `safe-folders` with another account's folders or a drive without file numbers (the sandbox has one account), preset detection at a million names, and the release workflow by running it (it was read). Three checks that only Windows can run were made on `redteam/m6a-*` branches in CI and are kept as tests; their first runs failed through mistakes in the checks themselves (a wrong expectation, and junction paths written with forward slashes), not through Bolt.

### The two halves together

The speed half merged first, so this half merged `main` in and measured the two together ([benchmarks.md](../benchmarks.md) §9).

19. **The third speed gate is still not met on Linux, and milestone 6 was merged with it open.** On the combined code `goto` took 2.019 and 2.425 times Git's time in two runs on GitHub's Linux runner (the limit is 2); `status` and a small save pass there, Windows passes all three steps, and the memory and `status` gates pass with room. This is not something the merge brought: on the speed half alone the same step passed in two runs of seven and missed in five (1.79 to 2.50 times), and these runs sit inside that band. The instruction was "if a gate fails, find and fix the cause". The cause is found: a `goto` flushes ten times, each flush there for what a crash may leave, and Git's checkout flushes nothing ([benchmarks.md](../benchmarks.md) §8). It was not fixed here, for three reasons: flushing less changes what a crash can leave, which [decision 109](../decisions/109-the-speed-sessions-follow-ups-and-format-1s-three-open-rows.md) leaves to the owner; `goto`'s speed is the other session's to change; and the first principle puts never losing data before speed. **Please decide** between fewer flushes in a `goto`, the walk on several threads (a design and a red-team round of its own), or a gate that says what Bolt does today. Milestone 6 is marked done all the same, as the instruction says, and the lines that say so also say that v0.1 itself has not shipped: [milestones.md](../milestones.md) has v0.1 ship when all three gates pass.
20. **A `goto` that needs an object the store lacks now stops partway.** Before the speed work it looked for every object first and refused cleanly. Now it starts, stops at the file it can't write ("is not in the store"), and leaves the stopped-switch state decisions 049 and 075 designed, with the hint that names the way on. Files written before the stop are written; the one it couldn't write is untouched; `bolt status` works and warns. Looking first again would undo part of the speed work, so the new behaviour stands and the test pins it. It needs a damaged store to happen.
21. **The numbers are of two commits, not of the last one.** The whole benchmark and the import were measured on the merge commit, and the three gates there and again on the code as its final review left it. The two commits after that change one flag in `verify`'s report, how `bolt setup` shows a pattern, tests and documents, so nothing was measured a third time; the nightly workflow runs the gates on `main` every night.

### Calls made during the build

22. **`bolt` alone prints the overview on stdout and exits with 2**, as the code did before (it was clap's help on stderr). 0 may be friendlier; it is one number.
23. **The undo list's labels are `redo N` and `undo N`**, as the design draws them; decision 072 says the redo steps are "marked as undone", which their place above "you are here" still does.
24. **`bolt show` names the save before by 12 digits of its fingerprint**, not by a short change ID: shortening a change ID for a save that isn't a line's newest costs a walk of the whole line.
25. **`bolt history` sizes its columns from its first 1,000 rows or 1 MiB of text**, so its memory stays bounded; a later, wider row runs long.
26. **A `goto` to where the project already is says "Already on line …"** and offers no undo; the engine now tells the command whether a step was recorded.
27. **A sentence wraps under its own text on a terminal, and a project's text is never broken and never starts a line four spaces in**, so nothing a repository holds can pass for one of Bolt's own lines.
28. **A time in `.bolt/state/tidied` that is later than now counts as absent.** It keeps a planted file from shrinking the undo window to nothing; the cost is that a machine whose clock ran ahead during a `tidy` and was then set back sees `tidy` stop until the clock catches up, if the window was also raised.
29. **In `bolt history`, a save message that ends like a line marker (`  [main]`) can pass for one when colour is off.** Left as the design has it; Git has the same limit; `--json` can't be mistaken.
30. **The guide shows what a person sees on a terminal** (symbols, `Next:` lines), colour aside, and a test runs every command in its runnable blocks; the printed output in its examples is checked by reading, not by the test.
31. **A `bolt setup --preset` that wrote nothing says so and exits with 0.** When every rule file changed between its plan and its write, it writes none of them, says "Nothing was written" with the command to run again, and `--json` has `"applied": false`. Its step is in the undo diary all the same, as for any switch that left files alone; a `bolt undo` of that step stops rather than write over the files as they are now.
32. **`bolt verify`'s diary line is about the current diary.** A gap in it replaces the "intact" line. An entry that only a leftover diary, cut off from the current one, can't read is told as a problem or a warning and doesn't change that line.

### Other calls

33. **Packaging was built in a worktree and a branch of its own** and merged into the milestone branch through a pull request of its own (#26) once CI had built all four archives there; it reached `main` only with the milestone.
34. **No accepted decision record was edited.** Records 042, 116, 118, 123 and 126 gained or lengthened only their "Refined by" back-link line.
35. **No new `unsafe`.** The platform crate gained one safe function (`Dir::own_meta`, 14 lines, for decision 125); the `unsafe` blocks and their walkthroughs are as they were.
36. **Slips, already fixed.** Twice a test went red on Linux and macOS because of a wrong premise of the controller's or of a test's author that only CI on those systems could show (a link at `.bolt/state/tidied`; a file name with an escape character): each was a wrong test, not a bug, and each fix round says so in the ledger. Unix-only tests are never compiled on this Windows machine, so they are now pushed and watched before more work is stacked on them.
37. **What only the owner can do before v0.1 is out:** choose the license (item 1); turn on private vulnerability reporting in the repository's settings, which `SECURITY.md` sends reporters to; run `bolt.exe` once on a clean Windows; read the draft release and publish it, which makes the `v0.1.0` tag. The draft exists: "Bolt 0.1.0", made from `main` at `1c0b52f` by the release workflow's first full run ([run 42951072197](https://github.com/WisHonor/Bolt/actions/runs/42951072197)), with the four archives and `SHA256SUMS`; the downloaded files match their checksums, and its Windows program was run once on the owner's laptop. The third gate (item 19) comes before publishing. A protected environment for the release job would add a second lock on who can start it; it is a repository setting too. Dependabot's pull requests were left alone, as asked.

## Every ruling, verbatim

Each line below is copied as it was written in the build's ledger, in the order the rulings were made, under the task it belongs to; the ones made between tasks, after the red-team sessions and at the final review are under "Other rulings of the build". A ruling names what was decided, why, and what it costs if it is wrong. Commit IDs and agent names in them refer to the build; "the controller" is the session that ran it; the decisions they mention are in `docs/decisions/`.

### Before the build

**Before the decisions**

- Ruling (owner's inputs): the terminal-UI design opened through the Artifact tool (the canvas "Bolt terminal design", 23 artboards); it is the version the review read on 2026-11-08, the correction prompt not applied; so the look comes from the canvas and every fact from the review's list; cost if wrong: none, the owner's own rule.
- Ruling (owner's inputs): "Milestone 5's please-confirm list" was left as the unfilled placeholder, so it counts as "not read yet": the 17 items are carried unchanged into milestone 6's please-confirm list, and nothing is reversed; cost if wrong: an item the owner meant to reverse stays built until they say so.
- Ruling (owner's inputs): "License" was left as the unfilled placeholder, so it counts as blank: v0.1 is prepared with no license, and the choice heads the please-confirm list; cost if wrong: one LICENSE file and three manifest lines to add.
- Ruling (housekeeping): two stale lines fixed (milestones.md's red-team paragraph lacked milestone 5's sessions; design.md §5.15 said "milestones 1 to 4"), and two facts recorded in the follow-ups (the unread please-confirm list; the unchanged canvas). PR #23, merged; cost if wrong: none.
- Ruling (who owns which follow-up row): a row is the speed session's if fixing it changes the walk, the store, or the import's speed or memory: hostile-save memory; many planted packs; the compress bound; the import's memo tables, header memo and repeated reads; Git tag objects stored once; deep folders; milestone 4's four "measure" rows; hostile lists of tiny chunks; where Git wins; the checked-copies set. Everything else is this session's; cost if wrong: a row done twice or by nobody, caught when main is merged in.
**The decisions (113 to 123) and their review**

- Ruling (113): Custom writes no file; premise: no code reads bolt.toml today and every existing project has none (checked: no reader in crates/); why: Bolt's own values are format 1's and freeze with it; cost if wrong: Custom projects gain a file later through `bolt setup`.
- Ruling (113): without a terminal and without `--preset`, `bolt start` stops (decision 017), except in a folder that already has a bolt.toml, which is judged first and needs neither; premise: about 150 `bolt start` call sites in tests and one in bench/run.sh; cost: each gains `--preset custom`; the speed session's new tests need it too once merged.
- Ruling (113): engine and language ignore lists are a reviewed part of GitHub's templates (github/gitignore at main, fetched 2026-11-09, CC0), under "when in doubt, save it"; left out: lines that match by an extension an asset can have, Unreal's Build/ and plugin Binaries/, Node's dist/; cost if wrong: generated files saved until the user adds a line.
- Ruling (113/115): the rule files are written by a `setup` diary step right after the start, not by `start` itself; premise: decision 071 says the first entry changes no file and can't be undone; kept.rs has `overlay`, and unshelf (shelf.rs:157-234) is the template; cost if wrong: one more entry in `undo --list` after a start.
- Ruling (114): a bolt.toml or a key Bolt can't use never stops a command: warn, and use Bolt's own value; why: `bolt discard bolt.toml` must work on a broken file; cost if wrong: a typo is noticed only through the warning.
- Ruling (114): sizes are text with a unit, both families (`MB` ×1,000 and `MiB` ×1,024); premise: decision 056's default is 100,000,000 bytes, which no 1,024-based unit gives exactly; cost if wrong: none.
- Ruling (114): `chunk-files-over` can be raised to 128 GiB, never lowered below 1 MiB, and is the smallest size that is cut (decision 082's range is inclusive); cost if wrong: a later decision widens it without a format change.
- Ruling (114): the draft key `moves-with-asset` is not a key in v0.1 (move notes are v0.2); cost: a v0.1 Bolt warns about it as unknown.
- Ruling (115): no single-rule flags on `bolt setup`: bolt.toml and .boltignore are edited as text; cost if wrong: flags added later.
- Ruling (116): shortcuts take the few Git forms everyone types (`commit -m`, `stash pop`, `stash list`), never `-m` on `bolt save`; `restore` with no path is explained, not run, since `bolt discard` alone throws everything away; cost if wrong: a Git user's first command fails.
- Ruling (116): the Git-word hint shows once per person per word, only on a terminal, remembered in `git-words` beside config.toml; cost if wrong: the hint shows again.
- Ruling (117): `bolt_json` 1 freezes at v0.1 after one review of the documents; `message` and `hint` wording is not frozen; cost if wrong: a mistake found later needs a new field or `bolt_json` 2.
- Ruling (118): `verify` checks bytes and needs, takes no lock, and looks twice before calling anything missing; cost if wrong: a rare false "missing" beside a running tidy.
- Ruling (119): four archives and a draft release from a workflow started by hand; no tag, no publishing, no signing, no registry; Windows links the C runtime in; Linux builds on the oldest Ubuntu runner; cost if wrong: a rebuild.
- Ruling (120): colour on Windows through `anstyle_query::windows::enable_ansi_colors()`; premise checked: anstyle-query 1.1.5 is in Cargo.lock through clap, the function exists, and it fails unless stdout and stderr are both the console; why: no new `unsafe` of Bolt's, so the milestone needn't wait for a line-by-line review; cost if wrong: the owner asks for Bolt's own console call instead, about 15 lines in the platform crate.
- Ruling (120): the terminal's width from the `terminal_size` crate, version 0.4.4; premise checked in a throwaway project: it brings `windows-sys 0.61.2` and `rustix 1.1.6`, both already in Cargo.lock, so one new package; cost if wrong: a dependency to remove and a fixed 80 columns.
- Ruling (120): "Next:" lines, "Changed your mind?", the Git-word hint, progress, wrapping and cutting happen only on a terminal; premise: tests pin an empty stderr for successful commands (cli.rs:1615, status.rs:518) and about 410 `contains` assertions match whole sentences; why: scripts and logs keep one line per message; cost if wrong: the canvas's plain mockups differ from what a log holds.
- Ruling (120): symbols follow the terminal (UTF-8 locale; Windows Terminal or TERM_PROGRAM), not NO_COLOR; `BOLT_ASCII=1` and `=0` are the only switches; no `--color`, `--ascii` flags; cost if wrong: a flag to add.
- Ruling (120): `save` and the other storing commands show a turning mark, not a bar; why: a bar needs a byte count from the storing loop, which is the speed session's path; cost: a follow-up row.
- Ruling (121): format 1 freezes when v0.1 is published; `feature chunks` stays a feature line (no code change; it keeps development builds from before milestone 5 out); line names stay compared by Unicode's lowercase mapping (17.0 with Rust 1.99.1, checked in the toolchain's unicode_data.rs); the cutter stays outside the format, as decision 082 and storage-format.md §6 say; cost if wrong: a feature or a new format later.
- Ruling (122): change IDs are written in Jujutsu's letters (0 is `z`); display only; cost if wrong: flip back before the release.
- Ruling (123): sixteen follow-ups done in milestone 6, eight moved past v0.1, each with its reason; premises checked row by row by a read-only agent on 2026-11-09 (file and line in its report), and `~\x` checked against Git for Windows 2.50.1 on this machine ("fatal: failed to expand user dir in: '~\ign'").
- Ruling (review of the decisions, finding 1): RT7-2 is fixed as decision 080 asked, by recording the window `tidy` used in `.bolt/state/tidied`, not by passing over any missing undo-only save; why: the first rule couldn't tell "freed" from "lost", let `tidy` delete what lay below a lost save, and let `verify` miss real loss; cost: one small file in `.bolt/state/`.
- Ruling (review, finding 2): the cutter is taken out of what freezes (it contradicted decision 082 and belonged to the speed session's area).
- Ruling (review, finding 3): a folder that already has a bolt.toml is judged first, so a script can start there without `--preset`.
- Ruling (review, finding 5): Unreal's `Build/` lines, `Plugins/**/Binaries/` and Node's `dist/` are taken out of the lists.
- Ruling (review, finding 6): `verify` reads `head` again and lists the packs again before it reports anything missing.
- Ruling (review, finding 7): bolt.toml is TOML 1.1.0, what the `toml` crate (1.1.8+spec-1.1.0) reads; a vector pins an inline table over several lines.
- Ruling (review, finding 8): the draft release names the built commit as its target; decision 119 no longer claims byte-for-byte repeatable builds or that checksums say who made a file.
- Ruling (review, finding 9): the reviewer's claim that `terminal_size` brings a second `windows-sys` holds for 0.4.3, not for 0.4.4; 0.4.4 is pinned.
- Ruling (review, finding 12): a size that is a setting is shown as bolt.toml would write it (`100 MB`), measured sizes 1,024-based.
- Ruling (review, vectors): a UTF-8 byte order mark at the start of a .boltignore stays first when the block goes in; an end mark with no LF after it still ends the block; a rule's keys are named in byte order; `preset` is measured in bytes.

### Task 1

- Ruling (Task 1, concern 3): the damaged-copy `hint:` taking an empty line before it, and import's renamed list using `->` without a terminal, both follow terminal.md (§5 empty lines, §4 symbols) — accepted — cost if wrong: two lines of text.
- Ruling (Task 1, minor 8): sizes take a comma between thousands in every unit, bytes included (`1,023 B`), as terminal.md §9 says for counts; the brief's "1023 B" was the controller's slip — carried into Task 2 — cost if wrong: one format string.
- Ruling (Task 1, concern 2): the empty line before the first `warning:` is printed only when the command has already printed something (its answer, or an earlier line on stderr); stderr never starts with an empty line — terminal.md §5 is amended in Task 2's commit — why: a failing command whose first line is a warning would otherwise open with a blank line — cost if wrong: one condition.

### Task 2

- Ruling (Task 2, bar spacing): the bar follows the canvas's drawn examples (label padded to 24, one space, 24 cells, one space, the percentage in 4, two spaces, the counts), not its sentence that says "two spaces" throughout; terminal.md §8 says what is built — the examples are the exact text the canvas shows — cost if wrong: two spaces to add. (A mismatch inside the canvas itself: for the rulings file.)
- Ruling (Task 2, Important 1; plan-mandated in part): the canvas's fixed limits (counts under 70, cells under 50) can't hold its own 53-column layout, so the rule is a measured fit: a progress line never shows part of a number or of the bar; the counts go whole first, then the cells whole; the label and the percentage always show; terminal.md §8 says so — cost if wrong: the two numbers come back as limits. (Canvas mismatch for the rulings file.)

### Task 4

- Ruling (Task 4, plan defect in Review Focus 7): the test pins what a missing bolt.toml must leave unchanged instead: a file of exactly 1,048,576 bytes is stored as chunks and one of 1,048,575 whole; Bolt's own values are 1 MiB / 100,000,000 / 10,000; the existing tests of the defaults pass unchanged. No test-only change-ID override — why: IDs never depend on the threshold, so they would prove nothing, and a hook that fixes change IDs is new surface for one test — cost if wrong: a weaker statement than "same pack names", which the unchanged existing suite still backs.
- Ruling (Task 4, Important 1): the range stays 1 MiB to 128 GiB; only a file under 1 MiB is ever read whole, whatever the setting; a file of 1 MiB or more that isn't cut (below a raised threshold, or over 128 GiB) is stored whole through the streaming path files over 128 GiB already take — why: memory must not depend on a project's file; narrowing the range would have needed a new decision and still left 64 MiB whole reads — cost if wrong: such files are read twice (hash, then store), only where a project raised the threshold.
- Ruling (Task 4, minor 1): `rules_ignored` names the first 20 keys and says how many more; storage-format.md §12 and json.md §4 say so — a bound on what one hostile file can print — cost if wrong: none.

### Task 5

- Ruling (Task 5 interpretations): accepted as built: Custom with no block leaves the file as it is, even empty; a line of only CR counts as empty; .git and .bolt are matched exactly when sizes are added up (it only changes the size shown).

### Task 6

- Ruling (Task 6, concern 1): a rules file under another spelling (`Bolt.toml`) on a drive that ignores letter case stops a preset switch with the "in the way" refusal, which says to rename or move it; write-out's rule (never write over a name spelled differently) stays as it is — it is the speed session's file and a decided safety rule — cost if wrong: one rename by the user.
- Ruling (Task 6, concern 2): apply_setup takes &Person, as every other step does (it carries the time); the plan's &Identity was the controller's slip.
- Ruling (Task 6, minor 1): the step's two saves cover only the rule files it changes; at a rule file it leaves alone both hold what the save the folder was on holds; storage-format.md §9's "The `setup` step" paragraph is reworded to say so (Task 7's commit) — undo is the same either way — cost if wrong: none.

### Task 7

- Ruling (Task 7): under the test hook BOLT_FAKE_TERMINAL stdin counts as a terminal too, so tests can answer a question by piping; "may Bolt ask?" lives in one place (stdin and stderr are terminals, or the hook is set; never with --json) for all four questions — cost if wrong: none outside tests.
- Ruling (Task 7): the preset question re-asks an answer it can't read at most 3 times, then stops like the no-terminal case; end of input stops the same way — a prompt that loops for ever on a closed stdin would hang a script — cost if wrong: one number.
- Ruling (Task 7, concern 3): when the project is made but its rule files can't be written, `bolt start` ends with exit code 1, saying the project exists and that `bolt setup --preset <name>` writes them — the command did half of what was asked, and a script must not go on believing the preset is in place — cost if wrong: a warning and exit 0 instead.
- Ruling (Task 7, concern 4): the two older questions (a large save, a new line's name) are now also asked only when stderr is a terminal, as terminal.md §10 says: a prompt on a redirected stderr can't be seen — cost if wrong: one condition.
- Ruling (Task 7, minor 3): after a failed rule write, `start --json` keeps `preset` as the preset asked for, `rules_written` is `[]`, and `error` says the files weren't written; json.md's sentence becomes "The project is there, with no rule files yet" (carried to Task 8) — the document reports what was asked and what was done — cost if wrong: one word.

### Task 8

- Ruling (Task 8, Important 1; plan-mandated): decision 124 (new, refines 116): `restore` is explained, never run; `commit` runs `save` only with -m/--message or with options alone (`commit <path>` without a message option is explained); the message is passed after `--` so it is never read as an option; `stash` runs only as `stash`, `stash push`, `stash pop [N]`, `stash list` (anything else explained); `branch -option` goes to `bolt line` for its own answer; the shortcut words are ten. Why: discard is Bolt's one command that throws work away, and a Git habit must not reach it expecting less. Cost if wrong: Git users type `bolt discard` themselves; the owner can put the row back.
- Ruling (Task 8, minor 7): storage-format.md §10 now says a link at `git-words` is followed when read and replaced by a plain file when written — it is the user's own folder — cost if wrong: none.
- Ruling (Task 8, `stash pop N`): it keeps running `unshelf N` as decisions 116 and 124 say. Premise checked in decision 073: Bolt's shelf numbers are labels (smallest free from 1), Git's are positions (0 is the newest), so a number typed from habit alone can name another shelf. But in both tools the number means "the one the list shows", and `bolt stash list` shows Bolt's — cost if wrong: one wrong shelf brought back, which `bolt undo` puts back; goes on the please-confirm list with decision 124.
- Ruling (Task 8, fix round 1, help check): `tag` stays in `tidy`'s help and is left out of the "help never names a Git word" check. Premise checked: decision 116 says `tag` "becomes Bolt's own command when tags arrive", and an import already brings tags that tidy keeps — cost if wrong: none; the word is Bolt's.

### Task 9

- Ruling (Task 9, rt7_2): the future rule stays, and the test changes so that its second tidy happens after its first (through the engine, with a later clock), then runs un-ignored. Premise checked in redteam7.rs: the first tidy is called on the engine with now + 3 days, the second is the binary on the real clock, an order no real project has. Why the rule stays: a planted state/tidied naming a future time would otherwise shrink the undo window to nothing for every later tidy — cost if wrong: a machine whose clock ran ahead during a tidy and was then set back sees tidy stop (and verify report missing undo data) until the clock passes the recorded time, if the window was also raised; nothing is lost. Goes in the deferred list as a known limit.
- Ruling (Task 9, what state/tidied holds): the window start actually used, as built; §11's wording is brought in line in the fix round — cost if wrong: none, §10 and decision 123 already say "the window it used".
- Ruling (Task 9, combining looks): a needs problem is reported only when two looks agree; two looks, a third when they differ in kind, the kind two of three share wins, and if all three differ the last one is told; `head` is read before the packs are listed — why: decision 118 promises no report from a single look — cost if wrong: one more pass over the needs in rare cases.
- Ruling (Task 9, opening): `bolt verify` opens the project without cleaning tmp/ or touching the lock (premise checked in repo.rs: open → clean_tmp try-locks state/lock and removes Bolt's own leftovers) — why: decision 118 says it changes nothing and takes no lock, and verify is what one runs when something is wrong — cost if wrong: a few lines; leftovers wait for the next other command.
- Ruling (Task 9, wording): an object not found while a pack couldn't be read is `object_missing` with a message saying it isn't in any pack Bolt could read; an ID stored as another kind gets its own message; per-kind counts come from the engine and `--json` doesn't change — cost if wrong: wording only.
- Ruling (Task 9, memory): verify's memory grows with the number of objects (the reviewer's estimate: hundreds of bytes each, so several GB at ten million objects), like tidy's. Not bounded in v0.1; goes to the deferred list as a known limit, with the one cheap cut (a look keeps only its verdict) made now — cost if wrong: verify can't run on a very large store on a small machine; nothing is harmed.
- Ruling (Task 9, an unreadable head): it counts as `head_lost`, with the reason in the message; json.md's row now reads "is missing, can't be read, or names nothing Bolt can read" — premise from the implementer: opening the project already reads head, so this needs head replaced while verify runs — cost if wrong: one kind covers two causes; the message tells them apart.
- Ruling (Task 9, one look when clean): kept. No problem is reported from that single look, which is all decision 118 asks — cost if wrong: none.
- Ruling (Task 9, head's own diary): unchanged, as decision 123 words it ("an end other than state/head's own").
- Ruling (Task 9, a link at state/tidied): the refusal stays: reading counts the link as absent, and tidy stops before it removes anything, as any write over a link does; the test and storage-format.md §10 say so. My round-1 sentence asked for the opposite and was wrong — premise checked: the platform crate's write refuses a link at the destination (dir.rs, LinkRefused) for every state file — cost if wrong: a project that arrives with a link planted there can't be tidied until the link is deleted by hand; nothing is lost.
- Ruling (Task 9, "changes nothing"): opening a project still makes .bolt/tmp and .bolt/cache when they are missing, verify's opening too, because the repository holds handles to both; json.md and terminal.md say so. Goes on the please-confirm list beside decision 118 — cost if wrong: verify creates two empty folders in a copy that lost them.
- Ruling (Task 9, "and N more"): the per-kind lines say only "… and N more"; one closing line says whether --json lists them all or the first 1,000.

### Task 10

- Ruling (Task 10, the implementer's 14 choices): accepted as built pending the review's view of each: no hint after "Nothing to save" or at a save in status (terminal.md §6 gives a hint only after an error); a partial save says "Only what you named was saved: bolt status lists the changes left out." (counting them would need a walk the save doesn't do); kind changes read "file → folder"; `changes` has no summary line and draws its rule between files only on a terminal; `history` sizes its columns from its first 1,000 rows and brackets every line whose newest save is the row; `show` indents the message body 8 so no message line can pass for a change row, gives the email beside each name, measures its columns in the walk it already makes, and names a Git merge's other parents by fingerprint; "Updated save" for an amend and "Saved" for a save; the setup preview's file headers follow `bolt changes` — cost if wrong: wording and layout only; each is listed for the owner in the rulings file.
- Ruling (Task 10, `After`): every parent is named by the first 12 hex digits of its fingerprint, the first one too — why: a short change ID for a save that isn't the line's newest costs a walk of the line, and this task may add no work and no warning — cost if wrong: `After` shows a fingerprint where the canvas shows a change ID.
- Ruling (Task 10, history's columns): sized from the rows held, and holding stops at 1,000 rows or 1 MiB of held text, whichever comes first — why: keeps the aligned table and bounds memory — cost if wrong: a row after the budget with a wider ID, time or author runs long.
- Ruling (Task 10, a message shaped like a line marker): stays as the canvas has it; a known limit for the threat list (Git's decorations have the same one); `--json` can't be mistaken — cost if wrong: with colour off, a crafted message can make a row look as if a line points at it. Goes on the please-confirm list.
- Ruling (Task 10, widths): wide characters count as one column and lines that aren't lists or tables may pass the width (terminal.md §7), both known limits for the deferred list — cost if wrong: a ragged row on a narrow terminal.
- Ruling (Task 10, file sizes): history's and show's printing move to modules named after the commands, as setup_output.rs and import_output.rs are, as a last commit that changes no behaviour — cost if wrong: one mechanical diff.

### Task 11

- Ruling (Task 11, the undo list's labels): `redo N` and `undo N`, as the canvas has them — premise checked in decision 072: the redo steps are "marked as undone and numbered for bolt redo N", which the label and their place above "you are here" both do; the owner's review calls the canvas's undo list strong — cost if wrong: one string in main.rs. Listed for the owner.
- Ruling (Task 11, long outcome lines): not cut or wrapped, as terminal.md §7 rules (only lists and tables are cut, only on a terminal); same as Task 10's ruling.
- Ruling (Task 11, the other choices): accepted pending the review: `discard` names one file and counts several; goto's counts read "2 files written, 1 removed."; details are one block; "Undid 3 steps:" names no order; no "(done 5 min ago)"; "Steps you can redo come first." only when there are any; no `Next:` after the two lists; `shelf` doesn't echo the message; "Nothing to free: storage holds X."; "Undid 2 steps (all there were):"; `goto` to where you are still says "Switched to"; new modules undo_output.rs and shelf_output.rs — cost if wrong: wording and layout only.
- Ruling (Task 11, goto to the same place): the command reads the head before the switch and compares after (a few bytes, no engine change); when no step was recorded it says "Already on line <name>, at save <id> <title>." or "Already at save <id> <title>." with no details and no "Changed your mind?" line; a position-only change still records a step and keeps both lines — why: the recovery line must never point at an unrelated step — cost if wrong: one tiny read per goto, on a path the speed session owns; the read is of state/head only.
- Ruling (Task 11, "Nothing to free"): chosen only when tidy found nothing to free; when removals failed the line says nothing could be freed.
- Ruling (Task 11, tidy's outcome): with no pack removed, the choice is by what tidy found to free alone.
- Ruling (Task 11, whether goto recorded a step): from the engine, if switch.rs can say so with one boolean on the outcome it already returns (set where it returns early as "already there"); then the two head reads go away and both races with them. If it takes more, the head reads stay. switch.rs is not among the speed session's files — cost if wrong: a few engine lines to merge by hand if that session touched the same function.
- Ruling (Task 11, tidy's outcome, as built): "Nothing to free" whenever tidy found nothing to free and storage didn't shrink, whatever it rewrote; "Nothing could be freed" when it found something and removed no pack; "Freed" otherwise — accepted: the person asked for space, and none was there to give.

### Task 12

- Ruling (Task 12, `bolt` alone): the overview goes to stdout as terminal.md §11 says, and the exit code stays 2, the code's fact today — cost if wrong: one number; listed for the owner, who may prefer 0.
- Ruling (Task 12, the other choices, pending the review): "code and big files together"; an Arguments section from clap; no "See also:" line; list rows left whole; the command whose help was asked is found from the arguments; clap's `about` lowercased after "bolt <command>:"; a second line for the stash hint. The wrapping of detail sentences is ruled after the review says which commands it touches.
- Ruling (Task 12, wrapping details): the rule is general and goes into terminal.md §5 and §6: on a terminal a detail sentence wraps at the smaller of the width and 100, later lines under its text; the outcome line is one line; rows are cut, never wrapped; text from a project or a Git repository is never broken and never starts a line 4 spaces in (where it doesn't fit it goes on a row of its own, 8 in). Every command's details go through one helper — why: the design's widths page says prose wraps "4 under a detail line", and one command following a rule the others don't is how docs drift from code — cost if wrong: about a dozen call sites change; without a terminal nothing changes.

### Task 13

- Ruling (Task 13, safe-folders): the entry is opened once without following its last part; a link there doesn't count; what the handle holds is compared with the project's folder (by the handle's identity on Unix; on Windows with the handle held without sharing delete, so the folder can't be swapped, then by resolved paths). No new `unsafe`, no new package; if that can't be done, stop. An entry with `..` in it doesn't count — why: this is the trust boundary the item exists for, and a check that can be raced isn't one — cost if wrong: about forty lines in the no-follow code, part of them Unix-only and checked only by CI.
- Ruling (Task 13, switch_dropped): handed over as soon as the journal is removed, and told before an error like head_not_flushed; json.md's list gains it.
- Ruling (Task 13, left as they are): a repository's own sparse setting still stops an import when GIT_CONFIG_COUNT turns it off (safe side); whether Git counts a dangling link as a missing optional file is unknown and goes to the known gaps; WrongKind and Decode also get the damage hint.
- Ruling (Task 13, Windows identity): accepted as built: one handle, its own file ID, the same comparison on both systems; better than the round's text.
- Ruling (Task 13, a missing object before any journal): no hint is added to the shared "not in the store" error; decision 123's item 9 is about a switch that stopped with a journal on disk, which is what is built and tested.
- Ruling (Task 13, no identity): decision 125 (new, refines 123 items 5 and 13): one open without following the last name; compared by identity; a file number of 0 never counts, so on a drive that gives no identity safe-folders can't vouch for a folder; an entry with `..` in it doesn't count; a link above the entry is still followed; switch_dropped is told as soon as the journal is removed and before an error — cost if wrong: a project on such a share can't be opened through safe-folders (it could before, by paths); it fails toward refusing. Goes on the please-confirm list. Commit d19adf6 (decision 125, back-links in 123 and 042, design.md's list and count).

### Task 14

- Ruling (Task 14, the three "must" items): discard's lists become `restored_paths` and `removed_paths` (`removed` is a count everywhere else); status's `not_written_here` becomes `not_written`; the warning kind `damaged_pack` becomes `pack_damaged` — why: after v0.1 a name can never change — cost if wrong: none before a release; the speed session's tests may match the old warning name and get fixed at the merge.
- Ruling (Task 14, one shape for where the project is): `position` replaces history's `line` and line's `current`.
- Ruling (Task 14, names): a changes file's `lines` string becomes `content`; `setup --preset`'s `changes` becomes `files`; setup's `rules_file` becomes `has_rules_file` and `ignore_lines` becomes `ignore_block_lines`; counts and lists may share a name across documents.
- Ruling (Task 14, values): every value Bolt chooses is written with underscores in --json (`nothing_changed`, `kind_changed`, `project_link`, `too_big`, `too_many_lines`); text output keeps its words.
- Ruling (Task 14, dead and soft fields): `files_in_use` leaves the four documents where it is always []; `status.unfinished_switch` becomes a boolean; `goto` gains `recorded`.
- Ruling (Task 14, errors): `error.message` and `error.hint` get the screen's escaping pass in --json too. An error code is not built for v0.1: json.md says scripts go by the exit code, and a code will be a new field later — cost if wrong: early scripts can't tell errors apart except by exit code and command. Goes to the deferred list.
- Ruling (Task 14, spec-only items and what stays): as listed in task-14-rulings.md (7, 10, 11, 13, 14, 15, 18 to 21, 24, 28 spec only; 16, 17, 22, 23, 26 and two points of 27 left).
- Ruling (Task 14, snapshots): every document without one gets a snapshot of its whole shape.
- Ruling (Task 14, `start`): `line` becomes `position` there too, before the freeze — cost if wrong: none before a release.
- Ruling (Task 14, the constant's name): `DAMAGED_PACK` is renamed after the speed session's work is merged, since it is used in store.rs and pack.rs — goes to the final fix wave.

### Task 15

- Ruling (Task 15, hostile names): a saved file's name in these tests is one the name rule accepts on every system and that still needs escaping; the name with an escape character (Unix only) is tested as an unsaved file in status and changes, and the save's refusal is asserted itself — cost if wrong: none; it is the tests that change.
- Ruling (Task 15, the folder of many names): 25,000 files, a quarter over the 20,000 the look stops at.

### Task 16

- Ruling (Task 16, the examples' form): the guide shows what a person sees on a terminal, colour aside; kept — why: that is what its reader will have in front of them — cost if wrong: none.
- Ruling (Task 16, links out of the archive): the relative links stay (the link test checks them), and README.md and the guide each say near the top where the other documents are; decided after the review counts them.
- Ruling (Task 16, checking a download): the guide checks only the one archive (`grep <archive> SHA256SUMS | sha256sum -c`, and `shasum -a 256 -c` on macOS), which prints "OK" and relies on no option that an older tool may lack.
- Ruling (Task 16, links): the links to the vulnerability policy and to json.md become absolute; the README's list of other documents keeps relative links, with one sentence saying where the repository is.

### Task 17

- Ruling (Task 17, where a release may be started from): the release job runs only on a hand start from `main` — why: a draft made from an unreviewed branch would carry archives nobody reviewed — cost if wrong: one `if` to loosen.

### Other rulings of the build

- Ruling: `bolt verify --json` has exactly json.md's fields (no `lines`, `shelves`); the text summary may still say them — json.md is the spec for documents — cost if wrong: two fields to add (fields may be added after the freeze).
- Ruling: Tasks 1-6 may write tests that run plain `bolt start`; Task 7's sweep adds `--preset custom` to all of them — avoids each early task inventing a flag that doesn't exist yet — cost if wrong: none.
- Ruling: the canvas extracts stay in the scratchpad, never committed: the canvas is the owner's private artifact — cost if wrong: none.
- Ruling (dispatch rules): implementers never run the whole workspace suite here (one job, one thread, and the system killed it under memory pressure); they run every test file their change reaches, in the foreground; CI on three systems is the gate — cost if wrong: a failure found by CI instead of locally.
- Ruling: every faked-terminal test keeps progress off unless it asks for it: the helper bolt_on_a_terminal sets BOLT_PROGRESS_QUIET_MS=600000 by default — fixed at the root rather than test by test — sent to the Task 4 implementer (the tree's current owner) as a separate commit — cost if wrong: none.
- Ruling (RT10-1, RT10-2, RT10-3): all three fixed, under decision 126 (new, refines 118): what would stop a reader from writing out a save the project needs, because of what is stored, is a problem verify reports: a chunk counts only as a chunk (else object_missing); a listing's size must be the stored file's (else object_damaged on the listing); a save past the readers' limits is the new kind `save_too_big`. tidy is unchanged. Commit 5a721a6 (decision 126, the back-link in 118, json.md's three rows, design.md's list and count) — why: verify is the command that vouches for a project — cost if wrong: more memory per object in verify.
- Ruling (RT10's suspicions): the deep-tree-bomb time and verify's time on bomb packs go to the follow-ups as known costs (the first is the speed session's subject); the protected environment for releases goes on the please-confirm list as a repository setting only the owner can make.
- Ruling (RT10's Windows check): not a finding. A .boltignore that isn't a regular file stops every command that reads it, a save included (walk.rs has `symlinked_boltignore_stops_the_save` from an earlier milestone), and plain `bolt setup` does the same; a bolt.toml that isn't one is passed over with a warning, by decision 114. The check's expectation is corrected (exit 0 with a warning for bolt.toml, exit 1 with that error for .boltignore) and run again on the same branch — cost if wrong: `bolt setup` alone can't show the rules while a link stands at .boltignore; its hint says to move it.
- Ruling (RT10 fixes, the three minors): all three closed now, by the same implementer — the second is a memory blow-up a hostile store could aim at verify, which the fix itself introduced.
- Ruling (when to run it): the whole benchmark runs now, on the merge commit, not after the last small fixes — why: it takes hours, and what is still to come (red-team fixes in verify, the final review's fix wave, docs) is outside the paths it times; the quick gate against Git is run again on the final commit to confirm — cost if wrong: one more whole run.
- Ruling (a goto that needs a missing object): the new behaviour stands; it is the stopped-switch state decisions 049 and 075 design, with the hint decision 123 gave it. The test is renamed and now pins what matters: exit 1, "is not in the store", the unwritable file untouched, and, where a journal is left, that the hint names deleting it and that a save works after doing so. Commit pushed — why: restoring an up-front look for every object would undo part of the speed work on a path that session owns — cost if wrong: a damaged store leaves a switch to clear by hand where it used to refuse cleanly. Listed for the owner.
- Ruling (RT11-1): fixed, under decision 127 (new, refines 126 and 118): verify keeps details for at most the 1,000 problems it tells and counts the rest; two looks agree on the kept problems and the counts; what a look holds is bounded by the objects stored and the links between folder listings, never by entries or paths. Commit e12adf5 (decision 127, the back-link in 126, design.md's list, header and count: 127 records).
- Ruling (the deep tree's cost in goto and changes): not fixed in v0.1. Both refuse such a save, after minutes and, for goto, half a gigabyte; measuring a save by its listings first would refuse it at once, but that adds a pass over the listings to every goto, in the walk the third speed gate measures and the speed session owns. Written down with the numbers, and put on the please-confirm list; `bolt verify` tells such a save in a tenth of a second — cost if wrong: a hostile project can keep `goto` busy for minutes; nothing is written and nothing is lost.
- Ruling (the folder judged by path, then opened by path): a known limit, as Git's safe.directory; goes to the follow-ups. Ruling (git-words over a link): as storage-format.md §10 says.
- Ruling (the third gate on the combined code): the miss is the speed session's own, not a regression of this merge: on its finished code Linux's `goto` passed in two runs of seven and missed in five (1.79 to 2.50 times Git's), and this run's 2.02, with status at 1.77 (theirs 1.60 to 1.81), sits inside those bands. The owner's instruction for a failing gate is "find and fix the cause". The cause is found and written down by that session (benchmarks.md §8: a `goto` flushes ten times, each flush there for what a crash may leave; Git's checkout flushes nothing). The fix is not made here: it means flushing less, which the first principle (never lose data) puts before speed; the speed session, which owns that path, left exactly this to the owner (its decision 109 and the first item of its please-confirm list); and the other two ways it names are the walk on several threads (its own design and red-team round) and new `unsafe` on Windows (the owner's review). So the numbers are published as they are, the miss is said plainly, and it goes on this file's please-confirm list too — cost if wrong: the milestone merges with the third gate at its limit on Linux, as the speed half already did.
- Ruling (the final fix wave): one dispatch for everything both reviews found, ab6a0e45119c2573d (fresh, on Opus), working from final-fix-wave.md: A verify (no tick for a diary with a gap; an object told once; two looks for every problem the needs walk tells; the diary-end warning carried into verify), B setup (a pattern with a space quoted; a switch that wrote nothing says so and gives `applied: false`), C small code items (stale comments, two doc comments, `PACK_DAMAGED`, two tests that could pass without testing), D the documents review's thirteen items with its replacement texts. One line of the release notes ("Speed:") is the controller's to replace when the combined numbers are in.
- Ruling (final review, residual 1): `verify`'s diary line speaks about the current diary only. An entry that only a diary cut off from it can't read is told among the problems (or as the `diary_end_unreadable` warning when the store lacks it) and no longer crosses the line out or leaves a count that isn't the gap's — the re-review found the two cases treated unevenly and the count taken before those walks; three lines in `tidy.rs` put the flag back after the walk of the other ends, a unit test fails without them, and terminal.md says so — if wrong, a damaged leftover diary shows as a problem and exit code 1 beside a line calling the current diary intact, which is what both are.
- Ruling (final review, residual 2): `bolt setup` shows a pattern between double quotes when it holds a space of any kind or a double quote, with `\"` and `\\` inside — a pattern holding a quote or a wide space read as two patterns; display only, with a unit test — if wrong, a quoted pattern looks unlike what the owner typed in a literal TOML string.
- Ruling (final review, residual 3): a `bolt setup` whose every file changed between the plan and the write leaves its step in the undo diary, as every switch that left files alone does, and that is not changed — checked in the code rather than assumed: `bolt undo` of that step stops ("so that step changed nothing"), naming the files, and the edits stay as written; the engine's test of that race now asserts it, and terminal.md says it — if wrong, a step that changed nothing sits in `undo --list` until the edits are saved, shelved or discarded.
- Ruling (final review, the note): milestones.md's status line says milestone 6 was planned on 2026-11-09 and 2026-11-10 (decisions 113 to 123 on the 9th, both plans written on the 10th), which is what design.md and the step list say — if wrong, one date in one sentence.
- Ruling (the gates and the commits after fe5b46c): the gates are not run a third time before the merge — fc252fb changes a flag in `verify`'s report, how `setup` shows a pattern, tests and documents, and aa5fb49 documents only: no step a gate times; the nightly workflow runs all three on `main` after the merge — if wrong, the nightly run on `main` shows it within a day.
- Ruling (the order of the please-confirm list): the license, then milestone 5's list, then this half's, as the plan's Task 20 says — the first assembly had milestone 5's list last; the plan is the record, so the file follows it — if wrong, a reader meets seventeen older items before the new ones.
- Ruling (what "done" means with the third gate open): milestone 6 is marked done once #28 is merged, as the instruction says, and the same sentence says v0.1 has not shipped — milestones.md says v0.1 ships when all three gates pass (decision 099), and the third misses on Linux; the draft release is made, and publishing is the owner's — if wrong, the status line is one sentence to change.
- Ruling (the order of the last steps): milestone 6 is marked done first (pull request #29, documents only), and the release workflow is started from the commit that merge makes — why: publishing the draft makes the `v0.1.0` tag at the commit the workflow ran on, and that commit's status lines should say the milestone is done — cost if wrong: none; the archives are built from the same code either way. The follow-ups row that says the release job has never run, and the rulings made after the rulings file was written, are brought up to date in one more small pull request once the job has run.
- Ruling (a run of the released Windows program on this laptop): the draft's own `bolt.exe` was unpacked into a temp folder and run with its own settings folder: `--version` says `bolt 0.1.0`, and `start --preset custom`, a save, `history` and `verify` work; the archive holds exactly the program, `README.md`, `docs/guide.md` and `THIRD-PARTY-LICENSES.html`; everything downloaded was deleted afterwards — why: it is the very file the owner would publish, and the workflow's smoke tests ran on GitHub's machines only — cost if wrong: none. It is not the run on a clean Windows the follow-ups ask for (this laptop has the build tools), and that row stays open and says so.

## Where the design lost to a fact

The terminal design sets the look; Bolt's decisions, json.md and the code set the facts. Every place where the design canvas shows something Bolt's facts don't allow is listed here, command by command, as the task that built it reported it: what the canvas shows, the fact that decided, and what was built. [terminal.md](../terminal.md) is the result.

### `bolt start` and `bolt setup`

#### Start (Start.txt)

| Canvas shows | Bolt's fact | Built |
|---|---|---|
| "found Unity files and 2.3 GB of assets" | measured sizes are 1,024-based, and the walk counts all files | "found Unity files and 2.3 GiB of files" (`more than` when the look stopped) |
| "3. Custom   pick each rule yourself" | Custom is Bolt's plain defaults, with no rule files | "Bolt's plain defaults, no rule files" |
| "Choice [2]: 2", then an empty line | the same | the same (the empty line is on stderr after a valid answer) |
| "Files over 1 MB are chunked and stored once" | 1 MiB or more, shown as bolt.toml writes it | "Files of 1 MiB or more are stored in chunks, so a changed part is stored once" |
| ".psd .fbx .blend .png files can be locked while you edit them" | nothing is locked in v0.1; patterns are only marked lockable | "Art and audio files, and the engine's scenes and assets, are marked lockable; locking arrives with syncing" |
| "Code files are stored and merged line by line" | nothing is merged in v0.1 | not built |
| (no large-save line) | decision 113 lists one | "A save that adds over 1 GiB or 10,000 new files asks first" |
| "Change these any time with: bolt setup" | decision 113's words | "The rules are in bolt.toml and .boltignore. See them any time with: bolt setup" |
| "Using Game settings:" | decision 113 names the engines | "Using Game settings (Unity):" |
| "Your first save is kxqm on line main." | the same | the same, with change-ID and line colours |
| `Next:` on stderr | the same | the same, on a terminal only |
| 1c: "can't ask ... there is no terminal. / Nothing was started." with the command on its own line, indented 10 | errors keep Bolt's one-message-plus-hint layout | "Nothing was started: Bolt can't ask what kind of project this is without a terminal." The hint names all three flags and the suggestion in one paragraph. No separate copy line and no "bolt help start" mention. |
| 1d: already inside a project, with the start date and a `bolt setup` hint | existing `AlreadyAProject` refusal | left as it was (not part of this task) |
| "If the folder looks like code ... 'found a .sln and no large assets'" | fixed words only | "Looks like a code project: found .NET files and N of files." |

#### Setup (Setup.txt)

| Canvas shows | Bolt's fact | Built |
|---|---|---|
| 2a/2b: "was → now" table of 2 rules, "Unchanged: ...", "Saves you already made keep their files" | decision 115: the exact line changes, as `bolt changes` shows a file | "Switching to Code settings changes these files:", then a unified diff per file. No table, no "Unchanged" line, no "saves keep" line. |
| "over 1 MB → over 10 MB" | Code chunks from 1 MiB too | not applicable (no table) |
| "Apply these 2 changes? [y/N]" | decision 115: `Apply these changes? [y/N]` | the decision's prompt |
| "Switched Skyrunner to Code settings. 2 rules changed." | the step writes files | "Switched to Code settings: changed bolt.toml, changed .boltignore." (no folder name: that is project text and not needed) |
| "Changed your mind? bolt undo" | the same | the same, on a terminal only |
| Answering n: "Kept Game settings. Nothing was changed." | the same idea | "Nothing was changed." |
| 2b stop: "can't apply without a confirmation..." with the command on its own line | one message plus a hint | "Nothing was changed: Bolt can't ask whether to apply these changes without a terminal." / hint "If they are what you want, run: bolt setup --preset code --yes" |
| 2c: "Skyrunner uses Game settings:", with rows "Big files are chunked over 1 MB", "Ignored folders Library/ ...", "Code files merged line by line" | decision 115's list; sizes as bolt.toml writes them; nothing is merged | "Using game settings, from bolt.toml:" (the label as written), with rows Files stored in chunks / A save asks first / Lockable / No conflict markers / Left out by the preset (N lines in .boltignore) |
| 2c `Next:` "... Or change one rule: bolt setup --lock .wav" | there are no single-rule flags | "Next: edit bolt.toml or .boltignore to change a rule, or switch presets with: bolt setup --preset <name>" |
| 2d: undo repeats the table reversed, "Changed your mind again? bolt redo" | undo is task-owned elsewhere | not built. Undo prints its existing "Undid setup: 2 files back." |

### `bolt verify`

- canvas shows `bolt verify --repair` in the hint, "fetches them from origin" / Bolt's fact: no `--repair` and no server before v0.3 / built: the hint says v0.1 can't repair, that saves not using the damaged data still work, and to keep any other copy safe.
- canvas shows "good copies may exist on the server or on another computer" / Bolt's fact: no server / built: no server mentioned.
- canvas shows "… and 14 more (bolt verify --report)" / Bolt's fact: no `--report` / built: "… and N more (bolt verify --json lists up to 1,000)".
- canvas shows an "in N saves" column / Bolt's fact: not built; one save and one path are named / built: "Save <12 hex> needs it at <path>." after the message.
- canvas shows "Until then, going to a save that uses them stops with an error" / Bolt's fact: the hint says saves that don't use the damaged data still work / built: decision 118's wording.
- canvas shows the bar counting objects ("12,480 / 12,480") / Bolt's fact: the bar counts bytes of pack read / built: bytes out of bytes in all.
- canvas shows "4.8 GB" / Bolt's fact: sizes are 1,024-based (terminal.md §9) / built: "4.8 GiB".
- canvas shows "the undo diary (24 steps) is intact" / Bolt's fact: the diary holds entries / built: "(24 entries)".
- canvas shows "(most likely a disk error)" after the damaged count / Bolt's fact: decision 118 doesn't say Bolt can tell the cause / built: no cause given.
- canvas shows "(1 chunk)" after the file and paths padded into a column / Bolt's fact: decision 118 doesn't ask for it / built: the path of the file a chunk belongs to, named in the sentence.
- canvas shows a dangling save as "1 save points to an object that does not exist: ptvz" (a change ID) / Bolt's fact: problems name the object's fingerprint and one save / built: an `object_missing` row with 12-hex fingerprints for the object and the save.
- canvas shows "2 problems" coloured as a number / Bolt's fact: terminal.md §3 colours no counts / built: plain.

### `save`, `status`, `changes`, `history`, `show`

Save (canvas Save.txt)
- save, 4a: canvas shows `(2 files, +31 -4)` / Bolt's fact: no line counts per file in `save` (decision 120 "Where the canvas and Bolt's facts disagree"; review correction fact 18) / built `(2 files)`.
- save, 4a note: canvas shows sizes `(1 file, 240 MB)` and `(3 files, +12 -3, 240 MB)` / the engine hands the command line only `changed_files` (`SaveOutcome::Saved`), and sizes are 1,024-based (decision 120 Sizes; review fact 17) / not built (would need an engine change or an extra walk).
- save, 4b: canvas shows a progress bar `Saving Rock_Albedo.psd ███ 48% 580 MB / 1.2 GB` / decision 120 Progress: `save` shows the turning mark, the engine doesn't report how far a save is / built: the turning mark `Saving` (already in place).
- save, 4b: canvas shows `Stored 1.1 GB in 1,180 chunks; 96 MB was already stored and was reused.` / decision 120: no "stored in N chunks" line; the engine doesn't report it / not built.
- save, 4b/4d/4k: canvas sizes in `MB`/`GB` / 1,024-based `MiB`/`GiB` (decision 120) / no sizes in save's outcome at all (see 4a).
- save, 4c: canvas shows `Nothing to save: no changes since your last save, ptvz "Add double jump".` / matches Bolt's facts / built (the `(<id>)` form became `, <id> "<message>"`).
- save, 4c: canvas shows `hint: bolt status shows what Bolt is tracking and what it leaves out.` after a success / terminal.md §6: `hint:` follows an `error:`; a success gets at most a `Next:` naming a command / not built (Chose, below).
- save, 4d: canvas asks over 1 GB or 5,000 new files / Bolt asks over 100 MB or 10,000 new files and names the three top folders with most new bytes (decisions 113, 114; review item 3) / built: unchanged.
- save, 4d: canvas prompt `Save all of it? [y/N]` / Bolt's `Save them? [y/N]` (terminal.md §10) / unchanged.
- save, 4d: canvas names `bolt setup --ignore <folder>` / no such flag (review item 5; `bolt setup` takes only `--preset` and `--yes`) / not built.
- save, 4d note: canvas says answering n prints "Nothing was saved." and exits 0 / Bolt fails with `error: Nothing was saved: you answered no.`, exit 1 (main.rs `save`) / unchanged. Missed by the review.
- save, 4d note: canvas says the thresholds are set with `bolt setup --ask-over 1GB` / they are `bolt.toml` keys (`ask-before-saving-over`, decision 114); no such flag / not built. Missed by the review.
- save, 4d: canvas shows a progress bar after the answer / turning mark (decision 120) / built (already).
- save, 4e: canvas's stop words `this save adds 2.3 GB in 12,000 new files; no terminal to confirm it.` and two-line hint with `--ignore` / Bolt's error and hint words stay (decision 017; global constraints), hint names `--yes` / unchanged; layout per §6 was already in place.
- save, 4f: canvas prompt `Name for the new line:` / Bolt's `Name a new line for this save:` (terminal.md §10) / unchanged.
- save, 4f: canvas detail `Started line feature/wall-run from kxqmz; you are on it now.` / naming the save it started from needs a read of the parent's change ID that save doesn't do today / built Bolt's existing words `You're on the new line <line>.` as a detail line.
- save, 4f note: canvas says an empty answer prints "Nothing was saved." / Bolt fails with `error: Nothing was saved: no name was given for the new line.` (main.rs `ask_line_name`) / unchanged.
- save, 4f note: canvas re-asks a bad name with the rule "letters, digits, - _ /" / Bolt asks once and fails with the name rule's own error (main.rs `ask_line_name`, then the engine's name check) / unchanged. Missed by the review.
- save, 4g: canvas's stop words / Bolt's `You're at a save, not on a line, so this save needs a new line to go on.` with hint `Nothing was saved. Name it: bolt save --line <name> "message"` (import.rs test) / unchanged.
- save, 4h: canvas shows `Not included: 2 other changed files. bolt status lists them.` / counting other changes needs a walk of the whole folder that a partial save doesn't do (brief: no extra walk) / built `Only what you named was saved: bolt status lists the changes left out.` (Chose).
- save, 4i: canvas stop words and `(bolt status --endings lists them)` / Bolt's line-ending error and its `--accept-crlf` hint words stay; `status --endings` doesn't exist (review fact 12) / unchanged.
- save, 4j: canvas offers `--allow-lfs-pointers` and `git -C ../skyrunner-git lfs pull` / no such flag (decisions 085, 090; review item 5); Bolt's hint is `git lfs checkout`, or `bolt discard` with those paths (review fact 16) / unchanged.
- save, 4j: canvas rows `pointer to a 240 MB file` / Bolt's error words stay / unchanged.
- save, 4k: canvas shows `Updated save ptvz: now "…"  (3 files, +52 -4)` / no line counts (decision 120); `Updated` (decision 120 Layout) / built `Updated save <id> "<msg>" on <line>  (3 files)`.
- save, 4k: canvas detail `Was "Add double jump" (2 files, +31 -4). The ID stays the same.` / the old message needs a read of the replaced save, and no line counts / built `The ID stays the same.` only.
- save, 4k: canvas ends with `Changed your mind? bolt undo` / that line ends only `discard`, `goto`, `unshelf` and `setup --preset` (decision 120 Layout; terminal.md §6) / not built. Missed by the review.
- save, 4k note: `--amend` with no message keeps the message / matches / built (same line shape).

Status (canvas Status.txt and Widths.txt)
- status, 5a: canvas `On line main at save ptvz "Add double jump"  (2 hours ago)` / times read `2 h ago` (terminal.md §9) / built.
- status, 5a: canvas `Changed since then (6 files):` / matches / built.
- status, 5a: canvas kinds `changed`, `deleted`, `renamed?` / Bolt's kinds are added, removed, modified, kind changed (review item 7 and fact 15; json.md "A change"; `ChangeKind`) / built `modified`, `removed`, `kind changed`; no rename row.
- status, 5a: canvas line counts `+64`, `+31 -4`, `-120` / no line counts in `status` (decision 120; review fact 18) / built the sizes status already has.
- status, 5a: canvas `1.2 GB → 1.3 GB` / 1,024-based (decision 120) / built `GiB`.
- status, 5a: canvas `was a file, now a folder` / chose `file → folder`, the was-and-now arrow of terminal.md §4, with the engine's kind names (`EntryKind::as_str`) / built.
- status, 5a: canvas status word padded to 10 / terminal.md §5: a table is as wide as what it holds / built: padded to the widest word present (12 when a kind changed).
- status, 5a: rows sorted by path / matches / built (the engine's order).
- status, 5a note: `locked by Ben Okafor` / locks are v0.2 (review item 6) / not built.
- status, 5a: canvas `Next: bolt save "message"   or see the lines: bolt changes` / at most one command in `Next:` (terminal.md §6) / built `Next: save them with: bolt save "describe your change"`.
- status, 5b: `Nothing changed since then.` right under the outcome / matches / built.
- status, 5c: `At save kxqmz "First playable", not on a line  (3 days ago)` / matches / built.
- status, 5c: canvas hint `saving from here starts a new line; Bolt will ask for its name. Back to a line: bolt goto main` / a `hint:` follows only an error (terminal.md §6); a `Next:` names one command / not built (Chose).
- status, 5d: unfinished switch takes over the first line, with `bolt goto --continue` / no `--continue`; Bolt says so in the `switch_unfinished` warning after the answer, and the next changing command finishes it (review item 2, fact 2; json.md §4) / unchanged.
- status, 5d note: "every other command refuses" / commands finish the switch first (`switch_finished`) / unchanged.
- status, 5e: a summary over 200 changes, `bolt status --all`, `bolt status Assets/Scenes` / no decision makes a summary; `status` takes no path and no `--all`, only `--full` (review fact 12) / not built.
- status, 5f: canvas `Not written here (2 files): they are in the save, but not on this computer.` with a reason and size per file / the engine hands only paths (`not_written_here: Vec<String>`); there is no per-computer size limit / built Bolt's existing heading words, each path a row indented 4.
- status, 5f: hint `bolt setup --write-under 4GB --here` / no such flags (review fact 12) / not built.
- status, W1: a stat that doesn't fit moves to a continuation line / terminal.md §7: a path is cut, and if it still doesn't fit the row runs long / not built.
- status, W2 (an error at 60 columns): a two-column option list becomes two lines per option / `goto` is not in this task; terminal.md has no such rule / not built here.
- status, W3/W4: paths cut from the middle at 80, whole at 120; the other rows align to the cut path / matches terminal.md §7 / built (`Columns::settle`).
- status, W4 note: `bolt status --full-paths` / invented (decision 120's list) / not built.
- status, Widths: "below 60 columns one field per line" / no such rule in terminal.md / not built.
- status, Widths: path cap `width − 16 − widest stat` / built as the room left by the row's other columns: width − (4 + kind + 2 + 2 + widest held).
- status, missing from canvas: `status --full` (review "What is missing") / prints the same as `status` / no change needed.

Changes (canvas Changes.txt)
- changes, 6a: a summary line first, `3 files changed since ptvz "…": +5 -2 lines, 1.2 GB → 1.3 GB` / `bolt changes` prints each file as it compares it and keeps none, so totals are known only at the end (main.rs `changes_command`) / not built (Chose).
- changes, 6a: header `Assets/Scripts/Player.cs  +5 -2` in bold blue / built `modified  <path>  +5 -2  (6 B → 6 B)`: the kind word stays first, because a header that starts with a letter is what no line of a file can (output.rs `write_lines`); no bold-blue role exists in terminal.md §3, so the path is blue.
- changes, 6a: hunk headers `@@ line 42 (was 42)  void Update()` / standard `@@ -a,b +c,d @@` (review fact 10) / built standard; no enclosing function.
- changes, 6a: `changed, no lines shown (image, 1.2 GB → 1.3 GB)`, "default 4 MB", files stored in chunks shown without lines / Bolt shows no lines for binary content, over 64 MiB, or over 1,000,000 lines, and big chunked text files still show lines (review item 3, fact 10) / unchanged words `(binary: no lines shown)` etc.; no file-type names.
- changes, 6a: line-ending-only change as a one-liner `line endings only: LF → CRLF (212 lines, text unchanged)` / a note in the header, then the hunks with `(CRLF)` (review fact 10) / unchanged.
- changes, 6a: files separated by a dim full-width rule / built on a terminal only (one column short of the width, at most 100); without a terminal an empty line (Chose).
- changes, 6a: context lines uncoloured, `+` and `-` lines coloured, bodies not indented / matches / built.
- changes, 6b: `Save kxqmr "…" changed 3 files: +2 -1 lines, 2.3 GB added` / no summary line (as 6a) / not built.
- changes, 6b: `added, no lines shown (image, 1.2 GB)` / added binary files show `(binary: no lines shown)` / unchanged.
- changes, 6c: "Shown in blocks", `--full`, and "the decision is deterministic, not a timer" / Bolt compares for up to 2 seconds, then shows coarser hunks and warns `diff_timed_out` (review fact 10; json.md §4); `--full` doesn't exist on `changes` (review fact 12) / unchanged. The timer claim was missed by the review.
- changes, 6d: `Nothing changed since ptvz "Add double jump".` / matches / built.

History (canvas History.txt; CrossCutting 18e)
- history, 7a: heading `Saves on line main, newest first:` / matches / built.
- history, 7a: marker `▸` in column 1, bold / matches / built.
- history, 7a: ID column followed by 3 spaces / terminal.md §5: columns at least two apart / built 2.
- history, 7a: columns sized to the widest value present / history never holds a whole line (it streams) / built: widths from the first 1,000 rows; a wider value after them runs long (Chose).
- history, 7a: `2 hours ago` / `2 h ago` (terminal.md §9) / built.
- history, 7a: `[main]` after the message, in line colour / matches / built; every line whose newest save is the row gets its own bracket (an imported `feature` shows `[feature]` on its row).
- history, 7a note: the message cut with `…` at the terminal's edge / matches terminal.md §7 / built.
- history, 7a: "times relative for a week, then dates" / relative up to 30 days (terminal.md §9; decision 120; review fact 17) / built Bolt's.
- history, 7b: `yesterday` / Bolt says `1 day ago` (output.rs `relative_time`) / built Bolt's. Missed by the review.
- history, 7a: authors "never truncated under 20 characters" / authors are never cut (terminal.md §7 cuts only paths and messages) / built: padded, never cut.
- history, 7b: 20 rows, `(20 of 1,284)`, `Next: bolt history -n 50 … bolt history --all` / no `-n` (decision 120's invented-flags list), no `--all`; Bolt lists every save / not built.
- history, 7b note: `bolt history <path>`, `--by`, `--since` / invented / not built. Missed by the review.
- history, 7c: at a save, the nearest line's newer saves shown above the marker / Bolt's history walks back from the save you're at (bolt-vcs-core history.rs `for_each_on_line`) / built heading `At save <id>, not on a line. Saves back from it, newest first:`, the marker on the first row.
- history, 18e: `bolt history -n 3` / no `-n` / not built; the escaped text is as before.

Show (canvas Show.txt)
- show, 8a: `Save ptvz "Add double jump"` / matches / built.
- show, 8a: labels padded to 12 / built padded to 13, since `Fingerprint` is 11 and columns stay two apart.
- show, 8a: one `Author` and one `When` / `show` gives author and saver (review fact 15; the brief's facts) / built `Author` and `Saved by`, each with its own time.
- show, 8a: `When Sun 2026-11-08 12:14  (2 hours ago, local time)` and `--utc` / times are in the save's own zone (terminal.md §9); `--utc` is invented (decision 120) / built `2026-11-08 12:14 +01:00  (2 h ago)`; no weekday, no `--utc`.
- show, 8a note: several lines listed after `Line`, and `none (reach it with: bolt goto ptvz)` / a save's line is the line it was made on, one value, none for imported saves (review fact 15; `Save::line`) / built the one line, or `none` without the hint.
- show, 8a: `After kxqmz "First playable"` / matches / built.
- show, 8a: fingerprint "in full (16 characters)" / the full fingerprint is 64 hex digits (review item 7; brief facts) / built 64.
- show, 8a: the message body indented 4 / change rows are now indented 4 too, so a message line could pass for a change / built indented 8 (Chose).
- show, 8a: `2 files changed: +31 -4 lines, 1.2 KB added` / no line counts (show doesn't compare contents); sizes 1,024-based / built `2 files changed:`.
- show, 8a rows: `changed`, `+31 -4`, `1.2 KB` / `modified`, sizes, `KiB` (as status) / built.
- show, 8a note: an amended save adds `Updated … (fingerprint was 5c1d…)` / a save records nothing about the save it replaced (`Save` has no such field; only `save --json`'s `replaced` says it) / not built. Missed by the review.
- show, 8a: `Next: see the lines: bolt changes --save ptvz` / matches §6 / built `Next: see its lines with: bolt changes --save <id>`, when the save changed files.
- show, 8b: no message body, no block / matches / built.
- show, 8b: `3 files changed: 1.2 GB → 1.3 GB, 410 MB deleted` / no decision for size totals / not built (they could be summed in the walk that measures the columns).
- show, 8b: `deleted`, `renamed` (no question mark in show) / `removed`; there is no renamed kind (decision 120; review item 7, fact 15) / built `removed`; a rename shows as removed and added.

Cross-cutting (as they touch these five commands)
- 18a/18b: lock waiting with a 30-second stop, `--wait`, `bolt unlock` / no timeout, no flag (decision 120; review fact 13) / not in this task; unchanged.
- 18c/18d: invented warnings (`locked by`, mixed line endings, stored whole) / Bolt's warnings are json.md §4's / unchanged; warnings keep their words.
- 18e: escaped text, widths counting the escaped form / matches (terminal.md §3, §7) / built.
- 18f: Git-word hints and `bolt setup --git-words quiet` / decision 116, first time only; no such flag / unchanged.

### `goto`, `discard`, lines, undo, shelves, `tidy`

Goto (Goto.txt; Widths W2; CrossCutting 18f)
- goto, 9a: canvas shows a progress bar `Writing files ███░ 38% 18 / 48` / the engine doesn't report how far a switch is; `goto` shows the turning mark (decision 120 Progress; terminal.md §8) / built: the turning mark `Writing files` (already wired).
- goto, 9a: canvas shows `Switched to line feature/double-jump at save mvtk "Double jump: coyote time".` / matches / built.
- goto, 9a: canvas shows `Updated 48 files (1 added, 46 changed, 1 deleted) from ptvz.` / the engine hands only what was written and what removed (`Applied::written`, `Applied::removed`, writeout.rs), not added against changed, and not the save it came from (`GotoOutcome`, switch.rs) / built `48 files written, 1 removed.` (`No file needed changing.` when nothing was), no "from".
- goto, 9a note: canvas shows `goto main` when already there as "Already on line main at save ptvz … Nothing was changed." / `GotoOutcome` doesn't say whether the position changed, and the goto is a step all the same / not built: it says `Switched to line main at save … .` and `No file needed changing.` Missed by the review.
- goto, 9b: canvas lists the unsaved changes that came along by path, "(2 files, untouched by the switch)", and "… and 4 more (bolt status lists them)" / `GotoOutcome::carried` is a count, with no paths / built Bolt's words `Your unsaved changes in 2 files came along.` as a detail.
- goto, 9c: canvas shows `error: 2 files you changed would be overwritten by this switch. Nothing was changed.` / Bolt's error words stay (`SwitchError::Collisions`, switch.rs): `Your unsaved changes, or files Bolt ignores, are in the way, so nothing was changed:` with the first 5 paths and `and N more` (`snapshot::NAMED`) / unchanged words; layout per §6 (already).
- goto, 9c: canvas's hint lists three ways out in two columns (`bolt save "message"`, `bolt shelf`, `bolt discard <file>`) / Bolt's hint is one sentence naming save, shelf, discard and moving ignored files (`SwitchError::hint`) / unchanged words. The two-column option layout (Widths W2) is not built: terminal.md has no such layout, and no Bolt hint has columns.
- goto, 9c note: no `--force` / matches.
- goto, 9d: canvas says files open in another program are found before any write, "Nothing was changed.", and names the program (`open in Unity`) / Bolt tries each file 10 times while writing, writes the rest, then fails with exit 1, the switch left unfinished (`RepoError::SwitchInUse`, switch.rs; `files_in_use`); it can't name the program (review item 2, correction fact 3) / unchanged.
- goto, 9d: canvas's hint `close it there (or close Unity), then run: bolt goto feature/double-jump` / Bolt's own hint stays / unchanged.
- goto, 9d note: canvas says a lock mid-write ends with exit code 2 and the unfinished state / exit 1; no `--continue`; the next changing command finishes the switch (decision 120 exit codes; review item 2, fact 2) / unchanged.
- goto, 9e: canvas shows `Switched to save kxqmz "First playable". You are not on a line.` / matches / built.
- goto, 9e: canvas gives a stderr `hint:` "saving from here starts a new line; Bolt will ask for its name. Back to where you were: bolt goto main or bolt undo" / a `hint:` follows only an `error:` (terminal.md §6), and `Changed your mind? bolt undo` ends `goto` / built Bolt's existing words as details on stdout, `Saving here asks for a new line's name.` and `To keep saving on <line>: bolt goto <line>` (only when the save is a line's newest, as before), then `Changed your mind? bolt undo` on a terminal.
- goto, all screens: canvas shows no `Changed your mind? bolt undo` on any goto screen / terminal.md §6 and decision 120 end `goto` with it / built, on a terminal only. Missed by the review.
- goto, 9b and 9e: canvas puts an empty line between detail groups / built one block of details. Missed by the review (a look, not a fact; Chose 3).
- goto, CrossCutting 18f: `bolt checkout` hint `in Bolt this is: bolt goto …` / decision 116, first time only / unchanged.
- goto, not on the canvas: an ambiguous target (review "What is missing") / `SwitchError::Ambiguous`'s words and hint / unchanged.

Discard (Discard.txt)
- discard, 10a: canvas shows `Threw away your unsaved changes to Assets/Scripts/Player.cs  (+31 -4 lines)` / `discard` doesn't compare contents and `DiscardOutcome` holds paths only; no line counts in commands that would have to read files to count them (decision 120, "no line counts per file in save or status") / built `Threw away your unsaved changes to <path>.` with no bracket.
- discard, 10a note: canvas reads `(1.2 GB → 1.3 GB)` for a binary file and `(the file is back)` for a deleted one / not handed (`DiscardOutcome`) / not built.
- discard, 10a: canvas detail `A safety copy was kept.` / Bolt counts the copies (`DiscardOutcome::safety_copies`; a deleted file put back needs none) / built `A safety copy of it was kept.` as the detail.
- discard, 10a: `Changed your mind? bolt undo` on its own stderr line / matches terminal.md §6 / built, terminal only.
- discard, 10b: canvas lists several files one per line with what each lost, under `to 2 files:` / the paths are in hand, but a bare `bolt discard` can name every changed file of a project, and terminal.md bounds no such list / built the count only, `to 2 files.` (Chose 1).
- discard, 10b: `new file, 64 lines, removed` / no line counts (as 10a) / not built.
- discard, 10b: `Safety copies were kept.` / Bolt's counted words: `A safety copy of each was kept.` or `Safety copies of the 3 that held unsaved work were kept.` / built.
- discard, 10c: `Nothing to discard: Assets/Scripts/Enemy.cs has no unsaved changes.` / Bolt's `Nothing to discard there: it matches your last save.` (output.rs `discarded`), which names no path / unchanged.
- discard, 10d: a bare `bolt discard` refused, `--all` as the way to mean everything, "exit code 3" / a bare discard throws away every unsaved change, undoably; there is no `--all`; exit codes are 0, 1, 2 (review item 4, fact 6; decision 120) / not built.
- discard, 10e: `error: Library/ArtifactDB is not tracked: Library/ is an ignored folder.` with a hint naming `bolt setup` / Bolt doesn't refuse a named ignored path: discard leaves ignored files alone (discard.rs test `discard_leaves_ignored_files_alone`), so it says `Nothing to discard there: …`; it refuses only when an ignored file is in the way of a file going back (`SwitchError::InTheWay`) / not built. Missed by the review.

Line (Line.txt)
- line, 14a: canvas heading `Lines in Skyrunner:` / matches (the project folder's name, escaped) / built.
- line, 14a: canvas rows give each line's newest save's ID, time and message / that needs reading each line's newest save and shortening its change ID against its line (no extra work; the controller's rule against shortening IDs in a way that walks a line) / not built: names only.
- line, 14a note: names padded, ID padded to 6, time padded to 11 / no columns with names only / not built.
- line, 14a: `On the server (origin):` block with `main@origin` rows / matches; one block per server, its rows right under the heading as drawn / built.
- line, 14a: `2 saves behind`, `3 saves ahead`, `ahead and behind` / needs walking both lines; comparing lines is v0.2 work (review item 6) / not built.
- line, 14a note: no server, no block / matches / built.
- line, 14b: `Lines in Skyrunner. You are at save kxqmz, not on a line.` / naming the save by change ID would read it and shorten its ID / built `You are at save <first 12 hex digits>` (controller's rule).
- line, 14b: `hint: to get onto a line: bolt goto main` / a `hint:` follows only an error (terminal.md §6) / not built.
- line, 14a: times `yesterday` / not shown (as above).

Undo (Undo.txt)
- undo, 11a: `Undid discard Assets/Scripts/Player.cs  (done 5 min ago)` / the diary records the command's name (`DiaryEntry::command`), not its paths; Bolt's step line says what changed after a colon (output.rs `step_line`) / built `Undid discard: 1 file back.`; no time (Chose 5).
- undo, 11a: detail `Your folder: Assets/Scripts/Player.cs got its unsaved changes back (+31 -4).` / no line counts, no per-file paths (the step's `Applied` counts) / built: what changed stays in the one line (`1 file back`).
- undo, 11a: `Changed your mind again? bolt redo` / that line ends only `discard`, `goto`, `unshelf` and `setup --preset` (terminal.md §6; decision 120) / not built. Missed by the review.
- undo, 11b: `Undid 2 steps, newest first:` / built `Undid 2 steps:` (Chose 4: for `redo` the order is the other way).
- undo, 11b: rows `discard Assets/…  its unsaved changes are back (+31 -4)` with the effect dim in a column padded to 36 / built Bolt's step line without its verb, indented 4, its effect after a colon, not dim (Chose 6).
- undo, 11b: `You are on line main at save kxqmz "First playable".` after the rows / where the project is now is already in the step lines (`main is back at …`), and the line would read the save again / not built.
- undo, 11b: `Changed your mind? bolt redo 2` / as 11a / not built.
- undo, 11b note: "Undid 3 steps (all there were)" when N is more than there are / matches / built `Undid 2 steps (all there were):`.
- undo, 11c: `Nothing to undo: this is the start of the diary (project started 2026-10-02).` / Bolt's `Nothing to undo.` (main.rs `undo_command`); the date would read the first entry / unchanged.
- undo, 11d: `error: undoing "goto feature/double-jump" would overwrite your unsaved changes to 2 files. Nothing was changed.` / Bolt's own error words stay / unchanged.
- undo, 11d: `hint: … bolt undo --shelve` then `Or save them first: bolt save "message"` / the hint names only `--shelve`: a save first would be the step undo takes next (review item 2, fact 4) / unchanged words (they name only `--shelve`).
- undo, 11e: `Shelved your unsaved changes to 2 files as shelf 1 "Before undo of goto".` / the message is `Set aside by undo` (review item 7, fact 5); the file count isn't handed (`UndoDone::shelved` holds label and save) / built `Shelved your unsaved changes in the way on shelf 1.`, first, as the canvas orders it (before: last).
- undo, 11e: `Undid goto feature/double-jump  (done 10 min ago)` and `You are back on line main at save ptvz …` / as 11a and 11b / built the step line.
- undo, 11e: `Next: bring the changes back with: bolt unshelf   or undo this: bolt redo` / one command a `Next:` (terminal.md §6) / built `Next: bring your changes back with: bolt unshelf`.
- undo, 11f: `Redid discard Assets/…` with detail `Your folder: its unsaved changes are gone again (safety copy kept).` / as 11a / built `Redid discard: 1 file written.`
- undo, 11f: `Changed your mind? bolt undo` after redo / not in §6's list / not built. Missed by the review.
- undo, 11f: `Nothing to redo: the newest step has not been undone.` / Bolt's `Nothing to redo.` / unchanged.
- undo, not on the canvas: `undo N` stopping part-way (review "What is missing") / built `Undid 1 of the 3 steps asked for:` (was a closing sentence with a full stop), rows, then the error.
- undo, not on the canvas: a lost `head` and `undo --to` picking an entry (review) / Bolt's `Made entry … the current one: …` / built, `Made` green.
- undo, not on the canvas: undoing a step whose saves tidy removed (review) / the engine's error and hint / unchanged.

Undo list (UndoList.txt)
- undo --list, 12a: heading `Undo diary for Skyrunner, newest first. Steps you can redo come first.` / matches / built; the second sentence only when there are steps to redo (Chose 7).
- undo --list, 12a: labels `redo 2`, `redo 1`, `undo 1` / decision 072 says the redo steps are "marked as undone and numbered for `bolt redo N`"; today's label was `undone N` / built `redo N` and `undo N` (Chose 8: the owner may rule).
- undo --list, 12a: entry IDs, 12 hex digits in fingerprint colour / matches / built, blue.
- undo --list, 12a note: step label padded to 7, time padded to 11 / terminal.md §5, columns as wide as what they hold / built.
- undo --list, 12a: steps named `save ptvz "Add double jump"`, `discard Assets/Scripts/Player.cs`, `unshelf "Enemy aim experiment"`, `goto main` / the diary keeps the command's name, not its paths or the shelf's message; Bolt's words are output.rs `entry_line`'s: `save: main moved to ptvz "Add double jump"`, `discard: 1 file changed`, `goto: on main at ptvz, 48 files changed` / built Bolt's.
- undo --list, 12a: the marker row `▸ you are here` between the redo and undo steps / matches / built.
- undo --list, 12a: 5 undo steps, then `… 12 older steps` / up to 20, then "and more" without a count: counting would read the whole diary (decision 072; review item 3) / built `  and more: bolt undo --list --all shows the whole diary`.
- undo --list, 12a: `Next: all steps: bolt undo --list --all   back to one: bolt undo --to <entry>` / one command a `Next:`; the "and more" line names `--all` already / not built (Chose 9).
- undo --list, 12a text: IDs accept prefixes of 4 or more / `undo --to` needs 8 (decision 072; review item 3) / unchanged.
- undo --list, 12b: `--all` with `redo N`/`undo N` labels, the marker, and a `start` label on the first entry / `--all` is streamed and the whole diary holds undo and redo entries and cleared steps, which no `undo N` reaches (decision 072) / not built: `--all` keeps its layout (entry, time, what), gaining colour.
- undo --list, 12b: rows `line new feature/double-jump`, `setup --lock .wav`, `import ../skyrunner-git`, `verify --repair` / no such commands or flags; `import` takes no argument (review items 1, 5, 6) / not built.
- undo --list, 12b: first entry `project started; the diary begins here` with a `start` label / Bolt's `start: the diary's first entry, which can't be undone`, with no label, since undo can't take it / built Bolt's.
- undo --list, 12c: `Undid 2 steps, back to entry 0a1b2c3d4e5f (save kxqmz "First playable"):` / built `Undid 2 steps:`: the entry is the one typed; naming its save would read it / built.
- undo --list, 12c: `You are on line main at save kxqmz …` and `Changed your mind? bolt redo 2` / as 11b / not built.
- undo --list, 12c note: an ambiguous 4-digit prefix error / Bolt's own error, 8 digits at least / unchanged.
- undo --list, 12c note: `--to` the start entry "undoes everything and says so" / not a decided behaviour of this task / unchanged.
- undo --list, not on the canvas: the lost-`head` list (review) / Bolt's words / unchanged layout, colour added.

Shelf (Shelf.txt; CrossCutting 18f)
- shelf, 13a: `Shelved your unsaved changes to 3 files as shelf 1 "Half-done wall run".` / `ShelfOutcome` hands label, save and file count, not the message (the default `Shelved` lives in the engine, shelf.rs `DEFAULT_MESSAGE`) / built `Shelved your unsaved changes in 3 files on shelf 1.` (Chose 10).
- shelf, 13a: detail `Your folder is back at save ptvz "Add double jump".` / naming the save would read it and shorten its change ID / not built.
- shelf, 13a: `Next: bring them back with: bolt unshelf` / matches: the new shelf is the newest, which `bolt unshelf` brings back / built, terminal only.
- shelf, 13a note: a shelf without a message is named `On main at ptvz, 3 files` / Bolt's `Shelved` (review item 7, fact 5) / unchanged (engine).
- shelf, 13a note: "Shelves that already exist move down one number; the list always starts at 1." / a new shelf takes the smallest free number and numbers never shift (decision 073; review item 2, fact 5) / not built.
- shelf, 13b: `Nothing to shelve: you have no unsaved changes.` / Bolt's `Nothing to shelve: your files match your last save.` / unchanged.
- shelf --list, 13c: heading `2 shelves, newest first:` / matches (`shelf::shelves` is newest first) / built.
- shelf --list, 13c: rows `1  "Half-done wall run"  3 files  2 hours ago  from main at ptvz`, facts dim / `from main at ptvz` would name the shelf's parent save, a read and a shortened ID; Bolt's words are `on <line>` or `at a save` (`ShelfInfo::line`); times read `2 h ago` / built number, quoted first line, files, time, `on main`; facts dim.
- shelf --list, 13c note: number right-aligned to 3, files padded to 8, time to 11 / terminal.md §5 / built as wide as what they hold.
- shelf --list, 13c: `Next: bring one back: bolt unshelf 2   (no number means the newest)` / a list only reports; a `Next:` naming one shelf would be a guess / not built (Chose 9).
- shelf --list, 13c note: `bolt shelf --show 2` / no such flag (review item 5) / not built.
- shelf --list, 13d: `No shelves.` on stdout and a `hint:` on stderr / a `hint:` follows only an error (terminal.md §6) / unchanged: `No shelves: bolt shelf sets your unsaved changes aside on one.`
- unshelf, 13e: `Unshelved "Half-done wall run": 3 files are unsaved changes again.` / Bolt says what the switch wrote (`Applied`), and keeps the shelf's number / built `Unshelved shelf 1 "Half-done wall run": 3 files written.`
- unshelf, 13e: detail `The shelf was removed. 1 shelf left.` / the count left would read the state again / not built.
- unshelf, 13e: `Changed your mind? bolt undo` / matches terminal.md §6 / built, terminal only.
- unshelf, 13f: the clash error with the hint `bolt shelf "message" then bolt unshelf 2` / numbers never shift, so `unshelf 2` would bring back the wrong shelf (review item 2); Bolt's own error and hint stay / unchanged.
- unshelf, 13f note: no combining / matches (v0.2).
- unshelf, not on the canvas: an unknown shelf (review) / Bolt's error / unchanged.
- shelf, CrossCutting 18f: `bolt stash` with the default message `On main at ptvz, 3 files` and the Git-word hint / `Shelved`; decision 116 / unchanged.

Tidy (Tidy.txt)
- tidy, 15a: `Checking what can be freed …` on stderr / the turning mark `Tidying` (decision 120; terminal.md §8) / built (already).
- tidy, 15a: `About 1.2 GB can be freed: 3 old packs and 840 chunks that no save uses.` on stdout / Bolt's `Tidying: about X to free` stays on stderr, on a terminal only (review fact 7; the brief) / unchanged.
- tidy, 15a: a bar `Rewriting packs ███ 66% 2 / 3` / the turning mark (decision 120) / built (already).
- tidy, 15a: `Freed 1.2 GB  (3 packs removed, 1 smaller pack written)` / sizes 1,024-based (decision 120) / built `Freed 1.2 GiB  (3 packs removed, 1 written)`.
- tidy, 15a: detail `Every save, shelf and undo step is still complete.` / false: tidy removes what only steps older than the undo window need (review item 4) / not built; the detail is the young-packs sentence when there are young packs.
- tidy, 15a note: `bolt tidy --check` / no such flag (review items 5, 7) / not built.
- tidy, 15a note: Ctrl-C prints `Stopped. Freed 410 MB so far; run bolt tidy again for the rest.` / no decision makes such a message / not built. Missed by the review.
- tidy, 15b: `Nothing to free: storage is already tidy (4.8 GB in 6 packs).` / `TidyReport` hands bytes, not the number of packs; "already tidy" isn't so while young packs wait / built `Nothing to free: storage holds 4.8 GiB.` (Chose 11).
- tidy, not on the canvas: packs changed in the last 24 hours left as they are (review fact 7) / built as a detail.
- tidy, not on the canvas: stopping at a pack it can't read (review's opening note) / the engine's error and hint words / built per §6: stdout empty, `error:`, empty line, `hint:`; terminal.md §12 row and a test.

Cross-cutting and widths, as they touch these commands
- 18a/18b: the lock wait with a 30-second stop, `--wait`, `bolt unlock` / no timeout, no flag (decision 120; review fact 13) / unchanged.
- Widths: continuation lines indented 4 under a detail line / stdout details aren't wrapped; terminal.md §6 wraps only `error:`, `warning:`, `hint:` and `Next:` / not built.
- Widths W2: two-column option lists in hints / no Bolt hint has columns / not built.
- Widths: "below 60 columns one field per line" / no such rule in terminal.md / not built.

### `import` and help

Import (canvas chapter 3):

- import, header: canvas shows `bolt import <git-repository>` and "into a Bolt project in the current folder" / Bolt's fact: `bolt import` takes no argument, works in place next to `.git/`, never changes the Git repository (review item 1; `main.rs` `Command::Import` has no fields) / built: no argument anywhere.
- import, header: canvas shows "refuses up front when the Git side is incomplete rather than importing something half-right" / Bolt's fact: most refusals come before `.bolt` is made, but a stop found while converting keeps the started `.bolt` so a rerun finishes (decision 041; `import.rs` `ImportError::Unfinished`) / built: the §12 row "Stopped after `.bolt` was begun".
- import, 3a: canvas shows phase lines on stderr that "also appear in logs" (`Reading Git history in ../skyrunner-git … 1,284 commits, 3 branches, 12 tags.`, `Importing 1,284 commits …`, `Bringing in 41 Git LFS files (11.6 GB) …`) / Bolt's fact: lines for people only on a terminal; progress is one line, erased at the end, and the outcome line is the summary (decision 120 option B; terminal.md §8) / not built.
- import, 3a: canvas shows the bar `Bringing in LFS files  … 12%  1.4 GB / 11.6 GB`, by bytes / Bolt's fact: the passes are reading Git's history, checking Git LFS files and importing commits, and the engine reports commits for each, never bytes of Git LFS files (`import.rs` `ImportPass`, `ImportRequest::progress`) / built: `Reading Git's history` (turning mark), `Checking Git LFS files` and `Importing commits` (bars by commits). The bytes bar isn't built: the engine doesn't report them, and the engine doesn't change in this task.
- import, 3a note: canvas shows "the first phase … completes the line with what it found" / Bolt's fact: the progress line is erased when the work ends (§8) / built: turning mark, then erased.
- import, 3a note: canvas shows "Ctrl-C at any point leaves no project behind … 'Stopped. Nothing was imported.'" / Bolt's fact: an interrupted import keeps the started `.bolt` so `bolt import` can finish it (review item 1; decision 041) / not built: no such message exists.
- import, 3b: canvas shows `$ bolt import ../skyrunner-git` / Bolt's fact: no argument (review item 1) / built `bolt import`.
- import, 3b: canvas shows the outcome line `Imported ../skyrunner-git into C:\Users\ana\Skyrunner` / Bolt's fact: there is no source path. The summary's counts are today's: `Imported N commits from Git: N lines[, N server lines from N servers], N tags.` (`import_output.rs`; brief) / built: today's line, `Imported` green and bold.
- import, 3b: canvas shows the table row `1,284 commits  became saves` / Bolt's fact: the commit count is in the outcome line (`import_output.rs`) / not built as a row: it would repeat the outcome line's count.
- import, 3b: canvas shows `3 branches  became lines: main, feature/double-jump, art/level2` / Bolt's fact: the report gives a count of lines, not their names (`ImportReport::lines`); naming them would mean reading the lines, extra work only to print / built: the count in the outcome line; names not built.
- import, 3b: canvas shows `12 tags  kept as tags` / Bolt's fact: the tag count is in the outcome line; the tags not imported are a detail with why (`import_output.rs`) / built: as today.
- import, 3b: canvas shows `41 LFS files  brought in whole, 11.6 GB` / Bolt's fact: `Brought in 41 Git LFS files (11 GiB) from .git/lfs/objects.`, sizes 1,024-based (terminal.md §9; `import_output.rs`) / built as a detail.
- import, 3b: canvas shows `1 name  changed: master → main` / Bolt's fact: Bolt keeps `master` as it is and renames only names its rules refuse (bad characters become `-`, clashes get `-2`) (review item 1; decision 033) / built: the `Renamed N names that Bolt's names don't allow:` detail, with `→` on a terminal that shows symbols.
- import, 3b: canvas shows `5 entries  skipped (4 submodules, 1 symlink): bolt import --report` / Bolt's fact: submodules become project links and symlinks are kept; only `.git` and `.bolt` spellings are left out; there is no `--report` (review items 1 and 5) / built: today's `Left out N entries whose names Bolt never writes to disk …` detail; `--report` not built.
- import, 3b: canvas has no row for refs not imported, inexact commits, commits holding Git LFS files, or inexact Git LFS pointers / Bolt's fact: today's summary says all of them (review item 1; `import_output.rs`) / built: each as a detail.
- import, 3b: canvas shows the table with counts right-aligned and bold, then the word / Bolt's fact: the counts sit inside today's sentences, which the brief keeps / not built as a table. The details keep today's sentences, laid out per §5.
- import, 3b: canvas shows `You are on line main at save ptvz "Add double jump".` / Bolt's fact: `You're on line main.`; the save isn't read for an import on a line, which would be extra work only to print; at a detached `HEAD` it is `You're at save <id>, where Git's HEAD was detached: not on a line. Saving here asks for a new line's name.` (`main.rs` `import`) / built: today's words, last, after an empty line, the line blue and the ID magenta and bold.
- import, 3b: canvas shows `warning: 3 text files have Windows line endings (CRLF) in Git. Your first save will ask what to do with them.` / Bolt's fact: the import gives no such warning; the first save stops at files whose only difference is the line endings Git converted, and `--accept-crlf` saves them (review item 16) / not built.
- import, 3b: canvas shows `Next: look around with: bolt status` / no conflicting fact / built, on a terminal only (decision 120 B).
- import, 3b note: canvas says "In a pipe the summary table is what arrives; the warning stays on stderr" / matches Bolt / built.
- import, 3c: canvas shows `$ bolt import ../skyrunner-git` and the phase line before the error / no argument (review item 1); no phase lines in logs (decision 120 B) / not built.
- import, 3c: canvas shows `error: 23 Git LFS files are missing from ../skyrunner-git. Only their pointers are there. Bolt imports whole files, so nothing was imported.` / Bolt's fact: `N Git LFS files in this history, SIZE in all, aren't in .git/lfs/objects:` (decision 085; `import.rs` `lfs_missing_message`) / built: the engine's words, laid out per §6.
- import, 3c: canvas shows a table of 20 paths padded to 46, cut in the middle, sizes right-aligned, largest first / Bolt's fact: the error lists the first 20 met in the history, each `` `path` in commit `hex` (size) ``, then `and N more`; an error is never cut (decision 085; terminal.md §7) / built: the engine's list, unchanged.
- import, 3c: canvas shows `23 files, 9.8 GB in total` / Bolt's fact: the count and total are in the error's first line, 1,024-based (decision 085) / built: as the engine says it.
- import, 3c: canvas shows the hint `fetch them into the Git repository, then import again: cd ../skyrunner-git && git lfs fetch --all && git lfs checkout` / `bolt import ../skyrunner-git` / Bolt's fact: `Nothing was changed. Download every version Git LFS keeps with `git lfs fetch --all`, then run bolt import again. …`, plus the `lfs.storage` sentences (decisions 085, 089; `ImportError::hint`) / built: the engine's hint, whole.
- import, 3c: canvas shows `Or leave those 23 files out of every save: bolt import ../skyrunner-git --skip-missing-lfs` / Bolt's fact: there is no such flag and no partial import (review item 5; decision 085) / not built.
- import, 3c note: canvas says "The full list is in bolt import --report, which also works after a stop" / no `--report` (review item 5) / not built.
- import, 3c: canvas shows "Nothing was written" (caption) / matches when it stops before `.bolt` is made; after `.bolt` began, the hint says the folder is kept (decision 041) / built.
- import, 3d (shallow clone): canvas shows `error: ../skyrunner-git is a shallow clone: its history stops after 50 commits. Bolt needs the whole history. Nothing was imported.` / Bolt's fact: `This Git repository is a shallow clone: its older history was never downloaded, and Bolt needs all of it.`; Bolt doesn't count the commits (`git/mod.rs` `Refusal::Shallow`) / built: the engine's words, per §6.
- import, 3d: canvas shows the hint `git -C ../skyrunner-git fetch --unshallow` then `bolt import ../skyrunner-git` / Bolt's fact: `Fetch the rest with `git fetch --unshallow`, then run bolt import again.` (`git/mod.rs` `Refusal::hint`) / built: the engine's hint.
- import, 3e (sparse checkout): canvas shows `error: ../skyrunner-git uses sparse checkout: some files are not on disk. Nothing was imported.` / Bolt's fact: `Git leaves some files out of this folder on purpose (a sparse checkout), so Bolt would record them as deleted.` (`git/mod.rs` `Refusal::SparseCheckout`) / built: the engine's words.
- import, 3e: canvas shows the hint `git -C ../skyrunner-git sparse-checkout disable`, `bolt import ../skyrunner-git` / Bolt's fact: `Turn it off with `git sparse-checkout disable`, then run bolt import again.` / built: the engine's hint.
- import, 3f (uncommitted changes refused): canvas shows `error: ../skyrunner-git has uncommitted changes in 14 files …` with `git stash` in the hint / Bolt's fact: uncommitted Git work is not refused; it comes along into the next save (review item 1) / not built. terminal.md §12 states the fact.
- import, 3f: canvas shows `bolt import ../skyrunner-git --ignore-uncommitted` / no such flag (review item 5 in spirit; `main.rs`) / not built.
- import, "What changed" list: canvas says "Tags are 'kept as tags' for lack of a Bolt word" / Bolt's fact: `tag` is Bolt's own word for the tags an import brings (decision 116; `tests/git_words.rs` leaves it out) / the outcome line counts `tags`.
- import, states the canvas lacks: a second import refused; the Git LFS stops for objects kept elsewhere, `ext-` lines and a SHA-256 mismatch; the work stop (decision 091); a stop after `.bolt` began (review "What is missing"; brief) / built: each laid out per §6 in its engine words, with a row in §12. Tests: `each_git_lfs_stop_keeps_its_words_and_names_no_flag`, `the_work_stop_is_laid_out_like_every_other_error`, `a_second_import_is_refused`.
- import, widths: canvas shows import's summary at 80 only / review "What is missing" asks for 60 and 120 / built: details wrap under 4 at 60 and 80, and the test checks 60, 80 and 120.

Help (canvas chapter 17):

- help, 17a: canvas shows `Bolt keeps every version of your project: code and big assets together.` / Bolt's tagline is "code and big files" (clap's `about` in `main.rs`: "Version control for code and big files.") / built with "big files" (see Chose 1).
- help, 17a: canvas shows `Usage: bolt <command> [options]` / matches / built.
- help, 17a: canvas shows the group titles `Every day`, `Lines, shelves, details`, `Project` / terminal.md §11 names them "every day; lines, shelves and details; the project" / built: `Every day`, `Lines, shelves and details`, `The project`.
- help, 17a: canvas shows `status  What changed since your last save, and where you are` / matches / built as `See what changed since your last save, and where you are`.
- help, 17a: canvas shows `save  Save your changes as a new save` / no fact against it / built as `Save your changes, new files too` (it names save's difference from Git's staging step, decision 116's `add` explanation).
- help, 17a: canvas shows `changes  Show what changed, line by line` / matches / built as `Show your unsaved changes line by line`.
- help, 17a: canvas shows `history  List saves, newest first` / Bolt's fact: history lists the saves on your line, back from where you are (terminal.md §12 `bolt history`) / built: `List the saves on your line, newest first`.
- help, 17a: canvas shows `goto  Switch to another line or save` / Bolt's fact: `switch` is a Git word, which help never names (decision 116; `help_never_names_a_git_word`) / built: `Go to another line or save`.
- help, 17a: canvas shows `undo  Undo your last Bolt command  (and: redo)`, the companion dim / Bolt's fact: undo takes back the last step; help is plain text, with only its titles bold (terminal.md §11) / built: `Take back your last step   (and: redo)`, not dim.
- help, 17a: canvas shows `shelf  Set unsaved changes aside  (and: unshelf)`, dim / §11: only titles are styled / built, not dim.
- help, 17a: canvas shows `discard  Throw away unsaved changes in a file  (undoable)` / Bolt's fact: with no paths, discard throws away every unsaved change (review item 4) / built: `Throw away your unsaved changes; undo brings them back`.
- help, 17a: canvas shows `show  Everything about one save` / matches / built: `Show everything about one save`.
- help, 17a: canvas shows `import  Import a Git repository` / Bolt's fact: it brings in the repository in this folder, in place (review item 1) / built: `Bring in the Git repository in this folder`.
- help, 17a: canvas shows `verify  Check that everything stored is intact` / matches / built: `Check that everything Bolt stored is intact`.
- help, 17a: canvas shows the start, setup and tidy rows / match / built.
- help, 17a: canvas shows `Help on one command: bolt help <command>      Version: bolt --version` / matches (`--version` is clap's) / built with 4 spaces between.
- help, 17a note: canvas says "Command names are padded to 10; descriptions start at column 15" / the canvas's own drawing starts them at column 11 / built: padded to 10, descriptions at column 11.
- help, 17a note: canvas says "bolt alone prints this even inside a project" / matches; exit code: §11 doesn't give one, and today it is 2 (clap's `DisplayHelpOnMissingArgumentOrSubcommand`) / built: the overview on stdout, exit code 2 (see Chose 2).
- help, 17b: canvas shows `bolt save: save your changes as a new save on the current line` / Bolt's fact: a save away from a line needs `--line`; the description is the command's own `about` in `main.rs`, so it can't drift from it / built: `bolt save: save the current state of your files: all of them, or only these files, folders and patterns (such as "*.png" or "art/**/*.png")`.
- help, 17b: canvas shows the example `bolt save "Add double jump"` / Bolt's fact: help may name no Git word, and the check reads every word, so the message `Add` counts as `add` (decision 116; `tests/git_words.rs`) / built: `bolt save "Double jump"`.
- help, 17b: canvas shows `bolt save "Fix aim" Assets/Scripts/Enemy.cs   save only these paths` / matches / built.
- help, 17b: canvas shows `bolt save --amend "Better message"   change your last save instead` / Bolt's fact: amend replaces your line's newest save and keeps its change ID (`main.rs` `--amend` help) / built: `Replace your line's newest save, keeping its change ID`.
- help, 17b: canvas shows `bolt save "Big import" --yes   no questions (for scripts)` / Bolt's fact: `--yes` answers only the large-save question; the new line's name needs `--line` (terminal.md §10) / built: `Save without asking, however much it adds`, plus a `--line` example.
- help, 17b: canvas shows the usage `bolt save [message] [paths...] [options]` / Bolt's fact: the usage comes from clap's definitions (task call) / built: `bolt save [OPTIONS] [MESSAGE] [PATH]...`.
- help, 17b: canvas shows `-m, --message <text>` / Bolt's fact: there is no `-m` on save (decision 120; review item 12; `save_itself_still_refuses_dash_m`) / not built; the test checks it never appears.
- help, 17b: canvas shows `--line <name>  When not on a line, start this line for the save` / the option exists; its words are clap's (`main.rs`) / built: `--line <NAME>  At a save rather than on a line, the name of the new line this save starts`.
- help, 17b: canvas shows `--yes  Answer yes to any question; never asks` / Bolt's fact: `--yes` answers the large-save question only (terminal.md §10; `main.rs`) / built: clap's `Save without asking, however much the save adds`.
- help, 17b: canvas shows `--accept-crlf  Allow Windows line endings in text files` / Bolt's fact: it applies only to files Git converted, after an import (review item 16; `main.rs`) / built: clap's words.
- help, 17b: canvas shows `--allow-lfs-pointers  Allow Git LFS pointer files (usually a mistake)` / no such flag (decision 090; review items 5 and 16) / not built; the test checks it never appears.
- help, 17b: canvas shows `--amend`, `--json` / exist / built with clap's words.
- help, 17b: canvas shows no `-h, --help` / clap gives every command `-h, --help` / built (it is an option the command takes).
- help, 17b: canvas shows no Arguments section / Bolt's fact: what N, NUMBER, SAVE and TARGET mean is only in clap's argument help / built: an `Arguments` section between Usage and Options, when a command takes arguments (see Chose 3).
- help, 17b: canvas shows `Without a message, Bolt opens your editor (BOLT_EDITOR, then EDITOR).` / Bolt has no editor support (no `EDITOR` anywhere in the code) / not built.
- help, 17b: canvas shows `See also: bolt status, bolt changes, bolt undo` / terminal.md §11 defines the page as description, examples, usage and options / not built (see Chose 4).
- help, 17b note: canvas says "bolt save --help and bolt save -h print the same page" / matches, and `bolt help save` too (task call) / built and tested.
- help, 17c: canvas shows `error: "sav" is not a Bolt command.` and `hint: did you mean: bolt save "Add double jump"`, `All commands: bolt help` / Bolt's fact: a command line Bolt can't use keeps clap's own error and tip on stderr with exit code 2 (decision 120 exit codes; the task's "errors keep their words") / not built: clap prints `error: unrecognized subcommand 'sav'` and `tip: a similar subcommand exists: 'save'`.
- help, 17c note: canvas says "Suggestions use edit distance 2 against Bolt words and the Git-word shortcuts, so 'comit' suggests 'bolt save'" / Bolt's fact: Git's words are accepted as hidden shortcuts and explained only when used (decision 116); a suggestion naming them would be new behaviour, not printing / not built.
- help, "What changed" list: canvas says "Command help: … a two-column layout with the command column padded to the longest example" / matches / built.

Cross-cutting, carried item 4:

- 18f: canvas shows `bolt stash` → `Shelved … as shelf 1 "On main at ptvz, 3 files"` and `hint: in Bolt this is: bolt shelf` / Bolt's fact: the hint says `in Bolt, stash is shelf, which takes new files too: bolt shelf` once (decision 116); the default shelf message is `Shelved` (review item 7) / built, with the second hint line of carried item 4.

### Added by the fix rounds

**`save`, `status`, `changes`, `history`, `show`:**

- show, 8a: canvas shows `After        kxqmz "First playable"` / naming the parent by a short change ID could read every save of the line, and could warn `change_id_not_shortened` where `show` never did (controller's ruling, fix round 1; bolt-vcs-core history.rs `short_change_id`) / not built: `After` shows the first 12 hex digits of the fingerprint, which `bolt show` takes back, then the message's first line.
- history, 7a: canvas sizes the columns to the widest value present / `history` holds at most 1,000 rows or 1 MiB of their escaped text (controller's ruling, fix round 1) / built: columns from what is held; later rows run long when wider.

This replaces my Chose 13 (the parent by change ID); Chose 7 now reads "the first 1,000 rows or 1 MiB of their text".

**`goto`, `discard`, lines, undo, shelves, `tidy`:**

- goto, 9a note: the canvas shows `goto main` when already there as `Already on line main at save ptvz "Add double jump". Nothing was changed.` (exit 0) / no step is recorded (switch.rs "Already there: nothing to record"), so no `bolt undo` may be offered; Bolt tells by reading `state/head` before and after (controller's ruling, fix round 1) / built `Already on line main, at save <id> "<title>".` (`Already at save <id> "<title>".` at a save), with no "Nothing was changed." line, as the ruling words it. This replaces my first-round line for 9a note.
- shelf, 13a: Bolt's stdout used to say `… bolt unshelf brings them back.` / the design puts the next step on stderr, on a terminal only (terminal.md §6; controller's ruling) / built: the advice is now `Next: bring them back with: bolt unshelf`, terminal only, and no longer on stdout. An output change for logs and scripts.
- undo, 11e: the canvas has one shelf made by `--shelve` / each step of `undo N --shelve` can make its own (undo.rs `take_steps`) / built: a `Shelved …` line per shelf, and with more than one the `Next:` names `bolt shelf --list`.
- tidy, 15b: Bolt said `Nothing to free` also when it found something to free but could remove no pack / built `Nothing could be freed: storage still holds X.` then.

## Deferred

What this half of milestone 6 left for later, each a row of [the follow-ups list](2026-11-01-milestone-1-follow-ups.md#deferred-by-milestone-6-presets-and-polish-2026-11-10), given here by who or when and its first sentence. None of them loses data. The eight rows [decision 123](../decisions/123-follow-ups-done-or-moved-past-v0.1.md) moved past v0.1 are in that list's earlier tables, each with its reason in the decision; three it moved had no row (a warning about files that look like secrets, `--debug`, and a progress bar for `save`). The speed session left one row for this half, which also waits: a compression rule in `bolt.toml` for file types that are already compressed, after v0.1.

- **When scripts need it:** No error code in `--json`.
- **When scripts need it:** A warning carries its paths only inside its `message`.
- **Known limit, measure:** `verify`'s memory grows with the number of objects (roughly hundreds of bytes for each: several GB at ten million objects), as `tidy`'s does.
- **Known limit:** A clock that ran ahead during a `tidy`, then was set back, with the undo window made longer afterwards.
- **Known limit:** A link planted at `.bolt/state/tidied` stops `tidy` (the write is refused before anything is removed) until someone deletes the link by hand.
- **Known cost, measure (speed):** How long a reader takes before it refuses a deep, narrow tree of shared listings.
- **Known cost, measure:** Very wide listings.
- **Known limit:** The project folder is judged by its path (who owns it, and its identity for `safe-folders`) and then opened by its path.
- **Known cost, measure:** How long `verify` takes on a pack of objects that each claim a huge size and decompress to it: an object may claim up to 1 TiB, and zstd turns about 24 MB into 1 TiB of zeros, so it reads and hashes all of it before it finds the object false; on the order of 20 seconds of `verify` per megabyte planted.
- **When a width table is worth a dependency:** Wide characters (most Chinese, Japanese and Korean, and many emoji) count as one column, so a table row of them can pass the terminal's width.
- **Known limit:** Lines that aren't lists or tables are never cut, so an outcome line with a long title runs long ([terminal.md](../terminal.md) §7).
- **Known limit:** A save message that ends like a line marker (`  [main]`) can pass for one in `bolt history` when colour is off.
- **Known limit:** A `safe-folders` entry can't vouch for a folder on a drive that gives no file numbers (some WebDAV and SMB shares); before, such an entry worked by comparing paths ([decision 125](../decisions/125-a-safe-folders-entry-is-judged-by-one-look.md), [security.md](../security.md) T23).
- **If real users hit it:** An optional `core.excludesFile` (`:(optional)…`) that is a dangling link.
- **Known limit:** A repository whose own Git config turns sparse checkout on is refused even when `GIT_CONFIG_COUNT` turns it off again, where Git would go on.
- **Known limit:** `bolt history` sizes its columns from its first 1,000 rows or 1 MiB of their text, since it never holds a whole line ([terminal.md](../terminal.md) §12); a later row with a wider ID, time or author runs long.
- **Known limit:** `bolt show <12 digits>` finds a save only when some line's history, along first parents, reaches it.
- **Known limit:** The guide's output examples are checked by reading, not by its test, which checks that each command it shows succeeds ([testing.md](../testing.md)).
- **Before the first release:** `bolt.exe` hasn't been run on a clean Windows.
- **Known limit:** `bolt.toml` has no cap on how many rules it holds, and the `rules_ignored` warning names at most 20 of those passed over; neither bounds the work of reading a file full of rules that are passed over.
- **Should do:** `read_rules` opens the project's top folder again by its path rather than through a handle the repository already holds.
- **Known limit:** A `bolt setup --preset` whose rule files all changed between its plan and its write writes nothing and says so, yet its step is in the undo diary, as for any switch that left files alone.
- **Should do:** `tidy` can print `Freed 0 B  (1 pack removed, 1 written)` when it found something to free, removed a pack and wrote one, and storage didn't shrink: its figure beforehand is only an estimate ([terminal.md](../terminal.md) §12).
- **Should do:** In `verify`, an object that only the walk of what is needed finds damaged shows under "objects are damaged" just below a check mark for the stored objects, because that first line counts only the pack check's own failures.
- **Known limit:** An explanation of a Git word runs long on a terminal narrower than the command it names: commands are never broken.
- **For the owner to rule:** `bolt` alone prints the overview on stdout and exits with 2 (before, it was clap's help on stderr). 0 may be friendlier.
- **The speed session:** `bolt start --preset code` in a folder of tens of thousands of names is slow because of the first save's walk: the walk's speed is the speed half's subject ([benchmarks.md](../benchmarks.md)).
