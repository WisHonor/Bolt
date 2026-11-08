# Red-team report: milestone 4, second session (decision 024, 2026-11-08)

Code attacked: a copy of `milestone-4` at 8722285 ("RT6-2, round 1: one check, and new paths looked at again too"), after the fixes for RT6-1, RT6-2 and RT6-4 (decision 079).
Proof tests: `crates/bolt-vcs/tests/redteam7.rs`. 2 tests, each marked `#[ignore = "RT7-n: …"]` and each asserting the SAFE behaviour. Both were run in the no-network sandbox and fail as claimed, four runs out of four. `rt7_1` is Linux-only: it watches how far Bolt has read into a file through `/proc`. `rt7_2` runs everywhere. No Windows-only attack was found, so nothing waits on the Windows runners.
Run them: `cargo test -p bolt-vcs --features fault-injection --test redteam7 -- --ignored` (or `redteam/attacks6/run-proofs.sh`, which runs it in the sandbox). In the sandbox, on the code attacked, it ended `test result: FAILED. 0 passed; 2 failed; 0 ignored; 0 measured; 0 filtered out; finished in 4.00s`. Without `--ignored` the file passes (2 ignored). The file is formatted with `rustfmt --edition 2024`.

Counts: Critical 0, High 0, Medium 1, Low 1.

The three fixes held where they were aimed. The RT6-2 fix covers files a switch *writes*, but not files it *removes*: a removal still trusts a check that can take as long as reading the whole file. And `tidy`'s rule for the undo window treats a missing save as damage, so changing the window setting can stop `tidy` for a month.

## Findings

- **RT7-1 Medium (unsaved work lost; the RT6-2 gap, still open for removals).**
  - **What.** Before a switch removes a file, `Run::remove` looks at it (`look`): it takes the file's details, then reads the whole file to hash it, and removes it if the hash is the saved one. Nothing is checked after the read. The RT6-2 fix added a second look only to `write_file`, which compares the details taken before the read with the details just before the rename. `remove` has no such look. So whatever another program writes into the part of the file already read is never seen, and the file is then deleted, with no safety copy. The gap is the whole read: tens of milliseconds for 64 MiB in the sandbox, and seconds for the multi-gigabyte files Bolt is built for. When the file cache vouches for the file, nothing is read and the gap is only an instant; it doesn't vouch right after a switch wrote the file, nor for a file another tool copied in. `make_folder` and `link` remove a file after the same kind of look, with the same gap (code reading).
  - **Who is hit.** Anyone running `undo`, `redo`, `goto`, `discard` or `unshelf` that removes a big file while a program writes into it in place: a database, a virtual-machine disk, a game engine's asset cache, an editor that saves without making a new file.
  - **Proof (test).** `rt7_1_a_file_edited_while_undo_checks_it_is_never_removed` (Linux): `data.db` (64 MiB) is saved, then deleted and saved again; `goto` the first save writes it back. Then `undo`, which removes it. The plan reads the file once; as soon as the switch's own read of it is past the first megabyte, "the database" writes 16 bytes in place at the start:
    ```
    undo: Some(0)
    Undid goto: back on main at oxul, 1 file removed.

    the edit made during the undo survived: false
    ```
    The undo exits 0 and says nothing, and the edited file is neither on disk nor in the store.
  - **Fix.** Do in `remove` (and before `make_folder` and `link` remove a file) what `write_file` now does: right before removing, compare the details again with `same_file` against what `look` saw before it read, and leave a file that differs as `changed_since_plan`. A test for each of the three would keep them from drifting apart. The instant left is the one row 70 of the follow-ups already accepts.

- **RT7-2 Low (`tidy` stopped for up to a month, with a false "damaged" message; T27).**
  - **What.** Decision 077 keeps, for every diary entry within the undo window, what an undo of it needs. The window is read from the settings each time. Shorten it (`undo-window-days = 1`), run `tidy`, and `tidy` rightly removes what only older steps needed: the work a `discard` threw away, say. Then lengthen it again (to the default 30). Those older steps are back inside the window, and `tidy` must keep what it removed. It can't read it, so it stops with "This project's Bolt data looks damaged … Ask for help before trying to repair it." It goes on stopping until those steps leave the window again, which takes up to the whole window. Nothing is damaged, and nothing is lost that `tidy` hadn't been told to free. Meanwhile `undo --list` still offers the old `discard` as a step to undo.
  - **Who is hit.** A user who tries a short window to free space and then goes back, or who copies the setting between computers. They are told their project is damaged, and the store can't be tidied for weeks.
  - **Proof (test).** `rt7_2_raising_the_undo_window_after_a_tidy_leaves_tidy_working`: a save, then a `discard` of an edit. `tidy` runs (through the library) three days later with a one-day window and removes packs. Then `bolt tidy` runs with the default window:
    ```
    tidy: Some(1)
    error: This project's Bolt data looks damaged: save 4f02b7c7…a727 can't be read (object 4f02b7c7…a727 is not in the store).
    hint: Nothing was changed. Ask for help before trying to repair it.
    ```
  - **Fix.** For what is kept only because a step is in the window, a record that isn't in the store anymore isn't damage: `tidy` can skip it, maybe with a warning. Only what the current state and the diary from `head` reach must stop it. Or record in the store the oldest time a `tidy` has kept the window from, and judge the window from the later of that and the setting. `undo` of such a step should then say it is past what was kept, not that the store is damaged. Either needs a decision refining 077.

## Not proven

These are suspicions without a failing test. None counts as a finding.

- **A damaged entry in a cut-off diary now stops `tidy`.** Since RT6-1, `tidy` walks every diary end in the window back to its first entry. One unreadable entry on that walk (a damaged pack, a planted end with a missing parent) stops `tidy` as damage, where before it stopped only for damage on `head`'s diary. A damaged end itself is skipped. Consistent with "anything it must keep but can't read stops it", so not counted.
- **The second look on Windows trusts `ChangeTime`.** `same_file` compares the file ID, size, modified time and `ChangeTime`. T25 already notes that any program that may write a file on Windows can set its `ChangeTime` back. Such a program could slip past the RT6-2 look too. Not tested; it needs a deliberate program, not an accident.

## Held up

Each of these was tried in the sandbox unless marked as code reading, and Bolt behaved correctly:

- **The RT6-2 look for files written (code reading).** An edit during `look`'s own read changes the details taken before it, so the look before the rename sees it. A file whose details are unchanged is never refused: hard-linked tracked files are looked at one at a time, after the earlier one's rename, and the case-renaming path takes the details again after its own rename. New paths and links are looked at again where the plan found nothing.
- **`tidy`'s diary walk (code reading, and the RT7-2 run).** Ends are found among the entries the packs list. A diary sharing a past with `head` is walked once per state: an entry walked without its state kept is walked again when an end needs its state. Undo and redo targets, their parents' states and folder changes are kept as for `head`.
- **The journal's front (code reading).** `read_front` reads at most 73 bytes, through the same regular-file open as the full read, and `unfinished_warning` only needs the target and `from`. Changing commands still read it whole, which is row 69's known cost.
