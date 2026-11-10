//! `bolt tidy`: removes from the store what nothing needs any more, keeping what lines, shelves,
//! tags, server lines, the diary and the undo window reach.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::ffi::{OsStr, OsString};
use std::io;
use std::path::Path;
use std::sync::Arc;

use bolt_platform::dir::{Dir, EntryMeta};

use crate::diary::{Action, DiaryEntry, Position, ServerLines, State, TagList};
use crate::encoding::DecodeError;
use crate::fsio::{self, fault_point};
use crate::id::Id;
use crate::listing::{EntryKind, Listing};
use crate::merge;
use crate::pack::{self, IndexEntry, ObjectKind, PackError, PackReader};
use crate::repo::{RepoError, Repository, cant_write, damaged, entry_in};
use crate::safe_fs;
use crate::save::Save;
use crate::store::{Store, StoreError, is_damage, is_pack_name};
use crate::text::{escape, thousands};
use crate::undo;
use crate::warning::{self, Warning};
use crate::writeout;

const DAY_MS: i64 = 86_400_000;
/// A pack changed this recently is left as it is: what refers to it may still be on its way.
const GRACE_NS: i128 = 24 * 3600 * 1_000_000_000;

/// What `tidy` did.
#[derive(Debug, Default)]
pub struct TidyReport {
    /// About how many bytes it would free, worked out before anything was removed.
    pub to_free: u64,
    /// The packs' sizes on disk, before and after.
    pub bytes_before: u64,
    pub bytes_after: u64,
    pub packs_removed: usize,
    pub packs_written: usize,
    /// Packs changed in the 24 hours before it started, left as they are.
    pub packs_young: usize,
}

/// A pack in the store when `tidy` started.
pub(crate) struct Listed {
    pub(crate) name: OsString,
    pub(crate) size: u64,
    /// Changed in the 24 hours before: left as it is.
    pub(crate) young: bool,
}

/// What a pack's index says it holds.
#[derive(Default)]
struct Holds {
    version: u32,
    /// How many of its objects are kept, and their stored bytes.
    kept: u64,
    kept_stored: u64,
    /// Whether it holds anything not kept.
    other: bool,
    /// What it counts toward a small pack.
    uncompressed: u64,
}

