//! Importing a Git repository into Bolt (milestone 2): `import_git`, the whole of `bolt import`,
//! why an import stops, and, in `convert`, how Git's objects become Bolt's.
#![deny(clippy::indexing_slicing, clippy::arithmetic_side_effects)]

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fmt::Write as _;
use std::path::Path;

use crate::diary::{
    Action, DiaryEntry, MAX_GIT_TAG_OBJECT_BYTES, MAX_LINES, MAX_SERVERS, MAX_TAGS, Position,
    ServerLines, State, TagList, is_line_name_char,
};
use crate::encoding::DecodeError;
use crate::git::{self, GitError, GitId, Head, ObjectType, Refs, parse_tag, parse_tree, shown};
use crate::id::{Id, hash_record};
use crate::listing::EntryKind;
use crate::names::{MAX_NAME_BYTES, NameError, is_git_alias, validate_name};
use crate::pack::{MAX_RECORD_SIZE, ObjectKind, PackError, PackSet};
use crate::repo::{RepoError, Repository};
use crate::save::Person;
use crate::store::{Store, StoreError};
use crate::text::escape;
use crate::warning::{self, Warning};

mod convert;

pub use convert::{Converter, Counts, TagOutcome};
use convert::{at, bug, expect_kind, shown_path, tree_and_parents};

/// How many folders named `.bolt` the import's warnings name; the rest are counted.
const MAX_NAMED_BOLT_FOLDERS: usize = 100;

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
    /// The whole names of the refs not imported: the stash, notes and the like.
    pub skipped_refs: Vec<String>,
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
    let roots = roots(&git, &refs)?;
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
        skipped_refs: refs.other.iter().map(|name| lossy(name)).collect(),
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

/// A ref an import brings in: its Git name, the name it takes in Bolt, and what it points at.
struct Named {
    git: Vec<u8>,
    bolt: String,
    target: GitId,
}

/// Which of Bolt's rules a name must pass.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Rules {
    /// Line and server names: decision 012's characters, and no two alike but for letter case.
    Line,
    /// Tag names: any UTF-8, and no two the same.
    Tag,
}

/// The refs an import brings in, by the names they take in Bolt (decision 033).
struct Names {
    lines: Vec<Named>,
    /// Each server's lines, by the server's Bolt name.
    servers: BTreeMap<String, Vec<Named>>,
    tags: Vec<Named>,
    renamed: Vec<Renaming>,
}

impl Names {
    /// Names every ref Bolt takes, once decision 033's limits are checked.
    fn of(refs: &Refs) -> Result<Names, ImportError> {
        let too_many = |what: &str| -> ImportError {
            GitError::TooManyRefs {
                what: what.to_owned(),
            }
            .into()
        };
        if refs.branches.len() as u64 > MAX_LINES {
            return Err(too_many(
                "it has more than 100,000 branches, and Bolt holds at most 100,000 lines",
            ));
        }
        let mut by_server: BTreeMap<&[u8], Vec<(Vec<u8>, GitId)>> = BTreeMap::new();
        for (server, branch, id) in &refs.servers {
            let own = by_server.entry(server.as_slice()).or_default();
            own.push((branch.clone(), *id));
        }
        if by_server.len() as u64 > MAX_SERVERS {
            return Err(too_many(
                "it has branches from more than 64 servers, and Bolt holds at most 64",
            ));
        }
        if let Some((server, _)) = by_server
            .iter()
            .find(|(_, own)| own.len() as u64 > MAX_LINES)
        {
            return Err(too_many(&format!(
                "server `{}` has more than 100,000 branches, and Bolt holds at most 100,000 \
                 lines for one server",
                shown(server)
            )));
        }
        // Every tag ref counts, the ones Bolt can't hold too: each costs a look.
        if refs.tags.len() as u64 > MAX_TAGS {
            return Err(too_many(
                "it has more than 100,000 tags, and Bolt holds at most 100,000",
            ));
        }
        let mut renamed = Vec::new();
        let lines = named(&refs.branches, Rules::Line, "line", &mut renamed);
        let server_names: Vec<&[u8]> = by_server.keys().copied().collect();
        let mut servers = BTreeMap::new();
        for ((git, own), bolt) in by_server.iter().zip(rename(&server_names, Rules::Line)) {
            if bolt.as_bytes() != *git {
                renamed.push(Renaming {
                    kind: "server",
                    git: lossy(git),
                    bolt: bolt.clone(),
                });
            }
            let mut own_renamed = Vec::new();
            let own = named(own, Rules::Line, "server_line", &mut own_renamed);
            // Shown as each tool shows a server's branch.
            renamed.extend(own_renamed.into_iter().map(|line| Renaming {
                git: format!("{}/{}", lossy(git), line.git),
                bolt: format!("{}@{bolt}", line.bolt),
                ..line
            }));
            servers.insert(bolt, own);
        }
        let tags = named(&refs.tags, Rules::Tag, "tag", &mut renamed);
        Ok(Names {
            lines,
            servers,
            tags,
            renamed,
        })
    }
}

