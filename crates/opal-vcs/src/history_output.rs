//! What `opal history` shows people: its heading and its rows, in columns.

use std::fmt::Write as _;
use std::io::{self, Write};

use crate::style::{self, Role, Sym};

/// A save as `opal history` lists it, each piece escaped: its change ID, when it was written, by
/// whom, its message's first line, and the lines whose newest save it is.
pub struct SaveRow {
    pub short: String,
    pub when: String,
    pub author: String,
    pub title: String,
    pub lines: Vec<String>,
}

impl SaveRow {
    /// The bytes of text it holds.
    fn size(&self) -> usize {
        let lines: usize = self.lines.iter().map(String::len).sum();
        self.short.len() + self.when.len() + self.author.len() + self.title.len() + lines
    }
}

/// `opal history` holds rows to set its columns, but never a whole line: it stops at this many
/// rows, or at [`HELD_BYTES`] of their text, and a wider row after them runs long.
const HELD_ROWS: usize = 1_000;
/// How much of the held rows' text `opal history` holds before it sets its columns.
const HELD_BYTES: usize = 1 << 20;

/// `opal history`'s list, written as its rows come.
pub struct History {
    /// The line the project is on, escaped; `None` at a save.
    line: Option<String>,
    held: Vec<SaveRow>,
    held_bytes: usize,
    widths: Option<[usize; 3]>,
    written: usize,
}

impl History {
    pub fn new(line: Option<String>) -> History {
        History {
            line,
            held: Vec::new(),
            held_bytes: 0,
            widths: None,
            written: 0,
        }
    }

    pub fn add(&mut self, out: &mut dyn Write, row: SaveRow) -> io::Result<()> {
        if let Some(widths) = self.widths {
            return self.write_row(out, &row, widths);
        }
        self.held_bytes += row.size();
        self.held.push(row);
        if self.held.len() >= HELD_ROWS || self.held_bytes >= HELD_BYTES {
            self.lay_out(out)?;
        }
        Ok(())
    }

    /// Writes what is still held. With nothing read, nothing is written, not even the heading.
    pub fn finish(&mut self, out: &mut dyn Write) -> io::Result<()> {
        match self.widths {
            None if !self.held.is_empty() => self.lay_out(out),
            _ => Ok(()),
        }
    }

    /// Sets the columns from the rows held, then writes the heading and them.
    fn lay_out(&mut self, out: &mut dyn Write) -> io::Result<()> {
        let s = style::out();
        let widths = self.held.iter().fold([0; 3], |w, r| {
            [
                w[0].max(style::columns(&r.short)),
                w[1].max(style::columns(&r.when)),
                w[2].max(style::columns(&r.author)),
            ]
        });
        self.widths = Some(widths);
        let held = std::mem::take(&mut self.held);
        self.held_bytes = 0;
        // Only called with a row held, the save you're at, which the heading names.
        let Some(first) = held.first() else {
            return Ok(());
        };
        let heading = match &self.line {
            Some(line) => format!("Saves on line {}, newest first:", s.paint(Role::Line, line)),
            None => format!(
                "At save {}, not on a line. Saves back from it, newest first:",
                s.paint(Role::ChangeId, &first.short)
            ),
        };
        writeln!(out, "{heading}\n")?;
        for row in held {
            self.write_row(out, &row, widths)?;
        }
        Ok(())
    }

    /// A row: the place marker on the first, the save you're at; the message cut at its end on a
    /// terminal, so that the row and its lines' names fit.
    fn write_row(
        &mut self,
        out: &mut dyn Write,
        row: &SaveRow,
        widths: [usize; 3],
    ) -> io::Result<()> {
        let s = style::out();
        let marker = match self.written {
            0 => s.paint(Role::Bold, s.sym(Sym::Marker)),
            _ => " ".to_owned(),
        };
        self.written += 1;
        let pad = |text: &str, width: usize| " ".repeat(width.saturating_sub(style::columns(text)));
        let lead = 2 + widths[0] + 2 + widths[1] + 2 + widths[2] + 2;
        let named: usize = row.lines.iter().map(|l| 4 + style::columns(l)).sum();
        let room = s
            .width
            .map_or(usize::MAX, |w| w.saturating_sub(lead + named));
        let mut text = format!(
            "{marker} {}{}  {}{}  {}{}  {}",
            s.paint(Role::ChangeId, &row.short),
            pad(&row.short, widths[0]),
            s.paint(Role::Dim, &row.when),
            pad(&row.when, widths[1]),
            s.paint(Role::Dim, &row.author),
            pad(&row.author, widths[2]),
            s.cut_end(&row.title, room)
        );
        for line in &row.lines {
            let _ = write!(text, "  [{}]", s.paint(Role::Line, line));
        }
        writeln!(out, "{}", text.trim_end())
    }
}
