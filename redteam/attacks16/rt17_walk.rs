#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::HashSet;

use opal_core::ancestry::only_in;
use opal_core::diary::{Action, DiaryEntry};
use opal_core::id::Id;
use opal_core::pack::ObjectKind;
use opal_core::repo::Repository;
use opal_core::save::{Person, Save};

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

fn plant(repo: &mut Repository, saves: &[(Vec<usize>, i64)]) -> Vec<Id> {
    let lock = repo.lock_for_change("save", &mut |_| {}).unwrap();
    let (head, entry) = repo.current().unwrap();
    let start = repo.store().save(&entry.state.lines["main"], false).unwrap();
    let mut packs = repo.new_packs().unwrap();
    let mut ids: Vec<Id> = Vec::new();
    for (n, (parents, time)) in saves.iter().enumerate() {
        let save = Save {
            parents: parents.iter().map(|&p| ids[p]).collect(),
            saver: Person { time_ms: *time, ..start.saver.clone() },
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

fn ancestors(saves: &[(Vec<usize>, i64)], from: usize) -> HashSet<usize> {
    let mut seen = HashSet::new();
    let mut stack = vec![from];
    while let Some(n) = stack.pop() {
        if seen.insert(n) {
            stack.extend(saves[n].0.iter().copied());
        }
    }
    seen
}

#[test]
fn walk_against_brute_force() {
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    let who = Person { name: "A".into(), email: "a@x".into(), time_ms: 0, tz_offset_min: 0 };
    let mut bad = 0;
    for case in 0..3000 {
        let dir = tempfile::tempdir().unwrap();
        let mut repo = Repository::init(dir.path(), &who).unwrap();
        let n = 2 + rng.below(40) as usize;
        let clock = rng.below(4);
        let mut saves: Vec<(Vec<usize>, i64)> = Vec::new();
        for k in 0..n {
            let mut parents = Vec::new();
            if k > 0 {
                let np = match rng.below(10) { 0 => 0, 1..=6 => 1, 7 | 8 => 2, _ => 3 };
                for _ in 0..np {
                    let p = rng.below(k as u64) as usize;
                    if !parents.contains(&p) { parents.push(p); }
                }
            }
            let time = match clock {
                0 => k as i64 * 10,
                1 => rng.below(5) as i64,
                2 => rng.below(1000) as i64,
                _ => [i64::MIN, i64::MAX, 0, -1, 1][rng.below(5) as usize],
            };
            saves.push((parents, time));
        }
        let ids = plant(&mut repo, &saves);
        for _ in 0..5 {
            let tip = rng.below(n as u64) as usize;
            let others: Vec<usize> = (0..rng.below(4)).map(|_| rng.below(n as u64) as usize).collect();
            let mine = ancestors(&saves, tip);
            let held = others.iter().any(|&o| ancestors(&saves, o).contains(&tip));
            let theirs: HashSet<usize> = others.iter().flat_map(|&o| ancestors(&saves, o)).collect();
            let exact = mine.difference(&theirs).count() as u64;
            let got = only_in(&repo, ids[tip], &others.iter().map(|&o| ids[o]).collect::<Vec<_>>()).unwrap();
            let ok = if held { got == 0 } else if clock == 0 { got == exact } else { got >= exact && got > 0 };
            if !ok {
                bad += 1;
                println!("case {case} clock {clock} tip {tip} others {others:?} held {held} exact {exact} got {got} saves {saves:?}");
            }
        }
    }
    assert_eq!(bad, 0);
}
