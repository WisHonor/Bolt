//! What the terminal can show: colour, symbols and width, found out once; and the lines laid out
//! by them.

use std::io::{self, IsTerminal};
use std::sync::OnceLock;

use crate::output::tell;

/// What one stream can show. `width` is `None` when it isn't a terminal: nothing on it is wrapped
/// or cut.
#[derive(Clone, Copy, Debug)]
pub struct Screen {
    pub color: bool,
    pub symbols: bool,
    pub width: Option<usize>,
}

const PLAIN: Screen = Screen {
    color: false,
    symbols: false,
    width: None,
};

struct Found {
    out: Screen,
    err: Screen,
    json: bool,
}

static FOUND: OnceLock<Found> = OnceLock::new();

/// stdout's screen; plain until [`find_out`] has run.
pub fn out() -> Screen {
    FOUND.get().map_or(PLAIN, |f| f.out)
}

/// stderr's screen; plain until [`find_out`] has run.
pub fn err() -> Screen {
    FOUND.get().map_or(PLAIN, |f| f.err)
}

/// Finds out what stdout and stderr can show. On Windows this also asks the console to read
/// colour codes.
pub fn find_out(json: bool) {
    let _ = FOUND.set(found(json));
}

fn found(json: bool) -> Found {
    let set = |name: &str| std::env::var_os(name).is_some_and(|v| !v.is_empty());
    let fake = fake_terminal();
    let width = |terminal: bool,
                 measured: Option<(terminal_size::Width, terminal_size::Height)>| {
        fake.or_else(|| terminal.then(|| measured.map_or(80, |(w, _)| usize::from(w.0))))
    };
    let out_width = width(
        io::stdout().is_terminal(),
        terminal_size::terminal_size_of(io::stdout()),
    );
    let err_width = width(
        io::stderr().is_terminal(),
        terminal_size::terminal_size_of(io::stderr()),
    );
    let any = out_width.is_some() || err_width.is_some();
    let color = any
        && !json
        && !set("NO_COLOR")
        && std::env::var_os("TERM").is_none_or(|t| t != "dumb")
        && (fake.is_some() || console_reads_colour());
    let shows = if cfg!(windows) {
        set("WT_SESSION") || set("TERM_PROGRAM")
    } else {
        utf8_locale(|name| std::env::var(name).ok())
    };
    let ascii = std::env::var("BOLT_ASCII").ok();
    let symbols = !json && symbols_wanted(ascii.as_deref(), shows);
    let screen = |width: Option<usize>| Screen {
        color: color && width.is_some(),
        symbols: symbols && width.is_some(),
        width,
    };
    Found {
        out: screen(out_width),
        err: screen(err_width),
        json,
    }
}

/// Whether the console reads colour codes. Windows Terminal always does; the older console only
/// once asked, and the request covers stdout and stderr together.
fn console_reads_colour() -> bool {
    !cfg!(windows)
        || std::env::var_os("WT_SESSION").is_some()
        || anstyle_query::windows::enable_ansi_colors() == Some(true)
}

/// The width tests give both streams to make them count as terminals.
#[cfg(feature = "fault-injection")]
fn fake_terminal() -> Option<usize> {
    std::env::var("BOLT_FAKE_TERMINAL").ok()?.parse().ok()
}

#[cfg(not(feature = "fault-injection"))]
fn fake_terminal() -> Option<usize> {
    None
}

/// Whether the first of `LC_ALL`, `LC_CTYPE` and `LANG` that is set names UTF-8.
fn utf8_locale(var: impl Fn(&str) -> Option<String>) -> bool {
    ["LC_ALL", "LC_CTYPE", "LANG"]
        .iter()
        .find_map(|name| var(name).filter(|v| !v.is_empty()))
        .is_some_and(|locale| {
            let locale = locale.to_ascii_lowercase();
            locale.contains("utf-8") || locale.contains("utf8")
        })
}

/// `BOLT_ASCII=1` and `BOLT_ASCII=0` win over what the terminal seems to show.
fn symbols_wanted(ascii: Option<&str>, shows: bool) -> bool {
    match ascii {
        Some("1") => false,
        Some("0") => true,
        _ => shows,
    }
}

