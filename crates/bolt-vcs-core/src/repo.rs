//! The repository: the `format` file, the head pointer, the change lock and the commit protocol
//! (docs/storage-format.md section 10, decision 010).

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::fs::{self, File, TryLockError};
use std::io;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use bolt_platform::Owner;
use bolt_platform::dir::{Dir, EntryType, is_link_refusal};

use crate::config::{self, ConfigError};
use crate::diary::{Action, DiaryEntry, Position, ServerLines, State, TagList};
use crate::encoding::DecodeError;
use crate::fsio::{self, Done, fault_point, write_atomic};
use crate::id::{ChangeId, Id};
use crate::journal::Journal;
use crate::keyed::KEY_LEN;
use crate::listing::Listing;
use crate::names::NameError;
use crate::pack::{ObjectKind, PackError, PackSet};
use crate::rules::{self, RulesRead};
use crate::safe_fs;
use crate::save::{EXTRA_LINE, Person, Save};
use crate::store::{Store, StoreError, is_damage};
use crate::text::escape;
use crate::warning::{self, Warning};

pub const FORMAT_TEXT: &str = "bolt-repository 1\nhash blake3\nfeature chunks\n";
/// The store may hold chunk lists, which builds from before them can't read safely.
const CHUNKS: &str = "chunks";

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
    /// `refusal`, though safe-folders lists `entry`, which never counts for `why`.
    #[error("{refusal} {}", passed_over(entry, why))]
    SafeFolderPassedOver {
        entry: PathBuf,
        why: &'static str,
        refusal: Box<RepoError>,
    },
    #[error(
        "The .bolt folder at {} changed while Bolt looked at it.",
        escape(&root.display().to_string(), false)
    )]
    BoltChanged { root: PathBuf },
    /// `bolt start` made `.bolt` but may not use it. Once the folder is trusted, running it again
    /// finishes the project.
    #[error("The .bolt folder was made, but the project isn't finished. {refusal}")]
    StartStopped { refusal: Box<RepoError> },
    /// A command found the `.bolt` of a start or import that stopped before it wrote `format`.
    /// `stored`: it holds a head or packs, which may be history Git no longer has.
    #[error(
        "An import or start began a Bolt project at {} and didn't finish it.",
        escape(&root.display().to_string(), false)
    )]
    Unfinished { root: PathBuf, stored: bool },
    /// A command found the `.bolt` of a start or import that is still running.
    #[error(
        "A Bolt import or start is still making the project at {}.",
        escape(&root.display().to_string(), false)
    )]
    BeingMade { root: PathBuf },
    /// `bolt start` found files an unfinished import stored, which an empty project would leave
    /// unused.
    #[error(
        "An import began a Bolt project here and didn't finish, and bolt start won't make an empty \
         project over what it stored."
    )]
    ImportUnfinished,
    #[error(
        "This project uses {}, which this version of Bolt doesn't understand.",
        escape(detail, false)
    )]
    NewerFormat { detail: String },
    #[error("This project's Bolt data looks damaged: {}.", escape(detail, false))]
    Damaged { detail: String },
    /// A folder of a save holds a spelling of `.bolt`, which no listing Bolt makes holds: of
    /// `save` once the command knows which save it was reading. `current` only when the project
    /// is on that save, which a save read for another reason (a shelf's base, say) isn't.
    #[error(
        "{} holds a name spelled like Bolt's own folder (.bolt), which Bolt never writes, so it \
         can't be used.",
        save.map_or_else(|| "A save".to_owned(), |id| format!("Save {}", id.short_hex()))
    )]
    BoltName { save: Option<Id>, current: bool },
    /// `tidy` can't tell what pack `path` keeps: `why` it can't open, read or check it, or a chunk
    /// list in it, or `None` when it holds a conflict record, which this version doesn't follow.
    #[error(
        "Bolt can't tidy this project: pack {} {}.",
        escape(&path.display().to_string(), false),
        escape(
            why.as_deref()
                .unwrap_or("holds a conflict record, which this version of Bolt doesn't follow"),
            false
        )
    )]
    CantTidy { path: PathBuf, why: Option<String> },
    /// `state/head` is missing, isn't an entry's ID, or names an entry the store can't give: no
    /// command can tell which state is current, though every save is in the store.
    #[error(
        "Bolt can't tell which diary entry is the project's current one: .bolt/state/head {}.",
        escape(detail, false)
    )]
    HeadLost { detail: String },
    /// `state/journal`, the record of a stopped switch between saves, can't be used. It is
    /// neither carried out nor passed over, since the folder may be half switched.
    #[error(
        "A switch between saves was stopped here, and Bolt can't finish it: \
         .bolt/state/journal {}.",
        escape(detail, false)
    )]
    JournalUnusable { detail: String },
    /// A switch between saves stopped at `path` with an error it can't go past.
    #[error("A switch between saves stopped at {}: {source}", escape(path, false))]
    SwitchStopped {
        path: String,
        source: Box<RepoError>,
    },
    /// A stopped switch was carried on, but another program still holds files it must write.
    #[error(
        "A switch between saves was stopped, and Bolt couldn't finish it: {}",
        escape(detail, true)
    )]
    SwitchInUse { detail: String },
    /// One of Bolt's own files in `.bolt` couldn't be replaced.
    #[error(
        "Bolt couldn't write {}: {}",
        escape(&path.display().to_string(), false),
        escape(&source.to_string(), false)
    )]
    CantWrite { path: PathBuf, source: io::Error },
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
    /// The user's own cache key can't be used. It is never replaced silently: a key Bolt didn't
    /// make there may mean someone changed the user's files.
    #[error(
        "Bolt can't use its cache key {}: {}.",
        escape(&path.display().to_string(), false),
        escape(detail, false)
    )]
    CacheKey { path: PathBuf, detail: String },
}

impl RepoError {
    /// The project folder, for the refusals whose hint is to trust it. The one place that says
    /// which they are.
    fn trust_path(&self) -> Option<&Path> {
        match self {
            RepoError::NotYours { path }
            | RepoError::OwnerUnknown { path }
            | RepoError::OwnerLookupFailed { path, .. }
            | RepoError::AdminsNotElevated { path } => Some(path.as_path()),
            RepoError::SafeFolderPassedOver { refusal, .. } => refusal.trust_path(),
            _ => None,
        }
    }

    /// What the user can do next, if there is something to say.
    #[must_use]
    pub fn hint(&self) -> Option<String> {
        if let RepoError::SafeFolderPassedOver { refusal, .. } = self {
            return refusal.hint();
        }
        if let Some(path) = self.trust_path() {
            let trust = config::trust_hint(path);
            return Some(if matches!(self, RepoError::AdminsNotElevated { .. }) {
                format!("Open a terminal as administrator and run the command there.\n{trust}")
            } else {
                trust
            });
        }
        Some(
            match self {
                RepoError::NotAProject => "Run `bolt start` to make this folder a Bolt project.",
                RepoError::InsideAnotherProject { .. } => {
                    "Run Bolt commands there, or pick a folder outside it."
                }
                RepoError::StartStopped { refusal } => {
                    return refusal.hint_then("Then run `bolt start` again to finish.");
                }
                // Deleting is offered only for a folder that holds nothing: what is stored may be
                // history Git no longer has, and running the command again finishes from it.
                RepoError::Unfinished { stored: false, .. } => {
                    "Run `bolt import` (or `bolt start`, if that began it) in that folder again to \
                     finish, or delete its hidden .bolt folder."
                }
                RepoError::Unfinished { stored: true, .. } => {
                    "Run `bolt import` (or `bolt start`, if that began it) in that folder again to \
                     finish."
                }
                RepoError::BeingMade { .. } => "Run the command again once it has finished.",
                RepoError::BoltChanged { .. } => "Run the command again.",
                RepoError::ImportUnfinished => "Run `bolt import` again to finish the import.",
                RepoError::NewerFormat { .. } => "Update Bolt to open it.",
                RepoError::Damaged { .. } => {
                    "Nothing was changed. Ask for help before trying to repair it."
                }
                // Every switch away from the save the project is on reads it first.
                RepoError::BoltName { current: true, .. } => {
                    "Bolt never makes such a save, so another program put it there. Nothing was \
                     changed: ask for help before trying to repair it."
                }
                RepoError::BoltName { .. } => {
                    "Bolt never makes such a save, so another program put it there. Nothing was \
                     changed: go on from another save, and ask for help before trying to repair \
                     this one."
                }
                RepoError::CantTidy { why: None, .. } => {
                    "Nothing was removed, and the project works as before. A version of Bolt that \
                     follows them can tidy it."
                }
                RepoError::CantTidy { .. } => {
                    "Nothing was removed. Check the pack's permissions. If it is damaged for good, \
                     moving it out of .bolt/store/packs lets `bolt tidy` run, losing what only it \
                     holds, and the chunks only its chunk lists name."
                }
                RepoError::HeadLost { .. } => {
                    "Your saves are safe. `bolt undo --list` lists the entries where the diary may \
                     have stopped, newest first, and `bolt undo --to <entry>` makes one of them \
                     current, without changing any file."
                }
                // Removing the journal is no roll back: the files stay as the switch left them.
                RepoError::JournalUnusable { .. } => {
                    "Nothing was changed. Your files are as the stopped switch left them, and \
                     `bolt status` shows how they differ from the save the project is on. To go \
                     on without finishing the switch, delete .bolt/state/journal: a later `bolt save` \
                     then records the folder as it is at that time."
                }
                RepoError::SwitchStopped { source, .. } => {
                    // Damage stops every later try too, so name the way on without that data.
                    let copy = match **source {
                        RepoError::Damaged { .. }
                        | RepoError::Store(StoreError::WrongKind { .. } | StoreError::Decode(_)) => {
                            Some("looks damaged")
                        }
                        RepoError::Store(StoreError::Missing(_)) => Some("is missing"),
                        _ => None,
                    };
                    let damage = copy.map_or(String::new(), |copy| {
                        format!(
                            " Bolt's copy of it {copy}: ask for help before trying to repair the \
                             project. To go on without it, delete .bolt/state/journal: your files \
                             stay as they are, and the next command goes on from there."
                        )
                    });
                    return Some(format!(
                        "The switch isn't finished: what it already wrote stays, and the next \
                         command that changes the project finishes it once that is fixed.{damage}"
                    ));
                }
                RepoError::SwitchInUse { .. } => {
                    "Close that program, then run the command again: Bolt finishes the switch \
                     before anything else."
                }
                RepoError::CantWrite { source, .. } if fsio::held(source) => {
                    "Another program, such as a virus scanner or sync tool, may have it open: run \
                     the command again in a moment. If it keeps failing, check that Bolt can write \
                     to the .bolt folder."
                }
                RepoError::CantWrite { .. } => {
                    "Check that Bolt can write to the .bolt folder and that its drive has free \
                     space, then run the command again."
                }
                RepoError::Config(e) => return e.hint(),
                RepoError::CacheKey { .. } => {
                    "If that file is there, delete it: Bolt makes a new key, and rebuilds the \
                     caches the old one sealed. If it isn't, check that Bolt can write to its \
                     folder."
                }
                _ => return None,
            }
            .to_owned(),
        )
    }

    /// This error, met reading the folders of the save `save`: a spelling of `.bolt` is told as
    /// that save's, `current` when the project is on it.
    #[must_use]
    pub(crate) fn in_save(self, save: Id, current: bool) -> RepoError {
        match self {
            RepoError::BoltName { save: None, .. } => RepoError::BoltName {
                save: Some(save),
                current,
            },
            other => other,
        }
    }

