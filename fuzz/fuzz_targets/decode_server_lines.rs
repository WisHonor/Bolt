#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = opal_core::diary::ServerLines::decode(data);
});
