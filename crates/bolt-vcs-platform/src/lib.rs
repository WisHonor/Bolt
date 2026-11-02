//! Operating-system calls for Bolt.
//!
//! The only crate where `unsafe` is allowed, and only for these calls.

use std::fs::OpenOptions;
use std::io;
use std::path::Path;

/// Makes `options` never follow a link at the end of the path, so callers can refuse it. Unix:
/// opening a link fails (`O_NOFOLLOW`); opening a pipe doesn't wait for a writer (`O_NONBLOCK`).
/// Windows: a link, junction or reparse point opens as itself. Replaces custom flags.
pub fn no_follow(options: &mut OpenOptions) -> &mut OpenOptions {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // Regular files ignore O_NONBLOCK.
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        use windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT;
        options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
    }
}

/// Hides `path` in file browsers. On Windows this sets the "hidden" attribute; elsewhere a
/// leading dot already hides it, so this only checks that `path` exists.
pub fn hide_path(path: &Path) -> io::Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use std::os::windows::fs::MetadataExt;
        use windows_sys::Win32::Storage::FileSystem::{FILE_ATTRIBUTE_HIDDEN, SetFileAttributesW};

        let attributes = std::fs::symlink_metadata(path)?.file_attributes();
        // shortcut: no `\\?\` prefix, so paths over 260 characters fail with an error (never
        // touch the wrong file); upgrade if projects that deep turn up.
        let mut wide: Vec<u16> = path.as_os_str().encode_wide().collect();
        if wide.contains(&0) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "path contains a NUL character",
            ));
        }
        wide.push(0);
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

/// Who owns a file or folder, as far as Bolt can tell (decisions 025 and 026).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Owner {
    /// The user running Bolt. Unix: the effective user. Windows: the user's own account, or the
    /// Administrators group when the user is an administrator, elevated or not.
    User,
    /// Another user account.
    Other,
    /// Can't tell: the drive records no owner (FAT, exFAT, some network shares), records
    /// "Everyone", or records something that isn't a well-formed account ID. Windows only: on
    /// Unix every file has an owner.
    Unknown,
}

/// Who owns `path`, so a `.bolt` that another account planted above the user's work is refused
/// (decisions 025 and 026, T23). A link or junction at the end of `path` is not followed: its own
/// owner is the answer. A missing `path` gives `NotFound`.
pub fn owner_of(path: &Path) -> io::Result<Owner> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let owner = std::fs::symlink_metadata(path)?.uid();
        // SAFETY: geteuid takes no arguments, cannot fail, and only returns a number.
        let is_user = owner == unsafe { libc::geteuid() };
        Ok(if is_user { Owner::User } else { Owner::Other })
    }
    #[cfg(windows)]
    {
        windows_owner::owner_of(path)
    }
}

/// The Windows half of `owner_of`.
#[cfg(windows)]
mod windows_owner {
    use super::Owner;
    use std::fs::OpenOptions;
    use std::io;
    use std::mem::offset_of;
    use std::os::windows::fs::OpenOptionsExt;
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
    use std::path::Path;
    use std::ptr::null_mut;

