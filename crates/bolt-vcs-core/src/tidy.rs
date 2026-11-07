//! `bolt tidy`: removes from the store what nothing needs any more, keeping what lines, shelves,
//! tags, server lines, the diary and the undo window reach.

use std::collections::HashSet;
use std::ffi::{OsStr, OsString};
use std::io;
use std::path::Path;
use std::sync::Arc;

use bolt_platform::dir::{Dir, EntryMeta, EntryType};

use crate::diary::{Action, DiaryEntry, Position, ServerLines, State, TagList};
use crate::fsio::{self, fault_point};
use crate::id::Id;
use crate::listing::{EntryKind, Listing};
use crate::merge::{self, Failed};
use crate::pack::{self, ObjectKind, PackError, PackReader};
use crate::repo::{RepoError, Repository, damaged};
use crate::save::Save;
use crate::store::{StoreError, is_damage};
use crate::text::{escape, thousands};
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

/// What a pack's index says it holds.
#[derive(Default)]
struct Holds {
    /// A kind of object this version doesn't follow, if it holds one.
    unfollowed: Option<ObjectKind>,
    /// Why its index can't be trusted to judge it by: it fails its checksum.
    untrusted: Option<PackError>,
    version: u32,
    /// How many of its objects are kept, and their stored bytes.
    kept: u64,
    kept_stored: u64,
    /// Whether it holds anything not kept.
    other: bool,
    uncompressed: u64,
}

