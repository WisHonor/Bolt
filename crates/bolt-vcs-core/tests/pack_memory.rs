//! Measures the process's peak memory, so this file holds a single test: any other test running
//! beside it would add its own. Only Linux tells the peak (VmHWM).
#![cfg(target_os = "linux")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use bolt_core::id::Id;
use bolt_core::pack::{ObjectKind, PACK_END_MAGIC, PACK_MAGIC};
use bolt_core::store::{Store, StoreError};
use std::io::{Seek, SeekFrom, Write};

/// The process's peak resident memory so far, in bytes.
fn peak() -> u64 {
    let status = std::fs::read_to_string("/proc/self/status").unwrap();
    let line = status.lines().find(|l| l.starts_with("VmHWM:")).unwrap();
    line.split_whitespace()
        .nth(1)
        .unwrap()
        .parse::<u64>()
        .unwrap()
        * 1024
}

#[test]
fn a_planted_pack_claiming_2_pow_24_objects_costs_reads_not_memory() {
    const COUNT: u64 = 1 << 24;
    let root = tempfile::tempdir().unwrap();
    // A version 2 pack whose index claims 2^24 objects, all hole but its first bytes and trailer.
    // Every ID starts with 0xff by the fan-out, and reads as zeros.
    let index_len = 8 + 1024 + 57 * COUNT;
    let mut f = std::fs::File::create(root.path().join("zz.pack")).unwrap();
    f.write_all(&PACK_MAGIC).unwrap();
    f.write_all(&2u32.to_le_bytes()).unwrap();
    f.write_all(&0u32.to_le_bytes()).unwrap();
    f.write_all(&COUNT.to_le_bytes()).unwrap();
    for b in 0..256 {
        let below: u32 = if b == 255 { 1 << 24 } else { 0 };
        f.write_all(&below.to_le_bytes()).unwrap();
    }
    f.set_len(16 + index_len).unwrap();
    f.seek(SeekFrom::End(0)).unwrap();
    f.write_all(&16u64.to_le_bytes()).unwrap();
    f.write_all(&[0; 32]).unwrap();
    f.write_all(&PACK_END_MAGIC).unwrap();
    drop(f);

    let before = peak();
    let store = Store::open(root.path()).unwrap();
    let wanted = Id([0xff; 32]);
    let read = store.read_record(&wanted, ObjectKind::Save);
    assert!(matches!(read, Err(StoreError::Missing(_))), "{read:?}");
    let grew = peak().saturating_sub(before);
    assert!(
        grew < 64 << 20,
        "opening and one lookup took {grew} bytes more"
    );
}
