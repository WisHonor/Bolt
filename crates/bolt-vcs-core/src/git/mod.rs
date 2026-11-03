//! Bolt's own read-only Git reader (decision 031). `Repo::open` checks a repository's format and
//! refuses what Bolt can't import; `Repo::refs` reads its refs. Everything under `.git` is read
//! through `safe_fs`, bounded, and treated as possibly hostile.
#![deny(clippy::indexing_slicing, clippy::arithmetic_side_effects)]

mod config;
#[cfg(any(test, feature = "test-support"))]
pub mod craft;
mod refs;

pub use refs::{Head, Refs};

use std::fmt::Write as _;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use bolt_platform::Owner;
// The hashing trait of both hashers (digest 0.10).
use sha2::Digest as _;

use crate::config::ConfigError;
use crate::repo::{self, RepoError};
use crate::safe_fs;
use crate::text::escape;

/// `.git/config` and `.git/config.worktree`, each.
const MAX_CONFIG_BYTES: u64 = 1 << 20;

/// The hash a repository names its objects with.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub enum HashKind {
    Sha1,
    Sha256,
}

impl HashKind {
    /// Bytes in an object ID.
    #[must_use]
    #[allow(clippy::len_without_is_empty)] // no ID is empty
    pub fn len(self) -> usize {
        match self {
            HashKind::Sha1 => 20,
            HashKind::Sha256 => 32,
        }
    }

    /// Hex digits in an object ID.
    fn hex_len(self) -> usize {
        match self {
            HashKind::Sha1 => 40,
            HashKind::Sha256 => 64,
        }
    }
}

/// A Git object's ID: `kind.len()` bytes of `bytes`, the rest zero.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct GitId {
    kind: HashKind,
    bytes: [u8; 32],
}

impl GitId {
    fn new(kind: HashKind, digest: &[u8]) -> GitId {
        let mut bytes = [0; 32];
        bytes.iter_mut().zip(digest).for_each(|(b, d)| *b = *d);
        GitId { kind, bytes }
    }

    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        self.bytes.get(..self.kind.len()).unwrap_or_default()
    }

    /// The all-zero ID, which names no object.
    fn is_zero(&self) -> bool {
        self.bytes.iter().all(|&b| b == 0)
    }

    /// The ID in lowercase hex, as Git writes it.
    #[must_use]
    pub fn to_hex(&self) -> String {
        let mut hex = String::with_capacity(self.kind.hex_len());
        for b in self.as_bytes() {
            let _ = write!(hex, "{b:02x}");
        }
        hex
    }

    /// The ID `hex` spells, in either letter case as Git reads it; `None` unless it is exactly
    /// the hex digits of one ID of `kind`.
    #[must_use]
    pub fn from_hex(kind: HashKind, hex: &str) -> Option<GitId> {
        let digits = hex.as_bytes();
        if digits.len() != kind.hex_len() || !digits.iter().all(u8::is_ascii_hexdigit) {
            return None;
        }
        let mut bytes = [0; 32];
        for (byte, pair) in bytes.iter_mut().zip(digits.as_chunks::<2>().0) {
            *byte = u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok()?;
        }
        Some(GitId { kind, bytes })
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ObjectType {
    Commit,
    Tree,
    Blob,
    Tag,
}

impl ObjectType {
    /// The name in an object's header.
    fn name(self) -> &'static str {
        match self {
            ObjectType::Commit => "commit",
            ObjectType::Tree => "tree",
            ObjectType::Blob => "blob",
            ObjectType::Tag => "tag",
        }
    }
}

/// What Git puts before an object's bytes, both to hash and to store: `<type> <length>\0`.
fn object_header(kind: ObjectType, len: usize) -> String {
    format!("{} {len}\0", kind.name())
}

