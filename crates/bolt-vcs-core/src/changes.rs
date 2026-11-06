//! `bolt changes`: what changed inside files, line by line (decision 058).

use std::ffi::OsStr;
use std::io::{self, Read, Write};
use std::path::Path;
use std::time::{Duration, Instant};

use similar::{Algorithm, DiffOp, DiffTag, capture_diff_slices_deadline, group_diff_ops};

use crate::diff::{ChangeKind, for_each_change, for_each_change_in};
use crate::history::tip_of;
use crate::id::Id;
use crate::listing::EntryKind;
use crate::repo::Repository;
use crate::scope::Scope;
use crate::snapshot::{SaveError, link_target, read_folder};
use crate::status::PathChange;
use crate::store::Store;
use crate::writeout::{self, Folders};

/// A file bigger than this shows without lines, and isn't read.
pub const MAX_LINES_SIZE: u64 = 64 << 20;
/// A file with a zero byte this near its start is binary, as Git decides.
const BINARY_CHECK: usize = 8000;
/// How long one file's diff may take.
pub const DEADLINE: Duration = Duration::from_secs(2);
const CONTEXT: usize = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tag {
    Same,
    Removed,
    Added,
}

/// A run of changed lines with the lines around them, as in a unified diff. A start is the first
/// line's number, from 1, or the line before for a side with no lines.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hunk {
    pub old_start: u64,
    pub old_lines: u64,
    pub new_start: u64,
    pub new_lines: u64,
    /// Each line with its ending (`\n`, `\r\n`, or none for a last line without one).
    pub lines: Vec<(Tag, Vec<u8>)>,
}

/// What changed inside one file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Lines {
    Binary,
    TooBig,
    /// `timed_out` when the diff ran past its deadline, so its hunks are coarser than they could
    /// be.
    Hunks {
        hunks: Vec<Hunk>,
        timed_out: bool,
    },
}

/// The changed lines between `old` and `new`, with 3 lines of context. Binary when either side
/// has a zero byte in its first 8,000.
#[must_use]
pub fn file_lines(old: &[u8], new: &[u8], deadline: Duration) -> Lines {
    if is_binary(old) || is_binary(new) {
        return Lines::Binary;
    }
    let (old, new) = (lines_of(old), lines_of(new));
    let start = Instant::now();
    let ops =
        capture_diff_slices_deadline(Algorithm::Myers, &old, &new, start.checked_add(deadline));
    // `similar` doesn't say whether it gave up: one that ran past its deadline did.
    let timed_out = start.elapsed() >= deadline;
    let hunks = group_diff_ops(ops, CONTEXT)
        .iter()
        .map(|group| hunk(group, &old, &new))
        .collect();
    Lines::Hunks { hunks, timed_out }
}

/// Whether each hunk only turns CRLF into LF, or LF into CRLF: what it removes is what it adds
/// once each `\r\n` is read as `\n`.
#[must_use]
pub fn only_line_endings(hunks: &[Hunk]) -> bool {
    let lf = |line: &[u8]| -> Vec<u8> {
        match line.strip_suffix(b"\r\n") {
            Some(body) => [body, b"\n"].concat(),
            None => line.to_vec(),
        }
    };
    !hunks.is_empty()
        && hunks.iter().all(|h| {
            let side = |tag| {
                h.lines
                    .iter()
                    .filter(move |(t, _)| *t == tag)
                    .map(|(_, line)| lf(line))
            };
            side(Tag::Removed).eq(side(Tag::Added))
        })
}

/// Hands `each` every unsaved change, as `status` finds them, with its lines: the folder against
/// the save the project is on. Only `paths` and what is in them, if given. Nothing is stored;
/// only the file-state cache is written. The folder walk's warnings go to the store's.
pub fn unsaved_changes(
    repo: &Repository,
    paths: Option<&[String]>,
    git_excludes: Option<&Path>,
    each: &mut dyn FnMut(&str, &PathChange, Lines) -> Result<(), SaveError>,
) -> Result<(), SaveError> {
    let store = repo.store();
    let (_, entry) = repo.current()?;
    let save = store.save(&tip_of(&entry)?, repo.signatures_allowed())?;
    let now = read_folder(repo, save.root, false, git_excludes)?;
    for w in now
        .warnings
        .into_iter()
        .chain(writeout::unfinished_warning(repo))
    {
        store.note(w);
    }
    let scope = scope(repo, paths)?;
    let mut disk = Folders::open(repo.root()).map_err(|e| unreadable("the project folder", e))?;
    for_each_change_in(store, &now.listings, Some(save.root), now.root, |c| {
        if !scope.as_ref().is_none_or(|s| s.holds(c.path)) {
            return Ok(());
        }
        let c = PathChange::from(c);
        let old = stored(store, c.old_kind, c.old_id, c.old_size)?;
        let new = on_disk(&mut disk, &c)?;
        each(&c.path, &c, lines(old, new))
    })
}

