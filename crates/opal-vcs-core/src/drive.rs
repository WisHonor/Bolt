//! What the drive does with names that look alike. Git asks the drive the same questions when it
//! sets `core.ignorecase` and `core.precomposeunicode`.

use std::ffi::OsStr;
use std::io;

use opal_platform::dir::Dir;

use crate::fsio;
use crate::names;

/// Which names the drive takes to be one file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Drive {
    /// `README` and `readme`.
    pub ignores_case: bool,
    /// `é` as one character, and as `e` followed by a separate accent.
    pub composes_unicode: bool,
}

impl Drive {
    /// `name` as this drive compares it: names that come out the same are one file here.
    #[must_use]
    pub fn compared(self, name: &str) -> String {
        names::fold(name, self.composes_unicode, self.ignores_case)
    }
}

/// How many times `probe` tests the drive before giving up on a file that keeps going away.
const TRIES: usize = 5;

/// Tests the drive that holds the folder `tmp` with a throwaway file there, which it removes
/// again. It runs without the change lock too (`status`, `changes`, the command line's early
/// checks), and opening a project clears `tmp/` while the lock is free: a file taken away midway
/// gives no answer, so it tests again.
pub fn probe(tmp: &Dir) -> io::Result<Drive> {
    for _ in 0..TRIES {
        if let Some(drive) = probe_once(tmp)? {
            return Ok(drive);
        }
    }
    Err(io::Error::other(
        "the file Opalro tests the drive with kept disappearing from .opal/tmp, which other Opalro \
         commands clear; run this one again",
    ))
}

/// One test of the drive, or `None` if its file went away before the answer was sure.
fn probe_once(tmp: &Dir) -> io::Result<Option<Drive>> {
    let (file, plain) = fsio::create_temp(tmp)?;
    drop(file);
    let name = plain.to_string_lossy().into_owned();
    let accented = format!("{name}{}", fsio::ACCENTED_SUFFIX);
    let result = (|| -> io::Result<Option<Drive>> {
        fsio::act_point("drive:case");
        let ignores_case = there(tmp, &name.to_ascii_uppercase())?;
        // Renamed rather than made anew, so every write still goes through `fsio`. It finds the
        // file by its exact name, so it fails if the file was gone for the check above.
        match fsio::rename_plain(tmp, &plain, tmp, OsStr::new(&accented)) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            renamed => renamed?,
        }
        fsio::act_point("drive:unicode");
        let composes_unicode = there(tmp, &format!("{name}-\u{e9}"))?;
        if !there(tmp, &accented)? {
            return Ok(None);
        }
        Ok(Some(Drive {
            ignores_case,
            composes_unicode,
        }))
    })();
    // Whatever is left is only litter, which opening the repository clears too.
    for left in [plain.as_os_str(), OsStr::new(&accented)] {
        let _ = fsio::remove_file(tmp, left);
    }
    result
}

/// Whether the folder `dir` holds something under the name `name`, as this drive compares names.
fn there(dir: &Dir, name: &str) -> io::Result<bool> {
    match dir.metadata(OsStr::new(name)) {
        Ok(_) => Ok(true),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;
    use std::fs;

    fn held(tmp: &tempfile::TempDir) -> Dir {
        Dir::open_top(tmp.path()).unwrap()
    }

    #[test]
    fn the_drive_probe_matches_the_ci_drives() {
        let tmp = tempfile::tempdir().unwrap();
        let d = probe(&held(&tmp)).unwrap();
        assert_eq!(d.ignores_case, cfg!(any(windows, target_os = "macos")));
        assert_eq!(d.composes_unicode, cfg!(target_os = "macos"));
    }

    #[test]
    fn names_are_one_file_where_the_drive_takes_them_for_one() {
        let drive = |ignores_case, composes_unicode| Drive {
            ignores_case,
            composes_unicode,
        };
        let one_file = |d: Drive, a, b| d.compared(a) == d.compared(b);
        let (upper, lower) = ("README", "readme");
        let (nfc, nfd) = ("caf\u{e9}", "cafe\u{301}");
        assert!(one_file(drive(true, false), upper, lower));
        assert!(!one_file(drive(true, false), nfc, nfd));
        assert!(one_file(drive(false, true), nfc, nfd));
        assert!(!one_file(drive(false, true), upper, lower));
        assert!(!one_file(drive(false, false), upper, lower));
        // Both at once, as on macOS.
        assert!(one_file(drive(true, true), "CAF\u{c9}", nfd));
    }

    #[test]
    fn the_probe_leaves_nothing_behind() {
        let tmp = tempfile::tempdir().unwrap();
        probe(&held(&tmp)).unwrap();
        assert_eq!(fs::read_dir(tmp.path()).unwrap().count(), 0);
    }

    /// Opening a project clears `tmp/` whenever the change lock is free, so another command can
    /// take the probe's file away midway: what is found then is no answer, and it probes again.
    #[cfg(feature = "fault-injection")]
    #[test]
    fn a_probe_file_removed_midway_is_probed_again() {
        use crate::fsio::fault;
        fn clear(tmp: &std::path::Path) {
            for left in fs::read_dir(tmp).unwrap() {
                fs::remove_file(left.unwrap().path()).unwrap();
            }
        }
        let tmp = tempfile::tempdir().unwrap();
        let at = tmp.path().to_owned();
        // Before the case check, then before the Unicode check of the next try.
        fault::act_at("drive:case", move || {
            clear(&at);
            fault::act_at("drive:unicode", move || clear(&at));
        });
        let d = probe(&held(&tmp)).unwrap();
        assert_eq!(d.ignores_case, cfg!(any(windows, target_os = "macos")));
        assert_eq!(d.composes_unicode, cfg!(target_os = "macos"));
        assert_eq!(fs::read_dir(tmp.path()).unwrap().count(), 0);
    }

    #[test]
    fn a_missing_folder_is_an_error_not_an_answer() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir(tmp.path().join("gone")).unwrap();
        let gone = Dir::open_top(&tmp.path().join("gone")).unwrap();
        fs::remove_dir(tmp.path().join("gone")).unwrap();
        assert!(probe(&gone).is_err());
    }
}
