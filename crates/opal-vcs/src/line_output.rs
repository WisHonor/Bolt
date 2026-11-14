//! What `opalro line new`, `rename` and `delete` show people. `at` names a save as
//! [`crate::goto_output::at_save`] does.

use opal_core::text::escape;

use crate::output;
use crate::style::{self, Role};

/// A line's name, blue.
fn named(line: &str) -> String {
    style::out().paint(Role::Line, &escape(line, false))
}

/// `text`, then a detail under it after an empty line.
fn with_detail(text: String, detail: &str) -> String {
    format!("{text}\n{}\n", style::out().detail(None, detail))
}

/// What `opalro line new` made.
pub fn started(line: &str, at: &str) -> String {
    let line = named(line);
    let started = style::out().paint(Role::Ok, "Started");
    with_detail(
        format!("{started} line {line} at save {at}.\n"),
        &format!("You're on it: your next save goes on {line}."),
    )
}

/// What `opalro line rename` did; `renamed` is false when the line had that name already.
pub fn renamed(from: &str, to: &str, renamed: bool) -> String {
    let (from, to) = (named(from), named(to));
    if !renamed {
        return format!("Line {to} already has that name.\n");
    }
    let word = style::out().paint(Role::Ok, "Renamed");
    with_detail(
        format!("{word} line {from} to {to}.\n"),
        &format!("Saves already made keep {from} as the line they were made on."),
    )
}

/// What `opalro line delete` did; `only_here` saves no other line or tag has.
pub fn deleted(line: &str, at: &str, only_here: u64) -> String {
    let word = style::out().paint(Role::Ok, "Deleted");
    let text = format!("{word} line {}, which was at save {at}.\n", named(line));
    if only_here == 0 {
        return text;
    }
    let saves = output::counted(only_here, "save", "saves");
    with_detail(
        text,
        &format!(
            "No other line or tag has its {saves}. opalro tidy frees them once the undo window \
             has passed."
        ),
    )
}
