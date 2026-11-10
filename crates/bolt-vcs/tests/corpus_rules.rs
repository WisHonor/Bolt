//! Projects that arrive with hostile rule files (security.md T6 and T21, storage-format.md §12): a
//! `bolt.toml` or `.boltignore` that is a link, a folder, a pipe or too long, TOML built to wear
//! the parser out, numbers at every limit, text that would drive the terminal, and folders planted
//! to fool the look that suggests a preset. Every command ends, in bounded time, says what it
//! passed over, escaped, and writes nothing it shouldn't.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use std::fs;
use std::path::Path;
use std::time::Instant;

use bolt_core::pack::ObjectKind;
use bolt_core::presets::{self, Engine, Found, LOOK_AT_MOST, Language, Preset};
use serde_json::Value;
use support::{Env, copies_of, head, link_file, link_folder, run_limited, text, write_noise};

/// `bolt <args>` in the project's folder: its exit code, stdout and stderr.
fn run(env: &Env, args: &[&str]) -> (Option<i32>, String, String) {
    let out = env.bolt().args(args).output().unwrap();
    (out.status.code(), text(&out.stdout), text(&out.stderr))
}

/// `bolt --json <args>`: its exit code and document.
fn json(env: &Env, args: &[&str]) -> (Option<i32>, Value) {
    let (code, out, err) = run(env, &[&["--json"], args].concat());
    let doc = serde_json::from_str(&out).unwrap_or_else(|e| panic!("{args:?}: {e}: {out:?} {err}"));
    (code, doc)
}

/// A project just started, with no rule files.
fn started() -> Env {
    let env = Env::empty();
    let (code, _, err) = run(&env, &["start", "--preset", "custom"]);
    assert_eq!(code, Some(0), "{err}");
    env
}

