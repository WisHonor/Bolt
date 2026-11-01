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
        eprintln!("error: {}", failure.message);
        if let Some(hint) = &failure.hint {
            eprintln!("hint: {hint}");
        }
    }
}
