//! The single place every disk write goes through: temp files, atomic replacement, durable
//! renames, and the fault points that crash tests use. Writes in `.bolt` go through its folders
//! held open, never by path.

use std::ffi::{OsStr, OsString};
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::Path;
use std::time::Duration;

use bolt_platform::dir::Dir;

// One rule for temporary names, shared with the links the platform crate makes in `.bolt/tmp`.
pub(crate) use bolt_platform::ACCENTED_SUFFIX;
pub use bolt_platform::is_temp_name;

use crate::safe_fs;

/// Creates a new, empty file in the folder `tmp`, named as `bolt_platform::temp_name` says:
/// `tmp-` and 16 random lowercase hex digits. Returns it and its name.
pub fn create_temp(tmp: &Dir) -> io::Result<(File, OsString)> {
    let name = bolt_platform::temp_name();
    let file = tmp.create_new(&name, false)?;
    Ok((file, name))
}

/// Replaces `name` in the folder `to` with `bytes`, written in the folder `tmp` first, so that,
/// even after a crash, it holds either the old or the new bytes, never a mix.
pub fn write_atomic(tmp: &Dir, to: &Dir, name: &str, bytes: &[u8]) -> io::Result<()> {
    let (mut file, temp) = create_temp(tmp)?;
    let result = (|| {
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        fault_point("write:before-rename")?;
        rename_durable(tmp, &temp, to, OsStr::new(name))
    })();
    if result.is_err() {
        // The temp file is only litter now; failing to remove it changes nothing.
        let _ = remove_file(tmp, &temp);
    }
    result
}

/// Moves `from` in the folder `from_dir` to `to` in the folder `to_dir`, replacing a file there,
/// then flushes `to_dir` so the move survives a crash.
pub fn rename_durable(from_dir: &Dir, from: &OsStr, to_dir: &Dir, to: &OsStr) -> io::Result<()> {
    let mut retries = 0;
    loop {
        match from_dir.rename_into(from, to_dir, to) {
            Ok(()) => break,
            // Virus scanners and indexers briefly hold files open on Windows.
            Err(e) if cfg!(windows) && retries < 10 && matches!(e.raw_os_error(), Some(5 | 32)) => {
                retries += 1;
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => return Err(e),
        }
    }
    to_dir.sync()
}

/// Flushes the folder that holds `path`.
fn sync_parent(path: &Path) -> io::Result<()> {
    match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => sync_dir(p),
        _ => sync_dir(Path::new(".")),
    }
}

/// Creates the folder `dir` in its existing parent and flushes the parent so it survives a crash.
/// Whatever is already at `dir`, a link included, is left alone for the caller to check.
pub fn create_dir_durable(dir: &Path) -> io::Result<()> {
    match fs::create_dir(dir) {
        Ok(()) => sync_parent(dir),
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => Ok(()),
        Err(e) => Err(e),
    }
}

/// Deletes the file `name` in the folder `dir`, or the link itself if it is one. Not flushed: only
/// used for leftovers, which may as well come back.
pub fn remove_file(dir: &Dir, name: &OsStr) -> io::Result<()> {
    dir.remove_file(name)
}

/// Opens the change-lock file `name` in the folder `state` for reading and writing, making it if
/// needed, never through a link (`safe_fs::open_or_create_in`); errors name it `shown`. Its
/// contents never matter, so it is never truncated.
pub fn open_lock_file(state: &Dir, name: &OsStr, shown: &Path) -> io::Result<File> {
    safe_fs::open_or_create_in(state, name, shown)
}

/// Flushes a folder's entries to disk. Windows has no such call: a rename there keeps its order
/// with earlier writes but can be lost on power failure.
pub fn sync_dir(dir: &Path) -> io::Result<()> {
    if cfg!(unix) {
        File::open(dir)?.sync_all()
    } else {
        // shortcut: renames are not durable on Windows; make them so in milestone 4.
        let _ = dir;
        Ok(())
    }
}

/// A place where crash tests can inject a failure. Does nothing without `fault-injection`.
pub fn fault_point(label: &'static str) -> io::Result<()> {
    #[cfg(feature = "fault-injection")]
    {
        fault::hit(label)
    }
    #[cfg(not(feature = "fault-injection"))]
    {
        let _ = label;
        Ok(())
    }
}

/// A place where a test can change the disk (`fault::act_at`); never a crash point, since nothing
/// is written there.
pub fn act_point(label: &'static str) {
    #[cfg(feature = "fault-injection")]
    fault::act(label);
    #[cfg(not(feature = "fault-injection"))]
    let _ = label;
}

/// Fault injection for crash tests: per-thread errors (`arm`), a process abort
/// (`BOLT_FAULT_AT=N`), an error at one named point (`BOLT_FAULT_FAIL=label`), or a test's own
/// change to the disk at one named point (`act_at`).
#[cfg(feature = "fault-injection")]
pub mod fault {
    use std::cell::{Cell, RefCell};
    use std::io;
    use std::sync::OnceLock;
    use std::sync::atomic::{AtomicU64, Ordering};

