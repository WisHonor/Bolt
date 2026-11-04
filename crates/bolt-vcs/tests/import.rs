//! `bolt import` end to end: repositories made with Git itself, imported, then read back with
//! Bolt's own commands.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use std::path::Path;

use bolt_core::diary::Position;
use bolt_core::repo::Repository;
use support::{Env, copy_dir, git, git_commit, git_with_input, text};

#[test]
fn a_simple_repository_imports() {
    let env = Env::new();
    let first = env.commit("a.txt", "a\n", "First save");
    env.git(&["branch", "feature"]);
    let second = env.commit("b.txt", "b\n", "Second save\n\nWith a body.");
    env.git(&["tag", "-a", "v1", "-m", "Version 1", &first]);
    let (out, _) = env.import();
    assert!(
        out.contains("Imported 2 commits from Git: 2 lines, 1 tag."),
        "{out}"
    );
    assert!(out.contains("You're on line main."), "{out}");

    let history = env.bolt().arg("history").output().unwrap();
    assert_eq!(history.status.code(), Some(0), "{}", text(&history.stderr));
    let history = text(&history.stdout);
    let lines: Vec<&str> = history.lines().collect();
    assert_eq!(lines.len(), 2, "{history}");
    assert!(lines[0].contains("Ada Lovelace") && lines[0].contains("Second save"));
    assert!(lines[1].contains("Ada Lovelace") && lines[1].contains("First save"));

    let show = env.bolt().arg("show").output().unwrap();
    assert_eq!(show.status.code(), Some(0), "{}", text(&show.stderr));
    let show = text(&show.stdout);
    assert!(show.contains("Ada Lovelace") && show.contains("    With a body."));
    assert!(show.contains("added     b.txt"), "{show}");

    let (repo, state) = env.state();
    assert_eq!(state.position, Position::OnLine("main".into()));
    assert_eq!(git_commit(&repo, &state.lines["main"]), second);
    assert_eq!(git_commit(&repo, &state.lines["feature"]), first);
    let tags = repo.tag_list(&state).unwrap();
    let v1 = &tags.tags["v1"];
    assert_eq!(git_commit(&repo, &v1.save), first);
    assert_eq!(v1.message, "Version 1\n");
    assert_eq!(v1.git_tags.len(), 1);
    assert!(state.server_lines.is_none());
}

#[test]
fn server_branches_become_server_lines() {
    let env = Env::new();
    let first = env.commit("a.txt", "a\n", "First");
    let second = env.commit("a.txt", "b\n", "Second");
    env.git(&["update-ref", "refs/remotes/origin/x", &first]);
    env.git(&["update-ref", "refs/remotes/origin/main", &second]);
    env.git(&[
        "symbolic-ref",
        "refs/remotes/origin/HEAD",
        "refs/remotes/origin/main",
    ]);
    let (out, _) = env.import();
    assert!(
        out.contains("Imported 2 commits from Git: 1 line, 2 server lines from 1 server, 0 tags."),
        "{out}"
    );
    // The server's own HEAD only names its default branch, so it isn't listed as left out.
    assert!(!out.contains("HEAD"), "{out}");
    let (repo, state) = env.state();
    let servers = repo.server_lines(&state).unwrap().servers;
    assert_eq!(servers.len(), 1);
    let origin = &servers["origin"];
    assert_eq!(origin.len(), 2);
    assert_eq!(git_commit(&repo, &origin["x"]), first);
    assert_eq!(git_commit(&repo, &origin["main"]), second);
    assert_eq!(state.lines.len(), 1);
}

