//! Progress on stderr, only on a terminal: a bar when the total is known, a turning mark when it
//! isn't. Nothing shows for the first 400 ms, and the line is erased when the work ends.

use std::io::{self, Write};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use bolt_core::text::{bytes, thousands};

use crate::style::{self, Screen, Sym, columns};

/// How long work goes before progress shows: a quick command shows none.
const QUIET: Duration = Duration::from_millis(400);
/// The shortest time between two drawings.
const EVERY: Duration = Duration::from_millis(100);
const LABEL: usize = 24;
const CELLS: usize = 24;

/// A bar's line: the label padded to 24, the cells, the percentage in 4, then done and total.
/// Under 70 columns the counts go; under 50 the cells go too.
pub fn bar_line(label: &str, done: u64, total: u64, as_bytes: bool, screen: Screen) -> String {
    let width = screen.width.unwrap_or(usize::MAX);
    let done = done.min(total);
    let part = |of: u64| match total {
        0 => of,
        _ => u64::try_from(u128::from(done) * u128::from(of) / u128::from(total)).unwrap_or(of),
    };
    let mut line = format!("{:<LABEL$}", screen.cut_end(label, LABEL));
    if width >= 50 {
        let full = usize::try_from(part(CELLS as u64)).unwrap_or(CELLS);
        line.push(' ');
        line += &screen.sym(Sym::Full).repeat(full);
        line += &screen.sym(Sym::Empty).repeat(CELLS - full);
    }
    line += &format!(" {:>3}%", part(100));
    if width >= 70 {
        let count = |n| if as_bytes { bytes(n) } else { thousands(n) };
        line += &format!("  {} / {}", count(done), count(total));
    }
    line
}

/// The turning mark's line at its `step`th turn.
fn turning_line(label: &str, step: usize, screen: Screen) -> String {
    format!("{label} {} {}", screen.sym(Sym::Cut), screen.turn(step))
}

/// One line drawn in place, and how many columns of it are on the screen.
struct Drawn {
    screen: Screen,
    shown: usize,
}

impl Drawn {
    /// Draws `text` over what was there, kept short of the last column so it never wraps. A
    /// failed write only loses the progress line, so it is let go.
    fn draw(&mut self, out: &mut dyn Write, text: &str) {
        let room = self.screen.width.unwrap_or(usize::MAX).saturating_sub(1);
        let text: String = text.chars().take(room).collect();
        let left = self.shown.saturating_sub(columns(&text));
        let _ = write!(out, "\r{text}{}", " ".repeat(left));
        let _ = out.flush();
        self.shown = columns(&text);
    }

    /// Takes the line off the screen, leaving the cursor at its start.
    fn erase(&mut self, out: &mut dyn Write) {
        if self.shown == 0 {
            return;
        }
        // A terminal that reads colour codes reads this one too.
        let _ = if self.screen.color {
            write!(out, "\r\x1b[2K")
        } else {
            write!(out, "\r{}\r", " ".repeat(self.shown))
        };
        let _ = out.flush();
        self.shown = 0;
    }
}

/// A progress line for work that says how far it is.
pub struct Bar<W: Write = io::Stderr> {
    out: W,
    on: bool,
    label: String,
    began: Instant,
    last: Option<Instant>,
    turns: usize,
    drawn: Drawn,
}

impl Bar {
    pub fn new(label: &str) -> Bar {
        Bar::to(
            io::stderr(),
            style::err(),
            style::for_people(),
            label,
            Instant::now(),
        )
    }
}

impl<W: Write> Bar<W> {
    /// A bar written to `out`, showing only when `on`, for work that began at `began`.
    fn to(out: W, screen: Screen, on: bool, label: &str, began: Instant) -> Self {
        Bar {
            out,
            on,
            label: label.to_owned(),
            began,
            last: None,
            turns: 0,
            drawn: Drawn { screen, shown: 0 },
        }
    }

    pub fn set_label(&mut self, label: &str) {
        if self.label != label {
            label.clone_into(&mut self.label);
        }
    }

