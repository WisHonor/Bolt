//! Reading Git's objects, loose or packed, whole or through deltas (decision 031). Sizes are
//! checked before anything is allocated, zlib streams inflate through fixed buffers, and every
//! object is checked against its ID.
#![deny(clippy::indexing_slicing, clippy::arithmetic_side_effects)]

use std::borrow::{Borrow, Cow};
use std::cell::{OnceCell, RefCell};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt;
use std::fs::{File, OpenOptions};
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use miniz_oxide::inflate::stream::{InflateState, inflate};
use miniz_oxide::{DataFormat, MZError, MZFlush, MZStatus};

use super::id::Hasher;
use super::pack::{self, Entry, EntryKind, Pack, PackFolder, apply_delta, delta_sizes};
use super::{GitError, GitId, ObjectType, Repo, broken, read_error, real_dir, shrank};
use crate::safe_fs;

/// The most an object read whole into memory may hold, and so the most a delta may build or
/// build on. Git never deltifies files over 512 MiB unless told to (`core.bigFileThreshold`).
pub const MAX_IN_MEMORY_OBJECT: u64 = 512 << 20;
/// The most deltas one object may be built through; Git's own cap is 4,095.
pub const MAX_DELTA_DEPTH: u32 = 4096;
/// The most the bases kept for later deltas may hold.
pub const DELTA_BASE_CACHE: u64 = 256 << 20;
/// The most a loose object may say it holds: 1 TiB.
const MAX_LOOSE_SIZE: u64 = 1 << 40;
/// A loose object's header fits in this many bytes with its NUL, as Git requires.
const MAX_HEADER_LEN: usize = 32;
/// Compressed bytes read at a time, and inflated ones handed on.
const CHUNK: usize = 16 << 10;
/// Packs kept open between reads, the last ones used: few enough for any system's limit on open
/// files.
const MAX_OPEN_PACKS: usize = 16;

/// An object read whole.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Object {
    pub kind: ObjectType,
    pub data: Vec<u8>,
}

/// What a repository reads its objects with: its packs, listed when first needed, those kept
/// open, and the bases kept for deltas.
#[derive(Default)]
pub(super) struct Objects {
    packs: OnceCell<PackFolder>,
    /// Packs opened for reading, by number, the most recently used first.
    open: RefCell<Vec<(usize, File)>>,
    cache: RefCell<BaseCache>,
}

impl fmt::Debug for Objects {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Objects").finish_non_exhaustive()
    }
}

/// Where an object is.
enum Place<'r> {
    /// In the repository's pack number `.0`, at an offset.
    Packed(usize, &'r Pack, u64),
    Loose(Loose),
}

impl Repo {
    /// An object's type and size, as its header says, without reading its data.
    pub fn header(&self, id: &GitId) -> Result<(ObjectType, u64), GitError> {
        match self.find(id)? {
            Place::Loose(loose) => Ok((loose.kind, loose.size)),
            Place::Packed(n, pack, at) => {
                let file = self.open_pack(n, pack)?;
                let header = packed_header(pack, &file, at);
                self.keep_open(n, file);
                header
            }
        }
    }

    /// An object whole, at most `MAX_IN_MEMORY_OBJECT` bytes, checked against its ID.
    pub fn read(&self, id: &GitId) -> Result<Object, GitError> {
        let object = match self.find(id)? {
            Place::Loose(loose) => loose.read()?,
            Place::Packed(n, pack, at) => {
                let file = self.open_pack(n, pack)?;
                let object = self.unpack(n, pack, &file, at);
                self.keep_open(n, file);
                object?
            }
        };
        self.check(id, &object)?;
        Ok(object)
    }

    /// A blob's bytes. A blob Git stored whole is inflated as it is read and checked against its
    /// ID as it ends; a deltified one is rebuilt in memory and checked first.
    pub fn blob_reader(&self, id: &GitId) -> Result<BlobReader<'_>, GitError> {
        let (inflater, size) = match self.find(id)? {
            Place::Loose(loose) => {
                is_blob(id, loose.kind)?;
                (loose.stream, loose.size)
            }
            Place::Packed(n, pack, at) => {
                let file = self.open_pack(n, pack)?;
                let entry = pack.entry(&file, at)?;
                let EntryKind::Whole(kind) = entry.kind else {
                    let object = self.unpack(n, pack, &file, at);
                    self.keep_open(n, file);
                    let object = object?;
                    self.check(id, &object)?;
                    is_blob(id, object.kind)?;
                    return Ok(BlobReader(Reading::Memory(io::Cursor::new(object.data))));
                };
                is_blob(id, kind)?;
                // The stream reads through this handle of its own until it is dropped.
                let input = Input::new(file, pack.path.as_path(), entry.data, pack.entries_end());
                (Inflater::new(input, Some(at)), entry.size)
            }
        };
        Ok(BlobReader(Reading::Stream(Box::new(Stream {
            inflater,
            hasher: Hasher::object(self.hash, ObjectType::Blob, size),
            id: *id,
            size,
            left: size,
        }))))
    }

    /// The repository's packs, listed and checked against their indexes when first needed.
    fn packs(&self) -> Result<&PackFolder, GitError> {
        if let Some(folder) = self.objects.packs.get() {
            return Ok(folder);
        }
        let folder = pack::load(&self.pack_dir(), self.hash)?;
        Ok(self.objects.packs.get_or_init(|| folder))
    }

    fn pack_dir(&self) -> PathBuf {
        self.git.join("objects").join("pack")
    }

    /// Pack `n`, opened for reading: kept open from an earlier read if it was, so most reads
    /// don't open it again.
    fn open_pack(&self, n: usize, pack: &Pack) -> Result<File, GitError> {
        if let Ok(mut open) = self.objects.open.try_borrow_mut()
            && let Some(i) = open.iter().position(|(m, _)| *m == n)
        {
            return Ok(open.remove(i).1);
        }
        pack.open()
    }

    /// Keeps pack `n` open for the reads after, closing the least recently used past
    /// `MAX_OPEN_PACKS`.
    fn keep_open(&self, n: usize, file: File) {
        if let Ok(mut open) = self.objects.open.try_borrow_mut() {
            open.insert(0, (n, file));
            open.truncate(MAX_OPEN_PACKS);
        }
    }

    /// Where `id` is: in the first pack that holds it, as Git looks, or else in its loose file.
    fn find(&self, id: &GitId) -> Result<Place<'_>, GitError> {
        let folder = self.packs()?;
        for (n, pack) in folder.packs.iter().enumerate() {
            if let Some(at) = pack.find(id) {
                return Ok(Place::Packed(n, pack, at));
            }
        }
        let hex = id.to_hex();
        if let Some((folder, file)) = hex.split_at_checked(2) {
            let folder = self.git.join("objects").join(folder);
            if real_dir(&folder)?
                && let Some(loose) = Loose::open(folder.join(file))?
            {
                return Ok(Place::Loose(loose));
            }
        }
        // Git may have packed it since Bolt listed the packs: then the list is different now.
        if !pack::list(&self.pack_dir()).is_ok_and(|now| now == folder.listed) {
            return Err(GitError::Changed {
                what: format!("the packs in {} changed", self.pack_dir().display()),
            });
        }
        Err(broken(format!("missing object `{hex}`")))
    }

    /// The object at `at` in pack number `n`, rebuilt through its deltas. A base comes from the
    /// cache if it is there, and goes into it once built on.
    fn unpack(&self, n: usize, pack: &Pack, file: &File, at: u64) -> Result<Object, GitError> {
        let cache = &self.objects.cache;
        let chain = chain(pack, file, at, &mut |base| {
            cache.try_borrow_mut().ok()?.take((n, base))
        })?;
        let mut base = match chain.bottom {
            Bottom::Cached(base) => base,
            Bottom::Whole(kind, entry) => Base {
                kind,
                data: inflate_entry(pack, file, chain.bottom_at, &entry)?,
                depth: 0,
            },
        };
        let mut base_at = chain.bottom_at;
        for (delta_at, entry) in chain.deltas.into_iter().rev() {
            let delta = inflate_entry(pack, file, delta_at, &entry)?;
            let built = apply_delta(&base.data, &delta).map_err(|why| {
                broken(format!(
                    "the delta at offset {delta_at} in {} {why}",
                    pack.path.display()
                ))
            })?;
            let next = Base {
                kind: base.kind,
                data: built,
                depth: base.depth.saturating_add(1),
            };
            let done = std::mem::replace(&mut base, next);
            if let Ok(mut cache) = cache.try_borrow_mut() {
                cache.put((n, base_at), done);
            }
            base_at = delta_at;
        }
        Ok(Object {
            kind: base.kind,
            data: base.data,
        })
    }

    /// Checks that `object` is the one `id` names.
    fn check(&self, id: &GitId, object: &Object) -> Result<(), GitError> {
        let mut hasher = Hasher::object(self.hash, object.kind, object.data.len() as u64);
        hasher.update(&object.data);
        verify(id, hasher)
    }
}