/// Removes what nothing needs any more: what no line, shelf, tag or server line reaches, and what
/// only steps older than `window_days` before `now_ms` needed. Packs changed in the 24 hours
/// before `now_ms` are left as they are. `on_estimate` is told about how much that frees before
/// anything is removed. Anything that must be kept but can't be read stops it before then, and so
/// does a chunk list or a conflict record, which it can't follow.
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
    let keep = kept(repo, window_days, now_ms)?;
    fault_point("tidy:marked")?;

    let folder = Arc::clone(repo.packs_folder());
    let packs_dir = repo.packs_dir().to_owned();
    let now_ns = i128::from(now_ms).saturating_mul(1_000_000);
    let mut report = TidyReport::default();
    let (mut unread, mut rewrite) = (Vec::new(), Vec::new());
    let (mut moved, mut unfollowed) = (0u64, None);
    // An open file can't always be replaced or removed on Windows.
    repo.store().close_all();
    for (name, meta) in packs(&folder)? {
        let path = packs_dir.join(&name);
        report.bytes_before = report.bytes_before.saturating_add(meta.size);
        let young = now_ns.saturating_sub(meta.modified_ns) < GRACE_NS;
        // A young pack is read too: a chunk list there can name chunks in older packs.
        let holds = judge(&folder, &name, &path, &keep, !young);
        if let Ok(Holds {
            unfollowed: Some(kind),
            ..
        }) = &holds
        {
            unfollowed.get_or_insert((path.clone(), *kind));
        }
        if young {
            report.packs_young += 1;
            continue;
        }
        let holds = match holds {
            Ok(Holds {
                untrusted: Some(e), ..
            })
            | Err(e) => {
                not_tidied(repo, &path, &e);
                continue;
            }
            Ok(holds) => holds,
        };
        if holds.kept == 0 {
            report.to_free = report.to_free.saturating_add(meta.size);
            unread.push(name);
        } else if holds.other
            || holds.version == 1
            || merge::is_small(meta.size, holds.uncompressed)
        {
            let freed = meta.size.saturating_sub(holds.kept_stored);
            report.to_free = report.to_free.saturating_add(freed);
            moved = moved.saturating_add(holds.kept);
            rewrite.push((name, holds.kept));
        }
    }
    if let Some((path, kind)) = unfollowed {
        let what = match kind {
            ObjectKind::ChunkList => "a chunk list",
            _ => "a conflict record",
        };
        return Err(RepoError::CantTidy { path, what });
    }
    if moved > 0 {
        report.to_free = report.to_free.saturating_sub(pack::v2_overhead(moved));
    }
    on_estimate(report.to_free);

    // Before anything is written, so that tidying frees space even on a full disk.
    remove(repo, &folder, &packs_dir, unread, &mut report);
    let mut out = repo.new_packs()?;
    let mut taken = Vec::new();
    for (name, kept) in rewrite {
        let path = packs_dir.join(&name);
        fsio::act_point("tidy:copying");
        match merge::copy_pack(&folder, &name, &path, &mut out, &|id| keep.contains(id)) {
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
            Err(Failed::Read(e)) => not_tidied(repo, &path, &e),
            Err(Failed::Write(e)) => return Err(e.into()),
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
        .iter()
        .fold(0u64, |sum, (_, meta)| sum.saturating_add(meta.size));
    Ok(report)
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

/// Every `.pack` file in `folder`, by name, with its details. Anything else, and one whose details
/// can't be read, is left for reads to report.
fn packs(folder: &Dir) -> Result<Vec<(OsString, EntryMeta)>, RepoError> {
    let mut found = Vec::new();
    for entry in folder.entries()? {
        if Path::new(&entry.name)
            .extension()
            .is_some_and(|e| e == "pack")
            && let Ok(meta) = folder.metadata(&entry.name)
            && meta.kind == EntryType::File
        {
            found.push((entry.name, meta));
        }
    }
    found.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(found)
}

/// What the index of the pack `name` says it holds, each line checked as it is read. With
/// `judged`, the whole index is checked against its checksum too: reads check each object
/// against its ID, but tidying deletes by the index alone.
fn judge(
    folder: &Dir,
    name: &OsStr,
    path: &Path,
    keep: &HashSet<Id>,
    judged: bool,
) -> Result<Holds, PackError> {
    let pack = PackReader::open_in(folder, name, path)?;
    let mut holds = Holds {
        version: pack.version(),
        ..Holds::default()
    };
    for entry in pack.entries() {
        let entry = entry?;
        if matches!(entry.kind, ObjectKind::ChunkList | ObjectKind::Conflict) {
            holds.unfollowed.get_or_insert(entry.kind);
        }
        holds.uncompressed = holds.uncompressed.saturating_add(entry.uncompressed_size);
        if keep.contains(&entry.id) {
            holds.kept = holds.kept.saturating_add(1);
            holds.kept_stored = holds.kept_stored.saturating_add(entry.stored_size);
        } else {
            holds.other = true;
        }
    }
    if judged {
        holds.untrusted = pack.check_index().err();
    }
    Ok(holds)
}

/// A pack's path, as messages show it.
fn shown(path: &Path) -> String {
    escape(&path.display().to_string(), false)
}

/// Notes that the pack at `path` is left as it is, since reading it failed with `e`.
fn not_tidied(repo: &Repository, path: &Path, e: &PackError) {
    let why = match e {
        PackError::Corrupt { reason, .. } => reason.clone(),
        other => other.to_string(),
    };
    let why = escape(why.trim_end_matches('.'), false);
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

/// What must be kept: every object reached (fingerprints).
pub(crate) fn kept(
    repo: &Repository,
    window_days: u32,
    now_ms: i64,
) -> Result<HashSet<Id>, RepoError> {
    let (head, _) = repo.current()?;
    let since = now_ms.saturating_sub(i64::from(window_days).saturating_mul(DAY_MS));
    let mut marks = Marks {
        repo,
        kept: HashSet::new(),
        followed: HashSet::new(),
        saves: Vec::new(),
        listings: Vec::new(),
    };
    // The head's own state is always kept; an older entry's when the one after it is within the
    // window, since undoing that one puts it back.
    let (mut at, mut after_within) = (Some(head), true);
    while let Some(id) = at {
        let entry = marks.entry(&id)?;
        let within = entry.who.time_ms >= since;
        if within || after_within {
            marks.state(&entry.state)?;
        }
        if within {
            marks.change(&entry);
            if let Action::Undo(step) | Action::Redo(step) = entry.action {
                let step = marks.entry(&step)?;
                marks.state(&step.state)?;
                marks.change(&step);
                if let Some(before) = step.parent {
                    let before = marks.entry(&before)?;
                    marks.state(&before.state)?;
                }
            }
        }
        after_within = within;
        at = entry.parent;
    }
    marks.follow()?;
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
}

impl Marks<'_> {
    /// Keeps `id`, and gives whether a record of `kind` with it is still to be followed.
    fn first(&mut self, kind: ObjectKind, id: Id) -> bool {
        self.kept.insert(id);
        self.followed.insert((kind as u8, id))
    }

    /// Keeps the diary entry `id` and reads it.
    fn entry(&mut self, id: &Id) -> Result<DiaryEntry, RepoError> {
        self.kept.insert(*id);
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
                        // Content is kept by its fingerprint, unread.
                        EntryKind::File | EntryKind::Executable | EntryKind::Symlink => {
                            self.kept.insert(entry.id);
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
