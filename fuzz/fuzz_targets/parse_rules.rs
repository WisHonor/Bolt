#![no_main]
use bolt_core::rules::{RulesRead, parse, parse_size, size_text};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    match parse(data) {
        Ok((rules, ignored)) => {
            // What is kept keeps to the limits.
            assert!((1 << 20..=1 << 37).contains(&rules.chunk_files_over));
            assert!(rules.preset.as_ref().is_none_or(|p| p.len() <= 64));
            for rule in &rules.rules {
                assert!(!rule.files.is_empty());
                assert!(rule.files.iter().all(|p| !p.is_empty()));
            }
            assert_eq!(parse_size(&size_text(rules.ask_over_bytes)), Some(rules.ask_over_bytes));
            let read = RulesRead {
                rules,
                file: true,
                skipped: None,
                ignored,
            };
            let warnings = read.warnings();
            assert!(warnings.len() <= 1);
            assert!(warnings.iter().all(|w| !w.message.contains('\u{1b}')));
        }
        Err(why) => {
            let read = RulesRead {
                skipped: Some(why),
                ..RulesRead::default()
            };
            assert!(!read.warnings()[0].message.contains('\u{1b}'));
        }
    }
    if let Ok(text) = std::str::from_utf8(data) {
        if let Some(bytes) = parse_size(text) {
            assert_eq!(parse_size(&size_text(bytes)), Some(bytes));
        }
    }
});