#[test]
fn names_bolt_refuses_are_renamed_and_listed() {
    let env = Env::new();
    let id = env.commit("a.txt", "a\n", "First");
    // Two names that differ only by letter case can't both be loose refs where the drive
    // ignores case, so they are packed.
    let packed = format!(
        "# pack-refs with: peeled fully-peeled sorted \n{id} refs/heads/Feature\n{id} \
         refs/heads/feature\n{id} refs/heads/feature+x\n"
    );
    std::fs::write(env.root().join(".git/packed-refs"), packed).unwrap();
    let (out, _) = env.import();
    assert!(
        out.contains("Renamed 2 names that Bolt's names don't allow:\n"),
        "{out}"
    );
    assert!(out.contains("\n  feature → feature-2\n"), "{out}");
    assert!(out.contains("\n  feature+x → feature-x\n"), "{out}");
    let (_, state) = env.state();
    let lines: Vec<&str> = state.lines.keys().map(String::as_str).collect();
    assert_eq!(lines, ["Feature", "feature-2", "feature-x", "main"]);

    // The JSON document names each one with its kind.
    let other = Env::new();
    other.commit("a.txt", "a\n", "First");
    other.git(&["branch", "feature+x"]);
    let out = other.bolt().args(["--json", "import"]).output().unwrap();
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(
        json["renamed"],
        serde_json::json!([{"kind": "line", "git": "feature+x", "bolt": "feature-x"}])
    );
}

/// A mirror of a busy repository holds a pull-request ref for every pull request.
#[test]
fn refs_left_out_are_listed_up_to_100_and_counted() {
    let env = Env::new();
    let id = env.commit("a.txt", "a\n", "First");
    let mut packed = "# pack-refs with: peeled fully-peeled sorted \n".to_owned();
    let mut pulls: Vec<String> = (1..=150).map(|n| format!("refs/pull/{n}/head")).collect();
    pulls.sort();
    for name in &pulls {
        packed += &format!("{id} {name}\n");
    }
    std::fs::write(env.root().join(".git/packed-refs"), packed).unwrap();
    let (out, _) = env.import();
    assert!(
        out.contains("Didn't import 150 refs (only branches, server branches and tags come in):"),
        "{out}"
    );
    assert!(out.contains(&format!("\n  {}\n", pulls[99])), "{out}");
    assert!(!out.contains(&format!("\n  {}\n", pulls[100])), "{out}");
    assert!(out.contains("\n  and 50 more\n"), "{out}");

    let copy = Env::new();
    copy.commit("a.txt", "a\n", "First");
    let id = copy.git(&["rev-parse", "HEAD"]);
    let packed: String = pulls.iter().map(|name| format!("{id} {name}\n")).collect();
    std::fs::write(copy.root().join(".git/packed-refs"), packed).unwrap();
    let out = copy.bolt().args(["--json", "import"]).output().unwrap();
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(json["skipped_refs"]["count"], 150);
    let first = json["skipped_refs"]["first"].as_array().unwrap();
    assert_eq!(first.len(), 100);
    assert_eq!(first[0], pulls[0].as_str());
}

#[test]
fn a_detached_head_puts_you_at_its_save() {
    let env = Env::new();
    let first = env.commit("a.txt", "a\n", "First");
    env.commit("a.txt", "b\n", "Second");
    env.git(&["checkout", "-q", "--detach", &first]);
    let (out, _) = env.import();
    assert!(
        out.contains(&format!(
            "where Git's HEAD was detached: not on a line, so bolt save won't work here.\n\
             {NOT_ON_A_LINE}\n"
        )),
        "{out}"
    );
    let (repo, state) = env.state();
    let Position::AtSave(save) = state.position else {
        panic!("{:?}", state.position);
    };
    assert_eq!(git_commit(&repo, &save), first);
    // `history` starts from there.
    let history = env.bolt().arg("history").output().unwrap();
    let history = text(&history.stdout);
    assert_eq!(history.lines().count(), 1, "{history}");
    // There is no line for a save to go on, and the hint names only commands that exist.
    env.overwrite("a.txt", "changed\n");
    let out = env.save(&["Change it"]);
    assert_eq!(out.status.code(), Some(1), "{}", text(&out.stdout));
    let err = text(&out.stderr);
    assert!(
        err.contains(&format!(
            "error: You are not on a line, so the save has nowhere to go.\nhint: {NOT_ON_A_LINE}\n"
        )),
        "{err}"
    );
}

const NOT_ON_A_LINE: &str = "This version of Bolt can only save on a line. Delete the hidden \
    .bolt folder here, switch Git to a branch (`git switch <branch>`) and run bolt import again, \
    or import a fresh clone that is on a branch.";

