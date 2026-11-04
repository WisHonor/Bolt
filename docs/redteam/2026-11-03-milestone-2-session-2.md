# Red-team report: milestone 2 fixes (session 2, decision 024, 2026-11-03)

Code attacked: this copy of milestone 2 with the fixes for RT3-1 to RT3-10 (the report it carries ends with RT3-5's "Fixed further in 6899abd").
Proof tests: `crates/bolt-vcs/tests/redteam4.rs`. 4 tests, all run in the no-network sandbox, all fail as claimed. One is Linux-only (`rt4_4`, which measures peak memory through `/proc`); the other three run everywhere. No Windows-only or macOS-only attack was found worth a gated test: NTFS's case folding was probed by hand instead (see "Held up"). Each test asserts the SAFE behaviour.
Scripts: `redteam/attacks4/`, each with its captured output in a `.txt` beside it. They reuse session 1's `redteam/attacks3/gitrepo.py` (a byte-exact Git writer) and `redteam/attacks2/measure.py` (exit, time, peak). Every command below runs in the sandbox (`bash redteam/sandbox.sh run …`, working dir `/work` = this copy); timings and peaks come from `/tmp/t/release/bolt` (`CARGO_TARGET_DIR=/tmp/t cargo build --release -p bolt-vcs`).
Run all tests: `RUST_TEST_THREADS=1 cargo test -p bolt-vcs --test redteam4`. In the sandbox, on the code attacked, it ended `test result: FAILED. 0 passed; 4 failed; 0 ignored; 0 measured; 0 filtered out; finished in 94.59s` (full output: `redteam/attacks4/redteam4-test-output.txt`). The test file is formatted (`rustfmt --check`) and passes `cargo clippy -p bolt-vcs --tests --all-features -- -D warnings`.

Counts: Critical 0, High 0, Medium 1, Low 4.

The fixes held where they were aimed: no sparse index, early or long zlib stream, `.git` spelling, shared change ID or include got through (see "Held up"). The findings are what the fixes leave beside them: disk an import fills through commits Git stores as deltas (RT3-2's limit covers parents only), work the RT3-4 allowance never counts, a `.boltignore` that switches RT3-6's warning off, a third buffer RT3-8's claim leaves out, and the packs RT3-1's cap doesn't count.

## Findings

- **RT4-1 Medium (T7, T11: what Git stores as deltas, an import stores whole; RT3-2's limit covers only parents).**
  - **What.** Decision 043 limits the parents an import may add because each save keeps its parents whole while Git stores a merge as a ~60-byte delta. Every other part of a commit is kept whole too, and so is every tree (as a listing). Here N commits in a chain share one random 1 MB message (letters and digits, so valid UTF-8): Git's pack holds it once, each later commit a ~80-byte delta on the first that inserts its parent line. Each save holds its own copy, which zstd shrinks only to about three quarters (730 KB a save). There are no merges, so the parent limit never applies.
    - The only bound left is RT3-4's work allowance. Each commit is rebuilt twice (pass 1 and pass 2), about 1 MB of work each, so the 4 GiB minimum lets about 2,100 such commits through: about 1.5 GB of `.bolt` from under 1 MB of Git.
    - Every byte the import inflates raises that bound by 500 bytes of work, so a bigger repository raises it with its size, and the holes of RT3-4's second look raise it for free (see Suspicions). Trees amplify the same way, and are rebuilt only once each.
    - design.md §5.16 says of parents that such a chain "would otherwise fill the disk; within the limit they take at most about 35 MB". That holds for parents; the disk fills anyway.
  - **Who is hit.** Anyone importing a repository from someone else, including a plain `git clone` from a hostile server: the pack arrives as it was sent.
  - **Proof (test).** `rt4_1_a_small_history_without_merges_does_not_import_into_a_huge_bolt` (100 commits; the bound is RT3-2's test's, 40 MiB, for a repository of about 1 MB):
    ```
    `bolt import` of a 765269-byte Git repository (100 commits, no merges) wrote a 74747365-byte .bolt: Ended { code: Some(0), timed_out: false, secs: 8.313490115, peak_mb: 17, stderr: "" }
    ```
  - **Proof (script).** `sh redteam/attacks4/messages.sh /tmp/t/release/bolt 100 2000 3000` (`messages.txt`):
    ```
    N=100 commits sharing one 1 MB message: .git 784 KB on disk   → exit 0, 0.9 s, peak 11 MB;  .bolt 73768 KB
    N=2000 …: .git 976 KB on disk, tar -czf of .git: 888343 bytes → exit 0, 12.1 s, peak 11 MB; .bolt 1460692 KB
    N=3000 …: .git 1076 KB on disk → exit 1, 14.3 s: "… more than the 4 GiB of work Bolt allows …"; .bolt 28 KB
    ```
    The new `.bolt` is about 1,500 times the Git repository. A stopped import leaves nothing behind (28 KB), so only an import that succeeds fills the disk.
  - **Fix.** An import-wide limit on what it writes, relative to what it read (say, a few GiB plus a multiple of the compressed bytes inflated), stopping before writing past it as the parent limit does; or an owner ruling that accepts whole records per save, with the bound stated (about half the allowance for commits, all of it for trees). The lasting fix is delta-compressing records in Bolt's packs, a format question. Either way, §5.16 and decision 043 should say the 35 MB covers parents only.
  - **Ruled in decision 044: whole records are accepted until chunked storage; proof test stays ignored.** Reading the code confirms that a full disk stops the import cleanly. Every write error comes back as an error, and `.bolt/format`, which commits the project, is written last and atomically, so `after_begin` reports an unfinished import: its hint says nothing was committed and that Bolt kept the `.bolt` folder it began, for a later run to finish from or to be deleted. `an_interrupted_import_can_be_run_again` injects a write error at every fault point of an import and checks exactly that. design.md §5.16 and security.md T7 now say that an import can need far more disk than the Git repository (about 1,500 times here), bounded only by the rebuilding allowance, and §5.16 says the 35 MB covers parents only. `rt4_1_…` stays ignored, citing the ruling.

- **RT4-2 Low (T11: `Repo::header` walks a delta chain outside the work allowance).**
  - **What.** RT3-4's fix counts inflating and applying deltas, "checked before each step". `Repo::header` doesn't rebuild, but `packed_header` calls `chain(…, &mut |_| None)`: it walks the whole chain of a deltified object, one positional read of an entry header at a time (up to 4,096), without the base cache and without counting.
    - Pass 1 (`roots`) asks for the header of every tag's target. Its `seen` remembers tag objects only, so each lightweight tag on a commit walks that commit's chain again. 100,000 tags (the limit) on a commit 4,096 deltas deep read 409,600,000 entry headers.
    - The same walk runs once for each distinct blob (`Converter::blob`) and twice for each commit (`read_commit`, in both passes), so a history whose objects all sit at the end of deep chains pays it for every object, outside the allowance.
  - **Who is hit.** Anyone importing a repository from someone else, `git clone` included: it fetches the tags on the commits it fetches, and a pack's delta depth is the sender's choice (Git's own `pack.depth` allows 4,095).
  - **Proof (test).** `rt4_2_tags_on_a_commit_deep_in_deltas_import_in_reasonable_time` (the same 100,000 tags on a commit stored whole import at once, checked first):
    ```
    `bolt import` of 100,000 tags on one commit 4,096 deltas deep (a 6133777-byte Git repository) was still running after 20 s: Ended { code: None, timed_out: true, secs: 20.019684074, peak_mb: 46, stderr: "" }
    ```
  - **Proof (script).** `sh redteam/attacks4/deep-tags.sh /tmp/t/release/bolt 1000 10000 100000` (`deep-tags.txt`):
    ```
    T=1000 tags on a commit 4,096 deltas deep: .git 328 KB    → exit 0, 4.5 s
    T=10000 …: .git 848 KB, tar -czf of .git: 162870 bytes      → exit 0, 42.9 s
    T=100000 …: .git 6032 KB, tar -czf of .git: 390186 bytes    → exit 0, 407.5 s, peak 40 MB
    T=100000 tags on a commit stored whole: .git 5808 KB        → exit 0, 0.3 s
    ```
    About 4 ms a tag, 1,360 times the whole commit's import, with no allowance in sight.
  - **Fix.** Count the entry headers a walk reads against the allowance; or keep each entry's type and size once known (both are fixed by the entry), so a header never walks a chain twice; and have `roots` remember the commits it has looked at, not only tag objects.
  - **Fixed** in 492b3b3, all three. A header walk keeps what it learns of each entry it passes, by pack and offset: the type at the bottom of its chain and how deep the entry is, and what it builds once a header asked for it. At most 524,288 entries are kept (about 43 MB); past that a walk is counted, not remembered. Keeping the depth means a chain over 4,096 deep is still refused. Every walk down a chain, a header's or a rebuild's, counts 1 KiB of work for each entry header it reads past the first, against the same allowance; on git/git that adds 221,056 reads, 0.2 GiB of its 10.5 GiB. `roots` remembers the commit, tree or blob each tag reaches, not only tag objects, so 100,000 tags on one commit look at it once. This also covers the "through blobs and commits" suspicion below. `rt4_2_…` passes on Windows: the test, both imports, takes 3.4 s in a debug build.

- **RT4-3 Low (T12: a `.boltignore` from the repository takes back what `.gitignore` keeps out, so RT3-6's warning never comes).**
  - **What.** RT3-6's fix warns when a save takes a change to a saved file "an ignore rule matches". A folder's `.boltignore` lines come after its `.gitignore` lines and may undo them (decision 038), and a Git repository can track a `.boltignore` like any other file. With `.gitignore` saying `.env` and `*.pem` and `.boltignore` saying `!.env` and `!*.pem`, Git ignores both, and for Bolt no rule matches either.
    - The user's real `.env` (over the tracked placeholder) is saved without a word: RT3-6 exactly, with one more file.
    - A new `deploy.pem` is saved too. Git never adds it, not even with `git add -A`; Bolt has no add step.
    - The import says nothing about the `.boltignore`, and `bolt save` names no files.
    - security.md T12 says "after a Git import the project's `.gitignore` files apply too … so a first save keeps out what Git kept out", and "a repository that tracks a placeholder `.env` and ignores it can't have the user's real one saved without a word". Both are false for this repository.
  - **Who is hit.** As RT3-6: anyone who imports a repository from someone else, then keeps a secret in the project folder trusting its `.gitignore`. The new-file case needs no tracked placeholder at all.
  - **Proof (test).** `rt4_3_a_boltignore_from_the_repository_does_not_save_what_git_ignores_silently`:
    ```
    with .gitignore keeping out .env and *.pem, the save took the user's .env ("API_KEY=sk_live_REAL_SECRET\n") and deploy.pem (true), and Bolt said only: Imported 1 commit from Git: 1 line, 0 tags.
    You're on line main.
    Saved nwpl  "Say hello"  (3 files)
    ```
  - **Proof (script).** `sh redteam/attacks4/boltignore-takes-back.sh` (`boltignore-takes-back.txt`): the save says `Saved tkss  "Say hello"  (3 files)`, and `bolt show` lists `modified  .env  (23 B → 28 B)` and `added     deploy.pem  (36 B)`.
  - **Fix.** Judge "would Git leave this out" by the `.gitignore` rules alone, beside Bolt's verdict. When a save takes a new or changed file that Git's rules ignore but a `.boltignore` line takes back, warn and name it, as RT3-6's warning does. Or have the import warn when the repository tracks a `.boltignore` that takes back what a `.gitignore` keeps out.
  - **Fixed** in c96462e, the first way. Each ignore file's rules say whether they come from a `.gitignore`. The matcher gives Git's verdict beside Bolt's, in the same place: where a `.boltignore` line decides, it looks again among the `.gitignore` lines alone. The walk notes what it keeps that Git's verdict leaves out, and everything in a folder Git would leave out. A save that takes such a file or link, new or changed, warns with a new kind, `gitignored_files_saved` (json.md), naming the first five; RT3-6's message didn't fit new files, which no Bolt rule matches. security.md T12's two statements hold again. `rt4_3_…` passes on Windows. The proof's case, run by hand with a debug build, now gives:
    ```
    Saved sxmw  "Say hello"  (3 files)
    warning: Saved 2 new or changed files that .gitignore keeps out but .boltignore takes back:
      .env
      deploy.pem
    Git would leave them out unless they were forced in with git add -f. If they shouldn't be saved, remove the .boltignore lines that take them back and move them out of the project folder; this save keeps them.
    ```

- **RT4-4 Low (T7: a delta is held beside its base and what it builds, three times the object; RT3-8's fix says two).**
  - **What.** RT3-8's report says "the limits allow two (a 512 MiB object and its 512 MiB base)". `unpack` inflates each delta whole too (`inflate_entry`, up to the same 512 MiB) and holds it beside the base and the object it builds while `apply_delta` runs. A delta of inserts is as big as what it builds and compresses as well as it does. So a 300 MiB file built on a 300 MiB base by a 300 MiB delta, a few hundred KB in the pack, peaks at three times its size; at the limits, about 1.5 GiB, plus the 256 MiB base cache. Git's `patch_delta` holds the same three buffers, so the code is at Git's level; the claim is stronger than it.
  - **Who is hit.** Anyone importing a repository from someone else, `git clone` included.
  - **Proof (test).** `rt4_4_rebuilding_a_deltified_file_holds_it_at_most_twice` (Linux; RT3-8's file and bound, built by inserts only):
    ```
    importing one 300 MiB deltified file took 914 MB: Ended { code: Some(0), timed_out: false, secs: 32.909956812, peak_mb: 914, stderr: "" }
    ```
    RT3-8's own case, after its fix, takes 612 MB.
  - **Fix.** Apply a delta as it inflates: its instructions are read in order, so only the base and the result need to be whole. Or refuse a delta much bigger than what it builds. Either way, say three buffers where the claim says two.
  - **Fixed** in 53e6e4b (and 2168483), the first way, so the claim of two holds. A delta is applied as it inflates: its instructions are read in order through a 16 KiB buffer, and only the base and what it builds are whole. Every check stays: the base's size, the result's size before allocating, copy bounds, cut inserts and copies, instruction 0, building more or less than it says, the delta's own size claim (at most 512 MiB, as before), its stream holding exactly that, and the work counted before inflating and before building. The slice form, `apply_delta`, now runs the same code and is built only for `Craft` and the fuzz target. `rt4_4_…` passes in the Linux container: one 300 MiB deltified file imports with a peak of 612 MB (was 914), in 34.9 s.

- **RT4-5 Low (T11: every lookup searches every pack, and 50,000 empty packs pass every check).**
  - **What.** RT3-1's fix caps the objects all packs may list (2^24), not the packs. `objects/pack` may hold 100,000 entries, so about 50,000 packs, and a pack of no objects passes every check: 1,104 bytes, the same each time. `Repo::find` searches every pack in name order for every object, so each lookup pays for all of them; Git looks in the pack that answered last first. Session 1 listed this as a suspicion, unmeasured.
  - **Who is hit.** Anyone importing a repository received as an archive: Git never writes empty packs, and `git gc` merges packs.
  - **Proof (script).** `sh redteam/attacks4/many-packs.sh /tmp/t/release/bolt 20000 0 49999` (`many-packs.txt`):
    ```
    N=20000 files beside P=0 empty packs: tar -czf of .git: 1104076 bytes     → exit 0, 0.4 s, peak 18 MB
    N=20000 files beside P=49999 empty packs: tar -czf of .git: 2862621 bytes → exit 0, 9.6 s, peak 36 MB
    ```
    24 times slower, about 0.23 ms a lookup, growing with every object a repository holds.
  - **Fix.** Skip packs that list no objects; look in the pack that answered last first, as Git does; or cap the packs read, as the folder's entries are.
  - **Fixed** in 2d27681, all three. A pack that lists no objects is checked, then never searched. Each lookup asks the pack that found the last object first, then the rest in order. More than 4,096 packs (Git merges its packs at 50, `gc.autoPackLimit`) are refused before any is read, with a new error whose hint says to run `git repack -a -d`, so the script's 49,999 empty packs would be refused before any is read. The proof is a script, so it wasn't run on this computer (the owner's rule). Instead `objects_are_found_among_many_packs_the_one_that_found_the_last_asked_first` reads 200 objects from 100 packs beside 101 empty ones, and `packs_of_no_objects_are_skipped_and_too_many_packs_refused_before_any_is_read` shows the skip and the cap.

## Suspicions (not proven)
- **RT4-1 without bound.** Tag objects of 1 MiB of hole each, pointing at a tree (skipped, so outside the tag budget), are read once in pass 1 and would each raise the allowance by 500 MiB: room for about 250 more of RT4-1's commits, about 180 MB more `.bolt` apiece, and 100,000 tags are allowed. Not run.
- **Trees amplify more than messages.** A listing is rebuilt once (pass 2 only) where a commit is rebuilt twice, and Bolt's 32-byte IDs are bigger than Git's 20, so a big tree changed by one entry a commit should fill two to three times the disk per unit of allowance that RT4-1's messages do. Arithmetic only; session 1's "big similar trees" suspicion stays open.
- **RT4-2 through blobs and commits.** A million distinct blobs at the end of 4,096-deep chains would cost about 67 minutes of header walks at RT4-2's 4 ms each, from a repository of about 70 MB. Not run.
- **The 2^24 cap refuses the biggest real repositories.** Linux's packs list about 10 million objects; Chromium's likely list more than 16.7 million, which Bolt now refuses. No network to check.
- **`EndedEarly`'s hint blames Bolt for a hostile object.** Git's `unpack_loose_rest` zero-fills a loose object whose stream ends before its declared size, so `git fsck` passes such an object below `core.bigFileThreshold`. Bolt refuses it, safely, but says "this may be a fault in Bolt's reader: please report it". No `git` in the sandbox to show Git's side.
- **Peak memory at RT3-1's cap.** An index of 2^24 SHA-256 names keeps about 740 MB (44 bytes an object, as T19 says) and builds a 64-bit offset table of up to 128 MiB beside it, about 900 MB before any object is read. Within the design; not measured.

## Held up (don't repeat)
- **RT3-1 (reading).** The count is checked against the cap, for all packs together, before the length and before any table is read. Names grow only as they pass the order check, so a table of zeros fails at its second name; offsets are reserved only after the names pass; the 64-bit table is bounded by count − 1 rows and read only then. SHA-256 widths are consistent: 32-byte names, 42-byte entry headers, `search::<32>`, 32-byte trailers. No other allocation is sized by a count or size from Git before it is checked: `read_all` and `apply_delta` hold their claims to 512 MiB first.
- **RT3-3 (reading).** Every status of the inflate loop: Buf with bytes written is taken as output; Ok or Buf without progress reads more only once the buffer is used up; anything else is refused. A stream that ends before its declared size is `EndedEarly`; one that holds more is refused by `end`; bytes after a loose object's stream are refused; a preset dictionary is refused by miniz_oxide as damaged data.
- **RT3-4.** Inflating and applying deltas are counted before each step on every path that rebuilds: `read`, `blob_reader`, tag chains, pass 1, the `.bolt` folder warnings. The line-ending check reads only Bolt's store. Chain entries the index doesn't list never raise the allowance, and an entry counts once. A stopped import leaves nothing behind (RT4-1's 3,000-commit run: 28 KB).
- **RT3-2.** Pass 1 at the limit: `todo` holds at most one index for each parent met, so commits plus 1,000,000; a commit's duplicate parents count toward the limit; the limit is checked before its parents are pushed.
- **RT3-5.** A save leaves out `.GIT`, `git~1`, `.git.`, `.git ` and `.git::$INDEX_ALLOCATION`, HFS+'s ignorable spellings, parts after a backslash, links and files named `.git`, in every folder; the last save's listings come without them (the top, kept folders, folders gone from disk), each named once. The one exception is the documented folder kept whole.
  - **Drive folding.** NTFS doesn't fold `.gıt` (U+0131), `.gİt` (U+0130), `.gIт` (Cyrillic), `．git` (fullwidth) or `.git` with U+200C onto `.git`. Probed by hand on this computer's NTFS drive, inside this copy, with PowerShell's `Test-Path` beside a real `.git` folder; no Bolt run, nothing built. HFS+ and APFS have no canonical decomposition onto ASCII `g`, `i` or `t` (only the Kelvin sign decomposes onto an ASCII letter, `K`), so no normalised spelling escapes Git's list.
- **RT3-6.** Files in held folders, links, kind changes (a file that became a link is warned about; one that became a folder isn't kept) and changes to the `x` bit all give the warning. Only RT4-3's take-back gets past it.
- **RT3-7.** The include warning catches `[Include]` and `[includeIf …]` in any letter case, in `config` and `config.worktree`.
- **RT3-9 (reading).** The tag list is written once, at its counted size, and checked a tag at a time; zstd's output while storing it is the only second copy, twice at worst, for a list that doesn't compress.
- **RT3-10.** No other path gives two saves one change ID: header IDs are deduplicated by value, an ID some commit's own ID gives is refused to every header whichever comes first, `bolt save` uses random IDs, and the import's order is fixed, so it stays repeatable.
- **Output.** The `git_names_left_out` warning escapes the joiners (`escape_all`). The import's own list of left-out entries uses `escape`, so `.g\u{200c}it` shows there as `.git`; harmless, since that entry was left out.
