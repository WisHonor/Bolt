//! Writes Git repositories byte by byte, for tests. Nothing here checks what it writes, so
//! malformed repositories are as easy to make as good ones.
#![deny(clippy::indexing_slicing, clippy::arithmetic_side_effects)]
// Test code: a write that fails should stop the test.
#![allow(clippy::expect_used)]

use std::fs;
use std::path::{Path, PathBuf};

use sha2::Digest as _;

use super::id::{Hasher, object_header};
use super::pack::apply_delta;
use super::{GitId, HashKind, ObjectType, object_id};

/// A Git repository a test is writing, at `<dir>/.git`.
pub struct Craft {
    git: PathBuf,
    hash: HashKind,
}

/// An entry of a pack that `Craft::pack` writes.
pub enum PackEntry {
    /// An object stored whole.
    Whole(ObjectType, Vec<u8>),
    /// A delta on the entry at index `base` of the same list: an earlier one, or this one.
    OfsDelta { base: usize, delta: Vec<u8> },
    /// A delta on the object `base`.
    RefDelta { base: GitId, delta: Vec<u8> },
    /// Bytes written as they are: header, base and compressed data.
    Raw(Vec<u8>),
}

impl Craft {
    /// `.git/objects`, `.git/refs`, a minimal config, and `HEAD` on `main`, which has no
    /// commits yet.
    #[must_use]
    pub fn new(dir: &Path, hash: HashKind) -> Craft {
        let craft = Craft {
            git: dir.join(".git"),
            hash,
        };
        for folder in ["objects", "refs/heads", "refs/tags"] {
            fs::create_dir_all(craft.git.join(folder)).expect("make a .git folder");
        }
        craft.config(match hash {
            HashKind::Sha1 => "[core]\n\trepositoryformatversion = 0\n",
            HashKind::Sha256 => {
                "[core]\n\trepositoryformatversion = 1\n[extensions]\n\tobjectformat = sha256\n"
            }
        });
        craft.head("ref: refs/heads/main\n");
        craft
    }

    #[must_use]
    pub fn hash(&self) -> HashKind {
        self.hash
    }

    pub fn config(&self, text: &str) {
        self.file("config", text.as_bytes());
    }

    pub fn head(&self, text: &str) {
        self.file("HEAD", text.as_bytes());
    }

    /// Writes `bytes` to `rel` (names joined by `/`) under `.git`, making its folders.
    pub fn file(&self, rel: &str, bytes: &[u8]) {
        let path = self.git.join(rel);
        if let Some(folder) = path.parent() {
            fs::create_dir_all(folder).expect("make the file's folder");
        }
        fs::write(&path, bytes).expect("write a file under .git");
    }

    /// Writes the ref `name` (`refs/heads/main`, say) as a loose ref holding `text`.
    pub fn loose_ref(&self, name: &str, text: &str) {
        self.file(name, text.as_bytes());
    }

    pub fn packed_refs(&self, text: &str) {
        self.file("packed-refs", text.as_bytes());
    }

    /// Stores `bytes` as a loose object of `kind`, as Git does: header and bytes, compressed with
    /// zlib, under the ID.
    pub fn object(&self, kind: ObjectType, bytes: &[u8]) -> GitId {
        let id = object_id(kind, bytes, self.hash).expect("no SHA-1 collision attack");
        let mut raw = object_header(kind, bytes.len() as u64).into_bytes();
        raw.extend_from_slice(bytes);
        self.raw_loose(&id.to_hex(), &zlib(&raw));
        id
    }

    /// Writes `compressed` as the loose object `id_hex`, whatever it holds.
    pub fn raw_loose(&self, id_hex: &str, compressed: &[u8]) {
        let (folder, file) = id_hex
            .split_at_checked(2)
            .expect("an ID of two digits or more");
        self.file(&format!("objects/{folder}/{file}"), compressed);
    }

