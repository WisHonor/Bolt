//! Helpers shared by unit tests: hex, making links, hostile text, damaged packs and the shared
//! tag and server-line records.

use crate::diary::{ServerLines, Tag, TagList};
use crate::id::Id;
use crate::pack::{ENTRY_HEADER_LEN, ObjectKind, PackReader, PackWriter};
use crate::repo::Repository;
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
    let mut pack = repo.new_pack().expect("a pack");
    let tags = pack
        .add_record(ObjectKind::TagList, &tags.encode())
        .expect("store the tag list");
    let servers = pack
        .add_record(ObjectKind::ServerLines, &servers.encode())
        .expect("store the server lines");
    entry.parent = Some(head);
    entry.command = "tag".into();
    entry.state.tags = Some(tags);
    entry.state.server_lines = Some(servers);
    repo.commit(&lock, pack, &entry).expect("commit");
    (tags, servers)
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
