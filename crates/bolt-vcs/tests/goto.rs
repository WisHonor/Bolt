//! `bolt goto`, end to end. For now its helpers, and a check that they do what tests need.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

/// Holds the file at `path` open, shared with nobody, as an editor or virus scanner may hold
/// one, until the handle is dropped. Another process's commands then can't replace or remove it.
#[cfg(windows)]
pub fn hold_open(path: &std::path::Path) -> std::fs::File {
    use std::os::windows::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(path)
        .unwrap()
}

#[cfg(windows)]
#[test]
fn a_held_file_cant_be_replaced_or_removed_until_let_go() {
    let dir = tempfile::tempdir().unwrap();
    let (held, other) = (dir.path().join("held"), dir.path().join("other"));
    std::fs::write(&held, b"old").unwrap();
    std::fs::write(&other, b"new").unwrap();
    let handle = hold_open(&held);
    const SHARING_VIOLATION: i32 = 32;
    // Windows' own rename says "access denied" here; Bolt's, by handle, a sharing violation.
    assert!(std::fs::rename(&other, &held).is_err());
    let removed = std::fs::remove_file(&held).unwrap_err();
    assert_eq!(removed.raw_os_error(), Some(SHARING_VIOLATION));
    drop(handle);
    std::fs::rename(&other, &held).unwrap();
    assert_eq!(std::fs::read(&held).unwrap(), b"new");
}
