//! `bolt changes`: what changed inside files, line by line (decisions 058 and 064).

use std::ffi::OsStr;
use std::io::{self, Read, Write};
use std::path::Path;
use std::time::{Duration, Instant};

use similar::algorithms::{Capture, Replace, myers};
use similar::{DiffOp, DiffTag, group_diff_ops};

use crate::diff::{ChangeKind, for_each_change, for_each_change_in};
use crate::history::tip_of;
use crate::id::Id;
use crate::listing::EntryKind;
use crate::repo::Repository;
use crate::scope::Scope;
use crate::snapshot::{SaveError, link_target, read_folder};
use crate::status::PathChange;
use crate::store::{Store, StoreError};
use crate::writeout::{self, Folders};

/// A file bigger than this shows without lines, and isn't read.
pub const MAX_LINES_SIZE: u64 = 64 << 20;
/// A file with more lines than this on either side shows without them (decision 064).
pub const MAX_LINES: usize = 1_000_000;
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
pub struct Hunk<'a> {
    pub old_start: u64,
    pub old_lines: u64,
    pub new_start: u64,
    pub new_lines: u64,
    /// Each line with its ending (`\n`, `\r\n`, or none for a last line without one), borrowed
    /// from the file.
    pub lines: Vec<(Tag, &'a [u8])>,
}

/// What changed inside one file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Lines<'a> {
    Binary,
    TooBig,
    TooManyLines,
    /// `timed_out` when the diff ran past its deadline, so its hunks are coarser than they could
    /// be.
    Hunks {
        hunks: Vec<Hunk<'a>>,
        timed_out: bool,
    },
}

/// The changed lines between `old` and `new`, with 3 lines of context. Binary when either side
/// has a zero byte in its first 8,000.
#[must_use]
pub fn file_lines<'a>(old: &'a [u8], new: &'a [u8], deadline: Duration) -> Lines<'a> {
    if is_binary(old) || is_binary(new) {
        return Lines::Binary;
    }
    // Counted before anything is held per line.
    if line_count(old) > MAX_LINES || line_count(new) > MAX_LINES {
        return Lines::TooManyLines;
    }
    let (old, new) = (lines_of(old), lines_of(new));
    let start = Instant::now();
    // Myers alone: the clean-up pass `similar` adds after it has no deadline.
    let mut ops = Replace::new(Capture::new());
    let diffed = myers::diff_deadline(
        &mut ops,
        &old[..],
        0..old.len(),
        &new[..],
        0..new.len(),
        start.checked_add(deadline),
    );
    let ops = match diffed {
        Ok(()) => ops.into_inner().into_ops(),
        Err(never) => match never {},
    };
    // `similar` doesn't say whether it gave up: one that ran past its deadline did.
    let timed_out = start.elapsed() >= deadline;
    let hunks = group_diff_ops(ops, CONTEXT)
        .iter()
        .map(|group| hunk(group, &old, &new))
        .collect();
    Lines::Hunks { hunks, timed_out }
}

/// How line endings changed, when nothing else did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Endings {
    CrlfToLf,
    LfToCrlf,
    /// Some lines one way, some the other.
    Both,
}

