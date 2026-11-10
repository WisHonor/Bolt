//! `bolt verify`: reads every object of every pack back, then works out what the project needs,
//! as `tidy` does, and looks for each of those in the packs. It takes no lock and changes nothing.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::ffi::OsString;
use std::io;
use std::path::Path;
use std::sync::Arc;

use bolt_platform::dir::Dir;

use crate::diary::{DiaryEntry, ServerLines, TagList};
use crate::encoding::DecodeError;
use crate::id::Id;
use crate::link::ProjectLink;
use crate::listing::{EntryKind, Listing};
use crate::pack::{ENTRY_HEADER_LEN, IndexEntry, ObjectKind, PackError, PackReader};
use crate::repo::{RepoError, Repository, head_entry};
use crate::save::Save;
use crate::store::{Store, is_damage};
use crate::text::escape;
use crate::tidy::{self, Noted, Source, Unread};
use crate::warning::Warning;
use crate::writeout;

/// What is wrong.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProblemKind {
    PackDamaged,
    PackUnreadable,
    ObjectDamaged,
    ObjectMissing,
    HeadLost,
}

impl ProblemKind {
    /// The kind as `--json` names it.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            ProblemKind::PackDamaged => "pack_damaged",
            ProblemKind::PackUnreadable => "pack_unreadable",
            ProblemKind::ObjectDamaged => "object_damaged",
            ProblemKind::ObjectMissing => "object_missing",
            ProblemKind::HeadLost => "head_lost",
        }
    }
}

/// One thing found wrong. `message` is escaped, so it is safe to print; `pack` and the path in
/// `needed_by` are as the project has them.
#[derive(Clone, Debug)]
pub struct Problem {
    pub kind: ProblemKind,
    /// The pack's file name.
    pub pack: Option<String>,
    pub object: Option<Id>,
    /// One save, and the path in it, that need the object.
    pub needed_by: Option<(Id, String)>,
    pub message: String,
}

/// How many problems are kept; the rest are only counted.
pub const PROBLEMS_KEPT: usize = 1_000;

/// What `verify` checked and found.
#[derive(Clone, Debug, Default)]
pub struct Verified {
    pub packs: u64,
    pub objects: u64,
    /// The packs' size on disk.
    pub bytes: u64,
    /// Saves the project needs, read.
    pub saves: u64,
    /// Diary entries read, from the current one back.
    pub entries: u64,
    pub lines: u64,
    pub shelves: u64,
    pub problems_total: u64,
    /// How many problems of each kind were found, kept or not.
    pub pack_damaged: u64,
    pub pack_unreadable: u64,
    pub object_damaged: u64,
    pub object_missing: u64,
    pub head_lost: u64,
    pub problems: Vec<Problem>,
    /// Objects that failed when the packs were read back, their own check.
    pub objects_failed: u64,
    /// Why the packs couldn't be listed, when they couldn't.
    pub unlisted: Option<String>,
    /// Whether what the project needs was worked out: not when `head` was lost, or the check
    /// couldn't be made.
    pub needs_checked: bool,
    /// Why what the project needs couldn't be worked out, and the kind it is told as.
    pub needs_failure: Option<(ProblemKind, String)>,
    pub warnings: Vec<Warning>,
}

impl Verified {
    fn found(&mut self, problem: Problem) {
        self.problems_total = self.problems_total.saturating_add(1);
        let count = match problem.kind {
            ProblemKind::PackDamaged => &mut self.pack_damaged,
            ProblemKind::PackUnreadable => &mut self.pack_unreadable,
            ProblemKind::ObjectDamaged => &mut self.object_damaged,
            ProblemKind::ObjectMissing => &mut self.object_missing,
            ProblemKind::HeadLost => &mut self.head_lost,
        };
        *count = count.saturating_add(1);
        if self.problems.len() < PROBLEMS_KEPT {
            self.problems.push(problem);
        }
    }

