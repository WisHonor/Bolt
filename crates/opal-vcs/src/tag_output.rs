//! What `opalro tag` shows people. `at` names a save as [`crate::goto_output::at_save`] does.

use opal_core::diary::TagList;
use opal_core::tags::shown_name;
use opal_core::text::escape;

use crate::output;
use crate::style::{self, Role};

/// A tag's name, blue like a line's, on `s`.
pub fn named(s: style::Screen, tag: &str) -> String {
    s.paint(Role::Line, &shown_name(tag))
}

/// The tags in name order: each name, 12 hex digits of its save, and its message's first line.
pub fn list(tags: &TagList) -> String {
    if tags.tags.is_empty() {
        return "No tags: opalro tag <name> names the save you're at.\n".to_owned();
    }
    let s = style::out();
    let names: Vec<String> = tags.tags.keys().map(|n| shown_name(n)).collect();
    let wide = names.iter().map(|n| style::columns(n)).max().unwrap_or(0);
    let mut text = format!(
        "{}:\n\n",
        output::counted(tags.tags.len() as u64, "tag", "tags")
    );
    for (name, tag) in names.iter().zip(tags.tags.values()) {
        let pad = " ".repeat(wide - style::columns(name));
        let mut row = format!(
            "  {}{pad}  {}",
            s.paint(Role::Line, name),
            s.paint(Role::Fingerprint, &tag.save.short_hex())
        );
        if let Some(first) = tag.message.lines().next().filter(|l| !l.is_empty()) {
            // What comes before the message, and its quotes.
            let room = s.width.map_or(usize::MAX, |w| w.saturating_sub(wide + 20));
            row += &format!("  \"{}\"", s.cut_end(&escape(first, false), room));
        }
        text += &row;
        text.push('\n');
    }
    text
}

/// What `opalro tag <name>` made.
pub fn made(tag: &str, at: &str) -> String {
    let s = style::out();
    let word = s.paint(Role::Ok, "Tagged");
    format!("{word} save {at} as {}.\n", named(s, tag))
}

/// What `opalro tag <name>` said when the tag was there already; `message_given` when a message
/// was typed, which the tag didn't take.
pub fn already(tag: &str, at: &str, message_given: bool) -> String {
    let s = style::out();
    let text = format!("Tag {} is already at save {at}.\n", named(s, tag));
    if !message_given {
        return text;
    }
    format!(
        "{text}\n{}\n",
        s.detail(None, "Its message was left as it is.")
    )
}

/// What `opalro tag <name>` did when it moved the tag from `was`.
pub fn moved(tag: &str, at: &str, was: &str) -> String {
    let s = style::out();
    let word = s.paint(Role::Ok, "Moved");
    format!(
        "{word} tag {} to save {at}. It was at save {was}.\n",
        named(s, tag)
    )
}

/// What `opalro tag --delete` did.
pub fn deleted(tag: &str, at: &str) -> String {
    let s = style::out();
    let word = s.paint(Role::Ok, "Deleted");
    format!("{word} tag {}, which was at save {at}.\n", named(s, tag))
}
