//! `bolt goto`: moving the project folder to another save, carrying unsaved work along unless
//! it is in the way, and `bolt discard`: throwing unsaved work away (decisions 049, 050 and 055).

use std::path::Path;

use crate::diary::{Action, DiaryEntry, FolderChange, Position, State};
use crate::drive::{self, Drive};
use crate::history::{ResolveError, tip_of};
use crate::id::Id;
use crate::journal::{Change, Journal};
use crate::kept::{self, KeptFrom};
use crate::lines::Target;
use crate::listing::EntryKind;
use crate::pack::PackSet;
use crate::repo::{ChangeLock, RepoError, Repository};
use crate::save::Person;
use crate::scope::within;
use crate::snapshot::{NAMED, SaveError, listed};
use crate::store::StoreError;
use crate::text::escape;
use crate::undo::short_entry;
use crate::warning::{self, Warning};
use crate::writeout::{self, Applied, Plan};

/// What `goto` did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GotoOutcome {
    pub position: Position,
    /// The save the folder is now on.
    pub save: Id,
    /// What was written and removed. Its `placeholders` are every link written as a file, its
    /// `not_written` every entry left unwritten, and its `warnings` all the switch's own.
    pub applied: Applied,
    pub safety_copies: usize,
    /// Unsaved changes that came along to the new save; 0 when there was nothing to switch.
    pub carried: usize,
}

/// What `discard` did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscardOutcome {
    /// Files and links put back as the save has them: changed or deleted ones.
    pub restored: Vec<String>,
    /// New files and links removed, and those a saved folder went back in place of.
    pub removed: Vec<String>,
    /// Folders made or removed that no file above counts for.
    pub folders: usize,
    /// What was written and removed, as `goto` gives it.
    pub applied: Applied,
    pub safety_copies: usize,
    /// From the save of what was thrown away to the save the project is on; `None` if nothing was.
    pub folder_change: Option<FolderChange>,
    /// Unsaved links left as they are, since they could come back only as files.
    pub links_left: Vec<String>,
}

/// Why a switch didn't happen. Paths come from the folder, so messages escape them.
#[derive(Debug, thiserror::Error)]
pub enum SwitchError {
    #[error(
        "Your unsaved changes, or files Bolt ignores, are in the way, so nothing was changed:{}",
        listed(paths.iter().take(NAMED), paths.len() as u64)
    )]
    Collisions { paths: Vec<String> },
    #[error(
        "Something is in the way where discarded files would go back (ignored, or new since \
         Bolt looked), so nothing was changed:{}",
        listed(paths.iter().take(NAMED), paths.len() as u64)
    )]
    InTheWay { paths: Vec<String> },
    #[error(
        "{} could be any of these, so nothing was changed:{}",
        escape(path, false),
        listed(spellings.iter().take(NAMED), spellings.len() as u64)
    )]
    Ambiguous {
        path: String,
        spellings: Vec<String>,
    },
    #[error(
        "Nothing in your last save or your unsaved changes is at {}, so nothing was changed.",
        escape(path, false)
    )]
    NoMatch { path: String },
    #[error(
        "Your unsaved changes, or files Bolt ignores, are where the {} would write, so it changed \
         nothing:{}",
        if *redo { "redo" } else { "undo" },
        listed(paths.iter().take(NAMED), paths.len() as u64)
    )]
    InTheWayOfStep { paths: Vec<String>, redo: bool },
    /// The saves of a step can't be read: `tidy` removed them, or they were lost.
    #[error(
        "The step of diary entry {} can't be undone or redone: what it changed can't be read \
         ({detail}).",
        short_entry(step)
    )]
    StepGone { step: Id, detail: String },
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
                "Save them first (bolt save \"describe your change\") or discard them, and move \
                 ignored files somewhere else, then run bolt goto again."
                    .to_owned(),
            ),
            SwitchError::InTheWay { .. } => {
                Some("Move them somewhere else, then run bolt discard again.".to_owned())
            }
            SwitchError::Ambiguous { .. } => {
                Some("Name the one you mean, spelled exactly as it is saved.".to_owned())
            }
            SwitchError::NoMatch { .. } => {
                Some("Check the path: bolt status lists your unsaved changes.".to_owned())
            }
            SwitchError::InTheWayOfStep { redo, .. } => {
                let (command, why) = if *redo {
                    (
                        "redo",
                        "each is a new step, and a new step clears what redo can bring back",
                    )
                } else {
                    (
                        "undo",
                        "each is a step of its own, which the next undo would take back",
                    )
                };
                Some(format!(
                    "Run bolt {command} --shelve to set your changes there aside on a new shelf, \
                     as part of the same step. Saving, shelving or discarding them first won't \
                     help: {why}. Files Bolt ignores, and unsaved links that could come back only \
                     as plain files, aren't set aside: move them somewhere else first."
                ))
            }
            SwitchError::StepGone { .. } => Some(
                "bolt tidy removes what steps older than the undo window changed, and a damaged \
                 store can lose it too, so undo and redo can't take this step any more."
                    .to_owned(),
            ),
            SwitchError::Resolve(e) => e.hint(),
            SwitchError::Save(e) => e.hint_when_reading(),
            SwitchError::Repo(e) => e.hint(),
        }
    }
}

