//! The other-lines cache, `.opalro/cache/other-lines` (storage-format.md section 10): what the check
//! before an amend read of the lines the project isn't on, so that the next amend needn't read
//! them again. Only ever a hint: one that is missing, sealed by another key, edited or unreadable
//! is just not used.
#![deny(clippy::indexing_slicing, clippy::arithmetic_side_effects)]

use std::collections::{BTreeMap, HashSet};
use std::ffi::OsStr;

use crate::diary::State;
use crate::encoding::{DecodeError, Reader, Writer, varint_len};
use crate::fsio;
use crate::id::{ChangeId, Id};
use crate::keyed;
use crate::repo::Repository;
use crate::safe_fs;

const MAGIC: &[u8; 8] = b"OPALLINE";
const VERSION: u8 = 1;
const FILE: &str = "other-lines";
/// The most the file may hold, seal included.
pub const MAX_BYTES: u64 = 64 << 20;
const MAX_RUNS: u64 = 100_000;
/// A save's ID and its change ID.
const SAVE_BYTES: usize = size_of::<Id>() + size_of::<ChangeId>();

/// Saves read from a line's newest back along first parents, and the save that comes next.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Run {
    /// Newest first, each with its change ID; never empty.
    pub saves: Vec<(Id, ChangeId)>,
    /// The last save's first parent; `None` if it has none.
    pub next: Option<Id>,
}

/// Runs keyed by their first save.
#[derive(Debug, Default, Clone)]
pub struct OtherLines {
    runs: BTreeMap<Id, Run>,
    /// What the runs take encoded, without the magic, version, run count and seal.
    bytes: u64,
    /// Whether it differs from the file it was read from.
    changed: bool,
}

