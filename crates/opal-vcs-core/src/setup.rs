//! `opalro setup`'s step: a preset's `opalro.toml` and `.opalroignore` written as one step that undo
//! takes back (docs/storage-format.md section 9).

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::io;
use std::path::Path;

use opal_platform::dir::{Dir, EntryType};

use crate::diary::Action;
use crate::history::tip_of;
use crate::id::Id;
use crate::ingest::{self, IngestError};
use crate::kept::{self, Put};
use crate::listing::EntryKind;
use crate::pack::PackSet;
use crate::presets::{self, Found, Looked, Preset};
use crate::repo::{ChangeLock, RepoError, Repository};
use crate::rules::MAX_RULES_FILE_BYTES;
use crate::safe_fs;
use crate::save::Person;
use crate::snapshot::{NAMED, SaveError, listed};
use crate::store::Store;
use crate::switch::{self, Earlier, Switch, SwitchError};
use crate::text::escape;
use crate::warning::Warning;
use crate::writeout;

/// The only paths the step ever writes, at the project's top.
const RULE_FILES: [&str; 2] = ["opalro.toml", ".opalroignore"];

/// What the step would do to one of the two rule files: `None` is no file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuleFileChange {
    /// `opalro.toml` or `.opalroignore`.
    pub path: &'static str,
    pub before: Option<Vec<u8>>,
    pub after: Option<Vec<u8>>,
}

/// What switching to a preset would do.
#[derive(Clone, Debug)]
pub struct SetupPlan {
    pub preset: Preset,
    pub found: Found,
    /// Only the files that would change.
    pub changes: Vec<RuleFileChange>,
}

/// What the step did.
#[derive(Debug)]
pub struct SetupOutcome {
    /// The rule files written or removed.
    pub written: Vec<&'static str>,
    pub warnings: Vec<Warning>,
}