/// Removes what nothing needs any more: what no line, shelf, tag or server line reaches, and what
/// only steps older than `window_days` before `now_ms` needed. Packs changed in the 24 hours
/// before `now_ms` are left as they are. `on_estimate` is told about how much that frees before
/// anything is removed. It reads only records and chunk lists to mark what to keep, never a file's
/// contents: one of those it can't read stops it before then, and so does a pack it can't open,
/// read or check, or a conflict record, which it can't follow.
pub fn tidy(
    repo: &mut Repository,
    window_days: u32,
    now_ms: i64,
    on_wait: &mut dyn FnMut(&str),
    on_estimate: &mut dyn FnMut(u64),
) -> Result<TidyReport, RepoError> {
    let lock = repo.lock_for_change("tidy", on_wait)?;
    if let Some(done) = writeout::finish_interrupted(repo, &lock)? {
        for w in done.warnings {
            repo.store().note(w);
        }
    }
    let folder = Arc::clone(repo.packs_folder());
    let packs_dir = repo.packs_dir().to_owned();
    let now_ns = i128::from(now_ms).saturating_mul(1_000_000);
    let listed = listed(&folder, &packs_dir, now_ns)?;
    let since = window_start(repo, window_days, now_ms);
    let keep = kept(repo, &listed, since)?;
    record_window(repo, since)?;
    fault_point("tidy:marked")?;

    let mut report = TidyReport::default();
    let (mut unread, mut rewrite) = (Vec::new(), Vec::new());
    let mut moved = 0u64;
    // An open file can't always be replaced or removed on Windows.
    repo.store().close_all();
    for pack in listed {
        report.bytes_before = report.bytes_before.saturating_add(pack.size);
        if pack.young {
            report.packs_young += 1;
            continue;
        }
        let path = packs_dir.join(&pack.name);
        let holds = judge(&folder, &pack.name, &path, &keep).map_err(|e| cant_tidy(&path, &e))?;
        if holds.kept == 0 {
            report.to_free = report.to_free.saturating_add(pack.size);
            unread.push(pack.name);
        } else if holds.other
            || holds.version == 1
            || merge::is_small(pack.size, holds.uncompressed)
        {
            let freed = pack.size.saturating_sub(holds.kept_stored);
            report.to_free = report.to_free.saturating_add(freed);
            moved = moved.saturating_add(holds.kept);
            rewrite.push((pack.name, holds.kept));
        }
    }
    if moved > 0 {
        report.to_free = report.to_free.saturating_sub(pack::v2_overhead(moved));
    }
    on_estimate(report.to_free);

    // Before anything is written, so that tidying frees space even on a full disk.
    remove(repo, &folder, &packs_dir, unread, &mut report);
    let mut out = repo.new_packs()?;
    let mut taken = Vec::new();
    let names: Vec<OsString> = rewrite.iter().map(|(name, _)| name.clone()).collect();
    let keeps = |id: &Id| keep.contains(id);
    let copied = merge::copy_packs(
        &folder,
        &packs_dir,
        &names,
        &mut out,
        &keeps,
        "tidy:copying",
    )?;
    for ((name, kept), copied) in rewrite.into_iter().zip(copied) {
        let path = packs_dir.join(&name);
        match copied {
            Ok(copied) if copied == kept => taken.push(name),
            // The index read otherwise than when it was checked: what it judged can't be trusted.
            Ok(copied) => repo.store().note(Warning::new(
                warning::PACK_NOT_TIDIED,
                format!(
                    "Pack {} wasn't tidied because its index read differently the second time: \
                     {} of the {} objects to keep in it were found. It is left as it is, and the \
                     next tidy tries again; a failing disk can cause this.",
                    shown(&path),
                    thousands(copied),
                    thousands(kept)
                ),
            )),
            Err(e) => not_tidied(repo, &path, &e),
        }
    }
    let made: Vec<OsString> = out
        .finish()?
        .iter()
        .filter_map(|p| p.file_name().map(OsStr::to_owned))
        .collect();
    report.packs_written = made.len();
    fault_point("tidy:written")?;
    // A new pack can have an old one's name: it holds the same, checked.
    taken.retain(|name| !merge::is_new(&made, name));
    remove(repo, &folder, &packs_dir, taken, &mut report);
    repo.store_mut().refresh()?;
    report.bytes_after = packs(&folder)?
        .into_iter()
        .filter_map(|(_, meta)| meta.ok())
        .fold(0u64, |sum, meta| sum.saturating_add(meta.size));
    Ok(report)
}

/// `state/tidied`, where the start of the window the last `tidy` used is kept.
const TIDIED: &str = "tidied";

/// The start of the undo window: now minus `window_days`, or the start the last `tidy` used if
/// that is later, so a window made longer never asks for what that one freed.
pub(crate) fn window_start(repo: &Repository, window_days: u32, now_ms: i64) -> i64 {
    let since = now_ms.saturating_sub(i64::from(window_days).saturating_mul(DAY_MS));
    tidied(repo, now_ms).map_or(since, |earlier| earlier.max(since))
}

/// What `state/tidied` names. A file that isn't digits and an LF, or names a time after `now_ms`,
/// counts as absent: that only brings back the stop it was there to prevent.
fn tidied(repo: &Repository, now_ms: i64) -> Option<i64> {
    let shown = repo.state_dir().join(TIDIED);
    let (name, max) = (OsStr::new(TIDIED), safe_fs::MAX_TIDIED_BYTES);
    let bytes = safe_fs::read_capped_in(repo.state_folder(), name, &shown, max).ok()?;
    let digits = bytes.strip_suffix(b"\n")?;
    if digits.is_empty() || !digits.iter().all(u8::is_ascii_digit) {
        return None;
    }
    let ms: i64 = std::str::from_utf8(digits).ok()?.parse().ok()?;
    (ms <= now_ms).then_some(ms)
}

/// Writes `since` into `state/tidied`, before anything is removed.
fn record_window(repo: &Repository, since: i64) -> Result<(), RepoError> {
    // Digits only: a window reaching back before 1970 starts there for the next one.
    let text = format!("{}\n", since.max(0));
    fsio::write_atomic(
        repo.tmp_folder(),
        repo.state_folder(),
        TIDIED,
        text.as_bytes(),
    )
    .map_err(|e| cant_write(repo.state_dir().join(TIDIED), e))
}

