//! Importing a Git repository into Bolt (milestone 2): `import_git`, the whole of `bolt import`,
//! and why an import stops; in `names`, the names refs take in Bolt; in `convert`, how Git's
//! objects become Bolt's.
#![deny(clippy::indexing_slicing, clippy::arithmetic_side_effects)]

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fmt::Write as _;
use std::path::Path;

use crate::diary::{Action, DiaryEntry, Position, ServerLines, State, TagList};
use crate::encoding::DecodeError;
use crate::git::{self, GitError, GitId, Head, ObjectType, Refs, parse_tag, parse_tree, shown};
use crate::id::{Id, hash_record};
use crate::listing::EntryKind;
use crate::names::{NameError, is_git_alias, validate_name};
use crate::pack::{ObjectKind, PackError, PackSet};
use crate::repo::{RepoError, Repository};
use crate::save::Person;
use crate::store::{Store, StoreError};
use crate::text::{escape, thousands};
use crate::warning::{self, Warning};

mod convert;
mod names;

pub use convert::{Converter, Counts, TagOutcome};
use convert::{
    MAX_LISTED, at, bug, check_tag_chain, check_tag_object, expect_kind, shown_path,
    tree_and_parents,
};
use names::{Named, Names};

/// What `import_git` needs besides the folder.
pub struct ImportRequest<'a> {
    /// Who runs the import, for the diary; the saves keep Git's people.
    pub who: &'a Person,
    /// The command as the diary records it; already redacted.
    pub command: &'a str,
    /// Called after each commit is converted, with how many are done and how many there are.
    pub progress: &'a mut dyn FnMut(u64, u64),
}

/// What an import brought in, and what it left out (decision 033's summary). Names and paths
/// from Git are kept as they are, with U+FFFD for bytes that aren't UTF-8, and escaped only where
/// they are shown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportReport {
    pub commits: u64,
    pub lines: u64,
    pub servers: u64,
    pub server_lines: u64,
    pub tags: u64,
    /// On the line of Git's current branch, or at the save of a detached `HEAD`.
    pub position: Position,
    pub renamed: Vec<Renaming>,
    /// The whole names of the first 100 refs not imported (the stash, notes, pull requests and
    /// the like), in byte order, and how many there are.
    pub skipped_refs: Vec<String>,
    pub skipped_ref_count: u64,
    /// Each tag not imported, by its Git name, and why.
    pub skipped_tags: Vec<(String, String)>,
    /// The first 100 entries left out of saves, as the commit (hex) and the path.
    pub skipped_entries: Vec<(String, String)>,
    pub skipped_entry_count: u64,
    /// Commits a later export can't recreate with their Git IDs.
    pub inexact_commits: u64,
    pub warnings: Vec<Warning>,
}

/// A name Bolt's rules refuse, and the one it was given (decision 033).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Renaming {
    /// `line`, `server`, `server_line` or `tag`.
    pub kind: &'static str,
    /// Git's name; a server line's is `<server>/<branch>`, as Git shows it.
    pub git: String,
    /// Bolt's name; a server line's is `<line>@<server>`, as Bolt shows it.
    pub bolt: String,
}

/// Why an import stops. Text from the repository is shown escaped.
#[derive(Debug, thiserror::Error)]
pub enum ImportError {
    #[error(transparent)]
    Git(#[from] GitError),
    #[error(transparent)]
    Repo(#[from] RepoError),
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error(transparent)]
    Pack(#[from] PackError),
    /// The import can't go on; `what` says why, naming the commit or tag.
    #[error("{}", escape(what, false))]
    Stop { what: String, hint: Option<String> },
    /// Names that aren't valid UTF-8 (decision 019): the first ones as the commit (hex) and the
    /// path, with U+FFFD for bytes that aren't UTF-8, and how many there are. Paths here, as in
    /// `Counts`, are kept as they are and escaped only where they are shown.
    #[error("{}", not_utf8_message(first, *count))]
    NotUtf8Names {
        first: Vec<(String, String)>,
        count: u64,
    },
    /// A Git LFS pointer (decision 037), in the commit `commit` (hex) at `path`.
    #[error(
        "Commit `{}` holds a Git LFS pointer at `{}`, and this version of Bolt can't import Git \
         LFS files yet: Git LFS repositories come in milestone 5.",
        escape(commit, false),
        escape(path, false)
    )]
    LfsPointer { commit: String, path: String },
}

