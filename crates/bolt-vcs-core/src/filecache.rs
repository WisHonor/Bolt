//! The file-state cache, `.bolt/cache/files` (storage-format.md section 10): each file's kind,
//! size, modified time, file ID and content ID, so that a file whose details still match needn't
//! be read. Only ever a hint: one that is missing, sealed by another key, edited or unreadable is
//! empty.
#![deny(clippy::indexing_slicing, clippy::arithmetic_side_effects)]

use std::cell::Cell;
use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::io::{self, Write};
use std::path::Path;

use bolt_platform::dir::{Dir, EntryMeta, EntryType};

use crate::encoding::{DecodeError, Reader, Writer};
use crate::id::Id;
use crate::listing::EntryKind;
use crate::repo::{RepoError, Repository};
use crate::{fsio, keyed, safe_fs};

const MAGIC: &[u8; 8] = b"BOLTFILE";
const VERSION: u8 = 3;
const FILE: &str = "files";
/// The most the file may hold, seal included.
pub const MAX_BYTES: u64 = 256 << 20;
/// A path's limit (decision 051).
const MAX_PATH: usize = 4096;
/// The fewest bytes an entry takes: an empty path, then kind, size, two times, file ID, racy and
/// ID.
const MIN_ENTRY: usize = 1 + 1 + 1 + 12 + 12 + 24 + 1 + 32;
const SECOND: i128 = 1_000_000_000;
/// FAT's resolution, the coarsest a drive Bolt runs on keeps (decision 053).
const RACY_NS: i128 = 2 * SECOND;

/// What the cache knows of one file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileState {
    pub kind: EntryKind,
    pub size: u64,
    pub modified_ns: i128,
    /// The change time, so a file written over in place isn't taken for the one recorded.
    pub changed_ns: i128,
    /// The drive's number and the file's on it, so a file moved onto the path isn't taken for it.
    pub file_id: (u64, u128),
    pub id: Id,
}

#[derive(Debug, Clone, Copy)]
struct Entry {
    state: FileState,
    /// Changed too close to when the cache was written to be trusted.
    racy: bool,
}

/// An entry of the body the cache was read from.
#[derive(Debug, Clone, Copy)]
struct Slot {
    /// Where its path is in the body: start and end.
    at: (u32, u32),
    entry: Entry,
    /// Vouched for by this scan, so written again.
    kept: bool,
    /// Recorded again by this scan: `added` holds what the path is now, if anything.
    gone: bool,
}

/// The file-state cache, by project path: the body it was read from, indexed in place, and what
/// this scan recorded.
#[derive(Debug, Default, Clone)]
pub struct FileCache {
    body: Vec<u8>,
    /// In the body's order, which is the paths'.
    slots: Vec<Slot>,
    added: BTreeMap<String, Entry>,
    /// `body` is what `cache/files` holds, so writing the same again can be skipped.
    loaded: bool,
    /// The slot after the last one asked for: files are asked for nearly in the slots' order.
    next: Cell<usize>,
    #[cfg(test)]
    searches: Cell<usize>,
}

