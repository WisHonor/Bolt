//! Which saves one line has in its history that others don't, following every parent.

use std::collections::hash_map::Entry;
use std::collections::{BTreeMap, BinaryHeap, HashMap};

use crate::id::Id;
use crate::repo::{RepoError, Repository};

const TIP: u8 = 1;
const OTHERS: u8 = 2;

struct Node {
    marks: u8,
    /// The marks it last gave its parents; 0 before that.
    spread: u8,
    time: i64,
    parents: Vec<Id>,
}

impl Node {
    /// Has marks its parents haven't been given yet.
    fn waiting(&self) -> bool {
        self.marks != self.spread
    }

    fn one_side_waiting(&self) -> bool {
        self.waiting() && (self.marks == TIP || self.marks == OTHERS)
    }

    /// Gained a mark after spreading, so what it gave before needs correcting.
    fn correcting(&self) -> bool {
        self.waiting() && self.spread != 0
    }
}

/// What decides whether the walk must go on, kept up to date as nodes change.
#[derive(Default)]
struct Tally {
    one_side_waiting: usize,
    correcting: usize,
    /// The times of the saves counted so far (marked by the tip alone), with how many share each.
    counted: BTreeMap<i64, usize>,
}

impl Tally {
    fn add(&mut self, node: &Node) {
        self.one_side_waiting += usize::from(node.one_side_waiting());
        self.correcting += usize::from(node.correcting());
        if node.marks == TIP {
            *self.counted.entry(node.time).or_default() += 1;
        }
    }

    fn remove(&mut self, node: &Node) {
        self.one_side_waiting -= usize::from(node.one_side_waiting());
        self.correcting -= usize::from(node.correcting());
        if node.marks == TIP
            && let Some(n) = self.counted.get_mut(&node.time)
        {
            *n -= 1;
            if *n == 0 {
                self.counted.remove(&node.time);
            }
        }
    }

    /// Whether a waiting save of this time must still spread. A shared save older than every
    /// counted one can't sit above any of them when no save is older than its parent.
    fn must_go_on(&self, time: i64) -> bool {
        self.one_side_waiting > 0
            || self.correcting > 0
            || self
                .counted
                .first_key_value()
                .is_some_and(|(&t, _)| time >= t)
    }
}

struct Walk<'r> {
    repo: &'r Repository,
    tip: Id,
    nodes: HashMap<Id, Node>,
    /// Newest first: every waiting node, and stale entries skipped when met.
    heap: BinaryHeap<(i64, Id)>,
    tally: Tally,
}

impl Walk<'_> {
    /// Adds `marks` to a save, reading it the first time it is met. True once the tip is held.
    fn mark(&mut self, id: Id, marks: u8) -> Result<bool, RepoError> {
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
            return Ok(false);
        }
        self.tally.remove(node);
        node.marks |= marks;
        self.tally.add(node);
        self.heap.push((node.time, id));
        Ok(id == self.tip && node.marks & OTHERS != 0)
    }
}

