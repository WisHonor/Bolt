//! `Dir` on Windows. Windows' own file calls take whole paths, so a name is opened with
//! `NtCreateFile` relative to the held folder, as Rust's standard library does in
//! `remove_dir_all`. Each `unsafe` block is one call into Windows.

use std::ffi::{OsStr, OsString};
use std::fs::{File, OpenOptions};
use std::io;
use std::mem::offset_of;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::{AsHandle, AsRawHandle, BorrowedHandle, FromRawHandle, OwnedHandle};
use std::path::Path;
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicBool, Ordering};

use windows_sys::Wdk::Foundation::OBJECT_ATTRIBUTES;
use windows_sys::Wdk::Storage::FileSystem::{
    FILE_CREATE, FILE_DIRECTORY_FILE, FILE_INFORMATION_CLASS, FILE_OPEN, FILE_OPEN_IF,
    FILE_OPEN_REPARSE_POINT, FILE_RENAME_IGNORE_READONLY_ATTRIBUTE, FILE_RENAME_INFORMATION,
    FILE_RENAME_POSIX_SEMANTICS, FILE_RENAME_REPLACE_IF_EXISTS, FILE_SYNCHRONOUS_IO_NONALERT,
    FileRenameInformation, FileRenameInformationEx, NtCreateFile, NtSetInformationFile,
    REPARSE_DATA_BUFFER, REPARSE_DATA_BUFFER_0_0, REPARSE_DATA_BUFFER_0_1, SYMLINK_FLAG_RELATIVE,
};
use windows_sys::Win32::Foundation::{
    ERROR_DELETE_PENDING, ERROR_FILE_NOT_FOUND, ERROR_INVALID_FUNCTION, ERROR_INVALID_PARAMETER,
    ERROR_NO_MORE_FILES, ERROR_NOT_SUPPORTED, HANDLE, NTSTATUS, OBJ_CASE_INSENSITIVE,
    OBJ_DONT_REPARSE, RtlNtStatusToDosError, STATUS_DELETE_PENDING, STATUS_INVALID_PARAMETER,
    STATUS_REPARSE_POINT_ENCOUNTERED, UNICODE_STRING,
};
use windows_sys::Win32::Storage::FileSystem::{
    BY_HANDLE_FILE_INFORMATION, CreateSymbolicLinkW, DELETE, FILE_ATTRIBUTE_DIRECTORY,
    FILE_ATTRIBUTE_NORMAL, FILE_ATTRIBUTE_REPARSE_POINT, FILE_ATTRIBUTE_TAG_INFO, FILE_BASIC_INFO,
    FILE_DISPOSITION_FLAG_DELETE, FILE_DISPOSITION_FLAG_IGNORE_READONLY_ATTRIBUTE,
    FILE_DISPOSITION_FLAG_POSIX_SEMANTICS, FILE_DISPOSITION_INFO, FILE_DISPOSITION_INFO_EX,
    FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_GENERIC_READ,
    FILE_GENERIC_WRITE, FILE_ID_BOTH_DIR_INFO, FILE_ID_INFO, FILE_INFO_BY_HANDLE_CLASS,
    FILE_LIST_DIRECTORY, FILE_NAME_NORMALIZED, FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE,
    FILE_SHARE_MODE, FILE_SHARE_READ, FILE_SHARE_WRITE, FileAttributeTagInfo, FileBasicInfo,
    FileDispositionInfo, FileDispositionInfoEx, FileIdBothDirectoryInfo,
    FileIdBothDirectoryRestartInfo, FileIdInfo, GetFileInformationByHandle,
    GetFileInformationByHandleEx, GetFinalPathNameByHandleW, MAXIMUM_REPARSE_DATA_BUFFER_SIZE,
    SYMBOLIC_LINK_FLAG_ALLOW_UNPRIVILEGED_CREATE, SYMBOLIC_LINK_FLAG_DIRECTORY, SYNCHRONIZE,
    SetFileInformationByHandle, VOLUME_NAME_DOS,
};
use windows_sys::Win32::System::IO::{DeviceIoControl, IO_STATUS_BLOCK};
use windows_sys::Win32::System::Ioctl::FSCTL_GET_REPARSE_POINT;
use windows_sys::Win32::System::SystemServices::{
    IO_REPARSE_TAG_MOUNT_POINT, IO_REPARSE_TAG_SYMLINK,
};

use super::{DirEntry, EntryMeta, EntryType, is_a_folder, link_refused};

pub(super) type Handle = OwnedHandle;

/// What a held folder's handle may do: list the folder and read its attributes.
const FOLDER: u32 = FILE_LIST_DIRECTORY | FILE_READ_ATTRIBUTES;

/// Others may read, write, rename and delete what Bolt has open, as with std's files: a held
/// folder can still be renamed, and a file being read can be replaced.
const SHARE_ALL: FILE_SHARE_MODE = FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE;

/// How many bytes of a folder's listing one call reads.
const LISTING_BYTES: usize = 64 * 1024;

/// The longest name Windows' drives hold, in UTF-16 units.
const MAX_NAME: usize = 255;

const BACKSLASH: u16 = b'\\' as u16;
const COLON: u16 = b':' as u16;
const DOT: u16 = b'.' as u16;
const QUESTION: u16 = b'?' as u16;

pub(super) fn open_top(path: &Path) -> io::Result<OwnedHandle> {
    // The one open by path. BACKUP_SEMANTICS lets a folder open; OPEN_REPARSE_POINT opens a link
    // at the last name as itself, so it can be refused. std makes long paths work.
    let top = OpenOptions::new()
        .access_mode(FOLDER)
        .share_mode(SHARE_ALL)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)?;
    let top = OwnedHandle::from(top);
    let what = info(top.as_handle(), FileAttributeTagInfo)?;
    as_folder(top, &what)
}

pub(super) fn open_dir(dir: &OwnedHandle, name: &OsStr) -> io::Result<OwnedHandle> {
    let (handle, what) = open_entry(dir, name, FOLDER)?;
    as_folder(handle, &what)
}

