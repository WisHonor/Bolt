//! What `opalro show` shows people before its table of changes: the save, its facts and its
//! message.

use std::io::{self, Write};

use opal_core::id::Id;
use opal_core::save::{Person, Save};
use opal_core::text::escape;

use crate::output;
use crate::style::{self, Role};

/// The save `opalro show` shows: `id`, shown as `short`, with its first parent's message's first
/// line quoted as `parent_title`, and the names of its tags, in order and not yet escaped.
pub struct Head<'a> {
    pub id: &'a Id,
    pub save: &'a Save,
    pub short: &'a str,
    pub parent_title: &'a str,
    pub tags: &'a [String],
}

/// Writes the save, its facts and the rest of its message, then the line that heads its `count`
/// changes.
pub fn write_head(out: &mut dyn Write, head: &Head, count: usize, now: i64) -> io::Result<()> {
    let Head {
        id,
        save,
        short,
        parent_title,
        tags,
    } = *head;
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
    if !tags.is_empty() {
        let names: Vec<String> = (tags.iter())
            .map(|t| s.paint(Role::Line, &escape(t, false)))
            .collect();
        writeln!(out, "    Tags         {}", names.join(", "))?;
    }
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
