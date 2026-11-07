//! `bolt shelf` and `bolt unshelf`: unsaved changes set aside on a numbered shelf, a kept save
//! the state lists, and brought back onto whatever save the project is on.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::Path;

use crate::diary::{Action, MAX_SHELVES};
use crate::diff::{count_changes, for_each_change};
use crate::history::{resolve, tip_of};
use crate::id::Id;
use crate::kept;
use crate::listing::{Entry, EntryKind};
use crate::repo::{RepoError, Repository, damaged};
use crate::save::Person;
use crate::scope::ancestors;
use crate::snapshot::{NAMED, listed};
use crate::store::StoreError;
use crate::switch::{self, Away, Earlier, ShelfCollisions, Switch, SwitchError};
use crate::warning::{self, Warning};
use crate::writeout::{self, Applied, Plan, Tree};

/// A shelf's message when it is given none.
const DEFAULT_MESSAGE: &str = "Shelved";

/// What `shelf` did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShelfOutcome {
    pub label: String,
    /// The kept save holding what was set aside.
    pub save: Id,
    /// How many files the shelf changes.
    pub files: usize,
    /// What putting the folder back wrote and removed.
    pub applied: Applied,
    /// Unsaved links left as they are, since they could come back only as files.
    pub links_left: Vec<String>,
}

/// What `shelf` did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Shelved {
    /// The unsaved changes went on a new shelf.
    Made(ShelfOutcome),
    /// No shelf and no step: nothing unsaved could go on one, or (`changed_again`) every file
    /// changed again while Bolt was reading it.
    Nothing {
        changed_again: bool,
        /// Unsaved links left as they are, since they could come back only as files.
        links_left: Vec<String>,
    },
}

/// What `unshelf` did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnshelfOutcome {
    /// The shelf brought back; empty when `finished`.
    pub label: String,
    /// The shelf's save; the save the project is on when `finished`.
    pub save: Id,
    pub applied: Applied,
    /// Unsaved changes that stayed as they were.
    pub carried: usize,
    /// A stopped switch was finished instead, so no shelf came back.
    pub finished: bool,
}

/// A shelf on the list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShelfInfo {
    pub label: String,
    pub save: Id,
    pub message: String,
    /// The line it was shelved on; `None` when that was at a save.
    pub line: Option<String>,
    /// Who shelved it, and when.
    pub saver: Person,
    /// How many files it changes.
    pub files: usize,
}

/// Sets every unsaved change aside on a new shelf, the newest, then puts the folder back to the
/// save the project is on as `discard` does, with the same rules; `message` says what it holds.
/// When nothing is set aside no step is made, and what reading the folder found is noted with the
/// store's warnings. `git_excludes` is read as `goto` reads it.
pub fn shelf(
    repo: &mut Repository,
    message: Option<&str>,
    who: &Person,
    git_excludes: Option<&Path>,
    on_wait: &mut dyn FnMut(&str),
) -> Result<Shelved, SwitchError> {
    let message = message
        .filter(|m| !m.trim().is_empty())
        .unwrap_or(DEFAULT_MESSAGE);
    switch::locked(repo, "shelf", on_wait, |repo, lock, earlier, _| {
        let away = switch::plan_away(repo, None, "shelf", git_excludes, links_left_warning)?;
        let Away {
            head,
            entry,
            from,
            drive,
            mut plan,
            links_left,
        } = away;
        if plan.actions.is_empty() {
            return Ok(set_aside_nothing(repo, plan, earlier, false, links_left));
        }
        within_limit(entry.state.shelves.len() + 1)?;
        crate::fsio::act_point("shelf:planned");
        let mut kept = switch::kept_on(&entry.state, from, "shelf", &[], who);
        kept.message = message.to_owned();
        let mut packs = repo.new_packs()?;
        let (folder_change, _) =
            kept::switch_saves(repo, &plan, from, from, &kept, drive, &mut packs)?;
        // Every file planned changed again before it could be kept, so they all stay.
        let Some(change) = folder_change else {
            let changed: Vec<String> = plan
                .actions
                .iter()
                .filter(|a| a.expected.is_some_and(|e| e.kind != EntryKind::Folder))
                .map(|a| a.path.clone())
                .collect();
            plan.warnings.push(writeout::changed_warning(&changed));
            return Ok(set_aside_nothing(repo, plan, earlier, true, links_left));
        };
        // Stored first so that the shelf can be counted before the step: once the folder is put
        // back, a count that fails mustn't fail the command.
        repo.store_packs(lock, packs)?;
        let files = changed_files(repo, from, change.from)?;
        let mut state = entry.state.clone();
        let label = free_label(&state.shelves);
        state.shelves.insert(0, (label.clone(), change.from));
        let to_make = Switch {
            head,
            action: Action::Command,
            state,
            folder_change,
            plan,
            drive,
            packs: repo.new_packs()?,
        };
        let applied = switch::switch(repo, lock, to_make, who, "shelf", earlier)?;
        Ok(Shelved::Made(ShelfOutcome {
            label,
            save: change.from,
            files,
            applied,
            links_left,
        }))
    })
}