impl ImportError {
    /// What the user can do next, if there is something to say.
    #[must_use]
    pub fn hint(&self) -> Option<String> {
        match self {
            ImportError::Git(e) => e.hint(),
            ImportError::Repo(e) => e.hint(),
            ImportError::Store(_) | ImportError::Pack(_) => None,
            ImportError::Stop { hint, .. } => hint.clone(),
            ImportError::NotUtf8Names { .. } => Some(
                "Nothing was changed. Rename them in the Git history to names in UTF-8, then run \
                 bolt import again."
                    .to_owned(),
            ),
            ImportError::LfsPointer { .. } => Some(
                "Nothing was changed. To import now, turn the Git LFS files back into ordinary \
                 Git files in a copy of the repository with `git lfs migrate export --everything \
                 --include=\"*\"`, which rewrites its history, then run bolt import in that copy."
                    .to_owned(),
            ),
        }
    }
}

fn not_utf8_message(first: &[(String, String)], count: u64) -> String {
    let mut out = if count == 1 {
        "A name in the Git history isn't valid UTF-8, and Bolt keeps only UTF-8 names:".to_owned()
    } else {
        format!(
            "{count} names in the Git history aren't valid UTF-8, and Bolt keeps only UTF-8 names:"
        )
    };
    for (commit, path) in first {
        let (path, commit) = (escape(path, false), escape(commit, false));
        let _ = write!(out, "\n  `{path}` in commit `{commit}`");
    }
    let more = count.saturating_sub(first.len() as u64);
    if more > 0 {
        let _ = write!(out, "\n  and {more} more");
    }
    out
}