/// `refs`, in the byte order of their names, with the names they take in Bolt under `rules`;
/// each one renamed is noted as `kind`.
fn named(
    refs: &[(Vec<u8>, GitId)],
    rules: Rules,
    kind: &'static str,
    renamed: &mut Vec<Renaming>,
) -> Vec<Named> {
    let names: Vec<&[u8]> = refs.iter().map(|(name, _)| name.as_slice()).collect();
    let bolt = rename(&names, rules);
    refs.iter()
        .zip(bolt)
        .map(|((git, target), bolt)| {
            if bolt.as_bytes() != git.as_slice() {
                renamed.push(Renaming {
                    kind,
                    git: lossy(git),
                    bolt: bolt.clone(),
                });
            }
            Named {
                git: git.clone(),
                bolt,
                target: *target,
            }
        })
        .collect()
}

/// Decision 033's renaming rule, over `names` in their byte order: each character `rules` refuse
/// becomes `-`, as do bytes that aren't UTF-8, wherever U+FFFD would stand for them; a name over
/// 255 bytes is cut at a character; and a name like one taken already (for lines, but for letter
/// case) gets `-2`, `-3` and so on, the first that is free. So the same names always come out the
/// same.
fn rename(names: &[&[u8]], rules: Rules) -> Vec<String> {
    let fold = |name: &str| match rules {
        Rules::Line => name.to_lowercase(),
        Rules::Tag => name.to_owned(),
    };
    let mut taken = HashSet::new();
    // The next number to try after each name, so a thousand alike names don't each count up.
    let mut next: HashMap<String, u64> = HashMap::new();
    let mut out = Vec::with_capacity(names.len());
    for name in names {
        let mut whole = String::with_capacity(name.len());
        for chunk in name.utf8_chunks() {
            for c in chunk.valid().chars() {
                let allowed = rules == Rules::Tag || is_line_name_char(c);
                whole.push(if allowed { c } else { '-' });
            }
            if !chunk.invalid().is_empty() {
                whole.push('-');
            }
        }
        let base = cut(&whole, MAX_NAME_BYTES);
        let mut bolt = base.to_owned();
        if !taken.insert(fold(&bolt)) {
            let n = next.entry(fold(base)).or_insert(2);
            loop {
                let suffix = format!("-{n}");
                let tried =
                    cut(base, MAX_NAME_BYTES.saturating_sub(suffix.len())).to_owned() + &suffix;
                *n = n.saturating_add(1);
                if taken.insert(fold(&tried)) {
                    bolt = tried;
                    break;
                }
            }
        }
        out.push(bolt);
    }
    out
}

/// `name`, cut to at most `max` bytes at a character, without a `/` at its end, which would make
/// an empty last part.
fn cut(name: &str, max: usize) -> &str {
    let end = name.floor_char_boundary(max);
    name.get(..end).unwrap_or(name).trim_end_matches('/')
}

