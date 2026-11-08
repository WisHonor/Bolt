//! Random edits, saves, amends, switches, discards, shelves, undos and redos, applied both to a
//! project and to a model of what its folder, saves and diary should hold (testing.md §3).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::time::{Duration, SystemTime};

use bolt_core::diary::{self, Action, FolderChange, Position};
use bolt_core::diff::ChangeKind;
use bolt_core::id::Id;
use bolt_core::lines::Target;
use bolt_core::listing::EntryKind;
use bolt_core::repo::Repository;
use bolt_core::save::Person;
use bolt_core::shelf::{Shelved, shelf, unshelf};
use bolt_core::snapshot::{SaveError, SaveOutcome, SaveRequest, save_folder};
use bolt_core::status::status;
use bolt_core::store::Store;
use bolt_core::switch::{SwitchError, discard, goto};
use bolt_core::undo::{
    Effect, StepInfo, StepRequest, UndoError, UndoOutcome, redo, undo, undo_list, undo_to,
};
use proptest::prelude::*;
use serde_json::{Value, json};

mod support;
use support::Env;

// Names differ in more than letter case, so every drive holds them all. No file name is ever a
// folder's.
const FILES: [&str; 6] = ["a", "b", "d/a", "d/c", "d/e/b", "e/a"];
const FOLDERS: [&str; 3] = ["d", "d/e", "e"];
// Few lengths, so files often keep their size when they change.
const LENGTHS: [usize; 3] = [1, 2, 5];
/// The shelf numbers `unshelf` asks for.
const LABELS: [&str; 3] = ["1", "2", "3"];
/// How many of the steps undo can take `bolt undo --list` shows.
const LISTED: usize = 20;

/// The modified time a write leaves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum When {
    /// The clock's, often within the file cache's 2 s of its last look.
    Now,
    /// A time long past that no earlier write had.
    Past,
    /// The time the file had before: a change the file cache is allowed to miss (decision 011).
    Same,
}

