//! What `bolt` prints: JSON shapes, warnings and error messages.

use std::fmt::{self, Write as _};
use std::io::{self, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock, PoisonError};

use bolt_core::changes::{Endings, Lines, Tag, eol_only, only_line_endings};
use bolt_core::diary::{Action, DiaryEntry, Position};
use bolt_core::diff::{ChangeKind, FileChange};
use bolt_core::id::Id;
use bolt_core::listing::EntryKind;
use bolt_core::save::{Person, Save};
use bolt_core::shelf::{ShelfInfo, Shelved};
use bolt_core::switch::DiscardOutcome;
use bolt_core::text::{bytes, escape, thousands};
use bolt_core::tidy::TidyReport;
use bolt_core::undo::{Effect, Taken};
use bolt_core::verify::{PROBLEMS_KEPT, Problem, ProblemKind, Verified};
use bolt_core::warning::{self, Warning};
use serde_json::{Value, json};

use crate::progress;
use crate::style::{self, Role, Sym};

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

/// Where the project is: `{"line": …}`, or `{"save": …}` when it is at a save.
pub fn position_json(position: &Position) -> Value {
    match position {
        Position::OnLine(line) => json!({ "line": line }),
        Position::AtSave(id) => json!({ "save": id.to_hex() }),
    }
}

/// A file's change as JSON (docs/json.md, "A change").
pub fn change_json(c: &FileChange) -> Value {
    json!({
        "path": c.path,
        "change": match c.change {
            ChangeKind::Added => "added",
            ChangeKind::Removed => "removed",
            ChangeKind::Modified => "modified",
            ChangeKind::KindChanged => "kind_changed",
        },
        // People see `project-link`; every value in JSON is written with underscores.
        "kind": match c.kind {
            EntryKind::ProjectLink => "project_link",
            kind => kind.as_str(),
        },
        "old_size": c.old_size,
        "new_size": c.new_size,
    })
}

/// A step undone or redone as JSON (docs/json.md, `bolt undo --json`).
pub fn step_json(t: &Taken) -> Value {
    let effects: Vec<Value> = t.effects.iter().map(effect_json).collect();
    let action = match t.action {
        Action::Undo(_) => "undo",
        Action::Redo(_) => "redo",
        Action::Command => "jump",
    };
    json!({
        "entry": t.step.to_hex(),
        "action": action,
        "command": t.entry.command,
        "time_ms": t.entry.who.time_ms,
        "effects": effects,
        "written": t.applied.written,
        "removed": t.applied.removed,
    })
}

fn effect_json(e: &Effect) -> Value {
    let hex = |id: &Option<Id>| id.map(|id| id.to_hex());
    match e {
        Effect::Line { name, from, to } => {
            json!({"kind": "line", "name": name, "from": hex(from), "to": hex(to)})
        }
        Effect::Position { from, to } => json!({
            "kind": "position",
            "from": position_json(from),
            "to": position_json(to),
        }),
        Effect::ShelfAdded { label, save } => {
            json!({"kind": "shelf_added", "label": label, "save": save.to_hex()})
        }
        Effect::ShelfGone { label, save } => {
            json!({"kind": "shelf_gone", "label": label, "save": save.to_hex()})
        }
        Effect::Tags => json!({"kind": "tags"}),
        Effect::ServerLines => json!({"kind": "server_lines"}),
        Effect::Files { from, to } => {
            json!({"kind": "files", "from": from.to_hex(), "to": to.to_hex()})
        }
    }
}

/// What a step taken did, in one line for people: undone, redone, or gone to by `undo --to`. The
/// verb (`Undid`, `Redid`, `Went`) comes apart from the rest, so a list of steps can leave it out.
/// `went` is the entry `undo --to` went to, when the step's command was one. `named` gives a
/// save's short change ID, and its message's first line if it can be read. The shelves `--shelve`
/// made (`shelved`) are told on lines of their own.
pub fn step_line(
    t: &Taken,
    went: Option<&Id>,
    shelved: &[(String, Id)],
    named: &mut dyn FnMut(&Id) -> (String, Option<String>),
) -> (&'static str, String) {
    let mut command = escape(&t.entry.command, false);
    if let Some(went) = went {
        let _ = write!(command, " --to {}", went.short_hex());
    }
    let (redo, jump) = (
        matches!(t.action, Action::Redo(_)),
        t.action == Action::Command,
    );
    let (verb, mut line) = match t.action {
        Action::Undo(_) => ("Undid", command),
        Action::Redo(_) => ("Redid", command),
        Action::Command => (
            "Went",
            format!("back to entry {} ({command})", t.step.short_hex()),
        ),
    };
    // The save a step made is the one a line moved to. `undo --to`, recorded as `undo`, makes
    // none: it moves lines to saves that were there.
    let made = t.effects.iter().find_map(|e| match e {
        Effect::Line { from, to, .. } if !jump && t.entry.command != "undo" => {
            if redo {
                *to
            } else {
                *from
            }
        }
        _ => None,
    });
    if let Some(save) = made {
        let (short, message) = named(&save);
        let _ = write!(line, " {short}");
        if let Some(message) = message {
            let _ = write!(line, " \"{}\"", escape(&message, false));
        }
    }
    let (mut parts, mut moved, mut files) = (Vec::new(), false, false);
    let again = |what: String| match t.action {
        Action::Undo(_) => format!("back {what}"),
        Action::Redo(_) => format!("{what} again"),
        Action::Command => what,
    };
    for e in &t.effects {
        match e {
            Effect::Line { name, to, .. } => {
                moved = true;
                let name = escape(name, false);
                parts.push(match to {
                    Some(to) if jump => format!("{name} is at {}", named(to).0),
                    Some(to) if redo => format!("{name} is at {} again", named(to).0),
                    Some(to) => format!("{name} is back at {}", named(to).0),
                    None => format!("line {name} is gone"),
                });
            }
            Effect::Position { to, .. } => {
                moved = true;
                parts.push(again(match to {
                    Position::OnLine(line) => {
                        let tip = t.after.lines.get(line).map(|s| named(s).0);
                        format!("on {} at {}", escape(line, false), tip.unwrap_or_default())
                    }
                    Position::AtSave(save) => format!("at save {}", named(save).0),
                }));
            }
            Effect::ShelfAdded { label, save } => {
                if !shelved.iter().any(|(_, s)| s == save) {
                    parts.push(format!("shelf {} is back", escape(label, false)));
                }
            }
            Effect::ShelfGone { label, .. } => {
                parts.push(format!("shelf {} is gone", escape(label, false)));
            }
            Effect::Tags if redo => parts.push("the tags changed again".to_owned()),
            Effect::Tags => parts.push("the tags changed back".to_owned()),
            Effect::ServerLines if redo => parts.push("the server lines changed again".to_owned()),
            Effect::ServerLines => parts.push("the server lines changed back".to_owned()),
            Effect::Files { .. } => files = true,
        }
    }
    if files {
        let count = |n: usize| match n {
            1 => "1 file".to_owned(),
            n => format!("{n} files"),
        };
        parts.push(match (t.applied.written, t.applied.removed) {
            (0, 0) => "no file needed changing".to_owned(),
            (0, removed) => format!("{} removed", count(removed)),
            (written, removed) => {
                // An undo that leaves the project where it was puts files back.
                let written = if moved || redo || jump {
                    format!("{} written", count(written))
                } else {
                    format!("{} back", count(written))
                };
                match removed {
                    0 => written,
                    removed => format!("{written}, {removed} removed"),
                }
            }
        });
    }
    if !parts.is_empty() {
        let _ = write!(line, ": {}", parts.join(", "));
    }
    // A save undone leaves its files as they are.
    if !redo && !files && made.is_some() {
        line.push_str("; its changes show as unsaved");
    }
    line.push('.');
    (verb, line)
}

