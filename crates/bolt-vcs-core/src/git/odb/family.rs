//! Rebuilding a big file's delta family once: the blob stored whole at the bottom of a chain and
//! every entry of its pack built on it, each built once, from the bottom up.
#![deny(clippy::indexing_slicing, clippy::arithmetic_side_effects)]

use std::collections::HashMap;
use std::ffi::OsString;
use std::fs::File;
use std::io::{self, Read, Write};

use bolt_platform::dir::Dir;

use super::super::pack::{Bases, Entry, EntryKind, Pack};
use super::super::{GitError, GitId, ObjectType, Repo};
use super::{Base, ENTRY_HEADER_WORK, MAX_DELTA_DEPTH, Place, lost};
use crate::fsio;

/// The most a delta family's objects kept in files, for more to be built on them, may hold at
/// once.
const MAX_KEPT_ON_DISK: u64 = 4 << 30;

/// What `Repo::rebuild_family` hands each object of a family to, with the object's name.
pub type Each<'a, E> = dyn FnMut(&GitId, &[u8]) -> Result<(), E> + 'a;

impl Repo {
    /// Rebuilds the delta family of the blob `id` once, if Git stored it as a delta in a pack:
    /// the blob stored whole at the bottom of its chain and every entry of that pack built on it,
    /// directly or not, each built once, from the bottom up, depth first, those built on one
    /// object in pack order. Each object that is the blob its index names is handed to `each`
    /// with that name; one that isn't, or whose delta fails, isn't, though one that was built
    /// still serves as the base of those built on it. The work counts as `read`'s does.
    ///
    /// An object with more to build on it stays in the base cache while it fits, else in a file
    /// in `tmp`, at most `MAX_KEPT_ON_DISK` bytes of them at once, else is built again from the
    /// bottom for the next built on it.
    pub fn rebuild_family<E: From<GitError>>(
        &self,
        id: &GitId,
        tmp: &Dir,
        each: &mut Each<'_, E>,
    ) -> Result<(), E> {
        let Place::Packed(n, pack, at) = self.find(id)? else {
            return Ok(());
        };
        let file = self.open_pack(n, pack)?;
        let rebuilt = self.rebuild_family_in(n, pack, &file, at, tmp, each);
        self.keep_open(n, file);
        rebuilt
    }

    fn rebuild_family_in<E: From<GitError>>(
        &self,
        n: usize,
        pack: &Pack,
        file: &File,
        at: u64,
        tmp: &Dir,
        each: &mut Each<'_, E>,
    ) -> Result<(), E> {
        // Only a delta has a family, so only then is the pack scanned.
        match pack.entry(file, at) {
            Ok(Entry {
                kind: EntryKind::OfsDelta(_) | EntryKind::RefDelta(_),
                ..
            }) => {}
            Ok(_) | Err(GitError::Broken { .. }) => return Ok(()),
            Err(e) => return Err(e.into()),
        }
        let bases = pack.bases(file)?;
        let Some((bottom, entry)) = self.family_bottom(n, pack, file, bases, at)? else {
            return Ok(());
        };
        let mut family = Family {
            repo: self,
            n,
            pack,
            file,
            bases,
            tmp,
            open: Vec::new(),
            last: None,
            kept: HashMap::new(),
            in_cache: 0,
            on_disk: 0,
        };
        family.rebuild(bottom, &entry, each)
    }

    /// The number and entry of the blob at the bottom of the chain from the delta at `at` in
    /// pack number `n`, if it has a family to rebuild that wasn't rebuilt yet; from here on it
    /// counts as rebuilt. Anything else is left to be read as any object is, which says why it
    /// can't be.
    fn family_bottom(
        &self,
        n: usize,
        pack: &Pack,
        file: &File,
        bases: &Bases,
        at: u64,
    ) -> Result<Option<(usize, Entry)>, GitError> {
        let Some(mut bottom) = pack.number(at) else {
            return Ok(None);
        };
        // Through no more deltas than an object may be read through: a loop never gets past.
        let mut deltas = 0;
        while let Some(base) = bases.base(bottom) {
            if deltas == MAX_DELTA_DEPTH {
                return Ok(None);
            }
            deltas = deltas.saturating_add(1);
            bottom = base;
        }
        // Stored whole, or a damaged entry, or a delta on what this pack doesn't hold.
        let (bottom_at, _) = pack.listed(bottom).ok_or_else(|| lost(pack))?;
        let entry = match pack.entry(file, bottom_at) {
            Ok(entry) => entry,
            Err(GitError::Broken { .. }) => return Ok(None),
            Err(e) => return Err(e),
        };
        if !matches!(entry.kind, EntryKind::Whole(ObjectType::Blob)) {
            return Ok(None);
        }
        let mut families = self
            .objects
            .families
            .try_borrow_mut()
            .map_err(|_| lost(pack))?;
        Ok(families.insert((n, bottom)).then_some((bottom, entry)))
    }
}

