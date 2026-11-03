//! The repository: the `format` file, the head pointer, the change lock and the commit protocol
//! (docs/storage-format.md section 10, decision 010).

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, TryLockError};
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use bolt_platform::Owner;

use crate::config::{self, ConfigError};
use crate::diary::{Action, DiaryEntry, Position, ServerLines, State, TagList};
use crate::encoding::DecodeError;
use crate::fsio::{self, fault_point, write_atomic};
use crate::id::{ChangeId, Id};
use crate::listing::Listing;
use crate::pack::{ObjectKind, PackError, PackSet};
use crate::safe_fs;
use crate::save::{EXTRA_LINE, Person, Save};
use crate::store::{Store, StoreError};
use crate::text::escape;

pub const FORMAT_TEXT: &str = "bolt-repository 1\nhash blake3\n";

/// What went wrong with a project. A path or detail in a message may hold text from outside Bolt
/// (the project folder's name, a hostile `.bolt/format`), so it is shown escaped.
#[derive(Debug, thiserror::Error)]
pub enum RepoError {
    #[error("Not inside a Bolt project.")]
    NotAProject,
    #[error("This folder is already a Bolt project.")]
    AlreadyAProject { root: PathBuf },
    #[error(
        "There's a .bolt folder here that Bolt didn't make (bolt.new and other tools use that \
         name); move or rename it first."
    )]
    OtherToolsFolder { root: PathBuf },
    #[error(
        "This folder is inside a Bolt project at {}.",
        escape(&root.display().to_string(), false)
    )]
    InsideAnotherProject { root: PathBuf },
    #[error(
        "The project at {} belongs to another user account.",
        escape(&path.display().to_string(), false)
    )]
    NotYours { path: PathBuf },
    #[error(
        "The project at {} belongs to the Administrators group, and this terminal isn't running \
         as administrator.",
        escape(&path.display().to_string(), false)
    )]
    AdminsNotElevated { path: PathBuf },
    #[error(
        "Bolt can't tell who owns the project at {}: this drive doesn't record who owns folders.",
        escape(&path.display().to_string(), false)
    )]
    OwnerUnknown { path: PathBuf },
    #[error(
        "Bolt couldn't find out who owns the project at {}: {}",
        escape(&path.display().to_string(), false),
        escape(&source.to_string(), false)
    )]
    OwnerLookupFailed { path: PathBuf, source: io::Error },
    /// `bolt start` made `.bolt` but may not use it. Once the folder is trusted, running it again
    /// finishes the project.
    #[error("The .bolt folder was made, but the project isn't finished. {refusal}")]
    StartStopped { refusal: Box<RepoError> },
    #[error(
        "This project uses {}, which this version of Bolt doesn't understand.",
        escape(detail, false)
    )]
    NewerFormat { detail: String },
    #[error("This project's Bolt data looks damaged: {}.", escape(detail, false))]
    Damaged { detail: String },
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error(transparent)]
    Store(StoreError),
    #[error(transparent)]
    Pack(PackError),
    #[error(transparent)]
    Decode(#[from] DecodeError),
    #[error(transparent)]
    Config(#[from] ConfigError),
}

impl RepoError {
    /// What the user can do next, if there is something to say.
    #[must_use]
    pub fn hint(&self) -> Option<String> {
        Some(
            match self {
                RepoError::NotAProject => "Run `bolt start` to make this folder a Bolt project.",
                RepoError::InsideAnotherProject { .. } => {
                    "Run Bolt commands there, or pick a folder outside it."
                }
                RepoError::NotYours { path }
                | RepoError::OwnerUnknown { path }
                | RepoError::OwnerLookupFailed { path, .. } => {
                    return Some(config::trust_hint(path));
                }
                RepoError::AdminsNotElevated { path } => {
                    return Some(format!(
                        "Open a terminal as administrator and run the command there.\n{}",
                        config::trust_hint(path)
                    ));
                }
                RepoError::StartStopped { refusal } => {
                    return refusal.hint_then("Then run `bolt start` again to finish.");
                }
                RepoError::NewerFormat { .. } => "Update Bolt to open it.",
                RepoError::Damaged { .. } => {
                    "Nothing was changed. Ask for help before trying to repair it."
                }
                RepoError::Config(e) => return e.hint(),
                _ => return None,
            }
            .to_owned(),
        )
    }

    /// `hint`, then `then`, for a refusal that stopped `bolt start` or `bolt import` after `.bolt`
    /// was made. Where other accounts can write, the hint says to move the project instead, so
    /// `then` is left off.
    pub(crate) fn hint_then(&self, then: &str) -> Option<String> {
        let hint = self.hint()?;
        let moves = match self {
            RepoError::NotYours { path }
            | RepoError::OwnerUnknown { path }
            | RepoError::OwnerLookupFailed { path, .. }
            | RepoError::AdminsNotElevated { path } => config::others_can_write(path),
            _ => false,
        };
        Some(if moves {
            hint
        } else {
            format!("{hint}\n{then}")
        })
    }
}

impl From<StoreError> for RepoError {
    fn from(e: StoreError) -> Self {
        match e {
            StoreError::Pack(p) => p.into(),
            // A save past the limits of the format is no save to trust (decision 025).
            StoreError::SaveTooLarge { .. } => damaged(e.to_string()),
            other => RepoError::Store(other),
        }
    }
}

impl From<PackError> for RepoError {
    /// A pack that fails its checks is damage, and the user must hear it as such (decision 017).
    fn from(e: PackError) -> Self {
        match e {
            PackError::Corrupt { path, reason } => {
                damaged(format!("pack {} ({reason})", path.display()))
            }
            PackError::FingerprintMismatch { id } => {
                damaged(format!("object {id} does not match its fingerprint"))
            }
            other => RepoError::Pack(other),
        }
    }
}

fn damaged(detail: impl Into<String>) -> RepoError {
    RepoError::Damaged {
        detail: detail.into(),
    }
}

/// A `safe_fs` refusal (a link, special file or oversized file where `.bolt` must hold a real
/// one) is damage; any other error stays an I/O error.
fn refused(e: io::Error) -> RepoError {
    match e.kind() {
        io::ErrorKind::InvalidInput | io::ErrorKind::InvalidData => damaged(e.to_string()),
        _ => e.into(),
    }
}

/// The folders that must be real folders, never links or junctions (storage-format.md §10),
/// parents first.
fn fixed_folders(bolt: &Path) -> [PathBuf; 6] {
    let store = bolt.join("store");
    let packs = store.join("packs");
    [
        bolt.to_owned(),
        store,
        packs,
        bolt.join("state"),
        bolt.join("cache"),
        bolt.join("tmp"),
    ]
}

/// Everything Bolt keeps directly in `.bolt` (storage-format.md §10).
const BOLT_ENTRIES: [&str; 6] = ["format", "store", "state", "cache", "tmp", "config.toml"];

/// Whether `.bolt` holds only names Bolt puts there. Anything else means another tool made it:
/// bolt.new and others use the name too.
fn holds_only_bolts_own(bolt: &Path) -> Result<bool, RepoError> {
    for entry in fs::read_dir(bolt)? {
        let name = entry?.file_name();
        if !BOLT_ENTRIES.iter().any(|own| name == *own) {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Refuses a `.bolt` whose fixed folders are links, junctions or files. A missing one passes:
/// `begin` creates them, and `open` makes `tmp` and `cache` again and reports the others as damage.
fn check_shape(bolt: &Path) -> Result<(), RepoError> {
    for dir in fixed_folders(bolt) {
        match safe_fs::check_real_dir(&dir) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(refused(e)),
            _ => {}
        }
    }
    Ok(())
}

/// Refuses the project at `root` unless the folder and its `.bolt` are the user's, or the user
/// listed `root` in safe-folders, as Git does (T23).
fn check_owner(root: &Path) -> Result<(), RepoError> {
    check_owner_with(root, &bolt_platform::owner_of, &config::safe_folders)
}

/// `check_owner`, given how to learn who owns a path and how to read safe-folders.
fn check_owner_with(
    root: &Path,
    owner_of: &dyn Fn(&Path) -> io::Result<Owner>,
    safe_folders: &dyn Fn() -> Result<Vec<PathBuf>, ConfigError>,
) -> Result<(), RepoError> {
    let mut owners = vec![owner_of(root)];
    match owner_of(&root.join(".bolt")) {
        // Before `bolt start` makes `.bolt`, the folder alone decides.
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        owner => owners.push(owner),
    }
    // Read only when needed, so a mistake in them never stops the user's own projects.
    let safe = if owners.iter().all(counts_as_users) {
        Vec::new()
    } else {
        safe_folders()?
    };
    owner_allows(owners, root, &safe)
}

/// Whether `owner` is the user's: their own account, or the Administrators group while it is
/// switched on in this terminal.
pub(crate) fn counts_as_users(owner: &io::Result<Owner>) -> bool {
    matches!(
        owner,
        Ok(Owner::User | Owner::Administrators { elevated: true })
    )
}

/// Whether Bolt may use the project at `root`, given who owns the folder and its `.bolt` (or why
/// that couldn't be found out): all must be the user's, unless `safe` lists `root` itself. The
/// Administrators group is the user's only while it is switched on in this terminal.
pub(crate) fn owner_allows(
    owners: impl IntoIterator<Item = io::Result<Owner>>,
    root: &Path,
    safe: &[PathBuf],
) -> Result<(), RepoError> {
    let path = || root.to_owned();
    let refusal = owners.into_iter().find_map(|owner| match owner {
        Ok(Owner::User | Owner::Administrators { elevated: true }) => None,
        Ok(Owner::Administrators { elevated: false }) => {
            Some(RepoError::AdminsNotElevated { path: path() })
        }
        Ok(Owner::Other) => Some(RepoError::NotYours { path: path() }),
        Ok(Owner::Unknown) => Some(RepoError::OwnerUnknown { path: path() }),
        Err(source) => Some(RepoError::OwnerLookupFailed {
            path: path(),
            source,
        }),
    });
    match refusal {
        Some(refusal) if !safe.iter().any(|folder| config::same_folder(folder, root)) => {
            Err(refusal)
        }
        _ => Ok(()),
    }
}

/// What `bolt start` tells the user when the project at `root` will open only from an
/// administrator terminal: its folder or `.bolt` belongs to the Administrators group, which counts
/// as the user's only there.
#[must_use]
pub fn elevated_only_note(root: &Path) -> Option<String> {
    elevated_only_note_with(root, &bolt_platform::owner_of)
}

/// `elevated_only_note`, given how to learn who owns a path.
fn elevated_only_note_with(
    root: &Path,
    owner_of: &dyn Fn(&Path) -> io::Result<Owner>,
) -> Option<String> {
    let group_owns =
        |path: &Path| matches!(owner_of(path), Ok(Owner::Administrators { elevated: true }));
    (group_owns(root) || group_owns(&root.join(".bolt"))).then(|| {
        format!(
            "This project belongs to the Administrators group, so it opens only from an \
             administrator terminal.\n{}",
            config::trust_hint(root)
        )
    })
}

/// Checks `root` and its `.bolt` before anything inside is looked at. `owner_check` must allow
/// both, and `.bolt`, if there, must be a real folder. The folders inside `.bolt` must be real too.
/// `Ok(false)`: no `.bolt`.
fn check_bolt(
    root: &Path,
    owner_check: &dyn Fn(&Path) -> Result<(), RepoError>,
) -> Result<bool, RepoError> {
    let bolt = root.join(".bolt");
    let is_there = || match safe_fs::check_real_dir(&bolt) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(refused(e)),
    };
    let found = is_there()?;
    owner_check(root)?;
    // Looked at again: it may have gone, or become a link, while its owner was asked.
    if !found || !is_there()? {
        return Ok(false);
    }
    check_shape(&bolt)?;
    Ok(true)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormatInfo {
    pub features: BTreeSet<String>,
}

/// A name a later Bolt could have written: not empty, no whitespace or control characters.
fn is_word(s: &str) -> bool {
    !s.is_empty() && !s.chars().any(|c| c.is_whitespace() || c.is_control())
}

/// Parses `.bolt/format`. Anything a newer Bolt could have written is `NewerFormat`;
/// anything else that isn't exactly right is `Damaged`.
pub fn parse_format(text: &str) -> Result<FormatInfo, RepoError> {
    let damaged_format = || damaged("the format file is not in the expected form");
    let mut lines = text
        .strip_suffix('\n')
        .ok_or_else(damaged_format)?
        .split('\n');
    match lines
        .next()
        .and_then(|l| l.strip_prefix("bolt-repository "))
    {
        Some("1") => {}
        Some(v) if v.bytes().all(|b| b.is_ascii_digit()) && !v.is_empty() => {
            return Err(RepoError::NewerFormat {
                detail: format!("repository format \"bolt-repository {v}\""),
            });
        }
        _ => return Err(damaged_format()),
    }
    match lines.next().and_then(|l| l.strip_prefix("hash ")) {
        Some("blake3") => {}
        Some(h) if is_word(h) => {
            return Err(RepoError::NewerFormat {
                detail: format!("the hash \"{h}\""),
            });
        }
        _ => return Err(damaged_format()),
    }
    // This version knows no features yet, so any further line is a refusal.
    if let Some(line) = lines.next() {
        return Err(match line.strip_prefix("feature ") {
            Some(f) if is_word(f) => RepoError::NewerFormat {
                detail: format!("the feature \"{f}\""),
            },
            _ => damaged_format(),
        });
    }
    Ok(FormatInfo {
        features: BTreeSet::new(),
    })
}

fn parse_head(bytes: &[u8]) -> Result<Id, RepoError> {
    std::str::from_utf8(bytes)
        .ok()
        .and_then(|t| t.strip_suffix('\n'))
        .and_then(|h| Id::from_hex(h).filter(|id| id.to_hex() == h))
        .ok_or_else(|| damaged("state/head is not a lowercase fingerprint and a newline"))
}

/// `state/head` of the `.bolt` folder `bolt`, or `None` before the first commit.
fn read_head(bolt: &Path) -> Result<Option<Id>, RepoError> {
    match safe_fs::read_capped(&bolt.join("state").join("head"), safe_fs::MAX_HEAD_BYTES) {
        Ok(bytes) => parse_head(&bytes).map(Some),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(refused(e)),
    }
}

/// Proof that this process holds the change lock; dropping it releases the lock.
pub struct ChangeLock {
    _file: File,
}

pub struct Repository {
    root: PathBuf,
    bolt: PathBuf,
    tmp: PathBuf,
    packs: PathBuf,
    format: FormatInfo,
    store: Store,
    /// The head read before the store was last loaded, so every object it reaches is
    /// registered. `None` when there was no head yet.
    head: Option<Id>,
}

/// The `.bolt/format` file of the project whose top folder is `root`.
fn format_path(root: &Path) -> PathBuf {
    root.join(".bolt").join("format")
}

/// Whether `folder` holds a project, for the searches up from a folder. A refusal from
/// `owner_check` is the error. Nothing is followed, and nothing in `.bolt` is looked at before
/// `owner_check` allows it: a planted `.bolt` could point at another computer.
fn has_project(
    folder: &Path,
    owner_check: &dyn Fn(&Path) -> Result<(), RepoError>,
) -> Result<bool, RepoError> {
    let is_link = match fs::symlink_metadata(folder.join(".bolt")) {
        Ok(meta) if safe_fs::is_link(&meta) => true,
        Ok(meta) if meta.is_dir() => false,
        _ => return Ok(false),
    };
    owner_check(folder)?;
    // A link counts, so that `open` refuses it; a real `.bolt` needs a `format`.
    Ok(is_link || has_format(folder).unwrap_or(false))
}

/// Whether `root/.bolt` holds an entry named `format`, of any kind (`open` checks it), without
/// following a link there.
pub(crate) fn has_format(root: &Path) -> io::Result<bool> {
    match fs::symlink_metadata(format_path(root)) {
        Ok(_) => Ok(true),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e),
    }
}

/// The nearest folder above `root` holding a project Bolt may use. One it may not use is looked
/// past, so that a `.bolt` planted above the user's work can't stop them starting their own.
fn enclosing_project<'a>(
    root: &'a Path,
    owner_check: &dyn Fn(&Path) -> Result<(), RepoError>,
) -> Result<Option<&'a Path>, RepoError> {
    for outer in root.ancestors().skip(1) {
        match has_project(outer, owner_check) {
            Ok(true) => return Ok(Some(outer)),
            Ok(false) => {}
            // The user's own settings are broken: worth saying.
            Err(e @ RepoError::Config(_)) => return Err(e),
            Err(_) => {}
        }
    }
    Ok(None)
}

