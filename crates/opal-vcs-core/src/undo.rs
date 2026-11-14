//! `opalro undo` and `opalro redo`: stepping back and forth through the commands the diary records,
//! with what each did to the folder written back or written again; the lists of those steps, and
//! `opalro undo --to`, which goes to any entry, and picks up a lost `head`.

use std::cmp::Reverse;
use std::collections::{BTreeSet, HashSet};
use std::fmt;
use std::path::Path;

use crate::diary::{Action, DiaryEntry, FolderChange, Position, State, TagList};
use crate::drive::Drive;
use crate::fsio::Done;
use crate::history::tip_of;
use crate::id::{Id, is_hex_start};
use crate::journal::Journal;
use crate::kept;
use crate::pack::ObjectKind;
use crate::repo::{ChangeLock, RepoError, Repository, entry_in};
use crate::save::Person;
use crate::shelf::{free_label, within_limit};
use crate::store::{Store, StoreError, is_damage};
use crate::switch::{self, Earlier, Switch, SwitchError};
use crate::timeline;
use crate::warning::{self, Warning};
use crate::writeout::{self, Applied, Plan};

/// What one step taken did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Taken {
    /// The command entry undone or redone, or the entry `undo --to` went to.
    pub step: Id,
    /// That entry.
    pub entry: DiaryEntry,
    /// What the step's own entry records: an undo or redo of `step`, or a command when
    /// `undo --to` went to an entry neither undo nor redo could reach.
    pub action: Action,
    pub before: State,
    pub after: State,
    /// What changed, for messages and lists.
    pub effects: Vec<Effect>,
    /// What the switch wrote and removed; empty when it changed no file.
    pub applied: Applied,
}

/// One thing a step changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    /// A line made (`from` is `None`), moved, or gone (`to` is `None`).
    Line {
        name: String,
        from: Option<Id>,
        to: Option<Id>,
    },
    Position {
        from: Position,
        to: Position,
    },
    ShelfAdded {
        label: String,
        save: Id,
    },
    ShelfGone {
        label: String,
        save: Id,
    },
    Tags,
    ServerLines,
    /// The folder was switched from one save to the other.
    Files {
        from: Id,
        to: Id,
    },
}

/// What changed between two states and through a folder change.
#[must_use]
pub fn effects(before: &State, after: &State, folder: Option<FolderChange>) -> Vec<Effect> {
    let mut found = Vec::new();
    let names: BTreeSet<&String> = before.lines.keys().chain(after.lines.keys()).collect();
    for name in names {
        let (from, to) = (before.lines.get(name), after.lines.get(name));
        if from != to {
            found.push(Effect::Line {
                name: name.clone(),
                from: from.copied(),
                to: to.copied(),
            });
        }
    }
    if before.position != after.position {
        found.push(Effect::Position {
            from: before.position.clone(),
            to: after.position.clone(),
        });
    }
    let holds = |state: &State, save: &Id| state.shelves.iter().any(|(_, s)| s == save);
    for (label, save) in &after.shelves {
        if !holds(before, save) {
            let (label, save) = (label.clone(), *save);
            found.push(Effect::ShelfAdded { label, save });
        }
    }
    for (label, save) in &before.shelves {
        if !holds(after, save) {
            let (label, save) = (label.clone(), *save);
            found.push(Effect::ShelfGone { label, save });
        }
    }
    if before.tags != after.tags {
        found.push(Effect::Tags);
    }
    if before.server_lines != after.server_lines {
        found.push(Effect::ServerLines);
    }
    if let Some(FolderChange { from, to }) = folder {
        found.push(Effect::Files { from, to });
    }
    found
}

/// What `undo`, `redo` or `undo --to` did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UndoOutcome {
    pub taken: Vec<Taken>,
    /// A stopped switch was finished, so no step was taken.
    pub finished: bool,
    /// The shelves `--shelve` made, by label, oldest first: one for each step it was needed for.
    pub shelved: Vec<(String, Id)>,
    /// `head` was lost, so `undo --to` made its entry current by one step that changed no file.
    pub head_lost: bool,
    /// The steps' warnings, as one command's (see [`UndoError::warnings`]).
    pub warnings: Vec<Warning>,
}

/// Why `undo`, `redo` or `undo --to` stopped, with the steps taken, and shelves made, before it
/// did.
#[derive(Debug)]
pub struct UndoError {
    pub taken: Vec<Taken>,
    pub shelved: Vec<(String, Id)>,
    /// The steps' warnings, as one command's: each told once, with one for every entry left
    /// unwritten and one for every link written as a file, whichever step they came from, the one
    /// a file in use stopped included.
    pub warnings: Vec<Warning>,
    pub error: Box<SwitchError>,
}

