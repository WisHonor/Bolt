#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(journal) = opal_core::journal::Journal::decode(data) {
        // What reads back writes back the same, and every path passed the name rules.
        assert_eq!(journal.encode(), data);
        for action in &journal.actions {
            assert!(opal_core::names::validate_path(&action.path).is_ok());
        }
    }
});
