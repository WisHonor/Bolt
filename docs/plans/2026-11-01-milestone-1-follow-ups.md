# Milestone 1 follow-ups

> Items the milestone 1 reviews deferred (2026-11-01). Each later milestone's plan must pull in the ones that apply to it. The "must do" items are load-bearing: forgetting them causes a security or data bug.

## Must do

| By | Item |
|---|---|
| Milestone 3 | Writing files out on Windows (`goto`, `discard`) must refuse any name containing `\` or `:` (`PathBuf::join("C:x")` replaces the root), with a test. |
| Milestone 3 | Merge small packs and open packs lazily (each save adds a pack and every pack stays open; already in `milestones.md`). |
| Milestone 3 | Handle-relative walking (`openat`-style), built with write-out: a *folder* swapped for a link while `save` walks or reads the project is still followed (only a link at a file's own name is refused, security.md T20), and a fixed `.bolt/` folder swapped after the shape check (for example `store/packs` before a pack is renamed into it) is too. |
| Milestone 3, before v0.1 | Decide the format's path-length limit with write-out (decision 026). |
| Before v0.1 | Revisit the memory a hostile save inside the limits can cost a reader (about 1.5 to 4 GB) with the speed-bar benchmarks (decision 026). |
| Milestone 4 | Undo must validate safety-copy paths from the diary through `names` (no `..`, no absolute paths) before writing anything. |
| Milestone 4 | Windows rename durability: use `MOVEFILE_WRITE_THROUGH` (platform crate) so a power cut can't lose a just-renamed pack or head. |
| Before networking (v0.3) | Cap the pack index allocation before reading packs received from a server (a hostile pack can ask for a huge allocation). |
| Before v0.1 | Choose a license; add `license` to the manifests and drop the `private` exemption in `deny.toml`. |

## Should do (minor review findings, verbatim from the task reviews)

- Decision 026: `sudo bolt` in the user's own project is refused, because Bolt then runs as root and the project belongs to the user. Trust `SUDO_UID` as Git does, in a later milestone.

- Task 1: minor (deferred): deny.toml advisories only implicit; ci.yml lacks `permissions: contents: read`, actions tag-pinned not SHA-pinned; push+pull_request duplicate runs (plan-mandated).
- Task 2: minor (deferred): from_hex/from_letters negative paths thinly tested; collision test covers 4 of 7 domains; ChangeId::random untested; to_hex allocates per byte.
- Task 3: minor (deferred): varint edge tests ([0x80,0x80,0x00], 10th byte 0x00/0x81), round-trips for bytes/id/change_id/bool/u64/i64, limit-before-input test; consider a property test that every non-minimal re-encoding is rejected.
- Task 4: minor (deferred): NTFS hashed short names (BOxxxx~1) not covered (same as Git); ignorable-range edge tests; folder_problems drops exact duplicates silently; Windows-only collisions (a vs a.) and HFS ignorable collisions not reported; `con:` flagged only as forbidden char; NameError messages lack the name/next step.
- Task 5: minor (deferred): rejection tests assert only is_err (not the specific reason); no tests for size rule on kinds 5/6, Listing::new rejections, >MAX_ENTRIES in new, other kinds in proptest; serde/serde_json loosely pinned.
- Task 6: minor (deferred): no tests hitting the 4 KiB person/path and 256-byte key limits.
- Task 7: minor (deferred): decoder I/O errors mapped to Corrupt; copy_decoded writes size+1 bytes before failing (spec wording "never more than recorded size"); unbounded index allocation on hostile pack (DoS; cap before network receive); Windows rename durability comment overclaims (consider MOVEFILE_WRITE_THROUGH in platform crate); sync_dir EINVAL on some FUSE/NFS; "unknown compression" message when sizes disagree; streaming truncation test wouldn't catch missing set_len; error messages lack next step.
- Task 1: minor (deferred): owner hasn't picked a license — add `license` to manifests and drop the private exemption; deny.toml now CRLF (mixed line endings in repo under `* -text`).
- Task 8: minor (deferred): pack enumeration duplicated in store/verify; no test that Store::open fails on a corrupt .pack or ignores stray files; verify not exercised on records; wrong-kind tested one direction only. Implementer also removed the "deleting cache/" done-when clause in milestones.md (no cache in M1) — reviewer judged correct.
- Task 9: minor (deferred): stale lock-info can mislead a writer that collides with clean_tmp's brief lock; missing state/head or tmp/ give Io not Damaged; head()/open read files unbounded (cap head at 65 bytes); line-name case-insensitivity uses to_lowercase not full folding, is_alphanumeric admits `½` — check vs decision 012; rejection tests assert only is_err; clean_tmp remove_file and state/lock creation bypass fsio.
- Task 10: minor (deferred): hide_path fails for Windows paths >260 chars (bolt start can't finish in very deep folders) — add \?\ prefix in platform crate; ancestor walk swallows read errors via is_file(); discover with relative start; config read errors reported as "not valid"; toml line/col dropped; Invalid/AlreadyAProject have no hint; &letters[..4]; small test gaps (error:/hint: prefixes, stderr empty with --json, double crash).
- Task 11: minor (deferred): pass-2 ContentChanged path untested (size/mtime catches first); changed_files hides count errors as 0; diff.rs lacks folder→file and kind assertions; nested .bolt skip untested.
- Task 11: minor (deferred): a dangling-symlink .boltignore is treated as absent (patterns silently dropped) — consider erroring via symlink_metadata; sockets/pipes/devices skipped silently.
- Task 12: minor (deferred): hex resolve misses saves reachable only via second parents/undone; ambiguity candidates printed in full; no CLI ambiguity test.
- Task 12: minor (deferred): sanitize shows \r/\t as \x0d/\x09 (CRLF messages ugly); bidi/line-separator chars (U+202E, U+2028) pass through.
- Task 13: minor (deferred): ~24/38 corpus names skipped on Windows (OS can't hold them); skip lines only visible with --nocapture; fuzz corpus cache entries accumulate; push trigger on milestone-1 must change after merge.
- Final: deferred — Windows rename durability (MOVEFILE_WRITE_THROUGH) to add to milestone 4 list; StoreError::Missing lacks hint/damage framing; Repository::head() doc note (use current() for store reads); pack.rs temp-file removals outside fsio; init now refuses a garbled head in an unfinished .bolt (untested).
