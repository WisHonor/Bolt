//! An author, committer or tagger line, read as Git reads one.

use super::{split_at_byte, trim_end, trim_start};
use crate::save::{MAX_PERSON_FIELD_BYTES, Person};

/// `Name <email> seconds ±hhmm`, read the way `git log` shows its parts. Never fails: the name
/// ends at the first `<` and the email at the first `>` after it; the time and zone follow the
/// last `>`, and both are zero unless a time and a signed zone are there. A line without an
/// email is all name.
pub fn parse_person(line: &[u8]) -> Person {
    let Some((name, after)) = split_at_byte(line, b'<') else {
        return person(trim_end(line), b"", 0, 0);
    };
    let name = trim_end(name);
    let Some((email, _)) = split_at_byte(after, b'>') else {
        return person(name, b"", 0, 0);
    };
    let date = line.rsplit(|&b| b == b'>').next().unwrap_or_default();
    let (time_ms, tz_offset_min) = parse_date(date).unwrap_or((0, 0));
    person(name, email, time_ms, tz_offset_min)
}

/// ` seconds ±hhmm`: `None` without the digits of a time or a sign and digit for the zone. A
/// time too large is zero, and a zone that isn't exactly four digits is zero, each alone.
fn parse_date(date: &[u8]) -> Option<(i64, i16)> {
    let date = trim_start(date);
    let digits = date.iter().take_while(|b| b.is_ascii_digit()).count();
    let (time, rest) = date.split_at_checked(digits)?;
    let rest = trim_start(rest);
    let (&sign, zone) = rest.split_first()?;
    if digits == 0 || !matches!(sign, b'+' | b'-') || !zone.first()?.is_ascii_digit() {
        return None;
    }
    let time_ms = std::str::from_utf8(time)
        .ok()
        .and_then(|time| time.parse::<i64>().ok())
        .and_then(|secs| secs.checked_mul(1000))
        .unwrap_or(0);
    let zone_digits = zone.iter().take_while(|b| b.is_ascii_digit()).count();
    let tz = (zone_digits == 4)
        .then(|| zone_minutes(sign, zone.get(..4)?))
        .flatten()
        .unwrap_or(0);
    Some((time_ms, tz))
}

/// `hhmm` after its sign, in minutes. Minutes past 59 count as they are.
fn zone_minutes(sign: u8, hhmm: &[u8]) -> Option<i16> {
    let hhmm: i16 = std::str::from_utf8(hhmm).ok()?.parse().ok()?;
    let minutes = hhmm
        .checked_div(100)?
        .checked_mul(60)?
        .checked_add(hhmm.checked_rem(100)?)?;
    if sign == b'-' {
        minutes.checked_neg()
    } else {
        Some(minutes)
    }
}

fn person(name: &[u8], email: &[u8], time_ms: i64, tz_offset_min: i16) -> Person {
    Person {
        name: text(name),
        email: text(email),
        time_ms,
        tz_offset_min,
    }
}

/// Bytes as text, the way Bolt turns other Git text into UTF-8, cut to the field's limit at a
/// character boundary.
fn text(bytes: &[u8]) -> String {
    let mut text = String::from_utf8_lossy(bytes).into_owned();
    text.truncate(text.floor_char_boundary(MAX_PERSON_FIELD_BYTES));
    text
}

#[cfg(test)]
mod tests;
