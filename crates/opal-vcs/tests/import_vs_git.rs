//! Differential tests (decision 002): Git repositories made with Git itself, imported, then every
//! commit, tree, file and ref compared with what Git holds. A real repository's turn comes
//! nightly.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use std::fmt::Write as _;
use std::path::Path;
use std::time::Instant;

use opal_core::import::{ImportRequest, import_git};
use opal_core::names::holdable_here;
use opal_core::repo::Repository;
use opal_core::save::Person;
use support::{
    Env, assert_ancestry_matches_git, assert_matches_git, assert_matches_git_counting, copy_dir,
    git, git_with_input, text, top_folder,
};

/// Writes `bytes` into the repository as a blob, and returns its ID.
fn blob(env: &Env, bytes: &[u8]) -> String {
    env.git_input(&["hash-object", "-w", "--stdin"], bytes)
}

#[test]
fn everyday_history_matches_git() {
    let env = Env::new();
    // As on Windows, Git takes no `x` bits from the files, so the mode the index is given below
    // stays in every later commit, on every system.
    env.git(&["config", "core.fileMode", "false"]);
    env.commit("a.txt", "a\n", "First");
    env.commit("src/lib/util.rs", "fn util() {}\n", "Add folders");
    env.overwrite("run.sh", "#!/bin/sh\necho hi\n");
    env.git(&["add", "run.sh"]);
    // Executable as Git marks it, whatever this system does with files.
    env.git(&["update-index", "--chmod=+x", "run.sh"]);
    env.git(&["commit", "-q", "-m", "Add a script"]);
    env.commit("caf\u{e9}/na\u{ef}ve.txt", "\u{e9}\n", "Unicode names");
    env.commit("\u{65e5}\u{672c}.txt", "\u{65e5}\n", "And more");
    env.git(&["mv", "a.txt", "b.txt"]);
    env.git(&["rm", "-q", "src/lib/util.rs"]);
    env.git(&["commit", "-q", "-m", "Rename and delete"]);
    env.git(&["commit", "-q", "--allow-empty", "-m", "Nothing changed"]);

    // A merge, then an octopus merge of two branches at once.
    let base = env.git(&["rev-parse", "HEAD"]);
    for branch in ["one", "two", "three"] {
        env.git(&["checkout", "-q", "-b", branch, &base]);
        env.commit(&format!("{branch}.txt"), "x\n", branch);
    }
    env.git(&["checkout", "-q", "main"]);
    env.git(&["merge", "-q", "--no-ff", "-m", "Merge one", "one"]);
    env.git(&["merge", "-q", "-m", "Merge two and three", "two", "three"]);

    // Entries only the index holds, which no checkout here could make: a link, two names only
    // letter case tells apart, a submodule, and names Opalro leaves out (a `.opalro` folder below the
    // top, and a name Windows takes for `.git`).
    let target = blob(&env, b"b.txt");
    let (upper, lower) = (blob(&env, b"Read me\n"), blob(&env, b"Or me\n"));
    for entry in [
        format!("120000,{target},link"),
        format!("100644,{upper},README"),
        format!("100644,{lower},Readme"),
        format!("160000,{},vendor/lib", "5a".repeat(20)),
        format!("100644,{upper},sub/kept.txt"),
        format!("100644,{upper},sub/.opalro/config"),
        format!("100644,{upper},sub/git~1"),
    ] {
        // Git refuses `git~1` unless told not to guard NTFS.
        let args = ["-c", "core.protectNTFS=false", "update-index", "--add"];
        env.git(&[&args[..], &["--cacheinfo", &entry]].concat());
    }
    env.git(&[
        "commit",
        "-q",
        "-m",
        "A link, twins, a submodule and names Opalro skips",
    ]);

    let head = env.git(&["rev-parse", "HEAD"]);
    // Commits only a server branch, or only a tag, reaches.
    let lone = |message| env.git(&["commit-tree", "HEAD^{tree}", "-p", &base, "-m", message]);
    let (fetched, tagged) = (lone("Only on the server"), lone("Only tagged"));
    env.git(&["branch", "feature+x", &base]);
    env.git(&["branch", "topic/deep/name", "one"]);
    env.git(&["branch", "caf\u{e9}", "two"]);
    env.git(&["update-ref", "refs/remotes/origin/main", &head]);
    env.git(&["update-ref", "refs/remotes/origin/fix+1", &base]);
    env.git(&["update-ref", "refs/remotes/upstream/topic/x", &fetched]);
    env.git(&["tag", "-a", "v2", "-m", "Version 2", &tagged]);
    env.git(&[
        "symbolic-ref",
        "refs/remotes/origin/HEAD",
        "refs/remotes/origin/main",
    ]);
    env.git(&["tag", "light", &base]);
    // A tag keeps characters a line's name can't hold.
    env.git(&["tag", "v1+build", &base]);
    env.git(&["tag", "-a", "v1", "-m", "Version 1", &head]);
    env.git(&["tag", "-a", "nested", "-m", "Again", "v1"]);
    env.git(&["tag", "folder", "HEAD^{tree}"]);
    env.git(&["tag", "-a", "folder-note", "-m", "A folder", "HEAD^{tree}"]);
    env.git(&["tag", "file", "HEAD:b.txt"]);
    // Two branches alike but for letter case, which only packed-refs can hold where the drive
    // ignores case.
    let packed = env.root().join(".git/packed-refs");
    assert!(!packed.exists());
    let twins = format!(
        "# pack-refs with: peeled fully-peeled sorted \n{base} refs/heads/Release\n{base} \
         refs/heads/release\n"
    );
    std::fs::write(packed, twins).unwrap();

    let detached = tempfile::tempdir().unwrap();
    copy_dir(env.root(), detached.path());
    env.import();
    // Git wrote every commit itself, so each save rebuilds its commit from its own fields.
    assert_eq!(assert_matches_git(env.root()), 0);
    // Nine lines across the merges: held ones, ones that hold some, and ones parted at a fork.
    assert_eq!(assert_ancestry_matches_git(env.root()), 64);
    // The renaming rule, pinned: `+` becomes `-`, and of names alike but for letter case, the
    // later one in byte order takes a suffix.
    let (repo, state) = env.state();
    let lines: Vec<&str> = state.lines.keys().map(String::as_str).collect();
    let want = [
        "Release",
        "caf\u{e9}",
        "feature-x",
        "main",
        "one",
        "release-2",
        "three",
        "topic/deep/name",
        "two",
    ];
    assert_eq!(lines, want);
    let servers = repo.server_lines(&state).unwrap().servers;
    let origin: Vec<&str> = servers["origin"].keys().map(String::as_str).collect();
    assert_eq!(origin, ["fix-1", "main"]);

    // The same, with HEAD detached at a commit only it reaches.
    git(detached.path(), &["checkout", "-q", "--detach"]);
    git(
        detached.path(),
        &["commit", "-q", "--allow-empty", "-m", "Only HEAD has this"],
    );
    env.opal_in(detached.path())
        .arg("import")
        .assert()
        .success();
    assert_eq!(assert_matches_git(detached.path()), 0);
    assert_ancestry_matches_git(detached.path());
}

