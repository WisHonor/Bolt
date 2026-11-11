//! Bolt's ignore rules against Git itself: the files a save keeps, and the verdict on each path,
//! for rule files drawn at random.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::OnceLock;

use bolt_core::drive::{self, Drive};
use bolt_core::id::Id;
use bolt_core::ignore::{Decision, Matcher, Rules};
use bolt_core::listing::EntryKind;
use bolt_core::repo::Repository;
use bolt_core::save::Person;
use bolt_core::snapshot::{SaveRequest, save_folder};
use bolt_core::store::Store;
use proptest::prelude::*;
use support::{git, git_with_input};
use unicode_normalization::UnicodeNormalization;

/// A project for one case: its files, and the rules of some of its folders.
#[derive(Clone, Debug)]
struct Tree {
    files: Vec<String>,
    /// Rule file text by folder (`""` is the top one), written as `.gitignore` for Git and
    /// `.boltignore` for Bolt.
    rules: BTreeMap<String, Vec<u8>>,
}

/// Names for files and folders: both letter cases, `é` composed and not, and characters patterns
/// treat specially, where a name on every system may hold them.
const NAMES: &[&str] = &[
    "a", "b", "A", "B", "ab", "Ab", "aB", "x.log", "Y.LOG", "\u{e9}", "e\u{301}", "[a]", "a]",
    "!a", "^b", "a-b", "#c", "{a,b}", "a b", "foo", "bar.txt", "d", "Dd", ".hidden",
];

/// What random pattern lines are made of.
const PIECES: &[&str] = &[
    "a",
    "A",
    "b",
    "B",
    "\u{e9}",
    "e\u{301}",
    "x",
    ".log",
    "*",
    "**",
    "?",
    "[",
    "]",
    "!",
    "^",
    "-",
    "\\",
    "/",
    " ",
    "#",
    "{",
    "}",
    "[a-z]",
    "[!a]",
    "[[:upper:]]",
    "\\ ",
];
const BEFORE: &[&str] = &["", "", "", "!", "/", "**/", "!/"];
/// For a `.boltignore` beside a `.gitignore`: mostly lines that take a path back.
const TAKE_BACK: &[&str] = &["!", "!", "!", "!/", "!**/", ""];
const AFTER: &[&str] = &["", "", "", "/", "  ", "\\ ", "/**", "*", "\\"];

/// Within a folder no two names may differ only by letter case or Unicode form, as on Windows
/// and macOS, and Bolt saves only names that work everywhere; no name is both file and folder.
fn tidy(raw: Vec<(Vec<&str>, &str)>) -> Vec<String> {
    let key = |name: &str| name.nfc().collect::<String>().to_lowercase();
    let mut seen: BTreeMap<(String, String), (String, bool)> = BTreeMap::new();
    let mut files = Vec::new();
    for (folders, file) in raw {
        let parts: Vec<(&str, bool)> = folders
            .iter()
            .map(|&f| (f, true))
            .chain([(file, false)])
            .collect();
        let mut fits = true;
        let mut path = String::new();
        for &(name, folder) in &parts {
            if let Some((had, was_folder)) = seen.get(&(path.clone(), key(name))) {
                fits &= had == name && *was_folder == folder;
            }
            path = join(&path, name);
        }
        if !fits {
            continue;
        }
        let mut path = String::new();
        for &(name, folder) in &parts {
            seen.insert((path.clone(), key(name)), (name.to_owned(), folder));
            path = join(&path, name);
        }
        files.push(path);
    }
    files.sort();
    files.dedup();
    files
}

fn join(folder: &str, name: &str) -> String {
    if folder.is_empty() {
        name.to_owned()
    } else {
        format!("{folder}/{name}")
    }
}

/// Every folder that holds a file, the top one (`""`) included.
fn folders_of(files: &[String]) -> Vec<String> {
    let mut out = vec![String::new()];
    for file in files {
        let mut at = file.as_str();
        while let Some((up, _)) = at.rsplit_once('/') {
            out.push(up.to_owned());
            at = up;
        }
    }
    out.sort();
    out.dedup();
    out
}