/// Hands `each` every file the save `save` changed, compared with its first parent (every file,
/// for a save without one), with its lines. Only `paths` and what is in them, if given.
pub fn save_changes(
    repo: &Repository,
    save: Id,
    paths: Option<&[String]>,
    each: &mut dyn FnMut(&str, &PathChange, Lines) -> Result<(), SaveError>,
) -> Result<(), SaveError> {
    let store = repo.store();
    let save = store.save(&save, repo.signatures_allowed())?;
    let old_root = match save.parents.first() {
        Some(p) => Some(store.save(p, repo.signatures_allowed())?.root),
        None => None,
    };
    let scope = scope(repo, paths)?;
    for_each_change(store, old_root, save.root, |c| {
        if !scope.as_ref().is_none_or(|s| s.holds(c.path)) {
            return Ok(());
        }
        let c = PathChange::from(c);
        let old = stored(store, c.old_kind, c.old_id, c.old_size)?;
        let new = stored(store, Some(c.kind), c.new_id, c.new_size)?;
        each(&c.path, &c, lines(old, new))
    })
}

fn scope(repo: &Repository, paths: Option<&[String]>) -> Result<Option<Scope>, SaveError> {
    Ok(match paths {
        Some(paths) => Some(Scope::paths(paths, repo.drive()?)),
        None => None,
    })
}

/// One side of a file: what it holds, or too much to read.
enum Side {
    Bytes(Vec<u8>),
    TooBig,
}

fn lines(old: Side, new: Side) -> Lines {
    match (old, new) {
        (Side::Bytes(old), Side::Bytes(new)) => file_lines(&old, &new, deadline()),
        _ => Lines::TooBig,
    }
}

/// How long a file's diff may take; crash tests can shorten it.
fn deadline() -> Duration {
    #[cfg(feature = "fault-injection")]
    if let Some(ms) = std::env::var("BOLT_DIFF_DEADLINE_MS")
        .ok()
        .and_then(|v| v.parse().ok())
    {
        return Duration::from_millis(ms);
    }
    DEADLINE
}

/// Files and links have contents; a project link or conflict shows as empty.
fn has_content(kind: EntryKind) -> bool {
    matches!(
        kind,
        EntryKind::File | EntryKind::Executable | EntryKind::Symlink
    )
}

/// What the store holds for one side, read whole unless it is over the limit. The size a listing
/// gives is checked first, and the read stops past the limit whatever the listing said.
fn stored(
    store: &Store,
    kind: Option<EntryKind>,
    id: Option<Id>,
    size: Option<u64>,
) -> Result<Side, SaveError> {
    let (Some(kind), Some(id)) = (kind, id) else {
        return Ok(Side::Bytes(Vec::new()));
    };
    if !has_content(kind) {
        return Ok(Side::Bytes(Vec::new()));
    }
    if size.unwrap_or(0) > MAX_LINES_SIZE {
        return Ok(Side::TooBig);
    }
    let mut out = Capped::default();
    match store.copy_content(&id, &mut out) {
        Ok(()) => Ok(Side::Bytes(out.bytes)),
        Err(_) if out.over => Ok(Side::TooBig),
        Err(e) => Err(e.into()),
    }
}

/// Bytes, refusing to hold more than the limit.
#[derive(Default)]
struct Capped {
    bytes: Vec<u8>,
    over: bool,
}