impl Repository {
    /// Makes `root` a new project: the folders, the first save on line `main`, then `.bolt/format`
    /// last. A `.bolt` without `format` is an interrupted start or import, which this finishes.
    pub fn init(root: &Path, who: &Person) -> Result<Repository, RepoError> {
        Repository::init_with(root, who, &check_owner)
    }

    /// `init`, with the owner check given (a stand-in in tests).
    fn init_with(
        root: &Path,
        who: &Person,
        owner_check: &dyn Fn(&Path) -> Result<(), RepoError>,
    ) -> Result<Repository, RepoError> {
        let (mut repo, lock) = Repository::begin_with(root, owner_check)?;
        let mut packs = repo.new_packs()?;
        let listing = packs.add_record(ObjectKind::FolderListing, &Listing::empty().encode())?;
        let save = Save {
            root: listing,
            parents: vec![],
            author: who.clone(),
            saver: who.clone(),
            message: "Start project".into(),
            change_id: ChangeId::random().map_err(io::Error::other)?,
            move_notes: vec![],
            signature: vec![],
            extra: BTreeMap::from([(EXTRA_LINE.to_owned(), b"main".to_vec())]),
        };
        let encoded = save.encode();
        // Refuse a save that would not read back (an over-long name, say) before storing it.
        Save::decode(&encoded, false)?;
        let save = packs.add_record(ObjectKind::Save, &encoded)?;
        let entry = DiaryEntry {
            parent: None,
            action: Action::Command,
            command: "start".into(),
            who: who.clone(),
            bolt_version: env!("CARGO_PKG_VERSION").into(),
            state: State {
                lines: BTreeMap::from([("main".to_owned(), save)]),
                tags: None,
                position: Position::OnLine("main".into()),
                shelves: vec![],
                server_lines: None,
            },
            safety_copies: vec![],
        };
        repo.commit_first(&lock, packs, &entry)?;
        Ok(repo)
    }

    /// Refuses to make `root` a project when it or its `.bolt` isn't the user's, when it is a
    /// project already, when another tool made its `.bolt`, or when it is inside another project.
    /// Makes nothing, so a command can ask before its own slow checks.
    pub fn check_new(root: &Path) -> Result<(), RepoError> {
        Repository::check_new_with(root, &check_owner).map(drop)
    }

    /// `check_new`, with the owner check given. `Ok(true)`: a `.bolt` that Bolt made is there
    /// already, left by a start or import that didn't finish.
    fn check_new_with(
        root: &Path,
        owner_check: &dyn Fn(&Path) -> Result<(), RepoError>,
    ) -> Result<bool, RepoError> {
        // Before anything is made: a folder that isn't the user's is refused at once.
        let found = check_bolt(root, owner_check)?;
        if found && has_format(root)? {
            return Err(RepoError::AlreadyAProject {
                root: root.to_owned(),
            });
        }
        if found && !holds_only_bolts_own(&root.join(".bolt"))? {
            return Err(RepoError::OtherToolsFolder {
                root: root.to_owned(),
            });
        }
        if let Some(outer) = enclosing_project(root, owner_check)? {
            return Err(RepoError::InsideAnotherProject {
                root: outer.to_owned(),
            });
        }
        Ok(found)
    }

    /// Starts making `root` a project, after `check_new`'s checks: the folders, and the change
    /// lock. Nothing is committed and there is no `format` yet, so `commit_first` must follow.
    pub fn begin(root: &Path) -> Result<(Repository, ChangeLock), RepoError> {
        Repository::begin_with(root, &check_owner)
    }

