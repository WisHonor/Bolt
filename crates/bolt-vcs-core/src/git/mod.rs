//! Bolt's own read-only Git reader (decision 031). `Repo::open` checks a repository's format and
//! refuses what Bolt can't import; `Repo::refs` reads its refs, `Repo::read` its objects, and
//! `parse_commit`, `parse_tree` and `parse_tag` what those hold. Everything under `.git` is read
//! through `safe_fs`, bounded, and treated as possibly hostile.
#![deny(clippy::indexing_slicing, clippy::arithmetic_side_effects)]

mod config;
#[cfg(any(test, feature = "test-support"))]
pub mod craft;
mod id;
mod lfs;
mod object;
mod odb;
mod pack;
mod person;
mod refs;

pub use id::{GitId, HashKind, ObjectType, object_id};
pub use lfs::{LfsPointer, MAX_LFS_POINTER_BYTES, lfs_pointer};
use object::split_at_byte;
pub use object::{
    Commit, TagObject, TreeEntry, is_exact_lfs_pointer, is_lfs_lookalike, looks_like_lfs_pointer,
    parse_commit, parse_tag, parse_tree, rebuild_commit,
};
#[cfg(any(test, feature = "test-support"))]
pub use odb::set_test_delta_limits;
pub use odb::{BlobReader, DELTA_BASE_CACHE, MAX_DELTA_DEPTH, MAX_IN_MEMORY_OBJECT, Object};
pub use person::parse_person;
// For the delta fuzz target.
#[cfg(feature = "test-support")]
pub use pack::apply_delta;
pub use refs::{Head, Refs};

pub(crate) use config::{
    UserSetting, read_bool, read_count, read_user_settings, read_user_variable,
};

use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf};

use bolt_platform::Owner;

use crate::config::ConfigError;
use crate::names::MAX_NEW_NAME_BYTES;
use crate::repo::{self, RepoError};
use crate::safe_fs;
use crate::text::escape;

/// `.git/config` and `.git/config.worktree`, each.
const MAX_CONFIG_BYTES: u64 = 1 << 20;

/// A Git feature Bolt can't import, found before anything is written. Each message says what it
/// is, and `hint` what to run to import anyway.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Refusal {
    #[error("This Git repository uses format version {0}, and Bolt reads only versions 0 and 1.")]
    FormatVersion(u64),
    #[error(
        "This Git repository uses extensions.{}, a Git feature Bolt doesn't know.",
        escape(.0, false)
    )]
    UnknownExtension(String),
    #[error(
        "This Git repository keeps its refs in the reftable format, which Bolt can't read yet."
    )]
    Reftable,
    #[error(
        "This Git repository keeps its refs at a path its config names (extensions.refStorage), \
         which Bolt doesn't follow."
    )]
    RefsElsewhere,
    #[error(
        "This Git repository is a partial clone: some of its history was never downloaded, and \
         Bolt needs all of it."
    )]
    PartialClone,
    #[error(
        "This Git repository is a shallow clone: its older history was never downloaded, and Bolt \
         needs all of it."
    )]
    Shallow,
    #[error(
        "This Git repository borrows objects from another one (.git/objects/info/alternates), \
         which Bolt doesn't follow."
    )]
    Alternates,
    #[error(
        "This .git points to a repository kept elsewhere (a linked worktree or a submodule), \
         which Bolt doesn't follow."
    )]
    GitFile,
    #[error(
        "Git leaves some files out of this folder on purpose (a sparse checkout), so Bolt would \
         record them as deleted."
    )]
    SparseCheckout,
    #[error(
        "This Git repository has a pack index in Git's old version 1 format, which Bolt doesn't \
         read."
    )]
    OldPackIndex,
}

impl Refusal {
    #[must_use]
    pub fn hint(&self) -> String {
        match self {
            Refusal::FormatVersion(_) => {
                "Git reads only those versions too, so check core.repositoryformatversion in \
                 .git/config."
            }
            Refusal::UnknownExtension(_) => {
                "Import a fresh clone made with `git clone`, which usually doesn't use it, or \
                 update Bolt."
            }
            Refusal::Reftable => {
                "Convert it with `git refs migrate --ref-storage-format=files`, then run bolt \
                 import again."
            }
            Refusal::RefsElsewhere => {
                "Import a fresh clone made with `git clone`, which keeps its refs in its .git \
                 folder."
            }
            Refusal::PartialClone => {
                "Clone the repository again without `--filter`, and run bolt import in the new \
                 clone."
            }
            Refusal::Shallow => {
                "Fetch the rest with `git fetch --unshallow`, then run bolt import again."
            }
            Refusal::Alternates => {
                "Copy those objects in with `git repack -a -d`, delete \
                 .git/objects/info/alternates, then run bolt import again."
            }
            Refusal::GitFile => {
                "Run bolt import where the repository's own .git folder is, or in a fresh clone \
                 of it."
            }
            Refusal::SparseCheckout => {
                "Turn it off with `git sparse-checkout disable`, then run bolt import again."
            }
            Refusal::OldPackIndex => {
                "Rewrite the packs with `git repack -a -d`, then run bolt import again."
            }
        }
        .to_owned()
    }
}

