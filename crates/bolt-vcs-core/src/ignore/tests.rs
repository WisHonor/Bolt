#![allow(clippy::unwrap_used)]
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
    assert_eq!(then[2].bolt, Decision::Kept);
}

#[test]
fn gits_verdict_comes_from_the_gitignore_lines_alone() {
    use Decision::*;
    let both = |bolt, git| Verdict { bolt, git };
    let mut m = Matcher::new(PLAIN);
    m.enter(
        "",
        vec![
            Rules::parse(b".env\n*.pem\nlogs/\n", "", true),
            Rules::parse(b"!.env\n!*.pem\n*.tmp\n!logs/\n", "", false),
        ],
    );
    // A .boltignore line takes back what a .gitignore keeps out.
    assert_eq!(m.verdict(".env", false), both(Kept, Ignored));
    assert_eq!(m.verdict("logs", true), both(Kept, Ignored));
    // It can keep out more; where nothing matches, both say so.
    assert_eq!(m.verdict("a.tmp", false), both(Ignored, NoRule));
    assert_eq!(m.verdict("a.txt", false), both(NoRule, NoRule));
    // Where a .gitignore line decides, Git's verdict is Bolt's.
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
    // A `\` that ends the line keeps the spaces before it.
    let m = matcher(PLAIN, &[("", "a  \\")]);
    assert_eq!(m.decide("a  ", false), NoRule);
    assert_eq!(m.decide("a", false), NoRule);
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
