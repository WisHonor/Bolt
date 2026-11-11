#![allow(clippy::unwrap_used, clippy::panic)]
#![allow(clippy::indexing_slicing, clippy::arithmetic_side_effects)]
use super::*;
use crate::git::{GitError, HashKind, Refusal};

fn format(text: &str) -> Result<Format, GitError> {
    read_format(text.as_bytes(), ".git/config")
}

#[test]
fn the_format_follows_gits_rules() {
    // Without a format version, Git ignores every extension.
    let unversioned = format(
        "[extensions]\n\tobjectFormat = sha256\n\tpartialClone = o\n\tworktreeConfig = true\n",
    )
    .unwrap();
    assert_eq!(unversioned.hash, HashKind::Sha1);
    assert!(!unversioned.worktree_config);
    // The last value of a key wins.
    let last = format("[core]\n\trepositoryformatversion = 2\n\trepositoryformatversion = 1\n[extensions]\n\tobjectformat = sha1\n\tobjectformat = sha256\n").unwrap();
    assert_eq!(last.hash, HashKind::Sha256);
    assert_eq!(
        format("[core]\n\tsparsecheckout\n").unwrap().sparse,
        Some(true)
    );
    // Extensions only format 1 knows refuse format 0, as Git refuses them.
    let v0 = format("[core]\n\trepositoryformatversion = 0\n[extensions]\n\tobjectformat = sha1\n");
    assert!(
        matches!(&v0, Err(GitError::Broken { what }) if what.contains("extensions.objectformat")),
        "{:?}",
        v0.err()
    );
    // A version is a number as Git reads one, and no version is negative.
    assert!(matches!(
        format("[core]\n\trepositoryformatversion = 1k\n"),
        Err(GitError::Refused(Refusal::FormatVersion(1024)))
    ));
    assert!(matches!(
        format("[core]\n\trepositoryformatversion = -1\n"),
        Err(GitError::Broken { .. })
    ));
    // The last refStorage counts, as in Git.
    let storage = |values: &str| {
        format(&format!(
            "[core]\n\trepositoryformatversion = 1\n[extensions]\n{values}"
        ))
    };
    assert!(storage("\trefStorage = reftable\n\trefStorage = files\n").is_ok());
    assert!(matches!(
        storage("\trefStorage = files\n\trefStorage = files://../refs\n"),
        Err(GitError::Refused(Refusal::RefsElsewhere))
    ));
    assert!(matches!(
        storage("\trefStorage = files:///x\n\trefStorage = reftable://y\n"),
        Err(GitError::Refused(Refusal::Reftable))
    ));
    // The last `lfs.storage` counts, as in Git; one with no value is empty.
    let lfs_storage = |text: &str| format(text).unwrap().lfs_storage;
    assert_eq!(
        lfs_storage("[lfs]\n\tstorage = ../shared\n"),
        Some((b"../shared".to_vec(), ".git/config"))
    );
    assert_eq!(
        lfs_storage("[lfs]\n\tstorage = x\n[LFS]\n\tStorage\n"),
        Some((Vec::new(), ".git/config"))
    );
    assert_eq!(lfs_storage("[lfs]\n\turl = x\n"), None);
    // Keys Bolt doesn't keep are still read, so a bad line among them refuses.
    assert!(format("[remote \"origin\"]\n\turl = \"x\n").is_err());
    assert!(format("[remote \"origin\"]\n\turl = x\n\tpromisor = maybe\n").is_ok());
    let worktree = |text: &str| {
        let read = read_worktree(text.as_bytes(), ".git/config.worktree").unwrap();
        (read.sparse, read.includes, read.lfs_storage)
    };
    assert_eq!(
        worktree("[core]\n\tsparseCheckout = no\n[lfs]\n\tstorage = lfs\n"),
        (
            Some(false),
            false,
            Some((b"lfs".to_vec(), ".git/config.worktree"))
        )
    );
    assert_eq!(worktree("[extensions]\n\tx = y\n"), (None, false, None));
}

