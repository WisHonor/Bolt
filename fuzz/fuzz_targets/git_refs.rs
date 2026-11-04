#![no_main]
use std::collections::HashSet;

use bolt_core::diary::validate_line_name;
use bolt_core::git::craft::Craft;
use bolt_core::git::{HashKind, Repo};
use bolt_core::import::{Rules, rename};
use libfuzzer_sys::fuzz_target;

// The input as `packed-refs` beside a good `HEAD`, then split: its first line as `HEAD`, the rest
// as `packed-refs`. The branches found are renamed as an import renames them, into names Bolt
// takes, no two alike but for letter case.
fuzz_target!(|data: &[u8]| {
    let Ok(dir) = tempfile::tempdir() else { return };
    let git = Craft::new(dir.path(), HashKind::Sha1);
    let line = data
        .iter()
        .position(|&b| b == b'\n')
        .map_or(data.len(), |at| at + 1);
    for (head, packed) in [(&b"ref: refs/heads/main\n"[..], data), data.split_at(line)] {
        git.file("HEAD", head);
        git.file("packed-refs", packed);
        let Ok(refs) = Repo::open(dir.path()).and_then(|repo| repo.refs()) else {
            continue;
        };
        let branches: Vec<&[u8]> = refs
            .branches
            .iter()
            .map(|(name, _)| name.as_slice())
            .collect();
        let mut folded = HashSet::new();
        for name in rename(&branches, Rules::Line) {
            assert!(validate_line_name(&name).is_ok(), "{name:?}");
            assert!(folded.insert(name.to_lowercase()), "{name:?}");
        }
        let tags: Vec<&[u8]> = refs.tags.iter().map(|(name, _)| name.as_slice()).collect();
        let _ = rename(&tags, Rules::Tag);
    }
});
