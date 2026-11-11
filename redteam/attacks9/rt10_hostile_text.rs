//! Exploration: a project's text with every kind of control in every place milestone 6 prints it,
//! at many widths, colour on and off, and through `--json`. Prints each raw control it finds.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use std::fs;

use support::{Env, text};

const HOSTILE: &str = "a\u{1b}[2Jb\r\u{202e}c\u{1}d\u{2}e\u{3}f\u{7f}g\u{9b}31mh\u{2028}i";

/// What in `said` isn't Bolt's: a control that isn't a line feed or one of Bolt's colour codes.
fn raw(said: &str) -> Vec<String> {
    let mut found = Vec::new();
    let chars: Vec<char> = said.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '\u{1b}' {
            let rest: String = chars[i..chars.len().min(i + 8)].iter().collect();
            let ok = ["\u{1b}[0m", "\u{1b}[1m", "\u{1b}[2m", "\u{1b}[2K"]
                .iter()
                .any(|p| rest.starts_with(p))
                || rest.len() > 4
                    && rest.starts_with("\u{1b}[")
                    && rest[2..].chars().take_while(|c| *c != 'm').all(|c| c.is_ascii_digit() || c == ';');
            if !ok {
                found.push(format!("ESC at {i}: {rest:?}"));
            }
        } else if (c.is_control() && c != '\n') || matches!(c, '\u{202e}' | '\u{2028}') {
            let around: String = chars[i.saturating_sub(10)..chars.len().min(i + 10)].iter().collect();
            found.push(format!("{c:?} in {around:?}"));
        }
        i += 1;
    }
    found
}

fn run(env: &Env, width: Option<usize>, color: bool, args: &[&str]) -> (String, String) {
    let mut cmd = env.bolt();
    cmd.env("TERM", "xterm-256color")
        .env("LC_ALL", "en_US.UTF-8")
        .env("BOLT_PROGRESS_QUIET_MS", "600000")
        .env_remove("NO_COLOR")
        .env_remove("BOLT_ASCII");
    if let Some(w) = width {
        cmd.env("BOLT_FAKE_TERMINAL", w.to_string());
    }
    if !color {
        cmd.env("NO_COLOR", "1");
    }
    let out = cmd.args(args).output().unwrap();
    (text(&out.stdout), text(&out.stderr))
}

#[test]
fn hostile_text_everywhere() {
    let env = Env::empty();
    env.bolt()
        .args(["start", "--preset", "custom"])
        .assert()
        .success();
    let toml_text = HOSTILE
        .replace('\u{1b}', "\\u001b")
        .replace('\r', "\\r")
        .replace('\u{202e}', "\\u202e")
        .replace('\u{1}', "\\u0001")
        .replace('\u{2}', "\\u0002")
        .replace('\u{3}', "\\u0003")
        .replace('\u{7f}', "\\u007f")
        .replace('\u{9b}', "\\u009b")
        .replace('\u{2028}', "\\u2028");
    let short: String = toml_text.chars().take(60).collect();
    fs::write(
        env.root().join("bolt.toml"),
        format!(
            "preset = \"{short}\"\n\"{toml_text}\" = 1\nchunk-files-over = \"{toml_text}\"\n\
             [[rule]]\nfiles = [\"{toml_text}\"]\nlockable = true\nconflict-markers = false\n\
             [[rule]]\nfiles = [\"x\"]\n\"{toml_text}\" = 2\n"
        ),
    )
    .unwrap();
    fs::write(env.root().join(".boltignore"), format!("{HOSTILE}\n")).unwrap();
    fs::write(env.root().join("plain.txt"), "x").unwrap();
    env.bolt().args(["save", "--yes", HOSTILE]).assert().success();
    fs::write(env.root().join("plain.txt"), "changed").unwrap();
    env.bolt().args(["shelf", HOSTILE]).assert().success();
    let file = bolt_core::listing::EntryKind::File;
    support::plant_save(env.root(), |p| {
        support::listings(
            p,
            &[
                (HOSTILE, file, b"planted"),
                (&format!("d{HOSTILE}/{HOSTILE}"), file, b"deep"),
            ],
        )
    });
    fs::write(env.root().join(format!("new{HOSTILE}")), "x").unwrap();
    fs::write(
        env.root().join(format!(".bolt/store/packs/{HOSTILE}.pack")),
        "not a pack",
    )
    .unwrap();
    let commands: &[&[&str]] = &[
        &["setup"],
        &["setup", "--preset", "code"],
        &["setup", "--preset", "custom"],
        &["status"],
        &["history"],
        &["show"],
        &["shelf", "--list"],
        &["undo", "--list", "--all"],
        &["verify"],
        &["tidy"],
        &["changes", "--save", "main"],
        &["save", "--yes", "again"],
    ];
    let mut found = 0;
    for args in commands {
        for width in [None, Some(1), Some(2), Some(7), Some(20), Some(41), Some(80), Some(500)] {
            for color in [false, true] {
                let (out, err) = run(&env, width, color, args);
                for (stream, said) in [("stdout", &out), ("stderr", &err)] {
                    for r in raw(said) {
                        found += 1;
                        if found < 60 {
                            eprintln!("{args:?} width {width:?} colour {color} {stream}: {r}");
                        }
                    }
                }
            }
        }
        let mut json = vec!["--json"];
        json.extend_from_slice(args);
        let (out, err) = run(&env, None, false, &json);
        let doc: Result<serde_json::Value, _> = serde_json::from_str(&out);
        match doc {
            Err(e) => eprintln!("{json:?}: not JSON ({e}): {:?}", &out[..out.len().min(200)]),
            Ok(doc) => {
                let text = doc.to_string();
                for r in raw(&text) {
                    eprintln!("{json:?} json: {r}");
                }
                for w in doc["warnings"].as_array().into_iter().flatten() {
                    for r in raw(w["message"].as_str().unwrap_or("")) {
                        eprintln!("{json:?} warning message: {r}");
                    }
                }
            }
        }
        for r in raw(&err) {
            eprintln!("{json:?} json stderr: {r}");
        }
    }
    eprintln!("raw controls found: {found}");
}
