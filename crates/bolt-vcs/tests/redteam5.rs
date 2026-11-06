//! Red-team proof tests for milestone 3 (decision 024): each asserts the safe behaviour and fails
//! on the code the session attacked. See docs/redteam/2026-11-06-milestone-3.md.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use std::fs;
use std::path::Path;
use std::time::{Duration, SystemTime};

use support::{Env, text};

fn project() -> Env {
    let env = Env::empty();
    env.bolt().arg("start").assert().success();
    env
}

fn set_time(path: &Path, time: SystemTime) {
    let file = fs::File::options().write(true).open(path).unwrap();
    file.set_modified(time).unwrap();
}

fn json(env: &Env, args: &[&str]) -> serde_json::Value {
    let out = env.bolt().arg("--json").args(args).output().unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    serde_json::from_slice(&out.stdout).unwrap()
}

fn walk(dir: &Path) -> Vec<(String, String)> {
    let mut out = vec![];
    let mut todo = vec![dir.to_owned()];
    while let Some(d) = todo.pop() {
        for e in fs::read_dir(&d).unwrap() {
            let e = e.unwrap();
            let p = e.path();
            let rel = p.strip_prefix(dir).unwrap().to_string_lossy().into_owned();
            if rel.starts_with(".bolt") {
                continue;
            }
            if e.file_type().unwrap().is_dir() {
                todo.push(p);
                out.push((rel, "<dir>".into()));
            } else {
                out.push((
                    rel,
                    String::from_utf8_lossy(&fs::read(&p).unwrap_or_default()).into_owned(),
                ));
            }
        }
    }
    out.sort();
    out
}

/// 1985-10-26 08:15 UTC, the time npm gives every file in a package; zip and reproducible-build
/// tools give whole trees one time too.
fn one_time() -> SystemTime {
    SystemTime::UNIX_EPOCH + Duration::from_secs(499_162_500)
}

