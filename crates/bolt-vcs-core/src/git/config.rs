//! Git's config files, read as `git config` reads them: the repository's `.git/config` and
//! `config.worktree`, for what decides how to read the repository, and the user's own Git
//! settings, for the few Bolt takes from them. A file Git refuses is refused, and a value Bolt
//! takes that it can't read exactly as Git does is refused rather than guessed.
#![deny(clippy::indexing_slicing, clippy::arithmetic_side_effects)]

use super::{GitError, HashKind, Refusal, broken, shown};

/// What `.git/config` says about how to read the repository.
pub(crate) struct Format {
    pub hash: HashKind,
    pub worktree_config: bool,
    pub sparse: Option<bool>,
    pub includes: bool,
    pub lfs_storage: Option<(Vec<u8>, &'static str)>,
}

/// What `config.worktree` adds.
pub(crate) struct Worktree {
    pub sparse: Option<bool>,
    pub includes: bool,
    pub lfs_storage: Option<(Vec<u8>, &'static str)>,
}

/// A setting Bolt takes from the user's own Git settings.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum UserSetting {
    ExcludesFile(Vec<u8>),
    Include(Vec<u8>),
    LfsStorage(Vec<u8>),
    /// `None` when Git wouldn't read the value as a boolean.
    SparseCheckout(Option<bool>),
}

/// One `name = value` line: the full name as Git spells it (section and key in lower case, the
/// subsection as written), its value, `None` for a name with no `=`, and the line it ends on.
struct Entry<'a> {
    name: &'a [u8],
    value: Option<&'a [u8]>,
    line: usize,
}

/// The bytes of a config file, with CRLF read as LF.
struct Reader<'a> {
    text: &'a [u8],
    pos: usize,
    line: usize,
}

impl Reader<'_> {
    fn next(&mut self) -> Option<u8> {
        let c = *self.text.get(self.pos)?;
        self.pos = self.pos.saturating_add(1);
        if c == b'\r' && self.text.get(self.pos) == Some(&b'\n') {
            self.pos = self.pos.saturating_add(1);
            return Some(b'\n');
        }
        Some(c)
    }

    fn new_line(&mut self) {
        self.line = self.line.saturating_add(1);
    }

    /// Skips a comment, and the line end after it; false at the end of the file.
    fn skip_comment(&mut self) -> bool {
        loop {
            match self.next() {
                None => return false,
                Some(b'\n') => return true,
                Some(_) => {}
            }
        }
    }

    /// A section header after its `[`: the section in lower case, and a subsection after a dot.
    /// `Err` is the line Git names, which is the next one when the file or line ends just after
    /// the name or the closing quote.
    fn header(&mut self) -> Result<Vec<u8>, usize> {
        let here = self.line;
        let next_line = here.saturating_add(1);
        let mut name = Vec::new();
        loop {
            match self.next() {
                None => return Err(next_line),
                Some(b']') if !name.is_empty() => return Ok(name),
                // A subsection may follow no section name at all.
                Some(b' ' | b'\t' | b'\r') => break,
                Some(c) if c.is_ascii_alphanumeric() || c == b'-' || c == b'.' => {
                    name.push(c.to_ascii_lowercase());
                }
                Some(_) => return Err(here),
            }
        }
        let quote = loop {
            match self.next() {
                Some(b' ' | b'\t' | b'\r') => {}
                other => break other,
            }
        };
        if quote != Some(b'"') {
            return Err(here);
        }
        name.push(b'.');
        loop {
            match self.next() {
                None | Some(b'\n') => return Err(here),
                Some(b'"') => break,
                Some(b'\\') => match self.next() {
                    None | Some(b'\n') => return Err(here),
                    Some(c) => name.push(c),
                },
                Some(c) => name.push(c),
            }
        }
        match self.next() {
            Some(b']') => Ok(name),
            None | Some(b'\n') => Err(next_line),
            Some(_) => Err(here),
        }
    }

    /// A value after its `=`, up to the end of its line. `Err` is the line Git names.
    fn value(&mut self) -> Result<(Vec<u8>, usize), usize> {
        let mut value = Vec::new();
        // Whitespace outside quotes at the end isn't kept; `kept` is the length without it.
        let mut kept = 0;
        let mut quoted = false;
        let ended_line = loop {
            let c = match self.next() {
                None | Some(b'\n') if quoted => return Err(self.line),
                None => break false,
                Some(b'\n') => break true,
                Some(b'#' | b';') if !quoted => {
                    let line = self.line;
                    if self.skip_comment() {
                        self.new_line();
                    }
                    value.truncate(kept);
                    return Ok((value, line));
                }
                Some(b'"') => {
                    quoted = !quoted;
                    kept = value.len();
                    continue;
                }
                Some(c @ (b' ' | b'\t' | b'\r')) if !quoted => {
                    if !value.is_empty() {
                        value.push(c);
                    }
                    continue;
                }
                Some(b'\\') => match self.next() {
                    // The end of the file after a backslash counts as a line end too.
                    // and keeps the whitespace before it.
                    None | Some(b'\n') => {
                        self.new_line();
                        kept = value.len();
                        continue;
                    }
                    Some(b'n') => b'\n',
                    Some(b't') => b'\t',
                    Some(b'b') => 0x08,
                    Some(c @ (b'"' | b'\\')) => c,
                    Some(_) => return Err(self.line),
                },
                Some(c) => c,
            };
            value.push(c);
            kept = value.len();
        };
        let line = self.line;
        if ended_line {
            self.new_line();
        }
        value.truncate(kept);
        Ok((value, line))
    }
}

