//! Hostile saves written out (security.md §4, T1 to T5, and the write-out cases of the review of
//! Git's published vulnerabilities): each case plants a save, or a journal, as a hostile `.bolt`
//! could hold it, runs `bolt goto` onto it, and checks what was written, that nothing reached the
//! folder beside the project or went into `.git` or `.bolt`, and what Bolt said.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use bolt_core::id::Id;
use bolt_core::journal::{Action, Change, Content, Journal};
use bolt_core::listing::{Entry, EntryKind, Listing};
use bolt_core::pack::{ObjectKind, PackSet};
use bolt_core::repo::Repository;
use bolt_platform::dir::Dir;
use support::{Env, add_listing, plant_save, text};
use tempfile::TempDir;

use EntryKind::{Executable, File, Symlink};

/// What every planted file holds, so a stray copy of it is easy to find.
const PAYLOAD: &[u8] = b"PWNED by a hostile save";

/// An entry as `seen` finds it, links never followed.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Seen {
    Folder,
    File(Vec<u8>),
    Link(PathBuf),
}

/// Everything below `dir`, by path with `/` between names; empty if there is no `dir`.
fn seen(dir: &Path) -> BTreeMap<String, Seen> {
    let mut found = BTreeMap::new();
    let mut folders = vec![(dir.to_path_buf(), String::new())];
    while let Some((at, rel)) = folders.pop() {
        let Ok(entries) = std::fs::read_dir(&at) else {
            continue;
        };
        for e in entries {
            let e = e.unwrap();
            let name = e.file_name().to_string_lossy().into_owned();
            let path = if rel.is_empty() {
                name
            } else {
                format!("{rel}/{name}")
            };
            let kind = e.file_type().unwrap();
            let what = if kind.is_symlink() {
                Seen::Link(std::fs::read_link(e.path()).unwrap())
            } else if kind.is_dir() {
                folders.push((e.path(), path.clone()));
                Seen::Folder
            } else {
                Seen::File(std::fs::read(e.path()).unwrap())
            };
            found.insert(path, what);
        }
    }
    found
}

/// The project's own entries: `seen`, without `.git` and `.bolt`.
fn project_entries(env: &Env) -> BTreeMap<String, Seen> {
    let mut all = seen(env.root());
    all.retain(|path, _| {
        let top = path.split('/').next().unwrap();
        top != ".git" && top != ".bolt"
    });
    all
}

fn file(bytes: &[u8]) -> Seen {
    Seen::File(bytes.to_vec())
}

/// A project with a `.git` folder beside `.bolt`, as after an import, at its only save but not on
/// a line, so that `bolt goto main` switches to whatever is planted on `main`; and a folder
/// beside it. `before_git` runs before `.git` is made.
struct Case {
    env: Env,
    outside: TempDir,
    git: BTreeMap<String, Seen>,
    bolt: BTreeMap<String, Seen>,
    beside: BTreeMap<String, Seen>,
}

impl Case {
    fn new() -> Case {
        Case::with(|_| {})
    }