impl FileCache {
    /// The cache's body, without its seal.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        self.body_of(true, None)
    }

    /// The body holding every entry (`all`) or only those this scan met, each marked racy if
    /// its file changed after `trusted_before`.
    fn body_of(&self, all: bool, trusted_before: Option<i128>) -> Vec<u8> {
        let wanted = |s: &&Slot| !s.gone && (all || s.kept);
        let count = self
            .slots
            .iter()
            .filter(wanted)
            .count()
            .saturating_add(self.added.len());
        let mut slots = self
            .slots
            .iter()
            .filter(wanted)
            .map(|s| (self.path_of(s), s.entry))
            .peekable();
        let mut added = self
            .added
            .iter()
            .map(|(path, entry)| (path.as_bytes(), *entry))
            .peekable();
        let mut w = Writer::with_capacity(self.body.len());
        for b in MAGIC {
            w.u8(*b);
        }
        w.u8(VERSION);
        w.varint(count as u64);
        loop {
            // A path is never in both.
            let next = match (slots.peek(), added.peek()) {
                (Some(s), Some(a)) if s.0 < a.0 => slots.next(),
                (Some(_), None) => slots.next(),
                _ => added.next(),
            };
            let Some((path, entry)) = next else {
                break;
            };
            let s = &entry.state;
            let racy = entry.racy || trusted_before.is_some_and(|t| s.modified_ns > t);
            w.bytes(path);
            w.u8(s.kind as u8);
            w.varint(s.size);
            // `record` keeps only times whose seconds fit.
            for ns in [s.modified_ns, s.changed_ns] {
                let (secs, nanos) = split_ns(ns).unwrap_or_default();
                w.i64(secs);
                w.u32(nanos);
            }
            let (drive, number) = s.file_id;
            w.u64(drive);
            w.u64(number as u64);
            w.u64((number >> 64) as u64);
            w.bool(racy);
            w.id(&s.id);
        }
        w.into_bytes()
    }

    /// Reads a body `encode` wrote. Paths must be in byte order, each once.
    pub fn decode(body: &[u8]) -> Result<FileCache, DecodeError> {
        FileCache::decode_owned(body.to_vec())
    }

    fn decode_owned(body: Vec<u8>) -> Result<FileCache, DecodeError> {
        let rest = body
            .strip_prefix(MAGIC.as_slice())
            .ok_or(DecodeError::Invalid("not a file-state cache"))?;
        let mut r = Reader::new(rest);
        if r.u8()? != VERSION {
            return Err(DecodeError::Invalid("file-state cache version"));
        }
        // Each entry takes at least this many bytes, so a count past what is left is refused.
        let fits = r.remaining().checked_div(MIN_ENTRY).unwrap_or(0);
        let count = r.count(fits as u64, "file count")?;
        let mut slots = Vec::with_capacity(fits.min(usize::try_from(count).unwrap_or(0)));
        let mut last: Option<&str> = None;
        let too_big = || DecodeError::Invalid("file-state cache too big");
        for _ in 0..count {
            let path = r.string(MAX_PATH, "path")?;
            if last.is_some_and(|before| before >= path) {
                return Err(DecodeError::Invalid("paths out of order"));
            }
            last = Some(path);
            let end = body.len().checked_sub(r.remaining()).ok_or_else(too_big)?;
            let start = end.checked_sub(path.len()).ok_or_else(too_big)?;
            let at = (
                u32::try_from(start).map_err(|_| too_big())?,
                u32::try_from(end).map_err(|_| too_big())?,
            );
            let kind = EntryKind::from_u8(r.u8()?)
                .filter(|k| kind_of_entry(*k))
                .ok_or(DecodeError::Invalid("file kind"))?;
            let size = r.varint()?;
            let secs = r.i64()?;
            let nanos = r.u32()?;
            let modified_ns = join_ns(secs, nanos).ok_or(DecodeError::Invalid("nanoseconds"))?;
            let (secs, nanos) = (r.i64()?, r.u32()?);
            let changed_ns = join_ns(secs, nanos).ok_or(DecodeError::Invalid("nanoseconds"))?;
            let drive = r.u64()?;
            let (low, high) = (r.u64()?, r.u64()?);
            let racy = r.bool()?;
            let id = r.id()?;
            let state = FileState {
                kind,
                size,
                modified_ns,
                changed_ns,
                file_id: (drive, u128::from(high) << 64 | u128::from(low)),
                id,
            };
            slots.push(Slot {
                at,
                entry: Entry { state, racy },
                kept: false,
                gone: false,
            });
        }
        r.finish()?;
        Ok(FileCache {
            body,
            slots,
            ..FileCache::default()
        })
    }

    /// The cache in `repo`, or an empty one if it is missing, unreadable, or not sealed by the
    /// handle's key, or the handle has none.
    #[must_use]
    pub fn load(repo: &Repository) -> FileCache {
        FileCache::load_from(repo.cache_folder(), &repo.cache_dir(), repo.cache_key())
    }

    /// `load`, on a helper thread while `meanwhile` runs on this one. Without a thread, the cache
    /// is loaded here afterwards; a helper that fails gives an empty cache.
    pub(crate) fn load_during<T>(
        repo: &Repository,
        meanwhile: impl FnOnce() -> T,
    ) -> (T, FileCache) {
        let (folder, shown, key) = (repo.cache_folder(), repo.cache_dir(), repo.cache_key());
        std::thread::scope(|scope| {
            let helper = std::thread::Builder::new()
                .spawn_scoped(scope, move || FileCache::load_from(folder, &shown, key));
            let done = meanwhile();
            let cache = match helper {
                Ok(helper) => helper.join().unwrap_or_default(),
                Err(_) => FileCache::load(repo),
            };
            (done, cache)
        })
    }

    /// The cache in the folder `cache/`, shown as `cache_dir`, sealed with `key`.
    fn load_from(folder: &Dir, cache_dir: &Path, key: Option<&[u8; keyed::KEY_LEN]>) -> FileCache {
        let Some(key) = key else {
            return FileCache::default();
        };
        let shown = cache_dir.join(FILE);
        let read = safe_fs::read_capped_in(folder, OsStr::new(FILE), &shown, MAX_BYTES);
        let Ok(mut bytes) = read else {
            return FileCache::default();
        };
        let Some(body_len) = keyed::open_sealed(key, &bytes).map(<[u8]>::len) else {
            return FileCache::default();
        };
        bytes.truncate(body_len);
        FileCache::decode_owned(bytes)
            .map(|cache| FileCache {
                loaded: true,
                ..cache
            })
            .unwrap_or_default()
    }

    fn path_of(&self, slot: &Slot) -> &[u8] {
        let (start, end) = slot.at;
        self.body
            .get(start as usize..end as usize)
            .unwrap_or_default()
    }

    /// The slot of `path`, if the body holds it.
    fn find(&self, path: &str) -> Option<usize> {
        let next = self.next.get();
        let at = if self
            .slots
            .get(next)
            .is_some_and(|s| self.path_of(s) == path.as_bytes())
        {
            Ok(next)
        } else {
            #[cfg(test)]
            self.searches.set(self.searches.get().saturating_add(1));
            self.slots
                .binary_search_by(|s| self.path_of(s).cmp(path.as_bytes()))
        };
        // After a path the cache doesn't hold, the next one asked for is likely where it would go.
        self.next
            .set(at.map_or_else(|gap| gap, |i| i.saturating_add(1)));
        at.ok()
    }

    /// What the cache knows of `path`, if the file there is still the same file, with that kind,
    /// size and time, and didn't change too close to when the cache was written. A file numbered
    /// 0 is never trusted: some network and FUSE drives give every file that number.
    #[must_use]
    pub fn matches(&self, path: &str, meta: &EntryMeta) -> Option<&FileState> {
        self.trusted(path, meta).map(|(_, s)| s)
    }

    /// `matches`, with the slot the entry came from, if not from what this scan recorded.
    fn trusted(&self, path: &str, meta: &EntryMeta) -> Option<(Option<usize>, &FileState)> {
        let slot = self
            .find(path)
            .filter(|&i| self.slots.get(i).is_some_and(|s| !s.gone));
        let entry = slot
            .and_then(|i| self.slots.get(i))
            .map(|s| &s.entry)
            .or_else(|| self.added.get(path))
            .filter(|e| !e.racy)?;
        let s = &entry.state;
        (Some(s.kind) == kind_of(meta)
            && s.size == meta.size
            && s.modified_ns == meta.modified_ns
            && s.changed_ns == meta.changed_ns
            && s.file_id == meta.file_id
            && meta.file_id.1 != 0)
            .then_some((slot, s))
    }

    /// `matches`, and keeps the entry for the next write.
    pub fn vouch(&mut self, path: &str, meta: &EntryMeta) -> Option<FileState> {
        self.vouch_if(path, meta, |_| true)
    }

    /// `vouch`, if `keep` takes what the cache knows.
    pub fn vouch_if(
        &mut self,
        path: &str,
        meta: &EntryMeta,
        keep: impl FnOnce(&FileState) -> bool,
    ) -> Option<FileState> {
        let (slot, state) = self
            .trusted(path, meta)
            .map(|(slot, s)| (slot, *s))
            .filter(|(_, s)| keep(s))?;
        if let Some(slot) = slot.and_then(|i| self.slots.get_mut(i)) {
            slot.kept = true;
        }
        Some(state)
    }

    /// Records that the file at `path`, with these details, holds the content `id`. Folders and
    /// anything that isn't a file or a link aren't kept, nor what the file couldn't read back.
    pub fn record(&mut self, path: &str, meta: &EntryMeta, id: Id) {
        let fits = path.len() <= MAX_PATH
            && split_ns(meta.modified_ns).is_some()
            && split_ns(meta.changed_ns).is_some();
        if let Some(slot) = self.find(path).and_then(|i| self.slots.get_mut(i)) {
            slot.gone = true;
            slot.kept = false;
        }
        let Some(kind) = kind_of(meta).filter(|_| fits) else {
            self.added.remove(path);
            return;
        };
        let state = FileState {
            kind,
            size: meta.size,
            modified_ns: meta.modified_ns,
            changed_ns: meta.changed_ns,
            file_id: meta.file_id,
            id,
        };
        self.added
            .insert(path.to_owned(), Entry { state, racy: false });
    }

    /// Writes what this scan vouched for and recorded, sealed, replacing the old cache whole;
    /// nothing when the handle has no key, or when that is what the file already holds. `now_ns`
    /// is the drive's time at the start of the scan (`drive_now`): a file whose modified time
    /// isn't at least 2 s older is kept as racy, never matched, since it may change again within
    /// one tick of the drive's clock.
    pub fn store(self, repo: &Repository, now_ns: i128) -> Result<(), RepoError> {
        let Some(key) = repo.cache_key() else {
            return Ok(());
        };
        let trusted_before = now_ns.saturating_sub(RACY_NS);
        // Every entry kept, none added and none turning racy: the same bytes, unencoded.
        let same = self.loaded
            && self.added.is_empty()
            && self
                .slots
                .iter()
                .all(|s| s.kept && (s.entry.racy || s.entry.state.modified_ns <= trusted_before));
        if same {
            return Ok(());
        }
        let body = self.body_of(false, Some(trusted_before));
        if self.loaded && body == self.body {
            return Ok(());
        }
        let sealed = keyed::seal(key, &body);
        if sealed.len() as u64 > MAX_BYTES {
            return Err(RepoError::Io(io::Error::other(
                "the file-state cache would be bigger than 256 MiB",
            )));
        }
        fsio::write_cache(repo.tmp_folder(), repo.cache_folder(), FILE, &sealed)?;
        Ok(())
    }
}

