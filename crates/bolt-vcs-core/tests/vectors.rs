//! Test vectors for the storage format (docs/storage-format.md).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use bolt_core::diary::{Action, DiaryEntry, Position, State};
use bolt_core::id::{ChangeId, Id};
use bolt_core::listing::{Entry, EntryKind, Listing};
use bolt_core::pack::{ObjectKind, PackReader, PackWriter};
use bolt_core::save::{MoveNote, Person, Save};
use serde::Deserialize;
use std::collections::BTreeMap;

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).expect("vector hex"))
        .collect()
}

#[derive(Deserialize)]
struct Vector {
    name: String,
    hex: String,
    id: String,
}

#[test]
#[ignore = "prints the vectors; run by hand to regenerate tests/vectors/listing.json"]
fn print_listing_vectors() {
    let one = Listing::new(vec![Entry {
        name: "a".into(),
        kind: EntryKind::File,
        id: Id([0; 32]),
        size: 5,
    }])
    .unwrap();
    let rows: Vec<String> = [("empty-listing", Listing::empty()), ("one-file", one)]
        .iter()
        .map(|(n, l)| {
            format!(
                "  {{\"name\": \"{n}\", \"hex\": \"{}\", \"id\": \"{}\"}}",
                hex(&l.encode()),
                l.id()
            )
        })
        .collect();
    println!("[\n{}\n]", rows.join(",\n"));
}

#[test]
fn listing_vectors_hold() {
    let text = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/vectors/listing.json"
    ))
    .unwrap();
    let vectors: Vec<Vector> = serde_json::from_str(&text).unwrap();
    assert!(!vectors.is_empty());
    for v in vectors {
        let l = Listing::decode(&unhex(&v.hex)).unwrap_or_else(|e| panic!("{}: {e}", v.name));
        assert_eq!(hex(&l.encode()), v.hex, "{}", v.name);
        assert_eq!(l.id().to_hex(), v.id, "{}", v.name);
    }
}

fn person() -> Person {
    Person {
        name: "A".into(),
        email: "a@b".into(),
        time_ms: 0,
        tz_offset_min: 0,
    }
}

fn save_vector_inputs() -> Vec<(&'static str, Save)> {
    let minimal = Save {
        root: Id([0; 32]),
        parents: vec![],
        author: person(),
        saver: person(),
        message: "m".into(),
        change_id: ChangeId([0; 16]),
        move_notes: vec![],
        signature: vec![],
        extra: BTreeMap::new(),
    };
    let mut rich = minimal.clone();
    rich.parents = vec![Id([1; 32])];
    rich.extra.insert("line".into(), b"main".to_vec());
    rich.move_notes = vec![MoveNote {
        from: "a.txt".into(),
        to: "b.txt".into(),
    }];
    vec![("minimal", minimal), ("with-parent-line-and-move", rich)]
}

#[test]
#[ignore = "prints the vectors; run by hand to regenerate tests/vectors/save.json"]
fn print_save_vectors() {
    let rows: Vec<String> = save_vector_inputs()
        .iter()
        .map(|(n, s)| {
            format!(
                "  {{\"name\": \"{n}\", \"hex\": \"{}\", \"id\": \"{}\"}}",
                hex(&s.encode()),
                s.id()
            )
        })
        .collect();
    println!(
        "[
{}
]",
        rows.join(
            ",
"
        )
    );
}

#[test]
fn save_vectors_hold() {
    let text = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/vectors/save.json"
    ))
    .unwrap();
    let vectors: Vec<Vector> = serde_json::from_str(&text).unwrap();
    assert!(!vectors.is_empty());
    for v in vectors {
        let s = Save::decode(&unhex(&v.hex), false).unwrap_or_else(|e| panic!("{}: {e}", v.name));
        assert_eq!(hex(&s.encode()), v.hex, "{}", v.name);
        assert_eq!(s.id().to_hex(), v.id, "{}", v.name);
    }
}

fn minimal_diary_entry() -> DiaryEntry {
    DiaryEntry {
        parent: None,
        action: Action::Command,
        command: "start".into(),
        who: person(),
        bolt_version: "0.1.0".into(),
        state: State {
            lines: BTreeMap::from([("main".to_owned(), Id([0; 32]))]),
            tags: BTreeMap::new(),
            position: Position::OnLine("main".into()),
            shelves: vec![],
            remotes: BTreeMap::new(),
        },
        safety_copies: vec![],
    }
}

#[test]
#[ignore = "prints the vector; run by hand to regenerate tests/vectors/diary.json"]
fn print_diary_vectors() {
    let e = minimal_diary_entry();
    println!(
        "[\n  {{\"name\": \"minimal\", \"hex\": \"{}\", \"id\": \"{}\"}}\n]",
        hex(&e.encode()),
        e.id()
    );
}

#[test]
fn diary_vectors_hold() {
    let text = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/vectors/diary.json"
    ))
    .unwrap();
    let vectors: Vec<Vector> = serde_json::from_str(&text).unwrap();
    assert!(!vectors.is_empty());
    for v in vectors {
        let e =
            DiaryEntry::decode(&unhex(&v.hex)).unwrap_or_else(|err| panic!("{}: {err}", v.name));
        assert_eq!(hex(&e.encode()), v.hex, "{}", v.name);
        assert_eq!(e.id().to_hex(), v.id, "{}", v.name);
    }
}

#[derive(Deserialize)]
struct PackVector {
    name: String,
    file: String,
    hex: String,
}

#[test]
#[ignore = "prints the vector; run by hand to regenerate tests/vectors/pack.json"]
fn print_pack_vector() {
    let root = tempfile::tempdir().unwrap();
    let mut w = PackWriter::create(root.path()).unwrap();
    w.add_record(ObjectKind::FolderListing, &Listing::empty().encode())
        .unwrap();
    let path = w.finish(root.path()).unwrap().unwrap();
    println!(
        "[\n  {{\"name\": \"empty-listing\", \"file\": \"{}\", \"hex\": \"{}\"}}\n]",
        path.file_name().unwrap().to_str().unwrap(),
        hex(&std::fs::read(&path).unwrap())
    );
}

#[test]
fn pack_vector_holds() {
    let text = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/vectors/pack.json"
    ))
    .unwrap();
    let vectors: Vec<PackVector> = serde_json::from_str(&text).unwrap();
    assert!(!vectors.is_empty());
    let root = tempfile::tempdir().unwrap();
    for v in vectors {
        let path = root.path().join(&v.file);
        std::fs::write(&path, unhex(&v.hex)).unwrap();
        let r = PackReader::open(&path).unwrap_or_else(|e| panic!("{}: {e}", v.name));
        assert_eq!(r.entries().len(), 1, "{}", v.name);
        let e = r.entries()[0];
        assert_eq!(e.kind, ObjectKind::FolderListing, "{}", v.name);
        assert_eq!(e.id, Listing::empty().id(), "{}", v.name);
        assert_eq!(
            r.read_record(&e).unwrap(),
            Listing::empty().encode(),
            "{}",
            v.name
        );
    }
}
