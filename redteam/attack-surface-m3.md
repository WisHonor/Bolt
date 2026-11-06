# Milestone 3 attack surface (red-team session, decision 024)

Milestone 3 makes Bolt write into the project folder. `goto` and `discard` switch the folder to another save, `status` and `changes` read it, and `save` can now save part of it, start a line, and skip files a cache vouches for. Pack files got a new version, and small packs are merged.

Threat list: `docs/security.md` §4 (T1–T24). Decisions 045–066 in `docs/decisions/` give the rules this code follows, and `docs/design.md` §5 summarises them. Your goals:
- lose or corrupt data: unsaved work, saved history, files outside Bolt;
- read, write, remove or follow a link outside the project, or inside `.git` or `.bolt`;
- make Bolt store what it shouldn't, or miss what it should (a wrong `status`, a save that leaves a change out);
- crash, hang, or exhaust memory or disk;
- trick the person at the terminal (spoofed output, misleading messages).

## Where the code is

- **Handle-relative file access:** `crates/bolt-vcs-platform/src/dir.rs`, `dir/unix.rs`, `dir/windows.rs` (the only `unsafe`). Every folder is opened from its parent, never following a link.
- **Names:** `crates/bolt-vcs-core/src/names.rs` (`validate_name`, `validate_path`, `writable_here`, `.git`/`.bolt`/`.gitmodules` spellings and short names, length limits), `drive.rs` (case and Unicode probe, `compared`), `scope.rs` (which paths a command takes).
- **The walk:** `snapshot/walk.rs` (held handles, at most 128 folders; lookers on 6 threads reading through the handle that listed each folder), `snapshot.rs` (save, status, partial saves in `snapshot/only.rs`, the large-save question).
- **Caches:** `keyed.rs` (the key in the user's settings folder; the seal), `changeids.rs`, `filecache.rs` (racy rule against the drive's own clock). A cache must only ever be a hint.
- **Write-out:** `writeout.rs` (`plan`, `apply`: folders top down, files through `.bolt/tmp` then rename, removals deepest first, links last; each entry checked against what the plan expected right before it is touched; case-only renames; placeholders for links that leave), `journal.rs` (the switch journal, `state/journal`), `switch.rs` (`goto`, `discard`, safety copies, collisions per decisions 050 and 065), `lines.rs`.
- **Finishing a stopped switch:** `writeout::finish_interrupted`, called by every changing command after it locks.
- **Packs:** `pack.rs`, `pack/v2.rs` (on-disk index), `store.rs` (lazy opening, at most 16 open, copy order, passing over packs that can't be read), `merge.rs` (decisions 057, 061, 062, 066).
- **`changes`:** `changes.rs` (the `similar` crate, 64 MiB and 1,000,000-line limits, 2 s deadline per file), `crates/bolt-vcs/src/output.rs` (escaping, streaming).
- **CLI:** `crates/bolt-vcs/src/main.rs`.

## Places worth attacking

- **Write-out names:** `.git`/`.bolt` spellings in every form (case, macOS-ignored characters, NTFS `:stream`, trailing dots and spaces, 8.3 short names including hashed ones), Windows device names, `\` and drive letters inside names, Unicode twins (NFC/NFD), names over 255 bytes, paths over 4,096 bytes. A hostile save or a hostile `state/journal` can carry any of them.
- **Links:** a link written earlier in the same switch, or planted by the user, used as a folder by a later write; a link replaced by a folder of the same name; targets with `..`, absolute paths, UNC paths, `\` on Windows. Bolt must never read a link's target on disk.
- **Races:** swap a folder, a file or `.bolt/state` between Bolt's look and its act (plan to apply, walk to read, check to rename). Fault points (`--features fault-injection`, `BOLT_FAULT_AT`) can stop Bolt at named steps; `act_point`s let a test change the folder mid-command.
- **Crashes:** stop a `goto` or `discard` at every fault point, then run any command. Every file must be wholly old or wholly new, and the next changing command must finish the switch. A planted or damaged journal must never be acted on.
- **Unsaved work:** make `goto` or `discard` destroy unsaved work without a safety copy; make `status` or the file cache hide a change (same size, same time, racy window, clock skew, FAT's 2 s times); make a partial save store a file it wasn't asked to, or skip one it was.
- **Caches:** plant `cache/files` or `cache/change-ids`, sealed with another key or with the user's own key copied from another project; damage `cache-key`.
- **Packs:** a version 2 index that lies (counts, fan-out, offsets, sizes); compression bombs; many packs sharing one object; a pack that is a FIFO or link; packs that make every merge expensive.
- **`changes`:** huge or pathological files, binary files, control and escape sequences in lines, file names that fake a header.
- **Terminal output:** names or line names with escape sequences or bidi characters in every message and `--json` document.

## Limits already known (not findings unless you show more)

`docs/security.md` lists known limits for T1–T24 and `docs/plans/2026-11-01-milestone-1-follow-ups.md` the deferred items. Among them:
- a same-size rewrite with the time set back exactly is accepted as unchanged (decision 011);
- write-out time grows with the square of folder depth;
- several commands still panic on a broken pipe;
- a case-only folder rename keeps the old spelling on a case-blind drive.
