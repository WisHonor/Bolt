//! `bolt undo` and `bolt redo`: stepping back and forth through the commands the diary records,
//! with what each did to the folder written back or written again.

use std::collections::BTreeSet;
use std::fmt;
use std::path::Path;

use crate::diary::{Action, DiaryEntry, FolderChange, Position, State};
use crate::drive::Drive;
use crate::history::tip_of;
use crate::id::Id;
use crate::kept;
use crate::repo::{ChangeLock, Repository};
use crate::save::Person;
use crate::store::{StoreError, is_damage};
use crate::switch::{self, Earlier, Switch, SwitchError};
use crate::timeline;
use crate::warning::Warning;
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

impl fmt::Display for UndoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.error.fmt(f)
    }
}

impl std::error::Error for UndoError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.error.source()
    }
}

/// The warnings of the steps taken, as one command's: each told once, with one for every entry
/// left unwritten and one for every link written as a file, whichever step they came from.
#[must_use]
pub fn warnings(taken: &[Taken]) -> Vec<Warning> {
    let (mut not_written, mut placeholders) = (BTreeSet::new(), BTreeSet::new());
    let mut told: Vec<Warning> = Vec::new();
    for t in taken {
        not_written.extend(t.applied.not_written.iter().cloned());
        placeholders.extend(t.applied.placeholders.iter().cloned());
        for w in &t.applied.warnings {
            if !switch::LISTED.contains(&w.kind) && !told.contains(w) {
                told.push(w.clone());
            }
        }
    }
    let listed = |paths: BTreeSet<String>| paths.into_iter().collect::<Vec<_>>();
    if !not_written.is_empty() {
        told.push(writeout::not_written_warning(&listed(not_written)));
    }
    if !placeholders.is_empty() {
        told.push(writeout::placeholders_warning(&listed(placeholders)));
    }
    told
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
        let mut read = |id: &Id| repo.entry(id);
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
        Some(id) => Some(repo.entry(&id)?.state),
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
    let (mut shelf, mut planned, mut folder_change) = (None, None, None);
    if let Some(change) = entry.folder_change {
        saves_readable(repo, step, change)?;
        // Undo switches from what the step wrote back to what the folder held, redo the other
        // way, and each records that switch: the step's two saves picture the folder before and
        // after it, at exactly its paths.
        let switched = if redo {
            change
        } else {
            FolderChange {
                from: change.to,
                to: change.from,
            }
        };
        let (mut from, to) = (switched.from, switched.to);
        let drive = repo.drive()?;
        let ignored = req.git_excludes;
        let mut plan = writeout::plan(repo, from, to, None, false, drive, ignored)?;
        if !plan.collisions.is_empty() && req.shelve {
            let shelving = Shelving {
                current: tip_of(&now)?,
                from,
                state: &now.state,
                req,
                redo,
                drive,
            };
            let (made, from_with_it) = shelving.set_aside(repo, lock, &plan.collisions)?;
            let label = free_label(&state.shelves);
            state.shelves.insert(0, (label.clone(), made));
            shelf = Some((label, made));
            from = from_with_it;
            plan = writeout::plan(repo, from, to, None, false, drive, ignored)?;
        }
        if !plan.collisions.is_empty() {
            return Err(SwitchError::InTheWayOfStep {
                paths: plan.collisions,
                redo,
                shelve: req.shelve,
            });
        }
        folder_change = Some(switched);
        planned = Some((plan, drive));
    }
    let packs = repo.new_packs()?;
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
struct Shelving<'a> {
    /// The save the project is on, which the shelf is made on.
    current: Id,
    /// The save the step's switch is planned from.
    from: Id,
    state: &'a State,
    req: &'a StepRequest<'a>,
    redo: bool,
    drive: Drive,
}

impl Shelving<'_> {
    /// Makes the shelf of the unsaved work at `collisions`, and the save to plan the step from
    /// instead, holding that work too so that the step overwrites it. Both are stored at once,
    /// for the planner to read, unless something in the way can't be set aside (a file Bolt
    /// ignores, or an unsaved link that could come back only as a plain file): then nothing is
    /// stored, and the error names what can't be.
    fn set_aside(
        &self,
        repo: &mut Repository,
        lock: &ChangeLock,
        collisions: &[String],
    ) -> Result<(Id, Id), SwitchError> {
        let (current, drive, command) = (self.current, self.drive, command(self.redo));
        // A discard of those paths throws away exactly the unsaved work there.
        let ignored = self.req.git_excludes;
        let mut work = writeout::plan(
            repo,
            current,
            current,
            Some(collisions),
            true,
            drive,
            ignored,
        )?;
        // Links that could come back only as plain files stay where they are.
        let links = kept::links_to_leave(repo, &work.actions, current, drive)?;
        let stays = kept::goes_with(links.iter().map(String::as_str), drive);
        work.actions.retain(|a| !stays(&a.path));
        let who = self.req.who;
        let mut shelf = switch::kept_on(self.state, current, command, &[], who);
        shelf.message = format!("Set aside by {command}");
        let onto = switch::kept_on(self.state, self.from, command, &[], who);
        let mut packs = repo.new_packs()?;
        let aside = kept::set_aside(repo, &work.actions, &shelf, &onto, drive, &mut packs)?;
        // A collision is out of the way when something was set aside at it, below it or above
        // it, and no link left there is; paths compared as the drive compares names.
        let left: Vec<String> = {
            let paths = aside
                .iter()
                .flat_map(|a| a.paths.iter().map(String::as_str));
            let taken_out = kept::goes_with(paths, drive);
            collisions
                .iter()
                .filter(|c| !taken_out(c) || stays(c))
                .cloned()
                .collect()
        };
        match aside {
            Some(aside) if left.is_empty() => {
                repo.store_packs(lock, packs)?;
                Ok((aside.shelf, aside.step_from))
            }
            _ => Err(SwitchError::InTheWayOfStep {
                paths: left,
                redo: self.redo,
                shelve: true,
            }),
        }
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

/// Reads the two saves of a step's folder change before anything is planned. One the store
/// lacks, can't decode or holds only damaged copies of is gone (`tidy` removed it, or it was
/// lost), and the step can't be taken; any other failure is told as itself.
fn saves_readable(repo: &Repository, step: Id, change: FolderChange) -> Result<(), SwitchError> {
    for save in [change.from, change.to] {
        let Err(e) = repo.store().save(&save, repo.signatures_allowed()) else {
            continue;
        };
        let gone = match &e {
            StoreError::Missing(_) | StoreError::Decode(_) | StoreError::WrongKind { .. } => true,
            StoreError::Pack(e) => is_damage(e),
            StoreError::SaveTooLarge { .. } => false,
        };
        return Err(if gone {
            SwitchError::StepGone {
                step,
                detail: e.to_string(),
            }
        } else {
            e.into()
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests;
