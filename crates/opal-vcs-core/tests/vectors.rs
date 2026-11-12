//! Test vectors for the storage format (docs/storage-format.md).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use opal_core::chunks::{Chunk, ChunkList, Cutter};
use opal_core::diary::{
    Action, DiaryEntry, FolderChange, Position, ServerLines, State, Tag, TagList,
};
use opal_core::id::{ChangeId, Id, hash_content};
use opal_core::link::ProjectLink;
use opal_core::listing::{Entry, EntryKind, Listing};
use opal_core::pack::{ObjectKind, PackReader, PackWriter};
use opal_core::presets::{self, Engine, Found, Language, Preset};
use opal_core::rules::{self, Rule, Rules};
use opal_core::save::{MoveNote, Person, Save};
use opal_core::store::Store;
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
        opal_version: "0.1.0".into(),
        state: State {
            lines: BTreeMap::from([("main".to_owned(), Id([0; 32]))]),
            tags: None,
            position: Position::OnLine("main".into()),
            shelves: vec![],
            server_lines: None,
        },
        folder_change: None,
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
        opal_version: "0.1.0".into(),
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
        folder_change: Some(FolderChange {
            from: Id([0x5a; 32]),
            to: Id([0x6b; 32]),
        }),
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
fn an_entry_without_a_folder_change_has_milestone_1s_bytes() {
    let text = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/vectors/diary.json"
    ))
    .unwrap();
    let vectors: Vec<Vector> = serde_json::from_str(&text).unwrap();
    let minimal = vectors.iter().find(|v| v.name == "minimal").unwrap();
    let e = minimal_diary_entry();
    assert_eq!(hex(&e.encode()), minimal.hex);
    // The ID milestone 1 gave it, which pins every byte.
    let milestone_1 = "616fa137a45ee91eeed7711ed61dec1fa79ce8d2d96011d9eb87e090170ec07c";
    assert_eq!(e.id().to_hex(), milestone_1);
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

/// `pack`, a version 2 pack, with its index in version 1, built from storage-format.md section 11,
/// since the writer no longer writes version 1.
fn as_version_1(pack: &[u8]) -> Vec<u8> {
    let u64_at = |at: usize| u64::from_le_bytes(pack[at..at + 8].try_into().unwrap());
    let index_at = usize::try_from(u64_at(pack.len() - 48)).unwrap();
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
    let mut index = opal_core::encoding::Writer::new();
    index.varint(lines.len() as u64);
    for l in &lines {
        index.id(&Id(l.0));
        index.u8(l.1);
        index.u64(l.2);
        index.varint(l.3);
        index.varint(l.4);
    }
    let index = index.into_bytes();
    let mut out = b"OPALPACK".to_vec();
    out.extend_from_slice(&1u32.to_le_bytes());
    out.extend_from_slice(&pack[12..index_at]);
    out.extend_from_slice(&index);
    out.extend_from_slice(&(index_at as u64).to_le_bytes());
    out.extend_from_slice(blake3::hash(&index).as_bytes());
    out.extend_from_slice(b"OPALPEND");
    out
}

#[test]
#[ignore = "prints the vectors; run by hand to regenerate tests/vectors/pack.json"]
fn print_pack_vectors() {
    let rows: Vec<String> = pack_vectors("pack.json")
        .iter()
        .map(|v| {
            // What the writer writes of the vector's objects, in their order, in version 1.
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
            let pack = as_version_1(&std::fs::read(path).unwrap());
            let name = hex(&pack[pack.len() - 40..pack.len() - 8]);
            let objects: Vec<String> = v
                .objects
                .iter()
                .map(|o| format!("{{\"kind\": {}, \"hex\": \"{}\"}}", o.kind, o.hex))
                .collect();
            format!(
                "  {{\"name\": \"{}\", \"file\": \"{name}.pack\", \"hex\": \"{}\", \"objects\": [{}]}}",
                v.name,
                hex(&pack),
                objects.join(", ")
            )
        })
        .collect();
    println!("[\n{}\n]", rows.join(",\n"));
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
    let mut out = b"OPALPACK".to_vec();
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&pack[12..index_at]);
    out.extend_from_slice(&index);
    out.extend_from_slice(&(index_at as u64).to_le_bytes());
    out.extend_from_slice(blake3::hash(&index).as_bytes());
    out.extend_from_slice(b"OPALPEND");
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

