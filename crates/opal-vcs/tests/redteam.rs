//! Red-team proof tests for milestone 1 (decision 024): each asserts the safe behaviour and fails
//! without the fix for the finding it is named after (`win_junction_…` always passed).
//! See docs/redteam/2026-11-01-milestone-1.md.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::Path;
use std::process::Command;

use opal_core::listing::{EntryKind, Listing};
use opal_core::pack::ObjectKind;

mod support;
#[cfg(unix)]
use support::opal_capped;
use support::{add_listing, plant_save, run_limited};

struct Env(support::Env);

impl std::ops::Deref for Env {
    type Target = support::Env;
    fn deref(&self) -> &support::Env {
        &self.0
    }
}

impl Env {
    fn new() -> Env {
        Env(support::Env::empty())
    }

    fn opal_anonymous(&self, dir: &Path) -> Command {
        let mut cmd = self.opal_in(dir);
        cmd.env_remove("OPAL_USER_NAME")
            .env_remove("OPAL_USER_EMAIL");
        cmd
    }

    fn opal_in(&self, dir: &Path) -> Command {
        self.opal_process_in(dir)
    }

    fn opal(&self) -> Command {
        self.opal_in(self.root())
    }

    fn ok(&self, args: &[&str]) -> String {
        let out = self.opal().args(args).output().unwrap();
        assert_eq!(
            out.status.code(),
            Some(0),
            "opal {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).into_owned()
    }
}

#[cfg(unix)]
fn mkfifo(path: &Path) {
    let st = Command::new("mkfifo").arg(path).status().unwrap();
    assert!(st.success());
}

// RT-1: a symlinked `.opal/tmp` makes ANY command delete files outside the project.

#[cfg(unix)]
#[test]
fn rt1_symlinked_opal_tmp_does_not_delete_files_outside_the_project() {
    let env = Env::new();
    env.ok(&["start", "--preset", "custom"]);
    let victim = tempfile::tempdir().unwrap(); // say, ~/Documents
    std::fs::write(victim.path().join("thesis.docx"), "four years of work").unwrap();
    let tmp = env.root().join(".opal/tmp");
    std::fs::remove_dir_all(&tmp).unwrap();
    std::os::unix::fs::symlink(victim.path(), &tmp).unwrap();

    // A read-only command, run by someone who just unpacked a shared project.
    let _ = env.opal().arg("history").output().unwrap();

    assert!(
        victim.path().join("thesis.docx").exists(),
        "`opal history` deleted a file outside the project through .opal/tmp"
    );
}

/// Windows twin of RT-1: a junction needs no privileges.
#[cfg(windows)]
#[test]
fn rt1_windows_junctioned_opal_tmp_does_not_delete_files_outside_the_project() {
    let env = Env::new();
    env.ok(&["start", "--preset", "custom"]);
    let victim = tempfile::tempdir().unwrap();
    std::fs::write(victim.path().join("thesis.docx"), "four years of work").unwrap();
    let tmp = env.root().join(".opal").join("tmp");
    std::fs::remove_dir_all(&tmp).unwrap();
    support::junction(&tmp, victim.path());
    let _ = env.opal().arg("history").output().unwrap();
    assert!(victim.path().join("thesis.docx").exists());
}

/// Windows: a junction INSIDE the project that points outside must be saved as a link, never walked
/// into. It passes because junctions count as symlinks for both `ignore` and `symlink_metadata`.
#[cfg(windows)]
#[test]
fn win_junction_in_project_is_not_walked_into() {
    use opal_core::repo::Repository;
    let env = Env::new();
    env.ok(&["start", "--preset", "custom"]);
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("secret.txt"), "hunter2").unwrap();
    support::junction(&env.root().join("j"), outside.path());
    env.ok(&["save", "with junction"]);
    let repo = Repository::open(env.root()).unwrap();
    let (_, entry) = repo.current().unwrap();
    let save = repo
        .store()
        .save(&entry.state.lines["main"], false)
        .unwrap();
    let root = repo.store().listing(&save.root).unwrap();
    let saved = root.get("j").expect("the junction is not in the save");
    assert_eq!(saved.kind, opal_core::listing::EntryKind::Symlink);
}

