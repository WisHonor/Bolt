//! A save of only the paths and patterns named (decision 054): the folder's version of what they
//! match, deletions included, and the base save's version of everything else.

use std::cmp::Reverse;
use std::collections::{HashMap, HashSet};
use std::ffi::OsStr;
use std::iter;

use super::walk::{Handles, kept_as_they_were, path_in, without_git_names};
use super::{Child, Folders, Kind, SaveError};
use crate::drive::Drive;
use crate::id::Id;
use crate::ignore::wildmatch;
use crate::listing::{Entry, EntryKind, Listing, SaveLimits};
use crate::names::Given;
use crate::store::{Store, StoreError};
use crate::writeout::within;

/// What a partial save takes, with names as the drive compares them, so that on a drive that
/// ignores letter case a path or pattern matches whatever the case.
pub(super) struct Scope {
    tops: Vec<Top>,
    drive: Drive,
}

enum Top {
    Path(String),
    Pattern { below: String, glob: Vec<u8> },
}

impl Scope {
    pub(super) fn new(given: &[Given], drive: Drive) -> Scope {
        let tops = given
            .iter()
            .map(|g| match g {
                Given::Path(path) => Top::Path(drive.compared(path)),
                Given::Pattern { below, glob } => Top::Pattern {
                    below: drive.compared(below),
                    glob: drive.compared(glob).into_bytes(),
                },
            })
            .collect();
        Scope { tops, drive }
    }

    /// Whether anything named takes the project path `path`; each that does is marked in
    /// `matched`.
    fn mark(&self, path: &str, matched: &mut [bool]) -> bool {
        let path = self.drive.compared(path);
        let mut any = false;
        for (top, seen) in self.tops.iter().zip(matched.iter_mut()) {
            if takes(top, &path) {
                *seen = true;
                any = true;
            }
        }
        any
    }

    fn holds(&self, path: &str) -> bool {
        let path = self.drive.compared(path);
        self.tops.iter().any(|top| takes(top, &path))
    }

    /// Whether something named can be below the folder `folder`.
    fn reaches_below(&self, folder: &str) -> bool {
        let folder = self.drive.compared(folder);
        self.tops.iter().any(|top| match top {
            Top::Path(path) => within(path, &folder),
            Top::Pattern { below, .. } => within(&folder, below) || within(below, &folder),
        })
    }
}

/// Whether `top` takes `path`: it is at or below a path, or a pattern matches it or a folder above
/// it, below the pattern's own folder.
fn takes(top: &Top, path: &str) -> bool {
    match top {
        Top::Path(top) => within(path, top),
        Top::Pattern { below, glob } => {
            let rest = if below.is_empty() {
                Some(path)
            } else {
                path.strip_prefix(below.as_str())
                    .and_then(|r| r.strip_prefix('/'))
            };
            let Some(rest) = rest.filter(|r| !r.is_empty()) else {
                return false;
            };
            rest.match_indices('/')
                .filter_map(|(at, _)| rest.get(..at))
                .chain(iter::once(rest))
                .any(|p| wildmatch(glob, p.as_bytes(), false))
        }
    }
}

/// An entry of the base save, at the project path `rel`, as `kind` keeps it.
fn base_child(e: &Entry, rel: String, kept: bool) -> Child {
    let (kind, id, size) = (e.kind, e.id, e.size);
    Child {
        name: e.name.clone(),
        kind: if kept {
            Kind::Kept { kind, id, size }
        } else {
            Kind::Left { kind, id, size }
        },
        rel,
        meta: None,
    }
}

/// Narrows what the walk found, `folders`, whose base save's listings are `old`, to what `scope`
/// takes, the base save's entries standing in for everything else. Folders gone from disk with
/// something named below them are read from `store` into `old`; `rebuilt` keeps only the
/// listings still used. Gives which of the named matched anything, in the folder or the base save.
pub(super) fn narrow(
    scope: &Scope,
    given: &[Given],
    store: &Store,
    folders: &mut Folders,
    old: &mut HashMap<String, Option<Listing>>,
    rebuilt: &mut Vec<Listing>,
) -> Result<Vec<bool>, SaveError> {
    let too_large = |detail| SaveError::from(StoreError::SaveTooLarge { detail });
    // The whole project always matches, even with nothing in it.
    let mut matched: Vec<bool> = given
        .iter()
        .map(|g| *g == Given::Path(String::new()))
        .collect();
    let mut limits = SaveLimits::default();
    let mut out = Folders::new();
    let mut gone = HashSet::new();
    let mut todo = vec![String::new()];
    while let Some(rel) = todo.pop() {
        let is_gone = gone.contains(&rel);
        let before = old.get(&rel).and_then(Option::as_ref);
        let mut kept = Vec::new();
        let mut met = HashSet::new();
        for c in folders.remove(&rel).unwrap_or_default() {
            let inside = scope.mark(&c.rel, &mut matched);
            // Unless named, a folder gone from disk stays as the base save has it, below.
            if matches!(c.kind, Kind::Back { .. }) && !inside {
                continue;
            }
            met.insert(c.name.clone());
            if inside || (c.kind == Kind::Folder && scope.reaches_below(&c.rel)) {
                if c.kind == Kind::Folder {
                    todo.push(c.rel.clone());
                }
                kept.push(c);
            } else if let Some(e) = before.and_then(|l| l.get(&c.name)) {
                kept.push(base_child(e, c.rel, false));
            }
        }
        // What the base save holds here and the folder doesn't.
        let mut read = Vec::new();
        if let Some(listing) = before {
            let held = if is_gone {
                kept_as_they_were(listing, scope.drive)
            } else {
                vec![false; listing.entries().len()]
            };
            for (e, held) in listing.entries().iter().zip(held) {
                if met.contains(&e.name) {
                    continue;
                }
                let path = path_in(&rel, &e.name);
                let inside = scope.mark(&path, &mut matched);
                // What this computer can't hold stays, as a full save keeps it.
                if held {
                    kept.push(base_child(e, path, inside));
                    continue;
                }
                let reaches = scope.reaches_below(&path);
                // The walk took care of what is named in a folder it found.
                let into = e.kind == EntryKind::Folder && reaches && (is_gone || !inside);
                if into {
                    limits
                        .check_depth(path.split('/').count())
                        .map_err(too_large)?;
                    let (listing, _) = without_git_names(store.listing(&e.id)?)?;
                    limits.add(listing.entries().len()).map_err(too_large)?;
                    read.push((path.clone(), listing));
                    kept.push(Child {
                        name: e.name.clone(),
                        kind: Kind::Folder,
                        rel: path,
                        meta: None,
                    });
                } else if !inside {
                    kept.push(base_child(e, path, false));
                }
            }
        }
        for (path, listing) in read {
            gone.insert(path.clone());
            old.insert(path.clone(), Some(listing));
            todo.push(path);
        }
        kept.sort_by(|a, b| a.name.cmp(&b.name));
        out.insert(rel, kept);
    }
    drop_empty(scope, &gone, old, &mut out);
    *folders = out;
    keep_used(folders, rebuilt);
    Ok(matched)
}