#[test]
fn unusual_commits_match_git() {
    let env = Env::new();
    let a = blob(&env, b"a\n");
    let tree = env.git_input(&["mktree"], format!("100644 blob {a}\ta.txt\n").as_bytes());
    let write = |parents: &[&str], rest: &[u8]| {
        let mut raw = format!("tree {tree}\n");
        for parent in parents {
            let _ = writeln!(raw, "parent {parent}");
        }
        let raw = [raw.as_bytes(), rest].concat();
        let args = [
            "hash-object",
            "-t",
            "commit",
            "--literally",
            "-w",
            "--stdin",
        ];
        env.git_input(&args, &raw)
    };
    let people = "author Ida Lark <ida.lark@example.com> 1601234567 -0400\n\
                  committer Omar Fenn <omar.fenn@example.com> 1601234627 +0545\n";
    let side = write(&[], format!("{people}\nA side commit\n").as_bytes());
    let mut last = write(&[], format!("{people}\nPlain\n").as_bytes());
    let tag = format!(
        "mergetag object {side}\n type commit\n tag v1.0\n \
         tagger Vera Toll <vera.toll@example.com> 1601234500 +0000\n \n Version 1.0\n"
    );
    for (others, rest) in [
        // A signature, its lines continued with a space.
        (
            None,
            format!(
                "{people}gpgsig -----BEGIN PGP SIGNATURE-----\n \n iHUEABYKAB0WIQRNmrJ5\n \
                 =Xd3h\n -----END PGP SIGNATURE-----\n\nSigned\n"
            )
            .into_bytes(),
        ),
        // A merge that carries the tag it merged.
        (
            Some(side.as_str()),
            format!("{people}{tag}\nMerge tag v1.0\n").into_bytes(),
        ),
        // A message in Latin-1, as it says, and one in no encoding at all.
        (
            None,
            [
                format!("{people}encoding ISO-8859-1\n\n").as_bytes(),
                b"Caf\xe9\n",
            ]
            .concat(),
        ),
        (None, [people.as_bytes(), b"\nBad \xff byte\n"].concat()),
        // Jujutsu's change ID.
        (
            None,
            format!("{people}change-id zyxwvutsrqponmlkzyxwvutsrqponmlk\n\nChanged\n").into_bytes(),
        ),
        // A zone written -0000.
        (
            None,
            b"author Ida Lark <ida.lark@example.com> 1601234567 -0000\n\
              committer Omar Fenn <omar.fenn@example.com> 1601234627 -0000\n\nZone\n"
                .to_vec(),
        ),
        // An author line no version of Git writes.
        (
            None,
            b"author Ida Lark <ida.lark@example.com>1601234567 -0400\n\
              committer Omar Fenn <omar.fenn@example.com> 1601234627 +0545\n\nOdd\n"
                .to_vec(),
        ),
        // A header Git doesn't know, continued on a second line.
        (
            None,
            format!("{people}x-unknown some value\n more of it\n\nUnknown\n").into_bytes(),
        ),
        // No newline at the end.
        (None, format!("{people}\nNo newline").into_bytes()),
    ] {
        let parents: Vec<&str> = [Some(last.as_str()), others]
            .into_iter()
            .flatten()
            .collect();
        last = write(&parents, &rest);
    }
    // A parent named twice.
    last = write(&[&last, &last], format!("{people}\nTwice\n").as_bytes());
    env.git(&["update-ref", "refs/heads/main", &last]);
    env.import();
    let counted = assert_matches_git_counting(env.root());
    assert_ancestry_matches_git(env.root());
    // All but the plain two, the one in no encoding and the one without a newline keep their
    // Git header.
    assert_eq!(counted.headers, 8);
    // The odd author line is the only person line not compared with Git's, so a parser that
    // gave up on more of them would show here.
    assert_eq!(counted.odd_people, 1);
}

