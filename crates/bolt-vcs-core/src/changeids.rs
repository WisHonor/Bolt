//! The change-ID cache, `.bolt/cache/change-ids` (storage-format.md section 10): the change IDs
//! of each line's saves, sorted, so that shortening one needn't read the line. Only ever a hint:
//! one that is missing, sealed by another key, edited or unreadable is rebuilt from the store.
#![deny(clippy::indexing_slicing, clippy::arithmetic_side_effects)]

use std::collections::BTreeMap;
use std::ffi::OsStr;

use crate::diary::State;
use crate::encoding::{DecodeError, Reader, Writer};
use crate::fsio;
use crate::id::{ChangeId, Id};
use crate::keyed::{self, KEY_LEN};
use crate::repo::{RepoError, Repository};
use crate::safe_fs;

const MAGIC: &[u8; 8] = b"BOLTCHID";
const VERSION: u8 = 1;
const FILE: &str = "change-ids";
/// The most the file may hold, seal included; a cache that would be bigger isn't written.
pub const MAX_BYTES: u64 = 256 << 20;
/// As many lines as a diary entry may have.
const MAX_LINES: u64 = 100_000;
const MAX_LINE_NAME: usize = 255;

/// The sorted change IDs of the saves on each line, from its newest save back along first
/// parents, and which save was newest when they were read.
#[derive(Debug, Default, Clone)]
pub struct ChangeIds {
    lines: BTreeMap<String, Line>,
    /// Whether it differs from the file it was read from.
    changed: bool,
}

#[derive(Debug, Clone)]
struct Line {
    tip: Id,
    ids: Vec<ChangeId>,
}