/// The files and folders below `folder`, as paths from it.
fn below(files: &[String], folder: &str) -> Vec<String> {
    let mut out = Vec::new();
    for file in files {
        let rest = if folder.is_empty() {
            file.as_str()
        } else if let Some(rest) = file.strip_prefix(&format!("{folder}/")) {
            rest
        } else {
            continue;
        };
        out.push(rest.to_owned());
        out.extend(folders_of(&[rest.to_owned()]).into_iter().skip(1));
    }
    out
}

/// One character of a path as a pattern line may write it.
fn respell(c: char, how: u8) -> String {
    match how {
        1 => "?".into(),
        2 => "*".into(),
        3 if c.is_lowercase() => c.to_uppercase().collect(),
        3 => c.to_lowercase().collect(),
        4 => format!("\\{c}"),
        5 => format!("[{c}]"),
        6 => format!("[!{c}]"),
        _ => c.into(),
    }
}

/// Git's version, as major and minor.
fn git_version() -> (u32, u32) {
    static VERSION: OnceLock<(u32, u32)> = OnceLock::new();
    *VERSION.get_or_init(|| {
        let dir = tempfile::tempdir().unwrap();
        let out = String::from_utf8(git(dir.path(), &["version"])).unwrap();
        let mut numbers = out
            .trim_start_matches("git version ")
            .split(['.', ' '])
            .map(|n| n.trim().parse::<u32>().unwrap());
        let version = (numbers.next().unwrap(), numbers.next().unwrap());
        if version < (2, 52) {
            eprintln!(
                "note: git {}.{} reads a `**` right after the start of a pattern with a `/` (as \
                 in foo**/bar) as Git 2.52 no longer does, and Bolt does as 2.52 does; such lines \
                 are left out",
                version.0, version.1
            );
        }
        version
    })
}

/// Whether Git 2.52 changed what `line` means: its matcher keeps one byte of a
/// pattern's literal start since, so a `**` right after it is no longer a whole folder name.
/// Errs towards yes.
fn changed_in_git_2_52(line: &str) -> bool {
    let text = line.strip_prefix('!').unwrap_or(line);
    let text = text.strip_suffix('/').unwrap_or(text);
    if !text.contains('/') {
        return false;
    }
    let text = text.strip_prefix('/').unwrap_or(text);
    let literal = text.find(['*', '?', '[', '\\']).unwrap_or(text.len());
    literal > 0 && text[literal..].starts_with("**") && !text[..literal].ends_with('/')
}

/// A rule file: random lines, and lines made from the paths below its folder, so that some
/// match and some don't.
fn rule_text(paths: Vec<String>) -> impl Strategy<Value = Vec<u8>> {
    rule_lines(paths, BEFORE)
}

/// `rule_text`, with lines made from paths beginning with one of `before`.
fn rule_lines(
    paths: Vec<String>,
    before: &'static [&'static str],
) -> impl Strategy<Value = Vec<u8>> {
    let old_git = git_version() < (2, 52);
    let random = prop::collection::vec(prop::sample::select(PIECES), 1..5).prop_map(|p| p.concat());
    let from_path = (
        prop::sample::select(paths),
        prop::sample::select(before),
        prop::sample::select(AFTER),
        any::<prop::sample::Index>(),
        0..10u8,
    )
        .prop_map(|(path, before, after, at, how)| {
            let chars: Vec<char> = path.chars().collect();
            let at = at.index(chars.len());
            let mid: String = chars
                .iter()
                .enumerate()
                .map(|(i, &c)| if i == at { respell(c, how) } else { c.into() })
                .collect();
            format!("{before}{mid}{after}")
        });
    let line = prop_oneof![2 => random, 3 => from_path];
    (
        any::<bool>(),
        any::<bool>(),
        prop::collection::vec(line, 1..7),
    )
        .prop_map(move |(bom, crlf, mut lines)| {
            if old_git {
                lines.retain(|line| !changed_in_git_2_52(line));
            }
            let mut text = if bom {
                "\u{feff}".to_owned()
            } else {
                String::new()
            };
            text.push_str(&lines.join(if crlf { "\r\n" } else { "\n" }));
            text.push('\n');
            text.into_bytes()
        })
}

