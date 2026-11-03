//! Turning Git's objects into Bolt's (decisions 032, 035, 036 and 037): blobs into content, trees
//! into folder listings, commits into saves and tags into Bolt tags. Each Git object is converted
//! once, and the same repository always gives the same saves.
#![deny(clippy::indexing_slicing, clippy::arithmetic_side_effects)]

use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::io;

use encoding_rs::{Encoding, UTF_8};

use super::ImportError;
use crate::diary::{
    self, MAX_GIT_TAG_OBJECT_BYTES, MAX_GIT_TAG_OBJECTS, MAX_TAG_MESSAGE_BYTES, MAX_TAGS, TagList,
};
use crate::encoding::DecodeError;
use crate::git::{
    self, Commit, GitError, GitId, ObjectType, is_lfs_pointer, parse_commit, parse_person,
    parse_tag, parse_tree, rebuild_commit, shown,
};
use crate::id::{ChangeId, ContentHasher, Id, hash_content, hash_record};
use crate::link::ProjectLink;
use crate::listing::{Entry, EntryKind, Listing, SaveLimits};
use crate::names::{NameError, is_git_alias, validate_name};
use crate::pack::{MAX_RECORD_SIZE, ObjectKind, PackError, PackSet};
use crate::save::{
    EXTRA_GIT_COMMIT, EXTRA_GIT_HEADER, EXTRA_GIT_ORIGINAL_MESSAGE, MAX_EXTRA_VALUE_BYTES,
    MAX_MESSAGE_BYTES, Save,
};
use crate::store::Store;

/// Blobs up to this size are read whole; bigger ones stream through, twice.
const MAX_WHOLE_BLOB: u64 = 64 << 20;

/// The biggest commit a save can keep exactly: its header lines, the empty line and its message
/// (decision 032).
const MAX_COMMIT_BYTES: u64 = MAX_EXTRA_VALUE_BYTES as u64 + 1 + MAX_MESSAGE_BYTES as u64;

/// How many skipped entries, or names that aren't UTF-8, are listed by path. The rest are counted.
const MAX_LISTED: usize = 100;

/// About the most bytes of a path a list or message holds.
const MAX_SHOWN_PATH: usize = 1024;

const BUG_HINT: &str = "Nothing was changed. This is a bug in Bolt, not a problem with the \
                        repository.";

/// A stop that only a bug in Bolt can cause.
fn bug(what: String) -> ImportError {
    ImportError::Stop {
        what: format!("{what}; please report this."),
        hint: Some(BUG_HINT.to_owned()),
    }
}

fn unimportable(what: String) -> ImportError {
    GitError::Unimportable { what }.into()
}

/// `e`, saying where it was met: Git's parsers can't name the object they refuse.
fn at(e: GitError, place: impl FnOnce() -> String) -> ImportError {
    match e {
        GitError::Broken { what } => GitError::Broken {
            what: format!("{what}, in {}", place()),
        },
        GitError::Unimportable { what } => GitError::Unimportable {
            what: format!("{what}, in {}", place()),
        },
        other => other,
    }
    .into()
}

