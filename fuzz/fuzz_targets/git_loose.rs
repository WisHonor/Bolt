#![no_main]
use opal_core::git::craft::{Craft, zlib};
use opal_core::git::{GitId, HashKind, Repo};
use libfuzzer_sys::fuzz_target;

// The input as a loose object's file, then compressed as one, which gets past zlib to the
// header and the sizes it claims.
fuzz_target!(|data: &[u8]| {
    let Ok(dir) = tempfile::tempdir() else { return };
    let git = Craft::new(dir.path(), HashKind::Sha1);
    let hex = "ab".repeat(20);
    let (Ok(repo), Some(id)) = (
        Repo::open(dir.path()),
        GitId::from_hex(HashKind::Sha1, &hex),
    ) else {
        return;
    };
    for file in [data.to_vec(), zlib(data)] {
        git.raw_loose(&hex, &file);
        let _ = repo.header(&id);
        let _ = repo.read(&id);
        if let Ok(mut blob) = repo.blob_reader(&id) {
            let _ = std::io::copy(&mut blob, &mut std::io::sink());
        }
    }
});
