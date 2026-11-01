//! The one way Bolt opens files it doesn't trust (everything in `.bolt/`, settings files and
//! `.boltignore`): links are never followed, only regular files are accepted, and reads are
//! capped (storage-format.md §10 and §12).

use std::fs::{self, File, Metadata, OpenOptions};
use std::io::{self, Read};
use std::path::Path;

// Read limits (storage-format.md §10 and §12).
pub const MAX_FORMAT_BYTES: u64 = 4096;
pub const MAX_HEAD_BYTES: u64 = 65;
pub const MAX_LOCK_INFO_BYTES: u64 = 4096;
pub const MAX_CONFIG_BYTES: u64 = 1 << 20;
pub const MAX_BOLTIGNORE_BYTES: u64 = 1 << 20;

/// Opens `path` with `options` (custom flags are replaced) without following a link at its
/// last part, and only if it is a regular file. A link, folder, pipe or device gives
/// `InvalidInput`; any other error from the open, `NotFound` included, comes back unchanged.
pub fn open_regular(path: &Path, options: &OpenOptions) -> io::Result<File> {
    let file = match bolt_platform::no_follow(&mut options.clone()).open(path) {
        Ok(file) => file,
        // The open has already refused; this look only picks the message. It catches a link
        // on Unix (`ELOOP`), a folder on Windows (`PermissionDenied`), a folder opened for
        // writing on Unix (`EISDIR`) and a pipe opened for writing on Unix (`ENXIO`).
        Err(e)
            if e.kind() != io::ErrorKind::NotFound
                && fs::symlink_metadata(path).is_ok_and(|m| !is_regular(&m)) =>
        {
            return Err(not_regular(path));
        }
        Err(e) => return Err(e),
    };
    // A Windows device opens, then fails the metadata query itself ("Incorrect function"): a
    // handle that can't be shown to be a regular file counts as not one.
    if file.metadata().is_ok_and(|m| is_regular(&m)) {
        Ok(file)
    } else {
        Err(not_regular(path))
    }
}

/// Reads the regular file at `path` (see `open_regular`), refusing one larger than `max` bytes
/// without reading past `max + 1`.
pub fn read_capped(path: &Path, max: u64) -> io::Result<Vec<u8>> {
    let file = open_regular(path, OpenOptions::new().read(true))?;
    let mut bytes = Vec::new();
    file.take(max.saturating_add(1)).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > max {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{} is larger than {max} bytes", path.display()),
        ));
    }
    Ok(bytes)
}

/// Checks that `path` is a real folder: not a link, junction or other reparse point, and not a
/// file. `NotFound` comes back unchanged.
pub fn check_real_dir(path: &Path) -> io::Result<()> {
    let meta = fs::symlink_metadata(path)?;
    if meta.is_dir() && !is_reparse_point(&meta) {
        return Ok(());
    }
    Err(io::Error::new(
        io::ErrorKind::InvalidInput,
        format!(
            "{} is not a real folder (it is a link, junction or file)",
            path.display()
        ),
    ))
}

/// A plain file: not a link, folder, pipe or device, and on Windows no reparse point at all.
fn is_regular(meta: &Metadata) -> bool {
    meta.is_file() && !is_reparse_point(meta)
}

/// Windows reparse points: links, junctions and other redirections. Bolt trusts none of them.
fn is_reparse_point(meta: &Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }
    #[cfg(not(windows))]
    {
        let _ = meta;
        false
    }
}