#[derive(Debug, Clone)]
enum Act {
    /// Writes the file whole, of one of the `LENGTHS`; the byte varies what it holds.
    Write(&'static str, u8, usize, When),
    /// Writes other bytes of the same length over a file that is there.
    Rewrite(&'static str, u8, When),
    /// Turns the executable bit on or off (Unix only; elsewhere it does nothing).
    Flip(&'static str),
    Delete(&'static str),
    Rename(&'static str, &'static str),
    MakeFolder(&'static str),
    DeleteFolder(&'static str),
    Save,
    /// Replaces the line's newest save, with a new message or keeping its own.
    Amend {
        new_message: bool,
    },
    /// Goes to the save or line `pick` chooses.
    Goto {
        pick: usize,
        by_line: bool,
        unsaved: Unsaved,
    },
    /// Discards everything, or what is at one path.
    Discard(Option<&'static str>),
    /// Undoes or redoes up to `n` steps; `shelve` sets unsaved work in the way aside.
    Undo {
        n: usize,
        shelve: bool,
    },
    Redo {
        n: usize,
        shelve: bool,
    },
    /// Goes back to the diary entry `pick` chooses.
    UndoTo {
        pick: usize,
        shelve: bool,
    },
    Shelf,
    /// Brings back the newest shelf, or the one with that number.
    Unshelf(Option<&'static str>),
    /// Status reading every file, not trusting the file cache.
    FullStatus,
}

/// What a goto does with unsaved work first.
#[derive(Debug, Clone, Copy)]
enum Unsaved {
    Save,
    Discard,
    /// Leaves it: it comes along, or stops the goto where it collides (decision 050).
    Keep,
}

fn act() -> impl Strategy<Value = Act> {
    let file = || prop::sample::select(&FILES[..]);
    let folder = || prop::sample::select(&FOLDERS[..]);
    let when = || prop_oneof![Just(When::Now), Just(When::Past), Just(When::Same)];
    let any_path = prop::sample::select([&FILES[..], &FOLDERS[..]].concat());
    let unsaved = prop_oneof![
        1 => Just(Unsaved::Save),
        1 => Just(Unsaved::Discard),
        2 => Just(Unsaved::Keep)
    ];
    let steps = || (1..=3usize, any::<bool>());
    let label = prop_oneof![
        2 => Just(None),
        1 => prop::sample::select(&LABELS[..]).prop_map(Some)
    ];
    prop_oneof![
        5 => (file(), any::<u8>(), prop::sample::select(&LENGTHS[..]), when())
            .prop_map(|(p, b, n, w)| Act::Write(p, b, n, w)),
        3 => (file(), any::<u8>(), when()).prop_map(|(p, b, w)| Act::Rewrite(p, b, w)),
        1 => file().prop_map(Act::Flip),
        2 => file().prop_map(Act::Delete),
        2 => (file(), file()).prop_map(|(a, b)| Act::Rename(a, b)),
        1 => folder().prop_map(Act::MakeFolder),
        1 => folder().prop_map(Act::DeleteFolder),
        3 => Just(Act::Save),
        2 => any::<bool>().prop_map(|new_message| Act::Amend { new_message }),
        4 => (any::<usize>(), any::<bool>(), unsaved)
            .prop_map(|(pick, by_line, unsaved)| Act::Goto { pick, by_line, unsaved }),
        1 => Just(Act::Discard(None)),
        1 => any_path.prop_map(|p| Act::Discard(Some(p))),
        4 => steps().prop_map(|(n, shelve)| Act::Undo { n, shelve }),
        3 => steps().prop_map(|(n, shelve)| Act::Redo { n, shelve }),
        2 => (any::<usize>(), any::<bool>())
            .prop_map(|(pick, shelve)| Act::UndoTo { pick, shelve }),
        2 => Just(Act::Shelf),
        3 => label.prop_map(Act::Unshelf),
        1 => Just(Act::FullStatus),
    ]
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct File {
    bytes: Vec<u8>,
    exec: bool,
}

/// Files by path, and every folder, a file's included.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Tree {
    files: BTreeMap<String, File>,
    folders: BTreeSet<String>,
}

impl Tree {
    fn put(&mut self, path: &str, file: File) {
        self.add_folders_above(path);
        self.files.insert(path.to_owned(), file);
    }

    fn add_folder(&mut self, path: &str) {
        self.add_folders_above(path);
        self.folders.insert(path.to_owned());
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

    /// Makes `path` and everything in it what `other` has there.
    fn take(&mut self, path: &str, other: &Tree) {
        self.remove(path);
        for (p, f) in other.files.iter().filter(|(p, _)| within(p, path)) {
            self.put(p, f.clone());
        }
        for p in other.folders.iter().filter(|p| within(p, path)) {
            self.add_folder(p);
        }
    }

    /// Makes the entry at `path` what `other` has there: a file, a folder, or nothing.
    fn set(&mut self, path: &str, other: &Tree) {
        if let Some(f) = other.files.get(path) {
            self.put(path, f.clone());
        } else if other.folders.contains(path) {
            self.add_folder(path);
        } else {
            self.remove(path);
        }
    }
}

fn within(p: &str, top: &str) -> bool {
    p == top
        || p.strip_prefix(top)
            .is_some_and(|rest| rest.starts_with('/'))
}

/// The files that differ between two trees.
fn changed_files(a: &Tree, b: &Tree) -> BTreeSet<String> {
    a.files
        .keys()
        .chain(b.files.keys())
        .filter(|p| a.files.get(*p) != b.files.get(*p))
        .cloned()
        .collect()
}

/// The paths where two trees differ: a file, or a folder on one side only.
fn differences(a: &Tree, b: &Tree) -> BTreeSet<String> {
    let mut paths = changed_files(a, b);
    paths.extend(a.folders.symmetric_difference(&b.folders).cloned());
    paths
}

/// What a switch of `folder` from the save `from` to `into` leaves, as `goto` makes it, or the
/// paths where unsaved work is in its way.
fn switch(folder: &Tree, from: &Tree, into: &Tree) -> Result<Tree, BTreeSet<String>> {
    let unsaved = changed_files(folder, from);
    let unlike_target: BTreeSet<String> = unsaved
        .intersection(&changed_files(folder, into))
        .cloned()
        .collect();
    // Decisions 050 and 065: a change unlike the target collides at, above or below a path
    // where the target differs from the save and the folder isn't already as the target is.
    let in_the_way: BTreeSet<String> = differences(from, into)
        .intersection(&differences(folder, into))
        .filter(|s| unlike_target.iter().any(|u| within(u, s) || within(s, u)))
        .cloned()
        .collect();
    if !in_the_way.is_empty() {
        return Err(in_the_way);
    }
    // The target's own differences, around the unsaved work that comes along.
    let mut after = folder.clone();
    for p in from.folders.difference(&into.folders) {
        after.remove(p);
    }
    // Folders made since the save stay, with the folders above them.
    for p in folder.folders.difference(&from.folders) {
        after.add_folder(p);
    }
    for p in changed_files(from, into) {
        match into.files.get(&p) {
            Some(f) => after.put(&p, f.clone()),
            None => drop(after.files.remove(&p)),
        }
    }
    for p in into.folders.difference(&from.folders) {
        after.add_folder(p);
    }
    Ok(after)
}

/// [`switch`], with the unsaved work in its way set aside first if `shelve` says so: the folder
/// after, and the shelf made on `tip`, the save the project is on. A stop gives the paths in
/// the way, unless it was still in the way once set aside.
fn switch_or_shelve(
    folder: &Tree,
    from: &Tree,
    into: &Tree,
    tip: &Tree,
    shelve: bool,
) -> Result<(Tree, Option<Tree>), Option<BTreeSet<String>>> {
    let paths = match switch(folder, from, into) {
        Ok(after) => return Ok((after, None)),
        Err(paths) if !shelve => return Err(Some(paths)),
        Err(paths) => paths,
    };
    let (shelf, from) = set_aside(folder, tip, from, &paths).ok_or(None)?;
    let after = switch(folder, &from, into).map_err(|_| None)?;
    Ok((after, Some(shelf)))
}

/// The shelf `--shelve` makes on `tip` of the work at and below `paths`, and `from` with that work
/// in it, to switch from instead: the folder's version wherever a discard of those paths changes
/// it, folders above them that `tip` holds included. `None` when there is nothing to set aside.
fn set_aside(
    folder: &Tree,
    tip: &Tree,
    from: &Tree,
    paths: &BTreeSet<String>,
) -> Option<(Tree, Tree)> {
    let discarded = |p: &String| {
        paths
            .iter()
            .any(|c| within(p, c) || (tip.folders.contains(p) && within(c, p)))
    };
    let held: Vec<String> = differences(folder, tip)
        .into_iter()
        .filter(discarded)
        .collect();
    if held.is_empty() {
        return None;
    }
    let (mut shelf, mut from) = (tip.clone(), from.clone());
    for p in &held {
        shelf.set(p, folder);
        from.set(p, folder);
    }
    Some((shelf, from))
}

/// A save the model knows.
#[derive(Debug, Clone)]
struct Saved {
    id: Id,
    tree: Tree,
    /// Its first parent.
    parent: Option<usize>,
    /// An amend keeps the change of the save it replaces.
    change: usize,
    message: String,
    /// A shelf's save, which no line reaches.
    shelf: bool,
}

/// The lines, where the project is and the shelves, newest first; saves by index.
#[derive(Debug, Clone, PartialEq, Eq)]
struct State {
    lines: BTreeMap<String, usize>,
    /// The line the project is on, or `None` at the save `on`.
    line: Option<String>,
    on: usize,
    shelves: Vec<(String, usize)>,
}

/// A step: a command's diary entry, with the state and the folder before and after it.
#[derive(Debug, Clone)]
struct Step {
    entry: Id,
    command: &'static str,
    /// `None` for the diary's first entry, which can't be undone.
    before: Option<State>,
    after: State,
    folder: (Tree, Tree),
}

impl Step {
    fn wrote(&self) -> bool {
        self.folder.0 != self.folder.1
    }

    /// What it changed, as undo's list tells it, `files` being its folder change.
    fn effects(&self, m: &Model, files: Option<FolderChange>) -> Vec<Effect> {
        let Some(before) = &self.before else {
            return Vec::new();
        };
        let (from, to) = (recorded(m, before), recorded(m, &self.after));
        let mut found = Vec::new();
        let names: BTreeSet<&String> = from.lines.keys().chain(to.lines.keys()).collect();
        for name in names {
            let (was, now) = (from.lines.get(name).copied(), to.lines.get(name).copied());
            if was != now {
                let name = name.clone();
                found.push(Effect::Line {
                    name,
                    from: was,
                    to: now,
                });
            }
        }
        if from.position != to.position {
            found.push(Effect::Position {
                from: from.position.clone(),
                to: to.position.clone(),
            });
        }
        let holds = |state: &diary::State, save: &Id| state.shelves.iter().any(|(_, s)| s == save);
        for (label, save) in &to.shelves {
            if !holds(&from, save) {
                let (label, save) = (label.clone(), *save);
                found.push(Effect::ShelfAdded { label, save });
            }
        }
        for (label, save) in &from.shelves {
            if !holds(&to, save) {
                let (label, save) = (label.clone(), *save);
                found.push(Effect::ShelfGone { label, save });
            }
        }
        found.extend(files.map(|FolderChange { from, to }| Effect::Files { from, to }));
        found
    }
}

/// What a new diary entry should record.
#[derive(Debug, Clone, Copy)]
enum Made {
    Command(&'static str),
    Undo(Id),
    Redo(Id),
}

struct Model {
    folder: Tree,
    saves: Vec<Saved>,
    state: State,
    /// The steps undo can take, the diary's first entry first, and those redo can bring back,
    /// the next one last.
    done: Vec<Step>,
    undone: Vec<Step>,
    /// Every diary entry, oldest first, with its state.
    entries: Vec<(Id, State)>,
    /// Files whose modified time an earlier file held (`When::Same`), so the file cache may
    /// take them for that file wherever they go.
    shared_time: BTreeSet<String>,
    past: u64,
    new_lines: usize,
}

impl Model {
    /// A project just started: an empty save on `main`.
    fn new(repo: &Repository) -> Model {
        let (head, entry) = repo.current().unwrap();
        let first = entry.state.lines["main"];
        let message = repo.store().save(&first, false).unwrap().message;
        let state = State {
            lines: BTreeMap::from([("main".to_owned(), 0)]),
            line: Some("main".to_owned()),
            on: 0,
            shelves: Vec::new(),
        };
        let start = Step {
            entry: head,
            command: "start",
            before: None,
            after: state.clone(),
            folder: Default::default(),
        };
        Model {
            folder: Tree::default(),
            saves: vec![Saved {
                id: first,
                tree: Tree::default(),
                parent: None,
                change: 0,
                message,
                shelf: false,
            }],
            done: vec![start],
            undone: Vec::new(),
            entries: vec![(head, state.clone())],
            state,
            shared_time: BTreeSet::new(),
            past: 0,
            new_lines: 0,
        }
    }

    fn head(&self) -> Id {
        self.entries[self.entries.len() - 1].0
    }

    /// What the save the project is on holds.
    fn tip(&self) -> &Tree {
        &self.saves[self.state.on].tree
    }
}

/// `s` as the diary records it.
fn recorded(m: &Model, s: &State) -> diary::State {
    let id = |i: &usize| m.saves[*i].id;
    diary::State {
        lines: s.lines.iter().map(|(n, i)| (n.clone(), id(i))).collect(),
        tags: None,
        position: match &s.line {
            Some(line) => Position::OnLine(line.clone()),
            None => Position::AtSave(id(&s.on)),
        },
        shelves: s.shelves.iter().map(|(l, i)| (l.clone(), id(i))).collect(),
        server_lines: None,
    }
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
    let env = Env::empty();
    let root = env.root();
    let mut repo = Repository::init(root, &who()).unwrap();
    // Without a key, as without the user's own, the file cache is never read or written.
    repo.set_cache_key([7; 32]);
    let mut m = Model::new(&repo);
    for (n, a) in acts.iter().enumerate() {
        let missable = apply(&mut repo, &mut m, root, a);
        check(
            &repo,
            &m,
            root,
            matches!(a, Act::FullStatus),
            missable.as_deref(),
        )
        .unwrap_or_else(|e| panic!("after action {n}, {a:?}: {e}"));
    }
    for s in &m.saves {
        let save = repo.store().save(&s.id, false).unwrap();
        let parent = s.parent.map(|p| m.saves[p].id);
        assert_eq!(
            (save.message.as_str(), save.parents.first().copied()),
            (s.message.as_str(), parent),
            "save {:?}",
            s.id
        );
        assert_eq!(saved_tree(repo.store(), s.id), s.tree, "save {:?}", s.id);
    }
    // A process for every action would be too slow on Windows: once a case pins the document.
    listed_document(&env, &repo, &m);
}

/// Applies `a` to the folder and the model. Gives the path of a change the file cache may
/// miss, if it made one.
fn apply(repo: &mut Repository, m: &mut Model, root: &Path, a: &Act) -> Option<String> {
    let who = who();
    let before = m.folder.clone();
    let missable = match *a {
        Act::Write(path, b, len, when) => write(m, root, path, b, len, when),
        Act::Rewrite(path, b, when) => {
            let len = m.folder.files.get(path)?.bytes.len();
            write(m, root, path, b, len, when)
        }
        Act::Flip(path) => {
            flip(m, root, path);
            None
        }
        Act::Delete(path) => {
            m.folder.files.remove(path)?;
            std::fs::remove_file(root.join(path)).unwrap();
            None
        }
        Act::Rename(from, to) => {
            if from == to {
                return None;
            }
            let file = m.folder.files.remove(from)?;
            let at = root.join(to);
            std::fs::create_dir_all(at.parent().unwrap()).unwrap();
            std::fs::rename(root.join(from), &at).unwrap();
            m.folder.put(to, file);
            let shared = m.shared_time.remove(from);
            m.shared_time.remove(to);
            if shared {
                m.shared_time.insert(to.to_owned());
            }
            let time = std::fs::metadata(&at).unwrap().modified().unwrap();
            (shared && long_ago(time)).then(|| to.to_owned())
        }
        Act::MakeFolder(path) => {
            std::fs::create_dir_all(root.join(path)).unwrap();
            m.folder.add_folder(path);
            None
        }
        Act::DeleteFolder(path) => {
            if m.folder.folders.contains(path) {
                std::fs::remove_dir_all(root.join(path)).unwrap();
                m.folder.remove(path);
            }
            None
        }
        Act::Save => {
            save(repo, m);
            None
        }
        Act::Amend { new_message } => {
            amend(repo, m, new_message);
            None
        }
        Act::Goto {
            pick,
            by_line,
            unsaved,
        } => {
            if m.folder != *m.tip() {
                match unsaved {
                    Unsaved::Save => save(repo, m),
                    Unsaved::Discard => discard_all(repo, m),
                    Unsaved::Keep => {}
                }
            }
            let (target, to, line) = if by_line {
                let lines = &m.state.lines;
                let (name, &to) = lines.iter().nth(pick % lines.len()).unwrap();
                (Target::Line(name.clone()), to, Some(name.clone()))
            } else {
                let saves: Vec<usize> = (0..m.saves.len()).filter(|&i| !m.saves[i].shelf).collect();
                let to = saves[pick % saves.len()];
                (Target::Save(m.saves[to].id), to, None)
            };
            let done = goto(repo, &target, &who, "goto", None, &mut |_| {});
            match switch(&m.folder, m.tip(), &m.saves[to].tree) {
                Err(_) => {
                    assert!(
                        matches!(done, Err(SwitchError::Collisions { .. })),
                        "{done:?}"
                    );
                    settle(repo, m, Vec::new());
                }
                Ok(folder) => {
                    done.unwrap();
                    let (was, held) = (m.state.clone(), std::mem::replace(&mut m.folder, folder));
                    m.state.on = to;
                    m.state.line = line;
                    stepped(repo, m, "goto", was, held);
                }
            }
            None
        }
        Act::Discard(None) => {
            discard_all(repo, m);
            None
        }
        Act::Discard(Some(path)) => {
            let saved = m.tip().clone();
            let at = |t: &Tree| {
                t.files.keys().any(|p| within(p, path)) || t.folders.iter().any(|p| within(p, path))
            };
            let paths = [path.to_owned()];
            let done = discard(repo, Some(&paths), &who, "discard", None, &mut |_| {});
            if !at(&saved) && !at(&m.folder) {
                assert!(matches!(done, Err(SwitchError::NoMatch { .. })), "{done:?}");
                settle(repo, m, Vec::new());
                return None;
            }
            done.unwrap();
            let (was, held) = (m.state.clone(), m.folder.clone());
            m.folder.take(path, &saved);
            stepped(repo, m, "discard", was, held);
            None
        }
        Act::Undo { n, shelve } | Act::Redo { n, shelve } => {
            let again = matches!(a, Act::Redo { .. });
            let req = StepRequest {
                n,
                shelve,
                who: &who,
                git_excludes: None,
            };
            let done = if again {
                redo(repo, &req, &mut |_| {})
            } else {
                undo(repo, &req, &mut |_| {})
            };
            take_steps(repo, m, n, (again, shelve), done);
            None
        }
        Act::UndoTo { pick, shelve } => {
            go_back_to(repo, m, pick, shelve);
            None
        }
        Act::Shelf => {
            shelve(repo, m);
            None
        }
        Act::Unshelf(label) => {
            bring_back(repo, m, label);
            None
        }
        Act::FullStatus => None,
    };
    let wrote = matches!(
        a,
        Act::Goto { .. }
            | Act::Discard(_)
            | Act::Undo { .. }
            | Act::Redo { .. }
            | Act::UndoTo { .. }
            | Act::Shelf
            | Act::Unshelf(_)
    );
    if wrote {
        // Files Bolt wrote or removed have times of their own now.
        let after = &m.folder;
        m.shared_time
            .retain(|p| after.files.contains_key(p) && before.files.get(p) == after.files.get(p));
    }
    missable
}

/// Writes `len` bytes to `path`, leaving the modified time `when` says.
fn write(m: &mut Model, root: &Path, path: &str, b: u8, len: usize, when: When) -> Option<String> {
    let mut bytes: Vec<u8> = (0..len).map(|i| b.wrapping_add(37 * i as u8)).collect();
    let old = m.folder.files.get(path).cloned();
    if when != When::Same && old.as_ref().is_some_and(|o| o.bytes == bytes) {
        bytes[0] ^= 1;
    }
    let at = root.join(path);
    let old_time = old
        .as_ref()
        .map(|_| std::fs::metadata(&at).unwrap().modified().unwrap());
    std::fs::create_dir_all(at.parent().unwrap()).unwrap();
    std::fs::write(&at, &bytes).unwrap();
    let time = match when {
        When::Now => None,
        When::Past => {
            m.past += 1;
            Some(SystemTime::UNIX_EPOCH + Duration::from_secs(1_600_000_000 + m.past))
        }
        When::Same => old_time,
    };
    if let Some(t) = time {
        let f = std::fs::OpenOptions::new().write(true).open(&at).unwrap();
        f.set_modified(t).unwrap();
    }
    let exec = old.as_ref().is_some_and(|o| o.exec);
    m.folder.put(
        path,
        File {
            bytes: bytes.clone(),
            exec,
        },
    );
    let kept_time = when == When::Same && old.is_some();
    if kept_time {
        m.shared_time.insert(path.to_owned());
    } else {
        m.shared_time.remove(path);
    }
    let missable = kept_time
        && old_time.is_some_and(long_ago)
        && old.is_some_and(|o| o.bytes.len() == len && o.bytes != bytes);
    missable.then(|| path.to_owned())
}

/// Whether a file with this modified time may be trusted by the file cache: only one at least
/// 2 s older than a look that recorded it is, and every look so far came before now.
fn long_ago(time: SystemTime) -> bool {
    // 1.5 s leaves room for the drive's clock running a little apart from this one.
    time + Duration::from_millis(1500) < SystemTime::now()
}

#[cfg(unix)]
fn flip(m: &mut Model, root: &Path, path: &str) {
    use std::os::unix::fs::PermissionsExt;
    let Some(file) = m.folder.files.get_mut(path) else {
        return;
    };
    file.exec = !file.exec;
    let mode = if file.exec { 0o755 } else { 0o644 };
    std::fs::set_permissions(root.join(path), std::fs::Permissions::from_mode(mode)).unwrap();
}

#[cfg(not(unix))]
fn flip(_: &mut Model, _: &Path, _: &str) {}

fn save(repo: &mut Repository, m: &mut Model) {
    let who = who();
    // A save made at a save starts a line.
    let new_line = m
        .state
        .line
        .is_none()
        .then(|| format!("line-{}", m.new_lines));
    let req = SaveRequest {
        message: Some("Save"),
        who: &who,
        command: "save",
        accept_crlf: false,
        git_excludes: None,
        new_line: new_line.as_deref(),
        only: None,
        amend: false,
    };
    match save_folder(repo, &req, &mut |_| {}, &mut |_| true).unwrap() {
        SaveOutcome::Saved { save_id, .. } => {
            assert_ne!(m.folder, *m.tip(), "saved with nothing changed");
            assert_eq!(saved_tree(repo.store(), save_id), m.folder);
            let was = m.state.clone();
            m.saves.push(Saved {
                id: save_id,
                tree: m.folder.clone(),
                parent: Some(m.state.on),
                change: m.saves.len(),
                message: "Save".to_owned(),
                shelf: false,
            });
            m.state.on = m.saves.len() - 1;
            if let Some(name) = new_line {
                m.new_lines += 1;
                m.state.line = Some(name);
            }
            m.state
                .lines
                .insert(m.state.line.clone().unwrap(), m.state.on);
            let held = m.folder.clone();
            stepped(repo, m, "save", was, held);
        }
        SaveOutcome::NothingChanged { .. } => {
            assert_eq!(m.folder, *m.tip(), "changes not saved");
            settle(repo, m, Vec::new());
        }
    }
}

/// `save --amend`: refused at a save, and where another line shows the save.
fn amend(repo: &mut Repository, m: &mut Model, new_message: bool) {
    let who = who();
    let message = new_message.then_some("Amended");
    let req = SaveRequest {
        message,
        who: &who,
        command: "save",
        accept_crlf: false,
        git_excludes: None,
        new_line: None,
        only: None,
        amend: true,
    };
    let done = save_folder(repo, &req, &mut |_| {}, &mut |_| true);
    let old = m.state.on;
    let Some(line) = m.state.line.clone() else {
        assert!(matches!(done, Err(SaveError::AmendAtSave)), "{done:?}");
        settle(repo, m, Vec::new());
        return;
    };
    if shown_elsewhere(m, &line, old) {
        assert!(
            matches!(done, Err(SaveError::AmendShared { .. })),
            "{done:?}"
        );
        settle(repo, m, Vec::new());
        return;
    }
    let message = message.unwrap_or(m.saves[old].message.as_str()).to_owned();
    let same = m.folder == m.saves[old].tree && message == m.saves[old].message;
    match done.unwrap() {
        SaveOutcome::NothingChanged { .. } => {
            assert!(same, "nothing amended");
            settle(repo, m, Vec::new());
        }
        SaveOutcome::Saved {
            save_id, replaced, ..
        } => {
            assert!(!same, "amended with nothing to amend");
            assert_eq!(replaced, Some(m.saves[old].id));
            assert_eq!(saved_tree(repo.store(), save_id), m.folder);
            let was = m.state.clone();
            let (parent, change) = (m.saves[old].parent, m.saves[old].change);
            m.saves.push(Saved {
                id: save_id,
                tree: m.folder.clone(),
                parent,
                change,
                message,
                shelf: false,
            });
            m.state.on = m.saves.len() - 1;
            m.state.lines.insert(line, m.state.on);
            let held = m.folder.clone();
            stepped(repo, m, "save", was, held);
        }
    }
}

/// Whether a line other than `line` holds `save`, or a save of its change, along first parents.
fn shown_elsewhere(m: &Model, line: &str, save: usize) -> bool {
    let change = m.saves[save].change;
    m.state
        .lines
        .iter()
        .filter(|(name, _)| *name != line)
        .any(|(_, &tip)| {
            std::iter::successors(Some(tip), |&s| m.saves[s].parent)
                .any(|s| m.saves[s].change == change)
        })
}

/// Throws every unsaved change away.
fn discard_all(repo: &mut Repository, m: &mut Model) {
    discard(repo, None, &who(), "discard", None, &mut |_| {}).unwrap();
    let saved = m.tip().clone();
    let (was, held) = (m.state.clone(), std::mem::replace(&mut m.folder, saved));
    stepped(repo, m, "discard", was, held);
}

/// `shelf`: sets every unsaved change aside, then puts the folder back to its save.
fn shelve(repo: &mut Repository, m: &mut Model) {
    let done = shelf(repo, None, &who(), None, &mut |_| {}).unwrap();
    if m.folder == *m.tip() {
        assert!(
            matches!(
                done,
                Shelved::Nothing {
                    changed_again: false,
                    ..
                }
            ),
            "{done:?}"
        );
        settle(repo, m, Vec::new());
        return;
    }
    let Shelved::Made(made) = done else {
        panic!("nothing shelved: {done:?}");
    };
    assert_eq!(made.files, changed_files(m.tip(), &m.folder).len());
    let label = free_label(&m.state.shelves);
    let was = m.state.clone();
    let mut made = std::iter::once((made.label, made.save));
    let (store, tree) = (repo.store(), m.folder.clone());
    let save = new_shelf(store, m, &mut made, &label, tree, "Shelved");
    m.state.shelves.insert(0, (label, save));
    let saved = m.tip().clone();
    let held = std::mem::replace(&mut m.folder, saved);
    stepped(repo, m, "shelf", was, held);
}

/// `unshelf`: brings back the newest shelf, or the one labelled `label`, as a switch from the
/// save it was made on.
fn bring_back(repo: &mut Repository, m: &mut Model, label: Option<&str>) {
    let done = unshelf(repo, label, &who(), None, &mut |_| {});
    let asked = match label {
        Some(label) => m.state.shelves.iter().position(|(l, _)| l == label),
        None => (!m.state.shelves.is_empty()).then_some(0),
    };
    let Some(k) = asked else {
        assert!(matches!(done, Err(SwitchError::NoShelf { .. })), "{done:?}");
        settle(repo, m, Vec::new());
        return;
    };
    let (name, save) = m.state.shelves[k].clone();
    let base = m.saves[save].parent.unwrap();
    match switch(&m.folder, &m.saves[base].tree, &m.saves[save].tree) {
        Err(_) => {
            assert!(
                matches!(done, Err(SwitchError::ShelfInTheWay(_))),
                "{done:?}"
            );
            settle(repo, m, Vec::new());
        }
        Ok(folder) => {
            let done = done.unwrap();
            assert_eq!((done.label, done.save), (name, m.saves[save].id));
            let was = m.state.clone();
            m.state.shelves.remove(k);
            let held = std::mem::replace(&mut m.folder, folder);
            stepped(repo, m, "unshelf", was, held);
        }
    }
}

/// The smallest number from 1 that no shelf has.
fn free_label(shelves: &[(String, usize)]) -> String {
    (1usize..)
        .map(|n| n.to_string())
        .find(|n| !shelves.iter().any(|(l, _)| l == n))
        .unwrap()
}

/// Adds the shelf the engine says it made next, on the save the project is on, once it holds
/// `tree`. Gives its index.
fn new_shelf(
    store: &Store,
    m: &mut Model,
    made: &mut dyn Iterator<Item = (String, Id)>,
    label: &str,
    tree: Tree,
    message: &str,
) -> usize {
    let (got, id) = made.next().expect("a shelf the engine didn't make");
    assert_eq!(got, label, "the new shelf's number");
    assert_eq!(saved_tree(store, id), tree, "what shelf {label} holds");
    m.saves.push(Saved {
        id,
        tree,
        parent: Some(m.state.on),
        change: m.saves.len(),
        message: message.to_owned(),
        shelf: true,
    });
    m.saves.len() - 1
}

/// Takes off the shelves a step added and puts back those it removed, compared by save, for an
/// undo; the other way round for a redo. One put back goes first, keeping its number unless
/// another shelf has it.
fn put_shelves_back(
    shelves: &mut Vec<(String, usize)>,
    before: &[(String, usize)],
    after: &[(String, usize)],
    again: bool,
) {
    let missing = |from: &[(String, usize)], there: &[(String, usize)]| -> Vec<(String, usize)> {
        let gone = |save: &usize| !there.iter().any(|(_, s)| s == save);
        from.iter().filter(|(_, s)| gone(s)).cloned().collect()
    };
    let (added, removed) = (missing(after, before), missing(before, after));
    let (off, back) = if again {
        (removed, added)
    } else {
        (added, removed)
    };
    shelves.retain(|(_, s)| !off.iter().any(|(_, o)| o == s));
    for (at, (label, save)) in back.into_iter().enumerate() {
        let label = if shelves.iter().any(|(l, _)| *l == label) {
            free_label(shelves)
        } else {
            label
        };
        shelves.insert(at, (label, save));
    }
}

/// What the model makes of the next step undo takes, or redo brings back.
enum Next {
    /// Taken: its entry records this.
    Took(Made),
    /// None left.
    Nothing,
    /// Unsaved work in the way stopped it, at these paths when the model knows them.
    Stopped(Option<BTreeSet<String>>),
}

/// Takes the next step back, or forward with `again`: the state from before it, or its own,
/// with the shelves it added or removed taken off or put back, and the folder switched back from
/// what it wrote to what it held, or the other way.
fn take(
    store: &Store,
    m: &mut Model,
    (again, shelve): (bool, bool),
    shelved: &mut dyn Iterator<Item = (String, Id)>,
) -> Next {
    let next = if again {
        m.undone.last()
    } else {
        m.done.last().filter(|s| s.before.is_some())
    };
    let Some(step) = next.cloned() else {
        return Next::Nothing;
    };
    let before = step.before.clone().unwrap();
    let source = if again { &step.after } else { &before };
    let mut state = State {
        shelves: m.state.shelves.clone(),
        ..source.clone()
    };
    put_shelves_back(
        &mut state.shelves,
        &before.shelves,
        &step.after.shelves,
        again,
    );
    let (held, wrote) = &step.folder;
    let (from, into) = if again { (held, wrote) } else { (wrote, held) };
    if held != wrote {
        let (folder, shelf) = match switch_or_shelve(&m.folder, from, into, m.tip(), shelve) {
            Ok(done) => done,
            Err(paths) => return Next::Stopped(paths),
        };
        if let Some(tree) = shelf {
            let label = free_label(&state.shelves);
            let message = format!("Set aside by {}", if again { "redo" } else { "undo" });
            let save = new_shelf(store, m, shelved, &label, tree, &message);
            state.shelves.insert(0, (label, save));
        }
        m.folder = folder;
    }
    m.state = state;
    let entry = step.entry;
    if again {
        m.undone.pop();
        m.done.push(step);
        Next::Took(Made::Redo(entry))
    } else {
        m.done.pop();
        m.undone.push(step);
        Next::Took(Made::Undo(entry))
    }
}

/// The steps an undo, a redo or an `undo --to` took, the shelves it made and why it stopped.
fn outcome(
    done: Result<UndoOutcome, UndoError>,
) -> (usize, Vec<(String, Id)>, Option<SwitchError>) {
    match done {
        Ok(done) => {
            assert!(!done.finished && !done.head_lost, "{done:?}");
            (done.taken.len(), done.shelved, None)
        }
        Err(e) => (e.taken.len(), e.shelved, Some(*e.error)),
    }
}

/// Plays up to `n` steps of undo, or of redo with `again`, on the model, and holds `done`, what
/// the engine did, to them.
fn take_steps(
    repo: &Repository,
    m: &mut Model,
    n: usize,
    (again, shelve): (bool, bool),
    done: Result<UndoOutcome, UndoError>,
) {
    let (taken, shelved, error) = outcome(done);
    let mut shelved = shelved.into_iter();
    let (mut made, mut stopped) = (Vec::new(), None);
    for _ in 0..n {
        match take(repo.store(), m, (again, shelve), &mut shelved) {
            Next::Took(entry) => made.push((entry, m.state.clone())),
            Next::Nothing => break,
            Next::Stopped(paths) => {
                stopped = Some(paths);
                break;
            }
        }
    }
    same_stop(stopped, error, (again, shelve));
    assert_eq!(taken, made.len(), "steps taken");
    assert!(shelved.next().is_none(), "a shelf the model didn't make");
    settle(repo, m, made);
}

/// Holds the engine's error to the model's stop: none, or unsaved work in the way of a step, at
/// the paths the model gives when it gives them.
fn same_stop(
    stopped: Option<Option<BTreeSet<String>>>,
    error: Option<SwitchError>,
    (again, shelve): (bool, bool),
) {
    match (stopped, error) {
        (None, None) => {}
        (
            Some(want),
            Some(SwitchError::InTheWayOfStep {
                paths,
                redo,
                shelve: shelved,
                ..
            }),
        ) => {
            assert_eq!((redo, shelved), (again, shelve), "{paths:?}");
            if let Some(want) = want {
                assert_eq!(paths, want.into_iter().collect::<Vec<_>>());
            }
        }
        (stopped, error) => panic!("the model stops at {stopped:?}, the engine with {error:?}"),
    }
}

/// `undo --to` the entry `pick` chooses: the current one changes nothing, one undo or redo
/// reaches is reached by them, and any other by a jump.
fn go_back_to(repo: &mut Repository, m: &mut Model, pick: usize, shelve: bool) {
    let who = who();
    let (id, target) = m.entries[pick % m.entries.len()].clone();
    let req = StepRequest {
        n: 1,
        shelve,
        who: &who,
        git_excludes: None,
    };
    let done = undo_to(repo, &id.to_hex(), &req, &mut |_| {});
    if id == m.head() {
        let (taken, shelved, error) = outcome(done);
        assert!(
            taken == 0 && shelved.is_empty() && error.is_none(),
            "{error:?}"
        );
        settle(repo, m, Vec::new());
    } else if let Some(i) = m.undone.iter().position(|s| s.entry == id) {
        let n = m.undone.len() - i;
        take_steps(repo, m, n, (true, shelve), done);
    } else if let Some(i) = m.done.iter().position(|s| s.entry == id) {
        let n = m.done.len() - 1 - i;
        take_steps(repo, m, n, (false, shelve), done);
    } else {
        jump(repo, m, target, shelve, done);
    }
}

/// `undo --to` an entry neither undo nor redo reaches: one step to its lines and position, the
/// shelves kept, switching the folder to its save as `goto` does. None if that is where things are.
fn jump(
    repo: &Repository,
    m: &mut Model,
    target: State,
    shelve: bool,
    done: Result<UndoOutcome, UndoError>,
) {
    let (taken, shelved, error) = outcome(done);
    let mut shelved = shelved.into_iter();
    let was = m.state.clone();
    let mut state = State {
        shelves: was.shelves.clone(),
        ..target
    };
    let there = recorded(m, &state) == recorded(m, &was);
    let into = m.saves[state.on].tree.clone();
    match switch_or_shelve(&m.folder, m.tip(), &into, m.tip(), shelve) {
        _ if there => {
            assert!(taken == 0 && error.is_none(), "{error:?}");
            settle(repo, m, Vec::new());
        }
        Err(paths) => {
            assert_eq!(taken, 0);
            same_stop(Some(paths), error, (false, shelve));
            settle(repo, m, Vec::new());
        }
        Ok((folder, shelf)) => {
            assert_eq!(taken, 1);
            same_stop(None, error, (false, shelve));
            if let Some(tree) = shelf {
                let label = free_label(&state.shelves);
                let save = new_shelf(
                    repo.store(),
                    m,
                    &mut shelved,
                    &label,
                    tree,
                    "Set aside by undo",
                );
                state.shelves.insert(0, (label, save));
            }
            let held = std::mem::replace(&mut m.folder, folder);
            m.state = state;
            stepped(repo, m, "undo", was, held);
        }
    }
    assert!(shelved.next().is_none(), "a shelf the model didn't make");
}

/// Settles what a command did: a step if it changed the state or the folder, else no entry.
/// The step clears what redo could bring back.
fn stepped(repo: &Repository, m: &mut Model, command: &'static str, was: State, held: Tree) {
    if recorded(m, &m.state) == recorded(m, &was) && m.folder == held {
        settle(repo, m, Vec::new());
        return;
    }
    let made = vec![(Made::Command(command), m.state.clone())];
    let entry = settle(repo, m, made)[0];
    let switched = repo.entry(&entry).unwrap().folder_change.is_some();
    assert_eq!(
        switched,
        m.folder != held,
        "{command} records a folder change"
    );
    m.undone.clear();
    m.done.push(Step {
        entry,
        command,
        before: Some(was),
        after: m.state.clone(),
        folder: (held, m.folder.clone()),
    });
}

/// Holds the entries the diary gained since the model's last one to `made`: what each records,
/// and the state the model gives it. Adds them to the model's, and gives their IDs.
fn settle(repo: &Repository, m: &mut Model, made: Vec<(Made, State)>) -> Vec<Id> {
    let mut new = Vec::new();
    let mut at = repo.head().unwrap();
    while at != m.head() {
        assert!(
            new.len() < made.len(),
            "the diary gained more than the {} entries the model made",
            made.len()
        );
        new.push(at);
        at = repo
            .entry(&at)
            .unwrap()
            .parent
            .expect("no entry the model has");
    }
    assert_eq!(new.len(), made.len(), "new diary entries");
    new.reverse();
    for (id, (kind, state)) in new.iter().zip(made) {
        let entry = repo.entry(id).unwrap();
        let (action, command) = match kind {
            Made::Command(command) => (Action::Command, command),
            Made::Undo(step) => (Action::Undo(step), "undo"),
            Made::Redo(step) => (Action::Redo(step), "redo"),
        };
        let told = (&entry.action, entry.command.as_str());
        assert_eq!(told, (&action, command), "entry {id:?}");
        if let Made::Undo(step) | Made::Redo(step) = kind {
            // The step's own folder change, backwards for an undo.
            let change = repo.entry(&step).unwrap().folder_change;
            let change = change.map(|c| match kind {
                Made::Undo(_) => FolderChange {
                    from: c.to,
                    to: c.from,
                },
                _ => c,
            });
            assert_eq!(entry.folder_change, change, "the folder change of {kind:?}");
        }
        assert_eq!(entry.state, recorded(m, &state), "entry {id:?}");
        m.entries.push((*id, state));
    }
    new
}

/// The changes `status` should list, by path.
fn changes(folder: &Tree, saved: &Tree) -> Vec<(String, ChangeKind)> {
    changed_files(folder, saved)
        .into_iter()
        .map(|p| {
            let kind = match (saved.files.get(&p), folder.files.get(&p)) {
                (None, _) => ChangeKind::Added,
                (_, None) => ChangeKind::Removed,
                (Some(a), Some(b)) if a.exec != b.exec => ChangeKind::KindChanged,
                _ => ChangeKind::Modified,
            };
            (p, kind)
        })
        .collect()
}

/// What the folder, `status`, the state and undo's lists say, against the model. `missable` is
/// a change the file cache may miss: status that trusts it may say either, and a full one must
/// see it.
fn check(
    repo: &Repository,
    m: &Model,
    root: &Path,
    full: bool,
    missable: Option<&str>,
) -> Result<(), String> {
    let on_disk = folder_tree(root);
    if on_disk != m.folder {
        return Err(format!("the folder holds {on_disk:?}, not {:?}", m.folder));
    }
    let want = changes(&m.folder, m.tip());
    let listed = |full: bool| -> Result<Vec<(String, ChangeKind)>, String> {
        let st = status(repo, full, None).map_err(|e| e.to_string())?;
        if st.save_id != m.saves[m.state.on].id {
            return Err("status compared with the wrong save".into());
        }
        Ok(st
            .changes
            .iter()
            .map(|c| (c.path.clone(), c.change))
            .collect())
    };
    let got = listed(full)?;
    let other = |l: &[(String, ChangeKind)]| -> Vec<(String, ChangeKind)> {
        l.iter()
            .filter(|(p, _)| Some(p.as_str()) != missable)
            .cloned()
            .collect()
    };
    if other(&got) != other(&want) {
        return Err(format!("status says {got:?}, not {want:?}"));
    }
    if missable.is_some() {
        let got = listed(true)?;
        if got != want {
            return Err(format!(
                "status reading every file says {got:?}, not {want:?}"
            ));
        }
    } else if got != want {
        return Err(format!("status says {got:?}, not {want:?}"));
    }
    let (head, entry) = repo.current().map_err(|e| e.to_string())?;
    if head != m.head() {
        return Err(format!("the diary is at {head:?}, not {:?}", m.head()));
    }
    let state = recorded(m, &m.state);
    if entry.state != state {
        return Err(format!("the state is {:?}, not {state:?}", entry.state));
    }
    let to_redo: Vec<&Step> = m.undone.iter().rev().collect();
    // A short list too, since a case rarely makes more steps than the list shows.
    for limit in [LISTED, 3] {
        let list = undo_list(repo, limit).map_err(|e| e.to_string())?;
        let to_undo: Vec<&Step> = m.done.iter().rev().take(limit).collect();
        steps_listed(m, "redo", &to_redo, &list.redo)?;
        steps_listed(m, "undo", &to_undo, &list.undo)?;
        // The diary's first entry, which can't be undone, is never one more.
        if list.more != (m.done.len() > limit + 1) {
            return Err(format!("undo's list of {limit} says more is {}", list.more));
        }
    }
    Ok(())
}

/// Whether one of undo's lists holds exactly `steps`, each with what it changed.
fn steps_listed(m: &Model, which: &str, steps: &[&Step], got: &[StepInfo]) -> Result<(), String> {
    let found: Vec<Id> = got.iter().map(|s| s.entry).collect();
    let want: Vec<Id> = steps.iter().map(|s| s.entry).collect();
    if found != want {
        return Err(format!("the {which} list holds {found:?}, not {want:?}"));
    }
    for (s, g) in steps.iter().zip(got) {
        let files = g.diary.folder_change;
        let effects = s.effects(m, files);
        let told = (
            &g.diary.action,
            g.diary.command.as_str(),
            files.is_some(),
            &g.effects,
        );
        if told != (&Action::Command, s.command, s.wrote(), &effects) {
            return Err(format!(
                "the {which} list tells {:?} as {told:?}, not {} {effects:?}",
                g.entry, s.command
            ));
        }
    }
    Ok(())
}

/// `bolt undo --list --json`, against the model.
fn listed_document(env: &Env, repo: &Repository, m: &Model) {
    let out = env
        .bolt()
        .args(["undo", "--list", "--json"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let doc: Value = serde_json::from_slice(&out.stdout).unwrap();
    let entries =
        |steps: Vec<&Step>| -> Value { steps.into_iter().map(|s| step_json(repo, m, s)).collect() };
    let want = json!({
        "bolt_json": 1,
        "redo": entries(m.undone.iter().rev().collect()),
        "undo": entries(m.done.iter().rev().take(LISTED).collect()),
        "more": m.done.len() > LISTED + 1,
    });
    let got = json!({
        "bolt_json": doc["bolt_json"],
        "redo": doc["redo"],
        "undo": doc["undo"],
        "more": doc["more"],
    });
    assert_eq!(got, want);
}

/// A step as `undo --list --json` gives it (json.md).
fn step_json(repo: &Repository, m: &Model, s: &Step) -> Value {
    let files = repo.entry(&s.entry).unwrap().folder_change;
    let effects: Vec<Value> = s.effects(m, files).iter().map(effect_json).collect();
    json!({
        "entry": s.entry.to_hex(),
        "action": "command",
        "target": null,
        "command": s.command,
        "time_ms": who().time_ms,
        "effects": effects,
        "first": s.before.is_none(),
    })
}

/// An effect as json.md gives it.
fn effect_json(e: &Effect) -> Value {
    let hex = |id: &Option<Id>| id.map(|id| id.to_hex());
    let place = |p: &Position| match p {
        Position::OnLine(line) => json!({ "line": line }),
        Position::AtSave(save) => json!({ "save": save.to_hex() }),
    };
    match e {
        Effect::Line { name, from, to } => {
            json!({"kind": "line", "name": name, "from": hex(from), "to": hex(to)})
        }
        Effect::Position { from, to } => {
            json!({"kind": "position", "from": place(from), "to": place(to)})
        }
        Effect::ShelfAdded { label, save } => {
            json!({"kind": "shelf_added", "label": label, "save": save.to_hex()})
        }
        Effect::ShelfGone { label, save } => {
            json!({"kind": "shelf_gone", "label": label, "save": save.to_hex()})
        }
        Effect::Files { from, to } => {
            json!({"kind": "files", "from": from.to_hex(), "to": to.to_hex()})
        }
        Effect::Tags | Effect::ServerLines => panic!("the model has no tags or server lines"),
    }
}

#[cfg(unix)]
fn executable(meta: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    meta.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn executable(_: &std::fs::Metadata) -> bool {
    false
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
            let meta = e.metadata().unwrap();
            if meta.is_dir() {
                tree.folders.insert(path.clone());
                todo.push((e.path(), format!("{path}/")));
            } else {
                let bytes = std::fs::read(e.path()).unwrap();
                let exec = executable(&meta);
                tree.files.insert(path, File { bytes, exec });
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
            let exec = match e.kind {
                EntryKind::Folder => {
                    tree.folders.insert(path.clone());
                    todo.push((e.id, format!("{path}/")));
                    continue;
                }
                EntryKind::File => false,
                EntryKind::Executable => true,
                other => panic!("{path} saved as {other:?}"),
            };
            let mut bytes = Vec::new();
            store.copy_content(&e.id, &mut bytes).unwrap();
            tree.files.insert(path, File { bytes, exec });
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
    fn edits_saves_switches_shelves_and_undos_match_the_model(
        acts in prop::collection::vec(act(), 1..=30)
    ) {
        run(&acts);
    }
}