impl fmt::Display for UndoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.error.fmt(f)
    }
}

impl std::error::Error for UndoError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.error.source()
    }
}

/// The warnings of the steps taken and of the one that `stopped` partway, as one command's.
fn warnings(taken: &[Taken], stopped: Earlier) -> Vec<Warning> {
    let mut not_written: BTreeSet<String> = stopped.not_written.into_iter().collect();
    let mut placeholders: BTreeSet<String> = stopped.placeholders.into_iter().collect();
    let mut told: Vec<Warning> = Vec::new();
    for t in taken {
        not_written.extend(t.applied.not_written.iter().cloned());
        placeholders.extend(t.applied.placeholders.iter().cloned());
        for w in &t.applied.warnings {
            if !switch::LISTED.contains(&w.kind) && !told.contains(w) {
                told.push(w.clone());
            }
        }
    }
    let listed = |paths: BTreeSet<String>| paths.into_iter().collect::<Vec<_>>();
    if !not_written.is_empty() {
        told.push(writeout::not_written_warning(&listed(not_written)));
    }
    if !placeholders.is_empty() {
        told.push(writeout::placeholders_warning(&listed(placeholders)));
    }
    told
}

/// What to undo or redo, and how.
pub struct StepRequest<'a> {
    /// How many steps, one diary entry each.
    pub n: usize,
    /// Set unsaved work in the way aside on a new shelf, as part of the step.
    pub shelve: bool,
    pub who: &'a Person,
    /// The user's own Git ignore list, read as a save reads it.
    pub git_excludes: Option<&'a Path>,
}

/// Undoes the newest `req.n` steps, one at a time: each puts back the state from just before
/// it and writes back what the folder held where it wrote. Running out at the diary's first
/// entry is success. After finishing a stopped switch it takes no step at all.
pub fn undo(
    repo: &mut Repository,
    req: &StepRequest,
    on_wait: &mut dyn FnMut(&str),
) -> Result<UndoOutcome, UndoError> {
    take(repo, req, on_wait, false)
}

/// Redoes the `req.n` steps undone last, as [`undo`] undoes them.
pub fn redo(
    repo: &mut Repository,
    req: &StepRequest,
    on_wait: &mut dyn FnMut(&str),
) -> Result<UndoOutcome, UndoError> {
    take(repo, req, on_wait, true)
}

fn take(
    repo: &mut Repository,
    req: &StepRequest,
    on_wait: &mut dyn FnMut(&str),
    redo: bool,
) -> Result<UndoOutcome, UndoError> {
    stepping(
        repo,
        on_wait,
        command(redo),
        |repo, lock, earlier, taken, shelved| {
            take_steps(repo, lock, req, redo, earlier, taken, shelved)
        },
    )
}

/// Makes the state the one right after the diary entry whose ID starts with `entry`: at least 8
/// hex digits, matching exactly one entry from `head` back. A step redo can bring back is reached
/// by redoing up to it, and one undo can take by undoing the steps after it, as [`redo`] and
/// [`undo`] take them; any other entry by one new step that puts back its lines, position, tags
/// and server lines, keeps the shelves, and switches the folder to its save as `goto` does.
/// With `head` lost, any entry the store holds is made current by one step that changes no file.
/// `req.n` is unused.
pub fn undo_to(
    repo: &mut Repository,
    entry: &str,
    req: &StepRequest,
    on_wait: &mut dyn FnMut(&str),
) -> Result<UndoOutcome, UndoError> {
    let text = entry.to_ascii_lowercase();
    // Before the lock, so that a mistyped entry finishes nothing and changes nothing.
    if !is_hex_start(&text) {
        return Err(UndoError {
            taken: Vec::new(),
            shelved: Vec::new(),
            warnings: Vec::new(),
            error: Box::new(SwitchError::NotAnEntry {
                text: entry.to_owned(),
            }),
        });
    }
    let mut head_lost = false;
    let went = stepping(
        repo,
        on_wait,
        "undo",
        |repo, lock, earlier, taken, shelved| {
            let head = match repo.current() {
                Ok((head, _)) => head,
                Err(RepoError::HeadLost { .. }) => {
                    head_lost = true;
                    taken.push(pick_up(repo, lock, &text, req.who)?);
                    return Ok(());
                }
                Err(e) => return Err(e.into()),
            };
            let (to, way) = {
                let mut read = |id: &Id| repo.entry(id);
                let to = entry_named(&text, head, &mut read)?;
                if to.0 == head {
                    return Ok(());
                }
                // Redo brings back the undone steps up to it, or undo takes the steps after it.
                let undone = timeline::undone_steps(head, &mut read)?;
                let way = match undone.iter().position(|(id, _)| *id == to.0) {
                    Some(i) => Some((true, i + 1)),
                    None => timeline::done_after(head, to.0, &mut read)?.map(|n| (false, n)),
                };
                (to, way)
            };
            let Some((redo, n)) = way else {
                if let Some((step, shelf)) = jump(repo, lock, req, earlier, to)? {
                    taken.push(step);
                    shelved.extend(shelf);
                }
                return Ok(());
            };
            let req = StepRequest { n, ..*req };
            take_steps(repo, lock, &req, redo, earlier, taken, shelved)
        },
    );
    match went {
        Ok(done) => Ok(UndoOutcome { head_lost, ..done }),
        Err(e) => Err(UndoError {
            error: Box::new(aimed(*e.error, &text)),
            ..e
        }),
    }
}

