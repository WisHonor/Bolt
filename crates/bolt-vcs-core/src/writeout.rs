//! Switching the project folder from one save to another (decisions 049 to 052). [`plan`] works
//! out what to change; [`apply`] changes it, a name at a time, through folders opened one below
//! the other without following a link. A save, and the journal made from it, may come from
//! anyone, so every name is checked again right before it reaches the disk.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::ffi::OsStr;
use std::io::{self, Read, Write};
use std::path::Path;
use std::time::Duration;

use bolt_platform::dir::{Dir, EntryType, is_link_refusal};

use crate::diary::{DiaryEntry, SafetyCopy};
use crate::diff::{for_each_change_in, for_each_entry_change_in};
use crate::drive::{self, Drive};
use crate::filecache::FileCache;
use crate::fsio;
use crate::history::tip_of;
use crate::id::{ContentHasher, Id, hash_content};
use crate::journal::{self, Action, Change, Content, Expected, Journal};
use crate::link;
use crate::listing::{Entry, EntryKind, Listing};
use crate::names::{
    MAX_PATH_BYTES, holdable_here, is_bolt_alias, is_git_alias, validate_path, writable_here,
};
use crate::pack::{ObjectKind, PackError, PackSet};
use crate::repo::{ChangeLock, RepoError, Repository, damaged};
use crate::snapshot::{NAMED, SaveError, listed, read_folder};
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

/// A file another program holds is tried this many times, this far apart (decision 049).
const TRIES: u32 = 10;
const BETWEEN_TRIES: Duration = Duration::from_millis(100);