/// What `undo --to` did for a lost `head`, in one line for people: the entry it made current, and
/// where that leaves the project. `named` names a save as for [`step_line`].
pub fn picked_up_line(t: &Taken, named: &mut dyn FnMut(&Id) -> (String, Option<String>)) -> String {
    let at = match &t.after.position {
        Position::OnLine(line) => {
            let tip = t.after.lines.get(line).map(|s| titled(named, s));
            format!("on {} at {}", escape(line, false), tip.unwrap_or_default())
        }
        Position::AtSave(save) => format!("at save {}", titled(named, save)),
    };
    format!(
        "{} entry {} ({}) the current one: {at}. No file was changed: bolt status shows how \
         your files differ from that save.",
        style::out().paint(Role::Ok, "Made"),
        t.step.short_hex(),
        escape(&t.entry.command, false)
    )
}

/// A diary entry as `bolt undo --list --json` gives it (docs/json.md, "An entry").
pub fn entry_json(id: &Id, entry: &DiaryEntry, effects: &[Effect]) -> Value {
    let (action, target) = match &entry.action {
        Action::Command => ("command", None),
        Action::Undo(step) => ("undo", Some(step.to_hex())),
        Action::Redo(step) => ("redo", Some(step.to_hex())),
    };
    let effects: Vec<Value> = effects.iter().map(effect_json).collect();
    json!({
        "entry": id.to_hex(),
        "action": action,
        "target": target,
        "command": entry.command,
        "time_ms": entry.who.time_ms,
        "effects": effects,
        "first": entry.parent.is_none(),
    })
}

/// What a diary entry did, as `bolt undo --list` tells it: its command, and the step it undid or
/// redid, then what changed. `named` names a save as for [`step_line`]; `files` counts the files
/// two saves differ in, if it can read them.
pub fn entry_line(
    entry: &DiaryEntry,
    effects: &[Effect],
    named: &mut dyn FnMut(&Id) -> (String, Option<String>),
    files: &mut dyn FnMut(Id, Id) -> Option<usize>,
) -> String {
    let mut line = match (&entry.action, entry.command.as_str()) {
        // `undo --to` records the steps it makes as `undo`; the step line names them so too.
        (Action::Command, "undo") => "undo --to".to_owned(),
        _ => escape(&entry.command, false),
    };
    if let Action::Undo(step) | Action::Redo(step) = &entry.action {
        let _ = write!(line, " of {}", step.short_hex());
    }
    if entry.parent.is_none() {
        line.push_str(": the diary's first entry, which can't be undone");
        return line;
    }
    let mut parts = Vec::new();
    for e in effects {
        parts.push(match e {
            Effect::Line {
                name,
                from: None,
                to: Some(to),
            } => format!("line {} made at {}", escape(name, false), titled(named, to)),
            Effect::Line {
                name, to: Some(to), ..
            } => format!("{} moved to {}", escape(name, false), titled(named, to)),
            Effect::Line { name, to: None, .. } => format!("line {} gone", escape(name, false)),
            Effect::Position {
                to: Position::OnLine(on),
                ..
            } => {
                let tip = entry.state.lines.get(on).map(|s| named(s).0);
                format!("on {} at {}", escape(on, false), tip.unwrap_or_default())
            }
            Effect::Position {
                to: Position::AtSave(save),
                ..
            } => format!("at save {}", named(save).0),
            Effect::ShelfAdded { label, .. } => format!("shelf {} added", escape(label, false)),
            Effect::ShelfGone { label, .. } => format!("shelf {} gone", escape(label, false)),
            Effect::Tags => "the tags changed".to_owned(),
            Effect::ServerLines => "the server lines changed".to_owned(),
            Effect::Files { from, to } => match files(*from, *to) {
                Some(0) => "only folders changed".to_owned(),
                Some(1) => "1 file changed".to_owned(),
                Some(n) => format!("{n} files changed"),
                None => "files changed".to_owned(),
            },
        });
    }
    if !parts.is_empty() {
        let _ = write!(line, ": {}", parts.join(", "));
    }
    line
}

/// A shelf as `bolt shelf --list --json` gives it (docs/json.md, "A shelf").
pub fn shelf_json(s: &ShelfInfo) -> Value {
    json!({
        "label": s.label,
        "save": s.save.to_hex(),
        "message": s.message,
        "line": s.line,
        "saver": person_json(&s.saver),
        "files": s.files,
    })
}

/// `bolt shelf --json`'s document for what `shelf` did.
pub fn shelved_json(done: &Shelved, warnings: &[Warning]) -> Value {
    let made = match done {
        Shelved::Made(made) => Some(made),
        Shelved::Nothing { .. } => None,
    };
    let applied = made.map(|d| d.applied.clone()).unwrap_or_default();
    let shelf = made.map(|d| json!({"label": d.label, "save": d.save.to_hex(), "files": d.files}));
    json!({
        "bolt_json": BOLT_JSON,
        "shelf": shelf,
        "changed_again": matches!(done, Shelved::Nothing { changed_again: true, .. }),
        "written": applied.written,
        "removed": applied.removed,
        "not_written": applied.not_written,
        "placeholders": applied.placeholders,
        "warnings": warnings_json(warnings),
    })
}

/// A save named, then its message's first line in quotes if it can be read.
fn titled(named: &mut dyn FnMut(&Id) -> (String, Option<String>), save: &Id) -> String {
    match named(save) {
        (short, Some(message)) => format!("{short} \"{}\"", escape(&message, false)),
        (short, None) => short,
    }
}

/// The document of a command that failed after doing part of its work: `done` with the error
/// and the warnings added.
pub fn failed_document(mut done: Value, failure: &Failure, warnings: &[Warning]) -> Value {
    done["bolt_json"] = BOLT_JSON.into();
    done["error"] = error_json(failure);
    done["warnings"] = warnings_json(warnings);
    done
}

/// A file's change as people see it: its kind's word and colour, its path escaped, and what it
/// holds (a size, two sizes, or two kinds).
pub struct Row {
    word: &'static str,
    role: Role,
    path: String,
    held: String,
}

impl Row {
    pub fn of(c: &FileChange) -> Row {
        let size = |s: Option<u64>| bytes(s.unwrap_or(0));
        let arrow = style::out().sym(Sym::Arrow);
        let (word, role, held) = match c.change {
            ChangeKind::Added => ("added", Role::Added, size(c.new_size)),
            ChangeKind::Removed => ("removed", Role::Removed, size(c.old_size)),
            ChangeKind::Modified => (
                "modified",
                Role::Changed,
                format!("{} {arrow} {}", size(c.old_size), size(c.new_size)),
            ),
            ChangeKind::KindChanged => {
                let old = c.old_kind.map_or("none", |k| k.as_str());
                let held = format!("{old} {arrow} {}", c.kind.as_str());
                ("kind changed", Role::Changed, held)
            }
        };
        Row {
            word,
            role,
            path: escape(c.path, false),
            held,
        }
    }
}

/// How wide a table of changes' columns are: as wide as what they hold.
#[derive(Default)]
pub struct Columns {
    word: usize,
    path: usize,
    held: usize,
}

impl Columns {
    pub fn fit(&mut self, row: &Row) {
        self.word = self.word.max(style::columns(row.word));
        self.path = self.path.max(style::columns(&row.path));
        self.held = self.held.max(style::columns(&row.held));
    }

    /// Makes the path column as wide as the widest path shown once cut, for a table whose rows
    /// are all at hand. Without it, a table with a cut path is as wide as the terminal lets it be.
    pub fn settle(&mut self, s: style::Screen, rows: &[Row]) {
        let room = self.room(s);
        self.path = (rows.iter())
            .map(|r| style::columns(&s.fit_path(&r.path, room)))
            .max()
            .unwrap_or(0);
    }

    fn room(&self, s: style::Screen) -> usize {
        let others = 4 + self.word + 2 + 2 + self.held;
        s.width.map_or(usize::MAX, |w| w.saturating_sub(others))
    }

    /// `row` in the table, indented four: on a terminal its path is cut from the middle when the
    /// row would pass the width.
    pub fn line(&self, s: style::Screen, row: &Row) -> String {
        let room = self.room(s);
        let path = s.fit_path(&row.path, room);
        let pad = |text: &str, width: usize| " ".repeat(width.saturating_sub(style::columns(text)));
        format!(
            "    {}{}  {}{}  {}{}",
            s.paint(row.role, row.word),
            pad(row.word, self.word),
            s.paint(Role::Path, &path),
            pad(&path, self.path.min(room)),
            pad(&row.held, self.held),
            row.held
        )
    }
}

