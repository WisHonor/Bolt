//! A project `opalro import` made, compared with the Git repository it came from (decision 002):
//! every commit, tree, file, branch, server branch and tag, as Git itself reads them.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt::Write as _;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Stdio};
use std::time::{Duration, Instant};

use opal_core::ancestry::only_in;
use opal_core::diary::{MAX_TAG_MESSAGE_BYTES, Position};
use opal_core::git::{GitId, HashKind, ObjectType, object_id, rebuild_commit};
use opal_core::id::Id;
use opal_core::import::{Rules, rename};
use opal_core::link::ProjectLink;
use opal_core::listing::EntryKind;
use opal_core::names::{is_git_alias, validate_name};
use opal_core::pack::ObjectKind;
use opal_core::repo::Repository;
use opal_core::save::{EXTRA_GIT_COMMIT, EXTRA_GIT_HEADER, MAX_PERSON_FIELD_BYTES};
use tempfile::TempDir;

use super::{git, git_command, spawn, text};

/// Asserts that the project `opalro import` made in `dir` holds what the Git repository there
/// holds. Each commit `git rev-list --all` lists has exactly one save, which rebuilds to the
/// commit's bytes. Each Git tree is a listing of the entries Opalro keeps, of the kinds their modes
/// make, with the files' bytes. The branches, server branches and tags are the lines, server
/// lines and tags, under the names Opalro's renaming rule gives them, and each annotated tag keeps
/// its tag objects' bytes.
///
/// Returns how many commits kept their Git header because their saves' own fields don't rebuild
/// them (decision 032). Commits Git writes itself need none, so a count above zero for them means
/// the saves' people, times or zones are wrong.
pub fn assert_matches_git(dir: &Path) -> usize {
    assert_matches_git_counting(dir).headers
}

/// What a comparison with Git left out of its checks.
pub struct Counted {
    /// The commits that kept their Git header (see `assert_matches_git`).
    pub headers: usize,
    /// The author and committer lines not in the form Git writes. Their saves' people can't be
    /// compared with Git's, so only a kept header vouches for them.
    pub odd_people: usize,
}