// RT-2: a symlinked `.opal/store/packs` makes `opal save` write outside the project.

#[cfg(unix)]
#[test]
fn rt2_symlinked_packs_folder_does_not_make_save_write_outside_the_project() {
    let env = Env::new();
    env.ok(&["start", "--preset", "custom"]);
    let outside = tempfile::tempdir().unwrap();
    let packs = env.root().join(".opal/store/packs");
    for e in std::fs::read_dir(&packs).unwrap() {
        let p = e.unwrap().path();
        std::fs::rename(&p, outside.path().join(p.file_name().unwrap())).unwrap();
    }
    std::fs::remove_dir(&packs).unwrap();
    std::os::unix::fs::symlink(outside.path(), &packs).unwrap();
    let before = std::fs::read_dir(outside.path()).unwrap().count();

    std::fs::write(env.root().join("a.txt"), "hello").unwrap();
    let _ = env.opal().args(["save", "add a"]).output().unwrap();

    let after = std::fs::read_dir(outside.path()).unwrap().count();
    assert_eq!(
        before, after,
        "`opal save` wrote a new file into a folder outside the project"
    );
}

#[cfg(unix)]
#[test]
fn rt2_symlinked_lock_file_does_not_create_files_outside_the_project() {
    let env = Env::new();
    env.ok(&["start", "--preset", "custom"]);
    let outside = tempfile::tempdir().unwrap();
    let target = outside.path().join("created-by-opal");
    let lock = env.root().join(".opal/state/lock");
    let _ = std::fs::remove_file(&lock);
    std::os::unix::fs::symlink(&target, &lock).unwrap();
    let _ = env.opal().arg("history").output().unwrap();
    assert!(
        !target.exists(),
        "`opal history` created a file outside the project through .opal/state/lock"
    );
}

#[cfg(unix)]
#[test]
fn rt2_start_does_not_build_a_repository_through_a_symlinked_opal_folder() {
    let env = Env::new();
    let outside = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink(outside.path(), env.root().join(".opal")).unwrap();
    let _ = env
        .opal()
        .args(["start", "--preset", "custom"])
        .output()
        .unwrap();
    assert_eq!(
        std::fs::read_dir(outside.path()).unwrap().count(),
        0,
        "`opal start` wrote its repository into a folder outside the project"
    );
}

// RT-3: FIFOs planted in `.opal` hang every command forever.

#[cfg(unix)]
#[test]
fn rt3_fifo_as_state_head_does_not_hang_history() {
    let env = Env::new();
    env.ok(&["start", "--preset", "custom"]);
    let head = env.root().join(".opal/state/head");
    std::fs::remove_file(&head).unwrap();
    mkfifo(&head);
    let mut cmd = env.opal();
    cmd.arg("history");
    let end = run_limited(cmd, 10);
    assert!(
        end.code == Some(1),
        "`opal history` hung or crashed on a FIFO at state/head: {end:?}"
    );
}

#[cfg(unix)]
#[test]
fn rt3_fifo_named_like_a_pack_does_not_hang_history() {
    let env = Env::new();
    env.ok(&["start", "--preset", "custom"]);
    mkfifo(&env.root().join(".opal/store/packs/zz.pack"));
    // History may never reach the FIFO, since packs are opened only when searched.
    let mut cmd = env.opal();
    cmd.arg("history");
    let end = run_limited(cmd, 10);
    assert!(
        !end.timed_out && matches!(end.code, Some(0 | 1)),
        "`opal history` hung or crashed on a FIFO named *.pack: {end:?}"
    );
    // A save looks for its new file in every pack, so it reaches the FIFO: it passes over it as
    // damaged, naming it, and stores the file in a pack of its own.
    std::fs::write(env.root().join("new.txt"), "new").unwrap();
    let mut cmd = env.opal();
    cmd.args(["save", "add new"]);
    let end = run_limited(cmd, 10);
    assert!(
        !end.timed_out && end.code == Some(0) && end.stderr.contains("zz.pack"),
        "`opal save` hung, failed or didn't name a FIFO named *.pack: {end:?}"
    );
    // The new save reads back: status reads its listing.
    let mut cmd = env.opal();
    cmd.arg("status");
    let end = run_limited(cmd, 10);
    assert!(
        !end.timed_out && end.code == Some(0),
        "`opal status` hung or failed after the save: {end:?}"
    );
    // The check that reads every pack passes over the FIFO too, naming it.
    let packs = env.root().join(".opal/store/packs");
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || tx.send(opal_core::verify::verify_store(&packs).problems));
    let problems = rx
        .recv_timeout(std::time::Duration::from_secs(10))
        .expect("verify hung on a FIFO named *.pack");
    assert!(
        problems.iter().any(|p| p.starts_with("zz.pack: ")),
        "{problems:?}"
    );
}

