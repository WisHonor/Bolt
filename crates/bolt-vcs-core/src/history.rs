//! Line history, short change IDs, and finding a save from what the user typed.

use std::collections::HashMap;

use crate::diary::Position;
use crate::id::Id;
use crate::repo::{RepoError, Repository};
use crate::save::Save;

#[derive(Debug, thiserror::Error)]
pub enum ResolveError {
    #[error("No save, line or ID matches \"{0}\".")]
    NotFound(String),
    #[error("\"{text}\" matches more than one save: {}.", candidates.join(", "))]
    Ambiguous {
        text: String,
        candidates: Vec<String>,
    },
    #[error(transparent)]
    Repo(#[from] RepoError),
}

impl From<crate::store::StoreError> for ResolveError {
    fn from(e: crate::store::StoreError) -> Self {
        RepoError::from(e).into()
    }
}

/// Saves from `start` back along first parents, newest first. A cycle can't be built (a
/// save's fingerprint covers its parents), but a store missing a parent fails with an error.
fn walk(repo: &Repository, start: Id) -> Result<Vec<(Id, Save)>, RepoError> {
    let mut out = Vec::new();
    let mut next = Some(start);
    while let Some(id) = next {
        let save = repo.store().save(&id, repo.signatures_allowed())?;
        next = save.parents.first().copied();
        out.push((id, save));
    }
    Ok(out)
}

/// The current line's saves (or the checked-out save's ancestors), newest first.
///
/// # Errors
/// Fails if the repository can't be read.
pub fn line_history(repo: &Repository) -> Result<Vec<(Id, Save)>, RepoError> {
    let (_, entry) = repo.current()?;
    let start = match &entry.state.position {
        Position::OnLine(name) => {
            *entry
                .state
                .lines
                .get(name)
                .ok_or_else(|| RepoError::Damaged {
                    detail: format!("the current line {name} does not exist"),
                })?
        }
        Position::AtSave(id) => *id,
    };
    walk(repo, start)
}

/// The shortest unique prefix of each save's change ID, at least 4 letters.
#[must_use]
pub fn short_change_ids(saves: &[(Id, Save)]) -> HashMap<Id, String> {
    let letters: Vec<String> = saves
        .iter()
        .map(|(_, s)| s.change_id.to_letters())
        .collect();
    saves
        .iter()
        .zip(&letters)
        .map(|((id, _), mine)| {
            let len = (4..mine.len())
                .find(|&n| letters.iter().filter(|o| o.starts_with(&mine[..n])).count() == 1)
                .unwrap_or(mine.len());
            (*id, mine[..len].to_owned())
        })
        .collect()
}

fn one_of(text: &str, mut found: Vec<(Id, String)>) -> Result<Id, ResolveError> {
    found.sort_by(|a, b| a.1.cmp(&b.1));
    found.dedup();
    match found.as_slice() {
        [] => Err(ResolveError::NotFound(text.to_owned())),
        [(id, _)] => Ok(*id),
        _ => Err(ResolveError::Ambiguous {
            text: text.to_owned(),
            candidates: found.into_iter().map(|(_, c)| c).collect(),
        }),
    }
}

fn by_change_prefix(saves: &[(Id, Save)], text: &str) -> Result<Id, ResolveError> {
    let found = saves
        .iter()
        .map(|(id, s)| (*id, s.change_id.to_letters()))
        .filter(|(_, l)| l.starts_with(text))
        .collect();
    one_of(text, found)
}

/// Finds a save from text: a line name, else a change-ID prefix (letters `k`-`z`, unique on
/// the current line), else at least 8 hex digits of a fingerprint.
///
/// # Errors
/// `NotFound`, `Ambiguous`, or a repository problem.
pub fn resolve(repo: &Repository, text: &str) -> Result<Id, ResolveError> {
    let (_, entry) = repo.current()?;
    if let Some(id) = entry.state.lines.get(text) {
        return Ok(*id);
    }
    if !text.is_empty() && text.bytes().all(|b| (b'k'..=b'z').contains(&b)) {
        return by_change_prefix(&line_history(repo)?, text);
    }
    if text.len() >= 8 && text.bytes().all(|b| b.is_ascii_hexdigit()) {
        let text = text.to_ascii_lowercase();
        let mut found = Vec::new();
        for tip in entry.state.lines.values() {
            for (id, _) in walk(repo, *tip)? {
                if id.to_hex().starts_with(&text) {
                    found.push((id, id.to_hex()));
                }
            }
        }
        return one_of(&text, found);
    }
    Err(ResolveError::NotFound(text.to_owned()))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;
    use crate::id::ChangeId;
    use crate::save::Person;

    fn saved(letters: &str, tag: u8) -> (Id, Save) {
        let who = Person {
            name: "Ada".into(),
            email: "a@example.com".into(),
            time_ms: 0,
            tz_offset_min: 0,
        };
        let save = Save {
            root: Id([tag; 32]),
            parents: vec![],
            author: who.clone(),
            saver: who,
            message: "m".into(),
            change_id: ChangeId::from_letters(letters).unwrap(),
            move_notes: vec![],
            signature: vec![],
            extra: Default::default(),
        };
        (save.id(), save)
    }

    #[test]
    fn ambiguous_prefix_lists_candidates() {
        let a = saved(&format!("klmn{}", "k".repeat(28)), 1);
        let b = saved(&format!("klmo{}", "k".repeat(28)), 2);
        let saves = vec![a.clone(), b.clone()];
        match by_change_prefix(&saves, "klm") {
            Err(ResolveError::Ambiguous { candidates, .. }) => {
                assert_eq!(candidates.len(), 2, "{candidates:?}");
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(by_change_prefix(&saves, "klmo").unwrap(), b.0);
        assert!(matches!(
            by_change_prefix(&saves, "zz"),
            Err(ResolveError::NotFound(_))
        ));
        let shorts = short_change_ids(&saves);
        assert_eq!(shorts[&a.0], "klmn");
        assert_eq!(shorts[&b.0], "klmo");
    }
}
