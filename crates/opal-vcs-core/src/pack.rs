//! Pack files: the container every stored object lives in (docs/storage-format.md section 11).

use crate::chunks::{ChunkList, ReadListError};
use crate::encoding::{DecodeError, Reader, Writer};
use crate::fsio;
use crate::id::{ContentHasher, Flavor, Id, hash_content, hash_record};
use crate::safe_fs;
use crate::text::escape;
use opal_platform::dir::Dir;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::fs::{File, OpenOptions};
use std::io::{self, BufRead, BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
#[cfg(any(test, feature = "fault-injection"))]
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

mod v2;

pub const PACK_MAGIC: [u8; 8] = *b"OPALPACK";
pub const PACK_END_MAGIC: [u8; 8] = *b"OPALPEND";
pub const PACK_VERSION: u32 = 2;
/// Largest uncompressed object (1 TiB).
pub const MAX_OBJECT_SIZE: u64 = 1 << 40;
/// Largest uncompressed structured record (512 MiB).
pub const MAX_RECORD_SIZE: u64 = 512 << 20;
/// Most objects one pack may hold (storage-format.md section 11).
pub const MAX_PACK_OBJECTS: u64 = 1 << 24;
// While files are stored whole, a save's pack holds at most an object per entry plus its root
// listing, the save and its diary entry. Chunked files can need more (decision 028).
const _: () = assert!(crate::listing::MAX_SAVE_ENTRIES + 3 <= MAX_PACK_OBJECTS);
/// kind, id, uncompressed size, stored size, compression.
pub const ENTRY_HEADER_LEN: u64 = 50;

const PACK_HEADER_LEN: u64 = 16;
/// Index offset, index checksum, end magic.
const TRAILER_LEN: u64 = 48;
/// Smallest possible index line: id, kind, offset, two one-byte varints.
const MIN_INDEX_LINE: u64 = 43;
/// Longest possible index line: id, kind, offset, two ten-byte varints.
const MAX_INDEX_LINE: u64 = 61;
const ZSTD_LEVEL: i32 = 3;
/// Streams decide whether to compress from this much of their start.
const SAMPLE_LEN: u64 = 64 << 10;
/// The longest zstd frame header, which holds the optional content size.
const ZSTD_MAX_FRAME_HEADER: u64 = 18;
const NO_COMPRESSION: u8 = 0;
const ZSTD: u8 = 1;
/// An entry this small is read whole in one read rather than streamed.
const ONE_READ: u64 = 1 << 20;

/// What an object in a pack is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum ObjectKind {
    Content = 1,
    FolderListing = 16,
    Save = 17,
    ChunkList = 18,
    Conflict = 19,
    DiaryEntry = 20,
    ProjectLink = 21,
    TagList = 22,
    ServerLines = 23,
}

impl ObjectKind {
    #[must_use]
    pub fn from_u8(v: u8) -> Option<ObjectKind> {
        Some(match v {
            1 => ObjectKind::Content,
            16 => ObjectKind::FolderListing,
            17 => ObjectKind::Save,
            18 => ObjectKind::ChunkList,
            19 => ObjectKind::Conflict,
            20 => ObjectKind::DiaryEntry,
            21 => ObjectKind::ProjectLink,
            22 => ObjectKind::TagList,
            23 => ObjectKind::ServerLines,
            _ => return None,
        })
    }

    /// The fingerprint flavor of a structured record; `None` for content.
    #[must_use]
    pub fn flavor(self) -> Option<Flavor> {
        Some(match self {
            ObjectKind::Content => return None,
            ObjectKind::FolderListing => Flavor::FolderListing,
            ObjectKind::Save => Flavor::Save,
            ObjectKind::ChunkList => Flavor::ChunkList,
            ObjectKind::Conflict => Flavor::Conflict,
            ObjectKind::DiaryEntry => Flavor::DiaryEntry,
            ObjectKind::ProjectLink => Flavor::ProjectLink,
            ObjectKind::TagList => Flavor::TagList,
            ObjectKind::ServerLines => Flavor::ServerLines,
        })
    }

    /// Whether an entry of this kind may record these sizes. A chunk list records its file's size,
    /// and its own bytes are a record's.
    fn sizes_allowed(self, uncompressed: u64, stored: u64) -> bool {
        match self {
            ObjectKind::Content => uncompressed <= MAX_OBJECT_SIZE,
            ObjectKind::ChunkList => uncompressed <= MAX_OBJECT_SIZE && stored <= MAX_RECORD_SIZE,
            _ => uncompressed <= MAX_RECORD_SIZE,
        }
    }
}

/// One line of a pack's index.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct IndexEntry {
    pub id: Id,
    pub kind: ObjectKind,
    /// Where the entry's header starts.
    pub offset: u64,
    pub uncompressed_size: u64,
    pub stored_size: u64,
}

#[derive(Debug, thiserror::Error)]
pub enum PackError {
    #[error(transparent)]
    Io(#[from] io::Error),
    // The path has the project folder's name in it, which came from outside Opalro.
    #[error(
        "pack {} is damaged: {reason}",
        escape(&path.display().to_string(), false)
    )]
    Corrupt { path: PathBuf, reason: String },
    #[error("object {id} does not match its fingerprint")]
    FingerprintMismatch { id: Id },
    #[error("the file changed while it was being stored")]
    ContentChanged,
    #[error("the object is larger than Opalro can store")]
    TooLarge,
    // Not damage: the same pack reads fine with more memory.
    #[error("Opalro ran out of memory while reading the project's data")]
    OutOfMemory,
}

fn misuse(what: &'static str) -> PackError {
    PackError::Io(io::Error::new(io::ErrorKind::InvalidInput, what))
}

/// Compressed data is kept only if it is smaller than 95% of the original.
fn worth_compressing(compressed: usize, original: usize) -> bool {
    (compressed as u64) * 20 < (original as u64) * 19
}

/// An object's bytes as a pack stores them, made ready by `prepare`, perhaps on another thread.
pub struct Prepared {
    pub id: Id,
    /// The object's own size.
    pub size: u64,
    pub zstd: bool,
    pub stored: Vec<u8>,
}

/// The bytes of the object `id`, compressed if that saves enough.
pub fn prepare(id: Id, bytes: &[u8]) -> Result<Prepared, PackError> {
    let compressed = zstd::bulk::compress(bytes, ZSTD_LEVEL)?;
    let zstd = worth_compressing(compressed.len(), bytes.len());
    let stored = if zstd {
        compressed
    } else {
        // Gone first, so the bytes are held twice at most.
        drop(compressed);
        bytes.to_vec()
    };
    Ok(Prepared {
        id,
        size: bytes.len() as u64,
        zstd,
        stored,
    })
}

/// Builds a pack in the temp folder; `finish` moves it into the store.
pub struct PackWriter {
    file: BufWriter<File>,
    /// The temp folder, and the pack's name there.
    tmp: Arc<Dir>,
    name: OsString,
    /// Where the next entry starts.
    end: u64,
    index: BTreeMap<Id, IndexEntry>,
    /// Set when a failed entry could not be undone; the pack can then not be finished.
    broken: bool,
    early: fsio::EarlyFlush,
}

impl PackWriter {
    /// A pack built in the folder at `tmp_dir`.
    pub fn create(tmp_dir: &Path) -> Result<PackWriter, PackError> {
        PackWriter::create_in(&Arc::new(Dir::open_top(tmp_dir)?))
    }

    /// A pack built in the folder `tmp`.
    pub(crate) fn create_in(tmp: &Arc<Dir>) -> Result<PackWriter, PackError> {
        let (file, name) = fsio::create_temp(tmp)?;
        let mut w = PackWriter {
            file: BufWriter::new(file),
            tmp: Arc::clone(tmp),
            name,
            end: PACK_HEADER_LEN,
            index: BTreeMap::new(),
            broken: false,
            early: fsio::EarlyFlush::new(),
        };
        w.file.write_all(&PACK_MAGIC)?;
        w.file.write_all(&PACK_VERSION.to_le_bytes())?;
        w.file.write_all(&0u32.to_le_bytes())?;
        Ok(w)
    }

    /// Stores an encoded structured record: any kind but `Content` or a chunk list, which goes
    /// under its file's ID through `add_chunk_list`.
    pub fn add_record(&mut self, kind: ObjectKind, encoded: &[u8]) -> Result<Id, PackError> {
        if kind == ObjectKind::ChunkList {
            return Err(misuse("add_record cannot store a chunk list"));
        }
        let flavor = kind
            .flavor()
            .ok_or_else(|| misuse("add_record cannot store content"))?;
        if encoded.len() as u64 > MAX_RECORD_SIZE {
            return Err(PackError::TooLarge);
        }
        let id = hash_record(flavor, encoded);
        self.add_whole(kind, id, encoded)?;
        Ok(id)
    }

    /// Stores content held in memory.
    pub fn add_content(&mut self, bytes: &[u8]) -> Result<Id, PackError> {
        if bytes.len() as u64 > MAX_OBJECT_SIZE {
            return Err(PackError::TooLarge);
        }
        let id = hash_content(bytes);
        self.add_whole(ObjectKind::Content, id, bytes)?;
        Ok(id)
    }

    /// Streams content that should have the fingerprint `expected` and returns its size.
    /// If the bytes turn out different (the file changed), nothing is stored.
    pub fn add_content_from(
        &mut self,
        reader: &mut dyn Read,
        expected: Id,
    ) -> Result<u64, PackError> {
        if let Some(e) = self.index.get(&expected) {
            return Ok(e.uncompressed_size);
        }
        let entry = self.add_entry(ObjectKind::Content, expected, |file| {
            let mut sample = Vec::new();
            (&mut *reader).take(SAMPLE_LEN).read_to_end(&mut sample)?;
            let compress = worth_compressing(
                zstd::bulk::compress(&sample, ZSTD_LEVEL)?.len(),
                sample.len(),
            );
            let mut hasher = ContentHasher::new();
            let size = if compress {
                let mut enc = zstd::stream::write::Encoder::new(&mut *file, ZSTD_LEVEL)?;
                let size = pump(&sample, reader, &mut enc, &mut hasher)?;
                enc.finish()?;
                size
            } else {
                pump(&sample, reader, file, &mut hasher)?
            };
            Ok((
                size,
                if compress { ZSTD } else { NO_COMPRESSION },
                hasher.finish(),
            ))
        })?;
        Ok(entry.uncompressed_size)
    }

    /// Copies an object from `pack` with its stored bytes as they are. Check the copy first.
    pub(crate) fn add_stored(
        &mut self,
        pack: &PackReader,
        entry: &IndexEntry,
    ) -> Result<(), PackError> {
        if self.contains(&entry.id) {
            return Ok(());
        }
        self.add_entry(entry.kind, entry.id, |file| {
            let compression = pack.copy_stored(entry, file)?;
            Ok((entry.uncompressed_size, compression, entry.id))
        })?;
        Ok(())
    }

    /// Stores an object whose bytes `prepare` made ready.
    pub fn add_prepared(&mut self, kind: ObjectKind, p: &Prepared) -> Result<(), PackError> {
        if self.contains(&p.id) {
            return Ok(());
        }
        self.add_entry(kind, p.id, |file| {
            file.write_all(&p.stored)?;
            Ok((p.size, if p.zstd { ZSTD } else { NO_COMPRESSION }, p.id))
        })?;
        Ok(())
    }

    /// Stores `list` as a copy of its file: under the file's ID, recording the file's size, its
    /// own bytes as they are.
    pub fn add_chunk_list(&mut self, list: &ChunkList) -> Result<(), PackError> {
        if self.contains(&list.file) {
            return Ok(());
        }
        // Written as it is encoded, so a long list is never held twice.
        self.add_entry(ObjectKind::ChunkList, list.file, |file| {
            list.write_to(file)?;
            Ok((list.size, NO_COMPRESSION, list.file))
        })?;
        Ok(())
    }

    #[must_use]
    pub fn contains(&self, id: &Id) -> bool {
        self.index.contains_key(id)
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.index.is_empty()
    }

    /// Writes the index and trailer, flushes, and moves the pack into the folder at `packs_dir`,
    /// returning its path; see `finish_into`.
    pub fn finish(self, packs_dir: &Path) -> Result<Option<PathBuf>, PackError> {
        let packs = Dir::open_top(packs_dir)?;
        Ok(self.finish_into(&packs)?.map(|name| packs_dir.join(name)))
    }

    /// Writes the index and trailer, flushes, and moves the pack into the folder `packs` under the
    /// name `<hex of the index checksum>.pack`, which it returns. An empty pack is deleted instead.
    pub(crate) fn finish_into(mut self, packs: &Dir) -> Result<Option<OsString>, PackError> {
        if self.broken {
            return Err(io::Error::other("an earlier write to this pack failed").into());
        }
        if self.index.is_empty() {
            fsio::remove_file(&self.tmp, &self.name)?;
            return Ok(None);
        }
        let index = v2::encode(self.index.values());
        let checksum = blake3::hash(&index);
        self.file.write_all(&index)?;
        self.file.write_all(&self.end.to_le_bytes())?;
        self.file.write_all(checksum.as_bytes())?;
        self.file.write_all(&PACK_END_MAGIC)?;
        self.file.flush()?;
        self.early.finish()?;
        self.file.get_ref().sync_all()?;
        fsio::fault_point("pack:before-rename")?;
        let dest = OsString::from(format!("{}.pack", checksum.to_hex()));
        fsio::rename_durable(&self.tmp, &self.name, packs, &dest)?;
        fsio::fault_point("pack:after-rename")?;
        Ok(Some(dest))
    }

    fn add_whole(&mut self, kind: ObjectKind, id: Id, data: &[u8]) -> Result<(), PackError> {
        if self.contains(&id) {
            return Ok(());
        }
        self.add_prepared(kind, &prepare(id, data)?)
    }

    /// Writes one entry: a zeroed header, then the data from `body`, then the real header.
    /// `body` returns the uncompressed size, the compression and the data's fingerprint, which
    /// must be `id`. On any failure the entry is cut off again.
    fn add_entry(
        &mut self,
        kind: ObjectKind,
        id: Id,
        body: impl FnOnce(&mut BufWriter<File>) -> Result<(u64, u8, Id), PackError>,
    ) -> Result<IndexEntry, PackError> {
        if self.broken {
            return Err(io::Error::other("an earlier write to this pack failed").into());
        }
        if self.index.len() as u64 >= MAX_PACK_OBJECTS {
            return Err(misuse("a pack can't hold more than 2^24 objects"));
        }
        let start = self.end;
        let result = (|| {
            self.file.write_all(&[0; ENTRY_HEADER_LEN as usize])?;
            let (size, compression, actual) = body(&mut self.file)?;
            if actual != id {
                return Err(PackError::ContentChanged);
            }
            let data_end = self.file.stream_position()?;
            let entry = IndexEntry {
                id,
                kind,
                offset: start,
                uncompressed_size: size,
                stored_size: data_end - start - ENTRY_HEADER_LEN,
            };
            if !kind.sizes_allowed(size, entry.stored_size) {
                return Err(PackError::TooLarge);
            }
            let mut header = Writer::new();
            header.u8(kind as u8);
            header.id(&id);
            header.u64(entry.uncompressed_size);
            header.u64(entry.stored_size);
            header.u8(compression);
            self.file.seek(SeekFrom::Start(start))?;
            self.file.write_all(&header.into_bytes())?;
            self.file.seek(SeekFrom::Start(data_end))?;
            Ok(entry)
        })();
        match result {
            Ok(entry) => {
                self.end = entry.offset + ENTRY_HEADER_LEN + entry.stored_size;
                self.early
                    .wrote(self.file.get_ref(), ENTRY_HEADER_LEN + entry.stored_size);
                self.index.insert(id, entry);
                Ok(entry)
            }
            Err(e) => {
                if self.cut_off(start).is_err() {
                    self.broken = true;
                }
                Err(e)
            }
        }
    }