    fn with(before_git: impl FnOnce(&Path)) -> Case {
        let env = Env::empty();
        env.bolt().arg("start").assert().success();
        before_git(env.root());
        let hooks = env.root().join(".git/hooks");
        std::fs::create_dir_all(&hooks).unwrap();
        std::fs::write(env.root().join(".git/config"), b"[core]\n").unwrap();
        std::fs::write(hooks.join("pre-commit.sample"), b"#!/bin/sh\n").unwrap();
        let first = env.state().1.lines["main"].to_hex();
        let out = env.bolt().args(["goto", &first]).output().unwrap();
        assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("theirs.txt"), b"theirs").unwrap();
        let mut case = Case {
            git: BTreeMap::new(),
            bolt: BTreeMap::new(),
            beside: BTreeMap::new(),
            env,
            outside,
        };
        case.look_around();
        case
    }

    fn root(&self) -> &Path {
        self.env.root()
    }

    /// Notes what `.git`, `.bolt` and the folder beside hold now.
    fn look_around(&mut self) {
        self.git = seen(&self.root().join(".git"));
        self.bolt = seen(&self.root().join(".bolt"));
        self.beside = seen(self.outside.path());
    }

    /// The folder beside the project, with `/` between names, as a link target.
    fn outside_target(&self) -> String {
        self.outside.path().to_string_lossy().replace('\\', "/")
    }

    /// Plants a save on `main` holding `entries` (path, kind, bytes).
    fn plant(&self, entries: &[(&str, EntryKind, &[u8])]) {
        plant_save(self.root(), |pack| listings(pack, entries));
    }

    /// `bolt goto main --json`: its exit code, its document and what it printed to stderr.
    fn goto_main(&self) -> (Option<i32>, serde_json::Value, String) {
        let out = self
            .env
            .bolt()
            .args(["goto", "main", "--json"])
            .output()
            .unwrap();
        let doc = serde_json::from_slice(&out.stdout)
            .unwrap_or_else(|e| panic!("{e}: {}", text(&out.stdout)));
        (out.status.code(), doc, text(&out.stderr))
    }

    /// Fails unless `.git` and the folder beside are as they were, and `.bolt` changed only
    /// where Bolt keeps its state, holding no planted name or contents.
    fn assert_nothing_escaped(&self) {
        assert_eq!(seen(self.outside.path()), self.beside, "beside the project");
        assert_eq!(seen(&self.root().join(".git")), self.git, "in .git");
        for (path, what) in seen(&self.root().join(".bolt")) {
            assert!(!path.contains("pwned"), ".bolt/{path}");
            if self.bolt.get(&path) == Some(&what) {
                continue;
            }
            let top = path.split('/').next().unwrap();
            assert!(
                ["state", "store", "cache"].contains(&top),
                "new in .bolt: {path}"
            );
            if let Seen::File(bytes) = what
                && top != "store"
            {
                assert!(!holds(&bytes, PAYLOAD), ".bolt/{path} holds the payload");
            }
        }
    }
}

fn holds(bytes: &[u8], wanted: &[u8]) -> bool {
    bytes.windows(wanted.len()).any(|w| w == wanted)
}

/// Stores the folder listings that hold `entries` and returns the top one's ID.
fn listings(pack: &mut PackSet, entries: &[(&str, EntryKind, &[u8])]) -> Id {
    let mut folders: BTreeMap<String, Vec<Entry>> = BTreeMap::from([(String::new(), vec![])]);
    for &(path, kind, bytes) in entries {
        let (up, name) = path.rsplit_once('/').unwrap_or(("", path));
        folders.entry(up.to_owned()).or_default().push(Entry {
            name: name.to_owned(),
            kind,
            id: pack.add_content(bytes).unwrap(),
            size: bytes.len() as u64,
        });
        // Every folder above, even one that holds only folders.
        let mut at = up;
        while let Some((above, _)) = at.rsplit_once('/') {
            folders.entry(above.to_owned()).or_default();
            at = above;
        }
        folders.entry(at.to_owned()).or_default();
    }
    // Longest path first, so a folder's listing is stored before the folder that holds it.
    loop {
        let path = folders.keys().max_by_key(|p| p.len()).cloned().unwrap();
        let entries = folders.remove(&path).unwrap();
        let id = pack
            .add_record(
                ObjectKind::FolderListing,
                &Listing::new(entries).unwrap().encode(),
            )
            .unwrap();
        if path.is_empty() {
            return id;
        }
        let (up, name) = path.rsplit_once('/').unwrap_or(("", &path));
        let folder = Entry {
            name: name.to_owned(),
            kind: EntryKind::Folder,
            id,
            size: 0,
        };
        let held = folders.entry(up.to_owned()).or_default();
        if !held.iter().any(|e| e.name == folder.name) {
            held.push(folder);
        }
    }
}

fn strings(v: &serde_json::Value) -> Vec<String> {
    v.as_array()
        .unwrap_or_else(|| panic!("not a list: {v}"))
        .iter()
        .map(|s| s.as_str().unwrap().to_owned())
        .collect()
}

fn kinds(doc: &serde_json::Value) -> Vec<String> {
    doc["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| w["kind"].as_str().unwrap().to_owned())
        .collect()
}

/// The message of the warning of kind `kind`.
fn message<'a>(doc: &'a serde_json::Value, kind: &str) -> &'a str {
    doc["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|w| w["kind"] == kind)
        .and_then(|w| w["message"].as_str())
        .unwrap_or_else(|| panic!("no {kind} warning: {doc}"))
}

fn sorted(paths: &[&str]) -> Vec<String> {
    let mut paths: Vec<String> = paths.iter().map(|p| (*p).to_owned()).collect();
    paths.sort_unstable();
    paths
}

