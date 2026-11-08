//! The switch journal, `.bolt/state/journal` (storage-format.md section 10): every change a switch
//! of the folder to another save will make, written before the first one so that the next command
//! after a crash can finish it. It drives writes into the user's folder, so reading it trusts
//! nothing: the checksum first, then every path against the name rules.
#![deny(clippy::indexing_slicing, clippy::arithmetic_side_effects)]

use std::ffi::OsStr;
use std::io::{self, Read};

use crate::encoding::{DecodeError, Reader, Writer};
use crate::fsio::{self, Done};
use crate::id::Id;
use crate::listing::EntryKind;
use crate::names::{MAX_PATH_BYTES, validate_path};
use crate::repo::{ChangeLock, RepoError, Repository, cant_write, refused};
use crate::safe_fs;

const MAGIC: &[u8; 8] = b"BOLTJRNL";
const VERSION: u8 = 1;
const FILE: &str = "journal";
const CHECKSUM_LEN: usize = 32;
/// The most the file may hold, checksum included; a bigger journal isn't written.
pub const MAX_BYTES: u64 = 4 << 30;
/// As many entries as a save may have.
pub const MAX_ACTIONS: u64 = 10_000_000;
/// The fewest bytes an action takes: a one-byte path and its length, the action and the flag.
const MIN_ACTION: usize = 4;
/// The magic, the version, the target and `from`: all a command that only reads needs.
const FRONT_BYTES: u64 = 8 + 1 + 32 + 32;

/// A switch of the folder from the save of one diary entry to another's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Journal {
    /// The diary entry the switch commits, already stored.
    pub target: Id,
    /// The entry `state/head` named when the switch began.
    pub from: Id,
    /// Sorted by path, each path once.
    pub actions: Vec<Action>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Action {
    /// A project path, `/` between names.
    pub path: String,
    pub change: Change,
    /// What the folder held there when the switch was planned, re-checked before it is changed;
    /// `None` if it held nothing.
    pub expected: Option<Expected>,
}

/// An entry as the plan found it. A folder's `id` is all zeros and its `size` 0.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Expected {
    /// A file, an executable, a link or a folder.
    pub kind: EntryKind,
    pub id: Id,
    pub size: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    File(Content),
    Executable(Content),
    Link(Content),
    MakeFolder,
    RemoveFile,
    RemoveFolder,
}

/// What a file or link is written with: its content ID and size (a link's: of its target).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Content {
    pub id: Id,
    pub size: u64,
}

impl Change {
    fn code(self) -> u8 {
        match self {
            Change::File(_) => 1,
            Change::Executable(_) => 2,
            Change::Link(_) => 3,
            Change::MakeFolder => 4,
            Change::RemoveFile => 5,
            Change::RemoveFolder => 6,
        }
    }

    fn content(self) -> Option<Content> {
        match self {
            Change::File(c) | Change::Executable(c) | Change::Link(c) => Some(c),
            _ => None,
        }
    }
}

