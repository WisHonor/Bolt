//! Git's packs and their version 2 indexes, entry headers and deltas (decision 031). An index's
//! name and offset tables are read into memory a buffer at a time, once its object count is
//! within Bolt's limit and its length fits that count; the rest of a pack is read only when an
//! object in it is.
#![deny(clippy::indexing_slicing, clippy::arithmetic_side_effects)]

use std::cell::Cell;
use std::collections::BTreeSet;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, Read};
use std::path::{Path, PathBuf};

use super::id::Hasher;
use super::odb::{Input, MAX_IN_MEMORY_OBJECT};
use super::{GitError, GitId, HashKind, ObjectType, Refusal, broken, read_error, real_dir, shrank};
use crate::safe_fs;

/// Entries `objects/pack` may hold, of any kind: room for tens of thousands of packs.
const MAX_PACK_FOLDER_ENTRIES: u64 = 100_000;
/// Packs Bolt reads, since an object may be looked for in each: Git merges them once there are
/// 50 (`gc.autoPackLimit`).
const MAX_PACKS: usize = 4096;
/// Objects Bolt reads from all of a repository's packs, whose indexes it keeps in memory (32 to
/// 44 bytes an object): git/git has about 0.4 million, and Linux about 10 million.
pub(super) const MAX_GIT_OBJECTS: u64 = 1 << 24;
/// Index entries read at a time.
const ENTRIES_AT_A_TIME: usize = 2048;
/// A pack's header: `PACK`, its version and its object count.
const PACK_HEADER_LEN: u64 = 12;
/// An index's header (`\377tOc` and its version), then its fan-out of 256 counts.
const INDEX_HEAD_LEN: u64 = 8 + 1024;
/// The most an entry's header takes: ten bytes of type and size, then its base's ID or distance.
const MAX_ENTRY_HEADER: usize = 10 + 32;

/// The packs of `objects/pack`, as listed and as read.
pub(super) struct PackFolder {
    /// The names listed, `pack-<checksum>`, to tell later whether Git has changed the folder.
    pub(super) listed: Vec<String>,
    /// The packs read, each checked against its index, that list objects. One Git removed after
    /// the listing isn't here, nor is one of no objects, where nothing can be found.
    pub(super) packs: Vec<Pack>,
}

/// A pack, checked against its index when the packs were listed.
pub(super) struct Pack {
    pub(super) path: PathBuf,
    /// The `.pack`'s length when it was checked.
    len: u64,
    hash: HashKind,
    /// The names of its objects, `hash.len()` bytes each, in order.
    names: Vec<u8>,
    /// Each object's offset, in the order of `names`.
    offsets: Vec<u64>,
    /// Places in `offsets`, in the order the entries come in the pack.
    in_order: Vec<u32>,
    /// One bit an entry, in that order: whether it has been inflated yet.
    inflated: Vec<Cell<u64>>,
}

/// What a pack entry holds.
#[derive(Clone, Copy)]
pub(super) enum EntryKind {
    Whole(ObjectType),
    /// A delta on the entry at this offset.
    OfsDelta(u64),
    /// A delta on the object with this ID, which must be in the same pack.
    RefDelta(GitId),
}

/// A pack entry's header.
pub(super) struct Entry {
    pub(super) kind: EntryKind,
    /// What its compressed data inflates to: the object, or the delta.
    pub(super) size: u64,
    /// Where its compressed data starts.
    pub(super) data: u64,
}

/// The packs in `folder` (`objects/pack`), each index read and each pack checked against it.
pub(super) fn load(folder: &Path, hash: HashKind) -> Result<PackFolder, GitError> {
    load_at_most(folder, hash, MAX_GIT_OBJECTS, MAX_PACKS)
}

/// `load`, refusing more than `max_packs` packs before any is read, and indexes that list more
/// than `max_objects` in all.
fn load_at_most(
    folder: &Path,
    hash: HashKind,
    max_objects: u64,
    max_packs: usize,
) -> Result<PackFolder, GitError> {
    let listed = list(folder)?;
    if listed.len() > max_packs {
        return Err(GitError::TooManyPacks {
            what: format!(
                "{} holds {} packs, more than the {max_packs} Bolt reads, since it may look for \
                 each object in every one",
                folder.display(),
                listed.len()
            ),
        });
    }
    let mut packs: Vec<Pack> = Vec::new();
    let mut objects = 0u64;
    for name in &listed {
        if let Some(pack) = Pack::load(folder, name, hash, objects, max_objects)?
            && !pack.offsets.is_empty()
        {
            objects = objects.saturating_add(pack.offsets.len() as u64);
            packs.push(pack);
        }
    }
    Ok(PackFolder { listed, packs })
}

/// The names of the packs in `folder`, sorted: every `pack-*.idx` that has its `.pack`. A file
/// without its partner is skipped, as Git's own reader skips it.
pub(super) fn list(folder: &Path) -> Result<Vec<String>, GitError> {
    list_at_most(folder, MAX_PACK_FOLDER_ENTRIES)
}

fn list_at_most(folder: &Path, max_entries: u64) -> Result<Vec<String>, GitError> {
    if !real_dir(folder)? {
        return Ok(Vec::new());
    }
    let (mut indexes, mut packs) = (BTreeSet::new(), BTreeSet::new());
    let mut entries = 0u64;
    for entry in fs::read_dir(folder).map_err(|e| read_error(folder, e))? {
        let entry = entry.map_err(|e| read_error(folder, e))?;
        entries = entries.saturating_add(1);
        if entries > max_entries {
            return Err(broken(format!(
                "{} holds more than {max_entries} entries",
                folder.display()
            )));
        }
        // Git names its packs in ASCII: a name that isn't text is no pack of Git's.
        let Ok(file) = entry.file_name().into_string() else {
            continue;
        };
        if !file.starts_with("pack-") {
            continue;
        }
        if let Some(name) = file.strip_suffix(".idx") {
            indexes.insert(name.to_owned());
        } else if let Some(name) = file.strip_suffix(".pack") {
            packs.insert(name.to_owned());
        }
    }
    // Safe to skip: Git renames a new .pack in before its .idx, and deletes an old .idx first.
    Ok(indexes.intersection(&packs).cloned().collect())
}

impl Pack {
    /// The pack `name` in `folder`, checked against its index; `None` if either file went away
    /// after the folder was listed, as when Git removes the pack. The packs before it listed
    /// `before` objects, and all may list at most `max`.
    fn load(
        folder: &Path,
        name: &str,
        hash: HashKind,
        before: u64,
        max: u64,
    ) -> Result<Option<Pack>, GitError> {
        let path = folder.join(format!("{name}.idx"));
        let Some(index) = read_index(&path, hash, before, max)? else {
            return Ok(None);
        };
        let path = folder.join(format!("{name}.pack"));
        let Some((file, len)) = open_measured(&path)? else {
            return Ok(None);
        };
        let (in_order, inflated) = pack_order(&index.offsets, &path)?;
        let pack = Pack {
            path,
            len,
            hash,
            names: index.names,
            offsets: index.offsets,
            in_order,
            inflated,
        };
        pack.check(&file, index.count, &index.pack_checksum)?;
        Ok(Some(pack))
    }