#[derive(Deserialize)]
struct ChunkedPackVector {
    name: String,
    file: String,
    hex: String,
}

#[test]
fn the_writer_writes_the_chunked_pack_vector_byte_for_byte() {
    let path = format!(
        "{}/tests/vectors/pack_chunked.json",
        env!("CARGO_MANIFEST_DIR")
    );
    let vectors: Vec<ChunkedPackVector> =
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    assert_eq!(vectors.len(), 1);
    for v in &vectors {
        // `hello world` cut into `hello ` and `world`, both stored raw, then its chunk list.
        let root = tempfile::tempdir().unwrap();
        let mut w = PackWriter::create(root.path()).unwrap();
        let chunks = ["hello ", "world"]
            .iter()
            .map(|piece| Chunk {
                id: w.add_content(piece.as_bytes()).unwrap(),
                size: piece.len() as u64,
            })
            .collect();
        let list = ChunkList {
            file: hash_content(b"hello world"),
            size: 11,
            chunks,
        };
        w.add_chunk_list(&list).unwrap();
        let path = w.finish(root.path()).unwrap().unwrap();
        assert_eq!(path.file_name().unwrap().to_str().unwrap(), v.file);
        assert_eq!(hex(&std::fs::read(&path).unwrap()), v.hex, "{}", v.name);

        // Read back: the list under the file's ID, and the file through it.
        let r = PackReader::open(&path).unwrap();
        let entry = r.find(&list.file).unwrap().unwrap();
        assert_eq!(entry.kind, ObjectKind::ChunkList);
        assert_eq!(r.read_chunk_list(&entry).unwrap(), list);
        let mut out = Vec::new();
        Store::open(root.path())
            .unwrap()
            .copy_content(&list.file, &mut out)
            .unwrap();
        assert_eq!(out, b"hello world");
    }
}

#[derive(Deserialize)]
struct SealedVector {
    name: String,
    key: String,
    body: String,
    sealed: String,
}

/// Version 1, which Opalro no longer reads: `main` at save 11…11 with change IDs 22…22 and 33…33,
/// and `side` at 44…44 with 22…22.
fn change_ids_v1_body() -> String {
    let id = |byte: &str, n: usize| byte.repeat(n);
    format!(
        "4f50414c43484944010204{}{}02{}{}04{}{}01{}",
        hex(b"main"),
        id("11", 32),
        id("22", 16),
        id("33", 16),
        hex(b"side"),
        id("44", 32),
        id("22", 16)
    )
}

/// The project at save 33…33 (change ID 44…44), `main` at 11…11 (22…22) and then 33…33, and
/// `side` at 55…55 (22…22), as storage-format.md §10 lays them out.
fn change_ids_body() -> String {
    let id = |byte: &str, n: usize| byte.repeat(n);
    format!(
        "4f50414c4348494402030001{}{}04{}02{}{}{}{}04{}01{}{}",
        id("33", 32),
        id("44", 16),
        hex(b"main"),
        id("11", 32),
        id("22", 16),
        id("33", 32),
        id("44", 16),
        hex(b"side"),
        id("55", 32),
        id("22", 16)
    )
}

#[test]
#[ignore = "prints the vectors; run by hand to regenerate tests/vectors/change_ids.json"]
fn print_change_ids_vector() {
    let key = [0x42; 32];
    let line = |name: &str, body: &str| {
        format!(
            "  {{\"name\": \"{name}\", \"key\": \"{}\", \"body\": \"{body}\", \"sealed\": \"{}\"}}",
            hex(&key),
            hex(&opal_core::keyed::seal(&key, &unhex(body)))
        )
    };
    println!(
        "[\n{},\n{}\n]",
        line("two-lines", &change_ids_v1_body()),
        line("version-2", &change_ids_body())
    );
}