/// Makes the entry the store holds whose ID starts with `text` current, for a lost `head`: one new
/// step after it, holding its state. No file changes, so `status` then shows how the folder
/// differs from that state's save.
fn pick_up(
    repo: &mut Repository,
    lock: &ChangeLock,
    text: &str,
    who: &Person,
) -> Result<Taken, SwitchError> {
    let mut found: Vec<Id> = stored_entries(repo)?
        .into_iter()
        .filter(|id| id.to_hex().starts_with(text))
        .collect();
    found.sort_unstable();
    let to = match found.as_slice() {
        [to] => *to,
        [] => {
            return Err(SwitchError::NoEntry {
                text: text.to_owned(),
                anywhere: true,
            });
        }
        _ => {
            return Err(SwitchError::EntryAmbiguous {
                text: text.to_owned(),
                entries: found,
            });
        }
    };
    let entry = repo.entry(&to)?;
    // Every record its state names must read, so that the pick never strands the project.
    let unusable = |what, detail: String| SwitchError::EntryUnusable {
        entry: to,
        what,
        detail,
    };
    let saves = "one of its saves";
    let tip = tip_of(&entry).map_err(|e| unusable(saves, e.to_string()))?;
    let shelved = entry.state.shelves.iter().map(|(_, save)| save);
    for save in entry.state.lines.values().chain(shelved).chain([&tip]) {
        let read = repo.store().save(save, repo.signatures_allowed());
        read.map_err(|e| unusable(saves, e.to_string()))?;
    }
    // Read here rather than through `tag_list`, so the line gives the store's own words.
    if let Some(id) = entry.state.tags {
        let tag_list = "its tag list";
        let bytes = (repo.store().read_record(&id, ObjectKind::TagList))
            .map_err(|e| unusable(tag_list, e.to_string()))?;
        TagList::decode(&bytes).map_err(|e| unusable(tag_list, e.to_string()))?;
    }
    repo.server_lines(&entry.state)
        .map_err(|e| unusable("its server lines", e.to_string()))?;
    // A switch stopped with `head` lost can't be finished any more; its saves keep what it held.
    // Dropped before the new entry, so a crash between them never leaves it to refuse every command.
    // Told through the store at once, so a pick-up that then fails still says so.
    if !matches!(Journal::read_front(repo), Ok(None)) {
        let done = Journal::remove(repo, lock)?;
        repo.store().note(Warning::new(
            warning::SWITCH_DROPPED,
            "A switch between saves was stopped, and the project's head was lost before it was \
             finished, so no command could finish it: Opalro dropped it, writing no file. The files \
             it had already written show as unsaved changes, and what it would have written over \
             is kept in that switch's own saves.",
        ));
        if let Done::NotFlushed(e) = done {
            return Err(RepoError::from(e).into());
        }
    }
    let made = DiaryEntry {
        parent: Some(to),
        action: Action::Command,
        command: "undo".to_owned(),
        who: who.clone(),
        opal_version: env!("CARGO_PKG_VERSION").into(),
        state: entry.state.clone(),
        folder_change: None,
    };
    let packs = repo.new_packs()?;
    repo.commit(lock, packs, &made)?;
    Ok(Taken {
        step: to,
        action: Action::Command,
        // No state is known from before it, so nothing is told as changed.
        before: entry.state.clone(),
        after: entry.state.clone(),
        effects: Vec::new(),
        applied: Applied::default(),
        entry,
    })
}

/// The ID of every diary entry the store holds, from the packs' indexes alone. A pack Opalro may not
/// read is passed over with a warning naming it, so that the others can still be picked.
fn stored_entries(repo: &Repository) -> Result<HashSet<Id>, RepoError> {
    Ok(repo.store().ids_of(ObjectKind::DiaryEntry)?)
}