/// Whether this drive takes names in other letter case for one, as `bolt` probes it.
fn ignores_case(root: &Path) -> bool {
    let probe = root.join("Case-Probe");
    std::fs::write(&probe, b"").unwrap();
    let blind = root.join("case-probe").exists();
    std::fs::remove_file(probe).unwrap();
    blind
}

/// Says a link that should have been made here is a placeholder because this account can't make
/// links (Windows without Developer Mode). That fails in CI, whose runners can, so a test that
/// proves nothing there can't pass for one that ran.
fn no_link_right_here(path: &str) {
    assert!(
        cfg!(windows) && std::env::var_os("CI").is_none(),
        "{path} should be a link: CI must be able to make links, so the link checks run there"
    );
    println!("Skipped the link checks: this account can't make links, so {path} is a file.");
}

// `.git` and `.bolt` spellings (T2).

#[test]
fn spellings_of_git_in_a_save_are_never_written() {
    let case = Case::new();
    let mut entries: Vec<(&str, EntryKind, &[u8])> = [
        ".git/hooks/post-checkout",
        ".GIT/hooks/pwned",
        "git~1/hooks/pwned",
        "GIT~1/pwned",
        ".git. /pwned",
        ".git::$INDEX_ALLOCATION/pwned",
        ".g\u{200c}it/hooks/pwned",
        "sub/.Git/hooks/pwned",
        // `.gitmodules`' hashed short name, refused for every entry, not just links.
        "gi7eba~1",
        "sub/gi7eb~12",
    ]
    .iter()
    .map(|p| (*p, File, PAYLOAD))
    .collect();
    entries.push(("ok.txt", File, b"fine"));
    case.plant(&entries);
    let (code, doc, _) = case.goto_main();
    assert_eq!(code, Some(0), "{doc}");
    let refused = sorted(&[
        ".git",
        ".GIT",
        "git~1",
        "GIT~1",
        ".git. ",
        ".git::$INDEX_ALLOCATION",
        ".g\u{200c}it",
        "sub/.Git",
        "gi7eba~1",
        "sub/gi7eb~12",
    ]);
    assert_eq!(strings(&doc["not_written"]), refused, "{doc}");
    assert_eq!(kinds(&doc), ["not_written_here"], "{doc}");
    let said = message(&doc, "not_written_here");
    assert!(
        said.starts_with("Left 10 entries unwritten: this computer can't hold them as they are")
            && said.contains("a spelling of .git or .bolt"),
        "{said}"
    );
    assert_eq!(
        project_entries(&case.env),
        BTreeMap::from([
            ("ok.txt".into(), file(b"fine")),
            ("sub".into(), Seen::Folder)
        ])
    );
    case.assert_nothing_escaped();
}

#[test]
fn the_short_name_of_gitmodules_is_never_written() {
    let case = Case::new();
    case.plant(&[
        (".gitmodules", File, b"[submodule \"x\"]\n"),
        ("gitmod~1", File, PAYLOAD),
        ("sub/GITMOD~4", File, PAYLOAD),
    ]);
    let (code, doc, _) = case.goto_main();
    assert_eq!(code, Some(0), "{doc}");
    assert_eq!(
        strings(&doc["not_written"]),
        ["gitmod~1", "sub/GITMOD~4"],
        "{doc}"
    );
    // Refused by name, not found in the way of `.gitmodules` on the drive.
    assert_eq!(kinds(&doc), ["not_written_here"], "{doc}");
    assert!(
        message(&doc, "not_written_here").contains("a spelling of .git or .bolt"),
        "{doc}"
    );
    assert_eq!(
        project_entries(&case.env),
        BTreeMap::from([
            (".gitmodules".into(), file(b"[submodule \"x\"]\n")),
            ("sub".into(), Seen::Folder)
        ])
    );
    case.assert_nothing_escaped();
}

