//! Hostile chunk lists (security.md T9): each case plants a save naming `big.bin`, which a hostile
//! `.bolt` holds through a chunk list no honest writer makes, then runs `goto`, `discard`,
//! `status`, `changes` and `show` over it. Each refuses, or reads a good copy; nothing wrong
//! reaches the folder, and nothing outside the project changes.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use std::collections::BTreeMap;
use std::path::Path;
use std::time::Instant;

use bolt_core::chunks::{Chunk, ChunkList, MAX_CHUNKS};
use bolt_core::encoding::Writer;
use bolt_core::id::{ContentHasher, Id, hash_content};
use bolt_core::pack::PackSet;
use serde_json::{Value, json};
use support::corpus::{Case, file, kinds, message, project_entries};
use support::{add_list_record, add_pack, cut, id_of, noise, plant_save, text, top_listing};

/// What the save names: past 1 MiB, so a copy of it streams out before its whole-file check.
const SIZE: usize = 3 << 19;
/// What the folder holds at `big.bin` while the project is on the save.
const MINE: &[u8] = b"mine\n";

fn big() -> Vec<u8> {
    noise(1, SIZE)
}

/// What a hostile `.bolt` holds for `big.bin`: the ID and size its save names it by, and the
/// pack holding the chunk list that is wrong.
struct Planted {
    file: Id,
    size: u64,
    pack: String,
}

fn name_of(path: &Path) -> String {
    path.file_name().unwrap().to_string_lossy().into_owned()
}

/// Plants a save on `main` naming `big.bin` by `file` and `size`, then a pack of what `store`
/// stores for it.
fn plant(root: &Path, file: Id, size: u64, store: impl FnOnce(&mut PackSet)) -> Planted {
    plant_save(root, |pack| top_listing(pack, &[("big.bin", file, size)]));
    let packs = add_pack(root, store);
    Planted {
        file,
        size,
        pack: name_of(&packs[0]),
    }
}

fn store_all(pack: &mut PackSet, chunks: &[&[u8]]) {
    for chunk in chunks {
        pack.add_content(chunk).unwrap();
    }
}

/// What the commands should come to: a refusal saying this, or the file read from a good copy,
/// with the pack holding a bad one named in a warning when there is one.
enum Expect {
    Refused(String),
    Good { named: bool },
}

/// A refusal of the chunk `c`, which the store doesn't hold as content of its size.
fn not_held(c: Chunk) -> Expect {
    let why = format!(
        "names chunk {} of {} bytes, which isn't in the store",
        c.id, c.size
    );
    Expect::Refused(why)
}

/// Fails unless `doc` refuses the file `p` names as damage in its pack, saying `why`, after
/// `before`, and warns of that pack.
fn refused(doc: &Value, p: &Planted, before: &str, why: &str) {
    let said = doc["error"]["message"].as_str().unwrap();
    let damage = "This project's Bolt data looks damaged: pack ";
    assert!(said.starts_with(&format!("{before}{damage}")), "{said}");
    let file = p.file.to_hex();
    for told in [p.pack.as_str(), file.as_str(), why] {
        assert!(said.contains(told), "{told} in {said}");
    }
    assert_eq!(kinds(doc), ["damaged_pack"], "{doc}");
    assert!(message(doc, "damaged_pack").contains(&p.pack), "{doc}");
}

/// The warnings of a document that read the file from a good copy.
fn read_well(doc: &Value, p: &Planted, named: bool) {
    if named {
        assert_eq!(kinds(doc), ["damaged_pack"], "{doc}");
        assert!(message(doc, "damaged_pack").contains(&p.pack), "{doc}");
    } else {
        assert!(kinds(doc).is_empty(), "{doc}");
    }
}

