//! `opalro status`: what changed in the project folder since the save the project is on.

use std::path::Path;

use crate::diary::Position;
use crate::diff::{ChangeKind, FileChange, for_each_change_in};
use crate::history::tip_of;
use crate::id::Id;
use crate::listing::EntryKind;
use crate::repo::Repository;
use crate::save::Save;
use crate::snapshot::{SaveError, kept_in_the_way, read_folder};
use crate::text::escape;
use crate::warning::Warning;
use crate::writeout;

/// What changed since the current save.
#[derive(Debug)]
pub struct Status {
    pub position: Position,
    /// The save the folder was compared with, and its ID.
    pub save_id: Id,
    pub save: Save,
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
    /// What each side holds, if it has a file.
    pub old_id: Option<Id>,
    pub new_id: Option<Id>,
}

impl From<FileChange<'_>> for PathChange {
    fn from(c: FileChange<'_>) -> PathChange {
        PathChange {
            path: c.path.to_owned(),
            change: c.change,
            kind: c.kind,
            old_kind: c.old_kind,
            old_size: c.old_size,
            new_size: c.new_size,
            old_id: c.old_id,
            new_id: c.new_id,
        }
    }
}

/// Why `status` stopped, in its own words where the save's would talk of saving: a name a save
/// can't hold, something in the way of an entry kept as it was, or a project past a save's limits.
#[must_use]
pub fn why_stopped(e: &SaveError) -> String {
    let why = match e {
        SaveError::InvalidName { path, reason } => format!("{}: {reason}", escape(path, false)),
        SaveError::InTheWay { path, kept } => {
            format!("{}: {}", escape(path, false), kept_in_the_way(kept))
        }
        SaveError::TooLarge { detail } => format!("this project is too large for Opalro: {detail}"),
        other => return other.to_string(),
    };
    format!("Opalro can't show the status: {why}.")
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
    let now = read_folder(repo, save.root, full, git_excludes, None)
        .map_err(|e| e.in_save(save_id, true))?;
    let mut warnings = now.warnings;
    let unfinished = writeout::unfinished_warning(repo);
    let unfinished_switch = unfinished.as_ref().map(|w| w.message.clone());
    warnings.extend(unfinished);
    let mut changes = Vec::new();
    for_each_change_in(
        repo.store(),
        &now.listings,
        Some(save.root),
        now.root,
        |c| {
            changes.push(PathChange::from(c));
            Ok::<(), SaveError>(())
        },
    )
    .map_err(|e| e.in_save(save_id, true))?;
    Ok(Status {
        position: entry.state.position,
        save_id,
        save,
        changes,
        not_written_here: now.not_written_here,
        unfinished_switch,
        warnings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_says_why_it_stops_without_talk_of_saving() {
        let bad_name = SaveError::InvalidName {
            path: "a/b\u{fffd}".into(),
            reason: "its name is not valid UTF-8".into(),
        };
        assert_eq!(
            why_stopped(&bad_name),
            "Opalro can't show the status: a/b\u{fffd}: its name is not valid UTF-8."
        );
        let too_large = SaveError::TooLarge {
            detail: "its folders nest more than 4096 levels deep".into(),
        };
        assert_eq!(
            why_stopped(&too_large),
            "Opalro can't show the status: this project is too large for Opalro: its folders nest \
             more than 4096 levels deep."
        );
        // What says nothing of saving stays as it is.
        let changing = SaveError::ChangedDuringSave {
            path: "a.txt".into(),
        };
        assert_eq!(why_stopped(&changing), changing.to_string());
    }
}