/// Why Bolt can't import a Git repository. Paths, ref names and config values come from the
/// repository, so messages show them escaped.
#[derive(Debug, thiserror::Error)]
pub enum GitError {
    #[error(
        "No Git repository here: run bolt import at the top of a Git project, where its .git \
         folder is."
    )]
    NotAGitFolder,
    #[error(transparent)]
    Refused(Refusal),
    /// The `.git` folder `git` isn't the user's: another account owns it, Bolt can't tell who
    /// does, or the safe-folders that would allow it can't be read. `refusal` is the one a Bolt
    /// project at the work tree gets (decisions 025 to 027 and 030), so its hint trusts the work
    /// tree, which is what safe-folders lists.
    #[error("{}", owner_message(git, refusal))]
    NotYours { git: PathBuf, refusal: RepoError },
    /// Damaged or hostile data; `what` names the file or object.
    #[error("Bolt can't import this Git repository: {}.", escape(what, false))]
    Broken { what: String },
    /// Something other than the plain file or real folder Git keeps there: a link, which `git
    /// fsck` follows without a word, a special file, a folder where a file belongs or a file where
    /// a folder does. Bolt reads none of them; `what` names it.
    #[error("Bolt can't import this Git repository: {}.", escape(what, false))]
    NotPlain { what: String },
    /// An object whose compressed data ended before the size it gives. Git doesn't write one, and
    /// `git fsck` doesn't always notice one, so it may be damage, a hostile object or a fault in
    /// Bolt's reader; `what` names it.
    #[error("Bolt can't import this Git repository: {}.", escape(what, false))]
    EndedEarly { what: String },
    /// An object Git reads, but past a limit or a rule of Bolt's that Git doesn't have, so
    /// `git fsck` finds nothing wrong; `what` names the object and says why.
    #[error(
        "Git reads this object, but Bolt can't import it as it is: {}.",
        escape(what, false)
    )]
    Unimportable { what: String },
    /// More refs, or longer names, than Bolt reads; `what` names the limit.
    #[error("Bolt can't import this Git repository: {}.", escape(what, false))]
    TooManyRefs { what: String },
    /// More objects in its packs than Bolt keeps the indexes of; `what` names the limit.
    #[error("Bolt can't import this Git repository: {}.", escape(what, false))]
    TooManyObjects { what: String },
    /// More packs than Bolt reads; `what` names the limit.
    #[error("Bolt can't import this Git repository: {}.", escape(what, false))]
    TooManyPacks { what: String },
    /// More work rebuilding objects through deltas than the import's budget; `what` says how
    /// much.
    #[error("Bolt can't import this Git repository: {}.", escape(what, false))]
    TooMuchWork { what: String },
    /// What only a bug in Bolt's reader causes; `what` says what went wrong.
    #[error(
        "Bolt can't import this Git repository: {}; please report this.",
        escape(what, false)
    )]
    Bug { what: String },
    #[error(
        "Git changed the repository while Bolt read it: {}.",
        escape(what, false)
    )]
    Changed { what: String },
    /// Too little free memory to read what `path` holds.
    #[error(
        "Bolt ran out of memory while reading {}.",
        escape(&path.display().to_string(), false)
    )]
    OutOfMemory { path: PathBuf },
    #[error(
        "Bolt couldn't read {}: {}",
        escape(&path.display().to_string(), false),
        escape(&source.to_string(), false)
    )]
    Io { path: PathBuf, source: io::Error },
}

impl GitError {
    /// What the user can do next, if there is something to say.
    #[must_use]
    pub fn hint(&self) -> Option<String> {
        Some(
            match self {
                GitError::NotAGitFolder => return None,
                GitError::Refused(refusal) => return Some(refusal.hint()),
                GitError::NotYours { refusal, .. } => return refusal.hint(),
                GitError::Broken { .. } => {
                    "Nothing was changed. Check the repository with `git fsck`, or import a \
                     fresh clone of it."
                }
                GitError::EndedEarly { .. } => {
                    "Nothing was changed. Git doesn't write objects that end early, so either the \
                     repository was damaged or made this way, which `git fsck` may not notice, or \
                     this is a fault in Bolt's reader. Import a fresh clone of it, and if that \
                     stops here too, please report it, with this message."
                }
                GitError::NotPlain { .. } => {
                    "Nothing was changed. Bolt reads only plain files and real folders in a Git \
                     repository, and never follows links there: put what a link points to in its \
                     place, or import a fresh clone."
                }
                GitError::Unimportable { .. } => {
                    "Nothing was changed. Rewrite the Git history that holds it, then run bolt \
                     import again."
                }
                GitError::TooManyRefs { .. } => {
                    "Nothing was changed. Delete refs you don't need, such as stale server \
                     branches (`git remote prune <server>`) or old tags (`git tag -d <tag>`), \
                     then run bolt import again."
                }
                GitError::TooManyObjects { .. } => {
                    "Nothing was changed. If its packs hold copies of the same objects, or objects \
                     nothing uses, merge them with `git repack -a -d`, then run bolt import again."
                }
                GitError::TooManyPacks { .. } => {
                    "Nothing was changed. Merge them into one with `git repack -a -d`, then run \
                     bolt import again."
                }
                GitError::TooMuchWork { .. } => {
                    "Nothing was changed. If you trust this repository, have Git store its files \
                     over 1 MiB whole and make its other deltas anew, with `git -c \
                     core.bigFileThreshold=1m repack -a -d -f`, then run bolt import again. Git's \
                     packs stay bigger until its next `git gc`."
                }
                GitError::Bug { .. } => {
                    "Nothing was changed. This is a bug in Bolt, not a problem with the \
                     repository."
                }
                GitError::Changed { .. } => {
                    "Nothing was changed. Run bolt import again once Git has finished."
                }
                GitError::OutOfMemory { .. } => {
                    "Nothing was changed. Close other programs to free memory, then run bolt \
                     import again."
                }
                GitError::Io { .. } => {
                    "Nothing was changed. Check that your account may read it and that no other \
                     program has it locked, then run bolt import again."
                }
            }
            .to_owned(),
        )
    }
}

