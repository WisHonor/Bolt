//! What changed between two saves, file by file.

use crate::id::Id;
use crate::listing::{Entry, EntryKind, Listing};
use crate::store::{Store, StoreError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeKind {
    Added,
    Removed,
    Modified,
    /// Same path, different kind of file (say, a file became executable or a symlink).
    KindChanged,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileChange {
    pub path: String,
    pub change: ChangeKind,
    /// The new kind, or the old one for a removed file.
    pub kind: EntryKind,
    pub old_size: Option<u64>,
    pub new_size: Option<u64>,
}

/// Every file that differs between two saved folders, sorted by path. Folders are walked
/// into, not listed. `old_root` is `None` when there is nothing to compare against.
pub fn diff_saves(
    store: &Store,
    old_root: Option<Id>,
    new_root: Id,
) -> Result<Vec<FileChange>, StoreError> {
    let mut out = Vec::new();
    diff_folders(store, old_root, Some(new_root), "", &mut out)?;
    out.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(out)
}

fn listing_of(store: &Store, id: Option<Id>) -> Result<Listing, StoreError> {
    id.map_or_else(|| Ok(Listing::empty()), |id| store.listing(&id))
}

fn folder_id(e: Option<&Entry>) -> Option<Id> {
    e.filter(|e| e.kind == EntryKind::Folder).map(|e| e.id)
}

fn file(e: Option<&Entry>) -> Option<&Entry> {
    e.filter(|e| e.kind != EntryKind::Folder)
}

fn diff_folders(
    store: &Store,
    old: Option<Id>,
    new: Option<Id>,
    prefix: &str,
    out: &mut Vec<FileChange>,
) -> Result<(), StoreError> {
    if old == new {
        return Ok(());
    }
    let (old, new) = (listing_of(store, old)?, listing_of(store, new)?);
    let mut names: Vec<&str> = old
        .entries()
        .iter()
        .chain(new.entries())
        .map(|e| e.name.as_str())
        .collect();
    names.sort_unstable();
    names.dedup();
    for name in names {
        let path = format!("{prefix}{name}");
        let (o, n) = (old.get(name), new.get(name));
        let (of, nf) = (folder_id(o), folder_id(n));
        if of.is_some() || nf.is_some() {
            diff_folders(store, of, nf, &format!("{path}/"), out)?;
        }
        let (fo, fnew) = (file(o), file(n));
        let change = match (fo, fnew) {
            (Some(_), None) => ChangeKind::Removed,
            (None, Some(_)) => ChangeKind::Added,
            (Some(a), Some(b)) if a.kind != b.kind => ChangeKind::KindChanged,
            (Some(a), Some(b)) if a.id != b.id => ChangeKind::Modified,
            _ => continue,
        };
        let Some(shown) = fnew.or(fo) else { continue };
        out.push(FileChange {
            path,
            change,
            kind: shown.kind,
            old_size: fo.map(|e| e.size),
            new_size: fnew.map(|e| e.size),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;
    use crate::listing::{Entry, Listing};
    use crate::pack::{ObjectKind, PackWriter};

    fn file(name: &str, kind: EntryKind, bytes: &[u8], pack: &mut PackWriter) -> Entry {
        Entry {
            name: name.into(),
            kind,
            id: pack.add_content(bytes).unwrap(),
            size: bytes.len() as u64,
        }
    }

    fn folder(name: &str, entries: Vec<Entry>, pack: &mut PackWriter) -> Entry {
        let id = pack
            .add_record(
                ObjectKind::FolderListing,
                &Listing::new(entries).unwrap().encode(),
            )
            .unwrap();
        Entry {
            name: name.into(),
            kind: EntryKind::Folder,
            id,
            size: 0,
        }
    }

    #[test]
    fn changes_are_listed_file_by_file() {
        let dir = tempfile::tempdir().unwrap();
        let (tmp, packs) = (dir.path().join("tmp"), dir.path().join("packs"));
        std::fs::create_dir(&tmp).unwrap();
        std::fs::create_dir(&packs).unwrap();
        let mut p = PackWriter::create(&tmp).unwrap();
        let old_gone = vec![file("x", EntryKind::File, b"x", &mut p)];
        let old = vec![
            file("same", EntryKind::File, b"s", &mut p),
            file("edit", EntryKind::File, b"old", &mut p),
            file("run", EntryKind::File, b"r", &mut p),
            folder("gone", old_gone, &mut p),
            file("swap", EntryKind::File, b"f", &mut p),
        ];
        let old = folder("", old, &mut p).id;
        let new_dir = vec![file("y", EntryKind::File, b"yy", &mut p)];
        let new_swap = vec![file("z", EntryKind::File, b"z", &mut p)];
        let new = vec![
            file("same", EntryKind::File, b"s", &mut p),
            file("edit", EntryKind::File, b"new!", &mut p),
            file("run", EntryKind::Executable, b"r", &mut p),
            folder("fresh", new_dir, &mut p),
            folder("swap", new_swap, &mut p),
        ];
        let new = folder("", new, &mut p).id;
        p.finish(&packs).unwrap();
        let store = Store::open(&packs).unwrap();
        let got: Vec<(String, ChangeKind, Option<u64>, Option<u64>)> =
            diff_saves(&store, Some(old), new)
                .unwrap()
                .into_iter()
                .map(|c| (c.path, c.change, c.old_size, c.new_size))
                .collect();
        let want = [
            ("edit", ChangeKind::Modified, Some(3), Some(4)),
            ("fresh/y", ChangeKind::Added, None, Some(2)),
            ("gone/x", ChangeKind::Removed, Some(1), None),
            ("run", ChangeKind::KindChanged, Some(1), Some(1)),
            ("swap", ChangeKind::Removed, Some(1), None),
            ("swap/z", ChangeKind::Added, None, Some(1)),
        ];
        let want: Vec<_> = want
            .into_iter()
            .map(|(p, c, a, b)| (p.to_owned(), c, a, b))
            .collect();
        assert_eq!(got, want);
        assert_eq!(diff_saves(&store, None, new).unwrap().len(), 5);
        assert!(diff_saves(&store, Some(new), new).unwrap().is_empty());
    }
}