/// Nested folders of files, with rule files at the top and in some folders.
fn ignore_tree() -> impl Strategy<Value = Tree> {
    let path = (
        prop::collection::vec(prop::sample::select(NAMES), 0..3),
        prop::sample::select(NAMES),
    );
    prop::collection::vec(path, 1..12)
        .prop_flat_map(|raw| {
            let files = tidy(raw);
            let rules: Vec<_> = folders_of(&files)
                .into_iter()
                .map(|folder| {
                    let odds = if folder.is_empty() { 0.9 } else { 0.4 };
                    let text = prop::option::weighted(odds, rule_text(below(&files, &folder)));
                    (Just(folder), text)
                })
                .collect();
            (Just(files), rules)
        })
        .prop_map(|(files, rules)| Tree {
            files,
            rules: rules
                .into_iter()
                .filter_map(|(folder, text)| Some((folder, text?)))
                .collect(),
        })
}

/// A tree and the `.boltignore` texts to lay beside its `.gitignore` files, by folder.
type WithBoltignore = (Tree, BTreeMap<String, Vec<u8>>);

/// Trees of `ignore_tree`'s kind with a `.boltignore` of lines that mostly take paths back in
/// some folders, `.gitignore` or not.
fn tree_with_boltignore() -> impl Strategy<Value = WithBoltignore> {
    ignore_tree()
        .prop_flat_map(|tree| {
            let texts: Vec<_> = folders_of(&tree.files)
                .into_iter()
                .map(|folder| {
                    let paths = below(&tree.files, &folder);
                    let text = prop::option::weighted(0.6, rule_lines(paths, TAKE_BACK));
                    (Just(folder), text)
                })
                .collect();
            (Just(tree), texts)
        })
        .prop_map(|(tree, texts)| {
            let boltignore = texts
                .into_iter()
                .filter_map(|(folder, text)| Some((folder, text?)))
                .collect();
            (tree, boltignore)
        })
}

/// The tree with a `.boltignore` beside every `.gitignore`, the same lines in both.
fn write_tree(root: &Path, tree: &Tree) {
    write_for_git(root, tree);
    for (folder, text) in &tree.rules {
        std::fs::write(root.join(folder).join(".boltignore"), text).unwrap();
    }
}

/// The tree's files and `.gitignore` files: all Git sees.
fn write_for_git(root: &Path, tree: &Tree) {
    for file in &tree.files {
        let path = root.join(file);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, b"x").unwrap();
    }
    for (folder, text) in &tree.rules {
        let dir = root.join(folder);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(".gitignore"), text).unwrap();
    }
}

/// The drive the tests run on, as `save` finds it.
fn this_drive() -> Drive {
    static DRIVE: OnceLock<Drive> = OnceLock::new();
    *DRIVE.get_or_init(|| {
        let tmp = tempfile::tempdir().unwrap();
        drive::probe(&bolt_platform::dir::Dir::open_top(tmp.path()).unwrap()).unwrap()
    })
}

/// A path as Git's walk reads it on this drive.
fn as_git_reads(path: &str, drive: Drive) -> String {
    if drive.composes_unicode {
        path.nfc().collect()
    } else {
        path.to_owned()
    }
}

/// Git told what this drive is like, and kept from the user's own ignore list.
fn git_args(drive: Drive, command: &[&str]) -> Vec<String> {
    let mut args: Vec<String> = [
        "-c".into(),
        "core.excludesFile=".into(),
        "-c".into(),
        format!("core.ignorecase={}", drive.ignores_case),
        "-c".into(),
        format!("core.precomposeunicode={}", drive.composes_unicode),
    ]
    .into();
    args.extend(command.iter().map(|&a| a.to_owned()));
    args
}