    use windows_sys::Win32::Foundation::{
        ERROR_INVALID_FUNCTION, ERROR_NOT_SUPPORTED, ERROR_SUCCESS, LocalFree,
    };
    use windows_sys::Win32::Security::Authorization::{GetSecurityInfo, SE_FILE_OBJECT};
    use windows_sys::Win32::Security::{
        EqualSid, GetTokenInformation, IsValidSid, IsWellKnownSid, OWNER_SECURITY_INFORMATION,
        PSECURITY_DESCRIPTOR, PSID, SID_AND_ATTRIBUTES, TOKEN_GROUPS, TOKEN_INFORMATION_CLASS,
        TOKEN_QUERY, TOKEN_USER, TokenGroups, TokenUser, WinBuiltinAdministratorsSid, WinWorldSid,
    };
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, READ_CONTROL,
    };
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    pub fn owner_of(path: &Path) -> io::Result<Owner> {
        // Opened only to read its security information (READ_CONTROL). BACKUP_SEMANTICS lets a
        // folder open; OPEN_REPARSE_POINT opens a link or junction itself, not what it points
        // to. std adds the `\\?\` prefix, so long paths work.
        let file = OpenOptions::new()
            .access_mode(READ_CONTROL)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
            .open(path)?;

        let mut owner: PSID = null_mut();
        let mut descriptor: PSECURITY_DESCRIPTOR = null_mut();
        // SAFETY: `file` is an open handle with READ_CONTROL access, and stays open for the whole
        // call. `owner` and `descriptor` are valid places for Windows to write one pointer each.
        // The three null pointers ask for no group, DACL or SACL.
        let status = unsafe {
            GetSecurityInfo(
                file.as_raw_handle(),
                SE_FILE_OBJECT,
                OWNER_SECURITY_INFORMATION,
                &mut owner,
                null_mut(),
                null_mut(),
                null_mut(),
                &mut descriptor,
            )
        };
        match status {
            ERROR_SUCCESS => {}
            // A file system or network share that records no owner: not shown to be the user's,
            // so Bolt refuses the folder unless the user lists it in safe-folders.
            ERROR_NOT_SUPPORTED | ERROR_INVALID_FUNCTION => return Ok(Owner::Unknown),
            _ => return Err(io::Error::from_raw_os_error(status as i32)),
        }
        // Frees the descriptor when this function returns, on every path. `owner` points into
        // it, so it must not be used after that.
        let _descriptor = SecurityDescriptor(descriptor);
        // No owner recorded: can't tell either.
        if owner.is_null() {
            return Ok(Owner::Unknown);
        }
        // An owner that is not a well-formed SID names nobody: can't tell (Git refuses it too).
        // SAFETY: `owner` is not null (checked just above) and points into the descriptor, which
        // stays allocated until this function returns; IsValidSid only reads it.
        if unsafe { IsValidSid(owner) } == 0 {
            return Ok(Owner::Unknown);
        }
        // FAT and exFAT record no owner and Windows reports "Everyone": can't tell (as Git does).
        // SAFETY: `owner` is a valid SID in the descriptor (checked above); IsWellKnownSid only
        // reads it.
        if unsafe { IsWellKnownSid(owner, WinWorldSid) } != 0 {
            return Ok(Owner::Unknown);
        }

        let token = process_token()?;
        let (user, user_filled) = token_information(&token, TokenUser)?;
        let user_sid = user_sid_in(&user, user_filled)?;
        // SAFETY: `owner` is a valid SID in the descriptor (checked above), and `user_sid` points
        // to the user's SID in `user`, which Windows filled; both stay allocated until this
        // function returns, and EqualSid only reads them.
        if unsafe { EqualSid(owner, user_sid) } != 0 {
            return Ok(Owner::User);
        }

        // SAFETY: `owner` is a valid SID in the descriptor (checked above); IsWellKnownSid only
        // reads it.
        if unsafe { IsWellKnownSid(owner, WinBuiltinAdministratorsSid) } == 0 {
            return Ok(Owner::Other);
        }
        // The Administrators group counts as the user if it is in their token: enabled when
        // elevated, "for deny only" when not. CheckTokenMembership counts enabled groups only and
        // would lock an administrator out of a project started in an elevated terminal.
        let (group_buffer, groups_filled) = token_information(&token, TokenGroups)?;
        let groups = groups_in(&group_buffer, groups_filled)?;
        let in_group = groups.iter().any(|group| {
            // SAFETY: the group's SID points into `group_buffer`, which Windows filled and
            // `groups` borrows, and `owner` is a valid SID in the descriptor; both stay
            // allocated for the call, and EqualSid only reads them.
            unsafe { EqualSid(group.Sid, owner) != 0 }
        });
        Ok(if in_group { Owner::User } else { Owner::Other })
    }

    /// The user's SID in a TOKEN_USER that Windows wrote into the first `filled` bytes of
    /// `buffer`. The SID is inside `buffer`, so it must not be used once `buffer` is gone.
    fn user_sid_in(buffer: &[usize], filled: usize) -> io::Result<PSID> {
        if filled < size_of::<TOKEN_USER>() || filled > size_of_val(buffer) {
            return Err(bad_token_information());
        }
        // SAFETY: a whole TOKEN_USER lies in the filled part of `buffer` (checked just above),
        // and `buffer` is aligned for it (a TOKEN_USER holds a pointer; `buffer` is made of
        // usizes). This copies out the SID pointer and reads nothing through it.
        Ok(unsafe { (*buffer.as_ptr().cast::<TOKEN_USER>()).User.Sid })
    }

    /// The groups in a TOKEN_GROUPS that Windows wrote into the first `filled` bytes of `buffer`: a
    /// count, then that many SID_AND_ATTRIBUTES (the type's one-element array runs past its end).
    /// The slice borrows `buffer`, so it can't outlive it.
    fn groups_in(buffer: &[usize], filled: usize) -> io::Result<&[SID_AND_ATTRIBUTES]> {
        let first = offset_of!(TOKEN_GROUPS, Groups);
        if filled < first || filled > size_of_val(buffer) {
            return Err(bad_token_information());
        }
        let header = buffer.as_ptr().cast::<TOKEN_GROUPS>();
        // SAFETY: the count is the first field, and it lies in the filled part of `buffer`
        // (checked just above), which is aligned for TOKEN_GROUPS (a TOKEN_GROUPS holds
        // pointers; `buffer` is made of usizes). This reads the count alone.
        let count = unsafe { (*header).GroupCount } as usize;
        // Where the list ends: past what Windows filled (or past any size at all) is refused.
        let end = count
            .checked_mul(size_of::<SID_AND_ATTRIBUTES>())
            .and_then(|list| list.checked_add(first));
        if end.is_none_or(|end| end > filled) {
            return Err(bad_token_information());
        }
        // SAFETY: the `count` entries lie in `buffer`'s filled part (checked above): initialised,
        // one allocation, aligned like `buffer`'s usizes. `header` reaches the buffer, and
        // `&raw const` makes no reference to the one declared element. `buffer` stays borrowed.
        Ok(unsafe {
            std::slice::from_raw_parts(
                (&raw const (*header).Groups).cast::<SID_AND_ATTRIBUTES>(),
                count,
            )
        })
    }

    /// Token information that is not the size Windows said it is.
    fn bad_token_information() -> io::Error {
        io::Error::other("Windows gave Bolt token information of an unexpected size")
    }

    /// A security descriptor Windows allocated for Bolt (with LocalAlloc); dropping it frees it.
    struct SecurityDescriptor(PSECURITY_DESCRIPTOR);

    impl Drop for SecurityDescriptor {
        fn drop(&mut self) {
            // SAFETY: the pointer came from a successful GetSecurityInfo, which leaves it to the
            // caller to free with LocalFree (which ignores a null pointer). Only this guard holds
            // it, and drop runs once.
            unsafe { LocalFree(self.0) };
        }
    }

    /// This process's access token, opened for queries. Dropping the handle closes it.
    fn process_token() -> io::Result<OwnedHandle> {
        let mut token = null_mut();
        // SAFETY: GetCurrentProcess returns a stand-in handle for this process that is always
        // valid and is never closed. `token` is a valid place for Windows to write the new handle.
        if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: OpenProcessToken succeeded, so `token` is an open handle that nothing else
        // holds. OwnedHandle closes it exactly once, when dropped.
        Ok(unsafe { OwnedHandle::from_raw_handle(token) })
    }

    /// The `class` information of `token` (a TOKEN_USER or TOKEN_GROUPS): a buffer, and how many
    /// bytes at its start Windows filled. These structures hold pointers, so the buffer is made
    /// of `usize`s, never bytes, to be aligned for them.
    fn token_information(
        token: &OwnedHandle,
        class: TOKEN_INFORMATION_CLASS,
    ) -> io::Result<(Vec<usize>, usize)> {
        // First ask for the size alone: with no buffer, the call fails with
        // ERROR_INSUFFICIENT_BUFFER and writes the size it needs to `bytes`.
        let mut bytes = 0;
        // SAFETY: a null buffer with a length of 0 asks Windows to write nothing but the size,
        // and `bytes` is a valid place for it. `token` is open for the call.
        unsafe { GetTokenInformation(token.as_raw_handle(), class, null_mut(), 0, &mut bytes) };
        if bytes == 0 {
            return Err(io::Error::last_os_error());
        }
        let mut buffer = vec![0_usize; (bytes as usize).div_ceil(size_of::<usize>())];
        let mut written = 0;
        // SAFETY: `buffer` holds at least `bytes` bytes and Windows writes at most that many.
        // `written` is a valid place for the length. `token` is open for the call. If the size grew
        // since the first call, this fails with ERROR_INSUFFICIENT_BUFFER, returned as an error.
        let ok = unsafe {
            GetTokenInformation(
                token.as_raw_handle(),
                class,
                buffer.as_mut_ptr().cast(),
                bytes,
                &mut written,
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        // It shrank since the first call: refused rather than read in part.
        if written != bytes {
            return Err(bad_token_information());
        }
        Ok((buffer, bytes as usize))
    }

    #[cfg(test)]
    mod tests {
        #![allow(clippy::unwrap_used)]
        use super::*;

        #[test]
        fn the_process_token_reads_whole() {
            let token = process_token().unwrap();
            let (user, filled) = token_information(&token, TokenUser).unwrap();
            assert!(!user_sid_in(&user, filled).unwrap().is_null());
            let (groups, filled) = token_information(&token, TokenGroups).unwrap();
            // Every token is in some groups, Everyone among them.
            assert!(!groups_in(&groups, filled).unwrap().is_empty());
        }

        #[test]
        fn token_information_too_short_for_its_contents_is_refused() {
            let mut buffer = [0_usize; 5];
            let whole = size_of_val(&buffer);
            // A TOKEN_USER: all of it filled, and inside the buffer.
            let user = size_of::<TOKEN_USER>();
            assert!(user_sid_in(&buffer, user - 1).is_err());
            assert!(user_sid_in(&buffer, whole + 1).is_err());
            assert!(user_sid_in(&buffer, user).is_ok());

            // A TOKEN_GROUPS: its count, then that many entries, all inside what was filled.
            let first = offset_of!(TOKEN_GROUPS, Groups);
            let entry = size_of::<SID_AND_ATTRIBUTES>();
            assert!(groups_in(&buffer, first - 1).is_err());
            assert!(groups_in(&buffer, first).unwrap().is_empty());
            // The count is the first field: all of `buffer[0]` on 32-bit Windows, and its low
            // half on 64-bit (Windows is little-endian).
            buffer[0] = 2;
            assert!(groups_in(&buffer, first + entry).is_err());
            assert_eq!(groups_in(&buffer, first + 2 * entry).unwrap().len(), 2);
            assert!(groups_in(&buffer, whole + 1).is_err());
            // A count whose list runs far past the buffer, or overflows on 32-bit.
            buffer[0] = u32::MAX as usize;
            assert!(groups_in(&buffer, whole).is_err());
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn own_temp_folder_is_owned() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(owner_of(dir.path()).unwrap(), Owner::User);
    }

    #[test]
    fn system_folder_is_not_owned() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            // Whoever runs the tests owns the folders they make. When that is root, `/` is
            // theirs too, and there is no folder of someone else's to try.
            let me = std::fs::metadata(tempfile::tempdir().unwrap().path())
                .unwrap()
                .uid();
            if std::fs::metadata("/").unwrap().uid() == me {
                return;
            }
            assert_eq!(owner_of(Path::new("/")).unwrap(), Owner::Other);
        }
        #[cfg(windows)]
        {
            let windows = std::env::var_os("SystemRoot").unwrap();
            assert_eq!(owner_of(Path::new(&windows)).unwrap(), Owner::Other);
        }
    }

    /// A link's own owner is the answer, never its target's: following a planted `.bolt` link
    /// could reach another computer (decision 026).
    #[test]
    fn a_link_is_judged_by_its_own_owner() {
        let dir = tempfile::tempdir().unwrap();
        let link = dir.path().join("link");
        // Followed, these would give NotFound (Unix: nothing there) or Other (Windows: a junction
        // to the Windows folder, which TrustedInstaller owns).
        #[cfg(unix)]
        std::os::unix::fs::symlink(dir.path().join("missing"), &link).unwrap();
        #[cfg(windows)]
        {
            let made = std::process::Command::new("cmd")
                .args(["/D", "/C", "mklink", "/J"])
                .arg(&link)
                .arg(std::env::var_os("SystemRoot").unwrap())
                .stdout(std::process::Stdio::null())
                .status()
                .unwrap();
            assert!(made.success());
        }
        assert_eq!(owner_of(&link).unwrap(), Owner::User);
    }

    #[test]
    fn missing_path_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let err = owner_of(&dir.path().join("missing")).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
    }

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
