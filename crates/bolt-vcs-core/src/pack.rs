//! Pack files: the container every stored object lives in (docs/storage-format.md section 11).

use crate::chunks::{ChunkList, ReadListError};
use crate::encoding::{DecodeError, Reader, Writer};
use crate::fsio;
use crate::id::{ContentHasher, Flavor, Id, hash_content, hash_record};
use crate::safe_fs;
use crate::text::escape;
use bolt_platform::dir::Dir;
use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::fs::{File, OpenOptions};
use std::io::{self, BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
#[cfg(any(test, feature = "fault-injection"))]
use std::sync::atomic::{AtomicU64, Ordering};

mod v2;

pub const PACK_MAGIC: [u8; 8] = *b"BOLTPACK";
pub const PACK_END_MAGIC: [u8; 8] = *b"BOLTPEND";
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
    // The path has the project folder's name in it, which came from outside Bolt.
    #[error(
        "pack {} is damaged: {reason}",
        escape(&path.display().to_string(), false)
    )]
    Corrupt { path: PathBuf, reason: String },
    #[error("object {id} does not match its fingerprint")]
    FingerprintMismatch { id: Id },
    #[error("the file changed while it was being stored")]
    ContentChanged,
    #[error("the object is larger than Bolt can store")]
    TooLarge,
    // Not damage: the same pack reads fine with more memory.
    #[error("Bolt ran out of memory while reading the project's data")]
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
        };
        w.file.write_all(&PACK_MAGIC)?;
        w.file.write_all(&PACK_VERSION.to_le_bytes())?;
        w.file.write_all(&0u32.to_le_bytes())?;
        Ok(w)
    }

    /// Stores an encoded structured record (any kind but `Content`).
    pub fn add_record(&mut self, kind: ObjectKind, encoded: &[u8]) -> Result<Id, PackError> {
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

/// Most objects Bolt's writer puts in one pack. The format allows more, but a writer holds the
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

    /// Stores an encoded structured record (any kind but `Content`).
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

        let header = read_vec(&file, 0, PACK_HEADER_LEN)?;
        if sub::<8>(&header, 0) != PACK_MAGIC {
            return Err(corrupt("the file does not start with BOLTPACK".into()));
        }
        let version = u32::from_le_bytes(sub(&header, 8));
        if !matches!(version, 1 | 2) || u32::from_le_bytes(sub(&header, 12)) != 0 {
            return Err(corrupt("unknown pack version or flags".into()));
        }

        let trailer = read_vec(&file, len - TRAILER_LEN, TRAILER_LEN)?;
        let (index_offset, checksum, end_magic) = (
            u64::from_le_bytes(sub(&trailer, 0)),
            sub::<32>(&trailer, 8),
            sub::<8>(&trailer, 40),
        );
        if end_magic != PACK_END_MAGIC {
            return Err(corrupt("the file does not end with BOLTPEND".into()));
        }
        if index_offset < PACK_HEADER_LEN || index_offset > len - TRAILER_LEN {
            return Err(corrupt("the index offset is outside the file".into()));
        }
        let index_end = len - TRAILER_LEN;
        let index = if version == 1 {
            let index = Section {
                file: &file,
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
            Index::OnDisk(index) => index.find(&self.file, &self.path, id),
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
        let mut out = Vec::new();
        out.try_reserve_exact(
            usize::try_from(entry.uncompressed_size).map_err(|_| PackError::TooLarge)?,
        )
        .map_err(|_| PackError::OutOfMemory)?;
        self.decode_into(entry, &mut out)?;
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
        let header = read_vec(&self.file, entry.offset, ENTRY_HEADER_LEN)?;
        let mut r = Reader::new(&header);
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
        let data_start = entry.offset + ENTRY_HEADER_LEN;
        let data = Section {
            file: &self.file,
            pos: data_start,
            end: data_start
                .checked_add(stored)
                .ok_or_else(|| self.corrupt("an entry runs past the end of the file"))?,
        };
        Ok((compression, data))
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
                let frame_header =
                    read_vec(&self.file, data_start, stored.min(ZSTD_MAX_FRAME_HEADER))?;
                match zstd::zstd_safe::get_frame_content_size(&frame_header) {
                    Ok(None) => {}
                    Ok(Some(n)) if n == size => {}
                    _ => {
                        return Err(
                            self.corrupt(format!("the zstd frame of {id} has the wrong size"))
                        );
                    }
                }
                // Making a decoder fails only when memory runs out.
                let decoder =
                    zstd::stream::read::Decoder::new(data).map_err(|_| PackError::OutOfMemory)?;
                copy_decoded(&mut decoder.take(size.saturating_add(1)), out).map_err(|e| {
                    if zstd_ran_out_of_memory(&e) {
                        PackError::OutOfMemory
                    } else {
                        self.corrupt(format!("the zstd data of {id} is invalid: {e}"))
                    }
                })??
            }
            _ => return Err(self.corrupt(format!("{id} has an unknown compression"))),
        };
        if produced != size {
            return Err(self.corrupt(format!("{id} does not have its recorded size")));
        }
        Ok(())
    }
}

/// How many packs this thread has opened, and how many it held open at once: tests of the store's
/// limit on open files read them.
#[cfg(test)]
pub(crate) mod open_files {
    use std::cell::Cell;
    use std::sync::atomic::{AtomicUsize, Ordering};

    thread_local! {
        static OPENED: Cell<usize> = const { Cell::new(0) };
        static OPEN: Cell<usize> = const { Cell::new(0) };
        static MOST: Cell<usize> = const { Cell::new(0) };
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
fn read_vec(file: &File, offset: u64, len: u64) -> Result<Vec<u8>, PackError> {
    let len = usize::try_from(len).map_err(|_| PackError::TooLarge)?;
    let mut out = Vec::new();
    out.try_reserve_exact(len)
        .map_err(|_| PackError::TooLarge)?;
    out.resize(len, 0);
    Section {
        file,
        pos: offset,
        end: offset.saturating_add(len as u64),
    }
    .read_exact(&mut out)?;
    Ok(out)
}

/// A byte range of a file, read with positional reads so readers never share a cursor.
struct Section<'a> {
    file: &'a File,
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
        let n = safe_fs::read_at(self.file, &mut buf[..want], self.pos)?;
        self.pos += n as u64;
        Ok(n)
    }
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
}