/// Removes the old packs `names` from `folder`, one at a time. One that can't be removed is left,
/// with a warning.
fn remove(
    repo: &Repository,
    folder: &Dir,
    packs_dir: &Path,
    names: Vec<OsString>,
    report: &mut TidyReport,
) {
    for name in names {
        let removed = fault_point("tidy:removing").and_then(|()| fsio::remove_file(folder, &name));
        match removed {
            Ok(()) => report.packs_removed += 1,
            Err(e) => repo.store().note(Warning::new(
                warning::PACK_NOT_TIDIED,
                format!(
                    "Bolt couldn't remove pack {} ({}): it is left as it is, which does no harm, \
                     and the next tidy tries again.",
                    shown(&packs_dir.join(&name)),
                    escape(e.to_string().trim_end_matches('.'), false)
                ),
            )),
        }
    }
}

/// [`listed`] for `verify`, which checks each pack on its own: one whose details can't be read is
/// left out.
pub(crate) fn listed_leniently(folder: &Dir, now_ns: i128) -> io::Result<Vec<Listed>> {
    let listed = packs(folder)?.into_iter().filter_map(|(name, meta)| {
        let meta = meta.ok()?;
        Some(Listed {
            name,
            size: meta.size,
            young: now_ns.saturating_sub(meta.modified_ns) < GRACE_NS,
        })
    });
    Ok(listed.collect())
}

/// Every entry in `folder` named like a pack, by name, with its details or why they can't be read.
pub(crate) fn packs(folder: &Dir) -> io::Result<Vec<(OsString, io::Result<EntryMeta>)>> {
    let mut found = Vec::new();
    for entry in folder.entries()? {
        if is_pack_name(&entry.name) {
            let meta = folder.metadata(&entry.name);
            found.push((entry.name, meta));
        }
    }
    found.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(found)
}

/// The packs in `folder`, at `packs_dir`, each young if changed less than a day before `now_ns`.
/// Everything named like a pack counts as one, so one whose details can't be read stops `tidy`,
/// and so, once it is opened, does one that isn't a pack.
fn listed(folder: &Dir, packs_dir: &Path, now_ns: i128) -> Result<Vec<Listed>, RepoError> {
    let mut listed = Vec::new();
    for (name, meta) in packs(folder)? {
        let meta = meta.map_err(|e| cant_tidy(&packs_dir.join(&name), &e.into()))?;
        listed.push(Listed {
            name,
            size: meta.size,
            young: now_ns.saturating_sub(meta.modified_ns) < GRACE_NS,
        });
    }
    Ok(listed)
}

/// What the index of the pack `name` says it holds of `keep`, each line checked as it is read and
/// the whole index against its checksum again: reads check each object against its ID, but
/// tidying deletes by the index alone.
fn judge(folder: &Dir, name: &OsStr, path: &Path, keep: &HashSet<Id>) -> Result<Holds, PackError> {
    let pack = PackReader::open_in(folder, name, path)?;
    let mut holds = Holds {
        version: pack.version(),
        ..Holds::default()
    };
    for entry in pack.entries() {
        let entry = entry?;
        holds.uncompressed = holds.uncompressed.saturating_add(merge::counted(&entry));
        if keep.contains(&entry.id) {
            holds.kept = holds.kept.saturating_add(1);
            holds.kept_stored = holds.kept_stored.saturating_add(entry.stored_size);
        } else {
            holds.other = true;
        }
    }
    pack.check_index()?;
    Ok(holds)
}

/// A pack's path, as messages show it.
fn shown(path: &Path) -> String {
    escape(&path.display().to_string(), false)
}

/// Why reading a pack failed with `e`, as a message says it.
fn why(e: &PackError) -> String {
    let why = match e {
        PackError::Corrupt { reason, .. } => reason.clone(),
        other => other.to_string(),
    };
    why.trim_end_matches('.').to_owned()
}

/// The stop for the pack at `path`, which can't be opened, read or checked, or holds a chunk list
/// that must be read but can't be: `e`. It could hold the only list naming chunks in the others.
fn cant_tidy(path: &Path, e: &PackError) -> RepoError {
    let what = if is_damage(e) {
        "is damaged"
    } else {
        "can't be read"
    };
    RepoError::CantTidy {
        path: path.to_owned(),
        why: Some(format!("{what} ({})", why(e))),
    }
}

/// Notes that the pack at `path` is left as it is, since reading it failed with `e`.
fn not_tidied(repo: &Repository, path: &Path, e: &PackError) {
    let why = escape(&why(e), false);
    let said = if is_damage(e) {
        format!(
            "Pack {} wasn't tidied because it is damaged ({why}), so it is left as it is. Ask for \
             help before trying to repair the project.",
            shown(path)
        )
    } else {
        format!(
            "Pack {} wasn't tidied because Bolt couldn't read it ({why}), so it is left as it is, \
             and the next tidy tries again.",
            shown(path)
        )
    };
    repo.store()
        .note(Warning::new(warning::PACK_NOT_TIDIED, said));
}