    /// How many problems of `kind` were found, kept or not.
    #[must_use]
    pub fn count(&self, kind: ProblemKind) -> u64 {
        match kind {
            ProblemKind::PackDamaged => self.pack_damaged,
            ProblemKind::PackUnreadable => self.pack_unreadable,
            ProblemKind::ObjectDamaged => self.object_damaged,
            ProblemKind::ObjectMissing => self.object_missing,
            ProblemKind::HeadLost => self.head_lost,
        }
    }
}

/// How many times the packs are listed again to check those that arrived while it checked.
const ROUNDS: usize = 8;

/// Checks every pack, then that the project has all it needs. Takes no lock and changes nothing.
/// `progress` hears bytes of pack read and bytes in all.
pub fn verify(
    repo: &Repository,
    window_days: u32,
    now_ms: i64,
    progress: &mut dyn FnMut(u64, u64),
) -> Verified {
    let mut v = Verified::default();
    v.warnings.extend(writeout::unfinished_warning(repo));
    let (folder, packs_dir) = (repo.packs_folder(), repo.packs_dir());
    let damaged = check_packs(
        folder,
        packs_dir,
        repo.signatures_allowed(),
        true,
        &mut v,
        progress,
    );
    // A problem with what the project needs is told only when two looks agree on it: a merge or
    // a tidy beside this one writes its new packs before it removes an old one, and a save its
    // packs before it moves `head`.
    let (verdict, look) = agreed_needs(repo, window_days, now_ms, &damaged);
    let (kind, message) = match verdict {
        Verdict::Found(missing) => {
            if let Some(look) = look {
                v.needs_checked = true;
                told_needs(&mut v, &look, &missing, &damaged);
                name_needers(&mut v, &look, repo.signatures_allowed());
            }
            return v;
        }
        Verdict::Lost(message) => (ProblemKind::HeadLost, message),
        Verdict::Failed(kind, message) => (kind, message),
    };
    v.needs_failure = Some((kind, message.clone()));
    v.found(Problem {
        kind,
        pack: None,
        object: None,
        needed_by: None,
        message,
    });
    v
}

/// What a check that reads only the packs found.
pub struct VerifyReport {
    pub objects_checked: u64,
    pub problems: Vec<String>,
}

impl VerifyReport {
    #[must_use]
    pub fn is_ok(&self) -> bool {
        self.problems.is_empty()
    }
}

/// Checks every pack in the folder at `packs_dir` as `verify` does, and nothing more: each object
/// against its fingerprint and its kind, and each chunked file rebuilt. Each problem starts with
/// its pack's name, escaped.
#[must_use]
pub fn verify_store(packs_dir: &Path) -> VerifyReport {
    let mut v = Verified::default();
    match Dir::open_top(packs_dir) {
        Ok(folder) => {
            check_packs(
                &Arc::new(folder),
                packs_dir,
                true,
                false,
                &mut v,
                &mut |_, _| {},
            );
        }
        Err(e) => {
            let folder = escape(&packs_dir.display().to_string(), false);
            return VerifyReport {
                objects_checked: 0,
                problems: vec![format!("{folder}: {e}")],
            };
        }
    }
    let problems = v.problems.iter().map(|p| {
        let pack = escape(p.pack.as_deref().unwrap_or_default(), false);
        match p.object {
            Some(id) => format!("{pack}: object {id}: {}", p.message),
            None => format!("{pack}: {}", p.message),
        }
    });
    VerifyReport {
        objects_checked: v.objects,
        problems: problems.collect(),
    }
}