#[test]
fn a_save_naming_a_spelling_of_bolt_is_refused_whole() {
    let case = Case::new();
    // No listing Bolt makes holds `.BOLT`, so it is spelled `-BOLT`, then the bytes are changed.
    plant_save(case.root(), |pack| {
        let inner = listings(pack, &[("pwned", File, PAYLOAD)]);
        let top = Listing::new(vec![Entry {
            name: "-BOLT".into(),
            kind: EntryKind::Folder,
            id: inner,
            size: 0,
        }])
        .unwrap()
        .encode();
        let at = top.windows(5).position(|w| w == b"-BOLT").unwrap();
        let mut hostile = top.clone();
        hostile[at] = b'.';
        pack.add_record(ObjectKind::FolderListing, &hostile)
            .unwrap()
    });
    let (code, doc, _) = case.goto_main();
    assert_eq!(code, Some(1), "{doc}");
    let said = doc["error"]["message"].as_str().unwrap();
    assert_eq!(
        said, "invalid name: a name cannot be spelled like Bolt's own folder (.bolt)",
        "{doc}"
    );
    assert!(project_entries(&case.env).is_empty());
    case.assert_nothing_escaped();
    assert!(!case.root().join(".bolt/state/journal").exists());
}

/// Writes `actions` as the journal of a switch from where the project is to the diary entry
/// before it, which the next command that changes the project carries out. Written as bytes,
/// as a hostile `.bolt` can hold any.
fn plant_journal(root: &Path, actions: &[(&str, Change)]) {
    let repo = Repository::open(root).unwrap();
    let (head, entry) = repo.current().unwrap();
    let mut actions: Vec<Action> = actions
        .iter()
        .map(|&(path, change)| Action {
            path: path.to_owned(),
            change,
            expected: None,
        })
        .collect();
    actions.sort_by(|a, b| a.path.cmp(&b.path));
    let journal = Journal {
        target: entry.parent.unwrap(),
        from: head,
        actions,
    };
    std::fs::write(root.join(".bolt/state/journal"), journal.encode()).unwrap();
}

/// The payload, stored in the project's store, as a journal names contents.
fn stored_payload(root: &Path) -> Content {
    let mut repo = Repository::open(root).unwrap();
    let lock = repo.lock_for_change("save", &mut |_| {}).unwrap();
    let mut pack = repo.new_packs().unwrap();
    let id = pack.add_content(PAYLOAD).unwrap();
    let (_, entry) = repo.current().unwrap();
    repo.commit(&lock, pack, &entry).unwrap();
    Content {
        id,
        size: PAYLOAD.len() as u64,
    }
}

#[test]
fn a_journal_naming_a_spelling_of_bolt_is_never_carried_out() {
    let mut case = Case::new();
    let c = stored_payload(case.root());
    plant_journal(
        case.root(),
        &[
            (".BOLT", Change::MakeFolder),
            (".BOLT/pwned", Change::File(c)),
            (".bolt/tmp/pwned", Change::File(c)),
            ("bolt~1/pwned", Change::File(c)),
            (".bolt./pwned", Change::File(c)),
        ],
    );
    case.look_around();
    let (code, doc, _) = case.goto_main();
    assert_eq!(code, Some(1), "{doc}");
    let said = doc["error"]["message"].as_str().unwrap();
    assert!(
        said.contains("switch") && said.contains("can't be read"),
        "{said}"
    );
    assert!(project_entries(&case.env).is_empty());
    case.assert_nothing_escaped();
}

/// The 8.3 short name Windows gave the top folder's `.git`, if the drive gives them.
#[cfg(windows)]
fn short_name_of_git(root: &Path) -> Option<String> {
    Dir::open_top(root)
        .unwrap()
        .entries()
        .unwrap()
        .into_iter()
        .find(|e| e.name == ".git")
        .and_then(|e| e.short_name)
        .map(|s| s.to_string_lossy().into_owned())
}