/// The warnings that name entries left unwritten or links written as files: one of each per
/// command, whichever switch they came from.
const LISTED: [&str; 2] = [warning::PLACEHOLDERS_WRITTEN, warning::NOT_WRITTEN_HERE];

/// Moves the project to `target`. Unsaved work the switch would overwrite stops it before
/// anything changes; the rest stays in the folder. `git_excludes` is the user's own Git ignore
/// list, read as a save reads it. Files another program holds stop it with `SwitchInUse`, the
/// journal kept for the next command to finish.
pub fn goto(
    repo: &mut Repository,
    target: &Target,
    who: &Person,
    command: &str,
    git_excludes: Option<&Path>,
    on_wait: &mut dyn FnMut(&str),
) -> Result<GotoOutcome, SwitchError> {
    locked(repo, command, on_wait, |repo, lock, earlier, _| {
        go(repo, lock, target, who, command, git_excludes, earlier)
    })
}

/// Makes `paths` (project paths, a folder standing for everything in it), or the whole folder
/// without any, match the save the project is on: changed and deleted files put back, new ones
/// removed, with a safety copy of what each changed or new one held (decision 050). The project
/// stays on its save; its diary entry records the switch from a save of what was thrown away.
/// Ignored files, project links, entries kept unwritten and nested projects are never touched,
/// nor are unsaved links that could come back only as files. Files in use stop it as they stop
/// `goto`. `git_excludes` is read as `goto` reads it.
pub fn discard(
    repo: &mut Repository,
    paths: Option<&[String]>,
    who: &Person,
    command: &str,
    git_excludes: Option<&Path>,
    on_wait: &mut dyn FnMut(&str),
) -> Result<DiscardOutcome, SwitchError> {
    locked(repo, command, on_wait, |repo, lock, earlier, _| {
        throw_away(repo, lock, paths, who, command, git_excludes, earlier)
    })
}

/// Runs `f` under the lock for changes, once a switch that was stopped is finished; `f` is told
/// whether there was one.
pub(crate) fn locked<T>(
    repo: &mut Repository,
    command: &str,
    on_wait: &mut dyn FnMut(&str),
    f: impl FnOnce(&mut Repository, &ChangeLock, &mut Earlier, bool) -> Result<T, SwitchError>,
) -> Result<T, SwitchError> {
    let lock = repo.lock_for_change(command, on_wait)?;
    let mut earlier = Earlier::default();
    let mut finished = false;
    // Kept with the store's warnings, so it is told even if the command then fails.
    if let Some(done) = writeout::finish_interrupted(repo, &lock)? {
        for w in done.warnings {
            if !LISTED.contains(&w.kind) {
                repo.store().note(w);
            }
        }
        earlier = Earlier {
            placeholders: done.placeholders,
            not_written: done.not_written,
        };
        finished = true;
    }
    let result = f(repo, &lock, &mut earlier, finished);
    // What a command that stopped early didn't take into its own warnings.
    for w in earlier.warnings() {
        repo.store().note(w);
    }
    result
}

