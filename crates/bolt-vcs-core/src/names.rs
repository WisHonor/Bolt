//! Which file names Bolt stores, and which it flags as non-portable (decision 004).
//!
//! The always-on rules ([`validate_name`]) refuse a name outright. The portability
//! checks ([`name_problems`], [`folder_problems`]) describe names that work here but
//! would break on another operating system.

use std::collections::BTreeMap;
use unicode_normalization::UnicodeNormalization;

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

/// A name that is stored fine here but would break on another system.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum PortabilityProblem {
    #[error(
        "\"{a}\" and \"{b}\" differ only in upper/lower case, so they clash on Windows and macOS"
    )]
    CaseCollision { a: String, b: String },
    #[error("\"{a}\" and \"{b}\" are the same name written two ways, so they clash on macOS")]
    NormalizationCollision { a: String, b: String },
    #[error("\"{0}\" is a reserved device name on Windows")]
    ReservedName(String),
    #[error("\"{0}\" ends with a dot or a space, which Windows does not allow")]
    TrailingDotOrSpace(String),
    #[error("\"{name}\" contains {ch:?}, which Windows does not allow in names")]
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

fn is_bolt_spelling(name: &str) -> bool {
    let kept: String = name.chars().filter(|c| !is_ignorable(*c)).collect();
    // NTFS ignores everything after ':' (streams such as `::$INDEX_ALLOCATION`), and a
    // backslash starts a sub-path on Windows (Git's is_ntfs_dotgit stops at both).
    let base = kept.split([':', '\\']).next().unwrap_or("");
    let base = base.trim_end_matches(['.', ' ']).to_ascii_lowercase();
    // `bolt~1`..`bolt~4` are NTFS 8.3 short names of `.bolt` (Git accepts ~1..~4).
    base == ".bolt" || matches!(base.as_bytes(), [b'b', b'o', b'l', b't', b'~', b'1'..=b'4'])
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
    // COM0-9 / LPT0-9 and the superscript digits Windows also treats as digits.
    ["COM", "LPT"].iter().any(|p| {
        up.strip_prefix(p).is_some_and(|d| {
            let mut it = d.chars();
            matches!(
                (it.next(), it.next()),
                (Some('0'..='9' | '\u{B9}' | '\u{B2}' | '\u{B3}'), None)
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
            "COM0",
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
        for fine in ["Player.cs", "console.txt", "COM10", "aux_data", "nul2"] {
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
}