#[test]
fn a_journal_naming_git_or_its_real_short_name_writes_nothing_there() {
    // On Windows these take `GIT~1` to `GIT~4`, so `.git` gets a hashed short name, such as
    // `GI2837~1`, which no rule on names knows.
    let mut case = Case::with(|root| {
        if cfg!(windows) {
            for taken in ["g it", "gi t", "g i t", "g  it"] {
                std::fs::create_dir(root.join(taken)).unwrap();
            }
        }
    });
    #[cfg(windows)]
    let hashed = short_name_of_git(case.root());
    #[cfg(not(windows))]
    let hashed: Option<String> = None;
    if cfg!(windows) && hashed.is_none() {
        println!("Skipped the hashed short name: this drive gives no 8.3 names.");
    }
    let c = stored_payload(case.root());
    let mut paths = vec![
        ".git/hooks/pwned".to_owned(),
        ".GIT/hooks/pwned".to_owned(),
        "git~1/hooks/pwned".to_owned(),
        "sub/.git".to_owned(),
    ];
    if let Some(short) = &hashed {
        assert!(!short.eq_ignore_ascii_case("git~1"), "{short}");
        // Nothing about the name alone gives it away.
        assert_eq!(bolt_core::names::writable_here(short, &[]), Ok(()));
        paths.push(format!("{short}/hooks/pwned"));
        paths.push(format!("{}/config", short.to_lowercase()));
    }
    let actions: Vec<(&str, Change)> = paths
        .iter()
        .map(|p| (p.as_str(), Change::File(c)))
        .collect();
    plant_journal(case.root(), &actions);
    case.look_around();
    let (code, doc, _) = case.goto_main();
    assert_eq!(code, Some(0), "{doc}");
    let mut refused = sorted(&[".git", ".GIT", "git~1", "sub/.git"]);
    if let Some(short) = &hashed {
        refused.push(short.clone());
        refused.push(short.to_lowercase());
        refused.sort_unstable();
    }
    assert_eq!(strings(&doc["not_written"]), refused, "{doc}");
    assert_eq!(
        kinds(&doc),
        ["not_written_here", "switch_finished"],
        "{doc}"
    );
    assert!(
        message(&doc, "switch_finished").contains("wrote 0 files"),
        "{doc}"
    );
    let left = project_entries(&case.env);
    let left: Vec<&String> = left.keys().collect();
    let made: Vec<&str> = if cfg!(windows) {
        vec!["g  it", "g i t", "g it", "gi t"]
    } else {
        vec![]
    };
    assert_eq!(left, made);
    case.assert_nothing_escaped();
}

// Names Windows reads as something else (T1, T5).

#[test]
fn backslashes_and_drive_letters_in_names_never_reach_windows() {
    let case = Case::new();
    let names = [
        "..\\..\\pwned",
        "a\\b/pwned",
        "C:pwned",
        "x:y/pwned",
        "\\\\server\\share",
    ];
    let mut entries: Vec<(&str, EntryKind, &[u8])> =
        names.iter().map(|p| (*p, File, PAYLOAD)).collect();
    entries.push(("ok.txt", File, b"fine"));
    case.plant(&entries);
    let (code, doc, _) = case.goto_main();
    assert_eq!(code, Some(0), "{doc}");
    let mut wanted = BTreeMap::from([("ok.txt".to_owned(), file(b"fine"))]);
    if cfg!(windows) {
        assert_eq!(
            strings(&doc["not_written"]),
            sorted(&[
                "..\\..\\pwned",
                "a\\b",
                "C:pwned",
                "x:y",
                "\\\\server\\share"
            ]),
            "{doc}"
        );
        assert_eq!(kinds(&doc), ["not_written_here"], "{doc}");
    } else {
        // Elsewhere they are ordinary names.
        assert!(strings(&doc["not_written"]).is_empty(), "{doc}");
        for name in names {
            wanted.insert(name.to_owned(), file(PAYLOAD));
        }
        wanted.insert("a\\b".into(), Seen::Folder);
        wanted.insert("x:y".into(), Seen::Folder);
    }
    assert_eq!(project_entries(&case.env), wanted);
    case.assert_nothing_escaped();
}

#[test]
fn device_names_are_never_written_on_windows() {
    let case = Case::new();
    let devices = [
        "CON",
        "nul.txt",
        "aux/pwned",
        "CONIN$",
        "COM\u{b9}",
        "lpt\u{b2}.log",
        "COM\u{b3}",
        "COM9",
    ];
    let mut entries: Vec<(&str, EntryKind, &[u8])> =
        devices.iter().map(|p| (*p, File, PAYLOAD)).collect();
    // Not on Microsoft's list, so ordinary names.
    entries.push(("COM0", File, b"fine"));
    entries.push(("LPT0", File, b"fine"));
    case.plant(&entries);
    let (code, doc, _) = case.goto_main();
    assert_eq!(code, Some(0), "{doc}");
    let mut wanted = BTreeMap::from([
        ("COM0".to_owned(), file(b"fine")),
        ("LPT0".to_owned(), file(b"fine")),
    ]);
    if cfg!(windows) {
        assert_eq!(
            strings(&doc["not_written"]),
            sorted(&[
                "CON",
                "nul.txt",
                "aux",
                "CONIN$",
                "COM\u{b9}",
                "lpt\u{b2}.log",
                "COM\u{b3}",
                "COM9"
            ]),
            "{doc}"
        );
        assert!(
            message(&doc, "not_written_here").contains("a name it can't make"),
            "{doc}"
        );
    } else {
        assert!(strings(&doc["not_written"]).is_empty(), "{doc}");
        for name in devices {
            wanted.insert(name.to_owned(), file(PAYLOAD));
        }
        wanted.insert("aux".into(), Seen::Folder);
    }
    assert_eq!(project_entries(&case.env), wanted);
    case.assert_nothing_escaped();
}