fn who() -> Person {
    Person {
        name: "Test".into(),
        email: "test@example.com".into(),
        time_ms: 1_700_000_000_000,
        tz_offset_min: 0,
    }
}

fn file_paths(store: &Store, id: Id, prefix: &str, out: &mut Vec<String>) {
    for e in store.listing(&id).unwrap().entries() {
        let path = format!("{prefix}{}", e.name);
        if e.kind == EntryKind::Folder {
            file_paths(store, e.id, &format!("{path}/"), out);
        } else {
            out.push(path);
        }
    }
}

/// The files of the line's newest save.
fn saved_files(repo: &Repository) -> Vec<String> {
    let (_, entry) = repo.current().unwrap();
    let id = entry.state.lines["main"];
    let save = repo.store().save(&id, repo.signatures_allowed()).unwrap();
    let mut out = Vec::new();
    file_paths(repo.store(), save.root, "", &mut out);
    out
}

/// Every file and folder of the tree, the rule files too, and whether each is a folder.
fn every_path(tree: &Tree) -> Vec<(String, bool)> {
    let mut files = tree.files.clone();
    for folder in tree.rules.keys() {
        files.push(join(folder, ".gitignore"));
        files.push(join(folder, ".boltignore"));
    }
    let mut out: Vec<(String, bool)> = folders_of(&files)
        .into_iter()
        .skip(1)
        .map(|f| (f, true))
        .collect();
    out.extend(files.into_iter().map(|f| (f, false)));
    out.sort();
    out.dedup();
    out
}

/// What Git's `check-ignore` says of `path`: the rules of the folders above it, read as a walk
/// meets them, so that a folder they ignore ignores what it holds too.
fn walk_verdict(
    rules: &BTreeMap<String, Vec<u8>>,
    drive: Drive,
    path: &str,
    is_dir: bool,
) -> Decision {
    walk_verdict_with(&[], rules, drive, path, is_dir)
}

/// `walk_verdict`, with Git's personal `lists` before the top folder's `.gitignore`.
fn walk_verdict_with(
    lists: &[&[u8]],
    rules: &BTreeMap<String, Vec<u8>>,
    drive: Drive,
    path: &str,
    is_dir: bool,
) -> Decision {
    let mut matcher = Matcher::new(drive);
    let mut folder = String::new();
    let parts: Vec<&str> = path.split('/').collect();
    for (i, part) in parts.iter().enumerate() {
        let mut level = Vec::new();
        if folder.is_empty() {
            level.extend(lists.iter().map(|text| Rules::parse(text, "", true)));
        }
        if let Some(text) = rules.get(&folder) {
            level.push(Rules::parse(text, &folder, true));
        }
        if !level.is_empty() {
            matcher.enter(&folder, level);
        }
        let here = join(&folder, part);
        if i + 1 == parts.len() {
            return matcher.decide(&here, is_dir);
        }
        if matcher.decide(&here, true) == Decision::Ignored {
            return Decision::Ignored;
        }
        folder = here;
    }
    unreachable!("a path has a last part")
}

/// `walk_verdict` for Bolt's verdict from the `.gitignore` lines alone, with each folder's
/// `.boltignore` in the matcher after its `.gitignore`, as a save puts them.
fn git_only_verdict(
    gitignore: &BTreeMap<String, Vec<u8>>,
    boltignore: &BTreeMap<String, Vec<u8>>,
    drive: Drive,
    path: &str,
    is_dir: bool,
) -> Decision {
    let mut matcher = Matcher::new(drive);
    let mut folder = String::new();
    let parts: Vec<&str> = path.split('/').collect();
    for (i, part) in parts.iter().enumerate() {
        let mut level = Vec::new();
        if let Some(text) = gitignore.get(&folder) {
            level.push(Rules::parse(text, &folder, true));
        }
        if let Some(text) = boltignore.get(&folder) {
            level.push(Rules::parse(text, &folder, false));
        }
        if !level.is_empty() {
            matcher.enter(&folder, level);
        }
        let here = join(&folder, part);
        if i + 1 == parts.len() {
            return matcher.verdict(&here, is_dir).git;
        }
        if matcher.verdict(&here, true).git == Decision::Ignored {
            return Decision::Ignored;
        }
        folder = here;
    }
    unreachable!("a path has a last part")
}

