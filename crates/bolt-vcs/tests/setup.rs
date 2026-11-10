//! `bolt start`'s preset, and `bolt setup`: the project's rules shown, and a preset switched.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;
use std::path::Path;

use bolt_core::presets::{self, Found, Language, Preset};

mod support;
use support::{Env, Files, files, head, text};

/// Runs `bolt` with `args` and gives the exit code, stdout and stderr.
fn run(env: &Env, args: &[&str]) -> (Option<i32>, String, String) {
    let out = env.bolt().args(args).output().unwrap();
    (out.status.code(), text(&out.stdout), text(&out.stderr))
}

fn ok(env: &Env, args: &[&str]) -> String {
    let (code, out, err) = run(env, args);
    assert_eq!(code, Some(0), "{args:?}: {err}");
    out
}

fn json(env: &Env, args: &[&str]) -> (Option<i32>, serde_json::Value) {
    let (code, out, err) = run(env, &[&["--json"], args].concat());
    let doc = serde_json::from_str(&out).unwrap_or_else(|e| panic!("{e}: {out:?} {err}"));
    (code, doc)
}

/// What one of `presets.json`'s vectors gives: its `bolt.toml` and `.boltignore` block.
fn vector(name: &str) -> (String, String) {
    let all: serde_json::Value = serde_json::from_str(include_str!(
        "../../bolt-vcs-core/tests/vectors/presets.json"
    ))
    .unwrap();
    let v = all["presets"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["name"] == name)
        .unwrap();
    (
        v["bolt_toml"].as_str().unwrap().to_owned(),
        v["boltignore_block"].as_str().unwrap().to_owned(),
    )
}

/// An empty folder that looks like a Unity project.
fn unity_folder() -> Env {
    let env = Env::empty();
    fs::create_dir_all(env.root().join("Assets")).unwrap();
    fs::create_dir_all(env.root().join("ProjectSettings")).unwrap();
    fs::write(
        env.root().join("ProjectSettings/ProjectVersion.txt"),
        "m_EditorVersion: 6000.0.0f1\n",
    )
    .unwrap();
    env
}

fn is_project(dir: &Path) -> bool {
    dir.join(".bolt").exists()
}

fn command_of_head(env: &Env) -> String {
    head(env).1.command
}

#[test]
fn start_without_a_terminal_needs_a_preset_and_names_the_flag() {
    let env = unity_folder();
    let (code, out, err) = run(&env, &["start"]);
    assert_eq!(code, Some(1), "{err}");
    assert!(out.is_empty(), "{out}");
    assert!(!is_project(env.root()));
    assert!(err.starts_with("error: "), "{err}");
    for flag in ["--preset code", "--preset game", "--preset custom"] {
        assert!(err.contains(flag), "{flag}: {err}");
    }
    assert!(err.contains("suggest --preset game"), "{err}");
    assert!(err.contains("found Unity files"), "{err}");
}

#[test]
fn start_with_json_never_asks() {
    let env = unity_folder();
    let out = env
        .bolt()
        .args(["--json", "start"])
        .env("BOLT_FAKE_TERMINAL", "80")
        .write_stdin("2\n")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(out.stderr.is_empty(), "{}", text(&out.stderr));
    let doc: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let hint = doc["error"]["hint"].as_str().unwrap();
    assert!(hint.contains("--preset game"), "{hint}");
    assert!(!is_project(env.root()));
}

#[test]
fn start_preset_custom_writes_no_file_and_is_as_before() {
    let env = Env::empty();
    fs::write(env.root().join("Cargo.toml"), "[package]\n").unwrap();
    let before = files(env.root());
    let (code, doc) = json(&env, &["start", "--preset", "custom"]);
    assert_eq!(code, Some(0), "{doc}");
    assert_eq!(doc["preset"], "custom");
    assert_eq!(doc["rules_written"], serde_json::json!([]));
    assert_eq!(files(env.root()), before);
    assert_eq!(command_of_head(&env), "start");
}

