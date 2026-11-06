//! The user's own cache key, and the seal it puts on caches (decision 053). A cache travels with
//! a copied project, so one is trusted only if this account's key sealed it.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use crate::config;
use crate::repo::RepoError;
use crate::safe_fs;

/// The key's length, and the seal's.
pub const KEY_LEN: usize = 32;
/// The key file's name, in the folder of the user's `config.toml`.
pub const KEY_FILE: &str = "cache-key";

/// The user's cache key, made on first use. Two Bolts making it at once end with one key. A key
/// file that can't be read, or isn't 32 bytes, is an error naming it: it is the user's own, and
/// replacing it silently could hide that someone changed it. With no settings folder, a key for
/// this run alone, so no cache is trusted again.
pub fn cache_key() -> Result<[u8; KEY_LEN], RepoError> {
    match settings_folder() {
        Some(dir) => key_in(&dir),
        None => Ok(random_key()?),
    }
}

/// `body` followed by its keyed BLAKE3 hash under `key`.
#[must_use]
pub fn seal(key: &[u8; KEY_LEN], body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(body.len().saturating_add(KEY_LEN));
    out.extend_from_slice(body);
    out.extend_from_slice(blake3::keyed_hash(key, body).as_bytes());
    out
}

/// The body of `bytes` if `key` sealed it, else `None`.
#[must_use]
pub fn open_sealed<'a>(key: &[u8; KEY_LEN], bytes: &'a [u8]) -> Option<&'a [u8]> {
    let (body, tag) = bytes.split_at_checked(bytes.len().checked_sub(KEY_LEN)?)?;
    let tag: [u8; KEY_LEN] = tag.try_into().ok()?;
    // `Hash` compares in constant time.
    (blake3::keyed_hash(key, body) == tag).then_some(body)
}

fn settings_folder() -> Option<PathBuf> {
    // Unit tests keep their key out of the user's own settings.
    if cfg!(test) {
        return Some(std::env::temp_dir().join("bolt-core-tests"));
    }
    Some(config::user_config_path()?.parent()?.to_owned())
}

fn random_key() -> io::Result<[u8; KEY_LEN]> {
    let mut key = [0; KEY_LEN];
    getrandom::fill(&mut key).map_err(io::Error::other)?;
    Ok(key)
}

/// The key in the folder `dir`, made there if there is none.
pub(crate) fn key_in(dir: &Path) -> Result<[u8; KEY_LEN], RepoError> {
    let path = dir.join(KEY_FILE);
    let unusable = |e: io::Error| key_error(&path, &e);
    match read_key(&path) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        read => return read.map_err(unusable),
    }
    make_key(dir, &path).map_err(unusable)?;
    // Whoever won a race, this reads the key both use.
    read_key(&path).map_err(unusable)
}

/// The user's own file, so a link is followed, as for their `config.toml`.
fn read_key(path: &Path) -> io::Result<[u8; KEY_LEN]> {
    let bytes = safe_fs::read_capped_following_links(path, 4096)?;
    let len = bytes.len();
    bytes.try_into().map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("it is {len} bytes long, not {KEY_LEN}"),
        )
    })
}

/// Writes a new key to a file of its own, then links it in as `path`, which never replaces a key
/// another Bolt linked in first.
fn make_key(dir: &Path, path: &Path) -> io::Result<()> {
    fs::create_dir_all(dir)?;
    let key = random_key()?;
    let temp = dir.join(bolt_platform::temp_name());
    let written = new_private(&temp).and_then(|mut file| {
        file.write_all(&key)?;
        file.sync_all()
    });
    let linked = written.and_then(|()| fs::hard_link(&temp, path));
    // Only litter now, whatever happened.
    let _ = fs::remove_file(&temp);
    match linked {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
        // A drive without hard links (FAT): made in place, where a racing Bolt may briefly see
        // it short and say so.
        Err(_) => match new_private(path) {
            Ok(mut file) => {
                file.write_all(&key)?;
                file.sync_all()?;
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e),
        },
    }
    Ok(())
}

/// A new file only the user may read.
fn new_private(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    options.open(path)
}

fn key_error(path: &Path, e: &io::Error) -> RepoError {
    // `safe_fs` starts its messages with the path, which the error names already.
    let said = e.to_string();
    let detail = match said.strip_prefix(&path.display().to_string()) {
        Some(rest) => format!("it{rest}"),
        None => said,
    };
    RepoError::CacheKey {
        path: path.to_owned(),
        detail,
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;

    #[test]
    fn a_key_is_made_once_and_read_back() {
        let dir = tempfile::tempdir().unwrap();
        let settings = dir.path().join("not yet made");
        let key = key_in(&settings).unwrap();
        assert_eq!(key_in(&settings).unwrap(), key);
        assert_eq!(fs::read(settings.join(KEY_FILE)).unwrap(), key);
        // Only the key is left behind.
        assert_eq!(fs::read_dir(&settings).unwrap().count(), 1);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(settings.join(KEY_FILE))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }

    #[test]
    fn bolts_making_the_key_at_once_end_with_one() {
        let dir = tempfile::tempdir().unwrap();
        let keys: Vec<_> = std::thread::scope(|s| {
            let racers: Vec<_> = (0..8).map(|_| s.spawn(|| key_in(dir.path()))).collect();
            racers
                .into_iter()
                .map(|r| r.join().unwrap().unwrap())
                .collect()
        });
        assert!(keys.iter().all(|k| *k == keys[0]));
    }

    #[test]
    fn a_key_of_the_wrong_length_is_an_error_naming_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(KEY_FILE);
        for bytes in [&b""[..], &[7; 31], &[7; 33]] {
            fs::write(&path, bytes).unwrap();
            let e = key_in(dir.path()).unwrap_err();
            assert!(matches!(e, RepoError::CacheKey { .. }), "{e:?}");
            assert!(e.to_string().contains(KEY_FILE), "{e}");
            assert!(e.hint().unwrap().contains("Delete"), "{e:?}");
            // Never replaced.
            assert_eq!(fs::read(&path).unwrap(), bytes);
        }
    }

    #[test]
    fn a_seal_opens_only_with_its_key_and_body() {
        let (key, other) = ([1; KEY_LEN], [2; KEY_LEN]);
        let sealed = seal(&key, b"body");
        assert_eq!(open_sealed(&key, &sealed), Some(&b"body"[..]));
        assert_eq!(open_sealed(&other, &sealed), None);
        for at in 0..sealed.len() {
            let mut edited = sealed.clone();
            edited[at] ^= 1;
            assert_eq!(open_sealed(&key, &edited), None, "{at}");
        }
        assert_eq!(open_sealed(&key, &sealed[..KEY_LEN - 1]), None);
        assert_eq!(open_sealed(&key, &seal(&key, b"")), Some(&b""[..]));
    }
}
