//! `Dir` on macOS and Linux: the `*at` calls, through rustix, which needs no `unsafe` here.

use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::io;
use std::os::fd::{AsFd, OwnedFd};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::Path;

use rustix::fs::{AtFlags, CWD, FileType, Mode, OFlags, Stat};
use rustix::io::Errno;

use super::{DirEntry, EntryMeta, EntryType, is_a_folder, link_refused};

pub(super) type Handle = OwnedFd;

/// A folder, never through a link at its name.
const FOLDER: OFlags = OFlags::RDONLY
    .union(OFlags::DIRECTORY)
    .union(OFlags::NOFOLLOW)
    .union(OFlags::CLOEXEC);

/// A file to read, never through a link at its name. O_NONBLOCK: a pipe swapped in fails instead
/// of waiting for a writer.
const TO_READ: OFlags = OFlags::RDONLY
    .union(OFlags::NOFOLLOW)
    .union(OFlags::CLOEXEC)
    .union(OFlags::NONBLOCK);

pub(super) fn open_top(path: &Path) -> io::Result<OwnedFd> {
    rustix::fs::open(path, FOLDER, Mode::empty()).map_err(|e| refusal(e, CWD, path.as_os_str()))
}

pub(super) fn open_dir(dir: &OwnedFd, name: &OsStr) -> io::Result<OwnedFd> {
    rustix::fs::openat(dir, name, FOLDER, Mode::empty()).map_err(|e| refusal(e, dir, name))
}

pub(super) fn entries(dir: &OwnedFd) -> io::Result<Vec<DirEntry>> {
    let mut listing = rustix::fs::Dir::read_from(dir)?;
    let mut entries = Vec::new();
    while let Some(entry) = listing.read() {
        let entry = entry?;
        let name = OsStr::from_bytes(entry.file_name().to_bytes());
        if name == "." || name == ".." {
            continue;
        }
        let kind = match entry.file_type() {
            // Some drives don't say in the listing.
            FileType::Unknown => match rustix::fs::statat(dir, name, AtFlags::SYMLINK_NOFOLLOW) {
                Ok(stat) => kind_of(FileType::from_raw_mode(stat.st_mode)),
                Err(Errno::NOENT) => continue, // gone since it was listed
                Err(e) => return Err(e.into()),
            },
            known => kind_of(known),
        };
        entries.push(DirEntry {
            name: name.to_owned(),
            kind,
            folder: kind == EntryType::Dir,
            short_name: None,
        });
    }
    Ok(entries)
}

pub(super) fn metadata(dir: &OwnedFd, name: &OsStr) -> io::Result<EntryMeta> {
    Ok(meta(&rustix::fs::statat(
        dir,
        name,
        AtFlags::SYMLINK_NOFOLLOW,
    )?))
}

pub(super) fn file_meta(file: &File) -> io::Result<EntryMeta> {
    Ok(meta(&rustix::fs::fstat(file)?))
}

pub(super) fn open_file(dir: &OwnedFd, name: &OsStr) -> io::Result<File> {
    let fd =
        rustix::fs::openat(dir, name, TO_READ, Mode::empty()).map_err(|e| refusal(e, dir, name))?;
    regular(fd)
}

pub(super) fn create_new(dir: &OwnedFd, name: &OsStr, executable: bool) -> io::Result<File> {
    let flags = OFlags::RDWR | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC;
    let fd = rustix::fs::openat(dir, name, flags, mode(executable))?;
    // The umask may have taken bits away. A drive that can't hold modes (FAT, say) refuses: the
    // new, empty file goes again, so trying once more doesn't find it in the way.
    if let Err(e) = rustix::fs::fchmod(&fd, mode(executable)) {
        let _ = rustix::fs::unlinkat(dir, name, AtFlags::empty());
        return Err(e.into());
    }
    Ok(File::from(fd))
}

pub(super) fn open_or_create(dir: &OwnedFd, name: &OsStr) -> io::Result<File> {
    // O_NONBLOCK, as for reading: a pipe there fails instead of waiting.
    let flags =
        OFlags::RDWR | OFlags::CREATE | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK;
    let fd =
        rustix::fs::openat(dir, name, flags, mode(false)).map_err(|e| refusal(e, dir, name))?;
    regular(fd)
}

pub(super) fn rename_into(
    dir: &OwnedFd,
    from: &OsStr,
    to: &OwnedFd,
    to_name: &OsStr,
) -> io::Result<()> {
    // A rename never follows a link at either name; these checks refuse what Windows refuses.
    if kind_at(dir, from)? == Some(EntryType::Symlink) {
        return Err(link_refused());
    }
    match kind_at(to, to_name)? {
        Some(EntryType::Symlink) => return Err(link_refused()),
        Some(EntryType::Dir) => return Err(is_a_folder()),
        _ => {}
    }
    Ok(rustix::fs::renameat(dir, from, to, to_name)?)
}

