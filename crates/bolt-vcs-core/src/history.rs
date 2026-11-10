//! Line history, short change IDs, and finding a save from what the user typed. A line can be
//! any length and a message 1 MiB, so history is read one save at a time and never held.

use std::collections::HashSet;

use crate::changeids::ChangeIds;
use crate::diary::{DiaryEntry, Position};
use crate::id::{ChangeId, Id, is_hex_start};
use crate::repo::{RepoError, Repository};
use crate::save::Save;
use crate::text::escape;

/// A short change ID has at least this many letters.
const MIN_SHORT: usize = 4;
/// The letters in a whole change ID.
const LETTERS: usize = 2 * size_of::<ChangeId>();

/// What the user typed comes back in the message, escaped. The candidates are IDs Bolt made.
#[derive(Debug, thiserror::Error)]
pub enum ResolveError {
    #[error("No save, line or ID matches \"{}\".", escape(.0, false))]
    NotFound(String),
    #[error(
        "\"{}\" matches more than one save: {}.",
        escape(text, false),
        candidates.join(", ")
    )]
    Ambiguous {
        text: String,
        candidates: Vec<String>,
    },
    #[error(
        "\"{}\" is the name of a line and also the start of another save's ID ({}), so Bolt \
         didn't pick either.",
        escape(text, false),
        saves.join(", ")
    )]
    LineOrSave { text: String, saves: Vec<String> },
    #[error(transparent)]
    Repo(#[from] RepoError),
}

impl ResolveError {
    /// What the user can do next, if there is something to say.
    #[must_use]
    pub fn hint(&self) -> Option<String> {
        match self {
            ResolveError::NotFound(_) => Some("See your saves with: bolt history".to_owned()),
            ResolveError::Ambiguous { .. } => Some("Type more of the ID to pick one.".to_owned()),
            ResolveError::LineOrSave { .. } => Some(
                "To reach the save, type more of its ID; bolt history --json gives the whole \
                 change ID and fingerprint. Until Bolt can rename lines, that line can't be \
                 reached by this name."
                    .to_owned(),
            ),
            ResolveError::Repo(r) => r.hint(),
        }
    }
}

impl From<crate::store::StoreError> for ResolveError {
    fn from(e: crate::store::StoreError) -> Self {
        RepoError::from(e).into()
    }
}

/// The newest save of the current line, or the checked-out save.
pub(crate) fn tip_of(entry: &DiaryEntry) -> Result<Id, RepoError> {
    match &entry.state.position {
        Position::OnLine(name) => {
            entry
                .state
                .lines
                .get(name)
                .copied()
                .ok_or_else(|| RepoError::Damaged {
                    detail: format!("the current line {name} does not exist"),
                })
        }
        Position::AtSave(id) => Ok(*id),
    }
}

/// The current line, or `None` at a save.
fn line_of(entry: &DiaryEntry) -> Option<&str> {
    match &entry.state.position {
        Position::OnLine(name) => Some(name),
        Position::AtSave(_) => None,
    }
}

/// The save the user is on: the current line's newest save, or the checked-out save.
pub fn current_save(repo: &Repository) -> Result<Id, RepoError> {
    tip_of(&repo.current()?.1)
}

/// How many leading letters two change IDs share.
fn common_letters(a: &ChangeId, b: &ChangeId) -> usize {
    let bytes = a.0.iter().zip(&b.0).take_while(|(x, y)| x == y).count();
    let next_letter = a.0.get(bytes).zip(b.0.get(bytes));
    2 * bytes + usize::from(next_letter.is_some_and(|(x, y)| x >> 4 == y >> 4))
}

/// The letters of a change ID that tell it from every other, when the closest one shares `common`.
fn short_len(common: usize) -> usize {
    (common + 1).clamp(MIN_SHORT, LETTERS)
}

/// For each change ID, how many of its letters tell it from all the others.
fn short_lengths(ids: &[ChangeId]) -> Vec<usize> {
    let mut order: Vec<usize> = (0..ids.len()).collect();
    order.sort_by_key(|&i| ids[i]);
    // Whoever shares the longest start with an ID is next to it in the sorted order.
    let mut common = vec![0; ids.len()];
    for (&a, &b) in order.iter().zip(order.iter().skip(1)) {
        let shared = common_letters(&ids[a], &ids[b]);
        common[a] = common[a].max(shared);
        common[b] = common[b].max(shared);
    }
    common.into_iter().map(short_len).collect()
}

/// The shortest form of `change_id` among the sorted `ids`, leaving one copy of it out when
/// `itself` (it is the change ID of one of their saves).
fn short_among(ids: &[ChangeId], change_id: &ChangeId, itself: bool) -> String {
    let at = ids.partition_point(|c| c < change_id);
    let (below, mut above) = ids.split_at(at);
    if itself && above.first() == Some(change_id) {
        above = &above[1..];
    }
    // The IDs sharing the longest start with it are its neighbours in the sorted order.
    let common = [below.last(), above.first()]
        .into_iter()
        .flatten()
        .map(|other| common_letters(change_id, other))
        .max()
        .unwrap_or(0);
    change_id.to_letters()[..short_len(common)].to_owned()
}

/// The shortest form of `change_id`, the change ID of the save `id`, that no other of `saves`
/// starts with.
pub(crate) fn short_in(saves: &[(Id, ChangeId)], id: &Id, change_id: &ChangeId) -> String {
    let common = saves
        .iter()
        .filter(|(other, _)| other != id)
        .map(|(_, other)| common_letters(change_id, other))
        .max()
        .unwrap_or(0);
    change_id.to_letters()[..short_len(common)].to_owned()
}

/// The shortest form of `change_id` (at least 4 letters) that no other save on the current line
/// starts with. `id` is the save it belongs to, on the line or not.
pub fn short_change_id(
    repo: &Repository,
    id: &Id,
    change_id: &ChangeId,
) -> Result<String, RepoError> {
    let (_, entry) = repo.current()?;
    let mut cache = ChangeIds::load(repo);
    let short = short_in(
        cache.chain(repo, line_of(&entry), tip_of(&entry)?)?,
        id,
        change_id,
    );
    cache.store(repo, &entry.state);
    Ok(short)
}

/// The current line's saves and their change IDs, read once and kept as IDs alone, to shorten
/// any number of change IDs as [`short_change_id`] shortens one.
pub struct LineIds {
    /// Sorted.
    change_ids: Vec<ChangeId>,
    saves: HashSet<Id>,
}

impl LineIds {
    /// Reads the current line, or the saves back from the one the project is at.
    pub fn read(repo: &Repository) -> Result<LineIds, RepoError> {
        let (_, entry) = repo.current()?;
        let mut cache = ChangeIds::load(repo);
        let line = cache.chain(repo, line_of(&entry), tip_of(&entry)?)?;
        let mut change_ids: Vec<ChangeId> = line.iter().map(|(_, c)| *c).collect();
        let saves = line.iter().map(|(id, _)| *id).collect();
        cache.store(repo, &entry.state);
        change_ids.sort_unstable();
        Ok(LineIds { change_ids, saves })
    }

    /// The shortest form of `change_id`, the change ID of the save `id`, that no other save on
    /// the line starts with.
    #[must_use]
    pub fn short(&self, id: &Id, change_id: &ChangeId) -> String {
        short_among(&self.change_ids, change_id, self.saves.contains(id))
    }

    /// Whether another save on the line has `change_id`, the change ID of the save `id`, so that
    /// no form of it tells them apart: an amended save does, once the amend is undone.
    #[must_use]
    pub fn shared(&self, id: &Id, change_id: &ChangeId) -> bool {
        let at = self.change_ids.partition_point(|c| c < change_id);
        let copies = self.change_ids[at..]
            .iter()
            .take_while(|c| *c == change_id)
            .count();
        copies > usize::from(self.saves.contains(id))
    }
}

/// Whether the first `count` saves `saves` lists are what the list says, read from the store:
/// each with the list's change ID, and as its first parent the next save listed, or none after
/// the last. `each` is handed each save that passes. An error for a save the store can't give.
fn links_hold(
    repo: &Repository,
    saves: &[(Id, ChangeId)],
    count: usize,
    mut each: impl FnMut(Save),
) -> Result<bool, crate::store::StoreError> {
    for (at, (id, change_id)) in saves.iter().enumerate().take(count) {
        let save = repo.store().save(id, repo.signatures_allowed())?;
        let next = saves.get(at + 1).map(|(next, _)| next);
        if save.change_id != *change_id || save.parents.first() != next {
            return Ok(false);
        }
        each(save);
    }
    Ok(true)
}