/// What a piece of text is, for its colour.
#[derive(Clone, Copy)]
pub enum Role {
    Ok,
    Error,
    Warning,
    Hint,
    Command,
    ChangeId,
    Fingerprint,
    Line,
    Path,
    Added,
    Removed,
    Changed,
    Hunk,
    Dim,
    Bold,
}

impl Role {
    fn code(self) -> &'static str {
        match self {
            Role::Ok => "1;32",
            Role::Error => "1;31",
            Role::Warning => "1;33",
            Role::Hint => "1;36",
            Role::Command | Role::Hunk => "36",
            Role::ChangeId => "1;35",
            Role::Fingerprint | Role::Line | Role::Path => "34",
            Role::Added => "32",
            Role::Removed => "31",
            Role::Changed => "33",
            Role::Dim => "2",
            Role::Bold => "1",
        }
    }
}

#[derive(Clone, Copy)]
pub enum Sym {
    Bullet,
    Marker,
    Arrow,
    Back,
    Check,
    Cross,
    Rule,
    Cut,
    Full,
    Empty,
}

impl Sym {
    /// The symbol, and what stands in for it.
    fn forms(self) -> (&'static str, &'static str) {
        match self {
            Sym::Bullet => ("•", "*"),
            Sym::Marker => ("▸", ">"),
            Sym::Arrow => ("→", "->"),
            Sym::Back => ("←", "<-"),
            Sym::Check => ("✓", "+"),
            Sym::Cross => ("✗", "x"),
            Sym::Rule => ("─", "-"),
            Sym::Cut => ("…", "..."),
            Sym::Full => ("█", "#"),
            Sym::Empty => ("░", "."),
        }
    }
}

/// Columns `text` takes, counted one per character: Bolt's own words and symbols are one column
/// each, and a wide character in a project's name only makes its row run a little long.
pub(crate) fn columns(text: &str) -> usize {
    text.chars().count()
}

impl Screen {
    /// `escaped` in the role's colour, or unchanged without colour.
    pub fn paint(self, role: Role, escaped: &str) -> String {
        if self.color {
            format!("\x1b[{}m{escaped}\x1b[0m", role.code())
        } else {
            escaped.to_owned()
        }
    }

    pub fn sym(self, sym: Sym) -> &'static str {
        let (symbol, plain) = sym.forms();
        if self.symbols { symbol } else { plain }
    }

    /// Columns a line drawn in place may take: one short of the width, so it never wraps; any
    /// number off a terminal.
    pub fn line_room(self) -> usize {
        self.width.map_or(usize::MAX, |w| w.saturating_sub(1))
    }

    /// The turning mark at its `step`th turn.
    pub fn turn(self, step: usize) -> char {
        const SYMBOLS: [char; 10] = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
        const PLAIN: [char; 4] = ['|', '/', '-', '\\'];
        let marks: &[char] = if self.symbols { &SYMBOLS } else { &PLAIN };
        marks[step % marks.len()]
    }

    /// `prefix` then `message`, wrapped under the message's text when this is a terminal.
    pub fn prefixed(self, prefix_role: Role, prefix: &str, message: &str) -> String {
        let head = self.paint(prefix_role, prefix);
        if self.width.is_none() {
            return format!("{head} {message}");
        }
        let indent = columns(prefix) + 1;
        let body = self.indented(indent, message);
        format!("{head} {}", &body[indent..])
    }

    /// `text` with each of its lines `indent` columns in. On a terminal each line is wrapped at
    /// the smaller of the width and 100, the lines it breaks into starting as far in.
    pub fn indented(self, indent: usize, text: &str) -> String {
        let mut lines = Vec::new();
        for line in text.split('\n') {
            match self.width {
                Some(width) => wrap(line, width.min(100).saturating_sub(indent), &mut lines),
                None => lines.push(line.to_owned()),
            }
        }
        let pad = " ".repeat(indent);
        let lines: Vec<String> = lines.into_iter().map(|l| format!("{pad}{l}")).collect();
        lines.join("\n")
    }

    /// `path` cut from its middle to fit `room` columns, or whole when it can't be or this is no
    /// terminal.
    pub fn fit_path(self, path: &str, room: usize) -> String {
        if self.width.is_none() || columns(path) <= room {
            return path.to_owned();
        }
        let parts: Vec<&str> = path.split('/').collect();
        let [first, middle @ .., name] = parts.as_slice() else {
            return path.to_owned();
        };
        // The folders nearest the name are kept longest.
        (0..middle.len())
            .rev()
            .map(|kept| {
                let mut shown = vec![*first, self.sym(Sym::Cut)];
                shown.extend(&middle[middle.len() - kept..]);
                shown.push(name);
                shown.join("/")
            })
            .find(|shown| columns(shown) <= room)
            .unwrap_or_else(|| path.to_owned())
    }

    /// `text` cut at its end, with the cut mark, to fit `room` columns on a terminal. A cut that
    /// would fall inside an escape `text::escape` wrote falls before its backslash instead.
    pub fn cut_end(self, text: &str, room: usize) -> String {
        if self.width.is_none() || columns(text) <= room {
            return text.to_owned();
        }
        let cut = self.sym(Sym::Cut);
        let chars: Vec<char> = text.chars().collect();
        let mut keep = room.saturating_sub(columns(cut));
        let mut at = 0;
        while at < keep {
            let step = escape_len(&chars[at..]);
            if at + step > keep {
                keep = at;
            }
            at += step.max(1);
        }
        chars[..keep].iter().collect::<String>() + cut
    }
}

