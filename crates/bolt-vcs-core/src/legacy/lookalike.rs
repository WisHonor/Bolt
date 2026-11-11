//! Whether Windows or macOS would take a name for `.git`, `.gitmodules` or `.bolt`.

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
    // The characters macOS ignores are all outside ASCII, so an ASCII first byte starts the base.
    if starts_outside(name, b".bB") {
        return false;
    }
    #[cfg(test)]
    tests::count_full_check();
    // `ntfs_base` of the name without its ignorable characters, in ASCII lowercase; both
    // spellings below are at most 6 characters, so a longer base is neither.
    let mut base = ['\0'; 6];
    let mut end = 0;
    let chars = name.chars().filter(|&c| !is_ignorable(c));
    for (at, c) in chars
        .take_while(|c| !matches!(c, ':' | '/' | '\\'))
        .enumerate()
    {
        let trailing = c == '.' || c == ' ';
        match base.get_mut(at) {
            Some(slot) => *slot = c.to_ascii_lowercase(),
            None if trailing => continue,
            None => return false,
        }
        if !trailing {
            end = at + 1;
        }
    }
    let base = base.get(..end).unwrap_or_default();
    // `bolt~1`..`bolt~4` are NTFS 8.3 short names of `.bolt` (Git accepts ~1..~4).
    base == ['.', 'b', 'o', 'l', 't'] || matches!(base, ['b', 'o', 'l', 't', '~', '1'..='4'])
}

/// Whether Windows or macOS would take `name` for Git's own folder, `.git`. This is Git's
/// `is_ntfs_dotgit` and `is_hfs_dotgit`, plus the look its fsck takes at every part after a
/// backslash, and it answers the same on every system.
pub fn is_git_alias(name: &str) -> bool {
    // Without a backslash the name is its only part, and both checks need `.`, `g` or `G` first.
    if starts_outside(name, b".gG") && !name.contains('\\') {
        return false;
    }
    #[cfg(test)]
    tests::count_full_check();
    is_hfs_dotgit(name) || name.split('\\').any(is_ntfs_dotgit)
}

