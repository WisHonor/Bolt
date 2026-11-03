//! Git's refs: `HEAD`, the loose refs under `.git/refs/` and `.git/packed-refs` (decisions 031
//! and 033). Names follow Git's `check-ref-format`, and anything Git wouldn't write is `Broken`.
#![deny(clippy::indexing_slicing, clippy::arithmetic_side_effects)]

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use super::{GitError, GitId, HashKind, broken, read_error, read_optional, real_dir};
use crate::diary::{MAX_LINES, MAX_SERVERS, MAX_TAGS};
use crate::names;
use crate::safe_fs;

/// `HEAD`, and each loose ref.
const MAX_REF_FILE_BYTES: u64 = 4096;
/// `packed-refs`: room for millions of refs, at some 70 bytes a line.
const MAX_PACKED_REFS_BYTES: u64 = 512 << 20;
/// Most refs Bolt reads, loose and packed together: every one decision 033's limits let an
/// import bring in (lines, each server's lines, tags), and a million more it only lists.
pub(super) const MAX_REFS: u64 = MAX_LINES * (1 + MAX_SERVERS) + MAX_TAGS + 1_000_000;

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

/// What a ref holds. A symbolic ref names another ref, which Bolt doesn't follow.
enum Target {
    Id(GitId),
    Symbolic,
}

/// The refs of the repository whose `.git` is `git`, reading at most `max` of them.
pub(super) fn read(git: &Path, hash: HashKind, max: u64) -> Result<Refs, GitError> {
    let head = read_head(git, hash)?;
    let mut count = 0;
    // Loose refs first: `git pack-refs` writes packed-refs before it deletes the loose ones, so
    // in this order no ref is missed while it moves.
    let loose = read_loose(git, hash, max, &mut count)?;
    let packed = match read_optional(&git.join("packed-refs"), MAX_PACKED_REFS_BYTES)? {
        Some(text) => read_packed(&text, hash, max, &mut count)?,
        None => BTreeMap::new(),
    };
    let mut all: BTreeMap<Vec<u8>, Target> = packed
        .into_iter()
        .map(|(name, id)| (name, Target::Id(id)))
        .collect();
    // A loose ref wins, as in Git: it is the newer.
    all.extend(loose);
    Ok(sort(head, all))
}

