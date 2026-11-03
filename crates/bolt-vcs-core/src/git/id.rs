//! Git object IDs: the hash of an object's header and bytes, as Git computes it.
#![deny(clippy::indexing_slicing, clippy::arithmetic_side_effects)]

use std::fmt::Write as _;

// The hashing trait of both hashers (digest 0.10).
use sha2::Digest as _;

use super::{GitError, broken};

/// The hash a repository names its objects with.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub enum HashKind {
    Sha1,
    Sha256,
}

impl HashKind {
    /// Bytes in an object ID.
    #[must_use]
    #[allow(clippy::len_without_is_empty)] // no ID is empty
    pub fn len(self) -> usize {
        match self {
            HashKind::Sha1 => 20,
            HashKind::Sha256 => 32,
        }
    }

    /// Hex digits in an object ID.
    pub(super) fn hex_len(self) -> usize {
        match self {
            HashKind::Sha1 => 40,
            HashKind::Sha256 => 64,
        }
    }
}

/// A Git object's ID: `kind.len()` bytes of `bytes`, the rest zero.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct GitId {
    kind: HashKind,
    bytes: [u8; 32],
}

impl GitId {
    fn new(kind: HashKind, digest: &[u8]) -> GitId {
        let mut bytes = [0; 32];
        bytes.iter_mut().zip(digest).for_each(|(b, d)| *b = *d);
        GitId { kind, bytes }
    }

    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        self.bytes.get(..self.kind.len()).unwrap_or_default()
    }

    /// The all-zero ID, which names no object.
    pub(super) fn is_zero(&self) -> bool {
        self.bytes.iter().all(|&b| b == 0)
    }

    /// The ID in lowercase hex, as Git writes it.
    #[must_use]
    pub fn to_hex(&self) -> String {
        let mut hex = String::with_capacity(self.kind.hex_len());
        for b in self.as_bytes() {
            let _ = write!(hex, "{b:02x}");
        }
        hex
    }

    /// The ID `hex` spells, in either letter case as Git reads it; `None` unless it is exactly
    /// the hex digits of one ID of `kind`.
    #[must_use]
    pub fn from_hex(kind: HashKind, hex: &str) -> Option<GitId> {
        let digits = hex.as_bytes();
        if digits.len() != kind.hex_len() || !digits.iter().all(u8::is_ascii_hexdigit) {
            return None;
        }
        let mut bytes = [0; 32];
        for (byte, pair) in bytes.iter_mut().zip(digits.as_chunks::<2>().0) {
            *byte = u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok()?;
        }
        Some(GitId { kind, bytes })
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ObjectType {
    Commit,
    Tree,
    Blob,
    Tag,
}

impl ObjectType {
    /// The name in an object's header.
    fn name(self) -> &'static str {
        match self {
            ObjectType::Commit => "commit",
            ObjectType::Tree => "tree",
            ObjectType::Blob => "blob",
            ObjectType::Tag => "tag",
        }
    }
}

/// What Git puts before an object's bytes, both to hash and to store: `<type> <length>\0`.
pub(super) fn object_header(kind: ObjectType, len: usize) -> String {
    format!("{} {len}\0", kind.name())
}

/// Git's ID for an object of `kind` holding `data`: the hash of its header and bytes. SHA-1 runs
/// with collision detection, as in Git, and an object made for a collision attack is `Broken`.
pub fn object_id(kind: ObjectType, data: &[u8], hash: HashKind) -> Result<GitId, GitError> {
    let header = object_header(kind, data.len());
    match hash {
        HashKind::Sha1 => {
            let mut hasher = sha1_checked::Sha1::new();
            hasher.update(header.as_bytes());
            hasher.update(data);
            let result = hasher.try_finalize();
            if result.has_collision() {
                return Err(broken(format!(
                    "a {} of {} bytes is part of a SHA-1 collision attack",
                    kind.name(),
                    data.len()
                )));
            }
            Ok(GitId::new(hash, result.hash()))
        }
        HashKind::Sha256 => {
            let mut hasher = sha2::Sha256::new();
            hasher.update(header.as_bytes());
            hasher.update(data);
            Ok(GitId::new(hash, &hasher.finalize()))
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    #![allow(clippy::indexing_slicing, clippy::arithmetic_side_effects)]
    use super::*;
    use crate::git::craft::Craft;
    use std::fs;

    #[test]
    fn object_ids_match_git() {
        // From `git hash-object` and `git mktree --missing`, in SHA-1 and SHA-256 repositories.
        let blob = b"hello\n";
        let rows = [
            (
                HashKind::Sha1,
                "ce013625030ba8dba906f756967f9e9ca394464a",
                "4b825dc642cb6eb9a060e54bf8d69288fbee4904",
                "b4d01e9b0c4a9356736dfddf8830ba9a54f5271c",
            ),
            (
                HashKind::Sha256,
                "2cf8d83d9ee29543b34a87727421fdecb7e3f3a183d337639025de576db9ebb4",
                "6ef19b41225c5369f1c104d45d8d85efa9b057b53b14b4b9b939dd74decc5321",
                "ab39bc840914e6c219053910a617f89e6e0b561cfab5953c79f014c4302f8a01",
            ),
        ];
        for (hash, blob_id, empty_tree, tree) in rows {
            let id = object_id(ObjectType::Blob, blob, hash).unwrap();
            assert_eq!(id.to_hex(), blob_id);
            assert_eq!(id.as_bytes().len(), hash.len());
            assert_eq!(GitId::from_hex(hash, blob_id), Some(id));
            let empty = object_id(ObjectType::Tree, b"", hash).unwrap();
            assert_eq!(empty.to_hex(), empty_tree);
            // Craft stores what Git would: the header and bytes, compressed, under the ID.
            let dir = tempfile::tempdir().unwrap();
            let git = Craft::new(dir.path(), hash);
            assert_eq!(git.object(ObjectType::Blob, blob), id);
            let (folder, file) = blob_id.split_at(2);
            let stored = fs::read(dir.path().join(".git/objects").join(folder).join(file)).unwrap();
            let raw = miniz_oxide::inflate::decompress_to_vec_zlib(&stored).unwrap();
            assert_eq!(raw, b"blob 6\0hello\n");
            assert_eq!(git.tree(&[(b"100644", b"hello", id)]).to_hex(), tree);
        }
    }

    #[test]
    fn ids_are_read_from_hex_as_git_reads_them() {
        let lower = "ce013625030ba8dba906f756967f9e9ca394464a";
        let id = GitId::from_hex(HashKind::Sha1, lower).unwrap();
        assert_eq!(
            GitId::from_hex(HashKind::Sha1, &lower.to_uppercase()),
            Some(id)
        );
        for bad in [
            &lower[..39],
            &format!("{lower}0"),
            &format!("{}g", &lower[..39]),
            &format!("+{}", &lower[..39]),
            "",
        ] {
            assert_eq!(GitId::from_hex(HashKind::Sha1, bad), None, "{bad:?}");
        }
        // An ID of the other hash is not one of this one.
        assert_eq!(GitId::from_hex(HashKind::Sha256, lower), None);
    }
}
