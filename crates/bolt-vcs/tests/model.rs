//! Random edits, saves, switches and discards, applied both to a project and to a model of what
//! its folder and saves should hold (testing.md §3).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use bolt_core::diary::Position;
use bolt_core::diff::ChangeKind;
use bolt_core::id::Id;
use bolt_core::lines::Target;
use bolt_core::listing::EntryKind;
use bolt_core::repo::Repository;
use bolt_core::save::Person;
use bolt_core::snapshot::{SaveOutcome, SaveRequest, save_folder};
use bolt_core::status::status;
use bolt_core::store::Store;
use bolt_core::switch::{SwitchError, discard, goto};
use proptest::prelude::*;

// Names differ in more than letter case, so every drive holds them all. No file name is ever a
// folder's.
const FILES: [&str; 6] = ["a", "b", "d/a", "d/c", "d/e/b", "e/a"];
const FOLDERS: [&str; 3] = ["d", "d/e", "e"];

#[derive(Debug, Clone)]
enum Act {
    /// Writes the file whole; the byte varies what it holds.
    Write(&'static str, u8),
    Delete(&'static str),
    Rename(&'static str, &'static str),
    MakeFolder(&'static str),
    DeleteFolder(&'static str),
    Save,
    /// Goes to the save or line `pick` chooses, first saving or discarding unsaved work.
    Goto {
        pick: usize,
        by_line: bool,
        save_first: bool,
    },
    /// Discards everything, or what is at one path.
    Discard(Option<&'static str>),
    /// Status reading every file, not trusting the file cache.
    FullStatus,
}

fn act() -> impl Strategy<Value = Act> {
    let file = || prop::sample::select(&FILES[..]);
    let folder = || prop::sample::select(&FOLDERS[..]);
    let any_path = prop::sample::select([&FILES[..], &FOLDERS[..]].concat());
    prop_oneof![
        6 => (file(), any::<u8>()).prop_map(|(p, b)| Act::Write(p, b)),
        2 => file().prop_map(Act::Delete),
        2 => (file(), file()).prop_map(|(a, b)| Act::Rename(a, b)),
        1 => folder().prop_map(Act::MakeFolder),
        1 => folder().prop_map(Act::DeleteFolder),
        3 => Just(Act::Save),
        3 => (any::<usize>(), any::<bool>(), any::<bool>())
            .prop_map(|(pick, by_line, save_first)| Act::Goto { pick, by_line, save_first }),
        1 => Just(Act::Discard(None)),
        1 => any_path.prop_map(|p| Act::Discard(Some(p))),
        1 => Just(Act::FullStatus),
    ]
}

/// Files by path, and every folder, a file's included.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Tree {
    files: BTreeMap<String, Vec<u8>>,
    folders: BTreeSet<String>,
}

impl Tree {
    fn put(&mut self, path: &str, bytes: Vec<u8>) {
        self.add_folders_above(path);
        self.files.insert(path.to_owned(), bytes);
    }

    fn add_folders_above(&mut self, path: &str) {
        let mut at = path;
        while let Some((up, _)) = at.rsplit_once('/') {
            self.folders.insert(up.to_owned());
            at = up;
        }
    }

    /// Drops `path` and everything in it.
    fn remove(&mut self, path: &str) {
        self.files.retain(|p, _| !within(p, path));
        self.folders.retain(|p| !within(p, path));
    }
}

fn within(p: &str, top: &str) -> bool {
    p == top
        || p.strip_prefix(top)
            .is_some_and(|rest| rest.starts_with('/'))
}

struct Model {
    folder: Tree,
    saves: Vec<(Id, Tree)>,
    lines: BTreeMap<String, usize>,
    /// The save the folder is on, and its line if it is on one.
    on: usize,
    line: Option<String>,
    /// Every write gets a size no earlier write had, so the same size always means the same
    /// bytes and no edit can hide from the file cache behind an unchanged size and time.
    writes: usize,
    new_lines: usize,
}

fn who() -> Person {
    Person {
        name: "Test".into(),
        email: "test@example.com".into(),
        time_ms: 1_700_000_000_000,
        tz_offset_min: 0,
    }
}

fn run(acts: &[Act]) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let who = who();
    let mut repo = Repository::init(root, &who).unwrap();
    let first = repo.current().unwrap().1.state.lines["main"];
    let mut m = Model {
        folder: Tree::default(),
        saves: vec![(first, Tree::default())],
        lines: BTreeMap::from([("main".to_owned(), 0)]),
        on: 0,
        line: Some("main".to_owned()),
        writes: 0,
        new_lines: 0,
    };
    for (n, a) in acts.iter().enumerate() {
        apply(&mut repo, &mut m, root, a);
        check(&repo, &m, root, matches!(a, Act::FullStatus))
            .unwrap_or_else(|e| panic!("after action {n}, {a:?}: {e}"));
    }
    for (id, tree) in &m.saves {
        assert_eq!(&saved_tree(repo.store(), *id), tree, "save {id:?}");
    }
}

fn apply(repo: &mut Repository, m: &mut Model, root: &Path, a: &Act) {
    let who = who();
    match *a {
        Act::Write(path, b) => {
            m.writes += 1;
            let bytes: Vec<u8> = (0..m.writes).map(|i| b.wrapping_add(i as u8)).collect();
            let at = root.join(path);
            std::fs::create_dir_all(at.parent().unwrap()).unwrap();
            std::fs::write(&at, &bytes).unwrap();
            m.folder.put(path, bytes);
        }
        Act::Delete(path) => {
            if m.folder.files.remove(path).is_some() {
                std::fs::remove_file(root.join(path)).unwrap();
            }
        }
        Act::Rename(from, to) => {
            if from != to
                && let Some(bytes) = m.folder.files.remove(from)
            {
                let at = root.join(to);
                std::fs::create_dir_all(at.parent().unwrap()).unwrap();
                std::fs::rename(root.join(from), at).unwrap();
                m.folder.put(to, bytes);
            }
        }
        Act::MakeFolder(path) => {
            std::fs::create_dir_all(root.join(path)).unwrap();
            m.folder.folders.insert(path.to_owned());
            m.folder.add_folders_above(path);
        }
        Act::DeleteFolder(path) => {
            if m.folder.folders.contains(path) {
                std::fs::remove_dir_all(root.join(path)).unwrap();
                m.folder.remove(path);
            }
        }
        Act::Save => save(repo, m),
        Act::Goto {
            pick,
            by_line,
            save_first,
        } => {
            if m.folder != m.saves[m.on].1 {
                if save_first {
                    save(repo, m);
                } else {
                    discard(repo, None, &who, "discard", None, &mut |_| {}).unwrap();
                    m.folder = m.saves[m.on].1.clone();
                }
            }
            let (target, to, line) = if by_line {
                let (name, &to) = m.lines.iter().nth(pick % m.lines.len()).unwrap();
                (Target::Line(name.clone()), to, Some(name.clone()))
            } else {
                let to = pick % m.saves.len();
                (Target::Save(m.saves[to].0), to, None)
            };
            let done = goto(repo, &target, &who, "goto", None, &mut |_| {}).unwrap();
            assert_eq!(done.carried, 0);
            m.folder = m.saves[to].1.clone();
            m.on = to;
            m.line = line;
        }
        Act::Discard(None) => {
            discard(repo, None, &who, "discard", None, &mut |_| {}).unwrap();
            m.folder = m.saves[m.on].1.clone();
        }
        Act::Discard(Some(path)) => {
            let saved = &m.saves[m.on].1;
            let at = |t: &Tree| {
                t.files.keys().any(|p| within(p, path)) || t.folders.iter().any(|p| within(p, path))
            };
            let paths = [path.to_owned()];
            let done = discard(repo, Some(&paths), &who, "discard", None, &mut |_| {});
            if !at(saved) && !at(&m.folder) {
                assert!(matches!(done, Err(SwitchError::NoMatch { .. })), "{done:?}");
                return;
            }
            done.unwrap();
            m.folder.remove(path);
            for (p, bytes) in saved.files.iter().filter(|(p, _)| within(p, path)) {
                m.folder.put(p, bytes.clone());
            }
            for p in saved.folders.iter().filter(|p| within(p, path)) {
                m.folder.folders.insert(p.clone());
                m.folder.add_folders_above(p);
            }
        }
        Act::FullStatus => {}
    }
}

fn save(repo: &mut Repository, m: &mut Model) {
    let who = who();
    // A save made at a save starts a line.
    let new_line = m.line.is_none().then(|| format!("line-{}", m.new_lines));
    let req = SaveRequest {
        message: "Save",
        who: &who,
        command: "save",
        accept_crlf: false,
        git_excludes: None,
        new_line: new_line.as_deref(),
        only: None,
    };
    match save_folder(repo, &req, &mut |_| {}, &mut |_| true).unwrap() {
        SaveOutcome::Saved { save_id, .. } => {
            assert_ne!(m.folder, m.saves[m.on].1, "saved with nothing changed");
            assert_eq!(saved_tree(repo.store(), save_id), m.folder);
            m.saves.push((save_id, m.folder.clone()));
            m.on = m.saves.len() - 1;
            if let Some(name) = new_line {
                m.new_lines += 1;
                m.line = Some(name);
            }
            m.lines.insert(m.line.clone().unwrap(), m.on);
        }
        SaveOutcome::NothingChanged { .. } => {
            assert_eq!(m.folder, m.saves[m.on].1, "changes not saved");
        }
    }
}

/// What the folder, `status`, the lines and the position say, against the model.
fn check(repo: &Repository, m: &Model, root: &Path, full: bool) -> Result<(), String> {
    let on_disk = folder_tree(root);
    if on_disk != m.folder {
        return Err(format!("the folder holds {on_disk:?}, not {:?}", m.folder));
    }
    let saved = &m.saves[m.on].1;
    let mut want = Vec::new();
    for (p, bytes) in &m.folder.files {
        match saved.files.get(p) {
            None => want.push((p.clone(), ChangeKind::Added)),
            Some(old) if old != bytes => want.push((p.clone(), ChangeKind::Modified)),
            Some(_) => {}
        }
    }
    for p in saved
        .files
        .keys()
        .filter(|p| !m.folder.files.contains_key(*p))
    {
        want.push((p.clone(), ChangeKind::Removed));
    }
    want.sort_by(|a, b| a.0.cmp(&b.0));
    let st = status(repo, full, None).map_err(|e| e.to_string())?;
    let got: Vec<_> = st
        .changes
        .iter()
        .map(|c| (c.path.clone(), c.change))
        .collect();
    if got != want {
        return Err(format!("status says {got:?}, not {want:?}"));
    }
    if st.save_id != m.saves[m.on].0 {
        return Err("status compared with the wrong save".into());
    }
    let (_, entry) = repo.current().map_err(|e| e.to_string())?;
    let lines: BTreeMap<String, Id> = m
        .lines
        .iter()
        .map(|(name, &i)| (name.clone(), m.saves[i].0))
        .collect();
    if entry.state.lines != lines {
        return Err(format!("lines are {:?}, not {lines:?}", entry.state.lines));
    }
    let position = match &m.line {
        Some(line) => Position::OnLine(line.clone()),
        None => Position::AtSave(m.saves[m.on].0),
    };
    if entry.state.position != position {
        return Err(format!("at {:?}, not {position:?}", entry.state.position));
    }
    Ok(())
}

/// What the project folder holds, `.bolt` left out.
fn folder_tree(root: &Path) -> Tree {
    let mut tree = Tree::default();
    let mut todo = vec![(root.to_owned(), String::new())];
    while let Some((dir, prefix)) = todo.pop() {
        for e in std::fs::read_dir(&dir).unwrap() {
            let e = e.unwrap();
            let name = e.file_name().into_string().unwrap();
            if prefix.is_empty() && name == ".bolt" {
                continue;
            }
            let path = format!("{prefix}{name}");
            if e.file_type().unwrap().is_dir() {
                tree.folders.insert(path.clone());
                todo.push((e.path(), format!("{path}/")));
            } else {
                tree.files.insert(path, std::fs::read(e.path()).unwrap());
            }
        }
    }
    tree
}

/// What a save holds, read from the store.
fn saved_tree(store: &Store, save: Id) -> Tree {
    let mut tree = Tree::default();
    let root = store.save(&save, false).unwrap().root;
    let mut todo = vec![(root, String::new())];
    while let Some((id, prefix)) = todo.pop() {
        for e in store.listing(&id).unwrap().entries() {
            let path = format!("{prefix}{}", e.name);
            match e.kind {
                EntryKind::Folder => {
                    tree.folders.insert(path.clone());
                    todo.push((e.id, format!("{path}/")));
                }
                EntryKind::File => {
                    let mut bytes = Vec::new();
                    store.copy_content(&e.id, &mut bytes).unwrap();
                    tree.files.insert(path, bytes);
                }
                other => panic!("{path} saved as {other:?}"),
            }
        }
    }
    tree
}

/// 64 cases on every push; nightly sets `PROPTEST_CASES` higher.
fn cases() -> u32 {
    std::env::var("PROPTEST_CASES")
        .ok()
        .and_then(|n| n.parse().ok())
        .unwrap_or(64)
}

proptest! {
    #![proptest_config(ProptestConfig { cases: cases(), ..ProptestConfig::default() })]

    #[test]
    fn edits_saves_and_switches_match_the_model(acts in prop::collection::vec(act(), 30)) {
        run(&acts);
    }
}
