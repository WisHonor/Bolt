//! The index of a version 2 pack: a fan-out table, then the IDs, then a fixed-width line for each,
//! so an object is found by reading a few IDs by position (docs/storage-format.md section 11).
#![deny(clippy::indexing_slicing, clippy::arithmetic_side_effects)]

use super::{ENTRY_HEADER_LEN, IndexEntry, ObjectKind, PACK_HEADER_LEN, PackError, Section};
use crate::encoding::{Reader, Writer};
use crate::id::Id;
use std::cmp::Ordering;
use std::fs::File;
use std::io::{self, BufReader, Read};
use std::path::Path;

/// The count and the fan-out: 8 + 256 × 4 bytes.
const HEAD_LEN: u64 = 1032;
const ID_LEN: u64 = 32;
/// Kind, offset, uncompressed size, stored size.
const LINE_LEN: u64 = 25;

/// How many bytes the index of `count` objects takes.
pub(super) fn index_len(count: u64) -> u64 {
    count
        .saturating_mul(ID_LEN.saturating_add(LINE_LEN))
        .saturating_add(HEAD_LEN)
}

/// The index of `entries`, which are sorted by ID.
pub(super) fn encode<'a>(entries: impl Iterator<Item = &'a IndexEntry> + Clone) -> Vec<u8> {
    let mut fan_out = [0u32; 256];
    let mut count = 0u64;
    for e in entries.clone() {
        count = count.saturating_add(1);
        if let Some(n) = fan_out.get_mut(usize::from(e.id.0[0])) {
            *n = n.saturating_add(1);
        }
    }
    let mut below = 0u32;
    for n in &mut fan_out {
        below = below.saturating_add(*n);
        *n = below;
    }
    let mut w = Writer::new();
    w.u64(count);
    for n in fan_out {
        w.u32(n);
    }
    for e in entries.clone() {
        w.id(&e.id);
    }
    for e in entries {
        w.u8(e.kind as u8);
        w.u64(e.offset);
        w.u64(e.uncompressed_size);
        w.u64(e.stored_size);
    }
    w.into_bytes()
}

/// What opening a version 2 pack read of its index: nothing that grows with its count.
pub(super) struct Index {
    count: u64,
    fan_out: [u32; 256],
    /// Where the index starts, which is where the entry area ends.
    start: u64,
    end: u64,
    ids: u64,
    lines: u64,
    checksum: [u8; 32],
}

fn corrupt(path: &Path, reason: impl Into<String>) -> PackError {
    PackError::Corrupt {
        path: path.to_path_buf(),
        reason: format!("index: {}", reason.into()),
    }
}

/// `N` bytes at `pos`.
fn read_at<const N: usize>(file: &File, pos: u64) -> Result<[u8; N], PackError> {
    let mut out = [0u8; N];
    Section {
        file,
        pos,
        end: pos.saturating_add(N as u64),
    }
    .read_exact(&mut out)?;
    Ok(out)
}

impl Index {
    /// Reads and checks the count and the fan-out of the index between `start` and `end`.
    pub(super) fn open(
        file: &File,
        path: &Path,
        (start, end): (u64, u64),
        checksum: [u8; 32],
        max_objects: u64,
    ) -> Result<Index, PackError> {
        let len = end.saturating_sub(start);
        if len < HEAD_LEN {
            return Err(corrupt(
                path,
                format!("its {len} bytes can't hold a fan-out"),
            ));
        }
        let head: [u8; HEAD_LEN as usize] = read_at(file, start)?;
        let mut r = Reader::new(&head);
        let count = r.u64().map_err(|e| corrupt(path, e.to_string()))?;
        if count > max_objects {
            return Err(corrupt(
                path,
                format!("it lists {count} objects, more than {max_objects}"),
            ));
        }
        let fits = count
            .checked_mul(ID_LEN.saturating_add(LINE_LEN))
            .and_then(|n| n.checked_add(HEAD_LEN));
        if fits != Some(len) {
            return Err(corrupt(
                path,
                format!("its {len} bytes don't fit its entry count of {count}"),
            ));
        }
        let mut fan_out = [0u32; 256];
        let mut below = 0;
        for n in &mut fan_out {
            *n = r.u32().map_err(|e| corrupt(path, e.to_string()))?;
            if *n < below {
                return Err(corrupt(path, "the fan-out decreases"));
            }
            below = *n;
        }
        if u64::from(below) != count {
            return Err(corrupt(
                path,
                format!("the fan-out ends at {below}, not at its count of {count}"),
            ));
        }
        // The length check above bounds both: the index lies inside the file.
        let ids = start.saturating_add(HEAD_LEN);
        let lines = ids.saturating_add(count.saturating_mul(ID_LEN));
        Ok(Index {
            count,
            fan_out,
            start,
            end,
            ids,
            lines,
            checksum,
        })
    }

