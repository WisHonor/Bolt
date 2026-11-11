//! A path one of Git's settings gives, tidied as Git tidies it.

/// `src` tidied as Git tidies a path: runs of separators become one `/`, `.` parts go, and `..`
/// takes away the part before it. `None` when a `..` has no part to take away. The path's root
/// is kept as it is (with `/` for `\`): a leading `/`, and on Windows (`windows`) a drive (any
/// one character and `:`, with the `/` after it) or a network share's `//server/share/`.
pub(crate) fn tidy_path(src: &[u8], windows: bool) -> Option<Vec<u8>> {
    let is_sep = |b: u8| b == b'/' || (windows && b == b'\\');
    let root = if windows {
        windows_root(src, is_sep)
    } else {
        usize::from(src.first() == Some(&b'/'))
    };
    let (root, mut rest) = src.split_at_checked(root)?;
    let mut out: Vec<u8> = root
        .iter()
        .map(|&b| if is_sep(b) { b'/' } else { b })
        .collect();
    let root_len = out.len();
    loop {
        while let Some((&b, after)) = rest.split_first()
            && is_sep(b)
        {
            rest = after;
        }
        if rest.is_empty() {
            return Some(out);
        }
        let end = rest.iter().position(|&b| is_sep(b)).unwrap_or(rest.len());
        let (part, after) = rest.split_at(end);
        match part {
            b"." => {}
            b".." => {
                // Take away the last part and the separator after it, but never the root.
                let kept = out.get(root_len..)?;
                let kept = kept.strip_suffix(b"/").unwrap_or(kept);
                if kept.is_empty() {
                    return None;
                }
                let cut = kept.iter().rposition(|&b| b == b'/').map_or(0, |at| at + 1);
                out.truncate(root_len + cut);
            }
            _ => {
                out.extend_from_slice(part);
                if !after.is_empty() {
                    out.push(b'/');
                }
            }
        }
        rest = after;
    }
}

/// How many bytes of `src` are its root on Windows. A drive is the first character (by its
/// UTF-8 lead byte) and `:`, with one separator after it. A network path is two separators,
/// the server, a separator, the share and the separator after it, if any; one without a
/// separator after the server has no root, so its leading separators go as any others do.
fn windows_root(src: &[u8], is_sep: impl Fn(u8) -> bool) -> usize {
    let first = match src.first() {
        Some(0xF0..) => 4,
        Some(0xE0..) => 3,
        Some(0xC0..) => 2,
        _ => 1,
    };
    if src.get(first) == Some(&b':') {
        return first + 1 + usize::from(src.get(first + 1).is_some_and(|&b| is_sep(b)));
    }
    let sep_after = |from: usize| {
        src.get(from..)
            .and_then(|tail| tail.iter().position(|&b| is_sep(b)))
            .map(|at| from + at)
    };
    match src {
        [a, b, ..] if is_sep(*a) && is_sep(*b) => match sep_after(2) {
            Some(server_end) => {
                sep_after(server_end + 1).map_or(src.len(), |share_end| share_end + 1)
            }
            None => 0,
        },
        [a, ..] if is_sep(*a) => 1,
        _ => 0,
    }
}

#[cfg(test)]
mod tests;
