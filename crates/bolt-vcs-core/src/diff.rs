//! What changed between two saves, file by file.

use crate::id::Id;
use crate::listing::{Entry, EntryKind, Listing, MAX_SAVE_DEPTH, MAX_SAVE_ENTRIES, SaveLimits};
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
    /// The kind before the change, if the path had a file.
    pub old_kind: Option<EntryKind>,
    pub old_size: Option<u64>,
    pub new_size: Option<u64>,
}

/// Every file that differs between two saved folders, sorted by path. Folders are walked
/// into, not listed. `old_root` is `None` when there is nothing to compare against.
///
/// Folders the two saves share are not read. Each save is held to the limits of
/// storage-format.md section 3 on its own (`MAX_SAVE_DEPTH` levels, `MAX_SAVE_ENTRIES` entries
/// counted along every path); passing one gives `StoreError::SaveTooLarge`. The folders that
/// differ are walked twice, the first time only to check the limits.
pub fn diff_saves(
    store: &Store,
    old_root: Option<Id>,
    new_root: Id,
) -> Result<Vec<FileChange>, StoreError> {
    diff_saves_with_limit(store, old_root, new_root, MAX_SAVE_ENTRIES)
}

pub(crate) fn diff_saves_with_limit(
    store: &Store,
    old_root: Option<Id>,
    new_root: Id,
    max_entries: u64,
) -> Result<Vec<FileChange>, StoreError> {
    // The first walk only counts. Without it, the 3.6 KB tree bomb of RT-10 would have the
    // changes of 10 million files in memory (over 1 GiB) before the entry limit refused it.
    walk_diff(store, old_root, new_root, max_entries, None)?;
    let mut out = Vec::new();
    walk_diff(store, old_root, new_root, max_entries, Some(&mut out))?;
    out.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(out)
}

