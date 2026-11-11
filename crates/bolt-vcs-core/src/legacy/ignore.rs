//! Git's ignore rules, ported from its `dir.c` and `wildmatch.c` quirks and all, so that Bolt
//! keeps out exactly the files Git keeps out.
#![deny(clippy::indexing_slicing, clippy::arithmetic_side_effects)]

use std::mem;
use std::sync::Arc;

use unicode_normalization::{UnicodeNormalization, is_nfc};

use crate::drive::Drive;

/// Git's `wildmatch` with `WM_PATHNAME`, and `WM_CASEFOLD` when `casefold`: whether `pattern`
/// matches the whole of `text`. Git's recurses at each `*`; this reads the text once and keeps
/// the set of pattern beginnings that match it so far, so its time is at most the pattern's
/// length times the text's.
pub fn wildmatch(pattern: &[u8], text: &[u8], casefold: bool) -> bool {
    let mut pieces = Pieces {
        rest: until_nul(pattern),
        before: None,
        casefold,
        read: Vec::new(),
    };
    let mut row = Vec::new();
    let mut matched = advance(&mut pieces, &[], &mut row, None);
    let mut next = Vec::new();
    for &byte in until_nul(text) {
        if row.is_empty() {
            return false;
        }
        matched = advance(&mut pieces, &row, &mut next, Some(byte));
        mem::swap(&mut row, &mut next);
    }
    matched
}

/// The bytes before the first NUL, where Git's C strings end.
fn until_nul(bytes: &[u8]) -> &[u8] {
    bytes.split(|&b| b == 0).next().unwrap_or(bytes)
}

/// A byte as `wildmatch` compares it: ASCII capitals lowered where letter case is ignored.
fn fold(b: u8, casefold: bool) -> u8 {
    if casefold { b.to_ascii_lowercase() } else { b }
}

/// One piece of a pattern.
#[derive(Clone, Copy)]
enum Token<'p> {
    /// One byte, lowered where letter case is ignored unless a `\` escaped it: Git leaves an
    /// escaped capital alone, so then it matches nothing.
    Byte(u8),
    /// `?`: any byte but `/`.
    Any,
    /// `[…]`: the bytes after its `[`, through its `]`.
    Class(&'p [u8]),
    /// `*`: any bytes but `/`.
    Star,
    /// `**` as a whole folder name: any bytes.
    Stars,
    /// The `/` after such a `**`, which may be skipped with it: `a/**/b` matches `a/b`.
    StarsSlash,
    /// A `\` at the end, or a `[` that never closes or names an unknown class: nothing matches
    /// past it, as in Git.
    Never,
}

/// A pattern's pieces, read as Git's `dowild` meets them, and only as far as they are needed.
struct Pieces<'p> {
    rest: &'p [u8],
    /// The byte before `rest`: a `**` is a whole folder name only after a `/` or at the start.
    before: Option<u8>,
    casefold: bool,
    read: Vec<Token<'p>>,
}

impl<'p> Pieces<'p> {
    /// Piece `j`, or `None` past the pattern's end. Pieces are asked for in order.
    fn get(&mut self, j: usize) -> Option<Token<'p>> {
        if let Some(&token) = self.read.get(j) {
            return Some(token);
        }
        let token = self.next_piece()?;
        self.read.push(token);
        Some(token)
    }

    fn next_piece(&mut self) -> Option<Token<'p>> {
        let (&c, rest) = self.rest.split_first()?;
        let (token, rest, last) = match c {
            b'\\' => match rest.split_first() {
                Some((&escaped, rest)) => (Token::Byte(escaped), rest, escaped),
                None => (Token::Never, rest, c),
            },
            b'?' => (Token::Any, rest, c),
            b'[' => match class(rest, 0, false).and_then(|(_, len)| rest.split_at_checked(len)) {
                Some((body, rest)) => (Token::Class(body), rest, b']'),
                None => (Token::Never, rest, c),
            },
            b'*' => {
                let more = rest.iter().take_while(|&&b| b == b'*').count();
                let rest = rest.get(more..).unwrap_or_default();
                let whole_name = more > 0
                    && matches!(self.before, None | Some(b'/'))
                    && (rest.is_empty() || rest.starts_with(b"/") || rest.starts_with(b"\\/"));
                let token = if whole_name {
                    Token::Stars
                } else {
                    Token::Star
                };
                (token, rest, c)
            }
            b'/' if matches!(self.read.last(), Some(Token::Stars)) => (Token::StarsSlash, rest, c),
            _ => (Token::Byte(fold(c, self.casefold)), rest, c),
        };
        self.rest = if matches!(token, Token::Never) {
            &[]
        } else {
            rest
        };
        self.before = Some(last);
        Some(token)
    }
}