/// Checks every pack in `folder`, at `packs_dir`, listing them again until a listing brings
/// nothing new, then rebuilds every chunked file from its chunks. With `skip_missing_chunks`, a
/// list naming a chunk no pack holds is left to the check of what the project needs. Gives the
/// objects found damaged.
fn check_packs(
    folder: &Arc<Dir>,
    packs_dir: &Path,
    signed: bool,
    skip_missing_chunks: bool,
    v: &mut Verified,
    progress: &mut dyn FnMut(u64, u64),
) -> HashSet<Id> {
    let mut found = Found::default();
    let (mut seen, mut done, mut total) = (HashSet::new(), 0u64, 0u64);
    for _ in 0..ROUNDS {
        let listed = match tidy::packs(folder) {
            Ok(listed) => listed,
            Err(e) => {
                let message = format!(
                    "Bolt couldn't list the packs in {}: {}.",
                    escape(&packs_dir.display().to_string(), false),
                    escape(e.to_string().trim_end_matches('.'), false)
                );
                v.unlisted = Some(message.clone());
                v.found(Problem {
                    kind: ProblemKind::PackUnreadable,
                    pack: None,
                    object: None,
                    needed_by: None,
                    message,
                });
                break;
            }
        };
        let new: Vec<_> = listed
            .into_iter()
            .filter(|(name, _)| seen.insert(name.clone()))
            .collect();
        if new.is_empty() {
            break;
        }
        for (_, meta) in &new {
            let size = meta.as_ref().map_or(0, |m| m.size);
            total = total.saturating_add(size);
        }
        for (name, meta) in new {
            let size = meta.as_ref().map_or(0, |m| m.size);
            let start = done;
            let mut at = |read: u64| progress(start.saturating_add(read.min(size)), total);
            if check_pack(folder, packs_dir, &name, signed, v, &mut found, &mut at) {
                v.packs = v.packs.saturating_add(1);
                v.bytes = v.bytes.saturating_add(size);
            }
            done = done.saturating_add(size);
            progress(done, total);
        }
    }
    rebuild(folder, packs_dir, skip_missing_chunks, v, &mut found);
    found.damaged
}

/// What checking the packs found that the later steps need.
#[derive(Default)]
struct Found {
    /// Objects found damaged.
    damaged: HashSet<Id>,
    /// Content read back whole.
    content: HashSet<Id>,
    /// Each chunk list whose structure checked out, by its pack, to rebuild its file.
    lists: Vec<(OsString, IndexEntry)>,
}

/// Checks the pack `name`: its index, against its checksum, then each object in it. `at` hears
/// how many of its bytes are read. Gives whether it was there: one gone was merged or tidied away.
fn check_pack(
    folder: &Dir,
    packs_dir: &Path,
    name: &OsString,
    signed: bool,
    v: &mut Verified,
    found: &mut Found,
    at: &mut dyn FnMut(u64),
) -> bool {
    let shown = name.to_string_lossy().into_owned();
    let pack = match PackReader::open_in(folder, name, &packs_dir.join(name)) {
        Ok(pack) => pack,
        Err(e) if is_gone(&e) => return false,
        Err(e) => {
            v.found(pack_problem(&shown, &e));
            return true;
        }
    };
    let mut told = false;
    if let Err(e) = pack.check_index() {
        v.found(pack_problem(&shown, &e));
        told = true;
    }
    let mut read = 0u64;
    for line in pack.entries() {
        let entry = match line {
            Ok(entry) => entry,
            Err(e) => {
                if !told {
                    v.found(pack_problem(&shown, &e));
                }
                break;
            }
        };
        v.objects = v.objects.saturating_add(1);
        match check_object(&pack, &entry, signed) {
            Ok(()) if entry.kind == ObjectKind::ChunkList => {
                found.lists.push((name.clone(), entry))
            }
            Ok(()) if entry.kind == ObjectKind::Content => {
                found.content.insert(entry.id);
            }
            Ok(()) => {}
            Err((kind, message)) => {
                if kind == ProblemKind::ObjectDamaged {
                    found.damaged.insert(entry.id);
                }
                v.objects_failed = v.objects_failed.saturating_add(1);
                v.found(Problem {
                    kind,
                    pack: Some(shown.clone()),
                    object: Some(entry.id),
                    needed_by: None,
                    message,
                });
            }
        }
        read = read.saturating_add(ENTRY_HEADER_LEN.saturating_add(entry.stored_size));
        at(read);
    }
    true
}

