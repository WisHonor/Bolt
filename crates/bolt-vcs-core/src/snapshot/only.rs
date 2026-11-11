//! A save of only the paths and patterns named (decision 054): the folder's version of what they
//! match, deletions included, and the base save's version of everything else.

use std::cmp::Reverse;
use std::collections::{HashMap, HashSet};
use std::ffi::OsStr;
use std::iter;

use super::walk::{Handles, holds_own_repository, kept_as_they_were, path_in, without_git_names};
use super::{Child, Folders, Kind, SaveError};
use crate::id::Id;
use crate::listing::{Entry, EntryKind, Listing, SaveLimits};
use crate::names::{Given, is_bolt_alias, is_git_alias};
use crate::scope::{Scope, ancestors, within};
use crate::store::{Store, StoreError};

/// What `narrow` found: which of the named matched anything, in the folder or the base save, and
/// the entries that aren't folders but that something named reaches below.
pub(super) struct Narrowed {
    pub(super) matched: Vec<bool>,
    blocked: Vec<Blocked>,
}

/// An entry something named reaches below that a save can't go into.
struct Blocked {
    path: String,
    /// What it is on disk, or `None` where every save keeps it as it was.
    what: Option<&'static str>,
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
/// listings still used. The gone folders read count against `limits`.
pub(super) fn narrow(
    scope: &Scope,
    given: &[Given],
    store: &Store,
    folders: &mut Folders,
    old: &mut HashMap<String, Option<Listing>>,
    rebuilt: &mut Vec<Listing>,
    mut limits: SaveLimits,
) -> Result<Narrowed, SaveError> {
    let too_large = |detail| SaveError::from(StoreError::SaveTooLarge { detail });
    // The whole project always matches, even with nothing in it.
    let mut matched: Vec<bool> = given
        .iter()
        .map(|g| *g == Given::Path(String::new()))
        .collect();
    let mut out = Folders::new();
    let mut gone = HashSet::new();
    // Those gone folders whose listings were counted whole already, with all they hold.
    let mut whole = HashSet::new();
    let mut blocked = Vec::new();
    let mut todo = vec![String::new()];
    while let Some(rel) = todo.pop() {
        let is_gone = gone.contains(&rel);
        let in_whole = whole.contains(&rel);
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
            let what = match c.kind {
                Kind::File => Some(Some("a file")),
                Kind::Symlink => Some(Some("a link")),
                Kind::Kept { .. } => Some(None),
                _ => None,
            };
            if let Some(what) = what
                && !inside
                && scope.reaches_below(&c.rel)
            {
                let path = c.rel.clone();
                blocked.push(Blocked { path, what });
            }
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
                kept_as_they_were(listing, scope.drive())
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
                    if !inside && scope.reaches_below(&path) {
                        let path = path.clone();
                        blocked.push(Blocked { path, what: None });
                    }
                    kept.push(base_child(e, path, inside));
                    continue;
                }
                let reaches = scope.reaches_below(&path);
                // The walk took care of what is named in a folder it found.
                let into = e.kind == EntryKind::Folder && reaches && (is_gone || !inside);
                if into {
                    let depth = path.split('/').count();
                    limits.check_depth(depth).map_err(too_large)?;
                    // Met as it is read, a folder met again is counted whole before it is walked.
                    let again = in_whole
                        || limits
                            .enter(e.id, depth, path.len(), |id| store.listing(id))?
                            .map_err(too_large)?;
                    let (listing, _) = without_git_names(store.listing(&e.id)?)?;
                    if again {
                        whole.insert(path.clone());
                    } else {
                        limits.add(listing.entries().len()).map_err(too_large)?;
                    }
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
    Ok(Narrowed { matched, blocked })
}

/// Removes, deepest first, the folders left empty that the new save shouldn't hold: those gone
/// from disk that are named or whose last named entries went, and new ones nothing named takes.
fn drop_empty(
    scope: &Scope,
    gone: &HashSet<String>,
    old: &HashMap<String, Option<Listing>>,
    out: &mut Folders,
) {
    let mut order: Vec<String> = out.keys().filter(|r| !r.is_empty()).cloned().collect();
    order.sort_by_key(|rel| Reverse(rel.split('/').count()));
    for rel in order {
        let saved = old.get(&rel).and_then(Option::as_ref);
        let emptied = saved.is_some_and(|l| !l.entries().is_empty());
        let named = scope.holds(&rel);
        let drop = if gone.contains(&rel) {
            named || emptied
        } else {
            saved.is_none() && !named
        };
        if drop && out.get(&rel).is_some_and(Vec::is_empty) {
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

/// Refuses the save if something named matched nothing, saying why: it is below an entry the save
/// can't go into, the walk left it out (a name Git or Bolt keeps for itself, a repository or
/// project of its own, an ignore rule), or it names nothing.
pub(super) fn check_matched(
    scope: &Scope,
    given: &[Given],
    narrowed: &Narrowed,
    handles: &mut Handles,
) -> Result<(), SaveError> {
    let unmatched = given.iter().zip(&narrowed.matched).find(|(_, m)| !**m);
    let Some((g, _)) = unmatched else {
        return Ok(());
    };
    let Given::Path(path) = g else {
        return Err(SaveError::NoMatch { given: g.shown() });
    };
    let drive = scope.drive();
    let named = drive.compared(path);
    let below = |b: &&Blocked| within(&named, &drive.compared(&b.path));
    if let Some(b) = narrowed.blocked.iter().find(below) {
        let path = b.path.clone();
        return Err(match b.what {
            Some(what) => SaveError::NotAFolder { path, what },
            None => SaveError::KeptWhole { path },
        });
    }
    for at in ancestors(path).chain(iter::once(path.as_str())) {
        let name = at.rsplit_once('/').map_or(at, |(_, name)| name);
        if is_git_alias(name) || is_bolt_alias(name) {
            return Err(SaveError::NeverSaved {
                path: at.to_owned(),
            });
        }
        match handles.folder(at) {
            Ok(dir) if holds_own_repository(dir) => {
                return Err(SaveError::NestedProject {
                    path: path.clone(),
                    at: at.to_owned(),
                });
            }
            Ok(_) => {}
            Err(_) => break,
        }
    }
    let (up, name) = path.rsplit_once('/').unwrap_or(("", path));
    let on_disk = handles
        .folder(up)
        .is_ok_and(|dir| dir.metadata(OsStr::new(name)).is_ok());
    if on_disk {
        return Err(SaveError::LeftOut { path: path.clone() });
    }
    Err(SaveError::NoMatch { given: g.shown() })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;
    use crate::listing::MAX_SAVE_DEPTH;
    use crate::snapshot::tests::{project, this_drive, walk_project, write};
    use crate::test_support::{add_listing, plant_root};

    fn entry(name: &str, kind: EntryKind, id: Id) -> Entry {
        Entry {
            name: name.into(),
            kind,
            id,
            size: 0,
        }
    }

    #[test]
    fn gone_folders_with_the_same_contents_count_every_path() {
        let (dir, mut repo) = project();
        // `a` and `b`, gone from disk, are one listing holding `x.txt` and the folder `in`.
        plant_root(&mut repo, |packs| {
            let empty = packs.add_content(b"").unwrap();
            let inner = add_listing(packs, vec![entry("y.txt", EntryKind::File, empty)]).0;
            let both = vec![
                entry("in", EntryKind::Folder, inner),
                entry("x.txt", EntryKind::File, empty),
            ];
            let shared = add_listing(packs, both).0;
            let top = ["a", "b"].map(|n| entry(n, EntryKind::Folder, shared));
            add_listing(packs, top.to_vec()).0
        });
        write(dir.path(), "new.txt", b"x");
        // A pattern that reaches into both and matches nothing there: they stay as they were.
        let given = [Given::Pattern {
            below: String::new(),
            glob: "**/z.txt".into(),
        }];
        let narrowed = |entries| {
            let scan = walk_project(&repo, SaveLimits::default()).unwrap();
            let (mut folders, mut old, mut rebuilt) = (scan.folders, scan.old, scan.rebuilt);
            let scope = Scope::new(&given, this_drive());
            let limits = SaveLimits::new(MAX_SAVE_DEPTH, entries);
            let store = repo.store();
            narrow(
                &scope,
                &given,
                store,
                &mut folders,
                &mut old,
                &mut rebuilt,
                limits,
            )
            .map(|_| folders)
        };
        // Each folder read holds 2 entries, and its `in` 1: 2 * (2 + 1).
        const PATHS: u64 = 2 * (2 + 1);
        let folders = narrowed(PATHS).unwrap();
        let names = |rel: &str| -> Vec<&str> {
            let children = folders.get(rel).map(Vec::as_slice).unwrap_or_default();
            children.iter().map(|c| c.name.as_str()).collect()
        };
        for top in ["a", "b"] {
            assert_eq!(names(top), ["in", "x.txt"], "{top}");
            assert_eq!(names(&format!("{top}/in")), ["y.txt"], "{top}");
        }
        let Err(err) = narrowed(PATHS - 1) else {
            panic!("{PATHS} paths were read under a limit of {}", PATHS - 1);
        };
        let said = err.to_string();
        assert!(
            said.contains("more than 5 files and folders in total"),
            "{said}"
        );
    }
}