/// A save's message's first line, escaped, in double quotes.
pub fn quoted_title(save: &Save) -> String {
    format!(
        "\"{}\"",
        escape(save.message.lines().next().unwrap_or_default(), false)
    )
}

/// `n` files, with a comma between thousands.
pub fn files(n: usize) -> String {
    counted(n as u64, "file", "files")
}

/// A file's change with its lines, as `bolt changes --json` gives it. It is written as it is
/// shown, a line at a time, never held as one value.
pub struct LinesJson<'a> {
    pub change: &'a FileChange<'a>,
    pub lines: &'a Lines<'a>,
}

impl fmt::Display for LinesJson<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (shown, hunks, timed_out) = match self.lines {
            Lines::Binary => ("binary", &[][..], false),
            Lines::TooBig => ("too_big", &[][..], false),
            Lines::TooManyLines => ("too_many_lines", &[][..], false),
            Lines::Hunks { hunks, timed_out } => ("text", &hunks[..], *timed_out),
        };
        f.write_str("{\"hunks\":[")?;
        for (i, h) in hunks.iter().enumerate() {
            write!(
                f,
                "{}{{\"new_lines\":{},\"new_start\":{},\"old_lines\":{},\"old_start\":{},\"lines\":[",
                if i == 0 { "" } else { "," },
                h.new_lines,
                h.new_start,
                h.old_lines,
                h.old_start
            )?;
            for (j, ((tag, line), eol)) in h.lines.iter().zip(eol_only(h)).enumerate() {
                let text = Value::from(String::from_utf8_lossy(line).as_ref());
                write!(
                    f,
                    "{}{{{}\"tag\":\"{}\",\"text\":{text}}}",
                    if j == 0 { "" } else { "," },
                    if eol { "\"eol_only\":true," } else { "" },
                    tag_name(*tag)
                )?;
            }
            f.write_str("]}")?;
        }
        // The rest is small, and serde_json writes it: `{` is left out, the hunks having opened
        // the object.
        let mut rest = change_json(self.change);
        rest["content"] = shown.into();
        rest["timed_out"] = timed_out.into();
        rest["only_line_endings"] = only_line_endings(hunks).is_some().into();
        let rest = rest.to_string();
        write!(f, "],{}", rest.get(1..).unwrap_or("}"))
    }
}

fn tag_name(tag: Tag) -> &'static str {
    match tag {
        Tag::Same => "same",
        Tag::Removed => "removed",
        Tag::Added => "added",
    }
}

/// Writes a file's change with its lines, in the unified form, for people, a line at a time. The
/// header starts with a letter, which no line of the file shown below it can. Coloured as stdout
/// allows.
pub fn write_lines(out: &mut dyn Write, c: &FileChange, lines: &Lines) -> io::Result<()> {
    let screen = style::out();
    let row = Row::of(c);
    let mut header = format!(
        "{}  {}",
        screen.paint(row.role, row.word),
        screen.paint(Role::Path, &row.path)
    );
    if let Lines::Hunks { hunks, .. } = lines {
        let count = |tag: Tag| -> usize {
            hunks
                .iter()
                .map(|h| h.lines.iter().filter(|(t, _)| *t == tag).count())
                .sum()
        };
        let _ = write!(
            header,
            "  {} {}",
            screen.paint(Role::Added, &format!("+{}", count(Tag::Added))),
            screen.paint(Role::Removed, &format!("-{}", count(Tag::Removed)))
        );
    }
    let _ = write!(
        header,
        "  {}",
        screen.paint(Role::Dim, &format!("({})", row.held))
    );
    let hunks = match lines {
        Lines::Binary => return writeln!(out, "{header}  (binary: no lines shown)"),
        Lines::TooBig => return writeln!(out, "{header}  (over 64 MiB: no lines shown)"),
        Lines::TooManyLines => {
            return writeln!(out, "{header}  (over 1,000,000 lines: no lines shown)");
        }
        Lines::Hunks { hunks, .. } => hunks,
    };
    match only_line_endings(hunks) {
        Some(Endings::CrlfToLf) => {
            writeln!(out, "{header}  (only line endings changed: CRLF to LF)")?;
        }
        Some(Endings::LfToCrlf) => {
            writeln!(out, "{header}  (only line endings changed: LF to CRLF)")?;
        }
        Some(Endings::Both) => writeln!(out, "{header}  (only line endings changed)")?,
        None => writeln!(out, "{header}")?,
    }
    for h in hunks {
        let at = format!(
            "@@ -{},{} +{},{} @@",
            h.old_start, h.old_lines, h.new_start, h.new_lines
        );
        writeln!(out, "{}", screen.paint(Role::Hunk, &at))?;
        for ((tag, line), eol) in h.lines.iter().zip(eol_only(h)) {
            let (mark, role) = match tag {
                Tag::Same => (' ', None),
                Tag::Removed => ('-', Some(Role::Removed)),
                Tag::Added => ('+', Some(Role::Added)),
            };
            let body = line
                .strip_suffix(b"\r\n")
                .or_else(|| line.strip_suffix(b"\n"));
            // Tabs are kept: they can't drive a terminal, and code is full of them.
            let shown: Vec<String> = String::from_utf8_lossy(body.unwrap_or(line))
                .split('\t')
                .map(|piece| escape(piece, false))
                .collect();
            let mut shown = format!("{mark}{}", shown.join("\t"));
            // Of a pair that differs only in its ending, the line that ends in CRLF says so.
            if eol && line.ends_with(b"\r\n") {
                shown.push_str(" (CRLF)");
            }
            match role {
                Some(role) => writeln!(out, "{}", screen.paint(role, &shown))?,
                None => writeln!(out, "{shown}")?,
            }
            if body.is_none() {
                writeln!(out, "\\ No newline at end of file")?;
            }
        }
    }
    Ok(())
}

/// What parts two files in `bolt changes`: on a terminal a dim rule across it, up to 100 columns;
/// elsewhere an empty line, as a rule of `-` could pass for a removed line.
pub fn between_files() -> String {
    let s = style::out();
    match s.width {
        Some(_) => s.paint(Role::Dim, &s.sym(Sym::Rule).repeat(s.line_room().min(100))),
        None => String::new(),
    }
}

/// Standard output. What it couldn't take is remembered for the whole command (its reader went
/// away, as in `bolt changes | head`, or a write failed), so [`ended`] can tell it on stderr.
pub struct Stdout {
    inner: io::StdoutLock<'static>,
}

/// A write that failed for another reason than its reader going away: its kind and the message.
type Failed = OnceLock<(io::ErrorKind, String)>;

/// The first write to stdout that failed so. Nothing more is written there after it, so what was
/// printed is cut off rather than spliced.
static FAILED: Failed = OnceLock::new();

/// Whether stdout's reader went away.
static GONE: AtomicBool = AtomicBool::new(false);

/// Whether stdout couldn't take something the command printed: its reader went, or a write failed.
fn lost() -> bool {
    GONE.load(Ordering::Relaxed) || FAILED.get().is_some()
}

/// `write` done, unless a write kept in `failed` came before it; a failure of its own is kept,
/// but not a reader gone, nor an interrupted write, which `write_all` tries again.
fn guarded<T>(failed: &Failed, write: impl FnOnce() -> io::Result<T>) -> io::Result<T> {
    let refused = |(kind, why): &(io::ErrorKind, String)| io::Error::new(*kind, why.clone());
    if let Some(earlier) = failed.get() {
        return Err(refused(earlier));
    }
    let passing = [io::ErrorKind::BrokenPipe, io::ErrorKind::Interrupted];
    match write() {
        Err(e) if passing.contains(&e.kind()) => Err(e),
        Err(e) => {
            let why = (e.kind(), format!("Bolt couldn't write its output: {e}"));
            Err(refused(failed.get_or_init(|| why)))
        }
        done => done,
    }
}

