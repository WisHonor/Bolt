//! A Git LFS pointer, read as Git LFS reads one.

use std::fmt::Write as _;

use super::split_at_byte;

/// The longest blob read as a pointer: Git LFS reads no more than this many bytes of one.
pub const MAX_LFS_POINTER_BYTES: usize = 1024;

/// The version Git LFS writes in its pointers.
pub(crate) const LFS_SPEC: &str = "https://git-lfs.github.com/spec/v1";

/// The versions of Git LFS's own pointer format: today's and its pre-release's.
pub(crate) const LFS_SPECS: [&str; 2] = [LFS_SPEC, "https://hawser.github.com/spec/v1"];

/// Git LFS's alpha's version, which its reader still accepts.
const ALPHA_SPEC: &str = "http://git-media.io/v/2";

/// What a Git LFS pointer says.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LfsPointer {
    /// The SHA-256 of the file it stands for.
    pub oid: [u8; 32],
    /// The file's size in bytes.
    pub size: u64,
    /// Whether it names any `ext-…` extension, a program the file went through.
    pub extensions: bool,
}

impl LfsPointer {
    /// The oid as 64 lowercase hex digits.
    #[must_use]
    pub fn oid_hex(&self) -> String {
        let mut hex = String::with_capacity(64);
        for byte in self.oid {
            let _ = write!(hex, "{byte:02x}");
        }
        hex
    }
}

/// `data` as a pointer, exactly when Git LFS reads it as one (not strictly). Whitespace around
/// it goes, lines end at LF with one CR before it dropped, and empty lines are skipped. The rest
/// is `version`, `oid` and `size` in that order, each key and value split by one space, with
/// `ext-<digit>-<name>` lines anywhere before the size, at most one name per digit.
#[must_use]
pub fn lfs_pointer(data: &[u8]) -> Option<LfsPointer> {
    if data.len() > MAX_LFS_POINTER_BYTES {
        return None;
    }
    let lines = trim_space(data)
        .split(|&b| b == b'\n')
        .map(|line| line.strip_suffix(b"\r").unwrap_or(line))
        .filter(|line| !line.is_empty());
    let mut names: [Option<&[u8]>; 10] = [None; 10];
    let (mut version, mut oid, mut size) = (false, None, None);
    for line in lines {
        if size.is_some() {
            return None;
        }
        let (key, value) = split_at_byte(line, b' ')?;
        if let Some((digit, name)) = extension(key) {
            sha256(value)?;
            let slot = names.get_mut(digit)?;
            if slot.is_some_and(|other| other != name) {
                return None;
            }
            *slot = Some(name);
            continue;
        }
        match key {
            b"version" if !version && is_version(value) => version = true,
            b"oid" if version && oid.is_none() => oid = Some(sha256(value)?),
            b"size" if oid.is_some() => size = Some(parse_size(value)?),
            _ => return None,
        }
    }
    Some(LfsPointer {
        oid: oid?,
        size: size?,
        extensions: names.iter().any(Option::is_some),
    })
}

fn is_version(value: &[u8]) -> bool {
    LFS_SPECS
        .iter()
        .chain([&ALPHA_SPEC])
        .any(|spec| value == spec.as_bytes())
}

/// `ext-`, a digit, `-` and a name that starts with an ASCII letter, digit or `_`: the digit and
/// the name.
fn extension(key: &[u8]) -> Option<(usize, &[u8])> {
    match key.strip_prefix(b"ext-")? {
        [digit, b'-', name @ ..]
            if name
                .first()
                .is_some_and(|&b| b.is_ascii_alphanumeric() || b == b'_') =>
        {
            let digit = char::from(*digit).to_digit(10)?;
            Some((usize::try_from(digit).ok()?, name))
        }
        _ => None,
    }
}

/// `sha256:` and 64 lowercase hex digits, as bytes.
fn sha256(value: &[u8]) -> Option<[u8; 32]> {
    let hex = value.strip_prefix(b"sha256:")?;
    if hex.len() != 64 || !hex.iter().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
        return None;
    }
    let mut oid = [0u8; 32];
    for (byte, pair) in oid.iter_mut().zip(hex.as_chunks::<2>().0) {
        *byte = u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok()?;
    }
    Some(oid)
}

/// A decimal number with an optional sign that fits a signed 64-bit integer, and isn't
/// negative (`-0` is zero).
fn parse_size(value: &[u8]) -> Option<u64> {
    let size: i64 = std::str::from_utf8(value).ok()?.parse().ok()?;
    u64::try_from(size).ok()
}

/// `data` without Unicode whitespace at either end. A byte that isn't UTF-8 stops the trimming,
/// as it is no whitespace.
fn trim_space(data: &[u8]) -> &[u8] {
    let trimmed = |valid: &str, kept: &str| valid.len().saturating_sub(kept.len());
    let start = data.utf8_chunks().next().map_or(0, |chunk| {
        trimmed(chunk.valid(), chunk.valid().trim_start())
    });
    let end = match data.utf8_chunks().last() {
        Some(chunk) if chunk.invalid().is_empty() => data
            .len()
            .saturating_sub(trimmed(chunk.valid(), chunk.valid().trim_end())),
        _ => data.len(),
    };
    data.get(start..end).unwrap_or_default()
}

#[cfg(test)]
mod tests;
