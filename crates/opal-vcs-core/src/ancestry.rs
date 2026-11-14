//! Which saves one line has in its history that others don't, following every parent.

use std::collections::hash_map::Entry;
use std::collections::{BinaryHeap, HashMap};

use crate::id::Id;
use crate::repo::{RepoError, Repository};

const TIP: u8 = 1;
const OTHERS: u8 = 2;
const BOTH: u8 = TIP | OTHERS;

struct Node {
    marks: u8,
    /// The marks its parents were last given; 0 before that.
    spread: u8,
    time: i64,
    parents: Vec<Id>,
}

impl Node {
    /// A shared save never spread can wait: everything under it is shared too.
    fn needs_look(&self) -> bool {
        self.marks != self.spread && !(self.marks == BOTH && self.spread == 0)
    }
}

struct Walk<'r> {
    repo: &'r Repository,
    nodes: HashMap<Id, Node>,
    /// Newest first; may hold stale entries, skipped when popped.
    heap: BinaryHeap<(i64, Id)>,
    /// How many nodes need looking at.
    waiting: usize,
}

impl Walk<'_> {
    /// Adds `marks` to a save, reading it the first time it is met.
    fn mark(&mut self, id: Id, marks: u8) -> Result<(), RepoError> {
        let node = match self.nodes.entry(id) {
            Entry::Occupied(e) => e.into_mut(),
            Entry::Vacant(e) => {
                let save = self
                    .repo
                    .store()
                    .save(&id, self.repo.signatures_allowed())?;
                e.insert(Node {
                    marks: 0,
                    spread: 0,
                    time: save.saver.time_ms,
                    parents: save.parents,
                })
            }
        };
        if node.marks | marks == node.marks {
            return Ok(());
        }
        let before = node.needs_look();
        node.marks |= marks;
        self.waiting = self.waiting - usize::from(before) + usize::from(node.needs_look());
        self.heap.push((node.time, id));
        Ok(())
    }
}