impl Stdout {
    pub fn new() -> Stdout {
        Stdout {
            inner: io::stdout().lock(),
        }
    }
}

/// `done`, with a reader gone remembered.
fn noting<T>(done: io::Result<T>) -> io::Result<T> {
    if let Err(e) = &done
        && e.kind() == io::ErrorKind::BrokenPipe
    {
        GONE.store(true, Ordering::Relaxed);
    }
    done
}

impl Write for Stdout {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if !buf.is_empty() {
            PRINTED_ANY.store(true, Ordering::Relaxed);
        }
        noting(guarded(&FAILED, || self.inner.write(buf)))
    }

    fn flush(&mut self) -> io::Result<()> {
        noting(guarded(&FAILED, || self.inner.flush()))
    }
}

/// Writes `text` on stdout. What it couldn't take is remembered, for [`ended`] to tell.
pub fn print_out(text: &str) {
    let mut out = Stdout::new();
    let _ = out.write_all(text.as_bytes()).and_then(|()| out.flush());
}

/// Writes `line` on stderr, with any progress off the screen first. A line stderr can't take is
/// lost, never a panic: its reader may have gone with stdout's.
pub fn tell(line: &str) {
    TOLD_ANY.store(true, Ordering::Relaxed);
    progress::tell(line);
}

/// Whether the command has written on stderr, and on stdout.
static TOLD_ANY: AtomicBool = AtomicBool::new(false);
static PRINTED_ANY: AtomicBool = AtomicBool::new(false);

/// Whether a person reading stderr has seen something from this command already: a line on
/// stderr, or its answer on stdout when both are on a terminal.
fn seen_any() -> bool {
    TOLD_ANY.load(Ordering::Relaxed)
        || PRINTED_ANY.load(Ordering::Relaxed)
            && style::out().width.is_some()
            && style::err().width.is_some()
}

/// What a changing command whose output was lost did all the same.
const DONE: &str = "The command did its work all the same: only what Bolt printed was lost. bolt \
                    status and bolt undo --list show where the project stands.";

/// Ends a command, once it has printed what it could on stdout.
///
/// While stdout took it all, `warnings` go to stderr (a failure its `--json` document ends with
/// lists them there instead), and `failure` comes back for [`print_error`].
///
/// When stdout couldn't take it (its reader went, or a write failed), stderr gets it as text, in
/// this order: the command's own `failure`, with the text `taken` makes, only then, of what an
/// `undo` or `redo` took before it stopped; then `warnings`, each once; then, only if a write
/// failed, that it did, with `done`, what the command did all the same, as the hint. It all
/// comes back told: the command's failure, or the write's; or nothing if it succeeded and only
/// the reader went.
pub fn ended(
    failure: Option<Failure>,
    taken: impl FnOnce() -> String,
    warnings: &[Warning],
    done: Option<&str>,
) -> Result<(), Failure> {
    if !lost() {
        if !failure.as_ref().is_some_and(|f| f.printed) {
            print_warnings(warnings);
        }
        return failure.map_or(Ok(()), Err);
    }
    if let Some(failure) = &failure {
        tell_failure(failure);
    }
    let taken = taken();
    if !taken.is_empty() {
        TOLD_ANY.store(true, Ordering::Relaxed);
        let _ = write!(io::stderr().lock(), "{taken}");
    }
    print_warnings(warnings);
    let wrote = FAILED.get().map(|(_, why)| Failure {
        message: why.clone(),
        hint: done.map(str::to_owned),
        printed: false,
    });
    if let Some(wrote) = &wrote {
        tell_failure(wrote);
    }
    match failure.or(wrote) {
        Some(failure) => Err(Failure {
            printed: true,
            ..failure
        }),
        None => Ok(()),
    }
}

/// Prints `text` on stdout, then ends the command with `warnings` ([`ended`]).
pub fn print_quietly(text: &str, warnings: &[Warning]) -> Result<(), Failure> {
    print_out(text);
    ended(None, String::new, warnings, None)
}

/// [`print_quietly`], for a command that changed the project: if a write fails, the hint says the
/// change stands.
pub fn print_done(text: &str, warnings: &[Warning]) -> Result<(), Failure> {
    print_out(text);
    ended(None, String::new, warnings, Some(DONE))
}

/// Ends a command that printed as it read ([`ended`]): `done` is how that went, with the warnings
/// it found by the end; `found` gives those it had so far, for when stdout stopped it first. Once
/// stdout is lost, a failure in `done` is stdout's own, unless it is `printed`: then it is the
/// command's, in the document stdout lost.
pub fn listed(
    done: Result<Vec<Warning>, Failure>,
    found: impl FnOnce() -> Vec<Warning>,
) -> Result<(), Failure> {
    match done {
        Ok(warnings) => ended(None, String::new, &warnings, None),
        Err(failure) if failure.printed && lost() => {
            ended(Some(failure), String::new, &found(), None)
        }
        Err(_) if lost() => ended(None, String::new, &found(), None),
        Err(failure) => Err(failure),
    }
}

/// The warnings of a `--json` document: `[{"kind": …, "message": …}]`.
pub fn warnings_json(warnings: &[Warning]) -> Value {
    warnings
        .iter()
        .map(|w| json!({"kind": w.kind, "message": w.message}))
        .collect()
}

/// The warnings already on stderr. A command whose output failed tells its own, the store's among
/// them, before the error, which then doesn't tell them again.
static TOLD: Mutex<Vec<Warning>> = Mutex::new(Vec::new());

/// Prints each warning on stderr for people, once, after an empty line when something came
/// before them; a damaged copy also gets the hint.
pub fn print_warnings(warnings: &[Warning]) {
    let screen = style::err();
    let mut told = TOLD.lock().unwrap_or_else(PoisonError::into_inner);
    let (mut first, mut damaged) = (true, false);
    for w in warnings {
        if !told.contains(w) {
            if first && seen_any() {
                tell("");
            }
            first = false;
            tell(&screen.prefixed(Role::Warning, "warning:", &w.message));
            damaged |= w.kind == warning::DAMAGED_PACK;
            told.push(w.clone());
        }
    }
    if damaged {
        tell("");
        tell(&screen.prefixed(
            Role::Hint,
            "hint:",
            "Bolt didn't use the damaged copy. Ask for help before trying to repair it.",
        ));
    }
}

/// A failed command's message and what to do about it. Text from outside Bolt in a message is
/// already escaped by the error that made it; the lines Bolt writes itself are not.
#[derive(Debug, Clone)]
pub struct Failure {
    pub message: String,
    pub hint: Option<String>,
    /// Already told: the command's `--json` document ends with it, or [`ended`] said it.
    pub printed: bool,
}

/// The error as JSON, escaped once more as it is on screen (see [`tell_failure`]).
fn error_json(failure: &Failure) -> Value {
    json!({
        "message": escape(&failure.message, true),
        "hint": failure.hint.as_deref().map(|hint| escape(hint, true)),
    })
}

/// The document of a command that failed before it printed any of its own: the error, and the
/// warnings found before it failed.
fn error_document(failure: &Failure, warnings: &[Warning]) -> Value {
    json!({
        "bolt_json": BOLT_JSON,
        "error": error_json(failure),
        "warnings": warnings_json(warnings),
    })
}

