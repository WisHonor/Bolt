//! The single place every disk write goes through: temp files, atomic replacement, durable
//! renames, and the fault points that crash tests use. Writes in `.opalro` go through its folders
//! held open, never by path.

use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::io::{self, Write};
use std::path::Path;
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::Duration;

use opal_platform::dir::Dir;

// One rule for temporary names, shared with the links the platform crate makes in `.opalro/tmp`.
pub(crate) use opal_platform::ACCENTED_SUFFIX;
pub use opal_platform::is_temp_name;

use crate::safe_fs;

/// A file another program holds is tried this many times, this far apart.
pub(crate) const TRIES: u32 = 10;
pub(crate) const BETWEEN_TRIES: Duration = Duration::from_millis(100);

/// Creates a new, empty file in the folder `tmp`, named as `opal_platform::temp_name` says:
/// `tmp-` and 16 random lowercase hex digits. Returns it and its name.
pub fn create_temp(tmp: &Dir) -> io::Result<(File, OsString)> {
    let name = opal_platform::temp_name();
    let file = tmp.create_new(&name, false)?;
    Ok((file, name))
}

/// Replaces `name` in the folder `to` with `bytes`, written in the folder `tmp` first, so that,
/// even after a crash, it holds either the old or the new bytes, never a mix.
pub fn write_atomic(tmp: &Dir, to: &Dir, name: &str, bytes: &[u8]) -> io::Result<()> {
    replace(tmp, to, name, bytes, true, true, rename_durable)
}

/// `write_atomic` for the head, `name` in the folder `state`: once it is renamed into place the
/// command has happened, so a flush that fails after that is told apart from an error.
pub(crate) fn write_head(tmp: &Dir, state: &Dir, name: &str, bytes: &[u8]) -> io::Result<Done> {
    replace(tmp, state, name, bytes, true, true, rename_reported)
}

/// `write_atomic` without a flush, for a file a power cut may leave older or missing at no cost:
/// still replaced whole, so no reader sees half of it.
pub(crate) fn write_unflushed(tmp: &Dir, to: &Dir, name: &str, bytes: &[u8]) -> io::Result<()> {
    replace(tmp, to, name, bytes, true, false, rename_plain)
}

/// `write_unflushed` for a cache, which is no part of what a command did, so crash tests don't
/// stop there. A cache a crash leaves torn fails its seal and is rebuilt.
pub(crate) fn write_cache(tmp: &Dir, to: &Dir, name: &str, bytes: &[u8]) -> io::Result<()> {
    replace(tmp, to, name, bytes, false, false, rename_plain)
}

