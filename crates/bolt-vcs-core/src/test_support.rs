//! Helpers shared by unit tests: hex, making links, hostile text, damaged packs, planted saves and
//! the shared tag and server-line records.

use crate::diary::{Position, ServerLines, Tag, TagList};
use crate::encoding::Writer;
use crate::id::{ChangeId, Id};
use crate::listing::{Entry, EntryKind, Listing};
use crate::pack::{ENTRY_HEADER_LEN, ObjectKind, PackReader, PackWriter};
use crate::repo::Repository;
use crate::save::Save;
use std::collections::BTreeMap;
use std::path::Path;

/// One tag and one server line, both at a made-up save.
pub(crate) fn sample_records() -> (TagList, ServerLines) {
    let save = Id([0x5a; 32]);
    let tag = Tag {
        save,
        message: "First playable".into(),
        git_tags: vec![],
    };
    let tags = TagList {
        tags: BTreeMap::from([("v1".to_owned(), tag)]),
    };
    let servers = ServerLines {
        servers: BTreeMap::from([(
            "origin".to_owned(),
            BTreeMap::from([("main".to_owned(), save)]),
        )]),
    };
    (tags, servers)
}

/// Commits an entry like the current one that points at `tags` and `servers`, stored with it.
/// Returns the IDs of the two records.
pub(crate) fn commit_records(
    repo: &mut Repository,
    tags: &TagList,
    servers: &ServerLines,
) -> (Id, Id) {
    let lock = repo.lock_for_change("tag", &mut |_| {}).expect("the lock");
    let (head, mut entry) = repo.current().expect("a current entry");
    let mut packs = repo.new_packs().expect("a pack set");
    let tags = packs
        .add_record(ObjectKind::TagList, &tags.encode())
        .expect("store the tag list");
    let servers = packs
        .add_record(ObjectKind::ServerLines, &servers.encode())
        .expect("store the server lines");
    entry.parent = Some(head);
    entry.command = "tag".into();
    entry.state.tags = Some(tags);
    entry.state.server_lines = Some(servers);
    repo.commit(&lock, packs, &entry).expect("commit");
    (tags, servers)
}

/// Commits a save on the current line that holds `entries` and the folders above them, whatever
/// the project folder holds, and returns its top listing. Each entry is a path with `/` between
/// names, a kind, and bytes: a file's contents, a link's target, or a Git submodule's commit ID.
pub(crate) fn plant_save(repo: &mut Repository, entries: &[(&str, EntryKind, &[u8])]) -> Listing {
    let lock = repo.lock_for_change("save", &mut |_| {}).expect("the lock");
    let (head, mut diary) = repo.current().expect("a current entry");
    let Position::OnLine(line) = diary.state.position.clone() else {
        panic!("the project is not on a line");
    };
    let parent_id = diary.state.lines[&line];
    let parent = repo
        .store()
        .save(&parent_id, false)
        .expect("the line's save");
    let mut packs = repo.new_packs().expect("a pack set");
    let mut folders = BTreeMap::from([(String::new(), Vec::new())]);
    for &(path, kind, bytes) in entries {
        let (id, size) = if kind == EntryKind::ProjectLink {
            // As an import makes it (storage-format.md §7).
            let mut link = Writer::new();
            link.u8(1);
            link.string("git");
            link.string("");
            link.bytes(bytes);
            (
                packs.add_record(ObjectKind::ProjectLink, &link.into_bytes()),
                0,
            )
        } else {
            (packs.add_content(bytes), bytes.len() as u64)
        };
        let (up, name) = path.rsplit_once('/').unwrap_or(("", path));
        folders.entry(up.to_owned()).or_default().push(Entry {
            name: name.to_owned(),
            kind,
            id: id.expect("store an entry"),
            size,
        });
    }
    // Longest path first, so a folder's listing is stored before the folder that holds it.
    let (root, top) = loop {
        let path = folders
            .keys()
            .max_by_key(|p| p.len())
            .cloned()
            .expect("a folder");
        let listing = Listing::new(folders.remove(&path).expect("its entries")).expect("a listing");
        let id = packs
            .add_record(ObjectKind::FolderListing, &listing.encode())
            .expect("store a listing");
        if path.is_empty() {
            break (id, listing);
        }
        let (up, name) = path.rsplit_once('/').unwrap_or(("", &path));
        folders.entry(up.to_owned()).or_default().push(Entry {
            name: name.to_owned(),
            kind: EntryKind::Folder,
            id,
            size: 0,
        });
    };
    let save = Save {
        root,
        parents: vec![parent_id],
        author: parent.author,
        saver: parent.saver,
        message: "Planted".into(),
        change_id: ChangeId::random().expect("a change ID"),
        move_notes: vec![],
        signature: vec![],
        extra: parent.extra,
    };
    let save_id = packs
        .add_record(ObjectKind::Save, &save.encode())
        .expect("store the save");
    diary.parent = Some(head);
    diary.command = "save".into();
    diary.state.lines.insert(line, save_id);
    repo.commit(&lock, packs, &diary).expect("commit");
    top
}