fn sort(head: Head, all: BTreeMap<Vec<u8>, Target>) -> Refs {
    let mut refs = Refs {
        head,
        branches: Vec::new(),
        servers: Vec::new(),
        tags: Vec::new(),
        other: Vec::new(),
    };
    for (name, target) in all {
        let server_ref = name.strip_prefix(b"refs/remotes/").and_then(|rest| {
            let mut parts = rest.splitn(2, |&c| c == b'/');
            Some((parts.next()?.to_vec(), parts.next()?.to_vec()))
        });
        let Target::Id(id) = target else {
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

/// The text of `name` for a message: Git allows any bytes in names but a few.
fn shown(name: &[u8]) -> String {
    String::from_utf8_lossy(name).into_owned()
}

fn hex_id(hash: HashKind, hex: &[u8]) -> Option<GitId> {
    GitId::from_hex(hash, std::str::from_utf8(hex).ok()?)
}

fn read_head(git: &Path, hash: HashKind) -> Result<Head, GitError> {
    let path = git.join("HEAD");
    let text = read_optional(&path, MAX_REF_FILE_BYTES)?
        .ok_or_else(|| broken(format!("{} is missing", path.display())))?;
    parse_head(&text, hash)
}

/// `ref: refs/heads/<name>` or a commit ID, each ending in a newline: how Git writes `HEAD`.
fn parse_head(text: &[u8], hash: HashKind) -> Result<Head, GitError> {
    let line = text.strip_suffix(b"\n");
    if let Some(target) = line.and_then(|l| l.strip_prefix(b"ref: ")) {
        if let Some(branch) = target.strip_prefix(b"refs/heads/")
            && is_valid_ref_name(target)
        {
            return Ok(Head::Branch(branch.to_vec()));
        }
    } else if let Some(id) = line.and_then(|l| hex_id(hash, l)) {
        return Ok(Head::Detached(id));
    }
    let x = line.unwrap_or(text);
    Err(broken(format!(
        "Git's HEAD points at `{}`, which isn't a branch",
        shown(x.strip_prefix(b"ref: ").unwrap_or(x))
    )))
}

/// The loose refs under `.git/refs`, walked with a stack of folders, not recursion. Each folder
/// must be a real one and each ref a regular file; names Git skips (`.` first, `.lock` last) are
/// skipped. Every entry counts toward `max`.
fn read_loose(
    git: &Path,
    hash: HashKind,
    max: u64,
    count: &mut u64,
) -> Result<BTreeMap<Vec<u8>, Target>, GitError> {
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
            counted(count, max)?;
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
            let target = parse_loose(&text, hash).ok_or_else(|| {
                broken(format!(
                    "the ref `{}` holds neither a commit ID nor another ref's name",
                    shown(&full)
                ))
            })?;
            refs.insert(full, target);
        }
    }
    Ok(refs)
}

/// A loose ref as Git writes it: an ID and a newline, or `ref:` and the name of another ref.
fn parse_loose(text: &[u8], hash: HashKind) -> Option<Target> {
    if text.starts_with(b"ref:") {
        return Some(Target::Symbolic);
    }
    hex_id(hash, text.strip_suffix(b"\n")?).map(Target::Id)
}

fn counted(count: &mut u64, max: u64) -> Result<(), GitError> {
    *count = count.saturating_add(1);
    if *count > max {
        return Err(broken(format!(
            "it has more than {max} refs, more than Bolt reads"
        )));
    }
    Ok(())
}

/// `packed-refs`: an optional `# pack-refs with:` line, then `<id> <name>` lines, each maybe
/// followed by a `^<id>` line naming what it peels to, which Bolt ignores: it peels tags itself.
fn read_packed(
    text: &[u8],
    hash: HashKind,
    max: u64,
    count: &mut u64,
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
        counted(count, max)?;
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

    /// `git check-ref-format [options] <name>`, built on `is_valid_ref_name`, which takes no
    /// options. Options read as in builtin/check-ref-format.c: `--normalize` collapses slashes
    /// first; `--refspec-pattern` allows one `*`, which checks like a plain letter; and
    /// `--allow-onelevel` drops only the two-level rule, as a level put above shows.
    fn check_ref_format(name: &[u8], options: &str) -> bool {
        let mut name = name.to_vec();
        if options.contains("--normalize") {
            name = collapse_slashes(&name);
        }
        if options.contains("--refspec-pattern") {
            if name.iter().filter(|&&c| c == b'*').count() > 1 {
                return false;
            }
            name = name
                .iter()
                .map(|&c| if c == b'*' { b'a' } else { c })
                .collect();
        }
        if options.contains("--allow-onelevel") {
            name != b"@" && is_valid_ref_name(&[b"x/", &name[..]].concat())
        } else {
            is_valid_ref_name(&name)
        }
    }

    /// builtin/check-ref-format.c's `collapse_slashes`.
    fn collapse_slashes(name: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut prev = b'/';
        for &c in name {
            if prev == b'/' && c == b'/' {
                continue;
            }
            out.push(c);
            prev = c;
        }
        out
    }

    /// Every `valid_ref`, `invalid_ref`, `valid_ref_normalized` and `invalid_ref_normalized` row
    /// of Git's t1402-check-ref-format.sh (v2.56.0), in order. The `--branch` tests need a
    /// repository's history and aren't name rules.
    #[test]
    fn ref_names_follow_check_ref_format() {
        let (onelevel, pattern, normalize) =
            ("--allow-onelevel", "--refspec-pattern", "--normalize");
        let both = "--refspec-pattern --allow-onelevel";
        let mut rows: Vec<(bool, Vec<u8>, &str)> = Vec::new();
        let mut row = |valid: bool, name: &[u8], options: &'static str| {
            rows.push((valid, name.to_vec(), options));
        };
        row(false, b"", "");
        row(false, b"/", "");
        row(false, b"/", onelevel);
        row(false, b"/", normalize);
        row(false, b"/", "--allow-onelevel --normalize");
        row(true, b"foo/bar/baz", "");
        row(true, b"foo/bar/baz", normalize);
        row(false, b"refs///heads/foo", "");
        row(true, b"refs///heads/foo", normalize);
        row(false, b"heads/foo/", "");
        row(false, b"/heads/foo", "");
        row(true, b"/heads/foo", normalize);
        row(false, b"///heads/foo", "");
        row(true, b"///heads/foo", normalize);
        row(false, b"./foo", "");
        row(false, b"./foo/bar", "");
        row(false, b"foo/./bar", "");
        row(false, b"foo/bar/.", "");
        row(false, b".refs/foo", "");
        row(false, b"refs/heads/foo.", "");
        for c in ["?", "~", "^", ":", "*", "[", " ", "\\", ".."] {
            row(false, format!("heads/foo{c}bar").as_bytes(), "");
        }
        row(true, b"foo./bar", "");
        row(false, b"heads/foo.lock", "");
        row(false, b"heads///foo.lock", "");
        row(false, b"foo.lock/bar", "");
        row(false, b"foo.lock///bar", "");
        row(true, b"heads/foo@bar", "");
        row(true, b"refs/@", "");
        row(false, b"@", onelevel);
        row(false, b"heads/v@{ation", "");
        row(false, b"heads/foo\t", "");
        row(false, b"heads/foo\x7f", "");
        row(true, b"heads/fu\xc3\x9f", "");
        row(true, b"heads/*foo/bar", pattern);
        row(true, b"heads/foo*/bar", pattern);
        row(true, b"heads/f*o/bar", pattern);
        row(false, b"heads/f*o*/bar", pattern);
        row(false, b"heads/foo*/bar*", pattern);

        row(false, b"foo", "");
        row(true, b"foo", onelevel);
        row(false, b"foo", pattern);
        row(true, b"foo", both);
        row(false, b"foo", normalize);
        row(true, b"foo", "--allow-onelevel --normalize");

        row(true, b"foo/bar", "");
        row(true, b"foo/bar", onelevel);
        row(true, b"foo/bar", pattern);
        row(true, b"foo/bar", both);
        row(true, b"foo/bar", normalize);

        row(false, b"foo/*", "");
        row(false, b"foo/*", onelevel);
        row(true, b"foo/*", pattern);
        row(true, b"foo/*", both);

        row(false, b"*/foo", "");
        row(false, b"*/foo", onelevel);
        row(true, b"*/foo", pattern);
        row(true, b"*/foo", both);
        row(false, b"*/foo", normalize);
        row(true, b"*/foo", "--refspec-pattern --normalize");

        row(false, b"foo/*/bar", "");
        row(false, b"foo/*/bar", onelevel);
        row(true, b"foo/*/bar", pattern);
        row(true, b"foo/*/bar", both);

        row(false, b"*", "");
        row(false, b"*", onelevel);
        row(false, b"*", pattern);
        row(true, b"*", both);

        row(false, b"foo/*/*", pattern);
        row(false, b"foo/*/*", both);

        row(false, b"*/foo/*", pattern);
        row(false, b"*/foo/*", both);

        row(false, b"*/*/foo", pattern);
        row(false, b"*/*/foo", both);

        row(false, b"/foo", "");
        row(false, b"/foo", onelevel);
        row(false, b"/foo", pattern);
        row(false, b"/foo", both);
        row(false, b"/foo", normalize);
        row(true, b"/foo", "--allow-onelevel --normalize");
        row(false, b"/foo", "--refspec-pattern --normalize");
        row(
            true,
            b"/foo",
            "--refspec-pattern --allow-onelevel --normalize",
        );

        assert_eq!(rows.len(), 89);
        for (valid, name, options) in &rows {
            assert_eq!(
                check_ref_format(name, options),
                *valid,
                "{:?} {options}",
                String::from_utf8_lossy(name)
            );
        }

        for (name, normalized) in [
            ("heads/foo", "heads/foo"),
            ("refs///heads/foo", "refs/heads/foo"),
            ("/heads/foo", "heads/foo"),
            ("///heads/foo", "heads/foo"),
        ] {
            assert_eq!(collapse_slashes(name.as_bytes()), normalized.as_bytes());
            assert!(check_ref_format(name.as_bytes(), normalize), "{name}");
        }
        for name in [
            "foo",
            "/foo",
            "heads/foo/../bar",
            "heads/./foo",
            "heads\\foo",
            "heads/foo.lock",
            "heads///foo.lock",
            "foo.lock/bar",
            "foo.lock///bar",
        ] {
            assert!(!check_ref_format(name.as_bytes(), normalize), "{name}");
        }
        // A NUL can't end a name early, as it would in C.
        assert!(!is_valid_ref_name(b"refs/heads/a\0"));
    }

    const A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    const C: &str = "cccccccccccccccccccccccccccccccccccccccc";
    const D: &str = "dddddddddddddddddddddddddddddddddddddddd";

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

    #[test]
    fn head_forms() {
        let (dir, git) = repo();
        let (detached, upper) = (format!("{A}\n"), format!("{}\n", A.to_uppercase()));
        for (head, want) in [
            ("ref: refs/heads/main\n", Head::Branch(b"main".to_vec())),
            (
                "ref: refs/heads/feature/x\n",
                Head::Branch(b"feature/x".to_vec()),
            ),
            (detached.as_str(), Head::Detached(id(A))),
            (upper.as_str(), Head::Detached(id(A))),
        ] {
            git.head(head);
            assert_eq!(refs_of(&dir).unwrap().head, want, "{head:?}");
        }
        let (spaced, long) = (format!("{A} "), "a".repeat(64));
        let (spaced_line, long_line) = (format!("{spaced}\n"), format!("{long}\n"));
        for (head, shown) in [
            ("ref: refs/tags/v1\n", "refs/tags/v1"),
            ("ref: refs/heads/a..b\n", "refs/heads/a..b"),
            (
                "ref: refs/remotes/origin/main\n",
                "refs/remotes/origin/main",
            ),
            ("ref: refs/heads/main", "refs/heads/main"),
            ("ref:refs/heads/main\n", "ref:refs/heads/main"),
            ("ref: refs/heads/main\r\n", "refs/heads/main\r"),
            (A, A),
            (spaced_line.as_str(), spaced.as_str()),
            (long_line.as_str(), long.as_str()),
            ("", ""),
        ] {
            git.head(head);
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
        assert!(matches!(refs_of(&dir), Err(GitError::Broken { .. })));
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
        assert_eq!(
            refs_of(&dir).unwrap(),
            Refs {
                head: Head::Branch(b"main".to_vec()),
                branches: named(&[("feature/x", B), ("main", A)]),
                servers: vec![(b"origin".to_vec(), b"main".to_vec(), id(A))],
                tags: named(&[("v1", C)]),
                // A symbolic ref is listed, unless it is a server's HEAD.
                other: vec![b"refs/heads/alias".to_vec(), b"refs/stash".to_vec()],
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
            ("refs/heads/x", A.to_owned()),
            ("refs/heads/x", format!("{A}\r\n")),
            ("refs/heads/x", "garbage\n".to_owned()),
            ("refs/heads/x", "x".repeat(4097)),
        ] {
            let (dir, git) = repo();
            git.loose_ref(name, &text);
            let err = refs_of(&dir).err();
            assert!(
                matches!(err, Some(GitError::Broken { .. })),
                "{name} {text:?}: {err:?}"
            );
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
            Err(GitError::Broken { what }) => assert!(what.contains("linked"), "{what}"),
            other => panic!("{other:?}"),
        }
        let (dir, _git) = repo();
        if crate::test_support::symlink(
            &outside.path().join("main"),
            &dir.path().join(".git/refs/heads/linked"),
        ) {
            assert!(matches!(refs_of(&dir), Err(GitError::Broken { .. })));
        }
    }

    #[test]
    fn the_refs_read_are_bounded() {
        let (dir, git) = repo();
        for name in ["a", "b"] {
            git.loose_ref(&format!("refs/heads/{name}"), &format!("{A}\n"));
        }
        let path = dir.path().join(".git");
        // Every entry counts, folders too: heads, tags, a and b.
        assert!(read(&path, HashKind::Sha1, 4).is_ok());
        assert!(matches!(
            read(&path, HashKind::Sha1, 3),
            Err(GitError::Broken { .. })
        ));
        git.packed_refs(&format!("{A} refs/heads/c\n"));
        assert!(read(&path, HashKind::Sha1, 5).is_ok());
        let err = read(&path, HashKind::Sha1, 4).err().unwrap();
        assert!(err.to_string().contains("more than 4 refs"), "{err}");
        // design.md §5.16 gives this number.
        assert_eq!(MAX_REFS, 7_600_000);
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
