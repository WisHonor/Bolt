//! Which file names Bolt stores, and which it flags as non-portable (decision 004):
//! [`validate_name`] refuses a name outright, and [`validate_path`] a path; [`name_problems`] and
//! [`folder_problems`] flag names that work here but would break on another operating system.
//! [`is_git_alias`] and [`is_bolt_alias`] spot the spellings of `.git` and `.bolt`, [`own_folder`]
//! the project's own ones, [`holdable_here`] says whether this system can hold a name,
//! [`writable_here`] whether write-out may write one, and [`fold`] compares names as a drive does.
//! The limits on names and paths are here too (decision 051).

use std::collections::BTreeMap;
use unicode_normalization::UnicodeNormalization;

use crate::drive::Drive;
use crate::text::escape;

/// Longest name the format allows, in bytes: Windows' 255 UTF-16 units, each at most three bytes
/// in UTF-8.
pub const MAX_NAME_BYTES: usize = 765;

/// Longest name a save may add, in bytes: what macOS and Linux hold.
pub const MAX_NEW_NAME_BYTES: usize = 255;

/// Longest path, in bytes, counted from the project's top with `/` between names.
pub const MAX_PATH_BYTES: usize = 4096;

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
    #[error("a path cannot be longer than {} bytes", MAX_PATH_BYTES)]
    PathTooLong,
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
    #[error(
        "\"{}\" is longer than {} bytes, which macOS and Linux do not allow",
        escape(.0, false),
        MAX_NEW_NAME_BYTES
    )]
    TooLong(String),
}

