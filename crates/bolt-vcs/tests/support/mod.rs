//! Helpers the integration tests share.
// Each test file uses only some of them.
#![allow(dead_code)]

pub mod corpus;
mod vs_git;

#[allow(unused_imports)]
pub use vs_git::{assert_matches_git, assert_matches_git_counting};

use std::collections::BTreeMap;
use std::io::{self, Write};
use std::path::Path;
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

use assert_cmd::Command as Bolt;
use bolt_core::diary::{Action, DiaryEntry, State};
use bolt_core::history::current_save;
use bolt_core::id::{ChangeId, Id};
use bolt_core::listing::{Entry, EntryKind, Listing};
use bolt_core::pack::{ObjectKind, PackSet};
use bolt_core::repo::Repository;
use bolt_core::save::{EXTRA_GIT_COMMIT, Save};
use tempfile::TempDir;

/// `git args` in `dir`, with nothing from this computer's Git set-up: no system or user config,
/// no templates, no replace objects, and messages in plain C. `home` stands in for the user's
/// home folder, and must last as long as Git runs.
fn git_command(dir: &Path, home: &Path, args: &[&str]) -> Command {
    let empty_config = home.join("gitconfig");
    std::fs::write(&empty_config, b"").unwrap();
    let templates = home.join("templates");
    std::fs::create_dir_all(&templates).unwrap();
    let mut cmd = Command::new("git");
    cmd.current_dir(dir).env_clear();
    for key in ["PATH", "SystemRoot", "TEMP", "TMP"] {
        if let Some(value) = std::env::var_os(key) {
            cmd.env(key, value);
        }
    }
    cmd.env("HOME", home);
    if cfg!(windows) {
        cmd.env("USERPROFILE", home);
    }
    cmd.env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", &empty_config)
        .env("GIT_TEMPLATE_DIR", &templates)
        .env("GIT_NO_REPLACE_OBJECTS", "1")
        .env("LC_ALL", "C")
        .args(["-c", "core.autocrlf=false", "-c", "user.name=Test"])
        .args([
            "-c",
            "user.email=test@example.com",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(["-c", "init.defaultBranch=main"])
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    cmd
}

/// Starts `cmd`, a Git command. A test that needs Git fails, saying so, where Git isn't
/// installed.
fn spawn(cmd: &mut Command) -> Child {
    match cmd.spawn() {
        Ok(child) => child,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            panic!("this test compares Bolt with Git, and git isn't installed or isn't on PATH")
        }
        Err(e) => panic!("can't run git: {e}"),
    }
}

/// Runs `git args` in `dir` with `input` on its standard input, and nothing from this computer's
/// Git set-up (`git_command`).
pub fn git_with_input(dir: &Path, args: &[&str], input: &[u8]) -> Output {
    let home = tempfile::tempdir().unwrap();
    let mut child = spawn(&mut git_command(dir, home.path(), args));
    let mut stdin = child.stdin.take().unwrap();
    let input = input.to_vec();
    // Fed from a thread of its own, so a full output pipe can't stall Git while it reads.
    let feeder = std::thread::spawn(move || stdin.write_all(&input));
    let out = child.wait_with_output().unwrap();
    // A Git that stopped early shows that in its status and messages, not here.
    let _ = feeder.join();
    out
}

/// Runs `git args` in `dir` as `git_with_input` does, and returns what it printed. The test fails
/// if Git does.
pub fn git(dir: &Path, args: &[&str]) -> Vec<u8> {
    let out = git_with_input(dir, args, b"");
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    out.stdout
}

/// Makes `link` a junction to the folder `target`, which needs no rights on Windows.
#[cfg(windows)]
pub fn junction(link: &Path, target: &Path) {
    let made = Command::new("cmd")
        .args(["/D", "/C", "mklink", "/J"])
        .arg(link)
        .arg(target)
        .stdout(Stdio::null())
        .status()
        .unwrap();
    assert!(
        made.success(),
        "mklink /J {} {} failed",
        link.display(),
        target.display()
    );
}

pub fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// The Git commit a save was imported from.
pub fn git_commit(repo: &Repository, save: &Id) -> String {
    let save = repo.store().save(save, false).unwrap();
    String::from_utf8(save.extra[EXTRA_GIT_COMMIT].clone()).unwrap()
}

/// The top folder of the save the project in `dir` is on.
pub fn top_folder(dir: &Path) -> Listing {
    let repo = Repository::open(dir).unwrap();
    let save = repo
        .store()
        .save(&current_save(&repo).unwrap(), false)
        .unwrap();
    repo.store().listing(&save.root).unwrap()
}

/// Files by path, with what each holds.
pub type Files = BTreeMap<String, Vec<u8>>;

/// Every file below `dir` but `.bolt` and `.git` at the top, by path, with what it holds.
pub fn files(dir: &Path) -> Files {
    let mut found = BTreeMap::new();
    let mut folders = vec![(dir.to_path_buf(), String::new())];
    while let Some((at, rel)) = folders.pop() {
        for e in std::fs::read_dir(&at).unwrap() {
            let e = e.unwrap();
            let name = e.file_name().to_string_lossy().into_owned();
            if rel.is_empty() && (name == ".bolt" || name == ".git") {
                continue;
            }
            let path = if rel.is_empty() {
                name
            } else {
                format!("{rel}/{name}")
            };
            if e.file_type().unwrap().is_dir() {
                folders.push((e.path(), path));
            } else {
                found.insert(path, std::fs::read(e.path()).unwrap());
            }
        }
    }
    found
}

pub fn holding(entries: &[(&str, &str)]) -> Files {
    entries
        .iter()
        .map(|(p, b)| ((*p).to_owned(), b.as_bytes().to_vec()))
        .collect()
}

/// Writes the file `path` of the project, and the folders above it.
pub fn write(env: &Env, path: &str, bytes: impl AsRef<[u8]>) {
    let at = env.root().join(path);
    std::fs::create_dir_all(at.parent().unwrap()).unwrap();
    std::fs::write(at, bytes).unwrap();
}

/// What `bolt status` finds changed.
pub fn changed(env: &Env) -> Vec<String> {
    let out = env.bolt().args(["status", "--json"]).output().unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    let doc: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    doc["changes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["path"].as_str().unwrap().to_owned())
        .collect()
}

pub fn main_tip(env: &Env) -> Id {
    env.state().1.lines["main"]
}

/// The newest diary entry, with its ID.
pub fn head(env: &Env) -> (Id, DiaryEntry) {
    Repository::open(env.root()).unwrap().current().unwrap()
}

/// Every file of the save `save`, by path, with what it holds.
pub fn saved_files(env: &Env, save: Id) -> Files {
    let repo = Repository::open(env.root()).unwrap();
    let store = repo.store();
    let mut found = BTreeMap::new();
    let mut folders = vec![(store.save(&save, false).unwrap().root, String::new())];
    while let Some((id, rel)) = folders.pop() {
        for e in store.listing(&id).unwrap().entries() {
            let path = if rel.is_empty() {
                e.name.clone()
            } else {
                format!("{rel}/{}", e.name)
            };
            if e.kind == EntryKind::Folder {
                folders.push((e.id, path));
            } else {
                let mut bytes = Vec::new();
                store.copy_content(&e.id, &mut bytes).unwrap();
                found.insert(path, bytes);
            }
        }
    }
    found
}

/// Copies the folder `from`, everything in it, to `to`.
pub fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

// Saves that no real folder tree could be, planted through Bolt's own library.

/// A listing whose every entry has the same kind and points at the same object.
pub fn add_listing(pack: &mut PackSet, names: &[String], kind: EntryKind, id: Id) -> Id {
    let entries = names
        .iter()
        .map(|name| Entry {
            name: name.clone(),
            kind,
            id,
            size: 0,
        })
        .collect();
    let listing = Listing::new(entries).unwrap();
    pack.add_record(ObjectKind::FolderListing, &listing.encode())
        .unwrap()
}

/// Stores the folder listings that hold `entries` (a path, a kind, and a file's bytes or a link's
/// target) and returns the top one's ID.
pub fn listings(pack: &mut PackSet, entries: &[(&str, EntryKind, &[u8])]) -> Id {
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

/// Stores a listing holding the folder `.BOLT`, with `inside` in it. No listing Bolt makes holds
/// one, so it is spelled `-BOLT`, then the bytes are changed.
pub fn holding_bolt_spelling(pack: &mut PackSet, inside: Id) -> Id {
    let listing = Listing::new(vec![Entry {
        name: "-BOLT".into(),
        kind: EntryKind::Folder,
        id: inside,
        size: 0,
    }])
    .unwrap()
    .encode();
    let at = listing.windows(5).position(|w| w == b"-BOLT").unwrap();
    let mut hostile = listing;
    hostile[at] = b'.';
    pack.add_record(ObjectKind::FolderListing, &hostile)
        .unwrap()
}

/// Stores a top listing holding `files` and a `.git` folder that holds a spelling of `.bolt`, and
/// returns its ID. A folder walk never reads `.git`, so the bad listing is first met comparing
/// the save with the folder, or with another save.
pub fn with_bolt_spelling_under_git(pack: &mut PackSet, files: &[(&str, &[u8])]) -> Id {
    let inside = listings(pack, &[("pwned", EntryKind::File, b"PWNED")]);
    let git = holding_bolt_spelling(pack, inside);
    let mut top: Vec<Entry> = files
        .iter()
        .map(|&(name, bytes)| Entry {
            name: name.into(),
            kind: EntryKind::File,
            id: pack.add_content(bytes).unwrap(),
            size: bytes.len() as u64,
        })
        .collect();
    top.push(Entry {
        name: ".git".into(),
        kind: EntryKind::Folder,
        id: git,
        size: 0,
    });
    let top = Listing::new(top).unwrap().encode();
    pack.add_record(ObjectKind::FolderListing, &top).unwrap()
}

/// A started project at its first save, not on a line, with `saves` planted on `main` one after
/// another, each holding its entries as for [`listings`]; their IDs. `bolt goto` then writes each.
pub fn planted_saves(saves: &[&[(&str, EntryKind, &[u8])]]) -> (Env, Vec<Id>) {
    let env = Env::empty();
    env.bolt().arg("start").assert().success();
    let first = main_tip(&env).to_hex();
    env.bolt().args(["goto", &first]).assert().success();
    let mut ids = Vec::new();
    for entries in saves {
        plant_save(env.root(), |pack| listings(pack, entries));
        ids.push(main_tip(&env));
    }
    (env, ids)
}

/// Stores a save of the folder whose top listing is `root`, made on the save `parent` by its
/// author; its ID.
pub fn add_save(repo: &Repository, pack: &mut PackSet, root: Id, parent: Id) -> Id {
    let made_on = repo.store().save(&parent, false).unwrap();
    let save = Save {
        root,
        parents: vec![parent],
        author: made_on.author,
        saver: made_on.saver,
        message: "Planted".into(),
        change_id: ChangeId::random().unwrap(),
        move_notes: vec![],
        signature: vec![],
        extra: made_on.extra,
    };
    pack.add_record(ObjectKind::Save, &save.encode()).unwrap()
}

/// Commits, through Bolt's own library, a save on `main` whose top folder `build` makes.
pub fn plant_save(root: &Path, build: impl FnOnce(&mut PackSet) -> Id) {
    let mut repo = Repository::open(root).unwrap();
    let lock = repo.lock_for_change("save", &mut |_| {}).unwrap();
    let (head, entry) = repo.current().unwrap();
    let parent_id = entry.state.lines["main"];
    let author = repo.store().save(&parent_id, false).unwrap().author;
    let mut pack = repo.new_packs().unwrap();
    let top = build(&mut pack);
    let save_id = add_save(&repo, &mut pack, top, parent_id);
    let mut state = entry.state.clone();
    state.lines.insert("main".into(), save_id);
    let diary = DiaryEntry {
        parent: Some(head),
        action: Action::Command,
        command: "save".into(),
        who: author,
        bolt_version: "0.1.0".into(),
        state,
        folder_change: None,
    };
    repo.commit(&lock, pack, &diary).unwrap();
}

// Commands that run under a limit: of time, or of memory.

/// `bolt <args>` in `dir`, as the test user with its settings in `config`, with its address space
/// capped at `cap_kb`, so a runaway allocation aborts instead of taking the whole machine down.
/// Where the cap isn't enforced (macOS), a test that relies on it proves nothing.
#[cfg(unix)]
pub fn bolt_capped_at(config: &Path, dir: &Path, cap_kb: u32, args: &[&str]) -> Command {
    let mut cmd = Command::new("sh");
    cmd.arg("-c")
        .arg(format!(
            "ulimit -v {cap_kb} 2>/dev/null; exec \"$0\" \"$@\""
        ))
        .arg(assert_cmd::cargo::cargo_bin("bolt"))
        .args(args)
        .current_dir(dir);
    as_test_user(&mut cmd, config);
    cmd
}

/// `bolt <args>` in `dir`, as the test user with its settings in `config`, allowed at most `most`
/// open files at once.
#[cfg(unix)]
pub fn bolt_with_open_files(config: &Path, dir: &Path, most: u32, args: &[&str]) -> Command {
    let mut cmd = Command::new("sh");
    cmd.arg("-c")
        .arg(format!("ulimit -n {most} && exec \"$0\" \"$@\""))
        .arg(assert_cmd::cargo::cargo_bin("bolt"))
        .args(args)
        .current_dir(dir);
    as_test_user(&mut cmd, config);
    cmd
}

/// Runs `cmd` as the test user, with no fault point and its own settings folder `config`.
fn as_test_user(cmd: &mut Command, config: &Path) {
    cmd.env("BOLT_USER_NAME", "Test")
        .env("BOLT_USER_EMAIL", "test@example.com")
        .env_remove("BOLT_FAULT_AT")
        .env("BOLT_CONFIG_DIR", config)
        // Git's own settings and ignore list are found from here (decision 047).
        .env("HOME", config)
        .env("XDG_CONFIG_HOME", config);
}

/// `bolt_capped_at`, with about 1.5 GB.
#[cfg(unix)]
pub fn bolt_capped(config: &Path, dir: &Path, args: &[&str]) -> Command {
    bolt_capped_at(config, dir, 1_500_000, args)
}

/// How a command run under a time limit ended.
#[derive(Debug)]
pub struct Ended {
    /// `None`: killed by a signal (crash, abort, out of memory) or by us after the time limit.
    pub code: Option<i32>,
    pub timed_out: bool,
    pub secs: f64,
    /// Highest resident memory seen (Linux `/proc` only; 0 elsewhere).
    pub peak_mb: u64,
    pub stderr: String,
}

/// The process's peak resident memory so far, in MB (Linux only).
fn peak_mb_of(pid: u32) -> u64 {
    std::fs::read_to_string(format!("/proc/{pid}/status"))
        .ok()
        .and_then(|s| {
            let line = s.lines().find(|l| l.starts_with("VmHWM:"))?.to_owned();
            line.split_whitespace().nth(1)?.parse::<u64>().ok()
        })
        .map_or(0, |kb| kb / 1024)
}

/// Runs `cmd`, killing it after `secs` seconds.
pub fn run_limited(mut cmd: Command, secs: u64) -> Ended {
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let start = Instant::now();
    let mut peak_mb = 0;
    loop {
        peak_mb = peak_mb.max(peak_mb_of(child.id()));
        if let Some(status) = child.try_wait().unwrap() {
            let mut stderr = String::new();
            std::io::Read::read_to_string(&mut child.stderr.take().unwrap(), &mut stderr).unwrap();
            // By characters: a cut inside one would panic.
            let stderr: String = stderr.chars().take(600).collect();
            return Ended {
                code: status.code(),
                timed_out: false,
                secs: start.elapsed().as_secs_f64(),
                peak_mb,
                stderr,
            };
        }
        if start.elapsed() > Duration::from_secs(secs) {
            child.kill().unwrap();
            child.wait().unwrap();
            return Ended {
                code: None,
                timed_out: true,
                secs: start.elapsed().as_secs_f64(),
                peak_mb,
                stderr: String::new(),
            };
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// A stdout every write to fails, for a reason other than its reader going: a file opened only
/// to read on Windows, the full device on Linux.
#[cfg(any(windows, target_os = "linux"))]
pub fn failing_stdout(env: &Env) -> std::fs::File {
    #[cfg(windows)]
    {
        let path = env.config().join("out");
        std::fs::write(&path, b"").unwrap();
        std::fs::File::open(path).unwrap()
    }
    #[cfg(target_os = "linux")]
    {
        let _ = env;
        std::fs::OpenOptions::new()
            .write(true)
            .open("/dev/full")
            .unwrap()
    }
}

/// A folder for the Git repository, and an empty one for the user's own Bolt settings.
pub struct Env {
    work: TempDir,
    config: TempDir,
}

impl Env {
    /// A Git repository with no commits yet, on `main`.
    pub fn new() -> Env {
        Env::init(&[])
    }

    /// A Git repository made by `git init` with `options`, with no commits yet, on `main`.
    pub fn init(options: &[&str]) -> Env {
        let env = Env::empty();
        env.git(&[&["init", "-q"], options].concat());
        env
    }

    /// An empty folder, for a repository the test writes itself.
    pub fn empty() -> Env {
        Env {
            work: tempfile::tempdir().unwrap(),
            config: tempfile::tempdir().unwrap(),
        }
    }

    pub fn root(&self) -> &Path {
        self.work.path()
    }

    /// `bolt`, run in `dir` as the test user.
    pub fn bolt_in(&self, dir: &Path) -> Bolt {
        Bolt::from_std(self.bolt_process_in(dir))
    }

    /// `bolt_in` as a process of the standard library, for a test that spawns it.
    pub fn bolt_process_in(&self, dir: &Path) -> Command {
        let mut cmd = Command::new(assert_cmd::cargo::cargo_bin("bolt"));
        cmd.current_dir(dir);
        as_test_user(&mut cmd, self.config.path());
        cmd
    }

    pub fn bolt(&self) -> Bolt {
        self.bolt_in(self.root())
    }

    /// The user's own settings folder.
    pub fn config(&self) -> &Path {
        self.config.path()
    }

    /// Runs Git in the repository and returns what it printed, trimmed.
    pub fn git(&self, args: &[&str]) -> String {
        self.git_input(args, b"")
    }

    /// Runs Git in the repository with `input` on its standard input, and returns what it
    /// printed, trimmed. The test fails if Git does.
    pub fn git_input(&self, args: &[&str], input: &[u8]) -> String {
        let out = git_with_input(self.root(), args, input);
        assert!(
            out.status.success(),
            "git {args:?} failed: {}",
            text(&out.stderr)
        );
        text(&out.stdout).trim().to_owned()
    }

    /// Writes `file` and commits it with `message`, by Ada; returns the commit's ID.
    pub fn commit(&self, file: &str, contents: &str, message: &str) -> String {
        let path = self.root().join(file);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, contents).unwrap();
        self.git(&["add", "-A"]);
        self.git(&[
            "commit",
            "-q",
            "--author=Ada Lovelace <ada@example.com>",
            "-m",
            message,
        ]);
        self.git(&["rev-parse", "HEAD"])
    }

    /// Runs `bolt import` and fails the test unless it worked; returns what it printed.
    pub fn import(&self) -> (String, String) {
        let out = self.bolt().arg("import").output().unwrap();
        assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
        (text(&out.stdout), text(&out.stderr))
    }

    /// Runs `bolt import`, which must fail, and returns its error output.
    pub fn refused_import(&self) -> String {
        let out = self.bolt().arg("import").output().unwrap();
        assert_eq!(out.status.code(), Some(1), "{}", text(&out.stdout));
        text(&out.stderr)
    }

    /// The project's state after the import.
    pub fn state(&self) -> (Repository, State) {
        let repo = Repository::open(self.root()).unwrap();
        let state = repo.current().unwrap().1.state;
        (repo, state)
    }

    /// `bolt save` with these arguments, whatever happens.
    pub fn save(&self, args: &[&str]) -> Output {
        self.bolt().arg("save").args(args).output().unwrap()
    }

    /// The newest diary entry and the packs in the store: a save that wrote nothing leaves both
    /// as they were.
    pub fn standing(&self) -> (Id, Vec<String>) {
        let repo = Repository::open(self.root()).unwrap();
        let mut packs: Vec<String> = std::fs::read_dir(repo.packs_dir())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        packs.sort();
        (repo.head().unwrap(), packs)
    }

    /// Writes `file` as it is, in the folder.
    pub fn overwrite(&self, file: &str, contents: &str) {
        std::fs::write(self.root().join(file), contents).unwrap();
    }
}