/// How many saves `tip` has in its history, itself included, that no save of `others` has in
/// its own. 0 exactly when one of `others` is `tip` or has `tip` in its history.
pub fn only_in(repo: &Repository, tip: Id, others: &[Id]) -> Result<u64, RepoError> {
    let mut walk = Walk {
        repo,
        nodes: HashMap::new(),
        heap: BinaryHeap::new(),
        waiting: 0,
    };
    walk.mark(tip, TIP)?;
    for &other in others {
        walk.mark(other, OTHERS)?;
    }
    while walk.waiting > 0 {
        let Some((_, id)) = walk.heap.pop() else {
            break;
        };
        let Some(node) = walk.nodes.get_mut(&id) else {
            continue;
        };
        if node.marks == node.spread {
            continue;
        }
        // A mark that arrives late spreads again, to correct what the first one reached.
        let before = node.needs_look();
        node.spread = node.marks;
        walk.waiting -= usize::from(before);
        let (marks, parents) = (node.marks, node.parents.clone());
        for parent in parents {
            walk.mark(parent, marks)?;
        }
    }
    Ok(walk
        .nodes
        .values()
        .filter(|n| n.marks == TIP)
        .map(|_| 1)
        .sum())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;
    use crate::diary::{Action, DiaryEntry};
    use crate::pack::ObjectKind;
    use crate::save::{Person, Save};
    use proptest::prelude::*;
    use std::collections::{BTreeSet, HashSet};

    fn project() -> (tempfile::TempDir, Repository) {
        let dir = tempfile::tempdir().unwrap();
        let who = Person {
            name: "Ada".into(),
            email: "a@example.com".into(),
            time_ms: 0,
            tz_offset_min: 0,
        };
        let repo = Repository::init(dir.path(), &who).unwrap();
        (dir, repo)
    }

    /// Stores one save per `(parents, time)`, its parents given as indexes of earlier ones, all
    /// in one command. Returns their IDs in order.
    fn plant(repo: &mut Repository, saves: &[(Vec<usize>, i64)]) -> Vec<Id> {
        let lock = repo.lock_for_change("save", &mut |_| {}).unwrap();
        let (head, entry) = repo.current().unwrap();
        let start = repo
            .store()
            .save(&entry.state.lines["main"], false)
            .unwrap();
        let mut packs = repo.new_packs().unwrap();
        let mut ids: Vec<Id> = Vec::new();
        for (n, (parents, time)) in saves.iter().enumerate() {
            let save = Save {
                parents: parents.iter().map(|&p| ids[p]).collect(),
                saver: Person {
                    time_ms: *time,
                    ..start.saver.clone()
                },
                message: n.to_string(),
                ..start.clone()
            };
            ids.push(packs.add_record(ObjectKind::Save, &save.encode()).unwrap());
        }
        let diary = DiaryEntry {
            parent: Some(head),
            action: Action::Command,
            command: "save".into(),
            who: start.saver.clone(),
            opal_version: "0.1.0".into(),
            state: entry.state.clone(),
            folder_change: None,
        };
        repo.commit(&lock, packs, &diary).unwrap();
        ids
    }

    /// A chain of `len` saves after `from`, each a minute later; the times start after `t`.
    fn chain_after(saves: &mut Vec<(Vec<usize>, i64)>, from: Option<usize>, len: usize, t: i64) {
        let mut parent = from;
        for k in 0..len {
            saves.push((parent.into_iter().collect(), t + 60_000 * (k as i64 + 1)));
            parent = Some(saves.len() - 1);
        }
    }

    #[test]
    fn a_line_another_holds_has_nothing_of_its_own() {
        let (_dir, mut repo) = project();
        let mut saves = Vec::new();
        chain_after(&mut saves, None, 3, 0);
        let [_a, b, c] = plant(&mut repo, &saves)[..] else {
            panic!()
        };
        assert_eq!(only_in(&repo, b, &[c]).unwrap(), 0);
        assert_eq!(only_in(&repo, c, &[b]).unwrap(), 1);
        assert_eq!(only_in(&repo, c, &[]).unwrap(), 3);
        assert_eq!(only_in(&repo, c, &[c]).unwrap(), 0);
    }

    #[test]
    fn a_mark_that_arrives_late_spreads_again() {
        // The held save is older than the tip, so the tip spreads before the other mark reaches it.
        let (_dir, mut repo) = project();
        let ids = plant(
            &mut repo,
            &[(vec![], 1000), (vec![0], 3000), (vec![1], 2000)],
        );
        assert_eq!(only_in(&repo, ids[1], &[ids[2]]).unwrap(), 0);
    }

    #[test]
    fn a_save_reached_only_through_a_second_parent_is_held() {
        let (_dir, mut repo) = project();
        let ids = plant(
            &mut repo,
            &[(vec![], 1000), (vec![], 2000), (vec![0, 1], 3000)],
        );
        let (y, m) = (ids[1], ids[2]);
        assert_eq!(only_in(&repo, y, &[m]).unwrap(), 0);
    }

    #[test]
    fn both_sides_of_a_fork_are_counted_apart() {
        let (_dir, mut repo) = project();
        // base, a1, a2, b1
        let ids = plant(
            &mut repo,
            &[
                (vec![], 1000),
                (vec![0], 2000),
                (vec![1], 3000),
                (vec![0], 2500),
            ],
        );
        let (a2, b1) = (ids[2], ids[3]);
        assert_eq!(only_in(&repo, a2, &[b1]).unwrap(), 2);
        assert_eq!(only_in(&repo, b1, &[a2]).unwrap(), 1);
    }

    #[test]
    fn shared_history_is_not_read() {
        let (_dir, mut repo) = project();
        let mut saves = Vec::new();
        chain_after(&mut saves, None, 200, 0);
        let fork = saves.len() - 1;
        let t = saves[fork].1;
        chain_after(&mut saves, Some(fork), 3, t);
        let side_a = saves.len() - 1;
        chain_after(&mut saves, Some(fork), 2, t + 30_000);
        let side_b = saves.len() - 1;
        let ids = plant(&mut repo, &saves);
        let before = repo.store().saves_read();
        assert_eq!(only_in(&repo, ids[side_a], &[ids[side_b]]).unwrap(), 3);
        assert!(repo.store().saves_read() - before <= 7);
    }

    /// Every save reachable from `starts`, themselves included.
    fn ancestors(parents: &[Vec<usize>], starts: &[usize]) -> HashSet<usize> {
        let mut seen = HashSet::new();
        let mut todo = starts.to_vec();
        while let Some(n) = todo.pop() {
            if seen.insert(n) {
                todo.extend(&parents[n]);
            }
        }
        seen
    }

    /// How saves are dated against their parents.
    #[derive(Clone, Copy, Debug, PartialEq)]
    enum Clock {
        /// Each save later than its parents.
        Forwards,
        Random,
        /// Each save earlier than its parents: every save is looked at before its children.
        Backwards,
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(256))]
        #[test]
        fn only_in_agrees_with_a_full_walk(
            shape in prop::collection::vec(
                (prop::collection::vec(any::<prop::sample::Index>(), 0..=3), 0..1000i64),
                1..=40,
            ),
            tip in any::<prop::sample::Index>(),
            others in prop::collection::vec(any::<prop::sample::Index>(), 0..=3),
            clock in prop_oneof![Just(Clock::Forwards), Just(Clock::Random), Just(Clock::Backwards)],
        ) {
            let parents: Vec<Vec<usize>> = shape
                .iter()
                .enumerate()
                .map(|(n, (picks, _))| {
                    if n == 0 {
                        return Vec::new();
                    }
                    let set: BTreeSet<usize> = picks.iter().map(|p| p.index(n)).collect();
                    set.into_iter().collect()
                })
                .collect();
            let saves: Vec<(Vec<usize>, i64)> = parents
                .iter()
                .zip(&shape)
                .enumerate()
                .map(|(n, (p, (_, t)))| {
                    let time = match clock {
                        Clock::Forwards => n as i64,
                        Clock::Random => *t,
                        Clock::Backwards => -(n as i64),
                    };
                    (p.clone(), time)
                })
                .collect();
            let tip = tip.index(saves.len());
            let others: Vec<usize> = others.iter().map(|o| o.index(saves.len())).collect();

            let mine = ancestors(&parents, &[tip]);
            let theirs = ancestors(&parents, &others);
            let oracle = mine.difference(&theirs).count() as u64;

            let (_dir, mut repo) = project();
            let ids = plant(&mut repo, &saves);
            let other_ids: Vec<Id> = others.iter().map(|&o| ids[o]).collect();
            let got = only_in(&repo, ids[tip], &other_ids).unwrap();
            if clock == Clock::Forwards {
                prop_assert_eq!(got, oracle);
            } else {
                prop_assert_eq!(got == 0, oracle == 0);
                prop_assert!(got >= oracle);
            }
        }
    }
}