#[test]
fn packed_and_loose_objects_match_git() {
    let env = Env::new();
    // Sixty versions of one growing file, so Git packs most as deltas.
    let mut stream = String::new();
    let mut file = String::new();
    for n in 0..60 {
        for line in 0..40 {
            let _ = writeln!(file, "Version {n}, line {line}: and the same words again");
        }
        let message = format!("Version {n}\n");
        let _ = write!(
            stream,
            "commit refs/heads/main\ncommitter Ada <ada@example.com> {} +0000\ndata {}\n{message}\
             M 100644 inline big.txt\ndata {}\n{file}\n",
            1_700_000_000 + n,
            message.len(),
            file.len()
        );
    }
    let made = git_with_input(env.root(), &["fast-import", "--quiet"], stream.as_bytes());
    assert!(made.status.success(), "{}", text(&made.stderr));
    env.git(&["repack", "-adfq", "--depth=50"]);
    env.git(&["reset", "-q", "--hard"]);
    // Then some objects loose beside the pack, and every ref packed.
    env.commit("loose.txt", "loose\n", "Loose");
    env.commit("big.txt", &format!("{file}one more line\n"), "Loose too");
    env.git(&["tag", "-a", "v1", "-m", "Version 1"]);
    env.git(&["branch", "old", "HEAD~30"]);
    env.git(&["pack-refs", "--all"]);
    let counts = env.git(&["count-objects", "-v"]);
    assert!(
        counts.contains("packs: 1") && !counts.contains("count: 0"),
        "{counts}"
    );
    env.import();
    assert_eq!(assert_matches_git(env.root()), 0);
    assert_ancestry_matches_git(env.root());
}