impl OtherLines {
    /// The cache's body, without its seal.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        self.encode_into(Writer::new())
    }

    fn encode_into(&self, mut w: Writer) -> Vec<u8> {
        for b in MAGIC {
            w.u8(*b);
        }
        w.u8(VERSION);
        w.varint(self.runs.len() as u64);
        for run in self.runs.values() {
            w.varint(run.saves.len() as u64);
            for (id, change_id) in &run.saves {
                w.id(id);
                w.change_id(change_id);
            }
            match &run.next {
                Some(next) => {
                    w.u8(1);
                    w.id(next);
                }
                None => w.u8(0),
            }
        }
        w.into_bytes()
    }

    /// Reads a body `encode` wrote. Runs must be in order of their first save and none empty.
    pub fn decode(body: &[u8]) -> Result<OtherLines, DecodeError> {
        let rest = body
            .strip_prefix(MAGIC.as_slice())
            .ok_or(DecodeError::Invalid("not an other-lines cache"))?;
        let mut r = Reader::new(rest);
        if r.u8()? != VERSION {
            return Err(DecodeError::Invalid("other-lines cache version"));
        }
        let count = r.count(MAX_RUNS, "run count")?;
        let mut lines = OtherLines::default();
        let mut last: Option<Id> = None;
        for _ in 0..count {
            // A count past what is left is refused before allocating.
            let fits = r.remaining().checked_div(SAVE_BYTES).unwrap_or(0);
            let n = usize::try_from(r.varint()?)
                .ok()
                .filter(|&n| n <= fits)
                .ok_or(DecodeError::UnexpectedEnd)?;
            if n == 0 {
                return Err(DecodeError::Invalid("a run with no saves"));
            }
            let mut saves = Vec::with_capacity(n);
            for _ in 0..n {
                saves.push((r.id()?, r.change_id()?));
            }
            let next = match r.u8()? {
                0 => None,
                1 => Some(r.id()?),
                _ => return Err(DecodeError::Invalid("a run's next save flag")),
            };
            let first = saves
                .first()
                .map(|(id, _)| *id)
                .ok_or(DecodeError::Invalid("a run with no saves"))?;
            if last.is_some_and(|before| before >= first) {
                return Err(DecodeError::Invalid("runs out of order"));
            }
            last = Some(first);
            lines.bytes = lines
                .bytes
                .saturating_add(run_bytes(n as u64, next.is_some()));
            lines.runs.insert(first, Run { saves, next });
        }
        r.finish()?;
        Ok(lines)
    }

    /// The cache in `repo`, or an empty one if it can't be read or the handle's key didn't seal
    /// it, or the handle has none.
    pub(crate) fn load(repo: &Repository) -> OtherLines {
        let Some(key) = repo.cache_key() else {
            return OtherLines::default();
        };
        let shown = repo.cache_dir().join(FILE);
        let read =
            safe_fs::read_capped_in(repo.cache_folder(), OsStr::new(FILE), &shown, MAX_BYTES);
        read.ok()
            .and_then(|bytes| OtherLines::decode(keyed::open_sealed(key, &bytes)?).ok())
            .unwrap_or_default()
    }

    /// Writes the cache back, sealed with the handle's key, if it changed, keeping only the runs
    /// that start at the newest save of a line of `state`. A cache that can't be written is just
    /// not there next time, so a failure is ignored.
    pub(crate) fn store(&mut self, repo: &Repository, state: &State) {
        let Some(key) = repo.cache_key() else {
            return;
        };
        let tips: HashSet<&Id> = state.lines.values().collect();
        let bytes = &mut self.bytes;
        let held = self.runs.len();
        self.runs.retain(|first, run| {
            let keep = tips.contains(first);
            if !keep {
                *bytes =
                    bytes.saturating_sub(run_bytes(run.saves.len() as u64, run.next.is_some()));
            }
            keep
        });
        if self.runs.len() != held {
            self.changed = true;
        }
        if !self.changed {
            return;
        }
        let total = self.total(self.runs.len() as u64, self.bytes);
        if total > MAX_BYTES {
            return;
        }
        let mut buf = Vec::new();
        let Some(size) = usize::try_from(total).ok() else {
            return;
        };
        if buf.try_reserve_exact(size).is_err() {
            return;
        }
        let mut buf = self.encode_into(Writer::from_vec(buf));
        keyed::seal_in_place(key, &mut buf);
        // Readers don't hold the lock, so another command may clear `tmp/` under this write; it
        // then fails, which is fine.
        let _ = fsio::write_cache(repo.tmp_folder(), repo.cache_folder(), FILE, &buf);
        self.changed = false;
    }

    /// The run that starts at `tip`.
    #[must_use]
    pub fn run(&self, tip: &Id) -> Option<&Run> {
        self.runs.get(tip)
    }

    /// Drops the run that starts at `tip`, if there is one.
    pub(crate) fn forget(&mut self, tip: &Id) {
        if let Some(run) = self.runs.remove(tip) {
            let bytes = run_bytes(run.saves.len() as u64, run.next.is_some());
            self.bytes = self.bytes.saturating_sub(bytes);
            self.changed = true;
        }
    }

    /// Puts `read`, saves newest first with their change IDs, at the end of the run that starts
    /// at `tip`, or starts that run with them if `read` starts with `tip`, and records `next` as
    /// the save after the last. Keeps only what fits under the cap: then the first save left out
    /// is the one that comes next.
    pub(crate) fn add(&mut self, tip: Id, read: &[(Id, ChangeId)], next: Option<Id>) {
        self.add_within(tip, read, next, MAX_BYTES);
    }

    fn add_within(&mut self, tip: Id, read: &[(Id, ChangeId)], next: Option<Id>, cap: u64) {
        let runs = self.runs.len() as u64;
        let (held, old_bytes, old_next, more_runs) = match self.runs.get(&tip) {
            Some(run) => {
                let held = run.saves.len() as u64;
                (held, run_bytes(held, run.next.is_some()), run.next, runs)
            }
            None => {
                if read.first().map(|(id, _)| *id) != Some(tip) || runs >= MAX_RUNS {
                    return;
                }
                (0, 0, None, runs.saturating_add(1))
            }
        };
        let rest = self.bytes.saturating_sub(old_bytes);
        let next_after = |k: usize| match read.get(k) {
            Some((id, _)) => Some(*id),
            None => next,
        };
        let fits = |k: usize| {
            let saves = held.saturating_add(k as u64);
            let run = run_bytes(saves, next_after(k).is_some());
            self.total(more_runs, rest.saturating_add(run)) <= cap
        };
        // A new run needs at least its first save.
        let least = usize::from(held == 0);
        let Some(k) = (least..=read.len()).rev().find(|&k| fits(k)) else {
            return;
        };
        let new_next = next_after(k);
        if k == 0 && new_next == old_next {
            return;
        }
        let Some(kept) = read.get(..k) else {
            return;
        };
        let run = self.runs.entry(tip).or_insert_with(|| Run {
            saves: Vec::new(),
            next: None,
        });
        if run.saves.try_reserve_exact(k).is_err() {
            if run.saves.is_empty() {
                self.runs.remove(&tip);
            }
            return;
        }
        run.saves.extend_from_slice(kept);
        run.next = new_next;
        let new_bytes = run_bytes(run.saves.len() as u64, new_next.is_some());
        self.bytes = rest.saturating_add(new_bytes);
        self.changed = true;
    }

    /// The file's size with `runs` runs that take `bytes`.
    fn total(&self, runs: u64, bytes: u64) -> u64 {
        (MAGIC.len() as u64)
            .saturating_add(1)
            .saturating_add(varint_len(runs))
            .saturating_add(bytes)
            .saturating_add(keyed::KEY_LEN as u64)
    }
}