/// Reads `text` as Git's config syntax, giving `each` every entry in order. `Err(Ok(line))` is
/// the first line Git can't read, `Err(Err(e))` what `each` refused.
fn read<E>(
    text: &[u8],
    each: &mut dyn FnMut(Entry) -> Result<(), E>,
) -> Result<(), Result<usize, E>> {
    let start = text.strip_prefix(b"\xef\xbb\xbf").unwrap_or(text);
    let mut r = Reader {
        text: start,
        pos: 0,
        line: 1,
    };
    let mut section: Option<Vec<u8>> = None;
    let mut name = Vec::new();
    while let Some(c) = r.next() {
        match c {
            b'\n' => r.new_line(),
            b' ' | b'\t' | b'\r' => {}
            b'#' | b';' => {
                if r.skip_comment() {
                    r.new_line();
                }
            }
            b'[' => section = Some(r.header().map_err(Ok)?),
            c if c.is_ascii_alphabetic() => {
                let line = r.line;
                name.clear();
                if let Some(section) = &section {
                    name.extend_from_slice(section);
                    name.push(b'.');
                }
                name.push(c.to_ascii_lowercase());
                let mut after = r.next();
                while let Some(c) = after.filter(|c| c.is_ascii_alphanumeric() || *c == b'-') {
                    name.push(c.to_ascii_lowercase());
                    after = r.next();
                }
                while let Some(b' ' | b'\t') = after {
                    after = r.next();
                }
                // Git's names end at a NUL, which a subsection may hold.
                let full = name.split(|&b| b == 0).next().unwrap_or_default();
                match after {
                    None | Some(b'\n') => {
                        each(Entry {
                            name: full,
                            value: None,
                            line,
                        })
                        .map_err(Err)?;
                        if after.is_some() {
                            r.new_line();
                        }
                    }
                    Some(b'=') => {
                        let (value, line) = r.value().map_err(Ok)?;
                        each(Entry {
                            name: full,
                            value: Some(&value),
                            line,
                        })
                        .map_err(Err)?;
                    }
                    Some(_) => return Err(Ok(line)),
                }
            }
            _ => return Err(Ok(r.line)),
        }
    }
    Ok(())
}

/// A value Bolt takes, unless it holds a NUL, where Git would see only the bytes before it.
fn whole(value: &[u8]) -> Option<&[u8]> {
    (!value.contains(&0)).then_some(value)
}

