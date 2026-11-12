//! Which problem a save names when the walk and a look at a file both find one. The file goes
//! from a thread the walk starts, so the change point is shared by the whole process: this file
//! holds a single test.
#![cfg(feature = "fault-injection")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;
use std::path::Path;

use opal_core::fsio::fault;
use opal_core::repo::Repository;
use opal_core::save::Person;
use opal_core::snapshot::{SaveError, SaveOutcome, SaveRequest, save_folder};

/// Saves a new project that `lay_out` fills, removing `gone` while the files are looked at.
fn save_removing(lay_out: impl FnOnce(&Path), gone: &str) -> Result<SaveOutcome, SaveError> {
    let who = Person {
        name: "Ada".into(),
        email: "ada@example.com".into(),
        time_ms: 0,
        tz_offset_min: 0,
    };
    let dir = tempfile::tempdir().unwrap();
    let mut repo = Repository::init(dir.path(), &who).unwrap();
    lay_out(dir.path());
    let gone = dir.path().join(gone);
    fault::act_at_on_any_thread("walk:looking", move || fs::remove_file(&gone).unwrap());
    let req = SaveRequest {
        message: Some("Two problems"),
        who: &who,
        command: "save",
        accept_crlf: false,
        git_excludes: None,
        new_line: None,
        only: None,
        amend: false,
    };
    save_folder(&mut repo, &req, &mut |_| {}, &mut |_| true)
}

/// An ignore file too large to read in the folder `rel`.
fn too_large_rules(root: &Path, rel: &str) {
    fs::create_dir(root.join(rel)).unwrap();
    fs::write(
        root.join(rel).join(".opalroignore"),
        vec![b'#'; (1 << 20) + 1],
    )
    .unwrap();
}

#[test]
fn the_first_problem_by_path_is_the_one_reported() {
    // The walk hands `a/x` over to be looked at, then meets `b` and its problem at once.
    let saved = save_removing(
        |root| {
            fs::create_dir(root.join("a")).unwrap();
            fs::write(root.join("a/x"), b"x").unwrap();
            too_large_rules(root, "b");
        },
        "a/x",
    );
    match saved {
        Err(SaveError::Unreadable { path, .. }) => assert_eq!(path, "a/x"),
        other => panic!("{:?}", other.map(|_| ())),
    }
    // The walk meets the top's file `z` before the folder `a`, which comes first by path.
    let saved = save_removing(
        |root| {
            fs::write(root.join("z"), b"z").unwrap();
            too_large_rules(root, "a");
        },
        "z",
    );
    match saved {
        Err(SaveError::Ignore { detail }) => {
            assert!(detail.starts_with("a/.opalroignore"), "{detail}")
        }
        other => panic!("{:?}", other.map(|_| ())),
    }
}
