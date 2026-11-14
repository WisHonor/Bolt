//! `opalro save`: walks the project folder, stores its files, and commits the save.

use std::cmp::Reverse;
use std::collections::{BTreeMap, HashMap};
use std::ffi::OsStr;
use std::fmt::{self, Write as _};
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::Arc;

use opal_platform::dir::{Dir, EntryMeta, EntryType};

use crate::changeids::ChangeIds;
use crate::diary::{Action, DiaryEntry, Position, State};
use crate::diff::count_changes;
use crate::drive::{self, Drive};
use crate::filecache::{self, FileCache, FileState};
use crate::fsio;
use crate::history;
use crate::id::{ChangeId, ContentHasher, Id, hash_content};
use crate::ingest::{self, IngestError, Ready};
use crate::lines::{NameError, check_new_name};
use crate::link;
use crate::listing::{Entry, EntryKind, Listing, SaveLimits};
use crate::names::{Given, PortabilityProblem, folder_problems};
use crate::otherlines::OtherLines;
use crate::pack::{ObjectKind, PackError, PackSet};
use crate::repo::{RepoError, Repository};
use crate::rules::Rules;
use crate::save::{EXTRA_GIT_COMMIT, EXTRA_LINE, GIT_FIELDS, Person, Save};
use crate::scope::Scope;
use crate::store::{Store, StoreError};
use crate::text::{self, escape, thousands};
use crate::warning::{self, Warning};
use crate::workers::{self, WINDOW, with_workers};
use crate::writeout;

mod endings;
mod only;
mod walk;

use walk::{Base, Handles, Note, Noted, Scan, walk};

pub struct SaveRequest<'a> {
    /// `None` keeps the message of the save an amend replaces; any other save needs one.
    pub message: Option<&'a str>,
    pub who: &'a Person,
    /// The command as the diary records it; already redacted.
    pub command: &'a str,
    /// Save files Git rewrote with CRLF as they are, without the stop after an import.
    pub accept_crlf: bool,
    /// The user's own Git ignore list, from `config::git_excludes_file`, whose lines come first
    /// (decision 047); `None` reads none.
    pub git_excludes: Option<&'a Path>,
    /// The new line a save made at a save goes on; refused on a line.
    pub new_line: Option<&'a str>,
    /// Save only the changes at these paths and patterns (decision 054); `None` saves all.
    pub only: Option<&'a [Given]>,
    /// Replace the newest save of the current line, keeping its change ID, instead of saving on
    /// top of it.
    pub amend: bool,
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
        /// The save an amend replaced, which stays stored.
        replaced: Option<Id>,
    },
    /// The folder matches the line's current save, which is `current`; for an amend, the message
    /// matches too.
    NothingChanged {
        current: Id,
        /// What the walk found wrong, such as a `.gitignore` it skipped.
        warnings: Vec<Warning>,
    },
}

/// What a save adds that the save it builds on doesn't hold, when that is unusually much.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LargeSave {
    pub new_files: u64,
    pub new_bytes: u64,
    /// The top-level folders holding the most new bytes, biggest first, at most three; `""` is
    /// the top folder itself, for the files directly in it.
    pub biggest_folders: Vec<(String, u64)>,
}

impl fmt::Display for LargeSave {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let files = if self.new_files == 1 { "file" } else { "files" };
        let folders: Vec<String> = self
            .biggest_folders
            .iter()
            .map(|(folder, size)| {
                let name = if folder.is_empty() {
                    "the top folder".to_owned()
                } else {
                    format!("{}/", escape(folder, false))
                };
                format!("{name} ({})", text::bytes(*size))
            })
            .collect();
        write!(
            f,
            "This save adds {} in {} new {files}, most in: {}.",
            text::bytes(self.new_bytes),
            thousands(self.new_files),
            folders.join(", ")
        )
    }
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
    #[error("{} changed while Opalro was reading it.", escape(path, false))]
    ChangedDuringSave { path: String },
    #[error(
        "{} is a special file Opalro can't read yet, such as a cloud placeholder.",
        escape(path, false)
    )]
    SpecialFile { path: String },
    // `reason` is Opalro's own wording.
    #[error("Opalro can't save {}: {reason}.", escape(path, false))]
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
        "Opalro can't save {}: {}.",
        escape(path, false),
        kept_in_the_way(kept)
    )]
    InTheWay { path: String, kept: String },
    #[error("{}", line_endings_changed(*count, first))]
    LineEndingsChanged {
        /// How many files, of which `first` names the first few by project path.
        count: u64,
        first: Vec<String>,
    },
    #[error("{}", lfs_pointers_in_folder(paths, *count))]
    LfsPointersInFolder {
        /// The first few by project path, of `count` files.
        paths: Vec<String>,
        count: u64,
    },
    #[error("A save needs a message.")]
    NeedsMessage,
    #[error("You're at a save, not on a line, so this save needs a new line to go on.")]
    NeedsLineName,
    /// `--line` named a line that can't be made; never [`NameError::Repo`].
    #[error(transparent)]
    Name(NameError),
    #[error(
        "{} matches nothing in your files or in the last save.",
        escape(given, false)
    )]
    NoMatch { given: String },
    #[error(
        "{} is new, and an ignore rule keeps it out of saves.",
        escape(path, false)
    )]
    LeftOut { path: String },
    #[error(
        "Opalro can't save part of {}: it is {what}, not a folder.",
        escape(path, false)
    )]
    NotAFolder { path: String, what: &'static str },
    #[error(
        "Opalro can't save part of {}: every save keeps it as it was, because this computer can't hold it as it is.",
        escape(path, false)
    )]
    KeptWhole { path: String },
    #[error(
        "Opalro never saves {}: Git treats its name as `.git`, or Opalro as `.opalro`.",
        escape(path, false)
    )]
    NeverSaved { path: String },
    #[error(
        "{} holds a repository or project of its own, so saves leave out {}.",
        escape(at, false),
        escape(path, false)
    )]
    NestedProject { path: String, at: String },
    /// The save adds unusually much, and no one said to go on.
    #[error("{0}")]
    LargeSave(LargeSave),
    #[error("You're at a save, not on a line, so there is no newest save of a line to amend.")]
    AmendAtSave,
    #[error("An amend replaces the newest save of your line, so it can't start a new line.")]
    AmendNewLine,
    #[error("{}", shown_too(by))]
    AmendShared {
        /// Who else shows the save: `line x`, `server line y@origin` or `tag v1`.
        by: Vec<String>,
    },
}

// Through `RepoError`, so a damaged pack is reported as damage.
impl From<StoreError> for SaveError {
    fn from(e: StoreError) -> Self {
        SaveError::Repo(e.into())
    }
}

impl From<NameError> for SaveError {
    fn from(e: NameError) -> Self {
        match e {
            NameError::Repo(e) => SaveError::Repo(e),
            e => SaveError::Name(e),
        }
    }
}

impl From<PackError> for SaveError {
    fn from(e: PackError) -> Self {
        SaveError::Repo(e.into())
    }
}

