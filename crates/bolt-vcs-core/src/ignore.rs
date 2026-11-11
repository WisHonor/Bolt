//! Git's ignore rules, so that Bolt keeps out exactly the files Git keeps out.
//!
//! A rule file is read into lines, each a compiled pattern. Matching is on bytes and never
//! backtracks: a pattern runs as a set of positions stepped once per byte of the path.
#![deny(clippy::indexing_slicing, clippy::arithmetic_side_effects)]

use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::Arc;

use unicode_normalization::UnicodeNormalization;

use crate::drive::Drive;

/// What the rules say of one path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decision {
    /// The last line that matches keeps it out.
    Ignored,
    /// The last line that matches is a `!` line.
    Kept,
    /// No line matches.
    NoRule,
}

/// Bolt's verdict, from every rule, and Git's, from Git's rules alone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Verdict {
    pub bolt: Decision,
    pub git: Decision,
}

/// Whether `pattern` matches the whole of `text`, `/` matched only by `/` and by a `**` that is
/// a whole folder name. With `casefold`, ASCII letters match without regard to case, as Git does
/// with `core.ignoreCase`.
#[must_use]
pub fn wildmatch(pattern: &[u8], text: &[u8], casefold: bool) -> bool {
    compile(pattern).is_some_and(|p| p.matches(text, casefold))
}

/// One piece of a pattern.
#[derive(Debug)]
enum Tok {
    /// This byte; `true` if a `\` made it a letter that keeps its case.
    Byte(u8, bool),
    /// `?`: one byte but `/`.
    One,
    /// `[…]`: one byte of the set.
    Set(Box<Set>),
    /// `*`: any run of bytes but `/`.
    Star,
    /// `**` at the end: any run of bytes.
    All,
    /// `**/`: nothing, or any run of bytes that ends in `/`.
    Dirs,
}

/// The bytes a `[…]` takes, `/` never among them: as written, and for a path whose letters were
/// lowered to ignore their case.
#[derive(Debug)]
struct Set {
    exact: [bool; 256],
    folded: [bool; 256],
}

impl Set {
    fn has(&self, b: u8, fold: bool) -> bool {
        let (set, b) = if fold {
            (&self.folded, b.to_ascii_lowercase())
        } else {
            (&self.exact, b)
        };
        set.get(usize::from(b)).copied().unwrap_or(false)
    }
}

/// A compiled pattern. The first two are the everyday shapes, matched without stepping.
#[derive(Debug)]
enum Pat {
    /// Bytes compared whole.
    Plain(Box<[u8]>),
    /// Plain bytes around one `*`.
    Star {
        head: Box<[u8]>,
        tail: Box<[u8]>,
    },
    Glob(Box<[Tok]>),
}

fn same(a: &[u8], b: &[u8], fold: bool) -> bool {
    if fold {
        a.eq_ignore_ascii_case(b)
    } else {
        a == b
    }
}

impl Pat {
    fn matches(&self, text: &[u8], fold: bool) -> bool {
        match self {
            Pat::Plain(p) => same(p, text, fold),
            Pat::Star { head, tail } => {
                let Some((start, rest)) = text.split_at_checked(head.len()) else {
                    return false;
                };
                let Some((mid, end)) = rest
                    .len()
                    .checked_sub(tail.len())
                    .and_then(|at| rest.split_at_checked(at))
                else {
                    return false;
                };
                same(head, start, fold) && same(tail, end, fold) && !mid.contains(&b'/')
            }
            Pat::Glob(toks) => run(toks, text, fold),
        }
    }
}