#[test]
fn sha256_repositories_match_git() {
    let version = text(&git(&std::env::temp_dir(), &["version"]));
    let mut parts = version
        .split_whitespace()
        .nth(2)
        .unwrap_or_default()
        .split('.');
    let mut number = || match parts.next().map(str::parse::<u32>) {
        Some(Ok(n)) => n,
        _ => panic!("can't read Git's version in {version:?}"),
    };
    if (number(), number()) < (2, 29) {
        println!(
            "Skipped: {} is older than 2.29, the first to make SHA-256 repositories.",
            version.trim()
        );
        return;
    }
    let env = Env::init(&["--object-format=sha256"]);
    env.commit("a.txt", "a\n", "First");
    env.commit("src/b.txt", "b\n", "Second");
    env.git(&["branch", "feature+x", "HEAD~1"]);
    env.git(&["repack", "-adq"]);
    // Then loose objects: a script, a link and a submodule, whose IDs are 32 bytes too.
    env.overwrite("run.sh", "#!/bin/sh\n");
    env.git(&["add", "run.sh"]);
    env.git(&["update-index", "--chmod=+x", "run.sh"]);
    let target = blob(&env, b"a.txt");
    for entry in [
        format!("120000,{target},link"),
        format!("160000,{},vendor/lib", "5a".repeat(32)),
    ] {
        env.git(&["update-index", "--add", "--cacheinfo", &entry]);
    }
    env.git(&["commit", "-q", "-m", "A script, a link and a submodule"]);
    env.git(&["tag", "-a", "v1", "-m", "Version 1"]);
    env.git(&["update-ref", "refs/remotes/origin/main", "HEAD~1"]);
    env.import();
    assert_eq!(assert_matches_git(env.root()), 0);
    assert_ancestry_matches_git(env.root());
}