/// Git's whitespace for numbers: C's `isspace`.
fn is_c_space(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\n' | b'\x0b' | b'\x0c' | b'\r')
}

#[derive(Debug, PartialEq, Eq)]
enum BadInt {
    Invalid,
    OutOfRange,
}

/// An integer as Git reads one in a config value: C's `strtoimax` in base 0 (`0x` hex, a leading
/// `0` octal), then an optional `k`, `m` or `g`, all within a 32-bit int.
fn parse_int(text: &[u8]) -> Result<i32, BadInt> {
    let mut rest = text;
    while let Some((&c, tail)) = rest.split_first()
        && is_c_space(c)
    {
        rest = tail;
    }
    let negative = rest.first() == Some(&b'-');
    if let Some((b'-' | b'+', tail)) = rest.split_first() {
        rest = tail;
    }
    let (radix, digits) = match rest {
        [b'0', b'x' | b'X', c, ..] if c.is_ascii_hexdigit() => (16, rest.get(2..).unwrap_or(&[])),
        [b'0', ..] => (8, rest),
        _ => (10, rest),
    };
    let len = digits
        .iter()
        .take_while(|c| char::from(**c).is_digit(radix))
        .count();
    let (number, unit) = digits.split_at_checked(len).ok_or(BadInt::Invalid)?;
    if number.is_empty() {
        return Err(BadInt::Invalid);
    }
    let mut magnitude: i128 = 0;
    for &c in number {
        let digit = char::from(c).to_digit(radix).ok_or(BadInt::Invalid)?;
        magnitude = magnitude
            .checked_mul(i128::from(radix))
            .and_then(|m| m.checked_add(i128::from(digit)))
            .ok_or(BadInt::OutOfRange)?;
        // strtoimax itself stops at 64 bits.
        if magnitude > i128::from(i64::MAX).saturating_add(1) {
            return Err(BadInt::OutOfRange);
        }
    }
    let signed = if negative {
        magnitude.saturating_neg()
    } else {
        magnitude
    };
    if signed > i128::from(i64::MAX) {
        return Err(BadInt::OutOfRange);
    }
    let factor: i128 = match unit {
        [] => 1,
        [b'k' | b'K'] => 1024,
        [b'm' | b'M'] => 1024 * 1024,
        [b'g' | b'G'] => 1024 * 1024 * 1024,
        _ => return Err(BadInt::Invalid),
    };
    let scaled = signed.checked_mul(factor).ok_or(BadInt::OutOfRange)?;
    i32::try_from(scaled).map_err(|_| BadInt::OutOfRange)
}

/// A value as Git reads a boolean: `true`, `yes`, `on`, `false`, `no`, `off` in any letter case,
/// empty for false, or an integer, true unless zero.
pub(crate) fn read_bool(value: &[u8]) -> Option<bool> {
    let is = |word: &str| value.eq_ignore_ascii_case(word.as_bytes());
    if is("true") || is("yes") || is("on") {
        Some(true)
    } else if value.is_empty() || is("false") || is("no") || is("off") {
        Some(false)
    } else {
        parse_int(value).ok().map(|n| n != 0)
    }
}

/// `GIT_CONFIG_COUNT` as Git reads it: C's `strtoul` in base 10 to the end of the text, then no
/// more than an int holds. Empty is none.
pub(crate) fn read_count(text: &[u8]) -> Result<u32, &'static str> {
    const NOT_A_NUMBER: &str = "isn't a number Git reads as a count";
    const TOO_BIG: &str = NOT_A_NUMBER;
    if text.is_empty() {
        return Ok(0);
    }
    let mut rest = text;
    while let Some((&c, tail)) = rest.split_first()
        && is_c_space(c)
    {
        rest = tail;
    }
    let negative = rest.first() == Some(&b'-');
    if let Some((b'-' | b'+', tail)) = rest.split_first() {
        rest = tail;
    }
    if rest.is_empty() || !rest.iter().all(u8::is_ascii_digit) {
        return Err(NOT_A_NUMBER);
    }
    let mut number: std::ffi::c_ulong = 0;
    for &c in rest {
        number = number
            .checked_mul(10)
            .and_then(|n| n.checked_add(std::ffi::c_ulong::from(c.saturating_sub(b'0'))))
            .ok_or(TOO_BIG)?;
    }
    // strtoul negates in an unsigned long, so a negative number wraps round.
    if negative {
        number = number.wrapping_neg();
    }
    // Git wants it to fit an int.
    i32::try_from(number)
        .map(i32::unsigned_abs)
        .map_err(|_| TOO_BIG)
}

