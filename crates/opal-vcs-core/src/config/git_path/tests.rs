#![allow(clippy::unwrap_used)]
use super::*;

/// What Windows adds: `\`, drives and network paths.
#[test]
fn git_config_system_is_tidied_as_git_tidies_it() {
    let failed = None;
    let to = |text: &str| Some(text.to_owned());
    let windows = [
        ("\\dev\\null", to("/dev/null")),
        ("C:\\dir\\..\\x", to("C:/x")),
        ("C:dir/../..", failed.clone()),
        ("C:\\..", failed.clone()),
        ("\\\\server\\share\\dir\\..\\x", to("//server/share/x")),
        ("//server/share/../x", failed.clone()),
        // A network path with no share keeps no root, as in Git.
        ("//server", to("server")),
        ("\u{e4}:\\x\\.\\y", to("\u{e4}:/x/y")),
    ];
    let tidy = |text: &str, windows| {
        let tidied = tidy_path(text.as_bytes(), windows)?;
        Some(String::from_utf8(tidied).unwrap())
    };
    // `\` is no separator elsewhere.
    assert_eq!(tidy("\\dev\\null", false), to("\\dev\\null"));
    for (text, want) in windows {
        assert_eq!(tidy(text, true), want, "{text:?} on Windows");
    }
}

/// Each Windows answer as Git for Windows shows it for `GIT_CONFIG_SYSTEM`.
#[test]
fn paths_are_tidied_part_by_part() {
    let tidy = |text: &str, windows| {
        tidy_path(text.as_bytes(), windows).map(|t| String::from_utf8(t).unwrap())
    };
    for windows in [false, true] {
        for (text, want) in [
            ("a/../b", Some("b")),
            ("a/..", Some("")),
            ("./x", Some("x")),
            (".", Some("")),
            ("x/./y/", Some("x/y/")),
            ("a///b//", Some("a/b/")),
            ("/a/b/../", Some("/a/")),
            ("/./x", Some("/x")),
            ("/", Some("/")),
            ("...", Some("...")),
            (".a/..b", Some(".a/..b")),
            ("/..", None),
            ("../x", None),
            ("a/../..", None),
        ] {
            assert_eq!(tidy(text, windows).as_deref(), want, "{text:?}, {windows}");
        }
    }
    // A drive is any one character and `:`; a network root keeps its separators as they are.
    for (text, want) in [
        ("C:x/..", Some("C:")),
        ("C:x/../y", Some("C:y")),
        ("C://x", Some("C:/x")),
        ("C:..", None),
        ("1:x/..", Some("1:")),
        ("\u{20ac}:x/..", Some("\u{20ac}:")),
        ("/:x/..", Some("/:")),
        ("ab:/x/../y", Some("ab:/y")),
        (":/..", Some("")),
        ("//", Some("")),
        ("///", Some("///")),
        ("////x", Some("////x")),
        ("//x//y/z", Some("//x//y/z")),
        ("//x/y", Some("//x/y")),
        ("//x/y/z/..", Some("//x/y/")),
        ("//x/y/..", None),
        (r"\x\y\z\..\..", Some("/x/")),
    ] {
        assert_eq!(tidy(text, true).as_deref(), want, "{text:?}");
    }
    // Elsewhere only one leading `/` is the root, and `C:` is a name.
    assert_eq!(tidy("//x", false).as_deref(), Some("/x"));
    assert_eq!(tidy("C:x/..", false).as_deref(), Some(""));
}
