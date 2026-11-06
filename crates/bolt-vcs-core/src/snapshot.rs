//! `bolt save`: walks the project folder, stores its files, and commits the save.

use std::cmp::Reverse;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::ffi::OsStr;
use std::fmt::Write as _;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;

use bolt_platform::dir::{Dir, EntryMeta, EntryType};

use crate::changeids::ChangeIds;
use crate::diary::{Action, DiaryEntry, Position};
use crate::diff::count_changes;
use crate::drive;
use crate::fsio;
use crate::history;
use crate::id::{ChangeId, ContentHasher, Id, hash_content};
use crate::keyed;
use crate::link;
use crate::listing::{Entry, EntryKind, Listing, SaveLimits};
use crate::names::{PortabilityProblem, folder_problems};
use crate::pack::{ObjectKind, PackError, PackSet};
use crate::repo::{RepoError, Repository};
use crate::save::{EXTRA_GIT_COMMIT, EXTRA_LINE, Person, Save};
use crate::store::{Store, StoreError};
use crate::text::escape;
use crate::warning::{self, Warning};

mod endings;
mod walk;

use walk::{Base, Handles, Scan, walk};

pub struct SaveRequest<'a> {
    pub message: &'a str,
    pub who: &'a Person,
    /// The command as the diary records it; already redacted.
    pub command: &'a str,
    /// Save files Git rewrote with CRLF as they are, without the stop after an import.
    pub accept_crlf: bool,
    /// The user's own Git ignore list, from `config::git_excludes_file`, whose lines come first
    /// (decision 047); `None` reads none.
    pub git_excludes: Option<&'a Path>,
}

// Made once per command, so boxing the save would buy nothing.
#[allow(clippy::large_enum_variant)]
#[derive(Debug)]
pub enum SaveOutcome {
    Saved {
        save_id: Id,
        save: Save,
        /// Files that differ from the save before; `None` if it can't be compared with this one.
        changed_files: Option<usize>,
        /// The shortest form of the save's change ID, or the whole ID if the line couldn't be read
        /// to work it out.
        short_change_id: String,
        /// What went wrong with the save's own results; the save is made all the same. Problems
        /// the store found are `Store::warnings`.
        warnings: Vec<Warning>,
    },
    /// The folder matches the line's current save, which is `current`.
    NothingChanged {
        current: Id,
        /// What the walk found wrong, such as a `.gitignore` it skipped.
        warnings: Vec<Warning>,
    },
}

/// Why a save was refused. Names from the project folder are untrusted, so messages escape them.
#[derive(Debug, thiserror::Error)]
pub enum SaveError {
    #[error(transparent)]
    Repo(#[from] RepoError),
    #[error(
        "Can't read {}: {}.",
        escape(path, false),
        escape(&source.to_string(), false)
    )]
    Unreadable { path: String, source: io::Error },
    #[error("{} changed while Bolt was saving it.", escape(path, false))]
    ChangedDuringSave { path: String },
    #[error(
        "{} is a special file Bolt can't save yet, such as a cloud placeholder.",
        escape(path, false)
    )]
    SpecialFile { path: String },
    // `reason` is Bolt's own wording.
    #[error("Bolt can't save {}: {reason}.", escape(path, false))]
    InvalidName { path: String, reason: String },
    #[error("{}", not_portable(problems))]
    NotPortable {
        /// The folder (empty for the project's top folder) and the problem found in it.
        problems: Vec<(String, PortabilityProblem)>,
    },
    #[error("An ignore file has a problem: {}.", escape(detail, false))]
    Ignore { detail: String },
    #[error("This project is too large to save: {detail}.")]
    TooLarge { detail: String },
    #[error(
        "Bolt can't save {}: the last save has a folder there holding {}, which every save keeps \
         as it was because this computer can't hold it as it is, and a save can't hold both \
         under one name.",
        escape(path, false),
        escape(kept, false)
    )]
    InTheWay { path: String, kept: String },
    #[error("{}", line_endings_changed(*count, first))]
    LineEndingsChanged {
        /// How many files, of which `first` names the first few by project path.
        count: u64,
        first: Vec<String>,
    },
    #[error("You are not on a line, so the save has nowhere to go.")]
    NotOnALine,
}

// Through `RepoError`, so a damaged pack is reported as damage.
impl From<StoreError> for SaveError {
    fn from(e: StoreError) -> Self {
        SaveError::Repo(e.into())
    }
}

impl From<PackError> for SaveError {
    fn from(e: PackError) -> Self {
        SaveError::Repo(e.into())
    }
}

/// One line for each problem; the names in the problems are escaped by their own messages.
fn not_portable(problems: &[(String, PortabilityProblem)]) -> String {
    let mut text = "Some new names would not work on every system:".to_owned();
    for (folder, problem) in problems {
        if folder.is_empty() {
            let _ = write!(text, "\n  {problem}");
        } else {
            let _ = write!(text, "\n  in {}/: {problem}", escape(folder, false));
        }
    }
    text
}

/// How many paths a message names; it counts the rest.
const NAMED: usize = 5;

/// The paths `first` of `count`, escaped, one to a line, and how many more there are.
fn listed<S: AsRef<str>>(first: impl IntoIterator<Item = S>, count: u64) -> String {
    let mut text = String::new();
    let mut shown = 0;
    for path in first {
        let _ = write!(text, "\n  {}", escape(path.as_ref(), false));
        shown += 1;
    }
    let more = count.saturating_sub(shown);
    if more > 0 {
        let _ = write!(text, "\n  and {more} more");
    }
    text
}

/// What Git did, then the files named, one to a line, and how many more there are.
fn line_endings_changed(count: u64, first: &[String]) -> String {
    let text = if count == 1 {
        "1 file differs from what Git stored only in line endings: Git turned LF into CRLF when \
         it wrote it out (core.autocrlf)."
            .to_owned()
    } else {
        format!(
            "{count} files differ from what Git stored only in line endings: Git turned LF into \
             CRLF when it wrote them out (core.autocrlf)."
        )
    };
    text + &listed(first, count)
}

impl SaveError {
    /// What the user can do next, if there is something to say.
    #[must_use]
    pub fn hint(&self) -> Option<String> {
        Some(
            match self {
                SaveError::Repo(e) => return e.hint(),
                SaveError::InTheWay { path, .. } => {
                    return Some(format!(
                        "Nothing was saved. Rename or move {}, then save again.",
                        escape(path, false)
                    ));
                }
                // `git checkout` alone skips files whose stat data the index matches.
                SaveError::LineEndingsChanged { .. } => {
                    "To keep history as Git stored it, commit or stash your changes in Git, then \
                     run `git config core.autocrlf false`, `git rm --cached -r -q .` and `git \
                     reset --hard`, and save again. To save the files with CRLF anyway: bolt save \
                     --accept-crlf \"message\""
                }
                SaveError::NotOnALine => {
                    "This version of Bolt can only save on a line. Delete the hidden .bolt folder \
                     at the top of the project, switch Git to a branch (`git switch <branch>`) and \
                     run bolt import there again, or import a fresh clone that is on a branch."
                }
                SaveError::Unreadable { .. } => {
                    "Nothing was saved. Let Bolt read it, or list it in .boltignore."
                }
                SaveError::ChangedDuringSave { .. } => {
                    "Nothing was saved. Try again when the file stops changing, or list it in .boltignore."
                }
                SaveError::SpecialFile { .. } => "Nothing was saved. List it in .boltignore for now.",
                SaveError::InvalidName { .. } => {
                    "Nothing was saved. Rename it, or list it in .boltignore."
                }
                SaveError::NotPortable { .. } => {
                    "Nothing was saved. Rename them, or list them in .boltignore."
                }
                SaveError::Ignore { .. } => {
                    // No line is ever refused, so the file itself is at fault: too big,
                    // unreadable, or a `.boltignore` that isn't a regular file.
                    "Nothing was saved. Fix that ignore file and try again."
                }
                SaveError::TooLarge { .. } => {
                    "Nothing was saved. List the deep or huge folder in .boltignore."
                }
            }
            .to_owned(),
        )
    }
}

