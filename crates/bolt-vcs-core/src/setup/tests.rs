#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use super::*;
use crate::diary::DiaryEntry;
use crate::id::hash_content;
use crate::kept::EXACT;
use crate::presets::{boltignore_block, edit_boltignore};
use crate::snapshot::{SaveOutcome, SaveRequest, save_folder};
use crate::test_support::symlink;
use crate::undo::{StepRequest, redo, undo};
use crate::writeout::Tree;
use std::collections::{BTreeMap, HashMap};
use std::fs;

fn who() -> Person {
    Person {
        name: "Ada".into(),
        email: "ada@example.com".into(),
        time_ms: 1_700_000_000_000,
        tz_offset_min: 0,
    }
}

fn save_now(repo: &mut Repository) -> SaveOutcome {
    let who = who();
    let req = SaveRequest {
        message: Some("next"),
        who: &who,
        command: "save",
        accept_crlf: false,
        git_excludes: None,
        new_line: None,
        only: None,
        amend: false,
    };
    save_folder(repo, &req, &mut |_| {}, &mut |_| true).unwrap()
}

/// A project whose folder holds `files`, saved on `main`.
fn saved(files: &[(&str, &[u8])]) -> (tempfile::TempDir, Repository) {
    let dir = tempfile::tempdir().unwrap();
    let mut repo = Repository::init(dir.path(), &who()).unwrap();
    for (path, bytes) in files {
        fs::write(dir.path().join(path), bytes).unwrap();
    }
    assert!(matches!(save_now(&mut repo), SaveOutcome::Saved { .. }));
    (dir, repo)
}

/// Every file at the top of `dir` but `.bolt`, with what it holds.
fn files(dir: &Path) -> BTreeMap<String, Vec<u8>> {
    fs::read_dir(dir)
        .unwrap()
        .map(Result::unwrap)
        .filter(|e| e.file_name() != ".bolt")
        .map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            (name, fs::read(e.path()).unwrap())
        })
        .collect()
}

fn holding(entries: &[(&str, &[u8])]) -> BTreeMap<String, Vec<u8>> {
    entries
        .iter()
        .map(|(p, b)| ((*p).to_owned(), b.to_vec()))
        .collect()
}

fn code_toml() -> Vec<u8> {
    presets::bolt_toml(Preset::Code, &Found::default())
        .unwrap()
        .into_bytes()
}

fn game_toml() -> Vec<u8> {
    presets::bolt_toml(Preset::Game, &Found::default())
        .unwrap()
        .into_bytes()
}

fn code_block() -> String {
    boltignore_block(Preset::Code, &Found::default()).unwrap()
}

fn apply(repo: &mut Repository, plan: &SetupPlan) -> Result<SetupOutcome, SetupError> {
    apply_setup(repo, plan, &who(), None, &mut |_| {})
}

fn set_up(dir: &Path, repo: &mut Repository, preset: Preset) -> SetupOutcome {
    let plan = plan_setup(dir, preset).unwrap();
    apply(repo, &plan).unwrap()
}

fn step(who: &Person) -> StepRequest<'_> {
    StepRequest {
        n: 1,
        shelve: false,
        who,
        git_excludes: None,
    }
}

fn head_entry(repo: &Repository) -> (Id, DiaryEntry) {
    repo.current().unwrap()
}

/// What the save `save` holds at `path`, as an ID.
fn held(repo: &Repository, save: Id, path: &str) -> Option<Id> {
    let root = repo.store().save(&save, false).unwrap().root;
    let unstored = HashMap::new();
    let mut tree = Tree::new(repo.store(), &unstored, root, EXACT);
    tree.entry(path).unwrap().map(|e| e.id)
}

#[test]
fn a_plan_lists_only_the_files_that_would_change() {
    let (dir, _repo) = saved(&[("a.txt", b"a")]);
    fs::write(dir.path().join(".boltignore"), code_block()).unwrap();
    let plan = plan_setup(dir.path(), Preset::Code).unwrap();
    assert_eq!(plan.preset, Preset::Code);
    assert_eq!(
        plan.changes,
        [RuleFileChange {
            path: "bolt.toml",
            before: None,
            after: Some(code_toml()),
        }]
    );
    // Nothing was written.
    assert!(!dir.path().join("bolt.toml").exists());
}

