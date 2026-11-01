//! Test vectors for the storage format (docs/storage-format.md).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use bolt_core::id::Id;
use bolt_core::listing::{Entry, EntryKind, Listing};
use serde::Deserialize;

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