/// What marking needs from the packs' indexes.
#[derive(Default)]
struct Survey {
    /// The diary entries stored.
    entries: HashSet<Id>,
    /// Each chunk list's index line, with its pack's place among the packs, by its file's ID.
    lists: HashMap<Id, Vec<(usize, IndexEntry)>>,
    /// The chunk lists whose chunks are kept, by their pack's place and offset: so far those in
    /// young packs.
    reads: BTreeMap<(usize, u64), IndexEntry>,
    /// Every ID an index names, for `verify`.
    all: HashMap<Id, Held>,
}

/// Reads the index of each of `packs`, in `folder` at `packs_dir`, young ones too, and checks it
/// against its checksum. A pack that can't be opened, read or checked stops `tidy`, since it could
/// hold the only chunk list naming chunks in the others; so does a conflict record, which this
/// version doesn't follow. For `verify` (`lenient`), which checks each pack on its own, they are
/// passed over, and every ID read is kept in `all`.
fn survey(
    folder: &Dir,
    packs_dir: &Path,
    packs: &[Listed],
    lenient: bool,
) -> Result<Survey, RepoError> {
    let mut survey = Survey::default();
    for (at, listed) in packs.iter().enumerate() {
        let path = packs_dir.join(&listed.name);
        let cant = |e: PackError| cant_tidy(&path, &e);
        let pack = match PackReader::open_in(folder, &listed.name, &path) {
            Ok(pack) => pack,
            Err(_) if lenient => continue,
            Err(e) => return Err(cant(e)),
        };
        for line in pack.entries() {
            let line = match line {
                Ok(line) => line,
                Err(_) if lenient => break,
                Err(e) => return Err(cant(e)),
            };
            if lenient {
                survey.all.entry(line.id).or_default().add(&line);
            }
            match line.kind {
                ObjectKind::DiaryEntry => {
                    survey.entries.insert(line.id);
                }
                ObjectKind::ChunkList => {
                    survey.lists.entry(line.id).or_default().push((at, line));
                    if listed.young {
                        survey.reads.insert((at, line.offset), line);
                    }
                }
                ObjectKind::Conflict if !lenient => {
                    return Err(RepoError::CantTidy {
                        path: path.clone(),
                        why: None,
                    });
                }
                _ => {}
            }
        }
        if !lenient {
            pack.check_index().map_err(cant)?;
        }
    }
    Ok(survey)
}

/// What must be kept: every object reached (fingerprints), and every chunk that a chunk list of a
/// file reached, or one in a young pack, names. `packs` are the store's packs; the undo window
/// starts at `since`.
fn kept(repo: &Repository, packs: &[Listed], since: i64) -> Result<HashSet<Id>, RepoError> {
    let survey = survey(repo.packs_folder(), repo.packs_dir(), packs, false)?;
    let (head, _) = repo.current()?;
    let (kept, _) = mark(&Source::of(repo), packs, survey, head, since, None)?;
    Ok(kept)
}

/// Where marking reads: a store, whether its saves may be signed, and the packs' folder.
pub(crate) struct Source<'a> {
    pub(crate) store: &'a Store,
    pub(crate) signed: bool,
    pub(crate) folder: &'a Dir,
    pub(crate) packs_dir: &'a Path,
}