/// Git's ID for an object of `kind` holding `data`: the hash of its header and bytes. SHA-1 runs
/// with collision detection, as in Git, and an object made for a collision attack is `Broken`.
pub fn object_id(kind: ObjectType, data: &[u8], hash: HashKind) -> Result<GitId, GitError> {
    let header = object_header(kind, data.len());
    match hash {
        HashKind::Sha1 => {
            let mut hasher = sha1_checked::Sha1::new();
            hasher.update(header.as_bytes());
            hasher.update(data);
            let result = hasher.try_finalize();
            if result.has_collision() {
                return Err(broken(format!(
                    "a {} of {} bytes is part of a SHA-1 collision attack",
                    kind.name(),
                    data.len()
                )));
            }
            Ok(GitId::new(hash, result.hash()))
        }
        HashKind::Sha256 => {
            let mut hasher = sha2::Sha256::new();
            hasher.update(header.as_bytes());
            hasher.update(data);
            Ok(GitId::new(hash, &hasher.finalize()))
        }
    }
}

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
    /// More refs, or longer names, than Bolt reads; `what` names the limit.
    #[error("Bolt can't import this Git repository: {}.", escape(what, false))]
    TooManyRefs { what: String },
    #[error(
        "Git changed the repository while Bolt read it: {}.",
        escape(what, false)
    )]
    Changed { what: String },
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
                GitError::TooManyRefs { .. } => {
                    "Nothing was changed. Delete refs you don't need, such as stale server \
                     branches (`git remote prune <server>`) or old tags (`git tag -d <tag>`), \
                     then run bolt import again."
                }
                GitError::Changed { .. } => {
                    "Nothing was changed. Run bolt import again once Git has finished."
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
fn owner_message(git: &Path, refusal: &RepoError) -> String {
    let git = escape(&git.display().to_string(), false);
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
        // The user's settings, which hold safe-folders, can't be read.
        other => other.to_string(),
    }
}

fn broken(what: impl Into<String>) -> GitError {
    GitError::Broken { what: what.into() }
}

/// Git's `isspace`: space, tab, LF and CR, but not vertical tab or form feed.
fn is_git_space(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\n' | b'\r')
}

