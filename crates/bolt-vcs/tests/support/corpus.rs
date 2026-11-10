//! What the hostile corpora share: a project as after an import with a folder beside it, and
//! what was written where.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use bolt_core::id::hash_content;
use tempfile::TempDir;

use super::{Env, text};

/// What every planted file holds, so a stray copy of it is easy to find.
pub const PAYLOAD: &[u8] = b"PWNED by a hostile save";

/// An entry as `seen` finds it, links never followed.
#[derive(Clone, PartialEq, Eq)]
pub enum Seen {
    Folder,
    File(Vec<u8>),
    Link(PathBuf),
}

impl std::fmt::Debug for Seen {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Seen::Folder => f.write_str("Folder"),
            // A big one by its size and ID, so a failure doesn't print megabytes.
            Seen::File(bytes) if bytes.len() > 4096 => {
                let id = hash_content(bytes).short_hex();
                write!(f, "File({} bytes, {id})", bytes.len())
            }
            Seen::File(bytes) => f.debug_tuple("File").field(bytes).finish(),
            Seen::Link(target) => f.debug_tuple("Link").field(target).finish(),
        }
    }
}

/// Everything below `dir`, by path with `/` between names; empty if there is no `dir`.
pub fn seen(dir: &Path) -> BTreeMap<String, Seen> {
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
pub fn project_entries(env: &Env) -> BTreeMap<String, Seen> {
    let mut all = seen(env.root());
    all.retain(|path, _| {
        let top = path.split('/').next().unwrap();
        top != ".git" && top != ".bolt"
    });
    all
}

pub fn file(bytes: &[u8]) -> Seen {
    Seen::File(bytes.to_vec())
}

/// A project with a `.git` folder beside `.bolt`, as after an import, at its only save but not on
/// a line, so that `bolt goto main` switches to whatever is planted on `main`; and a folder
/// beside it. `before_git` runs before `.git` is made.
pub struct Case {
    pub env: Env,
    pub outside: TempDir,
    git: BTreeMap<String, Seen>,
    bolt: BTreeMap<String, Seen>,
    beside: BTreeMap<String, Seen>,
}

impl Case {
    pub fn new() -> Case {
        Case::with(|_| {})
    }

    /// A case still on `main`, so a save planted there is the one the project is on, though the
    /// folder doesn't hold its files.
    pub fn on_main() -> Case {
        Case::make(|_| {}, false)
    }

    pub fn with(before_git: impl FnOnce(&Path)) -> Case {
        Case::make(before_git, true)
    }

    fn make(before_git: impl FnOnce(&Path), off_main: bool) -> Case {
        let env = Env::empty();
        env.bolt().arg("start").assert().success();
        before_git(env.root());
        let hooks = env.root().join(".git/hooks");
        std::fs::create_dir_all(&hooks).unwrap();
        std::fs::write(env.root().join(".git/config"), b"[core]\n").unwrap();
        std::fs::write(hooks.join("pre-commit.sample"), b"#!/bin/sh\n").unwrap();
        if off_main {
            let first = env.state().1.lines["main"].to_hex();
            let out = env.bolt().args(["goto", &first]).output().unwrap();
            assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
        }
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

    pub fn root(&self) -> &Path {
        self.env.root()
    }

    /// `bolt <args> --json` in the project, stopped if it runs a minute: its exit code, its
    /// document and what it printed to stderr.
    pub fn run_json(&self, args: &[&str]) -> (Option<i32>, serde_json::Value, String) {
        let out = self
            .env
            .bolt()
            .args(args)
            .arg("--json")
            .timeout(Duration::from_secs(60))
            .output()
            .unwrap();
        let stderr = text(&out.stderr);
        let doc = serde_json::from_slice(&out.stdout)
            .unwrap_or_else(|e| panic!("{args:?}: {e}: {}{stderr}", text(&out.stdout)));
        (out.status.code(), doc, stderr)
    }

    /// Notes what `.git`, `.bolt` and the folder beside hold now.
    pub fn look_around(&mut self) {
        self.git = seen(&self.root().join(".git"));
        self.bolt = seen(&self.root().join(".bolt"));
        self.beside = seen(self.outside.path());
    }

    /// The folder beside the project, with `/` between names, as a link target.
    pub fn outside_target(&self) -> String {
        self.outside.path().to_string_lossy().replace('\\', "/")
    }

    /// Fails unless `.git` and the folder beside are as they were, and `.bolt` changed only
    /// where Bolt keeps its state, holding no planted name or contents.
    pub fn assert_nothing_escaped(&self) {
        assert_eq!(seen(self.outside.path()), self.beside, "beside the project");
        // Where `../pwned` and `../../pwned` would land. Only their own names: these are the
        // system's temporary folders, shared with everything else.
        for up in self.root().ancestors().skip(1).take(2) {
            for e in std::fs::read_dir(up).unwrap() {
                let name = e.unwrap().file_name();
                assert!(
                    !name.to_string_lossy().contains("pwned"),
                    "{}",
                    up.join(name).display()
                );
            }
        }
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

pub fn holds(bytes: &[u8], wanted: &[u8]) -> bool {
    bytes.windows(wanted.len()).any(|w| w == wanted)
}

pub fn strings(v: &serde_json::Value) -> Vec<String> {
    v.as_array()
        .unwrap_or_else(|| panic!("not a list: {v}"))
        .iter()
        .map(|s| s.as_str().unwrap().to_owned())
        .collect()
}

pub fn kinds(doc: &serde_json::Value) -> Vec<String> {
    doc["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| w["kind"].as_str().unwrap().to_owned())
        .collect()
}

/// The message of the warning of kind `kind`.
pub fn message<'a>(doc: &'a serde_json::Value, kind: &str) -> &'a str {
    doc["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|w| w["kind"] == kind)
        .and_then(|w| w["message"].as_str())
        .unwrap_or_else(|| panic!("no {kind} warning: {doc}"))
}

pub fn sorted(paths: &[&str]) -> Vec<String> {
    let mut paths: Vec<String> = paths.iter().map(|p| (*p).to_owned()).collect();
    paths.sort_unstable();
    paths
}

/// Says a link that should have been made here is a placeholder because this account can't make
/// links (Windows without Developer Mode). That fails in CI, whose runners can, so a test that
/// proves nothing there can't pass for one that ran.
pub fn no_link_right_here(path: &str) {
    assert!(
        cfg!(windows) && std::env::var_os("CI").is_none(),
        "{path} should be a link: CI must be able to make links, so the link checks run there"
    );
    println!("Skipped the link checks: this account can't make links, so {path} is a file.");
}

/// Whether this drive takes names in other letter case for one, as `bolt` probes it.
pub fn ignores_case(root: &Path) -> bool {
    let probe = root.join("Case-Probe");
    std::fs::write(&probe, b"").unwrap();
    let blind = root.join("case-probe").exists();
    std::fs::remove_file(probe).unwrap();
    blind
}
