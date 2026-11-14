# Red-team report: milestone 7, lines and tags, second session (RT18, decision 024, 2026-11-14)

Code attacked: a copy of `milestone-7` at d149d80, as briefed: the four fix commits on top of 26e910a (one order for a name; a line or server line that is also a tag at another save refused; `line delete` naming a holder it can't read; odd tag names between quotes, and no stored tag name in a hint's command). Built with `--all-features` in the no-network sandbox (container `opal-redteam`, user `attacker`, 4 GB, 4 CPUs), debug build throughout.

Proof tests: `crates/opal-vcs/tests/redteam18.rs`, 6 tests, each marked `#[ignore = "RT18-n: …"]` and asserting what should be true. `bash redteam/sandbox.sh run sh redteam/attacks17/run.sh proofs` runs them with `--ignored`: `test result: FAILED. 0 passed; 6 failed` (whole output in `redteam/attacks17/proofs-output.txt`). Without `--ignored` the file passes (6 ignored with `--all-features`, 5 without, since RT18-4's proof needs the faked terminal and is gated on `fault-injection`). The file passes `rustfmt --edition 2024 --check` and `cargo clippy -p opal-vcs --all-targets --all-features -- -D warnings` in the sandbox, and uses nothing gated to one system, so it compiles on Windows too.
Platform tests: none. No attack depended on letter case, junctions or the Windows console, so there is no `redteam18_platforms.rs`.
Attack code: `redteam/attacks17/`: the probes `rt18_explore.rs.txt` and `rt18_explore2.rs.txt` (kept uncompiled; `run.sh explore` copies them in, runs them and removes them), their outputs `explore-1.txt` to `explore-3.txt`, and `rss.py`, which reports a command's peak memory.

Counts: Critical 0, High 0, Medium 1, Low 5. No data loss, no question skipped and no name taken for the wrong save were found. The fixes hold where they were aimed. What breaks is the advice around them: the refusal and two hints send a person to text that Opalro then doesn't take, and one hint outside milestone 7 still carries stored text inside a command.

## Findings

- **RT18-1 Medium (the IDs a line-or-tag refusal gives don't reach a save that no line reaches; T39; `history.rs` `find`, `by_fingerprint_prefix`).**
  - **What.** Decision 162 says the refusal names both saves by 12 hex digits "which `goto` and `show` take", and the hint says "To reach one of them, type its ID as shown here." But a fingerprint is only looked for among the saves the local lines reach by first parents. A server line's save is often past every local line (the server is ahead), and an imported tag can sit on a commit no branch reaches any more. For those, the 12 digits, and even the whole 64-digit fingerprint, give `No save, line or ID matches`.
  - **Who is hit.** A Git import where `origin/main` is ahead of `main` and a tag is named `main@origin` (RT17-1's case) at another save: `goto main@origin` is refused, the server line's ID is refused, and no text at all reaches the server line's save while the tag exists. The hint for a server line names no other way out; `opalro tag --delete main@origin` is it, and nothing says so. The same with a branch `v1` and a tag `v1` on a commit of a deleted branch: the tag's ID is refused (`explore-1.txt`, probe p2); `line rename` is hinted and does get the person out.
  - **Proof.** `the_ids_a_server_line_or_tag_refusal_gives_reach_both_saves`: the refusal says `server line 495d093271df`, and `goto 495d093271df` says `No save, line or ID matches "495d093271df".`
  - **What I would do.** Let a fingerprint also find the saves that server lines and tags reach (they are holders already for `line delete` and `tidy`), at least when it is 12 digits or more; or name in the refusal only IDs that `goto` will take, and for a server line give the way out that works (`opalro tag --delete`, with the name not carried, as decision 162 says for hints).

- **RT18-2 Low (`line delete` doesn't name a tag whose save is there but whose older saves aren't; T9, T39; `lines.rs` `unreadable_holder`).**
  - **What.** The RT17-3 fix looks, after the walk fails, for a holder whose own save can't be read. A planted tag at a save that is in the store but whose parent isn't makes the walk fail one step further on, and the look finds nothing: `line delete feat --yes` says `error: object 5555…5555 is not in the store`, no hint, `"hint":null` with `--json`, and no word of the tag. Nothing is written (the proof checks the diary head and the packs).
  - **Proof.** `line_delete_names_a_tag_whose_history_cant_be_read`.
  - **What I would do.** Name the holder whose walk failed rather than look again afterwards (the walk knows which mark reached the missing save), or fall back to the damage message with its hint and the words "a tag, line or server line names a save whose history can't be read". The extra look is cheap: with 100,000 tags at such a save, `line delete` took 1.4 s in a debug build (`explore-2.txt`, q4).

- **RT18-3 Low (a quoted tag name still puts a line's mark on another save in `history`; T21, T39; `history_output.rs`).**
  - **What.** The quotes mark where the name ends, but the mark's own brackets are outside them. A planted tag named `x]  [main]  [tag y` at the first save makes its row read `Start project  [tag "x]  [main]  [tag y"]`: there is a `[main]` in the row, and with colour on the whole name is painted the blue a line's mark gets. RT17-5's proof only looked for `[main]` with the old name (`v9]  [main`), whose quoted form ends `[main"]`, so it passes while the attack works with a name one character longer.
  - **Proof.** `a_quoted_tag_name_cannot_put_a_lines_mark_on_another_save`.
  - **What I would do.** Inside a quoted name, show `[` and `]` escaped as well (or quote with characters a mark never uses), and have the proof look for any `[main]`, not one spelling. Only a planted project gets here: Git refuses `[` and spaces in tag names.

- **RT18-4 Low (after `show` of a save off the current line, the next step it gives can't find the save; T39; `main.rs` `show`).**
  - **What.** At a terminal, `show` ends with `Next: see its lines with: opalro changes --save <short change ID>`. The short change ID is unique on the current line, but a change ID is looked for only on the current line, so for a save on another line, a tag's or a server line's, the command it gives fails: `show rel` (tag on line `feat`, project on `main`) says `opalro changes --save lsqo`, and that says `No save, line or ID matches "lsqo".` It was so before milestone 7 for another line's name; tags and server lines (now taken by `show`, RT17-1) make it the common case.
  - **Proof.** `the_command_shows_hint_gives_finds_the_save_shown` (needs `--all-features` for the faked terminal).
  - **What I would do.** Give the name the person typed when it was a line or tag, or the fingerprint's 12 digits, when the save isn't on the current line.

- **RT18-5 Low (`goto`'s advice to keep saving on a line names a line `goto` then refuses; T39; `goto_output.rs`).**
  - **What.** With line `v1` and an imported tag `v1` at another save, `goto v1` is refused, and its hint says to type the line's ID. Doing so works, and then says `To keep saving on v1: opalro goto v1`, which is the refusal again. The person goes round in a circle; the way out (`line rename`) is only in the first message.
  - **Proof.** `gotos_advice_to_keep_saving_on_a_line_can_be_followed`.
  - **What I would do.** Leave that line out (or point at `line rename`) when the line's name is refused, which `find` can be asked before the advice is printed.

- **RT18-6 Low (`unshelf`'s hint carries a stored shelf label inside a command, unquoted; T21, T26; `switch.rs` `SwitchError::hint`).**
  - **What.** The class of RT17-4, outside tags. A shelf's label is stored text, read back with no rule on its characters. When a shelf can't come back because of unsaved changes, the hint is `… then run opalro unshelf <label> …` with the label escaped for control characters only. A planted shelf labelled `1; touch pwned` gives `then run opalro unshelf 1; touch pwned (a new shelf would be the newest).` Pasted, that runs `touch pwned`. Opalro itself only makes labels that are numbers, so only a planted project does this; it predates milestone 7, but decision 162's rule ("no stored name inside a command in a hint") doesn't hold across the program.
  - **Proof.** `the_hint_to_bring_back_a_shelf_carries_no_stored_label_in_a_command`.
  - **What I would do.** As decision 162 did for tags: "run the same command again", or carry the label only when it is a number.

## Suspicions (no proof; never findings)

- **A damaged tag list now stops every `goto`, `show` and `changes --save`, even by ID.** `find` reads the tag list before it looks at IDs, and after a line to check for a tag of its name. With the tag list's record missing, `goto main`, `history` and `tag --delete` all stop with the damage message ("Ask for help before trying to repair it"); `save` still works, and `undo` of the step that planted it got the project back (`explore-1.txt`, p3). Refusing on damage is the rule, so this is a judgement: before milestone 7, `goto main` didn't read the tags. The damage message doesn't mention that `undo` can help.
- **Every `goto`, `show` and `changes --save` now holds the whole tag list twice.** With 7,900 tags of 64 KiB messages (a list just under the 512 MiB record limit, which an honest user can make), `goto main` peaked at 1.0 GB and took 1.2 s, `show` 1.0 GB, `history --json` 1.0 GB, `line delete` 1.0 GB, and `tag --json` 2.0 GB and 16 s (debug build, `explore-3.txt`). Decision 162 accepted "one more record per goto"; whether a gigabyte for `goto main` is acceptable is the owner's call. Looking a name up needs only the names, not the messages.
- **Wide characters.** `cut_end` keeps as many characters as there are columns of room, so a message of wide characters can run past the width. The tag list's own name column counts columns correctly. Already known (RT17's "held" notes it); not pursued.

## What was tried and held

- **One order.** `goto`, `show`, `changes --save` and `tag --save` all go through `find`; `unshelf`'s check of its base also uses it. No other path that takes a save's name was found (`undo --to` takes a step, not a save). Line, server and server-line names are all held to the line-name rules when a list is read, so none holds `@`, and `a@b@c` can't split two ways.
- **The refusal.** It can't be dodged by a tag whose save is missing (the IDs are compared, not read), and with a tag list that can't be read it fails closed. Its 12 digits name the right saves. A tag `main` at another save than line `main` (the line you're on) refuses `goto main` and `show main`; `history`, `save`, `line` and `tag` still work, and `line rename` gets out. `line rename`, `line delete`, `tag <name>` and `tag --delete` take their own kind's name and don't refuse.
- **Letter case.** A tag `Main` beside line `main` is a different name everywhere; neither passes for the other.
- **`line delete` writes nothing before it stops**, whether the holder or its history can't be read (proof checks diary head and packs).
- **Odd names on screen.** Names of only spaces, `\`, `a, b`, the zero-width joiners and wide characters show between quotes or as they are, in the list, `history`, `show`, both questions and the outcome lines; the escapes aren't sliced by `cut_end` (`escape_len`). No name ended a question early or made a second row (`explore-2.txt`, q1). Two names that differ only by a zero-width joiner and non-joiner both show as `""` with the invisible character inside: they look alike, but a person can't type either by mistake into the other's command.
- **Two commands at once (read only).** Every new step (`line new`, `rename`, `delete`, `tag`, `tag --delete`) reads `current()` after taking the lock, so a racing `save`, `goto`, `undo` or `tidy` is either fully before or fully after. `tag --save` resolves its save before the lock; under the lock it reads the save again, so a save `tidy` freed in between is refused, not tagged.
- **Tidying after a line delete (read only).** `tidy` marks every line, tag and server line of the current state and of every entry in the undo window (`tidy.rs` `state`), so a save something still names is kept whatever was added or removed in between.
- **A planted diary step with a folder change (read only).** Undo and redo handle a step's folder change by what it holds, not by the command's name, with the same check for unsaved changes in the way as any step (T26, attacked in milestone 4).

## Not done

- Running the tidy-after-the-window and the planted-diary attacks: read only, as above.
- A release build's timings; all numbers are from the debug build.
- 100,000 tags each with a 64 KiB message: such a list is larger than a record can be (512 MiB), so the largest that fits, 7,900, was used.
