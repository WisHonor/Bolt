//! What a switch changes in the project folder, as two saves, and what `discard` throws away,
//! kept as a save of its own.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::io;

use crate::diary::FolderChange;
use crate::diff::for_each_entry_change_in;
use crate::drive::Drive;
use crate::id::{ChangeId, Id};
use crate::journal::{Action, Change};
use crate::listing::{Entry, EntryKind, Listing};
use crate::pack::{ObjectKind, PackSet};
use crate::repo::{RepoError, Repository};
use crate::save::{EXTRA_LINE, Person, Save};
use crate::scope::{ancestors, within};
use crate::snapshot::{SaveError, store_listing};
use crate::store::{Store, StoreError};
use crate::writeout::{self, Folders, Plan, Tree};

/// The longest message a kept save gets, in bytes.
const MAX_MESSAGE: usize = 1024;

/// Names compared byte for byte: what a save holds, not what a drive takes for one name.
pub(crate) const EXACT: Drive = Drive {
    ignores_case: false,
    composes_unicode: false,
};

/// What a change puts at a path: a kind, ID and size. A folder's ID isn't used.
pub(crate) type Put = (EntryKind, Id, u64);

/// A folder put at a path, its listing built from the base's and the changes below it.
const FOLDER: Put = (EntryKind::Folder, Id([0; 32]), 0);

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

/// The folder change of `plan`, a switch from the save `from` to `target`, on a folder on
/// `kept.base`. The "to" save holds `target`'s version at each path the plan writes and each it
/// found the folder already holding (with everything below it): the folder after the switch,
/// wherever it touched. The "from" save is that with what the folder held at each path the plan
/// writes. Both are `kept.base` elsewhere, at what the plan leaves unwritten too, except inside a
/// folder the plan makes where the folder held a file, a link or nothing: there "to" holds only
/// what the plan writes or finds already there; and inside a folder the plan writes, both
/// hold `from`'s version wherever it differs from the base's. A folder above a path the plan writes, which it
/// neither makes nor removes, is a folder on both sides: it was one before and after. A held
/// file or link the store lacks is stored first; one that couldn't be, having changed since the
/// plan, is left out of both sides with what is planned below it, and the folders above it, which
/// can't be emptied, are neither made nor removed. Each side is `kept.base`, `from` or
/// `target` when its folders are exactly that save's, otherwise a kept save stored in `packs`.
/// `None` when the two sides are the same. Also gives how many safety copies it kept.
pub(crate) fn switch_saves(
    repo: &Repository,
    plan: &Plan,
    from: Id,
    target: Id,
    kept: &KeptFrom,
    drive: Drive,
    packs: &mut PackSet,
) -> Result<(Option<FolderChange>, usize), SaveError> {
    let store = repo.store();
    let signed = repo.signatures_allowed();
    let base_root = store.save(&kept.base, signed)?.root;
    let from_root = store.save(&from, signed)?.root;
    let target_root = store.save(&target, signed)?.root;
    let (held, copies) = held(repo, &plan.actions, kept.base, base_root, drive, packs)?;
    let unstored = HashMap::new();
    let put = |e: &Entry| (e.kind, e.id, e.size);
    let mut wanted = Tree::new(store, &unstored, target_root, EXACT);
    let mut after = BTreeMap::new();
    for path in held.keys() {
        after.insert(path.clone(), wanted.entry(path)?.as_ref().map(put));
    }
    if !plan.matched.is_empty() {
        let matched = |path: &str| plan.matched.iter().any(|m| within(path, m));
        let changes = |path: &str, _: Option<&Entry>, t: Option<&Entry>| {
            if matched(path) {
                after.insert(path.to_owned(), t.map(put));
            }
            Ok::<(), SaveError>(())
        };
        for_each_entry_change_in(store, &unstored, Some(from_root), target_root, changes)?;
    }
    // Inside a folder the plan writes, the folder held what `from` holds, not the base: where
    // `--shelve` set work aside, the step is planned from a save holding that work.
    if from_root != base_root {
        let below = |path: &str| held.keys().any(|h| path != h && within(path, h));
        let aside = |path: &str, _: Option<&Entry>, f: Option<&Entry>| {
            if below(path) {
                after.entry(path.to_owned()).or_insert(f.map(put));
            }
            Ok::<(), SaveError>(())
        };
        for_each_entry_change_in(store, &unstored, Some(base_root), from_root, aside)?;
    }
    // The base's entries in such a folder went with it when the user removed or replaced it: they
    // stay gone. Only a rename in letter case or Unicode form makes a folder over a folder.
    let mut base = Tree::new(store, &unstored, base_root, EXACT);
    let made = plan.actions.iter().filter(|a| {
        a.change == Change::MakeFolder
            && !a.expected.is_some_and(|e| e.kind == EntryKind::Folder)
            && held.contains_key(&a.path)
    });
    for a in made {
        if let Some(folder) = base.folder(&a.path)? {
            for e in folder.listing.entries() {
                after
                    .entry(format!("{}/{}", a.path, e.name))
                    .or_insert(None);
            }
        }
    }
    // A folder above a path the plan writes, which it neither makes nor removes, was one before
    // and after it, even where the base holds none, or a file: so it is one on both sides.
    let untouched: BTreeSet<String> = held
        .keys()
        .flat_map(|p| ancestors(p))
        .filter(|up| !held.contains_key(*up))
        .map(str::to_owned)
        .collect();
    let mut before = after.clone();
    before.extend(held);
    for up in untouched {
        before.entry(up.clone()).or_insert(Some(FOLDER));
        after.entry(up).or_insert(Some(FOLDER));
    }
    let before_root = overlay(store, base_root, &before, packs)?;
    let after_root = overlay(store, base_root, &after, packs)?;
    if before_root == after_root {
        return Ok((None, copies));
    }
    let mut side = |root: Id| match root {
        r if r == base_root => Ok(kept.base),
        r if r == from_root => Ok(from),
        r if r == target_root => Ok(target),
        r => kept_save(r, kept, packs),
    };
    let change = FolderChange {
        from: side(before_root)?,
        to: side(after_root)?,
    };
    Ok((Some(change), copies))
}