/// The kinds of a document's warnings about `bolt.toml`.
fn rules_warnings(doc: &Value) -> Vec<String> {
    let about = ["rules_file_skipped", "rules_ignored"];
    doc["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| w["kind"].as_str().unwrap().to_owned())
        .filter(|k| about.contains(&k.as_str()))
        .collect()
}

/// The message of a document's warning of `kind`.
fn message_of(doc: &Value, kind: &str) -> String {
    let warnings = doc["warnings"].as_array().unwrap();
    let w = warnings.iter().find(|w| w["kind"] == kind);
    w.unwrap_or_else(|| panic!("no {kind}: {doc}"))["message"]
        .as_str()
        .unwrap()
        .to_owned()
}

/// How long a command over a hostile file may take before the test calls it hung. Debug builds
/// on CI's slowest machine finish each of these in a few seconds.
const LIMIT_SECS: u64 = 120;

/// `bolt <args>` in the project, killed if it runs past [`LIMIT_SECS`]: its exit code and stderr.
fn bounded(env: &Env, args: &[&str]) -> (Option<i32>, String) {
    let mut cmd = env.bolt_process_in(env.root());
    cmd.args(args);
    let ended = run_limited(cmd, LIMIT_SECS);
    assert!(!ended.timed_out, "{args:?} ran past {LIMIT_SECS} s");
    eprintln!("{args:?} took {:.1} s", ended.secs);
    (ended.code, ended.stderr)
}

/// What `bolt save` and `bolt setup` make of the project's `bolt.toml`: each must work, and say
/// the same about the file; the setup document.
fn saved_and_shown(env: &Env, file: &str) -> Value {
    fs::write(env.root().join("bolt.toml"), file).unwrap();
    // Some files ask before every save.
    let (code, saved) = json(env, &["save", "--yes", "Rules"]);
    assert_eq!(code, Some(0), "{saved}");
    let (code, shown) = json(env, &["setup"]);
    assert_eq!(code, Some(0), "{shown}");
    assert_eq!(rules_warnings(&saved), rules_warnings(&shown), "{shown}");
    shown
}

/// Bolt's own values, as `bolt setup --json` gives them.
fn own_values(doc: &Value) -> bool {
    doc["chunk_files_over"] == 1 << 20
        && doc["ask_before_saving_over"] == 100_000_000
        && doc["ask_before_saving_files_over"] == 10_000
        && doc["preset"].is_null()
        && doc["rules"] == serde_json::json!([])
}

// The file that isn't one.

#[test]
fn a_rules_file_that_is_a_link_out_of_the_project() {
    let outside = tempfile::tempdir().unwrap();
    let secret = "preset = \"the-secret\"\nchunk-files-over = \"8 MiB\"\n";
    fs::write(outside.path().join("rules.toml"), secret).unwrap();
    fs::create_dir(outside.path().join("folder")).unwrap();
    fs::write(outside.path().join("folder/bolt.toml"), secret).unwrap();
    let before = support::files(outside.path());
    for to_a_file in [false, true] {
        let env = started();
        let link = env.root().join("bolt.toml");
        if to_a_file {
            if !link_file(&outside.path().join("rules.toml"), &link) {
                continue;
            }
        } else {
            link_folder(&outside.path().join("folder"), &link);
        }
        fs::write(env.root().join("a.txt"), "a\n").unwrap();
        let (code, saved) = json(&env, &["save", "With a link"]);
        assert_eq!(code, Some(0), "{saved}");
        assert_eq!(rules_warnings(&saved), ["rules_file_skipped"], "{saved}");
        let why = message_of(&saved, "rules_file_skipped");
        assert!(why.contains("a link"), "{why}");
        assert!(!why.contains("secret"), "{why}");
        let (code, shown) = json(&env, &["setup"]);
        assert_eq!(code, Some(0), "{shown}");
        assert_eq!(shown["has_rules_file"], false, "{shown}");
        assert!(own_values(&shown), "{shown}");
        // A preset is never written through it.
        let entry = head(&env).0;
        let (code, _, err) = run(&env, &["setup", "--preset", "code", "--yes"]);
        assert_eq!(code, Some(1), "{err}");
        assert!(err.contains("bolt.toml isn't a regular file"), "{err}");
        assert_eq!(head(&env).0, entry);
        assert!(!env.root().join(".boltignore").exists());
        assert_eq!(support::files(outside.path()), before);
    }
}

#[test]
fn a_rules_file_that_is_a_folder_a_pipe_or_2_mib() {
    let mut big = b"chunk-files-over = \"8 MiB\"\n#".to_vec();
    big.resize(2 << 20, b'x');
    for (case, why) in [
        ("folder", "not a regular file"),
        ("2 MiB", "larger than 1 MiB"),
    ] {
        let env = started();
        let at = env.root().join("bolt.toml");
        if case == "folder" {
            fs::create_dir(&at).unwrap();
            fs::write(at.join("inside.txt"), "x\n").unwrap();
        } else {
            fs::write(&at, &big).unwrap();
        }
        fs::write(env.root().join("a.txt"), "a\n").unwrap();
        let (code, saved) = json(&env, &["save", "Odd rules"]);
        assert_eq!(code, Some(0), "{case}: {saved}");
        assert_eq!(rules_warnings(&saved), ["rules_file_skipped"], "{case}");
        let message = message_of(&saved, "rules_file_skipped");
        assert!(message.contains(why), "{case}: {message}");
        let (_, shown) = json(&env, &["setup"]);
        assert_eq!(shown["has_rules_file"], false, "{case}");
        assert!(own_values(&shown), "{case}: {shown}");
        let (code, said, _) = run(&env, &["setup"]);
        assert_eq!(code, Some(0));
        assert!(
            said.starts_with("bolt.toml couldn't be used, so Bolt's own values apply:"),
            "{case}: {said}"
        );
    }
    // A pipe would hang a reader that waits for its writer.
    #[cfg(unix)]
    {
        let env = started();
        let made = std::process::Command::new("mkfifo")
            .arg(env.root().join("bolt.toml"))
            .status()
            .unwrap();
        assert!(made.success());
        let (code, said) = bounded(&env, &["setup"]);
        assert_eq!(code, Some(0), "{said}");
        assert!(said.contains("Bolt passed over bolt.toml"), "{said}");
        // A save meets the pipe in its walk too: it ends, whatever it makes of it.
        bounded(&env, &["save", "With a pipe"]);
    }
}

#[test]
fn a_rules_file_of_nested_tables_arrays_and_brackets() {
    let deep = 300_000;
    let dotted = vec!["a"; deep].join(".");
    let skipped = [
        "[".repeat(1 << 19),
        format!("a = {}", "[".repeat(1 << 19)),
        format!("a = {}1{}", "[".repeat(deep), "]".repeat(deep)),
        format!("a = {}", "{ b = ".repeat(deep / 2)),
        format!("[{dotted}]"),
        format!("[[{dotted}]]"),
    ];
    let env = started();
    for file in &skipped {
        assert!(file.len() <= 1 << 20);
        let shown = saved_and_shown(&env, file);
        assert_eq!(rules_warnings(&shown), ["rules_file_skipped"]);
        assert!(own_values(&shown), "{shown}");
    }
    // Deep keys the parser may take: passed over whole or by name, never a crash.
    for file in [
        format!("{dotted} = 1\n"),
        format!("[[rule]]\nfiles = [\"a\"]\n{dotted} = 1\n"),
    ] {
        let shown = saved_and_shown(&env, &file);
        assert_eq!(rules_warnings(&shown).len(), 1, "{shown}");
    }
    // Shapes Bolt doesn't take where it wants a list of rules, or a rule's list of patterns.
    let shown = saved_and_shown(&env, "rule = { files = [\"*.psd\"] }\n");
    let message = message_of(&shown, "rules_ignored");
    assert!(
        message.contains("rule: it must be a list of rules"),
        "{message}"
    );
    let file = "[[rule]]\nfiles = [[\"*.psd\"]]\n\n[[rule]]\n[rule.files]\na = 1\n\n[[rule]]\nfiles \
                = [\"*.png\"]\n[[rule.x]]\ny = 1\n[[rule]]\nfiles = [\"*.wav\", 1]\n";
    let shown = saved_and_shown(&env, file);
    let message = message_of(&shown, "rules_ignored");
    for name in ["rule[1]: ", "rule[2]: ", "rule[3].x: ", "rule[4]: "] {
        assert!(message.contains(name), "{name}: {message}");
    }
    assert_eq!(shown["rules"][0]["files"], serde_json::json!(["*.png"]));
    assert_eq!(shown["rules"].as_array().unwrap().len(), 1, "{shown}");
}

#[test]
fn a_rules_file_with_numbers_at_every_limit() {
    let env = started();
    let at_the_limits = [
        (
            format!(
                "preset = \"{}\"\nchunk-files-over = \"128 GiB\"\nask-before-saving-over = \
                 \"18446744073709551615 B\"\nask-before-saving-files-over = 9223372036854775807\n",
                "p".repeat(64)
            ),
            (137_438_953_472, u64::MAX, i64::MAX as u64),
        ),
        (
            format!(
                "preset = \"{}\"\nchunk-files-over = \"1 MiB\"\nask-before-saving-over = \"0 B\"\n\
                 ask-before-saving-files-over = 0\n",
                "é".repeat(32)
            ),
            (1 << 20, 0, 0),
        ),
        (
            "preset = \"\"\nchunk-files-over = \"1048576 B\"\nask-before-saving-over = \"16777215 \
             TiB\"\nask-before-saving-files-over = 0x10\n"
                .to_owned(),
            (1 << 20, 16_777_215 << 40, 16),
        ),
    ];
    for (file, (chunks, bytes, files)) in at_the_limits {
        let shown = saved_and_shown(&env, &file);
        assert!(rules_warnings(&shown).is_empty(), "{file}: {shown}");
        assert_eq!(shown["has_rules_file"], true);
        assert_eq!(shown["chunk_files_over"].as_u64(), Some(chunks), "{file}");
        assert_eq!(
            shown["ask_before_saving_over"].as_u64(),
            Some(bytes),
            "{file}"
        );
        assert_eq!(
            shown["ask_before_saving_files_over"].as_u64(),
            Some(files),
            "{file}"
        );
        let label = file.lines().next().unwrap();
        let label = &label["preset = \"".len()..label.len() - 1];
        assert_eq!(shown["preset"], label);
    }
    // One past each: each key passed over by name, in byte order, and Bolt's own value used.
    let past = [
        format!(
            "preset = \"{}\"\nchunk-files-over = \"137438953473 B\"\nask-before-saving-over = \
             \"18446744073709551616 B\"\nask-before-saving-files-over = -1\n",
            "p".repeat(65)
        ),
        format!(
            "preset = \"{}\"\nchunk-files-over = \"1048575 B\"\nask-before-saving-over = \
             \"16777216 TiB\"\nask-before-saving-files-over = 1e3\n",
            "é".repeat(33)
        ),
    ];
    for file in past {
        let shown = saved_and_shown(&env, &file);
        assert_eq!(rules_warnings(&shown), ["rules_ignored"], "{shown}");
        assert!(own_values(&shown), "{shown}");
        let message = message_of(&shown, "rules_ignored");
        let at = |key: &str| message.find(&format!("\n  {key}: ")).unwrap_or(usize::MAX);
        let order = [
            "ask-before-saving-files-over",
            "ask-before-saving-over",
            "chunk-files-over",
            "preset",
        ]
        .map(at);
        assert!(order.windows(2).all(|w| w[0] < w[1]), "{message}");
        assert!(order[3] < usize::MAX, "{message}");
    }
    // A whole number TOML can't hold makes the file no TOML at all.
    let shown = saved_and_shown(&env, "ask-before-saving-files-over = 9223372036854775808\n");
    assert_eq!(rules_warnings(&shown), ["rules_file_skipped"], "{shown}");
    // The file's own size: 1 MiB is read, a byte more is passed over whole.
    let mut file = "chunk-files-over = \"2 MiB\"\n#".to_owned();
    file.push_str(&"x".repeat((1 << 20) - file.len()));
    let shown = saved_and_shown(&env, &file);
    assert_eq!(shown["chunk_files_over"], 2 << 20, "{shown}");
    file.push('x');
    let shown = saved_and_shown(&env, &file);
    assert_eq!(rules_warnings(&shown), ["rules_file_skipped"]);
    assert!(own_values(&shown));
}

/// Escape codes Bolt itself never prints, so any found raw came from the project.
const CLEAR: &str = "\u{1b}[2J";
const TITLE: &str = "\u{1b}]0;pwned\u{7}";

/// Bolt's own colour codes and the line erase, which the checks below look past.
const BOLTS_OWN: [&str; 14] = [
    "\x1b[0m",
    "\x1b[1;32m",
    "\x1b[1;31m",
    "\x1b[1;33m",
    "\x1b[1;36m",
    "\x1b[36m",
    "\x1b[1;35m",
    "\x1b[34m",
    "\x1b[32m",
    "\x1b[31m",
    "\x1b[33m",
    "\x1b[2m",
    "\x1b[1m",
    "\x1b[2K",
];

/// Fails unless `said` holds no control character but its line breaks (and the carriage returns
/// progress redraws with), no character that hides or reorders text, and no escape code but
/// Bolt's own colours.
fn assert_escaped(said: &str, what: &str) {
    let mut plain = said.to_owned();
    for code in BOLTS_OWN {
        plain = plain.replace(code, "");
    }
    for c in plain.chars() {
        let hides = matches!(c, '\u{202e}' | '\u{2028}' | '\u{200b}');
        assert!(
            !hides && (!c.is_control() || c == '\n' || c == '\r'),
            "{what}: {c:?} in {said:?}"
        );
    }
}

/// `bolt <args>` in `dir` on a terminal 80 columns wide that shows colour and symbols.
#[cfg(feature = "fault-injection")]
fn on_a_terminal(env: &Env, dir: &Path, args: &[&str]) -> (Option<i32>, String, String) {
    let out = env
        .bolt_in(dir)
        .args(args)
        .env("BOLT_FAKE_TERMINAL", "80")
        .env("BOLT_PROGRESS_QUIET_MS", "600000")
        .env("TERM", "xterm-256color")
        .env("BOLT_ASCII", "0")
        .env_remove("NO_COLOR")
        .output()
        .unwrap();
    (out.status.code(), text(&out.stdout), text(&out.stderr))
}

#[test]
fn a_rules_file_whose_keys_and_values_hold_escape_codes() {
    let label = format!("{TITLE}\u{202e}x");
    let toml = |s: &str| {
        s.replace('\u{1b}', "\\u001b")
            .replace('\u{7}', "\\u0007")
            .replace('\u{202e}', "\\u202e")
            .replace('\u{2028}', "\\u2028")
    };
    let file = format!(
        "preset = \"{}\"\n\"{}key\" = 1\nchunk-files-over = \"{}1 MiB\"\n\n[[rule]]\nfiles = \
         [\"{}*.psd\u{2028}x\"]\nlockable = true\nconflict-markers = \"{}\"\n",
        toml(&label),
        toml(&format!("{CLEAR}\u{202e}")),
        toml(CLEAR),
        toml(CLEAR),
        toml(TITLE)
    );
    let file = file.replace('\u{2028}', "\\u2028");
    // `start` in a folder that brings the file, then `save` and `setup` in the project.
    let env = Env::empty();
    fs::write(env.root().join("bolt.toml"), &file).unwrap();
    let mut ways: Vec<Option<bool>> = vec![None];
    if cfg!(feature = "fault-injection") {
        ways.push(Some(true));
    }
    let said_by = |env: &Env, args: &[&str], terminal: Option<bool>| {
        let (code, out, err) = match terminal {
            None => run(env, args),
            #[cfg(feature = "fault-injection")]
            Some(_) => on_a_terminal(env, env.root(), args),
            #[cfg(not(feature = "fault-injection"))]
            Some(_) => unreachable!(),
        };
        assert_eq!(code, Some(0), "{args:?}: {err}");
        assert_escaped(&out, &format!("{args:?} stdout"));
        assert_escaped(&err, &format!("{args:?} stderr"));
        (out, err)
    };
    let (_, err) = said_by(&env, &["start"], None);
    assert!(
        err.contains("warning: Bolt passed over these parts"),
        "{err}"
    );
    assert!(err.contains("\\x1b[2J\\u{202e}key"), "{err}");
    for (n, terminal) in ways.into_iter().enumerate() {
        fs::write(env.root().join("a.txt"), format!("{n}\n")).unwrap();
        let (_, err) = said_by(&env, &["save", "Rules"], terminal);
        assert!(err.contains("\\x1b[2J1 MiB"), "{err}");
        assert!(
            err.contains("rule[1].conflict-markers: it must be true or false"),
            "{err}"
        );
        let (out, err) = said_by(&env, &["setup"], terminal);
        assert!(out.contains("\\x1b]0;pwned\\x07\\u{202e}x"), "{out}");
        assert!(out.contains("\\x1b[2J*.psd\\u{2028}x"), "{out}");
        assert!(err.contains("\\x1b[2J\\u{202e}key"), "{err}");
    }
    // With --json the label and patterns are data, as stored; Bolt's messages are escaped, and
    // nothing raw reaches the stream.
    let out = env.bolt().args(["--json", "setup"]).output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    assert!(!out.stdout.contains(&0x1b) && !out.stdout.contains(&0x07));
    let doc: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(doc["preset"], label.as_str());
    assert_eq!(
        doc["rules"][0]["files"][0],
        format!("{CLEAR}*.psd\u{2028}x")
    );
    for w in doc["warnings"].as_array().unwrap() {
        assert_escaped(w["message"].as_str().unwrap(), "a warning in the document");
    }
    let (_, saved) = json(&env, &["save", "Again"]);
    assert_escaped(&message_of(&saved, "rules_ignored"), "save's warning");
}

#[test]
fn a_rules_file_with_a_million_rules_under_the_size_limit() {
    // As many rules as fit the 1 MiB cap: 349,522 empty ones, each passed over, and 37,448 that
    // each name a pattern and count.
    let cap = 1 << 20;
    let mut empty = "rule = [".to_owned();
    while empty.len() + 3 + 2 <= cap {
        empty.push_str("{},");
    }
    empty.push_str("]\n");
    let one = "{files=[\"a\"],lockable=true},";
    let mut usable = "rule = [".to_owned();
    while usable.len() + one.len() + 2 <= cap {
        usable.push_str(one);
    }
    usable.push_str("]\n");
    let count = |file: &str, item: &str| (file.len() - "rule = []\n".len()) / item.len();
    let env = started();
    for (file, kept, passed_over) in [
        (&empty, 0, count(&empty, "{},")),
        (&usable, count(&usable, one), 0),
    ] {
        assert!(file.len() <= cap && kept + passed_over > 30_000);
        fs::write(env.root().join("bolt.toml"), file).unwrap();
        let started_at = Instant::now();
        for args in [
            &["setup"][..],
            &["--json", "setup"],
            &["save", "Many rules"],
        ] {
            let (code, said) = bounded(&env, args);
            assert_eq!(code, Some(0), "{args:?}: {said}");
        }
        eprintln!("{kept} and {passed_over}: {:?}", started_at.elapsed());
        let (_, shown) = json(&env, &["setup"]);
        assert_eq!(shown["rules"].as_array().unwrap().len(), kept);
        if passed_over > 0 {
            let message = message_of(&shown, "rules_ignored");
            assert!(
                message.contains(&format!("and {} more", passed_over - 20)),
                "{message}"
            );
            assert!(message.len() < 4_000, "{}", message.len());
        }
    }
}

// `.boltignore`.

#[test]
fn a_boltignore_with_ten_thousand_start_marks() {
    let block = presets::boltignore_block(Preset::Code, &Found::default()).unwrap();
    // Ten thousand blocks: only the first is replaced, every other byte stays.
    let mut blocks = "mine/\n".to_owned();
    let first = blocks.len();
    for i in 0..10_000 {
        blocks.push_str(&format!(
            "# >>> Bolt preset: x\nold{i}/\n# <<< Bolt preset\n"
        ));
    }
    let end = first + "# >>> Bolt preset: x\nold0/\n# <<< Bolt preset\n".len();
    let after_blocks = format!("{}{block}{}", &blocks[..first], &blocks[end..]);
    // Ten thousand start lines and no end line are comments: the block goes in at the top.
    let marks = "# >>> Bolt preset: x\n".repeat(10_000);
    let after_marks = format!("{block}\n{marks}");
    for (before, after) in [(blocks, after_blocks), (marks, after_marks)] {
        assert!(before.len() < 1 << 20);
        let env = started();
        let path = env.root().join(".boltignore");
        fs::write(&path, &before).unwrap();
        let (code, said) = bounded(&env, &["setup", "--preset", "code", "--yes"]);
        assert_eq!(code, Some(0), "{said}");
        assert!(
            fs::read_to_string(&path).unwrap() == after,
            "not the first block alone"
        );
        let (_, shown) = json(&env, &["setup"]);
        assert_eq!(shown["ignore_block_lines"], block.lines().count() - 2);
        let (code, said) = bounded(&env, &["undo"]);
        assert_eq!(code, Some(0), "{said}");
        assert!(fs::read_to_string(&path).unwrap() == before, "undo");
    }
}

/// Every entry at the top of `dir` but `.bolt`, by name, with what a file holds (`None` for
/// anything else), read without following a link.
fn top_entries(dir: &Path) -> Vec<(String, Option<Vec<u8>>)> {
    let mut found: Vec<_> = fs::read_dir(dir)
        .unwrap()
        .map(Result::unwrap)
        .filter(|e| e.file_name() != ".bolt")
        .map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let file = fs::symlink_metadata(e.path()).unwrap().is_file();
            (name, file.then(|| fs::read(e.path()).unwrap()))
        })
        .collect();
    found.sort();
    found
}