/// Includes aren't followed, but a section that can take Git to another file is noted, in
/// either file Bolt reads: what Git reads there may turn on sparse checkout.
#[test]
fn include_sections_are_noted() {
    for (text, includes) in [
        ("[include]\n\tpath = sparse.inc\n", true),
        ("[includeIf \"gitdir:~/work/\"]\n\tpath = work.inc\n", true),
        ("[Include]\n\tPath = x\n", true),
        (
            "[remote \"include\"]\n\turl = x\n[includes]\n\tpath = y\n",
            false,
        ),
        // A section with no settings takes Git nowhere.
        ("[include]\n", false),
    ] {
        assert_eq!(format(text).unwrap().includes, includes, "{text}");
        let worktree = read_worktree(text.as_bytes(), ".git/config.worktree").unwrap();
        assert_eq!(worktree.includes, includes, "{text}");
    }
}

#[test]
fn the_users_settings_for_the_list_are_read_as_git_reads_them() {
    use UserSetting::{ExcludesFile, Include};
    let read = |text: &str| read_user_settings(text.as_bytes());
    let excludes = |value: &[u8]| ExcludesFile(value.to_vec());
    assert_eq!(
        read("[core]\n\texcludesFile = ~/.gitignore_global\n"),
        Ok(vec![excludes(b"~/.gitignore_global")])
    );
    // Every one, in order; letter case doesn't matter in the name, and a quoted value is
    // unquoted as Git does it.
    assert_eq!(
        read("[core]\nexcludesfile = a\n[CORE]\nEXCLUDESFILE = \"C:\\\\x y\"\n"),
        Ok(vec![excludes(b"a"), excludes(b"C:\\x y")])
    );
    assert_eq!(read("[user]\n\tname = x\n"), Ok(vec![]));
    // Plain includes come where they stand; the others aren't Git's `include.path`.
    assert_eq!(
        read("[core]\nexcludesFile = a\n[Include]\n\tPath = ~/b\n[core]\nexcludesFile = c\n"),
        Ok(vec![
            excludes(b"a"),
            Include(b"~/b".to_vec()),
            excludes(b"c")
        ])
    );
    assert_eq!(
        read("[includeIf \"gitdir:~/\"]\n\tpath = x\n[include \"y\"]\n\tpath = z\n"),
        Ok(vec![])
    );
    // `lfs.storage` is kept whatever it holds; with no value it is empty.
    assert_eq!(
        read("[lfs]\n\tstorage = x\n[lfs]\n\tstorage\n"),
        Ok(vec![
            UserSetting::LfsStorage(b"x".to_vec()),
            UserSetting::LfsStorage(Vec::new())
        ])
    );
    // A path needs a value, and a line Git can't read is refused.
    assert_eq!(read("[core]\n\texcludesFile\n"), Err(2));
    assert_eq!(read("[include]\n\tpath\n"), Err(2));
    assert_eq!(read("[user]\nname = x\n[core\n"), Err(3));
}

/// As Git reads a `GIT_CONFIG_KEY_<n>`.
#[test]
fn keys_in_the_environment_are_read_as_git_reads_them() {
    let read = |key: &str| read_user_variable(key.as_bytes(), b"x");
    let excludes = Ok(Some(UserSetting::ExcludesFile(b"x".to_vec())));
    assert_eq!(read("core.excludesFile"), excludes);
    assert_eq!(read("CORE.EXCLUDESFILE"), excludes);
    assert_eq!(
        read("include.path"),
        Ok(Some(UserSetting::Include(b"x".to_vec())))
    );
    assert_eq!(
        read_user_variable(b"lfs.storage", b""),
        Ok(Some(UserSetting::LfsStorage(Vec::new())))
    );
    // A subsection is kept as written, so this is another setting; Git takes these keys.
    for key in [
        "core.Sub.excludesFile",
        "user.name",
        "-.x",
        ".a.b",
        "1.x",
        "a.b c\t.d",
    ] {
        assert_eq!(read(key), Ok(None), "{key:?}");
    }
    // No section, no name, a name that starts with a digit, a byte other than a letter, digit
    // or `-` outside the subsection, or a line break in it.
    for key in [
        "",
        "core",
        ".x",
        "core.",
        "core.1x",
        "co_re.x",
        "core.ex_file",
        "a.b\nc.d",
    ] {
        assert_eq!(read(key), Err(()), "{key:?}");
    }
    assert_eq!(read_user_variable(b"core.\xffx", b"x"), Err(()));
    assert_eq!(read_user_variable(b"a.\xff.x", b"x"), Ok(None));
}