#[test]
fn start_preset_game_in_a_unity_folder_writes_unitys_rules() {
    let env = unity_folder();
    let said = ok(&env, &["start", "--preset", "game"]);
    let (toml, block) = vector("game-unity");
    assert_eq!(
        fs::read_to_string(env.root().join("bolt.toml")).unwrap(),
        toml
    );
    assert_eq!(
        fs::read_to_string(env.root().join(".boltignore")).unwrap(),
        block
    );
    assert_eq!(command_of_head(&env), "setup", "{said}");
}

#[test]
fn start_preset_code_writes_the_lists_of_the_languages_found() {
    let env = Env::empty();
    fs::write(env.root().join("Cargo.toml"), "[package]\n").unwrap();
    fs::write(env.root().join("package.json"), "{}\n").unwrap();
    let (code, doc) = json(&env, &["start", "--preset", "code"]);
    assert_eq!(code, Some(0), "{doc}");
    let found = Found {
        engines: vec![],
        languages: vec![Language::Node, Language::Rust],
    };
    assert_eq!(
        fs::read_to_string(env.root().join(".boltignore")).unwrap(),
        presets::boltignore_block(Preset::Code, &found).unwrap()
    );
    assert_eq!(
        fs::read_to_string(env.root().join("bolt.toml")).unwrap(),
        presets::bolt_toml(Preset::Code, &found).unwrap()
    );
    assert_eq!(
        doc["found"]["languages"],
        serde_json::json!(["node", "rust"])
    );
    assert_eq!(
        doc["rules_written"],
        serde_json::json!(["bolt.toml", ".boltignore"])
    );
}

#[test]
fn start_says_what_the_rules_are_in_words() {
    let env = unity_folder();
    let said = ok(&env, &["start", "--preset", "game"]);
    let lines: Vec<&str> = said.lines().collect();
    assert!(lines[0].starts_with("Started a Bolt project in "), "{said}");
    assert_eq!(lines[1], "", "{said}");
    assert_eq!(lines[2], "Using Game settings (Unity):", "{said}");
    for words in [
        "    * Files of 1 MiB or more are stored in chunks, so a changed part is stored once",
        "    * Unity's Library/, Temp/, Logs/ and other generated files are left out",
        "    * A save that adds over 1 GiB or 10,000 new files asks first",
        "    The rules are in bolt.toml and .boltignore. See them any time with: bolt setup",
    ] {
        assert!(lines.contains(&words), "{words}\n{said}");
    }
    assert!(
        lines.last().unwrap().starts_with("Your first save is "),
        "{said}"
    );
    // Code, with nothing found, says what it left out all the same.
    let env = Env::empty();
    let said = ok(&env, &["start", "--preset", "code"]);
    assert!(said.contains("Using Code settings:\n"), "{said}");
    assert!(said.contains("over 100 MB or 10,000 new files"), "{said}");
    // Custom names no rule file.
    let env = Env::empty();
    let said = ok(&env, &["start", "--preset", "custom"]);
    assert!(said.contains("Using Custom settings"), "{said}");
    assert!(!said.contains("The rules are in bolt.toml"), "{said}");
}

#[test]
fn undo_after_a_start_gives_the_folder_back() {
    let env = Env::empty();
    fs::write(env.root().join("Cargo.toml"), "[package]\n").unwrap();
    fs::write(env.root().join(".boltignore"), "mine/\n").unwrap();
    let before = files(env.root());
    ok(&env, &["start", "--preset", "code"]);
    assert_ne!(files(env.root()), before);
    let said = ok(&env, &["undo"]);
    assert!(said.contains("setup"), "{said}");
    assert_eq!(files(env.root()), before);
}

#[test]
fn a_folder_with_its_own_rules_file_keeps_it_and_is_asked_nothing() {
    let env = unity_folder();
    let own = "preset = \"team\"\nchunk-files-over = \"4 MiB\"\n";
    fs::write(env.root().join("bolt.toml"), own).unwrap();
    let before = files(env.root());
    let (code, doc) = json(&env, &["start"]);
    assert_eq!(code, Some(0), "{doc}");
    assert_eq!(doc["preset"], serde_json::Value::Null);
    assert_eq!(doc["rules_written"], serde_json::json!([]));
    assert_eq!(doc["found"]["engines"], serde_json::json!(["unity"]));
    assert_eq!(files(env.root()), before);
    assert_eq!(command_of_head(&env), "start");
    // For people it says the rules stay, in words.
    let env = Env::empty();
    fs::write(env.root().join("bolt.toml"), own).unwrap();
    let said = ok(&env, &["start"]);
    assert!(said.contains("bolt.toml"), "{said}");
    assert!(said.contains("stay"), "{said}");
    assert!(said.contains("Files of 4 MiB or more"), "{said}");
}