    /// `begin`, with the owner check given.
    fn begin_with(
        root: &Path,
        owner_check: &dyn Fn(&Path) -> Result<(), RepoError>,
    ) -> Result<(Repository, ChangeLock), RepoError> {
        let bolt = root.join(".bolt");
        let found = Repository::check_new_with(root, owner_check)?;
        if !found {
            fsio::create_dir_durable(&bolt)?;
            // Some drives give a new folder another owner, or none, and every later command
            // would refuse it.
            let made = check_bolt(root, owner_check).map_err(|e| match e {
                e @ (RepoError::NotYours { .. }
                | RepoError::AdminsNotElevated { .. }
                | RepoError::OwnerUnknown { .. }
                | RepoError::OwnerLookupFailed { .. }) => RepoError::StartStopped {
                    refusal: Box::new(e),
                },
                other => other,
            })?;
            if !made {
                return Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    "The .bolt folder disappeared while the project was being started.",
                )
                .into());
            }
        }
        // Not `.bolt` itself: one gone since its check must not be made again unchecked.
        for dir in fixed_folders(&bolt).iter().skip(1) {
            fsio::create_dir_durable(dir)?;
        }
        check_shape(&bolt)?;
        bolt_platform::hide_path(&bolt)?;
        let tmp = bolt.join("tmp");
        let packs = bolt.join("store").join("packs");
        let mut repo = Repository {
            root: root.to_owned(),
            store: Store::open(&packs)?,
            bolt,
            tmp,
            packs,
            format: FormatInfo {
                features: BTreeSet::new(),
            },
            head: None,
        };
        // `state/lock-info` is never shown before there is a `format`, so an import's says
        // `start` too.
        let lock = repo.lock_for_change("start", &mut |_| {})?;
        // Another start or import may have finished while this one waited for the lock.
        if has_format(root)? {
            return Err(RepoError::AlreadyAProject {
                root: root.to_owned(),
            });
        }
        Ok((repo, lock))
    }

    /// Commits the first entry of a project `begin` started, then writes `.bolt/format`, which
    /// makes it a project.
    pub fn commit_first(
        &mut self,
        lock: &ChangeLock,
        packs: PackSet,
        entry: &DiaryEntry,
    ) -> Result<Id, RepoError> {
        let id = self.commit(lock, packs, entry)?;
        write_atomic(&self.tmp, &format_path(&self.root), FORMAT_TEXT.as_bytes())?;
        Ok(id)
    }

    /// Opens the project that holds `start`: the nearest folder, from `start` up, that holds one.
    /// A `.bolt` on the way that Bolt may not use is refused.
    pub fn discover(start: &Path) -> Result<Repository, RepoError> {
        Repository::discover_with(start, &check_owner)
    }

    /// `discover`, with the search's owner check given (a stand-in in tests).
    fn discover_with(
        start: &Path,
        owner_check: &dyn Fn(&Path) -> Result<(), RepoError>,
    ) -> Result<Repository, RepoError> {
        for folder in start.ancestors() {
            if has_project(folder, owner_check)? {
                return Repository::open(folder);
            }
        }
        Err(RepoError::NotAProject)
    }

    /// Opens the project whose top folder is `root` (the folder holding `.bolt`).
    pub fn open(root: &Path) -> Result<Repository, RepoError> {
        let bolt = root.join(".bolt");
        if !check_bolt(root, &check_owner)? {
            return Err(RepoError::NotAProject);
        }

        let text = match safe_fs::read_capped(&bolt.join("format"), safe_fs::MAX_FORMAT_BYTES) {
            Ok(bytes) => {
                String::from_utf8(bytes).map_err(|_| damaged("the format file is not text"))?
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Err(RepoError::NotAProject),
            Err(e) => return Err(refused(e)),
        };
        let format = parse_format(&text)?;
        let tmp = bolt.join("tmp");
        let packs = bolt.join("store").join("packs");
        // Every command needs these, so a missing one is damage, named as such.
        for (dir, name) in [(&bolt.join("state"), "state"), (&packs, "store/packs")] {
            if let Err(e) = safe_fs::check_real_dir(dir)
                && e.kind() == io::ErrorKind::NotFound
            {
                return Err(damaged(format!(".bolt/{name} is missing")));
            }
        }
        // The head first: its pack was in the store before the head pointed at it.
        let head = read_head(&bolt)?;
        let store = Store::open(&packs)?;
        // These hold nothing worth keeping, so a missing one is made again. Only now, so that a
        // project found damaged is left as it was.
        for dir in [&tmp, &bolt.join("cache")] {
            if let Err(e) = safe_fs::check_real_dir(dir)
                && e.kind() == io::ErrorKind::NotFound
            {
                fsio::create_dir_durable(dir)?;
                safe_fs::check_real_dir(dir).map_err(refused)?;
            }
        }
        let repo = Repository {
            root: root.to_owned(),
            bolt,
            tmp,
            packs,
            format,
            store,
            head,
        };
        repo.clean_tmp()?;
        Ok(repo)
    }

    /// Deletes leftovers of interrupted commands. A file in `tmp/` is only a leftover when no
    /// command holds the change lock; otherwise it may be a pack being written right now.
    fn clean_tmp(&self) -> Result<(), RepoError> {
        let lock = self.open_lock_file()?;
        if lock.try_lock().is_err() {
            return Ok(());
        }
        for entry in fs::read_dir(&self.tmp)? {
            let path = entry?.path();
            // Only Bolt's own temporary files: never a link, a folder or a name it doesn't make.
            let ours = path.file_name().is_some_and(fsio::is_temp_name);
            if ours && fs::symlink_metadata(&path).is_ok_and(|m| m.is_file()) {
                fsio::remove_file(&path)?;
            }
        }
        Ok(())
    }

    fn open_lock_file(&self) -> Result<File, RepoError> {
        fsio::open_lock_file(&self.bolt.join("state").join("lock")).map_err(refused)
    }

    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    #[must_use]
    pub fn tmp_dir(&self) -> &Path {
        &self.tmp
    }

    #[must_use]
    pub fn packs_dir(&self) -> &Path {
        &self.packs
    }

    #[must_use]
    pub fn store(&self) -> &Store {
        &self.store
    }

    #[must_use]
    pub fn signatures_allowed(&self) -> bool {
        self.format.features.contains("signatures")
    }

    /// Build packs only while holding the `ChangeLock`: `open` cleans `tmp/` whenever the lock
    /// is free, which would delete a pack being built without it.
    pub fn new_packs(&self) -> Result<PackSet, RepoError> {
        Ok(PackSet::new(&self.tmp, &self.packs)?)
    }

    /// The newest diary entry's ID. Readers never take the lock: `state/head` is replaced whole.
    pub fn head(&self) -> Result<Id, RepoError> {
        let head = self.bolt.join("state").join("head");
        parse_head(&safe_fs::read_capped(&head, safe_fs::MAX_HEAD_BYTES).map_err(refused)?)
    }

    /// The newest diary entry this handle can see: the one when it was opened, or when it
    /// last took the change lock or committed. Read from the store.
    pub fn current(&self) -> Result<(Id, DiaryEntry), RepoError> {
        let id = match self.head {
            Some(id) => id,
            None => self.head()?,
        };
        let bytes = self.store.read_record(&id, ObjectKind::DiaryEntry)?;
        Ok((id, DiaryEntry::decode(&bytes)?))
    }

    /// The tags of `state`; empty if it has none.
    pub fn tag_list(&self, state: &State) -> Result<TagList, RepoError> {
        let Some(id) = state.tags else {
            return Ok(TagList::default());
        };
        Ok(TagList::decode(
            &self.store.read_record(&id, ObjectKind::TagList)?,
        )?)
    }

    /// The server lines of `state`; empty if it has none.
    pub fn server_lines(&self, state: &State) -> Result<ServerLines, RepoError> {
        let Some(id) = state.server_lines else {
            return Ok(ServerLines::default());
        };
        Ok(ServerLines::decode(
            &self.store.read_record(&id, ObjectKind::ServerLines)?,
        )?)
    }

    /// Takes the change lock, waiting if another command holds it. `on_wait` gets that command's
    /// lock info before waiting. `command` must already be redacted: it goes into `state/lock-info`
    /// as is.
    pub fn lock_for_change(
        &mut self,
        command: &str,
        on_wait: &mut dyn FnMut(&str),
    ) -> Result<ChangeLock, RepoError> {
        let file = self.open_lock_file()?;
        let state = self.bolt.join("state");
        match file.try_lock() {
            Ok(()) => {}
            Err(TryLockError::WouldBlock) => {
                // Best effort: the text is only for the message.
                let info =
                    safe_fs::read_capped(&state.join("lock-info"), safe_fs::MAX_LOCK_INFO_BYTES)
                        .ok()
                        .and_then(|bytes| String::from_utf8(bytes).ok())
                        .unwrap_or_default();
                on_wait(&info);
                file.lock()?;
            }
            Err(TryLockError::Error(e)) => return Err(e.into()),
        }
        let started = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_millis());
        let info = format!(
            "pid {}\ncommand {command}\nstarted {started}\n",
            std::process::id()
        );
        write_atomic(&self.tmp, &state.join("lock-info"), info.as_bytes())?;
        // Another command may have committed since this handle was opened.
        self.head = read_head(&self.bolt)?;
        self.store.refresh(&self.packs)?;
        Ok(ChangeLock { _file: file })
    }

    /// Stores `entry` in the last of `packs`, moves that pack into the store, registers all of
    /// them, then replaces `state/head`. The command has happened exactly when the head is
    /// replaced (decision 010), so an `Io` error from the final folder sync does not mean
    /// "nothing changed".
    pub fn commit(
        &mut self,
        _lock: &ChangeLock,
        mut packs: PackSet,
        entry: &DiaryEntry,
    ) -> Result<Id, RepoError> {
        let bytes = entry.encode();
        // Refuse an entry that would not read back before storing it.
        DiaryEntry::decode(&bytes)?;
        let id = packs.add_record(ObjectKind::DiaryEntry, &bytes)?;
        for path in packs.finish()? {
            self.store.register(&path)?;
        }
        fault_point("commit:before-head")?;
        let head = format!("{id}\n");
        write_atomic(
            &self.tmp,
            &self.bolt.join("state").join("head"),
            head.as_bytes(),
        )?;
        self.head = Some(id);
        Ok(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diary::{Action, Position, ServerLines, State, TagList};
    use crate::pack::{PackReader, PackSet};
    use crate::save::Person;
    use crate::test_support::{commit_records, sample_records};
    use std::collections::BTreeMap;
    use std::sync::mpsc;
    use std::thread;
    use std::time::{Duration, Instant};

    #[test]
    fn format_file() {
        assert!(parse_format(FORMAT_TEXT).unwrap().features.is_empty());
        for (text, needle) in [
            ("bolt-repository 2\nhash blake3\n", "bolt-repository 2"),
            ("bolt-repository 1\nhash sha256\n", "sha256"),
            ("bolt-repository 1\nhash sha3_256\n", "sha3_256"),
            (
                "bolt-repository 1\nhash blake3\nfeature large_files\n",
                "large_files",
            ),
            (
                "bolt-repository 1\nhash blake3\nfeature teleport\n",
                "teleport",
            ),
        ] {
            let err = parse_format(text).unwrap_err();
            assert!(
                matches!(err, RepoError::NewerFormat { .. }) && err.to_string().contains(needle),
                "{text:?}"
            );
        }
        for damaged in [
            "",
            "bolt-repository 1\n",
            "bolt-repository 1\r\nhash blake3\r\n",
            "bolt-repository 1\nhash blake3\r\n",
            "bolt-repository 1\nhash blake3\nfeature \n",
            "bolt-repository 1\nhash blake3\nfeature a b\n",
            "hello\n",
        ] {
            assert!(
                matches!(parse_format(damaged), Err(RepoError::Damaged { .. })),
                "{damaged:?}"
            );
        }
    }

    /// A project made by hand: the folder layout and the `format` file, nothing committed.
    fn project() -> (tempfile::TempDir, Repository) {
        let dir = tempfile::tempdir().unwrap();
        let bolt = dir.path().join(".bolt");
        for sub in ["store/packs", "state", "cache", "tmp"] {
            std::fs::create_dir_all(bolt.join(sub)).unwrap();
        }
        std::fs::write(bolt.join("format"), FORMAT_TEXT).unwrap();
        let repo = Repository::open(dir.path()).unwrap();
        (dir, repo)
    }

    #[test]
    fn a_missing_fixed_folder_is_named_as_damage() {
        for name in ["state", "store/packs", "store"] {
            let (dir, repo) = project();
            drop(repo);
            std::fs::remove_dir_all(dir.path().join(".bolt").join(name)).unwrap();
            let missing = if name == "store" { "store/packs" } else { name };
            match Repository::open(dir.path()) {
                Err(e @ RepoError::Damaged { .. }) => {
                    let want = format!("damaged: .bolt/{missing} is missing.");
                    assert!(e.to_string().ends_with(&want), "{e}");
                    assert!(e.hint().unwrap().contains("Nothing was changed"));
                }
                other => panic!("{name}: {:?}", other.map(|_| ())),
            }
        }
    }

    /// Neither folder holds anything worth keeping, so losing one is no damage.
    #[test]
    fn a_missing_tmp_or_cache_is_recreated() {
        for name in ["tmp", "cache"] {
            let (dir, repo) = project();
            drop(repo);
            let folder = dir.path().join(".bolt").join(name);
            std::fs::remove_dir_all(&folder).unwrap();
            let mut repo = Repository::open(dir.path()).unwrap();
            assert!(safe_fs::check_real_dir(&folder).is_ok(), "{name}");
            // The project works again: commands write through `tmp/`.
            let first = commit(&mut repo, &entry(1)).unwrap();
            assert_eq!(Repository::open(dir.path()).unwrap().head().unwrap(), first);
        }
    }

    /// The hint for damage says nothing was changed, so a missing `tmp` or `cache` isn't made
    /// before the project is found damaged.
    #[test]
    fn a_damaged_project_is_left_as_found() {
        for spoil in ["state/head", "store/packs/00.pack"] {
            let (dir, repo) = project();
            drop(repo);
            let bolt = dir.path().join(".bolt");
            std::fs::write(bolt.join(spoil), b"not what Bolt wrote").unwrap();
            for name in ["tmp", "cache"] {
                std::fs::remove_dir_all(bolt.join(name)).unwrap();
            }
            let opened = Repository::open(dir.path());
            assert!(
                matches!(opened, Err(RepoError::Damaged { .. })),
                "{spoil}: {:?}",
                opened.map(|_| ())
            );
            for name in ["tmp", "cache"] {
                assert!(!bolt.join(name).exists(), "{spoil}: {name} was made again");
            }
        }
    }

    fn entry(save: u8) -> DiaryEntry {
        DiaryEntry {
            parent: None,
            action: Action::Command,
            command: "save".into(),
            who: Person {
                name: "A".into(),
                email: "a@b".into(),
                time_ms: 0,
                tz_offset_min: 0,
            },
            bolt_version: "0.1.0".into(),
            state: State {
                lines: BTreeMap::from([("main".to_owned(), Id([save; 32]))]),
                tags: None,
                position: Position::OnLine("main".into()),
                shelves: vec![],
                server_lines: None,
            },
            safety_copies: vec![],
        }
    }

    fn commit(repo: &mut Repository, e: &DiaryEntry) -> Result<Id, RepoError> {
        let lock = repo.lock_for_change("save", &mut |_| {})?;
        let packs = repo.new_packs()?;
        repo.commit(&lock, packs, e)
    }

    #[test]
    fn init_then_discover_from_a_subfolder() {
        let dir = tempfile::tempdir().unwrap();
        let who = entry(0).who;
        let repo = Repository::init(dir.path(), &who).unwrap();
        let (_, first) = repo.current().unwrap();
        assert_eq!(first.command, "start");
        let save = repo
            .store()
            .save(&first.state.lines["main"], false)
            .unwrap();
        assert_eq!(
            (save.message.as_str(), save.line()),
            ("Start project", Some("main"))
        );
        assert_eq!(repo.store().listing(&save.root).unwrap(), Listing::empty());
        let sub = dir.path().join("a").join("b");
        std::fs::create_dir_all(&sub).unwrap();
        assert_eq!(Repository::discover(&sub).unwrap().root(), dir.path());
        let outside = tempfile::tempdir().unwrap();
        assert!(matches!(
            Repository::discover(outside.path()),
            Err(RepoError::NotAProject)
        ));
    }

    #[test]
    fn start_writes_neither_record() {
        let dir = tempfile::tempdir().unwrap();
        let repo = Repository::init(dir.path(), &entry(0).who).unwrap();
        let state = repo.current().unwrap().1.state;
        assert!(state.tags.is_none() && state.server_lines.is_none());
        assert!(repo.tag_list(&state).unwrap().tags.is_empty());
        assert!(repo.server_lines(&state).unwrap().servers.is_empty());
        for pack in std::fs::read_dir(repo.packs_dir()).unwrap() {
            for e in PackReader::open(&pack.unwrap().path()).unwrap().entries() {
                assert!(!matches!(
                    e.kind,
                    ObjectKind::TagList | ObjectKind::ServerLines
                ));
            }
        }
    }

    #[test]
    fn the_records_a_state_points_to_are_read_back() {
        let dir = tempfile::tempdir().unwrap();
        let mut repo = Repository::init(dir.path(), &entry(0).who).unwrap();
        let (tags, servers) = sample_records();
        let (t, r) = commit_records(&mut repo, &tags, &servers);
        let reopened = Repository::open(dir.path()).unwrap();
        for repo in [&repo, &reopened] {
            let state = repo.current().unwrap().1.state;
            assert_eq!((state.tags, state.server_lines), (Some(t), Some(r)));
            assert_eq!(repo.tag_list(&state).unwrap(), tags);
            assert_eq!(repo.server_lines(&state).unwrap(), servers);
        }
    }

    #[test]
    fn a_pointer_that_leads_nowhere_good_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let mut repo = Repository::init(dir.path(), &entry(0).who).unwrap();
        let (tags, servers) = sample_records();
        let (t, r) = commit_records(&mut repo, &tags, &servers);
        let mut state = repo.current().unwrap().1.state;
        // Each pointer at the other record.
        (state.tags, state.server_lines) = (Some(r), Some(t));
        assert!(matches!(
            repo.tag_list(&state),
            Err(RepoError::Store(StoreError::WrongKind { .. }))
        ));
        assert!(matches!(
            repo.server_lines(&state),
            Err(RepoError::Store(StoreError::WrongKind { .. }))
        ));
        // A record that was never stored.
        (state.tags, state.server_lines) = (Some(Id([9; 32])), Some(Id([9; 32])));
        assert!(matches!(
            repo.tag_list(&state),
            Err(RepoError::Store(StoreError::Missing(_)))
        ));
        assert!(matches!(
            repo.server_lines(&state),
            Err(RepoError::Store(StoreError::Missing(_)))
        ));
        // Records with the right fingerprints that break the format: neither may be empty.
        commit_records(&mut repo, &TagList::default(), &ServerLines::default());
        let state = repo.current().unwrap().1.state;
        assert!(matches!(repo.tag_list(&state), Err(RepoError::Decode(_))));
        assert!(matches!(
            repo.server_lines(&state),
            Err(RepoError::Decode(_))
        ));
    }

    #[test]
    fn head_round_trips() {
        let (dir, mut repo) = project();
        let a = commit(&mut repo, &entry(1)).unwrap();
        assert_eq!(repo.head().unwrap(), a);
        assert_eq!(repo.current().unwrap(), (a, entry(1)));
        let repo = Repository::open(dir.path()).unwrap();
        assert_eq!(repo.current().unwrap(), (a, entry(1)));
        let text = std::fs::read_to_string(repo.root().join(".bolt/state/head")).unwrap();
        assert_eq!(text, format!("{a}\n"));
    }

    #[test]
    fn commit_refuses_an_invalid_entry() {
        let (_dir, mut repo) = project();
        let a = commit(&mut repo, &entry(1)).unwrap();
        let mut bad = entry(2);
        bad.state.position = Position::OnLine("dev".into());
        assert!(matches!(commit(&mut repo, &bad), Err(RepoError::Decode(_))));
        assert_eq!(repo.head().unwrap(), a);
    }

    #[test]
    fn a_reader_opened_before_a_commit_still_reads() {
        let (dir, mut repo) = project();
        let a = commit(&mut repo, &entry(1)).unwrap();
        let reader = Repository::open(dir.path()).unwrap();
        commit(&mut repo, &entry(2)).unwrap();
        assert_eq!(reader.current().unwrap(), (a, entry(1)));
    }

    #[test]
    fn a_damaged_pack_is_reported_as_damage() {
        let id = Id([7; 32]);
        for e in [
            StoreError::Pack(PackError::FingerprintMismatch { id }),
            StoreError::Pack(PackError::Corrupt {
                path: "p.pack".into(),
                reason: "bad index".into(),
            }),
        ] {
            let e = RepoError::from(e);
            assert!(matches!(e, RepoError::Damaged { .. }), "{e:?}");
            assert!(e.hint().unwrap().contains("Nothing was changed"));
        }
        let e = crate::snapshot::SaveError::from(PackError::FingerprintMismatch { id });
        assert!(e.hint().unwrap().contains("Nothing was changed"), "{e:?}");
    }

    #[test]
    fn running_out_of_memory_is_not_reported_as_damage() {
        let e = RepoError::from(StoreError::Pack(PackError::OutOfMemory));
        assert!(!matches!(e, RepoError::Damaged { .. }), "{e:?}");
        assert!(e.to_string().contains("ran out of memory"), "{e}");
        assert!(e.hint().is_none(), "{e:?}");
    }

    #[test]
    fn a_save_past_the_limits_is_reported_as_damage() {
        let e = RepoError::from(StoreError::SaveTooLarge { detail: "x".into() });
        assert!(matches!(e, RepoError::Damaged { .. }), "{e:?}");
        assert!(e.to_string().contains("too large to read safely: x"), "{e}");
        assert!(e.hint().unwrap().contains("Nothing was changed"));
    }

    #[test]
    fn head_rejects_garbage() {
        let (dir, repo) = project();
        let head = dir.path().join(".bolt/state/head");
        let upper = format!("{}\n", Id([0xab; 32]).to_hex().to_uppercase());
        let no_newline = Id([0xab; 32]).to_hex();
        for bad in ["xyz\n", upper.as_str(), no_newline.as_str(), ""] {
            std::fs::write(&head, bad).unwrap();
            assert!(
                matches!(repo.head(), Err(RepoError::Damaged { .. })),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn open_needs_a_format_file_and_cleans_tmp() {
        let dir = tempfile::tempdir().unwrap();
        assert!(matches!(
            Repository::open(dir.path()),
            Err(RepoError::NotAProject)
        ));
        let (dir, repo) = project();
        drop(repo);
        let litter = dir.path().join(".bolt/tmp/tmp-0123456789abcdef");
        std::fs::write(&litter, b"half a pack").unwrap();
        let repo = Repository::open(dir.path()).unwrap();
        assert!(!litter.exists());
        assert_eq!(repo.tmp_dir(), dir.path().join(".bolt/tmp"));
        assert_eq!(repo.packs_dir(), dir.path().join(".bolt/store/packs"));
        assert!(!repo.signatures_allowed());
    }

    #[test]
    fn open_refuses_a_linked_fixed_folder() {
        let who = entry(0).who;
        let folders = [
            ".bolt",
            ".bolt/store",
            ".bolt/store/packs",
            ".bolt/state",
            ".bolt/cache",
            ".bolt/tmp",
        ];
        for folder in folders {
            let dir = tempfile::tempdir().unwrap();
            Repository::init(dir.path(), &who).unwrap();
            let link = folder
                .split('/')
                .fold(dir.path().to_owned(), |path, name| path.join(name));
            // The real folder moves outside, so the project would still work through the link.
            let outside = tempfile::tempdir().unwrap();
            let target = outside.path().join("moved");
            std::fs::rename(&link, &target).unwrap();
            let bait = target.join("tmp-0123456789abcdef");
            std::fs::write(&bait, b"someone else's file").unwrap();
            #[cfg(unix)]
            std::os::unix::fs::symlink(&target, &link).unwrap();
            #[cfg(windows)]
            crate::test_support::junction(&link, &target);
            match Repository::open(dir.path()) {
                Err(RepoError::Damaged { detail }) => assert!(
                    detail.starts_with(&link.display().to_string()),
                    "{folder}: {detail}"
                ),
                other => panic!("{folder}: {:?}", other.map(|_| ())),
            }
            assert!(
                bait.exists(),
                "{folder}: a file in the link's target was deleted"
            );
        }
    }

    #[test]
    fn clean_tmp_removes_only_bolts_own_names() {
        let (dir, _repo) = project();
        let tmp = dir.path().join(".bolt").join("tmp");
        let (_, made) = fsio::create_temp(&tmp).unwrap();
        for name in [
            "tmp-0123456789abcdef",
            "notes.txt",
            "tmp-xyz",
            "tmp-ABCDEF0123456789",
        ] {
            std::fs::write(tmp.join(name), b"x").unwrap();
        }
        std::fs::create_dir(tmp.join("tmp-1111111111111111")).unwrap();
        Repository::open(dir.path()).unwrap();
        assert!(!made.exists(), "a temp file Bolt made was kept");
        let mut left: Vec<String> = std::fs::read_dir(&tmp)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        left.sort();
        assert_eq!(
            left,
            [
                "notes.txt",
                "tmp-1111111111111111",
                "tmp-ABCDEF0123456789",
                "tmp-xyz"
            ]
        );
    }

    #[test]
    fn owner_rule() {
        let dir = tempfile::tempdir().unwrap();
        let (root, sibling) = (dir.path().join("project"), dir.path().join("sibling"));
        for folder in [&root, &sibling] {
            std::fs::create_dir(folder).unwrap();
        }
        // Who owns the project folder, then its `.bolt`.
        let allows =
            |owners: [io::Result<Owner>; 2], safe: &[PathBuf]| owner_allows(owners, &root, safe);
        let failed = || Err(io::Error::other("Access is denied."));
        assert!(allows([Ok(Owner::User), Ok(Owner::User)], &[]).is_ok());
        // A `.bolt` of the user's in a folder that isn't theirs is refused too.
        match allows([Ok(Owner::Other), Ok(Owner::User)], &[]) {
            Err(RepoError::NotYours { path }) => assert_eq!(path, root),
            other => panic!("{other:?}"),
        }
        match allows([Ok(Owner::User), Ok(Owner::Unknown)], &[]) {
            Err(RepoError::OwnerUnknown { path }) => assert_eq!(path, root),
            other => panic!("{other:?}"),
        }
        match allows([Ok(Owner::User), failed()], &[]) {
            Err(RepoError::OwnerLookupFailed { path, source }) => {
                assert_eq!(
                    (path, source.to_string()),
                    (root.clone(), "Access is denied.".into())
                );
            }
            other => panic!("{other:?}"),
        }
        let refused = || {
            [
                [Ok(Owner::Other), Ok(Owner::User)],
                [Ok(Owner::User), Ok(Owner::Other)],
                [Ok(Owner::Unknown), Ok(Owner::User)],
                [Ok(Owner::User), failed()],
            ]
        };
        // Listing the project folder trusts both, as Git's safe.directory does, however it is
        // written; a folder above it or beside it trusts nothing.
        for owners in refused() {
            assert!(allows(owners, std::slice::from_ref(&root)).is_ok());
        }
        for owners in refused() {
            assert!(allows(owners, &[sibling.join("..").join("project")]).is_ok());
        }
        for owners in refused() {
            assert!(allows(owners, &[dir.path().to_owned(), sibling.clone()]).is_err());
        }
    }

    /// A refusal that needs the user's `safe-folders` is replaced by the mistake in it, with what
    /// to do about that.
    #[test]
    fn a_misplaced_safe_folders_line_is_what_the_user_is_told() {
        let dir = tempfile::tempdir().unwrap();
        let misplaced = || -> Result<Vec<PathBuf>, ConfigError> {
            Err(ConfigError::SafeFoldersNotAtTop {
                path: PathBuf::from("config.toml"),
            })
        };
        let told = check_owner_with(dir.path(), &|_| Ok(Owner::Other), &misplaced).unwrap_err();
        assert_eq!(
            told.to_string(),
            "safe-folders must be at the top of config.toml, before any [section]"
        );
        assert!(told.hint().unwrap().starts_with("Move that line above"));
    }

    #[test]
    fn administrators_count_only_when_elevated() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let elevated = Owner::Administrators { elevated: true };
        let normal = Owner::Administrators { elevated: false };
        assert!(owner_allows([Ok(elevated)], root, &[]).is_ok());
        assert!(owner_allows([Ok(elevated), Ok(Owner::User)], root, &[]).is_ok());
        assert!(matches!(
            owner_allows([Ok(normal)], root, &[]),
            Err(RepoError::AdminsNotElevated { .. })
        ));
        // The folder and its `.bolt` are each judged, and the refusal names the project folder.
        match owner_allows([Ok(Owner::User), Ok(normal)], root, &[]) {
            Err(RepoError::AdminsNotElevated { path }) => assert_eq!(path, root),
            other => panic!("{other:?}"),
        }
        let mixed = owner_allows([Ok(elevated), Ok(Owner::Other)], root, &[]);
        assert!(
            matches!(mixed, Err(RepoError::NotYours { .. })),
            "{mixed:?}"
        );
        // Listing the project folder trusts it, as for every other refusal.
        assert!(owner_allows([Ok(normal)], root, &[root.to_owned()]).is_ok());
    }

    /// The user's own project opens even when their settings can't be read; only a folder that
    /// needs safe-folders has to read them.
    #[test]
    fn an_elevated_administrators_project_needs_no_settings() {
        let dir = tempfile::tempdir().unwrap();
        let broken = || -> Result<Vec<PathBuf>, ConfigError> {
            Err(ConfigError::Invalid {
                path: PathBuf::from("config.toml"),
                detail: "broken".into(),
            })
        };
        let owned_by = |owner: Owner| check_owner_with(dir.path(), &|_| Ok(owner), &broken);
        assert!(owned_by(Owner::User).is_ok());
        assert!(owned_by(Owner::Administrators { elevated: true }).is_ok());
        let normal = owned_by(Owner::Administrators { elevated: false });
        assert!(matches!(normal, Err(RepoError::Config(_))), "{normal:?}");
    }

    #[test]
    fn start_notes_a_project_that_opens_only_elevated() {
        let root = Path::new("/srv/work");
        let elevated = Owner::Administrators { elevated: true };
        let note = elevated_only_note_with(root, &|_| Ok(elevated)).unwrap();
        assert!(note.contains("administrator terminal"), "{note}");
        // The way to open it from any terminal is the same line every refusal gives.
        assert!(note.ends_with(&config::trust_hint(root)), "{note}");
        // Either folder is enough.
        let only_bolt = |path: &Path| {
            Ok(if path.ends_with(".bolt") {
                elevated
            } else {
                Owner::User
            })
        };
        assert!(elevated_only_note_with(root, &only_bolt).is_some());
        // The user's own account, a group that isn't elevated, and a failed look-up need no note.
        for owner in [
            Owner::User,
            Owner::Other,
            Owner::Administrators { elevated: false },
        ] {
            assert!(elevated_only_note_with(root, &|_| Ok(owner)).is_none());
        }
        let failed = elevated_only_note_with(root, &|_| Err(io::Error::other("Access is denied.")));
        assert!(failed.is_none());
    }

    /// Names that only look alike are different folders on NTFS, side by side: trusting one
    /// never trusts the other. (A KELVIN SIGN lowercases to `k`, and an ANGSTROM SIGN to `å`.)
    #[cfg(windows)]
    #[test]
    fn look_alike_folders_are_different_folders() {
        let dir = tempfile::tempdir().unwrap();
        for (a, b) in [("Kit", "\u{212A}it"), ("\u{C5}ngstrom", "\u{212B}ngstrom")] {
            let (a, b) = (dir.path().join(a), dir.path().join(b));
            for folder in [&a, &b] {
                std::fs::create_dir(folder).unwrap();
            }
            assert!(!config::same_folder(&a, &b), "{a:?} {b:?}");
            let trusts = |root: &Path, listed: &Path| {
                owner_allows([Ok(Owner::Other)], root, &[listed.to_owned()]).is_ok()
            };
            assert!(!trusts(&a, &b), "{a:?}");
            assert!(!trusts(&b, &a), "{b:?}");
            assert!(trusts(&a, &a), "{a:?}");
        }
    }

    /// The real owner check, with the answers no test machine can give: `owner(path)` owns each
    /// path that exists, and the user's safe-folders are `safe`.
    fn checked_by(
        owner: impl Fn(&Path) -> Owner,
        safe: &[PathBuf],
    ) -> impl Fn(&Path) -> Result<(), RepoError> {
        move |root| {
            let owner_of = |path: &Path| std::fs::symlink_metadata(path).map(|_| owner(path));
            check_owner_with(root, &owner_of, &|| Ok(safe.to_vec()))
        }
    }

    fn start_where(
        root: &Path,
        owner: impl Fn(&Path) -> Owner,
        safe: &[PathBuf],
    ) -> Result<Repository, RepoError> {
        Repository::init_with(root, &entry(0).who, &checked_by(owner, safe))
    }

    /// The owner check where `owner` owns everything, and nothing is listed.
    fn all_owned_by(owner: Owner) -> impl Fn(&Path) -> Result<(), RepoError> {
        checked_by(move |_| owner, &[])
    }

    /// Below a project it may not use, `bolt start` looks further up, and makes the project if
    /// none above is one the user may use.
    #[test]
    fn start_looks_past_an_enclosing_project_it_may_not_use() {
        for drive_says in [Owner::Other, Owner::Unknown] {
            let dir = tempfile::tempdir().unwrap();
            let top = dir.path().join("top");
            let mid = top.join("mid");
            let root = mid.join("root");
            std::fs::create_dir_all(&root).unwrap();
            for outer in [&top, &mid] {
                std::fs::create_dir(outer.join(".bolt")).unwrap();
                std::fs::write(format_path(outer), FORMAT_TEXT).unwrap();
            }
            let owner = |path: &Path| {
                if path.starts_with(&root) {
                    Owner::User
                } else {
                    drive_says
                }
            };
            let inside = |safe: &[PathBuf]| match start_where(&root, owner, safe) {
                Err(RepoError::InsideAnotherProject { root }) => root,
                other => panic!("{drive_says:?}: {:?}", other.map(|_| ())),
            };
            // The nearest one the user trusts stops the search; one they don't is looked past.
            assert_eq!(inside(&[top.clone(), mid.clone()]), mid);
            assert_eq!(inside(std::slice::from_ref(&top)), top);
            assert!(!root.join(".bolt").exists(), "{drive_says:?}");
            // With none trusted, the project is made.
            let repo = start_where(&root, owner, &[]).unwrap();
            assert_eq!(repo.root(), root);
            assert!(format_path(&root).is_file(), "{drive_says:?}");
        }
    }

    /// Some drives give a new folder another owner, or none. `bolt start` checks the `.bolt` it
    /// made, stops before making anything in it, and finishes when run again once it is trusted.
    #[test]
    fn start_stops_when_it_may_not_use_the_bolt_folder_it_made() {
        for drive_says in [
            Owner::Unknown,
            Owner::Other,
            Owner::Administrators { elevated: false },
        ] {
            let dir = tempfile::tempdir().unwrap();
            let root = dir.path();
            let owner = |path: &Path| {
                if path == root {
                    Owner::User
                } else {
                    drive_says
                }
            };
            let refusal = match start_where(root, owner, &[]) {
                Err(RepoError::StartStopped { refusal }) => *refusal,
                other => panic!("{drive_says:?}: {:?}", other.map(|_| ())),
            };
            match (drive_says, refusal) {
                (Owner::Unknown, RepoError::OwnerUnknown { path })
                | (Owner::Other, RepoError::NotYours { path })
                | (
                    Owner::Administrators { elevated: false },
                    RepoError::AdminsNotElevated { path },
                ) => assert_eq!(path, root),
                other => panic!("{other:?}"),
            }
            // Nothing deleted, nothing made inside.
            assert!(root.join(".bolt").is_dir(), "{drive_says:?}");
            let inside = std::fs::read_dir(root.join(".bolt")).unwrap().count();
            assert_eq!(inside, 0, "{drive_says:?}");
            let again = start_where(root, owner, &[]);
            assert!(
                matches!(
                    again,
                    Err(RepoError::OwnerUnknown { .. }
                        | RepoError::NotYours { .. }
                        | RepoError::AdminsNotElevated { .. })
                ),
                "{drive_says:?}"
            );
            let repo = start_where(root, owner, &[root.to_owned()]).unwrap();
            assert_eq!(repo.current().unwrap().1.command, "start");
            assert!(Repository::open(root).is_ok(), "{drive_says:?}");
        }
    }

    #[test]
    fn what_counts_as_a_project_in_the_search() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path();
        let bolt = folder.join(".bolt");
        let (mine, theirs) = (all_owned_by(Owner::User), all_owned_by(Owner::Other));
        assert!(!has_project(folder, &theirs).unwrap(), "nothing there");
        std::fs::write(&bolt, b"").unwrap();
        assert!(!has_project(folder, &theirs).unwrap(), "a file named .bolt");
        std::fs::remove_file(&bolt).unwrap();
        std::fs::create_dir(&bolt).unwrap();
        // The user's own interrupted start (no `format` yet) is no project.
        assert!(!has_project(folder, &mine).unwrap(), "an interrupted start");
        // Another account's is found, empty or not, and refused.
        let found = has_project(folder, &theirs);
        assert!(
            matches!(found, Err(RepoError::NotYours { .. })),
            "{found:?}"
        );
        // An entry named `format` of any kind counts: `open` checks it.
        std::fs::create_dir(bolt.join("format")).unwrap();
        assert!(has_project(folder, &mine).unwrap(), "a folder named format");
    }

    /// Its owner is asked before anything in it is looked at, so what is inside can't matter.
    #[test]
    fn commands_refuse_a_foreign_bolt_above_even_an_empty_one() {
        for drive_says in [Owner::Other, Owner::Unknown] {
            for link in [false, true] {
                let dir = tempfile::tempdir().unwrap();
                let outer = dir.path().join("outer");
                let root = outer.join("root");
                std::fs::create_dir_all(&root).unwrap();
                let bolt = outer.join(".bolt");
                if link {
                    let elsewhere = dir.path().join("elsewhere");
                    std::fs::create_dir(&elsewhere).unwrap();
                    #[cfg(unix)]
                    std::os::unix::fs::symlink(&elsewhere, &bolt).unwrap();
                    #[cfg(windows)]
                    crate::test_support::junction(&bolt, &elsewhere);
                } else {
                    std::fs::create_dir(&bolt).unwrap();
                }
                match (
                    drive_says,
                    Repository::discover_with(&root, &all_owned_by(drive_says)),
                ) {
                    (Owner::Other, Err(RepoError::NotYours { path }))
                    | (Owner::Unknown, Err(RepoError::OwnerUnknown { path })) => {
                        assert_eq!(path, outer);
                    }
                    (_, other) => panic!("{drive_says:?}, {link}: {:?}", other.map(|_| ())),
                }
            }
        }
    }

    /// A link inside would be damage, but a `.bolt` the user may not use is refused before it is
    /// looked at.
    #[test]
    fn the_owner_of_bolt_is_asked_before_anything_inside_it() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("root");
        let elsewhere = dir.path().join("elsewhere");
        std::fs::create_dir_all(root.join(".bolt")).unwrap();
        std::fs::create_dir(&elsewhere).unwrap();
        let store = root.join(".bolt").join("store");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&elsewhere, &store).unwrap();
        #[cfg(windows)]
        crate::test_support::junction(&store, &elsewhere);
        let other = check_bolt(&root, &all_owned_by(Owner::Other));
        assert!(
            matches!(other, Err(RepoError::NotYours { .. })),
            "{other:?}"
        );
        let unknown = check_bolt(&root, &all_owned_by(Owner::Unknown));
        assert!(
            matches!(unknown, Err(RepoError::OwnerUnknown { .. })),
            "{unknown:?}"
        );
        let mine = check_bolt(&root, &all_owned_by(Owner::User));
        assert!(matches!(mine, Err(RepoError::Damaged { .. })), "{mine:?}");
    }

    /// Any signed-in Windows user can rename a folder the user made into `C:\.bolt`. `C:\`
    /// belongs to the system, so that project is refused whoever owns its `.bolt`, as Git does.
    #[test]
    fn a_bolt_of_the_users_in_a_folder_that_isnt_theirs_is_refused() {
        for drive_says in [Owner::Other, Owner::Unknown] {
            let dir = tempfile::tempdir().unwrap();
            let system = dir.path().join("system");
            let work = system.join("work");
            std::fs::create_dir_all(&work).unwrap();
            Repository::init(&system, &entry(0).who).unwrap();
            // The folder isn't the user's; its `.bolt`, and everything in `work`, is.
            let owner = |path: &Path| {
                if path == system {
                    drive_says
                } else if path.starts_with(&system) {
                    Owner::User
                } else {
                    drive_says
                }
            };
            let check = checked_by(owner, &[]);
            let refused = |result: Result<(), RepoError>| match (drive_says, result) {
                (Owner::Other, Err(RepoError::NotYours { path }))
                | (Owner::Unknown, Err(RepoError::OwnerUnknown { path })) => path == system,
                _ => false,
            };
            let found = Repository::discover_with(&work, &check).map(|_| ());
            assert!(refused(found), "{drive_says:?}");
            assert!(
                refused(check_bolt(&system, &check).map(|_| ())),
                "{drive_says:?}"
            );
            // `bolt start` below it looks past it.
            let started = Repository::init_with(&work, &entry(0).who, &check).unwrap();
            assert_eq!(started.root(), work);
            // Listed in safe-folders, it is used.
            let listed = checked_by(owner, std::slice::from_ref(&system));
            let opened = Repository::discover_with(&system, &listed).unwrap();
            assert_eq!(opened.root(), system);
        }
    }

    #[test]
    fn start_refuses_a_folder_that_isnt_the_users_before_making_anything() {
        for drive_says in [Owner::Other, Owner::Unknown] {
            let dir = tempfile::tempdir().unwrap();
            let root = dir.path().join("root");
            std::fs::create_dir(&root).unwrap();
            let owner = |path: &Path| {
                if path == root {
                    drive_says
                } else if path.starts_with(&root) {
                    Owner::User
                } else {
                    drive_says
                }
            };
            let started = start_where(&root, owner, &[]);
            match (drive_says, &started) {
                (Owner::Other, Err(RepoError::NotYours { path }))
                | (Owner::Unknown, Err(RepoError::OwnerUnknown { path })) => {
                    assert_eq!(path, &root);
                }
                (_, other) => panic!("{drive_says:?}: {:?}", other.as_ref().map(|_| ())),
            }
            // The same way on as for a project that is already there.
            let hint = started.err().and_then(|e| e.hint());
            assert_eq!(hint, Some(config::trust_hint(&root)), "{drive_says:?}");
            assert!(!root.join(".bolt").exists(), "{drive_says:?}");
            start_where(&root, owner, std::slice::from_ref(&root)).unwrap();
        }
    }

    /// Fails closed rather than make the project in a `.bolt` nobody checked.
    #[test]
    fn start_stops_if_the_bolt_it_made_vanishes() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("root");
        std::fs::create_dir(&root).unwrap();
        let bolt = root.join(".bolt");
        let rule = checked_by(
            |path| {
                if path.starts_with(&root) {
                    Owner::User
                } else {
                    Owner::Other
                }
            },
            &[],
        );
        let check = |folder: &Path| {
            // Someone removes the new `.bolt` while its owner is asked.
            if folder == root && bolt.is_dir() {
                std::fs::remove_dir(&bolt).unwrap();
            }
            rule(folder)
        };
        let started = Repository::init_with(&root, &entry(0).who, &check);
        assert!(started.is_err(), "{:?}", started.map(|_| ()));
        assert!(!bolt.exists() && !format_path(&root).exists());
    }

    /// A link that appears where `bolt start` is about to make `.bolt` is never followed.
    #[test]
    fn start_refuses_a_bolt_link_that_appears_while_it_starts() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("root");
        std::fs::create_dir(&root).unwrap();
        let (bolt, target) = (root.join(".bolt"), dir.path().join("target"));
        let rule = checked_by(
            |path| {
                if path.starts_with(&root) {
                    Owner::User
                } else {
                    Owner::Other
                }
            },
            &[],
        );
        let check = |folder: &Path| {
            if folder == root && std::fs::symlink_metadata(&bolt).is_err() {
                #[cfg(unix)]
                std::os::unix::fs::symlink(&target, &bolt).unwrap();
                #[cfg(windows)]
                crate::test_support::junction(&bolt, &target);
            }
            rule(folder)
        };
        let started = Repository::init_with(&root, &entry(0).who, &check);
        assert!(
            matches!(started, Err(RepoError::Damaged { .. })),
            "{:?}",
            started.map(|_| ())
        );
        assert!(!target.exists());
    }

    /// With the inside unreadable, any look inside a `.bolt` the user may not use would fail and
    /// change the answer.
    #[cfg(unix)]
    #[test]
    fn a_foreign_bolt_is_never_looked_inside() {
        use std::cell::RefCell;
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let outer = dir.path().join("outer");
        let root = outer.join("root");
        std::fs::create_dir_all(&root).unwrap();
        let bolt = outer.join(".bolt");
        std::fs::create_dir_all(bolt.join("store")).unwrap();
        std::fs::write(bolt.join("format"), FORMAT_TEXT).unwrap();
        let set_mode = |mode| {
            std::fs::set_permissions(&bolt, std::fs::Permissions::from_mode(mode)).unwrap();
        };
        set_mode(0o000);
        if std::fs::read_dir(&bolt).is_ok() {
            set_mode(0o755);
            eprintln!("note: skipped, this user can look inside any folder (root)");
            return;
        }
        let asked = RefCell::new(Vec::new());
        let rule = checked_by(
            |path| {
                if path.starts_with(&root) {
                    Owner::User
                } else {
                    Owner::Other
                }
            },
            &[],
        );
        let theirs = |folder: &Path| {
            asked.borrow_mut().push(folder.to_owned());
            rule(folder)
        };
        let found = Repository::discover_with(&root, &theirs).map(|_| ());
        let opened = check_bolt(&outer, &theirs).map(|_| ());
        let started = Repository::init_with(&root, &entry(0).who, &theirs).map(|_| ());
        set_mode(0o755);
        let refused = |result: &Result<_, RepoError>| matches!(result, Err(RepoError::NotYours { path }) if *path == outer);
        assert!(refused(&found), "{found:?}");
        assert!(refused(&opened), "{opened:?}");
        assert!(started.is_ok(), "{started:?}");
        assert!(asked.borrow().contains(&outer), "{:?}", asked.borrow());
    }

    /// A shared project can bring a `.bolt/format` link; following it could reach another
    /// computer.
    #[test]
    fn start_never_follows_a_format_link() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir(root.join(".bolt")).unwrap();
        // A link to nothing: followed, it would look like no `format` at all.
        let nothing = root.join("missing");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&nothing, format_path(root)).unwrap();
        #[cfg(windows)]
        crate::test_support::junction(&format_path(root), &nothing);
        let started = Repository::init(root, &entry(0).who);
        assert!(
            matches!(started, Err(RepoError::AlreadyAProject { .. })),
            "{:?}",
            started.map(|_| ())
        );
        let opened = Repository::open(root);
        assert!(
            matches!(opened, Err(RepoError::Damaged { .. })),
            "{:?}",
            opened.map(|_| ())
        );
    }

    /// A planted `.bolt` link could point at another computer, which Windows would contact.
    #[test]
    fn a_bolt_link_above_is_found_but_never_followed() {
        for with_format in [true, false] {
            let dir = tempfile::tempdir().unwrap();
            let target = dir.path().join("target");
            std::fs::create_dir(&target).unwrap();
            if with_format {
                std::fs::write(target.join("format"), FORMAT_TEXT).unwrap();
            }
            let outer = dir.path().join("outer");
            let root = outer.join("root");
            std::fs::create_dir_all(&root).unwrap();
            let link = outer.join(".bolt");
            #[cfg(unix)]
            std::os::unix::fs::symlink(&target, &link).unwrap();
            #[cfg(windows)]
            crate::test_support::junction(&link, &target);

            match Repository::discover(&root) {
                Err(RepoError::Damaged { detail }) => assert!(
                    detail.starts_with(&link.display().to_string()),
                    "{with_format}: {detail}"
                ),
                other => panic!("{with_format}: {:?}", other.map(|_| ())),
            }
            // The user's own link stops `bolt start` like their own project would; another
            // account's is looked past, and the new project is then found first.
            match Repository::init(&root, &entry(0).who) {
                Err(RepoError::InsideAnotherProject { root }) => assert_eq!(root, outer),
                other => panic!("{with_format}: {:?}", other.map(|_| ())),
            }
            let theirs = |path: &Path| {
                if path.starts_with(&root) {
                    Owner::User
                } else {
                    Owner::Other
                }
            };
            start_where(&root, theirs, &[]).unwrap();
            assert_eq!(Repository::discover(&root).unwrap().root(), root);
            let behind = std::fs::read_dir(&target).unwrap().count();
            assert_eq!(behind, usize::from(with_format));
        }
    }

    /// Reading through the link would never end.
    #[cfg(unix)]
    #[test]
    fn a_format_link_to_dev_zero_is_refused_without_hanging() {
        let dir = tempfile::tempdir().unwrap();
        let outer = dir.path().to_owned();
        std::fs::create_dir(outer.join(".bolt")).unwrap();
        std::os::unix::fs::symlink("/dev/zero", format_path(&outer)).unwrap();
        let root = outer.join("root");
        std::fs::create_dir(&root).unwrap();
        // A hang fails this test instead of stopping the whole run.
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let found = Repository::discover(&root).map(|_| ());
            let opened = Repository::open(&outer).map(|_| ());
            let _ = tx.send((found, opened));
        });
        let (found, opened) = rx
            .recv_timeout(Duration::from_secs(10))
            .expect("the search hung");
        for result in [found, opened] {
            assert!(
                matches!(&result, Err(RepoError::Damaged { detail }) if detail.contains("format")),
                "{result:?}"
            );
        }
    }

    #[test]
    fn hidden_bolt_folder_is_a_real_folder() {
        let dir = tempfile::tempdir().unwrap();
        Repository::init(dir.path(), &entry(0).who).unwrap();
        let bolt = dir.path().join(".bolt");
        assert!(bolt_platform::is_hidden(&bolt).unwrap());
        crate::safe_fs::check_real_dir(&bolt).unwrap();
    }

    #[test]
    fn errors_say_what_to_do() {
        assert_eq!(
            RepoError::NotAProject.to_string(),
            "Not inside a Bolt project."
        );
        assert!(
            RepoError::NotAProject
                .hint()
                .unwrap()
                .contains("bolt start")
        );
        let e = RepoError::Damaged { detail: "x".into() };
        assert_eq!(e.to_string(), "This project's Bolt data looks damaged: x.");
        assert!(e.hint().unwrap().contains("Nothing was changed"));
        let e = RepoError::NotYours {
            path: PathBuf::from("/srv/work"),
        };
        assert_eq!(
            e.to_string(),
            "The project at /srv/work belongs to another user account."
        );
        let hint = e.hint().unwrap();
        assert!(hint.ends_with("\nsafe-folders = [\"/srv/work\"]"), "{hint}");

        // A failed look-up names the folder, with the same way to trust it.
        let e = RepoError::OwnerLookupFailed {
            path: PathBuf::from("/srv/work"),
            source: io::Error::other("Access is denied. (os error 5)"),
        };
        assert_eq!(
            e.to_string(),
            "Bolt couldn't find out who owns the project at /srv/work: Access is denied. \
             (os error 5)"
        );
        assert_eq!(e.hint(), Some(hint));

        let e = RepoError::OwnerUnknown {
            path: PathBuf::from("/media/usb/work"),
        };
        assert_eq!(
            e.to_string(),
            "Bolt can't tell who owns the project at /media/usb/work: this drive doesn't record \
             who owns folders."
        );
        let trust = config::trust_hint(Path::new("/media/usb/work"));
        assert_eq!(e.hint().unwrap(), trust);
        assert!(
            trust.ends_with("\nsafe-folders = [\"/media/usb/work\"]"),
            "{trust}"
        );
        // `bolt start` that stops after making `.bolt` says so, and how to finish.
        let e = RepoError::StartStopped {
            refusal: Box::new(e),
        };
        assert_eq!(
            e.to_string(),
            "The .bolt folder was made, but the project isn't finished. Bolt can't tell who \
             owns the project at /media/usb/work: this drive doesn't record who owns folders."
        );
        assert_eq!(
            e.hint().unwrap(),
            format!("{trust}\nThen run `bolt start` again to finish.")
        );
    }

    #[test]
    fn the_refusal_names_the_group_and_both_ways_on() {
        let root = Path::new("/srv/work");
        let e = RepoError::AdminsNotElevated {
            path: root.to_owned(),
        };
        assert_eq!(
            e.to_string(),
            "The project at /srv/work belongs to the Administrators group, and this terminal \
             isn't running as administrator."
        );
        let hint = e.hint().unwrap();
        assert!(hint.contains("administrator"), "{hint}");
        assert!(hint.ends_with(&config::trust_hint(root)), "{hint}");
        // A `bolt start` that stops for it says how to finish, as for every other refusal.
        let e = RepoError::StartStopped {
            refusal: Box::new(e),
        };
        assert!(
            e.hint()
                .unwrap()
                .ends_with("\nThen run `bolt start` again to finish.")
        );
    }

    /// Where other accounts can write, the hint says to move the project, so it can't also say
    /// to come back and finish it in place.
    #[test]
    fn a_stopped_start_says_how_to_finish_only_where_the_folder_can_be_trusted() {
        let refusals = |path: &Path| {
            let path = path.to_owned();
            [
                RepoError::NotYours { path: path.clone() },
                RepoError::OwnerUnknown { path: path.clone() },
                RepoError::AdminsNotElevated { path: path.clone() },
                RepoError::OwnerLookupFailed {
                    path,
                    source: io::Error::other("Access is denied."),
                },
            ]
        };
        let stopped = |refusal| RepoError::StartStopped {
            refusal: Box::new(refusal),
        };
        let folder = tempfile::tempdir().unwrap();
        let open_to_all = crate::test_support::open_to_all(folder.path());
        for refusal in refusals(Path::new("/srv/work")) {
            let hint = stopped(refusal).hint().unwrap();
            assert!(
                hint.ends_with("\nThen run `bolt start` again to finish."),
                "{hint}"
            );
        }
        for refusal in refusals(&open_to_all) {
            let hint = stopped(refusal).hint().unwrap();
            assert!(
                hint.ends_with("Move your project into a folder of your own."),
                "{hint}"
            );
        }
    }

    #[test]
    fn untrusted_text_in_errors_is_escaped() {
        use crate::test_support::{HOSTILE, assert_hostile_text_escaped};
        for e in [
            RepoError::NewerFormat {
                detail: HOSTILE.into(),
            },
            RepoError::Damaged {
                detail: HOSTILE.into(),
            },
            RepoError::InsideAnotherProject {
                root: PathBuf::from(HOSTILE),
            },
            RepoError::NotYours {
                path: PathBuf::from(HOSTILE),
            },
            RepoError::OwnerUnknown {
                path: PathBuf::from(HOSTILE),
            },
            RepoError::AdminsNotElevated {
                path: PathBuf::from(HOSTILE),
            },
            RepoError::OwnerLookupFailed {
                path: PathBuf::from(HOSTILE),
                source: io::Error::other(HOSTILE),
            },
            RepoError::StartStopped {
                refusal: Box::new(RepoError::NotYours {
                    path: PathBuf::from(HOSTILE),
                }),
            },
            // The path of a damaged pack has the project folder's name in it.
            PackError::Corrupt {
                path: PathBuf::from(HOSTILE),
                reason: "x".into(),
            }
            .into(),
        ] {
            assert_hostile_text_escaped(&e.to_string());
            // The ownership refusals' hints name the folder too.
            if matches!(
                e,
                RepoError::NotYours { .. }
                    | RepoError::OwnerUnknown { .. }
                    | RepoError::AdminsNotElevated { .. }
                    | RepoError::OwnerLookupFailed { .. }
                    | RepoError::StartStopped { .. }
            ) {
                assert_hostile_text_escaped(&e.hint().unwrap());
            }
        }
        // The hash named by a hostile `.bolt/format` is quoted in the message.
        let e = parse_format("bolt-repository 1\nhash \u{202e}evil\n").unwrap_err();
        assert!(matches!(e, RepoError::NewerFormat { .. }), "{e:?}");
        assert!(e.to_string().contains("\"\\u{202e}evil\""), "{e}");
    }

    #[cfg(feature = "fault-injection")]
    #[test]
    fn commit_is_all_or_nothing() {
        use crate::fsio::fault;
        use crate::verify::verify_store;
        let (dir, mut repo) = project();
        let a = commit(&mut repo, &entry(1)).unwrap();
        for n in 1.. {
            assert!(n < 50, "the commit never succeeded");
            let lock = repo.lock_for_change("save", &mut |_| {}).unwrap();
            let packs = repo.new_packs().unwrap();
            fault::arm(n);
            let result = repo.commit(&lock, packs, &entry(2));
            fault::disarm();
            drop(lock);
            drop(repo);
            repo = Repository::open(dir.path()).unwrap();
            match result {
                Err(_) => {
                    assert_eq!(repo.head().unwrap(), a, "fault {n}");
                    assert!(verify_store(repo.packs_dir()).is_ok(), "fault {n}");
                }
                Ok(b) => {
                    assert_eq!(repo.head().unwrap(), b);
                    assert_eq!(repo.current().unwrap().1, entry(2));
                    assert!(verify_store(repo.packs_dir()).is_ok());
                    // Faults hit the pack rename, after it, before the head, and the head write.
                    assert!(n > 4, "only {} faults were injected", n - 1);
                    break;
                }
            }
        }
    }

    fn count_packs(repo: &Repository) -> usize {
        std::fs::read_dir(repo.packs_dir()).unwrap().count()
    }

    #[test]
    fn a_command_past_the_limit_writes_several_packs_and_takes_effect_once() {
        let (_dir, mut repo) = project();
        let a = commit(&mut repo, &entry(1)).unwrap();
        let packs_before = count_packs(&repo);
        let lock = repo.lock_for_change("save", &mut |_| {}).unwrap();
        let mut packs = PackSet::with_limit(repo.tmp_dir(), repo.packs_dir(), 3).unwrap();
        let ids: Vec<Id> = (0..7u8).map(|i| packs.add_content(&[i]).unwrap()).collect();
        // Two packs are in the folder by now, but nothing can see them before the commit.
        assert_eq!(count_packs(&repo), packs_before + 2);
        assert_eq!(repo.head().unwrap(), a);
        assert!(matches!(
            repo.store().copy_content(&ids[0], &mut io::sink()),
            Err(StoreError::Missing(_))
        ));
        let b = repo.commit(&lock, packs, &entry(2)).unwrap();
        // 3 + 3 + 1 objects, then the entry.
        assert_eq!(count_packs(&repo), packs_before + 3);
        assert_eq!(repo.head().unwrap(), b);
        for id in ids {
            repo.store().copy_content(&id, &mut io::sink()).unwrap();
        }
    }

    #[test]
    fn an_entry_that_fills_the_last_pack_leaves_no_empty_one_behind() {
        let (_dir, mut repo) = project();
        let lock = repo.lock_for_change("save", &mut |_| {}).unwrap();
        let mut packs = PackSet::with_limit(repo.tmp_dir(), repo.packs_dir(), 3).unwrap();
        packs.add_content(b"a").unwrap();
        packs.add_content(b"b").unwrap();
        // The entry is the third object, so it fills the pack.
        let id = repo.commit(&lock, packs, &entry(1)).unwrap();
        assert_eq!(repo.head().unwrap(), id);
        assert_eq!(count_packs(&repo), 1);
        assert_eq!(std::fs::read_dir(repo.tmp_dir()).unwrap().count(), 0);
        repo.store()
            .read_record(&id, ObjectKind::DiaryEntry)
            .unwrap();
    }

    #[test]
    fn the_diary_entry_is_in_the_last_pack() {
        let (_dir, mut repo) = project();
        let lock = repo.lock_for_change("save", &mut |_| {}).unwrap();
        let mut packs = PackSet::with_limit(repo.tmp_dir(), repo.packs_dir(), 3).unwrap();
        let ids: Vec<Id> = (0..7u8).map(|i| packs.add_content(&[i]).unwrap()).collect();
        let entry_id = repo.commit(&lock, packs, &entry(1)).unwrap();
        let mut holders = Vec::new();
        for pack in std::fs::read_dir(repo.packs_dir()).unwrap() {
            let reader = PackReader::open(&pack.unwrap().path()).unwrap();
            if reader.find(&entry_id).is_some() {
                holders.push(reader);
            }
        }
        assert_eq!(holders.len(), 1);
        // The last pack holds what came last: the seventh object and the entry.
        let held: BTreeSet<Id> = holders[0].entries().iter().map(|e| e.id).collect();
        assert_eq!(held, BTreeSet::from([ids[6], entry_id]));
    }

    #[cfg(feature = "fault-injection")]
    #[test]
    fn a_crash_between_packs_leaves_the_repository_as_it_was() {
        use crate::fsio::fault;
        use crate::id::hash_content;
        use crate::verify::verify_store;

        /// A command that needs three packs: seven objects with room for three in a pack, then
        /// its entry.
        fn past_the_limit(repo: &mut Repository, lock: &ChangeLock) -> Result<Id, RepoError> {
            let mut packs = PackSet::with_limit(repo.tmp_dir(), repo.packs_dir(), 3)?;
            for i in 0..7u8 {
                packs.add_content(&[i])?;
            }
            repo.commit(lock, packs, &entry(2))
        }

        let mut rolled_over = false;
        for n in 1.. {
            assert!(n < 50, "the command never succeeded");
            let (dir, mut repo) = project();
            let a = commit(&mut repo, &entry(1)).unwrap();
            let lock = repo.lock_for_change("save", &mut |_| {}).unwrap();
            fault::arm(n);
            let result = past_the_limit(&mut repo, &lock);
            fault::disarm();
            drop(lock);
            drop(repo);
            let mut repo = Repository::open(dir.path()).unwrap();
            let Err(crash) = result else {
                assert_eq!(repo.current().unwrap().1, entry(2));
                break;
            };
            assert_eq!(repo.head().unwrap(), a, "fault {n}");
            assert!(verify_store(repo.packs_dir()).is_ok(), "fault {n}");
            // Running the command again works, and finds what the crash left behind.
            let lock = repo.lock_for_change("save", &mut |_| {}).unwrap();
            if crash.to_string().ends_with("packs:rolled-over") {
                rolled_over = true;
                let first = hash_content(&[0]);
                assert!(
                    repo.store()
                        .has_good_copy(&first, ObjectKind::Content, 1)
                        .unwrap()
                );
            }
            let b = past_the_limit(&mut repo, &lock).unwrap();
            assert_eq!(repo.head().unwrap(), b, "fault {n}");
            assert!(verify_store(repo.packs_dir()).is_ok(), "fault {n}");
        }
        assert!(rolled_over, "no fault point sat between two packs");
    }

    #[test]
    fn second_changer_waits_and_is_told_why() {
        let (dir, mut repo) = project();
        let root = dir.path().to_owned();
        let (tx, rx) = mpsc::channel();
        let a = thread::spawn(move || {
            let mut repo = Repository::open(&root).unwrap();
            let lock = repo
                .lock_for_change("save", &mut |_| panic!("A should not wait"))
                .unwrap();
            tx.send(()).unwrap();
            thread::sleep(Duration::from_millis(300));
            let released = Instant::now();
            drop(lock);
            released
        });
        rx.recv().unwrap();
        let mut told = Vec::new();
        let lock = repo
            .lock_for_change("undo", &mut |info| told.push(info.to_owned()))
            .unwrap();
        let got = Instant::now();
        let released = a.join().unwrap();
        assert!(got >= released, "B got the lock while A held it");
        assert_eq!(told.len(), 1);
        assert!(told[0].contains("save"), "{told:?}");
        drop(lock);
    }

    #[test]
    fn readers_never_wait() {
        let (dir, mut repo) = project();
        let a = commit(&mut repo, &entry(1)).unwrap();
        let _lock = repo.lock_for_change("save", &mut |_| {}).unwrap();
        let root = dir.path().to_owned();
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let repo = Repository::open(&root).unwrap();
            tx.send(repo.head().unwrap()).unwrap();
        });
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), a);
    }
}
