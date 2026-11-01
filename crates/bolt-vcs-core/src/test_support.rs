//! Helpers shared by unit tests: hex, and making links.

use std::path::Path;

pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub(crate) fn unhex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).expect("test hex"))
        .collect()
}

/// Makes `link` point at `target`; false, with a note, where Windows lacks the privilege.
pub(crate) fn symlink(target: &Path, link: &Path) -> bool {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, link).expect("make a symlink");
        true
    }
    #[cfg(windows)]
    {
        let made = if target.is_dir() {
            std::os::windows::fs::symlink_dir(target, link)
        } else {
            std::os::windows::fs::symlink_file(target, link)
        };
        match made {
            Ok(()) => true,
            // ERROR_PRIVILEGE_NOT_HELD: symlinks need Developer Mode or admin rights.
            Err(e) if e.raw_os_error() == Some(1314) => {
                eprintln!("note: symlink check skipped, this account can't create symlinks");
                false
            }
            Err(e) => panic!("can't create a symlink: {e}"),
        }
    }
}

/// A Windows junction (no privilege needed) at `link`, pointing at the folder `target`.
#[cfg(windows)]
pub(crate) fn junction(link: &Path, target: &Path) {
    // `/D` skips cmd's AutoRun hooks; mklink's "Junction created" line is not wanted in logs.
    let made = std::process::Command::new("cmd")
        .args(["/D", "/C", "mklink", "/J"])
        .arg(link)
        .arg(target)
        .stdout(std::process::Stdio::null())
        .status()
        .expect("run mklink");
    assert!(made.success(), "mklink /J {} failed", link.display());
}
