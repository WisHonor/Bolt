//! Red-team session RT16 (decision 024), a probe for a `redteam/*` branch: which short names a
//! FAT drive gives `.opalro`, and whether `is_opal_alias` refuses each. On macOS it makes and
//! mounts a FAT32 image with `hdiutil`; elsewhere it uses a FAT folder named in `RT16_FAT_DIR`
//! (Linux: `mkfs.vfat` an image, `sudo mount -o loop,uid=$(id -u)` it). Without either it
//! returns. See docs/redteam/2026-11-12-v0.1-final.md, suspicion S1.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};
use std::process::Command;

use opal_core::names::is_opal_alias;

#[cfg(unix)]
fn ino(path: &Path) -> Option<(u64, u64)> {
    use std::os::unix::fs::MetadataExt;
    std::fs::symlink_metadata(path)
        .ok()
        .map(|m| (m.dev(), m.ino()))
}

#[cfg(not(unix))]
fn ino(_: &Path) -> Option<(u64, u64)> {
    None
}

/// Short names to try: the plain ones, Apple's (base cut where the tilde lands, `~~`), the
/// sequential tails up to four digits, NTFS's hashed one, and, with `random`, Linux's.
fn candidates(random: bool) -> Vec<String> {
    let mut out = Vec::new();
    for n in 1..=9 {
        out.extend([
            format!("OPALRO~{n}"),
            format!("OPALR~{n}"),
            format!("OPALR~~{n}"),
            format!("OP9975~{n}"),
        ]);
    }
    for n in 10..=9_999u32 {
        let digits = n.to_string();
        out.push(format!("{}~{digits}", &"OPALRO"[..7 - digits.len()]));
    }
    if random {
        for hex in 0..=0xFFFFu32 {
            for d in 1..=8 {
                out.push(format!("OP{hex:04X}~{d}"));
            }
        }
    }
    out
}

/// Makes `.opalro` in `dir` after `before` (names whose short base is `OPALRO` too), and gives
/// every candidate the drive takes for it.
fn short_names_of_opalro(dir: &Path, before: &[&str]) -> Vec<String> {
    std::fs::create_dir_all(dir).unwrap();
    for name in before {
        std::fs::create_dir(dir.join(name)).unwrap();
    }
    std::fs::create_dir(dir.join(".opalro")).unwrap();
    let own = ino(&dir.join(".opalro")).unwrap();
    candidates(before.len() >= 9)
        .into_iter()
        .filter(|c| ino(&dir.join(c)) == Some(own))
        .collect()
}

/// A FAT32 image made and mounted here, detached when dropped.
struct Mounted(PathBuf, Option<tempfile::TempDir>);

impl Drop for Mounted {
    fn drop(&mut self) {
        if self.1.is_some() {
            let _ = Command::new("hdiutil")
                .args(["detach", "-force"])
                .arg(&self.0)
                .status();
        }
    }
}

fn fat_folder() -> Option<Mounted> {
    if let Some(dir) = std::env::var_os("RT16_FAT_DIR") {
        return Some(Mounted(PathBuf::from(dir), None));
    }
    if !cfg!(target_os = "macos") {
        return None;
    }
    let work = tempfile::tempdir().unwrap();
    let image = work.path().join("fat.dmg");
    let mount = work.path().join("mnt");
    std::fs::create_dir(&mount).unwrap();
    let made = Command::new("hdiutil")
        .args([
            "create",
            "-size",
            "64m",
            "-fs",
            "MS-DOS FAT32",
            "-volname",
            "RT16",
        ])
        .arg(&image)
        .status()
        .unwrap();
    assert!(made.success(), "hdiutil create");
    let attached = Command::new("hdiutil")
        .args(["attach", "-nobrowse", "-mountpoint"])
        .arg(&mount)
        .arg(&image)
        .status()
        .unwrap();
    assert!(attached.success(), "hdiutil attach");
    Some(Mounted(mount, Some(work)))
}

#[test]
#[ignore = "RT16 probe S1, ruling 21: needs a FAT drive (macOS: made with hdiutil; elsewhere RT16_FAT_DIR)"]
fn every_short_name_a_fat_drive_gives_opalro_is_refused() {
    let Some(fat) = fat_folder() else {
        println!("no FAT drive here: set RT16_FAT_DIR, or run on macOS");
        return;
    };
    let base = fat.0.join(format!("rt16-{}", std::process::id()));
    // Each of these has the short base OPALRO and no extension, so each takes a tail first.
    let others = [
        "opa lro", "op alro", "o palro", "opal ro", "opalr o", "o p alro", "op a lro", "opa l ro",
        "o pa lro", "op al ro", "opa lr o", "o pal ro",
    ];
    let mut missed = Vec::new();
    for taken in [0, 1, 4, 5, 9, 12] {
        let dir = base.join(format!("after-{taken}"));
        let found = short_names_of_opalro(&dir, &others[..taken]);
        println!("after {taken} names with its base: .opalro answers to {found:?}");
        assert!(
            !found.is_empty() || taken > 0,
            "no short name found: not a FAT drive?"
        );
        missed.extend(found.into_iter().filter(|n| !is_opal_alias(n)));
    }
    let _ = std::fs::remove_dir_all(&base);
    assert!(
        missed.is_empty(),
        "short names of .opalro that a save may hold, so `goto` writes into .opalro: {missed:?}"
    );
}

/// Whether this FAT drive opens a long-named entry by any short form at all, and by which.
/// Always fails, so the log shows what it found.
#[test]
#[ignore = "RT16 probe S1, ruling 21: always fails, to print what a FAT drive opens"]
fn which_short_forms_open_a_long_named_entry() {
    let Some(fat) = fat_folder() else {
        println!("no FAT drive here");
        return;
    };
    let dir = fat.0.join(format!("forms-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("longfilename.txt"), b"x").unwrap();
    std::fs::create_dir(dir.join("averylongfoldername")).unwrap();
    std::fs::create_dir(dir.join(".opalro")).unwrap();
    let tries: [(&str, &[&str]); 3] = [
        (
            "longfilename.txt",
            &[
                "LONGFI~1.TXT",
                "LONGFIL~1.TXT",
                "LONGF~1.TXT",
                "LONGFILE.TXT",
                "longfi~1.txt",
            ],
        ),
        (
            "averylongfoldername",
            &["AVERYL~1", "AVERYLO~1", "AVERY~1", "AVERYLON", "averyl~1"],
        ),
        (
            ".opalro",
            &[
                "OPALRO", "OPALRO~1", "OPALR~1", "OPAL~1", "_OPALRO", "OPALRO~0", "opalro",
                "OPALRO.", "OPALR~~1",
            ],
        ),
    ];
    let mut report = Vec::new();
    for (long, forms) in tries {
        let own = ino(&dir.join(long));
        let opened: Vec<&str> = forms
            .iter()
            .copied()
            .filter(|f| ino(&dir.join(f)).is_some() && ino(&dir.join(f)) == own)
            .collect();
        report.push(format!("{long} (inode {own:?}) is opened by {opened:?}"));
    }
    let listed: Vec<String> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    let _ = std::fs::remove_dir_all(&dir);
    panic!("listed {listed:?}\n{}", report.join("\n"));
}