    /// Marks the entry at `at` as inflated, and says whether this is the first time. An offset
    /// the index doesn't list is never a first time.
    pub(super) fn first_inflation(&self, at: u64) -> bool {
        let found = self.in_order.binary_search_by_key(&at, |&place| {
            self.offsets
                .get(place as usize)
                .copied()
                .unwrap_or(u64::MAX)
        });
        let Ok(number) = found else {
            return false;
        };
        let Some(word) = self.inflated.get(number / 64) else {
            return false;
        };
        let bit = 1u64.wrapping_shl((number % 64) as u32);
        let first = word.get() & bit == 0;
        word.set(word.get() | bit);
        first
    }

    /// Checks the pack's header and checksum against its index, and that every offset the index
    /// gives is among the pack's entries.
    fn check(&self, file: &File, count: u32, checksum: &[u8]) -> Result<(), GitError> {
        let shown = self.path.display();
        if self.len < PACK_HEADER_LEN.saturating_add(self.hash.len() as u64) {
            return Err(broken(format!("{shown} is too short to be a pack")));
        }
        let mut header = [0u8; PACK_HEADER_LEN as usize];
        self.read_exact_at(file, &mut header, 0)?;
        let [p, a, c, k, v0, v1, v2, v3, n0, n1, n2, n3] = header;
        if [p, a, c, k] != *b"PACK" {
            return Err(broken(format!("{shown} is not a Git pack")));
        }
        let version = u32::from_be_bytes([v0, v1, v2, v3]);
        if !(2..=3).contains(&version) {
            return Err(broken(format!(
                "{shown} is a version {version} pack, which Git doesn't write"
            )));
        }
        let objects = u32::from_be_bytes([n0, n1, n2, n3]);
        if objects != count {
            return Err(broken(format!(
                "{shown} holds {objects} objects, but its index lists {count}"
            )));
        }
        let mut trailer = vec![0u8; self.hash.len()];
        self.read_exact_at(file, &mut trailer, self.entries_end())?;
        if trailer != checksum {
            return Err(broken(format!("{shown} doesn't match its index")));
        }
        let outside = |at: &&u64| **at < PACK_HEADER_LEN || **at >= self.entries_end();
        if let Some(at) = self.offsets.iter().find(outside) {
            return Err(broken(format!(
                "the index of {shown} gives the offset {at}, outside the pack's entries"
            )));
        }
        Ok(())
    }

    /// Fills `buf` from `at`, which the caller knows is inside the pack.
    fn read_exact_at(&self, file: &File, buf: &mut [u8], at: u64) -> Result<(), GitError> {
        let got = Input::new(file, self.path.as_path(), at, self.len).fill(buf)?;
        if got < buf.len() {
            return Err(shrank(&self.path));
        }
        Ok(())
    }

    /// Where the object `id` is in this pack, if it holds it.
    pub(super) fn find(&self, id: &GitId) -> Option<u64> {
        let index = match self.hash {
            HashKind::Sha1 => search::<20>(&self.names, id.as_bytes()),
            HashKind::Sha256 => search::<32>(&self.names, id.as_bytes()),
        }?;
        self.offsets.get(index).copied()
    }

    /// Opens the pack to read from it. `Changed` if Git removed it, or rewrote it to another
    /// length, since it was checked.
    pub(super) fn open(&self) -> Result<File, GitError> {
        let Some((file, len)) = open_measured(&self.path)? else {
            return Err(read_error(&self.path, io::ErrorKind::NotFound.into()));
        };
        if len != self.len {
            return Err(GitError::Changed {
                what: format!("{} changed size", self.path.display()),
            });
        }
        Ok(file)
    }

    /// Where the entries end and the pack's checksum starts.
    pub(super) fn entries_end(&self) -> u64 {
        self.len.saturating_sub(self.hash.len() as u64)
    }

    /// The header of the entry at `at`.
    pub(super) fn entry(&self, file: &File, at: u64) -> Result<Entry, GitError> {
        let mut head = [0u8; MAX_ENTRY_HEADER];
        let got = Input::new(file, self.path.as_path(), at, self.entries_end()).fill(&mut head)?;
        let damaged = |why: &str| {
            broken(format!(
                "the entry at offset {at} in {} {why}",
                self.path.display()
            ))
        };
        let mut rest = head.get(..got).unwrap_or_default();
        let (&first, tail) = rest
            .split_first()
            .ok_or_else(|| damaged("is past the end of the pack's entries"))?;
        rest = tail;
        // Four bits of size, then seven a byte, least significant first.
        let mut size = u64::from(first & 0x0f);
        if first & 0x80 != 0 {
            let high = varint(&mut rest, 9)
                .and_then(|high| high.checked_mul(16))
                .ok_or_else(|| damaged("has a damaged size"))?;
            size |= high;
        }
        let kind = match (first & 0x70).wrapping_shr(4) {
            1 => EntryKind::Whole(ObjectType::Commit),
            2 => EntryKind::Whole(ObjectType::Tree),
            3 => EntryKind::Whole(ObjectType::Blob),
            4 => EntryKind::Whole(ObjectType::Tag),
            6 => {
                let distance =
                    ofs_distance(&mut rest).ok_or_else(|| damaged("has a damaged base offset"))?;
                if distance == 0 {
                    return Err(damaged("names itself as its base"));
                }
                let base = at
                    .checked_sub(distance)
                    .filter(|base| *base >= PACK_HEADER_LEN)
                    .ok_or_else(|| {
                        damaged(&format!(
                            "names a base {distance} bytes back, before the pack's first entry"
                        ))
                    })?;
                EntryKind::OfsDelta(base)
            }
            7 => {
                let (base, tail) = rest
                    .split_at_checked(self.hash.len())
                    .ok_or_else(|| damaged("is cut off in its base's ID"))?;
                rest = tail;
                EntryKind::RefDelta(GitId::new(self.hash, base))
            }
            code => return Err(damaged(&format!("has type {code}, which Git doesn't use"))),
        };
        let used = got.saturating_sub(rest.len()) as u64;
        Ok(Entry {
            kind,
            size,
            data: at.saturating_add(used),
        })
    }
}

/// The regular file at `path` and its length, or `None` if nothing is there.
fn open_measured(path: &Path) -> Result<Option<(File, u64)>, GitError> {
    let file = match safe_fs::open_regular(path, OpenOptions::new().read(true)) {
        Ok(file) => file,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(read_error(path, e)),
    };
    let len = file.metadata().map_err(|e| read_error(path, e))?.len();
    Ok(Some((file, len)))
}

/// Places in `offsets` in the order their entries come in the pack, and a cleared bit for each.
fn pack_order(offsets: &[u64], path: &Path) -> Result<(Vec<u32>, Vec<Cell<u64>>), GitError> {
    let mut order: Vec<u32> = Vec::new();
    order
        .try_reserve_exact(offsets.len())
        .map_err(|_| out_of_memory(path))?;
    // The index's count is within the limit, so every place fits.
    order.extend(0..u32::try_from(offsets.len()).unwrap_or(u32::MAX));
    order.sort_unstable_by_key(|&place| offsets.get(place as usize).copied().unwrap_or(u64::MAX));
    let words = offsets.len().div_ceil(64);
    let mut bits = Vec::new();
    bits.try_reserve_exact(words)
        .map_err(|_| out_of_memory(path))?;
    bits.resize_with(words, Cell::default);
    Ok((order, bits))
}

