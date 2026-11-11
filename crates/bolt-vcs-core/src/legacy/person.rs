//! An author, committer or tagger line, read as Git reads one.
#![deny(clippy::indexing_slicing, clippy::arithmetic_side_effects)]

use crate::git::{split_at_byte, trim_end, trim_start};
use crate::save::{MAX_PERSON_FIELD_BYTES, Person};

/// A person as Git's `split_ident_line` reads an author, committer or tagger: the name up to the
/// first `<`, the email up to the next `>`, then the time and zone after the last `>`. What is
/// missing or out of range is empty or zero; names and emails are UTF-8 (U+FFFD for bytes that
/// aren't) of at most 4 KiB.
///
/// Lossy on purpose: `git-header` keeps the exact line.
#[must_use]
pub fn parse_person(line: &[u8]) -> Person {
    let Some((name, rest)) = split_at_byte(line, b'<') else {
        return person(line, b"", (0, 0));
    };
    let Some((email, _)) = split_at_byte(rest, b'>') else {
        return person(name, b"", (0, 0));
    };
    let after = line.rsplit(|&b| b == b'>').next().unwrap_or_default();
    person(name, email, when(after))
}

fn person(name: &[u8], email: &[u8], (time_ms, tz_offset_min): (i64, i16)) -> Person {
    Person {
        name: person_field(trim_end(name)),
        email: person_field(email),
        time_ms,
        tz_offset_min,
    }
}

/// A name or email as a save holds it: valid UTF-8, at most 4 KiB.
fn person_field(bytes: &[u8]) -> String {
    // Three bytes past the limit finish any character that starts inside it, so none is cut
    // into a U+FFFD that would fit; what lies further can't reach the result.
    let start = bytes.get(..MAX_PERSON_FIELD_BYTES.saturating_add(3));
    let text = String::from_utf8_lossy(start.unwrap_or(bytes));
    let end = text.floor_char_boundary(MAX_PERSON_FIELD_BYTES);
    text.get(..end).unwrap_or_default().to_owned()
}

/// The time in milliseconds and the zone in minutes after an ident's last `>`: digits, then `+`
/// or `-` and digits. As in Git, neither counts unless both are there; then a time past what
/// milliseconds hold, or a zone that isn't four digits, is zero by itself.
fn when(after: &[u8]) -> (i64, i16) {
    let (seconds, rest) = digits(trim_start(after));
    let Some((&sign, zone)) = trim_start(rest).split_first() else {
        return (0, 0);
    };
    let (zone, _) = digits(zone);
    if seconds.is_empty() || zone.is_empty() || !matches!(sign, b'+' | b'-') {
        return (0, 0);
    }
    let time_ms = decimal(seconds).and_then(|s| s.checked_mul(1000));
    (time_ms.unwrap_or(0), minutes(sign, zone).unwrap_or(0))
}

/// The ASCII digits `bytes` start with, and the rest.
fn digits(bytes: &[u8]) -> (&[u8], &[u8]) {
    let count = bytes.iter().take_while(|b| b.is_ascii_digit()).count();
    bytes.split_at_checked(count).unwrap_or_default()
}

/// The number decimal `digits` spell, if it fits.
fn decimal(digits: &[u8]) -> Option<i64> {
    digits.iter().try_fold(0i64, |n, &d| {
        n.checked_mul(10)?
            .checked_add(i64::from(d.checked_sub(b'0')?))
    })
}

/// A zone's offset in minutes, if it is `HHMM` after its sign. Minutes past 59 count as Git
/// counts them.
fn minutes(sign: u8, zone: &[u8]) -> Option<i16> {
    let (hours, minutes) = zone.split_at_checked(2).filter(|_| zone.len() == 4)?;
    let total = decimal(hours)?
        .checked_mul(60)?
        .checked_add(decimal(minutes)?)?;
    let total = i16::try_from(total).ok()?;
    if sign == b'-' {
        total.checked_neg()
    } else {
        Some(total)
    }
}