#[test]
fn a_plan_for_the_same_preset_twice_is_empty() {
    let (dir, mut repo) = saved(&[("a.txt", b"a")]);
    set_up(dir.path(), &mut repo, Preset::Code);
    let (head, _) = head_entry(&repo);
    let plan = plan_setup(dir.path(), Preset::Code).unwrap();
    assert!(plan.changes.is_empty(), "{plan:?}");
    // Applying an empty plan writes nothing and records nothing.
    let done = apply(&mut repo, &plan).unwrap();
    assert!(done.written.is_empty());
    assert_eq!(head_entry(&repo).0, head);
}

#[test]
fn applying_writes_both_files_and_records_one_setup_entry() {
    let (dir, mut repo) = saved(&[("a.txt", b"a")]);
    let (before, _) = head_entry(&repo);
    let done = set_up(dir.path(), &mut repo, Preset::Code);
    assert_eq!(done.written, ["bolt.toml", ".boltignore"]);
    let block = code_block();
    assert_eq!(
        files(dir.path()),
        holding(&[
            ("a.txt", b"a"),
            ("bolt.toml", &code_toml()),
            (".boltignore", block.as_bytes()),
        ])
    );
    let (_, entry) = head_entry(&repo);
    assert_eq!(entry.command, "setup");
    assert_eq!(entry.action, Action::Command);
    assert_eq!(entry.parent, Some(before));
    assert!(entry.folder_change.is_some());
}

#[test]
fn undo_removes_a_file_the_step_made_and_redo_writes_it_again() {
    let (dir, mut repo) = saved(&[("a.txt", b"a")]);
    set_up(dir.path(), &mut repo, Preset::Code);
    let after = files(dir.path());
    let who = who();
    // `undo --list` offers it as a step, under its own name.
    let list = crate::undo::undo_list(&repo, 10).unwrap();
    assert_eq!(list.undo[0].diary.command, "setup");
    undo(&mut repo, &step(&who), &mut |_| {}).unwrap();
    assert_eq!(files(dir.path()), holding(&[("a.txt", b"a")]));
    let list = crate::undo::undo_list(&repo, 10).unwrap();
    assert_eq!(list.redo[0].diary.command, "setup");
    redo(&mut repo, &step(&who), &mut |_| {}).unwrap();
    assert_eq!(files(dir.path()), after);
}

#[test]
fn undo_puts_back_edits_that_were_never_saved() {
    let (dir, mut repo) = saved(&[("a.txt", b"a"), ("bolt.toml", b"preset = \"code\"\n")]);
    fs::write(dir.path().join("bolt.toml"), b"# mine\n").unwrap();
    fs::write(dir.path().join(".boltignore"), b"own/\n").unwrap();
    let before = files(dir.path());
    set_up(dir.path(), &mut repo, Preset::Game);
    assert_eq!(fs::read(dir.path().join("bolt.toml")).unwrap(), game_toml());
    undo(&mut repo, &step(&who()), &mut |_| {}).unwrap();
    assert_eq!(files(dir.path()), before);
}

#[test]
fn a_boltignore_of_the_users_own_keeps_every_line() {
    let own = b"secret/\n*.log\n";
    let (dir, mut repo) = saved(&[("a.txt", b"a"), (".boltignore", own)]);
    set_up(dir.path(), &mut repo, Preset::Code);
    let now = fs::read(dir.path().join(".boltignore")).unwrap();
    assert_eq!(
        now,
        edit_boltignore(Some(own), Some(&code_block())).unwrap()
    );
    assert!(now.ends_with(own), "{}", String::from_utf8_lossy(&now));
}

#[test]
fn custom_removes_the_rules_file_and_the_block() {
    let own = b"secret/\n";
    let (dir, mut repo) = saved(&[("a.txt", b"a"), (".boltignore", own)]);
    set_up(dir.path(), &mut repo, Preset::Code);
    let with_block = fs::read(dir.path().join(".boltignore")).unwrap();
    set_up(dir.path(), &mut repo, Preset::Custom);
    // Only the block's lines go: the empty line that parted it from the user's stays.
    let left = edit_boltignore(Some(&with_block), None).unwrap();
    assert_eq!(left, b"\nsecret/\n");
    assert_eq!(
        files(dir.path()),
        holding(&[("a.txt", b"a"), (".boltignore", &left)])
    );
    // With no lines of the user's own, the file goes too.
    let (dir, mut repo) = saved(&[("a.txt", b"a")]);
    set_up(dir.path(), &mut repo, Preset::Game);
    set_up(dir.path(), &mut repo, Preset::Custom);
    assert_eq!(files(dir.path()), holding(&[("a.txt", b"a")]));
}