/// Imports the Git repository whose work tree is `root` into a new Bolt project there (decisions
/// 033, 034 and 040): every commit its branches, server branches, tags and `HEAD` reach becomes a
/// save. Everything that only reads comes first, so most refusals come before `.bolt` is made.
/// Nothing is committed unless all of it works, and running it again after a failure finishes the
/// job, reusing what was stored once it is checked (decision 041).
pub fn import_git(root: &Path, req: &mut ImportRequest) -> Result<ImportReport, ImportError> {
    Repository::check_new(root).map_err(starting)?;
    let git = git::Repo::open(root)?;
    let refs = git.refs()?;
    let names = Names::of(&refs)?;
    let roots = roots(&git, &refs, &names.tags)?;
    if roots.is_empty() {
        return Err(ImportError::Stop {
            what: "There's nothing to import: this Git repository has no commits. bolt start \
                   makes a new project here."
                .to_owned(),
            hint: None,
        });
    }
    let (head_line, head_commit) = match &refs.head {
        Head::Branch(branch) => match names.lines.iter().find(|line| line.git == *branch) {
            Some(line) => (Some(line.bolt.clone()), line.target),
            None => return Err(unborn(branch)),
        },
        Head::Detached(id) => (None, *id),
    };
    if let Some(name) = tracked_bolt(&git, &head_commit)? {
        return Err(ImportError::Stop {
            what: format!(
                "Git tracks `{name}` at the top of this project, and Bolt can't use it as its own \
                 .bolt folder."
            ),
            hint: Some(format!(
                "Nothing was changed. Take it out of Git with `git rm -r --cached {name}` and \
                 commit, or move it, then run bolt import again."
            )),
        });
    }
    let order = in_order(&git, &roots)?;
    let grafts = git.has_grafts()?;

    let (mut repo, lock) = Repository::begin(root).map_err(starting)?;
    let mut packs = repo.new_packs()?;
    let mut converter = Converter::new(&git, repo.store(), &mut packs);
    let total = order.len() as u64;
    for (done, (id, parents)) in (1..).zip(&order) {
        let parent_saves = parents
            .iter()
            .map(|parent| saved(&converter, parent))
            .collect::<Result<Vec<_>, _>>()?;
        converter.commit(id, &parent_saves)?;
        (req.progress)(done, total);
    }
    converter.finish()?;
    let mut tag_list = TagList::default();
    let mut skipped_tags = Vec::new();
    for tag in &names.tags {
        match converter.tag(&tag.bolt, &tag.target)? {
            TagOutcome::Tag(kept) => {
                tag_list.tags.insert(tag.bolt.clone(), kept);
            }
            TagOutcome::Skipped { reason } => skipped_tags.push((lossy(&tag.git), reason)),
        }
    }
    let lines = lines_of(&converter, &names.lines)?;
    let mut server_lines = ServerLines::default();
    for (server, own) in &names.servers {
        let own = lines_of(&converter, own)?;
        server_lines.servers.insert(server.clone(), own);
    }
    let position = match head_line {
        Some(line) => Position::OnLine(line),
        None => Position::AtSave(saved(&converter, &head_commit)?),
    };
    let counts = converter.counts().clone();

    let tags = if tag_list.tags.is_empty() {
        None
    } else {
        let encoded = tag_list.encode();
        let read_back = |bytes: &[u8]| TagList::decode(bytes).map(drop);
        Some(put_record(
            repo.store(),
            &mut packs,
            ObjectKind::TagList,
            &encoded,
            read_back,
        )?)
    };
    let servers = if server_lines.servers.is_empty() {
        None
    } else {
        let encoded = server_lines.encode();
        let read_back = |bytes: &[u8]| ServerLines::decode(bytes).map(drop);
        Some(put_record(
            repo.store(),
            &mut packs,
            ObjectKind::ServerLines,
            &encoded,
            read_back,
        )?)
    };
    let mut warnings = bolt_folder_warnings(&git, &head_commit)?;
    if grafts {
        warnings.push(Warning::new(
            warning::GRAFTS_IGNORED,
            "Git's grafts file was ignored; Bolt imported the commits as they are.",
        ));
    }
    let mut report = ImportReport {
        commits: total,
        lines: lines.len() as u64,
        servers: server_lines.servers.len() as u64,
        server_lines: server_lines
            .servers
            .values()
            .map(|own| own.len() as u64)
            .sum(),
        tags: tag_list.tags.len() as u64,
        position: position.clone(),
        renamed: names.renamed,
        skipped_refs: refs
            .other
            .iter()
            .take(MAX_LISTED)
            .map(|name| lossy(name))
            .collect(),
        skipped_ref_count: refs.other.len() as u64,
        skipped_tags,
        skipped_entries: counts.skipped_entries,
        skipped_entry_count: counts.skipped_entry_count,
        inexact_commits: counts.inexact_commits,
        warnings,
    };
    let entry = DiaryEntry {
        parent: None,
        action: Action::Command,
        command: req.command.to_owned(),
        who: req.who.clone(),
        bolt_version: env!("CARGO_PKG_VERSION").into(),
        state: State {
            lines,
            tags,
            position,
            shelves: Vec::new(),
            server_lines: servers,
        },
        safety_copies: Vec::new(),
    };
    repo.commit_first(&lock, packs, &entry)?;
    report.warnings.extend(repo.store().warnings());
    Ok(report)
}

/// A refusal from making the project, as an import says it.
fn starting(e: RepoError) -> ImportError {
    match e {
        RepoError::AlreadyAProject { .. } => ImportError::Stop {
            what: "This folder is already a Bolt project. Picking up new Git commits comes in a \
                   later version."
                .to_owned(),
            hint: Some(
                "To import the Git repository again, run bolt import in a fresh clone of it."
                    .to_owned(),
            ),
        },
        // Its own hint says to run `bolt start` again, which would make an empty project.
        RepoError::StartStopped { refusal } => {
            let hint = refusal
                .hint()
                .map(|hint| format!("{hint}\nThen run bolt import again to finish."));
            let what = RepoError::StartStopped { refusal }.to_string();
            ImportError::Stop { what, hint }
        }
        other => other.into(),
    }
}