/// Saves from `start` back along first parents, newest first, each read when asked for. A cycle
/// can't be built (a save's fingerprint covers its parents), but a store missing a parent fails
/// with an error, which ends the walk.
fn chain(repo: &Repository, start: Id) -> impl Iterator<Item = Result<(Id, Save), RepoError>> + '_ {
    let mut next = Some(start);
    std::iter::from_fn(move || {
        let id = next.take()?;
        Some(match repo.store().save(&id, repo.signatures_allowed()) {
            Ok(save) => {
                next = save.parents.first().copied();
                Ok((id, save))
            }
            Err(e) => Err(e.into()),
        })
    })
}

/// How many letters of each change ID of `saves` tell it from the others, in their order.
fn lengths_of(saves: &[(Id, ChangeId)]) -> Vec<usize> {
    short_lengths(&saves.iter().map(|(_, c)| *c).collect::<Vec<_>>())
}

/// Calls `each` with every save on the current line, newest first, and the shortest form of its
/// change ID. The saves come from their own first parents, read one at a time, so that only the
/// save being handed out is held; the short forms come from the cache's list of the line, or from
/// the line itself when that list is wrong.
pub fn for_each_on_line<E: From<RepoError>>(
    repo: &Repository,
    mut each: impl FnMut(&Id, &Save, &str) -> Result<(), E>,
) -> Result<(), E> {
    let (_, entry) = repo.current()?;
    let (line, tip) = (line_of(&entry), tip_of(&entry)?);
    let mut cache = ChangeIds::load(repo);
    cache.chain(repo, line, tip)?;
    // Kept before the saves are handed out, however that ends; asked again, it reads nothing.
    cache.store(repo, &entry.state);
    let listed = cache.chain(repo, line, tip)?;
    let lengths = lengths_of(listed);
    let short = |lengths: &[usize], at: usize, save: &Save| {
        let len = lengths.get(at).copied().unwrap_or(LETTERS);
        save.change_id.to_letters()[..len].to_owned()
    };
    let mut walk = chain(repo, tip).enumerate();
    let mut wrong = None;
    let mut count = 0;
    for (at, step) in walk.by_ref() {
        let (id, save) = step?;
        count = at + 1;
        if listed.get(at) != Some(&(id, save.change_id)) {
            wrong = Some((at, id, save));
            break;
        }
        each(&id, &save, &short(&lengths, at, &save))?;
    }
    if wrong.is_none() && count == listed.len() {
        return Ok(());
    }
    // The list isn't the line: it is dropped, the line read from the store, and the listing goes
    // on with the line's own short forms.
    let mut read = ChangeIds::default();
    let lengths = lengths_of(read.chain(repo, line, tip)?);
    read.store(repo, &entry.state);
    if let Some((at, id, save)) = wrong {
        each(&id, &save, &short(&lengths, at, &save))?;
        for (at, step) in walk {
            let (id, save) = step?;
            each(&id, &save, &short(&lengths, at, &save))?;
        }
    }
    Ok(())
}

fn one_of(text: &str, mut found: Vec<(Id, String)>) -> Result<Id, ResolveError> {
    found.sort_by(|a, b| a.1.cmp(&b.1));
    found.dedup();
    match found.as_slice() {
        [] => Err(ResolveError::NotFound(text.to_owned())),
        [(id, _)] => Ok(*id),
        _ => Err(ResolveError::Ambiguous {
            text: text.to_owned(),
            candidates: found.into_iter().map(|(_, c)| c).collect(),
        }),
    }
}

/// The saves of `saves` whose change ID starts with `text`, each with its change ID.
fn starting_with(saves: &[(Id, ChangeId)], text: &str) -> Vec<(Id, String)> {
    saves
        .iter()
        .filter_map(|(id, change_id)| {
            let letters = change_id.to_letters();
            letters.starts_with(text).then_some((*id, letters))
        })
        .collect()
}

/// The saves on the current line whose change ID starts with `text`, each with its change ID.
fn by_change_prefix(
    repo: &Repository,
    entry: &DiaryEntry,
    text: &str,
) -> Result<Vec<(Id, String)>, ResolveError> {
    let (line, tip) = (line_of(entry), tip_of(entry)?);
    let mut cache = ChangeIds::load(repo);
    let saves = cache.chain(repo, line, tip)?;
    let listed = starting_with(saves, text);
    // The one save the list names is shown to be on the line, link by link from its newest save;
    // no save or several, from the store. That the list hides no second save of the line with
    // this start is trusted, as the short IDs Bolt prints trust it: showing it reads the line.
    let checked = match listed.as_slice() {
        [(id, _)] => Some(
            saves
                .iter()
                .position(|(s, _)| s == id)
                .is_some_and(|at| matches!(links_hold(repo, saves, at + 1, |_| {}), Ok(true))),
        ),
        _ => None,
    };
    if checked == Some(true) {
        cache.store(repo, &entry.state);
        return Ok(listed);
    }
    let mut read = ChangeIds::default();
    let found = starting_with(read.chain(repo, line, tip)?, text);
    // A list the check found wrong is dropped even where it gave the right answer.
    if checked == Some(false) || found != listed {
        cache = read;
    }
    cache.store(repo, &entry.state);
    Ok(found)
}

/// Calls `visit` once for each save reachable from `tips` along first parents. A chain an
/// earlier tip walked is not walked again, so the cost doesn't grow with the number of tips.
fn each_once<E>(
    tips: impl IntoIterator<Item = Id>,
    mut first_parent: impl FnMut(&Id) -> Result<Option<Id>, E>,
    mut visit: impl FnMut(&Id),
) -> Result<(), E> {
    let mut seen = HashSet::new();
    for tip in tips {
        let mut next = Some(tip);
        while let Some(id) = next {
            if !seen.insert(id) {
                break;
            }
            visit(&id);
            next = first_parent(&id)?;
        }
    }
    Ok(())
}

/// The saves, among those any line reaches by first parents, whose fingerprint starts with
/// `text`, each with its fingerprint.
fn by_fingerprint_prefix(
    repo: &Repository,
    entry: &DiaryEntry,
    text: &str,
) -> Result<Vec<(Id, String)>, ResolveError> {
    let text = text.to_ascii_lowercase();
    if let Some(id) = Id::from_hex(&text) {
        // A line the cache knows at its newest save may hold it; the current one first. It is
        // shown to be on the line, link by link from its newest save, before it is believed.
        let mut cache = ChangeIds::load(repo);
        if let Some(line) = line_of(entry) {
            cache.chain(repo, Some(line), tip_of(entry)?)?;
        }
        let checked = cache.reaches(&entry.state, &id).map(|(saves, at)| {
            matches!(
                links_hold(repo, saves, at.saturating_add(1), |_| {}),
                Ok(true)
            )
        });
        if checked == Some(false) {
            // The lists are dropped; on a line, its own, read from the store as a handle with
            // no cache reads it, is kept. At a save nothing more is read.
            cache = ChangeIds::dropped();
            if let Some(line) = line_of(entry) {
                cache.chain(repo, Some(line), tip_of(entry)?)?;
            }
        }
        cache.store(repo, &entry.state);
        if checked == Some(true) {
            return Ok(vec![(id, text)]);
        }
    }
    let mut found = Vec::new();
    each_once(
        entry.state.lines.values().copied(),
        |id| {
            let save = repo.store().save(id, repo.signatures_allowed())?;
            Ok::<_, RepoError>(save.parents.first().copied())
        },
        |id| {
            let hex = id.to_hex();
            if hex.starts_with(&text) {
                found.push((*id, hex));
            }
        },
    )?;
    Ok(found)
}

/// Finds a save from text: a line name, else a change-ID prefix (letters `k`-`z`, unique on
/// the current line), else at least 8 hex digits of a fingerprint. Fails with `NotFound`,
/// `Ambiguous`, `LineOrSave`, or a repository problem.
pub fn resolve(repo: &Repository, text: &str) -> Result<Id, ResolveError> {
    let (_, entry) = repo.current()?;
    if let Some(id) = line_named(repo, &entry, text)? {
        return Ok(id);
    }
    one_of(text, by_id(repo, &entry, text)?)
}

/// The save of the line named `text`, if there is one. A name that is also the start of
/// another save's change ID or fingerprint is refused, so that neither passes for the other.
pub(crate) fn line_named(
    repo: &Repository,
    entry: &DiaryEntry,
    text: &str,
) -> Result<Option<Id>, ResolveError> {
    let Some(&line) = entry.state.lines.get(text) else {
        return Ok(None);
    };
    let mut saves: Vec<String> = by_id(repo, entry, text)?
        .into_iter()
        .filter(|(id, _)| *id != line)
        .map(|(_, shown)| shown)
        .collect();
    if saves.is_empty() {
        return Ok(Some(line));
    }
    saves.sort_unstable();
    saves.dedup();
    Err(ResolveError::LineOrSave {
        text: text.to_owned(),
        saves,
    })
}