/// How line endings changed, if the hunks change nothing else: every line they remove or add is
/// one of a pair [`eol_only`] marks.
#[must_use]
pub fn only_line_endings(hunks: &[Hunk<'_>]) -> Option<Endings> {
    let mut found = None;
    for h in hunks {
        let pairs = ending_pairs(&h.lines);
        let changed = h.lines.iter().filter(|(t, _)| *t != Tag::Same).count();
        if pairs.len() * 2 != changed {
            return None;
        }
        for (_, _, way) in pairs {
            found = match found {
                Some(seen) if seen != way => Some(Endings::Both),
                _ => Some(way),
            };
        }
    }
    found
}

/// For each line of `hunk`, whether it is one of a removed and an added line that differ only in
/// their ending, `\r\n` for `\n` or the other way round. The pairs are the diff's own: the i-th of
/// a run of removed lines with the i-th of the added lines right after it.
#[must_use]
pub fn eol_only(hunk: &Hunk<'_>) -> Vec<bool> {
    let mut marked = vec![false; hunk.lines.len()];
    for (old, new, _) in ending_pairs(&hunk.lines) {
        for at in [old, new] {
            if let Some(m) = marked.get_mut(at) {
                *m = true;
            }
        }
    }
    marked
}

/// The pairs [`eol_only`] marks, by where they are among `lines`, with the way each changed.
fn ending_pairs(lines: &[(Tag, &[u8])]) -> Vec<(usize, usize, Endings)> {
    let run = |from: usize, tag| {
        lines
            .iter()
            .skip(from)
            .take_while(|(t, _)| *t == tag)
            .count()
    };
    let mut found = Vec::new();
    let mut at = 0;
    while at < lines.len() {
        let removed = run(at, Tag::Removed);
        let added = run(at + removed, Tag::Added);
        for k in 0..removed.min(added) {
            let (old, new) = (at + k, at + removed + k);
            let way = match (lines.get(old), lines.get(new)) {
                (Some((_, a)), Some((_, b))) => ending_change(a, b),
                _ => None,
            };
            found.extend(way.map(|way| (old, new, way)));
        }
        at += (removed + added).max(1);
    }
    found
}

/// How `old` became `new`, if only its ending changed.
fn ending_change(old: &[u8], new: &[u8]) -> Option<Endings> {
    match (ending(old), ending(new)) {
        ((a, b"\r\n"), (b, b"\n")) if a == b => Some(Endings::CrlfToLf),
        ((a, b"\n"), (b, b"\r\n")) if a == b => Some(Endings::LfToCrlf),
        _ => None,
    }
}

/// A line split into what it holds and its ending.
fn ending(line: &[u8]) -> (&[u8], &[u8]) {
    for end in [&b"\r\n"[..], b"\n"] {
        if let Some(body) = line.strip_suffix(end) {
            return (body, end);
        }
    }
    (line, b"")
}

/// Hands `each` every unsaved change, as `status` finds them, with its lines: the folder against
/// the save the project is on. Only `paths` and what is in them, if given. Nothing is stored;
/// only the file-state cache is written. The folder walk's warnings go to the store's. One file
/// is held at a time.
pub fn unsaved_changes<E: From<SaveError> + From<StoreError>>(
    repo: &Repository,
    paths: Option<&[String]>,
    git_excludes: Option<&Path>,
    each: &mut dyn FnMut(&str, &PathChange, Lines<'_>) -> Result<(), E>,
) -> Result<(), E> {
    let store = repo.store();
    let (_, entry) = repo.current().map_err(SaveError::from)?;
    let tip = tip_of(&entry).map_err(SaveError::from)?;
    let save = store.save(&tip, repo.signatures_allowed())?;
    let now = read_folder(repo, save.root, false, git_excludes).map_err(|e| e.in_save(tip))?;
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
        each(&c.path, &c, lines(&old, &new))
    })
}