/// Why a path holds something a save can't take: the last save keeps a folder there, holding
/// `kept`, as it was.
pub(crate) fn kept_in_the_way(kept: &str) -> String {
    format!(
        "the last save has a folder there holding {}, which every save keeps as it was because \
         this computer can't hold it as it is, and a save can't hold both under one name",
        escape(kept, false)
    )
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

/// How many paths, lines, server lines, tags or diary entries a message names; it counts the rest.
pub(crate) const NAMED: usize = 5;

/// The paths `first` of `count` (or other names, such as diary entries' IDs), escaped, one to a
/// line, and how many more there are.
pub(crate) fn listed<S: AsRef<str>>(first: impl IntoIterator<Item = S>, count: u64) -> String {
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

/// What Git LFS didn't do, then the files named, one to a line, and how many more there are.
fn lfs_pointers_in_folder(paths: &[String], count: u64) -> String {
    let text = if count == 1 {
        "1 file is a Git LFS pointer, not the file it stands for: Git LFS wasn't installed when \
         the folder was checked out, or was told to skip it."
            .to_owned()
    } else {
        format!(
            "{count} files are Git LFS pointers, not the files they stand for: Git LFS wasn't \
             installed when the folder was checked out, or was told to skip them."
        )
    };
    text + &listed(paths, count)
}

/// Names the first few of those that show the save an amend would replace, and counts the rest.
fn shown_too(by: &[String]) -> String {
    let mut named: Vec<String> = by.iter().take(NAMED).map(|b| escape(b, false)).collect();
    let more = by.len().saturating_sub(NAMED);
    if more > 0 {
        named.push(format!("{more} more"));
    }
    let who = match named.split_last() {
        Some((last, rest)) if !rest.is_empty() => format!("{} and {last}", rest.join(", ")),
        _ => named.concat(),
    };
    let show = if by.len() == 1 { "shows" } else { "show" };
    format!(
        "Opalro can't amend this save: {who} {show} it too, and two saves you can see would then \
         share one change ID."
    )
}

impl SaveError {
    /// What the user can do next after a save, if there is something to say.
    #[must_use]
    pub fn hint(&self) -> Option<String> {
        self.next_step(true)
    }

    /// The same, for a command that only reads the folder, such as `status`: nothing was going
    /// to be saved.
    #[must_use]
    pub fn hint_when_reading(&self) -> Option<String> {
        self.next_step(false)
    }

    /// This error, met reading the folders of the save `save`: a spelling of `.opalro` is told as
    /// that save's, `current` when the project is on it.
    #[must_use]
    pub(crate) fn in_save(self, save: Id, current: bool) -> SaveError {
        match self {
            SaveError::Repo(e) => SaveError::Repo(e.in_save(save, current)),
            other => other,
        }
    }

    fn next_step(&self, saving: bool) -> Option<String> {
        let step = match self {
            SaveError::Repo(e) => return e.hint(),
            SaveError::InTheWay { path, .. } => {
                let path = escape(path, false);
                if saving {
                    format!("Rename or move {path}, then save again.")
                } else {
                    format!("Rename or move {path}.")
                }
            }
            // `git checkout` alone skips files whose stat data the index matches.
            SaveError::LineEndingsChanged { .. } => {
                return Some(
                    "To keep history as Git stored it, commit or stash your changes in Git, then \
                     run `git config core.autocrlf false`, `git rm --cached -r -q :/` and `git \
                     reset --hard`, and save again. Files `.gitattributes` marks `eol=crlf` stay \
                     CRLF after those steps, and --accept-crlf then saves just those. To save the \
                     files with CRLF anyway, run the same command again with --accept-crlf."
                        .to_owned(),
                );
            }
            SaveError::LfsPointersInFolder { .. } => {
                "Run `git lfs checkout` to have Git LFS write the files, then save again. If Git \
                 LFS can't, `opalro discard` with those paths puts back the files the import \
                 brought in."
                    .into()
            }
            SaveError::NeedsMessage => {
                "Say what you changed: opalro save \"describe your change\"".into()
            }
            SaveError::NeedsLineName => "Name it: opalro save --line <name> \"message\"".into(),
            SaveError::Name(e) => e.hint()?,
            SaveError::NoMatch { .. } => {
                "Name files, folders or patterns from the folder you are in.".into()
            }
            SaveError::LeftOut { .. } => "To save it, change the rule that ignores it.".into(),
            SaveError::NotAFolder { path, .. } => {
                format!("Name {} itself to save it as it is.", escape(path, false))
            }
            SaveError::KeptWhole { .. } => {
                "Leave it out of what you name: it stays as the last save has it.".into()
            }
            SaveError::NeverSaved { .. } => "Rename it if it should be saved.".into(),
            SaveError::NestedProject { .. } => {
                "Save it in that repository or project instead.".into()
            }
            SaveError::Unreadable { .. } => {
                "Let Opalro read it, or list it in .opalroignore.".into()
            }
            SaveError::ChangedDuringSave { .. } => {
                "Try again when the file stops changing, or list it in .opalroignore.".into()
            }
            SaveError::SpecialFile { .. } => "List it in .opalroignore for now.".into(),
            SaveError::InvalidName { .. } => "Rename it, or list it in .opalroignore.".into(),
            SaveError::NotPortable { .. } => "Rename them, or list them in .opalroignore.".into(),
            // No line is ever refused, so the file itself is at fault: too big, unreadable, or a
            // `.opalroignore` that isn't a regular file.
            SaveError::Ignore { .. } => "Fix that ignore file and try again.".into(),
            SaveError::TooLarge { .. } => "List the deep or huge folder in .opalroignore.".into(),
            SaveError::LargeSave(_) => "To save them, run the save again with --yes; to leave \
                                        some out, list them in .opalroignore."
                .into(),
            SaveError::AmendAtSave => {
                "To amend a line's newest save, go to the line first: opalro goto <line>".into()
            }
            SaveError::AmendNewLine => {
                "To start a line with these changes: opalro save --line <name> \"message\"".into()
            }
            SaveError::AmendShared { .. } => {
                "Save your changes as a new save instead: opalro save \"describe your change\""
                    .into()
            }
        };
        Some(if saving {
            format!("Nothing was saved. {step}")
        } else {
            step
        })
    }
}

/// Saves the project folder on the current line, or in place of its newest save for an amend.
/// Nothing is written unless every file was read and every name passed its checks. A save that
/// adds unusually much goes on only if `confirm_large` says so; it is asked before anything new
/// is read or stored.
pub fn save_folder(
    repo: &mut Repository,
    req: &SaveRequest,
    on_wait: &mut dyn FnMut(&str),
    confirm_large: &mut dyn FnMut(&LargeSave) -> bool,
) -> Result<SaveOutcome, SaveError> {
    if req.message.is_none() && !req.amend {
        return Err(SaveError::NeedsMessage);
    }
    if req.amend && req.new_line.is_some() {
        return Err(SaveError::AmendNewLine);
    }
    let lock = repo.lock_for_change(req.command, on_wait)?;
    // A stopped switch is finished first, so the save is of the folder it left. What came of it
    // is kept with the store's warnings, so it is told even if the save then fails.
    if let Some(done) = writeout::finish_interrupted(repo, &lock)? {
        for w in done.warnings {
            repo.store().note(w);
        }
    }
    let (head, entry) = repo.current()?;
    if req.amend && matches!(entry.state.position, Position::AtSave(_)) {
        return Err(SaveError::AmendAtSave);
    }
    let (line, parent_id) = save_goes_on(repo, &entry.state, req.new_line)?;
    let parent = repo.store().save(&parent_id, repo.signatures_allowed())?;
    // An amend needs the change-ID cache before the walk; a save, only once it has its save.
    let mut early_cache = None;
    if req.amend
        && let Some(line) = &line
    {
        let mut cache = ChangeIds::load(repo);
        let by = shown_elsewhere(repo, &mut cache, &entry.state, line, parent_id, &parent);
        // Kept however the amend ends: reading the other lines is what costs.
        cache.store(repo, &entry.state);
        let by = by?;
        if !by.is_empty() {
            return Err(SaveError::AmendShared { by });
        }
        early_cache = Some(cache);
    }
    // Before the walk, as for `read_folder`; without it every file is read.
    let now = filecache::drive_now(repo).ok();
    let drive = drive::probe(repo.tmp_folder()).map_err(RepoError::from)?;
    let base = Base::new(repo.store(), parent.root, SaveLimits::default());
    let walked = || {
        walk(
            repo.root(),
            base,
            SaveLimits::default(),
            drive,
            req.git_excludes,
        )
    };
    let (scan, cache) = match now {
        Some(_) => FileCache::load_during(repo, walked),
        None => (walked(), FileCache::default()),
    };
    let Scan {
        mut handles,
        mut folders,
        mut old,
        old_ids,
        mut rebuilt,
        noted,
        mut warnings,
        ..
    } = scan?;
    fsio::fault_point("save:walked").map_err(RepoError::from)?;
    warnings.extend(repo.rules().warnings());
    let imported = parent.extra.contains_key(EXTRA_GIT_COMMIT);
    // In the whole folder, before a partial save narrows it: a pointer left out would be saved as
    // it is by the next save, which doesn't look. Told after the save's own checks.
    let pointers = if imported {
        endings::pointers(repo.store(), drive, &cache, &mut handles, &folders, &old)
    } else {
        Ok(())
    };
    if let Some(given) = req.only {
        let scope = Scope::new(given, drive);
        let store = repo.store();
        let limits = SaveLimits::default();
        let narrowed = only::narrow(
            &scope,
            given,
            store,
            &mut folders,
            &mut old,
            &mut rebuilt,
            limits,
        )?;
        only::check_matched(&scope, given, &narrowed, &mut handles)?;
    }
    check_names(&folders, &old)?;
    // Before anything is stored, so that a stop leaves nothing behind (decision 039). Pointers
    // first: saving line endings as they are can't help while pointers stand in for files.
    pointers?;
    if imported && !req.accept_crlf {
        endings::check(repo.store(), drive, &mut handles, &folders, &old)?;
    }
    if let Some(large) = large_save(repo.store(), &repo.rules().rules, drive, &folders, &old)? {
        // New files mean something to save, so a missing line name is asked for now: the
        // question comes after it, and what is stored is stored once.
        if line.is_none() {
            return Err(SaveError::NeedsLineName);
        }
        if !confirm_large(&large) {
            return Err(SaveError::LargeSave(large));
        }
    }
    let mut packs = repo.new_packs()?;
    for listing in &rebuilt {
        store_listing(repo.store(), &mut packs, listing)?;
    }
    let mut storing = Storing {
        store: repo.store(),
        packs: &mut packs,
        cache,
    };
    let last = Last {
        listings: &old,
        ids: &old_ids,
    };
    let built = build_reading(
        &mut storing,
        Some(repo.store()),
        &mut handles,
        &folders,
        last,
        &noted,
    )?;
    let seen = storing.cache;
    let root = built.root;
    let kept = built.kept.len() + built.kept_below_gone;
    if kept > 0 {
        warnings.push(kept_warning(kept));
    }
    if !built.changed.is_empty() {
        warnings.push(ignored_changes_saved(built.changed));
    }
    if !built.taken_back.is_empty() {
        warnings.push(gitignored_files_saved(built.taken_back));
    }
    let message = req.message.unwrap_or(&parent.message);
    if root == parent.root && (!req.amend || message == parent.message) {
        // Anything stored here replaces a damaged copy the last save needs, so it is kept.
        if !packs.is_empty() {
            packs.finish()?;
        }
        keep_file_cache(repo, seen, now, &mut warnings);
        return Ok(SaveOutcome::NothingChanged {
            current: parent_id,
            warnings,
        });
    }
    // Asked only now, so that a save with nothing to save never asks for a name.
    let Some(line) = &line else {
        return Err(SaveError::NeedsLineName);
    };
    let save = if req.amend {
        let mut extra = parent.extra.clone();
        for field in GIT_FIELDS {
            extra.remove(field);
        }
        Save {
            root,
            parents: parent.parents.clone(),
            author: parent.author.clone(),
            saver: req.who.clone(),
            message: message.to_owned(),
            change_id: parent.change_id,
            move_notes: parent.move_notes.clone(),
            signature: vec![],
            extra,
        }
    } else {
        Save {
            root,
            parents: vec![parent_id],
            author: req.who.clone(),
            saver: req.who.clone(),
            message: message.to_owned(),
            change_id: ChangeId::random()
                .map_err(io::Error::other)
                .map_err(RepoError::from)?,
            move_notes: vec![],
            signature: vec![],
            extra: BTreeMap::from([(EXTRA_LINE.to_owned(), line.as_bytes().to_vec())]),
        }
    };
    let encoded = save.encode();
    // Refuse a save that would not read back (an over-long message, say) before storing it.
    Save::decode(&encoded, false).map_err(StoreError::from)?;
    let save_id = packs.add_record(ObjectKind::Save, &encoded)?;
    // Worked out before the commit, so that nothing after it can fail the save. The line's
    // saves come from the cache, which then learns this one; an amend takes the place of the
    // save it amends.
    let mut cache = early_cache.unwrap_or_else(|| ChangeIds::load(repo));
    let shortened = cache.chain(repo, Some(line), parent_id).map(|saves| {
        let behind = if req.amend {
            saves.get(1..).unwrap_or_default()
        } else {
            saves
        };
        history::short_in(behind, &save_id, &save.change_id)
    });
    let short_change_id = shortened.unwrap_or_else(|why| {
        warnings.push(Warning::new(
            warning::CHANGE_ID_NOT_SHORTENED,
            format!(
                "The save was made, but Opalro couldn't read this line's older saves to \
                     shorten its ID. {why}"
            ),
        ));
        save.change_id.to_letters()
    });
    let mut state = entry.state.clone();
    state.lines.insert(line.clone(), save_id);
    state.position = Position::OnLine(line.clone());
    let diary = DiaryEntry {
        parent: Some(head),
        action: Action::Command,
        command: req.command.to_owned(),
        who: req.who.clone(),
        opal_version: env!("CARGO_PKG_VERSION").into(),
        state,
        folder_change: None,
    };
    repo.commit(&lock, packs, &diary)?;
    if req.amend {
        cache.replace(line, parent_id, save_id);
    } else {
        cache.add(line, parent_id, save_id, save.change_id);
    }
    cache.store(repo, &diary.state);
    keep_file_cache(repo, seen, now, &mut warnings);
    // Counted from the save before it, which for an amend is the first parent it took over.
    let before = match (req.amend, save.parents.first()) {
        (false, _) => Ok(Some(parent.root)),
        (true, Some(first)) => repo
            .store()
            .save(first, repo.signatures_allowed())
            .map(|s| Some(s.root)),
        (true, None) => Ok(None),
    };
    // The save is committed now; failing to count its files must not make it look failed.
    let changed_files = match before.and_then(|before| count_changes(repo.store(), before, root)) {
        Ok(count) => Some(count),
        Err(why) => {
            warnings.push(Warning::new(
                warning::CHANGED_FILES_NOT_COUNTED,
                format!(
                    "The save was made, but Opalro couldn't count the files it changed. {}",
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
        replaced: req.amend.then_some(parent_id),
    })
}

/// Who else shows `id`, the newest save of `line`, as `AmendShared` names them: a line that holds
/// it or reaches its change ID along first parents, a server line or a tag that names it.
pub(crate) fn shown_elsewhere(
    repo: &Repository,
    cache: &mut ChangeIds,
    state: &State,
    line: &str,
    id: Id,
    save: &Save,
) -> Result<Vec<String>, SaveError> {
    let mut by: Vec<String> = lines_reaching(repo, cache, state, line, id, &save.change_id)?
        .into_iter()
        .map(|name| format!("line {name}"))
        .collect();
    for (server, lines) in &repo.server_lines(state)?.servers {
        for (name, &tip) in lines {
            if tip == id {
                by.push(format!("server line {name}@{server}"));
            }
        }
    }
    for (name, tag) in &repo.tag_list(state)?.tags {
        if tag.save == id {
            by.push(format!("tag {name}"));
        }
    }
    Ok(by)
}

/// The lines of `state` other than `line`, in order of name, that hold `id`, the newest save of
/// `line`, or reach a save with `change_id` along first parents. Only `line` gets a list in
/// `cache`: each other line is walked until it meets that list or a save an earlier walk passed,
/// so lines that share a history cost it once. What the walks read is kept in `cache/other-lines`,
/// and the next amend goes on from where they stopped.
fn lines_reaching<'s>(
    repo: &Repository,
    cache: &mut ChangeIds,
    state: &'s State,
    line: &str,
    id: Id,
    change_id: &ChangeId,
) -> Result<Vec<&'s str>, StoreError> {
    let mut others = state
        .lines
        .iter()
        .filter(|(name, _)| *name != line)
        .peekable();
    if others.peek().is_none() {
        return Ok(vec![]);
    }
    // Where each save of `line` is on its list, and the last place `change_id` is. A list that
    // can't be read just isn't used: the walks read what they need and say what fails.
    let (places, last) = match cache.chain(repo, Some(line), id) {
        Ok(saves) => (
            saves
                .iter()
                .enumerate()
                .map(|(at, (save, _))| (*save, at))
                .collect(),
            saves.iter().rposition(|(_, c)| c == change_id),
        ),
        Err(_) => (HashMap::new(), None),
    };
    // From `start` back along first parents: whether the line reaches the change ID, the saves
    // read, with their change IDs, and the save it stopped at unread, if any.
    let walk = |start: Option<Id>, known: &HashMap<Id, bool>| {
        let mut next = start;
        let mut passed = Vec::new();
        let reaches = loop {
            let Some(at) = next else { break false };
            // From here on this line is `line`'s list, so it reaches the change ID if the list
            // has it here or further back.
            if let Some(&p) = places.get(&at) {
                break last.is_some_and(|last| last >= p);
            }
            if let Some(&answer) = known.get(&at) {
                break answer;
            }
            let save = repo.store().save(&at, repo.signatures_allowed())?;
            passed.push((at, save.change_id));
            next = save.parents.first().copied();
            if save.change_id == *change_id {
                break true;
            }
        };
        Ok::<_, StoreError>((reaches, passed, next))
    };
    let mut known: HashMap<Id, bool> = HashMap::new();
    let mut runs: Option<OtherLines> = None;
    let mut found = Vec::new();
    for (name, &tip) in others {
        let listed = cache
            .get(name)
            .filter(|saves| saves.first().is_some_and(|(first, _)| *first == tip));
        let reaches = if tip == id {
            true
        } else if let Some(saves) = listed {
            saves.iter().any(|(_, c)| c == change_id)
        } else {
            let runs = runs.get_or_insert_with(|| OtherLines::load(repo));
            // What an earlier amend read of this line: if it doesn't hold the change ID, the
            // walk goes on from where it stopped.
            let (start, from_run) = match runs.run(&tip) {
                Some(run) if run.saves.iter().any(|(_, c)| c == change_id) => {
                    found.push(name.as_str());
                    continue;
                }
                Some(run) => (run.next, true),
                None => (Some(tip), false),
            };
            let (reaches, passed, next) = match walk(start, &known) {
                // A run that goes on to no save is a wrong cache, not a damaged store: the line
                // is walked again from its newest save, and the run replaced.
                Err(_) if from_run => {
                    runs.forget(&tip);
                    walk(Some(tip), &known)?
                }
                walked => walked?,
            };
            runs.add(tip, &passed, next);
            known.extend(passed.into_iter().map(|(at, _)| (at, reaches)));
            reaches
        };
        if reaches {
            found.push(name.as_str());
        }
    }
    if let Some(runs) = &mut runs {
        runs.store(repo, state);
    }
    Ok(found)
}

/// The base save's file named `name` in a folder it lists as `before`, if it holds a file there.
fn saved_file<'l>(before: Option<&'l Listing>, name: &str) -> Option<&'l Entry> {
    before
        .and_then(|l| l.get(name))
        .filter(|e| matches!(e.kind, EntryKind::File | EntryKind::Executable))
}

/// Hands `each` every file in `folders`, with its folder's path and the base save's file it stands
/// for, if any: one under a name `drive` takes as its own, in a folder of the base save whose name
/// `drive` takes as its folder's. `old` is what the base save holds in each folder of `folders`
/// under its own name. Only listings are read.
fn each_file_with_saved<'f>(
    store: &Store,
    drive: Drive,
    folders: &'f Folders,
    old: &HashMap<String, Option<Listing>>,
    mut each: impl FnMut(&'f str, &'f Child, Option<&Entry>) -> Result<(), SaveError>,
) -> Result<(), SaveError> {
    let folds = drive.ignores_case || drive.composes_unicode;
    // Base folders the walk didn't match because their name is spelled otherwise on disk.
    let mut found: HashMap<&str, Listing> = HashMap::new();
    for (rel, children) in folders {
        let respelled = found.remove(rel.as_str());
        let before = old.get(rel).and_then(Option::as_ref).or(respelled.as_ref());
        // Built at the first name missing from `before`: its entries by name as the drive sees it.
        let mut alike: Option<HashMap<String, &Entry>> = None;
        let mut lookup = |name: &str| -> Option<&Entry> {
            let listing = before?;
            if let Some(e) = listing.get(name) {
                return Some(e);
            }
            if !folds {
                return None;
            }
            let index = alike.get_or_insert_with(|| {
                let entries = listing.entries().iter();
                entries.map(|e| (drive.compared(&e.name), e)).collect()
            });
            index.get(&drive.compared(name)).copied()
        };
        for c in children {
            match c.kind {
                Kind::Folder if old.get(&c.rel).and_then(Option::as_ref).is_none() => {
                    if let Some(e) = lookup(&c.name).filter(|e| e.kind == EntryKind::Folder) {
                        found.insert(&c.rel, store.listing(&e.id)?);
                    }
                }
                Kind::File => {
                    let saved = lookup(&c.name).and_then(|e| saved_file(before, &e.name));
                    each(rel, c, saved)?;
                }
                _ => {}
            }
        }
    }
    Ok(())
}

/// What the save would add, from the walk's sizes, if it is more than a save adds without asking.
/// A file is new where the base save holds no file under a name `drive` takes as its own, so a
/// changed file, or one renamed only in letter case on a drive that ignores case, doesn't count.
/// Only listings are read. The project's rules say how much is more.
fn large_save(
    store: &Store,
    rules: &Rules,
    drive: Drive,
    folders: &Folders,
    old: &HashMap<String, Option<Listing>>,
) -> Result<Option<LargeSave>, SaveError> {
    let (mut new_files, mut new_bytes) = (0u64, 0u64);
    let mut by_top: HashMap<&str, u64> = HashMap::new();
    each_file_with_saved(store, drive, folders, old, |rel, c, saved| {
        if saved.is_none() {
            let size = c.meta.as_ref().map_or(0, |m| m.size);
            new_files = new_files.saturating_add(1);
            new_bytes = new_bytes.saturating_add(size);
            let top = rel.split('/').next().unwrap_or_default();
            let held = by_top.entry(top).or_default();
            *held = held.saturating_add(size);
        }
        Ok(())
    })?;
    if new_bytes <= rules.ask_over_bytes && new_files <= rules.ask_over_files {
        return Ok(None);
    }
    let mut biggest: Vec<(String, u64)> = by_top
        .into_iter()
        .map(|(top, size)| (top.to_owned(), size))
        .collect();
    biggest.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    biggest.truncate(3);
    Ok(Some(LargeSave {
        new_files,
        new_bytes,
        biggest_folders: biggest,
    }))
}

/// The line a save goes on, and the save it builds on: the current line, or the new line
/// `new_line`, which the save's diary entry then makes; the line the project was on stays where
/// it was. `None` at a save without one.
fn save_goes_on(
    repo: &Repository,
    state: &State,
    new_line: Option<&str>,
) -> Result<(Option<String>, Id), SaveError> {
    let parent = match &state.position {
        Position::OnLine(line) => *state.lines.get(line).ok_or_else(|| RepoError::Damaged {
            detail: format!("the current line {line} has no save"),
        })?,
        Position::AtSave(at) => *at,
    };
    let line = match (&state.position, new_line) {
        (_, Some(name)) => {
            check_new_name(repo, state, name, None)?;
            Some(name.to_owned())
        }
        (Position::OnLine(line), None) => Some(line.clone()),
        (Position::AtSave(_), None) => None,
    };
    Ok((line, parent))
}

