//! Progress on stderr, only on a terminal: a bar when the total is known, a turning mark when it
//! isn't. Nothing shows for the first 400 ms, and the line is erased when the work ends.

use std::io::{self, Write};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use opal_core::text::{bytes, thousands};

use crate::style::{self, Screen, Sym, columns};

/// How long work goes before progress shows: a quick command shows none.
const QUIET: Duration = Duration::from_millis(400);

/// The quiet time, which tests may set with `OPAL_PROGRESS_QUIET_MS`.
#[cfg(feature = "fault-injection")]
fn quiet() -> Duration {
    static SET: std::sync::OnceLock<Option<u64>> = std::sync::OnceLock::new();
    let set = SET.get_or_init(|| std::env::var("OPAL_PROGRESS_QUIET_MS").ok()?.parse().ok());
    set.map_or(QUIET, Duration::from_millis)
}

#[cfg(not(feature = "fault-injection"))]
fn quiet() -> Duration {
    QUIET
}
/// The shortest time between two drawings.
const EVERY: Duration = Duration::from_millis(100);
const LABEL: usize = 24;
const CELLS: usize = 24;

/// A bar's line: the label padded to 24, the cells, the percentage in 4, then done and total.
/// What doesn't fit the screen goes whole: first the counts, then the cells; the label and the
/// percentage always show, the label cut at its end if even they don't fit.
pub fn bar_line(label: &str, done: u64, total: u64, as_bytes: bool, screen: Screen) -> String {
    let room = screen.line_room();
    let done = done.min(total);
    let part = |of: u64| match total {
        0 => of,
        _ => u64::try_from(u128::from(done) * u128::from(of) / u128::from(total)).unwrap_or(of),
    };
    let full = usize::try_from(part(CELLS as u64)).unwrap_or(CELLS);
    let cells = format!(
        " {}{}",
        screen.sym(Sym::Full).repeat(full),
        screen.sym(Sym::Empty).repeat(CELLS - full)
    );
    let percent = format!(" {:>3}%", part(100));
    let count = |n| if as_bytes { bytes(n) } else { thousands(n) };
    let counts = format!("  {} / {}", count(done), count(total));
    let padded = format!("{:<LABEL$}", screen.cut_end(label, LABEL));
    [
        format!("{padded}{cells}{percent}{counts}"),
        format!("{padded}{cells}{percent}"),
        format!("{padded}{percent}"),
    ]
    .into_iter()
    .find(|line| columns(line) <= room)
    .unwrap_or_else(|| {
        let left = room.saturating_sub(columns(&percent));
        format!("{}{percent}", screen.cut_end(label, left))
    })
}

/// The turning mark's line at its `step`th turn, the label cut at its end to fit the screen.
fn turning_line(label: &str, step: usize, screen: Screen) -> String {
    let mark = format!(" {} {}", screen.sym(Sym::Cut), screen.turn(step));
    let left = screen.line_room().saturating_sub(columns(&mark));
    format!("{}{mark}", screen.cut_end(label, left))
}

/// One line drawn in place, and how many columns of it are on the screen.
struct Drawn {
    screen: Screen,
    shown: usize,
}

impl Drawn {
    /// Draws `text`, already made to fit the screen, over what was there. A failed write only
    /// loses the progress line, so it is let go.
    fn draw(&mut self, out: &mut dyn Write, text: &str) {
        let left = self.shown.saturating_sub(columns(text));
        let _ = write!(out, "\r{text}{}", " ".repeat(left));
        let _ = out.flush();
        self.shown = columns(text);
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
    quiet: Duration,
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
            quiet(),
        )
    }
}

impl<W: Write> Bar<W> {
    /// A bar written to `out`, showing only when `on`, for work that began at `began`, once it
    /// has gone for `quiet`.
    fn to(out: W, screen: Screen, on: bool, label: &str, began: Instant, quiet: Duration) -> Self {
        Bar {
            out,
            on,
            label: label.to_owned(),
            began,
            quiet,
            last: None,
            turns: 0,
            drawn: Drawn { screen, shown: 0 },
        }
    }

    /// A new label is drawn at its next update, however soon: a quick pass still shows.
    pub fn set_label(&mut self, label: &str) {
        if self.label != label {
            label.clone_into(&mut self.label);
            self.last = None;
        }
    }

