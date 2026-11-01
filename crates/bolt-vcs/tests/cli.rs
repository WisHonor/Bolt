//! End-to-end tests of the `bolt` command.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use assert_cmd::Command;
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