#[cfg(unix)]
#[test]
fn rt3_fifo_as_project_config_does_not_hang_save() {
    let env = Env::new();
    env.ok(&["start", "--preset", "custom"]);
    mkfifo(&env.root().join(".opal/config.toml"));
    std::fs::write(env.root().join("a.txt"), "a").unwrap();
    // Without an identity the save stops before it reads the project's settings.
    let mut cmd = env.opal_anonymous(env.root());
    cmd.args(["save", "add a"]);
    let end = run_limited(cmd, 10);
    assert!(
        end.code == Some(1) && end.stderr.contains("needs your name and email"),
        "`opal save` hung or crashed on a FIFO at .opal/config.toml: {end:?}"
    );
    // With one, it reads them for the note about an ignored identity, and must not hang there.
    let mut cmd = env.opal();
    cmd.args(["save", "add a"]);
    let end = run_limited(cmd, 10);
    assert!(
        end.code == Some(0),
        "`opal save` hung or failed on a FIFO at .opal/config.toml: {end:?}"
    );
}

// RT-4: `.opal` files that are symlinks to /dev/zero are read without a size limit.

#[cfg(unix)]
#[test]
fn rt4_state_head_pointing_at_dev_zero_is_refused_not_read_forever() {
    let env = Env::new();
    env.ok(&["start", "--preset", "custom"]);
    let head = env.root().join(".opal/state/head");
    std::fs::remove_file(&head).unwrap();
    std::os::unix::fs::symlink("/dev/zero", &head).unwrap();
    let end = run_limited(opal_capped(env.config(), env.root(), &["history"]), 60);
    // Without the cap the process grows to 4 GB and is OOM-killed (redteam/attacks/rt4-*.sh).
    assert!(
        end.code == Some(1) && !end.stderr.contains("memory"),
        "expected a clean 'damaged' error, not running out of memory: {end:?}"
    );
}

#[cfg(unix)]
#[test]
fn rt4_project_config_pointing_at_dev_zero_is_refused_not_read_forever() {
    let env = Env::new();
    env.ok(&["start", "--preset", "custom"]);
    std::os::unix::fs::symlink("/dev/zero", env.root().join(".opal/config.toml")).unwrap();
    std::fs::write(env.root().join("a.txt"), "a").unwrap();
    let mut cmd = opal_capped(env.config(), env.root(), &["save", "add a"]);
    cmd.env_remove("OPAL_USER_NAME")
        .env_remove("OPAL_USER_EMAIL");
    let end = run_limited(cmd, 60);
    assert!(
        end.code == Some(1) && !end.stderr.contains("memory"),
        "expected a clean settings error, not running out of memory: {end:?}"
    );
}

// RT-5: a file swapped for a symlink between the scan and the read is saved as the symlink's
// TARGET: `opal save` copies a file from outside the project into history.

