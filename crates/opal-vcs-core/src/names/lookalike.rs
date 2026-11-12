//! Whether Windows or macOS would take a name for `.git`, `.gitmodules` or `.opalro`.
//!
//! NTFS drops a `:stream` suffix and trailing dots and spaces, ignores ASCII letter case, and
//! answers to 8.3 short names. HFS+ ignores letter case and skips the characters Apple's TN1150
//! lists as ignorable, anywhere in a name. Git for Windows also reads `\` as a separator.

use std::borrow::Cow;
use std::ops::RangeInclusive;

/// Whether some Windows or macOS drive could take `name` for `.git`, as Git judges it.
pub fn is_git_alias(name: &str) -> bool {
    if !may_spell(name, b'g') && !name.contains('\\') {
        return false;
    }
    // Git checks the start and every part after a `\` on NTFS, each ending at a `/` or `\`, but
    // HFS+ only from the start.
    hfs_spells(name.split('/').next().unwrap_or(name), ".git")
        || name.split('\\').any(|part| {
            let part = ntfs_form(part.split('/').next().unwrap_or(part));
            part.eq_ignore_ascii_case(".git") || short_name(part, b"git", b'1'..=b'1')
        })
}

/// Whether some Windows or macOS drive could take `name` for `.opalro`: its part before any `/` or
/// `\`, without the characters HFS+ ignores, as NTFS opens it.
pub fn is_opal_alias(name: &str) -> bool {
    if !may_spell(name, b'o') {
        return false;
    }
    let part = name.split(['/', '\\']).next().unwrap_or(name);
    let kept: Cow<'_, str> = if part.chars().any(hfs_ignores) {
        Cow::Owned(part.chars().filter(|&c| !hfs_ignores(c)).collect())
    } else {
        Cow::Borrowed(part)
    };
    let part = ntfs_form(&kept);
    part.eq_ignore_ascii_case(".opalro") || short_name(part, b"opalro", b'1'..=b'4')
}

/// Whether `name` is NTFS's hashed short name with this six-byte start (two letters of the long
/// name and four hex digits of its hash), in any letter case, with any stream suffix and
/// trailing dots and spaces. That is the start, `~` and a digit 1 to 9; or, for a longer
/// counter, as much of the start as leaves eight characters with `~` and the counter, which
/// starts with 1 to 9.
pub(crate) fn is_hashed_short_name(name: &str, prefix: &[u8]) -> bool {
    let starts = |b: u8| b == b'~' || prefix.first().is_some_and(|p| p.eq_ignore_ascii_case(&b));
    if !name.bytes().next().is_some_and(starts) {
        return false;
    }
    let Some((start, counter)) = ntfs_form(name).split_once('~') else {
        return false;
    };
    let (start, counter) = (start.as_bytes(), counter.as_bytes());
    let start_fits = prefix
        .get(..start.len())
        .is_some_and(|p| p.eq_ignore_ascii_case(start));
    match counter {
        [b'1'..=b'9'] => start.len() == prefix.len() && start_fits,
        [b'1'..=b'9', rest @ ..] => {
            start_fits && rest.iter().all(u8::is_ascii_digit) && start.len() + counter.len() == 7
        }
        _ => false,
    }
}

/// Whether `name` is NTFS's plain short name with this start (the long name's first six
/// letters), `~` and a digit 1 to 4, which NTFS uses before it turns to hashed names; in any
/// letter case, with any stream suffix and trailing dots and spaces.
pub(crate) fn is_plain_short_name(name: &str, prefix: &[u8; 6]) -> bool {
    name.bytes()
        .next()
        .is_some_and(|b| b.eq_ignore_ascii_case(&prefix[0]))
        && short_name(ntfs_form(name), prefix, b'1'..=b'4')
}

/// The cheap first look: a spelling starts with `.`, the short name's first letter, or an
/// ignorable character (UTF-8 lead byte E2 for U+200C to U+206F, EF for U+FEFF).
fn may_spell(name: &str, letter: u8) -> bool {
    name.bytes()
        .next()
        .is_some_and(|b| matches!(b, b'.' | 0xE2 | 0xEF) || b.eq_ignore_ascii_case(&letter))
}

/// A name part as NTFS opens it: without a `:stream` suffix, then without trailing dots and
/// spaces.
fn ntfs_form(part: &str) -> &str {
    let part = part.split_once(':').map_or(part, |(file, _)| file);
    part.trim_end_matches(['.', ' '])
}

/// Whether `part` is `prefix`, `~` and one digit from `digits`, ignoring ASCII letter case.
fn short_name(part: &str, prefix: &[u8], digits: RangeInclusive<u8>) -> bool {
    match part.as_bytes().split_at_checked(prefix.len()) {
        Some((start, [b'~', digit])) => {
            start.eq_ignore_ascii_case(prefix) && digits.contains(digit)
        }
        _ => false,
    }
}

/// Whether HFS+ reads `name` as `dotted`: the same ASCII letters in any case once the ignorable
/// characters are skipped, then the end of the name or a `\`. Git takes U+FFFE or U+FFFF
/// as the end of the name, so Opalro does too.
fn hfs_spells(name: &str, dotted: &str) -> bool {
    let mut chars = name.chars().filter(|&c| !hfs_ignores(c));
    dotted
        .chars()
        .all(|want| chars.next().is_some_and(|c| c.eq_ignore_ascii_case(&want)))
        && matches!(chars.next(), None | Some('\\' | '\u{FFFE}' | '\u{FFFF}'))
}

/// The code points HFS+ leaves out of a name when it compares names (TN1150): joiners,
/// directional marks and formatting characters, and the byte order mark.
fn hfs_ignores(c: char) -> bool {
    matches!(
        c,
        '\u{200C}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{206A}'..='\u{206F}' | '\u{FEFF}'
    )
}

#[cfg(test)]
mod tests;