/// A `safe_fs` refusal (a link, special file or oversized file where Git keeps a plain one) is
/// `Broken`, a file gone while Bolt read is `Changed`, anything else `Io`. Each names `path`.
fn read_error(path: &Path, e: io::Error) -> GitError {
    match e.kind() {
        io::ErrorKind::InvalidInput | io::ErrorKind::InvalidData => broken(naming(path, &e)),
        io::ErrorKind::NotFound => GitError::Changed {
            what: format!("{} went away", path.display()),
        },
        _ => GitError::Io {
            path: path.to_owned(),
            source: e,
        },
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

/// Whether `path` is a real folder; `false` if nothing is there, `Broken` if a link or file is.
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
        let hash = read_format(&git)?;
        refuse_layouts(&git)?;
        Ok(Repo { git, hash })
    }

    #[must_use]
    pub fn hash(&self) -> HashKind {
        self.hash
    }

    /// `HEAD`, the loose refs and `packed-refs`.
    pub fn refs(&self) -> Result<Refs, GitError> {
        refs::read(&self.git, self.hash, refs::LIMITS)
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

/// The repository's hash, after refusing a format, extension or sparse checkout Bolt can't import.
fn read_format(git: &Path) -> Result<HashKind, GitError> {
    // Git reads a repository without a config with its defaults.
    let text = read_optional(&git.join("config"), MAX_CONFIG_BYTES)?.unwrap_or_default();
    let format = config::read_format(&text, ".git/config")?;
    let mut sparse = format.sparse;
    if format.worktree_config
        && let Some(text) = read_optional(&git.join("config.worktree"), MAX_CONFIG_BYTES)?
        && let Some(value) = config::read_sparse(&text, ".git/config.worktree")?
    {
        sparse = Some(value);
    }
    if sparse == Some(true) {
        return Err(GitError::Refused(Refusal::SparseCheckout));
    }
    Ok(format.hash)
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

    #[test]
    fn object_ids_match_git() {
        // From `git hash-object` and `git mktree --missing`, in SHA-1 and SHA-256 repositories.
        let blob = b"hello\n";
        let rows = [
            (
                HashKind::Sha1,
                "ce013625030ba8dba906f756967f9e9ca394464a",
                "4b825dc642cb6eb9a060e54bf8d69288fbee4904",
                "b4d01e9b0c4a9356736dfddf8830ba9a54f5271c",
            ),
            (
                HashKind::Sha256,
                "2cf8d83d9ee29543b34a87727421fdecb7e3f3a183d337639025de576db9ebb4",
                "6ef19b41225c5369f1c104d45d8d85efa9b057b53b14b4b9b939dd74decc5321",
                "ab39bc840914e6c219053910a617f89e6e0b561cfab5953c79f014c4302f8a01",
            ),
        ];
        for (hash, blob_id, empty_tree, tree) in rows {
            let id = object_id(ObjectType::Blob, blob, hash).unwrap();
            assert_eq!(id.to_hex(), blob_id);
            assert_eq!(id.as_bytes().len(), hash.len());
            assert_eq!(GitId::from_hex(hash, blob_id), Some(id));
            let empty = object_id(ObjectType::Tree, b"", hash).unwrap();
            assert_eq!(empty.to_hex(), empty_tree);
            // Craft stores what Git would: the header and bytes, compressed, under the ID.
            let (dir, git) = repo(hash);
            assert_eq!(git.object(ObjectType::Blob, blob), id);
            let (folder, file) = blob_id.split_at(2);
            let stored = fs::read(dir.path().join(".git/objects").join(folder).join(file)).unwrap();
            let raw = miniz_oxide::inflate::decompress_to_vec_zlib(&stored).unwrap();
            assert_eq!(raw, b"blob 6\0hello\n");
            assert_eq!(git.tree(&[(b"100644", b"hello", id)]).to_hex(), tree);
        }
    }

    #[test]
    fn ids_are_read_from_hex_as_git_reads_them() {
        let lower = "ce013625030ba8dba906f756967f9e9ca394464a";
        let id = GitId::from_hex(HashKind::Sha1, lower).unwrap();
        assert_eq!(
            GitId::from_hex(HashKind::Sha1, &lower.to_uppercase()),
            Some(id)
        );
        for bad in [
            &lower[..39],
            &format!("{lower}0"),
            &format!("{}g", &lower[..39]),
            &format!("+{}", &lower[..39]),
            "",
        ] {
            assert_eq!(GitId::from_hex(HashKind::Sha1, bad), None, "{bad:?}");
        }
        // An ID of the other hash is not one of this one.
        assert_eq!(GitId::from_hex(HashKind::Sha256, lower), None);
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
            Err(GitError::Broken { what }) => assert!(what.contains("not a real folder"), "{what}"),
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
        assert!(matches!(err, GitError::Broken { .. }), "{err:?}");
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

    /// A refusal from `safe_fs` is damage, a file gone mid-read is Git at work, the rest is I/O;
    /// each names the path once.
    #[test]
    fn read_errors_are_told_apart() {
        let path = Path::new("refs/heads/main");
        let kind = |kind| read_error(path, io::Error::from(kind));
        for k in [io::ErrorKind::InvalidInput, io::ErrorKind::InvalidData] {
            // As the system gives it: no path in its message, so Bolt adds it.
            match kind(k) {
                GitError::Broken { what } => {
                    assert!(what.starts_with("refs/heads/main: "), "{what}")
                }
                other => panic!("{other:?}"),
            }
        }
        // As safe_fs gives it: the path is in the message already.
        let named = io::Error::new(
            io::ErrorKind::InvalidInput,
            "refs/heads/main is not a regular file",
        );
        match read_error(path, named) {
            GitError::Broken { what } => assert_eq!(what, "refs/heads/main is not a regular file"),
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

    #[test]
    fn messages_show_repository_text_escaped() {
        let refused = GitError::Refused(Refusal::UnknownExtension(HOSTILE.to_owned()));
        assert_hostile_text_escaped(&refused.to_string());
        for err in [
            GitError::Broken {
                what: HOSTILE.into(),
            },
            GitError::TooManyRefs {
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
            GitError::Changed {
                what: HOSTILE.into(),
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
