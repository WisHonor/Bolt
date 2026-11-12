//! `opalro verify`: reads every object of every pack back, then works out what the project needs,
//! as `tidy` does, and looks for each of those in the packs. It takes no lock and changes nothing.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::ffi::OsString;
use std::io;
use std::path::Path;
use std::sync::Arc;

use opal_platform::dir::Dir;

use crate::diary::{DiaryEntry, ServerLines, TagList};
use crate::encoding::DecodeError;
use crate::id::Id;
use crate::link::ProjectLink;
use crate::listing::{EntryKind, Listing, Measure, SaveLimits};
use crate::pack::{ENTRY_HEADER_LEN, IndexEntry, ObjectKind, PackError, PackReader};
use crate::repo::{RepoError, Repository, head_entry};
use crate::save::Save;
use crate::store::{Store, is_damage};
use crate::text::{escape, thousands};
use crate::tidy::{self, Noted, Source, Unread};
use crate::warning::{self, Warning};
use crate::writeout;

/// What is wrong.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProblemKind {
    PackDamaged,
    PackUnreadable,
    ObjectDamaged,
    ObjectMissing,
    HeadLost,
    SaveTooBig,
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
            ProblemKind::SaveTooBig => "save_too_big",
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
    /// Whether the diary met an entry that is missing or can't be read: it isn't intact.
    pub diary_gap: bool,
    pub lines: u64,
    pub shelves: u64,
    pub problems_total: u64,
    /// How many problems of each kind were found, kept or not.
    pub pack_damaged: u64,
    pub pack_unreadable: u64,
    pub object_damaged: u64,
    pub object_missing: u64,
    pub head_lost: u64,
    pub save_too_big: u64,
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
        self.more(problem.kind, 1);
        if self.problems.len() < PROBLEMS_KEPT {
            self.problems.push(problem);
        }
    }

    /// Counts `n` more problems of `kind`, found but not kept.
    fn more(&mut self, kind: ProblemKind, n: u64) {
        self.problems_total = self.problems_total.saturating_add(n);
        let count = match kind {
            ProblemKind::PackDamaged => &mut self.pack_damaged,
            ProblemKind::PackUnreadable => &mut self.pack_unreadable,
            ProblemKind::ObjectDamaged => &mut self.object_damaged,
            ProblemKind::ObjectMissing => &mut self.object_missing,
            ProblemKind::HeadLost => &mut self.head_lost,
            ProblemKind::SaveTooBig => &mut self.save_too_big,
        };
        *count = count.saturating_add(n);
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
            ProblemKind::SaveTooBig => self.save_too_big,
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
    let told = check_packs(
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
    let (verdict, look) = agreed_needs(repo, window_days, now_ms, &told);
    let (kind, message) = match verdict {
        Verdict::Found(found) => {
            if let Some(look) = look {
                v.needs_checked = true;
                let names = told_needs(&mut v, &look, &found);
                name_needers(&mut v, &look, repo.signatures_allowed(), &names);
                // From the last look only, so told once.
                let ends = look.store.warnings().into_iter();
                v.warnings
                    .extend(ends.filter(|w| w.kind == warning::DIARY_END_UNREADABLE));
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
/// objects it told of.
fn check_packs(
    folder: &Arc<Dir>,
    packs_dir: &Path,
    signed: bool,
    skip_missing_chunks: bool,
    v: &mut Verified,
    progress: &mut dyn FnMut(u64, u64),
) -> Told {
    let mut found = Found::default();
    let (mut seen, mut done, mut total) = (HashSet::new(), 0u64, 0u64);
    for _ in 0..ROUNDS {
        let listed = match tidy::packs(folder) {
            Ok(listed) => listed,
            Err(e) => {
                let message = format!(
                    "Opalro couldn't list the packs in {}: {}.",
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
    Told {
        damaged: found.damaged,
        unreadable: found.unreadable,
    }
}

/// The objects the check of the packs told of, which the check of what is needed doesn't tell
/// again.
#[derive(Default)]
struct Told {
    damaged: HashSet<Id>,
    /// Those that couldn't be read for a reason other than damage.
    unreadable: HashSet<Id>,
}

impl Told {
    fn contains(&self, id: &Id) -> bool {
        self.damaged.contains(id) || self.unreadable.contains(id)
    }
}

/// What checking the packs found that the later steps need.
#[derive(Default)]
struct Found {
    /// Objects found damaged.
    damaged: HashSet<Id>,
    /// Objects that couldn't be read for another reason.
    unreadable: HashSet<Id>,
    /// Content read back whole.
    content: HashSet<Id>,
    /// Each chunk list whose structure checked out, by its pack, to rebuild its file.
    lists: Vec<(OsString, IndexEntry)>,
}

impl Found {
    /// Keeps `id`, told as `kind`.
    fn told(&mut self, kind: ProblemKind, id: Id) {
        let told = if kind == ProblemKind::ObjectDamaged {
            &mut self.damaged
        } else {
            &mut self.unreadable
        };
        told.insert(id);
    }
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
                found.told(kind, entry.id);
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
            found.told(kind, entry.id);
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
            format!("Opalro couldn't read the pack: {}.", why(e)),
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
            format!("Opalro couldn't read it: {}.", why(e)),
        ),
    }
}

/// What a look found wrong with what the project needs: of each kind of finding, the first that
/// can be told, by ID and then name, so that two looks of an unchanged store keep the same; and
/// how many of each there are.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Findings {
    kept: BTreeSet<Finding>,
    /// How many were found: missing, of a wrong size, too big, damaged, unreadable.
    counts: [u64; 5],
    /// Whether the diary met an entry that is missing or can't be read.
    diary_gap: bool,
}

impl Findings {
    fn is_empty(&self) -> bool {
        !self.diary_gap && self.counts.iter().all(|&n| n == 0)
    }

    /// What two looks agree on: what both kept, and of each kind, when both kept all they
    /// found, how many both found; else the smaller of their counts.
    fn with(&self, later: &Findings) -> Findings {
        let kept: BTreeSet<Finding> = &self.kept & &later.kept;
        let mut counts = [0u64; 5];
        for (kind, n) in counts.iter_mut().enumerate() {
            let (a, b) = (self.counts[kind], later.counts[kind]);
            *n = if a.max(b) <= PROBLEMS_KEPT as u64 {
                kept.iter().filter(|f| f.kind() == kind).count() as u64
            } else {
                a.min(b)
            };
        }
        Findings {
            kept,
            counts,
            diary_gap: self.diary_gap && later.diary_gap,
        }
    }

    /// Keeps the first that can be told of `found`, all of one `kind` (0 to 4), and counts
    /// them all.
    fn add(&mut self, kind: usize, found: u64, first: impl IntoIterator<Item = Finding>) {
        self.counts[kind] = found;
        self.kept.extend(first.into_iter().take(PROBLEMS_KEPT));
    }
}

/// What a look at what the project needs concluded, kept once its working sets are dropped.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Verdict {
    /// What it found wrong with what the project needs.
    Found(Findings),
    /// `state/head` names nothing Opalro can read: why.
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

    /// Two verdicts of the same kind as one: what both found, else the later.
    fn with(&self, later: &Verdict) -> Verdict {
        match (self, later) {
            (Verdict::Found(a), Verdict::Found(b)) => Verdict::Found(a.with(b)),
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
    told: &Told,
) -> (Verdict, Option<Box<Look>>) {
    let mut verdicts = Vec::new();
    let mut last: Option<Box<Look>> = None;
    loop {
        // Gone before the next look begins, so two looks never hold their sets at once.
        drop(last.take());
        let (verdict, look) = look(repo, window_days, now_ms, told);
        last = look;
        let clean = matches!(&verdict, Verdict::Found(found) if found.is_empty());
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
/// window, and what of it no pack holds, with the packs listed again. `told` are the objects the
/// check of the packs told of; the files of those found damaged are named too.
fn look(
    repo: &Repository,
    window_days: u32,
    now_ms: i64,
    told: &Told,
) -> (Verdict, Option<Box<Look>>) {
    match look_at(repo, window_days, now_ms, told) {
        Ok(Ok((found, look))) => (Verdict::Found(found), Some(look)),
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

/// Something wrong with what the project needs, as one look found it: two looks agree on it only
/// when both found it the same.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Finding {
    /// Needed, and damaged where the check of the packs didn't tell it.
    Damaged(Id),
    /// Needed, and unreadable for another reason, where the check of the packs didn't tell it.
    Unreadable(Id),
    /// Needed, and in no pack as what needs it.
    Missing(Id),
    /// The listing gives the file `name` another size than what is stored for it.
    Size {
        listing: Id,
        name: String,
        listed: u64,
        stored: u64,
    },
    /// The save passes a limit every reader keeps: which, in the readers' words.
    TooBig { save: Id, why: String },
}

impl Finding {
    /// Which count of [`Findings`] it is in.
    fn kind(&self) -> usize {
        match self {
            Finding::Missing(_) => 0,
            Finding::Size { .. } => 1,
            Finding::TooBig { .. } => 2,
            Finding::Damaged(_) => 3,
            Finding::Unreadable(_) => 4,
        }
    }
}

/// [`look`]: what is wrong and the look's sets, or a lost head, or why it couldn't be made.
#[allow(clippy::type_complexity)]
fn look_at(
    repo: &Repository,
    window_days: u32,
    now_ms: i64,
    told: &Told,
) -> Result<Result<(Findings, Box<Look>), RepoError>, Unmade> {
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
        watch: told.damaged.clone(),
        ..Noted::default()
    };
    let (kept, mut noted) = tidy::needs(&src, &packs, head, since, watch).map_err(packs_failed)?;
    // At most one of each object the store holds.
    let mut missing: BTreeSet<Id> = kept
        .iter()
        .filter(|id| !noted.known.contains_key(id))
        .copied()
        .collect();
    let (mut damaged, mut unreadable) = (BTreeSet::new(), BTreeSet::new());
    for (id, why) in &noted.unread {
        let into = match why {
            Unread::Missing | Unread::OtherKind => &mut missing,
            // Told already, by the check of the packs.
            _ if told.contains(id) => continue,
            Unread::Damaged(_) => &mut damaged,
            Unread::Unreadable(_) => &mut unreadable,
        };
        into.insert(*id);
    }
    let mut found = Findings {
        diary_gap: noted.diary_gap,
        ..Findings::default()
    };
    for (kind, ids, finding) in [
        (0, missing, Finding::Missing as fn(Id) -> Finding),
        (3, damaged, Finding::Damaged),
        (4, unreadable, Finding::Unreadable),
    ] {
        found.add(kind, ids.len() as u64, ids.into_iter().map(finding));
    }
    let sizes = std::mem::take(&mut noted.sizes);
    let sizes = sizes
        .into_iter()
        .map(|(listing, name, listed, stored)| Finding::Size {
            listing,
            name,
            listed,
            stored,
        });
    found.add(1, noted.sizes_found, sizes);
    let big: BTreeSet<Finding> = too_big(&noted).into_iter().collect();
    found.add(2, big.len() as u64, big);
    // Measured; not needed once done.
    noted.shapes = HashMap::new();
    let look = Look {
        store,
        kept,
        noted,
        lines: entry.state.lines.len() as u64,
        shelves: entry.state.shelves.len() as u64,
    };
    Ok(Ok((found, Box::new(look))))
}

/// Each save `noted` read that passes a limit every reader keeps, with which, in the readers'
/// words. Each listing is measured once, however many paths reach it, and a listing that
/// couldn't be read counts as empty.
fn too_big(noted: &Noted) -> Vec<Finding> {
    let mut measured: HashMap<Id, Measure> = HashMap::new();
    let mut found = Vec::new();
    for &(save, root) in &noted.roots {
        // Below every folder first, without a call per level: a save may nest deeper than a stack.
        let mut todo = vec![(root, false)];
        while let Some((id, ready)) = todo.pop() {
            if measured.contains_key(&id) {
                continue;
            }
            let Some(shape) = noted.shapes.get(&id) else {
                measured.insert(id, Measure::default());
                continue;
            };
            if !ready {
                todo.push((id, true));
                let unmeasured = shape
                    .folders
                    .iter()
                    .filter(|(f, ..)| !measured.contains_key(f));
                todo.extend(unmeasured.map(|&(folder, ..)| (folder, false)));
                continue;
            }
            let mut m = Measure {
                entries: shape.entries,
                depth: 0,
                longest: shape.longest,
            };
            // A folder named by several entries counts once for each of them.
            for &(folder, times, name) in &shape.folders {
                let below = measured.get(&folder).copied().unwrap_or_default();
                m.add_folder(below, times, name);
            }
            measured.insert(id, m);
        }
        let m = measured.get(&root).copied().unwrap_or_default();
        let mut limits = SaveLimits::default();
        let entries = usize::try_from(m.entries).unwrap_or(usize::MAX);
        let passed = [
            limits.add(entries),
            limits.check_depth(m.depth),
            limits.check_path(m.longest),
        ];
        let why = passed.into_iter().filter_map(Result::err);
        found.extend(why.map(|why| Finding::TooBig { save, why }));
    }
    found
}

/// Tells what the looks agree on, `found`, in the words of the last, `look`. Gives, for each
/// problem kept that is a listing giving a file the wrong size, the file's name.
fn told_needs(v: &mut Verified, look: &Look, found: &Findings) -> HashMap<usize, String> {
    v.saves = look.noted.saves.len() as u64;
    v.entries = look.noted.entries;
    v.lines = look.lines;
    v.shelves = look.shelves;
    v.diary_gap = found.diary_gap;
    // Something in a pack Opalro couldn't read may be there all the same.
    let all_read = v.pack_damaged == 0 && v.pack_unreadable == 0;
    let (mut other_kind, mut why_damaged, mut why_unreadable) =
        (HashSet::new(), HashMap::new(), HashMap::new());
    for (id, why) in &look.noted.unread {
        match why {
            Unread::Missing => {}
            Unread::OtherKind => {
                other_kind.insert(*id);
            }
            Unread::Damaged(why) => {
                why_damaged.entry(*id).or_insert(why.as_str());
            }
            Unread::Unreadable(why) => {
                why_unreadable.entry(*id).or_insert(why.as_str());
            }
        }
    }
    let mut names = HashMap::new();
    let mut told_of = [0u64; 5];
    for finding in &found.kept {
        told_of[finding.kind()] += 1;
        let (kind, object, message) = match finding {
            Finding::Damaged(id) => {
                let why = escape(why_damaged.get(id).copied().unwrap_or_default(), false);
                let message = format!("It can't be read: {why}.");
                (ProblemKind::ObjectDamaged, *id, message)
            }
            Finding::Unreadable(id) => {
                let why = escape(why_unreadable.get(id).copied().unwrap_or_default(), false);
                let message = format!("Opalro couldn't read it: {why}.");
                (ProblemKind::PackUnreadable, *id, message)
            }
            Finding::Missing(id) => {
                let message = if other_kind.contains(id) {
                    "It is stored, but as another kind of object than what needs it."
                } else if all_read {
                    "It isn't in any pack."
                } else {
                    "It isn't in any pack Opalro could read."
                };
                (ProblemKind::ObjectMissing, *id, message.to_owned())
            }
            Finding::Size {
                listing,
                name,
                listed,
                stored,
            } => {
                let message = format!(
                    "It gives {} as {} bytes, but what is stored for it is {} bytes.",
                    escape(name, false),
                    thousands(*listed),
                    thousands(*stored)
                );
                if v.problems.len() < PROBLEMS_KEPT {
                    names.insert(v.problems.len(), name.clone());
                }
                (ProblemKind::ObjectDamaged, *listing, message)
            }
            Finding::TooBig { save, why } => {
                let message = format!("No reader takes it: {why}.");
                (ProblemKind::SaveTooBig, *save, message)
            }
        };
        v.found(Problem {
            kind,
            pack: None,
            object: Some(object),
            needed_by: None,
            message,
        });
    }
    // Counted, not kept: past what is told.
    let kinds = [
        ProblemKind::ObjectMissing,
        ProblemKind::ObjectDamaged,
        ProblemKind::SaveTooBig,
        ProblemKind::ObjectDamaged,
        ProblemKind::PackUnreadable,
    ];
    for ((kind, &count), told) in kinds.into_iter().zip(&found.counts).zip(told_of) {
        v.more(kind, count.saturating_sub(told));
    }
    names
}

/// Names one save, and the path in it, that need each object found damaged or missing that the
/// project needs, walking the saves read once more. Each folder is read once. A listing that
/// gives a file the wrong size is needed at that file's path, its name in `names`.
fn name_needers(v: &mut Verified, look: &Look, signed: bool, names: &HashMap<usize, String>) {
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
    let joined = |path: &str, name: &str| {
        if path.is_empty() {
            name.to_owned()
        } else {
            format!("{path}/{name}")
        }
    };
    let mut seen = HashSet::new();
    for save in &look.noted.saves {
        if wanted.is_empty() {
            return;
        }
        let Ok(read) = look.store.save(save, signed) else {
            continue;
        };
        // Each folder is walked once, from the first path met; it is pushed again only when
        // wanted itself. Whether to walk it is decided when it is pushed.
        let mut folders = vec![(read.root, String::new(), seen.insert(read.root))];
        while let Some((folder, path, walk)) = folders.pop() {
            for i in wanted.remove(&folder).into_iter().flatten() {
                let at = names
                    .get(&i)
                    .map_or_else(|| path.clone(), |n| joined(&path, n));
                v.problems[i].needed_by = Some((*save, at));
            }
            if !walk {
                continue;
            }
            let Ok(listing) = look.store.listing(&folder) else {
                continue;
            };
            // A path is built only for a folder first met or an object wanted.
            for entry in listing.entries() {
                if entry.kind == EntryKind::Folder {
                    let walk = seen.insert(entry.id);
                    if walk || wanted.contains_key(&entry.id) {
                        folders.push((entry.id, joined(&path, &entry.name), walk));
                    }
                } else if let Some(found) = wanted.remove(&entry.id) {
                    let at = joined(&path, &entry.name);
                    for i in found {
                        v.problems[i].needed_by = Some((*save, at.clone()));
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;
