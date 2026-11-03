//! An import stopped partway, as by a crash or Ctrl+C, then run again. The test-only pack limit
//! applies to every command in its process, so this file holds a single test.
#![cfg(all(feature = "fault-injection", feature = "test-support"))]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::Path;

use bolt_core::diary::State;
use bolt_core::fsio::fault;
use bolt_core::git::craft::Craft;
use bolt_core::git::{GitId, HashKind, ObjectType};
use bolt_core::import::{ImportError, ImportReport, ImportRequest, import_git};
use bolt_core::pack::set_test_pack_limit;
use bolt_core::repo::Repository;
use bolt_core::save::Person;
use bolt_core::verify::verify_store;

/// A Git repository of three commits on `main`, each with a file and a folder of its own, an
/// annotated tag of the first and a server branch at the second, so the import stores a tag list
/// and server lines as well.
fn repository(dir: &Path) {
    let git = Craft::new(dir, HashKind::Sha1);
    let mut commits: Vec<GitId> = Vec::new();
    for i in 0..3u8 {
        let blob = git.object(ObjectType::Blob, &[b'0' + i]);
        let inner = git.tree(&[(b"100644", b"b.txt", blob)]);
        let tree = git.tree(&[(b"100644", b"a.txt", blob), (b"40000", b"sub", inner)]);
        let mut raw = format!("tree {}\n", tree.to_hex());
        if let Some(parent) = commits.last() {
            raw += &format!("parent {}\n", parent.to_hex());
        }
        raw += &format!(
            "author A <a@example.com> {i} +0000\ncommitter A <a@example.com> {i} +0000\n\nSave {i}\n"
        );
        commits.push(git.commit(raw.as_bytes()));
    }
    let tag = format!(
        "object {}\ntype commit\ntag v1\ntagger A <a@example.com> 3 +0000\n\nVersion 1\n",
        commits[0].to_hex()
    );
    let tag = git.object(ObjectType::Tag, tag.as_bytes());
    for (name, id) in [
        ("refs/heads/main", commits[2]),
        ("refs/remotes/origin/main", commits[1]),
        ("refs/tags/v1", tag),
    ] {
        git.loose_ref(name, &format!("{}\n", id.to_hex()));
    }
}

fn import(dir: &Path) -> Result<ImportReport, ImportError> {
    let who = Person {
        name: "Ada".into(),
        email: "ada@example.com".into(),
        time_ms: 0,
        tz_offset_min: 0,
    };
    import_git(
        dir,
        &mut ImportRequest {
            who: &who,
            command: "import",
            progress: &mut |_, _| {},
        },
        &mut None,
    )
}

fn state(dir: &Path) -> State {
    Repository::open(dir).unwrap().current().unwrap().1.state
}

#[test]
fn an_interrupted_import_can_be_run_again() {
    // Three objects a pack, so the import needs several.
    set_test_pack_limit(Some(3));
    let clean = tempfile::tempdir().unwrap();
    repository(clean.path());
    import(clean.path()).unwrap();
    let records = state(clean.path());
    assert!(records.tags.is_some() && records.server_lines.is_some());
    let mut between_packs = false;
    for n in 1.. {
        assert!(n < 100, "the import never finished");
        let dir = tempfile::tempdir().unwrap();
        repository(dir.path());
        fault::arm(n);
        let result = import(dir.path());
        fault::disarm();
        let Err(stopped) = result else {
            break;
        };
        assert!(!dir.path().join(".bolt/format").exists(), "fault {n}");
        between_packs |= stopped.to_string().ends_with("packs:rolled-over");
        // Running it again finishes the job, with the saves a clean import gives.
        import(dir.path()).unwrap();
        assert_eq!(state(dir.path()), state(clean.path()), "fault {n}");
        let report = verify_store(&dir.path().join(".bolt/store/packs"));
        assert!(report.is_ok(), "fault {n}: {:?}", report.problems);
    }
    set_test_pack_limit(None);
    assert!(between_packs, "no fault point sat between two packs");
}
