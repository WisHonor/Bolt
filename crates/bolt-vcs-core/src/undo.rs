//! `bolt undo` and `bolt redo`: stepping back and forth through the commands the diary records,
//! with what each did to the folder written back or written again.

use std::collections::BTreeSet;
use std::path::Path;

use crate::diary::{Action, DiaryEntry, FolderChange, Position, State};
use crate::drive::Drive;
use crate::history::tip_of;
use crate::id::Id;
use crate::kept;
use crate::pack::ObjectKind;
use crate::repo::{ChangeLock, RepoError, Repository, damaged};
use crate::save::Person;
use crate::snapshot::SaveError;
use crate::store::StoreError;
use crate::switch::{self, Earlier, Switch, SwitchError};
use crate::timeline;
use crate::writeout::{self, Applied};

/// What one step taken did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Taken {
    /// The command entry undone or redone.
    pub step: Id,
    /// That step's own entry.
    pub entry: DiaryEntry,
    pub before: State,
    pub after: State,
    /// What changed, for messages and lists.
    pub effects: Vec<Effect>,
    /// What the switch wrote and removed; empty when it changed no file.
    pub applied: Applied,
}

/// One thing a step changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    /// A line made (`from` is `None`), moved, or gone (`to` is `None`).
    Line {
        name: String,
        from: Option<Id>,
        to: Option<Id>,
    },
    Position {
        from: Position,
        to: Position,
    },
    ShelfAdded {
        label: String,
        save: Id,
    },
    ShelfGone {
        label: String,
        save: Id,
    },
    Tags,
    ServerLines,
    /// The folder was switched from one save to the other.
    Files {
        from: Id,
        to: Id,
    },
}

/// What changed between two states and through a folder change.
#[must_use]
pub fn effects(before: &State, after: &State, folder: Option<FolderChange>) -> Vec<Effect> {
    let mut found = Vec::new();
    let names: BTreeSet<&String> = before.lines.keys().chain(after.lines.keys()).collect();
    for name in names {
        let (from, to) = (before.lines.get(name), after.lines.get(name));
        if from != to {
            found.push(Effect::Line {
                name: name.clone(),
                from: from.copied(),
                to: to.copied(),
            });
        }
    }
    if before.position != after.position {
        found.push(Effect::Position {
            from: before.position.clone(),
            to: after.position.clone(),
        });
    }
    let holds = |state: &State, save: &Id| state.shelves.iter().any(|(_, s)| s == save);
    for (label, save) in &after.shelves {
        if !holds(before, save) {
            let (label, save) = (label.clone(), *save);
            found.push(Effect::ShelfAdded { label, save });
        }
    }
    for (label, save) in &before.shelves {
        if !holds(after, save) {
            let (label, save) = (label.clone(), *save);
            found.push(Effect::ShelfGone { label, save });
        }
    }
    if before.tags != after.tags {
        found.push(Effect::Tags);
    }
    if before.server_lines != after.server_lines {
        found.push(Effect::ServerLines);
    }
    if let Some(FolderChange { from, to }) = folder {
        found.push(Effect::Files { from, to });
    }
    found
}

/// What `undo` or `redo` did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UndoOutcome {
    pub taken: Vec<Taken>,
    /// How many steps were asked for.
    pub asked: usize,
    /// A stopped switch was finished, so no step was taken.
    pub finished: bool,
    /// The shelves `--shelve` made, by label, oldest first: one for each step it was needed for.
    pub shelved: Vec<(String, Id)>,
}

/// Why `undo` or `redo` stopped, with the steps taken, and shelves made, before it did.
#[derive(Debug)]
pub struct UndoError {
    pub taken: Vec<Taken>,
    pub shelved: Vec<(String, Id)>,
    pub error: SwitchError,
}

/// What to undo or redo, and how.
pub struct StepRequest<'a> {
    /// How many steps, one diary entry each.
    pub n: usize,
    /// Set unsaved work in the way aside on a new shelf, as part of the step.
    pub shelve: bool,
    pub who: &'a Person,
    /// The user's own Git ignore list, read as a save reads it.
    pub git_excludes: Option<&'a Path>,
}

/// Undoes the newest `req.n` steps, one at a time: each puts back the state from just before
/// it and writes back what the folder held where it wrote. Running out at the diary's first
/// entry is success. After finishing a stopped switch it takes no step at all.
pub fn undo(
    repo: &mut Repository,
    req: &StepRequest,
    on_wait: &mut dyn FnMut(&str),
) -> Result<UndoOutcome, UndoError> {
    take(repo, req, on_wait, false)
}

/// Redoes the `req.n` steps undone last, as [`undo`] undoes them.
pub fn redo(
    repo: &mut Repository,
    req: &StepRequest,
    on_wait: &mut dyn FnMut(&str),
) -> Result<UndoOutcome, UndoError> {
    take(repo, req, on_wait, true)
}

/// A diary entry's ID as lists and messages show it: its first 12 hex digits.
#[must_use]
pub fn short_entry(id: &Id) -> String {
    let mut hex = id.to_hex();
    hex.truncate(12);
    hex
}

