//! One view over every pack in the store. The lookup index lives in memory and is rebuilt on open.
use crate::encoding::DecodeError;
use crate::id::Id;
use crate::listing::Listing;
use crate::pack::{ObjectKind, PackError, PackReader};
use crate::save::Save;
use std::collections::HashMap;
use std::io::Write;
use std::path::Path;

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
}

pub struct Store {
    packs: Vec<PackReader>,
    /// Id to (pack number, entry number).
    index: HashMap<Id, (usize, usize)>,
}

impl Store {
    /// Opens every `*.pack` file in the folder and ignores everything else.
    pub fn open(packs_dir: &Path) -> Result<Store, StoreError> {
        let mut paths = Vec::new();
        for entry in std::fs::read_dir(packs_dir).map_err(PackError::from)? {
            let path = entry.map_err(PackError::from)?.path();
            if path.extension().is_some_and(|e| e == "pack") {
                paths.push(path);
            }
        }
        paths.sort();
        let mut store = Store {
            packs: Vec::new(),
            index: HashMap::new(),
        };
        for path in paths {
            store.register(&path)?;
        }
        Ok(store)
    }

    /// Adds a finished pack. Objects already known keep pointing at the earlier pack.
    pub fn register(&mut self, pack_path: &Path) -> Result<(), StoreError> {
        let reader = PackReader::open(pack_path)?;
        let n = self.packs.len();
        for (i, e) in reader.entries().iter().enumerate() {
            self.index.entry(e.id).or_insert((n, i));
        }
        self.packs.push(reader);
        Ok(())
    }

    #[must_use]
    pub fn pack_count(&self) -> usize {
        self.packs.len()
    }

    #[must_use]
    pub fn contains(&self, id: &Id) -> bool {
        self.index.contains_key(id)
    }

    #[must_use]
    pub fn kind(&self, id: &Id) -> Option<ObjectKind> {
        self.locate(id).ok().map(|(_, e)| e.kind)
    }

    fn locate(&self, id: &Id) -> Result<(&PackReader, crate::pack::IndexEntry), StoreError> {
        let &(p, e) = self.index.get(id).ok_or(StoreError::Missing(*id))?;
        let pack = self.packs.get(p).ok_or(StoreError::Missing(*id))?;
        let entry = *pack.entries().get(e).ok_or(StoreError::Missing(*id))?;
        Ok((pack, entry))
    }

    fn locate_kind(
        &self,
        id: &Id,
        expected: ObjectKind,
    ) -> Result<(&PackReader, crate::pack::IndexEntry), StoreError> {
        let (pack, entry) = self.locate(id)?;
        if entry.kind != expected {
            return Err(StoreError::WrongKind {
                id: *id,
                expected,
                found: entry.kind,
            });
        }
        Ok((pack, entry))
    }

    /// Reads a structured record of the given kind, fingerprint checked.
    pub fn read_record(&self, id: &Id, kind: ObjectKind) -> Result<Vec<u8>, StoreError> {
        let (pack, entry) = self.locate_kind(id, kind)?;
        Ok(pack.read_record(&entry)?)
    }

    /// Streams content into `out`; treat the output as untrusted until this returns `Ok`.
    pub fn copy_content(&self, id: &Id, out: &mut dyn Write) -> Result<(), StoreError> {
        let (pack, entry) = self.locate_kind(id, ObjectKind::Content)?;
        Ok(pack.copy_content(&entry, out)?)
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
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;
    use crate::id::hash_content;
    use crate::pack::PackWriter;
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
                prop_assert!(fresh.contains(&id));
                prop_assert_eq!(fresh.kind(&id), Some(ObjectKind::Content));
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
    fn missing_object_is_an_error() {
        let (_root, _tmp, packs) = dirs();
        let store = Store::open(&packs).unwrap();
        let id = hash_content(b"nope");
        assert!(!store.contains(&id));
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