impl Journal {
    /// The journal's bytes, checksum included.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        for b in MAGIC {
            w.u8(*b);
        }
        w.u8(VERSION);
        w.id(&self.target);
        w.id(&self.from);
        w.varint(self.actions.len() as u64);
        for a in &self.actions {
            w.string(&a.path);
            w.u8(a.change.code());
            w.bool(a.expected.is_some());
            if let Some(e) = &a.expected {
                w.u8(e.kind as u8);
                w.id(&e.id);
                w.varint(e.size);
            }
            if let Some(c) = a.change.content() {
                w.id(&c.id);
                w.varint(c.size);
            }
        }
        let mut bytes = w.into_bytes();
        let sum = blake3::hash(&bytes);
        bytes.extend_from_slice(sum.as_bytes());
        bytes
    }

    /// Reads bytes `encode` wrote, refusing any that break a rule of the format.
    pub fn decode(bytes: &[u8]) -> Result<Journal, DecodeError> {
        let (body, sum) = bytes
            .split_last_chunk::<CHECKSUM_LEN>()
            .ok_or(DecodeError::UnexpectedEnd)?;
        if blake3::hash(body).as_bytes() != sum {
            return Err(DecodeError::Invalid("journal checksum"));
        }
        let (mut r, target, from) = front(body)?;
        // Checked against what is left, and nothing allocated ahead for it.
        let fits = r.remaining().checked_div(MIN_ACTION).unwrap_or(0) as u64;
        let count = r.count(MAX_ACTIONS.min(fits), "action count")?;
        let mut actions = Vec::new();
        let mut last: Option<&str> = None;
        for _ in 0..count {
            let path = r.string(MAX_PATH_BYTES, "path")?;
            if last.is_some_and(|before| before >= path) {
                return Err(DecodeError::Invalid("paths out of order"));
            }
            validate_path(path)?;
            last = Some(path);
            let code = r.u8()?;
            let expected = if r.bool()? {
                Some(read_expected(&mut r)?)
            } else {
                None
            };
            let change = match code {
                1 => Change::File(read_content(&mut r)?),
                2 => Change::Executable(read_content(&mut r)?),
                3 => Change::Link(read_content(&mut r)?),
                4 => Change::MakeFolder,
                5 => Change::RemoveFile,
                6 => Change::RemoveFolder,
                _ => return Err(DecodeError::Invalid("unknown journal action")),
            };
            // A removal must say what it removes, so that it can be checked first.
            let removes_folder = match change {
                Change::RemoveFile => Some(false),
                Change::RemoveFolder => Some(true),
                _ => None,
            };
            if let Some(folder) = removes_folder
                && expected.is_none_or(|e| (e.kind == EntryKind::Folder) != folder)
            {
                return Err(DecodeError::Invalid("a removal without what it removes"));
            }
            actions.push(Action {
                path: path.to_owned(),
                change,
                expected,
            });
        }
        r.finish()?;
        Ok(Journal {
            target,
            from,
            actions,
        })
    }

    /// Writes the journal to `state/journal`, whole, flushed and renamed into place.
    pub fn write(&self, repo: &Repository, _lock: &ChangeLock) -> Result<(), RepoError> {
        let bytes = self.encode();
        if bytes.len() as u64 > MAX_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::FileTooLarge,
                "the plan for this switch is over 4 GiB, more than Bolt can record",
            )
            .into());
        }
        // One that wouldn't read back is refused before it can be relied on.
        Journal::decode(&bytes)?;
        fsio::write_atomic(repo.tmp_folder(), repo.state_folder(), FILE, &bytes)
            .map_err(|e| cant_write(repo.state_dir().join(FILE), e))?;
        Ok(())
    }

    /// The journal in `repo`, or `None` if no switch is unfinished. One that can't be read or
    /// breaks a rule is damage.
    pub fn read(repo: &Repository) -> Result<Option<Journal>, RepoError> {
        let shown = repo.state_dir().join(FILE);
        match safe_fs::read_capped_in(repo.state_folder(), OsStr::new(FILE), &shown, MAX_BYTES) {
            Ok(bytes) => Journal::decode(&bytes)
                .map(Some)
                .map_err(|e| unusable(format!("can't be read ({e})"))),
            Err(e) => not_read(e),
        }
    }

    /// The target and `from` of the journal in `repo`, from its front alone, or `None` if no
    /// switch is unfinished. For commands that only read: the rest is neither read nor checked.
    pub fn read_front(repo: &Repository) -> Result<Option<(Id, Id)>, RepoError> {
        let shown = repo.state_dir().join(FILE);
        let read = safe_fs::open_regular_in(repo.state_folder(), OsStr::new(FILE), &shown)
            .and_then(|file| {
                let mut bytes = Vec::new();
                file.take(FRONT_BYTES).read_to_end(&mut bytes)?;
                Ok(bytes)
            });
        match read {
            Ok(bytes) => front(&bytes)
                .map(|(_, target, from)| Some((target, from)))
                .map_err(|e| unusable(format!("can't be read ({e})"))),
            Err(e) => not_read(e),
        }
    }

    /// Removes `state/journal`, if it is there, for good. Once it is gone, a flush of `state/`
    /// that fails comes back as `NotFlushed`.
    pub(crate) fn remove(repo: &Repository, _lock: &ChangeLock) -> Result<Done, RepoError> {
        let state = repo.state_folder();
        fsio::fault_point("journal:removing")?;
        match state.remove_file(OsStr::new(FILE)) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e.into()),
            _ => {}
        }
        Ok(fsio::flush_after(state, "journal:removed"))
    }

    /// `remove`, by a command that hasn't moved its head yet and so can still stop: a flush that
    /// fails is an error too.
    pub(crate) fn remove_before_head(
        repo: &Repository,
        lock: &ChangeLock,
    ) -> Result<(), RepoError> {
        match Journal::remove(repo, lock)? {
            Done::Flushed => Ok(()),
            Done::NotFlushed(e) => Err(e.into()),
        }
    }
}