/// A `NotYours` message: it names the `.git` folder, whose owner was asked.
fn owner_message(path: &Path, refusal: &RepoError) -> String {
    let git = escape(&path.display().to_string(), false);
    match refusal {
        RepoError::NotYours { .. } => {
            format!("The Git repository at {git} belongs to another user account.")
        }
        RepoError::AdminsNotElevated { .. } => format!(
            "The Git repository at {git} belongs to the Administrators group, and this terminal \
             isn't running as administrator."
        ),
        RepoError::OwnerUnknown { .. } => format!(
            "Bolt can't tell who owns the Git repository at {git}: this drive doesn't record who \
             owns folders."
        ),
        RepoError::OwnerLookupFailed { source, .. } => format!(
            "Bolt couldn't find out who owns the Git repository at {git}: {}",
            escape(&source.to_string(), false)
        ),
        RepoError::SafeFolderPassedOver {
            entry,
            why,
            refusal,
        } => {
            format!(
                "{} {}",
                owner_message(path, refusal),
                repo::passed_over(entry, why)
            )
        }
        // The user's settings, which hold safe-folders, can't be read.
        other => other.to_string(),
    }
}

fn broken(what: impl Into<String>) -> GitError {
    GitError::Broken { what: what.into() }
}

fn unimportable(what: impl Into<String>) -> GitError {
    GitError::Unimportable { what: what.into() }
}

/// Bytes from a repository for a message, cut to the 255 bytes of a name every system holds, so a
/// hostile one can't flood it. Git allows nearly any bytes in names; the error escapes them when
/// it is shown.
pub(crate) fn shown(bytes: &[u8]) -> String {
    match bytes.get(..MAX_NEW_NAME_BYTES) {
        Some(start) if start.len() < bytes.len() => {
            format!("{}...", String::from_utf8_lossy(start))
        }
        _ => String::from_utf8_lossy(bytes).into_owned(),
    }
}

/// Whitespace as Git reads it in refs and person lines: space, tab, LF and CR, but not vertical
/// tab or form feed.
fn is_git_space(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\n' | b'\r')
}

/// `text` without Git's whitespace at its end.
fn trim_end(mut text: &[u8]) -> &[u8] {
    while let Some((&last, rest)) = text.split_last()
        && is_git_space(last)
    {
        text = rest;
    }
    text
}

/// `text` without Git's whitespace at its start.
fn trim_start(mut text: &[u8]) -> &[u8] {
    while let Some((&first, rest)) = text.split_first()
        && is_git_space(first)
    {
        text = rest;
    }
    text
}

/// `safe_fs`'s refusal of anything but the plain file or real folder Git keeps there (a link, a
/// special file, a folder for a file or a file for a folder) is `NotPlain`, and of a file past its
/// cap `Broken`; a file gone while Bolt read is `Changed`, running out of memory `OutOfMemory`,
/// anything else `Io`. Each names `path`.
fn read_error(path: &Path, e: io::Error) -> GitError {
    match e.kind() {
        io::ErrorKind::InvalidInput => GitError::NotPlain {
            what: naming(path, &e),
        },
        io::ErrorKind::InvalidData => broken(naming(path, &e)),
        io::ErrorKind::NotFound => GitError::Changed {
            what: format!("{} went away", path.display()),
        },
        io::ErrorKind::OutOfMemory => GitError::OutOfMemory {
            path: path.to_owned(),
        },
        _ => GitError::Io {
            path: path.to_owned(),
            source: e,
        },
    }
}

/// Running out of memory while reading `path`.
fn out_of_memory(path: &Path) -> GitError {
    read_error(path, io::ErrorKind::OutOfMemory.into())
}

/// A file that ended before the length Bolt measured when it opened it.
fn shrank(path: &Path) -> GitError {
    GitError::Changed {
        what: format!("{} got shorter", path.display()),
    }
}

/// `e`'s message, naming `path`: safe_fs's own messages name it already, the system's don't.
fn naming(path: &Path, e: &io::Error) -> String {
    let (shown, message) = (path.display().to_string(), e.to_string());
    if message.contains(&shown) {
        message
    } else {
        format!("{shown}: {message}")
    }
}

/// The regular file at `path`, at most `max` bytes, or `None` if there is none.
fn read_optional(path: &Path, max: u64) -> Result<Option<Vec<u8>>, GitError> {
    match safe_fs::read_capped(path, max) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(read_error(path, e)),
    }
}

/// Whether anything is at `path`, not following a link there.
fn exists(path: &Path) -> Result<bool, GitError> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(read_error(path, e)),
    }
}

/// Whether `path` is a real folder; `false` if nothing is there, `NotPlain` if a link or file is.
fn real_dir(path: &Path) -> Result<bool, GitError> {
    match safe_fs::check_real_dir(path) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(read_error(path, e)),
    }
}

/// A Git repository Bolt may read: its checks passed.
#[derive(Debug)]
pub struct Repo {
    git: PathBuf,
    hash: HashKind,
    /// Whether its config has `[include]` or `[includeIf]` sections, which Bolt doesn't follow.
    includes: bool,
    /// The last `lfs.storage` its config sets, and the file that sets it.
    lfs_storage: Option<(Vec<u8>, &'static str)>,
    /// The last `core.sparsecheckout` its config sets; never true, which `open` refuses.
    sparse: Option<bool>,
    objects: odb::Objects,
}

impl Repo {
    /// Opens the Git repository whose work tree is `work_tree`, refusing what Bolt can't import
    /// (decisions 031 and 039) before anything past its format is read.
    pub fn open(work_tree: &Path) -> Result<Repo, GitError> {
        Repo::open_with(
            work_tree,
            &bolt_platform::owner_of,
            &crate::config::safe_folders,
        )
    }

