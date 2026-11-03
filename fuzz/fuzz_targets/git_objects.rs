#![no_main]
use bolt_core::git::{HashKind, is_lfs_pointer, parse_commit, parse_person, parse_tag, parse_tree};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    for hash in [HashKind::Sha1, HashKind::Sha256] {
        let _ = parse_commit(data, hash);
        let _ = parse_tree(data, hash);
        let _ = parse_tag(data, hash);
    }
    let _ = parse_person(data);
    let _ = is_lfs_pointer(data);
});
