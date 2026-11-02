//! Helpers shared by unit tests: hex, making links, hostile text and damaged packs.

use crate::pack::{ENTRY_HEADER_LEN, ObjectKind, PackReader, PackWriter};
use std::path::Path;

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
