//! Diary entries (docs/storage-format.md section 9): one per changing command, and the tag list
//! and server lines that entries share.

use std::collections::{BTreeMap, BTreeSet};

use crate::encoding::{DecodeError, Reader, Writer, varint_len};
use crate::id::{Flavor, Id, hash_record};
use crate::listing::EntryKind;
use crate::names::MAX_PATH_BYTES;
use crate::save::{Person, read_person, write_person};

pub const MAX_NAME_BYTES: usize = 255;
pub const MAX_TAG_MESSAGE_BYTES: usize = 64 * 1024;
pub const MAX_COMMAND_BYTES: usize = 64 * 1024;
pub const MAX_VERSION_BYTES: usize = 256;
pub const MAX_LINES: u64 = 100_000;
pub const MAX_TAGS: u64 = 100_000;
pub const MAX_SHELVES: u64 = 10_000;
pub const MAX_SERVERS: u64 = 64;
pub const MAX_SAFETY_COPIES: u64 = 1_000_000;
pub const MAX_GIT_TAG_OBJECTS: u64 = 16;
pub const MAX_GIT_TAG_OBJECT_BYTES: usize = 1 << 20;

/// What kind of command an entry records.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Command,
    /// Undoes the given command entry.
    Undo(Id),
    /// Redoes the given command entry.
    Redo(Id),
}

/// Where the user is: on a line, or looking at a save directly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Position {
    OnLine(String),
    AtSave(Id),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tag {
    pub save: Id,
    pub message: String,
    /// The original bytes of each Git tag object from the tag's ref to the commit, so a Git export
    /// can recreate them. Empty for a tag made in Bolt or a lightweight Git tag.
    pub git_tags: Vec<Vec<u8>>,
}

/// Every tag by name. A stored list is never empty: an entry without tags has no list.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TagList {
    pub tags: BTreeMap<String, Tag>,
}

/// Each server's lines as last seen, by server name. A stored list is never empty, and no server
/// in it is without lines.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ServerLines {
    pub servers: BTreeMap<String, BTreeMap<String, Id>>,
}

/// The complete repository state after a command. Tags and server lines change rarely and can be
/// big, so entries share them as records of their own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct State {
    pub lines: BTreeMap<String, Id>,
    /// The tag list, if there are tags.
    pub tags: Option<Id>,
    pub position: Position,
    /// Newest first; labels are unique.
    pub shelves: Vec<(String, Id)>,
    /// The server-lines list, if there are servers.
    pub server_lines: Option<Id>,
}

/// A file Bolt kept before overwriting it, so the command can be undone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SafetyCopy {
    pub path: String,
    pub kind: EntryKind,
    pub id: Id,
    pub size: u64,
}

/// What a command did to the project folder: a switch from one save to another.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FolderChange {
    pub from: Id,
    pub to: Id,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiaryEntry {
    pub parent: Option<Id>,
    pub action: Action,
    pub command: String,
    pub who: Person,
    pub bolt_version: String,
    pub state: State,
    /// Paths must be unique; an entry with duplicates encodes but will not decode.
    pub safety_copies: Vec<SafetyCopy>,
}

/// Whether a line name may hold `c` (decision 012).
#[must_use]
pub fn is_line_name_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '-' | '_' | '.' | '/')
}

/// Decision 012's rules for line names.
pub fn validate_line_name(name: &str) -> Result<(), DecodeError> {
    if name.is_empty() {
        return Err(DecodeError::Invalid("a line name cannot be empty"));
    }
    if name.len() > MAX_NAME_BYTES {
        return Err(DecodeError::Invalid(
            "a line name cannot be longer than 255 bytes",
        ));
    }
    if !name.chars().all(is_line_name_char) {
        return Err(DecodeError::Invalid(
            "a line name can only hold letters, digits, -, _, . and /",
        ));
    }
    if name
        .split('/')
        .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return Err(DecodeError::Invalid(
            "a line name cannot start or end with /, hold //, or have a . or .. part",
        ));
    }
    Ok(())
}

/// The line among `lines` that `name` names but for letter case, if any: two line names may not
/// differ only by case.
#[must_use]
pub fn line_named_alike<'a>(lines: &'a BTreeMap<String, Id>, name: &str) -> Option<&'a str> {
    let folded = fold_line_name(name);
    lines
        .keys()
        .find(|line| fold_line_name(line) == folded)
        .map(String::as_str)
}

/// A line name as compared with others: two may not differ only by letter case.
pub(crate) fn fold_line_name(name: &str) -> String {
    name.to_lowercase()
}

fn write_lines(w: &mut Writer, lines: &BTreeMap<String, Id>) {
    w.varint(lines.len() as u64);
    for (name, save) in lines {
        w.string(name);
        w.id(save);
    }
}

