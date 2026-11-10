//! `bolt tidy`: removes from the store what nothing needs any more, keeping what lines, shelves,
//! tags, server lines, the diary and the undo window reach.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::ffi::{OsStr, OsString};
use std::io;
use std::path::Path;
use std::sync::Arc;

use bolt_platform::dir::{Dir, EntryMeta};

use crate::diary::{Action, DiaryEntry, Position, ServerLines, State, TagList};
use crate::fsio::{self, fault_point};
use crate::id::Id;
use crate::listing::{EntryKind, Listing};
use crate::merge;
use crate::pack::{self, IndexEntry, ObjectKind, PackError, PackReader};
use crate::repo::{RepoError, Repository, cant_write, damaged};
use crate::safe_fs;
use crate::save::Save;
use crate::store::{StoreError, is_damage, is_pack_name};
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
struct Listed {
    name: OsString,
    size: u64,
    /// Changed in the 24 hours before: left as it is.
    young: bool,
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

/// Every entry in `folder` named like a pack, by name, with its details or why they can't be read.
fn packs(folder: &Dir) -> io::Result<Vec<(OsString, io::Result<EntryMeta>)>> {
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
}

/// Reads the index of each of `packs`, young ones too, and checks it against its checksum. A pack
/// that can't be opened, read or checked stops `tidy`, since it could hold the only chunk list
/// naming chunks in the others; so does a conflict record, which this version doesn't follow.
fn survey(repo: &Repository, packs: &[Listed]) -> Result<Survey, RepoError> {
    let mut survey = Survey::default();
    for (at, listed) in packs.iter().enumerate() {
        let path = repo.packs_dir().join(&listed.name);
        let cant = |e: PackError| cant_tidy(&path, &e);
        let pack = PackReader::open_in(repo.packs_folder(), &listed.name, &path).map_err(cant)?;
        for line in pack.entries() {
            let line = line.map_err(cant)?;
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
                ObjectKind::Conflict => {
                    return Err(RepoError::CantTidy {
                        path: path.clone(),
                        why: None,
                    });
                }
                _ => {}
            }
        }
        pack.check_index().map_err(cant)?;
    }
    Ok(survey)
}

/// What must be kept: every object reached (fingerprints), and every chunk that a chunk list of a
/// file reached, or one in a young pack, names. `packs` are the store's packs; the undo window
/// starts at `since`.
fn kept(repo: &Repository, packs: &[Listed], since: i64) -> Result<HashSet<Id>, RepoError> {
    let Survey {
        entries,
        lists,
        reads,
    } = survey(repo, packs)?;
    let (head, _) = repo.current()?;
    let mut marks = Marks {
        repo,
        kept: HashSet::new(),
        followed: HashSet::new(),
        saves: Vec::new(),
        listings: Vec::new(),
        lists: &lists,
        reads,
        stored: entries.clone(),
        gap: None,
    };
    let mut walked = HashSet::new();
    marks.diary(head, since, &mut walked)?;
    // A diary cut off from `head` (a rolled-back `head`, a crash's leftover) lives out the window.
    for (end, entry) in undo::ends_among(repo, entries)? {
        if entry.who.time_ms >= since
            && let Err(e) = marks.diary(end, since, &mut walked)
        {
            // Only an entry the store lacks is passed over; what was reached stays kept.
            let Some(missing) = marks.gap.take() else {
                return Err(e);
            };
            repo.store().note(Warning::new(
                warning::DIARY_END_UNREADABLE,
                format!(
                    "The diary that ends at entry {}, cut off from the current one, can't be read \
                     back to its start: entry {} isn't in the store. Tidy kept what it could \
                     reach from that end and went on.",
                    end.short_hex(),
                    missing.short_hex()
                ),
            ));
        }
    }
    marks.follow()?;
    marks.chunks(packs)?;
    Ok(marks.kept)
}

/// What `kept` has found so far, and what it has still to read.
struct Marks<'a> {
    repo: &'a Repository,
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
    stored: HashSet<Id>,
    /// An entry a diary named that the store doesn't hold, once one stopped marking.
    gap: Option<Id>,
}