fn take(
    repo: &mut Repository,
    req: &StepRequest,
    on_wait: &mut dyn FnMut(&str),
    redo: bool,
) -> Result<UndoOutcome, UndoError> {
    let (mut taken, mut shelved) = (Vec::new(), Vec::new());
    let done = switch::locked(
        repo,
        command(redo),
        on_wait,
        |repo, lock, earlier, finished| {
            // Running the command again after a stop finishes it, and no more.
            if finished {
                return Ok(true);
            }
            for _ in 0..req.n {
                let Some((step, shelf)) = take_one(repo, lock, req, redo, earlier)? else {
                    break;
                };
                taken.push(step);
                shelved.extend(shelf);
            }
            Ok(false)
        },
    );
    match done {
        Ok(finished) => Ok(UndoOutcome {
            taken,
            asked: req.n,
            finished,
            shelved,
        }),
        Err(error) => Err(UndoError {
            taken,
            shelved,
            error,
        }),
    }
}

fn command(redo: bool) -> &'static str {
    if redo { "redo" } else { "undo" }
}

/// A shelf: its label and its save.
type Shelf = (String, Id);

/// Takes the next step, if there is one, as an entry of its own, with the shelf `--shelve` made
/// for it.
fn take_one(
    repo: &mut Repository,
    lock: &ChangeLock,
    req: &StepRequest,
    redo: bool,
    earlier: &mut Earlier,
) -> Result<Option<(Taken, Option<Shelf>)>, SwitchError> {
    let (head, now) = repo.current()?;
    let next = {
        let mut read = |id: &Id| read_entry(repo, id);
        if redo {
            timeline::undone_steps(head, &mut read)?.into_iter().next()
        } else {
            timeline::newest_done(head, &mut read)?
        }
    };
    let Some((step, entry)) = next else {
        return Ok(None);
    };
    let parent = match entry.parent {
        Some(id) => Some(read_entry(repo, &id)?.state),
        None => None,
    };
    // Undo puts back the state before the step, and redo the step's own. The diary's first
    // entry has none before it, and can't be undone.
    let Some(source) = (if redo {
        Some(&entry.state)
    } else {
        parent.as_ref()
    }) else {
        return Ok(None);
    };
    let mut state = State {
        lines: source.lines.clone(),
        tags: source.tags,
        position: source.position.clone(),
        shelves: now.state.shelves.clone(),
        server_lines: source.server_lines,
    };
    let before_step = parent.as_ref().map_or(&[][..], |p| p.shelves.as_slice());
    restore_shelves(&mut state.shelves, before_step, &entry.state.shelves, redo);

    let action = if redo {
        Action::Redo(step)
    } else {
        Action::Undo(step)
    };
    let command = command(redo);
    let (mut shelf, mut planned, mut folder_change, mut packs) = (None, None, None, None);
    if let Some(change) = entry.folder_change {
        // Undo switches from what the step wrote back to what the folder held, redo the other way.
        let (mut from, to) = if redo {
            (change.from, change.to)
        } else {
            (change.to, change.from)
        };
        let drive = repo.drive()?;
        let ignored = req.git_excludes;
        let mut plan = writeout::plan(repo, from, to, None, false, drive, ignored)
            .map_err(|e| step_gone(step, e))?;
        if !plan.collisions.is_empty() && req.shelve {
            let aside = SetAside {
                current: tip_of(&now)?,
                from,
                state: &now.state,
                req,
                command,
                drive,
            };
            if let Some((made, from_with_it)) = aside.make(repo, lock, &plan.collisions)? {
                let label = free_label(&state.shelves);
                state.shelves.insert(0, (label.clone(), made));
                shelf = Some((label, made));
                from = from_with_it;
                plan = writeout::plan(repo, from, to, None, false, drive, ignored)
                    .map_err(|e| step_gone(step, e))?;
            }
        }
        if !plan.collisions.is_empty() {
            return Err(SwitchError::InTheWayOfStep {
                paths: plan.collisions,
                redo,
            });
        }
        let mut made = repo.new_packs()?;
        let kept_from = switch::kept_on(&now.state, from, command, &[], req.who);
        folder_change =
            kept::switch_saves(repo, &plan.actions, to, &kept_from, drive, &mut made)?.0;
        packs = Some(made);
        planned = Some((plan, drive));
    }
    let packs = match packs {
        Some(packs) => packs,
        None => repo.new_packs()?,
    };
    let applied = match planned {
        Some((plan, drive)) if !plan.actions.is_empty() => {
            let to_make = Switch {
                head,
                action,
                state: state.clone(),
                folder_change,
                plan,
                drive,
                packs,
            };
            switch::switch(repo, lock, to_make, req.who, command, earlier)?
        }
        // A step that changed no file changes none undone or redone, and nor does one whose
        // files are already as it would leave them.
        planned => {
            let made = DiaryEntry {
                parent: Some(head),
                action,
                command: command.to_owned(),
                who: req.who.clone(),
                bolt_version: env!("CARGO_PKG_VERSION").into(),
                state: state.clone(),
                folder_change,
            };
            repo.commit(lock, packs, &made)?;
            match planned {
                Some((plan, _)) => switch::switch_warnings(plan, Applied::default(), earlier),
                None => Applied::default(),
            }
        }
    };
    let effects = effects(&now.state, &state, folder_change);
    let taken = Taken {
        step,
        entry,
        before: now.state,
        after: state,
        effects,
        applied,
    };
    Ok(Some((taken, shelf)))
}