/// How many characters the escape at the start of `rest` takes: `\xNN` or `\u{N…}`, else 0.
fn escape_len(rest: &[char]) -> usize {
    match rest {
        ['\\', 'x', a, b, ..] if a.is_ascii_hexdigit() && b.is_ascii_hexdigit() => 4,
        ['\\', 'u', '{', tail @ ..] => {
            let digits = tail.iter().take_while(|c| c.is_ascii_hexdigit()).count();
            if digits > 0 && tail.get(digits) == Some(&'}') {
                digits + 4
            } else {
                0
            }
        }
        _ => 0,
    }
}

/// `line` broken at spaces into lines of at most `room` columns. A word longer than that is
/// left long.
fn wrap(line: &str, room: usize, lines: &mut Vec<String>) {
    let mut current: Option<String> = None;
    for word in line.split(' ') {
        match &mut current {
            Some(c) if columns(c) + 1 + columns(word) > room && !c.trim().is_empty() => {
                lines.push(std::mem::replace(c, word.to_owned()));
            }
            Some(c) => {
                c.push(' ');
                c.push_str(word);
            }
            None => current = Some(word.to_owned()),
        }
    }
    lines.extend(current);
}

/// Whether lines meant only for people go on stderr.
pub(crate) fn for_people() -> bool {
    FOUND
        .get()
        .is_some_and(|f| !f.json && f.err.width.is_some())
}

/// Whether Bolt may ask a question: stdin and stderr are terminals and `--json` wasn't given.
pub fn may_ask() -> bool {
    for_people() && (io::stdin().is_terminal() || fake_terminal().is_some())
}

/// "Next: …" on stderr, after an empty line, only when stderr is a terminal.
pub fn next(line: &str) {
    if for_people() {
        tell("");
        tell(&err().prefixed(Role::Hint, "Next:", line));
    }
}

