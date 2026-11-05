//! Folders held open, and everything in them reached through the held folder one name at a time
//! (decision 048). No call looks a path up, so a parent renamed or swapped for a link after it was
//! opened can't redirect anything. A link or other reparse point at a name is never followed:
//! calls that would act on what it points to refuse it (`is_link_refusal`), and `metadata`,
//! `read_link` and `remove_file` deal with the link itself.

use std::ffi::{OsStr, OsString};
use std::fmt;
use std::fs::File;
use std::io;
use std::path::Path;

#[cfg(unix)]
mod unix;
#[cfg(unix)]
use unix as sys;
#[cfg(windows)]
mod windows;
#[cfg(windows)]
use windows as sys;

/// An open folder. What is in it is reached through it, one name at a time.
#[derive(Debug)]
pub struct Dir {
    handle: sys::Handle,
}

/// What an entry is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryType {
    File,
    Dir,
    /// A symlink; on Windows also a junction or another link.
    Symlink,
    /// A pipe, socket or device; on Windows a reparse point that isn't a link (a cloud placeholder,
    /// say), which is refused like a link.
    Other,
}

/// One entry of a folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirEntry {
    pub name: OsString,
    pub kind: EntryType,
    /// Whether the system marks it as a folder: on Windows a folder link or junction too, and a
    /// reparse point that isn't a link, such as a cloud folder, whose `kind` is `Other`.
    pub folder: bool,
    /// Windows only: the 8.3 name the drive also gave the entry, if any (`GIT~1` for `.git`).
    pub short_name: Option<OsString>,
}

/// An entry's details: a link's own, never those of what it points to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EntryMeta {
    pub kind: EntryType,
    /// In bytes.
    pub size: u64,
    /// When it last changed, in nanoseconds since 1970-01-01 UTC.
    pub modified_ns: i128,
    /// Whether an executable bit is set; `None` on Windows, which has none.
    pub executable: Option<bool>,
    /// The drive's number and the file's number on it: two names with the same ID are one file.
    pub file_id: (u64, u128),
}

impl Dir {
    /// Opens the folder at `path`, which the caller has already resolved. A link at its last name
    /// is refused.
    pub fn open_top(path: &Path) -> io::Result<Dir> {
        sys::open_top(path).map(Dir::held)
    }

    /// Opens the folder `name` in this one.
    pub fn open_dir(&self, name: &OsStr) -> io::Result<Dir> {
        one_name(name)?;
        sys::open_dir(&self.handle, name).map(Dir::held)
    }

    fn held(handle: sys::Handle) -> Dir {
        #[cfg(feature = "test-support")]
        open_count::opened();
        Dir { handle }
    }

    /// What is in this folder, without `.` and `..`, in no particular order.
    pub fn entries(&self) -> io::Result<Vec<DirEntry>> {
        sys::entries(&self.handle)
    }

    /// The details of `name` itself.
    pub fn metadata(&self, name: &OsStr) -> io::Result<EntryMeta> {
        one_name(name)?;
        sys::metadata(&self.handle, name)
    }

    /// The details of an open file, to check that it is the entry `metadata` described.
    pub fn file_meta(file: &File) -> io::Result<EntryMeta> {
        sys::file_meta(file)
    }

    /// Opens the regular file `name` to read.
    pub fn open_file(&self, name: &OsStr) -> io::Result<File> {
        one_name(name)?;
        sys::open_file(&self.handle, name)
    }

    /// Makes the file `name`, to write and read, unless something already has that name. On Unix
    /// its mode is exactly 0644, or 0755 if `executable`, whatever the umask.
    pub fn create_new(&self, name: &OsStr, executable: bool) -> io::Result<File> {
        one_name(name)?;
        sys::create_new(&self.handle, name, executable)
    }

    /// Opens the regular file `name` to read and write, making it if nothing has that name, and
    /// keeping what is in it. A link there is refused, never followed, so nothing is made where it
    /// points.
    pub fn open_or_create(&self, name: &OsStr) -> io::Result<File> {
        one_name(name)?;
        sys::open_or_create(&self.handle, name)
    }