/// Saves the project folder on the current line. Nothing is written unless every file was read
/// and every name passed its checks.
pub fn save_folder(
    repo: &mut Repository,
    req: &SaveRequest,
    on_wait: &mut dyn FnMut(&str),
) -> Result<SaveOutcome, SaveError> {
    let lock = repo.lock_for_change(req.command, on_wait)?;
    let (head, entry) = repo.current()?;
    let Position::OnLine(line) = &entry.state.position else {
        return Err(SaveError::NotOnALine);
    };
    let parent_id = *entry
        .state
        .lines
        .get(line)
        .ok_or_else(|| RepoError::Damaged {
            detail: format!("the current line {line} has no save"),
        })?;
    let parent = repo.store().save(&parent_id, repo.signatures_allowed())?;
    // Probed under the lock: the probe's file sits in `tmp/`, which is cleared when it is free.
    let drive = drive::probe(repo.tmp_folder()).map_err(RepoError::from)?;
    let base = Base::new(repo.store(), parent.root, SaveLimits::default());
    let Scan {
        mut handles,
        folders,
        old,
        rebuilt,
        ignored,
        gitignored,
        mut warnings,
    } = walk(
        repo.root(),
        base,
        SaveLimits::default(),
        drive,
        req.git_excludes,
    )?;
    fsio::fault_point("save:walked").map_err(RepoError::from)?;
    check_names(&folders, &old)?;
    // Before anything is stored, so that a stop leaves nothing behind (decision 039).
    if !req.accept_crlf && parent.extra.contains_key(EXTRA_GIT_COMMIT) {
        endings::check(repo.store(), &mut handles, &folders, &old)?;
    }
    let mut packs = repo.new_packs()?;
    for listing in &rebuilt {
        store_listing(repo.store(), &mut packs, listing)?;
    }
    let (root, kept, changed, taken_back) = build(
        repo.store(),
        &mut packs,
        &mut handles,
        &folders,
        &old,
        &ignored,
        &gitignored,
    )?;
    if kept > 0 {
        warnings.push(kept_warning(kept));
    }
    if !changed.is_empty() {
        warnings.push(ignored_changes_saved(changed));
    }
    if !taken_back.is_empty() {
        warnings.push(gitignored_files_saved(taken_back));
    }
    if root == parent.root {
        // Anything stored here replaces a damaged copy the last save needs, so it is kept.
        if !packs.is_empty() {
            packs.finish()?;
        }
        return Ok(SaveOutcome::NothingChanged {
            current: parent_id,
            warnings,
        });
    }
    let save = Save {
        root,
        parents: vec![parent_id],
        author: req.who.clone(),
        saver: req.who.clone(),
        message: req.message.to_owned(),
        change_id: ChangeId::random()
            .map_err(io::Error::other)
            .map_err(RepoError::from)?,
        move_notes: vec![],
        signature: vec![],
        extra: BTreeMap::from([(EXTRA_LINE.to_owned(), line.as_bytes().to_vec())]),
    };
    let encoded = save.encode();
    // Refuse a save that would not read back (an over-long message, say) before storing it.
    Save::decode(&encoded, false).map_err(StoreError::from)?;
    let save_id = packs.add_record(ObjectKind::Save, &encoded)?;
    // Worked out before the commit, so that nothing after it can fail the save. The line's
    // change IDs come from the cache, which then learns this one.
    let mut cache = None;
    let shortened = (|| {
        let key = keyed::cache_key()?;
        let mut ids = ChangeIds::load(repo, &key);
        let short = history::short_among(ids.line(repo, line, parent_id)?, &save.change_id, false);
        cache = Some((ids, key));
        Ok::<_, RepoError>(short)
    })();
    let short_change_id = shortened.unwrap_or_else(|why| {
        warnings.push(Warning::new(
            warning::CHANGE_ID_NOT_SHORTENED,
            format!(
                "The save was made, but Bolt couldn't read this line's older saves to \
                     shorten its ID. {why}"
            ),
        ));
        save.change_id.to_letters()
    });
    let mut state = entry.state.clone();
    state.lines.insert(line.clone(), save_id);
    let diary = DiaryEntry {
        parent: Some(head),
        action: Action::Command,
        command: req.command.to_owned(),
        who: req.who.clone(),
        bolt_version: env!("CARGO_PKG_VERSION").into(),
        state,
        safety_copies: vec![],
    };
    repo.commit(&lock, packs, &diary)?;
    if let Some((mut ids, key)) = cache {
        ids.add(line, parent_id, save_id, save.change_id);
        ids.store(repo, &key, &diary.state);
    }
    // The save is committed now; failing to count its files must not make it look failed.
    let changed_files = match count_changes(repo.store(), Some(parent.root), root) {
        Ok(count) => Some(count),
        Err(why) => {
            warnings.push(Warning::new(
                warning::CHANGED_FILES_NOT_COUNTED,
                format!(
                    "The save was made, but Bolt couldn't count the files it changed. {}",
                    RepoError::from(why)
                ),
            ));
            None
        }
    };
    Ok(SaveOutcome::Saved {
        save_id,
        save,
        changed_files,
        short_change_id,
        warnings,
    })
}

/// Says how many entries a save kept as they were, and why.
fn kept_warning(count: usize) -> Warning {
    let message = if count == 1 {
        "Kept 1 entry as it was in the last save, because this computer can't hold it as it is \
         (a project link, a name it can't create, or a link Git wrote as a file)."
            .to_owned()
    } else {
        format!(
            "Kept {count} entries as they were in the last save, because this computer can't \
             hold them as they are (project links, names it can't create, or links Git wrote as \
             files)."
        )
    };
    Warning::new(warning::KEPT_AS_THEY_WERE, message)
}

/// What Git does with a change to `count` files an ignore rule matches, whether Git tracks them
/// or not, for both warnings about such files.
fn as_git_would(count: u64) -> &'static str {
    if count == 1 {
        "Git would leave such a change out unless the file was added (with `git add -f` if Git \
         doesn't track it yet)."
    } else {
        "Git would leave such changes out unless the files were added (with `git add -f` if Git \
         doesn't track them yet)."
    }
}

/// Says the save took changes to `paths`, files an ignore rule matches that stay saved because
/// the last save holds them (decision 038), naming the first few.
fn ignored_changes_saved(mut paths: Vec<String>) -> Warning {
    let count = paths.len() as u64;
    paths.sort_unstable();
    paths.truncate(NAMED);
    let (what, held, kept) = if count == 1 {
        (
            "a change to 1 file".to_owned(),
            "The last save holds it, so it stays saved.",
            "Bolt keeps saving it until it is deleted or moved out of the project folder.",
        )
    } else {
        (
            format!("changes to {count} files"),
            "The last save holds them, so they stay saved.",
            "Bolt keeps saving them until they are deleted or moved out of the project folder.",
        )
    };
    let message = format!(
        "Saved {what} that an ignore rule matches:{}\n{held} {} {kept}",
        listed(&paths, count),
        as_git_would(count)
    );
    Warning::new(warning::IGNORED_CHANGES_SAVED, message)
}

