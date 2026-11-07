//! What a switch changes in the project folder, as two saves, and what `discard` throws away,
//! kept as a save of its own.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::io;

use crate::diary::FolderChange;
use crate::drive::Drive;
use crate::id::{ChangeId, Id};
use crate::journal::Action;
use crate::listing::{Entry, EntryKind, Listing};
use crate::pack::{ObjectKind, PackSet};
use crate::repo::{RepoError, Repository};
use crate::save::{EXTRA_LINE, Person, Save};
use crate::scope::{ancestors, within};
use crate::snapshot::{SaveError, store_listing};
use crate::store::{Store, StoreError};
use crate::writeout::{self, Tree};

/// The longest message a kept save gets, in bytes.
const MAX_MESSAGE: usize = 1024;

/// Names compared byte for byte: what a save holds, not what a drive takes for one name.
const EXACT: Drive = Drive {
    ignores_case: false,
    composes_unicode: false,
};

/// What a change puts at a path: a kind, ID and size. A folder's ID isn't used.
pub(crate) type Put = (EntryKind, Id, u64);

/// Who makes a kept save, and what it says.
pub(crate) struct KeptFrom<'a> {
    /// The save the folder is on, and every kept save's only parent.
    pub base: Id,
    pub message: String,
    pub who: &'a Person,
    /// Its `line` field, when made on a line.
    pub line: Option<&'a str>,
}

/// A kept save's message: the command, then the paths it was given, cut to 1 KiB.
pub(crate) fn message(command: &str, paths: &[String]) -> String {
    let mut message = command.to_owned();
    for path in paths.iter().filter(|p| !p.is_empty()) {
        if message.len() > MAX_MESSAGE {
            break;
        }
        message.push(' ');
        message.push_str(path);
    }
    message.truncate(message.floor_char_boundary(MAX_MESSAGE));
    message
}

/// The folder change of a switch planned as `actions` on a folder on `from.base`: the save
/// holding what the folder held at each path the actions write, and the one holding what they
/// write there, both `from.base` elsewhere. Held contents the store lacks are stored first; a
/// path whose content changed after the plan is left out of both. Each side is `from.base` or
/// `target` when its folders are exactly that save's, otherwise a kept save stored in `packs`.
/// `None` when the two sides are the same.
pub(crate) fn switch_saves(
    repo: &Repository,
    actions: &[Action],
    target: Id,
    from: &KeptFrom,
    drive: Drive,
    packs: &mut PackSet,
) -> Result<Option<FolderChange>, SaveError> {
    let store = repo.store();
    let signed = repo.signatures_allowed();
    let base_root = store.save(&from.base, signed)?.root;
    let target_root = store.save(&target, signed)?.root;
    let copies = writeout::keep_safety_copies(repo, actions, from.base, drive, packs)?;
    let kept: HashSet<&str> = copies.iter().map(|c| c.path.as_str()).collect();
    let renamed = writeout::renamed(actions, drive);
    let unstored = HashMap::new();
    let mut base = Tree::new(store, &unstored, base_root, EXACT);
    let mut wanted = Tree::new(store, &unstored, target_root, EXACT);
    // Neither kept nor saved: it changed after the plan, so the switch leaves it, what is planned
    // below it, and the folders above it, which it can't empty.
    let mut lost = Vec::new();
    for a in actions {
        if let Some(e) = a.expected
            && e.kind != EntryKind::Folder
            && !kept.contains(a.path.as_str())
            && !base
                .entry(&a.path)?
                .is_some_and(|s| s.kind == e.kind && s.id == e.id)
        {
            lost.push(a.path.as_str());
        }
    }
    let (mut held, mut written) = (BTreeMap::new(), BTreeMap::new());
    for a in actions {
        let path = a.path.as_str();
        if lost.iter().any(|l| within(path, l) || within(l, path)) {
            continue;
        }
        let was = a.expected.filter(|_| !renamed.contains(path));
        held.insert(a.path.clone(), was.map(|e| (e.kind, e.id, e.size)));
        let put = wanted.entry(path)?.map(|e| (e.kind, e.id, e.size));
        written.insert(a.path.clone(), put);
    }
    let from_root = overlay(store, base_root, &held, packs)?;
    let to_root = overlay(store, base_root, &written, packs)?;
    if from_root == to_root {
        return Ok(None);
    }
    let mut side = |root: Id| match root {
        r if r == base_root => Ok(from.base),
        r if r == target_root => Ok(target),
        r => kept_save(r, from, packs),
    };
    Ok(Some(FolderChange {
        from: side(from_root)?,
        to: side(to_root)?,
    }))
}

