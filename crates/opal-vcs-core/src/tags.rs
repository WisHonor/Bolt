//! Tags made, moved and deleted, and the tags at each save (decision 159).

use std::collections::HashMap;

use crate::diary::{
    Action, DiaryEntry, MAX_TAG_MESSAGE_BYTES, MAX_TAGS, State, Tag, TagList, fold_line_name,
    validate_line_name,
};
use crate::history::tip_of;
use crate::id::Id;
use crate::lines::{name_reason, step, told};
use crate::pack::ObjectKind;
use crate::repo::{ChangeLock, RepoError, Repository};
use crate::save::Person;
use crate::text::escape;
use crate::warning::Warning;

/// What `tag` and `delete_tag` ask before they change a tag that exists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ask<'a> {
    Move { name: &'a str, from: Id, to: Id },
    Delete { name: &'a str, save: Id },
}

/// What `tag` did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tagged {
    Made {
        save: Id,
    },
    /// The tag was at that save already, and no step was made.
    Already {
        save: Id,
    },
    Moved {
        from: Id,
        to: Id,
    },
}

/// Why a tag wasn't made, moved or deleted.
#[derive(Debug, thiserror::Error)]
pub enum TagError {
    // `reason` is Opalro's own wording.
    #[error("\"{}\" can't be a tag's name: {reason}.", escape(name, false))]
    BadName { name: String, reason: String },
    #[error(
        "There is already a tag named \"{}\", and two tags can't differ only by letter case.",
        escape(taken, false)
    )]
    Alike { name: String, taken: String },
    #[error(
        "\"{}\" is a line's name already, and a tag can't share it.",
        escape(name, false)
    )]
    Line { name: String },
    #[error("There is no tag named \"{}\".", escape(.0, false))]
    NoSuchTag(String),
    #[error("A tag's message can be at most 64 KiB.")]
    MessageTooLong,
    #[error("This project already has 100,000 tags, the most it can hold.")]
    TooMany,
    #[error("Tag \"{}\" wasn't moved.", escape(name, false))]
    NotMoved { name: String, from: Id, to: Id },
    #[error("Tag \"{}\" wasn't deleted.", escape(name, false))]
    NotDeleted { name: String },
    #[error(transparent)]
    Repo(#[from] RepoError),
}

impl TagError {
    /// What the user can do next, if there is something to say.
    #[must_use]
    pub fn hint(&self) -> Option<String> {
        match self {
            TagError::BadName { .. } => {
                Some("A tag's name takes letters, digits, - _ . and /.".to_owned())
            }
            TagError::Alike { .. } | TagError::Line { .. } => {
                Some("Choose another name.".to_owned())
            }
            TagError::NoSuchTag(_) => Some("See the tags with: opalro tag".to_owned()),
            TagError::MessageTooLong
            | TagError::TooMany
            | TagError::NotMoved { .. }
            | TagError::NotDeleted { .. } => None,
            TagError::Repo(e) => e.hint(),
        }
    }
}