/// The type and size of the object at `at` in `pack`, as its entry says; for a delta, what it
/// says it builds, and the type at the bottom of its chain.
fn packed_header(pack: &Pack, file: &File, at: u64) -> Result<(ObjectType, u64), GitError> {
    let entry = pack.entry(file, at)?;
    if let EntryKind::Whole(kind) = entry.kind {
        return Ok((kind, entry.size));
    }
    let size = built_size(pack, file, at, &entry)?;
    let kind = match chain(pack, file, at, &mut |_| None)?.bottom {
        Bottom::Whole(kind, _) | Bottom::Cached(Base { kind, .. }) => kind,
    };
    Ok((kind, size))
}

/// Checks that what `hasher` took is the object `id` names.
fn verify(id: &GitId, hasher: Hasher) -> Result<(), GitError> {
    let actual = hasher.finish(|| format!("object `{}`", id.to_hex()))?;
    if actual == *id {
        return Ok(());
    }
    Err(broken(format!(
        "object `{}` doesn't match its ID: what Git stored under it hashes to `{}`",
        id.to_hex(),
        actual.to_hex()
    )))
}

fn is_blob(id: &GitId, kind: ObjectType) -> Result<(), GitError> {
    if kind == ObjectType::Blob {
        return Ok(());
    }
    Err(broken(format!(
        "object `{}` is a {}, not a file's contents",
        id.to_hex(),
        kind.name()
    )))
}

/// The deltas from an entry down to what they build on, the outermost first.
struct Chain {
    deltas: Vec<(u64, Entry)>,
    bottom_at: u64,
    bottom: Bottom,
}

/// What a chain of deltas builds on.
enum Bottom {
    /// An object stored whole, and its entry.
    Whole(ObjectType, Entry),
    Cached(Base),
}

/// An object rebuilt for deltas to build on.
#[derive(Debug, PartialEq, Eq)]
struct Base {
    kind: ObjectType,
    data: Vec<u8>,
    /// How many deltas it was built through.
    depth: u32,
}

/// Follows the deltas from the entry at `at` to an object stored whole, or to a base `cached`
/// gives, without recursion: at most `MAX_DELTA_DEPTH` deltas in all, the cached base's
/// counted too, and never round a loop.
fn chain(
    pack: &Pack,
    file: &File,
    at: u64,
    cached: &mut dyn FnMut(u64) -> Option<Base>,
) -> Result<Chain, GitError> {
    let shown = pack.path.display();
    let too_deep = || {
        broken(format!(
            "the object at offset {at} in {shown} is more than {MAX_DELTA_DEPTH} deltas deep"
        ))
    };
    let mut deltas = Vec::new();
    let mut seen = HashSet::new();
    let mut here = at;
    loop {
        if let Some(base) = cached(here) {
            let depth = u32::try_from(deltas.len()).unwrap_or(u32::MAX);
            if depth.saturating_add(base.depth) > MAX_DELTA_DEPTH {
                return Err(too_deep());
            }
            return Ok(Chain {
                deltas,
                bottom_at: here,
                bottom: Bottom::Cached(base),
            });
        }
        if !seen.insert(here) {
            return Err(broken(format!(
                "the deltas that build the object at offset {at} in {shown} go round in a loop"
            )));
        }
        let entry = pack.entry(file, here)?;
        let base = match entry.kind {
            EntryKind::Whole(kind) => {
                return Ok(Chain {
                    deltas,
                    bottom_at: here,
                    bottom: Bottom::Whole(kind, entry),
                });
            }
            EntryKind::OfsDelta(base) => base,
            EntryKind::RefDelta(id) => pack.find(&id).ok_or_else(|| {
                broken(format!(
                    "the delta at offset {here} in {shown} builds on `{}`, which isn't in that \
                     pack",
                    id.to_hex()
                ))
            })?,
        };
        if deltas.len() >= MAX_DELTA_DEPTH as usize {
            return Err(too_deep());
        }
        deltas.push((here, entry));
        here = base;
    }
}