impl ChangeIds {
    /// The cache's body, without its seal.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        for b in MAGIC {
            w.u8(*b);
        }
        w.u8(VERSION);
        w.varint(self.lines.len() as u64);
        for (name, line) in &self.lines {
            w.string(name);
            w.id(&line.tip);
            w.varint(line.ids.len() as u64);
            for id in &line.ids {
                w.change_id(id);
            }
        }
        w.into_bytes()
    }

    /// Reads a body `encode` wrote. Lines must be in order of name and each line's IDs sorted.
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
            let tip = r.id()?;
            // Each ID takes 16 bytes, so a count past what is left is refused before allocating.
            let fits = r
                .remaining()
                .checked_div(size_of::<ChangeId>())
                .unwrap_or(0);
            let n = usize::try_from(r.varint()?)
                .ok()
                .filter(|&n| n <= fits)
                .ok_or(DecodeError::UnexpectedEnd)?;
            let mut ids = Vec::with_capacity(n);
            for _ in 0..n {
                ids.push(r.change_id()?);
            }
            if !ids.is_sorted() {
                return Err(DecodeError::Invalid("change IDs out of order"));
            }
            lines.insert(name.to_owned(), Line { tip, ids });
        }
        r.finish()?;
        Ok(ChangeIds {
            lines,
            changed: false,
        })
    }

    /// The newest save of `line` when its IDs were read, and the IDs.
    #[must_use]
    pub fn get(&self, line: &str) -> Option<(Id, &[ChangeId])> {
        self.lines.get(line).map(|l| (l.tip, l.ids.as_slice()))
    }

    /// The cache in `repo`, or an empty one if it can't be read or `key` didn't seal it.
    pub(crate) fn load(repo: &Repository, key: &[u8; KEY_LEN]) -> ChangeIds {
        let shown = repo.cache_dir().join(FILE);
        let read =
            safe_fs::read_capped_in(repo.cache_folder(), OsStr::new(FILE), &shown, MAX_BYTES);
        read.ok()
            .and_then(|bytes| ChangeIds::decode(keyed::open_sealed(key, &bytes)?).ok())
            .unwrap_or_default()
    }

    /// Writes the cache back, sealed with `key`, if it changed, keeping only the lines `state`
    /// has. A cache that can't be written is just not there next time, so a failure is ignored.
    pub(crate) fn store(&mut self, repo: &Repository, key: &[u8; KEY_LEN], state: &State) {
        let before = self.lines.len();
        self.lines.retain(|name, _| state.lines.contains_key(name));
        if !self.changed && self.lines.len() == before {
            return;
        }
        let sealed = keyed::seal(key, &self.encode());
        if sealed.len() as u64 > MAX_BYTES {
            return;
        }
        // Readers don't hold the lock, so another command may clear `tmp/` under this write; it
        // then fails, which is fine.
        let _ = fsio::write_cache(repo.tmp_folder(), repo.cache_folder(), FILE, &sealed);
        self.changed = false;
    }

    /// The sorted change IDs of the saves on the line `name`, whose newest save is `tip`. The store
    /// is read from `tip` back to the newest save the cache knows on that line, or to the line's
    /// first save if the cache knows none of them (an undo moved the line back, say).
    pub(crate) fn line(
        &mut self,
        repo: &Repository,
        name: &str,
        tip: Id,
    ) -> Result<&[ChangeId], RepoError> {
        let known = self.lines.get(name).map(|l| l.tip);
        if known != Some(tip) {
            let mut found = Vec::new();
            let mut next = Some(tip);
            while let Some(id) = next {
                if known == Some(id) {
                    found.extend(self.lines.remove(name).map(|l| l.ids).unwrap_or_default());
                    break;
                }
                let save = repo.store().save(&id, repo.signatures_allowed())?;
                found.push(save.change_id);
                next = save.parents.first().copied();
            }
            self.set(name, tip, found);
        }
        Ok(self.get(name).map_or(&[], |(_, ids)| ids))
    }

    /// Records that `ids` are the change IDs of the line `name` whose newest save is `tip`.
    pub(crate) fn set(&mut self, name: &str, tip: Id, mut ids: Vec<ChangeId>) {
        ids.sort_unstable();
        self.lines.insert(name.to_owned(), Line { tip, ids });
        self.changed = true;
    }

    /// Records the save `tip`, with `change_id`, made on `parent`, the newest save of `name`.
    /// Nothing changes unless the cache knows the line at `parent`.
    pub(crate) fn add(&mut self, name: &str, parent: Id, tip: Id, change_id: ChangeId) {
        let Some(line) = self.lines.get_mut(name).filter(|l| l.tip == parent) else {
            return;
        };
        let at = line.ids.partition_point(|c| *c < change_id);
        line.ids.insert(at, change_id);
        line.tip = tip;
        self.changed = true;
    }
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

    fn sample() -> ChangeIds {
        let mut ids = ChangeIds::default();
        ids.set(
            "main",
            Id([1; 32]),
            vec![ChangeId([9; 16]), ChangeId([3; 16]), ChangeId([3; 16])],
        );
        ids.set("side", Id([2; 32]), vec![]);
        ids
    }

    #[test]
    fn a_cache_reads_back_as_written() {
        let body = sample().encode();
        let back = ChangeIds::decode(&body).unwrap();
        assert_eq!(back.encode(), body);
        let (tip, ids) = back.get("main").unwrap();
        assert_eq!(tip, Id([1; 32]));
        assert_eq!(
            ids,
            [ChangeId([3; 16]), ChangeId([3; 16]), ChangeId([9; 16])]
        );
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
        // `main` is first: magic, version, line count, then "main" and its tip.
        let count_at = 8 + 1 + 1 + 5 + 32;
        let mut huge = body[..count_at].to_vec();
        huge.extend_from_slice(&[0xff, 0xff, 0xff, 0xff, 0x0f]);
        assert!(ChangeIds::decode(&huge).is_err());
        let mut unsorted = body.clone();
        unsorted[count_at + 1] = 0xff;
        assert!(ChangeIds::decode(&unsorted).is_err());
        let mut swapped = ChangeIds::default().encode();
        swapped[9] = 2;
        for name in ["b", "a"] {
            swapped.push(1);
            swapped.extend_from_slice(name.as_bytes());
            swapped.extend_from_slice(&[0; 33]);
        }
        assert!(ChangeIds::decode(&swapped).is_err());
        let mut version = body;
        version[8] = 2;
        assert!(ChangeIds::decode(&version).is_err());
    }

    #[test]
    fn a_save_is_added_only_on_top_of_what_the_cache_knows() {
        let mut ids = sample();
        ids.add("main", Id([7; 32]), Id([8; 32]), ChangeId([5; 16]));
        assert_eq!(ids.get("main").unwrap().0, Id([1; 32]));
        ids.add("main", Id([1; 32]), Id([8; 32]), ChangeId([5; 16]));
        let (tip, got) = ids.get("main").unwrap();
        assert_eq!(tip, Id([8; 32]));
        assert_eq!(got.iter().map(|c| c.0[0]).collect::<Vec<_>>(), [3, 3, 5, 9]);
        ids.add("new", Id([1; 32]), Id([8; 32]), ChangeId([5; 16]));
        assert!(ids.get("new").is_none());
    }
}