/// Says the save took `paths`, new or changed files that `.gitignore` keeps out but a
/// `.boltignore` line takes back, naming the first few.
fn gitignored_files_saved(mut paths: Vec<String>) -> Warning {
    let count = paths.len() as u64;
    paths.sort_unstable();
    paths.truncate(NAMED);
    let (what, undo) = if count == 1 {
        (
            "1 new or changed file".to_owned(),
            "If it shouldn't be saved, remove the .boltignore line that takes it back and move it \
             out of the project folder; this save keeps it.",
        )
    } else {
        (
            format!("{count} new or changed files"),
            "If they shouldn't be saved, remove the .boltignore lines that take them back and \
             move them out of the project folder; this save keeps them.",
        )
    };
    let message = format!(
        "Saved {what} that .gitignore keeps out but .boltignore takes back:{}\n{} {undo}",
        listed(&paths, count),
        as_git_would(count)
    );
    Warning::new(warning::GITIGNORED_FILES_SAVED, message)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    File,
    Symlink,
    Folder,
    /// An entry of the last save, kept as it was whatever the folder holds there.
    Kept {
        kind: EntryKind,
        id: Id,
        size: u64,
    },
    /// A folder gone from disk, back as the listing `id` holding only the `kept` entries kept as
    /// they were below it and the folders leading to them.
    Back {
        id: Id,
        kept: usize,
    },
}

struct Child {
    name: String,
    kind: Kind,
    /// Its project path, with `/` between names.
    rel: String,
    /// What the walk saw of a file or link: a file must still be that file when it is read.
    meta: Option<EntryMeta>,
}

/// Every folder by project path (`""` is the top folder), with its children sorted by name.
type Folders = BTreeMap<String, Vec<Child>>;

/// Judges each folder's new names against all of its names (decision 004).
fn check_names(folders: &Folders, old: &HashMap<String, Option<Listing>>) -> Result<(), SaveError> {
    let mut problems = Vec::new();
    for (rel, children) in folders {
        let before = old.get(rel).and_then(Option::as_ref);
        let names: Vec<&str> = children.iter().map(|c| c.name.as_str()).collect();
        let kept: Vec<&str> = names
            .iter()
            .copied()
            .filter(|n| before.is_some_and(|l| l.get(n).is_some()))
            .collect();
        if kept.len() == names.len() {
            continue;
        }
        let known = folder_problems(&kept);
        for p in folder_problems(&names) {
            if !known.contains(&p) {
                problems.push((rel.clone(), p));
            }
        }
    }
    if problems.is_empty() {
        Ok(())
    } else {
        Err(SaveError::NotPortable { problems })
    }
}

/// Stores every file and listing, deepest folders first, reading the files and links through
/// `handles`. Returns the top listing's ID, how many entries were kept as they were, which of the
/// files and links in `ignored` changed, and which of those in `gitignored` are new or changed.
fn build(
    store: &Store,
    packs: &mut PackSet,
    handles: &mut Handles,
    folders: &Folders,
    old: &HashMap<String, Option<Listing>>,
    ignored: &HashSet<String>,
    gitignored: &HashSet<String>,
) -> Result<(Id, usize, Vec<String>, Vec<String>), SaveError> {
    let mut order: Vec<&String> = folders.keys().collect();
    order.sort_by_key(|rel| {
        Reverse(if rel.is_empty() {
            0
        } else {
            rel.split('/').count()
        })
    });
    let mut built: HashMap<&str, Id> = HashMap::new();
    let missing = || RepoError::from(io::Error::other("a folder was built before its subfolders"));
    let mut kept = 0;
    let (mut changed, mut taken_back) = (Vec::new(), Vec::new());
    for rel in order {
        let before = old.get(rel).and_then(Option::as_ref);
        let mut entries = Vec::new();
        for c in folders.get(rel).into_iter().flatten() {
            let (kind, id, size) = match c.kind {
                Kind::Folder => {
                    let id = built.remove(c.rel.as_str()).ok_or_else(missing)?;
                    (EntryKind::Folder, id, 0)
                }
                Kind::File => {
                    let dir = handles.folder(rel)?;
                    let meta = c.meta.as_ref();
                    let (id, size) =
                        store_file(packs, store, dir, &c.name, &c.rel, meta, &mut || {})?;
                    let was = before.and_then(|l| l.get(&c.name));
                    // Git writes a link it can't make as a file holding its target: the link
                    // stays (decision 039).
                    let links_target =
                        |e: &Entry| e.kind == EntryKind::Symlink && (e.id, e.size) == (id, size);
                    if was.is_some_and(links_target) {
                        kept += 1;
                        (EntryKind::Symlink, id, size)
                    } else {
                        // Without `x` bits, a file stays executable if it was (decision 004).
                        let executable = c.meta.and_then(|m| m.executable).unwrap_or_else(|| {
                            was.is_some_and(|e| e.kind == EntryKind::Executable)
                        });
                        let kind = if executable {
                            EntryKind::Executable
                        } else {
                            EntryKind::File
                        };
                        (kind, id, size)
                    }
                }
                Kind::Symlink => {
                    let dir = handles.folder(rel)?;
                    let (id, size) = store_link(packs, store, dir, &c.name, &c.rel)?;
                    (EntryKind::Symlink, id, size)
                }
                Kind::Kept { kind, id, size } => {
                    kept += 1;
                    (kind, id, size)
                }
                Kind::Back { id, kept: below } => {
                    kept += below;
                    (EntryKind::Folder, id, 0)
                }
            };
            let was = before.and_then(|l| l.get(&c.name));
            if was.is_none_or(|e| (e.kind, e.id) != (kind, id)) {
                if ignored.contains(&c.rel) {
                    changed.push(c.rel.clone());
                } else if gitignored.contains(&c.rel) {
                    taken_back.push(c.rel.clone());
                }
            }
            entries.push(Entry {
                name: c.name.clone(),
                kind,
                id,
                size,
            });
        }
        let listing = Listing::new(entries).map_err(StoreError::from)?;
        built.insert(rel, store_listing(store, packs, &listing)?);
    }
    let root = built.remove("").ok_or_else(missing)?;
    Ok((root, kept, changed, taken_back))
}

/// Stores `listing` unless a good copy is held already, and returns its ID.
fn store_listing(store: &Store, packs: &mut PackSet, listing: &Listing) -> Result<Id, SaveError> {
    let id = listing.id();
    let encoded = listing.encode();
    let len = encoded.len() as u64;
    if !packs.contains(&id) && !store.has_good_copy(&id, ObjectKind::FolderListing, len)? {
        packs.add_record(ObjectKind::FolderListing, &encoded)?;
    }
    Ok(id)
}

/// Stores the target of the link `name` in `dir`, at the project path `rel`, as content with `/`
/// between its parts (decision 045); the link itself is never followed.
fn store_link(
    packs: &mut PackSet,
    store: &Store,
    dir: &Dir,
    name: &str,
    rel: &str,
) -> Result<(Id, u64), SaveError> {
    let unreadable = |source| SaveError::Unreadable {
        path: rel.to_owned(),
        source,
    };
    let target = dir.read_link(OsStr::new(name)).map_err(unreadable)?;
    let target = target.into_string().map_err(|_| {
        unreadable(io::Error::new(
            io::ErrorKind::InvalidData,
            "the link's target is not valid UTF-8",
        ))
    })?;
    let target = link::stored_target(target);
    let id = hash_content(target.as_bytes());
    if !store.has_good_copy(&id, ObjectKind::Content, target.len() as u64)? {
        packs.add_content(target.as_bytes())?;
    }
    Ok((id, target.len() as u64))
}

