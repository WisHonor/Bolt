//! Git's refs: `HEAD`, the loose refs under `.git/refs/` and `.git/packed-refs` (decisions 031
//! and 033). Ref files are read as Git reads them and names follow its `check-ref-format`;
//! anything else is `Broken`.
#![deny(clippy::indexing_slicing, clippy::arithmetic_side_effects)]

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::Path;

use super::id::hex_id;
use super::{
    GitError, GitId, HashKind, broken, is_git_space, naming, read_error, read_optional, real_dir,
    shown, trim_end, trim_start,
};
use crate::diary::{MAX_LINES, MAX_SERVERS, MAX_TAGS};
use crate::names;
use crate::safe_fs;

/// `HEAD`, and each loose ref.
const MAX_REF_FILE_BYTES: u64 = 4096;
/// `packed-refs`: room for millions of refs, at some 70 bytes a line.
const MAX_PACKED_REFS_BYTES: u64 = 512 << 20;

/// How much reading refs may take: entries (folders and files under `.git/refs`, and refs in
/// `packed-refs`), and bytes of the names and paths it keeps meanwhile.
#[derive(Clone, Copy, Debug)]
pub(super) struct Limits {
    pub(super) entries: u64,
    pub(super) bytes: u64,
}

/// Room for every ref decision 033's limits let an import bring in (lines, each server's lines,
/// tags) and a million more it only lists, with names taking as much as packed-refs may.
pub(super) const LIMITS: Limits = Limits {
    entries: MAX_LINES * (1 + MAX_SERVERS) + MAX_TAGS + 1_000_000,
    bytes: MAX_PACKED_REFS_BYTES,
};

/// What reading refs has taken so far, against `limits`.
struct Taken {
    limits: Limits,
    entries: u64,
    bytes: u64,
}

impl Taken {
    /// One more entry read.
    fn entry(&mut self) -> Result<(), GitError> {
        self.entries = self.entries.saturating_add(1);
        if self.entries > self.limits.entries {
            return Err(GitError::TooManyRefs {
                what: format!(
                    "its .git/refs and packed-refs hold more than {} entries",
                    self.limits.entries
                ),
            });
        }
        Ok(())
    }

    /// `bytes` more of names and paths kept.
    fn keep(&mut self, bytes: usize) -> Result<(), GitError> {
        let bytes = u64::try_from(bytes).unwrap_or(u64::MAX);
        self.bytes = self.bytes.saturating_add(bytes);
        if self.bytes > self.limits.bytes {
            return Err(GitError::TooManyRefs {
                what: format!(
                    "the names of its refs and their folders take more than {} bytes",
                    self.limits.bytes
                ),
            });
        }
        Ok(())
    }
}

/// Where Git's `HEAD` points.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Head {
    /// A branch, by its name after `refs/heads/`. It may have no commits yet.
    Branch(Vec<u8>),
    Detached(GitId),
}

/// A repository's refs, each list in the byte order of the names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refs {
    pub head: Head,
    /// Names after `refs/heads/`.
    pub branches: Vec<(Vec<u8>, GitId)>,
    /// The server, then the name, after `refs/remotes/`.
    pub servers: Vec<(Vec<u8>, Vec<u8>, GitId)>,
    /// Names after `refs/tags/`.
    pub tags: Vec<(Vec<u8>, GitId)>,
    /// Whole names of the refs not imported, symbolic ones included, for the summary.
    pub other: Vec<Vec<u8>>,
}

/// What a ref file holds.
enum Contents<'a> {
    Id(GitId),
    /// The name of another ref, which Bolt doesn't follow.
    Symbolic(&'a [u8]),
}

