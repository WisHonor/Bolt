//! What `bolt` prints: JSON shapes, warnings and error messages.

use std::fmt::{self, Write as _};
use std::io::{self, IsTerminal, Write};
use std::time::{Duration, Instant};

use bolt_core::changes::{Endings, Lines, Tag, only_line_endings};
use bolt_core::diary::Position;
use bolt_core::diff::{ChangeKind, FileChange};
use bolt_core::id::Id;
use bolt_core::save::{Person, Save};
use bolt_core::text::{escape, thousands};
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
            ChangeKind::KindChanged => "kind-changed",
        },
        "kind": c.kind.as_str(),
        "old_size": c.old_size,
        "new_size": c.new_size,
    })
}

/// A file's change as people see it, its path escaped.
pub fn change_line(c: &FileChange) -> String {
    let p = escape(c.path, false);
    let size = |s: Option<u64>| format_size(s.unwrap_or(0));
    match c.change {
        ChangeKind::Added => format!("  added     {p}  ({})", size(c.new_size)),
        ChangeKind::Removed => format!("  removed   {p}  ({})", size(c.old_size)),
        ChangeKind::Modified => format!(
            "  modified  {p}  ({} → {})",
            size(c.old_size),
            size(c.new_size)
        ),
        ChangeKind::KindChanged => {
            let old = c.old_kind.map_or("none", |k| k.as_str());
            format!("  changed   {p}  ({old} → {})", c.kind.as_str())
        }
    }
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
            Lines::TooBig => ("too-big", &[][..], false),
            Lines::TooManyLines => ("too-many-lines", &[][..], false),
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
            for (j, (tag, line)) in h.lines.iter().enumerate() {
                let text = Value::from(String::from_utf8_lossy(line).as_ref());
                write!(
                    f,
                    "{}{{\"tag\":\"{}\",\"text\":{text}}}",
                    if j == 0 { "" } else { "," },
                    tag_name(*tag)
                )?;
            }
            f.write_str("]}")?;
        }
        // The rest is small, and serde_json writes it: `{` is left out, the hunks having opened
        // the object.
        let mut rest = change_json(self.change);
        rest["lines"] = shown.into();
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
/// header starts with a letter, which no line of the file shown below it can. Colored when
/// `color`.
pub fn write_lines(
    out: &mut dyn Write,
    c: &FileChange,
    lines: &Lines,
    color: bool,
) -> io::Result<()> {
    let paint = |code: &str, text: &str| {
        if color {
            format!("\x1b[{code}m{text}\x1b[0m")
        } else {
            text.to_owned()
        }
    };
    let header = change_line(c);
    let header = header.trim_start();
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
        writeln!(out, "{}", paint("36", &at))?;
        for (tag, line) in &h.lines {
            let (mark, code) = match tag {
                Tag::Same => (' ', ""),
                Tag::Removed => ('-', "31"),
                Tag::Added => ('+', "32"),
            };
            let body = line
                .strip_suffix(b"\r\n")
                .or_else(|| line.strip_suffix(b"\n"));
            // Tabs are kept: they can't drive a terminal, and code is full of them.
            let shown: Vec<String> = String::from_utf8_lossy(body.unwrap_or(line))
                .split('\t')
                .map(|piece| escape(piece, false))
                .collect();
            let shown = format!("{mark}{}", shown.join("\t"));
            if code.is_empty() {
                writeln!(out, "{shown}")?;
            } else {
                writeln!(out, "{}", paint(code, &shown))?;
            }
            if body.is_none() {
                writeln!(out, "\\ No newline at end of file")?;
            }
        }
    }
    Ok(())
}

/// Standard output, remembering when whoever reads it went away (`bolt changes | head`), so the
/// command can end quietly instead of failing.
pub struct Stdout {
    inner: io::StdoutLock<'static>,
    pub closed: bool,
}

impl Stdout {
    pub fn new() -> Stdout {
        Stdout {
            inner: io::stdout().lock(),
            closed: false,
        }
    }

    fn noting<T>(&mut self, done: io::Result<T>) -> io::Result<T> {
        if let Err(e) = &done
            && e.kind() == io::ErrorKind::BrokenPipe
        {
            self.closed = true;
        }
        done
    }
}

impl Write for Stdout {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let done = self.inner.write(buf);
        self.noting(done)
    }

    fn flush(&mut self) -> io::Result<()> {
        let done = self.inner.flush();
        self.noting(done)
    }
}

