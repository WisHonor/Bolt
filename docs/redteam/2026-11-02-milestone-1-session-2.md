# Red-team report: milestone 1 fixes (session 2, decision 024, 2026-11-02)

Proof tests: `crates/bolt-vcs/tests/redteam2.rs`. 9 Linux tests, all fail as claimed in the no-network sandbox; 1 Windows/macOS test, not run in the session; on 2026-11-02 it failed as claimed on GitHub's Windows and macOS machines, run on the unfixed code from a throwaway branch (commit 34ce08e). That run also showed that the two RT2-1 tests and the RT2-6 test can't fail on macOS, which doesn't enforce their `ulimit -v` memory cap; their proof is the Linux run. Each asserts the SAFE behaviour.
Scripts: `redteam/attacks2/`. Every command below runs in the sandbox (`bash redteam/sandbox.sh run …`, working dir `/work` = this copy), except `planted-root.sh`, which drives the container from the host through `docker exec` (root's steps use `-u root`).
Run all tests: `cargo test -p bolt-vcs --test redteam2`. In the sandbox, on the unfixed code, it ended `test result: FAILED. 0 passed; 9 failed; 1 ignored` (full output: `redteam/attacks2/redteam2-test-output.txt`; the ignored one is a planting helper for the scripts). The line numbers in that file (`redteam2.rs:229` and so on) are those of the attacker's copy; the test file was formatted and marked ignored when it was added to the repository (369c891) and has changed since, so only the test names still match.
Release timings come from `/tmp/t/release/bolt` (`cargo build --release -p bolt-vcs`). The sandbox's 4 GB memory limit stands in for "the machine ran out of memory".

Counts: Critical 0, High 3, Medium 4 (RT2-9 confirmed after the session, on Windows and macOS), Low 2.

The fixes for RT-1–RT-11 held: no `.bolt` link, special file, oversized file, planted parent repository or TOML trick got through (see "Held up"). The findings below are in code the fixes did not touch, or in what the new `.boltignore` reader keeps in memory.

## Findings

- **RT2-1 High (T7, T11: size bomb in the history walk).**
  - **What.** `history::walk` (`line_history`) holds every save on the line in memory, message included. A message may be 1 MiB (or a save up to 512 MiB, through move notes), and zstd stores a 1 MiB repetitive message in ~300 bytes. Memory therefore grows without bound with the chain's length, about 3,400× the repository's size.
  - **Who is hit.** `bolt history`, `bolt show` (it calls `short_of` and `line_history`) and the victim's own `bolt save`. `save` calls `short_of` *after* `commit`, so the save lands and then the command dies.
  - **Proof (tests).** `rt2_1_history_of_a_tiny_repository_does_not_need_gigabytes` and `rt2_1_save_in_a_repository_with_a_long_history_does_not_die_after_committing` (2,000 saves, a 610,834-byte repository, 1.5 GB cap):
    `bolt history` … `peak_mb: 1459, stderr: "error: This project's Bolt data looks damaged: pack …pack (the zstd data of … is invalid: Allocation error : not enough memory)`, and for save `committed anyway: true`.
    Under memory pressure, a healthy repository is reported as *damaged*, with the hint "Nothing was changed. Ask for help before trying to repair it" (see RT2-8).
  - **Proof (script, release, no cap but the sandbox's 4 GB).** `sh redteam/attacks2/history-bombs.sh`:
    ```
    N=500  (192 KB on disk)  history: exit 0, 0.6 s, peak 506 MB
    N=1000 (340 KB on disk)  history: exit 0, 1.3 s, peak 1006 MB
    N=6000 (1828 KB on disk) history: exit killed by signal 9, 7.9 s, peak 4078 MB
                             save mine: exit killed by signal 9, 6.4 s, peak 4079 MB
    ```
  - **Fix.** Never hold the whole line.
    - `history`: stream it, printing each save as it is read.
    - `short_change_ids`: compute it from a first pass that keeps only `(Id, ChangeId)`.
    - `short_of` after a commit: needs only change IDs, never messages. Better still, don't walk the line at all after committing (see RT2-8).
    - Consider a format limit on the bytes of all saves one command may hold, or count saves walked against a limit, as listings are.

- **RT2-5 High (T19/T7: uncapped read in `.bolt`).**
  - **What.** `PackReader::open` reads a pack's whole index into memory (`read_vec(&file, index_offset, len − 48 − index_offset)`), limited only by the file's length. The trailer's index offset can point just past the header, so the "index" is the whole file. Every command opens every pack (`Repository::open` → `Store::open`).
  - **Why the existing defence misses it.** A sparse file costs nothing on disk and survives `tar --sparse`. A zip carries 5 GB of zeros in ~5 MB. `try_reserve_exact` does not help on Linux: overcommit grants the allocation, and `resize` then touches every page.
  - **Proof (test).** `rt2_5_a_sparse_pack_does_not_make_every_command_allocate_its_size`: a 2 GiB sparse pack → `bolt history` … `peak_mb: 2056` … `(the index checksum does not match)` after 14 s.
  - **Proof (script).** `sh redteam/attacks2/sparse-pack.sh /tmp/t/release/bolt 5`:
    ```
    pack: 5368709120 bytes apparent, 8 KB on disk
    tar --sparse -czf of .bolt: 944 bytes
    /tmp/t/release/bolt history: exit killed by signal 9, 4.6 s, peak 4070 MB
    ```
  - **Fix.** Put a limit on the index in the storage spec (for example, index length ≤ 64 × entries + header, with the entry count capped), and refuse a larger one *before* reading it. Or hash and parse the index streaming from the file, in bounded chunks, and keep only the parsed entries. Session 1's capped-read rule (T19) should name pack indexes explicitly.

- **RT2-7 High (T9, new: deduplication trusts objects nobody checked).**
  - **What.** `store_file` and `build` skip storing content or a listing whose ID `store.contains`. `PackReader::open` checks the pack's structure and index checksum, but no object's fingerprint, until that object is read.
  - **The attack.** A pack planted in a copied project (zip, tar, shared drive) can claim the ID of content the victim will add later. Content an attacker can predict: a standard licence, a vendored library, an engine DLL, a stock asset, an empty file.
  - **What happens.** The victim's `bolt save` reports success but never stores the bytes. The save points at garbage, and the content can't be recovered from Bolt even after the planted pack is deleted (the object is then missing).
  - **Proof (test).** `rt2_7_save_stores_a_file_even_if_a_planted_pack_claims_its_id`. The planted `00.pack` passes `bolt history` (every check made when a project opens). Then:
    ```
    `bolt save` said "Saved mplt  \"Add licence\"  (1 file)\n", but LICENSE can't be read back: Err(Pack(FingerprintMismatch { … }))
    ```
  - **Variant (not run).** A lying object at the empty listing's ID would poison every empty folder in later saves (same `!store.contains(&id)` in `build`).
  - **Related.** `Store::register` lets the first-registered pack win, and `00.pack` sorts before Bolt's 64-hex names, so a planted pack also shadows existing objects (used by RT2-8's test).
  - **Git parity.** Git trusts existing objects the same way, which is why this is not Critical. Bolt's own principle, "hash verification on every read and every received object", is not met for a copied repository.
  - **Fix.** Before relying on an existing object to skip a write, verify it once: read and hash it, then remember it as verified for this process. Or verify each pack fully the first time Bolt registers it, and record that in `cache/`. A mismatch should store the user's own copy and report the damaged pack.

- **RT2-2 Medium (T11: CPU bomb in `resolve`).**
  - **What.** `resolve` with 8 or more hex digits walks the whole history once for *every* line (`for tip in entry.state.lines.values() { walk(repo, *tip) }`). A diary entry may hold 100,000 lines (`MAX_LINES`), all pointing at the same chain. The cost is lines × chain length, and the chain length has no limit.
  - **Proof (test).** `rt2_2_show_by_fingerprint_does_not_walk_the_history_once_per_line` (99,999 + 1 lines, 300 saves, a 135,611-byte repository):
    ```
    `bolt show 0123abcd` was still walking after 60 s on a 135611-byte repository
    ```
  - **Proof (script, release).** `sh redteam/attacks2/history-bombs.sh`:
    ```
    1,000 lines  → show 0123abcd: exit 1, 0.5 s
    10,000 lines → show 0123abcd: exit 1, 37.3 s
    ```
    At the format's maximum, `sh redteam/attacks2/resolve-max-lines.sh` (output in `resolve-max-lines-output.txt`):
    ```
    on disk: 172 KB
    /tmp/t/release/bolt history: exit 0, 0.1 s, peak 29 MB
    show 0123abcd: exit 1, 319.1 s, peak 29 MB
    ```
    Ten times the chain makes it ~53 minutes.
  - **Fix.** Walk each distinct save once across all lines, with a visited set. Better, match fingerprint prefixes against the store index (`Store::index` keys) instead of walking history at all. Count saves walked against a limit.

- **RT2-6 Medium (T11/T7: memory in the new `.boltignore` reader).**
  - **What.** The scan keeps the compiled rules of every folder on its way down (`IgnoreRules::stack`). One 1 MiB `.boltignore` of character-class patterns compiles to ~17 MB retained (145 MB peak while compiling). Nested ones add up, and 4,096 levels are allowed (~70 GB). The victim only runs `bolt save` in a project they unpacked.
  - **Proof (test).** `rt2_6_nested_boltignore_files_do_not_exhaust_memory_in_save`: 100 nested folders, each with a 1 MiB `.boltignore`, 1.5 GB cap. Result: `code: None … stderr: "memory allocation of 8388608 bytes failed"` (abort). *(After the fix, this test expects a clean refusal that names `d/.boltignore`: decision 028's 1 MiB limit along a path makes this layout's save fail by design. It still fails on the unfixed code.)*
  - **Proof (script).** `sh redteam/attacks2/boltignore-nested.sh`, then `DEPTHS=256 sh …`:
    ```
    depth  1 (13 KB as .tar.gz): save x: exit 0, 0.5 s, peak 145 MB
    depth 16 (207 KB as .tar.gz): save x: exit 0, 3.9 s, peak 408 MB
    depth 32 (415 KB as .tar.gz): save x: exit 0, 5.0 s, peak 682 MB
    depth 256 (3347 KB as .tar.gz): save x: exit killed by signal 9, 46.4 s, peak 4077 MB
    ```
    `boltignore-cost.sh`: one file of `*a*b*c…z*N` patterns peaks at 572 MB, then fails with "error building NFA" (save refused cleanly).
  - **Fix.** Limit the `.boltignore` bytes or patterns *per save*, across all files (or along one path), not only per file. Or match with a non-compiling wildmatch, as Git does, whose memory is the size of the patterns.

- **RT2-8 Medium (new: a committed save reported as not made).**
  - **What.** After `repo.commit`, `save` still calls `short_of`, which walks the whole line. Any error there is printed through `RepoError::hint`. For damage, that hint says "Nothing was changed. Ask for help before trying to repair it", although the save *was* committed. `repo.rs`'s own note says callers "must not report 'nothing changed'" after a commit.
  - **Proof (test).** `rt2_8_a_committed_save_is_never_reported_as_nothing_changed`. The oldest save is damaged (a lying `00.pack`); the line's newest save is fine:
    ```
    the save was committed, but Bolt said:
    error: This project's Bolt data looks damaged: object ebd2b19b… does not match its fingerprint.
    hint: Nothing was changed. Ask for help before trying to repair it.
    ```
  - **Same message in RT2-1.** There, an allocation failure inside zstd is mapped to `PackError::Corrupt`, so a *healthy* repository is called damaged.
  - **Effect.** The user retries, re-clones or "repairs" a healthy project.
  - **Fix.**
    - Compute the short ID before committing, or from the new save alone.
    - After a commit, never return an error that claims nothing changed; print "Saved …" and a separate warning.
    - Map zstd's allocation errors to "not enough memory", not to damage.

- **RT2-9 Medium, confirmed after the session (T12, new: `.boltignore` is case-sensitive where Git is not).**
  - **What.** On Windows and macOS, Git sets `core.ignorecase=true`, and `.gitignore` patterns then match case-insensitively (`*.env` ignores `PROD.ENV`, `secrets/` ignores `Secrets/`). `boltignore_rules` builds a `GitignoreBuilder` without `.case_insensitive(true)`, so Bolt stores those files: secrets the user meant to keep out.
  - **Proof.** Test `win_rt2_9_boltignore_ignores_case_like_git_on_case_insensitive_systems` (`#[cfg(any(windows, target_os = "macos"))]`, not run in the sandbox; it failed as claimed on GitHub's Windows and macOS machines on 2026-11-02, commit 34ce08e). It expects only `.boltignore` and `readme.txt` saved; on those systems Bolt should also save `PROD.ENV` and `Secrets/key.pem`, which fails it.
  - **Fix.** On case-insensitive systems, or always on Windows and macOS, build the matcher case-insensitively, and spec it in storage-format.md §12.

- **RT2-3 Low (T21: invisible format characters pass `text::escape`).**
  - **What.** `escape` covers Cc, the bidi controls and U+2028/2029, but not other characters that "change what a terminal shows … without showing anything itself" (its own doc comment). Missed: U+200B (zero-width space), U+2060–2064, the deprecated bidi-shaping controls U+206A–206F, U+FEFF, U+00AD, U+180E, and the tag characters U+E0000–E007F (which can carry hidden ASCII text, e.g. to an AI agent reading `bolt show`). Two different file names print identically.
  - **Proof (test).** `rt2_3_invisible_characters_in_names_and_messages_are_not_printed_raw`:
    ```
    printed raw: ["U+200B", "U+2060", "U+2063", "U+206A", "U+206E", "U+FEFF", "U+E0041", "U+180E", "U+00AD"]
      added     setup.py  (4 B)
      added     setup​.py  (6 B)    ← 9 more lines that look exactly like this
    ```
  - **Fix.** Escape every format character (general category Cf), except U+200C/U+200D, which scripts and emoji need, plus the tag block and the Hangul fillers (U+115F, U+1160, U+3164, U+FFA0). Keep the one-place rule.

- **RT2-4 Low (T12, new: `.boltignore` is not Git's syntax).**
  - **What.** storage-format.md §12 promises "exactly the `.gitignore` pattern syntax", but `globset` reads `{a,b}` as a choice, where Git reads braces literally. With `{prod,dev}.env` in `.boltignore`, Bolt **stores** the file literally named `{prod,dev}.env`, which Git ignores, and drops `prod.env`, which Git keeps (silently not saved).
  - **Proof (test).** `rt2_4_braces_in_boltignore_are_literal_as_in_git`:
    ```
    Git ignores `{prod,dev}.env` and keeps `prod.env`; Bolt saved [".boltignore", "{prod,dev}.env"]
    ```
  - **Proof (differential script, 34 cases against real Git).** `python3 -I redteam/attacks2/boltignore-vs-git.py`.
    - Braces are the only case where Bolt stores what Git ignores.
    - Bolt ignores files Git keeps for `secret.txt<TAB>`, `[abc` and `a[/]b`.
    - Bolt refuses the whole save for `foo\` (Git accepts the line).
  - **Fix.** Escape `{` and `}` with a backslash before `add_line` (globset honours `\{`). Better: a wildmatch-compatible matcher (Git's own rules), with the differential script kept as a test against Git.

## Unproven
- **Soft wrap beats `show`'s indentation.** A message line padded to the terminal width makes the wrapped part start at column 0, where it can read as `  added …`. Git has the same limit. Not run.
- **The trust hint offers `/tmp`.** For a `.bolt` that another account planted in `/tmp`, the hint suggests `safe-folders = ["/tmp"]`, one paste away from the CVE-2022-24765 scenario (seen in `planted-root.sh` output). Git does the same. Suggestion: refuse, or warn, for world-writable sticky folders.
- **Trust can move.** `safe-folders` entries are resolved with `canonicalize`, which follows links. Whoever owns a trusted folder can replace it with a link to `/tmp`, moving the user's trust there. I believe Git 2.46+ resolves `safe.directory` the same way. Not run.
- **RT2-7 with the empty listing.** A lying empty-listing object would break `show` for every later save with an empty folder. Same code path; not run.
- **Windows: a `.BOLT`-cased project folder** (unpacked from an archive) opens, but every `save` refuses it as a `.bolt` spelling. Annoyance only; not run.
- **Not security.** Read-only commands open `state/lock` for writing (`clean_tmp`). So a trusted repository that the user can't write fails every command with a bare "Permission denied (os error 13)", which doesn't name the file (seen in `planted-root.sh` with a root-owned, trusted `/tmp/.bolt`).

## Held up (don't repeat)
- **T23, planted repositories (Linux, `planted-root.sh`).**
  - A root-planted `/tmp/.bolt` is refused for `history`/`save` below it, even when empty and when it is a root-owned symlink to `/etc`. Nothing is followed.
  - `start` below them works, and the new project's own `.bolt` wins.
  - `safe-folders` spellings with a trailing `/`, `..` or a symlink match exactly the canonical folder.
- **Project `.bolt/config.toml` nesting** (`toml-depth.sh`): 500,000 `[`, 250,000 `{a=` and 450,000 dotted keys under the 1 MiB cap. `save` exits 0; the toml parser refuses the depth, and Bolt ignores the file.
- **Crash at every fault point** (session 1's `crash-sweep.sh` on the fixed code, `--features fault-injection`; output in `crash-sweep-output.txt`): all 5 of save's and all 6 of start's (the sweep tried 12 and 8 positions). History, the next save and `show` all work afterwards, and `tmp/` is left empty.
- **`.boltignore` vs Git** (`boltignore-vs-git.py`): `!` re-include, anchors, `**`, `a/**/b`, `[!…]`, escaped `#`/`!`/space, trailing spaces, CRLF, BOM, subfolder files and `..` all match Git. `.bolt`/`.git` folders can't be re-included.
- **One hostile `.boltignore`**: a regex blow-up is refused cleanly ("error building NFA"). Only the nesting (RT2-6) hurts.
- **Code reading.**
  - RT-5's open-without-following plus handle checks: a FIFO, folder or socket swapped in gives `ChangedDuringSave`.
  - `check_bolt`'s second look after the owner check.
  - `start`'s re-check after creating `.bolt`.
  - `clean_tmp`'s name and kind rule (hard links lose only a name).
  - The writer and the reader count depth and entries identically (4,096 and 10,000,000).
  - The streaming diff's order.
  - No panic from indexing or arithmetic on hostile data in the library.
  - Every untrusted piece in error messages is escaped; `SaveError::TooLarge` and `PackError::Corrupt` carry only Bolt's own text or are re-escaped.
