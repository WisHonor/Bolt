//! `bolt verify`: what it says when everything checks out, and when it doesn't.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Output;
use std::time::SystemTime;

use bolt_core::pack::{ENTRY_HEADER_LEN, PackReader, PackWriter};

mod support;
use support::{Env, text, write};

fn project() -> Env {
    let env = Env::empty();
    env.bolt()
        .args(["start", "--preset", "custom"])
        .assert()
        .success();
    write(&env, "a.txt", "a\n");
    env.bolt().args(["save", "First"]).assert().success();
    env
}

fn run(env: &Env, args: &[&str]) -> Output {
    env.bolt().args(args).output().unwrap()
}

/// Writes `name` into the project's packs: a pack holding `contents`, each with a byte flipped.
fn plant_damaged(env: &Env, name: &str, contents: &[Vec<u8>]) {
    let made = tempfile::tempdir().unwrap();
    let mut w = PackWriter::create(made.path()).unwrap();
    for c in contents {
        w.add_content(c).unwrap();
    }
    let path = w.finish(made.path()).unwrap().unwrap();
    let mut pack = fs::read(&path).unwrap();
    for e in PackReader::open(&path).unwrap().entries() {
        let e = e.unwrap();
        pack[usize::try_from(e.offset + ENTRY_HEADER_LEN).unwrap()] ^= 1;
    }
    fs::write(env.root().join(".bolt/store/packs").join(name), pack).unwrap();
}

fn doc(out: &Output) -> serde_json::Value {
    serde_json::from_slice(&out.stdout).unwrap()
}