/// Where `id` is among `names`, IDs of `N` bytes in order.
fn search<const N: usize>(names: &[u8], id: &[u8]) -> Option<usize> {
    names
        .as_chunks::<N>()
        .0
        .binary_search_by(|name| name.as_slice().cmp(id))
        .ok()
}

/// What Bolt keeps of an index.
struct Index {
    count: u32,
    names: Vec<u8>,
    offsets: Vec<u64>,
    /// The checksum its pack must end with.
    pack_checksum: Vec<u8>,
}

/// Reads the version 2 index at `path` a buffer at a time and checks it, keeping its names and
/// offsets; `None` if it's gone. The packs before it listed `before` objects, and all may list at
/// most `max`. Its count is checked against those and its length before anything is kept, and
/// its tables grow only as their names pass, so a table of zeros fails at its second name.
fn read_index(
    path: &Path,
    hash: HashKind,
    before: u64,
    max: u64,
) -> Result<Option<Index>, GitError> {
    let Some((file, len)) = open_measured(path)? else {
        return Ok(None);
    };
    let shown = path.display();
    let damaged = |why: String| broken(format!("{shown} {why}"));
    let hash_len = hash.len() as u64;
    if len < INDEX_HEAD_LEN.saturating_add(hash_len.saturating_mul(2)) {
        return Err(damaged("is too short to be a pack index".into()));
    }
    let mut index = Hashed {
        file: BufReader::with_capacity(64 << 10, file),
        hasher: Hasher::new(hash),
        path,
    };
    let mut head = [0u8; INDEX_HEAD_LEN as usize];
    index.fill(&mut head)?;
    let Some(([s0, s1, s2, s3, v0, v1, v2, v3], fanout)) = head.split_first_chunk::<8>() else {
        return Err(damaged("is too short to be a pack index".into()));
    };
    if [*s0, *s1, *s2, *s3] != *b"\xfftOc" {
        // Version 1 has no header: it starts with its fan-out.
        return Err(if is_old_index(&head, len, hash_len) {
            GitError::Refused(Refusal::OldPackIndex)
        } else {
            damaged("is not a pack index".into())
        });
    }
    let version = u32::from_be_bytes([*v0, *v1, *v2, *v3]);
    if version != 2 {
        return Err(damaged(format!(
            "is a version {version} pack index, which Git doesn't write"
        )));
    }
    let count =
        fanout_count(fanout).ok_or_else(|| damaged("has a fan-out that goes down".into()))?;
    let objects = u64::from(count);
    if objects > max.saturating_sub(before) {
        let others = match before {
            0 => ",".to_owned(),
            n => format!(", which with the {n} in the packs before it makes"),
        };
        return Err(GitError::TooManyObjects {
            what: format!(
                "{shown} lists {count} objects{others} more than the {max} Bolt reads from a \
                 repository's packs, whose indexes it keeps in memory"
            ),
        });
    }
    // Each object's name, CRC and offset, and the two checksums; then 8 bytes for each offset of
    // 2 GiB or more, which only a second object or later can need.
    let least = INDEX_HEAD_LEN
        .saturating_add(objects.saturating_mul(hash_len.saturating_add(8)))
        .saturating_add(hash_len.saturating_mul(2));
    let large = len.saturating_sub(least);
    if len < least
        || large > objects.saturating_sub(1).saturating_mul(8)
        || !large.is_multiple_of(8)
    {
        return Err(damaged(format!(
            "is {len} bytes long, which doesn't fit the {count} objects its fan-out lists"
        )));
    }
    // Each name must come after the one before it, checked before it is kept.
    let listed = usize::try_from(objects.saturating_mul(hash_len)).unwrap_or(usize::MAX);
    let mut names: Vec<u8> = Vec::new();
    index.entries(objects, hash.len(), |name| {
        let last = names
            .len()
            .checked_sub(name.len())
            .and_then(|at| names.get(at..));
        if last.is_some_and(|last| last >= name) {
            return Err(damaged("lists its objects out of order".into()));
        }
        if names.len() == names.capacity() {
            // Doubling as the names pass, but never past what the index lists: no room is left
            // over, so the table stays at 20 or 32 bytes an object.
            let more = names.len().max(name.len());
            let more = more.min(listed.saturating_sub(names.len()));
            names
                .try_reserve_exact(more)
                .map_err(|_| out_of_memory(path))?;
        }
        names.extend_from_slice(name);
        Ok(())
    })?;
    // The CRCs: Bolt checks every object against its ID instead.
    index.skip(objects.saturating_mul(4))?;
    // The names passed, so the count is real. Offsets as Git stores them: one with its top bit
    // set is a row of the table of 64-bit offsets that follows.
    let mut offsets: Vec<u64> = Vec::new();
    offsets
        .try_reserve_exact(usize::try_from(count).unwrap_or(usize::MAX))
        .map_err(|_| out_of_memory(path))?;
    index.entries(objects, 4, |small| {
        offsets.push(u64::from(u32::from_be_bytes(
            small.try_into().unwrap_or_default(),
        )));
        Ok(())
    })?;
    let mut table: Vec<u64> = Vec::new();
    index.entries(large / 8, 8, |big| {
        table.try_reserve(1).map_err(|_| out_of_memory(path))?;
        table.push(u64::from_be_bytes(big.try_into().unwrap_or_default()));
        Ok(())
    })?;
    for offset in &mut offsets {
        if *offset & 0x8000_0000 != 0 {
            let row = usize::try_from(*offset & 0x7fff_ffff).unwrap_or(usize::MAX);
            *offset = *table
                .get(row)
                .ok_or_else(|| damaged("points past its table of 64-bit offsets".into()))?;
        }
    }
    let mut pack_checksum = vec![0u8; hash.len()];
    index.fill(&mut pack_checksum)?;
    let mut checksum = vec![0u8; hash.len()];
    index.fill_unhashed(&mut checksum)?;
    let actual = index.hasher.finish(|| shown.to_string())?;
    if actual.as_bytes() != checksum.as_slice() {
        return Err(damaged("doesn't match its checksum".into()));
    }
    Ok(Some(Index {
        count,
        names,
        offsets,
        pack_checksum,
    }))
}

/// The object count an index's fan-out gives, if its 256 counts never go down.
fn fanout_count(fanout: &[u8]) -> Option<u32> {
    let (counts, rest) = fanout.as_chunks::<4>();
    if counts.len() != 256 || !rest.is_empty() {
        return None;
    }
    let mut last = 0;
    for count in counts {
        let count = u32::from_be_bytes(*count);
        if count < last {
            return None;
        }
        last = count;
    }
    Some(last)
}

/// Whether an index without the version 2 header is a whole one of version 1: a fan-out that
/// never goes down, and the length its count needs.
fn is_old_index(head: &[u8], len: u64, hash_len: u64) -> bool {
    let fanout = head.get(..1024).and_then(fanout_count);
    fanout.is_some_and(|count| {
        let entries = u64::from(count).saturating_mul(hash_len.saturating_add(4));
        len == entries
            .saturating_add(1024)
            .saturating_add(hash_len.saturating_mul(2))
    })
}

