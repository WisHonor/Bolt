//! The steps undo can take and redo can bring back, worked out by walking back from `head`.

use std::collections::HashSet;

use crate::diary::{Action, DiaryEntry};
use crate::id::Id;
use crate::repo::{RepoError, damaged};

/// Reads a diary entry by its ID.
pub(crate) type Read<'a> = dyn FnMut(&Id) -> Result<DiaryEntry, RepoError> + 'a;

/// Steps done and undone, each a command's entry with its ID.
#[derive(Debug)]
pub(crate) struct Steps {
    pub done: Vec<(Id, DiaryEntry)>,
    /// Whether undo can take more steps than `done` holds: the next one isn't the diary's first
    /// entry, which can't be undone.
    pub more_done: bool,
    pub undone: Vec<(Id, DiaryEntry)>,
}

/// The steps at `head`: done, newest first (at most `limit`, and whether there are more), and
/// undone, in the order redo brings them back.
pub(crate) fn steps(head: Id, read: &mut Read<'_>, limit: usize) -> Result<Steps, RepoError> {
    let undone = undone_steps(head, read)?;
    let mut seen = HashSet::new();
    let mut done = Vec::new();
    let mut more_done = false;
    let mut at = Some(head);
    while let Some(step) = newest_done_at(at, read, &mut seen)? {
        if done.len() == limit {
            more_done = step.1.parent.is_some();
            break;
        }
        at = step.1.parent;
        done.push(step);
    }
    Ok(Steps {
        done,
        more_done,
        undone,
    })
}

/// How many done steps are newer than `step`, if it is done at `head`: the done steps are walked
/// newest first, counted and let go, as far as `step`.
pub(crate) fn done_after(
    head: Id,
    step: Id,
    read: &mut Read<'_>,
) -> Result<Option<usize>, RepoError> {
    let (mut seen, mut newer, mut at) = (HashSet::new(), 0, Some(head));
    while let Some((id, entry)) = newest_done_at(at, read, &mut seen)? {
        if id == step {
            return Ok(Some(newer));
        }
        newer += 1;
        at = entry.parent;
    }
    Ok(None)
}

/// The newest done step: what `undo` takes. `None` past the first entry.
pub(crate) fn newest_done(
    head: Id,
    read: &mut Read<'_>,
) -> Result<Option<(Id, DiaryEntry)>, RepoError> {
    newest_done_at(Some(head), read, &mut HashSet::new())
}

fn newest_done_at(
    mut at: Option<Id>,
    read: &mut Read<'_>,
    seen: &mut HashSet<Id>,
) -> Result<Option<(Id, DiaryEntry)>, RepoError> {
    while let Some(id) = at {
        let entry = move_to(id, read, seen)?;
        match entry.action {
            Action::Command => return Ok(Some((id, entry))),
            Action::Redo(step) => return Ok(Some((step, read_step(id, step, read)?))),
            // Once a step is undone, what is done is what was done before it.
            Action::Undo(step) => at = read_step(id, step, read)?.parent,
        }
    }
    Ok(None)
}

/// The steps redo can bring back, the next one first: collected back to the newest command, which
/// cleared the rest.
pub(crate) fn undone_steps(
    head: Id,
    read: &mut Read<'_>,
) -> Result<Vec<(Id, DiaryEntry)>, RepoError> {
    let mut seen = HashSet::new();
    let mut undone = Vec::new();
    // Each redo answers the newest undo before it that no other redo has, and must redo what
    // that undo undid. Held as (redo entry, its step), the last one met on top.
    let mut redos: Vec<(Id, Id)> = Vec::new();
    let mut at = head;
    loop {
        let entry = move_to(at, read, &mut seen)?;
        match entry.action {
            Action::Command => {
                return match redos.last() {
                    Some((redo, _)) => Err(damaged(format!(
                        "diary entry {redo} redoes a step no undo before it undid"
                    ))),
                    None => Ok(undone),
                };
            }
            Action::Redo(step) => redos.push((at, step)),
            Action::Undo(step) => match redos.pop() {
                Some((redo, redone)) if redone != step => {
                    return Err(damaged(format!(
                        "diary entry {redo} redoes {redone}, but the undo it answers, {at}, \
                         undid {step}"
                    )));
                }
                Some(_) => {}
                None => undone.push((step, read_step(at, step, read)?)),
            },
        }
        at = entry.parent.ok_or_else(|| {
            damaged(format!(
                "the diary starts at entry {at}, which isn't a command's"
            ))
        })?;
    }
}

/// Reads the entry a walk moves to. Moves only ever go to older entries, so coming back to one
/// is damage.
fn move_to(id: Id, read: &mut Read<'_>, seen: &mut HashSet<Id>) -> Result<DiaryEntry, RepoError> {
    if !seen.insert(id) {
        return Err(damaged(format!("the diary loops back to entry {id}")));
    }
    read(&id)
}

