//! The repository: the `format` file, the head pointer, the change lock and the commit protocol
//! (docs/storage-format.md section 10, decision 010).

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, TryLockError};
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::diary::{Action, DiaryEntry, Position, State};
use crate::encoding::DecodeError;
use crate::fsio::{self, fault_point, write_atomic};
use crate::id::{ChangeId, Id};
use crate::listing::Listing;
use crate::pack::{ObjectKind, PackError, PackWriter};
use crate::safe_fs;
use crate::save::{EXTRA_LINE, Person, Save};
use crate::store::{Store, StoreError};

pub const FORMAT_TEXT: &str = "bolt-repository 1\nhash blake3\n";

#[derive(Debug, thiserror::Error)]
pub enum RepoError {
    #[error("Not inside a Bolt project.")]
    NotAProject,
    #[error("This folder is already a Bolt project.")]
    AlreadyAProject { root: PathBuf },
    #[error("This folder is inside a Bolt project at {}.", root.display())]
    InsideAnotherProject { root: PathBuf },
    #[error("This project uses {detail}, which this version of Bolt doesn't understand.")]
    NewerFormat { detail: String },
    #[error("This project's Bolt data looks damaged: {detail}.")]
    Damaged { detail: String },
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error(transparent)]
    Store(StoreError),
    #[error(transparent)]
    Pack(PackError),
    #[error(transparent)]
    Decode(#[from] DecodeError),
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
                RepoError::NewerFormat { .. } => "Update Bolt to open it.",
                RepoError::Damaged { .. } => {
                    "Nothing was changed. Ask for help before trying to repair it."
                }
                _ => return None,
            }
            .to_owned(),
        )
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

/// Refuses a `.bolt` whose fixed folders are links, junctions or files. A missing one passes:
/// `init` creates them, and a command that needs one fails without it.
fn check_shape(bolt: &Path) -> Result<(), RepoError> {
    for dir in fixed_folders(bolt) {
        match safe_fs::check_real_dir(&dir) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(refused(e)),
            _ => {}
        }
    }
    Ok(())
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

impl Repository {
    /// Makes `root` a new project: the folders, then the first save (an empty listing) on line
    /// `main`, then `.bolt/format` last. A `.bolt` folder without `format` is an interrupted
    /// start, which this finishes.
    pub fn init(root: &Path, who: &Person) -> Result<Repository, RepoError> {
        let already = || RepoError::AlreadyAProject {
            root: root.to_owned(),
        };
        let bolt = root.join(".bolt");
        // Before anything is created: creating a folder through a planted link would write
        // outside the project.
        check_shape(&bolt)?;
        if format_path(root).try_exists()? {
            return Err(already());
        }
        if let Some(outer) = root.ancestors().skip(1).find(|a| format_path(a).is_file()) {
            return Err(RepoError::InsideAnotherProject {
                root: outer.to_owned(),
            });
        }
        for dir in fixed_folders(&bolt) {
            fsio::create_dir_durable(&dir)?;
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
        let lock = repo.lock_for_change("start", &mut |_| {})?;
        // Another `bolt start` may have finished while this one waited for the lock.
        if format_path(root).try_exists()? {
            return Err(already());
        }
        let mut pack = repo.new_pack()?;
        let listing = pack.add_record(ObjectKind::FolderListing, &Listing::empty().encode())?;
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
        let save = pack.add_record(ObjectKind::Save, &encoded)?;
        let entry = DiaryEntry {
            parent: None,
            action: Action::Command,
            command: "start".into(),
            who: who.clone(),
            bolt_version: env!("CARGO_PKG_VERSION").into(),
            state: State {
                lines: BTreeMap::from([("main".to_owned(), save)]),
                tags: BTreeMap::new(),
                position: Position::OnLine("main".into()),
                shelves: vec![],
                remotes: BTreeMap::new(),
            },
            safety_copies: vec![],
        };
        repo.commit(&lock, pack, &entry)?;
        write_atomic(&repo.tmp, &format_path(root), FORMAT_TEXT.as_bytes())?;
        Ok(repo)
    }

    /// Opens the project that holds `start`: the nearest folder, from `start` up, with a
    /// `.bolt/format` file.
    pub fn discover(start: &Path) -> Result<Repository, RepoError> {
        let root = start
            .ancestors()
            .find(|a| format_path(a).is_file())
            .ok_or(RepoError::NotAProject)?;
        Repository::open(root)
    }

    /// Opens the project whose top folder is `root` (the folder holding `.bolt`).
    pub fn open(root: &Path) -> Result<Repository, RepoError> {
        let bolt = root.join(".bolt");
        // Nothing in `.bolt` is read before its folders are known to be real folders.
        check_shape(&bolt)?;

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
        // The head first: its pack was in the store before the head pointed at it.
        let head = read_head(&bolt)?;
        let store = Store::open(&packs)?;
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
    pub fn new_pack(&self) -> Result<PackWriter, RepoError> {
        Ok(PackWriter::create(&self.tmp)?)
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

    /// Takes the change lock, waiting for another changing command if needed. `on_wait` is
    /// called once, with that command's lock information, before waiting. `command` must
    /// already be redacted: it is written to `state/lock-info` as given.
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

    /// Stores `entry` in `pack`, moves the pack into the store, then replaces `state/head`.
    /// The command has happened exactly when the head is replaced (decision 010).
    ///
    /// An entry that would not decode is refused before anything is written. Note: an `Io`
    /// error can come from the final folder sync after the head was already replaced, so
    /// callers must not report "nothing changed" for an `Io` error.
    pub fn commit(
        &mut self,
        _lock: &ChangeLock,
        mut pack: PackWriter,
        entry: &DiaryEntry,
    ) -> Result<Id, RepoError> {
        let bytes = entry.encode();
        DiaryEntry::decode(&bytes)?;
        let id = pack.add_record(ObjectKind::DiaryEntry, &bytes)?;
        if let Some(path) = pack.finish(&self.packs)? {
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
    use crate::diary::{Action, Position, State};
    use crate::save::Person;
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
                tags: BTreeMap::new(),
                position: Position::OnLine("main".into()),
                shelves: vec![],
                remotes: BTreeMap::new(),
            },
            safety_copies: vec![],
        }
    }

    fn commit(repo: &mut Repository, e: &DiaryEntry) -> Result<Id, RepoError> {
        let lock = repo.lock_for_change("save", &mut |_| {})?;
        let pack = repo.new_pack()?;
        repo.commit(&lock, pack, e)
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
            let pack = repo.new_pack().unwrap();
            fault::arm(n);
            let result = repo.commit(&lock, pack, &entry(2));
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
