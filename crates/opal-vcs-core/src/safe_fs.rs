//! The one way Opalro opens files it doesn't trust (everything in `.opalro/`, settings files and
//! ignore files): links are never followed (except to the user's own settings), only regular
//! files are accepted, and reads are capped (storage-format.md §10 and §12).

use std::ffi::OsStr;
use std::fs::{self, File, Metadata, OpenOptions};
use std::io::{self, Read};
use std::path::Path;

use opal_platform::dir::{Dir, EntryType};

// Read limits (storage-format.md §10 and §12).
pub const MAX_FORMAT_BYTES: u64 = 4096;
pub const MAX_HEAD_BYTES: u64 = 65;
pub const MAX_LOCK_INFO_BYTES: u64 = 4096;
/// `state/tidied`: a number of milliseconds and an LF.
pub const MAX_TIDIED_BYTES: u64 = 32;
pub const MAX_CONFIG_BYTES: u64 = 1 << 20;
/// One `.gitignore` or `.opalroignore`.
pub const MAX_IGNORE_FILE_BYTES: u64 = 1 << 20;
/// The ignore files in force at one folder (its own and those above it), together.
pub const MAX_IGNORE_IN_FORCE_BYTES: u64 = 1 << 20;
/// All the ignore files one save reads, together.
pub const MAX_IGNORE_SAVE_BYTES: u64 = 16 << 20;

