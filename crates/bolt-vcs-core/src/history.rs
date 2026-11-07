//! Line history, short change IDs, and finding a save from what the user typed. A line can be
//! any length and a message 1 MiB, so history is read one save at a time and never held.

use std::collections::HashSet;

use crate::changeids::ChangeIds;
use crate::diary::{DiaryEntry, Position};
use crate::id::{ChangeId, Id};
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
pub(crate) fn short_among(ids: &[ChangeId], change_id: &ChangeId, itself: bool) -> String {
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

/// The shortest form of `change_id` (at least 4 letters) that no other save on the current line
/// starts with. `id` is the save it belongs to, on the line or not. The line's change IDs come
/// from the cache when it is current.
pub fn short_change_id(
    repo: &Repository,
    id: &Id,
    change_id: &ChangeId,
) -> Result<String, RepoError> {
    let (_, entry) = repo.current()?;
    let tip = tip_of(&entry)?;
    let Position::OnLine(line) = &entry.state.position else {
        return walk_short(repo, tip, id, change_id);
    };
    let key = *repo.cache_key();
    let mut cache = ChangeIds::load(repo, &key);
    let ids = cache.line(repo, line, tip)?;
    let listed = ids.binary_search(change_id).is_ok();
    let short = match (listed, *id == tip) {
        (false, _) => Some(short_among(ids, change_id, false)),
        (true, true) => Some(short_among(ids, change_id, true)),
        // Whether `id` is on the line, or another save there has its change ID, only the line
        // says.
        (true, false) => None,
    };
    cache.store(repo, &key, &entry.state);
    match short {
        Some(short) => Ok(short),
        None => walk_short(repo, tip, id, change_id),
    }
}

/// `short_change_id`, by reading the line from `tip`.
fn walk_short(
    repo: &Repository,
    tip: Id,
    id: &Id,
    change_id: &ChangeId,
) -> Result<String, RepoError> {
    let mut common = 0;
    for step in chain(repo, tip) {
        let (other, save) = step?;
        if other != *id {
            common = common.max(common_letters(change_id, &save.change_id));
        }
    }
    Ok(change_id.to_letters()[..short_len(common)].to_owned())
}

/// Calls `each` with every save on the current line, newest first, and the shortest form of its
/// change ID. The line's change IDs come first, from the cache when it is current and otherwise
/// by reading the line once for them alone, so that only the save being handed out is held.
pub fn for_each_on_line<E: From<RepoError>>(
    repo: &Repository,
    mut each: impl FnMut(&Id, &Save, &str) -> Result<(), E>,
) -> Result<(), E> {
    let (_, entry) = repo.current()?;
    let tip = tip_of(&entry)?;
    let ids = match &entry.state.position {
        Position::OnLine(line) => {
            let key = *repo.cache_key();
            let mut cache = ChangeIds::load(repo, &key);
            let ids = cache.line(repo, line, tip)?.to_vec();
            cache.store(repo, &key, &entry.state);
            ids
        }
        Position::AtSave(_) => {
            let mut ids = chain(repo, tip)
                .map(|step| step.map(|(_, save)| save.change_id))
                .collect::<Result<Vec<ChangeId>, RepoError>>()?;
            ids.sort_unstable();
            ids
        }
    };
    let lengths = short_lengths(&ids);
    for step in chain(repo, tip) {
        let (id, save) = step?;
        let at = ids.partition_point(|c| *c < save.change_id);
        // Every save of the line is among `ids`; all its letters if somehow not.
        let len = match ids.get(at) {
            Some(c) if *c == save.change_id => lengths[at],
            _ => LETTERS,
        };
        each(&id, &save, &save.change_id.to_letters()[..len])?;
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

/// The saves on the line from `tip` whose change ID starts with `text`, each with its change ID.
fn by_change_prefix(
    repo: &Repository,
    tip: Id,
    text: &str,
) -> Result<Vec<(Id, String)>, ResolveError> {
    let mut found = Vec::new();
    for step in chain(repo, tip) {
        let (id, save) = step?;
        let letters = save.change_id.to_letters();
        if letters.starts_with(text) {
            found.push((id, letters));
        }
    }
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
        return by_change_prefix(repo, tip_of(entry)?, text);
    }
    if text.len() >= 8 && text.bytes().all(|b| b.is_ascii_hexdigit()) {
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

    /// A project whose line `main` ends with a save for each start given, the first one oldest.
    /// Also returns the saves' IDs, oldest first, and the first save's.
    fn project_with(starts: &[&str]) -> (tempfile::TempDir, Repository, Vec<Id>) {
        let dir = tempfile::tempdir().unwrap();
        let mut repo = Repository::init(dir.path(), &who()).unwrap();
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

    /// A cache that says `main`, at its newest save, holds one save more than it does, whose ID
    /// shares 7 letters with `klmnopk`.
    fn lying_cache(repo: &Repository) -> crate::changeids::ChangeIds {
        let tip = current_save(repo).unwrap();
        let mut ids: Vec<ChangeId> = chain(repo, tip).map(|s| s.unwrap().1.change_id).collect();
        ids.push(change("klmnopkl"));
        let mut lie = crate::changeids::ChangeIds::default();
        lie.set("main", tip, ids);
        lie
    }

    fn plant_cache(dir: &std::path::Path, bytes: &[u8]) {
        std::fs::write(dir.join(".bolt/cache/change-ids"), bytes).unwrap();
    }

    #[test]
    fn a_cache_from_another_account_is_ignored() {
        let (dir, repo, ids) = project_with(&["klmnopl", "klmnopk"]);
        let lie = lying_cache(&repo);
        // The lie, sealed with this account's key, is believed: the test can tell.
        let key = *repo.cache_key();
        plant_cache(dir.path(), &crate::keyed::seal(&key, &lie.encode()));
        assert_eq!(cached_line(&repo)[0], "klmnopkk");
        let other = [0x5a; crate::keyed::KEY_LEN];
        assert_ne!(other, key);
        plant_cache(dir.path(), &crate::keyed::seal(&other, &lie.encode()));
        assert_eq!(cached_line(&repo), uncached_line(&repo));
        assert_eq!(cached_line(&repo)[0], "klmnopk");
        plant_cache(dir.path(), &crate::keyed::seal(&other, &lie.encode()));
        let tip = change("klmnopk");
        assert_eq!(short_change_id(&repo, &ids[2], &tip).unwrap(), "klmnopk");
    }

    #[test]
    fn an_edited_cache_is_ignored() {
        let (dir, repo, ids) = project_with(&["klmnopl", "klmnopk"]);
        // A cache Bolt wrote, its body swapped for the lie and its seal kept.
        assert_eq!(cached_line(&repo), uncached_line(&repo));
        let path = dir.path().join(".bolt/cache/change-ids");
        let written = std::fs::read(&path).unwrap();
        let seal = &written[written.len() - crate::keyed::KEY_LEN..];
        let mut edited = lying_cache(&repo).encode();
        edited.extend_from_slice(seal);
        plant_cache(dir.path(), &edited);
        assert_eq!(cached_line(&repo)[0], "klmnopk");
        // Every single bit flipped in what Bolt wrote, too.
        for at in [0, written.len() / 2, written.len() - 1] {
            let mut flipped = written.clone();
            flipped[at] ^= 1;
            plant_cache(dir.path(), &flipped);
            let tip = change("klmnopk");
            assert_eq!(short_change_id(&repo, &ids[2], &tip).unwrap(), "klmnopk");
        }
    }

    #[derive(Debug, Clone)]
    enum Step {
        Save(&'static str),
        Back(usize),
        Side(&'static str),
    }

    fn step() -> impl Strategy<Value = Step> {
        let start = prop::sample::select(vec!["klmn", "klmo", "klmnop", "zzzz", "lmno"]);
        prop_oneof![
            3 => start.clone().prop_map(Step::Save),
            1 => (0..4usize).prop_map(Step::Back),
            1 => start.prop_map(Step::Side),
        ]
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(24))]
        // Saves the cache never saw, undos that move a line back, and saves off the line, with
        // the cache read and rewritten in between: the short IDs are always the uncached ones.
        #[test]
        fn short_ids_match_the_uncached_ones(steps in prop::collection::vec(step(), 1..10)) {
            let (_dir, mut repo, mut saves) = project_with(&[]);
            for step in steps {
                match step {
                    Step::Save(start) => {
                        let tip = current_save(&repo).unwrap();
                        saves.push(plant(&mut repo, &[tip], start, Some("main")));
                    }
                    Step::Back(n) => {
                        let back = chain(&repo, current_save(&repo).unwrap()).nth(n);
                        if let Some(Ok((id, _))) = back {
                            move_line(&mut repo, "main", id);
                        }
                    }
                    Step::Side(start) => saves.push(plant(&mut repo, &[saves[0]], start, None)),
                }
                prop_assert_eq!(cached_line(&repo), uncached_line(&repo));
                let tip = current_save(&repo).unwrap();
                for id in &saves {
                    let change_id = repo.store().save(id, false).unwrap().change_id;
                    prop_assert_eq!(
                        short_change_id(&repo, id, &change_id).unwrap(),
                        walk_short(&repo, tip, id, &change_id).unwrap()
                    );
                }
            }
        }
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
