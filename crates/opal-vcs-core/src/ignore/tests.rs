#![allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]
use super::*;
use crate::drive::Drive;

const PLAIN: Drive = Drive {
    ignores_case: false,
    composes_unicode: false,
};

fn matcher(drive: Drive, files: &[(&str, &str)]) -> Matcher {
    let mut m = Matcher::new(drive);
    for &(folder, text) in files {
        m.enter(folder, vec![Rules::parse(text.as_bytes(), folder, true)]);
    }
    m
}

#[test]
fn rules_kept_in_force_give_the_verdicts_they_gave_then() {
    let mut m = matcher(PLAIN, &[("", "*.x\n!keep.x\n"), ("sub", "!*.x\n*.y\n")]);
    let paths = ["a.x", "keep.x", "sub/a.x", "sub/a.y", "sub/b.z"];
    let then: Vec<Verdict> = paths.iter().map(|p| m.verdict(p, false)).collect();
    let kept = m.in_force();
    m.leave_to("");
    m.enter("other", vec![Rules::parse(b"*\n", "other", false)]);
    let later: Vec<Verdict> = paths.iter().map(|p| kept.verdict(p, false)).collect();
    assert_eq!(later, then);
    assert_eq!(then[2].opal, Decision::Kept);
}

#[test]
fn gits_verdict_comes_from_the_gitignore_lines_alone() {
    use Decision::*;
    let both = |opal, git| Verdict { opal, git };
    let mut m = Matcher::new(PLAIN);
    m.enter(
        "",
        vec![
            Rules::parse(b".env\n*.pem\nlogs/\n", "", true),
            Rules::parse(b"!.env\n!*.pem\n*.tmp\n!logs/\n", "", false),
        ],
    );
    // A .opalignore line takes back what a .gitignore keeps out.
    assert_eq!(m.verdict(".env", false), both(Kept, Ignored));
    assert_eq!(m.verdict("logs", true), both(Kept, Ignored));
    // It can keep out more; where nothing matches, both say so.
    assert_eq!(m.verdict("a.tmp", false), both(Ignored, NoRule));
    assert_eq!(m.verdict("a.txt", false), both(NoRule, NoRule));
    // Where a .gitignore line decides, Git's verdict is Opalro's.
    m.enter("sub", vec![Rules::parse(b"!*.tmp\n", "sub", true)]);
    assert_eq!(m.verdict("sub/a.tmp", false), both(Kept, Kept));
    assert_eq!(m.verdict("sub/key.pem", false), both(Kept, Ignored));
}

#[test]
fn lines_are_read_as_git_reads_them() {
    let text =
        b"\xef\xbb\xbf*.log\r\n#comment\n\\#notes\n\nfoo  \nbar\\ \nbaz\\\\ \n x\r\r\nnul\0after\n";
    let m = matcher(PLAIN, &[("", std::str::from_utf8(text).unwrap())]);
    use Decision::*;
    for (path, want) in [
        ("a.log", Ignored),
        ("#comment", NoRule),
        ("#notes", Ignored),
        ("foo", Ignored),
        ("foo  ", NoRule),
        ("bar ", Ignored),
        ("baz\\", Ignored),
        (" x\r", Ignored),
        ("nul", Ignored),
        ("nulafter", NoRule),
    ] {
        assert_eq!(m.decide(path, false), want, "{path:?}");
    }
    // Of the spaces at the end, only the first can be kept, by an odd run of `\` before it.
    let m = matcher(PLAIN, &[("", "c\\\\\\   \nd\\\\\\\\  \n")]);
    assert_eq!(m.decide("c\\ ", false), Ignored);
    assert_eq!(m.decide("c\\   ", false), NoRule);
    assert_eq!(m.decide("d\\\\", false), Ignored);
    assert_eq!(m.decide("d\\\\ ", false), NoRule);
    // Only one byte order mark goes, and only one CR before each LF.
    let m = matcher(PLAIN, &[("", "\u{feff}\u{feff}a\r\nb\r\r\n")]);
    assert_eq!(m.decide("\u{feff}a", false), Ignored);
    assert_eq!(m.decide("b\r", false), Ignored);
}

