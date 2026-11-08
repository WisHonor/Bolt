//! Switching the project folder from one save to another (decisions 049 to 052). [`plan`] works
//! out what to change; [`apply`] changes it, a name at a time, through folders opened one below
//! the other without following a link. A save, and the journal made from it, may come from
//! anyone, so every name is checked again right before it reaches the disk.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::ffi::OsStr;
use std::io::{self, Read, Write};
use std::path::Path;

use bolt_platform::dir::{Dir, EntryMeta, EntryType, is_link_refusal};

use crate::chunks::{is_cut, is_too_long_to_cut};
use crate::diary::DiaryEntry;
use crate::diff::{for_each_change, for_each_change_in, for_each_entry_change_in};
use crate::drive::{self, Drive};
use crate::filecache::FileCache;
use crate::fsio::{self, BETWEEN_TRIES, Done, TRIES};
use crate::history::{current_save, tip_of};
use crate::id::{ContentHasher, Id, hash_content};
use crate::ingest::{self, IngestError};
use crate::journal::{self, Action, Change, Content, Expected, Journal};
use crate::link;
use crate::listing::{Entry, EntryKind, Listing};
use crate::names::{MAX_PATH_BYTES, is_bolt_alias, is_git_alias, validate_path, writable_here};
use crate::pack::{ObjectKind, PackError, PackSet};
use crate::repo::{ChangeLock, RepoError, Repository, damaged};
use crate::scope::{Scope, ancestors};
use crate::snapshot::{NAMED, SaveError, ingest_failed, listed, read_folder, same_file};
use crate::store::{Store, StoreError};
use crate::warning::{self, Warning};

/// A switch worked out, before anything changes.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Plan {
    /// What to change, sorted by path, as the journal holds it.
    pub actions: Vec<Action>,
    /// Entries of the target this computer can't hold, or that are one file with another here:
    /// never touched. A folder stands for everything in it.
    pub not_written: Vec<String>,
    /// Links written as files holding their targets, since they lead out of the project.
    pub placeholders: Vec<String>,
    /// Unsaved work the switch would overwrite or remove, which it leaves out.
    pub collisions: Vec<String>,
    /// Where the target differs from `from` and the folder already holds the target's version,
    /// all that is below included: nothing to write there. The top such paths only.
    pub matched: Vec<String>,
    /// Unsaved changes the folder holds, which stay as they are unless they collide.
    pub carried: usize,
    /// What reading the folder found wrong.
    pub warnings: Vec<Warning>,
}

/// What carrying out a journal did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Applied {
    /// Files and links written, placeholders included.
    pub written: usize,
    /// Files, links and folders removed.
    pub removed: usize,
    /// Entries another program held open, left for the next run.
    pub in_use: Vec<String>,
    /// Entries that weren't what the plan found, left as they are. A folder stands for
    /// everything below it.
    pub changed_since_plan: Vec<String>,
    /// Links written as files holding their targets because this computer couldn't make them.
    pub placeholders: Vec<String>,
    /// Names this system mustn't write that the journal held, left unwritten.
    pub not_written: Vec<String>,
    pub warnings: Vec<Warning>,
}

/// The longest link target written as a link; a longer one is a placeholder. Linux's own limit.
const MAX_LINK_TARGET: u64 = MAX_PATH_BYTES as u64;

/// Works out how to switch the folder from the save `from`, the one it is on, to the save `to`.
/// The folder is read as `status` reads it. Unsaved work (a difference between the folder and
/// `from`) where `to` differs from `from`, at the same path or a folder above or below it, and
/// that doesn't already match `to`, is a collision and left out; the rest of it stays as it is.
/// With `remove_new`, the folder is made to match `to` exactly instead, new files removed, and
/// nothing collides. `only` limits the switch to those paths and what is in them.
pub fn plan(
    repo: &Repository,
    from: Id,
    to: Id,
    only: Option<&[String]>,
    remove_new: bool,
    drive: Drive,
    git_excludes: Option<&Path>,
) -> Result<Plan, SaveError> {
    planned(repo, from, to, only, remove_new, drive, git_excludes)
        .map_err(|e| told_whose(repo, e, from, to))
}

/// `e`, a spelling of `.bolt` the plan met past reading the folder against `from`, told as the
/// save that holds it: `from` if its whole tree meets one, as every switch from it would, else
/// `to`.
fn told_whose(repo: &Repository, e: SaveError, from: Id, to: Id) -> SaveError {
    if !matches!(e, SaveError::Repo(RepoError::BoltName { save: None, .. })) {
        return e;
    }
    let holds = || -> Result<(), RepoError> {
        let root = repo.store().save(&from, repo.signatures_allowed())?.root;
        for_each_change(repo.store(), None, root, |_| Ok::<(), RepoError>(()))
    };
    let named = match holds() {
        Err(RepoError::BoltName { .. }) => from,
        _ => to,
    };
    e.in_save(named, is_current(repo, named))
}

/// Whether the project is on `save`: an unshelf plans from the shelf's base, and an undo step
/// can plan from a kept save.
fn is_current(repo: &Repository, save: Id) -> bool {
    current_save(repo).is_ok_and(|on| on == save)
}