fn not_regular(path: &Path) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        format!(
            "{} is not a regular file (it is a link, folder or special file)",
            path.display()
        ),
    )
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;

    /// The options the change-lock file is opened with: read, write, create, never truncate.
    fn lock_options() -> OpenOptions {
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        options
    }

    fn assert_not_regular<T: std::fmt::Debug>(result: io::Result<T>, path: &Path) {
        let err = result.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput, "{err}");
        let expected = "is not a regular file (it is a link, folder or special file)";
        assert_eq!(err.to_string(), format!("{} {expected}", path.display()));
    }

    fn assert_not_real_dir(result: io::Result<()>, path: &Path) {
        let err = result.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput, "{err}");
        let expected = "is not a real folder (it is a link, junction or file)";
        assert_eq!(err.to_string(), format!("{} {expected}", path.display()));
    }

    /// Makes `link` point at `target`; false, with a note, where Windows lacks the privilege.
    fn symlink(target: &Path, link: &Path) -> bool {
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(target, link).unwrap();
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
    fn junction(link: &Path, target: &Path) {
        let made = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(link)
            .arg(target)
            .stdout(std::process::Stdio::null())
            .status()
            .unwrap();
        assert!(made.success());
    }

    #[test]
    fn regular_file_reads_back() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("head");
        fs::write(&path, b"abc").unwrap();
        assert_eq!(read_capped(&path, 3).unwrap(), b"abc");
    }

    #[test]
    fn missing_file_stays_not_found() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("missing");
        let read = read_capped(&path, 3);
        assert_eq!(read.unwrap_err().kind(), io::ErrorKind::NotFound);
        let opened = open_regular(&path, OpenOptions::new().read(true));
        assert_eq!(opened.unwrap_err().kind(), io::ErrorKind::NotFound);
    }

    #[cfg(unix)]
    #[test]
    fn permission_denied_comes_back_unchanged() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("head");
        fs::write(&path, b"abc").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o000)).unwrap();
        if fs::read(&path).is_ok() {
            eprintln!("note: skipped, this user can read any file (root)");
            return;
        }
        let read = read_capped(&path, 3);
        assert_eq!(read.unwrap_err().kind(), io::ErrorKind::PermissionDenied);
    }

    #[test]
    fn over_the_cap_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("head");
        fs::write(&path, b"abcd").unwrap();
        let err = read_capped(&path, 3).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        let expected = format!("{} is larger than 3 bytes", path.display());
        assert_eq!(err.to_string(), expected);
    }

    #[test]
    fn symlink_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("target");
        fs::write(&target, b"keep").unwrap();
        let link = dir.path().join("link");
        if !symlink(&target, &link) {
            return;
        }
        assert_not_regular(read_capped(&link, 100), &link);
        assert_not_regular(open_regular(&link, &lock_options()), &link);
        assert_eq!(fs::read(&target).unwrap(), b"keep");
        // A link to a missing file: opening it to create must not create the target.
        fs::remove_file(&target).unwrap();
        assert_not_regular(open_regular(&link, &lock_options()), &link);
        assert!(!target.exists());
    }

    #[cfg(unix)]
    #[test]
    fn fifo_is_refused_without_hanging() {
        let dir = tempfile::tempdir().unwrap();
        let fifo = dir.path().join("head");
        let made = std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .unwrap();
        assert!(made.success());
        // Opening a named pipe can wait forever for the other end, so open it on another
        // thread: a hang fails this test instead of stopping the whole run.
        let (tx, rx) = std::sync::mpsc::channel();
        let path = fifo.clone();
        std::thread::spawn(move || {
            let read = read_capped(&path, 65).map(|_| ());
            let write = open_regular(&path, OpenOptions::new().write(true)).map(|_| ());
            let _ = tx.send((read, write));
        });
        let (read, write) = rx
            .recv_timeout(std::time::Duration::from_secs(10))
            .expect("opening a named pipe hung");
        assert_not_regular(read, &fifo);
        assert_not_regular(write, &fifo);
    }

    #[test]
    fn device_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        // On Windows `nul` in any folder is the NUL device, whose metadata query itself fails.
        let device = if cfg!(windows) {
            dir.path().join("nul")
        } else {
            std::path::PathBuf::from("/dev/null")
        };
        let opened = open_regular(&device, OpenOptions::new().read(true));
        assert_not_regular(opened, &device);
        assert_not_regular(read_capped(&device, 65), &device);
    }

    #[test]
    fn folder_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("packs");
        fs::create_dir(&folder).unwrap();
        assert_not_regular(
            open_regular(&folder, OpenOptions::new().read(true)),
            &folder,
        );
        assert_not_regular(open_regular(&folder, &lock_options()), &folder);
        #[cfg(windows)]
        {
            let link = dir.path().join("junction");
            junction(&link, &folder);
            assert_not_regular(open_regular(&link, OpenOptions::new().read(true)), &link);
            assert_not_regular(open_regular(&link, &lock_options()), &link);
        }
    }

    #[test]
    fn check_real_dir_refuses_links_and_files() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join(".bolt");
        fs::create_dir(&real).unwrap();
        // Hiding `.bolt` (as `bolt start` does) must not make it look like a link.
        bolt_platform::hide_path(&real).unwrap();
        check_real_dir(&real).unwrap();
        let file = dir.path().join("format");
        fs::write(&file, b"x").unwrap();
        assert_not_real_dir(check_real_dir(&file), &file);
        let missing = check_real_dir(&dir.path().join("missing"));
        assert_eq!(missing.unwrap_err().kind(), io::ErrorKind::NotFound);
        let link = dir.path().join("link");
        if symlink(&real, &link) {
            assert_not_real_dir(check_real_dir(&link), &link);
        }
        #[cfg(windows)]
        {
            let link = dir.path().join("junction");
            junction(&link, &real);
            assert_not_real_dir(check_real_dir(&link), &link);
        }
    }
}
