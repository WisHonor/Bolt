#![no_main]
use bolt_core::drive::Drive;
use bolt_core::names;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(text) = std::str::from_utf8(data) {
        let given: Vec<&str> = text.split('\n').collect();
        // Each pair of names as a long name and the short name a folder gives it.
        let short_names: Vec<(&str, &str)> = given.chunks_exact(2).map(|p| (p[0], p[1])).collect();
        for n in &given {
            let _ = names::validate_name(n);
            let _ = names::is_git_alias(n);
            for ignores_case in [false, true] {
                let drive = Drive {
                    ignores_case,
                    composes_unicode: false,
                };
                let _ = names::own_folder(n, drive);
            }
            // Whatever write-out may write, this system can hold, and it is neither `.git` nor
            // `.bolt`.
            if names::writable_here(n, &short_names).is_ok() {
                assert!(names::holdable_here(n));
                assert!(!names::is_git_alias(n) && !names::is_bolt_alias(n));
            }
        }
        let _ = names::folder_problems(&given);
    }
});
