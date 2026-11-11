//! Exploration (not a proof): `verify`'s peak memory on a planted save whose listings give every
//! file a wrong size, against the same save with right sizes, and `tidy`'s on the first.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use bolt_core::listing::{Entry, EntryKind, Listing};
use bolt_core::pack::ObjectKind;
use support::{Env, main_tip, plant_save, run_limited};

fn planted(listings: usize, per: usize, size: u64) -> Env {
    let env = Env::empty();
    env.bolt()
        .args(["start", "--preset", "custom"])
        .assert()
        .success();
    let first = main_tip(&env).to_hex();
    env.bolt().args(["goto", &first]).assert().success();
    plant_save(env.root(), |p| {
        let x = p.add_content(b"x").unwrap();
        let mut folders = Vec::new();
        for l in 0..listings {
            let entries = (0..per)
                .map(|i| Entry {
                    name: format!("{l:02}{i:08}{}", "a".repeat(755)),
                    kind: EntryKind::File,
                    id: x,
                    size,
                })
                .collect();
            let bytes = Listing::new(entries).unwrap().encode();
            let id = p.add_record(ObjectKind::FolderListing, &bytes).unwrap();
            folders.push(Entry {
                name: format!("d{l}"),
                kind: EntryKind::Folder,
                id,
                size: 0,
            });
        }
        let top = Listing::new(folders).unwrap().encode();
        p.add_record(ObjectKind::FolderListing, &top).unwrap()
    });
    env
}

#[test]
fn rt11_explore_verify_memory() {
    let listings: usize = std::env::var("RT11_LISTINGS").map_or(4, |v| v.parse().unwrap());
    let per: usize = std::env::var("RT11_PER").map_or(200_000, |v| v.parse().unwrap());
    for (what, size) in [("right sizes", 1), ("wrong sizes", 2)] {
        let env = planted(listings, per, size);
        let mut cmd = env.bolt_process_in(env.root());
        cmd.arg("verify");
        let ended = run_limited(cmd, 1200);
        eprintln!(
            "{what}: {listings} listings of {per} entries: verify code {:?} in {:.1}s, peak {} MiB",
            ended.code, ended.secs, ended.peak_mb
        );
        if size == 2 {
            let mut cmd = env.bolt_process_in(env.root());
            cmd.arg("tidy");
            let ended = run_limited(cmd, 1200);
            eprintln!(
                "{what}: tidy code {:?} in {:.1}s, peak {} MiB",
                ended.code, ended.secs, ended.peak_mb
            );
        }
    }
}