/// A delta family being rebuilt from pack number `n`.
struct Family<'a> {
    repo: &'a Repo,
    n: usize,
    pack: &'a Pack,
    file: &'a File,
    bases: &'a Bases,
    tmp: &'a Dir,
    /// The objects with more to build on them, outermost first.
    open: Vec<Open>,
    /// The object built last, by number, while the next is built on it.
    last: Option<(usize, Vec<u8>)>,
    /// Where each open object is kept for the objects built on it after the first.
    kept: HashMap<usize, Kept<'a>>,
    /// Bytes of the family the base cache holds, and files in `tmp`.
    in_cache: u64,
    on_disk: u64,
}

/// An object of a family with more to build on it.
#[derive(Clone, Copy)]
struct Open {
    number: usize,
    /// How many deltas deep it is.
    depth: u32,
    /// How many of the objects built on it are built.
    done: usize,
}

/// Where a family's object is kept for more to be built on it.
enum Kept<'a> {
    /// In the base cache, under its offset; its length.
    Cache(u64, u64),
    File(Spill<'a>),
}

impl Family<'_> {
    fn rebuild<E: From<GitError>>(
        &mut self,
        bottom: usize,
        entry: &Entry,
        each: &mut Each<'_, E>,
    ) -> Result<(), E> {
        let (at, _) = self.listed(bottom)?;
        self.repo.spend(entry.size)?;
        let data = self.repo.inflate_entry(self.pack, self.file, at, entry)?;
        self.built(bottom, 0, data, each)?;
        while let Some(&Open {
            number: base,
            depth,
            done,
        }) = self.open.last()
        {
            let built_on = self.bases.built_on(base);
            let Some(&number) = built_on.get(done) else {
                self.open.pop();
                self.release(base);
                continue;
            };
            let done = done.saturating_add(1);
            let last_one = done >= built_on.len();
            match self.open.last_mut() {
                Some(open) if !last_one => open.done = done,
                _ => {
                    self.open.pop();
                }
            }
            let data = self.build(number as usize, base)?;
            if last_one {
                self.release(base);
            }
            if let Some(data) = data {
                self.built(number as usize, depth.saturating_add(1), data, each)?;
            }
        }
        Ok(())
    }

    /// Hands the object `number`, just built, to `each` if it is the blob its index names, then
    /// holds it for what is built on it.
    fn built<E: From<GitError>>(
        &mut self,
        number: usize,
        depth: u32,
        data: Vec<u8>,
        each: &mut Each<'_, E>,
    ) -> Result<(), E> {
        let (at, name) = self.listed(number)?;
        if self.repo.check(&name, ObjectType::Blob, &data).is_ok() {
            each(&name, &data)?;
        }
        // Nothing is built deeper than an object may be read through.
        let more = if depth < MAX_DELTA_DEPTH {
            self.bases.built_on(number).len()
        } else {
            0
        };
        if more == 0 {
            return Ok(());
        }
        self.last = match more {
            1 => Some((number, data)),
            _ => self
                .keep(number, at, depth, data)
                .map(|data| (number, data)),
        };
        self.open.push(Open {
            number,
            depth,
            done: 0,
        });
        Ok(())
    }

    /// The object `number` built on the object `base`, or `None` if its delta is damaged.
    fn build(&mut self, number: usize, base: usize) -> Result<Option<Vec<u8>>, GitError> {
        let (repo, pack, file) = (self.repo, self.pack, self.file);
        let (at, _) = self.listed(number)?;
        // Anything else held as the object built last is let go first.
        let last = self.last.take().filter(|(last, _)| *last == base);
        repo.spend(ENTRY_HEADER_WORK)?;
        let built = pack.entry(file, at).and_then(|entry| match last {
            Some((_, data)) => repo.apply(pack, file, at, &entry, &data),
            None => self.build_on_kept(at, &entry, base),
        });
        match built {
            Ok(data) => Ok(Some(data)),
            Err(GitError::Broken { .. } | GitError::EndedEarly { .. }) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// What the delta `entry` at `at` builds on the object `base`, kept since it was built, or
    /// built again from the bottom if it wasn't kept or was lost from the cache.
    fn build_on_kept(&self, at: u64, entry: &Entry, base: usize) -> Result<Vec<u8>, GitError> {
        let (repo, pack, file) = (self.repo, self.pack, self.file);
        match self.kept.get(&base) {
            Some(Kept::Cache(base_at, _)) => {
                if let Ok(mut cache) = repo.objects.cache.try_borrow_mut()
                    && let Some(kept) = cache.get((self.n, *base_at))
                {
                    return repo.apply(pack, file, at, entry, &kept.data);
                }
            }
            Some(Kept::File(spill)) => {
                if let Ok(data) = spill.read() {
                    return repo.apply(pack, file, at, entry, &data);
                }
            }
            None => {}
        }
        let (base_at, _) = self.listed(base)?;
        let base = repo.build(self.n, pack, file, base_at, false)?;
        repo.apply(pack, file, at, entry, &base.data)
    }

    /// Keeps the object `number` at `at` for the objects built on it after the first: in the
    /// base cache while the family's share of it has room, else in a file while there is room
    /// for that. Gives the bytes back unless the cache took them.
    fn keep(&mut self, number: usize, at: u64, depth: u32, data: Vec<u8>) -> Option<Vec<u8>> {
        let len = data.len() as u64;
        if let Ok(mut cache) = self.repo.objects.cache.try_borrow_mut()
            && self.in_cache.saturating_add(len) <= cache.budget
        {
            let kind = ObjectType::Blob;
            cache.put((self.n, at), Base { kind, data, depth });
            self.in_cache = self.in_cache.saturating_add(len);
            self.kept.insert(number, Kept::Cache(at, len));
            return None;
        }
        // A file that can't be written only costs building it again.
        if self.on_disk.saturating_add(len) <= MAX_KEPT_ON_DISK
            && let Ok(spill) = Spill::write(self.tmp, &data)
        {
            self.on_disk = self.on_disk.saturating_add(len);
            self.kept.insert(number, Kept::File(spill));
        }
        Some(data)
    }

    /// Lets go of the object `number` once the last object built on it is built.
    fn release(&mut self, number: usize) {
        match self.kept.remove(&number) {
            Some(Kept::Cache(at, len)) => {
                if let Ok(mut cache) = self.repo.objects.cache.try_borrow_mut() {
                    cache.take((self.n, at));
                }
                self.in_cache = self.in_cache.saturating_sub(len);
            }
            Some(Kept::File(spill)) => self.on_disk = self.on_disk.saturating_sub(spill.len),
            None => {}
        }
    }

    /// Where entry `number` is, and its name.
    fn listed(&self, number: usize) -> Result<(u64, GitId), GitError> {
        self.pack.listed(number).ok_or_else(|| lost(self.pack))
    }
}

/// An object kept in a file in `.bolt/tmp`, read back whole for each object built on it, and
/// removed once let go. The file is open only while it is written or read, so however deep a
/// family branches, it holds no more files open.
struct Spill<'a> {
    tmp: &'a Dir,
    name: OsString,
    len: u64,
}