/// Writes `00.pack` into `packs`: `objects` (kind and bytes), each with its first stored byte
/// flipped so it fails its fingerprint. The name sorts before every pack Bolt names.
pub(crate) fn plant_damaged_copies(packs: &Path, objects: &[(ObjectKind, &[u8])]) {
    let made = tempfile::tempdir().expect("a temporary folder");
    let mut w = PackWriter::create(made.path()).expect("a pack writer");
    for (kind, bytes) in objects {
        match kind {
            ObjectKind::Content => w.add_content(bytes),
            _ => w.add_record(*kind, bytes),
        }
        .expect("add an object");
    }
    let path = w.finish(made.path()).expect("finish").expect("a pack");
    let mut pack = std::fs::read(&path).expect("read the pack");
    for e in PackReader::open(&path).expect("open the pack").entries() {
        assert!(e.stored_size > 0, "an empty object has no byte to flip");
        pack[usize::try_from(e.offset + ENTRY_HEADER_LEN).expect("an offset")] ^= 1;
    }
    std::fs::write(packs.join("00.pack"), pack).expect("write the pack");
}

/// Text no terminal may be shown as it is: an escape sequence, a right-to-left override and a
/// line separator.
pub(crate) const HOSTILE: &str = "a\u{1b}[2Jb\u{202e}c\u{2028}d";

/// Fails unless `message` holds [`HOSTILE`] escaped, and none of its characters as they are.
pub(crate) fn assert_hostile_text_escaped(message: &str) {
    let escaped = "a\\x1b[2Jb\\u{202e}c\\u{2028}d";
    assert!(message.contains(escaped), "{message:?}");
    assert!(
        !message.contains(['\u{1b}', '\u{202e}', '\u{2028}']),
        "{message:?}"
    );
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub(crate) fn unhex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).expect("test hex"))
        .collect()
}

/// Makes `link` point at `target`; false, with a note, where Windows lacks the privilege.
pub(crate) fn symlink(target: &Path, link: &Path) -> bool {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, link).expect("make a symlink");
        true
    }
    #[cfg(windows)]
    {
        let made = if target.is_dir() {
            std::os::windows::fs::symlink_dir(target, link)
        } else {
            std::os::windows::fs::symlink_file(target, link)
        };
        match made {
            Ok(()) => true,
            // ERROR_PRIVILEGE_NOT_HELD: symlinks need Developer Mode or admin rights.
            Err(e) if e.raw_os_error() == Some(1314) => {
                eprintln!("note: symlink check skipped, this account can't create symlinks");
                false
            }
            Err(e) => panic!("can't create a symlink: {e}"),
        }
    }
}

/// A Windows junction (no privilege needed) at `link`, pointing at the folder `target`.
#[cfg(windows)]
pub(crate) fn junction(link: &Path, target: &Path) {
    // `/D` skips cmd's AutoRun hooks; mklink's "Junction created" line is not wanted in logs.
    let made = std::process::Command::new("cmd")
        .args(["/D", "/C", "mklink", "/J"])
        .arg(link)
        .arg(target)
        .stdout(std::process::Stdio::null())
        .status()
        .expect("run mklink");
    assert!(made.success(), "mklink /J {} failed", link.display());
}