/// `strtoul` in base 10, then Git's own checks, as Git reads `GIT_CONFIG_COUNT`.
#[test]
fn a_count_in_the_environment_is_read_as_git_reads_it() {
    for (text, want) in [
        ("", Some(0)),
        ("0", Some(0)),
        ("-0", Some(0)),
        ("+3", Some(3)),
        (" \t\n3", Some(3)),
        ("007", Some(7)),
        ("2147483647", Some(2_147_483_647)),
        // What strtoul doesn't read to the end.
        (" ", None),
        ("+", None),
        ("3 ", None),
        ("0x3", None),
        ("- 3", None),
        ("3k", None),
        // More than an int holds, as a negative number is once it wraps round.
        ("2147483648", None),
        ("-1", None),
        ("99999999999999999999999", None),
    ] {
        assert_eq!(read_count(text.as_bytes()).ok(), want, "{text:?}");
    }
    // strtoul negates in an unsigned long, whose size Git's build decides.
    let wraps_to_one = format!("-{}", std::ffi::c_ulong::MAX);
    assert_eq!(read_count(wraps_to_one.as_bytes()), Ok(1));
}

/// Every entry of `text` as `git config --list -z` shows it: names and values end at a NUL.
type Entries = Vec<(Vec<u8>, Option<Vec<u8>>)>;

fn entries(text: &[u8]) -> Result<Entries, usize> {
    let mut all = Vec::new();
    read(text, &mut |entry: Entry| {
        let value = entry
            .value
            .map(|v| v.split(|&b| b == 0).next().unwrap().to_vec());
        all.push((entry.name.to_vec(), value));
        Ok::<(), ()>(())
    })
    .map_err(|e| e.unwrap())?;
    Ok(all)
}

/// Whether `git` can be run here; a test that needs it says so and passes without it.
fn have_git() -> bool {
    let ok = std::process::Command::new("git")
        .arg("--version")
        .output()
        .is_ok_and(|out| out.status.success());
    if !ok {
        eprintln!("note: git isn't installed, so config files aren't compared with it");
    }
    ok
}

/// `git config --file … <args>` on `text`: its output, or what it says when it refuses.
fn git_config(dir: &std::path::Path, text: &[u8], args: &[&str]) -> Result<Vec<u8>, String> {
    let file = dir.join("config");
    std::fs::write(&file, text).unwrap();
    let out = std::process::Command::new("git")
        .arg("config")
        .arg("--file")
        .arg(&file)
        .args(args)
        .output()
        .unwrap();
    if out.status.success() {
        Ok(out.stdout)
    } else {
        Err(String::from_utf8_lossy(&out.stderr).into_owned())
    }
}

/// What `git config --list -z` reads in `text`, or the line Git names.
fn git_entries(dir: &std::path::Path, text: &[u8]) -> Result<Entries, usize> {
    match git_config(dir, text, &["--list", "-z"]) {
        Ok(out) => Ok(out
            .split(|&b| b == 0)
            .filter(|record| !record.is_empty())
            .map(|record| match record.iter().position(|&b| b == b'\n') {
                Some(at) => (record[..at].to_vec(), Some(record[at + 1..].to_vec())),
                None => (record.to_vec(), None),
            })
            .collect()),
        Err(message) => {
            let line = message
                .split("bad config line ")
                .nth(1)
                .and_then(|rest| rest.split(' ').next())
                .and_then(|n| n.parse().ok());
            Err(line.unwrap_or_else(|| panic!("{message}")))
        }
    }
}