    fn cut_off(&mut self, start: u64) -> io::Result<()> {
        self.file.seek(SeekFrom::Start(start))?;
        self.file.get_ref().set_len(start)
    }
}

impl Drop for PackWriter {
    fn drop(&mut self) {
        // The helper's handle goes first, so nothing holds the file being removed.
        let _ = self.early.finish();
        // After a successful finish the temp file is gone, and this fails harmlessly.
        let _ = fsio::remove_file(&self.tmp, &self.name);
    }
}

/// What a version 2 pack of `objects` objects takes on disk besides their stored bytes: its
/// header, trailer and index, and each object's entry header.
pub(crate) fn v2_overhead(objects: u64) -> u64 {
    (PACK_HEADER_LEN + TRAILER_LEN)
        .saturating_add(v2::index_len(objects))
        .saturating_add(objects.saturating_mul(ENTRY_HEADER_LEN))
}

/// Most objects Opalro's writer puts in one pack. The format allows more, but a writer holds the
/// index of the pack it builds in memory, about 100 bytes an object.
pub const WRITER_PACK_OBJECTS: u64 = 1 << 20;
const _: () = assert!(WRITER_PACK_OBJECTS <= MAX_PACK_OBJECTS);

/// What `set_test_pack_limit` set; 0 for nothing.
#[cfg(any(test, feature = "fault-injection"))]
static TEST_PACK_LIMIT: AtomicU64 = AtomicU64::new(0);

/// Makes every set started from now on, in any thread, roll over at `limit` objects; `None`
/// goes back to the writer's own. It reaches every command of the process, so a test that uses
/// it needs a process of its own.
#[cfg(any(test, feature = "fault-injection"))]
pub fn set_test_pack_limit(limit: Option<u64>) {
    TEST_PACK_LIMIT.store(limit.map_or(0, |l| l.max(1)), Ordering::Relaxed);
}

fn writer_limit() -> u64 {
    #[cfg(any(test, feature = "fault-injection"))]
    {
        let set = TEST_PACK_LIMIT.load(Ordering::Relaxed);
        if set != 0 {
            return set;
        }
    }
    WRITER_PACK_OBJECTS
}

fn failed_earlier() -> PackError {
    io::Error::other("an earlier pack of this command could not be stored").into()
}

/// The packs of one command. A pack that reaches the limit is moved into the store at once and a
/// new one takes its place. Nothing points at these packs until the command moves the head, so
/// any a crash leaves behind are only unused.
pub struct PackSet {
    tmp: Arc<Dir>,
    /// The store's folder, and its path, which names the packs moved there.
    packs: Arc<Dir>,
    packs_dir: PathBuf,
    limit: u64,
    /// The pack being built.
    current: PackWriter,
    /// The packs already moved into the store, in the order they were made.
    done: Vec<PathBuf>,
    /// Set when a full pack could not be moved: the objects in it are gone, so the set takes no
    /// more.
    broken: bool,
}

impl PackSet {
    /// A set built in the folder `tmp` that moves its packs into `packs`, the folder at
    /// `packs_dir`, rolling over at the writer's own limit.
    pub(crate) fn new(
        tmp: &Arc<Dir>,
        packs: &Arc<Dir>,
        packs_dir: &Path,
    ) -> Result<PackSet, PackError> {
        PackSet::start(tmp, packs, packs_dir, writer_limit())
    }

    /// A set that rolls over at `limit` objects, so tests can split small commands.
    #[cfg(any(test, feature = "fault-injection"))]
    pub fn with_limit(tmp_dir: &Path, packs_dir: &Path, limit: u64) -> Result<PackSet, PackError> {
        let tmp = Arc::new(Dir::open_top(tmp_dir)?);
        let packs = Arc::new(Dir::open_top(packs_dir)?);
        PackSet::start(&tmp, &packs, packs_dir, limit)
    }

    fn start(
        tmp: &Arc<Dir>,
        packs: &Arc<Dir>,
        packs_dir: &Path,
        limit: u64,
    ) -> Result<PackSet, PackError> {
        Ok(PackSet {
            tmp: Arc::clone(tmp),
            packs: Arc::clone(packs),
            packs_dir: packs_dir.to_owned(),
            limit,
            current: PackWriter::create_in(tmp)?,
            done: Vec::new(),
            broken: false,
        })
    }

    /// Stores an encoded structured record (any kind but `Content` or a chunk list).
    pub fn add_record(&mut self, kind: ObjectKind, encoded: &[u8]) -> Result<Id, PackError> {
        self.add(|pack| pack.add_record(kind, encoded))
    }

    /// Stores content held in memory.
    pub fn add_content(&mut self, bytes: &[u8]) -> Result<Id, PackError> {
        self.add(|pack| pack.add_content(bytes))
    }

    /// Streams content that should have the fingerprint `expected` and returns its size. If the
    /// bytes turn out different, nothing is stored.
    pub fn add_content_from(
        &mut self,
        reader: &mut dyn Read,
        expected: Id,
    ) -> Result<u64, PackError> {
        self.add(|pack| pack.add_content_from(reader, expected))
    }

    /// Copies an object from `pack` with its stored bytes as they are. Check the copy first.
    pub(crate) fn add_stored(
        &mut self,
        pack: &PackReader,
        entry: &IndexEntry,
    ) -> Result<(), PackError> {
        self.add(|w| w.add_stored(pack, entry))
    }

    /// Stores an object whose bytes `prepare` made ready.
    pub fn add_prepared(&mut self, kind: ObjectKind, p: &Prepared) -> Result<(), PackError> {
        self.add(|pack| pack.add_prepared(kind, p))
    }

    /// Stores `list` as a copy of its file, under the file's ID.
    pub fn add_chunk_list(&mut self, list: &ChunkList) -> Result<(), PackError> {
        self.add(|pack| pack.add_chunk_list(list))
    }

    /// Whether the pack being built holds `id`. Packs already in the store aren't looked in.
    #[must_use]
    pub fn contains(&self, id: &Id) -> bool {
        self.current.contains(id)
    }

    /// The folder the set builds its packs in.
    pub(crate) fn tmp(&self) -> Arc<Dir> {
        Arc::clone(&self.tmp)
    }

    /// The kind of what the pack being built holds under `id`.
    pub(crate) fn kind_of(&self, id: &Id) -> Option<ObjectKind> {
        self.current.index.get(id).map(|e| e.kind)
    }

    /// Moves the pack being built into the store and starts the next, which holds no ID yet.
    pub(crate) fn start_next(&mut self) -> Result<(), PackError> {
        if self.broken {
            return Err(failed_earlier());
        }
        if let Err(e) = self.roll_over() {
            self.broken = true;
            return Err(e);
        }
        Ok(())
    }

    /// Whether nothing has been added, to any of the packs.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.done.is_empty() && self.current.is_empty()
    }

    /// Moves the pack being built into the store, if it holds anything, and returns the paths of
    /// all the packs in the order they were made.
    pub fn finish(mut self) -> Result<Vec<PathBuf>, PackError> {
        if self.broken {
            return Err(failed_earlier());
        }
        // An empty pack is only dropped, which removes its temp file.
        if !self.current.is_empty()
            && let Some(name) = self.current.finish_into(&self.packs)?
        {
            self.done.push(self.packs_dir.join(name));
        }
        Ok(self.done)
    }

    /// Runs `write` on the pack being built, then moves the pack into the store if it is full.
    fn add<T>(
        &mut self,
        write: impl FnOnce(&mut PackWriter) -> Result<T, PackError>,
    ) -> Result<T, PackError> {
        if self.broken {
            return Err(failed_earlier());
        }
        let added = write(&mut self.current)?;
        if self.current.index.len() as u64 >= self.limit
            && let Err(e) = self.roll_over()
        {
            self.broken = true;
            return Err(e);
        }
        Ok(added)
    }

    /// Moves the full pack into the store and starts the next one.
    fn roll_over(&mut self) -> Result<(), PackError> {
        // The next one first, so the set always has a pack to add to.
        let next = PackWriter::create_in(&self.tmp)?;
        let full = std::mem::replace(&mut self.current, next);
        if let Some(name) = full.finish_into(&self.packs)? {
            self.done.push(self.packs_dir.join(name));
        }
        fsio::fault_point("packs:rolled-over")?;
        Ok(())
    }
}

/// Copies `first` and then the rest of `reader` into `out`, hashing it, and returns the size.
fn pump(
    first: &[u8],
    reader: &mut dyn Read,
    out: &mut dyn Write,
    hasher: &mut ContentHasher,
) -> Result<u64, PackError> {
    hasher.update(first);
    out.write_all(first)?;
    let mut size = first.len() as u64;
    let mut buf = vec![0u8; SAMPLE_LEN as usize];
    loop {
        let n = match reader.read(&mut buf) {
            Ok(0) => return Ok(size),
            Ok(n) => n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e.into()),
        };
        size += n as u64;
        if size > MAX_OBJECT_SIZE {
            return Err(PackError::TooLarge);
        }
        hasher.update(&buf[..n]);
        out.write_all(&buf[..n])?;
    }
}

/// Reads objects from a finished pack, verifying everything it hands out.
pub struct PackReader {
    file: File,
    path: PathBuf,
    index: Index,
    window: Window,
    /// What counts this pack while it is open, for a store's tests.
    #[cfg(test)]
    counted: Option<Arc<open_files::Count>>,
}

enum Index {
    /// Version 1: read whole, and checked, when the pack is opened.
    Whole(Vec<IndexEntry>),
    /// Version 2: read by position as it is searched.
    OnDisk(Box<v2::Index>),
}

/// A pack's index lines in order of ID; see `PackReader::entries`.
pub struct Entries<'a>(Lines<'a>);

enum Lines<'a> {
    Whole(std::slice::Iter<'a, IndexEntry>),
    OnDisk(v2::Lines<'a>),
}

impl Iterator for Entries<'_> {
    type Item = Result<IndexEntry, PackError>;

    fn next(&mut self) -> Option<Self::Item> {
        match &mut self.0 {
            Lines::Whole(lines) => lines.next().map(|e| Ok(*e)),
            Lines::OnDisk(lines) => lines.next(),
        }
    }
}

impl PackReader {
    /// Opens a pack and checks its header and trailer. A version 1 index is read and checked
    /// whole: its checksum, and that every entry lies inside the file without overlapping another.
    /// A version 2 index is read only as far as its fan-out; see `find` and `entries`.
    pub fn open(path: &Path) -> Result<PackReader, PackError> {
        PackReader::open_with(path, MAX_PACK_OBJECTS)
    }

    /// `open` for the pack `name` in the folder `dir`, at `path`, opened through `dir`.
    pub(crate) fn open_in(dir: &Dir, name: &OsStr, path: &Path) -> Result<PackReader, PackError> {
        let opened = safe_fs::open_regular_in(dir, name, path);
        PackReader::checked(opened, path, MAX_PACK_OBJECTS)
    }

    /// `open`, with a smaller limit on objects than the format's; tests use one.
    fn open_with(path: &Path, max_objects: u64) -> Result<PackReader, PackError> {
        let opened = safe_fs::open_regular(path, OpenOptions::new().read(true));
        PackReader::checked(opened, path, max_objects)
    }

    /// The pack `opened` at `path`, once it passes `open`'s checks.
    fn checked(
        opened: io::Result<File>,
        path: &Path,
        max_objects: u64,
    ) -> Result<PackReader, PackError> {
        let corrupt = |reason: String| PackError::Corrupt {
            path: path.to_path_buf(),
            reason,
        };
        // A link, pipe or device named like a pack is damage, never followed or waited on.
        let file = match opened {
            Err(e) if e.kind() == io::ErrorKind::InvalidInput => {
                return Err(corrupt("it is not a regular file".into()));
            }
            opened => opened?,
        };
        let len = file.metadata()?.len();
        if len < PACK_HEADER_LEN + TRAILER_LEN {
            return Err(corrupt("the file is too short to be a pack".into()));
        }

        let header = read_vec(&file, None, 0, PACK_HEADER_LEN)?;
        if sub::<8>(&header, 0) != PACK_MAGIC {
            return Err(corrupt("the file does not start with OPALPACK".into()));
        }
        let version = u32::from_le_bytes(sub(&header, 8));
        if !matches!(version, 1 | 2) || u32::from_le_bytes(sub(&header, 12)) != 0 {
            return Err(corrupt("unknown pack version or flags".into()));
        }

        let trailer = read_vec(&file, None, len - TRAILER_LEN, TRAILER_LEN)?;
        let (index_offset, checksum, end_magic) = (
            u64::from_le_bytes(sub(&trailer, 0)),
            sub::<32>(&trailer, 8),
            sub::<8>(&trailer, 40),
        );
        if end_magic != PACK_END_MAGIC {
            return Err(corrupt("the file does not end with OPALPEND".into()));
        }
        if index_offset < PACK_HEADER_LEN || index_offset > len - TRAILER_LEN {
            return Err(corrupt("the index offset is outside the file".into()));
        }
        let index_end = len - TRAILER_LEN;
        let index = if version == 1 {
            let index = Section {
                file: &file,
                window: None,
                pos: index_offset,
                end: index_end,
            };
            Index::Whole(read_index(path, index, max_objects, checksum)?)
        } else {
            let range = (index_offset, index_end);
            Index::OnDisk(Box::new(v2::Index::open(
                &file,
                path,
                range,
                checksum,
                max_objects,
            )?))
        };
        #[cfg(test)]
        open_files::opened();
        Ok(PackReader {
            file,
            path: path.to_path_buf(),
            index,
            window: Window::new(len, index_offset),
            #[cfg(test)]
            counted: None,
        })
    }

    /// Counts this pack in `count` until it is closed, whichever thread closes it.
    #[cfg(test)]
    pub(crate) fn count_in(&mut self, count: &Arc<open_files::Count>) {
        count.opened();
        self.counted = Some(Arc::clone(count));
    }