/// How many saves `tip` has in its history, itself included, that no save of `others` has in
/// its own. 0 exactly when one of `others` is `tip` or has `tip` in its history.
pub fn only_in(repo: &Repository, tip: Id, others: &[Id]) -> Result<u64, RepoError> {
    let mut walk = Walk {
        repo,
        tip,
        nodes: HashMap::new(),
        heap: BinaryHeap::new(),
        tally: Tally::default(),
    };
    walk.mark(tip, TIP)?;
    for &other in others {
        if walk.mark(other, OTHERS)? {
            return Ok(0);
        }
    }
    while let Some(&(time, id)) = walk.heap.peek() {
        let Some(node) = walk.nodes.get_mut(&id).filter(|n| n.waiting()) else {
            walk.heap.pop();
            continue;
        };
        if !walk.tally.must_go_on(time) {
            break;
        }
        walk.heap.pop();
        // A mark gained after spreading is passed on too.
        walk.tally.remove(node);
        node.spread = node.marks;
        walk.tally.add(node);
        let (marks, parents) = (node.marks, node.parents.clone());
        for parent in parents {
            if walk.mark(parent, marks)? {
                return Ok(0);
            }
        }
    }
    Ok(walk.nodes.values().filter(|n| n.marks == TIP).count() as u64)
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
    fn a_held_tip_dated_after_its_holder_is_still_held() {
        // The holder is older than the tip, so the tip spreads before the holder's mark reaches it.
        let (_dir, mut repo) = project();
        let ids = plant(
            &mut repo,
            &[(vec![], 1000), (vec![0], 3000), (vec![1], 2000)],
        );
        assert_eq!(only_in(&repo, ids[1], &[ids[2]]).unwrap(), 0);
    }

    #[test]
    fn a_history_dated_backwards_can_count_high_never_low() {
        let (_dir, mut repo) = project();
        // X, then S dated before it, then A merging S and X, and B after S.
        let [_x, _s, a, b] = plant(
            &mut repo,
            &[
                (vec![], 50),
                (vec![0], 10),
                (vec![1, 0], 100),
                (vec![1], 90),
            ],
        )[..] else {
            panic!()
        };
        // The answer is 1 (A alone), but newest first X is read before S, which would have carried
        // B's mark to it, so X is counted as A's too.
        assert_eq!(only_in(&repo, a, &[b]).unwrap(), 2);
    }

    /// `k` saves of their own first, so that each history's IDs, and so its tie order, differ.
    fn filler(k: usize) -> Vec<(Vec<usize>, i64)> {
        (0..k).map(|_| (vec![], 0)).collect()
    }

    #[test]
    fn saves_dated_alike_are_still_counted_right() {
        // Q <- P <- X and P <- Y, all at once; X has itself alone.
        let mut orders = HashSet::new();
        for k in 0..16 {
            let (_dir, mut repo) = project();
            let mut saves = filler(k);
            let q = saves.len();
            let (p, x, y) = (q + 1, q + 2, q + 3);
            saves.extend([(vec![], 5), (vec![q], 5), (vec![p], 5), (vec![p], 5)]);
            let ids = plant(&mut repo, &saves);
            assert_eq!(only_in(&repo, ids[x], &[ids[y]]).unwrap(), 1, "history {k}");
            orders.insert(ids[p] > ids[y]);
        }
        assert_eq!(orders.len(), 2, "both tie orders");
    }

    #[test]
    fn a_shared_save_dated_like_a_counted_one_still_passes_its_marks_on() {
        // A <- B, both at once, and C after them with parents [A, B]. B is held, so only C
        // counts; B must still tell A, though nothing waits with one mark when A has spread.
        let mut orders = HashSet::new();
        for k in 0..16 {
            let (_dir, mut repo) = project();
            let mut saves = filler(k);
            let a = saves.len();
            let (b, c) = (a + 1, a + 2);
            saves.extend([(vec![], 5), (vec![a], 5), (vec![a, b], 6)]);
            let ids = plant(&mut repo, &saves);
            assert_eq!(only_in(&repo, ids[c], &[ids[b]]).unwrap(), 1, "history {k}");
            orders.insert(ids[a] > ids[b]);
        }
        assert_eq!(orders.len(), 2, "both tie orders");
    }

    #[test]
    fn a_tie_where_two_lines_parted_does_not_run_to_the_start() {
        let mut orders = HashSet::new();
        for k in 0..16 {
            let (_dir, mut repo) = project();
            let mut saves = filler(k);
            let first = saves.len();
            chain_after(&mut saves, None, 300, 0);
            let parted = first + 250;
            let from = saves[parted].1;
            // The other side starts at the very time they parted.
            let mut other = parted;
            let other_first = saves.len();
            for later in [0, 20_000, 40_000] {
                saves.push((vec![other], from + later));
                other = saves.len() - 1;
            }
            saves.push((vec![parted], from + 30_000));
            let tip = saves.len() - 1;
            let ids = plant(&mut repo, &saves);
            let before = repo.store().saves_read();
            assert_eq!(
                only_in(&repo, ids[tip], &[ids[other]]).unwrap(),
                1,
                "history {k}"
            );
            let read = repo.store().saves_read() - before;
            assert!(read <= 12, "history {k}: {read} saves read");
            orders.insert(ids[parted] > ids[other_first]);
        }
        assert_eq!(orders.len(), 2, "both tie orders");
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

    /// Main save `k`'s time.
    fn at(k: usize) -> i64 {
        60_000 * (k as i64 + 1)
    }

    /// Three saves after `from`, dated within a minute of it. Returns the newest.
    fn feature_after(saves: &mut Vec<(Vec<usize>, i64)>, from: usize) -> usize {
        let mut parent = from;
        for j in 1..=3 {
            saves.push((vec![parent], saves[from].1 + 10_000 * j));
            parent = saves.len() - 1;
        }
        parent
    }

    #[test]
    fn a_line_combined_into_another_is_answered_at_once() {
        let (_dir, mut repo) = project();
        let mut saves = Vec::new();
        chain_after(&mut saves, None, 1000, 0);
        let feature = feature_after(&mut saves, 990);
        saves.push((vec![999, feature], at(1000)));
        let main = saves.len() - 1;
        let ids = plant(&mut repo, &saves);
        let before = repo.store().saves_read();
        assert_eq!(only_in(&repo, ids[feature], &[ids[main]]).unwrap(), 0);
        let read = repo.store().saves_read() - before;
        assert!(read <= 6, "{read} saves read");
    }

    #[test]
    fn an_older_side_line_does_not_send_the_walk_to_the_start() {
        let (_dir, mut repo) = project();
        let mut saves: Vec<(Vec<usize>, i64)> = Vec::new();
        let mut main: Vec<usize> = Vec::new();
        for k in 0..1000 {
            let mut parents: Vec<usize> = main.last().copied().into_iter().collect();
            if k == 951 {
                // A side line from save 900, dated before save 950, combined here.
                let mut side = main[900];
                for j in 1..=3 {
                    saves.push((vec![side], at(900) + 600_000 * j));
                    side = saves.len() - 1;
                }
                parents.push(side);
            }
            saves.push((parents, at(k)));
            main.push(saves.len() - 1);
        }
        let feature = feature_after(&mut saves, main[990]);
        let ids = plant(&mut repo, &saves);
        let before = repo.store().saves_read();
        assert_eq!(only_in(&repo, ids[feature], &[ids[main[999]]]).unwrap(), 3);
        let read = repo.store().saves_read() - before;
        assert!(read <= 140, "{read} saves read");
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
        /// Each save as late as its newest parent, often to the millisecond.
        Ties,
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
            clock in prop_oneof![
                Just(Clock::Forwards),
                Just(Clock::Random),
                Just(Clock::Backwards),
                Just(Clock::Ties),
            ],
            // Random times of a chain of new saves after the tip, its last one held by the others.
            under in prop::option::of(prop::collection::vec(0..1000i64, 1..=3)),
        ) {
            // The time of the next save after `saves`, with these parents.
            let dated = |saves: &[(Vec<usize>, i64)], parents: &[usize], t: i64| match clock {
                Clock::Forwards => saves.len() as i64,
                Clock::Random => t,
                Clock::Backwards => -(saves.len() as i64),
                Clock::Ties => {
                    let newest = parents.iter().map(|&p| saves[p].1).max().unwrap_or(0);
                    newest + i64::from(t % 4 == 0)
                }
            };
            let mut parents: Vec<Vec<usize>> = shape
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
            let mut saves: Vec<(Vec<usize>, i64)> = Vec::new();
            for (p, (_, t)) in parents.iter().zip(&shape) {
                let time = dated(&saves, p, *t);
                saves.push((p.clone(), time));
            }
            let tip = tip.index(saves.len());
            let mut others: Vec<usize> = others.iter().map(|o| o.index(saves.len())).collect();
            if let Some(times) = under {
                let mut parent = tip;
                for t in times {
                    parents.push(vec![parent]);
                    let time = dated(&saves, &[parent], t);
                    saves.push((vec![parent], time));
                    parent = saves.len() - 1;
                }
                others.push(parent);
            }

            let mine = ancestors(&parents, &[tip]);
            let theirs = ancestors(&parents, &others);
            let oracle = mine.difference(&theirs).count() as u64;

            let (_dir, mut repo) = project();
            let ids = plant(&mut repo, &saves);
            let other_ids: Vec<Id> = others.iter().map(|&o| ids[o]).collect();
            let got = only_in(&repo, ids[tip], &other_ids).unwrap();
            if matches!(clock, Clock::Forwards | Clock::Ties) {
                prop_assert_eq!(got, oracle);
            } else {
                prop_assert_eq!(got == 0, oracle == 0);
                prop_assert!(got >= oracle);
            }
        }
    }
}