fn shown_text(text: &[u8]) -> String {
    text.escape_ascii().to_string()
}

/// Bolt reads these files as `git config` does: the same names and values, or the same line
/// refused.
#[test]
fn files_are_read_as_git_reads_them() {
    if !have_git() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let cases: &[&[u8]] = &[
        b"[a]\nb = c\n",
        b"[a] b = c",
        b"[a ]\nb\n",
        b"[ a]\nb\n",
        b"[ \"x\"]\nb\n",
        b"[a  \"x\"]\nb\n",
        b"[a\t\"x\"]\nb\n",
        b"[a\r\"x\"]\nb\n",
        b"[a\x0b\"x\"]\nb\n",
        b"[a\"x\"]\nb\n",
        b"[a \"x\" ]\nb\n",
        b"[a \"x\"y]\nb\n",
        b"[a \"x\\ty\\\"\\\\\"]\nb\n",
        b"[A.B.c]\nk\n",
        b"[a.]\nk\n",
        b"[.a]\nk\n",
        b"[]\nk\n",
        b"[1a]\nk\n",
        b"[-]\nk\n",
        b"[a_b]\nk\n",
        b"[S \"A\"]\nK\n",
        b"k = v\n[a]\n",
        // Where Git says a header ends: on its line, or the next one.
        b"#\n[a\n",
        b"#\n[a",
        b"#\n[a\r\n",
        b"#\n[",
        b"#\n[\n",
        b"#\n[a_",
        b"#\n[a \"x\"",
        b"#\n[a \"x\"\n",
        b"#\n[a \"x\"\r\n",
        b"#\n[a \"x\"z",
        b"#\n[a \"x",
        b"#\n[a \"x\\",
        b"#\n[a \"x\\\n\"]\n",
        b"#\n[a \"x\n\"]\n",
        b"#\n[a ",
        b"#\n[a \n",
        b"#\n[a ]",
        // Names.
        b"[s]\nk_ = v\n",
        b"[s]\n1k = v\n",
        b"[s]\n-k = v\n",
        b"[s]\nk-1 = v\n",
        b"[s]\nK1-b\n",
        b"[s]\nk v\n",
        b"[s]\nk # c\n",
        b"[s]\nk\t\n",
        b"[s]\nk\r\n",
        b"[s]\nk\r = v\n",
        b"[s]\nk\x0b= v\n",
        b"[s]\nk\x00 = v\n",
        b"[s]\n\xffk = v\n",
        b"[s]\n=v\n",
        b"[s]\nk==v\n",
        b"[s]\nk\n=v\n",
        b"[s]\nk \\\n= v\n",
        b"[s] k ; c\n",
        b"[s]\x0bk\n",
        // Values.
        b"[s]\nk = a   b\t\tc  \n",
        b"[s]\nk =  a  \"  \" \n",
        b"[s]\nk = a \"\" \n",
        b"[s]\nk = \"\" x\n",
        b"[s]\nk = \"\"  \n",
        b"[s]\nk = \" a \"  b \" \n",
        b"[s]\nk = a\\nb\\tc\\bd\n",
        b"[s]\nk = a\\\"b\\\\c\n",
        b"[s]\nk = \"a\\\"b\\\\c\"\n",
        b"[s]\nk = a\\0b\n",
        b"[s]\nk = a\\qb\n",
        b"[s]\nk = a\\\nb\n",
        b"[s]\nk = a \\\n  b\n",
        b"[s]\nk = a  \\\n  b  \n",
        b"[s]\nk = a \\\n  \n",
        b"[s]\nk = a  \\\n  #c\n",
        b"[s]\nk = a \r\r\\",
        b"[s]\nk = \"a\" \\",
        b"[s]\nk = \"a\\\nb\"\n",
        b"[s]\nk = \"a\nb\"\n",
        b"[s]\nk = \"a\n",
        b"[s]\nk = \"a",
        b"[s]\nk = a\\",
        b"[s]\nk = \"a\\",
        b"[s]\nk = \"a\\\n",
        b"[s]\nk = a\\\r\nb\n",
        b"[s]\nk = a\\\rb\n",
        b"[s]\nk = a # c\n",
        b"[s]\nk = \"a # c\" ; d\n",
        b"[s]\nk = a#b\n",
        b"[s]\nk = a\"b\"c\n",
        b"[s]\nk = v\\\n# not a comment\n",
        b"[s]\nk = a\\\n\\\n\"\n",
        b"[s]\nk = a\\\n\\\nb\nc_\n",
        b"[s]\nk = \\t a\n",
        b"[s]\nk = a \\t\n",
        b"[s]\nk = \\  a\n",
        b"[s]\nk = x\r",
        b"[s]\nk = x\r\r\n",
        b"[s]\nk = \"x\r\"\n",
        b"[s]\nk =\ra\n",
        b"[s]\nk =\x0ba\x0b\n",
        b"[s]\nk =\x0ca\n",
        b"[s]\nk = \xff\xfe\n",
        b"[s \"\xff\"]\nk = a\n",
        // Byte order marks, line ends, comments and blank lines. A NUL in a subsection or a
        // value, which Bolt refuses where Git cuts it short, has its own test.
        b"\xef\xbb\xbf[s]\nk = a\n",
        b"\xef\xbb\xbf\xef\xbb\xbf[s]\nk = a\n",
        b"\n\xef\xbb\xbf[s]\nk = a\n",
        b"\xef\xbb[s]\nk = a\n",
        b"[s]\r\nk = v\r\nj = w\r\n",
        b"[s]\r\r\nk\n",
        b"[s]\n\rk = a\n",
        b"[s]\r",
        b"[s]\n\t\t\n  ; c\n#c \x00\n\nk=a\n",
        b"[s] ; c\nk\n",
        b"[s]#c\nk\n",
        b"",
        b"\n",
        b"\xef\xbb\xbf",
    ];
    for &text in cases {
        let git = git_entries(dir.path(), text);
        assert_eq!(entries(text), git, "{}", shown_text(text));
    }
    // And files made of pieces that often go wrong, the same ones each run.
    let pieces: &[&[u8]] = &[
        b"[",
        b"]",
        b"\"",
        b"\\",
        b"\n",
        b"\r\n",
        b"\r",
        b" ",
        b"\t",
        b"=",
        b"#",
        b";",
        b"a",
        b"B",
        b"k",
        b"1",
        b"-",
        b".",
        b"_",
        b"\x0b",
        b"\xff",
        b"[s]\n",
        b"[s \"X\"]",
        b"k = ",
        b"\"v w\"",
        b"\\n",
        b"\\q",
        b"\xef\xbb\xbf",
    ];
    let mut seed: u64 = 0x2545_f491_4f6c_dd1d;
    let mut random = |below: usize| {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        usize::try_from(seed % u64::try_from(below).unwrap()).unwrap()
    };
    let count = std::env::var("BOLT_GIT_CONFIG_CASES").map_or(150, |n| n.parse().unwrap());
    for _ in 0..count {
        let mut text = Vec::new();
        for _ in 0..1 + random(14) {
            text.extend_from_slice(pieces[random(pieces.len())]);
        }
        let git = git_entries(dir.path(), &text);
        assert_eq!(entries(&text), git, "{}", shown_text(&text));
    }
}

