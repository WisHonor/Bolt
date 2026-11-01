//! End-to-end tests of the `bolt` command.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use assert_cmd::Command;
use bolt_core::id::Id;
use bolt_core::listing::{Entry, EntryKind, Listing, MAX_SAVE_DEPTH};
use bolt_core::pack::{ObjectKind, PackWriter};
use bolt_core::repo::FORMAT_TEXT;
use std::path::Path;
use tempfile::TempDir;

/// A folder to work in, and an empty folder for the user's config.
struct Env {
    work: TempDir,
    config: TempDir,
}

impl Env {
    fn new() -> Env {
        Env {
            work: tempfile::tempdir().unwrap(),
            config: tempfile::tempdir().unwrap(),
        }
    }

    fn root(&self) -> &Path {
        self.work.path()
    }

    /// `bolt`, run in `dir`, with no user identity set.
    fn bolt_anonymous(&self, dir: &Path) -> Command {
        let mut cmd = Command::cargo_bin("bolt").unwrap();
        cmd.current_dir(dir)
            .env_remove("BOLT_USER_NAME")
            .env_remove("BOLT_USER_EMAIL")
            .env_remove("BOLT_FAULT_AT")
            .env("BOLT_CONFIG_DIR", self.config.path());
        cmd
    }

    /// `bolt`, run in `dir`, as the test user.
    fn bolt_in(&self, dir: &Path) -> Command {
        let mut cmd = self.bolt_anonymous(dir);
        cmd.env("BOLT_USER_NAME", "Test")
            .env("BOLT_USER_EMAIL", "test@example.com");
        cmd
    }

