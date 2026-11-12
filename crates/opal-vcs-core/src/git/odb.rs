//! Reading Git's objects, loose or packed, whole or through deltas (decision 031). Sizes are
//! checked before anything is allocated, zlib streams inflate through fixed buffers, and every
//! object is checked against its ID.
#![deny(clippy::indexing_slicing, clippy::arithmetic_side_effects)]

mod family;

use std::borrow::{Borrow, Cow};
use std::cell::{Cell, OnceCell, RefCell};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt;
use std::fs::{File, OpenOptions};
use std::io::{self, BufReader, Read};
use std::path::{Path, PathBuf};

use miniz_oxide::inflate::stream::{InflateState, inflate};
use miniz_oxide::{DataFormat, MZError, MZFlush, MZStatus};

use super::id::Hasher;
use super::pack::{
    self, DeltaError, Entry, EntryKind, Pack, PackFolder, build_delta, delta_sizes, delta_start,
};
use super::{
    GitError, GitId, ObjectType, Repo, broken, out_of_memory, read_error, real_dir, shrank,
};
use crate::safe_fs;

/// The most an object read whole into memory may hold, and so the most a delta may build or
/// build on. Git never deltifies files over 512 MiB unless told to (`core.bigFileThreshold`).
pub const MAX_IN_MEMORY_OBJECT: u64 = 512 << 20;
/// The most deltas one object may be built through; Git's own cap is 4,095.
pub const MAX_DELTA_DEPTH: u32 = 4096;
/// The most the bases kept for later deltas may hold: Git's own default.
pub const DELTA_BASE_CACHE: u64 = 96 << 20;
/// Bytes an import may inflate and build to rebuild objects through deltas: at least this...
const MIN_DELTA_WORK: u64 = 4 << 30;
/// ...or this many for each compressed byte of the pack entries it has inflated, each counted
/// once. Zeros a hole puts inside an entry's stream are inflated and count too, so a hostile
/// pack still buys 35 to 1,000 times its inflating time in rebuilding. git/git needs at most 47,
/// a margin of about 11.
const DELTA_WORK_PER_INFLATED_BYTE: u64 = 500;
/// Work counted for each entry header a walk down a delta chain reads past its first: one read
/// from disk takes about as long as inflating a kilobyte.
const ENTRY_HEADER_WORK: u64 = 1 << 10;
/// Pack entries whose type, depth and size are kept once a header walk learns them: about 43 MB
/// at most, or 64 MB for a moment while the map grows. Past it nothing more is kept, so walks
/// repeat, each still counted against the work allowance.
const MAX_KNOWN_ENTRIES: usize = 1 << 19;
/// The most a loose object may say it holds: 1 TiB.
const MAX_LOOSE_SIZE: u64 = 1 << 40;
/// The most deflate expands its input: a 258-byte match takes 2 bits.
const MAX_EXPANSION: u64 = 1032;
/// A loose object's header fits in this many bytes with its NUL, as Git requires.
const MAX_HEADER_LEN: usize = 32;
/// Compressed bytes read at a time, and inflated ones handed on.
const CHUNK: usize = 16 << 10;
/// Packs kept open between reads, the last ones used: few enough for any system's limit on open
/// files.
const MAX_OPEN_PACKS: usize = 16;

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    static TEST_LIMITS: Cell<Option<(u64, u64)>> = const { Cell::new(None) };
}

/// Sets this thread's limits for rebuilding through deltas to `(work, cache)`, so a test can
/// reach them with small data; `None` goes back. The work allowed, `work` bytes however much was
/// inflated, holds from the next step on, in repositories already open too. The cache's
/// `cache` bytes hold for the repositories opened from then on.
#[cfg(any(test, feature = "test-support"))]
pub fn set_test_delta_limits(limits: Option<(u64, u64)>) {
    TEST_LIMITS.set(limits);
}

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
    /// What header walks learned of pack entries, by pack number and offset. An entry never
    /// changes, so nothing here goes stale.
    known: RefCell<HashMap<(usize, u64), Known>>,
    /// The number of the pack the last packed object was found in.
    last_pack: Cell<usize>,
    /// Bytes inflated and built so far to rebuild objects through deltas.
    work: Cell<u64>,
    /// Compressed bytes of the pack entries inflated so far, each entry counted once.
    inflated: Cell<u64>,
    /// The delta families rebuilt, by pack number and the number of the entry at the bottom.
    families: RefCell<HashSet<(usize, usize)>>,
    /// Objects built through deltas so far.
    #[cfg(test)]
    built: Cell<u64>,
}

impl fmt::Debug for Objects {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Objects").finish_non_exhaustive()
    }
}

