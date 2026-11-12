# The final builds: attack surface (red-team session RT16, decision 024)

Two rules of format 1 changed just before it freezes at v0.1. Both are about bytes a stranger can write: a pack in a shared `.opalro` folder, and a name in a save.

1. **zstd is exactly one frame** ([decision 154](../docs/decisions/154-format-1s-zstd-is-exactly-one-frame.md)). An entry stored with compression 1 must be one zstd frame, as the spec defines it:
   - it starts with the magic `28 B5 2F FD`;
   - it has no `Dictionary_ID` field, and the frame header's bits 3 and 4 are clear;
   - its window is at most 2^27 bytes;
   - its content size, if it gives one, equals the entry's size;
   - its checksum, if present, matches;
   - it unpacks to exactly the entry's size, and nothing follows it.

   Before this change, the reader also read several frames in a row, passed over skippable frames, and decoded the pre-1.0 formats v0.4 to v0.7. zstd is now built without its legacy decoders. The rule is in [storage-format.md](../docs/storage-format.md) §11.
2. **More short names of `.opalro` are refused, on every system** ([decision 155](../docs/decisions/155-more-short-names-of-opalro-are-refused.md)). These are `opalro~5` to `opalro~9`, and NTFS's hashed short name of `.opalro`, `OP9975~1`, in the form `git fsck` refuses for `.gitmodules`: up to the six characters of `op9975`, then `~` and a counter (`op997~12`, `~1234567`). Before, only `opalro~1` to `opalro~4` were refused. The rule is in §3.

Your job is to break either rule: find bytes or names that get through, two paths that disagree, or a crash, hang or runaway memory on the way. Threat list: `docs/security.md` §4. The rules: decisions 059, 148, 152, 154 and 155 in `docs/decisions/`.

Your goals:
- **A frame the spec refuses, read anyway,** by any path: reading a small entry (one read, one-call decoder) or a large one (streamed); `opalro verify`; `opalro tidy` (which copies stored bytes as they are, after checking them); a switch (`goto`); `status`, `changes`, `show`; a chunk list, a chunk, a folder listing, a save, a diary entry; a store with the same ID in two packs, a good copy and a bad one, in each order. The candidates are many:
  - two frames;
  - a skippable frame first, last or between;
  - a pre-1.0 frame;
  - a dictionary ID of 0 or more;
  - the reserved bit or the unused bit set;
  - a window over 2^27, or a single-segment frame whose content size is over it;
  - a content size that disagrees with the entry, or none;
  - a wrong checksum;
  - a frame cut short;
  - a trailing byte;
  - a frame whose blocks claim more than its window;
  - a header that ends at the end of the stored bytes.
- **A good frame refused:** anything Opalro itself writes (level 3, with and without a content size, an object compressed in one go or a file streamed) that the new reader refuses would break every project. Check the boundaries: an empty entry, an entry of exactly 1 MiB, a window of exactly 2^27.
- **The two paths disagreeing:** a small entry and a large one must be refused with the same words, and a frame accepted on one path must be accepted on the other.
- **Cost:** a frame that passes the header check but makes the decoder use memory or time out of proportion, a header that makes the check itself read past the stored bytes, or a refusal that leaves the reader holding a lot.
- **A name the guard misses:** a name that Windows, macOS, a FAT stick, or an NTFS drive written from Linux takes for `.opalro`, that `is_opal_alias` (`crates/opal-vcs-core/src/names/lookalike.rs`) lets through. Also a refused spelling that some path doesn't check: a save, an import from Git (top and below), a write-out, a journal, a shelf, `undo`, `redo`. Try every counter form, letter case, trailing dots and spaces, `:stream`, characters macOS ignores (before the `~` too), and names that start with `~`.
- **A name refused that shouldn't be:** an ordinary file name now treated as `.opalro`, for example one starting with `~` (editors' backup files, `~$document.docx`), or `op9975…` names that aren't short names.
- **The build:** check that the release binary has no zstd legacy decoder in it, by behaviour (feed it a well-formed v0.5, v0.6 or v0.7 frame) and, if you like, by its symbols.

## Where the code is

- **zstd:** `crates/opal-vcs-core/src/pack.rs`: `read_small`, which uses `one_plain_frame` and the one-call decoder; `decode_into`, the streamed decoder, which now uses `single_frame()` and checks what is left after the frame; and `plain_frame_header`. `crates/opal-vcs-core/Cargo.toml` has `zstd` with `default-features = false`. Tests: `pack::tests::a_small_entry_is_refused_as_a_streamed_one_is` and `only_one_plain_frame_goes_to_the_one_call_decoder`.
- **Names:** `crates/opal-vcs-core/src/names/lookalike.rs` (`is_opal_alias`, `OPALRO_HASHED`, `is_hashed_short_name`, `may_spell`); `names.rs` (`validate_name`, `writable_here`); their callers in `snapshot/walk.rs`, `snapshot/only.rs`, `writeout.rs` and the import.
- **Making packs by hand:** `crates/opal-vcs/tests/redteam2.rs` (`plant_huge_copies`) writes a version 1 pack byte by byte. The pack format is in the spec's §11.

## How

- Build a release binary in the sandbox (`cargo build --release -p opal-vcs`; the program is `target/release/opalro`) and attack that. Unit-level probes against `opal_core` are fine too.
- Write each proven finding as a Rust test that fails now, in `redteam/attacks15/`, with the exact command or test that shows it. Write the report to `docs/redteam/2026-11-12-v0.1-final.md`: each finding with a severity (Critical, High, Medium, Low), what it costs a user, and its proof; and what held, with numbers (how many frames and names you tried).
- Attack code runs only in the sandbox. Nothing here needs Windows or macOS.

## Limits already known (not findings unless you show more)

- The follow-ups list's "Known limit" and "Known cost" rows, and the earlier red-team reports' known costs (`docs/redteam/`).
- Refusal vectors for zstd: the spec's test vectors have none. The review before the freeze listed that among the findings the owner left as they are (decision 148).
- A frame Opalro never writes, but which a hand could make and which the spec allows (a content checksum, a single-segment frame), is meant to be read.
