//! The walk of the project folder for `bolt save`: what the ignore rules let in, what the last
//! save holds along the way, and the entries a save keeps as they were, in folders gone from disk
//! too.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::fs::{self, Metadata};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;

use ignore::WalkBuilder;

use super::{Child, Folders, Kind, NAMED, SaveError, listed};
use crate::drive::Drive;
use crate::id::Id;
use crate::ignore::{Decision, Matcher, Rules};
use crate::listing::{Entry, EntryKind, Listing, MAX_SAVE_DEPTH, SaveLimits};
use crate::names::{holdable_here, is_git_alias, validate_name};
use crate::repo::RepoError;
use crate::safe_fs;
use crate::store::{Store, StoreError};
use crate::text::{escape, escape_all};
use crate::warning::{self, Warning};

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

/// Whether the folder `dir` holds a Git repository or Bolt project of its own: an entry named
/// `.git` of any kind (a worktree's or submodule's is a file), or a `.bolt` folder. Where the
/// drive ignores letter case, `.GIT` is found as `.git` (decision 046).
fn holds_own_repository(dir: &Path) -> bool {
    fs::symlink_metadata(dir.join(".git")).is_ok()
        || fs::symlink_metadata(dir.join(".bolt")).is_ok_and(|m| m.is_dir())
}

/// Whether `root` holds a real `.git` folder with a real `info` folder in it, where Git keeps its
/// personal list for the repository. Nothing in `.git` is read through a link.
fn has_real_git_info(root: &Path) -> bool {
    let git = root.join(".git");
    safe_fs::check_real_dir(&git).is_ok() && safe_fs::check_real_dir(&git.join("info")).is_ok()
}

/// The project path of `name` in the folder `rel` (`""` for the top folder).
fn path_in(rel: &str, name: &str) -> String {
    if rel.is_empty() {
        name.to_owned()
    } else {
        format!("{rel}/{name}")
    }
}

/// Which entries of `listing`, a folder of the last save, every save keeps as they were because
/// this computer can't hold them as they are: project links, names this system can't hold, and
/// names that are one file with another on this drive (decisions 036 and 039).
fn kept_as_they_were(listing: &Listing, drive: Drive) -> Vec<bool> {
    let entries = listing.entries();
    let mut kept: Vec<bool> = entries
        .iter()
        .map(|e| e.kind == EntryKind::ProjectLink || !holdable_here(&e.name))
        .collect();
    // A listing's names all differ, so only a drive that folds them can make two one file.
    if drive.ignores_case || drive.composes_unicode {
        let mut first: HashMap<String, usize> = HashMap::new();
        for (i, e) in entries.iter().enumerate() {
            let folded = drive.compared(&e.name);
            if let Some(&j) = first.get(&folded) {
                kept[i] = true;
                kept[j] = true;
            } else {
                first.insert(folded, i);
            }
        }
    }
    kept
}

/// A project path as Git matches it: the bytes of its names, with `/` between them.
fn path_bytes(rel: &Path) -> Vec<u8> {
    let names: Vec<&[u8]> = rel.iter().map(|c| c.as_encoded_bytes()).collect();
    names.join(&b'/')
}

/// The save a new one builds on. A walk reads its listings along the folders it keeps, which its
/// own depth limit holds, and below the folders gone from disk, which `limits` holds. They come
/// without the names Git treats as `.git`, which no save keeps, even from the last one, and each
/// read gives the names it dropped.
pub(super) struct Base<'a> {
    store: &'a Store,
    root: Id,
    /// The listings both read count against these, a shared one once per path.
    limits: SaveLimits,
    /// Whether the search of gone folders has passed `limits`, and so reads nothing more.
    spent: bool,
}