fn replace<T>(
    tmp: &Dir,
    to: &Dir,
    name: &str,
    bytes: &[u8],
    faults: bool,
    flush: bool,
    rename: fn(&Dir, &OsStr, &Dir, &OsStr) -> io::Result<T>,
) -> io::Result<T> {
    let (mut file, temp) = create_temp(tmp)?;
    let result = (|| {
        file.write_all(bytes)?;
        if flush {
            flush_file(&file)?;
        }
        drop(file);
        if faults {
            fault_point("write:before-rename")?;
        }
        rename(tmp, &temp, to, OsStr::new(name))
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
    rename(from_dir, from, to_dir, to)?;
    flush_dir(to_dir)
}

/// Moves `from` in the folder `from_dir` to `to` in the folder `to_dir`, replacing a file there,
/// with no flush: for a move a crash may undo at no cost.
pub(crate) fn rename_plain(
    from_dir: &Dir,
    from: &OsStr,
    to_dir: &Dir,
    to: &OsStr,
) -> io::Result<()> {
    rename(from_dir, from, to_dir, to)
}

/// Flushes `file` to the disk.
fn flush_file(file: &File) -> io::Result<()> {
    #[cfg(feature = "fault-injection")]
    fault::flushed();
    file.sync_all()
}

/// Flushes `dir`, so the names changed in it survive a crash.
fn flush_dir(dir: &Dir) -> io::Result<()> {
    #[cfg(feature = "fault-injection")]
    fault::flushed();
    dir.sync()
}

/// How much of a file is written between two of its early flushes.
const EARLY_FLUSH_STEP: u64 = 16 << 20;

/// Flushes a file's data from another thread while it is written, so the last flush has little
/// left to do. The last flush is still the caller's.
pub(crate) struct EarlyFlush {
    /// Bytes written since the helper was last woken.
    since: u64,
    /// Whether a helper was started, or failed to start: one try per file.
    tried: bool,
    helper: Option<Helper>,
}

struct Helper {
    shared: Arc<Shared>,
    thread: JoinHandle<()>,
}

#[derive(Default)]
struct Shared {
    state: Mutex<State>,
    wake: Condvar,
    #[cfg(feature = "fault-injection")]
    fails: bool,
}

#[derive(Default)]
struct State {
    /// A flush is asked for; one at most is ever pending.
    wanted: bool,
    done: bool,
    #[cfg(feature = "fault-injection")]
    made: u64,
    error: Option<io::Error>,
}

impl EarlyFlush {
    pub(crate) fn new() -> EarlyFlush {
        EarlyFlush {
            since: 0,
            tried: false,
            helper: None,
        }
    }

    /// Notes `n` more bytes written to `file`. Past 16 MiB it starts the helper, which flushes
    /// through its own handle (`try_clone`); every further 16 MiB wakes it.
    pub(crate) fn wrote(&mut self, file: &File, n: u64) {
        self.since = self.since.saturating_add(n);
        if self.since < EARLY_FLUSH_STEP {
            return;
        }
        self.since = 0;
        if !self.tried {
            self.tried = true;
            self.helper = Helper::start(file);
        }
        if let Some(helper) = &self.helper
            && let Ok(mut state) = helper.shared.state.lock()
        {
            state.wanted = true;
            helper.shared.wake.notify_one();
        }
    }

    /// Stops the helper and gives the first error one of its flushes met.
    pub(crate) fn finish(&mut self) -> io::Result<()> {
        let Some(Helper { shared, thread }) = self.helper.take() else {
            return Ok(());
        };
        // Told to stop even past a poisoned lock, so the join below can't wait forever.
        shared
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .done = true;
        shared.wake.notify_one();
        let joined = thread.join();
        let stopped = || io::Error::other("the thread flushing a file stopped unexpectedly");
        let mut state = shared.state.lock().map_err(|_| stopped())?;
        joined.map_err(|_| stopped())?;
        #[cfg(feature = "fault-injection")]
        fault::flushed_early(state.made);
        state.error.take().map_or(Ok(()), Err)
    }
}

impl Drop for EarlyFlush {
    fn drop(&mut self) {
        let _ = self.finish();
    }
}

impl Helper {
    /// `None` if the handle can't be cloned or the thread can't be started: the last flush then
    /// does all the work.
    fn start(file: &File) -> Option<Helper> {
        #[cfg(feature = "fault-injection")]
        let fails = match fault::early() {
            fault::Early::Off => return None,
            how => how == fault::Early::Fails,
        };
        let own = file.try_clone().ok()?;
        let shared = Arc::new(Shared {
            #[cfg(feature = "fault-injection")]
            fails,
            ..Shared::default()
        });
        let theirs = Arc::clone(&shared);
        let thread = std::thread::Builder::new()
            .name("opal-flush".into())
            .spawn(move || flush_loop(&own, &theirs))
            .ok()?;
        Some(Helper { shared, thread })
    }
}

/// Flushes `file` each time it is asked to, until told to stop or a flush fails. A flush asked
/// for before the stop is still made.
fn flush_loop(file: &File, shared: &Shared) {
    loop {
        let Ok(mut state) = shared.state.lock() else {
            return;
        };
        while !state.wanted && !state.done {
            state = match shared.wake.wait(state) {
                Ok(state) => state,
                Err(_) => return,
            };
        }
        if !state.wanted {
            return;
        }
        state.wanted = false;
        drop(state);
        #[cfg(feature = "fault-injection")]
        let flushed = if shared.fails {
            Err(io::Error::other("injected fault at flush:early"))
        } else {
            file.sync_data()
        };
        #[cfg(not(feature = "fault-injection"))]
        let flushed = file.sync_data();
        let Ok(mut state) = shared.state.lock() else {
            return;
        };
        #[cfg(feature = "fault-injection")]
        {
            state.made = state.made.saturating_add(1);
        }
        if let Err(e) = flushed {
            state.error = Some(e);
            return;
        }
    }
}

/// A change made in a folder: flushed to the disk after it, or not.
#[derive(Debug)]
pub(crate) enum Done {
    Flushed,
    /// The change happened, but flushing its folder afterwards failed.
    NotFlushed(io::Error),
}

/// `rename_durable` for `state/head`, whose move is the moment a command happens: a flush that
/// fails after it comes back as `NotFlushed`, not as an error.
pub(crate) fn rename_reported(
    from_dir: &Dir,
    from: &OsStr,
    to_dir: &Dir,
    to: &OsStr,
) -> io::Result<Done> {
    rename(from_dir, from, to_dir, to)?;
    Ok(flush_after(to_dir, "head:renamed"))
}

/// Flushes `dir` after a change made in it, past the fault point `label`.
pub(crate) fn flush_after(dir: &Dir, label: &'static str) -> Done {
    match fault_point(label).and_then(|()| flush_dir(dir)) {
        Ok(()) => Done::Flushed,
        Err(e) => Done::NotFlushed(e),
    }
}

fn rename(from_dir: &Dir, from: &OsStr, to_dir: &Dir, to: &OsStr) -> io::Result<()> {
    let mut tries = 1;
    loop {
        match from_dir.rename_into(from, to_dir, to) {
            Ok(()) => return Ok(()),
            Err(e) if held(&e) && tries < TRIES => {
                tries += 1;
                std::thread::sleep(BETWEEN_TRIES);
            }
            Err(e) => return Err(e),
        }
    }
}

/// Whether a write of Opalro's own file failed because another program, such as a virus scanner
/// or indexer, has the file open: refused, in use, or deleted but not yet closed (Windows only).
pub(crate) fn held(e: &io::Error) -> bool {
    // Refused (5), in use (32) or delete pending (303), all tried again: a real refusal only
    // fails a little later.
    const DELETE_PENDING: i32 = 303;
    cfg!(windows) && matches!(e.raw_os_error(), Some(5 | 32 | DELETE_PENDING))
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

/// Where a project file at `path` is opened to be read. A test can make it fail
/// (`fault::unreadable`) to prove Opalro doesn't open a file. Does nothing without `fault-injection`.
pub fn open_point(path: &str) -> io::Result<()> {
    #[cfg(feature = "fault-injection")]
    {
        fault::opening(path)
    }
    #[cfg(not(feature = "fault-injection"))]
    {
        let _ = path;
        Ok(())
    }
}

/// Fault injection for crash tests: per-thread errors (`arm`), a process abort
/// (`OPALRO_FAULT_AT=N`), an error at one named point (`OPALRO_FAULT_FAIL=label`), a test's own
/// change to the disk at one named point (`act_at`), a project file that can't be opened
/// (`unreadable`, `OPALRO_FAULT_UNREADABLE=path`), or early flushes that fail or never start
/// (`early_flush`).
#[cfg(feature = "fault-injection")]
pub mod fault {
    use std::cell::{Cell, RefCell};
    use std::io;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Mutex, OnceLock};

    /// What a test does when this thread reaches a named point.
    type Action = (&'static str, Box<dyn FnOnce()>);

    thread_local! {
        static ARMED: Cell<Option<u64>> = const { Cell::new(None) };
        static COUNT: Cell<u64> = const { Cell::new(0) };
        static ACTION: RefCell<Option<Action>> = const { RefCell::new(None) };
        static UNREADABLE: RefCell<Option<String>> = const { RefCell::new(None) };
        static FLUSHES: Cell<u64> = const { Cell::new(0) };
        static EARLY_FLUSHES: Cell<u64> = const { Cell::new(0) };
        static EARLY: Cell<Early> = const { Cell::new(Early::On) };
    }

    /// How many flushes this thread has made through `fsio`.
    pub fn flushes() -> u64 {
        FLUSHES.get()
    }

    pub(super) fn flushed() {
        FLUSHES.set(FLUSHES.get().saturating_add(1));
    }

    /// How many flushes the helpers of the files this thread finished made while they were
    /// written. Never a fault point, so crash tests stop at the same places with them or without.
    pub fn early_flushes() -> u64 {
        EARLY_FLUSHES.get()
    }

    pub(super) fn flushed_early(n: u64) {
        EARLY_FLUSHES.set(EARLY_FLUSHES.get().saturating_add(n));
    }

    /// What the helper that flushes a big file along the way does.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum Early {
        On,
        /// Its flushes fail, as `OPALRO_FAULT_FAIL=flush:early` makes them.
        Fails,
        /// It is never started.
        Off,
    }

    /// Sets what the helpers of the files this thread writes from now on do.
    pub fn early_flush(how: Early) {
        EARLY.set(how);
    }

    pub(super) fn early() -> Early {
        if fails_at() == Some("flush:early") {
            return Early::Fails;
        }
        EARLY.get()
    }

    /// Makes this thread's opens of the project file at `path` fail, until called with `None`.
    pub fn unreadable(path: Option<&str>) {
        UNREADABLE.set(path.map(str::to_owned));
    }

    /// What a test set on a thread, for the workers it starts to set on themselves: the file it
    /// made unreadable and the fault points it armed.
    #[derive(Clone)]
    pub(crate) struct Hooks {
        unreadable: Option<String>,
        armed: Option<u64>,
        count: u64,
        early: Early,
    }

    pub(crate) fn hooks() -> Hooks {
        Hooks {
            unreadable: UNREADABLE.with_borrow(Clone::clone),
            armed: ARMED.get(),
            count: COUNT.get(),
            early: EARLY.get(),
        }
    }

    pub(crate) fn set_hooks(hooks: &Hooks) {
        UNREADABLE.set(hooks.unreadable.clone());
        ARMED.set(hooks.armed);
        COUNT.set(hooks.count);
        EARLY.set(hooks.early);
    }

    /// `unreadable` for a command run in a process of its own.
    fn unreadable_in_process() -> Option<&'static str> {
        static AT: OnceLock<Option<String>> = OnceLock::new();
        AT.get_or_init(|| std::env::var("OPALRO_FAULT_UNREADABLE").ok())
            .as_deref()
    }

    pub(super) fn opening(path: &str) -> io::Result<()> {
        let refused = UNREADABLE.with_borrow(|p| p.as_deref() == Some(path))
            || unreadable_in_process() == Some(path);
        if refused {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!("injected: {path} can't be opened"),
            ));
        }
        Ok(())
    }

    /// Runs `action` once, when this thread next reaches the point `label`, so a test can change
    /// what Opalro finds there.
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
        *AT.get_or_init(|| std::env::var("OPALRO_FAULT_AT").ok()?.parse().ok())
    }

    /// The label of the one point that fails every time it is reached, for a test that runs the
    /// command in a process of its own and wants it to carry on after the error.
    fn fails_at() -> Option<&'static str> {
        static AT: OnceLock<Option<String>> = OnceLock::new();
        AT.get_or_init(|| std::env::var("OPALRO_FAULT_FAIL").ok())
            .as_deref()
    }

    /// What a test does when any thread reaches a named point.
    type SharedAction = (&'static str, Box<dyn FnOnce() + Send>);
    static SHARED: Mutex<Option<SharedAction>> = Mutex::new(None);

    /// `act_at` for a point reached on another thread, such as one the walk starts. Only one test
    /// in a process should use it at a time.
    pub fn act_at_on_any_thread(label: &'static str, action: impl FnOnce() + Send + 'static) {
        if let Ok(mut shared) = SHARED.lock() {
            *shared = Some((label, Box::new(action)));
        }
    }

    pub(super) fn act(label: &'static str) {
        let due = ACTION.with_borrow_mut(|action| action.take_if(|(at, _)| *at == label));
        if let Some((_, action)) = due {
            action();
        }
        let shared = SHARED
            .lock()
            .ok()
            .and_then(|mut shared| shared.take_if(|(at, _)| *at == label));
        if let Some((_, action)) = shared {
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
    use std::fs;

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
    fn temp_names_are_the_ones_opal_makes() {
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

    /// A virus scanner or sync tool may hold a file for a moment: open, or deleted but still open,
    /// which keeps its name until it is closed.
    #[cfg(windows)]
    #[test]
    fn a_file_held_for_a_moment_is_replaced_once_let_go() {
        use std::os::windows::fs::OpenOptionsExt;
        const DELETE: u32 = 0x0001_0000;
        const DELETE_ON_CLOSE: u32 = 0x0400_0000;
        let root = tempfile::tempdir().unwrap();
        let dir = Dir::open_top(root.path()).unwrap();
        let head = root.path().join("head");
        for deleted in [false, true] {
            write_atomic(&dir, &dir, "head", b"old").unwrap();
            let held = if deleted {
                let open = fs::File::open(&head).unwrap();
                // Deleted once this closes, but its name stays while `open` holds it.
                fs::OpenOptions::new()
                    .access_mode(DELETE)
                    .custom_flags(DELETE_ON_CLOSE)
                    .open(&head)
                    .unwrap();
                open
            } else {
                // Shared with nobody, so it can't be replaced while open.
                fs::OpenOptions::new()
                    .read(true)
                    .share_mode(0)
                    .open(&head)
                    .unwrap()
            };
            let holder = std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(300));
                drop(held);
            });
            write_atomic(&dir, &dir, "head", b"new").unwrap();
            holder.join().unwrap();
            assert_eq!(fs::read(&head).unwrap(), b"new", "deleted: {deleted}");
            assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
        }
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

    /// `write` run twice in a fresh folder, and how many flushes it made.
    #[cfg(feature = "fault-injection")]
    fn flushes_of(write: fn(&Dir, &Dir, &str, &[u8]) -> io::Result<()>) -> u64 {
        let root = tempfile::tempdir().unwrap();
        let dir = Dir::open_top(root.path()).unwrap();
        let before = fault::flushes();
        write(&dir, &dir, "file", b"old").unwrap();
        write(&dir, &dir, "file", b"new").unwrap();
        assert_eq!(fs::read(root.path().join("file")).unwrap(), b"new");
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
        fault::flushes() - before
    }

    #[cfg(feature = "fault-injection")]
    #[test]
    fn a_cache_write_flushes_nothing() {
        assert_eq!(flushes_of(write_atomic), 4);
        assert_eq!(flushes_of(write_cache), 0);
    }

    #[cfg(feature = "fault-injection")]
    #[test]
    fn lock_info_is_written_without_a_flush() {
        assert_eq!(flushes_of(write_unflushed), 0);
        // It keeps its fault point.
        let root = tempfile::tempdir().unwrap();
        let dir = Dir::open_top(root.path()).unwrap();
        fault::arm(1);
        let err = write_unflushed(&dir, &dir, "lock-info", b"pid 1").unwrap_err();
        fault::disarm();
        assert_eq!(err.to_string(), "injected fault at write:before-rename");
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
    }

    /// Writes `mib` MiB to a new file through an `EarlyFlush`, a MiB at a time, and finishes it;
    /// the result and how many early flushes were made.
    #[cfg(feature = "fault-injection")]
    fn written_early(mib: u64) -> (io::Result<()>, u64) {
        let root = tempfile::tempdir().unwrap();
        let mut file = File::create(root.path().join("big")).unwrap();
        let before = fault::early_flushes();
        let mut early = EarlyFlush::new();
        let piece = vec![7u8; 1 << 20];
        for _ in 0..mib {
            file.write_all(&piece).unwrap();
            early.wrote(&file, piece.len() as u64);
        }
        let started = early.helper.is_some();
        let finished = early.finish();
        assert!(early.helper.is_none());
        assert_eq!(started, mib >= 16);
        (finished, fault::early_flushes() - before)
    }

    #[cfg(feature = "fault-injection")]
    #[test]
    fn a_small_file_starts_no_helper() {
        let (finished, flushes) = written_early(15);
        finished.unwrap();
        assert_eq!(flushes, 0);
    }

    #[cfg(feature = "fault-injection")]
    #[test]
    fn a_big_file_is_flushed_along_the_way() {
        let before = fault::flushes();
        let (finished, flushes) = written_early(40);
        finished.unwrap();
        assert!(flushes >= 1, "{flushes}");
        // The last flush is still the caller's.
        assert_eq!(fault::flushes(), before);
    }

    #[cfg(feature = "fault-injection")]
    #[test]
    fn a_failed_early_flush_fails_finish() {
        fault::early_flush(fault::Early::Fails);
        let (finished, _) = written_early(40);
        fault::early_flush(fault::Early::On);
        assert_eq!(
            finished.unwrap_err().to_string(),
            "injected fault at flush:early"
        );
    }

    #[cfg(feature = "fault-injection")]
    #[test]
    fn a_helper_turned_off_never_starts() {
        fault::early_flush(fault::Early::Off);
        let root = tempfile::tempdir().unwrap();
        let file = File::create(root.path().join("big")).unwrap();
        let mut early = EarlyFlush::new();
        early.wrote(&file, 40 << 20);
        fault::early_flush(fault::Early::On);
        assert!(early.helper.is_none());
    }

    #[test]
    fn no_helper_outlives_its_file() {
        let root = tempfile::tempdir().unwrap();
        let file = File::create(root.path().join("big")).unwrap();
        for dropped in [false, true] {
            let mut early = EarlyFlush::new();
            early.wrote(&file, 16 << 20);
            // The helper's thread holds the other half of this, with its own handle.
            let shared = Arc::downgrade(&early.helper.as_ref().unwrap().shared);
            if dropped {
                drop(early);
            } else {
                early.finish().unwrap();
            }
            assert!(shared.upgrade().is_none(), "dropped: {dropped}");
        }
    }

    #[test]
    fn a_helper_is_started_once_at_most() {
        let root = tempfile::tempdir().unwrap();
        let file = File::create(root.path().join("big")).unwrap();
        let mut early = EarlyFlush::new();
        // As if it couldn't start.
        early.tried = true;
        early.wrote(&file, 40 << 20);
        assert!(early.helper.is_none());
    }
}