    /// A tree of `entries` (mode, name, ID), written in the order given, sorted or not.
    pub fn tree(&self, entries: &[(&[u8], &[u8], GitId)]) -> GitId {
        let mut bytes = Vec::new();
        for (mode, name, id) in entries {
            bytes.extend_from_slice(mode);
            bytes.push(b' ');
            bytes.extend_from_slice(name);
            bytes.push(0);
            bytes.extend_from_slice(id.as_bytes());
        }
        self.object(ObjectType::Tree, &bytes)
    }

    pub fn commit(&self, raw: &[u8]) -> GitId {
        self.object(ObjectType::Commit, raw)
    }

    /// Keeps `contents` in `.git/lfs/objects`, as `git lfs fetch` does; the pointer to them.
    pub fn lfs_object(&self, contents: &[u8]) -> String {
        let (pointer, path) = lfs_pointer_to(contents);
        self.file(&path, contents);
        pointer
    }

    /// Writes `entries` as a pack and its index in `objects/pack`, as Git would, and returns
    /// their paths. Each entry is listed under its object's ID, worked out through earlier
    /// entries; one whose object can't be worked out (a `Raw` entry, or a delta on an unknown
    /// base or that doesn't apply) is listed under the ID of a blob holding its delta or raw
    /// bytes.
    pub fn pack(&self, entries: &[PackEntry]) -> (PathBuf, PathBuf) {
        self.pack_with_large_offsets(entries, 0x7fff_ffff)
    }

    /// `pack`, listing the entries `renamed` gives by index under the names given, as a hostile
    /// pack can: the entries built on them are still listed under what they build.
    pub fn pack_renamed(
        &self,
        entries: &[PackEntry],
        renamed: &[(usize, GitId)],
    ) -> (PathBuf, PathBuf) {
        self.write_pack(entries, 0x7fff_ffff, renamed)
    }

    /// `pack`, with every offset over `limit` in the index's 64-bit table, as
    /// `git index-pack --index-version=2,<limit>` writes it.
    pub fn pack_with_large_offsets(&self, entries: &[PackEntry], limit: u64) -> (PathBuf, PathBuf) {
        self.write_pack(entries, limit, &[])
    }

    fn write_pack(
        &self,
        entries: &[PackEntry],
        limit: u64,
        renamed: &[(usize, GitId)],
    ) -> (PathBuf, PathBuf) {
        let count = u32::try_from(entries.len()).expect("fewer than 2^32 entries");
        let mut pack = b"PACK".to_vec();
        pack.extend_from_slice(&2u32.to_be_bytes());
        pack.extend_from_slice(&count.to_be_bytes());
        let mut listed: Vec<Listed> = Vec::new();
        for (index, entry) in entries.iter().enumerate() {
            let at = pack.len() as u64;
            let (bytes, object, own) = match entry {
                PackEntry::Whole(kind, data) => {
                    let mut bytes = entry_header(type_code(*kind), data.len() as u64);
                    bytes.extend_from_slice(&zlib(data));
                    (bytes, Some((*kind, data.clone())), data)
                }
                PackEntry::OfsDelta { base, delta } => {
                    let base_at = if *base == index {
                        at
                    } else {
                        listed.get(*base).expect("an earlier entry as base").at
                    };
                    let mut bytes = entry_header(6, delta.len() as u64);
                    bytes.extend_from_slice(&ofs_distance(
                        at.checked_sub(base_at).expect("a base before its delta"),
                    ));
                    bytes.extend_from_slice(&zlib(delta));
                    let base = listed.get(*base).and_then(|b| b.object.as_ref());
                    (bytes, built(base, delta), delta)
                }
                PackEntry::RefDelta { base, delta } => {
                    let mut bytes = entry_header(7, delta.len() as u64);
                    bytes.extend_from_slice(base.as_bytes());
                    bytes.extend_from_slice(&zlib(delta));
                    let base = listed.iter().find(|b| b.id == *base);
                    (
                        bytes,
                        built(base.and_then(|b| b.object.as_ref()), delta),
                        delta,
                    )
                }
                PackEntry::Raw(bytes) => (bytes.clone(), None, bytes),
            };
            let id = match &object {
                Some((kind, data)) => object_id(*kind, data, self.hash),
                None => object_id(ObjectType::Blob, own, self.hash),
            }
            .expect("no SHA-1 collision attack");
            let id = renamed
                .iter()
                .find(|(i, _)| *i == index)
                .map_or(id, |(_, name)| *name);
            listed.push(Listed {
                at,
                id,
                crc: crc32(&bytes),
                object,
            });
            pack.extend_from_slice(&bytes);
        }
        let checksum = digest(self.hash, &pack);
        pack.extend_from_slice(&checksum);
        let index = self.index(&mut listed, &checksum, limit);
        let name = format!(
            "objects/pack/pack-{}",
            GitId::new(self.hash, &checksum).to_hex()
        );
        self.file(&format!("{name}.pack"), &pack);
        self.file(&format!("{name}.idx"), &index);
        (
            self.git.join(format!("{name}.pack")),
            self.git.join(format!("{name}.idx")),
        )
    }

