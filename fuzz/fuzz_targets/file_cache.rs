#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(cache) = opal_core::filecache::FileCache::decode(data) {
        // What reads back writes back the same.
        assert_eq!(cache.encode(), data);
    }
});
