//! Folder listings (docs/storage-format.md section 3).

use std::collections::{HashMap, HashSet};

use crate::encoding::{DecodeError, Reader, Writer};
use crate::id::{Flavor, Id, hash_record};
use crate::names::{MAX_NAME_BYTES, MAX_PATH_BYTES, validate_name};

/// Most entries one listing may hold.
pub const MAX_ENTRIES: u64 = 1_000_000;

/// Most levels of folders below the root of one save (storage-format.md section 3).
pub const MAX_SAVE_DEPTH: usize = 4096;

/// Most entries one save may describe in total, counting every path: a listing reached through
/// two paths counts twice (storage-format.md section 3).
pub const MAX_SAVE_ENTRIES: u64 = 10_000_000;

/// The limits for a whole save, and how much of a save one walk has met so far. Every walk over a
/// save takes one of these (two when it compares two saves), so the rule lives here.
pub struct SaveLimits {
    max_depth: usize,
    max_entries: u64,
    max_path: usize,
    entries: u64,
    /// The folder listings the walk has gone into.
    entered: HashSet<Id>,
    /// What each listing measured holds, kept for the whole walk.
    measured: HashMap<Id, Measure>,
}

/// How big the part of a save below one folder listing is, counted by path.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Measure {
    /// Its entries and all below them, a listing again for every path to it.
    entries: u64,
    /// Levels of folders below it.
    depth: usize,
    /// Its longest path, from below it.
    longest: usize,
}

impl Default for SaveLimits {
    /// The limits of the format: `MAX_SAVE_DEPTH`, `MAX_SAVE_ENTRIES` and `MAX_PATH_BYTES`.
    fn default() -> SaveLimits {
        SaveLimits::new(MAX_SAVE_DEPTH, MAX_SAVE_ENTRIES)
    }
}

impl SaveLimits {
    /// Limits of other sizes than the format's, paths aside; tests use small ones.
    pub(crate) fn new(max_depth: usize, max_entries: u64) -> SaveLimits {
        SaveLimits {
            max_depth,
            max_entries,
            max_path: MAX_PATH_BYTES,
            entries: 0,
            entered: HashSet::new(),
            measured: HashMap::new(),
        }
    }

    /// These limits with paths of at most `max_path` bytes.
    #[cfg(test)]
    pub(crate) fn with_max_path(self, max_path: usize) -> SaveLimits {
        SaveLimits { max_path, ..self }
    }

    /// Says what is wrong if a path `len` bytes long, counted from the root with `/` between
    /// names, is longer than the limit.
    pub fn check_path(&self, len: usize) -> Result<(), String> {
        if len > self.max_path {
            return Err(format!(
                "a path in it is longer than {} bytes",
                self.max_path
            ));
        }
        Ok(())
    }

    /// Says what is wrong if a folder `depth` levels below the root of the save (the root's own
    /// listing is level 0) is deeper than the limit.
    pub fn check_depth(&self, depth: usize) -> Result<(), String> {
        if depth > self.max_depth {
            return Err(format!(
                "its folders nest more than {} levels deep",
                self.max_depth
            ));
        }
        Ok(())
    }

    /// Counts `n` more entries, a listing again for every path it is reached by; says what is
    /// wrong if that makes more than the limit.
    pub fn add(&mut self, n: usize) -> Result<(), String> {
        self.count(n as u64)
    }

    fn count(&mut self, n: u64) -> Result<(), String> {
        self.entries = self.entries.saturating_add(n);
        if self.entries > self.max_entries {
            return Err(format!(
                "it has more than {} files and folders in total",
                self.max_entries
            ));
        }
        Ok(())
    }

    /// Notes that the walk goes into the folder listing `id`, of a folder `depth` levels down
    /// whose path is `at` bytes long. If it went into it before, by another path, all of it is
    /// counted now, and this gives `true`: the walk then goes through it without counting it
    /// again. A walk calls this as it meets each folder, before going into any of them, so a
    /// shared listing is counted whole before its paths are walked.
    pub(crate) fn enter<E>(
        &mut self,
        id: Id,
        depth: usize,
        at: usize,
        read: impl FnMut(&Id) -> Result<Listing, E>,
    ) -> Result<Result<bool, String>, E> {
        if self.entered.insert(id) {
            return Ok(Ok(false));
        }
        Ok(self.add_all(id, depth, at, read)?.map(|()| true))
    }

    /// Counts all of the folder listing `id` at once, as [`enter`](Self::enter) does for one met
    /// again. Each listing below is read once with `read`, however many paths reach it.
    pub(crate) fn add_all<E>(
        &mut self,
        id: Id,
        depth: usize,
        at: usize,
        read: impl FnMut(&Id) -> Result<Listing, E>,
    ) -> Result<Result<(), String>, E> {
        let m = match self.measure(id, depth, read)? {
            Ok(m) => m,
            Err(why) => return Ok(Err(why)),
        };
        let longest = match m.longest {
            0 => at,
            below => at.saturating_add(1).saturating_add(below),
        };
        Ok(self
            .check_depth(depth.saturating_add(m.depth))
            .and_then(|()| self.check_path(longest))
            .and_then(|()| self.count(m.entries)))
    }