/// The entry of `step`, which entry `id` undoes or redoes.
fn read_step(id: Id, step: Id, read: &mut Read<'_>) -> Result<DiaryEntry, RepoError> {
    let entry = read(&step)?;
    match entry.action {
        Action::Command => Ok(entry),
        Action::Undo(_) | Action::Redo(_) => Err(damaged(format!(
            "diary entry {id} undoes or redoes {step}, which is not a step"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diary::{Position, State};
    use crate::id::hash_content;
    use crate::save::Person;
    use proptest::collection::vec;
    use proptest::prelude::*;
    use std::collections::{BTreeMap, HashMap};

    /// A diary in memory: any distinct IDs will do, and the states are left empty.
    #[derive(Default)]
    struct Diary {
        entries: HashMap<Id, DiaryEntry>,
        head: Option<Id>,
    }

    impl Diary {
        /// Adds an entry after the head and makes it the head.
        fn add(&mut self, action: Action) -> Id {
            let id = hash_content(&self.entries.len().to_le_bytes());
            let entry = DiaryEntry {
                parent: self.head,
                action,
                command: String::new(),
                who: Person {
                    name: String::new(),
                    email: String::new(),
                    time_ms: 0,
                    tz_offset_min: 0,
                },
                bolt_version: String::new(),
                state: State {
                    lines: BTreeMap::new(),
                    tags: None,
                    position: Position::AtSave(Id([0; 32])),
                    shelves: vec![],
                    server_lines: None,
                },
                folder_change: None,
            };
            self.entries.insert(id, entry);
            self.head = Some(id);
            id
        }

        fn read(&self) -> impl FnMut(&Id) -> Result<DiaryEntry, RepoError> + '_ {
            |id| {
                let entry = self.entries.get(id).cloned();
                entry.ok_or_else(|| damaged(format!("no entry {id}")))
            }
        }

        fn steps(&self, limit: usize) -> Result<Steps, RepoError> {
            steps(self.head.unwrap(), &mut self.read(), limit)
        }

        fn newest_done(&self) -> Result<Option<(Id, DiaryEntry)>, RepoError> {
            newest_done(self.head.unwrap(), &mut self.read())
        }

        fn done_after(&self, step: Id) -> Result<Option<usize>, RepoError> {
            done_after(self.head.unwrap(), step, &mut self.read())
        }

        /// `ids`, oldest first, as the lists hold them: newest first, each with its entry.
        fn listed(&self, ids: &[Id]) -> Vec<(Id, DiaryEntry)> {
            ids.iter()
                .rev()
                .map(|id| (*id, self.entries[id].clone()))
                .collect()
        }
    }

    fn assert_damaged(found: Result<impl std::fmt::Debug, RepoError>, entry: Id) {
        match found {
            Err(RepoError::Damaged { detail }) => {
                assert!(detail.contains(&entry.to_hex()), "{detail}");
            }
            other => panic!("expected damage naming {entry}, got {other:?}"),
        }
    }

    #[derive(Debug, Clone)]
    enum Op {
        Command,
        Undo,
        Redo,
    }

    fn op() -> impl Strategy<Value = Op> {
        prop_oneof![Just(Op::Command), Just(Op::Undo), Just(Op::Redo)]
    }

    proptest! {
        // The model undoes and redoes on two stacks; the diary only records what it did.
        #[test]
        fn the_lists_match_a_replay_of_the_diary(ops in vec(op(), 1..60)) {
            let mut diary = Diary::default();
            // Both oldest first: undo takes the last of `done`, redo the last of `undone`.
            let mut done = vec![diary.add(Action::Command)];
            let mut undone = vec![];
            for op in ops {
                match op {
                    Op::Command => {
                        done.push(diary.add(Action::Command));
                        undone.clear();
                    }
                    Op::Undo => {
                        if let Some(step) = done.pop() {
                            diary.add(Action::Undo(step));
                            undone.push(step);
                        }
                    }
                    Op::Redo => {
                        if let Some(step) = undone.pop() {
                            diary.add(Action::Redo(step));
                            done.push(step);
                        }
                    }
                }
                let all = diary.steps(1000).unwrap();
                prop_assert_eq!(&all.done, &diary.listed(&done));
                prop_assert!(!all.more_done);
                prop_assert_eq!(&all.undone, &diary.listed(&undone));
                prop_assert_eq!(diary.newest_done().unwrap(), all.done.first().cloned());
                let half = done.len() / 2;
                let some = diary.steps(half).unwrap();
                prop_assert_eq!(&some.done[..], &all.done[..half]);
                // More only if the next done step can be undone: the first entry can't.
                let next = all.done.get(half);
                prop_assert_eq!(some.more_done, next.is_some_and(|(_, e)| e.parent.is_some()));
                for (newer, (id, _)) in all.done.iter().enumerate() {
                    prop_assert_eq!(diary.done_after(*id).unwrap(), Some(newer));
                }
                for (id, _) in &all.undone {
                    prop_assert_eq!(diary.done_after(*id).unwrap(), None);
                }
            }
        }
    }

    #[test]
    fn the_first_entry_is_done_and_nothing_comes_before_it() {
        let mut diary = Diary::default();
        let first = diary.add(Action::Command);
        let steps = diary.steps(1).unwrap();
        assert_eq!(steps.done, diary.listed(&[first]));
        assert!(!steps.more_done);
        assert!(steps.undone.is_empty());
        assert_eq!(diary.newest_done().unwrap(), steps.done.first().cloned());

        // Past the first entry nothing is done.
        diary.add(Action::Undo(first));
        let steps = diary.steps(1).unwrap();
        assert!(steps.done.is_empty());
        assert!(!steps.more_done);
        assert_eq!(steps.undone, diary.listed(&[first]));
        assert_eq!(diary.newest_done().unwrap(), None);
    }

    #[test]
    fn a_target_that_is_not_a_command_is_damage() {
        for redo in [false, true] {
            let mut diary = Diary::default();
            let first = diary.add(Action::Command);
            let undo = diary.add(Action::Undo(first));
            let bad = diary.add(if redo {
                Action::Redo(undo)
            } else {
                Action::Undo(undo)
            });
            assert_damaged(diary.steps(1000), bad);
            assert_damaged(diary.newest_done(), bad);
        }
    }

    #[test]
    fn the_done_list_stops_at_the_limit_and_says_there_is_more() {
        let mut diary = Diary::default();
        let made: Vec<Id> = (0..5).map(|_| diary.add(Action::Command)).collect();
        diary.add(Action::Undo(made[4]));

        let steps = diary.steps(2).unwrap();
        assert_eq!(steps.done, diary.listed(&made[2..4]));
        assert!(steps.more_done);
        assert_eq!(steps.undone, diary.listed(&made[4..]));

        // Only the first entry is left past the limit, and undo can't take it.
        let steps = diary.steps(3).unwrap();
        assert_eq!(steps.done, diary.listed(&made[1..4]));
        assert!(!steps.more_done);

        let steps = diary.steps(4).unwrap();
        assert_eq!(steps.done, diary.listed(&made[..4]));
        assert!(!steps.more_done);

        let steps = diary.steps(0).unwrap();
        assert!(steps.done.is_empty());
        assert!(steps.more_done);
    }

    #[test]
    fn a_redo_must_redo_what_the_undo_it_answers_undid() {
        let mut diary = Diary::default();
        let first = diary.add(Action::Command);
        let second = diary.add(Action::Command);
        let undo = diary.add(Action::Undo(second));
        let redo = diary.add(Action::Redo(first));
        assert_damaged(diary.steps(1000), redo);
        assert_damaged(diary.steps(1000), undo);
    }

    #[test]
    fn a_redo_with_no_undo_before_it_is_damage() {
        let mut diary = Diary::default();
        let first = diary.add(Action::Command);
        let redo = diary.add(Action::Redo(first));
        assert_damaged(diary.steps(1000), redo);
    }

    #[test]
    fn a_diary_that_starts_without_a_command_is_damage() {
        let mut diary = Diary::default();
        let first = diary.add(Action::Command);
        let undo = diary.add(Action::Undo(first));
        diary.entries.get_mut(&undo).unwrap().parent = None;
        assert_damaged(diary.steps(1000), undo);
    }

    #[test]
    fn a_missing_entry_is_reported() {
        let mut diary = Diary::default();
        let first = diary.add(Action::Command);
        diary.add(Action::Undo(first));
        diary.entries.remove(&first);
        assert_damaged(diary.steps(1000), first);
        assert_damaged(diary.newest_done(), first);
    }

    #[test]
    fn a_diary_that_loops_is_damage() {
        // An undone step whose parent is the undo itself: the done steps would go round.
        let mut diary = Diary::default();
        let step = diary.add(Action::Command);
        let undo = diary.add(Action::Undo(step));
        diary.entries.get_mut(&step).unwrap().parent = Some(undo);
        assert_damaged(diary.newest_done(), undo);

        // An undo and a redo that are each other's parents: the undone steps would go round.
        let mut diary = Diary::default();
        let step = diary.add(Action::Command);
        let undo = diary.add(Action::Undo(step));
        let redo = diary.add(Action::Redo(step));
        diary.entries.get_mut(&undo).unwrap().parent = Some(redo);
        assert_damaged(diary.steps(1000), redo);
    }
}