#[test]
fn preset_with_a_rules_file_already_there_is_refused_before_anything_is_made() {
    let env = Env::empty();
    fs::write(env.root().join("bolt.toml"), "preset = \"code\"\n").unwrap();
    let (code, out, err) = run(&env, &["start", "--preset", "game"]);
    assert_eq!(code, Some(1), "{err}");
    assert!(out.is_empty());
    assert!(!is_project(env.root()));
    assert!(err.contains("bolt setup --preset"), "{err}");
    // A folder by that name counts too.
    let env = Env::empty();
    fs::create_dir(env.root().join("bolt.toml")).unwrap();
    let (code, _, err) = run(&env, &["start", "--preset", "code"]);
    assert_eq!(code, Some(1), "{err}");
    assert!(!is_project(env.root()));
}

#[test]
fn a_start_whose_rule_files_cant_be_written_says_how_to_write_them() {
    for json in [false, true] {
        let env = Env::empty();
        fs::create_dir(env.root().join(".boltignore")).unwrap();
        let args: &[&str] = if json {
            &["--json", "start", "--preset", "code"]
        } else {
            &["start", "--preset", "code"]
        };
        let (code, out, err) = run(&env, args);
        assert_eq!(code, Some(1), "{err}");
        // The project is there, with no rule files yet.
        assert!(is_project(env.root()));
        assert!(!env.root().join("bolt.toml").exists());
        assert_eq!(command_of_head(&env), "start");
        if json {
            let doc: serde_json::Value = serde_json::from_str(&out).unwrap();
            assert_eq!(doc["preset"], "code");
            assert_eq!(doc["rules_written"], serde_json::json!([]));
            let hint = doc["error"]["hint"].as_str().unwrap();
            assert!(hint.contains("bolt setup --preset code"), "{hint}");
        } else {
            assert!(err.contains("The project was started"), "{err}");
            assert!(err.contains("bolt setup --preset code"), "{err}");
        }
    }
}

#[test]
fn an_unknown_preset_is_refused_by_the_command_line() {
    let env = Env::empty();
    let (code, _, err) = run(&env, &["start", "--preset", "gmae"]);
    assert_eq!(code, Some(2), "{err}");
    assert!(err.contains("game"), "{err}");
    assert!(!is_project(env.root()));
}

/// A name that would drive a terminal, where this system can hold one, that makes the folder look
/// like an Unreal project.
fn planted_name() -> &'static str {
    if cfg!(windows) {
        "evil\u{202e}dlrow.uproject"
    } else {
        "\x1b[31mevil.uproject"
    }
}

#[test]
fn what_start_says_about_the_folder_names_nothing_from_it() {
    let env = Env::empty();
    fs::write(env.root().join(planted_name()), "{}\n").unwrap();
    let (code, _, err) = run(&env, &["start"]);
    assert_eq!(code, Some(1), "{err}");
    assert!(err.contains("found Unreal files"), "{err}");
    assert!(!err.contains("evil"), "{err}");
    assert!(!err.contains('\x1b'), "{err:?}");
}

#[test]
fn start_json_has_preset_found_and_rules_written() {
    let env = unity_folder();
    let (code, mut doc) = json(&env, &["start", "--preset", "game"]);
    assert_eq!(code, Some(0), "{doc}");
    let elevated = bolt_core::repo::elevated_only_note(env.root()).is_some();
    assert_eq!(
        doc["warnings"].as_array().unwrap().len(),
        usize::from(elevated)
    );
    doc["warnings"] = serde_json::json!([]);
    doc["root"] = "[redacted]".into();
    doc["save"] = "[redacted]".into();
    insta::assert_json_snapshot!(doc);
}

