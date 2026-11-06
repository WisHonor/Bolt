//! Random edits, saves, switches and discards, applied both to a project and to a model of what
//! its folder and saves should hold (testing.md §3).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::time::{Duration, SystemTime};

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
// Few lengths, so files often keep their size when they change.
const LENGTHS: [usize; 3] = [1, 2, 5];

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
    /// Goes to the save or line `pick` chooses.
    Goto {
        pick: usize,
        by_line: bool,
        unsaved: Unsaved,
    },
    /// Discards everything, or what is at one path.
    Discard(Option<&'static str>),
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
        4 => (any::<usize>(), any::<bool>(), unsaved)
            .prop_map(|(pick, by_line, unsaved)| Act::Goto { pick, by_line, unsaved }),
        1 => Just(Act::Discard(None)),
        1 => any_path.prop_map(|p| Act::Discard(Some(p))),
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

struct Model {
    folder: Tree,
    saves: Vec<(Id, Tree)>,
    lines: BTreeMap<String, usize>,
    /// The save the folder is on, and its line if it is on one.
    on: usize,
    line: Option<String>,
    /// Files whose modified time an earlier file held (`When::Same`), so the file cache may
    /// take them for that file wherever they go.
    shared_time: BTreeSet<String>,
    past: u64,
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
        shared_time: BTreeSet::new(),
        past: 0,
        new_lines: 0,
    };
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
    for (id, tree) in &m.saves {
        assert_eq!(&saved_tree(repo.store(), *id), tree, "save {id:?}");
    }
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
        Act::Goto {
            pick,
            by_line,
            unsaved,
        } => {
            if m.folder != m.saves[m.on].1 {
                match unsaved {
                    Unsaved::Save => save(repo, m),
                    Unsaved::Discard => {
                        discard(repo, None, &who, "discard", None, &mut |_| {}).unwrap();
                        m.folder = m.saves[m.on].1.clone();
                    }
                    Unsaved::Keep => {}
                }
            }
            let (target, to, line) = if by_line {
                let (name, &to) = m.lines.iter().nth(pick % m.lines.len()).unwrap();
                (Target::Line(name.clone()), to, Some(name.clone()))
            } else {
                let to = pick % m.saves.len();
                (Target::Save(m.saves[to].0), to, None)
            };
            let (from, into) = (&m.saves[m.on].1, &m.saves[to].1);
            let unsaved = changed_files(&m.folder, from);
            let unlike_target: BTreeSet<String> = unsaved
                .intersection(&changed_files(&m.folder, into))
                .cloned()
                .collect();
            let files = changed_files(from, into);
            let removed: BTreeSet<String> =
                from.folders.difference(&into.folders).cloned().collect();
            let added: BTreeSet<String> = into.folders.difference(&from.folders).cloned().collect();
            // Decisions 050 and 065: a change unlike the target collides at, above or below a
            // path where the target differs from the save and the folder isn't already as the
            // target is.
            let in_the_way: BTreeSet<String> = differences(from, into)
                .intersection(&differences(&m.folder, into))
                .cloned()
                .collect();
            let collides = unlike_target
                .iter()
                .any(|u| in_the_way.iter().any(|s| within(u, s) || within(s, u)));
            let done = goto(repo, &target, &who, "goto", None, &mut |_| {});
            if collides {
                assert!(
                    matches!(done, Err(SwitchError::Collisions { .. })),
                    "{done:?}"
                );
            } else {
                done.unwrap();
                // The target's own differences, around the unsaved work that comes along.
                let into = into.clone();
                // Folders made since the save stay, with the folders above them.
                let made: Vec<String> = m
                    .folder
                    .folders
                    .difference(&from.folders)
                    .cloned()
                    .collect();
                for p in &removed {
                    m.folder.remove(p);
                }
                for p in &made {
                    m.folder.add_folder(p);
                }
                for p in &files {
                    match into.files.get(p) {
                        Some(f) => m.folder.put(p, f.clone()),
                        None => drop(m.folder.files.remove(p)),
                    }
                }
                for p in &added {
                    m.folder.add_folder(p);
                }
                m.on = to;
                m.line = line;
            }
            None
        }
        Act::Discard(None) => {
            discard(repo, None, &who, "discard", None, &mut |_| {}).unwrap();
            m.folder = m.saves[m.on].1.clone();
            None
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
                return None;
            }
            done.unwrap();
            let saved = saved.clone();
            m.folder.take(path, &saved);
            None
        }
        Act::FullStatus => None,
    };
    if matches!(a, Act::Goto { .. } | Act::Discard(_)) {
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

/// What the folder, `status`, the lines and the position say, against the model. `missable` is
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
    let want = changes(&m.folder, &m.saves[m.on].1);
    let listed = |full: bool| -> Result<Vec<(String, ChangeKind)>, String> {
        let st = status(repo, full, None).map_err(|e| e.to_string())?;
        if st.save_id != m.saves[m.on].0 {
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
    fn edits_saves_and_switches_match_the_model(acts in prop::collection::vec(act(), 1..=30)) {
        run(&acts);
    }
}
