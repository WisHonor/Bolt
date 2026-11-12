use super::*;
use crate::names::{NameError, validate_name};

#[test]
fn every_opal_spelling_is_refused() {
    for bad in [
        ".opalro",
        ".OPALRO",
        ".Opalro",
        ".opalro.",
        ".opalro ",
        ".opalro. .",
        "opalro~1",
        "OPALRO~1",
        "opalro~4",
        ".opalro\\x",
        ".OPALRO.\\x",
        "opalro~1\\x",
        ".opalro::$INDEX_ALLOCATION",
        ".opalro:stream",
        ".o\u{200C}palro",
        "\u{FEFF}.opalro",
    ] {
        assert_eq!(validate_name(bad), Err(NameError::OpalSpelling), "{bad:?}");
        assert!(is_opal_alias(bad), "{bad:?}");
    }
    for ok in [
        ".opalroignore",
        "opalro.toml",
        ".opalros",
        "opalro",
        "opalro~5",
        "opalro~12",
        "x\\.opalro",
    ] {
        assert_eq!(validate_name(ok), Ok(()), "{ok}");
        assert!(!is_opal_alias(ok), "{ok}");
    }
}

#[test]
fn every_git_spelling_is_spotted() {
    // The spellings Windows or macOS take for `.git`: a stream, trailing dots and spaces,
    // invisible characters, and a part after a backslash.
    for name in [
        "GIT~1",
        ".git.",
        ".git ",
        ".git. .",
        ".git::$INDEX_ALLOCATION",
        "git~1 . :$DATA",
        ".g\u{200c}it",
        "\u{feff}.git",
        "x\\.git",
        "\\.git",
    ] {
        assert!(is_git_alias(name), "{name:?}");
    }
    // Git for Windows ends the HFS+ spelling at a backslash as well as at a slash, which only
    // shows once an ignorable character hides the spelling from the NTFS check.
    for name in [".git\u{200c}\\", "\u{200c}.git\\x", ".g\u{feff}it\\"] {
        assert!(is_git_alias(name), "{name:?}");
    }
    // Every character HFS+ ignores (Apple's TN1150 case-folding table maps them to nothing),
    // and the ones just outside each range, which it does not.
    let ignored = ('\u{200c}'..='\u{200f}')
        .chain('\u{202a}'..='\u{202e}')
        .chain('\u{206a}'..='\u{206f}')
        .chain(['\u{feff}']);
    for c in ignored {
        assert!(is_git_alias(&format!(".g{c}it")), "{c:?}");
    }
    for c in [
        '\u{200b}', '\u{2010}', '\u{2029}', '\u{202f}', '\u{2069}', '\u{2070}', '\u{fefe}',
        '\u{ff00}',
    ] {
        assert!(!is_git_alias(&format!(".g{c}it")), "{c:?}");
    }
    // Git takes U+FFFE or U+FFFF after `.git` as the end of the name.
    for name in [".git\u{fffe}", ".git\u{ffff}", ".git\u{200c}\u{fffe}"] {
        assert!(is_git_alias(name), "{name:?}");
    }
    for name in ["\u{fffe}.git", ".g\u{fffe}it"] {
        assert!(!is_git_alias(name), "{name:?}");
    }
    // Other names that begin like `.git`, and the ones `validate_name` refuses anyway.
    for name in [
        ".gitignore",
        ".gitmodules",
        ".git2",
        "xgit~1",
        "a.git",
        ".opalro",
        ".gi",
        "git~2",
        "git~10",
        "a\\b",
        "x\\.gitx",
        ".git\u{200c}x\\",
        "",
    ] {
        assert!(!is_git_alias(name), "{name:?}");
    }
}

#[test]
fn git_spellings_match_what_git_fsck_flags() {
    // Each checked with `git fsck`'s hasDotgit on a tree holding the name.
    for name in [
        ".git:",
        ".git.:x",
        ".git . :x",
        ".git:x.",
        "git~1:",
        "git~1\\",
        "x\\git~1.",
        "a:b\\.git",
        ".git\\\\",
        ".g\u{200c}it\\x",
        ".\u{200c}git",
        ".git\u{206e}",
    ] {
        assert!(is_git_alias(name), "{name:?}");
    }
    for name in [
        "x:.git",
        ".gitx:.git",
        "git~01",
        "git~1x",
        "git~1.x",
        " .git",
        ". git",
        ".git x",
        "..git",
        ".git~1",
        // HFS+ skips ignorables but not trailing dots or spaces, and only from the start.
        ".git\u{200c}.",
        ".git\u{200c} ",
        ".git\u{200c}:x",
        "git~1\u{200c}",
        "x\\.g\u{200c}it",
        ".git\u{200b}",
        ".git\u{2060}",
        ".git\u{a0}",
        ".g\u{130}t",
        ".\u{212a}IT",
    ] {
        assert!(!is_git_alias(name), "{name:?}");
    }
}

#[test]
fn short_names_of_gitmodules_match_what_git_fsck_flags() {
    // Each checked with `git fsck`'s gitmodulesSymlink on a link holding the name.
    let short =
        |name: &str| is_plain_short_name(name, b"gitmod") || is_hashed_short_name(name, b"gi7eba");
    for name in [
        "gitmod~1",
        "GITMOD~4",
        "GiTmOd~2",
        "gitmod~1 . ",
        "gitmod~1 :x",
        "gi7eba~1",
        "Gi7EbA~9",
        "gi7eba~1.",
        "gi7eba~1:",
        // A longer counter takes letters off the start, as NTFS does.
        "gi7eb~10",
        "gi7e~123",
        "~1234567",
        "gi7eb~10:x",
    ] {
        assert!(short(name), "{name:?}");
    }
    for name in [
        "gitmod~5",
        "gitmod~0",
        "gitmod~10",
        "gitmod~1x",
        "gitmod~1.x",
        "gitmo~1",
        "gi7eba~0",
        "gi7eba~10",
        "gi7eb~1",
        "gi7ebaa~1",
        "gi7ebb~1",
        "gitmod~1\u{200c}",
        ".gitmodules",
        "gi7eb~09",
        "gi7eb~100",
        "gitmod~1\\",
    ] {
        assert!(!short(name), "{name:?}");
    }
}

#[test]
fn a_slash_ends_a_part_but_a_backslash_starts_another() {
    for name in [".git/", "GIT~1/x", "/\\.git", "./\\GIT~1", "\t/0\\.Git"] {
        assert!(is_git_alias(name), "{name:?}");
    }
    for name in ["x/.git", "x/.git\\y"] {
        assert!(!is_git_alias(name), "{name:?}");
    }
}

#[test]
fn ordinary_names_are_answered_at_once() {
    for name in ["Player.cs", "README", "src", "名前", "x\\y"] {
        assert!(!is_git_alias(name), "{name:?}");
        assert!(!is_opal_alias(name), "{name:?}");
    }
}