    fn bolt(&self) -> Command {
        self.bolt_in(self.root())
    }
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

#[test]
fn start_creates_a_project() {
    let env = Env::new();
    let out = env.bolt().arg("start").output().unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    let format = std::fs::read_to_string(env.root().join(".bolt/format")).unwrap();
    assert_eq!(format, FORMAT_TEXT);
    assert!(env.root().join(".bolt/state/head").is_file());
    assert!(text(&out.stdout).contains("Started a Bolt project"));
}

#[test]
fn start_twice_is_refused() {
    let env = Env::new();
    env.bolt().arg("start").assert().success();
    let out = env.bolt().arg("start").output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(
        text(&out.stderr).contains("already a Bolt project"),
        "{}",
        text(&out.stderr)
    );
}

#[test]
fn start_inside_a_project_is_refused() {
    let env = Env::new();
    env.bolt().arg("start").assert().success();
    let sub = env.root().join("sub");
    std::fs::create_dir(&sub).unwrap();
    let out = env.bolt_in(&sub).arg("start").output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(
        text(&out.stderr).contains("inside a Bolt project"),
        "{}",
        text(&out.stderr)
    );
    assert!(!sub.join(".bolt").exists());
}

#[test]
fn start_without_a_name_explains_how_to_set_it() {
    let env = Env::new();
    let out = env
        .bolt_anonymous(env.root())
        .arg("start")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let err = text(&out.stderr);
    assert!(err.contains("[user]") && err.contains("name ="), "{err}");
    assert!(!env.root().join(".bolt").exists());
}

/// Dotfile managers (stow, Nix home-manager) keep the user's own settings as links.
#[cfg(unix)]
#[test]
fn start_reads_the_users_own_config_through_a_link() {
    let env = Env::new();
    let dotfiles = tempfile::tempdir().unwrap();
    let real = dotfiles.path().join("bolt.toml");
    std::fs::write(
        &real,
        "[user]\nname = \"Ada\"\nemail = \"ada@example.com\"\n",
    )
    .unwrap();
    std::os::unix::fs::symlink(&real, env.config.path().join("config.toml")).unwrap();
    let out = env
        .bolt_anonymous(env.root())
        .arg("start")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
}

#[test]
fn start_json_output() {
    let env = Env::new();
    let out = env.bolt().args(["--json", "start"]).output().unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    let mut json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    // Check what the redactions hide, so a missing field can't hide behind one.
    // Compare canonical forms: on macOS the temp folder is under a symlink (/var is
    // /private/var), and `bolt` reports the resolved path its working folder has.
    let root = std::fs::canonicalize(json["root"].as_str().unwrap()).unwrap();
    assert_eq!(root, std::fs::canonicalize(env.root()).unwrap());
    let save = &json["save"];
    let change_id = save["change_id"].as_str().unwrap();
    assert!(change_id.len() == 32 && change_id.bytes().all(|b| (b'k'..=b'z').contains(&b)));
    assert_eq!(save["change_id_short"], change_id[..4]);
    assert_eq!(save["id"].as_str().unwrap().len(), 64);
    for person in ["author", "saver"] {
        assert!(save[person]["time_ms"].as_i64().unwrap() > 1_700_000_000_000);
        assert!(save[person]["tz_offset_min"].is_i64());
    }
    let redact = |v: &mut serde_json::Value| *v = "[redacted]".into();
    redact(&mut json["root"]);
    let save = &mut json["save"];
    for key in ["id", "change_id", "change_id_short"] {
        redact(&mut save[key]);
    }
    for person in ["author", "saver"] {
        for key in ["time_ms", "tz_offset_min"] {
            redact(&mut save[person][key]);
        }
    }
    insta::assert_json_snapshot!(json);
}

#[test]
fn json_errors_go_to_stdout() {
    let env = Env::new();
    env.bolt().arg("start").assert().success();
    let out = env.bolt().args(["--json", "start"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(json["bolt_json"], 1);
    assert_eq!(
        json["error"]["message"],
        "This folder is already a Bolt project."
    );
}

#[test]
fn usage_errors_exit_2() {
    let env = Env::new();
    env.bolt().arg("frobnicate").assert().code(2);
}

#[cfg(feature = "fault-injection")]
#[test]
fn interrupted_start_can_be_finished() {
    for n in 1.. {
        assert!(n < 50, "bolt start never finished");
        let env = Env::new();
        let out = env
            .bolt()
            .arg("start")
            .env("BOLT_FAULT_AT", n.to_string())
            .output()
            .unwrap();
        if !text(&out.stderr).contains("injected crash") {
            assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
            assert!(n > 1, "no fault point was reached");
            break;
        }
        assert!(!env.root().join(".bolt/format").exists(), "fault {n}");
        let out = env.bolt().arg("start").output().unwrap();
        assert_eq!(
            out.status.code(),
            Some(0),
            "fault {n}: {}",
            text(&out.stderr)
        );
        let report = bolt_core::verify::verify_store(&env.root().join(".bolt/store/packs"));
        assert!(report.is_ok(), "fault {n}: {:?}", report.problems);
    }
}

#[cfg(windows)]
#[test]
fn bolt_folder_is_hidden_on_windows() {
    let env = Env::new();
    env.bolt().arg("start").assert().success();
    assert!(bolt_platform::is_hidden(&env.root().join(".bolt")).unwrap());
}

#[test]
fn save_then_nothing_to_save() {
    let env = Env::new();
    env.bolt().arg("start").assert().success();
    std::fs::write(env.root().join("a.txt"), "a").unwrap();
    std::fs::create_dir(env.root().join("src")).unwrap();
    std::fs::write(env.root().join("src/b.txt"), "b").unwrap();
    let out = env
        .bolt()
        .args(["save", "Add two files\n\nThe details."])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    let stdout = text(&out.stdout);
    let short = stdout
        .strip_prefix("Saved ")
        .and_then(|s| s.strip_suffix("  \"Add two files\"  (2 files)\n"))
        .unwrap_or_else(|| panic!("{stdout:?}"));
    assert_eq!(short.len(), 4);
    let out = env.bolt().args(["save", "Again"]).output().unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    assert_eq!(
        text(&out.stdout),
        format!("Nothing to save: no changes since your last save ({short}).\n")
    );
    std::fs::write(env.root().join("a.txt"), "changed").unwrap();
    let out = env.bolt().args(["save", "Change a"]).output().unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    let stdout = text(&out.stdout);
    assert!(stdout.ends_with("  \"Change a\"  (1 file)\n"), "{stdout:?}");
}

#[test]
fn save_requires_a_message() {
    let env = Env::new();
    env.bolt().arg("start").assert().success();
    std::fs::write(env.root().join("a.txt"), "a").unwrap();
    for args in [&["save"][..], &["save", ""], &["save", "  \n"]] {
        let out = env.bolt().args(args).output().unwrap();
        assert_eq!(out.status.code(), Some(1), "{args:?}");
        let err = text(&out.stderr);
        assert!(
            err.contains("error: ") && err.contains("hint: ") && err.contains("bolt save \""),
            "{err}"
        );
    }
}

#[test]
fn save_json_output() {
    let env = Env::new();
    env.bolt().arg("start").assert().success();
    std::fs::write(env.root().join("a.txt"), "a").unwrap();
    let out = env
        .bolt()
        .args(["--json", "save", "Add a"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    let mut json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let save = &json["save"];
    let change_id = save["change_id"].as_str().unwrap();
    assert_eq!(save["change_id_short"], change_id[..4]);
    assert_eq!(save["id"].as_str().unwrap().len(), 64);
    for person in ["author", "saver"] {
        assert!(save[person]["time_ms"].as_i64().unwrap() > 1_700_000_000_000);
        assert!(save[person]["tz_offset_min"].is_i64());
    }
    let redact = |v: &mut serde_json::Value| *v = "[redacted]".into();
    let save = &mut json["save"];
    for key in ["id", "change_id", "change_id_short"] {
        redact(&mut save[key]);
    }
    for person in ["author", "saver"] {
        for key in ["time_ms", "tz_offset_min"] {
            redact(&mut save[person][key]);
        }
    }
    insta::assert_json_snapshot!(json);
    let out = env
        .bolt()
        .args(["--json", "save", "Again"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(
        json,
        serde_json::json!({"bolt_json": 1, "result": "nothing-changed"})
    );
}

#[cfg(feature = "fault-injection")]
#[test]
fn interrupted_save_changes_nothing() {
    use bolt_core::repo::Repository;
    let env = Env::new();
    env.bolt().arg("start").assert().success();
    let first = Repository::open(env.root()).unwrap().current().unwrap().1;
    std::fs::write(env.root().join("a.txt"), "a").unwrap();
    for n in 1.. {
        assert!(n < 50, "bolt save never finished");
        let out = env
            .bolt()
            .args(["save", "Add a"])
            .env("BOLT_FAULT_AT", n.to_string())
            .output()
            .unwrap();
        if !text(&out.stderr).contains("injected crash") {
            assert!(n > 1, "no fault point was reached");
            assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
            break;
        }
        let repo = Repository::open(env.root()).unwrap();
        let (_, entry) = repo.current().unwrap();
        assert_eq!(entry.state, first.state, "fault {n}");
        let report = bolt_core::verify::verify_store(repo.packs_dir());
        assert!(report.is_ok(), "fault {n}: {:?}", report.problems);
    }
    let out = env.bolt().args(["save", "Add a"]).output().unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    let (_, entry) = Repository::open(env.root()).unwrap().current().unwrap();
    assert_ne!(entry.state.lines["main"], first.state.lines["main"]);
}

fn redact_save(save: &mut serde_json::Value) {
    for key in ["id", "change_id", "change_id_short"] {
        save[key] = "[redacted]".into();
    }
    for person in ["author", "saver"] {
        for key in ["time_ms", "tz_offset_min"] {
            save[person][key] = "[redacted]".into();
        }
    }
}

/// A started project with saves `one` (adds a.txt) and `two` (adds b.txt).
fn project_with_two_saves() -> Env {
    let env = Env::new();
    env.bolt().arg("start").assert().success();
    for (name, msg) in [("a.txt", "one"), ("b.txt", "two")] {
        std::fs::write(env.root().join(name), name).unwrap();
        env.bolt().args(["save", msg]).assert().success();
    }
    env
}

fn stdout_of(env: &Env, args: &[&str]) -> String {
    let out = env.bolt().args(args).output().unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    text(&out.stdout)
}

#[test]
fn history_lists_newest_first() {
    let env = project_with_two_saves();
    let out = stdout_of(&env, &["history"]);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines.len(), 3, "{out}");
    for (line, msg) in lines.iter().zip(["two", "one", "Start project"]) {
        let cols: Vec<&str> = line.split("  ").collect();
        assert_eq!(cols.len(), 4, "{line:?}");
        assert!(cols[0].len() >= 4);
        assert_eq!(cols[1], "just now");
        assert_eq!((cols[2], cols[3]), ("Test", msg), "{line:?}");
    }
}

#[test]
fn show_lists_what_a_save_changed() {
    let env = Env::new();
    env.bolt().arg("start").assert().success();
    std::fs::write(env.root().join("b.txt"), "b").unwrap();
    std::fs::write(env.root().join("c.txt"), "c").unwrap();
    env.bolt().args(["save", "base"]).assert().success();
    std::fs::write(env.root().join("a.txt"), "a").unwrap();
    std::fs::write(env.root().join("b.txt"), "bbbb").unwrap();
    std::fs::remove_file(env.root().join("c.txt")).unwrap();
    env.bolt().args(["save", "mix\n\nmore"]).assert().success();
    let out = stdout_of(&env, &["show"]);
    let lines: Vec<&str> = out.lines().collect();
    assert!(lines[0].starts_with("Save "), "{out}");
    assert!(lines[0].ends_with(" · Test · just now"), "{out}");
    assert_eq!(
        &lines[1..],
        [
            "mix",
            "",
            "more",
            "",
            "  added     a.txt  (1 B)",
            "  modified  b.txt  (1 B → 4 B)",
            "  removed   c.txt  (1 B)"
        ],
        "{out}"
    );
}

#[test]
fn show_with_no_file_changes() {
    let env = Env::new();
    env.bolt().arg("start").assert().success();
    assert!(stdout_of(&env, &["show"]).ends_with("  (no file changes)\n"));
}

#[test]
fn show_resolves_lines_prefixes_and_fingerprints() {
    let env = project_with_two_saves();
    let json = |what: &str| -> serde_json::Value {
        serde_json::from_str(&stdout_of(&env, &["--json", "show", what])).unwrap()
    };
    let current = json("main");
    let id = current["save"]["id"].as_str().unwrap();
    assert_eq!(current["save"]["message"], "two");
    let history: serde_json::Value =
        serde_json::from_str(&stdout_of(&env, &["--json", "history"])).unwrap();
    let one = &history["saves"][1];
    let letters = one["change_id"].as_str().unwrap();
    assert_eq!(json(&letters[..4])["save"]["id"], one["id"]);
    assert_eq!(json(&id[..8])["save"]["id"], id);
    let out = env.bolt().args(["show", "nosuch"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(
        text(&out.stderr).contains("nosuch"),
        "{}",
        text(&out.stderr)
    );
}

#[test]
fn history_and_show_json_output() {
    let env = project_with_two_saves();
    let mut history: serde_json::Value =
        serde_json::from_str(&stdout_of(&env, &["--json", "history"])).unwrap();
    assert_eq!(history["line"], "main");
    for save in history["saves"].as_array_mut().unwrap() {
        redact_save(save);
    }
    insta::assert_json_snapshot!(history);
    let mut show: serde_json::Value =
        serde_json::from_str(&stdout_of(&env, &["--json", "show"])).unwrap();
    redact_save(&mut show["save"]);
    insta::assert_json_snapshot!(show);
}

#[test]
fn show_json_is_the_one_compact_line_serde_json_would_write() {
    let env = Env::new();
    env.bolt().arg("start").assert().success();
    // `show` writes the changes one at a time as it finds them, so it must still end up with the
    // document it wrote when it held them all: one compact line, keys in order.
    let raw = stdout_of(&env, &["--json", "show"]);
    let none: serde_json::Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(raw, format!("{none}\n"));
    assert_eq!(none["changes"], serde_json::json!([]));

    std::fs::write(env.root().join("b.txt"), "b").unwrap();
    std::fs::write(env.root().join("c.txt"), "c").unwrap();
    env.bolt().args(["save", "base"]).assert().success();
    std::fs::write(env.root().join("a.txt"), "a").unwrap();
    std::fs::write(env.root().join("b.txt"), "bbbb").unwrap();
    std::fs::remove_file(env.root().join("c.txt")).unwrap();
    env.bolt().args(["save", "mix"]).assert().success();
    let raw = stdout_of(&env, &["--json", "show"]);
    let three: serde_json::Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(raw, format!("{three}\n"));
    let paths: Vec<&str> = three["changes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["path"].as_str().unwrap())
        .collect();
    assert_eq!(paths, ["a.txt", "b.txt", "c.txt"]);
}

#[test]
fn commands_outside_a_project_explain_what_to_do() {
    let env = Env::new();
    let out = env.bolt().arg("history").output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    let err = text(&out.stderr);
    assert!(
        err.contains("Not inside a Bolt project") && err.contains("bolt start"),
        "{err}"
    );
}

#[test]
fn commands_work_from_a_subfolder() {
    let env = Env::new();
    env.bolt().arg("start").assert().success();
    let sub = env.root().join("project/sub");
    std::fs::create_dir_all(&sub).unwrap();
    std::fs::write(sub.join("f.txt"), "f").unwrap();
    env.bolt_in(&sub)
        .args(["save", "From sub"])
        .assert()
        .success();
    let out = env.bolt_in(&sub).arg("history").output().unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    assert!(text(&out.stdout).contains("From sub"));
    let out = stdout_of_in(&env, &sub, &["show"]);
    assert!(out.contains("added     project/sub/f.txt"), "{out}");
}

fn stdout_of_in(env: &Env, dir: &Path, args: &[&str]) -> String {
    let out = env.bolt_in(dir).args(args).output().unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    text(&out.stdout)
}

#[test]
fn hostile_text_cannot_drive_the_terminal() {
    let env = Env::new();
    env.bolt().arg("start").assert().success();
    std::fs::write(env.root().join("a.txt"), "a").unwrap();
    let out = env
        .bolt()
        .args(["save", "\u{1b}[31mred\n\n\u{1b}]0;x\u{7}body"])
        .env("BOLT_USER_NAME", "Eve\u{1b}[2J")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    for args in [&["history"][..], &["show"]] {
        let out = stdout_of(&env, args);
        assert!(!out.contains('\u{1b}') && !out.contains('\u{7}'), "{out:?}");
        assert!(
            out.contains("\\x1b[31mred") && out.contains("Eve\\x1b[2J"),
            "{out}"
        );
    }
}

// ---------------------------------------------------------------------------------------------
// Saves that no real folder tree could be: too deep, or with paths far too long to hold together.
// They are planted through Bolt's own library.
// ---------------------------------------------------------------------------------------------

/// A listing whose every entry has the same kind and points at the same object.
fn add_listing(pack: &mut PackWriter, names: &[String], kind: EntryKind, id: Id) -> Id {
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

/// Commits, through Bolt's own library, a save on `main` whose top folder `build` makes.
fn plant_save(root: &Path, build: impl FnOnce(&mut PackWriter) -> Id) {
    use bolt_core::diary::{Action, DiaryEntry};
    use bolt_core::id::ChangeId;
    use bolt_core::repo::Repository;
    use bolt_core::save::Save;

    let mut repo = Repository::open(root).unwrap();
    let lock = repo.lock_for_change("save", &mut |_| {}).unwrap();
    let (head, entry) = repo.current().unwrap();
    let parent_id = entry.state.lines["main"];
    let parent = repo.store().save(&parent_id, false).unwrap();
    let mut pack = repo.new_pack().unwrap();
    let save = Save {
        root: build(&mut pack),
        parents: vec![parent_id],
        author: parent.author.clone(),
        saver: parent.saver.clone(),
        message: "Planted".into(),
        change_id: ChangeId::random().unwrap(),
        move_notes: vec![],
        signature: vec![],
        extra: parent.extra.clone(),
    };
    let save_id = pack.add_record(ObjectKind::Save, &save.encode()).unwrap();
    let mut state = entry.state.clone();
    state.lines.insert("main".into(), save_id);
    let diary = DiaryEntry {
        parent: Some(head),
        action: Action::Command,
        command: "save".into(),
        who: parent.author.clone(),
        bolt_version: "0.1.0".into(),
        state,
        safety_copies: vec![],
    };
    repo.commit(&lock, pack, &diary).unwrap();
}

/// A started project whose newest save has one level of folders more than a save may have, and
/// a file `mine.txt` to save next to it.
fn project_with_a_save_too_deep_to_walk() -> Env {
    let env = Env::new();
    env.bolt().arg("start").assert().success();
    plant_save(env.root(), |pack| {
        let empty = pack
            .add_record(ObjectKind::FolderListing, &Listing::empty().encode())
            .unwrap();
        (0..=MAX_SAVE_DEPTH).fold(empty, |inner, _| {
            add_listing(pack, &["a".to_owned()], EntryKind::Folder, inner)
        })
    });
    std::fs::write(env.root().join("mine.txt"), "my work").unwrap();
    env
}

#[test]
fn show_of_a_save_too_deep_to_walk_prints_only_the_error() {
    let env = project_with_a_save_too_deep_to_walk();
    let out = env.bolt().arg("show").output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(
        text(&out.stdout),
        "",
        "nothing is printed before the refusal"
    );
    assert!(
        text(&out.stderr).contains("too large"),
        "{}",
        text(&out.stderr)
    );
    // As JSON, the error alone: no half of the save's own document in front of it.
    let out = env.bolt().args(["--json", "show"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(
        json["error"]["message"]
            .as_str()
            .unwrap()
            .contains("too large"),
        "{json}"
    );
    assert!(json.get("changes").is_none(), "{json}");
}

#[test]
fn save_next_to_a_save_too_large_to_compare_still_succeeds_without_a_count() {
    let env = project_with_a_save_too_deep_to_walk();
    let out = env.bolt().args(["save", "my work"]).output().unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    let stdout = text(&out.stdout);
    assert!(stdout.starts_with("Saved "), "{stdout:?}");
    assert!(!stdout.contains("(0 files)"), "{stdout:?}");
    assert!(
        stdout.ends_with("  \"my work\"  (couldn't count the changed files)\n"),
        "{stdout:?}"
    );
    assert!(stdout_of(&env, &["history"]).contains("my work"));
}

#[test]
fn save_json_next_to_a_save_too_large_to_compare_has_no_count() {
    let env = project_with_a_save_too_deep_to_walk();
    let out = env
        .bolt()
        .args(["--json", "save", "my work"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(json["result"], "saved");
    assert!(json["changed_files"].is_null(), "{json}");
}

/// Folders one inside the other, and files at the bottom. All names are 255 bytes, so each file
/// has a path of about `LONG_FOLDERS * 256` bytes, 100 KB, and the 2,500 paths together are over
/// 250 MB: far more than `bolt` is given below, in packs of only 66 KB. A save of this kind is
/// inside the limits of the format (4,096 levels, 10 million entries), so it must be readable.
#[cfg(unix)]
const LONG_FOLDERS: usize = 400;
#[cfg(unix)]
const LONG_FILES: usize = 2500;

/// The address space `bolt` gets: about 3 times what it needs to run (`save` needs about 35 MB,
/// mostly for the stack of its scanning thread), and well under all the long paths together.
#[cfg(unix)]
const CAP_KB: u32 = 100_000;

#[cfg(unix)]
fn plant_long_paths(root: &Path) {
    plant_save(root, |pack| {
        let empty = pack.add_content(b"").unwrap();
        let files: Vec<String> = (0..LONG_FILES).map(|i| format!("{i:0>255}")).collect();
        let mut id = add_listing(pack, &files, EntryKind::File, empty);
        for _ in 0..LONG_FOLDERS {
            id = add_listing(pack, &["d".repeat(255)], EntryKind::Folder, id);
        }
        id
    });
}

/// `bolt <args>` in the project, with its address space capped, so that holding too much shows
/// as an abort ("memory allocation of … bytes failed") rather than slowing the machine down.
/// Where the system does not enforce the cap (macOS), the command runs without one, and the test
/// passes there without proving anything.
#[cfg(unix)]
fn bolt_capped(env: &Env, args: &[&str]) -> std::process::Command {
    let mut cmd = std::process::Command::new("sh");
    cmd.arg("-c")
        .arg(format!(
            "ulimit -v {CAP_KB} 2>/dev/null; exec \"$0\" \"$@\""
        ))
        .arg(assert_cmd::cargo::cargo_bin("bolt"))
        .args(args)
        .current_dir(env.root())
        .env_remove("BOLT_FAULT_AT")
        .env("BOLT_USER_NAME", "Test")
        .env("BOLT_USER_EMAIL", "test@example.com")
        .env("BOLT_CONFIG_DIR", env.config.path());
    cmd
}

#[cfg(unix)]
#[test]
fn show_does_not_hold_every_long_path_of_a_save_at_once() {
    let env = Env::new();
    env.bolt().arg("start").assert().success();
    plant_long_paths(env.root());
    // What `show` prints is hundreds of megabytes, so it goes nowhere.
    let out = bolt_capped(&env, &["show"])
        .stdout(std::process::Stdio::null())
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
}

#[cfg(unix)]
#[test]
fn save_does_not_hold_every_long_path_of_the_save_before_it() {
    let env = Env::new();
    env.bolt().arg("start").assert().success();
    plant_long_paths(env.root());
    std::fs::write(env.root().join("mine.txt"), "my work").unwrap();
    let out = bolt_capped(&env, &["save", "my work"]).output().unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    // The long paths are all gone from the new save, and `mine.txt` is new.
    let want = format!("  \"my work\"  ({} files)\n", LONG_FILES + 1);
    let stdout = text(&out.stdout);
    assert!(stdout.ends_with(&want), "{stdout:?}");
}
