//! The single place every disk write goes through: temp files, atomic replacement, durable
//! renames, and the fault points that crash tests use.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

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
    match to.parent() {
        Some(p) if !p.as_os_str().is_empty() => sync_dir(p),
        _ => sync_dir(Path::new(".")),
    }
}

/// Flushes a folder's entries to disk. Windows has no such call; NTFS journals renames.
pub fn sync_dir(dir: &Path) -> io::Result<()> {
    if cfg!(unix) {
        File::open(dir)?.sync_all()
    } else {
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

/// Fault injection for crash tests: per-thread errors (`arm`) or a process abort
/// (`BOLT_FAULT_AT=N`).
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

    pub(super) fn hit(label: &'static str) -> io::Result<()> {
        if let Some(n) = ARMED.get() {
            let count = COUNT.get() + 1;
            COUNT.set(count);
            if count == n {
                return Err(io::Error::other(format!("injected fault at {label}")));
            }
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