pub(super) fn entries(dir: &OwnedHandle) -> io::Result<Vec<DirEntry>> {
    // A handle of its own (the empty name is the folder itself), so two listings never share a
    // place in the folder.
    let listing = nt_open(
        dir.as_handle(),
        OsStr::new(""),
        FOLDER,
        FILE_OPEN,
        FILE_DIRECTORY_FILE,
    )?;
    let mut buffer = Vec::new();
    let area = aligned(&mut buffer, LISTING_BYTES)?;
    let mut class = FileIdBothDirectoryRestartInfo;
    let mut entries = Vec::new();
    loop {
        // SAFETY: `area` is writable, 8-byte aligned as this class needs, and Windows is told its
        // length, so it writes nothing past it; it keeps no pointer to it. `listing` is a folder
        // handle open for listing and for synchronous calls.
        let ok = unsafe {
            GetFileInformationByHandleEx(
                listing.as_raw_handle(),
                class,
                area.as_mut_ptr().cast(),
                len32(area.len()),
            )
        };
        if ok == 0 {
            let e = io::Error::last_os_error();
            let first = class == FileIdBothDirectoryRestartInfo;
            return if listing_done(first, &e) {
                Ok(entries)
            } else {
                Err(e)
            };
        }
        read_listing(area, &mut entries)?;
        class = FileIdBothDirectoryInfo;
    }
}

pub(super) fn metadata(dir: &OwnedHandle, name: &OsStr) -> io::Result<EntryMeta> {
    let handle = nt_open(dir.as_handle(), name, FILE_READ_ATTRIBUTES, FILE_OPEN, 0)?;
    meta(handle.as_handle())
}

pub(super) fn file_meta(file: &File) -> io::Result<EntryMeta> {
    meta(file.as_handle())
}

pub(super) fn open_file(dir: &OwnedHandle, name: &OsStr) -> io::Result<File> {
    let (handle, what) = open_entry(dir, name, FILE_GENERIC_READ)?;
    match kind_of(what.FileAttributes, what.ReparseTag) {
        EntryType::File => Ok(File::from(handle)),
        EntryType::Dir => Err(is_a_folder()),
        EntryType::Symlink | EntryType::Other => Err(link_refused()),
    }
}

pub(super) fn create_new(dir: &OwnedHandle, name: &OsStr, _executable: bool) -> io::Result<File> {
    let access = FILE_GENERIC_READ | FILE_GENERIC_WRITE;
    // FILE_CREATE makes a file and never opens what is there. No FILE_NON_DIRECTORY_FILE: with
    // it, a folder or junction at the name gives "access denied" instead of "already exists".
    let handle = nt_open(dir.as_handle(), name, access, FILE_CREATE, 0)?;
    Ok(File::from(handle))
}

pub(super) fn open_or_create(dir: &OwnedHandle, name: &OsStr) -> io::Result<File> {
    let access = FILE_GENERIC_READ | FILE_GENERIC_WRITE;
    // FILE_OPEN_IF opens what is there or makes a file. A link there opens as itself and is
    // refused below, so nothing is made where it points.
    let handle = nt_open(dir.as_handle(), name, access, FILE_OPEN_IF, 0)?;
    let what: FILE_ATTRIBUTE_TAG_INFO = info(handle.as_handle(), FileAttributeTagInfo)?;
    match kind_of(what.FileAttributes, what.ReparseTag) {
        EntryType::File => Ok(File::from(handle)),
        EntryType::Dir => Err(is_a_folder()),
        EntryType::Symlink | EntryType::Other => Err(link_refused()),
    }
}

pub(super) fn rename_into(
    dir: &OwnedHandle,
    from: &OsStr,
    to: &OwnedHandle,
    to_name: &OsStr,
) -> io::Result<()> {
    let (source, what) = open_entry(dir, from, DELETE)?;
    if what.FileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(link_refused());
    }
    // A link at the new name is refused, though the rename would replace it rather than follow
    // it, and so is a folder, as on Unix.
    match open_entry(to, to_name, 0) {
        Ok((_, there)) if there.FileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 => {
            return Err(link_refused());
        }
        Ok((_, there)) if there.FileAttributes & FILE_ATTRIBUTE_DIRECTORY != 0 => {
            return Err(is_a_folder());
        }
        Ok(_) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    // A read-only file at the new name is replaced too, as on Unix. Windows before 10 (1809)
    // refuses that flag; drives without POSIX renames (FAT, say) refuse both, and get the plain
    // rename, still replacing.
    let (source, to) = (source.as_handle(), to.as_raw_handle());
    let replace = FILE_RENAME_REPLACE_IF_EXISTS | FILE_RENAME_POSIX_SEMANTICS;
    for flags in [replace | FILE_RENAME_IGNORE_READONLY_ATTRIBUTE, replace] {
        let rename = RenameInfo::new(to, to_name, flags)?;
        match set_rename(source, &rename, FileRenameInformationEx) {
            Err(e) if unsupported(&e) => {}
            result => return result,
        }
    }
    let plain = RenameInfo::new(to, to_name, FILE_RENAME_REPLACE_IF_EXISTS)?;
    set_rename(source, &plain, FileRenameInformation)
}

pub(super) fn create_dir(dir: &OwnedHandle, name: &OsStr) -> io::Result<()> {
    let access = FILE_READ_ATTRIBUTES;
    nt_open(
        dir.as_handle(),
        name,
        access,
        FILE_CREATE,
        FILE_DIRECTORY_FILE,
    )
    .map(drop)
}

pub(super) fn remove_file(dir: &OwnedHandle, name: &OsStr) -> io::Result<()> {
    let (handle, what) = open_entry(dir, name, DELETE)?;
    // A link goes as itself, file and folder links alike; a folder that isn't a link stays.
    let link = kind_of(what.FileAttributes, what.ReparseTag) == EntryType::Symlink;
    if what.FileAttributes & FILE_ATTRIBUTE_DIRECTORY != 0 && !link {
        return Err(is_a_folder());
    }
    delete(handle.as_handle())
}

pub(super) fn remove_dir(dir: &OwnedHandle, name: &OsStr) -> io::Result<()> {
    let (handle, what) = open_entry(dir, name, DELETE)?;
    let folder = as_folder(handle, &what)?;
    delete(folder.as_handle())
}