/// Removes, deepest first, the folders left empty that the new save shouldn't hold: those gone
/// from disk, and new ones nothing named takes.
fn drop_empty(
    scope: &Scope,
    gone: &HashSet<String>,
    old: &HashMap<String, Option<Listing>>,
    out: &mut Folders,
) {
    let mut order: Vec<String> = out.keys().filter(|r| !r.is_empty()).cloned().collect();
    order.sort_by_key(|rel| Reverse(rel.split('/').count()));
    for rel in order {
        let new = old.get(&rel).is_none_or(Option::is_none);
        let empty = out.get(&rel).is_some_and(Vec::is_empty);
        if empty && (gone.contains(&rel) || (new && !scope.holds(&rel))) {
            out.remove(&rel);
            let (up, name) = rel.rsplit_once('/').unwrap_or(("", &rel));
            if let Some(children) = out.get_mut(up) {
                children.retain(|c| c.name != name);
            }
        }
    }
}

/// Keeps of `rebuilt` only the listings the folders that came back still use.
fn keep_used(folders: &Folders, rebuilt: &mut Vec<Listing>) {
    let by_id: HashMap<Id, &Listing> = rebuilt.iter().map(|l| (l.id(), l)).collect();
    let mut used: HashSet<Id> = HashSet::new();
    let mut todo: Vec<Id> = folders
        .values()
        .flatten()
        .filter_map(|c| match c.kind {
            Kind::Back { id, .. } => Some(id),
            _ => None,
        })
        .collect();
    while let Some(id) = todo.pop() {
        if !used.insert(id) {
            continue;
        }
        let inner = by_id.get(&id).into_iter().flat_map(|l| l.entries());
        todo.extend(inner.filter(|e| e.kind == EntryKind::Folder).map(|e| e.id));
    }
    rebuilt.retain(|l| used.contains(&l.id()));
}

/// Refuses the save if something named matched nothing: a path on disk the walk left out is new
/// and ignored, and anything else names nothing.
pub(super) fn check_matched(
    given: &[Given],
    matched: &[bool],
    handles: &mut Handles,
) -> Result<(), SaveError> {
    let Some((g, _)) = given.iter().zip(matched).find(|(_, m)| !**m) else {
        return Ok(());
    };
    if let Given::Path(path) = g {
        let (up, name) = path.rsplit_once('/').unwrap_or(("", path));
        let on_disk = handles
            .folder(up)
            .is_ok_and(|dir| dir.metadata(OsStr::new(name)).is_ok());
        if on_disk {
            return Err(SaveError::LeftOut { path: path.clone() });
        }
    }
    Err(SaveError::NoMatch { given: g.shown() })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
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
        let src = Scope::new(&[Given::Path("src".into())], exact);
        assert!(src.holds("src") && src.holds("src/a/b.rs"));
        assert!(!src.holds("srcx") && !src.holds("Src/a.rs"));
        assert!(src.reaches_below("") && !src.reaches_below("lib"));
        assert!(Scope::new(&[Given::Path("src".into())], folding).holds("SRC/a.rs"));
        let cs = Scope::new(&[pattern("dir", "*.cs")], exact);
        assert!(cs.holds("dir/a.cs") && !cs.holds("dir/x/a.cs") && !cs.holds("a.cs"));
        assert!(!cs.holds("dir"));
        assert!(cs.reaches_below("") && cs.reaches_below("dir/x") && !cs.reaches_below("lib"));
        let deep = Scope::new(&[pattern("", "**/*.png")], folding);
        assert!(deep.holds("A.PNG") && deep.holds("x/y/a.png"));
        // A folder a pattern matches takes everything in it.
        let folder = Scope::new(&[pattern("", "s?b")], exact);
        assert!(folder.holds("sub/x/y.txt") && !folder.holds("sbb2/y.txt"));
    }
}