    /// Moves `from` in this folder to `to_name` in `to`, replacing a file there in one step: the
    /// name holds the old file or the new one, never neither. A link at either name is refused,
    /// and a folder is never replaced.
    pub fn rename_into(&self, from: &OsStr, to: &Dir, to_name: &OsStr) -> io::Result<()> {
        one_name(from)?;
        one_name(to_name)?;
        sys::rename_into(&self.handle, from, &to.handle, to_name)
    }

    /// Makes the folder `name`, unless something already has that name.
    pub fn create_dir(&self, name: &OsStr) -> io::Result<()> {
        one_name(name)?;
        sys::create_dir(&self.handle, name)
    }

    /// Removes the file `name`, or the link `name` itself, never what it points to. A folder is
    /// refused.
    pub fn remove_file(&self, name: &OsStr) -> io::Result<()> {
        one_name(name)?;
        sys::remove_file(&self.handle, name)
    }

    /// Removes the folder `name` if it is empty.
    pub fn remove_dir(&self, name: &OsStr) -> io::Result<()> {
        one_name(name)?;
        sys::remove_dir(&self.handle, name)
    }

    /// Makes `name` a link to `target`, unless something already has that name. Windows needs to
    /// know whether it is a folder link (`to_dir`), and Developer Mode or an administrator to make
    /// one at all: without them the system's error comes back. Windows makes links only by path,
    /// so there the link is made in `scratch` (`.bolt/tmp`, on the same drive) under a name from
    /// `crate::temp_name`, and moved to `name` once `scratch` shows it holds `target`; an absolute
    /// target must therefore be written as Windows stores it (`C:\x`, no `.`, `..` or `/`). If
    /// Bolt stops before the move, or the link can't be opened or removed again, it stays in
    /// `scratch`, never at `name`. Unix makes the link at `name` directly and leaves `scratch`
    /// alone.
    pub fn symlink(
        &self,
        name: &OsStr,
        target: &OsStr,
        to_dir: bool,
        scratch: &Dir,
    ) -> io::Result<()> {
        one_name(name)?;
        sys::symlink(&self.handle, name, target, to_dir, &scratch.handle)
    }

    /// The target of the link `name`. On Windows an absolute target comes back as Rust's standard
    /// library shows it, `C:\x` rather than the stored `\??\C:\x`.
    pub fn read_link(&self, name: &OsStr) -> io::Result<OsString> {
        one_name(name)?;
        sys::read_link(&self.handle, name)
    }

    /// Sets the regular file `name`'s mode to exactly 0755, or 0644 if not `executable`. Windows
    /// has no executable bit: there this does nothing.
    pub fn set_executable(&self, name: &OsStr, executable: bool) -> io::Result<()> {
        one_name(name)?;
        sys::set_executable(&self.handle, name, executable)
    }

    /// Flushes this folder's entries to disk, so what was made, renamed or removed in it survives
    /// a crash. Windows can't flush a folder opened only to read it: there this does nothing yet.
    pub fn sync(&self) -> io::Result<()> {
        sys::sync(&self.handle)
    }
}

#[cfg(feature = "test-support")]
impl Drop for Dir {
    fn drop(&mut self) {
        open_count::closed();
    }
}

/// How many folders this thread holds open, for tests that bound it.
#[cfg(feature = "test-support")]
pub mod open_count {
    use std::cell::Cell;

    thread_local! {
        static NOW: Cell<usize> = const { Cell::new(0) };
        static MOST: Cell<usize> = const { Cell::new(0) };
    }

    pub(super) fn opened() {
        let now = NOW.get().saturating_add(1);
        NOW.set(now);
        MOST.set(MOST.get().max(now));
    }

    pub(super) fn closed() {
        NOW.set(NOW.get().saturating_sub(1));
    }

    /// How many folders this thread holds open now.
    pub fn now() -> usize {
        NOW.get()
    }

    /// The most folders this thread has held open at once since the last call, which starts the
    /// count again from those open now.
    pub fn most_since_last() -> usize {
        MOST.replace(NOW.get())
    }
}