/// What `--shelve` set aside before a step.
pub(crate) struct Aside {
    /// A kept save on the save the project is on, holding the folder's version at `paths`.
    pub shelf: Id,
    /// The save to plan the step from instead, holding the same there.
    pub step_from: Id,
    pub paths: Vec<String>,
}

/// What `--shelve` sets aside before a step, `actions` being a discard of the unsaved work in its
/// way: the shelf, a kept save on `shelf.base`, and `onto.base` with the same entries, to plan
/// the step from so that it overwrites them, both stored in `packs`. `None` when nothing differs
/// from `shelf.base`.
pub(crate) fn set_aside(
    repo: &Repository,
    actions: &[Action],
    shelf: &KeptFrom,
    onto: &KeptFrom,
    drive: Drive,
    packs: &mut PackSet,
) -> Result<Option<Aside>, SaveError> {
    let store = repo.store();
    let signed = repo.signatures_allowed();
    let base_root = store.save(&shelf.base, signed)?.root;
    let (held, _) = held(repo, actions, shelf.base, base_root, drive, packs)?;
    let shelf_root = overlay(store, base_root, &held, packs)?;
    if shelf_root == base_root {
        return Ok(None);
    }
    let made = kept_save(shelf_root, shelf, packs)?;
    let onto_root = store.save(&onto.base, signed)?.root;
    let step_root = overlay(store, onto_root, &held, packs)?;
    let step_from = match step_root {
        r if r == onto_root => onto.base,
        r => kept_save(r, onto, packs)?,
    };
    Ok(Some(Aside {
        shelf: made,
        step_from,
        paths: held.into_keys().collect(),
    }))
}

/// What the folder held at each path `actions` write, as a save of it on `base` (whose top listing
/// is `base_root`) holds it, with the safety copies that takes stored in `packs`, and how many
/// there were. A held file or link that couldn't be kept, having changed since the plan, is left
/// out, and so is what is planned below it and the folders above it, which can't be emptied.
fn held(
    repo: &Repository,
    actions: &[Action],
    base: Id,
    base_root: Id,
    drive: Drive,
    packs: &mut PackSet,
) -> Result<(BTreeMap<String, Option<Put>>, usize), SaveError> {
    let copies = writeout::keep_safety_copies(repo, actions, base, drive, packs)?;
    let kept: HashSet<&str> = copies.iter().map(|c| c.path.as_str()).collect();
    let renamed = writeout::renamed(actions, drive);
    let unstored = HashMap::new();
    let mut saved = Tree::new(repo.store(), &unstored, base_root, EXACT);
    let mut lost = Vec::new();
    for a in actions {
        if let Some(e) = a.expected
            && e.kind != EntryKind::Folder
            && !kept.contains(a.path.as_str())
            && !saved
                .entry(&a.path)?
                .is_some_and(|s| s.kind == e.kind && s.id == e.id)
        {
            lost.push(a.path.as_str());
        }
    }
    let lost = goes_with(lost, drive);
    let held = actions
        .iter()
        .filter(|a| !lost(&a.path))
        .map(|a| (a.path.clone(), held_at(a, &renamed)))
        .collect();
    Ok((held, copies.len()))
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
    overlay_with(store, base_root, changes, &mut |listing| {
        store_listing(store, packs, listing)
    })
}