    /// The range of positions whose IDs start with `byte`.
    fn range(&self, byte: u8) -> (u64, u64) {
        let at = |b: usize| self.fan_out.get(b).map_or(0, |n| u64::from(*n));
        let b = usize::from(byte);
        (b.checked_sub(1).map_or(0, at), at(b))
    }

    /// The line of `id`, by a binary search of the IDs in its fan-out range.
    pub(super) fn find(
        &self,
        file: &File,
        path: &Path,
        id: &Id,
    ) -> Result<Option<IndexEntry>, PackError> {
        let (mut lo, mut hi) = self.range(id.0[0]);
        while lo < hi {
            let mid = lo.saturating_add(hi.saturating_sub(lo) / 2);
            let at = Id(read_at(file, self.id_pos(mid))?);
            match at.cmp(id) {
                Ordering::Less => lo = mid.saturating_add(1),
                Ordering::Greater => hi = mid,
                Ordering::Equal => {
                    let line = read_at(file, self.line_pos(mid))?;
                    return self.entry(path, *id, &line).map(Some);
                }
            }
        }
        Ok(None)
    }

    // Positions below the count, which the length check at open keeps inside the index.
    fn id_pos(&self, i: u64) -> u64 {
        self.ids.saturating_add(i.saturating_mul(ID_LEN))
    }

    fn line_pos(&self, i: u64) -> u64 {
        self.lines.saturating_add(i.saturating_mul(LINE_LEN))
    }

    /// The line of `id`, checked: a known kind, a size within its limit, an entry inside the entry
    /// area.
    fn entry(
        &self,
        path: &Path,
        id: Id,
        line: &[u8; LINE_LEN as usize],
    ) -> Result<IndexEntry, PackError> {
        let mut r = Reader::new(line);
        let fields = (r.u8(), r.u64(), r.u64(), r.u64());
        let (Ok(kind), Ok(offset), Ok(uncompressed_size), Ok(stored_size)) = fields else {
            return Err(corrupt(path, "a line is cut short"));
        };
        let kind = ObjectKind::from_u8(kind)
            .ok_or_else(|| corrupt(path, format!("unknown object kind {kind}")))?;
        if !kind.sizes_allowed(uncompressed_size, stored_size) {
            return Err(corrupt(path, format!("{id} is larger than allowed")));
        }
        let entry_end = offset
            .checked_add(ENTRY_HEADER_LEN)
            .and_then(|o| o.checked_add(stored_size));
        if offset < PACK_HEADER_LEN || entry_end.is_none_or(|end| end > self.start) {
            return Err(corrupt(path, format!("{id} lies outside the entry area")));
        }
        Ok(IndexEntry {
            id,
            kind,
            offset,
            uncompressed_size,
            stored_size,
        })
    }

    /// Every line in order, each checked as `find` checks it, and also that the IDs ascend and
    /// sit in their fan-out ranges.
    pub(super) fn lines<'a>(&'a self, file: &'a File, path: &'a Path) -> Lines<'a> {
        let section = |pos, end| BufReader::new(Section { file, pos, end });
        Lines {
            index: self,
            path,
            ids: section(self.ids, self.lines),
            lines: section(self.lines, self.end),
            next: 0,
            prev: None,
            failed: false,
        }
    }

    /// Checks the index against the checksum in the trailer, reading it whole.
    pub(super) fn check_checksum(&self, file: &File, path: &Path) -> Result<(), PackError> {
        let mut hasher = blake3::Hasher::new();
        let mut index = Section {
            file,
            pos: self.start,
            end: self.end,
        };
        io::copy(&mut index, &mut hasher)?;
        if *hasher.finalize().as_bytes() != self.checksum {
            return Err(corrupt(path, "the checksum does not match"));
        }
        Ok(())
    }
}

pub(super) struct Lines<'a> {
    index: &'a Index,
    path: &'a Path,
    ids: BufReader<Section<'a>>,
    lines: BufReader<Section<'a>>,
    next: u64,
    prev: Option<Id>,
    failed: bool,
}

impl Lines<'_> {
    fn read_next(&mut self) -> Result<IndexEntry, PackError> {
        let path = self.path;
        let mut id = [0u8; ID_LEN as usize];
        let mut line = [0u8; LINE_LEN as usize];
        self.ids.read_exact(&mut id)?;
        self.lines.read_exact(&mut line)?;
        let id = Id(id);
        if self.prev.is_some_and(|prev| prev >= id) {
            return Err(corrupt(path, "entries are not sorted by id, or repeat one"));
        }
        let (lo, hi) = self.index.range(id.0[0]);
        if self.next < lo || self.next >= hi {
            return Err(corrupt(path, format!("{id} is outside its fan-out range")));
        }
        self.prev = Some(id);
        self.next = self.next.saturating_add(1);
        self.index.entry(path, id, &line)
    }
}

impl Iterator for Lines<'_> {
    type Item = Result<IndexEntry, PackError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.failed || self.next >= self.index.count {
            return None;
        }
        let read = self.read_next();
        self.failed = read.is_err();
        Some(read)
    }
}