#[test]
fn change_ids_match_their_vector() {
    use opal_core::changeids::ChangeIds;
    use opal_core::keyed::{open_sealed, seal};
    let path = format!(
        "{}/tests/vectors/change_ids.json",
        env!("CARGO_MANIFEST_DIR")
    );
    let vectors: Vec<SealedVector> =
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    // The bodies are the ones the layouts above give.
    assert_eq!(vectors[0].body, change_ids_v1_body());
    assert_eq!(vectors[1].body, change_ids_body());
    for v in &vectors {
        let key: [u8; 32] = unhex(&v.key).try_into().unwrap();
        let (body, sealed) = (unhex(&v.body), unhex(&v.sealed));
        assert_eq!(seal(&key, &body), sealed, "{}", v.name);
        assert_eq!(open_sealed(&key, &sealed), Some(&body[..]));
        assert_eq!(open_sealed(&[0; 32], &sealed), None);
    }
    // A version 1 cache is ignored, and rebuilt.
    assert!(ChangeIds::decode(&unhex(&vectors[0].body)).is_err());
    let body = unhex(&vectors[1].body);
    let ids = ChangeIds::decode(&body).unwrap();
    assert_eq!(ids.encode(), body);
    let save = |s: u8, c: u8| (Id([s; 32]), ChangeId([c; 16]));
    assert_eq!(ids.get("").unwrap(), [save(0x33, 0x44)]);
    assert_eq!(
        ids.get("main").unwrap(),
        [save(0x11, 0x22), save(0x33, 0x44)]
    );
    assert_eq!(ids.get("side").unwrap(), [save(0x55, 0x22)]);
}

/// A run from 11…11 (change ID 22…22) to 33…33 (44…44) that stops at 55…55, and a run of 66…66
/// (77…77), which has no first parent, as storage-format.md §10 lays them out.
fn other_lines_body() -> String {
    let id = |byte: &str, n: usize| byte.repeat(n);
    format!(
        "{}010202{}{}{}{}01{}01{}{}00",
        hex(b"OPALLINE"),
        id("11", 32),
        id("22", 16),
        id("33", 32),
        id("44", 16),
        id("55", 32),
        id("66", 32),
        id("77", 16)
    )
}

#[test]
#[ignore = "prints the vector; run by hand to regenerate tests/vectors/other_lines.json"]
fn print_other_lines_vector() {
    let key = [0x42; 32];
    let body = other_lines_body();
    println!(
        "[\n  {{\"name\": \"two-runs\", \"key\": \"{}\", \"body\": \"{body}\", \"sealed\": \"{}\"}}\n]",
        hex(&key),
        hex(&opal_core::keyed::seal(&key, &unhex(&body)))
    );
}

#[test]
fn other_lines_match_their_vector() {
    use opal_core::keyed::{open_sealed, seal};
    use opal_core::otherlines::OtherLines;
    let path = format!(
        "{}/tests/vectors/other_lines.json",
        env!("CARGO_MANIFEST_DIR")
    );
    let vectors: Vec<SealedVector> =
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    assert_eq!(vectors.len(), 1);
    let v = &vectors[0];
    assert_eq!(v.body, other_lines_body());
    let key: [u8; 32] = unhex(&v.key).try_into().unwrap();
    assert_eq!(key, [0x42; 32]);
    let (body, sealed) = (unhex(&v.body), unhex(&v.sealed));
    let lines = OtherLines::decode(&body).unwrap();
    assert_eq!(lines.encode(), body);
    let save = |s: u8, c: u8| (Id([s; 32]), ChangeId([c; 16]));
    let run = lines.run(&Id([0x11; 32])).unwrap();
    assert_eq!(run.saves, [save(0x11, 0x22), save(0x33, 0x44)]);
    assert_eq!(run.next, Some(Id([0x55; 32])));
    let run = lines.run(&Id([0x66; 32])).unwrap();
    assert_eq!(run.saves, [save(0x66, 0x77)]);
    assert_eq!(run.next, None);
    assert_eq!(seal(&key, &body), sealed, "{}", v.name);
    assert_eq!(open_sealed(&key, &sealed), Some(&body[..]));
    assert_eq!(open_sealed(&[0; 32], &sealed), None);
}