fn planned(
    repo: &Repository,
    from: Id,
    to: Id,
    only: Option<&[String]>,
    remove_new: bool,
    drive: Drive,
    git_excludes: Option<&Path>,
) -> Result<Plan, SaveError> {
    let store = repo.store();
    let from_root = store.save(&from, repo.signatures_allowed())?.root;
    let to_root = store.save(&to, repo.signatures_allowed())?.root;
    let now = read_folder(repo, from_root, false, git_excludes)
        .map_err(|e| e.in_save(from, is_current(repo, from)))?;
    let mut unsaved = BTreeSet::new();
    for_each_change_in(store, &now.listings, Some(from_root), now.root, |c| {
        unsaved.insert(c.path.to_owned());
        Ok::<(), SaveError>(())
    })?;
    // Work that already matches the target is in nothing's way, at a file or below a folder
    // (decision 065), and once there it is no longer unsaved.
    if !remove_new {
        let mut unlike_target = BTreeSet::new();
        for_each_change_in(store, &now.listings, Some(to_root), now.root, |c| {
            unlike_target.insert(c.path.to_owned());
            Ok::<(), SaveError>(())
        })?;
        unsaved.retain(|p| unlike_target.contains(p));
    }
    let carried = unsaved.len();
    let stored = HashMap::new();
    // Where the target differs from `from`, and, of those, where it holds an entry to write.
    let (mut switched, mut written) = (BTreeSet::new(), BTreeSet::new());
    for_each_entry_change_in(store, &stored, Some(from_root), to_root, |path, _, t| {
        switched.insert(path.to_owned());
        if t.is_some() {
            written.insert(path.to_owned());
        }
        Ok::<(), SaveError>(())
    })?;

    // Compared as the drive compares names, so a path given in another case still matches.
    let only = only.map(|o| Scope::paths(o, drive));
    let mut target = Tree::new(store, &stored, to_root, drive);
    let mut folder = Tree::new(store, &now.listings, now.root, drive);
    let mut refused = BTreeSet::new();
    let mut plan = Plan {
        carried,
        warnings: now.warnings,
        ..Plan::default()
    };
    // Every path where the folder differs from the target.
    let mut visited = BTreeSet::new();
    for_each_entry_change_in(
        store,
        &now.listings,
        Some(now.root),
        to_root,
        |path, n, t| {
            visited.insert(path.to_owned());
            if !in_scope(path, only.as_ref(), t) {
                return Ok(());
            }
            if let Some(at) = refused_at(path, &mut target, &mut folder)? {
                refused.insert(at);
                return Ok(());
            }
            // Project links and what their folders hold are never touched (decision 050).
            if [n, t].into_iter().flatten().any(not_ours)
                || below_project_link(path, &mut target, &mut folder)?
            {
                return Ok(());
            }
            if !remove_new {
                if !switched.contains(path) {
                    // A folder the target holds and the user removed is made again when the
                    // switch writes something of the target's into it; making it overwrites
                    // nothing. Removals below it don't count: the folder stays gone.
                    let needed = n.is_none()
                        && t.is_some_and(|e| e.kind == EntryKind::Folder)
                        && any_below(path, &written);
                    if !needed {
                        return Ok(()); // unsaved work the switch doesn't reach comes along
                    }
                } else if collides(path, &unsaved) {
                    plan.collisions.push(path.to_owned());
                    return Ok(());
                }
            }
            let expected = n.map(expected_of);
            let change = match t {
                None if n.is_some_and(|e| e.kind == EntryKind::Folder) => Change::RemoveFolder,
                None => Change::RemoveFile,
                Some(e) => {
                    let c = Content {
                        id: e.id,
                        size: e.size,
                    };
                    match e.kind {
                        EntryKind::Folder => Change::MakeFolder,
                        EntryKind::Executable => Change::Executable(c),
                        EntryKind::Symlink => {
                            let shape = match link_target(store, c)? {
                                Some(bytes) => link_shape(&mut target, path, &bytes)?,
                                None => None,
                            };
                            if shape.is_some() {
                                Change::Link(c)
                            } else {
                                plan.placeholders.push(path.to_owned());
                                Change::File(c)
                            }
                        }
                        _ => Change::File(c),
                    }
                }
            };
            plan.actions.push(Action {
                path: path.to_owned(),
                change,
                expected,
            });
            Ok::<(), SaveError>(())
        },
    )?;
    // Where the target differs from `from` but not from the folder, at the path and below it.
    let matched: BTreeSet<&str> = switched
        .iter()
        .filter(|p| !visited.contains(*p) && !any_below(p, &visited))
        .map(String::as_str)
        .collect();
    plan.matched = matched
        .iter()
        .filter(|p| !ancestors(p).any(|up| matched.contains(up)))
        .map(|p| (*p).to_owned())
        .collect();
    plan.actions.sort_by(|a, b| a.path.cmp(&b.path));
    if drive.ignores_case || drive.composes_unicode {
        take_over_renamed(&mut plan.actions, drive);
    }
    // Something new only to the plan, such as an ignored file, is in the way of a new entry.
    let mut disk = Folders::open(repo.root()).map_err(|e| unreadable("the project folder", e))?;
    let mut kept = Vec::with_capacity(plan.actions.len());
    for action in std::mem::take(&mut plan.actions) {
        let new_here = action.expected.is_none() && !removes(action.change);
        if new_here && in_the_way(&mut disk, &mut folder, &action.path)? {
            plan.collisions.push(action.path);
        } else {
            kept.push(action);
        }
    }
    // What the plan never read below a folder that goes (an ignored file, a nested project) is
    // in the way of a file or link put in its place. Where the target just removes the folder,
    // it stays for what is left, as Git leaves it.
    let mut blocked = Vec::new();
    let mut stays = HashSet::new();
    {
        let removed: HashSet<&str> = kept
            .iter()
            .filter(|a| removes(a.change))
            .map(|a| a.path.as_str())
            .collect();
        // Spelled as the drive compares names, since the walk meets them as they are on disk.
        let folded: HashSet<String> = removed.iter().map(|p| fold_path(p, drive)).collect();
        let goes: HashSet<&str> = kept
            .iter()
            .filter(|a| {
                a.expected.is_some_and(|e| e.kind == EntryKind::Folder)
                    && !matches!(a.change, Change::MakeFolder)
            })
            .map(|a| a.path.as_str())
            .collect();
        for &path in &goes {
            // The walk of the folder above already covers this one.
            if ancestors(path).any(|up| goes.contains(up)) {
                continue;
            }
            let left = left_below(&mut disk, path, &folded, drive)?;
            if left.is_empty() {
                continue;
            }
            if removed.contains(path) {
                for l in &left {
                    let up = ancestors(l).chain([l.as_str()]);
                    stays.extend(up.filter(|f| goes.contains(f)).map(str::to_owned));
                }
            } else {
                plan.collisions.extend(left);
                blocked.push(path.to_owned());
            }
        }
    }
    kept.retain(|a| !(a.change == Change::RemoveFolder && stays.contains(&a.path)));
    // Nothing is planned below a folder left out, or it would be made inside what is in the way.
    let left_out: HashSet<&str> = plan
        .collisions
        .iter()
        .chain(&blocked)
        .map(String::as_str)
        .collect();
    kept.retain(|a| {
        !left_out.contains(a.path.as_str()) && !ancestors(&a.path).any(|up| left_out.contains(up))
    });
    plan.actions = kept;
    plan.collisions.sort_unstable();
    plan.collisions.dedup();
    plan.not_written = refused.into_iter().collect();
    Ok(plan)
}

/// The paths of the entries of the save `save` that `drive` takes to be `path`: none, the one,
/// or, where the save holds names the drive takes for one, several.
pub fn spellings(
    repo: &Repository,
    save: Id,
    path: &str,
    drive: Drive,
) -> Result<Vec<String>, SaveError> {
    let store = repo.store();
    let root = store.save(&save, repo.signatures_allowed())?.root;
    let stored = HashMap::new();
    let mut tree = Tree::new(store, &stored, root, drive);
    let mut found = vec![String::new()];
    for name in path.split('/') {
        let wanted = drive.compared(name);
        let mut next = Vec::new();
        for up in &found {
            let Some(folder) = tree.folder(up)? else {
                continue;
            };
            for e in folder.listing.entries() {
                if drive.compared(&e.name) == wanted {
                    next.push(if up.is_empty() {
                        e.name.clone()
                    } else {
                        format!("{up}/{}", e.name)
                    });
                }
            }
        }
        found = next;
    }
    Ok(found)
}

/// A file or link Bolt kept before overwriting or removing it, so the command can be undone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SafetyCopy {
    pub path: String,
    pub kind: EntryKind,
    pub id: Id,
    pub size: u64,
}

/// Stores in `packs` what carrying out `actions` would overwrite or remove that the save `from`
/// doesn't hold at the same path, and lists it by path: the safety copies (decision 050). An
/// entry that is no longer what the plan found is left by the switch, so it needs none.
pub fn keep_safety_copies(
    repo: &Repository,
    actions: &[Action],
    from: Id,
    drive: Drive,
    packs: &mut PackSet,
) -> Result<Vec<SafetyCopy>, SaveError> {
    let store = repo.store();
    let from_root = store.save(&from, repo.signatures_allowed())?.root;
    let stored = HashMap::new();
    let mut saved = Tree::new(store, &stored, from_root, drive);
    let mut disk = Folders::open(repo.root()).map_err(|e| unreadable("the project folder", e))?;
    let mut copies = Vec::new();
    for a in actions {
        let Some(e) = a.expected else { continue };
        if !matches!(
            e.kind,
            EntryKind::File | EntryKind::Executable | EntryKind::Symlink
        ) || saved
            .entry(&a.path)?
            .is_some_and(|s| s.kind == e.kind && s.id == e.id)
        {
            continue;
        }
        // A big file is relied on as held only once it is cut again, which `copy_in` does.
        let held = packs.contains(&e.id)
            || (!is_cut(e.size) && store.has_good_copy(&e.id, ObjectKind::Content, e.size)?);
        if !held && !copy_in(&mut disk, &a.path, e, store, packs)? {
            continue;
        }
        copies.push(SafetyCopy {
            path: a.path.clone(),
            kind: e.kind,
            id: e.id,
            size: e.size,
        });
    }
    copies.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(copies)
}

/// Stores the entry at `path` if it still holds `e`; `false` if it doesn't.
fn copy_in(
    disk: &mut Folders,
    path: &str,
    e: Expected,
    store: &Store,
    packs: &mut PackSet,
) -> Result<bool, SaveError> {
    let (up, name) = split(path);
    match disk.enter(up) {
        Ok(()) => {}
        Err(err) if folder_changed(&err) => return Ok(false),
        Err(err) => return Err(unreadable(path, err)),
    }
    let dir = disk.here();
    if e.kind == EntryKind::Symlink {
        let Some(target) = link_in(dir, name, path, e)? else {
            return Ok(false);
        };
        ingest::store_held(packs, store, target.as_bytes())
            .map_err(|err| ingest_failed(err, path))?;
        return Ok(true);
    }
    let os = OsStr::new(name);
    let mut file = match dir.open_file(os) {
        Ok(file) => file,
        // Gone, or no longer what it was: the switch leaves what is there.
        Err(_)
            if !dir
                .metadata(os)
                .is_ok_and(|now| now.kind == EntryType::File) =>
        {
            return Ok(false);
        }
        Err(err) => return Err(unreadable(path, err)),
    };
    if is_too_long_to_cut(e.size) {
        // Stored whole, and compared with the plan's ID as it is read.
        return match packs.add_content_from(&mut file, e.id) {
            Ok(_) => Ok(true),
            Err(PackError::ContentChanged) => Ok(false),
            Err(err) => Err(err.into()),
        };
    }
    // A file that isn't what the plan found any more is left as it is, like a changed one.
    match ingest::store_bytes(packs, store, &mut file, e.size) {
        Ok(id) => Ok(id == e.id),
        Err(IngestError::Changed) => Ok(false),
        Err(err) => Err(ingest_failed(err, path)),
    }
}

/// The target of the link at `path` if it is still the one the plan found (`e`), or a file
/// holding exactly that target; `None` if it is gone or changed since. Anything else that stops
/// it being read is an error.
pub(crate) fn planned_link(
    disk: &mut Folders,
    path: &str,
    e: Expected,
) -> Result<Option<String>, SaveError> {
    let (up, name) = split(path);
    match disk.enter(up) {
        Ok(()) => {}
        Err(err) if folder_changed(&err) => return Ok(None),
        Err(err) => return Err(unreadable(path, err)),
    }
    link_in(disk.here(), name, path, e)
}