/// The commits an import starts from (decision 033): those of the branches and the server
/// branches, those the tags end at, and a detached `HEAD`'s.
fn roots(git: &git::Repo, refs: &Refs) -> Result<BTreeSet<GitId>, ImportError> {
    let mut roots: BTreeSet<GitId> = refs.branches.iter().map(|(_, id)| *id).collect();
    roots.extend(refs.servers.iter().map(|(_, _, id)| *id));
    if let Head::Detached(id) = &refs.head {
        roots.insert(*id);
    }
    // Where the chain from each tag object read ends: at a commit, or at nothing Bolt can tag.
    let mut ends: HashMap<GitId, Option<GitId>> = HashMap::new();
    for (name, target) in &refs.tags {
        let mut read = Vec::new();
        let mut id = *target;
        let end = loop {
            if let Some(end) = ends.get(&id) {
                break *end;
            }
            let hex = id.to_hex();
            let here = || format!("tag `{}`, at object `{hex}`", shown(name));
            let (kind, size) = git.header(&id).map_err(|e| at(e, here))?;
            match kind {
                ObjectType::Commit => break Some(id),
                ObjectType::Tree | ObjectType::Blob => break None,
                // `Converter::tag` refuses it, saying why.
                ObjectType::Tag if size > MAX_GIT_TAG_OBJECT_BYTES as u64 => break None,
                ObjectType::Tag => {}
            }
            let object = git.read(&id).map_err(|e| at(e, here))?;
            expect_kind(object.kind, ObjectType::Tag, &id, here)?;
            read.push(id);
            // Never round a loop: each tag object's ID covers the ID of the next.
            id = parse_tag(&object.data, git.hash())
                .map_err(|e| at(e, here))?
                .object;
        };
        for object in read {
            ends.insert(object, end);
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
/// already. Their encoders don't check what they write, so `read_back` decodes it first.
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
    if encoded.len() as u64 > MAX_RECORD_SIZE {
        return Err(ImportError::Stop {
            what: format!(
                "Bolt's {what} can't hold this repository's refs: it would be more than the 512 \
                 MiB a record holds."
            ),
            hint: Some(
                "Nothing was changed. Delete refs you don't need, such as stale server branches \
                 (`git remote prune <server>`), then run bolt import again."
                    .to_owned(),
            ),
        });
    }
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

/// A warning for each folder named `.bolt` that Git checked out below the top, from the tree of
/// `HEAD`'s commit: Bolt left them out of history, but commands run inside one would take it for
/// a project (decision 019).
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
            // At the top it is the project's own folder, checked before anything was made.
            if stack.len() == 1 {
                continue;
            }
            count = count.saturating_add(1);
            if warnings.len() < MAX_NAMED_BOLT_FOLDERS {
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
        warnings.push(Warning::new(
            warning::NESTED_BOLT_FOLDER,
            format!(
                "Git checked out {more} more folders named .bolt. Bolt left them out of history \
                 too; delete them unless you made them."
            ),
        ));
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
    use proptest::prelude::*;

    fn who() -> Person {
        Person {
            name: "Ada".into(),
            email: "ada@example.com".into(),
            time_ms: 0,
            tz_offset_min: 0,
        }
    }

    /// `rename` over `names`, given in any order.
    fn renamed(names: &[&[u8]], rules: Rules) -> Vec<String> {
        let mut sorted = names.to_vec();
        sorted.sort();
        rename(&sorted, rules)
    }

    #[test]
    fn the_renaming_rule() {
        use Rules::{Line, Tag};
        // Taken in byte order, so `Feature` keeps its name and `feature` gets the next free one.
        assert_eq!(
            renamed(&[b"main", b"feature+x", b"feature", b"Feature"], Line),
            ["Feature", "feature-2", "feature-x", "main"]
        );
        // A suffixed name must be free too.
        assert_eq!(renamed(&[b"X", b"X-2", b"x"], Line), ["X", "X-2", "x-3"]);
        // Bytes that aren't UTF-8 become `-` wherever U+FFFD would stand for them; letters of
        // any script stay.
        assert_eq!(
            renamed(
                &[
                    b"a\xff\xfeb",
                    b"a\xe2\x82b",
                    "caf\u{e9}/e\u{301}#1".as_bytes()
                ],
                Line
            ),
            ["a-b", "a--b", "caf\u{e9}/e--1"]
        );
        // Cut at a character, and never left ending in `/`.
        let long = "\u{e9}".repeat(200);
        let slashed = format!("{}/{}", "a".repeat(254), "b".repeat(10));
        let cut = renamed(&[long.as_bytes(), slashed.as_bytes()], Line);
        assert_eq!(cut, ["a".repeat(254), "\u{e9}".repeat(127)]);
        // A suffix still fits in 255 bytes.
        let (one, two) = ("a".repeat(300), format!("{}b", "a".repeat(299)));
        let alike = renamed(&[one.as_bytes(), two.as_bytes()], Line);
        assert_eq!(alike, ["a".repeat(255), format!("{}-2", "a".repeat(253))]);
        // Tags keep any UTF-8, and only names that are the same clash.
        assert_eq!(
            renamed(&[b"V1", b"v1", b"a#b@c", b"v\xff", b"v-"], Tag),
            ["V1", "a#b@c", "v-", "v1", "v--2"]
        );
        // Many names alike each get the next number, without counting up from 2 each time.
        let alike: Vec<Vec<u8>> = (0x80..=0xffu8).map(|b| vec![b'x', b]).collect();
        let alike: Vec<&[u8]> = alike.iter().map(Vec::as_slice).collect();
        let numbered = renamed(&alike, Line);
        assert_eq!(numbered.first().map(String::as_str), Some("x-"));
        assert_eq!(numbered.last().map(String::as_str), Some("x--128"));
    }

    /// A Git ref name's level: pieces Bolt allows, refuses, folds or can't read.
    fn level() -> impl Strategy<Value = Vec<u8>> {
        let piece = prop::sample::select(vec![
            &b"a"[..],
            b"A",
            b"0",
            b"+",
            b"#",
            b"@",
            b"-",
            "\u{e9}".as_bytes(),
            "\u{c9}".as_bytes(),
            b"\xff",
        ]);
        prop::collection::vec(piece, 1..90).prop_map(|pieces| pieces.concat())
    }

    proptest! {
        /// Whatever Git names come in, Bolt's come out valid, unlike but for letter case, and the
        /// same every time.
        #[test]
        fn renamed_lines_follow_bolts_rules(
            names in prop::collection::btree_set(
                prop::collection::vec(level(), 1..4).prop_map(|levels| levels.join(&b'/')),
                1..20,
            )
        ) {
            let names: Vec<&[u8]> = names.iter().map(Vec::as_slice).collect();
            let bolt = rename(&names, Rules::Line);
            let mut folded = HashSet::new();
            for name in &bolt {
                prop_assert!(crate::diary::validate_line_name(name).is_ok(), "{name}");
                prop_assert!(folded.insert(name.to_lowercase()), "{name}");
            }
            prop_assert_eq!(rename(&names, Rules::Line), bolt);
        }
    }

    #[test]
    fn refs_past_bolts_limits_stop_the_import() {
        let id = GitId::from_hex(HashKind::Sha1, &"1".repeat(40)).unwrap();
        let refs = |servers: usize, tags: usize| Refs {
            head: Head::Detached(id),
            branches: Vec::new(),
            servers: (0..servers)
                .map(|n| (format!("s{n:03}").into_bytes(), b"main".to_vec(), id))
                .collect(),
            tags: (0..tags)
                .map(|n| (format!("t{n:06}").into_bytes(), id))
                .collect(),
            other: Vec::new(),
        };
        assert!(Names::of(&refs(64, 100_000)).is_ok());
        for (refs, limit) in [(refs(65, 0), "64"), (refs(0, 100_001), "100,000 tags")] {
            match Names::of(&refs) {
                Err(ImportError::Git(e @ GitError::TooManyRefs { .. })) => {
                    assert!(e.to_string().contains(limit), "{e}");
                }
                other => panic!("{:?}", other.map(|_| ())),
            }
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
        // Every commit, and what the first one holds, in one pack; the rest in another.
        git.pack(&[
            PackEntry::Whole(Commit, first.clone().into_bytes()),
            PackEntry::Whole(Commit, second.clone().into_bytes()),
            PackEntry::Whole(Tree, tree1),
            PackEntry::Whole(Blob, one),
        ]);
        let (later, _) = git.pack(&[PackEntry::Whole(Tree, tree2), PackEntry::Whole(Blob, two)]);
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