/// Runs the commands over what `make` plants: `goto` onto the save from the project's first one;
/// then, in a project on the save with its own `big.bin` unsaved, `status`, `show`, `changes` and
/// `discard`.
fn over(make: impl Fn(&Path) -> Planted, expect: Expect) {
    let stopped = "A switch between saves stopped at big.bin: ";
    let holding = |bytes: &[u8]| BTreeMap::from([("big.bin".to_owned(), file(bytes))]);
    let case = Case::new();
    let p = make(case.root());
    let (code, doc, _) = case.run_json(&["goto", "main"]);
    match &expect {
        Expect::Refused(why) => {
            assert_eq!(code, Some(1), "{doc}");
            refused(&doc, &p, stopped, why);
            // The damage stops every later try too, so the hint names the way on.
            let hint = doc["error"]["hint"].as_str().unwrap();
            assert!(
                hint.contains("delete .bolt/state/journal: your files stay as they are"),
                "{hint}"
            );
            assert!(case.root().join(".bolt/state/journal").exists());
            assert!(project_entries(&case.env).is_empty());
        }
        Expect::Good { named } => {
            assert_eq!(code, Some(0), "{doc}");
            read_well(&doc, &p, *named);
            assert_eq!(project_entries(&case.env), holding(&big()));
        }
    }
    case.assert_nothing_escaped();

    let case = Case::on_main();
    let p = make(case.root());
    std::fs::write(case.root().join("big.bin"), MINE).unwrap();
    // Neither reads the file's contents, only its ID and size.
    let (code, doc, _) = case.run_json(&["status"]);
    assert_eq!(code, Some(0), "{doc}");
    let modified = json!([{"path": "big.bin", "change": "modified", "kind": "file",
        "old_size": p.size, "new_size": MINE.len()}]);
    assert_eq!(
        (&doc["changes"], kinds(&doc).len()),
        (&modified, 0),
        "{doc}"
    );
    let (code, doc, _) = case.run_json(&["show"]);
    assert_eq!(code, Some(0), "{doc}");
    let added = json!([{"path": "big.bin", "change": "added", "kind": "file",
        "old_size": null, "new_size": p.size}]);
    assert_eq!((&doc["changes"], kinds(&doc).len()), (&added, 0), "{doc}");
    let (code, doc, _) = case.run_json(&["changes"]);
    match &expect {
        Expect::Refused(why) => {
            assert_eq!(code, Some(1), "{doc}");
            refused(&doc, &p, "", why);
        }
        Expect::Good { named } => {
            assert_eq!(code, Some(0), "{doc}");
            assert_eq!(doc["changes"][0]["lines"], "binary", "{doc}");
            read_well(&doc, &p, *named);
        }
    }
    let (code, doc, _) = case.run_json(&["discard"]);
    match &expect {
        Expect::Refused(why) => {
            assert_eq!(code, Some(1), "{doc}");
            refused(&doc, &p, stopped, why);
            // The unsaved file stays as it was.
            assert_eq!(project_entries(&case.env), holding(MINE));
        }
        Expect::Good { named } => {
            assert_eq!(code, Some(0), "{doc}");
            assert_eq!(doc["restored_paths"], json!(["big.bin"]), "{doc}");
            read_well(&doc, &p, *named);
            assert_eq!(project_entries(&case.env), holding(&big()));
        }
    }
    case.assert_nothing_escaped();
}

#[test]
fn a_chunk_list_naming_another_files_chunks_is_never_written_out() {
    // Each chunk is good and the sizes add up, but they build another file.
    let other = noise(2, SIZE);
    let (theirs, chunks) = cut(&other);
    let list = ChunkList {
        file: hash_content(&big()),
        ..theirs
    };
    over(
        |root| plant_list(root, &list, &chunks),
        Expect::Refused("names build another file".into()),
    );
}

/// Plants a save naming `big.bin` as `list`'s file, of `SIZE` bytes whatever the list says, and a
/// pack of `chunks` and `list`.
fn plant_list(root: &Path, list: &ChunkList, chunks: &[&[u8]]) -> Planted {
    plant(root, list.file, SIZE as u64, |pack| {
        store_all(pack, chunks);
        pack.add_chunk_list(list).unwrap();
    })
}

#[test]
fn a_chunk_list_naming_a_chunk_of_another_size_is_refused() {
    let big = big();
    let (mut list, chunks) = cut(&big);
    // Two chunks swap a byte of size, so the sizes still add up.
    list.chunks[0].size += 1;
    list.chunks[1].size -= 1;
    over(
        |root| plant_list(root, &list, &chunks),
        not_held(list.chunks[0]),
    );
}

#[test]
fn a_chunk_list_naming_a_missing_chunk_is_refused() {
    let big = big();
    let (list, mut chunks) = cut(&big);
    chunks.remove(3);
    over(
        |root| plant_list(root, &list, &chunks),
        not_held(list.chunks[3]),
    );
}

#[test]
fn a_chunk_list_under_a_chunks_id_is_never_read_as_the_chunk() {
    let big = big();
    let (list, mut chunks) = cut(&big);
    // The second chunk is held only as a chunk list of its two halves, each stored whole. They
    // build its bytes, so a read that took the list for the chunk would write the file.
    let second = list.chunks[1];
    let held = chunks.remove(1);
    let halves: Vec<&[u8]> = held.chunks(held.len().div_ceil(2)).collect();
    let inner = ChunkList {
        file: second.id,
        size: second.size,
        chunks: halves
            .iter()
            .map(|h| Chunk {
                id: hash_content(h),
                size: h.len() as u64,
            })
            .collect(),
    };
    let make = |root: &Path| {
        plant(root, list.file, list.size, |pack| {
            store_all(pack, &chunks);
            store_all(pack, &halves);
            pack.add_chunk_list(&inner).unwrap();
            pack.add_chunk_list(&list).unwrap();
        })
    };
    over(make, not_held(second));
    // With the chunk stored whole as well, that copy is read.
    let with_the_chunk = |root: &Path| {
        let p = make(root);
        add_pack(root, |pack| store_all(pack, &[held]));
        p
    };
    over(with_the_chunk, Expect::Good { named: false });
}

