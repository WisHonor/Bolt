//! Pack files: the container every stored object lives in (docs/storage-format.md section 11).

use crate::encoding::{Reader, Writer};
use crate::fsio;
use crate::id::{ContentHasher, Flavor, Id, hash_content, hash_record};
use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{self, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

pub const PACK_MAGIC: [u8; 8] = *b"BOLTPACK";
pub const PACK_END_MAGIC: [u8; 8] = *b"BOLTPEND";
pub const PACK_VERSION: u32 = 1;
/// Largest uncompressed object (1 TiB).
pub const MAX_OBJECT_SIZE: u64 = 1 << 40;
/// Largest uncompressed structured record (512 MiB).
pub const MAX_RECORD_SIZE: u64 = 512 << 20;
/// kind, id, uncompressed size, stored size, compression.
pub const ENTRY_HEADER_LEN: u64 = 50;

const PACK_HEADER_LEN: u64 = 16;
/// Index offset, index checksum, end magic.
const TRAILER_LEN: u64 = 48;
/// Smallest possible index line: id, kind, offset, two one-byte varints.
const MIN_INDEX_LINE: u64 = 43;
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
        })
    }

    fn max_size(self) -> u64 {
        if self == ObjectKind::Content {
            MAX_OBJECT_SIZE
        } else {
            MAX_RECORD_SIZE
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
    #[error("pack {} is damaged: {reason}", path.display())]
    Corrupt { path: PathBuf, reason: String },
    #[error("object {id} does not match its fingerprint")]
    FingerprintMismatch { id: Id },
    #[error("the file changed while it was being stored")]
    ContentChanged,
    #[error("the object is larger than Bolt can store")]
    TooLarge,
}

fn misuse(what: &'static str) -> PackError {
    PackError::Io(io::Error::new(io::ErrorKind::InvalidInput, what))
}

/// Compressed data is kept only if it is smaller than 95% of the original.
fn worth_compressing(compressed: usize, original: usize) -> bool {
    (compressed as u64) * 20 < (original as u64) * 19
}

/// Builds a pack in the temp folder; `finish` moves it into the store.
pub struct PackWriter {
    file: BufWriter<File>,
    tmp_path: PathBuf,
    /// Where the next entry starts.
    end: u64,
    index: BTreeMap<Id, IndexEntry>,
    /// Set when a failed entry could not be undone; the pack can then not be finished.
    broken: bool,
}

impl PackWriter {
    pub fn create(tmp_dir: &Path) -> Result<PackWriter, PackError> {
        let (file, tmp_path) = fsio::create_temp(tmp_dir)?;
        let mut w = PackWriter {
            file: BufWriter::new(file),
            tmp_path,
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

    #[must_use]
    pub fn contains(&self, id: &Id) -> bool {
        self.index.contains_key(id)
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.index.is_empty()
    }

    /// Writes the index and trailer, flushes, and moves the pack into `packs_dir` under the
    /// name `<hex of the index checksum>.pack`. An empty pack is deleted instead.
    pub fn finish(mut self, packs_dir: &Path) -> Result<Option<PathBuf>, PackError> {
        if self.broken {
            return Err(io::Error::other("an earlier write to this pack failed").into());
        }
        if self.index.is_empty() {
            fs::remove_file(&self.tmp_path)?;
            return Ok(None);
        }
        let mut index = Writer::new();
        index.varint(self.index.len() as u64);
        for e in self.index.values() {
            index.id(&e.id);
            index.u8(e.kind as u8);
            index.u64(e.offset);
            index.varint(e.uncompressed_size);
            index.varint(e.stored_size);
        }
        let index = index.into_bytes();
        let checksum = blake3::hash(&index);
        self.file.write_all(&index)?;
        self.file.write_all(&self.end.to_le_bytes())?;
        self.file.write_all(checksum.as_bytes())?;
        self.file.write_all(&PACK_END_MAGIC)?;
        self.file.flush()?;
        self.file.get_ref().sync_all()?;
        fsio::fault_point("pack:before-rename")?;
        let dest = packs_dir.join(format!("{}.pack", checksum.to_hex()));
        fsio::rename_durable(&self.tmp_path, &dest)?;
        fsio::fault_point("pack:after-rename")?;
        Ok(Some(dest))
    }

    fn add_whole(&mut self, kind: ObjectKind, id: Id, data: &[u8]) -> Result<(), PackError> {
        if self.contains(&id) {
            return Ok(());
        }
        let compressed = zstd::bulk::compress(data, ZSTD_LEVEL)?;
        let (compression, stored) = if worth_compressing(compressed.len(), data.len()) {
            (ZSTD, &compressed[..])
        } else {
            (NO_COMPRESSION, data)
        };
        self.add_entry(kind, id, |file| {
            file.write_all(stored)?;
            Ok((data.len() as u64, compression, id))
        })?;
        Ok(())
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
        let start = self.end;
        let result = (|| {
            self.file.write_all(&[0; ENTRY_HEADER_LEN as usize])?;
            let (size, compression, actual) = body(&mut self.file)?;
            if actual != id {
                return Err(PackError::ContentChanged);
            }
            if size > kind.max_size() {
                return Err(PackError::TooLarge);
            }
            let data_end = self.file.stream_position()?;
            let entry = IndexEntry {
                id,
                kind,
                offset: start,
                uncompressed_size: size,
                stored_size: data_end - start - ENTRY_HEADER_LEN,
            };
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
        let _ = fs::remove_file(&self.tmp_path);
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
    entries: Vec<IndexEntry>,
}

impl PackReader {
    /// Opens a pack and checks its header, trailer, index checksum and that every entry lies
    /// inside the file without overlapping another.
    pub fn open(path: &Path) -> Result<PackReader, PackError> {
        let file = File::open(path)?;
        let corrupt = |reason: String| PackError::Corrupt {
            path: path.to_path_buf(),
            reason,
        };
        let len = file.metadata()?.len();
        if len < PACK_HEADER_LEN + TRAILER_LEN {
            return Err(corrupt("the file is too short to be a pack".into()));
        }

        let header = read_vec(&file, 0, PACK_HEADER_LEN)?;
        if sub::<8>(&header, 0) != PACK_MAGIC {
            return Err(corrupt("the file does not start with BOLTPACK".into()));
        }
        if u32::from_le_bytes(sub(&header, 8)) != PACK_VERSION
            || u32::from_le_bytes(sub(&header, 12)) != 0
        {
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
        let index_len = len - TRAILER_LEN - index_offset;
        let index = read_vec(&file, index_offset, index_len)?;
        if *blake3::hash(&index).as_bytes() != checksum {
            return Err(corrupt("the index checksum does not match".into()));
        }

        let entries = parse_index(&index, index_offset)
            .map_err(|reason| corrupt(format!("index: {reason}")))?;
        Ok(PackReader {
            file,
            path: path.to_path_buf(),
            entries,
        })
    }

    /// All entries, sorted by id.
    #[must_use]
    pub fn entries(&self) -> &[IndexEntry] {
        &self.entries
    }

    #[must_use]
    pub fn find(&self, id: &Id) -> Option<&IndexEntry> {
        self.entries
            .binary_search_by(|e| e.id.cmp(id))
            .ok()
            .map(|i| &self.entries[i])
    }

    /// Reads a structured record and checks its fingerprint.
    pub fn read_record(&self, entry: &IndexEntry) -> Result<Vec<u8>, PackError> {
        let flavor = entry
            .kind
            .flavor()
            .ok_or_else(|| misuse("read_record cannot read content"))?;
        let mut out = Vec::new();
        out.try_reserve_exact(
            usize::try_from(entry.uncompressed_size).map_err(|_| PackError::TooLarge)?,
        )
        .map_err(|_| PackError::TooLarge)?;
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

    fn corrupt(&self, reason: impl Into<String>) -> PackError {
        PackError::Corrupt {
            path: self.path.clone(),
            reason: reason.into(),
        }
    }

    /// Checks the entry's header against `entry`, then writes its uncompressed bytes to `out`,
    /// never producing more than one byte beyond the recorded size.
    fn decode_into(&self, entry: &IndexEntry, out: &mut dyn Write) -> Result<(), PackError> {
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
        let mut data = Section {
            file: &self.file,
            pos: data_start,
            end: data_start
                .checked_add(stored)
                .ok_or_else(|| self.corrupt("an entry runs past the end of the file"))?,
        };
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
                let decoder = zstd::stream::read::Decoder::new(data)?;
                copy_decoded(&mut decoder.take(size.saturating_add(1)), out)
                    .map_err(|e| self.corrupt(format!("the zstd data of {id} is invalid: {e}")))??
            }
            _ => return Err(self.corrupt(format!("{id} has an unknown compression"))),
        };
        if produced != size {
            return Err(self.corrupt(format!("{id} does not have its recorded size")));
        }
        Ok(())
    }
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

/// Parses and checks the index; `index_offset` is where the entry area ends.
fn parse_index(index: &[u8], index_offset: u64) -> Result<Vec<IndexEntry>, String> {
    let mut r = Reader::new(index);
    let count = r
        .count(index.len() as u64 / MIN_INDEX_LINE, "entry count")
        .map_err(|e| e.to_string())?;
    let mut entries: Vec<IndexEntry> = Vec::new();
    for _ in 0..count {
        let id = r.id().map_err(|e| e.to_string())?;
        let kind = r.u8().map_err(|e| e.to_string())?;
        let kind =
            ObjectKind::from_u8(kind).ok_or_else(|| format!("unknown object kind {kind}"))?;
        let offset = r.u64().map_err(|e| e.to_string())?;
        let uncompressed_size = r.varint().map_err(|e| e.to_string())?;
        let stored_size = r.varint().map_err(|e| e.to_string())?;
        if entries.last().is_some_and(|prev| prev.id >= id) {
            return Err("entries are not sorted by id, or repeat one".into());
        }
        if uncompressed_size > kind.max_size() {
            return Err(format!("{id} is larger than allowed"));
        }
        let end = offset
            .checked_add(ENTRY_HEADER_LEN)
            .and_then(|o| o.checked_add(stored_size));
        if offset < PACK_HEADER_LEN || end.is_none_or(|end| end > index_offset) {
            return Err(format!("{id} lies outside the entry area"));
        }
        entries.push(IndexEntry {
            id,
            kind,
            offset,
            uncompressed_size,
            stored_size,
        });
    }
    r.finish().map_err(|e| e.to_string())?;

    let mut by_offset: Vec<&IndexEntry> = entries.iter().collect();
    by_offset.sort_by_key(|e| e.offset);
    // Offsets and sizes were range-checked above, so these sums cannot overflow.
    if by_offset
        .windows(2)
        .any(|p| p[0].offset + ENTRY_HEADER_LEN + p[0].stored_size > p[1].offset)
    {
        return Err("two entries overlap".into());
    }
    Ok(entries)
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
        #[cfg(unix)]
        let n = std::os::unix::fs::FileExt::read_at(self.file, &mut buf[..want], self.pos)?;
        #[cfg(windows)]
        let n = std::os::windows::fs::FileExt::seek_read(self.file, &mut buf[..want], self.pos)?;
        self.pos += n as u64;
        Ok(n)
    }
}

/// Passes writes through while hashing them.
struct HashingWriter<'a> {
    inner: &'a mut dyn Write,
    hasher: ContentHasher,
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
        assert_eq!(r.entries().len(), 5);
        assert!(r.entries().windows(2).all(|p| p[0].id < p[1].id));
        for e in r.entries() {
            assert_eq!(read_any(&r, e).unwrap(), expected[&e.id]);
            assert_eq!(r.find(&e.id).unwrap().offset, e.offset);
        }
        assert!(is_empty_dir(&tmp));
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
        assert_eq!(r.entries().len(), 1);
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
                    let mut failed = false;
                    for e in r.entries() {
                        match read_any(&r, e) {
                            Ok(data) => assert_eq!(
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

    /// A pack with one content entry whose header and index claim `claimed` uncompressed bytes.
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
        out.extend_from_slice(&PACK_VERSION.to_le_bytes());
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
            let err = r.copy_content(&r.entries()[0], &mut out).unwrap_err();
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
        assert_eq!(r.entries().len(), 1);
        assert_eq!(read_any(&r, &r.entries()[0]).unwrap(), b"x");
        assert_eq!(r.entries()[0].id, x);
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
        assert!(r.entries()[0].stored_size < size);
        assert_eq!(read_any(&r, &r.entries()[0]).unwrap(), data);
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
                    assert_eq!(PackReader::open(&p.path()).unwrap().entries().len(), 1);
                }
                (3, Ok(Some(_))) => break,
                (n, other) => panic!("fault {n}: unexpected {other:?}"),
            }
        }
    }
}
