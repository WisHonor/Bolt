//! RT13 exploration: the memory of `status`, `changes`, `goto` and `save` on the widest last save
//! the limits allow: 9 folders that share one listing of 1,000,000 files (9 million entries, under
//! the 10 million a save may hold), none of them on disk. Peak memory (ru_maxrss) and time, this
//! build's release `bolt` against main's, each on a fresh copy of the project.
//! Run in the sandbox only: redteam/attacks12/run.sh explore rt13_wide_memory
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use std::process::Command;
use std::time::Instant;

use bolt_core::listing::EntryKind;
use support::{Env, copy_dir, text};

const NEW: &str = "/work/target/release/bolt";
const MAIN: &str = "/work/_main/target/release/bolt";
const MEASURE: &str = "import resource,subprocess,sys
p = subprocess.run(sys.argv[1:], stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
print(p.returncode, resource.getrusage(resource.RUSAGE_CHILDREN).ru_maxrss // 1024, \
p.stderr[-160:].decode('utf-8', 'replace').replace(chr(10), '|'))";

fn measure(env: &Env, bin: &str, dir: &std::path::Path, args: &[&str]) -> String {
    let t = Instant::now();
    let out = Command::new("python3")
        .args(["-I", "-c", MEASURE, bin])
        .args(args)
        .current_dir(dir)
        .env("BOLT_USER_NAME", "Test")
        .env("BOLT_USER_EMAIL", "test@example.com")
        .env("BOLT_CONFIG_DIR", env.config())
        .env("HOME", env.config())
        .output()
        .unwrap();
    format!(
        "{:.1} s, exit/peak MB/said: {}{}",
        t.elapsed().as_secs_f64(),
        text(&out.stdout).trim(),
        text(&out.stderr)
    )
}

#[test]
fn widest_save_memory() {
    let env = Env::empty();
    env.bolt().arg("start").assert().success();
    let first = support::main_tip(&env).to_hex();
    let names: Vec<String> = (0..1_000_000).map(|n| format!("f{n:07}")).collect();
    let folders: Vec<String> = (0..9).map(|n| format!("d{n}")).collect();
    support::plant_save(env.root(), |pack| {
        let empty = pack.add_content(b"").unwrap();
        let wide = support::add_listing(pack, &names, EntryKind::File, empty);
        support::add_listing(pack, &folders, EntryKind::Folder, wide)
    });
    drop(names);
    for (name, bin) in [("new", NEW), ("main", MAIN)] {
        let dir = tempfile::tempdir().unwrap();
        copy_dir(env.root(), dir.path());
        for args in [
            &["status"][..],
            &["status"],
            &["changes"],
            &["goto", &first],
            &["goto", "main"],
            &["save", "--yes", "gone"],
        ] {
            println!("{name} {args:?}: {}", measure(&env, bin, dir.path(), args));
        }
    }
}