/// What a run of `saves` saves takes encoded, with its next save or without.
fn run_bytes(saves: u64, has_next: bool) -> u64 {
    varint_len(saves)
        .saturating_add(saves.saturating_mul(SAVE_BYTES as u64))
        .saturating_add(1)
        .saturating_add(if has_next { size_of::<Id>() as u64 } else { 0 })
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
    use crate::diary::Position;

    fn save(n: u8) -> (Id, ChangeId) {
        (Id([n; 32]), ChangeId([n; 16]))
    }

    fn sample() -> OtherLines {
        let mut lines = OtherLines::default();
        lines.add(Id([3; 32]), &[save(3), save(2)], Some(Id([1; 32])));
        lines.add(Id([5; 32]), &[save(5)], None);
        lines
    }

    fn repo() -> (tempfile::TempDir, Repository) {
        let dir = tempfile::tempdir().unwrap();
        let person = crate::save::Person {
            name: "Ada".into(),
            email: "a@example.com".into(),
            time_ms: 0,
            tz_offset_min: 0,
        };
        let mut repo = Repository::init(dir.path(), &person).unwrap();
        repo.set_cache_key([7; 32]);
        (dir, repo)
    }

    fn state_at(tips: &[u8]) -> State {
        State {
            lines: tips
                .iter()
                .map(|&n| (format!("l{n}"), Id([n; 32])))
                .collect(),
            tags: None,
            position: Position::OnLine("l0".into()),
            shelves: vec![],
            server_lines: None,
        }
    }

    #[test]
    fn a_cache_reads_back_as_written() {
        let lines = sample();
        let body = lines.encode();
        let back = OtherLines::decode(&body).unwrap();
        assert_eq!(back.encode(), body);
        assert_eq!(back.bytes, lines.bytes);
        assert_eq!(body.len() as u64 + 32, lines.total(2, lines.bytes));
        let run = back.run(&Id([3; 32])).unwrap();
        assert_eq!(run.saves, [save(3), save(2)]);
        assert_eq!(run.next, Some(Id([1; 32])));
        assert_eq!(back.run(&Id([5; 32])).unwrap().next, None);
        assert!(back.run(&Id([2; 32])).is_none());
    }

    /// A body of `runs`, each its save count, its saves and its flag and next save as given.
    fn body(runs: &[(u64, &[u8], &[u8])]) -> Vec<u8> {
        let mut w = Writer::new();
        for b in MAGIC {
            w.u8(*b);
        }
        w.u8(VERSION);
        w.varint(runs.len() as u64);
        for (count, saves, tail) in runs {
            w.varint(*count);
            for &n in *saves {
                w.id(&save(n).0);
                w.change_id(&save(n).1);
            }
            for b in *tail {
                w.u8(*b);
            }
        }
        w.into_bytes()
    }

    #[test]
    fn a_cache_that_breaks_its_rules_is_refused() {
        let none: &[u8] = &[0];
        let good = body(&[(1, &[2], none), (1, &[4], none)]);
        assert!(OtherLines::decode(&good).is_ok());
        // Another magic, another version.
        let mut magic = good.clone();
        magic[0] = b'X';
        assert!(OtherLines::decode(&magic).is_err());
        let mut version = good.clone();
        version[8] = 2;
        assert!(OtherLines::decode(&version).is_err());
        // More runs than allowed is refused as such; as many as allowed only runs out of bytes.
        let counted = |runs: u64| {
            let mut w = Writer::from_vec(good[..9].to_vec());
            w.varint(runs);
            OtherLines::decode(&w.into_bytes())
        };
        assert!(matches!(
            counted(MAX_RUNS + 1),
            Err(DecodeError::LimitExceeded { .. })
        ));
        assert!(matches!(counted(MAX_RUNS), Err(DecodeError::UnexpectedEnd)));
        // A run of no saves.
        assert!(OtherLines::decode(&body(&[(0, &[], none)])).is_err());
        // A count the bytes left can't hold.
        assert!(OtherLines::decode(&body(&[(2, &[2], none)])).is_err());
        // Out of order, and a first save twice.
        assert!(OtherLines::decode(&body(&[(1, &[4], none), (1, &[2], none)])).is_err());
        assert!(OtherLines::decode(&body(&[(1, &[2], none), (1, &[2], none)])).is_err());
        // A flag other than 0 or 1, and a 1 with its save.
        assert!(OtherLines::decode(&body(&[(1, &[2], &[2])])).is_err());
        let mut next = vec![1];
        next.extend_from_slice(&[9; 32]);
        let flagged = body(&[(1, &[2], &next)]);
        assert_eq!(
            OtherLines::decode(&flagged)
                .unwrap()
                .run(&Id([2; 32]))
                .unwrap()
                .next,
            Some(Id([9; 32]))
        );
        // Bytes left over.
        let mut longer = good;
        longer.push(0);
        assert!(OtherLines::decode(&longer).is_err());
    }

    #[test]
    fn saves_past_the_cap_are_left_for_the_next_read() {
        let full = |saves: u64, next: bool| OtherLines::default().total(1, run_bytes(saves, next));
        // Room for a run of two saves and its next save, not three.
        let cap = full(2, true);
        let mut lines = OtherLines::default();
        lines.add_within(Id([9; 32]), &[save(9), save(8), save(7)], None, cap);
        let run = lines.run(&Id([9; 32])).unwrap();
        assert_eq!(run.saves, [save(9), save(8)]);
        assert_eq!(run.next, Some(Id([7; 32])));
        assert!(lines.total(1, lines.bytes) <= cap);
        // Nothing more fits; the run stays as it was.
        lines.add_within(Id([9; 32]), &[save(7), save(6)], None, cap);
        assert_eq!(lines.run(&Id([9; 32])).unwrap().saves.len(), 2);
        // Room for exactly three with no next save: all kept.
        let mut lines = OtherLines::default();
        lines.add_within(
            Id([9; 32]),
            &[save(9), save(8), save(7)],
            None,
            full(3, false),
        );
        assert_eq!(lines.run(&Id([9; 32])).unwrap().next, None);
        assert_eq!(lines.run(&Id([9; 32])).unwrap().saves.len(), 3);
        // No room for even the first save: no run.
        let mut lines = OtherLines::default();
        lines.add_within(Id([9; 32]), &[save(9)], None, full(1, false) - 1);
        assert!(lines.run(&Id([9; 32])).is_none());
    }

    #[test]
    fn a_run_is_started_only_at_its_tip_and_grows_at_its_end() {
        let mut lines = OtherLines::default();
        lines.add(Id([9; 32]), &[save(8)], None);
        assert!(lines.run(&Id([9; 32])).is_none());
        lines.add(Id([9; 32]), &[], None);
        assert!(lines.run(&Id([9; 32])).is_none());
        lines.add(Id([9; 32]), &[save(9)], Some(Id([8; 32])));
        lines.changed = false;
        lines.add(Id([9; 32]), &[], Some(Id([8; 32])));
        assert!(!lines.changed);
        lines.add(Id([9; 32]), &[save(8), save(7)], None);
        let run = lines.run(&Id([9; 32])).unwrap();
        assert_eq!(run.saves, [save(9), save(8), save(7)]);
        assert_eq!(run.next, None);
        assert_eq!(lines.bytes, run_bytes(3, false));
        lines.changed = false;
        lines.forget(&Id([9; 32]));
        assert!(lines.run(&Id([9; 32])).is_none());
        assert_eq!((lines.bytes, lines.changed), (0, true));
    }

    #[test]
    fn store_keeps_only_runs_a_line_starts_at() {
        let (dir, repo) = repo();
        let file = dir.path().join(".opalro/cache/other-lines");
        let mut lines = sample();
        lines.store(&repo, &state_at(&[5, 6]));
        let back = OtherLines::load(&repo);
        assert!(back.run(&Id([3; 32])).is_none());
        assert!(back.run(&Id([5; 32])).is_some());
        assert_eq!(back.bytes, lines.bytes);
        // Unchanged: not written again.
        std::fs::remove_file(&file).unwrap();
        lines.store(&repo, &state_at(&[5, 6]));
        assert!(!file.exists());
    }

    #[test]
    fn a_cache_sealed_with_another_key_loads_empty() {
        let (dir, repo) = repo();
        let file = dir.path().join(".opalro/cache/other-lines");
        std::fs::write(&file, keyed::seal(&[8; 32], &sample().encode())).unwrap();
        assert!(OtherLines::load(&repo).runs.is_empty());
        std::fs::write(&file, keyed::seal(&[7; 32], &sample().encode())).unwrap();
        assert_eq!(OtherLines::load(&repo).runs.len(), 2);
    }
}