/// The refs of the repository whose `.git` is `git`, taking at most `limits`.
pub(super) fn read(git: &Path, hash: HashKind, limits: Limits) -> Result<Refs, GitError> {
    let head = read_head(git, hash)?;
    let mut taken = Taken {
        limits,
        entries: 0,
        bytes: 0,
    };
    // Loose refs first: `git pack-refs` writes packed-refs before it deletes the loose ones, so
    // in this order no ref is missed while it moves.
    let loose = read_loose(git, hash, &mut taken)?;
    let path = git.join("packed-refs");
    let packed = match safe_fs::read_capped(&path, MAX_PACKED_REFS_BYTES) {
        Ok(text) => read_packed(&text, hash, &mut taken)?,
        Err(e) if e.kind() == io::ErrorKind::NotFound => BTreeMap::new(),
        // Past the cap: only millions of refs, or padding, make it that large.
        Err(e) if e.kind() == io::ErrorKind::InvalidData => {
            return Err(GitError::TooManyRefs {
                what: naming(&path, &e),
            });
        }
        Err(e) => return Err(read_error(&path, e)),
    };
    let mut all: BTreeMap<Vec<u8>, Option<GitId>> = packed
        .into_iter()
        .map(|(name, id)| (name, Some(id)))
        .collect();
    // A loose ref wins, as in Git: it is the newer.
    all.extend(loose);
    Ok(sort(head, all))
}

/// `all` maps each ref's name to its ID, or to `None` for a symbolic ref.
fn sort(head: Head, all: BTreeMap<Vec<u8>, Option<GitId>>) -> Refs {
    let mut refs = Refs {
        head,
        branches: Vec::new(),
        servers: Vec::new(),
        tags: Vec::new(),
        other: Vec::new(),
    };
    for (name, id) in all {
        let server_ref = name.strip_prefix(b"refs/remotes/").and_then(|rest| {
            let mut parts = rest.splitn(2, |&c| c == b'/');
            Some((parts.next()?.to_vec(), parts.next()?.to_vec()))
        });
        let Some(id) = id else {
            // A server's HEAD only names its default branch.
            if !server_ref.is_some_and(|(_, branch)| branch == b"HEAD") {
                refs.other.push(name);
            }
            continue;
        };
        if let Some(branch) = name.strip_prefix(b"refs/heads/") {
            refs.branches.push((branch.to_vec(), id));
        } else if let Some(tag) = name.strip_prefix(b"refs/tags/") {
            refs.tags.push((tag.to_vec(), id));
        } else if let Some((server, branch)) = server_ref {
            refs.servers.push((server, branch, id));
        } else {
            refs.other.push(name);
        }
    }
    refs
}

fn read_head(git: &Path, hash: HashKind) -> Result<Head, GitError> {
    let path = git.join("HEAD");
    let text = read_optional(&path, MAX_REF_FILE_BYTES)?
        .ok_or_else(|| broken(format!("{} is missing", path.display())))?;
    parse_head(&text, hash)
}

/// `HEAD` names a branch, or holds the commit it is detached at.
fn parse_head(text: &[u8], hash: HashKind) -> Result<Head, GitError> {
    let points_at = match parse_ref_file(text, hash) {
        // The all-zero ID names no commit.
        Some(Contents::Id(id)) if !id.is_zero() => return Ok(Head::Detached(id)),
        Some(Contents::Symbolic(target)) => {
            if let Some(branch) = target.strip_prefix(b"refs/heads/")
                && is_valid_ref_name(target)
            {
                return Ok(Head::Branch(branch.to_vec()));
            }
            target
        }
        _ => trim_end(text),
    };
    Err(broken(format!(
        "Git's HEAD points at `{}`, which isn't a branch",
        shown(points_at)
    )))
}

/// A ref file read as Git reads it (`parse_loose_ref_contents`, after `strbuf_rtrim`): `ref:`, any
/// whitespace and another ref's name; or an ID, then the end, or whitespace and anything at all
/// (FETCH_HEAD's form). Git would stop at a NUL; Bolt refuses one, as in config.
fn parse_ref_file(text: &[u8], hash: HashKind) -> Option<Contents<'_>> {
    if text.contains(&0) {
        return None;
    }
    let text = trim_end(text);
    if let Some(target) = text.strip_prefix(b"ref:") {
        return Some(Contents::Symbolic(trim_start(target)));
    }
    let (hex, rest) = text.split_at_checked(hash.hex_len())?;
    if rest.first().is_some_and(|&c| !is_git_space(c)) {
        return None;
    }
    hex_id(hash, hex).map(Contents::Id)
}

