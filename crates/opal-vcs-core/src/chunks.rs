//! Big files are cut into chunks where their contents say, so that versions of a file share most
//! of them, and a chunk list names the chunks that make a file (docs/storage-format.md section 6).
#![deny(clippy::indexing_slicing, clippy::arithmetic_side_effects)]

use std::io::{self, BufReader, ErrorKind, Read, Take, Write};
use std::sync::OnceLock;

use crate::encoding::{DecodeError, Reader, Writer};
use crate::id::Id;

/// A file this long or longer is cut into chunks...
pub const CHUNK_FROM: u64 = 1 << 20;
/// ...up to this long; a longer one is stored whole.
pub const CHUNK_UP_TO: u64 = 1 << 37;
/// Every chunk but a file's last is longer than this.
pub const MIN_CHUNK: usize = 16 << 10;
/// Past this the cutter's test gets easier, which keeps chunks near this long.
pub const AVG_CHUNK: usize = 64 << 10;
/// No chunk is longer.
pub const MAX_CHUNK: usize = 256 << 10;
/// The most chunks one list may hold, as the format allows.
pub const MAX_CHUNKS: u64 = 1 << 23;

/// The hash's top 17 bits, tested before `AVG_CHUNK`...
const MASK_S: u64 = 0xFFFF_8000_0000_0000;
/// ...and its top 15, tested from there on.
const MASK_L: u64 = 0xFFFE_0000_0000_0000;

const GEAR_CONTEXT: &str = "opal-vcs 2026-11-08 chunk-cutter gear v1";

/// The fewest bytes a chunk takes in a list: its ID and a one-byte size.
const LEAST_PER_CHUNK: u64 = 33;

/// Whether a file of `size` bytes is cut into chunks; any other is stored whole.
#[must_use]
pub fn is_cut(size: u64) -> bool {
    let (from, up_to) = cut_sizes();
    (from..=up_to).contains(&size)
}

/// `is_cut`, with files cut from `from` bytes on, unless a test set the sizes on this thread.
#[must_use]
pub(crate) fn is_cut_from(from: u64, size: u64) -> bool {
    #[cfg(any(test, feature = "test-support"))]
    if TEST_CUT_SIZES.get().is_some() {
        return is_cut(size);
    }
    (from..=CHUNK_UP_TO).contains(&size)
}

/// Whether a file of `size` bytes is too long to cut, so it is stored whole.
#[must_use]
pub fn is_too_long_to_cut(size: u64) -> bool {
    size > cut_sizes().1
}

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    static TEST_CUT_SIZES: std::cell::Cell<Option<(u64, u64)>> =
        const { std::cell::Cell::new(None) };
}

/// Makes this thread cut files from `from` up to `up_to` bytes long instead, so a test can turn
/// cutting off or reach its upper limit with small files; `None` goes back. How a file is cut
/// never changes.
#[cfg(any(test, feature = "test-support"))]
pub fn set_test_cut_sizes(sizes: Option<(u64, u64)>) {
    TEST_CUT_SIZES.set(sizes);
}

/// What `set_test_cut_sizes` set on this thread.
#[cfg(any(test, feature = "test-support"))]
pub(crate) fn test_cut_sizes() -> Option<(u64, u64)> {
    TEST_CUT_SIZES.get()
}

/// `CHUNK_FROM` and `CHUNK_UP_TO`, or what a test set on this thread.
fn cut_sizes() -> (u64, u64) {
    #[cfg(any(test, feature = "test-support"))]
    if let Some(sizes) = TEST_CUT_SIZES.get() {
        return sizes;
    }
    (CHUNK_FROM, CHUNK_UP_TO)
}

/// The number the rolling hash adds for each byte value: from BLAKE3, so the spec can state them.
fn gear() -> &'static [u64; 256] {
    static GEAR: OnceLock<[u64; 256]> = OnceLock::new();
    GEAR.get_or_init(|| {
        let mut xof = blake3::Hasher::new_derive_key(GEAR_CONTEXT).finalize_xof();
        let mut table = [0; 256];
        for number in &mut table {
            let mut bytes = [0; 8];
            xof.fill(&mut bytes);
            *number = u64::from_le_bytes(bytes);
        }
        table
    })
}