/// The tree of `base_root` with each path in `changes` set to that entry (`None`: removed), its
/// new folder listings stored in `packs`; returns the new root listing's ID. A folder entry's
/// listing is built from the changes below it, on the base's listing there if the base has a
/// folder there; the folders above an entry put are made as needed. A change below a path set
/// to a file, a link or nothing has nowhere to go and is dropped.
pub(crate) fn overlay(
    store: &Store,
    base_root: Id,
    changes: &BTreeMap<String, Option<Put>>,
    packs: &mut PackSet,
) -> Result<Id, SaveError> {
    let is_folder = |c: &Option<Put>| matches!(c, Some((EntryKind::Folder, ..)));
    // Each folder to build, with what changes right in it: an entry put there, or `None` removed.
    let mut todo: HashMap<String, Vec<(String, Option<Entry>)>> = HashMap::new();
    for (path, change) in changes {
        if ancestors(path).any(|up| changes.get(up).is_some_and(|c| !is_folder(c))) {
            continue;
        }
        for up in ancestors(path) {
            todo.entry(up.to_owned()).or_default();
        }
        if is_folder(change) {
            // It goes in its folder once it is built.
            todo.entry(path.clone()).or_default();
            continue;
        }
        let (up, name) = path.rsplit_once('/').unwrap_or(("", path));
        let entry = change.map(|(kind, id, size)| Entry {
            name: name.to_owned(),
            kind,
            id,
            size,
        });
        todo.entry(up.to_owned())
            .or_default()
            .push((name.to_owned(), entry));
    }
    let unstored = HashMap::new();
    let mut base = Tree::new(store, &unstored, base_root, EXACT);
    let mut paths: Vec<String> = todo.keys().filter(|p| !p.is_empty()).cloned().collect();
    // The deepest first, so each folder is built before the one holding it.
    paths.sort_by_key(|p| std::cmp::Reverse(p.len()));
    for path in paths {
        let here = todo.remove(&path).unwrap_or_default();
        let folder = base.folder(&path)?;
        let had = folder.is_some();
        let listing = changed(folder.map(|f| &f.listing), here)?;
        // Made only on the way to changes that put nothing in it, it doesn't come to be.
        if !had && listing.entries().is_empty() && !changes.contains_key(&path) {
            continue;
        }
        let id = store_listing(store, packs, &listing)?;
        let (up, name) = path.rsplit_once('/').unwrap_or(("", &path));
        let entry = Entry {
            name: name.to_owned(),
            kind: EntryKind::Folder,
            id,
            size: 0,
        };
        todo.entry(up.to_owned())
            .or_default()
            .push((name.to_owned(), Some(entry)));
    }
    let top = todo.remove("").unwrap_or_default();
    let listing = changed(base.folder("")?.map(|f| &f.listing), top)?;
    store_listing(store, packs, &listing)
}

/// `listing`, or an empty folder without one, with each name set to its entry or removed.
fn changed(
    listing: Option<&Listing>,
    changes: Vec<(String, Option<Entry>)>,
) -> Result<Listing, SaveError> {
    let mut entries: BTreeMap<String, Entry> = listing
        .map(|l| l.entries().iter().map(|e| (e.name.clone(), e.clone())))
        .into_iter()
        .flatten()
        .collect();
    for (name, entry) in changes {
        match entry {
            Some(e) => entries.insert(name, e),
            None => entries.remove(&name),
        };
    }
    Ok(Listing::new(entries.into_values().collect()).map_err(StoreError::from)?)
}

