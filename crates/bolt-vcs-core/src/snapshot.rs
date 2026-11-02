//! `bolt save`: walks the project folder, stores its files, and commits the save.

use std::cmp::Reverse;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt::Write as _;
use std::fs::{self, File, Metadata, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;

use ignore::WalkBuilder;

use crate::diary::{Action, DiaryEntry, Position};
use crate::diff::count_changes;
use crate::drive::{self, Drive};
use crate::history;
use crate::id::{ChangeId, ContentHasher, Id, hash_content};
use crate::ignore::{Decision, Matcher, Rules};
use crate::listing::{Entry, EntryKind, Listing, MAX_SAVE_DEPTH, SaveLimits};
use crate::names::{PortabilityProblem, folder_problems, holdable_here, validate_name};
use crate::pack::{ObjectKind, PackError, PackSet};
use crate::repo::{RepoError, Repository};
use crate::safe_fs;
use crate::save::{EXTRA_LINE, Person, Save};
use crate::store::{Store, StoreError};
use crate::text::escape;
use crate::warning::{self, Warning};

pub struct SaveRequest<'a> {
    pub message: &'a str,
    pub who: &'a Person,
    /// The command as the diary records it; already redacted.
    pub command: &'a str,
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
         as it was because this computer doesn't hold it as it is, and a save can't hold both \
         under one name.",
        escape(path, false),
        escape(kept, false)
    )]
    InTheWay { path: String, kept: String },
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
                _ => return None,
            }
            .to_owned(),
        )
    }

    /// The same error, naming the file by its project path instead of its full path.
    fn at(self, rel: &str) -> SaveError {
        match self {
            SaveError::Unreadable { source, .. } => SaveError::Unreadable {
                path: rel.to_owned(),
                source,
            },
            SaveError::ChangedDuringSave { .. } => SaveError::ChangedDuringSave {
                path: rel.to_owned(),
            },
            SaveError::SpecialFile { .. } => SaveError::SpecialFile {
                path: rel.to_owned(),
            },
            other => other,
        }
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
    let drive = drive::probe(repo.tmp_dir()).map_err(RepoError::from)?;
    let base = Base {
        store: repo.store(),
        root: parent.root,
        limits: SaveLimits::default(),
    };
    let Scan {
        folders,
        old,
        mut warnings,
    } = walk(repo.root(), base, SaveLimits::default(), drive)?;
    check_names(&folders, &old)?;
    let mut packs = repo.new_packs()?;
    let (root, kept) = build(repo.store(), &mut packs, &folders, &old)?;
    if kept > 0 {
        warnings.push(kept_warning(kept));
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
    // Worked out before the commit, so that nothing after it can fail the save.
    let short_change_id =
        history::short_change_id(repo, &save_id, &save.change_id).unwrap_or_else(|why| {
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
        "Kept 1 entry as it was in the last save, because this computer doesn't hold it as it \
         is, like a project link or a name it can't create. A later version's `bolt status` will \
         list it."
            .to_owned()
    } else {
        format!(
            "Kept {count} entries as they were in the last save, because this computer doesn't \
             hold them as they are, like project links or names it can't create. A later \
             version's `bolt status` will list them."
        )
    };
    Warning::new(warning::KEPT_AS_THEY_WERE, message)
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
}

struct Child {
    name: String,
    kind: Kind,
    path: PathBuf,
    /// Its project path, with `/` between names.
    rel: String,
    /// The `x` bits, where the system has them.
    executable: Option<bool>,
}

/// Every folder by project path (`""` is the top folder), with its children sorted by name.
type Folders = BTreeMap<String, Vec<Child>>;

/// A project path as the user should see it; names that aren't UTF-8 are shown lossily.
fn shown(rel: &Path) -> String {
    rel.iter()
        .map(|c| c.to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

#[cfg(unix)]
fn executable_bits(meta: &Metadata) -> Option<bool> {
    use std::os::unix::fs::PermissionsExt;
    Some(meta.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn executable_bits(_meta: &Metadata) -> Option<bool> {
    None
}

/// Whether a folder of this name is Bolt's or Git's, which a save never enters. Where the drive
/// ignores letter case, Git's folder may be spelled `.GIT`.
fn is_internal_folder(name: &str, ignores_case: bool) -> bool {
    name == ".bolt" || name == ".git" || (ignores_case && name.eq_ignore_ascii_case(".git"))
}

/// The project path of `name` in the folder `rel` (`""` for the top folder).
fn path_in(rel: &str, name: &str) -> String {
    if rel.is_empty() {
        name.to_owned()
    } else {
        format!("{rel}/{name}")
    }
}

/// A project path as Git matches it: the bytes of its names, with `/` between them.
fn path_bytes(rel: &Path) -> Vec<u8> {
    let names: Vec<&[u8]> = rel.iter().map(|c| c.as_encoded_bytes()).collect();
    names.join(&b'/')
}

/// The save a new one builds on. A walk reads its listings along the folders it keeps, where the
/// walk's depth limit holds them too, and through the folders gone from disk.
struct Base<'a> {
    store: &'a Store,
    root: Id,
    /// Every listing read counts against these, a shared one once per path.
    limits: SaveLimits,
}

impl Base<'_> {
    fn listing(&mut self, id: Id) -> Result<Listing, SaveError> {
        let listing = self.store.listing(&id)?;
        self.limits
            .add(listing.entries().len())
            .map_err(|detail| StoreError::SaveTooLarge { detail })?;
        Ok(listing)
    }
}

/// The walk met `path` before reading what the base save holds in the folder above it. It hands
/// over every folder before anything in it, so this is a fault in Bolt, or a folder that became
/// a link mid-walk (a known limit, security.md T20).
fn unnoted(path: &str) -> SaveError {
    let fault = format!(
        "Bolt reached {} before reading what the last save holds in the folder above it, so \
         nothing was saved. This is a fault in Bolt: please report it.",
        escape(path, false)
    );
    RepoError::from(io::Error::other(fault)).into()
}

/// The ignore rules of the folders a walk is inside, what the base save holds there, and the
/// first problem found.
struct IgnoreRules {
    root: PathBuf,
    matcher: Matcher,
    /// The bytes of every ignore file the walk has read (storage-format.md §12).
    read: u64,
    problem: Option<SaveError>,
    /// One for each `.gitignore` skipped.
    warnings: Vec<Warning>,
    /// The base save's listing of each folder the walk has noted, by project path, or `None`
    /// where the base save has no such folder. A folder not here hasn't been noted.
    old: HashMap<String, Option<Listing>>,
    /// Ignored folders the walk goes into only for the base save's entries in them.
    held: HashSet<String>,
    drive: Drive,
    /// The base save's entries the walk leaves untouched, by project path: the new save keeps
    /// them as they were, whatever the folder holds there.
    untouched: HashMap<String, Child>,
}

impl IgnoreRules {
    /// The rules for a walk of `root` on `drive`, beginning with its own ignore files. `top` is
    /// the base save's top listing.
    fn new(root: &Path, drive: Drive, top: Listing) -> Result<IgnoreRules, SaveError> {
        let mut rules = IgnoreRules {
            root: root.to_owned(),
            matcher: Matcher::new(drive),
            read: 0,
            problem: None,
            warnings: Vec::new(),
            old: HashMap::new(),
            held: HashSet::new(),
            drive,
            untouched: HashMap::new(),
        };
        rules.note_untouched("", &top);
        rules.old.insert(String::new(), Some(top));
        rules.enter(root)?;
        Ok(rules)
    }

    /// Reads the base save's listing of `rel`, a folder the walk keeps, if it has that folder
    /// too. The folder above it must have been noted already.
    fn note_folder(&mut self, rel: &str, base: &mut Base) -> Result<(), SaveError> {
        let (up, name) = rel.rsplit_once('/').unwrap_or(("", rel));
        let above = self.old.get(up).ok_or_else(|| unnoted(rel))?;
        let listing = match above.as_ref().and_then(|l| l.get(name)) {
            Some(e) if e.kind == EntryKind::Folder => Some(base.listing(e.id)?),
            _ => None,
        };
        if let Some(listing) = &listing {
            self.note_untouched(rel, listing);
        }
        self.old.insert(rel.to_owned(), listing);
        Ok(())
    }

    /// Notes the entries of `listing`, the base save's listing of the folder `rel`, that this
    /// computer doesn't hold as they are: project links, names this system can't hold, and
    /// names that are one file on this drive (decisions 036 and 039). A link Git wrote as a file
    /// is `build`'s to spot, from the file it reads anyway.
    fn note_untouched(&mut self, rel: &str, listing: &Listing) {
        let compared: Vec<String> = listing
            .entries()
            .iter()
            .map(|e| self.drive.compared(&e.name))
            .collect();
        let mut seen: HashMap<&str, usize> = HashMap::new();
        for name in &compared {
            *seen.entry(name).or_default() += 1;
        }
        for (e, name) in listing.entries().iter().zip(&compared) {
            let one_file = seen.get(name.as_str()).is_some_and(|&n| n > 1);
            if e.kind == EntryKind::ProjectLink || !holdable_here(&e.name) || one_file {
                let path = path_in(rel, &e.name);
                let kept = Child {
                    name: e.name.clone(),
                    kind: Kind::Kept {
                        kind: e.kind,
                        id: e.id,
                        size: e.size,
                    },
                    path: self.root.join(&path),
                    rel: path.clone(),
                    executable: None,
                };
                self.untouched.insert(path, kept);
            }
        }
    }

    /// After the walk, which found `found`: looks through the base save's folders it didn't find
    /// on disk for what is kept as it was there. Such an entry stays even when its folder is
    /// gone (decisions 019 and 036).
    fn note_gone(&mut self, base: &mut Base, found: &Folders) -> Result<(), SaveError> {
        let mut gone = Vec::new();
        for (rel, listing) in &self.old {
            let inner = listing.iter().flat_map(Listing::entries);
            for e in inner.filter(|e| e.kind == EntryKind::Folder) {
                let path = path_in(rel, &e.name);
                if !found.contains_key(&path) && !self.untouched.contains_key(&path) {
                    gone.push((path, e.id));
                }
            }
        }
        // In path order, so a damaged base save is reported the same way every time.
        gone.sort();
        for (rel, id) in gone {
            self.note_gone_tree(base, rel, id)?;
        }
        Ok(())
    }

    /// Notes what is kept as it was in the gone folder `rel` (listing `id`) and below it. As in
    /// the diff walk, only the folders from `rel` down to the one being read are held, and one
    /// goes into `old` only when something in or below it is kept.
    fn note_gone_tree(&mut self, base: &mut Base, rel: String, id: Id) -> Result<(), SaveError> {
        let depth = rel.split('/').count();
        let mut path = rel;
        // Each open folder's listing, the next of its entries to look at, and its path's length.
        let mut open: Vec<(Listing, usize, usize)> = Vec::new();
        // How many of the open folders, from the top, are in `old`.
        let mut known = 0;
        let mut read = Some(id);
        loop {
            // Past its limits a base save is invalid and nothing there can be kept, so the search
            // ends and the save goes ahead, as one beside a save too large to compare does.
            if let Some(id) = read.take()
                && base.limits.check_depth(depth + open.len()).is_ok()
            {
                let listing = base.store.listing(&id)?;
                if base.limits.add(listing.entries().len()).is_err() {
                    return Ok(());
                }
                let kept = self.untouched.len();
                self.note_untouched(&path, &listing);
                open.push((listing, 0, path.len()));
                if self.untouched.len() > kept {
                    for (listing, _, end) in &open[known..] {
                        self.old
                            .insert(path[..*end].to_owned(), Some(listing.clone()));
                    }
                    known = open.len();
                }
            }
            let Some((listing, next, end)) = open.last_mut() else {
                return Ok(());
            };
            path.truncate(*end);
            let Some(e) = listing.entries().get(*next) else {
                open.pop();
                known = known.min(open.len());
                continue;
            };
            *next += 1;
            if e.kind == EntryKind::Folder {
                path.push('/');
                path.push_str(&e.name);
                if !self.untouched.contains_key(&path) {
                    read = Some(e.id);
                }
            }
        }
    }

    /// Puts the rules of the ignore files in `dir` in force: its `.gitignore`, then its
    /// `.boltignore`, whose lines come later so they can add to Git's rules or undo them.
    fn enter(&mut self, dir: &Path) -> Result<(), SaveError> {
        let folder = shown(dir.strip_prefix(&self.root).unwrap_or(dir));
        let mut in_force = self.matcher.in_force_bytes();
        let mut rules = Vec::new();
        for name in [".gitignore", ".boltignore"] {
            rules.extend(self.file_rules(dir, &folder, name, &mut in_force)?);
        }
        if !rules.is_empty() {
            self.matcher.enter(&folder, rules);
        }
        Ok(())
    }

    /// The rules of the ignore file `name` in `dir`, the project's `folder`, or none if there
    /// isn't one. Read through `safe_fs` and held to the limits (storage-format.md §12) before a
    /// rule is read from it: `in_force` is the size of the files in force at `folder` so far. A
    /// `.gitignore` that isn't a regular file is skipped with a warning, as Git skips a linked
    /// one. A folder Bolt may not look into is `Unreadable`.
    fn file_rules(
        &mut self,
        dir: &Path,
        folder: &str,
        name: &str,
        in_force: &mut u64,
    ) -> Result<Option<Rules>, SaveError> {
        let file = dir.join(name);
        let rel = shown(file.strip_prefix(&self.root).unwrap_or(&file));
        let ignore = |detail| SaveError::Ignore { detail };
        let bytes = match safe_fs::read_capped(&file, safe_fs::MAX_IGNORE_FILE_BYTES) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => {
                let meta = match fs::symlink_metadata(&file) {
                    Ok(meta) => Some(meta),
                    Err(source) if source.kind() == io::ErrorKind::NotFound => None,
                    Err(source) => {
                        let path = if folder.is_empty() {
                            "the project folder".to_owned()
                        } else {
                            folder.to_owned()
                        };
                        return Err(SaveError::Unreadable { path, source });
                    }
                };
                // Judged by what the file is: an open can fail for other reasons with the same
                // error kind, and a regular .gitignore that can't be read must stop the save.
                if name == ".gitignore" && meta.is_some_and(|m| !safe_fs::is_regular(&m)) {
                    self.warnings.push(Warning::new(
                        warning::GITIGNORE_SKIPPED,
                        format!(
                            "Bolt skipped {}: it is a link, folder or special file, not a regular \
                             file, so its rules weren't used. Git skips a linked .gitignore too. \
                             Make it a regular file if its rules should apply.",
                            escape(&rel, false)
                        ),
                    ));
                    return Ok(None);
                }
                // `safe_fs` refusals start with the full path; the project path replaces it, so
                // the file is named once.
                let why = e.to_string();
                return Err(ignore(
                    match why.strip_prefix(&file.display().to_string()) {
                        Some(rest) => format!("{rel}{rest}"),
                        None => format!("{rel}: {why}"),
                    },
                ));
            }
        };
        let size = bytes.len() as u64;
        if *in_force + size > safe_fs::MAX_IGNORE_IN_FORCE_BYTES {
            return Err(ignore(format!(
                "{rel}: with the other ignore files in force at its folder, it is larger than {} \
                 bytes",
                safe_fs::MAX_IGNORE_IN_FORCE_BYTES
            )));
        }
        if self.read + size > safe_fs::MAX_IGNORE_SAVE_BYTES {
            return Err(ignore(format!(
                "{rel}: with the other ignore files this save reads, it is larger than {} bytes",
                safe_fs::MAX_IGNORE_SAVE_BYTES
            )));
        }
        *in_force += size;
        self.read += size;
        Ok(Some(Rules::parse(&bytes, folder)))
    }

    /// Whether the walk keeps `path`: never what it leaves untouched, and what Git would ignore
    /// stays out, unless the base save has it. After a problem, such as an ignore file that
    /// fails, the problem is kept and nothing more is: the save will stop anyway.
    fn keeps(&mut self, path: &Path, is_dir: bool) -> bool {
        if self.problem.is_some() {
            return false;
        }
        let rel = path.strip_prefix(&self.root).unwrap_or(path);
        // Before any rule: nothing at an untouched entry's path is read, a link's folder included.
        if self.untouched.contains_key(&shown(rel)) {
            return false;
        }
        let folder = shown(rel.parent().unwrap_or(rel));
        // The walk is depth first, so the rules of folders `path` is not in are done with.
        self.matcher.leave_to(&folder);
        // Inside an ignored folder everything is ignored, as in Git: no line takes a name back.
        if self.held.contains(&folder)
            || self.matcher.decide(path_bytes(rel), is_dir) == Decision::Ignored
        {
            return self.saved_before(&folder, rel, is_dir);
        }
        if is_dir && let Err(problem) = self.enter(path) {
            self.problem.get_or_insert(problem);
            return false;
        }
        true
    }

    /// Whether the base save has the ignored `rel` under that name in `folder`, as a folder if
    /// and only if it is one now: what is saved stays saved, as in Git (decision 038). A folder
    /// kept this way is held: its ignore files aren't read, and it keeps only what was saved.
    fn saved_before(&mut self, folder: &str, rel: &Path, is_dir: bool) -> bool {
        // Taking "not read" for "not saved" would record saved files as removed.
        let Some(listing) = self.old.get(folder) else {
            self.problem.get_or_insert(unnoted(&shown(rel)));
            return false;
        };
        let saved = rel
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(|name| listing.as_ref()?.get(name))
            .is_some_and(|e| (e.kind == EntryKind::Folder) == is_dir);
        if saved && is_dir {
            self.held.insert(shown(rel));
        }
        saved
    }
}

/// Stack for the scan thread: 4 KiB per folder level the depth limit allows. Dropping the linked
/// `ignore` matchers of a deep tree recurses once per level and needs about 2 MiB in a Windows
/// debug build, more than the main thread's 1 MiB. Unused stack costs address space only.
const WALK_STACK_BYTES: usize = (MAX_SAVE_DEPTH + 2) * 4096;

/// What a walk of the project folder found.
struct Scan {
    folders: Folders,
    /// The base save's listing of each folder in `folders`, if it has that folder.
    old: HashMap<String, Option<Listing>>,
    /// One for each `.gitignore` skipped.
    warnings: Vec<Warning>,
}

/// Scans the project folder on a thread of its own, so callers don't need a deep stack. `drive`
/// is what `drive::probe` found there.
fn walk(root: &Path, base: Base, limits: SaveLimits, drive: Drive) -> Result<Scan, SaveError> {
    thread::scope(|scope| {
        let scan = thread::Builder::new()
            .stack_size(WALK_STACK_BYTES)
            .spawn_scoped(scope, || scan_folder(root, base, limits, drive))
            .map_err(|e| SaveError::Repo(e.into()))?;
        scan.join().unwrap_or_else(|_| {
            let stopped = io::Error::other("the scan of the project folder stopped unexpectedly");
            Err(SaveError::Repo(stopped.into()))
        })
    })
}

fn scan_folder(
    root: &Path,
    mut base: Base,
    mut limits: SaveLimits,
    drive: Drive,
) -> Result<Scan, SaveError> {
    let too_large = |detail| SaveError::TooLarge { detail };
    // Limits are checked during the scan, so a tree too deep or too big is refused before it is all
    // read.
    let mut folders = Folders::from([(String::new(), Vec::new())]);
    let top = base.listing(base.root)?;
    // Bolt reads each ignore file itself: `ignore` would reopen it by path, following links and
    // reading without a limit.
    let rules = Arc::new(Mutex::new(IgnoreRules::new(root, drive, top)?));
    let walker = WalkBuilder::new(root)
        .hidden(false)
        .git_ignore(false)
        .git_exclude(false)
        .git_global(false)
        .ignore(false)
        .parents(false)
        .follow_links(false)
        .filter_entry({
            let rules = Arc::clone(&rules);
            move |e| {
                let is_dir = e.file_type().is_some_and(|t| t.is_dir());
                let internal = |name: &str| is_internal_folder(name, drive.ignores_case);
                if is_dir && e.file_name().to_str().is_some_and(internal) {
                    return false;
                }
                let mut rules = rules.lock().unwrap_or_else(PoisonError::into_inner);
                rules.keeps(e.path(), is_dir)
            }
        })
        .build();
    for item in walker {
        let entry = item.map_err(|e| walk_error(root, e))?;
        let path = entry.path();
        let rel_path = path.strip_prefix(root).unwrap_or(path);
        let rel = shown(rel_path);
        let mut names = Vec::new();
        for part in rel_path.iter() {
            names.push(part.to_str().ok_or_else(|| SaveError::InvalidName {
                path: rel.clone(),
                reason: "its name is not valid UTF-8".into(),
            })?);
        }
        let Some((name, parents)) = names.split_last() else {
            continue; // the project folder itself
        };
        validate_name(name).map_err(|e| SaveError::InvalidName {
            path: rel.clone(),
            reason: e.to_string(),
        })?;
        let meta = fs::symlink_metadata(path).map_err(|source| SaveError::Unreadable {
            path: rel.clone(),
            source,
        })?;
        let t = meta.file_type();
        let kind = if t.is_symlink() {
            Kind::Symlink
        } else if t.is_dir() {
            limits.check_depth(names.len()).map_err(too_large)?;
            folders.entry(rel.clone()).or_default();
            // The walk hands a folder over before it asks about anything inside.
            rules
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .note_folder(&rel, &mut base)?;
            Kind::Folder
        } else if t.is_file() {
            Kind::File
        } else {
            // shortcut: sockets, pipes and devices are skipped silently; revisit with `bolt status`.
            continue;
        };
        limits.add(1).map_err(too_large)?;
        folders.entry(parents.join("/")).or_default().push(Child {
            name: (*name).to_owned(),
            kind,
            path: path.to_owned(),
            rel,
            executable: executable_bits(&meta),
        });
    }
    let mut rules = rules.lock().unwrap_or_else(PoisonError::into_inner);
    // Going on without the rules of an ignore file that failed would save what it keeps out.
    if let Some(problem) = rules.problem.take() {
        return Err(problem);
    }
    rules.note_gone(&mut base, &folders)?;
    // In path order, so a save that stops names the same entry every time.
    let mut kept: Vec<Child> = rules.untouched.drain().map(|(_, c)| c).collect();
    kept.sort_by(|a, b| a.rel.cmp(&b.rel));
    for c in kept {
        if let Kind::Kept {
            kind: EntryKind::Folder,
            id,
            ..
        } = c.kind
        {
            count_kept_folder(base.store, &mut limits, &c.rel, id)?;
        }
        put_back(root, &mut folders, &mut limits, c)?;
    }
    for children in folders.values_mut() {
        children.sort_by(|a, b| a.name.cmp(&b.name));
    }
    Ok(Scan {
        folders,
        old: std::mem::take(&mut rules.old),
        warnings: std::mem::take(&mut rules.warnings),
    })
}

/// Counts everything the kept folder `rel` holds, from its listing `id` down, against `limits`,
/// one listing at a time: kept or not, a save holds no more than its format allows.
fn count_kept_folder(
    store: &Store,
    limits: &mut SaveLimits,
    rel: &str,
    id: Id,
) -> Result<(), SaveError> {
    let too_large = |detail| SaveError::TooLarge { detail };
    let mut todo = vec![(id, rel.split('/').count())];
    while let Some((id, depth)) = todo.pop() {
        limits.check_depth(depth).map_err(too_large)?;
        let listing = store.listing(&id)?;
        limits.add(listing.entries().len()).map_err(too_large)?;
        let inner = listing
            .entries()
            .iter()
            .filter(|e| e.kind == EntryKind::Folder);
        todo.extend(inner.map(|e| (e.id, depth + 1)));
    }
    Ok(())
}

/// Puts an entry kept as it was into its folder. A folder above it that is gone from disk comes
/// back holding only what is kept; if something else on disk now has its name, the save stops.
fn put_back(
    root: &Path,
    folders: &mut Folders,
    limits: &mut SaveLimits,
    kept: Child,
) -> Result<(), SaveError> {
    let too_large = |detail| SaveError::TooLarge { detail };
    let kept_path = kept.rel.clone();
    let mut child = kept;
    loop {
        limits.add(1).map_err(too_large)?;
        let (up, _) = child.rel.rsplit_once('/').unwrap_or_default();
        let up = up.to_owned();
        if let Some(children) = folders.get_mut(&up) {
            // The walk never takes in a kept entry's own name, but a folder brought back can meet
            // what it found.
            if child.kind == Kind::Folder
                && let Some(other) = children.iter().find(|c| c.name == child.name)
            {
                return Err(SaveError::InTheWay {
                    path: other.rel.clone(),
                    kept: kept_path,
                });
            }
            children.push(child);
            return Ok(());
        }
        let depth = up.split('/').count();
        limits.check_depth(depth).map_err(too_large)?;
        let name = up.rsplit('/').next().unwrap_or_default().to_owned();
        folders.insert(up.clone(), vec![child]);
        child = Child {
            name,
            kind: Kind::Folder,
            path: root.join(&up),
            rel: up,
            executable: None,
        };
    }
}

/// An `ignore` error with paths shown as project paths.
fn describe(root: &Path, err: &ignore::Error) -> String {
    match err {
        ignore::Error::WithPath { path, err } => {
            let rel = shown(path.strip_prefix(root).unwrap_or(path));
            format!("{rel}: {}", describe(root, err))
        }
        ignore::Error::WithLineNumber { line, err } => {
            format!("line {line}: {}", describe(root, err))
        }
        ignore::Error::WithDepth { err, .. } => describe(root, err),
        ignore::Error::Partial(errs) => errs
            .iter()
            .map(|e| describe(root, e))
            .collect::<Vec<_>>()
            .join("; "),
        other => other.to_string(),
    }
}

fn walk_error(root: &Path, err: ignore::Error) -> SaveError {
    fn path_of(err: &ignore::Error) -> Option<&Path> {
        match err {
            ignore::Error::WithPath { path, .. } => Some(path),
            ignore::Error::WithDepth { err, .. } | ignore::Error::WithLineNumber { err, .. } => {
                path_of(err)
            }
            _ => None,
        }
    }
    let path = path_of(&err).map(|p| shown(p.strip_prefix(root).unwrap_or(p)));
    match (err.io_error(), path) {
        (Some(e), Some(path)) => SaveError::Unreadable {
            path,
            source: io::Error::new(e.kind(), e.to_string()),
        },
        (Some(e), None) => RepoError::from(io::Error::new(e.kind(), e.to_string())).into(),
        // The walker reads no ignore files itself, so this is not expected; it fails closed.
        (None, _) => SaveError::Ignore {
            detail: describe(root, &err),
        },
    }
}

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

/// Stores every file and listing, deepest folders first. Returns the top listing's ID and how
/// many entries were kept as they were.
fn build(
    store: &Store,
    packs: &mut PackSet,
    folders: &Folders,
    old: &HashMap<String, Option<Listing>>,
) -> Result<(Id, usize), SaveError> {
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
                    let (id, size) =
                        store_file(packs, store, &c.path, &mut || {}).map_err(|e| e.at(&c.rel))?;
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
                        let executable = c.executable.unwrap_or_else(|| {
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
                    let (id, size) = store_link(packs, store, &c.path).map_err(|e| e.at(&c.rel))?;
                    (EntryKind::Symlink, id, size)
                }
                Kind::Kept { kind, id, size } => {
                    kept += 1;
                    (kind, id, size)
                }
            };
            entries.push(Entry {
                name: c.name.clone(),
                kind,
                id,
                size,
            });
        }
        let listing = Listing::new(entries).map_err(StoreError::from)?;
        let id = listing.id();
        let encoded = listing.encode();
        if !store.has_good_copy(&id, ObjectKind::FolderListing, encoded.len() as u64)? {
            packs.add_record(ObjectKind::FolderListing, &encoded)?;
        }
        built.insert(rel, id);
    }
    let root = built.remove("").ok_or_else(missing)?;
    Ok((root, kept))
}

/// Stores a symlink's target as content; the link itself is never followed.
fn store_link(packs: &mut PackSet, store: &Store, path: &Path) -> Result<(Id, u64), SaveError> {
    let unreadable = |source| SaveError::Unreadable {
        path: path.display().to_string(),
        source,
    };
    let target = fs::read_link(path).map_err(unreadable)?;
    let target = target.into_os_string().into_string().map_err(|_| {
        unreadable(io::Error::new(
            io::ErrorKind::InvalidData,
            "the link's target is not valid UTF-8",
        ))
    })?;
    let id = hash_content(target.as_bytes());
    if !store.has_good_copy(&id, ObjectKind::Content, target.len() as u64)? {
        packs.add_content(target.as_bytes())?;
    }
    Ok((id, target.len() as u64))
}

/// Stores one file in two passes: hash it, then copy it into `packs` unless already held. A file
/// that changes in between, or turns into a link or special file, gives `ChangedDuringSave`; a
/// Windows reparse point that isn't a link gives `SpecialFile`.
pub(crate) fn store_file(
    packs: &mut PackSet,
    store: &Store,
    path: &Path,
    between_passes: &mut dyn FnMut(),
) -> Result<(Id, u64), SaveError> {
    let unreadable = |source| SaveError::Unreadable {
        path: path.display().to_string(),
        source,
    };
    let changed = || SaveError::ChangedDuringSave {
        path: path.display().to_string(),
    };
    let stamp = |f: &File| f.metadata().map(|m| (m.len(), m.modified().ok()));
    // A link or special file here was swapped in since the scan, unless it is a Windows reparse
    // point that isn't a link (a cloud placeholder, say), which the scan takes for a file.
    let mut file = safe_fs::open_regular(path, OpenOptions::new().read(true)).map_err(|e| {
        if e.kind() != io::ErrorKind::InvalidInput {
            unreadable(e)
        } else if fs::symlink_metadata(path).is_ok_and(|m| safe_fs::is_reparse_file(&m)) {
            SaveError::SpecialFile {
                path: path.display().to_string(),
            }
        } else {
            changed()
        }
    })?;
    let before = stamp(&file).map_err(unreadable)?;
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
    use crate::id::hash_content;
    use crate::listing::{EntryKind, MAX_SAVE_DEPTH, MAX_SAVE_ENTRIES};
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

    fn project() -> (tempfile::TempDir, Repository) {
        let dir = tempfile::tempdir().unwrap();
        let repo = Repository::init(dir.path(), &who()).unwrap();
        (dir, repo)
    }

    fn save_now(repo: &mut Repository) -> Result<SaveOutcome, SaveError> {
        let who = who();
        let req = SaveRequest {
            message: "Change things",
            who: &who,
            command: "save",
        };
        save_folder(repo, &req, &mut |_| {})
    }

    /// The root listing of a save that must have happened.
    fn saved_root(outcome: SaveOutcome) -> Id {
        match outcome {
            SaveOutcome::Saved { save, .. } => save.root,
            SaveOutcome::NothingChanged { .. } => panic!("nothing was saved"),
        }
    }

    type Tree = BTreeMap<String, (EntryKind, Vec<u8>)>;

    /// Every entry of a saved folder, by path; folders and project links have no bytes.
    fn saved_tree(store: &Store, root: Id) -> Tree {
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

    fn write(root: &Path, rel: &str, bytes: &[u8]) {
        let path = root.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }

    /// The drive the tests run on, as `save` probes it.
    fn this_drive() -> Drive {
        drive::probe(tempfile::tempdir().unwrap().path()).unwrap()
    }

    /// The line's newest save, as the base of a walk, with the format's limits.
    fn newest(repo: &Repository) -> Base<'_> {
        let (_, entry) = repo.current().unwrap();
        let id = entry.state.lines["main"];
        let save = repo.store().save(&id, repo.signatures_allowed()).unwrap();
        Base {
            store: repo.store(),
            root: save.root,
            limits: SaveLimits::default(),
        }
    }

    /// `walk` of the project on top of the line's newest save, as `save` does.
    fn walk_project(repo: &Repository, limits: SaveLimits) -> Result<Scan, SaveError> {
        walk(repo.root(), newest(repo), limits, this_drive())
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
    fn damage_the_only_copy_of(repo: &Repository, id: &Id) {
        for e in fs::read_dir(repo.packs_dir()).unwrap() {
            let path = e.unwrap().path();
            let found = PackReader::open(&path).unwrap().find(id).copied();
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

    #[test]
    fn boltignore_git_and_bolt_folders_are_skipped() {
        let (dir, mut repo) = project();
        let root = dir.path();
        write(root, ".boltignore", b"*.tmp\nLibrary/\n");
        write(root, "a.tmp", b"x");
        write(root, "keep.txt", b"x");
        write(root, "Library/x", b"x");
        write(root, ".git/config", b"x");
        write(root, "sub/.git/config", b"x");
        write(root, "sub/.boltignore", b"local.txt\n");
        write(root, "sub/local.txt", b"x");
        write(root, "local.txt", b"x");
        let id = saved_root(save_now(&mut repo).unwrap());
        let tree = saved_tree(repo.store(), id);
        let paths: Vec<&str> = tree.keys().map(String::as_str).collect();
        assert_eq!(
            paths,
            [
                ".boltignore",
                "keep.txt",
                "local.txt",
                "sub",
                "sub/.boltignore"
            ]
        );
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
    fn boltignore_rules_work_as_in_git() {
        let (dir, mut repo) = project();
        let root = dir.path();
        // A byte order mark is not part of the first pattern, and the last match wins. `build/` is
        // folders only, nothing returns from inside an ignored folder, and `.git` stays out.
        let top = "\u{feff}*.log\n!keep.log\nbuild/\n!build/keep.txt\n!.git/\n";
        write(root, ".boltignore", top.as_bytes());
        write(root, "a.log", b"x");
        write(root, "keep.log", b"x");
        write(root, "build/out.bin", b"x");
        write(root, "build/keep.txt", b"x");
        write(root, "src/build", b"a file, not a folder");
        write(root, ".git/config", b"x");
        // The nearest .boltignore decides first: `sub` takes one .log file back, and the top
        // one still ignores the other.
        write(root, "sub/.boltignore", b"!debug.log\n");
        write(root, "sub/debug.log", b"x");
        write(root, "sub/other.log", b"x");
        let id = saved_root(save_now(&mut repo).unwrap());
        let tree = saved_tree(repo.store(), id);
        let paths: Vec<&str> = tree.keys().map(String::as_str).collect();
        assert_eq!(
            paths,
            [
                ".boltignore",
                "keep.log",
                "src",
                "src/build",
                "sub",
                "sub/.boltignore",
                "sub/debug.log"
            ]
        );
    }

    /// The paths of the newest save, sorted.
    fn saved_paths(repo: &Repository, outcome: SaveOutcome) -> Vec<String> {
        let tree = saved_tree(repo.store(), saved_root(outcome));
        tree.into_keys().collect()
    }

    #[test]
    fn boltignore_ignores_letter_case_where_git_does() {
        let (dir, mut repo) = project();
        let root = dir.path();
        write(root, ".boltignore", b"*.env\nsecrets/\n*.LOG\n");
        write(root, "PROD.ENV", b"x");
        write(root, "Secrets/key.pem", b"x");
        write(root, "debug.LOG", b"x");
        write(root, "keep.txt", b"x");
        let saved = save_now(&mut repo).unwrap();
        // Git sets core.ignorecase where the drive ignores letter case, as Windows' and macOS's
        // do; elsewhere a pattern matches exactly.
        let want: &[&str] = if this_drive().ignores_case {
            &[".boltignore", "keep.txt"]
        } else {
            &[
                ".boltignore",
                "PROD.ENV",
                "Secrets",
                "Secrets/key.pem",
                "keep.txt",
            ]
        };
        assert_eq!(saved_paths(&repo, saved), want);
    }

    #[test]
    fn a_name_taken_back_with_a_pattern_keeps_its_letter_case_where_case_is_ignored() {
        let (dir, mut repo) = project();
        let root = dir.path();
        write(root, ".boltignore", b"*.env\n!keep.env\n");
        write(root, "KEEP.ENV", b"x");
        write(root, "OTHER.ENV", b"x");
        write(root, "plain.txt", b"x");
        let saved = save_now(&mut repo).unwrap();
        let want: &[&str] = if this_drive().ignores_case {
            &[".boltignore", "KEEP.ENV", "plain.txt"]
        } else {
            &[".boltignore", "KEEP.ENV", "OTHER.ENV", "plain.txt"]
        };
        assert_eq!(saved_paths(&repo, saved), want);
    }

    #[test]
    fn a_git_folder_is_never_saved_in_any_letter_case_where_git_reads_it_so() {
        let (dir, mut repo) = project();
        let root = dir.path();
        write(root, "keep.txt", b"x");
        write(root, ".GIT/config", b"x");
        write(root, "sub/.Git/HEAD", b"x");
        let saved = save_now(&mut repo).unwrap();
        // Only where the drive ignores letter case is `.GIT` Git's folder; elsewhere it is an
        // ordinary one.
        let want: &[&str] = if this_drive().ignores_case {
            &["keep.txt", "sub"]
        } else {
            &[
                ".GIT",
                ".GIT/config",
                "keep.txt",
                "sub",
                "sub/.Git",
                "sub/.Git/HEAD",
            ]
        };
        assert_eq!(saved_paths(&repo, saved), want);
    }

    #[test]
    fn braces_in_boltignore_are_literal_as_in_git() {
        let (dir, mut repo) = project();
        let root = dir.path();
        let patterns = [
            "{prod,dev}.env", // not a choice
            "\\{kept\\}.txt", // already escaped: still one brace each
            "x{y",            // an unclosed brace is no error
            "}",
        ];
        write(root, ".boltignore", patterns.join("\n").as_bytes());
        for name in ["{prod,dev}.env", "{kept}.txt", "x{y", "}"] {
            write(root, name, b"ignored");
        }
        for name in ["prod.env", "dev.env", "kept.txt", "xy"] {
            write(root, name, b"kept");
        }
        let saved = save_now(&mut repo).unwrap();
        assert_eq!(
            saved_paths(&repo, saved),
            [".boltignore", "dev.env", "kept.txt", "prod.env", "xy"]
        );
    }

    #[test]
    fn lines_bolt_used_to_refuse_work_as_in_git() {
        let (dir, mut repo) = project();
        let root = dir.path();
        let lines = [
            "*.py[!c]",         // every .py? but .pyc
            "[[:alpha:]]*.log", // logs whose names start with a letter
            "a[/]b",            // a class never takes `/`, so this matches nothing
            "trail\\",          // nor does a `\` that escapes nothing
            "bar[",             // nor a `[` that never closes
            "tab\there",        // a tab is a character like any other
            "**\\*.tmp",        // `\` escapes the `*`, even beside `**`
            "cafe\u{301}.txt",  // decomposed, and used as written
            "\\B.md",           // where letter case is ignored, Git leaves this capital alone
        ];
        write(root, ".boltignore", lines.join("\n").as_bytes());
        for name in [
            "x.pyc", "x.pyo", "1.log", "a.log", "a/b", "trail", "bar[", "x.tmp", "B.md",
        ] {
            write(root, name, b"x");
        }
        let decomposed = "cafe\u{301}.txt";
        write(root, decomposed, b"x");
        let drive = this_drive();
        let saved = save_now(&mut repo).unwrap();
        let mut want = vec![
            ".boltignore",
            "1.log",
            "a",
            "a/b",
            "bar[",
            "trail",
            "x.pyc",
            "x.tmp",
        ];
        // Where the drive composes names the decomposed line matches nothing, and where it
        // ignores letter case neither does `\B`.
        if drive.composes_unicode {
            want.push(decomposed);
        }
        if drive.ignores_case {
            want.push("B.md");
        }
        want.sort_unstable();
        assert_eq!(saved_paths(&repo, saved), want);
    }

    #[test]
    fn the_scan_stops_at_the_first_boltignore_problem() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        // The top file leaves no room for another below it.
        write(
            root,
            ".boltignore",
            &comment_of(safe_fs::MAX_IGNORE_IN_FORCE_BYTES),
        );
        write(root, "a/.boltignore", b"#\n");
        write(root, "b/.boltignore", b"#\n");
        let mut rules = IgnoreRules::new(root, this_drive(), Listing::empty()).unwrap();
        assert!(!rules.keeps(&root.join("a"), true));
        // Past the first problem nothing is kept, and no other .boltignore is read.
        assert!(!rules.keeps(&root.join("b"), true));
        assert!(!rules.keeps(&root.join("c.txt"), false));
        let Some(SaveError::Ignore { detail }) = rules.problem else {
            panic!("no problem kept");
        };
        assert!(detail.starts_with("a/.boltignore: "), "{detail}");
        assert_eq!(rules.read, safe_fs::MAX_IGNORE_IN_FORCE_BYTES);
    }

    #[test]
    fn a_question_mark_or_letter_case_never_spans_a_character_beyond_ascii() {
        // As in Git, which matches bytes and folds only ASCII letters.
        let (dir, mut repo) = project();
        let root = dir.path();
        write(root, ".boltignore", "?.txt\nä.md\n".as_bytes());
        write(root, "a.txt", b"x");
        write(root, "ä.txt", b"x");
        write(root, "one/ä.md", b"x");
        write(root, "two/Ä.md", b"x");
        let saved = save_now(&mut repo).unwrap();
        assert_eq!(
            saved_paths(&repo, saved),
            [".boltignore", "one", "two", "two/Ä.md", "ä.txt"]
        );
    }

    #[test]
    fn a_folders_ignore_rules_end_with_the_folder() {
        let (dir, repo) = project();
        let root = dir.path();
        write(root, ".boltignore", b"*.log\n");
        write(root, "a/.boltignore", b"!x.log\n");
        fs::create_dir(root.join("b")).unwrap();
        let mut base = newest(&repo);
        let mut rules = IgnoreRules::new(root, this_drive(), Listing::empty()).unwrap();
        // In walk order: `a` and its file, then `b`, which `a`'s rules no longer reach. The walk
        // notes each folder it keeps before it looks inside.
        assert!(rules.keeps(&root.join("a"), true));
        rules.note_folder("a", &mut base).unwrap();
        assert!(rules.keeps(&root.join("a").join("x.log"), false));
        assert!(rules.keeps(&root.join("b"), true));
        rules.note_folder("b", &mut base).unwrap();
        assert!(!rules.keeps(&root.join("b").join("x.log"), false));
        assert!(rules.problem.is_none());
    }

    #[test]
    fn a_folder_the_walk_never_noted_stops_the_save() {
        let (dir, repo) = project();
        let root = dir.path();
        write(root, ".gitignore", b"*.dll\n");
        let mut rules = IgnoreRules::new(root, this_drive(), Listing::empty()).unwrap();
        // Bolt can't tell whether the last save holds `a/x.dll` without `a`'s listing, so it
        // stops rather than leave out what may be saved.
        assert!(!rules.keeps(&root.join("a").join("x.dll"), false));
        let Some(problem) = rules.problem.take() else {
            panic!("the save would go on");
        };
        assert!(problem.to_string().contains("fault in Bolt"), "{problem}");
        // Nor can a folder be noted before the one above it.
        let err = rules.note_folder("b/c", &mut newest(&repo)).unwrap_err();
        assert!(err.to_string().contains("fault in Bolt"), "{err}");
    }

    #[test]
    fn a_range_that_runs_backwards_matches_nothing_as_in_git() {
        let (dir, mut repo) = project();
        write(dir.path(), "secret.key", b"x");
        write(dir.path(), "m", b"x");
        write(dir.path(), ".boltignore", b"secret.key\n[z-a]\n");
        let saved = save_now(&mut repo).unwrap();
        assert_eq!(saved_paths(&repo, saved), [".boltignore", "m"]);
    }

    /// The save must stop with `Ignore` naming the top folder's ignore file `name` by its project
    /// path, once, and change nothing. Returns the problem's text.
    fn assert_ignore_problem(repo: &mut Repository, name: &str) -> String {
        let head = repo.head().unwrap();
        let detail = match save_now(repo) {
            Err(SaveError::Ignore { detail }) => detail,
            other => panic!("{:?}", other.map(|_| ())),
        };
        assert!(detail.starts_with(name), "{detail}");
        assert_eq!(detail.matches(name).count(), 1, "{detail}");
        assert_eq!(repo.head().unwrap(), head);
        detail
    }

    #[test]
    fn symlinked_boltignore_stops_the_save() {
        let (dir, mut repo) = project();
        let outside = tempfile::tempdir().unwrap();
        let patterns = outside.path().join("patterns");
        fs::write(&patterns, b"*.key\n").unwrap();
        write(dir.path(), "secret.key", b"x");
        if !crate::test_support::symlink(&patterns, &dir.path().join(".boltignore")) {
            return;
        }
        let detail = assert_ignore_problem(&mut repo, ".boltignore");
        assert!(detail.contains("regular file"), "{detail}");
        assert!(!detail.contains(dir.path().to_str().unwrap()), "{detail}");
    }

    #[test]
    fn an_oversized_ignore_file_stops_the_save() {
        // Only a .gitignore that isn't a regular file is skipped.
        for name in [".gitignore", ".boltignore"] {
            let (dir, mut repo) = project();
            // One comment line, 1 MiB + 1 byte with its newline.
            let mut big = vec![b'#'; 1 << 20];
            big.push(b'\n');
            write(dir.path(), name, &big);
            let detail = assert_ignore_problem(&mut repo, name);
            assert!(detail.contains("larger than 1048576 bytes"), "{detail}");
            assert!(!detail.contains(dir.path().to_str().unwrap()), "{detail}");
        }
    }

    /// A `.boltignore` of `len` bytes that holds no pattern.
    fn comment_of(len: u64) -> Vec<u8> {
        vec![b'#'; usize::try_from(len).unwrap()]
    }

    /// What `walk` makes of the ignore files in the project, which must be a problem.
    fn walk_problem(repo: &Repository) -> String {
        match walk_project(repo, SaveLimits::default()) {
            Err(SaveError::Ignore { detail }) => detail,
            other => panic!("{:?}", other.map(|_| ())),
        }
    }

    #[test]
    fn boltignore_files_in_force_at_one_folder_total_at_most_one_mebibyte() {
        let (dir, repo) = project();
        let root = dir.path();
        let half = safe_fs::MAX_IGNORE_IN_FORCE_BYTES / 2;
        write(root, ".boltignore", &comment_of(half));
        write(root, "a/.boltignore", &comment_of(half));
        // `a`'s file is not in force at `b`, so the two folders do not add up.
        write(root, "b/.boltignore", &comment_of(half));
        assert!(walk_project(&repo, SaveLimits::default()).is_ok());
        // Six bytes more below `a` put it past the limit; its pattern is never read.
        write(root, "a/x/.boltignore", b"[z-a]\n");
        let detail = walk_problem(&repo);
        assert!(detail.starts_with("a/x/.boltignore: "), "{detail}");
        assert!(detail.contains("larger than 1048576 bytes"), "{detail}");
        assert!(!detail.contains("[z-a]"), "{detail}");
    }

    #[test]
    fn one_save_reads_at_most_sixteen_mebibytes_of_boltignore_files() {
        let (dir, repo) = project();
        let root = dir.path();
        // As big as a file may be, and not UTF-8: Git reads such a file all the same.
        let mut each = comment_of(safe_fs::MAX_IGNORE_FILE_BYTES - 1);
        each.push(0xff);
        for i in 0..safe_fs::MAX_IGNORE_SAVE_BYTES / safe_fs::MAX_IGNORE_FILE_BYTES {
            write(root, &format!("d{i}/.boltignore"), &each);
        }
        assert!(walk_project(&repo, SaveLimits::default()).is_ok());
        write(root, "e/.boltignore", &comment_of(1));
        // Whichever file the walk reaches last is the one that goes over.
        let detail = walk_problem(&repo);
        assert!(detail.contains("/.boltignore: "), "{detail}");
        assert!(detail.contains("larger than 16777216 bytes"), "{detail}");
    }

    #[test]
    fn large_rule_files_work_whatever_the_drive() {
        // Thousands of paths, and a line of 300,000 `?`: Git reads both, and Bolt keeps no
        // compiled form whose size could grow past a limit.
        let (dir, mut repo) = project();
        let mut text: String = (0..6000)
            .map(|i| format!("Assets/Textures/Characters/Hero/diffuse_{i:05}.png\n"))
            .collect();
        text.push_str(&"?".repeat(300_000));
        write(dir.path(), ".boltignore", text.as_bytes());
        let hero = "Assets/Textures/Characters/Hero";
        write(dir.path(), &format!("{hero}/diffuse_00042.png"), b"x");
        write(dir.path(), &format!("{hero}/normal_00042.png"), b"x");
        let saved = save_now(&mut repo).unwrap();
        assert_eq!(
            saved_paths(&repo, saved),
            [
                ".boltignore",
                "Assets",
                "Assets/Textures",
                "Assets/Textures/Characters",
                hero,
                &format!("{hero}/normal_00042.png"),
            ]
        );
    }

    #[cfg(unix)]
    #[test]
    fn boltignore_pipe_stops_the_save_without_hanging() {
        let (dir, mut repo) = project();
        write(dir.path(), "sub/secret.key", b"x");
        let made = std::process::Command::new("mkfifo")
            .arg(dir.path().join("sub/.boltignore"))
            .status()
            .unwrap();
        assert!(made.success());
        // Opening a named pipe waits for a writer, so the save runs on another thread: a hang
        // fails this test instead of stopping the whole run.
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(save_now(&mut repo).map(|_| ()));
        });
        let result = rx
            .recv_timeout(std::time::Duration::from_secs(10))
            .expect("the save hung on a named pipe");
        match result {
            Err(SaveError::Ignore { detail }) => assert!(
                detail.starts_with("sub/.boltignore is not a regular file"),
                "{detail}"
            ),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_boltignore_that_isnt_utf8_is_read_as_git_reads_it() {
        let (dir, mut repo) = project();
        write(dir.path(), "secret.key", b"x");
        write(dir.path(), "keep.txt", b"x");
        // Git reads bytes: a line that isn't UTF-8 stops nothing, and those after it apply.
        write(dir.path(), ".boltignore", b"# \xff\n\xff*\nsecret.key\n");
        let saved = save_now(&mut repo).unwrap();
        assert_eq!(saved_paths(&repo, saved), [".boltignore", "keep.txt"]);
    }

    #[cfg(unix)]
    #[test]
    fn boltignore_that_is_a_folder_stops_the_save() {
        let (dir, mut repo) = project();
        write(dir.path(), "secret.key", b"x");
        fs::create_dir(dir.path().join(".boltignore")).unwrap();
        assert_ignore_problem(&mut repo, ".boltignore");
    }

    #[test]
    fn an_unreadable_ignore_file_stops_the_save() {
        for name in [".gitignore", ".boltignore"] {
            let (dir, mut repo) = project();
            let head = repo.head().unwrap();
            let rel = format!("sub/{name}");
            write(dir.path(), "sub/secret.key", b"x");
            write(dir.path(), &rel, b"secret.key\n");
            let ignore = dir.path().join(&rel);
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(&ignore, fs::Permissions::from_mode(0o000)).unwrap();
                // root reads everything, so there is nothing to test.
                if fs::read(&ignore).is_ok() {
                    return;
                }
            }
            #[cfg(windows)]
            let _guard = {
                use std::os::windows::fs::OpenOptionsExt;
                fs::OpenOptions::new()
                    .read(true)
                    .share_mode(0)
                    .open(&ignore)
                    .unwrap()
            };
            match save_now(&mut repo) {
                Err(SaveError::Ignore { detail }) => {
                    assert!(detail.starts_with(&format!("{rel}: ")), "{detail}");
                }
                other => panic!("{:?}", other.map(|_| ())),
            }
            assert_eq!(repo.head().unwrap(), head);
        }
    }

    #[test]
    fn gitignore_rules_apply_like_boltignore_rules() {
        let (dir, mut repo) = project();
        let root = dir.path();
        write(root, ".gitignore", b"*.log\n!keep.log\nbuild/\n");
        write(root, "a.log", b"x");
        write(root, "keep.log", b"x");
        write(root, "build/out.bin", b"x");
        // A folder's own .gitignore applies there, and only there.
        write(root, "sub/.gitignore", b"*.tmp\n");
        write(root, "sub/x.tmp", b"x");
        write(root, "x.tmp", b"x");
        let saved = save_now(&mut repo).unwrap();
        assert_eq!(
            saved_paths(&repo, saved),
            [".gitignore", "keep.log", "sub", "sub/.gitignore", "x.tmp"]
        );
    }

    #[test]
    fn boltignore_lines_come_after_gitignore_lines() {
        let (dir, mut repo) = project();
        let root = dir.path();
        write(root, ".gitignore", b"*.log\n");
        write(root, ".boltignore", b"!keep.log\n");
        write(root, "keep.log", b"x");
        write(root, "other.log", b"x");
        let saved = save_now(&mut repo).unwrap();
        assert_eq!(
            saved_paths(&repo, saved),
            [".boltignore", ".gitignore", "keep.log"]
        );
    }

    /// The save's warnings, whether or not anything changed.
    fn warnings_of(outcome: &SaveOutcome) -> &[Warning] {
        match outcome {
            SaveOutcome::Saved { warnings, .. } | SaveOutcome::NothingChanged { warnings, .. } => {
                warnings
            }
        }
    }

    #[test]
    fn a_linked_gitignore_is_skipped_with_a_warning() {
        let (dir, mut repo) = project();
        let outside = tempfile::tempdir().unwrap();
        let patterns = outside.path().join("patterns");
        fs::write(&patterns, b"*.key\n").unwrap();
        write(dir.path(), "secret.key", b"x");
        if !crate::test_support::symlink(&patterns, &dir.path().join(".gitignore")) {
            return;
        }
        let saved = save_now(&mut repo).unwrap();
        let warnings = warnings_of(&saved).to_vec();
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert_eq!(warnings[0].kind, "gitignore_skipped");
        assert!(warnings[0].message.contains(".gitignore"), "{warnings:?}");
        // Its rules don't apply, and the link itself is saved as a link.
        assert_eq!(saved_paths(&repo, saved), [".gitignore", "secret.key"]);
    }

    #[test]
    fn a_gitignore_that_is_a_folder_is_skipped_with_a_warning() {
        let (dir, mut repo) = project();
        write(dir.path(), ".gitignore/x", b"x");
        let saved = save_now(&mut repo).unwrap();
        let warnings = warnings_of(&saved);
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert_eq!(warnings[0].kind, "gitignore_skipped");
        assert!(warnings[0].message.contains(".gitignore"), "{warnings:?}");
    }

    #[test]
    fn a_skipped_gitignore_is_warned_about_when_nothing_changed() {
        let (dir, mut repo) = project();
        write(dir.path(), ".gitignore/x", b"x");
        save_now(&mut repo).unwrap();
        let again = save_now(&mut repo).unwrap();
        assert!(
            matches!(again, SaveOutcome::NothingChanged { .. }),
            "{again:?}"
        );
        let warnings = warnings_of(&again);
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert_eq!(warnings[0].kind, "gitignore_skipped");
    }

    #[test]
    fn both_files_count_toward_the_limits() {
        let (dir, mut repo) = project();
        let head = repo.head().unwrap();
        // Each is well within its own limit; together they are past the limit for one folder.
        write(dir.path(), ".gitignore", &comment_of(600 * 1024));
        write(dir.path(), ".boltignore", &comment_of(600 * 1024));
        match save_now(&mut repo) {
            // The .boltignore is read second, so it is the one that goes over.
            Err(SaveError::Ignore { detail }) => {
                assert!(detail.starts_with(".boltignore: "), "{detail}");
                assert!(detail.contains("larger than 1048576 bytes"), "{detail}");
            }
            other => panic!("{:?}", other.map(|_| ())),
        }
        assert_eq!(repo.head().unwrap(), head);
    }

    #[test]
    fn a_saved_file_stays_saved_when_a_rule_matches_it() {
        let (dir, mut repo) = project();
        let root = dir.path();
        write(root, "a.dll", b"1");
        save_now(&mut repo).unwrap();
        write(root, ".gitignore", b"*.dll\n");
        write(root, "a.dll", b"changed");
        write(root, "b.dll", b"2");
        let id = saved_root(save_now(&mut repo).unwrap());
        let tree = saved_tree(repo.store(), id);
        let paths: Vec<&str> = tree.keys().map(String::as_str).collect();
        assert_eq!(paths, [".gitignore", "a.dll"]);
        // Saved again from the folder, not copied from the save before.
        assert_eq!(tree["a.dll"].1, b"changed");
    }

    #[test]
    fn a_saved_file_in_an_ignored_folder_stays_and_new_ones_stay_out() {
        let (dir, mut repo) = project();
        let root = dir.path();
        write(root, "Library/keep.txt", b"x");
        write(root, "Library/deep/keep.txt", b"x");
        save_now(&mut repo).unwrap();
        write(root, ".gitignore", b"Library/\n");
        write(root, "Library/new.txt", b"x");
        write(root, "Library/deep/new.txt", b"x");
        write(root, "Library/newer/x.txt", b"x");
        let saved = save_now(&mut repo).unwrap();
        assert_eq!(
            saved_paths(&repo, saved),
            [
                ".gitignore",
                "Library",
                "Library/deep",
                "Library/deep/keep.txt",
                "Library/keep.txt"
            ]
        );
    }

    #[test]
    fn no_ignore_file_is_read_inside_an_ignored_folder_kept_for_saved_files() {
        let (dir, mut repo) = project();
        let root = dir.path();
        write(root, "Library/keep.txt", b"x");
        save_now(&mut repo).unwrap();
        write(root, ".gitignore", b"Library/\n");
        // As in Git, neither is read: one can't take a name back, and the other would stop the
        // save for its size.
        write(root, "Library/.gitignore", b"!new.txt\n");
        let too_big = comment_of(safe_fs::MAX_IGNORE_FILE_BYTES + 1);
        write(root, "Library/.boltignore", &too_big);
        write(root, "Library/new.txt", b"x");
        let saved = save_now(&mut repo).unwrap();
        assert_eq!(
            saved_paths(&repo, saved),
            [".gitignore", "Library", "Library/keep.txt"]
        );
    }

    #[test]
    fn deleting_a_saved_ignored_file_removes_it() {
        let (dir, mut repo) = project();
        let root = dir.path();
        write(root, "a.dll", b"1");
        save_now(&mut repo).unwrap();
        write(root, ".gitignore", b"*.dll\n");
        save_now(&mut repo).unwrap();
        fs::remove_file(root.join("a.dll")).unwrap();
        let saved = save_now(&mut repo).unwrap();
        assert_eq!(saved_paths(&repo, saved), [".gitignore"]);
        // Made again, it is a new file, which the rule keeps out.
        write(root, "a.dll", b"1");
        assert!(matches!(
            save_now(&mut repo).unwrap(),
            SaveOutcome::NothingChanged { .. }
        ));
    }

    #[test]
    fn an_ignored_name_that_changed_kind_is_not_kept() {
        let (dir, mut repo) = project();
        let root = dir.path();
        write(root, "a.dll", b"x");
        write(root, "b.dll/x", b"x");
        save_now(&mut repo).unwrap();
        // Each name now holds the other kind, as a new file or folder would.
        fs::remove_file(root.join("a.dll")).unwrap();
        fs::remove_dir_all(root.join("b.dll")).unwrap();
        write(root, "a.dll/x", b"x");
        write(root, "b.dll", b"x");
        write(root, ".gitignore", b"*.dll\n");
        let saved = save_now(&mut repo).unwrap();
        assert_eq!(saved_paths(&repo, saved), [".gitignore"]);
    }

    /// Saves the project, which must make a save, and returns that save's top listing.
    fn saved_top(repo: &mut Repository) -> Listing {
        let root = saved_root(save_now(repo).unwrap());
        repo.store().listing(&root).unwrap()
    }

    /// The commit of a planted project link.
    const COMMIT: &[u8] = &[0x5a; 20];

    #[cfg(windows)]
    #[test]
    fn a_name_this_system_cant_hold_is_kept() {
        let (dir, mut repo) = project();
        let base = plant_save(&mut repo, &[("aux.txt", EntryKind::File, b"x")]);
        write(dir.path(), "new.txt", b"y");
        let saved = saved_top(&mut repo);
        assert!(base.get("aux.txt").is_some());
        assert_eq!(saved.get("aux.txt"), base.get("aux.txt"));
    }

    #[test]
    fn names_that_are_one_file_on_this_drive_are_kept() {
        let (dir, mut repo) = project();
        let (nfc, nfd) = ("caf\u{e9}.txt", "cafe\u{301}.txt");
        // Two names, and what each holds on disk.
        let pairs = [
            (["README", "readme"], ["upper", "lower"]),
            ([nfc, nfd], ["nfc", "nfd"]),
        ];
        let planted: Vec<(&str, EntryKind, &[u8])> = pairs
            .iter()
            .flat_map(|(names, _)| names.map(|n| (n, EntryKind::File, n.as_bytes())))
            .collect();
        let base = plant_save(&mut repo, &planted);
        for (names, texts) in &pairs {
            for (name, text) in names.iter().zip(texts) {
                write(dir.path(), name, text.as_bytes());
            }
        }
        write(dir.path(), "new.txt", b"x");
        let drive = this_drive();
        let saved = saved_top(&mut repo);
        // Where the drive takes the two names for one file, both stay as they were; elsewhere
        // each is saved from its own file.
        for ((names, texts), one_file) in pairs
            .iter()
            .zip([drive.ignores_case, drive.composes_unicode])
        {
            for (name, text) in names.iter().zip(texts) {
                if one_file {
                    assert_eq!(saved.get(name), base.get(name), "{name}");
                } else {
                    let id = saved.get(name).map(|e| e.id);
                    assert_eq!(id, Some(hash_content(text.as_bytes())), "{name}");
                }
            }
        }
    }

    #[test]
    fn a_link_git_wrote_as_a_file_stays_a_link() {
        let (dir, mut repo) = project();
        let base = plant_save(&mut repo, &[("l", EntryKind::Symlink, b"target")]);
        write(dir.path(), "l", b"target");
        write(dir.path(), "new.txt", b"x");
        let saved = saved_top(&mut repo);
        assert_eq!(saved.get("l").map(|e| e.kind), Some(EntryKind::Symlink));
        assert_eq!(saved.get("l"), base.get("l"));
    }

    #[test]
    fn a_file_that_isnt_the_links_target_replaces_the_link() {
        // Only the exact bytes count: a newline more is another file.
        for bytes in [b"other".as_slice(), b"target\n"] {
            let (dir, mut repo) = project();
            plant_save(&mut repo, &[("l", EntryKind::Symlink, b"target")]);
            write(dir.path(), "l", bytes);
            let saved = saved_top(&mut repo);
            let l = saved.get("l").map(|e| (e.kind, e.id));
            assert_eq!(l, Some((EntryKind::File, hash_content(bytes))));
        }
    }

    #[test]
    fn a_project_link_is_kept_and_its_folder_never_read() {
        let (dir, mut repo) = project();
        let base = plant_save(&mut repo, &[("sub", EntryKind::ProjectLink, COMMIT)]);
        write(dir.path(), "sub/.git", b"gitdir: /etc");
        write(dir.path(), "sub/a.txt", b"x");
        // Read, it would stop the save for its size.
        let too_big = comment_of(safe_fs::MAX_IGNORE_FILE_BYTES + 1);
        write(dir.path(), "sub/.boltignore", &too_big);
        write(dir.path(), "new.txt", b"x");
        let saved = saved_top(&mut repo);
        let names: Vec<&str> = saved.entries().iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["new.txt", "sub"]);
        assert_eq!(saved.get("sub"), base.get("sub"));
    }

    #[test]
    fn a_project_link_is_kept_whatever_the_ignore_rules_say() {
        let (dir, mut repo) = project();
        let root = dir.path();
        let base = plant_save(
            &mut repo,
            &[
                ("sub", EntryKind::ProjectLink, COMMIT),
                ("Library/keep.txt", EntryKind::File, b"x"),
                ("Library/sub", EntryKind::ProjectLink, COMMIT),
            ],
        );
        // A rule matches both links, and `Library` is entered only for what the last save holds.
        write(root, ".gitignore", b"sub\nLibrary/\n");
        write(root, "Library/keep.txt", b"x");
        write(root, "Library/sub/a.txt", b"x");
        write(root, "sub/a.txt", b"x");
        let saved = saved_top(&mut repo);
        let names: Vec<&str> = saved.entries().iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, [".gitignore", "Library", "sub"]);
        assert_eq!(saved.get("sub"), base.get("sub"));
        assert_eq!(saved.get("Library"), base.get("Library"));
    }

    #[test]
    fn a_new_name_that_clashes_with_a_kept_one_is_refused() {
        // A new name is judged against every name of its folder, the kept ones too.
        let (dir, mut repo) = project();
        plant_save(&mut repo, &[("sub", EntryKind::ProjectLink, COMMIT)]);
        let head = repo.head().unwrap();
        write(dir.path(), "SUB", b"x");
        let Err(SaveError::NotPortable { problems }) = save_now(&mut repo) else {
            panic!("the clashing name was accepted");
        };
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].1.to_string().contains("SUB"), "{problems:?}");
        assert_eq!(repo.head().unwrap(), head);
    }

    #[test]
    fn kept_entries_are_counted_in_one_warning() {
        let (dir, mut repo) = project();
        plant_save(
            &mut repo,
            &[
                ("a", EntryKind::ProjectLink, COMMIT),
                ("b", EntryKind::ProjectLink, COMMIT),
                ("l", EntryKind::Symlink, b"target"),
            ],
        );
        write(dir.path(), "l", b"target");
        write(dir.path(), "new.txt", b"x");
        let saved = save_now(&mut repo).unwrap();
        let warnings = warnings_of(&saved).to_vec();
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert_eq!(warnings[0].kind, "kept_as_they_were");
        assert!(
            warnings[0]
                .message
                .starts_with("Kept 3 entries as they were"),
            "{warnings:?}"
        );
        // Said again when nothing changed, since they are still kept.
        let again = save_now(&mut repo).unwrap();
        assert!(
            matches!(again, SaveOutcome::NothingChanged { .. }),
            "{again:?}"
        );
        assert_eq!(warnings_of(&again), warnings);
    }

    /// The entry at `path` in the save whose top listing is `top`.
    fn entry_at(repo: &Repository, top: &Listing, path: &str) -> Option<Entry> {
        let (up, name) = path.rsplit_once('/').unwrap_or(("", path));
        let mut listing = top.clone();
        for part in up.split('/').filter(|p| !p.is_empty()) {
            listing = repo.store().listing(&listing.get(part)?.id).unwrap();
        }
        listing.get(name).cloned()
    }

    #[test]
    fn a_project_link_stays_when_the_folders_above_it_are_gone() {
        let (dir, mut repo) = project();
        let base = plant_save(
            &mut repo,
            &[
                ("gone/deep/sub", EntryKind::ProjectLink, COMMIT),
                ("gone/deep/a.txt", EntryKind::File, b"x"),
                ("vendor/libs/sub", EntryKind::ProjectLink, COMMIT),
                ("vendor/libs/a.txt", EntryKind::File, b"x"),
                ("vendor/old.txt", EntryKind::File, b"x"),
            ],
        );
        // `gone` and `vendor/libs` were deleted, and `vendor` holds something new.
        write(dir.path(), "vendor/new.txt", b"x");
        let saved = saved_top(&mut repo);
        let paths: Vec<String> = saved_tree(repo.store(), saved.id()).into_keys().collect();
        // The folders leading to a link come back holding only the link.
        assert_eq!(
            paths,
            [
                "gone",
                "gone/deep",
                "gone/deep/sub",
                "vendor",
                "vendor/libs",
                "vendor/libs/sub",
                "vendor/new.txt"
            ]
        );
        for link in ["gone/deep/sub", "vendor/libs/sub"] {
            let was = entry_at(&repo, &base, link);
            assert_eq!(entry_at(&repo, &saved, link), was, "{link}");
        }
    }

    #[test]
    fn names_this_computer_cant_hold_stay_when_their_folder_is_gone() {
        let (dir, mut repo) = project();
        let base = plant_save(
            &mut repo,
            &[
                ("docs/README", EntryKind::File, b"upper"),
                ("docs/readme", EntryKind::File, b"lower"),
                ("docs/aux.txt", EntryKind::File, b"aux"),
                ("docs/other.txt", EntryKind::File, b"other"),
            ],
        );
        write(dir.path(), "new.txt", b"x");
        let saved = saved_top(&mut repo);
        // Kept: the two names where the drive takes them for one file, and `aux.txt` where this
        // system can't hold it. Without any, `docs` is gone.
        let mut kept = Vec::new();
        if this_drive().ignores_case {
            kept.extend(["docs/README", "docs/readme"]);
        }
        if cfg!(windows) {
            kept.push("docs/aux.txt");
        }
        let mut want = kept.clone();
        if !kept.is_empty() {
            want.push("docs");
        }
        want.push("new.txt");
        want.sort_unstable();
        let paths: Vec<String> = saved_tree(repo.store(), saved.id()).into_keys().collect();
        assert_eq!(paths, want);
        for path in kept {
            let was = entry_at(&repo, &base, path);
            assert_eq!(entry_at(&repo, &saved, path), was, "{path}");
        }
    }

    #[test]
    fn the_search_of_a_gone_folder_stops_at_the_limits_and_the_save_goes_on() {
        let (dir, mut repo) = project();
        plant_save(
            &mut repo,
            &[("a/b/c/d/sub", EntryKind::ProjectLink, COMMIT)],
        );
        // None of it is on disk. `a/b/c/d` is four levels down, and the five listings from the
        // top to it hold one entry each.
        let found_within = |levels, entries| {
            let base = Base {
                limits: SaveLimits::new(levels, entries),
                ..newest(&repo)
            };
            let scan = walk(dir.path(), base, SaveLimits::default(), this_drive());
            scan.unwrap().folders
        };
        assert!(found_within(4, 5).contains_key("a/b/c/d"));
        // Past either limit the base save is invalid, so nothing there can be kept.
        assert!(!found_within(3, 5).contains_key("a"));
        assert!(!found_within(4, 4).contains_key("a"));
    }

    #[test]
    fn something_in_the_way_of_a_kept_entry_stops_the_save() {
        let (dir, mut repo) = project();
        plant_save(&mut repo, &[("libs/sub", EntryKind::ProjectLink, COMMIT)]);
        let head = repo.head().unwrap();
        // The folder holding the link is a file now, and a save can't hold both.
        write(dir.path(), "libs", b"x");
        let err = save_now(&mut repo).unwrap_err();
        let SaveError::InTheWay { path, kept } = &err else {
            panic!("{err:?}");
        };
        assert_eq!((path.as_str(), kept.as_str()), ("libs", "libs/sub"));
        let hint = err.hint().unwrap();
        assert!(hint.contains("Rename or move libs"), "{hint}");
        assert_eq!(repo.head().unwrap(), head);
    }

    #[test]
    fn a_kept_folder_counts_with_everything_it_holds() {
        let (dir, mut repo) = project();
        plant_save(
            &mut repo,
            &[
                ("Docs/a", EntryKind::File, b"a"),
                ("Docs/sub/b", EntryKind::File, b"b"),
                ("docs/c", EntryKind::File, b"c"),
            ],
        );
        // Where the drive ignores letter case both folders are kept, whatever this drive does:
        // two entries at the top, and the four they hold.
        let drive = Drive {
            ignores_case: true,
            composes_unicode: false,
        };
        let walk_under = |most| {
            let limits = SaveLimits::new(MAX_SAVE_DEPTH, most);
            walk(dir.path(), newest(&repo), limits, drive)
        };
        assert!(walk_under(6).is_ok());
        let Err(SaveError::TooLarge { detail }) = walk_under(5) else {
            panic!("6 entries were accepted under a limit of 5");
        };
        assert!(detail.contains("more than 5 "), "{detail}");
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
    fn the_boltignore_hint_does_not_blame_a_pattern() {
        // A link, a pipe, a huge file or one Bolt can't read has no pattern to fix.
        let hint = SaveError::Ignore { detail: "x".into() }.hint().unwrap();
        assert!(!hint.contains("pattern"), "{hint}");
    }

    #[test]
    fn save_stops_when_a_file_changes_during_the_save() {
        let (dir, repo) = project();
        let path = dir.path().join("growing.log");
        fs::write(&path, b"line 1\n").unwrap();
        let mut packs = repo.new_packs().unwrap();
        let got = store_file(&mut packs, repo.store(), &path, &mut || {}).unwrap();
        assert_eq!(got, (hash_content(b"line 1\n"), 7));
        let mut append = || {
            use std::io::Write;
            let mut f = fs::OpenOptions::new().append(true).open(&path).unwrap();
            f.write_all(b"line 2\n").unwrap();
        };
        assert!(matches!(
            store_file(&mut packs, repo.store(), &path, &mut append),
            Err(SaveError::ChangedDuringSave { .. })
        ));
    }

    /// Scans a project holding `victim.txt`, lets `make_link` put a link in its place (it says
    /// whether it could make one), then reads the files as `save` does after its scan.
    fn assert_swap_for_a_link_is_refused(make_link: impl FnOnce(&Path) -> bool) {
        let (dir, repo) = project();
        let victim = dir.path().join("victim.txt");
        fs::write(&victim, b"x").unwrap();
        let folders = walk_project(&repo, SaveLimits::default()).unwrap().folders;
        fs::remove_file(&victim).unwrap();
        if !make_link(&victim) {
            return;
        }
        let mut packs = repo.new_packs().unwrap();
        match build(repo.store(), &mut packs, &folders, &HashMap::new()) {
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
        let gone = dir.path().join("gone.txt");
        match store_file(&mut packs, repo.store(), &gone, &mut || {}) {
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
        std::thread::spawn(move || {
            let mut packs = repo.new_packs().unwrap();
            let stored = store_file(&mut packs, repo.store(), &pipe, &mut || {});
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

    /// Nested folders for a test, removed bottom-up on drop (even if the test fails), because
    /// `remove_dir_all` recurses once per level.
    struct DeepFolders {
        top: PathBuf,
        bottom: PathBuf,
    }

    impl DeepFolders {
        /// `levels` nested folders named `d` under `top`; `None` (with a printed reason) if the
        /// system can't hold that path: 4,097 levels are 8 KB, over the limit on Linux (4,096) and
        /// macOS (1,024). Windows takes it, so there a failure fails the test.
        fn make(top: &Path, levels: usize) -> Option<DeepFolders> {
            let mut deep = DeepFolders {
                top: top.to_path_buf(),
                bottom: top.to_path_buf(),
            };
            for _ in 0..levels {
                deep.bottom.push("d");
                if let Err(e) = fs::create_dir(&deep.bottom) {
                    deep.bottom.pop();
                    if cfg!(windows) {
                        panic!("this system can't make {levels} nested folders: {e}");
                    }
                    eprintln!("skipped: this system can't make {levels} nested folders: {e}");
                    return None;
                }
            }
            Some(deep)
        }
    }

    impl Drop for DeepFolders {
        fn drop(&mut self) {
            while self.bottom != self.top && fs::remove_dir(&self.bottom).is_ok() {
                self.bottom.pop();
            }
        }
    }

    #[test]
    fn folder_deeper_than_the_limit_is_refused() {
        let (dir, repo) = project();
        // a, a/b and a/b/c are folders 1, 2 and 3 levels down; f is a file, not a level.
        write(dir.path(), "a/b/c/f", b"x");
        assert!(walk_project(&repo, SaveLimits::new(3, MAX_SAVE_ENTRIES)).is_ok());
        let Err(err) = walk_project(&repo, SaveLimits::new(2, MAX_SAVE_ENTRIES)) else {
            panic!("a folder 3 levels down was accepted under a limit of 2");
        };
        match &err {
            SaveError::TooLarge { detail } => {
                assert!(detail.contains("more than 2 levels"), "{detail}");
            }
            other => panic!("{other:?}"),
        }
        assert!(err.hint().unwrap().contains(".boltignore"));
    }

    #[test]
    fn more_entries_than_the_limit_are_refused() {
        let (dir, repo) = project();
        // a, b, c and d are 4 entries of the top folder, and d/e is a fifth.
        for name in ["a", "b", "c", "d/e"] {
            write(dir.path(), name, b"x");
        }
        assert!(walk_project(&repo, SaveLimits::new(MAX_SAVE_DEPTH, 5)).is_ok());
        let Err(SaveError::TooLarge { detail }) =
            walk_project(&repo, SaveLimits::new(MAX_SAVE_DEPTH, 4))
        else {
            panic!("5 entries were accepted under a limit of 4");
        };
        assert!(detail.contains("more than 4 "), "{detail}");
    }

    #[test]
    fn reading_the_last_save_counts_every_path_against_the_entry_limit() {
        let (dir, mut repo) = project();
        // Three folders with the same two files: the save holds one listing for all of them.
        for folder in ["a", "b", "c"] {
            write(dir.path(), &format!("{folder}/x"), b"x");
            write(dir.path(), &format!("{folder}/y"), b"y");
        }
        save_now(&mut repo).unwrap();
        // The walk reads the listing of every folder: 3 entries (the top listing) and, for each
        // of the 3 folders, the 2 entries of the shared listing.
        const PATHS: u64 = 3 + 3 * 2;
        let walk_reading = |paths| {
            let base = Base {
                limits: SaveLimits::new(MAX_SAVE_DEPTH, paths),
                ..newest(&repo)
            };
            walk(dir.path(), base, SaveLimits::default(), this_drive())
        };
        assert!(walk_reading(PATHS).is_ok());
        let Err(err) = walk_reading(PATHS - 1) else {
            panic!("{} paths were read under a limit of {}", PATHS, PATHS - 1);
        };
        assert!(
            matches!(err, SaveError::Repo(RepoError::Damaged { .. })),
            "{err:?}"
        );
    }

    /// Runs `f` on a thread with a stack of `bytes`. Windows' main thread, where `bolt` runs a
    /// save, has 1 MiB.
    fn on_a_stack_of<T: Send>(bytes: usize, f: impl FnOnce() -> T + Send) -> T {
        std::thread::scope(|scope| {
            std::thread::Builder::new()
                .stack_size(bytes)
                .spawn_scoped(scope, f)
                .unwrap()
                .join()
                .unwrap()
        })
    }

    #[test]
    fn refusing_a_deep_tree_does_not_depend_on_the_callers_stack() {
        let (dir, repo) = project();
        // Dropping `ignore`'s linked matchers recurses once per level (about 400 bytes in a debug
        // build): 300 levels need about 120 KB, more than the 64 KiB stack here, and are few enough
        // to make everywhere.
        let Some(_deep) = DeepFolders::make(dir.path(), 300) else {
            return;
        };
        let drive = this_drive();
        let base = newest(&repo);
        let refused = on_a_stack_of(64 * 1024, || {
            let limits = SaveLimits::new(299, MAX_SAVE_ENTRIES);
            matches!(
                walk(dir.path(), base, limits, drive),
                Err(SaveError::TooLarge { .. })
            )
        });
        assert!(refused);
    }

    // The real limit of 4,096 levels takes about a minute per test on Windows, so these run on
    // request (and in CI on Windows only), on a 1 MiB stack like `bolt`'s main thread. Run with:
    // `cargo test -p bolt-vcs-core --all-features --lib -- --ignored real_folders`

    #[test]
    #[ignore = "slow on Windows (about a minute): run with --ignored"]
    fn real_folders_one_level_past_the_limit_are_refused() {
        let (dir, mut repo) = project();
        let head = repo.head().unwrap();
        let Some(_deep) = DeepFolders::make(dir.path(), MAX_SAVE_DEPTH + 1) else {
            return;
        };
        match on_a_stack_of(1 << 20, || save_now(&mut repo)) {
            Err(SaveError::TooLarge { detail }) => assert!(detail.contains("4096"), "{detail}"),
            other => panic!("{:?}", other.map(|_| ())),
        }
        assert_eq!(repo.head().unwrap(), head);
    }

    #[test]
    #[ignore = "slow on Windows (about a minute): run with --ignored"]
    fn real_folders_at_the_limit_are_saved() {
        let (dir, mut repo) = project();
        let Some(_deep) = DeepFolders::make(dir.path(), MAX_SAVE_DEPTH) else {
            return;
        };
        let saved = on_a_stack_of(1 << 20, || save_now(&mut repo));
        assert!(matches!(saved, Ok(SaveOutcome::Saved { .. })), "{saved:?}");
    }
}
