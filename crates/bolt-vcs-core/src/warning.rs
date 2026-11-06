//! Warnings: what went wrong without stopping a command. People see each one as
//! `warning: <message>`, and `--json` documents list them (docs/json.md).

use crate::text::escape;

/// A pack holding a copy that failed a check was passed over.
pub const DAMAGED_PACK: &str = "damaged_pack";
/// A stored copy couldn't be read for a reason other than damage, though another copy worked.
pub const COPY_UNREADABLE: &str = "copy_unreadable";
/// Small packs were merged after a command, but one or more couldn't be: the command happened.
pub const PACK_NOT_MERGED: &str = "pack_not_merged";
/// A change ID is shown whole because the line couldn't be read to shorten it.
pub const CHANGE_ID_NOT_SHORTENED: &str = "change_id_not_shortened";
/// The user's cache key can't be used, so the command went without its caches.
pub const CACHE_KEY_UNUSABLE: &str = "cache_key_unusable";
/// A save was made but its changed files couldn't be counted.
pub const CHANGED_FILES_NOT_COUNTED: &str = "changed_files_not_counted";
/// A `.gitignore` that isn't a regular file was skipped, so its rules didn't apply.
pub const GITIGNORE_SKIPPED: &str = "gitignore_skipped";
/// Entries this computer doesn't hold as they are were kept as the last save has them.
pub const KEPT_AS_THEY_WERE: &str = "kept_as_they_were";
/// A save left out entries whose names Git treats as `.git`, or Bolt as `.bolt`.
pub const GIT_NAMES_LEFT_OUT: &str = "git_names_left_out";
/// A save took changes to files an ignore rule matches, which stay saved because the last save
/// holds them.
pub const IGNORED_CHANGES_SAVED: &str = "ignored_changes_saved";
/// A save took new or changed files that `.gitignore` keeps out but `.boltignore` takes back.
pub const GITIGNORED_FILES_SAVED: &str = "gitignored_files_saved";
/// The project opens only from an administrator terminal.
pub const ELEVATED_ONLY: &str = "elevated_only";
/// `start` made the project but couldn't read its first save back to show it.
pub const FIRST_SAVE_NOT_READ: &str = "first_save_not_read";
/// Git checked out a folder named `.bolt` below the top of the project.
pub const NESTED_BOLT_FOLDER: &str = "nested_bolt_folder";
/// The Git repository has a grafts file, which the import ignored.
pub const GRAFTS_IGNORED: &str = "grafts_ignored";
/// Imported commits whose `change-id` header gives a change ID another commit has too took their
/// own commits' change IDs instead.
pub const DUPLICATE_CHANGE_ID: &str = "duplicate_change_id";
/// The Git repository's config includes other files, which the import didn't read, so it can't
/// tell whether they turn on sparse checkout.
pub const CONFIG_INCLUDES_IGNORED: &str = "config_includes_ignored";
/// Folders that hold a repository or project of their own, which saves take nothing new from.
pub const NESTED_PROJECTS_LEFT_OUT: &str = "nested_projects_left_out";
/// The file-state cache couldn't be written, so the next status reads every file again.
pub const FILE_CACHE_NOT_STORED: &str = "file_cache_not_stored";
/// Entries of the save being written that this computer can't hold were left unwritten.
pub const NOT_WRITTEN_HERE: &str = "not_written_here";
/// Links were written as files holding their targets.
pub const PLACEHOLDERS_WRITTEN: &str = "placeholders_written";
/// Files another program held open were left for the next run.
pub const FILES_IN_USE: &str = "files_in_use";
/// Entries that changed after a switch was planned were left as they were.
pub const CHANGED_SINCE_PLAN: &str = "changed_since_plan";
/// A switch between saves was stopped, and is still to finish.
pub const SWITCH_UNFINISHED: &str = "switch_unfinished";
/// A command finished a switch between saves that was stopped, before doing its own work.
pub const SWITCH_FINISHED: &str = "switch_finished";
/// A file's diff ran out of time, so its changes show in bigger pieces than they could.
pub const DIFF_TIMED_OUT: &str = "diff_timed_out";

/// Whether people see a warning of `kind` when the command then fails: it tells of files
/// already written, by a switch the command finished first or by the command itself before it
/// stopped, which happened all the same. Others are only in `--json`.
#[must_use]
pub fn told_on_failure(kind: &str) -> bool {
    [
        SWITCH_FINISHED,
        CHANGED_SINCE_PLAN,
        PLACEHOLDERS_WRITTEN,
        NOT_WRITTEN_HERE,
    ]
    .contains(&kind)
}

/// Something that went wrong without stopping a command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Warning {
    /// A stable code in snake_case, for programs.
    pub kind: &'static str,
    /// What happened, for people. Escaped, so it is safe to print.
    pub message: String,
}

impl Warning {
    /// Escapes `message` once more, keeping its newlines, so a piece nobody thought of escaping
    /// can't drive the terminal. Pieces from outside Bolt should already be escaped without
    /// theirs.
    #[must_use]
    pub fn new(kind: &'static str, message: impl AsRef<str>) -> Warning {
        Warning {
            kind,
            message: escape(message.as_ref(), true),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_message_is_escaped_but_keeps_the_lines_bolt_wrote() {
        let warning = Warning::new(DAMAGED_PACK, "pack a\u{1b}[2Jb is damaged.\nSecond line.");
        assert_eq!(warning.kind, "damaged_pack");
        assert_eq!(warning.message, "pack a\\x1b[2Jb is damaged.\nSecond line.");
        // Escaping what is escaped changes nothing.
        assert_eq!(Warning::new(warning.kind, &warning.message), warning);
    }
}