/// `assert_matches_git`, with both counts.
pub fn assert_matches_git_counting(dir: &Path) -> Counted {
    let repo = Repository::open(dir).unwrap();
    let state = repo.current().unwrap().1.state;
    let store = repo.store();
    let tags = repo.tag_list(&state).unwrap().tags;
    let servers = repo.server_lines(&state).unwrap().servers;
    let listed = text(&git(dir, &["rev-list", "--all", "HEAD"]));
    let hash = match listed.lines().next().map(str::len) {
        Some(64) => HashKind::Sha256,
        _ => HashKind::Sha1,
    };
    let id = |hex: &str| GitId::from_hex(hash, hex).unwrap();

    // Every save the lines, server lines, tags and position reach, and its commit.
    let mut todo: Vec<Id> = state.lines.values().copied().collect();
    todo.extend(servers.values().flat_map(|own| own.values().copied()));
    todo.extend(tags.values().map(|tag| tag.save));
    if let Position::AtSave(save) = &state.position {
        todo.push(*save);
    }
    let mut commit_of: HashMap<Id, GitId> = HashMap::new();
    let mut save_of: HashMap<GitId, Id> = HashMap::new();
    while let Some(save_id) = todo.pop() {
        if commit_of.contains_key(&save_id) {
            continue;
        }
        let save = store.save(&save_id, false).unwrap();
        let commit = id(std::str::from_utf8(&save.extra[EXTRA_GIT_COMMIT]).unwrap());
        let other = save_of.insert(commit, save_id);
        assert!(other.is_none(), "two saves of commit {}", commit.to_hex());
        commit_of.insert(save_id, commit);
        todo.extend(save.parents);
    }
    let in_git: HashSet<GitId> = listed.lines().map(id).collect();
    let missing: Vec<String> = in_git
        .iter()
        .filter(|commit| !save_of.contains_key(commit))
        .take(10)
        .map(GitId::to_hex)
        .collect();
    let extra: Vec<String> = save_of
        .keys()
        .filter(|commit| !in_git.contains(commit))
        .take(10)
        .map(GitId::to_hex)
        .collect();
    assert!(
        missing.is_empty() && extra.is_empty(),
        "commits without a save: {missing:?}; saves of commits Git doesn't list: {extra:?}"
    );
    let saved = |commit: &GitId| match save_of.get(commit) {
        Some(save) => *save,
        None => panic!("no save of commit {}", commit.to_hex()),
    };

    // Each commit rebuilds from its save, whose parents are Git's.
    let mut objects = CatFile::new(dir);
    let mut trees: Vec<(GitId, Id)> = Vec::new();
    let mut headers = 0;
    let mut odd_people = 0;
    for (commit, save_id) in &save_of {
        let hex = commit.to_hex();
        let raw = objects.read(commit, "commit");
        let save = store.save(save_id, false).unwrap();
        let (tree, parents) = tree_and_parents(&raw, hash);
        // A save lists each parent once, where Git may list one twice.
        let mut once: Vec<GitId> = Vec::new();
        for parent in parents {
            if !once.contains(&parent) {
                once.push(parent);
            }
        }
        let parents: Vec<GitId> = save.parents.iter().map(|p| commit_of[p]).collect();
        assert_eq!(parents, once, "the parents of commit {hex}");
        let rebuilt = rebuild_commit(&save, &tree, &parents);
        assert!(
            rebuilt == raw,
            "commit {hex} rebuilds as\n{}\nwhere Git has\n{}",
            text(&rebuilt),
            text(&raw)
        );
        // The header rebuilds a commit whatever the save's people hold, so they are compared with
        // Git's own lines. A commit rebuilt without it has every line in Git's form.
        let kept = save.extra.contains_key(EXTRA_GIT_HEADER);
        for (key, person) in [("author", &save.author), ("committer", &save.saver)] {
            let saved = (
                person.name.as_str(),
                person.email.as_str(),
                person.time_ms,
                person.tz_offset_min,
            );
            match person_in(&raw, key) {
                Some(git) => assert_eq!(saved, git, "the {key} of commit {hex}"),
                None => {
                    assert!(kept, "the {key} line of commit {hex} isn't in Git's form");
                    odd_people += 1;
                }
            }
        }
        headers += usize::from(kept);
        trees.push((tree, save.root));
    }

    // Each Git tree once, against the listing it became; each file's bytes once, by Opalro's ID.
    let mut listings: HashMap<GitId, Id> = HashMap::new();
    let mut blobs: HashMap<Id, (GitId, u64)> = HashMap::new();
    while let Some((tree, listing)) = trees.pop() {
        if let Some(other) = listings.insert(tree, listing) {
            assert_eq!(other, listing, "tree {} became two listings", tree.to_hex());
            continue;
        }
        let raw = objects.read(&tree, "tree");
        let mut kept = Vec::new();
        for (mode, name, entry) in tree_entries(&raw, hash) {
            let name = std::str::from_utf8(name).expect("an import stops at a name in UTF-8");
            // The import's own skip rules (decisions 019 and 035).
            if validate_name(name).is_ok() && !is_git_alias(name) {
                kept.push((name, mode, entry));
            }
        }
        kept.sort_by_key(|&(name, ..)| name);
        let listing = store.listing(&listing).unwrap();
        let names: Vec<&str> = listing.entries().iter().map(|e| e.name.as_str()).collect();
        let want: Vec<&str> = kept.iter().map(|(name, ..)| *name).collect();
        assert_eq!(names, want, "the names in tree {}", tree.to_hex());
        for (entry, (name, mode, git_id)) in listing.entries().iter().zip(kept) {
            let at = || format!("`{name}` in tree {}", tree.to_hex());
            assert_eq!(entry.kind, kind_of(mode), "the kind of {}", at());
            match entry.kind {
                EntryKind::Folder => {
                    let folder = GitId::from_hex(hash, &hex(git_id)).unwrap();
                    trees.push((folder, entry.id));
                }
                EntryKind::ProjectLink => {
                    let link = store.read_record(&entry.id, ObjectKind::ProjectLink);
                    let link = ProjectLink::decode(&link.unwrap()).unwrap();
                    assert!(
                        link.url.is_empty() && link.commit == git_id,
                        "submodule {} links to {link:?}",
                        at()
                    );
                }
                _ => {
                    let (blob, size) = *blobs.entry(entry.id).or_insert_with(|| {
                        let mut bytes = Vec::new();
                        store.copy_content(&entry.id, &mut bytes).unwrap();
                        let blob = object_id(ObjectType::Blob, &bytes, hash).unwrap();
                        (blob, bytes.len() as u64)
                    });
                    assert!(
                        blob.as_bytes() == git_id && size == entry.size,
                        "{} holds blob {} of {size} bytes, listed as {}",
                        at(),
                        blob.to_hex(),
                        entry.size
                    );
                }
            }
        }
    }

    // Branches, server branches and tags. Opalro doesn't follow symbolic refs.
    let refs = git(
        dir,
        &[
            "for-each-ref",
            "--format=%(refname)%00%(objectname)%00%(symref)",
        ],
    );
    let (mut branches, mut git_tags) = (Vec::new(), Vec::new());
    let mut by_server: BTreeMap<&[u8], Vec<(&[u8], GitId)>> = BTreeMap::new();
    for line in refs.split(|&b| b == b'\n').filter(|line| !line.is_empty()) {
        let [name, target, symref] = line.split(|&b| b == 0).collect::<Vec<_>>()[..] else {
            panic!("git for-each-ref printed {}", text(line));
        };
        if !symref.is_empty() {
            continue;
        }
        let target = id(std::str::from_utf8(target).unwrap());
        if let Some(branch) = name.strip_prefix(b"refs/heads/") {
            branches.push((branch, target));
        } else if let Some(tag) = name.strip_prefix(b"refs/tags/") {
            git_tags.push((tag, target));
        } else if let Some(rest) = name.strip_prefix(b"refs/remotes/")
            && let Some(slash) = rest.iter().position(|&b| b == b'/')
        {
            let (server, branch) = (&rest[..slash], &rest[slash + 1..]);
            by_server.entry(server).or_default().push((branch, target));
        }
    }
    let lines = renamed(branches, Rules::Line);
    let want: BTreeMap<String, Id> = lines
        .iter()
        .map(|(opalro, _, target)| (opalro.clone(), saved(target)))
        .collect();
    assert_eq!(state.lines, want, "the lines");
    let server_names: Vec<&[u8]> = by_server.keys().copied().collect();
    let mut want = BTreeMap::new();
    for (opalro, (_, own)) in rename(&server_names, Rules::Line)
        .into_iter()
        .zip(by_server)
    {
        let own: BTreeMap<String, Id> = renamed(own, Rules::Line)
            .into_iter()
            .map(|(line, _, target)| (line, saved(&target)))
            .collect();
        want.insert(opalro, own);
    }
    assert_eq!(servers, want, "the server lines");

    let head = text(&git(dir, &["rev-parse", "--symbolic-full-name", "HEAD"]));
    let position = match head.trim().strip_prefix("refs/heads/") {
        Some(branch) => {
            let line = lines.iter().find(|(_, git, _)| *git == branch.as_bytes());
            Position::OnLine(line.unwrap().0.clone())
        }
        None => Position::AtSave(saved(&id(text(&git(dir, &["rev-parse", "HEAD"])).trim()))),
    };
    assert_eq!(state.position, position, "where HEAD is");

    // Each tag that ends at a commit, with the tag objects on the way there, outermost first.
    let mut want: BTreeMap<String, (Id, Vec<Vec<u8>>)> = BTreeMap::new();
    for (opalro, _, target) in renamed(git_tags, Rules::Tag) {
        let mut chain = Vec::new();
        let mut at = target;
        let end = loop {
            let (kind, raw) = objects.read_any(&at);
            if kind != "tag" {
                break (kind == "commit").then_some(at);
            }
            let object = raw.split(|&b| b == b'\n').next().unwrap();
            at = id(std::str::from_utf8(object.strip_prefix(b"object ").unwrap()).unwrap());
            chain.push(raw);
        };
        if let Some(commit) = end {
            want.insert(opalro, (saved(&commit), chain));
        }
    }
    let names: Vec<&String> = tags.keys().collect();
    assert_eq!(names, want.keys().collect::<Vec<_>>(), "the tags");
    for (name, tag) in &tags {
        let (save, chain) = &want[name];
        assert_eq!(tag.save, *save, "the save of tag {name}");
        assert!(
            tag.git_tags == *chain,
            "tag {name} keeps other tag objects than Git's"
        );
        // A message in UTF-8 that fits is kept as it is.
        let message = chain.first().map_or(&[][..], |raw| message_of(raw));
        if let Ok(message) = std::str::from_utf8(message)
            && message.len() <= MAX_TAG_MESSAGE_BYTES
        {
            assert_eq!(tag.message, message, "the message of tag {name}");
        }
    }
    Counted {
        headers,
        odd_people,
    }
}