// The cells `advance` fills on this thread, so a test can bound `wildmatch`'s work without a clock.
#[cfg(test)]
thread_local! {
    static CELLS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// One row of `wildmatch`'s table: `row[j]` says whether the pattern's first `j` pieces match the
/// text read so far. Fills `new` from `old`, the row before `byte` (for `None`, the row of the
/// empty text), cut after its last true cell, and returns whether the whole pattern matches.
fn advance(pieces: &mut Pieces<'_>, old: &[bool], new: &mut Vec<bool>, byte: Option<u8>) -> bool {
    let casefold = pieces.casefold;
    let t = byte.map(|b| fold(b, casefold));
    new.clear();
    let (mut left2, mut left) = (false, byte.is_none());
    new.push(left);
    // `old[j]` and `old[j + 1]`, for piece `j`.
    let mut ups = old.iter().skip(1);
    let mut diag = old.first() == Some(&true);
    let mut j = 0usize;
    let matched = loop {
        let up = ups.next() == Some(&true);
        // Past `old`'s last true cell, only a `*` can carry a true cell further.
        if !(diag || up || left || left2) && j >= old.len() {
            break false;
        }
        let Some(token) = pieces.get(j) else {
            break left;
        };
        let cell = match (token, t) {
            (Token::Star | Token::Stars, None) => left,
            (Token::StarsSlash, None) => left2,
            (_, None) | (Token::Never, Some(_)) => false,
            (Token::Byte(_) | Token::Any | Token::Class(_), Some(t)) => {
                diag && takes(token, t, casefold)
            }
            (Token::Star, Some(t)) => left || (up && t != b'/'),
            (Token::Stars, Some(_)) => left || up,
            (Token::StarsSlash, Some(t)) => left2 || (diag && t == b'/'),
        };
        new.push(cell);
        #[cfg(test)]
        CELLS.with(|cells| cells.set(cells.get().saturating_add(1)));
        (left2, left, diag) = (left, cell, up);
        j = j.saturating_add(1);
    };
    while new.last() == Some(&false) {
        new.pop();
    }
    matched
}

/// Whether a piece that takes one byte takes `t`, a text byte already folded where letter case is
/// ignored.
fn takes(token: Token<'_>, t: u8, casefold: bool) -> bool {
    match token {
        Token::Byte(p) => t == p,
        Token::Any => t != b'/',
        Token::Class(body) => class(body, t, casefold).is_some_and(|c| c.0),
        Token::Star | Token::Stars | Token::StarsSlash | Token::Never => false,
    }
}

/// Git's `[…]`: whether it takes `t`, a text byte already folded where letter case is ignored,
/// and its length after the `[`, through the `]`. `None` if it never closes or names a
/// `[:class:]` Git doesn't know.
fn class(after: &[u8], t: u8, casefold: bool) -> Option<(bool, usize)> {
    let negated = matches!(after.first(), Some(b'!' | b'^'));
    let mut i = usize::from(negated);
    // The member before, which a `-` makes a range's start; none after a range or a class.
    let mut prev = 0u8;
    let mut hit = false;
    // Where the last search for a `[:`'s `]` ended, so a run of `[:` reads each byte once.
    let mut close = 0usize;
    loop {
        let mut c = *after.get(i)?;
        let next = i.checked_add(1)?;
        if c == b'\\' {
            i = next;
            c = *after.get(i)?;
            hit |= t == c;
        } else if c == b'-' && prev != 0 && after.get(next).is_some_and(|&b| b != b']') {
            i = next;
            let mut last = *after.get(i)?;
            if last == b'\\' {
                i = i.checked_add(1)?;
                last = *after.get(i)?;
            }
            let range = prev..=last;
            hit |= range.contains(&t)
                || (casefold && t.is_ascii_lowercase() && range.contains(&t.to_ascii_uppercase()));
            c = 0;
        } else if c == b'[' && after.get(next) == Some(&b':') {
            let from = next.checked_add(1)?;
            if close < from {
                let ahead = after.get(from..)?.iter().position(|&b| b == b']')?;
                close = from.checked_add(ahead)?;
            }
            match after
                .get(from..close)
                .and_then(|name| name.strip_suffix(b":"))
            {
                Some(name) => {
                    hit |= named_class(name, t, casefold)?;
                    i = close;
                    c = 0;
                }
                // No class name after all: the `[` is a member, and so is the `:`.
                None => hit |= t == b'[',
            }
        } else {
            hit |= t == c;
        }
        prev = c;
        i = i.checked_add(1)?;
        if after.get(i) == Some(&b']') {
            return Some((hit != negated && t != b'/', i.checked_add(1)?));
        }
    }
}

/// Git's `[:name:]` classes, ASCII only as in its `sane-ctype.h`; `None` for a name it doesn't
/// know.
fn named_class(name: &[u8], t: u8, casefold: bool) -> Option<bool> {
    Some(match name {
        b"alnum" => t.is_ascii_alphanumeric(),
        b"alpha" => t.is_ascii_alphabetic(),
        b"blank" => matches!(t, b' ' | b'\t'),
        b"cntrl" => t.is_ascii_control(),
        b"digit" => t.is_ascii_digit(),
        b"graph" => t.is_ascii_graphic(),
        b"lower" => t.is_ascii_lowercase(),
        b"print" => matches!(t, b' '..=b'~'),
        b"punct" => t.is_ascii_punctuation(),
        // Git's own table leaves out \v and \f.
        b"space" => matches!(t, b'\t' | b'\n' | b'\r' | b' '),
        b"upper" => t.is_ascii_uppercase() || (casefold && t.is_ascii_lowercase()),
        b"xdigit" => t.is_ascii_hexdigit(),
        _ => return None,
    })
}

/// One line of an ignore file, as Git's `parse_path_pattern` leaves it.
struct Pattern {
    /// Without its `!` and its final `/`.
    text: Box<[u8]>,
    negative: bool,
    /// It ended in `/`: only folders match.
    dir_only: bool,
    /// It has no `/` left: it matches a name at any depth.
    any_depth: bool,
    /// `*` and then no wildcard: Git compares the ends of names.
    ends_with: bool,
    /// The bytes before its first wildcard.
    literal: usize,
    /// Set for a name pattern of one-byte pieces around a single `*`, which needs no table.
    ends: Option<Ends>,
}

/// A line of one-byte pieces around a single `*`: where the pieces before the `*` end and
/// those after it start in `text`, and how many bytes of a name each side takes.
struct Ends {
    head: usize,       // text[..head] holds the pieces before the `*`
    tail: usize,       // text[tail..] holds the pieces after it
    head_width: usize, // pieces before the `*`, one byte of a name each
    tail_width: usize, // pieces after it
}

impl Ends {
    /// The shape of `text`, read as `wildmatch` reads it; `None` unless it is one-byte pieces,
    /// one `*`, then one-byte pieces.
    fn of(text: &[u8]) -> Option<Ends> {
        // `wildmatch` would stop at a NUL; such a pattern is left to it.
        if text.contains(&0) {
            return None;
        }
        let mut pieces = Pieces {
            rest: text,
            before: None,
            casefold: false,
            read: Vec::new(),
        };
        let (mut star, mut width) = (None, 0usize);
        loop {
            let at = text.len().checked_sub(pieces.rest.len())?;
            match pieces.next_piece() {
                None => break,
                Some(Token::Byte(_) | Token::Any | Token::Class(_)) => {
                    width = width.checked_add(1)?
                }
                Some(Token::Star) if star.is_none() => {
                    let tail = text.len().checked_sub(pieces.rest.len())?;
                    star = Some((at, tail, width));
                    width = 0;
                }
                Some(_) => return None,
            }
        }
        let (head, tail, head_width) = star?;
        Some(Ends {
            head,
            tail,
            head_width,
            tail_width: width,
        })
    }

    /// Whether `name` matches `text`, the pattern this was read from, as `wildmatch` would say.
    fn take(&self, text: &[u8], name: &[u8], casefold: bool) -> bool {
        let name = until_nul(name);
        let Some((front, back)) = name
            .len()
            .checked_sub(self.tail_width)
            .and_then(|at| name.split_at_checked(at))
        else {
            return false;
        };
        let Some((head, middle)) = front.split_at_checked(self.head_width) else {
            return false;
        };
        let (Some(head_text), Some(tail_text)) = (text.get(..self.head), text.get(self.tail..))
        else {
            return false;
        };
        pieces_take(head_text, head, casefold)
            && pieces_take(tail_text, back, casefold)
            && !middle.contains(&b'/')
    }
}

/// Whether the one-byte pieces of `pattern` take the bytes of `text`, one each.
fn pieces_take(pattern: &[u8], text: &[u8], casefold: bool) -> bool {
    let mut pieces = Pieces {
        rest: pattern,
        before: None,
        casefold,
        read: Vec::new(),
    };
    let taken = text.iter().all(|&b| {
        pieces
            .next_piece()
            .is_some_and(|token| takes(token, fold(b, casefold), casefold))
    });
    // `Ends` cut the text to as many bytes as there are pieces.
    debug_assert!(!taken || pieces.next_piece().is_none());
    taken
}

impl Pattern {
    fn parse(line: &[u8]) -> Pattern {
        let (negative, line) = match line.strip_prefix(b"!") {
            Some(rest) => (true, rest),
            None => (false, line),
        };
        let (dir_only, text) = match line.strip_suffix(b"/") {
            Some(rest) => (true, rest),
            None => (false, line),
        };
        let any_depth = !text.contains(&b'/');
        let ends_with = line
            .split_first()
            .is_some_and(|(&c, rest)| c == b'*' && literal_len(rest) == rest.len());
        let literal = literal_len(text);
        let ends = if any_depth && !ends_with && literal != text.len() {
            Ends::of(text)
        } else {
            None
        };
        Pattern {
            negative,
            dir_only,
            any_depth,
            ends_with,
            literal,
            ends,
            text: text.into(),
        }
    }
}

/// Git's `simple_length`: the bytes before the first `*`, `?`, `[` or `\`.
fn literal_len(s: &[u8]) -> usize {
    s.iter()
        .position(|b| matches!(b, b'*' | b'?' | b'[' | b'\\'))
        .unwrap_or(s.len())
}

/// Git's `trim_trailing_spaces`: the spaces at the end go unless a `\` escapes them, and after a
/// `\` that ends the line nothing goes.
fn trim_trailing_spaces(line: &[u8]) -> &[u8] {
    let (mut cut, mut escaped) = (None, false);
    for (i, &b) in line.iter().enumerate() {
        if escaped {
            escaped = false;
            cut = None;
        } else if b == b'\\' {
            escaped = true;
        } else if b == b' ' {
            cut.get_or_insert(i);
        } else {
            cut = None;
        }
    }
    match cut {
        Some(at) if !escaped => line.get(..at).unwrap_or(line),
        _ => line,
    }
}

/// Git's `fspathncmp`: equal bytes, with ASCII letter case folded where the drive ignores it.
fn same(a: &[u8], b: &[u8], casefold: bool) -> bool {
    if casefold {
        a.eq_ignore_ascii_case(b)
    } else {
        a == b
    }
}

/// Git's `match_basename`, for a pattern without `/`.
fn match_basename(name: &[u8], p: &Pattern, casefold: bool) -> bool {
    if p.literal == p.text.len() {
        same(&p.text, name, casefold)
    } else if p.ends_with {
        let tail = p.text.get(1..).unwrap_or_default();
        name.len()
            .checked_sub(tail.len())
            .and_then(|at| name.get(at..))
            .is_some_and(|end| same(tail, end, casefold))
    } else if let Some(ends) = &p.ends {
        ends.take(&p.text, name, casefold)
    } else {
        wildmatch(&p.text, name, casefold)
    }
}

/// Git's `match_pathname`, for a pattern with a `/`: it matches `path` below `folder`.
fn match_pathname(path: &[u8], folder: &[u8], p: &Pattern, casefold: bool) -> bool {
    let (pattern, literal) = match p.text.strip_prefix(b"/") {
        Some(rest) => (rest, p.literal.saturating_sub(1)),
        None => (&p.text[..], p.literal),
    };
    let Some(name) = below(path, folder, casefold) else {
        return false;
    };
    if literal == 0 {
        return wildmatch(pattern, name, casefold);
    }
    let (Some(lit), Some(head)) = (pattern.get(..literal), name.get(..literal)) else {
        return false;
    };
    if !same(lit, head, casefold) {
        return false;
    }
    if pattern.len() == literal && name.len() == literal {
        return true;
    }
    // One byte of the literal part stays, so `wildmatch` sees where a folder name starts.
    let keep = literal.saturating_sub(1);
    match (pattern.get(keep..), name.get(keep..)) {
        (Some(pattern), Some(name)) => wildmatch(pattern, name, casefold),
        _ => false,
    }
}

/// `path` with `folder` and a `/` taken off its front, as `match_pathname` compares it; `None`
/// if `path` isn't below `folder`.
fn below<'a>(path: &'a [u8], folder: &[u8], casefold: bool) -> Option<&'a [u8]> {
    if folder.is_empty() {
        return (!path.is_empty()).then_some(path);
    }
    let (head, rest) = path.split_at_checked(folder.len())?;
    if !same(head, folder, casefold) {
        return None;
    }
    rest.strip_prefix(b"/")
}