fn out_of_memory(path: &Path) -> GitError {
    GitError::Io {
        path: path.to_owned(),
        source: io::ErrorKind::OutOfMemory.into(),
    }
}

/// An index read in order, a buffer at a time, hashing what it reads for the checksum at its
/// end.
struct Hashed<'p> {
    file: BufReader<File>,
    hasher: Hasher,
    path: &'p Path,
}

impl Hashed<'_> {
    fn fill(&mut self, buf: &mut [u8]) -> Result<(), GitError> {
        self.fill_unhashed(buf)?;
        self.hasher.update(buf);
        Ok(())
    }

    fn fill_unhashed(&mut self, buf: &mut [u8]) -> Result<(), GitError> {
        self.file.read_exact(buf).map_err(|e| match e.kind() {
            // Its length was measured first, so it got shorter since.
            io::ErrorKind::UnexpectedEof => shrank(self.path),
            _ => read_error(self.path, e),
        })
    }

    /// Reads `count` entries of `width` bytes a buffer at a time, handing each to `each` in turn.
    fn entries(
        &mut self,
        count: u64,
        width: usize,
        mut each: impl FnMut(&[u8]) -> Result<(), GitError>,
    ) -> Result<(), GitError> {
        let width = width.max(1);
        let mut buf = vec![0u8; ENTRIES_AT_A_TIME.saturating_mul(width)];
        let mut left = count;
        while left > 0 {
            let n = ENTRIES_AT_A_TIME.min(usize::try_from(left).unwrap_or(usize::MAX));
            let part = buf.get_mut(..n.saturating_mul(width)).unwrap_or_default();
            self.fill(part)?;
            for entry in part.chunks_exact(width) {
                each(entry)?;
            }
            left = left.saturating_sub(n as u64);
        }
        Ok(())
    }

    /// Hashes `len` bytes without keeping them.
    fn skip(&mut self, mut len: u64) -> Result<(), GitError> {
        let mut scratch = [0u8; 8192];
        while len > 0 {
            let step = scratch
                .len()
                .min(usize::try_from(len).unwrap_or(usize::MAX));
            self.fill(scratch.get_mut(..step).unwrap_or_default())?;
            len = len.saturating_sub(step as u64);
        }
        Ok(())
    }
}

/// The base and result sizes a delta starts with.
pub(super) fn delta_sizes(mut delta: &[u8]) -> Option<(u64, u64)> {
    Some((varint(&mut delta, 10)?, varint(&mut delta, 10)?))
}

/// Why a delta can't be applied.
pub(super) enum DeltaError {
    /// Reading it failed.
    Read(io::Error),
    /// It is damaged: how, said to follow "the delta …".
    Damaged(String),
}

/// The object `delta` builds on `base`, with every instruction checked; `Err` says what is wrong
/// with the delta. For `Craft` and the delta fuzz target: imports apply deltas as they inflate.
#[cfg(any(test, feature = "test-support"))]
pub fn apply_delta(base: &[u8], mut delta: &[u8]) -> Result<Vec<u8>, String> {
    let built = delta_start(base, &mut delta).and_then(|size| build_delta(base, size, &mut delta));
    built.map_err(|e| match e {
        DeltaError::Damaged(why) => why,
        // Reading a slice never fails.
        DeltaError::Read(e) => e.to_string(),
    })
}

/// Reads the sizes a delta starts with, and returns what it builds once its base's size is
/// checked against `base`.
pub(super) fn delta_start(base: &[u8], delta: &mut impl BufRead) -> Result<u64, DeltaError> {
    let damaged_size = || DeltaError::Damaged("starts with a damaged size".to_owned());
    let base_size = read_varint(delta)?.ok_or_else(damaged_size)?;
    if base_size != base.len() as u64 {
        return Err(DeltaError::Damaged(format!(
            "says its base holds {base_size} bytes, but it holds {}",
            base.len()
        )));
    }
    read_varint(delta)?.ok_or_else(damaged_size)
}

/// The object of `size` bytes that the rest of `delta` builds on `base`, read an instruction at a
/// time as it comes, so the delta is never held whole. The size is checked before anything is
/// allocated, and every instruction before it is carried out.
pub(super) fn build_delta(
    base: &[u8],
    size: u64,
    delta: &mut impl BufRead,
) -> Result<Vec<u8>, DeltaError> {
    let damaged = DeltaError::Damaged;
    if size > MAX_IN_MEMORY_OBJECT {
        return Err(damaged(format!(
            "would build {size} bytes, more than the {MAX_IN_MEMORY_OBJECT} Bolt rebuilds in memory"
        )));
    }
    let size = usize::try_from(size).unwrap_or(usize::MAX);
    let mut out = Vec::new();
    out.try_reserve_exact(size).map_err(|_| {
        damaged(format!(
            "would build {size} bytes, more memory than Bolt could get"
        ))
    })?;
    let cut = || DeltaError::Damaged("ends inside an instruction".to_owned());
    let mut inserted = [0u8; 0x7f];
    while let Some(op) = next_byte(delta)? {
        let part = if op & 0x80 != 0 {
            // A copy from the base: the bits of `op` say which bytes of its offset and size
            // follow, least significant first.
            let (mut offset, mut len) = ([0u8; 4], [0u8; 4]);
            let offset_bytes = offset.iter_mut().zip([0x01, 0x02, 0x04, 0x08]);
            for (byte, bit) in offset_bytes.chain(len.iter_mut().zip([0x10, 0x20, 0x40])) {
                if op & bit != 0 {
                    *byte = next_byte(delta)?.ok_or_else(cut)?;
                }
            }
            let offset = u32::from_le_bytes(offset) as usize;
            let len = match u32::from_le_bytes(len) {
                0 => 0x10000,
                len => len as usize,
            };
            offset
                .checked_add(len)
                .and_then(|end| base.get(offset..end))
                .ok_or_else(|| damaged("copies from past the end of its base".to_owned()))?
        } else if op != 0 {
            let bytes = inserted.get_mut(..usize::from(op)).unwrap_or_default();
            delta.read_exact(bytes).map_err(|e| match e.kind() {
                io::ErrorKind::UnexpectedEof => cut(),
                _ => DeltaError::Read(e),
            })?;
            &*bytes
        } else {
            return Err(damaged("uses instruction 0, which Git reserves".to_owned()));
        };
        if part.len() > size.saturating_sub(out.len()) {
            return Err(damaged(format!(
                "builds more than the {size} bytes it says"
            )));
        }
        out.extend_from_slice(part);
    }
    if out.len() != size {
        return Err(damaged(format!(
            "builds {} bytes, not the {size} it says",
            out.len()
        )));
    }
    Ok(out)
}

/// The next byte of `delta`, or `None` at its end.
fn next_byte(delta: &mut impl BufRead) -> Result<Option<u8>, DeltaError> {
    let byte = delta.fill_buf().map_err(DeltaError::Read)?.first().copied();
    if byte.is_some() {
        delta.consume(1);
    }
    Ok(byte)
}