/// Writes `seen`, holding what this walk vouched for and learned of the files, as the file-state
/// cache, if the drive's "now" was found before the walk; failing to is only a warning.
fn keep_file_cache(
    repo: &Repository,
    seen: FileCache,
    now: Option<i128>,
    warnings: &mut Vec<Warning>,
) {
    if let Some(now) = now
        && let Err(why) = seen.store(repo, now)
    {
        warnings.push(Warning::new(
            warning::FILE_CACHE_NOT_STORED,
            format!(
                "Opalro couldn't keep what it learned about your files, so the next status or save \
                 reads them all again: {why}"
            ),
        ));
    }
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
            "Opalro keeps saving it until it is deleted or moved out of the project folder.",
        )
    } else {
        (
            format!("changes to {count} files"),
            "The last save holds them, so they stay saved.",
            "Opalro keeps saving them until they are deleted or moved out of the project folder.",
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
/// `.opalroignore` line takes back, naming the first few.
fn gitignored_files_saved(mut paths: Vec<String>) -> Warning {
    let count = paths.len() as u64;
    paths.sort_unstable();
    paths.truncate(NAMED);
    let (what, undo) = if count == 1 {
        (
            "1 new or changed file".to_owned(),
            "If it shouldn't be saved, remove the .opalroignore line that takes it back and move it \
             out of the project folder; this save keeps it.",
        )
    } else {
        (
            format!("{count} new or changed files"),
            "If they shouldn't be saved, remove the .opalroignore lines that take them back and \
             move them out of the project folder; this save keeps them.",
        )
    };
    let message = format!(
        "Saved {what} that .gitignore keeps out but .opalroignore takes back:{}\n{} {undo}",
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
    /// An entry of the base save that a partial save leaves as it was.
    Left {
        kind: EntryKind,
        id: Id,
        size: u64,
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

/// Where `build` puts what it reads: the store, for a save, or nowhere, for a look at what changed.
/// What the workers read comes in `read`, in `build_with`'s order, one result for each file the
/// cache doesn't vouch for and each link.
trait Contents {
    /// The ID and size the file-state cache gives the file `c`, which the base save holds as
    /// `was`, if it vouches for it: then it isn't read, and the cache keeps it.
    fn vouched(&mut self, c: &Child, was: Option<&Entry>) -> Option<(Id, u64)>;
    /// The ID and size of the file `c`: `known`, what `vouched` gave, or what was read.
    fn file(
        &mut self,
        read: &mut dyn Iterator<Item = Done>,
        c: &Child,
        known: Option<(Id, u64)>,
    ) -> Result<(Id, u64), SaveError>;
    /// The ID and size of the link `c`'s target.
    fn link(
        &mut self,
        read: &mut dyn Iterator<Item = Done>,
        c: &Child,
    ) -> Result<(Id, u64), SaveError>;
    fn listing(&mut self, listing: Listing) -> Result<Id, SaveError>;
}

/// A save's: everything goes into `packs` unless the store holds a good copy. A file `cache`
/// vouches for is taken from the base save unread, if the base save holds what the cache says it
/// held. `cache` keeps or learns every file met.
struct Storing<'a> {
    store: &'a Store,
    packs: &'a mut PackSet,
    cache: FileCache,
}

/// The ID and size `trusted` gives the file `c`, which the base save holds as `was`, if it vouches
/// for it: then a save doesn't read it.
fn vouched_by(trusted: &FileCache, c: &Child, was: Option<&Entry>) -> Option<(Id, u64)> {
    // The cache never names content to a save: it only says the base save's entry still holds.
    let known = trusted
        .matches(&c.rel, c.meta.as_ref()?)
        .filter(|k| saved_as(was, k))?;
    Some((known.id, known.size))
}

/// Whether the base save's entry `was` holds the content and size the cache knows.
fn saved_as(was: Option<&Entry>, known: &FileState) -> bool {
    was.is_some_and(|e| (e.id, e.size) == (known.id, known.size))
}

impl Contents for Storing<'_> {
    fn vouched(&mut self, c: &Child, was: Option<&Entry>) -> Option<(Id, u64)> {
        // As `vouched_by`, with the entry kept in the same step.
        let known = self
            .cache
            .vouch_if(&c.rel, c.meta.as_ref()?, |k| saved_as(was, k))?;
        Some((known.id, known.size))
    }

    fn file(
        &mut self,
        read: &mut dyn Iterator<Item = Done>,
        c: &Child,
        known: Option<(Id, u64)>,
    ) -> Result<(Id, u64), SaveError> {
        if let Some(known) = known {
            return Ok(known);
        }
        let (id, size) = self.store_read(read, &c.rel)?;
        // The walk's details, which the read found the file still had.
        if let Some(meta) = &c.meta {
            self.cache.record(&c.rel, meta, id);
        }
        Ok((id, size))
    }

    fn link(
        &mut self,
        read: &mut dyn Iterator<Item = Done>,
        c: &Child,
    ) -> Result<(Id, u64), SaveError> {
        self.store_read(read, &c.rel)
    }

    fn listing(&mut self, listing: Listing) -> Result<Id, SaveError> {
        store_listing(self.store, self.packs, &listing)
    }
}

impl Storing<'_> {
    /// Stores what the workers made of `rel`, the next file or link in the order. A big file
    /// they leave to this thread.
    fn store_read(
        &mut self,
        read: &mut dyn Iterator<Item = Done>,
        rel: &str,
    ) -> Result<(Id, u64), SaveError> {
        match read.next() {
            Some(Done::Read { rel: of, got }) if of == rel => {
                let ready = got?;
                ingest::add_ready(self.packs, &ready).map_err(|e| ingest_failed(e, rel))?;
                Ok((ready.id, ready.size))
            }
            Some(Done::Big(at)) if at.rel == rel => {
                let seen = at.meta.as_ref();
                store_file(
                    self.packs,
                    self.store,
                    &at.dir,
                    &at.name,
                    rel,
                    seen,
                    &mut || {},
                )
            }
            _ => Err(out_of_order()),
        }
    }
}

/// What the workers made of the next file or link in the order, which isn't stored: its ID and
/// size, if `rel` is the one it is.
fn hashed_read(read: &mut dyn Iterator<Item = Done>, rel: &str) -> Result<(Id, u64), SaveError> {
    match read.next() {
        Some(Done::Read { rel: of, got }) if of == rel => got.map(|ready| (ready.id, ready.size)),
        _ => Err(out_of_order()),
    }
}

/// A result that isn't the one `build_with` asked for, which no input can cause.
fn out_of_order() -> SaveError {
    RepoError::from(io::Error::other(
        "the project's files were read out of order",
    ))
    .into()
}

/// A file or link to read, through the handle of its folder, held open until it is read.
struct At {
    dir: Arc<Dir>,
    name: String,
    rel: String,
    /// What the walk saw of it.
    meta: Option<EntryMeta>,
}

impl At {
    fn size(&self) -> u64 {
        self.meta.map_or(0, |m| m.size)
    }
}

/// A job for the workers, for a file or link `build_with` needs read.
enum Job {
    /// A file: a save's small one read whole and made ready for a pack, a status's only hashed.
    File(At),
    /// A link's target.
    Link(At),
    /// A big file a save stores on the writer's thread, its chunks on workers of their own.
    Big(At),
    /// One whose folder couldn't be opened.
    Failed { rel: String, why: SaveError },
}

/// What a worker made of a job.
enum Done {
    Read {
        rel: String,
        got: Result<Ready, SaveError>,
    },
    Big(At),
}

impl Job {
    /// The most the job holds from when it is handed out until its result is taken.
    fn cost(&self, storing: bool) -> usize {
        match self {
            Job::File(at) if storing => ingest::cost(at.size()),
            // Hashed through one buffer, however long the file is.
            Job::File(_) => READ_BUFFER,
            Job::Link(at) => ingest::cost(at.size()),
            // Nothing else goes out while the writer stores a big file.
            Job::Big(_) => WINDOW,
            Job::Failed { .. } => 0,
        }
    }

    /// What a worker does: a save's (`store` given) reads a small file or a link whole and makes
    /// it ready for a pack, a status's only hashes what it reads.
    fn run(self, store: Option<&Store>) -> Done {
        let (rel, got) = match self {
            Job::Big(at) => return Done::Big(at),
            Job::Failed { rel, why } => (rel, Err(why)),
            Job::File(at) => {
                let got = match store {
                    Some(store) => read_small(store, &at),
                    None => hash_scanned(&at.dir, &at.name, &at.rel, at.meta.as_ref(), &mut || {})
                        .map(|(_, id, size)| Ready {
                            id,
                            size,
                            stored: None,
                        }),
                };
                (at.rel, got)
            }
            Job::Link(at) => {
                let got = link_target(&at.dir, &at.name, &at.rel).and_then(|target| {
                    let bytes = target.as_bytes();
                    match store {
                        Some(store) => {
                            ingest::ready(store, bytes).map_err(|e| ingest_failed(e, &at.rel))
                        }
                        None => Ok(Ready {
                            id: hash_content(bytes),
                            size: bytes.len() as u64,
                            stored: None,
                        }),
                    }
                });
                (at.rel, got)
            }
        };
        Done::Read { rel, got }
    }
}

/// The files and links `build_with` reads, in its order, as jobs: `todo` holds each with the
/// project path of its folder, which is opened through `handles` when its job is made.
struct Jobs<'a> {
    handles: &'a mut Handles,
    todo: std::vec::IntoIter<(&'a str, &'a Child)>,
    storing: Option<&'a Store>,
}

impl Iterator for Jobs<'_> {
    type Item = Job;

    fn next(&mut self) -> Option<Job> {
        let (folder, c) = self.todo.next()?;
        let dir = match self.handles.shared(folder) {
            Ok(dir) => dir,
            Err(why) => {
                let rel = c.rel.clone();
                return Some(Job::Failed { rel, why });
            }
        };
        let at = At {
            dir,
            name: c.name.clone(),
            rel: c.rel.clone(),
            meta: c.meta,
        };
        Some(match c.kind {
            Kind::Symlink => Job::Link(at),
            _ if self
                .storing
                .is_some_and(|s| c.meta.is_some_and(|m| !s.is_small(m.size))) =>
            {
                Job::Big(at)
            }
            // One the walk has no details of fails as it is opened, as any file does.
            _ => Job::File(at),
        })
    }
}

/// Reads the small file `at` whole, as `store_file` does, and makes it ready for a pack.
fn read_small(store: &Store, at: &At) -> Result<Ready, SaveError> {
    let bytes = read_scanned(&at.dir, &at.name, &at.rel, at.meta.as_ref())?;
    ingest::ready(store, &bytes).map_err(|e| ingest_failed(e, &at.rel))
}

/// The bytes of the file `name` in `dir`, at the project path `rel`, read whole, if it is the file
/// the walk saw (`seen`; see `open_scanned`) and doesn't change while it is read.
fn read_scanned(
    dir: &Dir,
    name: &str,
    rel: &str,
    seen: Option<&EntryMeta>,
) -> Result<Vec<u8>, SaveError> {
    let (mut file, opened) = open_scanned(dir, name, rel, seen)?;
    let bytes = ingest::read_whole(&mut file, opened.size).map_err(|e| ingest_failed(e, rel))?;
    let now = stamp(&file).map_err(|source| SaveError::Unreadable {
        path: rel.to_owned(),
        source,
    })?;
    if now != stamp_of(&opened) {
        return Err(SaveError::ChangedDuringSave {
            path: rel.to_owned(),
        });
    }
    Ok(bytes)
}

/// What a save says when bytes of the project path `rel` couldn't be stored.
pub(crate) fn ingest_failed(e: IngestError, rel: &str) -> SaveError {
    match e {
        IngestError::Changed => SaveError::ChangedDuringSave {
            path: rel.to_owned(),
        },
        IngestError::Read(source) => SaveError::Unreadable {
            path: rel.to_owned(),
            source,
        },
        IngestError::Pack(e) => e.into(),
        IngestError::Store(e) => e.into(),
    }
}

/// The base save's listing of each folder the walk found, if it has that folder, and the stored
/// ID of each nothing was dropped from.
#[derive(Clone, Copy)]
struct Last<'a> {
    listings: &'a HashMap<String, Option<Listing>>,
    ids: &'a HashMap<String, Id>,
}

/// `build_with`, its files and links read on the workers. For a save (`store` given) a small file
/// is read whole and made ready for a pack there, and a big one is stored by this thread with
/// its chunks on workers of their own; for a status every file is only hashed. Only this thread
/// writes packs, in `build_with`'s order. Folders are opened through `handles`.
fn build_reading(
    contents: &mut dyn Contents,
    store: Option<&Store>,
    handles: &mut Handles,
    folders: &Folders,
    last: Last,
    noted: &Noted,
) -> Result<Built, SaveError> {
    let mut todo = Vec::new();
    // What the cache said of each file, for `build_with`, which meets them in this same order.
    let mut known = Vec::new();
    for (rel, children) in in_build_order(folders) {
        let before = last.listings.get(rel).and_then(Option::as_ref);
        for c in children {
            let read = match c.kind {
                Kind::File => {
                    let vouched = contents.vouched(c, before.and_then(|l| l.get(&c.name)));
                    known.push((c.rel.as_str(), vouched));
                    vouched.is_none()
                }
                Kind::Symlink => true,
                _ => false,
            };
            if read {
                todo.push((rel.as_str(), c));
            }
        }
    }
    let storing = store.is_some();
    with_workers(|workers| {
        // Each job out holds its folder open.
        handles.lend(workers.most_out());
        let jobs = Jobs {
            handles: &mut *handles,
            todo: todo.into_iter(),
            storing: store,
        };
        let built = workers.results(
            jobs,
            WINDOW,
            |job| job.cost(storing),
            |job| job.run(store),
            |read| {
                let known = &mut known.into_iter();
                build_with(contents, read, known, folders, last, noted)
            },
        );
        handles.lend(0);
        built
    })
}

/// What `build_with` found.
struct Built {
    root: Id,
    /// The paths of the entries kept as they were, but for those in folders gone from disk.
    kept: Vec<String>,
    /// How many entries were kept as they were in folders gone from disk.
    kept_below_gone: usize,
    /// The files and links noted as ignored that changed.
    changed: Vec<String>,
    /// The files and links noted as kept that Git would leave out that are new or changed.
    taken_back: Vec<String>,
}

/// Stores every file and listing, deepest folders first, reading every file and link through
/// `handles`. Returns the top listing's ID, how many entries were kept as they were, which of the
/// files and links `noted` as ignored changed, and which of those noted as kept that Git would
/// leave out are new or changed.
#[cfg(test)]
fn build(
    store: &Store,
    packs: &mut PackSet,
    handles: &mut Handles,
    folders: &Folders,
    old: &HashMap<String, Option<Listing>>,
    noted: &Noted,
) -> Result<(Id, usize, Vec<String>, Vec<String>), SaveError> {
    let mut storing = Storing {
        store,
        packs,
        cache: FileCache::default(),
    };
    let last = Last {
        listings: old,
        ids: &HashMap::new(),
    };
    let b = build_reading(&mut storing, Some(store), handles, folders, last, noted)?;
    let kept = b.kept.len() + b.kept_below_gone;
    Ok((b.root, kept, b.changed, b.taken_back))
}