/// Whether `name` starts with an ASCII byte that isn't one of `firsts`.
fn starts_outside(name: &str, firsts: &[u8]) -> bool {
    name.as_bytes()
        .first()
        .is_some_and(|b| b.is_ascii() && !firsts.contains(b))
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

/// Whether `name` is an NTFS fallback short name made from `prefix` the way Git's
/// `is_ntfs_dot_generic` spells one: up to 6 of its letters, `~`, a digit 1-9, more digits to 8
/// characters, then only dots, spaces or a `:stream`. `gi7eba` is `.gitmodules`' prefix, which
/// Git refuses only for a link named so; write-out refuses it for every entry, on purpose, since
/// a file there would stand for `.gitmodules` too. `.git` and `.bolt` have no such prefix known
/// ahead (NTFS hashes the name), so their real short names are compared folder by folder instead.
pub(crate) fn is_hashed_short_name(name: &str, prefix: &[u8]) -> bool {
    let bytes = name.as_bytes();
    let mut pos = 0;
    let mut saw_tilde = false;
    while pos < 8 {
        let Some(&b) = bytes.get(pos) else {
            return false;
        };
        if saw_tilde {
            if !b.is_ascii_digit() {
                return false;
            }
        } else if b == b'~' {
            saw_tilde = true;
            pos += 1;
            if !bytes.get(pos).is_some_and(|d| (b'1'..=b'9').contains(d)) {
                return false;
            }
        } else if pos >= 6 || !prefix.get(pos).is_some_and(|p| b.eq_ignore_ascii_case(p)) {
            return false;
        }
        pos += 1;
    }
    only_dots_or_spaces(bytes.get(pos..).unwrap_or_default())
}

/// Whether `name` is the plain short name NTFS gives a name starting with `prefix`'s 6 letters
/// before it runs out of them, as Git's `is_ntfs_dot_generic` reads it: those letters in any
/// case, `~`, a digit 1-4, then only dots, spaces or a `:stream`. `gitmod` gives `.gitmodules`'
/// `GITMOD~1`; write-out refuses it for every entry, as it does the hashed ones.
pub(crate) fn is_plain_short_name(name: &str, prefix: &[u8; 6]) -> bool {
    let Some((head, rest)) = name.as_bytes().split_at_checked(6) else {
        return false;
    };
    let [b'~', b'1'..=b'4', rest @ ..] = rest else {
        return false;
    };
    head.eq_ignore_ascii_case(prefix) && only_dots_or_spaces(rest)
}

/// Whether what ends a short name holds only dots or spaces, up to a `:stream` if any: NTFS
/// drops those, so the name is the short name still.
fn only_dots_or_spaces(rest: &[u8]) -> bool {
    let end = rest.iter().position(|&b| b == b':').unwrap_or(rest.len());
    rest.get(..end)
        .unwrap_or_default()
        .iter()
        .all(|&b| b == b'.' || b == b' ')
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::names::validate_name;
    use proptest::prelude::*;

    /// The plain `is_bolt_alias`, which copies the name twice.
    fn bolt_alias_by_copying(name: &str) -> bool {
        let kept: String = name.chars().filter(|c| !is_ignorable(*c)).collect();
        let base = ntfs_base(&kept).to_ascii_lowercase();
        base == ".bolt" || matches!(base.as_bytes(), [b'b', b'o', b'l', b't', b'~', b'1'..=b'4'])
    }

    /// Names close to `.bolt` and `.git`: their pieces, in any letter case, among characters
    /// macOS ignores, trailing dots and spaces, streams and backslashes.
    fn near_alias() -> impl Strategy<Value = String> {
        let piece = prop_oneof![
            Just(".bolt"),
            Just(".git"),
            Just("bolt~1"),
            Just("bolt~2"),
            Just("bolt~3"),
            Just("bolt~5"),
            Just("git~1"),
            Just("."),
            Just(" "),
            Just(":"),
            Just("\\"),
            Just("/"),
            Just("\u{200C}"),
            Just("\u{FEFF}"),
            Just("\u{FFFF}"),
            Just("b"),
            Just("é"),
        ];
        (prop::collection::vec(piece, 0..6), any::<u8>()).prop_map(|(pieces, case)| {
            pieces
                .concat()
                .chars()
                .enumerate()
                .map(|(i, c)| {
                    if case >> (i % 8) & 1 == 1 {
                        c.to_ascii_uppercase()
                    } else {
                        c
                    }
                })
                .collect()
        })
    }

    proptest! {
        #[test]
        fn bolt_aliases_match_the_plain_check(name in near_alias()) {
            prop_assert_eq!(is_bolt_alias(&name), bolt_alias_by_copying(&name));
        }

        #[test]
        fn bolt_aliases_match_the_plain_check_in_any_string(name in any::<String>()) {
            prop_assert_eq!(is_bolt_alias(&name), bolt_alias_by_copying(&name));
        }
    }

    thread_local! {
        static FULL_CHECKS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    }

    pub(super) fn count_full_check() {
        FULL_CHECKS.with(|n| n.set(n.get().saturating_add(1)));
    }

    fn git_alias_in_full(name: &str) -> bool {
        is_hfs_dotgit(name) || name.split('\\').any(is_ntfs_dotgit)
    }

    fn bolt_alias_in_full(name: &str) -> bool {
        let mut base = ['\0'; 6];
        let mut end = 0;
        let chars = name.chars().filter(|&c| !is_ignorable(c));
        for (at, c) in chars
            .take_while(|c| !matches!(c, ':' | '/' | '\\'))
            .enumerate()
        {
            let trailing = c == '.' || c == ' ';
            match base.get_mut(at) {
                Some(slot) => *slot = c.to_ascii_lowercase(),
                None if trailing => continue,
                None => return false,
            }
            if !trailing {
                end = at + 1;
            }
        }
        let base = base.get(..end).unwrap_or_default();
        base == ['.', 'b', 'o', 'l', 't'] || matches!(base, ['b', 'o', 'l', 't', '~', '1'..='4'])
    }

    /// Names built from pieces at the edges of the early answer: `.git` and `.bolt` spellings,
    /// the first bytes it lets through, separators, ignorable characters and ordinary letters.
    fn near_first_byte() -> impl Strategy<Value = String> {
        let piece = prop_oneof![
            Just(".git"),
            Just(".GIT"),
            Just("git~1"),
            Just("GIT~1"),
            Just("git~2"),
            Just(".bolt"),
            Just("bolt~2"),
            Just("Bolt~3"),
            Just("BOLT~4"),
            Just("bolt~5"),
            Just("."),
            Just("g"),
            Just("G"),
            Just("b"),
            Just("B"),
            Just(" "),
            Just(":"),
            Just("/"),
            Just("\\"),
            Just("\u{200C}"),
            Just("\u{FEFF}"),
            Just("é"),
            Just("x"),
            Just("7"),
        ];
        prop::collection::vec(piece, 0..=6).prop_map(|pieces| pieces.concat())
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(10_000))]

        #[test]
        fn the_early_answer_for_git_is_the_full_one(name in near_first_byte()) {
            prop_assert_eq!(is_git_alias(&name), git_alias_in_full(&name));
        }

        #[test]
        fn the_early_answer_for_bolt_is_the_full_one(name in near_first_byte()) {
            prop_assert_eq!(is_bolt_alias(&name), bolt_alias_in_full(&name));
        }
    }

    #[test]
    fn the_early_answers_match_the_full_ones_around_every_spelling() {
        let alphabet = [
            '.', 'g', 'G', 'i', 't', 'b', 'o', 'l', '~', '1', '\\', ':', '/', ' ', 'a',
        ];
        // Every string of 0 to 2 bytes from the alphabet.
        let mut short = vec![String::new()];
        let mut last = short.clone();
        for _ in 0..2 {
            last = last
                .iter()
                .flat_map(|n| alphabet.iter().map(move |c| format!("{n}{c}")))
                .collect();
            short.extend(last.iter().cloned());
        }
        // The empty core keeps every string of up to 4 bytes in the run.
        let cores = [
            "", ".git", ".GIT", "git~1", "GIT~1", ".bolt", ".BOLT", "bolt~1", "bolt~2", "bolt~3",
            "bolt~4",
        ];
        let (mut git, mut bolt) = (0, 0);
        for core in cores {
            for before in &short {
                for after in &short {
                    let name = format!("{before}{core}{after}");
                    let (git_full, bolt_full) =
                        (git_alias_in_full(&name), bolt_alias_in_full(&name));
                    assert_eq!(is_git_alias(&name), git_full, "{name:?}");
                    assert_eq!(is_bolt_alias(&name), bolt_full, "{name:?}");
                    git += usize::from(git_full);
                    bolt += usize::from(bolt_full);
                }
            }
        }
        // The names must hold look-alikes, or a wrong early `false` would go unseen.
        assert!(git > 0 && bolt > 0, "{git} {bolt}");
    }

    #[test]
    fn ordinary_names_never_reach_the_full_checks() {
        let before = FULL_CHECKS.with(|n| n.get());
        for i in 0..1000 {
            let name = format!("file{i}.txt");
            assert_eq!(validate_name(&name), Ok(()));
            assert!(!is_git_alias(&name) && !is_bolt_alias(&name));
        }
        assert_eq!(FULL_CHECKS.with(|n| n.get()), before);
    }
}