    /// What a test does when this thread reaches a named point.
    type Action = (&'static str, Box<dyn FnOnce()>);

    thread_local! {
        static ARMED: Cell<Option<u64>> = const { Cell::new(None) };
        static COUNT: Cell<u64> = const { Cell::new(0) };
        static ACTION: RefCell<Option<Action>> = const { RefCell::new(None) };
    }

    /// Runs `action` once, when this thread next reaches the point `label`, so a test can change
    /// what Bolt finds there.
    pub fn act_at(label: &'static str, action: impl FnOnce() + 'static) {
        ACTION.set(Some((label, Box::new(action))));
    }

    static PROCESS_COUNT: AtomicU64 = AtomicU64::new(0);

    /// Makes the `n`th fault point this thread reaches from now on return an error.
    pub fn arm(n: u64) {
        ARMED.set(Some(n));
        COUNT.set(0);
    }

    pub fn disarm() {
        ARMED.set(None);
    }

    fn crash_at() -> Option<u64> {
        static AT: OnceLock<Option<u64>> = OnceLock::new();
        *AT.get_or_init(|| std::env::var("BOLT_FAULT_AT").ok()?.parse().ok())
    }

    /// The label of the one point that fails every time it is reached, for a test that runs the
    /// command in a process of its own and wants it to carry on after the error.
    fn fails_at() -> Option<&'static str> {
        static AT: OnceLock<Option<String>> = OnceLock::new();
        AT.get_or_init(|| std::env::var("BOLT_FAULT_FAIL").ok())
            .as_deref()
    }

    pub(super) fn act(label: &'static str) {
        let due = ACTION.with_borrow_mut(|action| action.take_if(|(at, _)| *at == label));
        if let Some((_, action)) = due {
            action();
        }
    }

    pub(super) fn hit(label: &'static str) -> io::Result<()> {
        act(label);
        if let Some(n) = ARMED.get() {
            let count = COUNT.get() + 1;
            COUNT.set(count);
            if count == n {
                return Err(io::Error::other(format!("injected fault at {label}")));
            }
        }
        if fails_at() == Some(label) {
            return Err(io::Error::other(format!("injected fault at {label}")));
        }
        if let Some(n) = crash_at()
            && PROCESS_COUNT.fetch_add(1, Ordering::SeqCst) + 1 == n
        {
            eprintln!("injected crash at {label}");
            std::process::abort();
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;
    use std::ffi::OsStr;

    #[test]
    fn write_atomic_replaces_the_file_whole() {
        let root = tempfile::tempdir().unwrap();
        let dir = Dir::open_top(root.path()).unwrap();
        write_atomic(&dir, &dir, "head", b"old").unwrap();
        write_atomic(&dir, &dir, "head", b"new").unwrap();
        assert_eq!(fs::read(root.path().join("head")).unwrap(), b"new");
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
    }

    #[test]
    fn temp_names_are_the_ones_bolt_makes() {
        let root = tempfile::tempdir().unwrap();
        let (_, made) = create_temp(&Dir::open_top(root.path()).unwrap()).unwrap();
        assert!(is_temp_name(&made));
        let hex = "0123456789abcdef";
        let probe_name = format!("tmp-{hex}{ACCENTED_SUFFIX}");
        assert_eq!(probe_name, format!("tmp-{hex}-e\u{301}"));
        assert!(is_temp_name(OsStr::new(&probe_name)));
        for other in [
            format!("tmp-{hex}-\u{e9}"),
            format!("tmp-{hex}-e"),
            format!("tmp-{hex}-e\u{301}x"),
            format!("tmp-{hex}{ACCENTED_SUFFIX}{ACCENTED_SUFFIX}"),
            format!("{hex}{ACCENTED_SUFFIX}"),
            format!("tmp-{}", hex.to_uppercase()),
            format!("tmp-{}", &hex[1..]),
            format!("tmp-{ACCENTED_SUFFIX}"),
            "notes.txt".to_owned(),
        ] {
            assert!(!is_temp_name(OsStr::new(&other)), "{other:?}");
        }
    }

    #[test]
    fn create_dir_durable_never_follows_what_is_already_there() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("a");
        create_dir_durable(&dir).unwrap();
        create_dir_durable(&dir).unwrap();
        assert!(dir.is_dir());
        // A link already there, to nothing: following it would look in its target.
        let (link, target) = (root.path().join("link"), root.path().join("target"));
        #[cfg(unix)]
        std::os::unix::fs::symlink(&target, &link).unwrap();
        #[cfg(windows)]
        crate::test_support::junction(&link, &target);
        create_dir_durable(&link).unwrap();
        assert!(!target.exists());
    }

    #[cfg(feature = "fault-injection")]
    #[test]
    fn fault_before_rename_keeps_the_old_file() {
        let root = tempfile::tempdir().unwrap();
        let dir = Dir::open_top(root.path()).unwrap();
        write_atomic(&dir, &dir, "head", b"old").unwrap();
        fault::arm(1);
        let err = write_atomic(&dir, &dir, "head", b"new").unwrap_err();
        fault::disarm();
        assert_eq!(err.to_string(), "injected fault at write:before-rename");
        assert_eq!(fs::read(root.path().join("head")).unwrap(), b"old");
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
    }
}
