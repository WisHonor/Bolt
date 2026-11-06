# Milestone 3: the `unsafe` code for folder handles, line by line

The owner reviews every `unsafe` line before milestone 3 is merged (decisions 022 and 048). The code
is in `crates/bolt-vcs-platform/src/`:

- `dir.rs`: the shared `Dir` API and its tests. No `unsafe`.
- `dir/unix.rs`: macOS and Linux, on the `rustix` crate. No `unsafe`.
- `dir/windows.rs`: Windows. Every `unsafe` block below is in this file: 13 blocks, plus one
  `unsafe fn` (`target_of`) and one `unsafe trait` with three `unsafe impl`s (the third, for the change time, is block 4b).

## In plain words

**The problem.** Rust's standard library works with paths. Each call (open, rename, delete) walks the
path again from the top, name by name, and follows any link on the way. Bolt makes many such calls
in a row while it walks and writes a project. If something swaps a folder for a link between two of
them, the next call lands wherever the link points: outside the project, or inside `.git` or
`.bolt` (security.md T3, T19, T20).

**The fix.** A path is like the full street address you give a taxi driver for every trip: swap a
street sign during the ride and you arrive somewhere else. A folder handle is like already standing
inside the building and asking for "room 12, here". Swapping signs outside changes nothing. `Dir`
holds a folder open and does everything in it one name at a time. When that name is a link, it
refuses. Only `metadata`, `read_link` and `remove_file` deal with a link, and then only with the
link itself, never with what it points to.

**Why Windows needs `unsafe`.** On macOS and Linux, the `rustix` crate wraps the system calls for
this (`openat` and its relatives) in safe Rust. Windows' public file functions (`CreateFileW`,
`MoveFileExW`, `DeleteFileW`) take only whole paths. One level down, in `ntdll`, `NtCreateFile`
takes "this name, inside that open folder". Rust's standard library has used it for
`remove_dir_all` since CVE-2022-21658. Rust can't check the promises a call into Windows depends on
(that a buffer is as big as we say, that a handle is open, that what Windows writes back is a valid
Rust value). So each such call sits in an `unsafe` block, with a `// SAFETY:` comment saying why
the promises hold.

**What to check in each block.**

1. Every pointer passed points to memory that lives until the call returns, and that memory is at
   least as long as the length passed with it.
2. Windows keeps no pointer after the call returns. None of these calls do.
3. Whatever Windows writes into Rust memory is a valid value of its type. Any bytes make a valid
   integer. A Rust `bool`, though, must be exactly 0 or 1.
4. Every handle passed is open for the whole call.

**Rules that keep each block small.**