/// Steps every position the pattern can be at through `text`, so the time is at most the
/// pattern's length times the text's.
fn run(toks: &[Tok], text: &[u8], fold: bool) -> bool {
    let n = toks.len();
    // `at[i]`: before piece `i` (`at[n]`: the whole pattern matched). `inside[i]`: within the
    // run of the `**/` at `i`, which may end only after a `/`.
    let mut at = vec![false; n.saturating_add(1)];
    let mut inside = vec![false; n];
    let mut next_at = at.clone();
    let mut next_inside = inside.clone();
    reach(&mut at, toks, 0);
    for &c in text {
        next_at.fill(false);
        next_inside.fill(false);
        let mut alive = false;
        for (i, tok) in toks.iter().enumerate() {
            let after = i.saturating_add(1);
            if inside.get(i) == Some(&true) {
                alive = true;
                set(&mut next_inside, i);
                if c == b'/' {
                    reach(&mut next_at, toks, after);
                }
            }
            if at.get(i) != Some(&true) {
                continue;
            }
            alive = true;
            match tok {
                Tok::Byte(p, keep) => {
                    let hit = if fold {
                        c.to_ascii_lowercase() == if *keep { *p } else { p.to_ascii_lowercase() }
                    } else {
                        c == *p
                    };
                    if hit {
                        reach(&mut next_at, toks, after);
                    }
                }
                Tok::One if c != b'/' => reach(&mut next_at, toks, after),
                Tok::Set(s) if s.has(c, fold) => reach(&mut next_at, toks, after),
                Tok::Star if c != b'/' => reach(&mut next_at, toks, i),
                Tok::All => reach(&mut next_at, toks, i),
                Tok::Dirs => {
                    set(&mut next_inside, i);
                    if c == b'/' {
                        reach(&mut next_at, toks, after);
                    }
                }
                _ => {}
            }
        }
        if !alive {
            return false;
        }
        std::mem::swap(&mut at, &mut next_at);
        std::mem::swap(&mut inside, &mut next_inside);
    }
    at.get(n) == Some(&true)
}

fn set(states: &mut [bool], i: usize) {
    if let Some(s) = states.get_mut(i) {
        *s = true;
    }
}

/// Puts position `i` in force, and those after it that a run of nothing reaches.
fn reach(states: &mut [bool], toks: &[Tok], mut i: usize) {
    loop {
        match states.get_mut(i) {
            Some(s) if !*s => *s = true,
            _ => return,
        }
        match toks.get(i) {
            Some(Tok::Star | Tok::All | Tok::Dirs) => i = i.saturating_add(1),
            _ => return,
        }
    }
}

/// Compiles a pattern, or `None` if it can match nothing: a lone `\` at the end, a `[` that
/// never closes, or a class Git doesn't know.
fn compile(pattern: &[u8]) -> Option<Pat> {
    let mut toks: Vec<Tok> = Vec::new();
    let mut rest = pattern;
    while let Some((&c, after)) = rest.split_first() {
        rest = after;
        match c {
            b'\\' => {
                let (&e, after) = rest.split_first()?;
                rest = after;
                toks.push(Tok::Byte(e, e.is_ascii_alphabetic()));
            }
            b'?' => toks.push(Tok::One),
            b'[' => {
                let (s, after) = set_of(rest)?;
                rest = after;
                toks.push(Tok::Set(Box::new(s)));
            }
            b'*' => {
                let more = rest.iter().take_while(|&&b| b == b'*').count();
                rest = rest.get(more..).unwrap_or_default();
                let starts_name =
                    matches!(toks.last(), None | Some(Tok::Byte(b'/', _) | Tok::Dirs));
                if more == 0 || !starts_name {
                    toks.push(Tok::Star);
                } else if let Some(after) = rest.strip_prefix(b"/") {
                    rest = after;
                    toks.push(Tok::Dirs);
                } else if rest.is_empty() {
                    toks.push(Tok::All);
                } else {
                    toks.push(Tok::Star);
                }
            }
            _ => toks.push(Tok::Byte(c, false)),
        }
    }
    Some(simplest(toks))
}

/// The pattern as `Plain` or `Star` where it has that shape.
fn simplest(toks: Vec<Tok>) -> Pat {
    let mut head = Vec::new();
    let mut tail = Vec::new();
    let mut stars = 0u8;
    for tok in &toks {
        match tok {
            Tok::Byte(b, false) => {
                if stars == 0 {
                    head.push(*b);
                } else {
                    tail.push(*b);
                }
            }
            Tok::Star if stars == 0 => stars = 1,
            _ => return Pat::Glob(toks.into_boxed_slice()),
        }
    }
    if stars == 0 {
        Pat::Plain(head.into_boxed_slice())
    } else {
        Pat::Star {
            head: head.into_boxed_slice(),
            tail: tail.into_boxed_slice(),
        }
    }
}

type Class = fn(&u8) -> bool;

