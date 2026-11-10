//! Strict byte encoding building blocks (docs/storage-format.md section 1).

use crate::id::{ChangeId, Id};
use crate::names::NameError;

/// Why a byte string was rejected.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum DecodeError {
    #[error("the data ended before the record was complete")]
    UnexpectedEnd,
    #[error("there are extra bytes after the end of the record")]
    TrailingBytes,
    #[error("a number is written in a longer form than necessary")]
    NonMinimalVarint,
    #[error("a number is too large to fit in 64 bits")]
    VarintOverflow,
    #[error("a yes/no value is {0:#04x}, but only 0x00 and 0x01 are allowed")]
    InvalidBool(u8),
    #[error("text is not valid UTF-8")]
    InvalidUtf8,
    #[error("{field} is larger than the allowed maximum of {limit}")]
    LimitExceeded { field: &'static str, limit: u64 },
    #[error("invalid value: {0}")]
    Invalid(&'static str),
    #[error("invalid name: {0}")]
    InvalidName(#[from] NameError),
}

/// Appends encoded values to a growing buffer.
#[derive(Debug, Default)]
pub struct Writer {
    buf: Vec<u8>,
}

impl Writer {
    #[must_use]
    pub fn new() -> Writer {
        Writer::default()
    }

    /// A writer whose buffer holds `capacity` bytes before it grows.
    #[must_use]
    pub fn with_capacity(capacity: usize) -> Writer {
        Writer {
            buf: Vec::with_capacity(capacity),
        }
    }

    /// A writer that appends to `buf`, using any room already reserved in it.
    #[must_use]
    pub(crate) fn from_vec(buf: Vec<u8>) -> Writer {
        Writer { buf }
    }

    pub fn u8(&mut self, v: u8) {
        self.buf.push(v);
    }

    pub fn u32(&mut self, v: u32) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }

    pub fn u64(&mut self, v: u64) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }

    pub fn i16(&mut self, v: i16) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }

    pub fn i64(&mut self, v: i64) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }

    /// Unsigned LEB128, always in the shortest form.
    pub fn varint(&mut self, mut v: u64) {
        while v >= 0x80 {
            self.buf.push((v & 0x7f) as u8 | 0x80);
            v >>= 7;
        }
        self.buf.push(v as u8);
    }

    pub fn bool(&mut self, v: bool) {
        self.buf.push(u8::from(v));
    }

    pub fn bytes(&mut self, v: &[u8]) {
        self.varint(v.len() as u64);
        self.buf.extend_from_slice(v);
    }

    pub fn string(&mut self, v: &str) {
        self.bytes(v.as_bytes());
    }

    pub fn id(&mut self, v: &Id) {
        self.buf.extend_from_slice(&v.0);
    }

    pub fn change_id(&mut self, v: &ChangeId) {
        self.buf.extend_from_slice(&v.0);
    }

    #[must_use]
    pub fn into_bytes(self) -> Vec<u8> {
        self.buf
    }
}

/// How many bytes `Writer::varint` writes for `v`: seven bits a byte, and at least one.
#[must_use]
pub fn varint_len(v: u64) -> u64 {
    u64::from((u64::BITS - v.leading_zeros()).div_ceil(7).max(1))
}

/// Reads encoded values, rejecting anything that is not the one valid encoding.
#[derive(Debug)]
pub struct Reader<'a> {
    rest: &'a [u8],
}