#[test]
fn verify_says_verified_and_exits_0() {
    let env = project();
    let out = run(&env, &["verify"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    assert_eq!(text(&out.stderr), "");
    let shown = text(&out.stdout);
    let name = env.root().file_name().unwrap().to_string_lossy();
    let lines: Vec<&str> = shown.lines().collect();
    assert_eq!(lines[0], format!("Verified {name}: no problems found."));
    assert_eq!(lines[1], "");
    assert!(lines[2].starts_with("    + "), "{shown}");
    assert!(lines[2].ends_with("match their fingerprints"), "{shown}");
    assert_eq!(
        lines[3],
        "    + 2 saves, 1 line and 0 shelves point to objects that exist"
    );
    assert_eq!(lines[4], "    + the undo diary (2 entries) is intact");
    assert_eq!(lines.len(), 5, "{shown}");
}

#[test]
fn verify_lists_problems_and_exits_1() {
    let env = project();
    let contents: Vec<Vec<u8>> = (0..25).map(|i| format!("copy {i}").into_bytes()).collect();
    plant_damaged(&env, "00.pack", &contents);
    let out = run(&env, &["verify"]);
    assert_eq!(out.status.code(), Some(1), "{}", text(&out.stderr));
    let shown = text(&out.stdout);
    let name = env.root().file_name().unwrap().to_string_lossy();
    let lines: Vec<&str> = shown.lines().collect();
    assert_eq!(lines[0], format!("Checked {name}: 25 problems found."));
    let cross = lines
        .iter()
        .position(|l| *l == "    x 25 objects are damaged:");
    let cross = cross.unwrap_or_else(|| panic!("{shown}"));
    let rows: Vec<&&str> = lines[cross + 1..]
        .iter()
        .take_while(|l| l.starts_with("        "))
        .collect();
    assert_eq!(rows.len(), 21, "{shown}");
    for row in &rows[..20] {
        assert!(
            row.ends_with("  It doesn't match its fingerprint."),
            "{row}"
        );
    }
    assert_eq!(
        *rows[20],
        "        ... and 5 more (bolt verify --json lists up to 1,000)"
    );
    // The objects that matched, and the checks that found nothing, still say so.
    assert!(lines[2].starts_with("    + "), "{shown}");
    assert!(shown.contains("    + the undo diary (2 entries) is intact"));
    let said = text(&out.stderr);
    assert!(
        said.starts_with("hint: Bolt can't repair a project in this version."),
        "{said}"
    );
    assert!(!said.contains("error:"), "{said}");
}

#[test]
fn verify_json_matches_json_md() {
    let env = project();
    let out = run(&env, &["--json", "verify"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    let clean = doc(&out);
    let keys: Vec<&String> = clean.as_object().unwrap().keys().collect();
    let fields = [
        "bolt_json",
        "bytes",
        "entries",
        "objects",
        "packs",
        "problems",
        "problems_total",
        "saves",
        "warnings",
    ];
    assert_eq!(keys, fields);
    assert_eq!(clean["problems_total"], 0);

    plant_damaged(&env, "00.pack", &[b"damaged".to_vec()]);
    let out = run(&env, &["--json", "verify"]);
    assert_eq!(out.status.code(), Some(1), "{}", text(&out.stderr));
    assert_eq!(text(&out.stderr), "");
    let mut found = doc(&out);
    let name = env.root().file_name().unwrap().to_string_lossy();
    assert_eq!(
        found["error"]["message"],
        format!("Checked {name}: 1 problem found.")
    );
    found["error"]["message"] = "[redacted]".into();
    found["bytes"] = "[redacted]".into();
    insta::assert_json_snapshot!(found);
}

#[test]
fn verify_names_a_pack_with_an_escaped_name() {
    let env = project();
    let packs = env.root().join(".bolt/store/packs");
    fs::write(packs.join("a\u{202e}b.pack"), b"not a pack").unwrap();
    let out = run(&env, &["verify"]);
    assert_eq!(out.status.code(), Some(1), "{}", text(&out.stderr));
    let shown = text(&out.stdout);
    assert!(!shown.contains('\u{202e}'), "{shown}");
    assert!(
        shown.contains("    x 1 pack is damaged:\n        a\\u{202e}b.pack  The pack is damaged: "),
        "{shown}"
    );
    let out = run(&env, &["--json", "verify"]);
    let found = doc(&out);
    // The name as the project has it, for programs; the message is safe to print.
    assert_eq!(found["problems"][0]["pack"], "a\u{202e}b.pack");
    assert_eq!(found["problems"][0]["kind"], "pack_damaged");
    let message = found["problems"][0]["message"].as_str().unwrap();
    assert!(!message.contains('\u{202e}'), "{message}");
}

#[test]
fn fsck_verifies() {
    let env = project();
    let typed = run(&env, &["fsck", "--json"]);
    let meant = run(&env, &["verify", "--json"]);
    assert_eq!(typed.status.code(), Some(0), "{}", text(&typed.stderr));
    assert_eq!(doc(&typed), doc(&meant));
}

/// On a terminal the checks get their marks and colours.
#[cfg(feature = "fault-injection")]
#[test]
fn on_a_terminal_the_checks_have_their_marks() {
    let env = project();
    let out = env
        .bolt()
        .env("BOLT_FAKE_TERMINAL", "80")
        .env("BOLT_PROGRESS_QUIET_MS", "600000")
        .env("TERM", "xterm-256color")
        .env("BOLT_ASCII", "0")
        .env_remove("NO_COLOR")
        .arg("verify")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    let shown = text(&out.stdout);
    assert!(shown.starts_with("\x1b[1;32mVerified\x1b[0m "), "{shown:?}");
    assert!(shown.contains("    \x1b[1;32m✓\x1b[0m "), "{shown:?}");
}

#[test]
fn every_problem_of_a_kind_is_counted_on_screen() {
    let env = project();
    let contents: Vec<Vec<u8>> = (0..1005)
        .map(|i| format!("copy {i}").into_bytes())
        .collect();
    plant_damaged(&env, "00.pack", &contents);
    let out = run(&env, &["verify"]);
    assert_eq!(out.status.code(), Some(1), "{}", text(&out.stderr));
    let shown = text(&out.stdout);
    assert!(
        shown.contains("    x 1,005 objects are damaged:\n"),
        "{shown}"
    );
    assert!(
        shown.contains("        ... and 985 more (bolt verify --json lists up to 1,000)\n"),
        "{shown}"
    );
}

/// Every file and folder under `dir`: a file's bytes, and when each was last changed.
fn picture(dir: &Path) -> BTreeMap<PathBuf, (Option<Vec<u8>>, SystemTime)> {
    let mut found = BTreeMap::new();
    let mut folders = vec![dir.to_owned()];
    while let Some(folder) = folders.pop() {
        let changed = fs::symlink_metadata(&folder).unwrap().modified().unwrap();
        found.insert(folder.clone(), (None, changed));
        for e in fs::read_dir(&folder).unwrap() {
            let path = e.unwrap().path();
            let meta = fs::symlink_metadata(&path).unwrap();
            if meta.is_dir() {
                folders.push(path);
            } else {
                let bytes = fs::read(&path).unwrap();
                found.insert(path, (Some(bytes), meta.modified().unwrap()));
            }
        }
    }
    found
}

#[test]
fn verify_leaves_every_file_as_it_was_even_bolts_leftovers() {
    for damaged in [false, true] {
        let env = project();
        if damaged {
            plant_damaged(&env, "00.pack", &[b"damaged".to_vec()]);
            let mut content = None;
            let gone = support::add_pack(env.root(), |p| {
                content = Some(p.add_content(b"only here").unwrap());
            });
            let content = content.unwrap();
            support::plant_save(env.root(), |p| {
                support::top_listing(p, &[("a.txt", content, 9)])
            });
            fs::remove_file(&gone[0]).unwrap();
        }
        let leftover = env.root().join(".bolt/tmp/tmp-0123456789abcdef");
        fs::write(&leftover, "left by a crash").unwrap();
        let bolt = env.root().join(".bolt");
        let before = picture(&bolt);
        let out = run(&env, &["verify"]);
        let code = if damaged { 1 } else { 0 };
        assert_eq!(out.status.code(), Some(code), "{}", text(&out.stdout));
        if damaged {
            let shown = text(&out.stdout);
            assert!(shown.contains("1 object is damaged:"), "{shown}");
            assert!(
                shown.contains("1 object the project needs is missing:"),
                "{shown}"
            );
        }
        assert!(leftover.exists());
        assert_eq!(picture(&bolt), before, "damaged: {damaged}");
    }
}