/// FNV-1a over the lowered bytes: cheap, and the same for any process, so a lookup needs only the
/// bytes; letter case is left to `same`.
fn folded_hash(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, &b| {
        (hash ^ u64::from(b.to_ascii_lowercase())).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

// The lines `Rules::last_match` tries on this thread, so a test can see the lookup skip the rest.
#[cfg(test)]
thread_local! {
    static FITS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// One ignore file's lines, for the folder it sits in.
pub struct Rules {
    /// The folder's project path, without a final `/`; empty for the top folder.
    folder: String,
    patterns: Vec<Pattern>,
    /// The lines without wildcard or `/`, by `folded_hash` of their text, and their positions.
    names: Vec<(u64, u32)>,
    /// The lines without wildcard but with a `/`, by `folded_hash` of their text past one leading
    /// `/`, and their positions.
    paths: Vec<(u64, u32)>,
    /// The other lines' positions, in order.
    rest: Vec<usize>,
    /// The file's size, which counts against Bolt's limits while its rules are in force.
    bytes: u64,
    /// Whether the file is a `.gitignore`, whose lines Git reads too, or a `.boltignore`.
    gitignore: bool,
}

impl Rules {
    /// Reads `text` as Git reads an ignore file: a UTF-8 byte order mark at the start is skipped,
    /// lines end at LF with one CR before it dropped, a line starting with `#` is a comment, and
    /// a line ends at a NUL. No line is refused; one Git can't use simply matches nothing.
    /// `gitignore` says whether the file is a `.gitignore`.
    pub fn parse(text: &[u8], folder: &str, gitignore: bool) -> Rules {
        let body = text.strip_prefix(b"\xef\xbb\xbf").unwrap_or(text);
        let patterns: Vec<Pattern> = body
            .split(|&b| b == b'\n')
            .filter(|line| line.first().is_some_and(|&b| b != b'#'))
            .map(|line| {
                let line = line.strip_suffix(b"\r").unwrap_or(line);
                Pattern::parse(trim_trailing_spaces(until_nul(line)))
            })
            .collect();
        let (mut names, mut paths, mut rest) = (Vec::new(), Vec::new(), Vec::new());
        for (i, p) in patterns.iter().enumerate() {
            match u32::try_from(i) {
                Ok(at) if p.literal == p.text.len() && p.any_depth => {
                    names.push((folded_hash(&p.text), at));
                }
                Ok(at) if p.literal == p.text.len() => {
                    let text = p.text.strip_prefix(b"/").unwrap_or(&p.text);
                    paths.push((folded_hash(text), at));
                }
                _ => rest.push(i),
            }
        }
        names.sort_unstable();
        paths.sort_unstable();
        Rules {
            folder: folder.to_owned(),
            patterns,
            names,
            paths,
            rest,
            bytes: u64::try_from(text.len()).unwrap_or(u64::MAX),
            gitignore,
        }
    }

    /// The last line that matches, as Git's `last_matching_pattern_from_list` finds it. `name` is
    /// the last part of `path`.
    fn last_match(
        &self,
        path: &[u8],
        name: &[u8],
        is_dir: bool,
        casefold: bool,
    ) -> Option<&Pattern> {
        let folder = self.folder.as_bytes();
        let fits = |i: usize| {
            #[cfg(test)]
            FITS.with(|n| n.set(n.get().saturating_add(1)));
            self.patterns.get(i).is_some_and(|p| {
                (is_dir || !p.dir_only)
                    && if p.any_depth {
                        match_basename(name, p, casefold)
                    } else {
                        match_pathname(path, folder, p, casefold)
                    }
            })
        };
        // A line without wildcard fits only bytes that are its own, letter case aside, so only
        // the lines hashed as `name` (or the path below the folder) can.
        let in_names = last_fit(&self.names, name, &fits);
        // `wildmatch` stops at a NUL, so the path is looked up only that far.
        let in_paths = below(path, folder, casefold)
            .and_then(|below| last_fit(&self.paths, until_nul(below), &fits));
        let best = in_names.max(in_paths);
        let in_rest = self
            .rest
            .iter()
            .rev()
            .take_while(|&&i| best.is_none_or(|b| i > b))
            .copied()
            .find(|&i| fits(i));
        in_rest.or(best).and_then(|i| self.patterns.get(i))
    }
}

/// The last of `index`'s lines hashed as `key` that fits.
fn last_fit(index: &[(u64, u32)], key: &[u8], fits: &impl Fn(usize) -> bool) -> Option<usize> {
    if index.is_empty() {
        return None;
    }
    let hash = folded_hash(key);
    let start = index.partition_point(|&(h, _)| h < hash);
    let end = index.partition_point(|&(h, _)| h <= hash);
    index
        .get(start..end)?
        .iter()
        .rev()
        .filter_map(|&(_, at)| usize::try_from(at).ok())
        .find(|&i| fits(i))
}

/// What the rules in force say about a path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decision {
    /// The last line that matches ignores it.
    Ignored,
    /// The last line that matches starts with `!`, which takes it back.
    Kept,
    /// No line matches.
    NoRule,
}

/// What the rules in force say about a path: Bolt's verdict, from the lines of every ignore file,
/// and Git's, from the lines of the `.gitignore` files alone. They differ only where a
/// `.boltignore` line decides.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Verdict {
    pub bolt: Decision,
    pub git: Decision,
}

/// The rules of the folders a walk is in, the nearest last.
pub struct Matcher {
    drive: Drive,
    levels: Vec<(String, Arc<[Rules]>)>,
}

/// The rules a `Matcher` had in force at one moment, kept to be asked later: the same rules,
/// shared, so they give the verdicts it gave then.
#[derive(Clone)]
pub(crate) struct InForce {
    drive: Drive,
    levels: Vec<Arc<[Rules]>>,
}

impl InForce {
    /// What these rules say about `path`, as `Matcher::verdict` said it then.
    pub(crate) fn verdict(&self, path: impl AsRef<[u8]>, is_dir: bool) -> Verdict {
        let levels = self.levels.iter().map(|rules| &**rules);
        verdict_in(self.drive, levels, path.as_ref(), is_dir)
    }
}

impl Matcher {
    /// No rules yet. `drive` says how Git compares names there: its `core.ignorecase` and
    /// `core.precomposeunicode`.
    pub fn new(drive: Drive) -> Matcher {
        Matcher {
            drive,
            levels: Vec::new(),
        }
    }

    /// Puts the rules of `folder` in force, in the order they are read: a later line wins.
    pub fn enter(&mut self, folder: &str, mut rules: Vec<Rules>) {
        // Paths are compared composed there, so the folders they are matched below are too.
        if self.drive.composes_unicode {
            for r in &mut rules {
                if !is_nfc(&r.folder) {
                    r.folder = r.folder.nfc().collect();
                }
            }
        }
        self.levels.push((folder.to_owned(), rules.into()));
    }

    /// The rules in force now, to be asked later.
    pub(crate) fn in_force(&self) -> InForce {
        InForce {
            drive: self.drive,
            levels: self.levels.iter().map(|(_, r)| Arc::clone(r)).collect(),
        }
    }

    /// Ends the rules of the folders that don't hold `folder`.
    pub fn leave_to(&mut self, folder: &str) {
        let holds = |outer: &str| {
            outer.is_empty()
                || folder
                    .strip_prefix(outer)
                    .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
        };
        while self.levels.last().is_some_and(|(outer, _)| !holds(outer)) {
            self.levels.pop();
        }
    }

    /// The size of the ignore files whose rules are in force.
    pub fn in_force_bytes(&self) -> u64 {
        self.levels
            .iter()
            .flat_map(|(_, rules)| rules.iter())
            .fold(0, |sum, r| sum.saturating_add(r.bytes))
    }

    /// Bolt's verdict on `path`, as `verdict` gives it.
    pub fn decide(&self, path: impl AsRef<[u8]>, is_dir: bool) -> Decision {
        self.verdict(path, is_dir).bolt
    }

    /// What the rules in force say about `path`, a project path with `/` between names, as Git's
    /// `last_matching_pattern` decides it: the nearest folder's rules first, and in them the last
    /// line that matches. Git's verdict is looked for again, among the `.gitignore` lines, only
    /// when a `.boltignore` line decides Bolt's. Call `leave_to` with the folder that holds `path`
    /// first.
    pub fn verdict(&self, path: impl AsRef<[u8]>, is_dir: bool) -> Verdict {
        let levels = self.levels.iter().map(|(_, rules)| &**rules);
        verdict_in(self.drive, levels, path.as_ref(), is_dir)
    }
}

/// `Matcher::verdict` on `path` with the rules of `levels`, the nearest last, on `drive`.
fn verdict_in<'a, L>(drive: Drive, levels: L, path: &[u8], is_dir: bool) -> Verdict
where
    L: DoubleEndedIterator<Item = &'a [Rules]> + Clone,
{
    // Where the drive composes Unicode, Git reads names composed (`core.precomposeunicode`).
    let composed: Option<String> = match std::str::from_utf8(path) {
        Ok(s) if drive.composes_unicode && !is_nfc(s) => Some(s.nfc().collect()),
        _ => None,
    };
    let path = composed.as_ref().map_or(path, String::as_bytes);
    let name = path.rsplit(|&b| b == b'/').next().unwrap_or(path);
    let casefold = drive.ignores_case;
    let bolt = last_match(levels.clone(), path, name, is_dir, casefold, false);
    let git = match bolt {
        Some((_, false)) => last_match(levels, path, name, is_dir, casefold, true),
        decided => decided,
    };
    let decision = |found: Option<(&Pattern, bool)>| match found {
        Some((p, _)) if p.negative => Decision::Kept,
        Some(_) => Decision::Ignored,
        None => Decision::NoRule,
    };
    Verdict {
        bolt: decision(bolt),
        git: decision(git),
    }
}

/// The line of `levels` that decides `path`, and whether a `.gitignore` holds it: among the lines
/// of every ignore file in force, or of the `.gitignore` files alone.
fn last_match<'a, L>(
    levels: L,
    path: &[u8],
    name: &[u8],
    is_dir: bool,
    casefold: bool,
    gitignore_only: bool,
) -> Option<(&'a Pattern, bool)>
where
    L: DoubleEndedIterator<Item = &'a [Rules]>,
{
    levels
        .rev()
        .flat_map(|rules| rules.iter().rev())
        .filter(|rules| rules.gitignore || !gitignore_only)
        .find_map(|rules| {
            let line = rules.last_match(path, name, is_dir, casefold)?;
            Some((line, rules.gitignore))
        })
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing,
        clippy::arithmetic_side_effects
    )]
    use super::*;
    use proptest::prelude::*;

    fn shown(bytes: &[u8]) -> String {
        String::from_utf8_lossy(bytes).into_owned()
    }

    const STAR_PIECES: &[&[u8]] = &[
        b"a",
        b"b",
        b"A",
        b"B",
        b".",
        b"-",
        b"_",
        b"\xff",
        b"?",
        b"*",
        b"**",
        b"[ab]",
        b"[!ab]",
        b"[^a]",
        b"[a-c]",
        b"[]a]",
        b"[[:digit:]]",
        b"[[:upper:]]",
        b"[A-C]",
        b"\\a",
        b"\\A",
        b"\\*",
        b"\\?",
        b"\\[",
        b"\\\\",
        b"[",
        b"\\",
    ];
    const NAME_BYTES: &[u8] = b"abcABC.-_1*?[]\\/\0\xff";

    proptest! {
        #![proptest_config(ProptestConfig { cases: 5_000, ..ProptestConfig::default() })]
        #[test]
        fn a_line_with_one_star_matches_as_wildmatch_does(
            pieces in prop::collection::vec(prop::sample::select(STAR_PIECES), 0..8),
            name in prop::collection::vec(prop::sample::select(NAME_BYTES), 0..13),
            casefold: bool,
        ) {
            let line = pieces.concat();
            prop_assume!(!line.contains(&b'/'));
            let p = Pattern::parse(&line);
            // The plain matcher: the whole name, a literal tail, or wildmatch.
            let plain = if p.literal == p.text.len() {
                same(&p.text, &name, casefold)
            } else if p.ends_with {
                let tail = &p.text[1..];
                name.len() >= tail.len() && same(tail, &name[name.len() - tail.len()..], casefold)
            } else {
                wildmatch(&p.text, &name, casefold)
            };
            prop_assert_eq!(
                match_basename(&name, &p, casefold),
                plain,
                "{:?} {:?}", shown(&line), shown(&name)
            );
        }
    }

    #[test]
    fn lines_with_one_star_take_the_two_ends() {
        let ends = |line: &str| {
            Pattern::parse(line.as_bytes())
                .ends
                .map(|e| (e.head, e.tail, e.head_width, e.tail_width))
        };
        for (line, want) in [
            ("?*", (1, 2, 1, 0)),
            ("a?*[bc]", (2, 3, 2, 1)),
            // A run of stars inside a name is one `*`.
            ("a**b", (1, 3, 1, 1)),
        ] {
            assert_eq!(ends(line), Some(want), "{line:?}");
        }
        for line in ["*a*", "**", "***", "[a", "a\\", "a/*b", "abc", "*", "*.o"] {
            assert_eq!(ends(line), None, "{line:?}");
        }
    }

    #[test]
    fn the_two_ends_fill_no_table() {
        let rules = Rules::parse(b"*.py[co]\n*.[aos]\n", "", true);
        CELLS.with(|cells| cells.set(0));
        let mut hits = 0;
        for i in 0..1_000 {
            let ext = ["pyc", "pyo", "py", "a", "o", "s", "txt", "PYC"][i % 8];
            let name = format!("file{i}.{ext}");
            let casefold = i % 3 == 0;
            if rules
                .last_match(name.as_bytes(), name.as_bytes(), false, casefold)
                .is_some()
            {
                hits += 1;
            }
        }
        assert!(hits > 500, "{hits}");
        assert_eq!(CELLS.with(std::cell::Cell::get), 0);
    }

    #[test]
    fn matching_takes_time_proportional_to_its_input() {
        // Git's recursion takes exponential time on these; each `**/` doubles it.
        for (pattern, text) in [
            ("*a".repeat(2000) + "b", "a".repeat(4000)),
            ("**/".repeat(1000) + "x", "a/".repeat(1000)),
        ] {
            CELLS.with(|cells| cells.set(0));
            assert!(!wildmatch(pattern.as_bytes(), text.as_bytes(), false));
            // At most a cell for each piece of the pattern, in a row for each byte and one before;
            // and at least one a byte, as no row here empties.
            let cells = CELLS.with(std::cell::Cell::get);
            assert!(cells <= (pattern.len() + 1) * (text.len() + 1), "{cells}");
            assert!(cells >= text.len(), "{cells}");
        }
    }

    #[test]
    fn trailing_spaces_go_as_git_trims_them() {
        for (line, want) in [
            ("foo  ", "foo"),
            ("foo\\ ", "foo\\ "),
            ("foo\\\\ ", "foo\\\\"),
            ("foo\\  ", "foo\\ "),
            ("foo \\ ", "foo \\ "),
            ("trail\\", "trail\\"),
            ("foo\t ", "foo\t"),
            ("   ", ""),
        ] {
            assert_eq!(
                trim_trailing_spaces(line.as_bytes()),
                want.as_bytes(),
                "{line:?}"
            );
        }
    }

    /// The plain `Rules::last_match`: every line tried, the last first.
    fn last_match_by_scan<'r>(
        rules: &'r Rules,
        path: &[u8],
        name: &[u8],
        is_dir: bool,
        casefold: bool,
    ) -> Option<&'r Pattern> {
        rules.patterns.iter().rev().find(|p| {
            (is_dir || !p.dir_only)
                && if p.any_depth {
                    match_basename(name, p, casefold)
                } else {
                    match_pathname(path, rules.folder.as_bytes(), p, casefold)
                }
        })
    }

    const LINE_BODIES: &[&[u8]] = &[
        b"",
        b"a",
        b"A",
        b"ab",
        b"Ab",
        b"aB",
        b"\xc3\xa9",
        b"\xc3\x89",
        b"\xff",
        b"/",
        b"/a",
        b"/A",
        b"/ab",
        b"a/b",
        b"A/B",
        b"/a/b",
        b"/a/B",
        b"b/ab",
        b"a/b/ab",
        b"*.b",
        b"*b",
        b"a*b",
        b"[ab]b",
        b"[ab]",
        b"*",
        b"a/*",
        b"/a/[ab]",
        b"a\\*",
        b"a?",
        b"a\\ ",
    ];
    const PATH_NAMES: &[&[u8]] = &[
        b"",
        b"a",
        b"A",
        b"b",
        b"ab",
        b"Ab",
        b"AB",
        b"\xc3\xa9",
        b"\xff",
        b"x.b",
        b"ab\0x",
    ];

    proptest! {
        #![proptest_config(ProptestConfig { cases: 5_000, ..ProptestConfig::default() })]
        #[test]
        fn the_index_finds_the_line_a_scan_finds(
            lines in prop::collection::vec(
                (any::<bool>(), prop::sample::select(LINE_BODIES), any::<bool>()),
                0..12,
            ),
            folder in prop::sample::select(&["", "a", "a/b"][..]),
            names in prop::collection::vec(prop::sample::select(PATH_NAMES), 1..4),
            is_dir: bool,
            casefold: bool,
        ) {
            let mut text = Vec::new();
            for (negative, body, dir) in &lines {
                if *negative {
                    text.push(b'!');
                }
                text.extend_from_slice(body);
                if *dir {
                    text.push(b'/');
                }
                text.push(b'\n');
            }
            let rules = Rules::parse(&text, folder, true);
            let path = names.join(&b'/');
            let name = *names.last().unwrap();
            let found = rules.last_match(&path, name, is_dir, casefold);
            let scanned = last_match_by_scan(&rules, &path, name, is_dir, casefold);
            let same_line = match (found, scanned) {
                (Some(a), Some(b)) => std::ptr::eq(a, b),
                (None, None) => true,
                _ => false,
            };
            prop_assert!(same_line, "{:?} {:?} {:?}", shown(&text), folder, shown(&path));
        }
    }

    #[test]
    fn plain_lines_are_not_tried_one_by_one() {
        let mut text = String::new();
        for i in 0..5_000 {
            text.push_str(&format!("name{i}\n"));
        }
        text.push_str("*.x\na*b\n[ab]c\n");
        let rules = Rules::parse(text.as_bytes(), "", true);
        FITS.with(|n| n.set(0));
        assert!(
            rules
                .last_match(b"d/other", b"other", false, false)
                .is_none()
        );
        let tried = FITS.with(std::cell::Cell::get);
        assert!(tried <= 3, "{tried}");
    }
}