pub(super) fn create_dir(dir: &OwnedFd, name: &OsStr) -> io::Result<()> {
    Ok(rustix::fs::mkdirat(dir, name, Mode::from_raw_mode(0o777))?)
}

pub(super) fn remove_file(dir: &OwnedFd, name: &OsStr) -> io::Result<()> {
    Ok(rustix::fs::unlinkat(dir, name, AtFlags::empty())?)
}

pub(super) fn remove_dir(dir: &OwnedFd, name: &OsStr) -> io::Result<()> {
    rustix::fs::unlinkat(dir, name, AtFlags::REMOVEDIR).map_err(|e| refusal(e, dir, name))
}

pub(super) fn symlink(
    dir: &OwnedFd,
    name: &OsStr,
    target: &OsStr,
    _to_dir: bool,
    _scratch: &OwnedFd,
) -> io::Result<()> {
    Ok(rustix::fs::symlinkat(target, dir, name)?)
}

pub(super) fn read_link(dir: &OwnedFd, name: &OsStr) -> io::Result<OsString> {
    let target = rustix::fs::readlinkat(dir, name, Vec::new())?;
    Ok(OsString::from_vec(target.into_bytes()))
}

pub(super) fn set_executable(dir: &OwnedFd, name: &OsStr, executable: bool) -> io::Result<()> {
    let fd =
        rustix::fs::openat(dir, name, TO_READ, Mode::empty()).map_err(|e| refusal(e, dir, name))?;
    Ok(rustix::fs::fchmod(regular(fd)?, mode(executable))?)
}

pub(super) fn sync(dir: &OwnedFd) -> io::Result<()> {
    // As std's File::sync_all: on Apple systems only F_FULLFSYNC reaches the disk itself.
    #[cfg(target_vendor = "apple")]
    rustix::fs::fcntl_fullfsync(dir)?;
    #[cfg(not(target_vendor = "apple"))]
    rustix::fs::fsync(dir)?;
    Ok(())
}

fn mode(executable: bool) -> Mode {
    Mode::from_raw_mode(if executable { 0o755 } else { 0o644 })
}

/// A link refusal if `name` is a link, else `e`. Opening or removing through a link fails with
/// ELOOP or ENOTDIR, which other things give too.
fn refusal(e: Errno, dir: impl AsFd, name: &OsStr) -> io::Error {
    if matches!(e, Errno::LOOP | Errno::NOTDIR)
        && kind_at(dir, name).is_ok_and(|kind| kind == Some(EntryType::Symlink))
    {
        return link_refused();
    }
    e.into()
}

/// What `name` is, or `None` if nothing has that name.
fn kind_at(dir: impl AsFd, name: &OsStr) -> io::Result<Option<EntryType>> {
    match rustix::fs::statat(dir, name, AtFlags::SYMLINK_NOFOLLOW) {
        Ok(stat) => Ok(Some(kind_of(FileType::from_raw_mode(stat.st_mode)))),
        Err(Errno::NOENT) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// `fd` as a file, if it is a regular one.
fn regular(fd: OwnedFd) -> io::Result<File> {
    match FileType::from_raw_mode(rustix::fs::fstat(&fd)?.st_mode) {
        FileType::RegularFile => Ok(File::from(fd)),
        FileType::Directory => Err(is_a_folder()),
        _ => Err(io::Error::other(
            "it is a pipe, socket or device, not a regular file",
        )),
    }
}

fn kind_of(file_type: FileType) -> EntryType {
    match file_type {
        FileType::RegularFile => EntryType::File,
        FileType::Directory => EntryType::Dir,
        FileType::Symlink => EntryType::Symlink,
        _ => EntryType::Other,
    }
}

// The fields' integer types differ between systems, so some casts are no-ops on some of them.
#[allow(clippy::unnecessary_cast)]
fn meta(stat: &Stat) -> EntryMeta {
    EntryMeta {
        kind: kind_of(FileType::from_raw_mode(stat.st_mode)),
        size: stat.st_size as u64,
        modified_ns: stat.st_mtime as i128 * 1_000_000_000 + stat.st_mtime_nsec as i128,
        changed_ns: stat.st_ctime as i128 * 1_000_000_000 + stat.st_ctime_nsec as i128,
        executable: Some(stat.st_mode & 0o111 != 0),
        file_id: (stat.st_dev as u64, stat.st_ino as u128),
    }
}
