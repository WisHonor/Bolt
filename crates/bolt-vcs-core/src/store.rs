//! One view over every pack in the store. The lookup index lives in memory and is rebuilt on open.
use crate::encoding::DecodeError;
use crate::id::Id;
use crate::listing::Listing;
use crate::pack::{IndexEntry, ObjectKind, PackError, PackReader};
use crate::save::Save;
use crate::text::escape;
use crate::warning::{self, Warning};
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
    /// Copies that were read whole and passed every check.
    good: HashSet<At>,
    /// Copies that failed, each with its problem. They are never read again.
    bad: HashMap<At, String>,
    /// Each pack that held a bad copy, with the first problem found in it.
    damaged: BTreeMap<PathBuf, String>,
    /// Each pack with a copy that couldn't be read for a reason other than damage, though another
    /// copy of the same object could, with the first reason found.
    unreadable: BTreeMap<PathBuf, String>,
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

    /// Whether a copy of `id` reads back whole and matches its fingerprint, so storing it can be
    /// skipped. A copy recording another `size` than the object's is damage, and is never read.
    pub fn has_good_copy(&self, id: &Id, kind: ObjectKind, size: u64) -> Result<bool, StoreError> {
        for at in self.copies(id) {
            if let Some((pack, entry)) = self.entry(at)
                && entry.kind == kind
                && entry.uncompressed_size != size
            {
                let wrong = PackError::Corrupt {
                    path: pack.path().to_owned(),
                    reason: format!("its copy of {id} records the wrong size"),
                };
                self.note_damage(at, pack, &wrong);
            }
        }
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
        warnings
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
                checked.good.contains(&at) && self.entry(at).is_some_and(|(_, e)| e.kind == kind)
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

    /// Reads copies of `id` with `read` until one passes: known good first, then smallest first, so
    /// a copy claiming a huge size comes last. With `once`, the first copy read is the last.
    fn first_good<T>(
        &self,
        id: &Id,
        kind: ObjectKind,
        once: bool,
        mut read: impl FnMut(&PackReader, &IndexEntry) -> Result<T, PackError>,
    ) -> Result<T, StoreError> {
        let mut copies: Vec<(At, &PackReader, &IndexEntry)> = self
            .copies(id)
            .filter_map(|at| self.entry(at).map(|(pack, entry)| (at, pack, entry)))
            .collect();
        {
            let checked = self.checked();
            copies.sort_by_key(|&(at, _, e)| (!checked.good.contains(&at), e.uncompressed_size));
        }
        let (mut wrong_kind, mut damage, mut other) = (None, None, None);
        let mut unreadable = Vec::new();
        for (at, pack, entry) in copies {
            if entry.kind != kind {
                wrong_kind.get_or_insert(entry.kind);
                continue;
            }
            // A bad copy is never read again but still counts, so no good copy is damage every time.
            let known_bad = self.checked().bad.get(&at).cloned();
            let read_now = known_bad.is_none();
            let reason = match known_bad {
                Some(reason) => Some(reason),
                None => match read(pack, entry) {
                    Ok(value) => {
                        let mut checked = self.checked();
                        checked.good.insert(at);
                        // Worth a warning only now: with no good copy the read fails, saying why.
                        for (path, reason) in unreadable {
                            checked.unreadable.entry(path).or_insert(reason);
                        }
                        return Ok(value);
                    }
                    Err(e) if is_damage(&e) => Some(self.note_damage(at, pack, &e)),
                    // Memory or I/O: the copy may be fine, so it is neither remembered nor damage.
                    Err(e) => {
                        unreadable.push((pack.path().to_owned(), e.to_string()));
                        other.get_or_insert(e);
                        None
                    }
                },
            };
            if let Some(reason) = reason {
                damage.get_or_insert_with(|| PackError::Corrupt {
                    path: pack.path().to_owned(),
                    reason,
                });
            }
            if once && read_now {
                break;
            }
        }
        // A failure that says nothing about the copy comes first: calling it damage would mislead.
        Err(match (other.or(damage), wrong_kind) {
            (Some(e), _) => StoreError::Pack(e),
            (None, Some(found)) => StoreError::WrongKind {
                id: *id,
                expected: kind,
                found,
            },
            (None, None) => StoreError::Missing(*id),
        })
    }

    fn copies(&self, id: &Id) -> impl Iterator<Item = At> + '_ {
        let more = self.more.get(id).into_iter().flatten();
        self.index.get(id).into_iter().chain(more).copied()
    }

    fn entry(&self, (p, e): At) -> Option<(&PackReader, &IndexEntry)> {
        let pack = self.packs.get(p)?;
        Some((pack, pack.entries().get(e)?))
    }

    /// Marks a copy bad and its pack damaged, and returns the problem found.
    fn note_damage(&self, at: At, pack: &PackReader, e: &PackError) -> String {
        let reason = match e {
            PackError::Corrupt { reason, .. } => reason.clone(),
            other => other.to_string(),
        };
        let mut checked = self.checked();
        checked.bad.insert(at, reason.clone());
        checked
            .damaged
            .entry(pack.path().to_owned())
            .or_insert_with(|| reason.clone());
        reason
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
                store.register(&path).unwrap();
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
        store.refresh(&packs).unwrap();
        store.refresh(&packs).unwrap();
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
        store.refresh(&packs).unwrap();
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