/// Whether `e` is the refusal of a link or other reparse point at a name.
pub fn is_link_refusal(e: &io::Error) -> bool {
    e.get_ref().is_some_and(|inner| inner.is::<LinkRefused>())
}

#[derive(Debug)]
struct LinkRefused;

impl fmt::Display for LinkRefused {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("it is a link, junction or other reparse point, which Bolt never follows")
    }
}

impl std::error::Error for LinkRefused {}

fn link_refused() -> io::Error {
    io::Error::other(LinkRefused)
}

fn is_a_folder() -> io::Error {
    io::Error::new(io::ErrorKind::IsADirectory, "it is a folder")
}

/// Refuses anything but one entry's name: empty, `.`, `..`, or holding `/` or NUL; on Windows
/// also `\`, or `:`, which would name a stream of a file.
fn one_name(name: &OsStr) -> io::Result<()> {
    let separator =
        |b: &u8| matches!(b, b'/' | b'\0') || (cfg!(windows) && matches!(b, b'\\' | b':'));
    let bytes = name.as_encoded_bytes();
    if matches!(bytes, b"" | b"." | b"..") || bytes.iter().any(separator) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "not the name of one entry in a folder",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic)]
    use super::*;
    use std::ffi::{OsStr, OsString};
    use std::fs::{self, File};
    use std::io::{self, Read, Write};
    use std::path::Path;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn n(name: &str) -> &OsStr {
        OsStr::new(name)
    }

    /// A new temporary folder, held open.
    fn held() -> (tempfile::TempDir, Dir) {
        let tmp = tempfile::tempdir().unwrap();
        let dir = Dir::open_top(tmp.path()).unwrap();
        (tmp, dir)
    }

    /// Makes `link` a link to the folder `target`: a symlink on Unix, a junction on Windows (which
    /// needs no special right).
    fn folder_link(link: &Path, target: &Path) {
        #[cfg(unix)]
        std::os::unix::fs::symlink(target, link).unwrap();
        #[cfg(windows)]
        {
            let made = std::process::Command::new("cmd")
                .args(["/D", "/C", "mklink", "/J"])
                .arg(link)
                .arg(target)
                .stdout(std::process::Stdio::null())
                .status()
                .unwrap();
            assert!(made.success(), "mklink /J {}", link.display());
        }
    }

    /// Makes `link` a symlink to `target`, or says why it can't: on Windows that takes Developer
    /// Mode or an administrator.
    fn symlink(link: &Path, target: &Path, to_dir: bool) -> bool {
        #[cfg(unix)]
        {
            let _ = to_dir;
            std::os::unix::fs::symlink(target, link).unwrap();
            true
        }
        #[cfg(windows)]
        {
            let made = if to_dir {
                std::os::windows::fs::symlink_dir(target, link)
            } else {
                std::os::windows::fs::symlink_file(target, link)
            };
            match made {
                Ok(()) => true,
                Err(e) => {
                    skip_without_link_right(e);
                    false
                }
            }
        }
    }

    /// Says a test can't make links here, which on Windows takes Developer Mode or an
    /// administrator, and lets it go on without them. Any other error fails the test, and so does
    /// this one in CI (where `CI` is set): its runner has the right, and a skip there would pass
    /// for a test that never ran.
    #[cfg(windows)]
    fn skip_without_link_right(e: io::Error) {
        if e.raw_os_error() != Some(1314) {
            panic!("couldn't make a link: {e}");
        }
        if std::env::var_os("CI").is_some() {
            panic!("CI must be able to make links, so that the link tests run there: {e}");
        }
        println!("Skipped: this account can't make links ({e}).");
    }

    fn gone(path: &Path) -> bool {
        fs::symlink_metadata(path).is_err()
    }

    #[test]
    fn a_link_at_a_name_is_refused_at_every_call() {
        let (tmp, d) = held();
        let real = tmp.path().join("real");
        fs::create_dir(&real).unwrap();
        fs::write(real.join("x"), "x").unwrap();
        fs::write(tmp.path().join("file"), "file").unwrap();
        let mut links = Vec::new();
        if symlink(&tmp.path().join("l"), &real, true) {
            links.push("l");
        }
        if symlink(&tmp.path().join("f"), &real.join("x"), false) {
            links.push("f");
        }
        #[cfg(windows)]
        {
            folder_link(&tmp.path().join("j"), &real);
            links.push("j");
        }
        if links.contains(&"l") && links.contains(&"f") {
            for e in [d.open_dir(n("l")).err(), d.open_file(n("f")).err()] {
                assert!(is_link_refusal(&e.unwrap()));
            }
            assert_eq!(d.metadata(n("l")).unwrap().kind, EntryType::Symlink);
        }
        for name in links {
            let refused = |e: io::Error| assert!(is_link_refusal(&e), "{name}: {e}");
            refused(d.open_dir(n(name)).unwrap_err());
            refused(d.open_file(n(name)).unwrap_err());
            refused(d.open_or_create(n(name)).unwrap_err());
            refused(d.remove_dir(n(name)).unwrap_err());
            refused(d.rename_into(n(name), &d, n("moved")).unwrap_err());
            refused(d.rename_into(n("file"), &d, n(name)).unwrap_err());
            refused(Dir::open_top(&tmp.path().join(name)).unwrap_err());
            #[cfg(unix)]
            refused(d.set_executable(n(name), true).unwrap_err());
            assert_eq!(
                d.metadata(n(name)).unwrap().kind,
                EntryType::Symlink,
                "{name}"
            );
        }
        assert!(!is_link_refusal(&d.open_dir(n("file")).unwrap_err()));
        // Nothing moved or changed, through the links or beside them.
        assert_eq!(fs::read_to_string(tmp.path().join("file")).unwrap(), "file");
        assert_eq!(fs::read_to_string(real.join("x")).unwrap(), "x");
        assert!(gone(&tmp.path().join("moved")));
    }

    #[test]
    fn a_folder_swapped_for_a_link_after_opening_stays_the_folder() {
        let (tmp, d) = held();
        let elsewhere = tempfile::tempdir().unwrap();
        fs::create_dir(tmp.path().join("a")).unwrap();
        let a = d.open_dir(n("a")).unwrap();
        fs::rename(tmp.path().join("a"), tmp.path().join("old")).unwrap();
        folder_link(&tmp.path().join("a"), elsewhere.path());

        a.create_new(n("x"), false)
            .unwrap()
            .write_all(b"x")
            .unwrap();
        a.create_dir(n("sub")).unwrap();
        assert_eq!(fs::read_to_string(tmp.path().join("old/x")).unwrap(), "x");
        assert!(tmp.path().join("old/sub").is_dir());
        assert_eq!(fs::read_dir(elsewhere.path()).unwrap().count(), 0);
        // Reaching `a` again from the top meets the link, and refuses it.
        assert!(is_link_refusal(&d.open_dir(n("a")).unwrap_err()));
    }

    #[test]
    fn rename_into_replaces_a_file_atomically() {
        let (tmp, d) = held();
        fs::create_dir(tmp.path().join("to")).unwrap();
        let to = d.open_dir(n("to")).unwrap();
        fs::write(tmp.path().join("to/name"), "old").unwrap();
        let mut new = d.create_new(n("new"), false).unwrap();
        new.write_all(b"new").unwrap();
        new.sync_all().unwrap();
        drop(new);

        // Whoever has the old file open keeps reading it; the name holds the new file at once.
        let mut reader = File::open(tmp.path().join("to/name")).unwrap();
        d.rename_into(n("new"), &to, n("name")).unwrap();
        assert_eq!(
            fs::read_to_string(tmp.path().join("to/name")).unwrap(),
            "new"
        );
        assert!(gone(&tmp.path().join("new")));
        let mut old = String::new();
        reader.read_to_string(&mut old).unwrap();
        assert_eq!(old, "old");

        // A name that is free is filled; a folder is never replaced.
        fs::write(tmp.path().join("more"), "more").unwrap();
        d.rename_into(n("more"), &to, n("free")).unwrap();
        assert_eq!(
            fs::read_to_string(tmp.path().join("to/free")).unwrap(),
            "more"
        );
        fs::create_dir(tmp.path().join("to/folder")).unwrap();
        fs::write(tmp.path().join("last"), "last").unwrap();
        let e = d.rename_into(n("last"), &to, n("folder")).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::IsADirectory, "{e}");
        assert!(tmp.path().join("to/folder").is_dir());
        assert_eq!(fs::read_to_string(tmp.path().join("last")).unwrap(), "last");
    }

    #[test]
    fn names_with_separators_or_dots_are_refused_before_any_call() {
        let (tmp, d) = held();
        fs::create_dir(tmp.path().join("a")).unwrap();
        fs::write(tmp.path().join("ok"), "ok").unwrap();
        for name in ["a/b", ".", "..", ""] {
            assert_eq!(
                d.open_dir(name.as_ref()).unwrap_err().kind(),
                io::ErrorKind::InvalidInput
            );
        }
        #[cfg(windows)]
        assert_eq!(
            d.open_dir("a\\b".as_ref()).unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );

        let mut bad = vec!["a/b", ".", "..", "", "a\0b"];
        // On Windows `\` separates names too, and `a:b` would name a stream of `a`.
        if cfg!(windows) {
            bad.extend(["a\\b", "a:b"]);
        }
        for name in bad {
            let name = n(name);
            let refused = |r: io::Result<()>| {
                let e = r.unwrap_err();
                assert_eq!(e.kind(), io::ErrorKind::InvalidInput, "{name:?}: {e}");
            };
            refused(d.open_dir(name).map(drop));
            refused(d.open_file(name).map(drop));
            refused(d.metadata(name).map(drop));
            refused(d.create_new(name, false).map(drop));
            refused(d.open_or_create(name).map(drop));
            refused(d.create_dir(name));
            refused(d.remove_file(name));
            refused(d.remove_dir(name));
            refused(d.symlink(name, n("ok"), false, &d));
            refused(d.read_link(name).map(drop));
            refused(d.set_executable(name, true));
            refused(d.rename_into(name, &d, n("ok2")));
            refused(d.rename_into(n("ok"), &d, name));
        }
        assert!(gone(&tmp.path().join("a/b")));
        assert_eq!(fs::read_to_string(tmp.path().join("ok")).unwrap(), "ok");
        assert_eq!(fs::read_dir(tmp.path()).unwrap().count(), 2);
    }

    #[test]
    fn entries_and_metadata_describe_what_is_there() {
        let (tmp, d) = held();
        d.create_new(n("file"), false)
            .unwrap()
            .write_all(b"12345")
            .unwrap();
        d.create_new(n("tool"), true).unwrap();
        d.create_dir(n("folder")).unwrap();
        #[cfg(unix)]
        d.symlink(n("link"), n("folder"), true, &d).unwrap();
        #[cfg(windows)]
        folder_link(&tmp.path().join("link"), &tmp.path().join("folder"));

        let mut entries = d.entries().unwrap();
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        let kinds: Vec<_> = entries
            .iter()
            .map(|e| (e.name.to_str().unwrap(), e.kind))
            .collect();
        assert_eq!(
            kinds,
            [
                ("file", EntryType::File),
                ("folder", EntryType::Dir),
                ("link", EntryType::Symlink),
                ("tool", EntryType::File)
            ]
        );
        #[cfg(unix)]
        assert!(entries.iter().all(|e| e.short_name.is_none()));
        // A link to a folder is marked as one only on Windows, where it is a junction here.
        let folders: Vec<_> = entries
            .iter()
            .filter(|e| e.folder)
            .map(|e| &e.name)
            .collect();
        let marked: &[&str] = if cfg!(windows) {
            &["folder", "link"]
        } else {
            &["folder"]
        };
        assert_eq!(folders, marked);

        let file = d.metadata(n("file")).unwrap();
        assert_eq!((file.kind, file.size), (EntryType::File, 5));
        assert_eq!(d.metadata(n("folder")).unwrap().kind, EntryType::Dir);
        assert_eq!(d.metadata(n("link")).unwrap().kind, EntryType::Symlink);
        assert_ne!(file.file_id, d.metadata(n("tool")).unwrap().file_id);
        // The time is the one std reads, and it is now.
        let since_1970 = |t: SystemTime| t.duration_since(UNIX_EPOCH).unwrap().as_nanos() as i128;
        let std_time = fs::metadata(tmp.path().join("file"))
            .unwrap()
            .modified()
            .unwrap();
        assert_eq!(file.modified_ns, since_1970(std_time));
        assert!((since_1970(SystemTime::now()) - file.modified_ns).abs() < 600 * 1_000_000_000);
        #[cfg(unix)]
        {
            assert_eq!(file.executable, Some(false));
            assert_eq!(d.metadata(n("tool")).unwrap().executable, Some(true));
            d.set_executable(n("file"), true).unwrap();
            assert_eq!(d.metadata(n("file")).unwrap().executable, Some(true));
            d.set_executable(n("file"), false).unwrap();
        }
        #[cfg(windows)]
        {
            assert_eq!(file.executable, None);
            d.set_executable(n("file"), true).unwrap();
        }

        // An open file is the entry the name showed.
        let mut opened = d.open_file(n("file")).unwrap();
        assert_eq!(
            Dir::file_meta(&opened).unwrap(),
            d.metadata(n("file")).unwrap()
        );
        let mut text = String::new();
        opened.read_to_string(&mut text).unwrap();
        assert_eq!(text, "12345");
        drop(opened);
        assert_eq!(
            d.read_link(n("link")).unwrap(),
            fs::read_link(tmp.path().join("link"))
                .unwrap()
                .into_os_string()
        );
        let e = d.open_file(n("folder")).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::IsADirectory, "{e}");
        let e = d.open_dir(n("file")).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::NotADirectory, "{e}");
        let e = d.create_new(n("file"), false).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::AlreadyExists, "{e}");
        d.sync().unwrap();
        d.open_dir(n("folder")).unwrap().sync().unwrap();

        // Removing a link leaves what it points to; a folder goes only once empty.
        d.create_new(n("inside"), false).unwrap();
        d.rename_into(n("inside"), &d.open_dir(n("folder")).unwrap(), n("inside"))
            .unwrap();
        d.remove_file(n("link")).unwrap();
        assert!(gone(&tmp.path().join("link")));
        assert!(tmp.path().join("folder/inside").is_file());
        assert!(d.remove_file(n("folder")).is_err());
        let e = d.remove_dir(n("folder")).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::DirectoryNotEmpty, "{e}");
        d.open_dir(n("folder"))
            .unwrap()
            .remove_file(n("inside"))
            .unwrap();
        d.remove_dir(n("folder")).unwrap();
        assert!(d.remove_dir(n("file")).is_err());
        d.remove_file(n("file")).unwrap();
        let left: Vec<OsString> = d.entries().unwrap().into_iter().map(|e| e.name).collect();
        assert_eq!(left, [OsString::from("tool")]);
    }

    /// What the change lock needs: a file made once and opened again after, to read, write and
    /// lock, never emptied.
    #[test]
    fn open_or_create_makes_a_file_or_opens_one_and_keeps_what_is_in_it() {
        let (_tmp, d) = held();
        let mut made = d.open_or_create(n("lock")).unwrap();
        made.write_all(b"held").unwrap();
        drop(made);
        let mut again = d.open_or_create(n("lock")).unwrap();
        let mut text = String::new();
        again.read_to_string(&mut text).unwrap();
        assert_eq!(text, "held");
        again.write_all(b"!").unwrap();
        again.try_lock().unwrap();
        d.create_dir(n("folder")).unwrap();
        let e = d.open_or_create(n("folder")).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::IsADirectory, "{e}");
    }

    #[cfg(unix)]
    #[test]
    fn new_files_get_exact_modes() {
        use rustix::fs::Mode;
        use std::os::unix::fs::PermissionsExt;

        /// Puts the old umask back however the test ends.
        struct Restore(Mode);
        impl Drop for Restore {
            fn drop(&mut self) {
                rustix::process::umask(self.0);
            }
        }

        for mask in [0o077, 0o000] {
            let (tmp, d) = held();
            let restore = Restore(rustix::process::umask(Mode::from_raw_mode(mask)));
            d.create_new(n("plain"), false).unwrap();
            d.create_new(n("tool"), true).unwrap();
            drop(restore);
            let mode = |name: &str| {
                fs::metadata(tmp.path().join(name))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o7777
            };
            assert_eq!(
                (mode("plain"), mode("tool")),
                (0o644, 0o755),
                "umask {mask:o}"
            );
            d.set_executable(n("plain"), true).unwrap();
            d.set_executable(n("tool"), false).unwrap();
            assert_eq!(
                (mode("plain"), mode("tool")),
                (0o755, 0o644),
                "umask {mask:o}"
            );
        }
    }

    /// The short name Windows lists for `.git` is the one its own `cmd` shows, or none on a drive
    /// that makes no short names.
    #[cfg(windows)]
    #[test]
    fn short_names_come_with_entries() {
        let (tmp, d) = held();
        d.create_dir(n(".git")).unwrap();
        d.create_new(n("short.txt"), false).unwrap();
        let out = std::process::Command::new("cmd")
            .current_dir(tmp.path())
            .args(["/D", "/C", "for %I in (.git) do @echo %~snxI"])
            .output()
            .unwrap();
        assert!(out.status.success());
        let shown = String::from_utf8(out.stdout).unwrap().trim().to_owned();
        let entries = d.entries().unwrap();
        let git = entries.iter().find(|e| e.name == ".git").unwrap();
        if shown == ".git" {
            println!("This drive makes no short names.");
            assert_eq!(git.short_name, None);
        } else {
            assert_eq!(git.short_name, Some(OsString::from(&shown)));
            assert!(shown.contains('~'), "{shown}");
        }
        // A name that is already short has no other.
        let short = entries.iter().find(|e| e.name == "short.txt").unwrap();
        assert_eq!(short.short_name, None);
    }

    #[cfg(windows)]
    #[test]
    fn links_are_made_for_folders_and_files() {
        let (tmp, d) = held();
        let (scratch_tmp, scratch) = held();
        d.create_dir(n("real")).unwrap();
        fs::write(tmp.path().join("real/x"), "x").unwrap();
        if let Err(e) = d.symlink(n("dl"), n("real"), true, &scratch) {
            skip_without_link_right(e);
            return;
        }
        d.symlink(n("fl"), n("real\\x"), false, &scratch).unwrap();
        let absolute = tmp.path().join("real");
        d.symlink(n("abs"), absolute.as_os_str(), true, &scratch)
            .unwrap();

        for name in ["dl", "fl", "abs"] {
            assert_eq!(
                d.metadata(n(name)).unwrap().kind,
                EntryType::Symlink,
                "{name}"
            );
            assert_eq!(
                d.read_link(n(name)).unwrap(),
                fs::read_link(tmp.path().join(name))
                    .unwrap()
                    .into_os_string(),
                "{name}"
            );
        }
        assert_eq!(d.read_link(n("dl")).unwrap(), "real");
        assert_eq!(d.read_link(n("fl")).unwrap(), "real\\x");
        // Real links, of the right kind: Windows follows them.
        use std::os::windows::fs::FileTypeExt;
        let kind = |name: &str| {
            fs::symlink_metadata(tmp.path().join(name))
                .unwrap()
                .file_type()
        };
        assert!(kind("dl").is_symlink_dir() && kind("abs").is_symlink_dir());
        assert!(kind("fl").is_symlink_file());
        assert_eq!(fs::read_to_string(tmp.path().join("dl/x")).unwrap(), "x");
        assert_eq!(fs::read_to_string(tmp.path().join("fl")).unwrap(), "x");
        // A name already taken is never replaced.
        let e = d.symlink(n("fl"), n("real"), false, &scratch).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::AlreadyExists, "{e}");
        assert_eq!(d.read_link(n("fl")).unwrap(), "real\\x");
        // The links were moved out of the scratch folder, and nothing else came in.
        let mut names: Vec<_> = d.entries().unwrap().into_iter().map(|e| e.name).collect();
        names.sort();
        assert_eq!(names, ["abs", "dl", "fl", "real"]);
        assert_eq!(fs::read_dir(scratch_tmp.path()).unwrap().count(), 0);
    }

    /// Windows makes a link by the scratch folder's path. A scratch folder swapped for a junction
    /// at that moment must not put a link at the name asked for, in the folder or anywhere else.
    #[cfg(windows)]
    #[test]
    fn a_link_raced_out_of_its_folder_lands_under_no_chosen_name() {
        let (tmp, d) = held();
        let elsewhere = tempfile::tempdir().unwrap();
        d.create_dir(n("tmp")).unwrap();
        let scratch = d.open_dir(n("tmp")).unwrap();
        let (top, outside) = (tmp.path().to_owned(), elsewhere.path().to_owned());
        windows::BEFORE_LINK.set(Some(Box::new(move || {
            fs::rename(top.join("tmp"), top.join("old")).unwrap();
            folder_link(&top.join("tmp"), &outside);
        })));
        let made = d.symlink(n("pre-commit"), n("target"), false, &scratch);
        if let Err(e) = made {
            if e.raw_os_error() == Some(1314) {
                skip_without_link_right(e);
                return;
            }
        } else {
            panic!("the link was placed");
        }
        assert!(gone(&tmp.path().join("pre-commit")));
        assert_eq!(fs::read_dir(tmp.path().join("old")).unwrap().count(), 0);
        let stray: Vec<_> = fs::read_dir(elsewhere.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        assert_eq!(stray.len(), 1, "{stray:?}");
        assert!(crate::is_temp_name(OsStr::new(&stray[0])), "{stray:?}");
    }

    /// As on Unix, an entry's own read-only flag doesn't stop Bolt removing or replacing it.
    #[test]
    fn a_read_only_file_can_be_removed_or_replaced() {
        let (tmp, d) = held();
        let read_only = |path: &Path| {
            let mut permissions = fs::metadata(path).unwrap().permissions();
            permissions.set_readonly(true);
            fs::set_permissions(path, permissions).unwrap();
        };
        fs::write(tmp.path().join("removed"), "removed").unwrap();
        read_only(&tmp.path().join("removed"));
        d.remove_file(n("removed")).unwrap();
        assert!(gone(&tmp.path().join("removed")));
        fs::write(tmp.path().join("replaced"), "replaced").unwrap();
        read_only(&tmp.path().join("replaced"));
        fs::write(tmp.path().join("new"), "new").unwrap();
        d.rename_into(n("new"), &d, n("replaced")).unwrap();
        assert_eq!(
            fs::read_to_string(tmp.path().join("replaced")).unwrap(),
            "new"
        );
        // An empty read-only folder goes too.
        fs::create_dir(tmp.path().join("folder")).unwrap();
        read_only(&tmp.path().join("folder"));
        d.remove_dir(n("folder")).unwrap();
        assert!(gone(&tmp.path().join("folder")));
    }

    /// Making a file, folder or link at a name that holds a link fails, even when the link points
    /// nowhere, and nothing appears where it points.
    #[test]
    fn nothing_is_made_through_a_dangling_link() {
        let (tmp, d) = held();
        let (scratch_tmp, scratch) = held();
        let missing = tmp.path().join("missing");
        fs::create_dir(&missing).unwrap();
        folder_link(&tmp.path().join("link"), &missing);
        fs::remove_dir(&missing).unwrap();
        let target = d.read_link(n("link")).unwrap();
        for e in [
            d.create_new(n("link"), false).map(drop).unwrap_err(),
            d.create_dir(n("link")).unwrap_err(),
            d.symlink(n("link"), n("elsewhere"), false, &scratch)
                .unwrap_err(),
        ] {
            assert_eq!(e.kind(), io::ErrorKind::AlreadyExists, "{e}");
        }
        assert!(is_link_refusal(&d.open_or_create(n("link")).unwrap_err()));
        assert!(gone(&missing));
        assert_eq!(d.metadata(n("link")).unwrap().kind, EntryType::Symlink);
        assert_eq!(d.read_link(n("link")).unwrap(), target);
        assert_eq!(fs::read_dir(scratch_tmp.path()).unwrap().count(), 0);
    }
}