/// What Git's `check-ignore` says of each of `paths` in the repository at `root`, and the file,
/// line and pattern that say it.
fn check_ignore(root: &Path, drive: Drive, paths: &[(String, bool)]) -> Vec<(Decision, String)> {
    check_ignore_with(root, drive, None, paths)
}

/// `check_ignore`, with `own` as the user's own ignore list.
fn check_ignore_with(
    root: &Path,
    drive: Drive,
    own: Option<&Path>,
    paths: &[(String, bool)],
) -> Vec<(Decision, String)> {
    let mut input = Vec::new();
    for (path, _) in paths {
        input.extend(as_git_reads(path, drive).into_bytes());
        input.push(0);
    }
    let mut args = git_args(
        drive,
        &["check-ignore", "--no-index", "-v", "-n", "-z", "--stdin"],
    );
    if let Some(own) = own {
        args[1] = format!("core.excludesFile={}", own.display());
    }
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let out = git_with_input(root, &args, &input);
    assert!(
        matches!(out.status.code(), Some(0 | 1)),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    // Four fields a path: the file, the line, the pattern and the path.
    let fields: Vec<&[u8]> = out.stdout.split(|&b| b == 0).collect();
    let (records, _) = fields.as_chunks::<4>();
    assert_eq!(records.len(), paths.len());
    records
        .iter()
        .map(|record| {
            let pattern = record[2];
            let says = if pattern.is_empty() {
                Decision::NoRule
            } else if pattern.starts_with(b"!") {
                Decision::Kept
            } else {
                Decision::Ignored
            };
            let why = format!(
                "{}, line {}: {:?}",
                String::from_utf8_lossy(record[0]),
                String::from_utf8_lossy(record[1]),
                String::from_utf8_lossy(pattern)
            );
            (says, why)
        })
        .collect()
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 256, ..ProptestConfig::default() })]

    #[test]
    fn bolt_keeps_exactly_the_files_git_keeps(tree in ignore_tree()) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write_tree(root, &tree);
        git(root, &["init", "-q"]);
        let drive = this_drive();
        let args = git_args(drive, &["ls-files", "--others", "--exclude-standard", "-z"]);
        let listed = git(root, &args.iter().map(String::as_str).collect::<Vec<_>>());
        let mut want: Vec<String> = listed
            .split(|&b| b == 0)
            .filter(|p| !p.is_empty())
            .map(|p| String::from_utf8(p.to_vec()).unwrap())
            .collect();
        let who = who();
        let mut repo = Repository::init(root, &who).unwrap();
        let req = SaveRequest {
            message: Some("Save"), who: &who, command: "save", accept_crlf: false, git_excludes: None, new_line: None, only: None, amend: false,
        };
        save_folder(&mut repo, &req, &mut |_| {}, &mut |_| true).unwrap();
        let mut got: Vec<String> =
            saved_files(&repo).iter().map(|p| as_git_reads(p, drive)).collect();
        got.sort();
        want.sort();
        prop_assert_eq!(got, want);
    }

    #[test]
    fn single_path_verdicts_match_check_ignore(tree in ignore_tree()) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write_tree(root, &tree);
        git(root, &["init", "-q"]);
        let drive = this_drive();
        let paths = every_path(&tree);
        let says = check_ignore(root, drive, &paths);
        for ((path, is_dir), (git_says, why)) in paths.iter().zip(says) {
            prop_assert_eq!(
                walk_verdict(&tree.rules, drive, path, *is_dir),
                git_says,
                "{:?}: {}",
                path,
                why
            );
        }
    }

    /// Where a `.boltignore` line takes back what a `.gitignore` line keeps out, Bolt still says
    /// what Git says of the `.gitignore` files alone, which are all Git is given.
    #[test]
    fn git_only_verdicts_match_check_ignore_beside_boltignore_lines(
        (tree, boltignore) in tree_with_boltignore()
    ) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write_for_git(root, &tree);
        git(root, &["init", "-q"]);
        let drive = this_drive();
        let paths = every_path(&tree);
        let says = check_ignore(root, drive, &paths);
        for ((path, is_dir), (git_says, why)) in paths.iter().zip(says) {
            prop_assert_eq!(
                git_only_verdict(&tree.rules, &boltignore, drive, path, *is_dir),
                git_says,
                "{:?}: {}",
                path,
                why
            );
        }
    }
}