/// The diary entries in the store that no other entry has as its parent, newest first by `who`
/// time: where the diary may have stopped, for a lost `head`. One that can't be read is left
/// out, since it couldn't be made current either.
pub fn diary_ends(repo: &Repository) -> Result<Vec<(Id, DiaryEntry)>, RepoError> {
    ends_among(repo.store(), stored_entries(repo)?)
}

/// [`diary_ends`] among the entries `ids`, read from `store`.
pub(crate) fn ends_among(
    store: &Store,
    ids: impl IntoIterator<Item = Id>,
) -> Result<Vec<(Id, DiaryEntry)>, RepoError> {
    let (mut followed, mut found) = (HashSet::new(), Vec::new());
    for id in ids {
        match entry_in(store, &id) {
            Ok(entry) => {
                followed.extend(entry.parent);
                found.push((Reverse(entry.who.time_ms), id));
            }
            Err(RepoError::Damaged { .. }) => {}
            Err(e) => return Err(e),
        }
    }
    found.retain(|(_, id)| !followed.contains(id));
    found.sort_unstable();
    // Read again rather than held: a store can hold any number of entries.
    found
        .into_iter()
        .map(|(_, id)| Ok((id, entry_in(store, &id)?)))
        .collect()
}

/// Runs `steps` under the lock for changes, unless the run finished a stopped switch first, with
/// the steps it takes and the shelves `--shelve` makes collected as it goes.
fn stepping(
    repo: &mut Repository,
    on_wait: &mut dyn FnMut(&str),
    command: &str,
    steps: impl FnOnce(
        &mut Repository,
        &ChangeLock,
        &mut Earlier,
        &mut Vec<Taken>,
        &mut Vec<Shelf>,
    ) -> Result<(), SwitchError>,
) -> Result<UndoOutcome, UndoError> {
    let (mut taken, mut shelved, mut stopped) = (Vec::new(), Vec::new(), Earlier::default());
    let done = switch::locked(repo, command, on_wait, |repo, lock, earlier, finished| {
        // Running the command again after a stop finishes it, and no more.
        if !finished {
            let stepped = steps(repo, lock, earlier, &mut taken, &mut shelved);
            stopped = std::mem::take(earlier);
            stepped?;
        }
        Ok(finished)
    });
    let warnings = warnings(&taken, stopped);
    match done {
        Ok(finished) => Ok(UndoOutcome {
            taken,
            finished,
            shelved,
            head_lost: false,
            warnings,
        }),
        Err(error) => Err(UndoError {
            taken,
            shelved,
            warnings,
            error: Box::new(error),
        }),
    }
}

/// Takes up to `req.n` steps back, or forward for `redo`, one entry each.
fn take_steps(
    repo: &mut Repository,
    lock: &ChangeLock,
    req: &StepRequest,
    redo: bool,
    earlier: &mut Earlier,
    taken: &mut Vec<Taken>,
    shelved: &mut Vec<Shelf>,
) -> Result<(), SwitchError> {
    for _ in 0..req.n {
        let Some((step, shelf)) = take_one(repo, lock, req, redo, earlier)? else {
            break;
        };
        taken.push(step);
        shelved.extend(shelf);
    }
    Ok(())
}

/// `error`, its hint naming `undo --to <text>` where it names a command to run again.
fn aimed(error: SwitchError, text: &str) -> SwitchError {
    match error {
        SwitchError::InTheWayOfStep {
            paths,
            redo,
            shelve,
            ..
        } => SwitchError::InTheWayOfStep {
            paths,
            redo,
            shelve,
            to: Some(text.to_owned()),
        },
        other => other,
    }
}

fn command(redo: bool) -> &'static str {
    if redo { "redo" } else { "undo" }
}

/// A shelf: its label and its save.
type Shelf = (String, Id);

/// An entry on one of undo's lists, and what it changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepInfo {
    pub entry: Id,
    pub diary: DiaryEntry,
    /// What it changed from the entry before it; nothing for the diary's first entry.
    pub effects: Vec<Effect>,
}

/// The steps redo can bring back and undo can take.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UndoList {
    /// The one `redo` brings back first.
    pub redo: Vec<StepInfo>,
    /// Newest first: the one `undo` takes first.
    pub undo: Vec<StepInfo>,
    /// Undo can take more steps than `undo` holds.
    pub more: bool,
}