// Links (T3, T4).

#[test]
fn a_link_named_like_a_folder_cant_lead_the_files_below_it_elsewhere() {
    // CVE-2021-21300 and CVE-2024-32002: on a drive that ignores case, `a/post-checkout` would go
    // through the link `A`.
    let case = Case::new();
    let outside = case.outside_target();
    case.plant(&[
        ("A", Symlink, b".git/hooks"),
        ("a/post-checkout", Executable, PAYLOAD),
        ("B", Symlink, outside.as_bytes()),
        ("b/pwned", File, PAYLOAD),
        ("C", Symlink, b"d"),
        ("c/pwned", File, PAYLOAD),
        ("d/keep.txt", File, b"fine"),
    ]);
    let (code, doc, _) = case.goto_main();
    assert_eq!(code, Some(0), "{doc}");
    let mut wanted = BTreeMap::from([
        ("d".to_owned(), Seen::Folder),
        ("d/keep.txt".to_owned(), file(b"fine")),
    ]);
    if ignores_case(case.root()) {
        assert_eq!(
            strings(&doc["not_written"]),
            ["A", "B", "C", "a", "b", "c"],
            "{doc}"
        );
        assert!(strings(&doc["placeholders"]).is_empty(), "{doc}");
        assert_eq!(kinds(&doc), ["not_written_here"], "{doc}");
        assert!(
            message(&doc, "not_written_here").contains("two names that are one file"),
            "{doc}"
        );
    } else {
        // Both links lead out of the project, so each is a file holding its target.
        assert!(strings(&doc["not_written"]).is_empty(), "{doc}");
        assert_eq!(strings(&doc["placeholders"]), ["A", "B"], "{doc}");
        wanted.insert("A".into(), file(b".git/hooks"));
        wanted.insert("B".into(), file(outside.as_bytes()));
        wanted.insert("C".into(), Seen::Link("d".into()));
        for folder in ["a", "b", "c"] {
            wanted.insert(folder.into(), Seen::Folder);
        }
        wanted.insert("a/post-checkout".into(), file(PAYLOAD));
        wanted.insert("b/pwned".into(), file(PAYLOAD));
        wanted.insert("c/pwned".into(), file(PAYLOAD));
    }
    assert_eq!(project_entries(&case.env), wanted);
    case.assert_nothing_escaped();
}

#[test]
fn links_that_leave_the_project_are_placeholders_never_followed() {
    // Git for Windows' NTLM leak: a link to `\\host\share` was looked at, which reaches the host.
    let case = Case::new();
    let outside = case.outside_target();
    let leaving: Vec<(&str, String)> = vec![
        ("abs", "/etc/passwd".into()),
        ("beside", outside.clone()),
        ("drive", "C:/Windows/System32".into()),
        ("relative-drive", "C:pwned".into()),
        ("unc", "//bolt-test.invalid/share/pwned".into()),
        ("unc-back", "\\\\bolt-test.invalid\\share\\pwned".into()),
        ("up", "../pwned".into()),
        ("deep/up", "../../pwned".into()),
        ("deep/through", "../ok/../../pwned".into()),
        ("dot-dot", "..".into()),
    ];
    let mut entries: Vec<(&str, EntryKind, &[u8])> = leaving
        .iter()
        .map(|(path, target)| (*path, Symlink, target.as_bytes()))
        .collect();
    entries.push(("deep/inside.txt", File, b"fine"));
    entries.push(("ok", Symlink, b"deep/inside.txt"));
    case.plant(&entries);
    let (code, doc, _) = case.goto_main();
    assert_eq!(code, Some(0), "{doc}");
    let mut wanted = BTreeMap::from([
        ("deep".to_owned(), Seen::Folder),
        ("deep/inside.txt".to_owned(), file(b"fine")),
    ]);
    for (path, target) in &leaving {
        wanted.insert((*path).to_owned(), file(target.as_bytes()));
    }
    let mut placeholders: Vec<&str> = leaving.iter().map(|(p, _)| *p).collect();
    let ok = std::fs::symlink_metadata(case.root().join("ok")).unwrap();
    if ok.file_type().is_symlink() {
        let back = if cfg!(windows) {
            "deep\\inside.txt"
        } else {
            "deep/inside.txt"
        };
        wanted.insert("ok".into(), Seen::Link(back.into()));
    } else {
        no_link_right_here("ok");
        wanted.insert("ok".into(), file(b"deep/inside.txt"));
        placeholders.push("ok");
    }
    placeholders.sort_unstable();
    assert_eq!(strings(&doc["placeholders"]), placeholders, "{doc}");
    assert_eq!(kinds(&doc), ["placeholders_written"], "{doc}");
    assert!(
        message(&doc, "placeholders_written").contains("they lead out of the project"),
        "{doc}"
    );
    assert_eq!(project_entries(&case.env), wanted);
    case.assert_nothing_escaped();
}

