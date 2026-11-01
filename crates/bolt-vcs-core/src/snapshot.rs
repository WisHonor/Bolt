//! `bolt save`: walks the project folder, stores its files in a pack, builds the folder
//! listings bottom-up, and commits the save with its diary entry.

use std::cmp::Reverse;
use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;
use std::fs::{self, File, Metadata};
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use ignore::WalkBuilder;

use crate::diary::{Action, DiaryEntry, Position};
use crate::diff::diff_saves;
use crate::id::{ChangeId, ContentHasher, Id, hash_content};
use crate::listing::{Entry, EntryKind, Listing};
use crate::names::{PortabilityProblem, folder_problems, validate_name};
use crate::pack::{ObjectKind, PackError, PackWriter};
use crate::repo::{RepoError, Repository};
use crate::safe_fs;
use crate::save::{EXTRA_LINE, Person, Save};
use crate::store::{Store, StoreError};

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
        changed_files: usize,
    },
    /// The folder matches the line's current save, which is `current`.
    NothingChanged { current: Id },
}

#[derive(Debug, thiserror::Error)]
pub enum SaveError {
    #[error(transparent)]
    Repo(#[from] RepoError),
    #[error("Can't read {path}: {source}.")]
    Unreadable { path: String, source: io::Error },
    #[error("{path} changed while Bolt was saving it.")]
    ChangedDuringSave { path: String },
    #[error("Bolt can't save {path}: {reason}.")]
    InvalidName { path: String, reason: String },
    #[error("{}", not_portable(problems))]
    NotPortable {
        /// The folder (empty for the project's top folder) and the problem found in it.
        problems: Vec<(String, PortabilityProblem)>,
    },
    #[error("A .boltignore file has a problem: {detail}.")]
    Ignore { detail: String },
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

fn not_portable(problems: &[(String, PortabilityProblem)]) -> String {
    let mut text = "Some new names would not work on every system:".to_owned();
    for (folder, problem) in problems {
        if folder.is_empty() {
            let _ = write!(text, "\n  {problem}");
        } else {
            let _ = write!(text, "\n  in {folder}/: {problem}");
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
                SaveError::Unreadable { .. } => {
                    "Nothing was saved. Let Bolt read the file, or list it in .boltignore."
                }
                SaveError::ChangedDuringSave { .. } => {
                    "Nothing was saved. Try again when the file stops changing, or list it in .boltignore."
                }
                SaveError::InvalidName { .. } => {
                    "Nothing was saved. Rename it, or list it in .boltignore."
                }
                SaveError::NotPortable { .. } => {
                    "Nothing was saved. Rename them, or list them in .boltignore."
                }
                SaveError::Ignore { .. } => "Nothing was saved. Fix the pattern and try again.",
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
    let folders = walk(repo.root())?;
    let old = old_listings(repo.store(), parent.root, &folders)?;
    check_names(&folders, &old)?;
    let mut pack = repo.new_pack()?;
    let root = build(repo.store(), &mut pack, &folders, &old)?;
    if root == parent.root {
        return Ok(SaveOutcome::NothingChanged { current: parent_id });
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
    let save_id = pack.add_record(ObjectKind::Save, &encoded)?;
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
    repo.commit(&lock, pack, &diary)?;
    // The save is committed now; failing to count its files must not make it look failed.
    let changed_files = diff_saves(repo.store(), Some(parent.root), root).map_or(0, |c| c.len());
    Ok(SaveOutcome::Saved {
        save_id,
        save,
        changed_files,
    })
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    File,
    Symlink,
    Folder,
}

struct Child {
    name: String,
    kind: Kind,
    /// Where it is on disk.
    path: PathBuf,
    /// Its project path, with `/` between names.
    rel: String,
    /// The `x` bits, where the system has them.
    executable: Option<bool>,
}

/// Every folder of the project by project path (`""` for the top folder), with its children
/// sorted by name.
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

/// What is wrong with the `.boltignore` in `dir`, named by its project path. A missing one is
/// fine; one that exists must be a regular UTF-8 file of at most 1 MiB (storage-format.md §12).
fn boltignore_problem(root: &Path, dir: &Path) -> Option<String> {
    let file = dir.join(".boltignore");
    let problem = match safe_fs::read_capped(&file, safe_fs::MAX_BOLTIGNORE_BYTES) {
        Ok(bytes) => std::str::from_utf8(&bytes).err()?.to_string(),
        Err(e) if e.kind() == io::ErrorKind::NotFound => return None,
        Err(e) => e.to_string(),
    };
    let rel = shown(file.strip_prefix(root).unwrap_or(&file));
    Some(format!("{rel}: {problem}"))
}

fn walk(root: &Path) -> Result<Folders, SaveError> {
    let mut folders = Folders::from([(String::new(), Vec::new())]);
    // `ignore` opens each folder's .boltignore itself before it hands the folder out: it follows
    // links, reads without a limit and drops read errors without a word. So each one is checked
    // first, the top folder's here and every other one before `ignore` enters its folder.
    if let Some(detail) = boltignore_problem(root, root) {
        return Err(SaveError::Ignore { detail });
    }
    let problem = Arc::new(OnceLock::new());
    let walker = WalkBuilder::new(root)
        .hidden(false)
        .git_ignore(false)
        .git_exclude(false)
        .git_global(false)
        .ignore(false)
        .parents(false)
        .follow_links(false)
        .add_custom_ignore_filename(".boltignore")
        .filter_entry({
            let (root, problem) = (root.to_owned(), Arc::clone(&problem));
            move |e| {
                if !e.file_type().is_some_and(|t| t.is_dir()) {
                    return true;
                }
                if matches!(e.file_name().to_str(), Some(".bolt" | ".git")) {
                    return false;
                }
                match boltignore_problem(&root, e.path()) {
                    // Not entered, so `ignore` never opens the file; the save stops below.
                    Some(detail) => {
                        let _ = problem.set(detail);
                        false
                    }
                    None => true,
                }
            }
        })
        .build();
    for item in walker {
        let entry = item.map_err(|e| walk_error(root, e))?;
        // A broken .boltignore comes with its folder's entry, not as an error item; going on
        // without it would save the files it was meant to keep out.
        if let Some(err) = entry.error() {
            return Err(SaveError::Ignore {
                detail: describe(root, err),
            });
        }
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
            folders.entry(rel.clone()).or_default();
            Kind::Folder
        } else if t.is_file() {
            Kind::File
        } else {
            // shortcut: sockets, pipes and devices are skipped silently; revisit with `bolt status`.
            continue;
        };
        folders.entry(parents.join("/")).or_default().push(Child {
            name: (*name).to_owned(),
            kind,
            path: path.to_owned(),
            rel,
            executable: executable_bits(&meta),
        });
    }
    if let Some(detail) = problem.get() {
        return Err(SaveError::Ignore {
            detail: detail.clone(),
        });
    }
    for children in folders.values_mut() {
        children.sort_by(|a, b| a.name.cmp(&b.name));
    }
    Ok(folders)
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
        // Anything else is a pattern Bolt could not understand.
        (None, _) => SaveError::Ignore {
            detail: describe(root, &err),
        },
    }
}

/// The parent save's listing of each folder, if it had that folder.
fn old_listings(
    store: &Store,
    old_root: Id,
    folders: &Folders,
) -> Result<HashMap<String, Option<Listing>>, SaveError> {
    let mut old: HashMap<String, Option<Listing>> = HashMap::new();
    // A folder's path sorts after its parent's, so the parent is always looked up first.
    for rel in folders.keys() {
        let listing = if rel.is_empty() {
            Some(store.listing(&old_root)?)
        } else {
            let (up, name) = rel.rsplit_once('/').unwrap_or(("", rel));
            match old
                .get(up)
                .and_then(Option::as_ref)
                .and_then(|l| l.get(name))
            {
                Some(e) if e.kind == EntryKind::Folder => Some(store.listing(&e.id)?),
                _ => None,
            }
        };
        old.insert(rel.clone(), listing);
    }
    Ok(old)
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

/// Stores every file and listing, deepest folders first, and returns the top listing's ID.
fn build(
    store: &Store,
    pack: &mut PackWriter,
    folders: &Folders,
    old: &HashMap<String, Option<Listing>>,
) -> Result<Id, SaveError> {
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
                        store_file(pack, store, &c.path, &mut || {}).map_err(|e| e.at(&c.rel))?;
                    // Without `x` bits, a file stays executable if it was (decision 004).
                    let executable = c.executable.unwrap_or_else(|| {
                        before
                            .and_then(|l| l.get(&c.name))
                            .is_some_and(|e| e.kind == EntryKind::Executable)
                    });
                    let kind = if executable {
                        EntryKind::Executable
                    } else {
                        EntryKind::File
                    };
                    (kind, id, size)
                }
                Kind::Symlink => {
                    let (id, size) = store_link(pack, store, &c.path).map_err(|e| e.at(&c.rel))?;
                    (EntryKind::Symlink, id, size)
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
        if !store.contains(&id) {
            pack.add_record(ObjectKind::FolderListing, &listing.encode())?;
        }
        built.insert(rel, id);
    }
    built.remove("").ok_or_else(missing).map_err(Into::into)
}

/// Stores a symlink's target as content; the link itself is never followed.
fn store_link(pack: &mut PackWriter, store: &Store, path: &Path) -> Result<(Id, u64), SaveError> {
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
    if !store.contains(&id) {
        pack.add_content(target.as_bytes())?;
    }
    Ok((id, target.len() as u64))
}

/// Stores one file in two passes: hash it, then copy it into `pack` unless the store or pack
/// already holds it. A file that changes in between gives `ChangedDuringSave`.
pub(crate) fn store_file(
    pack: &mut PackWriter,
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
    let mut file = File::open(path).map_err(unreadable)?;
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
    if store.contains(&id) || pack.contains(&id) {
        return Ok((id, size));
    }
    file.seek(SeekFrom::Start(0)).map_err(unreadable)?;
    match pack.add_content_from(&mut file, id) {
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
    use crate::listing::EntryKind;
    use crate::names::{name_problems, validate_name};
    use crate::save::Person;
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

    /// Every entry of a saved folder, by path; folders have no bytes.
    fn saved_tree(store: &Store, root: Id) -> Tree {
        fn walk(store: &Store, id: Id, prefix: &str, out: &mut Tree) {
            for e in store.listing(&id).unwrap().entries() {
                let path = format!("{prefix}{}", e.name);
                let mut bytes = Vec::new();
                if e.kind == EntryKind::Folder {
                    walk(store, e.id, &format!("{path}/"), out);
                } else {
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
        } = save_now(&mut repo).unwrap()
        else {
            panic!("nothing was saved");
        };
        assert_eq!(changed_files, 1);
        assert_eq!(save.line(), Some("main"));
        assert_eq!(save.author, who());
        let (head, entry) = repo.current().unwrap();
        assert_eq!(entry.state.lines["main"], save_id);
        assert_eq!(entry.command, "save");
        assert!(entry.parent.is_some());
        let first = repo.store().save(&save.parents[0], false).unwrap();
        assert_eq!(first.message, "Start project");
        match save_now(&mut repo).unwrap() {
            SaveOutcome::NothingChanged { current } => assert_eq!(current, save_id),
            SaveOutcome::Saved { .. } => panic!("saved again"),
        }
        assert_eq!(repo.head().unwrap(), head);
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
        let history: Vec<Id> = crate::history::line_history(&open())
            .unwrap()
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        assert_eq!(history[..2], [second, saved[0]]);
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
        // On a case-insensitive disk (macOS by default) `readme` overwrote `README`, so the
        // collision can't exist there.
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
    fn broken_boltignore_stops_the_save() {
        let (dir, mut repo) = project();
        let head = repo.head().unwrap();
        write(dir.path(), "secret.key", b"x");
        // `a[` would be fine: like Git, `ignore` reads an unclosed `[` literally.
        write(dir.path(), ".boltignore", b"secret.key\n[z-a]\n");
        match save_now(&mut repo) {
            Err(SaveError::Ignore { detail }) => {
                assert!(
                    detail.contains(".boltignore") && detail.contains("[z-a]"),
                    "{detail}"
                );
            }
            other => panic!("{:?}", other.map(|_| ())),
        }
        assert_eq!(repo.head().unwrap(), head);
    }

    /// The save must stop with `Ignore` naming `.boltignore`, and change nothing. Returns the
    /// problem's text.
    fn assert_ignore_problem(repo: &mut Repository) -> String {
        let head = repo.head().unwrap();
        let detail = match save_now(repo) {
            Err(SaveError::Ignore { detail }) => detail,
            other => panic!("{:?}", other.map(|_| ())),
        };
        assert!(detail.starts_with(".boltignore: "), "{detail}");
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
        let detail = assert_ignore_problem(&mut repo);
        assert!(detail.contains("regular file"), "{detail}");
    }

    #[test]
    fn oversized_boltignore_stops_the_save() {
        let (dir, mut repo) = project();
        // One comment line, 1 MiB + 1 byte with its newline.
        let mut big = vec![b'#'; 1 << 20];
        big.push(b'\n');
        write(dir.path(), ".boltignore", &big);
        let detail = assert_ignore_problem(&mut repo);
        assert!(detail.contains("larger than 1048576 bytes"), "{detail}");
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
                detail.starts_with("sub/.boltignore: ") && detail.contains("regular file"),
                "{detail}"
            ),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn non_utf8_boltignore_stops_the_save() {
        let (dir, mut repo) = project();
        write(dir.path(), "secret.key", b"x");
        // A byte that is not UTF-8: `ignore` would stop reading there and drop `secret.key`.
        write(dir.path(), ".boltignore", b"# \xff\nsecret.key\n");
        assert_ignore_problem(&mut repo);
    }

    #[cfg(unix)]
    #[test]
    fn boltignore_that_is_a_folder_stops_the_save() {
        let (dir, mut repo) = project();
        write(dir.path(), "secret.key", b"x");
        fs::create_dir(dir.path().join(".boltignore")).unwrap();
        assert_ignore_problem(&mut repo);
    }

    #[test]
    fn unreadable_boltignore_stops_the_save() {
        let (dir, mut repo) = project();
        let head = repo.head().unwrap();
        write(dir.path(), "sub/secret.key", b"x");
        write(dir.path(), "sub/.boltignore", b"secret.key\n");
        let ignore = dir.path().join("sub/.boltignore");
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
                assert!(detail.starts_with("sub/.boltignore: "), "{detail}");
            }
            other => panic!("{:?}", other.map(|_| ())),
        }
        assert_eq!(repo.head().unwrap(), head);
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

    #[test]
    fn save_stops_when_a_file_changes_during_the_save() {
        let (dir, repo) = project();
        let path = dir.path().join("growing.log");
        fs::write(&path, b"line 1\n").unwrap();
        let mut pack = repo.new_pack().unwrap();
        let got = store_file(&mut pack, repo.store(), &path, &mut || {}).unwrap();
        assert_eq!(got, (hash_content(b"line 1\n"), 7));
        let mut append = || {
            use std::io::Write;
            let mut f = fs::OpenOptions::new().append(true).open(&path).unwrap();
            f.write_all(b"line 2\n").unwrap();
        };
        assert!(matches!(
            store_file(&mut pack, repo.store(), &path, &mut append),
            Err(SaveError::ChangedDuringSave { .. })
        ));
    }
}
