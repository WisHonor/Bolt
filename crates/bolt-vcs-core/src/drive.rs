//! What the drive does with names that look alike. Git asks the drive the same questions when it
//! sets `core.ignorecase` and `core.precomposeunicode`.

use std::io;
use std::path::Path;

use crate::fsio;

/// Which names the drive takes to be one file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Drive {
    /// `README` and `readme`.
    pub ignores_case: bool,
    /// `é` as one character, and as `e` followed by a separate accent.
    pub composes_unicode: bool,
}

/// Tests the drive that holds `tmp_dir` with a throwaway file there, which it removes again. Call
/// it holding the change lock: opening the repository clears `tmp/` whenever the lock is free.
pub fn probe(tmp_dir: &Path) -> io::Result<Drive> {
    let (file, plain) = fsio::create_temp(tmp_dir)?;
    drop(file);
    let name = plain.file_name().unwrap_or_default().to_string_lossy();
    let accented = tmp_dir.join(format!("{name}{}", fsio::ACCENTED_SUFFIX));
    let result = (|| -> io::Result<Drive> {
        let ignores_case = tmp_dir.join(name.to_ascii_uppercase()).try_exists()?;
        // Renamed rather than made anew, so every write still goes through `fsio`.
        fsio::rename_durable(&plain, &accented)?;
        let composes_unicode = tmp_dir.join(format!("{name}-\u{e9}")).try_exists()?;
        Ok(Drive {
            ignores_case,
            composes_unicode,
        })
    })();
    // Whatever is left is only litter, which opening the repository clears too.
    for path in [&plain, &accented] {
        let _ = fsio::remove_file(path);
    }
    result
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;
    use std::fs;

    #[test]
    fn the_drive_probe_matches_the_ci_drives() {
        let tmp = tempfile::tempdir().unwrap();
        let d = probe(tmp.path()).unwrap();
        assert_eq!(d.ignores_case, cfg!(any(windows, target_os = "macos")));
        assert_eq!(d.composes_unicode, cfg!(target_os = "macos"));
    }

    #[test]
    fn the_probe_leaves_nothing_behind() {
        let tmp = tempfile::tempdir().unwrap();
        probe(tmp.path()).unwrap();
        assert_eq!(fs::read_dir(tmp.path()).unwrap().count(), 0);
    }

    #[test]
    fn a_missing_folder_is_an_error_not_an_answer() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(probe(&tmp.path().join("missing")).is_err());
    }
}