    /// A version 2 index of `listed`, putting offsets over `limit` in its 64-bit table.
    fn index(&self, listed: &mut [Listed], pack_checksum: &[u8], limit: u64) -> Vec<u8> {
        listed.sort_by_key(|entry| entry.id);
        let mut out = b"\xfftOc".to_vec();
        out.extend_from_slice(&2u32.to_be_bytes());
        for byte in 0..=255u8 {
            let below = listed
                .iter()
                .filter(|entry| entry.id.as_bytes().first() <= Some(&byte))
                .count();
            out.extend_from_slice(&u32::try_from(below).expect("a u32 count").to_be_bytes());
        }
        for entry in listed.iter() {
            out.extend_from_slice(entry.id.as_bytes());
        }
        for entry in listed.iter() {
            out.extend_from_slice(&entry.crc.to_be_bytes());
        }
        let mut large = Vec::new();
        for entry in listed.iter() {
            let small = if entry.at > limit {
                large.push(entry.at);
                0x8000_0000 | u32::try_from(large.len().wrapping_sub(1)).expect("a u31 index")
            } else {
                u32::try_from(entry.at).expect("a 31-bit offset")
            };
            out.extend_from_slice(&small.to_be_bytes());
        }
        for at in large {
            out.extend_from_slice(&at.to_be_bytes());
        }
        out.extend_from_slice(pack_checksum);
        let checksum = digest(self.hash, &out);
        out.extend_from_slice(&checksum);
        out
    }
}

/// The pointer Git LFS writes for a file holding `contents`, and where under `.git` its object
/// goes: `lfs/objects/<oid[0..2]>/<oid[2..4]>/<oid>`.
#[must_use]
pub fn lfs_pointer_to(contents: &[u8]) -> (String, String) {
    let oid: String = sha2::Sha256::digest(contents)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let (first, second) = (oid.get(..2), oid.get(2..4));
    let (first, second) = (first.unwrap_or_default(), second.unwrap_or_default());
    let pointer = format!(
        "version https://git-lfs.github.com/spec/v1\noid sha256:{oid}\nsize {}\n",
        contents.len()
    );
    (pointer, format!("lfs/objects/{first}/{second}/{oid}"))
}

/// What `Craft::pack` knows of an entry it wrote.
struct Listed {
    at: u64,
    id: GitId,
    crc: u32,
    object: Option<(ObjectType, Vec<u8>)>,
}

/// The object `delta` builds on `base`, if both are known and the delta applies.
fn built(base: Option<&(ObjectType, Vec<u8>)>, delta: &[u8]) -> Option<(ObjectType, Vec<u8>)> {
    let (kind, base) = base?;
    Some((*kind, apply_delta(base, delta).ok()?))
}

