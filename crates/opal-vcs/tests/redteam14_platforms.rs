//! Red-team session RT14 (decision 024): look-alikes of `.git` and `.opalro` that only a Windows or
//! macOS drive can show, asserting the safe behaviour. See docs/redteam/2026-11-11-v0.1-release.md.
#![cfg(any(windows, target_os = "macos"))]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use std::fs;
#[cfg(windows)]
use std::path::Path;

#[cfg(windows)]
use opal_core::listing::EntryKind;
use opal_core::names::{is_git_alias, is_opal_alias};
#[cfg(windows)]
use support::{Env, listings, main_tip, plant_save};

/// Every name made from `.git` or `.opalro` by putting one character from U+0080 to U+FFFF in
/// place of one of its characters, or in between two of them, that this drive opens as the real
/// folder: each must be one Opalro's look-alike tests catch.
#[test]
#[ignore = "RT14: an exploration of this drive's names, slow; run on a redteam/* branch"]
fn every_one_character_change_this_drive_opens_as_git_or_opal_is_caught() {
    let dir = tempfile::tempdir().unwrap();
    let mut missed = Vec::new();
    let mut opened = 0;
    for real in [".git", ".opalro"] {
        fs::create_dir(dir.path().join(real)).unwrap();
        fs::write(dir.path().join(real).join("marker"), b"x").unwrap();
        let chars: Vec<char> = real.chars().collect();
        for cp in 0x80u32..=0xFFFF {
            let Some(c) = char::from_u32(cp) else {
                continue;
            };
            for at in 0..=chars.len() {
                let mut variants = Vec::new();
                let mut put = chars.clone();
                put.insert(at, c);
                variants.push(put.into_iter().collect::<String>());
                if at < chars.len() {
                    let mut swap = chars.clone();
                    swap[at] = c;
                    variants.push(swap.into_iter().collect::<String>());
                }
                for name in variants {
                    if dir.path().join(&name).join("marker").exists() {
                        opened += 1;
                        if !is_git_alias(&name) && !is_opal_alias(&name) {
                            missed.push(format!("{name:?} opens {real} (U+{cp:04X})"));
                        }
                    }
                }
            }
        }
        fs::remove_dir_all(dir.path().join(real)).unwrap();
    }
    println!("names that open .git or .opalro: {opened}; missed: {missed:#?}");
    assert!(missed.is_empty(), "{missed:#?}");
}

/// The short names `dir /x` shows for the entries of `dir`, by long name.
#[cfg(windows)]
fn short_names(dir: &Path) -> Vec<(String, String)> {
    let out = std::process::Command::new("cmd")
        .args(["/c", "dir", "/x", "/a"])
        .current_dir(dir)
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    println!("{text}");
    let mut found = Vec::new();
    for line in text.lines() {
        // date time <DIR>|size short long
        let cols: Vec<&str> = line.split_whitespace().collect();
        if let Some(at) = cols
            .iter()
            .skip(3)
            .position(|c| c.contains('~'))
            .map(|at| at + 3)
        {
            found.push((cols[at + 1..].join(" "), cols[at].to_owned()));
        }
    }
    found
}

/// A project whose `.git` and `.opalro` got short names other than `GIT~1` and `OPALRO~1`, because
/// folders whose short names take those came first. A save holding folders named by those short
/// names must never write into `.git` or `.opalro`.
#[cfg(windows)]
#[test]
#[ignore = "RT14: run on a redteam/* branch"]
fn a_save_naming_the_real_short_names_of_git_and_opal_writes_into_neither() {
    let env = Env::empty();
    let root = env.root();
    for taken in ["g it", "o palro", "op alro", "opa lro", "opal ro"] {
        fs::create_dir(root.join(taken)).unwrap();
        fs::write(root.join(taken).join("f"), b"x").unwrap();
    }
    env.opalro()
        .args(["start", "--preset", "custom"])
        .assert()
        .success();
    fs::create_dir(root.join(".git")).unwrap();
    fs::write(root.join(".git").join("config"), b"ORIGINAL").unwrap();
    let shorts = short_names(root);
    let short_of = |long: &str| {
        shorts
            .iter()
            .find(|(l, _)| l == long)
            .map(|(_, s)| s.clone())
    };
    let (git_short, opal_short) = (short_of(".git"), short_of(".opalro"));
    println!("short names: .git {git_short:?}, .opalro {opal_short:?}");
    let (Some(git_short), Some(opal_short)) = (git_short, opal_short) else {
        panic!("this drive makes no short names, so nothing was attacked: {shorts:?}");
    };
    let first = main_tip(&env).to_hex();
    env.opalro().args(["goto", &first]).assert().success();
    let git_path = format!("{git_short}/config");
    let opal_path = format!("{opal_short}/planted");
    plant_save(root, |pack| {
        listings(
            pack,
            &[
                (git_path.as_str(), EntryKind::File, &b"PLANTED"[..]),
                (opal_path.as_str(), EntryKind::File, &b"PLANTED"[..]),
                ("g it/f", EntryKind::File, &b"x"[..]),
            ],
        )
    });
    let out = env.opalro().args(["goto", "main"]).output().unwrap();
    println!(
        "goto: {:?}\n{}\n{}",
        out.status.code(),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        fs::read(root.join(".git").join("config")).unwrap(),
        b"ORIGINAL"
    );
    assert!(!root.join(".opalro").join("planted").exists());
}