/// Git's personal lists (decision 047): the user's own list and `.git/info/exclude` keep out what
/// they keep out in Git, and the project's own rules win over them.
#[test]
fn gits_personal_lists_keep_out_what_git_keeps_out() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    git(root, &["init", "-q"]);
    let own = tempfile::tempdir().unwrap();
    let list = own.path().join("ignore");
    std::fs::write(&list, "*.log\n!b.log\nbuild/\n").unwrap();
    std::fs::create_dir_all(root.join(".git/info")).unwrap();
    std::fs::write(
        root.join(".git/info/exclude"),
        "b.log\nsecret.txt\n!keep.txt\n",
    )
    .unwrap();
    std::fs::write(root.join(".gitignore"), "keep.txt\n!c.log\n").unwrap();
    let files = [
        "a.log",
        "b.log",
        "c.log",
        "secret.txt",
        "keep.txt",
        "build/out.bin",
        "src/main.rs",
        "src/d.log",
    ];
    for path in files {
        let path = root.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, "x").unwrap();
    }
    let drive = this_drive();
    let mut args = git_args(drive, &["ls-files", "--others", "--exclude-standard", "-z"]);
    args[1] = format!("core.excludesFile={}", list.display());
    let listed = git(root, &args.iter().map(String::as_str).collect::<Vec<_>>());
    let mut want: Vec<String> = listed
        .split(|&b| b == 0)
        .filter(|p| !p.is_empty())
        .map(|p| String::from_utf8(p.to_vec()).unwrap())
        .collect();
    let who = who();
    let mut repo = Repository::init(root, &who).unwrap();
    let req = SaveRequest {
        message: Some("Save"),
        who: &who,
        command: "save",
        accept_crlf: false,
        git_excludes: Some(&list),
        new_line: None,
        only: None,
        amend: false,
    };
    save_folder(&mut repo, &req, &mut |_| {}, &mut |_| true).unwrap();
    let mut got: Vec<String> = saved_files(&repo)
        .iter()
        .map(|p| as_git_reads(p, drive))
        .collect();
    got.sort();
    want.sort();
    assert_eq!(got, want);
    // So that both sides can't agree on nothing.
    assert_eq!(want, [".gitignore", "c.log", "src/main.rs"]);
}

/// Single patterns made of the characters that matter, each in its own folder's `.gitignore`.
const PATTERNS: &[&str] = &[
    "a",
    "A",
    "*",
    "?",
    "a?",
    "?b",
    "*.txt",
    "*.TXT",
    "a*",
    "*b",
    "a*b",
    "*a*",
    "[ab]",
    "[!a]",
    "[^a]",
    "[a-c]",
    "[A-C]",
    "[c-a]",
    "[]a]",
    "[!]a]",
    "[a-]",
    "[-a]",
    "[a\\-c]",
    "[\\a-\\c]",
    "[a-c-e]*",
    "[[:alpha:]]",
    "[[:upper:]]",
    "[[:lower:]]*",
    "[[:digit:]]",
    "[[:punct:]]",
    "[[:alnum:]]*",
    "[[:space:]]*",
    "[[:foo:]]",
    "[[:alpha]",
    "[[:]a",
    "[a",
    "a]",
    "[B]",
    "[b]",
    "[Z-a]",
    "\\a",
    "\\A",
    "\\B",
    "\\[a]",
    "[\\]]",
    "\\!a",
    "!a",
    "\\#a",
    "#a",
    "{a,b}",
    "a\\",
    "a[/]b",
    "a\\/b",
    "**",
    "**/a",
    "**/b",
    "a/**",
    "x/**",
    "x/**/b",
    "a/**/b",
    "**/x/**",
    "x/*/b",
    "x/*",
    "/a",
    "/x/y",
    "x/y",
    "x/y/",
    "a/",
    "x/",
    "*/",
    "**/",
    "/*",
    "x**",
    "**x",
    "a/**b",
    "x/**y/z",
    "***",
    "a/***/b",
    "a/**/**/b",
    "*/**/b",
    "abc/**",
    "a/b/",
    "Foo/",
    "foo/*.TXT",
    "\u{e9}",
    "e\u{301}",
    "[\u{e9}]",
    "a b",
    "a\\ ",
    "a  ",
    "!*.log",
    "?/?",
];

