//! The change-ID cache, `.bolt/cache/change-ids` (storage-format.md section 10): each line's
//! saves in order, each with its change ID, so that shortening a change ID or finding a save
//! needn't read the line. Only ever a hint: one that is missing, sealed by another key, edited or
//! unreadable is rebuilt from the store.
#![deny(clippy::indexing_slicing, clippy::arithmetic_side_effects)]

use std::collections::BTreeMap;
use std::ffi::OsStr;

use crate::diary::{Position, State};
use crate::encoding::{DecodeError, Reader, Writer};
use crate::fsio;
use crate::id::{ChangeId, Id};
use crate::keyed;
use crate::repo::{RepoError, Repository};
use crate::safe_fs;

const MAGIC: &[u8; 8] = b"BOLTCHID";
const VERSION: u8 = 2;
const FILE: &str = "change-ids";
/// The most the file may hold, seal included; a cache that would be bigger isn't written.
pub const MAX_BYTES: u64 = 256 << 20;
/// As many lines as a diary entry may have.
const MAX_LINES: u64 = 100_000;
const MAX_LINE_NAME: usize = 255;
/// A save's ID and its change ID.
const SAVE_BYTES: usize = size_of::<Id>() + size_of::<ChangeId>();
/// The list kept for the project's position when it is at a save; no line has this name.
const AT_SAVE: &str = "";

/// For each line, its saves from the newest back along first parents, each with its change ID.
#[derive(Debug, Default, Clone)]
pub struct ChangeIds {
    lines: BTreeMap<String, Vec<(Id, ChangeId)>>,
    /// Whether it differs from the file it was read from.
    changed: bool,
}