/// Opens the file `name` in `dir`, at the project path `rel`, to read, if it is still the regular
/// file the walk saw (`seen`): the same file, size and time, so what is stored is what the walk
/// described. Anything else changed since, unless it is a Windows reparse point that isn't a link
/// (a cloud placeholder, say), which Bolt can't save yet. Returns the file and its details.
fn open_scanned(
    dir: &Dir,
    name: &str,
    rel: &str,
    seen: Option<&EntryMeta>,
) -> Result<(File, EntryMeta), SaveError> {
    let name = OsStr::new(name);
    let path = || rel.to_owned();
    let file = dir
        .open_file(name)
        .map_err(|source| match dir.metadata(name) {
            Ok(now) if cfg!(windows) && now.kind == EntryType::Other => {
                SaveError::SpecialFile { path: path() }
            }
            Ok(now) if now.kind != EntryType::File => SaveError::ChangedDuringSave { path: path() },
            _ => SaveError::Unreadable {
                path: path(),
                source,
            },
        })?;
    let now = Dir::file_meta(&file).map_err(|source| SaveError::Unreadable {
        path: path(),
        source,
    })?;
    let what = |m: &EntryMeta| (m.file_id, m.size, m.modified_ns);
    if seen.is_none_or(|seen| what(seen) != what(&now)) {
        return Err(SaveError::ChangedDuringSave { path: path() });
    }
    Ok((file, now))
}

/// A file's size and modification time, to see whether it changed while it was read.
fn stamp(file: &File) -> io::Result<(u64, i128)> {
    Dir::file_meta(file).map(|m| (m.size, m.modified_ns))
}