    /// `hint`, then `then`, for a refusal that stopped `bolt start` or `bolt import` after `.bolt`
    /// was made. Where other accounts can write, the hint says to move the project instead, so
    /// `then` is left off.
    pub(crate) fn hint_then(&self, then: &str) -> Option<String> {
        let hint = self.hint()?;
        let moves = self.trust_path().is_some_and(config::others_can_write);
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
            // Only a listing's names are held to the rules that refuse it.
            StoreError::Decode(DecodeError::InvalidName(NameError::BoltSpelling)) => {
                RepoError::BoltName {
                    save: None,
                    current: false,
                }
            }
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

pub(crate) fn damaged(detail: impl Into<String>) -> RepoError {
    RepoError::Damaged {
        detail: detail.into(),
    }
}

/// A `safe_fs` refusal (a link, special file or oversized file where `.bolt` must hold a real
/// one) is damage; any other error stays an I/O error.
pub(crate) fn refused(e: io::Error) -> RepoError {
    match e.kind() {
        io::ErrorKind::InvalidInput | io::ErrorKind::InvalidData => damaged(e.to_string()),
        _ => e.into(),
    }
}

/// `e`, from opening the lock file at `shown`, as `refused` takes it, naming the file.
fn lock_error(shown: &Path, e: io::Error) -> RepoError {
    match refused(e) {
        RepoError::Io(e) => naming(shown, "open", e).into(),
        e => e,
    }
}

/// `e`, met when Bolt went to `doing` the file at `shown`, naming the file.
fn naming(shown: &Path, doing: &str, e: io::Error) -> io::Error {
    let shown = escape(&shown.display().to_string(), false);
    io::Error::new(e.kind(), format!("Bolt couldn't {doing} {shown}: {e}"))
}

/// `e`, from replacing Bolt's own file at `path`, naming it.
pub(crate) fn cant_write(path: PathBuf, e: io::Error) -> RepoError {
    RepoError::CantWrite { path, source: e }
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

/// `.bolt` and the fixed folders in it that commands read and write, held open from the moment
/// its shape was checked: what Bolt reads and writes there goes through these, so a folder
/// swapped for a link since is never followed (security.md T19).
struct Held {
    bolt: Dir,
    state: Dir,
    packs: Arc<Dir>,
    tmp: Arc<Dir>,
    cache: Dir,
}

impl Held {
    /// The fixed folders of the `.bolt` folder `bolt`, held open as `top`, each made in the one
    /// above it if missing, then opened from it.
    fn make(top: Dir, bolt: &Path) -> Result<Held, RepoError> {
        let store = make_fixed(&top, bolt, "store")?;
        Ok(Held {
            state: make_fixed(&top, bolt, "state")?,
            packs: Arc::new(make_fixed(&store, &bolt.join("store"), "packs")?),
            tmp: Arc::new(make_fixed(&top, bolt, "tmp")?),
            cache: make_fixed(&top, bolt, "cache")?,
            bolt: top,
        })
    }
}

/// The folder `name` in `dir`, the folder at `path`. A link, junction or file there is damage, as
/// `check_shape` finds it.
fn fixed(dir: &Dir, path: &Path, name: &str) -> Result<Dir, RepoError> {
    dir.open_dir(OsStr::new(name))
        .map_err(|e| unreal(&path.join(name), e))
}

/// `fixed`, made first if nothing has the name, with `dir` flushed after.
fn make_fixed(dir: &Dir, path: &Path, name: &str) -> Result<Dir, RepoError> {
    match dir.create_dir(OsStr::new(name)) {
        Ok(()) => dir.sync()?,
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(e.into()),
    }
    fixed(dir, path, name)
}

/// What opening the fixed folder at `path` found: a link, junction or file there is damage, as
/// `check_shape` finds it.
fn unreal(path: &Path, e: io::Error) -> RepoError {
    if is_link_refusal(&e) || e.kind() == io::ErrorKind::NotADirectory {
        damaged(format!(
            "{} is not a real folder (it is a link, junction or file)",
            path.display()
        ))
    } else {
        e.into()
    }
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
/// listed `root` in safe-folders, as Git does (T23). `bolt_seen`: the caller just saw `.bolt`
/// there, so it may not be missing now.
fn check_owner(root: &Path, bolt_seen: bool) -> Result<(), RepoError> {
    check_owner_with(
        root,
        bolt_seen,
        &bolt_platform::owner_of,
        &config::safe_folders,
    )
}

/// `check_owner`, given how to learn who owns a path and how to read safe-folders.
fn check_owner_with(
    root: &Path,
    bolt_seen: bool,
    owner_of: &dyn Fn(&Path) -> io::Result<Owner>,
    safe_folders: &dyn Fn() -> Result<Vec<PathBuf>, ConfigError>,
) -> Result<(), RepoError> {
    let mut owners = vec![owner_of(root)];
    fsio::act_point("owner:asking");
    let bolt_owner = owner_of(&root.join(".bolt"));
    fsio::act_point("owner:asked");
    match bolt_owner {
        // Gone since it was seen: if it comes back, nobody asked who owns it.
        Err(e) if e.kind() == io::ErrorKind::NotFound && bolt_seen => {
            return Err(RepoError::BoltChanged {
                root: root.to_owned(),
            });
        }
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
/// that couldn't be found out): all must be the user's, unless `safe` lists `root` itself. An
/// entry that is itself a link never counts: whoever can swap a trusted folder for one would move
/// the trust to wherever it leads. The Administrators group is the user's only while it is
/// switched on in this terminal.
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
    let Some(refusal) = refusal else {
        return Ok(());
    };
    // Each entry is judged from one open of it, by the identity of the folder that open holds, so
    // swapping it for a link between two looks can't win. A link above its last part, such as
    // macOS's `/var`, is followed as usual.
    let project = safe_fs::folder_id(root).ok();
    let mut passed_over = None;
    for folder in safe {
        let why = if folder.components().any(|c| c == Component::ParentDir) {
            DOT_DOT
        } else {
            match safe_fs::folder_id(folder) {
                Ok(id) if Some(id) == project => return Ok(()),
                Err(e) if bolt_platform::dir::is_link_refusal(&e) => A_LINK,
                _ => continue,
            }
        };
        // Named only when it leads to the project, so the refusal says why that didn't count.
        if passed_over.is_none() && config::same_folder(folder, root) {
            passed_over = Some((folder.clone(), why));
        }
    }
    Err(match passed_over {
        Some((entry, why)) => RepoError::SafeFolderPassedOver {
            entry,
            why,
            refusal: Box::new(refusal),
        },
        None => refusal,
    })
}

/// Why an entry with `..` in it never counts.
const DOT_DOT: &str = "it has `..` in it, which the system resolves through any link before it";

/// Why a link entry never counts: what the check refuses there.
const A_LINK: &str = if cfg!(windows) {
    "it is a link, a junction, a mounted volume or another reparse point"
} else {
    "it is a link"
};

/// What a refusal adds when safe-folders lists the folder only through `entry`, passed over for
/// `why`.
pub(crate) fn passed_over(entry: &Path, why: &str) -> String {
    format!(
        "Its safe-folders entry {} doesn't count: {why}, and Bolt never trusts a folder that way.",
        escape(&entry.display().to_string(), false)
    )
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
    owner_check: &dyn Fn(&Path, bool) -> Result<(), RepoError>,
) -> Result<bool, RepoError> {
    let bolt = root.join(".bolt");
    let is_there = || match safe_fs::check_real_dir(&bolt) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(refused(e)),
    };
    let found = is_there()?;
    owner_check(root, found)?;
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
    let mut features = BTreeSet::new();
    for line in lines {
        let feature = line
            .strip_prefix("feature ")
            .filter(|f| is_word(f))
            .ok_or_else(damaged_format)?;
        if feature != CHUNKS {
            return Err(RepoError::NewerFormat {
                detail: format!("the feature \"{feature}\""),
            });
        }
        if !features.insert(feature.to_owned()) {
            return Err(damaged_format());
        }
    }
    Ok(FormatInfo { features })
}

/// What `state/head` holds.
enum Head {
    At(Id),
    Missing,
    /// Anything but an entry's ID, in lowercase hex, and a newline.
    Garbled,
}

impl Head {
    fn id(&self) -> Option<Id> {
        match self {
            Head::At(id) => Some(*id),
            Head::Missing | Head::Garbled => None,
        }
    }
}

/// `head` in `state`, the `state` folder of the `.bolt` folder `bolt`. A link or special file
/// there is damage.
fn read_head(state: &Dir, bolt: &Path) -> Result<Head, RepoError> {
    let shown = bolt.join("state").join("head");
    let max = safe_fs::MAX_HEAD_BYTES;
    let bytes = match safe_fs::read_capped_in(state, OsStr::new("head"), &shown, max) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Head::Missing),
        // Longer than any head.
        Err(e) if e.kind() == io::ErrorKind::InvalidData => return Ok(Head::Garbled),
        Err(e) => return Err(refused(e)),
    };
    let id = std::str::from_utf8(&bytes)
        .ok()
        .and_then(|t| t.strip_suffix('\n'))
        .and_then(|h| Id::from_hex(h).filter(|id| id.to_hex() == h));
    Ok(id.map_or(Head::Garbled, Head::At))
}

fn head_lost(detail: impl Into<String>) -> RepoError {
    RepoError::HeadLost {
        detail: detail.into(),
    }
}

/// The diary entry `id`, which `state/head` names, from `store`. One that can't be had is a lost
/// head, unless it couldn't be read for a reason that isn't damage.
pub(crate) fn head_entry(store: &Store, id: Id) -> Result<(Id, DiaryEntry), RepoError> {
    let lost = |why: String| head_lost(format!("names diary entry {}, {why}", id.short_hex()));
    let bytes = match store.read_record(&id, ObjectKind::DiaryEntry) {
        Ok(bytes) => bytes,
        Err(StoreError::Missing(_) | StoreError::WrongKind { .. }) => {
            return Err(lost("which the store doesn't hold".to_owned()));
        }
        Err(StoreError::Pack(e)) if is_damage(&e) => {
            return Err(lost(format!("which can't be read ({e})")));
        }
        Err(e) => return Err(e.into()),
    };
    let entry =
        DiaryEntry::decode(&bytes).map_err(|e| lost(format!("which can't be read ({e})")))?;
    Ok((id, entry))
}

/// The diary entry `id` in `store`. One the store lacks, holds as another kind of record, or
/// can't decode, is damage to the diary, named.
pub(crate) fn entry_in(store: &Store, id: &Id) -> Result<DiaryEntry, RepoError> {
    let bytes = match store.read_record(id, ObjectKind::DiaryEntry) {
        Ok(bytes) => bytes,
        Err(StoreError::Missing(_) | StoreError::WrongKind { .. }) => {
            return Err(damaged(format!("the diary has no entry {id}")));
        }
        Err(e) => return Err(e.into()),
    };
    DiaryEntry::decode(&bytes).map_err(|e| damaged(format!("diary entry {id} can't be read: {e}")))
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
    held: Held,
    format: FormatInfo,
    store: Store,
    /// The head read before the store was last loaded, so every object it reaches is
    /// registered. `None` when there was no head yet, or none that is an entry's ID.
    head: Option<Id>,
    /// The user's own key, which seals the caches; without it none is read or written.
    cache_key: Option<[u8; KEY_LEN]>,
    /// The project's `bolt.toml`, as the change lock last read it.
    rules: RulesRead,
}

/// `path` with its links, `..` and short names resolved. Windows' `\\?\` in front is left off
/// where the plain path leads to the same place, so the path reads as the user would type it.
/// Every command resolves the folder it starts in, so a link to a project is judged by the folder
/// it leads to. Some drives can't resolve a path (RAM disks, say): there `path` is kept, and a
/// link on it is refused as before.
#[must_use]
pub fn resolved(path: &Path) -> PathBuf {
    let Ok(full) = fs::canonicalize(path) else {
        return path.to_owned();
    };
    let plain = full
        .to_str()
        .and_then(|s| match s.strip_prefix(r"\\?\UNC\") {
            Some(share) => Some(format!(r"\\{share}")),
            None => s.strip_prefix(r"\\?\").map(str::to_owned),
        });
    match plain {
        Some(plain) if fs::canonicalize(&plain).is_ok_and(|again| again == full) => plain.into(),
        _ => full,
    }
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
    owner_check: &dyn Fn(&Path, bool) -> Result<(), RepoError>,
) -> Result<bool, RepoError> {
    let is_link = match fs::symlink_metadata(folder.join(".bolt")) {
        Ok(meta) if safe_fs::is_link(&meta) => true,
        Ok(meta) if meta.is_dir() => false,
        _ => return Ok(false),
    };
    owner_check(folder, true)?;
    // A link counts, so that `open` refuses it; a real `.bolt` needs a `format`.
    Ok(is_link || has_format(folder).unwrap_or(false))
}

/// What a command says of `folder` when it holds a real `.bolt` of Bolt's own names without
/// `format`; `None` for any other folder. A start or import holding the lock is still making the
/// project; a project whose diary shows saves lost its `format`, which is damage; anything else
/// is a start or import that stopped. Only for a folder `has_project` has passed, so its owner
/// was asked.
fn unfinished_here(folder: &Path) -> Option<RepoError> {
    let bolt = folder.join(".bolt");
    let begun = safe_fs::check_real_dir(&bolt).is_ok()
        && matches!(has_format(folder), Ok(false))
        && matches!(holds_only_bolts_own(&bolt), Ok(true));
    if !begun {
        return None;
    }
    let root = folder.to_owned();
    // Nothing in it is opened through a link.
    if let Err(e) = check_shape(&bolt) {
        return Some(e);
    }
    if lock_is_held(&bolt) {
        return Some(RepoError::BeingMade { root });
    }
    Some(match diary_of(&bolt) {
        Ok(Diary::Nothing) => RepoError::Unfinished {
            root,
            stored: false,
        },
        Ok(Diary::Begun) => RepoError::Unfinished { root, stored: true },
        Ok(Diary::Saves) => lost_format(&root),
        Err(e) => e,
    })
}

/// Whether a command holds the change lock of the `.bolt` folder `bolt`. Only looks: a lock file
/// that isn't there was never taken, and a failure other than a busy lock says nothing either way.
fn lock_is_held(bolt: &Path) -> bool {
    let path = bolt.join("state").join("lock");
    // Opened as `lock_for_change` opens it: some drives lock only a file open for writing.
    let opened = safe_fs::open_regular(&path, fs::OpenOptions::new().read(true).write(true));
    opened.is_ok_and(|file| matches!(file.try_lock(), Err(TryLockError::WouldBlock)))
}

/// How far the diary of a `.bolt` without `format` got.
enum Diary {
    /// No head and no packs: nothing is stored.
    Nothing,
    /// What a start or import stores before it finishes, which running it again redoes: packs,
    /// and at most the one entry it commits just before it writes `format`.
    Begun,
    /// The head's entry has a parent or, with no head, an entry in a pack does: a project that
    /// had saves and lost its `format`, and maybe its head too.
    Saves,
}

/// How far the diary of the `.bolt` folder `bolt`, whose shape was checked, got.
fn diary_of(bolt: &Path) -> Result<Diary, RepoError> {
    let state = bolt.join("state");
    // A start can stop before it makes `state`.
    let head = match Dir::open_top(&state) {
        Ok(dir) => match read_head(&dir, bolt)? {
            Head::Garbled => {
                return Err(damaged(
                    "state/head is not a lowercase fingerprint and a newline",
                ));
            }
            head => head.id(),
        },
        Err(e) if e.kind() == io::ErrorKind::NotFound => None,
        Err(e) => return Err(unreal(&state, e)),
    };
    let packs = bolt.join("store").join("packs");
    // A start can stop before it makes the store.
    let no_store =
        matches!(fs::symlink_metadata(&packs), Err(e) if e.kind() == io::ErrorKind::NotFound);
    if head.is_none() && no_store {
        return Ok(Diary::Nothing);
    }
    let store = Store::open(&packs)?;
    let has_parent = |id: &Id| -> Result<bool, RepoError> {
        let entry = store.read_record(id, ObjectKind::DiaryEntry)?;
        Ok(DiaryEntry::decode(&entry)?.parent.is_some())
    };
    let saves = match head {
        Some(head) => has_parent(&head)?,
        None if store.pack_count() == 0 => return Ok(Diary::Nothing),
        // Without a head, the entries of the saves it led to are still in the packs.
        None => store.any_entry(ObjectKind::DiaryEntry, has_parent)?,
    };
    Ok(if saves { Diary::Saves } else { Diary::Begun })
}

/// The project at `root` had saves and lost only its `format`: damage, whatever the cause, and
/// never a project to make again over its saves.
fn lost_format(root: &Path) -> RepoError {
    damaged(format!(
        "the project at {} has saves, but no .bolt/format file",
        root.display()
    ))
}

/// Whether `root/.bolt/state/head` is there, without following a link to it: some command
/// committed.
pub(crate) fn has_head(root: &Path) -> bool {
    fs::symlink_metadata(root.join(".bolt").join("state").join("head")).is_ok()
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
    owner_check: &dyn Fn(&Path, bool) -> Result<(), RepoError>,
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
    /// last. A `.bolt` without `format` is an interrupted start, which this finishes, or an
    /// interrupted import, which it refuses once the import has stored files: `bolt import`
    /// finishes that, and an empty project would leave them unused. One whose diary shows saves
    /// is a project that lost its `format`, which it refuses as damage (`check_new`).
    pub fn init(root: &Path, who: &Person) -> Result<Repository, RepoError> {
        Repository::init_with(root, who, &check_owner)
    }

    /// `init`, with the owner check given (a stand-in in tests).
    fn init_with(
        root: &Path,
        who: &Person,
        owner_check: &dyn Fn(&Path, bool) -> Result<(), RepoError>,
    ) -> Result<Repository, RepoError> {
        let (mut repo, lock) = Repository::begin_with(root, owner_check)?;
        // A start stores no file, so files are an import's.
        if repo.store.holds_content()? {
            return Err(RepoError::ImportUnfinished);
        }
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
            folder_change: None,
        };
        repo.commit_first(&lock, packs, &entry)?;
        Ok(repo)
    }

    /// Refuses to make `root` a project when it or its `.bolt` isn't the user's, when it is a
    /// project already, when another tool made its `.bolt`, when it is a project that had saves
    /// and lost its `format` (damage), or when it is inside another project. Makes nothing, so a
    /// command can ask before its own slow checks.
    pub fn check_new(root: &Path) -> Result<(), RepoError> {
        Repository::check_new_with(root, &check_owner).map(drop)
    }

    /// `check_new`, with the owner check given. `Ok(true)`: a `.bolt` that Bolt made is there
    /// already, left by a start or import that didn't finish.
    fn check_new_with(
        root: &Path,
        owner_check: &dyn Fn(&Path, bool) -> Result<(), RepoError>,
    ) -> Result<bool, RepoError> {
        let root = &resolved(root);
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
        // Its first entry would be written over the saves' diary, leaving them for clean-up.
        if found && matches!(diary_of(&root.join(".bolt"))?, Diary::Saves) {
            return Err(lost_format(root));
        }
        if let Some(outer) = enclosing_project(root, owner_check)? {
            return Err(RepoError::InsideAnotherProject {
                root: outer.to_owned(),
            });
        }
        Ok(found)
    }

    /// Starts making `root` a project, after `check_new`'s checks: the folders, and the change
    /// lock, with the leftovers in `tmp/` cleared. Nothing is committed and there is no `format`
    /// yet, so `commit_first` must follow.
    pub fn begin(root: &Path) -> Result<(Repository, ChangeLock), RepoError> {
        Repository::begin_with(root, &check_owner)
    }

    /// `begin`, with the owner check given.
    fn begin_with(
        root: &Path,
        owner_check: &dyn Fn(&Path, bool) -> Result<(), RepoError>,
    ) -> Result<(Repository, ChangeLock), RepoError> {
        let root = &resolved(root);
        let bolt = root.join(".bolt");
        let found = Repository::check_new_with(root, owner_check)?;
        let top = Dir::open_top(root).map_err(|e| {
            let shown = escape(&root.display().to_string(), false);
            let said = format!("Bolt couldn't open the project folder {shown}: {e}");
            io::Error::new(e.kind(), said)
        })?;
        if !found {
            match top.create_dir(OsStr::new(".bolt")) {
                Ok(()) => top.sync()?,
                // One made meanwhile, by another start say, is checked below as this one would be.
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
                Err(e) => return Err(e.into()),
            }
            // Some drives give a new folder another owner, or none, and every later command
            // would refuse it.
            let made = check_bolt(root, owner_check).map_err(|e| {
                if e.trust_path().is_some() {
                    RepoError::StartStopped {
                        refusal: Box::new(e),
                    }
                } else {
                    e
                }
            })?;
            if !made {
                return Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    "The .bolt folder disappeared while the project was being started.",
                )
                .into());
            }
        }
        // Opened, never made: one gone since its check must not be made again unchecked. Each
        // folder in it is made and opened through the one above, so none through a link.
        let held = Held::make(fixed(&top, root, ".bolt")?, &bolt)?;
        bolt_platform::hide_path(&bolt)?;
        let tmp = bolt.join("tmp");
        let packs = bolt.join("store").join("packs");
        let mut repo = Repository {
            root: root.to_owned(),
            store: Store::open_in(Arc::clone(&held.packs), &packs)?,
            held,
            bolt,
            tmp,
            packs,
            // As `commit_first` writes it, so taking the lock writes none before then.
            format: parse_format(FORMAT_TEXT)?,
            head: None,
            cache_key: None,
            rules: RulesRead::default(),
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
        // An interrupted run's half-written pack, as opening a project clears it.
        repo.remove_temp_files(&lock)?;
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
        let id = self.move_head(lock, packs, entry)?;
        self.write_format(lock)?;
        // Only once it is a project, so a stopped merge never leaves it unfinished.
        crate::merge::merge_after_commit(self, lock);
        Ok(id)
    }

    /// Replaces `.bolt/format` with this version's. It knows no feature but `chunks`, so that
    /// adds the one line a project of an older build can lack.
    fn write_format(&mut self, _lock: &ChangeLock) -> Result<(), RepoError> {
        let (held, name) = (&self.held, "format");
        write_atomic(&held.tmp, &held.bolt, name, FORMAT_TEXT.as_bytes())
            .map_err(|e| cant_write(self.bolt.join(name), e))?;
        self.format = parse_format(FORMAT_TEXT)?;
        Ok(())
    }

    /// Opens the project that holds `start`: the nearest folder, from `start` up, that holds one.
    /// A `.bolt` on the way that Bolt may not use is refused. With none, the nearest `.bolt` a
    /// start or import began is named, so that nobody is sent to `bolt start` over an import: one
    /// still being made, one that stopped, or one that lost only its `format`, which is damage.
    /// `start` is resolved first, so a link to the project is judged by the folder it leads to.
    pub fn discover(start: &Path) -> Result<Repository, RepoError> {
        Repository::discover_with(start, &check_owner)
    }

    /// [`Repository::discover`] for a command that changes nothing, `verify`: the leftovers in
    /// `tmp/` are left as they are, and the lock file isn't touched.
    pub fn discover_untouched(start: &Path) -> Result<Repository, RepoError> {
        Repository::search(start, &check_owner, Repository::open_untouched)
    }

    /// `discover`, with the search's owner check given (a stand-in in tests).
    fn discover_with(
        start: &Path,
        owner_check: &dyn Fn(&Path, bool) -> Result<(), RepoError>,
    ) -> Result<Repository, RepoError> {
        Repository::search(start, owner_check, Repository::open)
    }

    /// `discover`, opening the project found with `open`.
    fn search(
        start: &Path,
        owner_check: &dyn Fn(&Path, bool) -> Result<(), RepoError>,
        open: fn(&Path) -> Result<Repository, RepoError>,
    ) -> Result<Repository, RepoError> {
        let start = resolved(start);
        let mut unfinished = None;
        for folder in start.ancestors() {
            if has_project(folder, owner_check)? {
                return open(folder);
            }
            if unfinished.is_none() {
                unfinished = unfinished_here(folder);
            }
        }
        Err(unfinished.unwrap_or(RepoError::NotAProject))
    }

    /// Opens the project whose top folder is `root` (the folder holding `.bolt`).
    pub fn open(root: &Path) -> Result<Repository, RepoError> {
        let repo = Repository::open_untouched(root)?;
        repo.clean_tmp()?;
        Ok(repo)
    }

    /// [`Repository::open`] without removing the leftovers in `tmp/`, and so without trying the
    /// change lock.
    pub fn open_untouched(root: &Path) -> Result<Repository, RepoError> {
        let bolt = root.join(".bolt");
        if !check_bolt(root, &check_owner)? {
            return Err(RepoError::NotAProject);
        }
        // Held from here on, so nothing below goes through a folder swapped since the check.
        let top = Dir::open_top(&bolt).map_err(|e| unreal(&bolt, e))?;
        let max = safe_fs::MAX_FORMAT_BYTES;
        let text =
            match safe_fs::read_capped_in(&top, OsStr::new("format"), &format_path(root), max) {
                Ok(bytes) => {
                    String::from_utf8(bytes).map_err(|_| damaged("the format file is not text"))?
                }
                Err(e) if e.kind() == io::ErrorKind::NotFound => {
                    return Err(RepoError::NotAProject);
                }
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
        let state = fixed(&top, &bolt, "state")?;
        let packs_dir = Arc::new(fixed(
            &fixed(&top, &bolt, "store")?,
            &bolt.join("store"),
            "packs",
        )?);
        // The head first: its pack was in the store before the head pointed at it. One that is
        // lost is found by `current`, so that `undo --list` and `--to` can still open the project.
        let head = read_head(&state, &bolt)?.id();
        let store = Store::open_in(Arc::clone(&packs_dir), &packs)?;
        // These hold nothing worth keeping, so a missing one is made again. Only now, so that a
        // project found damaged is left as it was.
        let held = Held {
            tmp: Arc::new(make_fixed(&top, &bolt, "tmp")?),
            cache: make_fixed(&top, &bolt, "cache")?,
            bolt: top,
            state,
            packs: packs_dir,
        };
        Ok(Repository {
            root: root.to_owned(),
            bolt,
            tmp,
            packs,
            held,
            format,
            store,
            head,
            cache_key: None,
            rules: RulesRead::default(),
        })
    }

    /// Deletes leftovers of interrupted commands. A file in `tmp/` is only a leftover when no
    /// command holds the change lock; otherwise it may be a pack being written right now. The
    /// lock file is only opened to read, so a project the user can't write still opens; where it
    /// can't be, or a leftover can't be removed, the leftovers wait for a command that can.
    fn clean_tmp(&self) -> Result<(), RepoError> {
        let shown = self.lock_path();
        let file = match safe_fs::open_regular_in(&self.held.state, OsStr::new("lock"), &shown) {
            Ok(file) => file,
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::NotFound | io::ErrorKind::PermissionDenied
                ) =>
            {
                return Ok(());
            }
            Err(e) => return Err(lock_error(&shown, e)),
        };
        if file.try_lock().is_ok() {
            self.clear_tmp(&ChangeLock { _file: file });
        }
        Ok(())
    }

    /// `remove_temp_files` as far as it gets: a leftover this user can't remove (in a project
    /// shared read-only, say) is only litter, so it fails nothing.
    fn clear_tmp(&self, lock: &ChangeLock) {
        let _ = self.remove_temp_files(lock);
    }

    /// Deletes what Bolt left in `tmp/`, under the lock, so none of it is being written: its files,
    /// and the links it makes there on Windows before moving them into place, as links. Never a
    /// folder or a name it doesn't make.
    fn remove_temp_files(&self, _lock: &ChangeLock) -> Result<(), RepoError> {
        for entry in self.held.tmp.entries()? {
            let left = matches!(entry.kind, EntryType::File | EntryType::Symlink);
            if left && fsio::is_temp_name(&entry.name) {
                fsio::remove_file(&self.held.tmp, &entry.name)?;
            }
        }
        Ok(())
    }

    fn open_lock_file(&self) -> Result<File, RepoError> {
        let shown = self.lock_path();
        fsio::open_lock_file(&self.held.state, OsStr::new("lock"), &shown)
            .map_err(|e| lock_error(&shown, e))
    }

    /// `state/lock`, to name it.
    fn lock_path(&self) -> PathBuf {
        self.bolt.join("state").join("lock")
    }

    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    #[must_use]
    pub fn tmp_dir(&self) -> &Path {
        &self.tmp
    }

    /// How the project's drive compares names.
    pub fn drive(&self) -> Result<crate::drive::Drive, RepoError> {
        Ok(crate::drive::probe(self.tmp_folder())?)
    }

    /// `tmp/`, held open.
    pub(crate) fn tmp_folder(&self) -> &Dir {
        &self.held.tmp
    }

    /// `.bolt`, held open.
    pub(crate) fn bolt_folder(&self) -> &Dir {
        &self.held.bolt
    }

    /// The key that seals this project's caches, if `set_cache_key` gave one.
    #[must_use]
    pub fn cache_key(&self) -> Option<&[u8; KEY_LEN]> {
        self.cache_key.as_ref()
    }

    /// Seals caches with `key` from now on: the user's own, which the command line reads, so
    /// that they last from one command to the next. Until then no cache is read or written: what
    /// a key for one run seals, no later run trusts, and writing it would replace a good cache.
    pub fn set_cache_key(&mut self, key: [u8; KEY_LEN]) {
        self.cache_key = Some(key);
    }

    /// `state/`, held open.
    pub(crate) fn state_folder(&self) -> &Dir {
        &self.held.state
    }

    /// Where `state/` is, to name files in it.
    pub(crate) fn state_dir(&self) -> PathBuf {
        self.bolt.join("state")
    }

    /// `cache/`, held open.
    pub(crate) fn cache_folder(&self) -> &Dir {
        &self.held.cache
    }

    /// Where `cache/` is, to name files in it.
    pub(crate) fn cache_dir(&self) -> PathBuf {
        self.bolt.join("cache")
    }

    #[must_use]
    pub fn packs_dir(&self) -> &Path {
        &self.packs
    }

    #[must_use]
    pub fn store(&self) -> &Store {
        &self.store
    }

    pub(crate) fn store_mut(&mut self) -> &mut Store {
        &mut self.store
    }

    /// The project's rules as the last `lock_for_change` read them; Bolt's own before that.
    #[must_use]
    pub fn rules(&self) -> &RulesRead {
        &self.rules
    }

    /// Reads the project's `bolt.toml` now, for a command that takes no lock.
    #[must_use]
    pub fn read_rules(&self) -> RulesRead {
        match Dir::open_top(&self.root) {
            Ok(top) => rules::read(&top),
            Err(e) => RulesRead {
                skipped: Some(format!(
                    "Bolt couldn't open the project folder to read it: {}",
                    escape(&e.to_string(), false)
                )),
                ..RulesRead::default()
            },
        }
    }

    /// `store/packs`, held open.
    pub(crate) fn packs_folder(&self) -> &Arc<Dir> {
        &self.held.packs
    }

    #[must_use]
    pub fn signatures_allowed(&self) -> bool {
        self.format.features.contains("signatures")
    }

    /// Build packs only while holding the `ChangeLock`: `open` cleans `tmp/` whenever the lock
    /// is free, which would delete a pack being built without it.
    pub fn new_packs(&self) -> Result<PackSet, RepoError> {
        Ok(PackSet::new(&self.held.tmp, &self.held.packs, &self.packs)?)
    }

    /// The newest diary entry's ID. Readers never take the lock: `state/head` is replaced whole.
    /// `HeadLost` when it is missing or isn't an entry's ID.
    pub fn head(&self) -> Result<Id, RepoError> {
        match read_head(&self.held.state, &self.bolt)? {
            Head::At(id) => Ok(id),
            Head::Missing => Err(head_lost("is missing")),
            Head::Garbled => Err(head_lost("isn't a diary entry's ID and a newline")),
        }
    }

    /// The newest diary entry this handle can see: the one when it was opened, or when it
    /// last took the change lock or committed. Read from the store. Every command reads it, so
    /// every command stops at a lost head, `HeadLost`.
    pub fn current(&self) -> Result<(Id, DiaryEntry), RepoError> {
        let id = match self.head {
            Some(id) => id,
            None => self.head()?,
        };
        head_entry(&self.store, id)
    }

    /// The diary entry `id`. One the store lacks, holds as another kind of record, or can't
    /// decode, is damage to the diary, named.
    pub fn entry(&self, id: &Id) -> Result<DiaryEntry, RepoError> {
        entry_in(&self.store, id)
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
        let locking = |e| naming(&self.lock_path(), "lock", e);
        match file.try_lock() {
            Ok(()) => {}
            Err(TryLockError::WouldBlock) => {
                // Best effort: the text is only for the message.
                let shown = self.bolt.join("state").join("lock-info");
                let (name, max) = (OsStr::new("lock-info"), safe_fs::MAX_LOCK_INFO_BYTES);
                let info = safe_fs::read_capped_in(&self.held.state, name, &shown, max)
                    .ok()
                    .and_then(|bytes| String::from_utf8(bytes).ok())
                    .unwrap_or_default();
                on_wait(&info);
                file.lock().map_err(locking)?;
            }
            Err(TryLockError::Error(e)) => return Err(locking(e).into()),
        }
        let lock = ChangeLock { _file: file };
        // Opening the project couldn't clear `tmp/` where a drive locks only a file open for
        // writing.
        self.clear_tmp(&lock);
        let started = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_millis());
        let info = format!(
            "pid {}\ncommand {command}\nstarted {started}\n",
            std::process::id()
        );
        let (held, name) = (&self.held, "lock-info");
        write_atomic(&held.tmp, &held.state, name, info.as_bytes())
            .map_err(|e| cant_write(self.state_dir().join(name), e))?;
        // Before anything is stored: builds from before chunk lists must refuse this project now.
        if !self.format.features.contains(CHUNKS) {
            self.write_format(&lock)?;
        }
        // Another command may have committed since this handle was opened. A lost head stops
        // the command at `current`, unless it is the `undo --to` that picks one up.
        self.head = read_head(&self.held.state, &self.bolt)?.id();
        self.store.refresh()?;
        // Once per command, under the lock, before anything is stored.
        self.rules = self.read_rules();
        self.store.set_chunk_from(self.rules.rules.chunk_files_over);
        Ok(lock)
    }