/// Takes one byte into the rolling hash.
#[allow(clippy::indexing_slicing)] // a byte can't index past the table's 256 numbers
fn roll(h: u64, gear: &[u64; 256], byte: u8) -> u64 {
    h.wrapping_shl(1).wrapping_add(gear[usize::from(byte)])
}

/// Where in `bytes` the hash, carried on in `h`, first has none of `mask`'s bits set.
fn find_cut(h: &mut u64, bytes: &[u8], mask: u64) -> Option<usize> {
    let gear = gear();
    bytes.iter().position(|&byte| {
        *h = roll(*h, gear, byte);
        *h & mask == 0
    })
}

/// Length of the chunk that starts `data`, which holds every byte left or at least `MAX_CHUNK`.
#[must_use]
pub fn chunk_len(data: &[u8]) -> usize {
    if data.len() <= MIN_CHUNK {
        return data.len();
    }
    let end = data.len().min(MAX_CHUNK);
    let normal = end.min(AVG_CHUNK);
    let mut h = 0;
    let before_normal = data.get(MIN_CHUNK..normal).unwrap_or_default();
    if let Some(at) = find_cut(&mut h, before_normal, MASK_S) {
        return MIN_CHUNK.saturating_add(at).saturating_add(1);
    }
    let from_normal = data.get(normal..end).unwrap_or_default();
    match find_cut(&mut h, from_normal, MASK_L) {
        Some(at) => normal.saturating_add(at).saturating_add(1),
        None => end,
    }
}

/// Cuts what a reader gives into chunks, holding at most 2 × `MAX_CHUNK` bytes.
pub struct Cutter<R: Read> {
    reader: R,
    buf: Vec<u8>,
    /// The bytes not cut yet are `buf[start..end]`.
    start: usize,
    end: usize,
    /// The length of the chunk handed out last, cut off on the next call.
    given: usize,
    ended: bool,
}

impl<R: Read> Cutter<R> {
    pub fn new(reader: R) -> Self {
        Cutter {
            reader,
            buf: vec![0; MAX_CHUNK.saturating_mul(2)],
            start: 0,
            end: 0,
            given: 0,
            ended: false,
        }
    }

    /// The next chunk, or `None` once the reader has no more.
    pub fn next_chunk(&mut self) -> io::Result<Option<&[u8]>> {
        self.start = self.start.saturating_add(self.given).min(self.end);
        self.given = 0;
        // A cut can depend on the next `MAX_CHUNK` bytes, so they must all be here, however the
        // reader hands them out.
        if self.end.saturating_sub(self.start) < MAX_CHUNK && !self.ended {
            self.fill()?;
        }
        let rest = self.buf.get(self.start..self.end).unwrap_or_default();
        let len = chunk_len(rest);
        self.given = len;
        Ok(rest.get(..len).filter(|chunk| !chunk.is_empty()))
    }

    pub fn into_inner(self) -> R {
        self.reader
    }

    /// Moves the bytes not cut yet to the front, then reads until the buffer is full or the reader
    /// ends.
    fn fill(&mut self) -> io::Result<()> {
        let end = self.end.min(self.buf.len());
        let start = self.start.min(end);
        self.buf.copy_within(start..end, 0);
        self.start = 0;
        self.end = end.saturating_sub(start);
        while let Some(space) = self.buf.get_mut(self.end..).filter(|s| !s.is_empty()) {
            match self.reader.read(space) {
                Ok(0) => {
                    self.ended = true;
                    break;
                }
                Ok(n) if n <= space.len() => self.end = self.end.saturating_add(n),
                Ok(_) => return Err(io::Error::other("a read gave more bytes than asked for")),
                Err(e) if e.kind() == ErrorKind::Interrupted => {}
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }
}

/// One chunk of a file: the plain BLAKE3 of its bytes, and how many there are.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Chunk {
    pub id: Id,
    pub size: u64,
}

/// The chunks that make a file, in order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChunkList {
    pub file: Id,
    pub size: u64,
    pub chunks: Vec<Chunk>,
}