    /// `open`, given how to learn who owns a path and how to read safe-folders.
    fn open_with(
        work_tree: &Path,
        owner_of: &dyn Fn(&Path) -> io::Result<Owner>,
        safe_folders: &dyn Fn() -> Result<Vec<PathBuf>, ConfigError>,
    ) -> Result<Repo, GitError> {
        let git = work_tree.join(".git");
        let meta = match fs::symlink_metadata(&git) {
            Ok(meta) => meta,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Err(GitError::NotAGitFolder),
            Err(e) => return Err(read_error(&git, e)),
        };
        // A linked worktree or a submodule's checkout: the file names where the repository is.
        if safe_fs::is_regular(&meta) {
            return Err(GitError::Refused(Refusal::GitFile));
        }
        safe_fs::check_real_dir(&git).map_err(|e| read_error(&git, e))?;
        check_owner(work_tree, &git, owner_of, safe_folders)?;
        // Looked at again: it may have become a link while its owner was asked.
        safe_fs::check_real_dir(&git).map_err(|e| read_error(&git, e))?;
        let format = read_format(&git)?;
        refuse_layouts(&git)?;
        Ok(Repo {
            git,
            hash: format.hash,
            includes: format.includes,
            lfs_storage: format.lfs_storage,
            sparse: format.sparse,
            objects: odb::Objects::default(),
        })
    }

    #[must_use]
    pub fn hash(&self) -> HashKind {
        self.hash
    }

    /// Whether `.git/config`, or the `config.worktree` Git reads beside it, has an `[include]` or
    /// `[includeIf]` section. Bolt doesn't read the files they name, so the import says it can't
    /// tell whether they turn on sparse checkout (decision 043).
    #[must_use]
    pub fn has_config_includes(&self) -> bool {
        self.includes
    }

    /// `HEAD`, the loose refs and `packed-refs`.
    pub fn refs(&self) -> Result<Refs, GitError> {
        refs::read(&self.git, self.hash, refs::LIMITS)
    }

    /// Whether `.git/info/grafts` is there: Bolt ignores it, so the import says so. Only looked
    /// for, never read, and not through a linked `info`.
    pub fn has_grafts(&self) -> Result<bool, GitError> {
        let info = self.git.join("info");
        Ok(matches!(real_dir(&info), Ok(true)) && exists(&info.join("grafts"))?)
    }

    /// Whether `.git/lfs` is there, where Git LFS keeps its files. Only looked for, never read.
    pub fn has_lfs_folder(&self) -> Result<bool, GitError> {
        exists(&self.git.join("lfs"))
    }

    /// The `lfs.storage` this repository's own config sets, as Git reads it (`config.worktree`
    /// after `.git/config`), empty for one with no value, and the file that sets it. Read only to
    /// tell whether Git LFS keeps its files elsewhere.
    #[must_use]
    pub fn lfs_storage(&self) -> Option<(&[u8], &'static str)> {
        let (value, file) = self.lfs_storage.as_ref()?;
        Some((value, file))
    }

    /// The `core.sparsecheckout` this repository's own config sets, as Git reads it; never true,
    /// which `open` refuses. It wins over the user's own Git settings.
    #[must_use]
    pub fn sparse_checkout(&self) -> Option<bool> {
        self.sparse
    }

    /// Which of `.git/lfs` and `.git/lfs/objects` is a link, or isn't a folder, if one is: Git
    /// LFS's files are then kept behind it, where Bolt never reads. A missing one isn't a link.
    pub fn lfs_folder_link(&self) -> Result<Option<&'static str>, GitError> {
        let lfs = self.git.join("lfs");
        let objects = lfs.join("objects");
        for (folder, shown) in [(lfs, ".git/lfs"), (objects, ".git/lfs/objects")] {
            match safe_fs::check_real_dir(&folder) {
                Ok(()) => {}
                Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
                Err(e) if e.kind() == io::ErrorKind::InvalidInput => return Ok(Some(shown)),
                Err(e) => return Err(read_error(&folder, e)),
            }
        }
        Ok(None)
    }

    /// Where Git LFS keeps the object of `p`: `.git/lfs/objects/<oid[0..2]>/<oid[2..4]>/<oid>`.
    #[must_use]
    pub fn lfs_object_path(&self, p: &LfsPointer) -> PathBuf {
        let oid = p.oid_hex();
        let (first, second) = (oid.get(..2), oid.get(2..4));
        let folder = self.git.join("lfs").join("objects");
        folder
            .join(first.unwrap_or_default())
            .join(second.unwrap_or_default())
            .join(&oid)
    }

    /// The Git LFS object of `p`, opened as every file in `.git` is, without following a link, if
    /// it is a regular file of the size `p` gives. `None` if it isn't there as that: missing, a
    /// folder, a link (to it or to a folder on the way), or a file of another length.
    pub fn lfs_object(&self, p: &LfsPointer) -> Result<Option<File>, GitError> {
        let path = self.lfs_object_path(p);
        // What isn't a real folder or a plain file is never followed: the object isn't there.
        let absent = |e: &io::Error| {
            matches!(
                e.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::InvalidInput
            )
        };
        // `lfs`, `objects` and the two folders the oid names; `open` checked `.git` itself.
        let folders: Vec<&Path> = path.ancestors().skip(1).take(4).collect();
        for folder in folders.into_iter().rev() {
            match safe_fs::check_real_dir(folder) {
                Ok(()) => {}
                Err(e) if absent(&e) => return Ok(None),
                Err(e) => return Err(read_error(folder, e)),
            }
        }
        let file = match safe_fs::open_regular(&path, fs::OpenOptions::new().read(true)) {
            Ok(file) => file,
            Err(e) if absent(&e) => return Ok(None),
            Err(e) => return Err(read_error(&path, e)),
        };
        let len = file.metadata().map_err(|e| read_error(&path, e))?.len();
        Ok((len == p.size).then_some(file))
    }
}