#[test]
fn setup_shows_the_rules_as_bolt_reads_them() {
    let env = unity_folder();
    ok(&env, &["start", "--preset", "game"]);
    let said = ok(&env, &["setup"]);
    let lines: Vec<&str> = said.lines().collect();
    assert_eq!(lines[0], "Using game settings, from bolt.toml:", "{said}");
    for words in [
        "Files stored in chunks",
        "1 MiB or more",
        "over 1 GiB or 10,000 new files",
        "*.psd",
        "*.unity",
        "lines in .boltignore",
    ] {
        assert!(said.contains(words), "{words}\n{said}");
    }
    // It changed nothing, and took no step.
    assert_eq!(command_of_head(&env), "setup");
    let (code, doc) = json(&env, &["setup"]);
    assert_eq!(code, Some(0));
    assert_eq!(doc["rules_file"], true);
    assert_eq!(doc["preset"], "game");
    assert_eq!(doc["chunk_files_over"], 1 << 20);
    assert_eq!(doc["ask_before_saving_over"], 1_u64 << 30);
    assert_eq!(doc["ask_before_saving_files_over"], 10_000);
    let block = vector("game-unity").1;
    assert_eq!(
        doc["ignore_lines"],
        presets::block_lines(block.as_bytes()).unwrap()
    );
    assert_eq!(doc["rules"][0]["lockable"], true);
}

#[test]
fn setup_without_a_rules_file_shows_bolts_own_values() {
    let env = Env::empty();
    ok(&env, &["start", "--preset", "custom"]);
    let said = ok(&env, &["setup"]);
    assert!(said.starts_with("No bolt.toml here"), "{said}");
    assert!(said.contains("Bolt's own values"), "{said}");
    assert!(said.contains("over 100 MB or 10,000 new files"), "{said}");
    let (_, doc) = json(&env, &["setup"]);
    assert_eq!(doc["rules_file"], false);
    assert_eq!(doc["preset"], serde_json::Value::Null);
    assert_eq!(doc["ask_before_saving_over"], 100_000_000);
    assert_eq!(doc["ignore_lines"], serde_json::Value::Null);
    assert_eq!(doc["rules"], serde_json::json!([]));
}

#[test]
fn setup_shows_what_in_the_rules_file_was_passed_over() {
    let env = Env::empty();
    ok(&env, &["start", "--preset", "custom"]);
    fs::write(env.root().join("bolt.toml"), "colour = \"blue\"\n").unwrap();
    let (code, _, err) = run(&env, &["setup"]);
    assert_eq!(code, Some(0));
    assert!(err.starts_with("warning: "), "{err}");
    assert!(err.contains("colour"), "{err}");
    let (_, doc) = json(&env, &["setup"]);
    assert_eq!(doc["warnings"][0]["kind"], "rules_ignored");
    fs::write(env.root().join("bolt.toml"), "not toml [").unwrap();
    let (_, doc) = json(&env, &["setup"]);
    assert_eq!(doc["rules_file"], false);
    assert_eq!(doc["warnings"][0]["kind"], "rules_file_skipped");
    let said = ok(&env, &["setup"]);
    assert!(
        said.starts_with("bolt.toml couldn't be used, so Bolt's own values apply:"),
        "{said}"
    );
}

