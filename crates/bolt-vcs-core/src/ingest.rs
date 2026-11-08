//! How a file's bytes enter the store: read once, a small file whole and a big one cut into
//! chunks as it is read, each chunk hashed, checked and compressed on the workers.

use std::io::{self, Read};

use crate::chunks::{Chunk, ChunkList, Cutter, is_cut, is_too_long_to_cut};
use crate::id::{ContentHasher, Id, hash_content};
use crate::pack::{self, ObjectKind, PackError, PackSet, Prepared};
use crate::store::{Store, StoreError};
use crate::workers::{WINDOW, with_workers};

/// Why bytes couldn't be stored.
#[derive(Debug)]
pub(crate) enum IngestError {
    /// The reader gave more or fewer bytes than it was said to hold.
    Changed,
    Read(io::Error),
    Pack(PackError),
    Store(StoreError),
}

impl From<PackError> for IngestError {
    fn from(e: PackError) -> Self {
        IngestError::Pack(e)
    }
}

impl From<StoreError> for IngestError {
    fn from(e: StoreError) -> Self {
        IngestError::Store(e)
    }
}

/// Stores the `size` bytes `reader` gives, reading them once, and gives their ID. Fewer or more
/// bytes than `size` is `Changed`. A file too long to cut is its caller's to store whole.
pub(crate) fn store_bytes(
    packs: &mut PackSet,
    store: &Store,
    reader: &mut dyn Read,
    size: u64,
) -> Result<Id, IngestError> {
    if is_cut(size) {
        return cut(packs, store, reader, size);
    }
    if is_too_long_to_cut(size) {
        return Err(PackError::TooLarge.into());
    }
    let bytes = read_whole(reader, size)?;
    add_ready(packs, &ready(store, &bytes)?)
}

/// `store_bytes` for bytes already held in memory, a big file cut where it lies.
pub(crate) fn store_held(
    packs: &mut PackSet,
    store: &Store,
    bytes: &[u8],
) -> Result<Id, IngestError> {
    let size = bytes.len() as u64;
    if is_cut(size) {
        return cut(packs, store, &mut &*bytes, size);
    }
    add_ready(packs, &ready(store, bytes)?)
}

/// Whether a file of `size` bytes is read whole: neither cut nor too long to cut.
pub(crate) fn is_small(size: u64) -> bool {
    !is_cut(size) && !is_too_long_to_cut(size)
}