/// The drive's own "now": the modified time of a file written in `.bolt/tmp`, since a network or
/// FAT drive's clock and resolution may differ from this computer's.
pub fn drive_now(repo: &Repository) -> Result<i128, RepoError> {
    let tmp = repo.tmp_folder();
    let (mut file, name) = fsio::create_temp(tmp)?;
    let written = file.write_all(b"now").and_then(|()| file.sync_all());
    drop(file);
    // Read once closed: some network drives set the time only then.
    let meta = written.and_then(|()| tmp.metadata(&name));
    let removed = fsio::remove_file(tmp, &name);
    let now = meta?.modified_ns;
    removed?;
    Ok(now)
}

/// The kind a file on disk is kept as, if it is one the cache keeps.
fn kind_of(meta: &EntryMeta) -> Option<EntryKind> {
    match meta.kind {
        EntryType::File if meta.executable == Some(true) => Some(EntryKind::Executable),
        EntryType::File => Some(EntryKind::File),
        EntryType::Symlink => Some(EntryKind::Symlink),
        EntryType::Dir | EntryType::Other => None,
    }
}

fn kind_of_entry(kind: EntryKind) -> bool {
    matches!(
        kind,
        EntryKind::File | EntryKind::Executable | EntryKind::Symlink
    )
}