#[test]
fn a_boltignore_that_is_a_link_stops_setup_and_start() {
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("ignore"), "outside/\n").unwrap();
    fs::create_dir(outside.path().join("folder")).unwrap();
    fs::write(outside.path().join("folder/x"), "x\n").unwrap();
    let seen = support::files(outside.path());
    // Makes `.boltignore` in `dir` a link out of it, to a file or a folder; false where this
    // account can't make that link.
    let link = |dir: &Path, to_a_file: bool| {
        let at = dir.join(".boltignore");
        if to_a_file {
            link_file(&outside.path().join("ignore"), &at)
        } else {
            link_folder(&outside.path().join("folder"), &at);
            true
        }
    };
    for (to_a_file, own_toml) in [(false, false), (false, true), (true, false), (true, true)] {
        // `bolt setup --preset`: refused before anything is written, bolt.toml included.
        let env = started();
        if own_toml {
            fs::write(env.root().join("bolt.toml"), "# mine\n").unwrap();
        }
        if !link(env.root(), to_a_file) {
            continue;
        }
        let before = top_entries(env.root());
        let entry = head(&env).0;
        for args in [
            &["setup", "--preset", "game", "--yes"][..],
            &["setup", "--preset", "code"],
        ] {
            let (code, out, err) = run(&env, args);
            assert_eq!(code, Some(1), "{args:?}: {err}");
            assert!(out.is_empty(), "{out}");
            assert!(err.contains(".boltignore isn't a regular file"), "{err}");
            assert_eq!(top_entries(env.root()), before);
            assert_eq!(head(&env).0, entry);
        }
        // `bolt start --preset`: with a bolt.toml it starts nothing; without one the project is
        // made, as terminal.md says, and neither rule file is written.
        let env = Env::empty();
        if own_toml {
            fs::write(env.root().join("bolt.toml"), "# mine\n").unwrap();
        }
        link(env.root(), to_a_file);
        let before = top_entries(env.root());
        let (code, _, err) = run(&env, &["start", "--preset", "code"]);
        assert_eq!(code, Some(1), "{err}");
        assert_eq!(top_entries(env.root()), before);
        if own_toml {
            assert!(!env.root().join(".bolt").exists());
        } else {
            assert!(err.contains("its rule files weren't written"), "{err}");
            assert!(err.contains(".boltignore isn't a regular file"), "{err}");
            assert_eq!(head(&env).1.command, "start");
        }
        assert_eq!(support::files(outside.path()), seen);
    }
}