/// Git's `HEAD` names the branch `branch`, which has no commits yet (decision 033).
fn unborn(branch: &[u8]) -> ImportError {
    ImportError::Stop {
        what: format!(
            "Git's current branch `{}` has no commits yet; switch to a branch with commits, or \
             commit first.",
            shown(branch)
        ),
        hint: Some(
            "Nothing was changed. Run bolt import again once Git's current branch has commits."
                .to_owned(),
        ),
    }
}

/// Bytes from Git as text, with U+FFFD for bytes that aren't UTF-8.
fn lossy(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// The commits an import starts from (decision 033): those of the branches and the server
/// branches, those the tags end at, and a detached `HEAD`'s. Tags are held to the limits
/// `Converter::tag` holds them to, so a tag it would refuse stops the import before anything is
/// written.
fn roots(git: &git::Repo, refs: &Refs, tags: &[Named]) -> Result<BTreeSet<GitId>, ImportError> {
    let mut roots: BTreeSet<GitId> = refs.branches.iter().map(|(_, id)| *id).collect();
    roots.extend(refs.servers.iter().map(|(_, _, id)| *id));
    if let Head::Detached(id) = &refs.head {
        roots.insert(*id);
    }
    // Where the chain from each tag object read ends (a commit, or nothing Bolt can tag), and how
    // many tag objects it holds from there, that one included.
    let mut ends: HashMap<GitId, (Option<GitId>, u64)> = HashMap::new();
    for tag in tags {
        let name = tag.bolt.as_str();
        let mut read = Vec::new();
        let mut id = tag.target;
        let (end, rest) = loop {
            if let Some(&(end, objects)) = ends.get(&id) {
                check_tag_chain(name, (read.len() as u64).saturating_add(objects))?;
                break (end, objects);
            }
            let hex = id.to_hex();
            let here = || format!("tag `{name}`, at object `{hex}`");
            let (kind, size) = git.header(&id).map_err(|e| at(e, here))?;
            match kind {
                ObjectType::Commit => break (Some(id), 0),
                ObjectType::Tree | ObjectType::Blob => break (None, 0),
                ObjectType::Tag => {}
            }
            check_tag_object(name, &id, size)?;
            check_tag_chain(name, (read.len() as u64).saturating_add(1))?;
            let object = git.read(&id).map_err(|e| at(e, here))?;
            expect_kind(object.kind, ObjectType::Tag, &id, here)?;
            check_tag_object(name, &id, object.data.len() as u64)?;
            read.push(id);
            id = parse_tag(&object.data, git.hash())
                .map_err(|e| at(e, here))?
                .object;
        };
        for (i, object) in read.iter().enumerate() {
            let objects = (read.len().saturating_sub(i) as u64).saturating_add(rest);
            ends.insert(*object, (end, objects));
        }
        roots.extend(end);
    }
    Ok(roots)
}

/// Every commit `roots` reach, each after its parents, with its parents in its own order: pass 1,
/// which reads each commit once before anything is written. It walks with a stack of its own,
/// into roots and parents in the order of their Git IDs, so the order is the same every time.
fn in_order(
    git: &git::Repo,
    roots: &BTreeSet<GitId>,
) -> Result<Vec<(GitId, Vec<GitId>)>, ImportError> {
    let mut order = Vec::new();
    let mut seen = HashSet::new();
    // Each commit on the way down, its parents, and those still to go into.
    let mut stack: Vec<(GitId, Vec<GitId>, std::vec::IntoIter<GitId>)> = Vec::new();
    let enter = |id: GitId, stack: &mut Vec<_>| -> Result<(), ImportError> {
        let (_, parents) = tree_and_parents(git, &id)?;
        let mut next = parents.clone();
        next.sort();
        stack.push((id, parents, next.into_iter()));
        Ok(())
    };
    for root in roots {
        if !seen.insert(*root) {
            continue;
        }
        enter(*root, &mut stack)?;
        while let Some((_, _, next)) = stack.last_mut() {
            match next.next() {
                Some(parent) => {
                    if seen.insert(parent) {
                        enter(parent, &mut stack)?;
                    }
                }
                None => {
                    if let Some((id, parents, _)) = stack.pop() {
                        order.push((id, parents));
                    }
                }
            }
        }
    }
    Ok(order)
}

/// The save the converter made of `commit`, which `in_order` put before whatever needs it.
fn saved(converter: &Converter<'_>, commit: &GitId) -> Result<Id, ImportError> {
    converter.save_of(commit).ok_or_else(|| {
        bug(format!(
            "Bolt needed the save of commit `{}` before it made it",
            commit.to_hex()
        ))
    })
}

/// The lines `refs` become: each name with the save of its commit.
fn lines_of(
    converter: &Converter<'_>,
    refs: &[Named],
) -> Result<BTreeMap<String, Id>, ImportError> {
    refs.iter()
        .map(|line| Ok((line.bolt.clone(), saved(converter, &line.target)?)))
        .collect()
}

/// Stores the record `encoded` of `kind`, a tag list or server lines, unless a good copy is there
/// already. Their encoders don't check what they write, so `read_back` decodes it first. Their
/// sizes were held to a record's limit before anything was written: tags by the converter, server
/// lines by `Names::of`.
fn put_record(
    store: &Store,
    packs: &mut PackSet,
    kind: ObjectKind,
    encoded: &[u8],
    read_back: impl FnOnce(&[u8]) -> Result<(), DecodeError>,
) -> Result<Id, ImportError> {
    let what = if kind == ObjectKind::TagList {
        "tag list"
    } else {
        "server-lines list"
    };
    read_back(encoded).map_err(|e| bug(format!("Bolt made a {what} it can't read back ({e})")))?;
    let flavor = kind
        .flavor()
        .ok_or_else(|| bug(format!("Bolt took a {what} for content")))?;
    let id = hash_record(flavor, encoded);
    let size = encoded.len() as u64;
    if !packs.contains(&id) && !store.has_good_copy(&id, kind, size)? {
        packs.add_record(kind, encoded)?;
    }
    Ok(id)
}

/// The name of an entry spelled `.bolt` at the top of the tree of `HEAD`'s commit, of any kind,
/// if Git tracks one: Git checks it out where Bolt keeps the project, and Bolt never takes files
/// Git wrote for its own.
fn tracked_bolt(git: &git::Repo, head: &GitId) -> Result<Option<String>, ImportError> {
    let (tree, _) = tree_and_parents(git, head)?;
    let place = || format!("the top folder (tree `{}`) of `HEAD`", tree.to_hex());
    let object = git.read(&tree).map_err(|e| at(e, place))?;
    expect_kind(object.kind, ObjectType::Tree, &tree, place)?;
    let entries = parse_tree(&object.data, git.hash()).map_err(|e| at(e, place))?;
    Ok(entries.into_iter().find_map(|e| {
        let name = std::str::from_utf8(e.name).ok()?;
        (validate_name(name) == Err(NameError::BoltSpelling)).then(|| name.to_owned())
    }))
}

/// A warning for each folder named `.bolt` that Git checked out below the top, from the tree of
/// `HEAD`'s commit: Bolt left them out of history, but commands run inside one would take it for
/// a project (decision 019). `tracked_bolt` found none at the top.
fn bolt_folder_warnings(git: &git::Repo, head: &GitId) -> Result<Vec<Warning>, ImportError> {
    let (tree, _) = tree_and_parents(git, head)?;
    let mut warnings = Vec::new();
    let mut count = 0u64;
    // Each tree is looked at once, so a folder Git holds at two paths is named at the first.
    let mut seen = HashSet::from([tree]);
    // Each folder on the way down, by name, with the folders in it still to look at.
    let mut stack = vec![(String::new(), subfolders(git, &tree)?.into_iter())];
    while let Some((_, left)) = stack.last_mut() {
        let Some((name, id, is_bolt)) = left.next() else {
            stack.pop();
            continue;
        };
        if is_bolt {
            count = count.saturating_add(1);
            if warnings.len() < MAX_LISTED {
                let mut path: Vec<&str> = stack.iter().skip(1).map(|(n, _)| n.as_str()).collect();
                path.push(&name);
                let message = format!(
                    "Git checked out a folder named .bolt at `{}`. Bolt left it out of history, \
                     but commands run inside it would use it as a Bolt project; delete it unless \
                     you made it.",
                    escape(&shown_path(&path), false)
                );
                warnings.push(Warning::new(warning::NESTED_BOLT_FOLDER, message));
            }
        } else if seen.insert(id) {
            let inner = subfolders(git, &id)?.into_iter();
            stack.push((name, inner));
        }
    }
    let more = count.saturating_sub(warnings.len() as u64);
    if more > 0 {
        let message = if more == 1 {
            "Git checked out 1 more folder named .bolt. Bolt left it out of history too; delete \
             it unless you made it."
                .to_owned()
        } else {
            format!(
                "Git checked out {} more folders named .bolt. Bolt left them out of history too; \
                 delete them unless you made them.",
                thousands(more)
            )
        };
        warnings.push(Warning::new(warning::NESTED_BOLT_FOLDER, message));
    }
    Ok(warnings)
}

/// The folders in the Git tree `id` that saves keep, and those spelled `.bolt`, which they leave
/// out (`true`), as the converter judges their names.
fn subfolders(git: &git::Repo, id: &GitId) -> Result<Vec<(String, GitId, bool)>, ImportError> {
    let place = || format!("tree `{}`", id.to_hex());
    let object = git.read(id).map_err(|e| at(e, place))?;
    expect_kind(object.kind, ObjectType::Tree, id, place)?;
    let entries = parse_tree(&object.data, git.hash()).map_err(|e| at(e, place))?;
    Ok(entries
        .into_iter()
        .filter(|e| e.kind == EntryKind::Folder)
        .filter_map(|e| {
            let name = std::str::from_utf8(e.name).ok()?;
            match validate_name(name) {
                Err(NameError::BoltSpelling) => Some((name.to_owned(), e.id, true)),
                Ok(()) if !is_git_alias(name) => Some((name.to_owned(), e.id, false)),
                _ => None,
            }
        })
        .collect())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    #![allow(clippy::indexing_slicing, clippy::arithmetic_side_effects)]
    use super::*;
    use crate::git::craft::{Craft, PackEntry};
    use crate::git::{GitId, HashKind, ObjectType, object_id};
    use crate::save::Person;

    fn who() -> Person {
        Person {
            name: "Ada".into(),
            email: "ada@example.com".into(),
            time_ms: 0,
            tz_offset_min: 0,
        }
    }

    /// A repository of one commit on `main`, as `Craft` writes it, and the commit.
    fn one_commit(dir: &Path) -> (Craft, GitId) {
        let git = Craft::new(dir, HashKind::Sha1);
        let blob = git.object(ObjectType::Blob, b"a\n");
        let tree = git.tree(&[(b"100644", b"a.txt", blob)]);
        let raw = format!(
            "tree {}\nauthor A <a@example.com> 0 +0000\ncommitter A <a@example.com> 0 +0000\n\n\
             One\n",
            tree.to_hex()
        );
        let commit = git.commit(raw.as_bytes());
        git.loose_ref("refs/heads/main", &format!("{}\n", commit.to_hex()));
        (git, commit)
    }

    /// A tag object of `target`, a `kind`, with `message`.
    fn tag_object(git: &Craft, target: &GitId, kind: &str, message: &[u8]) -> GitId {
        let head = format!(
            "object {}\ntype {kind}\ntag t\ntagger T <t@example.com> 0 +0000\n\n",
            target.to_hex()
        );
        git.object(ObjectType::Tag, &[head.as_bytes(), message].concat())
    }

    fn import(dir: &Path) -> Result<ImportReport, ImportError> {
        import_git(
            dir,
            &mut ImportRequest {
                who: &who(),
                command: "import",
                progress: &mut |_, _| {},
            },
        )
    }

    /// The converter would refuse these tags only after every commit was converted; the first
    /// pass refuses them alike, before `.bolt` is made.
    #[test]
    fn tags_past_bolts_limits_stop_the_import_before_anything_is_written() {
        // A chain of `objects` tag objects to `commit`, the one nearest it holding `message`.
        let chain = |git: &Craft, commit: &GitId, objects: usize, message: &[u8]| {
            let mut id = tag_object(git, commit, "commit", message);
            for _ in 1..objects {
                id = tag_object(git, &id, "tag", b"n\n");
            }
            id
        };
        let big = vec![b'x'; crate::diary::MAX_GIT_TAG_OBJECT_BYTES];
        for (refusal, objects, message) in [
            ("more than the 1 MiB Bolt keeps", 1, big.as_slice()),
            ("more than 16 tag objects", 17, b"1\n"),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let (git, commit) = one_commit(dir.path());
            let id = chain(&git, &commit, objects, message);
            git.loose_ref("refs/tags/t", &format!("{}\n", id.to_hex()));
            match import(dir.path()) {
                Err(e @ ImportError::Git(GitError::Unimportable { .. })) => {
                    assert!(e.to_string().contains(refusal), "{e}");
                }
                other => panic!("{refusal}: {:?}", other.map(|_| ())),
            }
            assert!(!dir.path().join(".bolt").exists(), "{refusal}");
        }
        // Sixteen tag objects are as many as Bolt keeps.
        let dir = tempfile::tempdir().unwrap();
        let (git, commit) = one_commit(dir.path());
        let id = chain(&git, &commit, 16, b"1\n");
        git.loose_ref("refs/tags/t", &format!("{}\n", id.to_hex()));
        assert_eq!(import(dir.path()).unwrap().tags, 1);
    }

    /// Any spelling of `.bolt` the converter would leave out, of any kind, as long as Git tracks
    /// it at the top: here nothing is checked out, so only the tree shows it.
    #[test]
    fn a_bolt_entry_git_tracks_at_the_top_stops_the_import() {
        for (name, mode) in [
            (".bolt", "100644"),
            (".Bolt.", "40000"),
            ("BOLT~1", "120000"),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let git = Craft::new(dir.path(), HashKind::Sha1);
            let blob = git.object(ObjectType::Blob, b"a\n");
            let inner = git.tree(&[(b"100644", b"config.toml", blob)]);
            let id = if mode == "40000" { inner } else { blob };
            let tree = git.tree(&[(mode.as_bytes(), name.as_bytes(), id)]);
            let raw = format!(
                "tree {}\nauthor A <a@example.com> 0 +0000\ncommitter A <a@example.com> 0 \
                 +0000\n\nOne\n",
                tree.to_hex()
            );
            let commit = git.commit(raw.as_bytes());
            git.loose_ref("refs/heads/main", &format!("{}\n", commit.to_hex()));
            match import(dir.path()) {
                Err(e @ ImportError::Stop { .. }) => {
                    let what = format!("Git tracks `{name}` at the top of this project");
                    assert!(e.to_string().starts_with(&what), "{e}");
                    let rm = format!("`git rm -r --cached {name}`");
                    assert!(e.hint().unwrap().contains(&rm), "{e:?}");
                }
                other => panic!("{name}: {:?}", other.map(|_| ())),
            }
            assert!(!dir.path().join(".bolt").exists(), "{name}");
        }
    }

    /// Decision 033's limits on lines, read from a real `packed-refs`, stop the import before
    /// anything is written.
    #[test]
    fn more_lines_than_bolt_holds_stop_the_import_before_anything_is_written() {
        for (prefix, refusal) in [
            ("refs/heads/b", "more than 100,000 branches"),
            (
                "refs/remotes/origin/b",
                "server `origin` has more than 100,000 branches",
            ),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let (git, commit) = one_commit(dir.path());
            let mut packed = "# pack-refs with: peeled fully-peeled sorted \n".to_owned();
            for n in 0..=100_000 {
                let _ = writeln!(packed, "{} {prefix}{n:06}", commit.to_hex());
            }
            git.packed_refs(&packed);
            match import(dir.path()) {
                Err(ImportError::Git(e @ GitError::TooManyRefs { .. })) => {
                    assert!(e.to_string().contains(refusal), "{e}");
                }
                other => panic!("{refusal}: {:?}", other.map(|_| ())),
            }
            assert!(!dir.path().join(".bolt").exists(), "{refusal}");
        }
    }

    #[test]
    fn bolt_folders_past_the_first_100_are_counted() {
        for (folders, last) in [
            (
                101,
                "Git checked out 1 more folder named .bolt. Bolt left it out of history too; \
                 delete it unless you made it.",
            ),
            (
                102,
                "Git checked out 2 more folders named .bolt. Bolt left them out of history \
                 too; delete them unless you made them.",
            ),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let git = Craft::new(dir.path(), HashKind::Sha1);
            let blob = git.object(ObjectType::Blob, b"a\n");
            let bolt = git.tree(&[(b"100644", b"x", blob)]);
            // Each folder's tree its own, since each tree is looked at once.
            let folders: Vec<(Vec<u8>, GitId)> = (0..folders)
                .map(|n| {
                    let own = git.object(ObjectType::Blob, format!("{n}\n").as_bytes());
                    let folder =
                        git.tree(&[(b"40000", b".bolt", bolt), (b"100644", b"n.txt", own)]);
                    (format!("d{n:03}").into_bytes(), folder)
                })
                .collect();
            let entries: Vec<(&[u8], &[u8], GitId)> = folders
                .iter()
                .map(|(name, id)| (&b"40000"[..], name.as_slice(), *id))
                .collect();
            let tree = git.tree(&entries);
            let raw = format!(
                "tree {}\nauthor A <a@example.com> 0 +0000\ncommitter A <a@example.com> 0 \
                 +0000\n\nOne\n",
                tree.to_hex()
            );
            let commit = git.commit(raw.as_bytes());
            let repo = git::Repo::open(dir.path()).unwrap();
            let warnings = bolt_folder_warnings(&repo, &commit).unwrap();
            assert_eq!(warnings.len(), 101);
            assert!(
                warnings[0].message.contains("at `d000/.bolt`."),
                "{warnings:?}"
            );
            assert_eq!(warnings[100].message, last);
        }
    }

    /// Bolt keeps up to 16 packs open between reads, and goes on reading one it has open as it
    /// was, so the pack removed here is one the import hasn't read from yet.
    #[test]
    fn import_stops_cleanly_when_git_removes_a_pack() {
        use ObjectType::{Blob, Commit, Tree};
        let dir = tempfile::tempdir().unwrap();
        let git = Craft::new(dir.path(), HashKind::Sha1);
        let id = |kind, data: &[u8]| object_id(kind, data, HashKind::Sha1).unwrap();
        let tree = |blob: GitId| [b"100644 a.txt\0".as_slice(), blob.as_bytes()].concat();
        let commit = |tree: &[u8], parent: Option<&[u8]>, message: &str| {
            let mut raw = format!("tree {}\n", id(Tree, tree).to_hex());
            if let Some(parent) = parent {
                raw += &format!("parent {}\n", id(Commit, parent).to_hex());
            }
            raw + "author A <a@example.com> 0 +0000\ncommitter A <a@example.com> 0 +0000\n\n"
                + message
        };
        let (one, two) = (b"one".to_vec(), b"two".to_vec());
        let (tree1, tree2) = (tree(id(Blob, &one)), tree(id(Blob, &two)));
        let first = commit(&tree1, None, "One\n");
        let second = commit(&tree2, Some(first.as_bytes()), "Two\n");
        // All but the second commit's file in one pack, which the import reads before it writes
        // anything; that file alone in another.
        git.pack(&[
            PackEntry::Whole(Commit, first.clone().into_bytes()),
            PackEntry::Whole(Commit, second.clone().into_bytes()),
            PackEntry::Whole(Tree, tree1),
            PackEntry::Whole(Blob, one),
            PackEntry::Whole(Tree, tree2),
        ]);
        let (later, _) = git.pack(&[PackEntry::Whole(Blob, two)]);
        let head = id(Commit, second.as_bytes()).to_hex();
        git.loose_ref("refs/heads/main", &format!("{head}\n"));
        let mut progress = |done: u64, _: u64| {
            if done == 1 {
                std::fs::remove_file(&later).unwrap();
            }
        };
        let result = import_git(
            dir.path(),
            &mut ImportRequest {
                who: &who(),
                command: "import",
                progress: &mut progress,
            },
        );
        assert!(
            matches!(result, Err(ImportError::Git(GitError::Changed { .. }))),
            "{result:?}"
        );
        assert!(!later.exists(), "the pack was removed");
        assert!(!dir.path().join(".bolt/format").exists());
    }
}
