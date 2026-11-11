//! `.git/config`, read as Git reads it, keeping only what decides how to read the repository:
//! `core.repositoryformatversion`, the `extensions.*` keys and `core.sparsecheckout` (decisions
//! 031 and 039), and `lfs.storage`, which can keep Git LFS's files elsewhere. Includes aren't
//! followed, as Git's own format check doesn't follow them, but whether there are any is noted:
//! they can set `core.sparsecheckout` (decision 043). The user's own Git settings are read with
//! the same parser, for their ignore list (decisions 047 and 060), `lfs.storage` and
//! `core.sparsecheckout`, and so are those the environment gives, with the variables that place
//! them.
#![deny(clippy::indexing_slicing, clippy::arithmetic_side_effects)]

use crate::git::{GitError, HashKind, Refusal, broken, is_git_space};

/// What `.git/config` says about how to read the repository.
pub(crate) struct Format {
    pub hash: HashKind,
    /// `extensions.worktreeconfig`: Git reads `config.worktree` too.
    pub worktree_config: bool,
    /// The last `core.sparsecheckout`, if any.
    pub sparse: Option<bool>,
    /// Whether it has an `[include]` or `[includeIf]` section, whose files Bolt doesn't read.
    pub includes: bool,
    /// The last `lfs.storage`, empty for one with no value, and the file it is in.
    pub lfs_storage: Option<(Vec<u8>, &'static str)>,
}

/// Reads `text`, the config named `file` in messages, as Git's `read_repository_format` and
/// `verify_repository_format` do, then refuses reftable and partial clones.
pub(crate) fn read_format(text: &[u8], file: &'static str) -> Result<Format, GitError> {
    let mut found = Found::default();
    parse(text, &mut |name, value| found.take(name, value)).map_err(|line| bad_line(file, line))?;
    found.check(file)
}

/// What `config.worktree` adds to `.git/config`.
pub(crate) struct Worktree {
    /// The last `core.sparsecheckout`, if any.
    pub sparse: Option<bool>,
    /// Whether it has an `[include]` or `[includeIf]` section.
    pub includes: bool,
    /// The last `lfs.storage`, empty for one with no value, and the file it is in.
    pub lfs_storage: Option<(Vec<u8>, &'static str)>,
}

/// Reads `text`, a `config.worktree` named `file` in messages, for what Git takes from it beside
/// `.git/config`.
pub(crate) fn read_worktree(text: &[u8], file: &'static str) -> Result<Worktree, GitError> {
    let mut read = Worktree {
        sparse: None,
        includes: false,
        lfs_storage: None,
    };
    parse(text, &mut |name, value| {
        if name == b"core.sparsecheckout" {
            read.sparse = Some(parse_bool(value)?);
        } else if name == b"lfs.storage" {
            read.lfs_storage = Some((value.unwrap_or_default().to_vec(), file));
        }
        read.includes |= is_include(name);
        Some(())
    })
    .map_err(|line| bad_line(file, line))?;
    Ok(read)
}

/// A setting in the user's own Git settings that Bolt reads: one that decides their ignore list
/// (decisions 047 and 060), `lfs.storage`, which can keep Git LFS's files elsewhere, or
/// `core.sparsecheckout`, which an import refuses.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum UserSetting {
    /// A `core.excludesfile`.
    ExcludesFile(Vec<u8>),
    /// A plain `[include]` path, whose file Git reads at this place.
    Include(Vec<u8>),
    /// An `lfs.storage`, empty for one with no value.
    LfsStorage(Vec<u8>),
    /// A `core.sparsecheckout`, `None` for a value Git refuses.
    SparseCheckout(Option<bool>),
}

/// The settings in `text`, the user's own Git settings, that Bolt reads, in order, or the number
/// of the first line Git couldn't read. A path with no value is refused, as Git refuses it.
/// `[includeIf]` paths aren't given: their conditions need a repository.
pub(crate) fn read_user_settings(text: &[u8]) -> Result<Vec<UserSetting>, usize> {
    let mut found = Vec::new();
    parse(text, &mut |name, value| {
        found.extend(user_setting(name, value).ok()?);
        Some(())
    })?;
    Ok(found)
}

/// The setting Bolt reads that the environment gives as `key`, a `GIT_CONFIG_KEY_<n>`, and
/// `value`, its `GIT_CONFIG_VALUE_<n>`, if it is one. `Err` for a key Git refuses.
pub(crate) fn read_user_variable(key: &[u8], value: &[u8]) -> Result<Option<UserSetting>, ()> {
    user_setting(&variable_name(key).ok_or(())?, Some(value))
}

/// The setting Bolt reads that `name`, as `parse` gives names, is with `value`, if it is one.
/// `Err` for a path with no value, which Git refuses; `lfs.storage` with none is empty.
fn user_setting(name: &[u8], value: Option<&[u8]>) -> Result<Option<UserSetting>, ()> {
    let path = || value.map(<[u8]>::to_vec).ok_or(());
    Ok(Some(match name {
        b"core.excludesfile" => UserSetting::ExcludesFile(path()?),
        b"include.path" => UserSetting::Include(path()?),
        b"lfs.storage" => UserSetting::LfsStorage(value.unwrap_or_default().to_vec()),
        b"core.sparsecheckout" => UserSetting::SparseCheckout(parse_bool(value)),
        _ => return Ok(None),
    }))
}

/// Git's `git_config_parse_key`: the name `key` gives, in `parse`'s form, with the section and
/// key in lowercase and the subsection as written. `None` for what Git refuses: no `.` past the
/// first byte, nothing after the last, a section or key with a byte that isn't a letter, digit or
/// `-`, a key that doesn't start with a letter, or a line break in the subsection.
fn variable_name(key: &[u8]) -> Option<Vec<u8>> {
    let last_dot = key.iter().rposition(|&c| c == b'.').filter(|&at| at > 0)?;
    let first_of_key = last_dot.checked_add(1)?;
    key.get(first_of_key).filter(|c| c.is_ascii_alphabetic())?;
    let mut in_section = true;
    let mut name = Vec::with_capacity(key.len());
    for (at, &c) in key.iter().enumerate() {
        in_section &= c != b'.';
        if in_section || at >= first_of_key {
            if !is_key_char(c) {
                return None;
            }
            name.push(c.to_ascii_lowercase());
        } else if c == b'\n' {
            return None;
        } else {
            name.push(c);
        }
    }
    Some(name)
}

/// Git's `git_parse_maybe_bool` for a value that is there, as `git_env_bool` reads a variable of
/// the environment: `None` where Git stops.
pub(crate) fn read_bool(value: &[u8]) -> Option<bool> {
    parse_bool(Some(value))
}

/// `GIT_CONFIG_COUNT` read as Git's `git_config_from_parameters` reads it: C's `strtoul` in base
/// 10 (whitespace, a sign, then digits; a negative number wraps round as an `unsigned long` does,
/// and one too big for it is its largest), which must read the whole text, so an empty one is 0;
/// then at most `INT_MAX`. `Err` says why Git refuses it.
pub(crate) fn read_count(text: &[u8]) -> Result<u32, &'static str> {
    use std::ffi::c_ulong;
    let after_space = text
        .iter()
        .position(|c| !matches!(c, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r'))
        .and_then(|at| text.get(at..))
        .unwrap_or_default();
    let (negative, digits) = match after_space.split_first() {
        Some((b'-', rest)) => (true, rest),
        Some((b'+', rest)) => (false, rest),
        _ => (false, after_space),
    };
    // With no digit `strtoul` reads nothing, so only an empty text is read whole.
    if digits.is_empty() || !digits.iter().all(u8::is_ascii_digit) {
        return if text.is_empty() {
            Ok(0)
        } else {
            Err("isn't a number Git reads as a count")
        };
    }
    let zero: c_ulong = 0;
    let read = digits.iter().try_fold(zero, |n, &d| {
        n.checked_mul(10)?
            .checked_add(c_ulong::from(d.checked_sub(b'0')?))
    });
    let count = match read {
        None => c_ulong::MAX,
        Some(n) if negative => n.wrapping_neg(),
        Some(n) => n,
    };
    // At most `INT_MAX`; `count` is never negative.
    i32::try_from(count)
        .ok()
        .and_then(|n| u32::try_from(n).ok())
        .ok_or("is more than Git takes")
}

/// Whether the setting `name` is in an `[include]` or `[includeIf]` section, where Git takes
/// other files to read.
fn is_include(name: &[u8]) -> bool {
    let section = name.split(|&c| c == b'.').next().unwrap_or_default();
    section == b"include" || section == b"includeif"
}

fn bad_line(file: &str, line: usize) -> GitError {
    broken(format!("it can't read line {line} of {file}"))
}

/// The settings `read_format` keeps, as Git's `check_repo_format` records them.
#[derive(Default)]
struct Found {
    version: Option<i64>,
    sparse: Option<bool>,
    hash: Option<HashKind>,
    worktree_config: bool,
    partial_clone: bool,
    /// What the last `extensions.refstorage` makes Bolt refuse, if anything.
    ref_storage: Option<Refusal>,
    /// Extensions Git reads only in format version 1.
    v1_only: Vec<String>,
    unknown: Vec<String>,
    includes: bool,
    lfs_storage: Option<Vec<u8>>,
}

impl Found {
    /// Takes one setting; `None` for a value Git refuses too, or one Bolt can't read as Git does.
    fn take(&mut self, name: &[u8], value: Option<&[u8]>) -> Option<()> {
        self.includes |= is_include(name);
        if name == b"core.repositoryformatversion" {
            self.version = Some(parse_int(value?)?);
        } else if name == b"core.sparsecheckout" {
            self.sparse = Some(parse_bool(value)?);
        } else if name == b"lfs.storage" {
            self.lfs_storage = Some(value.unwrap_or_default().to_vec());
        } else if let Some(ext) = name.strip_prefix(b"extensions.") {
            self.extension(&String::from_utf8_lossy(ext), value)?;
        }
        Some(())
    }

    /// Git's `handle_extension_v0`, then `handle_extension`.
    fn extension(&mut self, ext: &str, value: Option<&[u8]>) -> Option<()> {
        match ext {
            "noop" => {}
            "preciousobjects" => {
                parse_bool(value)?;
            }
            "partialclone" => {
                value?;
                self.partial_clone = true;
            }
            "worktreeconfig" => self.worktree_config = parse_bool(value)?,
            _ => {
                match ext {
                    "noop-v1" => {}
                    "objectformat" => self.hash = Some(hash_named(value?)?),
                    "compatobjectformat" => {
                        hash_named(value?)?;
                        // Git takes it only once.
                        if self.v1_only.iter().any(|e| e == ext) {
                            return None;
                        }
                    }
                    "refstorage" => self.ref_storage(value?)?,
                    "relativeworktrees" | "submodulepathconfig" => {
                        parse_bool(value)?;
                    }
                    _ => {
                        self.unknown.push(ext.to_owned());
                        return Some(());
                    }
                }
                self.v1_only.push(ext.to_owned());
            }
        }
        Some(())
    }

    /// `extensions.refstorage`: `files` or `reftable`, each maybe with a `://` payload.
    fn ref_storage(&mut self, value: &[u8]) -> Option<()> {
        let (format, payload) = match value.windows(3).position(|w| w == b"://") {
            Some(at) => value.split_at_checked(at)?,
            None => (value, &b""[..]),
        };
        self.ref_storage = match format {
            b"files" if payload.is_empty() => None,
            // The payload is a path for the refs, which Bolt doesn't follow.
            b"files" => Some(Refusal::RefsElsewhere),
            b"reftable" => Some(Refusal::Reftable),
            _ => return None,
        };
        Some(())
    }

    /// Git's `verify_repository_format`, then what Bolt refuses on top.
    fn check(self, file: &'static str) -> Result<Format, GitError> {
        let lfs_storage = self.lfs_storage.map(|value| (value, file));
        // Without a format version, Git ignores every extension.
        let Some(version) = self.version else {
            return Ok(Format {
                hash: HashKind::Sha1,
                worktree_config: false,
                sparse: self.sparse,
                includes: self.includes,
                lfs_storage,
            });
        };
        match version {
            0 => {
                if let Some(ext) = self.v1_only.first() {
                    return Err(broken(format!(
                        "{file} sets extensions.{ext}, which needs format version 1, but the \
                         version is 0"
                    )));
                }
            }
            1 => {
                if let Some(ext) = self.unknown.into_iter().next() {
                    return Err(GitError::Refused(Refusal::UnknownExtension(ext)));
                }
            }
            _ => {
                return Err(match u64::try_from(version) {
                    Ok(v) => GitError::Refused(Refusal::FormatVersion(v)),
                    Err(_) => broken(format!(
                        "{file} gives the format version {version}, which Git never writes"
                    )),
                });
            }
        }
        if let Some(refusal) = self.ref_storage {
            return Err(GitError::Refused(refusal));
        }
        if self.partial_clone {
            return Err(GitError::Refused(Refusal::PartialClone));
        }
        Ok(Format {
            hash: self.hash.unwrap_or(HashKind::Sha1),
            worktree_config: self.worktree_config,
            sparse: self.sparse,
            includes: self.includes,
            lfs_storage,
        })
    }
}

/// Git's `hash_algo_by_name`.
fn hash_named(value: &[u8]) -> Option<HashKind> {
    match value {
        b"sha1" => Some(HashKind::Sha1),
        b"sha256" => Some(HashKind::Sha256),
        _ => None,
    }
}

/// Git's `git_parse_maybe_bool`: no value is true, an empty one false, then the words, then any
/// number Git reads, true unless zero.
fn parse_bool(value: Option<&[u8]>) -> Option<bool> {
    let Some(value) = value else {
        return Some(true);
    };
    if value.is_empty() {
        return Some(false);
    }
    let is = |words: [&[u8]; 3]| words.iter().any(|w| value.eq_ignore_ascii_case(w));
    if is([b"true", b"yes", b"on"]) {
        Some(true)
    } else if is([b"false", b"no", b"off"]) {
        Some(false)
    } else {
        parse_int(value).map(|n| n != 0)
    }
}

/// Git's `git_parse_int`: C's `strtoimax` in base 0 (leading whitespace, a sign, then octal after
/// a `0`, hex after `0x`, otherwise decimal), then an optional `k`, `m` or `g`, and it must fit
/// an `int`.
fn parse_int(value: &[u8]) -> Option<i64> {
    let start = value
        .iter()
        .position(|c| !matches!(c, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r'))?;
    let text = value.get(start..)?;
    let (negative, text) = match text.split_first() {
        Some((b'-', rest)) => (true, rest),
        Some((b'+', rest)) => (false, rest),
        _ => (false, text),
    };
    let (radix, text) = match text {
        [b'0', b'x' | b'X', digit, ..] if digit.is_ascii_hexdigit() => (16, text.get(2..)?),
        [b'0', ..] => (8, text),
        _ => (10, text),
    };
    let digits = text
        .iter()
        .take_while(|c| char::from(**c).is_digit(radix))
        .count();
    let (digits, unit) = text.split_at_checked(digits)?;
    let mut magnitude: i128 = 0;
    for &digit in digits {
        let digit = i128::from(char::from(digit).to_digit(radix)?);
        magnitude = magnitude
            .checked_mul(i128::from(radix))?
            .checked_add(digit)?;
    }
    if digits.is_empty() {
        return None;
    }
    let factor = match unit {
        b"" => 1,
        _ if unit.eq_ignore_ascii_case(b"k") => 1 << 10,
        _ if unit.eq_ignore_ascii_case(b"m") => 1 << 20,
        _ if unit.eq_ignore_ascii_case(b"g") => 1 << 30,
        _ => return None,
    };
    let value = if negative {
        magnitude.checked_neg()?
    } else {
        magnitude
    };
    // An `int` fits strtoimax's own range, so this one check is both of Git's.
    i32::try_from(value.checked_mul(factor)?)
        .ok()
        .map(i64::from)
}

/// Git's `iskeychar`.
fn is_key_char(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'-'
}

/// The text being parsed, read as Git's `get_next_char` reads it.
struct Source<'a> {
    bytes: std::iter::Peekable<std::slice::Iter<'a, u8>>,
    line: usize,
    eof: bool,
}

impl Source<'_> {
    /// The next byte, with CRLF read as LF, and the end of the text read as one more LF.
    fn next_char(&mut self) -> u8 {
        let c = match self.bytes.next() {
            Some(b'\r') if self.bytes.next_if_eq(&&b'\n').is_some() => b'\n',
            Some(&c) => c,
            None => {
                self.eof = true;
                self.line = self.line.saturating_add(1);
                return b'\n';
            }
        };
        if c == b'\n' {
            self.line = self.line.saturating_add(1);
        }
        c
    }

    /// For an error found at a line's end: the line that ended, not the next.
    fn unread_line(&mut self) {
        self.line = self.line.saturating_sub(1);
    }
}

const BOM: &[u8] = b"\xef\xbb\xbf";

/// What `parse` gives each setting to: its name and value. `None` refuses the setting's line.
type Each<'a> = dyn FnMut(&[u8], Option<&[u8]>) -> Option<()> + 'a;

/// Git's `git_parse_source`: calls `each` with every setting's name (section, subsection and key,
/// the section and key in lowercase) and value (`None` for a key without `=`). Stops at the first
/// line it can't read, or whose setting `each` refuses, giving that line's number, as Git's "bad
/// config line" does.
fn parse(text: &[u8], each: &mut Each) -> Result<(), usize> {
    let mut src = Source {
        bytes: text.iter().peekable(),
        line: 1,
        eof: false,
    };
    let mut name = Vec::new();
    let mut section_len = 0;
    let mut comment = false;
    // What is left of a UTF-8 byte order mark the text may start with.
    let mut bom = BOM;
    loop {
        let c = src.next_char();
        if let Some((&want, rest)) = bom.split_first() {
            if c == want {
                bom = rest;
                continue;
            }
            if bom.len() != BOM.len() {
                return Err(src.line);
            }
            bom = &[];
        }
        if c == b'\n' {
            if src.eof {
                return Ok(());
            }
            comment = false;
            continue;
        }
        if comment || is_git_space(c) {
            continue;
        }
        if c == b'#' || c == b';' {
            comment = true;
            continue;
        }
        if c == b'[' {
            name.clear();
            if section(&mut src, &mut name).is_none() || name.is_empty() {
                return Err(src.line);
            }
            name.push(b'.');
            section_len = name.len();
            continue;
        }
        if !c.is_ascii_alphabetic() {
            return Err(src.line);
        }
        name.truncate(section_len);
        name.push(c.to_ascii_lowercase());
        setting(&mut src, &mut name, each)?;
    }
}

/// Git's `get_base_var`: a section name after `[`, in lowercase, up to `]`.
fn section(src: &mut Source, name: &mut Vec<u8>) -> Option<()> {
    loop {
        let c = src.next_char();
        if src.eof {
            return None;
        }
        if c == b']' {
            return Some(());
        }
        if is_git_space(c) {
            return subsection(src, name, c);
        }
        if !is_key_char(c) && c != b'.' {
            return None;
        }
        name.push(c.to_ascii_lowercase());
    }
}

/// Git's `get_extended_base_var`: `"subsection"]`, kept as written, after the space `c`.
fn subsection(src: &mut Source, name: &mut Vec<u8>, mut c: u8) -> Option<()> {
    loop {
        if c == b'\n' {
            src.unread_line();
            return None;
        }
        c = src.next_char();
        if !is_git_space(c) {
            break;
        }
    }
    if c != b'"' {
        return None;
    }
    name.push(b'.');
    loop {
        let mut c = src.next_char();
        if c == b'\n' {
            src.unread_line();
            return None;
        }
        if c == b'"' {
            break;
        }
        if c == b'\\' {
            c = src.next_char();
            if c == b'\n' {
                src.unread_line();
                return None;
            }
        }
        // Git would end the name at a NUL, so Bolt can't read it as Git does.
        if c == 0 {
            return None;
        }
        name.push(c);
    }
    (src.next_char() == b']').then_some(())
}

/// Git's `get_value`: the rest of a key, then its value if it has one, given to `each`.
fn setting(src: &mut Source, name: &mut Vec<u8>, each: &mut Each) -> Result<(), usize> {
    let mut c = src.next_char();
    while is_key_char(c) {
        name.push(c.to_ascii_lowercase());
        c = src.next_char();
    }
    while c == b' ' || c == b'\t' {
        c = src.next_char();
    }
    let value = if c == b'\n' {
        None
    } else if c != b'=' {
        return Err(src.line);
    } else {
        match value(src) {
            Some(value) => Some(value),
            None => return Err(src.line),
        }
    };
    // The line just read, not the one after it.
    let line = src.line.saturating_sub(1);
    each(name, value.as_deref()).ok_or(line)
}

/// Git's `parse_value`: quotes; the escapes `\"`, `\\`, `\n`, `\t` and `\b`; a `\` at a line's end
/// that continues it; comments; whitespace trimmed outside quotes.
fn value(src: &mut Source) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let (mut quote, mut comment) = (false, false);
    // Where whitespace outside quotes began, to cut it off if nothing follows it.
    let mut trim_len = 0;
    loop {
        let c = src.next_char();
        if c == b'\n' {
            if quote {
                src.unread_line();
                return None;
            }
            if trim_len != 0 {
                out.truncate(trim_len);
            }
            return Some(out);
        }
        if comment {
            continue;
        }
        if is_git_space(c) && !quote {
            if trim_len == 0 {
                trim_len = out.len();
            }
            if !out.is_empty() {
                out.push(c);
            }
            continue;
        }
        if !quote && (c == b';' || c == b'#') {
            comment = true;
            continue;
        }
        trim_len = 0;
        let c = match c {
            b'\\' => match src.next_char() {
                b'\n' => continue,
                b't' => b'\t',
                b'b' => 0x08,
                b'n' => b'\n',
                c @ (b'\\' | b'"') => c,
                _ => return None,
            },
            b'"' => {
                quote = !quote;
                continue;
            }
            // Git would end the value at a NUL, so Bolt can't read it as Git does.
            0 => return None,
            c => c,
        };
        out.push(c);
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    #![allow(clippy::indexing_slicing, clippy::arithmetic_side_effects)]
    use super::*;

    /// Every setting in `text` as `git config --list` shows it: the name, then the value, if any.
    fn list(text: &[u8]) -> Result<Vec<(String, Option<String>)>, usize> {
        let mut out = Vec::new();
        parse(text, &mut |name, value| {
            let text = |b: &[u8]| String::from_utf8_lossy(b).into_owned();
            out.push((text(name), value.map(text)));
            Some(())
        })?;
        Ok(out)
    }

    /// `list`, for settings that all have values.
    fn values(text: &str) -> Vec<(String, String)> {
        list(text.as_bytes())
            .unwrap()
            .into_iter()
            .map(|(name, value)| (name, value.unwrap()))
            .collect()
    }

    fn pairs(rows: &[(&str, &str)]) -> Vec<(String, String)> {
        rows.iter()
            .map(|(n, v)| ((*n).to_owned(), (*v).to_owned()))
            .collect()
    }

    /// A byte order mark, CRLF line ends, `\t` and `\b`, a key on a section's line, and where a
    /// line that can't be read is counted.
    #[test]
    fn config_reading_follows_git_at_the_edges() {
        let s = |v: &str| Some(v.to_owned());
        assert_eq!(
            list(b"\xef\xbb\xbf[core]\r\n\tbare = a\\tb\\bc\r\n[x] y = 1\r\n").unwrap(),
            [
                ("core.bare".to_owned(), s("a\tb\u{8}c")),
                ("x.y".to_owned(), s("1")),
            ]
        );
        // A lone CR is whitespace; a section name's letter case folds, a subsection's doesn't.
        assert_eq!(
            values("[Core \"Sub\\\"x\"]\r\tKey = v \r\n"),
            pairs(&[("core.Sub\"x.key", "v")])
        );
        for (text, line) in [
            // Part of a byte order mark.
            (&b"\xef\xbbx = 1\n"[..], 1),
            (b"\xefx\n", 1),
            (b"[core]\n\tbare = \"x\n", 2),
            (b"[core]\n\tbare = a\\x\n", 2),
            (b"[core]\n\tbare ; no value\n", 2),
            (b"[core]\n\t-bare = 1\n", 2),
            (b"[]\n", 1),
            (b"[core \"x\" ]\n", 1),
            (b"[core x]\n", 1),
            (b"[core \"x\n", 1),
            // At the end of the text Git counts one line past the last.
            (b"[core", 2),
            // Git would end these at the NUL, so Bolt can't read them as Git does.
            (b"[core]\n\tbare = a\0b\n", 2),
            (b"[core \"a\0\"]\n", 1),
        ] {
            assert_eq!(list(text), Err(line), "{text:?}");
        }
        // NUL is harmless in a comment.
        assert_eq!(values("# a\0b\n[a]\nb = c\n"), pairs(&[("a.b", "c")]));
        assert_eq!(list(b"").unwrap(), []);
        assert_eq!(list(b"\xef\xbb\xbf").unwrap(), []);
    }

    #[test]
    fn booleans_and_numbers_read_as_git_reads_them() {
        for (value, want) in [
            (None, Some(true)),
            (Some(&b"on"[..]), Some(true)),
            (Some(b"Off"), Some(false)),
            (Some(b"foobar"), None),
        ] {
            assert_eq!(parse_bool(value), want, "{value:?}");
        }
        for (value, want) in [
            // C's strtoimax in base 0: octal after a 0, hex after 0x, leading space, a sign.
            (&b"010"[..], Some(8)),
            (b"0x1F", Some(31)),
            (b" \t+7", Some(7)),
            (b"-2", Some(-2)),
            (b"08", None),
            (b"0x", None),
            (b"7 ", None),
            (b"", None),
            (b"-", None),
            // It must fit an int, unit and all.
            (b"2147483647", Some(2_147_483_647)),
            (b"2147483648", None),
            (b"-2147483648", Some(-2_147_483_648)),
            (b"2g", None),
            (b"1g", Some(1 << 30)),
            (b"99999999999999999999", None),
        ] {
            assert_eq!(parse_int(value), want, "{value:?}");
        }
    }
}