/// Whether entering the folder above a planned path failed because it changed since the plan
/// (gone, a file now, or a link), so the switch leaves what is there. Any other error stops it.
fn folder_changed(err: &io::Error) -> bool {
    is_link_refusal(err)
        || matches!(
            err.kind(),
            io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
        )
}

/// The target of the link `name` in `dir`, at the project path `path`, if it is still the one
/// the plan found (`e`) or a file holding exactly that target, which the run takes for it;
/// `None` if it is neither.
fn link_in(dir: &Dir, name: &str, path: &str, e: Expected) -> Result<Option<String>, SaveError> {
    let os = OsStr::new(name);
    let target = match crate::snapshot::link_target(dir, name, path) {
        Ok(target) => target,
        Err(err)
            if dir
                .metadata(os)
                .is_ok_and(|now| now.kind == EntryType::Symlink) =>
        {
            return Err(err);
        }
        Err(_) => {
            let file = match dir.open_file(os) {
                Ok(file) => file,
                Err(_)
                    if !dir
                        .metadata(os)
                        .is_ok_and(|now| now.kind == EntryType::File) =>
                {
                    return Ok(None);
                }
                Err(err) => return Err(unreadable(path, err)),
            };
            let mut bytes = Vec::new();
            file.take(e.size.saturating_add(1))
                .read_to_end(&mut bytes)
                .map_err(|err| unreadable(path, err))?;
            match String::from_utf8(bytes) {
                Ok(text) => text,
                Err(_) => return Ok(None),
            }
        }
    };
    Ok((hash_content(target.as_bytes()) == e.id).then_some(target))
}

/// Carries out `journal`, made by [`plan`] or read back after a crash. Each change is checked
/// against the folder right before it is made: what is there must be what the plan found, or
/// already what the switch puts there, which is then left (so a journal carried out twice
/// changes nothing more). Anything else is left as it is and listed. Folders are made first,
/// top down; then files; then removals, deepest first; links last. `drive` is the drive's own
/// answer (`drive::probe`), the one the plan was made with: which names it takes for one is
/// that drive's business, and a journal finished on another drive is judged by that one.
pub fn apply(
    repo: &Repository,
    _lock: &ChangeLock,
    journal: &Journal,
    drive: Drive,
) -> Result<Applied, RepoError> {
    let mut run = Run {
        repo,
        folders: Folders::open(repo.root())?,
        cache: FileCache::load(repo),
        target_entry: journal.target,
        stored: HashMap::new(),
        target_root: None,
        short_names: HashMap::new(),
        dirty: BTreeSet::new(),
        applied: Applied::default(),
        refused: BTreeSet::new(),
        changed: BTreeSet::new(),
        placeholders: Vec::new(),
    };
    let mut actions: Vec<&Action> = Vec::new();
    for action in &journal.actions {
        match refused_name(&action.path) {
            Some(at) => {
                run.refused.insert(at);
            }
            None => actions.push(action),
        }
    }
    actions.sort_by(|a, b| a.path.cmp(&b.path));
    // On a drive that folds names, a write that takes over a removed name's file (`Readme.md`
    // to `README.md`) is a rename: the removal is skipped, or it would remove the new file.
    let folds = drive.ignores_case || drive.composes_unicode;
    let folded = |path: &str| fold_path(path, drive);
    let mut written_at: HashMap<String, &str> = HashMap::new();
    let mut removed_at: HashMap<String, &str> = HashMap::new();
    if folds {
        for a in &actions {
            let map = if removes(a.change) {
                &mut removed_at
            } else {
                &mut written_at
            };
            map.insert(folded(&a.path), &a.path);
        }
    }
    // The other spelling of `path` in `map`, if the drive takes it for the same name.
    let other = |map: &HashMap<String, &str>, path: &str| -> Option<String> {
        let p = map.get(&folded(path))?;
        (folds && *p != path).then(|| (*p).to_owned())
    };
    let holds_folder = |a: &Action| a.expected.is_some_and(|e| e.kind == EntryKind::Folder);

    for a in actions.iter().filter(|a| a.change == Change::MakeFolder) {
        run.step(a, |run| run.make_folder(a))?;
    }
    for a in actions.iter().filter(|a| !holds_folder(a)) {
        if let Change::File(c) | Change::Executable(c) = a.change {
            let renaming = other(&removed_at, &a.path);
            run.step(a, |run| run.put(a, c, renaming.as_deref()))?;
        }
    }
    for a in actions.iter().rev() {
        match a.change {
            Change::RemoveFile | Change::RemoveFolder => {
                // The old spelling of a name just written is that entry where the drive folds
                // names, and is left; where a folder keeps case apart it is a file of its own.
                let written = other(&written_at, &a.path);
                if written.is_none_or(|w| !run.one_entry_at(&a.path, &w)) {
                    run.step(a, |run| run.remove(a))?;
                }
            }
            Change::File(c) | Change::Executable(c) if holds_folder(a) => {
                run.step(a, |run| run.put(a, c, None))?;
            }
            _ => {}
        }
    }
    for a in &actions {
        if let Change::Link(c) = a.change {
            run.step(a, |run| run.link(a, c))?;
        }
    }
    run.finish()
}

/// Carries out `journal`, then, unless another program held a file open, moves `head` to its
/// entry and removes it: the switch has then happened, and what goes wrong after that is only a
/// warning. With files in use the journal stays and `head` doesn't move, so the next command
/// finishes the rest.
pub fn carry_out(
    repo: &mut Repository,
    lock: &ChangeLock,
    journal: &Journal,
    drive: Drive,
) -> Result<Applied, RepoError> {
    let done = apply(repo, lock, journal, drive)?;
    if done.in_use.is_empty() {
        repo.move_head_to(lock, journal.target)?;
        match Journal::remove(repo, lock) {
            Ok(Done::Flushed) => {}
            // The head's own flush came first, so a power cut can only bring the journal back.
            Ok(Done::NotFlushed(e)) => repo.store().note(Warning::new(
                warning::HEAD_NOT_FLUSHED,
                format!(
                    "The switch between saves is done, but Bolt couldn't flush .bolt/state to the \
                     disk after removing its journal ({}): after a power cut the journal may be \
                     back, and the next command that changes the project removes it.",
                    e.to_string().trim_end_matches('.')
                ),
            )),
            Err(e) => repo.store().note(Warning::new(
                warning::SWITCH_UNFINISHED,
                format!(
                    "The switch between saves is done, but Bolt couldn't remove its journal, \
                     .bolt/state/journal ({}): the next command that changes the project removes \
                     it first.",
                    e.to_string().trim_end_matches('.')
                ),
            )),
        }
    }
    Ok(done)
}

/// Finishes a switch that a crash or a stop left in `state/journal`, before a command changes
/// anything: never rolled back. `None` when there was none. When `head` already
/// names its entry, only the journal was left: it is removed, and what was done is empty. A
/// switch that still can't finish is an error, the journal kept, so no command builds on a
/// half-switched folder. A lost `head` leaves the journal as it is, for the command to stop at.
pub fn finish_interrupted(
    repo: &mut Repository,
    lock: &ChangeLock,
) -> Result<Option<Applied>, RepoError> {
    let Some(journal) = Journal::read(repo)? else {
        return Ok(None);
    };
    let head = match repo.current() {
        Ok((head, _)) => head,
        // Nothing can be judged against it; only `undo --to` goes on, and changes no file.
        Err(RepoError::HeadLost { .. }) => return Ok(None),
        Err(e) => return Err(e),
    };
    if head == journal.target {
        Journal::remove_before_head(repo, lock)?;
        return Ok(Some(Applied::default()));
    }
    usable(repo, journal.target, journal.from, head)?;
    let drive = drive::probe(repo.tmp_folder())?;
    let mut done = carry_out(repo, lock, &journal, drive)?;
    if !done.in_use.is_empty() {
        return Err(RepoError::SwitchInUse {
            detail: in_use_warning(&done.in_use).message,
        });
    }
    done.warnings.push(Warning::new(
        warning::SWITCH_FINISHED,
        format!(
            "Finished a switch between saves that was stopped before it was done: wrote {} and \
             removed {}.",
            counted(done.written, "file", "files"),
            counted(done.removed, "entry", "entries")
        ),
    ));
    Ok(Some(done))
}

/// Whether the switch from `from` to `target` can be carried out on this project, whose `head` is
/// `head`: made from that entry, to one that reads. Otherwise it is no switch of this folder to finish.
fn usable(repo: &Repository, target: Id, from: Id, head: Id) -> Result<(), RepoError> {
    if head != from {
        return Err(journal::unusable(
            "was made for another save than the one the project is on",
        ));
    }
    let entry = repo
        .store()
        .read_record(&target, ObjectKind::DiaryEntry)
        .map_err(RepoError::from)
        .and_then(|bytes| Ok(DiaryEntry::decode(&bytes)?));
    match entry {
        Ok(_) => Ok(()),
        Err(e) => Err(journal::unusable(format!(
            "names a diary entry that can't be read ({e})"
        ))),
    }
}

