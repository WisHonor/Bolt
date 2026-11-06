//! One view over every pack in the store. Packs are listed when the store opens, opened when first
//! searched, and searched on disk; at most `MAX_OPEN` stay open.
use bolt_platform::dir::Dir;

use crate::encoding::DecodeError;
use crate::id::Id;
use crate::listing::Listing;
use crate::pack::{IndexEntry, ObjectKind, PackError, PackReader};
use crate::save::Save;
use crate::text::escape;
use crate::warning::{self, Warning};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::ffi::OsString;
use std::io::{self, Write};
use std::ops::ControlFlow;
use std::path::{Path, PathBuf};
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

/// A copy found in pack `n`. It doesn't hold the pack open: however many packs hold an object,
/// at most `MAX_OPEN` stay open.
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

pub struct Store {
    dir: Arc<Dir>,
    path: PathBuf,
    packs: Mutex<Packs>,
    checked: Mutex<Checked>,
    #[cfg(test)]
    saves_read: std::sync::atomic::AtomicUsize,
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
    /// The pack that held the last object read.
    last: usize,
}

impl Packs {
    /// Adds the packs among `names` not listed yet and, unless `keep`, forgets those listed but
    /// not among them.
    fn list(&mut self, mut names: Vec<OsString>, keep: bool) {
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
}

/// What reading copies has shown so far.
#[derive(Default)]
struct Checked {
    /// Copies that were read whole and passed every check.
    good: HashSet<At>,
    /// Copies that failed, each with its problem. They are never read again.
    bad: HashMap<At, String>,
    /// Each pack that held a bad copy, with the first problem found in it.
    damaged: BTreeMap<PathBuf, String>,
    /// Each pack with a copy that couldn't be read for a reason other than damage, though another
    /// copy of the same object could, with the first reason found.
    unreadable: BTreeMap<PathBuf, String>,
    /// Warnings about the store from the command itself, such as a merge that didn't finish.
    notes: Vec<Warning>,
}

/// What the copies of one object tried so far came to.
#[derive(Default)]
struct Tries {
    wrong_kind: Option<ObjectKind>,
    damage: Option<PackError>,
    /// A failure that says nothing about the copy, such as a lack of memory.
    other: Option<PackError>,
    unreadable: Vec<(PathBuf, String)>,
}

impl Tries {
    /// Keeps `e`, a failure that wasn't a copy's own.
    fn note(&mut self, e: PackError) {
        if is_damage(&e) {
            self.damage.get_or_insert(e);
        } else {
            self.other.get_or_insert(e);
        }
    }

    fn error(self, id: &Id, kind: ObjectKind) -> StoreError {
        // A failure that says nothing about the copy comes first: calling it damage would mislead.
        match (self.other.or(self.damage), self.wrong_kind) {
            (Some(e), _) => StoreError::Pack(e),
            (None, Some(found)) => StoreError::WrongKind {
                id: *id,
                expected: kind,
                found,
            },
            (None, None) => StoreError::Missing(*id),
        }
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
            #[cfg(test)]
            saves_read: std::sync::atomic::AtomicUsize::new(0),
        };
        store.list()?;
        Ok(store)
    }

