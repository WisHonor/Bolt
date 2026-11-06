//! Test vectors for the storage format (docs/storage-format.md).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use bolt_core::diary::{
    Action, DiaryEntry, Position, SafetyCopy, ServerLines, State, Tag, TagList,
};
use bolt_core::id::{ChangeId, Id};
use bolt_core::link::ProjectLink;
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
    // A name of 600 bytes, past the 255 names had before decision 051: 200 characters of three
    // bytes, which Windows holds as 200 UTF-16 units. Its length is a two-byte varint.
    let long_name = Listing::new(vec![entry(&"語".repeat(200), EntryKind::File, 0x66, 3)]).unwrap();
    let rows: Vec<String> = [
        ("empty-listing", Listing::empty()),
        ("one-file", one),
        ("mixed", mixed),
        ("long-name", long_name),
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

#[test]
#[ignore = "prints the vectors; run by hand to regenerate tests/vectors/project_link.json"]
fn print_project_link_vectors() {
    // A submodule in a SHA-1 repository and one in a SHA-256 repository.
    let rows: Vec<String> = [
        ("sha1-submodule", ProjectLink::submodule(&[0x5a; 20])),
        ("sha256-submodule", ProjectLink::submodule(&[0xa5; 32])),
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
fn project_link_vectors_hold() {
    let text = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/vectors/project_link.json"
    ))
    .unwrap();
    let vectors: Vec<Vector> = serde_json::from_str(&text).unwrap();
    assert!(!vectors.is_empty());
    for v in vectors {
        let l = ProjectLink::decode(&unhex(&v.hex)).unwrap_or_else(|e| panic!("{}: {e}", v.name));
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

fn pack_vectors(file: &str) -> Vec<PackVector> {
    let path = format!("{}/tests/vectors/{file}", env!("CARGO_MANIFEST_DIR"));
    let vectors: Vec<PackVector> =
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    assert!(!vectors.is_empty());
    vectors
}

/// Opens the pack `v` describes and checks that it holds exactly its objects.
fn pack_reads_back(v: &PackVector) {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join(&v.file);
    std::fs::write(&path, unhex(&v.hex)).unwrap();
    let r = PackReader::open(&path).unwrap_or_else(|e| panic!("{}: {e}", v.name));
    let entries: Vec<_> = r.entries().collect::<Result<_, _>>().unwrap();
    assert_eq!(entries.len(), v.objects.len(), "{}", v.name);
    // The reader checks every fingerprint; each object must come back byte for byte.
    let mut found: Vec<(u8, String)> = entries
        .iter()
        .map(|e| {
            assert_eq!(r.find(&e.id).unwrap(), Some(*e), "{}", v.name);
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
        v.objects.iter().map(|o| (o.kind, o.hex.clone())).collect();
    found.sort();
    expected.sort();
    assert_eq!(found, expected, "{}", v.name);
}

#[test]
fn version_1_packs_still_read() {
    for v in pack_vectors("pack.json") {
        assert_eq!(&v.hex[16..24], "01000000", "{}", v.name);
        pack_reads_back(&v);
    }
}

/// `pack`, a version 1 pack, with its index in version 2, built from storage-format.md section 11
/// and not with the writer.
fn as_version_2(pack: &[u8]) -> Vec<u8> {
    let u64_at = |at: usize| u64::from_le_bytes(pack[at..at + 8].try_into().unwrap());
    let trailer = pack.len() - 48;
    let index_at = usize::try_from(u64_at(trailer)).unwrap();
    // (id, kind, offset, uncompressed size, stored size), read from the entries' own headers.
    let mut lines = Vec::new();
    let mut at = 16;
    while at < index_at {
        let id: [u8; 32] = pack[at + 1..at + 33].try_into().unwrap();
        let (size, stored) = (u64_at(at + 33), u64_at(at + 41));
        lines.push((id, pack[at], at as u64, size, stored));
        at += 50 + usize::try_from(stored).unwrap();
    }
    lines.sort();
    let mut index = (lines.len() as u64).to_le_bytes().to_vec();
    for byte in 0..=255u8 {
        let below = lines.iter().filter(|l| l.0[0] <= byte).count();
        index.extend_from_slice(&u32::try_from(below).unwrap().to_le_bytes());
    }
    for l in &lines {
        index.extend_from_slice(&l.0);
    }
    for l in &lines {
        index.push(l.1);
        for n in [l.2, l.3, l.4] {
            index.extend_from_slice(&n.to_le_bytes());
        }
    }
    let mut out = b"BOLTPACK".to_vec();
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&pack[12..index_at]);
    out.extend_from_slice(&index);
    out.extend_from_slice(&(index_at as u64).to_le_bytes());
    out.extend_from_slice(blake3::hash(&index).as_bytes());
    out.extend_from_slice(b"BOLTPEND");
    out
}

#[test]
#[ignore = "prints the vector; run by hand to regenerate tests/vectors/pack_v2.json"]
fn print_pack_v2_vector() {
    let v1 = pack_vectors("pack.json")
        .into_iter()
        .find(|v| v.name == "two-objects")
        .unwrap();
    let pack = as_version_2(&unhex(&v1.hex));
    let index_at = usize::try_from(u64::from_le_bytes(
        pack[pack.len() - 48..pack.len() - 40].try_into().unwrap(),
    ))
    .unwrap();
    let name = blake3::hash(&pack[index_at..pack.len() - 48]).to_hex();
    let objects: Vec<String> = v1
        .objects
        .iter()
        .map(|o| format!("{{\"kind\": {}, \"hex\": \"{}\"}}", o.kind, o.hex))
        .collect();
    println!(
        "[\n  {{\"name\": \"two-objects\", \"file\": \"{name}.pack\", \"hex\": \"{}\", \"objects\": [{}]}}\n]",
        hex(&pack),
        objects.join(", ")
    );
}

#[test]
fn pack_v2_matches_its_vector() {
    for v in pack_vectors("pack_v2.json") {
        // The writer, given the objects in the vector's order, writes exactly its bytes.
        let root = tempfile::tempdir().unwrap();
        let mut w = PackWriter::create(root.path()).unwrap();
        for o in &v.objects {
            let bytes = unhex(&o.hex);
            match ObjectKind::from_u8(o.kind).unwrap() {
                ObjectKind::Content => w.add_content(&bytes).unwrap(),
                kind => w.add_record(kind, &bytes).unwrap(),
            };
        }
        let path = w.finish(root.path()).unwrap().unwrap();
        assert_eq!(path.file_name().unwrap().to_str().unwrap(), v.file);
        assert_eq!(hex(&std::fs::read(&path).unwrap()), v.hex, "{}", v.name);
        pack_reads_back(&v);
    }
}
