//! `bolt status`: what changed in the project folder since the save the project is on.

use std::path::Path;

use crate::diary::Position;
use crate::diff::{ChangeKind, for_each_change_in};
use crate::history::tip_of;
use crate::id::Id;
use crate::listing::EntryKind;
use crate::repo::Repository;
use crate::snapshot::{SaveError, read_folder};
use crate::warning::Warning;

/// What changed since the current save.
#[derive(Debug)]
pub struct Status {
    pub position: Position,
    /// The save the folder was compared with.
    pub save: Id,
    /// The files that differ from it, in path order.
    pub changes: Vec<PathChange>,
    /// Entries of the save this computer can't hold as they are, by path: saves keep them as
    /// they were.
    pub not_written_here: Vec<String>,
    /// What a stopped switch left to finish, if one did.
    pub unfinished_switch: Option<String>,
    pub warnings: Vec<Warning>,
}

/// A file that differs from the save.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathChange {
    pub path: String,
    pub change: ChangeKind,
    /// The kind now, or before for a removed file.
    pub kind: EntryKind,
    /// The kind before, if the save has a file there.
    pub old_kind: Option<EntryKind>,
    pub old_size: Option<u64>,
    pub new_size: Option<u64>,
}

/// Compares the project folder with the current save, walking it as a save would. Only files the
/// file-state cache doesn't vouch for are read, every file with `full`. Nothing is stored; only
/// the cache is written. `git_excludes` is the user's own Git ignore list, as for a save.
pub fn status(
    repo: &Repository,
    full: bool,
    git_excludes: Option<&Path>,
) -> Result<Status, SaveError> {
    let (_, entry) = repo.current()?;
    let save_id = tip_of(&entry)?;
    let save = repo.store().save(&save_id, repo.signatures_allowed())?;
    let now = read_folder(repo, save.root, full, git_excludes)?;
    let mut changes = Vec::new();
    for_each_change_in(
        repo.store(),
        &now.listings,
        Some(save.root),
        now.root,
        |c| {
            changes.push(PathChange {
                path: c.path.to_owned(),
                change: c.change,
                kind: c.kind,
                old_kind: c.old_kind,
                old_size: c.old_size,
                new_size: c.new_size,
            });
            Ok::<(), SaveError>(())
        },
    )?;
    Ok(Status {
        position: entry.state.position,
        save: save_id,
        changes,
        not_written_here: now.not_written_here,
        unfinished_switch: None,
        warnings: now.warnings,
    })
}