/// Refuses an object found to be of another kind than the one it was named as.
fn expect_kind(
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
/// many levels of folders are below it, and whether an export gives its Git ID back.
#[derive(Clone, Copy)]
struct Folder {
    listing: Id,
    total: u64,
    depth: usize,
    exact: bool,
}

/// A folder on the walk's stack, with the entries of its Git tree still to convert.
struct Frame {
    git: GitId,
    name: String,
    left: std::vec::IntoIter<Item>,
    entries: Vec<Entry>,
    total: u64,
    depth: usize,
    exact: bool,
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

/// Converts one Git repository's objects into one command's packs. The packs only know the
/// objects of the pack being written, and the store only what was committed before, so memo
/// tables keep each Git object to one conversion, and a set of the records stored keeps a record
/// two Git objects make alike to one copy.
pub struct Converter<'r> {
    git: &'r git::Repo,
    store: &'r Store,
    packs: &'r mut PackSet,
    /// Each blob's content ID and size.
    blobs: HashMap<GitId, (Id, u64)>,
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
    /// What the tag list will hold so far, at most, and the most it may: its record's limit,
    /// which tests lower.
    tag_bytes: u64,
    tag_budget: u64,
}

impl<'r> Converter<'r> {
    #[must_use]
    pub fn new(git: &'r git::Repo, store: &'r Store, packs: &'r mut PackSet) -> Converter<'r> {
        Converter {
            git,
            store,
            packs,
            blobs: HashMap::new(),
            links: HashMap::new(),
            trees: HashMap::new(),
            saves: HashMap::new(),
            records: HashSet::new(),
            counts: Counts::default(),
            not_utf8: Vec::new(),
            not_utf8_count: 0,
            chains: HashMap::new(),
            tag_bytes: TagList::head_len(MAX_TAGS),
            tag_budget: MAX_RECORD_SIZE,
        }
    }

    #[must_use]
    pub fn counts(&self) -> &Counts {
        &self.counts
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

    /// Stores the save of the commit `id`, with its snapshot, given its parents' saves in the
    /// commit's own order (decision 032). A commit converted already gives the same save.
    pub fn commit(&mut self, id: &GitId, parent_saves: &[Id]) -> Result<Id, ImportError> {
        if let Some(save) = self.saves.get(id) {
            return Ok(*save);
        }
        let hex = id.to_hex();
        let here = || format!("commit `{hex}`");
        // The header isn't checked against the ID, but it bounds what reading would hold.
        let (kind, size) = self.git.header(id).map_err(|e| at(e, here))?;
        expect_kind(kind, ObjectType::Commit, id, here)?;
        if size > MAX_COMMIT_BYTES {
            return Err(unimportable(format!(
                "commit `{hex}` is {size} bytes, more than any commit Bolt can keep (64 KiB of \
                 header lines and 1 MiB of message)"
            )));
        }
        let object = self.git.read(id).map_err(|e| at(e, here))?;
        expect_kind(object.kind, ObjectType::Commit, id, here)?;
        let c = parse_commit(&object.data, self.git.hash()).map_err(|e| at(e, here))?;
        keepable(&c, &hex)?;
        if parent_saves.len() != c.parents.len() {
            return Err(bug(format!(
                "Bolt was given {} saves for the {} parents of commit `{hex}`",
                parent_saves.len(),
                c.parents.len()
            )));
        }
        // Another commit's save in a parent's place would export as another commit.
        for (save, git) in parent_saves.iter().zip(&c.parents) {
            if self.saves.get(git).is_some_and(|own| own != save) {
                return Err(bug(format!(
                    "Bolt was given another save than its own for parent `{}` of commit `{hex}`",
                    git.to_hex()
                )));
            }
        }
        // Git may list a parent twice and a save can't; `git-header` keeps the commit exact.
        let (mut parents, mut git_parents) = (Vec::new(), Vec::new());
        for (save, git) in parent_saves.iter().zip(&c.parents) {
            if !parents.contains(save) {
                parents.push(*save);
                git_parents.push(*git);
            }
        }
        let root = self.snapshot(&c.tree, &hex)?;
        let message = message_text(c.message, c.encoding);
        let mut extra = BTreeMap::from([(EXTRA_GIT_COMMIT.to_owned(), hex.clone().into_bytes())]);
        if message.as_bytes() != c.message {
            extra.insert(EXTRA_GIT_ORIGINAL_MESSAGE.to_owned(), c.message.to_vec());
        }
        let mut save = Save {
            root: root.listing,
            parents,
            author: parse_person(c.author),
            saver: parse_person(c.committer),
            message: message.into_owned(),
            change_id: ChangeId::from_git_commit(id.as_bytes(), c.change_id),
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
    /// counts toward the 512 MiB one tag list holds, so the tags returned hold at most that, and
    /// reading holds one chain more, at most 16 MiB. The converter keeps no tag's bytes.
    pub fn tag(&mut self, name: &str, target: &GitId) -> Result<TagOutcome, ImportError> {
        // The least this tag adds to the list: its name and save, without a message.
        let lightweight = diary::Tag {
            save: Id([0; 32]),
            message: String::new(),
            git_tags: Vec::new(),
        };
        let least = TagList::tag_len(name, &lightweight);
        let full = || ImportError::Stop {
            what: format!(
                "Bolt's tag list can't hold the Git tags: with tag `{name}`, their names, \
                 messages and tag objects come to more than the 512 MiB it holds."
            ),
            hint: Some(
                "Nothing was changed. Delete tags you don't need with `git tag -d <name>`, then \
                 run bolt import again."
                    .to_owned(),
            ),
        };
        // Checked before anything is read, so a spent budget reads no more tags.
        if self.tag_bytes.saturating_add(least) > self.tag_budget {
            return Err(full());
        }
        let outcome = self.read_tag(name, target)?;
        if let TagOutcome::Tag(tag) = &outcome {
            let total = self.tag_bytes.saturating_add(TagList::tag_len(name, tag));
            if total > self.tag_budget {
                return Err(full());
            }
            self.tag_bytes = total;
        }
        Ok(outcome)
    }

    /// Follows the tag `name` from `target` to what it tags. A tag object met before tells where
    /// its chain ends: one that ends at a tree or blob isn't read again, and one that ends at a
    /// commit is, for the bytes the tag keeps.
    fn read_tag(&mut self, name: &str, target: &GitId) -> Result<TagOutcome, ImportError> {
        let too_long = || {
            unimportable(format!(
                "tag `{name}` reaches its commit through more than {MAX_GIT_TAG_OBJECTS} tag \
                 objects, more than Bolt keeps"
            ))
        };
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
                if (read.len() as u64).saturating_add(known.objects) > MAX_GIT_TAG_OBJECTS {
                    return Err(too_long());
                }
                if known.end != ObjectType::Commit {
                    self.remember(&read, known.objects, known.end);
                    return Ok(skipped(known.end));
                }
            }
            let hex = id.to_hex();
            // The ref's own target may be a commit, or missing, as well as a tag object.
            let here = || format!("tag `{name}`, at object `{hex}`");
            let too_big = |size: u64| {
                unimportable(format!(
                    "tag `{name}` has a tag object of {size} bytes (`{hex}`), more than the 1 \
                     MiB Bolt keeps of one"
                ))
            };
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
                if size > MAX_GIT_TAG_OBJECT_BYTES as u64 {
                    return Err(too_big(size));
                }
            }
            if read.len() as u64 >= MAX_GIT_TAG_OBJECTS {
                return Err(too_long());
            }
            let object = self.git.read(&id).map_err(|e| at(e, here))?;
            expect_kind(object.kind, ObjectType::Tag, &id, here)?;
            if object.data.len() > MAX_GIT_TAG_OBJECT_BYTES {
                return Err(too_big(object.data.len() as u64));
            }
            let tag = parse_tag(&object.data, self.git.hash()).map_err(|e| at(e, here))?;
            if read.is_empty() {
                message = message_text(tag.message, None).into_owned();
                message.truncate(message.floor_char_boundary(MAX_TAG_MESSAGE_BYTES));
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
                    let entry = self.leaf(item, &stack, commit)?;
                    if let Some(top) = stack.last_mut() {
                        top.entries.push(entry);
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
        // The top folder is level 0.
        let level = stack.len();
        if let Some(done) = self.trees.get(id).copied() {
            walk.depth(level.saturating_add(done.depth))?;
            walk.add(done.total)?;
            return Ok(attach(stack, name, done));
        }
        // Checked before the tree is read, so a deep one stops at the limit.
        walk.depth(level)?;
        stack.push(Frame {
            git: *id,
            name,
            left: Vec::new().into_iter(),
            entries: Vec::new(),
            total: 0,
            depth: 0,
            exact: true,
        });
        self.read_folder(stack, walk)?;
        Ok(None)
    }

    /// Reads the Git tree of the folder on top of the stack and judges its names: those that
    /// aren't UTF-8 are left out and listed, for `finish` to stop at (decision 019), dangerous
    /// ones and spellings of `.bolt` skipped (decisions 035 and 019), and one too long stops the
    /// import.
    fn read_folder(&mut self, stack: &mut [Frame], walk: &mut Walk<'_>) -> Result<(), ImportError> {
        let Some(id) = stack.last().map(|top| top.git) else {
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
        for e in entries {
            exact &= e.standard_mode;
            let Ok(name) = std::str::from_utf8(e.name) else {
                exact = false;
                self.not_utf8(commit, || path_of(stack, &[shown(e.name).as_str()]));
                continue;
            };
            match validate_name(name) {
                Ok(()) if !is_git_alias(name) => items.push(Item {
                    name: name.to_owned(),
                    kind: e.kind,
                    id: e.id,
                }),
                Err(NameError::TooLong) => {
                    return Err(unimportable(format!(
                        "commit `{commit}` has a name longer than 255 bytes, more than Bolt \
                         allows: `{}`",
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
            exact: frame.exact,
        };
        self.trees.insert(frame.git, done);
        Ok((frame.name, done))
    }

    /// The entry of a file, symlink or submodule in the folder on top of the stack.
    fn leaf(&mut self, item: Item, stack: &[Frame], commit: &str) -> Result<Entry, ImportError> {
        let (id, size) = if item.kind == EntryKind::ProjectLink {
            (self.link(&item.id)?, 0)
        } else {
            self.blob(&item.id, commit, &|| path_of(stack, &[item.name.as_str()]))?
        };
        Ok(Entry {
            name: item.name,
            kind: item.kind,
            id,
            size,
        })
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

    /// A blob's content ID and size, stored the first time it is met. `path` says where, for
    /// messages. A Git LFS pointer stops the import (decision 037).
    fn blob(
        &mut self,
        id: &GitId,
        commit: &str,
        path: &dyn Fn() -> String,
    ) -> Result<(Id, u64), ImportError> {
        if let Some(done) = self.blobs.get(id) {
            return Ok(*done);
        }
        let place = || format!("the file `{}` of commit `{commit}`", path());
        // The size in the header only picks how to read it: the bytes are checked against the ID.
        let (_, size) = self.git.header(id).map_err(|e| at(e, place))?;
        let done = if size <= MAX_WHOLE_BLOB {
            let object = self.git.read(id).map_err(|e| at(e, place))?;
            expect_kind(object.kind, ObjectType::Blob, id, place)?;
            if is_lfs_pointer(&object.data) {
                return Err(ImportError::LfsPointer {
                    commit: commit.to_owned(),
                    path: path(),
                });
            }
            let size = object.data.len() as u64;
            (self.put(ObjectKind::Content, &object.data)?, size)
        } else {
            self.stream(id, &place)?
        };
        self.blobs.insert(*id, done);
        Ok(done)
    }

    /// Stores a blob too big to read whole: one pass learns its content ID, and a second stores
    /// it unless a good copy is stored already.
    fn stream(&mut self, id: &GitId, place: &dyn Fn() -> String) -> Result<(Id, u64), ImportError> {
        let failed = |e: io::Error| match e.downcast::<GitError>() {
            Ok(e) => at(e, place),
            Err(e) => PackError::Io(e).into(),
        };
        let mut hasher = ContentHasher::new();
        let mut blob = self.git.blob_reader(id).map_err(|e| at(e, place))?;
        let size = io::copy(&mut blob, &mut Hashing(&mut hasher)).map_err(failed)?;
        let content = hasher.finish();
        if !self.packs.contains(&content)
            && !self
                .store
                .has_good_copy(&content, ObjectKind::Content, size)?
        {
            let mut blob = self.git.blob_reader(id).map_err(|e| at(e, place))?;
            match self.packs.add_content_from(&mut blob, content) {
                Err(PackError::Io(e)) => return Err(failed(e)),
                other => other?,
            };
        }
        Ok((content, size))
    }

    /// Stores an object unless this converter stored it already, this pack holds it or the store
    /// has a good copy, and returns its ID.
    fn put(&mut self, kind: ObjectKind, bytes: &[u8]) -> Result<Id, ImportError> {
        let id = match kind.flavor() {
            Some(flavor) => hash_record(flavor, bytes),
            None => hash_content(bytes),
        };
        // Content is stored once through its blob's Git ID; a record can come from two Git
        // objects, such as two trees whose modes Bolt reads alike.
        let record = kind != ObjectKind::Content;
        if record && self.records.contains(&id) {
            return Ok(id);
        }
        if !self.packs.contains(&id) && !self.store.has_good_copy(&id, kind, bytes.len() as u64)? {
            if record {
                self.packs.add_record(kind, bytes)?;
            } else {
                self.packs.add_content(bytes)?;
            }
        }
        if record {
            self.records.insert(id);
        }
        Ok(id)
    }
}

/// Adds a finished folder to the folder above it, or returns it if it is the top one.
fn attach(stack: &mut [Frame], name: String, done: Folder) -> Option<Folder> {
    let Some(parent) = stack.last_mut() else {
        return Some(done);
    };
    parent.entries.push(Entry {
        name,
        kind: EntryKind::Folder,
        id: done.listing,
        size: 0,
    });
    parent.total = parent.total.saturating_add(done.total);
    parent.depth = parent.depth.max(done.depth.saturating_add(1));
    parent.exact &= done.exact;
    None
}

/// The path of `names` in the folder on top of the stack, `/` between names, for a list or a
/// message. A deep one keeps only the whole names at its end that fit in `MAX_SHOWN_PATH`, after
/// `…/`: a tree 4,096 levels deep would otherwise make each path a megabyte.
fn path_of(stack: &[Frame], names: &[&str]) -> String {
    let folders = stack.iter().skip(1).map(|frame| frame.name.as_str());
    let parts: Vec<&str> = folders.chain(names.iter().copied()).collect();
    let mut start = parts.len();
    let mut len = 0usize;
    while let Some(part) = start.checked_sub(1).and_then(|i| parts.get(i)) {
        len = len.saturating_add(part.len()).saturating_add(1);
        // The last name is kept whatever its length; callers cut names to 255 bytes.
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
    use crate::git::craft::{Craft, PackEntry};
    use crate::git::{HashKind, object_id};
    use crate::pack::{PackReader, WRITER_PACK_OBJECTS};
    use crate::save::Person;
    use crate::test_support::{HOSTILE, assert_hostile_text_escaped};
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
        let result = run(&mut Converter::new(&git, &store, &mut packs));
        packs.finish().unwrap();
        (result, Store::open(&packs_dir).unwrap())
    }

    /// How many objects of `kind` the store at `dir/<name>` holds, in all its packs.
    fn stored(dir: &Path, name: &str, kind: ObjectKind) -> usize {
        let packs = fs::read_dir(dir.join(name).join("packs")).unwrap();
        packs
            .map(|pack| PackReader::open(&pack.unwrap().path()).unwrap())
            .map(|pack| pack.entries().iter().filter(|e| e.kind == kind).count())
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
        assert_eq!(ChangeId::from_git_commit(&id, None).0.to_vec(), expected);
        assert_eq!(
            ChangeId::from_git_commit(&id, Some(b"zyxwvutsrqponmlkzyxwvutsrqponmlk")).0,
            [
                0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, 0x01, 0x23, 0x45, 0x67, 0x89, 0xab,
                0xcd, 0xef
            ]
        );
        // Jujutsu reads its letters in either case.
        assert_eq!(
            ChangeId::from_git_commit(&id, Some(b"ZYXWVUTSRQPONMLKzyxwvutsrqponmlk")).0[..2],
            [0x01, 0x23]
        );
        // The last 16 bytes count, so a SHA-256 ID's first 16 don't.
        let sha256: Vec<u8> = (0..32).collect();
        let want: Vec<u8> = (16..32u8).rev().map(u8::reverse_bits).collect();
        assert_eq!(ChangeId::from_git_commit(&sha256, None).0.to_vec(), want);
        // A header that isn't 16 bytes in Jujutsu's letters is passed over, as Jujutsu does.
        for header in [
            &b"zyxwvutsrqponmlk"[..],
            b"zyxwvutsrqponmlkzyxwvutsrqponmlkzz",
            b"zyxwvutsrqponmlkzyxwvutsrqponmlj",
            b"zyxwvutsrqponmlkzyxwvutsrqponml ",
            b"",
        ] {
            assert_eq!(
                ChangeId::from_git_commit(&id, Some(header)).0.to_vec(),
                expected,
                "{}",
                String::from_utf8_lossy(header)
            );
        }
    }

    #[test]
    fn blobs_are_stored_and_big_ones_streamed() {
        let (dir, git) = repo(Sha1);
        let small = b"hello\n".to_vec();
        // One byte past what is read whole.
        let big: Vec<u8> = (0..(64usize << 20) + 1).map(|i| (i % 251) as u8).collect();
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
        let (save, store) = convert(dir.path(), "a", |c| {
            let save = c.commit(&first, &[]).unwrap();
            c.commit(&second, &[]).unwrap();
            save
        });
        let root = store
            .listing(&store.save(&save, false).unwrap().root)
            .unwrap();
        let entry = |name: &str| root.get(name).unwrap().clone();
        assert_eq!(entry("small").size, 6);
        assert_eq!(content(&store, &entry("small").id), small);
        assert_eq!(entry("big").size, big.len() as u64);
        assert!(content(&store, &entry("big").id) == big);
        // Each blob is stored once, however often it comes.
        assert_eq!(entry("small-again").id, entry("small").id);
        assert_eq!(stored(dir.path(), "a", ObjectKind::Content), 2);
        // A store that holds them already gets neither again.
        convert(dir.path(), "a", |c| c.commit(&first, &[]).unwrap());
        assert_eq!(stored(dir.path(), "a", ObjectKind::Content), 2);
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
                let save = c.commit(&id, &[]).unwrap();
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
            (c.commit(&id, &[]).unwrap(), c.counts().clone())
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
            let save = c.commit(&first, &[]).unwrap();
            c.commit(&second, &[]).unwrap();
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
            (c.commit(&id, &[]).unwrap(), c.counts().clone())
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
        let longest = "y".repeat(255);
        let fine = commit_of(&git, &[(b"100644", longest.as_bytes(), file)]);
        let too_long = "x".repeat(256);
        let deep = git.tree(&[(b"100644", too_long.as_bytes(), file)]);
        let id = commit_of(&git, &[(b"40000", b"deep", deep)]);
        let ((fine, long), _) = convert(dir.path(), "a", |c| {
            (c.commit(&fine, &[]), c.commit(&id, &[]))
        });
        assert!(fine.is_ok(), "{fine:?}");
        // The name is shown cut, so a hostile one can't flood the message.
        let shown = format!("deep/{}...", "x".repeat(255));
        assert_refused(long, &[&id.to_hex(), "255 bytes", &shown]);
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
            let one = c.commit(&first, &[]).unwrap();
            let two = c.commit(&second, &[one]).unwrap();
            let three = c.commit(&third, &[two]).unwrap();
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
            c.commit(&id, &[]).unwrap();
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
        // Each under 20 levels of 255-byte folder names: about 5 KB of path above it.
        let folder = "d".repeat(255);
        let deep = |name: &[u8]| {
            let mut tree = git.tree(&[(b"100644", name, file)]);
            for _ in 0..20 {
                tree = git.tree(&[(b"40000", folder.as_bytes(), tree)]);
            }
            git.commit(&commit_bytes(&tree, &[], "", b"m\n"))
        };
        let (skipped, bad, too_long) = (deep(&slashed), deep(&not_utf8), deep(&long));
        let ((counts, finished, stopped), _) = convert(dir.path(), "a", |c| {
            c.commit(&skipped, &[]).unwrap();
            c.commit(&bad, &[]).unwrap();
            (c.counts().clone(), c.finish(), c.commit(&too_long, &[]))
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
        let (result, _) = convert(dir.path(), "a", |c| c.commit(&id, &[]));
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
        let (result, _) = convert(dir.path(), "a", |c| c.commit(&id, &[]));
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

    #[test]
    fn a_deep_tree_stops_at_4096_without_running_out_of_stack() {
        let (dir, git) = repo(Sha1);
        // Tree `k` holds `k` levels of folders; one pack holds them all.
        let mut trees = vec![(Vec::new(), object_id(ObjectType::Tree, b"", Sha1).unwrap())];
        for _ in 0..4097 {
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
        let at_limit = git.commit(&commit_bytes(&trees[4096].1, &[], "", b"m\n"));
        let past = git.commit(&commit_bytes(&trees[4097].1, &[], "", b"m\n"));
        let path = dir.path().to_owned();
        // A walk that recursed once a level would need far more stack than this.
        let (fresh, fine, known) = std::thread::Builder::new()
            .stack_size(256 << 10)
            .spawn(move || {
                convert(&path, "a", |c| {
                    // Read level by level the first time, then from what was converted since.
                    let fresh = c.commit(&past, &[]).map(drop);
                    let fine = c.commit(&at_limit, &[]).map(drop);
                    (fresh, fine, c.commit(&past, &[]).map(drop))
                })
                .0
            })
            .unwrap()
            .join()
            .unwrap();
        assert!(fine.is_ok(), "{fine:?}");
        assert_refused(fresh, &[&past.to_hex(), "more than 4096 levels"]);
        assert_refused(known, &[&past.to_hex(), "more than 4096 levels"]);
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
            let first = c.commit(&first, &[]).unwrap();
            let other = c.commit(&other, &[]).unwrap();
            [first, other, c.commit(&merge, &[first, other]).unwrap()]
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
        assert_eq!(
            save.change_id,
            ChangeId::from_git_commit(merge.as_bytes(), None)
        );
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
    fn parent_saves_must_be_the_parents_own() {
        let (dir, git) = repo(Sha1);
        let tree = git.tree(&[]);
        let first = git.commit(&commit_bytes(&tree, &[], "", b"first\n"));
        let other = git.commit(&commit_bytes(&tree, &[], "", b"other\n"));
        let child = git.commit(&commit_bytes(&tree, &[first], "", b"child\n"));
        // Another commit's save in a parent's place would export as another commit.
        let (wrong, _) = convert(dir.path(), "a", |c| {
            c.commit(&first, &[]).unwrap();
            let other = c.commit(&other, &[]).unwrap();
            c.commit(&child, &[other])
        });
        match wrong {
            Err(ImportError::Stop { what, .. }) => {
                assert!(what.contains(&child.to_hex()), "{what}");
                assert!(what.contains(&first.to_hex()), "{what}");
                assert!(what.contains("report"), "{what}");
            }
            other => panic!("{other:?}"),
        }
        // A parent this converter hasn't met is taken as given.
        let (unknown, _) = convert(dir.path(), "b", |c| c.commit(&child, &[Id([7; 32])]));
        assert!(unknown.is_ok(), "{unknown:?}");
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
            [standard, group_writable].map(|id| c.commit(&id, &[]).unwrap())
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
            let plain = c.commit(&plain, &[]).unwrap();
            [
                plain,
                c.commit(&signed_id, &[plain]).unwrap(),
                c.commit(&twice_id, &[plain, plain]).unwrap(),
                c.commit(&jj_id, &[]).unwrap(),
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
            let mut commits: Vec<(&str, GitId, &[usize])> = Vec::new();
            for case in &cases {
                let parents: Vec<GitId> = case.parents.iter().map(|&i| others[i]).collect();
                let raw = raw_commit(
                    &tree.to_hex(),
                    &parents,
                    &case.people,
                    case.headers,
                    case.message,
                );
                commits.push((case.what, git.commit(&raw), case.parents));
            }
            // A tree written in capitals, which Git reads too.
            let upper = raw_commit(
                &tree.to_hex().to_uppercase(),
                &[],
                PEOPLE.as_bytes(),
                "",
                b"m\n",
            );
            commits.push(("a tree ID in capitals", git.commit(&upper), &[]));
            let (saves, store) = convert(dir.path(), "a", |c| {
                let other_saves: Vec<Id> =
                    others.iter().map(|o| c.commit(o, &[]).unwrap()).collect();
                commits
                    .iter()
                    .map(|(what, id, parents)| {
                        let parents: Vec<Id> = parents.iter().map(|&i| other_saves[i]).collect();
                        c.commit(id, &parents)
                            .unwrap_or_else(|e| panic!("{what}: {e}"))
                    })
                    .collect::<Vec<Id>>()
            });
            for ((what, id, _), save) in commits.iter().zip(&saves) {
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
            commits.map(|id| c.commit(&id, &[]).unwrap())
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
            .map(|id| c.commit(&id, &[]))
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

    #[test]
    fn an_lfs_pointer_stops_the_import_naming_it() {
        let (dir, git) = repo(Sha1);
        let pointer = b"version https://git-lfs.github.com/spec/v1\noid \
                        sha256:4d7a214614ab2935c943f9e0ff69d22eadbb8f32b1258daaa5e2ca24d17e2393\n\
                        size 12345\n";
        let assets = git.tree(&[(b"100644", b"hero.psd", blob(&git, pointer))]);
        let id = commit_of(&git, &[(b"40000", b"assets", assets)]);
        let (result, _) = convert(dir.path(), "a", |c| c.commit(&id, &[]));
        let e = result.unwrap_err();
        match &e {
            ImportError::LfsPointer { commit, path } => {
                assert_eq!(
                    (commit.as_str(), path.as_str()),
                    (id.to_hex().as_str(), "assets/hero.psd")
                );
            }
            other => panic!("{other:?}"),
        }
        assert!(e.to_string().contains("assets/hero.psd"), "{e}");
        assert!(e.to_string().contains("milestone 5"), "{e}");
        let hint = e.hint().unwrap();
        assert!(hint.contains("git lfs migrate export"), "{hint}");
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
            let save = c.commit(&commit, &[]).unwrap();
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
            c.commit(&commit, &[]).unwrap();
            let before = c.tag_bytes;
            let first = c.tag("v1", &signed).unwrap();
            // Room for one more tag of the same chain, not two: each ref keeps its bytes.
            let each = c.tag_bytes - before;
            c.tag_budget = c.tag_bytes + each + each / 2;
            let again = c.tag("v1-again", &signed).unwrap();
            let refused = c.tag("v1-third", &signed);
            // Once the budget is spent, nothing more is read, not even an object that's missing.
            c.tag_budget = c.tag_bytes;
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
                let r = c.commit(&root, &[]).unwrap();
                let s = c.commit(&side, &[r]).unwrap();
                let m = c.commit(&main, &[r]).unwrap();
                [r, s, m, c.commit(&merge, &[m, s]).unwrap()]
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
            [lost, jumbled, tree_is_a_file, folder_is_a_file, a_tree].map(|id| c.commit(&id, &[]))
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
            ImportError::LfsPointer {
                commit: HOSTILE.into(),
                path: HOSTILE.into(),
            },
        ] {
            assert_hostile_text_escaped(&e.to_string());
        }
        // A name too long, from a real tree.
        let (dir, git) = repo(Sha1);
        let name = format!("{HOSTILE}{}", "x".repeat(300));
        let id = commit_of(&git, &[(b"100644", name.as_bytes(), blob(&git, b"x"))]);
        let (result, _) = convert(dir.path(), "a", |c| c.commit(&id, &[]));
        assert_hostile_text_escaped(&result.unwrap_err().to_string());
    }
}