#[test]
fn the_nearest_folder_and_its_last_line_decide() {
    use Decision::*;
    let mut m = matcher(
        PLAIN,
        &[
            ("", "*.log\n!keep.log\nbuild/\n"),
            ("sub", "!debug.log\n/top.txt\n"),
        ],
    );
    assert_eq!(m.decide("sub/debug.log", false), Kept);
    assert_eq!(m.decide("sub/other.log", false), Ignored);
    assert_eq!(m.decide("sub/keep.log", false), Kept);
    assert_eq!(m.decide("sub/top.txt", false), Ignored);
    assert_eq!(m.decide("sub/x/top.txt", false), NoRule);
    assert_eq!(m.decide("sub/build", false), NoRule);
    assert_eq!(m.decide("sub/build", true), Ignored);
    // Back in the top folder, `sub`'s rules are over.
    m.leave_to("");
    assert_eq!(m.decide("debug.log", false), Ignored);
    assert_eq!(m.decide("top.txt", false), NoRule);
}

#[test]
fn letter_case_follows_the_drive_with_gits_quirks() {
    use Decision::*;
    let text = "*.ENV\n\\B.txt\n[C].txt\n[A-Z].md\n";
    let plain = matcher(PLAIN, &[("", text)]);
    let folding = matcher(
        Drive {
            ignores_case: true,
            composes_unicode: false,
        },
        &[("", text)],
    );
    for (path, exact, folded) in [
        ("prod.env", NoRule, Ignored),
        ("PROD.ENV", Ignored, Ignored),
        ("B.txt", Ignored, NoRule),
        ("b.txt", NoRule, NoRule),
        ("C.txt", Ignored, NoRule),
        ("x.md", NoRule, Ignored),
        ("X.md", Ignored, Ignored),
    ] {
        assert_eq!(plain.decide(path, false), exact, "{path}");
        assert_eq!(
            folding.decide(path, false),
            folded,
            "{path} (letter case ignored)"
        );
    }
}

#[test]
fn names_are_compared_composed_where_the_drive_composes_them() {
    use Decision::*;
    let (nfc, nfd) = ("caf\u{e9}", "cafe\u{301}");
    let text = format!("/{nfc}.txt\n{nfd}.md\n");
    let composing = Drive {
        ignores_case: false,
        composes_unicode: true,
    };
    for folder in [nfc, nfd] {
        let m = matcher(composing, &[(folder, &text)]);
        // Git reads names composed there, and patterns as they are written.
        for name in [nfc, nfd] {
            let path = format!("{folder}/{name}");
            assert_eq!(m.decide(format!("{path}.txt"), false), Ignored, "{path}");
            assert_eq!(m.decide(format!("{path}.md"), false), NoRule, "{path}");
        }
        let m = matcher(PLAIN, &[(folder, &text)]);
        assert_eq!(m.decide(format!("{folder}/{nfc}.txt"), false), Ignored);
        assert_eq!(m.decide(format!("{folder}/{nfd}.txt"), false), NoRule);
        assert_eq!(m.decide(format!("{folder}/{nfd}.md"), false), Ignored);
    }
}