/// Files below each pattern's folder, with names that patterns treat specially.
const PATTERN_FILES: &[&str] = &[
    "a",
    "b",
    "B",
    "ab",
    "aB",
    "Ab.txt",
    "x.log",
    "-",
    "_",
    "]",
    "[a]",
    "a]",
    "!a",
    "#a",
    "{a,b}",
    "a b",
    "1",
    "\u{e9}",
    "e\u{301}",
    "ax/b",
    "x/a",
    "x/y/b",
    "a/b",
    "a/x/b",
    "a/x/y/b",
    "abc/d",
    "abc/x/y",
    "Foo/bar.txt",
    "x/ay/z",
    "x/y/z",
    "c/a b",
];

#[test]
fn single_patterns_match_check_ignore() {
    let old_git = git_version() < (2, 52);
    let mut raw = Vec::new();
    let mut rules = BTreeMap::new();
    for (i, pattern) in PATTERNS.iter().enumerate() {
        if old_git && changed_in_git_2_52(pattern) {
            continue;
        }
        let folder = format!("p{i}");
        rules.insert(folder.clone(), format!("{pattern}\n").into_bytes());
        for file in PATTERN_FILES {
            let mut parts: Vec<&str> = file.split('/').collect();
            let name = parts.pop().unwrap();
            parts.insert(0, "");
            raw.push((parts, name));
        }
    }
    // Every pattern's files, in its own folder.
    let mut files = Vec::new();
    for folder in rules.keys() {
        let these: Vec<(Vec<&str>, &str)> = raw
            .iter()
            .take(PATTERN_FILES.len())
            .map(|(parts, name)| {
                let mut parts = parts.clone();
                parts[0] = folder.as_str();
                (parts, *name)
            })
            .collect();
        files.extend(tidy(these));
    }
    let tree = Tree { files, rules };
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write_for_git(root, &tree);
    git(root, &["init", "-q"]);
    let drive = this_drive();
    let paths = every_path(&tree);
    let says = check_ignore(root, drive, &paths);
    let mut wrong = Vec::new();
    for ((path, is_dir), (git_says, why)) in paths.iter().zip(says) {
        let bolt_says = walk_verdict(&tree.rules, drive, path, *is_dir);
        if bolt_says != git_says {
            let folder = path.split('/').next().unwrap();
            let pattern = String::from_utf8_lossy(&tree.rules[folder]).into_owned();
            wrong.push(format!(
                "{pattern:?} on {path:?}: Bolt {bolt_says:?}, Git {why}"
            ));
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// A project for `scenarios_match_check_ignore`: Git's two personal lists, the `.gitignore`
/// text of each folder that has one, and the files.
struct Scenario {
    own: &'static str,
    exclude: &'static str,
    rules: &'static [(&'static str, &'static str)],
    files: &'static [&'static str],
}

const SCENARIOS: &[Scenario] = &[
    // Negations across nested folders, and lines tied to their folder.
    Scenario {
        own: "",
        exclude: "",
        rules: &[
            ("", "*.log\n!keep.log\nbuild/\n/top.txt\n"),
            ("sub", "!debug.log\nkeep.log\n*.tmp\n!/x.tmp\n"),
            ("sub/deep", "!*.tmp\nbuild\n!top.txt\n"),
        ],
        files: &[
            "a.log",
            "keep.log",
            "debug.log",
            "top.txt",
            "build/out.o",
            "sub/a.log",
            "sub/debug.log",
            "sub/keep.log",
            "sub/top.txt",
            "sub/a.tmp",
            "sub/x.tmp",
            "sub/build/o",
            "sub/deep/x.tmp",
            "sub/deep/top.txt",
            "sub/deep/build",
            "sub/deep/q/build/o",
        ],
    },
    // The personal lists come first, the user's own before `.git/info/exclude`.
    Scenario {
        own: "*.bak\n!important.bak\nsecret/\n!*.swp\n",
        exclude: "important.bak\n*.swp\n!x.bak\n",
        rules: &[("", "!keep.swp\n!secret/\n"), ("src", "*.bak\n")],
        files: &[
            "a.bak",
            "x.bak",
            "important.bak",
            "x.swp",
            "keep.swp",
            "secret/a",
            "src/c.bak",
            "src/x.bak",
            "src/important.bak",
            "src/secret/b",
        ],
    },
    // Folder-only lines, and `**` in each place.
    Scenario {
        own: "",
        exclude: "",
        rules: &[
            ("", "/a/b/\n**/c/\nd/**\n!d/e\ne/**/f\n"),
            ("x", "a/b/\nc\n!/c/\n"),
        ],
        files: &[
            "a/b/x", "x/a/b/y", "c/z", "x/c/z", "x/q/c", "y/c/z", "d/e", "d/f/g", "e/q/r/f",
            "e/f/g",
        ],
    },
    // Nothing takes back a name inside an ignored folder; `dir/*` leaves the folder open.
    Scenario {
        own: "",
        exclude: "",
        rules: &[
            ("", "dir/\n!dir/keep\ndir2/*\n!dir2/keep\n"),
            ("dir", "!*\n"),
            ("dir2", "other\n"),
        ],
        files: &["dir/keep", "dir/other", "dir2/keep", "dir2/other", "dir2/x"],
    },
    // Letter case, which counts only where the drive heeds it.
    Scenario {
        own: "*.TMP\n",
        exclude: "",
        rules: &[("", "*.LOG\nBuild/\n!b.log\n"), ("Sub", "/X.txt\n!*.tmp\n")],
        files: &[
            "a.log",
            "B.LOG",
            "build/x",
            "BUILD2/y",
            "Sub/x.txt",
            "Sub/q.tmp",
            "c.tmp",
        ],
    },
];

#[test]
fn scenarios_match_check_ignore() {
    let drive = this_drive();
    for (n, scenario) in SCENARIOS.iter().enumerate() {
        let tree = Tree {
            files: scenario.files.iter().map(|&f| f.to_owned()).collect(),
            rules: scenario
                .rules
                .iter()
                .map(|&(folder, text)| (folder.to_owned(), text.as_bytes().to_vec()))
                .collect(),
        };
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        git(root, &["init", "-q"]);
        write_for_git(root, &tree);
        std::fs::create_dir_all(root.join(".git/info")).unwrap();
        std::fs::write(root.join(".git/info/exclude"), scenario.exclude).unwrap();
        let own = tempfile::tempdir().unwrap();
        let list = own.path().join("ignore");
        std::fs::write(&list, scenario.own).unwrap();
        let paths = every_path(&tree);
        let says = check_ignore_with(root, drive, Some(&list), &paths);
        let lists = [scenario.own.as_bytes(), scenario.exclude.as_bytes()];
        for ((path, is_dir), (git_says, why)) in paths.iter().zip(says) {
            assert_eq!(
                walk_verdict_with(&lists, &tree.rules, drive, path, *is_dir),
                git_says,
                "scenario {n}, {path:?}: {why}"
            );
        }
    }
}