/// What a header walk learned of a pack entry.
#[derive(Clone, Copy)]
struct Known {
    /// The type at the bottom of its chain: its own, if it is stored whole.
    kind: ObjectType,
    /// How many deltas deep it is.
    depth: u32,
    /// What it holds or builds, once a header was asked for it.
    size: Option<u64>,
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
                let header = self.packed_header(n, pack, &file, at);
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
        self.check(id, object.kind, &object.data)?;
        Ok(object)
    }

    /// A blob's bytes. A blob Git stored whole is inflated as it is read and checked against its
    /// ID as it ends; a deltified one is rebuilt in memory and checked first.
    pub fn blob_reader(&self, id: &GitId) -> Result<BlobReader<'_>, GitError> {
        let (inflater, size, entry) = match self.find(id)? {
            Place::Loose(loose) => {
                is_blob(id, loose.kind)?;
                (loose.stream, loose.size, None)
            }
            Place::Packed(n, pack, at) => {
                let file = self.open_pack(n, pack)?;
                let entry = pack.entry(&file, at)?;
                let EntryKind::Whole(kind) = entry.kind else {
                    let object = self.unpack(n, pack, &file, at);
                    self.keep_open(n, file);
                    let object = object?;
                    self.check(id, object.kind, &object.data)?;
                    is_blob(id, object.kind)?;
                    return Ok(BlobReader(Reading::Memory(io::Cursor::new(object.data))));
                };
                is_blob(id, kind)?;
                // The stream reads through this handle of its own until it is dropped.
                let input = Input::new(file, pack.path.as_path(), entry.data, pack.entries_end());
                (Inflater::new(input, Some(at)), entry.size, Some((pack, at)))
            }
        };
        Ok(BlobReader(Reading::Stream(Box::new(Stream {
            inflater,
            hasher: Hasher::object(self.hash, ObjectType::Blob, size),
            id: *id,
            size,
            left: size,
            entry,
            inflated: &self.objects.inflated,
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

    /// Where `id` is: in the pack that found the last object if it holds it, else in the first
    /// pack that does, as Git looks (objects read one after another are mostly packed together),
    /// or else in its loose file.
    fn find(&self, id: &GitId) -> Result<Place<'_>, GitError> {
        let folder = self.packs()?;
        let last = self.objects.last_pack.get();
        let last = folder.packs.get(last).map(|pack| (last, pack));
        for (n, pack) in last.into_iter().chain(folder.packs.iter().enumerate()) {
            if let Some(at) = pack.find(id) {
                self.objects.last_pack.set(n);
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
        // Git may have packed it since Opalro listed the packs: then the list is different now.
        if !pack::list(&self.pack_dir()).is_ok_and(|now| now == folder.listed) {
            return Err(GitError::Changed {
                what: format!("the packs in {} changed", self.pack_dir().display()),
            });
        }
        Err(broken(format!("missing object `{hex}`")))
    }

    /// The object at `at` in pack number `n`, rebuilt through its deltas from the nearest base
    /// the cache holds, or from the object stored whole at the bottom of its chain. A base used
    /// stays in the cache and counts as just used; what is built goes in too.
    fn unpack(&self, n: usize, pack: &Pack, file: &File, at: u64) -> Result<Object, GitError> {
        self.build(n, pack, file, at, true)
    }

    /// `unpack`, putting nothing new in the cache unless `keep`. What the deltas take to inflate
    /// and apply is counted against the import's budget before each step.
    fn build(
        &self,
        n: usize,
        pack: &Pack,
        file: &File,
        at: u64,
        keep: bool,
    ) -> Result<Object, GitError> {
        let cache = &self.objects.cache;
        let chain = self.chain(pack, file, at, &mut |here| {
            let depth = cache.try_borrow().ok()?.depth((n, here))?;
            Some(((), depth))
        })?;
        let stored_whole = chain.deltas.is_empty();
        let mut deltas = chain.deltas.into_iter().rev();
        let mut built_at = chain.bottom_at;
        let mut object = match chain.bottom {
            // Read, not built, so the cache is left as it is.
            Bottom::Whole(kind, entry) if stored_whole => {
                let data = self.inflate_entry(pack, file, built_at, &entry)?;
                return Ok(Object { kind, data });
            }
            Bottom::Whole(kind, entry) => {
                self.spend(entry.size)?;
                Base {
                    kind,
                    data: self.inflate_entry(pack, file, built_at, &entry)?,
                    depth: 0,
                }
            }
            // Built on where it lies, so it stays in the cache.
            Bottom::Cached((), _) => {
                let mut cache = cache.try_borrow_mut().map_err(|_| lost(pack))?;
                let base = cache.get((n, built_at)).ok_or_else(|| lost(pack))?;
                let Some((delta_at, entry)) = deltas.next() else {
                    let mut data = Vec::new();
                    data.try_reserve_exact(base.data.len())
                        .map_err(|_| out_of_memory(&pack.path))?;
                    data.extend_from_slice(&base.data);
                    return Ok(Object {
                        kind: base.kind,
                        data,
                    });
                };
                built_at = delta_at;
                Base {
                    kind: base.kind,
                    data: self.apply(pack, file, delta_at, &entry, &base.data)?,
                    depth: base.depth.saturating_add(1),
                }
            }
        };
        for (delta_at, entry) in deltas {
            let next = Base {
                kind: object.kind,
                data: self.apply(pack, file, delta_at, &entry, &object.data)?,
                depth: object.depth.saturating_add(1),
            };
            let base = std::mem::replace(&mut object, next);
            if keep && let Ok(mut cache) = cache.try_borrow_mut() {
                cache.put((n, built_at), base);
            }
            built_at = delta_at;
        }
        if keep && let Ok(mut cache) = cache.try_borrow_mut() {
            cache.put_copy((n, built_at), &object);
        }
        Ok(Object {
            kind: object.kind,
            data: object.data,
        })
    }

    /// What the delta entry at `at` builds on `base`, applied as it is inflated, so only the base
    /// and what it builds are held whole. The delta may say it holds at most
    /// `MAX_IN_MEMORY_OBJECT`, as an object read whole may. Inflating it and building what it
    /// builds count as work, each before it is done; the compressed bytes it took raise the
    /// allowance, once.
    fn apply(
        &self,
        pack: &Pack,
        file: &File,
        at: u64,
        entry: &Entry,
        base: &[u8],
    ) -> Result<Vec<u8>, GitError> {
        let damaged = |why: String| delta_error(&pack.path, at, DeltaError::Damaged(why));
        let failed = |e: DeltaError| delta_error(&pack.path, at, e);
        self.spend(entry.size)?;
        if entry.size > MAX_IN_MEMORY_OBJECT {
            return Err(damaged(format!(
                "says it holds {} bytes, more than the {MAX_IN_MEMORY_OBJECT} Opalro reads of a delta",
                entry.size
            )));
        }
        let input = Input::new(file, pack.path.as_path(), entry.data, pack.entries_end());
        let data = DeltaData {
            inflater: Inflater::new(input, Some(at)),
            size: entry.size,
            left: entry.size,
        };
        let mut delta = BufReader::with_capacity(CHUNK, data);
        let size = delta_start(base, &mut delta).map_err(failed)?;
        self.spend(size)?;
        let built = build_delta(base, size, &mut delta).map_err(failed)?;
        let taken = delta.into_inner().inflater.taken;
        count_inflated(&self.objects.inflated, pack, at, taken);
        #[cfg(test)]
        self.objects
            .built
            .set(self.objects.built.get().saturating_add(1));
        Ok(built)
    }

    /// An entry's data inflated whole: exactly the size its header gives, at most
    /// `MAX_IN_MEMORY_OBJECT`. What it took counts toward the work allowance, once.
    fn inflate_entry(
        &self,
        pack: &Pack,
        file: &File,
        at: u64,
        entry: &Entry,
    ) -> Result<Vec<u8>, GitError> {
        let input = Input::new(file, pack.path.as_path(), entry.data, pack.entries_end());
        let mut stream = Inflater::new(input, Some(at));
        let data = stream.read_all(entry.size)?;
        count_inflated(&self.objects.inflated, pack, at, stream.taken);
        Ok(data)
    }

    /// Counts `bytes` more inflated or built through deltas, stopping past the import's
    /// allowance: a delta of a few bytes can build hundreds of megabytes, again for each object
    /// read on it. An honest history of many small edits to a big file gets there too, since
    /// each edit adds little to what is inflated, so the stop names both.
    fn spend(&self, bytes: u64) -> Result<(), GitError> {
        // No step does more: it refuses a bigger claim itself, in its own words.
        let bytes = bytes.min(MAX_IN_MEMORY_OBJECT);
        let work = self.objects.work.get().saturating_add(bytes);
        self.objects.work.set(work);
        let allowance = delta_work_allowance(self.objects.inflated.get());
        if work <= allowance {
            return Ok(());
        }
        Err(GitError::TooMuchWork {
            what: format!(
                "rebuilding objects from Git's deltas would take more than the {} GiB of work Opalro \
                 allows: the repository holds a long history of small edits to a big file, or it \
                 was built to make importing slow",
                allowance >> 30
            ),
        })
    }

    /// Checks that `data` is the object of `kind` that `id` names.
    fn check(&self, id: &GitId, kind: ObjectType, data: &[u8]) -> Result<(), GitError> {
        let mut hasher = Hasher::object(self.hash, kind, data.len() as u64);
        hasher.update(data);
        verify(id, hasher)
    }
}

/// What `e` stopping the delta at `at` in the pack at `path` is told as: damage, or a read error
/// through the reader's one mapping, which says when Opalro ran out of memory.
fn delta_error(path: &Path, at: u64, e: DeltaError) -> GitError {
    match e {
        DeltaError::Damaged(why) => broken(format!(
            "the delta at offset {at} in {} {why}",
            path.display()
        )),
        DeltaError::Read(e) => e
            .downcast::<GitError>()
            .unwrap_or_else(|e| read_error(path, e)),
    }
}

/// What only a bug in Opalro causes: something the reader kept of `pack` went missing.
fn lost(pack: &Pack) -> GitError {
    GitError::Bug {
        what: format!("Opalro lost track of what it read of {}", pack.path.display()),
    }
}

impl Repo {
    /// The type and size of the object at `at` in pack number `n`, as its entry says; for a
    /// delta, what it says it builds, and the type at the bottom of its chain. What the walk
    /// down the chain learns is kept (up to `MAX_KNOWN_ENTRIES` entries), so no header walks past
    /// one of those again.
    fn packed_header(
        &self,
        n: usize,
        pack: &Pack,
        file: &File,
        at: u64,
    ) -> Result<(ObjectType, u64), GitError> {
        let known = self.known(n, at);
        if let Some(Known {
            kind,
            size: Some(size),
            ..
        }) = known
        {
            return Ok((kind, size));
        }
        let entry = pack.entry(file, at)?;
        let (kind, depth, size) = match entry.kind {
            EntryKind::Whole(kind) => (kind, 0, entry.size),
            EntryKind::OfsDelta(_) | EntryKind::RefDelta(_) => {
                let size = built_size(pack, file, at, &entry)?;
                let (kind, depth) = match known {
                    Some(known) => (known.kind, known.depth),
                    None => self.chain_kind(n, pack, file, at)?,
                };
                (kind, depth, size)
            }
        };
        let learned = Known {
            kind,
            depth,
            size: Some(size),
        };
        self.learn(n, at, learned);
        Ok((kind, size))
    }

    /// The type at the bottom of the chain from the entry at `at` in pack number `n`, and how
    /// many deltas deep that entry is. The walk stops at an entry learned before, and every
    /// entry it passes is learned.
    fn chain_kind(
        &self,
        n: usize,
        pack: &Pack,
        file: &File,
        at: u64,
    ) -> Result<(ObjectType, u32), GitError> {
        let chain = self.chain(pack, file, at, &mut |here| {
            self.known(n, here).map(|known| (known.kind, known.depth))
        })?;
        let (kind, mut depth) = match chain.bottom {
            Bottom::Cached(kind, depth) => (kind, depth),
            Bottom::Whole(kind, entry) => {
                let whole = Known {
                    kind,
                    depth: 0,
                    size: Some(entry.size),
                };
                self.learn(n, chain.bottom_at, whole);
                (kind, 0)
            }
        };
        for &(here, _) in chain.deltas.iter().rev() {
            depth = depth.saturating_add(1);
            let delta = Known {
                kind,
                depth,
                size: None,
            };
            self.learn(n, here, delta);
        }
        Ok((kind, depth))
    }

    /// What a header walk learned of the entry at `at` in pack number `n`, if anything.
    fn known(&self, n: usize, at: u64) -> Option<Known> {
        self.objects.known.try_borrow().ok()?.get(&(n, at)).copied()
    }

    /// Keeps what was learned of the entry at `at` in pack number `n`: in place of what was
    /// known of it, or beside the rest while there is room.
    fn learn(&self, n: usize, at: u64, known: Known) {
        let Ok(mut map) = self.objects.known.try_borrow_mut() else {
            return;
        };
        if let Some(was) = map.get_mut(&(n, at)) {
            *was = known;
        } else if map.len() < MAX_KNOWN_ENTRIES {
            map.insert((n, at), known);
        }
    }

    /// Follows the deltas from the entry at `at` to an object stored whole, or to an entry
    /// `cached` gives something for with how many deltas deep it is, without recursion: at most
    /// `MAX_DELTA_DEPTH` deltas in all, the cached entry's counted too, and never round a loop.
    /// Each entry header read past the first counts against the work allowance.
    fn chain<T>(
        &self,
        pack: &Pack,
        file: &File,
        at: u64,
        cached: &mut dyn FnMut(u64) -> Option<(T, u32)>,
    ) -> Result<Chain<T>, GitError> {
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
            if let Some((found, below)) = cached(here) {
                let depth = u32::try_from(deltas.len()).unwrap_or(u32::MAX);
                if depth.saturating_add(below) > MAX_DELTA_DEPTH {
                    return Err(too_deep());
                }
                return Ok(Chain {
                    deltas,
                    bottom_at: here,
                    bottom: Bottom::Cached(found, below),
                });
            }
            if !seen.insert(here) {
                return Err(broken(format!(
                    "the deltas that build the object at offset {at} in {shown} go round in a loop"
                )));
            }
            if !deltas.is_empty() {
                self.spend(ENTRY_HEADER_WORK)?;
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
                        "the delta at offset {here} in {shown} builds on `{}`, which isn't in \
                         that pack",
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
}

/// The most an import may inflate and build through deltas once it has inflated `inflated`
/// compressed bytes of pack entries.
fn delta_work_allowance(inflated: u64) -> u64 {
    #[cfg(any(test, feature = "test-support"))]
    if let Some((work, _)) = TEST_LIMITS.get() {
        return work;
    }
    MIN_DELTA_WORK.max(DELTA_WORK_PER_INFLATED_BYTE.saturating_mul(inflated))
}

/// Adds the `taken` compressed bytes the entry at `at` in `pack` inflated from to `total`, the
/// first time only, so data read again can't raise the allowance.
fn count_inflated(total: &Cell<u64>, pack: &Pack, at: u64, taken: u64) {
    if pack.first_inflation(at) {
        total.set(total.get().saturating_add(taken));
    }
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
struct Chain<T> {
    deltas: Vec<(u64, Entry)>,
    bottom_at: u64,
    bottom: Bottom<T>,
}

/// What a chain of deltas builds on.
enum Bottom<T> {
    /// An object stored whole, and its entry.
    Whole(ObjectType, Entry),
    /// What was cached for the entry, and how many deltas deep it is.
    Cached(T, u32),
}

/// An object rebuilt for deltas to build on.
#[derive(Debug, PartialEq, Eq)]
struct Base {
    kind: ObjectType,
    data: Vec<u8>,
    /// How many deltas it was built through.
    depth: u32,
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
                "{} says it holds {size} bytes, more than the {MAX_LOOSE_SIZE} Opalro reads",
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
    /// Where it began.
    start: u64,
    pos: u64,
    end: u64,
}

impl<'p, F: Borrow<File>> Input<'p, F> {
    pub(super) fn new(file: F, path: impl Into<Cow<'p, Path>>, pos: u64, end: u64) -> Self {
        Input {
            file,
            path: path.into(),
            start: pos,
            pos,
            end,
        }
    }

    /// How many bytes it was given to read.
    fn len(&self) -> u64 {
        self.end.saturating_sub(self.start)
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
    /// Compressed bytes inflated so far.
    taken: u64,
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
            taken: 0,
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
            self.taken = self.taken.saturating_add(result.bytes_consumed as u64);
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
                // It took all it was given: read more. A read that fails leaves the buffer as it
                // was, so it can't be taken for new input.
                Ok(_) | Err(MZError::Buf) if self.start >= self.end => {
                    let got = self.input.read(&mut self.buf)?;
                    if got == 0 {
                        return Err(broken(format!(
                            "{} ends in the middle of its compressed data",
                            self.what()
                        )));
                    }
                    self.start = 0;
                    self.end = got;
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
                "{} says it holds {size} bytes, more than the {MAX_IN_MEMORY_OBJECT} Opalro reads \
                 into memory",
                self.what()
            )));
        }
        // A size the bytes can't hold is refused before any room is made for it.
        let most = self.input.len().saturating_mul(MAX_EXPANSION);
        if size > most {
            return Err(broken(format!(
                "{} says it holds {size} bytes, but the {} bytes of compressed data it has can't \
                 hold more than {most}",
                self.what(),
                self.input.len()
            )));
        }
        let len = usize::try_from(size).unwrap_or(usize::MAX);
        let mut data = Vec::new();
        data.try_reserve_exact(len)
            .map_err(|_| out_of_memory(&self.input.path))?;
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

    /// Inflates into `out` at most the `left` bytes still to come of the `size` its object gives,
    /// takes what came off `left`, and says how much that was: 0 only once all have come. The
    /// read that brings `left` to 0 checks that the stream ends there. `out` mustn't be empty.
    fn read_part(&mut self, out: &mut [u8], left: &mut u64, size: u64) -> Result<usize, GitError> {
        let want = out.len().min(usize::try_from(*left).unwrap_or(usize::MAX));
        let got = self.read(out.get_mut(..want).unwrap_or_default())?;
        if got == 0 && *left > 0 {
            return Err(self.ended_early(size.saturating_sub(*left), size));
        }
        *left = left.saturating_sub(got as u64);
        if *left == 0 {
            self.end(size)?;
        }
        Ok(got)
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
/// finish them fails instead, and so does every read after it. So does a read that fails for any
/// other reason.
pub struct BlobReader<'r>(Reading<'r>);

enum Reading<'r> {
    /// Checked already: rebuilt from deltas, or a stream that passed.
    Memory(io::Cursor<Vec<u8>>),
    Stream(Box<Stream<'r>>),
    /// A read failed, with this message: so does every read after it.
    Failed(String),
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
            Reading::Stream(stream) => match stream.read(buf) {
                Ok(got) => {
                    if stream.left == 0 {
                        self.0 = Reading::Memory(io::Cursor::default());
                    }
                    Ok(got)
                }
                Err(e) => {
                    self.0 = Reading::Failed(e.to_string());
                    Err(io::Error::other(e))
                }
            },
            Reading::Failed(message) => Err(io::Error::other(message.clone())),
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
    /// Its pack and offset, if it is packed, and the repository's count of what was inflated.
    entry: Option<(&'r Pack, u64)>,
    inflated: &'r Cell<u64>,
}

impl Stream<'_> {
    /// More of the blob. The read that would end it checks it first.
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, GitError> {
        let got = self.inflater.read_part(buf, &mut self.left, self.size)?;
        self.hasher.update(buf.get(..got).unwrap_or_default());
        if self.left == 0 {
            verify(&self.id, self.hasher.clone())?;
            if let Some((pack, at)) = self.entry {
                count_inflated(self.inflated, pack, at, self.inflater.taken);
            }
        }
        Ok(got)
    }
}

/// A delta entry's data, inflated as it is read: exactly the size its header gives, then the end
/// of its stream. A failure comes as an `io::Error` holding the `GitError`.
struct DeltaData<'a> {
    inflater: Inflater<'a, &'a File>,
    size: u64,
    left: u64,
}

impl Read for DeltaData<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        let got = self.inflater.read_part(buf, &mut self.left, self.size);
        got.map_err(io::Error::other)
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
        #[cfg(any(test, feature = "test-support"))]
        if let Some((_, cache)) = TEST_LIMITS.get() {
            return BaseCache::new(cache);
        }
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

    /// How many deltas deep the base at `key` is, if it is kept.
    fn depth(&self, key: (usize, u64)) -> Option<u32> {
        self.entries.get(&key).map(|(_, base)| base.depth)
    }

    /// The base at `key`, to build on where it lies: it counts as just used.
    fn get(&mut self, key: (usize, u64)) -> Option<&Base> {
        let clock = self.clock.saturating_add(1);
        let (used, base) = self.entries.get_mut(&key)?;
        self.by_use.remove(used);
        self.by_use.insert(clock, key);
        *used = clock;
        self.clock = clock;
        Some(base)
    }

    /// Takes the base at `key` out.
    fn take(&mut self, key: (usize, u64)) -> Option<Base> {
        let (used, base) = self.entries.remove(&key)?;
        self.by_use.remove(&used);
        self.bytes = self.bytes.saturating_sub(base.data.len() as u64);
        Some(base)
    }

    /// Keeps a base, dropping the least recently used ones to stay within the budget. One bigger
    /// than the whole budget isn't kept.
    fn put(&mut self, key: (usize, u64), base: Base) {
        if self.make_room(key, base.data.len() as u64) {
            self.insert(key, base);
        }
    }

    /// `put` for a copy of `base`, made once there is room for it.
    fn put_copy(&mut self, key: (usize, u64), base: &Base) {
        if !self.make_room(key, base.data.len() as u64) {
            return;
        }
        let mut data = Vec::new();
        if data.try_reserve_exact(base.data.len()).is_ok() {
            data.extend_from_slice(&base.data);
            let (kind, depth) = (base.kind, base.depth);
            self.insert(key, Base { kind, data, depth });
        }
    }

    /// Drops what `key` held, then the least recently used bases until `len` more bytes fit:
    /// false if they never can.
    fn make_room(&mut self, key: (usize, u64), len: u64) -> bool {
        if len > self.budget {
            return false;
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
        true
    }

    fn insert(&mut self, key: (usize, u64), base: Base) {
        self.clock = self.clock.saturating_add(1);
        self.by_use.insert(self.clock, key);
        self.bytes = self.bytes.saturating_add(base.data.len() as u64);
        self.entries.insert(key, (self.clock, base));
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

    pub(super) fn craft(hash: HashKind) -> (tempfile::TempDir, Craft) {
        let dir = tempfile::tempdir().unwrap();
        let git = Craft::new(dir.path(), hash);
        (dir, git)
    }

    pub(super) fn open(dir: &tempfile::TempDir) -> Repo {
        Repo::open(dir.path()).unwrap()
    }

    pub(super) fn id(kind: ObjectType, data: &[u8], hash: HashKind) -> GitId {
        object_id(kind, data, hash).unwrap()
    }

    fn deflate(bytes: &[u8]) -> Vec<u8> {
        miniz_oxide::deflate::compress_to_vec_zlib(bytes, 6)
    }

    /// The message of a `Broken` error, failing the test on anything else.
    #[track_caller]
    pub(super) fn broken<T: std::fmt::Debug>(result: Result<T, GitError>) -> String {
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
    pub(super) fn read_all(mut reader: impl Read, step: usize) -> io::Result<Vec<u8>> {
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
    fn objects_are_found_among_many_packs_the_one_that_found_the_last_asked_first() {
        let (dir, git) = craft(Sha1);
        let blobs: Vec<Vec<u8>> = (0..200)
            .map(|i| format!("blob {i}\n").into_bytes())
            .collect();
        for pair in blobs.chunks(2) {
            let pair: Vec<PackEntry> = pair
                .iter()
                .map(|b| PackEntry::Whole(Blob, b.clone()))
                .collect();
            git.pack(&pair);
        }
        // An object two packs hold, each with one of its own.
        let shared = b"in two packs".to_vec();
        for own in [b"own one".as_slice(), b"own two"] {
            git.pack(&[
                PackEntry::Whole(Blob, shared.clone()),
                PackEntry::Whole(Blob, own.to_vec()),
            ]);
        }
        // Packs of no objects under names of their own, as an archive can hold them.
        let (pack, idx) = git.pack(&[]);
        for i in 0..100 {
            fs::copy(&pack, pack.with_file_name(format!("pack-empty{i}.pack"))).unwrap();
            fs::copy(&idx, idx.with_file_name(format!("pack-empty{i}.idx"))).unwrap();
        }
        let repo = open(&dir);
        for blob in &blobs {
            let id = id(Blob, blob, Sha1);
            assert_eq!(&repo.read(&id).unwrap().data, blob);
            let folder = repo.packs().unwrap();
            assert!(
                folder.packs[repo.objects.last_pack.get()]
                    .find(&id)
                    .is_some()
            );
        }
        // The pack that found the last object is asked first: after an object only the later of
        // two packs holds, one both hold is found there, not in the earlier.
        let folder = repo.packs().unwrap();
        let shared = id(Blob, &shared, Sha1);
        let both: Vec<usize> = (0..folder.packs.len())
            .filter(|&n| folder.packs[n].find(&shared).is_some())
            .collect();
        assert_eq!(both.len(), 2);
        let later = both[1];
        let own = ["own one", "own two"]
            .map(|own| id(Blob, own.as_bytes(), Sha1))
            .into_iter()
            .find(|own| folder.packs[later].find(own).is_some())
            .unwrap();
        repo.read(&own).unwrap();
        assert_eq!(repo.objects.last_pack.get(), later);
        repo.read(&shared).unwrap();
        assert_eq!(repo.objects.last_pack.get(), later);
        // Packs of no objects are listed, so a change to them is noticed, but never searched.
        assert_eq!((folder.listed.len(), folder.packs.len()), (203, 102));
    }

    #[test]
    fn rebuilding_through_deltas_counts_what_they_inflate_and_build() {
        let (dir, git) = craft(Sha1);
        let objects = pack_with_deltas(&git);
        let [(_, base), (_, second), (_, third), (_, tree)] = &objects[..] else {
            unreachable!()
        };
        let repo = open(&dir);
        // Objects stored whole are read, not rebuilt.
        repo.read(&id(Blob, base, Sha1)).unwrap();
        repo.read(&id(Tree, tree, Sha1)).unwrap();
        assert_eq!(repo.objects.work.get(), 0);
        // The two entry headers read below the first, the bottom inflated, then each delta
        // inflated and applied.
        repo.read(&id(Blob, third, Sha1)).unwrap();
        let rebuilt = 2 * ENTRY_HEADER_WORK as usize
            + base.len()
            + delta(base, second).len()
            + second.len()
            + delta(second, third).len()
            + third.len();
        assert_eq!(repo.objects.work.get(), rebuilt as u64);
        // A base the cache kept is built on again for nothing.
        repo.read(&id(Blob, second, Sha1)).unwrap();
        assert_eq!(repo.objects.work.get(), rebuilt as u64);
    }

    #[test]
    fn the_work_allowance_grows_only_with_data_inflated_once() {
        let (dir, git) = craft(Sha1);
        let objects = pack_with_deltas(&git);
        let [(_, base), (_, second), (_, third), (_, tree)] = &objects[..] else {
            unreachable!()
        };
        git.pack(&[PackEntry::Whole(Blob, b"streamed".to_vec())]);
        // An empty pack padded with a megabyte of zeros, as a hole would leave it.
        let (empty, _) = git.pack(&[]);
        let bytes = fs::read(&empty).unwrap();
        let (head, trailer) = bytes.split_at(12);
        fs::write(&empty, [head, &vec![0; 1 << 20], trailer].concat()).unwrap();
        let repo = open(&dir);
        repo.read(&id(Blob, third, Sha1)).unwrap();
        let chain = deflate(base).len()
            + deflate(&delta(base, second)).len()
            + deflate(&delta(second, third)).len();
        assert_eq!(repo.objects.inflated.get(), chain as u64);
        // Each entry counts once, however often it is inflated again.
        repo.read(&id(Blob, base, Sha1)).unwrap();
        *repo.objects.cache.borrow_mut() = BaseCache::default();
        repo.read(&id(Blob, third, Sha1)).unwrap();
        assert_eq!(repo.objects.inflated.get(), chain as u64);
        // An entry read whole or streamed counts once it has been inflated.
        repo.read(&id(Tree, tree, Sha1)).unwrap();
        let reader = repo.blob_reader(&id(Blob, b"streamed", Sha1)).unwrap();
        read_all(reader, 3).unwrap();
        let more = deflate(tree).len() + deflate(b"streamed").len();
        assert_eq!(repo.objects.inflated.get(), (chain + more) as u64);
        // So a small repository has the least allowance, and only data read raises it.
        assert_eq!(
            delta_work_allowance(repo.objects.inflated.get()),
            MIN_DELTA_WORK
        );
        assert!(delta_work_allowance(10 << 30) > MIN_DELTA_WORK);
    }

    #[test]
    fn rebuilding_stops_before_a_step_past_the_budget() {
        let (dir, git) = craft(Sha1);
        let objects = pack_with_deltas(&git);
        let [(_, base), (_, second), ..] = &objects[..] else {
            unreachable!()
        };
        let repo = open(&dir);
        let budget = MIN_DELTA_WORK;
        // Room for the base's entry header, the base and the delta, but not for what the delta
        // builds.
        let room = ENTRY_HEADER_WORK as usize + base.len() + delta(base, second).len();
        repo.objects.work.set(budget - room as u64);
        match repo.read(&id(Blob, second, Sha1)) {
            Err(e @ GitError::TooMuchWork { .. }) => {
                let says = format!(
                    "more than the {} GiB of work Opalro allows",
                    MIN_DELTA_WORK >> 30
                );
                assert!(e.to_string().contains(&says), "{e}");
                // An honest history of small edits to a big file can get here too, and the way
                // out works for both.
                let why = "the repository holds a long history of small edits to a big file, or \
                           it was built to make importing slow";
                assert!(e.to_string().contains(why), "{e}");
                let hint = "Nothing was changed. If you trust this repository, have Git store its \
                            files over 1 MiB whole and make its other deltas anew, with `git -c \
                            core.bigFileThreshold=1m repack -a -d -f`, then run opal import \
                            again. Git's packs stay bigger until its next `git gc`.";
                assert_eq!(e.hint().unwrap(), hint);
                assert!(hint.contains("`git -c core.bigFileThreshold=1m repack -a -d -f`"));
            }
            other => panic!("{other:?}"),
        }
        // The inflating fit exactly; building is what was refused.
        assert_eq!(repo.objects.work.get(), budget + second.len() as u64);
    }

    #[test]
    fn a_header_walks_down_a_chain_once_counting_what_it_reads() {
        let (dir, git) = craft(Sha1);
        // 100 deltas, each on the one before, over a blob stored whole.
        let versions: Vec<Vec<u8>> = (0..=100).map(|i| format!("v{i}").into_bytes()).collect();
        let mut entries = vec![PackEntry::Whole(Blob, versions[0].clone())];
        for i in 1..versions.len() {
            entries.push(PackEntry::OfsDelta {
                base: i - 1,
                delta: delta(&versions[i - 1], &versions[i]),
            });
        }
        git.pack(&entries);
        let header = |repo: &Repo, i: usize| repo.header(&id(Blob, &versions[i], Sha1)).unwrap();
        let repo = open(&dir);
        // Each entry below the first is read once, and counts.
        assert_eq!(header(&repo, 100), (Blob, 4));
        assert_eq!(repo.objects.work.get(), 100 * ENTRY_HEADER_WORK);
        // No entry of that chain is walked past again, the first included.
        for i in [100, 50, 0, 99] {
            assert_eq!(header(&repo, i), (Blob, versions[i].len() as u64), "{i}");
        }
        assert_eq!(repo.objects.work.get(), 100 * ENTRY_HEADER_WORK);
        // A walk past the allowance stops.
        let repo = open(&dir);
        repo.objects
            .work
            .set(MIN_DELTA_WORK - 99 * ENTRY_HEADER_WORK);
        let what = repo.header(&id(Blob, &versions[100], Sha1));
        assert!(
            matches!(what, Err(GitError::TooMuchWork { .. })),
            "{what:?}"
        );
        // With no room left to keep what it learns, a header walks the whole chain each time.
        let repo = open(&dir);
        let other = Known {
            kind: Tree,
            depth: 0,
            size: None,
        };
        let full = (0..MAX_KNOWN_ENTRIES as u64).map(|at| ((usize::MAX, at), other));
        repo.objects.known.borrow_mut().extend(full);
        for _ in 0..2 {
            assert_eq!(header(&repo, 100), (Blob, 4));
        }
        assert_eq!(repo.objects.work.get(), 200 * ENTRY_HEADER_WORK);
        assert_eq!(repo.objects.known.borrow().len(), MAX_KNOWN_ENTRIES);
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
    fn a_streamed_blob_keeps_failing_after_a_read_fails() {
        let (dir, git) = craft(Sha1);
        // Bytes deflate can't shrink, so the file holds more than one chunk of them.
        let mut x = 0x2545_f491_4f6c_dd1d_u64;
        let data: Vec<u8> = (0..100_000)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                (x >> 24) as u8
            })
            .collect();
        let blob = git.object(Blob, &data);
        let repo = open(&dir);
        let mut reader = repo.blob_reader(&blob).unwrap();
        let mut buf = [0; 4096];
        reader.read_exact(&mut buf).unwrap();
        // The file gets shorter while Opalro reads it: the next chunk can't be read.
        let hex = blob.to_hex();
        let path = dir
            .path()
            .join(".git/objects")
            .join(&hex[..2])
            .join(&hex[2..]);
        fs::OpenOptions::new()
            .write(true)
            .open(path)
            .unwrap()
            .set_len(20)
            .unwrap();
        let first = loop {
            match reader.read(&mut buf) {
                Ok(0) => panic!("the blob ended"),
                Ok(_) => {}
                Err(e) => break e,
            }
        };
        for _ in 0..3 {
            let again = reader.read(&mut buf).unwrap_err();
            assert_eq!(again.to_string(), first.to_string());
        }
    }

    #[test]
    fn a_size_the_compressed_bytes_cannot_hold_is_refused_before_room_is_made() {
        let (dir, git) = craft(Sha1);
        // 500 MiB said, a few bytes stored.
        let liar = id(Blob, b"liar", Sha1);
        git.raw_loose(&liar.to_hex(), &deflate(b"blob 524288000\0x"));
        // Zeros shrink about as much as deflate can make anything, about 1,000 to 1.
        let zeros = vec![0u8; 8 << 20];
        let stored = [b"blob 8388608\0".as_slice(), &zeros].concat();
        let squeezed = id(Blob, &zeros, Sha1);
        git.raw_loose(&squeezed.to_hex(), &deflate(&stored));
        let repo = open(&dir);
        // A damaged or hostile object, as the hint says: no fault of Opalro's to report.
        match repo.read(&liar) {
            Err(e @ GitError::Broken { .. }) => {
                let message = e.to_string();
                assert!(
                    message.contains("says it holds 524288000 bytes, but the")
                        && message.contains("can't hold more than"),
                    "{message}"
                );
                assert!(e.hint().unwrap().contains("git fsck"), "{e:?}");
            }
            other => panic!("{other:?}"),
        }
        assert!(repo.read(&squeezed).unwrap().data == zeros);
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
        // Data that ends early, read whole or streamed, is damage `git fsck` may not see, or a fault
        // in Opalro's reader, and the hint says both.
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
                hint.contains("the repository was damaged or made this way")
                    && hint.contains("a fault in Opalro's reader")
                    && hint.contains("please report it"),
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
        // A pack rewritten to another length before Opalro opens it to read has changed too, and
        // so has one that gets shorter while Opalro has it open.
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
        // A loose object Git packed meanwhile: its file is gone, and a pack Opalro hasn't listed
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
        // 0.9.1 writes the rest of a copy given no input and says Buf: those bytes must count.
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

    /// Big files of zeros and sparse data, 110 to 300 MiB, the sizes where Buf with bytes written
    /// comes most often. The data compressed is what a sound inflater gives back, so each stream
    /// is checked against it byte for byte.
    #[test]
    #[ignore = "slow: 120 streams of 110 to 300 MiB, a few minutes in release; run nightly"]
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

    /// Where `id` is: its pack's number and its offset there.
    pub(super) fn place(repo: &Repo, id: &GitId) -> (usize, u64) {
        match repo.find(id).unwrap() {
            Place::Packed(n, _, at) => (n, at),
            Place::Loose(_) => panic!("{} is loose", id.to_hex()),
        }
    }

    #[test]
    fn the_base_cache_keeps_what_it_builds() {
        let (dir, git) = craft(Sha1);
        let objects = pack_with_deltas(&git);
        let [(_, base), (_, second), (_, third), _] = &objects[..] else {
            unreachable!()
        };
        let blobs = [base, second, third];
        let ids = blobs.map(|data| id(Blob, data, Sha1));
        let kept = |repo: &Repo| {
            ids.map(|id| {
                let (n, at) = place(repo, &id);
                repo.objects.cache.borrow().depth((n, at))
            })
        };
        let repo = open(&dir);
        repo.read(&ids[2]).unwrap();
        // The bases it built on and what it built, each with how deep it is.
        assert_eq!(kept(&repo), [Some(0), Some(1), Some(2)]);
        // Each is read from where it lies, and left there: nothing is built again.
        let work = repo.objects.work.get();
        for _ in 0..2 {
            for (id, data) in ids.iter().zip(blobs) {
                assert_eq!(&repo.read(id).unwrap().data, data);
            }
        }
        assert_eq!(repo.objects.work.get(), work);
        assert_eq!(kept(&repo), [Some(0), Some(1), Some(2)]);
        // A base used counts as just used, so it outlasts one left alone.
        let mut cache = BaseCache::new(10);
        let small = |byte| Base {
            kind: Blob,
            data: vec![byte; 4],
            depth: 0,
        };
        cache.put((0, 1), small(1));
        cache.put((0, 2), small(2));
        assert_eq!(cache.get((0, 1)), Some(&small(1)));
        cache.put((0, 3), small(3));
        let held = |cache: &BaseCache| [1, 2, 3].map(|at| cache.depth((0, at)).is_some());
        assert_eq!(held(&cache), [true, false, true]);
        // One as big as the whole budget is kept.
        cache.put_copy(
            (0, 4),
            &Base {
                kind: Blob,
                data: vec![4; 10],
                depth: 0,
            },
        );
        assert_eq!(cache.depth((0, 4)), Some(0));
        assert_eq!(cache.bytes, 10);
    }

    /// The hint for running out of memory, not the `git fsck` one or the permissions one.
    fn says_out_of_memory(e: &GitError) {
        assert!(matches!(e, GitError::OutOfMemory { .. }), "{e:?}");
        let hint = e.hint().unwrap();
        assert!(hint.contains("free memory"), "{hint}");
        assert!(
            !hint.contains("fsck") && !hint.contains("may read"),
            "{hint}"
        );
    }

    #[test]
    fn running_out_of_memory_building_a_delta_says_so() {
        let path = Path::new("objects/pack/pack-1.pack");
        says_out_of_memory(&delta_error(path, 12, DeltaError::no_memory()));
    }

    #[test]
    fn a_raw_delta_read_that_ran_out_of_memory_says_so() {
        let path = Path::new("objects/pack/pack-1.pack");
        let e = io::Error::new(io::ErrorKind::OutOfMemory, "inflating");
        says_out_of_memory(&delta_error(path, 12, DeltaError::Read(e)));
    }
}