/// `a.txt`, 3 bytes, modified at 1,700,000,000.123456789 s, changed at 1,700,000,001.5 s, file 9
/// on drive 0x0102030405060708, racy, with content 11…11; and `b`, an executable of 300 bytes,
/// modified at -1.5 s (whole seconds round down, to -2), changed at -0.999999999 s, file 2^64 + 5
/// on drive 7 (a 128-bit file ID), with content 22…22.
fn file_cache_body() -> String {
    format!(
        "4f50414c46494c45030205{}0103{}{}{}{}{}01{}016202ac02{}{}{}{}{}00{}",
        hex(b"a.txt"),
        hex(&1_700_000_000i64.to_le_bytes()),
        hex(&123_456_789u32.to_le_bytes()),
        hex(&1_700_000_001i64.to_le_bytes()),
        hex(&500_000_000u32.to_le_bytes()),
        "0807060504030201".to_owned() + "0900000000000000" + "0000000000000000",
        "11".repeat(32),
        hex(&(-2i64).to_le_bytes()),
        hex(&500_000_000u32.to_le_bytes()),
        hex(&(-1i64).to_le_bytes()),
        hex(&1u32.to_le_bytes()),
        "0700000000000000".to_owned() + "0500000000000000" + "0100000000000000",
        "22".repeat(32)
    )
}

#[test]
#[ignore = "prints the vector; run by hand to regenerate tests/vectors/file_cache.json"]
fn print_file_cache_vector() {
    let key = [0x42; 32];
    let body = file_cache_body();
    println!(
        "[\n  {{\"name\": \"two-files\", \"key\": \"{}\", \"body\": \"{body}\", \"sealed\": \"{}\"}}\n]",
        hex(&key),
        hex(&opal_core::keyed::seal(&key, &unhex(&body)))
    );
}

#[test]
fn file_cache_matches_its_vector() {
    use opal_core::filecache::FileCache;
    use opal_core::keyed::{open_sealed, seal};
    use opal_platform::dir::{EntryMeta, EntryType};
    let path = format!(
        "{}/tests/vectors/file_cache.json",
        env!("CARGO_MANIFEST_DIR")
    );
    let vectors: Vec<SealedVector> =
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    assert_eq!(vectors[0].body, file_cache_body());
    let meta = |size, modified_ns, changed_ns, executable, file_id| EntryMeta {
        kind: EntryType::File,
        size,
        modified_ns,
        changed_ns,
        executable,
        file_id,
    };
    for v in vectors {
        let key: [u8; 32] = unhex(&v.key).try_into().unwrap();
        let (body, sealed) = (unhex(&v.body), unhex(&v.sealed));
        assert_eq!(seal(&key, &body), sealed, "{}", v.name);
        assert_eq!(open_sealed(&key, &sealed), Some(&body[..]));
        assert_eq!(open_sealed(&[0; 32], &sealed), None);
        let cache = FileCache::decode(&body).unwrap();
        assert_eq!(cache.encode(), body);
        // Racy, so never trusted.
        let a_id = (0x0102_0304_0506_0708, 9);
        let a = meta(
            3,
            1_700_000_000_123_456_789,
            1_700_000_001_500_000_000,
            None,
            a_id,
        );
        assert!(cache.matches("a.txt", &a).is_none());
        let (b_id, b_changed) = ((7, 1 << 64 | 5), -999_999_999);
        let b = cache
            .matches("b", &meta(300, -1_500_000_000, b_changed, Some(true), b_id))
            .unwrap();
        assert_eq!((b.kind, b.id), (EntryKind::Executable, Id([0x22; 32])));
        // Another file there, whose number differs only in its high bits, is not trusted.
        let other = meta(300, -1_500_000_000, b_changed, Some(true), (7, 5));
        assert!(cache.matches("b", &other).is_none());
        // Nor the same file written over in place, which has a new change time.
        let rewritten = meta(300, -1_500_000_000, b_changed + 1, Some(true), b_id);
        assert!(cache.matches("b", &rewritten).is_none());
    }
}

#[derive(Deserialize)]
struct JournalVector {
    name: String,
    hex: String,
}

/// A switch from entry 22…22 to 11…11: make the folder `a`; write `a/b.txt` (300 bytes, 44…44)
/// over the 5-byte file 33…33; write the link `link` (a 6-byte target, 55…55); remove the empty
/// executable `old` (66…66); remove the folder `z`. Laid out as storage-format.md §10 says,
/// without the checksum.
fn journal_body() -> String {
    let id = |byte: &str| byte.repeat(32);
    format!(
        "{}01{}{}05\
         0161{}\
         07{}0101{}05{}ac02\
         04{}0300{}06\
         03{}050102{}00\
         017a060104{}00",
        hex(b"OPALJRNL"),
        id("11"),
        id("22"),
        "0400",
        hex(b"a/b.txt"),
        "01".to_owned() + &id("33"),
        id("44"),
        hex(b"link"),
        id("55"),
        hex(b"old"),
        id("66"),
        id("00"),
    )
}

