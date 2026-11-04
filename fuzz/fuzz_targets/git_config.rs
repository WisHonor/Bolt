#![no_main]
use bolt_core::git::craft::Craft;
use bolt_core::git::{HashKind, Repo};
use libfuzzer_sys::fuzz_target;

// The input as `.git/config`, and as `config.worktree`, which Bolt reads too when the config
// turns on `extensions.worktreeConfig`.
fuzz_target!(|data: &[u8]| {
    let Ok(dir) = tempfile::tempdir() else { return };
    let git = Craft::new(dir.path(), HashKind::Sha1);
    git.file("config", data);
    git.file("config.worktree", data);
    let _ = Repo::open(dir.path());
});