#[test]
fn optional_git_files_are_ignored() {
    let env = Env::new();
    env.commit("a.txt", "a\n", "First");
    env.commit("b/c.txt", "c\n", "Second");
    env.git(&["branch", "side", "HEAD~1"]);
    env.git(&["tag", "-a", "v1", "-m", "Version 1"]);
    // One pack with a bitmap and a reverse index, a multi-pack index and a commit-graph over it,
    // only packed refs, and a worktree config that turns nothing on.
    env.git(&["-c", "pack.writeReverseIndex=true", "repack", "-adbq"]);
    env.git(&["multi-pack-index", "write"]);
    env.git(&["commit-graph", "write", "--reachable"]);
    env.git(&["pack-refs", "--all"]);
    env.git(&["config", "extensions.worktreeConfig", "true"]);
    env.git(&["config", "--worktree", "core.sparseCheckout", "false"]);
    let git_dir = env.root().join(".git");
    let packs: Vec<String> = std::fs::read_dir(git_dir.join("objects/pack"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    for made in [".bitmap", ".rev", "multi-pack-index"] {
        assert!(packs.iter().any(|f| f.ends_with(made)), "{made}: {packs:?}");
    }
    for made in [
        "objects/info/commit-graph",
        "config.worktree",
        "packed-refs",
    ] {
        assert!(git_dir.join(made).is_file(), "{made}");
    }
    assert!(!git_dir.join("refs/heads/main").exists());
    env.import();
    assert_eq!(assert_matches_git(env.root()), 0);
    assert_ancestry_matches_git(env.root());
}

#[test]
fn windows_cant_hold_some_names_and_saving_keeps_them() {
    let env = Env::new();
    env.commit("a.txt", "a\n", "First");
    // A commit as Linux makes one. Git for Windows refuses these names in its index, so the tree
    // is written whole.
    let files = [
        ("README", "Read me\n"),
        ("Readme", "Or me\n"),
        ("a:b", "A colon\n"),
        ("aux.c", "int aux;\n"),
    ];
    let mut tree = env.git(&["ls-tree", "HEAD"]) + "\n";
    for (name, contents) in files {
        let _ = writeln!(
            tree,
            "100644 blob {}\t{name}",
            blob(&env, contents.as_bytes())
        );
    }
    let tree = env.git_input(&["mktree"], tree.as_bytes());
    let names = "Names Windows can't hold";
    let commit = env.git(&["commit-tree", &tree, "-p", "HEAD", "-m", names]);
    env.git(&["update-ref", "refs/heads/main", &commit]);
    env.import();
    assert_eq!(assert_matches_git(env.root()), 0);
    assert_ancestry_matches_git(env.root());

    // What Git would write out here: each name this system can hold. Where letter case doesn't
    // count, the second of the twins is written over the first.
    for (name, contents) in files {
        if holdable_here(name) {
            env.overwrite(name, contents);
        }
    }
    env.overwrite("new.txt", "new\n");
    let imported = top_folder(env.root());
    let out = env.save(&["Add a file"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    let saved = top_folder(env.root());
    for (name, _) in files {
        assert!(saved.get(name).is_some(), "{name}");
        assert_eq!(saved.get(name), imported.get(name), "{name}");
    }
    assert!(saved.get("new.txt").is_some());
}

/// Git writes the link out itself, as a real link wherever it can make one; the first save after
/// the import keeps the target Git stored, though Git for Windows writes its `/` as `\`.
#[test]
fn a_link_git_wrote_keeps_its_target_when_saved() {
    let env = Env::new();
    env.commit("dir/target.txt", "t\n", "First");
    let target = blob(&env, b"dir/target.txt");
    let entry = format!("120000,{target},link");
    env.git(&["update-index", "--add", "--cacheinfo", &entry]);
    env.git(&["commit", "-q", "-m", "A link"]);
    let args = ["-c", "core.symlinks=true", "checkout", "--", "link"];
    let out = git_with_input(env.root(), &args, b"");
    let link = std::fs::symlink_metadata(env.root().join("link"));
    if !link.is_ok_and(|meta| meta.file_type().is_symlink()) {
        // CI's runners can make links, so a skip there would pass for a test that never ran.
        assert!(
            std::env::var_os("CI").is_none(),
            "Git couldn't make the link in CI: {}",
            text(&out.stderr)
        );
        println!("Skipped: Git can't make links here. {}", text(&out.stderr));
        return;
    }
    env.import();
    env.overwrite("new.txt", "new\n");
    let out = env.save(&["Add a file"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    let repo = Repository::open(env.root()).unwrap();
    let link = top_folder(env.root()).get("link").cloned().unwrap();
    let mut saved = Vec::new();
    repo.store().copy_content(&link.id, &mut saved).unwrap();
    assert_eq!(text(&saved), "dir/target.txt");
}

#[test]
#[ignore = "nightly: needs OPALRO_DIFF_REPO, a full clone of a real repository"]
fn a_real_repository_matches_git() {
    let clone = std::env::var_os("OPALRO_DIFF_REPO").expect("OPALRO_DIFF_REPO names a clone");
    let shallow = text(&git(
        Path::new(&clone),
        &["rev-parse", "--is-shallow-repository"],
    ));
    assert_eq!(
        shallow.trim(),
        "false",
        "OPALRO_DIFF_REPO must be a full clone"
    );
    // Its .git copied, so the clone stays as it was and the test can run again.
    let work = tempfile::tempdir().unwrap();
    copy_dir(&Path::new(&clone).join(".git"), &work.path().join(".git"));
    // A clone has one branch of its own; the server's others become branches too, so the ancestry
    // walk has pairs of lines to compare.
    let fetched = text(&git(
        work.path(),
        &[
            "for-each-ref",
            "--format=%(refname:lstrip=3)",
            "refs/remotes/origin",
        ],
    ));
    for branch in fetched.lines().filter(|&branch| branch != "HEAD") {
        let (local, remote) = (
            format!("refs/heads/{branch}"),
            format!("refs/remotes/origin/{branch}"),
        );
        git(work.path(), &["update-ref", &local, &remote]);
    }
    let who = Person {
        name: "Test".into(),
        email: "test@example.com".into(),
        time_ms: 0,
        tz_offset_min: 0,
    };
    let began = Instant::now();
    // Here in this process, so the peak memory below is the import's own.
    let report = import_git(
        work.path(),
        &mut ImportRequest {
            who: &who,
            command: "import",
            git_settings: None,
            progress: &mut |_, _, _| {},
        },
        &mut None,
    );
    let took = began.elapsed();
    let report = report.unwrap_or_else(|e| panic!("{e}\n{}", e.hint().unwrap_or_default()));
    println!(
        "Imported {} commits in {:.1} s, peak memory {}: {} lines, {} server lines, {} tags, {} \
         commits not exact, {} entries skipped.",
        report.commits,
        took.as_secs_f64(),
        peak_memory(),
        report.lines,
        report.server_lines,
        report.tags,
        report.inexact_commits,
        report.skipped_entry_count
    );
    let began = Instant::now();
    let headers = assert_matches_git(work.path());
    println!(
        "Every commit, tree, file and ref matches Git; {headers} commits keep their Git header \
         ({:.1} s to compare).",
        began.elapsed().as_secs_f64()
    );
    assert_ancestry_matches_git(work.path());
}

/// The most memory this process has held, as Linux reports it (VmHWM). Other systems need a
/// system call to tell.
fn peak_memory() -> String {
    let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    match status.lines().find_map(|line| line.strip_prefix("VmHWM:")) {
        Some(kb) => {
            let kb: u64 = kb.trim().trim_end_matches("kB").trim().parse().unwrap();
            format!("{} MiB", kb / 1024)
        }
        None => "not measured on this system".to_owned(),
    }
}