impl<'a> Source<'a> {
    pub(crate) fn of(repo: &'a Repository) -> Source<'a> {
        Source {
            store: repo.store(),
            signed: repo.signatures_allowed(),
            folder: repo.packs_folder(),
            packs_dir: repo.packs_dir(),
        }
    }
}

/// What `verify` learns while marking, which goes on past what it can't read.
#[derive(Default)]
pub(crate) struct Noted {
    /// What couldn't be read, and why.
    pub(crate) unread: Vec<(Id, Unread)>,
    /// Every ID the packs' indexes name.
    pub(crate) known: HashMap<Id, Held>,
    /// Chunks to find the files of, as well as those no index names.
    pub(crate) watch: HashSet<Id>,
    /// Each chunk no index names, or watched, with the file whose list names it.
    pub(crate) via: HashMap<Id, Id>,
    /// The saves read.
    pub(crate) saves: Vec<Id>,
    /// The diary entries read from the current one back.
    pub(crate) entries: u64,
    /// Each save read, with its top folder's listing.
    pub(crate) roots: Vec<(Id, Id)>,
    /// What each listing read holds, to count a save's entries by path.
    pub(crate) shapes: HashMap<Id, Shape>,
    /// Each file a listing gives another size than what is stored: the listing, the file's
    /// name, the size it gives and the size stored.
    pub(crate) sizes: Vec<(Id, String, u64, u64)>,
}

impl Noted {
    /// Notes what the listing `id` holds: its shape, each file whose size isn't the size stored,
    /// and each file stored only as another kind of object.
    fn listing_read(&mut self, id: Id, listing: &Listing) {
        let mut shape = Shape {
            entries: listing.entries().len() as u64,
            ..Shape::default()
        };
        for entry in listing.entries() {
            if entry.kind == EntryKind::Folder {
                shape.folders.push((entry.name.len(), entry.id));
                continue;
            }
            shape.longest = shape.longest.max(entry.name.len());
            if !matches!(
                entry.kind,
                EntryKind::File | EntryKind::Executable | EntryKind::Symlink
            ) {
                continue;
            }
            // One no index names at all is told as missing from the set of IDs kept.
            let Some(held) = self.known.get(&entry.id).copied() else {
                continue;
            };
            match held.file_size() {
                Some(stored) if stored != entry.size => {
                    let name = entry.name.clone();
                    self.sizes.push((id, name, entry.size, stored));
                }
                Some(_) => {}
                None => self.unread.push((entry.id, Unread::OtherKind)),
            }
        }
        self.shapes.insert(id, shape);
    }
}

/// How an ID is held in the packs' indexes.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Held {
    /// As file contents, of this size.
    pub(crate) content: Option<u64>,
    /// As a chunk list, of a file of this size.
    pub(crate) list: Option<u64>,
    /// As some other kind.
    pub(crate) other: bool,
}

impl Held {
    fn add(&mut self, line: &IndexEntry) {
        match line.kind {
            ObjectKind::Content => {
                self.content.get_or_insert(line.uncompressed_size);
            }
            ObjectKind::ChunkList => {
                self.list.get_or_insert(line.uncompressed_size);
            }
            _ => self.other = true,
        }
    }

    /// The size of the file stored under this ID, if it is stored as one.
    pub(crate) fn file_size(self) -> Option<u64> {
        self.content.or(self.list)
    }
}

/// What a listing holds, as far as a save's size goes.
#[derive(Default)]
pub(crate) struct Shape {
    pub(crate) entries: u64,
    /// The longest name of an entry that isn't a folder.
    pub(crate) longest: usize,
    /// Each folder's name length and listing.
    pub(crate) folders: Vec<(usize, Id)>,
}

/// Why `verify`'s marking couldn't read a record.
pub(crate) enum Unread {
    Missing,
    /// Stored, but as another kind of object.
    OtherKind,
    Damaged(String),
    Unreadable(String),
}

impl Unread {
    fn of(e: &StoreError) -> Unread {
        match e {
            StoreError::Missing(_) => Unread::Missing,
            StoreError::WrongKind { .. } => Unread::OtherKind,
            StoreError::Pack(p) if !is_damage(p) => Unread::Unreadable(p.to_string()),
            e => Unread::Damaged(e.to_string()),
        }
    }
}

/// What `tidy` keeps, as `verify` asks for it: from the entry `head` in `src`'s store, with the
/// undo window starting at `since`, noting in `noted` what can't be read instead of stopping
/// there.
pub(crate) fn needs(
    src: &Source,
    packs: &[Listed],
    head: Id,
    since: i64,
    noted: Noted,
) -> Result<(HashSet<Id>, Noted), RepoError> {
    let survey = survey(src.folder, src.packs_dir, packs, true)?;
    let (kept, noted) = mark(src, packs, survey, head, since, Some(noted))?;
    Ok((kept, noted.unwrap_or_default()))
}