/// The magic, version, target and `from` that start a journal, with a reader of what follows.
fn front(bytes: &[u8]) -> Result<(Reader<'_>, Id, Id), DecodeError> {
    let rest = bytes
        .strip_prefix(MAGIC.as_slice())
        .ok_or(DecodeError::Invalid("not a switch journal"))?;
    let mut r = Reader::new(rest);
    if r.u8()? != VERSION {
        return Err(DecodeError::Invalid("journal version"));
    }
    let target = r.id()?;
    let from = r.id()?;
    Ok((r, target, from))
}

/// `Ok(None)` for a journal that isn't there; otherwise why it couldn't be read.
fn not_read<T>(e: io::Error) -> Result<Option<T>, RepoError> {
    if e.kind() == io::ErrorKind::NotFound {
        return Ok(None);
    }
    Err(match refused(e) {
        RepoError::Damaged { detail } => unusable(format!("can't be used ({detail})")),
        other => other,
    })
}

/// The journal can't be used: an error of its own, so the user hears which file it is.
pub(crate) fn unusable(detail: impl Into<String>) -> RepoError {
    RepoError::JournalUnusable {
        detail: detail.into(),
    }
}

fn read_expected(r: &mut Reader<'_>) -> Result<Expected, DecodeError> {
    let kind = EntryKind::from_u8(r.u8()?)
        .filter(|k| {
            matches!(
                k,
                EntryKind::File | EntryKind::Executable | EntryKind::Symlink | EntryKind::Folder
            )
        })
        .ok_or(DecodeError::Invalid("expected entry kind"))?;
    let id = r.id()?;
    let size = r.varint()?;
    if kind == EntryKind::Folder && (id != Id([0; 32]) || size != 0) {
        return Err(DecodeError::Invalid("a folder with contents"));
    }
    Ok(Expected { kind, id, size })
}

