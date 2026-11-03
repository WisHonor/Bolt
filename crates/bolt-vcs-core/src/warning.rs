//! Warnings: what went wrong without stopping a command. People see each one as
//! `warning: <message>`, and `--json` documents list them (docs/json.md).

use crate::text::escape;

/// A pack holding a copy that failed a check was passed over.
pub const DAMAGED_PACK: &str = "damaged_pack";
/// A stored copy couldn't be read for a reason other than damage, though another copy worked.
pub const COPY_UNREADABLE: &str = "copy_unreadable";
/// A change ID is shown whole because the line couldn't be read to shorten it.
pub const CHANGE_ID_NOT_SHORTENED: &str = "change_id_not_shortened";
/// A save was made but its changed files couldn't be counted.
pub const CHANGED_FILES_NOT_COUNTED: &str = "changed_files_not_counted";
/// A `.gitignore` that isn't a regular file was skipped, so its rules didn't apply.
pub const GITIGNORE_SKIPPED: &str = "gitignore_skipped";
/// Entries this computer doesn't hold as they are were kept as the last save has them.
pub const KEPT_AS_THEY_WERE: &str = "kept_as_they_were";
/// The project opens only from an administrator terminal.
pub const ELEVATED_ONLY: &str = "elevated_only";
/// `start` made the project but couldn't read its first save back to show it.
pub const FIRST_SAVE_NOT_READ: &str = "first_save_not_read";
/// Git checked out a folder named `.bolt` below the top of the project.
pub const NESTED_BOLT_FOLDER: &str = "nested_bolt_folder";
/// The Git repository has a grafts file, which the import ignored.
pub const GRAFTS_IGNORED: &str = "grafts_ignored";

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