    /// Lists the packs again, to find those another process committed since this store was
    /// opened.
    pub fn refresh(&mut self) -> Result<(), StoreError> {
        self.list()
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
            if Path::new(&entry.name)
                .extension()
                .is_some_and(|e| e == "pack")
            {
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
        let mut skipped = None;
        let found = self.each_pack(&mut skipped, |_, pack| {
            for e in pack.entries() {
                let e = e.map_err(StoreError::from)?;
                if e.kind == kind && test(&e.id)? {
                    return Ok::<_, E>(ControlFlow::Break(()));
                }
            }
            Ok(ControlFlow::Continue(()))
        })?;
        match (found, skipped) {
            (Some(()), _) => Ok(true),
            // A pack that couldn't be read may hold one.
            (None, Some(e)) => Err(StoreError::from(e).into()),
            (None, None) => Ok(false),
        }
    }

    /// Whether any pack holds a file's content, as its index lists it.
    pub fn holds_content(&self) -> Result<bool, StoreError> {
        self.any_entry(ObjectKind::Content, |_| Ok(true))
    }

    /// Whether a copy of `id` reads back whole and matches its fingerprint, so storing it can be
    /// skipped. A copy recording another `size` than the object's is damage, and is never read.
    pub fn has_good_copy(&self, id: &Id, kind: ObjectKind, size: u64) -> Result<bool, StoreError> {
        let mut tries = Tries::default();
        let mut skipped = None;
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
            if good || self.attempt(&c, kind, &mut tries, read_whole(kind)).is_ok() {
                return ControlFlow::Break(());
            }
            ControlFlow::Continue(())
        })?;
        // A pack that couldn't be read may hold a good copy, so its error is the answer.
        match (found, tries.other.or(skipped)) {
            (Some(()), _) => Ok(true),
            (None, Some(e)) => Err(e.into()),
            (None, None) => Ok(false),
        }
    }

    /// The packs found holding a bad copy of an object, each with the first problem found in it.
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
    /// a copy that couldn't be read, though another copy could.
    #[must_use]
    pub fn warnings(&self) -> Vec<Warning> {
        let mut warnings: Vec<Warning> = self
            .damaged_packs()
            .iter()
            .map(|pack| Warning::new(warning::DAMAGED_PACK, format!("{pack}.")))
            .collect();
        let checked = self.checked();
        warnings.extend(checked.unreadable.iter().map(|(path, reason)| {
            Warning::new(
                warning::COPY_UNREADABLE,
                format!(
                    "Bolt couldn't read a copy in pack {}: {}.",
                    escape(&path.display().to_string(), false),
                    escape(reason.trim_end_matches('.'), false)
                ),
            )
        }));
        warnings.extend(checked.notes.iter().cloned());
        warnings
    }

    /// Keeps `warning` to come with the others.
    pub(crate) fn note(&self, warning: Warning) {
        self.checked().notes.push(warning);
    }

    /// Closes every pack kept open, so they can be removed; they are opened again when searched.
    pub(crate) fn close_all(&self) {
        self.packs().open.clear();
    }

    /// Reads a structured record of the given kind, fingerprint checked.
    pub fn read_record(&self, id: &Id, kind: ObjectKind) -> Result<Vec<u8>, StoreError> {
        self.first_good(id, kind, false, |pack, entry| pack.read_record(entry))
    }

    /// Streams content into `out`; treat the output as untrusted until this returns `Ok`.
    pub fn copy_content(&self, id: &Id, out: &mut dyn Write) -> Result<(), StoreError> {
        let content = ObjectKind::Content;
        let first = self.first_copy(id)?;
        let mut tries = Tries::default();
        // Bytes can't be taken back out of `out`, so it gets one copy only.
        let known_good = |c: &Copy| c.entry.kind == content && self.is_good(c);
        if let Some(c) = &first
            && !known_good(c)
        {
            // A small first copy, and only that one, is read whole before anything goes out.
            if c.entry.kind == content && c.entry.uncompressed_size <= NEAR {
                let mut bytes = Vec::new();
                let read = self.attempt(c, content, &mut tries, |pack, entry| {
                    bytes.clear();
                    pack.copy_content(entry, &mut bytes)
                });
                if read.is_ok() {
                    return Ok(out.write_all(&bytes).map_err(PackError::from)?);
                }
            }
            // Otherwise it is streamed: when several packs hold it, a good copy is found first.
            if self.all_copies(id)?.0.len() > 1 {
                let check = read_whole(content);
                let tried = std::mem::take(&mut tries);
                self.first_good_from(first.clone(), tried, id, content, false, check)?;
            }
        }
        self.first_good_from(first, tries, id, content, true, |pack, entry| {
            pack.copy_content(entry, out)
        })
    }

    pub fn listing(&self, id: &Id) -> Result<Listing, StoreError> {
        Ok(Listing::decode(
            &self.read_record(id, ObjectKind::FolderListing)?,
        )?)
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
        read: impl FnMut(&PackReader, &IndexEntry) -> Result<T, PackError>,
    ) -> Result<T, StoreError> {
        let first = self.first_copy(id)?;
        self.first_good_from(first, Tries::default(), id, kind, once, read)
    }

    /// `first_good`, given the first copy found and what reading copies has come to so far.
    fn first_good_from<T>(
        &self,
        first: Option<Copy>,
        mut tries: Tries,
        id: &Id,
        kind: ObjectKind,
        once: bool,
        mut read: impl FnMut(&PackReader, &IndexEntry) -> Result<T, PackError>,
    ) -> Result<T, StoreError> {
        let Some(first) = first else {
            return Err(StoreError::Missing(*id));
        };
        let near = first.entry.uncompressed_size <= NEAR || self.is_good(&first);
        if near {
            match self.attempt(&first, kind, &mut tries, &mut read) {
                Ok(value) => return Ok(value),
                Err(true) if once => return Err(tries.error(id, kind)),
                Err(_) => {}
            }
        }
        let (mut copies, skipped) = self.all_copies(id)?;
        if let Some(e) = skipped {
            tries.note(e);
        }
        if near {
            copies.retain(|c| c.n != first.n);
        }
        {
            let checked = self.checked();
            copies.sort_by_key(|c| (!checked.good.contains(&c.at()), c.entry.uncompressed_size));
        }
        for c in copies {
            match self.attempt(&c, kind, &mut tries, &mut read) {
                Ok(value) => return Ok(value),
                Err(true) if once => break,
                Err(_) => {}
            }
        }
        Err(tries.error(id, kind))
    }

    /// Reads the copy `c` with `read`, unless it is of another kind or known bad, noting what
    /// happened in `tries`. A failure says whether the copy was read.
    fn attempt<T>(
        &self,
        c: &Copy,
        kind: ObjectKind,
        tries: &mut Tries,
        mut read: impl FnMut(&PackReader, &IndexEntry) -> Result<T, PackError>,
    ) -> Result<T, bool> {
        if c.entry.kind != kind {
            tries.wrong_kind.get_or_insert(c.entry.kind);
            return Err(false);
        }
        let path = || self.pack_path(c.n);
        // A bad copy is never read again but still counts, so no good copy is damage every time.
        let known_bad = self.checked().bad.get(&c.at()).cloned();
        if let Some(reason) = known_bad {
            tries.damage.get_or_insert_with(|| PackError::Corrupt {
                path: path(),
                reason,
            });
            return Err(false);
        }
        match self.pack(c.n).and_then(|pack| read(&pack, &c.entry)) {
            Ok(value) => {
                self.packs().last = c.n;
                let mut checked = self.checked();
                checked.good.insert(c.at());
                // Worth a warning only now: with no good copy the read fails, saying why.
                for (path, reason) in tries.unreadable.drain(..) {
                    checked.unreadable.entry(path).or_insert(reason);
                }
                Ok(value)
            }
            Err(e) if is_damage(&e) => {
                let reason = self.note_damage(c.at(), path(), &e);
                tries.damage.get_or_insert_with(|| PackError::Corrupt {
                    path: path(),
                    reason,
                });
                Err(true)
            }
            // Memory or I/O: the copy may be fine, so it is neither remembered nor damage.
            Err(e) => {
                tries.unreadable.push((path(), e.to_string()));
                tries.other.get_or_insert(e);
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
        let mut skipped = None;
        match (self.search(id, &mut skipped, ControlFlow::Break)?, skipped) {
            (None, Some(e)) => Err(e.into()),
            (found, _) => Ok(found),
        }
    }

    /// Every copy of `id` found, and the error of the first pack that couldn't be read.
    fn all_copies(&self, id: &Id) -> Result<(Vec<Copy>, Option<PackError>), StoreError> {
        let mut copies = Vec::new();
        let mut skipped = None;
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
        skipped: &mut Option<PackError>,
        mut visit: impl FnMut(Copy) -> ControlFlow<T>,
    ) -> Result<Option<T>, StoreError> {
        let mut unsearched = Vec::new();
        let found = self.each_pack(skipped, |n, pack| {
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

    /// Notes pack `n`, which couldn't be opened or searched, as damaged if `e` says so, and keeps
    /// the first such error in `skipped`.
    fn pass_over(&self, n: usize, e: PackError, skipped: &mut Option<PackError>) {
        if let PackError::Corrupt { reason, .. } = &e {
            let path = self.pack_path(n);
            let reason = reason.clone();
            self.checked().damaged.entry(path).or_insert(reason);
        }
        skipped.get_or_insert(e);
    }

    /// Hands `f` each pack, first the one that held the last object read, then the others open,
    /// then the closed ones by name, until it breaks. Open ones first, so a lookup that finds
    /// nothing opens only the packs that don't fit among the open ones. A pack gone when it is
    /// opened (merged into another) sends the store to list the packs again, once; a pack already
    /// handed out isn't handed out again. A pack that can't be opened is passed over, noted as
    /// damaged, and its error kept in `skipped`: a lookup can still find a good copy elsewhere,
    /// but can't tell that the pack doesn't hold one.
    fn each_pack<T, E: From<StoreError>>(
        &self,
        skipped: &mut Option<PackError>,
        mut f: impl FnMut(usize, &Arc<PackReader>) -> Result<ControlFlow<T>, E>,
    ) -> Result<Option<T>, E> {
        let mut done = HashSet::new();
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
    /// the least recently used past `MAX_OPEN`.
    fn pack(&self, n: usize) -> Result<Arc<PackReader>, PackError> {
        let name = {
            let mut packs = self.packs();
            if let Some(i) = packs.open.iter().position(|(m, _)| *m == n) {
                let open = packs.open.remove(i);
                let pack = Arc::clone(&open.1);
                packs.open.insert(0, open);
                return Ok(pack);
            }
            // Closed first, so no more than `MAX_OPEN` are open even for a moment.
            packs.open.truncate(MAX_OPEN - 1);
            packs.names.get(n).cloned().flatten()
        };
        let name = name.ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))?;
        let path = self.path.join(&name);
        let pack = Arc::new(PackReader::open_in(&self.dir, &name, &path)?);
        let mut packs = self.packs();
        packs.open.insert(0, (n, Arc::clone(&pack)));
        packs.open.truncate(MAX_OPEN);
        Ok(pack)
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

    /// Marks a copy bad and its pack damaged, and returns the problem found.
    fn note_damage(&self, at: At, path: PathBuf, e: &PackError) -> String {
        let reason = match e {
            PackError::Corrupt { reason, .. } => reason.clone(),
            other => other.to_string(),
        };
        let mut checked = self.checked();
        checked.bad.insert(at, reason.clone());
        checked
            .damaged
            .entry(path)
            .or_insert_with(|| reason.clone());
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

/// A copy that fails its own checks is damage; an I/O error or a lack of memory says nothing
/// about the copy.
pub(crate) fn is_damage(e: &PackError) -> bool {
    matches!(
        e,
        PackError::FingerprintMismatch { .. } | PackError::Corrupt { .. }
    )
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;
    use crate::id::hash_content;
    use crate::pack::{PackWriter, open_files};
    use crate::repo::RepoError;
    use crate::test_support::plant_damaged_copies;
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
        assert_eq!(warnings[0].kind, "damaged_pack");
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
                "Bolt couldn't read a copy in pack {}: the disk\\x1b[2J said no.",
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
        let out_of_memory = |pack: &PackReader, entry: &IndexEntry| {
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