#[test]
fn an_unborn_branch_stops_the_import() {
    let env = Env::new();
    env.commit("a.txt", "a\n", "First");
    env.git(&["checkout", "-q", "--orphan", "new"]);
    let err = env.refused_import();
    assert!(
        err.contains("Git's current branch `new` has no commits yet; switch to a branch with commits, or commit first."),
        "{err}"
    );
    assert!(!env.root().join(".bolt").exists());
}

#[test]
fn an_empty_repository_stops_the_import() {
    let env = Env::new();
    let err = env.refused_import();
    assert!(
        err.contains(
            "There's nothing to import: this Git repository has no commits. bolt start makes a \
             new project here."
        ),
        "{err}"
    );
    assert!(!env.root().join(".bolt").exists());
}

/// A Git LFS pointer, as Git LFS writes it.
const POINTER: &str = "version https://git-lfs.github.com/spec/v1\noid sha256:\
    4d7a214614ab2935c943f9e0ff69d22eadbb8f32b1258daaa5e2ca24d17e2393\nsize 12345\n";

/// Once `.bolt` is begun, "nothing was changed" isn't so: the hint says what became of it. A stop
/// the repository causes comes again, so the hint doesn't say that running it again finishes.
#[test]
fn an_import_that_stops_partway_says_its_bolt_folder_is_kept() {
    let env = Env::new();
    // With no sign of Git LFS but the pointer itself, only the converter finds it.
    env.commit("art/big.psd", POINTER, "Add a big file");
    let err = env.refused_import();
    assert!(
        err.contains("holds a Git LFS pointer at `art/big.psd`"),
        "{err}"
    );
    assert!(
        err.contains(
            "Nothing was committed. Bolt kept the .bolt folder it began here: bolt import stops \
             the same way until the cause is fixed, then finishes from what it stored. Or delete \
             that folder."
        ),
        "{err}"
    );
    assert!(!err.contains("Nothing was changed"), "{err}");
    assert!(env.root().join(".bolt").is_dir());
    assert!(!env.root().join(".bolt/format").exists());

    // Other commands say what the folder is, and send nobody to `bolt start` alone.
    for args in [&["save", "Save it"][..], &["history"], &["show"]] {
        let out = env.bolt().args(args).output().unwrap();
        assert_eq!(out.status.code(), Some(1), "{args:?}");
        let err = text(&out.stderr);
        assert!(
            err.contains("error: An import or start began a Bolt project at ")
                && err.contains(
                    "hint: Run `bolt import` (or `bolt start`, if that began it) in that folder \
                     again to finish, or delete its hidden .bolt folder.\n"
                ),
            "{args:?}: {err}"
        );
    }
}

/// An import that stored files before it stopped is finished by `bolt import`, never covered by an
/// empty project.
#[cfg(feature = "fault-injection")]
#[test]
fn start_refuses_a_folder_an_unfinished_import_stored_files_in() {
    let env = Env::new();
    env.commit("a.txt", "a\n", "First");
    let out = env
        .bolt()
        .arg("import")
        .env("BOLT_FAULT_FAIL", "commit:before-head")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1), "{}", text(&out.stdout));
    assert!(
        text(&out.stderr).contains("running bolt import again can finish the import"),
        "{}",
        text(&out.stderr)
    );
    let out = env.bolt().arg("start").output().unwrap();
    assert_eq!(out.status.code(), Some(1), "{}", text(&out.stdout));
    let err = text(&out.stderr);
    assert!(
        err.contains(
            "error: An import began a Bolt project here and didn't finish, and bolt start won't \
             make an empty project over what it stored.\nhint: Run `bolt import` again to finish \
             the import, or delete the hidden .bolt folder and run `bolt start` again.\n"
        ),
        "{err}"
    );
    assert!(!env.root().join(".bolt/format").exists());
    let (out, _) = env.import();
    assert!(out.contains("Imported 1 commit from Git"), "{out}");
}

#[test]
fn a_second_import_is_refused() {
    let env = Env::new();
    env.commit("a.txt", "a\n", "First");
    env.import();
    let err = env.refused_import();
    assert!(
        err.contains(
            "This folder is already a Bolt project. Picking up new Git commits comes in a later \
             version"
        ),
        "{err}"
    );
}

