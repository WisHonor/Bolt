//! Which file names Bolt stores, and which it flags as non-portable (decision 004):
//! [`validate_name`] refuses a name outright; [`name_problems`] and [`folder_problems`] flag names
//! that work here but would break on another operating system. [`is_git_alias`] spots the
//! spellings of `.git`, and [`holdable_here`] says whether this system can hold a name.

use std::collections::BTreeMap;
use unicode_normalization::UnicodeNormalization;

use crate::text::escape;

/// Longest allowed name, in bytes.
pub const MAX_NAME_BYTES: usize = 255;

/// Why a name is refused on every system.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum NameError {
    #[error("a name cannot be empty")]
    Empty,
    #[error("a name cannot be \".\" or \"..\"")]
    DotOrDotDot,
    #[error("a name cannot contain a slash")]
    Slash,
    #[error("a name cannot contain a NUL character")]
    Nul,
    #[error("a name cannot be longer than {} bytes", MAX_NAME_BYTES)]
    TooLong,
    #[error("a name cannot be spelled like Bolt's own folder (.bolt)")]
    BoltSpelling,
}

/// A name that is stored fine here but would break on another system. The names in the messages
/// came from outside Bolt and are shown escaped.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum PortabilityProblem {
    #[error(
        "\"{}\" and \"{}\" differ only in upper/lower case, so they clash on Windows and macOS",
        escape(a, false),
        escape(b, false)
    )]
    CaseCollision { a: String, b: String },
    #[error(
        "\"{}\" and \"{}\" are the same name written two ways, so they clash on macOS",
        escape(a, false),
        escape(b, false)
    )]
    NormalizationCollision { a: String, b: String },
    #[error("\"{}\" is a reserved device name on Windows", escape(.0, false))]
    ReservedName(String),
    #[error(
        "\"{}\" ends with a dot or a space, which Windows does not allow",
        escape(.0, false)
    )]
    TrailingDotOrSpace(String),
    #[error(
        "\"{}\" contains {ch:?}, which Windows does not allow in names",
        escape(name, false)
    )]
    ForbiddenCharacter { name: String, ch: char },
}

/// The always-on rules: what no name may ever be.
pub fn validate_name(name: &str) -> Result<(), NameError> {
    if name.is_empty() {
        return Err(NameError::Empty);
    }
    if name == "." || name == ".." {
        return Err(NameError::DotOrDotDot);
    }
    if name.contains('/') {
        return Err(NameError::Slash);
    }
    if name.contains('\0') {
        return Err(NameError::Nul);
    }
    if name.len() > MAX_NAME_BYTES {
        return Err(NameError::TooLong);
    }
    if is_bolt_spelling(name) {
        return Err(NameError::BoltSpelling);
    }
    Ok(())
}

/// Characters macOS (HFS+) ignores when comparing names; Git's `next_hfs_char` list.
fn is_ignorable(c: char) -> bool {
    matches!(c, '\u{200C}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{206A}'..='\u{206F}' | '\u{FEFF}')
}

/// What NTFS takes a name to be: the part before a `:` (streams such as `::$INDEX_ALLOCATION`)
/// or a path separator (a backslash starts a sub-path on Windows), without the trailing dots and
/// spaces it drops. Git's `is_ntfs_dotgit` stops at the same places.
fn ntfs_base(name: &str) -> &str {
    let base = name.split([':', '/', '\\']).next().unwrap_or("");
    base.trim_end_matches(['.', ' '])
}

fn is_bolt_spelling(name: &str) -> bool {
    let kept: String = name.chars().filter(|c| !is_ignorable(*c)).collect();
    let base = ntfs_base(&kept).to_ascii_lowercase();
    // `bolt~1`..`bolt~4` are NTFS 8.3 short names of `.bolt` (Git accepts ~1..~4).
    base == ".bolt" || matches!(base.as_bytes(), [b'b', b'o', b'l', b't', b'~', b'1'..=b'4'])
}

