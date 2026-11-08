//! Red-team proof tests for milestone 4, second session (decision 024): each asserts the safe
//! behaviour and fails on the code the session attacked. See
//! docs/redteam/2026-11-08-milestone-4-session-2.md.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use std::fs;

use support::{Env, text};

fn project() -> Env {
    let env = Env::empty();
    env.bolt().arg("start").assert().success();
    env
}

fn ok(env: &Env, args: &[&str]) -> String {
    let out = env.bolt().args(args).output().unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "{args:?}: {}",
        text(&out.stderr)
    );
    text(&out.stdout)
}

/// Whether `bytes` are anywhere Bolt keeps content: the folder's file `name` or the store.
#[cfg(target_os = "linux")]
fn kept(env: &Env, name: &str, bytes: &[u8]) -> bool {
    if fs::read(env.root().join(name)).is_ok_and(|b| b == bytes) {
        return true;
    }
    let repo = bolt_core::repo::Repository::open(env.root()).unwrap();
    let mut out = Vec::new();
    repo.store()
        .copy_content(&bolt_core::id::hash_content(bytes), &mut out)
        .is_ok()
}

#[cfg(target_os = "linux")]
fn big(seed: u8) -> Vec<u8> {
    let mut x = u64::from(seed) | 0x9e37_79b9_7f4a_7c00;
    (0..64usize << 20)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x.to_le_bytes()[2]
        })
        .collect()
}

/// How far the process `pid` has read into its `nth` opening of `file`, each time it is seen,
/// until `act` says to stop. Gives whether it stopped.
#[cfg(target_os = "linux")]
fn watch_reads(
    pid: u32,
    file: &std::path::Path,
    nth: usize,
    mut act: impl FnMut(u64) -> bool,
) -> bool {
    use std::time::{Duration, Instant};
    let (mut was_open, mut opened) = (false, 0usize);
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(60) {
        let Ok(fds) = fs::read_dir(format!("/proc/{pid}/fd")) else {
            return false;
        };
        let pos = fds.flatten().find_map(|fd| {
            fs::read_link(fd.path()).ok().filter(|t| t == file)?;
            let info = fs::read_to_string(format!(
                "/proc/{pid}/fdinfo/{}",
                fd.file_name().to_string_lossy()
            ))
            .ok()?;
            info.lines()
                .find_map(|l| l.strip_prefix("pos:"))
                .and_then(|p| p.trim().parse().ok())
        });
        if pos.is_some() && !was_open {
            opened += 1;
        }
        was_open = pos.is_some();
        if let Some(pos) = pos
            && opened == nth
            && act(pos)
        {
            return true;
        }
        std::thread::sleep(Duration::from_micros(100));
    }
    false
}

#[cfg(target_os = "linux")]
#[test]
fn rt7_1_a_file_edited_while_undo_checks_it_is_never_removed() {
    use std::io::Write;
    let env = project();
    let file = env.root().join("data.db");
    fs::write(&file, big(1)).unwrap();
    ok(&env, &["save", "With the database"]);
    let with: serde_json::Value = serde_json::from_str(&ok(&env, &["--json", "status"])).unwrap();
    let with = with["save"]["id"].as_str().unwrap().to_owned();
    fs::remove_file(&file).unwrap();
    ok(&env, &["save", "Without"]);
    ok(&env, &["goto", &with]);
    // Undoing the goto removes data.db. The plan reads it once; the switch reads it again to
    // check it before removing it. Once that read is past the start, a program writes the first
    // bytes in place: the part already read, so the check still finds the saved contents.
    let mut theirs = big(1);
    theirs[..16].copy_from_slice(b"edited in place!");
    let mut cmd = env.bolt_process_in(env.root());
    cmd.arg("undo")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let child = cmd.spawn().unwrap();
    let (pid, path) = (child.id(), file.canonicalize().unwrap());
    let watcher = std::thread::spawn(move || {
        watch_reads(pid, &path, 2, |pos| {
            if pos < 1 << 20 {
                return false;
            }
            let mut f = fs::OpenOptions::new().write(true).open(&path).unwrap();
            f.write_all(b"edited in place!").unwrap();
            true
        })
    });
    let out = child.wait_with_output().unwrap();
    let raced = watcher.join().unwrap();
    assert!(raced, "the undo never read data.db a second time");
    let survived = kept(&env, "data.db", &theirs);
    assert!(
        survived,
        "undo: {:?}\n{}{}\nthe edit made during the undo survived: {survived}",
        out.status.code(),
        text(&out.stdout),
        text(&out.stderr)
    );
}

fn now_ms() -> i64 {
    let since = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap();
    i64::try_from(since.as_millis()).unwrap()
}

#[test]
#[ignore = "RT7-2: raising undo-window-days after a tidy stops tidy, calling the store damaged"]
fn rt7_2_raising_the_undo_window_after_a_tidy_leaves_tidy_working() {
    let env = project();
    fs::write(env.root().join("a.txt"), "one\n").unwrap();
    ok(&env, &["save", "One"]);
    fs::write(env.root().join("a.txt"), "work thrown away\n").unwrap();
    ok(&env, &["discard"]);
    // Three days on, `tidy` runs with `undo-window-days = 1`: the discarded work goes, rightly.
    let mut repo = bolt_core::repo::Repository::open(env.root()).unwrap();
    let later = now_ms() + 3 * 24 * 3600 * 1000;
    let first = bolt_core::tidy::tidy(&mut repo, 1, later, &mut |_| {}, &mut |_| {});
    assert!(first.is_ok_and(|done| done.packs_removed > 0));
    drop(repo);
    // The user then sets the window back to the default and tidies again.
    let tidy = env.bolt().arg("tidy").output().unwrap();
    let list = env.bolt().args(["undo", "--list"]).output().unwrap();
    assert!(
        tidy.status.code() == Some(0),
        "tidy: {:?}\n{}{}\nundo --list:\n{}",
        tidy.status.code(),
        text(&tidy.stdout),
        text(&tidy.stderr),
        text(&list.stdout)
    );
}