/// Booleans as `git config --type=bool` reads them.
#[test]
fn booleans_are_read_as_git_reads_them() {
    if !have_git() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let values: &[&[u8]] = &[
        b"",
        b"true",
        b"TRUE",
        b"Yes",
        b"on",
        b"oN",
        b"1",
        b"false",
        b"No",
        b"OFF",
        b"0",
        b"2",
        b"-1",
        b"-0",
        b"+1",
        b" 1",
        b"\t1",
        b"\n1",
        b"\x0b1",
        b"1 ",
        b" true",
        b"0x10",
        b"0X1f",
        b"010",
        b"08",
        b"0x",
        b"0xg",
        b"0k",
        b"1k",
        b"1K",
        b"1m",
        b"1g",
        b"2g",
        b"-2g",
        b"1t",
        b"1kk",
        b"k",
        b"-",
        b"+",
        b"--1",
        b"- 1",
        b"2147483647",
        b"2147483648",
        b"-2147483648",
        b"-2147483649",
        b"2097151k",
        b"2097152k",
        b"99999999999999999999",
        b"truex",
        b"t",
        b"y",
        b"1.0",
    ];
    for &value in values {
        let mut text = b"[s]\nk = \"".to_vec();
        for &c in value {
            match c {
                b'\n' => text.extend_from_slice(b"\\n"),
                b'\\' | b'"' => text.extend_from_slice(&[b'\\', c]),
                _ => text.push(c),
            }
        }
        text.extend_from_slice(b"\"\n");
        let git = git_config(dir.path(), &text, &["--type=bool", "--get", "s.k"])
            .ok()
            .map(|out| out.starts_with(b"true"));
        assert_eq!(read_bool(value), git, "{}", shown_text(value));
    }
}