/// One `GIT_CONFIG_KEY_<n>` and its value, as Git takes them: `Ok(None)` for a setting Bolt
/// doesn't take, `Err` for a key Git refuses.
pub(crate) fn read_user_variable(key: &[u8], value: &[u8]) -> Result<Option<UserSetting>, ()> {
    let (first, last) = match (
        key.iter().position(|&b| b == b'.'),
        key.iter().rposition(|&b| b == b'.'),
    ) {
        (Some(first), Some(last)) if last > 0 => (first, last),
        _ => return Err(()),
    };
    let (section, subsection, name) = match (
        key.get(..first),
        key.get(first..last),
        key.get(last.saturating_add(1)..),
    ) {
        (Some(section), Some(subsection), Some(name)) => (section, subsection, name),
        _ => return Err(()),
    };
    let key_byte = |c: &u8| c.is_ascii_alphanumeric() || *c == b'-';
    if !section.iter().all(key_byte)
        || subsection.contains(&b'\n')
        || !name.first().is_some_and(u8::is_ascii_alphabetic)
        || !name.iter().all(key_byte)
    {
        return Err(());
    }
    let mut full = section.to_ascii_lowercase();
    full.extend_from_slice(subsection);
    full.push(b'.');
    full.extend(name.iter().map(u8::to_ascii_lowercase));
    Ok(match full.as_slice() {
        b"core.excludesfile" => Some(UserSetting::ExcludesFile(value.to_vec())),
        b"include.path" => Some(UserSetting::Include(value.to_vec())),
        b"lfs.storage" => Some(UserSetting::LfsStorage(value.to_vec())),
        b"core.sparsecheckout" => Some(UserSetting::SparseCheckout(read_bool(value))),
        _ => None,
    })
}

/// The user's own Git settings file: the settings Bolt takes, in order, a plain `include.path`
/// among them. `Err` is the first line Git can't read.
pub(crate) fn read_user_settings(text: &[u8]) -> Result<Vec<UserSetting>, usize> {
    let mut settings = Vec::new();
    read(text, &mut |entry: Entry| {
        let value = entry.value.map(whole);
        let setting = match (entry.name, value) {
            (_, Some(None)) if is_taken_by_user(entry.name) => return Err(entry.line),
            // Git refuses a path with no value.
            (b"core.excludesfile" | b"include.path", None) => return Err(entry.line),
            (b"core.excludesfile", Some(Some(path))) => UserSetting::ExcludesFile(path.to_vec()),
            (b"include.path", Some(Some(path))) => UserSetting::Include(path.to_vec()),
            (b"lfs.storage", value) => {
                UserSetting::LfsStorage(value.flatten().unwrap_or_default().to_vec())
            }
            (b"core.sparsecheckout", None) => UserSetting::SparseCheckout(Some(true)),
            (b"core.sparsecheckout", Some(value)) => {
                UserSetting::SparseCheckout(value.and_then(read_bool))
            }
            _ => return Ok(()),
        };
        settings.push(setting);
        Ok(())
    })
    .map_err(|e| e.unwrap_or_else(|line| line))?;
    Ok(settings)
}

fn is_taken_by_user(name: &[u8]) -> bool {
    matches!(
        name,
        b"core.excludesfile" | b"include.path" | b"lfs.storage" | b"core.sparsecheckout"
    )
}

/// Where `extensions.refStorage` keeps the refs.
#[derive(Clone, Copy, PartialEq, Eq)]
enum RefStorage {
    Files,
    Reftable,
    Elsewhere,
}