/// The loose refs under `.git/refs`, walked with a stack of folders, not recursion: each ref's ID,
/// or `None` for a symbolic ref. Each folder must be a real one and each ref a regular file; names
/// Git skips (`.` first, `.lock` last) are skipped. Every entry counts, and every name and path
/// kept.
fn read_loose(
    git: &Path,
    hash: HashKind,
    taken: &mut Taken,
) -> Result<BTreeMap<Vec<u8>, Option<GitId>>, GitError> {
    let root = git.join("refs");
    if !real_dir(&root)? {
        return Err(broken(format!("{} is missing", root.display())));
    }
    let mut refs = BTreeMap::new();
    let mut folders = vec![(root, b"refs".to_vec())];
    while let Some((folder, prefix)) = folders.pop() {
        safe_fs::check_real_dir(&folder).map_err(|e| read_error(&folder, e))?;
        for entry in fs::read_dir(&folder).map_err(|e| read_error(&folder, e))? {
            let entry = entry.map_err(|e| read_error(&folder, e))?;
            taken.entry()?;
            let file_name = entry.file_name();
            let name = file_name.as_encoded_bytes();
            if name.starts_with(b".") || name.ends_with(b".lock") {
                continue;
            }
            let full = [prefix.as_slice(), b"/", name].concat();
            // Looked at before anything opens it: on Windows `NUL` or `aux.c` opens a device,
            // and a name ending in a dot opens another file.
            if cfg!(windows) && !std::str::from_utf8(name).is_ok_and(names::holdable_here) {
                return Err(broken(format!(
                    "`{}` has a name Windows treats specially, so Bolt didn't open it",
                    shown(&full)
                )));
            }
            let path = entry.path();
            if entry
                .file_type()
                .map_err(|e| read_error(&path, e))?
                .is_dir()
            {
                // Its path and name wait on the stack until the walk gets to it.
                taken.keep(path.as_os_str().len().saturating_add(full.len()))?;
                folders.push((path, full));
                continue;
            }
            if !is_valid_ref_name(&full) {
                return Err(broken(format!(
                    "the ref `{}` has a name Git doesn't allow",
                    shown(&full)
                )));
            }
            let text = safe_fs::read_capped(&path, MAX_REF_FILE_BYTES)
                .map_err(|e| read_error(&path, e))?;
            let id = match parse_ref_file(&text, hash) {
                // Git takes a loose ref holding it for a broken one.
                Some(Contents::Id(id)) if id.is_zero() => {
                    return Err(broken(format!(
                        "the ref `{}` holds the all-zero ID, which names no commit",
                        shown(&full)
                    )));
                }
                Some(Contents::Id(id)) => Some(id),
                Some(Contents::Symbolic(_)) => None,
                None => {
                    return Err(broken(format!(
                        "the ref `{}` holds neither a commit ID nor another ref's name",
                        shown(&full)
                    )));
                }
            };
            taken.keep(full.len())?;
            refs.insert(full, id);
        }
    }
    Ok(refs)
}

