//! Which project paths a command limited to some paths and patterns takes: everything at or below
//! a path, and what a pattern matches, with everything in a folder it matches (decision 054).

use std::iter;

use crate::drive::Drive;
use crate::ignore::wildmatch;
use crate::names::{Given, fold};

/// The paths and patterns a command was given, as the drive compares names: a path matches
/// whatever its case on a drive that ignores case, and a pattern as Git's `core.ignorecase`
/// matches, folding ASCII letters only.
pub(crate) struct Scope {
    tops: Vec<Top>,
    drive: Drive,
}

enum Top {
    Path(String),
    /// `glob` is matched below `below`, which is `names` names long.
    Pattern {
        below: String,
        names: usize,
        glob: Vec<u8>,
    },
}

impl Scope {
    pub(crate) fn new(given: &[Given], drive: Drive) -> Scope {
        let tops = given
            .iter()
            .map(|g| match g {
                Given::Path(path) => Top::Path(drive.compared(path)),
                Given::Pattern { below, glob } => Top::Pattern {
                    below: drive.compared(below),
                    names: if below.is_empty() {
                        0
                    } else {
                        below.split('/').count()
                    },
                    glob: fold(glob, drive.composes_unicode, false).into_bytes(),
                },
            })
            .collect();
        Scope { tops, drive }
    }

    /// Project paths only.
    pub(crate) fn paths(paths: &[String], drive: Drive) -> Scope {
        let given: Vec<Given> = paths.iter().cloned().map(Given::Path).collect();
        Scope::new(&given, drive)
    }

    pub(crate) fn drive(&self) -> Drive {
        self.drive
    }

    /// Whether anything given takes the project path `path`; each that does is marked in
    /// `matched`, one flag for each, in order.
    pub(crate) fn mark(&self, path: &str, matched: &mut [bool]) -> bool {
        let mut any = false;
        for (top, seen) in self.tops.iter().zip(matched.iter_mut()) {
            if self.takes(top, path) {
                *seen = true;
                any = true;
            }
        }
        any
    }

    pub(crate) fn holds(&self, path: &str) -> bool {
        self.tops.iter().any(|top| self.takes(top, path))
    }

    /// Whether something given can be below the folder `folder`.
    pub(crate) fn reaches_below(&self, folder: &str) -> bool {
        let folder = self.drive.compared(folder);
        self.tops.iter().any(|top| match top {
            Top::Path(path) => within(path, &folder),
            Top::Pattern { below, .. } => within(&folder, below) || within(below, &folder),
        })
    }

    /// Whether `top` takes `path`: it is at or below a path, or a pattern matches it or a folder
    /// above it, below the pattern's own folder.
    fn takes(&self, top: &Top, path: &str) -> bool {
        match top {
            Top::Path(top) => within(&self.drive.compared(path), top),
            Top::Pattern { below, names, glob } => {
                if !within(&self.drive.compared(path), below) {
                    return false;
                }
                // Letter case is left to wildmatch, which folds it as Git does.
                let path = fold(path, self.drive.composes_unicode, false);
                let rest = match names.checked_sub(1) {
                    None => Some(path.as_str()),
                    Some(skip) => path
                        .match_indices('/')
                        .nth(skip)
                        .and_then(|(at, _)| path.get(at + 1..)),
                };
                let Some(rest) = rest.filter(|r| !r.is_empty()) else {
                    return false;
                };
                ancestors(rest)
                    .chain(iter::once(rest))
                    .any(|p| wildmatch(glob, p.as_bytes(), self.drive.ignores_case))
            }
        }
    }
}

/// Whether `path` is `top` or below it; `""` is the whole project.
pub(crate) fn within(path: &str, top: &str) -> bool {
    top.is_empty()
        || path == top
        || path
            .strip_prefix(top)
            .is_some_and(|rest| rest.starts_with('/'))
}

/// The folders above `path`, nearest last: `a`, `a/b` for `a/b/c`.
pub(crate) fn ancestors(path: &str) -> impl Iterator<Item = &str> {
    path.match_indices('/').filter_map(|(at, _)| path.get(..at))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_a_scope_takes() {
        let exact = Drive {
            ignores_case: false,
            composes_unicode: false,
        };
        let folding = Drive {
            ignores_case: true,
            composes_unicode: false,
        };
        let pattern = |below: &str, glob: &str| Given::Pattern {
            below: below.into(),
            glob: glob.into(),
        };
        let src = Scope::paths(&["src".into()], exact);
        assert!(src.holds("src") && src.holds("src/a/b.rs"));
        assert!(!src.holds("srcx") && !src.holds("Src/a.rs"));
        assert!(src.reaches_below("") && !src.reaches_below("lib"));
        assert!(Scope::paths(&["src".into()], folding).holds("SRC/a.rs"));
        assert!(Scope::paths(&[String::new()], exact).holds("any/thing"));
        let cs = Scope::new(&[pattern("dir", "*.cs")], exact);
        assert!(cs.holds("dir/a.cs") && !cs.holds("dir/x/a.cs") && !cs.holds("a.cs"));
        assert!(!cs.holds("dir"));
        assert!(cs.reaches_below("") && cs.reaches_below("dir/x") && !cs.reaches_below("lib"));
        assert!(Scope::new(&[pattern("Dir", "*.CS")], folding).holds("dir/A.cs"));
        let deep = Scope::new(&[pattern("", "**/*.png")], folding);
        assert!(deep.holds("A.PNG") && deep.holds("x/y/a.png"));
        // A folder a pattern matches takes everything in it.
        let folder = Scope::new(&[pattern("", "s?b")], exact);
        assert!(folder.holds("sub/x/y.txt") && !folder.holds("sbb2/y.txt"));
        // As Git folds case, a class of capitals takes small letters too.
        let upper = Scope::new(&[pattern("", "[[:upper:]]*.txt")], folding);
        assert!(upper.holds("A.txt") && upper.holds("a.TXT"));
        assert!(!Scope::new(&[pattern("", "[[:upper:]]*")], exact).holds("a"));
        // A literal `[` is named with a class holding it.
        let literal = Scope::new(&[pattern("", "a[[]1]")], exact);
        assert!(literal.holds("a[1]/x.txt") && !literal.holds("a1"));
    }

    #[test]
    fn within_and_ancestors() {
        assert!(within("a/b", "a") && within("a", "a") && within("a", ""));
        assert!(!within("ab", "a"));
        assert_eq!(ancestors("a/b/c").collect::<Vec<_>>(), ["a", "a/b"]);
    }
}