/// Prints `text` on stdout, then `warnings` on stderr; a reader that went away ends it quietly.
pub fn print_quietly(text: &str, warnings: &[Warning]) -> Result<(), Failure> {
    let mut out = Stdout::new();
    let done = out.write_all(text.as_bytes()).and_then(|()| out.flush());
    if out.closed {
        return Ok(());
    }
    done?;
    print_warnings(warnings);
    Ok(())
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
#[derive(Debug)]
pub struct Failure {
    pub message: String,
    pub hint: Option<String>,
    /// The command's `--json` document already ends with this failure.
    pub printed: bool,
}

fn error_json(failure: &Failure) -> Value {
    json!({ "message": failure.message, "hint": failure.hint })
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
/// with the error in it instead of being cut off, and the failure comes back marked `printed`.
pub fn stream_document<W: Write>(
    out: &mut W,
    opening: &str,
    walk: impl FnOnce(&mut dyn FnMut(&dyn fmt::Display) -> Result<(), Failure>) -> Result<(), Failure>,
    end: impl FnOnce() -> (Value, Vec<Warning>),
) -> Result<Vec<Warning>, Failure> {
    let mut begun = false;
    let mut item = |value: &dyn fmt::Display| -> Result<(), Failure> {
        write!(out, "{}{value}", if begun { "," } else { opening })?;
        begun = true;
        Ok(())
    };
    match walk(&mut item) {
        Err(failure) if !begun => Err(failure),
        walked => {
            let (more, warnings) = end();
            if !begun {
                write!(out, "{opening}")?;
            }
            let closing = document_end(more, &warnings, walked.as_ref().err());
            writeln!(out, "{closing}")?;
            match walked {
                Ok(()) => Ok(warnings),
                Err(failure) => Err(Failure {
                    printed: true,
                    ..failure
                }),
            }
        }
    }
}

/// Prints `failure`: to stdout as JSON with `--json`, else to stderr as text. `warnings` are what
/// the command had found before it failed; only the JSON document lists them. A failure the
/// command's own document already ends with is not printed again.
pub fn print_error(json: bool, failure: &Failure, warnings: &[Warning]) {
    if failure.printed {
        return;
    }
    if json {
        println!("{}", error_document(failure, warnings));
    } else {
        for w in warnings.iter().filter(|w| warning::told_on_failure(w.kind)) {
            eprintln!("warning: {}", w.message);
        }
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

/// A line such as "Importing commits: 1,234 of 5,678" on stderr while a long step runs, or
/// "Reading Git's history: 1,234 commits" while it can't say yet how many there are, kept up to
/// date at most four times a second, and only in a terminal.
pub struct Progress<W: Write> {
    what: &'static str,
    /// What is counted, said after the count when there is no total.
    unit: &'static str,
    out: W,
    terminal: bool,
    since: Instant,
    last: (u64, Option<u64>),
    shown: bool,
}

impl Progress<std::io::Stderr> {
    pub fn new(what: &'static str, unit: &'static str) -> Self {
        let out = std::io::stderr();
        let terminal = out.is_terminal();
        Progress::to(what, unit, out, terminal)
    }
}

impl<W: Write> Progress<W> {
    /// Progress written to `out`, which is a terminal or isn't.
    fn to(what: &'static str, unit: &'static str, out: W, terminal: bool) -> Self {
        Progress {
            what,
            unit,
            out,
            terminal,
            since: Instant::now(),
            last: (0, None),
            shown: false,
        }
    }

    pub fn update(&mut self, done: u64, total: Option<u64>) {
        self.last = (done, total);
        if self.terminal && self.since.elapsed() >= Duration::from_millis(250) {
            self.show();
            self.since = Instant::now();
        }
    }

    /// Ends the line, with the last count, if one is shown; called again, it does nothing.
    pub fn finish(&mut self) {
        if self.shown {
            self.show();
            let _ = writeln!(self.out);
            self.shown = false;
        }
    }

    /// A failed write only loses the progress line, so it is let go.
    fn show(&mut self) {
        let (done, total) = self.last;
        let done = thousands(done);
        let _ = match total {
            Some(total) => write!(self.out, "\r{}: {done} of {}", self.what, thousands(total)),
            None => write!(self.out, "\r{}: {done} {}", self.what, self.unit),
        };
        let _ = self.out.flush();
        self.shown = true;
    }
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

    fn failure() -> Failure {
        Failure {
            message: "Can't read it.".into(),
            hint: Some("Try again.".into()),
            printed: false,
        }
    }

    fn damaged() -> Warning {
        Warning::new("damaged_pack", "pack x is damaged.")
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

    const SAVES: &str = r#"{"bolt_json":1,"line":"main","saves":["#;
    const CHANGES: &str = r#"{"bolt_json":1,"changes":["#;

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
            json!([{"kind": "damaged_pack", "message": "pack x is damaged."}])
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
            "warnings": [{"kind": "damaged_pack", "message": "pack x is damaged."}],
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
                "warnings": [{"kind": "damaged_pack", "message": "pack x is damaged."}],
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

    #[test]
    fn progress_shows_only_in_a_terminal_and_at_most_four_times_a_second() {
        let a_while_ago = || {
            Instant::now()
                .checked_sub(Duration::from_millis(300))
                .unwrap()
        };
        let importing =
            |terminal| Progress::to("Importing commits", "commits", Vec::new(), terminal);
        // Without a terminal, nothing, however long the step takes.
        let mut quiet = importing(false);
        quiet.since = a_while_ago();
        quiet.update(1, Some(2));
        quiet.finish();
        assert!(quiet.out.is_empty());
        // In one, nothing for the first quarter of a second, so a quick step shows nothing.
        let mut quick = importing(true);
        quick.update(1, Some(2));
        quick.finish();
        assert!(quick.out.is_empty());
        // Then at most once a quarter of a second, and the last count when it ends, once.
        let mut slow = importing(true);
        slow.since = a_while_ago();
        slow.update(2, Some(1234));
        slow.update(3, Some(1234));
        slow.finish();
        slow.finish();
        assert_eq!(
            String::from_utf8(slow.out).unwrap(),
            "\rImporting commits: 2 of 1,234\rImporting commits: 3 of 1,234\n"
        );
        // A step that can't say yet how many there are counts what it has done.
        let mut reading = Progress::to("Reading Git's history", "commits", Vec::new(), true);
        reading.since = a_while_ago();
        reading.update(1234, None);
        reading.finish();
        assert_eq!(
            String::from_utf8(reading.out).unwrap(),
            "\rReading Git's history: 1,234 commits\rReading Git's history: 1,234 commits\n"
        );
    }
}