/// For commands that only read: says a switch between saves is still to finish, or stopped just
/// after it was done, or that its journal can't be used.
#[must_use]
pub fn unfinished_warning(repo: &Repository) -> Option<Warning> {
    // Whether the switch took effect, when there is one.
    let pending = Journal::read_front(repo).and_then(|found| match found {
        None => Ok(None),
        Some((target, from)) => {
            let (head, _) = repo.current()?;
            let took_effect = head == target;
            if !took_effect {
                usable(repo, target, from, head)?;
            }
            Ok(Some(took_effect))
        }
    });
    let message = match pending {
        // A lost head is what the command tells.
        Ok(None) | Err(RepoError::HeadLost { .. }) => return None,
        Ok(Some(true)) => "A switch between saves was stopped just after it was done: its files \
                           are all written, and the next command that changes the project removes \
                           its journal, .bolt/state/journal, first."
            .to_owned(),
        Ok(Some(false)) => "A switch between saves was stopped before it was done, so some files \
                            may still be as they were. Running `bolt goto` or `bolt discard` \
                            again, or any command that changes the project, finishes it first if \
                            its journal reads whole, or else says how to go on."
            .to_owned(),
        Err(e) => format!(
            "{e} Commands that change the project won't run until it is dealt with. {}",
            e.hint().unwrap_or_default()
        ),
    };
    Some(Warning::new(warning::SWITCH_UNFINISHED, message))
}

/// What one change came to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Outcome {
    Wrote,
    /// A folder made, which counts only through what goes in it.
    Made,
    /// Written as a file holding the link's target.
    Placeholder,
    Removed,
    /// Already as the switch leaves it, or nothing to do.
    Nothing,
    /// Not what the plan found, so left.
    Changed,
    /// Its folder isn't there, or isn't a folder any more.
    Gone,
    /// Another program holds it open.
    InUse,
    /// A name this system mustn't write here.
    Refused,
}

/// A journal being carried out.
struct Run<'a> {
    repo: &'a Repository,
    folders: Folders,
    cache: FileCache,
    /// The diary entry the switch commits, whose saves say which links lead to folders.
    target_entry: Id,
    stored: HashMap<Id, Listing>,
    target_root: Option<Id>,
    /// The short names of `.git` and `.bolt` spellings in each folder written in (Windows).
    short_names: HashMap<String, Vec<(String, String)>>,
    /// Folders changed, flushed at the end.
    dirty: BTreeSet<String>,
    applied: Applied,
    refused: BTreeSet<String>,
    changed: BTreeSet<String>,
    placeholders: Vec<String>,
}

