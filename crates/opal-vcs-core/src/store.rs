//! One view over every pack in the store. Packs are listed when the store opens, opened when first
//! searched, and searched on disk; at most `MAX_OPEN` are open at once, unless every open one is
//! being read.
use opal_platform::dir::Dir;

use crate::chunks::{self, CHUNK_FROM, ChunkList};
use crate::encoding::DecodeError;
use crate::id::{ContentHasher, Id};
use crate::listing::Listing;
use crate::pack::{HashingWriter, IndexEntry, ObjectKind, PackError, PackReader};
use crate::save::Save;
use crate::text::escape;
use crate::warning::{self, Warning};
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::ffi::{OsStr, OsString};
use std::io::{self, Write};
use std::ops::ControlFlow;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

/// Packs kept open between lookups, the last ones used: with the folders a walk holds, few enough
/// for any system's limit on open files.
const MAX_OPEN: usize = 16;
/// The first copy found of an object of up to this size is read straight away; a bigger one only
/// once every pack has been searched for a smaller copy.
const NEAR: u64 = 1 << 20;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error(transparent)]
    Pack(#[from] PackError),
    #[error(transparent)]
    Decode(#[from] DecodeError),
    #[error("object {0} is not in the store")]
    Missing(Id),
    #[error("object {id} is {found:?}, not {expected:?}")]
    WrongKind {
        id: Id,
        expected: ObjectKind,
        found: ObjectKind,
    },
    #[error("this save is too large to read safely: {detail}")]
    SaveTooLarge { detail: String },
}

/// A copy of an object: (pack number, its id).
type At = (usize, Id);

/// What a lookup takes as a copy of the object it wants.
#[derive(Clone, Copy)]
enum Wanted {
    /// An object of this kind.
    Kind(ObjectKind),
    /// A file: its content stored whole, or a chunk list.
    File,
    /// A chunk of a file: content stored whole, of the size its list gives. Never a chunk list, so
    /// a read goes no deeper than one list.
    Chunk(u64),
}

impl Wanted {
    fn takes(self, entry: &IndexEntry) -> bool {
        match self {
            Wanted::Kind(kind) => entry.kind == kind,
            Wanted::File => matches!(entry.kind, ObjectKind::Content | ObjectKind::ChunkList),
            Wanted::Chunk(size) => {
                entry.kind == ObjectKind::Content && entry.uncompressed_size == size
            }
        }
    }

    /// The kind a lookup that found only other objects names.
    fn kind(self) -> ObjectKind {
        match self {
            Wanted::Kind(kind) => kind,
            Wanted::File | Wanted::Chunk(_) => ObjectKind::Content,
        }
    }
}

/// A copy found in pack `n`. It doesn't hold the pack open, so the packs holding an object don't
/// all stay open.
#[derive(Clone)]
struct Copy {
    n: usize,
    entry: IndexEntry,
}

impl Copy {
    fn at(&self) -> At {
        (self.n, self.entry.id)
    }
}

/// A folder listing a second handle on the store read: its length as stored, and the pack and
/// index line it was read from.
#[derive(Debug)]
pub(crate) struct Peeked {
    listing: Listing,
    size: usize,
    pack: PathBuf,
    entry: IndexEntry,
}

impl Peeked {
    pub(crate) fn listing(&self) -> &Listing {
        &self.listing
    }

    pub(crate) fn size(&self) -> usize {
        self.size
    }
}

#[cfg(test)]
impl Peeked {
    /// One that names no pack, for tests of what holds them.
    pub(crate) fn of(listing: Listing, size: usize) -> Peeked {
        let id = listing.id();
        Peeked {
            listing,
            size,
            pack: PathBuf::new(),
            entry: IndexEntry {
                id,
                kind: ObjectKind::FolderListing,
                offset: 0,
                uncompressed_size: 0,
                stored_size: 0,
            },
        }
    }
}

/// What `listing_after` came to: the listing read ahead, or the bytes of a copy it read itself.
enum Got {
    Ready(Listing),
    Bytes(Vec<u8>),
}

/// How many index lookups and record reads this thread's stores made, to show what
/// `listing_after` is spared.
#[cfg(test)]
pub(crate) mod counted {
    use std::cell::Cell;

    thread_local! {
        static FINDS: Cell<usize> = const { Cell::new(0) };
        static RECORDS: Cell<usize> = const { Cell::new(0) };
        static LISTINGS: Cell<usize> = const { Cell::new(0) };
    }

    pub(super) fn listing() {
        LISTINGS.set(LISTINGS.get() + 1);
    }

    /// The listings this thread's stores were asked for, read ahead or not.
    pub(crate) fn listings() -> usize {
        LISTINGS.get()
    }

    pub(super) fn find() {
        FINDS.set(FINDS.get() + 1);
    }

    pub(super) fn record() {
        RECORDS.set(RECORDS.get() + 1);
    }

    /// The lookups and record reads so far.
    pub(crate) fn now() -> (usize, usize) {
        (FINDS.get(), RECORDS.get())
    }
}

pub struct Store {
    dir: Arc<Dir>,
    path: PathBuf,
    packs: Mutex<Packs>,
    checked: Mutex<Checked>,
    /// The smallest file stored in chunks, as the project's rules say.
    chunk_from: AtomicU64,
    #[cfg(test)]
    saves_read: std::sync::atomic::AtomicUsize,
    /// This store's packs while they are open.
    #[cfg(test)]
    counted: Arc<crate::pack::open_files::Count>,
}

/// The packs listed, and those open.
#[derive(Default)]
struct Packs {
    /// File names by pack number, in the order they were found; `None` for a pack gone since.
    names: Vec<Option<OsString>>,
    /// The numbers of the packs still there, in order of name.
    by_name: Vec<usize>,
    /// Open packs, the most recently used first.
    open: Vec<(usize, Arc<PackReader>)>,
    /// Packs being opened, which count as open.
    opening: usize,
    /// The pack that held the last object read.
    last: usize,
    /// Packs that failed to open, with the error, until the packs are listed again.
    failed: HashMap<usize, PackError>,
}

impl Packs {
    /// Adds the packs among `names` not listed yet and, unless `keep`, forgets those listed but
    /// not among them.
    fn list(&mut self, mut names: Vec<OsString>, keep: bool) {
        // A pack can be replaced under its name, by one this process writes, say.
        self.failed.clear();
        if !keep {
            let there: HashSet<&OsString> = names.iter().collect();
            for slot in &mut self.names {
                if slot.as_ref().is_some_and(|name| !there.contains(name)) {
                    *slot = None;
                }
            }
        }
        let known: HashSet<&OsString> = self.names.iter().flatten().collect();
        names.retain(|name| !known.contains(name));
        names.sort();
        self.names.extend(names.into_iter().map(Some));
        let names = &self.names;
        self.open
            .retain(|(n, _)| names.get(*n).is_some_and(Option::is_some));
        self.by_name = (0..names.len())
            .filter(|n| names.get(*n).is_some_and(Option::is_some))
            .collect();
        self.by_name
            .sort_by_key(|n| names.get(*n).cloned().flatten());
    }

    /// Pack `n` if it is open, made the most recently used.
    fn take_open(&mut self, n: usize) -> Option<Arc<PackReader>> {
        let i = self.open.iter().position(|(m, _)| *m == n)?;
        let open = self.open.remove(i);
        let pack = Arc::clone(&open.1);
        self.open.insert(0, open);
        Some(pack)
    }

    /// Closes the least recently used packs until one more can open. Only a pack no lookup is
    /// reading closes: lookups take theirs from this list, under its lock, so one only the list
    /// holds is free.
    fn make_room(&mut self) {
        while self.open.len() + self.opening >= MAX_OPEN {
            let free = self
                .open
                .iter()
                .rposition(|(_, p)| Arc::strong_count(p) == 1);
            // shortcut: with every open pack being read, one more opens past `MAX_OPEN`; that
            // takes more lookups at once than a command's workers and writer make.
            let Some(i) = free else { break };
            self.open.remove(i);
        }
    }
}

/// What reading copies has shown so far.
#[derive(Default)]
struct Checked {
    /// Copies that were read whole and passed every check.
    good: HashSet<At>,
    /// Copies that failed, each with the pack its problem was found in (its own, or for a chunk
    /// list a chunk's) and the problem. They are never read again.
    bad: HashMap<At, (PathBuf, String)>,
    /// Each pack that held a bad copy, with one problem found in it, the first by its text
    /// (`keep_first`).
    damaged: BTreeMap<PathBuf, String>,
    /// Each pack with a copy that couldn't be read for a reason other than damage, though another
    /// copy of the same object could, with one reason, the first by its text (`keep_first`).
    unreadable: BTreeMap<PathBuf, String>,
    /// Each pack that couldn't be opened or searched for a reason other than damage, with one
    /// reason, the first by its text (`keep_first`), and its kind.
    unreadable_packs: BTreeMap<PathBuf, (String, io::ErrorKind)>,
    /// Warnings about the store from the command itself, such as a merge that didn't finish.
    notes: Vec<Warning>,
}

/// What the copies of one object tried so far came to.
#[derive(Default)]
struct Tries {
    wrong_kind: Option<ObjectKind>,
    /// Every failure but a copy of another kind, as it came.
    failed: Vec<PackError>,
    unreadable: Vec<(PathBuf, String)>,
}

impl Tries {
    fn error(self, id: &Id, wanted: Wanted) -> StoreError {
        match (most(self.failed), self.wrong_kind) {
            (Some(e), _) => StoreError::Pack(e),
            (None, Some(found)) => StoreError::WrongKind {
                id: *id,
                expected: wanted.kind(),
                found,
            },
            (None, None) => StoreError::Missing(*id),
        }
    }
}

/// The most memory the chunks kept while one file is written may take.
const KEPT: usize = 8 << 20;
/// What keeping a chunk costs besides its bytes: its key, its map and queue slots, its allocation.
const PER_KEPT: usize = 128;

/// Chunks already read and checked while one file is written, so a list naming one again doesn't
/// read it again. The first read goes first when they would take more than `KEPT`.
#[derive(Default)]
struct Kept {
    chunks: HashMap<Id, Vec<u8>>,
    order: VecDeque<Id>,
    total: usize,
}

#[cfg(test)]
thread_local! {
    /// The most memory charged, and the most chunks kept, at once.
    static KEPT_MOST: std::cell::Cell<(usize, usize)> = const { std::cell::Cell::new((0, 0)) };
}

fn charge(bytes: &Vec<u8>) -> usize {
    bytes.capacity().saturating_add(PER_KEPT)
}

impl Kept {
    /// The chunk `id`, if it is kept and of `size` bytes.
    fn get(&self, id: &Id, size: u64) -> Option<&[u8]> {
        self.chunks
            .get(id)
            .filter(|bytes| bytes.len() as u64 == size)
            .map(Vec::as_slice)
    }

    /// Keeps `bytes` as the chunk `id` if there is room, letting the oldest go to make it. Only a
    /// shortcut: with no memory to spare, the chunk is just not kept.
    fn keep(&mut self, id: Id, bytes: Vec<u8>) {
        let cost = charge(&bytes);
        if cost > KEPT || self.chunks.contains_key(&id) {
            return;
        }
        while self.total.saturating_add(cost) > KEPT {
            let Some(oldest) = self.order.pop_front() else {
                break;
            };
            if let Some(gone) = self.chunks.remove(&oldest) {
                self.total = self.total.saturating_sub(charge(&gone));
            }
        }
        if self.chunks.try_reserve(1).is_err() || self.order.try_reserve(1).is_err() {
            return;
        }
        self.total = self.total.saturating_add(cost);
        self.order.push_back(id);
        self.chunks.insert(id, bytes);
        #[cfg(test)]
        KEPT_MOST.set({
            let (total, count) = KEPT_MOST.get();
            (total.max(self.total), count.max(self.chunks.len()))
        });
    }
}

impl Store {
    /// Lists every `*.pack` file in the folder at `packs_dir` and ignores everything else.
    pub fn open(packs_dir: &Path) -> Result<Store, StoreError> {
        let dir = Dir::open_top(packs_dir).map_err(PackError::from)?;
        Store::open_in(Arc::new(dir), packs_dir)
    }

    /// `open` for the folder `dir`, at `packs_dir`, read through `dir`.
    pub(crate) fn open_in(dir: Arc<Dir>, packs_dir: &Path) -> Result<Store, StoreError> {
        let store = Store {
            dir,
            path: packs_dir.to_owned(),
            packs: Mutex::default(),
            checked: Mutex::default(),
            chunk_from: AtomicU64::new(CHUNK_FROM),
            #[cfg(test)]
            saves_read: std::sync::atomic::AtomicUsize::new(0),
            #[cfg(test)]
            counted: Arc::default(),
        };
        store.list()?;
        Ok(store)
    }

    /// Lists the packs again, to find those another process committed since this store was
    /// opened.
    pub fn refresh(&mut self) -> Result<(), StoreError> {
        self.list()
    }

    /// Makes files of `bytes` or more (up to the most that is cut) stored in chunks from now on.
    pub(crate) fn set_chunk_from(&self, bytes: u64) {
        self.chunk_from.store(bytes, Ordering::Relaxed);
    }

    /// Whether a file of `size` bytes is stored in chunks.
    pub(crate) fn is_cut(&self, size: u64) -> bool {
        chunks::is_cut_from(self.chunk_from.load(Ordering::Relaxed), size)
    }

    /// Whether a file of `size` bytes is read whole into memory: under 1 MiB, whatever the
    /// project's rules say.
    pub(crate) fn is_small(&self, size: u64) -> bool {
        !chunks::is_cut(size) && !chunks::is_too_long_to_cut(size)
    }

    /// Whether a file of `size` bytes is stored whole by streaming it: neither small nor cut.
    pub(crate) fn is_streamed(&self, size: u64) -> bool {
        !self.is_small(size) && !self.is_cut(size)
    }

    /// Adds the pack at `pack_path`, which this process has just moved into the store. An object
    /// an earlier pack holds too gets a further copy here, read if the earlier ones fail.
    pub fn register(&mut self, pack_path: &Path) {
        if let Some(name) = pack_path.file_name() {
            self.packs().list(vec![name.to_owned()], true);
        }
    }

    fn list(&self) -> Result<(), StoreError> {
        let mut names = Vec::new();
        for entry in self.dir.entries().map_err(PackError::from)? {
            if is_pack_name(&entry.name) {
                names.push(entry.name);
            }
        }
        self.packs().list(names, false);
        Ok(())
    }

    #[must_use]
    pub fn pack_count(&self) -> usize {
        self.packs().by_name.len()
    }

    /// Whether some index line of `kind`, in any pack, passes `test`.
    pub(crate) fn any_entry<E: From<StoreError>>(
        &self,
        kind: ObjectKind,
        mut test: impl FnMut(&Id) -> Result<bool, E>,
    ) -> Result<bool, E> {
        let mut skipped = Vec::new();
        let found = self.each_pack(&mut skipped, |_, pack| {
            for e in pack.entries() {
                let e = e.map_err(StoreError::from)?;
                if e.kind == kind && test(&e.id)? {
                    return Ok::<_, E>(ControlFlow::Break(()));
                }
            }
            Ok(ControlFlow::Continue(()))
        })?;
        match (found, most(skipped)) {
            (Some(()), _) => Ok(true),
            // A pack that couldn't be read may hold one.
            (None, Some(e)) => Err(StoreError::from(e).into()),
            (None, None) => Ok(false),
        }
    }

    /// The ID of every index line of `kind`. A pack that can't be opened or searched for a reason
    /// other than damage is passed over whole, with its warning; damage is the error.
    pub(crate) fn ids_of(&self, kind: ObjectKind) -> Result<HashSet<Id>, StoreError> {
        let mut ids = HashSet::new();
        let (mut skipped, mut unread) = (Vec::new(), Vec::new());
        self.each_pack(&mut skipped, |n, pack| {
            // Kept only once the whole index has read: the lines before a failure name entries
            // that may not read either.
            let mut found = Vec::new();
            for e in pack.entries() {
                match e {
                    Ok(e) if e.kind == kind => found.push(e.id),
                    Ok(_) => {}
                    Err(e) => {
                        unread.push((n, e));
                        return Ok(ControlFlow::Continue(()));
                    }
                }
            }
            ids.extend(found);
            Ok::<_, StoreError>(ControlFlow::<()>::Continue(()))
        })?;
        for (n, e) in unread {
            self.pass_over(n, e, &mut skipped);
        }
        match skipped.into_iter().find(is_damage) {
            Some(e) => Err(e.into()),
            None => Ok(ids),
        }
    }

    /// Whether any pack holds a file's content, as its index lists it.
    pub fn holds_content(&self) -> Result<bool, StoreError> {
        self.any_entry(ObjectKind::Content, |_| Ok(true))
    }

    /// Whether a copy of `id` reads back whole and matches its fingerprint, so storing it can be
    /// skipped. A copy recording another `size` than the object's is damage, and is never read.
    /// Never a chunk list: a writer relies on one only once it has cut the file into the same
    /// chunks (`holds_chunk_list`).
    pub fn has_good_copy(&self, id: &Id, kind: ObjectKind, size: u64) -> Result<bool, StoreError> {
        if kind == ObjectKind::ChunkList {
            return Ok(false);
        }
        let (wanted, mut tries) = (Wanted::Kind(kind), Tries::default());
        let check = read_whole(kind);
        let mut skipped = Vec::new();
        let found = self.search(id, &mut skipped, |c| {
            if c.entry.kind == kind && c.entry.uncompressed_size != size {
                let wrong = PackError::Corrupt {
                    path: self.pack_path(c.n),
                    reason: format!("its copy of {id} records the wrong size"),
                };
                self.note_damage(c.at(), self.pack_path(c.n), &wrong);
                return ControlFlow::Continue(());
            }
            let good = c.entry.kind == kind && self.is_good(&c);
            let read = |pack: Arc<PackReader>, entry: &IndexEntry| check(&pack, entry);
            if good || self.attempt(&c, wanted, &mut tries, read).is_ok() {
                return ControlFlow::Break(());
            }
            ControlFlow::Continue(())
        })?;
        // A pack or copy that couldn't be read for a reason other than damage may hold a good
        // copy, so its error is the answer. Damage isn't: storing the object again is harmless.
        match (found, most(tries.failed.into_iter().chain(skipped))) {
            (Some(()), _) => Ok(true),
            (None, Some(e)) if !is_damage(&e) && !is_gone(&e) => Err(e.into()),
            (None, _) => Ok(false),
        }
    }

    /// Whether a chunk list stored under `list.file` is exactly `list`. Each copy is checked by its
    /// structure only, so a writer relies on one only once it has cut the file into the same
    /// chunks, each checked or stored as it went.
    pub fn holds_chunk_list(&self, list: &ChunkList) -> Result<bool, StoreError> {
        let (mut failed, mut skipped) = (Vec::new(), Vec::new());
        let found = self.search(&list.file, &mut skipped, |c| {
            let known_bad = self.checked().bad.contains_key(&c.at());
            if c.entry.kind != ObjectKind::ChunkList
                || c.entry.uncompressed_size != list.size
                || known_bad
            {
                return ControlFlow::Continue(());
            }
            let same = self.pack(c.n).and_then(|pack| {
                // Compared as it is read, so a long list is never held twice. Another is read
                // whole, so damage in it is told.
                if pack.holds_chunk_list(&c.entry, list)? {
                    return Ok(true);
                }
                pack.read_chunk_list(&c.entry).map(|stored| stored == *list)
            });
            match same {
                Ok(true) => return ControlFlow::Break(()),
                Ok(false) => {}
                Err(e) => {
                    if is_damage(&e) {
                        self.note_damage(c.at(), self.pack_path(c.n), &e);
                    }
                    failed.push(e);
                }
            }
            ControlFlow::Continue(())
        })?;
        // As for `has_good_copy`: a copy that couldn't be read may be this list.
        match (found, most(failed.into_iter().chain(skipped))) {
            (Some(()), _) => Ok(true),
            (None, Some(e)) if !is_damage(&e) && !is_gone(&e) => Err(e.into()),
            (None, _) => Ok(false),
        }
    }

    /// The packs found holding a bad copy of an object, each with one problem found in it, the
    /// first by its text.
    #[must_use]
    pub fn damaged_packs(&self) -> Vec<PackError> {
        self.checked()
            .damaged
            .iter()
            .map(|(path, reason)| PackError::Corrupt {
                path: path.clone(),
                reason: reason.clone(),
            })
            .collect()
    }

    /// A warning for each pack found holding a bad copy of an object, then one for each pack with
    /// a copy that couldn't be read, though another copy could, then one for each pack that
    /// couldn't be opened or searched for a reason other than damage.
    #[must_use]
    pub fn warnings(&self) -> Vec<Warning> {
        let mut warnings: Vec<Warning> = self
            .damaged_packs()
            .iter()
            .map(|pack| Warning::new(warning::PACK_DAMAGED, format!("{pack}.")))
            .collect();
        let checked = self.checked();
        warnings.extend(checked.unreadable.iter().map(|(path, reason)| {
            Warning::new(
                warning::COPY_UNREADABLE,
                format!(
                    "Opalro couldn't read a copy in pack {}: {}.",
                    escape(&path.display().to_string(), false),
                    escape(reason.trim_end_matches('.'), false)
                ),
            )
        }));
        warnings.extend(
            checked
                .unreadable_packs
                .iter()
                .map(|(path, (reason, kind))| {
                    // The same pack reads fine with more memory.
                    let next = if *kind == io::ErrorKind::OutOfMemory {
                        "closing other programs may free enough memory to read it"
                    } else {
                        "check that the file can be read"
                    };
                    Warning::new(
                        warning::PACK_UNREADABLE,
                        format!(
                            "Opalro couldn't read pack {}: {}. It went on without it; {next}.",
                            escape(&path.display().to_string(), false),
                            escape(reason.trim_end_matches('.'), false)
                        ),
                    )
                }),
        );
        warnings.extend(checked.notes.iter().cloned());
        warnings
    }

    /// Keeps `warning` to come with the others, once however often it is given: a command that
    /// tries again (a save asking for a line's name) meets what it met the first time.
    pub fn note(&self, warning: Warning) {
        let notes = &mut self.checked().notes;
        if !notes.contains(&warning) {
            notes.push(warning);
        }
    }

    /// Closes every pack kept open, so they can be removed; they are opened again when searched.
    pub(crate) fn close_all(&self) {
        self.packs().open.clear();
    }

    /// Reads a structured record of the given kind, fingerprint checked.
    pub fn read_record(&self, id: &Id, kind: ObjectKind) -> Result<Vec<u8>, StoreError> {
        self.first_good(id, kind, false, |pack, entry| pack.read_record(entry))
    }

    /// Streams a file into `out`, from its content stored whole or from its chunk list; treat the
    /// output as untrusted until this returns `Ok`.
    pub fn copy_content(&self, id: &Id, out: &mut dyn Write) -> Result<(), StoreError> {
        self.copy(id, Wanted::File, out)
    }

    /// `copy_content`, of a file or of one of its chunks.
    fn copy(&self, id: &Id, wanted: Wanted, out: &mut dyn Write) -> Result<(), StoreError> {
        let mut first = self.first_copy(id)?;
        let mut tries = Tries::default();
        // Bytes can't be taken back out of `out`, so it gets one copy only.
        let known_good = |c: &Copy| wanted.takes(&c.entry) && self.is_good(c);
        if let Some(c) = &first
            && !known_good(c)
        {
            // A small first copy, and only that one, is read whole before anything goes out.
            if wanted.takes(&c.entry) && c.entry.uncompressed_size <= NEAR {
                let mut bytes = Vec::new();
                let read = self.attempt(c, wanted, &mut tries, |pack, entry| {
                    bytes.clear();
                    self.copy_file(pack, entry, &mut bytes)
                });
                if read.is_ok() {
                    return Ok(out.write_all(&bytes).map_err(PackError::from)?);
                }
            }
            // Otherwise it is streamed: when several packs hold it, a good copy is found first and
            // read first, as one that fails may fail again for a reason that isn't damage.
            if self.all_copies(id)?.0.len() > 1 {
                let tried = std::mem::take(&mut tries);
                let (_, good) = self.first_good_from(
                    first.clone(),
                    tried,
                    id,
                    wanted,
                    false,
                    |pack, entry| self.copy_file(pack, entry, &mut io::sink()),
                )?;
                first = Some(good);
            }
        }
        self.first_good_from(first, tries, id, wanted, true, |pack, entry| {
            self.copy_file(pack, entry, out)
        })
        .map(drop)
    }

    /// Writes the copy `entry` of a file, in `pack`, to `out`: content as it is stored, a chunk
    /// list chunk by chunk.
    fn copy_file(
        &self,
        pack: Arc<PackReader>,
        entry: &IndexEntry,
        out: &mut dyn Write,
    ) -> Result<(), PackError> {
        if entry.kind != ObjectKind::ChunkList {
            return pack.copy_content(entry, out);
        }
        let list = pack.read_chunk_list(entry)?;
        let at = pack.path().to_owned();
        // Let go of before the chunks are found, so the packs kept open are all that are open.
        drop(pack);
        self.copy_chunks(&list, &at, out)
    }

    /// Writes the file `list` names to `out`, each chunk read and checked as content stored whole
    /// of the size the list gives, and checks the whole against the file's ID. A list that can't
    /// build its file is damage in the pack at `at`, which holds it; `out` is untrusted until
    /// this returns `Ok`.
    pub(crate) fn copy_chunks(
        &self,
        list: &ChunkList,
        at: &Path,
        out: &mut dyn Write,
    ) -> Result<(), PackError> {
        let damaged = |reason: String| PackError::Corrupt {
            path: at.to_owned(),
            reason,
        };
        let mut hashing = HashingWriter {
            inner: out,
            hasher: ContentHasher::new(),
        };
        let mut kept = Kept::default();
        for chunk in &list.chunks {
            let copied = match kept.get(&chunk.id, chunk.size) {
                Some(bytes) => hashing
                    .write_all(bytes)
                    .map_err(|e| PackError::from(e).into()),
                None if chunk.size <= NEAR => {
                    self.read_chunk(&chunk.id, chunk.size).and_then(|bytes| {
                        hashing.write_all(&bytes).map_err(PackError::from)?;
                        kept.keep(chunk.id, bytes);
                        Ok(())
                    })
                }
                // Opalro cuts no chunk this big; one a list names anyway is streamed, never held.
                None => self.copy(&chunk.id, Wanted::Chunk(chunk.size), &mut hashing),
            };
            match copied {
                Ok(()) => {}
                // Damage in the chunk's own pack is noted there; memory and I/O aren't damage.
                Err(StoreError::Pack(e)) => return Err(e),
                Err(_) => {
                    return Err(damaged(format!(
                        "the chunk list of {} names chunk {} of {} bytes, which isn't in the store",
                        list.file, chunk.id, chunk.size
                    )));
                }
            }
        }
        if hashing.hasher.finish() != list.file {
            return Err(damaged(format!(
                "the chunks the list of {} names build another file",
                list.file
            )));
        }
        Ok(())
    }

    /// A chunk of `size` bytes, read whole and checked. Nothing has gone out yet, so a copy that
    /// fails can still give way to another.
    fn read_chunk(&self, id: &Id, size: u64) -> Result<Vec<u8>, StoreError> {
        let first = self.first_copy(id)?;
        let wanted = Wanted::Chunk(size);
        self.first_good_from(first, Tries::default(), id, wanted, false, |pack, entry| {
            pack.read_content(entry)
        })
        .map(|(bytes, _)| bytes)
    }

    pub fn listing(&self, id: &Id) -> Result<Listing, StoreError> {
        #[cfg(test)]
        counted::listing();
        Ok(Listing::decode(
            &self.read_record(id, ObjectKind::FolderListing)?,
        )?)
    }

    /// `listing`, given what a second handle read of `id`: the same read through the same steps,
    /// so this store notes what `listing` would. It is spared only the lookup in the pack asked
    /// first when that is the pack `peeked` came from, and the read of `peeked`'s own copy.
    pub(crate) fn listing_after(&self, id: &Id, peeked: Peeked) -> Result<Listing, StoreError> {
        #[cfg(test)]
        counted::listing();
        let Peeked {
            listing,
            pack: at,
            entry: seen,
            ..
        } = peeked;
        // A pack is named by its index's checksum and never changes once named, so the pack of
        // that name holds the same lines and bytes through any handle.
        let first = match self.last_is(&at).filter(|_| seen.id == *id) {
            Some(n) => Some(Copy { n, entry: seen }),
            None => self.first_copy(id)?,
        };
        let mut ready = Some(listing);
        let wanted = Wanted::Kind(ObjectKind::FolderListing);
        let (got, _) =
            self.first_good_from(first, Tries::default(), id, wanted, false, |pack, entry| {
                if pack.path() == at
                    && *entry == seen
                    && let Some(listing) = ready.take()
                {
                    return Ok(Got::Ready(listing));
                }
                #[cfg(test)]
                counted::record();
                pack.read_record(entry).map(Got::Bytes)
            })?;
        match got {
            Got::Ready(listing) => Ok(listing),
            Got::Bytes(bytes) => Ok(Listing::decode(&bytes)?),
        }
    }

    /// The pack `packs.last`, if `first_copy` would ask it first (it is listed and open) and it is
    /// the pack at `path`. Changes nothing: reading the copy moves the pack up the open list.
    fn last_is(&self, path: &Path) -> Option<usize> {
        let packs = self.packs();
        let n = packs.last;
        let (_, open) = packs.open.iter().find(|(m, _)| *m == n)?;
        (packs.by_name.contains(&n) && open.path() == path).then_some(n)
    }

    /// A second handle on this store's packs, sharing only the folder: its own list of packs,
    /// copied from this one, its own open packs and notes. Reads through it leave this store
    /// exactly as it was.
    pub(crate) fn second(&self) -> Store {
        let names: Vec<OsString> = self.packs().names.iter().flatten().cloned().collect();
        let mut packs = Packs::default();
        packs.list(names, false);
        Store {
            dir: Arc::clone(&self.dir),
            path: self.path.clone(),
            packs: Mutex::new(packs),
            checked: Mutex::default(),
            chunk_from: AtomicU64::new(self.chunk_from.load(Ordering::Relaxed)),
            #[cfg(test)]
            saves_read: std::sync::atomic::AtomicUsize::new(0),
            #[cfg(test)]
            counted: Arc::default(),
        }
    }

    /// `listing` as it reads a good first copy, with the copy's length and where it is, or `None`
    /// on any failure. It tries that one copy only and notes nothing about copies or packs, so a
    /// `listing` after it tells of them as it would have.
    pub(crate) fn peek_listing(&self, id: &Id) -> Option<Peeked> {
        let mut quiet = Vec::new();
        let found = self.each_pack_in(&mut quiet, true, |n, pack| {
            let copy = match pack.find(id) {
                Ok(Some(entry)) => Some((n, Arc::clone(pack), entry)),
                Ok(None) => return Ok::<_, StoreError>(ControlFlow::Continue(())),
                Err(_) => None,
            };
            Ok(ControlFlow::Break(copy))
        });
        let Ok(Some(Some((n, pack, entry)))) = found else {
            return None;
        };
        // Only a copy `first_good_from` reads first, so one that passes is the one it returns.
        if entry.kind != ObjectKind::FolderListing
            || entry.uncompressed_size > NEAR
            || self.checked().bad.contains_key(&(n, entry.id))
        {
            return None;
        }
        let bytes = pack.read_record(&entry).ok()?;
        Some(Peeked {
            listing: Listing::decode(&bytes).ok()?,
            size: bytes.len(),
            pack: pack.path().to_owned(),
            entry,
        })
    }

    pub fn save(&self, id: &Id, signatures_allowed: bool) -> Result<Save, StoreError> {
        #[cfg(test)]
        self.saves_read
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Ok(Save::decode(
            &self.read_record(id, ObjectKind::Save)?,
            signatures_allowed,
        )?)
    }

    /// Reads copies of `id` with `read` until one passes. The first copy found is read first when
    /// it is small or known good; otherwise, or when it fails, every pack is searched and the
    /// copies are tried known good first, then smallest first, so a copy claiming a huge size
    /// comes last. With `once`, the first copy read is the last.
    fn first_good<T>(
        &self,
        id: &Id,
        kind: ObjectKind,
        once: bool,
        read: impl FnMut(Arc<PackReader>, &IndexEntry) -> Result<T, PackError>,
    ) -> Result<T, StoreError> {
        let first = self.first_copy(id)?;
        let wanted = Wanted::Kind(kind);
        self.first_good_from(first, Tries::default(), id, wanted, once, read)
            .map(|(value, _)| value)
    }

    /// `first_good`, given the first copy found and what reading copies has come to so far. Gives
    /// the copy that passed too.
    fn first_good_from<T>(
        &self,
        first: Option<Copy>,
        mut tries: Tries,
        id: &Id,
        wanted: Wanted,
        once: bool,
        mut read: impl FnMut(Arc<PackReader>, &IndexEntry) -> Result<T, PackError>,
    ) -> Result<(T, Copy), StoreError> {
        let Some(first) = first else {
            return Err(StoreError::Missing(*id));
        };
        let near = first.entry.uncompressed_size <= NEAR || self.is_good(&first);
        if near {
            match self.attempt(&first, wanted, &mut tries, &mut read) {
                Ok(value) => return Ok((value, first)),
                Err(true) if once => return Err(tries.error(id, wanted)),
                Err(_) => {}
            }
        }
        let (mut copies, skipped) = self.all_copies(id)?;
        tries.failed.extend(skipped);
        if near {
            copies.retain(|c| c.n != first.n);
        }
        {
            let checked = self.checked();
            copies.sort_by_key(|c| (!checked.good.contains(&c.at()), c.entry.uncompressed_size));
        }
        for c in copies {
            match self.attempt(&c, wanted, &mut tries, &mut read) {
                Ok(value) => return Ok((value, c)),
                Err(true) if once => break,
                Err(_) => {}
            }
        }
        Err(tries.error(id, wanted))
    }

    /// Reads the copy `c` with `read`, unless it isn't what `wanted` takes or is known bad, noting
    /// what happened in `tries`. A failure says whether the copy was read. `read` is given the
    /// pack, which it may let go of once it no longer needs it.
    fn attempt<T>(
        &self,
        c: &Copy,
        wanted: Wanted,
        tries: &mut Tries,
        mut read: impl FnMut(Arc<PackReader>, &IndexEntry) -> Result<T, PackError>,
    ) -> Result<T, bool> {
        if !wanted.takes(&c.entry) {
            tries.wrong_kind.get_or_insert(c.entry.kind);
            return Err(false);
        }
        let path = || self.pack_path(c.n);
        // A bad copy is never read again but still counts, so no good copy is damage every time.
        let known_bad = self.checked().bad.get(&c.at()).cloned();
        if let Some((path, reason)) = known_bad {
            tries.failed.push(PackError::Corrupt { path, reason });
            return Err(false);
        }
        match self.pack(c.n).and_then(|pack| read(pack, &c.entry)) {
            Ok(value) => {
                self.packs().last = c.n;
                let mut checked = self.checked();
                checked.good.insert(c.at());
                // Worth a warning only now: with no good copy the read fails, saying why.
                for (path, reason) in tries.unreadable.drain(..) {
                    keep_first(&mut checked.unreadable, path, reason);
                }
                Ok(value)
            }
            Err(e) if is_damage(&e) => {
                // Met in a chunk this copy's list names, it is another pack's, noted there: this
                // copy is only passed over from now on.
                if let PackError::Corrupt {
                    path: there,
                    reason,
                } = &e
                    && *there != path()
                {
                    let failed = (there.clone(), reason.clone());
                    self.checked().bad.insert(c.at(), failed);
                    tries.failed.push(e);
                    return Err(true);
                }
                let reason = self.note_damage(c.at(), path(), &e);
                tries.failed.push(PackError::Corrupt {
                    path: path(),
                    reason,
                });
                Err(true)
            }
            // Memory or I/O: the copy may be fine, so it is neither remembered nor damage.
            Err(e) => {
                tries.unreadable.push((path(), e.to_string()));
                tries.failed.push(e);
                Err(true)
            }
        }
    }

    fn is_good(&self, c: &Copy) -> bool {
        self.checked().good.contains(&c.at())
    }

    /// The first copy of `id` found: in the pack that held the last object read, else in the
    /// first pack by name that holds it.
    /// A pack that couldn't be read may hold `id`, so without a copy its error is the answer.
    fn first_copy(&self, id: &Id) -> Result<Option<Copy>, StoreError> {
        let mut skipped = Vec::new();
        match (
            self.search(id, &mut skipped, ControlFlow::Break)?,
            most(skipped),
        ) {
            (None, Some(e)) => Err(e.into()),
            (found, _) => Ok(found),
        }
    }

    /// Every copy of `id` found, and the errors of the packs that couldn't be read.
    fn all_copies(&self, id: &Id) -> Result<(Vec<Copy>, Vec<PackError>), StoreError> {
        let mut copies = Vec::new();
        let mut skipped = Vec::new();
        self.search(id, &mut skipped, |c| {
            copies.push(c);
            ControlFlow::<()>::Continue(())
        })?;
        Ok((copies, skipped))
    }

    /// Hands `visit` each copy of `id`, in the order `first_copy` finds them, until it breaks. A
    /// pack whose index can't be searched is passed over as `each_pack` passes one over.
    fn search<T>(
        &self,
        id: &Id,
        skipped: &mut Vec<PackError>,
        mut visit: impl FnMut(Copy) -> ControlFlow<T>,
    ) -> Result<Option<T>, StoreError> {
        let mut unsearched = Vec::new();
        let found = self.each_pack(skipped, |n, pack| {
            #[cfg(test)]
            counted::find();
            let entry = match pack.find(id) {
                Ok(Some(entry)) => entry,
                Ok(None) => return Ok(ControlFlow::Continue(())),
                Err(e) => {
                    unsearched.push((n, e));
                    return Ok(ControlFlow::Continue(()));
                }
            };
            Ok::<_, StoreError>(visit(Copy { n, entry }))
        })?;
        for (n, e) in unsearched {
            self.pass_over(n, e, skipped);
        }
        Ok(found)
    }

    /// Notes pack `n`, which couldn't be opened or searched, as damaged or unreadable as `e`
    /// says, and keeps `e` in `skipped`, naming the pack.
    fn pass_over(&self, n: usize, e: PackError, skipped: &mut Vec<PackError>) {
        let path = self.pack_path(n);
        if let PackError::Corrupt { reason, .. } = &e {
            let reason = reason.clone();
            keep_first(&mut self.checked().damaged, path, reason);
        } else if !is_damage(&e) && !is_gone(&e) {
            let kind = match &e {
                PackError::Io(e) => e.kind(),
                PackError::OutOfMemory => io::ErrorKind::OutOfMemory,
                _ => io::ErrorKind::Other,
            };
            let shown = escape(&path.display().to_string(), false);
            let said = format!("Opalro couldn't read pack {shown}: {e}");
            let reason = (e.to_string(), kind);
            keep_first(&mut self.checked().unreadable_packs, path, reason);
            skipped.push(io::Error::new(kind, said).into());
            return;
        }
        skipped.push(e);
    }

    /// Hands `f` each pack, first the one that held the last object read, then the others open,
    /// then the closed ones by name, until it breaks. Open ones first, so a lookup that finds
    /// nothing opens only the packs that don't fit among the open ones. A pack gone when it is
    /// opened (merged into another) sends the store to list the packs again, once; a pack already
    /// handed out isn't handed out again. A pack that can't be opened is passed over, noted, and
    /// its error kept in `skipped`: a lookup can still find a good copy elsewhere, but can't tell
    /// that the pack doesn't hold one.
    fn each_pack<T, E: From<StoreError>>(
        &self,
        skipped: &mut Vec<PackError>,
        f: impl FnMut(usize, &Arc<PackReader>) -> Result<ControlFlow<T>, E>,
    ) -> Result<Option<T>, E> {
        self.each_pack_in(skipped, false, f)
    }

    /// `each_pack`, or, when `quiet`, one that ends at the first pack it can't open, noting
    /// nothing and listing nothing again.
    fn each_pack_in<T, E: From<StoreError>>(
        &self,
        skipped: &mut Vec<PackError>,
        quiet: bool,
        mut f: impl FnMut(usize, &Arc<PackReader>) -> Result<ControlFlow<T>, E>,
    ) -> Result<Option<T>, E> {
        let mut done = HashSet::new();
        // The pack that held the last object, if open, is asked before the order is built: it
        // goes first in that order, and most lookups end there.
        let last = {
            let mut packs = self.packs();
            let n = packs.last;
            if packs.by_name.contains(&n) {
                packs.take_open(n).map(|pack| (n, pack))
            } else {
                None
            }
        };
        if let Some((n, pack)) = last {
            if let ControlFlow::Break(value) = f(n, &pack)? {
                return Ok(Some(value));
            }
            done.insert(n);
        }
        let mut listed_again = false;
        loop {
            let order: Vec<usize> = {
                let packs = self.packs();
                let open: Vec<usize> = packs.open.iter().map(|(n, _)| *n).collect();
                let last = packs.by_name.contains(&packs.last).then_some(packs.last);
                // The last one goes first only if open: opening it first would close an open
                // pack this lookup hasn't searched yet. Packs handed out twice are skipped below.
                let last_open = last.filter(|n| open.contains(n));
                (last_open.into_iter().chain(open).chain(last))
                    .chain(packs.by_name.iter().copied())
                    .collect()
            };
            let mut gone = false;
            for n in order {
                if !done.insert(n) {
                    continue;
                }
                let pack = match self.pack(n) {
                    Ok(pack) => pack,
                    Err(_) if quiet => return Ok(None),
                    Err(PackError::Io(e))
                        if e.kind() == io::ErrorKind::NotFound && !listed_again =>
                    {
                        gone = true;
                        break;
                    }
                    Err(e) => {
                        self.pass_over(n, e, skipped);
                        continue;
                    }
                };
                if let ControlFlow::Break(value) = f(n, &pack)? {
                    return Ok(Some(value));
                }
            }
            if !gone {
                return Ok(None);
            }
            listed_again = true;
            self.list()?;
        }
    }

    /// Pack `n`, kept open from an earlier lookup if it was, else opened and kept open, closing
    /// the least recently used that no lookup is reading. One that failed to open fails again
    /// unopened.
    fn pack(&self, n: usize) -> Result<Arc<PackReader>, PackError> {
        let name = {
            let mut packs = self.packs();
            if let Some(pack) = packs.take_open(n) {
                return Ok(pack);
            }
            if let Some(e) = packs.failed.get(&n) {
                return Err(again(e));
            }
            // Closed first, and counted while it opens, so no more than `MAX_OPEN` are open even
            // for a moment, unless every open one is being read.
            packs.make_room();
            packs.opening += 1;
            packs.names.get(n).cloned().flatten()
        };
        let opened = match name {
            Some(name) => PackReader::open_in(&self.dir, &name, &self.path.join(&name)),
            None => Err(io::Error::from(io::ErrorKind::NotFound).into()),
        };
        let mut packs = self.packs();
        packs.opening -= 1;
        match opened {
            #[cfg_attr(not(test), allow(unused_mut))]
            Ok(mut pack) => {
                #[cfg(test)]
                pack.count_in(&self.counted);
                // Another thread may have opened it meanwhile: the one kept is used.
                if let Some(there) = packs.take_open(n) {
                    return Ok(there);
                }
                let pack = Arc::new(pack);
                packs.open.insert(0, (n, Arc::clone(&pack)));
                Ok(pack)
            }
            Err(e) => {
                if !is_gone(&e) {
                    packs.failed.insert(n, again(&e));
                }
                Err(e)
            }
        }
    }

    /// Where pack `n` is, to name it.
    fn pack_path(&self, n: usize) -> PathBuf {
        let name = self.packs().names.get(n).cloned().flatten();
        name.map_or_else(|| self.path.clone(), |name| self.path.join(name))
    }

    /// How many saves this store has been asked for.
    #[cfg(test)]
    pub(crate) fn saves_read(&self) -> usize {
        self.saves_read.load(std::sync::atomic::Ordering::Relaxed)
    }

    #[cfg(test)]
    fn open_count(&self) -> usize {
        self.packs().open.len()
    }

    /// Everything this store remembers about its packs and copies, packs by name, in a fixed
    /// order.
    #[cfg(test)]
    pub(crate) fn remembered(&self) -> String {
        let packs = self.packs();
        let names = packs.names.clone();
        let name = |n: &usize| names.get(*n).cloned().flatten();
        let open: Vec<_> = packs.open.iter().map(|(n, _)| name(n)).collect();
        let failed: BTreeMap<_, _> = (packs.failed.iter())
            .map(|(n, e)| (name(n), e.to_string()))
            .collect();
        let last = name(&packs.last);
        drop(packs);
        let c = self.checked();
        let at = |(n, id): &At| (name(n), *id);
        let good: std::collections::BTreeSet<_> = c.good.iter().map(at).collect();
        let bad: BTreeMap<_, _> = c.bad.iter().map(|(a, why)| (at(a), why)).collect();
        format!(
            "last {last:?}\nopen {open:?}\nfailed {failed:?}\ngood {good:?}\nbad {bad:?}\n{:?}\n{:?}\n{:?}\n{:?}",
            c.damaged, c.unreadable, c.unreadable_packs, c.notes
        )
    }

    /// Marks a copy bad and its pack damaged, and returns the problem found.
    fn note_damage(&self, at: At, path: PathBuf, e: &PackError) -> String {
        let reason = match e {
            PackError::Corrupt { reason, .. } => reason.clone(),
            other => other.to_string(),
        };
        let mut checked = self.checked();
        checked.bad.insert(at, (path.clone(), reason.clone()));
        keep_first(&mut checked.damaged, path, reason.clone());
        reason
    }

    fn packs(&self) -> MutexGuard<'_, Packs> {
        self.packs.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn checked(&self) -> MutexGuard<'_, Checked> {
        self.checked.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Reads a copy of `kind` whole, for its checks, and keeps nothing.
pub(crate) fn read_whole(
    kind: ObjectKind,
) -> impl Fn(&PackReader, &IndexEntry) -> Result<(), PackError> {
    move |pack, entry| match kind {
        ObjectKind::Content => pack.copy_content(entry, &mut io::sink()),
        _ => pack.read_record(entry).map(drop),
    }
}

/// Whether `name`, in the packs folder, is a pack's. Reads look in every one, so `tidy` must
/// judge every one: this is the one place that says which.
pub(crate) fn is_pack_name(name: &OsStr) -> bool {
    Path::new(name).extension().is_some_and(|e| e == "pack")
}

/// A copy that fails its own checks is damage; an I/O error or a lack of memory says nothing
/// about the copy.
pub(crate) fn is_damage(e: &PackError) -> bool {
    matches!(
        e,
        PackError::FingerprintMismatch { .. } | PackError::Corrupt { .. }
    )
}

/// A pack gone since it was listed, merged into another, say.
fn is_gone(e: &PackError) -> bool {
    matches!(e, PackError::Io(e) if e.kind() == io::ErrorKind::NotFound)
}

/// The failure of `failed` that matters most: one that says nothing about the copy, which may be
/// fine (calling it damage would mislead), then damage, then a pack gone, which says nothing at
/// all. Of those alike, the first by what it says, so lookups on several threads, which meet
/// copies in other orders, still tell the same one.
fn most(failed: impl IntoIterator<Item = PackError>) -> Option<PackError> {
    let rank = |e: &PackError| match e {
        e if is_gone(e) => 0,
        e if is_damage(e) => 1,
        _ => 2,
    };
    let told = failed.into_iter().map(|e| (rank(&e), e.to_string(), e));
    told.reduce(|kept, e| {
        if (e.0, &kept.1) > (kept.0, &e.1) {
            e
        } else {
            kept
        }
    })
    .map(|(_, _, e)| e)
}

/// Keeps `reason` for `path` unless one that comes first by what it says is kept, so which is
/// kept doesn't hang on the order lookups on several threads meet them in.
fn keep_first<T: Ord>(kept: &mut BTreeMap<PathBuf, T>, path: PathBuf, reason: T) {
    match kept.get_mut(&path) {
        Some(was) if reason < *was => *was = reason,
        Some(_) => {}
        None => {
            kept.insert(path, reason);
        }
    }
}

/// `e` once more, for a pack that failed to open before.
fn again(e: &PackError) -> PackError {
    match e {
        PackError::Io(e) => io::Error::new(e.kind(), e.to_string()).into(),
        PackError::Corrupt { path, reason } => PackError::Corrupt {
            path: path.clone(),
            reason: reason.clone(),
        },
        PackError::FingerprintMismatch { id } => PackError::FingerprintMismatch { id: *id },
        PackError::ContentChanged => PackError::ContentChanged,
        PackError::TooLarge => PackError::TooLarge,
        PackError::OutOfMemory => PackError::OutOfMemory,
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;
    use crate::chunks::Chunk;
    use crate::id::hash_content;
    use crate::pack::{PackWriter, Prepared, open_files};
    use crate::repo::RepoError;
    use crate::test_support::{lock_pack, plant_damaged_copies};
    use proptest::prelude::*;

    fn dirs() -> (tempfile::TempDir, std::path::PathBuf, std::path::PathBuf) {
        let root = tempfile::tempdir().unwrap();
        let tmp = root.path().join("tmp");
        let packs = root.path().join("packs");
        std::fs::create_dir(&tmp).unwrap();
        std::fs::create_dir(&packs).unwrap();
        (root, tmp, packs)
    }

    fn pack_of(tmp: &Path, packs: &Path, items: &[Vec<u8>]) -> std::path::PathBuf {
        let mut w = PackWriter::create(tmp).unwrap();
        for c in items {
            w.add_content(c).unwrap();
        }
        w.finish(packs).unwrap().unwrap()
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(32))]
        #[test]
        fn random_stores_and_fetches(
            contents in prop::collection::vec(prop::collection::vec(any::<u8>(), 0..2048), 1..30),
            n_packs in 1usize..=3,
        ) {
            let (_root, tmp, packs) = dirs();
            let mut store = Store::open(&packs).unwrap();
            for chunk in contents.chunks(contents.len().div_ceil(n_packs)) {
                let path = pack_of(&tmp, &packs, chunk);
                store.register(&path);
            }
            let fresh = Store::open(&packs).unwrap();
            for c in &contents {
                let id = hash_content(c);
                prop_assert!(fresh.has_good_copy(&id, ObjectKind::Content, c.len() as u64).unwrap());
                let mut out = Vec::new();
                fresh.copy_content(&id, &mut out).unwrap();
                prop_assert_eq!(&out, c);
            }
        }
    }

    #[test]
    fn a_note_is_kept_once_however_often_it_is_given() {
        let (_root, _tmp, packs) = dirs();
        let store = Store::open(&packs).unwrap();
        let finished = Warning::new(warning::SWITCH_FINISHED, "Finished a switch.");
        let unflushed = Warning::new(warning::HEAD_NOT_FLUSHED, "Couldn't flush.");
        // A save the command line tries again meets what the first try met.
        store.note(finished.clone());
        store.note(unflushed.clone());
        store.note(finished.clone());
        assert_eq!(store.warnings(), [finished, unflushed]);
    }

    #[test]
    fn overlapping_packs_are_fine() {
        let (_root, tmp, packs) = dirs();
        let a = b"shared".to_vec();
        pack_of(&tmp, &packs, &[a.clone(), b"one".to_vec()]);
        pack_of(&tmp, &packs, &[a.clone(), b"two".to_vec()]);
        let store = Store::open(&packs).unwrap();
        assert_eq!(store.pack_count(), 2);
        let mut out = Vec::new();
        store.copy_content(&hash_content(&a), &mut out).unwrap();
        assert_eq!(out, a);
    }

    #[test]
    fn refresh_registers_only_new_packs() {
        let (_root, tmp, packs) = dirs();
        pack_of(&tmp, &packs, &[b"one".to_vec()]);
        let mut store = Store::open(&packs).unwrap();
        pack_of(&tmp, &packs, &[b"two".to_vec()]);
        store.refresh().unwrap();
        store.refresh().unwrap();
        assert_eq!(store.pack_count(), 2);
        assert!(
            store
                .has_good_copy(&hash_content(b"two"), ObjectKind::Content, 3)
                .unwrap()
        );
    }

    #[test]
    fn a_read_falls_back_to_a_good_copy_in_another_pack() {
        let (_root, tmp, packs) = dirs();
        let content = b"the same bytes in two packs".to_vec();
        let listing = Listing::empty().encode();
        plant_damaged_copies(
            &packs,
            &[
                (ObjectKind::Content, &content),
                (ObjectKind::FolderListing, &listing),
            ],
        );
        let mut w = PackWriter::create(&tmp).unwrap();
        w.add_content(&content).unwrap();
        let listing_id = w.add_record(ObjectKind::FolderListing, &listing).unwrap();
        w.finish(&packs).unwrap();

        // The damaged pack sorts first, so its copies are tried first.
        let store = Store::open(&packs).unwrap();
        let mut out = Vec::new();
        store
            .copy_content(&hash_content(&content), &mut out)
            .unwrap();
        assert_eq!(out, content);
        assert_eq!(store.listing(&listing_id).unwrap(), Listing::empty());
        let damaged: Vec<String> = store
            .damaged_packs()
            .iter()
            .map(ToString::to_string)
            .collect();
        assert_eq!(damaged.len(), 1, "{damaged:?}");
        assert!(damaged[0].contains("00.pack is damaged"), "{damaged:?}");
        // The same pack, as a warning.
        let warnings = store.warnings();
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert_eq!(warnings[0].kind, "pack_damaged");
        assert_eq!(warnings[0].message, format!("{}.", damaged[0]));
    }

    #[test]
    fn only_a_copy_that_reads_back_counts_as_good() {
        let (_root, tmp, packs) = dirs();
        let content = b"bytes anyone could predict".to_vec();
        let (id, size) = (hash_content(&content), content.len() as u64);
        plant_damaged_copies(&packs, &[(ObjectKind::Content, &content)]);
        let mut store = Store::open(&packs).unwrap();
        assert!(!store.has_good_copy(&id, ObjectKind::Content, size).unwrap());
        assert_eq!(store.damaged_packs().len(), 1);
        // A good copy arriving later is found past the damaged one.
        pack_of(&tmp, &packs, &[content]);
        store.refresh().unwrap();
        assert!(store.has_good_copy(&id, ObjectKind::Content, size).unwrap());
        assert!(
            !store
                .has_good_copy(&id, ObjectKind::FolderListing, size)
                .unwrap()
        );
    }

    #[test]
    fn a_copy_recording_another_size_than_the_object_is_skipped_unread() {
        let (_root, tmp, packs) = dirs();
        let content = b"twelve bytes".to_vec();
        pack_of(&tmp, &packs, std::slice::from_ref(&content));
        let store = Store::open(&packs).unwrap();
        let id = hash_content(&content);
        // The copy itself is fine, so only skipping it unread explains what follows.
        assert!(!store.has_good_copy(&id, ObjectKind::Content, 13).unwrap());
        let damaged = store.damaged_packs()[0].to_string();
        assert!(damaged.contains("records the wrong size"), "{damaged}");
        assert!(store.copy_content(&id, &mut Vec::new()).is_err());
    }

    #[test]
    fn a_copy_that_runs_out_of_memory_is_passed_over_for_the_next() {
        let (_root, tmp, packs) = dirs();
        let content = b"held in two packs".to_vec();
        let id = hash_content(&content);
        pack_of(&tmp, &packs, std::slice::from_ref(&content));
        pack_of(&tmp, &packs, &[content.clone(), b"other".to_vec()]);
        let store = Store::open(&packs).unwrap();
        let mut tried = Vec::new();
        let read = store.first_good(&id, ObjectKind::Content, false, |pack, entry| {
            tried.push(pack.path().to_owned());
            if tried.len() == 1 {
                return Err(PackError::OutOfMemory);
            }
            pack.copy_content(entry, &mut io::sink())
                .map(|()| pack.path().to_owned())
        });
        assert_eq!(tried.len(), 2);
        assert_eq!(read.unwrap(), tried[1]);
        assert!(store.damaged_packs().is_empty());
        // Not damage, but the user should hear that a copy couldn't be read.
        let warnings = store.warnings();
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert_eq!(warnings[0].kind, "copy_unreadable");
        assert!(
            warnings[0]
                .message
                .contains(&tried[0].display().to_string()),
            "{warnings:?}"
        );
        assert!(
            warnings[0].message.contains("ran out of memory"),
            "{warnings:?}"
        );
    }

    #[test]
    fn a_copy_that_cant_be_read_is_reported_only_when_another_copy_worked() {
        let (_root, tmp, packs) = dirs();
        let content = b"held in two packs".to_vec();
        let id = hash_content(&content);
        pack_of(&tmp, &packs, std::slice::from_ref(&content));
        pack_of(&tmp, &packs, &[content.clone(), b"other".to_vec()]);
        let store = Store::open(&packs).unwrap();
        let no_disk = || -> Result<(), PackError> {
            Err(io::Error::other("the disk\u{1b}[2J said no.").into())
        };
        // Every copy fails: that is the read's error, and nothing else is said.
        let read = store.first_good(&id, ObjectKind::Content, false, |_, _| no_disk());
        assert!(
            matches!(read, Err(StoreError::Pack(PackError::Io(_)))),
            "{read:?}"
        );
        assert!(store.warnings().is_empty());

        // The first copy fails and the second works. The failure's text is escaped, and the full
        // stop it ends with isn't doubled.
        let mut failed = None;
        store
            .first_good(&id, ObjectKind::Content, false, |pack, entry| {
                if failed.is_none() {
                    failed = Some(pack.path().to_owned());
                    return no_disk();
                }
                pack.copy_content(entry, &mut io::sink())
            })
            .unwrap();
        let failed = failed.unwrap();
        let warnings = store.warnings();
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert_eq!(warnings[0].kind, "copy_unreadable");
        assert_eq!(
            warnings[0].message,
            format!(
                "Opalro couldn't read a copy in pack {}: the disk\\x1b[2J said no.",
                failed.display()
            )
        );
    }

    #[test]
    fn with_no_good_copy_a_failure_that_isnt_damage_is_named_first() {
        let (_root, tmp, packs) = dirs();
        let content = b"held twice, and the 00 copy is bad".to_vec();
        let id = hash_content(&content);
        plant_damaged_copies(&packs, &[(ObjectKind::Content, &content)]);
        pack_of(&tmp, &packs, std::slice::from_ref(&content));
        let store = Store::open(&packs).unwrap();
        let out_of_memory = |pack: Arc<PackReader>, entry: &IndexEntry| {
            if pack.path().ends_with("00.pack") {
                pack.copy_content(entry, &mut io::sink())
            } else {
                Err(PackError::OutOfMemory)
            }
        };
        let failed = store.first_good(&id, ObjectKind::Content, false, out_of_memory);
        assert!(
            matches!(failed, Err(StoreError::Pack(PackError::OutOfMemory))),
            "{failed:?}"
        );
        assert_eq!(store.damaged_packs().len(), 1);
        // The copy that ran out of memory isn't marked bad: with memory, it reads fine.
        let mut out = Vec::new();
        store.copy_content(&id, &mut out).unwrap();
        assert_eq!(out, content);
    }

    #[test]
    fn an_object_with_no_good_copy_is_damage_on_every_read() {
        let (_root, _tmp, packs) = dirs();
        let content = b"bytes with no good copy anywhere".to_vec();
        let listing = Listing::empty().encode();
        let objects = [
            (ObjectKind::Content, &content[..]),
            (ObjectKind::FolderListing, &listing[..]),
        ];
        // A bad copy in each of two packs; `00.pack` is read first, so its copy is the one named.
        plant_damaged_copies(&packs, &objects);
        std::fs::rename(packs.join("00.pack"), packs.join("01.pack")).unwrap();
        plant_damaged_copies(&packs, &objects);
        let store = Store::open(&packs).unwrap();
        let content_id = hash_content(&content);
        assert!(
            !store
                .has_good_copy(&content_id, ObjectKind::Content, content.len() as u64)
                .unwrap()
        );
        for _ in 0..2 {
            let failed = [
                store
                    .copy_content(&content_id, &mut Vec::new())
                    .unwrap_err(),
                store
                    .read_record(&Listing::empty().id(), ObjectKind::FolderListing)
                    .unwrap_err(),
            ];
            for e in failed {
                let StoreError::Pack(PackError::Corrupt { path, .. }) = &e else {
                    panic!("{e:?}");
                };
                assert!(path.ends_with("00.pack"), "{e:?}");
                assert!(matches!(RepoError::from(e), RepoError::Damaged { .. }));
            }
        }
    }

    /// Streaming is what finds the only copy bad here, and it is the streaming that must say so
    /// the second time, without reading the copy again.
    #[test]
    fn a_lone_bad_copy_is_damage_each_time_it_is_streamed() {
        let (_root, _tmp, packs) = dirs();
        let content = b"bytes with one copy, and a bad one".to_vec();
        plant_damaged_copies(&packs, &[(ObjectKind::Content, &content)]);
        let store = Store::open(&packs).unwrap();
        let id = hash_content(&content);
        let damage = |e: StoreError| {
            let StoreError::Pack(PackError::Corrupt { path, .. }) = &e else {
                panic!("{e:?}");
            };
            assert!(path.ends_with("00.pack"), "{e:?}");
            assert!(matches!(RepoError::from(e), RepoError::Damaged { .. }));
        };
        // What the first stream wrote is untrusted, as always.
        damage(store.copy_content(&id, &mut Vec::new()).unwrap_err());
        let mut again = Vec::new();
        damage(store.copy_content(&id, &mut again).unwrap_err());
        assert!(again.is_empty(), "a copy known to be bad was read again");
        assert_eq!(store.damaged_packs().len(), 1);
    }

    #[test]
    fn at_most_16_packs_are_open_at_once() {
        let (_root, tmp, packs) = dirs();
        let contents: Vec<Vec<u8>> = (0..40u8).map(|i| vec![i; 3]).collect();
        for c in &contents {
            pack_of(&tmp, &packs, std::slice::from_ref(c));
        }
        let store = Store::open(&packs).unwrap();
        assert_eq!(store.pack_count(), 40);
        assert_eq!(store.open_count(), 0);
        for c in contents.iter().rev() {
            let mut out = Vec::new();
            store.copy_content(&hash_content(c), &mut out).unwrap();
            assert_eq!(&out, c);
            assert!(store.open_count() <= 16, "{} open", store.open_count());
        }
        assert_eq!(store.open_count(), 16);
    }

    /// Lookups on several threads at once keep at most 16 packs open too: none is closed while a
    /// lookup still reads it, those being opened count, and none is open twice.
    #[test]
    fn lookups_on_several_threads_keep_at_most_16_packs_open() {
        let (_root, tmp, packs) = dirs();
        // Long enough that reading one holds its pack a while.
        let contents: Vec<Vec<u8>> = (0..40u8).map(|i| vec![i; 1 << 18]).collect();
        for c in &contents {
            pack_of(&tmp, &packs, std::slice::from_ref(c));
        }
        let store = Store::open(&packs).unwrap();
        std::thread::scope(|scope| {
            for t in 0..8u8 {
                let (store, contents) = (&store, &contents);
                scope.spawn(move || {
                    for round in 0..3u8 {
                        let from = usize::from(t) * 5;
                        for c in contents.iter().cycle().skip(from).take(40) {
                            let mut out = Vec::new();
                            store.copy_content(&hash_content(c), &mut out).unwrap();
                            assert!(out == *c);
                        }
                        // A miss looks in every pack.
                        let missing = hash_content(&[t, round]);
                        assert!(
                            !store
                                .has_good_copy(&missing, ObjectKind::Content, 2)
                                .unwrap()
                        );
                    }
                });
            }
        });
        let most = store.counted.most();
        assert!(most <= 16, "{most} packs open at once");
        let packs = store.packs();
        let mut open: Vec<usize> = packs.open.iter().map(|(n, _)| *n).collect();
        open.sort_unstable();
        open.dedup();
        assert_eq!(open.len(), packs.open.len(), "a pack is open twice");
    }

    /// Lookups on several threads meet copies in another order: of two failures alike, the one
    /// told is the same whatever the order.
    #[test]
    fn the_error_told_among_equals_is_the_same_whatever_their_order() {
        let errors = || {
            let corrupt = |p: &str| PackError::Corrupt {
                path: p.into(),
                reason: "bad".into(),
            };
            let unread =
                |p: &str| PackError::Io(io::Error::other(format!("couldn't read pack {p}")));
            vec![
                corrupt("b.pack"),
                unread("b.pack"),
                corrupt("a.pack"),
                unread("a.pack"),
            ]
        };
        let mut backwards = errors();
        backwards.reverse();
        let (told, told_backwards) = (most(errors()).unwrap(), most(backwards).unwrap());
        assert_eq!(told.to_string(), told_backwards.to_string());
        assert!(told.to_string().contains("a.pack"), "{told}");
    }

    /// A pack Opalro may not read is passed over, named in one warning however many lookups pass
    /// it, and named in the error of a lookup that finds nothing.
    #[test]
    fn a_pack_that_cannot_be_opened_is_named() {
        let (_root, tmp, packs) = dirs();
        let content = b"found past the locked pack".to_vec();
        let id = hash_content(&content);
        // Searched first on a cold store.
        let locked = packs.join("00.pack");
        std::fs::rename(pack_of(&tmp, &packs, &[b"locked".to_vec()]), &locked).unwrap();
        pack_of(&tmp, &packs, std::slice::from_ref(&content));
        let Some(_held) = lock_pack(&locked) else {
            return;
        };
        let store = Store::open(&packs).unwrap();
        for _ in 0..2 {
            let mut out = Vec::new();
            store.copy_content(&id, &mut out).unwrap();
            assert_eq!(out, content);
        }
        let missing = store
            .copy_content(&hash_content(b"nowhere"), &mut Vec::new())
            .unwrap_err();
        assert!(missing.to_string().contains("00.pack"), "{missing}");
        assert!(!matches!(
            RepoError::from(missing),
            RepoError::Damaged { .. }
        ));
        // Listing every ID, as for a lost head, passes over it too.
        let ids = store.ids_of(ObjectKind::Content).unwrap();
        assert_eq!(ids, HashSet::from([id]));
        let warnings = store.warnings();
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert_eq!(warnings[0].kind, "pack_unreadable");
        assert!(warnings[0].message.contains("00.pack"), "{warnings:?}");
    }

    /// Opening a junk pack again would close a good open one each time; every lookup that reaches
    /// it still gets its damage.
    #[test]
    fn a_junk_pack_is_opened_once_per_command() {
        let (_root, tmp, packs) = dirs();
        for i in 0..16u8 {
            pack_of(&tmp, &packs, &[vec![i; 3]]);
        }
        std::fs::write(packs.join("zz.pack"), vec![b'x'; 200]).unwrap();
        let store = Store::open(&packs).unwrap();
        let miss = |i: u8| {
            let e = store
                .copy_content(&hash_content(&[i; 5]), &mut Vec::new())
                .unwrap_err();
            let junk = matches!(&e, StoreError::Pack(PackError::Corrupt { path, .. })
                if path.ends_with("zz.pack"));
            assert!(junk, "{e:?}");
        };
        // The first try at the junk pack closed a good one, which the next miss opens again.
        miss(0);
        miss(1);
        let opened = open_files::total();
        for i in 2..6 {
            miss(i);
        }
        assert_eq!(open_files::total(), opened, "packs were opened again");
        assert_eq!(store.damaged_packs().len(), 1);
        // Listing every ID, as for a lost head, stops at damage.
        let listed = store.ids_of(ObjectKind::Content);
        let damage = matches!(&listed, Err(StoreError::Pack(PackError::Corrupt { .. })));
        assert!(damage, "{:?}", listed.map(|ids| ids.len()));
    }

    /// A pack that failed to open fails unopened until the packs are listed again: a good pack
    /// may have been put under its name since.
    #[test]
    fn a_listing_forgets_which_packs_failed_to_open() {
        let (_root, tmp, packs) = dirs();
        let content = b"under the junk's name".to_vec();
        let id = hash_content(&content);
        let path = pack_of(&tmp, &packs, std::slice::from_ref(&content));
        let good = std::fs::read(&path).unwrap();
        std::fs::write(&path, vec![b'x'; 200]).unwrap();
        let mut store = Store::open(&packs).unwrap();
        assert!(store.copy_content(&id, &mut Vec::new()).is_err());
        std::fs::write(&path, good).unwrap();
        assert!(store.copy_content(&id, &mut Vec::new()).is_err());
        store.refresh().unwrap();
        let mut out = Vec::new();
        store.copy_content(&id, &mut out).unwrap();
        assert_eq!(out, content);
    }

    /// An index read to the end fails for a reason other than damage, here a pack cut short
    /// while open: a listing of every ID, as for a lost head, passes over it with its warning.
    #[test]
    fn a_listing_of_every_id_passes_over_an_index_it_cannot_read() {
        let (_root, tmp, packs) = dirs();
        pack_of(&tmp, &packs, &[b"one".to_vec()]);
        let cut = pack_of(&tmp, &packs, &[b"two".to_vec()]);
        let store = Store::open(&packs).unwrap();
        // Opened now, and kept open.
        store
            .copy_content(&hash_content(b"two"), &mut Vec::new())
            .unwrap();
        let len = std::fs::metadata(&cut).unwrap().len();
        let file = std::fs::OpenOptions::new().write(true).open(&cut).unwrap();
        // Into its one ID: what is left of the index ends in the middle of it.
        file.set_len(len - 100).unwrap();
        drop(file);
        let ids = store.ids_of(ObjectKind::Content).unwrap();
        assert_eq!(ids, HashSet::from([hash_content(b"one")]));
        let warnings = store.warnings();
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert_eq!(warnings[0].kind, "pack_unreadable");
        let name = cut.file_name().unwrap().to_str().unwrap();
        assert!(warnings[0].message.contains(name), "{warnings:?}");
    }

    /// Whatever the failure, the error kept names the pack, and the warning's advice fits it.
    #[test]
    fn a_pack_passed_over_is_named_whatever_the_failure() {
        let (_root, tmp, packs) = dirs();
        let path = pack_of(&tmp, &packs, &[b"one".to_vec()]);
        let name = path.file_name().unwrap().to_str().unwrap();
        let store = Store::open(&packs).unwrap();
        let mut skipped = Vec::new();
        store.pass_over(0, PackError::OutOfMemory, &mut skipped);
        let kept = skipped[0].to_string();
        assert!(
            kept.contains(name) && kept.contains("ran out of memory"),
            "{kept}"
        );
        let said = &store.warnings()[0].message;
        assert!(said.contains(name), "{said}");
        assert!(
            said.ends_with("may free enough memory to read it."),
            "{said}"
        );
    }

    #[test]
    fn a_pack_gone_when_opened_sends_the_store_to_list_again() {
        let (_root, tmp, packs) = dirs();
        let gone = pack_of(&tmp, &packs, &[b"moved".to_vec()]);
        let store = Store::open(&packs).unwrap();
        // Merged into another pack after the store listed them.
        std::fs::remove_file(gone).unwrap();
        pack_of(&tmp, &packs, &[b"moved".to_vec(), b"other".to_vec()]);
        let mut out = Vec::new();
        store
            .copy_content(&hash_content(b"moved"), &mut out)
            .unwrap();
        assert_eq!(out, b"moved");
        assert_eq!(store.pack_count(), 1);
    }

    /// Writes a version 1 pack `name` holding a copy of `id` whose bytes are `data`, stored as
    /// they are, so the copy claims `data`'s size.
    fn plant_lying_copy(packs: &Path, name: &str, id: &Id, data: &[u8]) {
        plant_lying(packs, name, ObjectKind::Content, id, data);
    }

    /// `plant_lying_copy`, of any kind.
    fn plant_lying(packs: &Path, name: &str, kind: ObjectKind, id: &Id, data: &[u8]) {
        use crate::encoding::Writer;
        use crate::pack::{PACK_END_MAGIC, PACK_MAGIC};
        let len = data.len() as u64;
        let mut index = Writer::new();
        index.varint(1);
        index.id(id);
        index.u8(kind as u8);
        index.u64(16);
        index.varint(len);
        index.varint(len);
        let index = index.into_bytes();
        let mut out = Writer::new();
        out.u8(kind as u8);
        out.id(id);
        out.u64(len);
        out.u64(len);
        out.u8(0);
        let mut pack = PACK_MAGIC.to_vec();
        pack.extend_from_slice(&1u32.to_le_bytes());
        pack.extend_from_slice(&0u32.to_le_bytes());
        pack.extend_from_slice(&out.into_bytes());
        pack.extend_from_slice(data);
        let at = pack.len() as u64;
        pack.extend_from_slice(&index);
        pack.extend_from_slice(&at.to_le_bytes());
        pack.extend_from_slice(blake3::hash(&index).as_bytes());
        pack.extend_from_slice(&PACK_END_MAGIC);
        std::fs::write(packs.join(name), pack).unwrap();
    }

    /// Keeps what is written, and the largest single write.
    #[derive(Default)]
    struct Writes {
        data: Vec<u8>,
        largest: usize,
    }

    impl Write for Writes {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.largest = self.largest.max(buf.len());
            self.data.extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn a_small_bad_copy_found_first_doesnt_make_a_big_good_one_be_held_whole() {
        let (_root, tmp, packs) = dirs();
        let big = vec![7u8; 2 << 20];
        let id = hash_content(&big);
        // `00.pack` sorts first, and its copy claims four bytes.
        plant_lying_copy(&packs, "00.pack", &id, b"tiny");
        pack_of(&tmp, &packs, std::slice::from_ref(&big));
        let store = Store::open(&packs).unwrap();
        let mut out = Writes::default();
        store.copy_content(&id, &mut out).unwrap();
        assert!(out.data == big);
        // Streamed a buffer at a time, never handed over whole.
        assert!(
            out.largest <= 64 << 10,
            "{} bytes in one write",
            out.largest
        );
        assert_eq!(store.damaged_packs().len(), 1);
    }

    #[test]
    fn a_first_copy_of_another_kind_doesnt_hide_the_good_copy() {
        let (_root, tmp, packs) = dirs();
        let good = b"the bytes the user saved".to_vec();
        let id = hash_content(&good);
        // Found first, of the wrong kind; then a small copy that lies; then the good one.
        plant_lying(&packs, "00.pack", ObjectKind::Save, &id, b"a save");
        plant_lying_copy(&packs, "01.pack", &id, b"tiny");
        pack_of(&tmp, &packs, std::slice::from_ref(&good));
        let store = Store::open(&packs).unwrap();
        let mut out = Vec::new();
        store.copy_content(&id, &mut out).unwrap();
        assert_eq!(out, good);
    }

    #[test]
    fn copies_in_many_packs_keep_at_most_16_open() {
        let (_root, tmp, packs) = dirs();
        let big = vec![9u8; 2 << 20];
        for i in 0..40u8 {
            pack_of(&tmp, &packs, &[big.clone(), vec![i; 3]]);
        }
        let store = Store::open(&packs).unwrap();
        open_files::most_since_last();
        let mut out = Vec::new();
        store.copy_content(&hash_content(&big), &mut out).unwrap();
        assert!(out == big);
        let most = open_files::most_since_last();
        assert!(most <= 16, "{most} packs open at once");
    }

    /// A list's pack is let go of once the list is read, so finding its chunks in many packs keeps
    /// no more open than any read does.
    #[test]
    fn a_chunked_read_keeps_at_most_16_packs_open() {
        let (_root, tmp, packs) = dirs();
        let bytes: Vec<u8> = (0..20u8).flat_map(|i| [i; 3]).collect();
        let (list, chunks) = chunked(&bytes, 3);
        // The first chunk in the pack searched last, so finding it opens all 20.
        for (i, c) in chunks.iter().enumerate() {
            let name = format!("c{:02}.pack", 19 - i);
            pack_with(&tmp, &packs, Some(&name), &[c], &[]);
        }
        pack_with(&tmp, &packs, Some("00.pack"), &[], &[&list]);
        let store = Store::open(&packs).unwrap();
        open_files::most_since_last();
        let mut out = Vec::new();
        store.copy_content(&list.file, &mut out).unwrap();
        assert_eq!(out, bytes);
        let most = open_files::most_since_last();
        assert!(most <= 16, "{most} packs open at once");
    }

    #[test]
    fn a_miss_opens_only_the_packs_that_dont_stay_open() {
        let (_root, tmp, packs) = dirs();
        for i in 0..20u8 {
            pack_of(&tmp, &packs, &[vec![i; 3]]);
        }
        let store = Store::open(&packs).unwrap();
        let miss = |i: u8| {
            let id = hash_content(&[i; 5]);
            assert!(!store.has_good_copy(&id, ObjectKind::Content, 5).unwrap());
        };
        miss(0);
        for i in 1..10 {
            let before = open_files::total();
            miss(i);
            let opened = open_files::total() - before;
            assert!(opened <= 4, "a miss opened {opened} packs");
        }
    }

    /// `bytes` cut every `every` bytes: its chunk list, and the chunks' bytes.
    fn chunked(bytes: &[u8], every: usize) -> (ChunkList, Vec<Vec<u8>>) {
        let pieces: Vec<Vec<u8>> = bytes.chunks(every).map(<[u8]>::to_vec).collect();
        let list = ChunkList {
            file: hash_content(bytes),
            size: bytes.len() as u64,
            chunks: pieces
                .iter()
                .map(|p| Chunk {
                    id: hash_content(p),
                    size: p.len() as u64,
                })
                .collect(),
        };
        (list, pieces)
    }

    /// A pack of `contents` and `lists`, named `name` if given, else as Opalro names it.
    fn pack_with(
        tmp: &Path,
        packs: &Path,
        name: Option<&str>,
        contents: &[&[u8]],
        lists: &[&ChunkList],
    ) -> PathBuf {
        let mut w = PackWriter::create(tmp).unwrap();
        for c in contents {
            w.add_content(c).unwrap();
        }
        for l in lists {
            w.add_chunk_list(l).unwrap();
        }
        let path = w.finish(packs).unwrap().unwrap();
        let Some(name) = name else { return path };
        std::fs::rename(&path, packs.join(name)).unwrap();
        packs.join(name)
    }

    /// 3 MiB that don't compress: past `NEAR`, so streamed.
    fn big() -> Vec<u8> {
        let mut big = vec![0u8; 3 << 20];
        blake3::Hasher::new()
            .update(b"big")
            .finalize_xof()
            .fill(&mut big);
        big
    }

    /// "hello world", read whole before it goes out, and 3 MiB, streamed, cut into 256 KiB chunks.
    fn files() -> [(Vec<u8>, usize); 2] {
        [(b"hello world".to_vec(), 6), (big(), 256 << 10)]
    }

    fn pieces(pieces: &[Vec<u8>]) -> Vec<&[u8]> {
        pieces.iter().map(Vec::as_slice).collect()
    }

    /// Fails unless `e` is damage found in the pack at `path`.
    fn damage_in(e: StoreError, path: &Path) {
        match &e {
            StoreError::Pack(PackError::Corrupt { path: found, .. }) => {
                assert_eq!(found, path, "{e}");
            }
            other => panic!("{other:?}"),
        }
        assert!(matches!(RepoError::from(e), RepoError::Damaged { .. }));
    }

    #[test]
    fn a_chunked_file_reads_back_from_one_pack_or_several() {
        for (bytes, every) in files() {
            let (list, chunks) = chunked(&bytes, every);
            for spread in [false, true] {
                let (_root, tmp, packs) = dirs();
                if spread {
                    for c in &chunks {
                        pack_with(&tmp, &packs, None, &[c], &[]);
                    }
                    pack_with(&tmp, &packs, None, &[], &[&list]);
                } else {
                    pack_with(&tmp, &packs, None, &pieces(&chunks), &[&list]);
                }
                let store = Store::open(&packs).unwrap();
                for _ in 0..2 {
                    let mut out = Writes::default();
                    store.copy_content(&list.file, &mut out).unwrap();
                    assert!(out.data == bytes, "{} bytes", bytes.len());
                    // A chunk at a time, never the whole file.
                    assert!(out.largest <= 256 << 10, "{} bytes at once", out.largest);
                }
                assert!(store.warnings().is_empty(), "{:?}", store.warnings());
            }
        }
    }

    #[test]
    fn a_chunk_with_a_bad_copy_is_read_from_its_good_one() {
        for (bytes, every) in files() {
            let (_root, tmp, packs) = dirs();
            let (list, chunks) = chunked(&bytes, every);
            plant_damaged_copies(&packs, &[(ObjectKind::Content, &chunks[0])]);
            pack_with(&tmp, &packs, None, &pieces(&chunks), &[&list]);
            let store = Store::open(&packs).unwrap();
            let mut out = Vec::new();
            store.copy_content(&list.file, &mut out).unwrap();
            assert!(out == bytes);
            let damaged: Vec<String> = store
                .damaged_packs()
                .iter()
                .map(ToString::to_string)
                .collect();
            assert_eq!(damaged.len(), 1, "{damaged:?}");
            assert!(damaged[0].contains("00.pack is damaged"), "{damaged:?}");

            // With no good copy of the chunk, the read fails at the chunk's pack, each time, and
            // the list's pack isn't called damaged.
            let (_root, tmp, packs) = dirs();
            plant_damaged_copies(&packs, &[(ObjectKind::Content, &chunks[0])]);
            pack_with(&tmp, &packs, None, &pieces(&chunks[1..]), &[&list]);
            let store = Store::open(&packs).unwrap();
            for _ in 0..2 {
                let e = store.copy_content(&list.file, &mut Vec::new()).unwrap_err();
                damage_in(e, &packs.join("00.pack"));
            }
            let damaged = store.damaged_packs();
            assert_eq!(damaged.len(), 1, "{damaged:?}");
            assert!(damaged[0].to_string().contains("00.pack"), "{damaged:?}");
        }
    }

    /// A list naming one 100-byte chunk `times` times.
    fn repeated(times: usize) -> (Vec<u8>, ChunkList, Vec<Vec<u8>>) {
        let bytes: Vec<u8> = (0..100u8).cycle().take(100 * times).collect();
        let (list, chunks) = chunked(&bytes, 100);
        (bytes, list, chunks)
    }

    #[test]
    fn a_chunk_named_twice_is_read_once() {
        // The reads a list naming its chunk `times` times takes, the list's own included.
        let reads = |times| {
            let (_root, tmp, packs) = dirs();
            let (bytes, list, chunks) = repeated(times);
            pack_with(&tmp, &packs, None, &[&chunks[0]], &[&list]);
            let store = Store::open(&packs).unwrap();
            let before = open_files::reads();
            let mut out = Vec::new();
            store.copy_content(&list.file, &mut out).unwrap();
            assert!(out == bytes);
            open_files::reads() - before
        };
        // Without the kept chunks, each naming took 3 reads.
        let (once, often) = (reads(1), reads(1001));
        assert!(
            often < once + 20,
            "{often} reads for 1,001 namings, {once} for one"
        );
    }

    #[test]
    fn a_repeated_chunk_with_no_good_copy_is_damage() {
        let (_root, tmp, packs) = dirs();
        let (_, list, chunks) = repeated(1000);
        plant_damaged_copies(&packs, &[(ObjectKind::Content, &chunks[0])]);
        pack_with(&tmp, &packs, None, &[], &[&list]);
        let store = Store::open(&packs).unwrap();
        for _ in 0..2 {
            let e = store.copy_content(&list.file, &mut Vec::new()).unwrap_err();
            damage_in(e, &packs.join("00.pack"));
        }
    }

    #[test]
    fn the_kept_chunks_stay_within_their_budget() {
        let (_root, tmp, packs) = dirs();
        let mut bytes = vec![0u8; 200 << 18];
        blake3::Hasher::new()
            .update(b"budget")
            .finalize_xof()
            .fill(&mut bytes);
        let (list, chunks) = chunked(&bytes, 256 << 10);
        pack_with(&tmp, &packs, None, &pieces(&chunks), &[&list]);
        let store = Store::open(&packs).unwrap();
        KEPT_MOST.set((0, 0));
        let mut out = Vec::new();
        store.copy_content(&list.file, &mut out).unwrap();
        assert!(out == bytes);
        let (most, _) = KEPT_MOST.get();
        assert!(most <= 8 << 20 && most > 7 << 20, "{most} bytes charged");
    }

    #[test]
    fn many_tiny_chunks_stay_within_the_budget() {
        let (_root, tmp, packs) = dirs();
        // 100,000 different chunks of 1 to 3 bytes.
        let chunks: Vec<Vec<u8>> = (0u32..100_000)
            .map(|i| {
                let len = if i < 1 << 8 {
                    1
                } else if i < 1 << 16 {
                    2
                } else {
                    3
                };
                i.to_le_bytes()[..len].to_vec()
            })
            .collect();
        let bytes = chunks.concat();
        let list = ChunkList {
            file: hash_content(&bytes),
            size: bytes.len() as u64,
            chunks: chunks
                .iter()
                .map(|c| Chunk {
                    id: hash_content(c),
                    size: c.len() as u64,
                })
                .collect(),
        };
        pack_with(&tmp, &packs, None, &pieces(&chunks), &[&list]);
        let store = Store::open(&packs).unwrap();
        KEPT_MOST.set((0, 0));
        let mut out = Vec::new();
        store.copy_content(&list.file, &mut out).unwrap();
        assert!(out == bytes);
        let (most, count) = KEPT_MOST.get();
        assert!(most <= 8 << 20, "{most} bytes charged");
        assert!(count <= (8 << 20) / 129, "{count} chunks kept");
    }

    #[test]
    fn a_kept_chunk_named_with_another_size_is_damage() {
        let (_root, tmp, packs) = dirs();
        let chunk = b"a chunk".to_vec();
        let id = hash_content(&chunk);
        let list = ChunkList {
            file: hash_content(b"a file"),
            size: 15,
            chunks: vec![Chunk { id, size: 7 }, Chunk { id, size: 8 }],
        };
        let at = pack_with(&tmp, &packs, None, &[&chunk], &[&list]);
        let store = Store::open(&packs).unwrap();
        let e = store.copy_content(&list.file, &mut Vec::new()).unwrap_err();
        let shown = e.to_string();
        assert!(
            shown.contains("8 bytes, which isn't in the store"),
            "{shown}"
        );
        damage_in(e, &at);
    }

    /// Writes into memory, and flips the first stored byte of `entry` in the pack at `pack` once
    /// `after` bytes have come.
    struct DamagingAfter {
        data: Vec<u8>,
        after: usize,
        pack: PathBuf,
        entry: IndexEntry,
    }

    impl Write for DamagingAfter {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            let was = self.data.len();
            self.data.extend_from_slice(buf);
            if was < self.after && self.data.len() >= self.after {
                use std::io::{Read, Seek, SeekFrom};
                let mut f = std::fs::OpenOptions::new()
                    .read(true)
                    .write(true)
                    .open(&self.pack)?;
                let at = SeekFrom::Start(self.entry.offset + crate::pack::ENTRY_HEADER_LEN);
                let mut byte = [0u8];
                f.seek(at)?;
                f.read_exact(&mut byte)?;
                f.seek(at)?;
                f.write_all(&[byte[0] ^ 1])?;
            }
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn a_chunk_dropped_from_the_kept_ones_is_read_and_checked_again() {
        let (_root, tmp, packs) = dirs();
        // 33 chunks of 256 KiB take more than the 8 MiB, so the first is let go by the last.
        let mut bytes = vec![0u8; 33 << 18];
        blake3::Hasher::new()
            .update(b"dropped")
            .finalize_xof()
            .fill(&mut bytes);
        let (_, chunks) = chunked(&bytes, 256 << 10);
        let ending = |last: usize| {
            let mut file = bytes.clone();
            file.extend_from_slice(&chunks[last]);
            let (list, _) = chunked(&file, 256 << 10);
            (file, list)
        };
        let (kept_file, kept_list) = ending(32);
        let (dropped_file, dropped_list) = ending(0);
        let pack = pack_with(
            &tmp,
            &packs,
            None,
            &pieces(&chunks),
            &[&kept_list, &dropped_list],
        );
        let reads = |list: &ChunkList, file: &[u8]| {
            let store = Store::open(&packs).unwrap();
            let before = open_files::reads();
            let mut out = Vec::new();
            store.copy_content(&list.file, &mut out).unwrap();
            assert!(out == file);
            open_files::reads() - before
        };
        // Lists of the same length: the dropped chunk is read again, the kept one isn't.
        let (kept, dropped) = (
            reads(&kept_list, &kept_file),
            reads(&dropped_list, &dropped_file),
        );
        assert!(
            dropped > kept,
            "{dropped} reads when dropped, {kept} when kept"
        );

        // Damaged once it was first written out, it is damage when read again.
        let entry = PackReader::open(&pack)
            .unwrap()
            .find(&hash_content(&chunks[0]))
            .unwrap()
            .unwrap();
        let store = Store::open(&packs).unwrap();
        let mut out = DamagingAfter {
            data: Vec::new(),
            after: 256 << 10,
            pack: pack.clone(),
            entry,
        };
        let e = store
            .copy_content(&dropped_list.file, &mut out)
            .unwrap_err();
        damage_in(e, &pack);
    }

    /// A list whose chunk has no good copy fails once and is passed over from then on, so the read
    /// goes on to a good whole copy, though the list is found first and, when small, read first.
    #[test]
    fn a_failed_chunk_list_never_hides_a_good_whole_copy() {
        for (bytes, every) in files() {
            let (_root, tmp, packs) = dirs();
            let (list, chunks) = chunked(&bytes, every);
            plant_damaged_copies(&packs, &[(ObjectKind::Content, &chunks[0])]);
            pack_with(
                &tmp,
                &packs,
                Some("01.pack"),
                &pieces(&chunks[1..]),
                &[&list],
            );
            pack_with(&tmp, &packs, Some("ff.pack"), &[&bytes], &[]);
            let store = Store::open(&packs).unwrap();
            for _ in 0..2 {
                let mut out = Vec::new();
                store.copy_content(&list.file, &mut out).unwrap();
                assert!(out == bytes, "{} bytes", bytes.len());
            }
            // Only the chunk's pack is ever called damaged, never the list's. A small file's
            // first copy, the list, is read first, so there the chunk is always met.
            let damaged: Vec<String> = store
                .damaged_packs()
                .iter()
                .map(ToString::to_string)
                .collect();
            assert!(damaged.iter().all(|d| d.contains("00.pack")), "{damaged:?}");
            if bytes.len() as u64 <= NEAR {
                assert_eq!(damaged.len(), 1, "{damaged:?}");
            }
        }
    }

    /// A list whose chunk is only in a pack Opalro may not open fails each time it is read, for a
    /// reason that isn't damage: the read goes on to the copy the check proved good, no pack is
    /// called damaged, and the list isn't marked bad, so it builds its file once the pack opens.
    #[test]
    fn a_list_that_cant_be_read_never_hides_a_good_whole_copy() {
        let (_root, tmp, packs) = dirs();
        let (list, chunks) = chunked(b"hello world", 6);
        // The list is found first, and is small, so it is read first.
        pack_with(&tmp, &packs, Some("00.pack"), &[&chunks[1]], &[&list]);
        let locked = pack_with(&tmp, &packs, Some("11.pack"), &[&chunks[0]], &[]);
        pack_with(&tmp, &packs, Some("ff.pack"), &[b"hello world"], &[]);
        let Some(_held) = lock_pack(&locked) else {
            return;
        };
        let store = Store::open(&packs).unwrap();
        for _ in 0..2 {
            let mut out = Vec::new();
            store.copy_content(&list.file, &mut out).unwrap();
            assert_eq!(out, b"hello world");
        }
        assert!(
            store.damaged_packs().is_empty(),
            "{:?}",
            store.damaged_packs()
        );
        assert!(store.checked().bad.is_empty());
        // The copy that couldn't be read, and the pack that couldn't be opened, which is why.
        let warnings = store.warnings();
        let kinds: Vec<&str> = warnings.iter().map(|w| w.kind).collect();
        let expected = [warning::COPY_UNREADABLE, warning::PACK_UNREADABLE];
        assert_eq!(kinds, expected, "{warnings:?}");
        assert!(warnings[0].message.contains("00.pack"), "{warnings:?}");
        assert!(warnings[1].message.contains("11.pack"), "{warnings:?}");
    }

    #[test]
    fn a_chunk_list_naming_a_missing_chunk_fails_the_read() {
        let (list, chunks) = chunked(b"hello world", 6);
        // A chunk held only with another size than the list gives counts as missing too.
        let mut shorter = chunked(b"hello worl", 6).0;
        shorter.chunks[1].id = list.chunks[1].id;
        for (list, held) in [(&list, &chunks[..1]), (&shorter, &chunks[..])] {
            let (_root, tmp, packs) = dirs();
            let path = pack_with(&tmp, &packs, None, &pieces(held), &[list]);
            let store = Store::open(&packs).unwrap();
            let e = store.copy_content(&list.file, &mut Vec::new()).unwrap_err();
            let named = e.to_string().contains(&list.chunks[1].id.to_hex());
            assert!(named, "{e}");
            damage_in(e, &path);
        }
    }

    #[test]
    fn a_chunk_list_of_good_chunks_that_build_another_file_fails_the_read() {
        for (bytes, every) in files() {
            let (_root, tmp, packs) = dirs();
            let (mut list, chunks) = chunked(&bytes, every);
            let mut other = bytes.clone();
            other[0] ^= 1;
            list.file = hash_content(&other);
            let path = pack_with(&tmp, &packs, None, &pieces(&chunks), &[&list]);
            let store = Store::open(&packs).unwrap();
            for _ in 0..2 {
                // What went out is untrusted, as always.
                let e = store.copy_content(&list.file, &mut Vec::new()).unwrap_err();
                assert!(e.to_string().contains("build another file"), "{e}");
                damage_in(e, &path);
            }
            assert_eq!(store.damaged_packs().len(), 1);
            // Its chunks are fine.
            let mut out = Vec::new();
            store.copy_content(&list.chunks[0].id, &mut out).unwrap();
            assert_eq!(out, chunks[0]);
        }
    }

    #[test]
    fn a_chunk_list_under_a_chunks_id_is_passed_over() {
        let (list, chunks) = chunked(b"hello world", 6);
        let (inner, inner_chunks) = chunked(&chunks[0], 3);
        let (_root, tmp, packs) = dirs();
        // "hello " is held only as a chunk list, in the pack searched first.
        let mut held = pieces(&inner_chunks);
        held.push(&chunks[1]);
        pack_with(&tmp, &packs, Some("00.pack"), &held, &[&inner]);
        let path = pack_with(&tmp, &packs, None, &[], &[&list]);
        let store = Store::open(&packs).unwrap();
        // As a file of its own it reads through its list...
        let mut out = Vec::new();
        store.copy_content(&inner.file, &mut out).unwrap();
        assert_eq!(out, chunks[0]);
        // ...but never as a chunk of another file, so a read goes no deeper than one list.
        let e = store.copy_content(&list.file, &mut Vec::new()).unwrap_err();
        damage_in(e, &path);
        // With the chunk stored whole too, the file reads, past the list found first.
        pack_with(&tmp, &packs, None, &[&chunks[0]], &[]);
        let store = Store::open(&packs).unwrap();
        let mut out = Vec::new();
        store.copy_content(&list.file, &mut out).unwrap();
        assert_eq!(out, b"hello world");
        assert!(store.warnings().is_empty(), "{:?}", store.warnings());
    }

    /// A pack of one chunk list entry: `stored` under `id`, recording `size`.
    fn plant_list_entry(tmp: &Path, packs: &Path, id: Id, size: u64, zstd: bool, stored: Vec<u8>) {
        let mut w = PackWriter::create(tmp).unwrap();
        let entry = Prepared {
            id,
            size,
            zstd,
            stored,
        };
        w.add_prepared(ObjectKind::ChunkList, &entry).unwrap();
        w.finish(packs).unwrap();
    }

    #[test]
    fn a_chunk_list_that_names_another_file_or_size_is_damage() {
        let (list, chunks) = chunked(b"hello world", 6);
        let another = hash_content(b"another file");
        for (id, size) in [(another, 11), (list.file, 12)] {
            let (_root, tmp, packs) = dirs();
            pack_with(&tmp, &packs, None, &pieces(&chunks), &[]);
            plant_list_entry(&tmp, &packs, id, size, false, list.encode());
            let store = Store::open(&packs).unwrap();
            let e = store.copy_content(&id, &mut Vec::new()).unwrap_err();
            assert!(e.to_string().contains("names another file"), "{e}");
            assert!(matches!(RepoError::from(e), RepoError::Damaged { .. }));
            assert_eq!(store.damaged_packs().len(), 1);
        }
    }

    #[test]
    fn a_compressed_chunk_list_is_damage() {
        let (_root, tmp, packs) = dirs();
        let (list, chunks) = chunked(b"hello world", 6);
        pack_with(&tmp, &packs, None, &pieces(&chunks), &[]);
        let compressed = zstd::bulk::compress(&list.encode(), 3).unwrap();
        plant_list_entry(&tmp, &packs, list.file, 11, true, compressed);
        let store = Store::open(&packs).unwrap();
        let e = store.copy_content(&list.file, &mut Vec::new()).unwrap_err();
        assert!(e.to_string().contains("compressed"), "{e}");
        assert!(matches!(RepoError::from(e), RepoError::Damaged { .. }));
        // Nor is it the list a writer looks for.
        assert!(!store.holds_chunk_list(&list).unwrap());
        assert_eq!(store.damaged_packs().len(), 1);
    }

    #[test]
    fn a_whole_copy_and_a_chunked_copy_are_both_copies() {
        for (bytes, every) in files() {
            let (list, chunks) = chunked(&bytes, every);
            // A bad whole copy, searched first, and a good chunk list.
            let (_root, tmp, packs) = dirs();
            plant_damaged_copies(&packs, &[(ObjectKind::Content, &bytes)]);
            pack_with(&tmp, &packs, None, &pieces(&chunks), &[&list]);
            let store = Store::open(&packs).unwrap();
            let mut out = Vec::new();
            store.copy_content(&list.file, &mut out).unwrap();
            assert!(out == bytes);
            assert_eq!(store.damaged_packs().len(), 1);
            // A list that builds another file, searched first, and a good whole copy.
            let (_root, tmp, packs) = dirs();
            let mut other = bytes.clone();
            other[0] ^= 1;
            let (mut lying, other_chunks) = chunked(&other, every);
            lying.file = list.file;
            pack_with(
                &tmp,
                &packs,
                Some("00.pack"),
                &pieces(&other_chunks),
                &[&lying],
            );
            pack_with(&tmp, &packs, None, &[&bytes], &[]);
            let store = Store::open(&packs).unwrap();
            let mut out = Vec::new();
            store.copy_content(&list.file, &mut out).unwrap();
            assert!(out == bytes);
            let damaged = store.damaged_packs();
            assert_eq!(damaged.len(), 1, "{damaged:?}");
            assert!(damaged[0].to_string().contains("00.pack"), "{damaged:?}");
        }
    }

    #[test]
    fn has_good_copy_never_counts_a_chunk_list() {
        let (_root, tmp, packs) = dirs();
        let (list, chunks) = chunked(b"hello world", 6);
        pack_with(&tmp, &packs, None, &pieces(&chunks), &[&list]);
        let store = Store::open(&packs).unwrap();
        // The list builds its file...
        store.copy_content(&list.file, &mut Vec::new()).unwrap();
        // ...but a writer relies on one only once it has cut the file into the same chunks.
        for kind in [ObjectKind::Content, ObjectKind::ChunkList] {
            assert!(!store.has_good_copy(&list.file, kind, 11).unwrap());
        }
        assert!(
            store
                .has_good_copy(&list.chunks[0].id, ObjectKind::Content, 6)
                .unwrap()
        );
        assert!(store.warnings().is_empty(), "{:?}", store.warnings());
    }

    #[test]
    fn holds_chunk_list_matches_only_the_same_list() {
        let (_root, tmp, packs) = dirs();
        let (list, _) = chunked(b"hello world", 6);
        pack_with(&tmp, &packs, None, &[], &[&list]);
        let store = Store::open(&packs).unwrap();
        assert!(store.holds_chunk_list(&list).unwrap());
        let (other_cut, _) = chunked(b"hello world", 4);
        let mut other_file = list.clone();
        other_file.file = hash_content(b"HELLO WORLD");
        for other in [other_cut, other_file] {
            assert!(!store.holds_chunk_list(&other).unwrap());
        }
        // Nor does the file stored whole count.
        let (whole, _) = chunked(b"stored whole", 6);
        pack_with(&tmp, &packs, None, &[b"stored whole"], &[]);
        let store = Store::open(&packs).unwrap();
        assert!(!store.holds_chunk_list(&whole).unwrap());
        assert!(store.warnings().is_empty(), "{:?}", store.warnings());
    }

    #[test]
    fn missing_object_is_an_error() {
        let (_root, _tmp, packs) = dirs();
        let store = Store::open(&packs).unwrap();
        let id = hash_content(b"nope");
        assert!(!store.has_good_copy(&id, ObjectKind::Content, 4).unwrap());
        assert!(matches!(
            store.copy_content(&id, &mut Vec::new()),
            Err(StoreError::Missing(m)) if m == id
        ));
    }

    /// A listing of one file, `name`.
    fn one_file(name: &str) -> Listing {
        use crate::listing::{Entry, EntryKind};
        let entry = Entry {
            name: name.into(),
            kind: EntryKind::File,
            id: hash_content(name.as_bytes()),
            size: name.len() as u64,
        };
        Listing::new(vec![entry]).unwrap()
    }

    /// Writes a pack of `objects` into `packs` as `name`.
    fn pack_as(packs: &Path, name: &str, objects: &[(ObjectKind, &[u8])]) {
        let made = tempfile::tempdir().unwrap();
        let mut w = PackWriter::create(made.path()).unwrap();
        for (kind, bytes) in objects {
            match kind {
                ObjectKind::Content => w.add_content(bytes),
                _ => w.add_record(*kind, bytes),
            }
            .unwrap();
        }
        let path = w.finish(made.path()).unwrap().unwrap();
        std::fs::rename(path, packs.join(name)).unwrap();
    }

    /// Writes a pack of bad copies of `objects` into `packs` as `name`.
    fn damaged_as(packs: &Path, name: &str, objects: &[(ObjectKind, &[u8])]) {
        plant_damaged_copies(packs, objects);
        std::fs::rename(packs.join("00.pack"), packs.join(name)).unwrap();
    }

    #[test]
    fn a_second_handle_leaves_the_first_store_as_it_was() {
        let (_root, _tmp, packs) = dirs();
        let (first, second, twice) = (one_file("a"), one_file("b"), one_file("c"));
        let (first_bytes, second_bytes) = (first.encode(), second.encode());
        let twice_bytes = twice.encode();
        let listing = ObjectKind::FolderListing;
        pack_as(
            &packs,
            "a.pack",
            &[(listing, &first_bytes), (ObjectKind::Content, b"data")],
        );
        pack_as(&packs, "b.pack", &[(listing, &second_bytes)]);
        damaged_as(&packs, "c.pack", &[(listing, &twice_bytes)]);
        pack_as(&packs, "d.pack", &[(listing, &twice_bytes)]);
        std::fs::write(packs.join("zz.pack"), b"not a pack").unwrap();

        let store = Store::open(&packs).unwrap();
        store
            .copy_content(&hash_content(b"data"), &mut Vec::new())
            .unwrap();
        store.listing(&second.id()).unwrap();
        let (before, open) = (store.remembered(), store.open_count());
        assert!(open > 0 && before.contains("a.pack"), "{before}");

        let other = store.second();
        let peeked = other.peek_listing(&first.id()).unwrap();
        assert_eq!(peeked.listing, first);
        assert_eq!(peeked.size, first_bytes.len());
        assert!(other.peek_listing(&second.id()).is_some());
        assert!(other.peek_listing(&twice.id()).is_none());
        assert!(other.peek_listing(&hash_content(b"nowhere")).is_none());
        assert!(other.warnings().is_empty());
        assert_eq!(store.remembered(), before);
        assert_eq!(store.open_count(), open);
        assert!(store.warnings().is_empty(), "{:?}", store.warnings());
    }

    /// What a read of `id` brought: the lookups, record reads and reads of pack files made.
    type Spent = (usize, usize, usize);

    /// Opens two stores on `packs` and brings both to one state with `prepare`; then one reads
    /// `id` with `listing`, the other with `listing_after`, given what a second handle peeked.
    /// Both must end alike: the same answer, the same memory of packs and copies.
    fn alike(packs: &Path, id: Id, prepare: impl Fn(&Store)) -> Spent {
        let (one, two) = (Store::open(packs).unwrap(), Store::open(packs).unwrap());
        prepare(&one);
        prepare(&two);
        assert_eq!(one.remembered(), two.remembered());
        let peeked = two.second().peek_listing(&id).unwrap();
        let plain = format!("{:?}", one.listing(&id));
        let ((finds, records), reads) = (counted::now(), open_files::reads());
        let after = format!("{:?}", two.listing_after(&id, peeked));
        let spent = (
            counted::now().0 - finds,
            counted::now().1 - records,
            open_files::reads() - reads,
        );
        assert_eq!(after, plain);
        assert_eq!(two.remembered(), one.remembered());
        assert_eq!(two.warnings(), one.warnings());
        spent
    }

    #[test]
    fn listing_after_leaves_the_store_as_listing_does() {
        let listing = one_file("a");
        let bytes = listing.encode();
        let kind = ObjectKind::FolderListing;
        let data = (ObjectKind::Content, &b"data"[..]);
        let read_data = |s: &Store| {
            s.copy_content(&hash_content(b"data"), &mut Vec::new())
                .unwrap();
        };
        // Opens every pack, the last first in the list of open ones.
        let open_all = |s: &Store| {
            s.listing(&hash_content(b"nowhere")).unwrap_err();
        };

        // One copy, in the pack asked first: nothing is looked up or read again.
        let (_root, _tmp, packs) = dirs();
        pack_as(&packs, "a.pack", &[(kind, &bytes), data]);
        assert_eq!(alike(&packs, listing.id(), read_data), (0, 0, 0));

        // One copy, in another pack than the one asked first: the store's own search runs.
        let (_root, _tmp, packs) = dirs();
        pack_as(&packs, "a.pack", &[(kind, &bytes)]);
        pack_as(&packs, "b.pack", &[data]);
        let (finds, records, _) = alike(&packs, listing.id(), read_data);
        assert!(finds > 0);
        assert_eq!(records, 0);

        // Two copies, the store's first damaged and the second's the good one.
        let (_root, _tmp, packs) = dirs();
        pack_as(&packs, "a.pack", &[data]);
        pack_as(&packs, "g.pack", &[(kind, &bytes)]);
        damaged_as(&packs, "z.pack", &[(kind, &bytes)]);
        alike(&packs, listing.id(), open_all);
        let store = Store::open(&packs).unwrap();
        open_all(&store);
        store.listing(&listing.id()).unwrap();
        assert!(!store.warnings().is_empty());

        // The second's copy is the store's first, which it knows to be bad.
        let (_root, _tmp, packs) = dirs();
        pack_as(&packs, "g.pack", &[(kind, &bytes)]);
        pack_as(&packs, "h.pack", &[(kind, &bytes)]);
        let known_bad = |s: &Store| {
            let reason = (packs.join("g.pack"), "planted".to_owned());
            s.checked().bad.insert((0, listing.id()), reason);
        };
        let (_, records, _) = alike(&packs, listing.id(), known_bad);
        assert_eq!(records, 1);

        // The same, with that copy's pack the last one read, so the shortcut meets the check.
        let (_root, _tmp, packs) = dirs();
        pack_as(&packs, "g.pack", &[(kind, &bytes), data]);
        pack_as(&packs, "h.pack", &[(kind, &bytes)]);
        let read_then_bad = |s: &Store| {
            read_data(s);
            let reason = (packs.join("g.pack"), "planted".to_owned());
            s.checked().bad.insert((0, listing.id()), reason);
        };
        let (_, records, _) = alike(&packs, listing.id(), read_then_bad);
        assert_eq!(records, 1);

        // The store's first copy isn't a folder listing.
        let (_root, _tmp, packs) = dirs();
        pack_as(&packs, "a.pack", &[data]);
        pack_as(&packs, "g.pack", &[(kind, &bytes)]);
        plant_lying(&packs, "z.pack", ObjectKind::Content, &listing.id(), &bytes);
        alike(&packs, listing.id(), open_all);
    }

    #[test]
    fn a_peeked_copy_that_isnt_the_stores_is_never_used() {
        let (_root, _tmp, packs) = dirs();
        let listing = one_file("a");
        let bytes = listing.encode();
        pack_as(&packs, "a.pack", &[(ObjectKind::FolderListing, &bytes)]);
        pack_as(&packs, "b.pack", &[(ObjectKind::FolderListing, &bytes)]);
        let peek = || {
            let mut peeked = Store::open(&packs)
                .unwrap()
                .second()
                .peek_listing(&listing.id())
                .unwrap();
            peeked.listing = Listing::empty();
            peeked
        };
        let in_another_pack = || Peeked {
            pack: packs.join("b.pack"),
            ..peek()
        };
        let moved = || {
            let mut peeked = peek();
            peeked.entry.offset += 1;
            peeked
        };
        // A store that has read from `a.pack` asks it first, and takes a line of its index
        // unlooked-up only from a peek of that pack, which only `peek_listing` makes.
        let warm = Store::open(&packs).unwrap();
        warm.listing(&listing.id()).unwrap();
        let cold = || Store::open(&packs).unwrap();
        for (store, peeked) in [
            (cold(), in_another_pack()),
            (warm, in_another_pack()),
            (cold(), moved()),
        ] {
            let records = counted::now().1;
            assert_eq!(store.listing_after(&listing.id(), peeked).unwrap(), listing);
            assert_eq!(counted::now().1, records + 1);
        }
    }

    #[test]
    fn wrong_kind_is_an_error() {
        let (_root, tmp, packs) = dirs();
        pack_of(&tmp, &packs, &[b"data".to_vec()]);
        let store = Store::open(&packs).unwrap();
        let id = hash_content(b"data");
        assert!(matches!(
            store.read_record(&id, ObjectKind::FolderListing),
            Err(StoreError::WrongKind {
                expected: ObjectKind::FolderListing,
                found: ObjectKind::Content,
                ..
            })
        ));
    }
}