/// Reads the object `entry` names in `pack` back: content against its fingerprint, a record
/// against its fingerprint and as its kind, a chunk list by its structure. A conflict record,
/// which nothing writes yet, by its fingerprint only.
fn check_object(
    pack: &PackReader,
    entry: &IndexEntry,
    signed: bool,
) -> Result<(), (ProblemKind, String)> {
    match entry.kind {
        ObjectKind::Content => pack.copy_content(entry, &mut io::sink()),
        ObjectKind::ChunkList => pack.read_chunk_list(entry).map(drop),
        kind => {
            let bytes = pack.read_record(entry).map_err(|e| object_problem(&e))?;
            return decodes(kind, &bytes, signed).map_err(|e| {
                let said = format!(
                    "It doesn't read as {}: {}.",
                    what(kind),
                    escape(e.to_string().trim_end_matches('.'), false)
                );
                (ProblemKind::ObjectDamaged, said)
            });
        }
    }
    .map_err(|e| object_problem(&e))
}

/// Decodes `bytes` as a record of `kind`, with the decoder that kind has.
fn decodes(kind: ObjectKind, bytes: &[u8], signed: bool) -> Result<(), DecodeError> {
    match kind {
        ObjectKind::FolderListing => Listing::decode(bytes).map(drop),
        ObjectKind::Save => Save::decode(bytes, signed).map(drop),
        ObjectKind::DiaryEntry => DiaryEntry::decode(bytes).map(drop),
        ObjectKind::ProjectLink => ProjectLink::decode(bytes).map(drop),
        ObjectKind::TagList => TagList::decode(bytes).map(drop),
        ObjectKind::ServerLines => ServerLines::decode(bytes).map(drop),
        ObjectKind::Content | ObjectKind::ChunkList | ObjectKind::Conflict => Ok(()),
    }
}

/// A record of `kind`, as a message names it.
fn what(kind: ObjectKind) -> &'static str {
    match kind {
        ObjectKind::FolderListing => "a folder listing",
        ObjectKind::Save => "a save",
        ObjectKind::DiaryEntry => "a diary entry",
        ObjectKind::ProjectLink => "a project link",
        ObjectKind::TagList => "a tag list",
        ObjectKind::ServerLines => "server lines",
        ObjectKind::Content | ObjectKind::ChunkList | ObjectKind::Conflict => "a record",
    }
}

/// Rebuilds the file each chunk list in `found` names from its chunks. A list naming a chunk no
/// pack holds is passed over when `skip_missing_chunks`, and so is one naming a damaged chunk,
/// which is told already.
fn rebuild(
    folder: &Arc<Dir>,
    packs_dir: &Path,
    skip_missing_chunks: bool,
    v: &mut Verified,
    found: &mut Found,
) {
    let Ok(store) = Store::open_in(Arc::clone(folder), packs_dir) else {
        return;
    };
    let mut open: Option<(OsString, PackReader)> = None;
    for (name, entry) in std::mem::take(&mut found.lists) {
        if open.as_ref().is_none_or(|(at, _)| *at != name) {
            let pack = PackReader::open_in(folder, &name, &packs_dir.join(&name));
            open = pack.ok().map(|pack| (name.clone(), pack));
        }
        let Some((_, pack)) = &open else {
            continue;
        };
        let Ok(list) = pack.read_chunk_list(&entry) else {
            continue;
        };
        let missing = |c: &crate::chunks::Chunk| !found.content.contains(&c.id);
        if list.chunks.iter().any(|c| found.damaged.contains(&c.id))
            || skip_missing_chunks && list.chunks.iter().any(missing)
        {
            continue;
        }
        if let Err(e) = store.copy_chunks(&list, pack.path(), &mut io::sink()) {
            let (kind, message) = object_problem(&e);
            if kind == ProblemKind::ObjectDamaged {
                found.damaged.insert(entry.id);
            }
            v.objects_failed = v.objects_failed.saturating_add(1);
            v.found(Problem {
                kind,
                pack: Some(name.to_string_lossy().into_owned()),
                object: Some(entry.id),
                needed_by: None,
                message,
            });
        }
    }
}