/// Asserts that `only_in(a, [b])` counts what `git rev-list --count a ^b` does, for up to 64
/// ordered pairs of tips: the lines in name order, then the server lines in name order, each
/// standing for the commit its save records. Where a commit in the tips' history is dated before
/// a parent the walk may count too many, though never too few and never a held tip as holding
/// some, so only that is asserted there. Returns how many pairs it compared, and whether the
/// counts had to be equal.
pub fn assert_ancestry_matches_git(dir: &Path) -> (usize, bool) {
    let repo = Repository::open(dir).unwrap();
    let state = repo.current().unwrap().1.state;
    let servers = repo.server_lines(&state).unwrap().servers;
    let mut named: Vec<(String, Id)> = state.lines.iter().map(|(n, t)| (n.clone(), *t)).collect();
    for (server, own) in &servers {
        named.extend(own.iter().map(|(n, t)| (format!("{server}/{n}"), *t)));
    }
    let tips: Vec<(String, Id, String)> = named
        .into_iter()
        .map(|(name, tip)| {
            let save = repo.store().save(&tip, false).unwrap();
            let Some(commit) = save.extra.get(EXTRA_GIT_COMMIT) else {
                panic!("the save of {name} records no Git commit");
            };
            let commit = std::str::from_utf8(commit).unwrap().to_owned();
            (name, tip, commit)
        })
        .collect();
    let n = tips.len();
    let pairs: Vec<(usize, usize)> = (0..n)
        .flat_map(|a| (0..n).filter(move |&b| b != a).map(move |b| (a, b)))
        .take(64)
        .collect();
    if pairs.is_empty() {
        println!("The ancestry walk has no pairs to compare.");
        return (0, true);
    }
    let compared: Vec<&str> = (0..n)
        .filter(|&i| pairs.iter().any(|&(a, b)| a == i || b == i))
        .map(|i| tips[i].2.as_str())
        .collect();
    let exact = !dated_before_a_parent(dir, &compared);
    let (mut walking, mut equal) = (Duration::ZERO, 0);
    for &(a, b) in &pairs {
        let ((a_name, a_tip, a_commit), (b_name, b_tip, b_commit)) = (&tips[a], &tips[b]);
        let began = Instant::now();
        let ours = only_in(&repo, *a_tip, &[*b_tip]).unwrap();
        walking += began.elapsed();
        let not_b = format!("^{b_commit}");
        let counted = text(&git(dir, &["rev-list", "--count", a_commit, &not_b]));
        let theirs: u64 = counted.trim().parse().unwrap();
        let at = format!("only_in({a_name}, [{b_name}]) is {ours} where Git counts {theirs}");
        if exact {
            assert_eq!(ours, theirs, "{at}");
        } else {
            assert_eq!(ours == 0, theirs == 0, "{at}");
            assert!(ours >= theirs, "{at}");
        }
        equal += usize::from(ours == theirs);
    }
    let took = walking.as_secs_f64() * 1000.0;
    if exact {
        println!(
            "The ancestry walk matches git rev-list on {} pairs of tips ({took:.1} ms walking).",
            pairs.len()
        );
    } else {
        println!(
            "The ancestry walk on {} pairs of tips, where some commit is dated before a parent: \
             {equal} equal to git rev-list's count, the rest higher ({took:.1} ms walking).",
            pairs.len()
        );
    }
    (pairs.len(), exact)
}