/// Reads the next name of a sorted list of line names: valid, and not the same as an earlier one
/// but for letter case.
fn read_line_name(
    r: &mut Reader,
    prev: &mut Option<String>,
    folded: &mut BTreeSet<String>,
    field: &'static str,
) -> Result<String, DecodeError> {
    let name = read_sorted_name(r, prev, field)?;
    validate_line_name(&name)?;
    if !folded.insert(fold_line_name(&name)) {
        return Err(DecodeError::Invalid("two names differ only by letter case"));
    }
    Ok(name)
}

/// Reads a sorted list of valid, case-insensitively unique line names.
fn read_lines(r: &mut Reader) -> Result<BTreeMap<String, Id>, DecodeError> {
    let n = r.count(MAX_LINES, "line count")?;
    let mut lines = BTreeMap::new();
    let mut folded = BTreeSet::new();
    let mut prev = None;
    for _ in 0..n {
        let name = read_line_name(r, &mut prev, &mut folded, "line name")?;
        lines.insert(name, r.id()?);
    }
    Ok(lines)
}

fn write_optional_id(w: &mut Writer, id: Option<Id>) {
    w.bool(id.is_some());
    if let Some(id) = id {
        w.id(&id);
    }
}

fn read_optional_id(r: &mut Reader) -> Result<Option<Id>, DecodeError> {
    if r.bool()? {
        Ok(Some(r.id()?))
    } else {
        Ok(None)
    }
}

/// Reads a name that must sort strictly after `prev`.
fn read_sorted_name(
    r: &mut Reader,
    prev: &mut Option<String>,
    field: &'static str,
) -> Result<String, DecodeError> {
    let name = r.string(MAX_NAME_BYTES, field)?.to_owned();
    if prev.as_ref().is_some_and(|p| *p >= name) {
        return Err(DecodeError::Invalid("names not strictly sorted"));
    }
    *prev = Some(name.clone());
    Ok(name)
}

impl DiaryEntry {
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        w.u8(1);
        match &self.parent {
            None => w.varint(0),
            Some(p) => {
                w.varint(1);
                w.id(p);
            }
        }
        match &self.action {
            Action::Command => w.u8(1),
            Action::Undo(target) => {
                w.u8(2);
                w.id(target);
            }
            Action::Redo(target) => {
                w.u8(3);
                w.id(target);
            }
        }
        w.string(&self.command);
        write_person(&mut w, &self.who);
        w.string(&self.bolt_version);
        let s = &self.state;
        write_lines(&mut w, &s.lines);
        write_optional_id(&mut w, s.tags);
        match &s.position {
            Position::OnLine(line) => {
                w.u8(1);
                w.string(line);
            }
            Position::AtSave(save) => {
                w.u8(2);
                w.id(save);
            }
        }
        w.varint(s.shelves.len() as u64);
        for (label, save) in &s.shelves {
            w.string(label);
            w.id(save);
        }
        write_optional_id(&mut w, s.server_lines);
        let mut copies: Vec<&SafetyCopy> = self.safety_copies.iter().collect();
        copies.sort_by(|a, b| a.path.cmp(&b.path));
        w.varint(copies.len() as u64);
        for c in copies {
            w.string(&c.path);
            w.u8(c.kind as u8);
            w.id(&c.id);
            w.varint(c.size);
        }
        w.into_bytes()
    }

    pub fn decode(bytes: &[u8]) -> Result<DiaryEntry, DecodeError> {
        let mut r = Reader::new(bytes);
        if r.u8()? != 1 {
            return Err(DecodeError::Invalid("unknown diary entry version"));
        }
        let parent = match r.count(1, "parent count")? {
            0 => None,
            _ => Some(r.id()?),
        };
        let action = match r.u8()? {
            1 => Action::Command,
            2 => Action::Undo(r.id()?),
            3 => Action::Redo(r.id()?),
            _ => return Err(DecodeError::Invalid("unknown diary action")),
        };
        let command = r.string(MAX_COMMAND_BYTES, "command")?.to_owned();
        let who = read_person(&mut r)?;
        let bolt_version = r.string(MAX_VERSION_BYTES, "Bolt version")?.to_owned();
        let lines = read_lines(&mut r)?;
        let tags = read_optional_id(&mut r)?;
        let position = match r.u8()? {
            1 => {
                let line = r.string(MAX_NAME_BYTES, "line name")?.to_owned();
                if !lines.contains_key(&line) {
                    return Err(DecodeError::Invalid("the current line does not exist"));
                }
                Position::OnLine(line)
            }
            2 => Position::AtSave(r.id()?),
            _ => return Err(DecodeError::Invalid("unknown position")),
        };
        let n = r.count(MAX_SHELVES, "shelf count")?;
        let mut shelves: Vec<(String, Id)> = Vec::new();
        let mut labels = BTreeSet::new();
        for _ in 0..n {
            let label = r.string(MAX_NAME_BYTES, "shelf label")?.to_owned();
            if !labels.insert(label.clone()) {
                return Err(DecodeError::Invalid("duplicate shelf label"));
            }
            shelves.push((label, r.id()?));
        }
        let server_lines = read_optional_id(&mut r)?;
        let n = r.count(MAX_SAFETY_COPIES, "safety-copy count")?;
        let mut safety_copies: Vec<SafetyCopy> = Vec::new();
        for _ in 0..n {
            let path = r.string(MAX_PATH_BYTES, "safety-copy path")?.to_owned();
            if safety_copies.last().is_some_and(|c| c.path >= path) {
                return Err(DecodeError::Invalid(
                    "safety copies not strictly sorted by path",
                ));
            }
            let kind = EntryKind::from_u8(r.u8()?)
                .ok_or(DecodeError::Invalid("unknown safety-copy kind"))?;
            let id = r.id()?;
            let size = r.varint()?;
            safety_copies.push(SafetyCopy {
                path,
                kind,
                id,
                size,
            });
        }
        r.finish()?;
        Ok(DiaryEntry {
            parent,
            action,
            command,
            who,
            bolt_version,
            state: State {
                lines,
                tags,
                position,
                shelves,
                server_lines,
            },
            safety_copies,
        })
    }

    #[must_use]
    pub fn id(&self) -> Id {
        hash_record(Flavor::DiaryEntry, &self.encode())
    }
}