/// Whether Windows or macOS would take `name` for Git's own folder, `.git`. This is Git's
/// `is_ntfs_dotgit` and `is_hfs_dotgit`, plus the look its fsck takes at every part after a
/// backslash, and it answers the same on every system.
pub fn is_git_alias(name: &str) -> bool {
    is_hfs_dotgit(name) || name.split('\\').any(is_ntfs_dotgit)
}

fn is_ntfs_dotgit(name: &str) -> bool {
    let base = ntfs_base(name);
    // `git~1` is the 8.3 short name `.git` gets.
    base.eq_ignore_ascii_case(".git") || base.eq_ignore_ascii_case("git~1")
}

/// HFS+ skips the ignorable characters wherever they are and folds the case of ASCII letters only.
/// Git's UTF-8 reader refuses U+FFFE and U+FFFF, and it takes such a character for the name's end.
fn is_hfs_dotgit(name: &str) -> bool {
    let mut chars = name.chars().filter(|&c| !is_ignorable(c));
    chars.next() == Some('.')
        && ['g', 'i', 't']
            .into_iter()
            .all(|g| chars.next().is_some_and(|c| c.eq_ignore_ascii_case(&g)))
        && matches!(chars.next(), None | Some('/' | '\u{fffe}' | '\u{ffff}'))
}

fn is_reserved(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or("").trim_end_matches(' ');
    let up = stem.to_ascii_uppercase();
    if matches!(
        up.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) {
        return true;
    }
    // Microsoft's "Naming Files, Paths, and Namespaces" page lists COM1-COM9 and LPT1-LPT9, and
    // the same names with a superscript 1, 2 or 3 (U+00B9, U+00B2, U+00B3) as the digit. COM0
    // and LPT0 are not on that list, and Windows 11 creates them as ordinary files, so they pass.
    ["COM", "LPT"].iter().any(|p| {
        up.strip_prefix(p).is_some_and(|d| {
            let mut it = d.chars();
            matches!(
                (it.next(), it.next()),
                (Some('1'..='9' | '\u{B9}' | '\u{B2}' | '\u{B3}'), None)
            )
        })
    })
}

/// Portability problems with a single name.
pub fn name_problems(name: &str) -> Vec<PortabilityProblem> {
    let mut out = Vec::new();
    if is_reserved(name) {
        out.push(PortabilityProblem::ReservedName(name.into()));
    }
    if name.ends_with(['.', ' ']) {
        out.push(PortabilityProblem::TrailingDotOrSpace(name.into()));
    }
    if let Some(ch) = name
        .chars()
        .find(|c| matches!(c, '<' | '>' | ':' | '"' | '\\' | '|' | '?' | '*') || *c < ' ')
    {
        out.push(PortabilityProblem::ForbiddenCharacter {
            name: name.into(),
            ch,
        });
    }
    out
}

/// Whether this operating system can hold a file called `name` (a name `validate_name` accepts):
/// decision 004's portability rules that apply here, which only Windows has. Names that clash on
/// the drive are `drive::probe`'s business.
pub fn holdable_here(name: &str) -> bool {
    !cfg!(windows) || name_problems(name).is_empty()
}