/// Whole seconds (rounded down) and nanoseconds, if the seconds fit 64 bits.
fn split_ns(ns: i128) -> Option<(i64, u32)> {
    let secs = i64::try_from(ns.checked_div_euclid(SECOND)?).ok()?;
    let nanos = u32::try_from(ns.checked_rem_euclid(SECOND)?).ok()?;
    Some((secs, nanos))
}

fn join_ns(secs: i64, nanos: u32) -> Option<i128> {
    let nanos = i128::from(nanos);
    if nanos >= SECOND {
        return None;
    }
    i128::from(secs).checked_mul(SECOND)?.checked_add(nanos)
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing,
        clippy::arithmetic_side_effects
    )]
    use super::*;
    use crate::save::Person;

    const SECOND: i128 = 1_000_000_000;
    const KEY: [u8; 32] = [7; 32];

    fn project() -> (tempfile::TempDir, Repository) {
        let dir = tempfile::tempdir().unwrap();
        let who = Person {
            name: "Ada".into(),
            email: "ada@example.com".into(),
            time_ms: 0,
            tz_offset_min: 0,
        };
        let mut repo = Repository::init(dir.path(), &who).unwrap();
        repo.set_cache_key(KEY);
        (dir, repo)
    }

    fn reopen(dir: &tempfile::TempDir, key: [u8; 32]) -> Repository {
        let mut repo = Repository::open(dir.path()).unwrap();
        repo.set_cache_key(key);
        repo
    }

    fn file(size: u64, modified_ns: i128) -> EntryMeta {
        EntryMeta {
            kind: EntryType::File,
            size,
            modified_ns,
            changed_ns: modified_ns,
            executable: None,
            file_id: (1, 2),
        }
    }

    /// A time long past, well clear of any "now" the tests use.
    const THEN: i128 = 1_700_000_000 * SECOND + 123_456_789;

    fn stored(dir: &tempfile::TempDir) -> std::path::PathBuf {
        dir.path().join(".bolt").join("cache").join("files")
    }

    #[test]
    fn a_matching_entry_is_trusted() {
        let (dir, repo) = project();
        let mut cache = FileCache::load(&repo);
        assert!(cache.matches("a.txt", &file(3, THEN)).is_none());
        cache.record("a.txt", &file(3, THEN), Id([1; 32]));
        cache.record("b/c.txt", &file(0, -THEN), Id([2; 32]));
        cache.store(&repo, THEN + 60 * SECOND).unwrap();

        let back = FileCache::load(&reopen(&dir, KEY));
        let state = back.matches("a.txt", &file(3, THEN)).unwrap();
        assert_eq!(
            *state,
            FileState {
                kind: EntryKind::File,
                size: 3,
                modified_ns: THEN,
                changed_ns: THEN,
                file_id: (1, 2),
                id: Id([1; 32])
            }
        );
        // A time before 1970 reads back too.
        assert_eq!(
            back.matches("b/c.txt", &file(0, -THEN)).unwrap().id,
            Id([2; 32])
        );
        assert!(back.matches("other.txt", &file(3, THEN)).is_none());
        assert_eq!(
            back.encode(),
            FileCache::decode(&back.encode()).unwrap().encode()
        );
    }

    #[test]
    fn a_file_changed_within_two_seconds_of_now_is_read_again() {
        let (dir, repo) = project();
        std::fs::write(dir.path().join("new.txt"), b"hi").unwrap();
        let top = bolt_platform::dir::Dir::open_top(dir.path()).unwrap();
        let fresh = top.metadata(std::ffi::OsStr::new("new.txt")).unwrap();
        // The drive's clock, read after the file was written, and nothing left in `tmp/`.
        let now = drive_now(&repo).unwrap();
        assert!(now >= fresh.modified_ns, "{now} {}", fresh.modified_ns);
        assert!(std::fs::read_dir(repo.tmp_dir()).unwrap().next().is_none());

        let mut cache = FileCache::load(&repo);
        cache.record("new.txt", &fresh, Id([1; 32]));
        cache.record("second.txt", &file(1, now - SECOND), Id([2; 32]));
        cache.record("later.txt", &file(1, now + 60 * SECOND), Id([3; 32]));
        cache.record("old.txt", &file(1, now - 2 * SECOND), Id([4; 32]));
        cache.store(&repo, now).unwrap();

        let back = FileCache::load(&reopen(&dir, KEY));
        assert!(back.matches("new.txt", &fresh).is_none());
        assert!(back.matches("second.txt", &file(1, now - SECOND)).is_none());
        assert!(
            back.matches("later.txt", &file(1, now + 60 * SECOND))
                .is_none()
        );
        assert!(
            back.matches("old.txt", &file(1, now - 2 * SECOND))
                .is_some()
        );
        // Written again later, a racy entry nobody read again isn't trusted.
        let mut back = back;
        assert!(back.vouch("new.txt", &fresh).is_none());
        back.vouch("old.txt", &file(1, now - 2 * SECOND)).unwrap();
        back.store(&repo, now + 60 * SECOND).unwrap();
        let again = FileCache::load(&reopen(&dir, KEY));
        assert!(again.matches("new.txt", &fresh).is_none());
        assert!(
            again
                .matches("old.txt", &file(1, now - 2 * SECOND))
                .is_some()
        );
    }

    #[test]
    fn a_cache_sealed_by_another_key_is_ignored() {
        let (dir, repo) = project();
        let mut cache = FileCache::load(&repo);
        cache.record("a.txt", &file(3, THEN), Id([1; 32]));
        cache.store(&repo, THEN + 60 * SECOND).unwrap();
        assert!(
            FileCache::load(&reopen(&dir, KEY))
                .matches("a.txt", &file(3, THEN))
                .is_some()
        );
        let other = FileCache::load(&reopen(&dir, [8; 32]));
        assert!(other.matches("a.txt", &file(3, THEN)).is_none());
        assert!(other.slots.is_empty());
    }

    #[test]
    fn a_truncated_or_edited_cache_is_ignored() {
        let (dir, repo) = project();
        let mut cache = FileCache::load(&repo);
        cache.record("a.txt", &file(3, THEN), Id([1; 32]));
        cache.store(&repo, THEN + 60 * SECOND).unwrap();
        let sealed = std::fs::read(stored(&dir)).unwrap();
        let tries = [
            sealed[..sealed.len() - 1].to_vec(),
            sealed[..10].to_vec(),
            Vec::new(),
            {
                let mut edited = sealed.clone();
                // The size byte: 3 becomes 4.
                let at = sealed.len() - 32 - 32 - 1 - 24 - 12 - 4 - 8 - 1;
                assert_eq!(edited[at], 3);
                edited[at] = 4;
                edited
            },
        ];
        for bytes in tries {
            std::fs::write(stored(&dir), &bytes).unwrap();
            let back = FileCache::load(&reopen(&dir, KEY));
            assert!(back.slots.is_empty(), "{}", bytes.len());
        }
        // A folder where the file should be is no cache either, and no error.
        std::fs::remove_file(stored(&dir)).unwrap();
        std::fs::create_dir(stored(&dir)).unwrap();
        assert!(FileCache::load(&reopen(&dir, KEY)).slots.is_empty());
    }

    #[test]
    fn size_or_time_or_kind_or_identity_changes_miss() {
        let (_dir, repo) = project();
        let mut cache = FileCache::load(&repo);
        cache.record("a", &file(3, THEN), Id([1; 32]));
        assert!(cache.matches("a", &file(3, THEN)).is_some());
        assert!(cache.matches("a", &file(4, THEN)).is_none());
        assert!(cache.matches("a", &file(3, THEN + 1)).is_none());
        // Another file moved onto the path, on this drive or another, is not the one recorded.
        for file_id in [(1, 3), (2, 2), (1, 2 | 1 << 64)] {
            assert!(
                cache
                    .matches(
                        "a",
                        &EntryMeta {
                            file_id,
                            ..file(3, THEN)
                        }
                    )
                    .is_none()
            );
        }
        // The same file written over in place, its size and modified time kept, was changed.
        let rewritten = EntryMeta {
            changed_ns: THEN + 1,
            ..file(3, THEN)
        };
        assert!(cache.matches("a", &rewritten).is_none());
        // A drive that numbers every file 0 says nothing about which file it is.
        let unnumbered = EntryMeta {
            file_id: (1, 0),
            ..file(3, THEN)
        };
        cache.record("n", &unnumbered, Id([5; 32]));
        assert!(cache.matches("n", &unnumbered).is_none());
        let executable = EntryMeta {
            executable: Some(true),
            ..file(3, THEN)
        };
        assert!(cache.matches("a", &executable).is_none());
        let link = EntryMeta {
            kind: EntryType::Symlink,
            ..file(3, THEN)
        };
        assert!(cache.matches("a", &link).is_none());
        cache.record("x", &executable, Id([2; 32]));
        assert_eq!(
            cache.matches("x", &executable).unwrap().kind,
            EntryKind::Executable
        );
        assert!(cache.matches("x", &file(3, THEN)).is_none());
        // Folders and anything else aren't kept.
        let folder = EntryMeta {
            kind: EntryType::Dir,
            ..file(0, THEN)
        };
        cache.record("d", &folder, Id([3; 32]));
        assert!(cache.matches("d", &folder).is_none());
        // Nor a path the file couldn't read back, which would spoil the whole cache.
        let long = "a".repeat(MAX_PATH + 1);
        cache.record(&long, &file(3, THEN), Id([4; 32]));
        assert!(cache.matches(&long, &file(3, THEN)).is_none());
        assert!(FileCache::decode(&cache.encode()).is_ok());
    }

    #[test]
    fn a_cache_that_breaks_its_rules_is_refused() {
        let mut cache = FileCache::default();
        cache.record("a", &file(3, THEN), Id([1; 32]));
        cache.record("b", &file(5, THEN), Id([2; 32]));
        let body = cache.encode();
        assert_eq!(FileCache::decode(&body).unwrap().encode(), body);
        for len in 0..body.len() {
            assert!(FileCache::decode(&body[..len]).is_err(), "{len}");
        }
        let mut longer = body.clone();
        longer.push(0);
        assert!(FileCache::decode(&longer).is_err());
        // Magic, version, count, then "a": its length and name.
        let a_kind = 8 + 1 + 1 + 2;
        let entry = 1 + 1 + 12 + 12 + 24 + 1 + 32;
        let edit = |at: usize, v: u8| {
            let mut b = body.clone();
            b[at] = v;
            FileCache::decode(&b)
        };
        assert!(edit(8, 1).is_err(), "a version 1 cache");
        assert!(
            edit(8, 2).is_err(),
            "a version 2 cache, without change times"
        );
        assert!(edit(8, 4).is_err(), "a later version");
        assert!(edit(9, 0x7f).is_err(), "count past the bytes");
        assert!(edit(a_kind, 4).is_err(), "a folder");
        assert!(edit(a_kind, 0).is_err(), "no kind");
        assert!(
            edit(a_kind + 2 + 8 + 3, 0xff).is_err(),
            "nanoseconds past a second"
        );
        assert!(
            edit(a_kind + 2 + 12 + 8 + 3, 0xff).is_err(),
            "change-time nanoseconds past a second"
        );
        assert!(edit(a_kind + 2 + 24 + 24, 2).is_err(), "racy not a bool");
        assert!(edit(a_kind + entry + 1, b'a').is_err(), "a name twice");
        // A size that isn't written in the fewest bytes.
        let mut long_size = body[..a_kind + 1].to_vec();
        long_size.extend_from_slice(&[0x83, 0x00]);
        long_size.extend_from_slice(&body[a_kind + 2..]);
        assert!(FileCache::decode(&long_size).is_err());
    }

    /// The stored file's ID, which a write through `tmp/` and a rename changes, and its bytes.
    fn on_disk(dir: &tempfile::TempDir) -> ((u64, u128), Vec<u8>) {
        let cache = bolt_platform::dir::Dir::open_top(&dir.path().join(".bolt").join("cache"));
        let meta = cache.unwrap().metadata(OsStr::new(FILE)).unwrap();
        (meta.file_id, std::fs::read(stored(dir)).unwrap())
    }

    /// A cache holding `a` and `b`, both long unchanged.
    fn two_files() -> (tempfile::TempDir, Repository) {
        let (dir, repo) = project();
        let mut cache = FileCache::load(&repo);
        cache.record("a", &file(1, THEN), Id([1; 32]));
        cache.record("b", &file(2, THEN), Id([2; 32]));
        cache.store(&repo, THEN + 60 * SECOND).unwrap();
        (dir, repo)
    }

    #[test]
    fn an_unchanged_cache_is_not_written_again() {
        let (dir, repo) = two_files();
        let before = on_disk(&dir);
        let mut cache = FileCache::load(&repo);
        assert!(cache.vouch("a", &file(1, THEN)).is_some());
        assert!(cache.vouch("b", &file(2, THEN)).is_some());
        cache.store(&repo, THEN + 120 * SECOND).unwrap();
        assert_eq!(on_disk(&dir), before);
    }

    #[test]
    fn a_cache_is_written_when_an_entry_is_added_dropped_or_changed() {
        let later = THEN + 120 * SECOND;
        // Added.
        let (dir, repo) = two_files();
        let before = on_disk(&dir);
        let mut cache = FileCache::load(&repo);
        cache.vouch("a", &file(1, THEN)).unwrap();
        cache.vouch("b", &file(2, THEN)).unwrap();
        cache.record("c", &file(3, THEN), Id([3; 32]));
        cache.store(&repo, later).unwrap();
        assert_ne!(on_disk(&dir), before);
        let back = FileCache::load(&repo);
        for (path, size) in [("a", 1), ("b", 2), ("c", 3)] {
            assert!(back.matches(path, &file(size, THEN)).is_some(), "{path}");
        }
        // Dropped: `b` wasn't met.
        let (dir, repo) = two_files();
        let before = on_disk(&dir);
        let mut cache = FileCache::load(&repo);
        cache.vouch("a", &file(1, THEN)).unwrap();
        cache.store(&repo, later).unwrap();
        assert_ne!(on_disk(&dir), before);
        let back = FileCache::load(&repo);
        assert!(back.matches("a", &file(1, THEN)).is_some());
        assert!(back.matches("b", &file(2, THEN)).is_none());
        // Changed: `b` was read again with other details.
        let (dir, repo) = two_files();
        let before = on_disk(&dir);
        let mut cache = FileCache::load(&repo);
        cache.vouch("a", &file(1, THEN)).unwrap();
        assert!(cache.vouch("b", &file(5, THEN)).is_none());
        cache.record("b", &file(5, THEN), Id([5; 32]));
        assert_eq!(cache.matches("b", &file(5, THEN)).unwrap().id, Id([5; 32]));
        cache.store(&repo, later).unwrap();
        assert_ne!(on_disk(&dir), before);
        let back = FileCache::load(&repo);
        assert!(back.matches("b", &file(2, THEN)).is_none());
        assert_eq!(back.matches("b", &file(5, THEN)).unwrap().id, Id([5; 32]));
    }

    #[test]
    fn an_entry_that_turns_racy_is_written() {
        let (dir, repo) = two_files();
        let before = on_disk(&dir);
        let mut cache = FileCache::load(&repo);
        cache.vouch("a", &file(1, THEN)).unwrap();
        cache.vouch("b", &file(2, THEN)).unwrap();
        // A "now" less than 2 s after the files' time: a drive whose clock went back.
        cache.store(&repo, THEN + SECOND).unwrap();
        assert_ne!(on_disk(&dir), before);
        assert!(
            FileCache::load(&repo)
                .matches("a", &file(1, THEN))
                .is_none()
        );
    }

    #[test]
    fn a_racy_entry_read_again_later_is_written() {
        let (dir, repo) = project();
        let mut cache = FileCache::load(&repo);
        cache.record("a", &file(1, THEN), Id([1; 32]));
        cache.store(&repo, THEN + SECOND).unwrap();
        let before = on_disk(&dir);
        let mut cache = FileCache::load(&repo);
        assert!(cache.vouch("a", &file(1, THEN)).is_none());
        cache.record("a", &file(1, THEN), Id([1; 32]));
        cache.store(&repo, THEN + 60 * SECOND).unwrap();
        assert_ne!(on_disk(&dir), before);
        assert!(
            FileCache::load(&repo)
                .matches("a", &file(1, THEN))
                .is_some()
        );
    }

    #[test]
    fn a_torn_file_cache_is_ignored_and_rebuilt() {
        let (dir, repo) = two_files();
        let first = std::fs::read(stored(&dir)).unwrap();
        let mut other = FileCache::default();
        other.record("a", &file(7, THEN), Id([7; 32]));
        other.record("b", &file(8, THEN), Id([8; 32]));
        other.store(&repo, THEN + 60 * SECOND).unwrap();
        let second = std::fs::read(stored(&dir)).unwrap();
        assert_eq!(first.len(), second.len());
        let half = first.len() / 2;
        let mixed = [&first[..half], &second[half..]].concat();
        for torn in [first[..half].to_vec(), vec![0; first.len()], mixed] {
            std::fs::write(stored(&dir), &torn).unwrap();
            let mut cache = FileCache::load(&repo);
            assert!(cache.matches("a", &file(1, THEN)).is_none());
            assert!(cache.matches("a", &file(7, THEN)).is_none());
            cache.record("a", &file(1, THEN), Id([1; 32]));
            cache.store(&repo, THEN + 60 * SECOND).unwrap();
            let back = FileCache::load(&repo);
            assert!(back.matches("a", &file(1, THEN)).is_some());
            assert!(back.matches("b", &file(2, THEN)).is_none());
        }
    }

    /// A cache read back from a body holding `paths`, each with the ID `[1; 32]`.
    fn read_back<'a>(paths: impl IntoIterator<Item = &'a str>) -> FileCache {
        let mut cache = FileCache::default();
        for path in paths {
            cache.record(path, &file(1, THEN), Id([1; 32]));
        }
        FileCache::decode(&cache.encode()).unwrap()
    }

    /// The slot a plain search finds.
    fn searched(cache: &FileCache, path: &str) -> Option<usize> {
        cache
            .slots
            .binary_search_by(|s| cache.path_of(s).cmp(path.as_bytes()))
            .ok()
    }

    #[test]
    fn a_slot_is_found_where_the_last_one_ended() {
        let paths: Vec<String> = (0..1000)
            .map(|i| format!("d{:02}/f{i:04}.txt", i / 100))
            .collect();
        let cache = read_back(paths.iter().map(String::as_str));
        for path in &paths {
            assert!(cache.matches(path, &file(1, THEN)).is_some(), "{path}");
        }
        assert!(cache.searches.get() <= 1, "{}", cache.searches.get());

        let mut asked: Vec<&str> = paths.iter().map(String::as_str).rev().collect();
        asked.extend(["", "a", "d00", "d05/f0500.txt!", "zz", "d09/f0999.txt"]);
        let mut seed = 7u64;
        for _ in 0..1000 {
            seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
            asked.push(&paths[(seed >> 33) as usize % paths.len()]);
        }
        for path in asked {
            assert_eq!(cache.find(path), searched(&cache, path), "{path}");
            let known = searched(&cache, path).is_some();
            assert_eq!(cache.matches(path, &file(1, THEN)).is_some(), known);
        }
    }

    #[derive(Debug, Clone)]
    enum Ask {
        Matches(String),
        Vouch(String),
        Record(String),
    }

    fn ask() -> impl Strategy<Value = Ask> {
        let path = "[ab/]{0,4}";
        prop_oneof![
            path.prop_map(Ask::Matches),
            path.prop_map(Ask::Vouch),
            path.prop_map(Ask::Record),
        ]
    }

    use proptest::prelude::*;

    proptest! {
        #[test]
        fn the_hint_never_changes_an_answer(
            paths in proptest::collection::btree_set("[ab/]{0,4}", 0..12),
            asks in proptest::collection::vec(ask(), 0..40),
        ) {
            let mut hinted = read_back(paths.iter().map(String::as_str));
            // Pointed past the end before every question, it always searches.
            let mut plain = hinted.clone();
            let meta = file(1, THEN);
            for ask in asks {
                plain.next.set(usize::MAX);
                match ask {
                    Ask::Matches(p) => {
                        prop_assert_eq!(hinted.find(&p), searched(&plain, &p));
                        prop_assert_eq!(
                            hinted.matches(&p, &meta).copied(),
                            plain.matches(&p, &meta).copied()
                        );
                    }
                    Ask::Vouch(p) => {
                        prop_assert_eq!(hinted.vouch(&p, &meta), plain.vouch(&p, &meta));
                    }
                    Ask::Record(p) => {
                        hinted.record(&p, &meta, Id([2; 32]));
                        plain.record(&p, &meta, Id([2; 32]));
                    }
                }
            }
            prop_assert_eq!(hinted.body_of(false, None), plain.body_of(false, None));
            prop_assert_eq!(hinted.encode(), plain.encode());
        }
    }
}