#[test]
fn the_last_line_wins_whether_plain_or_not() {
    use Decision::*;
    let folding = Drive {
        ignores_case: true,
        composes_unicode: false,
    };
    for (folder, text, path, is_dir, exact, folded) in [
        ("", "a.log\n!*.log\n", "a.log", false, Kept, Kept),
        ("", "!*.log\na.log\n", "a.log", false, Ignored, Ignored),
        ("", "*.tmp\n!keep.tmp\n", "keep.tmp", false, Kept, Kept),
        ("", "*.tmp\n!keep.tmp\n", "x.tmp", false, Ignored, Ignored),
        ("", "build/\n", "build", false, NoRule, NoRule),
        ("", "build/\n", "build", true, Ignored, Ignored),
        ("sub", "/a/b\n", "sub/a/b", false, Ignored, Ignored),
        ("sub", "/a/b\n", "a/b", false, NoRule, NoRule),
        ("sub", "/a/b\n", "sub/x/a/b", false, NoRule, NoRule),
        ("sub", "/a/b\n", "SUB/A/B", false, NoRule, Ignored),
        ("", "Makefile\n", "MAKEFILE", false, NoRule, Ignored),
        ("", "Makefile\n", "Makefile", false, Ignored, Ignored),
    ] {
        let what = format!("{text:?} {path:?}");
        assert_eq!(
            matcher(PLAIN, &[(folder, text)]).decide(path, is_dir),
            exact,
            "{what}"
        );
        assert_eq!(
            matcher(folding, &[(folder, text)]).decide(path, is_dir),
            folded,
            "{what} (letter case ignored)"
        );
    }
}

#[test]
fn bytes_in_force_are_the_files_of_the_folders_entered() {
    let mut m = matcher(PLAIN, &[("", "abc\n"), ("a", "x\n"), ("a/b", "yz\n")]);
    assert_eq!(m.in_force_bytes(), 9);
    m.leave_to("a/c");
    assert_eq!(m.in_force_bytes(), 6);
    m.leave_to("");
    assert_eq!(m.in_force_bytes(), 4);
}

const FOLDING: Drive = Drive {
    ignores_case: true,
    composes_unicode: false,
};

/// Pattern, then the paths it matches and those it doesn't, as `git check-ignore` decided them.
fn check_glob_match(cases: &[(&str, &[&str], &[&str])], casefold: bool) {
    for &(pattern, yes, no) in cases {
        for path in yes {
            assert!(
                glob_match(pattern.as_bytes(), path.as_bytes(), casefold),
                "{pattern:?} should match {path:?}"
            );
        }
        for path in no {
            assert!(
                !glob_match(pattern.as_bytes(), path.as_bytes(), casefold),
                "{pattern:?} should not match {path:?}"
            );
        }
    }
}

#[test]
fn double_stars_cross_folders_only_as_whole_folder_names() {
    check_glob_match(
        &[
            ("**", &["a", "a/b"], &[]),
            ("abc/**", &["abc/x", "abc/x/y"], &["abc"]),
            ("a/***/b", &["a/b", "a/x/b", "a/x/y/b"], &["a/xb"]),
            ("***/b", &["b", "x/b", "x/y/b"], &[]),
            ("a/**/**/b", &["a/b", "a/x/b", "a/x/y/z/b"], &[]),
            ("**/a", &["a", "x/a", "x/y/a"], &["xa", "a/x"]),
            ("a/**b", &["a/xb", "a/b"], &["a/x/b"]),
            ("**a/b", &["xa/b", "a/b"], &["x/a/b"]),
            ("x/**y/z", &["x/ay/z", "x/y/z"], &["x/a/y/z"]),
            ("*/**/b", &["a/b", "a/x/b"], &["b"]),
            // Git 2.52 and later: a `**` after other bytes of a name is a `*`.
            ("a**/b", &["ax/b", "a/b"], &["ax/y/b"]),
            ("a/b**", &["a/bc"], &["a/bc/d"]),
            // Before an escaped `/`, a whole-name `**` takes any run, but not nothing.
            ("**\\/b", &["a/x/b", "x/b"], &["b"]),
            ("x\\/**\\/y", &["x/k/y", "x/k/l/y"], &["x/y", "xy"]),
            ("**\\/", &["///", "a/"], &["a"]),
            ("a/**\\x", &["a/bx", "a/x"], &["a/b/x"]),
        ],
        false,
    );
    let m = matcher(PLAIN, &[("", "**\\/*\n")]);
    assert_eq!(m.decide("a/a/a", false), Decision::Ignored);
    assert_eq!(m.decide("a", false), Decision::NoRule);
}

