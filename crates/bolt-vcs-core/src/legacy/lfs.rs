//! A Git LFS pointer, read as Git LFS reads one.
#![deny(clippy::indexing_slicing, clippy::arithmetic_side_effects)]

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

/// Most bytes a Git LFS pointer may have (decision 037).
pub const MAX_LFS_POINTER_BYTES: usize = 1024;

/// The `version` Git LFS writes.
pub(crate) const LFS_SPEC: &str = "https://git-lfs.github.com/spec/v1";

/// The `version` values of Git LFS's pointers: its own and its pre-release's.
pub(crate) const LFS_SPECS: [&str; 2] = [LFS_SPEC, "https://hawser.github.com/spec/v1"];

/// The `version` of Git LFS's alpha, which it reads too.
const LFS_ALPHA: &str = "http://git-media.io/v/2";

/// What a Git LFS pointer says: the SHA-256 and size of the file it stands for, and whether it
/// has extension lines, whose programs Git LFS would run to rebuild that file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LfsPointer {
    pub oid: [u8; 32],
    pub size: u64,
    pub extensions: bool,
}

impl LfsPointer {
    /// The oid in lowercase hex, as Git LFS names the object's file.
    #[must_use]
    pub fn oid_hex(&self) -> String {
        let mut hex = String::with_capacity(64);
        for b in self.oid {
            let _ = write!(hex, "{b:02x}");
        }
        hex
    }
}

/// The pointer a blob is, by the test Git LFS's own parser applies (`DecodeFrom` in its
/// lfs/pointer.go): `version`, `oid` and `size` lines in that order, extension lines before
/// `size` and nothing after, blank lines and surrounding whitespace ignored. An empty blob, or
/// one over 1,024 bytes, isn't one.
#[must_use]
pub fn lfs_pointer(data: &[u8]) -> Option<LfsPointer> {
    if data.is_empty() || data.len() > MAX_LFS_POINTER_BYTES {
        return None;
    }
    // Git LFS reads bytes: a U+FFFD where a byte isn't UTF-8 matches no more than the byte did.
    let text = String::from_utf8_lossy(data);
    let mut values: Vec<&str> = Vec::new();
    let mut extensions = BTreeMap::new();
    for line in text.trim().lines().filter(|line| !line.is_empty()) {
        let (key, value) = line.split_once(' ')?;
        if ["version", "oid", "size"].get(values.len())? == &key {
            values.push(value);
        } else if is_lfs_extension(key) {
            // A key given twice keeps its last value, as in Git LFS.
            extensions.insert(key, value);
        } else {
            return None;
        }
    }
    let [version, oid, size] = values[..] else {
        return None;
    };
    let priorities: BTreeSet<_> = extensions.keys().map(|key| key.as_bytes().get(4)).collect();
    let valid = (LFS_SPECS.contains(&version) || version == LFS_ALPHA)
        && extensions.values().all(|oid| lfs_oid(oid).is_some())
        && priorities.len() == extensions.len();
    if !valid {
        return None;
    }
    // Git LFS reads the size as a signed 64-bit number, which can't be negative.
    let size = size.parse::<i64>().ok()?;
    Some(LfsPointer {
        oid: lfs_oid(oid)?,
        size: u64::try_from(size).ok()?,
        extensions: !extensions.is_empty(),
    })
}

/// An extension's key: `ext-`, a digit, `-` and a word character, then anything (Git LFS's
/// `\Aext-\d{1}-\w+`). The digit is its priority.
fn is_lfs_extension(key: &str) -> bool {
    matches!(
        key.as_bytes(),
        [b'e', b'x', b't', b'-', priority, b'-', first, ..]
            if priority.is_ascii_digit() && (first.is_ascii_alphanumeric() || *first == b'_')
    )
}

/// The SHA-256 `value` gives: `sha256:` and 64 lowercase hex digits.
fn lfs_oid(value: &str) -> Option<[u8; 32]> {
    let hex = value.strip_prefix("sha256:")?;
    if hex.len() != 64 || !hex.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
        return None;
    }
    let mut oid = [0; 32];
    for (byte, pair) in oid.iter_mut().zip(hex.as_bytes().as_chunks::<2>().0) {
        *byte = u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok()?;
    }
    Some(oid)
}