/// A format version is an int as Git reads one: base 0, a unit, within 32 bits.
#[test]
fn format_versions_are_read_as_git_reads_them() {
    let version = |text: &str| format(&format!("[core]\n\trepositoryformatversion = {text}\n"));
    for zero in ["0", "00", "0x0", "-0", "+0", " 0", "\" 0\"", "0k", "0G"] {
        assert!(version(zero).is_ok(), "{zero}");
    }
    for (text, want) in [
        ("0x2", 2),
        ("010", 8),
        ("1K", 1024),
        ("2097151k", 2_147_482_624),
        ("2147483647", 2_147_483_647),
    ] {
        assert!(
            matches!(version(text), Err(GitError::Refused(Refusal::FormatVersion(v))) if v == want),
            "{text}"
        );
    }
    for bad in [
        "",
        "1x",
        "\"1 \"",
        "08",
        "0x",
        "2147483648",
        "2097152k",
        "-2",
        "-2147483649",
        "1t",
    ] {
        assert!(
            matches!(version(bad), Err(GitError::Broken { .. })),
            "{bad}"
        );
    }
    let one = "[core]\n\trepositoryformatversion = 0x1\n[extensions]\n\tobjectformat = sha256\n";
    assert_eq!(format(one).unwrap().hash, HashKind::Sha256);
}