    /// Measures the listing `id`, of a folder `depth` levels down; a listing that would be deeper
    /// than the limit stops it, unread.
    fn measure<E>(
        &mut self,
        id: Id,
        depth: usize,
        mut read: impl FnMut(&Id) -> Result<Listing, E>,
    ) -> Result<Result<Measure, String>, E> {
        enum Step {
            Read(Id, usize),
            /// A listing's entries and longest name of a file, and its folders with their names'
            /// lengths, once all of those are measured.
            Sum(Id, u64, usize, Vec<(Id, usize)>),
        }
        // Below every folder first, without a call per level: a save may nest deeper than a stack.
        let mut todo = vec![Step::Read(id, depth)];
        while let Some(step) = todo.pop() {
            match step {
                Step::Read(id, level) => {
                    if self.measured.contains_key(&id) {
                        continue;
                    }
                    if let Err(why) = self.check_depth(level) {
                        return Ok(Err(why));
                    }
                    let listing = read(&id)?;
                    let (mut longest, mut folders) = (0, Vec::new());
                    for e in listing.entries() {
                        if e.kind == EntryKind::Folder {
                            folders.push((e.id, e.name.len()));
                        } else {
                            longest = longest.max(e.name.len());
                        }
                    }
                    let mut below: Vec<Id> = folders.iter().map(|&(f, _)| f).collect();
                    below.sort_unstable();
                    below.dedup();
                    let entries = listing.entries().len() as u64;
                    todo.push(Step::Sum(id, entries, longest, folders));
                    let next = level.saturating_add(1);
                    todo.extend(below.into_iter().map(|f| Step::Read(f, next)));
                }
                Step::Sum(id, entries, longest, folders) => {
                    let mut m = Measure {
                        entries,
                        depth: 0,
                        longest,
                    };
                    for (folder, name) in folders {
                        let below = self.measured.get(&folder).copied().unwrap_or_default();
                        m.entries = m.entries.saturating_add(below.entries);
                        m.depth = m.depth.max(below.depth.saturating_add(1));
                        let path = match below.longest {
                            0 => name,
                            longest => name.saturating_add(1).saturating_add(longest),
                        };
                        m.longest = m.longest.max(path);
                    }
                    self.measured.insert(id, m);
                }
            }
        }
        Ok(Ok(self.measured.get(&id).copied().unwrap_or_default()))
    }
}

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
    fn names_of_up_to_765_bytes_decode() {
        let one = |name: &str| {
            let mut w = Writer::new();
            w.u8(1);
            w.varint(1);
            w.string(name);
            w.u8(EntryKind::File as u8);
            w.id(&Id([0; 32]));
            w.varint(0);
            w.into_bytes()
        };
        // Windows' 255 UTF-16 units, in characters of three bytes.
        let longest = "語".repeat(255);
        let decoded = Listing::decode(&one(&longest)).unwrap();
        assert_eq!(decoded.entries()[0].name, longest);
        assert!(Listing::decode(&one(&"a".repeat(766))).is_err());
    }

    #[test]
    fn a_path_is_held_to_the_limit() {
        let limits = SaveLimits::default();
        assert!(limits.check_path(4096).is_ok());
        let why = limits.check_path(4097).unwrap_err();
        assert_eq!(why, "a path in it is longer than 4096 bytes");
    }

    /// Listings by ID, as a test's store, and how many reads each took.
    #[derive(Default)]
    struct Held {
        listings: HashMap<Id, Listing>,
        reads: HashMap<Id, usize>,
    }

    impl Held {
        /// Holds a listing of `names`, all of `kind` and naming `id`; its ID.
        fn add(&mut self, names: impl IntoIterator<Item = String>, kind: EntryKind, id: Id) -> Id {
            let entries = names.into_iter().map(|name| Entry {
                name,
                kind,
                id,
                size: 0,
            });
            let listing = Listing::new(entries.collect()).unwrap();
            let id = listing.id();
            self.listings.insert(id, listing);
            id
        }

        fn read(&mut self, id: &Id) -> Result<Listing, &'static str> {
            *self.reads.entry(*id).or_default() += 1;
            self.listings.get(id).cloned().ok_or("missing")
        }
    }

    fn names(prefix: &str, n: usize) -> impl Iterator<Item = String> + '_ {
        (0..n).map(move |i| format!("{prefix}{i:05}"))
    }

    /// The red team's deep tree: 24 listings each naming the next twice, `a` and `b`, ending in
    /// a listing of two files.
    fn chain(held: &mut Held) -> Id {
        let file = Id([7; 32]);
        let mut at = held.add(["a".into(), "b".into()], EntryKind::File, file);
        for _ in 0..24 {
            at = held.add(["a".into(), "b".into()], EntryKind::Folder, at);
        }
        at
    }

    #[test]
    fn the_measure_counts_a_shared_chain_by_path_reading_each_listing_once() {
        let mut held = Held::default();
        let top = chain(&mut held);
        let mut limits = SaveLimits::default();
        let m = limits.measure(top, 0, |id| held.read(id)).unwrap().unwrap();
        // The 2^25 files at the bottom, and the 2^25 - 2 folders above them.
        let want = Measure {
            entries: (1 << 26) - 2,
            depth: 24,
            longest: "a/".len() * 24 + 1,
        };
        assert_eq!(m, want);
        assert_eq!(held.reads.len(), 25);
        assert!(held.reads.values().all(|&n| n == 1), "{:?}", held.reads);
        // Kept for the rest of the walk: met again, nothing is read.
        assert_eq!(limits.measure(top, 0, |id| held.read(id)).unwrap(), Ok(m));
        assert!(held.reads.values().all(|&n| n == 1));
    }

    #[test]
    fn a_listing_met_again_is_counted_whole_at_once() {
        let mut held = Held::default();
        let top = chain(&mut held);
        let mut limits = SaveLimits::default();
        // The first time the walk goes into it and counts it itself.
        assert_eq!(limits.enter(top, 1, 1, |id| held.read(id)), Ok(Ok(false)));
        assert!(held.reads.is_empty());
        let refused = limits.enter(top, 1, 1, |id| held.read(id)).unwrap();
        assert_eq!(
            refused,
            Err("it has more than 10000000 files and folders in total".into())
        );
        // An unreadable listing gives the walk's own error.
        let mut limits = SaveLimits::default();
        let missing = Id([9; 32]);
        assert_eq!(
            limits.enter(missing, 1, 1, |id| held.read(id)),
            Ok(Ok(false))
        );
        assert_eq!(
            limits.enter(missing, 1, 1, |id| held.read(id)),
            Err("missing")
        );
    }

    /// A save of `files` files and ten folders at the top, each holding the same 9,999 folders of
    /// the same 99 files: 10 + `files` + 10 * 9,999 * 100 entries by path, from four listings.
    fn shared_save(held: &mut Held, files: usize) -> Id {
        let x = held.add(names("x", 99), EntryKind::File, Id([7; 32]));
        let y = held.add(names("y", 9_999), EntryKind::Folder, x);
        let top = Listing::new(
            names("t", 10)
                .map(|name| Entry {
                    name,
                    kind: EntryKind::Folder,
                    id: y,
                    size: 0,
                })
                .chain(names("f", files).map(|name| Entry {
                    name,
                    kind: EntryKind::File,
                    id: Id([7; 32]),
                    size: 0,
                }))
                .collect(),
        )
        .unwrap();
        let id = top.id();
        held.listings.insert(id, top);
        id
    }

    #[test]
    fn a_save_is_held_to_the_entry_limit_by_path_however_its_listings_are_shared() {
        let mut held = Held::default();
        let (under, over) = (shared_save(&mut held, 989), shared_save(&mut held, 991));
        let mut limits = SaveLimits::default();
        let m = limits
            .measure(under, 0, |id| held.read(id))
            .unwrap()
            .unwrap();
        assert_eq!(m.entries, 9_999_999);
        let mut counted = |top| SaveLimits::default().add_all(top, 0, 0, |id| held.read(id));
        assert_eq!(counted(under), Ok(Ok(())));
        assert_eq!(
            counted(over),
            Ok(Err(
                "it has more than 10000000 files and folders in total".into()
            ))
        );
    }

    #[test]
    fn a_listing_counted_whole_is_held_to_the_depth_and_path_limits_from_where_it_is() {
        let mut held = Held::default();
        let top = chain(&mut held);
        // 24 levels below a folder 1 level down, and paths of 49 bytes below one of 1 byte.
        let mut fits = SaveLimits::new(25, 1 << 26).with_max_path(51);
        assert_eq!(fits.add_all(top, 1, 1, |id| held.read(id)), Ok(Ok(())));
        let mut deep = SaveLimits::new(24, MAX_SAVE_ENTRIES);
        let why = deep
            .add_all(top, 1, 1, |id| held.read(id))
            .unwrap()
            .unwrap_err();
        assert_eq!(why, "its folders nest more than 24 levels deep");
        let mut long = SaveLimits::new(25, 1 << 26).with_max_path(50);
        let why = long
            .add_all(top, 1, 1, |id| held.read(id))
            .unwrap()
            .unwrap_err();
        assert_eq!(why, "a path in it is longer than 50 bytes");
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