/// Tags `save`, or the save the project is on. `confirm` is asked before a tag moves.
pub fn tag(
    repo: &mut Repository,
    name: &str,
    message: Option<&str>,
    save: Option<Id>,
    who: &Person,
    on_wait: &mut dyn FnMut(&str),
    confirm: &mut dyn FnMut(&Ask<'_>) -> bool,
) -> Result<(Tagged, Vec<Warning>), TagError> {
    step(repo, "tag", on_wait, |repo, lock, earlier| {
        if message.is_some_and(|m| m.len() > MAX_TAG_MESSAGE_BYTES) {
            return Err(TagError::MessageTooLong);
        }
        let (head, entry) = repo.current()?;
        let save = match save {
            Some(save) => {
                repo.store()
                    .save(&save, repo.signatures_allowed())
                    .map_err(RepoError::from)?;
                save
            }
            None => tip_of(&entry)?,
        };
        let mut list = repo.tag_list(&entry.state)?;
        let tagged = match list.tags.get_mut(name) {
            Some(tag) if tag.save == save => {
                return Ok((Tagged::Already { save }, told(earlier)));
            }
            Some(tag) => {
                let from = tag.save;
                if !confirm(&Ask::Move {
                    name,
                    from,
                    to: save,
                }) {
                    return Err(TagError::NotMoved {
                        name: name.to_owned(),
                        from,
                        to: save,
                    });
                }
                tag.save = save;
                if let Some(message) = message {
                    message.clone_into(&mut tag.message);
                }
                // They describe the old place.
                tag.git_tags.clear();
                Tagged::Moved { from, to: save }
            }
            None => {
                check_new_name(&list, &entry.state, name)?;
                if list.tags.len() as u64 >= MAX_TAGS {
                    return Err(TagError::TooMany);
                }
                let tag = Tag {
                    save,
                    message: message.unwrap_or_default().to_owned(),
                    git_tags: vec![],
                };
                list.tags.insert(name.to_owned(), tag);
                Tagged::Made { save }
            }
        };
        record(repo, lock, head, entry.state, &list, who)?;
        Ok((tagged, told(earlier)))
    })
}

/// Deletes a tag, once `confirm` says yes. Gives the save it was at.
pub fn delete_tag(
    repo: &mut Repository,
    name: &str,
    who: &Person,
    on_wait: &mut dyn FnMut(&str),
    confirm: &mut dyn FnMut(&Ask<'_>) -> bool,
) -> Result<(Id, Vec<Warning>), TagError> {
    step(repo, "tag", on_wait, |repo, lock, earlier| {
        let (head, entry) = repo.current()?;
        let mut list = repo.tag_list(&entry.state)?;
        let Some(tag) = list.tags.remove(name) else {
            return Err(TagError::NoSuchTag(name.to_owned()));
        };
        if !confirm(&Ask::Delete {
            name,
            save: tag.save,
        }) {
            return Err(TagError::NotDeleted {
                name: name.to_owned(),
            });
        }
        record(repo, lock, head, entry.state, &list, who)?;
        Ok((tag.save, told(earlier)))
    })
}

/// The tags at each save, names in order, for `history` and `show`.
pub fn by_save(repo: &Repository, state: &State) -> Result<HashMap<Id, Vec<String>>, RepoError> {
    let mut at: HashMap<Id, Vec<String>> = HashMap::new();
    for (name, tag) in repo.tag_list(state)?.tags {
        at.entry(tag.save).or_default().push(name);
    }
    Ok(at)
}

/// The one check of a new tag's name. Names a tag already has, imported ones too, aren't checked.
fn check_new_name(list: &TagList, state: &State, name: &str) -> Result<(), TagError> {
    validate_line_name(name).map_err(|e| TagError::BadName {
        name: name.to_owned(),
        reason: name_reason(e, "a tag's name"),
    })?;
    // It would read as an option. Only new names: a stored one may still start with `-`.
    if name.starts_with('-') {
        return Err(TagError::BadName {
            name: name.to_owned(),
            reason: "a tag's name cannot start with -".into(),
        });
    }
    let folded = fold_line_name(name);
    if let Some(taken) = list.tags.keys().find(|t| fold_line_name(t) == folded) {
        return Err(TagError::Alike {
            name: name.to_owned(),
            taken: taken.clone(),
        });
    }
    if state.lines.contains_key(name) {
        return Err(TagError::Line {
            name: name.to_owned(),
        });
    }
    Ok(())
}

/// Records a step to `state` with the tag list `list`, which changes no file.
fn record(
    repo: &mut Repository,
    lock: &ChangeLock,
    head: Id,
    mut state: State,
    list: &TagList,
    who: &Person,
) -> Result<(), RepoError> {
    let mut packs = repo.new_packs()?;
    // A stored list is never empty.
    state.tags = if list.tags.is_empty() {
        None
    } else {
        let encoded = list.encode();
        TagList::check(&encoded)?;
        Some(packs.add_record(ObjectKind::TagList, &encoded)?)
    };
    let entry = DiaryEntry {
        parent: Some(head),
        action: Action::Command,
        command: "tag".to_owned(),
        who: who.clone(),
        opal_version: env!("CARGO_PKG_VERSION").into(),
        state,
        folder_change: None,
    };
    repo.commit(lock, packs, &entry)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;
    use crate::id::ChangeId;
    use crate::save::Save;
    use crate::undo::{StepRequest, undo};
    use std::collections::BTreeMap;

    fn who() -> Person {
        Person {
            name: "Ada".into(),
            email: "ada@example.com".into(),
            time_ms: 1_700_000_000_000,
            tz_offset_min: 0,
        }
    }

    /// A new project, on `main` at its first save, which this gives.
    fn project() -> (tempfile::TempDir, Repository, Id) {
        let dir = tempfile::tempdir().unwrap();
        let repo = Repository::init(dir.path(), &who()).unwrap();
        let start = repo.current().unwrap().1.state.lines["main"];
        (dir, repo, start)
    }

    /// A new project whose `main` is one save past its first, which is given first.
    fn two_saves() -> (tempfile::TempDir, Repository, Id, Id) {
        let (dir, mut repo, start) = project();
        let lock = repo.lock_for_change("save", &mut |_| {}).unwrap();
        let (head, mut entry) = repo.current().unwrap();
        let first = repo.store().save(&start, false).unwrap();
        let save = Save {
            parents: vec![start],
            message: "second".into(),
            change_id: ChangeId::random().unwrap(),
            ..first
        };
        let mut packs = repo.new_packs().unwrap();
        let two = packs.add_record(ObjectKind::Save, &save.encode()).unwrap();
        entry.parent = Some(head);
        entry.command = "save".into();
        entry.state.lines.insert("main".into(), two);
        repo.commit(&lock, packs, &entry).unwrap();
        (dir, repo, start, two)
    }

    /// Commits a step whose tag list is `tags`, as an import would leave it.
    fn plant(repo: &mut Repository, tags: &TagList) {
        let lock = repo.lock_for_change("test", &mut |_| {}).unwrap();
        let (head, entry) = repo.current().unwrap();
        let mut packs = repo.new_packs().unwrap();
        let id = packs
            .add_record(ObjectKind::TagList, &tags.encode())
            .unwrap();
        let entry = DiaryEntry {
            parent: Some(head),
            action: Action::Command,
            command: "test".into(),
            folder_change: None,
            state: State {
                tags: Some(id),
                ..entry.state
            },
            ..entry
        };
        repo.commit(&lock, packs, &entry).unwrap();
    }

    fn one(name: &str, save: Id, message: &str, git_tags: Vec<Vec<u8>>) -> TagList {
        TagList {
            tags: BTreeMap::from([(
                name.to_owned(),
                Tag {
                    save,
                    message: message.into(),
                    git_tags,
                },
            )]),
        }
    }

    fn tags(repo: &Repository) -> BTreeMap<String, Tag> {
        let state = repo.current().unwrap().1.state;
        repo.tag_list(&state).unwrap().tags
    }

    /// What `confirm` was asked: the tag, where it is, and where it would go.
    type Asked = Vec<(String, Id, Option<Id>)>;

    fn note(asked: &mut Asked, ask: &Ask<'_>) {
        asked.push(match ask {
            Ask::Move { name, from, to } => ((*name).to_owned(), *from, Some(*to)),
            Ask::Delete { name, save } => ((*name).to_owned(), *save, None),
        });
    }

    fn tagged(
        repo: &mut Repository,
        name: &str,
        message: Option<&str>,
        save: Option<Id>,
        answer: bool,
    ) -> (Result<Tagged, TagError>, Asked) {
        let mut asked = Vec::new();
        let done = tag(repo, name, message, save, &who(), &mut |_| {}, &mut |a| {
            note(&mut asked, a);
            answer
        });
        (done.map(|(t, _)| t), asked)
    }

    fn deleted(repo: &mut Repository, name: &str, answer: bool) -> (Result<Id, TagError>, Asked) {
        let mut asked = Vec::new();
        let done = delete_tag(repo, name, &who(), &mut |_| {}, &mut |a| {
            note(&mut asked, a);
            answer
        });
        (done.map(|(id, _)| id), asked)
    }

    #[test]
    fn a_tag_names_the_save_the_project_is_on() {
        let (_dir, mut repo, s) = project();
        let (done, asked) = tagged(&mut repo, "v1.0", Some("First playable"), None, true);
        assert_eq!(done.unwrap(), Tagged::Made { save: s });
        assert!(asked.is_empty());
        assert_eq!(
            tags(&repo)["v1.0"],
            one("v1.0", s, "First playable", vec![]).tags["v1.0"]
        );
        let (_, entry) = repo.current().unwrap();
        assert_eq!(entry.command, "tag");
        assert_eq!(entry.folder_change, None);

        let (done, _) = tagged(&mut repo, "plain", None, None, true);
        assert_eq!(done.unwrap(), Tagged::Made { save: s });
        assert_eq!(tags(&repo)["plain"].message, "");
    }

    #[test]
    fn a_tag_can_name_another_save() {
        let (_dir, mut repo, start, two) = two_saves();
        let (done, _) = tagged(&mut repo, "old", None, Some(start), true);
        assert_eq!(done.unwrap(), Tagged::Made { save: start });
        let (done, _) = tagged(&mut repo, "new", None, None, true);
        assert_eq!(done.unwrap(), Tagged::Made { save: two });
        assert_eq!(tags(&repo)["old"].save, start);

        let head = repo.head().unwrap();
        let (missing, _) = tagged(&mut repo, "gone", None, Some(Id([7; 32])), true);
        assert!(matches!(missing, Err(TagError::Repo(_))), "{missing:?}");
        assert_eq!(repo.head().unwrap(), head);
    }

    #[test]
    fn a_new_tags_name_follows_the_line_rules() {
        let (_dir, mut repo, _) = project();
        tagged(&mut repo, "v1.0", None, None, true).0.unwrap();
        let head = repo.head().unwrap();

        let bad = tagged(&mut repo, "v 1", None, None, true).0.unwrap_err();
        assert!(matches!(bad, TagError::BadName { .. }), "{bad:?}");
        assert_eq!(
            bad.to_string(),
            "\"v 1\" can't be a tag's name: a tag's name can only hold letters, digits, -, _, . \
             and /."
        );
        assert_eq!(
            bad.hint().as_deref(),
            Some("A tag's name takes letters, digits, - _ . and /.")
        );

        let alike = tagged(&mut repo, "V1.0", None, None, true).0.unwrap_err();
        assert!(
            matches!(&alike, TagError::Alike { taken, .. } if taken == "v1.0"),
            "{alike:?}"
        );
        assert_eq!(
            alike.to_string(),
            "There is already a tag named \"v1.0\", and two tags can't differ only by letter case."
        );
        assert_eq!(alike.hint().as_deref(), Some("Choose another name."));

        let line = tagged(&mut repo, "main", None, None, true).0.unwrap_err();
        assert!(matches!(line, TagError::Line { .. }), "{line:?}");
        assert_eq!(
            line.to_string(),
            "\"main\" is a line's name already, and a tag can't share it."
        );
        assert_eq!(line.hint().as_deref(), Some("Choose another name."));
        assert_eq!(repo.head().unwrap(), head);
    }

    #[test]
    fn a_new_tags_name_cant_start_with_a_dash_but_a_stored_one_still_works() {
        let (_dir, mut repo, start, two) = two_saves();
        let head = repo.head().unwrap();
        for name in ["-", "-x", "--yes"] {
            let bad = tagged(&mut repo, name, None, None, true).0.unwrap_err();
            assert!(
                matches!(&bad, TagError::BadName { reason, .. }
                    if reason == "a tag's name cannot start with -"),
                "{bad:?}"
            );
        }
        assert_eq!(repo.head().unwrap(), head);
        // One an import brought in still moves and deletes.
        plant(&mut repo, &one("-old", start, "", vec![]));
        let (done, _) = tagged(&mut repo, "-old", None, Some(two), true);
        assert_eq!(
            done.unwrap(),
            Tagged::Moved {
                from: start,
                to: two
            }
        );
        deleted(&mut repo, "-old", true).0.unwrap();
        assert!(tags(&repo).is_empty());
    }

    #[test]
    fn tagging_the_same_save_again_is_nothing_to_do() {
        let (_dir, mut repo, s) = project();
        tagged(&mut repo, "v1", Some("First"), None, true)
            .0
            .unwrap();
        let head = repo.head().unwrap();
        let (done, asked) = tagged(&mut repo, "v1", Some("Another"), Some(s), true);
        assert_eq!(done.unwrap(), Tagged::Already { save: s });
        assert!(asked.is_empty());
        assert_eq!(repo.head().unwrap(), head);
        assert_eq!(tags(&repo)["v1"].message, "First");
    }

    #[test]
    fn moving_a_tag_asks_and_keeps_its_message() {
        let (_dir, mut repo, start, two) = two_saves();
        plant(
            &mut repo,
            &one("v1", start, "First playable", vec![b"object".to_vec()]),
        );
        let (done, asked) = tagged(&mut repo, "v1", None, None, true);
        assert_eq!(asked, [("v1".to_owned(), start, Some(two))]);
        assert_eq!(
            done.unwrap(),
            Tagged::Moved {
                from: start,
                to: two
            }
        );
        let moved = &tags(&repo)["v1"];
        assert_eq!(moved.save, two);
        assert_eq!(moved.message, "First playable");
        assert!(moved.git_tags.is_empty());
        assert_eq!(repo.current().unwrap().1.command, "tag");

        let (done, _) = tagged(&mut repo, "v1", Some("Back"), Some(start), true);
        assert_eq!(
            done.unwrap(),
            Tagged::Moved {
                from: two,
                to: start
            }
        );
        assert_eq!(tags(&repo)["v1"].message, "Back");
    }

    #[test]
    fn a_no_leaves_the_tag_where_it_was() {
        let (_dir, mut repo, start, two) = two_saves();
        tagged(&mut repo, "v1", None, Some(start), true).0.unwrap();
        let head = repo.head().unwrap();
        let (refused, asked) = tagged(&mut repo, "v1", Some("moved"), None, false);
        assert_eq!(asked.len(), 1);
        assert!(
            matches!(&refused, Err(TagError::NotMoved { name, from, to })
                if name == "v1" && *from == start && *to == two),
            "{refused:?}"
        );
        assert_eq!(repo.head().unwrap(), head);
    }

    #[test]
    fn deleting_asks_first_and_the_last_tag_leaves_no_list() {
        let (_dir, mut repo, s) = project();
        tagged(&mut repo, "v1", None, None, true).0.unwrap();
        let head = repo.head().unwrap();

        let (refused, asked) = deleted(&mut repo, "v1", false);
        assert_eq!(asked, [("v1".to_owned(), s, None)]);
        assert!(
            matches!(&refused, Err(TagError::NotDeleted { name }) if name == "v1"),
            "{refused:?}"
        );
        let unknown = deleted(&mut repo, "v2", true).0.unwrap_err();
        assert_eq!(unknown.to_string(), "There is no tag named \"v2\".");
        assert_eq!(
            unknown.hint().as_deref(),
            Some("See the tags with: opalro tag")
        );
        assert_eq!(repo.head().unwrap(), head);

        let (done, asked) = deleted(&mut repo, "v1", true);
        assert_eq!(done.unwrap(), s);
        assert_eq!(asked, [("v1".to_owned(), s, None)]);
        let (_, entry) = repo.current().unwrap();
        assert_eq!(entry.state.tags, None);
        assert_eq!(entry.command, "tag");
        assert_eq!(entry.folder_change, None);
    }

    #[test]
    fn undo_brings_a_deleted_tag_back() {
        let (_dir, mut repo, s) = project();
        tagged(&mut repo, "v1", Some("First"), None, true)
            .0
            .unwrap();
        deleted(&mut repo, "v1", true).0.unwrap();
        let req = StepRequest {
            n: 1,
            shelve: false,
            who: &who(),
            git_excludes: None,
        };
        undo(&mut repo, &req, &mut |_| {}).unwrap();
        assert_eq!(tags(&repo)["v1"], one("v1", s, "First", vec![]).tags["v1"]);
    }

    #[test]
    fn an_imported_tag_with_any_name_can_be_moved_and_deleted() {
        let (_dir, mut repo, start, two) = two_saves();
        plant(&mut repo, &one("weird name", start, "", vec![]));
        let (done, _) = tagged(&mut repo, "weird name", None, None, true);
        assert_eq!(
            done.unwrap(),
            Tagged::Moved {
                from: start,
                to: two
            }
        );
        assert_eq!(deleted(&mut repo, "weird name", true).0.unwrap(), two);
        assert_eq!(repo.current().unwrap().1.state.tags, None);
    }

    #[test]
    fn by_save_gives_each_saves_tags_in_order() {
        let (_dir, mut repo, start, two) = two_saves();
        tagged(&mut repo, "b", None, Some(start), true).0.unwrap();
        tagged(&mut repo, "a", None, Some(start), true).0.unwrap();
        tagged(&mut repo, "c", None, None, true).0.unwrap();
        let state = repo.current().unwrap().1.state;
        let before = repo.store().saves_read();
        let at = by_save(&repo, &state).unwrap();
        assert_eq!(repo.store().saves_read(), before);
        assert_eq!(
            at,
            HashMap::from([
                (start, vec!["a".to_owned(), "b".to_owned()]),
                (two, vec!["c".to_owned()]),
            ])
        );
        assert!(
            by_save(
                &repo,
                &State {
                    tags: None,
                    ..state
                }
            )
            .unwrap()
            .is_empty()
        );
    }

    #[test]
    fn a_message_and_the_tag_count_have_limits() {
        let (_dir, mut repo, s) = project();
        let head = repo.head().unwrap();
        let long = "x".repeat(64 * 1024 + 1);
        let refused = tagged(&mut repo, "v1", Some(&long), None, true)
            .0
            .unwrap_err();
        assert!(matches!(refused, TagError::MessageTooLong), "{refused:?}");
        assert_eq!(
            refused.to_string(),
            "A tag's message can be at most 64 KiB."
        );
        assert_eq!(repo.head().unwrap(), head);

        let full = TagList {
            tags: (0..MAX_TAGS)
                .map(|n| {
                    let tag = Tag {
                        save: s,
                        message: String::new(),
                        git_tags: vec![],
                    };
                    (format!("t{n}"), tag)
                })
                .collect(),
        };
        plant(&mut repo, &full);
        let head = repo.head().unwrap();
        let refused = tagged(&mut repo, "one-more", None, None, true)
            .0
            .unwrap_err();
        assert!(matches!(refused, TagError::TooMany), "{refused:?}");
        assert_eq!(
            refused.to_string(),
            "This project already has 100,000 tags, the most it can hold."
        );
        assert_eq!(repo.head().unwrap(), head);
        // A full list still lets a tag go.
        deleted(&mut repo, "t0", true).0.unwrap();
        assert_eq!(tags(&repo).len() as u64, MAX_TAGS - 1);
    }
}