/// What the repository's config files set, as far as Bolt reads them.
#[derive(Default)]
struct Seen {
    /// The last `core.repositoryformatversion`, and its line.
    version: Option<(i32, usize)>,
    /// Every extension named, in order, with its line.
    extensions: Vec<(Vec<u8>, usize)>,
    object_format: Option<HashKind>,
    compat: Option<(HashKind, usize)>,
    partial_clone: bool,
    worktree_config: bool,
    refs: Option<RefStorage>,
    sparse: Option<bool>,
    includes: bool,
    lfs_storage: Option<Vec<u8>>,
}

/// The extensions Git knows, and whether a version 0 repository may use each.
const EXTENSIONS: [(&[u8], bool); 10] = [
    (b"noop", true),
    (b"partialclone", true),
    (b"preciousobjects", true),
    (b"worktreeconfig", true),
    (b"noop-v1", false),
    (b"objectformat", false),
    (b"compatobjectformat", false),
    (b"refstorage", false),
    (b"relativeworktrees", false),
    (b"submodulepathconfig", false),
];

fn hash_named(value: &[u8]) -> Option<HashKind> {
    match value {
        b"sha1" => Some(HashKind::Sha1),
        b"sha256" => Some(HashKind::Sha256),
        _ => None,
    }
}

/// Reads one of the repository's config files into `seen`; the format's own settings only from
/// `.git/config` (`common`).
fn read_repository(
    text: &[u8],
    file: &'static str,
    common: bool,
    seen: &mut Seen,
) -> Result<(), GitError> {
    let bad = |line: usize, why: String| broken(format!("line {line} of {file}: {why}"));
    let result = read(text, &mut |entry: Entry| {
        let (name, line) = (entry.name, entry.line);
        let shown_name = || shown(name);
        let missing = || bad(line, format!("{} has no value", shown_name()));
        let value = match entry.value {
            Some(value) => Some(whole(value).ok_or_else(|| {
                bad(
                    line,
                    format!("the value of {} holds a NUL byte", shown_name()),
                )
            })?),
            None => None,
        };
        let boolean = |value: Option<&[u8]>| match value {
            None => Ok(true),
            Some(value) => read_bool(value).ok_or_else(|| {
                bad(
                    line,
                    format!("{} is `{}`, not true or false", shown_name(), shown(value)),
                )
            }),
        };
        match name {
            b"core.sparsecheckout" => seen.sparse = Some(boolean(value)?),
            b"lfs.storage" => seen.lfs_storage = Some(value.unwrap_or_default().to_vec()),
            b"include.path" => match value {
                // Git can't read an include with no path.
                None | Some(b"") => return Err(missing()),
                Some(_) => seen.includes = true,
            },
            _ if name.starts_with(b"includeif.")
                && name.ends_with(b".path")
                && name.len() >= b"includeif..path".len() =>
            {
                seen.includes = true;
            }
            b"core.repositoryformatversion" if common => {
                let version = parse_int(value.unwrap_or_default()).map_err(|_| {
                    bad(
                        line,
                        format!(
                            "core.repositoryformatversion is `{}`, not a number",
                            shown(value.unwrap_or_default())
                        ),
                    )
                })?;
                seen.version = Some((version, line));
            }
            _ if common && name.starts_with(b"extensions.") => {
                let extension = name.get(b"extensions.".len()..).unwrap_or_default();
                seen.extensions.push((extension.to_vec(), line));
                let unknown = |value: &[u8]| {
                    bad(
                        line,
                        format!(
                            "{} is `{}`, which Git doesn't know",
                            shown_name(),
                            shown(value)
                        ),
                    )
                };
                match extension {
                    b"objectformat" => {
                        let value = value.ok_or_else(missing)?;
                        seen.object_format = Some(hash_named(value).ok_or_else(|| unknown(value))?);
                    }
                    b"compatobjectformat" => {
                        let value = value.ok_or_else(missing)?;
                        let hash = hash_named(value).ok_or_else(|| unknown(value))?;
                        if seen.compat.is_some() {
                            return Err(bad(line, format!("{} is set twice", shown_name())));
                        }
                        seen.compat = Some((hash, line));
                    }
                    b"refstorage" => {
                        let value = value.ok_or_else(missing)?;
                        seen.refs = Some(match value {
                            b"files" => RefStorage::Files,
                            b"reftable" => RefStorage::Reftable,
                            _ if value.starts_with(b"files://") => RefStorage::Elsewhere,
                            _ if value.starts_with(b"reftable://") => RefStorage::Reftable,
                            _ => return Err(unknown(value)),
                        });
                    }
                    b"partialclone" => {
                        value.ok_or_else(missing)?;
                        seen.partial_clone = true;
                    }
                    b"worktreeconfig" => seen.worktree_config = boolean(value)?,
                    b"preciousobjects" | b"relativeworktrees" | b"submodulepathconfig" => {
                        boolean(value)?;
                    }
                    _ => {}
                }
            }
            _ => {}
        }
        Ok(())
    });
    match result {
        Ok(()) => Ok(()),
        Err(Ok(line)) => Err(bad(line, "Git can't read this line".to_owned())),
        Err(Err(e)) => Err(e),
    }
}