/// Marks what must be kept from the entry `head`, as [`kept`] says, reading from `src`. With
/// `noted`, what can't be read is noted there and passed over.
fn mark(
    src: &Source,
    packs: &[Listed],
    survey: Survey,
    head: Id,
    since: i64,
    noted: Option<Noted>,
) -> Result<(HashSet<Id>, Option<Noted>), RepoError> {
    let Survey {
        entries,
        lists,
        reads,
        all,
    } = survey;
    let mut marks = Marks {
        src,
        kept: HashSet::new(),
        followed: HashSet::new(),
        saves: Vec::new(),
        listings: Vec::new(),
        lists: &lists,
        reads,
        stored: &entries,
        gap: None,
        passed: None,
        noted,
    };
    if let Some(noted) = &mut marks.noted {
        noted.known = all;
    }
    let mut walked = HashSet::new();
    if let Err(e) = marks.diary(head, since, &mut walked, false) {
        match (marks.gap.take(), &mut marks.noted) {
            (Some(missing), Some(noted)) => noted.unread.push((missing, Unread::Missing)),
            _ => return Err(e),
        }
    }
    if let Some(noted) = &mut marks.noted {
        let entry = ObjectKind::DiaryEntry as u8;
        noted.entries = marks.followed.iter().filter(|(k, _)| *k == entry).count() as u64;
    }
    // A diary cut off from `head` (a rolled-back `head`, a crash's leftover) lives out the window.
    for (end, entry) in undo::ends_among(src.store, entries.iter().copied())? {
        if entry.who.time_ms < since {
            continue;
        }
        let walked_out = marks.diary(end, since, &mut walked, true);
        let passed = marks.passed.take();
        // Only an entry the store lacks is passed over; what was reached stays kept.
        let missing = match (walked_out, marks.gap.take()) {
            (Err(e), None) => return Err(e),
            (_, Some(missing)) => passed.unwrap_or(missing),
            (Ok(()), None) => match passed {
                Some(missing) => missing,
                None => continue,
            },
        };
        src.store.note(Warning::new(
            warning::DIARY_END_UNREADABLE,
            format!(
                "The diary that ends at entry {}, cut off from the current one, can't be read \
                 back to its start: entry {} isn't in the store. Tidy kept what it could reach \
                 from that end and went on.",
                end.short_hex(),
                missing.short_hex()
            ),
        ));
    }
    marks.follow()?;
    marks.chunks(packs)?;
    Ok((marks.kept, marks.noted))
}

/// What `mark` has found so far, and what it has still to read.
struct Marks<'a> {
    src: &'a Source<'a>,
    kept: HashSet<Id>,
    /// The records already followed, by kind: an ID kept as one kind can't stop another being read.
    followed: HashSet<(u8, Id)>,
    saves: Vec<Id>,
    listings: Vec<Id>,
    /// Every chunk list's line, by its file's ID.
    lists: &'a HashMap<Id, Vec<(usize, IndexEntry)>>,
    /// The chunk lists to read once all else is marked.
    reads: BTreeMap<(usize, u64), IndexEntry>,
    /// The diary entries the store holds.
    stored: &'a HashSet<Id>,
    /// An entry a diary named that the store doesn't hold, once one stopped marking.
    gap: Option<Id>,
    /// The first undo or redo step a diary end's walk passed over, the store lacking it.
    passed: Option<Id>,
    /// `verify`'s notes; without them, what can't be read stops marking.
    noted: Option<Noted>,
}

