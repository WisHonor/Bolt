//! What `bolt show` shows people before its table of changes: the save, its facts and its
//! message.

use std::io::{self, Write};

use bolt_core::id::Id;
use bolt_core::save::{Person, Save};
use bolt_core::text::escape;

use crate::output;
use crate::style::{self, Role};

/// Writes the save `id`, shown as `short`, its facts and the rest of its message, then the line
/// that heads its `count` changes. `parent_title` is its first parent's message's first line,
/// quoted.
pub fn write_head(
    out: &mut dyn Write,
    id: &Id,
    save: &Save,
    short: &str,
    parent_title: &str,
    count: usize,
    now: i64,
) -> io::Result<()> {
    let s = style::out();
    let person = |p: &Person| {
        let ago = output::relative_time(p.time_ms, p.tz_offset_min, now);
        format!(
            "{} <{}>, {}  {}",
            escape(&p.name, false),
            escape(&p.email, false),
            output::date_time(p.time_ms, p.tz_offset_min),
            s.paint(Role::Dim, &format!("({ago})"))
        )
    };
    writeln!(
        out,
        "Save {} {}\n",
        s.paint(Role::ChangeId, short),
        output::quoted_title(save)
    )?;
    writeln!(out, "    Author       {}", person(&save.author))?;
    writeln!(out, "    Saved by     {}", person(&save.saver))?;
    let line = save.line().map_or("none".to_owned(), |line| {
        s.paint(Role::Line, &escape(line, false))
    });
    writeln!(out, "    Line         {line}")?;
    // By fingerprint: shortening a change ID for it could read the whole line.
    // A merge brought in from Git has more than one; the first is the one compared with.
    let mut named = save
        .parents
        .iter()
        .map(|p| s.paint(Role::Fingerprint, &p.short_hex()));
    if let Some(first) = named.next() {
        let others: String = named.map(|p| format!(", {p}")).collect();
        writeln!(out, "    After        {first} {parent_title}{others}")?;
    }
    writeln!(
        out,
        "    Fingerprint  {}",
        s.paint(Role::Fingerprint, &id.to_hex())
    )?;
    // The rest of the message, indented eight, so that none of it can pass for a change row,
    // which is indented four.
    let message = escape(save.message.trim_end(), true);
    let body: Vec<&str> = message
        .lines()
        .skip(1)
        .skip_while(|l| l.trim().is_empty())
        .collect();
    if !body.is_empty() {
        writeln!(out)?;
    }
    for line in body {
        if line.trim().is_empty() {
            writeln!(out)?;
        } else {
            writeln!(out, "        {line}")?;
        }
    }
    if count == 0 {
        writeln!(out, "\nNo files changed.")
    } else {
        writeln!(out, "\n{} changed:\n", output::files(count))
    }
}