    /// Stores `entry` in the last of `packs`, moves that pack into the store, registers all of
    /// them, then replaces `state/head`. The command has happened exactly when the head is
    /// replaced, so nothing after that fails it.
    /// Then merges the small packs; problems doing so are only warnings, kept with the store's.
    pub fn commit(
        &mut self,
        lock: &ChangeLock,
        packs: PackSet,
        entry: &DiaryEntry,
    ) -> Result<Id, RepoError> {
        let id = self.move_head(lock, packs, entry)?;
        crate::merge::merge_after_commit(self, lock);
        Ok(id)
    }

    /// `commit`, without the merge. A finished switch's journal, left when removing it failed, is
    /// removed first: once `head` moves on, nothing could tell it was finished.
    fn move_head(
        &mut self,
        lock: &ChangeLock,
        packs: PackSet,
        entry: &DiaryEntry,
    ) -> Result<Id, RepoError> {
        if let Some(journal) = Journal::read(self)?
            && self.head == Some(journal.target)
        {
            Journal::remove_before_head(self, lock)?;
        }
        let id = self.store_entry(lock, packs, entry)?;
        fault_point("commit:before-head")?;
        self.move_head_to(lock, id)?;
        Ok(id)
    }

    /// Stores `entry` in the last of `packs` and moves them all into the store, leaving
    /// `state/head` as it is: a switch stores its entry before it changes the folder.
    pub fn store_entry(
        &mut self,
        lock: &ChangeLock,
        mut packs: PackSet,
        entry: &DiaryEntry,
    ) -> Result<Id, RepoError> {
        let bytes = entry.encode();
        // Refuse an entry that would not read back before storing it.
        DiaryEntry::decode(&bytes)?;
        let id = packs.add_record(ObjectKind::DiaryEntry, &bytes)?;
        self.store_packs(lock, packs)?;
        Ok(id)
    }