/// The steps redo can bring back, and the newest `limit` that undo can take.
pub fn undo_list(repo: &Repository, limit: usize) -> Result<UndoList, RepoError> {
    let (head, _) = repo.current()?;
    let steps = timeline::steps(head, &mut |id| repo.entry(id), limit)?;
    Ok(UndoList {
        redo: infos(repo, steps.undone)?,
        undo: infos(repo, steps.done)?,
        more: steps.more_done,
    })
}

/// Where the diary may have stopped, for a lost `head`: [`diary_ends`], each with what it changed
/// from its parent, or nothing when that can't be read.
pub fn stopped_at(repo: &Repository) -> Result<Vec<StepInfo>, RepoError> {
    let mut found = Vec::new();
    for (entry, diary) in diary_ends(repo)? {
        // An unreadable parent is often why this entry is an end at all; one naming a record of
        // another kind is no entry either.
        let before = match diary.parent.map(|p| repo.entry(&p)).transpose() {
            Err(RepoError::Damaged { .. }) => None,
            read => read?,
        };
        let effects = did(before.as_ref(), &diary);
        found.push(StepInfo {
            entry,
            diary,
            effects,
        });
    }
    Ok(found)
}

/// The entries of `list`, each with what it changed from the one before it.
fn infos(repo: &Repository, list: Vec<(Id, DiaryEntry)>) -> Result<Vec<StepInfo>, RepoError> {
    list.into_iter()
        .map(|(entry, diary)| {
            let before = diary.parent.map(|p| repo.entry(&p)).transpose()?;
            let effects = did(before.as_ref(), &diary);
            Ok(StepInfo {
                entry,
                diary,
                effects,
            })
        })
        .collect()
}

/// Calls `each` with every entry from `head` back, newest first, and what it changed.
pub fn whole_diary<E: From<RepoError>>(
    repo: &Repository,
    mut each: impl FnMut(Id, &DiaryEntry, &[Effect]) -> Result<(), E>,
) -> Result<(), E> {
    let (head, _) = repo.current()?;
    back_from(head, &mut |id| repo.entry(id), |id, entry, before| {
        each(id, entry, &did(before, entry))
    })
}

/// Calls `each` with every entry from `head` back, newest first, and the entry before it, read
/// by `read`. An entry's ID covers its parent's, and the store checks every entry against its
/// ID, so the walk can't come round again.
fn back_from<E: From<RepoError>>(
    head: Id,
    read: &mut timeline::Read<'_>,
    mut each: impl FnMut(Id, &DiaryEntry, Option<&DiaryEntry>) -> Result<(), E>,
) -> Result<(), E> {
    let mut at = Some((head, read(&head)?));
    while let Some((id, entry)) = at {
        at = match entry.parent {
            Some(parent) => Some((parent, read(&parent)?)),
            None => None,
        };
        each(id, &entry, at.as_ref().map(|(_, e)| e))?;
    }
    Ok(())
}

/// The entry a step `undo --to` took went to, given that step's own entry, the one command
/// entry that records `undo`: the newest before it in the state it put back, the shelves aside
/// (that entry, or one just like it). `None` for any other step.
pub fn went_to(repo: &Repository, step: &DiaryEntry) -> Result<Option<Id>, RepoError> {
    if step.action != Action::Command || step.command != "undo" {
        return Ok(None);
    }
    let (now, mut at) = (&step.state, step.parent);
    while let Some(id) = at {
        let entry = repo.entry(&id)?;
        let then = &entry.state;
        let same = (&then.lines, &then.position, then.tags, then.server_lines)
            == (&now.lines, &now.position, now.tags, now.server_lines);
        if same {
            return Ok(Some(id));
        }
        at = entry.parent;
    }
    Ok(None)
}

/// What `entry` changed from `before`, the entry before it; nothing for the diary's first entry.
fn did(before: Option<&DiaryEntry>, entry: &DiaryEntry) -> Vec<Effect> {
    before.map_or_else(Vec::new, |b| {
        effects(&b.state, &entry.state, entry.folder_change)
    })
}

/// The one entry from `head` back whose ID starts with `text`, in lowercase hex digits.
fn entry_named(
    text: &str,
    head: Id,
    read: &mut timeline::Read<'_>,
) -> Result<(Id, DiaryEntry), SwitchError> {
    let mut found = Vec::new();
    back_from(head, read, |id, entry, _| {
        if id.to_hex().starts_with(text) {
            found.push((id, entry.clone()));
        }
        Ok::<(), RepoError>(())
    })?;
    if found.len() > 1 {
        let entries = found.into_iter().map(|(id, _)| id).collect();
        return Err(SwitchError::EntryAmbiguous {
            text: text.to_owned(),
            entries,
        });
    }
    found.pop().ok_or_else(|| SwitchError::NoEntry {
        text: text.to_owned(),
        anywhere: false,
    })
}

