#![no_main]
use opal_core::drive::Drive;
use opal_core::ignore::{Matcher, Rules, glob_match};
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
            // A `.gitignore` at the top and a `.opalroignore` below, so Git's verdict is looked
            // for again where the `.opalroignore` decides.
            matcher.enter("", vec![Rules::parse(data, "", true)]);
            matcher.enter("a/b", vec![Rules::parse(data, "a/b", false)]);
            for path in paths {
                for is_dir in [false, true] {
                    let _ = matcher.verdict(path, is_dir);
                }
            }
        }
    }
    let (pattern, text) = data.split_at(data.len() / 2);
    let _ = glob_match(pattern, text, false);
    let _ = glob_match(pattern, text, true);
});