/// What the delta at `at` says it builds, read from the start of its data.
fn built_size(pack: &Pack, file: &File, at: u64, entry: &Entry) -> Result<u64, GitError> {
    let input = Input::new(file, pack.path.as_path(), entry.data, pack.entries_end());
    let mut stream = Inflater::new(input, Some(at));
    // Two sizes of at most ten bytes each.
    let mut start = [0u8; 20];
    let want = start
        .len()
        .min(usize::try_from(entry.size).unwrap_or(usize::MAX));
    let mut got = 0;
    while let Some(rest) = start.get_mut(got..want).filter(|rest| !rest.is_empty()) {
        match stream.read(rest)? {
            0 => break,
            more => got = got.saturating_add(more),
        }
    }
    start
        .get(..got)
        .and_then(delta_sizes)
        .map(|(_, size)| size)
        .ok_or_else(|| {
            broken(format!(
                "the delta at offset {at} in {} starts with a damaged size",
                pack.path.display()
            ))
        })
}

/// An entry's data inflated whole: exactly the size its header gives, at most
/// `MAX_IN_MEMORY_OBJECT`.
fn inflate_entry(pack: &Pack, file: &File, at: u64, entry: &Entry) -> Result<Vec<u8>, GitError> {
    let input = Input::new(file, pack.path.as_path(), entry.data, pack.entries_end());
    Inflater::new(input, Some(at)).read_all(entry.size)
}

/// A loose object with its header read: the rest of its stream is its data.
struct Loose {
    stream: Inflater<'static, File>,
    kind: ObjectType,
    size: u64,
}

impl Loose {
    /// Opens the loose object at `path` and reads its header; `None` if there's no such file.
    fn open(path: PathBuf) -> Result<Option<Loose>, GitError> {
        let file = match safe_fs::open_regular(&path, OpenOptions::new().read(true)) {
            Ok(file) => file,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(read_error(&path, e)),
        };
        let len = file.metadata().map_err(|e| read_error(&path, e))?.len();
        let mut stream = Inflater::new(Input::new(file, path, 0, len), None);
        // A byte at a time, so none of the data comes out with it.
        let mut header = Vec::with_capacity(MAX_HEADER_LEN);
        loop {
            let mut byte = [0u8];
            if stream.read(&mut byte)? == 0 {
                return Err(broken(format!("{} ends inside its header", stream.what())));
            }
            let [byte] = byte;
            if byte == 0 {
                break;
            }
            header.push(byte);
            if header.len() == MAX_HEADER_LEN {
                return Err(broken(format!(
                    "{} has a header longer than Git reads",
                    stream.what()
                )));
            }
        }
        let (kind, size) = parse_header(&header)
            .ok_or_else(|| broken(format!("{} has a damaged header", stream.what())))?;
        if size > MAX_LOOSE_SIZE {
            return Err(broken(format!(
                "{} says it holds {size} bytes, more than the {MAX_LOOSE_SIZE} Bolt reads",
                stream.what()
            )));
        }
        Ok(Some(Loose { stream, kind, size }))
    }

    fn read(mut self) -> Result<Object, GitError> {
        let data = self.stream.read_all(self.size)?;
        Ok(Object {
            kind: self.kind,
            data,
        })
    }
}

/// `<type> <size>`, as Git reads them: a type it knows, one space, and the size in decimal
/// without leading zeros.
fn parse_header(header: &[u8]) -> Option<(ObjectType, u64)> {
    let space = header.iter().position(|&b| b == b' ')?;
    let (name, size) = header.split_at_checked(space)?;
    let size = size.get(1..)?;
    let canonical = match size {
        [b'0'] => true,
        [first, ..] => *first != b'0' && size.iter().all(u8::is_ascii_digit),
        [] => false,
    };
    if !canonical {
        return None;
    }
    let kind = ObjectType::named(name)?;
    Some((kind, std::str::from_utf8(size).ok()?.parse().ok()?))
}

/// Part of a file read in order with positional reads, from `pos` up to `end`.
pub(super) struct Input<'p, F> {
    file: F,
    path: Cow<'p, Path>,
    pos: u64,
    end: u64,
}

impl<'p, F: Borrow<File>> Input<'p, F> {
    pub(super) fn new(file: F, path: impl Into<Cow<'p, Path>>, pos: u64, end: u64) -> Self {
        Input {
            file,
            path: path.into(),
            pos,
            end,
        }
    }

    /// Reads as much as one read gives; 0 only at `end`. A file that ends sooner got shorter
    /// after it was measured.
    pub(super) fn read(&mut self, buf: &mut [u8]) -> Result<usize, GitError> {
        let left = usize::try_from(self.end.saturating_sub(self.pos)).unwrap_or(usize::MAX);
        let want = buf.len().min(left);
        let Some(buf) = buf.get_mut(..want).filter(|buf| !buf.is_empty()) else {
            return Ok(0);
        };
        let got = loop {
            match safe_fs::read_at(self.file.borrow(), buf, self.pos) {
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                result => break result.map_err(|e| read_error(&self.path, e))?,
            }
        };
        if got == 0 {
            return Err(shrank(&self.path));
        }
        self.pos = self.pos.saturating_add(got as u64);
        Ok(got)
    }

    /// Fills as much of `buf` as comes before `end`, and says how much that was.
    pub(super) fn fill(&mut self, buf: &mut [u8]) -> Result<usize, GitError> {
        let mut filled = 0;
        while let Some(rest) = buf.get_mut(filled..).filter(|rest| !rest.is_empty()) {
            match self.read(rest)? {
                0 => break,
                got => filled = filled.saturating_add(got),
            }
        }
        Ok(filled)
    }
}

/// Inflates a zlib stream through fixed buffers, so a bomb costs time, never memory.
struct Inflater<'p, F> {
    input: Input<'p, F>,
    /// The pack entry's offset, for messages; `None` for a loose object, whose file must end
    /// where its stream does.
    entry: Option<u64>,
    state: Box<InflateState>,
    buf: Vec<u8>,
    /// What is left of `buf` to inflate.
    start: usize,
    end: usize,
    ended: bool,
}

impl<'p, F: Borrow<File>> Inflater<'p, F> {
    fn new(input: Input<'p, F>, entry: Option<u64>) -> Self {
        Inflater {
            input,
            entry,
            state: InflateState::new_boxed(DataFormat::Zlib),
            buf: vec![0; CHUNK],
            start: 0,
            end: 0,
            ended: false,
        }
    }

    /// The stream, for messages: a loose object's file, or a pack entry.
    fn what(&self) -> String {
        match self.entry {
            Some(at) => format!("the entry at offset {at} in {}", self.input.path.display()),
            None => self.input.path.display().to_string(),
        }
    }