/// Collisions among the names in one folder, plus each name's own problems.
pub fn folder_problems(names: &[&str]) -> Vec<PortabilityProblem> {
    let mut out: Vec<_> = names.iter().flat_map(|n| name_problems(n)).collect();
    let mut by_nfc: BTreeMap<String, &str> = BTreeMap::new();
    let mut by_fold: BTreeMap<String, &str> = BTreeMap::new();
    for &n in names {
        let nfc: String = n.nfc().collect();
        match by_nfc.get(&nfc) {
            Some(&first) if first != n => {
                out.push(PortabilityProblem::NormalizationCollision {
                    a: first.into(),
                    b: n.into(),
                });
                continue;
            }
            Some(_) => continue,
            None => {
                by_nfc.insert(nfc.clone(), n);
            }
        }
        // shortcut: Unicode lowercase approximates NTFS's uppercase table; upgrade if it matters.
        let fold = nfc.to_lowercase();
        if let Some(&first) = by_fold.get(&fold) {
            out.push(PortabilityProblem::CaseCollision {
                a: first.into(),
                b: n.into(),
            });
        } else {
            by_fold.insert(fold, n);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn always_on_rules() {
        for ok in [
            "Player.cs",
            "README",
            "a b",
            "aux.c",
            "a:b",
            "a\\b",
            "\u{e9}",
            "名前",
        ] {
            assert_eq!(validate_name(ok), Ok(()), "{ok}");
        }
        assert_eq!(validate_name(""), Err(NameError::Empty));
        assert_eq!(validate_name("."), Err(NameError::DotOrDotDot));
        assert_eq!(validate_name(".."), Err(NameError::DotOrDotDot));
        assert_eq!(validate_name("a/b"), Err(NameError::Slash));
        assert_eq!(validate_name("a\0b"), Err(NameError::Nul));
        assert_eq!(validate_name(&"x".repeat(256)), Err(NameError::TooLong));
        assert_eq!(validate_name(&"x".repeat(255)), Ok(()));
    }

    #[test]
    fn every_bolt_spelling_is_refused() {
        for bad in [
            ".bolt",
            ".BOLT",
            ".Bolt",
            ".bolt.",
            ".bolt ",
            ".bolt. .",
            "bolt~1",
            "BOLT~1",
            "bolt~4",
            ".bolt\\x",
            ".BOLT.\\x",
            "bolt~1\\x",
            ".bolt::$INDEX_ALLOCATION",
            ".bolt:stream",
            ".b\u{200C}olt",
            "\u{FEFF}.bolt",
        ] {
            assert_eq!(validate_name(bad), Err(NameError::BoltSpelling), "{bad:?}");
        }
        for ok in [
            ".boltignore",
            "bolt.toml",
            ".bolts",
            "bolt",
            "bolt~5",
            "bolt~12",
        ] {
            assert_eq!(validate_name(ok), Ok(()), "{ok}");
        }
    }

    #[test]
    fn single_name_portability() {
        for reserved in [
            "CON",
            "con",
            "con.txt",
            "NUL",
            "AUX",
            "PRN",
            "COM1",
            "com9.log",
            "LPT1",
            "CONIN$",
            "CONOUT$",
            "COM\u{b9}",
            "lpt\u{b3}.txt",
            "nul .txt",
        ] {
            assert!(
                matches!(
                    name_problems(reserved).as_slice(),
                    [PortabilityProblem::ReservedName(_)]
                ),
                "{reserved}"
            );
        }
        assert!(matches!(
            name_problems("name.").as_slice(),
            [PortabilityProblem::TrailingDotOrSpace(_)]
        ));
        assert!(matches!(
            name_problems("name ").as_slice(),
            [PortabilityProblem::TrailingDotOrSpace(_)]
        ));
        for (name, ch) in [
            ("a:b", ':'),
            ("a\\b", '\\'),
            ("a?b", '?'),
            ("a*b", '*'),
            ("a|b", '|'),
            ("a<b", '<'),
            ("a>b", '>'),
            ("a\"b", '"'),
            ("a\u{1}b", '\u{1}'),
        ] {
            assert!(
                name_problems(name).contains(&PortabilityProblem::ForbiddenCharacter {
                    name: name.into(),
                    ch
                }),
                "{name}"
            );
        }
        for fine in [
            "Player.cs",
            "console.txt",
            "COM10",
            "aux_data",
            "nul2",
            "COM0",
            "LPT0",
        ] {
            assert!(name_problems(fine).is_empty(), "{fine}");
        }
    }

    #[test]
    fn sibling_collisions() {
        assert!(
            folder_problems(&["README", "readme"])
                .iter()
                .any(|p| matches!(p, PortabilityProblem::CaseCollision { .. }))
        );
        assert!(
            folder_problems(&["\u{e9}", "e\u{301}"])
                .iter()
                .any(|p| matches!(p, PortabilityProblem::NormalizationCollision { .. }))
        );
        assert!(folder_problems(&["a", "b", "A.txt"]).is_empty());
        assert!(!folder_problems(&["CON", "a"]).is_empty());
    }

    #[test]
    fn git_aliases_match_gits_own_lists() {
        // Git's t1014-read-tree-confusing.sh: the spellings its read-tree refuses, plus more of
        // the same kind (a stream, trailing dots and spaces, invisible characters).
        for name in [
            ".git",
            ".GIT",
            "\u{200c}.Git",
            ".gI\u{200c}T",
            ".GiT\u{200c}",
            "git~1",
            "GIT~1",
            ".git. ",
            ".git.",
            ".git ",
            ".git. .",
            ".\\.GIT\\foobar",
            ".git\\foobar",
            ".git...:alternate-stream",
            ".git::$INDEX_ALLOCATION",
            "git~1 . :$DATA",
            ".g\u{200c}it",
            "\u{feff}.git",
        ] {
            assert!(is_git_alias(name), "{name:?}");
        }
        // The trailing dots and spaces that Git's t0060-path-utils.sh allows after `.gitmodules`.
        for base in [".git", ".Git", "git~1", "GIT~1"] {
            for tail in [
                "", " ", ".", "  ", ". ", " .", "..", "   ", ".  ", " . ", "  .", ":$DATA",
            ] {
                let name = format!("{base}{tail}");
                assert!(is_git_alias(&name), "{name:?}");
            }
        }
        // `next_hfs_char` in Git's utf8.c: every character HFS+ ignores, and the ones just
        // outside each range, which it does not.
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
        // Git's UTF-8 reader refuses U+FFFE and U+FFFF, which ends the name after `.git`.
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
            "git",
            ".git2",
            "xgit~1",
            ".bolt",
            ".git x",
            ".git .x",
            " .git",
            "..git",
            ".gi",
            ".git,:$DATA",
            "git~2",
            "git~10",
            "a\\b",
            "x\\.gitx",
            ".",
            "..",
            "",
        ] {
            assert!(!is_git_alias(name), "{name:?}");
        }
    }

    #[test]
    fn holdable_here_follows_this_systems_rules() {
        // Windows can't hold these; the other systems can.
        for name in [
            "aux.txt",
            "con",
            "NUL",
            "COM1",
            "COM\u{b9}",
            "lpt\u{b3}.txt",
            "name.",
            "name ",
            "a:b",
            "a\\b",
            "a?b",
            "a*b",
            "a|b",
            "a<b",
            "a>b",
            "a\"b",
            "a\u{1}b",
        ] {
            assert_eq!(holdable_here(name), !cfg!(windows), "{name:?}");
        }
        for name in [
            "Player.cs",
            "console.txt",
            "COM10",
            "COM0",
            "aux_data",
            "\u{e9}",
            "名前",
            ".gitignore",
        ] {
            assert!(holdable_here(name), "{name:?}");
        }
    }

    #[test]
    fn problems_show_the_names_escaped() {
        use crate::test_support::{HOSTILE, assert_hostile_text_escaped};
        let name = || HOSTILE.to_owned();
        for problem in [
            PortabilityProblem::CaseCollision {
                a: name(),
                b: name(),
            },
            PortabilityProblem::NormalizationCollision {
                a: name(),
                b: name(),
            },
            PortabilityProblem::ReservedName(name()),
            PortabilityProblem::TrailingDotOrSpace(name()),
            PortabilityProblem::ForbiddenCharacter {
                name: name(),
                ch: '*',
            },
        ] {
            assert_hostile_text_escaped(&problem.to_string());
        }
    }
}
