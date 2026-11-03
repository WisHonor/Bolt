//! The single place every disk write goes through: temp files, atomic replacement, durable
//! renames, and the fault points that crash tests use.

use std::ffi::OsStr;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::safe_fs;

/// Creates a new, empty file named `tmp-<16 random hex>` in `tmp_dir`.
pub fn create_temp(tmp_dir: &Path) -> io::Result<(File, PathBuf)> {
    let mut random = [0u8; 8];
    getrandom::fill(&mut random).map_err(io::Error::other)?;
    let name: String = random.iter().map(|b| format!("{b:02x}")).collect();
    let path = tmp_dir.join(format!("tmp-{name}"));
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)?;
    Ok((file, path))
}

/// What `drive::probe` adds to a temp name: `-e` and a separate accent, which a drive may not tell
/// from `-é`.
pub(crate) const ACCENTED_SUFFIX: &str = "-e\u{301}";

/// Whether `name` is one Bolt makes: `tmp-` and 16 lowercase hex digits, as `create_temp` names a
/// file, and then `ACCENTED_SUFFIX` as well when `drive::probe` has renamed it.
pub fn is_temp_name(name: &OsStr) -> bool {
    name.to_str()
        .and_then(|n| n.strip_prefix("tmp-"))
        .map(|rest| rest.strip_suffix(ACCENTED_SUFFIX).unwrap_or(rest))
        .is_some_and(|hex| {
            hex.len() == 16 && hex.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
        })
}

/// Replaces `dest` with `bytes` so that, even after a crash, it holds either the old or the
/// new bytes, never a mix.
pub fn write_atomic(tmp_dir: &Path, dest: &Path, bytes: &[u8]) -> io::Result<()> {
    let (mut file, tmp) = create_temp(tmp_dir)?;
    let result = (|| {
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        fault_point("write:before-rename")?;
        rename_durable(&tmp, dest)
    })();
    if result.is_err() {
        // The temp file is only litter now; failing to remove it changes nothing.
        let _ = fs::remove_file(&tmp);
    }
    result
}

/// Renames `from` to `to`, then flushes the parent folder so the rename survives a crash.
pub fn rename_durable(from: &Path, to: &Path) -> io::Result<()> {
    let mut retries = 0;
    loop {
        match fs::rename(from, to) {
            Ok(()) => break,
            // Virus scanners and indexers briefly hold files open on Windows.
            Err(e) if cfg!(windows) && retries < 10 && matches!(e.raw_os_error(), Some(5 | 32)) => {
                retries += 1;
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => return Err(e),
        }
    }
    sync_parent(to)
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

/// Deletes a file. Not flushed: only used for leftovers, which may as well come back.
pub fn remove_file(path: &Path) -> io::Result<()> {
    fs::remove_file(path)
}

/// Opens the change-lock file for reading and writing, creating it if needed, never through a
/// link (`safe_fs::open_regular`). Its contents never matter, so it is never truncated.
pub fn open_lock_file(path: &Path) -> io::Result<File> {
    safe_fs::open_regular(
        path,
        OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false),
    )
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

/// Fault injection for crash tests: per-thread errors (`arm`), a process abort
/// (`BOLT_FAULT_AT=N`), or an error at one named point (`BOLT_FAULT_FAIL=label`).
#[cfg(feature = "fault-injection")]
pub mod fault {
    use std::cell::Cell;
    use std::io;
    use std::sync::OnceLock;
    use std::sync::atomic::{AtomicU64, Ordering};

    thread_local! {
        static ARMED: Cell<Option<u64>> = const { Cell::new(None) };
        static COUNT: Cell<u64> = const { Cell::new(0) };
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

    pub(super) fn hit(label: &'static str) -> io::Result<()> {
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

    #[test]
    fn write_atomic_replaces_the_file_whole() {
        let root = tempfile::tempdir().unwrap();
        let dest = root.path().join("head");
        write_atomic(root.path(), &dest, b"old").unwrap();
        write_atomic(root.path(), &dest, b"new").unwrap();
        assert_eq!(fs::read(&dest).unwrap(), b"new");
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
    }

    #[test]
    fn temp_names_are_the_ones_bolt_makes() {
        let root = tempfile::tempdir().unwrap();
        let (_, made) = create_temp(root.path()).unwrap();
        assert!(is_temp_name(made.file_name().unwrap()));
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
        let dest = root.path().join("head");
        write_atomic(root.path(), &dest, b"old").unwrap();
        fault::arm(1);
        let err = write_atomic(root.path(), &dest, b"new").unwrap_err();
        fault::disarm();
        assert_eq!(err.to_string(), "injected fault at write:before-rename");
        assert_eq!(fs::read(&dest).unwrap(), b"old");
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
    }
}
