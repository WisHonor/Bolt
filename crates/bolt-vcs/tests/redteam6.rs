//! Red-team proof tests for milestone 4 (decision 024): each asserts the safe behaviour and fails
//! on the code the session attacked. See docs/redteam/2026-11-08-milestone-4.md.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use std::fs;
use std::time::{Duration, Instant, SystemTime};

use support::{Env, text};

fn project() -> Env {
    let env = Env::empty();
    env.bolt()
        .args(["start", "--preset", "custom"])
        .assert()
        .success();
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

/// Makes every pack of the project `hours` old, so that `tidy` may change it.
fn age_packs(env: &Env, hours: u64) {
    let packs = env.root().join(".bolt/store/packs");
    for entry in fs::read_dir(packs).unwrap() {
        let file = fs::File::options()
            .write(true)
            .open(entry.unwrap().path())
            .unwrap();
        file.set_modified(SystemTime::now() - Duration::from_secs(hours * 3600))
            .unwrap();
    }
}

#[test]
fn rt6_1_tidy_never_deletes_saves_made_after_an_older_head() {
    let env = project();
    fs::write(env.root().join("a.txt"), "one\n").unwrap();
    ok(&env, &["save", "One"]);
    let head = env.root().join(".bolt/state/head");
    let older = fs::read(&head).unwrap();
    // A day's work, saved twice; the folder then moves on.
    fs::write(env.root().join("a.txt"), "monday's work\n").unwrap();
    ok(&env, &["save", "Monday"]);
    fs::write(env.root().join("a.txt"), "tuesday's work\n").unwrap();
    ok(&env, &["save", "Tuesday"]);
    fs::write(env.root().join("a.txt"), "wednesday, unsaved\n").unwrap();
    // A sync tool or a restore puts back an older `state/head`, which still reads.
    fs::write(&head, &older).unwrap();
    let status = ok(&env, &["status"]);
    age_packs(&env, 48);
    let tidy = env.bolt().arg("tidy").output().unwrap();
    let monday = kept(&env, "a.txt", b"monday's work\n");
    let tuesday = kept(&env, "a.txt", b"tuesday's work\n");
    assert!(
        monday && tuesday,
        "status before tidy:\n{status}\ntidy: {:?}\n{}{}\nMonday's save still readable: \
         {monday}; Tuesday's: {tuesday}",
        tidy.status.code(),
        text(&tidy.stdout),
        text(&tidy.stderr)
    );
}

/// 64 MiB of `seed`'s pattern: a file big enough that writing it takes a while.
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

#[test]
fn rt6_2_a_file_saved_while_undo_writes_over_it_is_never_lost() {
    let env = project();
    let file = env.root().join("art.psd");
    fs::write(&file, big(1)).unwrap();
    ok(&env, &["save", "First"]);
    let first = ok(&env, &["--json", "status"]);
    let first: serde_json::Value = serde_json::from_str(&first).unwrap();
    let first = first["save"]["id"].as_str().unwrap().to_owned();
    fs::write(&file, big(2)).unwrap();
    ok(&env, &["save", "Second"]);
    ok(&env, &["goto", &first]);
    // The undo writes the second version back. While it writes it in .bolt/tmp, the artist's
    // program saves their new work over the file it is about to replace.
    let theirs = big(3);
    let tmp = env.root().join(".bolt/tmp");
    let watcher = {
        let (tmp, file, theirs) = (tmp.clone(), file.clone(), theirs.clone());
        std::thread::spawn(move || {
            let start = Instant::now();
            while start.elapsed() < Duration::from_secs(60) {
                let writing = fs::read_dir(&tmp).into_iter().flatten().flatten().any(|e| {
                    e.file_name().to_string_lossy().starts_with("tmp-")
                        // Asked of the file itself: a folder listing on Windows lags its size.
                        && fs::metadata(e.path()).is_ok_and(|m| m.len() > 1 << 20)
                });
                if writing {
                    fs::write(&file, &theirs).unwrap();
                    return true;
                }
                std::thread::sleep(Duration::from_millis(1));
            }
            false
        })
    };
    let out = env.bolt().arg("undo").output().unwrap();
    let raced = watcher.join().unwrap();
    assert!(raced, "the undo never wrote a big temporary file");
    let survived = kept(&env, "art.psd", &theirs);
    assert!(
        survived,
        "undo: {:?}\n{}{}\nthe work saved during the undo survived: {survived}",
        out.status.code(),
        text(&out.stdout),
        text(&out.stderr)
    );
}

#[cfg(target_os = "linux")]
#[test]
fn rt6_4_a_huge_journal_costs_status_little_time_and_memory() {
    let env = project();
    fs::write(env.root().join("a.txt"), "one\n").unwrap();
    ok(&env, &["save", "One"]);
    // A hole of 4 GiB less a byte: nothing on disk, so it is planted in an instant.
    let journal = fs::File::create(env.root().join(".bolt/state/journal")).unwrap();
    journal.set_len((4 << 30) - 1).unwrap();
    drop(journal);
    let mut cmd = env.bolt_process_in(env.root());
    cmd.arg("status");
    let ended = support::run_limited(cmd, 120);
    assert!(
        ended.code == Some(0) && ended.secs < 10.0 && ended.peak_mb < 1024,
        "bolt status: {ended:?}"
    );
}

/// Runs `bolt <args>` on copies of `template` until a run stops at `label`; that copy.
#[cfg(feature = "fault-injection")]
fn stopped_at(template: &Env, args: &[&str], label: &str) -> Env {
    for n in 1..100 {
        let env = Env::empty();
        support::copy_dir(template.root(), env.root());
        let out = env
            .bolt()
            .args(args)
            .env("BOLT_FAULT_AT", n.to_string())
            .output()
            .unwrap();
        let said = text(&out.stderr);
        assert!(
            said.contains("injected crash"),
            "{args:?} never stopped at {label}"
        );
        if said.contains(&format!("injected crash at {label}\n")) {
            return env;
        }
    }
    panic!("{args:?} never stopped at {label}");
}

#[cfg(feature = "fault-injection")]
#[test]
#[ignore = "RT6-3: picking up a lost head during a stopped switch leaves every command refused"]
fn rt6_3_picking_up_a_lost_head_after_a_stopped_switch_lets_the_project_go_on() {
    let template = project();
    fs::write(template.root().join("a.txt"), "one\n").unwrap();
    fs::write(template.root().join("b.txt"), "bee\n").unwrap();
    ok(&template, &["save", "One"]);
    fs::write(template.root().join("a.txt"), "work A\n").unwrap();
    fs::write(template.root().join("b.txt"), "work B\n").unwrap();
    // The discard stops after its first file, and `state/head` is then lost.
    let env = stopped_at(&template, &["discard"], "switch:changed");
    fs::remove_file(env.root().join(".bolt/state/head")).unwrap();
    let ends: serde_json::Value =
        serde_json::from_str(&ok(&env, &["--json", "undo", "--list"])).unwrap();
    let ends = ends["ends"].as_array().unwrap();
    // The only entry offered is the stopped discard's own.
    assert_eq!(ends.len(), 1, "{ends:?}");
    assert_eq!(ends[0]["command"], "discard");
    let pick = ends[0]["entry"].as_str().unwrap().to_owned();
    let picked = ok(&env, &["undo", "--to", &pick]);
    let next = env.bolt().args(["discard"]).output().unwrap();
    let undo = env.bolt().args(["undo"]).output().unwrap();
    assert!(
        next.status.success() && undo.status.success(),
        "undo --to {pick}: {picked}\ndiscard: {:?} {}\nundo: {:?} {}",
        next.status.code(),
        text(&next.stderr),
        undo.status.code(),
        text(&undo.stderr)
    );
}