    /// How far the work is: `done` of `total`, or a turning mark while the total isn't known.
    /// `as_bytes` shows the counts as sizes.
    pub fn update(&mut self, done: u64, total: Option<u64>, as_bytes: bool) {
        self.update_at(Instant::now(), done, total, as_bytes);
    }

    fn update_at(&mut self, now: Instant, done: u64, total: Option<u64>, as_bytes: bool) {
        let soon = |since: Instant, wait| now.saturating_duration_since(since) < wait;
        if !self.on || soon(self.began, QUIET) || self.last.is_some_and(|t| soon(t, EVERY)) {
            return;
        }
        self.last = Some(now);
        let screen = self.drawn.screen;
        let text = match total {
            Some(total) => bar_line(&self.label, done, total, as_bytes, screen),
            None => {
                let step = self.turns;
                self.turns = step.wrapping_add(1);
                turning_line(&self.label, step, screen)
            }
        };
        self.drawn.draw(&mut self.out, &text);
    }

    /// Erases the line: the outcome line that follows is the summary.
    pub fn finish(self) {}
}

impl<W: Write> Drop for Bar<W> {
    fn drop(&mut self) {
        self.drawn.erase(&mut self.out);
    }
}

/// The turning mark on the screen, shared by its thread and the lines told while it turns.
struct Mark {
    /// The turning mark that may draw, by the number it started with; none when 0.
    turning: u64,
    started: u64,
    paused: bool,
    drawn: Drawn,
}

static MARK: Mutex<Mark> = Mutex::new(Mark {
    turning: 0,
    started: 0,
    paused: false,
    drawn: Drawn {
        screen: Screen {
            color: false,
            symbols: false,
            width: None,
        },
        shown: 0,
    },
});

fn mark() -> MutexGuard<'static, Mark> {
    MARK.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A turning mark with what a command is doing, drawn on stderr by a thread of its own while the
/// command works. Dropping it erases the line.
pub struct Working {
    on: bool,
}

impl Working {
    /// Starts the turning mark; nothing shows when stderr is no terminal.
    pub fn start(label: &'static str) -> Working {
        if !style::for_people() {
            return Working { on: false };
        }
        let mut m = mark();
        m.started += 1;
        let number = m.started;
        m.turning = number;
        m.paused = false;
        m.drawn.screen = style::err();
        drop(m);
        // A thread that couldn't start only loses the mark.
        let _ = thread::Builder::new().spawn(move || {
            thread::sleep(QUIET);
            for step in 0.. {
                {
                    let mut m = mark();
                    if m.turning != number {
                        return;
                    }
                    if !m.paused {
                        let line = turning_line(label, step, m.drawn.screen);
                        m.drawn.draw(&mut io::stderr(), &line);
                    }
                }
                thread::sleep(EVERY);
            }
        });
        Working { on: true }
    }

    /// Takes the mark off the screen until [`Working::resume`]: before a question.
    pub fn pause(&self) {
        if self.on {
            let mut m = mark();
            m.paused = true;
            m.drawn.erase(&mut io::stderr());
        }
    }

    pub fn resume(&self) {
        if self.on {
            mark().paused = false;
        }
    }
}

impl Drop for Working {
    fn drop(&mut self) {
        if self.on {
            let mut m = mark();
            m.turning = 0;
            m.drawn.erase(&mut io::stderr());
        }
    }
}

