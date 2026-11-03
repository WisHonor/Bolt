//! Importing a Git repository into Bolt (milestone 2): why an import stops, and, in `convert`,
//! how Git's objects become Bolt's.
#![deny(clippy::indexing_slicing, clippy::arithmetic_side_effects)]

use std::fmt::Write as _;

use crate::git::GitError;
use crate::pack::PackError;
use crate::repo::RepoError;
use crate::store::StoreError;
use crate::text::escape;

mod convert;

pub use convert::{Converter, Counts, TagOutcome};

/// Why an import stops. Text from the repository is shown escaped.
#[derive(Debug, thiserror::Error)]
pub enum ImportError {
    #[error(transparent)]
    Git(#[from] GitError),
    #[error(transparent)]
    Repo(#[from] RepoError),
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error(transparent)]
    Pack(#[from] PackError),
    /// The import can't go on; `what` says why, naming the commit or tag.
    #[error("{}", escape(what, false))]
    Stop { what: String, hint: Option<String> },
    /// Names that aren't valid UTF-8 (decision 019): the first ones as the commit (hex) and the
    /// path, with U+FFFD for bytes that aren't UTF-8, and how many there are. Paths here, as in
    /// `Counts`, are kept as they are and escaped only where they are shown.
    #[error("{}", not_utf8_message(first, *count))]
    NotUtf8Names {
        first: Vec<(String, String)>,
        count: u64,
    },
    /// A Git LFS pointer (decision 037), in the commit `commit` (hex) at `path`.
    #[error(
        "Commit `{}` holds a Git LFS pointer at `{}`, and this version of Bolt can't import Git \
         LFS files yet: Git LFS repositories come in milestone 5.",
        escape(commit, false),
        escape(path, false)
    )]
    LfsPointer { commit: String, path: String },
}

impl ImportError {
    /// What the user can do next, if there is something to say.
    #[must_use]
    pub fn hint(&self) -> Option<String> {
        match self {
            ImportError::Git(e) => e.hint(),
            ImportError::Repo(e) => e.hint(),
            ImportError::Store(_) | ImportError::Pack(_) => None,
            ImportError::Stop { hint, .. } => hint.clone(),
            ImportError::NotUtf8Names { .. } => Some(
                "Nothing was changed. Rename them in the Git history to names in UTF-8, then run \
                 bolt import again."
                    .to_owned(),
            ),
            ImportError::LfsPointer { .. } => Some(
                "Nothing was changed. To import now, turn the Git LFS files back into ordinary \
                 Git files in a copy of the repository with `git lfs migrate export --everything \
                 --include=\"*\"`, which rewrites its history, then run bolt import in that copy."
                    .to_owned(),
            ),
        }
    }
}

fn not_utf8_message(first: &[(String, String)], count: u64) -> String {
    let mut out = if count == 1 {
        "A name in the Git history isn't valid UTF-8, and Bolt keeps only UTF-8 names:".to_owned()
    } else {
        format!(
            "{count} names in the Git history aren't valid UTF-8, and Bolt keeps only UTF-8 names:"
        )
    };
    for (commit, path) in first {
        let (path, commit) = (escape(path, false), escape(commit, false));
        let _ = write!(out, "\n  `{path}` in commit `{commit}`");
    }
    let more = count.saturating_sub(first.len() as u64);
    if more > 0 {
        let _ = write!(out, "\n  and {more} more");
    }
    out
}