/// Stores a kept save of the tree `root`, made as `from` says.
fn kept_save(root: Id, from: &KeptFrom, packs: &mut PackSet) -> Result<Id, SaveError> {
    let save = Save {
        root,
        parents: vec![from.base],
        author: from.who.clone(),
        saver: from.who.clone(),
        message: from.message.clone(),
        change_id: ChangeId::random()
            .map_err(io::Error::other)
            .map_err(RepoError::from)?,
        move_notes: vec![],
        signature: vec![],
        extra: from
            .line
            .map(|line| (EXTRA_LINE.to_owned(), line.as_bytes().to_vec()))
            .into_iter()
            .collect(),
    };
    let encoded = save.encode();
    // Refused before it is stored if it wouldn't read back (an over-long name, say).
    Save::decode(&encoded, false).map_err(StoreError::from)?;
    Ok(packs.add_record(ObjectKind::Save, &encoded)?)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;
    use crate::drive;
    use crate::history::tip_of;
    use crate::pack::PackWriter;
    use crate::snapshot::{SaveOutcome, SaveRequest, save_folder};
    use crate::switch::{DiscardOutcome, discard};
    use crate::test_support::plant_save;
    use proptest::prelude::*;
    use std::fs;
    use std::path::Path;

    fn who() -> Person {
        Person {
            name: "Ada".into(),
            email: "ada@example.com".into(),
            time_ms: 1_700_000_000_000,
            tz_offset_min: 0,
        }
    }

    fn write_all(dir: &Path, files: &[(&str, &[u8])]) {
        for (path, bytes) in files {
            let at = dir.join(path);
            fs::create_dir_all(at.parent().unwrap()).unwrap();
            fs::write(at, bytes).unwrap();
        }
    }

    /// A project whose folder holds `files`, saved on `main`, and that save.
    fn saved(files: &[(&str, &[u8])]) -> (tempfile::TempDir, Repository, Id) {
        let dir = tempfile::tempdir().unwrap();
        let mut repo = Repository::init(dir.path(), &who()).unwrap();
        write_all(dir.path(), files);
        let me = who();
        let req = SaveRequest {
            message: "saved",
            who: &me,
            command: "save",
            accept_crlf: false,
            git_excludes: None,
            new_line: None,
            only: None,
        };
        let done = save_folder(&mut repo, &req, &mut |_| {}, &mut |_| true).unwrap();
        assert!(matches!(done, SaveOutcome::Saved { .. }));
        let save = tip_of(&repo.current().unwrap().1).unwrap();
        (dir, repo, save)
    }

    /// Commits a save of `files` on `main`, whatever the folder holds, and gives it.
    fn planted(repo: &mut Repository, files: &[(&str, &[u8])]) -> Id {
        let entries: Vec<(&str, EntryKind, &[u8])> = files
            .iter()
            .map(|&(p, b)| (p, EntryKind::File, b))
            .collect();
        plant_save(repo, &entries);
        tip_of(&repo.current().unwrap().1).unwrap()
    }

    fn discarded(repo: &mut Repository, paths: Option<&[String]>) -> DiscardOutcome {
        discard(repo, paths, &who(), "discard", None, &mut |_| {}).unwrap()
    }

    /// The folder change of a goto from `left`, the save the folder is on, to `target`, as
    /// `goto` plans it, and how many packs it stored.
    fn goto_saves(repo: &mut Repository, left: Id, target: Id) -> (Option<FolderChange>, usize) {
        let _lock = repo.lock_for_change("goto", &mut |_| {}).unwrap();
        let drive = drive::probe(repo.tmp_folder()).unwrap();
        let plan = writeout::plan(repo, left, target, None, false, drive, None).unwrap();
        let me = who();
        let from = KeptFrom {
            base: left,
            message: "goto".into(),
            who: &me,
            line: Some("main"),
        };
        let mut packs = repo.new_packs().unwrap();
        let change = switch_saves(repo, &plan.actions, target, &from, drive, &mut packs).unwrap();
        let made = packs.finish().unwrap();
        for p in &made {
            repo.store_mut().register(p);
        }
        (change, made.len())
    }

    type Entries = BTreeMap<String, (EntryKind, Vec<u8>)>;

    /// Every entry of a save by path: a file or link with what it holds, a folder with nothing.
    fn tree_of(repo: &Repository, save: Id) -> Entries {
        let store = repo.store();
        let mut found = BTreeMap::new();
        let mut todo = vec![(String::new(), store.save(&save, false).unwrap().root)];
        while let Some((at, id)) = todo.pop() {
            for e in store.listing(&id).unwrap().entries() {
                let path = joined(&at, &e.name);
                let mut bytes = Vec::new();
                if e.kind == EntryKind::Folder {
                    todo.push((path.clone(), e.id));
                } else {
                    store.copy_content(&e.id, &mut bytes).unwrap();
                }
                found.insert(path, (e.kind, bytes));
            }
        }
        found
    }

    fn holding(entries: &[(&str, EntryKind, &[u8])]) -> Entries {
        entries
            .iter()
            .map(|&(p, kind, b)| (p.to_owned(), (kind, b.to_vec())))
            .collect()
    }

    fn joined(at: &str, name: &str) -> String {
        if at.is_empty() {
            name.to_owned()
        } else {
            format!("{at}/{name}")
        }
    }

    const FILE: EntryKind = EntryKind::File;
    const FOLDER: EntryKind = EntryKind::Folder;

    #[test]
    fn the_kept_save_holds_what_discard_threw_away() {
        let files: &[(&str, &[u8])] = &[
            ("a.txt", b"a"),
            ("b.txt", b"b"),
            ("dir/c.txt", b"c"),
            ("tool", b"t"),
        ];
        let (dir, mut repo, save) = saved(files);
        let root = dir.path();
        write_all(
            root,
            &[
                ("a.txt", b"mine"),
                ("new.txt", b"new"),
                ("newdir/x.txt", b"x"),
            ],
        );
        fs::remove_file(root.join("b.txt")).unwrap();
        fs::create_dir(root.join("newdir/y")).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(root.join("tool"), fs::Permissions::from_mode(0o755)).unwrap();
        }
        let done = discarded(&mut repo, None);
        let change = done.folder_change.unwrap();
        assert_eq!(change.to, save);
        let kept = repo.store().save(&change.from, false).unwrap();
        assert_eq!(kept.parents, [save]);
        assert_eq!(kept.message, "discard");
        assert_eq!((&kept.author, &kept.saver), (&who(), &who()));
        assert_ne!(
            kept.change_id,
            repo.store().save(&save, false).unwrap().change_id
        );
        assert!(kept.move_notes.is_empty() && kept.signature.is_empty());
        let line = BTreeMap::from([(EXTRA_LINE.to_owned(), b"main".to_vec())]);
        assert_eq!(kept.extra, line);
        let tool = if cfg!(unix) {
            EntryKind::Executable
        } else {
            FILE
        };
        let want = holding(&[
            ("a.txt", FILE, b"mine"),
            ("dir", FOLDER, b""),
            ("dir/c.txt", FILE, b"c"),
            ("new.txt", FILE, b"new"),
            ("newdir", FOLDER, b""),
            ("newdir/x.txt", FILE, b"x"),
            ("newdir/y", FOLDER, b""),
            ("tool", tool, b"t"),
        ]);
        assert_eq!(tree_of(&repo, change.from), want);
        assert_eq!(fs::read(root.join("b.txt")).unwrap(), b"b");
        assert!(!root.join("newdir").exists());
    }

    #[test]
    fn a_partial_discard_keeps_only_what_it_names() {
        let (dir, mut repo, save) = saved(&[("a.txt", b"a"), ("b.txt", b"b")]);
        write_all(dir.path(), &[("a.txt", b"mine"), ("b.txt", b"mine too")]);
        let done = discarded(&mut repo, Some(&["a.txt".to_owned()]));
        let change = done.folder_change.unwrap();
        assert_eq!(change.to, save);
        let kept = repo.store().save(&change.from, false).unwrap();
        assert_eq!(kept.message, "discard a.txt");
        let want = holding(&[("a.txt", FILE, b"mine"), ("b.txt", FILE, b"b")]);
        assert_eq!(tree_of(&repo, change.from), want);
    }

    #[test]
    fn a_kind_change_is_kept_both_ways() {
        let (dir, mut repo, save) = saved(&[("f", b"f"), ("g/z", b"z")]);
        let root = dir.path();
        fs::remove_file(root.join("f")).unwrap();
        fs::remove_dir_all(root.join("g")).unwrap();
        write_all(root, &[("f/x", b"x"), ("g", b"g")]);
        let done = discarded(&mut repo, None);
        let change = done.folder_change.unwrap();
        assert_eq!(change.to, save);
        let want = holding(&[("f", FOLDER, b""), ("f/x", FILE, b"x"), ("g", FILE, b"g")]);
        assert_eq!(tree_of(&repo, change.from), want);
        assert_eq!(fs::read(root.join("f")).unwrap(), b"f");
        assert_eq!(fs::read(root.join("g/z")).unwrap(), b"z");
    }

    #[cfg(feature = "fault-injection")]
    #[test]
    fn a_file_changed_after_the_plan_is_left_out_of_both_sides() {
        use crate::fsio::fault;
        let (dir, mut repo, save) = saved(&[("a.txt", b"a"), ("b.txt", b"b")]);
        write_all(dir.path(), &[("a.txt", b"mine"), ("b.txt", b"mine too")]);
        let at = dir.path().join("a.txt");
        fault::act_at("discard:planned", move || fs::write(at, b"edited").unwrap());
        let done = discarded(&mut repo, None);
        assert_eq!(done.applied.changed_since_plan, ["a.txt"]);
        assert_eq!(fs::read(dir.path().join("a.txt")).unwrap(), b"edited");
        let change = done.folder_change.unwrap();
        assert_eq!(change.to, save);
        // Both sides hold the saved a.txt.
        let want = holding(&[("a.txt", FILE, b"a"), ("b.txt", FILE, b"mine too")]);
        assert_eq!(tree_of(&repo, change.from), want);
    }

    #[test]
    fn nothing_to_discard_keeps_nothing() {
        let (_dir, mut repo, _) = saved(&[("a.txt", b"a")]);
        let (head, packs) = (repo.head().unwrap(), repo.store().pack_count());
        let done = discarded(&mut repo, None);
        assert_eq!(done.folder_change, None);
        assert_eq!(repo.head().unwrap(), head);
        assert_eq!(repo.store().pack_count(), packs);
    }

    #[test]
    fn a_plain_gotos_two_sides_are_the_two_saves() {
        let (_dir, mut repo, left) = saved(&[("a.txt", b"a"), ("gone/x.txt", b"x")]);
        let target = planted(&mut repo, &[("a.txt", b"A"), ("new/b.txt", b"b")]);
        let (change, packs) = goto_saves(&mut repo, left, target);
        assert_eq!(
            change,
            Some(FolderChange {
                from: left,
                to: target
            })
        );
        assert_eq!(packs, 0);
    }

    #[test]
    fn work_that_matched_the_target_is_left_out_of_the_switch() {
        let (dir, mut repo, left) = saved(&[("a.txt", b"a"), ("gone/x.txt", b"x")]);
        let target = planted(&mut repo, &[("a.txt", b"A"), ("new/b.txt", b"b")]);
        write_all(dir.path(), &[("a.txt", b"A")]);
        let change = goto_saves(&mut repo, left, target).0.unwrap();
        assert_eq!(change.from, left);
        assert_ne!(change.to, target);
        let kept = repo.store().save(&change.to, false).unwrap();
        assert_eq!((kept.parents, kept.message.as_str()), (vec![left], "goto"));
        let want = holding(&[
            ("a.txt", FILE, b"a"),
            ("new", FOLDER, b""),
            ("new/b.txt", FILE, b"b"),
        ]);
        assert_eq!(tree_of(&repo, change.to), want);
    }

    #[test]
    fn a_name_changed_only_in_letter_case_is_one_entry_on_each_side() {
        let (_dir, mut repo, left) = saved(&[("Readme.md", b"r")]);
        let target = planted(&mut repo, &[("README.md", b"r")]);
        let (change, packs) = goto_saves(&mut repo, left, target);
        assert_eq!(
            change,
            Some(FolderChange {
                from: left,
                to: target
            })
        );
        assert_eq!(packs, 0);
    }

    #[cfg(unix)]
    #[test]
    fn discard_leaves_a_link_it_could_not_write_back() {
        use std::os::unix::fs::symlink;
        let (dir, mut repo, save) = saved(&[("a.txt", b"a")]);
        let root = dir.path();
        write_all(root, &[("a.txt", b"mine"), ("newdir/x.txt", b"x")]);
        symlink("/tmp", root.join("out")).unwrap();
        symlink("/etc", root.join("newdir/inner")).unwrap();
        symlink("a.txt", root.join("inside")).unwrap();
        let done = discarded(&mut repo, None);
        assert_eq!(done.links_left, ["newdir/inner", "out"]);
        let warned: Vec<&str> = done
            .applied
            .warnings
            .iter()
            .filter(|w| w.kind == crate::warning::LINKS_LEFT)
            .map(|w| w.message.as_str())
            .collect();
        assert_eq!(warned.len(), 1, "{warned:?}");
        assert!(warned[0].contains("newdir/inner") && warned[0].contains("out"));
        for (link, to) in [("out", "/tmp"), ("newdir/inner", "/etc")] {
            assert_eq!(fs::read_link(root.join(link)).unwrap(), Path::new(to));
        }
        // The rest is thrown away.
        assert_eq!(fs::read(root.join("a.txt")).unwrap(), b"a");
        assert!(!root.join("newdir/x.txt").exists());
        assert!(root.join("inside").symlink_metadata().is_err());
        let change = done.folder_change.unwrap();
        assert_eq!(change.to, save);
        let want = holding(&[
            ("a.txt", FILE, b"mine"),
            ("inside", EntryKind::Symlink, b"a.txt"),
            ("newdir", FOLDER, b""),
            ("newdir/x.txt", FILE, b"x"),
        ]);
        assert_eq!(tree_of(&repo, change.from), want);
    }

    /// A folder as the overlay test sees it: its ID and size are zero.
    const A_FOLDER: Put = (EntryKind::Folder, Id([0; 32]), 0);

    fn path() -> impl Strategy<Value = String> {
        let name = prop::sample::select(vec!["a", "a.txt", "b"]);
        prop::collection::vec(name, 1..4).prop_map(|names| names.join("/"))
    }

    fn leaf() -> impl Strategy<Value = Put> {
        let kind = prop::sample::select(vec![EntryKind::File, EntryKind::Symlink]);
        (kind, 1u8..4, 0u64..3).prop_map(|(kind, b, size)| (kind, Id([b; 32]), size))
    }

    /// A save's entries by path, with every folder above one.
    fn tree() -> impl Strategy<Value = BTreeMap<String, Put>> {
        let entry = prop_oneof![leaf(), Just(A_FOLDER)];
        prop::collection::vec((path(), entry), 0..12).prop_map(|entries| {
            let mut tree = BTreeMap::new();
            for (path, leaf) in entries {
                let in_a_file = ancestors(&path)
                    .any(|up| tree.get(up).is_some_and(|e: &Put| e.0 != EntryKind::Folder));
                if in_a_file || tree.contains_key(&path) {
                    continue;
                }
                for up in ancestors(&path) {
                    tree.insert(up.to_owned(), A_FOLDER);
                }
                tree.insert(path, leaf);
            }
            tree
        })
    }

    fn changes() -> impl Strategy<Value = BTreeMap<String, Option<Put>>> {
        let change = prop_oneof![Just(None), leaf().prop_map(Some), Just(Some(A_FOLDER))];
        prop::collection::btree_map(path(), change, 0..6)
    }

    /// Stores `tree` and gives its top listing's ID.
    fn store_tree(tree: &BTreeMap<String, Put>, pack: &mut PackWriter) -> Id {
        let mut folders: Vec<&str> = tree
            .iter()
            .filter(|(_, leaf)| leaf.0 == EntryKind::Folder)
            .map(|(p, _)| p.as_str())
            .chain([""])
            .collect();
        // The deepest first, so each folder's ID is known to the one holding it.
        folders.sort_by_key(|p| std::cmp::Reverse(p.len()));
        let mut ids: HashMap<&str, Id> = HashMap::new();
        for folder in folders {
            let entries = tree
                .iter()
                .filter(|(p, _)| p.rsplit_once('/').map_or("", |(up, _)| up) == folder)
                .map(|(p, &(kind, id, size))| Entry {
                    name: p.rsplit('/').next().unwrap().to_owned(),
                    kind,
                    id: if kind == EntryKind::Folder {
                        ids[p.as_str()]
                    } else {
                        id
                    },
                    size,
                })
                .collect();
            let listing = Listing::new(entries).unwrap().encode();
            let id = pack
                .add_record(ObjectKind::FolderListing, &listing)
                .unwrap();
            ids.insert(folder, id);
        }
        ids[""]
    }

    fn read_back(store: &Store, root: Id) -> BTreeMap<String, Put> {
        let mut found = BTreeMap::new();
        let mut todo = vec![(String::new(), root)];
        while let Some((at, id)) = todo.pop() {
            for e in store.listing(&id).unwrap().entries() {
                let path = joined(&at, &e.name);
                if e.kind == EntryKind::Folder {
                    todo.push((path.clone(), e.id));
                    found.insert(path, A_FOLDER);
                } else {
                    found.insert(path, (e.kind, e.id, e.size));
                }
            }
        }
        found
    }

    /// `base` with `changes` made one by one, shallowest first: what lay at or below a path goes,
    /// unless a folder is put where one was, and the folders above an entry put are made. A
    /// change below a file, a link or nothing that another change put there is dropped.
    fn applied(
        base: &BTreeMap<String, Put>,
        changes: &BTreeMap<String, Option<Put>>,
    ) -> BTreeMap<String, Put> {
        let is_folder = |e: Option<&Put>| e.is_some_and(|e| e.0 == EntryKind::Folder);
        let mut tree = base.clone();
        for (path, change) in changes {
            if ancestors(path).any(|up| changes.get(up).is_some_and(|c| !is_folder(c.as_ref()))) {
                continue;
            }
            if !(is_folder(change.as_ref()) && is_folder(tree.get(path))) {
                tree.retain(|p, _| !within(p, path));
            }
            let Some(leaf) = change else { continue };
            for up in ancestors(path) {
                if !is_folder(tree.get(up)) {
                    tree.retain(|p, _| !within(p, up));
                    tree.insert(up.to_owned(), A_FOLDER);
                }
            }
            tree.insert(path.clone(), *leaf);
        }
        tree
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]
        #[test]
        fn overlay_sets_exactly_the_paths_given(base in tree(), changes in changes()) {
            let dir = tempfile::tempdir().unwrap();
            let (tmp, packs) = (dir.path().join("tmp"), dir.path().join("packs"));
            fs::create_dir(&tmp).unwrap();
            fs::create_dir(&packs).unwrap();
            let mut pack = PackWriter::create(&tmp).unwrap();
            let base_root = store_tree(&base, &mut pack);
            pack.finish(&packs).unwrap();
            let mut store = Store::open(&packs).unwrap();
            let mut set = PackSet::with_limit(&tmp, &packs, u64::MAX).unwrap();
            let root = overlay(&store, base_root, &changes, &mut set).unwrap();
            for made in set.finish().unwrap() {
                store.register(&made);
            }
            prop_assert_eq!(read_back(&store, root), applied(&base, &changes));
        }
    }
}