/// The order `build_with` builds folders in: deepest first, and by path at one depth.
fn build_order(folders: &Folders) -> Vec<&String> {
    let mut order: Vec<&String> = folders.keys().collect();
    order.sort_by_key(|rel| {
        Reverse(if rel.is_empty() {
            0
        } else {
            rel.split('/').count()
        })
    });
    order
}

/// Each folder in `build_order`, with its children: both of `build_reading`'s passes go through
/// this, so they meet the files in the same order.
fn in_build_order(folders: &Folders) -> impl Iterator<Item = (&String, &[Child])> {
    build_order(folders)
        .into_iter()
        .map(|rel| (rel, folders.get(rel).map_or(&[][..], Vec::as_slice)))
}

#[cfg(test)]
thread_local! {
    /// The folders whose listings `build_with` has made on this thread.
    static LISTINGS_BUILT: std::cell::RefCell<Vec<String>> = const {
        std::cell::RefCell::new(Vec::new())
    };
    /// How many entries `build_with` has made on this thread.
    static ENTRIES_BUILT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// The listings of `folders`, in `build_order`, with each file, link and listing handed to
/// `contents`, what the cache said of each file in `known`, and what was read of the files and
/// links in `read`. A folder whose entries are all its last listing's has that listing's ID.
fn build_with<'a>(
    contents: &mut dyn Contents,
    read: &mut dyn Iterator<Item = Done>,
    known: &mut dyn Iterator<Item = (&'a str, Option<(Id, u64)>)>,
    folders: &'a Folders,
    last: Last,
    noted: &Noted,
) -> Result<Built, SaveError> {
    let mut built: HashMap<&str, Id> = HashMap::new();
    let missing = || RepoError::from(io::Error::other("a folder was built before its subfolders"));
    let (mut kept, mut kept_below_gone) = (Vec::new(), 0);
    let (mut changed, mut taken_back) = (Vec::new(), Vec::new());
    for (rel, children) in in_build_order(folders) {
        let before = last.listings.get(rel).and_then(Option::as_ref);
        let mut found = Vec::new();
        for c in children {
            let was = before.and_then(|l| l.get(&c.name));
            let (kind, id, size) = match c.kind {
                Kind::Folder => {
                    let id = built.remove(c.rel.as_str()).ok_or_else(missing)?;
                    (EntryKind::Folder, id, 0)
                }
                Kind::File => {
                    let vouched = match known.next() {
                        Some((of, vouched)) if std::ptr::eq(of, c.rel.as_str()) => vouched,
                        _ => return Err(out_of_order()),
                    };
                    let (id, size) = contents.file(read, c, vouched)?;
                    // Git writes a link it can't make as a file holding its target: the link
                    // stays (decision 039).
                    let links_target =
                        |e: &Entry| e.kind == EntryKind::Symlink && (e.id, e.size) == (id, size);
                    if was.is_some_and(links_target) {
                        kept.push(c.rel.clone());
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
                    let (id, size) = contents.link(read, c)?;
                    (EntryKind::Symlink, id, size)
                }
                Kind::Kept { kind, id, size } => {
                    kept.push(c.rel.clone());
                    (kind, id, size)
                }
                Kind::Left { kind, id, size } => (kind, id, size),
                Kind::Back { id, kept: below } => {
                    kept_below_gone += below;
                    (EntryKind::Folder, id, 0)
                }
            };
            if was.is_none_or(|e| (e.kind, e.id) != (kind, id)) {
                match noted.of(rel, c, was) {
                    Some(Note::Ignored) => changed.push(c.rel.clone()),
                    Some(Note::GitIgnored) => taken_back.push(c.rel.clone()),
                    None => {}
                }
            }
            found.push((c, kind, id, size));
        }
        // The store holds that listing: the walk just read it and checked it.
        let unchanged = before.is_some_and(|l| is_listing(l, &found));
        if let Some(&id) = last.ids.get(rel).filter(|_| unchanged) {
            built.insert(rel, id);
            continue;
        }
        #[cfg(test)]
        LISTINGS_BUILT.with_borrow_mut(|b| b.push(rel.clone()));
        #[cfg(test)]
        ENTRIES_BUILT.set(ENTRIES_BUILT.get() + found.len());
        let entries = found
            .into_iter()
            .map(|(c, kind, id, size)| Entry {
                name: c.name.clone(),
                kind,
                id,
                size,
            })
            .collect();
        let listing = Listing::new(entries).map_err(StoreError::from)?;
        built.insert(rel, contents.listing(listing)?);
    }
    let root = built.remove("").ok_or_else(missing)?;
    Ok(Built {
        root,
        kept,
        kept_below_gone,
        changed,
        taken_back,
    })
}

/// Whether `children`, each with the kind, ID and size it was found to have, are `listing`'s
/// entries, in order.
fn is_listing(listing: &Listing, children: &[(&Child, EntryKind, Id, u64)]) -> bool {
    let entries = listing.entries();
    entries.len() == children.len()
        && entries
            .iter()
            .zip(children)
            .all(|(e, &(c, kind, id, size))| {
                e.name == c.name && e.kind == kind && e.id == id && e.size == size
            })
}

/// Stores `listing` unless a good copy is held already, and returns its ID.
pub(crate) fn store_listing(
    store: &Store,
    packs: &mut PackSet,
    listing: &Listing,
) -> Result<Id, SaveError> {
    let id = listing.id();
    let encoded = listing.encode();
    let len = encoded.len() as u64;
    if !packs.contains(&id) && !store.has_good_copy(&id, ObjectKind::FolderListing, len)? {
        packs.add_record(ObjectKind::FolderListing, &encoded)?;
    }
    Ok(id)
}

/// The target of the link `name` in `dir`, at the project path `rel`, as a save stores it: with
/// `/` between its parts, the link itself never followed. One that is no longer a link changed
/// since the walk.
pub(crate) fn link_target(dir: &Dir, name: &str, rel: &str) -> Result<String, SaveError> {
    let unreadable = |source| SaveError::Unreadable {
        path: rel.to_owned(),
        source,
    };
    let name = OsStr::new(name);
    let target = dir
        .read_link(name)
        .map_err(|source| match dir.metadata(name) {
            Ok(now) if now.kind != EntryType::Symlink => SaveError::ChangedDuringSave {
                path: rel.to_owned(),
            },
            _ => unreadable(source),
        })?;
    let target = target.into_string().map_err(|_| {
        unreadable(io::Error::new(
            io::ErrorKind::InvalidData,
            "the link's target is not valid UTF-8",
        ))
    })?;
    Ok(link::stored_target(target))
}

/// Opens the file `name` in `dir`, at the project path `rel`, to read, if it is still the regular
/// file the walk saw (`seen`): the same file, size and times, so what is stored is what the walk
/// described. Anything else changed since, unless it is a Windows reparse point that isn't a link
/// (a cloud placeholder, say), which Opalro can't save yet. Returns the file and its details.
fn open_scanned(
    dir: &Dir,
    name: &str,
    rel: &str,
    seen: Option<&EntryMeta>,
) -> Result<(File, EntryMeta), SaveError> {
    let name = OsStr::new(name);
    let path = || rel.to_owned();
    let file = fsio::open_point(rel)
        .and_then(|()| dir.open_file(name))
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
    if seen.is_none_or(|seen| !same_file(seen, &now)) {
        return Err(SaveError::ChangedDuringSave { path: path() });
    }
    Ok((file, now))
}

/// What tells that a file changed: its size and its modification and change times, as the
/// file-state cache compares them.
fn stamp_of(m: &EntryMeta) -> (u64, i128, i128) {
    (m.size, m.modified_ns, m.changed_ns)
}

/// Whether `now` is still the file seen as `seen`: the same file, with the same `stamp_of`.
pub(crate) fn same_file(seen: &EntryMeta, now: &EntryMeta) -> bool {
    seen.file_id == now.file_id && stamp_of(seen) == stamp_of(now)
}

/// The `stamp_of` the open `file` has now, to see whether it changed while it was read.
fn stamp(file: &File) -> io::Result<(u64, i128, i128)> {
    Dir::file_meta(file).map(|m| stamp_of(&m))
}

/// Stores one file into `packs`, reading it once: a big one is cut into chunks as it is read. The
/// file is `name` in `dir`, at the project path `rel`, and must be the one the walk saw (`seen`;
/// see `open_scanned`). A file that changes while it is read gives `ChangedDuringSave`, which is
/// checked once `after_read` has run.
pub(crate) fn store_file(
    packs: &mut PackSet,
    store: &Store,
    dir: &Dir,
    name: &str,
    rel: &str,
    seen: Option<&EntryMeta>,
    after_read: &mut dyn FnMut(),
) -> Result<(Id, u64), SaveError> {
    let unreadable = |source| SaveError::Unreadable {
        path: rel.to_owned(),
        source,
    };
    let changed = || SaveError::ChangedDuringSave {
        path: rel.to_owned(),
    };
    if seen.is_some_and(|seen| store.is_streamed(seen.size)) {
        // Hashed first, then stored whole unless the store holds it already.
        let (mut file, id, size) = hash_scanned(dir, name, rel, seen, after_read)?;
        let again = || {
            file.seek(SeekFrom::Start(0))
                .map(|_| file)
                .map_err(unreadable)
        };
        if !ingest::store_long(packs, store, id, size, again)? {
            return Err(changed());
        }
        return Ok((id, size));
    }
    let (mut file, opened) = open_scanned(dir, name, rel, seen)?;
    let before = stamp_of(&opened);
    // Chunks stored before a change is found stay in the packs, which nothing names yet.
    let id = ingest::store_bytes(packs, store, &mut file, opened.size)
        .map_err(|e| ingest_failed(e, rel))?;
    after_read();
    if stamp(&file).map_err(unreadable)? != before {
        return Err(changed());
    }
    Ok((id, opened.size))
}

/// What `hash_scanned` reads a file through.
const READ_BUFFER: usize = 1 << 20;

/// Reads the file `name` in `dir` through and gives it, still open, with its content ID and size:
/// to know the ID, or as the first of two reads of a file that isn't cut. It must stay the file
/// the walk saw (`seen`) until read, which is checked once `after_read` has run.
fn hash_scanned(
    dir: &Dir,
    name: &str,
    rel: &str,
    seen: Option<&EntryMeta>,
    after_read: &mut dyn FnMut(),
) -> Result<(File, Id, u64), SaveError> {
    let unreadable = |source| SaveError::Unreadable {
        path: rel.to_owned(),
        source,
    };
    let changed = || SaveError::ChangedDuringSave {
        path: rel.to_owned(),
    };
    let (mut file, opened) = open_scanned(dir, name, rel, seen)?;
    let before = stamp_of(&opened);
    let mut hasher = ContentHasher::new();
    let mut buf = vec![0u8; READ_BUFFER];
    let mut size = 0u64;
    loop {
        // A worker whose results are no longer wanted stops reading here.
        if workers::stopped() {
            let stopped = io::Error::new(io::ErrorKind::Interrupted, "the read was stopped");
            return Err(RepoError::from(stopped).into());
        }
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
    after_read();
    if size != before.0 || stamp(&file).map_err(unreadable)? != before {
        return Err(changed());
    }
    Ok((file, id, size))
}

/// What the project folder holds now, as a save would take it, read but not stored.
pub(crate) struct FolderNow {
    /// Its top listing.
    pub(crate) root: Id,
    /// Its listings, which the store may not hold, and the base save's the walk read, by ID.
    pub(crate) listings: HashMap<Id, Listing>,
    /// The entries kept as the save has them because this computer can't hold them as they are,
    /// by path.
    pub(crate) not_written_here: Vec<String>,
    pub(crate) warnings: Vec<Warning>,
}

/// Reads the project folder as a save on top of the save whose top listing is `base` would, by
/// the same rules, but stores nothing. A file the file-state cache vouches for isn't opened,
/// unless `full`; the cache is then written again, and failing to is only a warning. `drive` is
/// what `drive::probe` found, if the caller has tested it already.
pub(crate) fn read_folder(
    repo: &Repository,
    base: Id,
    full: bool,
    git_excludes: Option<&Path>,
    drive: Option<Drive>,
) -> Result<FolderNow, SaveError> {
    // Taken before the walk, so a file changed during it is too new to trust next time. Without
    // it no time can be trusted, so every file is read and nothing cached.
    let now = filecache::drive_now(repo).ok();
    let drive = match drive {
        Some(drive) => drive,
        None => drive::probe(repo.tmp_folder()).map_err(RepoError::from)?,
    };
    let walked = || {
        walk(
            repo.root(),
            Base::new(repo.store(), base, SaveLimits::default()),
            SaveLimits::default(),
            drive,
            git_excludes,
        )
    };
    let (scan, cache) = match now {
        Some(_) if !full => FileCache::load_during(repo, walked),
        _ => (walked(), FileCache::default()),
    };
    let Scan {
        mut handles,
        folders,
        mut old,
        old_ids,
        rebuilt,
        gone_kept,
        noted,
        mut warnings,
    } = scan?;
    let mut reading = Reading {
        cache,
        listings: HashMap::new(),
    };
    for listing in rebuilt {
        reading.listing(listing)?;
    }
    let last = Last {
        listings: &old,
        ids: &old_ids,
    };
    let built = build_reading(&mut reading, None, &mut handles, &folders, last, &noted)?;
    keep_file_cache(repo, reading.cache, now, &mut warnings);
    let mut not_written_here = built.kept;
    not_written_here.extend(gone_kept);
    not_written_here.sort_unstable();
    // So the comparisons that follow find the base save's listings without reading them again.
    let mut listings = reading.listings;
    for (rel, id) in old_ids {
        if let Some(Some(listing)) = old.remove(&rel) {
            listings.entry(id).or_insert(listing);
        }
    }
    Ok(FolderNow {
        root: built.root,
        listings,
        not_written_here,
        warnings,
    })
}

/// Reads files and links to know their IDs, opening only those `cache` doesn't vouch for, and
/// keeps listings in memory. `cache` keeps or learns every file met.
struct Reading {
    cache: FileCache,
    listings: HashMap<Id, Listing>,
}

impl Contents for Reading {
    fn vouched(&mut self, c: &Child, _was: Option<&Entry>) -> Option<(Id, u64)> {
        let meta = c.meta.as_ref()?;
        let known = self.cache.vouch(&c.rel, meta)?;
        Some((known.id, meta.size))
    }

    fn file(
        &mut self,
        read: &mut dyn Iterator<Item = Done>,
        c: &Child,
        known: Option<(Id, u64)>,
    ) -> Result<(Id, u64), SaveError> {
        if let Some(known) = known {
            return Ok(known);
        }
        let (id, size) = hashed_read(read, &c.rel)?;
        // The walk's details, which the read found the file still had.
        if let Some(meta) = &c.meta {
            self.cache.record(&c.rel, meta, id);
        }
        Ok((id, size))
    }

    fn link(
        &mut self,
        read: &mut dyn Iterator<Item = Done>,
        c: &Child,
    ) -> Result<(Id, u64), SaveError> {
        hashed_read(read, &c.rel)
    }

    fn listing(&mut self, listing: Listing) -> Result<Id, SaveError> {
        let id = listing.id();
        self.listings.insert(id, listing);
        Ok(id)
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
    use crate::test_support::{
        Written, commit_records, names, noise, plant_damaged_copies, plant_save, sample_records,
        written,
    };
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

    /// A new project, its caches sealed with a key as the command line hands over the user's.
    pub(super) fn project() -> (tempfile::TempDir, Repository) {
        let dir = tempfile::tempdir().unwrap();
        let mut repo = Repository::init(dir.path(), &who()).unwrap();
        repo.set_cache_key([7; 32]);
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
            message: Some("Change things"),
            who: &who,
            command: "save",
            accept_crlf: false,
            git_excludes,
            new_line: None,
            only: None,
            amend: false,
        };
        save_folder(repo, &req, &mut |_| {}, &mut |_| true)
    }

    /// A save of only `only`.
    pub(super) fn save_only(
        repo: &mut Repository,
        only: &[Given],
    ) -> Result<SaveOutcome, SaveError> {
        let who = who();
        let req = SaveRequest {
            message: Some("Some things"),
            who: &who,
            command: "save",
            accept_crlf: false,
            git_excludes: None,
            new_line: None,
            only: Some(only),
            amend: false,
        };
        save_folder(repo, &req, &mut |_| {}, &mut |_| true)
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

    /// The project folder as it is on disk, without `.opalro`.
    fn disk_tree(root: &Path) -> Tree {
        fn walk(dir: &Path, prefix: &str, out: &mut Tree) {
            for e in fs::read_dir(dir).unwrap() {
                let e = e.unwrap();
                let name = e.file_name().into_string().unwrap();
                if prefix.is_empty() && name == ".opalro" {
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

    /// The files the two warnings would name, from a walk and a build that read the project as
    /// `opalro status` does, or the first problem.
    fn warned(repo: &Repository) -> Result<(Vec<String>, Vec<String>), String> {
        let shown = |e: SaveError| format!("{e:?}");
        let Scan {
            mut handles,
            folders,
            old,
            noted,
            ..
        } = walk_project(repo, SaveLimits::default()).map_err(shown)?;
        let mut reading = Reading {
            cache: FileCache::default(),
            listings: HashMap::new(),
        };
        let last = Last {
            listings: &old,
            ids: &HashMap::new(),
        };
        let built = build_reading(&mut reading, None, &mut handles, &folders, last, &noted);
        let built = built.map_err(shown)?;
        Ok((built.changed, built.taken_back))
    }

    /// A path up to three levels down, from a few names.
    fn late_path() -> impl Strategy<Value = String> {
        let folders = prop::collection::vec(prop::sample::select(vec!["a", "b"]), 0..3);
        let name = prop::sample::select(vec!["a", "b", "c.x", "d.y"]);
        (folders, name).prop_map(|(folders, name)| {
            folders.iter().map(|f| format!("{f}/")).collect::<String>() + name
        })
    }

    /// A few lines of an ignore file.
    fn rule_lines() -> impl Strategy<Value = Vec<&'static str>> {
        let lines = [
            "*.x", "!*.x", "c.x", "!c.x", "*.y", "d.y", "!d.y", "a/", "!a/", "b", "!b", "/a",
            "a/c.x", "!a/c.x", "*",
        ];
        prop::collection::vec(prop::sample::select(lines.to_vec()), 1..5)
    }

    /// An ignore file: its folder, whether it is a `.gitignore`, and its lines.
    fn rule_file() -> impl Strategy<Value = (String, bool, Vec<&'static str>)> {
        // Mostly where the files are.
        let folder = prop::sample::select(vec!["", "", "", "a/", "b/", "a/a/", "b/a/"]);
        (folder.prop_map(str::to_owned), any::<bool>(), rule_lines())
    }

    /// Writes `bytes` at `rel`, unless a file or folder of the other kind is in the way.
    fn put(root: &Path, rel: &str, bytes: &[u8]) {
        let path = root.join(rel);
        let _ = fs::create_dir_all(path.parent().unwrap());
        let _ = fs::write(path, bytes);
    }

    fn put_rules(root: &Path, (folder, gitignore, lines): &(String, bool, Vec<&str>)) {
        let name = if *gitignore {
            ".gitignore"
        } else {
            ".opalroignore"
        };
        put(
            root,
            &format!("{folder}{name}"),
            lines.join("\n").as_bytes(),
        );
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(256))]
        #[test]
        fn the_late_verdict_names_the_files_the_walk_would(
            files in prop::collection::vec(late_path(), 0..16),
            rules in prop::collection::vec(rule_file(), 0..3),
            git_after in rule_lines(),
            opal_after in rule_lines(),
            rules_after in prop::collection::vec(rule_file(), 0..3),
            changes in prop::collection::vec((0..6u8, any::<prop::sample::Index>()), 1..12),
        ) {
            let (dir, mut repo) = project();
            let root = dir.path();
            for rel in &files {
                put(root, rel, b"first");
            }
            for file in &rules {
                put_rules(root, file);
            }
            let _ = save_now(&mut repo);
            // Mostly the saved files, changed under new rules.
            put_rules(root, &(String::new(), true, git_after));
            put_rules(root, &(String::new(), false, opal_after));
            for file in &rules_after {
                put_rules(root, file);
            }
            for (what, at) in &changes {
                let Some(file) = files.get(at.index(files.len().max(1))) else {
                    continue;
                };
                // A saved file, or the folder that holds it.
                let rel = match file.rsplit_once('/') {
                    Some((up, _)) if *what == 5 => up,
                    _ => file.as_str(),
                };
                let path = root.join(rel);
                match what {
                    3 => drop(fs::remove_file(&path)),
                    4 => {
                        let _ = fs::remove_file(&path);
                        put(root, &format!("{rel}/c.x"), b"was a file");
                    }
                    5 => {
                        let _ = fs::remove_dir_all(&path);
                        put(root, rel, b"was a folder");
                    }
                    _ => put(root, rel, b"changed"),
                }
            }
            let late = warned(&repo);
            walk::verdicts::match_every_name(repo.root(), true);
            let every_name = warned(&repo);
            walk::verdicts::match_every_name(repo.root(), false);
            prop_assert_eq!(late, every_name);
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
            replaced,
        } = save_now(&mut repo).unwrap()
        else {
            panic!("nothing was saved");
        };
        assert_eq!(replaced, None);
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
        // One key for every handle, as the user's own key would be.
        const KEY: [u8; 32] = [7; 32];
        let (dir, mut repo) = project();
        repo.set_cache_key(KEY);
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
        let mut fresh = Repository::open(dir.path()).unwrap();
        fresh.set_cache_key(KEY);
        let (id, change_id, short) = shorts.last().unwrap();
        assert_eq!(
            *short,
            history::short_change_id(&fresh, id, change_id).unwrap()
        );
        // History reads the line once.
        let mut fresh = Repository::open(dir.path()).unwrap();
        fresh.set_cache_key(KEY);
        history::for_each_on_line::<RepoError>(&fresh, |_, _, _| Ok(())).unwrap();
        assert_eq!(fresh.store().saves_read(), 7);
    }

    #[test]
    fn save_stores_a_file_even_when_a_damaged_copy_claims_its_id() {
        let (dir, _repo) = project();
        // Content anyone can predict, like a licence, in a pack that came with the project.
        let licence = b"Permission is hereby granted, free of charge, to any person\n";
        plant_damaged_copies(
            &dir.path().join(".opalro/store/packs"),
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

    /// The only copy is in a pack too damaged to open, which might hold a good one: storing it
    /// again is harmless, where stopping would leave the user unable to save.
    #[test]
    fn a_save_stores_again_past_a_damaged_only_copy() {
        let (dir, _repo) = project();
        let licence = b"Permission is hereby granted, free of charge, to any person\n";
        let opalro = dir.path().join(".opalro");
        let mut w = crate::pack::PackWriter::create(&opalro.join("tmp")).unwrap();
        w.add_content(licence).unwrap();
        let pack = w.finish(&opalro.join("store/packs")).unwrap().unwrap();
        let mut bytes = fs::read(&pack).unwrap();
        *bytes.last_mut().unwrap() ^= 1;
        fs::write(&pack, bytes).unwrap();
        write(dir.path(), "LICENSE", licence);
        let mut repo = Repository::open(dir.path()).unwrap();
        let root = saved_root(save_now(&mut repo).unwrap());
        let name = pack.file_name().unwrap().to_str().unwrap();
        let warnings = repo.store().warnings();
        let named = |w: &Warning| w.kind == warning::PACK_DAMAGED && w.message.contains(name);
        assert!(warnings.iter().any(named), "{warnings:?}");
        let fresh = Repository::open(dir.path()).unwrap();
        assert_eq!(saved_tree(fresh.store(), root), disk_tree(dir.path()));
    }

    /// What a key for one run seals, no later run trusts: writing it would only replace a cache
    /// the user's own key sealed.
    #[test]
    fn a_run_only_key_writes_no_cache() {
        let (dir, mut repo) = project();
        write(dir.path(), "a.txt", b"1");
        save_now(&mut repo).unwrap();
        let cache = dir.path().join(".opalro/cache");
        let read = |name: &str| fs::read(cache.join(name)).unwrap();
        let before = (read("change-ids"), read("files"));
        // No key given, as a program using the engine runs it.
        let mut repo = Repository::open(dir.path()).unwrap();
        write(dir.path(), "a.txt", b"2");
        save_now(&mut repo).unwrap();
        history::for_each_on_line::<RepoError>(&repo, |_, _, _| Ok(())).unwrap();
        assert!((read("change-ids"), read("files")) == before);
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
            &dir.path().join(".opalro/store/packs"),
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
        fs::remove_file(dir.path().join(".opalro/cache/change-ids")).unwrap();

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
                "The save was made, but Opalro couldn't read this line's older saves to shorten its ID."
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

    /// RT-11: a folder Opalro may not enter is an unreadable folder, not a `.opalroignore` problem.
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

    /// Status only reads, so what it says of a file it couldn't read never speaks of saving.
    #[test]
    fn a_file_problem_reads_right_for_a_command_that_only_reads() {
        let path = || "a.txt".to_owned();
        let cases = [
            (
                SaveError::ChangedDuringSave { path: path() },
                "a.txt changed while Opalro was reading it.",
                "Try again when the file stops changing, or list it in .opalroignore.",
            ),
            (
                SaveError::SpecialFile { path: path() },
                "a.txt is a special file Opalro can't read yet, such as a cloud placeholder.",
                "List it in .opalroignore for now.",
            ),
            (
                SaveError::Unreadable {
                    path: path(),
                    source: io::Error::other("denied"),
                },
                "Can't read a.txt: denied.",
                "Let Opalro read it, or list it in .opalroignore.",
            ),
        ];
        for (e, message, step) in cases {
            assert_eq!(e.to_string(), message);
            assert_eq!(e.hint_when_reading().unwrap(), step);
            assert_eq!(e.hint().unwrap(), format!("Nothing was saved. {step}"));
        }
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
            SaveError::LfsPointersInFolder {
                paths: vec!["fine".into(), hostile()],
                count: 2,
            },
            SaveError::AmendShared {
                by: vec!["line fine".into(), format!("tag {HOSTILE}")],
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
        // The lines Opalro writes itself stay lines; the folders and names in them are escaped.
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
    fn the_files_opalroignore_takes_back_from_gitignore_are_named_escaped_and_counted() {
        use crate::test_support::{HOSTILE, assert_hostile_text_escaped};
        assert_hostile_text_escaped(&gitignored_files_saved(vec![HOSTILE.to_owned()]).message);
        let paths = (1..=6).rev().map(|i| format!("k{i}.pem")).collect();
        let message = gitignored_files_saved(paths).message;
        let listed = "Saved 6 new or changed files that .gitignore keeps out but .opalroignore takes \
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
    fn the_opalroignore_hint_does_not_blame_a_pattern() {
        // A link, a pipe, a huge file or one Opalro can't read has no pattern to fix.
        let hint = SaveError::Ignore { detail: "x".into() }.hint().unwrap();
        assert!(!hint.contains("pattern"), "{hint}");
    }

    #[test]
    fn a_refused_amend_names_the_first_five_that_show_the_save() {
        let shown = |by: Vec<String>| SaveError::AmendShared { by }.to_string();
        let one = shown(vec!["line b".into()]);
        assert!(one.contains(": line b shows it too,"), "{one}");
        let many = shown((1..=7).map(|n| format!("tag v{n}")).collect());
        let named = ": tag v1, tag v2, tag v3, tag v4, tag v5 and 2 more show it too,";
        assert!(many.contains(named), "{many}");
    }

    /// A save of the whole folder with `message`, or an amend.
    fn save_as(
        repo: &mut Repository,
        message: Option<&str>,
        amend: bool,
    ) -> Result<SaveOutcome, SaveError> {
        let who = who();
        let req = SaveRequest {
            message,
            who: &who,
            command: "save",
            accept_crlf: false,
            git_excludes: None,
            new_line: None,
            only: None,
            amend,
        };
        save_folder(repo, &req, &mut |_| {}, &mut |_| true)
    }

    /// Commits the line `name` at a new save on `parent` that holds the same files.
    fn plant_line(repo: &mut Repository, name: &str, parent: Id) {
        let lock = repo.lock_for_change("save", &mut |_| {}).unwrap();
        let (head, entry) = repo.current().unwrap();
        let mut save = repo.store().save(&parent, false).unwrap();
        save.parents = vec![parent];
        save.change_id = ChangeId::random().unwrap();
        let mut packs = repo.new_packs().unwrap();
        let id = packs.add_record(ObjectKind::Save, &save.encode()).unwrap();
        let mut state = entry.state.clone();
        state.lines.insert(name.to_owned(), id);
        let diary = DiaryEntry {
            parent: Some(head),
            action: Action::Command,
            command: "save".into(),
            who: who(),
            opal_version: "0.1.0".into(),
            state,
            folder_change: None,
        };
        repo.commit(&lock, packs, &diary).unwrap();
    }

    #[test]
    fn a_save_without_a_message_is_refused_unless_it_amends() {
        let (dir, mut repo) = project();
        write(dir.path(), "a.txt", b"a");
        let head = repo.head().unwrap();
        let refused = save_as(&mut repo, None, false);
        assert!(
            matches!(refused, Err(SaveError::NeedsMessage)),
            "{refused:?}"
        );
        assert_eq!(repo.head().unwrap(), head);
        let Ok(SaveOutcome::Saved { save, .. }) = save_as(&mut repo, None, true) else {
            panic!("nothing was amended");
        };
        assert_eq!(save.message, "Start project");
    }

    #[test]
    fn an_amend_can_t_start_a_new_line() {
        let (dir, mut repo) = project();
        write(dir.path(), "a.txt", b"a");
        let head = repo.head().unwrap();
        let who = who();
        let req = SaveRequest {
            message: Some("A"),
            who: &who,
            command: "save",
            accept_crlf: false,
            git_excludes: None,
            new_line: Some("x"),
            only: None,
            amend: true,
        };
        let refused = save_folder(&mut repo, &req, &mut |_| {}, &mut |_| true);
        assert!(
            matches!(refused, Err(SaveError::AmendNewLine)),
            "{refused:?}"
        );
        assert_eq!(repo.head().unwrap(), head);
    }

    #[test]
    fn a_new_line_from_a_save_can_t_start_with_a_dash() {
        let (dir, mut repo) = project();
        write(dir.path(), "a.txt", b"a");
        let head = repo.head().unwrap();
        let who = who();
        let req = SaveRequest {
            message: Some("A"),
            who: &who,
            command: "save",
            accept_crlf: false,
            git_excludes: None,
            new_line: Some("-x"),
            only: None,
            amend: false,
        };
        let refused = save_folder(&mut repo, &req, &mut |_| {}, &mut |_| true);
        assert!(
            matches!(&refused, Err(SaveError::Name(NameError::Bad { reason, .. }))
                if reason == "a line name cannot start with -"),
            "{refused:?}"
        );
        assert_eq!(repo.head().unwrap(), head);
    }

    #[test]
    fn an_amend_that_goes_no_further_keeps_its_line_s_list_and_no_other() {
        const KEY: [u8; 32] = [7; 32];
        let (dir, mut repo) = project();
        repo.set_cache_key(KEY);
        let first = repo.current().unwrap().1.state.lines["main"];
        write(dir.path(), "a.txt", b"a");
        let SaveOutcome::Saved { save_id, .. } = save_now(&mut repo).unwrap() else {
            panic!("nothing was saved");
        };
        let cached = |repo: &Repository, line: &str| ChangeIds::load(repo).get(line).is_some();
        // A line from the first save, which doesn't hold this one: nothing to amend.
        plant_line(&mut repo, "b", first);
        // Gone before each amend, so a list found after it is one the amend kept.
        let file = dir.path().join(".opalro/cache/change-ids");
        fs::remove_file(&file).unwrap();
        let same = save_as(&mut repo, None, true);
        assert!(
            matches!(same, Ok(SaveOutcome::NothingChanged { .. })),
            "{same:?}"
        );
        assert!(cached(&repo, "main"));
        assert!(!cached(&repo, "b"));
        // A line from this save: refused.
        plant_line(&mut repo, "c", save_id);
        fs::remove_file(&file).unwrap();
        let refused = save_as(&mut repo, None, true);
        let by_c = matches!(&refused, Err(SaveError::AmendShared { by }) if by == &["line c"]);
        assert!(by_c, "{refused:?}");
        assert!(cached(&repo, "main"));
        assert!(!cached(&repo, "c"));
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

    /// The file the walk saw, written over in place with as many bytes and the time the walk saw:
    /// only its change time tells.
    #[cfg(feature = "fault-injection")]
    #[test]
    fn a_file_written_over_in_place_after_the_walk_is_refused_by_its_change_time() {
        let (dir, mut repo) = project();
        let a = dir.path().join("a.txt");
        fs::write(&a, b"one").unwrap();
        let walked = fs::metadata(&a).unwrap().modified().unwrap();
        assert_changed_after_the_walk(&mut repo, move || {
            // Past the clock's tick, so the change time moves.
            std::thread::sleep(std::time::Duration::from_millis(50));
            fs::write(&a, b"two").unwrap();
            set_time(&a, walked);
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
            &Noted::default(),
        ) {
            Err(SaveError::ChangedDuringSave { path }) => assert_eq!(path, "victim.txt"),
            other => panic!("{other:?}"),
        }
        assert!(packs.is_empty());
    }

    #[test]
    fn a_link_swapped_for_a_file_is_changed_during_the_save() {
        let (dir, repo) = project();
        let (link, outside) = (dir.path().join("link"), tempfile::tempdir().unwrap());
        // A junction on Windows, which needs no right to make, and is removed as a folder.
        #[cfg(windows)]
        crate::test_support::junction(&link, outside.path());
        #[cfg(unix)]
        std::os::unix::fs::symlink(outside.path(), &link).unwrap();
        let Scan {
            mut handles,
            folders,
            ..
        } = walk_project(&repo, SaveLimits::default()).unwrap();
        #[cfg(windows)]
        fs::remove_dir(&link).unwrap();
        #[cfg(unix)]
        fs::remove_file(&link).unwrap();
        fs::write(&link, b"a file now").unwrap();
        let mut packs = repo.new_packs().unwrap();
        match build(
            repo.store(),
            &mut packs,
            &mut handles,
            &folders,
            &HashMap::new(),
            &Noted::default(),
        ) {
            Err(SaveError::ChangedDuringSave { path }) => assert_eq!(path, "link"),
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
        // `.opalroignore` is saved like any other file, so it can be swapped like one.
        let pipe = dir.path().join(".opalroignore");
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
            let name = ".opalroignore";
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

    /// Writes `rel`, then sets its time an hour back, so the file-state cache can vouch for it.
    fn write_old(root: &Path, rel: &str, bytes: &[u8]) {
        write(root, rel, bytes);
        let hour_ago = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
        set_time(&root.join(rel), hour_ago);
    }

    #[cfg(any(windows, feature = "fault-injection"))]
    #[test]
    fn an_unchanged_file_is_not_read_by_save() {
        let (dir, mut repo) = project();
        write_old(dir.path(), "kept.txt", b"kept");
        save_now(&mut repo).unwrap();
        #[cfg(windows)]
        let kept = dir.path().join("kept.txt");
        // Nothing can read it now, so a save that opened it would fail. Not by taking the right
        // to read it: that moves its change time, and the cache stops vouching for it.
        #[cfg(feature = "fault-injection")]
        crate::fsio::fault::unreadable(Some("kept.txt"));
        #[cfg(windows)]
        let _shut = {
            use std::os::windows::fs::OpenOptionsExt;
            let held = fs::File::options().read(true).share_mode(0).open(&kept);
            assert!(fs::read(&kept).is_err());
            held.unwrap()
        };
        write(dir.path(), "new.txt", b"new");
        let root = saved_root(save_now(&mut repo).unwrap());
        let tree = saved_tree(repo.store(), root);
        assert_eq!(tree["kept.txt"].1, b"kept");
        assert_eq!(tree["new.txt"].1, b"new");
        // And a save that must open it can't.
        #[cfg(windows)]
        drop(_shut);
        #[cfg(feature = "fault-injection")]
        {
            write(dir.path(), "kept.txt", b"changed");
            match save_now(&mut repo) {
                Err(SaveError::Unreadable { path, .. }) => assert_eq!(path, "kept.txt"),
                other => panic!("{:?}", other.map(|_| ())),
            }
            crate::fsio::fault::unreadable(None);
        }
    }

    #[test]
    fn a_file_the_cache_misses_is_read_and_saved() {
        let (dir, mut repo) = project();
        write_old(dir.path(), "a.txt", b"one");
        write_old(dir.path(), "b.txt", b"one");
        save_now(&mut repo).unwrap();
        // As many bytes, at another time; and more bytes, at the same time.
        let b = dir.path().join("b.txt");
        let then = fs::metadata(&b).unwrap().modified().unwrap();
        write_old(dir.path(), "a.txt", b"two");
        set_time(
            &dir.path().join("a.txt"),
            then - std::time::Duration::from_secs(60),
        );
        write(dir.path(), "b.txt", b"three");
        set_time(&b, then);
        let root = saved_root(save_now(&mut repo).unwrap());
        let tree = saved_tree(repo.store(), root);
        assert_eq!(tree["a.txt"].1, b"two");
        assert_eq!(tree["b.txt"].1, b"three");
    }

    #[test]
    fn an_edit_status_vouched_for_is_still_stored() {
        let (dir, mut repo) = project();
        write_old(dir.path(), "a.txt", b"one");
        save_now(&mut repo).unwrap();
        // Another content at another old time, and a new file, both learned by status.
        write_old(dir.path(), "a.txt", b"two!");
        let a = dir.path().join("a.txt");
        let then = fs::metadata(&a).unwrap().modified().unwrap();
        set_time(&a, then - std::time::Duration::from_secs(60));
        write_old(dir.path(), "b.txt", b"new");
        crate::status::status(&repo, false, None).unwrap();
        // The cache now vouches for both, but the base save holds neither, so both are stored.
        let root = saved_root(save_now(&mut repo).unwrap());
        let fresh = Repository::open(dir.path()).unwrap();
        let tree = saved_tree(fresh.store(), root);
        assert_eq!(tree["a.txt"].1, b"two!");
        assert_eq!(tree["b.txt"].1, b"new");
    }

    #[test]
    fn a_deleted_file_drops_out_of_the_file_cache() {
        let (dir, mut repo) = project();
        for name in ["a.txt", "b.txt", "c.txt"] {
            write_old(dir.path(), name, name.as_bytes());
        }
        save_now(&mut repo).unwrap();
        let top = Dir::open_top(dir.path()).unwrap();
        let meta = |name: &str| top.metadata(OsStr::new(name)).unwrap();
        let (a, b, c) = (meta("a.txt"), meta("b.txt"), meta("c.txt"));
        let cache = FileCache::load(&repo);
        assert!(cache.matches("b.txt", &b).is_some());
        assert!(cache.matches("c.txt", &c).is_some());
        fs::remove_file(dir.path().join("b.txt")).unwrap();
        save_now(&mut repo).unwrap();
        let cache = FileCache::load(&repo);
        assert!(cache.matches("a.txt", &a).is_some());
        assert!(cache.matches("b.txt", &b).is_none());
        // Status keeps only what it met too.
        fs::remove_file(dir.path().join("c.txt")).unwrap();
        crate::status::status(&repo, false, None).unwrap();
        let cache = FileCache::load(&repo);
        assert!(cache.matches("a.txt", &a).is_some());
        assert!(cache.matches("c.txt", &c).is_none());
    }

    #[test]
    fn a_status_whose_walk_fails_leaves_no_thread_and_no_changed_cache() {
        let (dir, mut repo) = project();
        write_old(dir.path(), "a.txt", b"a");
        save_now(&mut repo).unwrap();
        let stored = dir.path().join(".opalro/cache/files");
        let before = fs::read(&stored).unwrap();
        // A new file a finished scan would add to the cache, and an ignore list the walk refuses.
        write_old(dir.path(), "b.txt", b"b");
        let own = tempfile::tempdir().unwrap();
        let list = own.path().join("ignore");
        fs::write(
            &list,
            vec![b'#'; crate::safe_fs::MAX_IGNORE_FILE_BYTES as usize + 1],
        )
        .unwrap();
        let err = crate::status::status(&repo, false, Some(&list)).unwrap_err();
        assert!(matches!(err, SaveError::Ignore { .. }), "{err}");
        let err = save_with(&mut repo, Some(&list)).unwrap_err();
        assert!(matches!(err, SaveError::Ignore { .. }), "{err}");
        // The cache's helper was joined before the error came back; nothing wrote the cache.
        assert_eq!(fs::read(&stored).unwrap(), before);
    }

    #[test]
    fn a_save_whose_file_cache_cant_be_written_is_made_with_a_warning() {
        let (dir, mut repo) = project();
        fs::create_dir(dir.path().join(".opalro/cache/files")).unwrap();
        write(dir.path(), "a.txt", b"a");
        let SaveOutcome::Saved {
            save_id, warnings, ..
        } = save_now(&mut repo).unwrap()
        else {
            panic!("nothing was saved");
        };
        let kinds: Vec<&str> = warnings.iter().map(|w| w.kind).collect();
        assert_eq!(kinds, ["file_cache_not_stored"]);
        assert!(
            warnings[0]
                .message
                .starts_with("Opalro couldn't keep what it learned about your files"),
            "{warnings:?}"
        );
        assert_eq!(repo.current().unwrap().1.state.lines["main"], save_id);
        // And when nothing changed.
        let SaveOutcome::NothingChanged { warnings, .. } = save_now(&mut repo).unwrap() else {
            panic!("saved again");
        };
        assert_eq!(warnings[0].kind, "file_cache_not_stored");
    }

    fn path(p: &str) -> Given {
        Given::Path(p.into())
    }

    fn pattern(below: &str, glob: &str) -> Given {
        Given::Pattern {
            below: below.into(),
            glob: glob.into(),
        }
    }

    /// The bytes of each file in the line's newest save, by path; folders are left out.
    fn newest_files(repo: &Repository) -> BTreeMap<String, Vec<u8>> {
        newest_tree(repo)
            .into_iter()
            .filter(|(_, (kind, _))| *kind != EntryKind::Folder)
            .map(|(p, (_, bytes))| (p, bytes))
            .collect()
    }

    fn newest_tree(repo: &Repository) -> Tree {
        let tip = repo.current().unwrap().1.state.lines["main"];
        let root = repo.store().save(&tip, false).unwrap().root;
        saved_tree(repo.store(), root)
    }

    fn files(pairs: &[(&str, &str)]) -> BTreeMap<String, Vec<u8>> {
        pairs
            .iter()
            .map(|(p, b)| ((*p).to_owned(), b.as_bytes().to_vec()))
            .collect()
    }

    #[test]
    fn a_partial_save_takes_only_the_named_file() {
        let (dir, mut repo) = project();
        write(dir.path(), "a.txt", b"a1");
        write(dir.path(), "b.txt", b"b1");
        save_now(&mut repo).unwrap();
        write(dir.path(), "a.txt", b"a2");
        write(dir.path(), "b.txt", b"b2");
        write(dir.path(), "new.txt", b"n");
        save_only(&mut repo, &[path("a.txt")]).unwrap();
        assert_eq!(
            newest_files(&repo),
            files(&[("a.txt", "a2"), ("b.txt", "b1")])
        );
    }

    #[test]
    fn a_partial_save_leaves_the_rest_unsaved() {
        let (dir, mut repo) = project();
        write(dir.path(), "a.txt", b"a1");
        write(dir.path(), "b.txt", b"b1");
        save_now(&mut repo).unwrap();
        write(dir.path(), "a.txt", b"a2");
        write(dir.path(), "b.txt", b"b2");
        write(dir.path(), "new.txt", b"n");
        save_only(&mut repo, &[path("a.txt")]).unwrap();
        let now = crate::status::status(&repo, false, None).unwrap();
        let changed: Vec<&str> = now.changes.iter().map(|c| c.path.as_str()).collect();
        assert_eq!(changed, ["b.txt", "new.txt"]);
    }

    #[test]
    fn a_named_folder_takes_everything_in_it() {
        let (dir, mut repo) = project();
        write(dir.path(), "src/a.rs", b"a1");
        write(dir.path(), "src/gone.rs", b"g");
        write(dir.path(), "src/sub/b.rs", b"b1");
        write(dir.path(), "srcx.txt", b"x1");
        write(dir.path(), "top.txt", b"t1");
        save_now(&mut repo).unwrap();
        write(dir.path(), "src/a.rs", b"a2");
        fs::remove_file(dir.path().join("src/gone.rs")).unwrap();
        write(dir.path(), "src/sub/b.rs", b"b2");
        write(dir.path(), "src/sub/new.rs", b"n");
        write(dir.path(), "srcx.txt", b"x2");
        write(dir.path(), "top.txt", b"t2");
        save_only(&mut repo, &[path("src")]).unwrap();
        assert_eq!(
            newest_files(&repo),
            files(&[
                ("src/a.rs", "a2"),
                ("src/sub/b.rs", "b2"),
                ("src/sub/new.rs", "n"),
                ("srcx.txt", "x1"),
                ("top.txt", "t1"),
            ])
        );
    }

    #[test]
    fn a_pattern_takes_the_files_it_matches() {
        let (dir, mut repo) = project();
        for p in ["a.cs", "b.txt", "sub/c.cs", "sub/d.txt"] {
            write(dir.path(), p, b"1");
        }
        save_now(&mut repo).unwrap();
        for p in [
            "a.cs",
            "b.txt",
            "sub/c.cs",
            "sub/d.txt",
            "new/e.cs",
            "new/f.txt",
        ] {
            write(dir.path(), p, b"2");
        }
        save_only(&mut repo, &[pattern("", "*.cs")]).unwrap();
        let top_only = files(&[
            ("a.cs", "2"),
            ("b.txt", "1"),
            ("sub/c.cs", "1"),
            ("sub/d.txt", "1"),
        ]);
        assert_eq!(newest_files(&repo), top_only);
        save_only(&mut repo, &[pattern("", "**/*.cs")]).unwrap();
        let every = files(&[
            ("a.cs", "2"),
            ("b.txt", "1"),
            ("new/e.cs", "2"),
            ("sub/c.cs", "2"),
            ("sub/d.txt", "1"),
        ]);
        assert_eq!(newest_files(&repo), every);
        // A folder a pattern matches means everything in it.
        save_only(&mut repo, &[pattern("", "s*")]).unwrap();
        let tree = newest_tree(&repo);
        assert_eq!(tree["sub/d.txt"].1, b"2");
        assert!(!tree.contains_key("new/f.txt"), "{tree:?}");
    }

    #[test]
    fn a_named_deleted_file_is_saved_as_deleted() {
        let (dir, mut repo) = project();
        for p in ["a.txt", "b.txt", "dir/x.txt", "dir/y.txt", "old/z.txt"] {
            write(dir.path(), p, b"1");
        }
        save_now(&mut repo).unwrap();
        fs::remove_file(dir.path().join("a.txt")).unwrap();
        fs::remove_file(dir.path().join("b.txt")).unwrap();
        fs::remove_dir_all(dir.path().join("dir")).unwrap();
        fs::remove_dir_all(dir.path().join("old")).unwrap();
        save_only(&mut repo, &[path("a.txt"), path("dir/x.txt")]).unwrap();
        assert_eq!(
            newest_files(&repo),
            files(&[("b.txt", "1"), ("dir/y.txt", "1"), ("old/z.txt", "1")])
        );
        // The last file of a folder gone from disk takes the folder with it.
        save_only(&mut repo, &[path("dir/y.txt")]).unwrap();
        let tree = newest_tree(&repo);
        assert!(!tree.contains_key("dir"), "{tree:?}");
        // A pattern takes deleted files too, in folders gone from disk.
        save_only(&mut repo, &[pattern("", "**/*.txt")]).unwrap();
        assert_eq!(newest_files(&repo), files(&[]));
    }

    #[test]
    fn an_argument_matching_nothing_is_refused() {
        let (dir, mut repo) = project();
        write(dir.path(), "a.txt", b"1");
        save_now(&mut repo).unwrap();
        write(dir.path(), "a.txt", b"2");
        let before = repo.current().unwrap().1.state.lines["main"];
        for given in [
            path("nope.txt"),
            pattern("", "*.zip"),
            pattern("a.txt", "*"),
        ] {
            let only = [path("a.txt"), given.clone()];
            match save_only(&mut repo, &only) {
                Err(SaveError::NoMatch { given: shown }) => assert_eq!(shown, given.shown()),
                other => panic!("{other:?}"),
            }
        }
        assert_eq!(repo.current().unwrap().1.state.lines["main"], before);
        // A file only the last save holds is a match: its deletion is saved.
        fs::remove_file(dir.path().join("a.txt")).unwrap();
        save_only(&mut repo, &[path("a.txt")]).unwrap();
        assert_eq!(newest_files(&repo), files(&[]));
    }

    #[test]
    fn an_ignored_new_file_named_outright_is_refused() {
        let (dir, mut repo) = project();
        write(dir.path(), ".opalroignore", b"*.log\nbuild/\n");
        write(dir.path(), "a.txt", b"1");
        save_now(&mut repo).unwrap();
        write(dir.path(), "a.txt", b"2");
        write(dir.path(), "x.log", b"log");
        write(dir.path(), "build/out.o", b"o");
        for named in ["x.log", "build", "build/out.o"] {
            match save_only(&mut repo, &[path(named)]) {
                Err(e @ SaveError::LeftOut { .. }) => {
                    assert!(e.to_string().contains(named), "{e}");
                    let hint = e.hint().unwrap();
                    assert!(hint.contains("change the rule"), "{hint}");
                }
                other => panic!("{named}: {other:?}"),
            }
        }
        // A pattern just leaves it out.
        save_only(&mut repo, &[pattern("", "*")]).unwrap();
        assert_eq!(
            newest_files(&repo),
            files(&[(".opalroignore", "*.log\nbuild/\n"), ("a.txt", "2")])
        );
    }

    #[test]
    fn the_file_cache_after_a_partial_save_vouches_for_no_unsaved_change() {
        let (dir, mut repo) = project();
        write_old(dir.path(), "a.txt", b"one");
        write_old(dir.path(), "b.txt", b"one");
        save_now(&mut repo).unwrap();
        // Changed at old times, so status trusts what it reads and the cache learns both.
        let b = dir.path().join("b.txt");
        let then = fs::metadata(&b).unwrap().modified().unwrap();
        let earlier = then - std::time::Duration::from_secs(60);
        write_old(dir.path(), "a.txt", b"two!");
        write_old(dir.path(), "b.txt", b"two!");
        set_time(&dir.path().join("a.txt"), earlier);
        set_time(&b, earlier);
        crate::status::status(&repo, false, None).unwrap();
        save_only(&mut repo, &[path("a.txt")]).unwrap();
        let top = Dir::open_top(dir.path()).unwrap();
        let b_now = top.metadata(OsStr::new("b.txt")).unwrap();
        assert!(FileCache::load(&repo).matches("b.txt", &b_now).is_none());
        // A full save then stores b's change, which a fresh look at the store reads back.
        save_now(&mut repo).unwrap();
        let fresh = Repository::open(dir.path()).unwrap();
        assert_eq!(
            newest_files(&fresh),
            files(&[("a.txt", "two!"), ("b.txt", "two!")])
        );
    }

    #[test]
    fn a_pattern_keeps_an_empty_folder_gone_from_disk_that_it_doesnt_match() {
        let (dir, mut repo) = project();
        write(dir.path(), "a.txt", b"1");
        fs::create_dir(dir.path().join("old")).unwrap();
        save_now(&mut repo).unwrap();
        assert!(newest_tree(&repo).contains_key("old"));
        fs::remove_dir(dir.path().join("old")).unwrap();
        write(dir.path(), "a.txt", b"2");
        save_only(&mut repo, &[pattern("", "**/*.txt")]).unwrap();
        let tree = newest_tree(&repo);
        assert_eq!(tree["a.txt"].1, b"2");
        assert!(tree.contains_key("old"), "{tree:?}");
        // Named, it goes.
        save_only(&mut repo, &[path("old")]).unwrap();
        assert!(!newest_tree(&repo).contains_key("old"));
    }

    /// The refusal a partial save of `only` gives, which must save nothing.
    fn refused(repo: &mut Repository, only: &[Given]) -> SaveError {
        let before = repo.current().unwrap().1.state.lines["main"];
        let err = save_only(repo, only).unwrap_err();
        assert_eq!(repo.current().unwrap().1.state.lines["main"], before);
        err
    }

    #[test]
    fn naming_inside_what_isnt_a_folder_says_so() {
        let (dir, mut repo) = project();
        write(dir.path(), "x/a.txt", b"1");
        save_now(&mut repo).unwrap();
        fs::remove_dir_all(dir.path().join("x")).unwrap();
        write(dir.path(), "x", b"now a file");
        let err = refused(&mut repo, &[path("x/a.txt")]);
        assert!(
            matches!(&err, SaveError::NotAFolder { path, what: "a file" } if path == "x"),
            "{err:?}"
        );
        assert_eq!(
            err.to_string(),
            "Opalro can't save part of x: it is a file, not a folder."
        );
    }

    #[test]
    fn naming_inside_an_entry_kept_as_it_was_says_so() {
        const COMMIT: &[u8] = &[0x5a; 20];
        let (dir, mut repo) = project();
        plant_save(&mut repo, &[("sub", EntryKind::ProjectLink, COMMIT)]);
        write(dir.path(), "sub/a.txt", b"x");
        let err = refused(&mut repo, &[path("sub/a.txt")]);
        assert!(
            matches!(&err, SaveError::KeptWhole { path } if path == "sub"),
            "{err:?}"
        );
    }

    #[test]
    fn naming_what_saves_never_hold_says_why() {
        let (dir, mut repo) = project();
        write(dir.path(), "a.txt", b"1");
        save_now(&mut repo).unwrap();
        write(dir.path(), ".git/config", b"[core]");
        write(dir.path(), "nested/.git/HEAD", b"ref: refs/heads/main");
        write(dir.path(), "nested/a.txt", b"x");
        write(dir.path(), "inner/.opalro/format", b"x");
        write(dir.path(), "inner/b.txt", b"x");
        let err = refused(&mut repo, &[path(".git/config")]);
        assert!(
            matches!(&err, SaveError::NeverSaved { path } if path == ".git"),
            "{err:?}"
        );
        for (named, at) in [
            ("nested/a.txt", "nested"),
            ("nested", "nested"),
            ("inner/b.txt", "inner"),
        ] {
            let err = refused(&mut repo, &[path(named)]);
            assert!(
                matches!(&err, SaveError::NestedProject { path, at: a } if path == named && a == at),
                "{named}: {err:?}"
            );
        }
    }

    /// A save whose question about adding much is answered `answer`, and what it asked, if it
    /// asked; of only `only` when given.
    fn save_answering(
        repo: &mut Repository,
        only: Option<&[Given]>,
        answer: bool,
    ) -> (Result<SaveOutcome, SaveError>, Option<LargeSave>) {
        let who = who();
        let req = SaveRequest {
            message: Some("Add much"),
            who: &who,
            command: "save",
            accept_crlf: false,
            git_excludes: None,
            new_line: None,
            only,
            amend: false,
        };
        let mut asked = None;
        let result = save_folder(repo, &req, &mut |_| {}, &mut |large| {
            asked = Some(large.clone());
            answer
        });
        (result, asked)
    }

    /// A file of `size` zeros, made without writing them.
    fn sized(root: &Path, rel: &str, size: u64) {
        write(root, rel, b"");
        File::options()
            .write(true)
            .open(root.join(rel))
            .unwrap()
            .set_len(size)
            .unwrap();
    }

    #[test]
    fn a_save_adding_more_than_10000_files_asks() {
        let (dir, mut repo) = project();
        write(dir.path(), "a.txt", b"1");
        save_now(&mut repo).unwrap();
        for i in 0..10_001 {
            write(dir.path(), &format!("many/{}/{i}.txt", i % 10), b"x");
        }
        let (result, asked) = save_answering(&mut repo, None, false);
        let large = asked.expect("asked");
        assert_eq!((large.new_files, large.new_bytes), (10_001, 10_001));
        assert_eq!(large.biggest_folders, [("many".to_owned(), 10_001)]);
        assert!(
            matches!(result, Err(SaveError::LargeSave(l)) if l == large),
            "nothing was saved"
        );
    }

    #[test]
    fn a_save_adding_more_than_100_mb_asks() {
        let (dir, mut repo) = project();
        write(dir.path(), "a.txt", b"1");
        save_now(&mut repo).unwrap();
        sized(dir.path(), "art/huge.bin", 100_000_001);
        write(dir.path(), "docs/b.txt", b"12345");
        write(dir.path(), "src/c.txt", b"123");
        write(dir.path(), "top.txt", b"1234");
        let (result, asked) = save_answering(&mut repo, None, false);
        let large = asked.expect("asked");
        assert_eq!((large.new_files, large.new_bytes), (4, 100_000_013));
        let biggest = [("art", 100_000_001), ("docs", 5), ("", 4)];
        assert_eq!(
            large.biggest_folders,
            biggest.map(|(f, n)| (f.to_owned(), n))
        );
        let err = result.unwrap_err();
        assert_eq!(
            err.to_string(),
            "This save adds 95 MiB in 4 new files, most in: art/ (95 MiB), docs/ (5 B), the top folder (4 B)."
        );
        assert!(err.hint().unwrap().contains("--yes"), "{err:?}");
        // A partial save counts only what it saves.
        let (result, asked) = save_answering(&mut repo, Some(&[path("docs")]), false);
        assert_eq!(asked, None);
        result.unwrap();
    }

    #[test]
    fn answering_no_stores_nothing() {
        let (dir, mut repo) = project();
        write(dir.path(), "a.txt", b"1");
        save_now(&mut repo).unwrap();
        // Head, diary, packs, `tmp/` and the file cache: all of `.opalro`, byte for byte, but for
        // the lock's note of who last held it.
        let opalro = || {
            let mut tree = disk_tree(&dir.path().join(".opalro"));
            tree.remove("state/lock-info");
            tree
        };
        let before = opalro();
        sized(dir.path(), "art/huge.bin", 100_000_001);
        let (result, asked) = save_answering(&mut repo, None, false);
        assert!(asked.is_some());
        assert!(matches!(result, Err(SaveError::LargeSave(_))), "{result:?}");
        assert!(opalro() == before, "something was stored");
    }

    #[test]
    fn a_folder_renamed_in_letter_case_only_is_not_new_where_the_drive_ignores_case() {
        let (dir, mut repo) = project();
        sized(dir.path(), "Art/Huge.bin", 100_000_001);
        save_now(&mut repo).unwrap();
        fs::rename(dir.path().join("Art"), dir.path().join("art")).unwrap();
        fs::rename(
            dir.path().join("art/Huge.bin"),
            dir.path().join("art/huge.bin"),
        )
        .unwrap();
        let (_, asked) = save_answering(&mut repo, None, false);
        assert_eq!(asked.is_some(), !this_drive().ignores_case, "{asked:?}");
    }

    #[test]
    fn answering_yes_saves() {
        let (dir, mut repo) = project();
        write(dir.path(), "a.txt", b"1");
        save_now(&mut repo).unwrap();
        sized(dir.path(), "art/huge.bin", 100_000_001);
        let (result, asked) = save_answering(&mut repo, None, true);
        assert!(asked.is_some());
        let tree = saved_tree(repo.store(), saved_root(result.unwrap()));
        assert_eq!(tree["art/huge.bin"].1.len(), 100_000_001);
    }

    #[test]
    fn changed_files_dont_count() {
        let (dir, mut repo) = project();
        write(dir.path(), "art/huge.bin", b"1");
        save_now(&mut repo).unwrap();
        sized(dir.path(), "art/huge.bin", 100_000_001);
        write(dir.path(), "b.txt", b"new");
        let (result, asked) = save_answering(&mut repo, None, false);
        assert_eq!(asked, None);
        result.unwrap();
    }

    /// Small files in a few folders, some of them alike, two big files, an empty one and, where
    /// links can be made, a link.
    fn lay_out_many(root: &Path) {
        for i in 0..150u64 {
            let bytes = match i % 3 {
                0 => format!("file {i}\n").repeat(i as usize + 1).into_bytes(),
                1 => noise(i, 100 + (i as usize * 97) % 5000),
                _ => b"the same in many folders\n".to_vec(),
            };
            write(root, &format!("d{}/e{}/f{i:03}.txt", i % 4, i % 2), &bytes);
        }
        write(root, "d1/big.bin", &noise(1000, 3 << 20));
        write(root, "d2/e0/big.bin", &noise(1001, (1 << 20) + 12_345));
        write(root, "empty.txt", b"");
        #[cfg(unix)]
        std::os::unix::fs::symlink("e1/f003.txt", root.join("d3/link")).unwrap();
    }

    /// What `build` makes of a new project `lay_out` fills, with `count` workers and `after_walk`
    /// run between the walk and the reading: the top listing, or the first problem, and the packs
    /// it wrote, in sets of 7 objects so that the set rolls over many times.
    fn built_with(
        count: usize,
        lay_out: impl Fn(&Path),
        after_walk: impl Fn(&Path),
    ) -> (Result<Id, String>, Written) {
        let (dir, repo) = project();
        lay_out(dir.path());
        let Scan {
            mut handles,
            folders,
            old,
            noted,
            ..
        } = walk_project(&repo, SaveLimits::default()).unwrap();
        after_walk(dir.path());
        let tmp = dir.path().join(".opalro/tmp");
        let mut packs = PackSet::with_limit(&tmp, repo.packs_dir(), 7).unwrap();
        crate::workers::set_test_workers(Some(count));
        let built = build(
            repo.store(),
            &mut packs,
            &mut handles,
            &folders,
            &old,
            &noted,
        );
        crate::workers::set_test_workers(None);
        let built = built.map(|(root, ..)| root).map_err(|e| format!("{e:?}"));
        (built, written(packs))
    }

    #[test]
    fn a_save_writes_the_same_packs_with_one_thread_or_eight() {
        let one = built_with(1, lay_out_many, |_| {});
        let eight = built_with(8, lay_out_many, |_| {});
        assert!(one.0.is_ok(), "{:?}", one.0);
        assert!(one.1.len() > 20, "{} packs", one.1.len());
        assert_eq!(one.0, eight.0);
        assert_eq!(names(&one.1), names(&eight.1));
        assert!(one.1 == eight.1, "the packs hold other bytes");
    }

    #[test]
    fn the_first_problem_is_the_same_with_one_thread_or_eight() {
        // Gone after the walk. The first a save reads is the last file of the first folder it
        // reads, so with eight workers a later one may well fail first.
        let gone = |root: &Path| {
            for rel in [
                "d0/e0/f148.txt",
                "d1/e1/f001.txt",
                "d3/e1/f003.txt",
                "d1/big.bin",
            ] {
                fs::remove_file(root.join(rel)).unwrap();
            }
        };
        let one = built_with(1, lay_out_many, gone);
        let eight = built_with(8, lay_out_many, gone);
        let first = one.0.as_ref().unwrap_err();
        assert!(
            first.contains("Unreadable") && first.contains("\"d0/e0/f148.txt\""),
            "{first}"
        );
        assert_eq!(one.0, eight.0);
        // And what was stored before it, the same.
        assert_eq!(names(&one.1), names(&eight.1));
        assert!(one.1 == eight.1, "the packs hold other bytes");
    }

    /// Many files changed at once, read on eight workers: the hook this thread sets reaches them.
    #[cfg(feature = "fault-injection")]
    #[test]
    fn an_unchanged_file_is_still_not_read_by_save() {
        let (dir, mut repo) = project();
        for i in 0..40 {
            write_old(dir.path(), &format!("f{i:02}.txt"), b"old");
        }
        save_now(&mut repo).unwrap();
        crate::workers::set_test_workers(Some(8));
        crate::fsio::fault::unreadable(Some("f20.txt"));
        for i in (0..40).filter(|&i| i != 20) {
            write(
                dir.path(),
                &format!("f{i:02}.txt"),
                format!("new {i}").as_bytes(),
            );
        }
        let saved = save_now(&mut repo).map(saved_root);
        // Changed, it must be read, and can't be.
        write(dir.path(), "f20.txt", b"changed");
        let refused = save_now(&mut repo);
        crate::fsio::fault::unreadable(None);
        crate::workers::set_test_workers(None);
        let tree = saved_tree(repo.store(), saved.unwrap());
        assert_eq!(tree["f20.txt"].1, b"old");
        assert_eq!(tree["f21.txt"].1, b"new 21");
        match refused {
            Err(SaveError::Unreadable { path, .. }) => assert_eq!(path, "f20.txt"),
            other => panic!("{:?}", other.map(|_| ())),
        }
    }

    #[test]
    fn status_hashes_many_files_on_several_threads() {
        let (dir, mut repo) = project();
        lay_out_many(dir.path());
        let (_, entry) = repo.current().unwrap();
        let newest = entry.state.lines["main"];
        let base = repo.store().save(&newest, false).unwrap().root;
        let mut seen = Vec::new();
        for count in [1, 8] {
            crate::workers::set_test_workers(Some(count));
            // Without the cache, so every file is read.
            let now = read_folder(&repo, base, true, None, None);
            crate::workers::set_test_workers(None);
            let now = now.unwrap();
            let mut listings: Vec<Id> = now.listings.keys().copied().collect();
            listings.sort_by_key(|id| id.0);
            seen.push((now.root, listings));
        }
        assert_eq!(seen[0], seen[1]);
        // The same answer a save gives.
        assert_eq!(saved_root(save_now(&mut repo).unwrap()), seen[0].0);
    }

    /// The ID of the folder `path` in the saved tree whose top listing is `root`.
    fn saved_folder(store: &Store, root: Id, path: &str) -> Id {
        path.split('/')
            .filter(|n| !n.is_empty())
            .fold(root, |id, name| {
                store.listing(&id).unwrap().get(name).unwrap().id
            })
    }

    /// The ID of the listing of the folder at `dir`, built here from what it holds on disk.
    fn listing_on_disk(dir: &Path) -> Id {
        let mut entries = Vec::new();
        for e in fs::read_dir(dir).unwrap() {
            let e = e.unwrap();
            let name = e.file_name().into_string().unwrap();
            if name == ".opalro" {
                continue;
            }
            let is = e.file_type().unwrap();
            let (kind, id, size) = if is.is_dir() {
                (EntryKind::Folder, listing_on_disk(&e.path()), 0)
            } else if is.is_symlink() {
                let target = fs::read_link(e.path()).unwrap();
                let target = target.to_str().unwrap().as_bytes().to_vec();
                (
                    EntryKind::Symlink,
                    hash_content(&target),
                    target.len() as u64,
                )
            } else {
                let bytes = fs::read(e.path()).unwrap();
                (
                    file_kind(&e.path()),
                    hash_content(&bytes),
                    bytes.len() as u64,
                )
            };
            entries.push(Entry {
                name,
                kind,
                id,
                size,
            });
        }
        Listing::new(entries).unwrap().id()
    }

    /// A file's kind as a save takes it, for a file the last save didn't hold as executable.
    fn file_kind(path: &Path) -> EntryKind {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if fs::metadata(path).unwrap().permissions().mode() & 0o111 != 0 {
                return EntryKind::Executable;
            }
        }
        let _ = path;
        EntryKind::File
    }

    /// A project of three folders below the top one, saved.
    fn three_folders() -> (tempfile::TempDir, Repository) {
        let (dir, mut repo) = project();
        let root = dir.path();
        write(root, "top.txt", b"top");
        write(root, "a/one.txt", b"one");
        write(root, "a/b/two.txt", b"two");
        write(root, "a/b/three.txt", b"three");
        write(root, "c/four.txt", b"four");
        save_now(&mut repo).unwrap();
        (dir, repo)
    }

    fn newest_root(repo: &Repository) -> Id {
        let (_, entry) = repo.current().unwrap();
        let id = entry.state.lines["main"];
        repo.store().save(&id, false).unwrap().root
    }

    #[test]
    fn an_unchanged_folders_listing_is_the_one_a_build_would_make() {
        let (dir, mut repo) = three_folders();
        let root = dir.path();
        write(root, "a/one.txt", b"one, changed");
        let id = saved_root(save_now(&mut repo).unwrap());
        for unchanged in ["a/b", "c"] {
            assert_eq!(
                saved_folder(repo.store(), id, unchanged),
                listing_on_disk(&root.join(unchanged)),
                "{unchanged}"
            );
        }
        assert_eq!(id, listing_on_disk(root));
    }

    /// Clears the record of the listings built on this thread.
    fn build_count_from_here() {
        LISTINGS_BUILT.with_borrow_mut(Vec::clear);
    }

    /// The folders whose listings were built on this thread since `build_count_from_here`.
    fn built() -> Vec<String> {
        LISTINGS_BUILT.with_borrow(Clone::clone)
    }

    #[test]
    fn an_unchanged_folder_is_not_rebuilt() {
        let (dir, repo) = three_folders();
        let base = newest_root(&repo);
        let status = |repo: &Repository| {
            build_count_from_here();
            let now = read_folder(repo, base, false, None, None).unwrap();
            (built(), now.root)
        };
        assert_eq!(status(&repo), (Vec::<String>::new(), base));
        // Only the folders on the changed file's path; `c` beside them is not.
        write(dir.path(), "a/b/two.txt", b"two, changed");
        let (rebuilt, root) = status(&repo);
        assert_eq!(rebuilt, ["a/b", "a", ""]);
        assert_eq!(root, listing_on_disk(dir.path()));
        // `a/b`, unchanged inside its changed folder, is not either.
        write(dir.path(), "a/b/two.txt", b"two");
        write(dir.path(), "a/one.txt", b"one, changed");
        let (rebuilt, root) = status(&repo);
        assert_eq!(rebuilt, ["a", ""]);
        assert_eq!(root, listing_on_disk(dir.path()));
    }

    /// Saves a project of a folder `d` and one beside it, `e`; makes `change` in `d`, if it can;
    /// and saves again. `d`'s new ID must be the one a build of what it now holds gives, and not
    /// the one it had; only `d` and the top folder are built.
    fn assert_one_change_is_seen(change: impl FnOnce(&Path) -> bool) {
        let (dir, mut repo) = project();
        let root = dir.path();
        write(root, "d/a.txt", b"aaa");
        write(root, "d/b.txt", b"bbb");
        write(root, "e/c.txt", b"ccc");
        let first = saved_root(save_now(&mut repo).unwrap());
        let before = saved_folder(repo.store(), first, "d");
        if !change(&root.join("d")) {
            return;
        }
        build_count_from_here();
        let second = saved_root(save_now(&mut repo).unwrap());
        assert_eq!(built(), ["d", ""]);
        let after = saved_folder(repo.store(), second, "d");
        assert_eq!(after, listing_on_disk(&root.join("d")));
        assert_ne!(after, before);
        assert_eq!(second, listing_on_disk(root));
    }

    #[test]
    fn a_change_of_one_field_alone_is_seen() {
        // The bytes, but not the size.
        assert_one_change_is_seen(|d| {
            fs::write(d.join("a.txt"), b"xyz").unwrap();
            true
        });
        // The size.
        assert_one_change_is_seen(|d| {
            fs::write(d.join("a.txt"), b"aaaa").unwrap();
            true
        });
        // The name's letter case, the bytes kept, whether the drive folds case or not.
        assert_one_change_is_seen(|d| {
            fs::rename(d.join("a.txt"), d.join("A.txt")).unwrap();
            true
        });
        // One file gone and another come, so the count stays.
        assert_one_change_is_seen(|d| {
            fs::remove_file(d.join("b.txt")).unwrap();
            fs::write(d.join("c.txt"), b"bbb").unwrap();
            true
        });
    }

    #[cfg(unix)]
    #[test]
    fn a_change_of_kind_alone_is_seen() {
        use std::os::unix::fs::PermissionsExt;
        // The executable bit: the kind changes, the ID and size don't.
        assert_one_change_is_seen(|d| {
            let to = fs::Permissions::from_mode(0o755);
            fs::set_permissions(d.join("a.txt"), to).unwrap();
            true
        });
        // A file replaced by a link.
        assert_one_change_is_seen(|d| {
            fs::remove_file(d.join("a.txt")).unwrap();
            crate::test_support::symlink(Path::new("b.txt"), &d.join("a.txt"))
        });
    }

    #[test]
    fn a_folder_with_entries_carried_over_gets_the_id_a_build_would_give() {
        let (dir, mut repo) = project();
        let root = dir.path();
        let top = plant_save(
            &mut repo,
            &[
                ("top.txt", EntryKind::File, b"top"),
                ("d/in.txt", EntryKind::File, b"in"),
                ("d/left.txt", EntryKind::File, b"left"),
                ("d/sub", EntryKind::ProjectLink, &[0x5a; 20]),
            ],
        );
        write(root, "top.txt", b"top");
        write(root, "d/in.txt", b"in");
        write(root, "d/left.txt", b"left");
        let d_before = top.get("d").unwrap().id;
        let d_saved = repo.store().listing(&d_before).unwrap();
        assert_eq!(
            Listing::new(d_saved.entries().to_vec()).unwrap().id(),
            d_before
        );
        // `d` holds `sub`, kept as it was, and, as this save leaves it, `left.txt`.
        write(root, "top.txt", b"top, changed");
        build_count_from_here();
        let only = [path("top.txt"), path("d/in.txt")];
        let id = saved_root(save_only(&mut repo, &only).unwrap());
        assert_eq!(built(), [""]);
        assert_eq!(saved_folder(repo.store(), id, "d"), d_before);
        // Both of its files changed; this save takes only `in.txt`.
        write(root, "d/in.txt", b"in, changed");
        write(root, "d/left.txt", b"left, changed");
        let id = saved_root(save_only(&mut repo, &[path("d/in.txt")]).unwrap());
        let with = |name: &str, bytes: &[u8], from: &Listing| {
            let mut entries = from.entries().to_vec();
            let e = entries.iter_mut().find(|e| e.name == name).unwrap();
            (e.id, e.size) = (hash_content(bytes), bytes.len() as u64);
            Listing::new(entries).unwrap()
        };
        let want = with("in.txt", b"in, changed", &d_saved);
        assert_eq!(saved_folder(repo.store(), id, "d"), want.id());
        assert_ne!(want.id(), d_before);
        // A whole save then takes `left.txt` too, `sub` still kept.
        let id = saved_root(save_now(&mut repo).unwrap());
        let want = with("left.txt", b"left, changed", &want);
        assert_eq!(saved_folder(repo.store(), id, "d"), want.id());
        // Gone from disk, `d` comes back holding only `sub`, still kept.
        fs::remove_dir_all(root.join("d")).unwrap();
        let id = saved_root(save_now(&mut repo).unwrap());
        let sub: Vec<Entry> = d_saved
            .entries()
            .iter()
            .filter(|e| e.name == "sub")
            .cloned()
            .collect();
        let back = Listing::new(sub).unwrap().id();
        assert_eq!(saved_folder(repo.store(), id, "d"), back);
        // Asked again with nothing changed, neither it nor the top folder is built.
        build_count_from_here();
        let now = read_folder(&repo, newest_root(&repo), false, None, None).unwrap();
        assert_eq!((built(), now.root), (Vec::<String>::new(), id));
    }

    #[test]
    fn an_unchanged_folder_copies_no_name() {
        let (dir, repo) = three_folders();
        let base = newest_root(&repo);
        let status = |repo: &Repository| {
            ENTRIES_BUILT.set(0);
            let now = read_folder(repo, base, false, None, None).unwrap();
            (ENTRIES_BUILT.get(), now.root)
        };
        assert_eq!(status(&repo), (0, base));
        // `a/b` holds two, `a` two and the top folder three; `c` beside them none.
        write(dir.path(), "a/b/two.txt", b"two, changed");
        let (entries, root) = status(&repo);
        assert_eq!(entries, 7);
        assert_eq!(root, listing_on_disk(dir.path()));
    }

    #[test]
    fn the_comparison_after_a_read_finds_the_base_listings_in_memory() {
        let (dir, repo) = three_folders();
        let base = newest_root(&repo);
        write(dir.path(), "a/b/two.txt", b"two, changed");
        let now = read_folder(&repo, base, false, None, None).unwrap();
        let before = crate::pack::open_files::reads();
        let mut changed = Vec::new();
        crate::diff::for_each_change_in(repo.store(), &now.listings, Some(base), now.root, |c| {
            changed.push(c.path.to_owned());
            Ok::<(), SaveError>(())
        })
        .unwrap();
        assert_eq!(changed, ["a/b/two.txt"]);
        // The walk read the last save's listings; the comparison finds them in memory.
        assert_eq!(crate::pack::open_files::reads() - before, 0);
    }
}
