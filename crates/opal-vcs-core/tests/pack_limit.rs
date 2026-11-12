//! The test-only pack limit applies to every command in its process, so this file holds a single
//! test: any other test running beside it would have its packs split too.
#![cfg(feature = "fault-injection")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use opal_core::pack::set_test_pack_limit;
use opal_core::repo::Repository;
use opal_core::save::Person;

fn packs_in(root: &std::path::Path) -> usize {
    std::fs::read_dir(root.join(".opal/store/packs"))
        .unwrap()
        .count()
}

#[test]
fn the_test_limit_splits_a_commands_packs_until_it_is_cleared() {
    let who = Person {
        name: "Ada".into(),
        email: "ada@example.com".into(),
        time_ms: 0,
        tz_offset_min: 0,
    };
    // `start` stores a listing, a save and its entry: three objects.
    let split = tempfile::tempdir().unwrap();
    set_test_pack_limit(Some(2));
    Repository::init(split.path(), &who).unwrap();
    set_test_pack_limit(None);
    let whole = tempfile::tempdir().unwrap();
    Repository::init(whole.path(), &who).unwrap();
    assert_eq!(packs_in(split.path()), 2);
    assert_eq!(packs_in(whole.path()), 1);
}