impl<'a> Spill<'a> {
    fn write(tmp: &'a Dir, data: &[u8]) -> io::Result<Spill<'a>> {
        let (mut file, name) = fsio::create_temp(tmp)?;
        let written = file.write_all(data);
        // Closed before a failed one is removed: Windows doesn't remove a file that is open.
        drop(file);
        let len = data.len() as u64;
        let spill = Spill { tmp, name, len };
        written.map(|()| spill)
    }

    fn read(&self) -> io::Result<Vec<u8>> {
        let file = self.tmp.open_file(&self.name)?;
        let mut data = Vec::new();
        data.try_reserve_exact(usize::try_from(self.len).unwrap_or(usize::MAX))
            .map_err(|_| io::ErrorKind::OutOfMemory)?;
        file.take(self.len).read_to_end(&mut data)?;
        if data.len() as u64 != self.len {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        Ok(data)
    }
}

impl Drop for Spill<'_> {
    fn drop(&mut self) {
        let _ = fsio::remove_file(self.tmp, &self.name);
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    #![allow(clippy::indexing_slicing, clippy::arithmetic_side_effects)]
    use super::super::BaseCache;
    use super::super::tests::{broken, craft, id, open, place, read_all};
    use super::*;
    use crate::git::HashKind::Sha1;
    use crate::git::craft::{Craft, PackEntry, delta, entry_header, zlib};
    use crate::git::object_id;
    use std::fs;

    use ObjectType::{Blob, Tree};

    /// Packs a family of blobs: one stored whole, one built on it, two built on that (the second
    /// through a REF_DELTA) and one built on the last; among them another family of two blobs,
    /// and one of two trees. Returns the first family's blobs, in the order a rebuild hands them
    /// over, and the second tree.
    fn pack_with_a_family(git: &Craft) -> (Vec<Vec<u8>>, Vec<u8>) {
        let a: Vec<u8> = (0..20_000u32).flat_map(|i| i.to_le_bytes()).collect();
        let on = |base: &[u8], tail: &str| [base, tail.as_bytes()].concat();
        let b = on(&a, "b");
        let (c, d) = (on(&b, "c"), on(&b, "d"));
        let e = on(&d, "e");
        let other = b"another file\n".to_vec();
        let tree = |name: &[u8], data: &[u8]| {
            [
                b"100644 ",
                name,
                b"\0",
                id(Blob, data, git.hash()).as_bytes(),
            ]
            .concat()
        };
        let (first_tree, second_tree) = (tree(b"a", &a), tree(b"b", &b));
        git.pack(&[
            PackEntry::Whole(Blob, a.clone()),
            PackEntry::OfsDelta {
                base: 0,
                delta: delta(&a, &b),
            },
            PackEntry::Whole(Blob, other.clone()),
            PackEntry::OfsDelta {
                base: 1,
                delta: delta(&b, &c),
            },
            PackEntry::RefDelta {
                base: id(Blob, &b, git.hash()),
                delta: delta(&b, &d),
            },
            PackEntry::OfsDelta {
                base: 2,
                delta: delta(&other, &on(&other, "changed")),
            },
            PackEntry::OfsDelta {
                base: 4,
                delta: delta(&d, &e),
            },
            PackEntry::Whole(Tree, first_tree.clone()),
            PackEntry::OfsDelta {
                base: 7,
                delta: delta(&first_tree, &second_tree),
            },
        ]);
        (vec![a, b, c, d, e], second_tree)
    }

    /// The work rebuilding `pack_with_a_family`'s family takes: the bottom, then for each object
    /// built, its entry header, its delta and what it builds.
    fn family_work(family: &[Vec<u8>]) -> u64 {
        let mut work = family[0].len() as u64;
        for (base, built) in [(0, 1), (1, 2), (1, 3), (3, 4)] {
            let (base, built) = (&family[base], &family[built]);
            work += ENTRY_HEADER_WORK + delta(base, built).len() as u64 + built.len() as u64;
        }
        work
    }

    fn tmp_folder() -> (tempfile::TempDir, Dir) {
        let tmp = tempfile::tempdir().unwrap();
        let dir = Dir::open_top(tmp.path()).unwrap();
        (tmp, dir)
    }

    /// The names `rebuild_family` hands over, asked for `id`, each checked against its bytes.
    fn rebuilt(repo: &Repo, id: &GitId, tmp: &Dir) -> Vec<GitId> {
        let mut handed = Vec::new();
        let mut hand = |name: &GitId, data: &[u8]| {
            assert_eq!(object_id(Blob, data, Sha1).unwrap(), *name);
            handed.push(*name);
            Ok::<_, GitError>(())
        };
        repo.rebuild_family(id, tmp, &mut hand).unwrap();
        handed
    }

    #[test]
    fn each_object_of_a_family_is_built_once() {
        let (dir, git) = craft(Sha1);
        let (family, tree) = pack_with_a_family(&git);
        let ids: Vec<GitId> = family.iter().map(|data| id(Blob, data, Sha1)).collect();
        let (_tmp, tmp) = tmp_folder();
        let repo = open(&dir);
        // From the bottom up, depth first, those built on one object in pack order.
        assert_eq!(rebuilt(&repo, &ids[4], &tmp), ids);
        assert_eq!(repo.objects.built.get(), 4);
        assert_eq!(repo.objects.work.get(), family_work(&family));
        // A family is rebuilt once, whichever member asks again; an object stored whole has no
        // family, and one of trees isn't rebuilt.
        for asked in [ids[2], ids[0], id(Tree, &tree, Sha1)] {
            assert!(rebuilt(&repo, &asked, &tmp).is_empty());
        }
        assert_eq!(repo.objects.built.get(), 4);
        // Its objects still read as any other.
        for (id, data) in ids.iter().zip(&family) {
            assert_eq!(&repo.read(id).unwrap().data, data);
        }
    }

    #[test]
    fn a_family_with_branches_keeps_memory_bounded() {
        let (dir, git) = craft(Sha1);
        let (family, _) = pack_with_a_family(&git);
        let ids: Vec<GitId> = family.iter().map(|data| id(Blob, data, Sha1)).collect();
        let (folder, tmp) = tmp_folder();
        let files = || fs::read_dir(folder.path()).unwrap().count();
        // The second blob has two built on it: while the first of them is built, it waits in
        // the cache, and leaves it once the second is.
        let repo = open(&dir);
        let second = place(&repo, &ids[1]);
        let mut seen = Vec::new();
        let mut hand = |name: &GitId, _: &[u8]| {
            let cached = repo.objects.cache.borrow().depth(second).is_some();
            seen.push((*name, cached, files()));
            Ok::<_, GitError>(())
        };
        repo.rebuild_family(&ids[4], &tmp, &mut hand).unwrap();
        let cached = [false, false, true, false, false];
        let want: Vec<_> = ids.iter().zip(cached).map(|(id, c)| (*id, c, 0)).collect();
        assert_eq!(seen, want);
        // Too big for the cache, it waits in a file in tmp instead, read back for the second:
        // each object is still built once.
        let repo = open(&dir);
        *repo.objects.cache.borrow_mut() = BaseCache::new(family[1].len() as u64 - 1);
        let mut seen = Vec::new();
        let mut hand = |name: &GitId, _: &[u8]| {
            seen.push((*name, files()));
            Ok::<_, GitError>(())
        };
        repo.rebuild_family(&ids[4], &tmp, &mut hand).unwrap();
        let in_files = [0, 0, 1, 0, 0];
        let want: Vec<_> = ids.iter().zip(in_files).map(|(id, f)| (*id, f)).collect();
        assert_eq!(seen, want);
        assert_eq!(repo.objects.built.get(), 4);
        assert_eq!(repo.objects.work.get(), family_work(&family));
        assert_eq!(files(), 0);
        // Lost from the cache before its second is built, it is built again from the bottom,
        // and that counts.
        let repo = open(&dir);
        let mut seen = Vec::new();
        let mut hand = |name: &GitId, _: &[u8]| {
            if *name == ids[2] {
                *repo.objects.cache.borrow_mut() = BaseCache::default();
            }
            seen.push(*name);
            Ok::<_, GitError>(())
        };
        repo.rebuild_family(&ids[4], &tmp, &mut hand).unwrap();
        assert_eq!(seen, ids);
        assert_eq!(repo.objects.built.get(), 5);
        let again = ENTRY_HEADER_WORK
            + (family[0].len() + delta(&family[0], &family[1]).len() + family[1].len()) as u64;
        assert_eq!(repo.objects.work.get(), family_work(&family) + again);
    }

    #[test]
    fn the_family_scan_runs_only_for_a_pack_it_needs() {
        let (dir, git) = craft(Sha1);
        let (family, _) = pack_with_a_family(&git);
        // Beside it, a pack of 5,000 objects, half of them deltas.
        let mut entries = Vec::new();
        for i in 0..2500 {
            let whole = format!("file {i}\n").into_bytes();
            let changed = format!("file {i}, changed\n").into_bytes();
            entries.push(PackEntry::Whole(Blob, whole.clone()));
            entries.push(PackEntry::OfsDelta {
                base: 2 * i,
                delta: delta(&whole, &changed),
            });
        }
        git.pack(&entries);
        let repo = open(&dir);
        let scanned = || {
            let folder = repo.packs().unwrap();
            folder.packs.iter().map(Pack::scanned).collect::<Vec<_>>()
        };
        // Reading objects, whole, through deltas or streamed, scans no pack.
        let last = id(Blob, &family[4], Sha1);
        repo.read(&last).unwrap();
        repo.header(&id(Blob, &family[3], Sha1)).unwrap();
        read_all(repo.blob_reader(&id(Blob, &family[0], Sha1)).unwrap(), 4096).unwrap();
        repo.read(&id(Blob, b"file 7, changed\n", Sha1)).unwrap();
        assert_eq!(scanned(), [None, None]);
        // Nor does asking for the family of an object stored whole, which has none.
        let (_tmp, tmp) = tmp_folder();
        for whole in [&family[0][..], b"file 7\n"] {
            assert!(rebuilt(&repo, &id(Blob, whole, Sha1), &tmp).is_empty());
        }
        assert_eq!(scanned(), [None, None]);
        // Rebuilding a family scans its own pack only, keeping at most 8 bytes an entry.
        let mut hand = |_: &GitId, _: &[u8]| Ok::<_, GitError>(());
        repo.rebuild_family(&last, &tmp, &mut hand).unwrap();
        let (n, _) = place(&repo, &last);
        let scans = scanned();
        assert_eq!(scans[1 - n], None);
        assert!(scans[n].is_some_and(|bytes| bytes <= 8 * 9), "{scans:?}");
    }

    #[test]
    fn a_ref_delta_loop_has_no_family() {
        let (dir, git) = craft(Sha1);
        // Two REF_DELTAs, each on the other.
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
        let (_tmp, tmp) = tmp_folder();
        for asked in [a, b] {
            assert!(rebuilt(&repo, &asked, &tmp).is_empty());
        }
        assert_eq!(repo.objects.work.get(), 0);
        for asked in [a, b] {
            assert!(broken(repo.read(&asked)).contains("loop"));
        }
    }

    #[test]
    fn a_damaged_entry_header_ends_a_family_there() {
        let (dir, git) = craft(Sha1);
        // A blob, one built on it, an entry of a type Git doesn't use, and a delta on that.
        let first: Vec<u8> = (0..20_000u32).flat_map(|i| i.to_le_bytes()).collect();
        let second = [&first[..], b"second"].concat();
        let damaged = [entry_header(5, 3), zlib(b"abc")].concat();
        let on_damaged = delta(b"abc", b"abcd");
        git.pack(&[
            PackEntry::Whole(Blob, first.clone()),
            PackEntry::OfsDelta {
                base: 0,
                delta: delta(&first, &second),
            },
            PackEntry::Raw(damaged),
            PackEntry::OfsDelta {
                base: 2,
                delta: on_damaged.clone(),
            },
        ]);
        let repo = open(&dir);
        let (_tmp, tmp) = tmp_folder();
        // Built on the damaged entry, it has no family...
        let on_it = id(Blob, &on_damaged, Sha1);
        assert!(rebuilt(&repo, &on_it, &tmp).is_empty());
        assert_eq!(repo.objects.work.get(), 0);
        assert!(broken(repo.read(&on_it)).contains("type 5"));
        // ...and the pack's other family is rebuilt as ever.
        let family = [&first, &second].map(|data| id(Blob, data, Sha1));
        assert_eq!(rebuilt(&repo, &family[1], &tmp), family);
        assert_eq!(repo.read(&family[1]).unwrap().data, second);
    }

    #[test]
    fn a_ref_delta_on_another_pack_has_no_family() {
        let (dir, git) = craft(Sha1);
        let base = b"in the first pack".to_vec();
        git.pack(&[PackEntry::Whole(Blob, base.clone())]);
        let on_it = delta(&base, b"in the second pack, built on the first");
        git.pack(&[PackEntry::RefDelta {
            base: id(Blob, &base, Sha1),
            delta: on_it.clone(),
        }]);
        let repo = open(&dir);
        let (_tmp, tmp) = tmp_folder();
        // Listed under a blob of its delta, as a pack Git didn't write can list it.
        let asked = id(Blob, &on_it, Sha1);
        assert!(rebuilt(&repo, &asked, &tmp).is_empty());
        assert_eq!(repo.objects.work.get(), 0);
        assert!(broken(repo.read(&asked)).contains("isn't in that pack"));
    }

    #[test]
    fn a_family_deeper_than_an_object_may_be_read_through_stops_there() {
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
        let ids: Vec<GitId> = versions.iter().map(|v| id(Blob, v, Sha1)).collect();
        let repo = open(&dir);
        let (_tmp, tmp) = tmp_folder();
        // Past 4,096 deltas: no family.
        assert!(rebuilt(&repo, &ids[4097], &tmp).is_empty());
        assert_eq!(repo.objects.work.get(), 0);
        // At 4,096: the family, without what is built deeper.
        assert_eq!(rebuilt(&repo, &ids[4096], &tmp), ids[..=4096]);
        assert!(broken(repo.read(&ids[4097])).contains("more than 4096 deltas deep"));
        assert_eq!(repo.read(&ids[4096]).unwrap().data, versions[4096]);
    }
}