/// The saves `text` names as the start of a change ID or of a fingerprint.
fn by_id(
    repo: &Repository,
    entry: &DiaryEntry,
    text: &str,
) -> Result<Vec<(Id, String)>, ResolveError> {
    if !text.is_empty() && text.bytes().all(|b| (b'k'..=b'z').contains(&b)) {
        return by_change_prefix(repo, entry, text);
    }
    if is_hex_start(text) {
        return by_fingerprint_prefix(repo, entry, text);
    }
    Ok(Vec::new())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;
    use crate::diary::Action;
    use crate::pack::ObjectKind;
    use crate::save::{EXTRA_LINE, Person};
    use proptest::prelude::*;
    use std::collections::BTreeMap;

    fn who() -> Person {
        Person {
            name: "Ada".into(),
            email: "a@example.com".into(),
            time_ms: 0,
            tz_offset_min: 0,
        }
    }

    /// 32 letters starting with `start`.
    fn letters(start: &str) -> String {
        format!("{start}{}", "k".repeat(LETTERS - start.len()))
    }

    fn change(start: &str) -> ChangeId {
        ChangeId::from_letters(&letters(start)).unwrap()
    }

    /// Commits a save with the given parents and change ID, and points `line` at it if given.
    fn plant(repo: &mut Repository, parents: &[Id], start: &str, line: Option<&str>) -> Id {
        let lock = repo.lock_for_change("save", &mut |_| {}).unwrap();
        let (head, entry) = repo.current().unwrap();
        let root = repo
            .store()
            .save(&entry.state.lines["main"], false)
            .unwrap()
            .root;
        let save = Save {
            root,
            parents: parents.to_vec(),
            author: who(),
            saver: who(),
            message: "m".into(),
            change_id: change(start),
            move_notes: vec![],
            signature: vec![],
            extra: BTreeMap::from([(EXTRA_LINE.to_owned(), b"main".to_vec())]),
        };
        let mut packs = repo.new_packs().unwrap();
        let id = packs.add_record(ObjectKind::Save, &save.encode()).unwrap();
        let mut state = entry.state.clone();
        if let Some(line) = line {
            state.lines.insert(line.to_owned(), id);
        }
        let diary = DiaryEntry {
            parent: Some(head),
            action: Action::Command,
            command: "save".into(),
            who: who(),
            bolt_version: "0.1.0".into(),
            state,
            folder_change: None,
        };
        repo.commit(&lock, packs, &diary).unwrap();
        id
    }

    /// `short_change_id`, by reading the line from `tip`.
    fn walk_short(repo: &Repository, tip: Id, id: &Id, change_id: &ChangeId) -> String {
        let saves: Vec<(Id, ChangeId)> = chain(repo, tip)
            .map(|s| s.map(|(id, save)| (id, save.change_id)).unwrap())
            .collect();
        short_in(&saves, id, change_id)
    }

    /// The user's own cache key, as the command line hands it over.
    const KEY: [u8; crate::keyed::KEY_LEN] = [7; crate::keyed::KEY_LEN];

    /// A project whose line `main` ends with a save for each start given, the first one oldest.
    /// Also returns the saves' IDs, oldest first, and the first save's.
    fn project_with(starts: &[&str]) -> (tempfile::TempDir, Repository, Vec<Id>) {
        let dir = tempfile::tempdir().unwrap();
        let mut repo = Repository::init(dir.path(), &who()).unwrap();
        repo.set_cache_key(KEY);
        let mut tip = repo.current().unwrap().1.state.lines["main"];
        let mut ids = vec![tip];
        for start in starts {
            tip = plant(&mut repo, &[tip], start, Some("main"));
            ids.push(tip);
        }
        (dir, repo, ids)
    }

    /// Each ID's shortest unique form by the plain definition: try longer starts until no other
    /// ID has it, and settle for all the letters if another ID is just the same.
    fn plainly(ids: &[ChangeId]) -> Vec<usize> {
        let all: Vec<String> = ids.iter().map(ChangeId::to_letters).collect();
        (0..ids.len())
            .map(|k| {
                (MIN_SHORT..=LETTERS)
                    .find(|&len| {
                        (0..ids.len()).all(|j| j == k || !all[j].starts_with(&all[k][..len]))
                    })
                    .unwrap_or(LETTERS)
            })
            .collect()
    }

    #[test]
    fn short_ids_grow_to_stay_unique() {
        let ids = [
            change("klmnop"),
            change("klmnopq"),
            change("zzzz"),
            change("zzzz"),
        ];
        assert_eq!(short_lengths(&ids), [7, 7, LETTERS, LETTERS]);
        assert_eq!(short_lengths(&ids[..3]), [7, 7, MIN_SHORT]);
        assert_eq!(short_lengths(&ids[..1]), [MIN_SHORT]);
        assert!(short_lengths(&[]).is_empty());
    }

    proptest! {
        // The first three bytes vary over a few values and the rest are alike, so that start
        // letters are shared at every length, and some IDs are the same.
        #[test]
        fn short_ids_are_the_shortest_that_tell_a_save_from_the_rest(
            heads in prop::collection::vec(
                prop::array::uniform3(prop::sample::select(vec![0x00u8, 0x01, 0x10, 0x11])),
                0..40,
            ),
        ) {
            let ids: Vec<ChangeId> = heads
                .iter()
                .map(|h| {
                    let mut bytes = [0; 16];
                    bytes[..3].copy_from_slice(h);
                    ChangeId(bytes)
                })
                .collect();
            let want = plainly(&ids);
            prop_assert_eq!(short_lengths(&ids), want.clone());
            // Taking the closest of the others one by one gives the same lengths.
            let one_by_one: Vec<usize> = (0..ids.len())
                .map(|k| {
                    let closest = (0..ids.len())
                        .filter(|&j| j != k)
                        .map(|j| common_letters(&ids[k], &ids[j]))
                        .max()
                        .unwrap_or(0);
                    short_len(closest)
                })
                .collect();
            prop_assert_eq!(one_by_one, want);
        }
    }

    #[test]
    fn what_the_user_typed_is_shown_escaped() {
        use crate::test_support::{HOSTILE, assert_hostile_text_escaped};
        let typed = ResolveError::NotFound(HOSTILE.into());
        assert_hostile_text_escaped(&typed.to_string());
        let ambiguous = ResolveError::Ambiguous {
            text: HOSTILE.into(),
            candidates: vec!["klmn".into(), "klmo".into()],
        };
        assert_hostile_text_escaped(&ambiguous.to_string());
    }

    #[test]
    fn a_change_prefix_names_one_save_or_lists_the_candidates() {
        let (_dir, repo, ids) = project_with(&["klmnopk", "klmnopl", "mmmmmm"]);
        // Long prefixes, so the random first save is not likely to share them.
        assert_eq!(resolve(&repo, "klmnopk").unwrap(), ids[1]);
        assert_eq!(resolve(&repo, "klmnopl").unwrap(), ids[2]);
        assert_eq!(resolve(&repo, "mmmmmm").unwrap(), ids[3]);
        match resolve(&repo, "klmnop") {
            Err(ResolveError::Ambiguous { candidates, .. }) => {
                assert_eq!(candidates, [letters("klmnopk"), letters("klmnopl")]);
            }
            other => panic!("{other:?}"),
        }
        assert!(matches!(
            resolve(&repo, "zzzzzz"),
            Err(ResolveError::NotFound(_))
        ));
    }

    /// Gives the save `id` the line `name` too.
    fn name_line(repo: &mut Repository, name: &str, id: Id) {
        let lock = repo.lock_for_change("line", &mut |_| {}).unwrap();
        let (head, entry) = repo.current().unwrap();
        let mut state = entry.state.clone();
        state.lines.insert(name.to_owned(), id);
        let diary = DiaryEntry {
            parent: Some(head),
            action: Action::Command,
            command: "line".into(),
            who: who(),
            bolt_version: "0.1.0".into(),
            state,
            folder_change: None,
        };
        let packs = repo.new_packs().unwrap();
        repo.commit(&lock, packs, &diary).unwrap();
    }

    #[test]
    fn a_line_named_like_another_saves_id_is_refused_and_one_named_like_its_own_is_not() {
        let (_dir, mut repo, ids) = project_with(&["mnopqr", "nopqrs"]);
        let elsewhere = plant(&mut repo, &[ids[0]], "kkkkkk", Some("other"));
        let refused = |repo: &Repository, text: &str| match resolve(repo, text) {
            Err(ResolveError::LineOrSave { saves, .. }) => saves,
            other => panic!("{other:?}"),
        };
        // `mnop` starts the change ID of a save on main, and names a line elsewhere.
        name_line(&mut repo, "mnop", elsewhere);
        assert_eq!(refused(&repo, "mnop"), [letters("mnopqr")]);
        let target = crate::lines::resolve_target(&repo, "mnop");
        assert!(matches!(target, Err(ResolveError::LineOrSave { .. })));
        // A line named like the start of another save's fingerprint is refused the same way.
        let hex = elsewhere.to_hex()[..8].to_owned();
        name_line(&mut repo, &hex, ids[1]);
        assert_eq!(refused(&repo, &hex), [elsewhere.to_hex()]);
        // A line named like the start of its own save's IDs is just the line.
        name_line(&mut repo, "nopq", ids[2]);
        assert_eq!(resolve(&repo, "nopq").unwrap(), ids[2]);
        let own = ids[2].to_hex()[..8].to_owned();
        name_line(&mut repo, &own, ids[2]);
        assert_eq!(
            crate::lines::resolve_target(&repo, &own).unwrap(),
            crate::lines::Target::Line(own.clone())
        );
    }

    #[test]
    fn a_line_or_save_refusal_shows_the_name_escaped() {
        use crate::test_support::{HOSTILE, assert_hostile_text_escaped};
        let refused = ResolveError::LineOrSave {
            text: HOSTILE.into(),
            saves: vec!["klmn".into()],
        };
        assert_hostile_text_escaped(&refused.to_string());
    }

    #[test]
    fn a_fingerprint_prefix_finds_saves_on_any_line_but_not_through_a_second_parent() {
        let (_dir, mut repo, ids) = project_with(&["kkkk"]);
        let (first, tip) = (ids[0], ids[1]);
        let side = plant(&mut repo, &[first], "lmno", Some("side"));
        // `lone` is on no line; the only way to it is the merge's second parent.
        let lone = plant(&mut repo, &[first], "mnop", None);
        let merge = plant(&mut repo, &[tip, lone], "nopq", Some("main"));
        let prefix = |id: &Id| id.to_hex()[..8].to_uppercase();
        for found in [first, tip, side, merge] {
            assert_eq!(resolve(&repo, &prefix(&found)).unwrap(), found);
        }
        assert!(matches!(
            resolve(&repo, &prefix(&lone)),
            Err(ResolveError::NotFound(_))
        ));
        assert!(matches!(
            resolve(&repo, "0123456"),
            Err(ResolveError::NotFound(_))
        ));
    }

    #[test]
    fn ambiguous_candidates_are_sorted_and_listed_once() {
        let a = (Id([0x12; 32]), "1234".to_owned());
        let b = (Id([0x11; 32]), "1111".to_owned());
        match one_of("1", vec![a.clone(), b.clone(), a.clone()]) {
            Err(ResolveError::Ambiguous { candidates, .. }) => {
                assert_eq!(candidates, ["1111", "1234"]);
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(one_of("1", vec![a.clone(), a.clone()]).unwrap(), a.0);
    }

    /// An ID that records its position in its first four bytes.
    fn numbered(n: u32) -> Id {
        let mut bytes = [0; 32];
        bytes[..4].copy_from_slice(&n.to_be_bytes());
        Id(bytes)
    }

    fn number(id: &Id) -> u32 {
        u32::from_be_bytes(id.0[..4].try_into().unwrap())
    }

    #[test]
    fn a_chain_many_tips_lead_to_is_walked_once() {
        // 300 saves in a chain, each the child of the one before, and 100,000 tips at the newest.
        let mut reads = 0;
        let mut visited = Vec::new();
        each_once(
            (0..100_000).map(|_| numbered(299)),
            |id| {
                reads += 1;
                Ok::<_, ()>(number(id).checked_sub(1).map(numbered))
            },
            |id| visited.push(number(id)),
        )
        .unwrap();
        assert_eq!(reads, 300);
        assert_eq!(visited.len(), 300);
    }

    #[test]
    fn each_save_is_visited_once_however_the_tips_overlap() {
        // Saves 0 to 49 in a chain, and 50 to 89 in another that joins it at 9.
        let parent = |n: u32| match n {
            0 => None,
            50 => Some(9),
            _ => Some(n - 1),
        };
        let tips = [49, 20, 89, 49, 9, 60, 0].map(numbered);
        let (mut reads, mut visited) = (0, Vec::new());
        each_once(
            tips,
            |id| {
                reads += 1;
                Ok::<_, ()>(parent(number(id)).map(numbered))
            },
            |id| visited.push(number(id)),
        )
        .unwrap();
        assert_eq!(reads, 90);
        visited.sort_unstable();
        assert_eq!(visited, (0..=89).collect::<Vec<u32>>());
    }

    #[test]
    fn a_walk_that_fails_stops_with_the_error() {
        let result = each_once(
            [numbered(5)],
            |id| {
                if number(id) == 3 {
                    Err("a save is missing")
                } else {
                    Ok(Some(numbered(number(id) - 1)))
                }
            },
            |_| {},
        );
        assert_eq!(result, Err("a save is missing"));
    }

    #[test]
    fn the_line_is_handed_out_newest_first_with_short_ids() {
        // Prefixes shared at several lengths, and one change ID used twice.
        let starts = [
            "klmnopk", "klmnopl", "klmnoqk", "zzzz", "klmnopk", "mmmmmmmm", "mmmmmmmn",
        ];
        let (_dir, repo, ids) = project_with(&starts);
        let mut shown = Vec::new();
        for_each_on_line::<RepoError>(&repo, |id, save, short| {
            shown.push((*id, save.change_id, short.to_owned()));
            Ok(())
        })
        .unwrap();
        let newest_first: Vec<Id> = ids.iter().rev().copied().collect();
        assert_eq!(
            shown.iter().map(|(id, ..)| *id).collect::<Vec<_>>(),
            newest_first
        );
        let change_ids: Vec<ChangeId> = shown.iter().map(|(_, c, _)| *c).collect();
        for ((id, change_id, short), len) in shown.iter().zip(plainly(&change_ids)) {
            assert_eq!(short, &change_id.to_letters()[..len]);
            // One save at a time gives the same answer.
            assert_eq!(&short_change_id(&repo, id, change_id).unwrap(), short);
        }
        // The change ID that two saves have shows whole for both.
        let twice = change("klmnopk").to_letters();
        assert_eq!(shown.iter().filter(|(_, _, s)| *s == twice).count(), 2);
    }

    #[test]
    fn a_save_off_the_line_is_told_from_the_line() {
        let (_dir, mut repo, _) = project_with(&["klmnopk"]);
        let off = plant(&mut repo, &[], "klmnopl", None);
        let short = short_change_id(&repo, &off, &change("klmnopl")).unwrap();
        assert_eq!(short, "klmnopl");
    }

    /// Points `line` at `id`, as an undo would, without a new save.
    fn move_line(repo: &mut Repository, line: &str, id: Id) {
        let lock = repo.lock_for_change("undo", &mut |_| {}).unwrap();
        let (head, entry) = repo.current().unwrap();
        let mut state = entry.state.clone();
        state.lines.insert(line.to_owned(), id);
        let diary = DiaryEntry {
            parent: Some(head),
            action: Action::Command,
            command: "undo".into(),
            who: who(),
            bolt_version: "0.1.0".into(),
            state,
            folder_change: None,
        };
        let packs = repo.new_packs().unwrap();
        repo.commit(&lock, packs, &diary).unwrap();
    }

    /// The short IDs of the current line by reading it whole, newest first.
    fn uncached_line(repo: &Repository) -> Vec<String> {
        let saves: Vec<ChangeId> = chain(repo, current_save(repo).unwrap())
            .map(|s| s.unwrap().1.change_id)
            .collect();
        saves
            .iter()
            .zip(plainly(&saves))
            .map(|(c, len)| c.to_letters()[..len].to_owned())
            .collect()
    }

    fn cached_line(repo: &Repository) -> Vec<String> {
        let mut shown = Vec::new();
        for_each_on_line::<RepoError>(repo, |_, _, short| {
            shown.push(short.to_owned());
            Ok(())
        })
        .unwrap();
        shown
    }

    /// `main`'s saves as the store has them, newest first.
    fn true_line(repo: &Repository) -> Vec<(Id, ChangeId)> {
        chain(repo, repo.current().unwrap().1.state.lines["main"])
            .map(|s| s.map(|(id, save)| (id, save.change_id)).unwrap())
            .collect()
    }

    /// A cache that has `main`'s saves right, but gives its oldest a change ID that shares 7
    /// letters with `klmnopk`.
    fn lying_cache(repo: &Repository) -> crate::changeids::ChangeIds {
        let mut saves = true_line(repo);
        saves.last_mut().unwrap().1 = change("klmnopkl");
        let mut lie = crate::changeids::ChangeIds::default();
        lie.set("main", saves);
        lie
    }

    fn plant_cache(dir: &std::path::Path, bytes: &[u8]) {
        std::fs::write(dir.join(".bolt/cache/change-ids"), bytes).unwrap();
    }

    /// Plants `saves` as `main`'s list, sealed with this account's own key.
    fn plant_list(dir: &std::path::Path, saves: Vec<(Id, ChangeId)>) {
        let mut lie = crate::changeids::ChangeIds::default();
        lie.set("main", saves);
        plant_cache(dir, &crate::keyed::seal(&KEY, &lie.encode()));
    }

    /// What `resolve` gives for `text` with `main`'s list planted as `saves`, and with no cache.
    fn resolved(
        dir: &std::path::Path,
        repo: &Repository,
        saves: &[(Id, ChangeId)],
        text: &str,
    ) -> (String, String) {
        plant_list(dir, saves.to_vec());
        let cached = format!("{:?}", resolve(repo, text));
        let cold = Repository::open(dir).unwrap();
        (cached, format!("{:?}", resolve(&cold, text)))
    }

    #[test]
    fn a_cache_from_another_account_is_ignored() {
        let (dir, repo, ids) = project_with(&["klmnopl", "klmnopk"]);
        let lie = lying_cache(&repo);
        let tip = change("klmnopk");
        // The lie, sealed with this account's key, is believed where no save is read: the test
        // can tell.
        plant_cache(dir.path(), &crate::keyed::seal(&KEY, &lie.encode()));
        assert_eq!(short_change_id(&repo, &ids[2], &tip).unwrap(), "klmnopkk");
        let other = [0x5a; crate::keyed::KEY_LEN];
        assert_ne!(other, KEY);
        plant_cache(dir.path(), &crate::keyed::seal(&other, &lie.encode()));
        assert_eq!(cached_line(&repo), uncached_line(&repo));
        assert_eq!(cached_line(&repo)[0], "klmnopk");
        plant_cache(dir.path(), &crate::keyed::seal(&other, &lie.encode()));
        assert_eq!(short_change_id(&repo, &ids[2], &tip).unwrap(), "klmnopk");
    }

    #[test]
    fn an_edited_cache_is_ignored() {
        let (dir, repo, ids) = project_with(&["klmnopl", "klmnopk"]);
        let tip = change("klmnopk");
        // A cache Bolt wrote, its body swapped for the lie and its seal kept.
        assert_eq!(cached_line(&repo), uncached_line(&repo));
        let path = dir.path().join(".bolt/cache/change-ids");
        let written = std::fs::read(&path).unwrap();
        let seal = &written[written.len() - crate::keyed::KEY_LEN..];
        let mut edited = lying_cache(&repo).encode();
        edited.extend_from_slice(seal);
        plant_cache(dir.path(), &edited);
        assert_eq!(short_change_id(&repo, &ids[2], &tip).unwrap(), "klmnopk");
        // Every single bit flipped in what Bolt wrote, too.
        for at in [0, written.len() / 2, written.len() - 1] {
            let mut flipped = written.clone();
            flipped[at] ^= 1;
            plant_cache(dir.path(), &flipped);
            assert_eq!(short_change_id(&repo, &ids[2], &tip).unwrap(), "klmnopk");
        }
    }

    /// Every save the listing hands out, with its short form.
    fn listing(repo: &Repository) -> Vec<(Id, Save, String)> {
        let mut shown = Vec::new();
        for_each_on_line::<RepoError>(repo, |id, save, short| {
            shown.push((*id, save.clone(), short.to_owned()));
            Ok(())
        })
        .unwrap();
        shown
    }

    #[test]
    fn a_wrong_list_changes_no_save_handed_out() {
        let (dir, mut repo, ids) = project_with(&["klmnopl", "klmnopk", "mmmm"]);
        let side = plant(&mut repo, &[ids[1]], "klmnopkl", Some("side"));
        let truth = true_line(&repo);
        let cold = listing(&Repository::open(dir.path()).unwrap());
        let mut swapped = truth.clone();
        swapped[1] = (side, change("klmnopkl"));
        let mut crossed = truth.clone();
        (crossed[1].1, crossed[2].1) = (truth[2].1, truth[1].1);
        let mut added = truth.clone();
        added.push((Id([0x5a; 32]), change("klmnopkz")));
        let lies = [
            swapped,
            crossed,
            vec![truth[0], truth[2], truth[3]],
            truth[..2].to_vec(),
            added,
        ];
        for lie in lies {
            let wrong = (0..).find(|&at| lie.get(at) != truth.get(at)).unwrap();
            plant_list(dir.path(), lie);
            let shown = listing(&repo);
            let saves = |l: &[(Id, Save, String)]| -> Vec<(Id, Save)> {
                l.iter().map(|(id, save, _)| (*id, save.clone())).collect()
            };
            assert_eq!(saves(&shown), saves(&cold));
            // The short forms before the first wrong entry rest on the list, as all of them do.
            let shorts = |l: &[(Id, Save, String)]| -> Vec<String> {
                l.iter().skip(wrong).map(|(.., s)| s.clone()).collect()
            };
            assert_eq!(shorts(&shown), shorts(&cold));
            let fixed = crate::changeids::ChangeIds::load(&repo);
            assert_eq!(fixed.get("main").unwrap(), truth.as_slice());
        }
    }

    #[test]
    fn a_warm_listing_reads_each_save_once_and_one_before_the_first() {
        let starts = vec!["lmno"; 299];
        let (_dir, repo, _) = project_with(&starts);
        cached_line(&repo);
        let before = repo.store().saves_read();
        let mut first = None;
        for_each_on_line::<RepoError>(&repo, |_, _, _| {
            first.get_or_insert(repo.store().saves_read() - before);
            Ok(())
        })
        .unwrap();
        assert_eq!(first, Some(1));
        assert_eq!(repo.store().saves_read() - before, 300);
    }

    #[test]
    fn a_listing_cut_short_reads_no_more_of_the_line() {
        let (_dir, mut repo, ids) = project_with(&vec!["lmno"; 99]);
        cached_line(&repo);
        // A save the cache doesn't hold yet, as after `bolt history | head`.
        let newest = plant(&mut repo, &[ids[99]], "lmnp", Some("main"));
        let cold = crate::changeids::ChangeIds::load(&repo);
        assert_ne!(cold.get("main").unwrap()[0].0, newest);
        let before = repo.store().saves_read();
        let stopped = for_each_on_line::<RepoError>(&repo, |_, _, _| {
            Err(RepoError::Damaged {
                detail: "the pipe closed".into(),
            })
        });
        assert!(stopped.is_err());
        assert!(repo.store().saves_read() - before <= 2);
        // Written before the first save was handed out, the cache holds the line as it is.
        let kept = crate::changeids::ChangeIds::load(&repo);
        assert_eq!(kept.get("main").unwrap(), true_line(&repo).as_slice());
    }

    #[test]
    fn a_list_with_change_ids_swapped_finds_what_the_store_finds() {
        let (dir, repo, _) = project_with(&["klmnopl", "klmnopk", "mmmm"]);
        let truth = true_line(&repo);
        let mut lie = truth.clone();
        (lie[1].1, lie[2].1) = (truth[2].1, truth[1].1);
        for (_, change_id) in &truth[1..3] {
            let (cached, cold) = resolved(dir.path(), &repo, &lie, &change_id.to_letters());
            assert_eq!(cached, cold);
            assert!(cold.starts_with("Ok"), "{cold}");
        }
    }

    #[test]
    fn a_list_wrong_above_the_save_found_is_dropped_though_the_answer_agrees() {
        let mut starts = vec!["mmmm"; 20];
        starts.extend(["klmnopl", "nnnn"]);
        let (dir, repo, ids) = project_with(&starts);
        let mut lie = true_line(&repo);
        lie[0].1 = change("pppp");
        let text = lie[1].1.to_letters();
        let (cached, cold) = resolved(dir.path(), &repo, &lie, &text);
        assert_eq!(cached, cold);
        // The lie is gone: asked again, only the saves down to the one found are read.
        let before = repo.store().saves_read();
        assert_eq!(resolve(&repo, &text).unwrap(), ids[21]);
        let read = repo.store().saves_read() - before;
        assert!(read <= 4, "{read} saves read for a save 1 back");
    }

    #[test]
    fn a_list_that_drops_a_save_still_finds_it() {
        let (dir, repo, _) = project_with(&["klmnopl", "klmnopk", "mmmm"]);
        let truth = true_line(&repo);
        let lie = vec![truth[0], truth[2], truth[3]];
        let (cached, cold) = resolved(dir.path(), &repo, &lie, &truth[1].1.to_letters());
        assert_eq!(cached, cold);
        assert!(cold.starts_with("Ok"), "{cold}");
    }

    #[test]
    fn a_list_that_adds_a_save_invents_no_clash() {
        let (dir, mut repo, ids) = project_with(&["klmnopl", "klmnopk", "mmmm"]);
        // A real save with a change ID like one of main's, but on no line main reaches.
        let elsewhere = plant(&mut repo, &[ids[0]], "klmnopkm", Some("side"));
        let truth = true_line(&repo);
        let invented = (Id([0x5a; 32]), change("klmnopkz"));
        let mut added = truth.clone();
        added.push(invented);
        let mut from_elsewhere = truth.clone();
        from_elsewhere.push((elsewhere, change("klmnopkm")));
        // And a list where the invented save takes the place of the real one.
        let mut instead = truth.clone();
        instead[1] = invented;
        for lie in [added, from_elsewhere, instead] {
            let (cached, cold) = resolved(dir.path(), &repo, &lie, "klmnopk");
            assert_eq!(cached, cold);
            assert_eq!(cold, format!("{:?}", Ok::<Id, ()>(ids[2])));
        }
    }

    #[test]
    fn a_whole_id_a_list_holds_wrongly_is_found_as_without_the_list() {
        let (dir, mut repo, ids) = project_with(&["klmnopl", "klmnopk", "mmmm"]);
        let listing = repo.store().save(&ids[3], false).unwrap().root;
        let stray = plant(&mut repo, &[], "nnnn", None);
        let truth = true_line(&repo);
        for (stranger, change_id) in [(listing, change("pppp")), (stray, change("nnnn"))] {
            let mut lie = truth.clone();
            lie.insert(1, (stranger, change_id));
            let (cached, cold) = resolved(dir.path(), &repo, &lie, &stranger.to_hex());
            assert_eq!(cached, cold);
            assert!(cold.starts_with("Err(NotFound"), "{cold}");
            let fixed = crate::changeids::ChangeIds::load(&repo);
            assert_eq!(fixed.get("main").unwrap(), truth.as_slice());
        }
        // At a save, the lie is dropped and nothing is read but the lines' walk and the one
        // link the check reads, the newest save, which the walk reads too.
        set_position(&mut repo, Position::AtSave(ids[1]));
        let mut lie = truth.clone();
        lie.insert(1, (stray, change("nnnn")));
        plant_list(dir.path(), lie);
        let before = repo.store().saves_read();
        let cached = format!("{:?}", resolve(&repo, &stray.to_hex()));
        let cached_reads = repo.store().saves_read() - before;
        let cold = Repository::open(dir.path()).unwrap();
        let shown = format!("{:?}", resolve(&cold, &stray.to_hex()));
        assert_eq!(cached, shown);
        assert_eq!(cached_reads, cold.store().saves_read() + 1);
        let dropped = crate::changeids::ChangeIds::load(&repo);
        assert!(dropped.get("main").is_none());
    }

    #[test]
    fn a_whole_id_costs_what_its_save_is_far_from_the_newest() {
        let (_dir, repo, ids) = project_with(&vec!["lmno"; 30]);
        cached_line(&repo);
        for (back, most) in [(0, 2), (10, 12)] {
            let id = ids[ids.len() - 1 - back];
            let before = repo.store().saves_read();
            assert_eq!(resolve(&repo, &id.to_hex()).unwrap(), id);
            let read = repo.store().saves_read() - before;
            assert!(read <= most, "{read} saves read for one {back} back");
        }
    }

    #[test]
    fn a_list_with_another_line_s_save_changes_no_answer() {
        let (dir, mut repo, ids) = project_with(&["klmnopl", "klmnopk", "mmmm"]);
        let side = plant(&mut repo, &[ids[1]], "nopqrs", Some("side"));
        let mut lie = true_line(&repo);
        lie[1] = (side, change("nopqrs"));
        let (cached, cold) = resolved(dir.path(), &repo, &lie, "nopqr");
        assert_eq!(cached, cold);
        // A line name that a planted save's change ID starts with still names the line.
        let work = plant(&mut repo, &[ids[0]], "workkk", Some("elsewhere"));
        name_line(&mut repo, "work", ids[1]);
        let mut lie = true_line(&repo);
        lie[1] = (work, change("workkk"));
        let (cached, cold) = resolved(dir.path(), &repo, &lie, "work");
        assert_eq!(cached, cold);
        assert_eq!(cold, format!("{:?}", Ok::<Id, ()>(ids[1])));
    }

    #[test]
    fn a_change_id_costs_what_its_save_is_far_from_the_newest() {
        let starts: Vec<String> = (0..200u8)
            .map(|n| format!("{}{}", char::from(b'k' + n / 16), char::from(b'k' + n % 16)))
            .collect();
        let starts: Vec<&str> = starts.iter().map(String::as_str).collect();
        let (_dir, repo, ids) = project_with(&starts);
        cached_line(&repo);
        for (back, most) in [(1, 4), (50, 55)] {
            let id = ids[ids.len() - 1 - back];
            let letters = repo
                .store()
                .save(&id, false)
                .unwrap()
                .change_id
                .to_letters();
            let before = repo.store().saves_read();
            assert_eq!(resolve(&repo, &letters).unwrap(), id);
            let read = repo.store().saves_read() - before;
            assert!(read <= most, "{read} saves read for one {back} back");
        }
    }

    #[derive(Debug, Clone)]
    enum Step {
        Save(&'static str),
        /// A save of the folder through `save`, on the line the project is on.
        RealSave,
        Back(usize),
        /// Moves `main` to one of the saves made so far: back, forward or elsewhere.
        To(usize),
        Side(&'static str),
        /// Makes or moves the line `side` to one of the saves made so far.
        Line(usize),
        Amend,
        /// Goes to one of the saves made so far, off the line.
        Goto(usize),
        OnLine,
    }

    const STARTS: [&str; 5] = ["klmn", "klmo", "klmnop", "zzzz", "lmno"];

    fn step() -> impl Strategy<Value = Step> {
        let start = prop::sample::select(STARTS.to_vec());
        prop_oneof![
            3 => start.clone().prop_map(Step::Save),
            1 => Just(Step::RealSave),
            1 => (0..4usize).prop_map(Step::Back),
            1 => any::<usize>().prop_map(Step::To),
            1 => start.prop_map(Step::Side),
            1 => any::<usize>().prop_map(Step::Line),
            1 => Just(Step::Amend),
            1 => any::<usize>().prop_map(Step::Goto),
            1 => Just(Step::OnLine),
        ]
    }

    /// Saves the folder through `save`, or amends `main`'s newest save, with a message no save
    /// has yet. `None` if the amend is refused because another line shows the save.
    fn save_through(
        repo: &mut Repository,
        message: &str,
        amend: bool,
    ) -> Option<(Id, ChangeId, String)> {
        let ada = who();
        let req = crate::snapshot::SaveRequest {
            message: Some(message),
            who: &ada,
            command: "save",
            accept_crlf: false,
            git_excludes: None,
            new_line: None,
            only: None,
            amend,
        };
        match crate::snapshot::save_folder(repo, &req, &mut |_| {}, &mut |_| true) {
            Ok(crate::snapshot::SaveOutcome::Saved {
                save_id,
                save,
                short_change_id,
                ..
            }) => Some((save_id, save.change_id, short_change_id)),
            Err(crate::snapshot::SaveError::AmendShared { .. }) => None,
            other => panic!("{other:?}"),
        }
    }

    /// Whether a line other than `main` holds `main`'s newest save or reaches its change ID.
    fn shown_elsewhere(repo: &Repository) -> bool {
        let lines = repo.current().unwrap().1.state.lines;
        let tip = lines["main"];
        let change_id = repo.store().save(&tip, false).unwrap().change_id;
        lines
            .iter()
            .filter(|(name, _)| *name != "main")
            .any(|(_, &other)| {
                other == tip || chain(repo, other).any(|s| s.unwrap().1.change_id == change_id)
            })
    }

    #[test]
    fn an_amend_asks_each_line_without_building_it_a_list() {
        let starts: Vec<String> = (0..29u8)
            .map(|n| format!("{}{}", char::from(b'k' + n / 16), char::from(b'k' + n % 16)))
            .collect();
        let starts: Vec<&str> = starts.iter().map(String::as_str).collect();
        let (_dir, mut repo, ids) = project_with(&starts);
        let tip = ids[29];
        let mut side_saves = 0;
        for n in 0..11 {
            name_line(&mut repo, &format!("at{n:02}"), ids[n * 2]);
        }
        for n in 0..5 {
            name_line(&mut repo, &format!("amended{n}"), tip);
        }
        // Lines of a few saves of their own on a save of main, and two that share one.
        for (n, &on) in ids.iter().take(20).enumerate() {
            let mut top = on;
            for _ in 0..=n % 3 {
                top = plant(&mut repo, &[top], "pppp", None);
                side_saves += 1;
            }
            name_line(&mut repo, &format!("side{n:02}"), top);
        }
        let shared = plant(&mut repo, &[ids[4]], "qqqq", Some("shared"));
        plant(&mut repo, &[shared], "rrrr", Some("shared2"));
        side_saves += 2;
        // A history that never meets main's.
        let alone = plant(&mut repo, &[], "ssss", None);
        plant(&mut repo, &[alone], "tttt", Some("alone"));
        side_saves += 2;
        // Another save with the amended save's change ID, as an undone amend leaves.
        plant(&mut repo, &[ids[6]], starts[28], Some("twin"));
        side_saves += 1;
        // A line of two saves of its own on the amended save.
        let above = plant(&mut repo, &[tip], "uuuu", None);
        plant(&mut repo, &[above], "uuuu", Some("above"));
        side_saves += 2;
        assert_eq!(repo.current().unwrap().1.state.lines.len(), 42);
        let by = asked_as_walked(&repo, side_saves);
        for shown in ["twin", "amended0", "above"] {
            assert!(by.contains(&format!("line {shown}")), "{shown}: {by:?}");
        }
        assert!(!by.contains(&"line side00".to_owned()), "{by:?}");
    }

    #[test]
    fn an_amend_finds_a_line_between_two_saves_of_its_change_id() {
        // `main` holds the amended save's change ID twice, at its newest save and two back.
        let (_dir, mut repo, ids) = project_with(&["kkkk", "klkk", "zzzz", "kmkk", "zzzz"]);
        name_line(&mut repo, "between", ids[4]);
        plant(&mut repo, &[ids[4]], "uuuu", Some("between2"));
        name_line(&mut repo, "older", ids[2]);
        plant(&mut repo, &[ids[2]], "uuuu", Some("older2"));
        let by = asked_as_walked(&repo, 2);
        assert_eq!(by, ["line between", "line between2"]);
    }

    /// The lines `shown_elsewhere` names for an amend of `main`'s newest save, checked against
    /// walking each line whole; that only `main` gains a list; and that no more saves are read
    /// than the `on_no_list` saves no line's list holds.
    fn asked_as_walked(repo: &Repository, on_no_list: usize) -> Vec<String> {
        let state = repo.current().unwrap().1.state;
        let tip = state.lines["main"];
        let save = repo.store().save(&tip, false).unwrap();
        let expected: Vec<String> = state
            .lines
            .iter()
            .filter(|(name, _)| *name != "main")
            .filter(|&(_, &other)| {
                other == tip || chain(repo, other).any(|s| s.unwrap().1.change_id == save.change_id)
            })
            .map(|(name, _)| format!("line {name}"))
            .collect();
        cached_line(repo);
        let mut cache = ChangeIds::load(repo);
        let before = repo.store().saves_read();
        let by =
            crate::snapshot::shown_elsewhere(repo, &mut cache, &state, "main", tip, &save).unwrap();
        let read = repo.store().saves_read() - before;
        assert_eq!(by, expected);
        assert!(
            read <= on_no_list,
            "{read} saves read, {on_no_list} on no list"
        );
        assert!(cache.get("main").is_some());
        for name in state.lines.keys().filter(|name| *name != "main") {
            assert!(cache.get(name).is_none(), "{name}");
        }
        by
    }

    /// What `shown_elsewhere` names for an amend of `line`'s newest save, and the saves it read.
    fn shown_for(repo: &Repository, line: &str) -> (Vec<String>, usize) {
        let state = repo.current().unwrap().1.state;
        let tip = state.lines[line];
        let save = repo.store().save(&tip, false).unwrap();
        let mut cache = ChangeIds::load(repo);
        let before = repo.store().saves_read();
        let by =
            crate::snapshot::shown_elsewhere(repo, &mut cache, &state, line, tip, &save).unwrap();
        cache.store(repo, &state);
        (by, repo.store().saves_read() - before)
    }

    /// Plants `n` saves of their own on `on`, the last as the line `name`, and returns them,
    /// oldest first.
    fn side_line(repo: &mut Repository, name: &str, on: Id, start: &str, n: usize) -> Vec<Id> {
        let mut saves = vec![];
        let mut top = on;
        for k in 0..n {
            top = plant(repo, &[top], start, (k + 1 == n).then_some(name));
            saves.push(top);
        }
        saves
    }

    #[test]
    fn a_second_amend_reads_no_save_of_a_line_that_has_not_moved() {
        let (dir, mut repo, ids) = project_with(&["kkkk", "klkk", "kmkk"]);
        side_line(&mut repo, "old", ids[1], "pppp", 20);
        cached_line(&repo);
        let (by, first) = shown_for(&repo, "main");
        assert!(by.is_empty(), "{by:?}");
        assert!(first >= 20, "{first}");
        assert_eq!(shown_for(&repo, "main"), (vec![], 0));
        // An amend moves main; the side line's run still meets main's list.
        assert!(save_through(&mut repo, "again", true).is_some());
        cached_line(&repo);
        assert_eq!(shown_for(&repo, "main"), (vec![], 0));
        // With the file gone, the line is read again.
        std::fs::remove_file(dir.path().join(".bolt/cache/other-lines")).unwrap();
        assert!(shown_for(&repo, "main").1 >= 20);
    }

    /// What `shown_elsewhere` names for an amend of `line`, with `cache/other-lines` as it is and
    /// with it deleted, both checked against walking each line whole.
    fn named_both_ways(dir: &std::path::Path, repo: &Repository, line: &str) -> Vec<String> {
        let state = repo.current().unwrap().1.state;
        let tip = state.lines[line];
        let change_id = repo.store().save(&tip, false).unwrap().change_id;
        let walked: Vec<String> = state
            .lines
            .iter()
            .filter(|(name, _)| *name != line)
            .filter(|&(_, &other)| {
                other == tip || chain(repo, other).any(|s| s.unwrap().1.change_id == change_id)
            })
            .map(|(name, _)| format!("line {name}"))
            .collect();
        let file = dir.join(".bolt/cache/other-lines");
        let kept = std::fs::read(&file).ok();
        let _ = std::fs::remove_file(&file);
        let cold = shown_for(repo, line).0;
        match kept {
            Some(bytes) => std::fs::write(&file, bytes).unwrap(),
            None => std::fs::remove_file(&file).unwrap(),
        }
        let warm = shown_for(repo, line).0;
        assert_eq!(cold, walked);
        assert_eq!(warm, walked);
        warm
    }

    #[test]
    fn the_amend_check_names_the_same_lines_with_kept_runs_and_without() {
        let (dir, mut repo, ids) = project_with(&["kkkk", "klkk", "kmkk", "knkk", "kokk"]);
        let check = |repo: &Repository, line: &str| named_both_ways(dir.path(), repo, line);
        // A line off main below the amended save, and one off the middle of that line's run.
        let below = side_line(&mut repo, "below", ids[2], "qqqq", 3);
        side_line(&mut repo, "off-run", below[1], "rrrr", 2);
        assert!(check(&repo, "main").is_empty());
        assert!(check(&repo, "main").is_empty());
        assert!(save_through(&mut repo, "amended", true).is_some());
        // A line whose own saves hold the amended save's change ID, as an undone amend leaves.
        let twin = plant(&mut repo, &[ids[1]], "kokk", None);
        side_line(&mut repo, "twin", twin, "pppp", 2);
        assert_eq!(check(&repo, "main"), ["line twin"]);
        assert_eq!(check(&repo, "main"), ["line twin"]);
        // A line that moved after its run was kept.
        plant(&mut repo, &[below[2]], "kokk", Some("below"));
        assert_eq!(check(&repo, "main"), ["line below", "line twin"]);
        // Amending another line, where main is one of the others.
        set_position(&mut repo, Position::OnLine("off-run".into()));
        let off_run = repo.current().unwrap().1.state.lines["off-run"];
        side_line(&mut repo, "copy", off_run, "rrrr", 1);
        assert_eq!(check(&repo, "off-run"), ["line copy"]);
        assert_eq!(check(&repo, "off-run"), ["line copy"]);
        assert_eq!(check(&repo, "below"), ["line main", "line twin"]);
    }

    #[test]
    fn a_run_that_starts_at_no_line_s_newest_save_is_not_used() {
        let (dir, mut repo, ids) = project_with(&["kkkk", "klkk"]);
        let below = side_line(&mut repo, "below", ids[0], "qqqq", 3);
        let off = side_line(&mut repo, "off", below[1], "rrrr", 1);
        let tip = repo.current().unwrap().1.state.lines["main"];
        let change_id = repo.store().save(&tip, false).unwrap().change_id;
        // Under a good seal, runs that say main's change ID is on `below` and `off` past their
        // newest save.
        let mut lie = crate::otherlines::OtherLines::default();
        lie.add(below[1], &[(below[1], change_id)], None);
        lie.add(below[0], &[(below[0], change_id)], None);
        assert!(lie.run(&off[0]).is_none());
        let file = dir.path().join(".bolt/cache/other-lines");
        std::fs::write(&file, crate::keyed::seal(&KEY, &lie.encode())).unwrap();
        assert!(shown_for(&repo, "main").0.is_empty());
        // A lie at a line's newest save is believed: the seal is what it is trusted for.
        let mut lie = crate::otherlines::OtherLines::default();
        lie.add(off[0], &[(off[0], change_id)], None);
        std::fs::write(&file, crate::keyed::seal(&KEY, &lie.encode())).unwrap();
        assert_eq!(shown_for(&repo, "main").0, ["line off"]);
    }

    /// Puts the project at `position`, as `goto` would, without touching the folder.
    fn set_position(repo: &mut Repository, position: Position) {
        let lock = repo.lock_for_change("goto", &mut |_| {}).unwrap();
        let (head, entry) = repo.current().unwrap();
        let mut state = entry.state.clone();
        state.position = position;
        let diary = DiaryEntry {
            parent: Some(head),
            action: Action::Command,
            command: "goto".into(),
            who: who(),
            bolt_version: "0.1.0".into(),
            state,
            folder_change: None,
        };
        let packs = repo.new_packs().unwrap();
        repo.commit(&lock, packs, &diary).unwrap();
    }

    /// Everything the cache answers for, as `repo` answers it, for the saves `saves`.
    fn answers(repo: &Repository, saves: &[Id]) -> Vec<String> {
        let mut out = vec![format!("{:?}", cached_line(repo))];
        let line = LineIds::read(repo).unwrap();
        for id in saves {
            let change_id = repo.store().save(id, false).unwrap().change_id;
            out.push(short_change_id(repo, id, &change_id).unwrap());
            out.push(format!(
                "{} {}",
                line.short(id, &change_id),
                line.shared(id, &change_id)
            ));
            out.push(format!("{:?}", resolve(repo, &change_id.to_letters())));
            out.push(format!("{:?}", resolve(repo, &id.to_hex())));
            out.push(format!("{:?}", resolve(repo, &id.to_hex().to_uppercase())));
        }
        for start in STARTS {
            out.push(format!("{:?}", resolve(repo, start)));
        }
        out.push(format!("{:?}", resolve(repo, &Id([0x5a; 32]).to_hex())));
        out
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(24))]
        // Saves the cache never saw and saves it learns of, undos that move a line back and
        // forward again, saves off the line, a second line, amends that give a new save an old
        // change ID, and the project at a save, with the cache read and rewritten in between:
        // every answer is the one a handle with no cache gives.
        #[test]
        fn answers_match_the_uncached_ones(steps in prop::collection::vec(step(), 1..12)) {
            let (dir, mut repo, mut saves) = project_with(&[]);
            for step in steps {
                let on_line =
                    matches!(repo.current().unwrap().1.state.position, Position::OnLine(_));
                match step {
                    Step::Save(start) => {
                        let tip = repo.current().unwrap().1.state.lines["main"];
                        saves.push(plant(&mut repo, &[tip], start, Some("main")));
                    }
                    Step::RealSave if on_line => {
                        let n = saves.len();
                        std::fs::write(dir.path().join("f.txt"), format!("{n}")).unwrap();
                        let (id, change_id, short) =
                            save_through(&mut repo, &format!("{n}"), false).unwrap();
                        prop_assert_eq!(short, walk_short(&repo, id, &id, &change_id));
                        saves.push(id);
                    }
                    Step::RealSave => {}
                    Step::Back(n) => {
                        let tip = repo.current().unwrap().1.state.lines["main"];
                        let back = chain(&repo, tip).nth(n).map(|s| s.unwrap().0);
                        if let Some(id) = back {
                            move_line(&mut repo, "main", id);
                        }
                    }
                    Step::To(n) => move_line(&mut repo, "main", saves[n % saves.len()]),
                    Step::Side(start) => saves.push(plant(&mut repo, &[saves[0]], start, None)),
                    Step::Line(n) => name_line(&mut repo, "side", saves[n % saves.len()]),
                    // A message no save has yet, so there is always something to amend.
                    Step::Amend if on_line => {
                        let shared = shown_elsewhere(&repo);
                        let done = save_through(&mut repo, &format!("{}", saves.len()), true);
                        prop_assert_eq!(done.is_none(), shared);
                        if let Some((id, change_id, short)) = done {
                            prop_assert_eq!(short, walk_short(&repo, id, &id, &change_id));
                            saves.push(id);
                        }
                    }
                    Step::Amend => {}
                    Step::Goto(n) => set_position(&mut repo, Position::AtSave(saves[n % saves.len()])),
                    Step::OnLine => set_position(&mut repo, Position::OnLine("main".into())),
                }
                prop_assert_eq!(cached_line(&repo), uncached_line(&repo));
                let cold = Repository::open(dir.path()).unwrap();
                prop_assert_eq!(answers(&repo, &saves), answers(&cold, &saves));
                let tip = current_save(&repo).unwrap();
                for id in &saves {
                    let change_id = repo.store().save(id, false).unwrap().change_id;
                    let walked = walk_short(&repo, tip, id, &change_id);
                    prop_assert_eq!(&short_change_id(&repo, id, &change_id).unwrap(), &walked);
                    let another = chain(&repo, tip)
                        .map(Result::unwrap)
                        .any(|(other, save)| other != *id && save.change_id == change_id);
                    let line = LineIds::read(&repo).unwrap();
                    prop_assert_eq!(line.shared(id, &change_id), another);
                }
            }
        }
    }

    /// The saves read by going to the save before the newest of a line of `len` saves and back,
    /// by its change ID and by its whole ID, once a command has shown the line.
    fn reads_to_go_back_and_forth(len: u8) -> usize {
        let starts: Vec<String> = (0..len)
            .map(|n| format!("{}{}", char::from(b'k' + n / 16), char::from(b'k' + n % 16)))
            .collect();
        let starts: Vec<&str> = starts.iter().map(String::as_str).collect();
        let (_dir, mut repo, ids) = project_with(&starts);
        let before = ids[ids.len() - 2];
        let letters = repo
            .store()
            .save(&before, false)
            .unwrap()
            .change_id
            .to_letters();
        cached_line(&repo);
        let read = repo.store().saves_read();
        for text in [&letters[..4], "main", &before.to_hex(), "main"] {
            let target = crate::lines::resolve_target(&repo, text).unwrap();
            let done =
                crate::switch::goto(&mut repo, &target, &who(), "goto", None, &mut |_| {}).unwrap();
            let save = repo.store().save(&done.save, false).unwrap();
            short_change_id(&repo, &done.save, &save.change_id).unwrap();
        }
        repo.store().saves_read() - read
    }

    #[test]
    fn a_goto_to_the_save_before_reads_no_line() {
        assert_eq!(
            reads_to_go_back_and_forth(20),
            reads_to_go_back_and_forth(120)
        );
    }

    #[test]
    fn a_line_that_cant_be_read_is_an_error() {
        let (dir, repo, ids) = project_with(&["klmnopk"]);
        for entry in std::fs::read_dir(repo.packs_dir()).unwrap() {
            let path = entry.unwrap().path();
            let found = crate::pack::PackReader::open(&path)
                .unwrap()
                .find(&ids[0])
                .unwrap();
            if let Some(found) = found {
                let mut bytes = std::fs::read(&path).unwrap();
                bytes[usize::try_from(found.offset + crate::pack::ENTRY_HEADER_LEN).unwrap()] ^= 1;
                std::fs::write(&path, bytes).unwrap();
            }
        }
        let repo = Repository::open(dir.path()).unwrap();
        assert!(matches!(
            short_change_id(&repo, &ids[1], &change("klmnopk")),
            Err(RepoError::Damaged { .. })
        ));
        let mut printed = 0;
        let result = for_each_on_line::<RepoError>(&repo, |_, _, _| {
            printed += 1;
            Ok(())
        });
        assert!(result.is_err());
        assert_eq!(printed, 0, "nothing is handed out before the line is known");
    }
}