/// Works out how to switch the folder from the save `from`, the one it is on, to the save `to`.
/// The folder is read as `status` reads it. Unsaved work (a difference between the folder and
/// `from`) where `to` differs from `from`, at the same path or a folder above or below it, is a
/// collision and left out; the rest of it stays as it is. With `remove_new`, the folder is made
/// to match `to` exactly instead, new files removed, and nothing collides. `only` limits the
/// switch to those paths and what is in them.
pub fn plan(
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
    let now = read_folder(repo, from_root, false, git_excludes)?;
    let mut unsaved = BTreeSet::new();
    for_each_change_in(store, &now.listings, Some(from_root), now.root, |c| {
        unsaved.insert(c.path.to_owned());
        Ok::<(), SaveError>(())
    })?;
    let stored = HashMap::new();
    let mut switched = HashSet::new();
    for_each_entry_change_in(store, &stored, Some(from_root), to_root, |path, _, _| {
        switched.insert(path.to_owned());
        Ok::<(), SaveError>(())
    })?;

    let mut target = Tree::new(store, &stored, to_root, drive);
    let mut folder = Tree::new(store, &now.listings, now.root, drive);
    let mut refused = BTreeSet::new();
    let mut plan = Plan {
        carried: unsaved.len(),
        warnings: now.warnings,
        ..Plan::default()
    };
    for_each_entry_change_in(
        store,
        &now.listings,
        Some(now.root),
        to_root,
        |path, n, t| {
            if !in_scope(path, only, t) {
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
                    return Ok(()); // unsaved work the switch doesn't reach comes along
                }
                if collides(path, &unsaved) {
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
    // Nothing is planned below a folder left out, or it would be made inside what is in the way.
    let left_out: HashSet<&str> = plan.collisions.iter().map(String::as_str).collect();
    kept.retain(|a| !ancestors(&a.path).any(|up| left_out.contains(up)));
    plan.actions = kept;
    plan.collisions.sort_unstable();
    plan.not_written = refused.into_iter().collect();
    Ok(plan)
}

/// Whether the save `save` holds an entry at `path`, spelled exactly so.
pub fn holds(repo: &Repository, save: Id, path: &str) -> Result<bool, SaveError> {
    let store = repo.store();
    let root = store.save(&save, repo.signatures_allowed())?.root;
    let stored = HashMap::new();
    // Exact names only, so the drive's way of comparing them doesn't matter.
    let drive = Drive {
        ignores_case: false,
        composes_unicode: false,
    };
    Ok(Tree::new(store, &stored, root, drive)
        .entry(path)?
        .is_some())
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
        let held =
            packs.contains(&e.id) || store.has_good_copy(&e.id, ObjectKind::Content, e.size)?;
        if !held && !copy_in(&mut disk, &a.path, e, packs)? {
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
    packs: &mut PackSet,
) -> Result<bool, SaveError> {
    let (up, name) = split(path);
    if disk.enter(up).is_err() {
        return Ok(false);
    }
    let dir = disk.here();
    let os = OsStr::new(name);
    // Gone, or no longer what it was: the switch leaves what is there.
    let still = |kind: EntryType| dir.metadata(os).is_ok_and(|now| now.kind == kind);
    if e.kind == EntryKind::Symlink {
        let target = match crate::snapshot::link_target(dir, name, path) {
            Ok(target) => target,
            Err(_) if !still(EntryType::Symlink) => return Ok(false),
            Err(err) => return Err(err),
        };
        if hash_content(target.as_bytes()) != e.id {
            return Ok(false);
        }
        packs.add_content(target.as_bytes())?;
        return Ok(true);
    }
    let mut file = match dir.open_file(os) {
        Ok(file) => file,
        Err(_) if !still(EntryType::File) => return Ok(false),
        Err(err) => return Err(unreadable(path, err)),
    };
    match packs.add_content_from(&mut file, e.id) {
        Ok(_) => Ok(true),
        Err(PackError::ContentChanged) => Ok(false),
        Err(err) => Err(err.into()),
    }
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
/// entry and removes it: the switch has then happened. With files in use the journal stays and
/// `head` doesn't move, so the next command finishes the rest.
pub fn carry_out(
    repo: &mut Repository,
    lock: &ChangeLock,
    journal: &Journal,
    drive: Drive,
) -> Result<Applied, RepoError> {
    let done = apply(repo, lock, journal, drive)?;
    if done.in_use.is_empty() {
        repo.move_head_to(lock, journal.target)?;
        Journal::remove(repo, lock)?;
    }
    Ok(done)
}

/// Finishes a switch that a crash or a stop left in `state/journal`, before a command changes
/// anything (decision 049): never rolled back. `None` when there was none, or when `head`
/// already names its entry and only the journal was left. A switch that still can't finish is
/// an error, the journal kept, so no command builds on a half-switched folder.
pub fn finish_interrupted(
    repo: &mut Repository,
    lock: &ChangeLock,
) -> Result<Option<Applied>, RepoError> {
    let Some(journal) = Journal::read(repo)? else {
        return Ok(None);
    };
    let head = repo.head()?;
    if head == journal.target {
        Journal::remove(repo, lock)?;
        return Ok(None);
    }
    let journal = usable(repo, journal, head)?;
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

/// `journal` if it can be carried out on this project, whose `head` is `head`: made from that
/// entry, to one that reads. Otherwise it is no switch of this folder to finish.
fn usable(repo: &Repository, journal: Journal, head: Id) -> Result<Journal, RepoError> {
    if head != journal.from {
        return Err(journal::unusable(
            "was made for another save than the one the project is on",
        ));
    }
    let entry = repo
        .store()
        .read_record(&journal.target, ObjectKind::DiaryEntry)
        .map_err(RepoError::from)
        .and_then(|bytes| Ok(DiaryEntry::decode(&bytes)?));
    match entry {
        Ok(_) => Ok(journal),
        Err(e) => Err(journal::unusable(format!(
            "names a diary entry that can't be read ({e})"
        ))),
    }
}

/// For commands that only read: says a switch between saves is still to finish, or that its
/// journal can't be used.
#[must_use]
pub fn unfinished_warning(repo: &Repository) -> Option<Warning> {
    let pending = Journal::read(repo).and_then(|found| match found {
        None => Ok(None),
        Some(j) => {
            let head = repo.head()?;
            // It took effect; the next change only removes the journal.
            if head == j.target {
                return Ok(None);
            }
            usable(repo, j, head).map(Some)
        }
    });
    let message = match pending {
        Ok(None) => return None,
        Ok(Some(_)) => "A switch between saves was stopped before it was done, so some files may \
                        still be as they were. Running `bolt goto` or `bolt discard` again, or any \
                        command that changes the project, finishes it first."
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
    /// The diary entry the switch commits, whose save says which links lead to folders.
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
        let meta = match retried(|| dir.metadata(os)) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            meta => meta?,
        };
        let kind = match meta.kind {
            EntryType::Dir => {
                return Ok(Some(Found {
                    kind: Some(EntryKind::Folder),
                    size: 0,
                    id: Some(Id([0; 32])),
                }));
            }
            EntryType::Other => {
                return Ok(Some(Found {
                    kind: None,
                    size: meta.size,
                    id: None,
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
        }))
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
        let dir = self.folders.here();
        match found {
            Some(f) if f.kind == Some(EntryKind::Folder) => return Ok(Outcome::Nothing),
            None => {}
            Some(f) if f.is(a.expected) => {
                if let Err(e) = retried(|| dir.remove_file(os)) {
                    return classify(e);
                }
            }
            Some(_) => return Ok(Outcome::Changed),
        }
        match dir.create_dir(os) {
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
        {
            if f.kind == Some(kind) || cfg!(windows) {
                return Ok(Outcome::Nothing);
            }
            // Only the executable bit differs.
            return match self.folders.here().set_executable(os, executable) {
                Ok(()) => Ok(Outcome::Wrote),
                Err(e) => classify(e),
            };
        }
        let ours = |f: &Found| f.is(a.expected) || (renaming.is_some() && f.is_content(kind, c));
        match &found {
            None => {}
            Some(f) if !ours(f) => return Ok(Outcome::Changed),
            Some(f) => {
                if let Some(outcome) = self.clear(os, f.kind)? {
                    return Ok(outcome);
                }
            }
        }
        // Some drives keep the old spelling when a file replaces one whose name differs only as
        // they ignore (macOS), so the old file takes the new spelling first. Only when both
        // names are that one file: a folder may keep case apart (Windows' per-folder setting, or
        // another drive mounted there), and then the old one is a file of its own, which its own
        // removal checks.
        // Both paths fold to one, so `old` is in this folder however its folders are spelled.
        if let Some((_, old)) = renaming.map(split)
            && old != name
            && found
                .as_ref()
                .is_some_and(|f| matches!(f.kind, Some(EntryKind::File | EntryKind::Executable)))
            && one_entry(self.folders.here(), old, name)
        {
            let dir = self.folders.here();
            if let Err(e) = retried(|| dir.rename_into(OsStr::new(old), dir, os)) {
                return classify(e);
            }
        }
        self.write_file(os, c, executable)
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
    fn clear(&self, name: &OsStr, kind: Option<EntryKind>) -> Result<Option<Outcome>, RepoError> {
        let dir = self.folders.here();
        let cleared = match kind {
            Some(EntryKind::Folder) => retried(|| dir.remove_dir(name)),
            Some(EntryKind::Symlink) => retried(|| dir.remove_file(name)),
            _ => return Ok(None),
        };
        match cleared {
            Ok(()) => Ok(None),
            Err(e) => classify(e).map(Some),
        }
    }

    /// Writes the content `c` from the store to `name` in the folder entered, through a file in
    /// `.bolt/tmp`, flushed, then renamed over whatever file is there.
    fn write_file(&self, name: &OsStr, c: Content, executable: bool) -> Result<Outcome, RepoError> {
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
            Ok(())
        })();
        drop(file);
        let outcome = match written {
            Ok(()) => match retried(|| tmp.rename_into(&temp, self.folders.here(), name)) {
                Ok(()) => return Ok(Outcome::Wrote),
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
                    removed => removed,
                }
            }
            Some(_) => retried(|| dir.remove_file(os)),
        };
        match removed {
            Ok(()) => Ok(Outcome::Removed),
            Err(e) => classify(e),
        }
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
            // A placeholder written before counts as done.
            Some(f) if f.is_content(EntryKind::Symlink, c) || f.is_content(EntryKind::File, c) => {
                return Ok(Outcome::Nothing);
            }
            None => {}
            Some(f) if !f.is(a.expected) => return Ok(Outcome::Changed),
            Some(f) => {
                let dir = self.folders.here();
                let cleared = match f.kind {
                    Some(EntryKind::Folder) => retried(|| dir.remove_dir(os)),
                    _ => retried(|| dir.remove_file(os)),
                };
                if let Err(e) = cleared {
                    return classify(e);
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
        let dir = self.folders.here();
        match dir.symlink(os, OsStr::new(&target), to_dir, self.repo.tmp_folder()) {
            Ok(()) => Ok(Outcome::Wrote),
            Err(e) if cant_link(&e) => self.placeholder(os, c),
            Err(e) => classify(e),
        }
    }

    fn placeholder(&self, name: &OsStr, c: Content) -> Result<Outcome, RepoError> {
        Ok(match self.write_file(name, c, false)? {
            Outcome::Wrote => Outcome::Placeholder,
            other => other,
        })
    }

    /// The top listing of the save the journal's diary entry is on.
    fn target_root(&mut self) -> Result<Id, RepoError> {
        if let Some(root) = self.target_root {
            return Ok(root);
        }
        let store = self.repo.store();
        let bytes = store.read_record(&self.target_entry, ObjectKind::DiaryEntry)?;
        let save = tip_of(&DiaryEntry::decode(&bytes)?)?;
        let root = store.save(&save, self.repo.signatures_allowed())?.root;
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

/// Windows has no executable bit, so there a file is either.
fn same_kind(a: EntryKind, b: EntryKind) -> bool {
    let file = |k| matches!(k, EntryKind::File | EntryKind::Executable);
    a == b || (cfg!(windows) && file(a) && file(b))
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

/// The folders above `path`, nearest last: `a`, `a/b` for `a/b/c`.
fn ancestors(path: &str) -> impl Iterator<Item = &str> {
    path.match_indices('/').filter_map(|(at, _)| path.get(..at))
}

/// Whether `path` is `top` or below it; `""` is the whole project.
pub(crate) fn within(path: &str, top: &str) -> bool {
    top.is_empty()
        || path == top
        || path
            .strip_prefix(top)
            .is_some_and(|rest| rest.starts_with('/'))
}

/// Whether the entry at `path` belongs to a switch limited to `only`: it is at or below one of
/// them, or is a folder that must be made on the way to one.
fn in_scope(path: &str, only: Option<&[String]>, t: Option<&Entry>) -> bool {
    let folder = t.is_some_and(|e| e.kind == EntryKind::Folder);
    only.is_none_or(|only| {
        only.iter()
            .any(|top| within(path, top) || (folder && within(top, path)))
    })
}

/// Whether unsaved work at `path`, or in a folder above or below it, is in the way.
fn collides(path: &str, unsaved: &BTreeSet<String>) -> bool {
    let below = format!("{path}/");
    unsaved.contains(path)
        || ancestors(path).any(|up| unsaved.contains(up))
        || unsaved
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
    let removed: HashMap<String, Option<Expected>> = actions
        .iter()
        .filter(|a| removes(a.change))
        .map(|a| (fold_path(&a.path, drive), a.expected))
        .collect();
    for a in actions.iter_mut() {
        if a.expected.is_none()
            && !removes(a.change)
            && let Some(expected) = removed.get(&fold_path(&a.path, drive))
        {
            a.expected = *expected;
        }
    }
}

fn fold_path(path: &str, drive: Drive) -> String {
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
/// save written alone, never from the disk (decision 052): `Some(true)` if it leads to a folder
/// of that save, `Some(false)` otherwise; `None` for a link to write as a placeholder. A target
/// leaves when it is absolute, starts with a drive letter or `\\`, or climbs past the top with
/// `..`; on Windows also when it holds a `\` (decision 045) or a name Windows reads otherwise.
/// So is a target whose names before the last aren't all folders of that save.
fn link_shape(tree: &mut Tree, path: &str, target: &[u8]) -> Result<Option<bool>, StoreError> {
    let Ok(target) = std::str::from_utf8(target) else {
        return Ok(None);
    };
    let bytes = target.as_bytes();
    let drive_letter =
        bytes.first().is_some_and(u8::is_ascii_alphabetic) && bytes.get(1) == Some(&b':');
    let windows_reads_otherwise = cfg!(windows) && target.contains(['\\', ':']);
    if target.is_empty()
        || target.starts_with(['/', '\\'])
        || drive_letter
        || windows_reads_otherwise
    {
        return Ok(None);
    }
    let mut at: Vec<&str> = parent(path).split('/').filter(|n| !n.is_empty()).collect();
    let parts: Vec<&str> = target
        .split('/')
        .filter(|p| !p.is_empty() && *p != ".")
        .collect();
    for (i, part) in parts.iter().enumerate() {
        if *part == ".." {
            if at.pop().is_none() {
                return Ok(None);
            }
            continue;
        }
        if !holdable_here(part) {
            return Ok(None);
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
struct Folders {
    top: Dir,
    at: Option<(String, Dir)>,
}

impl Folders {
    fn open(root: &Path) -> io::Result<Folders> {
        Ok(Folders {
            top: Dir::open_top(root)?,
            at: None,
        })
    }

    /// Opens the folder at the project path `rel` (`""` for the top).
    fn enter(&mut self, rel: &str) -> io::Result<()> {
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

    fn here(&self) -> &Dir {
        self.at.as_ref().map_or(&self.top, |(_, dir)| dir)
    }
}

/// One save's folders, read as needed: from the store, or from `unstored` for a folder read but
/// not saved.
struct Tree<'a> {
    store: &'a Store,
    unstored: &'a HashMap<Id, Listing>,
    root: Id,
    drive: Drive,
    folders: HashMap<String, Option<Folder>>,
}

struct Folder {
    listing: Listing,
    /// Names that are one file with another of this folder's on the drive.
    twins: HashSet<String>,
}

impl<'a> Tree<'a> {
    fn new(
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
    fn folder(&mut self, path: &str) -> Result<Option<&Folder>, StoreError> {
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

    fn entry(&mut self, path: &str) -> Result<Option<Entry>, StoreError> {
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

fn changed_warning(paths: &[String]) -> Warning {
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
