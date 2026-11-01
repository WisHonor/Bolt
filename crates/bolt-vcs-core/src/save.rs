//! Saves (docs/storage-format.md section 4).

use std::collections::BTreeMap;

use crate::encoding::{DecodeError, Reader, Writer};
use crate::id::{ChangeId, Flavor, Id, hash_record};

pub const MAX_PARENTS: u64 = 1024;
pub const MAX_MESSAGE_BYTES: usize = 1 << 20;
pub const MAX_MOVE_NOTES: u64 = 100_000;
pub const MAX_EXTRA_FIELDS: u64 = 64;
pub const MAX_EXTRA_VALUE_BYTES: usize = 64 * 1024;
pub const MAX_PERSON_FIELD_BYTES: usize = 4096;
pub const MAX_PATH_BYTES: usize = 4096;
pub const MAX_SIGNATURE_BYTES: usize = 64 * 1024;
pub const MAX_EXTRA_KEY_BYTES: usize = 256;
pub const EXTRA_LINE: &str = "line";

/// Who did something, and when.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Person {
    pub name: String,
    pub email: String,
    pub time_ms: i64,
    pub tz_offset_min: i16,
}

/// A note that a file moved from one path to another in this save.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MoveNote {
    pub from: String,
    pub to: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Save {
    pub root: Id,
    pub parents: Vec<Id>,
    pub author: Person,
    pub saver: Person,
    pub message: String,
    pub change_id: ChangeId,
    /// The `to` paths must be unique; a save with duplicates encodes but will not decode.
    pub move_notes: Vec<MoveNote>,
    pub signature: Vec<u8>,
    pub extra: BTreeMap<String, Vec<u8>>,
}

pub(crate) fn write_person(w: &mut Writer, p: &Person) {
    w.string(&p.name);
    w.string(&p.email);
    w.i64(p.time_ms);
    w.i16(p.tz_offset_min);
}

pub(crate) fn read_person(r: &mut Reader) -> Result<Person, DecodeError> {
    Ok(Person {
        name: r.string(MAX_PERSON_FIELD_BYTES, "person name")?.to_owned(),
        email: r.string(MAX_PERSON_FIELD_BYTES, "person email")?.to_owned(),
        time_ms: r.i64()?,
        tz_offset_min: r.i16()?,
    })
}