impl Marks<'_> {
    /// Marks the diary back from `start`, as undo can reach it. `walked` holds the entries already
    /// marked, with whether their state was kept, so that diaries sharing a past are read once.
    fn diary(
        &mut self,
        start: Id,
        since: i64,
        walked: &mut HashSet<(Id, bool)>,
    ) -> Result<(), RepoError> {
        // The start's own state is always kept; an older entry's when the one after it is within
        // the window, since undoing that one puts it back.
        let (mut at, mut after_within) = (Some(start), true);
        while let Some(id) = at {
            if walked.contains(&(id, true)) || !walked.insert((id, after_within)) {
                return Ok(());
            }
            let entry = self.entry(&id)?;
            let within = entry.who.time_ms >= since;
            if within || after_within {
                self.state(&entry.state)?;
            }
            if within {
                self.change(&entry);
                if let Action::Undo(step) | Action::Redo(step) = entry.action {
                    let step = self.entry(&step)?;
                    self.state(&step.state)?;
                    self.change(&step);
                    if let Some(before) = step.parent {
                        let before = self.entry(&before)?;
                        self.state(&before.state)?;
                    }
                }
            }
            after_within = within;
            at = entry.parent;
        }
        Ok(())
    }

    /// Keeps `id`, and gives whether a record of `kind` with it is still to be followed.
    fn first(&mut self, kind: ObjectKind, id: Id) -> bool {
        self.kept.insert(id);
        self.followed.insert((kind as u8, id))
    }

    /// Keeps the diary entry `id` and reads it.
    fn entry(&mut self, id: &Id) -> Result<DiaryEntry, RepoError> {
        self.kept.insert(*id);
        if !self.stored.contains(id) {
            self.gap = Some(*id);
            return Err(damaged(format!("the diary has no entry {id}")));
        }
        self.repo.entry(id)
    }

    /// Reads the record `id` of `kind`, which must be kept.
    fn read(&self, id: &Id, kind: ObjectKind, what: &str) -> Result<Vec<u8>, RepoError> {
        self.repo
            .store()
            .read_record(id, kind)
            .map_err(|e| match e {
                // Memory or I/O: the copy may be fine, so this isn't damage.
                StoreError::Pack(e) if !is_damage(&e) => RepoError::Io(io::Error::other(format!(
                    "Bolt couldn't read {what} {id}, which must be kept: {e}"
                ))),
                e => damaged(format!("{what} {id} can't be read ({e})")),
            })
    }

    /// Keeps every chunk the lists to read name, opening each pack once. A list names only
    /// chunks, so nothing more is followed. One that can't be read stops `tidy`, naming its pack.
    fn chunks(&mut self, packs: &[Listed]) -> Result<(), RepoError> {
        for (at, listed) in packs.iter().enumerate() {
            let mut lines = self.reads.range((at, 0)..=(at, u64::MAX)).peekable();
            if lines.peek().is_none() {
                continue;
            }
            let path = self.repo.packs_dir().join(&listed.name);
            let cant = |e: PackError| cant_tidy(&path, &e);
            let pack =
                PackReader::open_in(self.repo.packs_folder(), &listed.name, &path).map_err(cant)?;
            for (_, line) in lines {
                let list = pack.read_chunk_list(line).map_err(cant)?;
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
        {
            let bytes = self.read(&id, ObjectKind::TagList, "tag list")?;
            let tags = TagList::decode(&bytes)
                .map_err(|e| damaged(format!("tag list {id} can't be read ({e})")))?;
            for tag in tags.tags.values() {
                self.save(tag.save);
            }
        }
        if let Some(id) = state.server_lines
            && self.first(ObjectKind::ServerLines, id)
        {
            let bytes = self.read(&id, ObjectKind::ServerLines, "server lines")?;
            let servers = ServerLines::decode(&bytes)
                .map_err(|e| damaged(format!("server lines {id} can't be read ({e})")))?;
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
        let signed = self.repo.signatures_allowed();
        loop {
            if let Some(id) = self.saves.pop() {
                let bytes = self.read(&id, ObjectKind::Save, "save")?;
                let save = Save::decode(&bytes, signed)
                    .map_err(|e| damaged(format!("save {id} can't be read ({e})")))?;
                for parent in save.parents {
                    self.save(parent);
                }
                self.listing(save.root);
            } else if let Some(id) = self.listings.pop() {
                let bytes = self.read(&id, ObjectKind::FolderListing, "folder listing")?;
                let listing = Listing::decode(&bytes)
                    .map_err(|e| damaged(format!("folder listing {id} can't be read ({e})")))?;
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
                            return Err(damaged(format!(
                                "folder listing {id} holds a conflict, which this version of \
                                 Bolt can't follow"
                            )));
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