#[test]
fn another_tools_bolt_folder_is_refused() {
    let env = Env::new();
    env.commit("a.txt", "a\n", "First");
    let bolt = env.root().join(".bolt");
    std::fs::create_dir(&bolt).unwrap();
    std::fs::write(bolt.join("config.json"), "{}").unwrap();
    let err = env.refused_import();
    assert!(
        err.contains(
            "There's a .bolt folder here that Bolt didn't make (bolt.new and other tools use \
             that name); move or rename it first."
        ),
        "{err}"
    );
    // Nothing was made in it.
    assert_eq!(std::fs::read_dir(&bolt).unwrap().count(), 1);
}

/// Git checks such a folder out where Bolt keeps the project, and Bolt never takes files Git
/// wrote for its own, even when their names are the ones Bolt uses.
#[test]
fn a_bolt_folder_git_tracks_stops_the_import() {
    let env = Env::new();
    env.commit(
        ".bolt/config.toml",
        "[user]\nname = \"Mallory\"\n",
        "Track it",
    );
    let err = env.refused_import();
    assert!(
        err.contains(
            "Git tracks `.bolt` at the top of this project, and Bolt can't use it as its own \
             .bolt folder."
        ),
        "{err}"
    );
    assert!(err.contains("git rm -r --cached .bolt"), "{err}");
    // Nothing was made in it.
    let inside: Vec<_> = std::fs::read_dir(env.root().join(".bolt"))
        .unwrap()
        .collect();
    assert_eq!(inside.len(), 1);
}

/// What Git checked out decides the first look at `.bolt`, which would say it is a project
/// already, another tool's folder, or damaged; the import says what is really wrong.
#[test]
fn a_bolt_git_tracks_is_named_whatever_it_holds() {
    for (file, refused) in [
        (".bolt/format", "already a Bolt project"),
        (".bolt/prompt", "Bolt didn't make"),
        (".bolt", "damaged"),
    ] {
        let env = Env::new();
        env.commit(file, "not Bolt's\n", "Track it");
        let err = env.refused_import();
        assert!(
            err.contains("Git tracks `.bolt` at the top of this project"),
            "{file}: {err}"
        );
        assert!(!err.contains(refused), "{file}: {err}");
        assert!(!env.root().join(".bolt/state").exists(), "{file}");
    }
}

#[test]
fn import_inside_another_project_is_refused() {
    let env = Env::new();
    let outer = tempfile::tempdir().unwrap();
    env.bolt_in(outer.path()).arg("start").assert().success();
    let inner = outer.path().join("repo");
    std::fs::create_dir(&inner).unwrap();
    git(&inner, &["init", "-q"]);
    std::fs::write(inner.join("a.txt"), "a").unwrap();
    git(&inner, &["add", "-A"]);
    git(&inner, &["commit", "-q", "-m", "First"]);
    let out = env.bolt_in(&inner).arg("import").output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(
        text(&out.stderr).contains("inside a Bolt project"),
        "{}",
        text(&out.stderr)
    );
    assert!(!inner.join(".bolt").exists());
}

#[test]
fn the_same_repository_imports_to_the_same_saves_anywhere() {
    let env = Env::new();
    let first = env.commit("a.txt", "a\n", "First");
    env.git(&["branch", "feature+x"]);
    env.commit("b/c.txt", "c\n", "Second");
    env.git(&["tag", "-a", "v1", "-m", "Version 1", &first]);
    env.git(&["update-ref", "refs/remotes/origin/main", &first]);
    let copy = tempfile::tempdir().unwrap();
    copy_dir(env.root(), copy.path());
    env.import();
    env.bolt_in(copy.path()).arg("import").assert().success();

    let (_, here) = env.state();
    let there = Repository::open(copy.path()).unwrap().current().unwrap().1;
    assert_eq!(here, there.state);
    let saves = |dir: &Path| {
        let out = env
            .bolt_in(dir)
            .args(["--json", "history"])
            .output()
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
        json["saves"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| (s["id"].clone(), s["change_id"].clone()))
            .collect::<Vec<_>>()
    };
    assert_eq!(saves(env.root()), saves(copy.path()));
}