/// Whether a commit in the history of `tips` has a committer date earlier than a parent's.
fn dated_before_a_parent(dir: &Path, tips: &[&str]) -> bool {
    let args = [&["log", "--format=%H %ct %P"], tips].concat();
    let log = text(&git(dir, &args));
    let rows: Vec<Vec<&str>> = log
        .lines()
        .map(|l| l.split_whitespace().collect())
        .collect();
    let time: HashMap<&str, i64> = rows
        .iter()
        .map(|row| (row[0], row[1].parse().unwrap()))
        .collect();
    rows.iter()
        .any(|row| row[2..].iter().any(|parent| time[parent] > time[row[0]]))
}

/// `git cat-file --batch`, asked for one object at a time.
struct CatFile {
    git: Child,
    input: Option<ChildStdin>,
    output: BufReader<ChildStdout>,
    _home: TempDir,
}

impl CatFile {
    fn new(dir: &Path) -> CatFile {
        let home = tempfile::tempdir().unwrap();
        let mut cmd = git_command(dir, home.path(), &["cat-file", "--batch"]);
        // Shown as it comes, so no pipe nobody reads fills up.
        let mut git = spawn(cmd.stderr(Stdio::inherit()));
        let input = git.stdin.take();
        let output = BufReader::new(git.stdout.take().unwrap());
        CatFile {
            git,
            input,
            output,
            _home: home,
        }
    }