/// Why the step changed nothing.
#[derive(Debug, thiserror::Error)]
pub enum SetupError {
    #[error(
        "{} isn't a regular file (it is a link, a folder or a special file), so nothing was \
         changed.",
        escape(path, false)
    )]
    NotAFile { path: &'static str },
    #[error(
        "{} is larger than 1 MiB, so nothing was changed.",
        escape(path, false)
    )]
    TooLarge { path: &'static str },
    #[error(
        "{} changed after Opalro worked out what to write, so nothing was changed.",
        escape(path, false)
    )]
    ChangedSincePlan { path: &'static str },
    #[error(
        "Something Opalro won't write over is where the rule files would go (a file spelled in \
         other letter case, say), so nothing was changed:{}",
        listed(paths.iter().take(NAMED), paths.len() as u64)
    )]
    InTheWay { paths: Vec<String> },
    #[error(transparent)]
    Switch(#[from] SwitchError),
    #[error(transparent)]
    Save(#[from] SaveError),
    #[error(transparent)]
    Repo(#[from] RepoError),
}

impl From<crate::store::StoreError> for SetupError {
    fn from(e: crate::store::StoreError) -> Self {
        SetupError::Repo(e.into())
    }
}

impl SetupError {
    /// What the user can do next, if there is something to say.
    #[must_use]
    pub fn hint(&self) -> Option<String> {
        match self {
            SetupError::NotAFile { .. } => Some(
                "Opalro reads and writes it only as a regular file: move it somewhere else, then \
                 run opalro setup again."
                    .to_owned(),
            ),
            SetupError::TooLarge { .. } => Some(
                "Make it smaller or move it somewhere else, then run opalro setup again."
                    .to_owned(),
            ),
            SetupError::ChangedSincePlan { .. } => {
                Some("Run opalro setup again to see what it would change now.".to_owned())
            }
            SetupError::InTheWay { .. } => {
                Some("Rename or move them out of the way, then run opalro setup again.".to_owned())
            }
            SetupError::Switch(e) => e.hint(),
            SetupError::Save(e) => e.hint_when_reading(),
            SetupError::Repo(e) => e.hint(),
        }
    }
}

/// A look at the folder `root` before a start, and whether anything is named `opalro.toml` there: a
/// file, a link or a folder, in any letter case the drive takes for that name.
pub fn look_at(root: &Path) -> io::Result<(Looked, bool)> {
    let top = Dir::open_top(root)?;
    let looked = presets::look(&top)?;
    let has_rules_file = match top.metadata(OsStr::new(RULE_FILES[0])) {
        Ok(_) => true,
        Err(e) if e.kind() == io::ErrorKind::NotFound => looked.has_rules_file,
        Err(e) => return Err(e),
    };
    Ok((looked, has_rules_file))
}

/// How many lines the preset's block of the project's `.opalroignore` holds; `None` when there is
/// no file or no block.
pub fn opalroignore_block_lines(root: &Path) -> Result<Option<usize>, SetupError> {
    let top = Dir::open_top(root).map_err(RepoError::from)?;
    Ok(read_rule_file(&top, RULE_FILES[1])?.and_then(|bytes| presets::block_lines(&bytes)))
}

/// What switching to `preset` would do to the two rule files of the project at `root`. Changes
/// nothing.
pub fn plan_setup(root: &Path, preset: Preset) -> Result<SetupPlan, SetupError> {
    let top = Dir::open_top(root).map_err(RepoError::from)?;
    let found = presets::look(&top).map_err(RepoError::from)?.found;
    let toml = read_rule_file(&top, RULE_FILES[0])?;
    let ignore = read_rule_file(&top, RULE_FILES[1])?;
    let new_toml = presets::opal_toml(preset, &found).map(String::into_bytes);
    let block = presets::opalroignore_block(preset, &found);
    let new_ignore = presets::edit_opalroignore(ignore.as_deref(), block.as_deref());
    let changes = [
        (RULE_FILES[0], toml, new_toml),
        (RULE_FILES[1], ignore, new_ignore),
    ]
    .into_iter()
    .filter(|(_, before, after)| before != after)
    .map(|(path, before, after)| RuleFileChange {
        path,
        before,
        after,
    })
    .collect();
    Ok(SetupPlan {
        preset,
        found,
        changes,
    })
}

/// Writes the plan as one `setup` step, which `opalro undo` takes back. Refuses if either file
/// changed since the plan was made. A plan with no changes writes and records nothing.
pub fn apply_setup(
    repo: &mut Repository,
    plan: &SetupPlan,
    who: &Person,
    git_excludes: Option<&Path>,
    on_wait: &mut dyn FnMut(&str),
) -> Result<SetupOutcome, SetupError> {
    if plan.changes.is_empty() {
        return Ok(nothing());
    }
    switch::locked(repo, "setup", on_wait, |repo, lock, earlier, _| {
        Ok(write_step(repo, lock, earlier, plan, who, git_excludes))
    })?
}

fn nothing() -> SetupOutcome {
    SetupOutcome {
        written: Vec::new(),
        warnings: Vec::new(),
    }
}

/// The step, under the lock: from the save the folder is on with the two files as the folder
/// holds them, to the same with the plan's.
fn write_step(
    repo: &mut Repository,
    lock: &ChangeLock,
    earlier: &mut Earlier,
    plan: &SetupPlan,
    who: &Person,
    git_excludes: Option<&Path>,
) -> Result<SetupOutcome, SetupError> {
    let (head, entry) = repo.current()?;
    let current = tip_of(&entry)?;
    let top = Dir::open_top(repo.root()).map_err(RepoError::from)?;
    let mut packs = repo.new_packs()?;
    // Both names, so a link or a folder that appeared at the one left alone refuses too.
    for path in RULE_FILES {
        let now = read_rule_file(&top, path)?;
        // What a person confirmed must be what is applied.
        if let Some(change) = plan.changes.iter().find(|c| c.path == path)
            && now != change.before
        {
            return Err(SetupError::ChangedSincePlan { path });
        }
    }
    let (mut before, mut after) = (BTreeMap::new(), BTreeMap::new());
    for change in plan.changes.iter().filter(|c| RULE_FILES.contains(&c.path)) {
        // An executable file stays one, so the folder matches the "from" save.
        let kind = match top.metadata(OsStr::new(change.path)) {
            Ok(m) if m.executable == Some(true) => EntryKind::Executable,
            _ => EntryKind::File,
        };
        let path = change.path.to_owned();
        before.insert(
            path.clone(),
            stored(&mut packs, repo.store(), kind, &change.before)?,
        );
        after.insert(path, stored(&mut packs, repo.store(), kind, &change.after)?);
    }
    let paths: Vec<String> = before.keys().cloned().collect();
    let kept = switch::kept_on(&entry.state, current, "setup", &paths, who);
    let base_root = repo.store().save(&current, repo.signatures_allowed())?.root;
    let mut side = |changes: &BTreeMap<String, Option<Put>>| -> Result<Id, SaveError> {
        let root = kept::overlay(repo.store(), base_root, changes, &mut packs)?;
        if root == base_root {
            return Ok(current);
        }
        kept::kept_save(root, &kept, &mut packs)
    };
    let (from, to) = (side(&before)?, side(&after)?);
    if from == to {
        return Ok(nothing());
    }
    // The plan reads both saves from the store.
    repo.store_packs(lock, packs)?;
    let drive = repo.drive()?;
    let step = writeout::plan(repo, from, to, Some(&paths), false, drive, git_excludes)?;
    if !step.collisions.is_empty() {
        return Err(SetupError::InTheWay {
            paths: step.collisions,
        });
    }
    let mut packs = repo.new_packs()?;
    let (folder_change, _) = kept::switch_saves(repo, &step, from, to, &kept, drive, &mut packs)?;
    // Each file changed again while it was being kept.
    if folder_change.is_none() {
        let path = plan.changes.first().map_or(RULE_FILES[0], |c| c.path);
        return Err(SetupError::ChangedSincePlan { path });
    }
    crate::fsio::act_point("setup:planned");
    let to_make = Switch {
        head,
        action: Action::Command,
        state: entry.state.clone(),
        folder_change,
        plan: step,
        drive,
        packs,
    };
    let applied = switch::switch(repo, lock, to_make, who, "setup", earlier)?;
    let written = plan
        .changes
        .iter()
        .map(|c| c.path)
        .filter(|p| RULE_FILES.contains(p) && !applied.changed_since_plan.iter().any(|l| l == p))
        .collect();
    Ok(SetupOutcome {
        written,
        warnings: applied.warnings,
    })
}

/// `bytes` stored as a file of `kind`, or nothing.
fn stored(
    packs: &mut PackSet,
    store: &Store,
    kind: EntryKind,
    bytes: &Option<Vec<u8>>,
) -> Result<Option<Put>, SaveError> {
    let Some(bytes) = bytes else {
        return Ok(None);
    };
    let id = ingest::store_held(packs, store, bytes).map_err(|e| match e {
        IngestError::Pack(e) => SaveError::from(e),
        IngestError::Store(e) => SaveError::from(e),
        IngestError::Read(e) => SaveError::from(RepoError::from(e)),
        IngestError::Changed => SaveError::from(RepoError::from(io::Error::other(
            "the bytes changed while they were stored",
        ))),
    })?;
    Ok(Some((kind, id, bytes.len() as u64)))
}

/// The rule file `path` at the top, read through the folder's handle and never through a link;
/// `None` when nothing has that name.
fn read_rule_file(top: &Dir, path: &'static str) -> Result<Option<Vec<u8>>, SetupError> {
    let name = OsStr::new(path);
    let e = match safe_fs::read_capped_in(top, name, Path::new(path), MAX_RULES_FILE_BYTES) {
        Ok(bytes) => return Ok(Some(bytes)),
        Err(e) => e,
    };
    // Judged by what is there: a link to nothing is there all the same.
    match top.metadata(name) {
        Err(m) if m.kind() == io::ErrorKind::NotFound && e.kind() == io::ErrorKind::NotFound => {
            Ok(None)
        }
        Ok(m) if m.kind != EntryType::File => Err(SetupError::NotAFile { path }),
        Ok(m) if m.size > MAX_RULES_FILE_BYTES => Err(SetupError::TooLarge { path }),
        _ => Err(SaveError::Unreadable {
            path: path.to_owned(),
            source: e,
        }
        .into()),
    }
}

#[cfg(test)]
mod tests;