/// `packed-refs`: an optional `# pack-refs with:` line, then `<id> <name>` lines, each maybe
/// followed by a `^<id>` line naming what it peels to, which Bolt ignores: it peels tags itself.
fn read_packed(
    text: &[u8],
    hash: HashKind,
    taken: &mut Taken,
) -> Result<BTreeMap<Vec<u8>, GitId>, GitError> {
    let mut refs = BTreeMap::new();
    if text.is_empty() {
        return Ok(refs);
    }
    let body = text
        .strip_suffix(b"\n")
        .ok_or_else(|| broken(".git/packed-refs ends partway through a line"))?;
    let bad = |n: usize| broken(format!("line {n} of .git/packed-refs isn't a ref"));
    let mut after_ref = false;
    for (line, n) in body.split(|&c| c == b'\n').zip(1..) {
        if n == 1 && line.starts_with(b"#") {
            if !line.starts_with(b"# pack-refs with: ") {
                return Err(bad(n));
            }
            continue;
        }
        if let Some(peeled) = line.strip_prefix(b"^") {
            if !after_ref || hex_id(hash, peeled).is_none() {
                return Err(bad(n));
            }
            after_ref = false;
            continue;
        }
        let (hex, rest) = line
            .split_at_checked(hash.hex_len())
            .ok_or_else(|| bad(n))?;
        let (Some(id), Some(name)) = (hex_id(hash, hex), rest.strip_prefix(b" ")) else {
            return Err(bad(n));
        };
        if !is_valid_ref_name(name) {
            return Err(broken(format!(
                "the ref `{}` in .git/packed-refs has a name Git doesn't allow",
                shown(name)
            )));
        }
        taken.entry()?;
        taken.keep(name.len())?;
        // An all-zero ID stays: Git's packed-refs reader takes it as it is, unlike a loose one.
        if refs.insert(name.to_vec(), id).is_some() {
            return Err(broken(format!(
                "the ref `{}` is in .git/packed-refs twice",
                shown(name)
            )));
        }
        after_ref = true;
    }
    Ok(refs)
}

