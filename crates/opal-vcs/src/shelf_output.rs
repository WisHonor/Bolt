//! What `opal shelf`, `opal shelf --list` and `opal unshelf` show people.

use opal_core::shelf::{ShelfInfo, ShelfOutcome, UnshelfOutcome};
use opal_core::text::{escape, thousands};

use crate::output::{self, counted, relative_time};
use crate::style::{self, Role};

/// What `opal shelf` set aside, and on which shelf.
pub fn shelved(made: &ShelfOutcome) -> String {
    let what = match made.files {
        0 => String::new(),
        n => format!(" in {}", output::files(n)),
    };
    format!(
        "{} your unsaved changes{what} on shelf {}.\n",
        style::out().paint(Role::Ok, "Shelved"),
        escape(&made.label, false)
    )
}

/// What `opal unshelf` brought back; `title` is the shelf's message, quoted, if it could be read.
pub fn unshelved(done: &UnshelfOutcome, title: &str) -> String {
    let (written, removed) = (done.applied.written, done.applied.removed);
    let what = match (written, removed) {
        (0, 0) => "your files already held its changes".to_owned(),
        (0, removed) => format!("{} removed", output::files(removed)),
        (written, 0) => format!("{} written", output::files(written)),
        (written, removed) => format!(
            "{} written, {} removed",
            output::files(written),
            thousands(removed as u64)
        ),
    };
    let mut text = format!(
        "{} shelf {}{title}: {what}.\n",
        style::out().paint(Role::Ok, "Unshelved"),
        escape(&done.label, false)
    );
    if done.carried > 0 {
        let stayed = format!(
            "Your other unsaved changes in {} stayed as they are.",
            output::files(done.carried)
        );
        text += &format!("\n{}\n", style::out().detail(None, &stayed));
    }
    text
}

/// The shelves, newest first, in columns: the number `opal unshelf` takes, the message's first
/// line, then how many files, when, and where it was made. On a terminal the message is cut at
/// its end to fit.
pub fn listed(list: &[ShelfInfo], now_ms: i64) -> String {
    if list.is_empty() {
        return "No shelves: opal shelf sets your unsaved changes aside on one.\n".to_owned();
    }
    let s = style::out();
    let rows: Vec<[String; 5]> = list
        .iter()
        .map(|shelf| {
            let files = match shelf.files {
                0 => "only folders".to_owned(),
                n => output::files(n),
            };
            let at = match &shelf.line {
                Some(line) => format!("on {}", escape(line, false)),
                None => "at a save".to_owned(),
            };
            [
                escape(&shelf.label, false),
                escape(shelf.message.lines().next().unwrap_or_default(), false),
                files,
                relative_time(shelf.saver.time_ms, shelf.saver.tz_offset_min, now_ms),
                at,
            ]
        })
        .collect();
    let widest = |i: usize| {
        (rows.iter())
            .map(|row| style::columns(&row[i]))
            .max()
            .unwrap_or(0)
    };
    let [label, title, files, when] = [0, 1, 2, 3].map(widest);
    let others = 4 + label + 2 + 2 + files + 2 + when + 2 + widest(4);
    // The quotes take two columns.
    let title = (title + 2).min(s.width.map_or(usize::MAX, |w| w.saturating_sub(others)));
    let mut text = format!(
        "{}, newest first:\n\n",
        counted(list.len() as u64, "shelf", "shelves")
    );
    for [number, message, held, made, at] in &rows {
        let quoted = format!("\"{}\"", s.cut_end(message, title.saturating_sub(2)));
        let facts = format!("{held:<files$}  {made:<when$}  {at}");
        text += &format!(
            "    {number:>label$}  {quoted:<title$}  {}\n",
            s.paint(Role::Dim, &facts)
        );
    }
    text
}