#[test]
#[ignore = "prints the vector; run by hand to regenerate tests/vectors/journal.json"]
fn print_journal_vector() {
    let body = unhex(&journal_body());
    let mut bytes = body.clone();
    bytes.extend_from_slice(blake3::hash(&body).as_bytes());
    println!(
        "[\n  {{\"name\": \"five-actions\", \"hex\": \"{}\"}}\n]",
        hex(&bytes)
    );
}

#[test]
fn the_journal_matches_its_vectors() {
    use opal_core::journal::{Action, Change, Content, Expected, Journal};
    let path = format!("{}/tests/vectors/journal.json", env!("CARGO_MANIFEST_DIR"));
    let vectors: Vec<JournalVector> =
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let v = &vectors[0];
    let bytes = unhex(&v.hex);
    // The body is the one the layout above gives, and the last 32 bytes are its BLAKE3.
    let (body, sum) = bytes.split_at(bytes.len() - 32);
    assert_eq!(hex(body), journal_body(), "{}", v.name);
    assert_eq!(sum, blake3::hash(body).as_bytes());
    let found = |kind, id, size| {
        Some(Expected {
            kind,
            id: Id([id; 32]),
            size,
        })
    };
    let at = |path: &str, change, expected| Action {
        path: path.into(),
        change,
        expected,
    };
    let journal = Journal {
        target: Id([0x11; 32]),
        from: Id([0x22; 32]),
        actions: vec![
            at("a", Change::MakeFolder, None),
            at(
                "a/b.txt",
                Change::File(Content {
                    id: Id([0x44; 32]),
                    size: 300,
                }),
                found(EntryKind::File, 0x33, 5),
            ),
            at(
                "link",
                Change::Link(Content {
                    id: Id([0x55; 32]),
                    size: 6,
                }),
                None,
            ),
            at(
                "old",
                Change::RemoveFile,
                found(EntryKind::Executable, 0x66, 0),
            ),
            at("z", Change::RemoveFolder, found(EntryKind::Folder, 0, 0)),
        ],
    };
    assert_eq!(Journal::decode(&bytes).unwrap(), journal);
    assert_eq!(journal.encode(), bytes);
}

#[derive(Deserialize)]
struct FormatVector {
    name: String,
    text: String,
    hex: String,
}

#[test]
fn the_format_file_matches_its_vector() {
    use opal_core::repo::{Repository, parse_format};
    let path = format!("{}/tests/vectors/format.json", env!("CARGO_MANIFEST_DIR"));
    let vectors: Vec<FormatVector> =
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    assert_eq!(vectors.len(), 1);
    let v = &vectors[0];
    let bytes = unhex(&v.hex);
    assert_eq!(bytes, v.text.as_bytes(), "{}", v.name);
    // It says the store may hold chunk lists, and nothing more.
    let read = parse_format(&v.text).unwrap();
    assert_eq!(read.features.into_iter().collect::<Vec<_>>(), ["chunks"]);
    // `start` writes exactly these bytes.
    let dir = tempfile::tempdir().unwrap();
    drop(Repository::init(dir.path(), &person()).unwrap());
    let format = dir.path().join(".opal").join("format");
    assert_eq!(hex(&std::fs::read(&format).unwrap()), v.hex, "{}", v.name);
    // A project whose format file holds them opens.
    std::fs::write(&format, &bytes).unwrap();
    Repository::open(dir.path()).unwrap();
}

#[derive(Deserialize)]
struct Chunking {
    gear_context: String,
    noise_context: String,
    inputs: Vec<ChunkingInput>,
}

/// Noise or zeros, perhaps with one byte inserted, and the chunk sizes Opalro's cutter makes of it.
#[derive(Deserialize)]
struct ChunkingInput {
    name: String,
    noise: Option<usize>,
    zeros: Option<usize>,
    insert_at: Option<usize>,
    insert_byte: Option<u8>,
    file: String,
    sizes: Vec<u64>,
}

