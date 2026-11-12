//! Red-team proof tests for milestone 6, presets and polish, second session (decision 024): each
//! asserts the safe behaviour and fails on the code the session attacked. See
//! docs/redteam/2026-11-10-milestone-6-session-2.md.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

/// A planted save whose `listings` folders each hold `per` files with names of 765 bytes, every
/// one naming the 1-byte content `x` with the size `size`.
#[cfg(target_os = "linux")]
fn wide_listings(listings: usize, per: usize, size: u64) -> support::Env {
    use opal_core::listing::{Entry, EntryKind, Listing};
    use opal_core::pack::ObjectKind;
    let env = support::Env::empty();
    env.opal()
        .args(["start", "--preset", "custom"])
        .assert()
        .success();
    let first = support::main_tip(&env).to_hex();
    env.opal().args(["goto", &first]).assert().success();
    support::plant_save(env.root(), |p| {
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

/// Every file of a planted save given a wrong size: `verify` keeps each one's name, in every look,
/// though it reports at most 1,000 problems. The same save with right sizes, and `tidy` on this
/// one, read the same listings well within the cap; `verify` must report the problems within it
/// too, not run out of memory.
#[cfg(target_os = "linux")]
#[test]
fn rt11_1_verify_reports_many_wrong_sizes_within_bounded_memory() {
    const CAP_KB: u32 = 1_000_000;
    let (listings, per) = (12, 50_000);
    let run = |env: &support::Env, cmd: &str| {
        let capped = support::opal_capped_at(env.config(), env.root(), CAP_KB, &[cmd]);
        let ended = support::run_limited(capped, 1_800);
        eprintln!(
            "opal {cmd}, address space capped at {CAP_KB} KB: code {:?} in {:.0}s, peak {} MiB: {}",
            ended.code,
            ended.secs,
            ended.peak_mb,
            ended.stderr.lines().last().unwrap_or_default()
        );
        ended
    };
    let right = wide_listings(listings, per, 1);
    assert_eq!(run(&right, "verify").code, Some(0), "the cap is too small");
    drop(right);
    let wrong = wide_listings(listings, per, 2);
    let verify = run(&wrong, "verify");
    assert_eq!(run(&wrong, "tidy").code, Some(0), "the cap is too small");
    assert_eq!(
        verify.code,
        Some(1),
        "verify should report the wrong sizes, not run out of memory"
    );
}
