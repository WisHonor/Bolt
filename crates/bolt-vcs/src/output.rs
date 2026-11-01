//! What `bolt` prints: JSON shapes and error messages.

use bolt_core::id::Id;
use bolt_core::save::{Person, Save};
use serde_json::{Value, json};

/// The version of every JSON document `bolt --json` prints.
pub const BOLT_JSON: u32 = 1;

fn person_json(p: &Person) -> Value {
    json!({
        "name": p.name,
        "email": p.email,
        "time_ms": p.time_ms,
        "tz_offset_min": p.tz_offset_min,
    })
}

/// A save as JSON; `short` is the shortest form of its change ID worth showing.
pub fn save_json(id: &Id, save: &Save, short: &str) -> Value {
    json!({
        "change_id": save.change_id.to_letters(),
        "change_id_short": short,
        "id": id.to_hex(),
        "message": save.message,
        "author": person_json(&save.author),
        "saver": person_json(&save.saver),
        "line": save.line(),
    })
}

/// Makes repository text safe to print: control characters (a hostile repository could embed
/// terminal escapes) become a visible `\xNN`; a newline stays when `keep_newlines`.
pub fn sanitize(text: &str, keep_newlines: bool) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if c.is_control() && !(keep_newlines && c == '\n') {
            out.push_str(&format!("\\x{:02x}", u32::from(c)));
        } else {
            out.push(c);
        }
    }
    out
}

/// A failed command's message and what to do about it.
pub struct Failure {
    pub message: String,
    pub hint: Option<String>,
}

/// Prints `failure`: to stdout as JSON with `--json`, else to stderr as text.
pub fn print_error(json: bool, failure: &Failure) {
    if json {
        println!(
            "{}",
            json!({
                "bolt_json": BOLT_JSON,
                "error": { "message": failure.message, "hint": failure.hint },
            })
        );
    } else {
        eprintln!("error: {}", sanitize(&failure.message, false));
        if let Some(hint) = &failure.hint {
            eprintln!("hint: {}", sanitize(hint, false));
        }
    }
}

/// How long ago `time_ms` was, in words; after 30 days, the date in the save's own zone.
pub fn relative_time(time_ms: i64, tz_offset_min: i16, now_ms: i64) -> String {
    let secs = now_ms.saturating_sub(time_ms) / 1000;
    let (mins, hours, days) = (secs / 60, secs / 3600, secs / 86400);
    match secs {
        ..60 => "just now".into(),
        _ if hours == 0 => format!("{mins} min ago"),
        _ if days == 0 => format!("{hours} h ago"),
        _ if days == 1 => "1 day ago".into(),
        _ if days <= 30 => format!("{days} days ago"),
        _ => date(time_ms, tz_offset_min),
    }
}

fn date(time_ms: i64, tz_offset_min: i16) -> String {
    let zoned = jiff::Timestamp::from_millisecond(time_ms).and_then(|ts| {
        let offset = jiff::tz::Offset::from_seconds(i32::from(tz_offset_min) * 60)?;
        Ok(ts.to_zoned(jiff::tz::TimeZone::fixed(offset)))
    });
    // A time outside the calendar can't be shown as a date; show the raw number instead.
    zoned.map_or_else(|_| format!("{time_ms} ms"), |z| z.date().to_string())
}

/// 1024-based size: bytes below 1 KiB, else KB, MB or GB with one decimal.
#[allow(clippy::cast_precision_loss)]
pub fn format_size(bytes: u64) -> String {
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64;
    let mut unit = "";
    for u in ["KB", "MB", "GB"] {
        value /= 1024.0;
        unit = u;
        // Round first: 1023.95 KB must read as 1.0 MB, not 1024.0 KB.
        if (value * 10.0).round() < 10240.0 {
            break;
        }
    }
    format!("{value:.1} {unit}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times_and_sizes_read_naturally() {
        let now = 40 * 86_400_000;
        let ago = |s: i64| relative_time(now - s * 1000, 0, now);
        assert_eq!(ago(59), "just now");
        assert_eq!(ago(60), "1 min ago");
        assert_eq!(ago(3599), "59 min ago");
        assert_eq!(ago(3600), "1 h ago");
        assert_eq!(ago(86_400), "1 day ago");
        assert_eq!(ago(2 * 86_400), "2 days ago");
        assert_eq!(ago(30 * 86_400), "30 days ago");
        assert_eq!(ago(31 * 86_400), "1970-01-10");
        assert_eq!(relative_time(0, 120, now), "1970-01-01");
        assert_eq!(relative_time(now + 5000, 0, now), "just now");
        let sizes = [0, 1023, 1024, 1536, 1 << 20, 3 << 30, 5 << 40].map(format_size);
        assert_eq!(
            sizes,
            [
                "0 B",
                "1023 B",
                "1.0 KB",
                "1.5 KB",
                "1.0 MB",
                "3.0 GB",
                "5120.0 GB"
            ]
        );
        assert_eq!(format_size(1024 * 1024 - 51), "1.0 MB");
        assert_eq!(sanitize("a\x1b[31m\nb", false), "a\\x1b[31m\\x0ab");
        assert_eq!(sanitize("a\x1b]0;t\x07\nb", true), "a\\x1b]0;t\\x07\nb");
    }
}