    /// The type and bytes of the object `id`.
    fn read_any(&mut self, id: &GitId) -> (String, Vec<u8>) {
        let input = self.input.as_mut().unwrap();
        writeln!(input, "{}", id.to_hex()).unwrap();
        input.flush().unwrap();
        let mut header = String::new();
        self.output.read_line(&mut header).unwrap();
        // `<id> <type> <size>`, or `<id> missing`.
        let [_, kind, size] = header.split_whitespace().collect::<Vec<_>>()[..] else {
            panic!("git cat-file says {header}");
        };
        let mut data = vec![0; size.parse::<usize>().unwrap() + 1];
        self.output.read_exact(&mut data).unwrap();
        // The newline Git puts after each object.
        assert_eq!(data.pop(), Some(b'\n'));
        (kind.to_owned(), data)
    }

    /// The bytes of the object `id`, which must be a `kind`.
    fn read(&mut self, id: &GitId, kind: &str) -> Vec<u8> {
        let (found, data) = self.read_any(id);
        assert_eq!(found, kind, "object {}", id.to_hex());
        data
    }
}

impl Drop for CatFile {
    fn drop(&mut self) {
        // Its input closed, Git ends.
        drop(self.input.take());
        let _ = self.git.wait();
    }
}

/// `refs`, in byte order, each with the name Opalro's renaming rule gives it under `rules`.
fn renamed(mut refs: Vec<(&[u8], GitId)>, rules: Rules) -> Vec<(String, &[u8], GitId)> {
    refs.sort();
    let names: Vec<&[u8]> = refs.iter().map(|(name, _)| *name).collect();
    rename(&names, rules)
        .into_iter()
        .zip(refs)
        .map(|(opalro, (git, target))| (opalro, git, target))
        .collect()
}