/// A pack gone since it was listed: merged into another, or tidied away.
fn is_gone(e: &PackError) -> bool {
    matches!(e, PackError::Io(e) if e.kind() == io::ErrorKind::NotFound)
}

/// Why `e` happened, as a message says it, escaped.
fn why(e: &PackError) -> String {
    let why = match e {
        PackError::Corrupt { reason, .. } => reason.clone(),
        other => other.to_string(),
    };
    escape(why.trim_end_matches('.'), false)
}

/// The problem with the pack `name`, which can't be opened or read: `e`.
fn pack_problem(name: &str, e: &PackError) -> Problem {
    let (kind, message) = if is_damage(e) {
        (
            ProblemKind::PackDamaged,
            format!("The pack is damaged: {}.", why(e)),
        )
    } else {
        (
            ProblemKind::PackUnreadable,
            format!("Bolt couldn't read the pack: {}.", why(e)),
        )
    };
    Problem {
        kind,
        pack: Some(name.to_owned()),
        object: None,
        needed_by: None,
        message,
    }
}

/// What reading an object back failing with `e` says.
fn object_problem(e: &PackError) -> (ProblemKind, String) {
    match e {
        PackError::FingerprintMismatch { .. } => (
            ProblemKind::ObjectDamaged,
            "It doesn't match its fingerprint.".to_owned(),
        ),
        e if is_damage(e) => (
            ProblemKind::ObjectDamaged,
            format!("It is damaged: {}.", why(e)),
        ),
        PackError::ContentChanged | PackError::TooLarge => (
            ProblemKind::ObjectDamaged,
            format!("It is damaged: {}.", why(e)),
        ),
        e => (
            ProblemKind::PackUnreadable,
            format!("Bolt couldn't read it: {}.", why(e)),
        ),
    }
}

/// What a look at what the project needs concluded, kept once its working sets are dropped.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Verdict {
    /// What the project needs that no pack holds.
    Found(BTreeSet<Id>),
    /// `state/head` names nothing Bolt can read: why.
    Lost(String),
    /// The look couldn't be made: the kind it is told as, and why.
    Failed(ProblemKind, String),
}

impl Verdict {
    /// The kind of problem it would be told as; `None` for a look that found the head.
    fn told_as(&self) -> Option<ProblemKind> {
        match self {
            Verdict::Found(_) => None,
            Verdict::Lost(_) => Some(ProblemKind::HeadLost),
            Verdict::Failed(kind, _) => Some(*kind),
        }
    }

    /// Whether two looks agree: both found the head, or both would be told as the same problem.
    fn same_kind(&self, other: &Verdict) -> bool {
        self.told_as() == other.told_as()
    }

    /// Two verdicts of the same kind as one: missing only what both found missing, else the later.
    fn with(&self, later: &Verdict) -> Verdict {
        match (self, later) {
            (Verdict::Found(a), Verdict::Found(b)) => Verdict::Found(a & b),
            _ => later.clone(),
        }
    }
}

/// What the looks so far agree on, or `None` while another is needed: two of the same kind are
/// combined; of three, the two that share a kind, or the last when none do.
fn agreed(looks: &[Verdict]) -> Option<Verdict> {
    match looks {
        [a, b] => a.same_kind(b).then(|| a.with(b)),
        [a, b, c] => {
            let pair = [(a, c), (b, c), (a, b)]
                .into_iter()
                .find(|(x, y)| x.same_kind(y));
            Some(pair.map_or_else(|| c.clone(), |(x, y)| x.with(y)))
        }
        _ => looks.last().cloned(),
    }
}

