//! End-to-end tests of the `bolt` command.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use assert_cmd::Command;
use bolt_core::id::Id;
use bolt_core::listing::{EntryKind, Listing, MAX_SAVE_DEPTH};
use bolt_core::pack::{ObjectKind, PackWriter};
use bolt_core::repo::FORMAT_TEXT;
use std::path::Path;

mod support;
use support::{add_listing, plant_save, text};

/// An empty folder to work in, and one for the user's config.
struct Env(support::Env);

impl std::ops::Deref for Env {
    type Target = support::Env;
    fn deref(&self) -> &support::Env {
        &self.0
    }
}

impl Env {
    fn new() -> Env {
        Env(support::Env::empty())
    }

    /// `bolt`, run in `dir`, with no user identity set.
    fn bolt_anonymous(&self, dir: &Path) -> Command {
        let mut cmd = self.bolt_in(dir);
        cmd.env_remove("BOLT_USER_NAME")
            .env_remove("BOLT_USER_EMAIL");
        cmd
    }
}

/// The `kind` of each warning in a `--json` document.
fn warning_kinds(doc: &serde_json::Value) -> Vec<&str> {
    doc["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| w["kind"].as_str().unwrap())
        .collect()
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

/// The check that refuses another account's project (T23) must never refuse the user's own, even
/// where an elevated administrator's folders belong to the Administrators group, as on Windows CI.
#[test]
fn start_and_save_work_in_a_normal_folder() {
    let env = Env::new();
    env.bolt().arg("start").assert().success();
    std::fs::write(env.root().join("a.txt"), "a").unwrap();
    env.bolt().args(["save", "first"]).assert().success();
    env.bolt().arg("history").assert().success();
}

/// A variable that places the user's Git settings, set to what Git refuses, stops a save as it
/// stops Git, naming the variable and how to get past it.
#[test]
fn a_git_settings_variable_git_refuses_stops_a_save_and_is_named() {
    let env = Env::new();
    env.bolt().arg("start").assert().success();
    std::fs::write(env.root().join("a.txt"), "a").unwrap();
    let out = env
        .bolt()
        .args(["save", "first"])
        .env("GIT_CONFIG_COUNT", "two")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let err = text(&out.stderr);
    assert!(
        err.starts_with(
            "error: Bolt can't read your Git settings in GIT_CONFIG_COUNT: `two` isn't a number \
             Git reads as a count, and Git refuses it too\nhint: Fix that variable, or unset it."
        ),
        "{err}"
    );
    env.bolt().args(["save", "first"]).assert().success();
}

/// A project reached through a link (a junction on Windows) is the folder the link leads to.
#[test]
fn save_works_through_a_link_to_the_project_folder() {
    let env = Env::new();
    env.bolt().arg("start").assert().success();
    std::fs::write(env.root().join("a.txt"), "a").unwrap();
    let links = tempfile::tempdir().unwrap();
    let link = links.path().join("project");
    #[cfg(unix)]
    std::os::unix::fs::symlink(env.root(), &link).unwrap();
    #[cfg(windows)]
    support::junction(&link, env.root());
    let out = env.bolt_in(&link).args(["save", "first"]).output().unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    let history = env.bolt().arg("history").output().unwrap();
    assert!(text(&history.stdout).contains("first"));
}

/// Whether a folder belongs to an elevated Administrators group is up to the computer (an
/// administrator's terminal does it, as on Windows CI). Where it does, `start` says the project
/// opens only from such a terminal, on stderr, whatever the output format; with `--json` it is
/// also a warning of the document.
#[test]
fn start_says_when_its_project_opens_only_from_an_administrator_terminal() {
    for args in [&["start"][..], &["--json", "start"]] {
        let env = Env::new();
        let out = env.bolt().args(args).output().unwrap();
        let (stdout, stderr) = (text(&out.stdout), text(&out.stderr));
        assert_eq!(out.status.code(), Some(0), "{args:?}: {stderr}");
        let expected = bolt_core::repo::elevated_only_note(env.root()).is_some();
        let said = stderr.contains("warning: This project belongs to the Administrators group");
        assert_eq!(said, expected, "{args:?}: {stderr}");
        if args.contains(&"--json") {
            let doc: serde_json::Value = serde_json::from_str(&stdout).unwrap();
            let kinds = warning_kinds(&doc);
            assert_eq!(kinds.contains(&"elevated_only"), expected, "{doc}");
            let note = doc["warnings"][0]["message"].as_str().unwrap_or_default();
            assert_eq!(note.contains("administrator terminal"), expected, "{doc}");
        } else {
            assert!(!stdout.contains("administrator terminal"), "{args:?}");
        }
    }
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

/// However deep the folder is, `start` finds the project above it, names it, and makes nothing.
#[test]
fn start_inside_a_project_is_refused() {
    let env = Env::new();
    env.bolt().arg("start").assert().success();
    // The last part of the project's path, which every system shows the same way.
    let project = env.root().file_name().unwrap().to_str().unwrap();
    for sub in [env.root().join("sub"), env.root().join("a/b/c")] {
        std::fs::create_dir_all(&sub).unwrap();
        let out = env.bolt_in(&sub).arg("start").output().unwrap();
        assert_eq!(out.status.code(), Some(1));
        let err = text(&out.stderr);
        let said = err.lines().next().unwrap();
        assert!(said.contains("inside a Bolt project at "), "{err}");
        assert!(said.ends_with(&format!("{project}.")), "{err}");
        assert!(!sub.join(".bolt").exists());
    }
}

/// bolt.new and other tools keep their own `.bolt` folders; Bolt never settles into one.
#[test]
fn start_refuses_another_tools_bolt_folder() {
    let env = Env::new();
    let bolt = env.root().join(".bolt");
    std::fs::create_dir(&bolt).unwrap();
    std::fs::write(bolt.join("prompt"), "Build me an app").unwrap();
    let out = env.bolt().arg("start").output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(
        text(&out.stderr).contains(
            "There's a .bolt folder here that Bolt didn't make (bolt.new and other tools use \
             that name); move or rename it first."
        ),
        "{}",
        text(&out.stderr)
    );
    let inside: Vec<_> = std::fs::read_dir(&bolt).unwrap().collect();
    assert_eq!(inside.len(), 1, "nothing was made in it");
    // An interrupted start of Bolt's own, with only its own folders, is finished.
    std::fs::remove_file(bolt.join("prompt")).unwrap();
    std::fs::create_dir(bolt.join("store")).unwrap();
    env.bolt().arg("start").assert().success();
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
    // The snippet to add is lines of its own, not one line with `\x0a` between them.
    assert!(err.contains(":\n[user]\nname = "), "{err}");
    assert!(!env.root().join(".bolt").exists());
}

/// The error says which file it is once, not in the message and again in the reason.
#[test]
fn a_settings_file_that_cant_be_read_is_named_once() {
    let env = Env::new();
    let settings = env.config().join("config.toml");
    std::fs::create_dir(&settings).unwrap();
    let out = env
        .bolt_anonymous(env.root())
        .arg("start")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let err = text(&out.stderr);
    assert_eq!(err.matches("config.toml").count(), 1, "{err}");
    assert!(err.contains("is not a valid Bolt settings file: it is not a regular file"));
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
    std::os::unix::fs::symlink(&real, env.config().join("config.toml")).unwrap();
    let out = env
        .bolt_anonymous(env.root())
        .arg("start")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
}

// Who the user is: only from the environment and their own settings, never from a project's
// (RT-9, decision 025).

const REAL_USER: &str = "[user]\nname = \"Real User\"\nemail = \"real@example.com\"\n";
const MALLORY: &str = "[user]\nname = \"Mallory\"\nemail = \"mallory@evil.example\"\n";

/// The user's own settings, a started project, and a `.bolt/config.toml` in it (the one a shared
/// project can bring) that names someone else; `a.txt` is waiting to be saved.
fn project_naming_mallory() -> Env {
    let env = Env::new();
    std::fs::write(env.config().join("config.toml"), REAL_USER).unwrap();
    env.bolt_anonymous(env.root())
        .arg("start")
        .assert()
        .success();
    std::fs::write(env.root().join(".bolt/config.toml"), MALLORY).unwrap();
    std::fs::write(env.root().join("a.txt"), "a").unwrap();
    env
}

fn json_of(env: &Env, args: &[&str]) -> serde_json::Value {
    let out = env.bolt_anonymous(env.root()).args(args).output().unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    serde_json::from_slice(&out.stdout).unwrap()
}

const MALLORY_NOTE: &str = "note: This project's settings name Mallory <mallory@evil.example>; \
                            Bolt ignores identities from project settings.";

#[test]
fn save_notes_an_ignored_project_identity() {
    let env = project_naming_mallory();
    let out = env
        .bolt_anonymous(env.root())
        .args(["save", "my work"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    // The save is the user's own, whatever the project's settings say.
    let history = json_of(&env, &["--json", "history"]);
    assert_eq!(history["saves"][0]["author"]["name"], "Real User");
    assert_eq!(history["saves"][0]["saver"]["email"], "real@example.com");

    let stdout = text(&out.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    let config = env.config().join("config.toml");
    let folder = env.root().file_name().unwrap().to_str().unwrap();
    assert_eq!(lines.len(), 7, "{stdout}");
    assert!(lines[0].starts_with("Saved "), "{stdout}");
    assert_eq!(lines[1], MALLORY_NOTE, "{stdout}");
    assert_eq!(
        lines[2],
        format!(
            "hint: To save as them in this project, add this to {}:",
            config.display()
        ),
        "{stdout}"
    );
    // This folder, and who the project names, ready to be added to the user's own settings.
    assert_eq!(lines[3], "[[project]]", "{stdout}");
    assert!(
        lines[4].starts_with("folder = ") && lines[4].contains(folder),
        "{stdout}"
    );
    assert_eq!(lines[5], "name = \"Mallory\"", "{stdout}");
    assert_eq!(lines[6], "email = \"mallory@evil.example\"", "{stdout}");

    // Said again when there is nothing to save: the settings are still there.
    let out = env
        .bolt_anonymous(env.root())
        .args(["save", "again"])
        .output()
        .unwrap();
    let stdout = text(&out.stdout);
    assert!(stdout.starts_with("Nothing to save"), "{stdout}");
    assert_eq!(stdout.lines().nth(1), Some(MALLORY_NOTE), "{stdout}");
}

/// With both variables set they decide who the user is, so the lines the note offers would change
/// nothing, and it says so. One alone names nobody: the user's settings still decide.
#[test]
fn the_note_says_when_the_environment_comes_first() {
    let env = project_naming_mallory();
    let out = env.bolt().args(["save", "my work"]).output().unwrap();
    let stdout = text(&out.stdout);
    let hint = stdout
        .lines()
        .find(|l| l.starts_with("hint:"))
        .expect(&stdout);
    assert!(
        hint.starts_with("hint: BOLT_USER_NAME and BOLT_USER_EMAIL are set")
            && hint.contains("take no effect until you unset them"),
        "{stdout}"
    );
    assert!(stdout.contains("\n[[project]]\nfolder = "), "{stdout}");

    std::fs::write(env.root().join("b.txt"), "b").unwrap();
    let out = env
        .bolt_anonymous(env.root())
        .env("BOLT_USER_NAME", "Test")
        .args(["save", "settings decide"])
        .output()
        .unwrap();
    let stdout = text(&out.stdout);
    assert!(
        stdout.contains("hint: To save as them in this project, add this to"),
        "{stdout}"
    );
    assert!(!stdout.contains("BOLT_USER"), "{stdout}");
}

#[test]
fn save_json_names_the_ignored_project_identity() {
    let env = project_naming_mallory();
    let json = json_of(&env, &["--json", "save", "my work"]);
    assert_eq!(
        json["ignored_project_identity"],
        serde_json::json!({"name": "Mallory", "email": "mallory@evil.example"})
    );
    assert_eq!(json["save"]["author"]["name"], "Real User");
    // Nothing was saved this time, and the key is there all the same. (`json_of` reads all of
    // stdout as one document, so `--json` prints no note beside it.)
    let again = json_of(&env, &["--json", "save", "again"]);
    assert_eq!(again["result"], "nothing-changed");
    assert_eq!(again["ignored_project_identity"]["name"], "Mallory");

    // A project that names nobody, or the user themselves, has nothing to say.
    for named in ["", REAL_USER] {
        std::fs::write(env.root().join(".bolt/config.toml"), named).unwrap();
        let again = json_of(&env, &["--json", "save", "again"]);
        assert_eq!(
            again,
            serde_json::json!({"bolt_json": 1, "result": "nothing-changed", "paths": [], "warnings": []})
        );
        let out = env
            .bolt_anonymous(env.root())
            .args(["save", "again"])
            .output()
            .unwrap();
        assert_eq!(text(&out.stdout).lines().count(), 1);
    }
}

/// The user approves the identity on purpose by adding the lines the note shows to their own
/// settings. From then on it is used for that folder, and for no other.
#[test]
fn approving_the_ignored_identity_takes_effect() {
    let env = project_naming_mallory();
    let out = env
        .bolt_anonymous(env.root())
        .args(["save", "my work"])
        .output()
        .unwrap();
    let stdout = text(&out.stdout);
    let snippet = &stdout[stdout.find("[[project]]").expect(&stdout)..];
    let config = env.config().join("config.toml");
    std::fs::write(&config, format!("{REAL_USER}\n{snippet}")).unwrap();

    std::fs::write(env.root().join("b.txt"), "b").unwrap();
    let json = json_of(&env, &["--json", "save", "as Mallory, on purpose"]);
    assert_eq!(json["save"]["author"]["name"], "Mallory");
    assert_eq!(json["save"]["author"]["email"], "mallory@evil.example");
    // It is what the project names, so there is nothing left to ignore.
    assert!(json.get("ignored_project_identity").is_none(), "{json}");
    // Only this folder: another project still gets the user's own.
    let other = tempfile::tempdir().unwrap();
    let out = env
        .bolt_anonymous(other.path())
        .args(["--json", "start"])
        .output()
        .unwrap();
    let started: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(started["save"]["author"]["name"], "Real User");
}

/// The first save of a project is recorded as the user said it should be for that folder.
#[test]
fn start_uses_the_project_entry_for_its_folder() {
    let env = Env::new();
    let entry = format!(
        "\n[[project]]\nfolder = '{}'\nname = \"Work Name\"\nemail = \"me@work.example\"\n",
        env.root().display()
    );
    std::fs::write(
        env.config().join("config.toml"),
        format!("{REAL_USER}{entry}"),
    )
    .unwrap();
    env.bolt_anonymous(env.root())
        .arg("start")
        .assert()
        .success();
    let history = json_of(&env, &["--json", "history"]);
    assert_eq!(history["saves"][0]["author"]["name"], "Work Name");
}

// A project's own settings are read only after the user's identity is known. A file there that
// can't be read the ordinary way (a pipe that waits for a writer, a link to a file that never ends)
// must not stop the save. `rt3_…` and `rt4_…` in redteam.rs stop earlier, for lack of an identity.

#[cfg(unix)]
#[test]
fn save_with_an_identity_ignores_a_fifo_project_config() {
    let env = Env::new();
    env.bolt().arg("start").assert().success();
    let made = std::process::Command::new("mkfifo")
        .arg(env.root().join(".bolt/config.toml"))
        .status()
        .unwrap();
    assert!(made.success());
    std::fs::write(env.root().join("a.txt"), "a").unwrap();
    // Opening a pipe to read waits for someone to write to it, so a save that did would never end.
    let out = env
        .bolt()
        .args(["save", "my work"])
        .timeout(std::time::Duration::from_secs(30))
        .output()
        .unwrap();
    let stdout = text(&out.stdout);
    assert_eq!(
        out.status.code(),
        Some(0),
        "bolt save hung on a pipe at .bolt/config.toml, or failed: {:?}\n{}",
        out.status,
        text(&out.stderr)
    );
    // Saved, and nothing said about the pipe: it names nobody.
    assert!(stdout.starts_with("Saved "), "{stdout}");
    assert_eq!(stdout.lines().count(), 1, "{stdout}");
}

#[cfg(unix)]
#[test]
fn save_with_an_identity_ignores_a_dev_zero_project_config() {
    let env = Env::new();
    env.bolt().arg("start").assert().success();
    std::os::unix::fs::symlink("/dev/zero", env.root().join(".bolt/config.toml")).unwrap();
    std::fs::write(env.root().join("a.txt"), "a").unwrap();
    // The address space is capped at about 1.5 GB, so a read of the link that did not stop could
    // not take the machine down.
    let cmd = support::bolt_capped(env.config(), env.root(), &["save", "my work"]);
    let (out, peak_kb) = run_noting_peak_memory(cmd, 30);
    let (stdout, stderr) = (text(&out.stdout), text(&out.stderr));
    assert_eq!(
        out.status.code(),
        Some(0),
        "bolt save read a link to /dev/zero, or failed: {:?}\n{stderr}",
        out.status
    );
    assert!(!stderr.contains("memory"), "{stderr}");
    assert!(stdout.starts_with("Saved "), "{stdout}");
    assert_eq!(stdout.lines().count(), 1, "{stdout}");
    // Reaching the cap is not an abort here: reads fail with an error when memory runs out, and
    // unreadable settings are ignored, so a save that read the link still ends well, a gigabyte
    // later. The peak memory shows it (no /proc on macOS: the peak is unknown and this passes).
    assert!(
        peak_kb < 200_000,
        "bolt save held {} MB: it read the link to /dev/zero",
        peak_kb / 1024
    );
}

/// macOS lets a program hold 256 files open by default, folders and packs included. A save of a
/// tree deeper than the folders it holds open stays within that, however far CI raises the limit.
#[cfg(unix)]
#[test]
fn a_deep_tree_saves_within_macos_open_file_limit() {
    use bolt_platform::dir::Dir;
    use std::ffi::OsStr;
    let env = Env::new();
    env.bolt().arg("start").assert().success();
    // 600 levels, each with a file, made a name at a time: macOS takes no path that long.
    {
        let n = OsStr::new;
        let mut at = Dir::open_top(env.root()).unwrap();
        for _ in 0..600 {
            at.create_new(n("f"), false).unwrap();
            at.create_dir(n("d")).unwrap();
            at = at.open_dir(n("d")).unwrap();
        }
    }
    let mut save = support::bolt_with_open_files(env.config(), env.root(), 256, &["save", "deep"]);
    let out = save.output().unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    assert!(text(&out.stdout).starts_with("Saved "));
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
    // The one warning a fresh project can have depends on the computer, so the snapshot has none.
    let elevated = bolt_core::repo::elevated_only_note(env.root()).is_some();
    assert_eq!(
        warning_kinds(&json),
        if elevated {
            vec!["elevated_only"]
        } else {
            vec![]
        }
    );
    json["warnings"] = serde_json::json!([]);
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
    // Present even here, where the project was never opened and nothing was noted.
    assert_eq!(json.get("warnings"), Some(&serde_json::json!([])), "{json}");
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
        // The read-back comes after the project is made, so it is whole: only the report is lost.
        if text(&out.stderr).contains("injected crash at start:read-back") {
            assert!(env.root().join(".bolt/format").exists(), "fault {n}");
            env.bolt().arg("history").assert().success();
            continue;
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

/// The project exists once `start` has committed, so a first save that can't be read back to show
/// is a warning and the command still succeeds.
#[cfg(feature = "fault-injection")]
#[test]
fn start_reports_success_when_its_first_save_cannot_be_read_back() {
    for json in [false, true] {
        let env = Env::new();
        let mut bolt = env.bolt();
        if json {
            bolt.arg("--json");
        }
        let out = bolt
            .arg("start")
            .env("BOLT_FAULT_FAIL", "start:read-back")
            .output()
            .unwrap();
        let (stdout, stderr) = (text(&out.stdout), text(&out.stderr));
        assert_eq!(out.status.code(), Some(0), "{stderr}");
        assert!(
            stderr.contains(
                "warning: The project was made, but Bolt couldn't read its first save back. \
                 injected fault at start:read-back\n\
                 Run `bolt history` to check that the project reads.\n"
            ),
            "{stderr}"
        );
        if json {
            let doc: serde_json::Value = serde_json::from_str(&stdout).unwrap();
            assert_eq!(doc.get("save"), Some(&serde_json::Value::Null), "{doc}");
            assert_eq!(doc["line"], "main");
            assert!(
                warning_kinds(&doc).contains(&"first_save_not_read"),
                "{doc}"
            );
        } else {
            assert!(stdout.contains("Started a Bolt project in"), "{stdout}");
            assert!(!stdout.contains("first save is"), "{stdout}");
        }
        // The project is whole: it has its first save, and nothing is missing.
        let history = env.bolt().args(["--json", "history"]).output().unwrap();
        let history: serde_json::Value = serde_json::from_slice(&history.stdout).unwrap();
        assert_eq!(
            history["saves"].as_array().map(Vec::len),
            Some(1),
            "{history}"
        );
        let packs = env.root().join(".bolt/store/packs");
        assert!(bolt_core::verify::verify_store(&packs).is_ok());
    }
}

#[cfg(windows)]
#[test]
fn bolt_folder_is_hidden_on_windows() {
    let env = Env::new();
    env.bolt().arg("start").assert().success();
    assert!(bolt_platform::is_hidden(&env.root().join(".bolt")).unwrap());
}

#[cfg(windows)]
#[test]
fn a_command_names_its_own_file_another_program_holds() {
    use std::os::windows::fs::OpenOptionsExt;
    let env = Env::new();
    env.bolt().arg("start").assert().success();
    std::fs::write(env.root().join("a.txt"), "a").unwrap();
    let bolt = env.root().join(".bolt");
    // As a virus scanner or sync tool may hold it, for longer than Bolt tries.
    let held = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(bolt.join("state/lock-info"))
        .unwrap();
    let out = env.bolt().args(["save", "Add a"]).output().unwrap();
    drop(held);
    assert_eq!(out.status.code(), Some(1), "{}", text(&out.stdout));
    let err = text(&out.stderr);
    assert!(err.contains(r"\.bolt\state\lock-info: "), "{err}");
    assert!(err.contains("virus scanner"), "{err}");
    assert_eq!(std::fs::read_dir(bolt.join("tmp")).unwrap().count(), 0);
}

#[test]
fn ordinary_commands_still_write_one_pack() {
    let env = Env::new();
    let packs = || {
        std::fs::read_dir(env.root().join(".bolt/store/packs"))
            .unwrap()
            .count()
    };
    env.bolt().arg("start").assert().success();
    assert_eq!(packs(), 1);
    std::fs::write(env.root().join("a.txt"), "a").unwrap();
    env.bolt().args(["save", "Add a"]).assert().success();
    assert_eq!(packs(), 2);
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

/// A started project holding a new file of 100,000,001 zeros, made without writing them.
fn project_adding_100_mb() -> Env {
    let env = Env::new();
    env.bolt().arg("start").assert().success();
    std::fs::create_dir(env.root().join("art")).unwrap();
    let huge = std::fs::File::create(env.root().join("art/huge.bin")).unwrap();
    huge.set_len(100_000_001).unwrap();
    env
}

#[test]
fn without_a_terminal_it_stops_and_names_yes() {
    let env = project_adding_100_mb();
    let out = env.bolt().args(["save", "Add art"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1), "{}", text(&out.stdout));
    let err = text(&out.stderr);
    assert!(
        err.contains("This save adds 100 MB in 1 new file, most in: art/ (100 MB)."),
        "{err}"
    );
    assert!(
        err.contains("Nothing was saved.") && err.contains("--yes"),
        "{err}"
    );
    let out = env
        .bolt()
        .args(["save", "--json", "Add art"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let doc: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(
        doc["error"]["hint"].as_str().unwrap().contains("--yes"),
        "{doc}"
    );
    let out = env.bolt().arg("status").output().unwrap();
    assert!(text(&out.stdout).contains("art/huge.bin"), "still unsaved");
}

#[test]
fn yes_skips_the_question() {
    let env = project_adding_100_mb();
    let out = env
        .bolt()
        .args(["save", "--yes", "Add art"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    assert!(text(&out.stdout).ends_with(
        "  \"Add art\"  (1 file)
"
    ));
}

/// Microsoft's list of reserved names doesn't hold `COM0` and `LPT0`, and Windows makes them as
/// ordinary files, so they save like any other name.
#[cfg(windows)]
#[test]
fn com0_and_lpt0_save_on_windows() {
    let env = Env::new();
    env.bolt().arg("start").assert().success();
    for name in ["COM0", "LPT0", "com0.txt", "LPT0.log"] {
        std::fs::write(env.root().join(name), name).unwrap();
        assert_eq!(
            std::fs::read(env.root().join(name)).unwrap(),
            name.as_bytes()
        );
    }
    let out = env
        .bolt()
        .args(["save", "Device-like names"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    let stdout = text(&out.stdout);
    assert!(stdout.ends_with("(4 files)\n"), "{stdout:?}");
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

/// A started project with a `LICENSE` to save, and a pack that holds a damaged copy of its content.
fn project_with_a_damaged_copy_of_its_licence() -> Env {
    let env = Env::new();
    env.bolt().arg("start").assert().success();
    let licence = b"Permission is hereby granted, free of charge, to any person\n";
    // The licence with one bit flipped, in a pack whose name sorts before Bolt's own.
    let made = tempfile::tempdir().unwrap();
    let mut w = PackWriter::create(made.path()).unwrap();
    w.add_content(licence).unwrap();
    let mut pack = std::fs::read(w.finish(made.path()).unwrap().unwrap()).unwrap();
    pack[16 + 50] ^= 1; // the first byte after the pack header and the entry header
    std::fs::write(env.root().join(".bolt/store/packs/00.pack"), pack).unwrap();
    std::fs::write(env.root().join("LICENSE"), licence).unwrap();
    env
}

/// The warning goes in the document too, with the words people see on stderr.
#[test]
fn save_json_carries_a_damaged_pack_warning() {
    let env = project_with_a_damaged_copy_of_its_licence();
    let out = env
        .bolt()
        .args(["--json", "save", "Add licence"])
        .output()
        .unwrap();
    let stderr = text(&out.stderr);
    assert_eq!(out.status.code(), Some(0), "{stderr}");
    let doc: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(doc["result"], "saved", "{doc}");
    assert_eq!(warning_kinds(&doc), ["damaged_pack"], "{doc}");
    let message = doc["warnings"][0]["message"].as_str().unwrap();
    assert!(message.contains("00.pack is damaged: object "), "{message}");
    assert!(
        stderr.contains(&format!("warning: {message}\n")),
        "{stderr}"
    );
}

/// A pack holding a damaged copy of a file's content doesn't stop the save, and is named.
#[test]
fn save_warns_about_a_pack_holding_a_damaged_copy() {
    let env = project_with_a_damaged_copy_of_its_licence();
    let out = env.bolt().args(["save", "Add licence"]).output().unwrap();
    let stderr = text(&out.stderr);
    assert_eq!(out.status.code(), Some(0), "{stderr}");
    assert!(
        text(&out.stdout).starts_with("Saved "),
        "{}",
        text(&out.stdout)
    );
    assert!(
        stderr.contains("warning: pack ")
            && stderr.contains("00.pack is damaged: object ")
            && stderr.contains("hint: Bolt didn't use the damaged copy."),
        "{stderr}"
    );
}

/// Bit rot in the only copy of a save, as `rot` makes it. The change-ID cache goes too, so that the
/// line is read again.
fn damage_the_only_copy_of(root: &Path, id: &Id) {
    std::fs::remove_file(root.join(".bolt/cache/change-ids")).unwrap();
    rot(root, id);
}

/// Bit rot in the only copy of a save: its first stored byte is flipped.
fn rot(root: &Path, id: &Id) {
    use bolt_core::pack::{ENTRY_HEADER_LEN, PackReader};
    for e in std::fs::read_dir(root.join(".bolt/store/packs")).unwrap() {
        let path = e.unwrap().path();
        let found = PackReader::open(&path).unwrap().find(id).unwrap();
        if let Some(entry) = found {
            let mut bytes = std::fs::read(&path).unwrap();
            bytes[usize::try_from(entry.offset + ENTRY_HEADER_LEN).unwrap()] ^= 1;
            std::fs::write(&path, bytes).unwrap();
        }
    }
}

/// Writes `00.pack`, which sorts before Bolt's own packs, holding a copy of the record `id` (of
/// `kind`) with one bit flipped. The record's own copy is fine, so reads pass the damaged one over.
fn plant_damaged_copy_of(root: &Path, id: &Id, kind: ObjectKind) {
    use bolt_core::pack::{ENTRY_HEADER_LEN, PackReader};
    use bolt_core::repo::Repository;
    let repo = Repository::open(root).unwrap();
    let record = repo.store().read_record(id, kind).unwrap();
    let made = tempfile::tempdir().unwrap();
    let mut w = PackWriter::create(made.path()).unwrap();
    w.add_record(kind, &record).unwrap();
    let path = w.finish(made.path()).unwrap().unwrap();
    let mut bytes = std::fs::read(&path).unwrap();
    let entry = PackReader::open(&path).unwrap().find(id).unwrap().unwrap();
    bytes[usize::try_from(entry.offset + ENTRY_HEADER_LEN).unwrap()] ^= 1;
    std::fs::write(root.join(".bolt/store/packs/00.pack"), bytes).unwrap();
}

/// A started project with two saves, the first of which can no longer be read, and a file
/// `mine.txt` to save next to them.
fn project_whose_first_save_is_damaged() -> Env {
    use bolt_core::repo::Repository;
    let env = Env::new();
    env.bolt().arg("start").assert().success();
    let first = Repository::open(env.root())
        .unwrap()
        .current()
        .unwrap()
        .1
        .state
        .lines["main"];
    std::fs::write(env.root().join("a.txt"), "a").unwrap();
    env.bolt().args(["save", "one"]).assert().success();
    damage_the_only_copy_of(env.root(), &first);
    std::fs::write(env.root().join("mine.txt"), "my work").unwrap();
    env
}

/// A save that Bolt can't shorten the ID of is still a save: it exits 0 and says "Saved", names
/// the whole change ID, and warns on its own line. It never says nothing was changed.
#[test]
fn save_is_reported_as_saved_when_the_line_cant_be_read_to_shorten_its_id() {
    use bolt_core::repo::Repository;
    let env = project_whose_first_save_is_damaged();
    let before = Repository::open(env.root()).unwrap().head().unwrap();
    let out = env.bolt().args(["save", "two"]).output().unwrap();
    let (stdout, stderr) = (text(&out.stdout), text(&out.stderr));
    assert_eq!(out.status.code(), Some(0), "{stderr}");
    assert_ne!(
        Repository::open(env.root()).unwrap().head().unwrap(),
        before
    );
    let shown = stdout.strip_prefix("Saved ").expect(&stdout);
    let (id, rest) = shown.split_once("  ").expect(&stdout);
    assert_eq!(id.len(), 32, "the whole change ID: {stdout:?}");
    assert_eq!(rest, "\"two\"  (1 file)\n");
    assert!(
        stderr.contains(
            "warning: The save was made, but Bolt couldn't read this line's older saves to \
             shorten its ID."
        ),
        "{stderr}"
    );
    assert!(
        !stderr.contains("Nothing was changed") && !stderr.contains("error:"),
        "{stderr}"
    );
}

/// The same for nothing to save, which may have stored a repair, and for `show`: the whole ID
/// and a warning, never a failure.
#[test]
fn nothing_to_save_and_show_work_when_the_line_cant_be_read_to_shorten_ids() {
    let env = project_whose_first_save_is_damaged();
    env.bolt().args(["save", "two"]).assert().success();
    for args in [&["save", "again"][..], &["show"]] {
        let out = env.bolt().args(args).output().unwrap();
        let (stdout, stderr) = (text(&out.stdout), text(&out.stderr));
        assert_eq!(out.status.code(), Some(0), "{args:?}: {stderr}");
        assert!(
            stderr.contains("warning: Bolt couldn't read this line's older saves"),
            "{args:?}: {stderr}"
        );
        assert!(
            !stderr.contains("Nothing was changed"),
            "{args:?}: {stderr}"
        );
        assert!(!stdout.is_empty(), "{args:?}");
    }
}

#[test]
fn save_json_is_the_same_when_the_line_cant_be_read_to_shorten_its_id() {
    let env = project_whose_first_save_is_damaged();
    let out = env.bolt().args(["--json", "save", "two"]).output().unwrap();
    let stderr = text(&out.stderr);
    assert_eq!(out.status.code(), Some(0), "{stderr}");
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(json["result"], "saved", "{json}");
    assert_eq!(json["save"]["change_id_short"], json["save"]["change_id"]);
    assert_eq!(json["save"]["change_id"].as_str().unwrap().len(), 32);
    assert!(stderr.contains("warning: The save was made"), "{stderr}");
    assert!(!stderr.contains("Nothing was changed"), "{stderr}");
    // The same words are in the document, and the damaged save's pack is named beside them.
    assert_eq!(
        warning_kinds(&json),
        ["change_id_not_shortened", "damaged_pack"],
        "{json}"
    );
    let said = json["warnings"][0]["message"].as_str().unwrap();
    assert!(
        said.starts_with("The save was made, but Bolt couldn't read"),
        "{said}"
    );
}

/// `show` shortens the ID of the save it shows, so it can fail to, too.
#[test]
fn show_json_carries_the_warning_about_the_id_it_couldnt_shorten() {
    let env = project_whose_first_save_is_damaged();
    env.bolt().args(["save", "two"]).assert().success();
    let out = env.bolt().args(["--json", "show"]).output().unwrap();
    let (stdout, stderr) = (text(&out.stdout), text(&out.stderr));
    assert_eq!(out.status.code(), Some(0), "{stderr}");
    let doc: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(
        warning_kinds(&doc),
        ["change_id_not_shortened", "damaged_pack"],
        "{doc}"
    );
    let said = doc["warnings"][0]["message"].as_str().unwrap();
    assert!(
        said.starts_with("Bolt couldn't read this line's older saves"),
        "{said}"
    );
    assert!(stderr.contains(&format!("warning: {said}\n")), "{stderr}");
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
        serde_json::json!({"bolt_json": 1, "result": "nothing-changed", "paths": [], "warnings": []})
    );
}

#[cfg(feature = "fault-injection")]
#[test]
fn interrupted_save_changes_nothing_until_its_head_moves() {
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
        let said = text(&out.stderr);
        if !said.contains("injected crash") {
            assert!(n > 1, "no fault point was reached");
            assert_eq!(out.status.code(), Some(0), "{said}");
            break;
        }
        let repo = Repository::open(env.root()).unwrap();
        let (_, entry) = repo.current().unwrap();
        let report = bolt_core::verify::verify_store(repo.packs_dir());
        assert!(report.is_ok(), "fault {n}: {:?}", report.problems);
        // Its last stop comes once the new head is in place: the save happened.
        if said.contains("injected crash at head:renamed") {
            assert_ne!(entry.state, first.state, "fault {n}");
            break;
        }
        assert_eq!(entry.state, first.state, "fault {n}");
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
            "    mix",
            "",
            "    more",
            "",
            "  added     a.txt  (1 B)",
            "  modified  b.txt  (1 B → 4 B)",
            "  removed   c.txt  (1 B)"
        ],
        "{out}"
    );
}

/// A message is the author's to write, so `show` indents every line of it by four spaces, as Git
/// does: none of its lines can start with the two spaces and the word of a change line.
#[test]
fn show_indents_the_message_so_it_cannot_fake_a_change() {
    let env = Env::new();
    env.bolt().arg("start").assert().success();
    std::fs::write(env.root().join("invoice.pdf"), "pdf").unwrap();
    let message = "add the invoice\n\n  removed   payroll.xlsx  (12 KB)";
    env.bolt().args(["save", message]).assert().success();
    let out = stdout_of(&env, &["show"]);
    // The only line that can pass for a change is the one real change.
    let like_changes: Vec<&str> = out
        .lines()
        .filter(|l| l.starts_with("  ") && !l.starts_with("   "))
        .collect();
    assert_eq!(like_changes, ["  added     invoice.pdf  (3 B)"], "{out}");
    // Every line of the message is there: indented, its own spaces kept, blank lines blank.
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(
        &lines[1..],
        [
            "    add the invoice",
            "",
            "      removed   payroll.xlsx  (12 KB)",
            "",
            "  added     invoice.pdf  (3 B)"
        ],
        "{out}"
    );
    assert!(out.lines().all(|l| l.trim_end() == l), "{out}");
    // `--json` is the message as it was written.
    let json: serde_json::Value =
        serde_json::from_str(&stdout_of(&env, &["--json", "show"])).unwrap();
    assert_eq!(json["save"]["message"], message);
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
fn history_json_is_the_one_compact_line_serde_json_would_write() {
    let env = project_with_two_saves();
    // `history` writes the saves one at a time as it reads them, so it must still end up with the
    // document it wrote when it held them all: one compact line, keys in order.
    let raw = stdout_of(&env, &["--json", "history"]);
    let history: serde_json::Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(raw, format!("{history}\n"));
    assert_eq!(history["line"], "main");
    let messages: Vec<&str> = history["saves"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["message"].as_str().unwrap())
        .collect();
    assert_eq!(messages, ["two", "one", "Start project"]);
}

/// Every document, whatever it says, has `warnings`; with nothing to warn about, an empty list.
#[test]
fn every_json_document_has_warnings() {
    let env = Env::new();
    let run = |args: &[&str]| -> serde_json::Value {
        let out = env.bolt().args(args).output().unwrap();
        assert_eq!(
            out.status.code(),
            Some(0),
            "{args:?}: {}",
            text(&out.stderr)
        );
        serde_json::from_slice(&out.stdout).unwrap()
    };
    let started = run(&["--json", "start"]);
    std::fs::write(env.root().join("a.txt"), "a").unwrap();
    let saved = run(&["--json", "save", "m"]);
    let again = run(&["--json", "save", "m"]);
    let history = run(&["--json", "history"]);
    let shown = run(&["--json", "show"]);
    assert_eq!(saved["result"], "saved");
    assert_eq!(again["result"], "nothing-changed");
    for (what, doc) in [
        ("start", &started),
        ("save", &saved),
        ("nothing to save", &again),
        ("history", &history),
        ("show", &shown),
    ] {
        assert!(doc["warnings"].is_array(), "{what}: {doc}");
    }
    // A healthy project has nothing to warn about (`start` may say how it opens, see above).
    for doc in [&saved, &again, &history, &shown] {
        assert_eq!(doc["warnings"], serde_json::json!([]), "{doc}");
    }
}

/// The failed command's own document. `history` and `show` read every save before they print
/// anything, so a save that can't be read gives the document of a failed command, not a cut-off
/// one: `error`, none of `saves` or `changes`, and the `warnings` the store noted on the way (the
/// damaged pack). Ending a document after output began is tested in output.rs.
#[test]
fn history_json_stays_valid_when_a_save_is_unreadable() {
    let env = project_with_two_saves();
    let history = json_of(&env, &["--json", "history"]);
    // Three saves, and the middle one has no other copy.
    let middle = Id::from_hex(history["saves"][1]["id"].as_str().unwrap()).unwrap();
    damage_the_only_copy_of(env.root(), &middle);
    for command in ["history", "show"] {
        let out = env.bolt().args(["--json", command]).output().unwrap();
        assert_eq!(out.status.code(), Some(1), "{command}");
        // `from_slice` takes one document and nothing after it.
        let doc: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
        assert_eq!(doc["bolt_json"], 1, "{command}: {doc}");
        let error = doc["error"]["message"].as_str().unwrap();
        assert!(error.contains("damaged"), "{command}: {error}");
        assert!(
            doc.get("saves").is_none() && doc.get("changes").is_none(),
            "{doc}"
        );
        // The pack with the damaged save was noted while it was read, before the command failed.
        assert_eq!(warning_kinds(&doc), ["damaged_pack"], "{command}: {doc}");
        let said = doc["warnings"][0]["message"].as_str().unwrap();
        assert!(said.contains("is damaged"), "{command}: {said}");
    }
}

/// Copies that were passed over are reported by the commands that only read, too. The damaged
/// copy is of the head's diary entry: the first object read, before any pack is open, so the pack
/// that sorts first is searched first.
#[test]
fn history_and_show_json_carry_a_damaged_pack_warning() {
    use bolt_core::repo::Repository;
    let env = project_with_two_saves();
    let head = Repository::open(env.root()).unwrap().head().unwrap();
    plant_damaged_copy_of(env.root(), &head, ObjectKind::DiaryEntry);
    for command in ["history", "show"] {
        let out = env.bolt().args(["--json", command]).output().unwrap();
        let stderr = text(&out.stderr);
        assert_eq!(out.status.code(), Some(0), "{command}: {stderr}");
        let doc: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
        assert_eq!(warning_kinds(&doc), ["damaged_pack"], "{command}: {doc}");
        let said = doc["warnings"][0]["message"].as_str().unwrap();
        assert!(said.contains("00.pack is damaged"), "{command}: {said}");
        assert!(stderr.contains(&format!("warning: {said}\n")), "{stderr}");
    }
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

/// RT-8 (`rt8_…` in redteam.rs uses an escape sequence, which only unix lets a folder be named
/// with): the project folder's name, in what `start` prints and in a message that names it.
#[test]
fn folder_names_are_shown_escaped() {
    let env = Env::new();
    let folder = env.root().join("proj\u{202e}gnp.exe");
    std::fs::create_dir(&folder).unwrap();
    let out = env.bolt_in(&folder).arg("start").output().unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    let shown = text(&out.stdout);
    assert!(!shown.contains('\u{202e}'), "{shown:?}");
    assert!(shown.contains("proj\\u{202e}gnp.exe"), "{shown:?}");

    let sub = folder.join("sub");
    std::fs::create_dir(&sub).unwrap();
    let out = env.bolt_in(&sub).arg("start").output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    let shown = text(&out.stderr);
    assert!(!shown.contains('\u{202e}'), "{shown:?}");
    assert!(shown.contains("proj\\u{202e}gnp.exe"), "{shown:?}");
}

#[test]
fn boltignore_files_past_the_size_limit_stop_the_save_and_name_the_file() {
    let env = Env::new();
    env.bolt().arg("start").assert().success();
    // Each file is well within its own limit; together they are past the limit for one folder.
    let comment = "#".repeat(600 * 1024);
    std::fs::create_dir(env.root().join("sub")).unwrap();
    std::fs::write(env.root().join(".boltignore"), &comment).unwrap();
    std::fs::write(env.root().join("sub/.boltignore"), &comment).unwrap();
    let out = env.bolt().args(["save", "x"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    let shown = text(&out.stderr);
    assert!(
        shown.contains("problem: sub/.boltignore: with the other ignore files in force at its "),
        "{shown:?}"
    );
    assert!(
        shown.contains("it is larger than 1048576 bytes."),
        "{shown:?}"
    );
    assert!(shown.contains("hint: Nothing was saved."), "{shown:?}");
}

#[test]
fn text_quoted_in_an_error_is_shown_escaped() {
    let env = Env::new();
    env.bolt().arg("start").assert().success();
    // The error names a folder, and a right-to-left override in its name must not reach the
    // terminal.
    let folder = env.root().join("x\u{202e}");
    std::fs::create_dir(&folder).unwrap();
    std::fs::write(folder.join(".boltignore"), vec![b'#'; (1 << 20) + 1]).unwrap();
    let out = env.bolt().args(["save", "x"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    let shown = text(&out.stderr);
    assert!(!shown.contains('\u{202e}'), "{shown:?}");
    assert!(shown.contains("x\\u{202e}/.boltignore"), "{shown:?}");
    // The message is for people in JSON too, so it is just as safe to print.
    let out = env.bolt().args(["--json", "save", "x"]).output().unwrap();
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let message = json["error"]["message"].as_str().unwrap();
    assert!(!message.contains('\u{202e}'), "{message:?}");
    assert!(message.contains("x\\u{202e}/.boltignore"), "{message:?}");
}

/// Two names that would not work on every system, so the error has two lines of problems. Where
/// any name is allowed: a device name and a name with a trailing dot. On Windows, which allows
/// neither: one name written two ways, in the top folder and in a folder below it.
fn plant_two_non_portable_names(root: &Path) {
    #[cfg(not(windows))]
    for name in ["CON", "name."] {
        std::fs::write(root.join(name), "x").unwrap();
    }
    #[cfg(windows)]
    for folder in [root.to_owned(), root.join("sub")] {
        std::fs::create_dir_all(&folder).unwrap();
        for name in ["caf\u{e9}.txt", "cafe\u{301}.txt"] {
            std::fs::write(folder.join(name), "x").unwrap();
        }
    }
}

/// A refused save as JSON: the error, with the warnings the store gave before the save was refused.
/// Here the head's diary entry has a damaged copy. It is the first object read, before any pack
/// has held one, so the pack that sorts first is searched first, and its copy is passed over.
#[test]
fn save_json_error_lists_the_warnings_found_before_it_failed() {
    use bolt_core::repo::Repository;
    let env = Env::new();
    env.bolt().arg("start").assert().success();
    let head = Repository::open(env.root()).unwrap().head().unwrap();
    plant_damaged_copy_of(env.root(), &head, ObjectKind::DiaryEntry);
    plant_two_non_portable_names(env.root());
    let out = env.bolt().args(["--json", "save", "x"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    let doc: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let error = doc["error"]["message"].as_str().unwrap();
    assert!(error.contains("would not work on every system"), "{doc}");
    assert_eq!(warning_kinds(&doc), ["damaged_pack"], "{doc}");
    // People are shown the error alone, which the document has too.
    assert_eq!(text(&out.stderr), "");
}

/// RT-11: the problems of a save that is refused are on lines of their own.
#[test]
fn multi_line_error_prints_lines() {
    let env = Env::new();
    env.bolt().arg("start").assert().success();
    plant_two_non_portable_names(env.root());
    let out = env.bolt().args(["save", "x"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    let err = text(&out.stderr);
    assert!(!err.contains("\\x0a"), "{err}");
    let lines: Vec<&str> = err.lines().collect();
    assert_eq!(lines.len(), 4, "{err}");
    assert_eq!(
        lines[0],
        "error: Some new names would not work on every system:"
    );
    assert!(
        lines[1].starts_with("  ") && lines[2].starts_with("  "),
        "{err}"
    );
    assert!(lines[3].starts_with("hint: "), "{err}");
}

/// RT-11: a folder that can't be entered is unreadable, not a problem with a `.boltignore` in it.
/// (Windows never made this mistake: only unix refuses to look inside a folder for a name.)
#[cfg(unix)]
#[test]
fn unreadable_folder_is_reported_as_unreadable() {
    use std::os::unix::fs::PermissionsExt;
    let env = Env::new();
    env.bolt().arg("start").assert().success();
    let locked = env.root().join("locked");
    std::fs::create_dir(&locked).unwrap();
    std::fs::write(locked.join("f.txt"), "secret").unwrap();
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
    // root reads everything, so there is nothing to test.
    let root_user = std::fs::read_dir(&locked).is_ok();
    let out = env.bolt().args(["save", "x"]).output().unwrap();
    // Back to normal first, so that the temporary folder can be removed whatever happens.
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
    if root_user {
        return;
    }
    assert_eq!(out.status.code(), Some(1));
    let err = text(&out.stderr);
    let message = err.lines().next().unwrap();
    assert!(message.starts_with("error: Can't read locked: "), "{err}");
    assert!(!err.contains("ignore file has a problem"), "{err}");
}

// Saves that no real folder tree could be (too deep, or paths far too long to hold together),
// planted through Bolt's own library.

/// Commits, through Bolt's own library, `n` more saves on `main`, each the child of the last and
/// with a random change ID; `shape` may change each one first. Returns their change IDs, oldest
/// first.
fn plant_history(
    root: &Path,
    n: usize,
    mut shape: impl FnMut(&mut bolt_core::save::Save),
) -> Vec<bolt_core::id::ChangeId> {
    use bolt_core::diary::{Action, DiaryEntry};
    use bolt_core::id::ChangeId;
    use bolt_core::repo::Repository;

    let mut repo = Repository::open(root).unwrap();
    let lock = repo.lock_for_change("save", &mut |_| {}).unwrap();
    let (head, entry) = repo.current().unwrap();
    let mut tip = entry.state.lines["main"];
    let mut save = repo.store().save(&tip, false).unwrap();
    let mut pack = repo.new_packs().unwrap();
    let mut change_ids = Vec::new();
    for _ in 0..n {
        save.parents = vec![tip];
        save.change_id = ChangeId::random().unwrap();
        shape(&mut save);
        change_ids.push(save.change_id);
        tip = pack.add_record(ObjectKind::Save, &save.encode()).unwrap();
    }
    let mut state = entry.state.clone();
    state.lines.insert("main".into(), tip);
    let diary = DiaryEntry {
        parent: Some(head),
        action: Action::Command,
        command: "save".into(),
        who: save.author.clone(),
        bolt_version: "0.1.0".into(),
        state,
        folder_change: None,
    };
    repo.commit(&lock, pack, &diary).unwrap();
    change_ids
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
    assert_eq!(json.get("warnings"), Some(&serde_json::json!([])), "{json}");
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
    // The reason is on stderr, with the words the document has.
    let stderr = text(&out.stderr);
    assert!(
        stderr
            .contains("warning: The save was made, but Bolt couldn't count the files it changed."),
        "{stderr}"
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
    // Present and null, not left out; the warning says why.
    assert_eq!(json.get("changed_files"), Some(&serde_json::Value::Null));
    assert_eq!(
        warning_kinds(&json),
        ["changed_files_not_counted"],
        "{json}"
    );
    let said = json["warnings"][0]["message"].as_str().unwrap();
    assert!(said.contains("too large"), "{said}");
    assert!(text(&out.stderr).contains(&format!("warning: {said}\n")));
}

/// Fourteen nested folders, then 250 folders that are one listing of 250 files, all names 255
/// bytes: each file's path is 4,095 bytes, just within a path's limit, and the 62,500 paths
/// together over 250 MB, far more than `bolt` is given below. This is within the format's limits
/// (paths of 4,096 bytes, 10 million entries), so it must be readable.
#[cfg(unix)]
const LONG_LEVELS: usize = 14;
#[cfg(unix)]
const LONG_FOLDERS: usize = 250;
#[cfg(unix)]
const LONG_FILES: usize = 250;

/// The address space `bolt` gets: well under all the long paths together, and under 64 MiB, so the
/// C library can't reserve a whole arena for a thread and starve the rest (at 90 MB the save below
/// failed half its runs for that alone). `save` needs under 40 MB.
#[cfg(unix)]
const CAP_KB: u32 = 64_000;

#[cfg(unix)]
fn plant_long_paths(root: &Path) {
    plant_save(root, |pack| {
        let empty = pack.add_content(b"").unwrap();
        let named = |n: usize| -> Vec<String> { (0..n).map(|i| format!("{i:0>255}")).collect() };
        let files = add_listing(pack, &named(LONG_FILES), EntryKind::File, empty);
        let mut id = add_listing(pack, &named(LONG_FOLDERS), EntryKind::Folder, files);
        for _ in 0..LONG_LEVELS {
            id = add_listing(pack, &["d".repeat(255)], EntryKind::Folder, id);
        }
        id
    });
}

/// `bolt <args>` in the project, with its address space capped, so that holding too much aborts
/// ("memory allocation of … bytes failed") instead of slowing the machine down. Where the cap isn't
/// enforced (macOS), the test passes without proving anything.
#[cfg(unix)]
fn bolt_capped(env: &Env, args: &[&str]) -> std::process::Command {
    support::bolt_capped_at(env.config(), env.root(), CAP_KB, args)
}

/// Runs `cmd` for at most `secs` seconds, then kills it, and returns its output with the peak
/// resident set it was seen to hold, in KB, read from `/proc` (0 where there is none).
#[cfg(unix)]
fn run_noting_peak_memory(
    mut cmd: std::process::Command,
    secs: u64,
) -> (std::process::Output, u64) {
    use std::process::Stdio;
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let started = std::time::Instant::now();
    let mut peak_kb = 0;
    // What `bolt` prints is a few lines, which wait in the pipes until it has ended.
    while child.try_wait().unwrap().is_none() && started.elapsed().as_secs() < secs {
        let status = std::fs::read_to_string(format!("/proc/{}/status", child.id()));
        let held = status
            .as_deref()
            .unwrap_or_default()
            .lines()
            .find_map(|line| {
                line.strip_prefix("VmHWM:")?
                    .trim()
                    .strip_suffix("kB")?
                    .trim()
                    .parse::<u64>()
                    .ok()
            });
        peak_kb = peak_kb.max(held.unwrap_or(0));
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    // Does something only after the time limit; the output is then of a killed process.
    let _ = child.kill();
    (child.wait_with_output().unwrap(), peak_kb)
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
    let want = format!("  \"my work\"  ({} files)\n", LONG_FOLDERS * LONG_FILES + 1);
    let stdout = text(&out.stdout);
    assert!(stdout.ends_with(&want), "{stdout:?}");
}

/// Each change ID cut to the fewest letters, at least four, that no other ID starts with.
fn shortest_unique(all: &[String]) -> Vec<String> {
    all.iter()
        .enumerate()
        .map(|(k, mine)| {
            let unique = |len: usize| {
                let start = &mine[..len];
                all.iter()
                    .enumerate()
                    .all(|(j, other)| j == k || !other.starts_with(start))
            };
            let len = (4..=mine.len()).find(|&len| unique(len));
            mine[..len.unwrap_or(mine.len())].to_owned()
        })
        .collect()
}

#[test]
fn history_shows_every_save_with_the_fewest_letters_that_keep_it_unique() {
    let env = Env::new();
    env.bolt().arg("start").assert().success();
    // So many saves that some change IDs share their first four letters.
    let planted = plant_history(env.root(), 1500, |_| {});
    let history: serde_json::Value =
        serde_json::from_str(&stdout_of(&env, &["--json", "history"])).unwrap();
    let saves = history["saves"].as_array().unwrap();
    let field = |key: &str| -> Vec<String> {
        saves
            .iter()
            .map(|s| s[key].as_str().unwrap().to_owned())
            .collect()
    };
    let change_ids = field("change_id");
    // Newest first: the planted saves, then the one `start` made.
    let newest_first: Vec<String> = planted.iter().rev().map(|c| c.to_letters()).collect();
    assert_eq!(change_ids.len(), 1501);
    assert_eq!(change_ids[..1500], newest_first[..]);

    let want = shortest_unique(&change_ids);
    assert!(want.iter().any(|s| s.len() > 4), "no change IDs collided");
    assert_eq!(field("change_id_short"), want);
    let printed: Vec<String> = stdout_of(&env, &["history"])
        .lines()
        .map(|line| line.split("  ").next().unwrap().to_owned())
        .collect();
    assert_eq!(printed, want);
}

/// 150 messages of 1 MiB take a few hundred bytes each on disk but 150 MiB together, more than
/// `bolt` is given here; one at a time they take a few MiB.
#[cfg(unix)]
#[test]
fn history_show_and_save_hold_one_save_at_a_time() {
    let env = Env::new();
    env.bolt().arg("start").assert().success();
    let message = format!("Looks harmless\n{}", "A".repeat((1 << 20) - 15));
    let planted = plant_history(env.root(), 150, |save| save.message.clone_from(&message));
    std::fs::write(env.root().join("mine.txt"), "my work").unwrap();
    let oldest = planted[0].to_letters();
    let commands: [&[&str]; 6] = [
        &["history"],
        &["--json", "history"],
        &["show"],
        &["--json", "show"],
        &["show", &oldest[..8]],
        &["save", "my work"],
    ];
    for args in commands {
        let out = bolt_capped(&env, args)
            .stdout(std::process::Stdio::null())
            .output()
            .unwrap();
        assert_eq!(
            out.status.code(),
            Some(0),
            "{args:?}: {}",
            text(&out.stderr)
        );
    }
}

/// A key file Bolt can't use is the user's to deal with: commands still work, without caches,
/// and say so, and the file is left as it is.
#[test]
fn a_cache_key_bolt_cant_use_is_a_warning_not_a_failure() {
    let env = Env::new();
    let key = env.config().join("cache-key");
    std::fs::write(&key, b"short").unwrap();
    let start = json_as_user(&env, &["--json", "start"]);
    assert_eq!(warning_kinds(&start), ["cache_key_unusable"], "{start}");
    std::fs::write(env.root().join("a.txt"), "a").unwrap();
    for args in [
        &["--json", "save", "one"][..],
        &["--json", "history"],
        &["--json", "show"],
    ] {
        let doc = json_as_user(&env, args);
        assert_eq!(
            warning_kinds(&doc),
            ["cache_key_unusable"],
            "{args:?}: {doc}"
        );
        let said = doc["warnings"][0]["message"].as_str().unwrap();
        assert!(
            said.contains("cache-key") && said.contains("delete it"),
            "{said}"
        );
    }
    let out = env.bolt().arg("history").output().unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    assert!(
        text(&out.stderr).contains("warning: Bolt can't use its cache key"),
        "{}",
        text(&out.stderr)
    );
    assert_eq!(std::fs::read(&key).unwrap(), b"short");
}

/// With the change-ID cache current, a save needn't read the line, so a damaged older save
/// doesn't touch it; `history` hands out the newer saves and then ends its document with the error.
#[test]
fn a_current_cache_reads_a_damaged_older_save_only_when_history_reaches_it() {
    let env = project_with_two_saves();
    let history = json_as_user(&env, &["--json", "history"]);
    let oldest = Id::from_hex(history["saves"][2]["id"].as_str().unwrap()).unwrap();
    rot(env.root(), &oldest);
    std::fs::write(env.root().join("c.txt"), "c").unwrap();
    let saved = json_as_user(&env, &["--json", "save", "three"]);
    assert_eq!(saved["result"], "saved", "{saved}");
    assert!(warning_kinds(&saved).is_empty(), "{saved}");
    let out = env.bolt().args(["--json", "history"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    let doc: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(doc["saves"].as_array().unwrap().len(), 3, "{doc}");
    assert!(
        doc["error"]["message"]
            .as_str()
            .unwrap()
            .contains("damaged"),
        "{doc}"
    );
}

/// `json_of`, run as the test user, so that a save may be made.
fn json_as_user(env: &Env, args: &[&str]) -> serde_json::Value {
    let out = env.bolt().args(args).output().unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    serde_json::from_slice(&out.stdout).unwrap()
}

#[test]
fn paths_are_relative_to_the_current_folder() {
    let env = Env::new();
    env.bolt().arg("start").assert().success();
    let sub = env.root().join("sub");
    std::fs::create_dir_all(sub.join("deep")).unwrap();
    for (path, bytes) in [
        ("a.txt", "a"),
        ("sub/a.txt", "a"),
        ("sub/b.png", "b"),
        ("sub/deep/c.png", "c"),
    ] {
        std::fs::write(env.root().join(path), bytes).unwrap();
    }
    let saved = |args: &[&str]| {
        let out = env.bolt_in(&sub).arg("--json").args(args).output().unwrap();
        assert_eq!(out.status.code(), Some(0), "{}", text(&out.stdout));
        let doc: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
        doc["paths"].clone()
    };
    assert_eq!(
        saved(&["save", "A", "a.txt"]),
        serde_json::json!(["sub/a.txt"])
    );
    let shown = stdout_of_in(&env, &sub, &["show"]);
    assert!(shown.contains("added     sub/a.txt"), "{shown}");
    assert!(
        !shown.contains("b.png") && !shown.contains("added     a.txt"),
        "{shown}"
    );
    // A pattern is matched from here too, and `..` reaches above.
    assert_eq!(
        saved(&["save", "B", "*.png"]),
        serde_json::json!(["sub/*.png"])
    );
    let shown = stdout_of_in(&env, &sub, &["show"]);
    assert!(
        shown.contains("sub/b.png") && !shown.contains("c.png"),
        "{shown}"
    );
    assert_eq!(
        saved(&["save", "C", "../a.txt"]),
        serde_json::json!(["a.txt"])
    );
    // Nothing named left to save.
    let out = stdout_of_in(&env, &sub, &["save", "D", "a.txt"]);
    assert!(out.contains("Nothing to save there"), "{out}");
    // Outside the project, or matching nothing, saves nothing.
    for (args, said) in [
        (&["save", "E", "../../x"][..], "is outside the project"),
        (
            &["save", "E", "nope.txt"][..],
            "sub/nope.txt matches nothing",
        ),
    ] {
        let out = env.bolt_in(&sub).args(args).output().unwrap();
        assert_eq!(out.status.code(), Some(1));
        let err = text(&out.stderr);
        assert!(err.contains(said), "{err}");
    }
    std::fs::write(env.root().join("a.txt"), "f").unwrap();
    assert_eq!(saved(&["save", "F"]), serde_json::json!([]));
}

/// `bolt <args>` in `env`'s folder with its stdout a pipe whose reader has gone before the command
/// starts: how it ended, and what it said on stderr.
fn with_reader_gone(env: &support::Env, args: &[&str]) -> (Option<i32>, String) {
    let (reader, writer) = std::io::pipe().unwrap();
    drop(reader);
    let out = env
        .bolt_process_in(env.root())
        .args(args)
        .stdout(writer)
        .output()
        .unwrap();
    (out.status.code(), text(&out.stderr))
}

/// `bolt <args>` in `env`'s folder with stdout and stderr on one pipe whose reader has gone before
/// the command starts (`bolt status 2>&1 | true`): its exit code.
fn with_both_readers_gone(env: &support::Env, args: &[&str]) -> Option<i32> {
    let (reader, writer) = std::io::pipe().unwrap();
    drop(reader);
    let out = env
        .bolt_process_in(env.root())
        .args(args)
        .stdout(writer.try_clone().unwrap())
        .stderr(writer)
        .output()
        .unwrap();
    out.status.code()
}

#[test]
fn a_command_with_warnings_ends_quietly_when_stderr_shares_the_gone_reader() {
    let env = support::Env::empty();
    env.bolt().arg("start").assert().success();
    // A folder holding a Git repository of its own: `status` and `save` warn about it.
    std::fs::create_dir_all(env.root().join("inner/.git")).unwrap();
    env.overwrite("inner/x.txt", "x\n");
    env.overwrite("a.txt", "a\n");
    for args in [&["status"][..], &["status", "--json"], &["save", "Add a"]] {
        assert_eq!(with_both_readers_gone(&env, args), Some(0), "{args:?}");
    }
    let history = env.bolt().arg("history").output().unwrap();
    assert!(text(&history.stdout).contains("Add a"));
}

#[cfg(feature = "fault-injection")]
#[test]
fn a_failed_json_command_whose_reader_has_gone_tells_its_error_and_warnings_on_stderr() {
    let env = support::Env::empty();
    env.bolt().arg("start").assert().success();
    let first = support::main_tip(&env).to_hex();
    env.overwrite("a.txt", "a\n");
    env.overwrite("b.txt", "b\n");
    env.bolt().args(["save", "Two files"]).assert().success();
    // A goto stopped after removing a.txt, then a.txt written again.
    let stopped = env
        .bolt()
        .args(["goto", &first])
        .env("BOLT_FAULT_FAIL", "switch:changed")
        .output()
        .unwrap();
    assert_eq!(stopped.status.code(), Some(1), "{}", text(&stopped.stderr));
    // A listing that stopped early still gives what it found first.
    let (code, said) = with_reader_gone(&env, &["history"]);
    assert_eq!(code, Some(0), "{said}");
    assert!(
        said.starts_with("warning: A switch between saves was stopped before it was done"),
        "{said}"
    );
    env.overwrite("a.txt", "mine\n");
    // The next goto finishes it, warning that a.txt changed, then stops: a.txt is in the way.
    let (code, said) = with_reader_gone(&env, &["goto", "main", "--json"]);
    assert_eq!(code, Some(1), "{said}");
    assert!(
        said.starts_with(
            "error: Your unsaved changes, or files Bolt ignores, are in the way, so nothing was \
             changed:\n  a.txt\nhint: "
        ),
        "{said}"
    );
    for kind in [
        "warning: Finished a switch between saves that was stopped",
        "warning: 1 entry changed while Bolt was switching",
    ] {
        assert_eq!(said.matches(kind).count(), 1, "{said}");
    }
    assert!(!said.contains("couldn't write"), "{said}");
}

#[test]
fn a_json_listing_tells_the_error_it_ended_with_when_stdout_loses_the_end() {
    let env = support::Env::empty();
    env.bolt().arg("start").assert().success();
    env.overwrite("a.txt", "a\n");
    env.overwrite("b.txt", "b\n");
    env.bolt().args(["save", "Two files"]).assert().success();
    let top = support::top_folder(env.root());
    let b = top.entries().iter().find(|e| e.name == "b.txt").unwrap().id;
    rot(env.root(), &b);
    env.overwrite("a.txt", "A\n");
    env.overwrite("b.txt", "B\n");
    // a.txt is listed, then b.txt's saved copy can't be read, and the document ends with that.
    let out = env.bolt().args(["changes", "--json"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    let doc: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(doc["changes"].as_array().unwrap().len(), 1, "{doc}");
    let error = format!("error: {}\n", doc["error"]["message"].as_str().unwrap());
    // The items wait in stdout's buffer, so the document's end is the write that meets the gone
    // reader, or fails.
    let (code, said) = with_reader_gone(&env, &["changes", "--json"]);
    assert_eq!(code, Some(1), "{said}");
    assert!(said.starts_with(&error), "{said}");
    #[cfg(any(windows, target_os = "linux"))]
    {
        let out = env
            .bolt_process_in(env.root())
            .args(["changes", "--json"])
            .stdout(support::failing_stdout(&env))
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(1));
        let said = text(&out.stderr);
        let lost = said.find("error: Bolt couldn't write its output: ");
        assert!(said.starts_with(&error) && lost.is_some(), "{said}");
    }
}

#[test]
fn every_command_ends_quietly_when_its_reader_has_gone() {
    let mut loud = Vec::new();
    let mut quiet = |env: &support::Env, args: &[&str]| {
        let (code, said) = with_reader_gone(env, args);
        if (code, said.as_str()) != (Some(0), "") {
            loud.push(format!("{args:?} ended with {code:?}: {said}"));
        }
    };
    for json in [false, true] {
        let with = |args: &[&'static str]| -> Vec<&'static str> {
            let mut args = args.to_vec();
            if json {
                args.push("--json");
            }
            args
        };
        let git = support::Env::new();
        git.commit("a.txt", "a\n", "First");
        quiet(&git, &with(&["import"]));
        let env = support::Env::empty();
        quiet(&env, &with(&["start"]));
        env.overwrite("a.txt", "a\n");
        quiet(&env, &with(&["save", "Add a"]));
        quiet(&env, &with(&["save", "Nothing new"]));
        quiet(&env, &with(&["goto", "main"]));
        env.overwrite("a.txt", "mine\n");
        quiet(&env, &with(&["discard"]));
        quiet(&env, &with(&["discard"]));
    }
    assert!(loud.is_empty(), "{loud:#?}");
}

/// A project at its first save with one planted after it holding a link that leads out of the
/// project, which `bolt goto` writes as a plain file and warns about; that save's ID.
fn warned_about() -> (support::Env, String) {
    let link = ("out", EntryKind::Symlink, b"../outside" as &[u8]);
    let (env, ids) = support::planted_saves(&[&[link]]);
    (env, ids[0].to_hex())
}

const PLACEHOLDER_WARNING: &str = "warning: Wrote 1 link as plain files holding their targets: \
                                   they lead out of the project, or this computer can't make \
                                   links (on Windows that takes Developer Mode):\n  out\n";

#[test]
fn a_commands_warnings_reach_stderr_when_its_reader_has_gone() {
    for json in [false, true] {
        let (env, save) = warned_about();
        let mut args = vec!["goto", save.as_str()];
        if json {
            args.push("--json");
        }
        // Still no error: only the warning.
        assert_eq!(
            with_reader_gone(&env, &args),
            (Some(0), PLACEHOLDER_WARNING.to_owned())
        );
    }
}

#[cfg(any(windows, target_os = "linux"))]
#[test]
fn a_commands_warnings_come_before_the_error_when_stdout_fails() {
    let (env, save) = warned_about();
    let out = env
        .bolt_process_in(env.root())
        .args(["goto", &save])
        .stdout(support::failing_stdout(&env))
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let said = text(&out.stderr);
    let after = said.strip_prefix(PLACEHOLDER_WARNING).expect(&said);
    assert!(
        after.starts_with("error: Bolt couldn't write its output: "),
        "{said}"
    );
    assert!(
        after.ends_with(
            "\nhint: The command did its work all the same: only what Bolt printed was lost. \
             bolt status and bolt undo --list show where the project stands.\n"
        ),
        "{said}"
    );
    assert_eq!(after.lines().count(), 2, "{said}");
}

#[cfg(all(feature = "fault-injection", any(windows, target_os = "linux")))]
#[test]
fn a_warning_the_store_keeps_is_told_once_when_stdout_fails() {
    let env = support::Env::empty();
    env.bolt().arg("start").assert().success();
    let first = support::main_tip(&env).to_hex();
    env.overwrite("a.txt", "a\n");
    env.overwrite("b.txt", "b\n");
    env.bolt().args(["save", "Two files"]).assert().success();
    // A goto stopped after its first change. The next one finishes it first and says so, in its
    // own warnings and in the store's, which a failure tells too.
    let stopped = env
        .bolt()
        .args(["goto", &first])
        .env("BOLT_FAULT_FAIL", "switch:changed")
        .output()
        .unwrap();
    assert_eq!(stopped.status.code(), Some(1), "{}", text(&stopped.stderr));
    let out = env
        .bolt_process_in(env.root())
        .args(["goto", "main"])
        .stdout(support::failing_stdout(&env))
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let said = text(&out.stderr);
    let finished = "warning: Finished a switch between saves that was stopped";
    assert_eq!(said.matches(finished).count(), 1, "{said}");
}

#[cfg(any(windows, target_os = "linux"))]
#[test]
fn a_command_that_did_its_work_but_couldnt_print_says_so() {
    let env = support::Env::empty();
    env.bolt().arg("start").assert().success();
    env.overwrite("a.txt", "a\n");
    let out = env
        .bolt_process_in(env.root())
        .args(["save", "Add a"])
        .stdout(support::failing_stdout(&env))
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let said = text(&out.stderr);
    assert!(
        said.starts_with("error: Bolt couldn't write its output: "),
        "{said}"
    );
    assert!(
        said.ends_with(
            "\nhint: The command did its work all the same: only what Bolt printed was lost. \
             bolt status and bolt undo --list show where the project stands.\n"
        ),
        "{said}"
    );
    // It did: the save is made.
    let history = env.bolt().arg("history").output().unwrap();
    assert!(text(&history.stdout).contains("Add a"));
}
