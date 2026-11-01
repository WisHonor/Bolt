//! Operating-system calls for Bolt.
//!
//! This is the only crate where `unsafe` is allowed, and only for operating-system calls.
//! The owner reviews every line of it.

use std::io;
use std::path::Path;

/// Hides `path` in file browsers. On Windows this sets the "hidden" attribute; elsewhere a
/// leading dot already hides it, so this only checks that `path` exists.
pub fn hide_path(path: &Path) -> io::Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use std::os::windows::fs::MetadataExt;
        use windows_sys::Win32::Storage::FileSystem::{FILE_ATTRIBUTE_HIDDEN, SetFileAttributesW};

        let attributes = std::fs::symlink_metadata(path)?.file_attributes();
        let mut wide: Vec<u16> = path.as_os_str().encode_wide().collect();
        if wide.contains(&0) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "path contains a NUL character",
            ));
        }
        wide.push(0);
        // shortcut: no `\\?\` prefix, so paths over 260 characters fail with an error (never
        // touch the wrong file); upgrade if projects that deep turn up.
        // SAFETY: `wide` is a NUL-terminated UTF-16 string with no other NULs, and it outlives
        // the call; SetFileAttributesW only reads it and keeps no pointer to it.
        let ok = unsafe { SetFileAttributesW(wide.as_ptr(), attributes | FILE_ATTRIBUTE_HIDDEN) };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        std::fs::symlink_metadata(path).map(|_| ())
    }
}

/// Whether file browsers hide `path`: the "hidden" attribute on Windows, a leading dot elsewhere.
pub fn is_hidden(path: &Path) -> io::Result<bool> {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_HIDDEN;
        Ok(std::fs::symlink_metadata(path)?.file_attributes() & FILE_ATTRIBUTE_HIDDEN != 0)
    }
    #[cfg(not(windows))]
    {
        std::fs::symlink_metadata(path)?;
        Ok(path
            .file_name()
            .is_some_and(|n| n.as_encoded_bytes().starts_with(b".")))
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn hiding_a_folder() {
        let dir = std::env::temp_dir().join(format!("bolt-platform-{}", std::process::id()));
        let dot = dir.join(".hidden");
        std::fs::create_dir_all(&dot).unwrap();
        hide_path(&dot).unwrap();
        assert!(is_hidden(&dot).unwrap());
        assert!(hide_path(&dir.join("missing")).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