/// Looks at what the project needs until two looks agree, at most three times. A first look that
/// finds nothing missing is enough: no problem comes from it. Gives the verdict, and the last
/// look's working sets when it found the head.
fn agreed_needs(
    repo: &Repository,
    window_days: u32,
    now_ms: i64,
    damaged: &HashSet<Id>,
) -> (Verdict, Option<Box<Look>>) {
    let mut verdicts = Vec::new();
    let mut last: Option<Box<Look>> = None;
    loop {
        // Gone before the next look begins, so two looks never hold their sets at once.
        drop(last.take());
        let (verdict, look) = look(repo, window_days, now_ms, damaged);
        last = look;
        let clean = matches!(&verdict, Verdict::Found(missing) if missing.is_empty());
        verdicts.push(verdict);
        let done = match verdicts.as_slice() {
            [only] if clean => Some(only.clone()),
            [_] => None,
            looks => agreed(looks),
        };
        if let Some(verdict) = done {
            return (verdict, last);
        }
    }
}

/// One look's working sets, when it found the head.
struct Look {
    /// The store the look read from, listed when it began.
    store: Store,
    kept: HashSet<Id>,
    noted: Noted,
    lines: u64,
    shelves: u64,
}

/// Why a look couldn't be made.
enum Unmade {
    /// `state/head` couldn't be read, though it is there.
    Head(RepoError),
    /// The packs, or a pack, couldn't be read.
    Packs(RepoError),
}

/// Works out what the project needs from `state/head`, read again, as `tidy` does with the same
/// window, and what of it no pack holds, with the packs listed again. `damaged` are the objects
/// found damaged, whose files are named too.
fn look(
    repo: &Repository,
    window_days: u32,
    now_ms: i64,
    damaged: &HashSet<Id>,
) -> (Verdict, Option<Box<Look>>) {
    match look_at(repo, window_days, now_ms, damaged) {
        Ok(Ok((missing, look))) => (Verdict::Found(missing), Some(look)),
        Ok(Err(lost)) => (Verdict::Lost(lost.to_string()), None),
        Err(unmade) => {
            let (kind, e) = match unmade {
                Unmade::Head(e) => (ProblemKind::HeadLost, e),
                Unmade::Packs(e) => (ProblemKind::PackUnreadable, e),
            };
            let why = escape(e.to_string().trim_end_matches('.'), false);
            let said = format!("What the project needs couldn't be checked: {why}.");
            (Verdict::Failed(kind, said), None)
        }
    }
}

/// [`look`]: what is missing and the look's sets, or a lost head, or why it couldn't be made.
#[allow(clippy::type_complexity)]
fn look_at(
    repo: &Repository,
    window_days: u32,
    now_ms: i64,
    damaged: &HashSet<Id>,
) -> Result<Result<(BTreeSet<Id>, Box<Look>), RepoError>, Unmade> {
    // `head` before the packs are listed: whatever it names was stored before the listing began.
    let head = match repo.head() {
        Ok(head) => head,
        Err(e @ RepoError::HeadLost { .. }) => return Ok(Err(e)),
        Err(e) => return Err(Unmade::Head(e)),
    };
    let packs_failed = |e: RepoError| Unmade::Packs(e);
    let store = Store::open_in(Arc::clone(repo.packs_folder()), repo.packs_dir())
        .map_err(|e| packs_failed(e.into()))?;
    let (head, entry) = match head_entry(&store, head) {
        Ok(found) => found,
        Err(e @ RepoError::HeadLost { .. }) => return Ok(Err(e)),
        Err(e) => return Err(packs_failed(e)),
    };
    let now_ns = i128::from(now_ms).saturating_mul(1_000_000);
    let packs =
        tidy::listed_leniently(repo.packs_folder(), now_ns).map_err(|e| packs_failed(e.into()))?;
    let since = tidy::window_start(repo, window_days, now_ms);
    let src = Source {
        store: &store,
        signed: repo.signatures_allowed(),
        folder: repo.packs_folder(),
        packs_dir: repo.packs_dir(),
    };
    let watch = Noted {
        watch: damaged.clone(),
        ..Noted::default()
    };
    let (kept, noted) = tidy::needs(&src, &packs, head, since, watch).map_err(packs_failed)?;
    let mut missing: BTreeSet<Id> = kept
        .iter()
        .filter(|id| !noted.known.contains(id))
        .copied()
        .collect();
    let unread = noted.unread.iter();
    missing.extend(
        unread
            .filter(|(_, why)| matches!(why, Unread::Missing | Unread::OtherKind))
            .map(|(id, _)| *id),
    );
    let look = Look {
        store,
        kept,
        noted,
        lines: entry.state.lines.len() as u64,
        shelves: entry.state.shelves.len() as u64,
    };
    Ok(Ok((missing, Box::new(look))))
}