/// A number in Git's size encoding read from `delta`, as `varint` reads one: `None` if it is
/// damaged.
fn read_varint(delta: &mut impl BufRead) -> Result<Option<u64>, DeltaError> {
    let mut bytes = [0u8; 10];
    let mut len = 0usize;
    for slot in &mut bytes {
        let Some(byte) = next_byte(delta)? else {
            break;
        };
        *slot = byte;
        len = len.saturating_add(1);
        if byte & 0x80 == 0 {
            break;
        }
    }
    Ok(varint(&mut bytes.get(..len).unwrap_or_default(), 10))
}

/// A number in Git's size encoding, seven bits a byte, least significant first, while the top
/// bit is set: at most `max` bytes, and it must fit 64 bits. Moves `bytes` past it.
fn varint(bytes: &mut &[u8], max: u32) -> Option<u64> {
    let mut value = 0u128;
    for shift in (0..max.checked_mul(7)?).step_by(7) {
        let (&byte, rest) = bytes.split_first()?;
        *bytes = rest;
        value |= u128::from(byte & 0x7f).checked_shl(shift)?;
        if byte & 0x80 == 0 {
            return u64::try_from(value).ok();
        }
    }
    None
}

/// An `OFS_DELTA`'s distance back to its base, in Git's offset encoding: seven bits a byte,
/// most significant first, with one added before each shift. Moves `bytes` past it.
fn ofs_distance(bytes: &mut &[u8]) -> Option<u64> {
    let (&first, rest) = bytes.split_first()?;
    *bytes = rest;
    let mut distance = u64::from(first & 0x7f);
    let mut more = first & 0x80 != 0;
    while more {
        let (&byte, rest) = bytes.split_first()?;
        *bytes = rest;
        distance = distance
            .checked_add(1)?
            .checked_mul(128)?
            .checked_add(u64::from(byte & 0x7f))?;
        more = byte & 0x80 != 0;
    }
    Some(distance)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    #![allow(clippy::indexing_slicing, clippy::arithmetic_side_effects)]
    use super::*;
    use crate::git::Repo;
    use crate::git::craft::{Craft, PackEntry, delta, entry_header};
    use crate::git::object_id;

    use ObjectType::{Blob, Tree};

    fn craft() -> (tempfile::TempDir, Craft) {
        let dir = tempfile::tempdir().unwrap();
        let git = Craft::new(dir.path(), HashKind::Sha1);
        (dir, git)
    }

    fn open(dir: &tempfile::TempDir) -> Repo {
        Repo::open(dir.path()).unwrap()
    }

    fn id(kind: ObjectType, data: &[u8]) -> GitId {
        object_id(kind, data, HashKind::Sha1).unwrap()
    }

    /// The repository at `dir` without `Repo::open`'s checks, which tests that open it
    /// thousands of times would only repeat.
    fn reopen(dir: &tempfile::TempDir) -> Repo {
        Repo {
            git: dir.path().join(".git"),
            hash: HashKind::Sha1,
            includes: false,
            lfs_storage: None,
            objects: Default::default(),
        }
    }

    fn deflate(bytes: &[u8]) -> Vec<u8> {
        miniz_oxide::deflate::compress_to_vec_zlib(bytes, 6)
    }

    /// The message of a `Broken` error, failing the test on anything else.
    #[track_caller]
    fn broken<T: std::fmt::Debug>(result: Result<T, GitError>) -> String {
        match result {
            Err(e @ GitError::Broken { .. }) => {
                assert!(e.hint().is_some());
                e.to_string()
            }
            other => panic!("expected Broken, got {other:?}"),
        }
    }

    /// Rewrites the SHA-1 index at `path` with `change` made, and a checksum that fits again.
    fn patch_index(path: &Path, change: impl FnOnce(&mut Vec<u8>)) {
        let mut bytes = fs::read(path).unwrap();
        change(&mut bytes);
        let body = bytes.len() - 20;
        let mut hasher = Hasher::new(HashKind::Sha1);
        hasher.update(&bytes[..body]);
        let checksum = hasher.finish(String::new).unwrap();
        bytes.truncate(body);
        bytes.extend_from_slice(checksum.as_bytes());
        fs::write(path, bytes).unwrap();
    }

    /// Where the index's 4-byte offsets start, for `count` SHA-1 objects.
    fn offsets_at(count: usize) -> usize {
        8 + 1024 + count * (20 + 4)
    }

    #[test]
    fn large_offsets_are_followed() {
        let blobs: Vec<Vec<u8>> = (0..5)
            .map(|i| format!("blob number {i}\n").into_bytes())
            .collect();
        let changed = b"blob number 0, changed\n".to_vec();
        let mut entries: Vec<PackEntry> = blobs
            .iter()
            .map(|b| PackEntry::Whole(Blob, b.clone()))
            .collect();
        entries.push(PackEntry::OfsDelta {
            base: 0,
            delta: delta(&blobs[0], &changed),
        });
        // Every offset past the first entry's in the 64-bit table, then only some.
        let small_len = 8 + 1024 + entries.len() * (20 + 8) + 2 * 20;
        for from in [13, 60] {
            let (dir, git) = craft();
            let (_, idx) = git.pack_with_large_offsets(&entries, from);
            let large = (fs::read(&idx).unwrap().len() - small_len) / 8;
            assert!((1..entries.len()).contains(&large), "{from}: {large}");
            let repo = open(&dir);
            for data in blobs.iter().chain([&changed]) {
                assert_eq!(&repo.read(&id(Blob, data)).unwrap().data, data);
            }
        }
        // The first entry, at offset 12, never needs one, so Git's own reader refuses an index
        // that is long enough to give every object a large offset.
        let (dir, git) = craft();
        git.pack_with_large_offsets(&entries, 0);
        let message = broken(open(&dir).read(&id(Blob, &blobs[0])));
        assert!(message.contains("doesn't fit the 6 objects"), "{message}");
        // A large offset must be in the table: this one points past its end.
        let (dir, git) = craft();
        let (_, idx) = git.pack_with_large_offsets(&entries, 13);
        let at = offsets_at(entries.len());
        patch_index(&idx, |bytes| {
            bytes[at..at + 4].copy_from_slice(&(0x8000_0000u32 | 99).to_be_bytes());
        });
        let message = broken(open(&dir).read(&id(Blob, &blobs[0])));
        assert!(message.contains("64-bit"), "{message}");
    }

    #[test]
    fn a_pack_claiming_four_billion_objects_is_refused() {
        let (dir, git) = craft();
        let (pack, idx) = git.pack(&[PackEntry::Whole(Blob, b"x".to_vec())]);
        let x = id(Blob, b"x");
        // The pack says so while its index lists one.
        let good = fs::read(&pack).unwrap();
        let mut claiming = good.clone();
        claiming[8..12].copy_from_slice(&u32::MAX.to_be_bytes());
        fs::write(&pack, &claiming).unwrap();
        let message = broken(open(&dir).read(&x));
        assert!(message.contains("4294967295"), "{message}");
        fs::write(&pack, &good).unwrap();
        // The index says so: past Bolt's limit, which is found before its tables are read, so
        // nothing that size is allocated.
        claim(&idx, 4_000_000_000);
        match open(&dir).read(&x) {
            Err(GitError::TooManyObjects { what }) => {
                assert!(what.contains("4000000000"), "{what}")
            }
            other => panic!("{other:?}"),
        }
    }

    /// An index whose fan-out says it lists `count` objects, with the checksum fixed.
    fn claim(idx: &Path, count: u64) {
        let count = u32::try_from(count).unwrap();
        patch_index(idx, |bytes| {
            for at in (8..8 + 1024).step_by(4) {
                bytes[at..at + 4].copy_from_slice(&count.to_be_bytes());
            }
        });
    }

    #[test]
    fn an_index_past_the_object_limit_is_refused_on_its_count() {
        let (dir, git) = craft();
        let (_, idx) = git.pack(&[PackEntry::Whole(Blob, b"x".to_vec())]);
        let x = id(Blob, b"x");
        // Refused before its length is even looked at, so before any table is read.
        claim(&idx, MAX_GIT_OBJECTS + 1);
        match open(&dir).read(&x) {
            Err(e @ GitError::TooManyObjects { .. }) => {
                let message = e.to_string();
                let says = format!("lists 16777217 objects, more than the {MAX_GIT_OBJECTS} Bolt");
                assert!(message.contains(&says), "{message}");
                assert!(message.contains("keeps in memory"), "{message}");
                assert!(e.hint().unwrap().contains("git repack -a -d"), "{e:?}");
            }
            other => panic!("{other:?}"),
        }
        // At the limit, only the length is wrong.
        claim(&idx, MAX_GIT_OBJECTS);
        let message = broken(open(&dir).read(&x));
        assert!(
            message.contains("doesn't fit the 16777216 objects"),
            "{message}"
        );
    }

    #[test]
    fn the_object_limit_counts_the_objects_of_every_pack() {
        let (dir, git) = craft();
        git.pack(&[
            PackEntry::Whole(Blob, b"one".to_vec()),
            PackEntry::Whole(Blob, b"two".to_vec()),
        ]);
        git.pack(&[
            PackEntry::Whole(Blob, b"three".to_vec()),
            PackEntry::Whole(Blob, b"four".to_vec()),
        ]);
        let folder = dir.path().join(".git/objects/pack");
        let loaded = load_at_most(&folder, HashKind::Sha1, 4, MAX_PACKS).unwrap();
        assert_eq!(loaded.packs.len(), 2);
        match load_at_most(&folder, HashKind::Sha1, 3, MAX_PACKS).err() {
            Some(GitError::TooManyObjects { what }) => {
                let says = "lists 2 objects, which with the 2 in the packs before it makes more \
                            than the 3";
                assert!(what.contains(says), "{what}");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn names_are_checked_in_order_across_the_buffers_they_are_read_in() {
        let (dir, git) = craft();
        let blobs: Vec<Vec<u8>> = (0..ENTRIES_AT_A_TIME + 50)
            .map(|i| format!("blob {i}\n").into_bytes())
            .collect();
        let entries: Vec<PackEntry> = blobs
            .iter()
            .map(|b| PackEntry::Whole(Blob, b.clone()))
            .collect();
        let (pack, idx) = git.pack(&entries);
        let repo = open(&dir);
        for data in &blobs {
            assert_eq!(&repo.read(&id(Blob, data)).unwrap().data, data);
        }
        // The names table holds them with no room to spare.
        let folder = pack.parent().unwrap();
        let name = list(folder).unwrap().remove(0);
        let loaded = Pack::load(folder, &name, HashKind::Sha1, 0, MAX_GIT_OBJECTS).unwrap();
        let names = loaded.unwrap().names;
        assert_eq!(
            (names.len(), names.capacity()),
            (blobs.len() * 20, blobs.len() * 20)
        );
        // The last name of the first buffer swapped with the first of the second.
        let at = 8 + 1024 + (ENTRIES_AT_A_TIME - 1) * 20;
        patch_index(&idx, |bytes| {
            let (last, first) = bytes[at..at + 40].split_at_mut(20);
            last.swap_with_slice(first);
        });
        let message = broken(open(&dir).read(&id(Blob, &blobs[0])));
        assert!(message.contains("out of order"), "{message}");
    }

    /// A pack of a blob, a delta on it and a tree, and the IDs of the three.
    fn small_pack(git: &Craft) -> ([GitId; 3], PathBuf, PathBuf) {
        let base = b"hello, hello, hello\n".to_vec();
        let edited = b"hello, hello, goodbye\n".to_vec();
        let tree = [b"100644 a\0".as_slice(), id(Blob, &base).as_bytes()].concat();
        let ids = [id(Blob, &base), id(Blob, &edited), id(Tree, &tree)];
        let (pack, idx) = git.pack(&[
            PackEntry::Whole(Blob, base.clone()),
            PackEntry::OfsDelta {
                base: 0,
                delta: delta(&base, &edited),
            },
            PackEntry::Whole(Tree, tree),
        ]);
        (ids, pack, idx)
    }

    #[test]
    fn every_truncation_of_a_pack_or_index_is_an_error() {
        let (dir, git) = craft();
        let (ids, pack, idx) = small_pack(&git);
        assert!(ids.iter().all(|id| open(&dir).read(id).is_ok()));
        for path in [&pack, &idx] {
            let good = fs::read(path).unwrap();
            // Every length in the last 160 bytes: all of the pack, and all of the index past its
            // fan-out. Before that every cut fails the same first check, so every 16th will do.
            let cuts = (0..good.len()).filter(|len| len % 16 == 0 || len + 160 >= good.len());
            for len in cuts {
                fs::write(path, &good[..len]).unwrap();
                let repo = reopen(&dir);
                let what = format!("{} cut to {len} bytes", path.display());
                for id in &ids {
                    assert!(repo.read(id).is_err(), "{what}");
                }
                // The deltified blob, the one the most is read for.
                assert!(repo.header(&ids[1]).is_err(), "{what}");
                assert!(repo.blob_reader(&ids[1]).is_err(), "{what}");
            }
            fs::write(path, &good).unwrap();
        }
    }

    #[test]
    fn every_changed_byte_of_a_pack_is_refused_or_harmless() {
        let (dir, git) = craft();
        let (ids, pack, _) = small_pack(&git);
        let good = fs::read(&pack).unwrap();
        let objects: Vec<_> = ids.iter().map(|id| open(&dir).read(id).unwrap()).collect();
        for at in 0..good.len() {
            for flip in [0x01, 0x80, 0xff] {
                let mut bytes = good.clone();
                bytes[at] ^= flip;
                fs::write(&pack, &bytes).unwrap();
                let repo = reopen(&dir);
                // Each object either reads back exactly or is refused.
                for (id, object) in ids.iter().zip(&objects) {
                    if let Ok(read) = repo.read(id) {
                        assert_eq!(&read, object, "byte {at} ^ {flip:#x}");
                    }
                }
            }
        }
    }

    #[test]
    fn bad_deltas_are_refused() {
        let (dir, git) = craft();
        let base = b"0123456789".to_vec();
        // A delta saying it builds `result` bytes on `base` bytes, with these instructions.
        let with = |base: u8, result: u8, ops: &[u8]| [&[base, result][..], ops].concat();
        let copy_past_base_end = with(10, 5, &[0x91, 8, 5]);
        let opcode_zero = with(10, 1, &[0x00]);
        let result_size_mismatch = with(10, 5, &[0x90, 3]);
        let base_size_mismatch = with(9, 3, &[0x90, 3]);
        let cases = [
            (copy_past_base_end, "past the end of its base"),
            (opcode_zero, "instruction 0"),
            (result_size_mismatch, "builds 3 bytes, not the 5"),
            (base_size_mismatch, "base holds 9 bytes"),
            (with(10, 2, &[0x90, 3]), "more than the 2 bytes"),
            (with(10, 5, &[0x05, b'a', b'b']), "ends inside"),
            (with(10, 5, &[0x91]), "ends inside"),
            (vec![0x80], "damaged size"),
        ];
        let mut entries = vec![PackEntry::Whole(Blob, base.clone())];
        entries.extend(cases.iter().map(|(bad, _)| PackEntry::OfsDelta {
            base: 0,
            delta: bad.clone(),
        }));
        git.pack(&entries);
        let repo = open(&dir);
        for (bad, why) in &cases {
            // Craft lists a delta that doesn't apply as a blob of its bytes.
            let message = broken(repo.read(&id(Blob, bad)));
            assert!(message.contains(why), "{message}");
            assert!(message.contains("delta"), "{message}");
        }
        // A copy of size 0 copies 0x10000 bytes: Git writes no size for one that long.
        let long = vec![7u8; 0x10000];
        let longer = [long.as_slice(), b"!"].concat();
        let copy_all = delta(&long, &longer);
        assert_eq!(
            copy_all,
            [0x80, 0x80, 0x04, 0x81, 0x80, 0x04, 0x80, 0x01, b'!']
        );
        let (dir, git) = craft();
        git.pack(&[
            PackEntry::Whole(Blob, long),
            PackEntry::OfsDelta {
                base: 0,
                delta: copy_all,
            },
        ]);
        assert_eq!(open(&dir).read(&id(Blob, &longer)).unwrap().data, longer);
    }

    #[test]
    fn a_delta_claiming_a_huge_result_is_refused_before_allocating() {
        let (dir, git) = craft();
        // Base size 1, then a result of 2^62 bytes in Git's size encoding.
        let mut huge = vec![1];
        let mut size = 1u64 << 62;
        while size >= 0x80 {
            huge.push(size as u8 | 0x80);
            size >>= 7;
        }
        huge.push(size as u8);
        huge.extend_from_slice(&[0x01, b'x']);
        // And a delta whose own data says it is 2^40 bytes.
        let big_delta = [
            entry_header(7, 1 << 40),
            id(Blob, b"x").as_bytes().to_vec(),
            deflate(b"\x01\x01\x01x"),
        ]
        .concat();
        git.pack(&[
            PackEntry::Whole(Blob, b"x".to_vec()),
            PackEntry::OfsDelta {
                base: 0,
                delta: huge.clone(),
            },
            PackEntry::Raw(big_delta.clone()),
        ]);
        let repo = open(&dir);
        let message = broken(repo.read(&id(Blob, &huge)));
        assert!(message.contains(&(1u64 << 62).to_string()), "{message}");
        let message = broken(repo.read(&id(Blob, &big_delta)));
        assert!(message.contains(&(1u64 << 40).to_string()), "{message}");
        // The header only reports what the delta says.
        assert_eq!(repo.header(&id(Blob, &huge)).unwrap(), (Blob, 1 << 62));
    }

    #[test]
    fn a_delta_is_applied_as_it_inflates_and_holds_exactly_what_its_entry_says() {
        let size = |mut n: usize| {
            let mut out = Vec::new();
            while n >= 0x80 {
                out.push(n as u8 | 0x80);
                n >>= 7;
            }
            out.push(n as u8);
            out
        };
        let base: Vec<u8> = (0..100_000u32).map(|i| (i % 251) as u8).collect();
        // Inserts of every length and copies from all over the base, about 200 KiB of them, so
        // instructions fall across the buffers the delta is read through.
        let (mut ops, mut built) = (Vec::new(), Vec::new());
        for i in 0..3000u32 {
            let insert: Vec<u8> = (0..i % 127 + 1).map(|j| (i ^ j) as u8).collect();
            ops.push(insert.len() as u8);
            ops.extend_from_slice(&insert);
            built.extend_from_slice(&insert);
            let (offset, len) = (i * 31 % 90_000, i % 500 + 1);
            ops.push(0x80 | 0x0f | 0x30);
            ops.extend_from_slice(&offset.to_le_bytes());
            ops.extend_from_slice(&(len as u16).to_le_bytes());
            built.extend_from_slice(&base[offset as usize..(offset + len) as usize]);
        }
        let delta = [size(base.len()), size(built.len()), ops].concat();
        // Entries saying the delta holds a byte more, and a byte less, than it does.
        let says = |len: usize| {
            let base = id(Blob, &base);
            [
                entry_header(7, len as u64),
                base.as_bytes().to_vec(),
                deflate(&delta),
            ]
            .concat()
        };
        let (long, short) = (says(delta.len() + 1), says(delta.len() - 1));
        let (dir, git) = craft();
        git.pack(&[
            PackEntry::Whole(Blob, base.clone()),
            PackEntry::OfsDelta {
                base: 0,
                delta: delta.clone(),
            },
            PackEntry::Raw(long.clone()),
            PackEntry::Raw(short.clone()),
        ]);
        let repo = open(&dir);
        assert!(repo.read(&id(Blob, &built)).unwrap().data == built);
        match repo.read(&id(Blob, &long)) {
            Err(e @ GitError::EndedEarly { .. }) => {
                let says = format!(
                    "ended after {} of the {} bytes",
                    delta.len(),
                    delta.len() + 1
                );
                assert!(e.to_string().contains(&says), "{e}");
            }
            other => panic!("{other:?}"),
        }
        let message = broken(repo.read(&id(Blob, &short)));
        let says = format!("holds more than the {} bytes it says", delta.len() - 1);
        assert!(message.contains(&says), "{message}");
    }

    #[test]
    fn damaged_indexes_and_packs_are_refused() {
        type Damage = fn(&Path, &Path);
        let cases: [(&str, Damage, &str); 11] = [
            (
                "an index without its signature",
                |_, idx| patch_index(idx, |b| b[0] = 0),
                "not a pack index",
            ),
            (
                "index version 3",
                |_, idx| patch_index(idx, |b| b[7] = 3),
                "version 3",
            ),
            (
                "a fan-out that goes down",
                |_, idx| patch_index(idx, |b| b[8..12].copy_from_slice(&3u32.to_be_bytes())),
                "fan-out",
            ),
            (
                "names out of order",
                |_, idx| {
                    patch_index(idx, |b| {
                        let (first, second) = b[1032..1072].split_at_mut(20);
                        first.swap_with_slice(second);
                    })
                },
                "out of order",
            ),
            (
                "a name twice",
                |_, idx| patch_index(idx, |b| b.copy_within(1032..1052, 1052)),
                "out of order",
            ),
            (
                "an offset past the pack's entries",
                |_, idx| {
                    let at = offsets_at(2);
                    patch_index(idx, |b| {
                        b[at..at + 4].copy_from_slice(&1000u32.to_be_bytes())
                    })
                },
                "outside",
            ),
            (
                "an offset inside the pack's header",
                |_, idx| {
                    let at = offsets_at(2);
                    patch_index(idx, |b| b[at..at + 4].copy_from_slice(&4u32.to_be_bytes()))
                },
                "outside",
            ),
            (
                "a checksum that doesn't fit",
                |_, idx| {
                    let mut b = fs::read(idx).unwrap();
                    b[1100] ^= 1;
                    fs::write(idx, b).unwrap();
                },
                "checksum",
            ),
            (
                "an index of another pack",
                |_, idx| {
                    patch_index(idx, |b| {
                        let at = b.len() - 40;
                        b[at] ^= 1;
                    })
                },
                "doesn't match its index",
            ),
            (
                "a pack without its signature",
                |pack, _| {
                    let mut b = fs::read(pack).unwrap();
                    b[0] = b'X';
                    fs::write(pack, b).unwrap();
                },
                "not a Git pack",
            ),
            (
                "pack version 4",
                |pack, _| {
                    let mut b = fs::read(pack).unwrap();
                    b[7] = 4;
                    fs::write(pack, b).unwrap();
                },
                "version 4",
            ),
        ];
        let two = || {
            let (dir, git) = craft();
            let (pack, idx) = git.pack(&[
                PackEntry::Whole(Blob, b"one".to_vec()),
                PackEntry::Whole(Blob, b"two".to_vec()),
            ]);
            (dir, pack, idx)
        };
        for (what, damage, expected) in cases {
            let (dir, pack, idx) = two();
            damage(&pack, &idx);
            let message = broken(open(&dir).read(&id(Blob, b"one")));
            assert!(message.contains(expected), "{what}: {message}");
        }
        // A folder where the pack belongs isn't a plain file, which Bolt refuses as it refuses a
        // link there.
        let (dir, pack, _) = two();
        fs::remove_file(&pack).unwrap();
        fs::create_dir(&pack).unwrap();
        match open(&dir).read(&id(Blob, b"one")) {
            Err(GitError::NotPlain { what }) => {
                assert!(what.contains("not a regular file"), "{what}")
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn an_old_index_is_refused_with_a_way_out() {
        let (dir, git) = craft();
        let (pack, idx) = git.pack(&[PackEntry::Whole(Blob, b"one".to_vec())]);
        let one = id(Blob, b"one");
        // Version 1: the fan-out, then each object's offset and name, then the checksums.
        let mut old = Vec::new();
        for byte in 0..=255u8 {
            let below = u32::from(one.as_bytes()[0] <= byte);
            old.extend_from_slice(&below.to_be_bytes());
        }
        old.extend_from_slice(&12u32.to_be_bytes());
        old.extend_from_slice(one.as_bytes());
        let pack_bytes = fs::read(&pack).unwrap();
        old.extend_from_slice(&pack_bytes[pack_bytes.len() - 20..]);
        old.extend_from_slice(&[0; 20]);
        fs::write(&idx, &old).unwrap();
        patch_index(&idx, |_| {});
        let err = open(&dir).read(&one).unwrap_err();
        assert!(
            matches!(&err, GitError::Refused(Refusal::OldPackIndex)),
            "{err:?}"
        );
        assert!(err.hint().unwrap().contains("git repack -a -d"));
    }

    #[test]
    fn a_pack_file_without_its_partner_is_skipped_as_git_skips_it() {
        // A .pack Git is still writing has no .idx yet; an .idx whose .pack is gone is a pack
        // being removed.
        for (alone, gone) in [(".pack", ".idx"), (".idx", ".pack")] {
            let (dir, git) = craft();
            let (pack, idx) = git.pack(&[PackEntry::Whole(Blob, b"one".to_vec())]);
            let loose = git.object(Blob, b"loose");
            fs::remove_file(if gone == ".idx" { &idx } else { &pack }).unwrap();
            assert_eq!(list(pack.parent().unwrap()).unwrap(), Vec::<String>::new());
            let repo = open(&dir);
            assert_eq!(repo.read(&loose).unwrap().data, b"loose", "{alone}");
            // Its objects aren't found, and nothing changed since the listing.
            let message = broken(repo.read(&id(Blob, b"one")));
            assert!(message.contains("missing object"), "{alone}: {message}");
        }
        // Either file gone between the listing and the reading: the pack is skipped too.
        for gone in [".idx", ".pack"] {
            let (_dir, git) = craft();
            let (pack, idx) = git.pack(&[PackEntry::Whole(Blob, b"one".to_vec())]);
            let folder = pack.parent().unwrap();
            let names = list(folder).unwrap();
            assert_eq!(names.len(), 1);
            fs::remove_file(if gone == ".idx" { &idx } else { &pack }).unwrap();
            let read = Pack::load(folder, &names[0], HashKind::Sha1, 0, MAX_GIT_OBJECTS).unwrap();
            assert!(read.is_none(), "{gone}");
        }
    }

    #[test]
    fn a_pack_folder_past_its_limit_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..11 {
            fs::write(dir.path().join(format!("pack-{i}.keep")), b"").unwrap();
        }
        let message = broken(list_at_most(dir.path(), 10));
        assert!(message.contains("more than 10 entries"), "{message}");
        assert!(list_at_most(dir.path(), 11).unwrap().is_empty());
    }

    #[test]
    fn packs_of_no_objects_are_skipped_and_too_many_packs_refused_before_any_is_read() {
        let (_dir, git) = craft();
        git.pack(&[PackEntry::Whole(Blob, b"one".to_vec())]);
        // Git never writes a pack of no objects, but an archive can hold any number of copies.
        let (pack, idx) = git.pack(&[]);
        for i in 0..3 {
            fs::copy(&pack, pack.with_file_name(format!("pack-copy{i}.pack"))).unwrap();
            fs::copy(&idx, idx.with_file_name(format!("pack-copy{i}.idx"))).unwrap();
        }
        let folder = pack.parent().unwrap();
        let loaded = load_at_most(folder, HashKind::Sha1, MAX_GIT_OBJECTS, 5).unwrap();
        assert_eq!((loaded.listed.len(), loaded.packs.len()), (5, 1));
        // One past the limit, refused even though a damaged index would be refused too.
        patch_index(&idx, |b| b[0] = 0);
        match load_at_most(folder, HashKind::Sha1, MAX_GIT_OBJECTS, 4).err() {
            Some(e @ GitError::TooManyPacks { .. }) => {
                let message = e.to_string();
                assert!(
                    message.contains("holds 5 packs, more than the 4"),
                    "{message}"
                );
                assert!(e.hint().unwrap().contains("git repack -a -d"), "{e:?}");
            }
            other => panic!("{other:?}"),
        }
        let loaded = load_at_most(folder, HashKind::Sha1, MAX_GIT_OBJECTS, 5);
        let message = broken(loaded.map(drop));
        assert!(message.contains("not a pack index"), "{message}");
    }
}