/// The repository's `.git/config` (`file` names it in messages): its format, refusing what Bolt
/// can't import, and the settings Bolt takes from it.
pub(crate) fn read_format(text: &[u8], file: &'static str) -> Result<Format, GitError> {
    let mut seen = Seen::default();
    read_repository(text, file, true, &mut seen)?;
    let lfs_storage = seen.lfs_storage.take().map(|value| (value, file));
    let unversioned = Format {
        hash: HashKind::Sha1,
        worktree_config: false,
        sparse: seen.sparse,
        includes: seen.includes,
        lfs_storage,
    };
    // Without a version, Git ignores the extensions, once it has checked their values.
    let Some((version, line)) = seen.version else {
        return Ok(unversioned);
    };
    let refuse = |refusal| Err(GitError::Refused(refusal));
    if version < 0 {
        return Err(broken(format!(
            "line {line} of {file}: core.repositoryformatversion is negative"
        )));
    }
    if version > 1 {
        return refuse(Refusal::FormatVersion(version.unsigned_abs().into()));
    }
    let known = |name: &[u8]| EXTENSIONS.iter().find(|(known, _)| *known == name);
    for (name, line) in &seen.extensions {
        match known(name) {
            None if version == 1 => {
                return refuse(Refusal::UnknownExtension(shown(name)));
            }
            Some((_, false)) if version == 0 => {
                return Err(broken(format!(
                    "line {line} of {file}: extensions.{} needs format version 1, and \
                     core.repositoryformatversion is 0",
                    shown(name)
                )));
            }
            _ => {}
        }
    }
    let hash = seen.object_format.unwrap_or(HashKind::Sha1);
    if let Some((compat, line)) = seen.compat
        && compat == hash
    {
        return Err(broken(format!(
            "line {line} of {file}: extensions.compatobjectformat is the repository's own hash"
        )));
    }
    if seen.partial_clone {
        return refuse(Refusal::PartialClone);
    }
    match seen.refs {
        Some(RefStorage::Reftable) => return refuse(Refusal::Reftable),
        Some(RefStorage::Elsewhere) => return refuse(Refusal::RefsElsewhere),
        Some(RefStorage::Files) | None => {}
    }
    Ok(Format {
        hash,
        worktree_config: seen.worktree_config,
        ..unversioned
    })
}

/// `config.worktree`, which Git reads after `.git/config` when `extensions.worktreeConfig` is on:
/// the settings Bolt takes from it.
pub(crate) fn read_worktree(text: &[u8], file: &'static str) -> Result<Worktree, GitError> {
    let mut seen = Seen::default();
    read_repository(text, file, false, &mut seen)?;
    Ok(Worktree {
        sparse: seen.sparse,
        includes: seen.includes,
        lfs_storage: seen.lfs_storage.map(|value| (value, file)),
    })
}

#[cfg(test)]
mod tests;
