//! The file-state cache, `.bolt/cache/files` (storage-format.md section 10): each file's kind,
//! size, modified time, file ID and content ID, so that a file whose details still match needn't
//! be read. Only ever a hint: one that is missing, sealed by another key, edited or unreadable is
//! empty.
#![deny(clippy::indexing_slicing, clippy::arithmetic_side_effects)]

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::io::{self, Write};

use bolt_platform::dir::{EntryMeta, EntryType};

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

/// The file-state cache, by project path.
#[derive(Debug, Default, Clone)]
pub struct FileCache {
    files: BTreeMap<String, Entry>,
}

impl FileCache {
    /// The cache's body, without its seal.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        for b in MAGIC {
            w.u8(*b);
        }
        w.u8(VERSION);
        w.varint(self.files.len() as u64);
        for (path, entry) in &self.files {
            let s = &entry.state;
            w.string(path);
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
            w.bool(entry.racy);
            w.id(&s.id);
        }
        w.into_bytes()
    }

    /// Reads a body `encode` wrote. Paths must be in byte order, each once.
    pub fn decode(body: &[u8]) -> Result<FileCache, DecodeError> {
        let rest = body
            .strip_prefix(MAGIC.as_slice())
            .ok_or(DecodeError::Invalid("not a file-state cache"))?;
        let mut r = Reader::new(rest);
        if r.u8()? != VERSION {
            return Err(DecodeError::Invalid("file-state cache version"));
        }
        // Each entry takes at least this many bytes, so a count past what is left is refused.
        let fits = r.remaining().checked_div(MIN_ENTRY).unwrap_or(0) as u64;
        let count = r.count(fits, "file count")?;
        let mut files = BTreeMap::new();
        let mut last: Option<&str> = None;
        for _ in 0..count {
            let path = r.string(MAX_PATH, "path")?;
            if last.is_some_and(|before| before >= path) {
                return Err(DecodeError::Invalid("paths out of order"));
            }
            last = Some(path);
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
            files.insert(path.to_owned(), Entry { state, racy });
        }
        r.finish()?;
        Ok(FileCache { files })
    }

    /// The cache in `repo`, or an empty one if it is missing, unreadable, or not sealed by this
    /// handle's key.
    #[must_use]
    pub fn load(repo: &Repository) -> FileCache {
        let shown = repo.cache_dir().join(FILE);
        let read =
            safe_fs::read_capped_in(repo.cache_folder(), OsStr::new(FILE), &shown, MAX_BYTES);
        read.ok()
            .and_then(|bytes| FileCache::decode(keyed::open_sealed(repo.cache_key(), &bytes)?).ok())
            .unwrap_or_default()
    }

    /// What the cache knows of `path`, if the file there is still the same file, with that kind,
    /// size and time, and didn't change too close to when the cache was written. A file numbered
    /// 0 is never trusted: some network and FUSE drives give every file that number.
    #[must_use]
    pub fn matches(&self, path: &str, meta: &EntryMeta) -> Option<&FileState> {
        let entry = self.files.get(path).filter(|e| !e.racy)?;
        let s = &entry.state;
        (Some(s.kind) == kind_of(meta)
            && s.size == meta.size
            && s.modified_ns == meta.modified_ns
            && s.changed_ns == meta.changed_ns
            && s.file_id == meta.file_id
            && meta.file_id.1 != 0)
            .then_some(s)
    }

    /// Records that the file at `path`, with these details, holds the content `id`. Folders and
    /// anything that isn't a file or a link aren't kept, nor what the file couldn't read back.
    pub fn record(&mut self, path: &str, meta: &EntryMeta, id: Id) {
        let fits = path.len() <= MAX_PATH
            && split_ns(meta.modified_ns).is_some()
            && split_ns(meta.changed_ns).is_some();
        let Some(kind) = kind_of(meta).filter(|_| fits) else {
            self.files.remove(path);
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
        self.files
            .insert(path.to_owned(), Entry { state, racy: false });
    }

    /// Writes the cache, sealed, replacing the old one whole. `now_ns` is the drive's time at the
    /// start of the scan (`drive_now`): a file whose modified time isn't at least 2 s older is kept
    /// as racy, never matched, since it may change again within one tick of the drive's clock.
    pub fn store(mut self, repo: &Repository, now_ns: i128) -> Result<(), RepoError> {
        let trusted_before = now_ns.saturating_sub(RACY_NS);
        for entry in self.files.values_mut() {
            entry.racy |= entry.state.modified_ns > trusted_before;
        }
        let sealed = keyed::seal(repo.cache_key(), &self.encode());
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
        // A racy entry stays racy when the cache is written again later.
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
        assert!(other.files.is_empty());
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
            assert!(back.files.is_empty(), "{}", bytes.len());
        }
        // A folder where the file should be is no cache either, and no error.
        std::fs::remove_file(stored(&dir)).unwrap();
        std::fs::create_dir(stored(&dir)).unwrap();
        assert!(FileCache::load(&reopen(&dir, KEY)).files.is_empty());
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
}