/// Goes to the state right after `to`, an entry neither undo nor redo can reach, as one new step:
/// its lines, position, tags and server lines with the shelves kept, and the folder switched from
/// the current save to that state's save as `goto` switches it. `None` when things are that way
/// already.
fn jump(
    repo: &mut Repository,
    lock: &ChangeLock,
    req: &StepRequest,
    earlier: &mut Earlier,
    (to, entry): (Id, DiaryEntry),
) -> Result<Option<(Taken, Option<Shelf>)>, SwitchError> {
    let (head, now) = repo.current()?;
    let (current, target) = (tip_of(&now)?, tip_of(&entry)?);
    let mut state = State {
        shelves: now.state.shelves.clone(),
        ..entry.state.clone()
    };
    let gone = |detail| SwitchError::EntryGone { entry: to, detail };
    put_back_readable(repo, &now.state, &state, None, gone)?;
    let drive = repo.drive()?;
    let mut plan = writeout::plan(repo, current, target, None, false, drive, req.git_excludes)?;
    let (mut from, mut shelf) = (current, None);
    if !plan.collisions.is_empty() && req.shelve {
        let shelving = Shelving {
            current,
            from,
            state: &now.state,
            req,
            redo: false,
            drive,
        };
        let shelves = &mut state.shelves;
        let (again, with_it, made) =
            shelving.replan(repo, lock, &plan.collisions, target, shelves)?;
        (plan, from, shelf) = (again, with_it, Some(made));
    }
    if !plan.collisions.is_empty() {
        return Err(SwitchError::InTheWayOfStep {
            paths: plan.collisions,
            redo: false,
            shelve: req.shelve,
            to: None,
        });
    }
    if state == now.state && plan.actions.is_empty() {
        // Nothing to record, but what reading the folder found is told, as `goto` tells it.
        for w in switch::switch_warnings(plan, Applied::default(), earlier).warnings {
            repo.store().note(w);
        }
        return Ok(None);
    }
    let kept = switch::kept_on(&now.state, current, "undo", &[], req.who);
    let mut packs = repo.new_packs()?;
    let (folder_change, _) =
        kept::switch_saves(repo, &plan, from, target, &kept, drive, &mut packs)?;
    let to_make = Switch {
        head,
        action: Action::Command,
        state: state.clone(),
        folder_change,
        plan,
        drive,
        packs,
    };
    let applied = switch::switch(repo, lock, to_make, req.who, "undo", earlier)?;
    let effects = effects(&now.state, &state, folder_change);
    let taken = Taken {
        step: to,
        entry,
        action: Action::Command,
        before: now.state,
        after: state,
        effects,
        applied,
    };
    Ok(Some((taken, shelf)))
}