/// Refuses a `.git` that isn't the user's, unless they listed the work tree in safe-folders, as
/// for a Bolt project (T23).
fn check_owner(
    work_tree: &Path,
    git: &Path,
    owner_of: &dyn Fn(&Path) -> io::Result<Owner>,
    safe_folders: &dyn Fn() -> Result<Vec<PathBuf>, ConfigError>,
) -> Result<(), GitError> {
    let refused = |refusal| GitError::NotYours {
        git: git.to_owned(),
        refusal,
    };
    let owner = owner_of(git);
    // Read only when needed, so a mistake in them never stops the user's own repositories.
    let safe = if repo::counts_as_users(&owner) {
        Vec::new()
    } else {
        safe_folders().map_err(|e| refused(RepoError::Config(e)))?
    };
    repo::owner_allows([owner], work_tree, &safe).map_err(refused)
}

/// What `.git/config` says, with what `config.worktree` adds when Git reads it too, after refusing
/// a format, extension or sparse checkout Bolt can't import.
fn read_format(git: &Path) -> Result<config::Format, GitError> {
    // Git reads a repository without a config with its defaults.
    let text = read_optional(&git.join("config"), MAX_CONFIG_BYTES)?.unwrap_or_default();
    let mut format = config::read_format(&text, ".git/config")?;
    if format.worktree_config
        && let Some(text) = read_optional(&git.join("config.worktree"), MAX_CONFIG_BYTES)?
    {
        let worktree = config::read_worktree(&text, ".git/config.worktree")?;
        format.sparse = worktree.sparse.or(format.sparse);
        format.includes |= worktree.includes;
        format.lfs_storage = worktree.lfs_storage.or(format.lfs_storage);
    }
    if format.sparse == Some(true) {
        return Err(GitError::Refused(Refusal::SparseCheckout));
    }
    Ok(format)
}

/// Refuses the files and folders that mean refs, history or objects Bolt doesn't read.
fn refuse_layouts(git: &Path) -> Result<(), GitError> {
    let refuse = |refusal| Err(GitError::Refused(refusal));
    if exists(&git.join("reftable"))? {
        return refuse(Refusal::Reftable);
    }
    if is_shallow(git)? {
        return refuse(Refusal::Shallow);
    }
    let objects = git.join("objects");
    let info = objects.join("info");
    if real_dir(&objects)? && real_dir(&info)? && exists(&info.join("alternates"))? {
        return refuse(Refusal::Alternates);
    }
    // This `.git` is a linked worktree's: the repository it shares is elsewhere.
    if exists(&git.join("commondir"))? {
        return refuse(Refusal::GitFile);
    }
    if has_promisor_pack(&objects)? {
        return refuse(Refusal::PartialClone);
    }
    Ok(())
}

/// Whether `.git/shallow` lists commits whose parents were never fetched; an empty one lists none.
fn is_shallow(git: &Path) -> Result<bool, GitError> {
    let path = git.join("shallow");
    match safe_fs::open_regular(&path, fs::OpenOptions::new().read(true)) {
        Ok(file) => Ok(file.metadata().map_err(|e| read_error(&path, e))?.len() > 0),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(read_error(&path, e)),
    }
}