/// The end of a `--json` document printed in pieces, whose last list is still open: the list's
/// `]`, then the members of `more` and the warnings, the error if the command failed, and the
/// closing `}`. They come in alphabetical order, as serde_json writes them, so a finished
/// document is the one that would have been written whole.
fn document_end(more: Value, warnings: &[Warning], failure: Option<&Failure>) -> String {
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

/// Prints a `--json` document whose last list is filled as `walk` goes, so that none of it is
/// held. `opening` is the document up to the `[` of that list, and goes out with the first item.
/// `walk` is given a function that adds one item to the list. When it is over, `end` gives the
/// members that come after the list and the warnings, and the document is closed. Returns the
/// warnings, for the caller to show people.
///
/// If `walk` fails before its first item, nothing is written and the failure comes back as it is,
/// for the caller to print as a document of its own. If it fails later, the document is closed
/// with the error in it instead of being cut off, and the failure comes back marked `printed`,
/// even when stdout loses that end: [`ended`] then tells it. If a write fails, the document stops
/// there, and that failure comes back.
pub fn stream_document<W: Write>(
    out: &mut W,
    opening: &str,
    walk: impl FnOnce(&mut dyn FnMut(&dyn fmt::Display) -> Result<(), Failure>) -> Result<(), Failure>,
    end: impl FnOnce() -> (Value, Vec<Warning>),
) -> Result<Vec<Warning>, Failure> {
    let (mut begun, mut cut) = (false, false);
    let mut item = |value: &dyn fmt::Display| -> Result<(), Failure> {
        let wrote = write!(out, "{}{value}", if begun { "," } else { opening });
        cut |= wrote.is_err();
        wrote?;
        begun = true;
        Ok(())
    };
    match walk(&mut item) {
        Err(failure) if !begun || cut => Err(failure),
        walked => {
            let (more, warnings) = end();
            if !begun {
                write!(out, "{opening}")?;
            }
            let closing = document_end(more, &warnings, walked.as_ref().err());
            let closed = writeln!(out, "{closing}");
            match walked {
                Ok(()) => closed.map(|()| warnings).map_err(Failure::from),
                Err(failure) => Err(Failure {
                    printed: true,
                    ..failure
                }),
            }
        }
    }
}

/// Prints `failure`, unless it was already told: with `--json`, as a document on stdout, which
/// lists `warnings`, what the command had found before it failed, and which [`ended`] tells on
/// stderr if stdout can't take it; else on stderr as text, after the warnings a failure tells.
pub fn print_error(json: bool, failure: &Failure, warnings: &[Warning]) {
    if failure.printed {
        return;
    }
    if json {
        print_out(&format!("{}\n", error_document(failure, warnings)));
        let in_document = Failure {
            printed: true,
            ..failure.clone()
        };
        let _ = ended(Some(in_document), String::new, warnings, None);
        return;
    }
    let told: Vec<Warning> = warnings
        .iter()
        .filter(|w| warning::told_on_failure(w.kind))
        .cloned()
        .collect();
    print_warnings(&told);
    tell_failure(failure);
}

/// `failure`'s message and hint on stderr. Escaped once more, so that a piece nobody thought of
/// escaping still can't drive the terminal; only the newlines Bolt writes survive, the pieces
/// having had theirs escaped.
fn tell_failure(failure: &Failure) {
    for line in failure_lines(style::err(), failure) {
        tell(&line);
    }
}

/// The lines `failure` is told in on `screen`: `error:` and its message, then `hint:` after an
/// empty line.
pub fn failure_lines(screen: style::Screen, failure: &Failure) -> Vec<String> {
    let mut told = screen.prefixed(Role::Error, "error:", &escape(&failure.message, true));
    if let Some(hint) = &failure.hint {
        told += "\n\n";
        told += &screen.prefixed(Role::Hint, "hint:", &escape(hint, true));
    }
    told.split('\n').map(str::to_owned).collect()
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
    zoned(time_ms, tz_offset_min, "%Y-%m-%d")
}

/// The date, time and zone of `time_ms`, in the save's own zone: `2026-11-08 12:14 +01:00`.
pub fn date_time(time_ms: i64, tz_offset_min: i16) -> String {
    zoned(time_ms, tz_offset_min, "%Y-%m-%d %H:%M %:z")
}

fn zoned(time_ms: i64, tz_offset_min: i16, format: &str) -> String {
    let zoned = jiff::Timestamp::from_millisecond(time_ms).and_then(|ts| {
        let offset = jiff::tz::Offset::from_seconds(i32::from(tz_offset_min) * 60)?;
        Ok(ts.to_zoned(jiff::tz::TimeZone::fixed(offset)))
    });
    // A time outside the calendar can't be shown as a date; show the raw number instead.
    zoned.map_or_else(
        |_| format!("{time_ms} ms"),
        |z| z.strftime(format).to_string(),
    )
}

/// What `bolt discard` did, for people; `everywhere` when it was given no paths.
pub fn discarded(done: &DiscardOutcome, everywhere: bool) -> String {
    let s = style::out();
    let n = done.restored.len() + done.removed.len();
    // One file is named; folders count only where no file does.
    let what = match (n, done.restored.iter().chain(&done.removed).next()) {
        (1, Some(path)) => s.paint(Role::Path, &escape(path, false)),
        (0, _) => counted(done.folders as u64, "folder", "folders"),
        _ => files(n),
    };
    // A deleted file put back loses nothing, so it needs no copy.
    let copies = match done.safety_copies {
        0 => None,
        k if k == n && n == 1 => Some("A safety copy of it was kept.".to_owned()),
        k if k == n => Some("A safety copy of each was kept.".to_owned()),
        1 => Some("A safety copy of the 1 that held unsaved work was kept.".to_owned()),
        k => Some(format!(
            "Safety copies of the {} that held unsaved work were kept.",
            thousands(k as u64)
        )),
    };
    let copies = copies.map_or(String::new(), |c| format!("\n{}\n", s.detail(None, &c)));
    let nothing = n == 0 && done.folders == 0;
    // The warning that follows names what stayed and says why.
    let said = match (nothing, everywhere) {
        (true, true) if !done.applied.changed_since_plan.is_empty() => {
            "Nothing was discarded: your files changed while Bolt was discarding them. Run bolt \
             discard again."
                .to_owned()
        }
        (true, false) if !done.applied.changed_since_plan.is_empty() => {
            "Nothing was discarded there: your files changed while Bolt was discarding them. Run \
             the same bolt discard again."
                .to_owned()
        }
        (true, _) if done.links_left.len() == 1 => {
            "Nothing was discarded: the unsaved link stays as it is.".to_owned()
        }
        (true, _) if !done.links_left.is_empty() => {
            "Nothing was discarded: the unsaved links stay as they are.".to_owned()
        }
        (true, true) => "Nothing to discard: your files match your last save.".to_owned(),
        (true, false) => "Nothing to discard there: it matches your last save.".to_owned(),
        _ => {
            let threw = s.paint(Role::Ok, "Threw away");
            return format!("{threw} your unsaved changes to {what}.\n{copies}");
        }
    };
    said + "\n"
}

/// What `bolt tidy` did, for people: what it freed, then the packs it left young.
pub fn tidied(report: &TidyReport) -> String {
    let s = style::out();
    let freed = report.bytes_before.saturating_sub(report.bytes_after);
    let held = bytes(report.bytes_after);
    // A routine tidy rewrites small packs it keeps whole, which frees nothing.
    let text = if report.to_free == 0 && freed == 0 {
        format!("Nothing to free: storage holds {held}.\n")
    } else if report.packs_removed == 0 {
        // The warnings that follow say why each pack stayed.
        format!("Nothing could be freed: storage still holds {held}.\n")
    } else {
        let packs = counted(report.packs_removed as u64, "pack", "packs");
        let packs = format!("({packs} removed, {} written)", report.packs_written);
        format!(
            "{} {}  {}\n",
            s.paint(Role::Ok, "Freed"),
            bytes(freed),
            s.paint(Role::Dim, &packs)
        )
    };
    let left = match report.packs_young {
        0 => return text,
        1 => "Left 1 pack changed in the last 24 hours as it is: a later bolt tidy frees what it \
              holds that nothing needs."
            .to_owned(),
        n => format!(
            "Left {} changed in the last 24 hours as they are: a later bolt tidy frees what they \
             hold that nothing needs.",
            counted(n as u64, "pack", "packs")
        ),
    };
    text + "\n" + &s.detail(None, &left) + "\n"
}

/// How many problems of a kind `bolt verify` lists on screen.
const PROBLEMS_SHOWN: usize = 20;

/// `n`, then `one` or `many`.
pub fn counted(n: u64, one: &str, many: &str) -> String {
    format!("{} {}", thousands(n), if n == 1 { one } else { many })
}

/// `bolt verify`'s outcome line, for `--json`'s `error` too.
pub fn verify_outcome(v: &Verified, project: &str) -> String {
    let problems = counted(v.problems_total, "problem", "problems");
    format!("Checked {project}: {problems} found.")
}

/// What to do about what `bolt verify` found: v0.1 can't repair.
pub fn verify_hint(v: &Verified) -> String {
    let general = "Bolt can't repair a project in this version. Saves that don't use the damaged \
                   or missing data still work; keep any other copy of this project safe.";
    if v.head_lost > 0 {
        let lost = bolt_core::repo::RepoError::HeadLost {
            detail: String::new(),
        };
        if let Some(hint) = lost.hint() {
            return format!("{hint} {general}");
        }
    }
    general.to_owned()
}

/// What `bolt verify` found, for people: the outcome line, then a line for each check, or the
/// problems it found, grouped by kind. Headers count every problem found, kept or not.
pub fn verified(v: &Verified, project: &str) -> String {
    let s = style::out();
    let mut text = if v.problems_total == 0 {
        format!(
            "{} {project}: no problems found.\n\n",
            s.paint(Role::Ok, "Verified")
        )
    } else {
        format!("{}\n\n", verify_outcome(v, project))
    };
    let check = s.paint(Role::Ok, s.sym(Sym::Check));
    let cross = s.paint(Role::Error, s.sym(Sym::Cross));
    // The packs' listing and the needs check, when they fail, have a line each instead of a row.
    let rows = |kind: ProblemKind| -> (Vec<&Problem>, u64) {
        let rows: Vec<&Problem> = v
            .problems
            .iter()
            .filter(|p| p.kind == kind && (p.pack.is_some() || p.object.is_some()))
            .collect();
        let unlisted = v.unlisted.is_some() && kind == ProblemKind::PackUnreadable;
        let unchecked = v.needs_failure.as_ref().is_some_and(|(k, _)| *k == kind);
        let alone = u64::from(unlisted) + u64::from(unchecked);
        (rows, v.count(kind).saturating_sub(alone))
    };
    let (packs, unread, damaged) = (
        rows(ProblemKind::PackDamaged),
        rows(ProblemKind::PackUnreadable),
        rows(ProblemKind::ObjectDamaged),
    );

    // The stored objects.
    let matched = v.objects.saturating_sub(v.objects_failed);
    let stored = format!(
        "{} ({}) {}",
        counted(v.packs, "pack", "packs"),
        bytes(v.bytes),
        if matched == 1 {
            "matches its fingerprint"
        } else {
            "match their fingerprints"
        }
    );
    let objects = counted(v.objects, "stored object", "stored objects");
    if let Some(why) = &v.unlisted {
        let _ = writeln!(text, "{}", s.detail(Some(&cross), why));
    } else if v.objects_failed == 0 && (packs.1 == 0 && unread.1 == 0 || v.objects > 0) {
        let _ = writeln!(
            text,
            "{}",
            s.detail(Some(&check), &format!("{objects} in {stored}"))
        );
    } else if matched > 0 {
        let line = format!("{} of {objects} in {stored}", thousands(matched));
        let _ = writeln!(text, "{}", s.detail(Some(&check), &line));
    }
    let mut unshown = group(
        &mut text,
        s,
        &packs,
        ("pack is damaged:", "packs are damaged:"),
    );
    let reading = ("problem reading packs:", "problems reading packs:");
    unshown |= group(&mut text, s, &unread, reading);
    let objects_damaged = ("object is damaged:", "objects are damaged:");
    unshown |= group(&mut text, s, &damaged, objects_damaged);

    // What the saves, lines and shelves need; the diary.
    if v.needs_checked {
        let missing = rows(ProblemKind::ObjectMissing);
        if missing.1 == 0 {
            let line = format!(
                "{}, {} and {} point to objects that exist",
                counted(v.saves, "save", "saves"),
                counted(v.lines, "line", "lines"),
                counted(v.shelves, "shelf", "shelves")
            );
            let _ = writeln!(text, "{}", s.detail(Some(&check), &line));
        }
        let needed = (
            "object the project needs is missing:",
            "objects the project needs are missing:",
        );
        unshown |= group(&mut text, s, &missing, needed);
        let entries = counted(v.entries, "entry", "entries");
        let _ = writeln!(
            text,
            "{}",
            s.detail(
                Some(&check),
                &format!("the undo diary ({entries}) is intact")
            )
        );
    } else {
        let why = v
            .needs_failure
            .as_ref()
            .map_or("What the project needs couldn't be checked.", |(_, why)| {
                why.as_str()
            });
        let _ = writeln!(text, "{}", s.detail(Some(&cross), why));
    }
    if unshown {
        let json = s.command("bolt verify --json");
        let which = if v.problems_total <= PROBLEMS_KEPT as u64 {
            "them all".to_owned()
        } else {
            format!("the first {}", thousands(PROBLEMS_KEPT as u64))
        };
        let _ = writeln!(
            text,
            "{}",
            s.detail(None, &format!("{json} lists {which}."))
        );
    }
    text
}

/// A `✗` line counting all `found.1` problems of a kind, then `header` (one, many), and a row
/// for each of the first 20 kept, `found.0`. Gives whether some weren't shown.
fn group(
    text: &mut String,
    s: style::Screen,
    found: &(Vec<&Problem>, u64),
    header: (&str, &str),
) -> bool {
    let (rows, n) = (&found.0, found.1);
    if n == 0 {
        return false;
    }
    let cross = s.paint(Role::Error, s.sym(Sym::Cross));
    let _ = writeln!(
        text,
        "{}",
        s.detail(Some(&cross), &counted(n, header.0, header.1))
    );
    let shown = rows.len().min(PROBLEMS_SHOWN);
    for p in &rows[..shown] {
        let _ = writeln!(text, "        {}", problem_row(s, p));
    }
    let more = n.saturating_sub(shown as u64);
    if more > 0 {
        let cut = s.sym(Sym::Cut);
        let _ = writeln!(text, "        {cut} and {} more", thousands(more));
    }
    more > 0
}

/// A problem's row: what it is about, its message, and what needs it.
fn problem_row(s: style::Screen, p: &Problem) -> String {
    let (label, plain) = match (p.object, &p.pack) {
        (Some(id), _) => (s.paint(Role::Fingerprint, &id.short_hex()), id.short_hex()),
        (None, Some(pack)) => {
            let pack = escape(pack, false);
            (s.paint(Role::Path, &pack), pack)
        }
        (None, None) => return p.message.clone(),
    };
    let mut row = format!("{label}  {}", p.message);
    if let Some((save, path)) = &p.needed_by {
        let save = save.short_hex();
        let at = if path.is_empty() {
            "its top folder".to_owned()
        } else {
            let said = format!("{plain}  {} Save {save} needs it at .", p.message);
            let room = s.line_room().saturating_sub(8 + style::columns(&said));
            s.paint(Role::Path, &s.fit_path(&escape(path, false), room))
        };
        let save = s.paint(Role::Fingerprint, &save);
        let _ = write!(row, " Save {save} needs it at {at}.");
    }
    row
}

/// `bolt verify --json`'s document, without `error`.
pub fn verified_json(v: &Verified) -> Value {
    let problems: Vec<Value> = v
        .problems
        .iter()
        .map(|p| {
            let needed_by = p.needed_by.as_ref().map_or(
                Value::Null,
                |(save, path)| json!({"save": save.to_hex(), "path": path}),
            );
            json!({
                "kind": p.kind.name(),
                "pack": p.pack,
                "object": p.object.map(|id| id.to_hex()),
                "needed_by": needed_by,
                "message": p.message,
            })
        })
        .collect();
    json!({
        "bolt_json": BOLT_JSON,
        "packs": v.packs,
        "objects": v.objects,
        "bytes": v.bytes,
        "saves": v.saves,
        "entries": v.entries,
        "problems_total": v.problems_total,
        "problems": problems,
        "warnings": warnings_json(&v.warnings),
    })
}

/// `hint` on stderr, after an empty line when something came before it.
pub fn tell_hint(hint: &str) {
    if seen_any() {
        tell("");
    }
    tell(&style::err().prefixed(Role::Hint, "hint:", &escape(hint, true)));
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;

    /// A problem about neither a pack nor an object, of `kind`.
    fn unmade(kind: ProblemKind, message: &str) -> Problem {
        Problem {
            kind,
            pack: None,
            object: None,
            needed_by: None,
            message: message.into(),
        }
    }

    #[test]
    fn an_error_in_json_is_escaped_as_on_screen() {
        let failure = Failure {
            message: "Can't read a\u{1b}[2Jb.\nSecond line.".into(),
            hint: Some("Delete \u{202e}it.".into()),
            printed: false,
        };
        let error = error_json(&failure);
        assert_eq!(
            error,
            json!({"message": "Can't read a\\x1b[2Jb.\nSecond line.", "hint": "Delete \\u{202e}it."})
        );
        // Escaping what is escaped changes nothing.
        let again = Failure {
            message: error["message"].as_str().unwrap().into(),
            hint: error["hint"].as_str().map(Into::into),
            printed: false,
        };
        assert_eq!(error_json(&again), error);
    }

    #[test]
    fn values_bolt_chooses_are_written_with_underscores() {
        let change = FileChange {
            path: "sub",
            change: ChangeKind::KindChanged,
            kind: EntryKind::ProjectLink,
            old_kind: Some(EntryKind::File),
            old_size: Some(1),
            new_size: Some(0),
            old_id: None,
            new_id: None,
        };
        let doc = change_json(&change);
        assert_eq!(
            (&doc["change"], &doc["kind"]),
            (&json!("kind_changed"), &json!("project_link"))
        );
        for (lines, content) in [
            (Lines::TooBig, "too_big"),
            (Lines::TooManyLines, "too_many_lines"),
        ] {
            let shown = LinesJson {
                change: &change,
                lines: &lines,
            }
            .to_string();
            let doc: Value = serde_json::from_str(&shown).unwrap();
            assert_eq!(doc["content"], content, "{doc}");
        }
    }

    #[test]
    fn shelf_json_tells_its_own_race_from_nothing_to_shelve() {
        for changed_again in [true, false] {
            let done = Shelved::Nothing {
                changed_again,
                links_left: vec![],
            };
            let document = shelved_json(&done, &[]);
            assert_eq!(document["shelf"], Value::Null);
            assert_eq!(document["changed_again"], changed_again);
            assert_eq!(document["written"], 0);
        }
        let made = Shelved::Made(bolt_core::shelf::ShelfOutcome {
            label: "1".into(),
            save: Id([7; 32]),
            files: 2,
            applied: Default::default(),
            links_left: vec![],
        });
        let document = shelved_json(&made, &[]);
        assert_eq!(document["shelf"]["label"], "1");
        assert_eq!(document["changed_again"], false);
    }

    #[test]
    fn a_needs_check_that_could_not_be_made_shows_no_marks_for_it() {
        let why = "What the project needs couldn't be checked: no permission.";
        let v = Verified {
            packs: 1,
            objects: 3,
            problems_total: 1,
            pack_unreadable: 1,
            problems: vec![unmade(ProblemKind::PackUnreadable, why)],
            needs_failure: Some((ProblemKind::PackUnreadable, why.into())),
            ..Verified::default()
        };
        let shown = verified(&v, "Skyrunner");
        let lines: Vec<&str> = shown.lines().collect();
        assert_eq!(lines[0], "Checked Skyrunner: 1 problem found.");
        assert!(
            lines[2].starts_with("    + 3 stored objects in 1 pack"),
            "{shown}"
        );
        assert_eq!(lines[3], format!("    x {why}"));
        assert_eq!(lines.len(), 4, "{shown}");
    }

    #[test]
    fn packs_that_could_not_be_listed_get_no_check_mark() {
        let unlisted = "Bolt couldn't list the packs in .bolt/store/packs: no permission.";
        let lost = "Bolt can't tell which diary entry is the project's current one.";
        let v = Verified {
            problems_total: 2,
            pack_unreadable: 1,
            head_lost: 1,
            problems: vec![
                unmade(ProblemKind::PackUnreadable, unlisted),
                unmade(ProblemKind::HeadLost, lost),
            ],
            unlisted: Some(unlisted.into()),
            needs_failure: Some((ProblemKind::HeadLost, lost.into())),
            ..Verified::default()
        };
        let shown = verified(&v, "Skyrunner");
        let lines: Vec<&str> = shown.lines().collect();
        assert_eq!(lines[2], format!("    x {unlisted}"), "{shown}");
        assert_eq!(lines[3], format!("    x {lost}"), "{shown}");
        assert_eq!(lines.len(), 4, "{shown}");
        assert!(!shown.contains("    + "), "{shown}");
    }

    fn failure() -> Failure {
        Failure {
            message: "Can't read it.".into(),
            hint: Some("Try again.".into()),
            printed: false,
        }
    }

    fn damaged() -> Warning {
        Warning::new("pack_damaged", "pack x is damaged.")
    }

    /// Streams a document into a buffer. `walk` is the command's walk; `more` and a damaged pack
    /// are what the command has to say once the walk is over.
    fn stream(
        opening: &str,
        more: Value,
        walk: impl FnOnce(
            &mut dyn FnMut(&dyn fmt::Display) -> Result<(), Failure>,
        ) -> Result<(), Failure>,
    ) -> (String, Result<Vec<Warning>, Failure>) {
        let mut out = Vec::new();
        let ended = stream_document(&mut out, opening, walk, || (more, vec![damaged()]));
        (String::from_utf8(out).unwrap(), ended)
    }

    const SAVES: &str = r#"{"bolt_json":1,"position":{"line":"main"},"saves":["#;
    const CHANGES: &str = r#"{"bolt_json":1,"changes":["#;

    #[test]
    fn an_undone_jump_says_where_it_went_and_leaves_nothing_unsaved() {
        use bolt_core::diary::State;
        use bolt_core::writeout::Applied;
        use std::collections::BTreeMap;
        let (old, new, went) = (Id([1; 32]), Id([2; 32]), Id([3; 32]));
        let on = |save| State {
            lines: BTreeMap::from([("main".to_owned(), save)]),
            tags: None,
            position: Position::OnLine("main".into()),
            shelves: vec![],
            server_lines: None,
        };
        // A step that moved main and wrote no file, undone.
        let undone = |command: &str| Taken {
            step: Id([4; 32]),
            entry: DiaryEntry {
                parent: Some(Id([5; 32])),
                action: Action::Command,
                command: command.into(),
                who: Person {
                    name: "Ada".into(),
                    email: "a@example.com".into(),
                    time_ms: 0,
                    tz_offset_min: 0,
                },
                bolt_version: String::new(),
                state: on(new),
                folder_change: None,
            },
            action: Action::Undo(Id([4; 32])),
            before: on(new),
            after: on(old),
            effects: vec![Effect::Line {
                name: "main".into(),
                from: Some(new),
                to: Some(old),
            }],
            applied: Applied::default(),
        };
        let mut named = |id: &Id| (id.short_hex(), Some("Add b".to_owned()));
        let (old, new) = (old.short_hex(), new.short_hex());
        assert_eq!(
            step_line(&undone("undo"), Some(&went), &[], &mut named),
            (
                "Undid",
                format!("undo --to {}: main is back at {old}.", went.short_hex())
            )
        );
        // A save undone leaves its changes unsaved, and says so.
        assert_eq!(
            step_line(&undone("save"), None, &[], &mut named),
            (
                "Undid",
                format!(
                    "save {new} \"Add b\": main is back at {old}; its changes show as unsaved."
                )
            )
        );
    }

    #[test]
    fn a_stream_that_fails_after_its_first_items_ends_as_one_document_with_the_error() {
        let (out, ended) = stream(SAVES, json!({}), |each| {
            each(&json!({"message": "one"}))?;
            each(&json!({"message": "two"}))?;
            Err(failure())
        });
        // One document, nothing after it: what was printed, then the error and the warnings.
        let doc: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(
            doc["saves"],
            json!([{"message": "one"}, {"message": "two"}])
        );
        assert_eq!(
            doc["error"],
            json!({"message": "Can't read it.", "hint": "Try again."})
        );
        assert_eq!(
            doc["warnings"],
            json!([{"kind": "pack_damaged", "message": "pack x is damaged."}])
        );
        // The failure goes on to the exit code, and says it is in the document already.
        let failure = ended.unwrap_err();
        assert!(failure.printed);
        assert_eq!(failure.message, "Can't read it.");
    }

    #[test]
    fn a_stream_that_fails_before_its_first_item_writes_nothing() {
        let (out, ended) = stream(SAVES, json!({}), |_| Err(failure()));
        assert_eq!(out, "", "the caller prints the error-only document");
        let failure = ended.unwrap_err();
        assert!(!failure.printed);
        assert_eq!(failure.message, "Can't read it.");
    }

    /// Takes `room` bytes, then refuses every write.
    struct Full {
        held: Vec<u8>,
        room: usize,
    }

    impl Write for Full {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            if self.held.len() + buf.len() > self.room {
                return Err(io::Error::other("the disk is full"));
            }
            self.held.extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    const ONE: &str = r#"{"message":"one"}"#;

    /// Streams a document whose walk fails into a writer with room for the opening and `ONE`
    /// only: the failure that comes back.
    fn stream_into_full(
        walk: impl FnOnce(
            &mut dyn FnMut(&dyn fmt::Display) -> Result<(), Failure>,
        ) -> Result<(), Failure>,
    ) -> Failure {
        let mut out = Full {
            held: Vec::new(),
            room: SAVES.len() + ONE.len(),
        };
        stream_document(&mut out, SAVES, walk, || (json!({}), vec![damaged()])).unwrap_err()
    }

    #[test]
    fn a_stream_that_cant_be_written_gives_back_its_own_failure_or_the_writes() {
        // The walk failed after an item, and the end with its error couldn't be written: its
        // failure, as in the document, for `ended` to tell.
        let back = stream_into_full(|each| {
            each(&ONE)?;
            Err(failure())
        });
        assert_eq!(
            (back.message.as_str(), back.printed),
            ("Can't read it.", true)
        );
        // A write that fails stops it there: the write's failure.
        let back = stream_into_full(|each| {
            each(&ONE)?;
            each(&ONE)?;
            Err(failure())
        });
        assert_eq!(
            (back.message.as_str(), back.printed),
            ("the disk is full", false)
        );
    }

    #[test]
    fn a_finished_stream_is_the_document_serde_json_would_write() {
        let (out, ended) = stream(CHANGES, json!({"save": {"id": "x"}}), |each| {
            each(&json!({"path": "a"}))?;
            each(&json!({"path": "b"}))
        });
        let expected = json!({
            "bolt_json": 1,
            "changes": [{"path": "a"}, {"path": "b"}],
            "save": {"id": "x"},
            "warnings": [{"kind": "pack_damaged", "message": "pack x is damaged."}],
        });
        assert_eq!(out, format!("{expected}\n"));
        assert_eq!(ended.unwrap(), vec![damaged()]);
    }

    #[test]
    fn a_stream_with_no_items_is_still_a_whole_document() {
        let (out, ended) = stream(CHANGES, json!({"save": {"id": "x"}}), |_| Ok(()));
        let doc: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(doc["changes"], json!([]));
        assert_eq!(doc["save"], json!({"id": "x"}));
        assert!(ended.is_ok());
    }

    #[test]
    fn an_error_document_carries_the_warnings_found_before_the_failure() {
        assert_eq!(
            error_document(&failure(), &[damaged()]),
            json!({
                "bolt_json": 1,
                "error": {"message": "Can't read it.", "hint": "Try again."},
                "warnings": [{"kind": "pack_damaged", "message": "pack x is damaged."}],
            })
        );
        // With nothing found, the list is there and empty.
        assert_eq!(error_document(&failure(), &[])["warnings"], json!([]));
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
        assert_eq!(date_time(90 * 60_000, 60), "1970-01-01 02:30 +01:00");
    }

    #[test]
    fn tidy_says_what_it_freed_and_what_it_left_young() {
        let report = TidyReport {
            to_free: 3000,
            bytes_before: 5000,
            bytes_after: 2000,
            packs_removed: 1,
            packs_written: 1,
            packs_young: 1,
        };
        assert_eq!(
            tidied(&report),
            "Freed 2.9 KiB  (1 pack removed, 1 written)\n\n    Left 1 pack changed in the last 24 \
             hours as it is: a later bolt tidy frees what it holds that nothing needs.\n"
        );
        let nothing = TidyReport {
            bytes_before: 2048,
            bytes_after: 2048,
            ..TidyReport::default()
        };
        assert_eq!(
            tidied(&nothing),
            "Nothing to free: storage holds 2.0 KiB.\n"
        );
        // Small packs rewritten whole free nothing: nothing to free, whatever was rewritten.
        let rewritten = TidyReport {
            packs_removed: 2,
            packs_written: 1,
            ..nothing
        };
        assert_eq!(
            tidied(&rewritten),
            "Nothing to free: storage holds 2.0 KiB.\n"
        );
        // A small pack rewritten under its own name, so none removed, is nothing to free too.
        let renamed_alike = TidyReport {
            packs_written: 1,
            ..nothing
        };
        assert_eq!(
            tidied(&renamed_alike),
            "Nothing to free: storage holds 2.0 KiB.\n"
        );
        // Merged packs that do free something say so.
        let merged = TidyReport {
            to_free: 100,
            bytes_before: 2148,
            ..rewritten
        };
        assert_eq!(
            tidied(&merged),
            "Freed 100 B  (2 packs removed, 1 written)\n"
        );
        // Something to free, but no pack could be removed.
        let stuck = TidyReport {
            to_free: 1024,
            packs_written: 1,
            ..nothing
        };
        assert_eq!(
            tidied(&stuck),
            "Nothing could be freed: storage still holds 2.0 KiB.\n"
        );
    }

    #[test]
    fn a_failed_write_is_kept_and_refuses_the_writes_after_it() {
        let failed = Failed::new();
        let fine = || Ok(1);
        // A reader gone, or a write interrupted and tried again, isn't kept.
        for kind in [io::ErrorKind::BrokenPipe, io::ErrorKind::Interrupted] {
            let gone = guarded(&failed, || Err::<usize, _>(io::Error::from(kind)));
            assert_eq!(gone.unwrap_err().kind(), kind);
            assert_eq!(guarded(&failed, fine).unwrap(), 1);
        }
        // Any other failure is, and a write that would go through after it is refused with it.
        let full = guarded(&failed, || {
            Err::<usize, _>(io::Error::other("the disk is full"))
        });
        let full = full.unwrap_err();
        assert_eq!(
            full.to_string(),
            "Bolt couldn't write its output: the disk is full"
        );
        let refused = guarded(&failed, fine).unwrap_err();
        assert_eq!(
            (refused.kind(), refused.to_string()),
            (full.kind(), full.to_string())
        );
    }

    #[test]
    fn a_discard_whose_files_all_changed_again_says_nothing_was_discarded() {
        use bolt_core::writeout::Applied;
        let nothing = DiscardOutcome {
            restored: vec![],
            removed: vec![],
            folders: 0,
            applied: Applied::default(),
            safety_copies: 0,
            folder_change: None,
            links_left: vec![],
        };
        assert_eq!(
            discarded(&nothing, true),
            "Nothing to discard: your files match your last save.\n"
        );
        // Every file it planned to throw away changed before it was written: they don't match.
        let changed_again = DiscardOutcome {
            applied: Applied {
                changed_since_plan: vec!["a.txt".into()],
                ..Applied::default()
            },
            ..nothing
        };
        assert_eq!(
            discarded(&changed_again, true),
            "Nothing was discarded: your files changed while Bolt was discarding them. Run bolt \
             discard again.\n"
        );
        // Given paths, it says the same of them, so running it again throws away no more.
        assert_eq!(
            discarded(&changed_again, false),
            "Nothing was discarded there: your files changed while Bolt was discarding them. Run \
             the same bolt discard again.\n"
        );
    }
}