/// [`overlay`], with each listing it builds handed to `keep`, which gives its ID.
fn overlay_with(
    store: &Store,
    base_root: Id,
    changes: &BTreeMap<String, Option<Put>>,
    keep: &mut dyn FnMut(&Listing) -> Result<Id, SaveError>,
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
        let id = keep(&listing)?;
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
    keep(&listing)
}

/// What the folder held at the path `a` writes, as the save of what it held has it. A write
/// [`writeout::renamed`] names held nothing under its own spelling.
fn held_at(a: &Action, renamed: &HashSet<&str>) -> Option<Put> {
    a.expected
        .filter(|_| !renamed.contains(a.path.as_str()))
        .map(|e| (e.kind, e.id, e.size))
}

/// Whether a path goes with an entry left at one of `left`: the entry itself, what is planned
/// below it, or a folder above it, which can't be emptied while it stays. Paths are compared as
/// `drive` compares names, since a write under another spelling of a left entry would replace it.
pub(crate) fn goes_with<'a>(
    left: impl IntoIterator<Item = &'a str>,
    drive: Drive,
) -> impl Fn(&str) -> bool {
    let at: HashSet<String> = left
        .into_iter()
        .map(|l| writeout::fold_path(l, drive))
        .collect();
    let above: HashSet<String> = at
        .iter()
        .flat_map(|l| ancestors(l))
        .map(str::to_owned)
        .collect();
    move |path: &str| {
        let path = writeout::fold_path(path, drive);
        at.contains(&path) || above.contains(&path) || ancestors(&path).any(|up| at.contains(up))
    }
}

/// The paths of the unsaved links `actions` would throw away that write-out could bring back
/// only as plain files: judged by its own rule against the tree the kept save would have, which
/// each link left changes. A link no longer as the plan found it is the switch's to leave; a file
/// holding exactly its target is still that link, as the switch takes it. Whether links can be
/// made is tried once.
pub(crate) fn links_to_leave(
    repo: &Repository,
    actions: &[Action],
    base: Id,
    drive: Drive,
) -> Result<Vec<String>, SaveError> {
    let store = repo.store();
    let base_root = store.save(&base, repo.signatures_allowed())?.root;
    let unstored = HashMap::new();
    let mut saved = Tree::new(store, &unstored, base_root, EXACT);
    let mut disk = Folders::open(repo.root()).map_err(|source| SaveError::Unreadable {
        path: "the project folder".to_owned(),
        source,
    })?;
    // A write that takes over a removed spelling found that entry there: it is named once, there.
    let renamed = writeout::renamed(actions, drive);
    let mut links = Vec::new();
    for a in actions {
        let Some(e) = a.expected.filter(|e| e.kind == EntryKind::Symlink) else {
            continue;
        };
        if renamed.contains(a.path.as_str()) {
            continue;
        }
        if saved
            .entry(&a.path)?
            .is_some_and(|s| s.kind == e.kind && s.id == e.id)
        {
            continue;
        }
        if let Some(target) = writeout::planned_link(&mut disk, &a.path, e)? {
            links.push((a.path.as_str(), target));
        }
    }
    if links.is_empty() {
        return Ok(Vec::new());
    }
    if !writeout::can_make_links(repo)? {
        return Ok(links.into_iter().map(|(path, _)| path.to_owned()).collect());
    }
    let left = placeholders(store, base_root, actions, &renamed, &links, drive)?;
    Ok(left.into_iter().collect())
}