/// A valid delta that builds `result` from `base`, as Git writes one: the bytes they share at
/// the start are copied, at most 0x10000 at a time, and the rest is inserted.
#[must_use]
pub fn delta(base: &[u8], result: &[u8]) -> Vec<u8> {
    let mut out = size(base.len() as u64);
    out.extend_from_slice(&size(result.len() as u64));
    let shared = base.iter().zip(result).take_while(|(a, b)| a == b).count();
    let mut at = 0usize;
    while at < shared {
        let len = shared.wrapping_sub(at).min(0x10000);
        // Only the bytes that aren't zero are written, so a copy of 0x10000 has no size.
        let mut op = 0x80u8;
        let mut args = Vec::new();
        let offset = u32::try_from(at).expect("a 32-bit offset").to_le_bytes();
        let length = u32::try_from(len).expect("a 17-bit length").to_le_bytes();
        let fields = offset.iter().zip([0x01, 0x02, 0x04, 0x08]);
        for (&byte, bit) in fields.chain(length.iter().take(2).zip([0x10, 0x20])) {
            if byte != 0 {
                op |= bit;
                args.push(byte);
            }
        }
        out.push(op);
        out.extend_from_slice(&args);
        at = at.wrapping_add(len);
    }
    for chunk in result.get(shared..).unwrap_or_default().chunks(127) {
        out.push(u8::try_from(chunk.len()).expect("at most 127"));
        out.extend_from_slice(chunk);
    }
    out
}

/// A pack entry's header: its type code (1 to 4 for an object stored whole, 6 for an
/// `OFS_DELTA`, 7 for a `REF_DELTA`) and size, in Git's encoding.
#[must_use]
pub fn entry_header(code: u8, size: u64) -> Vec<u8> {
    let mut out = Vec::new();
    let mut byte = code.wrapping_shl(4) | (size & 0x0f) as u8;
    let mut rest = size.wrapping_shr(4);
    while rest > 0 {
        out.push(byte | 0x80);
        byte = (rest & 0x7f) as u8;
        rest = rest.wrapping_shr(7);
    }
    out.push(byte);
    out
}

fn type_code(kind: ObjectType) -> u8 {
    match kind {
        ObjectType::Commit => 1,
        ObjectType::Tree => 2,
        ObjectType::Blob => 3,
        ObjectType::Tag => 4,
    }
}

/// A delta's size field: seven bits a byte, least significant first.
fn size(mut value: u64) -> Vec<u8> {
    let mut out = Vec::new();
    while value >= 0x80 {
        out.push((value & 0x7f) as u8 | 0x80);
        value = value.wrapping_shr(7);
    }
    out.push(value as u8);
    out
}

/// An `OFS_DELTA`'s distance back to its base, in Git's offset encoding: seven bits a byte,
/// most significant first, one taken off before each shift.
fn ofs_distance(mut distance: u64) -> Vec<u8> {
    let mut out = vec![(distance & 0x7f) as u8];
    distance = distance.wrapping_shr(7);
    while distance > 0 {
        distance = distance.wrapping_sub(1);
        out.push(0x80 | (distance & 0x7f) as u8);
        distance = distance.wrapping_shr(7);
    }
    out.reverse();
    out
}

/// `bytes` compressed as Git compresses objects, for `raw_loose`.
#[must_use]
pub fn zlib(bytes: &[u8]) -> Vec<u8> {
    miniz_oxide::deflate::compress_to_vec_zlib(bytes, 6)
}

/// The repository's hash of `bytes`, as packs and indexes end with.
#[must_use]
pub fn digest(hash: HashKind, bytes: &[u8]) -> Vec<u8> {
    let mut hasher = Hasher::new(hash);
    hasher.update(bytes);
    hasher
        .finish(String::new)
        .expect("no SHA-1 collision attack")
        .as_bytes()
        .to_vec()
}

/// zlib's CRC-32, which a pack index keeps for each entry.
fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &byte in bytes {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = crc.wrapping_shr(1) ^ (0xedb8_8320 & (crc & 1).wrapping_neg());
        }
    }
    !crc
}