impl ChunkList {
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        self.pieces().fold(Vec::new(), |mut all, piece| {
            all.extend_from_slice(&piece);
            all
        })
    }

    /// Writes what `encode` gives a piece at a time, so a long list is never held twice.
    pub fn write_to(&self, out: &mut dyn Write) -> io::Result<()> {
        self.pieces().try_for_each(|piece| out.write_all(&piece))
    }

    /// Whether the `len` bytes `reader` gives are what `encode` gives, compared as they are read,
    /// so a long list is never held twice.
    pub fn is_record(&self, reader: &mut dyn Read, len: u64) -> io::Result<bool> {
        let mut left = len;
        let mut got = Vec::new();
        for piece in self.pieces() {
            let Some(rest) = left.checked_sub(piece.len() as u64) else {
                return Ok(false);
            };
            got.resize(piece.len(), 0);
            match reader.read_exact(&mut got) {
                Err(e) if e.kind() == ErrorKind::UnexpectedEof => return Ok(false),
                read => read?,
            }
            if got != piece {
                return Ok(false);
            }
            left = rest;
        }
        Ok(left == 0)
    }

    /// The record's bytes: its head, then its chunks a thousand or so at a time.
    fn pieces(&self) -> impl Iterator<Item = Vec<u8>> + '_ {
        let mut head = Writer::new();
        head.u8(1);
        head.id(&self.file);
        head.varint(self.size);
        head.varint(self.chunks.len() as u64);
        let chunks = self.chunks.chunks(1024).map(|piece| {
            let mut w = Writer::new();
            for chunk in piece {
                w.id(&chunk.id);
                w.varint(chunk.size);
            }
            w.into_bytes()
        });
        std::iter::once(head.into_bytes()).chain(chunks)
    }

    /// Decodes a list by storage-format.md section 6's rules, refusing anything else.
    pub fn decode(bytes: &[u8]) -> Result<ChunkList, DecodeError> {
        let mut rest = bytes;
        ChunkList::read_from(&mut rest, bytes.len() as u64).map_err(|e| match e {
            ReadListError::Damaged(why) => why,
            // Reading a slice can only run out, which is damage already.
            ReadListError::Io(_) => DecodeError::UnexpectedEnd,
            ReadListError::OutOfMemory => {
                DecodeError::Invalid("there is not enough memory for the chunk list")
            }
        })
    }

    /// The same rules, decoding `len` bytes as they are read, never holding them whole.
    pub fn read_from(reader: &mut dyn Read, len: u64) -> Result<ChunkList, ReadListError> {
        let mut r = Pieces {
            src: BufReader::new(reader.take(len)),
            left: len,
        };
        let [version] = r.array()?;
        if version != 1 {
            return Err(DecodeError::Invalid("unknown chunk list version").into());
        }
        let file = Id(r.array()?);
        let size = r.varint()?;
        let count = r.varint()?;
        if count > MAX_CHUNKS {
            return Err(DecodeError::LimitExceeded {
                field: "chunk count",
                limit: MAX_CHUNKS,
            }
            .into());
        }
        if count == 0 {
            return Err(DecodeError::Invalid("a chunk list holds at least one chunk").into());
        }
        // Checked before anything is allocated for them.
        if count.saturating_mul(LEAST_PER_CHUNK) > r.left {
            return Err(
                DecodeError::Invalid("the chunk count is more than the record holds").into(),
            );
        }
        let mut chunks = Vec::new();
        chunks
            .try_reserve_exact(usize::try_from(count).unwrap_or(usize::MAX))
            .map_err(|_| ReadListError::OutOfMemory)?;
        let wrong_sizes = || DecodeError::Invalid("the chunk sizes don't add up to the file size");
        let mut total = 0u64;
        for _ in 0..count {
            let chunk = Chunk {
                id: Id(r.array()?),
                size: r.varint()?,
            };
            if chunk.size == 0 {
                return Err(DecodeError::Invalid("a chunk is at least one byte").into());
            }
            total = total
                .checked_add(chunk.size)
                .filter(|total| *total <= size)
                .ok_or_else(wrong_sizes)?;
            // Within the room reserved above, so this never allocates.
            chunks.push(chunk);
        }
        if total != size {
            return Err(wrong_sizes().into());
        }
        if r.left != 0 {
            return Err(DecodeError::TrailingBytes.into());
        }
        Ok(ChunkList { file, size, chunks })
    }
}