fn class_named(name: &[u8]) -> Option<Class> {
    Some(match name {
        b"alnum" => u8::is_ascii_alphanumeric,
        b"alpha" => u8::is_ascii_alphabetic,
        b"blank" => |b| matches!(*b, b' ' | b'\t'),
        b"cntrl" => u8::is_ascii_control,
        b"digit" => u8::is_ascii_digit,
        b"graph" => u8::is_ascii_graphic,
        b"lower" => u8::is_ascii_lowercase,
        b"print" => |b| *b == b' ' || b.is_ascii_graphic(),
        b"punct" => u8::is_ascii_punctuation,
        b"space" => |b| matches!(*b, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r'),
        b"upper" => u8::is_ascii_uppercase,
        b"xdigit" => u8::is_ascii_hexdigit,
        _ => return None,
    })
}

/// Reads a `[…]` whose `[` is already read: its set and what follows it, or `None` if the line
/// can match nothing.
fn set_of(text: &[u8]) -> Option<(Set, &[u8])> {
    let (negated, mut rest) = match text.split_first() {
        Some((b'!' | b'^', after)) => (true, after),
        _ => (false, text),
    };
    let mut bytes = [false; 256];
    let mut ranges: Vec<(u8, u8)> = Vec::new();
    let mut classes: Vec<Class> = Vec::new();
    let mut first = true;
    // The byte just read on its own, which a `-` may make the start of a range.
    let mut prev: Option<u8> = None;
    loop {
        let (&c, after) = rest.split_first()?;
        if c == b']' && !first {
            rest = after;
            break;
        }
        first = false;
        let mut one = c;
        rest = after;
        if c == b'\\' {
            let (&e, after) = rest.split_first()?;
            one = e;
            rest = after;
        } else if let (b'-', Some(lo), Some((&hi, after))) = (c, prev, rest.split_first())
            && hi != b']'
        {
            let (hi, after) = if hi == b'\\' {
                let (&e, after) = after.split_first()?;
                (e, after)
            } else {
                (hi, after)
            };
            ranges.push((lo, hi));
            prev = None;
            rest = after;
            continue;
        } else if c == b'['
            && let Some(body) = rest.strip_prefix(b":")
            && let Some(end) = body.iter().position(|&b| b == b']')
            && let Some(name) = end
                .checked_sub(1)
                .and_then(|colon| body.get(..colon).filter(|_| body.get(colon) == Some(&b':')))
        {
            classes.push(class_named(name)?);
            prev = None;
            rest = body.get(end.saturating_add(1)..).unwrap_or_default();
            continue;
        }
        if let Some(b) = bytes.get_mut(usize::from(one)) {
            *b = true;
        }
        prev = Some(one);
    }
    // Where letter case is ignored the path's letters come lowered: a range or class takes a
    // letter in either case, a byte written alone only as written.
    let takes = |t: u8, fold: bool| {
        let up = t.to_ascii_uppercase();
        let either = |f: &dyn Fn(u8) -> bool| f(t) || (fold && f(up));
        bytes.get(usize::from(t)).copied().unwrap_or(false)
            || ranges
                .iter()
                .any(|&(lo, hi)| either(&|b| (lo..=hi).contains(&b)))
            || classes.iter().any(|class| either(&|b| class(&b)))
    };
    let mut set = Set {
        exact: [false; 256],
        folded: [false; 256],
    };
    for t in 0..=u8::MAX {
        if t == b'/' {
            continue;
        }
        if let Some(e) = set.exact.get_mut(usize::from(t)) {
            *e = takes(t, false) != negated;
        }
        if let Some(f) = set.folded.get_mut(usize::from(t)) {
            *f = takes(t, true) != negated;
        }
    }
    Some((set, rest))
}

/// One line of a rule file that can match something.
#[derive(Debug)]
struct Line {
    pat: Pat,
    /// `!` at the start.
    negated: bool,
    /// `/` at the end.
    dir_only: bool,
    /// No other `/`: it matches a name at any depth.
    basename: bool,
}

impl Line {
    /// Reads one line, without its LF, or `None` if it matches nothing.
    fn read(raw: &[u8]) -> Option<Line> {
        let raw = raw.strip_suffix(b"\r").unwrap_or(raw);
        let raw = raw.split(|&b| b == 0).next().unwrap_or_default();
        if raw.is_empty() || raw.starts_with(b"#") {
            return None;
        }
        let text = without_trailing_spaces(raw);
        let (negated, text) = strip(text, b"!", true);
        let (dir_only, text) = strip(text, b"/", false);
        let (anchored, text) = strip(text, b"/", true);
        if text.is_empty() {
            return None;
        }
        Some(Line {
            pat: compile(text)?,
            negated,
            dir_only,
            basename: !anchored && !text.contains(&b'/'),
        })
    }

    fn decision(&self) -> Decision {
        if self.negated {
            Decision::Kept
        } else {
            Decision::Ignored
        }
    }
}

fn strip<'a>(text: &'a [u8], what: &[u8], front: bool) -> (bool, &'a [u8]) {
    let cut = if front {
        text.strip_prefix(what)
    } else {
        text.strip_suffix(what)
    };
    match cut {
        Some(rest) => (true, rest),
        None => (false, text),
    }
}