    /// Inflates into `out` and says how much came; 0 only once the stream has ended. zlib's
    /// checksum is checked as it ends.
    fn read(&mut self, out: &mut [u8]) -> Result<usize, GitError> {
        if self.ended || out.is_empty() {
            return Ok(0);
        }
        loop {
            let input = self.buf.get(self.start..self.end).unwrap_or_default();
            let result = inflate(&mut self.state, input, out, MZFlush::None);
            self.start = self.start.saturating_add(result.bytes_consumed);
            match result.status {
                Ok(MZStatus::StreamEnd) => {
                    self.ended = true;
                    return Ok(result.bytes_written);
                }
                // Given no input, miniz_oxide says Buf even after writing the rest of a copy it
                // had begun: those bytes count as any others.
                Ok(_) | Err(MZError::Buf) if result.bytes_written > 0 => {
                    return Ok(result.bytes_written);
                }
                // It took all it was given: read more.
                Ok(_) | Err(MZError::Buf) if self.start >= self.end => {
                    self.start = 0;
                    self.end = self.input.read(&mut self.buf)?;
                    if self.end == 0 {
                        return Err(broken(format!(
                            "{} ends in the middle of its compressed data",
                            self.what()
                        )));
                    }
                }
                _ => {
                    return Err(broken(format!(
                        "{} holds damaged compressed data",
                        self.what()
                    )));
                }
            }
        }
    }

    /// The whole rest of the stream: exactly `size` bytes, at most `MAX_IN_MEMORY_OBJECT`.
    fn read_all(&mut self, size: u64) -> Result<Vec<u8>, GitError> {
        if size > MAX_IN_MEMORY_OBJECT {
            return Err(broken(format!(
                "{} says it holds {size} bytes, more than the {MAX_IN_MEMORY_OBJECT} Bolt reads \
                 into memory",
                self.what()
            )));
        }
        let len = usize::try_from(size).unwrap_or(usize::MAX);
        let mut data = Vec::new();
        data.try_reserve_exact(len).map_err(|_| GitError::Io {
            path: self.input.path.to_path_buf(),
            source: io::ErrorKind::OutOfMemory.into(),
        })?;
        let mut chunk = [0u8; CHUNK];
        while data.len() < len {
            let want = chunk.len().min(len.saturating_sub(data.len()));
            let part = chunk.get_mut(..want).unwrap_or_default();
            let got = self.read(part)?;
            if got == 0 {
                return Err(self.ended_early(data.len() as u64, size));
            }
            data.extend_from_slice(part.get(..got).unwrap_or_default());
        }
        self.end(size)?;
        Ok(data)
    }

    /// The stream ended after `got` of the `size` bytes its object gives.
    fn ended_early(&self, got: u64, size: u64) -> GitError {
        GitError::EndedEarly {
            what: format!(
                "{} ended after {got} of the {size} bytes it says it holds",
                self.what()
            ),
        }
    }

    /// Checks that the stream ends after its `size` bytes, and a loose object's file with it.
    fn end(&mut self, size: u64) -> Result<(), GitError> {
        if self.read(&mut [0u8])? != 0 {
            return Err(broken(format!(
                "{} holds more than the {size} bytes it says",
                self.what()
            )));
        }
        if self.entry.is_none() && (self.start < self.end || self.input.pos < self.input.end) {
            return Err(broken(format!(
                "{} has bytes after its compressed data",
                self.what()
            )));
        }
        Ok(())
    }
}

/// A blob's bytes, checked against its ID as they end: if they don't match, the read that would
/// finish them fails instead, and so does every read after it.
pub struct BlobReader<'r>(Reading<'r>);

enum Reading<'r> {
    /// Checked already: rebuilt from deltas, or a stream that passed.
    Memory(io::Cursor<Vec<u8>>),
    Stream(Box<Stream<'r>>),
}

impl fmt::Debug for BlobReader<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BlobReader").finish_non_exhaustive()
    }
}

impl Read for BlobReader<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        match &mut self.0 {
            Reading::Memory(data) => data.read(buf),
            Reading::Stream(stream) => {
                let got = stream.read(buf).map_err(io::Error::other)?;
                if stream.left == 0 {
                    self.0 = Reading::Memory(io::Cursor::default());
                }
                Ok(got)
            }
        }
    }
}

/// A blob Git stored whole, inflated as it is read.
struct Stream<'r> {
    inflater: Inflater<'r, File>,
    hasher: Hasher,
    id: GitId,
    size: u64,
    left: u64,
}

impl Stream<'_> {
    /// More of the blob. The read that would end it checks it first.
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, GitError> {
        let want = buf
            .len()
            .min(usize::try_from(self.left).unwrap_or(usize::MAX));
        let buf = buf.get_mut(..want).unwrap_or_default();
        let got = self.inflater.read(buf)?;
        if got == 0 && self.left > 0 {
            let read = self.size.saturating_sub(self.left);
            return Err(self.inflater.ended_early(read, self.size));
        }
        self.hasher.update(buf.get(..got).unwrap_or_default());
        self.left = self.left.saturating_sub(got as u64);
        if self.left == 0 {
            self.inflater.end(self.size)?;
            verify(&self.id, self.hasher.clone())?;
        }
        Ok(got)
    }
}

/// Bases rebuilt for deltas, kept for the next object built on one. Once they pass the budget,
/// the least recently used go first.
struct BaseCache {
    budget: u64,
    bytes: u64,
    clock: u64,
    /// Each base by pack number and offset, with when it was put in.
    entries: HashMap<(usize, u64), (u64, Base)>,
    /// Each entry's key, by when it was put in.
    by_use: BTreeMap<u64, (usize, u64)>,
}

impl Default for BaseCache {
    fn default() -> Self {
        BaseCache::new(DELTA_BASE_CACHE)
    }
}

impl BaseCache {
    fn new(budget: u64) -> BaseCache {
        BaseCache {
            budget,
            bytes: 0,
            clock: 0,
            entries: HashMap::new(),
            by_use: BTreeMap::new(),
        }
    }

    /// Takes the base at `key` out, to build on.
    fn take(&mut self, key: (usize, u64)) -> Option<Base> {
        let (used, base) = self.entries.remove(&key)?;
        self.by_use.remove(&used);
        self.bytes = self.bytes.saturating_sub(base.data.len() as u64);
        Some(base)
    }

