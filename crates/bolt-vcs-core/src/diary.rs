//! Diary entries (docs/storage-format.md section 9): one per changing command.

use std::collections::{BTreeMap, BTreeSet};

use crate::encoding::{DecodeError, Reader, Writer};
use crate::id::{Flavor, Id, hash_record};
use crate::listing::EntryKind;
use crate::save::{MAX_PATH_BYTES, Person, read_person, write_person};

pub const MAX_NAME_BYTES: usize = 255;
pub const MAX_TAG_MESSAGE_BYTES: usize = 64 * 1024;
pub const MAX_COMMAND_BYTES: usize = 64 * 1024;
pub const MAX_VERSION_BYTES: usize = 256;
pub const MAX_LINES: u64 = 100_000;
pub const MAX_TAGS: u64 = 100_000;
pub const MAX_SHELVES: u64 = 10_000;
pub const MAX_REMOTES: u64 = 64;
pub const MAX_SAFETY_COPIES: u64 = 1_000_000;

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
}

/// The complete repository state after a command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct State {
    pub lines: BTreeMap<String, Id>,
    pub tags: BTreeMap<String, Tag>,
    pub position: Position,
    /// Newest first; labels are unique.
    pub shelves: Vec<(String, Id)>,
    pub remotes: BTreeMap<String, BTreeMap<String, Id>>,
}

/// A file Bolt kept before overwriting it, so the command can be undone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SafetyCopy {
    pub path: String,
    pub kind: EntryKind,
    pub id: Id,
    pub size: u64,
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
    if !name
        .chars()
        .all(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | '.' | '/'))
    {
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

fn write_lines(w: &mut Writer, lines: &BTreeMap<String, Id>) {
    w.varint(lines.len() as u64);
    for (name, save) in lines {
        w.string(name);
        w.id(save);
    }
}

/// Reads a sorted list of valid, case-insensitively unique line names.
fn read_lines(r: &mut Reader) -> Result<BTreeMap<String, Id>, DecodeError> {
    let n = r.count(MAX_LINES, "line count")?;
    let mut lines = BTreeMap::new();
    let mut folded = BTreeSet::new();
    let mut prev: Option<String> = None;
    for _ in 0..n {
        let name = read_sorted_name(r, &mut prev, "line name")?;
        validate_line_name(&name)?;
        if !folded.insert(name.to_lowercase()) {
            return Err(DecodeError::Invalid(
                "two line names differ only by letter case",
            ));
        }
        lines.insert(name, r.id()?);
    }
    Ok(lines)
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
        w.varint(s.tags.len() as u64);
        for (name, tag) in &s.tags {
            w.string(name);
            w.id(&tag.save);
            w.string(&tag.message);
        }
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
        w.varint(s.remotes.len() as u64);
        for (name, lines) in &s.remotes {
            w.string(name);
            write_lines(&mut w, lines);
        }
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
        let n = r.count(MAX_TAGS, "tag count")?;
        let mut tags = BTreeMap::new();
        let mut prev = None;
        for _ in 0..n {
            let name = read_sorted_name(&mut r, &mut prev, "tag name")?;
            let save = r.id()?;
            let message = r.string(MAX_TAG_MESSAGE_BYTES, "tag message")?.to_owned();
            tags.insert(name, Tag { save, message });
        }
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
        let n = r.count(MAX_REMOTES, "remote count")?;
        let mut remotes = BTreeMap::new();
        let mut prev = None;
        for _ in 0..n {
            let name = read_sorted_name(&mut r, &mut prev, "remote name")?;
            remotes.insert(name, read_lines(&mut r)?);
        }
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
                remotes,
            },
            safety_copies,
        })
    }

    #[must_use]
    pub fn id(&self) -> Id {
        hash_record(Flavor::DiaryEntry, &self.encode())
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
                tags: BTreeMap::new(),
                position: Position::OnLine("main".into()),
                shelves: vec![],
                remotes: BTreeMap::new(),
            },
            safety_copies: vec![],
        }
    }

    #[test]
    fn minimal_entry_bytes() {
        let person = format!("0141{}{}{}", "03614062", "00".repeat(8), "0000");
        let expected = [
            "01",           // version
            "00",           // no parent
            "01",           // action: command
            "057374617274", // "start"
            &person,
            "05302e312e30", // "0.1.0"
            "01046d61696e", // one line: "main"
            &"00".repeat(32),
            "00",           // no tags
            "01046d61696e", // on line "main"
            "00",           // no shelves
            "00",           // no remotes
            "00",           // no safety copies
        ]
        .concat();
        let bytes = minimal().encode();
        assert_eq!(hex(&bytes), expected);
        assert_eq!(DiaryEntry::decode(&bytes).unwrap(), minimal());
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

    fn id_strategy() -> impl Strategy<Value = Id> {
        any::<[u8; 32]>().prop_map(Id)
    }

    proptest! {
        #[test]
        fn entries_round_trip(
            parent in prop::option::of(id_strategy()),
            action in 0u8..3,
            target in id_strategy(),
            command in ".{0,40}",
            lines in prop::collection::btree_map("[a-z][a-z0-9_.-]{0,8}(/[a-z0-9_-]{1,8}){0,2}", id_strategy(), 1..6),
            tags in prop::collection::btree_map(".{1,10}", (id_strategy(), ".{0,20}"), 0..4),
            at_save in prop::option::of(id_strategy()),
            shelves in prop::collection::btree_map(".{1,10}", id_strategy(), 0..4),
            remotes in prop::collection::btree_map(".{1,10}", prop::collection::btree_map("[a-z]{1,8}", id_strategy(), 0..3), 0..3),
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
            e.state.tags = tags.into_iter().map(|(k, (save, message))| (k, Tag { save, message })).collect();
            e.state.shelves = shelves.into_iter().collect();
            e.state.remotes = remotes;
            e.safety_copies = copies.into_iter().map(|(path, (k, id, size))| SafetyCopy {
                path, kind: EntryKind::from_u8(k).unwrap(), id, size,
            }).collect();
            let bytes = e.encode();
            prop_assert_eq!(DiaryEntry::decode(&bytes).unwrap(), e.clone());
            prop_assert_eq!(e.id(), hash_record(Flavor::DiaryEntry, &bytes));
        }
    }
}