// The look that suggests a preset.

/// Makes `dir` look like a project of every engine and language Bolt knows.
fn plant_every_engines_files(dir: &Path) {
    fs::create_dir_all(dir.join("Assets")).unwrap();
    fs::create_dir_all(dir.join("ProjectSettings")).unwrap();
    fs::write(dir.join("ProjectSettings/ProjectVersion.txt"), "m_Editor\n").unwrap();
    fs::write(dir.join("Game.uproject"), "{}\n").unwrap();
    fs::write(dir.join("project.godot"), "[application]\n").unwrap();
    for name in [
        "package.json",
        "pyproject.toml",
        "Cargo.toml",
        "go.mod",
        "pubspec.yaml",
        "App.sln",
        "pom.xml",
        "build.gradle",
    ] {
        fs::write(dir.join(name), "").unwrap();
    }
}

#[test]
fn planted_engine_files_only_change_the_suggestion() {
    let every = Found {
        engines: Engine::ALL.to_vec(),
        languages: Language::ALL.to_vec(),
    };
    let env = Env::empty();
    plant_every_engines_files(env.root());
    let (code, _, err) = run(&env, &["start"]);
    assert_eq!(code, Some(1));
    assert!(err.contains("suggest --preset game"), "{err}");
    let (code, doc) = json(&env, &["start", "--preset", "code"]);
    assert_eq!(code, Some(0), "{doc}");
    assert_eq!(
        doc["found"]["engines"],
        serde_json::json!(["unity", "unreal", "godot"])
    );
    assert_eq!(doc["found"]["languages"].as_array().unwrap().len(), 8);
    let read = |name: &str| fs::read_to_string(env.root().join(name)).unwrap();
    assert_eq!(
        read("bolt.toml"),
        presets::bolt_toml(Preset::Code, &every).unwrap()
    );
    let code_block = presets::boltignore_block(Preset::Code, &every).unwrap();
    assert_eq!(read(".boltignore"), code_block);
    // Code's lists only: nothing of the engines' found its way in.
    let game_lines = presets::boltignore_block(Preset::Game, &every).unwrap();
    let game_only = game_lines
        .lines()
        .filter(|l| !code_block.contains(l))
        .count();
    assert!(game_only > 50);
    assert!(!read(".boltignore").contains("/[Ll]ibrary/"));
}