// Modes (gitoxide CVE-2025-22620).

#[cfg(unix)]
#[test]
fn modes_are_exactly_0644_and_0755_whatever_the_umask() {
    use std::os::unix::fs::PermissionsExt;
    let case = Case::new();
    case.plant(&[
        ("plain", File, b"p"),
        ("tool", Executable, b"t"),
        ("deep/tool", Executable, b"t2"),
    ]);
    // With no umask to narrow them, any mode Bolt doesn't set itself shows.
    let bolt = case.env.bolt_process_in(case.root());
    let mut cmd = std::process::Command::new("sh");
    cmd.arg("-c")
        .arg("umask 000; exec \"$0\" \"$@\"")
        .arg(bolt.get_program())
        .args(["goto", "main", "--json"])
        .current_dir(case.root());
    for (key, value) in bolt.get_envs() {
        match value {
            Some(value) => cmd.env(key, value),
            None => cmd.env_remove(key),
        };
    }
    let out = cmd.output().unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    let mode = |path: &str| {
        std::fs::symlink_metadata(case.root().join(path))
            .unwrap()
            .permissions()
            .mode()
            & 0o7777
    };
    assert_eq!(
        (mode("plain"), mode("tool"), mode("deep/tool")),
        (0o644, 0o755, 0o755)
    );
    case.assert_nothing_escaped();
}

// No folder checked once and trusted after (gitoxide, 2026).

/// Makes `link` lead to the folder `target`: a symlink on Unix, a junction on Windows, which
/// needs no rights.
#[cfg(feature = "fault-injection")]
fn folder_link(target: &Path, link: &Path) {
    #[cfg(unix)]
    std::os::unix::fs::symlink(target, link).unwrap();
    #[cfg(windows)]
    support::junction(link, target);
}

#[cfg(feature = "fault-injection")]
#[test]
fn a_folder_made_in_a_stopped_switch_is_checked_again_when_it_finishes() {
    let mut case = Case::new();
    case.plant(&[("a/pwned", File, PAYLOAD), ("a/b/pwned", File, PAYLOAD)]);
    let out = case
        .env
        .bolt()
        .args(["goto", "main"])
        .env("BOLT_FAULT_FAIL", "switch:changed")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1), "{}", text(&out.stdout));
    assert!(case.root().join(".bolt/state/journal").exists());
    // The switch made `a` and stopped; now it leads out of the project.
    assert!(case.root().join("a").is_dir());
    std::fs::remove_dir(case.root().join("a")).unwrap();
    folder_link(case.outside.path(), &case.root().join("a"));
    case.look_around();
    let (code, doc, _) = case.goto_main();
    assert_eq!(code, Some(0), "{doc}");
    assert_eq!(
        kinds(&doc),
        ["changed_since_plan", "switch_finished"],
        "{doc}"
    );
    assert!(
        message(&doc, "changed_since_plan").contains("changed while Bolt was switching"),
        "{doc}"
    );
    let a = std::fs::symlink_metadata(case.root().join("a")).unwrap();
    assert!(a.file_type().is_symlink(), "{a:?}");
    case.assert_nothing_escaped();
}

// Lengths (decision 051).

