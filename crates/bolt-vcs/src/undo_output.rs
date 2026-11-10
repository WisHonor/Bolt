//! What `bolt undo --list` shows people: the steps redo can bring back, where you are, then the
//! steps undo can take, in columns.

use std::io::{self, Write};

use crate::style::{self, Role, Sym};

/// A step on the list, each piece ready to print and escaped: the command that takes it back
/// (`redo 1`, `undo 2`; nothing for the diary's first entry), its entry's first 12 hex digits,
/// when it ran, and what it did.
pub struct Step {
    pub label: String,
    pub entry: String,
    pub when: String,
    pub what: String,
}

/// The list, newest first: `redo` above the place marker, `undo` below it, then whether undo can
/// take more than are shown. On a terminal what a step did is cut at its end to fit.
pub fn write_list(
    out: &mut dyn Write,
    project: &str,
    redo: &[Step],
    undo: &[Step],
    more: bool,
) -> io::Result<()> {
    let s = style::out();
    let redo_first = if redo.is_empty() {
        ""
    } else {
        " Steps you can redo come first."
    };
    writeln!(out, "Undo diary for {project}, newest first.{redo_first}\n")?;
    let widest = |piece: fn(&Step) -> &str| {
        (redo.iter().chain(undo))
            .map(|step| style::columns(piece(step)))
            .max()
            .unwrap_or(0)
    };
    let (label, entry, when) = (
        widest(|s| &s.label),
        widest(|s| &s.entry),
        widest(|s| &s.when),
    );
    let room = s.width.map_or(usize::MAX, |w| {
        w.saturating_sub(2 + label + 2 + entry + 2 + when + 2)
    });
    let pad = |text: &str, width: usize| " ".repeat(width.saturating_sub(style::columns(text)));
    let row = |step: &Step| {
        format!(
            "  {}{}  {}{}  {}{}  {}",
            step.label,
            pad(&step.label, label),
            s.paint(Role::Fingerprint, &step.entry),
            pad(&step.entry, entry),
            s.paint(Role::Dim, &step.when),
            pad(&step.when, when),
            s.cut_end(&step.what, room)
        )
    };
    for step in redo {
        writeln!(out, "{}", row(step))?;
    }
    writeln!(
        out,
        "{} you are here",
        s.paint(Role::Bold, s.sym(Sym::Marker))
    )?;
    for step in undo {
        writeln!(out, "{}", row(step))?;
    }
    if more {
        let all = s.paint(Role::Command, "bolt undo --list --all");
        writeln!(out, "  and more: {all} shows the whole diary")?;
    }
    Ok(())
}