pub(super) fn symlink(
    dir: &OwnedHandle,
    name: &OsStr,
    target: &OsStr,
    to_dir: bool,
    scratch: &OwnedHandle,
) -> io::Result<()> {
    // Nothing may have the name: checked now, and again when the link is moved there.
    match nt_open(dir.as_handle(), name, FILE_READ_ATTRIBUTES, FILE_OPEN, 0) {
        Ok(_) => return Err(io::Error::from(io::ErrorKind::AlreadyExists)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    // Windows makes links only by path, and a folder moved meanwhile would put the link wherever
    // its old path now leads. So the link is made in `scratch`, under a name nobody chose, and
    // moved to `name` by handle only once `scratch` shows it is this link. If Bolt stops before
    // the move, or can't open or remove the link again below, it stays in `scratch`, never at
    // `name`.
    let temp = crate::temp_name();
    let mut path = final_path(scratch.as_handle())?;
    if path.last() != Some(&BACKSLASH) {
        path.push(BACKSLASH);
    }
    path.extend(temp.encode_wide());
    path.push(0);
    let wanted: Vec<u16> = target.encode_wide().collect();
    let target = with_nul(&wanted)?;
    let mut flags = SYMBOLIC_LINK_FLAG_ALLOW_UNPRIVILEGED_CREATE;
    if to_dir {
        flags |= SYMBOLIC_LINK_FLAG_DIRECTORY;
    }
    #[cfg(test)]
    if let Some(race) = BEFORE_LINK.with_borrow_mut(Option::take) {
        race();
    }
    // SAFETY: `path` and `target` are NUL-terminated UTF-16 strings that live through the call;
    // CreateSymbolicLinkW only reads them and keeps no pointer to them.
    if !unsafe { CreateSymbolicLinkW(path.as_ptr(), target.as_ptr(), flags) } {
        return Err(io::Error::last_os_error());
    }
    let elsewhere = || {
        io::Error::other(
            "the link isn't in the scratch folder: the folder moved, or something replaced the \
             link, while it was made",
        )
    };
    let (link, what) = match open_entry(scratch, &temp, DELETE) {
        Ok(found) => found,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Err(elsewhere()),
        // Not opened, so it can't be removed by handle either: it stays in `scratch`.
        Err(e) => return Err(e),
    };
    if what.FileAttributes & FILE_ATTRIBUTE_REPARSE_POINT == 0
        || what.ReparseTag != IO_REPARSE_TAG_SYMLINK
    {
        return Err(elsewhere());
    }
    let placed = (|| {
        // SAFETY: `link` was opened by `nt_open`, which opens every handle for synchronous calls.
        if unsafe { target_of(link.as_handle()) }? != wanted {
            return Err(io::Error::other(
                "the link Windows made doesn't hold the target asked for",
            ));
        }
        // Flags 0: the move fails if something took the name meanwhile. Into `dir`, by handle;
        // `scratch` must be on the same drive.
        let rename = RenameInfo::new(dir.as_raw_handle(), name, 0)?;
        set_rename(link.as_handle(), &rename, FileRenameInformation)
    })();
    // A link left under a name nobody chose would only puzzle; removing it can't touch its target.
    if placed.is_err() {
        let _ = delete(link.as_handle());
    }
    placed
}

pub(super) fn read_link(dir: &OwnedHandle, name: &OsStr) -> io::Result<OsString> {
    let link = nt_open(dir.as_handle(), name, FILE_READ_ATTRIBUTES, FILE_OPEN, 0)?;
    // SAFETY: `link` was opened by `nt_open`, which opens every handle for synchronous calls.
    let target = unsafe { target_of(link.as_handle()) }?;
    Ok(OsString::from_wide(&target))
}

/// The target of the link `link` has open, as `link_target` shows it.
///
/// # Safety
///
/// `link` must be open for synchronous calls (FILE_SYNCHRONOUS_IO_NONALERT, which `nt_open` asks
/// for every time). On a handle open for overlapped calls, Windows would finish the call later,
/// writing into a buffer this function has already freed.
unsafe fn target_of(link: BorrowedHandle<'_>) -> io::Result<Vec<u16>> {
    let mut buffer = Vec::new();
    let area = aligned(&mut buffer, MAXIMUM_REPARSE_DATA_BUFFER_SIZE as usize)?;
    let mut filled = 0u32;
    // SAFETY: there is no input (null, 0 bytes). `area` is writable and Windows is told its
    // length, so it writes nothing past it, and `filled` is a valid place for the count it writes;
    // it keeps no pointer to either. `link` is open for the call, and for synchronous calls, as
    // the caller promises, so the call is over when it returns and there is no OVERLAPPED (null).
    let ok = unsafe {
        DeviceIoControl(
            link.as_raw_handle(),
            FSCTL_GET_REPARSE_POINT,
            null(),
            0,
            area.as_mut_ptr().cast(),
            len32(area.len()),
            &mut filled,
            null_mut(),
        )
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    let data = area.get(..filled as usize).ok_or_else(unreadable)?;
    link_target(data)
}

pub(super) fn set_executable(
    _dir: &OwnedHandle,
    _name: &OsStr,
    _executable: bool,
) -> io::Result<()> {
    Ok(())
}

pub(super) fn sync(_dir: &OwnedHandle) -> io::Result<()> {
    // shortcut: Windows flushes only handles open for writing, which a held folder isn't, so
    // renames keep their order but can be lost on power failure, as with fsio::sync_dir; make
    // them durable when renames on Windows are.
    Ok(())
}

#[cfg(test)]
thread_local! {
    /// What a test runs between reading a folder's path and making a link in it, to race them.
    pub(super) static BEFORE_LINK: std::cell::RefCell<Option<Box<dyn FnOnce()>>> =
        const { std::cell::RefCell::new(None) };
}

/// Whether this Windows takes OBJ_DONT_REPARSE; older ones refuse the flag.
static DONT_REPARSE: AtomicBool = AtomicBool::new(true);

/// Opens or makes `name` in the folder `dir`, never by path: a reparse point at the name opens as
/// itself (FILE_OPEN_REPARSE_POINT). OBJ_DONT_REPARSE would refuse one met on the way, as in std,
/// though a single name has nothing before it.
fn nt_open(
    dir: BorrowedHandle<'_>,
    name: &OsStr,
    access: u32,
    disposition: u32,
    options: u32,
) -> io::Result<OwnedHandle> {
    let wide: Vec<u16> = name.encode_wide().collect();
    let length = wide
        .len()
        .checked_mul(2)
        .and_then(|bytes| u16::try_from(bytes).ok())
        .ok_or_else(|| io::Error::from(io::ErrorKind::InvalidFilename))?;
    let name = UNICODE_STRING {
        Length: length,
        MaximumLength: length,
        Buffer: wide.as_ptr().cast_mut(),
    };
    let open = |attributes| create_file(dir, &name, attributes, access, disposition, options);
    if DONT_REPARSE.load(Ordering::Relaxed) {
        match open(OBJ_CASE_INSENSITIVE | OBJ_DONT_REPARSE) {
            // A single name has no reparse point before it to follow, so the flag can go.
            Err(STATUS_INVALID_PARAMETER) => {
                let opened = open(OBJ_CASE_INSENSITIVE);
                if opened.is_ok() {
                    DONT_REPARSE.store(false, Ordering::Relaxed);
                }
                opened.map_err(nt_error)
            }
            opened => opened.map_err(nt_error),
        }
    } else {
        open(OBJ_CASE_INSENSITIVE).map_err(nt_error)
    }
}

/// One NtCreateFile call; an error is Windows' status.
fn create_file(
    dir: BorrowedHandle<'_>,
    name: &UNICODE_STRING,
    attributes: u32,
    access: u32,
    disposition: u32,
    options: u32,
) -> Result<OwnedHandle, NTSTATUS> {
    let object = OBJECT_ATTRIBUTES {
        Length: len32(size_of::<OBJECT_ATTRIBUTES>()),
        RootDirectory: dir.as_raw_handle(),
        ObjectName: name,
        Attributes: attributes,
        SecurityDescriptor: null(),
        SecurityQualityOfService: null(),
    };
    let mut handle: HANDLE = null_mut();
    let mut status_block = IO_STATUS_BLOCK::default();
    // SAFETY: `handle` and `status_block` are valid places for Windows to write a handle and a
    // status. `object` and the name it points to (whose buffer holds `Length` bytes) live through
    // the call, and Windows only reads them. `dir` is an open folder handle for the call. No
    // allocation size and no extended attributes (null, 0).
    let status = unsafe {
        NtCreateFile(
            &mut handle,
            access | SYNCHRONIZE,
            &object,
            &mut status_block,
            null(),
            FILE_ATTRIBUTE_NORMAL,
            SHARE_ALL,
            disposition,
            options | FILE_OPEN_REPARSE_POINT | FILE_SYNCHRONOUS_IO_NONALERT,
            null(),
            0,
        )
    };
    if status < 0 {
        return Err(status);
    }
    // SAFETY: NtCreateFile succeeded, so `handle` is an open handle that nothing else holds;
    // OwnedHandle closes it once, when dropped.
    Ok(unsafe { OwnedHandle::from_raw_handle(handle) })
}

/// The error a failed NT call's status stands for.
fn nt_error(status: NTSTATUS) -> io::Error {
    match status {
        // Windows' own mapping says "access denied", which hides what happened.
        STATUS_DELETE_PENDING => io::Error::from_raw_os_error(ERROR_DELETE_PENDING as i32),
        STATUS_REPARSE_POINT_ENCOUNTERED => link_refused(),
        // SAFETY: RtlNtStatusToDosError only maps one number to another.
        _ => io::Error::from_raw_os_error(unsafe { RtlNtStatusToDosError(status) } as i32),
    }
}

/// Opens `name` in `dir` as itself, whatever it is, and says what it is.
fn open_entry(
    dir: &OwnedHandle,
    name: &OsStr,
    access: u32,
) -> io::Result<(OwnedHandle, FILE_ATTRIBUTE_TAG_INFO)> {
    let handle = nt_open(
        dir.as_handle(),
        name,
        access | FILE_READ_ATTRIBUTES,
        FILE_OPEN,
        0,
    )?;
    let what = info(handle.as_handle(), FileAttributeTagInfo)?;
    Ok((handle, what))
}

/// `handle`, if it is a folder; a link or other reparse point is refused.
fn as_folder(handle: OwnedHandle, what: &FILE_ATTRIBUTE_TAG_INFO) -> io::Result<OwnedHandle> {
    match kind_of(what.FileAttributes, what.ReparseTag) {
        EntryType::Dir => Ok(handle),
        // The kind Unix's ENOTDIR has.
        EntryType::File => Err(io::Error::new(
            io::ErrorKind::NotADirectory,
            "it is not a folder",
        )),
        EntryType::Symlink | EntryType::Other => Err(link_refused()),
    }
}

/// What an entry with these attributes is. A reparse point is a link when its tag says it stands
/// for another name (symlinks and junctions do); any other (a cloud placeholder, say) is `Other`.
fn kind_of(attributes: u32, tag: u32) -> EntryType {
    // IsReparseTagNameSurrogate in Windows' headers.
    const NAME_SURROGATE: u32 = 0x2000_0000;
    if attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        if tag & NAME_SURROGATE != 0 {
            EntryType::Symlink
        } else {
            EntryType::Other
        }
    } else if attributes & FILE_ATTRIBUTE_DIRECTORY != 0 {
        EntryType::Dir
    } else {
        EntryType::File
    }
}

/// From 1601-01-01, where Windows counts its 100-nanosecond ticks from, to 1970-01-01.
const TICKS_TO_1970: i128 = 116_444_736_000_000_000;

fn meta(handle: BorrowedHandle<'_>) -> io::Result<EntryMeta> {
    let mut about = BY_HANDLE_FILE_INFORMATION::default();
    // SAFETY: `about` is a whole BY_HANDLE_FILE_INFORMATION, made only of integers, for Windows
    // to fill; it keeps no pointer to it. `handle` is open for the call.
    if unsafe { GetFileInformationByHandle(handle.as_raw_handle(), &mut about) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let kind = if about.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        let what = info::<FILE_ATTRIBUTE_TAG_INFO>(handle, FileAttributeTagInfo)?;
        kind_of(what.FileAttributes, what.ReparseTag)
    } else {
        kind_of(about.dwFileAttributes, 0)
    };
    let file_id = match info::<FILE_ID_INFO>(handle, FileIdInfo) {
        Ok(id) => (
            id.VolumeSerialNumber,
            u128::from_le_bytes(id.FileId.Identifier),
        ),
        // FAT and some network drives have only 64-bit file numbers.
        Err(e) if unsupported(&e) => (
            u64::from(about.dwVolumeSerialNumber),
            u128::from(u64::from(about.nFileIndexHigh) << 32 | u64::from(about.nFileIndexLow)),
        ),
        Err(e) => return Err(e),
    };
    // Moved by every change to the file, setting its times included, so a copy written over it
    // in place shows.
    let changed = info::<FILE_BASIC_INFO>(handle, FileBasicInfo)?.ChangeTime;
    let written = about.ftLastWriteTime;
    let ticks = u64::from(written.dwHighDateTime) << 32 | u64::from(written.dwLowDateTime);
    Ok(EntryMeta {
        kind,
        size: u64::from(about.nFileSizeHigh) << 32 | u64::from(about.nFileSizeLow),
        modified_ns: (i128::from(ticks) - TICKS_TO_1970) * 100,
        changed_ns: (i128::from(changed) - TICKS_TO_1970) * 100,
        executable: None,
        file_id,
    })
}

/// A structure made only of integers: any bytes Windows writes into it make a valid value.
///
/// # Safety
///
/// Every bit pattern must be a valid value of the type.
unsafe trait Plain: Default {}

// SAFETY: two u32s.
unsafe impl Plain for FILE_ATTRIBUTE_TAG_INFO {}
// SAFETY: a u64 and an array of 16 bytes.
unsafe impl Plain for FILE_ID_INFO {}
// SAFETY: four i64s and a u32 (padded to 40 bytes, as in C).
unsafe impl Plain for FILE_BASIC_INFO {}

/// The `class` information about the open `handle`; `T` must be the structure of that class.
fn info<T: Plain>(handle: BorrowedHandle<'_>, class: FILE_INFO_BY_HANDLE_CLASS) -> io::Result<T> {
    let mut value = T::default();
    // SAFETY: `value` is a whole T and Windows is told its size, so it writes nothing past it,
    // and whatever it writes is a valid T (`Plain`). It keeps no pointer to it. `handle` is open
    // for the call.
    let ok = unsafe {
        GetFileInformationByHandleEx(
            handle.as_raw_handle(),
            class,
            std::ptr::from_mut(&mut value).cast(),
            len32(size_of::<T>()),
        )
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(value)
}

/// Sets the `class` information of the open `handle` to `value`, the structure of that class.
fn set_info<T>(
    handle: BorrowedHandle<'_>,
    class: FILE_INFO_BY_HANDLE_CLASS,
    value: &T,
) -> io::Result<()> {
    // SAFETY: Windows reads at most the size it is told, which is `value`'s, and keeps no pointer
    // to it. `handle` is open for the call.
    let ok = unsafe {
        SetFileInformationByHandle(
            handle.as_raw_handle(),
            class,
            std::ptr::from_ref(value).cast(),
            len32(size_of::<T>()),
        )
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// Deletes the open entry. Its name goes as soon as this handle closes, even while others still
/// have it open (POSIX semantics).
fn delete(handle: BorrowedHandle<'_>) -> io::Result<()> {
    // A read-only file goes too, as on Unix. Windows before 10 (1809) refuses that flag; drives
    // without POSIX deletes (FAT, say) refuse both, and get the plain delete, whose name goes once
    // the last handle closes.
    let posix = FILE_DISPOSITION_FLAG_DELETE | FILE_DISPOSITION_FLAG_POSIX_SEMANTICS;
    for flags in [
        posix | FILE_DISPOSITION_FLAG_IGNORE_READONLY_ATTRIBUTE,
        posix,
    ] {
        let info = FILE_DISPOSITION_INFO_EX { Flags: flags };
        match set_info(handle, FileDispositionInfoEx, &info) {
            Err(e) if unsupported(&e) => {}
            result => return result,
        }
    }
    let plain = FILE_DISPOSITION_INFO { DeleteFile: true };
    set_info(handle, FileDispositionInfo, &plain)
}

/// FILE_RENAME_INFORMATION with room for a name of up to MAX_NAME units and a NUL.
#[repr(C)]
struct RenameInfo {
    flags: u32,
    root: HANDLE,
    name_bytes: u32,
    name: [u16; MAX_NAME + 1],
}

// FILE_RENAME_INFORMATION's layout, its last field running on past its end.
const _: () = assert!(
    offset_of!(RenameInfo, flags) == offset_of!(FILE_RENAME_INFORMATION, Anonymous)
        && offset_of!(RenameInfo, root) == offset_of!(FILE_RENAME_INFORMATION, RootDirectory)
        && offset_of!(RenameInfo, name_bytes)
            == offset_of!(FILE_RENAME_INFORMATION, FileNameLength)
        && offset_of!(RenameInfo, name) == offset_of!(FILE_RENAME_INFORMATION, FileName)
        && align_of::<RenameInfo>() == align_of::<FILE_RENAME_INFORMATION>()
);

impl RenameInfo {
    /// Moving to `name` in the folder `root`, as `flags` say.
    fn new(root: HANDLE, name: &OsStr, flags: u32) -> io::Result<RenameInfo> {
        let wide: Vec<u16> = name.encode_wide().collect();
        let mut rename = RenameInfo {
            flags,
            root,
            name_bytes: len32(wide.len() * 2),
            name: [0; MAX_NAME + 1],
        };
        let room = rename
            .name
            .get_mut(..wide.len())
            .filter(|_| wide.len() <= MAX_NAME);
        room.ok_or_else(|| io::Error::from(io::ErrorKind::InvalidFilename))?
            .copy_from_slice(&wide);
        Ok(rename)
    }
}

/// Renames the open `source` as `rename` says. The NT call, because SetFileInformationByHandle
/// takes the new name as a path from the current folder and refuses a root folder with it.
fn set_rename(
    source: BorrowedHandle<'_>,
    rename: &RenameInfo,
    class: FILE_INFORMATION_CLASS,
) -> io::Result<()> {
    let mut status_block = IO_STATUS_BLOCK::default();
    // SAFETY: Windows reads at most the size it is told, which is `rename`'s, and writes only
    // `status_block`, a valid place for a status; it keeps no pointer to either. `source` is open,
    // with DELETE access, for the call, and `rename.root` is a folder handle its caller holds open.
    let status = unsafe {
        NtSetInformationFile(
            source.as_raw_handle(),
            &mut status_block,
            std::ptr::from_ref(rename).cast(),
            len32(size_of::<RenameInfo>()),
            class,
        )
    };
    if status < 0 {
        return Err(nt_error(status));
    }
    Ok(())
}

/// The folder's path as Windows sees it now (`\\?\C:\...`).
fn final_path(dir: BorrowedHandle<'_>) -> io::Result<Vec<u16>> {
    let mut path = vec![0u16; 512];
    // The second try has the room the first asked for; a third only if the path grew meanwhile.
    for _ in 0..3 {
        // SAFETY: Windows is told `path`'s length, so it writes nothing past it, and keeps no
        // pointer to it. `dir` is open for the call.
        let len = unsafe {
            GetFinalPathNameByHandleW(
                dir.as_raw_handle(),
                path.as_mut_ptr(),
                len32(path.len()),
                FILE_NAME_NORMALIZED | VOLUME_NAME_DOS,
            )
        } as usize;
        if len == 0 {
            return Err(io::Error::last_os_error());
        }
        // Too small, and `len` is the room it takes, NUL included; else the path's length.
        if len < path.len() {
            path.truncate(len);
            return Ok(path);
        }
        path.resize(len, 0);
    }
    Err(io::Error::other(
        "the folder's path kept changing while Bolt read it",
    ))
}

/// `units` with a NUL after them, for Windows; a NUL among them is refused.
fn with_nul(units: &[u16]) -> io::Result<Vec<u16>> {
    if units.contains(&0) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "a link's target can't hold a NUL",
        ));
    }
    Ok(units.iter().copied().chain([0]).collect())
}

/// The target in the reparse data of a symlink or junction. An absolute target is stored as an NT
/// path; it comes back as std shows it: `\??\C:\x` as `C:\x`, `\??\UNC\server\share` as
/// `\\server\share`, and any other as `\\?\...`.
fn link_target(data: &[u8]) -> io::Result<Vec<u16>> {
    type Symlink = REPARSE_DATA_BUFFER_0_0;
    type Junction = REPARSE_DATA_BUFFER_0_1;
    let union = offset_of!(REPARSE_DATA_BUFFER, Anonymous);
    let (paths, relative) = match u32_at(data, offset_of!(REPARSE_DATA_BUFFER, ReparseTag))? {
        IO_REPARSE_TAG_SYMLINK => {
            let flags = u32_at(data, union + offset_of!(Symlink, Flags))?;
            (
                union + offset_of!(Symlink, PathBuffer),
                flags & SYMLINK_FLAG_RELATIVE != 0,
            )
        }
        IO_REPARSE_TAG_MOUNT_POINT => (union + offset_of!(Junction, PathBuffer), false),
        _ => return Err(io::Error::other("it is a reparse point, but not a link")),
    };
    // Where the substitute name, the path Windows follows, is: in the same place in both kinds.
    let offset = u16_at(data, union + offset_of!(Symlink, SubstituteNameOffset))?;
    let length = u16_at(data, union + offset_of!(Symlink, SubstituteNameLength))?;
    let target = wide_at(data, paths + usize::from(offset), usize::from(length))?;
    let nt_prefix = [BACKSLASH, QUESTION, QUESTION, BACKSLASH];
    let rest = match target.strip_prefix(&nt_prefix) {
        Some(rest) if !relative => rest,
        _ => return Ok(target),
    };
    const U: u16 = b'U' as u16;
    const N: u16 = b'N' as u16;
    const C: u16 = b'C' as u16;
    let (start, rest): (&[u16], _) = match rest {
        [_, COLON, BACKSLASH, ..] => (&[], rest),
        [U, N, C, BACKSLASH, share @ ..] => (&[BACKSLASH, BACKSLASH], share),
        _ => (&[BACKSLASH, BACKSLASH, QUESTION, BACKSLASH], rest),
    };
    Ok(start.iter().chain(rest).copied().collect())
}

/// Whether a listing call that failed with `e` means the listing is whole: no more entries, or, on
/// the first call only, none at all (a FAT drive's top folder has not even `.`). Later, "not
/// found" would cut the listing short, and the entries left out would look deleted.
fn listing_done(first: bool, e: &io::Error) -> bool {
    let code = e.raw_os_error();
    code == Some(ERROR_NO_MORE_FILES as i32) || (first && code == Some(ERROR_FILE_NOT_FOUND as i32))
}

/// Adds the entries in `area`, one batch of FILE_ID_BOTH_DIR_INFO records. Every offset and length
/// is checked against `area`: one that doesn't fit ends the listing with an error.
fn read_listing(area: &[u8], entries: &mut Vec<DirEntry>) -> io::Result<()> {
    type Info = FILE_ID_BOTH_DIR_INFO;
    /// The room for a short name: 12 UTF-16 units.
    const SHORT_NAME_BYTES: usize = 24;
    let mut at = 0usize;
    loop {
        let record = area.get(at..).ok_or_else(unreadable)?;
        let next = u32_at(record, offset_of!(Info, NextEntryOffset))?;
        let attributes = u32_at(record, offset_of!(Info, FileAttributes))?;
        // For a reparse point this holds its tag, as FindFirstFileW's dwReserved0 does.
        let tag = u32_at(record, offset_of!(Info, EaSize))?;
        let name_bytes = u32_at(record, offset_of!(Info, FileNameLength))?;
        let name = wide_at(record, offset_of!(Info, FileName), name_bytes as usize)?;
        if name != [DOT] && name != [DOT, DOT] {
            let [short_bytes] = bytes_at::<1>(record, offset_of!(Info, ShortNameLength))?;
            let short_bytes = usize::from(short_bytes);
            if short_bytes > SHORT_NAME_BYTES {
                return Err(unreadable());
            }
            let short = wide_at(record, offset_of!(Info, ShortName), short_bytes)?;
            entries.push(DirEntry {
                name: OsString::from_wide(&name),
                kind: kind_of(attributes, tag),
                folder: attributes & FILE_ATTRIBUTE_DIRECTORY != 0,
                short_name: (!short.is_empty()).then(|| OsString::from_wide(&short)),
            });
        }
        if next == 0 {
            return Ok(());
        }
        at = at.checked_add(next as usize).ok_or_else(unreadable)?;
    }
}

/// The `N` bytes at `at` in `bytes`, if all are there.
fn bytes_at<const N: usize>(bytes: &[u8], at: usize) -> io::Result<[u8; N]> {
    at.checked_add(N)
        .and_then(|end| bytes.get(at..end))
        .and_then(|found| found.try_into().ok())
        .ok_or_else(unreadable)
}

fn u16_at(bytes: &[u8], at: usize) -> io::Result<u16> {
    bytes_at(bytes, at).map(u16::from_ne_bytes)
}

fn u32_at(bytes: &[u8], at: usize) -> io::Result<u32> {
    bytes_at(bytes, at).map(u32::from_ne_bytes)
}

/// The `len` bytes at `at` in `bytes`, as UTF-16 units.
fn wide_at(bytes: &[u8], at: usize, len: usize) -> io::Result<Vec<u16>> {
    let found = at
        .checked_add(len)
        .and_then(|end| bytes.get(at..end))
        .ok_or_else(unreadable)?;
    let (units, odd) = found.as_chunks::<2>();
    if !odd.is_empty() {
        return Err(unreadable());
    }
    Ok(units.iter().map(|unit| u16::from_ne_bytes(*unit)).collect())
}

fn unreadable() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "Windows described an entry in a form Bolt can't read",
    )
}

