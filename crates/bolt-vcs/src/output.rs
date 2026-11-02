//! What `bolt` prints: JSON shapes, warnings and error messages.

use std::fmt::Write as _;

use bolt_core::id::Id;
use bolt_core::save::{Person, Save};
use bolt_core::text::escape;
use bolt_core::warning::{self, Warning};
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

/// The warnings of a `--json` document: `[{"kind": …, "message": …}]`.
pub fn warnings_json(warnings: &[Warning]) -> Value {
    warnings
        .iter()
        .map(|w| json!({"kind": w.kind, "message": w.message}))
        .collect()
}

/// Prints each warning on stderr for people; a damaged copy also gets the hint.
pub fn print_warnings(warnings: &[Warning]) {
    for warning in warnings {
        eprintln!("warning: {}", warning.message);
    }
    if warnings.iter().any(|w| w.kind == warning::DAMAGED_PACK) {
        eprintln!(
            "hint: Bolt didn't use the damaged copy. Ask for help before trying to repair it."
        );
    }
}

/// A failed command's message and what to do about it. Text from outside Bolt in a message is
/// already escaped by the error that made it; the lines Bolt writes itself are not.
pub struct Failure {
    pub message: String,
    pub hint: Option<String>,
    /// The command's `--json` document already ends with this failure.
    pub printed: bool,
}

fn error_json(failure: &Failure) -> Value {
    json!({ "message": failure.message, "hint": failure.hint })
}

/// The end of a `--json` document that is printed in pieces and whose last array is still open:
/// the array's `]`, then the members of `more` and the warnings, then the error if the command
/// failed after printing began, and the closing `}`. Members come in alphabetical order, as
/// serde_json writes them, so a finished document reads as if it had been written whole.
pub fn document_end(more: Value, warnings: &[Warning], failure: Option<&Failure>) -> String {
    let mut members = match more {
        Value::Object(members) => members,
        _ => serde_json::Map::new(),
    };
    members.insert("warnings".into(), warnings_json(warnings));
    if let Some(failure) = failure {
        members.insert("error".into(), error_json(failure));
    }
    let mut end = "]".to_owned();
    for (key, value) in &members {
        let _ = write!(end, ",{}:{value}", Value::from(key.as_str()));
    }
    end.push('}');
    end
}

/// Prints [`document_end`] and passes `result` on. A failure comes back marked as printed, since
/// the document says it.
pub fn end_document(
    more: Value,
    warnings: &[Warning],
    result: Result<(), Failure>,
) -> Result<(), Failure> {
    println!("{}", document_end(more, warnings, result.as_ref().err()));
    result.map_err(|failure| Failure {
        printed: true,
        ..failure
    })
}

/// Prints `failure`: to stdout as JSON with `--json`, else to stderr as text. A failure the
/// command's own document already ends with is not printed again.
pub fn print_error(json: bool, failure: &Failure) {
    if failure.printed {
        return;
    }
    if json {
        println!(
            "{}",
            json!({ "bolt_json": BOLT_JSON, "error": error_json(failure) })
        );
    } else {
        // Escaped once more, so that a piece nobody thought of escaping still can't drive the
        // terminal. Only the newlines Bolt writes survive: the pieces had theirs escaped.
        eprintln!("error: {}", escape(&failure.message, true));
        if let Some(hint) = &failure.hint {
            eprintln!("hint: {}", escape(hint, true));
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
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;

    /// A `history` document that was printing its saves when the command failed.
    const SO_FAR: &str =
        r#"{"bolt_json":1,"line":"main","saves":[{"message":"one"},{"message":"two"}"#;

    #[test]
    fn a_document_cut_short_by_an_error_ends_as_one_valid_document() {
        let failure = Failure {
            message: "Can't read it.".into(),
            hint: Some("Try again.".into()),
            printed: false,
        };
        let warnings = [Warning::new("damaged_pack", "pack x is damaged.")];
        let end = document_end(json!({}), &warnings, Some(&failure));
        let doc: Value = serde_json::from_str(&format!("{SO_FAR}{end}")).unwrap();
        // What was printed stays, and the error and the warnings come after it.
        assert_eq!(doc["saves"].as_array().unwrap().len(), 2);
        assert_eq!(
            doc["error"],
            json!({"message": "Can't read it.", "hint": "Try again."})
        );
        assert_eq!(doc["warnings"][0]["kind"], "damaged_pack");
        assert_eq!(doc["warnings"][0]["message"], "pack x is damaged.");
    }

    #[test]
    fn a_finished_document_ends_as_serde_json_would_write_it() {
        let end = document_end(json!({"save": {"id": "x"}}), &[], None);
        let whole = format!(r#"{{"bolt_json":1,"changes":[{{"path":"a"}}{end}"#);
        let expected = json!({
            "bolt_json": 1,
            "changes": [{"path": "a"}],
            "save": {"id": "x"},
            "warnings": [],
        });
        assert_eq!(whole, expected.to_string());
    }

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
    }
}