impl Run<'_> {
    /// Makes one change and notes what came of it. Nothing below a folder already left as it
    /// was is tried.
    fn step(
        &mut self,
        a: &Action,
        change: impl FnOnce(&mut Self) -> Result<Outcome, RepoError>,
    ) -> Result<(), RepoError> {
        if ancestors(&a.path).any(|up| self.changed.contains(up) || self.refused.contains(up)) {
            return Ok(());
        }
        let path = a.path.clone();
        let outcome = change(self).map_err(|e| RepoError::SwitchStopped {
            path: path.clone(),
            source: Box::new(e),
        })?;
        match outcome {
            Outcome::Wrote => self.applied.written += 1,
            Outcome::Made => {}
            Outcome::Placeholder => {
                self.applied.written += 1;
                self.placeholders.push(path.clone());
            }
            Outcome::Removed => self.applied.removed += 1,
            Outcome::Nothing => return Ok(()),
            Outcome::Changed | Outcome::Gone => {
                self.changed.insert(path);
                return Ok(());
            }
            Outcome::InUse => {
                self.applied.in_use.push(path);
                return Ok(());
            }
            // `enter` noted the name refused.
            Outcome::Refused => return Ok(()),
        }
        self.dirty.insert(parent(&a.path).to_owned());
        fsio::fault_point("switch:changed")?;
        Ok(())
    }

    /// Opens the folder holding `path`, checking every name on the way against what may be
    /// written in its folder, giving the last name, or what to report instead. On Windows each
    /// name is also checked against the real short names of a `.git` or `.bolt` in its folder,
    /// since the drive resolves those names to them.
    fn enter<'p>(&mut self, path: &'p str) -> Result<Result<&'p str, Outcome>, RepoError> {
        let (up, name) = split(path);
        if cfg!(windows) {
            let (folders, known) = (&mut self.folders, &mut self.short_names);
            let refused = first_refused(path, |folder| {
                if let Some(own) = known.get(folder) {
                    return Ok(own.clone());
                }
                folders.enter(folder)?;
                let own = own_short_names(folders.here())?;
                known.insert(folder.to_owned(), own.clone());
                Ok(own)
            });
            match refused {
                Ok(Some(at)) => {
                    self.refused.insert(at.to_owned());
                    return Ok(Err(Outcome::Refused));
                }
                Ok(None) => {}
                Err(e) => return entering_failed(e).map(Err),
            }
        }
        if let Err(e) = self.folders.enter(up) {
            return entering_failed(e).map(Err);
        }
        Ok(Ok(name))
    }

    /// What is at `name` in the folder entered, read only as far as telling it from entries of
    /// the sizes `sizes` needs.
    fn look(&self, name: &str, path: &str, sizes: &[u64]) -> io::Result<Option<Found>> {
        let dir = self.folders.here();
        let os = OsStr::new(name);
        let Some(meta) = self.meta_of(os)? else {
            return Ok(None);
        };
        let kind = match meta.kind {
            EntryType::Dir => {
                return Ok(Some(Found {
                    kind: Some(EntryKind::Folder),
                    size: 0,
                    id: Some(Id([0; 32])),
                    meta,
                }));
            }
            EntryType::Other => {
                return Ok(Some(Found {
                    kind: None,
                    size: meta.size,
                    id: None,
                    meta,
                }));
            }
            EntryType::Symlink => {
                let target = dir
                    .read_link(os)?
                    .into_string()
                    .ok()
                    .map(link::stored_target);
                return Ok(Some(Found {
                    kind: Some(EntryKind::Symlink),
                    size: target.as_ref().map_or(0, |t| t.len() as u64),
                    id: target.map(|t| crate::id::hash_content(t.as_bytes())),
                    meta,
                }));
            }
            EntryType::File if meta.executable == Some(true) => EntryKind::Executable,
            EntryType::File => EntryKind::File,
        };
        let id = if !sizes.contains(&meta.size) {
            None
        } else if let Some(known) = self.cache.matches(path, &meta) {
            Some(known.id)
        } else {
            let mut file = retried(|| dir.open_file(os))?;
            let opened = Dir::file_meta(&file)?;
            if opened.file_id != meta.file_id || opened.size != meta.size {
                None
            } else {
                Some(hash(&mut file)?)
            }
        };
        Ok(Some(Found {
            kind: Some(kind),
            size: meta.size,
            id,
            meta,
        }))
    }

    /// The details of what is at `name` in the folder entered, `None` if nothing is.
    fn meta_of(&self, name: &OsStr) -> io::Result<Option<EntryMeta>> {
        match retried(|| self.folders.here().metadata(name)) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            meta => meta.map(Some),
        }
    }

    fn make_folder(&mut self, a: &Action) -> Result<Outcome, RepoError> {
        let name = match self.enter(&a.path)? {
            Ok(name) => name,
            Err(outcome) => return Ok(outcome),
        };
        let sizes: Vec<u64> = a.expected.iter().map(|e| e.size).collect();
        let found = match self.look(name, &a.path, &sizes) {
            Ok(found) => found,
            Err(e) => return classify(e),
        };
        let os = OsStr::new(name);
        match found {
            Some(f) if f.kind == Some(EntryKind::Folder) => return Ok(Outcome::Nothing),
            None => {}
            Some(f) if f.is(a.expected) => match self.remove_unchanged(os, f.meta) {
                Ok(true) => {}
                Ok(false) => return Ok(Outcome::Changed),
                Err(e) => return classify(e),
            },
            Some(_) => return Ok(Outcome::Changed),
        }
        match self.folders.here().create_dir(os) {
            Ok(()) => Ok(Outcome::Made),
            Err(e) => classify(e),
        }
    }

    /// Writes a file. `renaming`: the path of a file the switch removes whose name differs only
    /// as the drive ignores, which this one takes over; it is written even when its contents are
    /// already there, to take the new spelling.
    fn put(
        &mut self,
        a: &Action,
        c: Content,
        renaming: Option<&str>,
    ) -> Result<Outcome, RepoError> {
        let executable = matches!(a.change, Change::Executable(_));
        let kind = if executable {
            EntryKind::Executable
        } else {
            EntryKind::File
        };
        let name = match self.enter(&a.path)? {
            Ok(name) => name,
            Err(outcome) => return Ok(outcome),
        };
        let mut sizes = vec![c.size];
        sizes.extend(a.expected.map(|e| e.size));
        let found = match self.look(name, &a.path, &sizes) {
            Ok(found) => found,
            Err(e) => return classify(e),
        };
        let os = OsStr::new(name);
        if let Some(f) = &found
            && renaming.is_none()
            && f.size == c.size
            && f.id == Some(c.id)
            && matches!(f.kind, Some(EntryKind::File | EntryKind::Executable))
            && (f.kind == Some(kind) || cfg!(windows))
        {
            return Ok(Outcome::Nothing);
        }
        // Where only the executable bit differs the file is written anew too: changing its mode
        // in place would change it for every other name a hard link gives it, outside included.
        let ours = |f: &Found| f.is(a.expected) || (renaming.is_some() && f.is_content(kind, c));
        match &found {
            None => {}
            Some(f) if !ours(f) => return Ok(Outcome::Changed),
            Some(f) => {
                if let Some(outcome) = self.clear(os, f)? {
                    return Ok(outcome);
                }
            }
        }
        // A file stays until the rename replaces it; a link or folder was cleared.
        let mut there = found
            .as_ref()
            .filter(|f| matches!(f.kind, Some(EntryKind::File | EntryKind::Executable)))
            .map(|f| f.meta);
        // Some drives keep the old spelling when a file replaces one whose name differs only as
        // they ignore (macOS), so the old file takes the new spelling first. Only when both
        // names are that one file: a folder may keep case apart (Windows' per-folder setting, or
        // another drive mounted there), and then the old one is a file of its own, which its own
        // removal checks.
        // Both paths fold to one, so `old` is in this folder however its folders are spelled.
        if let Some((_, old)) = renaming.map(split)
            && old != name
            && there.is_some()
            && one_entry(self.folders.here(), old, name)
        {
            match self.unchanged(os, there) {
                Ok(true) => {}
                Ok(false) => return Ok(Outcome::Changed),
                Err(e) => return classify(e),
            }
            let dir = self.folders.here();
            if let Err(e) = retried(|| dir.rename_into(OsStr::new(old), dir, os)) {
                return classify(e);
            }
            // Renaming it changed its change time.
            there = match self.meta_of(os) {
                Ok(Some(meta)) => Some(meta),
                Ok(None) => return Ok(Outcome::Changed),
                Err(e) => return classify(e),
            };
        }
        self.write_file(os, c, executable, there)
    }

    /// Whether `name` in the folder entered still holds what was seen there: the same file, or
    /// nothing for `None`.
    fn unchanged(&self, name: &OsStr, seen: Option<EntryMeta>) -> io::Result<bool> {
        Ok(match (seen, self.meta_of(name)?) {
            (None, None) => true,
            (Some(seen), Some(now)) => same_file(&seen, &now),
            _ => false,
        })
    }

    /// Whether the project paths `a` and `b` name one entry on the drive.
    fn one_entry_at(&mut self, a: &str, b: &str) -> bool {
        let id = |folders: &mut Folders, path: &str| {
            let (up, name) = split(path);
            folders.enter(up).ok()?;
            let meta = folders.here().metadata(OsStr::new(name)).ok()?;
            Some(meta.file_id)
        };
        let first = id(&mut self.folders, a);
        first.is_some() && first == id(&mut self.folders, b)
    }

    /// Removes what is at `name` before something else is put there: a link or folder, which a
    /// rename won't replace. A file stays, for the rename to replace in one step.
    fn clear(&self, name: &OsStr, found: &Found) -> Result<Option<Outcome>, RepoError> {
        let cleared = match found.kind {
            Some(EntryKind::Folder) => {
                retried(|| self.folders.here().remove_dir(name)).map(|()| true)
            }
            Some(EntryKind::Symlink) => self.remove_unchanged(name, found.meta),
            _ => return Ok(None),
        };
        match cleared {
            Ok(true) => Ok(None),
            Ok(false) => Ok(Some(Outcome::Changed)),
            Err(e) => classify(e).map(Some),
        }
    }

    /// Writes the content `c` from the store to `name` in the folder entered, through a file in
    /// `.bolt/tmp`, flushed, then renamed into place. `there`: what the plan's look saw at
    /// `name`, `None` for nothing; if that changed meanwhile, it is left.
    fn write_file(
        &self,
        name: &OsStr,
        c: Content,
        executable: bool,
        there: Option<EntryMeta>,
    ) -> Result<Outcome, RepoError> {
        let tmp = self.repo.tmp_folder();
        let temp = bolt_platform::temp_name();
        let mut file = tmp.create_new(&temp, executable)?;
        let written = (|| -> Result<(), RepoError> {
            self.repo.store().copy_content(&c.id, &mut file)?;
            if file.metadata()?.len() != c.size {
                return Err(damaged(format!(
                    "the content {} isn't the size the switch expects",
                    c.id.to_hex()
                )));
            }
            file.sync_all()?;
            fsio::fault_point("switch:written")?;
            fsio::act_point("switch:written");
            Ok(())
        })();
        drop(file);
        // Writing can take long: another program may have saved there meanwhile.
        let outcome = match written {
            Ok(()) => match self.unchanged(name, there) {
                Ok(false) => Ok(Outcome::Changed),
                Ok(true) => match retried(|| tmp.rename_into(&temp, self.folders.here(), name)) {
                    Ok(()) => return Ok(Outcome::Wrote),
                    Err(e) => classify(e),
                },
                Err(e) => classify(e),
            },
            Err(e) => Err(e),
        };
        // The temporary file is only litter now, which opening the repository clears too.
        let _ = tmp.remove_file(&temp);
        outcome
    }

    fn remove(&mut self, a: &Action) -> Result<Outcome, RepoError> {
        let name = match self.enter(&a.path)? {
            Ok(name) => name,
            // Gone with the folder it was in.
            Err(Outcome::Gone) => return Ok(Outcome::Nothing),
            Err(outcome) => return Ok(outcome),
        };
        let sizes: Vec<u64> = a.expected.iter().map(|e| e.size).collect();
        let found = match self.look(name, &a.path, &sizes) {
            Ok(found) => found,
            Err(e) => return classify(e),
        };
        let os = OsStr::new(name);
        let dir = self.folders.here();
        let removed = match found {
            None => return Ok(Outcome::Nothing),
            Some(f) if !f.is(a.expected) => return Ok(Outcome::Changed),
            Some(f) if f.kind == Some(EntryKind::Folder) => {
                match retried(|| dir.remove_dir(os)) {
                    // It holds what the switch doesn't know of, an ignored file say: it stays.
                    Err(e) if e.kind() == io::ErrorKind::DirectoryNotEmpty => {
                        return Ok(Outcome::Nothing);
                    }
                    removed => removed.map(|()| true),
                }
            }
            Some(f) => self.remove_unchanged(os, f.meta),
        };
        match removed {
            Ok(true) => Ok(Outcome::Removed),
            Ok(false) => Ok(Outcome::Changed),
            Err(e) => classify(e),
        }
    }

    /// Removes the file or link at `name` in the folder entered unless it changed since it was
    /// `seen` (a second look at its metadata, just before the removal); whether it was removed.
    fn remove_unchanged(&self, name: &OsStr, seen: EntryMeta) -> io::Result<bool> {
        fsio::act_point("switch:removing");
        if !self.unchanged(name, Some(seen))? {
            return Ok(false);
        }
        retried(|| self.folders.here().remove_file(name))?;
        Ok(true)
    }

    fn link(&mut self, a: &Action, c: Content) -> Result<Outcome, RepoError> {
        let name = match self.enter(&a.path)? {
            Ok(name) => name,
            Err(outcome) => return Ok(outcome),
        };
        let mut sizes = vec![c.size];
        sizes.extend(a.expected.map(|e| e.size));
        let found = match self.look(name, &a.path, &sizes) {
            Ok(found) => found,
            Err(e) => return classify(e),
        };
        let os = OsStr::new(name);
        match &found {
            // The link, or a placeholder written before, counts as done.
            Some(f) if f.is_content(EntryKind::Symlink, c) => return Ok(Outcome::Nothing),
            None => {}
            Some(f) if !f.is(a.expected) => return Ok(Outcome::Changed),
            Some(f) => {
                let cleared = match f.kind {
                    Some(EntryKind::Folder) => {
                        retried(|| self.folders.here().remove_dir(os)).map(|()| true)
                    }
                    _ => self.remove_unchanged(os, f.meta),
                };
                match cleared {
                    Ok(true) => {}
                    Ok(false) => return Ok(Outcome::Changed),
                    Err(e) => return classify(e),
                }
            }
        }
        let target = link_target(self.repo.store(), c)?;
        let shape = match &target {
            Some(bytes) => {
                let root = self.target_root()?;
                // Only which names are folders counts here, not how the drive compares them.
                let exact = Drive {
                    ignores_case: false,
                    composes_unicode: false,
                };
                let mut tree = Tree::new(self.repo.store(), &self.stored, root, exact);
                link_shape(&mut tree, &a.path, bytes)?
            }
            None => None,
        };
        let (Some(to_dir), Some(target)) = (shape, target) else {
            return self.placeholder(os, c);
        };
        // Checked to be UTF-8 by `link_shape`.
        let target = String::from_utf8_lossy(&target);
        let target = if cfg!(windows) {
            target.replace('/', "\\")
        } else {
            target.into_owned()
        };
        match self.unchanged(os, None) {
            Ok(true) => {}
            Ok(false) => return Ok(Outcome::Changed),
            Err(e) => return classify(e),
        }
        let dir = self.folders.here();
        match dir.symlink(os, OsStr::new(&target), to_dir, self.repo.tmp_folder()) {
            Ok(()) => Ok(Outcome::Wrote),
            Err(e) if cant_link(&e) => self.placeholder(os, c),
            Err(e) => classify(e),
        }
    }

    fn placeholder(&self, name: &OsStr, c: Content) -> Result<Outcome, RepoError> {
        Ok(match self.write_file(name, c, false, None)? {
            Outcome::Wrote => Outcome::Placeholder,
            other => other,
        })
    }

    /// The top listing of the save the switch writes: the "to" side of its diary entry's folder
    /// change (an undo ends on another save than it writes), or else the save the entry is on.
    fn target_root(&mut self) -> Result<Id, RepoError> {
        if let Some(root) = self.target_root {
            return Ok(root);
        }
        let entry = self.repo.entry(&self.target_entry)?;
        let save = match entry.folder_change {
            Some(change) => change.to,
            None => tip_of(&entry)?,
        };
        let root = self
            .repo
            .store()
            .save(&save, self.repo.signatures_allowed())?
            .root;
        self.target_root = Some(root);
        Ok(root)
    }

    /// Flushes every folder changed, and says what was left.
    fn finish(mut self) -> Result<Applied, RepoError> {
        for rel in std::mem::take(&mut self.dirty) {
            if self.folders.enter(&rel).is_ok() {
                self.folders.here().sync()?;
            }
        }
        let mut applied = self.applied;
        applied.changed_since_plan = self.changed.into_iter().collect();
        applied.placeholders = self.placeholders;
        applied.not_written = self.refused.into_iter().collect();
        let warnings = [
            (!applied.not_written.is_empty()).then(|| not_written_warning(&applied.not_written)),
            (!applied.placeholders.is_empty()).then(|| placeholders_warning(&applied.placeholders)),
            (!applied.in_use.is_empty()).then(|| in_use_warning(&applied.in_use)),
            (!applied.changed_since_plan.is_empty())
                .then(|| changed_warning(&applied.changed_since_plan)),
        ];
        applied.warnings.extend(warnings.into_iter().flatten());
        Ok(applied)
    }
}