/// Hands `each` every file the save `save` changed, compared with its first parent (every file,
/// for a save without one), with its lines. Only `paths` and what is in them, if given. One file
/// is held at a time.
pub fn save_changes<E: From<SaveError> + From<StoreError>>(
    repo: &Repository,
    save: Id,
    paths: Option<&[String]>,
    each: &mut dyn FnMut(&str, &PathChange, Lines<'_>) -> Result<(), E>,
) -> Result<(), E> {
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
        each(&c.path, &c, lines(&old, &new))
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

fn lines<'a>(old: &'a Side, new: &'a Side) -> Lines<'a> {
    match (old, new) {
        (Side::Bytes(old), Side::Bytes(new)) => file_lines(old, new, deadline()),
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

/// How many lines `bytes` holds, a last one without `\n` included.
fn line_count(bytes: &[u8]) -> usize {
    let breaks = bytes.iter().filter(|&&b| b == b'\n').count();
    breaks.saturating_add(usize::from(bytes.last().is_some_and(|&b| b != b'\n')))
}

/// Each line with its `\n`; a last line without one stays as it is.
fn lines_of(bytes: &[u8]) -> Vec<&[u8]> {
    bytes.split_inclusive(|&b| b == b'\n').collect()
}

fn hunk<'a>(group: &[DiffOp], old: &[&'a [u8]], new: &[&'a [u8]]) -> Hunk<'a> {
    let mut lines = Vec::new();
    let mut push = |tag, side: &[&'a [u8]], range: std::ops::Range<usize>| {
        lines.extend(
            side.get(range)
                .unwrap_or_default()
                .iter()
                .map(|l| (tag, *l)),
        );
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

    fn hunks_of<'a>(old: &'a str, new: &'a str) -> Vec<Hunk<'a>> {
        match file_lines(old.as_bytes(), new.as_bytes(), DEADLINE) {
            Lines::Hunks { hunks, .. } => hunks,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn hunks_have_their_numbers_and_context() {
        let hunks = hunks_of(
            "1\n2\n3\n4\n5\n6\n7\n8\n9\n",
            "1\n2\n3\n4\nfive\n6\n7\n8\n9\n",
        );
        assert_eq!(hunks.len(), 1);
        let h = &hunks[0];
        assert_eq!(
            (h.old_start, h.old_lines, h.new_start, h.new_lines),
            (2, 7, 2, 7)
        );
        assert_eq!(h.lines[3], (Tag::Removed, &b"5\n"[..]));
        assert_eq!(h.lines[4], (Tag::Added, &b"five\n"[..]));
        // A new file starts after line 0.
        let hunks = hunks_of("", "a");
        assert_eq!((hunks[0].old_start, hunks[0].old_lines), (0, 0));
        assert_eq!(hunks[0].lines, [(Tag::Added, &b"a"[..])]);
        assert!(hunks_of("", "").is_empty());
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
    fn more_than_a_million_lines_on_a_side_shows_none() {
        let at_limit = vec![b'\n'; MAX_LINES];
        assert!(matches!(
            file_lines(b"", &at_limit, DEADLINE),
            Lines::Hunks { .. }
        ));
        let mut over = at_limit;
        over.push(b'x');
        assert_eq!(file_lines(&over, b"", DEADLINE), Lines::TooManyLines);
        assert_eq!(line_count(b"a\nb"), 2);
        assert_eq!(line_count(b""), 0);
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
    fn a_diff_that_would_take_long_stops_at_its_deadline() {
        // No line in common: Myers' work grows with the square of the lines, minutes here.
        let old: String = (0..60_000).map(|n| format!("a{n}\n")).collect();
        let new: String = (0..60_000).map(|n| format!("b{n}\n")).collect();
        let started = Instant::now();
        let lines = file_lines(old.as_bytes(), new.as_bytes(), Duration::from_millis(100));
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "{:?}",
            started.elapsed()
        );
        let Lines::Hunks { hunks, timed_out } = lines else {
            panic!("not lines");
        };
        assert!(timed_out);
        // Coarser, still whole: every line goes and comes.
        let count = |tag| {
            hunks
                .iter()
                .flat_map(|h| &h.lines)
                .filter(|(t, _)| *t == tag)
                .count()
        };
        assert_eq!((count(Tag::Removed), count(Tag::Added)), (60_000, 60_000));
    }

    #[test]
    fn only_line_endings_is_told_apart() {
        let way = |old, new| only_line_endings(&hunks_of(old, new));
        assert_eq!(way("a\r\nb\r\n", "a\nb\n"), Some(Endings::CrlfToLf));
        assert_eq!(way("a\nb\n", "a\r\nb\n"), Some(Endings::LfToCrlf));
        assert_eq!(way("a\r\nb\n", "a\nb\r\n"), Some(Endings::Both));
        assert_eq!(way("a\r\n", "b\n"), None);
        assert_eq!(way("a", "a\n"), None);
        assert_eq!(way("a\n", "a\n"), None);
        // The endings changed, and a line moved too.
        assert_eq!(way("a\r\nb\r\ns\n", "a\ns\nb\n"), None);
    }

    #[test]
    fn a_pair_that_differs_only_in_its_ending_is_marked() {
        let marked =
            |old, new| -> Vec<Vec<bool>> { hunks_of(old, new).iter().map(eol_only).collect() };
        assert_eq!(
            marked("one\r\ntwo\nthree\n", "one\ntwo\nTHREE\n"),
            [[true, true, false, false, false]]
        );
        // The i-th line of each run of removed lines goes with the i-th of the added lines after it.
        assert_eq!(
            marked("a\r\nb\r\ns\n", "a\ns\nb\n"),
            [[true, false, true, false, false]]
        );
        // A line without an ending, or two that are the same, aren't such a pair.
        assert_eq!(marked("a\r\n", "a"), [[false, false]]);
        assert_eq!(marked("a\n", "b\n"), [[false, false]]);
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