#[test]
fn nested_bolt_folders_are_warned_about() {
    let env = Env::new();
    env.commit("a.txt", "a\n", "First");
    env.commit(
        "sub/.bolt/format",
        "not Bolt's\n",
        "Add a folder named .bolt",
    );
    let (out, err) = env.import();
    assert!(
        err.contains(
            "warning: Git checked out a folder named .bolt at `sub/.bolt`. Bolt left it out of \
             history, but commands run inside it would use it as a Bolt project; delete it \
             unless you made it."
        ),
        "{err}"
    );
    assert!(out.contains("sub/.bolt"), "{out}");
    let json = env.bolt().args(["--json", "show"]).output().unwrap();
    let json: serde_json::Value = serde_json::from_slice(&json.stdout).unwrap();
    // The commit that added only the folder adds nothing in Bolt.
    assert_eq!(json["changes"], serde_json::json!([]));
}

/// Bolt doesn't use Git's personal exclude list (decision 038), so the import says when it hides
/// files; Git's own sample of one holds only comments.
#[test]
fn a_personal_exclude_list_with_patterns_is_warned_about() {
    let import_with = |exclude: &str| {
        let env = Env::new();
        env.commit("a.txt", "a\n", "First");
        let info = env.root().join(".git/info");
        std::fs::create_dir_all(&info).unwrap();
        std::fs::write(info.join("exclude"), exclude).unwrap();
        let out = env.bolt().args(["--json", "import"]).output().unwrap();
        assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
        let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
        let warnings = json["warnings"].as_array().unwrap().clone();
        warnings
            .into_iter()
            .find(|w| w["kind"] == "info_exclude_ignored")
            .map(|w| w["message"].as_str().unwrap().to_owned())
    };
    let sample = "# git ls-files --others --exclude-from=.git/info/exclude\n# Lines that start \
                  with '#' are comments.\n\n   \n";
    assert_eq!(import_with(sample), None);
    let message = import_with(&format!("{sample}secrets.env\r\n*.log\n")).unwrap();
    assert_eq!(
        message,
        "Git's personal exclude list here, .git/info/exclude, has 2 patterns. Bolt doesn't use \
         Git's personal exclude lists, so files they hide will be saved; copy the lines into \
         .boltignore to keep those files out."
    );
}

#[test]
fn progress_is_not_printed_without_a_terminal() {
    let env = Env::new();
    for i in 0..3 {
        env.commit("a.txt", &format!("{i}\n"), &format!("Save {i}"));
    }
    let (_, err) = env.import();
    assert!(!err.contains("Importing commits"), "{err}");
}

/// A repository made whole by `git fast-import`, so its commits, and their IDs, are the same on
/// every computer.
const STREAM: &str = "blob\nmark :1\ndata 2\na\n\n\
commit refs/heads/main\nmark :2\n\
author Ada Lovelace <ada@example.com> 1700000000 +0000\n\
committer Ada Lovelace <ada@example.com> 1700000000 +0000\n\
data 6\nFirst\n\nM 100644 :1 a.txt\n\n\
blob\nmark :3\ndata 0\n\n\
commit refs/heads/main\nmark :4\n\
author Ada Lovelace <ada@example.com> 1700000100 +0000\n\
committer Ada Lovelace <ada@example.com> 1700000100 +0000\n\
data 7\nSecond\n\nfrom :2\nM 100644 :3 sub/.bolt/config\n\n\
reset refs/heads/feature+x\nfrom :2\n\n\
reset refs/remotes/origin/main\nfrom :4\n\n\
tag v1\nfrom :2\ntagger Ada Lovelace <ada@example.com> 1700000200 +0000\ndata 11\nVersion 1.\n\n\
reset refs/notes/commits\nfrom :2\n\n";