fn throw_away(
    repo: &mut Repository,
    lock: &ChangeLock,
    paths: Option<&[String]>,
    who: &Person,
    command: &str,
    git_excludes: Option<&Path>,
    earlier: &mut Earlier,
) -> Result<DiscardOutcome, SwitchError> {
    let (head, entry) = repo.current()?;
    let from = tip_of(&entry)?;
    let drive = repo.drive()?;
    // A switch to the save the folder is on that removes new files too.
    let mut plan = writeout::plan(repo, from, from, paths, true, drive, git_excludes)?;
    if !plan.collisions.is_empty() {
        return Err(SwitchError::InTheWay {
            paths: plan.collisions,
        });
    }
    for path in paths.unwrap_or_default().iter().filter(|p| !p.is_empty()) {
        let spellings = writeout::spellings(repo, from, path, drive)?;
        if spellings.len() > 1 {
            return Err(SwitchError::Ambiguous {
                path: path.clone(),
                spellings,
            });
        }
        let top = drive.compared(path);
        let reached = plan
            .actions
            .iter()
            .any(|a| within(&drive.compared(&a.path), &top));
        if !reached && spellings.is_empty() {
            return Err(SwitchError::NoMatch { path: path.clone() });
        }
    }
    let links_left = kept::links_to_leave(repo, &plan.actions, from, drive)?;
    if !links_left.is_empty() {
        let stays = kept::goes_with(links_left.iter().map(String::as_str), drive);
        plan.actions.retain(|a| !stays(&a.path));
        plan.placeholders.retain(|p| !stays(p));
        plan.warnings.push(links_left_warning(&links_left));
    }
    let (mut restored, mut removed, mut folders) = (Vec::new(), Vec::new(), 0);
    for a in &plan.actions {
        // A file or link the folder held at this path is thrown away, whatever replaces it.
        let held_file = a.expected.is_some_and(|e| e.kind != EntryKind::Folder);
        match a.change {
            Change::File(_) | Change::Executable(_) | Change::Link(_) => {
                restored.push(a.path.clone());
            }
            Change::RemoveFile => removed.push(a.path.clone()),
            Change::MakeFolder if held_file => removed.push(a.path.clone()),
            Change::MakeFolder | Change::RemoveFolder => folders += 1,
        }
    }
    if plan.actions.is_empty() {
        return Ok(DiscardOutcome {
            restored,
            removed,
            folders,
            applied: switch_warnings(plan, Applied::default(), earlier),
            safety_copies: 0,
            folder_change: None,
            links_left,
        });
    }
    crate::fsio::act_point("discard:planned");
    let kept_from = kept_on(&entry.state, from, command, paths.unwrap_or_default(), who);
    let mut packs = repo.new_packs()?;
    let (folder_change, safety_copies) =
        kept::switch_saves(repo, &plan.actions, from, &kept_from, drive, &mut packs)?;
    let to_make = Switch {
        head,
        action: Action::Command,
        state: entry.state.clone(),
        folder_change,
        plan,
        drive,
        packs,
    };
    let applied = switch(repo, lock, to_make, who, command, earlier)?;
    // What had changed again since the plan stays as it is.
    let done = |list: Vec<String>| -> Vec<String> {
        list.into_iter()
            .filter(|p| {
                !applied
                    .changed_since_plan
                    .iter()
                    .any(|left| within(p, left))
            })
            .collect()
    };
    Ok(DiscardOutcome {
        restored: done(restored),
        removed: done(removed),
        folders,
        applied,
        safety_copies,
        folder_change,
        links_left,
    })
}

/// Says unsaved links weren't thrown away.
fn links_left_warning(paths: &[String]) -> Warning {
    let list = listed(paths.iter().take(NAMED), paths.len() as u64);
    let message = match paths.len() {
        1 => format!(
            "Didn't throw away 1 unsaved link: an undo could bring it back only as a plain file \
             holding its target, since it may lead out of the project or this computer can't \
             make links (on Windows that takes Developer Mode):{list}\nDelete it yourself if you \
             don't want it."
        ),
        n => format!(
            "Didn't throw away {n} unsaved links: an undo could bring them back only as plain \
             files holding their targets, since they may lead out of the project or this \
             computer can't make links (on Windows that takes Developer Mode):{list}\nDelete them \
             yourself if you don't want them."
        ),
    };
    Warning::new(warning::LINKS_LEFT, message)
}

fn go(
    repo: &mut Repository,
    lock: &ChangeLock,
    target: &Target,
    who: &Person,
    command: &str,
    git_excludes: Option<&Path>,
    earlier: &mut Earlier,
) -> Result<GotoOutcome, SwitchError> {
    let (head, entry) = repo.current()?;
    let from = tip_of(&entry)?;
    // Read again under the lock: another command may have moved the line.
    let (position, to) = match target {
        Target::Line(line) => match entry.state.lines.get(line) {
            Some(save) => (Position::OnLine(line.clone()), *save),
            None => return Err(ResolveError::NotFound(line.clone()).into()),
        },
        Target::ServerLine { server, line, .. } => {
            let servers = repo.server_lines(&entry.state)?;
            match servers
                .servers
                .get(server)
                .and_then(|lines| lines.get(line))
            {
                Some(save) => (Position::AtSave(*save), *save),
                None => return Err(ResolveError::NotFound(format!("{line}@{server}")).into()),
            }
        }
        Target::Save(save) => (Position::AtSave(*save), *save),
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
    if state == entry.state && plan.actions.is_empty() {
        // Already there: nothing to record, and nothing came along.
        return Ok(GotoOutcome {
            position,
            save: to,
            applied: switch_warnings(plan, Applied::default(), earlier),
            safety_copies: 0,
            carried: 0,
        });
    }
    let carried = plan.carried;
    let mut packs = repo.new_packs()?;
    let kept_from = kept_on(&entry.state, from, command, &[], who);
    let (folder_change, safety_copies) =
        kept::switch_saves(repo, &plan.actions, to, &kept_from, drive, &mut packs)?;
    let to_make = Switch {
        head,
        action: Action::Command,
        state,
        folder_change,
        plan,
        drive,
        packs,
    };
    let applied = switch(repo, lock, to_make, who, command, earlier)?;
    Ok(GotoOutcome {
        position,
        save: to,
        applied,
        safety_copies,
        carried,
    })
}

/// What finishing a stopped switch left unwritten or wrote as placeholders, told together with
/// what the command itself does.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Earlier {
    placeholders: Vec<String>,
    not_written: Vec<String>,
}