#[cfg(feature = "fault-injection")]
#[test]
fn a_start_without_its_rule_files_tells_its_warnings_before_the_error() {
    let env = Env::empty();
    fs::create_dir(env.root().join(".boltignore")).unwrap();
    let out = env
        .bolt()
        .env("BOLT_FAULT_FAIL", "start:read-back")
        .args(["start", "--preset", "code"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let err = text(&out.stderr);
    let warned = err.find("warning: The project was made, but Bolt couldn't read its first save");
    let failed = err.find("error: The project was started, but its rule files weren't written");
    assert!(warned.is_some() && warned < failed, "{err}");
}

#[test]
fn setup_preset_shows_the_line_changes_then_stops_without_yes() {
    let env = Env::empty();
    ok(&env, &["start", "--preset", "code"]);
    let before = files(env.root());
    let entry = head(&env).0;
    let (code, out, err) = run(&env, &["setup", "--preset", "game"]);
    assert_eq!(code, Some(1), "{err}");
    assert!(out.contains("-preset = \"code\""), "{out}");
    assert!(out.contains("+preset = \"game\""), "{out}");
    assert!(out.contains("@@ "), "{out}");
    assert!(err.starts_with("error: "), "{err}");
    assert!(err.contains("--yes"), "{err}");
    assert_eq!(files(env.root()), before);
    assert_eq!(head(&env).0, entry);
}

#[test]
fn setup_preset_yes_applies_and_undo_takes_it_back() {
    let env = Env::empty();
    ok(&env, &["start", "--preset", "code"]);
    let before: Files = files(env.root());
    let said = ok(&env, &["setup", "--preset", "game", "--yes"]);
    assert!(said.contains("Switched to Game settings"), "{said}");
    assert_eq!(command_of_head(&env), "setup");
    let (toml, _) = vector("game-no-engine");
    assert_eq!(
        fs::read_to_string(env.root().join("bolt.toml")).unwrap(),
        toml
    );
    ok(&env, &["undo"]);
    assert_eq!(files(env.root()), before);
    // Custom takes both files away.
    ok(&env, &["setup", "--preset", "custom", "--yes"]);
    assert!(files(env.root()).is_empty());
}

#[test]
fn setup_preset_with_nothing_to_change_says_so() {
    let env = Env::empty();
    ok(&env, &["start", "--preset", "code"]);
    let entry = head(&env).0;
    let said = ok(&env, &["setup", "--preset", "code"]);
    assert!(said.starts_with("Nothing to change"), "{said}");
    assert_eq!(head(&env).0, entry);
    let (code, doc) = json(&env, &["setup", "--preset", "code"]);
    assert_eq!(code, Some(0));
    assert_eq!(doc["changes"], serde_json::json!([]));
    assert_eq!(doc["applied"], false);
}

#[test]
fn setup_json_matches_json_md() {
    let env = Env::empty();
    ok(&env, &["start", "--preset", "code"]);
    let (code, shown) = json(&env, &["setup"]);
    assert_eq!(code, Some(0));
    insta::assert_json_snapshot!("setup_shown", shown);
    // Without --yes: what would change, and the error naming it.
    let (code, stopped) = json(&env, &["setup", "--preset", "custom"]);
    assert_eq!(code, Some(1));
    assert!(stopped["error"]["hint"].as_str().unwrap().contains("--yes"));
    insta::assert_json_snapshot!("setup_stopped", stopped);
    let (code, applied) = json(&env, &["setup", "--preset", "custom", "--yes"]);
    assert_eq!(code, Some(0));
    insta::assert_json_snapshot!("setup_applied", applied);
}

#[test]
fn a_preview_escapes_the_lines_it_shows() {
    let env = Env::empty();
    ok(&env, &["start", "--preset", "custom"]);
    fs::write(
        env.root().join(".boltignore"),
        "# >>> Bolt preset\n\x1b[31mred/\n# <<< Bolt preset\n",
    )
    .unwrap();
    let (code, out, _) = run(&env, &["setup", "--preset", "code"]);
    assert_eq!(code, Some(1));
    assert!(out.contains("red/"), "{out}");
    assert!(!out.contains('\x1b'), "{out:?}");
}

#[test]
fn setup_refuses_a_rules_file_that_is_a_folder() {
    let env = Env::empty();
    ok(&env, &["start", "--preset", "custom"]);
    fs::create_dir(env.root().join(".boltignore")).unwrap();
    let (code, _, err) = run(&env, &["setup", "--preset", "code", "--yes"]);
    assert_eq!(code, Some(1), "{err}");
    assert!(err.contains("isn't a regular file"), "{err}");
    assert!(err.contains("hint: "), "{err}");
}

#[cfg(feature = "fault-injection")]
mod on_a_terminal {
    use super::*;

    /// `bolt` in the project's folder as if at a terminal 80 columns wide, plain, with `input`
    /// typed in.
    fn asked(env: &Env, args: &[&str], input: &str) -> (Option<i32>, String, String) {
        let out = env
            .bolt()
            .args(args)
            .env("BOLT_FAKE_TERMINAL", "80")
            .env("BOLT_PROGRESS_QUIET_MS", "600000")
            .env("BOLT_ASCII", "1")
            .env("NO_COLOR", "1")
            .write_stdin(input)
            .output()
            .unwrap();
        (out.status.code(), text(&out.stdout), text(&out.stderr))
    }

    #[test]
    fn the_question_shows_the_look_and_the_suggestion() {
        let env = unity_folder();
        let (code, out, err) = asked(&env, &["start"], "\n");
        assert_eq!(code, Some(0), "{err}");
        let shown = err.split("Choice [2]: ").next().unwrap();
        assert!(
            shown.starts_with("Looks like a game project: found Unity files and "),
            "{shown}"
        );
        assert!(
            shown.contains("\n\nWhat kind of project is this?\n\n"),
            "{shown}"
        );
        assert!(
            shown.contains("    1. Code     apps, websites, libraries\n"),
            "{shown}"
        );
        assert!(
            shown.contains("    2. Game     Unity, Unreal, Godot, big assets   <- suggested\n"),
            "{shown}"
        );
        assert!(
            shown.contains("    3. Custom   Bolt's plain defaults, no rule files\n"),
            "{shown}"
        );
        // An empty answer took the suggestion.
        assert!(out.contains("Using Game settings (Unity):"), "{out}");
        assert!(err.contains("Next: "), "{err}");
    }

    #[test]
    fn an_answer_is_a_number_or_a_name_in_any_case() {
        for (answer, preset) in [
            ("1\n", "Code"),
            ("GAME\n", "Game"),
            (" custom \n", "Custom"),
        ] {
            let env = Env::empty();
            let (code, out, err) = asked(&env, &["start"], answer);
            assert_eq!(code, Some(0), "{err}");
            assert!(out.contains(&format!("Using {preset} settings")), "{out}");
        }
    }

    #[test]
    fn a_wrong_answer_asks_again_three_times_at_most() {
        let env = Env::empty();
        let (code, out, err) = asked(&env, &["start"], "4\nyes\n2\n");
        assert_eq!(code, Some(0), "{err}");
        assert_eq!(err.matches("Choice [3]: ").count(), 3, "{err}");
        assert!(out.contains("Using Game settings"), "{out}");
        let env = Env::empty();
        let (code, _, err) = asked(&env, &["start"], "x\ny\nz\n2\n");
        assert_eq!(code, Some(1), "{err}");
        assert!(err.contains("--preset custom"), "{err}");
        assert!(!is_project(env.root()));
        // The end of the input stops the same way.
        let (code, _, err) = asked(&env, &["start"], "");
        assert_eq!(code, Some(1), "{err}");
        assert!(!is_project(env.root()));
    }

    #[test]
    fn the_question_names_nothing_from_the_folder() {
        let env = Env::empty();
        fs::write(env.root().join(planted_name()), "{}\n").unwrap();
        let (code, _, err) = asked(&env, &["start"], "3\n");
        assert_eq!(code, Some(0), "{err}");
        assert!(err.contains("found Unreal files"), "{err}");
        assert!(!err.contains("evil"), "{err}");
    }

    #[test]
    fn a_folder_with_a_rules_file_is_asked_nothing() {
        let env = Env::empty();
        fs::write(env.root().join("bolt.toml"), "preset = \"code\"\n").unwrap();
        let (code, _, err) = asked(&env, &["start"], "");
        assert_eq!(code, Some(0), "{err}");
        assert!(!err.contains("Choice"), "{err}");
    }

    #[test]
    fn setup_preset_asks_before_it_writes() {
        let env = Env::empty();
        ok(&env, &["start", "--preset", "code"]);
        let before = files(env.root());
        let entry = head(&env).0;
        for no in ["n\n", "\n", ""] {
            let (code, out, err) = asked(&env, &["setup", "--preset", "game"], no);
            assert_eq!(code, Some(0), "{err}");
            assert!(err.contains("Apply these changes? [y/N] "), "{err}");
            assert!(out.contains("Nothing was changed"), "{out}");
            assert_eq!(files(env.root()), before);
            assert_eq!(head(&env).0, entry);
        }
        let (code, out, err) = asked(&env, &["setup", "--preset", "game"], "y\n");
        assert_eq!(code, Some(0), "{err}");
        assert!(out.contains("Switched to Game settings"), "{out}");
        assert!(err.ends_with("Changed your mind? bolt undo\n"), "{err:?}");
        assert_eq!(command_of_head(&env), "setup");
    }
}