impl ChangeIds {
    /// The cache's body, without its seal.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        self.encode_kept(|_| true)
    }

    fn encode_kept(&self, keep: impl Fn(&str) -> bool) -> Vec<u8> {
        let mut w = Writer::new();
        for b in MAGIC {
            w.u8(*b);
        }
        w.u8(VERSION);
        let kept: Vec<_> = self.lines.iter().filter(|(name, _)| keep(name)).collect();
        w.varint(kept.len() as u64);
        for (name, saves) in kept {
            w.string(name);
            w.varint(saves.len() as u64);
            for (id, change_id) in saves {
                w.id(id);
                w.change_id(change_id);
            }
        }
        w.into_bytes()
    }

    /// Reads a body `encode` wrote. Lines must be in order of name and none empty.
    pub fn decode(body: &[u8]) -> Result<ChangeIds, DecodeError> {
        let rest = body
            .strip_prefix(MAGIC.as_slice())
            .ok_or(DecodeError::Invalid("not a change-ID cache"))?;
        let mut r = Reader::new(rest);
        if r.u8()? != VERSION {
            return Err(DecodeError::Invalid("change-ID cache version"));
        }
        let count = r.count(MAX_LINES, "line count")?;
        let mut lines = BTreeMap::new();
        let mut last: Option<&str> = None;
        for _ in 0..count {
            let name = r.string(MAX_LINE_NAME, "line name")?;
            if last.is_some_and(|before| before >= name) {
                return Err(DecodeError::Invalid("lines out of order"));
            }
            last = Some(name);
            // A count past what is left is refused before allocating.
            let fits = r.remaining().checked_div(SAVE_BYTES).unwrap_or(0);
            let n = usize::try_from(r.varint()?)
                .ok()
                .filter(|&n| n <= fits)
                .ok_or(DecodeError::UnexpectedEnd)?;
            if n == 0 {
                return Err(DecodeError::Invalid("a line with no saves"));
            }
            let mut saves = Vec::with_capacity(n);
            for _ in 0..n {
                saves.push((r.id()?, r.change_id()?));
            }
            lines.insert(name.to_owned(), saves);
        }
        r.finish()?;
        Ok(ChangeIds {
            lines,
            changed: false,
        })
    }

    /// The saves the cache holds for `line`, newest first.
    #[must_use]
    pub fn get(&self, line: &str) -> Option<&[(Id, ChangeId)]> {
        self.lines.get(line).map(Vec::as_slice)
    }

    /// The cache in `repo`, or an empty one if it can't be read or the handle's key didn't seal
    /// it, or the handle has none.
    pub(crate) fn load(repo: &Repository) -> ChangeIds {
        let Some(key) = repo.cache_key() else {
            return ChangeIds::default();
        };
        let shown = repo.cache_dir().join(FILE);
        let read =
            safe_fs::read_capped_in(repo.cache_folder(), OsStr::new(FILE), &shown, MAX_BYTES);
        read.ok()
            .and_then(|bytes| ChangeIds::decode(keyed::open_sealed(key, &bytes)?).ok())
            .unwrap_or_default()
    }

    /// Writes the cache back, sealed with the handle's key, if it changed, keeping only the lines
    /// `state` has and, at a save, the position's list; nothing when the handle has no key. A
    /// cache that can't be written is just not there next time, so a failure is ignored.
    pub(crate) fn store(&mut self, repo: &Repository, state: &State) {
        let Some(key) = repo.cache_key() else {
            return;
        };
        let at_save = matches!(state.position, Position::AtSave(_));
        let keep = |name: &str| state.lines.contains_key(name) || (at_save && name == AT_SAVE);
        if !self.changed && self.lines.keys().all(|name| keep(name)) {
            return;
        }
        let sealed = keyed::seal(key, &self.encode_kept(keep));
        if sealed.len() as u64 > MAX_BYTES {
            return;
        }
        // Readers don't hold the lock, so another command may clear `tmp/` under this write; it
        // then fails, which is fine.
        let _ = fsio::write_cache(repo.tmp_folder(), repo.cache_folder(), FILE, &sealed);
        self.changed = false;
    }

    /// The saves from `tip` back along first parents, newest first, each with its change ID.
    /// `line` is the line whose newest save `tip` is, or `None` for a save the project is at.
    /// The store is read only for saves no list here holds in front of the ones it does.
    pub(crate) fn chain(
        &mut self,
        repo: &Repository,
        line: Option<&str>,
        tip: Id,
    ) -> Result<&[(Id, ChangeId)], RepoError> {
        let name = line.unwrap_or(AT_SAVE);
        let own = self.lines.get(name).and_then(|saves| at(saves, &tip));
        match own {
            Some(0) => {}
            // Moved back: what was in front is no longer on it.
            Some(n) => {
                if let Some(saves) = self.lines.get_mut(name) {
                    saves.drain(..n);
                }
                self.changed = true;
            }
            None => {
                let held = self
                    .lines
                    .iter()
                    .find_map(|(other, saves)| Some((other.clone(), at(saves, &tip)?)));
                match (line, held) {
                    // What a line holds behind a save is what any line holds behind it.
                    (None, Some((other, n))) => {
                        return Ok(self.tail(&other, n));
                    }
                    (Some(_), Some((other, n))) => {
                        let tail = self.tail(&other, n).to_vec();
                        self.lines.insert(name.to_owned(), tail);
                        self.changed = true;
                    }
                    (_, None) => self.read(repo, name, tip)?,
                }
            }
        }
        Ok(self.tail(name, 0))
    }

    fn tail(&self, name: &str, from: usize) -> &[(Id, ChangeId)] {
        self.lines
            .get(name)
            .and_then(|saves| saves.get(from..))
            .unwrap_or_default()
    }

    /// Reads the saves from `tip` back until the first one `name`'s list holds, and puts them in
    /// front of it; the whole line if they never meet it.
    fn read(&mut self, repo: &Repository, name: &str, tip: Id) -> Result<(), RepoError> {
        let first = self
            .lines
            .get(name)
            .and_then(|saves| saves.first())
            .map(|(id, _)| *id);
        let mut found = Vec::new();
        let mut next = Some(tip);
        while let Some(id) = next {
            if first == Some(id) {
                found.extend(self.lines.remove(name).unwrap_or_default());
                break;
            }
            let save = repo.store().save(&id, repo.signatures_allowed())?;
            found.push((id, save.change_id));
            next = save.parents.first().copied();
        }
        self.lines.insert(name.to_owned(), found);
        self.changed = true;
        Ok(())
    }

    /// Whether a line of `state` reaches `id` along first parents, by the lists that start at
    /// the line's newest save. `false` says nothing: the cache may not know the line.
    pub(crate) fn reaches(&self, state: &State, id: &Id) -> bool {
        state.lines.iter().any(|(name, tip)| {
            self.lines.get(name).is_some_and(|saves| {
                saves.first().is_some_and(|(first, _)| first == tip) && at(saves, id).is_some()
            })
        })
    }

    /// Records the save `tip`, with `change_id`, made on `parent`, the newest save of `name`.
    /// Nothing changes unless the cache knows the line at `parent`.
    pub(crate) fn add(&mut self, name: &str, parent: Id, tip: Id, change_id: ChangeId) {
        let Some(saves) = self.known_at(name, parent) else {
            return;
        };
        saves.insert(0, (tip, change_id));
        self.changed = true;
    }

    /// Records that `tip`, which has the change ID and parents of `old`, took the place of `old`
    /// as the newest save of `name`. Nothing changes unless the cache knows the line at `old`.
    pub(crate) fn replace(&mut self, name: &str, old: Id, tip: Id) {
        let Some(first) = self.known_at(name, old).and_then(|saves| saves.first_mut()) else {
            return;
        };
        first.0 = tip;
        self.changed = true;
    }

    #[cfg(test)]
    pub(crate) fn set(&mut self, name: &str, saves: Vec<(Id, ChangeId)>) {
        self.lines.insert(name.to_owned(), saves);
        self.changed = true;
    }

    /// `name`'s list, if its newest save is `tip`.
    fn known_at(&mut self, name: &str, tip: Id) -> Option<&mut Vec<(Id, ChangeId)>> {
        self.lines
            .get_mut(name)
            .filter(|saves| saves.first().is_some_and(|(id, _)| *id == tip))
    }
}