/// The extensions' values are checked as Git checks them, with any version or none, before the
/// version decides which ones count.
#[test]
fn extension_values_are_checked_as_git_checks_them() {
    let v = |version: &str, extensions: &str| {
        format(&format!(
            "[core]\n\trepositoryformatversion = {version}\n[extensions]\n{extensions}"
        ))
    };
    let broken_at = |result: Result<Format, GitError>, line: usize| match result {
        Err(GitError::Broken { what }) => {
            assert_eq!(what, format!("it can't read line {line} of .git/config"));
        }
        Err(other) => panic!("{other:?}"),
        Ok(_) => panic!("read"),
    };
    for (extensions, line) in [
        ("\tobjectFormat = SHA256\n", 4),
        ("\tobjectformat\n", 4),
        ("\tobjectformat =\n", 4),
        ("\tobjectformat = sha\\\n256x\n", 5),
        ("\tcompatObjectFormat = md5\n", 4),
        (
            "\tcompatObjectFormat = sha256\n\tcompatObjectFormat = sha256\n",
            5,
        ),
        ("\trefStorage\n", 4),
        ("\trefStorage = FILES\n", 4),
        ("\trefStorage = files:/x\n", 4),
        ("\trefStorage = files\n\trefStorage = bogus\n", 5),
        ("\tpartialClone\n", 4),
        ("\tworktreeConfig = maybe\n", 4),
        ("\tpreciousObjects = maybe\n", 4),
        ("\trelativeWorktrees = 2g\n", 4),
        ("\tunknown = 1\n\tobjectformat = bogus\n", 5),
    ] {
        broken_at(v("1", extensions), line);
    }
    // Checked under format 0 and 2, and with no version at all.
    broken_at(v("0", "\trefStorage = bogus\n"), 4);
    broken_at(v("2", "\tobjectformat = bogus\n"), 4);
    broken_at(format("[extensions]\n\tworktreeConfig = maybe\n"), 2);
    broken_at(format("[extensions]\n\tobjectformat"), 2);
    // Values Git doesn't check, and those it takes.
    for extensions in [
        "\tnoop = maybe\n\tnoop-v1 = maybe\n",
        "\tworktreeConfig\n\tpreciousObjects = 0\n",
        "\tcompatObjectFormat = sha256\n",
        "\tcompatObjectFormat = sha1\n",
        "\tcompatObjectFormat = sha256\n\tobjectformat = sha256\n",
        "\tobjectformat = \"sha256\"\n\tobjectformat = sha1\n",
    ] {
        assert!(v("1", extensions).is_ok(), "{extensions}");
    }
    // Extensions only version 1 knows are refused under version 0, naming the first.
    for (name, value) in [
        ("noop-v1", "x"),
        ("relativeWorktrees", "true"),
        ("compatObjectFormat", "sha256"),
    ] {
        let extensions = format!("\tnoop = 1\n\t{name} = {value}\n\tobjectformat = sha1\n");
        match v("0", &extensions) {
            Err(GitError::Broken { what }) => {
                let want = format!(
                    ".git/config sets extensions.{}, which needs format version 1, but the \
                     version is 0",
                    name.to_lowercase()
                );
                assert_eq!(what, want);
            }
            other => panic!("{name}: {:?}", other.err()),
        }
    }
    // A subsection makes another extension, which version 1 doesn't know.
    for text in ["[extensions \"x\"]\n\ty = 1\n", "[extensions.x]\n\ty = 1\n"] {
        let result = format(&format!("[core]\n\trepositoryformatversion = 1\n{text}"));
        assert!(
            matches!(&result, Err(GitError::Refused(Refusal::UnknownExtension(name))) if name == "x.y"),
            "{text}"
        );
    }
    // A negative version is refused, though Git reads it as none.
    match v("-1", "\tobjectformat = sha256\n") {
        Err(GitError::Broken { what }) => assert_eq!(
            what,
            ".git/config gives the format version -1, which Git never writes"
        ),
        other => panic!("{:?}", other.err()),
    }
    // Refs kept elsewhere are named before a partial clone.
    for (refs, refusal) in [
        ("files://x", Refusal::RefsElsewhere),
        ("reftable://x", Refusal::Reftable),
    ] {
        let result = v("1", &format!("\tpartialClone = o\n\trefStorage = {refs}\n"));
        assert!(
            matches!(&result, Err(GitError::Refused(r)) if *r == refusal),
            "{refs}"
        );
    }
    // A compatObjectFormat equal to the hash doesn't stop the reading of later lines.
    let result = v(
        "1",
        "\tcompatObjectFormat = sha1\n\trefStorage = reftable\n",
    );
    assert!(matches!(result, Err(GitError::Refused(Refusal::Reftable))));
}

/// Git cuts a name or value short at a NUL; Bolt refuses the line instead, wherever the NUL is
/// but in a comment.
#[test]
fn a_nul_in_a_subsection_or_value_is_refused() {
    for (text, line) in [
        (b"[core \"excludesfile\x00x\"]\n\tfoo = bar\n".as_slice(), 1),
        (b"#\n[core \"a\\\x00\"]\n", 2),
        (b"[core]\n\texcludesFile = a\x00b\n", 2),
        (b"[user]\n\tname = \"a\x00b\"\n", 2),
        (b"[lfs]\n\tstorage = a\\\n\x00b\n", 3),
    ] {
        let shown = shown_text(text);
        assert_eq!(read_user_settings(text), Err(line), "{shown}");
        let want = format!("it can't read line {line} of .git/config");
        assert!(
            matches!(read_format(text, ".git/config"), Err(GitError::Broken { what }) if what == want),
            "{shown}"
        );
        assert!(
            read_worktree(text, ".git/config.worktree").is_err(),
            "{shown}"
        );
    }
    assert_eq!(
        read_user_settings(b"# \x00\n[core]\n\texcludesFile = a\n"),
        Ok(vec![UserSetting::ExcludesFile(b"a".to_vec())])
    );
}

