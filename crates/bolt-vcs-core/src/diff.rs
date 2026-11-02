//! What changed between two saves, file by file.

use std::cmp::Ordering;

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

/// A file that differs between two saves. The path is borrowed from the walk's own buffer and is
/// only good until the callback returns: copy it to keep it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileChange<'a> {
    pub path: &'a str,
    pub change: ChangeKind,
    /// The new kind, or the old one for a removed file.
    pub kind: EntryKind,
    /// The kind before the change, if the path had a file.
    pub old_kind: Option<EntryKind>,
    pub old_size: Option<u64>,
    pub new_size: Option<u64>,
}

/// Hands every file that differs between two saves to `each`, in whole-path byte order. `old_root`
/// is `None` with nothing to compare. A save past the limits of storage-format.md section 3 gives
/// `SaveTooLarge` when the walk reaches it, after `each` has seen earlier changes; to find out
/// before showing anything, run [`count_changes`] first, which walks the same way.
pub fn for_each_change<E: From<StoreError>>(
    store: &Store,
    old_root: Option<Id>,
    new_root: Id,
    each: impl FnMut(FileChange<'_>) -> Result<(), E>,
) -> Result<(), E> {
    walk(store, old_root, new_root, MAX_SAVE_ENTRIES, each)
}

/// How many files differ between the two saves. It is the walk of [`for_each_change`] with nothing
/// done per change, so it stops at the same limits and tells whether the saves can be walked.
pub fn count_changes(
    store: &Store,
    old_root: Option<Id>,
    new_root: Id,
) -> Result<usize, StoreError> {
    let mut count = 0;
    for_each_change(store, old_root, new_root, |_| {
        count += 1;
        Ok::<(), StoreError>(())
    })?;
    Ok(count)
}

/// What the walk still has to do with one name in a folder.
enum Step {
    /// Hand out a file that differs. Its path is empty until its turn comes.
    File(FileChange<'static>),
    /// Compare two folders, `depth` levels below the root; a side is `None` if it has no folder
    /// of that name.
    Folder {
        old: Option<Id>,
        new: Option<Id>,
        depth: usize,
    },
}

/// A name in a folder that the walk has not come to yet.
struct Todo {
    name: String,
    /// How long the path of the folder holding the name is.
    at: usize,
    step: Step,
}

impl Todo {
    /// The bytes that decide where the paths of this name come among the paths of its folder.
    /// A file is just its name. A folder's files are all `name/…`, so the folder `a` comes
    /// after `a.txt` (`.` is 0x2E) and before `a0` (`0` is 0x30), because `/` is 0x2F.
    fn key(&self) -> impl Iterator<Item = u8> + '_ {
        let slash = matches!(self.step, Step::Folder { .. }).then_some(b'/');
        self.name.bytes().chain(slash)
    }
}

/// The walk behind [`for_each_change`], with the entry limit of the saves as a parameter.
fn walk<E: From<StoreError>>(
    store: &Store,
    old_root: Option<Id>,
    new_root: Id,
    max_entries: u64,
    mut each: impl FnMut(FileChange<'_>) -> Result<(), E>,
) -> Result<(), E> {
    if old_root == Some(new_root) {
        return Ok(());
    }
    let too_large = |detail| StoreError::SaveTooLarge { detail };
    // Each save is held to the limits on its own, so two saves that pass can always be compared.
    let (mut old_seen, mut new_seen) = (
        SaveLimits::new(MAX_SAVE_DEPTH, max_entries),
        SaveLimits::new(MAX_SAVE_DEPTH, max_entries),
    );
    // What is left to do waits here, not in a call per folder level, so a save nested thousands of
    // levels deep cannot overflow the stack. A folder's differing names are pushed last path first,
    // so the next one popped is always the one whose paths come next.
    let mut todo = vec![Todo {
        name: String::new(),
        at: 0,
        step: Step::Folder {
            old: old_root,
            new: Some(new_root),
            depth: 0,
        },
    }];
    // The path of what is being worked on (`/` after every folder's name) is the only path in
    // memory. A waiting name carries just the length of its folder's path: what was pushed after it
    // is deeper, so when it is popped, the path up to that length is still its folder's.
    let mut path = String::new();
    while let Some(Todo { name, at, step }) = todo.pop() {
        path.truncate(at);
        path.push_str(&name);
        match step {
            Step::File(change) => each(FileChange {
                path: &path,
                ..change
            })?,
            Step::Folder { old, new, depth } => {
                old_seen.check_depth(depth).map_err(too_large)?; // the new save is as deep
                if depth > 0 {
                    path.push('/');
                }
                let (old, new) = (listing_of(store, old)?, listing_of(store, new)?);
                old_seen.add(old.entries().len()).map_err(too_large)?;
                new_seen.add(new.entries().len()).map_err(too_large)?;
                let (here, first) = (path.len(), todo.len());
                for (name, o, n) in pairs(old.entries(), new.entries()) {
                    let (of, nf) = (folder_id(o), folder_id(n));
                    if of != nf {
                        todo.push(Todo {
                            name: name.to_owned(),
                            at: here,
                            step: Step::Folder {
                                old: of,
                                new: nf,
                                depth: depth + 1,
                            },
                        });
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
                    todo.push(Todo {
                        name: name.to_owned(),
                        at: here,
                        step: Step::File(FileChange {
                            path: "",
                            change,
                            kind: shown.kind,
                            old_kind: fo.map(|e| e.kind),
                            old_size: fo.map(|e| e.size),
                            new_size: fnew.map(|e| e.size),
                        }),
                    });
                }
                todo[first..].sort_unstable_by(|a, b| b.key().cmp(a.key()));
            }
        }
    }
    Ok(())
}

/// The entries of two listings that share a name, or have it on one side only, in the order of
/// the names. Listings are sorted by name, so this reads each once.
fn pairs<'a>(
    old: &'a [Entry],
    new: &'a [Entry],
) -> impl Iterator<Item = (&'a str, Option<&'a Entry>, Option<&'a Entry>)> {
    let (mut old, mut new) = (old.iter().peekable(), new.iter().peekable());
    std::iter::from_fn(move || {
        let order = match (old.peek(), new.peek()) {
            (None, None) => return None,
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (Some(o), Some(n)) => o.name.cmp(&n.name),
        };
        Some(match order {
            Ordering::Less => {
                let o = old.next()?;
                (o.name.as_str(), Some(o), None)
            }
            Ordering::Greater => {
                let n = new.next()?;
                (n.name.as_str(), None, Some(n))
            }
            Ordering::Equal => {
                let (o, n) = (old.next()?, new.next()?);
                (o.name.as_str(), Some(o), Some(n))
            }
        })
    })
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
    use proptest::prelude::*;
    use std::collections::{BTreeMap, BTreeSet};

    /// A change with its path copied out of the walk's buffer.
    #[derive(Debug, PartialEq, Eq)]
    struct Seen {
        path: String,
        change: ChangeKind,
        kind: EntryKind,
        old_kind: Option<EntryKind>,
        old_size: Option<u64>,
        new_size: Option<u64>,
    }

    /// Every change the walk hands out, in the order it hands them out, under an entry limit.
    fn collect(
        store: &Store,
        old: Option<Id>,
        new: Id,
        max_entries: u64,
    ) -> Result<Vec<Seen>, StoreError> {
        let mut seen = Vec::new();
        walk(store, old, new, max_entries, |c| {
            seen.push(Seen {
                path: c.path.to_owned(),
                change: c.change,
                kind: c.kind,
                old_kind: c.old_kind,
                old_size: c.old_size,
                new_size: c.new_size,
            });
            Ok::<(), StoreError>(())
        })?;
        Ok(seen)
    }

    /// `collect` under the limits of the format, for saves that are meant to pass.
    fn changes(store: &Store, old: Option<Id>, new: Id) -> Vec<Seen> {
        collect(store, old, new, MAX_SAVE_ENTRIES).unwrap()
    }

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
            changes(&store, Some(old), new)
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
        assert_eq!(changes(&store, None, new).len(), 5);
        assert!(changes(&store, Some(new), new).is_empty());
        assert_eq!(count_changes(&store, Some(old), new).unwrap(), 6);
        assert_eq!(count_changes(&store, None, new).unwrap(), 5);
        assert_eq!(count_changes(&store, Some(new), new).unwrap(), 0);
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
        let got: Vec<(String, ChangeKind)> = changes(&store, Some(old), new)
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
        let got = changes(&store, None, root);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].path, format!("{}f", "d/".repeat(MAX_SAVE_DEPTH)));
        assert_eq!(count_changes(&store, None, root).unwrap(), 1);
    }

    #[test]
    fn depth_past_the_limit_is_refused() {
        let (_dir, store, [root]) = store_with(|p| [chain(MAX_SAVE_DEPTH + 1, p)]);
        let err = count_changes(&store, None, root).unwrap_err();
        assert!(matches!(err, StoreError::SaveTooLarge { .. }), "{err:?}");
        // Walking says the same, though it has handed out changes by the time it gets there.
        let err = collect(&store, None, root, MAX_SAVE_ENTRIES).unwrap_err();
        assert!(matches!(err, StoreError::SaveTooLarge { .. }), "{err:?}");
    }

    #[test]
    fn entry_limit_counts_every_path() {
        // The top listing has 3 entries. Its three folders are one listing of 2 files, which the
        // walk reads once for each path to it: 3 + 3 * 2 = 9 entries, though only 5 are stored.
        const PATHS: u64 = 3 + 3 * 2;
        let (_dir, store, [root]) = store_with(|p| [three_paths_to_one_listing(b"y", p)]);
        let got = collect(&store, None, root, PATHS).unwrap();
        assert_eq!(got.len(), 6);
        let err = collect(&store, None, root, PATHS - 1).unwrap_err();
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
        let got = collect(&store, Some(old), new, 9).unwrap();
        assert_eq!(got.len(), 3);
        // Either one alone is too big for 8, whichever side of the comparison it is on.
        assert!(collect(&store, Some(old), small, 8).is_err());
        assert!(collect(&store, Some(small), new, 8).is_err());
    }

    #[test]
    fn changes_come_in_the_order_of_whole_paths() {
        let (_dir, store, [old, new]) = store_with(|p| {
            // `a` is a file here and becomes a folder.
            let old = folder("", vec![file("a", EntryKind::File, b"a", p)], p).id;
            // The names next to the folder `a`, whose files are `a/…`: `-` (0x2D) and `.` (0x2E)
            // sort before `/` (0x2F), `0` and `~` after it. The same holds inside `a`, one level
            // down, around the folder `b`.
            let inside = vec![
                file("b-1", EntryKind::File, b"1", p),
                file("b.c", EntryKind::File, b"2", p),
                folder("b", vec![file("d", EntryKind::File, b"3", p)], p),
                file("b0", EntryKind::File, b"4", p),
            ];
            let new = vec![
                file("a~", EntryKind::File, b"~", p),
                file("a0", EntryKind::File, b"0", p),
                folder("a", inside, p),
                file("a.txt", EntryKind::File, b"t", p),
                file("a-b", EntryKind::File, b"b", p),
            ];
            [old, folder("", new, p).id]
        });
        let got: Vec<String> = changes(&store, Some(old), new)
            .into_iter()
            .map(|c| c.path)
            .collect();
        let want = [
            "a", "a-b", "a.txt", "a/b-1", "a/b.c", "a/b/d", "a/b0", "a0", "a~",
        ];
        assert_eq!(got, want);
        // Which is what sorting the whole paths gives.
        assert!(got.is_sorted());
    }

    /// A tree as a test describes it, before it is stored.
    #[derive(Debug, Clone)]
    enum Node {
        File(EntryKind, Vec<u8>),
        Folder(BTreeMap<String, Node>),
    }

    /// Names that sort differently as a folder (`name/`) than as a file: they differ in bytes just
    /// below and just above `/`. Some are not ASCII, so a byte is not a character.
    fn name() -> impl Strategy<Value = String> {
        let names = vec![
            "a",
            "a ",
            "a-",
            "a-b",
            "a.",
            "a.txt",
            "a0",
            "a~",
            "ab",
            "b",
            "\u{e9}",
            "\u{e9}-x",
            "\u{65e5}\u{672c}",
        ];
        prop::sample::select(names).prop_map(str::to_owned)
    }

    fn tree() -> impl Strategy<Value = BTreeMap<String, Node>> {
        let kinds = vec![EntryKind::File, EntryKind::Executable, EntryKind::Symlink];
        let contents = vec![b"1".to_vec(), b"2".to_vec(), Vec::new()];
        let leaf = (prop::sample::select(kinds), prop::sample::select(contents))
            .prop_map(|(kind, bytes)| Node::File(kind, bytes));
        let node = leaf.prop_recursive(3, 24, 4, |inner| {
            prop::collection::btree_map(name(), inner, 0..4).prop_map(Node::Folder)
        });
        prop::collection::btree_map(name(), node, 0..5)
    }

    /// Stores a tree and gives its top folder.
    fn store_tree(name: &str, tree: &BTreeMap<String, Node>, pack: &mut PackWriter) -> Entry {
        let entries = tree
            .iter()
            .map(|(n, node)| match node {
                Node::File(kind, bytes) => file(n, *kind, bytes, pack),
                Node::Folder(inner) => store_tree(n, inner, pack),
            })
            .collect();
        folder(name, entries, pack)
    }

    /// Every file of a tree by its whole path.
    fn flatten<'a>(
        tree: &'a BTreeMap<String, Node>,
        prefix: &str,
        out: &mut BTreeMap<String, (EntryKind, &'a [u8])>,
    ) {
        for (name, node) in tree {
            match node {
                Node::File(kind, bytes) => {
                    out.insert(format!("{prefix}{name}"), (*kind, bytes));
                }
                Node::Folder(inner) => flatten(inner, &format!("{prefix}{name}/"), out),
            }
        }
    }

    /// What comparing two trees should give, worked out the other way round: compare every file
    /// by its whole path, and put the paths in order.
    fn by_whole_path(old: &BTreeMap<String, Node>, new: &BTreeMap<String, Node>) -> Vec<Seen> {
        let (mut a, mut b) = (BTreeMap::new(), BTreeMap::new());
        flatten(old, "", &mut a);
        flatten(new, "", &mut b);
        let paths: BTreeSet<&String> = a.keys().chain(b.keys()).collect();
        let mut out = Vec::new();
        for path in paths {
            let (o, n) = (a.get(path), b.get(path));
            let change = match (o, n) {
                (Some(_), None) => ChangeKind::Removed,
                (None, Some(_)) => ChangeKind::Added,
                (Some(x), Some(y)) if x.0 != y.0 => ChangeKind::KindChanged,
                (Some(x), Some(y)) if x.1 != y.1 => ChangeKind::Modified,
                _ => continue,
            };
            let Some(shown) = n.or(o) else { continue };
            out.push(Seen {
                path: path.clone(),
                change,
                kind: shown.0,
                old_kind: o.map(|x| x.0),
                old_size: o.map(|x| x.1.len() as u64),
                new_size: n.map(|y| y.1.len() as u64),
            });
        }
        out
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]
        #[test]
        fn order_is_that_of_sorting_all_the_paths(old in tree(), new in tree()) {
            let (_dir, store, [o, n]) = store_with(|p| {
                [store_tree("", &old, p).id, store_tree("", &new, p).id]
            });
            let want = by_whole_path(&old, &new);
            prop_assert_eq!(&changes(&store, Some(o), n), &want);
            prop_assert_eq!(count_changes(&store, Some(o), n).unwrap(), want.len());
        }
    }

    #[test]
    fn the_callback_can_stop_the_walk() {
        let (_dir, store, [root]) = store_with(|p| {
            let files = ["a", "b", "c", "d"].map(|n| file(n, EntryKind::File, n.as_bytes(), p));
            [folder("", files.to_vec(), p).id]
        });
        let mut seen = Vec::new();
        let stopped: Result<(), Box<dyn std::error::Error>> =
            for_each_change(&store, None, root, |c| {
                seen.push(c.path.to_owned());
                if seen.len() == 2 {
                    return Err("enough".into());
                }
                Ok(())
            });
        assert_eq!(stopped.unwrap_err().to_string(), "enough");
        assert_eq!(seen, ["a", "b"]);
    }

    #[test]
    fn folders_both_saves_share_are_never_read() {
        // A folder both saves hold, whose listing is not in the store: reading it would fail.
        let shared = Entry {
            name: "shared".into(),
            kind: EntryKind::Folder,
            id: Id([9; 32]),
            size: 0,
        };
        let (_dir, store, [old, new]) = store_with(|p| {
            [
                folder(
                    "",
                    vec![shared.clone(), file("f", EntryKind::File, b"1", p)],
                    p,
                )
                .id,
                folder(
                    "",
                    vec![shared.clone(), file("f", EntryKind::File, b"2", p)],
                    p,
                )
                .id,
            ]
        });
        let got = changes(&store, Some(old), new);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].path, "f");
    }
}