/// Of `links`, the unsaved links `actions` throw away with their targets, the ones write-out
/// would write as plain files in the kept save. Each round judges them against the tree that save
/// would have without the links already left, until a round leaves no more.
fn placeholders(
    store: &Store,
    base_root: Id,
    actions: &[Action],
    renamed: &HashSet<&str>,
    links: &[(&str, String)],
    drive: Drive,
) -> Result<BTreeSet<String>, SaveError> {
    let mut left = BTreeSet::new();
    loop {
        let held: BTreeMap<String, Option<Put>> = {
            let goes = goes_with(left.iter().map(String::as_str), drive);
            actions
                .iter()
                .filter(|a| !goes(&a.path))
                .map(|a| (a.path.clone(), held_at(a, renamed)))
                .collect()
        };
        let mut listings = HashMap::new();
        let root = overlay_with(store, base_root, &held, &mut |listing| {
            let id = listing.id();
            listings.insert(id, listing.clone());
            Ok(id)
        })?;
        let mut kept = Tree::new(store, &listings, root, EXACT);
        let mut more = Vec::new();
        for (path, target) in links {
            if !left.contains(*path)
                && !writeout::writes_as_link(&mut kept, path, target.as_bytes())?
            {
                more.push((*path).to_owned());
            }
        }
        if more.is_empty() {
            return Ok(left);
        }
        left.extend(more);
    }
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
    use crate::id::hash_content;
    use crate::journal::{Change, Expected};
    use crate::pack::PackWriter;
    use crate::scope::within;
    use crate::snapshot::{SaveOutcome, SaveRequest, save_folder};
    use crate::switch::{DiscardOutcome, discard};
    use crate::test_support::{plant_save, symlink};
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
            message: Some("saved"),
            who: &me,
            command: "save",
            accept_crlf: false,
            git_excludes: None,
            new_line: None,
            only: None,
            amend: false,
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
        let (change, _) =
            switch_saves(repo, &plan, left, target, &from, drive, &mut packs).unwrap();
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

    #[test]
    fn a_new_folder_left_for_an_ignored_file_is_a_folder_on_both_sides() {
        let ignore: &[u8] = b"*.log\n";
        let (dir, mut repo, save) = saved(&[("a.txt", b"a"), (".gitignore", ignore)]);
        let root = dir.path();
        write_all(root, &[("n/x.txt", b"x"), ("n/keep.log", b"log")]);
        let done = discarded(&mut repo, None);
        assert!(!root.join("n/x.txt").exists());
        assert_eq!(fs::read(root.join("n/keep.log")).unwrap(), b"log");
        // `n` stays for the ignored file, so the "to" save holds it, empty.
        let change = done.folder_change.unwrap();
        let to = repo.store().save(&change.to, false).unwrap();
        assert_eq!(to.parents, [save]);
        let after = holding(&[
            (".gitignore", FILE, ignore),
            ("a.txt", FILE, b"a"),
            ("n", FOLDER, b""),
        ]);
        assert_eq!(tree_of(&repo, change.to), after);
        let before = holding(&[
            (".gitignore", FILE, ignore),
            ("a.txt", FILE, b"a"),
            ("n", FOLDER, b""),
            ("n/x.txt", FILE, b"x"),
        ]);
        assert_eq!(tree_of(&repo, change.from), before);
        // A shelf throws it away the same way.
        write_all(root, &[("n/x.txt", b"x")]);
        crate::shelf::shelf(&mut repo, None, &who(), None, &mut |_| {}).unwrap();
        let change = repo.current().unwrap().1.folder_change.unwrap();
        assert_eq!(tree_of(&repo, change.to), after);
        assert_eq!(tree_of(&repo, change.from), before);
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
    fn work_that_matched_the_target_is_kept_on_the_from_side() {
        let (dir, mut repo, left) = saved(&[("a.txt", b"a"), ("gone/x.txt", b"x")]);
        let target = planted(&mut repo, &[("a.txt", b"A"), ("new/b.txt", b"b")]);
        write_all(dir.path(), &[("a.txt", b"A")]);
        let change = goto_saves(&mut repo, left, target).0.unwrap();
        // The folder after the goto is the target, the matched a.txt included; before it, it
        // held that a.txt already, so an undo leaves it be.
        assert_eq!(change.to, target);
        assert_ne!(change.from, left);
        let kept = repo.store().save(&change.from, false).unwrap();
        assert_eq!((kept.parents, kept.message.as_str()), (vec![left], "goto"));
        let want = holding(&[
            ("a.txt", FILE, b"A"),
            ("gone", FOLDER, b""),
            ("gone/x.txt", FILE, b"x"),
        ]);
        assert_eq!(tree_of(&repo, change.from), want);
    }

    #[test]
    fn what_a_goto_leaves_unwritten_stays_out_of_both_sides() {
        let (_dir, mut repo, left) = saved(&[("a.txt", b"a")]);
        let target = planted(&mut repo, &[("a.txt", b"A"), (".git/hooks/x", b"x")]);
        let change = goto_saves(&mut repo, left, target).0.unwrap();
        // `.git` is never written, so the folder after the goto doesn't hold it.
        assert_eq!(change.from, left);
        assert_eq!(tree_of(&repo, change.to), holding(&[("a.txt", FILE, b"A")]));
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
        // `newdir` stays for the link left in it, so the "to" save holds it, empty.
        let to = repo.store().save(&change.to, false).unwrap();
        assert_eq!(to.parents, [save]);
        let after = holding(&[("a.txt", FILE, b"a"), ("newdir", FOLDER, b"")]);
        assert_eq!(tree_of(&repo, change.to), after);
        let want = holding(&[
            ("a.txt", FILE, b"mine"),
            ("inside", EntryKind::Symlink, b"a.txt"),
            ("newdir", FOLDER, b""),
            ("newdir/x.txt", FILE, b"x"),
        ]);
        assert_eq!(tree_of(&repo, change.from), want);
    }

    #[cfg(unix)]
    #[test]
    fn discard_leaves_a_link_whose_way_runs_through_what_the_kept_save_lacks() {
        use std::os::unix::fs::symlink;
        let (dir, mut repo, _) = saved(&[("a.txt", b"a")]);
        let root = dir.path();
        write_all(root, &[("bin/x", b"x"), ("lib/real.so", b"so")]);
        symlink("/opt/node", root.join("node")).unwrap();
        symlink("../node/bin/tool", root.join("bin/tool")).unwrap();
        symlink("lib/real.so", root.join("current")).unwrap();
        // Alone, current would come back without lib.
        let done = discarded(&mut repo, Some(&["current".to_owned()]));
        assert_eq!(done.links_left, ["current"]);
        assert_eq!(done.folder_change, None);
        assert!(fs::read_link(root.join("current")).is_ok());
        // With everything, lib comes back with it; node leads out, and bin/tool goes through node.
        let done = discarded(&mut repo, None);
        assert_eq!(done.links_left, ["bin/tool", "node"]);
        assert!(fs::read_link(root.join("bin/tool")).is_ok());
        assert!(!root.join("bin/x").exists() && !root.join("lib").exists());
        assert!(root.join("current").symlink_metadata().is_err());
    }

    /// A discard's removal of what the folder holds at `path`: a file or link holding `bytes`, or
    /// a folder.
    fn thrown(path: &str, kind: EntryKind, bytes: &[u8]) -> Action {
        let (change, id, size) = if kind == FOLDER {
            (Change::RemoveFolder, Id([0; 32]), 0)
        } else {
            (Change::RemoveFile, hash_content(bytes), bytes.len() as u64)
        };
        Action {
            path: path.to_owned(),
            change,
            expected: Some(Expected { kind, id, size }),
        }
    }

    #[test]
    fn a_link_the_kept_save_would_write_as_a_file_is_left() {
        let (_dir, repo, save) = saved(&[("a.txt", b"a")]);
        let store = repo.store();
        let base_root = store.save(&save, false).unwrap().root;
        let left = |thrown_away: &[(&str, EntryKind, &[u8])]| -> Vec<String> {
            let actions: Vec<Action> = thrown_away
                .iter()
                .map(|&(path, kind, bytes)| thrown(path, kind, bytes))
                .collect();
            let links: Vec<(&str, String)> = thrown_away
                .iter()
                .filter(|(_, kind, _)| *kind == EntryKind::Symlink)
                .map(|&(path, _, bytes)| (path, String::from_utf8(bytes.to_vec()).unwrap()))
                .collect();
            let none = HashSet::new();
            let left = placeholders(store, base_root, &actions, &none, &links, EXACT).unwrap();
            left.into_iter().collect()
        };
        const LINK: EntryKind = EntryKind::Symlink;
        // node leads out, so it stays a link in the kept save, and bin/tool's way runs through it.
        let node = left(&[
            ("bin", FOLDER, b""),
            ("bin/tool", LINK, b"../node/bin/tool"),
            ("node", LINK, b"/opt/node"),
        ]);
        assert_eq!(node, ["bin/tool", "node"]);
        // The kept save has lib only when lib is thrown away too.
        assert_eq!(left(&[("current", LINK, b"lib/real.so")]), ["current"]);
        let with_lib = left(&[
            ("current", LINK, b"lib/real.so"),
            ("lib", FOLDER, b""),
            ("lib/real.so", FILE, b"so"),
        ]);
        assert!(with_lib.is_empty());
        // Leaving lib/x keeps lib, which then holds nothing else thrown away: current goes too.
        let only_a_link = left(&[
            ("current", LINK, b"lib/x"),
            ("lib", FOLDER, b""),
            ("lib/x", LINK, b"/abs"),
        ]);
        assert_eq!(only_a_link, ["current", "lib/x"]);
    }

    #[test]
    fn what_goes_with_a_left_entry_is_found_as_the_drive_compares_names() {
        let folds = Drive {
            ignores_case: true,
            composes_unicode: false,
        };
        let goes = goes_with(["Dir/Node"], folds);
        for path in ["Dir/Node", "dir/node", "DIR", "dir/NODE/x"] {
            assert!(goes(path), "{path}");
        }
        for path in ["dir/other", "Node", "Dir/Nodes"] {
            assert!(!goes(path), "{path}");
        }
        assert!(!goes_with(["Dir/Node"], EXACT)("dir/node"));
    }

    /// The links a discard of `paths`, or of everything, would leave.
    fn leaving(repo: &mut Repository, save: Id, paths: Option<&[String]>) -> Vec<String> {
        let _lock = repo.lock_for_change("discard", &mut |_| {}).unwrap();
        let drive = drive::probe(repo.tmp_folder()).unwrap();
        let plan = writeout::plan(repo, save, save, paths, true, drive, None).unwrap();
        links_to_leave(repo, &plan.actions, save, drive).unwrap()
    }

    #[test]
    fn links_to_leave_judges_the_links_in_the_folder() {
        let (dir, mut repo, save) = saved(&[("a.txt", b"a")]);
        let root = dir.path();
        write_all(root, &[("bin/x", b"x"), ("lib/real.so", b"so")]);
        if !symlink(&std::env::temp_dir(), &root.join("node")) {
            return;
        }
        let tool = Path::new("..").join("node").join("bin").join("tool");
        assert!(symlink(&tool, &root.join("bin/tool")));
        assert!(symlink(
            &Path::new("lib").join("real.so"),
            &root.join("current")
        ));
        assert_eq!(leaving(&mut repo, save, None), ["bin/tool", "node"]);
        let current = ["current".to_owned()];
        assert_eq!(leaving(&mut repo, save, Some(&current)), ["current"]);
    }

    #[test]
    fn a_placeholder_swapped_in_for_an_unsaved_link_is_judged_as_that_link() {
        let (dir, mut repo, save) = saved(&[("a.txt", b"a")]);
        let target = b"../out";
        // The plan found an unsaved link `l` leading out of the project, which comes back only
        // as a plain file; another program has since swapped it for one holding its target.
        let actions = [Action {
            path: "l".into(),
            change: Change::RemoveFile,
            expected: Some(Expected {
                kind: EntryKind::Symlink,
                id: hash_content(target),
                size: 6,
            }),
        }];
        fs::write(dir.path().join("l"), target).unwrap();
        let _lock = repo.lock_for_change("discard", &mut |_| {}).unwrap();
        let drive = drive::probe(repo.tmp_folder()).unwrap();
        assert_eq!(links_to_leave(&repo, &actions, save, drive).unwrap(), ["l"]);
    }

    #[test]
    fn a_kept_saves_message_is_cut_at_a_character_within_1_kib() {
        let given = |paths: &[&str]| -> Vec<String> { paths.iter().map(|&p| p.into()).collect() };
        let named = message("discard", &given(&["a.txt", "", "dir"]));
        assert_eq!(named, "discard a.txt dir");
        // "discard " and 1,015 bytes come to 1,023, so the two bytes of é straddle the limit.
        let long = format!("{}\u{e9} and more", "x".repeat(1015));
        let cut = message("discard", &given(&[&long]));
        assert_eq!(cut, format!("discard {}", "x".repeat(1015)));
        assert_eq!(cut.len(), 1023);
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