    /// Keeps a base, dropping the least recently used ones to stay within the budget. One bigger
    /// than the whole budget isn't kept.
    fn put(&mut self, key: (usize, u64), base: Base) {
        let len = base.data.len() as u64;
        if len > self.budget {
            return;
        }
        self.take(key);
        while self.bytes.saturating_add(len) > self.budget {
            let Some((_, oldest)) = self.by_use.pop_first() else {
                break;
            };
            if let Some((_, gone)) = self.entries.remove(&oldest) {
                self.bytes = self.bytes.saturating_sub(gone.data.len() as u64);
            }
        }
        self.clock = self.clock.saturating_add(1);
        self.by_use.insert(self.clock, key);
        self.entries.insert(key, (self.clock, base));
        self.bytes = self.bytes.saturating_add(len);
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    #![allow(clippy::indexing_slicing, clippy::arithmetic_side_effects)]
    use super::*;
    use crate::git::craft::{Craft, PackEntry, delta, entry_header};
    use crate::git::object_id;
    use crate::pack::PackSet;
    use std::fs;

    use crate::git::HashKind::{self, Sha1, Sha256};
    use ObjectType::{Blob, Commit, Tree};

    fn craft(hash: HashKind) -> (tempfile::TempDir, Craft) {
        let dir = tempfile::tempdir().unwrap();
        let git = Craft::new(dir.path(), hash);
        (dir, git)
    }

    fn open(dir: &tempfile::TempDir) -> Repo {
        Repo::open(dir.path()).unwrap()
    }

    fn id(kind: ObjectType, data: &[u8], hash: HashKind) -> GitId {
        object_id(kind, data, hash).unwrap()
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

    /// The message of a `Changed` error, failing the test on anything else.
    #[track_caller]
    fn changed<T: std::fmt::Debug>(result: Result<T, GitError>) -> String {
        match result {
            Err(e @ GitError::Changed { .. }) => e.to_string(),
            other => panic!("expected Changed, got {other:?}"),
        }
    }

    /// All `reader` gives, `step` bytes at a time.
    fn read_all(mut reader: impl Read, step: usize) -> io::Result<Vec<u8>> {
        let mut out = Vec::new();
        let mut buf = vec![0; step];
        loop {
            match reader.read(&mut buf)? {
                0 => return Ok(out),
                n => out.extend_from_slice(&buf[..n]),
            }
        }
    }

    #[test]
    fn loose_objects_read_back() {
        let (dir, git) = craft(Sha1);
        let hello = git.object(Blob, b"hello\n");
        let tree_bytes = [b"100644 hello\0".as_slice(), hello.as_bytes()].concat();
        let tree = git.object(Tree, &tree_bytes);
        let commit_bytes = format!(
            "tree {}\nauthor A <a@example.com> 0 +0000\ncommitter A <a@example.com> 0 +0000\n\n\
             first\n",
            tree.to_hex()
        )
        .into_bytes();
        let commit = git.commit(&commit_bytes);
        let empty = git.object(Blob, b"");
        let repo = open(&dir);
        for (id, kind, data) in [
            (hello, Blob, b"hello\n".as_slice()),
            (tree, Tree, &tree_bytes),
            (commit, Commit, &commit_bytes),
            (empty, Blob, b""),
        ] {
            assert_eq!(repo.header(&id).unwrap(), (kind, data.len() as u64));
            let object = repo.read(&id).unwrap();
            assert_eq!((object.kind, object.data.as_slice()), (kind, data));
        }
        assert_eq!(
            read_all(repo.blob_reader(&hello).unwrap(), 3).unwrap(),
            b"hello\n"
        );
        // An object found nowhere is named.
        let nowhere = id(Blob, b"nowhere", Sha1);
        let missing = format!("missing object `{}`", nowhere.to_hex());
        assert!(broken(repo.read(&nowhere)).contains(&missing));
        assert!(broken(repo.header(&nowhere)).contains(&missing));
        assert!(broken(repo.blob_reader(&nowhere)).contains(&missing));
    }

    /// Packs a blob whole, one built on it by an `OFS_DELTA`, one built on that by a
    /// `REF_DELTA`, and a tree; returns each object's type and bytes, in that order.
    fn pack_with_deltas(git: &Craft) -> Vec<(ObjectType, Vec<u8>)> {
        // Over 64 KiB, so copies of 0x10000 bytes come into it.
        let base: Vec<u8> = (0..20_000u32).flat_map(|i| i.to_le_bytes()).collect();
        let mut second = base.clone();
        second.extend_from_slice(b"one more line\n");
        let mut third = second[..1000].to_vec();
        third.extend_from_slice(b"and a new ending\n");
        let tree = [
            b"100644 a\0".as_slice(),
            id(Blob, &base, git.hash()).as_bytes(),
        ]
        .concat();
        git.pack(&[
            PackEntry::Whole(Blob, base.clone()),
            PackEntry::OfsDelta {
                base: 0,
                delta: delta(&base, &second),
            },
            PackEntry::RefDelta {
                base: id(Blob, &second, git.hash()),
                delta: delta(&second, &third),
            },
            PackEntry::Whole(Tree, tree.clone()),
        ]);
        vec![(Blob, base), (Blob, second), (Blob, third), (Tree, tree)]
    }

    #[test]
    fn packed_objects_read_back_whole_and_through_both_kinds_of_delta() {
        let (dir, git) = craft(Sha1);
        let objects = pack_with_deltas(&git);
        let repo = open(&dir);
        // In an order that takes bases from the cache and puts them back.
        for i in [2, 1, 2, 0, 3, 1, 0, 2] {
            let (kind, data) = &objects[i];
            let id = id(*kind, data, Sha1);
            assert_eq!(repo.header(&id).unwrap(), (*kind, data.len() as u64), "{i}");
            let object = repo.read(&id).unwrap();
            assert_eq!((object.kind, &object.data), (*kind, data), "{i}");
        }
        assert!(repo.objects.cache.borrow().bytes > 0);
    }

    #[test]
    fn sha256_objects_read_back() {
        let (dir, git) = craft(Sha256);
        let loose = git.object(Blob, b"hello\n");
        let objects = pack_with_deltas(&git);
        let repo = open(&dir);
        assert_eq!(repo.read(&loose).unwrap().data, b"hello\n");
        for (kind, data) in &objects {
            let id = id(*kind, data, Sha256);
            assert_eq!(id.as_bytes().len(), 32);
            assert_eq!(repo.header(&id).unwrap(), (*kind, data.len() as u64));
            assert_eq!(&repo.read(&id).unwrap().data, data);
        }
        let (_, blob) = &objects[2];
        let reader = repo.blob_reader(&id(Blob, blob, Sha256)).unwrap();
        assert_eq!(&read_all(reader, 1000).unwrap(), blob);
    }

    #[test]
    fn blobs_stored_whole_are_streamed() {
        let (dir, git) = craft(Sha1);
        let loose: Vec<u8> = (0..300_000u32)
            .flat_map(|i| (i % 251).to_le_bytes())
            .collect();
        let packed: Vec<u8> = loose.iter().rev().copied().collect();
        let deltified = [packed.as_slice(), b"!"].concat();
        git.object(Blob, &loose);
        git.object(Blob, b"");
        git.pack(&[
            PackEntry::Whole(Blob, packed.clone()),
            PackEntry::OfsDelta {
                base: 0,
                delta: delta(&packed, &deltified),
            },
        ]);
        let repo = open(&dir);
        for (data, streamed) in [
            (&loose, true),
            (&packed, true),
            (&deltified, false),
            (&Vec::new(), true),
        ] {
            let mut reader = repo.blob_reader(&id(Blob, data, Sha1)).unwrap();
            assert_eq!(matches!(reader.0, Reading::Stream(_)), streamed);
            let got = read_all(&mut reader, 1000).unwrap();
            assert_eq!(got.len(), data.len());
            assert!(got == *data);
            // A reader that has ended keeps saying so.
            assert_eq!(reader.read(&mut [0; 10]).unwrap(), 0);
        }
        // Only blobs.
        let tree = git.object(Tree, b"");
        let message = broken(repo.blob_reader(&tree));
        assert!(
            message.contains("a tree, not a file's contents"),
            "{message}"
        );
    }

    #[test]
    fn a_streamed_blob_with_the_wrong_id_fails_its_last_read() {
        let (dir, git) = craft(Sha1);
        let data = vec![b'x'; 100_000];
        // The bytes of one blob, under another's ID, loose and packed.
        let loose = id(Blob, b"another blob", Sha1);
        let stored = [format!("blob {}\0", data.len()).as_bytes(), &data].concat();
        git.raw_loose(&loose.to_hex(), &deflate(&stored));
        let raw = [entry_header(3, data.len() as u64), deflate(&data)].concat();
        git.pack(&[PackEntry::Raw(raw.clone())]);
        let packed = id(Blob, &raw, Sha1);
        let repo = open(&dir);
        for wrong in [loose, packed] {
            let mut reader = repo.blob_reader(&wrong).unwrap();
            let mut buf = [0; 4096];
            let mut delivered = 0;
            let err = loop {
                match reader.read(&mut buf) {
                    Ok(0) => panic!("the blob ended without a check"),
                    Ok(n) => delivered += n,
                    Err(e) => break e,
                }
            };
            // The bytes that would finish it never come.
            assert!(delivered < data.len());
            let inner = err.get_ref().and_then(|e| e.downcast_ref::<GitError>());
            match inner {
                Some(e @ GitError::Broken { .. }) => {
                    assert!(e.to_string().contains(&wrong.to_hex()), "{e}");
                }
                other => panic!("{other:?}"),
            }
            assert!(reader.read(&mut buf).is_err(), "it keeps failing");
            // So a pack that streams it in keeps nothing of it.
            let store = tempfile::tempdir().unwrap();
            let (tmp, packs) = (store.path().join("tmp"), store.path().join("packs"));
            fs::create_dir_all(&tmp).unwrap();
            fs::create_dir_all(&packs).unwrap();
            let mut set = PackSet::with_limit(&tmp, &packs, 100).unwrap();
            let mut reader = repo.blob_reader(&wrong).unwrap();
            let expected = crate::id::hash_content(&data);
            assert!(set.add_content_from(&mut reader, expected).is_err());
            assert!(set.is_empty());
        }
    }

    #[test]
    fn a_huge_declared_size_is_refused_before_allocating() {
        let (dir, git) = craft(Sha1);
        let id = id(Blob, b"x", Sha1);
        git.raw_loose(&id.to_hex(), &deflate(b"blob 99999999999\0x"));
        let raw = [entry_header(3, 1 << 40), deflate(b"x")].concat();
        git.pack(&[PackEntry::Raw(raw.clone())]);
        let repo = open(&dir);
        assert!(matches!(repo.read(&id), Err(GitError::Broken { .. })));
        // The header only reports what the object says, and a stream finds it short at its end.
        assert_eq!(repo.header(&id).unwrap(), (Blob, 99_999_999_999));
        assert!(read_all(repo.blob_reader(&id).unwrap(), 4096).is_err());
        let message = broken(repo.read(&self::id(Blob, &raw, Sha1)));
        assert!(message.contains(&(1u64 << 40).to_string()), "{message}");
    }

    #[test]
    fn loose_headers_are_read_as_git_reads_them() {
        let (dir, git) = craft(Sha1);
        let long = [b"blob 5".as_slice(), &[b' '; 40], b"\0hello"].concat();
        let refused: Vec<&[u8]> = vec![
            b"blob 05\0hello",
            b"blob  5\0hello",
            b"blob 5 \0hello",
            b"blob -5\0hello",
            b"blob +5\0hello",
            b"blob\0",
            b"blob 5",
            b"blub 5\0hello",
            b"BLOB 5\0hello",
            b"blob 1099511627777\0x",
            &long,
        ];
        let ids: Vec<GitId> = (0..refused.len())
            .map(|n| id(Blob, format!("case {n}").as_bytes(), Sha1))
            .collect();
        for (stored, id) in refused.iter().zip(&ids) {
            git.raw_loose(&id.to_hex(), &deflate(stored));
        }
        // The largest size Git allows, with too few bytes after it.
        let largest = id(Blob, b"largest", Sha1);
        git.raw_loose(&largest.to_hex(), &deflate(b"blob 1099511627776\0x"));
        let short = id(Blob, b"hello", Sha1);
        git.raw_loose(&short.to_hex(), &deflate(b"blob 5\0hell"));
        let long_data = id(Blob, b"world", Sha1);
        git.raw_loose(&long_data.to_hex(), &deflate(b"blob 5\0world!"));
        let repo = open(&dir);
        for (stored, id) in refused.iter().zip(&ids) {
            let what = String::from_utf8_lossy(stored);
            assert!(repo.header(id).is_err(), "{what}");
            let message = broken(repo.read(id));
            assert!(message.contains(&id.to_hex()[2..]), "{what}: {message}");
        }
        assert_eq!(repo.header(&largest).unwrap(), (Blob, 1 << 40));
        broken(repo.read(&largest));
        assert!(broken(repo.read(&long_data)).contains("more than the 5 bytes"));
        // Data that ends early is a fault to report, which `git fsck` may not see, read whole or
        // streamed.
        let streamed = read_all(repo.blob_reader(&short).unwrap(), 4096).unwrap_err();
        let streamed = streamed
            .into_inner()
            .unwrap()
            .downcast::<GitError>()
            .unwrap();
        for e in [repo.read(&short).unwrap_err(), *streamed] {
            assert!(matches!(e, GitError::EndedEarly { .. }), "{e:?}");
            let message = e.to_string();
            assert!(
                message.contains("ended after 4 of the 5 bytes"),
                "{message}"
            );
            let hint = e.hint().unwrap();
            assert!(
                hint.contains("please report it") && !hint.contains("fsck"),
                "{hint}"
            );
        }
    }

    #[test]
    fn delta_chains_are_bounded_and_cycles_refused() {
        let (dir, git) = craft(Sha1);
        // 4,097 deltas, each on the one before, over a blob stored whole.
        let versions: Vec<Vec<u8>> = (0..=4097).map(|i| format!("v{i}").into_bytes()).collect();
        let mut entries = vec![PackEntry::Whole(Blob, versions[0].clone())];
        for i in 1..versions.len() {
            entries.push(PackEntry::OfsDelta {
                base: i - 1,
                delta: delta(&versions[i - 1], &versions[i]),
            });
        }
        git.pack(&entries);
        let repo = open(&dir);
        let deepest_allowed = id(Blob, &versions[4096], Sha1);
        assert_eq!(repo.read(&deepest_allowed).unwrap().data, versions[4096]);
        assert_eq!(repo.header(&deepest_allowed).unwrap(), (Blob, 5));
        // Refused the same way whether its bases are in the cache, as they are now, or not.
        let too_deep = id(Blob, &versions[4097], Sha1);
        assert!(broken(repo.read(&too_deep)).contains("more than 4096 deltas deep"));
        assert!(broken(open(&dir).read(&too_deep)).contains("more than 4096 deltas deep"));
        assert!(broken(repo.header(&too_deep)).contains("more than 4096 deltas deep"));

        // Two REF_DELTAs, each on the other.
        let (dir, git) = craft(Sha1);
        let (to_a, to_b) = (delta(b"x", b"a"), delta(b"x", b"b"));
        let (a, b) = (id(Blob, &to_a, Sha1), id(Blob, &to_b, Sha1));
        git.pack(&[
            PackEntry::RefDelta {
                base: b,
                delta: to_a,
            },
            PackEntry::RefDelta {
                base: a,
                delta: to_b,
            },
        ]);
        let repo = open(&dir);
        for id in [a, b] {
            assert!(broken(repo.read(&id)).contains("loop"));
            assert!(broken(repo.header(&id)).contains("loop"));
        }

        // Three REF_DELTAs in a ring: A on B, B on C, C on A.
        let (dir, git) = craft(Sha1);
        let (to_a, to_b, to_c) = (delta(b"x", b"a"), delta(b"x", b"b"), delta(b"x", b"c"));
        let (a, b, c) = (
            id(Blob, &to_a, Sha1),
            id(Blob, &to_b, Sha1),
            id(Blob, &to_c, Sha1),
        );
        git.pack(&[
            PackEntry::RefDelta {
                base: b,
                delta: to_a,
            },
            PackEntry::RefDelta {
                base: c,
                delta: to_b,
            },
            PackEntry::RefDelta {
                base: a,
                delta: to_c,
            },
        ]);
        let repo = open(&dir);
        for id in [a, b, c] {
            assert!(broken(repo.read(&id)).contains("loop"));
            assert!(broken(repo.header(&id)).contains("loop"));
        }

        // An OFS_DELTA 0 bytes back, on itself; and a REF_DELTA on an object outside its pack.
        let (dir, git) = craft(Sha1);
        let on_itself = delta(b"x", b"y");
        let outside = delta(b"loose", b"elsewhere");
        let loose = git.object(Blob, b"loose");
        git.pack(&[
            PackEntry::Whole(Blob, b"x".to_vec()),
            PackEntry::OfsDelta {
                base: 1,
                delta: on_itself.clone(),
            },
            PackEntry::RefDelta {
                base: loose,
                delta: outside.clone(),
            },
        ]);
        let repo = open(&dir);
        assert!(broken(repo.read(&id(Blob, &on_itself, Sha1))).contains("itself"));
        assert!(broken(repo.read(&id(Blob, &outside, Sha1))).contains("isn't in that pack"));
    }

    #[test]
    fn a_zlib_bomb_and_trailing_garbage_are_refused() {
        let (dir, git) = craft(Sha1);
        // 8 MiB of zeros, a few kilobytes compressed, in objects that say they hold 10 bytes.
        let zeros = vec![0u8; 8 << 20];
        let bomb = id(Blob, b"bomb", Sha1);
        let stored = [b"blob 10\0".as_slice(), &zeros].concat();
        git.raw_loose(&bomb.to_hex(), &deflate(&stored));
        let raw = [entry_header(3, 10), deflate(&zeros)].concat();
        git.pack(&[PackEntry::Raw(raw.clone())]);
        // Bytes after a loose object's stream, and a stream whose checksum is wrong.
        let hello = id(Blob, b"hello", Sha1);
        let mut trailing = deflate(b"blob 5\0hello");
        trailing.extend_from_slice(b"garbage");
        git.raw_loose(&hello.to_hex(), &trailing);
        let world = id(Blob, b"world", Sha1);
        let mut bad_sum = deflate(b"blob 5\0world");
        *bad_sum.last_mut().unwrap() ^= 1;
        git.raw_loose(&world.to_hex(), &bad_sum);
        let repo = open(&dir);
        for id in [bomb, id(Blob, &raw, Sha1)] {
            assert!(broken(repo.read(&id)).contains("more than the 10 bytes"));
            assert!(read_all(repo.blob_reader(&id).unwrap(), 4096).is_err());
        }
        assert!(broken(repo.read(&hello)).contains("after its compressed data"));
        assert!(read_all(repo.blob_reader(&hello).unwrap(), 4096).is_err());
        // A stream this short is inflated whole with its header, so its checksum fails there.
        assert!(broken(repo.read(&world)).contains("damaged"));
        assert!(broken(repo.blob_reader(&world)).contains("damaged"));
    }

    #[test]
    fn an_object_whose_id_doesnt_match_is_refused() {
        let (dir, git) = craft(Sha1);
        let other = id(Blob, b"other", Sha1);
        git.raw_loose(&other.to_hex(), &deflate(b"blob 5\0hello"));
        let raw = [entry_header(3, 5), deflate(b"hello")].concat();
        git.pack(&[PackEntry::Raw(raw.clone())]);
        let repo = open(&dir);
        for wrong in [other, id(Blob, &raw, Sha1)] {
            let message = broken(repo.read(&wrong));
            assert!(message.contains(&wrong.to_hex()), "{message}");
            assert!(message.contains("doesn't match"), "{message}");
        }
    }

    #[test]
    fn a_pack_removed_while_reading_is_reported_as_changed() {
        let (dir, git) = craft(Sha1);
        let (pack, _) = git.pack(&[PackEntry::Whole(Blob, b"packed".to_vec())]);
        let loose = git.object(Blob, b"loose");
        let packed = id(Blob, b"packed", Sha1);
        let repo = open(&dir);
        // Reading anything lists the packs and checks each against its index.
        assert_eq!(repo.read(&loose).unwrap().data, b"loose");
        fs::remove_file(&pack).unwrap();
        let name = pack.file_name().unwrap().to_string_lossy().into_owned();
        for message in [
            changed(repo.read(&packed)),
            changed(repo.header(&packed)),
            changed(repo.blob_reader(&packed)),
        ] {
            assert!(message.contains(&name), "{message}");
        }
        // A pack rewritten to another length before Bolt opens it to read has changed too, and
        // so has one that gets shorter while Bolt has it open.
        let (dir, git) = craft(Sha1);
        let (pack, _) = git.pack(&[PackEntry::Whole(Blob, b"packed".to_vec())]);
        git.object(Blob, b"loose");
        let repo = open(&dir);
        assert_eq!(repo.read(&loose).unwrap().data, b"loose");
        let good = fs::read(&pack).unwrap();
        fs::write(&pack, [good.as_slice(), &[0]].concat()).unwrap();
        let message = changed(repo.read(&packed));
        assert!(
            message.contains(&name) && message.contains("changed size"),
            "{message}"
        );
        fs::write(&pack, &good).unwrap();
        let repo = open(&dir);
        assert_eq!(repo.read(&packed).unwrap().data, b"packed");
        fs::OpenOptions::new()
            .write(true)
            .open(&pack)
            .unwrap()
            .set_len(20)
            .unwrap();
        let message = changed(repo.read(&packed));
        assert!(
            message.contains(&name) && message.contains("got shorter"),
            "{message}"
        );
        // A loose object Git packed meanwhile: its file is gone, and a pack Bolt hasn't listed
        // holds it.
        let (dir, git) = craft(Sha1);
        let moved = git.object(Blob, b"moved");
        let repo = open(&dir);
        assert_eq!(repo.read(&moved).unwrap().data, b"moved");
        git.pack(&[PackEntry::Whole(Blob, b"moved".to_vec())]);
        let hex = moved.to_hex();
        fs::remove_file(
            dir.path()
                .join(".git/objects")
                .join(&hex[..2])
                .join(&hex[2..]),
        )
        .unwrap();
        changed(repo.read(&moved));
    }

    /// What an `Inflater` gives for the zlib stream `z`, read `step` bytes at a time.
    fn inflated(z: &[u8], step: usize) -> Result<Vec<u8>, GitError> {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("stream");
        fs::write(&path, z).unwrap();
        let file = File::open(&path).unwrap();
        let mut stream = Inflater::new(Input::new(file, path.as_path(), 0, z.len() as u64), None);
        let mut out = Vec::new();
        let mut buf = vec![0; step];
        loop {
            match stream.read(&mut buf)? {
                0 => return Ok(out),
                n => out.extend_from_slice(&buf[..n]),
            }
        }
    }

    #[test]
    fn a_stream_inflates_whole_where_miniz_oxide_says_buf_after_writing() {
        // The smallest stream of zeros found where, with 16 KiB of input a call, miniz_oxide
        // 0.9.1 writes the rest of a copy given no input and says Buf (RT3-3).
        let zeros = vec![0u8; 52_864 << 10];
        let z = deflate(&zeros);
        for step in [CHUNK, 8192] {
            let out = inflated(&z, step).unwrap();
            assert_eq!(out.len(), zeros.len(), "{step}");
            assert!(out == zeros, "{step}");
        }
    }

    /// `len` bytes of zeros or, if `sparse`, of 1 MiB pages of zeros each ending in 4 KiB of
    /// noise, as disk images and padded archives hold.
    fn mostly_zeros(len: usize, sparse: bool) -> Vec<u8> {
        let mut data = vec![0u8; len];
        let mut seed = 1u64;
        if sparse {
            for page in data.chunks_mut(1 << 20) {
                let at = page.len().saturating_sub(4096);
                for byte in &mut page[at..] {
                    seed = seed
                        .wrapping_mul(6364136223846793005)
                        .wrapping_add(1442695040888963407);
                    *byte = (seed >> 56) as u8;
                }
            }
        }
        data
    }

    /// RT3-3's sweep: the sizes and shapes where streams came out short. The data compressed is
    /// what a sound inflater gives back, so each stream is checked against it byte for byte.
    #[test]
    #[ignore = "RT3-3 sweep: 120 streams of 110 to 300 MiB; run in release, a few minutes"]
    fn big_mostly_zero_streams_inflate_whole() {
        let mut wrong = Vec::new();
        for mib in (110..=300).step_by(10) {
            for sparse in [false, true] {
                let data = mostly_zeros(mib << 20, sparse);
                for level in [1, 6, 9] {
                    let z = miniz_oxide::deflate::compress_to_vec_zlib(&data, level);
                    for step in [CHUNK, 8192] {
                        match inflated(&z, step) {
                            Ok(out) if out == data => {}
                            other => wrong.push((mib, sparse, level, step, other.map(|o| o.len()))),
                        }
                    }
                }
            }
        }
        assert!(wrong.is_empty(), "{} wrong: {wrong:?}", wrong.len());
    }

    #[test]
    fn the_base_cache_keeps_recent_bases_within_its_budget() {
        let base = |byte, len| Base {
            kind: Blob,
            data: vec![byte; len],
            depth: 0,
        };
        let mut cache = BaseCache::new(10);
        cache.put((0, 1), base(1, 4));
        cache.put((0, 2), base(2, 4));
        // Over the budget: the least recently used goes.
        cache.put((0, 3), base(3, 4));
        assert_eq!(cache.take((0, 1)), None);
        assert_eq!(cache.take((0, 2)), Some(base(2, 4)));
        assert_eq!(cache.bytes, 4);
        // A base bigger than the whole budget isn't kept.
        cache.put((1, 1), base(0, 11));
        assert_eq!(cache.take((1, 1)), None);
        assert_eq!(cache.bytes, 4);
    }
}
