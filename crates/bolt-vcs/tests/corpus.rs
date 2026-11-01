//! The malicious-name corpus, end to end: each name Bolt refuses (decision 004) must stop
//! `bolt save` with exit code 1 and the name in the message, and must leave history alone.
//! Names the operating system cannot create are skipped, so Linux and macOS run the most.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use assert_cmd::Command;
use std::path::Path;
use tempfile::TempDir;

fn bolt(dir: &Path, config: &Path) -> Command {
    let mut cmd = Command::cargo_bin("bolt").unwrap();
    cmd.current_dir(dir)
        .env("BOLT_USER_NAME", "Test")
        .env("BOLT_USER_EMAIL", "test@example.com")
        .env_remove("BOLT_FAULT_AT")
        .env("BOLT_CONFIG_DIR", config);
    cmd
}

/// Creates each file; true only if the folder really lists exactly these names.
fn create_all(dir: &Path, names: &[&str]) -> bool {
    for n in names {
        if std::fs::write(dir.join(n), "x").is_err() {
            return false;
        }
    }
    let mut listed: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| names.contains(&n.as_str()))
        .collect();
    listed.sort();
    let mut want: Vec<String> = names.iter().map(|n| (*n).to_owned()).collect();
    want.sort();
    want.dedup();
    listed == want
}

/// Each case is the files to create; the first name is the one the message must show.
const CASES: &[&[&str]] = &[
    // Refused outright (validate_name): Bolt's own folder, spelled sneakily.
    &[".bolt."],
    &[".bolt "],
    &["bolt~1"],
    &["BOLT~4"],
    &[".b\u{200C}olt"],
    &["\u{FEFF}.bolt"],
    &[".bolt:stream"],
    &[".BOLT.\\x"],
    &[".BOLT"],
    &[".Bolt"],
    &[".bolt. ."],
    &[".bolt\\x"],
    &["bolt~1\\x"],
    &[".bolt::$INDEX_ALLOCATION"],
    // Not portable (folder_problems).
    &["CON"],
    &["con.txt"],
    &["Aux"],
    &["COM1"],
    &["CONIN$"],
    &["CONOUT$"],
    &["COM\u{b9}"],
    &["COM0"],
    &["nul .txt"],
    &["a\u{1}b"],
    &["lpt9.log"],
    &["name."],
    &["name "],
    &["a:b"],
    &["a?b"],
    &["a*b"],
    &["a|b"],
    &["a<b"],
    &["a>b"],
    &["a\"b"],
    &["a\\b"],
    &["README", "readme"],
    &["Player.cs", "player.CS"],
    &["\u{e9}", "e\u{301}"],
];

fn history_of(root: &Path, config: &Path) -> Vec<u8> {
    let out = bolt(root, config).arg("history").output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&out.stdout).contains("good save"));
    out.stdout
}

#[test]
fn every_malicious_name_stops_the_save() {
    let mut ran = 0;
    for case in CASES {
        let work = TempDir::new().unwrap();
        let config = TempDir::new().unwrap();
        let root = work.path();
        bolt(root, config.path()).arg("start").assert().success();
        std::fs::write(root.join("good.txt"), "good").unwrap();
        bolt(root, config.path())
            .args(["save", "good save"])
            .assert()
            .success();
        if !create_all(root, case) {
            eprintln!("skipped (this system cannot hold these names): {case:?}");
            continue;
        }
        ran += 1;
        let history_before = history_of(root, config.path());

        let out = bolt(root, config.path())
            .args(["save", "bad"])
            .output()
            .unwrap();
        let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        assert_eq!(out.status.code(), Some(1), "{case:?}: {stderr}{stdout}");
        // Names with control or special characters may be shown escaped; compare on the
        // first printable run of the name, which every case has.
        let shown = case[0].escape_debug().to_string();
        let shown_hex: String = case[0]
            .chars()
            .map(|c| {
                if c < ' ' {
                    format!("\\x{:02x}", c as u32)
                } else {
                    c.to_string()
                }
            })
            .collect();
        assert!(
            stderr.contains(case[0]) || stderr.contains(&shown) || stderr.contains(&shown_hex),
            "{case:?}: {stderr}"
        );

        assert_eq!(history_of(root, config.path()), history_before, "{case:?}");
    }
    assert!(ran > 0, "no case could be created on this system");
}