/// Drops the spaces at the end of a line, except those a `\` escapes; after a `\` that ends the
/// line none go.
fn without_trailing_spaces(raw: &[u8]) -> &[u8] {
    let mut keep = 0;
    let mut bytes = raw.iter().enumerate();
    while let Some((at, &b)) = bytes.next() {
        if b == b'\\' {
            keep = match bytes.next() {
                Some(_) => at.saturating_add(2),
                None => raw.len(),
            };
        } else if b != b' ' {
            keep = at.saturating_add(1);
        }
    }
    raw.get(..keep).unwrap_or(raw)
}

/// A name's key in a rule file's index of plain names, the same in either letter case.
fn name_key(name: &[u8]) -> u64 {
    name.iter().fold(0xcbf2_9ce4_8422_2325, |h, &b| {
        (h ^ u64::from(b.to_ascii_lowercase())).wrapping_mul(0x100_0000_01b3)
    })
}

/// The rules of one rule file.
#[derive(Debug)]
pub struct Rules {
    /// The folder the file is in, as the path is compared.
    folder: String,
    /// Git's rules (a `.gitignore` or a personal list), not a `.boltignore`.
    gits: bool,
    /// The file's size.
    size: u64,
    lines: Vec<Line>,
    /// The lines with no special character, by `name_key` of their pattern, so that they are
    /// found without trying them one by one.
    plain: HashMap<u64, Vec<usize>>,
    /// Every other line, in order.
    others: Vec<usize>,
}

impl Rules {
    /// Reads one rule file's `text`; `folder` is the folder it sits in (`""` at the top) and
    /// `gitignore` says whether its rules are Git's. A line that can't be used matches nothing.
    #[must_use]
    pub fn parse(text: &[u8], folder: &str, gitignore: bool) -> Rules {
        let body = text.strip_prefix(b"\xef\xbb\xbf").unwrap_or(text);
        let mut rules = Rules {
            folder: folder.to_owned(),
            gits: gitignore,
            size: u64::try_from(text.len()).unwrap_or(u64::MAX),
            lines: Vec::new(),
            plain: HashMap::new(),
            others: Vec::new(),
        };
        for line in body.split(|&b| b == b'\n').filter_map(Line::read) {
            let at = rules.lines.len();
            match &line.pat {
                Pat::Plain(name) => rules.plain.entry(name_key(name)).or_default().push(at),
                _ => rules.others.push(at),
            }
            rules.lines.push(line);
        }
        rules
    }

    /// The last line that matches: `rel` is the path from the file's folder, `base` its name.
    fn last(&self, rel: &[u8], base: &[u8], is_dir: bool, fold: bool) -> Option<&Line> {
        let fits = |&at: &usize| {
            self.lines.get(at).is_some_and(|line| {
                (is_dir || !line.dir_only)
                    && line
                        .pat
                        .matches(if line.basename { base } else { rel }, fold)
            })
        };
        // A plain line matches the whole name or the whole path from the folder.
        let plain = |key| {
            self.plain
                .get(&key)
                .and_then(|lines| lines.iter().rev().copied().find(fits))
        };
        let (by_name, by_path) = (name_key(base), name_key(rel));
        let named = plain(by_name).max(if by_path == by_name {
            None
        } else {
            plain(by_path)
        });
        let other = self
            .others
            .iter()
            .rev()
            .copied()
            .take_while(|&at| named.is_none_or(|n| at > n))
            .find(fits);
        other.or(named).and_then(|at| self.lines.get(at))
    }
}

/// The rule files of one folder, and those of the folders above it.
#[derive(Debug)]
struct Level {
    folder: String,
    rules: Vec<Rules>,
    /// The size of these files and of those above.
    bytes: u64,
    up: Option<Arc<Level>>,
}

/// The rules in force at one point of a walk. Cloning it is cheap, and it keeps giving the
/// answers it gave then.
#[derive(Clone, Debug)]
pub(crate) struct InForce {
    drive: Drive,
    top: Option<Arc<Level>>,
}

