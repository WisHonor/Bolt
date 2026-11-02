//! Test vectors for the storage format (docs/storage-format.md).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use bolt_core::diary::{
    Action, DiaryEntry, Position, SafetyCopy, ServerLines, State, Tag, TagList,
};
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
    // Raw-byte order ("B" 0x42 < "a" 0x61 < "é" 0xc3a9), every kind a save writes,
    // a two-byte size (128 = 80 01) and an empty file.
    let entry = |name: &str, kind, fill, size| Entry {
        name: name.into(),
        kind,
        id: Id([fill; 32]),
        size,
    };
    let mixed = Listing::new(vec![
        entry("B", EntryKind::Folder, 0x44, 0),
        entry("a", EntryKind::File, 0x11, 128),
        entry("b", EntryKind::Executable, 0x22, 7),
        entry("l", EntryKind::Symlink, 0x33, 1),
        entry("é", EntryKind::File, 0x55, 0),
    ])
    .unwrap();
    let rows: Vec<String> = [
        ("empty-listing", Listing::empty()),
        ("one-file", one),
        ("mixed", mixed),
    ]
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
    // Non-zero, asymmetric bytes so byte order and the sign of time-zone offsets are pinned.
    let mut realistic = minimal.clone();
    realistic.root = Id([0xab; 32]);
    realistic.parents = vec![Id([2; 32]), Id([1; 32])];
    realistic.author = Person {
        name: "Zoé".into(),
        email: "zoe@example.com".into(),
        time_ms: 1_793_520_000_123,
        tz_offset_min: -300,
    };
    realistic.saver.time_ms = 1_793_523_600_000;
    realistic.saver.tz_offset_min = 60;
    realistic.message = "Fix café menu ✓".into();
    realistic.change_id = ChangeId([
        0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, 0xfe, 0xdc, 0xba, 0x98, 0x76, 0x54, 0x32,
        0x10,
    ]);
    realistic.extra.insert("line".into(), b"main".to_vec());
    vec![
        ("minimal", minimal),
        ("with-parent-line-and-move", rich),
        ("realistic", realistic),
    ]
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
            tags: None,
            position: Position::OnLine("main".into()),
            shelves: vec![],
            server_lines: None,
        },
        safety_copies: vec![],
    }
}

/// One lightweight tag and one annotated tag that keeps its Git tag object.
fn vector_tag_list() -> TagList {
    let git_tag = "object 2222222222222222222222222222222222222222\n\
                   type commit\n\
                   tag v2\n\
                   tagger Zoé <zoe@example.com> 1793520000 -0500\n\
                   \n\
                   Second release ✓\n";
    TagList {
        tags: BTreeMap::from([
            (
                "v1".to_owned(),
                Tag {
                    save: Id([0x07; 32]),
                    message: String::new(),
                    git_tags: vec![],
                },
            ),
            (
                "v2".to_owned(),
                Tag {
                    save: Id([0x08; 32]),
                    message: "Second release ✓".into(),
                    git_tags: vec![git_tag.as_bytes().to_vec()],
                },
            ),
        ]),
    }
}

fn vector_server_lines() -> ServerLines {
    ServerLines {
        servers: BTreeMap::from([(
            "origin".to_owned(),
            BTreeMap::from([
                ("feature/menu".to_owned(), Id([0x1f; 32])),
                ("main".to_owned(), Id([0x1c; 32])),
            ]),
        )]),
    }
}

/// Every optional part of a diary entry, with distinct fill bytes so each id is easy to spot.
fn full_diary_entry() -> DiaryEntry {
    DiaryEntry {
        parent: Some(Id([0x0d; 32])),
        action: Action::Undo(Id([0x0e; 32])),
        command: "undo".into(),
        who: Person {
            name: "Zoé".into(),
            email: "zoe@example.com".into(),
            time_ms: 1_793_520_000_123,
            tz_offset_min: -300,
        },
        bolt_version: "0.1.0".into(),
        state: State {
            lines: BTreeMap::from([
                ("feature/menu".to_owned(), Id([0x0f; 32])),
                ("main".to_owned(), Id([0x0a; 32])),
            ]),
            // Pointing at the two lists of the other vectors.
            tags: Some(vector_tag_list().id()),
            position: Position::AtSave(Id([0x5a; 32])),
            // Newest first, deliberately not in name order.
            shelves: vec![
                ("try-b".into(), Id([0xb2; 32])),
                ("try-a".into(), Id([0xa1; 32])),
            ],
            server_lines: Some(vector_server_lines().id()),
        },
        safety_copies: vec![
            SafetyCopy {
                path: "Assets/é.png".into(),
                kind: EntryKind::File,
                id: Id([0xe1; 32]),
                size: 1000,
            },
            SafetyCopy {
                path: "run.sh".into(),
                kind: EntryKind::Executable,
                id: Id([0xe2; 32]),
                size: 12,
            },
        ],
    }
}

