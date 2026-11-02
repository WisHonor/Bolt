//! One view over every pack in the store. The lookup index lives in memory and is rebuilt on open.
use crate::encoding::DecodeError;
use crate::id::Id;
use crate::listing::Listing;
use crate::pack::{IndexEntry, ObjectKind, PackError, PackReader};
use crate::save::Save;
use std::collections::hash_map::Entry;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::ffi::OsString;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, PoisonError};

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

/// A copy of an object: (pack number, entry number).
type At = (usize, usize);

pub struct Store {
    packs: Vec<PackReader>,
    /// Id to its first copy.
    index: HashMap<Id, At>,
    /// The further copies of ids that more than one pack holds, in the packs' order.
    more: HashMap<Id, Vec<At>>,
    /// File names of the registered packs.
    known: HashSet<OsString>,
    checked: Mutex<Checked>,
}

/// What reading copies has shown so far.
#[derive(Default)]
struct Checked {
    /// Each copy read so far, and whether it passed every check.
    copies: HashMap<At, bool>,
    /// Each pack that held a bad copy, with the first problem found in it.
    damaged: BTreeMap<PathBuf, String>,
}

impl Store {
    /// Opens every `*.pack` file in the folder and ignores everything else.
    pub fn open(packs_dir: &Path) -> Result<Store, StoreError> {
        let mut store = Store {
            packs: Vec::new(),
            index: HashMap::new(),
            more: HashMap::new(),
            known: HashSet::new(),
            checked: Mutex::default(),
        };
        store.refresh(packs_dir)?;
        Ok(store)
    }

    /// Registers every `*.pack` file in the folder that is not registered yet, such as packs
    /// another process committed since this store was opened.
    pub fn refresh(&mut self, packs_dir: &Path) -> Result<(), StoreError> {
        let mut paths = Vec::new();
        for entry in std::fs::read_dir(packs_dir).map_err(PackError::from)? {
            let path = entry.map_err(PackError::from)?.path();
            let new = path.file_name().is_some_and(|n| !self.known.contains(n));
            if new && path.extension().is_some_and(|e| e == "pack") {
                paths.push(path);
            }
        }
        paths.sort();
        for path in paths {
            self.register(&path)?;
        }
        Ok(())
    }

    /// Adds a finished pack. An object an earlier pack holds too gets a further copy here, read if
    /// the earlier ones fail.
    pub fn register(&mut self, pack_path: &Path) -> Result<(), StoreError> {
        let reader = PackReader::open(pack_path)?;
        let n = self.packs.len();
        for (i, e) in reader.entries().iter().enumerate() {
            match self.index.entry(e.id) {
                Entry::Vacant(first) => {
                    first.insert((n, i));
                }
                Entry::Occupied(_) => self.more.entry(e.id).or_default().push((n, i)),
            }
        }
        self.packs.push(reader);
        if let Some(name) = pack_path.file_name() {
            self.known.insert(name.to_owned());
        }
        Ok(())
    }

    #[must_use]
    pub fn pack_count(&self) -> usize {
        self.packs.len()
    }