/// Plants `n` empty files in `dir`, a thousand to a folder.
fn plant_names(dir: &Path, n: u64) {
    for i in 0..n {
        let folder = dir.join(format!("f{}", i / 1000));
        if i % 1000 == 0 {
            fs::create_dir_all(&folder).unwrap();
        }
        fs::write(folder.join(i.to_string()), b"").unwrap();
    }
}

#[test]
fn a_folder_of_a_million_names_is_looked_at_in_bounded_time() {
    // Two and a half times what a look meets before it stops, to be sure it stops; planting a
    // million names would take minutes on Windows and prove no more.
    let env = Env::empty();
    plant_names(env.root(), LOOK_AT_MOST * 5 / 2);
    plant_every_engines_files(env.root());
    let (code, said) = bounded(&env, &["start"]);
    assert_eq!(code, Some(1), "{said}");
    assert!(
        said.contains("found Unity, Unreal and Godot files and more than "),
        "{said}"
    );
    let (code, said) = bounded(&env, &["start", "--preset", "code"]);
    assert_eq!(code, Some(0), "{said}");
    let (code, said) = bounded(&env, &["setup", "--preset", "game", "--yes"]);
    assert_eq!(code, Some(0), "{said}");
}

#[test]
fn links_named_like_an_engines_folders_are_never_followed() {
    // A folder outside the project, of more names than a look meets, with Unity's version file.
    let outside = tempfile::tempdir().unwrap();
    let big = outside.path().join("big");
    plant_names(&big, LOOK_AT_MOST + 1_000);
    fs::create_dir(big.join("ProjectSettings")).unwrap();
    fs::write(big.join("ProjectSettings/ProjectVersion.txt"), "m_Editor\n").unwrap();
    let env = Env::empty();
    fs::write(env.root().join("own.txt"), "abc").unwrap();
    for name in ["Assets", "ProjectSettings", "Content", "Config"] {
        let target = if name == "ProjectSettings" {
            big.join("ProjectSettings")
        } else {
            big.clone()
        };
        link_folder(&target, &env.root().join(name));
    }
    // A link to a file named like Unreal's and Godot's, where this account can make one.
    link_file(&big.join("0"), &env.root().join("Game.uproject"));
    link_file(&big.join("1"), &env.root().join("project.godot"));
    let (code, said) = bounded(&env, &["start"]);
    assert_eq!(code, Some(1), "{said}");
    assert!(
        said.contains("found no game engine's or language's project files, and 3 B of files"),
        "{said}"
    );
    assert!(said.contains("suggest --preset custom"), "{said}");
    let (code, said) = bounded(&env, &["start", "--preset", "code"]);
    assert_eq!(code, Some(0), "{said}");
    let read = |name: &str| fs::read_to_string(env.root().join(name)).unwrap();
    let nothing = Found::default();
    assert_eq!(
        read("bolt.toml"),
        presets::bolt_toml(Preset::Code, &nothing).unwrap()
    );
    assert_eq!(
        read(".boltignore"),
        presets::boltignore_block(Preset::Code, &nothing).unwrap()
    );
    assert_eq!(fs::read_dir(&big).unwrap().count(), 22);
}