/// The first part of `path` (itself or a folder above it) whose name may not be written in its
/// folder, given each folder's `.git` and `.bolt` spellings with their short names (`own`).
fn first_refused(
    path: &str,
    mut own: impl FnMut(&str) -> io::Result<Vec<(String, String)>>,
) -> io::Result<Option<&str>> {
    for (at, name) in names_with_paths(path) {
        let here = own(parent(at))?;
        let pairs: Vec<(&str, &str)> = here.iter().map(|(l, s)| (l.as_str(), s.as_str())).collect();
        if writable_here(name, &pairs).is_err() {
            return Ok(Some(at));
        }
    }
    Ok(None)
}

/// Whether the names `a` and `b` in `dir` are one entry: the same file on the drive.
fn one_entry(dir: &Dir, a: &str, b: &str) -> bool {
    let id = |name: &str| dir.metadata(OsStr::new(name)).ok().map(|m| m.file_id);
    let first = id(a);
    first.is_some() && first == id(b)
}

/// The `.git` and `.bolt` spellings in `dir` that have a short name, with it.
fn own_short_names(dir: &Dir) -> io::Result<Vec<(String, String)>> {
    let entries = retried(|| dir.entries())?;
    Ok(entries
        .into_iter()
        .filter_map(|e| {
            let long = e.name.to_str()?.to_owned();
            let short = e.short_name?.to_str()?.to_owned();
            (is_git_alias(&long) || is_bolt_alias(&long)).then_some((long, short))
        })
        .collect())
}

/// An entry as `Run::look` found it.
struct Found {
    /// `None` for a special file, which is never what the plan expects.
    kind: Option<EntryKind>,
    size: u64,
    /// `None` if it wasn't read, or couldn't be.
    id: Option<Id>,
    meta: EntryMeta,
}

impl Found {
    /// Whether this is the entry the plan found; nothing matches `None`.
    fn is(&self, expected: Option<Expected>) -> bool {
        expected.is_some_and(|e| {
            self.kind.is_some_and(|k| same_kind(k, e.kind))
                && self.size == e.size
                && self.id == Some(e.id)
        })
    }

    fn is_content(&self, kind: EntryKind, c: Content) -> bool {
        self.is(Some(Expected {
            kind,
            id: c.id,
            size: c.size,
        }))
    }
}

/// Whether an entry `found` of one kind can be one `expected` of another. Windows has no
/// executable bit, so there a file is either; and a file holding a link's target is that link's
/// placeholder, as a save reads it, but a link is never a file.
fn same_kind(found: EntryKind, expected: EntryKind) -> bool {
    let file = |k| matches!(k, EntryKind::File | EntryKind::Executable);
    found == expected
        || (cfg!(windows) && file(found) && file(expected))
        || (expected == EntryKind::Symlink && file(found))
}

fn removes(change: Change) -> bool {
    matches!(change, Change::RemoveFile | Change::RemoveFolder)
}

/// What the journal records of an entry the folder holds. A folder has no contents there.
fn expected_of(e: &Entry) -> Expected {
    if e.kind == EntryKind::Folder {
        Expected {
            kind: e.kind,
            id: Id([0; 32]),
            size: 0,
        }
    } else {
        Expected {
            kind: e.kind,
            id: e.id,
            size: e.size,
        }
    }
}

/// The folder part and the last name of a project path.
fn split(path: &str) -> (&str, &str) {
    path.rsplit_once('/').unwrap_or(("", path))
}

fn parent(path: &str) -> &str {
    split(path).0
}

/// Whether the entry at `path` belongs to a switch limited to `only`: it is at or below one of
/// them, or is a folder that must be made on the way to one.
fn in_scope(path: &str, only: Option<&Scope>, t: Option<&Entry>) -> bool {
    let folder = t.is_some_and(|e| e.kind == EntryKind::Folder);
    only.is_none_or(|only| only.holds(path) || (folder && only.reaches_below(path)))
}

/// Whether unsaved work at `path`, or in a folder above or below it, is in the way.
fn collides(path: &str, unsaved: &BTreeSet<String>) -> bool {
    unsaved.contains(path)
        || ancestors(path).any(|up| unsaved.contains(up))
        || any_below(path, unsaved)
}

/// Whether `paths` holds something inside the folder `path`.
fn any_below(path: &str, paths: &BTreeSet<String>) -> bool {
    let below = format!("{path}/");
    paths
        .range(below.clone()..)
        .next()
        .is_some_and(|p| p.starts_with(&below))
}

/// The first part of `path` (itself or a folder above it) that a journal must never write:
/// a path outside the format's rules, or a name this system can't take, by every rule of
/// [`writable_here`] that needs no look at the disk.
fn refused_name(path: &str) -> Option<String> {
    if validate_path(path).is_err() {
        return Some(path.to_owned());
    }
    names_with_paths(path)
        .find(|(_, name)| writable_here(name, &[]).is_err())
        .map(|(at, _)| at.to_owned())
}

/// Each name of `path` with the path up to it: `("a", "a")`, then `("a/b", "b")`.
fn names_with_paths(path: &str) -> impl Iterator<Item = (&str, &str)> {
    let ends = path
        .match_indices('/')
        .map(|(at, _)| at)
        .chain([path.len()]);
    let mut start = 0;
    ends.filter_map(move |end| {
        let name = path.get(start..end)?;
        start = end + 1;
        Some((path.get(..end)?, name))
    })
}

/// [`refused_name`], and names that are one file with another of their folder on this drive,
/// in the save written or in the folder now.
fn refused_at<'a>(
    path: &str,
    target: &mut Tree<'a>,
    folder: &mut Tree<'a>,
) -> Result<Option<String>, StoreError> {
    if let Some(at) = refused_name(path) {
        return Ok(Some(at));
    }
    for (at, name) in names_with_paths(path) {
        for tree in [&mut *target, &mut *folder] {
            if tree
                .folder(parent(at))?
                .is_some_and(|f| f.twins.contains(name))
            {
                return Ok(Some(at.to_owned()));
            }
        }
    }
    Ok(None)
}