    /// Every index line, sorted by id. A version 2 pack's are read from disk as they are handed
    /// out, and checked; the first that fails ends them.
    #[must_use]
    pub fn entries(&self) -> Entries<'_> {
        match &self.index {
            Index::Whole(entries) => Entries(Lines::Whole(entries.iter())),
            Index::OnDisk(index) => Entries(Lines::OnDisk(index.lines(&self.file, &self.path))),
        }
    }

    /// Checks a version 2 index against its checksum, which opening it doesn't; a version 1
    /// index was checked when it was read.
    pub fn check_index(&self) -> Result<(), PackError> {
        match &self.index {
            Index::Whole(_) => Ok(()),
            Index::OnDisk(index) => index.check_checksum(&self.file, &self.path),
        }
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The pack's version: 1 or 2.
    #[must_use]
    pub fn version(&self) -> u32 {
        match self.index {
            Index::Whole(_) => 1,
            Index::OnDisk(_) => 2,
        }
    }

    /// The index line of `id`, if the pack holds it.
    pub fn find(&self, id: &Id) -> Result<Option<IndexEntry>, PackError> {
        match &self.index {
            Index::Whole(entries) => Ok(entries
                .binary_search_by(|e| e.id.cmp(id))
                .ok()
                .and_then(|i| entries.get(i).copied())),
            Index::OnDisk(index) => index.find(&self.file, Some(&self.window), &self.path, id),
        }
    }

    /// Reads a structured record and checks its fingerprint.
    pub fn read_record(&self, entry: &IndexEntry) -> Result<Vec<u8>, PackError> {
        // Stored under its file's ID, a chunk list has no fingerprint of its own.
        if entry.kind == ObjectKind::ChunkList {
            return Err(misuse("read_record cannot read a chunk list"));
        }
        let flavor = entry
            .kind
            .flavor()
            .ok_or_else(|| misuse("read_record cannot read content"))?;
        let out = match self.read_small(entry)? {
            Some(out) => out,
            None => {
                let mut out = Vec::new();
                out.try_reserve_exact(
                    usize::try_from(entry.uncompressed_size).map_err(|_| PackError::TooLarge)?,
                )
                .map_err(|_| PackError::OutOfMemory)?;
                self.decode_into(entry, &mut out)?;
                out
            }
        };
        if hash_record(flavor, &out) != entry.id {
            return Err(PackError::FingerprintMismatch { id: entry.id });
        }
        Ok(out)
    }

    /// Streams content into `out`. The fingerprint is checked at the end, so the output must be
    /// treated as untrusted until this returns `Ok`.
    pub fn copy_content(&self, entry: &IndexEntry, out: &mut dyn Write) -> Result<(), PackError> {
        if entry.kind != ObjectKind::Content {
            return Err(misuse("copy_content can only read content"));
        }
        // Only content small once uncompressed too is held whole.
        if entry.uncompressed_size <= ONE_READ
            && let Some(bytes) = self.read_small(entry)?
        {
            if hash_content(&bytes) != entry.id {
                return Err(PackError::FingerprintMismatch { id: entry.id });
            }
            out.write_all(&bytes)?;
            return Ok(());
        }
        let mut hashing = HashingWriter {
            inner: out,
            hasher: ContentHasher::new(),
        };
        self.decode_into(entry, &mut hashing)?;
        if hashing.hasher.finish() != entry.id {
            return Err(PackError::FingerprintMismatch { id: entry.id });
        }
        Ok(())
    }

    /// Content read whole and checked, for an entry small enough to hold.
    pub(crate) fn read_content(&self, entry: &IndexEntry) -> Result<Vec<u8>, PackError> {
        if entry.kind == ObjectKind::Content
            && entry.uncompressed_size <= ONE_READ
            && let Some(bytes) = self.read_small(entry)?
        {
            if hash_content(&bytes) != entry.id {
                return Err(PackError::FingerprintMismatch { id: entry.id });
            }
            return Ok(bytes);
        }
        let mut out = Vec::new();
        out.try_reserve_exact(
            usize::try_from(entry.uncompressed_size).map_err(|_| PackError::TooLarge)?,
        )
        .map_err(|_| PackError::OutOfMemory)?;
        self.copy_content(entry, &mut out)?;
        Ok(out)
    }

    /// Reads a chunk list and checks it by its structure: stored as it is, decoding by
    /// storage-format.md section 6's rules, and naming its entry's file and size. Neither its
    /// chunks nor the file they build are checked here.
    pub fn read_chunk_list(&self, entry: &IndexEntry) -> Result<ChunkList, PackError> {
        self.read_chunk_list_hashed(entry).map(|(list, _)| list)
    }

    /// Whether the chunk list `entry` holds is `list`, byte for byte, compared as it is read so a
    /// long list is never held twice. Any other, damaged or not, isn't.
    pub(crate) fn holds_chunk_list(
        &self,
        entry: &IndexEntry,
        list: &ChunkList,
    ) -> Result<bool, PackError> {
        let (compression, mut data) = self.stored(entry)?;
        Ok(entry.kind == ObjectKind::ChunkList
            && compression == NO_COMPRESSION
            && list.is_record(&mut data, entry.stored_size)?)
    }

    /// `read_chunk_list`, with the plain BLAKE3 hash of the list's bytes, which tells lists apart.
    pub(crate) fn read_chunk_list_hashed(
        &self,
        entry: &IndexEntry,
    ) -> Result<(ChunkList, Id), PackError> {
        let id = entry.id;
        if entry.kind != ObjectKind::ChunkList {
            return Err(misuse("read_chunk_list can only read a chunk list"));
        }
        let (compression, data) = self.stored(entry)?;
        if compression != NO_COMPRESSION {
            return Err(self.corrupt(format!("the chunk list of {id} is compressed")));
        }
        let mut hashing = HashingReader {
            inner: data,
            hasher: blake3::Hasher::new(),
        };
        // Decoded as it is read, so the list's bytes are never held as well.
        let list = ChunkList::read_from(&mut hashing, entry.stored_size).map_err(|e| match e {
            ReadListError::Damaged(why) => {
                self.corrupt(format!("the chunk list of {id} is invalid: {why}"))
            }
            ReadListError::Io(e) => PackError::Io(e),
            ReadListError::OutOfMemory => PackError::OutOfMemory,
        })?;
        if list.file != id || list.size != entry.uncompressed_size {
            return Err(self.corrupt(format!("the chunk list of {id} names another file or size")));
        }
        Ok((list, Id(*hashing.hasher.finalize().as_bytes())))
    }

    fn corrupt(&self, reason: impl Into<String>) -> PackError {
        PackError::Corrupt {
            path: self.path.clone(),
            reason: reason.into(),
        }
    }

    /// Copies the entry's stored bytes to `out` as they are, after checking its header, and
    /// returns their compression. Nothing is checked against the ID: read the object first.
    pub(crate) fn copy_stored(
        &self,
        entry: &IndexEntry,
        out: &mut dyn Write,
    ) -> Result<u8, PackError> {
        let (compression, mut data) = self.stored(entry)?;
        if io::copy(&mut data, out)? != entry.stored_size {
            return Err(self.corrupt(format!("{} runs past the end of the file", entry.id)));
        }
        Ok(compression)
    }

    /// Checks the entry's header against `entry`, and returns its compression and stored bytes.
    fn stored(&self, entry: &IndexEntry) -> Result<(u8, Section<'_>), PackError> {
        let header = read_vec(
            &self.file,
            Some(&self.window),
            entry.offset,
            ENTRY_HEADER_LEN,
        )?;
        let compression = self.check_header(entry, &header)?;
        let data_start = entry.offset + ENTRY_HEADER_LEN;
        let data = Section {
            file: &self.file,
            window: Some(&self.window),
            pos: data_start,
            end: data_start
                .checked_add(entry.stored_size)
                .ok_or_else(|| self.corrupt("an entry runs past the end of the file"))?,
        };
        Ok((compression, data))
    }

    /// Checks an entry's header against `entry`, and returns its compression.
    fn check_header(&self, entry: &IndexEntry, header: &[u8]) -> Result<u8, PackError> {
        let mut r = Reader::new(header);
        let fields = (r.u8(), r.id(), r.u64(), r.u64(), r.u8());
        let (Ok(kind), Ok(id), Ok(size), Ok(stored), Ok(compression)) = fields else {
            return Err(self.corrupt("an entry header is cut short"));
        };
        if kind != entry.kind as u8
            || id != entry.id
            || size != entry.uncompressed_size
            || stored != entry.stored_size
        {
            return Err(self.corrupt(format!("the header of {id} does not match the index")));
        }
        Ok(compression)
    }

    /// A small entry's bytes, from one read of its header and stored bytes and one call to zstd
    /// into room for exactly its size. `None` when anything is off, or it isn't small: the
    /// streamed path then reads it again and tells what.
    fn read_small(&self, entry: &IndexEntry) -> Result<Option<Vec<u8>>, PackError> {
        if entry.stored_size > ONE_READ {
            return Ok(None);
        }
        let mut whole = Vec::new();
        let len = ENTRY_HEADER_LEN + entry.stored_size;
        whole
            .try_reserve_exact(len as usize)
            .map_err(|_| PackError::OutOfMemory)?;
        whole.resize(len as usize, 0);
        let mut section = Section {
            file: &self.file,
            window: Some(&self.window),
            pos: entry.offset,
            end: entry.offset.saturating_add(len),
        };
        if section.read_exact(&mut whole).is_err() {
            return Ok(None);
        }
        let (header, data) = whole.split_at(ENTRY_HEADER_LEN as usize);
        let Ok(compression) = self.check_header(entry, header) else {
            return Ok(None);
        };
        let size = entry.uncompressed_size;
        match compression {
            NO_COMPRESSION if entry.stored_size == size => {
                whole.drain(..ENTRY_HEADER_LEN as usize);
                Ok(Some(whole))
            }
            ZSTD => {
                if !one_plain_frame(data, size) {
                    return Ok(None);
                }
                let Ok(size) = usize::try_from(size) else {
                    return Ok(None);
                };
                let mut out = Vec::new();
                out.try_reserve_exact(size)
                    .map_err(|_| PackError::OutOfMemory)?;
                let decoded = DECOMPRESSOR
                    .try_with(|d| -> Result<bool, PackError> {
                        let Ok(mut d) = d.try_borrow_mut() else {
                            return Ok(false);
                        };
                        if d.is_none() {
                            // Making one fails only when memory runs out.
                            let made = zstd::bulk::Decompressor::new()
                                .map_err(|_| PackError::OutOfMemory)?;
                            *d = Some(made);
                        }
                        Ok(d.as_mut()
                            .is_some_and(|d| d.decompress_to_buffer(data, &mut out).is_ok()))
                    })
                    .unwrap_or(Ok(false))?;
                Ok((decoded && out.len() == size).then_some(out))
            }
            _ => Ok(None),
        }
    }

    /// Checks the entry's header against `entry`, then writes its uncompressed bytes to `out`,
    /// never producing more than one byte beyond the recorded size.
    fn decode_into(&self, entry: &IndexEntry, out: &mut dyn Write) -> Result<(), PackError> {
        let (compression, mut data) = self.stored(entry)?;
        let (id, size, stored) = (entry.id, entry.uncompressed_size, entry.stored_size);
        let data_start = data.pos;
        let produced = match compression {
            NO_COMPRESSION if stored == size => io::copy(&mut data, out)?,
            ZSTD => {
                let frame_header = read_vec(
                    &self.file,
                    Some(&self.window),
                    data_start,
                    stored.min(ZSTD_MAX_FRAME_HEADER),
                )?;
                let start = match frame_start(&frame_header, size) {
                    Ok(start) => start,
                    Err(FrameFault::NotAllowed) => {
                        return Err(self.corrupt(format!(
                            "the zstd data of {id} is not a frame format 1 allows"
                        )));
                    }
                    Err(FrameFault::WrongSize) => {
                        return Err(
                            self.corrupt(format!("the zstd frame of {id} has the wrong size"))
                        );
                    }
                };
                let walked = Walked {
                    inner: data,
                    walk: BlockWalk::new(start),
                };
                // Making a decoder fails only when memory runs out.
                let decoder = zstd::stream::read::Decoder::new(walked)
                    .map_err(|_| PackError::OutOfMemory)?
                    .single_frame();
                let mut decoded = decoder.take(size.saturating_add(1));
                let produced = copy_decoded(&mut decoded, out).map_err(|e| {
                    if zstd_ran_out_of_memory(&e) {
                        PackError::OutOfMemory
                    } else {
                        self.corrupt(format!("the zstd data of {id} is invalid: {e}"))
                    }
                })??;
                let mut rest = decoded.into_inner().into_inner();
                if produced == size && !rest.fill_buf()?.is_empty() {
                    return Err(
                        self.corrupt(format!("the zstd data of {id} goes on after its frame"))
                    );
                }
                produced
            }
            _ => return Err(self.corrupt(format!("{id} has an unknown compression"))),
        };
        if produced != size {
            return Err(self.corrupt(format!("{id} does not have its recorded size")));
        }
        Ok(())
    }
}

/// How many packs this thread has opened, how many it held open at once, and how many reads it
/// made of them: tests of the store's limit on open files and of reading read them.
#[cfg(test)]
pub(crate) mod open_files {
    use std::cell::Cell;
    use std::sync::atomic::{AtomicUsize, Ordering};

    thread_local! {
        static OPENED: Cell<usize> = const { Cell::new(0) };
        static OPEN: Cell<usize> = const { Cell::new(0) };
        static MOST: Cell<usize> = const { Cell::new(0) };
        static READS: Cell<usize> = const { Cell::new(0) };
    }

    pub(super) fn read() {
        READS.set(READS.get() + 1);
    }

    /// How many reads of a pack file this thread has made.
    pub(crate) fn reads() -> usize {
        READS.get()
    }

    pub(super) fn opened() {
        OPENED.set(OPENED.get() + 1);
        OPEN.set(OPEN.get() + 1);
        MOST.set(MOST.get().max(OPEN.get()));
    }

    pub(super) fn closed() {
        OPEN.set(OPEN.get().saturating_sub(1));
    }

    pub(crate) fn total() -> usize {
        OPENED.get()
    }

    /// The most held open at once since the last call.
    pub(crate) fn most_since_last() -> usize {
        MOST.replace(OPEN.get())
    }

    /// Packs counted together while they are open, whichever threads open and close them.
    #[derive(Default)]
    pub(crate) struct Count {
        now: AtomicUsize,
        most: AtomicUsize,
    }

    impl Count {
        pub(super) fn opened(&self) {
            let now = self.now.fetch_add(1, Ordering::SeqCst) + 1;
            self.most.fetch_max(now, Ordering::SeqCst);
        }

        pub(super) fn closed(&self) {
            self.now.fetch_sub(1, Ordering::SeqCst);
        }

        /// The most open at once so far.
        pub(crate) fn most(&self) -> usize {
            self.most.load(Ordering::SeqCst)
        }
    }
}

#[cfg(test)]
impl Drop for PackReader {
    fn drop(&mut self) {
        open_files::closed();
        if let Some(count) = &self.counted {
            count.closed();
        }
    }
}

thread_local! {
    // Kept, so reading many small entries doesn't make a decompressor for each.
    static DECOMPRESSOR: RefCell<Option<zstd::bulk::Decompressor<'static>>> =
        const { RefCell::new(None) };
}

/// Whether `data` is one zstd frame of `size` bytes as format 1 allows it, and nothing more, as
/// far as can be told without decoding it. Only these go to the one-call decoder.
fn one_plain_frame(data: &[u8], size: u64) -> bool {
    frame_start(data, size).is_ok_and(|start| BlockWalk::new(start).feed(data))
        && zstd::zstd_safe::find_frame_compressed_size(data) == Ok(data.len())
}

/// Why the start of an entry's zstd data breaks format 1's rules.
#[derive(Debug, PartialEq)]
enum FrameFault {
    NotAllowed,
    WrongSize,
}