#[test]
fn engine_files_deeper_than_a_look_goes_change_nothing() {
    // Deeper than the 128 folders a look goes down, engine and language files are never seen.
    let env = Env::empty();
    let mut deep = env.root().to_path_buf();
    for _ in 0..200 {
        deep.push("d");
    }
    fs::create_dir_all(&deep).unwrap();
    plant_every_engines_files(&deep);
    let (code, said) = bounded(&env, &["start"]);
    assert_eq!(code, Some(1), "{said}");
    assert!(
        said.contains("found no game engine's or language's project files, and more than "),
        "{said}"
    );
    assert!(said.contains("suggest --preset custom"), "{said}");
}

// Rule files from Git.

#[test]
fn rule_files_inside_an_imported_repository_are_data_like_any_other() {
    let env = Env::new();
    // History holds a file that isn't TOML, then rules that keep a 3 MiB file whole.
    env.commit("bolt.toml", &"[".repeat(100_000), "Broken rules");
    env.commit("bolt.toml", "chunk-files-over = \"8 MiB\"\n", "Rules");
    let big = write_noise(&env.root().join("big.bin"), 7, 3 << 20);
    env.git(&["add", "big.bin"]);
    env.git(&["commit", "-q", "-m", "Big"]);
    // The folder's copy, changed and not committed, is the one that counts.
    fs::write(
        env.root().join("bolt.toml"),
        "chunk-files-over = \"2 MiB\"\n",
    )
    .unwrap();
    let (code, doc) = json(&env, &["import"]);
    assert_eq!(code, Some(0), "{doc}");
    assert!(rules_warnings(&doc).is_empty(), "{doc}");
    assert_eq!(copies_of(env.root(), &big), [ObjectKind::ChunkList]);
    // And a save reads it from the folder too.
    let other = write_noise(&env.root().join("other.bin"), 8, 3 << 20);
    let (code, doc) = json(&env, &["save", "Other"]);
    assert_eq!(code, Some(0), "{doc}");
    assert_eq!(copies_of(env.root(), &other), [ObjectKind::ChunkList]);
    let (_, shown) = json(&env, &["setup"]);
    assert_eq!(shown["chunk_files_over"], 2 << 20);
}