#[test]
fn a_nul_ends_what_glob_match_is_given() {
    check_glob_match(
        &[
            ("\0", &["", "\0", "\0x"], &["x"]),
            ("a*\0b", &["a", "ax\0y"], &["xa"]),
            ("a\0[", &["a"], &[]),
        ],
        false,
    );
}

#[test]
fn classes_read_as_git_reads_them() {
    check_glob_match(
        &[
            ("[]a]x", &["]x", "ax"], &["bx"]),
            ("[!]a]x", &["bx"], &["]x", "ax", "/x"]),
            ("[^a]x", &["bx"], &["ax"]),
            ("[a-]x", &["ax", "-x"], &["bx"]),
            ("[-a]x", &["ax", "-x"], &["bx"]),
            ("[\\]]x", &["]x"], &["\\x"]),
            ("[a\\-c]x", &["ax", "-x", "cx"], &["bx"]),
            ("[\\a-\\c]x", &["ax", "bx", "cx"], &["dx"]),
            // A backwards range takes nothing, but its first byte stands on its own too.
            ("[z-a]x", &["zx"], &["ax", "bx"]),
            ("[a-c-e]x", &["ax", "bx", "cx", "-x", "ex"], &["dx"]),
            ("[--0]x", &["-x", ".x", "0x"], &["1x", "/x"]),
            ("[a-\\]]x", &["ax"], &["]x", "bx"]),
            ("[[:alpha:]]x", &["ax", "Zx"], &["1x"]),
            ("[[:digit:]-a]x", &["1x", "-x", "ax"], &["bx"]),
            // `[:` that isn't a class name is a `[` and a `:`.
            ("[[:alpha]x", &["ax", "lx", "[x", ":x"], &["bx"]),
            ("[[:]x", &["[x", ":x"], &["]x", "x"]),
            ("[[:al]pha:]]x", &["apha:]]x"], &["ax", "x"]),
            ("[{]x", &["{x"], &[]),
            // Ranges across the whole byte range.
            ("[ -\u{7f}]", &[" ", "~", "\u{7f}", "."], &["\t", "/"]),
            ("[!a-z]", &["A", "\u{1}"], &["a", "q", "z", "/"]),
            ("{a,b}", &["{a,b}"], &["a", "b"]),
            // A class never takes a `/`.
            ("a[.-0]c", &["a.c"], &["a/c"]),
            ("a[!b]c", &["adc"], &["a/c", "abc"]),
            ("**/a[!b]c", &["a/c/adc", "adc"], &[]),
        ],
        false,
    );
    // Bytes above ASCII, which a path that isn't UTF-8 can hold.
    assert!(glob_match(b"[\x7f-\xc1]", b"\x80", false));
    assert!(glob_match(b"[\x7f-\xc1]", b"\xc1", false));
    assert!(!glob_match(b"[\x7f-\xc1]", b"\xc2", false));
    assert!(glob_match(b"[!a]", b"\xff", false));
    // These lines match nothing at all.
    for pattern in [
        "[[:foo:]]x",
        "[[::]]x",
        "a[b",
        "[^]x",
        "[]",
        "a\\",
        "[a\\",
        "a[/]b",
    ] {
        for path in ["x", "fx", "a[b", "ab", "]x", "^x", "a", "a\\", "a/b", "[]"] {
            assert!(
                !glob_match(pattern.as_bytes(), path.as_bytes(), false),
                "{pattern} {path}"
            );
        }
    }
}