impl Write for Capped {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let room = usize::try_from(MAX_LINES_SIZE)
            .unwrap_or(usize::MAX)
            .saturating_sub(self.bytes.len());
        if buf.len() > room {
            self.over = true;
            return Err(io::Error::other("over the size lines are shown for"));
        }
        self.bytes.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// What the folder holds for the new side of an unsaved change.
fn on_disk(disk: &mut Folders, c: &PathChange) -> Result<Side, SaveError> {
    if c.change == ChangeKind::Removed || !has_content(c.kind) {
        return Ok(Side::Bytes(Vec::new()));
    }
    if c.new_size.unwrap_or(0) > MAX_LINES_SIZE {
        return Ok(Side::TooBig);
    }
    let (up, name) = c.path.rsplit_once('/').unwrap_or(("", &c.path));
    disk.enter(up).map_err(|e| unreadable(&c.path, e))?;
    let dir = disk.here();
    if c.kind == EntryKind::Symlink {
        return Ok(Side::Bytes(link_target(dir, name, &c.path)?.into_bytes()));
    }
    let file = dir
        .open_file(OsStr::new(name))
        .map_err(|e| unreadable(&c.path, e))?;
    // It may have grown since the walk.
    let mut bytes = Vec::new();
    file.take(MAX_LINES_SIZE.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|e| unreadable(&c.path, e))?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_LINES_SIZE {
        return Ok(Side::TooBig);
    }
    Ok(Side::Bytes(bytes))
}

fn unreadable(path: &str, source: io::Error) -> SaveError {
    SaveError::Unreadable {
        path: path.to_owned(),
        source,
    }
}

fn is_binary(bytes: &[u8]) -> bool {
    bytes.iter().take(BINARY_CHECK).any(|&b| b == 0)
}

/// Each line with its `\n`; a last line without one stays as it is.
fn lines_of(bytes: &[u8]) -> Vec<&[u8]> {
    bytes.split_inclusive(|&b| b == b'\n').collect()
}

fn hunk(group: &[DiffOp], old: &[&[u8]], new: &[&[u8]]) -> Hunk {
    let mut lines = Vec::new();
    let mut push = |tag, side: &[&[u8]], range: std::ops::Range<usize>| {
        for line in side.get(range).unwrap_or_default() {
            lines.push((tag, line.to_vec()));
        }
    };
    for op in group {
        let (kind, o, n) = op.as_tag_tuple();
        match kind {
            DiffTag::Equal => push(Tag::Same, old, o),
            DiffTag::Delete => push(Tag::Removed, old, o),
            DiffTag::Insert => push(Tag::Added, new, n),
            DiffTag::Replace => {
                push(Tag::Removed, old, o);
                push(Tag::Added, new, n);
            }
        }
    }
    let count = |side: fn(&DiffOp) -> std::ops::Range<usize>| -> (u64, u64) {
        let first = group.first().map_or(0, |op| side(op).start);
        let len: usize = group.iter().map(|op| side(op).len()).sum();
        let start = if len == 0 {
            first
        } else {
            first.saturating_add(1)
        };
        (number(start), number(len))
    };
    let (old_start, old_lines) = count(DiffOp::old_range);
    let (new_start, new_lines) = count(DiffOp::new_range);
    Hunk {
        old_start,
        old_lines,
        new_start,
        new_lines,
        lines,
    }
}

fn number(n: usize) -> u64 {
    u64::try_from(n).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(lines: &[(Tag, &str)]) -> Vec<(Tag, Vec<u8>)> {
        lines
            .iter()
            .map(|(t, l)| (*t, l.as_bytes().to_vec()))
            .collect()
    }

    #[test]
    fn hunks_have_their_numbers_and_context() {
        let old = "1\n2\n3\n4\n5\n6\n7\n8\n9\n";
        let new = "1\n2\n3\n4\nfive\n6\n7\n8\n9\n";
        let Lines::Hunks { hunks, timed_out } =
            file_lines(old.as_bytes(), new.as_bytes(), DEADLINE)
        else {
            panic!("not lines");
        };
        assert!(!timed_out);
        assert_eq!(hunks.len(), 1);
        let h = &hunks[0];
        assert_eq!(
            (h.old_start, h.old_lines, h.new_start, h.new_lines),
            (2, 7, 2, 7)
        );
        assert_eq!(h.lines[3], (Tag::Removed, b"5\n".to_vec()));
        assert_eq!(h.lines[4], (Tag::Added, b"five\n".to_vec()));
        // A new file starts after line 0.
        let Lines::Hunks { hunks, .. } = file_lines(b"", b"a", DEADLINE) else {
            panic!("not lines");
        };
        assert_eq!((hunks[0].old_start, hunks[0].old_lines), (0, 0));
        assert_eq!(hunks[0].lines, text(&[(Tag::Added, "a")]));
        assert_eq!(
            file_lines(b"", b"", DEADLINE),
            Lines::Hunks {
                hunks: Vec::new(),
                timed_out: false
            }
        );
    }

    #[test]
    fn a_zero_byte_in_the_first_8000_is_binary() {
        let mut late = vec![b'a'; BINARY_CHECK];
        late.push(0);
        assert!(matches!(
            file_lines(&late, b"", DEADLINE),
            Lines::Hunks { .. }
        ));
        late.insert(0, 0);
        assert_eq!(file_lines(b"", &late, DEADLINE), Lines::Binary);
    }

    #[test]
    fn a_diff_past_its_deadline_says_so() {
        let old: String = (0..2000).map(|n| format!("{n}\n")).collect();
        let new: String = (0..2000).map(|n| format!("{}\n", n * 7 % 2000)).collect();
        let Lines::Hunks { hunks, timed_out } =
            file_lines(old.as_bytes(), new.as_bytes(), Duration::ZERO)
        else {
            panic!("not lines");
        };
        assert!(timed_out && !hunks.is_empty());
    }

    #[test]
    fn only_line_endings_is_told_apart() {
        let lines =
            |old: &str, new: &str| match file_lines(old.as_bytes(), new.as_bytes(), DEADLINE) {
                Lines::Hunks { hunks, .. } => hunks,
                other => panic!("{other:?}"),
            };
        assert!(only_line_endings(&lines("a\r\nb\r\n", "a\nb\n")));
        assert!(only_line_endings(&lines("a\nb\n", "a\r\nb\n")));
        assert!(!only_line_endings(&lines("a\r\n", "b\n")));
        assert!(!only_line_endings(&lines("a", "a\n")));
    }

    #[test]
    fn capped_stops_at_the_limit() {
        let mut out = Capped {
            bytes: vec![0; usize::try_from(MAX_LINES_SIZE).unwrap() - 1],
            over: false,
        };
        assert!(out.write_all(b"a").is_ok() && !out.over);
        assert!(out.write_all(b"b").is_err() && out.over);
    }
}
