#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // Encoding is canonical, so anything that decodes must encode back to the same bytes.
    if let Ok(list) = bolt_core::chunks::ChunkList::decode(data) {
        assert_eq!(list.encode(), data);
    }
});