fn read_content(r: &mut Reader<'_>) -> Result<Content, DecodeError> {
    Ok(Content {
        id: r.id()?,
        size: r.varint()?,
    })
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing,
        clippy::arithmetic_side_effects
    )]
    use super::*;
    use crate::save::Person;
    use proptest::prelude::*;

    fn action(path: &str, change: Change, expected: Option<Expected>) -> Action {
        Action {
            path: path.into(),
            change,
            expected,
        }
    }

    fn file(id: u8, size: u64) -> Content {
        Content {
            id: Id([id; 32]),
            size,
        }
    }

    fn found(kind: EntryKind, id: u8, size: u64) -> Option<Expected> {
        Some(Expected {
            kind,
            id: Id([id; 32]),
            size,
        })
    }

    fn sample() -> Journal {
        Journal {
            target: Id([1; 32]),
            from: Id([2; 32]),
            actions: vec![
                action("a", Change::MakeFolder, None),
                action("a/b", Change::Executable(file(3, 9)), None),
                action("c", Change::RemoveFile, found(EntryKind::File, 4, 1)),
                action("d", Change::RemoveFolder, found(EntryKind::Folder, 0, 0)),
            ],
        }
    }

    /// `body` with a checksum that matches it, so that only the rule under test is broken.
    fn sealed(mut body: Vec<u8>) -> Vec<u8> {
        let sum = blake3::hash(&body);
        body.extend_from_slice(sum.as_bytes());
        body
    }

    fn body(journal: &Journal) -> Vec<u8> {
        let bytes = journal.encode();
        bytes[..bytes.len() - 32].to_vec()
    }

    #[test]
    fn a_journal_with_a_wrong_checksum_or_unsorted_paths_is_refused() {
        let good = sample().encode();
        assert_eq!(Journal::decode(&good).unwrap(), sample());
        // Any byte changed, the checksum's included.
        for at in [0, 8, 9, 80, good.len() - 33, good.len() - 1] {
            let mut bad = good.clone();
            bad[at] ^= 1;
            assert_eq!(
                Journal::decode(&bad),
                Err(DecodeError::Invalid("journal checksum")),
                "{at}"
            );
        }
        for len in 0..good.len() {
            assert!(Journal::decode(&good[..len]).is_err(), "{len}");
        }
        let mut swapped = sample();
        swapped.actions.swap(0, 1);
        let mut twice = sample();
        twice.actions[1].path = "a".into();
        for bad in [swapped, twice] {
            assert_eq!(
                Journal::decode(&bad.encode()),
                Err(DecodeError::Invalid("paths out of order"))
            );
        }
        // Each still carrying a checksum that matches.
        let mut longer = body(&sample());
        longer.push(0);
        let mut version = body(&sample());
        version[8] = 2;
        let mut huge = body(&Journal {
            actions: vec![],
            ..sample()
        });
        huge.pop();
        huge.extend_from_slice(&[0xff, 0xff, 0xff, 0xff, 0x0f]);
        let mut unknown = body(&sample());
        unknown[8 + 1 + 64 + 1 + 2] = 7;
        for bad in [longer, version, huge, unknown] {
            assert!(Journal::decode(&sealed(bad)).is_err());
        }
        let refused = |change, expected| {
            let journal = Journal {
                actions: vec![action("x", change, expected)],
                ..sample()
            };
            Journal::decode(&journal.encode()).is_err()
        };
        assert!(refused(Change::RemoveFile, None));
        assert!(refused(Change::RemoveFile, found(EntryKind::Folder, 0, 0)));
        assert!(refused(Change::RemoveFolder, None));
        assert!(refused(Change::RemoveFolder, found(EntryKind::File, 1, 1)));
        assert!(refused(Change::MakeFolder, found(EntryKind::Folder, 1, 0)));
        assert!(refused(Change::MakeFolder, found(EntryKind::Folder, 0, 1)));
        assert!(refused(
            Change::MakeFolder,
            found(EntryKind::ProjectLink, 1, 0)
        ));
        assert!(!refused(
            Change::MakeFolder,
            found(EntryKind::Symlink, 1, 1)
        ));
    }

    #[test]
    fn a_journal_naming_a_path_outside_the_rules_is_refused() {
        let with = |path: &str| {
            Journal::decode(
                &Journal {
                    actions: vec![action(path, Change::File(file(1, 1)), None)],
                    ..sample()
                }
                .encode(),
            )
        };
        let longest = "a/".repeat(2047) + "bc";
        assert_eq!(longest.len(), MAX_PATH_BYTES);
        assert!(with(&longest).is_ok());
        for bad in [
            "..".to_owned(),
            "a/../b".into(),
            ".".into(),
            ".bolt/x".into(),
            "a/.bolt".into(),
            ".BOLT/x".into(),
            ".Bolt./x".into(),
            "BOLT~1/x".into(),
            "/a".into(),
            "a/".into(),
            "a//b".into(),
            String::new(),
            "a\0b".into(),
            longest.clone() + "d",
        ] {
            assert!(with(&bad).is_err(), "{bad:?}");
        }
    }

    fn project() -> (tempfile::TempDir, Repository) {
        let dir = tempfile::tempdir().unwrap();
        let who = Person {
            name: "Ada".into(),
            email: "ada@example.com".into(),
            time_ms: 0,
            tz_offset_min: 0,
        };
        let repo = Repository::init(dir.path(), &who).unwrap();
        (dir, repo)
    }

    #[test]
    fn a_journal_is_written_read_and_removed() {
        let (dir, mut repo) = project();
        let lock = repo.lock_for_change("goto", &mut |_| {}).unwrap();
        assert_eq!(Journal::read(&repo).unwrap(), None);
        sample().write(&repo, &lock).unwrap();
        assert_eq!(Journal::read(&repo).unwrap(), Some(sample()));
        let on_disk = dir.path().join(".bolt/state/journal");
        assert_eq!(std::fs::read(&on_disk).unwrap(), sample().encode());
        // Nothing left behind in `tmp/`.
        let tmp = std::fs::read_dir(dir.path().join(".bolt/tmp")).unwrap();
        assert_eq!(tmp.count(), 0);

        let mut unsorted = sample();
        unsorted.actions.reverse();
        assert!(unsorted.write(&repo, &lock).is_err());
        assert_eq!(Journal::read(&repo).unwrap(), Some(sample()));

        let mut bytes = std::fs::read(&on_disk).unwrap();
        bytes[20] ^= 1;
        std::fs::write(&on_disk, bytes).unwrap();
        assert!(matches!(
            Journal::read(&repo),
            Err(RepoError::JournalUnusable { .. })
        ));

        Journal::remove(&repo, &lock).unwrap();
        assert_eq!(Journal::read(&repo).unwrap(), None);
        Journal::remove(&repo, &lock).unwrap();
    }

    #[test]
    fn a_huge_journal_is_read_only_as_far_as_its_front() {
        let (dir, repo) = project();
        let on_disk = dir.path().join(".bolt/state/journal");
        assert_eq!(Journal::read_front(&repo).unwrap(), None);
        // A good front, then a hole far bigger than the front: only the front is read.
        let mut front = sample().encode();
        front.truncate(FRONT_BYTES as usize);
        std::fs::write(&on_disk, &front).unwrap();
        let file = std::fs::OpenOptions::new()
            .write(true)
            .open(&on_disk)
            .unwrap();
        file.set_len(64 << 20).unwrap();
        drop(file);
        let s = sample();
        assert_eq!(
            Journal::read_front(&repo).unwrap(),
            Some((s.target, s.from))
        );
        // Read whole, it is refused.
        assert!(Journal::read(&repo).is_err());
        // All zeros, as a planted sparse file is: refused from its first bytes.
        std::fs::write(&on_disk, []).unwrap();
        let file = std::fs::OpenOptions::new()
            .write(true)
            .open(&on_disk)
            .unwrap();
        file.set_len(64 << 20).unwrap();
        drop(file);
        assert!(matches!(
            Journal::read_front(&repo),
            Err(RepoError::JournalUnusable { .. })
        ));
    }

    fn id() -> impl Strategy<Value = Id> {
        any::<[u8; 32]>().prop_map(Id)
    }

    fn content() -> impl Strategy<Value = Content> {
        (id(), any::<u64>()).prop_map(|(id, size)| Content { id, size })
    }

    fn expected(kinds: Vec<EntryKind>) -> impl Strategy<Value = Expected> {
        (prop::sample::select(kinds), id(), any::<u64>()).prop_map(|(kind, id, size)| {
            if kind == EntryKind::Folder {
                Expected {
                    kind,
                    id: Id([0; 32]),
                    size: 0,
                }
            } else {
                Expected { kind, id, size }
            }
        })
    }

    fn change_and_expected() -> impl Strategy<Value = (Change, Option<Expected>)> {
        let any_kind = vec![
            EntryKind::File,
            EntryKind::Executable,
            EntryKind::Symlink,
            EntryKind::Folder,
        ];
        let files = vec![EntryKind::File, EntryKind::Executable, EntryKind::Symlink];
        let maybe = || prop::option::of(expected(any_kind.clone()));
        prop_oneof![
            (content(), maybe()).prop_map(|(c, e)| (Change::File(c), e)),
            (content(), maybe()).prop_map(|(c, e)| (Change::Executable(c), e)),
            (content(), maybe()).prop_map(|(c, e)| (Change::Link(c), e)),
            maybe().prop_map(|e| (Change::MakeFolder, e)),
            expected(files).prop_map(|e| (Change::RemoveFile, Some(e))),
            expected(vec![EntryKind::Folder]).prop_map(|e| (Change::RemoveFolder, Some(e))),
        ]
    }

    proptest! {
        #[test]
        fn round_trips(
            target in id(),
            from in id(),
            actions in prop::collection::btree_map(
                "[a-zé.]{1,6}(/[a-zé.]{1,6}){0,3}",
                change_and_expected(),
                0..20,
            ),
        ) {
            let actions: Vec<Action> = actions
                .into_iter()
                .filter(|(path, _)| validate_path(path).is_ok())
                .map(|(path, (change, expected))| Action { path, change, expected })
                .collect();
            let journal = Journal { target, from, actions };
            let bytes = journal.encode();
            let back = Journal::decode(&bytes).unwrap();
            prop_assert_eq!(&back, &journal);
            prop_assert_eq!(back.encode(), bytes);
        }
    }
}