/// The start of a frame format 1 allows: how long its header is, and the most a block may hold.
#[derive(Clone, Copy, Debug)]
struct FrameStart {
    header_len: u64,
    block_limit: u64,
}

/// Checks `header`, the first bytes of an entry's stored data, as the start of a zstd frame of
/// `size` bytes: the frame's magic (so no skippable frame and no format from before zstd 1.0),
/// no dictionary ID, the header's spare bits clear, a window of at most 2^27 bytes, and a
/// content size of `size` if the header has one.
fn frame_start(header: &[u8], size: u64) -> Result<FrameStart, FrameFault> {
    const MAGIC: [u8; 4] = [0x28, 0xB5, 0x2F, 0xFD];
    const MAX_WINDOW: u64 = 1 << 27;
    const MAX_BLOCK: u64 = 128 << 10;
    // Bits 0 and 1 give a dictionary ID's size, bit 3 is reserved and bit 4 unused.
    const NOT_IN_FORMAT_1: u8 = 0x1B;
    let Some(&descriptor) = header.get(4) else {
        return Err(FrameFault::NotAllowed);
    };
    if !header.starts_with(&MAGIC) || descriptor & NOT_IN_FORMAT_1 != 0 {
        return Err(FrameFault::NotAllowed);
    }
    let single_segment = descriptor & 0x20 != 0;
    let stated =
        zstd::zstd_safe::get_frame_content_size(header).map_err(|_| FrameFault::NotAllowed)?;
    // zstd takes a content size of all ones for none, so the flags say whether there is one.
    if descriptor & 0xE0 != 0 && stated != Some(size) {
        return Err(FrameFault::WrongSize);
    }
    let window = if single_segment {
        // A single segment's window is its content size.
        stated
    } else {
        header.get(5).and_then(|&b| {
            let base = 1u64.checked_shl(10 + u32::from(b >> 3))?;
            (base >> 3).checked_mul(u64::from(b & 7))?.checked_add(base)
        })
    };
    let window = window
        .filter(|&w| w <= MAX_WINDOW)
        .ok_or(FrameFault::NotAllowed)?;
    let size_field = match descriptor >> 6 {
        0 => u64::from(single_segment),
        1 => 2,
        2 => 4,
        _ => 8,
    };
    Ok(FrameStart {
        header_len: 5 + u64::from(!single_segment) + size_field,
        block_limit: window.min(MAX_BLOCK),
    })
}

/// Follows a zstd frame's blocks as its bytes go by, and refuses a block over the frame's limit
/// (its window, at most 128 KiB). zstd's streamed decoder checks that, but its one-call decoder
/// doesn't for raw and RLE blocks, so both read paths check here first.
struct BlockWalk {
    limit: u64,
    /// Bytes still to pass: the frame header's, then the body of each block.
    skip: u64,
    header: [u8; 3],
    have: usize,
    last_seen: bool,
}

impl BlockWalk {
    fn new(start: FrameStart) -> Self {
        BlockWalk {
            limit: start.block_limit,
            skip: start.header_len,
            header: [0; 3],
            have: 0,
            last_seen: false,
        }
    }

    /// Takes the frame's next bytes; false once a block holds more than the limit.
    fn feed(&mut self, mut bytes: &[u8]) -> bool {
        while !self.last_seen && !bytes.is_empty() {
            if self.skip > 0 {
                let n = bytes
                    .len()
                    .min(usize::try_from(self.skip).unwrap_or(usize::MAX));
                self.skip -= n as u64;
                bytes = bytes.get(n..).unwrap_or_default();
                continue;
            }
            let Some((&b, rest)) = bytes.split_first() else {
                break;
            };
            bytes = rest;
            if let Some(slot) = self.header.get_mut(self.have) {
                *slot = b;
            }
            self.have += 1;
            if self.have == 3 {
                self.have = 0;
                let [a, b, c] = self.header;
                let header = u32::from_le_bytes([a, b, c, 0]);
                // An RLE block's size is how many bytes it makes; it stores one.
                let (size, rle) = (u64::from(header >> 3), (header >> 1) & 3 == 1);
                if size > self.limit {
                    return false;
                }
                self.skip = if rle { 1 } else { size };
                self.last_seen = header & 1 == 1;
            }
        }
        true
    }
}

/// An entry's stored bytes on their way to the streamed decoder, checked block by block.
struct Walked<R> {
    inner: R,
    walk: BlockWalk,
}

impl<R: Read> Read for Walked<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.read(buf)?;
        if !self.walk.feed(buf.get(..n).unwrap_or_default()) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "a block holds more than its frame allows",
            ));
        }
        Ok(n)
    }
}

/// zstd tells its errors apart only by their text.
fn zstd_ran_out_of_memory(e: &io::Error) -> bool {
    let code = zstd::zstd_safe::zstd_sys::ZSTD_ErrorCode::ZSTD_error_memory_allocation as usize;
    e.to_string() == zstd::zstd_safe::get_error_name(code.wrapping_neg())
}

/// Like `io::copy`, but read errors (bad compressed data) and write errors are kept apart:
/// the outer error is a read error, the inner one a write error.
fn copy_decoded(
    src: &mut dyn Read,
    out: &mut dyn Write,
) -> Result<Result<u64, PackError>, io::Error> {
    let mut buf = vec![0u8; SAMPLE_LEN as usize];
    let mut total = 0u64;
    loop {
        let n = match src.read(&mut buf) {
            Ok(0) => return Ok(Ok(total)),
            Ok(n) => n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        };
        if let Err(e) = out.write_all(&buf[..n]) {
            return Ok(Err(e.into()));
        }
        total += n as u64;
    }
}

/// Reads and checks the index (it starts where the entry area ends) a buffer at a time, keeping
/// only lines that passed, so a huge or sparse index costs little memory.
fn read_index(
    path: &Path,
    index: Section,
    max_objects: u64,
    checksum: [u8; 32],
) -> Result<Vec<IndexEntry>, PackError> {
    let (entry_area_end, len) = (index.pos, index.end - index.pos);
    let hashing = HashingReader {
        inner: index,
        hasher: blake3::Hasher::new(),
    };
    let mut r = IndexReader {
        src: BufReader::with_capacity(SAMPLE_LEN as usize, hashing),
        read: 0,
        path,
    };
    let count = r.varint()?;
    // The cheap checks first: nothing more is read of an index that can't be right.
    if count > max_objects {
        return Err(r.corrupt(format!("it lists {count} objects, more than {max_objects}")));
    }
    let lines = len - r.read;
    if lines < count.saturating_mul(MIN_INDEX_LINE) || lines > count.saturating_mul(MAX_INDEX_LINE)
    {
        return Err(r.corrupt(format!(
            "its {lines} bytes don't fit its entry count of {count}"
        )));
    }
    let mut entries: Vec<IndexEntry> = Vec::new();
    for _ in 0..count {
        let id = Id(r.bytes()?);
        let [kind] = r.bytes()?;
        let kind = ObjectKind::from_u8(kind)
            .ok_or_else(|| r.corrupt(format!("unknown object kind {kind}")))?;
        let offset = u64::from_le_bytes(r.bytes()?);
        let uncompressed_size = r.varint()?;
        let stored_size = r.varint()?;
        if entries.last().is_some_and(|prev| prev.id >= id) {
            return Err(r.corrupt("entries are not sorted by id, or repeat one"));
        }
        if !kind.sizes_allowed(uncompressed_size, stored_size) {
            return Err(r.corrupt(format!("{id} is larger than allowed")));
        }
        let end = offset
            .checked_add(ENTRY_HEADER_LEN)
            .and_then(|o| o.checked_add(stored_size));
        if offset < PACK_HEADER_LEN || end.is_none_or(|end| end > entry_area_end) {
            return Err(r.corrupt(format!("{id} lies outside the entry area")));
        }
        entries.push(IndexEntry {
            id,
            kind,
            offset,
            uncompressed_size,
            stored_size,
        });
    }
    if r.read != len {
        return Err(r.corrupt(DecodeError::TrailingBytes.to_string()));
    }
    if *r.src.get_ref().hasher.finalize().as_bytes() != checksum {
        return Err(PackError::Corrupt {
            path: path.to_path_buf(),
            reason: "the index checksum does not match".into(),
        });
    }

    let mut by_offset: Vec<&IndexEntry> = entries.iter().collect();
    by_offset.sort_by_key(|e| e.offset);
    // Offsets and sizes were range-checked above, so these sums cannot overflow.
    if by_offset
        .windows(2)
        .any(|p| p[0].offset + ENTRY_HEADER_LEN + p[0].stored_size > p[1].offset)
    {
        return Err(r.corrupt("two entries overlap"));
    }
    Ok(entries)
}

/// Hands out an index's bytes a buffer at a time, counting them; everything read is hashed.
struct IndexReader<'a> {
    src: BufReader<HashingReader<Section<'a>>>,
    /// Bytes handed out so far.
    read: u64,
    path: &'a Path,
}

impl IndexReader<'_> {
    fn corrupt(&self, reason: impl Into<String>) -> PackError {
        PackError::Corrupt {
            path: self.path.to_path_buf(),
            reason: format!("index: {}", reason.into()),
        }
    }

    fn bytes<const N: usize>(&mut self) -> Result<[u8; N], PackError> {
        let mut out = [0u8; N];
        match self.src.read_exact(&mut out) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => {
                return Err(self.corrupt(DecodeError::UnexpectedEnd.to_string()));
            }
            Err(e) => return Err(e.into()),
        }
        self.read += N as u64;
        Ok(out)
    }

    /// A varint, read a byte at a time and decoded by the format's strict rules.
    fn varint(&mut self) -> Result<u64, PackError> {
        let mut bytes = [0u8; 10];
        let mut n = 0;
        while n < bytes.len() {
            let [b] = self.bytes()?;
            bytes[n] = b;
            n += 1;
            if b & 0x80 == 0 {
                break;
            }
        }
        Reader::new(&bytes[..n])
            .varint()
            .map_err(|e| self.corrupt(e.to_string()))
    }
}

/// Passes reads through while hashing them.
struct HashingReader<R> {
    inner: R,
    hasher: blake3::Hasher,
}

impl<R: Read> Read for HashingReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.hasher.update(&buf[..n]);
        Ok(n)
    }
}

/// `N` bytes of `bytes` starting at `at`; the caller guarantees the range.
fn sub<const N: usize>(bytes: &[u8], at: usize) -> [u8; N] {
    let mut out = [0u8; N];
    out.copy_from_slice(&bytes[at..at + N]);
    out
}

/// Reads `len` bytes at `offset`, refusing allocations the system can't give.
fn read_vec(
    file: &File,
    window: Option<&Window>,
    offset: u64,
    len: u64,
) -> Result<Vec<u8>, PackError> {
    let len = usize::try_from(len).map_err(|_| PackError::TooLarge)?;
    let mut out = Vec::new();
    out.try_reserve_exact(len)
        .map_err(|_| PackError::TooLarge)?;
    out.resize(len, 0);
    Section {
        file,
        window,
        pos: offset,
        end: offset.saturating_add(len as u64),
    }
    .read_exact(&mut out)?;
    Ok(out)
}

/// A byte range of a file, read with positional reads so readers never share a cursor. Every
/// read of a pack goes through here.
struct Section<'a> {
    file: &'a File,
    /// Answers small reads, once the pack is open.
    window: Option<&'a Window>,
    pos: u64,
    end: u64,
}

impl Read for Section<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let left = usize::try_from(self.end.saturating_sub(self.pos)).unwrap_or(usize::MAX);
        let want = buf.len().min(left);
        if want == 0 {
            return Ok(0);
        }
        let buf = &mut buf[..want];
        if self
            .window
            .is_some_and(|w| w.read(self.file, self.pos, buf))
        {
            self.pos += want as u64;
            return Ok(want);
        }
        #[cfg(test)]
        open_files::read();
        let n = safe_fs::read_at(self.file, buf, self.pos)?;
        self.pos += n as u64;
        Ok(n)
    }
}

/// What a read of the file that misses the window fills.
const BLOCK: u64 = 65_536;
/// The largest read answered from a block; bigger ones go to the file.
const SMALL_READ: u64 = 4_096;
/// The most a slot may read as blocks for each byte asked of it.
const BLOCK_BYTES_PER_BYTE_ASKED: u64 = 4;

/// The last block read of a pack's entries, and of its index, so the many small reads of records
/// and index lines that sit side by side cost one read of the file. A block is read only at the
/// second read in a row that falls in it, and only if the blocks a slot has read stay within four
/// times the bytes asked of it, so reads scattered over the pack, or in pairs, cost small reads.
/// A pack never changes once it is named, so a block is only ever a copy of what the file holds.
struct Window {
    /// The file's length when it was opened.
    len: u64,
    index_start: u64,
    /// The entries' block and the index's.
    blocks: [Mutex<Block>; 2],
}

#[derive(Default)]
struct Block {
    start: u64,
    bytes: Vec<u8>,
    /// Where the block of the last read that missed starts.
    missed: Option<u64>,
    /// The bytes of the small reads asked of this slot.
    asked: u64,
    /// The bytes read as blocks.
    read: u64,
}

impl Window {
    fn new(len: u64, index_start: u64) -> Window {
        Window {
            len,
            index_start,
            blocks: Default::default(),
        }
    }

    /// Fills `buf` with the bytes at `pos` from a block, reading the block first if it isn't the
    /// one held and the last read that missed fell in it too. False if the read is too big,
    /// crosses a block's end, is the first in a row to fall in its block, would take the blocks
    /// read past their share of the bytes asked, or the block can't be read whole: the caller
    /// then reads the file itself, and reports what goes wrong.
    fn read(&self, file: &File, pos: u64, buf: &mut [u8]) -> bool {
        let start = pos - pos % BLOCK;
        let end = start.saturating_add(BLOCK).min(self.len);
        let want = buf.len() as u64;
        if want > SMALL_READ || pos.saturating_add(want) > end {
            return false;
        }
        let Some(slot) = self.blocks.get(usize::from(pos >= self.index_start)) else {
            return false;
        };
        let (at, len) = ((pos - start) as usize, buf.len());
        let size = end - start;
        // The lock is never held across a read of the file.
        let mut bytes = {
            let Ok(mut held) = slot.lock() else {
                return false;
            };
            held.asked = held.asked.saturating_add(want);
            if held.start == start
                && let Some(from) = held.bytes.get(at..at + len)
            {
                buf.copy_from_slice(from);
                return true;
            }
            if held.missed.replace(start) != Some(start) {
                return false;
            }
            let read = held.read.saturating_add(size);
            if read > held.asked.saturating_mul(BLOCK_BYTES_PER_BYTE_ASKED) {
                return false;
            }
            held.read = read;
            std::mem::take(&mut held.bytes)
        };
        let size = size as usize;
        bytes.clear();
        if bytes.try_reserve_exact(size).is_err() {
            return false;
        }
        bytes.resize(size, 0);
        if !read_whole(file, start, &mut bytes) {
            return false;
        }
        let Some(from) = bytes.get(at..at + len) else {
            return false;
        };
        buf.copy_from_slice(from);
        if let Ok(mut held) = slot.lock() {
            (held.start, held.bytes) = (start, bytes);
        }
        true
    }