#[test]
fn a_chunk_list_claiming_10_gib_for_3_small_chunks_is_refused() {
    let big = big();
    let thirds: Vec<&[u8]> = big.chunks(SIZE / 3).collect();
    let list = ChunkList {
        file: hash_content(&big),
        size: 10 << 30,
        chunks: thirds
            .iter()
            .map(|c| Chunk {
                id: hash_content(c),
                size: c.len() as u64,
            })
            .collect(),
    };
    let why = "is invalid: invalid value: the chunk sizes don't add up to the file size";
    over(
        |root| plant_list(root, &list, &thirds),
        Expect::Refused(why.into()),
    );
}

#[test]
fn a_chunk_count_far_past_its_bytes_is_refused() {
    let big = big();
    let (list, chunks) = cut(&big);
    // The list's own record, but for the most chunks a list may hold.
    let mut record = Writer::new();
    record.u8(1);
    record.id(&list.file);
    record.varint(list.size);
    record.varint(MAX_CHUNKS);
    for c in &list.chunks {
        record.id(&c.id);
        record.varint(c.size);
    }
    let record = record.into_bytes();
    let make = |root: &Path| {
        plant(root, list.file, list.size, |pack| {
            store_all(pack, &chunks);
            add_list_record(pack, list.file, list.size, record.clone());
        })
    };
    let why = "is invalid: invalid value: the chunk count is more than the record holds";
    over(make, Expect::Refused(why.into()));
}

#[test]
fn a_damaged_chunk_list_beside_a_good_whole_copy_is_passed_over_and_named() {
    let big = big();
    let (mut list, mut chunks) = cut(&big);
    // Cut short: smaller than the whole copy, so it is tried first.
    let last = list.chunks.pop().unwrap();
    chunks.pop();
    list.size -= last.size;
    let make = |root: &Path| {
        plant_save(root, |pack| {
            top_listing(pack, &[("big.bin", list.file, SIZE as u64)])
        });
        add_pack(root, |pack| store_all(pack, &[&big]));
        let damaged = add_pack(root, |pack| {
            store_all(pack, &chunks);
            pack.add_chunk_list(&list).unwrap();
        });
        Planted {
            file: list.file,
            size: SIZE as u64,
            pack: name_of(&damaged[0]),
        }
    };
    over(make, Expect::Good { named: true });
}

/// The ID of `size` bytes of `byte`, hashed a block at a time.
fn id_of_repeated(byte: u8, size: u64) -> Id {
    let block = vec![byte; 1 << 20];
    let mut hasher = ContentHasher::new();
    let mut left = size;
    while left > 0 {
        let n = left.min(block.len() as u64) as usize;
        hasher.update(&block[..n]);
        left -= n as u64;
    }
    hasher.finish()
}

#[test]
#[ignore = "takes minutes: nightly runs it in a release build"]
fn a_list_of_the_most_chunks_a_list_may_hold_is_written_out_whole() {
    // 8 MiB of one byte, each byte a chunk, all the same one: valid, as any cut is.
    let case = Case::new();
    let file = id_of_repeated(b'a', MAX_CHUNKS);
    let one = Chunk {
        id: hash_content(b"a"),
        size: 1,
    };
    let list = ChunkList {
        file,
        size: MAX_CHUNKS,
        chunks: vec![one; MAX_CHUNKS as usize],
    };
    plant(case.root(), file, MAX_CHUNKS, |pack| {
        pack.add_content(b"a").unwrap();
        pack.add_chunk_list(&list).unwrap();
    });
    // Not held while bolt reads its own copy of it, 335 MB.
    drop(list);
    let started = Instant::now();
    let out = case.env.bolt().args(["goto", "main"]).output().unwrap();
    println!("goto took {:.1} s", started.elapsed().as_secs_f64());
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    let written = case.root().join("big.bin");
    assert_eq!(std::fs::metadata(&written).unwrap().len(), MAX_CHUNKS);
    assert_eq!(id_of(&written), file);
    case.assert_nothing_escaped();
}