fn chunking() -> Chunking {
    let path = format!("{}/tests/vectors/chunking.json", env!("CARGO_MANIFEST_DIR"));
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

/// The bytes of `input`. The noise is BLAKE3's extendable output in key-derivation mode, with no
/// key material.
fn chunking_input(v: &Chunking, input: &ChunkingInput) -> Vec<u8> {
    let mut bytes = vec![0; input.noise.or(input.zeros).unwrap()];
    if input.noise.is_some() {
        blake3::Hasher::new_derive_key(&v.noise_context)
            .finalize_xof()
            .fill(&mut bytes);
    }
    if let Some(at) = input.insert_at {
        bytes.insert(at, input.insert_byte.unwrap());
    }
    bytes
}

/// The bytes of the input `name` of chunking.json, checked against its file ID.
fn input_bytes(v: &Chunking, name: &str) -> Vec<u8> {
    let input = v.inputs.iter().find(|i| i.name == name).unwrap();
    let bytes = chunking_input(v, input);
    assert_eq!(blake3::hash(&bytes).to_hex().as_str(), input.file, "{name}");
    bytes
}

/// The chunks Opalro's cutter makes of `bytes`.
fn cut(bytes: &[u8]) -> Vec<Chunk> {
    let mut cutter = Cutter::new(bytes);
    let mut chunks = Vec::new();
    while let Some(chunk) = cutter.next_chunk().unwrap() {
        chunks.push(Chunk {
            id: hash_content(chunk),
            size: chunk.len() as u64,
        });
    }
    chunks
}

#[test]
#[ignore = "prints the vectors; run by hand to regenerate tests/vectors/chunking.json"]
fn print_chunking_vectors() {
    let v = chunking();
    // GEAR as storage-format.md section 6 makes it: 256 little-endian numbers from BLAKE3.
    let mut table = [0; 2048];
    blake3::Hasher::new_derive_key(&v.gear_context)
        .finalize_xof()
        .fill(&mut table);
    let gear = |at: usize| u64::from_le_bytes(table[8 * at..8 * at + 8].try_into().unwrap());
    let rows: Vec<String> = v
        .inputs
        .iter()
        .map(|input| {
            let bytes = chunking_input(&v, input);
            let sizes: Vec<u64> = cut(&bytes).iter().map(|c| c.size).collect();
            let mut row = format!("    {{\"name\": \"{}\"", input.name);
            for (key, n) in [
                ("noise", input.noise),
                ("zeros", input.zeros),
                ("insert_at", input.insert_at),
                ("insert_byte", input.insert_byte.map(usize::from)),
            ] {
                if let Some(n) = n {
                    row += &format!(", \"{key}\": {n}");
                }
            }
            let file = blake3::hash(&bytes).to_hex();
            format!("{row}, \"file\": \"{file}\", \"sizes\": {sizes:?}}}")
        })
        .collect();
    println!(
        "{{\n  \"gear_context\": \"{}\",\n  \"gear\": {{\"0\": \"{:016x}\", \"1\": \"{:016x}\", \"255\": \"{:016x}\"}},\n  \"noise_context\": \"{}\",\n  \"inputs\": [\n{}\n  ]\n}}",
        v.gear_context,
        gear(0),
        gear(1),
        gear(255),
        v.noise_context,
        rows.join(",\n")
    );
}

#[test]
fn the_cutter_cuts_the_vector_inputs_where_the_spec_says() {
    let v = chunking();
    assert_eq!(v.inputs.len(), 4);
    for input in &v.inputs {
        let sizes: Vec<u64> = cut(&input_bytes(&v, &input.name))
            .iter()
            .map(|c| c.size)
            .collect();
        assert_eq!(sizes, input.sizes, "{}", input.name);
    }
}

#[test]
fn one_inserted_byte_changes_one_chunk() {
    let v = chunking();
    let before = cut(&input_bytes(&v, "noise-3mib"));
    let after = cut(&input_bytes(&v, "noise-3mib-one-byte-inserted"));
    assert_eq!((before.len(), after.len()), (41, 41));
    // The 13th chunk holds the inserted byte; every other is the same chunk as before.
    assert_eq!(before[..12], after[..12]);
    assert_ne!(before[12].id, after[12].id);
    assert_eq!(before[12].size + 1, after[12].size);
    assert_eq!(before[13..], after[13..]);
}

#[derive(Deserialize)]
struct ChunkListVectors {
    valid: Vec<ValidChunkList>,
    invalid: Vec<InvalidChunkList>,
}

/// A file given by its text and the pieces it is cut into, or by an input of chunking.json.
#[derive(Deserialize)]
struct ValidChunkList {
    name: String,
    text: Option<String>,
    chunks: Option<Vec<String>>,
    input: Option<String>,
    file: String,
    size: u64,
    hex: String,
}

#[derive(Deserialize)]
struct InvalidChunkList {
    name: String,
    hex: String,
    why: String,
}

fn chunk_list_vectors() -> ChunkListVectors {
    let path = format!(
        "{}/tests/vectors/chunk_list.json",
        env!("CARGO_MANIFEST_DIR")
    );
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

/// The list of `vector`'s file: cut into the pieces it gives, or as Opalro's cutter cuts it.
fn valid_chunk_list(vector: &ValidChunkList) -> ChunkList {
    let (file, chunks) = match (&vector.text, &vector.chunks, &vector.input) {
        (Some(text), Some(pieces), None) => {
            assert_eq!(pieces.concat(), *text, "{}", vector.name);
            let chunks = pieces
                .iter()
                .map(|piece| Chunk {
                    id: hash_content(piece.as_bytes()),
                    size: piece.len() as u64,
                })
                .collect();
            (text.as_bytes().to_vec(), chunks)
        }
        (None, None, Some(input)) => {
            let name = input.strip_prefix("chunking.json ").unwrap();
            let bytes = input_bytes(&chunking(), name);
            let chunks = cut(&bytes);
            (bytes, chunks)
        }
        _ => panic!("{}: a text and its chunks, or an input", vector.name),
    };
    ChunkList {
        file: hash_content(&file),
        size: file.len() as u64,
        chunks,
    }
}

#[test]
#[ignore = "prints the vectors; run by hand to regenerate tests/vectors/chunk_list.json"]
fn print_chunk_list_vectors() {
    let v = chunk_list_vectors();
    let valid: Vec<String> = v
        .valid
        .iter()
        .map(|vector| {
            let list = valid_chunk_list(vector);
            let given = match (&vector.text, &vector.chunks, &vector.input) {
                (Some(text), Some(pieces), _) => {
                    format!("\"text\": \"{text}\", \"chunks\": {pieces:?}")
                }
                (_, _, input) => format!("\"input\": \"{}\"", input.as_ref().unwrap()),
            };
            format!(
                "    {{\"name\": \"{}\", {given}, \"file\": \"{}\", \"size\": {}, \"hex\": \"{}\"}}",
                vector.name,
                list.file,
                list.size,
                hex(&list.encode())
            )
        })
        .collect();
    let invalid: Vec<String> = v
        .invalid
        .iter()
        .map(|i| {
            format!(
                "    {{\"name\": \"{}\", \"hex\": \"{}\", \"why\": \"{}\"}}",
                i.name, i.hex, i.why
            )
        })
        .collect();
    println!(
        "{{\n  \"valid\": [\n{}\n  ],\n  \"invalid\": [\n{}\n  ]\n}}",
        valid.join(",\n"),
        invalid.join(",\n")
    );
}

#[test]
fn chunk_lists_match_their_vectors() {
    let v = chunk_list_vectors();
    assert_eq!(v.valid.len(), 2);
    for vector in &v.valid {
        let list = valid_chunk_list(vector);
        assert_eq!(list.file.to_hex(), vector.file, "{}", vector.name);
        assert_eq!(list.size, vector.size, "{}", vector.name);
        assert_eq!(hex(&list.encode()), vector.hex, "{}", vector.name);
        assert_eq!(
            ChunkList::decode(&unhex(&vector.hex)),
            Ok(list),
            "{}",
            vector.name
        );
    }
}

#[test]
fn chunk_lists_that_break_a_rule_are_refused() {
    let v = chunk_list_vectors();
    assert_eq!(v.invalid.len(), 8);
    for vector in &v.invalid {
        let refused = ChunkList::decode(&unhex(&vector.hex));
        assert!(refused.is_err(), "{}: {refused:?}", vector.name);
    }
}

#[derive(Deserialize)]
struct RulesVector {
    name: String,
    toml: String,
    skipped: bool,
    preset: Option<String>,
    chunk_files_over: u64,
    ask_before_saving_over: u64,
    ask_before_saving_files_over: u64,
    rules: Vec<RuleVector>,
    ignored: Vec<String>,
}

#[derive(Deserialize)]
struct RuleVector {
    files: Vec<String>,
    lockable: Option<bool>,
    conflict_markers: Option<bool>,
}

#[test]
fn every_vector_reads_as_it_says() {
    let path = format!("{}/tests/vectors/rules.json", env!("CARGO_MANIFEST_DIR"));
    let vectors: Vec<RulesVector> =
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    assert_eq!(vectors.len(), 50);
    for v in &vectors {
        let parsed = rules::parse(v.toml.as_bytes());
        let (read, ignored) = match parsed {
            Ok(_) if v.skipped => panic!("{}: read, though it isn't TOML", v.name),
            Err(why) if !v.skipped => panic!("{}: passed over: {why}", v.name),
            Ok((read, ignored)) => (read, ignored),
            Err(_) => (Rules::default(), vec![]),
        };
        let expected = Rules {
            preset: v.preset.clone(),
            chunk_files_over: v.chunk_files_over,
            ask_over_bytes: v.ask_before_saving_over,
            ask_over_files: v.ask_before_saving_files_over,
            rules: v
                .rules
                .iter()
                .map(|r| Rule {
                    files: r.files.clone(),
                    lockable: r.lockable,
                    conflict_markers: r.conflict_markers,
                })
                .collect(),
        };
        assert_eq!(read, expected, "{}", v.name);
        let names: Vec<&str> = ignored.iter().map(|(name, _)| name.as_str()).collect();
        assert_eq!(names, v.ignored, "{}", v.name);
        assert!(ignored.iter().all(|(_, why)| !why.is_empty()), "{}", v.name);
    }
}

#[derive(Deserialize)]
struct PresetVectors {
    presets: Vec<PresetVector>,
    opalignore_edits: Vec<OpalignoreEdit>,
}

#[derive(Deserialize)]
struct PresetVector {
    name: String,
    preset: String,
    found: Vec<String>,
    opal_toml: String,
    opalignore_block: String,
}

#[derive(Deserialize)]
struct OpalignoreEdit {
    name: String,
    before: Option<String>,
    block: Option<String>,
    after: Option<String>,
}

fn preset_vectors() -> PresetVectors {
    let path = format!("{}/tests/vectors/presets.json", env!("CARGO_MANIFEST_DIR"));
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

#[test]
fn every_preset_vector_is_written_byte_for_byte() {
    let vectors = preset_vectors().presets;
    assert_eq!(vectors.len(), 8);
    for v in &vectors {
        let preset = Preset::from_name(&v.preset).unwrap();
        let found = Found {
            engines: Engine::ALL
                .into_iter()
                .filter(|e| v.found.iter().any(|f| f == e.name()))
                .collect(),
            languages: Language::ALL
                .into_iter()
                .filter(|l| v.found.iter().any(|f| f == l.name()))
                .collect(),
        };
        assert_eq!(found.engines.len() + found.languages.len(), v.found.len());
        assert_eq!(
            presets::opal_toml(preset, &found).as_deref(),
            Some(v.opal_toml.as_str()),
            "{}",
            v.name
        );
        assert_eq!(
            presets::opalignore_block(preset, &found).as_deref(),
            Some(v.opalignore_block.as_str()),
            "{}",
            v.name
        );
    }
}

#[test]
fn every_opalignore_edit_gives_its_vector() {
    let vectors = preset_vectors().opalignore_edits;
    assert_eq!(vectors.len(), 16);
    for v in &vectors {
        let after =
            presets::edit_opalignore(v.before.as_ref().map(String::as_bytes), v.block.as_deref());
        assert_eq!(
            after.as_deref(),
            v.after.as_ref().map(String::as_bytes),
            "{}",
            v.name
        );
    }
}
