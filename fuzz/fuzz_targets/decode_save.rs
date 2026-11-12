#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = opal_core::save::Save::decode(data, false);
    let _ = opal_core::save::Save::decode(data, true);
});
