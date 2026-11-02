#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(text) = std::str::from_utf8(data) {
        let names: Vec<&str> = text.split('\n').collect();
        for n in &names {
            let _ = bolt_core::names::validate_name(n);
            let _ = bolt_core::names::is_git_alias(n);
        }
        let _ = bolt_core::names::folder_problems(&names);
    }
});