    /// Moves `packs` into the store, for what a command must read back before its diary entry is
    /// ready. Nothing names what they hold until an entry does.
    pub(crate) fn store_packs(
        &mut self,
        _lock: &ChangeLock,
        packs: PackSet,
    ) -> Result<(), RepoError> {
        for path in packs.finish()? {
            self.store.register(&path);
        }
        Ok(())
    }

    /// Points `state/head` at `id`, a diary entry already stored: how a command takes effect.
    /// Once the new head is in place, a flush of `state/` that fails is only a warning.
    pub fn move_head_to(&mut self, _lock: &ChangeLock, id: Id) -> Result<(), RepoError> {
        let (name, head) = ("head", format!("{id}\n"));
        let done = fsio::write_head(&self.held.tmp, &self.held.state, name, head.as_bytes())
            .map_err(|e| cant_write(self.state_dir().join(name), e))?;
        self.head = Some(id);
        if let Done::NotFlushed(e) = done {
            self.store.note(Warning::new(
                warning::HEAD_NOT_FLUSHED,
                format!(
                    "The command happened, but Bolt couldn't flush .bolt/state to the disk after \
                     it ({}), so a power cut soon after could undo it.",
                    e.to_string().trim_end_matches('.')
                ),
            ));
        }
        Ok(())
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

    /// The format Bolt wrote before chunk lists.
    const OLDER_FORMAT: &str = "bolt-repository 1\nhash blake3\n";

    #[test]
    fn format_file() {
        let chunks = BTreeSet::from(["chunks".to_owned()]);
        assert_eq!(
            FORMAT_TEXT,
            "bolt-repository 1\nhash blake3\nfeature chunks\n"
        );
        assert_eq!(parse_format(FORMAT_TEXT).unwrap().features, chunks);
        assert!(parse_format(OLDER_FORMAT).unwrap().features.is_empty());
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
            (
                "bolt-repository 1\nhash blake3\nfeature chunks\nfeature teleport\n",
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
            "bolt-repository 1\nhash blake3\nfeature chunks\nfeature chunks\n",
            "bolt-repository 1\nhash blake3\nfeature chunks\n\n",
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
        let dir = tempdir();
        let bolt = dir.path().join(".bolt");
        for sub in ["store/packs", "state", "cache", "tmp"] {
            std::fs::create_dir_all(bolt.join(sub)).unwrap();
        }
        std::fs::write(bolt.join("format"), FORMAT_TEXT).unwrap();
        // As `start` leaves it: opening a project never makes it.
        std::fs::write(bolt.join("state").join("lock"), b"").unwrap();
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

    /// A junk pack is passed over, with a warning, and only a lookup that finds nothing fails.
    /// `00.pack` is searched first on a cold store, `zz.pack` last.
    #[test]
    fn a_junk_pack_is_damage_only_where_a_lookup_reaches_it() {
        for junk_name in ["00.pack", "zz.pack"] {
            let (dir, mut repo) = project();
            let head = commit(&mut repo, &entry(1)).unwrap();
            drop(repo);
            let bolt = dir.path().join(".bolt");
            // Over 1 MiB, so reading it searches every pack.
            let big = vec![5u8; 2 << 20];
            let mut w = crate::pack::PackWriter::create(&bolt.join("tmp")).unwrap();
            let big_id = w.add_content(&big).unwrap();
            w.finish(&bolt.join("store/packs")).unwrap();
            let junk = bolt.join("store/packs").join(junk_name);
            std::fs::write(&junk, b"not what Bolt wrote").unwrap();

            let repo = Repository::open(dir.path()).unwrap();
            let found = repo.store().read_record(&head, ObjectKind::DiaryEntry);
            assert!(found.is_ok(), "{junk_name}: {found:?}");
            let mut out = Vec::new();
            let read = repo.store().copy_content(&big_id, &mut out);
            assert!(read.is_ok() && out == big, "{junk_name}: {read:?}");
            let warnings = repo.store().warnings();
            assert!(
                warnings
                    .iter()
                    .any(|w| w.kind == "damaged_pack" && w.message.contains(junk_name)),
                "{junk_name}: {warnings:?}"
            );
            let missing = repo
                .store()
                .read_record(&Id([0x11; 32]), ObjectKind::Save)
                .map_err(RepoError::from);
            assert!(
                matches!(missing, Err(RepoError::Damaged { .. })),
                "{junk_name}: {missing:?}"
            );
            assert_eq!(std::fs::read(&junk).unwrap(), b"not what Bolt wrote");
            for name in ["tmp", "cache"] {
                assert!(safe_fs::check_real_dir(&bolt.join(name)).is_ok(), "{name}");
            }
        }
    }

    /// The hint for damage says nothing was changed, so a missing `tmp` or `cache` isn't made
    /// before the project is found damaged.
    #[test]
    fn a_damaged_project_is_left_as_found() {
        // Not a damaged pack: packs are opened only when first searched, so opening the project
        // doesn't find one. Nor a garbled head: that is a lost head, which the project is opened
        // to pick up.
        for spoil in ["format"] {
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
            folder_change: None,
        }
    }

    fn commit(repo: &mut Repository, e: &DiaryEntry) -> Result<Id, RepoError> {
        let lock = repo.lock_for_change("save", &mut |_| {})?;
        let packs = repo.new_packs()?;
        repo.commit(&lock, packs, e)
    }

    #[test]
    fn init_then_discover_from_a_subfolder() {
        let dir = tempdir();
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
        let outside = tempdir();
        assert!(matches!(
            Repository::discover(outside.path()),
            Err(RepoError::NotAProject)
        ));
    }

    #[test]
    fn start_writes_neither_record() {
        let dir = tempdir();
        let repo = Repository::init(dir.path(), &entry(0).who).unwrap();
        let state = repo.current().unwrap().1.state;
        assert!(state.tags.is_none() && state.server_lines.is_none());
        assert!(repo.tag_list(&state).unwrap().tags.is_empty());
        assert!(repo.server_lines(&state).unwrap().servers.is_empty());
        for pack in std::fs::read_dir(repo.packs_dir()).unwrap() {
            for e in PackReader::open(&pack.unwrap().path()).unwrap().entries() {
                assert!(!matches!(
                    e.unwrap().kind,
                    ObjectKind::TagList | ObjectKind::ServerLines
                ));
            }
        }
    }

    /// Builds from before chunk lists can't read them safely, so the first command that changes an
    /// older project says the project may hold them, before it stores anything.
    #[test]
    fn a_changing_command_adds_feature_chunks_before_storing_anything() {
        let dir = tempdir();
        drop(Repository::init(dir.path(), &entry(0).who).unwrap());
        let format = format_path(dir.path());
        std::fs::write(&format, OLDER_FORMAT).unwrap();
        let mut repo = Repository::open(dir.path()).unwrap();
        let packs = || {
            std::fs::read_dir(dir.path().join(".bolt/store/packs"))
                .unwrap()
                .count()
        };
        let before = packs();
        let lock = repo.lock_for_change("save", &mut |_| {}).unwrap();
        assert_eq!(std::fs::read_to_string(&format).unwrap(), FORMAT_TEXT);
        assert_eq!(packs(), before);
        let (head, mut next) = repo.current().unwrap();
        next.parent = Some(head);
        let stored = repo.new_packs().unwrap();
        repo.commit(&lock, stored, &next).unwrap();
        drop(lock);
        let mut reopened = Repository::open(dir.path()).unwrap();
        assert!(reopened.format.features.contains("chunks"));
        drop(reopened.lock_for_change("save", &mut |_| {}).unwrap());
        assert_eq!(std::fs::read_to_string(&format).unwrap(), FORMAT_TEXT);
    }

    #[test]
    fn a_command_that_only_reads_leaves_format_alone() {
        let dir = tempdir();
        drop(Repository::init(dir.path(), &entry(0).who).unwrap());
        let format = format_path(dir.path());
        std::fs::write(&format, OLDER_FORMAT).unwrap();
        let repo = Repository::discover(dir.path()).unwrap();
        assert!(repo.format.features.is_empty());
        let (_, current) = repo.current().unwrap();
        let save = repo
            .store()
            .save(&current.state.lines["main"], false)
            .unwrap();
        repo.store().listing(&save.root).unwrap();
        drop(repo);
        assert_eq!(std::fs::read_to_string(&format).unwrap(), OLDER_FORMAT);
    }

    #[test]
    fn the_records_a_state_points_to_are_read_back() {
        let dir = tempdir();
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
        let dir = tempdir();
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
                matches!(repo.head(), Err(RepoError::HeadLost { .. })),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn a_head_naming_a_damaged_or_undecodable_entry_is_lost() {
        let (dir, mut repo) = project();
        // The only copy of an entry, failing its check.
        let bytes = entry(1).encode();
        let flavor = ObjectKind::DiaryEntry.flavor().unwrap();
        let damaged = crate::id::hash_record(flavor, &bytes);
        let planted = [(ObjectKind::DiaryEntry, bytes.as_slice())];
        crate::test_support::plant_damaged_copies(repo.packs_dir(), &planted);
        // A record that checks out but isn't an entry.
        let lock = repo.lock_for_change("test", &mut |_| {}).unwrap();
        let mut packs = repo.new_packs().unwrap();
        let undecodable = packs
            .add_record(ObjectKind::DiaryEntry, b"not an entry")
            .unwrap();
        repo.store_packs(&lock, packs).unwrap();
        drop((lock, repo));
        for id in [damaged, undecodable] {
            std::fs::write(dir.path().join(".bolt/state/head"), format!("{id}\n")).unwrap();
            let read = Repository::open(dir.path()).unwrap().current();
            match read {
                Err(e @ RepoError::HeadLost { .. }) => {
                    let why = format!(
                        "names diary entry {}, which can't be read (",
                        id.short_hex()
                    );
                    assert!(e.to_string().contains(&why), "{e}");
                    assert!(e.hint().unwrap().starts_with("Your saves are safe"), "{e}");
                }
                other => panic!("{id}: {other:?}"),
            }
        }
    }

    /// A `.bolt` without `format` is a start or import that stopped, judged as before: a head
    /// there that isn't an entry's ID is damage, not a lost head.
    #[test]
    fn a_garbled_head_without_format_is_still_damage() {
        let who = entry(0).who;
        let dir = tempdir();
        drop(Repository::init(dir.path(), &who).unwrap());
        std::fs::remove_file(dir.path().join(".bolt/format")).unwrap();
        std::fs::write(dir.path().join(".bolt/state/head"), b"not an ID\n").unwrap();
        let found = Repository::discover(dir.path()).map(drop);
        let started = Repository::init(dir.path(), &who).map(drop);
        for refused in [found, started] {
            match refused {
                Err(e @ RepoError::Damaged { .. }) => {
                    let why = "state/head is not a lowercase fingerprint and a newline";
                    assert!(e.to_string().contains(why), "{e}");
                }
                other => panic!("{other:?}"),
            }
        }
    }

    /// A start or import that stopped before `format` is named to every command, and `start`
    /// takes over what a start stores, but never the files an import stored.
    #[test]
    fn an_unfinished_project_is_named_and_kept_for_the_import_that_began_it() {
        let who = entry(0).who;
        let begun = |content: bool| {
            let dir = tempdir();
            let (repo, lock) = Repository::begin(dir.path()).unwrap();
            let mut packs = repo.new_packs().unwrap();
            if content {
                packs.add_content(b"a file an import stored").unwrap();
            } else {
                let empty = Listing::empty().encode();
                packs.add_record(ObjectKind::FolderListing, &empty).unwrap();
            }
            packs.finish().unwrap();
            drop((lock, repo));
            dir
        };
        let dir = begun(true);
        let sub = dir.path().join("sub");
        std::fs::create_dir(&sub).unwrap();
        // It holds what the import stored, so nobody is told to delete it; running the import
        // again finishes from it.
        match Repository::discover(&sub) {
            Err(e @ RepoError::Unfinished { stored: true, .. }) => {
                assert!(!e.hint().unwrap().contains("delete"), "{e}");
                let at_root =
                    matches!(&e, RepoError::Unfinished { root, .. } if root == dir.path());
                assert!(at_root, "{e:?}");
            }
            other => panic!("{:?}", other.map(|_| ())),
        }
        match Repository::init(dir.path(), &who) {
            Err(e @ RepoError::ImportUnfinished) => {
                assert!(!e.hint().unwrap().contains("delete"), "{e}");
            }
            other => panic!("{:?}", other.map(|_| ())),
        }
        assert!(!dir.path().join(".bolt/format").exists());
        // Another tool's folder is nothing of Bolt's to finish.
        std::fs::write(dir.path().join(".bolt/prompt"), b"").unwrap();
        assert!(matches!(
            Repository::discover(&sub),
            Err(RepoError::NotAProject)
        ));
        let dir = begun(false);
        Repository::init(dir.path(), &who).unwrap();

        // Only a folder that holds nothing is offered for deleting.
        let dir = tempdir();
        drop(Repository::begin(dir.path()).unwrap());
        match Repository::discover(dir.path()) {
            Err(e @ RepoError::Unfinished { stored: false, .. }) => {
                assert!(e.hint().unwrap().contains("or delete"), "{e}");
            }
            other => panic!("{:?}", other.map(|_| ())),
        }

        // Nor once its one entry is written, which holds all the import made.
        let dir = tempdir();
        let (mut repo, lock) = Repository::begin(dir.path()).unwrap();
        let mut packs = repo.new_packs().unwrap();
        packs.add_content(b"a file an import stored").unwrap();
        repo.commit(&lock, packs, &entry(0)).unwrap();
        drop((lock, repo));
        match Repository::discover(dir.path()) {
            Err(e @ RepoError::Unfinished { stored: true, .. }) => {
                assert!(!e.hint().unwrap().contains("delete"), "{e}");
            }
            other => panic!("{:?}", other.map(|_| ())),
        }
    }

    /// While a start or import still holds the lock, a command says so instead of calling the
    /// project unfinished; and a project that lost only its `format` after it had saves is
    /// damaged, never one to delete.
    #[test]
    fn a_project_being_made_or_missing_its_format_is_not_called_unfinished() {
        let dir = tempdir();
        let (repo, lock) = Repository::begin(dir.path()).unwrap();
        match Repository::discover(dir.path()) {
            Err(e @ RepoError::BeingMade { .. }) => {
                assert!(e.hint().unwrap().contains("once it has finished"), "{e}");
            }
            other => panic!("{:?}", other.map(|_| ())),
        }
        drop((lock, repo));
        assert!(matches!(
            Repository::discover(dir.path()),
            Err(RepoError::Unfinished { .. })
        ));

        let who = entry(0).who;
        let dir = tempdir();
        let mut repo = Repository::init(dir.path(), &who).unwrap();
        let format = dir.path().join(".bolt/format");
        // Only its first entry: a start that stopped as it wrote `format` looks the same.
        std::fs::remove_file(&format).unwrap();
        assert!(matches!(
            Repository::discover(dir.path()),
            Err(RepoError::Unfinished { stored: true, .. })
        ));
        std::fs::write(&format, FORMAT_TEXT).unwrap();
        let mut second = entry(1);
        second.parent = Some(repo.head().unwrap());
        commit(&mut repo, &second).unwrap();
        drop(repo);
        std::fs::remove_file(&format).unwrap();
        match Repository::discover(dir.path()) {
            Err(e @ RepoError::Damaged { .. }) => {
                assert!(
                    e.to_string().contains("has saves, but no .bolt/format"),
                    "{e}"
                );
                assert!(e.hint().unwrap().contains("Ask for help"), "{e}");
            }
            other => panic!("{:?}", other.map(|_| ())),
        }
        // With its head lost too, the saves' entries are still in the packs: it is damage all the
        // same, and no start or import makes the project again over them.
        std::fs::remove_file(dir.path().join(".bolt/state/head")).unwrap();
        assert!(matches!(
            Repository::discover(dir.path()),
            Err(RepoError::Damaged { .. })
        ));
        let refused = Repository::init(dir.path(), &who);
        assert!(
            matches!(refused, Err(RepoError::Damaged { .. })),
            "{:?}",
            refused.map(|_| ())
        );
        assert!(!dir.path().join(".bolt/state/head").exists());
    }

    /// The lock is never probed through a `state` that is a link: such a `.bolt` is damage, even
    /// where the link leads to a lock some command holds.
    #[test]
    fn a_linked_state_is_damage_before_its_lock_is_probed() {
        let dir = tempdir();
        let bolt = dir.path().join(".bolt");
        std::fs::create_dir_all(bolt.join("store/packs")).unwrap();
        let elsewhere = dir.path().join("elsewhere");
        std::fs::create_dir(&elsewhere).unwrap();
        let outside = Dir::open_top(&elsewhere).unwrap();
        let lock = elsewhere.join("lock");
        let held = fsio::open_lock_file(&outside, OsStr::new("lock"), &lock).unwrap();
        held.lock().unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&elsewhere, bolt.join("state")).unwrap();
        #[cfg(windows)]
        crate::test_support::junction(&bolt.join("state"), &elsewhere);
        let found = Repository::discover(dir.path());
        assert!(
            matches!(found, Err(RepoError::Damaged { .. })),
            "{:?}",
            found.map(|_| ())
        );
        drop(held);
    }

    /// A run that finishes an interrupted one starts without the half-written pack it left.
    #[test]
    fn begin_clears_what_an_interrupted_run_left_in_tmp() {
        let dir = tempdir();
        drop(Repository::begin(dir.path()).unwrap());
        let tmp = dir.path().join(".bolt/tmp");
        let litter = tmp.join("tmp-0123456789abcdef");
        std::fs::write(&litter, b"half a pack").unwrap();
        std::fs::write(tmp.join("not-bolts"), b"x").unwrap();
        let (repo, _lock) = Repository::begin(dir.path()).unwrap();
        assert!(!litter.exists());
        assert!(tmp.join("not-bolts").exists());
        drop(repo);
    }

    #[test]
    fn open_needs_a_format_file_and_cleans_tmp() {
        let dir = tempdir();
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
            let dir = tempdir();
            Repository::init(dir.path(), &who).unwrap();
            let link = folder
                .split('/')
                .fold(dir.path().to_owned(), |path, name| path.join(name));
            // The real folder moves outside, so the project would still work through the link.
            let outside = tempdir();
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

    /// A fixed folder swapped for a link once the project is open is never written through: the
    /// command goes on in the folder it checked, and the next one refuses the link.
    #[test]
    fn store_packs_swapped_for_a_link_after_the_shape_check_is_refused() {
        let (dir, mut repo) = project();
        let store = dir.path().join(".bolt").join("store");
        let outside = tempdir();
        std::fs::rename(store.join("packs"), store.join("checked")).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(outside.path(), store.join("packs")).unwrap();
        #[cfg(windows)]
        crate::test_support::junction(&store.join("packs"), outside.path());
        let id = commit(&mut repo, &entry(1)).unwrap();
        assert_eq!(std::fs::read_dir(outside.path()).unwrap().count(), 0);
        assert_eq!(std::fs::read_dir(store.join("checked")).unwrap().count(), 1);
        assert_eq!(repo.head().unwrap(), id);
        match Repository::open(dir.path()) {
            Err(RepoError::Damaged { detail }) => assert!(detail.contains("packs"), "{detail}"),
            other => panic!("{:?}", other.map(|_| ())),
        }
    }

    /// The change lock is taken through the held `state` folder: one swapped for a link once the
    /// project is open gets no lock where the link points, so no command can take a lock there
    /// while another holds the real one.
    #[test]
    fn state_swapped_for_a_link_after_the_shape_check_gets_no_lock_outside() {
        let (dir, mut repo) = project();
        let bolt = dir.path().join(".bolt");
        let outside = tempdir();
        std::fs::rename(bolt.join("state"), bolt.join("checked")).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(outside.path(), bolt.join("state")).unwrap();
        #[cfg(windows)]
        crate::test_support::junction(&bolt.join("state"), outside.path());
        let lock = repo.lock_for_change("save", &mut |_| {}).unwrap();
        assert_eq!(std::fs::read_dir(outside.path()).unwrap().count(), 0);
        assert!(bolt.join("checked").join("lock").is_file());
        drop(lock);
    }

    #[test]
    fn clean_tmp_removes_only_bolts_own_names() {
        let (dir, _repo) = project();
        let tmp = dir.path().join(".bolt").join("tmp");
        let (_, made) = fsio::create_temp(&Dir::open_top(&tmp).unwrap()).unwrap();
        let made = tmp.join(made);
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

    /// A project shared read-only, say: reading needs no write to `state/lock` or `tmp/`, and a
    /// command that would change the project names the file it can't open.
    #[test]
    fn a_project_the_user_cannot_write_still_reads() {
        let dir = tempdir();
        let (head, _) = Repository::init(dir.path(), &entry(0).who)
            .unwrap()
            .current()
            .unwrap();
        let bolt = dir.path().join(".bolt");
        let (state, tmp) = (bolt.join("state"), bolt.join("tmp"));
        let lock = state.join("lock");
        // A stopped command's leftover, which this user can't remove.
        let left = tmp.join(bolt_platform::temp_name());
        std::fs::write(&left, b"half a pack").unwrap();
        #[cfg(windows)]
        let held = {
            use std::os::windows::fs::OpenOptionsExt;
            std::fs::OpenOptions::new()
                .read(true)
                .share_mode(0)
                .open(&left)
                .unwrap()
        };
        // The folders too, mode 0555 then on Unix (Windows ignores a folder's read-only flag).
        let was = [&lock, &state, &tmp].map(|p| (p, std::fs::metadata(p).unwrap().permissions()));
        let read_only = |on: bool| {
            for (path, was) in &was {
                let mut now = was.clone();
                if on {
                    now.set_readonly(true);
                }
                std::fs::set_permissions(path, now).unwrap();
            }
        };
        read_only(true);
        // root writes anyway, so there is nothing to test.
        let root_user = std::fs::OpenOptions::new().write(true).open(&lock).is_ok();
        let opened = Repository::open(dir.path()).map(|mut repo| {
            let refused = repo.lock_for_change("save", &mut |_| {}).map(drop);
            (repo, refused)
        });
        read_only(false);
        #[cfg(windows)]
        drop(held);
        if root_user {
            return;
        }
        let (repo, refused) = opened.unwrap();
        assert_eq!(repo.current().unwrap().0, head);
        assert!(left.exists());
        let e = refused.unwrap_err();
        assert!(e.to_string().contains(&lock.display().to_string()), "{e}");
    }

    /// Where a drive locks only a file open for writing, opening a project leaves `tmp/` as it
    /// is; taking the lock to change the project clears it.
    #[test]
    fn taking_the_lock_clears_what_stopped_commands_left_in_tmp() {
        let (dir, mut repo) = project();
        let left = dir
            .path()
            .join(".bolt/tmp")
            .join(bolt_platform::temp_name());
        std::fs::write(&left, b"half a pack").unwrap();
        let lock = repo.lock_for_change("save", &mut |_| {}).unwrap();
        assert!(!left.exists());
        drop(lock);
    }

    /// Windows makes a link in `tmp/` before moving it into place, so a run that stops can leave
    /// one: it goes as a link, file and folder links alike, and what it points to stays. A link
    /// under another name stays too.
    #[test]
    fn leftover_temporary_links_go_and_what_they_point_to_stays() {
        let (dir, repo) = project();
        drop(repo);
        let tmp = dir.path().join(".bolt").join("tmp");
        let outside = tempdir();
        let (file, folder) = (outside.path().join("file"), outside.path().join("folder"));
        std::fs::write(&file, b"x").unwrap();
        std::fs::create_dir(&folder).unwrap();
        let temp = || tmp.join(bolt_platform::temp_name());
        let mine = tmp.join("mine");
        let mut links = Vec::new();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&folder, &mine).unwrap();
            for target in [&file, &folder] {
                let link = temp();
                std::os::unix::fs::symlink(target, &link).unwrap();
                links.push(link);
            }
        }
        #[cfg(windows)]
        {
            crate::test_support::junction(&mine, &folder);
            let junction = temp();
            crate::test_support::junction(&junction, &folder);
            links.push(junction);
            let (file_link, folder_link) = (temp(), temp());
            match std::os::windows::fs::symlink_file(&file, &file_link) {
                Ok(()) => {
                    std::os::windows::fs::symlink_dir(&folder, &folder_link).unwrap();
                    links.extend([file_link, folder_link]);
                }
                Err(e) => crate::test_support::skip_without_link_right(e),
            }
        }
        Repository::open(dir.path()).unwrap();
        for link in &links {
            let left = std::fs::symlink_metadata(link);
            assert!(left.is_err(), "{} was kept", link.display());
        }
        assert!(std::fs::symlink_metadata(&mine).is_ok());
        assert!(file.exists() && folder.exists());
    }

    #[test]
    fn owner_rule() {
        let dir = tempdir();
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
        // Listing the project folder trusts both, as Git's safe.directory does; written with `..`
        // it trusts nothing, and nor does a folder above it or beside it.
        for owners in refused() {
            assert!(allows(owners, std::slice::from_ref(&root)).is_ok());
        }
        for owners in refused() {
            assert!(allows(owners, &[sibling.join("..").join("project")]).is_err());
        }
        for owners in refused() {
            assert!(allows(owners, &[dir.path().to_owned(), sibling.clone()]).is_err());
        }
    }

    /// A refusal that needs the user's `safe-folders` is replaced by the mistake in it, with what
    /// to do about that.
    #[test]
    fn a_misplaced_safe_folders_line_is_what_the_user_is_told() {
        let dir = tempdir();
        let misplaced = || -> Result<Vec<PathBuf>, ConfigError> {
            Err(ConfigError::NotAtTop {
                key: "safe-folders",
                path: PathBuf::from("config.toml"),
            })
        };
        let told =
            check_owner_with(dir.path(), false, &|_| Ok(Owner::Other), &misplaced).unwrap_err();
        assert_eq!(
            told.to_string(),
            "safe-folders must be at the top of config.toml, before any [section]"
        );
        assert!(told.hint().unwrap().starts_with("Move that line above"));
    }

    #[test]
    fn administrators_count_only_when_elevated() {
        let dir = tempdir();
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
        let dir = tempdir();
        let broken = || -> Result<Vec<PathBuf>, ConfigError> {
            Err(ConfigError::Invalid {
                path: PathBuf::from("config.toml"),
                detail: "broken".into(),
            })
        };
        let owned_by = |owner: Owner| check_owner_with(dir.path(), false, &|_| Ok(owner), &broken);
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
        let dir = tempdir();
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

    /// A temporary folder named as `discover` names it: on macOS `/var` is a link to `/private/var`.
    fn tempdir() -> tempfile::TempDir {
        tempfile::tempdir_in(resolved(&std::env::temp_dir())).unwrap()
    }

    /// The real owner check, with the answers no test machine can give: `owner(path)` owns each
    /// path that exists, and the user's safe-folders are `safe`.
    fn checked_by(
        owner: impl Fn(&Path) -> Owner,
        safe: &[PathBuf],
    ) -> impl Fn(&Path, bool) -> Result<(), RepoError> {
        move |root, bolt_seen| {
            let owner_of = |path: &Path| std::fs::symlink_metadata(path).map(|_| owner(path));
            check_owner_with(root, bolt_seen, &owner_of, &|| Ok(safe.to_vec()))
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
    fn all_owned_by(owner: Owner) -> impl Fn(&Path, bool) -> Result<(), RepoError> {
        checked_by(move |_| owner, &[])
    }

    /// Below a project it may not use, `bolt start` looks further up, and makes the project if
    /// none above is one the user may use.
    #[test]
    fn start_looks_past_an_enclosing_project_it_may_not_use() {
        for drive_says in [Owner::Other, Owner::Unknown] {
            let dir = tempdir();
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
            let dir = tempdir();
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
        let dir = tempdir();
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
                let dir = tempdir();
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
        let dir = tempdir();
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
            let dir = tempdir();
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
            let dir = tempdir();
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
        let dir = tempdir();
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
        let check = |folder: &Path, seen| {
            // Someone removes the new `.bolt` while its owner is asked.
            if folder == root && bolt.is_dir() {
                std::fs::remove_dir(&bolt).unwrap();
            }
            rule(folder, seen)
        };
        let started = Repository::init_with(&root, &entry(0).who, &check);
        assert!(started.is_err(), "{:?}", started.map(|_| ()));
        assert!(!bolt.exists() && !format_path(&root).exists());
    }

    #[test]
    fn start_names_a_project_folder_it_cannot_open() {
        let dir = tempdir();
        let root = dir.path().join("root");
        std::fs::create_dir(&root).unwrap();
        // Gone while its owner is asked, so it can't be opened after the checks.
        let check = |folder: &Path, _seen: bool| {
            if folder == root && root.exists() {
                std::fs::remove_dir(&root).unwrap();
            }
            Ok(())
        };
        let e = Repository::init_with(&root, &entry(0).who, &check)
            .err()
            .unwrap();
        assert!(e.to_string().contains(&root.display().to_string()), "{e}");
    }

    /// A link that appears where `bolt start` is about to make `.bolt` is never followed.
    #[test]
    fn start_refuses_a_bolt_link_that_appears_while_it_starts() {
        let dir = tempdir();
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
        let check = |folder: &Path, seen| {
            if folder == root && std::fs::symlink_metadata(&bolt).is_err() {
                #[cfg(unix)]
                std::os::unix::fs::symlink(&target, &bolt).unwrap();
                #[cfg(windows)]
                crate::test_support::junction(&bolt, &target);
            }
            rule(folder, seen)
        };
        let started = Repository::init_with(&root, &entry(0).who, &check);
        assert!(
            matches!(started, Err(RepoError::Damaged { .. })),
            "{:?}",
            started.map(|_| ())
        );
        assert!(!target.exists());
    }

    /// A link put in an unfinished `.bolt` once its shape was checked is refused, and nothing is
    /// made where it points.
    #[test]
    fn start_never_makes_a_fixed_folder_through_a_link() {
        for name in ["store", "state", "cache", "tmp"] {
            let dir = tempdir();
            let (root, target) = (dir.path().join("root"), dir.path().join("target"));
            std::fs::create_dir_all(root.join(".bolt")).unwrap();
            std::fs::create_dir(&target).unwrap();
            // A `.bolt` above, with no `format`: the search for a project around this one asks
            // its owner after this one was checked, and the link appears then.
            std::fs::create_dir(dir.path().join(".bolt")).unwrap();
            let link = root.join(".bolt").join(name);
            let check = |folder: &Path, _seen: bool| {
                if folder == dir.path() && std::fs::symlink_metadata(&link).is_err() {
                    #[cfg(unix)]
                    std::os::unix::fs::symlink(&target, &link).unwrap();
                    #[cfg(windows)]
                    crate::test_support::junction(&link, &target);
                }
                Ok(())
            };
            let started = Repository::init_with(&root, &entry(0).who, &check);
            assert!(
                matches!(started, Err(RepoError::Damaged { .. })),
                "{name}: {:?}",
                started.map(|_| ())
            );
            let made: Vec<_> = std::fs::read_dir(&target).unwrap().collect();
            assert!(made.is_empty(), "{name}: {made:?}");
        }
    }

    /// With the inside unreadable, any look inside a `.bolt` the user may not use would fail and
    /// change the answer.
    #[cfg(unix)]
    #[test]
    fn a_foreign_bolt_is_never_looked_inside() {
        use std::cell::RefCell;
        use std::os::unix::fs::PermissionsExt;
        let dir = tempdir();
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
        let theirs = |folder: &Path, seen| {
            asked.borrow_mut().push(folder.to_owned());
            rule(folder, seen)
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
        let dir = tempdir();
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
            let dir = tempdir();
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

    /// A link to the project is judged by the folder it leads to, the one `.bolt` is in.
    #[test]
    fn the_owner_check_judges_the_resolved_folder() {
        let dir = tempdir();
        let real = dir.path().join("real");
        std::fs::create_dir(&real).unwrap();
        Repository::init(&real, &entry(0).who).unwrap();
        let link = dir.path().join("link");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&real, &link).unwrap();
        #[cfg(windows)]
        crate::test_support::junction(&link, &real);
        // Another account's link to the user's own project.
        let check = checked_by(
            |path| {
                if path == link {
                    Owner::Other
                } else {
                    Owner::User
                }
            },
            &[],
        );
        let opened = Repository::discover_with(&link, &check).unwrap();
        assert!(
            config::same_folder(opened.root(), &real),
            "{:?}",
            opened.root()
        );
        // Shown as the user would type it, not as `\\?\C:\...`.
        let shown = opened.root().display().to_string();
        assert!(!shown.starts_with(r"\\?\"), "{shown}");
        assert!(Repository::discover(&link).is_ok());
    }

    /// The user's own link into another account's folder: `bolt start` judges the folder, and
    /// makes nothing there.
    #[test]
    fn start_through_a_link_judges_the_folder_it_leads_to() {
        let dir = tempdir();
        let theirs = dir.path().join("theirs");
        std::fs::create_dir(&theirs).unwrap();
        let link = dir.path().join("link");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&theirs, &link).unwrap();
        #[cfg(windows)]
        crate::test_support::junction(&link, &theirs);
        let owner = |path: &Path| {
            if path.starts_with(&theirs) {
                Owner::Other
            } else {
                Owner::User
            }
        };
        let started = start_where(&link, owner, &[]);
        assert!(
            matches!(&started, Err(RepoError::NotYours { path }) if *path == theirs),
            "{:?}",
            started.map(|_| ())
        );
        let checked = Repository::check_new_with(&link, &checked_by(owner, &[]));
        assert!(
            matches!(checked, Err(RepoError::NotYours { .. })),
            "{checked:?}"
        );
        assert!(!theirs.join(".bolt").exists());
    }

    /// A `.bolt` that is gone while its owner is asked, then back, was never checked.
    #[cfg(feature = "fault-injection")]
    #[test]
    fn a_bolt_that_vanishes_during_the_owner_check_is_refused() {
        let dir = tempdir();
        Repository::init(dir.path(), &entry(0).who).unwrap();
        let bolt = dir.path().join(".bolt");
        let away = dir.path().join("away");
        fsio::fault::act_at("owner:asking", move || {
            std::fs::rename(&bolt, &away).unwrap();
            fsio::fault::act_at("owner:asked", move || {
                std::fs::rename(&away, &bolt).unwrap()
            });
        });
        match Repository::discover(dir.path()) {
            Err(e @ RepoError::BoltChanged { .. }) => {
                assert_eq!(e.hint().as_deref(), Some("Run the command again."));
            }
            other => panic!("{:?}", other.map(|_| ())),
        }
        assert!(format_path(dir.path()).exists());
    }

    #[test]
    fn a_resolved_path_reads_as_the_user_would_type_it() {
        let dir = tempdir();
        let sub = dir.path().join("sub");
        std::fs::create_dir(&sub).unwrap();
        // `..` and the like are resolved; no `\\?\` is added.
        assert_eq!(resolved(&sub.join("..").join("sub")), sub);
        // Kept as given where it can't be resolved.
        let missing = dir.path().join("missing");
        assert_eq!(resolved(&missing), missing);
    }

    /// `C:\...\a.` would lead to `a`, so a folder named `a.` keeps its `\\?\`.
    #[cfg(windows)]
    #[test]
    fn a_path_that_means_something_else_plain_stays_verbatim() {
        let dir = tempdir();
        let verbatim = PathBuf::from(format!(r"\\?\{}", dir.path().display())).join("a.");
        std::fs::create_dir(&verbatim).unwrap();
        assert_eq!(resolved(&verbatim), verbatim);
        std::fs::create_dir(dir.path().join("a")).unwrap();
        assert_eq!(resolved(&verbatim), verbatim);
    }

    /// Reading through the link would never end.
    #[cfg(unix)]
    #[test]
    fn a_format_link_to_dev_zero_is_refused_without_hanging() {
        let dir = tempdir();
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
        let dir = tempdir();
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
    fn a_file_bolt_cant_replace_is_named_with_what_to_do() {
        let failed = |source| RepoError::CantWrite {
            path: PathBuf::from("/srv/work/.bolt/state/head"),
            source,
        };
        let full = failed(io::Error::other("no space left"));
        assert_eq!(
            full.to_string(),
            "Bolt couldn't write /srv/work/.bolt/state/head: no space left"
        );
        assert!(full.hint().unwrap().contains("free space"), "{full:?}");
        // Code 32 means another program has the file open only on Windows.
        let held = failed(io::Error::from_raw_os_error(32));
        let hint = held.hint().unwrap();
        assert_eq!(hint.contains("virus scanner"), cfg!(windows), "{hint}");
        assert_eq!(hint.contains("free space"), !cfg!(windows), "{hint}");
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
        let folder = tempdir();
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
            RepoError::SafeFolderPassedOver {
                entry: PathBuf::from(HOSTILE),
                why: A_LINK,
                refusal: Box::new(RepoError::NotYours {
                    path: PathBuf::from(HOSTILE),
                }),
            },
            RepoError::Unfinished {
                root: PathBuf::from(HOSTILE),
                stored: false,
            },
            RepoError::BeingMade {
                root: PathBuf::from(HOSTILE),
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
                    | RepoError::SafeFolderPassedOver { .. }
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
            if reader.find(&entry_id).unwrap().is_some() {
                holders.push(reader);
            }
        }
        assert_eq!(holders.len(), 1);
        // The last pack holds what came last: the seventh object and the entry.
        let held: BTreeSet<Id> = holders[0].entries().map(|e| e.unwrap().id).collect();
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

    #[cfg(windows)]
    #[test]
    fn a_head_another_program_holds_past_every_try_is_named() {
        use std::os::windows::fs::OpenOptionsExt;
        let (dir, mut repo) = project();
        let a = commit(&mut repo, &entry(1)).unwrap();
        let b = commit(&mut repo, &entry(2)).unwrap();
        let lock = repo.lock_for_change("undo", &mut |_| {}).unwrap();
        let head = dir.path().join(".bolt").join("state").join("head");
        // As a virus scanner or sync tool may hold it, for longer than Bolt tries.
        let held = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&head)
            .unwrap();
        let started = Instant::now();
        let e = repo.move_head_to(&lock, a).unwrap_err();
        let took = started.elapsed();
        drop(held);
        // Ten tries, 100 ms apart.
        assert!(took >= Duration::from_millis(900), "{took:?}");
        let shown = escape(&head.display().to_string(), false);
        let said = e.to_string();
        assert!(
            said.starts_with(&format!("Bolt couldn't write {shown}: ")),
            "{said}"
        );
        assert!(e.hint().unwrap().contains("virus scanner"), "{e:?}");
        assert_eq!(std::fs::read_to_string(&head).unwrap(), format!("{b}\n"));
        let tmp = std::fs::read_dir(dir.path().join(".bolt").join("tmp")).unwrap();
        assert_eq!(tmp.count(), 0);
    }

    #[test]
    fn a_switch_stopped_by_damage_or_a_missing_object_names_the_way_on() {
        let stopped = |source| RepoError::SwitchStopped {
            path: "a.txt".into(),
            source: Box::new(source),
        };
        let damaged = stopped(damaged("bad copy".to_owned()));
        let missing = stopped(RepoError::Store(StoreError::Missing(Id([9; 32]))));
        for (e, copy) in [(damaged, "looks damaged"), (missing, "is missing")] {
            let hint = e.hint().unwrap();
            assert!(
                hint.contains(&format!("Bolt's copy of it {copy}")),
                "{hint}"
            );
            assert!(
                hint.contains(
                    "To go on without it, delete .bolt/state/journal: your files stay as they \
                     are, and the next command goes on from there."
                ),
                "{hint}"
            );
        }
        // Another stop is fixed where it is, so the switch can still finish.
        let other = stopped(RepoError::Io(std::io::Error::other("no space")));
        assert!(!other.hint().unwrap().contains("state/journal"));
    }

    /// A link at `link` to the folder `target`: a junction on Windows, a symbolic link elsewhere.
    fn folder_link(link: &Path, target: &Path) {
        #[cfg(windows)]
        crate::test_support::junction(link, target);
        #[cfg(unix)]
        assert!(crate::test_support::symlink(target, link));
    }

    /// `folder_link`, for a test that races it: a link that can't be made is left unmade.
    fn try_folder_link(link: &Path, target: &Path) {
        #[cfg(windows)]
        let _ = std::process::Command::new("cmd")
            .args(["/D", "/C", "mklink", "/J"])
            .arg(link)
            .arg(target)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
        #[cfg(unix)]
        let _ = std::os::unix::fs::symlink(target, link);
    }

    /// Fails unless `entry` is passed over for `root`, whose owner isn't the user, saying `why`.
    fn passed_over_for(root: &Path, entry: &Path, why: &str) {
        let owners = [Ok(Owner::Other), Ok(Owner::User)];
        let Err(e) = owner_allows(owners, root, &[entry.to_owned()]) else {
            panic!("{} counted", entry.display());
        };
        match &e {
            RepoError::SafeFolderPassedOver {
                entry: named,
                refusal,
                ..
            } => {
                assert_eq!(named, entry);
                assert!(matches!(**refusal, RepoError::NotYours { .. }), "{e:?}");
            }
            other => panic!("{other:?}"),
        }
        let said = e.to_string();
        assert!(
            said.starts_with("The project at ")
                && said.contains("belongs to another user account. Its safe-folders entry ")
                && said.contains(&format!("doesn't count: {why}")),
            "{said}"
        );
        assert!(e.hint().unwrap().contains("safe-folders"), "{e:?}");
    }

    #[test]
    fn a_safe_folders_entry_that_is_a_link_does_not_count() {
        let dir = tempdir();
        let (root, link) = (dir.path().join("project"), dir.path().join("trusted"));
        std::fs::create_dir(&root).unwrap();
        folder_link(&link, &root);
        let slashed = PathBuf::from(format!("{}{}", link.display(), std::path::MAIN_SEPARATOR));
        for entry in [&link, &slashed] {
            passed_over_for(&root, entry, A_LINK);
        }
        // A symbolic link to a folder, where this account may make one.
        #[cfg(windows)]
        {
            let symbolic = dir.path().join("symbolic");
            if crate::test_support::symlink(&root, &symbolic) {
                passed_over_for(&root, &symbolic, A_LINK);
            }
        }
    }

    /// `..` is resolved by the system, through any link before it, so it never counts.
    #[test]
    fn a_safe_folders_entry_with_dot_dot_does_not_count() {
        let dir = tempdir();
        let root = dir.path().join("project");
        std::fs::create_dir_all(root.join("sub")).unwrap();
        let why = "it has `..` in it";
        passed_over_for(&root, &root.join("sub").join(".."), why);
    }

    #[test]
    fn a_safe_folders_entry_that_is_a_real_folder_still_counts() {
        let dir = tempdir();
        let root = dir.path().join("project");
        std::fs::create_dir(&root).unwrap();
        let owners = || [Ok(Owner::Other), Ok(Owner::User)];
        assert!(owner_allows(owners(), &root, std::slice::from_ref(&root)).is_ok());
        // A link above the entry's own folder is followed, as on macOS, where `/var` is one.
        let above = dir.path().join("above");
        folder_link(&above, dir.path());
        assert!(owner_allows(owners(), &root, &[above.join("project")]).is_ok());
        // Another real folder doesn't count for it.
        let other = dir.path().join("other");
        std::fs::create_dir(&other).unwrap();
        assert!(owner_allows(owners(), &root, &[other]).is_err());
    }

    /// The trusted folder swapped back and forth between a real folder and a link to someone
    /// else's project: whatever is there when Bolt looks, that project is never trusted.
    #[test]
    fn a_safe_folders_entry_swapped_for_a_link_never_trusts_the_link_target() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let dir = tempdir();
        let (theirs, entry) = (dir.path().join("theirs"), dir.path().join("trusted"));
        std::fs::create_dir(&theirs).unwrap();
        std::fs::create_dir(&entry).unwrap();
        let done = std::sync::Arc::new(AtomicBool::new(false));
        let swapper = {
            let (theirs, entry, done) = (theirs.clone(), entry.clone(), done.clone());
            std::thread::spawn(move || {
                for _ in 0..40 {
                    // Errors are fine: a swap that fails leaves one or the other there.
                    let _ = std::fs::remove_dir(&entry);
                    try_folder_link(&entry, &theirs);
                    let _ = std::fs::remove_dir(&entry);
                    let _ = std::fs::remove_file(&entry);
                    let _ = std::fs::create_dir(&entry);
                }
                done.store(true, Ordering::SeqCst);
            })
        };
        let owners = || [Ok(Owner::Other), Ok(Owner::User)];
        for _ in 0..100_000 {
            let checked = owner_allows(owners(), &theirs, std::slice::from_ref(&entry));
            assert!(checked.is_err(), "trusted through a swapped link");
            if done.load(Ordering::SeqCst) {
                break;
            }
        }
        swapper.join().unwrap();
    }
}