#[test]
fn letter_case_is_folded_as_git_folds_it() {
    check_glob_match(
        &[
            ("[A-Z]1", &["x1", "X1"], &[]),
            ("[a-z]2", &["x2", "X2"], &[]),
            ("[B]3", &[], &["b3", "B3"]),
            ("[b]4", &["b4", "B4"], &[]),
            ("[[:upper:]]5", &["x5", "X5"], &["15"]),
            ("[[:lower:]]6", &["x6", "X6"], &[]),
            ("\\B7", &[], &["b7", "B7"]),
            ("\\b8", &["b8", "B8"], &[]),
            ("[!B]9", &["b9", "B9", "x9"], &[]),
            ("[!b]0", &["x0"], &["b0", "B0"]),
            ("[B-B]1", &["b1", "B1"], &[]),
            ("[Z-a]2", &["z2", "Z2", "_2", "A2"], &["y2"]),
            ("[Z-A]3", &[], &["z3", "Z3"]),
            ("[^A-Z]4", &["_4"], &["x4", "X4"]),
            ("Make*", &["makefile", "MAKEFILE.am"], &["make/x"]),
        ],
        true,
    );
    check_glob_match(
        &[
            ("[Z-a]2", &["Z2", "a2", "_2"], &["z2", "A2"]),
            ("[Z-A]3", &["Z3"], &["z3"]),
            ("[[:upper:]]5", &["X5"], &["x5"]),
        ],
        false,
    );
}

#[test]
fn lines_git_cant_use_match_nothing() {
    use Decision::*;
    let text = "tab\t\n!\n/\n!/\n\\!bang\n lead\nd\\/\nc//\na\\/b\n*.o\n";
    let m = matcher(PLAIN, &[("", text)]);
    for (path, is_dir, want) in [
        ("tab\t", false, Ignored),
        ("tab", false, NoRule),
        ("!", false, NoRule),
        ("!bang", false, Ignored),
        ("bang", false, NoRule),
        (" lead", false, Ignored),
        ("lead", false, NoRule),
        ("d", true, NoRule),
        ("c", true, NoRule),
        ("x/c", true, NoRule),
        ("a/b", false, Ignored),
        ("x/a/b", false, NoRule),
        ("x/y.o", false, Ignored),
        ("x/y.o/z", false, NoRule),
    ] {
        assert_eq!(m.decide(path, is_dir), want, "{path:?}");
    }
}

#[test]
fn plain_names_and_other_lines_keep_their_order() {
    use Decision::*;
    // The plain names are looked up apart from the rest, which mustn't change which line wins.
    let text = "a\n*\n!a\nb/\n!*.c\nc.c\nc.c/\n";
    for drive in [PLAIN, FOLDING] {
        let m = matcher(drive, &[("", text)]);
        for (path, is_dir, want) in [
            ("a", false, Kept),
            ("x/a", false, Kept),
            ("b", false, Ignored),
            ("b", true, Ignored),
            ("q.c", false, Kept),
            ("c.c", false, Ignored),
            ("c.c", true, Ignored),
        ] {
            assert_eq!(m.decide(path, is_dir), want, "{path:?} {is_dir}");
        }
    }
    let m = matcher(FOLDING, &[("", "A\n!a\n")]);
    assert_eq!(m.decide("x/A", false), Decision::Kept);
    let m = matcher(PLAIN, &[("", "A\n!a\n")]);
    assert_eq!(m.decide("x/A", false), Decision::Ignored);
    // Plain paths too, tied to the folder or not.
    for (text, path, want) in [
        ("/a/b\n!a/*\n", "s/a/b", Kept),
        ("!a/*\n/a/b\n", "s/a/b", Ignored),
        ("!a/*\na/b\n", "s/a/b", Ignored),
        ("a/b\n!b\n", "s/a/b", Kept),
        ("!b\na/b\n", "s/a/b", Ignored),
        ("a/b\n", "s/x/a/b", NoRule),
    ] {
        let m = matcher(PLAIN, &[("s", text)]);
        assert_eq!(m.decide(path, false), want, "{text:?} {path:?}");
    }
}