/// Compares the saves, adding what differs to `out` if there is one, and holds them to the
/// limits either way.
fn walk_diff(
    store: &Store,
    old_root: Option<Id>,
    new_root: Id,
    max_entries: u64,
    mut out: Option<&mut Vec<FileChange>>,
) -> Result<(), StoreError> {
    let too_large = |detail| StoreError::SaveTooLarge { detail };
    // Each save is held to the limits on its own, so two saves that pass can always be compared.
    let (mut old_seen, mut new_seen) = (
        SaveLimits::new(MAX_SAVE_DEPTH, max_entries),
        SaveLimits::new(MAX_SAVE_DEPTH, max_entries),
    );
    // The pairs of folders still to compare, each with its name and how many levels down it is.
    // They wait here instead of in a call per level, so a save nested thousands of levels deep
    // cannot overflow the stack (RT-6). The roots go first.
    let mut todo = vec![(old_root, Some(new_root), String::new(), 0)];
    // The path of the folder being compared (`/` after every name), and `ends[k]`: how long the
    // path was k levels down. A waiting pair carries only its own name, so a folder with many
    // subfolders does not copy its path once for each of them.
    let mut path = String::new();
    let mut ends: Vec<usize> = Vec::new();
    while let Some((old, new, name, depth)) = todo.pop() {
        if old == new {
            continue;
        }
        old_seen.check_depth(depth).map_err(too_large)?; // the new save is as deep
        // The folder compared last may be deeper than this one: back out of its folders first.
        ends.truncate(depth);
        path.truncate(ends.last().copied().unwrap_or(0));
        if depth > 0 {
            path.push_str(&name);
            path.push('/');
        }
        ends.push(path.len());
        let (old, new) = (listing_of(store, old)?, listing_of(store, new)?);
        old_seen.add(old.entries().len()).map_err(too_large)?;
        new_seen.add(new.entries().len()).map_err(too_large)?;
        let mut names: Vec<&str> = old
            .entries()
            .iter()
            .chain(new.entries())
            .map(|e| e.name.as_str())
            .collect();
        names.sort_unstable();
        names.dedup();
        for name in names {
            let (o, n) = (old.get(name), new.get(name));
            let (of, nf) = (folder_id(o), folder_id(n));
            if of.is_some() || nf.is_some() {
                todo.push((of, nf, name.to_owned(), depth + 1));
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
            if let Some(out) = out.as_deref_mut() {
                out.push(FileChange {
                    path: format!("{path}{name}"),
                    change,
                    kind: shown.kind,
                    old_kind: fo.map(|e| e.kind),
                    old_size: fo.map(|e| e.size),
                    new_size: fnew.map(|e| e.size),
                });
            }
        }
    }
    Ok(())
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

    /// A store holding what `build` adds to one pack, and the IDs it hands back.
    fn store_with<const N: usize>(
        build: impl FnOnce(&mut PackWriter) -> [Id; N],
    ) -> (tempfile::TempDir, Store, [Id; N]) {
        let dir = tempfile::tempdir().unwrap();
        let (tmp, packs) = (dir.path().join("tmp"), dir.path().join("packs"));
        std::fs::create_dir(&tmp).unwrap();
        std::fs::create_dir(&packs).unwrap();
        let mut p = PackWriter::create(&tmp).unwrap();
        let ids = build(&mut p);
        p.finish(&packs).unwrap();
        (dir, Store::open(&packs).unwrap(), ids)
    }

    /// `levels` nested folders, all named `d`, with one file `f` at the bottom.
    fn chain(levels: usize, pack: &mut PackWriter) -> Id {
        let mut inner = vec![file("f", EntryKind::File, b"x", pack)];
        for _ in 0..levels {
            inner = vec![folder("d", inner, pack)];
        }
        folder("", inner, pack).id
    }

    /// A top folder whose subfolders `a`, `b` and `c` are one listing, holding `x` and `y`.
    fn three_paths_to_one_listing(y: &[u8], pack: &mut PackWriter) -> Id {
        let both = vec![
            file("x", EntryKind::File, b"x", pack),
            file("y", EntryKind::File, y, pack),
        ];
        let shared = folder("a", both, pack);
        let top = ["a", "b", "c"].map(|name| Entry {
            name: name.into(),
            ..shared.clone()
        });
        folder("", top.to_vec(), pack).id
    }

    #[test]
    fn paths_stay_right_when_the_walk_backs_out_of_deep_folders() {
        let (_dir, store, [old, new]) = store_with(|p| {
            let c = folder("c", vec![file("f", EntryKind::File, b"1", p)], p);
            let b = folder("b", vec![c, file("g", EntryKind::File, b"g", p)], p);
            let i = folder("i", vec![file("j", EntryKind::File, b"j", p)], p);
            let h = folder("h", vec![i], p);
            let a = folder("a", vec![b, h], p);
            let old = folder("", vec![a, file("z", EntryKind::File, b"z", p)], p).id;

            let c = folder("c", vec![file("f", EntryKind::File, b"2", p)], p);
            let b = folder("b", vec![c], p);
            let both = vec![
                file("j", EntryKind::File, b"j", p),
                file("k", EntryKind::File, b"k", p),
            ];
            let h = folder("h", vec![folder("i", both, p)], p);
            let n = folder("n", vec![file("o", EntryKind::File, b"o", p)], p);
            let m = folder("m", vec![n], p);
            let a = folder("a", vec![b, h, m], p);
            let new = folder("", vec![a, file("z", EntryKind::File, b"zz", p)], p).id;
            [old, new]
        });
        let got: Vec<(String, ChangeKind)> = diff_saves(&store, Some(old), new)
            .unwrap()
            .into_iter()
            .map(|c| (c.path, c.change))
            .collect();
        let want = [
            ("a/b/c/f", ChangeKind::Modified),
            ("a/b/g", ChangeKind::Removed),
            ("a/h/i/k", ChangeKind::Added),
            ("a/m/n/o", ChangeKind::Added),
            ("z", ChangeKind::Modified),
        ];
        let want: Vec<_> = want.into_iter().map(|(p, c)| (p.to_owned(), c)).collect();
        assert_eq!(got, want);
    }

    #[test]
    fn depth_at_the_limit_is_accepted() {
        let (_dir, store, [root]) = store_with(|p| [chain(MAX_SAVE_DEPTH, p)]);
        let got = diff_saves(&store, None, root).unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].path, format!("{}f", "d/".repeat(MAX_SAVE_DEPTH)));
    }

    #[test]
    fn depth_past_the_limit_is_refused() {
        let (_dir, store, [root]) = store_with(|p| [chain(MAX_SAVE_DEPTH + 1, p)]);
        let err = diff_saves(&store, None, root).unwrap_err();
        assert!(matches!(err, StoreError::SaveTooLarge { .. }), "{err:?}");
    }

    #[test]
    fn entry_limit_counts_every_path() {
        // The top listing has 3 entries. Its three folders are one listing of 2 files, which the
        // walk reads once for each path to it: 3 + 3 * 2 = 9 entries, though only 5 are stored.
        const PATHS: u64 = 3 + 3 * 2;
        let (_dir, store, [root]) = store_with(|p| [three_paths_to_one_listing(b"y", p)]);
        let got = diff_saves_with_limit(&store, None, root, PATHS).unwrap();
        assert_eq!(got.len(), 6);
        let err = diff_saves_with_limit(&store, None, root, PATHS - 1).unwrap_err();
        assert!(matches!(err, StoreError::SaveTooLarge { .. }), "{err:?}");
    }

    #[test]
    fn each_save_is_counted_on_its_own() {
        let (_dir, store, [old, new, small]) = store_with(|p| {
            [
                three_paths_to_one_listing(b"y", p),
                three_paths_to_one_listing(b"z", p),
                folder("", vec![file("x", EntryKind::File, b"x", p)], p).id,
            ]
        });
        // Two saves of 9 paths each, one file apart: 18 together, but the limit is per save.
        let got = diff_saves_with_limit(&store, Some(old), new, 9).unwrap();
        assert_eq!(got.len(), 3);
        // Either one alone is too big for 8, whichever side of the comparison it is on.
        assert!(diff_saves_with_limit(&store, Some(old), small, 8).is_err());
        assert!(diff_saves_with_limit(&store, Some(small), new, 8).is_err());
    }
}
