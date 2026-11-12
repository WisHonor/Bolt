#![no_main]
use opal_core::git::craft::delta;
use opal_core::git::{MAX_IN_MEMORY_OBJECT, apply_delta};
use libfuzzer_sys::fuzz_target;

// The input's first two bytes give the base's length; the rest is a delta on it, and also an
// object, which a delta made as Git makes one must build back.
fuzz_target!(|data: &[u8]| {
    let Some((len, rest)) = data.split_first_chunk::<2>() else {
        return;
    };
    let (base, rest) = rest.split_at(usize::from(u16::from_le_bytes(*len)).min(rest.len()));
    if let Ok(built) = apply_delta(base, rest) {
        assert!(built.len() as u64 <= MAX_IN_MEMORY_OBJECT);
    }
    assert_eq!(apply_delta(base, &delta(base, rest)).as_deref(), Ok(rest));
});