/// What `--shelve` needs to set the unsaved work in a step's way aside.
struct SetAside<'a> {
    /// The save the project is on, which the shelf is made on.
    current: Id,
    /// The save the step's switch is planned from.
    from: Id,
    state: &'a State,
    req: &'a StepRequest<'a>,
    command: &'static str,
    drive: Drive,
}

impl SetAside<'_> {
    /// Makes the shelf of the unsaved work at `collisions`, and the save to plan the step from
    /// instead, holding that work too so that the step overwrites it; both are stored at once for
    /// the planner to read. `None` when none of it is unsaved work.
    fn make(
        &self,
        repo: &mut Repository,
        lock: &ChangeLock,
        collisions: &[String],
    ) -> Result<Option<(Id, Id)>, SwitchError> {
        let (current, drive) = (self.current, self.drive);
        // A discard of those paths throws away exactly the unsaved work there.
        let mut work = writeout::plan(
            repo,
            current,
            current,
            Some(collisions),
            true,
            drive,
            self.req.git_excludes,
        )?;
        // Links that could come back only as plain files stay, and still stop the step.
        let links = kept::links_to_leave(repo, &work.actions, current, drive)?;
        let stays = kept::goes_with(links.iter().map(String::as_str), drive);
        work.actions.retain(|a| !stays(&a.path));
        let who = self.req.who;
        let mut shelf = switch::kept_on(self.state, current, self.command, &[], who);
        shelf.message = format!("Set aside by {}", self.command);
        let onto = switch::kept_on(self.state, self.from, self.command, &[], who);
        let mut packs = repo.new_packs()?;
        let made = kept::set_aside(repo, &work.actions, &shelf, &onto, drive, &mut packs)?;
        if made.is_some() {
            repo.store_packs(lock, packs)?;
        }
        Ok(made)
    }
}

/// Takes off the shelves a step added and puts back the ones it removed, for an undo, and the
/// other way round for a redo: `parent` and `step` are the shelves before and after the step,
/// compared by save. One put back keeps its label unless another shelf has it.
fn restore_shelves(
    shelves: &mut Vec<(String, Id)>,
    parent: &[(String, Id)],
    step: &[(String, Id)],
    redo: bool,
) {
    let missing = |from: &[(String, Id)], there: &[(String, Id)]| -> Vec<(String, Id)> {
        let gone = |save: &Id| !there.iter().any(|(_, s)| s == save);
        from.iter()
            .filter(|(_, save)| gone(save))
            .cloned()
            .collect()
    };
    let (added, removed) = (missing(step, parent), missing(parent, step));
    let (off, back) = if redo {
        (removed, added)
    } else {
        (added, removed)
    };
    shelves.retain(|(_, save)| !off.iter().any(|(_, s)| s == save));
    // Back as the newest.
    let mut at = 0;
    for (label, save) in back {
        if shelves.iter().any(|(_, s)| *s == save) {
            continue;
        }
        let label = if shelves.iter().any(|(l, _)| *l == label) {
            free_label(shelves)
        } else {
            label
        };
        shelves.insert(at, (label, save));
        at += 1;
    }
}

/// The smallest number from 1 that no shelf has as its label.
fn free_label(shelves: &[(String, Id)]) -> String {
    let mut n = 1usize;
    while shelves.iter().any(|(label, _)| *label == n.to_string()) {
        n += 1;
    }
    n.to_string()
}

/// A diary entry the walk needs. One the store lacks or can't decode is damage, named.
fn read_entry(repo: &Repository, id: &Id) -> Result<DiaryEntry, RepoError> {
    let bytes = match repo.store().read_record(id, ObjectKind::DiaryEntry) {
        Ok(bytes) => bytes,
        Err(StoreError::Missing(_)) => {
            return Err(damaged(format!("the diary has no entry {id}")));
        }
        Err(e) => return Err(e.into()),
    };
    DiaryEntry::decode(&bytes).map_err(|e| damaged(format!("diary entry {id} can't be read: {e}")))
}

/// A step's saves the store can't give while planning: `tidy` removed them, or they were lost.
fn step_gone(step: Id, e: SaveError) -> SwitchError {
    match e {
        SaveError::Repo(RepoError::Store(_) | RepoError::Damaged { .. } | RepoError::Decode(_)) => {
            SwitchError::StepGone {
                step,
                detail: e.to_string(),
            }
        }
        e => e.into(),
    }
}

#[cfg(test)]
mod tests;