    /// How far the work is: `done` of `total`, or a turning mark while the total isn't known.
    /// `as_bytes` shows the counts as sizes.
    pub fn update(&mut self, done: u64, total: Option<u64>, as_bytes: bool) {
        self.update_at(Instant::now(), done, total, as_bytes);
    }

    fn update_at(&mut self, now: Instant, done: u64, total: Option<u64>, as_bytes: bool) {
        let soon = |since: Instant, wait| now.saturating_duration_since(since) < wait;
        if !self.on || soon(self.began, self.quiet) || self.last.is_some_and(|t| soon(t, EVERY)) {
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
/// command works. Dropping it erases the line. It isn't made to be nested: dropping an inner one
/// stops the outer one too.
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
        let quiet = quiet();
        // With no quiet time, the first turn shows at once.
        if quiet.is_zero() {
            let line = turning_line(label, 0, m.drawn.screen);
            m.drawn.draw(&mut io::stderr(), &line);
        }
        drop(m);
        // A thread that couldn't start only loses the mark.
        let _ = thread::Builder::new().spawn(move || {
            thread::sleep(quiet);
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
    fn what_does_not_fit_goes_whole_first_the_counts_then_the_cells() {
        let label = "Importing commits";
        for (done, total) in [(334, 1284), (1_000_000, 1_000_000)] {
            let counts = format!("  {} / {}", thousands(done), thousands(total));
            let percent = format!(" {:>3}%", done * 100 / total);
            for width in [40, 50, 53, 54, 60, 70, 77, 120] {
                let line = bar_line(label, done, total, false, screen(true, width));
                let at = format!("{width}, {done}: {line:?}");
                assert!(columns(&line) < width, "{at}");
                assert!(line.starts_with(label), "{at}");
                let cells = line.chars().filter(|c| matches!(c, '█' | '░')).count();
                assert!(cells == 0 || cells == CELLS, "{at}");
                let shown = line.trim_end_matches(&counts);
                // With the counts gone whole, the line ends at the percentage.
                assert!(shown.ends_with(&percent), "{at}");
            }
            // There is room for everything.
            let wide = bar_line(label, done, total, false, screen(true, 120));
            assert!(wide.ends_with(&counts), "{wide:?}");
        }
        assert_eq!(
            bar_line(label, 334, 1284, false, screen(true, 69)),
            "Importing commits        ██████░░░░░░░░░░░░░░░░░░  26%  334 / 1,284"
        );
        assert_eq!(
            bar_line(label, 334, 1284, false, screen(true, 60)),
            "Importing commits        ██████░░░░░░░░░░░░░░░░░░  26%"
        );
        assert_eq!(
            bar_line(label, 334, 1284, false, screen(true, 53)),
            "Importing commits         26%"
        );
        // Only when the label and the percentage don't fit is the label cut.
        assert_eq!(
            bar_line(label, 334, 1284, false, screen(true, 16)),
            "Importing…  26%"
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
        Bar::to(
            out,
            screen(false, 80),
            true,
            "Importing commits",
            began,
            QUIET,
        )
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
    fn a_new_label_is_drawn_at_once() {
        let t0 = Instant::now();
        let mut out = Vec::new();
        let mut b = bar(&mut out, t0);
        b.update_at(t0 + QUIET, 1, Some(4), false);
        b.set_label("Checking Git LFS files");
        b.update_at(t0 + QUIET + Duration::from_millis(1), 1, Some(4), false);
        std::mem::forget(b);
        let said = String::from_utf8(out).unwrap();
        assert!(said.contains("\rChecking Git LFS files"), "{said:?}");
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
        drawn.draw(&mut out, "a longer");
        drawn.draw(&mut out, "short");
        assert_eq!(String::from_utf8(out).unwrap(), "\ra longer\rshort   ");
        // The label is cut to leave room for the mark.
        assert_eq!(turning_line("Tidying", 0, screen(true, 10)), "Tidy… … ⠋");
        // With colour, the line is cleared whole.
        let mut out = Vec::new();
        drawn.screen.color = true;
        drawn.erase(&mut out);
        assert_eq!(out, b"\r\x1b[2K");
    }
}