#[test]
fn import_json_output() {
    let env = Env::new();
    let made = git_with_input(env.root(), &["fast-import", "--quiet"], STREAM.as_bytes());
    assert!(made.status.success(), "{}", text(&made.stderr));
    let tree = env.git(&["rev-parse", "main^{tree}"]);
    env.git(&["update-ref", "refs/tags/folder", &tree]);
    env.git(&[
        "symbolic-ref",
        "refs/remotes/origin/HEAD",
        "refs/remotes/origin/main",
    ]);
    std::fs::create_dir_all(env.root().join(".git/info")).unwrap();
    std::fs::write(env.root().join(".git/info/grafts"), "").unwrap();
    let out = env.bolt().args(["--json", "import"]).output().unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    let mut json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    // A note that depends on the computer stays out of the snapshot, as in `start`'s.
    if bolt_core::repo::elevated_only_note(env.root()).is_some() {
        let warnings = json["warnings"].as_array_mut().unwrap();
        assert_eq!(warnings.remove(0)["kind"], "elevated_only");
    }
    insta::assert_json_snapshot!(json);
}

const CRLF_HINT: &str = "To keep history as Git stored it, commit or stash your changes in Git, \
    then run `git config core.autocrlf false`, `git rm --cached -r -q .` and `git reset --hard`, \
    and save again. To save the files with CRLF anyway: bolt save --accept-crlf \"message\"";

/// A repository as Git for Windows leaves it: stored with LF, written out with CRLF.
fn converted_by_git() -> Env {
    let env = Env::new();
    env.commit("a.txt", "one\ntwo\n", "First");
    env.commit("src/b.txt", "three\nfour\n", "Second");
    env.commit("notes.txt", "five\nsix\n", "Third");
    env.import();
    env.overwrite("a.txt", "one\r\ntwo\r\n");
    env.overwrite("src/b.txt", "three\r\nfour\r\n");
    env
}