/// Brings back the newest shelf, or the one labelled `label`, onto the save the project is on, as
/// a switch from the save it was shelved on to the shelf that carries other unsaved work along.
/// Where the folder differs from the save shelved on at a path the shelf changes, nothing changes.
/// The shelf leaves the list. After finishing a stopped switch it brings back none.
pub fn unshelf(
    repo: &mut Repository,
    label: Option<&str>,
    who: &Person,
    git_excludes: Option<&Path>,
    on_wait: &mut dyn FnMut(&str),
) -> Result<UnshelfOutcome, SwitchError> {
    switch::locked(repo, "unshelf", on_wait, |repo, lock, earlier, finished| {
        let (head, entry) = repo.current()?;
        let current = tip_of(&entry)?;
        // Running it again after a stop finishes that, and brings back no second shelf.
        if finished {
            return Ok(UnshelfOutcome {
                label: String::new(),
                save: current,
                applied: Applied::default(),
                carried: 0,
                finished: true,
            });
        }
        let asked = match label {
            Some(label) => entry.state.shelves.iter().find(|(l, _)| l == label),
            None => entry.state.shelves.first(),
        };
        let Some((name, shelf)) = asked.cloned() else {
            let label = label.map(str::to_owned);
            return Err(SwitchError::NoShelf { label });
        };
        let base = shelved_on(repo, &name, shelf)?;
        let drive = repo.drive()?;
        let plan = writeout::plan(repo, base, shelf, None, false, drive, git_excludes)?;
        if !plan.collisions.is_empty() {
            // Each kind of path in the way has its own way out.
            let (changed, unsaved) = changed_since(repo, base, current, plan.collisions)?;
            // `goto` finds only the saves a line reaches.
            let base_on_a_line = !changed.is_empty()
                && matches!(resolve(repo, &base.short_hex()), Ok(found) if found == base);
            return Err(SwitchError::ShelfInTheWay(Box::new(ShelfCollisions {
                label: name,
                base,
                base_on_a_line,
                unsaved,
                changed,
            })));
        }
        // Planned from the save shelved on, the plan counts what the save it is on changed.
        let carried = if base == current {
            plan.carried
        } else {
            carried_onto(repo, base, shelf, git_excludes)?
        };
        let mut state = entry.state.clone();
        state.shelves.retain(|(l, _)| *l != name);
        // From the save the project is on with what the folder held, to it with what was written.
        let named = std::slice::from_ref(&name);
        let kept = switch::kept_on(&entry.state, current, "unshelf", named, who);
        let mut packs = repo.new_packs()?;
        let (folder_change, _) =
            kept::switch_saves(repo, &plan, base, shelf, &kept, drive, &mut packs)?;
        let to_make = Switch {
            head,
            action: Action::Command,
            state,
            folder_change,
            plan,
            drive,
            packs,
        };
        let applied = switch::switch(repo, lock, to_make, who, "unshelf", earlier)?;
        Ok(UnshelfOutcome {
            label: name,
            save: shelf,
            applied,
            carried,
            finished: false,
        })
    })
}

/// The shelves, newest first.
pub fn shelves(repo: &Repository) -> Result<Vec<ShelfInfo>, RepoError> {
    let (_, entry) = repo.current()?;
    let (store, signed) = (repo.store(), repo.signatures_allowed());
    let mut found = Vec::with_capacity(entry.state.shelves.len());
    for (label, id) in entry.state.shelves {
        let save = store.save(&id, signed)?;
        let base = match save.parents.first() {
            Some(parent) => Some(store.save(parent, signed)?.root),
            None => None,
        };
        found.push(ShelfInfo {
            label,
            save: id,
            line: save.line().map(str::to_owned),
            files: count_changes(store, base, save.root)?,
            message: save.message,
            saver: save.saver,
        });
    }
    Ok(found)
}

/// The smallest number from 1 that no shelf has as its label.
pub(crate) fn free_label(shelves: &[(String, Id)]) -> String {
    let taken: HashSet<&str> = shelves.iter().map(|(label, _)| label.as_str()).collect();
    let mut n = 1usize;
    loop {
        let label = n.to_string();
        if !taken.contains(label.as_str()) {
            return label;
        }
        n += 1;
    }
}