/// Writes `line` on stderr with the turning mark off the screen first; the mark comes back below
/// it on its next turn.
pub fn tell(line: &str) {
    let mut m = mark();
    let mut err = io::stderr().lock();
    m.drawn.erase(&mut err);
    let _ = writeln!(err, "{line}");
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;

    fn screen(symbols: bool, width: usize) -> Screen {
        Screen {
            color: false,
            symbols,
            width: Some(width),
        }
    }

    #[test]
    fn a_bar_line_has_the_label_padded_to_24_then_24_cells_the_percentage_and_the_counts() {
        assert_eq!(
            bar_line("Importing commits", 334, 1284, false, screen(true, 80)),
            "Importing commits        ██████░░░░░░░░░░░░░░░░░░  26%  334 / 1,284"
        );
        assert_eq!(
            bar_line("Checking", 3 << 30, 4 << 30, true, screen(true, 80)),
            "Checking                 ██████████████████░░░░░░  75%  3.0 GiB / 4.0 GiB"
        );
        // Nothing to do is all done.
        assert!(bar_line("Reading", 0, 0, false, screen(true, 80)).contains(" 100%  0 / 0"));
    }

    #[test]
    fn under_70_columns_the_counts_go_and_under_50_the_cells() {
        assert_eq!(
            bar_line("Importing commits", 334, 1284, false, screen(true, 69)),
            "Importing commits        ██████░░░░░░░░░░░░░░░░░░  26%"
        );
        assert_eq!(
            bar_line("Importing commits", 334, 1284, false, screen(true, 49)),
            "Importing commits         26%"
        );
    }

    #[test]
    fn a_bar_uses_plain_cells_without_symbols() {
        assert_eq!(
            bar_line("Importing commits", 1, 2, false, screen(false, 80)),
            "Importing commits        ############............  50%  1 / 2"
        );
    }

    #[test]
    fn a_long_label_is_cut_to_24() {
        let line = bar_line(&"x".repeat(30), 1, 2, false, screen(false, 80));
        assert!(
            line.starts_with(&format!("{}... #", "x".repeat(21))),
            "{line}"
        );
    }

    fn bar(out: &mut Vec<u8>, began: Instant) -> Bar<&mut Vec<u8>> {
        Bar::to(out, screen(false, 80), true, "Importing commits", began)
    }

    #[test]
    fn nothing_is_drawn_before_400_ms_or_more_often_than_every_100() {
        let t0 = Instant::now();
        let at = |ms| t0 + Duration::from_millis(ms);
        let mut out = Vec::new();
        let mut b = bar(&mut out, t0);
        b.update_at(at(399), 1, Some(4), false);
        assert!(b.out.is_empty());
        b.update_at(at(400), 1, Some(4), false);
        let once = b.out.len();
        assert!(once > 0);
        b.update_at(at(499), 2, Some(4), false);
        assert_eq!(b.out.len(), once);
        b.update_at(at(500), 2, Some(4), false);
        assert!(b.out.len() > once);
        std::mem::forget(b);
        let said = String::from_utf8(out).unwrap();
        assert_eq!(said.matches('\r').count(), 2, "{said:?}");
        assert!(said.ends_with("  50%  2 / 4"), "{said:?}");
    }

    #[test]
    fn finishing_erases_the_line() {
        let t0 = Instant::now();
        let mut out = Vec::new();
        let mut b = bar(&mut out, t0);
        b.update_at(t0 + QUIET, 1, None, false);
        b.finish();
        let said = String::from_utf8(out).unwrap();
        let drawn = "Importing commits ... |";
        assert_eq!(said, format!("\r{drawn}\r{}\r", " ".repeat(drawn.len())));
        // A bar never drawn writes nothing at its end.
        let mut out = Vec::new();
        bar(&mut out, t0).finish();
        assert!(out.is_empty());
    }

    #[test]
    fn the_turning_mark_turns_and_a_shorter_line_covers_a_longer_one() {
        let s = screen(true, 80);
        assert_eq!(turning_line("Tidying", 0, s), "Tidying … ⠋");
        assert_eq!(
            turning_line("Tidying", 10, screen(false, 80)),
            "Tidying ... -"
        );
        let mut drawn = Drawn {
            screen: screen(false, 10),
            shown: 0,
        };
        let mut out = Vec::new();
        drawn.draw(&mut out, "a long line");
        drawn.draw(&mut out, "short");
        assert_eq!(String::from_utf8(out).unwrap(), "\ra long li\rshort    ");
        // With colour, the line is cleared whole.
        let mut out = Vec::new();
        drawn.screen.color = true;
        drawn.erase(&mut out);
        assert_eq!(out, b"\r\x1b[2K");
    }
}