#[cfg(unix)]
#[test]
fn rt5_save_never_stores_the_target_of_a_file_swapped_for_a_symlink() {
    use opal_core::listing::EntryKind;
    use opal_core::repo::Repository;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    const SECRET: &[u8] = b"-----BEGIN OPENSSH PRIVATE KEY----- hunter2";
    let env = Env::new();
    env.ok(&["start", "--preset", "custom"]);
    let outside = tempfile::tempdir().unwrap();
    let secret = outside.path().join("id_ed25519");
    std::fs::write(&secret, SECRET).unwrap();

    // Enough files that the gap between the scan and the read of the top folder is wide.
    let bulk = env.root().join("bulk");
    std::fs::create_dir(&bulk).unwrap();
    for i in 0..1500 {
        std::fs::write(bulk.join(format!("f{i:05}")), format!("{i}").repeat(2000)).unwrap();
    }
    let victim = env.root().join("victim.txt");
    std::fs::write(&victim, "x").unwrap();

    // The attacker, who can write in the project folder, flips victim.txt between a plain file
    // and a symlink to the secret, each step an atomic rename.
    let stop = Arc::new(AtomicBool::new(false));
    let flipper = {
        let (stop, root, secret) = (stop.clone(), env.root().to_owned(), secret.clone());
        std::thread::spawn(move || {
            let (link, plain, victim) = (root.join(".l"), root.join(".p"), root.join("victim.txt"));
            while !stop.load(Ordering::Relaxed) {
                let _ = std::os::unix::fs::symlink(&secret, &link);
                let _ = std::fs::rename(&link, &victim);
                let _ = std::fs::write(&plain, "x");
                let _ = std::fs::rename(&plain, &victim);
            }
        })
    };

    let mut stolen_on = None;
    for attempt in 0..40 {
        let _ = env
            .opal()
            .args(["save", &format!("attempt {attempt}")])
            .output()
            .unwrap();
        let repo = Repository::open(env.root()).unwrap();
        let (_, entry) = repo.current().unwrap();
        let save = repo
            .store()
            .save(&entry.state.lines["main"], false)
            .unwrap();
        let root = repo.store().listing(&save.root).unwrap();
        if let Some(e) = root.get("victim.txt")
            && e.kind == EntryKind::File
        {
            let mut bytes = Vec::new();
            repo.store().copy_content(&e.id, &mut bytes).unwrap();
            if bytes == SECRET {
                stolen_on = Some(attempt);
                break;
            }
        }
    }
    stop.store(true, Ordering::Relaxed);
    flipper.join().unwrap();
    assert_eq!(
        stolen_on, None,
        "a file from OUTSIDE the project was stored in history as victim.txt"
    );
}

// RT-6: deeply nested folder listings in a shared repository crash `opal show` (and the
// diff that runs after every `opal save`).

/// Commits, through Opalro's own library, a save whose top folder is `a/a/a/...` `depth` deep.
fn plant_deep_save(root: &Path, depth: usize) {
    plant_save(root, |pack| {
        let mut id = pack
            .add_record(ObjectKind::FolderListing, &Listing::empty().encode())
            .unwrap();
        for _ in 0..depth {
            id = add_listing(pack, &["a".to_owned()], EntryKind::Folder, id);
        }
        id
    });
}

#[test]
fn rt6_deeply_nested_listings_do_not_crash_show() {
    let env = Env::new();
    env.ok(&["start", "--preset", "custom"]);
    plant_deep_save(env.root(), 100_000);
    let mut cmd = env.opal();
    cmd.arg("show");
    let end = run_limited(cmd, 120);
    assert!(
        matches!(end.code, Some(0 | 1)),
        "`opal show` died instead of answering or refusing: {end:?}"
    );
}

#[test]
fn rt6_deeply_nested_listings_do_not_crash_the_next_save() {
    let env = Env::new();
    env.ok(&["start", "--preset", "custom"]);
    plant_deep_save(env.root(), 100_000);
    std::fs::write(env.root().join("mine.txt"), "my work").unwrap();
    let mut cmd = env.opal();
    cmd.args(["save", "my work"]);
    let end = run_limited(cmd, 120);
    assert!(
        matches!(end.code, Some(0 | 1)),
        "`opal save` died instead of answering or refusing: {end:?}"
    );
}