fn write_at(env: &Env, name: &str, body: &str, time: SystemTime) {
    fs::write(env.root().join(name), body).unwrap();
    set_time(&env.root().join(name), time);
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

#[test]
fn rt5_1_a_file_renamed_over_a_saved_one_is_seen_and_never_lost() {
    let env = project();
    write_at(
        &env,
        "a.txt",
        "XXXX",
        SystemTime::now() - Duration::from_secs(3600),
    );
    env.bolt().args(["save", "First"]).assert().success();
    let first = json(&env, &["status"])["save"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    // Unpacked from one archive: both files have its one time.
    write_at(&env, "a.txt", "AAAA", one_time());
    env.bolt()
        .args(["save", "Second", "a.txt"])
        .assert()
        .success();
    let saved = json(&env, &["status"]);
    write_at(&env, "notes.txt", "KKKK", one_time());
    let before = json(&env, &["status"]);
    // The user moves their notes over a.txt: no time is set, nothing is rewritten.
    fs::rename(env.root().join("notes.txt"), env.root().join("a.txt")).unwrap();
    let after = json(&env, &["status"]);
    let paths = |doc: &serde_json::Value| -> Vec<String> {
        doc["changes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| format!("{} {}", c["change"], c["path"]))
            .collect()
    };
    let shown = paths(&after);
    let out = env.bolt().args(["goto", &first]).output().unwrap();
    let survived = kept(&env, "a.txt", b"KKKK");
    assert!(
        shown.iter().any(|c| c.contains("a.txt")) && survived,
        "saved: {:?}\nbefore the rename: {:?}\nafter it: {shown:?}\ngoto: {:?} {}\n\
         the notes (KKKK) survived the goto: {survived}",
        paths(&saved),
        paths(&before),
        out.status.code(),
        text(&out.stderr)
    );
}

#[test]
#[ignore = "RT5-2: an ignored file in a folder goto replaces with a file half-switches and blames a change mid-switch"]
fn rt5_2_an_ignored_file_in_a_folder_becoming_a_file_is_in_the_way() {
    let env = project();
    fs::write(env.root().join(".gitignore"), "*.o\n").unwrap();
    fs::create_dir(env.root().join("d")).unwrap();
    fs::write(env.root().join("d/f"), "f").unwrap();
    env.bolt().args(["save", "A"]).assert().success();
    let a = json(&env, &["status"])["save"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    fs::remove_dir_all(env.root().join("d")).unwrap();
    fs::write(env.root().join("d"), "file d").unwrap();
    env.bolt().args(["save", "B"]).assert().success();
    env.bolt().args(["goto", &a]).assert().success();
    // A build output, ignored, made before the goto.
    fs::write(env.root().join("d/x.o"), "build").unwrap();
    let out = env.bolt().args(["goto", "main"]).output().unwrap();
    let said = text(&out.stderr) + &text(&out.stdout);
    let status = json(&env, &["status"]);
    let whole = if out.status.success() {
        fs::read(env.root().join("d")).is_ok_and(|b| b == b"file d")
    } else {
        fs::read(env.root().join("d/f")).is_ok_and(|b| b == b"f")
    };
    assert!(
        whole && !said.contains("changed while Bolt was switching"),
        "goto: {:?}\n{said}\nfolder now: {:?}\nstatus: {}",
        out.status.code(),
        walk(env.root()),
        status["changes"]
    );
}

fn hex_bytes(hex: &str) -> Vec<u8> {
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
        .collect()
}

#[test]
#[ignore = "RT5-3: a line named like a save's short change ID silently takes goto elsewhere"]
fn rt5_3_goto_by_the_change_id_history_shows_reaches_that_save() {
    let env = Env::new();
    let good = env.commit("app.txt", "safe version\n", "Fix the security bug");
    env.commit("app.txt", "newer\n", "Newer work");
    // The hostile repository names a branch with the fix's change ID as Bolt will show it.
    let letters = bolt_core::id::ChangeId::from_git_commit(&hex_bytes(&good)).to_letters();
    let short = &letters[..4];
    env.git(&["checkout", "-q", "-b", "side", &good]);
    env.commit("app.txt", "attacker's version\n", "Unrelated");
    env.git(&["branch", "-q", short]);
    env.git(&["checkout", "-q", "main"]);
    env.git(&["branch", "-q", "-D", "side"]);
    env.import();
    let history = text(&env.bolt().args(["history"]).output().unwrap().stdout);
    let shown = history
        .lines()
        .find(|l| l.contains("Fix the security bug"))
        .unwrap()
        .to_owned();
    assert!(shown.contains(short), "{history}");
    // The user copies the change ID history shows for the fix.
    let out = env.bolt().args(["goto", short]).output().unwrap();
    let now = fs::read_to_string(env.root().join("app.txt")).unwrap();
    assert!(
        now == "safe version\n",
        "history shows: {shown}\nbolt goto {short}: {}{}app.txt now holds: {now:?}",
        text(&out.stdout),
        text(&out.stderr)
    );
}

#[cfg(unix)]
#[test]
#[ignore = "RT5-4: goto sets the executable bit through a hard link, on a file outside the project"]
fn rt5_4_goto_never_changes_a_file_outside_through_a_hard_link() {
    use std::os::unix::fs::PermissionsExt;
    let outside = tempfile::tempdir().unwrap();
    let theirs = outside.path().join("deploy-key-helper");
    fs::write(&theirs, "#!/bin/sh\necho hi\n").unwrap();
    fs::set_permissions(&theirs, fs::Permissions::from_mode(0o644)).unwrap();
    let env = project();
    let tool = env.root().join("tool.sh");
    fs::write(&tool, "#!/bin/sh\necho hi\n").unwrap();
    fs::set_permissions(&tool, fs::Permissions::from_mode(0o644)).unwrap();
    env.bolt().args(["save", "Plain"]).assert().success();
    let plain = json(&env, &["status"])["save"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    fs::set_permissions(&tool, fs::Permissions::from_mode(0o755)).unwrap();
    env.bolt().args(["save", "Executable"]).assert().success();
    env.bolt().args(["goto", &plain]).assert().success();
    // The user keeps one copy of the script, linked into the project.
    fs::remove_file(&tool).unwrap();
    fs::hard_link(&theirs, &tool).unwrap();
    let out = env.bolt().args(["goto", "main"]).output().unwrap();
    let mode = fs::metadata(&theirs).unwrap().permissions().mode() & 0o777;
    assert_eq!(
        mode,
        0o644,
        "the file outside the project is now {mode:o}; goto: {:?} {}",
        out.status.code(),
        text(&out.stderr)
    );
}