/// Refuses a list of `count` shelves, more than a diary entry holds.
pub(crate) fn within_limit(count: usize) -> Result<(), SwitchError> {
    if count as u64 > MAX_SHELVES {
        return Err(SwitchError::TooManyShelves);
    }
    Ok(())
}

/// `paths` parted into those the save `now` changed since the save `then`, and the rest. A path
/// counts as changed where the two saves differ at it, or where a folder above it is a folder in
/// only one of them: a collision below a folder a save turned into a file is named below it.
fn changed_since(
    repo: &Repository,
    then: Id,
    now: Id,
    paths: Vec<String>,
) -> Result<(Vec<String>, Vec<String>), StoreError> {
    if then == now {
        return Ok((Vec::new(), paths));
    }
    let (store, signed) = (repo.store(), repo.signatures_allowed());
    let unstored = HashMap::new();
    let mut before = Tree::new(
        store,
        &unstored,
        store.save(&then, signed)?.root,
        kept::EXACT,
    );
    let mut after = Tree::new(
        store,
        &unstored,
        store.save(&now, signed)?.root,
        kept::EXACT,
    );
    let held = |e: Option<Entry>| e.map(|e| (e.kind, e.id));
    let (mut changed, mut rest) = (Vec::new(), Vec::new());
    for path in paths {
        let mut differ = held(before.entry(&path)?) != held(after.entry(&path)?);
        for up in ancestors(&path) {
            if differ {
                break;
            }
            differ = before.folder(up)?.is_some() != after.folder(up)?.is_some();
        }
        if differ {
            changed.push(path);
        } else {
            rest.push(path);
        }
    }
    Ok((changed, rest))
}

/// Nothing set aside, with what reading the folder found noted with the store's warnings.
fn set_aside_nothing(
    repo: &Repository,
    plan: Plan,
    earlier: &mut Earlier,
    changed_again: bool,
    links_left: Vec<String>,
) -> Shelved {
    for w in switch::switch_warnings(plan, Applied::default(), earlier).warnings {
        repo.store().note(w);
    }
    Shelved::Nothing {
        changed_again,
        links_left,
    }
}

/// How many files differ between the saves `from` and `to`.
fn changed_files(repo: &Repository, from: Id, to: Id) -> Result<usize, StoreError> {
    let root = |id: &Id| -> Result<Id, StoreError> {
        Ok(repo.store().save(id, repo.signatures_allowed())?.root)
    };
    count_changes(repo.store(), Some(root(&from)?), root(&to)?)
}

/// The save the shelf `shelf`, labelled `label`, was made on: its save's first parent.
fn shelved_on(repo: &Repository, label: &str, shelf: Id) -> Result<Id, SwitchError> {
    let save = repo.store().save(&shelf, repo.signatures_allowed())?;
    match save.parents.first() {
        Some(base) => Ok(*base),
        None => Err(damaged(format!("the save of shelf {label} has no parent")).into()),
    }
}

/// How many files hold unsaved changes against the save the project is on that an unshelf of
/// `shelf`, made on another save `base`, leaves as they are: those where the shelf changes nothing.
fn carried_onto(
    repo: &Repository,
    base: Id,
    shelf: Id,
    git_excludes: Option<&Path>,
) -> Result<usize, SwitchError> {
    let root = |id: &Id| -> Result<Id, StoreError> {
        Ok(repo.store().save(id, repo.signatures_allowed())?.root)
    };
    let mut shelved = BTreeSet::new();
    for_each_change(repo.store(), Some(root(&base)?), root(&shelf)?, |c| {
        shelved.insert(c.path.to_owned());
        Ok::<(), StoreError>(())
    })?;
    let now = crate::status::status(repo, false, git_excludes)?;
    Ok(now
        .changes
        .iter()
        .filter(|c| !shelved.contains(&c.path))
        .count())
}

/// Says unsaved links weren't shelved.
fn links_left_warning(paths: &[String]) -> Warning {
    let list = listed(paths.iter().take(NAMED), paths.len() as u64);
    let message = match paths.len() {
        1 => format!(
            "Didn't shelve 1 unsaved link: it could come back only as a plain file holding its \
             target, {}:{list}\nIt stays in your folder as it is: delete it yourself if you \
             don't want it there.",
            switch::why_links_stay(true)
        ),
        n => format!(
            "Didn't shelve {n} unsaved links: they could come back only as plain files holding \
             their targets, {}:{list}\nThey stay in your folder as they are: delete them yourself \
             if you don't want them there.",
            switch::why_links_stay(false)
        ),
    };
    Warning::new(warning::LINKS_LEFT, message)
}

#[cfg(test)]
mod tests;