#[test]
fn converted_line_endings_stop_the_first_save() {
    let env = converted_by_git();
    // An edit of the user's own is not what stops it.
    env.overwrite("notes.txt", "five\nsix\nseven\n");
    let before = env.standing();

    let out = env.save(&["Save it"]);
    assert_eq!(out.status.code(), Some(1), "{}", text(&out.stdout));
    let err = text(&out.stderr);
    assert!(
        err.contains(
            "error: 2 files differ from what Git stored only in line endings: Git turned LF \
             into CRLF when it wrote them out (core.autocrlf).\n  a.txt\n  src/b.txt\n"
        ),
        "{err}"
    );
    assert!(!err.contains("notes.txt"), "{err}");
    assert!(err.contains(&format!("hint: {CRLF_HINT}\n")), "{err}");
    // Nothing was written: not a save, not an object.
    assert_eq!(env.standing(), before);
    let history = env.bolt().arg("history").output().unwrap();
    assert_eq!(text(&history.stdout).lines().count(), 3);

    let out = env
        .bolt()
        .args(["--json", "save", "Save it"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(json["error"]["hint"], CRLF_HINT);
    let message = json["error"]["message"].as_str().unwrap();
    assert!(
        message.starts_with("2 files differ from what Git stored only"),
        "{message}"
    );
    assert_eq!(env.standing(), before);
}

#[test]
fn accept_crlf_saves_them() {
    let env = converted_by_git();
    let out = env.save(&["--accept-crlf", "Keep the CRLF"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    assert!(
        text(&out.stdout).contains("(2 files)"),
        "{}",
        text(&out.stdout)
    );

    // Saved as they are: each line ending is a byte longer than it was.
    let shown = env.bolt().args(["--json", "show"]).output().unwrap();
    let json: serde_json::Value = serde_json::from_slice(&shown.stdout).unwrap();
    let sizes: Vec<_> = json["changes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| (c["path"].as_str().unwrap(), &c["old_size"], &c["new_size"]))
        .collect();
    assert_eq!(
        sizes,
        [
            ("a.txt", &8.into(), &10.into()),
            ("src/b.txt", &11.into(), &13.into())
        ]
    );
}

/// Each of these is the right size to be a conversion, and none is one.
#[test]
fn a_real_edit_is_not_mistaken_for_a_conversion() {
    let env = Env::new();
    for file in ["extra", "mixed", "edited", "cut"] {
        env.commit(file, "one\ntwo\nthree\n", file);
    }
    // Saved with CRLF in Git already, so converting it again would double the CR.
    env.commit("already", "a\r\nb\r\n", "already");
    env.import();
    env.overwrite("extra", "one\r\ntwo\r\nthree\r\nfour\r\n");
    env.overwrite("mixed", "one\r\ntwo\nthree\r\n");
    env.overwrite("edited", "ONE\r\ntwo\r\nthree\r\n");
    env.overwrite("cut", "one\r\ntwo\r\nthree");
    env.overwrite("already", "a\r\r\nb\r\r\n");

    let out = env.save(&["Real changes"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    assert!(
        text(&out.stdout).contains("(5 files)"),
        "{}",
        text(&out.stdout)
    );
}

#[test]
fn later_saves_dont_check() {
    let env = converted_by_git();
    let first = env.save(&["--accept-crlf", "Keep the CRLF"]);
    assert_eq!(first.status.code(), Some(0), "{}", text(&first.stderr));
    // The save before this one is Bolt's, not Git's.
    env.overwrite("notes.txt", "five\r\nsix\r\n");
    let out = env.save(&["Another"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    assert!(
        text(&out.stdout).contains("(1 file)"),
        "{}",
        text(&out.stdout)
    );
}

/// The hint's own Git commands, where Git for Windows wrote the files with CRLF and refreshed its
/// index, so their stat data matches it and `git checkout` alone would leave them as they are.
#[test]
fn following_the_line_ending_hint_saves_what_git_stored() {
    let env = Env::new();
    env.commit("a.txt", "one\ntwo\n", "First");
    env.commit("src/b.txt", "three\nfour\n", "Second");
    env.import();
    let crlf = |args: &[&str]| env.git(&[&["-c", "core.autocrlf=true"], args].concat());
    for file in ["a.txt", "src/b.txt"] {
        std::fs::remove_file(env.root().join(file)).unwrap();
    }
    crlf(&["checkout", "--", "."]);
    crlf(&["status", "--short"]);
    // An edit made in an editor that keeps the CRLF.
    env.overwrite("a.txt", "one\r\ntwo\r\nthree\r\n");
    let out = env.save(&["Add a line"]);
    assert_eq!(out.status.code(), Some(1), "{}", text(&out.stdout));
    let err = text(&out.stderr);
    assert!(err.contains("error: 1 file differs from what Git stored only in line endings"));
    assert!(err.contains(&format!("hint: {CRLF_HINT}\n")), "{err}");

    // What the hint says, in its order: the change committed first, which Git stores with LF.
    crlf(&["commit", "-q", "-a", "-m", "Add a line"]);
    env.git(&["config", "core.autocrlf", "false"]);
    env.git(&["rm", "--cached", "-r", "-q", "."]);
    env.git(&["reset", "--hard"]);
    let read = |file: &str| std::fs::read(env.root().join(file)).unwrap();
    assert_eq!(read("a.txt"), b"one\ntwo\nthree\n");
    assert_eq!(read("src/b.txt"), b"three\nfour\n");
    let out = env.save(&["Add a line"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    assert!(
        text(&out.stdout).contains("(1 file)"),
        "{}",
        text(&out.stdout)
    );
    let shown = env.bolt().args(["--json", "show"]).output().unwrap();
    let json: serde_json::Value = serde_json::from_slice(&shown.stdout).unwrap();
    assert_eq!(json["changes"][0]["path"], "a.txt");
    assert_eq!(json["changes"][0]["new_size"], 14);
}

/// Not bytes the tests wrote: a file as Git itself writes it out with `core.autocrlf`.
#[test]
fn a_file_git_wrote_with_crlf_is_found() {
    let env = Env::new();
    env.commit("a.txt", "one\ntwo\n", "First");
    env.import();
    std::fs::remove_file(env.root().join("a.txt")).unwrap();
    env.git(&["-c", "core.autocrlf=true", "checkout", "--", "a.txt"]);
    assert_eq!(
        std::fs::read(env.root().join("a.txt")).unwrap(),
        b"one\r\ntwo\r\n"
    );

    let out = env.save(&["Save it"]);
    assert_eq!(out.status.code(), Some(1), "{}", text(&out.stdout));
    let err = text(&out.stderr);
    assert!(
        err.contains("error: 1 file differs from what Git stored only in line endings"),
        "{err}"
    );
}