#[test]
fn names_longer_than_this_system_holds_are_left_unwritten() {
    let case = Case::new();
    let longest = "n".repeat(255);
    let too_long = "n".repeat(256);
    // 400 bytes but 200 UTF-16 units: too long for macOS and Linux, not for Windows.
    let wide = "\u{e9}".repeat(200);
    let folder = format!("{too_long}/pwned");
    case.plant(&[
        (&longest, File, b"fine"),
        (&too_long, File, PAYLOAD),
        (&folder, File, PAYLOAD),
        (&wide, File, PAYLOAD),
    ]);
    let (code, doc, _) = case.goto_main();
    assert_eq!(code, Some(0), "{doc}");
    let mut wanted = BTreeMap::from([(longest.clone(), file(b"fine"))]);
    let mut refused = vec![too_long.clone()];
    if cfg!(windows) {
        wanted.insert(wide.clone(), file(PAYLOAD));
    } else {
        refused.push(wide.clone());
    }
    refused.sort_unstable();
    assert_eq!(strings(&doc["not_written"]), refused, "{doc}");
    assert_eq!(kinds(&doc), ["not_written_here"], "{doc}");
    assert_eq!(project_entries(&case.env), wanted);
    case.assert_nothing_escaped();
}

/// Goes down `names` from the project's top a folder at a time, as macOS, which refuses long
/// paths, still allows; returns the last folder.
fn down(root: &Path, names: impl IntoIterator<Item = String>) -> Dir {
    let mut dir = Dir::open_top(root).unwrap();
    for name in names {
        dir = dir
            .open_dir(OsStr::new(&name))
            .unwrap_or_else(|e| panic!("{name}: {e}"));
    }
    dir
}

/// The names in `dir`, sorted.
fn names(dir: &Dir) -> Vec<String> {
    let mut names: Vec<String> = dir
        .entries()
        .unwrap()
        .into_iter()
        .map(|e| e.name.to_string_lossy().into_owned())
        .collect();
    names.sort_unstable();
    names
}

/// Fails unless `bolt goto main` refused the save as too large to read, and changed nothing.
fn assert_refused_as_too_large(case: &Case) {
    let (code, doc, _) = case.goto_main();
    assert_eq!(code, Some(1), "{doc}");
    let said = doc["error"]["message"].as_str().unwrap();
    assert!(
        said.contains("too large to read safely: a path in it is longer than 4096 bytes"),
        "{doc}"
    );
    assert!(
        doc["error"]["hint"]
            .as_str()
            .unwrap()
            .contains("Nothing was changed"),
        "{doc}"
    );
    assert!(project_entries(&case.env).is_empty());
    assert!(!case.root().join(".bolt/state/journal").exists());
    case.assert_nothing_escaped();
}

#[test]
fn a_path_of_4096_bytes_is_written_and_a_save_with_a_longer_one_is_refused() {
    // 16 folders of 250 bytes make 4,015 bytes; a name of 80 more ends at 4,096 exactly.
    let folders = vec!["d".repeat(250); 16];
    let up = folders.join("/");
    let fits = format!("{up}/{}", "f".repeat(80));
    let over = format!("{up}/{}", "g".repeat(81));
    assert_eq!((fits.len(), over.len()), (4096, 4097));

    let case = Case::new();
    case.plant(&[(&fits, File, b"fine")]);
    let (code, doc, _) = case.goto_main();
    assert_eq!(code, Some(0), "{doc}");
    assert!(kinds(&doc).is_empty(), "{doc}");
    let deepest = down(case.root(), folders);
    assert_eq!(names(&deepest), ["f".repeat(80)]);
    case.assert_nothing_escaped();

    let case = Case::new();
    case.plant(&[(&fits, File, b"fine"), (&over, File, PAYLOAD)]);
    assert_refused_as_too_large(&case);
}

#[test]
fn a_save_4096_folders_deep_is_refused_quickly() {
    let case = Case::new();
    plant_save(case.root(), |pack| {
        let mut id = listings(pack, &[("pwned", File, PAYLOAD)]);
        for _ in 0..4096 {
            id = add_listing(pack, &["a".to_owned()], EntryKind::Folder, id);
        }
        id
    });
    // `a/a/…/a` passes 4,096 bytes 2,049 folders down, long before the limit on depth.
    let started = std::time::Instant::now();
    assert_refused_as_too_large(&case);
    assert!(started.elapsed().as_secs() < 30, "{:?}", started.elapsed());
}