/// Where `id` is in `saves`.
fn at(saves: &[(Id, ChangeId)], id: &Id) -> Option<usize> {
    saves.iter().position(|(s, _)| s == id)
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

    fn save(n: u8) -> (Id, ChangeId) {
        (Id([n; 32]), ChangeId([n; 16]))
    }

    fn with(lines: &[(&str, Vec<(Id, ChangeId)>)]) -> ChangeIds {
        let mut ids = ChangeIds::default();
        for (name, saves) in lines {
            ids.set(name, saves.clone());
        }
        ids
    }

    fn sample() -> ChangeIds {
        with(&[
            ("", vec![save(2)]),
            ("main", vec![save(3), save(2), save(1)]),
            ("side", vec![save(4)]),
        ])
    }

    #[test]
    fn a_cache_reads_back_as_written() {
        let body = sample().encode();
        let back = ChangeIds::decode(&body).unwrap();
        assert_eq!(back.encode(), body);
        assert_eq!(back.get("main").unwrap(), [save(3), save(2), save(1)]);
        assert_eq!(back.get("").unwrap(), [save(2)]);
        assert!(back.get("other").is_none());
    }

    #[test]
    fn a_cache_that_breaks_its_rules_is_refused() {
        let body = sample().encode();
        // Every cut short.
        for len in 0..body.len() {
            assert!(ChangeIds::decode(&body[..len]).is_err(), "{len}");
        }
        let mut longer = body.clone();
        longer.push(0);
        assert!(ChangeIds::decode(&longer).is_err());
        // The empty name is first: magic, version, line count, the name, then its save count.
        let count_at = 8 + 1 + 1 + 1;
        assert_eq!(body[count_at], 1);
        let mut huge = body[..count_at].to_vec();
        huge.extend_from_slice(&[0xff, 0xff, 0xff, 0xff, 0x0f]);
        assert!(ChangeIds::decode(&huge).is_err());
        // One save more than the bytes left hold.
        let mut past = body.clone();
        past[count_at] = u8::try_from((body.len() - count_at - 1) / SAVE_BYTES + 1).unwrap();
        assert!(ChangeIds::decode(&past).is_err());
        let empty = with(&[("main", vec![])]).encode();
        assert!(ChangeIds::decode(&empty).is_err());
        for names in [["b", "a"], ["a", "a"]] {
            let mut bad = ChangeIds::default().encode();
            bad[9] = 2;
            for name in names {
                bad.push(1);
                bad.extend_from_slice(name.as_bytes());
                bad.push(1);
                bad.extend_from_slice(&[0; SAVE_BYTES]);
            }
            assert!(ChangeIds::decode(&bad).is_err(), "{names:?}");
        }
        let mut version = body;
        version[8] = 1;
        assert!(ChangeIds::decode(&version).is_err());
    }

    #[test]
    fn a_save_is_added_only_on_top_of_what_the_cache_knows() {
        let mut ids = sample();
        ids.add("main", Id([7; 32]), Id([8; 32]), ChangeId([5; 16]));
        assert_eq!(ids.get("main").unwrap().len(), 3);
        ids.add("main", Id([3; 32]), Id([8; 32]), ChangeId([5; 16]));
        let got = ids.get("main").unwrap();
        assert_eq!(got[0], (Id([8; 32]), ChangeId([5; 16])));
        assert_eq!(&got[1..], [save(3), save(2), save(1)]);
        ids.add("new", Id([3; 32]), Id([8; 32]), ChangeId([5; 16]));
        assert!(ids.get("new").is_none());
    }

    #[test]
    fn a_replaced_save_keeps_the_line_s_change_ids() {
        let mut ids = sample();
        ids.replace("main", Id([7; 32]), Id([8; 32]));
        assert_eq!(ids.get("main").unwrap()[0], save(3));
        ids.replace("main", Id([3; 32]), Id([8; 32]));
        let got = ids.get("main").unwrap();
        assert_eq!(got[0], (Id([8; 32]), ChangeId([3; 16])));
        assert_eq!(&got[1..], [save(2), save(1)]);
        ids.replace("new", Id([3; 32]), Id([8; 32]));
        assert!(ids.get("new").is_none());
    }
}