/// Takes the next step, if there is one, as an entry of its own, with the shelf `--shelve` made
/// for it.
fn take_one(
    repo: &mut Repository,
    lock: &ChangeLock,
    req: &StepRequest,
    redo: bool,
    earlier: &mut Earlier,
) -> Result<Option<(Taken, Option<Shelf>)>, SwitchError> {
    let (head, now) = repo.current()?;
    let next = {
        let mut read = |id: &Id| repo.entry(id);
        if redo {
            timeline::undone_steps(head, &mut read)?.into_iter().next()
        } else {
            timeline::newest_done(head, &mut read)?
        }
    };
    let Some((step, entry)) = next else {
        return Ok(None);
    };
    let parent = match entry.parent {
        Some(id) => Some(repo.entry(&id)?.state),
        None => None,
    };
    // Undo puts back the state before the step, and redo the step's own. The diary's first
    // entry has none before it, and can't be undone.
    let Some(source) = (if redo {
        Some(&entry.state)
    } else {
        parent.as_ref()
    }) else {
        return Ok(None);
    };
    let mut state = State {
        lines: source.lines.clone(),
        tags: source.tags,
        position: source.position.clone(),
        shelves: now.state.shelves.clone(),
        server_lines: source.server_lines,
    };
    let before_step = parent.as_ref().map_or(&[][..], |p| p.shelves.as_slice());
    restore_shelves(&mut state.shelves, before_step, &entry.state.shelves, redo);
    within_limit(state.shelves.len())?;
    let gone = |detail| SwitchError::StepGone { step, detail };
    put_back_readable(repo, &now.state, &state, entry.folder_change, gone)?;

    let action = if redo {
        Action::Redo(step)
    } else {
        Action::Undo(step)
    };
    let command = command(redo);
    let (mut shelf, mut planned, mut folder_change) = (None, None, None);
    if let Some(change) = entry.folder_change {
        // Undo switches from what the step wrote back to what the folder held, redo the other
        // way, and each records that switch: the step's two saves picture the folder before and
        // after it, at exactly its paths.
        let switched = if redo {
            change
        } else {
            FolderChange {
                from: change.to,
                to: change.from,
            }
        };
        let (from, to) = (switched.from, switched.to);
        let drive = repo.drive()?;
        let mut plan = writeout::plan(repo, from, to, None, false, drive, req.git_excludes)?;
        if !plan.collisions.is_empty() && req.shelve {
            let shelving = Shelving {
                current: tip_of(&now)?,
                from,
                state: &now.state,
                req,
                redo,
                drive,
            };
            let shelves = &mut state.shelves;
            let (again, _, made) = shelving.replan(repo, lock, &plan.collisions, to, shelves)?;
            (plan, shelf) = (again, Some(made));
        }
        if !plan.collisions.is_empty() {
            return Err(SwitchError::InTheWayOfStep {
                paths: plan.collisions,
                redo,
                shelve: req.shelve,
                to: None,
            });
        }
        folder_change = Some(switched);
        planned = Some((plan, drive));
    }
    let packs = repo.new_packs()?;
    let applied = match planned {
        Some((plan, drive)) if !plan.actions.is_empty() => {
            let to_make = Switch {
                head,
                action: action.clone(),
                state: state.clone(),
                folder_change,
                plan,
                drive,
                packs,
            };
            switch::switch(repo, lock, to_make, req.who, command, earlier)?
        }
        // A step that changed no file changes none undone or redone, and nor does one whose
        // files are already as it would leave them.
        planned => {
            let made = DiaryEntry {
                parent: Some(head),
                action: action.clone(),
                command: command.to_owned(),
                who: req.who.clone(),
                opal_version: env!("CARGO_PKG_VERSION").into(),
                state: state.clone(),
                folder_change,
            };
            repo.commit(lock, packs, &made)?;
            match planned {
                Some((plan, _)) => switch::switch_warnings(plan, Applied::default(), earlier),
                None => Applied::default(),
            }
        }
    };
    let effects = effects(&now.state, &state, folder_change);
    let taken = Taken {
        step,
        entry,
        action,
        before: now.state,
        after: state,
        effects,
        applied,
    };
    Ok(Some((taken, shelf)))
}

/// What `--shelve` needs to set the unsaved work in a step's way aside.
struct Shelving<'a> {
    /// The save the project is on, which the shelf is made on.
    current: Id,
    /// The save the step's switch is planned from.
    from: Id,
    state: &'a State,
    req: &'a StepRequest<'a>,
    redo: bool,
    drive: Drive,
}

impl Shelving<'_> {
    /// Sets the unsaved work at `collisions` aside on a new shelf, put first in `shelves`, then
    /// plans the step's switch to `to` again, from a save that holds that work too. Gives that
    /// plan, the save it is planned from, and the shelf.
    fn replan(
        &self,
        repo: &mut Repository,
        lock: &ChangeLock,
        collisions: &[String],
        to: Id,
        shelves: &mut Vec<Shelf>,
    ) -> Result<(Plan, Id, Shelf), SwitchError> {
        within_limit(shelves.len() + 1)?;
        let (made, from) = self.set_aside(repo, lock, collisions)?;
        let label = free_label(shelves);
        shelves.insert(0, (label.clone(), made));
        let ignored = self.req.git_excludes;
        let plan = writeout::plan(repo, from, to, None, false, self.drive, ignored)?;
        Ok((plan, from, (label, made)))
    }

    /// Makes the shelf of the unsaved work at `collisions`, and the save to plan the step from
    /// instead, holding that work too so that the step overwrites it. Both are stored at once,
    /// for the planner to read, unless something in the way can't be set aside (a file Opalro
    /// ignores, or an unsaved link that could come back only as a plain file): then nothing is
    /// stored, and the error names what can't be.
    fn set_aside(
        &self,
        repo: &mut Repository,
        lock: &ChangeLock,
        collisions: &[String],
    ) -> Result<(Id, Id), SwitchError> {
        let (current, drive, command) = (self.current, self.drive, command(self.redo));
        // A discard of those paths throws away exactly the unsaved work there.
        let ignored = self.req.git_excludes;
        let mut work = writeout::plan(
            repo,
            current,
            current,
            Some(collisions),
            true,
            drive,
            ignored,
        )?;
        // Links that could come back only as plain files stay where they are.
        let links = kept::links_to_leave(repo, &work.actions, current, drive)?;
        let stays = kept::goes_with(links.iter().map(String::as_str), drive);
        work.actions.retain(|a| !stays(&a.path));
        let who = self.req.who;
        let mut shelf = switch::kept_on(self.state, current, command, &[], who);
        shelf.message = format!("Set aside by {command}");
        let onto = switch::kept_on(self.state, self.from, command, &[], who);
        let mut packs = repo.new_packs()?;
        let aside = kept::set_aside(repo, &work.actions, &shelf, &onto, drive, &mut packs)?;
        // A collision is out of the way when something was set aside at it, below it or above
        // it, and no link left there is; paths compared as the drive compares names.
        let left: Vec<String> = {
            let paths = aside
                .iter()
                .flat_map(|a| a.paths.iter().map(String::as_str));
            let taken_out = kept::goes_with(paths, drive);
            collisions
                .iter()
                .filter(|c| !taken_out(c) || stays(c))
                .cloned()
                .collect()
        };
        match aside {
            Some(aside) if left.is_empty() => {
                repo.store_packs(lock, packs)?;
                Ok((aside.shelf, aside.step_from))
            }
            _ => Err(SwitchError::InTheWayOfStep {
                paths: left,
                redo: self.redo,
                shelve: true,
                to: None,
            }),
        }
    }
}

