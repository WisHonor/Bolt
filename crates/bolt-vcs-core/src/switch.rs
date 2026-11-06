//! `bolt goto`: moving the project folder to another save, carrying unsaved work along unless
//! it is in the way (decisions 049, 050 and 055).

use std::path::Path;

use crate::diary::{Action, DiaryEntry, Position, State};
use crate::drive::{self, Drive};
use crate::history::{ResolveError, tip_of};
use crate::id::Id;
use crate::journal::Journal;
use crate::lines::Target;
use crate::repo::{ChangeLock, RepoError, Repository};
use crate::save::Person;
use crate::snapshot::{NAMED, SaveError, listed};
use crate::store::StoreError;
use crate::warning;
use crate::writeout::{self, Applied, Plan};

/// What `goto` did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GotoOutcome {
    pub position: Position,
    /// The save the folder is now on.
    pub save: Id,
    /// What was written and removed. Its `placeholders` are every link written as a file, and
    /// its `warnings` all the switch's own.
    pub applied: Applied,
    /// Entries of the save this computer can't hold, left unwritten.
    pub not_written: Vec<String>,
    pub safety_copies: usize,
    /// Unsaved changes that stayed in the folder.
    pub carried: usize,
}

/// Why a switch didn't happen. Paths come from the folder, so messages escape them.
#[derive(Debug, thiserror::Error)]
pub enum SwitchError {
    #[error(
        "Your unsaved changes are in the way, so nothing was changed:{}",
        listed(paths.iter().take(NAMED), paths.len() as u64)
    )]
    Collisions { paths: Vec<String> },
    #[error(transparent)]
    Resolve(#[from] ResolveError),
    #[error(transparent)]
    Save(#[from] SaveError),
    #[error(transparent)]
    Repo(#[from] RepoError),
}

impl From<StoreError> for SwitchError {
    fn from(e: StoreError) -> Self {
        SwitchError::Repo(e.into())
    }
}

impl SwitchError {
    /// What the user can do next, if there is something to say.
    #[must_use]
    pub fn hint(&self) -> Option<String> {
        match self {
            // Shelving comes in milestone 4.
            SwitchError::Collisions { .. } => Some(
                "Save them first (bolt save \"describe your change\") or discard them, then run \
                 bolt goto again."
                    .to_owned(),
            ),
            SwitchError::Resolve(e) => e.hint(),
            SwitchError::Save(e) => e.hint_when_reading(),
            SwitchError::Repo(e) => e.hint(),
        }
    }
}

/// Moves the project to `target`. Unsaved work the switch would overwrite stops it before
/// anything changes; the rest stays in the folder. `git_excludes` is the user's own Git ignore
/// list, read as a save reads it.
pub fn goto(
    repo: &mut Repository,
    target: &Target,
    who: &Person,
    command: &str,
    git_excludes: Option<&Path>,
    on_wait: &mut dyn FnMut(&str),
) -> Result<GotoOutcome, SwitchError> {
    let lock = repo.lock_for_change(command, on_wait)?;
    // Kept with the store's warnings, so it is told even if the goto then fails.
    if let Some(done) = writeout::finish_interrupted(repo, &lock)? {
        for w in done.warnings {
            repo.store().note(w);
        }
    }
    let (head, entry) = repo.current()?;
    let from = tip_of(&entry)?;
    // Read again under the lock: another command may have moved the line.
    let (position, to) = match target {
        Target::Line(line) => match entry.state.lines.get(line) {
            Some(save) => (Position::OnLine(line.clone()), *save),
            None => return Err(ResolveError::NotFound(line.clone()).into()),
        },
        Target::ServerLine { save, .. } | Target::Save(save) => (Position::AtSave(*save), *save),
    };
    let drive = drive::probe(repo.tmp_folder()).map_err(RepoError::from)?;
    let plan = writeout::plan(repo, from, to, None, false, drive, git_excludes)?;
    if !plan.collisions.is_empty() {
        return Err(SwitchError::Collisions {
            paths: plan.collisions,
        });
    }
    let mut state = entry.state.clone();
    state.position = position.clone();
    let carried = plan.carried;
    let not_written = plan.not_written.clone();
    let (applied, safety_copies) = if state == entry.state && plan.actions.is_empty() {
        // Already there: nothing to record.
        (switch_warnings(plan, Applied::default()), 0)
    } else {
        let to_make = Switch {
            head,
            from,
            state,
            plan,
            drive,
        };
        switch(repo, &lock, to_make, who, command)?
    };
    Ok(GotoOutcome {
        position,
        save: to,
        applied,
        not_written,
        safety_copies,
        carried,
    })
}

/// A switch planned, ready to be made.
struct Switch {
    /// The diary entry `head` names, and the save the folder is on.
    head: Id,
    from: Id,
    /// The state the switch's diary entry records.
    state: State,
    plan: Plan,
    drive: Drive,
}

/// Makes a planned switch in decision 049's order: the safety copies and the diary entry are
/// stored, then the journal written, the folder changed, `head` moved and the journal removed.
/// A crash at any step leaves what the next changing command finishes. Gives what was done and
/// how many safety copies were kept.
fn switch(
    repo: &mut Repository,
    lock: &ChangeLock,
    s: Switch,
    who: &Person,
    command: &str,
) -> Result<(Applied, usize), SwitchError> {
    let mut packs = repo.new_packs()?;
    let safety_copies =
        writeout::keep_safety_copies(repo, &s.plan.actions, s.from, s.drive, &mut packs)?;
    let kept = safety_copies.len();
    let entry = DiaryEntry {
        parent: Some(s.head),
        action: Action::Command,
        command: command.to_owned(),
        who: who.clone(),
        bolt_version: env!("CARGO_PKG_VERSION").into(),
        state: s.state,
        safety_copies,
    };
    let target = repo.store_entry(lock, packs, &entry)?;
    let journal = Journal {
        target,
        from: s.head,
        actions: s.plan.actions.clone(),
    };
    journal.write(repo, lock)?;
    let applied = writeout::carry_out(repo, lock, &journal, s.drive)?;
    crate::merge::merge_after_commit(repo, lock);
    Ok((switch_warnings(s.plan, applied), kept))
}

/// `applied` with the plan's warnings first, and one warning for every link written as a file,
/// whether the plan chose it or this computer couldn't make the link.
fn switch_warnings(plan: Plan, mut applied: Applied) -> Applied {
    let mut warnings = plan.warnings;
    if !plan.not_written.is_empty() {
        warnings.push(writeout::not_written_warning(&plan.not_written));
    }
    let mut placeholders = plan.placeholders;
    placeholders.append(&mut applied.placeholders);
    placeholders.sort_unstable();
    placeholders.dedup();
    if !placeholders.is_empty() {
        warnings.push(writeout::placeholders_warning(&placeholders));
    }
    applied.placeholders = placeholders;
    warnings.extend(
        applied
            .warnings
            .drain(..)
            .filter(|w| w.kind != warning::PLACEHOLDERS_WRITTEN),
    );
    applied.warnings = warnings;
    applied
}

#[cfg(test)]
mod tests;