/// `len` zeroed bytes in `buffer`, starting 8-byte aligned as Windows' structures need.
fn aligned(buffer: &mut Vec<u8>, len: usize) -> io::Result<&mut [u8]> {
    *buffer = vec![0; len + 7];
    let start = buffer.as_ptr().align_offset(8);
    start
        .checked_add(len)
        .and_then(|end| buffer.get_mut(start..end))
        .ok_or_else(|| io::Error::other("no aligned room for Windows to write in"))
}

/// A length as Windows takes it: never more than the real one.
fn len32(len: usize) -> u32 {
    u32::try_from(len).unwrap_or(u32::MAX)
}

/// Whether `e` says the drive, or this Windows, can't do what was asked.
fn unsupported(e: &io::Error) -> bool {
    [
        ERROR_INVALID_PARAMETER,
        ERROR_NOT_SUPPORTED,
        ERROR_INVALID_FUNCTION,
    ]
    .iter()
    .any(|&code| e.raw_os_error() == Some(code as i32))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_ARCHIVE;
    use windows_sys::Win32::System::SystemServices::{
        IO_REPARSE_TAG_APPEXECLINK, IO_REPARSE_TAG_CLOUD_6, IO_REPARSE_TAG_DEDUP,
    };

    fn put(bytes: &mut [u8], at: usize, value: &[u8]) {
        bytes[at..at + value.len()].copy_from_slice(value);
    }

    fn wide(text: &str) -> Vec<u8> {
        text.encode_utf16().flat_map(u16::to_ne_bytes).collect()
    }

    /// One FILE_ID_BOTH_DIR_INFO record, padded to a multiple of 8 bytes as Windows lays them out.
    fn record(attributes: u32, tag: u32, name: &str, short: &str) -> Vec<u8> {
        type Info = FILE_ID_BOTH_DIR_INFO;
        let (name, short) = (wide(name), wide(short));
        let mut bytes = vec![0; (offset_of!(Info, FileName) + name.len()).next_multiple_of(8)];
        put(
            &mut bytes,
            offset_of!(Info, FileAttributes),
            &attributes.to_ne_bytes(),
        );
        put(
            &mut bytes,
            offset_of!(Info, FileNameLength),
            &len32(name.len()).to_ne_bytes(),
        );
        put(&mut bytes, offset_of!(Info, EaSize), &tag.to_ne_bytes());
        put(
            &mut bytes,
            offset_of!(Info, ShortNameLength),
            &[short.len() as u8],
        );
        put(&mut bytes, offset_of!(Info, ShortName), &short);
        put(&mut bytes, offset_of!(Info, FileName), &name);
        bytes
    }

    /// Records chained one after another, as one call returns them.
    fn listing(records: &[Vec<u8>]) -> Vec<u8> {
        let mut bytes = Vec::new();
        for (i, record) in records.iter().enumerate() {
            let mut record = record.clone();
            if i + 1 < records.len() {
                let next = len32(record.len());
                put(&mut record, 0, &next.to_ne_bytes());
            }
            bytes.extend(record);
        }
        bytes
    }

    #[test]
    fn a_listing_says_what_each_entry_is() {
        let reparse = FILE_ATTRIBUTE_REPARSE_POINT;
        let folder = FILE_ATTRIBUTE_DIRECTORY;
        let bytes = listing(&[
            record(folder, 0, ".", ""),
            record(folder, 0, "..", ""),
            record(folder, 0, ".git", "GIT~1"),
            record(FILE_ATTRIBUTE_ARCHIVE, 0, "file.txt", ""),
            record(reparse, IO_REPARSE_TAG_SYMLINK, "link", ""),
            record(reparse | folder, IO_REPARSE_TAG_MOUNT_POINT, "junction", ""),
            // Reparse points that aren't links: a OneDrive placeholder, a deduplicated file and
            // an app's command alias.
            record(reparse, IO_REPARSE_TAG_CLOUD_6, "cloud", ""),
            record(reparse | folder, IO_REPARSE_TAG_CLOUD_6, "cloud folder", ""),
            record(reparse, IO_REPARSE_TAG_DEDUP, "dedup", ""),
            record(reparse, IO_REPARSE_TAG_APPEXECLINK, "alias", ""),
        ]);
        let mut entries = Vec::new();
        read_listing(&bytes, &mut entries).unwrap();
        // The folder bit as Windows marks it, a link's and a cloud folder's included.
        let folders: Vec<_> = entries
            .iter()
            .filter(|e| e.folder)
            .map(|e| &e.name)
            .collect();
        assert_eq!(folders, [".git", "junction", "cloud folder"]);
        let found: Vec<_> = entries
            .iter()
            .map(|e| (e.name.to_str().unwrap(), e.kind, e.short_name.as_deref()))
            .collect();
        assert_eq!(
            found,
            [
                (".git", EntryType::Dir, Some(OsStr::new("GIT~1"))),
                ("file.txt", EntryType::File, None),
                ("link", EntryType::Symlink, None),
                ("junction", EntryType::Symlink, None),
                ("cloud", EntryType::Other, None),
                ("cloud folder", EntryType::Other, None),
                ("dedup", EntryType::Other, None),
                ("alias", EntryType::Other, None),
            ]
        );
    }

    #[test]
    fn a_listing_that_does_not_fit_is_refused() {
        type Info = FILE_ID_BOTH_DIR_INFO;
        let good = listing(&[record(0, 0, "a", ""), record(0, 0, "b", "")]);
        let read = |bytes: &[u8]| read_listing(bytes, &mut Vec::new());
        read(&good).unwrap();
        for cut in 0..good.len() - 8 {
            assert!(read(&good[..cut]).is_err(), "cut at {cut}");
        }
        let mut next_too_far = good.clone();
        put(&mut next_too_far, 0, &u32::MAX.to_ne_bytes());
        assert!(read(&next_too_far).is_err());
        let mut name_too_long = good.clone();
        put(
            &mut name_too_long,
            offset_of!(Info, FileNameLength),
            &4096u32.to_ne_bytes(),
        );
        assert!(read(&name_too_long).is_err());
        let mut short_too_long = good.clone();
        put(
            &mut short_too_long,
            offset_of!(Info, ShortNameLength),
            &[26],
        );
        assert!(read(&short_too_long).is_err());
        let mut odd_name = good;
        put(
            &mut odd_name,
            offset_of!(Info, FileNameLength),
            &1u32.to_ne_bytes(),
        );
        assert!(read(&odd_name).is_err());
    }

    /// Reparse data as Windows lays it out, the substitute name first, then the print name.
    fn reparse(tag: u32, relative: bool, target: &str) -> Vec<u8> {
        let name = wide(target);
        let size = len32(name.len()) as u16;
        let mut bytes = Vec::new();
        bytes.extend(tag.to_ne_bytes());
        bytes.extend([0; 4]); // the data's length and a reserved field, which aren't read
        for field in [0, size, size, 0] {
            bytes.extend(u16::to_ne_bytes(field));
        }
        if tag == IO_REPARSE_TAG_SYMLINK {
            bytes.extend(u32::from(relative).to_ne_bytes());
        }
        bytes.extend(name);
        bytes
    }

    fn target(bytes: &[u8]) -> String {
        String::from_utf16(&link_target(bytes).unwrap()).unwrap()
    }

    #[test]
    fn link_targets_read_as_std_shows_them() {
        let (symlink, junction) = (IO_REPARSE_TAG_SYMLINK, IO_REPARSE_TAG_MOUNT_POINT);
        assert_eq!(target(&reparse(symlink, true, r"..\a\b")), r"..\a\b");
        assert_eq!(target(&reparse(symlink, false, r"\??\C:\a")), r"C:\a");
        assert_eq!(
            target(&reparse(symlink, false, r"\??\UNC\server\share")),
            r"\\server\share"
        );
        assert_eq!(target(&reparse(junction, false, r"\??\C:\a")), r"C:\a");
        assert_eq!(
            target(&reparse(junction, false, r"\??\Volume{1}\")),
            r"\\?\Volume{1}\"
        );
        // A relative target is kept as it is, whatever it starts with.
        assert_eq!(target(&reparse(symlink, true, r"\??\C:\a")), r"\??\C:\a");
    }

    #[test]
    fn reparse_data_that_does_not_fit_is_refused() {
        let good = reparse(IO_REPARSE_TAG_SYMLINK, true, "target");
        assert_eq!(target(&good), "target");
        for cut in 0..good.len() {
            assert!(link_target(&good[..cut]).is_err(), "cut at {cut}");
        }
        let mut past_the_end = good.clone();
        put(&mut past_the_end, 8, &2u16.to_ne_bytes());
        assert!(link_target(&past_the_end).is_err());
        assert!(link_target(&reparse(IO_REPARSE_TAG_CLOUD_6, false, "x")).is_err());
    }

    #[test]
    fn the_folder_path_is_the_one_std_finds() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = open_top(tmp.path()).unwrap();
        let found = final_path(dir.as_handle()).unwrap();
        let std = std::fs::canonicalize(tmp.path()).unwrap();
        assert_eq!(OsString::from_wide(&found), std.into_os_string());
    }

    #[test]
    fn a_name_longer_than_windows_holds_is_refused() {
        let root = null_mut();
        assert!(RenameInfo::new(root, OsStr::new(&"x".repeat(MAX_NAME)), 0).is_ok());
        let e = RenameInfo::new(root, OsStr::new(&"x".repeat(MAX_NAME + 1)), 0)
            .err()
            .unwrap();
        assert_eq!(e.kind(), io::ErrorKind::InvalidFilename);
    }

    /// "File not found" may end only the first call: later, it would cut a listing short.
    #[test]
    fn only_the_first_listing_call_may_find_nothing() {
        let error = |code: u32| io::Error::from_raw_os_error(code as i32);
        assert!(listing_done(true, &error(ERROR_NO_MORE_FILES)));
        assert!(listing_done(false, &error(ERROR_NO_MORE_FILES)));
        assert!(listing_done(true, &error(ERROR_FILE_NOT_FOUND)));
        assert!(!listing_done(false, &error(ERROR_FILE_NOT_FOUND)));
        assert!(!listing_done(true, &error(ERROR_INVALID_PARAMETER)));
    }
}