#[test]
fn other_unsaved_work_is_left_alone() {
    let (dir, mut repo) = saved(&[("a.txt", b"a"), ("b.txt", b"b")]);
    fs::write(dir.path().join("a.txt"), b"edited").unwrap();
    fs::write(dir.path().join("new.txt"), b"new").unwrap();
    fs::remove_file(dir.path().join("b.txt")).unwrap();
    let before = files(dir.path());
    set_up(dir.path(), &mut repo, Preset::Code);
    let mut wanted = before.clone();
    wanted.insert("bolt.toml".into(), code_toml());
    wanted.insert(".boltignore".into(), code_block().into_bytes());
    assert_eq!(files(dir.path()), wanted);
    undo(&mut repo, &step(&who()), &mut |_| {}).unwrap();
    assert_eq!(files(dir.path()), before);
}

#[test]
fn the_state_does_not_change() {
    let (dir, mut repo) = saved(&[("a.txt", b"a")]);
    let (_, before) = head_entry(&repo);
    set_up(dir.path(), &mut repo, Preset::Code);
    let (_, after) = head_entry(&repo);
    assert_eq!(after.state, before.state);
    let status = crate::status::status(&repo, false, None).unwrap();
    let paths: Vec<&str> = status.changes.iter().map(|c| c.path.as_str()).collect();
    assert_eq!(paths, [".boltignore", "bolt.toml"]);
}

#[test]
fn a_link_or_a_folder_named_like_a_rule_file_refuses() {
    let (dir, _repo) = saved(&[("a.txt", b"a")]);
    fs::create_dir(dir.path().join("bolt.toml")).unwrap();
    let e = plan_setup(dir.path(), Preset::Code).unwrap_err();
    assert!(
        matches!(e, SetupError::NotAFile { path: "bolt.toml" }),
        "{e:?}"
    );
    fs::remove_dir(dir.path().join("bolt.toml")).unwrap();

    // Too large: refused, never read whole.
    let big = vec![b'#'; (1 << 20) + 1];
    fs::write(dir.path().join(".boltignore"), &big).unwrap();
    let e = plan_setup(dir.path(), Preset::Code).unwrap_err();
    assert!(
        matches!(
            e,
            SetupError::TooLarge {
                path: ".boltignore"
            }
        ),
        "{e:?}"
    );
    fs::remove_file(dir.path().join(".boltignore")).unwrap();

    let outside = tempfile::tempdir().unwrap();
    let target = outside.path().join("rules");
    fs::write(&target, b"outside").unwrap();
    if !symlink(&target, &dir.path().join(".boltignore")) {
        return;
    }
    let e = plan_setup(dir.path(), Preset::Code).unwrap_err();
    assert!(
        matches!(
            e,
            SetupError::NotAFile {
                path: ".boltignore"
            }
        ),
        "{e:?}"
    );
    assert_eq!(fs::read(&target).unwrap(), b"outside");
    // A link to nothing is a link all the same.
    fs::remove_file(&target).unwrap();
    let e = plan_setup(dir.path(), Preset::Code).unwrap_err();
    assert!(
        matches!(
            e,
            SetupError::NotAFile {
                path: ".boltignore"
            }
        ),
        "{e:?}"
    );
}

#[test]
fn a_file_edited_after_the_plan_refuses() {
    let (dir, mut repo) = saved(&[("a.txt", b"a")]);
    let plan = plan_setup(dir.path(), Preset::Code).unwrap();
    fs::write(dir.path().join("bolt.toml"), b"# typed meanwhile\n").unwrap();
    let (head, _) = head_entry(&repo);
    let e = apply(&mut repo, &plan).unwrap_err();
    assert!(
        matches!(e, SetupError::ChangedSincePlan { path: "bolt.toml" }),
        "{e:?}"
    );
    assert!(e.hint().is_some());
    assert_eq!(head_entry(&repo).0, head);
    assert_eq!(
        files(dir.path()),
        holding(&[("a.txt", b"a"), ("bolt.toml", b"# typed meanwhile\n")])
    );
}

