//! Folder listings (docs/storage-format.md section 3).

use crate::encoding::{DecodeError, Reader, Writer};
use crate::id::{Flavor, Id, hash_record};
use crate::names::{MAX_NAME_BYTES, validate_name};

/// Most entries one listing may hold.
pub const MAX_ENTRIES: u64 = 1_000_000;

/// What a listing entry points to.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    File = 1,
    Executable = 2,
    Symlink = 3,
    Folder = 4,
    ProjectLink = 5,
    Conflict = 6,
}

impl EntryKind {
    pub fn from_u8(v: u8) -> Option<EntryKind> {
        Some(match v {
            1 => EntryKind::File,
            2 => EntryKind::Executable,
            3 => EntryKind::Symlink,
            4 => EntryKind::Folder,
            5 => EntryKind::ProjectLink,
            6 => EntryKind::Conflict,
            _ => return None,
        })
    }

    pub fn as_str(self) -> &'static str {
        match self {
            EntryKind::File => "file",
            EntryKind::Executable => "executable",
            EntryKind::Symlink => "symlink",
            EntryKind::Folder => "folder",
            EntryKind::ProjectLink => "project-link",
            EntryKind::Conflict => "conflict",
        }
    }

    fn has_size(self) -> bool {
        matches!(
            self,
            EntryKind::File | EntryKind::Executable | EntryKind::Symlink
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub kind: EntryKind,
    pub id: Id,
    pub size: u64,
}

impl Entry {
    fn check(&self) -> Result<(), DecodeError> {
        validate_name(&self.name)?;
        if !self.kind.has_size() && self.size != 0 {
            return Err(DecodeError::Invalid(
                "size must be 0 for this kind of entry",
            ));
        }
        Ok(())
    }
}

/// One folder's contents; always sorted by name bytes, unique and valid.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listing {
    entries: Vec<Entry>,
}

impl Listing {
    pub fn empty() -> Listing {
        Listing {
            entries: Vec::new(),
        }
    }

    pub fn new(mut entries: Vec<Entry>) -> Result<Listing, DecodeError> {
        if entries.len() as u64 > MAX_ENTRIES {
            return Err(DecodeError::LimitExceeded {
                field: "entry count",
                limit: MAX_ENTRIES,
            });
        }
        for e in &entries {
            e.check()?;
        }
        entries.sort_by(|a, b| a.name.as_bytes().cmp(b.name.as_bytes()));
        if entries.windows(2).any(|w| w[0].name == w[1].name) {
            return Err(DecodeError::Invalid("duplicate name"));
        }
        Ok(Listing { entries })
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    pub fn get(&self, name: &str) -> Option<&Entry> {
        self.entries
            .binary_search_by(|e| e.name.as_bytes().cmp(name.as_bytes()))
            .ok()
            .map(|i| &self.entries[i])
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        w.u8(1);
        w.varint(self.entries.len() as u64);
        for e in &self.entries {
            w.string(&e.name);
            w.u8(e.kind as u8);
            w.id(&e.id);
            w.varint(e.size);
        }
        w.into_bytes()
    }

    /// Rejects unsorted or duplicate input rather than fixing it.
    pub fn decode(bytes: &[u8]) -> Result<Listing, DecodeError> {
        let mut r = Reader::new(bytes);
        if r.u8()? != 1 {
            return Err(DecodeError::Invalid("unknown listing version"));
        }
        let count = r.count(MAX_ENTRIES, "entry count")?;
        let mut entries: Vec<Entry> = Vec::new();
        for _ in 0..count {
            let name = r.string(MAX_NAME_BYTES, "name")?.to_owned();
            let kind =
                EntryKind::from_u8(r.u8()?).ok_or(DecodeError::Invalid("unknown entry kind"))?;
            let id = r.id()?;
            let size = r.varint()?;
            let e = Entry {
                name,
                kind,
                id,
                size,
            };
            e.check()?;
            if let Some(prev) = entries.last()
                && prev.name.as_bytes() >= e.name.as_bytes()
            {
                return Err(DecodeError::Invalid("entries are not sorted and unique"));
            }
            entries.push(e);
        }
        r.finish()?;
        Ok(Listing { entries })
    }

    pub fn id(&self) -> Id {
        hash_record(Flavor::FolderListing, &self.encode())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{hex, unhex};
    use proptest::prelude::*;

    fn entry_hex(name: &str, kind: u8, size: u8) -> String {
        format!(
            "{:02x}{}{:02x}{}{:02x}",
            name.len(),
            hex(name.as_bytes()),
            kind,
            "00".repeat(32),
            size
        )
    }

    #[test]
    fn empty_listing_bytes() {
        assert_eq!(hex(&Listing::empty().encode()), "0100");
    }

    #[test]
    fn one_entry_bytes() {
        let l = Listing::new(vec![Entry {
            name: "a".into(),
            kind: EntryKind::File,
            id: Id([0; 32]),
            size: 5,
        }])
        .unwrap();
        assert_eq!(hex(&l.encode()), format!("0101{}", entry_hex("a", 1, 5)));
    }

    #[test]
    fn decode_rejects_bad_listings() {
        let two =
            |a: &str, b: &str| unhex(&format!("0102{}{}", entry_hex(a, 1, 0), entry_hex(b, 1, 0)));
        assert!(Listing::decode(&two("b", "a")).is_err()); // unsorted
        assert!(Listing::decode(&two("a", "a")).is_err()); // duplicate
        assert!(Listing::decode(&unhex(&format!("0101{}", entry_hex("..", 1, 0)))).is_err()); // bad name
        assert!(Listing::decode(&unhex(&format!("0101{}", entry_hex("a", 7, 0)))).is_err()); // bad kind
        assert!(Listing::decode(&unhex(&format!("0101{}", entry_hex("a", 4, 5)))).is_err()); // folder with a size
        assert!(matches!(
            Listing::decode(&unhex("01c1843d")),
            Err(DecodeError::LimitExceeded { .. })
        )); // 1,000,001 entries
        assert!(Listing::decode(&unhex("0100ff")).is_err()); // trailing byte
    }

    #[test]
    fn new_sorts_and_get_finds() {
        let e = |n: &str| Entry {
            name: n.into(),
            kind: EntryKind::File,
            id: Id([1; 32]),
            size: 1,
        };
        let l = Listing::new(vec![e("b"), e("a")]).unwrap();
        assert_eq!(l.entries()[0].name, "a");
        assert!(l.get("b").is_some() && l.get("c").is_none());
        assert!(Listing::new(vec![e("a"), e("a")]).is_err());
    }

    proptest! {
        #[test]
        fn listings_round_trip(names in prop::collection::btree_set("[a-zA-Z0-9_.-]{1,12}", 0..20),
                               size in any::<u64>()) {
            let entries: Vec<Entry> = names.into_iter()
                .filter(|n| validate_name(n).is_ok())
                .map(|n| Entry { name: n, kind: EntryKind::File, id: Id([7; 32]), size })
                .collect();
            let l = Listing::new(entries).unwrap();
            prop_assert_eq!(Listing::decode(&l.encode()).unwrap().encode(), l.encode());
        }
    }
}