// RT-10: a "tree bomb": four tiny listings of 1,000 entries that all point at the same
// sub-listing describe 10^9 files. `opal show` tries to list them all.

#[cfg(unix)]
#[test]
fn rt10_tree_bomb_does_not_exhaust_memory_in_show() {
    let env = Env::new();
    env.ok(&["start", "--preset", "custom"]);
    plant_save(env.root(), |pack| {
        let names: Vec<String> = (0..1000).map(|i| format!("n{i:03}")).collect();
        let empty = pack.add_content(b"").unwrap();
        let mut id = add_listing(pack, &names, EntryKind::File, empty);
        for _ in 0..2 {
            id = add_listing(pack, &names, EntryKind::Folder, id);
        }
        id
    });
    let pack_bytes: u64 = std::fs::read_dir(env.root().join(".opal/store/packs"))
        .unwrap()
        .map(|e| e.unwrap().metadata().unwrap().len())
        .sum();
    eprintln!("all packs together: {pack_bytes} bytes");
    let end = run_limited(opal_capped(env.config(), env.root(), &["show"]), 120);
    assert!(
        matches!(end.code, Some(0 | 1)) && !end.stderr.contains("memory"),
        "`opal show` on a ~{pack_bytes}-byte repository died or hung: {end:?}"
    );
}

// RT-7: Unicode format characters (bidi overrides) pass the terminal sanitizer.

#[test]
fn rt7_bidi_override_in_a_file_name_is_not_printed_raw() {
    let env = Env::new();
    env.ok(&["start", "--preset", "custom"]);
    // Shows as "invoice_exe.pdf" in a bidi-aware terminal; really ends in ".exe".
    std::fs::write(env.root().join("invoice_\u{202E}fdp.exe"), "MZ").unwrap();
    env.ok(&["save", "add the invoice\u{2028}  removed   payroll.xlsx"]);
    let shown = env.ok(&["show"]);
    let history = env.ok(&["history"]);
    for out in [&shown, &history] {
        assert!(
            !out.contains(['\u{202E}', '\u{2028}']),
            "raw bidi override / line separator reached the terminal: {out:?}"
        );
    }
}

// RT-8: `opal start` prints the project path without sanitizing it.

#[cfg(unix)]
#[test]
fn rt8_start_does_not_print_escape_sequences_from_the_folder_name() {
    let env = Env::new();
    let dir: std::path::PathBuf = env.root().join("proj\u{1b}]0;pwned\u{7}\u{1b}[2J");
    std::fs::create_dir(&dir).unwrap();
    let out = env
        .opal_in(&dir)
        .args(["start", "--preset", "custom"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        !stdout.contains(['\u{1b}', '\u{7}']),
        "terminal escapes from the folder name reached stdout: {stdout:?}"
    );
}

// RT-9: a shared project's `.opal/config.toml` decides who YOUR saves say you are.

#[test]
fn rt9_project_config_cannot_override_the_users_own_identity() {
    let env = Env::new();
    std::fs::write(
        env.config().join("config.toml"),
        "[user]\nname = \"Real User\"\nemail = \"real@example.com\"\n",
    )
    .unwrap();
    assert!(
        env.opal_anonymous(env.root())
            .args(["start", "--preset", "custom"])
            .status()
            .unwrap()
            .success()
    );
    // What the person who shared the project zip planted.
    std::fs::write(
        env.root().join(".opal/config.toml"),
        "[user]\nname = \"Mallory\"\nemail = \"mallory@evil.example\"\n",
    )
    .unwrap();
    std::fs::write(env.root().join("a.txt"), "my work").unwrap();
    assert!(
        env.opal_anonymous(env.root())
            .args(["save", "my work"])
            .status()
            .unwrap()
            .success()
    );
    let out = env
        .opal_anonymous(env.root())
        .args(["history", "--json"])
        .output()
        .unwrap();
    let history = String::from_utf8_lossy(&out.stdout);
    assert!(
        !history.contains("Mallory"),
        "the user's save was recorded under the planted identity: {history}"
    );
}