/// Git's `check_refname_format` with no options: two levels at least, and no level that is
/// empty, starts with `.`, ends with `.lock`, or holds `..`, `@{`, a control character, a space,
/// or any of `:?[\^~*`; the name isn't `@` and doesn't end with `.`.
fn is_valid_ref_name(name: &[u8]) -> bool {
    let mut levels = 0usize;
    for level in name.split(|&c| c == b'/') {
        if level.is_empty() || level.starts_with(b".") || level.ends_with(b".lock") {
            return false;
        }
        let mut last = 0;
        for &c in level {
            let bad = match c {
                0..=0x1f | 0x7f | b' ' | b':' | b'?' | b'[' | b'\\' | b'^' | b'~' | b'*' => true,
                b'.' => last == b'.',
                b'{' => last == b'@',
                _ => false,
            };
            if bad {
                return false;
            }
            last = c;
        }
        levels = levels.saturating_add(1);
    }
    name != b"@" && !name.ends_with(b".") && levels >= 2
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    #![allow(clippy::indexing_slicing, clippy::arithmetic_side_effects)]
    use super::*;
    use crate::git::craft::Craft;
    use crate::git::{GitError, GitId, HashKind, Repo};

    #[test]
    fn a_nul_cannot_end_a_ref_name_early() {
        // A NUL can't end a name early, as it would in C.
        assert!(!is_valid_ref_name(b"refs/heads/a\0"));
    }

    const A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    const C: &str = "cccccccccccccccccccccccccccccccccccccccc";
    const D: &str = "dddddddddddddddddddddddddddddddddddddddd";
    const ZERO: &str = "0000000000000000000000000000000000000000";

    fn id(hex: &str) -> GitId {
        GitId::from_hex(HashKind::Sha1, hex).unwrap()
    }

    fn repo() -> (tempfile::TempDir, Craft) {
        let dir = tempfile::tempdir().unwrap();
        let git = Craft::new(dir.path(), HashKind::Sha1);
        (dir, git)
    }

    fn refs_of(dir: &tempfile::TempDir) -> Result<Refs, GitError> {
        Repo::open(dir.path()).unwrap().refs()
    }

    fn named(rows: &[(&str, &str)]) -> Vec<(Vec<u8>, GitId)> {
        rows.iter()
            .map(|(name, hex)| (name.as_bytes().to_vec(), id(hex)))
            .collect()
    }

    #[test]
    fn packed_refs_are_read_and_loose_refs_win() {
        let (dir, git) = repo();
        git.packed_refs(&format!(
            "# pack-refs with: peeled fully-peeled sorted \n\
             {A} refs/heads/main\n\
             {B} refs/heads/topic\n\
             {B} refs/pull/1/head\n\
             {A} refs/remotes/lonely\n\
             {C} refs/remotes/origin/main\n\
             {D} refs/tags/v1\n\
             ^{A}\n"
        ));
        git.loose_ref("refs/heads/main", &format!("{C}\n"));
        git.loose_ref(
            "refs/remotes/origin/HEAD",
            "ref: refs/remotes/origin/main\n",
        );
        let refs = refs_of(&dir).unwrap();
        assert_eq!(
            refs,
            Refs {
                head: Head::Branch(b"main".to_vec()),
                branches: named(&[("main", C), ("topic", B)]),
                servers: vec![(b"origin".to_vec(), b"main".to_vec(), id(C))],
                tags: named(&[("v1", D)]),
                // A remote ref with no server part isn't a server's branch.
                other: vec![
                    b"refs/pull/1/head".to_vec(),
                    b"refs/remotes/lonely".to_vec()
                ],
            }
        );
        // Without a header line, and in a SHA-256 repository.
        let dir = tempfile::tempdir().unwrap();
        let git = Craft::new(dir.path(), HashKind::Sha256);
        let long = "e".repeat(64);
        git.packed_refs(&format!("{long} refs/heads/main\n"));
        let refs = refs_of(&dir).unwrap();
        let main = GitId::from_hex(HashKind::Sha256, &long).unwrap();
        assert_eq!(refs.branches, [(b"main".to_vec(), main)]);
        // Git's packed-refs reader takes an all-zero ID as it is; so does Bolt's.
        let (dir, git) = repo();
        git.packed_refs(&format!("{ZERO} refs/tags/zero\n"));
        assert_eq!(refs_of(&dir).unwrap().tags, named(&[("zero", ZERO)]));
    }

    #[test]
    fn malformed_packed_refs_are_broken() {
        for text in [
            "# not the header\n".to_owned(),
            format!("{A} refs/heads/main"),
            format!("^{A}\n"),
            format!("{A} refs/heads/main\n^{B}\n^{C}\n"),
            format!("{A} refs/heads/main\n^{}\n", &B[1..]),
            format!("{} refs/heads/main\n", "z".repeat(40)),
            format!("{} refs/heads/main\n", &A[1..]),
            format!("{A}  refs/heads/main\n"),
            format!("{A}\trefs/heads/main\n"),
            format!("{A} refs/heads/a..b\n"),
            format!("{A} refs/heads/main\r\n"),
            format!("{A} refs/heads/main\0x\n"),
            format!("{A} refs/heads/main\n{B} refs/heads/main\n"),
            format!("{A} refs/heads/main\n# pack-refs with: sorted \n"),
            format!("{A} main\n"),
            "\n".to_owned(),
            // A SHA-256 ID in a SHA-1 repository.
            format!("{} refs/heads/main\n", "a".repeat(64)),
        ] {
            let (dir, git) = repo();
            git.packed_refs(&text);
            let err = refs_of(&dir).err();
            assert!(
                matches!(err, Some(GitError::Broken { .. })),
                "{text:?}: {err:?}"
            );
        }
    }

    /// Git's `parse_loose_ref_contents`, after the `strbuf_rtrim` of `read_ref_internal`
    /// (refs/files-backend.c, v2.56.0).
    #[test]
    fn ref_files_are_read_as_git_reads_them() {
        fn sha1(text: &str) -> Option<Contents<'_>> {
            parse_ref_file(text.as_bytes(), HashKind::Sha1)
        }
        fn sha256(text: &str) -> Option<Contents<'_>> {
            parse_ref_file(text.as_bytes(), HashKind::Sha256)
        }
        for text in [
            format!("{A}\n"),
            A.to_owned(),
            // Saved by Notepad, and with what FETCH_HEAD puts after the ID: Git reads both.
            format!("{A}\r\n"),
            format!("{A} trailing\n"),
            format!("{A}\tbranch 'main' of https://example.com/x\n"),
            format!("{A}\n\n \t"),
            format!("{}\n", A.to_uppercase()),
        ] {
            assert!(
                matches!(sha1(&text), Some(Contents::Id(i)) if i == id(A)),
                "{text:?}"
            );
        }
        for text in [
            "ref: refs/heads/main\n",
            "ref:refs/heads/main\n",
            "ref: refs/heads/main  \n",
            "ref: refs/heads/main\r\n",
            "ref: refs/heads/main",
            "ref:\t \r\nrefs/heads/main",
        ] {
            assert!(
                matches!(sha1(text), Some(Contents::Symbolic(b"refs/heads/main"))),
                "{text:?}"
            );
        }
        // The name is kept as written; whoever reads it checks it.
        assert!(matches!(
            sha1("ref: refs/heads/a b\n"),
            Some(Contents::Symbolic(b"refs/heads/a b"))
        ));
        assert!(matches!(sha1("ref:"), Some(Contents::Symbolic(b""))));
        // Git refuses these: no ID or `ref:` first, or something stuck to the ID.
        for text in [
            String::new(),
            "\n".to_owned(),
            "garbage\n".to_owned(),
            format!(" {A}\n"),
            format!("{A}x\n"),
            format!("{}\n", &A[1..]),
            format!("{}\n", "a".repeat(64)),
            // To Git, vertical tab and form feed aren't whitespace.
            format!("{A}\x0b\n"),
            format!("{A}\x0c"),
            "REF: refs/heads/main\n".to_owned(),
            "refs/heads/main\n".to_owned(),
            // Git would read up to the NUL; Bolt refuses, as in config.
            format!("{A}\0\n"),
            "ref: refs/heads/main\0\n".to_owned(),
        ] {
            assert!(sha1(&text).is_none(), "{text:?}");
        }
        let long = "e".repeat(64);
        assert!(matches!(
            sha256(&format!("{long}\r\n")),
            Some(Contents::Id(_))
        ));
        assert!(sha256(&format!("{A}\n")).is_none());
    }

    #[test]
    fn head_forms() {
        let (dir, git) = repo();
        let branch = |name: &[u8]| Head::Branch(name.to_vec());
        for (head, want) in [
            ("ref: refs/heads/main\n".to_owned(), branch(b"main")),
            (
                "ref: refs/heads/feature/x\n".to_owned(),
                branch(b"feature/x"),
            ),
            // Forms Git reads besides its own: hand-edited, saved by Notepad, no newline.
            ("ref: refs/heads/main\r\n".to_owned(), branch(b"main")),
            ("ref:refs/heads/main\n".to_owned(), branch(b"main")),
            ("ref: refs/heads/main  \n".to_owned(), branch(b"main")),
            ("ref: refs/heads/main".to_owned(), branch(b"main")),
            (format!("{A}\n"), Head::Detached(id(A))),
            (format!("{A}\r\n"), Head::Detached(id(A))),
            (format!("{A} trailing\n"), Head::Detached(id(A))),
            (A.to_owned(), Head::Detached(id(A))),
            (format!("{}\n", A.to_uppercase()), Head::Detached(id(A))),
        ] {
            git.head(&head);
            assert_eq!(refs_of(&dir).unwrap().head, want, "{head:?}");
        }
        let long = "a".repeat(64);
        for (head, shown) in [
            ("ref: refs/tags/v1\n".to_owned(), "refs/tags/v1".to_owned()),
            ("ref: refs/heads/a..b\n".into(), "refs/heads/a..b".into()),
            (
                "ref: refs/remotes/origin/main\n".into(),
                "refs/remotes/origin/main".into(),
            ),
            ("ref: refs/heads/\n".into(), "refs/heads/".into()),
            ("ref:\n".into(), String::new()),
            // Without `ref:`, a ref's name is no symbolic ref.
            ("refs/heads/main\n".into(), "refs/heads/main".into()),
            (format!("{A}x\n"), format!("{A}x")),
            (format!("{long}\n"), long.clone()),
            // The all-zero ID names no commit.
            (format!("{ZERO}\n"), ZERO.to_owned()),
            (
                "ref: refs/heads/main\0\n".into(),
                "ref: refs/heads/main\0".into(),
            ),
            (String::new(), String::new()),
        ] {
            git.head(&head);
            match refs_of(&dir) {
                Err(GitError::Broken { what }) => assert_eq!(
                    what,
                    format!("Git's HEAD points at `{shown}`, which isn't a branch"),
                ),
                other => panic!("{head:?}: {other:?}"),
            }
        }
        // A HEAD that isn't a small regular file.
        let head = dir.path().join(".git/HEAD");
        git.head(&"x".repeat(4097));
        assert!(matches!(refs_of(&dir), Err(GitError::Broken { .. })));
        std::fs::remove_file(&head).unwrap();
        assert!(matches!(refs_of(&dir), Err(GitError::Broken { .. })));
        std::fs::create_dir(&head).unwrap();
        assert!(matches!(refs_of(&dir), Err(GitError::NotPlain { .. })));
    }

    #[test]
    fn loose_refs_are_walked_as_git_walks_them() {
        let (dir, git) = repo();
        git.loose_ref("refs/heads/main", &format!("{A}\n"));
        git.loose_ref("refs/heads/feature/x", &format!("{B}\n"));
        // Git skips these: an update in progress, and hidden names.
        git.loose_ref("refs/heads/main.lock", "half written");
        git.loose_ref("refs/heads/.hidden", "anything");
        git.loose_ref("refs/.cache/x", "anything");
        git.loose_ref("refs/remotes/origin/main", &format!("{A}\n"));
        git.loose_ref(
            "refs/remotes/origin/HEAD",
            "ref: refs/remotes/origin/main\n",
        );
        git.loose_ref("refs/heads/alias", "ref: refs/heads/main\n");
        git.loose_ref("refs/stash", &format!("{C}\n"));
        git.loose_ref("refs/tags/v1", &format!("{C}\n"));
        // Forms Git reads besides its own.
        git.loose_ref("refs/heads/crlf", &format!("{B}\r\n"));
        git.loose_ref("refs/heads/bare", A);
        git.loose_ref(
            "refs/heads/fetched",
            &format!("{C}\tbranch 'main' of ../x\n"),
        );
        git.loose_ref("refs/heads/sym", "ref:refs/heads/main");
        assert_eq!(
            refs_of(&dir).unwrap(),
            Refs {
                head: Head::Branch(b"main".to_vec()),
                branches: named(&[
                    ("bare", A),
                    ("crlf", B),
                    ("feature/x", B),
                    ("fetched", C),
                    ("main", A),
                ]),
                servers: vec![(b"origin".to_vec(), b"main".to_vec(), id(A))],
                tags: named(&[("v1", C)]),
                // A symbolic ref is listed, unless it is a server's HEAD.
                other: vec![
                    b"refs/heads/alias".to_vec(),
                    b"refs/heads/sym".to_vec(),
                    b"refs/stash".to_vec(),
                ],
            }
        );
        // Without refs/ there are no loose refs Git could read.
        std::fs::remove_dir_all(dir.path().join(".git/refs")).unwrap();
        assert!(matches!(refs_of(&dir), Err(GitError::Broken { .. })));
    }

    #[test]
    fn loose_refs_bolt_cant_read_are_broken() {
        for (name, text) in [
            ("refs/heads/a..b", format!("{A}\n")),
            ("refs/heads/x", format!("{A}x\n")),
            ("refs/heads/x", format!(" {A}\n")),
            ("refs/heads/x", "garbage\n".to_owned()),
            ("refs/heads/x", format!("{A}\0\n")),
            ("refs/heads/x", "x".repeat(4097)),
            // Git takes a loose ref holding the all-zero ID for a broken one.
            ("refs/heads/x", format!("{ZERO}\n")),
        ] {
            let (dir, git) = repo();
            git.loose_ref(name, &text);
            let err = refs_of(&dir).err();
            assert!(
                matches!(err, Some(GitError::Broken { .. })),
                "{name} {text:?}: {err:?}"
            );
        }
        let (dir, git) = repo();
        git.loose_ref("refs/heads/x", &format!("{ZERO}\n"));
        match refs_of(&dir) {
            Err(GitError::Broken { what }) => assert_eq!(
                what,
                "the ref `refs/heads/x` holds the all-zero ID, which names no commit"
            ),
            other => panic!("{other:?}"),
        }
        // A linked folder or file under refs/.
        let (dir, git) = repo();
        git.loose_ref("refs/heads/main", &format!("{A}\n"));
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("main"), format!("{A}\n")).unwrap();
        let link = dir.path().join(".git").join("refs").join("linked");
        #[cfg(unix)]
        std::os::unix::fs::symlink(outside.path(), &link).unwrap();
        #[cfg(windows)]
        crate::test_support::junction(&link, outside.path());
        match refs_of(&dir) {
            Err(GitError::NotPlain { what }) => assert!(what.contains("linked"), "{what}"),
            other => panic!("{other:?}"),
        }
        let (dir, _git) = repo();
        if crate::test_support::symlink(
            &outside.path().join("main"),
            &dir.path().join(".git/refs/heads/linked"),
        ) {
            assert!(matches!(refs_of(&dir), Err(GitError::NotPlain { .. })));
        }
    }

    #[test]
    fn the_refs_read_are_bounded() {
        let (dir, git) = repo();
        for name in ["a", "b"] {
            git.loose_ref(&format!("refs/heads/{name}"), &format!("{A}\n"));
        }
        let path = dir.path().join(".git");
        let read_with = |entries, bytes| read(&path, HashKind::Sha1, Limits { entries, bytes });
        // Every entry counts, folders too: heads, tags, a and b.
        assert!(read_with(4, u64::MAX).is_ok());
        assert!(matches!(
            read_with(3, u64::MAX),
            Err(GitError::TooManyRefs { .. })
        ));
        git.packed_refs(&format!("{A} refs/heads/c\n"));
        assert!(read_with(5, u64::MAX).is_ok());
        let err = read_with(4, u64::MAX).err().unwrap();
        assert!(err.to_string().contains("more than 4 entries"), "{err}");
        assert!(err.hint().unwrap().contains("git remote prune"), "{err}");
        // So do the bytes kept: each folder's path and name while it waits, and each ref's name.
        let refs = path.join("refs");
        let folders: usize = ["heads", "tags"]
            .iter()
            .map(|f| refs.join(f).as_os_str().len() + "refs/".len() + f.len())
            .sum();
        let names = "refs/heads/a".len() + "refs/heads/b".len() + "refs/heads/c".len();
        let need = u64::try_from(folders + names).unwrap();
        assert!(read_with(u64::MAX, need).is_ok());
        let err = read_with(u64::MAX, need - 1).err().unwrap();
        assert!(matches!(err, GitError::TooManyRefs { .. }), "{err:?}");
        let want = format!("take more than {} bytes", need - 1);
        assert!(err.to_string().contains(&want), "{err}");
        // design.md §5.16 gives these numbers.
        assert_eq!((LIMITS.entries, LIMITS.bytes), (7_600_000, 512 << 20));
    }

    /// Windows opens a device, not the file listed, for a name like `NUL`; the walk must refuse
    /// it by name. `\\?\` paths make such a file without Windows reading the name.
    #[cfg(windows)]
    #[test]
    fn a_ref_named_like_a_device_is_refused_before_opening() {
        for name in ["NUL", "con", "aux.lock.x", "COM1"] {
            let (dir, git) = repo();
            git.loose_ref("refs/heads/main", &format!("{A}\n"));
            let heads = dir.path().join(".git").join("refs").join("heads");
            let verbatim = std::path::PathBuf::from(format!(r"\\?\{}", heads.display()));
            std::fs::write(verbatim.join(name), format!("{A}\n")).unwrap();
            match refs_of(&dir) {
                Err(GitError::Broken { what }) => {
                    assert!(what.contains("Windows"), "{name}: {what}");
                }
                other => panic!("{name}: {other:?}"),
            }
            std::fs::remove_file(verbatim.join(name)).unwrap();
        }
    }
}