impl TagList {
    /// The list's bytes, written once into a buffer of their size.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let tags = self
            .tags
            .iter()
            .map(|(name, tag)| TagList::tag_len(name, tag));
        let len = TagList::head_len(self.tags.len() as u64) + tags.sum::<u64>();
        let mut w = TagListWriter::with_capacity(len);
        for (name, tag) in &self.tags {
            w.push(name, tag);
        }
        w.finish()
    }

    /// What `encode` writes before the tags of a list of `count`: its version and the count.
    #[must_use]
    pub fn head_len(count: u64) -> u64 {
        1 + varint_len(count)
    }

    /// What `encode` writes for the tag `name`, so a list's size can be known before it is built.
    #[must_use]
    pub fn tag_len(name: &str, tag: &Tag) -> u64 {
        let objects = tag.git_tags.iter().map(Vec::len);
        TagList::tag_len_from(name.len(), tag.message.len(), objects)
    }

    /// `tag_len` from lengths alone: the name's, the message's and each Git tag object's, so a
    /// list's size can be known before even its tags are.
    #[must_use]
    pub fn tag_len_from(
        name: usize,
        message: usize,
        objects: impl ExactSizeIterator<Item = usize>,
    ) -> u64 {
        let bytes = |len: usize| varint_len(len as u64) + len as u64;
        let count = varint_len(objects.len() as u64);
        let save = size_of::<Id>() as u64;
        bytes(name) + save + bytes(message) + count + objects.map(bytes).sum::<u64>()
    }

    pub fn decode(bytes: &[u8]) -> Result<TagList, DecodeError> {
        let mut tags = BTreeMap::new();
        TagList::read(bytes, |name, tag| {
            tags.insert(name, tag);
        })?;
        Ok(TagList { tags })
    }

    /// Checks `bytes` as `decode` does, but lets each tag go once it is read: for a list too big
    /// to hold twice.
    pub fn check(bytes: &[u8]) -> Result<(), DecodeError> {
        TagList::read(bytes, |_, _| {})
    }

    /// Reads the list `bytes`, giving `each` every tag in name order.
    fn read(bytes: &[u8], mut each: impl FnMut(String, Tag)) -> Result<(), DecodeError> {
        let mut r = Reader::new(bytes);
        if r.u8()? != 1 {
            return Err(DecodeError::Invalid("unknown tag list version"));
        }
        let n = r.count(MAX_TAGS, "tag count")?;
        if n == 0 {
            return Err(DecodeError::Invalid("a tag list cannot be empty"));
        }
        let mut prev = None;
        for _ in 0..n {
            let name = read_sorted_name(&mut r, &mut prev, "tag name")?;
            if name.is_empty() {
                return Err(DecodeError::Invalid("a tag name cannot be empty"));
            }
            let save = r.id()?;
            let message = r.string(MAX_TAG_MESSAGE_BYTES, "tag message")?.to_owned();
            let objects = r.count(MAX_GIT_TAG_OBJECTS, "Git tag object count")?;
            let mut git_tags = Vec::new();
            for _ in 0..objects {
                let object = r.bytes(MAX_GIT_TAG_OBJECT_BYTES, "Git tag object")?;
                git_tags.push(object.to_vec());
            }
            each(
                name,
                Tag {
                    save,
                    message,
                    git_tags,
                },
            );
        }
        r.finish()
    }

    #[must_use]
    pub fn id(&self) -> Id {
        hash_record(Flavor::TagList, &self.encode())
    }
}

/// Writes a tag list a tag at a time, so a caller can let each tag go once it is written. Tags go
/// in name order, as a list holds them.
pub struct TagListWriter {
    tags: Writer,
    count: u64,
}