/// Stores one file in two passes: hash it, then copy it into `packs` unless already held. The
/// file is `name` in `dir`, at the project path `rel`, and must be the one the walk saw (`seen`;
/// see `open_scanned`). A file that changes in between gives `ChangedDuringSave`.
pub(crate) fn store_file(
    packs: &mut PackSet,
    store: &Store,
    dir: &Dir,
    name: &str,
    rel: &str,
    seen: Option<&EntryMeta>,
    between_passes: &mut dyn FnMut(),
) -> Result<(Id, u64), SaveError> {
    let unreadable = |source| SaveError::Unreadable {
        path: rel.to_owned(),
        source,
    };
    let changed = || SaveError::ChangedDuringSave {
        path: rel.to_owned(),
    };
    let (mut file, opened) = open_scanned(dir, name, rel, seen)?;
    let before = (opened.size, opened.modified_ns);
    let mut hasher = ContentHasher::new();
    let mut buf = vec![0u8; 1 << 20];
    let mut size = 0u64;
    loop {
        let n = match file.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(unreadable(e)),
        };
        hasher.update(&buf[..n]);
        size += n as u64;
    }
    let id = hasher.finish();
    between_passes();
    if size != before.0 || stamp(&file).map_err(unreadable)? != before {
        return Err(changed());
    }
    if packs.contains(&id) || store.has_good_copy(&id, ObjectKind::Content, size)? {
        return Ok((id, size));
    }
    file.seek(SeekFrom::Start(0)).map_err(unreadable)?;
    match packs.add_content_from(&mut file, id) {
        Ok(_) => Ok((id, size)),
        Err(PackError::ContentChanged) => Err(changed()),
        Err(e) => Err(e.into()),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;
    use crate::drive::Drive;
    use crate::id::hash_content;
    use crate::listing::EntryKind;
    use crate::names::{name_problems, validate_name};
    use crate::pack::{ENTRY_HEADER_LEN, PackReader};
    use crate::save::Person;
    use crate::test_support::{commit_records, plant_damaged_copies, plant_save, sample_records};
    use proptest::prelude::*;
    use std::collections::BTreeMap;
    use std::fs;
    use std::path::Path;

    fn who() -> Person {
        Person {
            name: "Ada".into(),
            email: "ada@example.com".into(),
            time_ms: 1_700_000_000_000,
            tz_offset_min: 60,
        }
    }

    pub(super) fn project() -> (tempfile::TempDir, Repository) {
        let dir = tempfile::tempdir().unwrap();
        let repo = Repository::init(dir.path(), &who()).unwrap();
        (dir, repo)
    }

    pub(super) fn save_now(repo: &mut Repository) -> Result<SaveOutcome, SaveError> {
        save_with(repo, None)
    }

    /// A save with `git_excludes` as the user's own Git ignore list.
    pub(super) fn save_with(
        repo: &mut Repository,
        git_excludes: Option<&Path>,
    ) -> Result<SaveOutcome, SaveError> {
        let who = who();
        let req = SaveRequest {
            message: "Change things",
            who: &who,
            command: "save",
            accept_crlf: false,
            git_excludes,
        };
        save_folder(repo, &req, &mut |_| {})
    }

    /// The root listing of a save that must have happened.
    pub(super) fn saved_root(outcome: SaveOutcome) -> Id {
        match outcome {
            SaveOutcome::Saved { save, .. } => save.root,
            SaveOutcome::NothingChanged { .. } => panic!("nothing was saved"),
        }
    }

    pub(super) type Tree = BTreeMap<String, (EntryKind, Vec<u8>)>;

    /// Every entry of a saved folder, by path; folders and project links have no bytes.
    pub(super) fn saved_tree(store: &Store, root: Id) -> Tree {
        fn walk(store: &Store, id: Id, prefix: &str, out: &mut Tree) {
            for e in store.listing(&id).unwrap().entries() {
                let path = format!("{prefix}{}", e.name);
                let mut bytes = Vec::new();
                if e.kind == EntryKind::Folder {
                    walk(store, e.id, &format!("{path}/"), out);
                } else if e.kind != EntryKind::ProjectLink {
                    store.copy_content(&e.id, &mut bytes).unwrap();
                    assert_eq!(bytes.len() as u64, e.size, "{path}");
                }
                out.insert(path, (e.kind, bytes));
            }
        }
        let mut out = Tree::new();
        walk(store, root, "", &mut out);
        out
    }

    /// The project folder as it is on disk, without `.bolt`.
    fn disk_tree(root: &Path) -> Tree {
        fn walk(dir: &Path, prefix: &str, out: &mut Tree) {
            for e in fs::read_dir(dir).unwrap() {
                let e = e.unwrap();
                let name = e.file_name().into_string().unwrap();
                if prefix.is_empty() && name == ".bolt" {
                    continue;
                }
                let path = format!("{prefix}{name}");
                if e.file_type().unwrap().is_dir() {
                    walk(&e.path(), &format!("{path}/"), out);
                    out.insert(path, (EntryKind::Folder, Vec::new()));
                } else {
                    out.insert(path, (EntryKind::File, fs::read(e.path()).unwrap()));
                }
            }
        }
        let mut out = Tree::new();
        walk(root, "", &mut out);
        out
    }

    pub(super) fn write(root: &Path, rel: &str, bytes: &[u8]) {
        let path = root.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }

    /// The drive the tests run on, as `save` probes it.
    pub(super) fn this_drive() -> Drive {
        let tmp = tempfile::tempdir().unwrap();
        drive::probe(&Dir::open_top(tmp.path()).unwrap()).unwrap()
    }

    /// The line's newest save, as the base of a walk, with the format's limits.
    pub(super) fn newest(repo: &Repository) -> Base<'_> {
        let (_, entry) = repo.current().unwrap();
        let id = entry.state.lines["main"];
        let save = repo.store().save(&id, repo.signatures_allowed()).unwrap();
        Base::new(repo.store(), save.root, SaveLimits::default())
    }

    /// `walk` of the project on top of the line's newest save, as `save` does.
    pub(super) fn walk_project(repo: &Repository, limits: SaveLimits) -> Result<Scan, SaveError> {
        walk(repo.root(), newest(repo), limits, this_drive(), None)
    }

    #[test]
    fn saved_files_read_back_byte_identical() {
        let (dir, mut repo) = project();
        let root = dir.path();
        write(root, "src/game/player.rs", b"fn jump() {}\n");
        write(root, "src/main.rs", b"fn main() {}\n");
        write(root, "src/empty.txt", b"");
        fs::create_dir_all(root.join("assets/empty")).unwrap();
        let big: Vec<u8> = (0..3u64 << 20).map(|i| (i * 7919 % 251) as u8).collect();
        write(root, "assets/big.bin", &big);
        write(
            root,
            "caf\u{e9}/\u{65e5}\u{672c}.txt",
            "na\u{ef}ve".as_bytes(),
        );
        let id = saved_root(save_now(&mut repo).unwrap());
        let tree = saved_tree(repo.store(), id);
        assert_eq!(tree, disk_tree(root));
        assert_eq!(tree["assets/empty"].0, EntryKind::Folder);
        assert!(!tree.keys().any(|k| k.starts_with("assets/empty/")));
        assert_eq!(tree["assets/big.bin"].1, big);
    }

    fn name() -> impl Strategy<Value = String> {
        "[a-z0-9_.-]{1,10}".prop_filter("a storable, portable name", |n| {
            validate_name(n).is_ok() && name_problems(n).is_empty()
        })
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(32))]
        #[test]
        fn random_folders_round_trip(
            files in prop::collection::vec(
                (prop::collection::vec(name(), 1..4), prop::collection::vec(any::<u8>(), 0..300)),
                0..12,
            ),
        ) {
            let (dir, mut repo) = project();
            let root = dir.path();
            for (parts, bytes) in &files {
                let path = parts.iter().fold(root.to_path_buf(), |p, n| p.join(n));
                // A path already used as a file or folder in another shape is skipped.
                if path.exists() || fs::create_dir_all(path.parent().unwrap()).is_err() {
                    continue;
                }
                let _ = fs::write(&path, bytes);
            }
            let expected = disk_tree(root);
            match save_now(&mut repo).unwrap() {
                SaveOutcome::Saved { save, .. } => {
                    prop_assert_eq!(saved_tree(repo.store(), save.root), expected);
                }
                SaveOutcome::NothingChanged { .. } => prop_assert!(expected.is_empty()),
            }
        }
    }

    #[test]
    fn second_save_without_changes_is_nothing_changed() {
        let (dir, mut repo) = project();
        write(dir.path(), "a.txt", b"a");
        let SaveOutcome::Saved {
            save_id,
            save,
            changed_files,
            short_change_id,
            warnings,
        } = save_now(&mut repo).unwrap()
        else {
            panic!("nothing was saved");
        };
        assert_eq!(changed_files, Some(1));
        assert_eq!(
            short_change_id,
            history::short_change_id(&repo, &save_id, &save.change_id).unwrap()
        );
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(save.line(), Some("main"));
        assert_eq!(save.author, who());
        let (head, entry) = repo.current().unwrap();
        assert_eq!(entry.state.lines["main"], save_id);
        assert_eq!(entry.command, "save");
        assert!(entry.parent.is_some());
        let first = repo.store().save(&save.parents[0], false).unwrap();
        assert_eq!(first.message, "Start project");
        match save_now(&mut repo).unwrap() {
            SaveOutcome::NothingChanged { current, warnings } => {
                assert_eq!(current, save_id);
                assert!(warnings.is_empty(), "{warnings:?}");
            }
            SaveOutcome::Saved { .. } => panic!("saved again"),
        }
        assert_eq!(repo.head().unwrap(), head);
    }

    #[test]
    fn a_save_does_not_read_its_line_once_the_cache_is_built() {
        let (dir, mut repo) = project();
        let mut shorts = Vec::new();
        for n in 0..6 {
            write(dir.path(), "a.txt", format!("{n}").as_bytes());
            let before = repo.store().saves_read();
            let SaveOutcome::Saved {
                save_id,
                save,
                short_change_id,
                ..
            } = save_now(&mut repo).unwrap()
            else {
                panic!("nothing was saved");
            };
            // The save it builds on, and nothing else of the line, once a save built the cache.
            if n > 0 {
                assert_eq!(repo.store().saves_read() - before, 1, "save {n}");
            }
            shorts.push((save_id, save.change_id, short_change_id));
        }
        // The newest save's short ID is the one the line gives it.
        let fresh = Repository::open(dir.path()).unwrap();
        let (id, change_id, short) = shorts.last().unwrap();
        assert_eq!(
            *short,
            history::short_change_id(&fresh, id, change_id).unwrap()
        );
        // History reads the line once.
        let fresh = Repository::open(dir.path()).unwrap();
        history::for_each_on_line::<RepoError>(&fresh, |_, _, _| Ok(())).unwrap();
        assert_eq!(fresh.store().saves_read(), 7);
    }

    #[test]
    fn save_stores_a_file_even_when_a_damaged_copy_claims_its_id() {
        let (dir, _repo) = project();
        // Content anyone can predict, like a licence, in a pack that came with the project.
        let licence = b"Permission is hereby granted, free of charge, to any person\n";
        plant_damaged_copies(
            &dir.path().join(".bolt/store/packs"),
            &[(ObjectKind::Content, licence)],
        );
        write(dir.path(), "LICENSE", licence);
        let mut repo = Repository::open(dir.path()).unwrap();
        let root = saved_root(save_now(&mut repo).unwrap());
        let damaged = repo.store().damaged_packs();
        assert_eq!(damaged.len(), 1, "{damaged:?}");
        assert!(damaged[0].to_string().contains("00.pack"), "{damaged:?}");
        let fresh = Repository::open(dir.path()).unwrap();
        assert_eq!(saved_tree(fresh.store(), root), disk_tree(dir.path()));
    }

    #[test]
    fn save_stores_listings_even_when_damaged_copies_claim_their_ids() {
        let (dir, _repo) = project();
        fs::create_dir(dir.path().join("empty")).unwrap();
        write(dir.path(), "sub/a.txt", b"x");
        let sub = Listing::new(vec![Entry {
            name: "a.txt".into(),
            kind: EntryKind::File,
            id: hash_content(b"x"),
            size: 1,
        }])
        .unwrap();
        // The empty listing has the same ID in every project.
        plant_damaged_copies(
            &dir.path().join(".bolt/store/packs"),
            &[
                (ObjectKind::FolderListing, &Listing::empty().encode()),
                (ObjectKind::FolderListing, &sub.encode()),
            ],
        );
        let mut repo = Repository::open(dir.path()).unwrap();
        let root = saved_root(save_now(&mut repo).unwrap());
        assert_eq!(repo.store().damaged_packs().len(), 1);
        let fresh = Repository::open(dir.path()).unwrap();
        assert_eq!(saved_tree(fresh.store(), root), disk_tree(dir.path()));
    }

    #[test]
    fn a_save_with_no_changes_keeps_the_copy_it_stored_in_place_of_a_damaged_one() {
        let (dir, mut repo) = project();
        let licence = b"Permission is hereby granted, free of charge, to any person\n";
        let id = hash_content(licence);
        write(dir.path(), "LICENSE", licence);
        save_now(&mut repo).unwrap();
        damage_the_only_copy_of(&repo, &id);
        let mut repo = Repository::open(dir.path()).unwrap();
        assert!(matches!(
            save_now(&mut repo).unwrap(),
            SaveOutcome::NothingChanged { .. }
        ));
        let mut back = Vec::new();
        let fresh = Repository::open(dir.path()).unwrap();
        fresh.store().copy_content(&id, &mut back).unwrap();
        assert_eq!(back, licence);
    }

    #[test]
    fn concurrent_saves_from_separate_handles_both_succeed() {
        let (dir, _repo) = project();
        write(dir.path(), "a.txt", b"a");
        let open = || Repository::open(dir.path()).unwrap();
        // All three load the store before any of them saves.
        let (mut a, mut b, mut late) = (open(), open(), open());
        let outcomes = std::thread::scope(|s| {
            let ta = s.spawn(|| save_now(&mut a).unwrap());
            let tb = s.spawn(|| save_now(&mut b).unwrap());
            [ta.join().unwrap(), tb.join().unwrap()]
        });
        // Same folder: one saves it, the other then finds nothing new.
        let saved: Vec<Id> = outcomes
            .iter()
            .filter_map(|o| match o {
                SaveOutcome::Saved { save_id, .. } => Some(*save_id),
                SaveOutcome::NothingChanged { .. } => None,
            })
            .collect();
        assert_eq!(saved.len(), 1, "{outcomes:?}");
        write(dir.path(), "b.txt", b"b");
        let SaveOutcome::Saved {
            save_id: second, ..
        } = save_now(&mut late).unwrap()
        else {
            panic!("nothing was saved");
        };
        let mut line = Vec::new();
        history::for_each_on_line::<RepoError>(&open(), |id, _, _| {
            line.push(*id);
            Ok(())
        })
        .unwrap();
        assert_eq!(line[..2], [second, saved[0]]);
    }

    #[test]
    fn a_save_keeps_the_records_of_the_entry_before_it() {
        let (dir, mut repo) = project();
        let (tags, servers) = sample_records();
        let (t, r) = commit_records(&mut repo, &tags, &servers);
        write(dir.path(), "a.txt", b"a");
        saved_root(save_now(&mut repo).unwrap());
        let after = repo.current().unwrap().1.state;
        assert_eq!((after.tags, after.server_lines), (Some(t), Some(r)));
        assert_eq!(repo.tag_list(&after).unwrap(), tags);
        assert_eq!(repo.server_lines(&after).unwrap(), servers);
    }

    /// Bit rot in the only copy of an object: its first stored byte is flipped.
    pub(super) fn damage_the_only_copy_of(repo: &Repository, id: &Id) {
        for e in fs::read_dir(repo.packs_dir()).unwrap() {
            let path = e.unwrap().path();
            let found = PackReader::open(&path).unwrap().find(id).unwrap();
            if let Some(entry) = found {
                let mut bytes = fs::read(&path).unwrap();
                bytes[usize::try_from(entry.offset + ENTRY_HEADER_LEN).unwrap()] ^= 1;
                fs::write(&path, bytes).unwrap();
            }
        }
    }

    #[test]
    fn a_save_is_made_even_when_the_line_cant_be_read_to_shorten_its_id() {
        let (dir, mut repo) = project();
        let first = repo.current().unwrap().1.state.lines["main"];
        write(dir.path(), "a.txt", b"a");
        saved_root(save_now(&mut repo).unwrap());
        damage_the_only_copy_of(&repo, &first);
        // Without the cache, the line must be read.
        fs::remove_file(dir.path().join(".bolt/cache/change-ids")).unwrap();

        let mut repo = Repository::open(dir.path()).unwrap();
        write(dir.path(), "b.txt", b"b");
        let SaveOutcome::Saved {
            save_id,
            save,
            short_change_id,
            warnings,
            ..
        } = save_now(&mut repo).unwrap()
        else {
            panic!("nothing was saved");
        };
        // The whole ID stands in for the short one, and a warning says why.
        assert_eq!(short_change_id, save.change_id.to_letters());
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert_eq!(warnings[0].kind, "change_id_not_shortened");
        assert!(
            warnings[0].message.starts_with(
                "The save was made, but Bolt couldn't read this line's older saves to shorten its ID."
            ),
            "{warnings:?}"
        );
        // The save is made all the same, and is the line's newest.
        let fresh = Repository::open(dir.path()).unwrap();
        assert_eq!(fresh.current().unwrap().1.state.lines["main"], save_id);
        assert_eq!(saved_tree(fresh.store(), save.root), disk_tree(dir.path()));
    }

    #[cfg(unix)]
    #[test]
    fn executable_flag_is_saved() {
        use std::os::unix::fs::PermissionsExt;
        let (dir, mut repo) = project();
        write(dir.path(), "run.sh", b"#!/bin/sh\n");
        write(dir.path(), "notes.txt", b"hi");
        let run = dir.path().join("run.sh");
        fs::set_permissions(&run, fs::Permissions::from_mode(0o744)).unwrap();
        let id = saved_root(save_now(&mut repo).unwrap());
        let tree = saved_tree(repo.store(), id);
        assert_eq!(tree["run.sh"].0, EntryKind::Executable);
        assert_eq!(tree["notes.txt"].0, EntryKind::File);
    }

    #[cfg(any(unix, windows))]
    #[test]
    fn symlinks_are_saved_not_followed() {
        let (dir, mut repo) = project();
        let root = dir.path();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(".", root.join("loop")).unwrap();
            std::os::unix::fs::symlink("/", root.join("out")).unwrap();
        }
        #[cfg(windows)]
        {
            // Making symlinks needs Developer Mode or admin rights on Windows.
            if std::os::windows::fs::symlink_dir(".", root.join("loop")).is_err() {
                return;
            }
            std::os::windows::fs::symlink_dir("/", root.join("out")).unwrap();
        }
        let id = saved_root(save_now(&mut repo).unwrap());
        let tree = saved_tree(repo.store(), id);
        assert_eq!(tree.len(), 2, "{tree:?}");
        assert_eq!(tree["loop"], (EntryKind::Symlink, b".".to_vec()));
        assert_eq!(tree["out"], (EntryKind::Symlink, b"/".to_vec()));
    }

    #[cfg(unix)]
    #[test]
    fn save_refuses_non_portable_new_names() {
        let (dir, mut repo) = project();
        let head = repo.head().unwrap();
        for n in ["README", "readme", "a:b", "CON", "fine.txt"] {
            write(dir.path(), n, b"x");
        }
        let Err(SaveError::NotPortable { problems }) = save_now(&mut repo) else {
            panic!("the names were accepted");
        };
        let text: Vec<String> = problems.iter().map(|(_, p)| p.to_string()).collect();
        // On a case-insensitive disk (macOS by default) `readme` overwrote `README`: no collision.
        let both_cases = fs::read_dir(dir.path())
            .unwrap()
            .filter(|e| {
                e.as_ref()
                    .unwrap()
                    .file_name()
                    .eq_ignore_ascii_case("readme")
            })
            .count()
            == 2;
        let mut needles = vec!["a:b", "CON"];
        if both_cases {
            needles.push("readme");
        }
        assert_eq!(problems.len(), needles.len(), "{text:?}");
        for needle in needles {
            assert!(
                text.iter().any(|t| t.contains(needle)),
                "{needle}: {text:?}"
            );
        }
        assert_eq!(repo.head().unwrap(), head);
    }

    #[test]
    fn names_already_saved_are_not_checked_again() {
        // Only the folder's new names are judged, but against all of its names.
        let (dir, mut repo) = project();
        write(dir.path(), "README", b"x");
        save_now(&mut repo).unwrap();
        write(dir.path(), "notes.txt", b"x");
        assert!(matches!(save_now(&mut repo), Ok(SaveOutcome::Saved { .. })));
    }

    #[test]
    fn a_new_name_over_255_bytes_is_refused_and_one_saved_before_stays() {
        let (dir, mut repo) = project();
        // 300 bytes, which Windows holds as 100 UTF-16 units; macOS and Linux can't make it.
        let long = "語".repeat(100);
        if fs::write(dir.path().join(&long), b"x").is_ok() {
            let head = repo.head().unwrap();
            let Err(SaveError::NotPortable { problems }) = save_now(&mut repo) else {
                panic!("a new name of 300 bytes was saved");
            };
            assert!(
                matches!(problems.as_slice(), [(_, PortabilityProblem::TooLong(_))]),
                "{problems:?}"
            );
            assert_eq!(repo.head().unwrap(), head);
        }
        // One the last save holds, as an import brings it, stays: read again where this system
        // holds it, and kept as it was where it doesn't.
        plant_save(&mut repo, &[(long.as_str(), EntryKind::File, b"x")]);
        write(dir.path(), "new.txt", b"x");
        let saved = saved_root(save_now(&mut repo).unwrap());
        let tree = saved_tree(repo.store(), saved);
        let paths: Vec<&str> = tree.keys().map(String::as_str).collect();
        assert_eq!(paths, ["new.txt", long.as_str()]);
    }

    #[cfg(unix)]
    #[test]
    fn new_name_clashing_with_a_saved_name_is_refused() {
        let (dir, mut repo) = project();
        write(dir.path(), "README", b"x");
        save_now(&mut repo).unwrap();
        let head = repo.head().unwrap();
        write(dir.path(), "readme", b"y");
        // A case-insensitive disk (macOS by default) can't hold both names.
        if fs::read(dir.path().join("README")).unwrap() != b"x" {
            return;
        }
        let Err(SaveError::NotPortable { problems }) = save_now(&mut repo) else {
            panic!("the clashing name was accepted");
        };
        assert_eq!(problems.len(), 1);
        assert!(problems[0].1.to_string().contains("readme"), "{problems:?}");
        assert_eq!(repo.head().unwrap(), head);
    }

    #[cfg(unix)]
    #[test]
    fn save_refuses_non_utf8_names() {
        use std::os::unix::ffi::OsStrExt;
        let (dir, mut repo) = project();
        let head = repo.head().unwrap();
        let bad = dir.path().join(std::ffi::OsStr::from_bytes(b"f\xff"));
        // macOS refuses such names itself, so there is nothing to test there.
        if fs::write(&bad, b"x").is_err() {
            return;
        }
        match save_now(&mut repo) {
            Err(SaveError::InvalidName { path, .. }) => assert_eq!(path, "f\u{fffd}"),
            other => panic!("{:?}", other.err()),
        }
        assert_eq!(repo.head().unwrap(), head);
    }

    #[test]
    fn save_names_an_unreadable_file_and_changes_nothing() {
        let (dir, mut repo) = project();
        let head = repo.head().unwrap();
        write(dir.path(), "ok.txt", b"x");
        write(dir.path(), "sub/locked.txt", b"secret");
        let locked = dir.path().join("sub/locked.txt");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
            // root reads everything, so there is nothing to test.
            if fs::read(&locked).is_ok() {
                return;
            }
        }
        #[cfg(windows)]
        let _guard = {
            use std::os::windows::fs::OpenOptionsExt;
            fs::OpenOptions::new()
                .read(true)
                .share_mode(0)
                .open(&locked)
                .unwrap()
        };
        match save_now(&mut repo) {
            Err(SaveError::Unreadable { path, .. }) => assert_eq!(path, "sub/locked.txt"),
            other => panic!("{:?}", other.err()),
        }
        assert_eq!(repo.head().unwrap(), head);
    }

    /// RT-11: a folder Bolt may not enter is an unreadable folder, not a `.boltignore` problem.
    #[cfg(unix)]
    #[test]
    fn save_names_an_unreadable_folder_as_unreadable() {
        use std::os::unix::fs::PermissionsExt;
        let (dir, mut repo) = project();
        let head = repo.head().unwrap();
        write(dir.path(), "sub/locked/f.txt", b"secret");
        let locked = dir.path().join("sub/locked");
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
        // root reads everything, so there is nothing to test.
        let root_user = fs::read_dir(&locked).is_ok();
        let saved = save_now(&mut repo);
        // Back to normal first, so that the temporary folder can be removed whatever happens.
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
        if root_user {
            return;
        }
        match saved {
            Err(SaveError::Unreadable { path, source }) => {
                assert_eq!(path, "sub/locked");
                assert_eq!(source.kind(), io::ErrorKind::PermissionDenied);
            }
            other => panic!("{:?}", other.map(|_| ())),
        }
        assert_eq!(repo.head().unwrap(), head);
    }

    #[test]
    fn errors_show_untrusted_text_escaped() {
        use crate::test_support::{HOSTILE, assert_hostile_text_escaped};
        let hostile = || HOSTILE.to_owned();
        for e in [
            SaveError::Unreadable {
                path: hostile(),
                source: io::Error::other("fine"),
            },
            SaveError::Unreadable {
                path: "fine".into(),
                source: io::Error::other(HOSTILE),
            },
            SaveError::ChangedDuringSave { path: hostile() },
            SaveError::SpecialFile { path: hostile() },
            SaveError::InvalidName {
                path: hostile(),
                reason: "fine".into(),
            },
            SaveError::Ignore { detail: hostile() },
            SaveError::InTheWay {
                path: hostile(),
                kept: "fine".into(),
            },
            SaveError::InTheWay {
                path: "fine".into(),
                kept: hostile(),
            },
            SaveError::LineEndingsChanged {
                count: 2,
                first: vec!["fine".into(), hostile()],
            },
        ] {
            assert_hostile_text_escaped(&e.to_string());
        }
        // This hint names the path too.
        let e = SaveError::InTheWay {
            path: hostile(),
            kept: "fine".into(),
        };
        assert_hostile_text_escaped(&e.hint().unwrap());
        // The lines Bolt writes itself stay lines; the folders and names in them are escaped.
        let e = SaveError::NotPortable {
            problems: vec![
                (String::new(), PortabilityProblem::ReservedName(hostile())),
                (
                    hostile(),
                    PortabilityProblem::TrailingDotOrSpace("a.".into()),
                ),
            ],
        };
        let shown = e.to_string();
        assert_hostile_text_escaped(&shown);
        assert_eq!(shown.lines().count(), 3, "{shown}");
        assert!(
            shown.lines().skip(1).all(|l| l.starts_with("  ")),
            "{shown}"
        );
    }

    #[test]
    fn the_ignored_files_a_save_changed_are_named_escaped_and_counted() {
        use crate::test_support::{HOSTILE, assert_hostile_text_escaped};
        assert_hostile_text_escaped(&ignored_changes_saved(vec![HOSTILE.to_owned()]).message);
        let paths = (1..=7).rev().map(|i| format!("f{i}.dll")).collect();
        let message = ignored_changes_saved(paths).message;
        let listed = "Saved changes to 7 files that an ignore rule matches:\n  f1.dll\n  f2.dll\n  \
                      f3.dll\n  f4.dll\n  f5.dll\n  and 2 more\nThe last save holds them";
        assert!(message.starts_with(listed), "{message}");
    }

    #[test]
    fn the_files_boltignore_takes_back_from_gitignore_are_named_escaped_and_counted() {
        use crate::test_support::{HOSTILE, assert_hostile_text_escaped};
        assert_hostile_text_escaped(&gitignored_files_saved(vec![HOSTILE.to_owned()]).message);
        let paths = (1..=6).rev().map(|i| format!("k{i}.pem")).collect();
        let message = gitignored_files_saved(paths).message;
        let listed = "Saved 6 new or changed files that .gitignore keeps out but .boltignore takes \
                      back:\n  k1.pem\n  k2.pem\n  k3.pem\n  k4.pem\n  k5.pem\n  and 1 more\nGit \
                      would leave such changes out";
        assert!(message.starts_with(listed), "{message}");
        // The same words, true whether Git tracks a file or not, in both warnings.
        let one = gitignored_files_saved(vec!["a.pem".into()]).message;
        let git = "Git would leave such a change out unless the file was added (with `git add -f` \
                   if Git doesn't track it yet).";
        assert!(one.contains(git), "{one}");
        let one = ignored_changes_saved(vec!["a.dll".into()]).message;
        assert!(one.contains(git), "{one}");
    }

    #[test]
    fn the_boltignore_hint_does_not_blame_a_pattern() {
        // A link, a pipe, a huge file or one Bolt can't read has no pattern to fix.
        let hint = SaveError::Ignore { detail: "x".into() }.hint().unwrap();
        assert!(!hint.contains("pattern"), "{hint}");
    }

    #[test]
    fn save_stops_when_a_file_changes_during_the_save() {
        let (dir, repo) = project();
        let name = "growing.log";
        let path = dir.path().join(name);
        fs::write(&path, b"line 1\n").unwrap();
        let top = Dir::open_top(dir.path()).unwrap();
        let seen = top.metadata(OsStr::new(name)).unwrap();
        let mut packs = repo.new_packs().unwrap();
        let got = store_file(
            &mut packs,
            repo.store(),
            &top,
            name,
            name,
            Some(&seen),
            &mut || {},
        );
        assert_eq!(got.unwrap(), (hash_content(b"line 1\n"), 7));
        let mut append = || {
            use std::io::Write;
            let mut f = fs::OpenOptions::new().append(true).open(&path).unwrap();
            f.write_all(b"line 2\n").unwrap();
        };
        assert!(matches!(
            store_file(
                &mut packs,
                repo.store(),
                &top,
                name,
                name,
                Some(&seen),
                &mut append
            ),
            Err(SaveError::ChangedDuringSave { .. })
        ));
    }

    /// The packs in the store, by name.
    #[cfg(feature = "fault-injection")]
    fn pack_names(repo: &Repository) -> Vec<std::ffi::OsString> {
        let packs = fs::read_dir(repo.packs_dir()).unwrap();
        let mut names: Vec<_> = packs.map(|e| e.unwrap().file_name()).collect();
        names.sort();
        names
    }

    /// Saves with `change` run between the walk and the reading of what it found: the save must
    /// stop at `a.txt`, and store and change nothing.
    #[cfg(feature = "fault-injection")]
    fn assert_changed_after_the_walk(repo: &mut Repository, change: impl FnOnce() + 'static) {
        let (head, packs) = (repo.head().unwrap(), pack_names(repo));
        crate::fsio::fault::act_at("save:walked", change);
        match save_now(repo) {
            Err(SaveError::ChangedDuringSave { path }) => assert_eq!(path, "a.txt"),
            other => panic!("{:?}", other.map(|_| ())),
        }
        assert_eq!(repo.head().unwrap(), head);
        assert_eq!(pack_names(repo), packs);
    }

    /// Sets when `path` last changed.
    #[cfg(feature = "fault-injection")]
    fn set_time(path: &Path, time: std::time::SystemTime) {
        let file = fs::File::options().write(true).open(path).unwrap();
        file.set_modified(time).unwrap();
    }

    /// The file the walk saw, rewritten before it is read with as many bytes and another time.
    #[cfg(feature = "fault-injection")]
    #[test]
    fn a_file_rewritten_after_the_walk_is_refused() {
        let (dir, mut repo) = project();
        let a = dir.path().join("a.txt");
        fs::write(&a, b"one").unwrap();
        let walked = fs::metadata(&a).unwrap().modified().unwrap();
        assert_changed_after_the_walk(&mut repo, move || {
            fs::write(&a, b"two").unwrap();
            set_time(&a, walked + std::time::Duration::from_secs(10));
        });
    }

    /// Another file renamed over the one the walk saw, with its size and the time the walk saw:
    /// only the file ID tells them apart.
    #[cfg(feature = "fault-injection")]
    #[test]
    fn a_file_renamed_over_after_the_walk_is_refused_by_its_id() {
        let (dir, mut repo) = project();
        let (a, b) = (dir.path().join("a.txt"), dir.path().join("b.txt"));
        fs::write(&a, b"one").unwrap();
        let walked = fs::metadata(&a).unwrap().modified().unwrap();
        assert_changed_after_the_walk(&mut repo, move || {
            fs::write(&b, b"two").unwrap();
            set_time(&b, walked);
            fs::rename(&b, &a).unwrap();
        });
    }

    /// Scans a project holding `victim.txt`, lets `make_link` put a link in its place (it says
    /// whether it could make one), then reads the files as `save` does after its scan.
    fn assert_swap_for_a_link_is_refused(make_link: impl FnOnce(&Path) -> bool) {
        let (dir, repo) = project();
        let victim = dir.path().join("victim.txt");
        fs::write(&victim, b"x").unwrap();
        let Scan {
            mut handles,
            folders,
            ..
        } = walk_project(&repo, SaveLimits::default()).unwrap();
        fs::remove_file(&victim).unwrap();
        if !make_link(&victim) {
            return;
        }
        let mut packs = repo.new_packs().unwrap();
        match build(
            repo.store(),
            &mut packs,
            &mut handles,
            &folders,
            &HashMap::new(),
            &HashSet::new(),
            &HashSet::new(),
        ) {
            Err(SaveError::ChangedDuringSave { path }) => assert_eq!(path, "victim.txt"),
            other => panic!("{other:?}"),
        }
        assert!(packs.is_empty());
    }

    #[test]
    fn file_replaced_by_a_link_between_scan_and_read_is_refused() {
        let outside = tempfile::tempdir().unwrap();
        let secret = outside.path().join("id_ed25519");
        fs::write(&secret, b"hunter2").unwrap();
        assert_swap_for_a_link_is_refused(|victim| crate::test_support::symlink(&secret, victim));
    }

    /// Making a junction needs no privilege, so this runs on every Windows account.
    #[cfg(windows)]
    #[test]
    fn file_replaced_by_a_junction_between_scan_and_read_is_refused() {
        let outside = tempfile::tempdir().unwrap();
        assert_swap_for_a_link_is_refused(|victim| {
            crate::test_support::junction(victim, outside.path());
            true
        });
    }

    #[test]
    fn file_deleted_between_scan_and_read_is_still_unreadable() {
        let (dir, repo) = project();
        let mut packs = repo.new_packs().unwrap();
        let top = Dir::open_top(dir.path()).unwrap();
        match store_file(
            &mut packs,
            repo.store(),
            &top,
            "gone.txt",
            "gone.txt",
            None,
            &mut || {},
        ) {
            Err(SaveError::Unreadable { source, .. }) => {
                assert_eq!(source.kind(), io::ErrorKind::NotFound);
            }
            other => panic!("{other:?}"),
        }
    }

    #[cfg(unix)]
    #[test]
    fn file_replaced_by_a_pipe_between_scan_and_read_is_refused_without_hanging() {
        let (dir, repo) = project();
        // `.boltignore` is saved like any other file, so it can be swapped like one.
        let pipe = dir.path().join(".boltignore");
        let made = std::process::Command::new("mkfifo")
            .arg(&pipe)
            .status()
            .unwrap();
        assert!(made.success());
        // Opening a named pipe waits for a writer, so the read runs on another thread: a hang
        // fails this test instead of stopping the whole run.
        let (tx, rx) = std::sync::mpsc::channel();
        let top = Dir::open_top(dir.path()).unwrap();
        let seen = top.metadata(pipe.file_name().unwrap()).unwrap();
        std::thread::spawn(move || {
            let mut packs = repo.new_packs().unwrap();
            let name = ".boltignore";
            let seen = Some(&seen);
            let stored = store_file(&mut packs, repo.store(), &top, name, name, seen, &mut || {});
            let _ = tx.send((stored, packs.is_empty()));
        });
        let (stored, pack_is_empty) = rx
            .recv_timeout(std::time::Duration::from_secs(10))
            .expect("reading a named pipe hung");
        assert!(
            matches!(stored, Err(SaveError::ChangedDuringSave { .. })),
            "{stored:?}"
        );
        assert!(pack_is_empty);
    }
}
