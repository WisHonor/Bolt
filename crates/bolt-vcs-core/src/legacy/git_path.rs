//! A path one of Git's settings gives, tidied as Git tidies it.

/// Git's `normalize_path_copy`, which Git runs on `GIT_CONFIG_SYSTEM` alone: on Windows (`windows`)
/// `\` is a separator too, and a drive or `//server/share/` starts a path; each run of separators
/// becomes one `/`; a `.` part is dropped; and a `..` part takes the part before it away. Only the
/// text changes. `None` where a `..` would climb above the path's start, where Git's fails.
#[deny(clippy::indexing_slicing, clippy::arithmetic_side_effects)]
pub(crate) fn tidy_path(src: &[u8], windows: bool) -> Option<Vec<u8>> {
    let is_sep = |c: u8| c == b'/' || (windows && c == b'\\');
    // Past the end reads as the C string's closing NUL.
    let at = |i: usize| src.get(i).copied().unwrap_or(0);
    let skip_seps = |mut i: usize| {
        while is_sep(at(i)) {
            i = i.saturating_add(1);
        }
        i
    };
    let start = first_component(src, windows);
    let root = src.get(..start)?.iter();
    let mut out: Vec<u8> = root.map(|&c| if is_sep(c) { b'/' } else { c }).collect();
    let kept = out.len();
    let mut i = skip_seps(start);
    loop {
        if at(i) == b'.' {
            let up_to = match (at(i.saturating_add(1)), at(i.saturating_add(2))) {
                (0, _) => {
                    i = i.saturating_add(1);
                    None
                }
                (c, _) if is_sep(c) => {
                    i = skip_seps(i.saturating_add(2));
                    continue;
                }
                (b'.', 0) => Some(i.saturating_add(2)),
                (b'.', c) if is_sep(c) => Some(skip_seps(i.saturating_add(3))),
                _ => None,
            };
            if let Some(next) = up_to {
                // Back past the `/` that ends the part before, to the `/` before that part.
                let end = out.len().checked_sub(1).filter(|&end| end > kept)?;
                let part = out.get(kept..end)?.iter().rposition(|&c| c == b'/');
                out.truncate(part.map_or(kept, |at| kept.saturating_add(at).saturating_add(1)));
                i = next;
                continue;
            }
        }
        // A part, up to the next separator, then that run of separators as one `/`.
        loop {
            let c = at(i);
            i = i.saturating_add(1);
            if c == 0 {
                return Some(out);
            }
            if is_sep(c) {
                out.push(b'/');
                i = skip_seps(i);
                break;
            }
            out.push(c);
        }
    }
}

/// Git's `offset_1st_component`: how long the root of `path` is, which `tidy_path` keeps but for
/// its `\`, which becomes `/`: a leading `/`, and on Windows a drive (Git's `has_dos_drive_prefix`,
/// in compat/win32/path-utils.c) or the `//server/share/` of a network path.
#[deny(clippy::indexing_slicing, clippy::arithmetic_side_effects)]
fn first_component(path: &[u8], windows: bool) -> usize {
    let is_sep = |c: u8| c == b'/' || (windows && c == b'\\');
    let at = |i: usize| path.get(i).copied().unwrap_or(0);
    if !windows {
        return usize::from(is_sep(at(0)));
    }
    let drive = match at(0) {
        0 => 0,
        c if c & 0x80 == 0 => {
            if at(1) == b':' {
                2
            } else {
                0
            }
        }
        // A drive named by a letter beyond ASCII, of up to four bytes in UTF-8.
        _ => {
            let letter = (1..4).find(|&i| at(i) & 0x80 == 0).unwrap_or(4);
            if at(letter) == b':' {
                letter.saturating_add(1)
            } else {
                0
            }
        }
    };
    if drive == 0 && is_sep(at(0)) && is_sep(at(1)) {
        // Past the server's name, then the share's.
        let Some(server) = path.iter().skip(2).position(|&c| is_sep(c)) else {
            return 0;
        };
        let mut end = server.saturating_add(3);
        while at(end) != 0 && !is_sep(at(end)) {
            end = end.saturating_add(1);
        }
        return end.saturating_add(usize::from(is_sep(at(end))));
    }
    drive.saturating_add(usize::from(is_sep(at(drive))))
}