- One call per block. The work before and after is safe Rust.
- Bytes Windows hands back (a folder listing, a link's data) are read by safe code that checks every
  offset and length against the buffer. No pointer is cast to a structure. A bad offset gives an
  error, never a read outside the buffer, and tests feed the readers bad data on purpose.
- Every structure Windows fills is made of integers only, so any bytes make a valid value (the
  `Plain` trait, block 4).
- Buffers are zeroed before Windows writes into them, and the length passed with a buffer is never
  more than its real length (`len32`).
- Every `unsafe` block carries a `// SAFETY:` comment, and the compiler checks it: the crate sets
  Clippy's `undocumented_unsafe_blocks` lint to `deny`, so a block without one doesn't build.

## Where the plan changed, for the owner to rule on

1. **Renaming uses `NtSetInformationFile`, not `SetFileInformationByHandle`.** The plan named
   `SetFileInformationByHandle(FileRenameInfoEx)` with a root folder. On this machine (Windows 11,
   build 26200) that call fails with "The parameter is incorrect" (error 87) whenever it is given a
   root folder and a plain name. Microsoft's page for its structure explains why: the Win32
   [`FILE_RENAME_INFO`][file_rename_info] says `FileName` is "an absolute path" or "a path relative
   to the process's current directory", so the Win32 layer turns the name into a full path first.
   The NT structure underneath, [`FILE_RENAME_INFORMATION`][file_rename_information] (the same
   layout), says that with a root folder `FileName` "specifies only the file name or a relative
   pathname". The standard library's own `Dir::rename` uses the NT call too. The flags are the same
   values under their NT names: `FILE_RENAME_REPLACE_IF_EXISTS` (1) and
   `FILE_RENAME_POSIX_SEMANTICS` (2), where the plan wrote `FILE_RENAME_FLAG_…`.
2. **An entry's details come from `GetFileInformationByHandle` and `FileIdInfo`,** where the plan
   named `FileBasicInfo`, `FileStandardInfo` and `FileIdInfo`. windows-sys declares two fields of
   `FILE_STANDARD_INFO` as Rust `bool`s. If Windows, or a network drive answering through it, ever
   wrote anything but 0 or 1 there, that would be undefined behaviour. `GetFileInformationByHandle`
   fills a structure of integers only, with the attributes, last-write time, size and a 64-bit file
   number in one call. It is what the standard library's own `metadata()` uses on Windows. Its
   64-bit number also serves as the file ID on drives without 128-bit IDs (FAT, some network
   shares; Microsoft's `FILE_ID_INFO` page doesn't promise any desktop Windows supports it).
3. **Fallbacks for older Windows and for drives without POSIX semantics** (FAT and exFAT USB
   drives). When Windows answers "not supported", "invalid parameter" or "invalid function", a
   removal or a replacing rename is tried again without the read-only flag of item 6 (Windows 10
   before version 1809 doesn't know it), and then as the plain call (as the standard library does
   for removal).
4. **Names are matched ignoring case** (`OBJ_CASE_INSENSITIVE`), as every other Windows program sees
   them: `CreateFileW` sets this flag itself.
5. **`:` is refused in a name on Windows** (`a:b` names a stream inside the file `a`, not an entry),
   and NUL on every system.
6. **Read-only files are removed and replaced, as on Unix** (ruled during review). Unless told
   otherwise, Windows refuses to remove or replace a file that has the read-only attribute
   (Microsoft: "STATUS_CANNOT_DELETE indicates that either the file is read-only…"), while on macOS
   and Linux a file's own permissions never stop that. Removal passes
   `FILE_DISPOSITION_FLAG_IGNORE_READONLY_ATTRIBUTE`, and the replacing rename
   `FILE_RENAME_IGNORE_READONLY_ATTRIBUTE` (0x40, the NT name of the
   `FILE_RENAME_FLAG_IGNORE_READONLY_ATTRIBUTE` the review named), as Git for Windows and the
   standard library's `remove_dir_all` do. Since `remove_dir` removes through the same `delete`,
   it now removes an empty read-only folder too, as on Unix.
7. **`sync` does nothing on Windows**, as `fsio::sync_dir` already doesn't. Windows flushes only
   handles opened for writing, and a held folder isn't one.
8. **A link is made in a scratch folder under a random name, checked, and only then moved to its
   name** (from the reviews). This changes the interface the plan gave: `symlink(&self, name,
   target, to_dir, scratch: &Dir)`, where callers pass `.bolt/tmp`. Making a link is the one step
   Windows does only by path (block 10), so a folder moved at that moment would send the link
   wherever its old path now leads. With the link made under a meaningless name in `.bolt/tmp`,
   a lost race leaves only a stray link nobody would run, never one at a name the save chose; and
   a crash before the move leaves it in `.bolt/tmp`, never in the user's folder. It stays there
   for now: Bolt's start-up cleanup of `.bolt/tmp` removes only regular files today, and learns to
   remove such links (through `Dir::remove_file`) with Task 3, which opens `.bolt`'s fixed folders
   through `Dir`. macOS and Linux make the link at its name directly and ignore `scratch`. Block
   10 has the steps.
9. **One rule for temporary names** (from the reviews). `tmp-` and 16 random lowercase hexadecimal
   digits, and the accented ending the engine's drive probe adds, are now defined once, in
   `bolt_platform::temp_name`, `bolt_platform::is_temp_name` and `bolt_platform::ACCENTED_SUFFIX`.
   The engine's `fsio::create_temp` and `fsio::is_temp_name` use them, and so does `symlink`.

## The blocks

Each block below gives the code exactly as it is, what each argument means, why it is sound, and
Microsoft's pages for the function and its flags.

| Block | Function in `dir/windows.rs` | What it calls |
|---|---|---|
| 1, 2 | `create_file` | `NtCreateFile`, then `OwnedHandle::from_raw_handle` |
| 3 | `nt_error` | `RtlNtStatusToDosError` |
| 4 | `info`, with the `Plain` trait | `GetFileInformationByHandleEx` |
| 4b | the `Plain` promise for `FILE_BASIC_INFO`, used by `meta` | nothing: it lets block 4 read the change time |
| 5 | `entries` | `GetFileInformationByHandleEx`, to list a folder |
| 6 | `meta` | `GetFileInformationByHandle` |
| 7 | `set_info`, used by `delete` | `SetFileInformationByHandle` |
| 8 | `set_rename`, used by `rename_into` and `symlink` | `NtSetInformationFile` |
| 9 | `final_path` | `GetFinalPathNameByHandleW` |
| 10 | `symlink` | `CreateSymbolicLinkW` |
| 11 | `target_of`, an `unsafe fn` | `DeviceIoControl` |
| 12, 13 | `symlink` and `read_link` | `target_of` |

### Block 1 and 2: opening a name inside a held folder (`create_file`)

**In plain words:** "Windows, open (or make) this one name, inside the folder this handle holds. If
the name is a link, open the link itself. Give me a handle to it."

```rust
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
```

The arguments to `NtCreateFile`, in order:

| Argument | What it is |
|---|---|
| `&mut handle` | Where Windows writes the new handle. |
| `access \| SYNCHRONIZE` | What the handle may do: list a folder (`FILE_LIST_DIRECTORY`), read a file (`FILE_GENERIC_READ`), write one (`FILE_GENERIC_WRITE`), delete or rename (`DELETE`), read attributes (`FILE_READ_ATTRIBUTES`). `SYNCHRONIZE` is what synchronous calls need. |
| `&object` | The name to open. `RootDirectory` is the held folder and `ObjectName` the name in it, so nothing is looked up by path. `Attributes` is `OBJ_CASE_INSENSITIVE` (names match as in the rest of Windows) and `OBJ_DONT_REPARSE` (Windows refuses a link met on the way; a single name has nothing before it, so this is a second guard, as in the standard library). |
| `&mut status_block` | Where Windows writes details of how the open went. Bolt reads only the status the call returns. |
| `null()` | No allocation size: that only matters when making a large file. |
| `FILE_ATTRIBUTE_NORMAL` | Nothing special about a file made here. |
| `SHARE_ALL` | Others may read, write, rename and delete the entry while Bolt has it open, as with any file the standard library opens. A held folder can still be renamed (the tests do), and a file being read can still be replaced. |
| `disposition` | `FILE_OPEN` (it must exist), `FILE_CREATE` (it must not exist: making never replaces anything, so it can't go through a link either) or, for the change-lock file only, `FILE_OPEN_IF` (what is there, or a new file if nothing is: a link there opens as itself, by `FILE_OPEN_REPARSE_POINT` below, and is refused before anything is read or written, so nothing is made where it points). |
| `options` | `FILE_DIRECTORY_FILE` where it must be a folder (making one, or listing one), plus `FILE_OPEN_REPARSE_POINT` (a link, junction or other reparse point at the name opens as itself, not what it points to) and `FILE_SYNCHRONOUS_IO_NONALERT` (every later call on the handle waits for its result). A new file gets no `FILE_NON_DIRECTORY_FILE`: `FILE_CREATE` never opens what is there anyway, and with that flag a folder or junction at the name answers "access denied" instead of "already exists". |
| `null(), 0` | No extended attributes. |

**Why it is sound.**

- `handle` and `status_block` are local variables that live through the call, and each is exactly
  the type Windows writes there.
- `object` is a local variable too. Its `ObjectName` points at `name`, a `UNICODE_STRING` made in
  `nt_open`, whose `Buffer` points into `wide`, a `Vec<u16>` of exactly `Length` bytes that lives
  until `nt_open` returns. Windows only reads all of these. (windows-sys declares `Buffer` as a
  mutable pointer because the same structure is used where Windows writes; `NtCreateFile` doesn't.)
- `dir` is a `BorrowedHandle`, a borrow of the `Dir` that holds the folder, so Rust's borrow checker
  guarantees the handle stays open during the call.
- The second block: `OwnedHandle::from_raw_handle` is `unsafe` because handing it a handle that
  isn't open, or that something else will also close, would close a handle twice. A status of 0 or
  more means success (Microsoft's `NT_SUCCESS`: negative statuses are errors), so `handle` is a new
  handle that only this function has, and `OwnedHandle` closes it exactly once.

**Around it, in safe code.** `nt_open` builds the name and tries `OBJ_DONT_REPARSE`. A Windows too
old to know that flag answers `STATUS_INVALID_PARAMETER`; the open is then tried without it, and if
that works the flag is dropped for the rest of the run. It is never dropped on the strength of an
error alone. `open_entry` then asks what was opened (block 4), and each caller refuses what it must:
`open_dir` anything but a folder, `open_file` anything but a regular file, `rename_into` a link at
either name. `open_or_create`, for the change-lock file, calls `nt_open` with `FILE_OPEN_IF` and
refuses anything but a regular file the same way; it adds no `unsafe` of its own.

Microsoft: [NtCreateFile][ntcreatefile] (all the flags above are on this page);
[OBJECT_ATTRIBUTES][object_attributes] (`OBJ_CASE_INSENSITIVE`, `OBJ_DONT_REPARSE`);
[UNICODE_STRING][unicode_string]; [IO_STATUS_BLOCK][io_status_block];
[NTSTATUS values and NT_SUCCESS][ntstatus]. Rust: [OwnedHandle][ownedhandle],
[FromRawHandle::from_raw_handle][from_raw_handle].

### Block 3: turning an NT status into an error code (`nt_error`)

```rust
// SAFETY: RtlNtStatusToDosError only maps one number to another.
_ => io::Error::from_raw_os_error(unsafe { RtlNtStatusToDosError(status) } as i32),
```

**In plain words:** NT calls answer with an `NTSTATUS`; the rest of Rust and Bolt understand the
Win32 error codes that `io::Error` holds. This converts one into the other, so that "not found",
"already exists" or "sharing violation" come out as the usual `io::ErrorKind`s.

**Why it is sound.** It takes a number and returns a number. No pointers, no handles. It is
`unsafe` only because every call into a foreign library is.

Two statuses are turned into errors by hand instead: `STATUS_DELETE_PENDING` (Windows' own mapping
says "access denied", which hides what happened) and `STATUS_REPARSE_POINT_ENCOUNTERED` (what
`OBJ_DONT_REPARSE` gives), which becomes Bolt's link refusal.

Microsoft: [RtlNtStatusToDosError][rtlntstatustodoserror].

### Block 4: fixed-size information about an open handle (`info`, and the `Plain` trait)

```rust
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
```

**In plain words:** "Windows, fill in this structure about the file this handle has open." It is used
twice: `FileAttributeTagInfo` (the attributes, and the reparse tag when there is one: what an entry
is) and `FileIdInfo` (the drive's number and the file's 128-bit number on it).

**An `unsafe trait`** is a promise the compiler can't check, made once per type by an
`unsafe impl`. Here the promise is "any bytes are a valid value of this type". To check it, look at
the two structures in windows-sys 0.61.2 (`Win32/Storage/FileSystem`): `FILE_ATTRIBUTE_TAG_INFO` is
`FileAttributes: u32, ReparseTag: u32`, and `FILE_ID_INFO` is `VolumeSerialNumber: u64,
FileId: FILE_ID_128`, where `FILE_ID_128` is `Identifier: [u8; 16]`. Integers only, so the promise
holds.

**Why the call is sound.** `value` is a whole `T` on the stack and lives through the call. Windows
is told its exact size, so it writes nothing past it. Whatever it writes is a valid `T`, because
`T: Plain`. If a caller passed the wrong structure for `class`, the values would be wrong but never
unsound, for the same two reasons.

Microsoft: [GetFileInformationByHandleEx][getfileinformationbyhandleex];
[FILE_INFO_BY_HANDLE_CLASS][file_info_by_handle_class];
[FILE_ATTRIBUTE_TAG_INFO][file_attribute_tag_info]; [FILE_ID_INFO][file_id_info]. Rust:
[unsafe traits][unsafe_traits].

### Block 4b: the change time (`unsafe impl Plain for FILE_BASIC_INFO`)

Added after the second red-team session (RT5-5, [decision 069](../decisions/069-the-file-cache-matches-the-change-time.md)).

```rust
// SAFETY: four i64s and a u32 (padded to 40 bytes, as in C).
unsafe impl Plain for FILE_BASIC_INFO {}
```

and its one use, in `meta`, through block 4 unchanged:

```rust
let changed = info::<FILE_BASIC_INFO>(handle, FileBasicInfo)?.ChangeTime;
```

**In plain words:** "the change-time structure is only numbers, so block 4 may ask Windows to fill
it." Windows moves a file's `ChangeTime` whenever its contents or details change, setting its
last-write time included, so a file written over in place with its old time put back still shows.
The file-state cache matches it, as Git's index matches `ctime` on Unix.

**Why `FILE_BASIC_INFO` is plain data.** In windows-sys 0.61.2 (`Win32/Storage/FileSystem`) it is
`#[repr(C)] #[derive(Clone, Copy, Default)]` with the fields `CreationTime: i64`,
`LastAccessTime: i64`, `LastWriteTime: i64`, `ChangeTime: i64` and `FileAttributes: u32`. Every bit
pattern is a valid `i64` or `u32`, and there are no pointers, references, `bool`s, enums or
`char`s, so any bytes Windows writes make a valid value: the promise `Plain` asks for. `Default`
(all zeros), which block 4 starts from, comes with the type.

**Why the size and alignment block 4 relies on hold.** Block 4 passes `size_of::<T>()` and a pointer
to a `T` on the stack. `#[repr(C)]` lays the fields out as C does: four 8-byte integers at offsets
0, 8, 16 and 24, the `u32` at 32, then 4 bytes of padding, so the size is 40 and the alignment 8.
That is exactly the C `FILE_BASIC_INFO` of Windows' headers (its `LARGE_INTEGER` fields are 8-byte
aligned), which `GetFileInformationByHandleEx` expects for `FileBasicInfo`: Windows is told 40 bytes
and writes at most 40, into memory aligned for the structure. The padding is never read as a field,
so whatever Windows leaves there doesn't matter. Nothing else changes: block 4's call, its handle
and its `SAFETY` comment are as they were, and no other `unsafe` was added.

Microsoft: [FILE_BASIC_INFO][file_basic_info]; [FILE_INFO_BY_HANDLE_CLASS][file_info_by_handle_class].

### Block 5: listing a folder (`entries`)

```rust
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
```

**In plain words:** "Windows, write as many entries of this folder as fit into this buffer." The
first call uses `FileIdBothDirectoryRestartInfo` (start from the top); later calls use
`FileIdBothDirectoryInfo` (go on) until Windows answers "no more files". On the first call only,
"file not found" also means the end: a folder with no entries at all (a FAT drive's top folder has
not even `.`). On a later call it is an error, because taking it as the end would cut the listing
short, and the entries left out would look deleted (`listing_done`, which a test checks).

**Why it is sound.**

- `area` comes from `aligned()`: 64 KiB of zeroed bytes inside a `Vec`, starting on an 8-byte
  boundary, because Microsoft says each record "must be aligned on a DWORDLONG (8-byte) boundary".
  The `Vec` lives until `entries` returns.
- `len32(area.len())` is its exact length, so Windows writes nothing past it.
- `listing` is a handle of its own on the same folder (the empty name opens the folder itself), so
  two listings never share a place in the folder. It was opened with `FILE_LIST_DIRECTORY` and
  `FILE_SYNCHRONOUS_IO_NONALERT`, which this call needs.

**Around it, in safe code.** `read_listing` reads each `FILE_ID_BOTH_DIR_INFO` record from the bytes.
Every field goes through `bytes_at` or `wide_at`, which give an error if the field doesn't fit
inside the buffer. The next record's offset must stay inside it too, and every step moves forward,
so the reading always ends. `.` and `..` are skipped. For a reparse point, a record's `EaSize`
field holds its reparse tag (as `FindFirstFileW`'s `dwReserved0` does), which is how a listing
tells a link from a cloud placeholder without opening either. Tests hand-build listings with bad
offsets, names too long, short names too long and names of odd length, and check that each gives
an error.

Microsoft: [GetFileInformationByHandleEx][getfileinformationbyhandleex];
[FILE_ID_BOTH_DIR_INFO][file_id_both_dir_info].

### Block 6: an entry's details (`meta`)

```rust
let mut about = BY_HANDLE_FILE_INFORMATION::default();
// SAFETY: `about` is a whole BY_HANDLE_FILE_INFORMATION, made only of integers, for Windows
// to fill; it keeps no pointer to it. `handle` is open for the call.
if unsafe { GetFileInformationByHandle(handle.as_raw_handle(), &mut about) } == 0 {
```

**In plain words:** "Windows, tell me this file's attributes, last-write time, size and number."

**Why it is sound.** The function takes a pointer to exactly one `BY_HANDLE_FILE_INFORMATION`, and
`about` is one, on the stack. Every field is a `u32`, or a `FILETIME` made of two `u32`s, so any
bytes Windows writes are valid. The handle comes from `metadata` (opened through the held folder
with `FILE_READ_ATTRIBUTES`, which this call needs) or from an open `File`.

**Around it, in safe code.** Windows counts time in 100-nanosecond ticks since 1601; Bolt's
`modified_ns` counts nanoseconds since 1970, so the time is shifted by 116,444,736,000,000,000
ticks and multiplied by 100 (in `i128`, which can't overflow). A test checks it equals the time
the standard library reads.

Microsoft: [GetFileInformationByHandle][getfileinformationbyhandle];
[BY_HANDLE_FILE_INFORMATION][by_handle_file_information].

### Block 7: deleting through a handle (`set_info`, used by `delete`)

```rust
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
```

**In plain words:** "Windows, mark the file this handle has open for deletion." The handle was opened
through the held folder with `DELETE` access, and checked first: `remove_dir` refuses anything but
a folder, `remove_file` refuses a folder that isn't a link.

`delete` passes `FILE_DISPOSITION_INFO_EX` with `FILE_DISPOSITION_FLAG_DELETE` (delete it),
`FILE_DISPOSITION_FLAG_POSIX_SEMANTICS` (the name goes as soon as this handle closes, even while
others still have the file open; they keep using their handles) and
`FILE_DISPOSITION_FLAG_IGNORE_READONLY_ATTRIBUTE` (a read-only file goes too, and so does an empty
read-only folder, as on Unix). Windows
10 before version 1809 refuses that last flag, so the call is then made without it. A drive that
can't do POSIX deletes (FAT) refuses both and gets the older
`FILE_DISPOSITION_INFO { DeleteFile: true }`; the name then goes once the last handle closes.

**Why it is sound.** Windows only reads here, at most the size it is told, which is exactly `value`'s,
and `value` is a borrow that lives through the call. Reading needs no promise about what the bytes
mean. The `bool` in `FILE_DISPOSITION_INFO` is one Bolt writes (`true`), so it is valid.

Microsoft: [SetFileInformationByHandle][setfileinformationbyhandle];
[FILE_DISPOSITION_INFORMATION_EX][file_disposition_information_ex] (the flags and what POSIX
semantics mean; the Win32 `FILE_DISPOSITION_INFO_EX` page has moved);
[FILE_DISPOSITION_INFO][file_disposition_info].

### Block 8: renaming through a handle (`set_rename`)

```rust
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
```

**In plain words:** "Windows, move the file this handle has open to this name, in the folder that
other handle holds, replacing a file there" (for `rename_into`), or "…only if nothing has that
name" (for `symlink`, block 10).

What is passed in `rename`, a `RenameInfo`:

```rust
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
```

Windows' structure ends with a one-character array that "runs on" past the end of the structure for
as long as the name is. `RenameInfo` is the same layout with room for 255 characters (the longest
name Windows' drives hold) and a NUL; a longer name is refused before the call. The `const _`
line checks the layout against windows-sys's own definition while compiling: if they ever differ,
the crate doesn't build. For `rename_into`, `flags` is `FILE_RENAME_REPLACE_IF_EXISTS` (replace a
file at the new name), `FILE_RENAME_POSIX_SEMANTICS` (replace it even while others have it open;
they keep reading the old file, which a test checks) and `FILE_RENAME_IGNORE_READONLY_ATTRIBUTE`
(replace it even if it is read-only, which a test checks too). For `symlink` it is 0, with the
plain `FileRenameInformation`: the move fails if anything has the name. `root` is the target
folder's handle; `name` the new name, relative to it.

**Why it is sound.** `rename` lives through the call, and Windows reads at most
`size_of::<RenameInfo>()` bytes, all inside it. It writes only into `status_block`, a local
variable of the type it expects. `source` is a borrowed handle, open for the call. `root` is a
handle value copied out of the target `Dir`, which the caller borrows for the whole call, so it is
open too.

**Around it, in safe code.** `rename_into` opens `from` through the held folder with `DELETE`
access and refuses it if it is a link; then it looks at `to_name` in the target folder and refuses
a link or a folder there. Windows 10 before version 1809 refuses the read-only flag, so the call is
then made without it. A drive without POSIX renames refuses both, and gets `FileRenameInformation`
with `FILE_RENAME_REPLACE_IF_EXISTS` alone.

Microsoft: [NtSetInformationFile][ntsetinformationfile];
[FILE_RENAME_INFORMATION][file_rename_information] (all three flags are on this page). For why not
the Win32 call: [FILE_RENAME_INFO][file_rename_info].

### Block 9: the held folder's path (`final_path`)

```rust
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
```

**In plain words:** "Windows, where is the folder this handle holds right now?" The answer looks like
`\\?\C:\Users\…\project\sub`. It is needed only to make a link (block 10).

**Why it is sound.** `path` is a `Vec<u16>` of `path.len()` characters, and that is the length
Windows is told, in characters. Microsoft's rules for the answer: a success returns the path's
length without the NUL (always less than the room given); too little room returns the room needed,
NUL included. The loop grows the buffer to that and asks again, at most three times.
`FILE_NAME_NORMALIZED` asks for the full long names, `VOLUME_NAME_DOS` for a drive letter.

Microsoft: [GetFinalPathNameByHandleW][getfinalpathnamebyhandlew].

### Block 10: making a link (`symlink`)

```rust
// SAFETY: `path` and `target` are NUL-terminated UTF-16 strings that live through the call;
// CreateSymbolicLinkW only reads them and keeps no pointer to them.
if !unsafe { CreateSymbolicLinkW(path.as_ptr(), target.as_ptr(), flags) } {
```

**In plain words:** Windows has no "make a link inside this folder handle" call, so this is the
one step Bolt has to do by path. To keep a moved folder from redirecting it, `symlink` goes in four
steps. Its caller passes two held folders: the one the link belongs in, and a scratch folder,
`.bolt/tmp`.

1. Through the held folder, check that nothing has the name yet.
2. Ask where the scratch folder is now (block 9), and make the link there, by that path, under a
   name nobody chose, from `bolt_platform::temp_name`: `tmp-` and 16 random lowercase hexadecimal
   digits, the form of every name Bolt gives what it makes in `.bolt/tmp`. It has no extension, so
   nothing runs it by its name.
3. Open that random name through the scratch folder's handle, and check that it is a symlink and
   that its target (read by block 11, called as block 12) is exactly the one asked for.
4. Only then move that very link, through its own handle, into the held folder under the name asked
   for (block 8, with no replacing: if something took the name meanwhile, the move fails). The two
   folders must be on the same drive, as `.bolt/tmp` and the project are.

If step 3 or 4 fails after finding a symlink under the random name, that symlink is removed again
(removing a link never touches what it points to), and the error comes back. In three cases the
link stays in `.bolt/tmp` under its `tmp-` name, never in the user's folder:

- Bolt stops between steps 2 and 4 (a crash, a power cut).
- Step 3 can't open it for another reason than "not found" (say, a virus scanner holds it).
- Removing it again fails.

It stays there for now. Bolt's start-up cleanup of `.bolt/tmp` removes only regular files today;
it learns to remove such links, through `Dir::remove_file`, with Task 3, which opens `.bolt`'s
fixed folders through `Dir`.

The arguments: `path` is the scratch folder's path, `\` and the random name, then a NUL. `target`
is what the link points to, then a NUL; a target that holds a NUL is refused before the call.
`flags` is `SYMBOLIC_LINK_FLAG_ALLOW_UNPRIVILEGED_CREATE` (lets a process that isn't an
administrator make links when Developer Mode is on), plus `SYMBOLIC_LINK_FLAG_DIRECTORY` for a
folder link.

**Why it is sound.** Both strings are `Vec<u16>`s ending in NUL, alive through the call; the
function only reads them. One more point: windows-sys declares this function as returning a Rust
`bool` (Windows' `BOOLEAN`), and a byte other than 0 or 1 there would be undefined behaviour.
Windows returns `TRUE` (1) or `FALSE` (0), and the standard library declares and calls it the same
way.

**What a race can still do.** The window is inside step 2: between reading the scratch folder's
path (block 9) and this call making the link. Someone who renames the scratch folder (or a folder
above it) in that moment, and puts a junction at its old path, makes Windows put the link wherever
the junction points. Step 3 then doesn't find it in the scratch folder, so nothing is moved, and
Bolt reports the error. What is left is one stray link with a meaningless name and the save's
target, never a link at a name the save chose (`.git\hooks\pre-commit`, say). The random digits
come from Rust's `RandomState`, whose keys come from the system's random number generator, so the
name can't be guessed in advance. Decision 048 records that links on Windows are made by path. Only someone who
can rename folders inside the project can try this, and the owner checks (decisions 025 and 026)
keep Bolt out of projects other accounts own. A test runs exactly this race.

Step 3 compares the target Windows stored, so an absolute target must be written the way Windows
stores it (`C:\x`, with no `.`, `..` or `/`); otherwise the check fails. A relative target, the
only kind Bolt writes as a link (decision 052 writes absolute ones as placeholders), is stored as
given.

Without Developer Mode or an administrator, Windows answers error 1314 ("a required privilege is
not held"); `symlink` returns it unchanged, so callers can write a placeholder instead.

Microsoft: [CreateSymbolicLinkW][createsymboliclinkw] (both flags are on this page).

### Block 11: reading a link's target (`target_of`, an `unsafe fn`)

```rust
/// The target of the link `link` has open, as `link_target` shows it.
///
/// # Safety
///
/// `link` must be open for synchronous calls (FILE_SYNCHRONOUS_IO_NONALERT, which `nt_open` asks
/// for every time). On a handle open for overlapped calls, Windows would finish the call later,
/// writing into a buffer this function has already freed.
unsafe fn target_of(link: BorrowedHandle<'_>) -> io::Result<Vec<u16>> {
```

```rust
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
```

**In plain words:** "Windows, give me the stored data of this link." `link` was opened with
`FILE_OPEN_REPARSE_POINT`, so it is the link itself: by `read_link` through the held folder, or by
`symlink`'s check through the scratch folder (block 10, step 3).

**An `unsafe fn`** is a function with a promise its caller must keep, which the compiler can't
check. The promise is written in its `# Safety` section, and every call must sit in an `unsafe`
block whose `// SAFETY:` comment says why the promise holds (blocks 12 and 13). Here the promise
is that the handle was opened for synchronous calls. Microsoft: "If hDevice was opened without
specifying FILE_FLAG_OVERLAPPED, lpOverlapped is ignored. If hDevice was opened with the
FILE_FLAG_OVERLAPPED flag, the operation is performed as an overlapped (asynchronous) operation."
On such a handle, a null `OVERLAPPED` could let Windows write into `area` after this function had
returned and freed it. Making the function `unsafe` means a future caller can't pass a handle
without writing `unsafe` and a reason: the compiler refuses the call outside an `unsafe` block, and
Clippy's `undocumented_unsafe_blocks` lint, set to `deny` in this crate, refuses a block without a
`// SAFETY:` comment. So no overlapped handle slips in unnoticed.

**Why it is sound.** `FSCTL_GET_REPARSE_POINT` takes no input (null and 0, as Microsoft's page
shows). `area` is 16 KiB (`MAXIMUM_REPARSE_DATA_BUFFER_SIZE`, the most a link's data can be),
zeroed and 8-byte aligned, and Windows is told its exact length. `filled` is a local `u32`;
Microsoft: "If lpOverlapped is NULL, lpBytesReturned cannot be NULL", and it isn't. The handle is
synchronous, as the caller promises, so the call is over when it returns and needs no
`OVERLAPPED`.

**Around it, in safe code.** `link_target` reads the `REPARSE_DATA_BUFFER` layout from the first
`filled` bytes: the tag, then for a symlink or junction where the "substitute name" (the path
Windows follows) is. Every offset and length is checked against the data, and tests check that
data cut short at every byte gives an error. A relative target comes back as it is stored. An
absolute one is stored as an NT path (`\??\C:\x`) and comes back as the standard library shows it:
`C:\x`, `\\server\share` for `\??\UNC\server\share`, and `\\?\…` for anything else. One small
difference from the standard library: it keeps `\\?\C:\…` for a path that wouldn't come back the
same from `GetFullPathNameW` (over 260 characters, `.` or `..` in it); Bolt drops the prefix there
too, as Git does. Only absolute targets are affected, and Bolt writes those as placeholders anyway
(decision 052).

Microsoft: [DeviceIoControl][deviceiocontrol]; [FSCTL_GET_REPARSE_POINT][fsctl_get_reparse_point];
[REPARSE_DATA_BUFFER][reparse_data_buffer]. Rust: [unsafe functions][unsafe_functions].

### Blocks 12 and 13: the two calls of `target_of` (`symlink`, `read_link`)

```rust
// SAFETY: `link` was opened by `nt_open`, which opens every handle for synchronous calls.
if unsafe { target_of(link.as_handle()) }? != wanted {
```

```rust
// SAFETY: `link` was opened by `nt_open`, which opens every handle for synchronous calls.
let target = unsafe { target_of(link.as_handle()) }?;
```

**In plain words:** each keeps block 11's promise. In both, `link` comes straight from `nt_open`
(through `open_entry` in `symlink`), and `nt_open` adds `FILE_SYNCHRONOUS_IO_NONALERT` to every
open it makes (block 1). Nothing else in the file calls `target_of`.

## What an entry is, on Windows

Not `unsafe`, but it decides what is refused. `kind_of` reads the attributes and the reparse tag:

- No `FILE_ATTRIBUTE_REPARSE_POINT`: a folder if `FILE_ATTRIBUTE_DIRECTORY`, else a file.
- A reparse point whose tag has the "name surrogate" bit (`0x20000000`, what Windows'
  `IsReparseTagNameSurrogate` tests): a link. Symlinks and junctions have it.
- Any other reparse point (OneDrive placeholders, deduplicated files, app aliases): `Other`, refused
  like a link by `open_file` and `open_dir`. OneDrive usually hides its placeholders from programs
  like Bolt, so this is rare.

Microsoft: [reparse point tags][reparse_point_tags]; [IsReparseTagNameSurrogate][isreparsetagnamesurrogate].

## How it was tested

Where the tests ran:

- **CI run 41985581123, on commit ef05ef7:** Windows, macOS and Linux all green. On Windows the
  three tests marked *(makes symlinks on Windows)* below ran and passed. Under CI they fail on
  error 1314 ("a required privilege is not held") instead of skipping, so "ok" there means they
  made links.
- **This machine (Windows 11, no Developer Mode):** everything below. The parts that make symlinks
  print a reason and skip, and with `CI` set here they fail, as they should.

The tests:

- `a_link_at_a_name_is_refused_at_every_call` *(makes symlinks on Windows)*: links to a folder and
  a file (and a junction on Windows) are refused by `open_dir`, `open_file`, `open_or_create`,
  `remove_dir`, `rename_into` at either name, `open_top`, and on Unix `set_executable`; `metadata`
  calls each a link; nothing changes on disk. Making `as_folder` accept a link makes this test and
  the next one fail. The junction part needs no right, so it runs on this machine too.
- `a_folder_swapped_for_a_link_after_opening_stays_the_folder`: a held folder renamed away and
  replaced by a link (a junction on Windows) still receives the new file and folder; nothing lands
  where the link points.
- `rename_into_replaces_a_file_atomically`: a reader holding the old file keeps reading it while
  the name already holds the new one; a free name is filled; a folder is never replaced.
- `names_with_separators_or_dots_are_refused_before_any_call`: `a/b`, `.`, `..`, the empty name and
  NUL (and `a\b`, `a:b` on Windows) are refused by every call, and nothing is made.
- `entries_and_metadata_describe_what_is_there`: kinds, sizes, times (equal to the standard
  library's), file IDs, executable bits, `read_link` (equal to the standard library's), the errors
  for a folder opened as a file and the other way round, and removal of a link, a full folder and an
  empty one.
- `new_files_get_exact_modes` (Unix): 0644 and 0755 exactly under umasks 0o077 and 0o000.
- `short_names_come_with_entries` (Windows): `.git`'s short name equals what Windows' own `cmd`
  shows.
- `links_are_made_for_folders_and_files` (Windows) *(makes symlinks on Windows)*: folder, file and
  absolute links made through `Dir`, by way of a scratch folder, work for Windows and read back as
  the standard library reads them; a name already taken is never replaced; nothing is left in the
  scratch folder, nor under any other name.
- `a_link_raced_out_of_its_folder_lands_under_no_chosen_name` (Windows) *(makes symlinks on
  Windows)*: a test-only hook renames the scratch folder and puts a junction at its old path inside
  step 2 of block 10, between reading the scratch folder's path and making the link. `symlink`
  must fail; nothing may appear at the name in the held folder, nor in the moved scratch folder;
  and where the junction points there must be only one stray link, named `tmp-` and 16 lowercase
  hexadecimal digits, never `pre-commit`.
- `a_read_only_file_can_be_removed_or_replaced` (every system): `remove_file` removes a read-only
  file, `rename_into` replaces one, and `remove_dir` removes an empty read-only folder. Before the
  read-only flags, Windows answered "access denied".
- `nothing_is_made_through_a_dangling_link` (every system): `create_new`, `create_dir` and
  `symlink` at a name holding a link that points nowhere fail with "already exists", and
  `open_or_create` refuses the link; nothing appears where the link points, and nothing in the
  scratch folder.
- `open_or_create_makes_a_file_or_opens_one_and_keeps_what_is_in_it` (every system): the file is
  made once, opened again with what was written still in it, written and locked; a folder at the
  name is refused.
- In `dir/windows.rs`: hand-built listings and link data, good and bad; the folder's path equals
  the standard library's `canonicalize`; a name over 255 characters is refused; "file not found"
  ends only the first listing call; a link's temporary names are random, have no extension, and
  take `.bolt/tmp`'s form (`tmp-` and 16 lowercase hexadecimal digits).

Not tested anywhere yet: the FAT and exFAT fallbacks, and the older-Windows fallbacks for the
read-only flags (no such drive or Windows in CI); OneDrive files opened this way and paths over 260
characters (decision 048 lists both for a real machine); a real non-link reparse point on disk
(only through hand-built listings); a drive that can't hold Unix modes, where `create_new` now
removes the file it couldn't set (no such drive in CI).

[file_rename_info]: https://learn.microsoft.com/en-us/windows/win32/api/winbase/ns-winbase-file_rename_info
[file_rename_information]: https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/ntifs/ns-ntifs-_file_rename_information
[ntcreatefile]: https://learn.microsoft.com/en-us/windows/win32/api/winternl/nf-winternl-ntcreatefile
[object_attributes]: https://learn.microsoft.com/en-us/windows/win32/api/ntdef/ns-ntdef-_object_attributes
[unicode_string]: https://learn.microsoft.com/en-us/windows/win32/api/ntdef/ns-ntdef-_unicode_string
[io_status_block]: https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/wdm/ns-wdm-_io_status_block
[ntstatus]: https://learn.microsoft.com/en-us/windows-hardware/drivers/kernel/using-ntstatus-values
[ownedhandle]: https://doc.rust-lang.org/std/os/windows/io/struct.OwnedHandle.html
[from_raw_handle]: https://doc.rust-lang.org/std/os/windows/io/trait.FromRawHandle.html#tymethod.from_raw_handle
[rtlntstatustodoserror]: https://learn.microsoft.com/en-us/windows/win32/api/winternl/nf-winternl-rtlntstatustodoserror
[getfileinformationbyhandleex]: https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-getfileinformationbyhandleex
[file_info_by_handle_class]: https://learn.microsoft.com/en-us/windows/win32/api/minwinbase/ne-minwinbase-file_info_by_handle_class
[file_attribute_tag_info]: https://learn.microsoft.com/en-us/windows/win32/api/winbase/ns-winbase-file_attribute_tag_info
[file_id_info]: https://learn.microsoft.com/en-us/windows/win32/api/winbase/ns-winbase-file_id_info
[file_basic_info]: https://learn.microsoft.com/en-us/windows/win32/api/winbase/ns-winbase-file_basic_info
[unsafe_traits]: https://doc.rust-lang.org/book/ch20-01-unsafe-rust.html#implementing-an-unsafe-trait
[unsafe_functions]: https://doc.rust-lang.org/book/ch20-01-unsafe-rust.html#calling-an-unsafe-function-or-method
[file_id_both_dir_info]: https://learn.microsoft.com/en-us/windows/win32/api/winbase/ns-winbase-file_id_both_dir_info
[getfileinformationbyhandle]: https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-getfileinformationbyhandle
[by_handle_file_information]: https://learn.microsoft.com/en-us/windows/win32/api/fileapi/ns-fileapi-by_handle_file_information
[setfileinformationbyhandle]: https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-setfileinformationbyhandle
[file_disposition_information_ex]: https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/ntddk/ns-ntddk-_file_disposition_information_ex
[file_disposition_info]: https://learn.microsoft.com/en-us/windows/win32/api/winbase/ns-winbase-file_disposition_info
[ntsetinformationfile]: https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/ntifs/nf-ntifs-ntsetinformationfile
[getfinalpathnamebyhandlew]: https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-getfinalpathnamebyhandlew
[createsymboliclinkw]: https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-createsymboliclinkw
[deviceiocontrol]: https://learn.microsoft.com/en-us/windows/win32/api/ioapiset/nf-ioapiset-deviceiocontrol
[fsctl_get_reparse_point]: https://learn.microsoft.com/en-us/windows/win32/api/winioctl/ni-winioctl-fsctl_get_reparse_point
[reparse_data_buffer]: https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/ntifs/ns-ntifs-_reparse_data_buffer
[reparse_point_tags]: https://learn.microsoft.com/en-us/windows/win32/fileio/reparse-point-tags
[isreparsetagnamesurrogate]: https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/ntifs/nf-ntifs-isreparsetagnamesurrogate