/// Tells what `look` found the project needs but couldn't read, past the objects in `damaged`,
/// told already, and `missing`, which the looks agree no pack holds.
fn told_needs(v: &mut Verified, look: &Look, missing: &BTreeSet<Id>, damaged: &HashSet<Id>) {
    v.saves = look.noted.saves.len() as u64;
    v.entries = look.noted.entries;
    v.lines = look.lines;
    v.shelves = look.shelves;
    // Something in a pack Bolt couldn't read may be there all the same.
    let all_read = v.pack_damaged == 0 && v.pack_unreadable == 0;
    let mut told = HashSet::new();
    let mut other_kind = HashSet::new();
    for (id, why) in &look.noted.unread {
        let (kind, message) = match why {
            Unread::Missing => continue,
            Unread::OtherKind => {
                other_kind.insert(*id);
                continue;
            }
            Unread::Damaged(_) if damaged.contains(id) => continue,
            Unread::Damaged(why) => (
                ProblemKind::ObjectDamaged,
                format!("It can't be read: {}.", escape(why, false)),
            ),
            Unread::Unreadable(why) => (
                ProblemKind::PackUnreadable,
                format!("Bolt couldn't read it: {}.", escape(why, false)),
            ),
        };
        if told.insert(*id) {
            v.found(Problem {
                kind,
                pack: None,
                object: Some(*id),
                needed_by: None,
                message,
            });
        }
    }
    for id in missing {
        let message = if other_kind.contains(id) {
            "It is stored, but as another kind of object than what needs it."
        } else if all_read {
            "It isn't in any pack."
        } else {
            "It isn't in any pack Bolt could read."
        };
        v.found(Problem {
            kind: ProblemKind::ObjectMissing,
            pack: None,
            object: Some(*id),
            needed_by: None,
            message: message.to_owned(),
        });
    }
}

/// Names one save, and the path in it, that need each object found damaged or missing that the
/// project needs, walking the saves read once more. Each folder is read once.
fn name_needers(v: &mut Verified, look: &Look, signed: bool) {
    let mut wanted: HashMap<Id, Vec<usize>> = HashMap::new();
    for (i, p) in v.problems.iter().enumerate() {
        let needs = matches!(
            p.kind,
            ProblemKind::ObjectDamaged | ProblemKind::ObjectMissing
        );
        if let Some(object) = p.object
            && needs
            && look.kept.contains(&object)
        {
            // A chunk is needed where its file is.
            let at = look.noted.via.get(&object).copied().unwrap_or(object);
            wanted.entry(at).or_default().push(i);
        }
    }
    let mut seen = HashSet::new();
    for save in &look.noted.saves {
        if wanted.is_empty() {
            return;
        }
        let Ok(read) = look.store.save(save, signed) else {
            continue;
        };
        let mut folders = vec![(read.root, String::new())];
        while let Some((folder, path)) = folders.pop() {
            for i in wanted.remove(&folder).into_iter().flatten() {
                v.problems[i].needed_by = Some((*save, path.clone()));
            }
            if !seen.insert(folder) {
                continue;
            }
            let Ok(listing) = look.store.listing(&folder) else {
                continue;
            };
            for entry in listing.entries() {
                let at = if path.is_empty() {
                    entry.name.clone()
                } else {
                    format!("{path}/{}", entry.name)
                };
                if entry.kind == EntryKind::Folder {
                    folders.push((entry.id, at));
                } else {
                    for i in wanted.remove(&entry.id).into_iter().flatten() {
                        v.problems[i].needed_by = Some((*save, at.clone()));
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;