/// Any setting in an `[include]` or `[includeIf]` section is noted, whatever its subsection or
/// value: Bolt follows none of them.
#[test]
fn every_include_setting_is_noted() {
    for (text, includes) in [
        ("[include]\n\tpath\n", true),
        ("[include]\n\tpath =\n", true),
        ("[include]\n\tpath = #x\n", true),
        ("[includeIf \"gitdir:/x/\"]\n\tpath\n", true),
        ("[includeIf.x]\n\tpath = y\n", true),
        ("[includeIf]\n\tpath = y\n", true),
        ("[includeIf]\n sparsecheckoutcone\n", true),
        ("[include \"x\"]\n\ta\n", true),
        ("[include]\n\tother = y\n", true),
        ("[includes]\n\tpath = y\n", false),
        ("[x \"include\"]\n\tpath = y\n", false),
    ] {
        assert_eq!(format(text).unwrap().includes, includes, "{text}");
        let worktree = read_worktree(text.as_bytes(), ".git/config.worktree").unwrap();
        assert_eq!(worktree.includes, includes, "{text}");
    }
    // In the user's settings only a plain `include.path` is followed, and it needs a value.
    assert_eq!(read_user_settings(b"[include]\n\tpath\n"), Err(2));
    assert_eq!(
        read_user_settings(b"[include]\n\tpath =\n"),
        Ok(vec![UserSetting::Include(Vec::new())])
    );
}

/// Git names the second line for a file that is only part of a byte order mark.
#[test]
fn a_byte_order_mark_cut_short_is_refused_where_git_says() {
    for (text, line) in [
        (b"\xef".as_slice(), 2),
        (b"\xef\xbb", 2),
        (b"\xef\n", 2),
        (b"\xef\xbb\r\n", 2),
        (b"\xefx", 1),
        (b"\xef\xbb[core]\n", 1),
    ] {
        assert_eq!(read_user_settings(text), Err(line), "{}", shown_text(text));
    }
}

/// The reasons a count is refused.
#[test]
fn a_count_is_refused_with_its_reason() {
    for (text, why) in [
        ("two", "isn't a number Git reads as a count"),
        ("3 ", "isn't a number Git reads as a count"),
        ("-1", "is more than Git takes"),
        ("2147483648", "is more than Git takes"),
        ("99999999999999999999999", "is more than Git takes"),
    ] {
        assert_eq!(read_count(text.as_bytes()), Err(why), "{text}");
    }
}

/// Settings in a subsection, after a dotted section or in no section aren't the ones Bolt takes.
#[test]
fn only_the_settings_named_are_taken() {
    for text in [
        "[core \"\"]\n\tsparseCheckout = true\n",
        "[core.]\n\tsparseCheckout = true\n",
        "[core.x]\n\tsparseCheckout = true\n",
        "sparseCheckout = true\n",
    ] {
        assert_eq!(format(text).unwrap().sparse, None, "{text}");
        assert_eq!(read_user_settings(text.as_bytes()), Ok(vec![]), "{text}");
    }
    assert_eq!(
        read_user_settings(b"[core]\n\tsparseCheckout = maybe\n\tsparseCheckout\n"),
        Ok(vec![
            UserSetting::SparseCheckout(None),
            UserSetting::SparseCheckout(Some(true))
        ])
    );
    // Each sparseCheckout in the repository is read as Git reads it, even one a later line sets.
    assert!(format("[core]\n\tsparseCheckout = maybe\n\tsparseCheckout = no\n").is_err());
    assert_eq!(
        format("[core]\n\tsparseCheckout = 0x0\n").unwrap().sparse,
        Some(false)
    );
}