impl TagListWriter {
    /// A writer with room for `len` bytes: the list's size, as `head_len` and `tag_len` count it,
    /// so the list is written without its buffer growing.
    #[must_use]
    pub fn with_capacity(len: u64) -> TagListWriter {
        TagListWriter {
            tags: Writer::with_capacity(usize::try_from(len).unwrap_or(0)),
            count: 0,
        }
    }

    pub fn push(&mut self, name: &str, tag: &Tag) {
        let w = &mut self.tags;
        w.string(name);
        w.id(&tag.save);
        w.string(&tag.message);
        w.varint(tag.git_tags.len() as u64);
        for object in &tag.git_tags {
            w.bytes(object);
        }
        self.count = self.count.saturating_add(1);
    }

    /// How many tags it holds.
    #[must_use]
    pub fn count(&self) -> u64 {
        self.count
    }

    /// The list: its version and count, then the tags. The head goes in front of them in place,
    /// in the room `with_capacity` left for it.
    #[must_use]
    pub fn finish(self) -> Vec<u8> {
        let mut head = Writer::new();
        head.u8(1);
        head.varint(self.count);
        let mut bytes = self.tags.into_bytes();
        bytes.splice(0..0, head.into_bytes());
        bytes
    }
}

impl ServerLines {
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        w.u8(1);
        w.varint(self.servers.len() as u64);
        for (name, lines) in &self.servers {
            w.string(name);
            write_lines(&mut w, lines);
        }
        w.into_bytes()
    }

    /// What `encode` writes before the servers of a list of `count`: its version and the count.
    #[must_use]
    pub fn head_len(count: u64) -> u64 {
        1 + varint_len(count)
    }

    /// What `encode` writes for the server `name` with lines named `lines`, so a list's size can
    /// be known before its saves are.
    #[must_use]
    pub fn server_len<'a>(name: &str, lines: impl ExactSizeIterator<Item = &'a str>) -> u64 {
        let bytes = |len: usize| varint_len(len as u64) + len as u64;
        let count = varint_len(lines.len() as u64);
        let saves = size_of::<Id>() as u64;
        bytes(name.len()) + count + lines.map(|line| bytes(line.len()) + saves).sum::<u64>()
    }

    pub fn decode(bytes: &[u8]) -> Result<ServerLines, DecodeError> {
        let mut servers = BTreeMap::new();
        ServerLines::read(bytes, |name, lines| {
            servers.insert(name, lines);
        })?;
        Ok(ServerLines { servers })
    }

    /// Checks `bytes` as `decode` does, but lets each server's lines go once they are read: for a
    /// list too big to hold twice.
    pub fn check(bytes: &[u8]) -> Result<(), DecodeError> {
        ServerLines::read(bytes, |_, _| {})
    }

    /// Reads the list `bytes`, giving `each` every server and its lines in name order.
    fn read(
        bytes: &[u8],
        mut each: impl FnMut(String, BTreeMap<String, Id>),
    ) -> Result<(), DecodeError> {
        let mut r = Reader::new(bytes);
        if r.u8()? != 1 {
            return Err(DecodeError::Invalid("unknown server lines version"));
        }
        let n = r.count(MAX_SERVERS, "server count")?;
        if n == 0 {
            return Err(DecodeError::Invalid("a server-lines list cannot be empty"));
        }
        let mut folded = BTreeSet::new();
        let mut prev = None;
        for _ in 0..n {
            let name = read_line_name(&mut r, &mut prev, &mut folded, "server name")?;
            let lines = read_lines(&mut r)?;
            if lines.is_empty() {
                return Err(DecodeError::Invalid("a server cannot be without lines"));
            }
            each(name, lines);
        }
        r.finish()
    }

    #[must_use]
    pub fn id(&self) -> Id {
        hash_record(Flavor::ServerLines, &self.encode())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::hex;
    use proptest::prelude::*;

    fn person() -> Person {
        Person {
            name: "A".into(),
            email: "a@b".into(),
            time_ms: 0,
            tz_offset_min: 0,
        }
    }

    fn minimal() -> DiaryEntry {
        DiaryEntry {
            parent: None,
            action: Action::Command,
            command: "start".into(),
            who: person(),
            bolt_version: "0.1.0".into(),
            state: State {
                lines: BTreeMap::from([("main".to_owned(), Id([0; 32]))]),
                tags: None,
                position: Position::OnLine("main".into()),
                shelves: vec![],
                server_lines: None,
            },
            safety_copies: vec![],
        }
    }

    /// `person()` as written: name, email, time and zone.
    fn person_hex() -> String {
        format!("0141{}{}{}", "03614062", "00".repeat(8), "0000")
    }

    #[test]
    fn minimal_entry_bytes() {
        let expected = [
            "01",           // version
            "00",           // no parent
            "01",           // action: command
            "057374617274", // "start"
            &person_hex(),
            "05302e312e30", // "0.1.0"
            "01046d61696e", // one line: "main"
            &"00".repeat(32),
            "00",           // no tags
            "01046d61696e", // on line "main"
            "00",           // no shelves
            "00",           // no server lines
            "00",           // no safety copies
        ]
        .concat();
        let bytes = minimal().encode();
        assert_eq!(hex(&bytes), expected);
        assert_eq!(DiaryEntry::decode(&bytes).unwrap(), minimal());
    }

    #[test]
    fn the_shared_records_are_a_flag_and_an_id() {
        let mut e = minimal();
        e.state.tags = Some(Id([0x11; 32]));
        e.state.shelves = vec![("s".into(), Id([0x33; 32]))];
        e.state.server_lines = Some(Id([0x22; 32]));
        let expected = [
            "01",             // version
            "00",             // no parent
            "01",             // action: command
            "057374617274",   // "start"
            &person_hex(),    // who and when
            "05302e312e30",   // "0.1.0"
            "01046d61696e",   // one line: "main"
            &"00".repeat(32), // its save
            "01",             // tags: the tag list's ID follows
            &"11".repeat(32), // the tag list
            "01046d61696e",   // on line "main"
            "01",             // one shelf
            "0173",           // "s"
            &"33".repeat(32), // its save
            "01",             // server lines: the list's ID follows
            &"22".repeat(32), // the server-lines list
            "00",             // no safety copies
        ]
        .concat();
        let bytes = e.encode();
        assert_eq!(hex(&bytes), expected);
        assert_eq!(DiaryEntry::decode(&bytes).unwrap(), e);
    }

    #[test]
    fn diary_rejects_bad_entries() {
        let good = minimal().encode();
        // Action byte 4 (byte 2: after the version and the parent count).
        let mut bytes = good.clone();
        bytes[2] = 4;
        assert!(DiaryEntry::decode(&bytes).is_err());
        // Two lines that differ only by case.
        let mut e = minimal();
        e.state.lines.insert("Main".into(), Id([1; 32]));
        assert!(DiaryEntry::decode(&e.encode()).is_err());
        // On a line that doesn't exist.
        let mut e = minimal();
        e.state.position = Position::OnLine("dev".into());
        assert!(DiaryEntry::decode(&e.encode()).is_err());
        // A line name with a space.
        let mut e = minimal();
        e.state.lines.insert("has space".into(), Id([1; 32]));
        assert!(DiaryEntry::decode(&e.encode()).is_err());
        // A parent count of 2.
        let mut bytes = good.clone();
        bytes[1] = 2;
        bytes.splice(2..2, [7u8; 64]);
        assert!(matches!(
            DiaryEntry::decode(&bytes),
            Err(DecodeError::LimitExceeded { .. })
        ));
        // A has-tags byte and a has-server-lines byte of 2: the 10th and the 2nd from the end.
        for from_end in [10, 2] {
            let mut bytes = good.clone();
            let at = bytes.len() - from_end;
            bytes[at] = 2;
            assert_eq!(
                DiaryEntry::decode(&bytes),
                Err(DecodeError::InvalidBool(2)),
                "{from_end}"
            );
        }
        // Trailing bytes.
        let mut bytes = good;
        bytes.push(0);
        assert!(DiaryEntry::decode(&bytes).is_err());
    }

    #[test]
    fn line_names() {
        for ok in ["main", "feature/login", "v1.2", "a_b-c", "café", "日本"] {
            assert!(validate_line_name(ok).is_ok(), "{ok:?}");
        }
        let long = "a".repeat(256);
        for bad in [
            "",
            "has space",
            "/a",
            "a/",
            "a//b",
            ".",
            "..",
            "a/./b",
            "a/../b",
            "a:b",
            "a\0b",
            &long,
        ] {
            assert!(validate_line_name(bad).is_err(), "{bad:?}");
        }
        assert!(validate_line_name(&"a".repeat(255)).is_ok());
    }

    #[test]
    fn unsorted_safety_copies_are_rejected() {
        let copy = |p: &str| SafetyCopy {
            path: p.into(),
            kind: EntryKind::File,
            id: Id([3; 32]),
            size: 9,
        };
        let mut e = minimal();
        e.safety_copies = vec![copy("b"), copy("a")];
        // Encoding sorts them, so build the bad bytes by hand: swap the two copies.
        let bytes = e.encode();
        let one = 1 + 1 + 1 + 32 + 1; // path, kind, id, size
        let start = bytes.len() - 2 * one;
        let mut bad = bytes[..start].to_vec();
        bad.extend_from_slice(&bytes[start + one..]);
        bad.extend_from_slice(&bytes[start..start + one]);
        assert!(DiaryEntry::decode(&bytes).is_ok());
        assert!(DiaryEntry::decode(&bad).is_err());
    }

    /// A tag list written by hand, so that it can break rules a `BTreeMap` keeps.
    fn raw_tag_list(tags: &[(&str, &str, Vec<Vec<u8>>)]) -> Vec<u8> {
        let mut w = Writer::new();
        w.u8(1);
        w.varint(tags.len() as u64);
        for (name, message, git_tags) in tags {
            w.string(name);
            w.id(&Id([1; 32]));
            w.string(message);
            w.varint(git_tags.len() as u64);
            for object in git_tags {
                w.bytes(object);
            }
        }
        w.into_bytes()
    }

    /// A tag with no message and no Git tag objects.
    fn lightweight(name: &str) -> (&str, &str, Vec<Vec<u8>>) {
        (name, "", vec![])
    }

    /// A server-lines list written by hand; every line points at the same save.
    fn raw_server_lines(servers: &[(&str, Vec<&str>)]) -> Vec<u8> {
        let mut w = Writer::new();
        w.u8(1);
        w.varint(servers.len() as u64);
        for (name, lines) in servers {
            w.string(name);
            w.varint(lines.len() as u64);
            for line in lines {
                w.string(line);
                w.id(&Id([2; 32]));
            }
        }
        w.into_bytes()
    }

    #[test]
    fn tag_list_bytes() {
        let tag = |fill, message: &str, git_tags| Tag {
            save: Id([fill; 32]),
            message: message.into(),
            git_tags,
        };
        let list = TagList {
            tags: BTreeMap::from([
                ("v1".to_owned(), tag(0x11, "", vec![])),
                (
                    "v2".to_owned(),
                    tag(0x22, "ok", vec![b"tag".to_vec(), vec![]]),
                ),
            ]),
        };
        let expected = [
            "01",             // version
            "02",             // two tags
            "027631",         // "v1"
            &"11".repeat(32), // its save
            "00",             // no message
            "00",             // no Git tag objects
            "027632",         // "v2"
            &"22".repeat(32), // its save
            "026f6b",         // "ok"
            "02",             // two Git tag objects
            "03746167",       // "tag"
            "00",             // and an empty one
        ]
        .concat();
        let bytes = list.encode();
        assert_eq!(hex(&bytes), expected);
        assert_eq!(TagList::decode(&bytes).unwrap(), list);
    }

    #[test]
    fn tag_lengths_are_what_encode_writes() {
        // Lengths that take one, two and three bytes to write.
        let tags = BTreeMap::from([
            (
                "v1".to_owned(),
                Tag {
                    save: Id([1; 32]),
                    message: String::new(),
                    git_tags: vec![],
                },
            ),
            (
                "n".repeat(255),
                Tag {
                    save: Id([2; 32]),
                    message: "m".repeat(200),
                    git_tags: vec![vec![b't'; 130], vec![b'u'; 20_000], vec![]],
                },
            ),
        ]);
        let lengths: u64 = tags
            .iter()
            .map(|(name, tag)| TagList::tag_len(name, tag))
            .sum();
        let list = TagList { tags };
        let bytes = list.encode();
        assert_eq!(TagList::head_len(2) + lengths, bytes.len() as u64);
        // The head is longer for more tags.
        assert_eq!(TagList::head_len(127), 2);
        assert_eq!(TagList::head_len(MAX_TAGS), 4);
    }

    #[test]
    fn server_lengths_are_what_encode_writes() {
        // Counts and names whose lengths take one and two bytes to write.
        let lines = |count: u8| -> BTreeMap<String, Id> {
            (0..count)
                .map(|n| (format!("{}{n:03}", "l".repeat(200)), Id([n; 32])))
                .collect()
        };
        let servers = BTreeMap::from([("o".to_owned(), lines(1)), ("s".repeat(255), lines(130))]);
        let lengths: u64 = servers
            .iter()
            .map(|(name, lines)| ServerLines::server_len(name, lines.keys().map(String::as_str)))
            .sum();
        let list = ServerLines { servers };
        assert_eq!(
            ServerLines::head_len(2) + lengths,
            list.encode().len() as u64
        );
        assert_eq!(ServerLines::head_len(MAX_SERVERS), 2);
    }

    #[test]
    fn server_lines_bytes() {
        let list = ServerLines {
            servers: BTreeMap::from([(
                "origin".to_owned(),
                BTreeMap::from([
                    ("dev".to_owned(), Id([0x11; 32])),
                    ("main".to_owned(), Id([0x22; 32])),
                ]),
            )]),
        };
        let expected = [
            "01",             // version
            "01",             // one server
            "066f726967696e", // "origin"
            "02",             // two lines
            "03646576",       // "dev"
            &"11".repeat(32), // its save
            "046d61696e",     // "main"
            &"22".repeat(32), // its save
        ]
        .concat();
        let bytes = list.encode();
        assert_eq!(hex(&bytes), expected);
        assert_eq!(ServerLines::decode(&bytes).unwrap(), list);
    }

    #[test]
    fn tag_list_rejections() {
        let mut other_version = raw_tag_list(&[lightweight("a")]);
        other_version[0] = 2;
        let mut extra_byte = raw_tag_list(&[lightweight("a")]);
        extra_byte.push(0);
        let long_message = "m".repeat(MAX_TAG_MESSAGE_BYTES + 1);
        let bad = [
            ("no tags", raw_tag_list(&[])),
            (
                "tags out of order",
                raw_tag_list(&[lightweight("b"), lightweight("a")]),
            ),
            (
                "a tag twice",
                raw_tag_list(&[lightweight("a"), lightweight("a")]),
            ),
            ("an empty name", raw_tag_list(&[lightweight("")])),
            (
                "a name of 256 bytes",
                raw_tag_list(&[lightweight(&"a".repeat(256))]),
            ),
            (
                "a message over 64 KiB",
                raw_tag_list(&[("a", &long_message, vec![])]),
            ),
            (
                "17 Git tag objects",
                raw_tag_list(&[("a", "", vec![vec![]; 17])]),
            ),
            (
                "a Git tag object over 1 MiB",
                raw_tag_list(&[("a", "", vec![vec![0; MAX_GIT_TAG_OBJECT_BYTES + 1]])]),
            ),
            ("another version", other_version),
            ("a byte after the list", extra_byte),
        ];
        for (what, bytes) in bad {
            assert!(TagList::decode(&bytes).is_err(), "{what}");
            assert!(TagList::check(&bytes).is_err(), "{what}");
        }
        // Just inside each limit, the same lists are fine.
        let message = "m".repeat(MAX_TAG_MESSAGE_BYTES);
        let good = [
            raw_tag_list(&[lightweight("a"), lightweight("b")]),
            raw_tag_list(&[lightweight(&"a".repeat(255))]),
            raw_tag_list(&[("a", &message, vec![])]),
            raw_tag_list(&[("a", "", vec![vec![]; 16])]),
            raw_tag_list(&[("a", "", vec![vec![0; MAX_GIT_TAG_OBJECT_BYTES]])]),
        ];
        for bytes in good {
            TagList::decode(&bytes).unwrap();
            TagList::check(&bytes).unwrap();
        }
    }

    #[test]
    fn a_tag_list_holds_at_most_100000_tags() {
        let list = |count: u32| {
            let names: Vec<String> = (0..count).map(|i| format!("{i:06}")).collect();
            raw_tag_list(&names.iter().map(|n| lightweight(n)).collect::<Vec<_>>())
        };
        assert_eq!(TagList::decode(&list(100_000)).unwrap().tags.len(), 100_000);
        assert!(TagList::decode(&list(100_001)).is_err());
    }

    #[test]
    fn server_lines_rejections() {
        let mut other_version = raw_server_lines(&[("a", vec!["main"])]);
        other_version[0] = 2;
        let mut extra_byte = raw_server_lines(&[("a", vec!["main"])]);
        extra_byte.push(0);
        let bad = [
            ("no servers", raw_server_lines(&[])),
            (
                "a server name with a space",
                raw_server_lines(&[("a b", vec!["main"])]),
            ),
            (
                "servers out of order",
                raw_server_lines(&[("b", vec!["main"]), ("a", vec!["main"])]),
            ),
            (
                "a server twice",
                raw_server_lines(&[("a", vec!["main"]), ("a", vec!["main"])]),
            ),
            (
                "servers that differ only by letter case",
                raw_server_lines(&[("Origin", vec!["main"]), ("origin", vec!["main"])]),
            ),
            (
                "a server without lines",
                raw_server_lines(&[("origin", vec![])]),
            ),
            (
                "a line name with a space",
                raw_server_lines(&[("origin", vec!["a b"])]),
            ),
            (
                "lines out of order",
                raw_server_lines(&[("origin", vec!["b", "a"])]),
            ),
            (
                "a line twice",
                raw_server_lines(&[("origin", vec!["a", "a"])]),
            ),
            (
                "lines that differ only by letter case",
                raw_server_lines(&[("origin", vec!["Main", "main"])]),
            ),
            ("another version", other_version),
            ("a byte after the list", extra_byte),
        ];
        for (what, bytes) in bad {
            assert!(ServerLines::decode(&bytes).is_err(), "{what}");
            assert!(ServerLines::check(&bytes).is_err(), "{what}");
        }
        // Letter case matters only inside one list of names.
        let same_line_on_two_servers =
            raw_server_lines(&[("a", vec!["Main"]), ("b", vec!["main"])]);
        ServerLines::decode(&same_line_on_two_servers).unwrap();
        ServerLines::check(&same_line_on_two_servers).unwrap();
    }

    #[test]
    fn server_lines_hold_at_most_64_servers_and_100000_lines_each() {
        let servers = |count: u32| {
            let names: Vec<String> = (0..count).map(|i| format!("s{i:02}")).collect();
            raw_server_lines(
                &names
                    .iter()
                    .map(|n| (n.as_str(), vec!["main"]))
                    .collect::<Vec<_>>(),
            )
        };
        assert_eq!(ServerLines::decode(&servers(64)).unwrap().servers.len(), 64);
        assert!(ServerLines::decode(&servers(65)).is_err());
        let lines = |count: u32| {
            let names: Vec<String> = (0..count).map(|i| format!("l{i:06}")).collect();
            raw_server_lines(&[("origin", names.iter().map(String::as_str).collect())])
        };
        let list = ServerLines::decode(&lines(100_000)).unwrap();
        assert_eq!(list.servers["origin"].len(), 100_000);
        assert!(ServerLines::decode(&lines(100_001)).is_err());
    }

    fn id_strategy() -> impl Strategy<Value = Id> {
        any::<[u8; 32]>().prop_map(Id)
    }

    /// Lowercase, so that no two names differ only by letter case.
    const LINE_NAME: &str = "[a-z][a-z0-9_.-]{0,8}(/[a-z0-9_-]{1,8}){0,2}";

    fn tag_strategy() -> impl Strategy<Value = Tag> {
        (
            id_strategy(),
            ".{0,20}",
            prop::collection::vec(prop::collection::vec(any::<u8>(), 0..40), 0..4),
        )
            .prop_map(|(save, message, git_tags)| Tag {
                save,
                message,
                git_tags,
            })
    }

    proptest! {
        #[test]
        fn tag_lists_round_trip(tags in prop::collection::btree_map(".{1,10}", tag_strategy(), 1..5)) {
            let list = TagList { tags };
            let bytes = list.encode();
            prop_assert_eq!(TagList::decode(&bytes).unwrap(), list.clone());
            prop_assert!(TagList::check(&bytes).is_ok());
            prop_assert_eq!(list.id(), hash_record(Flavor::TagList, &bytes));
            let lengths: u64 = list.tags.iter().map(|(name, tag)| TagList::tag_len(name, tag)).sum();
            prop_assert_eq!(TagList::head_len(list.tags.len() as u64) + lengths, bytes.len() as u64);
            // Written once, into a buffer of its size.
            prop_assert_eq!(bytes.capacity(), bytes.len());
            // A tag at a time, with room for the longest head, as an import writes it.
            let mut w = TagListWriter::with_capacity(TagList::head_len(MAX_TAGS) + lengths);
            for (name, tag) in &list.tags {
                w.push(name, tag);
            }
            prop_assert_eq!(w.finish(), bytes);
        }

        #[test]
        fn server_lines_round_trip(
            servers in prop::collection::btree_map(
                LINE_NAME,
                prop::collection::btree_map(LINE_NAME, id_strategy(), 1..4),
                1..4,
            ),
        ) {
            let list = ServerLines { servers };
            let bytes = list.encode();
            prop_assert_eq!(ServerLines::decode(&bytes).unwrap(), list.clone());
            prop_assert!(ServerLines::check(&bytes).is_ok());
            prop_assert_eq!(list.id(), hash_record(Flavor::ServerLines, &bytes));
            let lengths: u64 = list
                .servers
                .iter()
                .map(|(name, lines)| ServerLines::server_len(name, lines.keys().map(String::as_str)))
                .sum();
            let head = ServerLines::head_len(list.servers.len() as u64);
            prop_assert_eq!(head + lengths, bytes.len() as u64);
        }

        #[test]
        fn entries_round_trip(
            parent in prop::option::of(id_strategy()),
            action in 0u8..3,
            target in id_strategy(),
            command in ".{0,40}",
            lines in prop::collection::btree_map(LINE_NAME, id_strategy(), 1..6),
            tags in prop::option::of(id_strategy()),
            at_save in prop::option::of(id_strategy()),
            shelves in prop::collection::btree_map(".{1,10}", id_strategy(), 0..4),
            server_lines in prop::option::of(id_strategy()),
            copies in prop::collection::btree_map(".{1,20}", (1u8..=6, id_strategy(), any::<u64>()), 0..4),
            time in any::<i64>(),
        ) {
            let mut e = minimal();
            e.parent = parent;
            e.action = match action { 0 => Action::Command, 1 => Action::Undo(target), _ => Action::Redo(target) };
            e.command = command;
            e.who.time_ms = time;
            e.state.position = match at_save {
                Some(id) => Position::AtSave(id),
                None => Position::OnLine(lines.keys().next().cloned().unwrap_or_default()),
            };
            // Keep line names unique ignoring case.
            let mut seen = std::collections::BTreeSet::new();
            e.state.lines = lines.into_iter().filter(|(k, _)| seen.insert(k.to_lowercase())).collect();
            e.state.tags = tags;
            e.state.shelves = shelves.into_iter().collect();
            e.state.server_lines = server_lines;
            e.safety_copies = copies.into_iter().map(|(path, (k, id, size))| SafetyCopy {
                path, kind: EntryKind::from_u8(k).unwrap(), id, size,
            }).collect();
            let bytes = e.encode();
            prop_assert_eq!(DiaryEntry::decode(&bytes).unwrap(), e.clone());
            prop_assert_eq!(e.id(), hash_record(Flavor::DiaryEntry, &bytes));
        }
    }
}