impl Save {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        w.u8(1);
        w.id(&self.root);
        w.varint(self.parents.len() as u64);
        for p in &self.parents {
            w.id(p);
        }
        write_person(&mut w, &self.author);
        write_person(&mut w, &self.saver);
        w.string(&self.message);
        w.change_id(&self.change_id);
        let mut notes: Vec<&MoveNote> = self.move_notes.iter().collect();
        notes.sort_by(|a, b| a.to.cmp(&b.to));
        w.varint(notes.len() as u64);
        for n in notes {
            w.string(&n.from);
            w.string(&n.to);
        }
        w.bytes(&self.signature);
        w.varint(self.extra.len() as u64);
        for (k, v) in &self.extra {
            w.string(k);
            w.bytes(v);
        }
        w.into_bytes()
    }

    pub fn decode(bytes: &[u8], signatures_allowed: bool) -> Result<Save, DecodeError> {
        let mut r = Reader::new(bytes);
        if r.u8()? != 1 {
            return Err(DecodeError::Invalid("unknown save version"));
        }
        let root = r.id()?;
        let n = r.count(MAX_PARENTS, "parent count")?;
        let mut parents: Vec<Id> = Vec::new();
        for _ in 0..n {
            let p = r.id()?;
            if parents.contains(&p) {
                return Err(DecodeError::Invalid("duplicate parent"));
            }
            parents.push(p);
        }
        let author = read_person(&mut r)?;
        let saver = read_person(&mut r)?;
        let message = r.string(MAX_MESSAGE_BYTES, "message")?.to_owned();
        let change_id = r.change_id()?;
        let n = r.count(MAX_MOVE_NOTES, "move-note count")?;
        let mut move_notes: Vec<MoveNote> = Vec::new();
        for _ in 0..n {
            let from = r.string(MAX_PATH_BYTES, "move-note path")?.to_owned();
            let to = r.string(MAX_PATH_BYTES, "move-note path")?.to_owned();
            if move_notes.last().is_some_and(|l| l.to >= to) {
                return Err(DecodeError::Invalid(
                    "move notes not strictly sorted by `to`",
                ));
            }
            move_notes.push(MoveNote { from, to });
        }
        let signature = r.bytes(MAX_SIGNATURE_BYTES, "signature")?;
        if !signature.is_empty() && !signatures_allowed {
            return Err(DecodeError::Invalid(
                "signature without the signatures feature",
            ));
        }
        let signature = signature.to_vec();
        let n = r.count(MAX_EXTRA_FIELDS, "extra-field count")?;
        let mut extra = BTreeMap::new();
        let mut prev: Option<String> = None;
        for _ in 0..n {
            let key = r.string(MAX_EXTRA_KEY_BYTES, "extra-field key")?.to_owned();
            if prev.as_ref().is_some_and(|p| *p >= key) {
                return Err(DecodeError::Invalid("extra-field keys not strictly sorted"));
            }
            let value = r
                .bytes(MAX_EXTRA_VALUE_BYTES, "extra-field value")?
                .to_vec();
            prev = Some(key.clone());
            extra.insert(key, value);
        }
        r.finish()?;
        Ok(Save {
            root,
            parents,
            author,
            saver,
            message,
            change_id,
            move_notes,
            signature,
            extra,
        })
    }

    pub fn id(&self) -> Id {
        hash_record(Flavor::Save, &self.encode())
    }

    /// The line this save was made on, if the field is present and UTF-8.
    pub fn line(&self) -> Option<&str> {
        std::str::from_utf8(self.extra.get(EXTRA_LINE)?).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{hex, unhex};
    use proptest::prelude::*;

    fn person() -> Person {
        Person {
            name: "A".into(),
            email: "a@b".into(),
            time_ms: 0,
            tz_offset_min: 0,
        }
    }

    fn minimal() -> Save {
        Save {
            root: Id([0; 32]),
            parents: vec![],
            author: person(),
            saver: person(),
            message: "m".into(),
            change_id: ChangeId([0; 16]),
            move_notes: vec![],
            signature: vec![],
            extra: BTreeMap::new(),
        }
    }

    #[test]
    fn minimal_save_bytes() {
        let p = format!("0141{}{}{}", "03614062", "00".repeat(8), "0000");
        let expected = format!(
            "01{}00{}{}016d{}000000",
            "00".repeat(32),
            p,
            p,
            "00".repeat(16)
        );
        assert_eq!(hex(&minimal().encode()), expected);
    }

    #[test]
    fn decode_rejects_bad_saves() {
        let mut s = minimal();
        s.parents = vec![Id([1; 32]), Id([1; 32])];
        assert!(Save::decode(&s.encode(), false).is_err()); // duplicate parents
        let mut s = minimal();
        s.signature = vec![1, 2, 3];
        assert!(Save::decode(&s.encode(), false).is_err()); // signature without the feature
        assert!(Save::decode(&s.encode(), true).is_ok());
        let mut s = minimal();
        s.message = "x".repeat(MAX_MESSAGE_BYTES + 1);
        assert!(matches!(
            Save::decode(&s.encode(), false),
            Err(DecodeError::LimitExceeded { .. })
        ));
        let mut s = minimal();
        s.parents = vec![Id([2; 32]); 1025];
        assert!(matches!(
            Save::decode(&s.encode(), false),
            Err(DecodeError::LimitExceeded { .. })
        ));
        let mut bytes = minimal().encode();
        bytes.pop(); // drop "0 extra fields"
        bytes.extend(unhex("02016200016100")); // keys "b" then "a"
        assert!(Save::decode(&bytes, false).is_err());
    }

    #[test]
    fn duplicate_move_note_targets_are_rejected_on_decode() {
        let mut s = minimal();
        s.move_notes = vec![
            MoveNote {
                from: "a".into(),
                to: "c".into(),
            },
            MoveNote {
                from: "b".into(),
                to: "c".into(),
            },
        ];
        assert!(Save::decode(&s.encode(), false).is_err());
    }

    #[test]
    fn unknown_version_is_rejected() {
        let mut bytes = minimal().encode();
        bytes[0] = 2;
        assert!(Save::decode(&bytes, false).is_err());
    }

    #[test]
    fn line_extra_field() {
        let mut s = minimal();
        s.extra.insert(EXTRA_LINE.into(), b"main".to_vec());
        assert_eq!(
            Save::decode(&s.encode(), false).unwrap().line(),
            Some("main")
        );
    }

    proptest! {
        #[test]
        fn saves_round_trip(
            parents in prop::collection::btree_set(any::<[u8; 32]>(), 0..8),
            message in ".{0,100}",
            cid in any::<[u8; 16]>(),
            extra in prop::collection::btree_map("[a-z-]{1,20}", prop::collection::vec(any::<u8>(), 0..50), 0..6),
            time in any::<i64>(),
            tz in any::<i16>(),
        ) {
            let mut s = minimal();
            s.parents = parents.into_iter().map(Id).collect();
            s.message = message;
            s.change_id = ChangeId(cid);
            s.extra = extra;
            s.author.time_ms = time;
            s.saver.tz_offset_min = tz;
            s.move_notes = vec![MoveNote { from: "a".into(), to: "b".into() }];
            prop_assert_eq!(Save::decode(&s.encode(), false).unwrap(), s);
        }
    }
}
