//! Writes Git repositories byte by byte, for tests. Nothing here checks what it writes, so
//! malformed repositories are as easy to make as good ones.
#![deny(clippy::indexing_slicing, clippy::arithmetic_side_effects)]
// Test code: a write that fails should stop the test.
#![allow(clippy::expect_used)]

use std::fs;
use std::path::{Path, PathBuf};

use super::{GitId, HashKind, ObjectType, object_header, object_id};

/// A Git repository a test is writing, at `<dir>/.git`.
pub struct Craft {
    git: PathBuf,
    hash: HashKind,
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
        let mut raw = object_header(kind, bytes.len()).into_bytes();
        raw.extend_from_slice(bytes);
        self.raw_loose(
            &id.to_hex(),
            &miniz_oxide::deflate::compress_to_vec_zlib(&raw, 6),
        );
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
}