/// The tree a commit's bytes name on their first line, and the parents on the lines after it.
fn tree_and_parents(raw: &[u8], hash: HashKind) -> (GitId, Vec<GitId>) {
    let id = |hex: &[u8]| GitId::from_hex(hash, std::str::from_utf8(hex).unwrap()).unwrap();
    let mut lines = raw.split(|&b| b == b'\n');
    let tree = lines.next().and_then(|line| line.strip_prefix(b"tree "));
    let parents = lines.map_while(|line| line.strip_prefix(b"parent "));
    (id(tree.unwrap()), parents.map(id).collect())
}

/// The person on a commit's `key` line (`author` or `committer`), if Git wrote it in its own form,
/// `name <email> seconds ±hhmm` in UTF-8: the name, the email, the time in milliseconds and the
/// zone in minutes.
fn person_in<'a>(raw: &'a [u8], key: &str) -> Option<(&'a str, &'a str, i64, i16)> {
    let prefix = format!("{key} ");
    let mut header = raw
        .split(|&b| b == b'\n')
        .take_while(|line| !line.is_empty());
    let line = header.find_map(|line| line.strip_prefix(prefix.as_bytes()))?;
    let (name, rest) = std::str::from_utf8(line).ok()?.split_once('<')?;
    let name = name.strip_suffix(' ')?;
    let (email, when) = rest.split_once('>')?;
    let (seconds, zone) = when.strip_prefix(' ')?.split_once(' ')?;
    let (sign, hhmm) = zone.split_at_checked(1)?;
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    let fits = |s: &str| s.len() <= MAX_PERSON_FIELD_BYTES;
    // Git's own whitespace, which it never leaves at a name's end.
    if name.ends_with([' ', '\t', '\n', '\r'])
        || !fits(name)
        || !fits(email)
        || !digits(seconds)
        || hhmm.len() != 4
        || !digits(hhmm)
    {
        return None;
    }
    let time_ms = seconds.parse::<i64>().ok()?.checked_mul(1000)?;
    let minutes = hhmm[..2].parse::<i16>().ok()? * 60 + hhmm[2..].parse::<i16>().ok()?;
    let zone = match sign {
        "+" => minutes,
        "-" => -minutes,
        _ => return None,
    };
    Some((name, email, time_ms, zone))
}

/// A Git tree's entries, in its own order: each `<mode> <name>\0` and then the ID's bytes.
fn tree_entries(mut raw: &[u8], hash: HashKind) -> Vec<(u32, &[u8], &[u8])> {
    let mut entries = Vec::new();
    while !raw.is_empty() {
        let space = raw.iter().position(|&b| b == b' ').unwrap();
        let nul = raw.iter().position(|&b| b == 0).unwrap();
        let mode = u32::from_str_radix(std::str::from_utf8(&raw[..space]).unwrap(), 8).unwrap();
        let (id, rest) = raw[nul + 1..].split_at(hash.len());
        entries.push((mode, &raw[space + 1..nul], id));
        raw = rest;
    }
    entries
}

/// The kind of entry a mode makes, as Git reads modes.
fn kind_of(mode: u32) -> EntryKind {
    match mode & 0o170_000 {
        0o040_000 => EntryKind::Folder,
        0o120_000 => EntryKind::Symlink,
        0o160_000 => EntryKind::ProjectLink,
        0o100_000 if mode & 0o100 != 0 => EntryKind::Executable,
        0o100_000 => EntryKind::File,
        _ => panic!("Git doesn't write mode {mode:o}"),
    }
}

/// The message of a tag object: everything after the empty line that ends its header.
fn message_of(raw: &[u8]) -> &[u8] {
    raw.windows(2)
        .position(|pair| pair == b"\n\n")
        .map_or(&[], |at| &raw[at + 2..])
}

fn hex(bytes: &[u8]) -> String {
    let mut hex = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(hex, "{b:02x}");
    }
    hex
}
