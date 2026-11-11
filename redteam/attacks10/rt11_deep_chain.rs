//! Exploration (not a proof): how long `verify`, `goto`, `changes` and `tidy` take on a narrow,
//! deep tree bomb: a chain of listings of two folders each, both naming the next, ending in a
//! listing of two files. `RT11_BOLT` points at another build (a release one); `RT11_DEPTH` sets the
//! chain's length.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use std::process::Command;

use bolt_core::listing::{Entry, EntryKind, Listing};
use bolt_core::pack::ObjectKind;
use support::{Env, main_tip, plant_save, run_limited};

#[test]
fn rt11_explore_deep_chain() {
    let depth: usize = std::env::var("RT11_DEPTH").map_or(24, |v| v.parse().unwrap());
    let env = Env::empty();
    env.bolt()
        .args(["start", "--preset", "custom"])
        .assert()
        .success();
    let first = main_tip(&env).to_hex();
    env.bolt().args(["goto", &first]).assert().success();
    plant_save(env.root(), |p| {
        let x = p.add_content(b"x").unwrap();
        let two = |kind, id, size| {
            let entries = ["a", "b"]
                .iter()
                .map(|n| Entry {
                    name: (*n).to_owned(),
                    kind,
                    id,
                    size,
                })
                .collect();
            Listing::new(entries).unwrap().encode()
        };
        let mut at = p
            .add_record(ObjectKind::FolderListing, &two(EntryKind::File, x, 1))
            .unwrap();
        for _ in 0..depth {
            at = p
                .add_record(ObjectKind::FolderListing, &two(EntryKind::Folder, at, 0))
                .unwrap();
        }
        at
    });
    let bin = std::env::var("RT11_BOLT")
        .unwrap_or_else(|_| assert_cmd::cargo::cargo_bin("bolt").display().to_string());
    for args in [
        &["verify"][..],
        &["changes", "--save", "main"],
        &["goto", "main"],
        &["tidy"],
    ] {
        let mut cmd = Command::new(&bin);
        cmd.args(args)
            .current_dir(env.root())
            .env("BOLT_CONFIG_DIR", env.config())
            .env("BOLT_USER_NAME", "T")
            .env("BOLT_USER_EMAIL", "t@example.com");
        let ended = run_limited(cmd, 900);
        eprintln!(
            "depth {depth}: bolt {}: code {:?}, timed out {}, {:.1}s, peak {} MiB: {}",
            args.join(" "),
            ended.code,
            ended.timed_out,
            ended.secs,
            ended.peak_mb,
            ended.stderr.lines().last().unwrap_or_default()
        );
    }
}