#[test]
#[ignore = "prints the vectors; run by hand to regenerate tests/vectors/diary.json"]
fn print_diary_vectors() {
    let rows: Vec<String> = [
        ("minimal", minimal_diary_entry()),
        ("full", full_diary_entry()),
    ]
    .iter()
    .map(|(n, e)| {
        format!(
            "  {{\"name\": \"{n}\", \"hex\": \"{}\", \"id\": \"{}\"}}",
            hex(&e.encode()),
            e.id()
        )
    })
    .collect();
    println!("[\n{}\n]", rows.join(",\n"));
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

#[test]
#[ignore = "prints the vectors; run by hand to regenerate tests/vectors/tag_list.json"]
fn print_tag_list_vectors() {
    let list = vector_tag_list();
    println!(
        "[\n  {{\"name\": \"lightweight-and-annotated\", \"hex\": \"{}\", \"id\": \"{}\"}}\n]",
        hex(&list.encode()),
        list.id()
    );
}

#[test]
fn tag_list_vectors_hold() {
    let text = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/vectors/tag_list.json"
    ))
    .unwrap();
    let vectors: Vec<Vector> = serde_json::from_str(&text).unwrap();
    assert!(!vectors.is_empty());
    for v in vectors {
        let l = TagList::decode(&unhex(&v.hex)).unwrap_or_else(|e| panic!("{}: {e}", v.name));
        assert_eq!(hex(&l.encode()), v.hex, "{}", v.name);
        assert_eq!(l.id().to_hex(), v.id, "{}", v.name);
    }
}

#[test]
#[ignore = "prints the vectors; run by hand to regenerate tests/vectors/server_lines.json"]
fn print_server_lines_vectors() {
    let list = vector_server_lines();
    println!(
        "[\n  {{\"name\": \"one-server-two-lines\", \"hex\": \"{}\", \"id\": \"{}\"}}\n]",
        hex(&list.encode()),
        list.id()
    );
}

#[test]
fn server_lines_vectors_hold() {
    let text = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/vectors/server_lines.json"
    ))
    .unwrap();
    let vectors: Vec<Vector> = serde_json::from_str(&text).unwrap();
    assert!(!vectors.is_empty());
    for v in vectors {
        let l = ServerLines::decode(&unhex(&v.hex)).unwrap_or_else(|e| panic!("{}: {e}", v.name));
        assert_eq!(hex(&l.encode()), v.hex, "{}", v.name);
        assert_eq!(l.id().to_hex(), v.id, "{}", v.name);
    }
}

#[derive(Deserialize)]
struct PackVector {
    name: String,
    file: String,
    hex: String,
    /// Every object the pack holds: its kind number and its uncompressed bytes.
    objects: Vec<PackObject>,
}

#[derive(Deserialize)]
struct PackObject {
    kind: u8,
    hex: String,
}

/// A pack's objects: kind and uncompressed bytes, in the order they are added.
type PackObjects = Vec<(ObjectKind, Vec<u8>)>;

fn pack_vector_inputs() -> Vec<(&'static str, PackObjects)> {
    // Repetitive text, so the writer stores it with zstd.
    let content = "Hello, Bolt! ".repeat(40).into_bytes();
    let listing = Listing::new(vec![Entry {
        name: "hello.txt".into(),
        kind: EntryKind::File,
        id: Id(*blake3::hash(&content).as_bytes()),
        size: content.len() as u64,
    }])
    .unwrap();
    vec![
        (
            "empty-listing",
            vec![(ObjectKind::FolderListing, Listing::empty().encode())],
        ),
        (
            "two-objects",
            // Written listing first (id 90b1…), so the index must reorder: content (437d…) first.
            vec![
                (ObjectKind::FolderListing, listing.encode()),
                (ObjectKind::Content, content),
            ],
        ),
    ]
}

#[test]
#[ignore = "prints the vectors; run by hand to regenerate tests/vectors/pack.json"]
fn print_pack_vectors() {
    let mut rows = Vec::new();
    for (name, objects) in pack_vector_inputs() {
        let root = tempfile::tempdir().unwrap();
        let mut w = PackWriter::create(root.path()).unwrap();
        for (kind, bytes) in &objects {
            match kind {
                ObjectKind::Content => w.add_content(bytes).unwrap(),
                _ => w.add_record(*kind, bytes).unwrap(),
            };
        }
        let path = w.finish(root.path()).unwrap().unwrap();
        let objects: Vec<String> = objects
            .iter()
            .map(|(k, b)| format!("{{\"kind\": {}, \"hex\": \"{}\"}}", *k as u8, hex(b)))
            .collect();
        rows.push(format!(
            "  {{\"name\": \"{name}\", \"file\": \"{}\", \"hex\": \"{}\", \"objects\": [{}]}}",
            path.file_name().unwrap().to_str().unwrap(),
            hex(&std::fs::read(&path).unwrap()),
            objects.join(", ")
        ));
    }
    println!("[\n{}\n]", rows.join(",\n"));
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
        assert_eq!(r.entries().len(), v.objects.len(), "{}", v.name);
        // The reader checks every fingerprint; each object must come back byte for byte.
        let mut found: Vec<(u8, String)> = r
            .entries()
            .iter()
            .map(|e| {
                let bytes = if e.kind == ObjectKind::Content {
                    let mut out = Vec::new();
                    r.copy_content(e, &mut out).unwrap();
                    out
                } else {
                    r.read_record(e).unwrap()
                };
                (e.kind as u8, hex(&bytes))
            })
            .collect();
        let mut expected: Vec<(u8, String)> =
            v.objects.into_iter().map(|o| (o.kind, o.hex)).collect();
        found.sort();
        expected.sort();
        assert_eq!(found, expected, "{}", v.name);
    }
}