/// Whether `objects/pack` holds a `.promisor` file: a pack a partial clone fetched.
fn has_promisor_pack(objects: &Path) -> Result<bool, GitError> {
    let pack = objects.join("pack");
    if !real_dir(objects)? || !real_dir(&pack)? {
        return Ok(false);
    }
    for entry in fs::read_dir(&pack).map_err(|e| read_error(&pack, e))? {
        let entry = entry.map_err(|e| read_error(&pack, e))?;
        if entry.file_name().as_encoded_bytes().ends_with(b".promisor") {
            return Ok(true);
        }
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    #![allow(clippy::indexing_slicing, clippy::arithmetic_side_effects)]
    use super::craft::Craft;
    use super::*;
    use crate::config::ConfigError;
    use crate::test_support::{HOSTILE, assert_hostile_text_escaped};
    use bolt_platform::Owner;
    use std::fs;
    use std::io;
    use std::path::{Path, PathBuf};

    fn repo(hash: HashKind) -> (tempfile::TempDir, Craft) {
        let dir = tempfile::tempdir().unwrap();
        let git = Craft::new(dir.path(), hash);
        (dir, git)
    }

    /// A format 1 config with these lines in its `[extensions]`.
    fn v1(extensions: &str) -> String {
        format!("[core]\n\trepositoryformatversion = 1\n[extensions]\n{extensions}")
    }

    #[test]
    fn a_plain_repository_opens() {
        let (dir, _git) = repo(HashKind::Sha1);
        assert_eq!(Repo::open(dir.path()).unwrap().hash(), HashKind::Sha1);
    }

    #[test]
    fn sha256_repositories_open() {
        let (dir, _git) = repo(HashKind::Sha256);
        assert_eq!(Repo::open(dir.path()).unwrap().hash(), HashKind::Sha256);
        // Either hash may be named, and the key's letter case doesn't matter.
        let (dir, git) = repo(HashKind::Sha1);
        git.config(&v1("\tobjectFormat = sha1\n"));
        assert_eq!(Repo::open(dir.path()).unwrap().hash(), HashKind::Sha1);
    }

    /// Makes a repository in the given folder.
    type Setup = fn(&Path);

    #[test]
    fn each_refusal_says_what_to_do() {
        use HashKind::Sha1;
        let cases: [(&str, Setup, Refusal); 15] = [
            (
                "reftable folder",
                |d| Craft::new(d, Sha1).file("reftable/tables.list", b""),
                Refusal::Reftable,
            ),
            (
                "reftable extension",
                |d| Craft::new(d, Sha1).config(&v1("\trefStorage = reftable\n")),
                Refusal::Reftable,
            ),
            (
                "shallow file",
                |d| Craft::new(d, Sha1).file("shallow", format!("{}\n", "a".repeat(40)).as_bytes()),
                Refusal::Shallow,
            ),
            (
                "alternates file",
                |d| Craft::new(d, Sha1).file("objects/info/alternates", b"/elsewhere/objects\n"),
                Refusal::Alternates,
            ),
            (
                "partial clone extension",
                |d| Craft::new(d, Sha1).config(&v1("\tpartialClone = origin\n")),
                Refusal::PartialClone,
            ),
            (
                "partial clone in format 0",
                |d| {
                    let git = Craft::new(d, Sha1);
                    git.config("[core]\n\trepositoryformatversion = 0\n[extensions]\n\tpartialclone = origin\n");
                },
                Refusal::PartialClone,
            ),
            (
                "promisor pack",
                |d| Craft::new(d, Sha1).file("objects/pack/pack-1234.promisor", b""),
                Refusal::PartialClone,
            ),
            (
                "sparse checkout",
                |d| {
                    Craft::new(d, Sha1)
                        .config("[core]\n\trepositoryformatversion = 0\n\tsparseCheckout = true\n")
                },
                Refusal::SparseCheckout,
            ),
            (
                "sparse checkout in config.worktree",
                |d| {
                    let git = Craft::new(d, Sha1);
                    git.config(&v1("\tworktreeConfig = true\n"));
                    git.file("config.worktree", b"[core]\n\tsparseCheckout = true\n");
                },
                Refusal::SparseCheckout,
            ),
            (
                "unknown extension",
                |d| Craft::new(d, Sha1).config(&v1("\tx = y\n")),
                Refusal::UnknownExtension("x".into()),
            ),
            (
                "refs kept outside .git",
                |d| Craft::new(d, Sha1).config(&v1("\trefStorage = files:///elsewhere\n")),
                Refusal::RefsElsewhere,
            ),
            (
                "format version 2",
                |d| Craft::new(d, Sha1).config("[core]\n\trepositoryformatversion = 2\n"),
                Refusal::FormatVersion(2),
            ),
            (
                ".git is a file",
                |d| fs::write(d.join(".git"), "gitdir: ../elsewhere/.git\n").unwrap(),
                Refusal::GitFile,
            ),
            (
                "a linked worktree's .git folder",
                |d| Craft::new(d, Sha1).file("commondir", b"../..\n"),
                Refusal::GitFile,
            ),
            (
                "an empty shallow file is not shallow, but alternates still refuse",
                |d| {
                    let git = Craft::new(d, Sha1);
                    git.file("shallow", b"");
                    git.file("objects/info/alternates", b"");
                },
                Refusal::Alternates,
            ),
        ];
        for (what, setup, refusal) in cases {
            let dir = tempfile::tempdir().unwrap();
            setup(dir.path());
            let err = Repo::open(dir.path())
                .err()
                .unwrap_or_else(|| panic!("{what}: opened"));
            assert!(
                matches!(&err, GitError::Refused(r) if *r == refusal),
                "{what}: {err:?}"
            );
            assert!(err.hint().is_some(), "{what}");
        }
    }

    #[test]
    fn what_doesnt_refuse_opens() {
        use HashKind::Sha1;
        for (what, config) in [
            (
                "unknown extensions in format 0",
                "[core]\n\trepositoryformatversion = 0\n[extensions]\n\tx = y\n".to_owned(),
            ),
            (
                "extensions Git accepts",
                v1(
                    "\tnoop = true\n\tnoop-v1 = true\n\tpreciousObjects = true\n\trelativeWorktrees = true\n\tsubmodulePathConfig = true\n\tcompatObjectFormat = sha256\n\trefStorage = files\n",
                ),
            ),
            (
                "sparse checkout turned off",
                "[core]\n\trepositoryformatversion = 0\n\tsparseCheckout = false\n".to_owned(),
            ),
            // Without worktreeConfig, Git doesn't read config.worktree.
            (
                "config.worktree unread",
                "[core]\n\trepositoryformatversion = 1\n".to_owned(),
            ),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let git = Craft::new(dir.path(), Sha1);
            git.config(&config);
            git.file("config.worktree", b"[core]\n\tsparseCheckout = true\n");
            assert!(Repo::open(dir.path()).is_ok(), "{what}");
        }
        // config.worktree is read last, as Git reads it, and may turn sparse checkouts off.
        let dir = tempfile::tempdir().unwrap();
        let git = Craft::new(dir.path(), Sha1);
        git.config(&v1(
            "\tworktreeConfig = true\n[core]\n\tsparseCheckout = true\n",
        ));
        git.file("config.worktree", b"[core]\n\tsparseCheckout = false\n");
        assert!(Repo::open(dir.path()).is_ok());
        // A repository without a config is read with Git's defaults.
        let (dir, _git) = repo(Sha1);
        fs::remove_file(dir.path().join(".git/config")).unwrap();
        assert_eq!(Repo::open(dir.path()).unwrap().hash(), Sha1);
    }

    #[test]
    fn a_dot_git_link_is_refused() {
        let (real, _git) = repo(HashKind::Sha1);
        let dir = tempfile::tempdir().unwrap();
        let link = dir.path().join(".git");
        #[cfg(unix)]
        std::os::unix::fs::symlink(real.path().join(".git"), &link).unwrap();
        #[cfg(windows)]
        crate::test_support::junction(&link, &real.path().join(".git"));
        match Repo::open(dir.path()) {
            Err(e @ GitError::NotPlain { .. }) => {
                assert!(e.to_string().contains("not a real folder"), "{e}");
                // `git fsck` follows the link and finds nothing wrong, so the hint says what to do.
                let hint = e.hint().unwrap();
                assert!(hint.contains("never follows links"), "{hint}");
                assert!(!hint.contains("fsck"), "{hint}");
            }
            other => panic!("{other:?}"),
        }
        // Nothing at all is not a repository.
        let empty = tempfile::tempdir().unwrap();
        let err = Repo::open(empty.path()).err().unwrap();
        assert!(matches!(err, GitError::NotAGitFolder), "{err:?}");
        assert_eq!(
            err.to_string(),
            "No Git repository here: run bolt import at the top of a Git project, where its \
             .git folder is."
        );
    }

    #[test]
    fn config_lines_bolt_cant_read_refuse_with_the_line_number() {
        let v0 = "[core]\n\trepositoryformatversion = 0\n";
        for (config, line) in [
            (format!("{v0}[broken\n"), 3),
            (format!("{v0}\tbare = \"true\n"), 3),
            (format!("{v0}\tx = a\\qb\n"), 3),
            (format!("{v0}1x = 2\n"), 3),
            ("[core]\n\trepositoryformatversion\n".to_owned(), 2),
            ("[core]\n\trepositoryformatversion = one\n".to_owned(), 2),
            ("[core]\n\trepositoryformatversion = 0\0\n".to_owned(), 2),
            ("[core]\n\tsparseCheckout = maybe\n".to_owned(), 2),
            (v1("\tobjectFormat = sha3\n"), 4),
            (v1("\tpartialClone\n"), 4),
            (v1("\trefStorage = postgres://db\n"), 4),
            (
                v1("\tcompatObjectFormat = sha1\n\tcompatObjectFormat = sha256\n"),
                5,
            ),
        ] {
            let (dir, git) = repo(HashKind::Sha1);
            git.config(&config);
            match Repo::open(dir.path()) {
                Err(e @ GitError::Broken { .. }) => {
                    let want = format!("line {line} of .git/config");
                    assert!(e.to_string().contains(&want), "{config:?}: {e}");
                    assert!(e.hint().is_some());
                }
                other => panic!("{config:?}: {other:?}"),
            }
        }
        let (dir, git) = repo(HashKind::Sha1);
        git.config(&v1("\tworktreeConfig = true\n"));
        git.file("config.worktree", b"[core]\n\tsparseCheckout = sometimes\n");
        let err = Repo::open(dir.path()).err().unwrap();
        assert!(
            err.to_string().contains("line 2 of .git/config.worktree"),
            "{err}"
        );
    }

    #[test]
    fn a_config_bolt_cant_hold_is_refused() {
        let (dir, git) = repo(HashKind::Sha1);
        git.file("config", &vec![b'#'; (1 << 20) + 1]);
        let err = Repo::open(dir.path()).err().unwrap();
        assert!(matches!(err, GitError::Broken { .. }), "{err:?}");
        let (dir, _git) = repo(HashKind::Sha1);
        let config = dir.path().join(".git/config");
        fs::remove_file(&config).unwrap();
        fs::create_dir(&config).unwrap();
        let err = Repo::open(dir.path()).err().unwrap();
        assert!(matches!(err, GitError::NotPlain { .. }), "{err:?}");
    }

    #[test]
    fn a_dot_git_owned_by_another_account_is_refused() {
        let (dir, _git) = repo(HashKind::Sha1);
        let root = dir.path();
        let git = root.join(".git");
        // Who owns the folder, then how many times the reader asked.
        let asked = std::cell::Cell::new(0);
        let open_as = |owner: Owner, safe: &[PathBuf]| {
            let safe = safe.to_vec();
            let owner_of = |path: &Path| {
                assert_eq!(path, git);
                asked.set(asked.get() + 1);
                Ok(owner)
            };
            Repo::open_with(root, &owner_of, &move || Ok(safe.clone()))
        };
        match open_as(Owner::Other, &[]) {
            Err(
                e @ GitError::NotYours {
                    refusal: RepoError::NotYours { .. },
                    ..
                },
            ) => {
                // The message names the `.git` folder, whose owner was asked; the hint lists
                // the work tree, which is what safe-folders trusts.
                let shown = |path: &Path| escape(&path.display().to_string(), false);
                assert!(e.to_string().starts_with("The Git repository at "), "{e}");
                assert!(e.to_string().contains(&shown(&git)), "{e}");
                let hint = e.hint().unwrap();
                assert_eq!(hint, crate::config::trust_hint(root), "{hint}");
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(asked.get(), 1);
        // Each reason keeps its own message, about the `.git` folder.
        let admins = open_as(Owner::Administrators { elevated: false }, &[]);
        let unknown = open_as(Owner::Unknown, &[]);
        let failed = Repo::open_with(
            root,
            &|_| Err(io::Error::other("Access is denied.")),
            &|| Ok(Vec::new()),
        );
        for (result, reason) in [
            (admins, "Administrators group"),
            (unknown, "can't tell who owns"),
            (failed, "Access is denied."),
        ] {
            match result {
                Err(e @ GitError::NotYours { .. }) => {
                    assert!(e.to_string().contains(reason), "{e}");
                    assert!(e.to_string().contains("Git repository"), "{e}");
                    assert!(e.hint().unwrap().contains("safe-folders"), "{e}");
                }
                other => panic!("{other:?}"),
            }
        }
        // Listing the work tree in safe-folders trusts its `.git`.
        asked.set(0);
        assert!(open_as(Owner::Other, &[root.to_owned()]).is_ok());
        assert_eq!(asked.get(), 1);
        // The user's own `.git` opens without reading safe-folders, even broken ones.
        let broken = || -> Result<Vec<PathBuf>, ConfigError> {
            Err(ConfigError::Invalid {
                path: PathBuf::from("config.toml"),
                detail: "broken".into(),
            })
        };
        for owner in [Owner::User, Owner::Administrators { elevated: true }] {
            assert!(Repo::open_with(root, &|_| Ok(owner), &broken).is_ok());
        }
        // Another account's, with safe-folders unreadable: the settings' own error.
        match Repo::open_with(root, &|_| Ok(Owner::Other), &broken) {
            Err(
                e @ GitError::NotYours {
                    refusal: RepoError::Config(_),
                    ..
                },
            ) => assert!(
                e.to_string().contains("not a valid Bolt settings file"),
                "{e}"
            ),
            other => panic!("{other:?}"),
        }
    }

    /// `safe_fs` refusing what isn't the plain file or real folder Git keeps is `NotPlain`, and a
    /// file past its cap damage; a file gone mid-read is Git at work, the rest is I/O. Each names
    /// the path once.
    #[test]
    fn read_errors_are_told_apart() {
        let path = Path::new("refs/heads/main");
        let kind = |kind| read_error(path, io::Error::from(kind));
        // As the system gives them: no path in the message, so Bolt adds it.
        match kind(io::ErrorKind::InvalidInput) {
            GitError::NotPlain { what } => assert!(what.starts_with("refs/heads/main: "), "{what}"),
            other => panic!("{other:?}"),
        }
        match kind(io::ErrorKind::InvalidData) {
            GitError::Broken { what } => assert!(what.starts_with("refs/heads/main: "), "{what}"),
            other => panic!("{other:?}"),
        }
        // As safe_fs gives it: the path is in the message already.
        let named = io::Error::new(
            io::ErrorKind::InvalidInput,
            "refs/heads/main is not a regular file",
        );
        match read_error(path, named) {
            e @ GitError::NotPlain { .. } => {
                assert_eq!(
                    e.to_string(),
                    "Bolt can't import this Git repository: refs/heads/main is not a regular file."
                );
                assert!(e.hint().unwrap().contains("never follows links"), "{e}");
            }
            other => panic!("{other:?}"),
        }
        match kind(io::ErrorKind::NotFound) {
            e @ GitError::Changed { .. } => {
                assert!(e.to_string().contains("refs/heads/main"), "{e}");
                assert!(e.hint().unwrap().contains("bolt import again"));
            }
            other => panic!("{other:?}"),
        }
        match kind(io::ErrorKind::PermissionDenied) {
            e @ GitError::Io { .. } => {
                assert!(e.to_string().contains("refs/heads/main"), "{e}");
                assert!(e.hint().unwrap().contains("may read it"), "{e}");
            }
            other => panic!("{other:?}"),
        }
    }

    /// Running out of memory says so and how to free some, not that the file may be locked.
    /// A `.git` refused although safe-folders lists the folder through a link names the `.git` and
    /// the entry passed over, and keeps the trust hint.
    #[test]
    fn a_git_refusal_names_a_safe_folders_entry_it_passed_over() {
        let e = GitError::NotYours {
            git: "work/.git".into(),
            refusal: RepoError::SafeFolderPassedOver {
                entry: "trusted".into(),
                why: "it is a link",
                refusal: Box::new(RepoError::NotYours {
                    path: "work".into(),
                }),
            },
        };
        let said = e.to_string();
        assert!(
            said.starts_with("The Git repository at work")
                && said.contains("belongs to another user account. Its safe-folders entry trusted")
                && said.contains("doesn't count: it is a link"),
            "{said}"
        );
        assert!(e.hint().unwrap().contains("safe-folders"), "{e:?}");
    }

    #[test]
    fn running_out_of_memory_says_so() {
        let path = Path::new("objects/pack/pack-1.pack");
        for e in [
            read_error(path, io::ErrorKind::OutOfMemory.into()),
            out_of_memory(path),
        ] {
            assert!(matches!(e, GitError::OutOfMemory { .. }), "{e:?}");
            assert_eq!(
                e.to_string(),
                "Bolt ran out of memory while reading objects/pack/pack-1.pack."
            );
            let hint = e.hint().unwrap();
            assert!(
                hint.contains("Close other programs to free memory") && !hint.contains("locked"),
                "{hint}"
            );
        }
    }

    #[test]
    fn messages_show_repository_text_escaped() {
        let refused = GitError::Refused(Refusal::UnknownExtension(HOSTILE.to_owned()));
        assert_hostile_text_escaped(&refused.to_string());
        for err in [
            GitError::Broken {
                what: HOSTILE.into(),
            },
            GitError::NotPlain {
                what: HOSTILE.into(),
            },
            GitError::EndedEarly {
                what: HOSTILE.into(),
            },
            GitError::Unimportable {
                what: HOSTILE.into(),
            },
            GitError::TooManyRefs {
                what: HOSTILE.into(),
            },
            GitError::TooManyObjects {
                what: HOSTILE.into(),
            },
            GitError::TooManyPacks {
                what: HOSTILE.into(),
            },
            GitError::TooMuchWork {
                what: HOSTILE.into(),
            },
            GitError::Bug {
                what: HOSTILE.into(),
            },
            GitError::NotYours {
                git: HOSTILE.into(),
                refusal: RepoError::NotYours { path: "x".into() },
            },
            GitError::NotYours {
                git: "x".into(),
                refusal: RepoError::OwnerLookupFailed {
                    path: "x".into(),
                    source: io::Error::other(HOSTILE),
                },
            },
            GitError::NotYours {
                git: "x".into(),
                refusal: RepoError::SafeFolderPassedOver {
                    entry: HOSTILE.into(),
                    why: "it is a link",
                    refusal: Box::new(RepoError::NotYours { path: "x".into() }),
                },
            },
            GitError::Changed {
                what: HOSTILE.into(),
            },
            GitError::OutOfMemory {
                path: HOSTILE.into(),
            },
            GitError::Io {
                path: HOSTILE.into(),
                source: io::Error::other("denied"),
            },
            GitError::Io {
                path: "x".into(),
                source: io::Error::other(HOSTILE),
            },
        ] {
            assert_hostile_text_escaped(&err.to_string());
            assert!(!err.to_string().contains('\n'));
        }
    }
}
