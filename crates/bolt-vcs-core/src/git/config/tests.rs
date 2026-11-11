#![allow(clippy::unwrap_used)]
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