/// Opens `path` with `options` (custom flags are replaced) without following a link at its
/// last part, and only if it is a regular file. A link, folder, pipe or device gives
/// `InvalidInput`; any other error from the open, `NotFound` included, comes back unchanged.
/// Don't pass `truncate`: it acts when the file is opened, before a file that isn't a regular one
/// is refused. No caller does.
pub fn open_regular(path: &Path, options: &OpenOptions) -> io::Result<File> {
    let file = match opal_platform::no_follow(&mut options.clone()).open(path) {
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
    read_to_cap(
        open_regular(path, OpenOptions::new().read(true))?,
        path,
        max,
    )
}

/// Opens the regular file `name` in the folder `dir` to read, as `open_regular` opens a path: a
/// link, folder, pipe or device gives `InvalidInput`, naming it `shown`, and any other error from
/// the open comes back unchanged.
pub(crate) fn open_regular_in(dir: &Dir, name: &OsStr, shown: &Path) -> io::Result<File> {
    dir.open_file(name)
        .map_err(|e| refused_in(dir, name, shown, e))
}

/// Opens the regular file `name` in the folder `dir` to read and write, making it if nothing has
/// that name and keeping what is in it, with `open_regular_in`'s refusals.
pub(crate) fn open_or_create_in(dir: &Dir, name: &OsStr, shown: &Path) -> io::Result<File> {
    dir.open_or_create(name)
        .map_err(|e| refused_in(dir, name, shown, e))
}

/// `e`, from opening `name` in `dir`, as `not_regular` names it when what is there is a link,
/// folder or special file.
fn refused_in(dir: &Dir, name: &OsStr, shown: &Path, e: io::Error) -> io::Error {
    // The open has already refused; this look only picks the message.
    let not_a_file = || dir.metadata(name).is_ok_and(|m| m.kind != EntryType::File);
    if e.kind() != io::ErrorKind::NotFound && not_a_file() {
        not_regular(shown)
    } else {
        e
    }
}

/// Reads the regular file `name` in the folder `dir` as `read_capped` reads a path; see
/// `open_regular_in`.
pub(crate) fn read_capped_in(
    dir: &Dir,
    name: &OsStr,
    shown: &Path,
    max: u64,
) -> io::Result<Vec<u8>> {
    read_to_cap(open_regular_in(dir, name, shown)?, shown, max)
}

/// Like `read_capped`, but follows links. Only for the user's own settings files, which dotfile
/// managers keep as links; never for a file a project can bring. What the link reaches must
/// still be a regular file.
pub(crate) fn read_capped_following_links(path: &Path, max: u64) -> io::Result<Vec<u8>> {
    // Looked at before opening, because opening a named pipe waits for a writer.
    // shortcut: a pipe swapped in between the look and the open still waits, which needs write
    // access to the user's own files; open without blocking (platform crate) if that matters.
    if !fs::metadata(path)?.is_file() {
        return Err(not_regular(path));
    }
    read_to_cap(File::open(path)?, path, max)
}

/// Reads `file`, refusing more than `max` bytes without reading past `max + 1`.
fn read_to_cap(file: File, path: &Path, max: u64) -> io::Result<Vec<u8>> {
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

/// Reads into `buf` from offset `pos` of `file`, as much as one read gives, so readers of one
/// file never share a cursor.
pub fn read_at(file: &File, buf: &mut [u8], pos: u64) -> io::Result<usize> {
    #[cfg(unix)]
    {
        std::os::unix::fs::FileExt::read_at(file, buf, pos)
    }
    #[cfg(windows)]
    {
        std::os::windows::fs::FileExt::seek_read(file, buf, pos)
    }
}

/// The drive and file number of the folder at `path`, from one open of it that doesn't follow its
/// last name: a link, junction or other reparse point there is refused
/// (`opal_platform::dir::is_link_refusal`). Links before the last name are followed. A trailing
/// separator is dropped first, since on Unix it would make the system follow a link.
pub fn folder_id(path: &Path) -> io::Result<(u64, u128)> {
    let tidied: std::path::PathBuf = path.components().collect();
    Ok(Dir::open_top(&tidied)?.own_meta()?.file_id)
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

/// Whether `meta`, from `symlink_metadata`, is a link, junction or other reparse point: what Opalro
/// never follows.
pub fn is_link(meta: &Metadata) -> bool {
    meta.is_symlink() || is_reparse_point(meta)
}

/// A plain file: not a link, folder, pipe or device, and on Windows no reparse point at all.
/// `meta` comes from `symlink_metadata`.
pub fn is_regular(meta: &Metadata) -> bool {
    meta.is_file() && !is_reparse_point(meta)
}

/// Windows reparse points: links, junctions and other redirections. Opalro trusts none of them.
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
    #[cfg(windows)]
    use crate::test_support::junction;
    use crate::test_support::symlink;

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
    fn following_read_takes_links_to_regular_files_only() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("config.toml");
        fs::write(&target, b"abc").unwrap();
        assert_eq!(read_capped_following_links(&target, 3).unwrap(), b"abc");
        let err = read_capped_following_links(&target, 2).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        let missing = read_capped_following_links(&dir.path().join("missing"), 3);
        assert_eq!(missing.unwrap_err().kind(), io::ErrorKind::NotFound);
        assert_not_regular(read_capped_following_links(dir.path(), 3), dir.path());
        let link = dir.path().join("link");
        if symlink(&target, &link) {
            assert_eq!(read_capped_following_links(&link, 3).unwrap(), b"abc");
        }
    }

    #[cfg(unix)]
    #[test]
    fn following_read_refuses_pipes_and_devices_without_hanging() {
        let dir = tempfile::tempdir().unwrap();
        let fifo = dir.path().join("config.toml");
        let made = std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .unwrap();
        assert!(made.success());
        let zero = dir.path().join("zero");
        std::os::unix::fs::symlink("/dev/zero", &zero).unwrap();
        // A hang fails this test instead of stopping the whole run.
        let (tx, rx) = std::sync::mpsc::channel();
        let (f, z) = (fifo.clone(), zero.clone());
        std::thread::spawn(move || {
            let from_fifo = read_capped_following_links(&f, 65).map(|_| ());
            let from_zero = read_capped_following_links(&z, 65).map(|_| ());
            let _ = tx.send((from_fifo, from_zero));
        });
        let (from_fifo, from_zero) = rx
            .recv_timeout(std::time::Duration::from_secs(10))
            .expect("reading a named pipe hung");
        assert_not_regular(from_fifo, &fifo);
        assert_not_regular(from_zero, &zero);
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
        let real = dir.path().join(".opalro");
        fs::create_dir(&real).unwrap();
        // Hiding `.opalro` (as `opalro start` does) must not make it look like a link.
        opal_platform::hide_path(&real).unwrap();
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
