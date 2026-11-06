//! A folder swapped for another between its listing and the look at its files. The change point
//! is reached on a thread the walk starts, so it is shared by the whole process: this file holds
//! a single test.
#![cfg(feature = "fault-injection")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;

use bolt_core::fsio::fault;
use bolt_core::repo::Repository;
use bolt_core::save::Person;
use bolt_core::snapshot::{SaveError, SaveOutcome, SaveRequest, save_folder};

#[test]
fn a_folder_swapped_before_its_files_are_looked_at_is_refused() {
    let who = Person {
        name: "Ada".into(),
        email: "ada@example.com".into(),
        time_ms: 0,
        tz_offset_min: 0,
    };
    let dir = tempfile::tempdir().unwrap();
    let mut repo = Repository::init(dir.path(), &who).unwrap();
    let sub = dir.path().join("sub");
    fs::create_dir(&sub).unwrap();
    fs::write(sub.join("a.txt"), b"old").unwrap();
    // Another real folder holding a file of the same name, on the same drive.
    let elsewhere = tempfile::tempdir_in(dir.path().parent().unwrap()).unwrap();
    let other = elsewhere.path().join("other");
    fs::create_dir(&other).unwrap();
    fs::write(other.join("a.txt"), b"new!").unwrap();
    let gone = elsewhere.path().join("gone");
    let head = repo.head().unwrap();
    let swap = sub.clone();
    fault::act_at_on_any_thread("walk:looking", move || {
        fs::rename(&swap, &gone).unwrap();
        fs::rename(&other, &swap).unwrap();
    });
    let req = SaveRequest {
        message: "Swapped",
        who: &who,
        command: "save",
        accept_crlf: false,
        git_excludes: None,
        new_line: None,
    };
    match save_folder(&mut repo, &req, &mut |_| {}) {
        Err(SaveError::ChangedDuringSave { path }) => assert_eq!(path, "sub/a.txt"),
        Ok(SaveOutcome::Saved { save, .. }) => {
            let listing = repo.store().listing(&save.root).unwrap();
            panic!("saved a mix of two folders: {:?}", listing.entries());
        }
        other => panic!("{other:?}"),
    }
    // The swap happened, and nothing was saved.
    assert_eq!(fs::read(sub.join("a.txt")).unwrap(), b"new!");
    assert_eq!(repo.head().unwrap(), head);
}