/// A project link, or a conflict, which write-out leaves to others.
fn not_ours(e: &Entry) -> bool {
    matches!(e.kind, EntryKind::ProjectLink | EntryKind::Conflict)
}

/// Whether a folder above `path` is a project link or conflict in the save written or in the
/// folder now: what is there belongs to the other project.
fn below_project_link<'a>(
    path: &str,
    target: &mut Tree<'a>,
    folder: &mut Tree<'a>,
) -> Result<bool, StoreError> {
    for up in ancestors(path) {
        for tree in [&mut *target, &mut *folder] {
            if tree.entry(up)?.is_some_and(|e| not_ours(&e)) {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

/// On a drive that folds names, a new entry whose name the drive takes for one the switch
/// removes (`README.md` where `Readme.md` goes) is that entry, renamed: it is checked against
/// what the removed one held.
fn take_over_renamed(actions: &mut [Action], drive: Drive) {
    let removed = removed_by_name(actions, drive);
    for a in actions.iter_mut() {
        if a.expected.is_none()
            && !removes(a.change)
            && let Some(expected) = removed.get(&fold_path(&a.path, drive))
        {
            a.expected = *expected;
        }
    }
}

/// The writes [`take_over_renamed`] checks against a removed entry: the folder held that entry,
/// under the removed spelling, and nothing under theirs.
pub(crate) fn renamed(actions: &[Action], drive: Drive) -> HashSet<&str> {
    if !(drive.ignores_case || drive.composes_unicode) {
        return HashSet::new();
    }
    let removed = removed_by_name(actions, drive);
    actions
        .iter()
        .filter(|a| {
            a.expected.is_some()
                && !removes(a.change)
                && removed.get(&fold_path(&a.path, drive)) == Some(&a.expected)
        })
        .map(|a| a.path.as_str())
        .collect()
}

/// What each entry the switch removes held, by its path as `drive` compares names.
fn removed_by_name(actions: &[Action], drive: Drive) -> HashMap<String, Option<Expected>> {
    actions
        .iter()
        .filter(|a| removes(a.change))
        .map(|a| (fold_path(&a.path, drive), a.expected))
        .collect()
}

/// `path` as `drive` compares names: paths that come out the same are one entry there.
pub(crate) fn fold_path(path: &str, drive: Drive) -> String {
    path.split('/')
        .map(|name| drive.compared(name))
        .collect::<Vec<_>>()
        .join("/")
}

/// Whether the folder holds something at `path` that the plan doesn't know of: an ignored
/// file, say. Only asked where the folder above is one the plan read.
fn in_the_way(disk: &mut Folders, folder: &mut Tree, path: &str) -> Result<bool, SaveError> {
    let (up, name) = split(path);
    if folder.folder(up)?.is_none() {
        return Ok(false);
    }
    if disk.enter(up).is_err() {
        return Ok(true);
    }
    match disk.here().metadata(OsStr::new(name)) {
        Ok(_) => Ok(true),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(unreadable(path, e)),
    }
}

/// What is on disk below the folder `path` that no action takes away; `removed` holds their
/// paths as `drive` compares them. A folder that can't be opened counts as left, since the switch
/// couldn't empty it either.
fn left_below(
    disk: &mut Folders,
    path: &str,
    removed: &HashSet<String>,
    drive: Drive,
) -> Result<Vec<String>, SaveError> {
    let mut left = Vec::new();
    let mut todo = vec![path.to_owned()];
    while let Some(dir) = todo.pop() {
        let listed = disk.enter(&dir).and_then(|()| disk.here().entries());
        let entries = match listed {
            Ok(entries) => entries,
            Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
            Err(_) => {
                left.push(dir);
                continue;
            }
        };
        for e in entries {
            let below = format!("{dir}/{}", e.name.to_string_lossy());
            let gone = e.name.to_str().is_some() && removed.contains(&fold_path(&below, drive));
            if !gone {
                left.push(below);
            } else if e.kind == EntryType::Dir {
                todo.push(below);
            }
        }
    }
    Ok(left)
}

fn unreadable(path: &str, source: io::Error) -> SaveError {
    SaveError::Unreadable {
        path: path.to_owned(),
        source,
    }
}

/// The target a link holds, from the store, or `None` if it is too long to be one.
fn link_target(store: &Store, c: Content) -> Result<Option<Vec<u8>>, StoreError> {
    if c.size > MAX_LINK_TARGET {
        return Ok(None);
    }
    let mut out = Capped::default();
    match store.copy_content(&c.id, &mut out) {
        Ok(()) => Ok(Some(out.bytes)),
        // The listing understated its size: what it holds is too long for a link.
        Err(_) if out.over => Ok(None),
        Err(e) => Err(e),
    }
}

/// Takes at most `MAX_LINK_TARGET` bytes, and says when more came.
#[derive(Default)]
struct Capped {
    bytes: Vec<u8>,
    over: bool,
}

impl Write for Capped {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if (self.bytes.len() + buf.len()) as u64 > MAX_LINK_TARGET {
            self.over = true;
            return Err(io::Error::new(
                io::ErrorKind::FileTooLarge,
                "too long for a link",
            ));
        }
        self.bytes.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Whether the link at `path` with this stored target stays inside the project, judged from the
/// save written alone, never from the disk: `Some(true)` if it leads to a folder of that save,
/// `Some(false)` otherwise; `None` for a link to write as a placeholder: one [`link_parts`]
/// refuses, or whose names before the last aren't all folders of that save.
fn link_shape(tree: &mut Tree, path: &str, target: &[u8]) -> Result<Option<bool>, StoreError> {
    let Some(parts) = link_parts(path, target) else {
        return Ok(None);
    };
    let mut at: Vec<&str> = parent(path).split('/').filter(|n| !n.is_empty()).collect();
    for (i, part) in parts.iter().enumerate() {
        if *part == ".." {
            at.pop();
            continue;
        }
        at.push(part);
        // `..` is taken on the drive after the name before it, so every name but the last must
        // be a folder of the save: through a file, a link or nothing, it could lead anywhere.
        let last = i + 1 == parts.len();
        if !last && tree.folder(&at.join("/"))?.is_none() {
            return Ok(None);
        }
    }
    Ok(Some(tree.folder(&at.join("/"))?.is_some()))
}

/// The parts of a link's target in the order they are followed from the link's folder, or `None`
/// if it leads out of the project: it is absolute, starts with a drive letter or `\\`, or climbs
/// past the top with `..`; on Windows also when it holds a `\` or a name Windows reads otherwise.
/// `None` too when it passes through a name write-out wouldn't write, such as a spelling of
/// `.git` or `.bolt`: a link into the project's own `.git` could run what it finds there.
fn link_parts<'a>(path: &str, target: &'a [u8]) -> Option<Vec<&'a str>> {
    let target = std::str::from_utf8(target).ok()?;
    let bytes = target.as_bytes();
    let drive_letter =
        bytes.first().is_some_and(u8::is_ascii_alphabetic) && bytes.get(1) == Some(&b':');
    let windows_reads_otherwise = cfg!(windows) && target.contains(['\\', ':']);
    if target.is_empty()
        || target.starts_with(['/', '\\'])
        || drive_letter
        || windows_reads_otherwise
    {
        return None;
    }
    let parts: Vec<&str> = target
        .split('/')
        .filter(|p| !p.is_empty() && *p != ".")
        .collect();
    let mut depth = parent(path).split('/').filter(|n| !n.is_empty()).count();
    for part in &parts {
        if *part == ".." {
            depth = depth.checked_sub(1)?;
        } else if writable_here(part, &[]).is_ok() {
            depth += 1;
        } else {
            return None;
        }
    }
    Some(parts)
}

/// Whether write-out writes a link at `path` with `target` as a link, not a placeholder, where
/// `tree` is the save it writes and links can be made: the target isn't too long, and
/// [`link_shape`] keeps it inside the project.
pub(crate) fn writes_as_link(
    tree: &mut Tree,
    path: &str,
    target: &[u8],
) -> Result<bool, StoreError> {
    Ok(target.len() as u64 <= MAX_LINK_TARGET && link_shape(tree, path, target)?.is_some())
}

/// Whether this system and account can make links: tried with a throwaway one in `.bolt/tmp`.
pub(crate) fn can_make_links(repo: &Repository) -> Result<bool, SaveError> {
    let tmp = repo.tmp_folder();
    let name = bolt_platform::temp_name();
    match tmp.symlink(&name, OsStr::new("x"), false, tmp) {
        Ok(()) => {
            // Left behind it is only litter, which opening the project clears.
            let _ = fsio::remove_file(tmp, &name);
            Ok(true)
        }
        Err(e) if cant_link(&e) => Ok(false),
        Err(e) => Err(RepoError::from(e).into()),
    }
}

/// Whether making a link failed because this system or account can't make one here: Windows
/// without Developer Mode or an administrator, or a drive without links.
fn cant_link(e: &io::Error) -> bool {
    const PRIVILEGE_NOT_HELD: i32 = 1314;
    (cfg!(windows) && e.raw_os_error() == Some(PRIVILEGE_NOT_HELD))
        || e.kind() == io::ErrorKind::Unsupported
        || (cfg!(unix) && e.kind() == io::ErrorKind::PermissionDenied)
}

/// Whether another program holds the file open: a sharing or lock violation on Windows.
fn in_use(e: &io::Error) -> bool {
    // A project file in use (32) or locked (33) is left for the next run; a refusal (5) may be real.
    cfg!(windows) && matches!(e.raw_os_error(), Some(32 | 33))
}

/// `act`, tried again while another program holds the file.
fn retried<T>(mut act: impl FnMut() -> io::Result<T>) -> io::Result<T> {
    let mut tries = 1;
    loop {
        match act() {
            Err(e) if in_use(&e) && tries < TRIES => {
                tries += 1;
                std::thread::sleep(BETWEEN_TRIES);
            }
            done => return done,
        }
    }
}

/// What a failure making a change comes to: a file in use; a change in the folder since the
/// plan (something gone, in the way, or a link where a folder was); or an error that stops.
fn classify(e: io::Error) -> Result<Outcome, RepoError> {
    use io::ErrorKind as K;
    if in_use(&e) {
        return Ok(Outcome::InUse);
    }
    if is_link_refusal(&e)
        || matches!(
            e.kind(),
            K::NotFound
                | K::AlreadyExists
                | K::NotADirectory
                | K::IsADirectory
                | K::DirectoryNotEmpty
        )
    {
        return Ok(Outcome::Changed);
    }
    Err(e.into())
}

/// What a folder that can't be entered comes to: one that isn't there, or is a file now, is
/// `Gone`; one swapped for a link is changed since the plan.
fn entering_failed(e: io::Error) -> Result<Outcome, RepoError> {
    if matches!(
        e.kind(),
        io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
    ) && !is_link_refusal(&e)
    {
        return Ok(Outcome::Gone);
    }
    classify(e)
}

fn hash(file: &mut std::fs::File) -> io::Result<Id> {
    let mut hasher = ContentHasher::new();
    let mut buf = vec![0; 64 * 1024];
    loop {
        match file.read(&mut buf) {
            Ok(0) => return Ok(hasher.finish()),
            Ok(n) => hasher.update(buf.get(..n).unwrap_or_default()),
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
}

/// The project's folders, each opened from the top one name at a time, never through a link. One
/// is held at a time, so a folder about to be removed is never held open.
pub(crate) struct Folders {
    top: Dir,
    at: Option<(String, Dir)>,
}

impl Folders {
    pub(crate) fn open(root: &Path) -> io::Result<Folders> {
        Ok(Folders {
            top: Dir::open_top(root)?,
            at: None,
        })
    }

    /// Opens the folder at the project path `rel` (`""` for the top).
    pub(crate) fn enter(&mut self, rel: &str) -> io::Result<()> {
        if rel.is_empty() || self.at.as_ref().is_some_and(|(at, _)| at == rel) {
            if rel.is_empty() {
                self.at = None;
            }
            return Ok(());
        }
        self.at = None;
        let mut names = rel.split('/');
        let mut dir = self.top.open_dir(OsStr::new(names.next().unwrap_or("")))?;
        for name in names {
            dir = dir.open_dir(OsStr::new(name))?;
        }
        self.at = Some((rel.to_owned(), dir));
        Ok(())
    }

    pub(crate) fn here(&self) -> &Dir {
        self.at.as_ref().map_or(&self.top, |(_, dir)| dir)
    }
}

/// One save's folders, read as needed: from the store, or from `unstored` for a folder read but
/// not saved.
pub(crate) struct Tree<'a> {
    store: &'a Store,
    unstored: &'a HashMap<Id, Listing>,
    root: Id,
    drive: Drive,
    folders: HashMap<String, Option<Folder>>,
}

pub(crate) struct Folder {
    pub(crate) listing: Listing,
    /// Names that are one file with another of this folder's on the drive.
    twins: HashSet<String>,
}

impl<'a> Tree<'a> {
    pub(crate) fn new(
        store: &'a Store,
        unstored: &'a HashMap<Id, Listing>,
        root: Id,
        drive: Drive,
    ) -> Tree<'a> {
        Tree {
            store,
            unstored,
            root,
            drive,
            folders: HashMap::new(),
        }
    }

    /// The folder at `path` (`""` for the top), if the save has one there. Read down from the
    /// deepest folder already read on the way, a name at a time.
    pub(crate) fn folder(&mut self, path: &str) -> Result<Option<&Folder>, StoreError> {
        if !self.folders.contains_key(path) {
            let mut known = path;
            while !known.is_empty() && !self.folders.contains_key(known) {
                known = parent(known);
            }
            if !self.folders.contains_key(known) {
                let top = self.read(Some(self.root))?;
                self.folders.insert(String::new(), top);
            }
            let rest = if known.is_empty() {
                path
            } else {
                path.get(known.len() + 1..).unwrap_or("")
            };
            let mut at = known.to_owned();
            for name in rest.split('/').filter(|n| !n.is_empty()) {
                let id = self
                    .folders
                    .get(&at)
                    .and_then(Option::as_ref)
                    .and_then(|f| f.listing.get(name))
                    .filter(|e| e.kind == EntryKind::Folder)
                    .map(|e| e.id);
                let next = if at.is_empty() {
                    name.to_owned()
                } else {
                    format!("{at}/{name}")
                };
                let folder = self.read(id)?;
                self.folders.insert(next.clone(), folder);
                at = next;
            }
        }
        Ok(self.folders.get(path).and_then(Option::as_ref))
    }

    pub(crate) fn entry(&mut self, path: &str) -> Result<Option<Entry>, StoreError> {
        let (up, name) = split(path);
        Ok(self.folder(up)?.and_then(|f| f.listing.get(name)).cloned())
    }

    fn read(&self, id: Option<Id>) -> Result<Option<Folder>, StoreError> {
        let Some(id) = id else {
            return Ok(None);
        };
        let listing = match self.unstored.get(&id) {
            Some(listing) => listing.clone(),
            None => self.store.listing(&id)?,
        };
        let mut twins = HashSet::new();
        if self.drive.ignores_case || self.drive.composes_unicode {
            let mut first: HashMap<String, &str> = HashMap::new();
            for e in listing.entries() {
                if let Some(other) = first.insert(self.drive.compared(&e.name), &e.name) {
                    twins.insert(other.to_owned());
                    twins.insert(e.name.clone());
                }
            }
        }
        Ok(Some(Folder { listing, twins }))
    }
}

fn counted(count: usize, one: &str, many: &str) -> String {
    if count == 1 {
        format!("1 {one}")
    } else {
        format!("{count} {many}")
    }
}

fn first(paths: &[String]) -> String {
    listed(paths.iter().take(NAMED), paths.len() as u64)
}

/// Says entries of the save weren't written because this computer can't hold them.
#[must_use]
pub fn not_written_warning(paths: &[String]) -> Warning {
    Warning::new(
        warning::NOT_WRITTEN_HERE,
        format!(
            "Left {} unwritten: this computer can't hold them as they are (a name it can't make, \
             a spelling of .git or .bolt, or two names that are one file on this drive):{}\nThey \
             stay in history, and saves keep them as they are.",
            counted(paths.len(), "entry", "entries"),
            first(paths)
        ),
    )
}

/// Says links were written as files holding their targets.
#[must_use]
pub fn placeholders_warning(paths: &[String]) -> Warning {
    Warning::new(
        warning::PLACEHOLDERS_WRITTEN,
        format!(
            "Wrote {} as plain files holding their targets: they lead out of the project, or this \
             computer can't make links (on Windows that takes Developer Mode):{}",
            counted(paths.len(), "link", "links"),
            first(paths)
        ),
    )
}

fn in_use_warning(paths: &[String]) -> Warning {
    Warning::new(
        warning::FILES_IN_USE,
        format!(
            "Another program has {} open, so Bolt left them as they are:{}\nClose it, then run \
             the command again to finish.",
            counted(paths.len(), "file", "files"),
            first(paths)
        ),
    )
}

/// Says entries changed after the plan, so they were left as they are.
pub(crate) fn changed_warning(paths: &[String]) -> Warning {
    Warning::new(
        warning::CHANGED_SINCE_PLAN,
        format!(
            "{} changed while Bolt was switching, so it left them as they are:{}",
            counted(paths.len(), "entry", "entries"),
            first(paths)
        ),
    )
}

#[cfg(test)]
mod tests;