impl InForce {
    /// Both verdicts for a project path (`a/b/c`): the last line that matches decides, the
    /// nearest folder's files counting last.
    pub(crate) fn verdict(&self, path: impl AsRef<[u8]>, is_dir: bool) -> Verdict {
        let path = self.composed(path.as_ref());
        let fold = self.drive.ignores_case;
        let base = path.rsplit(|&b| b == b'/').next().unwrap_or_default();
        let (mut bolt, mut git) = (None, None);
        let mut level = self.top.as_deref();
        while let Some(here) = level {
            for rules in here.rules.iter().rev() {
                if bolt.is_some() && (git.is_some() || !rules.gits) {
                    continue;
                }
                let Some(rel) = below(&path, rules.folder.as_bytes(), fold) else {
                    continue;
                };
                if let Some(line) = rules.last(rel, base, is_dir, fold) {
                    bolt.get_or_insert(line.decision());
                    if rules.gits {
                        git.get_or_insert(line.decision());
                    }
                }
            }
            if bolt.is_some() && git.is_some() {
                break;
            }
            level = here.up.as_deref();
        }
        Verdict {
            bolt: bolt.unwrap_or(Decision::NoRule),
            git: git.unwrap_or(Decision::NoRule),
        }
    }

    /// The path as this drive compares names: composed where it composes them.
    fn composed<'a>(&self, path: &'a [u8]) -> Cow<'a, [u8]> {
        if !self.drive.composes_unicode || path.is_ascii() {
            return Cow::Borrowed(path);
        }
        match std::str::from_utf8(path) {
            Ok(s) if !unicode_normalization::is_nfc(s) => {
                Cow::Owned(s.nfc().collect::<String>().into_bytes())
            }
            _ => Cow::Borrowed(path),
        }
    }
}

/// The part of `path` below `folder`, if it is below it.
fn below<'a>(path: &'a [u8], folder: &[u8], fold: bool) -> Option<&'a [u8]> {
    let rest = if folder.is_empty() {
        path
    } else {
        let (start, rest) = path.split_at_checked(folder.len())?;
        if !same(start, folder, fold) {
            return None;
        }
        rest.strip_prefix(b"/")?
    };
    (!rest.is_empty()).then_some(rest)
}

/// The rules in force as a walk goes down and back up the project's folders.
#[derive(Debug)]
pub struct Matcher {
    now: InForce,
}

impl Matcher {
    #[must_use]
    pub fn new(drive: Drive) -> Matcher {
        Matcher {
            now: InForce { drive, top: None },
        }
    }

    /// The walk enters `folder` and gives the rule files read there, in order: at the top Git's
    /// personal lists, then `.gitignore`, then `.boltignore`.
    pub fn enter(&mut self, folder: &str, mut rules: Vec<Rules>) {
        if self.now.drive.composes_unicode {
            for r in &mut rules {
                if !r.folder.is_ascii() {
                    r.folder = r.folder.nfc().collect();
                }
            }
        }
        let below = self.in_force_bytes();
        let bytes = rules
            .iter()
            .fold(below, |sum, r| sum.saturating_add(r.size));
        self.now.top = Some(Arc::new(Level {
            folder: folder.to_owned(),
            rules,
            bytes,
            up: self.now.top.take(),
        }));
    }

    /// The walk goes back up to `folder`: the rules of folders that don't hold it stop applying.
    pub fn leave_to(&mut self, folder: &str) {
        while let Some(level) = &self.now.top {
            let f = level.folder.as_str();
            let holds = f.is_empty()
                || folder
                    .strip_prefix(f)
                    .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'));
            if holds {
                return;
            }
            self.now.top = level.up.clone();
        }
    }

    /// The total size of the rule files in force now.
    #[must_use]
    pub fn in_force_bytes(&self) -> u64 {
        self.now.top.as_ref().map_or(0, |level| level.bytes)
    }

    /// Bolt's verdict for a project path (`a/b/c`) under the rules in force.
    #[must_use]
    pub fn decide(&self, path: impl AsRef<[u8]>, is_dir: bool) -> Decision {
        self.now.verdict(path, is_dir).bolt
    }

    /// Bolt's verdict and Git's.
    #[must_use]
    pub fn verdict(&self, path: impl AsRef<[u8]>, is_dir: bool) -> Verdict {
        self.now.verdict(path, is_dir)
    }

    /// A snapshot of the rules in force, to judge a path later with the same answer.
    pub(crate) fn in_force(&self) -> InForce {
        self.now.clone()
    }
}

#[cfg(test)]
mod tests;