impl Marks<'_> {
    /// Marks the diary back from `start`, as undo can reach it. `walked` holds the entries already
    /// marked, with whether their state was kept, so that diaries sharing a past are read once.
    /// From a diary `end` other than `head`'s, an undo or redo whose step the store lacks is
    /// noted in `passed` and the walk goes on along parents.
    fn diary(
        &mut self,
        start: Id,
        since: i64,
        walked: &mut HashSet<(Id, bool)>,
        end: bool,
    ) -> Result<(), RepoError> {
        // The start's own state is always kept; an older entry's when the one after it is within
        // the window, since undoing that one puts it back.
        let (mut at, mut after_within) = (Some(start), true);
        while let Some(id) = at {
            if walked.contains(&(id, true)) || !walked.insert((id, after_within)) {
                return Ok(());
            }
            let Some(entry) = self.entry(&id)? else {
                return Ok(());
            };
            let within = entry.who.time_ms >= since;
            if within || after_within {
                self.state(&entry.state)?;
            }
            if within {
                self.change(&entry);
                if let Action::Undo(step) | Action::Redo(step) = entry.action
                    && let Some(step) = self.step_entry(step, end)?
                {
                    self.state(&step.state)?;
                    self.change(&step);
                    if let Some(before) = step.parent
                        && let Some(before) = self.step_entry(before, end)?
                    {
                        self.state(&before.state)?;
                    }
                }
            }
            after_within = within;
            at = entry.parent;
        }
        Ok(())
    }

    /// [`Marks::entry`] for an undo or redo's step, or the entry before it. From a diary `end`, one
    /// the store lacks is noted in `passed` and passed over.
    fn step_entry(&mut self, id: Id, end: bool) -> Result<Option<DiaryEntry>, RepoError> {
        if end && !self.stored.contains(&id) {
            self.passed.get_or_insert(id);
            return Ok(None);
        }
        self.entry(&id)
    }

    /// Keeps `id`, and gives whether a record of `kind` with it is still to be followed.
    fn first(&mut self, kind: ObjectKind, id: Id) -> bool {
        self.kept.insert(id);
        self.followed.insert((kind as u8, id))
    }

    /// Keeps the diary entry `id` and reads it. One the store doesn't hold stops marking, noted in
    /// `gap`; `None` is one `verify` noted it couldn't read.
    fn entry(&mut self, id: &Id) -> Result<Option<DiaryEntry>, RepoError> {
        if !self.stored.contains(id) {
            self.gap = Some(*id);
            return Err(damaged(format!("the diary has no entry {id}")));
        }
        self.kept.insert(*id);
        let read = entry_in(self.src.store, id);
        let Some(noted) = &mut self.noted else {
            return read.map(Some);
        };
        match read {
            Ok(entry) => {
                self.followed.insert((ObjectKind::DiaryEntry as u8, *id));
                Ok(Some(entry))
            }
            Err(e) => {
                let why = match e {
                    RepoError::Damaged { .. } => Unread::Damaged(e.to_string()),
                    e => Unread::Unreadable(e.to_string()),
                };
                noted.unread.push((*id, why));
                Ok(None)
            }
        }
    }

    /// Reads the record `id` of `kind`, which must be kept; `None` when `verify` noted it couldn't.
    fn read(
        &mut self,
        id: &Id,
        kind: ObjectKind,
        what: &str,
    ) -> Result<Option<Vec<u8>>, RepoError> {
        let e = match self.src.store.read_record(id, kind) {
            Ok(bytes) => return Ok(Some(bytes)),
            Err(e) => e,
        };
        if let Some(noted) = &mut self.noted {
            noted.unread.push((*id, Unread::of(&e)));
            return Ok(None);
        }
        Err(match e {
            // Memory or I/O: the copy may be fine, so this isn't damage.
            StoreError::Pack(e) if !is_damage(&e) => RepoError::Io(io::Error::other(format!(
                "Bolt couldn't read {what} {id}, which must be kept: {e}"
            ))),
            e => damaged(format!("{what} {id} can't be read ({e})")),
        })
    }

    /// `decoded` as the record `id`, `what`; `None` when `verify` noted it couldn't be.
    fn decoded<T>(
        &mut self,
        id: &Id,
        what: &str,
        decoded: Result<T, DecodeError>,
    ) -> Result<Option<T>, RepoError> {
        match decoded {
            Ok(value) => Ok(Some(value)),
            Err(e) => {
                let said = format!("{what} {id} can't be read ({e})");
                let Some(noted) = &mut self.noted else {
                    return Err(damaged(said));
                };
                noted.unread.push((*id, Unread::Damaged(said)));
                Ok(None)
            }
        }
    }

    /// Keeps every chunk the lists to read name, opening each pack once. A list names only
    /// chunks, so nothing more is followed. One that can't be read stops `tidy`, naming its pack;
    /// `verify` checks each pack on its own, so passes over it here.
    fn chunks(&mut self, packs: &[Listed]) -> Result<(), RepoError> {
        for (at, listed) in packs.iter().enumerate() {
            let mut lines = self.reads.range((at, 0)..=(at, u64::MAX)).peekable();
            if lines.peek().is_none() {
                continue;
            }
            let path = self.src.packs_dir.join(&listed.name);
            let cant = |e: PackError| cant_tidy(&path, &e);
            let pack = match PackReader::open_in(self.src.folder, &listed.name, &path) {
                Ok(pack) => pack,
                Err(_) if self.noted.is_some() => continue,
                Err(e) => return Err(cant(e)),
            };
            for (_, line) in lines {
                let list = match pack.read_chunk_list(line) {
                    Ok(list) => list,
                    Err(_) if self.noted.is_some() => continue,
                    Err(e) => return Err(cant(e)),
                };
                if let Some(noted) = &mut self.noted {
                    for chunk in &list.chunks {
                        // A chunk is read as file contents only, never through another list.
                        let held = noted.known.get(&chunk.id).copied();
                        let as_content = held.is_some_and(|h| h.content.is_some());
                        if !as_content {
                            let why = if held.is_some() {
                                Unread::OtherKind
                            } else {
                                Unread::Missing
                            };
                            noted.unread.push((chunk.id, why));
                        }
                        if !as_content || noted.watch.contains(&chunk.id) {
                            noted.via.entry(chunk.id).or_insert(list.file);
                        }
                    }
                }
                self.kept.extend(list.chunks.iter().map(|c| c.id));
            }
        }
        Ok(())
    }

    /// Marks what `state` names: its lines, the save it is at, its shelves, its tags and its
    /// server lines.
    fn state(&mut self, state: &State) -> Result<(), RepoError> {
        for save in state.lines.values() {
            self.save(*save);
        }
        if let Position::AtSave(save) = &state.position {
            self.save(*save);
        }
        for (_, save) in &state.shelves {
            self.save(*save);
        }
        if let Some(id) = state.tags
            && self.first(ObjectKind::TagList, id)
            && let Some(bytes) = self.read(&id, ObjectKind::TagList, "tag list")?
            && let Some(tags) = self.decoded(&id, "tag list", TagList::decode(&bytes))?
        {
            for tag in tags.tags.values() {
                self.save(tag.save);
            }
        }
        if let Some(id) = state.server_lines
            && self.first(ObjectKind::ServerLines, id)
            && let Some(bytes) = self.read(&id, ObjectKind::ServerLines, "server lines")?
            && let Some(servers) = self.decoded(&id, "server lines", ServerLines::decode(&bytes))?
        {
            for save in servers.servers.values().flat_map(|lines| lines.values()) {
                self.save(*save);
            }
        }
        Ok(())
    }

    /// Marks both saves of `entry`'s folder change.
    fn change(&mut self, entry: &DiaryEntry) {
        if let Some(change) = entry.folder_change {
            self.save(change.from);
            self.save(change.to);
        }
    }

    fn save(&mut self, id: Id) {
        if self.first(ObjectKind::Save, id) {
            self.saves.push(id);
        }
    }

    fn listing(&mut self, id: Id) {
        if self.first(ObjectKind::FolderListing, id) {
            self.listings.push(id);
        }
    }

    /// Reads every save and listing marked, and marks what each names, until none is left.
    fn follow(&mut self) -> Result<(), RepoError> {
        loop {
            if let Some(id) = self.saves.pop() {
                let Some(bytes) = self.read(&id, ObjectKind::Save, "save")? else {
                    continue;
                };
                let decoded = Save::decode(&bytes, self.src.signed);
                let Some(save) = self.decoded(&id, "save", decoded)? else {
                    continue;
                };
                if let Some(noted) = &mut self.noted {
                    noted.saves.push(id);
                    noted.roots.push((id, save.root));
                }
                for parent in save.parents {
                    self.save(parent);
                }
                self.listing(save.root);
            } else if let Some(id) = self.listings.pop() {
                let Some(bytes) = self.read(&id, ObjectKind::FolderListing, "folder listing")?
                else {
                    continue;
                };
                let decoded = Listing::decode(&bytes);
                let Some(listing) = self.decoded(&id, "folder listing", decoded)? else {
                    continue;
                };
                if let Some(noted) = &mut self.noted {
                    noted.listing_read(id, &listing);
                }
                for entry in listing.entries() {
                    match entry.kind {
                        // Content is kept by its fingerprint, unread; a big file's chunk lists
                        // are read once all else is marked.
                        EntryKind::File | EntryKind::Executable | EntryKind::Symlink => {
                            self.kept.insert(entry.id);
                            for &(at, line) in self.lists.get(&entry.id).into_iter().flatten() {
                                self.reads.insert((at, line.offset), line);
                            }
                        }
                        EntryKind::Folder => self.listing(entry.id),
                        EntryKind::ProjectLink => {
                            if self.first(ObjectKind::ProjectLink, entry.id) {
                                self.read(&entry.id, ObjectKind::ProjectLink, "project link")?;
                            }
                        }
                        EntryKind::Conflict => {
                            let said = format!(
                                "folder listing {id} holds a conflict, which this version of \
                                 Bolt can't follow"
                            );
                            let Some(noted) = &mut self.noted else {
                                return Err(damaged(said));
                            };
                            noted.unread.push((id, Unread::Damaged(said)));
                        }
                    }
                }
            } else {
                return Ok(());
            }
        }
    }
}

#[cfg(test)]
mod tests;
