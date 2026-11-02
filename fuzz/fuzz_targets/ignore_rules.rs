#![no_main]
use bolt_core::drive::Drive;
use bolt_core::ignore::{Matcher, Rules, wildmatch};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // The input is a rule file, and its first line (kept to a name's length) a path as well.
    let first = data.split(|&b| b == b'\n').next().unwrap_or_default();
    let fuzzed = &first[..first.len().min(300)];
    let paths: [&[u8]; 7] = [
        b"a",
        b"a/b",
        b"a/b/c.txt",
        b"A/B/.Git",
        "a/b/caf\u{e9}".as_bytes(),
        "a/b/cafe\u{301}".as_bytes(),
        fuzzed,
    ];
    for ignores_case in [false, true] {
        for composes_unicode in [false, true] {
            let mut matcher = Matcher::new(Drive {
                ignores_case,
                composes_unicode,
            });
            matcher.enter("", vec![Rules::parse(data, "")]);
            matcher.enter("a/b", vec![Rules::parse(data, "a/b")]);
            for path in paths {
                for is_dir in [false, true] {
                    let _ = matcher.decide(path, is_dir);
                }
            }
        }
    }
    let (pattern, text) = data.split_at(data.len() / 2);
    let _ = wildmatch(pattern, text, false);
    let _ = wildmatch(pattern, text, true);
});
