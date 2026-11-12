#![no_main]
use opal_core::git::{
    HashKind, is_exact_lfs_pointer, is_lfs_lookalike, lfs_pointer, looks_like_lfs_pointer,
    parse_commit, parse_person, parse_tag, parse_tree,
};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    for hash in [HashKind::Sha1, HashKind::Sha256] {
        let _ = parse_commit(data, hash);
        let _ = parse_tree(data, hash);
        let _ = parse_tag(data, hash);
    }
    let _ = parse_person(data);
    let _ = looks_like_lfs_pointer(data);
    let _ = is_lfs_lookalike(data);
    // Any pointer Git LFS reads, written as Git LFS writes it, reads back the same and is exact.
    if let Some(p) = lfs_pointer(data) {
        let _ = is_exact_lfs_pointer(data, &p);
        if p.size > 0 && !p.extensions {
            let written = format!(
                "version https://git-lfs.github.com/spec/v1\noid sha256:{}\nsize {}\n",
                p.oid_hex(),
                p.size
            );
            assert_eq!(lfs_pointer(written.as_bytes()), Some(p));
            assert!(is_exact_lfs_pointer(written.as_bytes(), &p));
        }
    }
});
