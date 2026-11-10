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
    assert!(out.contains("\n  feature -> feature-2\n"), "{out}");
    assert!(out.contains("\n  feature+x -> feature-x\n"), "{out}");
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

/// The summary lists the first 100 of each and counts the rest; the JSON document lists them all.
#[test]
fn renamed_names_and_skipped_tags_are_listed_up_to_100_and_counted() {
    let env = Env::new();
    let id = env.commit("a.txt", "a\n", "First");
    let tree = env.git(&["rev-parse", "HEAD^{tree}"]);
    let mut packed = "# pack-refs with: peeled fully-peeled sorted \n".to_owned();
    for n in 0..150 {
        packed += &format!("{id} refs/heads/b+{n:03}\n");
    }
    for n in 0..150 {
        packed += &format!("{tree} refs/tags/t{n:03}\n");
    }
    std::fs::write(env.root().join(".git/packed-refs"), &packed).unwrap();
    let copy = tempfile::tempdir().unwrap();
    copy_dir(env.root(), copy.path());
    let (out, _) = env.import();
    for heading in [
        "Renamed 150 names that Bolt's names don't allow:\n",
        "Didn't import 150 tags:\n",
    ] {
        let list = &out[out.find(heading).unwrap() + heading.len()..];
        let items: Vec<&str> = list.lines().take_while(|l| l.starts_with("  ")).collect();
        assert_eq!(items.len(), 101, "{out}");
        assert_eq!(items[100], "  and 50 more", "{out}");
    }
    assert!(out.contains("\n  b+000 -> b-000\n"), "{out}");
    assert!(
        out.contains("\n  t000, as it tags a folder (a Git tree), not a commit\n"),
        "{out}"
    );

    let out = env
        .bolt_in(copy.path())
        .args(["--json", "import"])
        .output()
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(json["renamed"].as_array().unwrap().len(), 150);
    assert_eq!(json["skipped_tags"].as_array().unwrap().len(), 150);
}

/// Names left out of history show every hidden character, so none passes for another.
#[test]
fn entries_left_out_show_their_hidden_characters() {
    let env = Env::new();
    let stream = "blob\nmark :1\ndata 2\na\n\n\
        commit refs/heads/main\nmark :2\n\
        author Ada Lovelace <ada@example.com> 1700000000 +0000\n\
        committer Ada Lovelace <ada@example.com> 1700000000 +0000\n\
        data 6\nFirst\n\nM 100644 :1 a.txt\nM 100644 :1 .g\u{200c}it/x\n\n";
    // Git for macOS refuses this spelling of .git unless told not to.
    let args = ["-c", "core.protectHFS=false", "fast-import", "--quiet"];
    let made = git_with_input(env.root(), &args, stream.as_bytes());
    assert!(made.status.success(), "{}", text(&made.stderr));
    let (out, _) = env.import();
    assert!(
        out.contains(
            "Left out 1 entry whose name Bolt never writes to disk (such as .git and \
                      .bolt), with everything in it: `.g\\u{200c}it` in commit "
        ),
        "{out}"
    );
}