#[test]
fn a_name_in_another_letter_case_is_the_rule_file_where_the_drive_ignores_case() {
    let (dir, mut repo) = saved(&[("a.txt", b"a")]);
    if !repo.drive().unwrap().ignores_case {
        return;
    }
    fs::write(dir.path().join("Bolt.toml"), b"# mine\n").unwrap();
    let plan = plan_setup(dir.path(), Preset::Code).unwrap();
    let change = plan.changes.iter().find(|c| c.path == "bolt.toml").unwrap();
    assert_eq!(change.before.as_deref(), Some(&b"# mine\n"[..]));
    // Write-out won't write over a name spelled otherwise, so the step is refused, losing nothing.
    let (head, _) = head_entry(&repo);
    let e = apply(&mut repo, &plan).unwrap_err();
    assert!(
        matches!(&e, SetupError::InTheWay { paths } if paths == &["bolt.toml"]),
        "{e:?}"
    );
    assert_eq!(head_entry(&repo).0, head);
    assert_eq!(
        files(dir.path()),
        holding(&[("a.txt", b"a"), ("Bolt.toml", b"# mine\n")])
    );
}

#[test]
fn the_entrys_two_saves_hold_the_two_files_before_and_after() {
    let (dir, mut repo) = saved(&[("a.txt", b"a")]);
    let current = tip_of(&head_entry(&repo).1).unwrap();
    fs::write(dir.path().join("bolt.toml"), b"# mine\n").unwrap();
    set_up(dir.path(), &mut repo, Preset::Code);
    let change = head_entry(&repo).1.folder_change.unwrap();
    let id = |b: &[u8]| Some(hash_content(b));
    assert_eq!(held(&repo, change.from, "bolt.toml"), id(b"# mine\n"));
    assert_eq!(held(&repo, change.from, ".boltignore"), None);
    assert_eq!(held(&repo, change.to, "bolt.toml"), id(&code_toml()));
    assert_eq!(
        held(&repo, change.to, ".boltignore"),
        id(code_block().as_bytes())
    );
    for side in [change.from, change.to] {
        assert_eq!(held(&repo, side, "a.txt"), id(b"a"));
        let save = repo.store().save(&side, false).unwrap();
        assert_eq!(save.parents, [current]);
        assert!(save.message.starts_with("setup"), "{}", save.message);
    }
    // Where the folder holds what the save does, "from" is that save.
    let (dir, mut repo) = saved(&[("a.txt", b"a")]);
    let current = tip_of(&head_entry(&repo).1).unwrap();
    set_up(dir.path(), &mut repo, Preset::Code);
    let change = head_entry(&repo).1.folder_change.unwrap();
    assert_eq!(change.from, current);
}

#[cfg(feature = "fault-injection")]
#[test]
fn a_setup_stopped_at_any_write_is_finished_by_the_next_command() {
    use crate::fsio::fault;
    use crate::test_support::after_head;
    let start: &[(&str, &[u8])] = &[("a.txt", b"a"), ("bolt.toml", b"# mine\n")];
    let block = code_block();
    let done: &[(&str, &[u8])] = &[
        ("a.txt", b"a"),
        ("bolt.toml", &code_toml()),
        (".boltignore", block.as_bytes()),
    ];
    let mut finished_later = 0;
    for n in 1.. {
        assert!(n < 200, "the setup never finished");
        let (dir, mut repo) = saved(&[("a.txt", b"a")]);
        fs::write(dir.path().join("bolt.toml"), b"# mine\n").unwrap();
        let plan = plan_setup(dir.path(), Preset::Code).unwrap();
        fault::arm(n);
        let result = apply(&mut repo, &plan);
        fault::disarm();
        let went_on = after_head(&repo);
        drop(repo);
        // The next changing command finishes what was started: the folder holds the two files
        // as they were or as the step wrote them, whole, and the step is recorded only if written.
        let mut repo = Repository::open(dir.path()).unwrap();
        let lock = repo.lock_for_change("test", &mut |_| {}).unwrap();
        crate::writeout::finish_interrupted(&mut repo, &lock).unwrap();
        drop(lock);
        let recorded = head_entry(&repo).1.command == "setup";
        let wanted = if recorded { done } else { start };
        assert_eq!(files(dir.path()), holding(wanted), "fault {n}");
        assert_eq!(
            crate::journal::Journal::read(&repo).unwrap(),
            None,
            "fault {n}"
        );
        if result.is_err() && recorded {
            finished_later += 1;
        }
        if result.is_ok() && !went_on {
            assert!(recorded);
            assert!(finished_later >= 2, "only {finished_later} finished later");
            break;
        }
    }
}
