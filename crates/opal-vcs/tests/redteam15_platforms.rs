//! Red-team session RT15 (decision 024): the rename's look-alikes that only a Windows or macOS
//! drive can show, asserting the safe behaviour. See docs/redteam/2026-11-12-the-name.md.
#![cfg(any(windows, target_os = "macos"))]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use std::fs;
use std::path::Path;

#[cfg(windows)]
use opal_core::listing::EntryKind;
use opal_core::names::{is_opal_alias, validate_name};
use support::Env;
#[cfg(windows)]
use support::{listings, main_tip, plant_save};

/// The short names `dir /x` shows for the entries of `dir`, by long name; none off Windows.
fn short_names(dir: &Path) -> Vec<(String, String)> {
    if !cfg!(windows) {
        return Vec::new();
    }
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

fn short_of(shorts: &[(String, String)], long: &str) -> Option<String> {
    shorts
        .iter()
        .find(|(l, _)| l == long)
        .map(|(_, s)| s.clone())
}

/// Every name made from `.opalro`, or from the short name the drive gave it, by putting one
/// character from U+0080 to U+FFFF in place of one of its characters or between two of them,
/// that the drive opens as the real folder: each must be one Opalro refuses. Twice: with
/// `.opalro` made first (`OPALRO~1` on NTFS), and after `.opalroignore`, which shares its six
/// letters and so takes `OPALRO~1` first.
#[test]
#[ignore = "RT15: an exploration of this drive's names, slow; run on a redteam/* branch"]
fn every_one_character_change_of_opalro_or_its_short_name_is_refused() {
    let mut missed = Vec::new();
    let mut opened = 0;
    for ignore_first in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        if ignore_first {
            fs::write(dir.path().join(".opalroignore"), b"*.tmp\n").unwrap();
        }
        fs::create_dir(dir.path().join(".opalro")).unwrap();
        fs::write(dir.path().join(".opalro").join("marker"), b"x").unwrap();
        let shorts = short_names(dir.path());
        let short = short_of(&shorts, ".opalro");
        println!(
            ".opalroignore first: {ignore_first}; short names: .opalro {short:?}, \
             .opalroignore {:?}",
            short_of(&shorts, ".opalroignore")
        );
        if let Some(short) = &short {
            assert!(
                is_opal_alias(short),
                "the short name {short} is let through"
            );
        }
        for real in [Some(".opalro".to_owned()), short].into_iter().flatten() {
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
                            if validate_name(&name).is_ok() {
                                missed.push(format!("{name:?} opens .opalro (U+{cp:04X})"));
                            }
                        }
                    }
                }
            }
        }
    }
    println!("names that open .opalro: {opened}; missed: {missed:#?}");
    assert!(missed.is_empty(), "{missed:#?}");
}

/// `.opalroignore` and three other names whose short names start `OPALRO` take `OPALRO~1` to
/// `OPALRO~4`, so `.opalro` gets a hashed short name no fixed list knows, and so does an
/// `.opalroignore` made after it. A save naming both short names must write into neither.
#[cfg(windows)]
#[test]
#[ignore = "RT15: run on a redteam/* branch"]
fn a_save_naming_the_hashed_short_names_of_opalro_and_its_ignore_file_writes_into_neither() {
    let env = Env::empty();
    let root = env.root();
    fs::write(root.join(".opalroignore"), b"# first\n").unwrap();
    for taken in ["o palro", "op alro", "opa lro"] {
        fs::create_dir(root.join(taken)).unwrap();
        fs::write(root.join(taken).join("f"), b"x").unwrap();
    }
    env.opalro()
        .args(["start", "--preset", "custom"])
        .assert()
        .success();
    fs::create_dir(root.join("sub")).unwrap();
    for taken in ["o palro", "op alro", "opa lro", "opal ro"] {
        fs::create_dir(root.join("sub").join(taken)).unwrap();
    }
    fs::write(root.join("sub").join(".opalroignore"), b"# sub\n").unwrap();
    env.opalro().args(["save", "both"]).assert().success();
    let shorts = short_names(root);
    let (opal_short, ignore_short) = (
        short_of(&shorts, ".opalro"),
        short_of(&shorts, ".opalroignore"),
    );
    let sub_ignore_short = short_of(&short_names(&root.join("sub")), ".opalroignore");
    println!(
        "short names: .opalro {opal_short:?}, .opalroignore {ignore_short:?}, \
         sub/.opalroignore {sub_ignore_short:?}"
    );
    let (Some(opal_short), Some(sub_ignore_short)) = (opal_short, sub_ignore_short) else {
        panic!("this drive makes no short names, so nothing was attacked: {shorts:?}");
    };
    let first = main_tip(&env).to_hex();
    let opal_path = format!("{opal_short}/planted");
    let sub_path = format!("sub/{sub_ignore_short}");
    plant_save(root, |pack| {
        listings(
            pack,
            &[
                (opal_path.as_str(), EntryKind::File, &b"PLANTED"[..]),
                (".opalroignore", EntryKind::File, &b"# first\n"[..]),
                ("sub/.opalroignore", EntryKind::File, &b"# sub\n"[..]),
                (sub_path.as_str(), EntryKind::File, &b"PLANTED"[..]),
                ("o palro/f", EntryKind::File, &b"x"[..]),
            ],
        )
    });
    let out = env.opalro().args(["goto", "main"]).output().unwrap();
    println!(
        "goto: {:?}\n{}\n{}\nback to {first}",
        out.status.code(),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(!root.join(".opalro").join("planted").exists());
    assert_eq!(
        fs::read(root.join("sub").join(".opalroignore")).unwrap(),
        b"# sub\n"
    );
}

/// On a drive that ignores letter case, another tool's `.OPALRO` is the `.opalro` Opalro would use:
/// `opalro start` and `opalro import` must refuse it, as they refuse `.opalro` itself.
#[test]
#[ignore = "RT15: run on a redteam/* branch"]
fn start_and_import_refuse_another_tools_folder_in_another_letter_case() {
    for command in ["start", "import"] {
        let env = if command == "import" {
            let env = Env::new();
            env.commit("a.txt", "a\n", "First");
            env
        } else {
            let env = Env::empty();
            fs::write(env.root().join("a.txt"), b"a\n").unwrap();
            env
        };
        let theirs = env.root().join(".OPALRO");
        fs::create_dir(&theirs).unwrap();
        fs::write(theirs.join("their-data"), b"THEIRS").unwrap();
        let mut args = vec![command];
        if command == "start" {
            args.extend(["--preset", "custom"]);
        }
        let out = env.opalro().args(&args).output().unwrap();
        println!(
            "{command}: {:?}\n{}\n{}",
            out.status.code(),
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        let names: Vec<String> = fs::read_dir(&theirs)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert!(
            !out.status.success(),
            "{command} settled into .OPALRO: {names:?}"
        );
        assert_eq!(names, ["their-data"], "{command}");
    }
}