#[test]
fn a_detached_head_puts_you_at_its_save() {
    let env = Env::new();
    let first = env.commit("a.txt", "a\n", "First");
    env.commit("a.txt", "b\n", "Second");
    env.git(&["checkout", "-q", "--detach", &first]);
    let (out, _) = env.import();
    assert!(
        out.contains(
            "where Git's HEAD was detached: not on a line. Saving here asks for a new line's \
             name.\n"
        ),
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
    // A save there needs a new line, and the hint says how to name it.
    env.overwrite("a.txt", "changed\n");
    let out = env.save(&["Change it"]);
    assert_eq!(out.status.code(), Some(1), "{}", text(&out.stdout));
    let err = text(&out.stderr);
    assert!(
        err.contains(
            "error: You're at a save, not on a line, so this save needs a new line to go on.\n\n\
             hint: Nothing was saved. Name it: bolt save --line <name> \"message\"\n"
        ),
        "{err}"
    );
}

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
    env.commit("a.txt", "a\n", "First");
    // A commit naming a file Git doesn't have: only the converter reads files, so only it finds
    // this one missing.
    let blob = "12".repeat(20);
    let entry = format!("100644,{blob},art/big.psd");
    env.git(&["update-index", "--add", "--cacheinfo", &entry]);
    let tree = env.git(&["write-tree", "--missing-ok"]);
    let commit = env.git(&["commit-tree", "-p", "HEAD", "-m", "Add a big file", &tree]);
    env.git(&["update-ref", "refs/heads/main", &commit]);
    let err = env.refused_import();
    assert!(
        err.contains(&format!(
            "missing object `{blob}`, in the file `art/big.psd` of commit `{commit}`"
        )),
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

/// A project that had saves and lost only its `format` is damage: neither command writes a new
/// first entry over its saves, which would leave them for clean-up.
#[test]
fn import_and_start_refuse_a_project_that_lost_only_its_format() {
    let env = Env::new();
    let refused_over_saves = |dir: &Path, command: &str| {
        std::fs::write(dir.join("new.txt"), "new\n").unwrap();
        let saved = env
            .bolt_in(dir)
            .args(["save", "Add a file"])
            .output()
            .unwrap();
        assert_eq!(saved.status.code(), Some(0), "{}", text(&saved.stderr));
        std::fs::remove_file(dir.join(".bolt/format")).unwrap();
        let head = std::fs::read(dir.join(".bolt/state/head")).unwrap();
        let out = env.bolt_in(dir).arg(command).output().unwrap();
        assert_eq!(
            out.status.code(),
            Some(1),
            "{command}: {}",
            text(&out.stdout)
        );
        let err = text(&out.stderr);
        assert!(
            err.contains("has saves, but no .bolt/format file.\n")
                && err.contains(
                    "hint: Nothing was changed. Ask for help before trying to repair it.\n"
                ),
            "{command}: {err}"
        );
        assert_eq!(
            std::fs::read(dir.join(".bolt/state/head")).unwrap(),
            head,
            "{command}"
        );
        assert!(!dir.join(".bolt/format").exists(), "{command}");
    };
    env.commit("a.txt", "a\n", "First");
    env.import();
    refused_over_saves(env.root(), "import");
    let started = tempfile::tempdir().unwrap();
    env.bolt_in(started.path())
        .args(["start", "--preset", "custom"])
        .assert()
        .success();
    refused_over_saves(started.path(), "start");
}

/// A sign of Git LFS has the first pass look for every Git LFS file's object, so the usual Git
/// LFS repository missing some stops before `.bolt` is made.
#[test]
fn a_git_lfs_repository_stops_before_anything_is_written() {
    let lfs_line = "*.psd filter=lfs diff=lfs merge=lfs -text\n";
    for sign in ["gitattributes", "lfs folder"] {
        let env = Env::new();
        env.commit("a.txt", "a\n", "First");
        if sign == "gitattributes" {
            env.commit(".gitattributes", lfs_line, "Track art with Git LFS");
        } else {
            std::fs::create_dir_all(env.root().join(".git/lfs/objects")).unwrap();
        }
        let commit = env.commit("art/big.psd", POINTER, "Add a big file");
        let err = env.refused_import();
        assert!(
            err.contains(&format!(
                "error: A Git LFS file in this history isn't in .git/lfs/objects:\n  \
                 `art/big.psd` in commit `{commit}` (12 KiB)\n"
            )),
            "{sign}: {err}"
        );
        assert!(err.contains("hint: Nothing was changed."), "{sign}: {err}");
        assert!(!env.root().join(".bolt").exists(), "{sign}");
    }
    // A sign can outlast the pointers, as `.git/lfs` does `git lfs migrate export`, and doesn't
    // stop the import by itself.
    let env = Env::new();
    env.commit(".gitattributes", lfs_line, "Track art with Git LFS");
    std::fs::create_dir_all(env.root().join(".git/lfs/objects")).unwrap();
    env.commit("art/big.psd", "the real file\n", "Add a big file");
    let (out, _) = env.import();
    assert!(out.contains("Imported 2 commits from Git"), "{out}");
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
    let out = env
        .bolt()
        .args(["start", "--preset", "custom"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1), "{}", text(&out.stdout));
    let err = text(&out.stderr);
    assert!(
        err.contains(
            "error: An import began a Bolt project here and didn't finish, and bolt start won't \
             make an empty project over what it stored.\n\nhint: Run `bolt import` again to finish \
             the import.\n"
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
    env.bolt_in(outer.path())
        .args(["start", "--preset", "custom"])
        .assert()
        .success();
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

/// A commit Jujutsu made shows in Bolt with the letters of its `change-id` header.
#[test]
fn an_imported_save_shows_the_letters_its_change_id_header_has() {
    let env = Env::new();
    let blob = env.git_input(&["hash-object", "-w", "--stdin"], b"a\n");
    let tree = env.git_input(
        &["mktree"],
        format!("100644 blob {blob}\ta.txt\n").as_bytes(),
    );
    let letters = "zyxwvutsrqponmlkzyxwvutsrqponmlk";
    let raw = format!(
        "tree {tree}\nauthor A <a@example.com> 0 +0000\ncommitter A <a@example.com> 0 +0000\n\
         change-id {letters}\n\nFirst\n"
    );
    let args = [
        "hash-object",
        "-t",
        "commit",
        "--literally",
        "-w",
        "--stdin",
    ];
    let commit = env.git_input(&args, raw.as_bytes());
    env.git(&["update-ref", "refs/heads/main", &commit]);
    env.git(&["reset", "-q", "--hard"]);
    env.import();

    let out = env.bolt().args(["--json", "history"]).output().unwrap();
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(json["saves"][0]["change_id"], letters);
    let shown = text(&env.bolt().arg("history").output().unwrap().stdout);
    assert!(shown.starts_with("zyxw"), "{shown}");
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

/// Bolt reads Git's personal exclude list (decision 047), so the first save keeps out what it
/// hides, and the import has nothing to say about it.
#[test]
fn the_first_save_keeps_out_what_gits_exclude_list_hides() {
    let env = Env::new();
    env.commit("a.txt", "a\n", "First");
    let info = env.root().join(".git/info");
    std::fs::create_dir_all(&info).unwrap();
    std::fs::write(info.join("exclude"), "secret.txt\n").unwrap();
    std::fs::write(env.root().join("secret.txt"), "API_KEY=x\n").unwrap();
    std::fs::write(env.root().join("notes.txt"), "mine\n").unwrap();
    let out = env.bolt().args(["--json", "import"]).output().unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(json["warnings"], serde_json::json!([]));
    let out = env.bolt().args(["save", "Work"]).output().unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    let shown = env.bolt().args(["--json", "show"]).output().unwrap();
    let json: serde_json::Value = serde_json::from_slice(&shown.stdout).unwrap();
    let paths: Vec<&str> = json["changes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["path"].as_str().unwrap())
        .collect();
    assert_eq!(paths, ["notes.txt"]);
}

#[test]
fn progress_is_not_printed_without_a_terminal() {
    let env = Env::new();
    for i in 0..3 {
        env.commit("a.txt", &format!("{i}\n"), &format!("Save {i}"));
    }
    let (_, err) = env.import();
    assert!(!err.contains("Importing commits"), "{err}");
    assert!(!err.contains("Reading Git's history"), "{err}");
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
    and save again. To save the files with CRLF anyway, run the same command again with \
    --accept-crlf.";

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