/// Why write-out won't write a name on this system.
#[derive(Debug, PartialEq, Eq)]
pub enum NotWritable {
    /// Not a name at all ([`validate_name`]).
    Invalid(NameError),
    /// A spelling of `.git` ([`is_git_alias`]).
    GitName,
    /// A spelling of `.bolt` ([`is_bolt_alias`]).
    BoltName,
    /// A name this system can't hold, by decision 004's rules for it.
    NotHoldable(PortabilityProblem),
    /// Longer than this system holds ([`fits_here`]).
    TooLong,
    /// The real short name of a `.git` or `.bolt` in the same folder.
    ShortNameOf(&'static str),
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
    if is_bolt_alias(name) {
        return Err(NameError::BoltSpelling);
    }
    Ok(())
}

/// The always-on rules for a project path: `/` between names, each one [`validate_name`] allows,
/// and at most [`MAX_PATH_BYTES`] in all. So no empty name, no `..`, and no `/` at either end.
pub fn validate_path(path: &str) -> Result<(), NameError> {
    if path.len() > MAX_PATH_BYTES {
        return Err(NameError::PathTooLong);
    }
    path.split('/').try_for_each(validate_name)
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

/// Whether Windows or macOS would take `name` for Bolt's own folder, `.bolt`: any letter case,
/// trailing dots or spaces, a `:stream`, the short names `bolt~1` to `bolt~4`, or characters macOS
/// ignores. It answers the same on every system.
pub fn is_bolt_alias(name: &str) -> bool {
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

/// Whether `name`, in the project's top folder, is the project's own `.bolt` or Git's `.git` as
/// this drive spells them: exactly, or in other letter case where the drive ignores it (decision
/// 059). Other spellings of either are left out of saves with a warning instead.
pub fn own_folder(name: &str, drive: Drive) -> bool {
    [".bolt", ".git"]
        .iter()
        .any(|own| name == *own || (drive.ignores_case && name.eq_ignore_ascii_case(own)))
}

fn is_ntfs_dotgit(name: &str) -> bool {
    let base = ntfs_base(name);
    // `git~1` is the 8.3 short name `.git` gets.
    base.eq_ignore_ascii_case(".git") || base.eq_ignore_ascii_case("git~1")
}

/// HFS+ skips the ignorable characters wherever they are and folds the case of ASCII letters only.
/// The name ends at `/`, and at `\` as it does in Git for Windows. Git's UTF-8 reader refuses
/// U+FFFE and U+FFFF, and it takes such a character for the name's end too.
fn is_hfs_dotgit(name: &str) -> bool {
    let mut chars = name.chars().filter(|&c| !is_ignorable(c));
    chars.next() == Some('.')
        && ['g', 'i', 't']
            .into_iter()
            .all(|g| chars.next().is_some_and(|c| c.eq_ignore_ascii_case(&g)))
        && matches!(
            chars.next(),
            None | Some('/' | '\\' | '\u{fffe}' | '\u{ffff}')
        )
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

/// Portability problems with a single name: Windows' rules, and the length every system holds.
pub fn name_problems(name: &str) -> Vec<PortabilityProblem> {
    let mut out = windows_problems(name);
    if name.len() > MAX_NEW_NAME_BYTES {
        out.push(PortabilityProblem::TooLong(name.into()));
    }
    out
}

/// What Windows refuses in a name: device names, a dot or space at its end, and characters.
fn windows_problems(name: &str) -> Vec<PortabilityProblem> {
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
/// it must [fit](fits_here), and pass decision 004's portability rules that apply here, which only
/// Windows has. Names that clash on the drive are `drive::probe`'s business.
pub fn holdable_here(name: &str) -> bool {
    fits_here(name) && (!cfg!(windows) || windows_problems(name).is_empty())
}

/// Whether this system holds a name this long: Windows holds 255 UTF-16 units, up to 765 bytes,
/// and macOS and Linux 255 bytes (decision 051).
pub fn fits_here(name: &str) -> bool {
    if cfg!(windows) {
        name.encode_utf16().count() <= 255
    } else {
        name.len() <= MAX_NEW_NAME_BYTES
    }
}

/// Whether write-out may write `name` into a folder whose entries have the long and short names
/// `short_names_here` (Windows' 8.3 names, as `Dir::entries` gives them). Never a spelling of
/// `.git` or `.bolt`, on any system: Linux reaches NTFS and vfat drives too (CVE-2019-1353). Nor
/// a name this system can't hold, nor the real short name of a `.git` or `.bolt` in that folder.
/// Nor anything [`validate_name`] refuses, on any system.
pub fn writable_here(name: &str, short_names_here: &[(&str, &str)]) -> Result<(), NotWritable> {
    match validate_name(name) {
        Err(NameError::BoltSpelling) => return Err(NotWritable::BoltName),
        Err(e) => return Err(NotWritable::Invalid(e)),
        Ok(()) => {}
    }
    if is_git_alias(name) {
        return Err(NotWritable::GitName);
    }
    if is_bolt_alias(name) {
        return Err(NotWritable::BoltName);
    }
    if cfg!(windows)
        && let Some(problem) = windows_problems(name).into_iter().next()
    {
        return Err(NotWritable::NotHoldable(problem));
    }
    if !fits_here(name) {
        return Err(NotWritable::TooLong);
    }
    // After four clashes NTFS hashes a short name (`GI7EBA~1`), so the real one is compared.
    for (long, short) in short_names_here {
        if short.eq_ignore_ascii_case(name) {
            if is_git_alias(long) {
                return Err(NotWritable::ShortNameOf(".git"));
            }
            if is_bolt_alias(long) {
                return Err(NotWritable::ShortNameOf(".bolt"));
            }
        }
    }
    Ok(())
}

/// `name` as a drive compares names: composed (NFC) where it composes Unicode, then lowercase
/// where it ignores letter case. Both what a save refuses and what it keeps fold names this way.
pub fn fold(name: &str, compose: bool, ignore_case: bool) -> String {
    let name: String = if compose {
        name.nfc().collect()
    } else {
        name.to_owned()
    };
    // shortcut: Unicode lowercase approximates NTFS's uppercase table; upgrade if it matters.
    if ignore_case {
        name.to_lowercase()
    } else {
        name
    }
}

/// Collisions among the names in one folder, plus each name's own problems.
pub fn folder_problems(names: &[&str]) -> Vec<PortabilityProblem> {
    let mut out: Vec<_> = names.iter().flat_map(|n| name_problems(n)).collect();
    let mut by_nfc: BTreeMap<String, &str> = BTreeMap::new();
    let mut by_fold: BTreeMap<String, &str> = BTreeMap::new();
    for &n in names {
        let nfc = fold(n, true, false);
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
        let folded = fold(&nfc, false, true);
        if let Some(&first) = by_fold.get(&folded) {
            out.push(PortabilityProblem::CaseCollision {
                a: first.into(),
                b: n.into(),
            });
        } else {
            by_fold.insert(folded, n);
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
        assert_eq!(validate_name(&"x".repeat(766)), Err(NameError::TooLong));
        assert_eq!(validate_name(&"x".repeat(765)), Ok(()));
    }

    #[test]
    fn names_up_to_765_bytes_decode_and_fit_by_this_systems_rule() {
        let cjk = "語".repeat(200); // 600 bytes, 200 UTF-16 units
        assert!(validate_name(&cjk).is_ok());
        assert_eq!(fits_here(&cjk), cfg!(windows));
        assert!(validate_name(&"a".repeat(766)).is_err());
        // 255 of either unit fit everywhere; one more fits nowhere.
        assert!(fits_here(&"a".repeat(255)) && !fits_here(&"a".repeat(256)));
        // Four bytes and two UTF-16 units each: 127 fit Windows (254 units), 63 fit 255 bytes.
        let emoji = |n| "\u{1f600}".repeat(n);
        assert!(fits_here(&emoji(63)));
        assert_eq!(fits_here(&emoji(64)), cfg!(windows));
        assert!(!fits_here(&emoji(128)));
    }

    #[test]
    fn a_new_name_over_255_bytes_is_not_portable_and_holdable_where_it_fits() {
        let cjk = "語".repeat(200);
        assert_eq!(
            name_problems(&cjk),
            [PortabilityProblem::TooLong(cjk.clone())]
        );
        assert!(name_problems(&"a".repeat(255)).is_empty());
        // Holding it is this system's own question: Windows holds 255 UTF-16 units.
        assert_eq!(holdable_here(&cjk), cfg!(windows));
        assert!(!holdable_here(&"a".repeat(256)));
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
            assert!(is_bolt_alias(bad), "{bad:?}");
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
            assert!(!is_bolt_alias(ok), "{ok}");
        }
    }

    #[test]
    fn the_projects_own_folders_are_spelled_as_the_drive_spells_them() {
        let drive = |ignores_case| Drive {
            ignores_case,
            composes_unicode: false,
        };
        for ignores_case in [false, true] {
            for name in [".bolt", ".git"] {
                assert!(own_folder(name, drive(ignores_case)), "{name}");
            }
            for name in [".BOLT", ".Git", ".gIT"] {
                assert_eq!(
                    own_folder(name, drive(ignores_case)),
                    ignores_case,
                    "{name}"
                );
            }
            // Other spellings are left out of saves instead, and the rest are other names.
            for name in [
                ".bolt.",
                "bolt~1",
                "git~1",
                ".git ",
                ".g\u{200c}it",
                ".gitignore",
                "bolt",
            ] {
                assert!(!own_folder(name, drive(ignores_case)), "{name:?}");
            }
        }
    }

    #[test]
    fn writable_here_refuses_every_spelling_of_git_and_bolt_on_every_system() {
        for n in [
            ".git",
            ".GIT",
            "git~1",
            ".git.",
            ".bolt",
            ".BOLT",
            "bolt~1",
            ".bolt::$INDEX_ALLOCATION",
            ".g\u{200c}it",
        ] {
            assert!(writable_here(n, &[]).is_err(), "{n:?}");
        }
        assert_eq!(writable_here(".Git", &[]), Err(NotWritable::GitName));
        assert_eq!(writable_here("BOLT~2", &[]), Err(NotWritable::BoltName));
        for n in ["Player.cs", ".gitignore", "bolt.toml", "名前"] {
            assert_eq!(writable_here(n, &[]), Ok(()), "{n:?}");
        }
    }

    #[test]
    fn writable_here_refuses_whatever_validate_name_refuses() {
        for n in ["", ".", "..", "a/b", "a\0b", &"x".repeat(766)] {
            assert!(validate_name(n).is_err(), "{n:?}");
            assert!(writable_here(n, &[]).is_err(), "{n:?}");
        }
        assert_eq!(
            writable_here("..", &[]),
            Err(NotWritable::Invalid(NameError::DotOrDotDot))
        );
    }

    #[test]
    fn a_real_short_name_of_git_is_refused() {
        assert!(writable_here("GI7EBA~1", &[(".git", "GI7EBA~1")]).is_err());
        // Ignoring case, and for `.bolt` and other spellings of either too.
        let here = [
            (".git", "GI7EBA~1"),
            (".BOLT", "BOC2A3~1"),
            ("Player.cs", "PLAYER~1.CS"),
        ];
        assert_eq!(
            writable_here("gi7eba~1", &here),
            Err(NotWritable::ShortNameOf(".git"))
        );
        assert_eq!(
            writable_here("BOC2A3~1", &here),
            Err(NotWritable::ShortNameOf(".bolt"))
        );
        // Another entry's short name is write-out's to sort out as a clash, not a name to refuse.
        assert_eq!(writable_here("PLAYER~1.CS", &here), Ok(()));
    }

    #[test]
    fn writable_here_refuses_names_this_system_cant_hold() {
        let long = "語".repeat(200);
        assert_eq!(
            writable_here(&long, &[]).is_ok(),
            cfg!(windows),
            "a 600-byte name"
        );
        assert_eq!(
            writable_here(&"a".repeat(256), &[]),
            Err(NotWritable::TooLong)
        );
        for n in ["a\\b", "c:d", "aux.txt", "name."] {
            assert_eq!(writable_here(n, &[]).is_ok(), !cfg!(windows), "{n:?}");
        }
    }

    #[cfg(windows)]
    #[test]
    fn windows_refuses_separators_devices_and_trailing_dots() {
        for n in [
            "a\\b",
            "c:d",
            "con",
            "aux.txt",
            "COM\u{b9}",
            "LPT\u{b3}.log",
            "name.",
            "name ",
        ] {
            assert!(writable_here(n, &[]).is_err(), "{n:?}");
        }
        assert!(matches!(
            writable_here("con", &[]),
            Err(NotWritable::NotHoldable(PortabilityProblem::ReservedName(
                _
            )))
        ));
    }

    #[cfg(windows)]
    #[test]
    fn com0_and_lpt0_are_ordinary_names_on_windows() {
        // Microsoft's list of reserved names doesn't hold them, and Windows 11 creates them.
        for n in ["COM0", "LPT0", "COM0.txt", "LPT0.txt"] {
            assert_eq!(writable_here(n, &[]), Ok(()), "{n}");
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
        // Git for Windows ends the HFS+ spelling at a backslash as well as at a slash, which only
        // shows once an ignorable character hides the spelling from the NTFS check.
        for name in [".git\u{200c}\\", "\u{200c}.git\\x", ".g\u{feff}it\\"] {
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
            ".git\u{200c}x\\",
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
            PortabilityProblem::TooLong(name()),
        ] {
            assert_hostile_text_escaped(&problem.to_string());
        }
    }
}