    /// Where each held block starts, and its length.
    #[cfg(test)]
    fn held(&self) -> Vec<(u64, usize)> {
        let held = |b: &Mutex<Block>| b.lock().map(|b| (b.start, b.bytes.len())).unwrap();
        self.blocks.iter().map(held).collect()
    }

    /// The bytes asked of each slot, and the bytes it read as blocks.
    #[cfg(test)]
    fn counts(&self) -> Vec<(u64, u64)> {
        let counts = |b: &Mutex<Block>| b.lock().map(|b| (b.asked, b.read)).unwrap();
        self.blocks.iter().map(counts).collect()
    }
}

/// Whether `buf` could be filled from `pos` on.
fn read_whole(file: &File, mut pos: u64, mut buf: &mut [u8]) -> bool {
    while !buf.is_empty() {
        #[cfg(test)]
        open_files::read();
        match safe_fs::read_at(file, buf, pos) {
            Ok(0) => return false,
            Ok(n) => {
                buf = &mut buf[n..];
                pos += n as u64;
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(_) => return false,
        }
    }
    true
}

/// Passes writes through while hashing them.
pub(crate) struct HashingWriter<'a> {
    pub(crate) inner: &'a mut dyn Write,
    pub(crate) hasher: ContentHasher,
}

impl Write for HashingWriter<'_> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let n = self.inner.write(buf)?;
        self.hasher.update(&buf[..n]);
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;
    use crate::chunks::Chunk;
    use crate::id::{Flavor, hash_content, hash_record};
    use crate::listing::{Entry, EntryKind, Listing};
    use std::collections::BTreeMap;
    use std::path::Path;

    fn dirs() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let root = tempfile::tempdir().unwrap();
        let tmp = root.path().join("tmp");
        let packs = root.path().join("packs");
        std::fs::create_dir(&tmp).unwrap();
        std::fs::create_dir(&packs).unwrap();
        (root, tmp, packs)
    }

    fn is_empty_dir(dir: &Path) -> bool {
        std::fs::read_dir(dir).unwrap().next().is_none()
    }

    fn one_file_listing() -> Listing {
        Listing::new(vec![Entry {
            name: "a".into(),
            kind: EntryKind::File,
            id: Id([0; 32]),
            size: 5,
        }])
        .unwrap()
    }

    fn minimal_save() -> crate::save::Save {
        let person = crate::save::Person {
            name: "A".into(),
            email: "a@b".into(),
            time_ms: 0,
            tz_offset_min: 0,
        };
        crate::save::Save {
            root: Id([0; 32]),
            parents: vec![],
            author: person.clone(),
            saver: person,
            message: "m".into(),
            change_id: crate::id::ChangeId([0; 16]),
            move_notes: vec![],
            signature: vec![],
            extra: BTreeMap::new(),
        }
    }

    fn entries(r: &PackReader) -> Vec<IndexEntry> {
        r.entries().collect::<Result<_, _>>().unwrap()
    }

    /// Reads an object back whatever its kind.
    fn read_any(r: &PackReader, e: &IndexEntry) -> Result<Vec<u8>, PackError> {
        if e.kind == ObjectKind::Content {
            let mut out = Vec::new();
            r.copy_content(e, &mut out)?;
            Ok(out)
        } else {
            r.read_record(e)
        }
    }

    #[test]
    fn round_trip_records_and_content() {
        let (_root, tmp, packs) = dirs();
        let listing = Listing::empty().encode();
        let save = minimal_save().encode();
        let zeros = vec![0u8; 1 << 20];
        let mut random = vec![0u8; 1 << 20];
        getrandom::fill(&mut random).unwrap();

        let mut w = PackWriter::create(&tmp).unwrap();
        let mut expected = BTreeMap::new();
        expected.insert(
            w.add_record(ObjectKind::FolderListing, &listing).unwrap(),
            listing.clone(),
        );
        expected.insert(w.add_record(ObjectKind::Save, &save).unwrap(), save.clone());
        for c in [Vec::new(), zeros, random] {
            expected.insert(w.add_content(&c).unwrap(), c);
        }
        assert!(expected.contains_key(&hash_record(Flavor::FolderListing, &listing)));
        assert!(expected.contains_key(&hash_record(Flavor::Save, &save)));
        let path = w.finish(&packs).unwrap().unwrap();

        let r = PackReader::open(&path).unwrap();
        assert_eq!(r.entries().count(), 5);
        assert!(entries(&r).windows(2).all(|p| p[0].id < p[1].id));
        for e in &entries(&r) {
            assert_eq!(read_any(&r, e).unwrap(), expected[&e.id]);
            assert_eq!(r.find(&e.id).unwrap().unwrap().offset, e.offset);
        }
        assert!(is_empty_dir(&tmp));
    }

    #[test]
    fn tag_lists_and_server_lines_are_kinds_of_their_own() {
        let (_root, tmp, packs) = dirs();
        let mut w = PackWriter::create(&tmp).unwrap();
        let tags = w.add_record(ObjectKind::TagList, b"tags").unwrap();
        let servers = w.add_record(ObjectKind::ServerLines, b"servers").unwrap();
        assert_eq!(tags, hash_record(Flavor::TagList, b"tags"));
        assert_eq!(servers, hash_record(Flavor::ServerLines, b"servers"));
        let r = PackReader::open(&w.finish(&packs).unwrap().unwrap()).unwrap();
        let (tags, servers) = (
            r.find(&tags).unwrap().unwrap(),
            r.find(&servers).unwrap().unwrap(),
        );
        assert_eq!((tags.kind as u8, servers.kind as u8), (22, 23));
        assert_eq!(r.read_record(&tags).unwrap(), b"tags");
        assert_eq!(r.read_record(&servers).unwrap(), b"servers");
    }

    #[test]
    fn identical_objects_are_stored_once() {
        let (_root, tmp, packs) = dirs();
        let mut w = PackWriter::create(&tmp).unwrap();
        let a = w.add_content(b"same").unwrap();
        let b = w.add_content(b"same").unwrap();
        assert_eq!(a, b);
        assert!(w.contains(&a));
        let r = PackReader::open(&w.finish(&packs).unwrap().unwrap()).unwrap();
        assert_eq!(r.entries().count(), 1);
    }

    #[test]
    fn empty_writer_writes_no_pack() {
        let (_root, tmp, packs) = dirs();
        let w = PackWriter::create(&tmp).unwrap();
        assert!(w.is_empty());
        assert!(w.finish(&packs).unwrap().is_none());
        assert!(is_empty_dir(&tmp));
        assert!(is_empty_dir(&packs));
    }

    #[test]
    fn pack_name_is_the_index_checksum() {
        let (_root, tmp, packs) = dirs();
        let mut w = PackWriter::create(&tmp).unwrap();
        w.add_content(b"hello").unwrap();
        w.add_record(ObjectKind::FolderListing, &Listing::empty().encode())
            .unwrap();
        let path = w.finish(&packs).unwrap().unwrap();
        let bytes = std::fs::read(&path).unwrap();
        let trailer = bytes.len() - 48;
        let index_offset = u64::from_le_bytes(bytes[trailer..trailer + 8].try_into().unwrap());
        let index = &bytes[usize::try_from(index_offset).unwrap()..trailer];
        let name = format!("{}.pack", blake3::hash(index).to_hex());
        assert_eq!(path.file_name().unwrap().to_str().unwrap(), name);
    }

    #[test]
    fn every_single_byte_flip_is_detected() {
        let (_root, tmp, packs) = dirs();
        let mut w = PackWriter::create(&tmp).unwrap();
        let mut expected = BTreeMap::new();
        for rec in [Listing::empty().encode(), one_file_listing().encode()] {
            expected.insert(w.add_record(ObjectKind::FolderListing, &rec).unwrap(), rec);
        }
        // Compressible, so the pack holds both stored forms.
        let content = b"hello pack ".repeat(100);
        expected.insert(w.add_content(&content).unwrap(), content);
        let path = w.finish(&packs).unwrap().unwrap();
        let good = std::fs::read(&path).unwrap();
        let flipped_path = tmp.join("flipped.pack");

        for i in 0..good.len() {
            let mut bad = good.clone();
            bad[i] ^= 1;
            std::fs::write(&flipped_path, &bad).unwrap();
            let failed = match PackReader::open(&flipped_path) {
                Err(_) => true,
                Ok(r) => {
                    // Opening skips the index checksum, which `verify` checks.
                    let mut failed = r.check_index().is_err();
                    for e in r.entries() {
                        match e.and_then(|e| read_any(&r, &e).map(|data| (e, data))) {
                            Ok((e, data)) => assert_eq!(
                                Some(&data),
                                expected.get(&e.id),
                                "byte {i}: different data returned"
                            ),
                            Err(_) => failed = true,
                        }
                    }
                    failed
                }
            };
            assert!(failed, "flipping byte {i} went unnoticed");
        }
    }

    #[test]
    fn truncated_pack_is_refused() {
        let (_root, tmp, packs) = dirs();
        let mut w = PackWriter::create(&tmp).unwrap();
        w.add_content(b"hello").unwrap();
        let path = w.finish(&packs).unwrap().unwrap();
        let bytes = std::fs::read(&path).unwrap();
        std::fs::write(&path, &bytes[..bytes.len() - 1]).unwrap();
        assert!(PackReader::open(&path).is_err());
    }

    /// A version 1 pack with no entries but `index`, under a correct checksum.
    fn pack_with_index(index: &[u8]) -> Vec<u8> {
        let mut out = PACK_MAGIC.to_vec();
        out.extend_from_slice(&1u32.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(index);
        out.extend_from_slice(&PACK_HEADER_LEN.to_le_bytes());
        out.extend_from_slice(blake3::hash(index).as_bytes());
        out.extend_from_slice(&PACK_END_MAGIC);
        out
    }

    fn zero_the_checksum(pack: &mut [u8]) {
        let at = pack.len() - 40;
        pack[at..at + 32].fill(0);
    }

    /// Why `bytes`, as a pack, is refused.
    fn refusal(dir: &Path, bytes: &[u8]) -> String {
        let path = dir.join("bad.pack");
        std::fs::write(&path, bytes).unwrap();
        match PackReader::open(&path) {
            Err(PackError::Corrupt { reason, .. }) => reason,
            other => panic!("{:?}", other.map(|r| r.entries().count())),
        }
    }

    fn varint(n: u64) -> Vec<u8> {
        let mut w = Writer::new();
        w.varint(n);
        w.into_bytes()
    }

    #[test]
    fn an_index_listing_too_many_objects_is_refused_before_its_lines_are_read() {
        let (_root, tmp, _packs) = dirs();
        // Only the count is there: reading lines would have run into the end of the index.
        let reason = refusal(&tmp, &pack_with_index(&varint(MAX_PACK_OBJECTS + 1)));
        assert!(reason.contains("more than 16777216"), "{reason}");
    }

    #[test]
    fn an_index_whose_length_does_not_fit_its_count_is_refused_before_its_lines_are_read() {
        let (_root, tmp, _packs) = dirs();
        // A line is 43 to 61 bytes: no lines take 0 bytes, one at most 61, two at least 86.
        for (count, len) in [(0, 1), (1, 62), (2, 85)] {
            let mut index = varint(count);
            index.resize(index.len() + len, 0);
            let reason = refusal(&tmp, &pack_with_index(&index));
            let want = format!("its {len} bytes don't fit its entry count of {count}");
            assert!(reason.contains(&want), "{reason}");
        }
    }

    #[test]
    fn an_index_of_zeros_is_refused_long_before_its_checksum() {
        let (_root, tmp, _packs) = dirs();
        // A sparse pack in small: zeros, and a checksum of zeros.
        let mut zeros = pack_with_index(&vec![0; 1 << 20]);
        zero_the_checksum(&mut zeros);
        let reason = refusal(&tmp, &zeros);
        assert!(
            reason.contains("don't fit its entry count of 0"),
            "{reason}"
        );
        // With a count that fits the length, zeros fail at the first line.
        let mut index = varint(1000);
        index.resize(index.len() + 1000 * 43, 0);
        let mut zeros = pack_with_index(&index);
        zero_the_checksum(&mut zeros);
        let reason = refusal(&tmp, &zeros);
        assert!(reason.contains("unknown object kind 0"), "{reason}");
    }

    #[test]
    fn an_index_at_the_limit_is_accepted() {
        let (_root, tmp, packs) = dirs();
        let mut w = PackWriter::create(&tmp).unwrap();
        for c in [&b"a"[..], b"b", b"c"] {
            w.add_content(c).unwrap();
        }
        let path = w.finish(&packs).unwrap().unwrap();
        assert_eq!(
            PackReader::open_with(&path, 3).unwrap().entries().count(),
            3
        );
        assert!(matches!(
            PackReader::open_with(&path, 2),
            Err(PackError::Corrupt { .. })
        ));
    }

    /// A pack of `n` small contents, its bytes, and where its index starts.
    fn written(n: u32) -> (tempfile::TempDir, Vec<Id>, Vec<u8>, usize) {
        let (root, tmp, packs) = dirs();
        let mut w = PackWriter::create(&tmp).unwrap();
        let ids = (0..n)
            .map(|i| w.add_content(&i.to_le_bytes()).unwrap())
            .collect();
        let bytes = std::fs::read(w.finish(&packs).unwrap().unwrap()).unwrap();
        let trailer = bytes.len() - 48;
        let at = u64::from_le_bytes(bytes[trailer..trailer + 8].try_into().unwrap());
        (root, ids, bytes, usize::try_from(at).unwrap())
    }

    #[test]
    fn a_v2_index_line_is_found_by_position() {
        let (root, mut ids, mut bytes, index) = written(300);
        assert_eq!(&bytes[8..12], &2u32.to_le_bytes());
        ids.sort();
        // Line 7 gets a kind no pack has: opening never reads it, only looking it up does.
        bytes[index + 1032 + 32 * 300 + 25 * 7] = 0xee;
        let path = root.path().join("v2.pack");
        std::fs::write(&path, &bytes).unwrap();
        let r = PackReader::open(&path).unwrap();
        for (i, id) in ids.iter().enumerate() {
            match r.find(id) {
                Ok(Some(e)) => {
                    assert_ne!(i, 7);
                    assert_eq!(e.id, *id);
                    assert_eq!(read_any(&r, &e).unwrap().len(), 4);
                }
                Err(PackError::Corrupt { reason, .. }) => {
                    assert_eq!(i, 7);
                    assert!(reason.contains("unknown object kind 238"), "{reason}");
                }
                other => panic!("{i}: {other:?}"),
            }
        }
        assert_eq!(r.find(&hash_content(b"not in it")).unwrap(), None);
    }

    #[test]
    fn a_fan_out_that_decreases_or_overshoots_is_refused() {
        let (root, ids, good, index) = written(40);
        let fan = |b: usize| index + 8 + 4 * b;
        let set = |bytes: &mut Vec<u8>, b: usize, n: u32| {
            bytes[fan(b)..fan(b) + 4].copy_from_slice(&n.to_le_bytes());
        };
        // Number 1 one more than number 2, still within the count.
        let second = u32::from_le_bytes(good[fan(2)..fan(2) + 4].try_into().unwrap());
        assert!(second < 40);
        let mut decreasing = good.clone();
        set(&mut decreasing, 1, second + 1);
        let mut overshooting = good.clone();
        set(&mut overshooting, 255, 41);
        let mut short = good.clone();
        set(&mut short, 255, 39);
        assert_eq!(ids.len(), 40);
        for bad in [decreasing, overshooting, short] {
            let reason = refusal(root.path(), &bad);
            assert!(reason.contains("fan-out"), "{reason}");
        }
    }

    /// A chunk list is stored only under its file's ID, never under a hash of its bytes.
    #[test]
    fn add_record_refuses_a_chunk_list() {
        let (_root, tmp, packs) = dirs();
        let mut w = PackWriter::create(&tmp).unwrap();
        let refused = w.add_record(ObjectKind::ChunkList, b"a list");
        assert!(matches!(refused, Err(PackError::Io(_))), "{refused:?}");
        assert!(w.is_empty());
        let mut set = PackSet::with_limit(&tmp, &packs, 10).unwrap();
        assert!(set.add_record(ObjectKind::ChunkList, b"a list").is_err());
        // The set still takes what it may.
        set.add_content(b"fine").unwrap();
        assert_eq!(set.finish().unwrap().len(), 1);
    }

    /// A chunk list's line records its file's size, up to 1 TiB, but its own bytes are a record's.
    #[test]
    fn a_chunk_lists_line_may_record_a_big_file_but_not_a_big_record() {
        let (root, tmp, packs) = dirs();
        let list = |id: u8, size: u64| ChunkList {
            file: Id([id; 32]),
            size,
            chunks: vec![Chunk {
                id: Id([0x80; 32]),
                size,
            }],
        };
        let ten_gib = list(1, 10 << 30);
        let mut w = PackWriter::create(&tmp).unwrap();
        w.add_chunk_list(&ten_gib).unwrap();
        let over = w.add_chunk_list(&list(2, MAX_OBJECT_SIZE + 1));
        assert!(matches!(over, Err(PackError::TooLarge)), "{over:?}");
        let path = w.finish(&packs).unwrap().unwrap();
        let r = PackReader::open(&path).unwrap();
        let e = r.find(&ten_gib.file).unwrap().unwrap();
        assert_eq!(
            (e.kind, e.uncompressed_size),
            (ObjectKind::ChunkList, 10 << 30)
        );
        assert_eq!(r.read_chunk_list(&e).unwrap(), ten_gib);

        // The line read by position in version 2: as a save, or with too many stored bytes.
        let good = std::fs::read(&path).unwrap();
        let trailer = good.len() - 48;
        let index = u64::from_le_bytes(good[trailer..trailer + 8].try_into().unwrap());
        let line = usize::try_from(index).unwrap() + 1032 + 32;
        let mut as_save = good.clone();
        as_save[line] = ObjectKind::Save as u8;
        let mut stored = good.clone();
        stored[line + 17..line + 25].copy_from_slice(&(MAX_RECORD_SIZE + 1).to_le_bytes());
        for bad in [as_save, stored] {
            let path = root.path().join("bad.pack");
            std::fs::write(&path, bad).unwrap();
            let refused = PackReader::open(&path).unwrap().find(&ten_gib.file);
            assert!(
                matches!(&refused, Err(PackError::Corrupt { reason, .. }) if reason.contains("larger than allowed")),
                "{refused:?}"
            );
        }

        // And in version 1, as the index is read: the size check comes before the entry's place,
        // which these lines, in a pack with no entries, can't pass.
        let v1_line = |kind: ObjectKind, size: u64, stored: u64| {
            let mut index = Writer::new();
            index.varint(1);
            index.id(&ten_gib.file);
            index.u8(kind as u8);
            index.u64(PACK_HEADER_LEN);
            index.varint(size);
            index.varint(stored);
            refusal(root.path(), &pack_with_index(&index.into_bytes()))
        };
        let list_line = v1_line(ObjectKind::ChunkList, 10 << 30, 100);
        assert!(
            list_line.contains("lies outside the entry area"),
            "{list_line}"
        );
        for refused in [
            v1_line(ObjectKind::Save, 10 << 30, 100),
            v1_line(ObjectKind::ChunkList, 10 << 30, MAX_RECORD_SIZE + 1),
        ] {
            assert!(refused.contains("larger than allowed"), "{refused}");
        }
    }

    #[test]
    fn zstd_running_out_of_memory_is_told_apart_from_bad_data() {
        // The text zstd gives when memory runs out.
        let oom = io::Error::other("Allocation error : not enough memory");
        assert!(zstd_ran_out_of_memory(&oom));
        assert!(!zstd_ran_out_of_memory(&io::Error::other(
            "Corrupted block detected"
        )));
    }

    #[test]
    fn a_damaged_packs_path_is_shown_escaped() {
        let (_root, _tmp, packs) = dirs();
        // A name no terminal may be shown as it is; the project folder's name is in the path too.
        let path = packs.join("a\u{202e}b.pack");
        std::fs::write(&path, b"short").unwrap();
        let shown = PackReader::open(&path).err().unwrap().to_string();
        assert!(!shown.contains('\u{202e}'), "{shown}");
        assert!(shown.contains("a\\u{202e}b.pack is damaged"), "{shown}");
    }

    /// A version 1 pack with one content entry whose header and index claim `claimed`
    /// uncompressed bytes.
    fn hand_built_pack(stored: &[u8], claimed: u64, id: Id) -> Vec<u8> {
        let mut w = crate::encoding::Writer::new();
        w.u8(ObjectKind::Content as u8);
        w.id(&id);
        w.u64(claimed);
        w.u64(stored.len() as u64);
        w.u8(1);
        let entry = w.into_bytes();

        let mut index = crate::encoding::Writer::new();
        index.varint(1);
        index.id(&id);
        index.u8(ObjectKind::Content as u8);
        index.u64(16);
        index.varint(claimed);
        index.varint(stored.len() as u64);
        let index = index.into_bytes();

        let mut out = Vec::new();
        out.extend_from_slice(&PACK_MAGIC);
        out.extend_from_slice(&1u32.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&entry);
        out.extend_from_slice(stored);
        let index_offset = out.len() as u64;
        out.extend_from_slice(&index);
        out.extend_from_slice(&index_offset.to_le_bytes());
        out.extend_from_slice(blake3::hash(&index).as_bytes());
        out.extend_from_slice(&PACK_END_MAGIC);
        out
    }

    #[test]
    fn decompression_bomb_is_refused() {
        let (_root, tmp, _packs) = dirs();
        let zeros = vec![0u8; 1 << 20];
        // One frame records its content size, the streamed one doesn't.
        let with_size = zstd::bulk::compress(&zeros, 3).unwrap();
        let mut enc = zstd::stream::write::Encoder::new(Vec::new(), 3).unwrap();
        std::io::Write::write_all(&mut enc, &zeros).unwrap();
        let without_size = enc.finish().unwrap();

        for frame in [with_size, without_size] {
            let path = tmp.join("bomb.pack");
            std::fs::write(
                &path,
                hand_built_pack(&frame, 10, hash_content(&zeros[..10])),
            )
            .unwrap();
            let r = PackReader::open(&path).unwrap();
            let mut out = Vec::new();
            let err = r.copy_content(&entries(&r)[0], &mut out).unwrap_err();
            assert!(matches!(err, PackError::Corrupt { .. }), "{err:?}");
            assert!(out.len() <= 11, "produced {} bytes", out.len());
        }
    }

    #[derive(Clone, Copy, Debug, PartialEq)]
    enum Damage {
        Header,
        FrameSize,
        MoreBytes,
        FewerBytes,
        CutShort,
        Fingerprint { zstd: bool },
        HugeWindow,
        SecondFrame,
        SecondPlainFrame,
        SkippableFirst,
        SkippableAfter,
        TrailingByte,
        DictionaryId,
        UnusedBit,
        WithChecksum,
        WrongChecksum,
        OversizedBlock,
        Legacy,
        UnknownCompression,
        RawSizeDiffers,
        PackCutShort,
    }

    /// A skippable frame holding four bytes.
    const SKIPPABLE: [u8; 12] = [0x50, 0x2A, 0x4D, 0x18, 4, 0, 0, 0, 1, 2, 3, 4];

    /// A frame of one block of `kind` (0 raw, 1 RLE) holding `body` and making `size` bytes, with a
    /// window from `window` (0x00 for 1 KiB, 0x50 for 1 MiB) and its content size given.
    fn one_block_frame(window: u8, kind: u32, body: &[u8], size: usize) -> Vec<u8> {
        let mut frame = ZSTD_MAGIC.to_vec();
        frame.extend_from_slice(&[0x80, window]);
        frame.extend_from_slice(&(size as u32).to_le_bytes());
        let header = (size as u32) << 3 | kind << 1 | 1;
        frame.extend_from_slice(&header.to_le_bytes()[..3]);
        frame.extend_from_slice(body);
        frame
    }

    /// `frame` with a `Dictionary_ID` field of one byte, 0: no dictionary, which zstd reads.
    fn with_dictionary_id(frame: &[u8]) -> Vec<u8> {
        let mut out = frame.to_vec();
        // The field comes after the window byte, which a single-segment frame doesn't have.
        let at = if out[4] & 0x20 != 0 { 5 } else { 6 };
        out[4] |= 0x01;
        out.insert(at, 0);
        out
    }

    const ZSTD_MAGIC: [u8; 4] = [0x28, 0xB5, 0x2F, 0xFD];

    /// One frame of raw blocks holding `bytes`, declaring their size and a window of 2^30: over
    /// the streamed decoder's limit, under zstd's own.
    fn huge_window_frame(bytes: &[u8]) -> Vec<u8> {
        let mut frame = ZSTD_MAGIC.to_vec();
        frame.extend_from_slice(&[0x80, 0xA0]);
        frame.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
        let blocks: Vec<&[u8]> = bytes.chunks(128 << 10).collect();
        for (i, block) in blocks.iter().enumerate() {
            let last = u32::from(i + 1 == blocks.len());
            frame.extend_from_slice(&((block.len() as u32) << 3 | last).to_le_bytes()[..3]);
            frame.extend_from_slice(block);
        }
        frame
    }

    /// Reading an entry of `len` random bytes stored as `kind` with `damage` done to it, told
    /// without its ID or the pack's path, and the reads it took.
    fn read_damaged(kind: ObjectKind, len: usize, damage: Damage) -> (String, usize) {
        let (_root, tmp, packs) = dirs();
        let mut data = vec![0u8; len];
        getrandom::fill(&mut data).unwrap();
        let id_of = |bytes: &[u8]| {
            kind.flavor()
                .map_or(hash_content(bytes), |f| hash_record(f, bytes))
        };
        let unsized_frame = |bytes: &[u8]| {
            let mut enc = zstd::stream::write::Encoder::new(Vec::new(), 3).unwrap();
            enc.include_contentsize(false).unwrap();
            std::io::Write::write_all(&mut enc, bytes).unwrap();
            enc.finish().unwrap()
        };
        let mut longer = data.clone();
        longer.push(7);
        let mut id = id_of(&data);
        let (zstd, stored) = match damage {
            Damage::Header => (false, data.clone()),
            Damage::FrameSize => (true, zstd::bulk::compress(&longer, 3).unwrap()),
            Damage::MoreBytes => (true, unsized_frame(&longer)),
            Damage::FewerBytes => (true, unsized_frame(&data[..len - 1])),
            Damage::CutShort => {
                let frame = zstd::bulk::compress(&data, 3).unwrap();
                (true, frame[..frame.len() - 100].to_vec())
            }
            Damage::Fingerprint { zstd } => {
                id.0[0] ^= 1;
                let stored = if zstd {
                    zstd::bulk::compress(&data, 3).unwrap()
                } else {
                    data.clone()
                };
                (zstd, stored)
            }
            Damage::HugeWindow => (true, huge_window_frame(&data)),
            Damage::SecondFrame => {
                let mut frames = zstd::bulk::compress(&data, 3).unwrap();
                // No content size, a window of 2^30, one empty last block.
                frames.extend_from_slice(&ZSTD_MAGIC);
                frames.extend_from_slice(&[0x00, 0xA0, 0x01, 0x00, 0x00]);
                (true, frames)
            }
            Damage::SecondPlainFrame => {
                let mut frames = zstd::bulk::compress(&data, 3).unwrap();
                frames.extend_from_slice(&zstd::bulk::compress(&[], 3).unwrap());
                (true, frames)
            }
            Damage::SkippableFirst => {
                let mut frames = SKIPPABLE.to_vec();
                frames.extend_from_slice(&zstd::bulk::compress(&data, 3).unwrap());
                (true, frames)
            }
            Damage::SkippableAfter => {
                let mut frames = zstd::bulk::compress(&data, 3).unwrap();
                frames.extend_from_slice(&SKIPPABLE);
                (true, frames)
            }
            Damage::TrailingByte => {
                let mut frame = zstd::bulk::compress(&data, 3).unwrap();
                frame.push(0);
                (true, frame)
            }
            Damage::DictionaryId => (
                true,
                with_dictionary_id(&zstd::bulk::compress(&data, 3).unwrap()),
            ),
            Damage::UnusedBit => {
                let mut frame = zstd::bulk::compress(&data, 3).unwrap();
                frame[4] |= 0x10;
                (true, frame)
            }
            Damage::WithChecksum | Damage::WrongChecksum => {
                let mut enc = zstd::stream::write::Encoder::new(Vec::new(), 3).unwrap();
                enc.include_checksum(true).unwrap();
                std::io::Write::write_all(&mut enc, &data).unwrap();
                let mut frame = enc.finish().unwrap();
                if damage == Damage::WrongChecksum {
                    *frame.last_mut().unwrap() ^= 1;
                }
                (true, frame)
            }
            Damage::OversizedBlock => (true, one_block_frame(0x50, 0, &data, len)),
            Damage::Legacy => {
                let mut frame = vec![0x27, 0xB5, 0x2F, 0xFD];
                // Fixed bytes, so both sizes meet the same old-format header; more than a
                // block's largest read, so the streamed path reads the file again.
                frame.extend_from_slice(&[0; SMALL_READ as usize]);
                (true, frame)
            }
            Damage::UnknownCompression | Damage::PackCutShort => (false, data.clone()),
            Damage::RawSizeDiffers => (false, longer.clone()),
        };
        let p = Prepared {
            id,
            size: len as u64,
            zstd,
            stored,
        };
        let mut w = PackWriter::create(&tmp).unwrap();
        w.add_prepared(kind, &p).unwrap();
        let path = w.finish(&packs).unwrap().unwrap();
        let mut bytes = std::fs::read(&path).unwrap();
        let entry = entries(&PackReader::open(&path).unwrap())[0];
        let at = entry.offset as usize;
        match damage {
            // The low byte of the uncompressed size.
            Damage::Header => bytes[at + 33] ^= 1,
            Damage::UnknownCompression => bytes[at + 49] = 2,
            _ => {}
        }
        std::fs::write(&path, &bytes).unwrap();
        let r = PackReader::open(&path).unwrap();
        if let Damage::PackCutShort = damage {
            let cut = entry.offset + ENTRY_HEADER_LEN + entry.stored_size / 2;
            let file = OpenOptions::new().write(true).open(&path).unwrap();
            file.set_len(cut).unwrap();
        }
        let before = open_files::reads();
        let shown = match read_any(&r, &entry) {
            Ok(_) => "read".to_string(),
            Err(PackError::Corrupt { reason, .. }) => format!("damaged: {reason}"),
            Err(e) => e.to_string(),
        };
        let reads = open_files::reads() - before;
        (shown.replace(&id.to_string(), "<id>"), reads)
    }

    #[test]
    fn a_small_entry_is_refused_as_a_streamed_one_is() {
        let invalid = "damaged: the zstd data of <id> is invalid: ";
        let wrong_size = "damaged: <id> does not have its recorded size";
        let not_allowed = Some("damaged: the zstd data of <id> is not a frame format 1 allows");
        let goes_on = Some("damaged: the zstd data of <id> goes on after its frame");
        // `None`: whatever the streamed path says, the decoder's own words.
        let cases = [
            (
                Damage::Header,
                Some("damaged: the header of <id> does not match the index"),
            ),
            (
                Damage::FrameSize,
                Some("damaged: the zstd frame of <id> has the wrong size"),
            ),
            (Damage::MoreBytes, Some(wrong_size)),
            (Damage::FewerBytes, Some(wrong_size)),
            (Damage::CutShort, None),
            (
                Damage::Fingerprint { zstd: false },
                Some("object <id> does not match its fingerprint"),
            ),
            (
                Damage::Fingerprint { zstd: true },
                Some("object <id> does not match its fingerprint"),
            ),
            (Damage::HugeWindow, not_allowed),
            (Damage::SecondFrame, goes_on),
            (Damage::SecondPlainFrame, goes_on),
            (Damage::SkippableFirst, not_allowed),
            (Damage::SkippableAfter, goes_on),
            (Damage::TrailingByte, goes_on),
            (Damage::DictionaryId, not_allowed),
            (Damage::UnusedBit, not_allowed),
            (Damage::WithChecksum, Some("read")),
            (Damage::WrongChecksum, None),
            (
                Damage::OversizedBlock,
                Some(
                    "damaged: the zstd data of <id> is invalid: a block holds more than its frame allows",
                ),
            ),
            (Damage::Legacy, not_allowed),
            (
                Damage::UnknownCompression,
                Some("damaged: <id> has an unknown compression"),
            ),
            (
                Damage::RawSizeDiffers,
                Some("damaged: <id> has an unknown compression"),
            ),
            (Damage::PackCutShort, Some(wrong_size)),
        ];
        for kind in [ObjectKind::Content, ObjectKind::FolderListing] {
            for (damage, expected) in cases {
                let (small, reads) = read_damaged(kind, ONE_READ as usize - 4096, damage);
                let (streamed, _) = read_damaged(kind, ONE_READ as usize + 4096, damage);
                assert_eq!(small, streamed, "{kind:?} {damage:?}");
                // Only a good frame, or a wrong fingerprint, checked after either path, is
                // settled by the one read.
                let settled = matches!(damage, Damage::Fingerprint { .. } | Damage::WithChecksum);
                assert_eq!(reads == 1, settled, "{kind:?} {damage:?}: {reads} reads");
                if matches!(damage, Damage::CutShort | Damage::WrongChecksum) {
                    assert!(small.starts_with(invalid), "{kind:?} {damage:?}: {small}");
                }
                if let Some(expected) = expected {
                    assert_eq!(small, expected, "{kind:?} {damage:?}");
                }
            }
        }
    }

    #[test]
    fn only_one_plain_frame_goes_to_the_one_call_decoder() {
        let bytes = b"plain frame ".repeat(100);
        let plain_frame = |frame: &[u8]| one_plain_frame(frame, bytes.len() as u64);
        // The two ways Opalro writes a frame, and a frame with a checksum.
        let plain = zstd::bulk::compress(&bytes, ZSTD_LEVEL).unwrap();
        assert!(plain_frame(&plain));
        let mut streamed = zstd::stream::write::Encoder::new(Vec::new(), ZSTD_LEVEL).unwrap();
        std::io::Write::write_all(&mut streamed, &bytes).unwrap();
        assert!(plain_frame(&streamed.finish().unwrap()));
        let mut checked = zstd::stream::write::Encoder::new(Vec::new(), 3).unwrap();
        checked.include_checksum(true).unwrap();
        std::io::Write::write_all(&mut checked, &bytes).unwrap();
        assert!(plain_frame(&checked.finish().unwrap()));
        assert!(!one_plain_frame(&plain, bytes.len() as u64 + 1));

        // Real frames, which zstd's one-call decoder reads but format 1 doesn't hold: it takes
        // any window, a dictionary ID of 0 means no dictionary, the unused bit is ignored, and
        // it doesn't check the size of a raw or RLE block against the window.
        let huge = huge_window_frame(&bytes);
        assert!(!plain_frame(&huge));
        assert_eq!(zstd::bulk::decompress(&huge, bytes.len()).unwrap(), bytes);
        let with_id = with_dictionary_id(&plain);
        assert!(!plain_frame(&with_id));
        assert_eq!(
            zstd::bulk::decompress(&with_id, bytes.len()).unwrap(),
            bytes
        );
        let mut unused = plain.clone();
        unused[4] |= 0x10;
        assert!(!plain_frame(&unused));
        assert_eq!(zstd::bulk::decompress(&unused, bytes.len()).unwrap(), bytes);
        // A window of 1 KiB, so blocks of at most 1 KiB.
        let raw = one_block_frame(0x00, 0, &bytes, bytes.len());
        assert!(!plain_frame(&raw));
        assert_eq!(zstd::bulk::decompress(&raw, bytes.len()).unwrap(), bytes);
        let rle = one_block_frame(0x00, 1, b"z", 2048);
        assert!(!one_plain_frame(&rle, 2048));
        assert_eq!(zstd::bulk::decompress(&rle, 2048).unwrap(), [b'z'; 2048]);
        assert!(one_plain_frame(&one_block_frame(0x00, 1, b"z", 1024), 1024));

        let mut skippable = SKIPPABLE.to_vec();
        skippable.extend_from_slice(&plain);
        assert!(!plain_frame(&skippable));
        let mut twice = plain.clone();
        twice.extend_from_slice(&plain);
        assert!(!plain_frame(&twice));
        let mut legacy = plain.clone();
        legacy[0] = 0x27;
        assert!(!plain_frame(&legacy));
        assert!(!plain_frame(&plain[..plain.len() - 1]));
        assert!(!plain_frame(&[]));
    }

    #[test]
    fn a_block_walk_takes_the_bytes_in_any_pieces() {
        // Several blocks: runs of zeros, then text.
        let mut bytes = vec![0u8; 300 << 10];
        bytes.extend(b"some text, then more text ".repeat(4000));
        let frame = zstd::bulk::compress(&bytes, ZSTD_LEVEL).unwrap();
        let start = frame_start(&frame, bytes.len() as u64).unwrap();
        for piece in [1, 7, frame.len()] {
            let mut walk = BlockWalk::new(start);
            assert!(
                frame.chunks(piece).all(|p| walk.feed(p)),
                "pieces of {piece}"
            );
        }
        // A raw block over its window of 1 KiB, its header cut between two pieces.
        let raw = one_block_frame(0x00, 0, &[1; 2048], 2048);
        let start = frame_start(&raw, 2048).unwrap();
        let at = usize::try_from(start.header_len).unwrap() + 1;
        let mut walk = BlockWalk::new(start);
        assert!(walk.feed(&raw[..at]));
        assert!(!walk.feed(&raw[at..]));
    }

    #[test]
    fn frame_starts_are_judged_at_the_limits() {
        let start = |rest: &[u8], size| frame_start(&[&ZSTD_MAGIC[..], rest].concat(), size);
        // Windows of exactly 2^27, and of 2^27 + 2^24.
        assert!(start(&[0x00, 0x88], 0).is_ok());
        assert_eq!(start(&[0x00, 0x89], 0).err(), Some(FrameFault::NotAllowed));
        // A single segment's window is its content size: 2^27, then 2^27 + 1.
        assert!(start(&[0xA0, 0, 0, 0, 0x08], 1 << 27).is_ok());
        assert_eq!(
            start(&[0xA0, 1, 0, 0, 0x08], (1 << 27) + 1).err(),
            Some(FrameFault::NotAllowed)
        );
        // The reserved bit, and a content size of all ones, which zstd reads as none given.
        assert_eq!(start(&[0x08, 0x50], 0).err(), Some(FrameFault::NotAllowed));
        let all_ones = [0xC0, 0x50, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF];
        assert_eq!(start(&all_ones, 10).err(), Some(FrameFault::WrongSize));
        // No content size given, and one given in 1, 2, 4 and 8 bytes: the header's length.
        let len = |rest: &[u8], size| start(rest, size).unwrap().header_len;
        assert_eq!(len(&[0x00, 0x50], 7), 6);
        assert_eq!(len(&[0x20, 7], 7), 6);
        assert_eq!(len(&[0x40, 0x50, 7, 0], 263), 8);
        assert_eq!(len(&[0x80, 0x50, 7, 0, 0, 0], 7), 10);
        assert_eq!(len(&[0xC0, 0x50, 7, 0, 0, 0, 0, 0, 0, 0], 7), 14);
    }

    #[test]
    fn the_build_reads_no_zstd_format_from_before_1_0() {
        // A v0.7 frame's header, giving a content size of 5, which the legacy decoders read.
        let v07 = [0x27, 0xB5, 0x2F, 0xFD, 0x20, 0x05];
        assert!(zstd::zstd_safe::get_frame_content_size(&v07).is_err());
        let v1 = [0x28, 0xB5, 0x2F, 0xFD, 0x20, 0x05];
        assert!(matches!(
            zstd::zstd_safe::get_frame_content_size(&v1),
            Ok(Some(5))
        ));
    }

    /// Reads each entry of the pack at `path` with a reader of its own, so no block read for one
    /// holds the next, and checks it takes one read.
    fn assert_each_read_once(path: &Path) {
        for e in entries(&PackReader::open(path).unwrap()) {
            let r = PackReader::open(path).unwrap();
            let before = open_files::reads();
            read_any(&r, &e).unwrap();
            assert_eq!(open_files::reads() - before, 1, "{e:?}");
        }
    }

    #[test]
    fn a_small_entry_is_read_in_one_read() {
        let (_root, tmp, packs) = dirs();
        let mut w = PackWriter::create(&tmp).unwrap();
        // A listing that compresses, though not below a block's largest read.
        let mut names: Vec<String> = (0u32..200)
            .map(|i| hash_content(&i.to_le_bytes()).to_string())
            .collect();
        names.sort();
        let listing = Listing::new(
            names
                .into_iter()
                .map(|name| Entry {
                    id: hash_content(name.as_bytes()),
                    name,
                    kind: EntryKind::File,
                    size: 1,
                })
                .collect(),
        )
        .unwrap();
        w.add_record(ObjectKind::FolderListing, &listing.encode())
            .unwrap();
        w.add_content(&random(5000)).unwrap();
        // Repeated, so it compresses to about half.
        w.add_content(&random(10_000).repeat(2)).unwrap();
        w.add_content(&random(ONE_READ as usize - 100)).unwrap();
        let path = w.finish(&packs).unwrap().unwrap();
        // Too big for a block, small enough for one read: the streamed path would take two.
        let all = entries(&PackReader::open(&path).unwrap());
        assert!(
            all.iter()
                .all(|e| (SMALL_READ..=ONE_READ).contains(&e.stored_size))
        );
        assert_eq!(
            all.iter()
                .filter(|e| e.stored_size < e.uncompressed_size)
                .count(),
            2
        );
        assert_each_read_once(&path);
    }

    #[test]
    fn a_tiny_entry_is_read_with_its_block() {
        let (_root, tmp, packs) = dirs();
        let mut w = PackWriter::create(&tmp).unwrap();
        w.add_record(ObjectKind::FolderListing, &one_file_listing().encode())
            .unwrap();
        w.add_content(&random(1000)).unwrap();
        w.add_content(&b"squeezes well ".repeat(1000)).unwrap();
        assert_each_read_once(&w.finish(&packs).unwrap().unwrap());
    }

    #[test]
    fn content_that_changes_while_streaming_is_rejected() {
        let (_root, tmp, packs) = dirs();
        let mut w = PackWriter::create(&tmp).unwrap();
        let err = w
            .add_content_from(&mut &b"new"[..], hash_content(b"old"))
            .unwrap_err();
        assert!(matches!(err, PackError::ContentChanged), "{err:?}");
        let x = w.add_content(b"x").unwrap();
        let r = PackReader::open(&w.finish(&packs).unwrap().unwrap()).unwrap();
        assert_eq!(r.entries().count(), 1);
        assert_eq!(read_any(&r, &entries(&r)[0]).unwrap(), b"x");
        assert_eq!(entries(&r)[0].id, x);
    }

    #[test]
    fn streamed_content_round_trips() {
        let (_root, tmp, packs) = dirs();
        let data = b"streamed ".repeat(20_000);
        let mut w = PackWriter::create(&tmp).unwrap();
        let size = w
            .add_content_from(&mut &data[..], hash_content(&data))
            .unwrap();
        assert_eq!(size, data.len() as u64);
        let r = PackReader::open(&w.finish(&packs).unwrap().unwrap()).unwrap();
        assert!(entries(&r)[0].stored_size < size);
        assert_eq!(read_any(&r, &entries(&r)[0]).unwrap(), data);
    }

    #[cfg(feature = "fault-injection")]
    #[test]
    fn crash_inside_finish_leaves_no_pack() {
        use crate::fsio::fault;
        // Fault 1 is before the rename, fault 2 after it; 3 means no fault fires.
        for n in 1.. {
            let (_root, tmp, packs) = dirs();
            let mut w = PackWriter::create(&tmp).unwrap();
            w.add_content(b"hello").unwrap();
            fault::arm(n);
            let result = w.finish(&packs);
            fault::disarm();
            match (n, result) {
                (1, Err(PackError::Io(_))) => {
                    assert!(is_empty_dir(&packs), "fault {n} left a pack");
                    assert!(is_empty_dir(&tmp), "fault {n} left a temp file");
                }
                // After the rename the pack is complete; finishing only failed to report it.
                (2, Err(PackError::Io(_))) => {
                    let p = std::fs::read_dir(&packs).unwrap().next().unwrap().unwrap();
                    assert_eq!(PackReader::open(&p.path()).unwrap().entries().count(), 1);
                }
                (3, Ok(Some(_))) => break,
                (n, other) => panic!("fault {n}: unexpected {other:?}"),
            }
        }
    }

    /// Adds the one-byte contents 0 to `n - 1` and returns their IDs.
    fn add_bytes(set: &mut PackSet, n: u8) -> Vec<Id> {
        (0..n).map(|i| set.add_content(&[i]).unwrap()).collect()
    }

    fn objects_in(path: &Path) -> usize {
        PackReader::open(path).unwrap().entries().count()
    }

    #[test]
    fn a_set_moves_each_full_pack_into_the_store_and_starts_the_next() {
        let (_root, tmp, packs) = dirs();
        let mut set = PackSet::with_limit(&tmp, &packs, 3).unwrap();
        let ids = add_bytes(&mut set, 7);
        // The first two packs are in the store; only the newest is still being built.
        assert_eq!(std::fs::read_dir(&packs).unwrap().count(), 2);
        assert!(set.contains(&ids[6]) && !set.contains(&ids[0]));
        let paths = set.finish().unwrap();
        let sizes: Vec<usize> = paths.iter().map(|p| objects_in(p)).collect();
        assert_eq!(sizes, [3, 3, 1]);
        // In the order they were made, so the last one holds what came last.
        assert!(
            PackReader::open(&paths[2])
                .unwrap()
                .find(&ids[6])
                .unwrap()
                .is_some()
        );
        assert!(is_empty_dir(&tmp));
    }

    #[test]
    fn a_pack_that_just_filled_is_not_followed_by_an_empty_one() {
        let (_root, tmp, packs) = dirs();
        let mut set = PackSet::with_limit(&tmp, &packs, 3).unwrap();
        add_bytes(&mut set, 6);
        assert!(!set.is_empty());
        assert_eq!(set.finish().unwrap().len(), 2);
        assert!(is_empty_dir(&tmp));
    }

    #[test]
    fn an_object_added_twice_counts_once_towards_the_limit() {
        let (_root, tmp, packs) = dirs();
        let mut set = PackSet::with_limit(&tmp, &packs, 2).unwrap();
        set.add_content(b"a").unwrap();
        set.add_content(b"a").unwrap();
        assert!(is_empty_dir(&packs));
        set.add_content(b"b").unwrap();
        assert_eq!(std::fs::read_dir(&packs).unwrap().count(), 1);
    }

    #[test]
    fn a_set_with_nothing_in_it_writes_no_pack() {
        let (_root, tmp, packs) = dirs();
        let set = PackSet::with_limit(&tmp, &packs, 3).unwrap();
        assert!(set.is_empty());
        assert!(set.finish().unwrap().is_empty());
        assert!(is_empty_dir(&tmp) && is_empty_dir(&packs));
    }

    #[test]
    fn streamed_content_goes_through_the_set() {
        let (_root, tmp, packs) = dirs();
        let mut set = PackSet::with_limit(&tmp, &packs, 2).unwrap();
        // Content that turns out different stores nothing, and the set carries on.
        let err = set
            .add_content_from(&mut &b"new"[..], hash_content(b"old"))
            .unwrap_err();
        assert!(matches!(err, PackError::ContentChanged), "{err:?}");
        assert!(set.is_empty());
        let data = b"streamed ".repeat(20_000);
        let size = set
            .add_content_from(&mut &data[..], hash_content(&data))
            .unwrap();
        assert_eq!(size, data.len() as u64);
        assert!(set.contains(&hash_content(&data)));
        set.add_record(ObjectKind::FolderListing, &Listing::empty().encode())
            .unwrap();
        // That was the second object, so the pack is in the store.
        let paths = set.finish().unwrap();
        assert_eq!(paths.len(), 1);
        assert_eq!(objects_in(&paths[0]), 2);
    }

    #[cfg(feature = "fault-injection")]
    #[test]
    fn a_crash_while_rolling_over_leaves_only_whole_packs() {
        use crate::fsio::fault;
        let mut rolled_over = false;
        for n in 1.. {
            assert!(n < 20, "the set never finished");
            let (_root, tmp, packs) = dirs();
            let mut set = PackSet::with_limit(&tmp, &packs, 2).unwrap();
            fault::arm(n);
            let result = match (0..5u8).try_for_each(|i| set.add_content(&[i]).map(drop)) {
                Ok(()) => set.finish(),
                Err(e) => {
                    // The objects of a pack that could not be moved are gone, so the set takes
                    // no more and cannot be finished.
                    assert!(set.add_content(b"more").is_err(), "fault {n}");
                    assert!(set.finish().is_err(), "fault {n}");
                    Err(e)
                }
            };
            fault::disarm();
            assert!(is_empty_dir(&tmp), "fault {n} left a temp file");
            match result {
                Err(PackError::Io(e)) => {
                    rolled_over |= e.to_string().ends_with("packs:rolled-over");
                    for pack in std::fs::read_dir(&packs).unwrap() {
                        assert!(objects_in(&pack.unwrap().path()) > 0, "fault {n}");
                    }
                }
                Ok(paths) => {
                    assert_eq!(paths.len(), 3);
                    break;
                }
                Err(other) => panic!("fault {n}: unexpected {other:?}"),
            }
        }
        assert!(rolled_over, "no fault point sat between two packs");
    }

    fn random(len: usize) -> Vec<u8> {
        let mut bytes = vec![0u8; len];
        getrandom::fill(&mut bytes).unwrap();
        bytes
    }

    /// A pack of `sizes` random contents, in that order, and its reader.
    fn random_pack(packs: &Path, tmp: &Path, sizes: &[usize]) -> (PathBuf, Vec<Vec<u8>>) {
        let mut w = PackWriter::create(tmp).unwrap();
        let contents: Vec<_> = sizes.iter().map(|&n| random(n)).collect();
        for c in &contents {
            w.add_content(c).unwrap();
        }
        (w.finish(packs).unwrap().unwrap(), contents)
    }

    #[test]
    fn reads_inside_a_block_are_answered_from_memory() {
        let (_root, tmp, packs) = dirs();
        let (path, contents) = random_pack(&packs, &tmp, &[100; 100]);
        let r = PackReader::open(&path).unwrap();
        let read_all = || {
            let before = open_files::reads();
            for c in &contents {
                let e = r.find(&hash_content(c)).unwrap().unwrap();
                assert!(read_any(&r, &e).unwrap() == *c);
            }
            open_files::reads() - before
        };
        // Small reads until enough is asked of each slot, then one block each.
        read_all();
        assert!(r.window.held().iter().all(|&(_, len)| len > 0));
        // Then every read is answered from memory.
        assert_eq!(read_all(), 0);
    }

    /// The reads of the file that 8 bytes at `pos`, read through the window, take.
    fn reads_for(r: &PackReader, pos: u64) -> usize {
        let before = open_files::reads();
        let mut got = [0u8; 8];
        Section {
            file: &r.file,
            window: Some(&r.window),
            pos,
            end: pos + 8,
        }
        .read_exact(&mut got)
        .unwrap();
        open_files::reads() - before
    }

    /// A reader of a pack whose entries fill more than five blocks.
    fn big_pack() -> (tempfile::TempDir, PackReader) {
        let (root, tmp, packs) = dirs();
        let sizes: Vec<usize> = (0..300).map(|i| 100 + i * 37 % 3000).collect();
        let (path, _) = random_pack(&packs, &tmp, &sizes);
        let r = PackReader::open(&path).unwrap();
        assert!(r.window.index_start > 5 * BLOCK);
        (root, r)
    }

    #[test]
    fn scattered_small_reads_never_read_a_block() {
        let (_root, r) = big_pack();
        for k in 0..5 {
            assert_eq!(reads_for(&r, k * BLOCK + 100), 1, "block {k}");
        }
        assert_eq!(r.window.held(), [(0, 0), (0, 0)]);
    }

    /// Asks `reads` small reads of 4 KiB of the entries' slot, never two in a row in one block.
    fn ask(r: &PackReader, reads: u64) {
        for i in 0..reads {
            let pos = (i % 2) * 4 * BLOCK;
            let mut got = vec![0u8; SMALL_READ as usize];
            let mut section = Section {
                file: &r.file,
                window: Some(&r.window),
                pos,
                end: pos + SMALL_READ,
            };
            section.read_exact(&mut got).unwrap();
        }
        assert_eq!(r.window.counts()[0], (reads * SMALL_READ, 0));
    }

    #[test]
    fn a_block_is_read_at_the_second_read_in_a_row_in_it() {
        let (_root, r) = big_pack();
        // Enough asked for two blocks.
        ask(&r, 8);
        // A small read, then the block, then nothing.
        let run: Vec<_> = [100, 200, 300].map(|d| reads_for(&r, BLOCK + d)).into();
        assert_eq!(run, [1, 1, 0]);
        assert_eq!(r.window.held()[0], (BLOCK, BLOCK as usize));
        // A read elsewhere is a small read and keeps the block; back in it, nothing. Elsewhere
        // again, in the same block as the last miss, reads that block, and then the first block
        // is a small read again.
        let (far, near) = (3 * BLOCK, BLOCK);
        let steps: Vec<_> = [far + 10, near + 400, far + 20, near + 500]
            .map(|pos| reads_for(&r, pos))
            .into();
        assert_eq!(steps, [1, 0, 1, 1]);
        assert_eq!(r.window.held()[0], (far, BLOCK as usize));
    }

    #[test]
    fn reads_in_pairs_read_blocks_only_within_their_share() {
        let (_root, r) = big_pack();
        let mut asked = 0;
        for round in 0..4000u64 {
            let start = (round % 5) * BLOCK;
            for d in [100, 200] {
                reads_for(&r, start + d);
                asked += 8;
            }
        }
        let (got_asked, read) = r.window.counts()[0];
        assert_eq!(got_asked, asked);
        assert!(read <= 4 * asked, "{read} read for {asked} asked");
        // The share was spent, not refused outright.
        assert!(read > 0);
    }

    #[test]
    fn side_by_side_reads_use_one_block_once_16_kib_is_asked() {
        let (_root, r) = big_pack();
        // 2,048 reads of 8 bytes ask 16 KiB: the last of them reads the block.
        let costs: Vec<usize> = (0..2048).map(|i| reads_for(&r, BLOCK + 8 * i)).collect();
        assert!(costs.iter().all(|&c| c == 1), "{costs:?}");
        assert_eq!(r.window.held()[0], (BLOCK, BLOCK as usize));
        assert_eq!(r.window.counts()[0], (16_384, BLOCK));
        for i in 2048..4096 {
            assert_eq!(reads_for(&r, BLOCK + 8 * i), 0, "read {i}");
        }
    }

    #[test]
    fn a_read_across_two_blocks_is_made_from_the_file() {
        let (_root, tmp, packs) = dirs();
        // The second entry starts 50 bytes before the first block ends.
        let filler = (BLOCK - 50 - PACK_HEADER_LEN - ENTRY_HEADER_LEN) as usize;
        let (path, contents) = random_pack(&packs, &tmp, &[filler, 200, 200]);
        let r = PackReader::open(&path).unwrap();
        let find = |c: &[u8]| r.find(&hash_content(c)).unwrap().unwrap();
        let (across, after) = (find(&contents[1]), find(&contents[2]));
        assert_eq!(across.offset, BLOCK - 50);
        // Twice, so that its block is held.
        for _ in 0..2 {
            assert!(read_any(&r, &after).unwrap() == contents[2]);
        }
        let held = r.window.held();
        assert_eq!(held[0].0, BLOCK);
        let before = open_files::reads();
        assert!(read_any(&r, &across).unwrap() == contents[1]);
        assert!(open_files::reads() > before);
        assert_eq!(r.window.held(), held);
    }

    #[test]
    fn a_big_read_is_made_from_the_file() {
        let (_root, tmp, packs) = dirs();
        let (path, contents) = random_pack(&packs, &tmp, &[100, 5000]);
        let r = PackReader::open(&path).unwrap();
        let find = |c: &[u8]| r.find(&hash_content(c)).unwrap().unwrap();
        let (small, big) = (find(&contents[0]), find(&contents[1]));
        // Enough times that its block is held.
        for _ in 0..50 {
            assert!(read_any(&r, &small).unwrap() == contents[0]);
        }
        let held = r.window.held();
        assert!(held[0].1 > 0);
        let before = open_files::reads();
        assert!(read_any(&r, &big).unwrap() == contents[1]);
        assert!(open_files::reads() > before);
        assert_eq!(r.window.held(), held);
    }

    #[test]
    fn a_window_never_changes_what_is_read() {
        let (_root, tmp, packs) = dirs();
        let sizes: Vec<usize> = (0..300).map(|i| 100 + i * 37 % 3000).collect();
        let (path, _) = random_pack(&packs, &tmp, &sizes);
        let file = std::fs::read(&path).unwrap();
        let len = file.len() as u64;
        assert!(len > 300_000, "{len}");
        let r = PackReader::open(&path).unwrap();
        use proptest::prelude::*;
        // A third near a block's edge, a sixth near the file's end, the rest anywhere.
        let start = prop_oneof![
            2 => (0..=len / BLOCK, 0..128u64)
                .prop_map(move |(k, d)| (k * BLOCK + d).saturating_sub(64).min(len - 1)),
            1 => (len - 8192)..len,
            3 => 0..len,
        ];
        let reads = proptest::collection::vec((start, 1..=2 * SMALL_READ), 1..64);
        let mut runner = proptest::test_runner::TestRunner::default();
        runner
            .run(&reads, |reads| {
                // Half the reads on each of two threads sharing the reader.
                std::thread::scope(|s| {
                    for half in [0, 1] {
                        let (r, file, reads) = (&r, &file, &reads);
                        s.spawn(move || {
                            for &(pos, n) in reads.iter().skip(half).step_by(2) {
                                // A read may run past the file's end: both must fail alike.
                                let read = |window| {
                                    let mut got = vec![0u8; n as usize];
                                    let mut section = Section {
                                        file: &r.file,
                                        window,
                                        pos,
                                        end: pos + n,
                                    };
                                    section.read_exact(&mut got).map(|()| got)
                                };
                                match (read(Some(&r.window)), read(None)) {
                                    (Ok(got), Ok(straight)) => {
                                        let end = (pos + n) as usize;
                                        assert!(got == straight, "{pos} {n}");
                                        assert!(got == file[pos as usize..end], "{pos} {n}");
                                    }
                                    (Err(a), Err(b)) => {
                                        assert_eq!(a.kind(), b.kind(), "{pos} {n}");
                                        assert!(pos + n > len, "{pos} {n}: {a}");
                                    }
                                    (a, b) => panic!("{pos} {n}: {a:?} against {b:?}"),
                                }
                            }
                        });
                    }
                });
                Ok(())
            })
            .unwrap();
    }

    #[test]
    fn a_pack_shorter_than_its_index_says_is_corrupt_or_cut_short() {
        let (_root, tmp, packs) = dirs();
        let (path, contents) = random_pack(&packs, &tmp, &[1000, 1000, 1000]);
        let last = hash_content(&contents[2]);
        let entry = PackReader::open(&path)
            .unwrap()
            .find(&last)
            .unwrap()
            .unwrap();
        let whole = std::fs::read(&path).unwrap();
        let read_cut = |cut: u64| {
            std::fs::write(&path, &whole).unwrap();
            let r = PackReader::open(&path).unwrap();
            let file = OpenOptions::new().write(true).open(&path).unwrap();
            file.set_len(cut).unwrap();
            read_any(&r, &entry).unwrap_err()
        };
        let in_data = entry.offset + ENTRY_HEADER_LEN + 500;
        match read_cut(in_data) {
            PackError::Corrupt { reason, .. } => {
                assert_eq!(reason, format!("{last} does not have its recorded size"));
            }
            other => panic!("{other:?}"),
        }
        match read_cut(entry.offset + 10) {
            PackError::Io(e) => assert_eq!(e.kind(), io::ErrorKind::UnexpectedEof, "{e}"),
            other => panic!("{other:?}"),
        }
    }
}