/// The `size` bytes `reader` gives, read whole. Fewer or more is `Changed`.
pub(crate) fn read_whole(reader: &mut dyn Read, size: u64) -> Result<Vec<u8>, IngestError> {
    let mut bytes = Vec::new();
    // One byte past `size` is enough to tell that the file grew.
    let room = usize::try_from(size.saturating_add(1)).unwrap_or(usize::MAX);
    bytes
        .try_reserve_exact(room)
        .map_err(|_| PackError::OutOfMemory)?;
    reader
        .take(size.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(IngestError::Read)?;
    if bytes.len() as u64 != size {
        return Err(IngestError::Changed);
    }
    Ok(bytes)
}

/// A small file, a link's target or a chunk, as a worker leaves it: its ID and size, and the
/// bytes a pack stores for it, unless the store holds a good copy.
pub(crate) struct Ready {
    pub(crate) id: Id,
    pub(crate) size: u64,
    pub(crate) stored: Option<Prepared>,
}

/// Hashes `bytes` and, unless the store holds a good copy, compresses them for a pack. Any
/// thread can.
pub(crate) fn ready(store: &Store, bytes: &[u8]) -> Result<Ready, IngestError> {
    let id = hash_content(bytes);
    let size = bytes.len() as u64;
    let held = store.has_good_copy(&id, ObjectKind::Content, size)?;
    let stored = if held {
        None
    } else {
        Some(pack::prepare(id, bytes)?)
    };
    Ok(Ready { id, size, stored })
}

/// Adds what `ready` made ready, unless the pack being built holds it already, and gives its ID.
/// Only the thread writing the packs does.
pub(crate) fn add_ready(packs: &mut PackSet, ready: &Ready) -> Result<Id, IngestError> {
    if let Some(stored) = &ready.stored
        && !packs.contains(&ready.id)
    {
        packs.add_prepared(ObjectKind::Content, stored)?;
    }
    Ok(ready.id)
}

/// The most a job that reads `len` bytes holds: them, a copy if they don't compress, and what
/// compressing them may make.
pub(crate) fn cost(len: u64) -> usize {
    let len = usize::try_from(len).unwrap_or(usize::MAX);
    len.saturating_mul(2)
        .saturating_add(zstd::zstd_safe::compress_bound(len))
}

/// Cuts the `size` bytes `reader` gives into chunks, storing each one the store doesn't hold,
/// then the file's chunk list unless the store holds that same list. The chunks are read here,
/// in order, and hashed, checked and compressed on the workers.
fn cut(
    packs: &mut PackSet,
    store: &Store,
    reader: &mut dyn Read,
    size: u64,
) -> Result<Id, IngestError> {
    let mut cutter = Cutter::new(reader.take(size.saturating_add(1)));
    let mut whole = ContentHasher::new();
    let (mut total, mut failed) = (0u64, false);
    let pieces = std::iter::from_fn(|| {
        if failed {
            return None;
        }
        match cutter.next_chunk() {
            Ok(Some(bytes)) => {
                whole.update(bytes);
                total = total.saturating_add(bytes.len() as u64);
                Some(Ok(bytes.to_vec()))
            }
            Ok(None) => None,
            // Nothing more is read once a read fails.
            Err(e) => {
                failed = true;
                Some(Err(IngestError::Read(e)))
            }
        }
    });
    let mut chunks = Vec::new();
    with_workers(|workers| {
        workers.in_order(
            pieces,
            WINDOW,
            |piece| piece.as_ref().map_or(0, |bytes| cost(bytes.len() as u64)),
            |piece| piece.and_then(|bytes| ready(store, &bytes)),
            |done| {
                let done = done?;
                add_ready(packs, &done)?;
                chunks.try_reserve(1).map_err(|_| PackError::OutOfMemory)?;
                chunks.push(Chunk {
                    id: done.id,
                    size: done.size,
                });
                Ok::<_, IngestError>(())
            },
        )
    })?;
    if total != size {
        return Err(IngestError::Changed);
    }
    let list = ChunkList {
        file: whole.finish(),
        size,
        chunks,
    };
    // Every chunk was just checked or stored, so a list that names the same ones is a good copy.
    // A copy stored whole before files were cut isn't read to prove the file: the list goes
    // beside it.
    if !packs.contains(&list.file) && !store.holds_chunk_list(&list)? {
        packs.add_chunk_list(&list)?;
    }
    Ok(list.file)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;
    use crate::chunks::{MAX_CHUNK, set_test_cut_sizes};
    use crate::pack::{IndexEntry, PackReader, WRITER_PACK_OBJECTS};
    use bolt_platform::dir::Dir;
    use std::ffi::OsStr;
    use std::path::PathBuf;

    /// A store's packs, and the folder its commands build packs in.
    struct Folders {
        _root: tempfile::TempDir,
        tmp: PathBuf,
        packs: PathBuf,
    }

    impl Folders {
        fn new() -> Folders {
            let root = tempfile::tempdir().unwrap();
            let (tmp, packs) = (root.path().join("tmp"), root.path().join("packs"));
            std::fs::create_dir(&tmp).unwrap();
            std::fs::create_dir(&packs).unwrap();
            Folders {
                _root: root,
                tmp,
                packs,
            }
        }

        fn store(&self) -> Store {
            Store::open(&self.packs).unwrap()
        }

        fn packs(&self) -> PackSet {
            PackSet::with_limit(&self.tmp, &self.packs, WRITER_PACK_OBJECTS).unwrap()
        }

        /// Stores `bytes` in a command of its own; their ID, and the objects it added.
        fn stored(&self, bytes: &[u8]) -> (Id, Vec<IndexEntry>) {
            let mut packs = self.packs();
            let size = bytes.len() as u64;
            let id = store_bytes(&mut packs, &self.store(), &mut &*bytes, size).unwrap();
            (id, added(packs))
        }

        fn read_back(&self, id: &Id) -> Vec<u8> {
            let mut out = Vec::new();
            self.store().copy_content(id, &mut out).unwrap();
            out
        }
    }

    /// The objects `packs` added, once they are in the store, in order of ID.
    fn added(packs: PackSet) -> Vec<IndexEntry> {
        let mut entries = Vec::new();
        for path in packs.finish().unwrap() {
            entries.extend(
                PackReader::open(&path)
                    .unwrap()
                    .entries()
                    .map(Result::unwrap),
            );
        }
        entries.sort_by_key(|e| e.id.0);
        entries
    }

    /// Bytes that look random, made from `seed`.
    fn noise(seed: u64, len: usize) -> Vec<u8> {
        let mut out = vec![0; len];
        blake3::Hasher::new()
            .update(&seed.to_le_bytes())
            .finalize_xof()
            .fill(&mut out);
        out
    }

    /// Hands out `data`, counting the bytes it gave.
    struct Counting<'a> {
        data: &'a [u8],
        given: u64,
    }

    impl Read for Counting<'_> {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            let n = self.data.read(buf)?;
            self.given += n as u64;
            Ok(n)
        }
    }

    #[test]
    fn a_new_file_is_read_once() {
        let f = Folders::new();
        for len in [100_000, 3 << 20] {
            let data = noise(len as u64, len);
            let mut packs = f.packs();
            let mut reader = Counting {
                data: &data,
                given: 0,
            };
            let id = store_bytes(&mut packs, &f.store(), &mut reader, len as u64).unwrap();
            assert_eq!((id, reader.given), (hash_content(&data), len as u64));
            assert!(added(packs).iter().any(|e| e.id == id));
            assert_eq!(f.read_back(&id), data);
        }
    }

    #[test]
    fn a_big_file_is_stored_as_chunks_and_reads_back() {
        let f = Folders::new();
        let data = noise(1, 3 << 20);
        let (id, added_first) = f.stored(&data);
        assert_eq!(id, hash_content(&data));
        // The file's chunk list under its ID, and its chunks; never the file whole.
        let (lists, chunks): (Vec<&IndexEntry>, Vec<_>) = added_first
            .iter()
            .partition(|e| e.kind == ObjectKind::ChunkList);
        assert_eq!(lists.len(), 1);
        assert_eq!(
            (lists[0].id, lists[0].uncompressed_size),
            (id, data.len() as u64)
        );
        assert!(chunks.len() > 10, "{}", chunks.len());
        assert!(chunks.iter().all(|e| e.kind == ObjectKind::Content
            && e.id != id
            && e.uncompressed_size <= MAX_CHUNK as u64));
        assert_eq!(f.read_back(&id), data);
        // Held in memory, it is cut the same.
        let g = Folders::new();
        let mut packs = g.packs();
        assert_eq!(store_held(&mut packs, &g.store(), &data).unwrap(), id);
        let named = |e: &[IndexEntry]| e.iter().map(|e| (e.id, e.kind)).collect::<Vec<_>>();
        assert_eq!(named(&added(packs)), named(&added_first));
    }

    #[test]
    fn an_edited_big_file_adds_only_its_changed_chunks() {
        let f = Folders::new();
        let mut data = noise(2, 3 << 20);
        let (_, first) = f.stored(&data);
        data[1_500_000..1_500_008].copy_from_slice(b"an edit!");
        let (id, added) = f.stored(&data);
        // The chunk the edit is in, perhaps one either side of it, and the new list.
        assert!(first.len() > 40, "{}", first.len());
        assert!((2..=4).contains(&added.len()), "{added:?}");
        assert!(
            added
                .iter()
                .any(|e| (e.id, e.kind) == (id, ObjectKind::ChunkList))
        );
        assert_eq!(f.read_back(&id), data);
    }

    #[test]
    fn the_same_list_is_not_stored_twice() {
        let f = Folders::new();
        let data = noise(3, 3 << 20);
        let (id, first) = f.stored(&data);
        // In a command of its own: every chunk, and the list, are held already.
        let (again, added_again) = f.stored(&data);
        assert_eq!(again, id);
        assert!(added_again.is_empty(), "{added_again:?}");
        // Twice in one command: once.
        let g = Folders::new();
        let mut packs = g.packs();
        for _ in 0..2 {
            assert_eq!(store_held(&mut packs, &g.store(), &data).unwrap(), id);
        }
        assert_eq!(added(packs), first);
    }

    #[test]
    fn a_whole_copy_from_before_chunking_gets_a_list_beside_it() {
        let f = Folders::new();
        let data = noise(4, 3 << 20);
        let mut packs = f.packs();
        let id = packs.add_content(&data).unwrap();
        packs.finish().unwrap();
        let (again, added) = f.stored(&data);
        assert_eq!(again, id);
        // The whole copy isn't relied on: the list and its chunks go beside it.
        let under_id: Vec<_> = added.iter().filter(|e| e.id == id).collect();
        assert_eq!(under_id.len(), 1);
        assert_eq!(under_id[0].kind, ObjectKind::ChunkList);
        assert!(added.len() > 10, "{}", added.len());
        assert_eq!(f.read_back(&id), data);
    }

    #[test]
    fn a_file_that_grows_or_shrinks_while_read_is_changed() {
        let f = Folders::new();
        for len in [1000, 3 << 20] {
            let data = noise(len as u64, len + 1);
            let (store, mut packs) = (f.store(), f.packs());
            // A byte more than it said, then a byte fewer.
            for given in [&data[..], &data[..len - 1]] {
                let stored = store_bytes(&mut packs, &store, &mut &*given, len as u64);
                assert!(
                    matches!(stored, Err(IngestError::Changed)),
                    "{len}: {stored:?}"
                );
            }
        }
    }

    #[test]
    fn a_file_over_the_limit_is_stored_whole() {
        // Files of 1 to 2 MiB are cut; a longer one is too long to cut.
        set_test_cut_sizes(Some((1 << 20, 2 << 20)));
        assert!(is_cut(2 << 20) && is_too_long_to_cut((2 << 20) + 1));
        let dir = tempfile::tempdir().unwrap();
        let top = Dir::open_top(dir.path()).unwrap();
        let f = Folders::new();
        let (store, mut packs) = (f.store(), f.packs());
        for (name, len, kind) in [
            ("cut", 3 << 19, ObjectKind::ChunkList),
            ("whole", 3 << 20, ObjectKind::Content),
        ] {
            let data = noise(len as u64, len);
            std::fs::write(dir.path().join(name), &data).unwrap();
            let seen = top.metadata(OsStr::new(name)).unwrap();
            let stored = crate::snapshot::store_file(
                &mut packs,
                &store,
                &top,
                name,
                name,
                Some(&seen),
                &mut || {},
            );
            let id = hash_content(&data);
            assert_eq!(stored.unwrap(), (id, len as u64));
            assert_eq!(packs.kind_of(&id), Some(kind), "{name}");
        }
        set_test_cut_sizes(None);
    }
}