impl<'a> Base<'a> {
    /// The save whose top listing is `root`, read from `store` within `limits`.
    pub(super) fn new(store: &'a Store, root: Id, limits: SaveLimits) -> Base<'a> {
        Base {
            store,
            root,
            limits,
            spent: false,
        }
    }

    fn listing(&mut self, id: Id) -> Result<(Listing, Vec<String>), SaveError> {
        let listing = self.store.listing(&id)?;
        self.limits
            .add(listing.entries().len())
            .map_err(|detail| StoreError::SaveTooLarge { detail })?;
        without_git_names(listing)
    }

    /// The listing `id` of a folder `depth` levels down that is gone from disk, or `None` from
    /// the first time a read would pass the limits on. Past them the base save is invalid and
    /// nothing can be kept, so the whole search ends there, reading nothing more, and the save
    /// goes ahead, as one beside a save too large to compare does.
    fn gone_listing(
        &mut self,
        id: Id,
        depth: usize,
    ) -> Result<Option<(Listing, Vec<String>)>, SaveError> {
        if self.spent || self.limits.check_depth(depth).is_err() {
            self.spent = true;
            return Ok(None);
        }
        let listing = self.store.listing(&id)?;
        if self.limits.add(listing.entries().len()).is_err() {
            self.spent = true;
            return Ok(None);
        }
        without_git_names(listing).map(Some)
    }
}

/// `listing` without its names Git treats as `.git`, and those names.
fn without_git_names(listing: Listing) -> Result<(Listing, Vec<String>), SaveError> {
    if !listing.entries().iter().any(|e| is_git_alias(&e.name)) {
        return Ok((listing, Vec::new()));
    }
    let (git, rest): (Vec<Entry>, Vec<Entry>) = listing
        .entries()
        .iter()
        .cloned()
        .partition(|e| is_git_alias(&e.name));
    let rest = Listing::new(rest).map_err(StoreError::from)?;
    Ok((rest, git.into_iter().map(|e| e.name).collect()))
}

/// Paths a warning names: the first few by path, and how many.
#[derive(Default)]
struct Named {
    first: BTreeSet<String>,
    count: u64,
}

impl Named {
    fn add(&mut self, path: String) {
        self.count += 1;
        self.first.insert(path);
        if self.first.len() > NAMED {
            self.first.pop_last();
        }
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

/// What a walk keeps track of: the ignore rules of the folders it is inside, what the base save
/// holds there, what it leaves untouched, and the first problem found.
struct WalkState {
    root: PathBuf,
    matcher: Matcher,
    /// The bytes of every ignore file the walk has read (storage-format.md §12).
    read: u64,
    problem: Option<SaveError>,
    /// One for each `.gitignore` skipped.
    warnings: Vec<Warning>,
    /// What the save leaves out because Git treats its name as `.git`: from the folder, and from
    /// the base save.
    git_names: Named,
    /// Folders that hold a repository or project of their own (decision 046), for the warning.
    nested: Named,
    /// Those the walk goes into for the base save's entries, and the folders kept inside them.
    nested_held: HashSet<String>,
    /// The paths of those the base save holds in the folders noted, named already.
    saved_git_names: HashSet<String>,
    /// The base save's listing of each folder the walk has noted, by project path, or `None`
    /// where the base save has no such folder. A folder not here hasn't been noted.
    old: HashMap<String, Option<Listing>>,
    /// Ignored folders the walk goes into only for the base save's entries in them.
    held: HashSet<String>,
    /// Files and links an ignore rule matches, kept because the base save holds them.
    ignored: HashSet<String>,
    /// Folders the walk keeps that Git would leave out, with all they hold: the `.gitignore`
    /// lines alone ignore them, or a folder they are in, and a `.boltignore` line takes them back.
    git_held: HashSet<String>,
    /// Files and links the walk keeps that Git would leave out, in the same way.
    gitignored: HashSet<String>,
    drive: Drive,
    /// The base save's entries the walk leaves untouched, by project path: the new save keeps
    /// them as they were, whatever the folder holds there.
    untouched: HashMap<String, Child>,
}

impl WalkState {
    /// The state for a walk of `root` on `drive`, beginning with its own ignore files and Git's
    /// personal lists, the user's own `own_list` among them. `top` is the base save's top listing,
    /// and `dropped` the names Git treats as `.git` it held.
    fn new(
        root: &Path,
        drive: Drive,
        top: Listing,
        dropped: Vec<String>,
        own_list: Option<&Path>,
    ) -> Result<WalkState, SaveError> {
        let mut rules = WalkState {
            root: root.to_owned(),
            matcher: Matcher::new(drive),
            read: 0,
            problem: None,
            warnings: Vec::new(),
            git_names: Named::default(),
            nested: Named::default(),
            nested_held: HashSet::new(),
            saved_git_names: HashSet::new(),
            old: HashMap::new(),
            held: HashSet::new(),
            ignored: HashSet::new(),
            git_held: HashSet::new(),
            gitignored: HashSet::new(),
            drive,
            untouched: HashMap::new(),
        };
        rules.drop_saved("", dropped);
        rules.note_untouched("", &top);
        rules.old.insert(String::new(), Some(top));
        rules.enter(root, own_list)?;
        Ok(rules)
    }

    /// Reads the base save's listing of `rel`, a folder the walk keeps, if it has that folder
    /// too. The folder above it must have been noted already.
    fn note_folder(&mut self, rel: &str, base: &mut Base) -> Result<(), SaveError> {
        let (up, name) = rel.rsplit_once('/').unwrap_or(("", rel));
        let above = self.old.get(up).ok_or_else(|| unnoted(rel))?;
        let saved = match above.as_ref().and_then(|l| l.get(name)) {
            Some(e) if e.kind == EntryKind::Folder => Some(base.listing(e.id)?),
            _ => None,
        };
        let listing = saved.map(|(listing, dropped)| {
            self.drop_saved(rel, dropped);
            self.note_untouched(rel, &listing);
            listing
        });
        self.old.insert(rel.to_owned(), listing);
        Ok(())
    }

    /// Names `dropped`, the entries the base save holds in the folder `rel` under names Git
    /// treats as `.git`, which the new save leaves out.
    fn drop_saved(&mut self, rel: &str, dropped: Vec<String>) {
        for name in dropped {
            let path = path_in(rel, &name);
            self.git_names.add(path.clone());
            self.saved_git_names.insert(path);
        }
    }

    /// Notes the entries of `listing`, the base save's listing of the folder `rel`, that are
    /// kept as they were. A link Git wrote as a file is `build`'s to spot, from the file it reads
    /// anyway. A folder kept this way keeps all it holds, names Git treats as `.git` included.
    fn note_untouched(&mut self, rel: &str, listing: &Listing) {
        let kept = kept_as_they_were(listing, self.drive);
        for (e, _) in listing.entries().iter().zip(kept).filter(|(_, k)| *k) {
            let path = path_in(rel, &e.name);
            let child = Child {
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
            self.untouched.insert(path, child);
        }
    }

    /// After the walk, which found `found`: the base save's folders, in the folders it found,
    /// that it didn't find on disk and that aren't kept whole. Each comes as its folder, name and
    /// listing, in path order, so a save that stops says the same thing every time.
    fn gone(&self, found: &Folders) -> Vec<(String, String, Id)> {
        let mut gone = Vec::new();
        for (rel, listing) in &self.old {
            let inner = listing.iter().flat_map(Listing::entries);
            for e in inner.filter(|e| e.kind == EntryKind::Folder) {
                let path = path_in(rel, &e.name);
                if !found.contains_key(&path) && !self.untouched.contains_key(&path) {
                    gone.push((rel.clone(), e.name.clone(), e.id));
                }
            }
        }
        gone.sort();
        gone
    }

    /// Puts the rules of the ignore files in `dir` in force: its `.gitignore`, then its
    /// `.boltignore`, whose lines come later so they can add to Git's rules or undo them. At the
    /// top, Git's personal lists come first, as in Git: the user's own `own_list`, then
    /// `.git/info/exclude` (decision 047).
    fn enter(&mut self, dir: &Path, own_list: Option<&Path>) -> Result<(), SaveError> {
        let folder = shown(dir.strip_prefix(&self.root).unwrap_or(dir));
        let mut in_force = self.matcher.in_force_bytes();
        let mut rules = Vec::new();
        if let Some(list) = own_list {
            rules.extend(self.own_list_rules(list, &mut in_force)?);
        }
        if folder.is_empty() && has_real_git_info(dir) {
            let info = dir.join(".git").join("info");
            rules.extend(self.file_rules(&info, &folder, "exclude", &mut in_force)?);
        }
        for name in [".gitignore", ".boltignore"] {
            rules.extend(self.file_rules(dir, &folder, name, &mut in_force)?);
        }
        if !rules.is_empty() {
            self.matcher.enter(&folder, rules);
        }
        Ok(())
    }

    /// The rules of the user's own Git ignore list `file`, or none if it isn't there. It is the
    /// user's own, so a link to it is followed; any other problem stops the save, as for a
    /// `.gitignore`.
    fn own_list_rules(
        &mut self,
        file: &Path,
        in_force: &mut u64,
    ) -> Result<Option<Rules>, SaveError> {
        let shown = file.display().to_string();
        match safe_fs::read_capped_following_links(file, safe_fs::MAX_IGNORE_FILE_BYTES) {
            Ok(bytes) => self.counted(&bytes, &shown, "", true, in_force).map(Some),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => {
                let why = e.to_string();
                let detail = if why.starts_with(&shown) {
                    why
                } else {
                    format!("{shown}: {why}")
                };
                Err(SaveError::Ignore { detail })
            }
        }
    }

    /// The rules of the ignore file `name` in `dir`, for the project's `folder`, or none if there
    /// isn't one. Read through `safe_fs`. A `.gitignore`, or Git's `.git/info/exclude`, that isn't
    /// a regular file is skipped with a warning, as Git skips a linked `.gitignore`. A folder
    /// Bolt may not look into is `Unreadable`.
    fn file_rules(
        &mut self,
        dir: &Path,
        folder: &str,
        name: &str,
        in_force: &mut u64,
    ) -> Result<Option<Rules>, SaveError> {
        let file = dir.join(name);
        let rel = shown(file.strip_prefix(&self.root).unwrap_or(&file));
        let gits = name != ".boltignore";
        let bytes = match safe_fs::read_capped(&file, safe_fs::MAX_IGNORE_FILE_BYTES) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => {
                let meta = match fs::symlink_metadata(&file) {
                    Ok(meta) => Some(meta),
                    Err(source) if source.kind() == io::ErrorKind::NotFound => None,
                    Err(source) => {
                        let path = match rel.rsplit_once('/') {
                            Some((up, _)) => up.to_owned(),
                            None => "the project folder".to_owned(),
                        };
                        return Err(SaveError::Unreadable { path, source });
                    }
                };
                // Judged by what the file is: an open can fail for other reasons with the same
                // error kind, and a regular .gitignore that can't be read must stop the save.
                if gits && meta.is_some_and(|m| !safe_fs::is_regular(&m)) {
                    let why = if name == ".gitignore" {
                        "Git skips a linked .gitignore too"
                    } else {
                        "Bolt reads nothing in .git through a link"
                    };
                    self.warnings.push(Warning::new(
                        warning::GITIGNORE_SKIPPED,
                        format!(
                            "Bolt skipped {}: it is a link, folder or special file, not a regular \
                             file, so its rules weren't used. {why}. Make it a regular file if its \
                             rules should apply.",
                            escape(&rel, false)
                        ),
                    ));
                    return Ok(None);
                }
                // `safe_fs` refusals start with the full path; the project path replaces it, so
                // the file is named once.
                let why = e.to_string();
                return Err(SaveError::Ignore {
                    detail: match why.strip_prefix(&file.display().to_string()) {
                        Some(rest) => format!("{rel}{rest}"),
                        None => format!("{rel}: {why}"),
                    },
                });
            }
        };
        self.counted(&bytes, &rel, folder, gits, in_force).map(Some)
    }

    /// The rules in `bytes`, an ignore file shown as `rel`, for the project's `folder`, held to
    /// the limits (storage-format.md §12) before a rule is read from it: `in_force` is the size
    /// of the files in force at `folder` so far. `gits` says whether they are Git's rules.
    fn counted(
        &mut self,
        bytes: &[u8],
        rel: &str,
        folder: &str,
        gits: bool,
        in_force: &mut u64,
    ) -> Result<Rules, SaveError> {
        let ignore = |detail| SaveError::Ignore { detail };
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
        Ok(Rules::parse(bytes, folder, gits))
    }

    /// Whether the walk keeps `path`: never what it leaves untouched, and what the ignore rules
    /// keep out stays out, unless the base save has it. A name Git treats as `.git` stays out too,
    /// noted for the warning. What it keeps that Git's rules alone would leave out is noted too.
    /// After a problem, such as an ignore file that fails, the problem is kept and nothing more
    /// is: the save will stop anyway.
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
        // Inside an ignored folder everything is ignored, as in Git: no line takes a name back,
        // so none is matched.
        if self.held.contains(&folder) {
            return self.saved_before(&folder, rel, is_dir);
        }
        // Before `path`'s own ignore files are read, if it is a folder: they are for what it holds.
        let verdict = self.matcher.verdict(path_bytes(rel), is_dir);
        if verdict.bolt == Decision::Ignored {
            return self.saved_before(&folder, rel, is_dir);
        }
        let git_ignores = self.git_held.contains(&folder) || verdict.git == Decision::Ignored;
        // After the rules, so an ignored one stays out without a word, unless the base save held
        // it: `Base` dropped and named those.
        let name = rel.file_name().and_then(|n| n.to_str());
        if name.is_some_and(is_git_alias) {
            let rel = shown(rel);
            if !self.saved_git_names.contains(&rel) {
                self.git_names.add(rel);
            }
            return false;
        }
        // After the rules, so an ignored one stays out without a word.
        if is_dir && holds_own_repository(path) {
            let kept_rel = shown(rel);
            self.nested.add(kept_rel.clone());
            let kept = self.saved_before(&folder, rel, is_dir);
            if kept {
                self.nested_held.insert(kept_rel);
            }
            return kept;
        }
        if is_dir && let Err(problem) = self.enter(path, None) {
            self.problem.get_or_insert(problem);
            return false;
        }
        if git_ignores && is_dir {
            self.git_held.insert(shown(rel));
        } else if git_ignores {
            self.gitignored.insert(shown(rel));
        }
        true
    }

    /// Whether the base save has the ignored `rel` under that name in `folder`, as a folder if
    /// and only if it is one now: what is saved stays saved, as in Git (decision 038). A folder
    /// kept this way is held: its ignore files aren't read, and it keeps only what was saved. A
    /// file or link kept this way is noted, so that a change to it can be warned about.
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
        // No ignore rule matches what a repository of its own holds, so none is blamed for it.
        let nested = self.nested_held.contains(folder);
        if saved && is_dir {
            self.held.insert(shown(rel));
            if nested {
                self.nested_held.insert(shown(rel));
            }
        } else if saved && !nested {
            self.ignored.insert(shown(rel));
        }
        saved
    }
}

/// Stack for the scan thread: 4 KiB per folder level the depth limit allows. Dropping the linked
/// `ignore` matchers of a deep tree recurses once per level and needs about 2 MiB in a Windows
/// debug build, more than the main thread's 1 MiB. Unused stack costs address space only.
const WALK_STACK_BYTES: usize = (MAX_SAVE_DEPTH + 2) * 4096;

/// What a walk of the project folder found.
pub(super) struct Scan {
    pub(super) folders: Folders,
    /// The base save's listing of each folder in `folders`, if it has that folder.
    pub(super) old: HashMap<String, Option<Listing>>,
    /// The new listings of the folders gone from disk that come back, still to be stored.
    pub(super) rebuilt: Vec<Listing>,
    /// The project paths of the files and links an ignore rule matches, kept because the base
    /// save holds them.
    pub(super) ignored: HashSet<String>,
    /// The project paths of the files and links kept that Git's rules alone would leave out,
    /// because a `.boltignore` line takes them back, or a folder they are in.
    pub(super) gitignored: HashSet<String>,
    /// One for each `.gitignore` skipped, and one for the names Git treats as `.git` left out.
    pub(super) warnings: Vec<Warning>,
}

/// Scans the project folder on a thread of its own, so callers don't need a deep stack. `drive`
/// is what `drive::probe` found there, and `own_list` the user's own Git ignore list, if any.
pub(super) fn walk(
    root: &Path,
    base: Base,
    limits: SaveLimits,
    drive: Drive,
    own_list: Option<&Path>,
) -> Result<Scan, SaveError> {
    thread::scope(|scope| {
        let scan = thread::Builder::new()
            .stack_size(WALK_STACK_BYTES)
            .spawn_scoped(scope, || scan_folder(root, base, limits, drive, own_list))
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
    own_list: Option<&Path>,
) -> Result<Scan, SaveError> {
    let too_large = |detail| SaveError::TooLarge { detail };
    // Limits are checked during the scan, so a tree too deep or too big is refused before it is all
    // read.
    let mut folders = Folders::from([(String::new(), Vec::new())]);
    let (top, dropped) = base.listing(base.root)?;
    // Bolt reads each ignore file itself: `ignore` would reopen it by path, following links and
    // reading without a limit.
    let rules = Arc::new(Mutex::new(WalkState::new(
        root, drive, top, dropped, own_list,
    )?));
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
    // Sorted, so a name the walk found can be looked up.
    for children in folders.values_mut() {
        children.sort_by(|a, b| a.name.cmp(&b.name));
    }
    // A folder gone from disk comes back if something below it is kept as it was.
    let mut back = Vec::new();
    let mut rebuilt = Vec::new();
    for (rel, name, id) in rules.gone(&folders) {
        let found = |c: &[Child]| c.binary_search_by(|c| c.name.as_str().cmp(&name)).is_ok();
        let taken = folders.get(&rel).is_some_and(|c| found(c));
        let top = path_in(&rel, &name);
        let named = &mut rules.git_names;
        if let Some(gone) = rebuild_gone(&mut base, &mut limits, drive, &top, id, taken, named)? {
            rebuilt.extend(gone.listings);
            let kind = Kind::Back {
                id: gone.id,
                kept: gone.kept,
            };
            let path = root.join(&top);
            let child = Child {
                name,
                kind,
                path,
                rel: top,
                executable: None,
            };
            back.push((rel, child));
        }
    }
    for (rel, child) in back {
        folders.entry(rel).or_default().push(child);
    }
    for (_, kept) in rules.untouched.drain() {
        if let Kind::Kept {
            kind: EntryKind::Folder,
            id,
            ..
        } = kept.kind
        {
            count_kept_folder(base.store, &mut limits, kept.rel.split('/').count(), id)?;
        }
        limits.add(1).map_err(too_large)?;
        let (up, _) = kept.rel.rsplit_once('/').unwrap_or_default();
        folders.entry(up.to_owned()).or_default().push(kept);
    }
    for children in folders.values_mut() {
        children.sort_by(|a, b| a.name.cmp(&b.name));
    }
    if rules.git_names.count > 0 {
        let left_out = git_names_left_out(&rules.git_names);
        rules.warnings.push(left_out);
    }
    if rules.nested.count > 0 {
        let left_out = nested_projects_left_out(&rules.nested);
        rules.warnings.push(left_out);
    }
    Ok(Scan {
        folders,
        old: std::mem::take(&mut rules.old),
        rebuilt,
        ignored: std::mem::take(&mut rules.ignored),
        gitignored: std::mem::take(&mut rules.gitignored),
        warnings: std::mem::take(&mut rules.warnings),
    })
}

/// Says the save left out entries whose names Git treats as `.git`, naming the first few.
fn git_names_left_out(names: &Named) -> Warning {
    let count = names.count;
    let (what, next) = if count == 1 {
        (
            "1 entry whose name Git treats as .git, with everything in it".to_owned(),
            "If Git uses it (a submodule's or worktree's .git file), list it in .boltignore to \
             leave it out without this warning; otherwise rename it to save it.",
        )
    } else {
        (
            format!("{count} entries whose names Git treats as .git, with everything in them"),
            "If Git uses them (a submodule's or worktree's .git file), list them in .boltignore to \
             leave them out without this warning; otherwise rename them to save them.",
        )
    };
    // Every invisible character shown, so a spelling can be told from `.git` itself.
    let first = names.first.iter().map(|path| escape_all(path));
    let message = format!(
        "Left out {what}:{}\nGit keeps such names for itself (some spellings only on Windows or \
         macOS), so Bolt leaves them out of saves, as it does on import. {next}",
        listed(first, count)
    );
    Warning::new(warning::GIT_NAMES_LEFT_OUT, message)
}

/// Says saves take nothing new from folders that hold a repository or project of their own,
/// naming the first few (decision 046).
fn nested_projects_left_out(named: &Named) -> Warning {
    let listed = listed(&named.first, named.count);
    let message = if named.count == 1 {
        format!(
            "1 folder holds a Git repository or Bolt project of its own, so saves take nothing new \
             from it:{listed}\nGit and Jujutsu leave such folders out too. To save its files, \
             remove its .git or .bolt; to leave it out without this warning, list it in \
             .boltignore."
        )
    } else {
        format!(
            "{} folders hold a Git repository or Bolt project of their own, so saves take nothing \
             new from them:{listed}\nGit and Jujutsu leave such folders out too. To save a \
             folder's files, remove its .git or .bolt; to leave it out without this warning, list \
             it in .boltignore.",
            named.count
        )
    };
    Warning::new(warning::NESTED_PROJECTS_LEFT_OUT, message)
}

/// A folder gone from disk, back with only what is kept as it was below it.
struct Gone {
    /// Its new listing's ID.
    id: Id,
    /// How many entries below it are kept as they were.
    kept: usize,
    /// Its new listings, its own last.
    listings: Vec<Listing>,
}

/// One folder open in [`rebuild_gone`]'s search.
struct Open {
    /// Its name; the folder the search starts from has none.
    name: String,
    listing: Listing,
    /// Which of its entries are kept as they were.
    kept: Vec<bool>,
    /// The next of its entries to look at.
    next: usize,
    /// What it holds once rebuilt: its kept entries, and its subfolders that come back.
    holds: Vec<Entry>,
}

/// The project path of `last` in the folder `name`, below `top` and the folders `open` down from
/// it; the folder the search starts from has no name.
fn path_below(top: &str, open: &[Open], name: &str, last: &str) -> String {
    let names = open.iter().map(|f| f.name.as_str()).chain([name, last]);
    names
        .filter(|n| !n.is_empty())
        .fold(top.to_owned(), |path, n| path + "/" + n)
}

/// Rebuilds the gone folder `top` from its base listing `id`, holding only the entries kept as
/// they were below it and the folders leading to them, counted against `limits`; `None` if none
/// is kept. If `taken`, something the walk found has `top`'s name, and a kept entry stops the
/// save. The names Git treats as `.git` it drops go to `git_names`. As in the diff walk, only
/// names and the listings from `top` down are held.
fn rebuild_gone(
    base: &mut Base,
    limits: &mut SaveLimits,
    drive: Drive,
    top: &str,
    id: Id,
    taken: bool,
    git_names: &mut Named,
) -> Result<Option<Gone>, SaveError> {
    let too_large = |detail| SaveError::TooLarge { detail };
    let depth = top.split('/').count();
    let mut open: Vec<Open> = Vec::new();
    let mut kept_in_all = 0;
    let mut listings = Vec::new();
    let mut read = Some((String::new(), id));
    loop {
        if let Some((name, id)) = read.take()
            && let Some((listing, dropped)) = base.gone_listing(id, depth + open.len())?
        {
            // Only now is a path below `top` needed, to name it.
            for git_name in &dropped {
                git_names.add(path_below(top, &open, &name, git_name));
            }
            let kept = kept_as_they_were(&listing, drive);
            let mut holds = Vec::new();
            for (e, _) in listing.entries().iter().zip(&kept).filter(|(_, k)| **k) {
                if taken {
                    return Err(SaveError::InTheWay {
                        path: top.to_owned(),
                        kept: path_below(top, &open, &name, &e.name),
                    });
                }
                if e.kind == EntryKind::Folder {
                    count_kept_folder(base.store, limits, depth + open.len() + 1, e.id)?;
                }
                limits.add(1).map_err(too_large)?;
                holds.push(e.clone());
            }
            kept_in_all += holds.len();
            open.push(Open {
                name,
                listing,
                kept,
                next: 0,
                holds,
            });
        }
        let Some(folder) = open.last_mut() else {
            return Ok(None);
        };
        if let Some(e) = folder.listing.entries().get(folder.next) {
            let kept = folder.kept.get(folder.next).copied().unwrap_or(false);
            folder.next += 1;
            if e.kind == EntryKind::Folder && !kept {
                read = Some((e.name.clone(), e.id));
            }
            continue;
        }
        // Done with this folder, which comes back if it holds anything.
        let Some(done) = open.pop() else {
            return Ok(None);
        };
        let mut back = None;
        if !done.holds.is_empty() {
            limits.check_depth(depth + open.len()).map_err(too_large)?;
            limits.add(1).map_err(too_large)?;
            let listing = Listing::new(done.holds).map_err(StoreError::from)?;
            back = Some(listing.id());
            listings.push(listing);
        }
        match (open.last_mut(), back) {
            (Some(above), Some(id)) => above.holds.push(Entry {
                name: done.name,
                kind: EntryKind::Folder,
                id,
                size: 0,
            }),
            (Some(_), None) => {}
            (None, back) => {
                return Ok(back.map(|id| Gone {
                    id,
                    kept: kept_in_all,
                    listings,
                }));
            }
        }
    }
}

/// Counts everything the kept folder `depth` levels down holds, from its listing `id` down,
/// against `limits`, one listing at a time: kept or not, a save holds no more than its format
/// allows.
fn count_kept_folder(
    store: &Store,
    limits: &mut SaveLimits,
    depth: usize,
    id: Id,
) -> Result<(), SaveError> {
    let too_large = |detail| SaveError::TooLarge { detail };
    let mut todo = vec![(id, depth)];
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

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;
    use crate::id::hash_content;
    use crate::listing::MAX_SAVE_ENTRIES;
    use crate::repo::Repository;
    use crate::snapshot::SaveOutcome;
    use crate::snapshot::tests::{
        damage_the_only_copy_of, newest, project, save_now, save_with, saved_root, saved_tree,
        this_drive, walk_project, write,
    };
    use crate::test_support::{add_link, add_listing, plant_root, plant_save};

    #[test]
    fn boltignore_git_and_bolt_folders_are_skipped() {
        let (dir, mut repo) = project();
        let root = dir.path();
        write(root, ".boltignore", b"*.tmp\nLibrary/\n");
        write(root, "a.tmp", b"x");
        write(root, "keep.txt", b"x");
        write(root, "Library/x", b"x");
        write(root, ".git/config", b"x");
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
    fn an_escaped_bang_ignores_a_name_that_begins_with_one() {
        let (dir, mut repo) = project();
        let root = dir.path();
        write(root, ".boltignore", b"\\!name\n");
        write(root, "!name", b"ignored");
        write(root, "sub/!name", b"ignored");
        write(root, "name", b"kept");
        let saved = save_now(&mut repo).unwrap();
        assert_eq!(saved_paths(&repo, saved), [".boltignore", "name", "sub"]);
    }

    /// A line with a slash in it counts from the folder of its own file, not from the project, so
    /// those in `sub` reach `sub/only.txt` and `sub/mid/dir.txt`, and nothing deeper or elsewhere.
    #[test]
    fn anchored_rules_in_a_nested_ignore_file_start_at_its_folder() {
        let (dir, mut repo) = project();
        let root = dir.path();
        write(root, "sub/.boltignore", b"/only.txt\nmid/dir.txt\n");
        write(root, "sub/deeper/.boltignore", b"/here.txt\n");
        let ignored = ["sub/only.txt", "sub/mid/dir.txt", "sub/deeper/here.txt"];
        let kept = [
            "only.txt",
            "mid/dir.txt",
            "here.txt",
            "sub/deep/only.txt",
            "sub/other/mid/dir.txt",
            "sub/here.txt",
            "sub/deeper/deep/here.txt",
        ];
        for path in ignored.iter().chain(&kept) {
            write(root, path, b"x");
        }
        let saved = save_now(&mut repo).unwrap();
        let saved = saved_paths(&repo, saved);
        for path in ignored {
            assert!(!saved.iter().any(|p| p == path), "{path} was saved");
        }
        for path in kept {
            assert!(saved.iter().any(|p| p == path), "{path} was left out");
        }
    }

    /// Git doesn't look inside a folder it ignores, so no line, there or above, takes a name
    /// back from it.
    #[test]
    fn nothing_is_taken_back_from_inside_an_ignored_folder() {
        let (dir, mut repo) = project();
        let root = dir.path();
        write(root, ".boltignore", b"build/\n!keep\n");
        write(root, "build/.boltignore", b"!keep\n");
        write(root, "build/keep", b"x");
        write(root, "build/deep/keep", b"x");
        write(root, "other/keep", b"x");
        let saved = save_now(&mut repo).unwrap();
        assert_eq!(
            saved_paths(&repo, saved),
            [".boltignore", "other", "other/keep"]
        );
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
    fn a_git_folder_is_never_saved_in_any_letter_case() {
        let (dir, mut repo) = project();
        let root = dir.path();
        write(root, "keep.txt", b"x");
        write(root, ".GIT/config", b"x");
        write(root, "sub/.Git/HEAD", b"x");
        let saved = save_now(&mut repo).unwrap();
        let warnings = warnings_of(&saved).to_vec();
        // Where the drive ignores letter case they are Git's folder: the top one is skipped
        // without a word like `.git`, and `sub` holds a repository of its own (decision 046).
        // Elsewhere they are what Windows and macOS would take for it.
        if this_drive().ignores_case {
            assert_eq!(saved_paths(&repo, saved), ["keep.txt"]);
            assert_eq!(warnings.len(), 1, "{warnings:?}");
            assert_eq!(warnings[0].kind, "nested_projects_left_out");
        } else {
            assert_eq!(saved_paths(&repo, saved), ["keep.txt", "sub"]);
            assert_eq!(warnings.len(), 1, "{warnings:?}");
            assert_eq!(warnings[0].kind, "git_names_left_out");
            let listed = ":\n  .GIT\n  sub/.Git\nGit keeps";
            assert!(warnings[0].message.contains(listed), "{warnings:?}");
        }
    }

    #[test]
    fn every_name_git_treats_as_git_is_left_out_with_a_warning() {
        let (dir, mut repo) = project();
        let root = dir.path();
        // First, so that on Windows no other name's 8.3 short name can take it.
        write(root, "git~1/hooks/post-checkout", b"#!/bin/sh\n");
        // Below the top too, and spellings with characters macOS's HFS+ ignores, which any system
        // can hold. A real `.git` makes its folder a repository of its own (decision 046).
        write(root, "sub/git~1", b"gitdir: /elsewhere\n");
        write(root, "sub/notes.txt", b"mine");
        write(root, ".g\u{200c}it/config", b"[core]\n");
        write(root, "a/.git\u{feff}/config", b"[core]\n");
        // An ignored one stays out without a word.
        write(root, ".gitignore", b"/b/git~1\n");
        write(root, "b/git~1", b"gitdir: /elsewhere\n");
        let saved = save_now(&mut repo).unwrap();
        let warnings = warnings_of(&saved).to_vec();
        // The rest of the folder is saved.
        assert_eq!(
            saved_paths(&repo, saved),
            [".gitignore", "a", "b", "sub", "sub/notes.txt"]
        );
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert_eq!(warnings[0].kind, "git_names_left_out");
        // Every invisible character is shown, the joiner too, so no spelling passes for `.git`.
        assert_eq!(
            warnings[0].message,
            "Left out 4 entries whose names Git treats as .git, with everything in them:\n  \
             .g\\u{200c}it\n  a/.git\\u{feff}\n  git~1\n  sub/git~1\nGit keeps such names for \
             itself (some spellings only on Windows or macOS), so Bolt leaves them out of saves, \
             as it does on import. If Git uses them (a submodule's or worktree's .git file), list \
             them in .boltignore to leave them out without this warning; otherwise rename them to \
             save them."
        );
        // Said again when nothing changed, since they are still there.
        let again = save_now(&mut repo).unwrap();
        assert!(
            matches!(again, SaveOutcome::NothingChanged { .. }),
            "{again:?}"
        );
        assert_eq!(warnings_of(&again), warnings);
    }

    #[test]
    fn the_names_left_out_are_counted_past_the_first_few() {
        let (dir, mut repo) = project();
        for i in (1..=7).rev() {
            write(
                dir.path(),
                &format!("sub{i}/git~1"),
                b"gitdir: /elsewhere\n",
            );
        }
        let saved = save_now(&mut repo).unwrap();
        let message = &warnings_of(&saved)[0].message;
        let listed = "Left out 7 entries whose names Git treats as .git, with everything in them:\n  \
                      sub1/git~1\n  sub2/git~1\n  sub3/git~1\n  sub4/git~1\n  sub5/git~1\n  and 2 \
                      more\n";
        assert!(message.starts_with(listed), "{message}");
    }

    #[test]
    fn a_git_name_the_last_save_holds_is_left_out_and_named() {
        let (dir, mut repo) = project();
        let root = dir.path();
        // A save made before saves left these out, or a planted one.
        plant_save(
            &mut repo,
            &[
                (".GIT/x", EntryKind::File, b"x"),
                ("sub/.git", EntryKind::File, b"gitdir: /elsewhere\n"),
                ("keep.txt", EntryKind::File, b"x"),
            ],
        );
        write(root, "keep.txt", b"x");
        // Still in the folder too, and named once; it also makes `sub` a repository of its own,
        // which keeps what the last save holds there (decision 046).
        write(root, "sub/.git", b"gitdir: /elsewhere\n");
        let saved = save_now(&mut repo).unwrap();
        let warnings = warnings_of(&saved).to_vec();
        assert_eq!(saved_paths(&repo, saved), ["keep.txt", "sub"]);
        let kinds: Vec<&str> = warnings.iter().map(|w| w.kind).collect();
        assert_eq!(kinds, ["git_names_left_out", "nested_projects_left_out"]);
        let listed = "Left out 2 entries whose names Git treats as .git, with everything in them:\n  \
                      .GIT\n  sub/.git\nGit keeps";
        assert!(warnings[0].message.starts_with(listed), "{warnings:?}");
    }

    #[test]
    fn a_git_name_kept_as_it_was_or_saved_and_ignored_is_left_out_and_named() {
        let (dir, mut repo) = project();
        let root = dir.path();
        // Entries a save would keep as they were (a project link, and another in a folder gone
        // from disk), and an ignored file the last save holds.
        plant_save(
            &mut repo,
            &[
                ("git~1", EntryKind::ProjectLink, COMMIT),
                ("gone/.g\u{200c}it", EntryKind::ProjectLink, COMMIT),
                ("lib/.git", EntryKind::File, b"gitdir: /elsewhere\n"),
                ("lib/a.txt", EntryKind::File, b"x"),
            ],
        );
        write(root, ".gitignore", b".git\n");
        write(root, "lib/.git", b"gitdir: /elsewhere\n");
        write(root, "lib/a.txt", b"x");
        let saved = save_now(&mut repo).unwrap();
        let warnings = warnings_of(&saved).to_vec();
        // None is kept, and the gone folder doesn't come back for one. `lib` holds a repository
        // of its own, so it keeps what the last save holds there (decision 046).
        assert_eq!(
            saved_paths(&repo, saved),
            [".gitignore", "lib", "lib/a.txt"]
        );
        let kinds: Vec<&str> = warnings.iter().map(|w| w.kind).collect();
        assert_eq!(kinds, ["git_names_left_out", "nested_projects_left_out"]);
        let listed = ":\n  git~1\n  gone/.g\\u{200c}it\n  lib/.git\nGit keeps";
        assert!(warnings[0].message.contains(listed), "{warnings:?}");
        // Gone from the save, so the next one names none; `lib` still holds its `.git`.
        let again = save_now(&mut repo).unwrap();
        let kinds: Vec<&str> = warnings_of(&again).iter().map(|w| w.kind).collect();
        assert_eq!(kinds, ["nested_projects_left_out"]);
    }

    #[test]
    fn the_names_left_out_are_shown_escaped() {
        use crate::test_support::{HOSTILE, assert_hostile_text_escaped};
        let mut names = Named::default();
        names.add(format!("{HOSTILE}/.g\u{200c}it"));
        let message = git_names_left_out(&names).message;
        assert_hostile_text_escaped(&message);
        assert!(message.contains("/.g\\u{200c}it\n"), "{message}");
    }

    #[test]
    fn one_name_left_out_is_said_in_the_singular() {
        let (dir, mut repo) = project();
        let root = dir.path();
        write(root, "sub/git~1", b"gitdir: /elsewhere\n");
        write(root, "sub/notes.txt", b"mine");
        let saved = save_now(&mut repo).unwrap();
        let warnings = warnings_of(&saved).to_vec();
        assert_eq!(saved_paths(&repo, saved), ["sub", "sub/notes.txt"]);
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert_eq!(warnings[0].kind, "git_names_left_out");
        assert_eq!(
            warnings[0].message,
            "Left out 1 entry whose name Git treats as .git, with everything in it:\n  sub/git~1\n\
             Git keeps such names for itself (some spellings only on Windows or macOS), so Bolt \
             leaves them out of saves, as it does on import. If Git uses it (a submodule's or \
             worktree's .git file), list it in .boltignore to leave it out without this warning; \
             otherwise rename it to save it."
        );
    }

    /// A worktree, a clone and a Bolt project inside the project keep their own history, so a save
    /// takes nothing from them, and says so each time (decision 046).
    #[test]
    fn folders_that_hold_a_repository_of_their_own_are_left_out() {
        let (dir, mut repo) = project();
        let root = dir.path();
        write(root, "keep.txt", b"x");
        write(root, "wt/.git", b"gitdir: /elsewhere\n");
        write(root, "wt/src/main.rs", b"fn main() {}\n");
        write(root, "clone/.git/HEAD", b"ref: refs/heads/main\n");
        write(root, "clone/readme.md", b"x");
        write(root, "game/.bolt/format", b"bolt-repository 1\n");
        write(root, "game/level.bin", b"x");
        let saved = save_now(&mut repo).unwrap();
        let warnings = warnings_of(&saved).to_vec();
        assert_eq!(saved_paths(&repo, saved), ["keep.txt"]);
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert_eq!(warnings[0].kind, "nested_projects_left_out");
        assert_eq!(
            warnings[0].message,
            "3 folders hold a Git repository or Bolt project of their own, so saves take nothing \
             new from them:\n  clone\n  game\n  wt\nGit and Jujutsu leave such folders out too. To \
             save a folder's files, remove its .git or .bolt; to leave it out without this \
             warning, list it in .boltignore."
        );
        let again = save_now(&mut repo).unwrap();
        assert!(
            matches!(again, SaveOutcome::NothingChanged { .. }),
            "{again:?}"
        );
        assert_eq!(warnings_of(&again), warnings);
    }

    /// What is saved stays saved, as in an ignored folder: a change to it is taken, a new file
    /// isn't, and no ignore rule is blamed (decisions 038 and 046).
    #[test]
    fn a_folder_that_becomes_a_repository_keeps_what_was_saved() {
        let (dir, mut repo) = project();
        let root = dir.path();
        plant_save(&mut repo, &[("lib/a.txt", EntryKind::File, b"old")]);
        write(root, "lib/a.txt", b"new");
        write(root, "lib/b.txt", b"x");
        write(root, "lib/.git/HEAD", b"ref: refs/heads/main\n");
        let saved = save_now(&mut repo).unwrap();
        let kinds: Vec<&str> = warnings_of(&saved).iter().map(|w| w.kind).collect();
        assert_eq!(kinds, ["nested_projects_left_out"]);
        assert_eq!(
            warnings_of(&saved)[0].message,
            "1 folder holds a Git repository or Bolt project of its own, so saves take nothing new \
             from it:\n  lib\nGit and Jujutsu leave such folders out too. To save its files, \
             remove its .git or .bolt; to leave it out without this warning, list it in \
             .boltignore."
        );
        let tree = saved_tree(repo.store(), saved_root(saved));
        assert_eq!(tree.keys().collect::<Vec<_>>(), ["lib", "lib/a.txt"]);
        assert_eq!(tree["lib/a.txt"].1, b"new");
    }

    #[test]
    fn an_ignored_folder_that_holds_a_repository_stays_out_without_a_word() {
        let (dir, mut repo) = project();
        let root = dir.path();
        write(root, ".gitignore", b"/wt/\n");
        write(root, "wt/.git", b"gitdir: /elsewhere\n");
        write(root, "wt/x.txt", b"x");
        let saved = save_now(&mut repo).unwrap();
        assert!(warnings_of(&saved).is_empty(), "{saved:?}");
        assert_eq!(saved_paths(&repo, saved), [".gitignore"]);
    }

    /// The user's own list, then `.git/info/exclude`, then the project's own rules, as in Git: the
    /// last line that matches decides (decision 047).
    #[test]
    fn gits_personal_lists_keep_files_out_behind_the_projects_rules() {
        let (dir, mut repo) = project();
        let root = dir.path();
        let own = tempfile::tempdir().unwrap();
        let list = own.path().join("ignore");
        fs::write(&list, "*.log\n!b.log\n.claude/settings.local.json\n").unwrap();
        write(
            root,
            ".git/info/exclude",
            b"b.log\n/.worktrees/\n!keep.txt\n",
        );
        write(root, ".gitignore", b"keep.txt\n!c.log\n");
        let files = [
            "a.log",
            "b.log",
            "c.log",
            "keep.txt",
            ".claude/settings.local.json",
            ".worktrees/a/x.txt",
            "plain.txt",
        ];
        for path in files {
            write(root, path, b"x");
        }
        let saved = save_with(&mut repo, Some(&list)).unwrap();
        assert!(warnings_of(&saved).is_empty(), "{saved:?}");
        assert_eq!(
            saved_paths(&repo, saved),
            [".claude", ".gitignore", "c.log", "plain.txt"]
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_linked_info_exclude_is_skipped_with_a_warning() {
        let (dir, mut repo) = project();
        let root = dir.path();
        let elsewhere = tempfile::tempdir().unwrap();
        fs::write(elsewhere.path().join("exclude"), "a.txt\n").unwrap();
        fs::create_dir_all(root.join(".git/info")).unwrap();
        let link = root.join(".git/info/exclude");
        std::os::unix::fs::symlink(elsewhere.path().join("exclude"), link).unwrap();
        write(root, "a.txt", b"x");
        let saved = save_now(&mut repo).unwrap();
        let kinds: Vec<&str> = warnings_of(&saved).iter().map(|w| w.kind).collect();
        assert_eq!(kinds, ["gitignore_skipped"]);
        assert_eq!(saved_paths(&repo, saved), ["a.txt"]);
    }

    #[test]
    fn an_own_list_bolt_cant_read_stops_the_save() {
        let (dir, mut repo) = project();
        write(dir.path(), "a.txt", b"x");
        let own = tempfile::tempdir().unwrap();
        let list = own.path().join("ignore");
        fs::write(&list, comment_of(safe_fs::MAX_IGNORE_FILE_BYTES + 1)).unwrap();
        let err = save_with(&mut repo, Some(&list)).unwrap_err();
        assert!(matches!(err, SaveError::Ignore { .. }), "{err}");
        assert!(err.to_string().contains("ignore"), "{err}");
        // One that isn't there is none.
        let gone = own.path().join("none");
        assert!(save_with(&mut repo, Some(&gone)).is_ok());
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
        let mut rules =
            WalkState::new(root, this_drive(), Listing::empty(), Vec::new(), None).unwrap();
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
        let mut rules =
            WalkState::new(root, this_drive(), Listing::empty(), Vec::new(), None).unwrap();
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
        let mut rules =
            WalkState::new(root, this_drive(), Listing::empty(), Vec::new(), None).unwrap();
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
    fn a_change_to_a_saved_file_a_rule_matches_is_warned_about() {
        let (dir, mut repo) = project();
        let root = dir.path();
        write(root, "a.dll", b"1");
        write(root, "b.dll", b"2");
        write(root, "gone.dll", b"3");
        write(root, "Library/keep.txt", b"x");
        save_now(&mut repo).unwrap();
        // Unchanged, the files a rule now matches say nothing.
        write(root, ".gitignore", b"*.dll\nLibrary/\n");
        let saved = save_now(&mut repo).unwrap();
        assert!(warnings_of(&saved).is_empty(), "{saved:?}");
        write(root, "a.dll", b"changed");
        write(root, "Library/keep.txt", b"changed");
        // Neither a new file, which the rule keeps out, nor a deleted one counts.
        write(root, "c.dll", b"new");
        fs::remove_file(root.join("gone.dll")).unwrap();
        let saved = save_now(&mut repo).unwrap();
        let warnings = warnings_of(&saved).to_vec();
        assert_eq!(
            saved_paths(&repo, saved),
            [
                ".gitignore",
                "Library",
                "Library/keep.txt",
                "a.dll",
                "b.dll"
            ]
        );
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert_eq!(warnings[0].kind, "ignored_changes_saved");
        assert_eq!(
            warnings[0].message,
            "Saved changes to 2 files that an ignore rule matches:\n  Library/keep.txt\n  a.dll\n\
             The last save holds them, so they stay saved. Git would leave such changes out \
             unless the files were added (with `git add -f` if Git doesn't track them yet). Bolt \
             keeps saving them until they are deleted or moved out of the project folder."
        );
    }

    #[test]
    fn a_saved_file_a_rule_matches_that_became_a_link_is_warned_about() {
        let (dir, mut repo) = project();
        let root = dir.path();
        write(root, "a.dll", b"no-such-file");
        save_now(&mut repo).unwrap();
        write(root, ".gitignore", b"*.dll\n");
        // A link holding the file's bytes as its target: only the kind changed.
        fs::remove_file(root.join("a.dll")).unwrap();
        if !crate::test_support::symlink(Path::new("no-such-file"), &root.join("a.dll")) {
            return;
        }
        let saved = save_now(&mut repo).unwrap();
        let warnings = warnings_of(&saved);
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert_eq!(warnings[0].kind, "ignored_changes_saved");
        let listed = "a change to 1 file that an ignore rule matches:\n  a.dll\n";
        assert!(warnings[0].message.contains(listed), "{warnings:?}");
    }

    #[test]
    fn what_boltignore_takes_back_from_gitignore_is_warned_about_when_new_or_changed() {
        let (dir, mut repo) = project();
        let root = dir.path();
        write(root, ".env", b"API_KEY=put-yours-here\n");
        write(root, "secrets/old.key", b"1");
        save_now(&mut repo).unwrap();
        // Git keeps out `.env`, every `.pem` and `secrets/`, which .boltignore takes back.
        write(root, ".gitignore", b".env\n*.pem\nsecrets/\n*.log\n");
        write(
            root,
            ".boltignore",
            b"!.env\n!*.pem\n!secrets/\n!notes.txt\n",
        );
        // Unchanged, they say nothing.
        let saved = save_now(&mut repo).unwrap();
        assert!(warnings_of(&saved).is_empty(), "{saved:?}");
        write(root, ".env", b"API_KEY=sk_live_REAL_SECRET\n");
        write(root, "deploy.pem", b"key");
        write(root, "secrets/new.key", b"2");
        // Neither what Git keeps too, nor what both keep out, counts.
        write(root, "notes.txt", b"n");
        write(root, "debug.log", b"d");
        let saved = save_now(&mut repo).unwrap();
        let warnings = warnings_of(&saved).to_vec();
        assert_eq!(
            saved_paths(&repo, saved),
            [
                ".boltignore",
                ".env",
                ".gitignore",
                "deploy.pem",
                "notes.txt",
                "secrets",
                "secrets/new.key",
                "secrets/old.key"
            ]
        );
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert_eq!(warnings[0].kind, "gitignored_files_saved");
        assert_eq!(
            warnings[0].message,
            "Saved 3 new or changed files that .gitignore keeps out but .boltignore takes back:\n  \
             .env\n  deploy.pem\n  secrets/new.key\nGit would leave such changes out unless the \
             files were added (with `git add -f` if Git doesn't track them yet). If they \
             shouldn't be saved, remove the .boltignore lines that take them back and move them \
             out of the project folder; this save keeps them."
        );
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
        // It names every kind it keeps, and promises nothing a later version may or may not do.
        assert_eq!(
            warnings[0].message,
            "Kept 3 entries as they were in the last save, because this computer can't hold them \
             as they are (project links, names it can't create, or links Git wrote as files)."
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
        let comes_back_within = |levels, entries| {
            let base = Base {
                limits: SaveLimits::new(levels, entries),
                ..newest(&repo)
            };
            let scan = walk(dir.path(), base, SaveLimits::default(), this_drive(), None).unwrap();
            scan.folders[""].iter().any(|c| c.name == "a")
        };
        assert!(comes_back_within(4, 5));
        // Past either limit the base save is invalid, so nothing there can be kept.
        assert!(!comes_back_within(3, 5));
        assert!(!comes_back_within(4, 4));
    }

    /// A folder or project link entry.
    fn entry(name: &str, kind: EntryKind, id: Id) -> Entry {
        Entry {
            name: name.to_owned(),
            kind,
            id,
            size: 0,
        }
    }

    /// Saves the project, which must make a save, and says how long it took.
    fn timed_save(repo: &mut Repository, what: &str) -> Listing {
        let started = std::time::Instant::now();
        let saved = saved_top(repo);
        let took = started.elapsed();
        eprintln!("{what}: saved in {took:?}");
        // Well above what a debug build takes; copying a path per folder needs gigabytes here.
        assert!(
            took < std::time::Duration::from_secs(10),
            "{what}: {took:?}"
        );
        saved
    }

    #[test]
    fn a_deep_chain_of_gone_folders_comes_back_in_linear_time() {
        let (dir, mut repo) = project();
        // 4,000 folders of 200-byte names with a link at the bottom: an 800 KB path that nothing
        // may copy once per folder. Planted from the bottom up, so planting isn't quadratic.
        let name = "n".repeat(200);
        let mut first = None;
        plant_root(&mut repo, |packs| {
            let link = add_link(packs, COMMIT);
            let mut id = add_listing(packs, vec![entry("sub", EntryKind::ProjectLink, link)]).0;
            for _ in 1..4000 {
                id = add_listing(packs, vec![entry(&name, EntryKind::Folder, id)]).0;
            }
            first = Some(id);
            add_listing(packs, vec![entry(&name, EntryKind::Folder, id)]).0
        });
        write(dir.path(), "new.txt", b"x");
        let saved = timed_save(&mut repo, "4,000 gone folders deep");
        // The chain comes back whole, so it holds the link at the bottom.
        assert_eq!(saved.get(&name).map(|e| e.id), first);
    }

    #[test]
    fn many_gone_folders_side_by_side_come_back_in_linear_time() {
        let (dir, mut repo) = project();
        // 20,000 gone folders in the top folder and 20,000 in a gone folder, each holding a link.
        let mut wide = None;
        let mut inner = None;
        plant_root(&mut repo, |packs| {
            let link = add_link(packs, COMMIT);
            let holds_link = add_listing(packs, vec![entry("sub", EntryKind::ProjectLink, link)]).0;
            let many = |prefix: char| -> Vec<Entry> {
                (0..20_000)
                    .map(|i| entry(&format!("{prefix}{i:05}"), EntryKind::Folder, holds_link))
                    .collect()
            };
            let below = add_listing(packs, many('g')).0;
            let mut top = many('f');
            top.push(entry("wide", EntryKind::Folder, below));
            (wide, inner) = (Some(below), Some(holds_link));
            add_listing(packs, top).0
        });
        write(dir.path(), "new.txt", b"x");
        let saved = timed_save(&mut repo, "40,000 gone folders side by side");
        assert_eq!(saved.entries().len(), 20_002);
        assert_eq!(saved.get("f12345").map(|e| e.id), inner);
        assert_eq!(saved.get("wide").map(|e| e.id), wide);
    }

    #[test]
    fn kept_entries_in_two_gone_subfolders_come_back_together() {
        let (dir, mut repo) = project();
        let base = plant_save(
            &mut repo,
            &[
                ("gone/a/one", EntryKind::ProjectLink, COMMIT),
                ("gone/a/x.txt", EntryKind::File, b"x"),
                ("gone/b/two", EntryKind::ProjectLink, COMMIT),
                ("gone/c/y.txt", EntryKind::File, b"y"),
            ],
        );
        write(dir.path(), "new.txt", b"x");
        let saved = saved_top(&mut repo);
        let paths: Vec<String> = saved_tree(repo.store(), saved.id()).into_keys().collect();
        assert_eq!(
            paths,
            [
                "gone",
                "gone/a",
                "gone/a/one",
                "gone/b",
                "gone/b/two",
                "new.txt"
            ]
        );
        for link in ["gone/a/one", "gone/b/two"] {
            let was = entry_at(&repo, &base, link);
            assert_eq!(entry_at(&repo, &saved, link), was, "{link}");
        }
    }

    /// A drive that ignores letter case, whatever this one does.
    const CASE_BLIND: Drive = Drive {
        ignores_case: true,
        composes_unicode: false,
    };

    #[test]
    fn a_kept_folder_in_a_gone_folder_counts_with_everything_it_holds() {
        let (dir, mut repo) = project();
        plant_save(
            &mut repo,
            &[
                ("gone/Docs/a", EntryKind::File, b"a"),
                ("gone/Docs/b", EntryKind::File, b"b"),
                ("gone/docs/c", EntryKind::File, b"c"),
            ],
        );
        // `gone`, the two kept folders, and the three files they hold.
        let walk_under = |most| {
            let limits = SaveLimits::new(MAX_SAVE_DEPTH, most);
            walk(dir.path(), newest(&repo), limits, CASE_BLIND, None)
        };
        assert!(walk_under(6).is_ok());
        let Err(SaveError::TooLarge { detail }) = walk_under(5) else {
            panic!("6 entries were accepted under a limit of 5");
        };
        assert!(detail.contains("more than 5 "), "{detail}");
    }

    #[test]
    fn a_kept_folder_counts_its_depth_against_the_limit() {
        let (dir, mut repo) = project();
        plant_save(
            &mut repo,
            &[
                ("Docs/a/b/x", EntryKind::File, b"x"),
                ("docs/y", EntryKind::File, b"y"),
            ],
        );
        // `Docs/a/b` is three levels down.
        let walk_within = |levels| {
            let limits = SaveLimits::new(levels, MAX_SAVE_ENTRIES);
            walk(dir.path(), newest(&repo), limits, CASE_BLIND, None)
        };
        assert!(walk_within(3).is_ok());
        let Err(SaveError::TooLarge { detail }) = walk_within(2) else {
            panic!("a kept folder three levels deep was accepted under a limit of 2");
        };
        assert!(detail.contains("more than 2 levels"), "{detail}");
    }

    #[test]
    fn a_kept_folder_in_a_gone_folder_counts_its_depth_against_the_limit() {
        let (dir, mut repo) = project();
        plant_save(
            &mut repo,
            &[
                ("gone/Docs/a/x", EntryKind::File, b"x"),
                ("gone/docs/y", EntryKind::File, b"y"),
            ],
        );
        // None of it is on disk, and `gone/Docs/a` is three levels down.
        let walk_within = |levels| {
            let limits = SaveLimits::new(levels, MAX_SAVE_ENTRIES);
            walk(dir.path(), newest(&repo), limits, CASE_BLIND, None)
        };
        assert!(walk_within(3).is_ok());
        let Err(SaveError::TooLarge { detail }) = walk_within(2) else {
            panic!("a kept folder three levels deep was accepted under a limit of 2");
        };
        assert!(detail.contains("more than 2 levels"), "{detail}");
    }

    #[test]
    fn a_damaged_listing_in_a_gone_folder_stops_the_save() {
        let (dir, mut repo) = project();
        let base = plant_save(&mut repo, &[("gone/x/sub", EntryKind::ProjectLink, COMMIT)]);
        let x = entry_at(&repo, &base, "gone/x").unwrap().id;
        damage_the_only_copy_of(&repo, &x);
        write(dir.path(), "new.txt", b"x");
        // Unread, it might hold a link that must stay, so the save can't go on without it.
        let mut repo = Repository::open(dir.path()).unwrap();
        let head = repo.head().unwrap();
        let err = save_now(&mut repo).unwrap_err();
        assert!(matches!(err, SaveError::Repo(_)), "{err:?}");
        assert_eq!(repo.head().unwrap(), head);
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
    fn something_in_the_way_further_down_is_named_with_its_whole_path() {
        let (dir, mut repo) = project();
        plant_save(
            &mut repo,
            &[("vendor/libs/a/b/sub", EntryKind::ProjectLink, COMMIT)],
        );
        // `vendor` is still a folder; the folder that leads to the link is a file now.
        write(dir.path(), "vendor/libs", b"x");
        let err = save_now(&mut repo).unwrap_err();
        let SaveError::InTheWay { path, kept } = &err else {
            panic!("{err:?}");
        };
        assert_eq!(
            (path.as_str(), kept.as_str()),
            ("vendor/libs", "vendor/libs/a/b/sub")
        );
    }

    #[test]
    fn a_base_save_past_its_limits_is_read_no_further() {
        let (_dir, mut repo) = project();
        // A listing of three files, and an ID no object has: reading that one fails.
        let mut three = None;
        plant_root(&mut repo, |packs| {
            let empty = packs.add_content(b"").unwrap();
            let file = |name: &str| Entry {
                name: name.to_owned(),
                kind: EntryKind::File,
                id: empty,
                size: 0,
            };
            let id = add_listing(packs, vec![file("a"), file("b"), file("c")]).0;
            three = Some(id);
            id
        });
        let (three, missing) = (three.unwrap(), Id([0xee; 32]));
        let base = |limits| Base::new(repo.store(), three, limits);
        // Within its limits it reads, so the missing listing fails.
        assert!(
            base(SaveLimits::default())
                .gone_listing(missing, 1)
                .is_err()
        );
        // The third read of three entries passes a limit of 8; nothing is read after that.
        let mut spent = base(SaveLimits::new(MAX_SAVE_DEPTH, 8));
        assert!(spent.gone_listing(three, 1).unwrap().is_some());
        assert!(spent.gone_listing(three, 1).unwrap().is_some());
        assert!(spent.gone_listing(three, 1).unwrap().is_none());
        assert!(spent.gone_listing(missing, 1).unwrap().is_none());
        // Likewise once a folder lies past the depth limit.
        let mut spent = base(SaveLimits::new(2, MAX_SAVE_ENTRIES));
        assert!(spent.gone_listing(three, 3).unwrap().is_none());
        assert!(spent.gone_listing(missing, 1).unwrap().is_none());
    }

    #[test]
    fn the_search_of_gone_folders_ends_once_the_base_save_passes_its_limits() {
        let (dir, mut repo) = project();
        // A tree bomb in the gone folder `a`: 5,000 folders all pointing at one listing of 5,000
        // files, 25,000,000 paths in all. Then the gone folder `b`, which holds a link.
        plant_root(&mut repo, |packs| {
            let empty = packs.add_content(b"").unwrap();
            let files = (0..5000).map(|i| Entry {
                name: format!("x{i:04}"),
                kind: EntryKind::File,
                id: empty,
                size: 0,
            });
            let wide = add_listing(packs, files.collect()).0;
            let folders = (0..5000).map(|i| entry(&format!("f{i:04}"), EntryKind::Folder, wide));
            let bomb = add_listing(packs, folders.collect()).0;
            let link = add_link(packs, COMMIT);
            let holds_link = add_listing(packs, vec![entry("sub", EntryKind::ProjectLink, link)]).0;
            let top = vec![
                entry("a", EntryKind::Folder, bomb),
                entry("b", EntryKind::Folder, holds_link),
            ];
            add_listing(packs, top).0
        });
        // The base's limit is passed on the third read of the wide listing.
        let base = Base {
            limits: SaveLimits::new(MAX_SAVE_DEPTH, 20_000),
            ..newest(&repo)
        };
        let started = std::time::Instant::now();
        let scan = walk(dir.path(), base, SaveLimits::default(), this_drive(), None).unwrap();
        let took = started.elapsed();
        eprintln!("a tree bomb past the base's limits: walked in {took:?}");
        // Reading on would take thousands of reads of 5,000 entries.
        assert!(took < std::time::Duration::from_secs(5), "{took:?}");
        // Nothing past the limits is read, so `b` isn't either: past them the base save is
        // invalid, and nothing in it can be kept.
        assert!(scan.folders[""].is_empty());
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
            walk(dir.path(), newest(&repo), limits, drive, None)
        };
        assert!(walk_under(6).is_ok());
        let Err(SaveError::TooLarge { detail }) = walk_under(5) else {
            panic!("6 entries were accepted under a limit of 5");
        };
        assert!(detail.contains("more than 5 "), "{detail}");
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
            walk(dir.path(), base, SaveLimits::default(), this_drive(), None)
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
                walk(dir.path(), base, limits, drive, None),
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
