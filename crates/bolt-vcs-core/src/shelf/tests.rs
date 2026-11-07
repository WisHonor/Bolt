#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use super::*;
use crate::snapshot::{SaveOutcome, SaveRequest, save_folder};
use std::fs;

fn who() -> Person {
    Person {
        name: "Ada".into(),
        email: "ada@example.com".into(),
        time_ms: 1_700_000_000_000,
        tz_offset_min: 0,
    }
}

/// A project whose folder holds `files`, saved on `main`.
fn saved(files: &[(&str, &[u8])]) -> (tempfile::TempDir, Repository) {
    let dir = tempfile::tempdir().unwrap();
    let mut repo = Repository::init(dir.path(), &who()).unwrap();
    for (path, bytes) in files {
        fs::write(dir.path().join(path), bytes).unwrap();
    }
    let me = who();
    let req = SaveRequest {
        message: Some("saved"),
        who: &me,
        command: "save",
        accept_crlf: false,
        git_excludes: None,
        new_line: None,
        only: None,
        amend: false,
    };
    let done = save_folder(&mut repo, &req, &mut |_| {}, &mut |_| true).unwrap();
    assert!(matches!(done, SaveOutcome::Saved { .. }));
    (dir, repo)
}

#[test]
fn a_new_shelf_takes_the_smallest_number_free() {
    let at = Id([1; 32]);
    let shelves = |labels: &[&str]| -> Vec<(String, Id)> {
        labels.iter().map(|l| ((*l).to_owned(), at)).collect()
    };
    assert_eq!(free_label(&[]), "1");
    assert_eq!(free_label(&shelves(&["2", "1", "4"])), "3");
    // A label Bolt didn't give takes no number.
    assert_eq!(free_label(&shelves(&["1", "x", "01"])), "2");
}

#[test]
fn a_shelf_past_the_most_a_project_keeps_is_refused_before_anything_changes() {
    let (dir, mut repo) = saved(&[("a.txt", b"a")]);
    {
        let lock = repo.lock_for_change("test", &mut |_| {}).unwrap();
        let (head, mut entry) = repo.current().unwrap();
        let save = tip_of(&entry).unwrap();
        entry.parent = Some(head);
        entry.state.shelves = (1..=MAX_SHELVES).map(|n| (n.to_string(), save)).collect();
        let packs = repo.new_packs().unwrap();
        repo.commit(&lock, packs, &entry).unwrap();
    }
    fs::write(dir.path().join("a.txt"), b"mine").unwrap();
    let (head, packs) = (repo.head().unwrap(), repo.store().pack_count());
    let refused = shelf(&mut repo, None, &who(), None, &mut |_| {});
    let Err(e) = refused else {
        panic!("a shelf past the limit was made: {refused:?}");
    };
    assert!(matches!(e, SwitchError::TooManyShelves), "{e:?}");
    assert!(e.to_string().contains("more than 10,000 shelves"), "{e}");
    assert!(e.hint().unwrap().contains("bolt unshelf"), "{e:?}");
    assert_eq!(
        (repo.head().unwrap(), repo.store().pack_count()),
        (head, packs)
    );
    assert_eq!(fs::read(dir.path().join("a.txt")).unwrap(), b"mine");
}

#[cfg(feature = "fault-injection")]
#[test]
fn files_that_change_before_they_are_kept_stay_and_nothing_is_shelved() {
    use crate::fsio::fault;
    let (dir, mut repo) = saved(&[("a.txt", b"a")]);
    fs::write(dir.path().join("a.txt"), b"mine").unwrap();
    let at = dir.path().join("a.txt");
    fault::act_at("shelf:planned", move || fs::write(at, b"edited").unwrap());
    let (head, packs) = (repo.head().unwrap(), repo.store().pack_count());
    let done = shelf(&mut repo, None, &who(), None, &mut |_| {}).unwrap();
    let nothing = Shelved::Nothing {
        changed_again: true,
        links_left: vec![],
    };
    assert_eq!(done, nothing);
    assert_eq!(
        (repo.head().unwrap(), repo.store().pack_count()),
        (head, packs)
    );
    assert_eq!(fs::read(dir.path().join("a.txt")).unwrap(), b"edited");
    let told = repo.store().warnings();
    let changed: Vec<&Warning> = told
        .iter()
        .filter(|w| w.kind == warning::CHANGED_SINCE_PLAN)
        .collect();
    assert_eq!(changed.len(), 1, "{told:?}");
    assert!(changed[0].message.contains("a.txt"), "{told:?}");
}