    /// Whether the store holds a copy of `id` of `kind` that reads back whole and matches its
    /// fingerprint, so storing it again can be skipped. Once a copy passes, it isn't read again.
    pub fn has_good_copy(&self, id: &Id, kind: ObjectKind) -> Result<bool, StoreError> {
        match self.verify(id, kind) {
            Ok(()) => Ok(true),
            Err(StoreError::Missing(_) | StoreError::WrongKind { .. }) => Ok(false),
            Err(StoreError::Pack(e)) if is_damage(&e) => Ok(false),
            Err(e) => Err(e),
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

    /// Reads a structured record of the given kind, fingerprint checked.
    pub fn read_record(&self, id: &Id, kind: ObjectKind) -> Result<Vec<u8>, StoreError> {
        self.first_good(id, kind, false, |pack, entry| pack.read_record(entry))
    }

    /// Streams content into `out`; treat the output as untrusted until this returns `Ok`.
    pub fn copy_content(&self, id: &Id, out: &mut dyn Write) -> Result<(), StoreError> {
        // Bytes can't be taken back out of `out`, so it gets one copy only: when several packs
        // hold `id`, the good one is found first.
        if self.more.contains_key(id) {
            self.verify(id, ObjectKind::Content)?;
        }
        self.first_good(id, ObjectKind::Content, true, |pack, entry| {
            pack.copy_content(entry, out)
        })
    }

    pub fn listing(&self, id: &Id) -> Result<Listing, StoreError> {
        Ok(Listing::decode(
            &self.read_record(id, ObjectKind::FolderListing)?,
        )?)
    }

    pub fn save(&self, id: &Id, signatures_allowed: bool) -> Result<Save, StoreError> {
        Ok(Save::decode(
            &self.read_record(id, ObjectKind::Save)?,
            signatures_allowed,
        )?)
    }

    /// Finds a copy of `id` of `kind` that reads back whole and matches its fingerprint, unless
    /// one already has.
    fn verify(&self, id: &Id, kind: ObjectKind) -> Result<(), StoreError> {
        let known_good = {
            let checked = self.checked();
            self.copies(id).any(|at| {
                checked.copies.get(&at) == Some(&true)
                    && self.entry(at).is_some_and(|(_, e)| e.kind == kind)
            })
        };
        if known_good {
            return Ok(());
        }
        self.first_good(id, kind, false, |pack, entry| match kind {
            ObjectKind::Content => pack.copy_content(entry, &mut io::sink()),
            _ => pack.read_record(entry).map(drop),
        })
    }

    /// Reads the copies of `id` of `kind` with `read`, in the packs' order, until one passes. A
    /// bad copy is noted and never read again, so a damaged pack can't hide a good copy in
    /// another. With `once`, the first copy read is the last.
    fn first_good<T>(
        &self,
        id: &Id,
        kind: ObjectKind,
        once: bool,
        mut read: impl FnMut(&PackReader, &IndexEntry) -> Result<T, PackError>,
    ) -> Result<T, StoreError> {
        let mut problem = StoreError::Missing(*id);
        for at in self.copies(id) {
            let Some((pack, entry)) = self.entry(at) else {
                continue;
            };
            if self.checked().copies.get(&at) == Some(&false) {
                continue;
            }
            if entry.kind != kind {
                if matches!(problem, StoreError::Missing(_)) {
                    problem = StoreError::WrongKind {
                        id: *id,
                        expected: kind,
                        found: entry.kind,
                    };
                }
                continue;
            }
            match read(pack, entry) {
                Ok(value) => {
                    self.checked().copies.insert(at, true);
                    return Ok(value);
                }
                Err(e) if is_damage(&e) => {
                    self.note_damage(at, pack, &e);
                    problem = e.into();
                    if once {
                        break;
                    }
                }
                Err(e) => return Err(e.into()),
            }
        }
        Err(problem)
    }

    fn copies(&self, id: &Id) -> impl Iterator<Item = At> + '_ {
        let more = self.more.get(id).into_iter().flatten();
        self.index.get(id).into_iter().chain(more).copied()
    }

    fn entry(&self, (p, e): At) -> Option<(&PackReader, &IndexEntry)> {
        let pack = self.packs.get(p)?;
        Some((pack, pack.entries().get(e)?))
    }

    fn note_damage(&self, at: At, pack: &PackReader, e: &PackError) {
        let reason = match e {
            PackError::Corrupt { reason, .. } => reason.clone(),
            other => other.to_string(),
        };
        let mut checked = self.checked();
        checked.copies.insert(at, false);
        checked
            .damaged
            .entry(pack.path().to_owned())
            .or_insert(reason);
    }

    fn checked(&self) -> MutexGuard<'_, Checked> {
        self.checked.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// A copy that fails its own checks is damage; an I/O error or a lack of memory says nothing
/// about the copy.
fn is_damage(e: &PackError) -> bool {
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
    use crate::pack::PackWriter;
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
                store.register(&path).unwrap();
            }
            let fresh = Store::open(&packs).unwrap();
            for c in &contents {
                let id = hash_content(c);
                prop_assert!(fresh.has_good_copy(&id, ObjectKind::Content).unwrap());
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
        store.refresh(&packs).unwrap();
        store.refresh(&packs).unwrap();
        assert_eq!(store.pack_count(), 2);
        assert!(
            store
                .has_good_copy(&hash_content(b"two"), ObjectKind::Content)
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
    }

    #[test]
    fn only_a_copy_that_reads_back_counts_as_good() {
        let (_root, tmp, packs) = dirs();
        let content = b"bytes anyone could predict".to_vec();
        let id = hash_content(&content);
        plant_damaged_copies(&packs, &[(ObjectKind::Content, &content)]);
        let mut store = Store::open(&packs).unwrap();
        assert!(!store.has_good_copy(&id, ObjectKind::Content).unwrap());
        assert_eq!(store.damaged_packs().len(), 1);
        // A good copy arriving later is found past the damaged one.
        pack_of(&tmp, &packs, &[content]);
        store.refresh(&packs).unwrap();
        assert!(store.has_good_copy(&id, ObjectKind::Content).unwrap());
        assert!(!store.has_good_copy(&id, ObjectKind::FolderListing).unwrap());
    }

    #[test]
    fn missing_object_is_an_error() {
        let (_root, _tmp, packs) = dirs();
        let store = Store::open(&packs).unwrap();
        let id = hash_content(b"nope");
        assert!(!store.has_good_copy(&id, ObjectKind::Content).unwrap());
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