/// Takes off the shelves a step added and puts back the ones it removed, for an undo, and the
/// other way round for a redo: `parent` and `step` are the shelves before and after the step,
/// compared by save. One put back keeps its label unless another shelf has it.
fn restore_shelves(
    shelves: &mut Vec<(String, Id)>,
    parent: &[(String, Id)],
    step: &[(String, Id)],
    redo: bool,
) {
    let missing = |from: &[(String, Id)], there: &[(String, Id)]| -> Vec<(String, Id)> {
        let gone = |save: &Id| !there.iter().any(|(_, s)| s == save);
        from.iter()
            .filter(|(_, save)| gone(save))
            .cloned()
            .collect()
    };
    let (added, removed) = (missing(step, parent), missing(parent, step));
    let (off, back) = if redo {
        (removed, added)
    } else {
        (added, removed)
    };
    shelves.retain(|(_, save)| !off.iter().any(|(_, s)| s == save));
    // Back as the newest.
    let mut at = 0;
    for (label, save) in back {
        if shelves.iter().any(|(_, s)| *s == save) {
            continue;
        }
        let label = if shelves.iter().any(|(l, _)| *l == label) {
            free_label(shelves)
        } else {
            label
        };
        shelves.insert(at, (label, save));
        at += 1;
    }
}

/// Reads what a step would put back before anything is planned: each save `state` names that
/// `now` doesn't, its tag list and server lines where they differ, and both saves of `change`.
/// One the store lacks, can't decode or holds only damaged copies of is gone (`tidy` removed it,
/// or it was lost), and `gone` says so; any other failure is told as itself.
fn put_back_readable(
    repo: &Repository,
    now: &State,
    state: &State,
    change: Option<FolderChange>,
    gone: impl FnOnce(String) -> SwitchError,
) -> Result<(), SwitchError> {
    let read = || -> Result<(), StoreError> {
        let store = repo.store();
        let had: HashSet<&Id> = saves_named(now).collect();
        let new = saves_named(state).filter(|id| !had.contains(id)).copied();
        for save in new.chain(change.into_iter().flat_map(|c| [c.from, c.to])) {
            store.save(&save, repo.signatures_allowed())?;
        }
        if let Some(id) = state.tags.filter(|_| state.tags != now.tags) {
            store.read_record(&id, ObjectKind::TagList)?;
        }
        if let Some(id) = state
            .server_lines
            .filter(|_| state.server_lines != now.server_lines)
        {
            store.read_record(&id, ObjectKind::ServerLines)?;
        }
        Ok(())
    };
    let Err(e) = read() else {
        return Ok(());
    };
    let lost = match &e {
        StoreError::Missing(_) | StoreError::Decode(_) | StoreError::WrongKind { .. } => true,
        StoreError::Pack(e) => is_damage(e),
        StoreError::SaveTooLarge { .. } => false,
    };
    Err(if lost { gone(e.to_string()) } else { e.into() })
}

/// The saves `state` names: its lines', the one it is at, and its shelves'.
fn saves_named(state: &State) -> impl Iterator<Item = &Id> {
    let at = match &state.position {
        Position::AtSave(id) => Some(id),
        Position::OnLine(_) => None,
    };
    let shelved = state.shelves.iter().map(|(_, id)| id);
    state.lines.values().chain(at).chain(shelved)
}

#[cfg(test)]
mod tests;