impl Earlier {
    /// Its warnings, once: what it holds is taken.
    fn warnings(&mut self) -> Vec<Warning> {
        let placeholders = std::mem::take(&mut self.placeholders);
        let not_written = std::mem::take(&mut self.not_written);
        let mut warnings = Vec::new();
        if !not_written.is_empty() {
            warnings.push(writeout::not_written_warning(&not_written));
        }
        if !placeholders.is_empty() {
            warnings.push(writeout::placeholders_warning(&placeholders));
        }
        warnings
    }
}

/// Who makes the kept saves of a switch from `from`, the save the folder is on in `state`, and
/// what they say.
pub(crate) fn kept_on<'a>(
    state: &'a State,
    from: Id,
    command: &str,
    paths: &[String],
    who: &'a Person,
) -> KeptFrom<'a> {
    KeptFrom {
        base: from,
        message: kept::message(command, paths),
        who,
        line: match &state.position {
            Position::OnLine(line) => Some(line.as_str()),
            Position::AtSave(_) => None,
        },
    }
}

/// A switch planned, ready to be made.
pub(crate) struct Switch {
    /// The diary entry `head` names.
    pub head: Id,
    /// What the switch's diary entry records: a command, an undo or a redo.
    pub action: Action,
    /// The state the switch's diary entry records.
    pub state: State,
    pub folder_change: Option<FolderChange>,
    pub plan: Plan,
    pub drive: Drive,
    /// What is stored with its diary entry.
    pub packs: PackSet,
}

/// Makes a planned switch in decision 049's order: its packs and diary entry are stored, then
/// the journal written, the folder changed, `head` moved and the journal removed. A crash at any
/// step leaves what the next changing command finishes, and so do files another program holds,
/// which stop it with `SwitchInUse` after its warnings are noted. Gives what was done.
pub(crate) fn switch(
    repo: &mut Repository,
    lock: &ChangeLock,
    s: Switch,
    who: &Person,
    command: &str,
    earlier: &mut Earlier,
) -> Result<Applied, SwitchError> {
    let entry = DiaryEntry {
        parent: Some(s.head),
        action: s.action,
        command: command.to_owned(),
        who: who.clone(),
        bolt_version: env!("CARGO_PKG_VERSION").into(),
        state: s.state,
        folder_change: s.folder_change,
    };
    let target = repo.store_entry(lock, s.packs, &entry)?;
    let journal = Journal {
        target,
        from: s.head,
        actions: s.plan.actions.clone(),
    };
    journal.write(repo, lock)?;
    let applied = writeout::carry_out(repo, lock, &journal, s.drive)?;
    crate::merge::merge_after_commit(repo, lock);
    let applied = switch_warnings(s.plan, applied, earlier);
    if !applied.in_use.is_empty() {
        let detail = applied
            .warnings
            .iter()
            .find(|w| w.kind == warning::FILES_IN_USE)
            .map(|w| w.message.clone())
            .unwrap_or_default();
        for w in applied.warnings {
            repo.store().note(w);
        }
        return Err(RepoError::SwitchInUse { detail }.into());
    }
    Ok(applied)
}

/// `applied` with the plan's warnings first, then one warning for every entry left unwritten
/// and one for every link written as a file, wherever the list came from: `earlier`, the plan,
/// or carrying it out.
pub(crate) fn switch_warnings(plan: Plan, mut applied: Applied, earlier: &mut Earlier) -> Applied {
    let merged = |mut a: Vec<String>, b: &mut Vec<String>, c: &mut Vec<String>| {
        a.append(b);
        a.append(c);
        a.sort_unstable();
        a.dedup();
        a
    };
    applied.not_written = merged(
        plan.not_written,
        &mut earlier.not_written,
        &mut applied.not_written,
    );
    applied.placeholders = merged(
        plan.placeholders,
        &mut earlier.placeholders,
        &mut applied.placeholders,
    );
    let mut warnings = plan.warnings;
    if !applied.not_written.is_empty() {
        warnings.push(writeout::not_written_warning(&applied.not_written));
    }
    if !applied.placeholders.is_empty() {
        warnings.push(writeout::placeholders_warning(&applied.placeholders));
    }
    let own = applied.warnings.drain(..);
    warnings.extend(own.filter(|w| !LISTED.contains(&w.kind)));
    applied.warnings = warnings;
    applied
}

#[cfg(test)]
mod tests;