/// Why a chunk list couldn't be read. The store tells damage from a failed read and from too
/// little memory, after which the same list may read fine.
#[derive(Debug, thiserror::Error)]
pub enum ReadListError {
    #[error("the chunk list is damaged: {0}")]
    Damaged(#[from] DecodeError),
    #[error(transparent)]
    Io(io::Error),
    #[error("Opalro ran out of memory while reading a chunk list")]
    OutOfMemory,
}

/// A record's bytes as they are read, counting how many of its length are left.
struct Pieces<'a> {
    src: BufReader<Take<&'a mut dyn Read>>,
    left: u64,
}

impl Pieces<'_> {
    fn array<const N: usize>(&mut self) -> Result<[u8; N], ReadListError> {
        let mut out = [0; N];
        self.src.read_exact(&mut out).map_err(|e| {
            if e.kind() == ErrorKind::UnexpectedEof {
                // The bytes ended before the record did.
                ReadListError::Damaged(DecodeError::UnexpectedEnd)
            } else {
                ReadListError::Io(e)
            }
        })?;
        self.left = self.left.saturating_sub(N as u64);
        Ok(out)
    }

    /// A varint, gathered a byte at a time and read by the format's one rule for them.
    fn varint(&mut self) -> Result<u64, ReadListError> {
        let mut bytes = [0; 10];
        let mut len = 0_usize;
        for slot in &mut bytes {
            let [byte] = self.array()?;
            *slot = byte;
            len = len.saturating_add(1);
            if byte & 0x80 == 0 {
                break;
            }
        }
        Ok(Reader::new(bytes.get(..len).unwrap_or_default()).varint()?)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::indexing_slicing, clippy::arithmetic_side_effects)]
    use super::*;
    use crate::test_support::{noise, unhex};
    use proptest::prelude::*;

    const CHUNKING: &str = include_str!("../tests/vectors/chunking.json");
    const CHUNK_LISTS: &str = include_str!("../tests/vectors/chunk_list.json");

    /// Hands out `data` as many bytes a read as `sizes` says, each read after an interrupted one.
    struct Uneven<'a> {
        data: &'a [u8],
        sizes: Box<dyn FnMut() -> usize>,
        interrupted: bool,
    }

    impl Read for Uneven<'_> {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            self.interrupted = !self.interrupted;
            if self.interrupted {
                return Err(io::ErrorKind::Interrupted.into());
            }
            let n = (self.sizes)().max(1).min(buf.len()).min(self.data.len());
            buf[..n].copy_from_slice(&self.data[..n]);
            self.data = &self.data[n..];
            Ok(n)
        }
    }

    /// Readers of `data` that hand out 1, 7, 4,096, and 1 to 70,000 bytes a read.
    fn uneven(data: &[u8]) -> Vec<Uneven<'_>> {
        let mut x = 0x9e37_79b9_7f4a_7c15_u64;
        let random = move || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            (x % 70_000) as usize + 1
        };
        let sizes: Vec<Box<dyn FnMut() -> usize>> = vec![
            Box::new(|| 1),
            Box::new(|| 7),
            Box::new(|| 4096),
            Box::new(random),
        ];
        sizes
            .into_iter()
            .map(|sizes| Uneven {
                data,
                sizes,
                interrupted: false,
            })
            .collect()
    }

    /// The lengths `chunk_len` gives, cutting `data` from its start.
    fn lens(mut data: &[u8]) -> Vec<usize> {
        let mut out = Vec::new();
        while !data.is_empty() {
            let n = chunk_len(data);
            assert!(n > 0);
            out.push(n);
            data = &data[n..];
        }
        out
    }

    #[test]
    fn a_list_written_or_compared_in_pieces_is_its_record() {
        // More chunks than one piece holds.
        let chunks = (0..2500u32).map(|i| Chunk {
            id: crate::id::hash_content(&i.to_le_bytes()),
            size: u64::from(i % 300) + 1,
        });
        let chunks: Vec<Chunk> = chunks.collect();
        let size = chunks.iter().map(|c| c.size).sum();
        let list = ChunkList {
            file: Id([7; 32]),
            size,
            chunks,
        };
        let record = list.encode();
        let mut written = Vec::new();
        list.write_to(&mut written).unwrap();
        assert_eq!(written, record);
        assert_eq!(ChunkList::decode(&record).unwrap(), list);
        let len = record.len() as u64;
        let is = |bytes: &[u8], len: u64| list.is_record(&mut &*bytes, len).unwrap();
        assert!(is(&record, len));
        // A byte off anywhere, one byte short or one more: another record.
        for at in [0, 33, record.len() / 2, record.len() - 1] {
            let mut other = record.clone();
            other[at] ^= 1;
            assert!(!is(&other, len), "{at}");
        }
        assert!(!is(&record[..record.len() - 1], len - 1));
        assert!(!is(&record[..record.len() - 1], len));
        let mut longer = record.clone();
        longer.push(0);
        assert!(!is(&longer, len + 1));
    }

    #[test]
    fn the_cutters_table_matches_the_spec() {
        let v: serde_json::Value = serde_json::from_str(CHUNKING).unwrap();
        assert_eq!(v["gear_context"], GEAR_CONTEXT);
        let numbers = v["gear"].as_object().unwrap();
        assert_eq!(numbers.len(), 3);
        for (at, number) in numbers {
            let at: usize = at.parse().unwrap();
            let number = u64::from_str_radix(number.as_str().unwrap(), 16).unwrap();
            assert_eq!(gear()[at], number, "GEAR[{at}]");
        }
    }

    #[test]
    fn files_from_1_mib_to_128_gib_are_cut() {
        assert!(!is_cut(1_048_575));
        assert!(is_cut(1_048_576));
        assert!(is_cut(137_438_953_472));
        assert!(!is_cut(137_438_953_473));
        // Every chunk but the last is longer than the minimum, so a list holds any file cut.
        assert_eq!(CHUNK_UP_TO.div_ceil(MIN_CHUNK as u64 + 1), 8_388_097);
        assert_eq!(MAX_CHUNKS, 8_388_608);
    }

    #[test]
    fn the_cutter_cuts_the_same_whatever_the_reads() {
        // Noise around a run of zeros, so some chunks end at the most a chunk may hold.
        let mut data = noise(1, 1_500_000);
        data[400_000..1_000_000].fill(0);
        let expected = lens(&data);
        assert!(expected.contains(&MAX_CHUNK), "{expected:?}");
        for reader in uneven(&data) {
            let mut cutter = Cutter::new(reader);
            let mut at = 0;
            for &n in &expected {
                assert_eq!(cutter.next_chunk().unwrap(), Some(&data[at..at + n]));
                at += n;
            }
            assert_eq!(cutter.next_chunk().unwrap(), None);
            assert_eq!(cutter.next_chunk().unwrap(), None);
            assert_eq!(cutter.buf.len(), 2 * MAX_CHUNK);
            assert!(cutter.into_inner().data.is_empty());
        }
    }

    #[test]
    fn a_count_past_the_bytes_is_refused_before_anything_is_allocated() {
        let header = |count| {
            let mut w = Writer::new();
            w.u8(1);
            w.id(&Id([7; 32]));
            w.varint(1 << 40);
            w.varint(count);
            w.into_bytes()
        };
        let past_the_bytes =
            || DecodeError::Invalid("the chunk count is more than the record holds");
        let over_the_limit = || DecodeError::LimitExceeded {
            field: "chunk count",
            limit: MAX_CHUNKS,
        };
        // Just a header, claiming the most chunks a list may hold, then one more.
        assert_eq!(
            ChunkList::decode(&header(MAX_CHUNKS)),
            Err(past_the_bytes())
        );
        assert_eq!(
            ChunkList::decode(&header(MAX_CHUNKS + 1)),
            Err(over_the_limit())
        );
        // Read as a stream whose length holds one byte too few, far too few, or enough: nothing
        // is read past the header, so nothing is allocated for the chunks it claims.
        for (count, room, refused) in [
            (MAX_CHUNKS, MAX_CHUNKS * 33 - 1, past_the_bytes()),
            (5_000_000, 10 * 33, past_the_bytes()),
            (MAX_CHUNKS + 1, (MAX_CHUNKS + 1) * 33, over_the_limit()),
        ] {
            let bytes = header(count);
            let len = bytes.len() as u64 + room;
            let mut once = Once(Some(bytes));
            match ChunkList::read_from(&mut once, len) {
                Err(ReadListError::Damaged(why)) => assert_eq!(why, refused),
                other => panic!("{count}: {other:?}"),
            }
        }
    }

    /// A disk that fails every read.
    struct Failing;

    impl Read for Failing {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "the disk said no",
            ))
        }
    }

    #[test]
    fn a_read_that_fails_part_way_is_a_read_error_not_damage() {
        let bytes = ChunkList {
            file: Id([1; 32]),
            size: 3,
            chunks: vec![
                Chunk {
                    id: Id([2; 32]),
                    size: 1,
                },
                Chunk {
                    id: Id([3; 32]),
                    size: 2,
                },
            ],
        }
        .encode();
        let len = bytes.len() as u64;
        // The reader fails after 50 bytes: its own error comes back.
        match ChunkList::read_from(&mut (&bytes[..50]).chain(Failing), len) {
            Err(ReadListError::Io(e)) => {
                assert_eq!(e.kind(), io::ErrorKind::PermissionDenied);
                assert_eq!(e.to_string(), "the disk said no");
            }
            other => panic!("{other:?}"),
        }
        // Bytes that end before the length says are damage: the record is cut short.
        let mut short = &bytes[..50];
        match ChunkList::read_from(&mut short, len) {
            Err(ReadListError::Damaged(DecodeError::UnexpectedEnd)) => {}
            other => panic!("{other:?}"),
        }
    }

    /// Gives its bytes in one read, and fails the test if read again.
    struct Once(Option<Vec<u8>>);

    impl Read for Once {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            let bytes = self.0.take().expect("read past the header");
            buf[..bytes.len()].copy_from_slice(&bytes);
            Ok(bytes.len())
        }
    }

    #[test]
    fn read_from_agrees_with_decode_on_every_vector() {
        let v: serde_json::Value = serde_json::from_str(CHUNK_LISTS).unwrap();
        let vectors: Vec<Vec<u8>> = ["valid", "invalid"]
            .iter()
            .flat_map(|kind| v[*kind].as_array().unwrap())
            .map(|vector| unhex(vector["hex"].as_str().unwrap()))
            .collect();
        assert_eq!(vectors.len(), 10);
        for bytes in &vectors {
            let decoded = ChunkList::decode(bytes);
            // A byte past the record's length is never read as part of it.
            let mut longer = bytes.clone();
            longer.push(0);
            for mut reader in uneven(&longer) {
                let read =
                    ChunkList::read_from(&mut reader, bytes.len() as u64).map_err(|e| match e {
                        ReadListError::Damaged(why) => why,
                        other => panic!("{other:?}"),
                    });
                assert_eq!(read, decoded);
            }
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]

        #[test]
        fn every_chunk_but_the_last_is_longer_than_the_minimum(
            seed in any::<u64>(),
            len in 0usize..800_000,
            period in 0usize..4,
        ) {
            // Noise, or a few bytes over and over.
            let mut data = noise(seed, len);
            if period > 0 {
                for i in period..len {
                    data[i] = data[i - period];
                }
            }
            let lens = lens(&data);
            prop_assert_eq!(lens.iter().sum::<usize>(), len);
            prop_assert!(lens.iter().all(|&n| n <= MAX_CHUNK), "{:?}", lens);
            if let Some((_, all_but_the_last)) = lens.split_last() {
                prop_assert!(all_but_the_last.iter().all(|&n| n > MIN_CHUNK), "{:?}", lens);
            }
        }

        #[test]
        fn decode_then_encode_gives_the_same_bytes(
            file in any::<[u8; 32]>(),
            chunks in prop::collection::vec(
                (any::<[u8; 32]>(), prop_oneof![1u64..300, 1u64..1 << 58]),
                1..40,
            ),
        ) {
            let list = ChunkList {
                file: Id(file),
                size: chunks.iter().map(|c| c.1).sum(),
                chunks: chunks.into_iter().map(|(id, size)| Chunk { id: Id(id), size }).collect(),
            };
            let bytes = list.encode();
            let decoded = ChunkList::decode(&bytes).unwrap();
            prop_assert_eq!(decoded.encode(), bytes);
            prop_assert_eq!(decoded, list);
        }
    }
}