impl<'a> Reader<'a> {
    #[must_use]
    pub fn new(input: &'a [u8]) -> Reader<'a> {
        Reader { rest: input }
    }

    /// How many bytes are left, to check a count against before allocating for it.
    #[must_use]
    pub fn remaining(&self) -> usize {
        self.rest.len()
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], DecodeError> {
        if n > self.rest.len() {
            return Err(DecodeError::UnexpectedEnd);
        }
        let (head, tail) = self.rest.split_at(n);
        self.rest = tail;
        Ok(head)
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], DecodeError> {
        let mut out = [0u8; N];
        out.copy_from_slice(self.take(N)?);
        Ok(out)
    }

    pub fn u8(&mut self) -> Result<u8, DecodeError> {
        Ok(self.array::<1>()?[0])
    }

    pub fn u32(&mut self) -> Result<u32, DecodeError> {
        Ok(u32::from_le_bytes(self.array()?))
    }

    pub fn u64(&mut self) -> Result<u64, DecodeError> {
        Ok(u64::from_le_bytes(self.array()?))
    }

    pub fn i16(&mut self) -> Result<i16, DecodeError> {
        Ok(i16::from_le_bytes(self.array()?))
    }

    pub fn i64(&mut self) -> Result<i64, DecodeError> {
        Ok(i64::from_le_bytes(self.array()?))
    }

    pub fn varint(&mut self) -> Result<u64, DecodeError> {
        let mut value = 0u64;
        for i in 0..10 {
            let byte = self.u8()?;
            if i == 9 && byte > 1 {
                return Err(DecodeError::VarintOverflow);
            }
            if i > 0 && byte == 0 {
                return Err(DecodeError::NonMinimalVarint);
            }
            value |= u64::from(byte & 0x7f) << (7 * i);
            if byte & 0x80 == 0 {
                return Ok(value);
            }
        }
        Err(DecodeError::VarintOverflow)
    }

    /// A varint that must be at most `limit`.
    pub fn count(&mut self, limit: u64, field: &'static str) -> Result<u64, DecodeError> {
        let n = self.varint()?;
        if n > limit {
            return Err(DecodeError::LimitExceeded { field, limit });
        }
        Ok(n)
    }

    pub fn bool(&mut self) -> Result<bool, DecodeError> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            other => Err(DecodeError::InvalidBool(other)),
        }
    }

    /// Checks the length against `limit`, then against the remaining input, before reading.
    pub fn bytes(&mut self, limit: usize, field: &'static str) -> Result<&'a [u8], DecodeError> {
        let limit64 = limit as u64;
        let len = self.count(limit64, field)?;
        // `len <= limit`, so it fits in usize.
        self.take(usize::try_from(len).map_err(|_| DecodeError::Invalid(field))?)
    }

    pub fn string(&mut self, limit: usize, field: &'static str) -> Result<&'a str, DecodeError> {
        std::str::from_utf8(self.bytes(limit, field)?).map_err(|_| DecodeError::InvalidUtf8)
    }

    pub fn id(&mut self) -> Result<Id, DecodeError> {
        Ok(Id(self.array()?))
    }

    pub fn change_id(&mut self) -> Result<ChangeId, DecodeError> {
        Ok(ChangeId(self.array()?))
    }

    /// Fails if any input is left over.
    pub fn finish(self) -> Result<(), DecodeError> {
        if self.rest.is_empty() {
            Ok(())
        } else {
            Err(DecodeError::TrailingBytes)
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn enc(v: u64) -> Vec<u8> {
        let mut w = Writer::new();
        w.varint(v);
        w.into_bytes()
    }

    #[test]
    fn varint_known_encodings() {
        assert_eq!(enc(0), [0x00]);
        assert_eq!(enc(127), [0x7f]);
        assert_eq!(enc(128), [0x80, 0x01]);
        assert_eq!(enc(300), [0xac, 0x02]);
        assert_eq!(
            enc(u64::MAX),
            [0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x01]
        );
    }

    #[test]
    fn a_writer_from_a_vector_appends_after_what_it_held() {
        let mut held = Vec::with_capacity(16);
        held.extend_from_slice(b"ab");
        let mut w = Writer::from_vec(held);
        w.varint(300);
        assert_eq!(w.into_bytes(), [b'a', b'b', 0xac, 0x02]);
    }

    #[test]
    fn varint_len_is_what_the_writer_writes() {
        for shift in 0..64 {
            let v = 1u64 << shift;
            for v in [v - 1, v, v + 1, u64::MAX] {
                assert_eq!(varint_len(v), enc(v).len() as u64, "{v}");
            }
        }
    }

    #[test]
    fn varint_rejects_non_minimal_forms_and_overflow() {
        assert_eq!(
            Reader::new(&[0x80, 0x00]).varint(),
            Err(DecodeError::NonMinimalVarint)
        );
        assert_eq!(
            Reader::new(&[0xff, 0x00]).varint(),
            Err(DecodeError::NonMinimalVarint)
        );
        let mut too_big = vec![0xff; 9];
        too_big.push(0x02);
        assert_eq!(
            Reader::new(&too_big).varint(),
            Err(DecodeError::VarintOverflow)
        );
        assert_eq!(
            Reader::new(&[0xff; 10]).varint(),
            Err(DecodeError::VarintOverflow)
        );
        assert_eq!(
            Reader::new(&[0x80]).varint(),
            Err(DecodeError::UnexpectedEnd)
        );
    }

    #[test]
    fn fixed_width_is_little_endian() {
        let mut w = Writer::new();
        w.u32(1);
        w.i16(-2);
        w.i64(-1);
        assert_eq!(
            w.into_bytes(),
            [
                1, 0, 0, 0, 0xfe, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff
            ]
        );
    }

    #[test]
    fn readers_are_strict() {
        assert_eq!(
            Reader::new(&[0x02]).bool(),
            Err(DecodeError::InvalidBool(2))
        );
        assert_eq!(
            Reader::new(&[0x01, 0xff]).string(10, "name"),
            Err(DecodeError::InvalidUtf8)
        );
        assert_eq!(
            Reader::new(&[0x05, b'a']).bytes(10, "value"),
            Err(DecodeError::UnexpectedEnd)
        );
        assert_eq!(
            Reader::new(&[0x03, b'a', b'b', b'c']).string(2, "name"),
            Err(DecodeError::LimitExceeded {
                field: "name",
                limit: 2
            })
        );
        assert_eq!(
            Reader::new(&[0x0b]).count(10, "entries"),
            Err(DecodeError::LimitExceeded {
                field: "entries",
                limit: 10
            })
        );
        let mut r = Reader::new(&[0x00, 0x00]);
        r.u8().unwrap();
        assert_eq!(r.finish(), Err(DecodeError::TrailingBytes));
    }

    proptest! {
        #[test]
        fn varints_round_trip(v in any::<u64>()) {
            let bytes = enc(v);
            let mut r = Reader::new(&bytes);
            prop_assert_eq!(r.varint(), Ok(v));
            prop_assert_eq!(r.finish(), Ok(()));
        }

        #[test]
        fn strings_round_trip(s in ".{0,64}") {
            let mut w = Writer::new();
            w.string(&s);
            let bytes = w.into_bytes();
            prop_assert_eq!(Reader::new(&bytes).string(1024, "s"), Ok(s.as_str()));
        }
    }
}
