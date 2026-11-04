#![no_main]
use bolt_core::git::craft::{Craft, digest};
use bolt_core::git::{GitId, HashKind, Repo};
use libfuzzer_sys::fuzz_target;

// The input's first two bytes give the index's length; the rest is the pack. Each gets the
// checksums it ends with, which no fuzzer finds by chance, then every object the index lists is
// read.
fuzz_target!(|data: &[u8]| {
    let Some((len, rest)) = data.split_first_chunk::<2>() else {
        return;
    };
    let (idx, pack) = rest.split_at(usize::from(u16::from_le_bytes(*len)).min(rest.len()));
    // The pack's own checksum is only compared with the index's copy of it, never hashed.
    let trailer = [0u8; 20];
    let idx = [idx, &trailer].concat();
    let idx = [idx.as_slice(), &digest(HashKind::Sha1, &idx)].concat();
    let Ok(dir) = tempfile::tempdir() else { return };
    let git = Craft::new(dir.path(), HashKind::Sha1);
    let name = format!("objects/pack/pack-{}", "0".repeat(40));
    git.file(&format!("{name}.idx"), &idx);
    git.file(&format!("{name}.pack"), &[pack, &trailer].concat());
    let Ok(repo) = Repo::open(dir.path()) else {
        return;
    };
    // The fan-out's last count, then the names, 20 bytes each; a damaged index reads none.
    let count = idx
        .get(1028..1032)
        .and_then(|n| n.try_into().ok())
        .map_or(0, u32::from_be_bytes);
    for at in (1032..).step_by(20).take(count.min(64) as usize) {
        let Some(name) = idx.get(at..at + 20) else {
            break;
        };
        let hex: String = name.iter().map(|b| format!("{b:02x}")).collect();
        let Some(id) = GitId::from_hex(HashKind::Sha1, &hex) else {
            continue;
        };
        let _ = repo.header(&id);
        let _ = repo.read(&id);
        if let Ok(mut blob) = repo.blob_reader(&id) {
            let _ = std::io::copy(&mut blob, &mut std::io::sink());
        }
    }
});