#[test]
fn rules_apply_only_below_their_folder() {
    use Decision::*;
    let mut m = matcher(PLAIN, &[("", "*.x\n"), ("sub", "!*.x\n/top\nmid/end\n")]);
    assert_eq!(m.decide("a.x", false), Ignored);
    assert_eq!(m.decide("subx/a.x", false), Ignored);
    assert_eq!(m.decide("sub/a.x", false), Kept);
    assert_eq!(m.decide("sub", true), NoRule);
    assert_eq!(m.decide("sub/top", false), Ignored);
    assert_eq!(m.decide("sub/mid/end", false), Ignored);
    assert_eq!(m.decide("sub/x/mid/end", false), NoRule);
    assert_eq!(m.decide("top", false), NoRule);
    // A sibling folder doesn't keep `sub`'s rules in force.
    m.leave_to("subway");
    assert_eq!(m.decide("sub/a.x", false), Ignored);
    assert_eq!(m.in_force_bytes(), 4);
}

#[test]
fn matching_takes_time_in_proportion_to_pattern_and_path() {
    // A matcher that backtracks would take years over these.
    let pattern = "**/".repeat(30) + "x";
    let path = "a/".repeat(3000) + "y";
    assert!(!glob_match(pattern.as_bytes(), path.as_bytes(), false));
    let pattern = "*a".repeat(40) + "b";
    let path = "a".repeat(5000);
    assert!(!glob_match(pattern.as_bytes(), path.as_bytes(), true));
    let pattern = "a/**/".repeat(30) + "[!/]*?b";
    let path = "a/".repeat(2000) + "zzb";
    assert!(glob_match(pattern.as_bytes(), path.as_bytes(), false));
}

#[test]
fn the_quick_test_never_turns_away_a_match() {
    let pieces = [
        "a",
        "B",
        "/",
        "*",
        "**",
        "?",
        "[a-c]",
        "[!a]",
        "[[:upper:]]",
        "\\b",
        "\\B",
        "**/",
        "/**",
        "[B]",
    ];
    let mut texts = vec![String::new()];
    for _ in 0..3 {
        let longer: Vec<String> = texts
            .iter()
            .flat_map(|t| ["a", "B", "c", "/"].map(|c| format!("{t}{c}")))
            .collect();
        texts.extend(longer);
    }
    texts.sort();
    texts.dedup();
    let mut tried = 0;
    for a in pieces {
        for b in pieces {
            for c in pieces {
                let pattern = format!("{a}{b}{c}");
                let Some(pat) = compile(pattern.as_bytes()) else {
                    continue;
                };
                let sieve = Sieve::of(&pat);
                for text in &texts {
                    for fold in [false, true] {
                        if pat.matches(text.as_bytes(), fold) {
                            tried += 1;
                            let held = Bits::held_by(text.as_bytes(), fold);
                            assert!(
                                sieve.passes(text.as_bytes(), &held),
                                "{pattern:?} {text:?} fold {fold}"
                            );
                        }
                    }
                }
            }
        }
    }
    assert!(tried > 1000);
}

#[test]
fn reading_a_bracket_takes_time_in_proportion_to_its_length() {
    // A reader that looks ahead for `]` again from every `[:` would take hours over these.
    let n = 500_000;
    let started = std::time::Instant::now();
    let opens = "[:".repeat(n);
    for (pattern, takes, refuses) in [
        (format!("[{opens}x]"), "x", "y"),
        (format!("[{}]", "[:a".repeat(n)), "a", "b"),
        (format!("[{}]", "\\]".repeat(n)), "]", "\\"),
        (format!("[a{}]", "-[:".repeat(n)), ":", "b"),
    ] {
        assert!(glob_match(pattern.as_bytes(), takes.as_bytes(), false));
        assert!(!glob_match(pattern.as_bytes(), refuses.as_bytes(), false));
    }
    // Never closed, or a class name that runs to the end: these match nothing.
    for pattern in [format!("[{opens}x"), format!("[{opens}:]")] {
        assert!(!glob_match(pattern.as_bytes(), b"x", false));
        let m = matcher(PLAIN, &[("", &format!("{pattern}\n{pattern}\n"))]);
        assert_eq!(m.decide("x", false), Decision::NoRule);
    }
    assert!(started.elapsed() < std::time::Duration::from_secs(60));
}