/// "Changed your mind? bolt undo" on stderr, after an empty line, only when stderr is a terminal.
pub fn changed_your_mind() {
    if for_people() {
        let e = err();
        tell("");
        tell(&format!(
            "Changed your mind? {}",
            e.paint(Role::Command, "bolt undo")
        ));
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;

    fn screen(color: bool, symbols: bool, width: Option<usize>) -> Screen {
        Screen {
            color,
            symbols,
            width,
        }
    }

    const ROLES: [(Role, &str); 15] = [
        (Role::Ok, "1;32"),
        (Role::Error, "1;31"),
        (Role::Warning, "1;33"),
        (Role::Hint, "1;36"),
        (Role::Command, "36"),
        (Role::ChangeId, "1;35"),
        (Role::Fingerprint, "34"),
        (Role::Line, "34"),
        (Role::Path, "34"),
        (Role::Added, "32"),
        (Role::Removed, "31"),
        (Role::Changed, "33"),
        (Role::Hunk, "36"),
        (Role::Dim, "2"),
        (Role::Bold, "1"),
    ];

    #[test]
    fn each_role_has_its_code_and_ends_with_a_reset() {
        let s = screen(true, false, Some(80));
        for (role, code) in ROLES {
            assert_eq!(s.paint(role, "x"), format!("\x1b[{code}mx\x1b[0m"));
        }
    }

    #[test]
    fn without_colour_paint_changes_nothing() {
        let s = screen(false, true, Some(80));
        for (role, _) in ROLES {
            assert_eq!(s.paint(role, "a \\x1b b"), "a \\x1b b");
        }
    }

    #[test]
    fn every_symbol_has_a_plain_form_in_ascii() {
        let all = [
            (Sym::Bullet, "•", "*"),
            (Sym::Marker, "▸", ">"),
            (Sym::Arrow, "→", "->"),
            (Sym::Back, "←", "<-"),
            (Sym::Check, "✓", "+"),
            (Sym::Cross, "✗", "x"),
            (Sym::Rule, "─", "-"),
            (Sym::Cut, "…", "..."),
            (Sym::Full, "█", "#"),
            (Sym::Empty, "░", "."),
        ];
        for (sym, symbol, plain) in all {
            assert_eq!(screen(false, true, Some(80)).sym(sym), symbol);
            assert_eq!(screen(true, false, Some(80)).sym(sym), plain);
            assert!(plain.is_ascii());
        }
    }

    #[test]
    fn a_long_error_wraps_under_its_text_at_the_width_or_100() {
        let message = "word ".repeat(40);
        let message = message.trim_end();
        for (width, limit) in [(60, 60), (140, 100)] {
            let shown = screen(false, false, Some(width)).prefixed(Role::Error, "error:", message);
            let lines: Vec<&str> = shown.lines().collect();
            assert!(lines.len() > 1, "{shown}");
            assert!(lines[0].starts_with("error: word"), "{shown}");
            for line in &lines {
                assert!(line.len() <= limit, "{width}: {line:?}");
            }
            for line in &lines[1..] {
                assert!(line.starts_with("       word"), "{width}: {line:?}");
            }
            // The last line before a break was full: the next word would have passed the limit.
            assert!(lines[0].len() + " word".len() > limit, "{shown}");
            assert_eq!(
                shown
                    .split_whitespace()
                    .skip(1)
                    .collect::<Vec<_>>()
                    .join(" "),
                message
            );
        }
        let hint = screen(false, false, Some(30)).prefixed(Role::Hint, "hint:", message);
        assert!(
            hint.lines().nth(1).unwrap().starts_with("      word"),
            "{hint}"
        );
        let warning = screen(false, false, Some(30)).prefixed(Role::Warning, "warning:", message);
        assert!(
            warning.lines().nth(1).unwrap().starts_with("         word"),
            "{warning}"
        );
    }

    #[test]
    fn a_message_with_its_own_lines_keeps_them() {
        let shown = screen(false, false, Some(80)).prefixed(
            Role::Error,
            "error:",
            "These files changed:\n  a.txt\n  b.txt",
        );
        assert_eq!(
            shown,
            "error: These files changed:\n         a.txt\n         b.txt"
        );
    }

    #[test]
    fn a_word_longer_than_the_width_is_left_long() {
        let long = "x".repeat(50);
        let shown =
            screen(false, false, Some(30)).prefixed(Role::Error, "error:", &format!("a {long} b"));
        assert_eq!(shown, format!("error: a\n       {long}\n       b"));
    }

    #[test]
    fn nothing_wraps_without_a_terminal() {
        let message = format!("{} \nsecond line", "word ".repeat(40));
        let s = screen(false, false, None);
        assert_eq!(
            s.prefixed(Role::Error, "error:", &message),
            format!("error: {message}")
        );
        assert_eq!(
            s.fit_path("Assets/a/b/c/d/Rock.psd", 5),
            "Assets/a/b/c/d/Rock.psd"
        );
        assert_eq!(s.cut_end("a long message", 5), "a long message");
    }

    #[test]
    fn a_path_loses_folders_from_its_middle_and_keeps_its_first_folder_and_name() {
        let s = screen(false, true, Some(80));
        let path = "Assets/Textures/Rocks/Granite/Rock_Albedo.psd";
        assert_eq!(s.fit_path(path, 80), path);
        assert_eq!(s.fit_path(path, 24), "Assets/…/Rock_Albedo.psd");
        assert_eq!(s.fit_path(path, 32), "Assets/…/Granite/Rock_Albedo.psd");
        let plain = screen(false, false, Some(80));
        assert_eq!(plain.fit_path(path, 26), "Assets/.../Rock_Albedo.psd");
        assert_eq!(s.cut_end("a long message", 8), "a long …");
    }

    #[test]
    fn a_cut_never_falls_inside_an_escape() {
        let s = screen(false, false, Some(80));
        for raw in ["\u{1b}", "\u{202e}"] {
            let escape = bolt_core::text::escape(raw, false);
            let text = format!("ab{escape}cdefghijklmnop");
            let len = escape.chars().count();
            // Kept before the cut mark, `...`: every place from the start to past the escape.
            for keep in 0..=len + 4 {
                let shown = s.cut_end(&text, keep + 3);
                let kept: String = if (3..2 + len).contains(&keep) {
                    "ab".to_owned()
                } else {
                    text.chars().take(keep).collect()
                };
                assert_eq!(shown, format!("{kept}..."), "{escape} at {keep}");
            }
        }
        // A title of nothing but escapes keeps only the escapes that fit whole.
        let only = bolt_core::text::escape("\u{1b}\u{1b}\u{1b}", false);
        assert_eq!(s.cut_end(&only, 6), "...");
        assert_eq!(s.cut_end(&only, 9), "\\x1b...");
        // A backslash typed by the user is printed as it is, and cut like any character.
        let typed = bolt_core::text::escape("a\\b\\cdefgh", false);
        assert_eq!(typed, "a\\b\\cdefgh");
        assert_eq!(s.cut_end(&typed, 5), "a\\...");
        assert_eq!(s.cut_end(&typed, 7), "a\\b\\...");
    }

    #[test]
    fn a_path_that_cannot_fit_is_left_whole() {
        let s = screen(false, true, Some(80));
        let path = "Assets/Textures/Rocks/Granite/Rock_Albedo.psd";
        assert_eq!(s.fit_path(path, 10), path);
        assert_eq!(
            s.fit_path("Assets/Rock_Albedo.psd", 10),
            "Assets/Rock_Albedo.psd"
        );
    }

    #[test]
    fn widths_count_escaped_text() {
        // The escaped form is what is printed, so it is what is counted.
        let escaped = bolt_core::text::escape("a\u{202e}b", false);
        assert_eq!(columns(&escaped), "a\\u{202e}b".len());
        let s = screen(false, true, Some(80));
        let path = format!("{escaped}/xyz/name");
        assert_eq!(s.fit_path(&path, columns(&path)), path);
        assert_eq!(
            s.fit_path(&path, columns(&path) - 1),
            format!("{escaped}/…/name")
        );
    }

    #[test]
    fn sizes_are_1024_based_with_one_decimal_below_ten() {
        let sizes =
            [0, 1023, 1024, 10_380_902, 10 << 20, 1_288_490_189].map(bolt_core::text::bytes);
        assert_eq!(
            sizes,
            ["0 B", "1,023 B", "1.0 KiB", "9.9 MiB", "10 MiB", "1.2 GiB"]
        );
    }

    #[test]
    fn utf8_locales_are_recognised() {
        let with = |vars: &[(&str, &str)]| {
            let vars: Vec<(String, String)> = vars
                .iter()
                .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                .collect();
            utf8_locale(|name| vars.iter().find(|(k, _)| k == name).map(|(_, v)| v.clone()))
        };
        assert!(with(&[("LANG", "en_US.UTF-8")]));
        assert!(with(&[("LANG", "C.utf8")]));
        assert!(!with(&[("LANG", "POSIX")]));
        assert!(!with(&[]));
        // LC_ALL before LC_CTYPE before LANG; an empty one is passed over.
        assert!(!with(&[("LC_ALL", "C"), ("LANG", "en_US.UTF-8")]));
        assert!(with(&[
            ("LC_ALL", ""),
            ("LC_CTYPE", "de_DE.UTF-8"),
            ("LANG", "C")
        ]));
        assert!(!with(&[("LC_CTYPE", "POSIX"), ("LANG", "en_US.UTF-8")]));
    }

    #[test]
    fn bolt_ascii_wins_either_way() {
        assert!(!symbols_wanted(Some("1"), true));
        assert!(symbols_wanted(Some("0"), false));
        assert!(symbols_wanted(Some("yes"), true));
        assert!(!symbols_wanted(Some("yes"), false));
        assert!(!symbols_wanted(None, false));
    }
}
