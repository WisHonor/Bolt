//! The proofs of red-team session RT18 (milestone 7, lines and tags, second session): one test
//! per finding, each asserting what should be true.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use opal_core::id::{ChangeId, Id};
use opal_core::pack::ObjectKind;
use opal_core::save::Save;

mod support;
use support::{Env, text};

/// `opalro args`: its exit code, stdout and stderr.
fn run(env: &Env, args: &[&str]) -> (Option<i32>, String, String) {
    let out = env.opalro().args(args).output().unwrap();
    (out.status.code(), text(&out.stdout), text(&out.stderr))
}

/// A started project on `main`, with `a.txt` saved as "A".
fn project() -> Env {
    let env = Env::empty();
    env.opalro()
        .args(["start", "--preset", "custom"])
        .assert()
        .success();
    env.overwrite("a.txt", "a");
    env.opalro().args(["save", "A"]).assert().success();
    env
}

/// `project`, with line `name` one save ("F") past `main`, and the project back on `main`.
fn with_line(name: &str) -> Env {
    let env = project();
    env.opalro().args(["line", "new", name]).assert().success();
    env.overwrite("a.txt", "f");
    env.opalro().args(["save", "F"]).assert().success();
    env.opalro().args(["goto", "main"]).assert().success();
    env
}

#[test]
fn the_ids_a_server_line_or_tag_refusal_gives_reach_both_saves() {
    // origin/main is one commit ahead of main, and a tag named main@origin is at main.
    let env = Env::new();
    let one = env.commit("a.txt", "one", "one");
    let two = env.commit("a.txt", "two", "two");
    env.git(&["update-ref", "refs/remotes/origin/main", &two]);
    env.git(&["reset", "-q", "--hard", &one]);
    env.git(&["tag", "main@origin", &one]);
    env.import();
    let (repo, state) = env.state();
    let server = repo.server_lines(&state).unwrap().servers["origin"]["main"];
    drop(repo);
    let shown = &server.to_hex()[..12];
    let (code, _, said) = run(&env, &["goto", "main@origin"]);
    assert_eq!(code, Some(1), "{said}");
    assert!(said.contains(&format!("server line {shown}")), "{said}");
    assert!(said.contains("type its ID as shown here"), "{said}");
    // The refusal says to type the ID it shows.
    let (code, _, followed) = run(&env, &["goto", shown]);
    assert_eq!(
        code,
        Some(0),
        "the refusal said to type {shown}, and goto refused it: {followed}"
    );
}

#[test]
#[ignore = "RT18-2: line delete doesn't name a tag whose save's older saves can't be read"]
fn line_delete_names_a_tag_whose_history_cant_be_read() {
    let env = with_line("feat");
    // A tag at a save that is in the store, whose parent isn't.
    let (repo, state) = env.state();
    let tip = repo.store().save(&state.lines["main"], false).unwrap();
    drop(repo);
    let missing = Id::from_hex(&"5".repeat(64)).unwrap();
    let mut orphan = None;
    support::add_pack(env.root(), |pack| {
        let save = Save {
            parents: vec![missing],
            change_id: ChangeId::random().unwrap(),
            message: "orphan".into(),
            ..tip
        };
        orphan = Some(pack.add_record(ObjectKind::Save, &save.encode()).unwrap());
    });
    env.plant_tags(["ghost".to_owned()].into_iter(), orphan.unwrap());
    let before = env.standing();
    let (code, _, said) = run(&env, &["line", "delete", "feat", "--yes"]);
    assert_eq!(env.standing(), before);
    assert!(
        code == Some(0) || said.contains("ghost"),
        "line delete --yes stopped on the tag's unreadable history without naming the tag: \
         {said}"
    );
}

#[test]
#[ignore = "RT18-3: a quoted tag name still puts a line's mark on another save in history"]
fn a_quoted_tag_name_cannot_put_a_lines_mark_on_another_save() {
    let env = project();
    let (repo, state) = env.state();
    let first = repo
        .store()
        .save(&state.lines["main"], false)
        .unwrap()
        .parents[0];
    drop(repo);
    env.plant_tags(["x]  [main]  [tag y".to_owned()].into_iter(), first);
    let (code, shown, said) = run(&env, &["history"]);
    assert_eq!(code, Some(0), "{said}");
    let start = shown.lines().find(|l| l.contains("Start project")).unwrap();
    assert!(
        !start.contains("[main]"),
        "the first save's row reads as if line main were at it:\n{shown}"
    );
}

#[cfg(feature = "fault-injection")]
#[test]
fn the_command_shows_hint_gives_finds_the_save_shown() {
    let env = with_line("feat");
    env.opalro().args(["goto", "feat"]).assert().success();
    env.opalro().args(["tag", "rel"]).assert().success();
    env.opalro().args(["goto", "main"]).assert().success();
    let out = env
        .opalro()
        .args(["show", "rel"])
        .env("OPALRO_FAKE_TERMINAL", "80")
        .env("OPALRO_ASCII", "1")
        .env("NO_COLOR", "1")
        .output()
        .unwrap();
    let said = text(&out.stderr);
    let hinted = said
        .split("opalro ")
        .nth(1)
        .expect("show gave a next step")
        .trim()
        .to_owned();
    assert!(hinted.starts_with("changes --save "), "{said}");
    let words: Vec<&str> = hinted.split_whitespace().collect();
    let (code, _, followed) = run(&env, &words);
    assert_eq!(
        code,
        Some(0),
        "show suggested `opalro {hinted}`, which failed: {followed}"
    );
}

#[test]
fn gotos_advice_to_keep_saving_on_a_line_can_be_followed() {
    let env = with_line("v1");
    let (repo, state) = env.state();
    let line = state.lines["v1"];
    let main = state.lines["main"];
    drop(repo);
    // An imported tag v1 at another save, as Git allows.
    env.plant_tags(["v1".to_owned()].into_iter(), main);
    let (code, _, said) = run(&env, &["goto", "v1"]);
    assert_eq!(code, Some(1), "{said}");
    // The refusal's way to the line: its ID.
    let (code, shown, said) = run(&env, &["goto", &line.to_hex()[..12]]);
    assert_eq!(code, Some(0), "{said}");
    if shown.contains("opalro goto v1") {
        let (code, _, followed) = run(&env, &["goto", "v1"]);
        assert_eq!(
            code,
            Some(0),
            "goto said `opalro goto v1` keeps saving on v1, and that is refused:\n{shown}\n\
             {followed}"
        );
    }
}

#[test]
#[ignore = "RT18-6: unshelf's hint carries a stored shelf label inside a command, unquoted"]
fn the_hint_to_bring_back_a_shelf_carries_no_stored_label_in_a_command() {
    use opal_core::diary::{Action, DiaryEntry};
    use opal_core::repo::Repository;
    let env = project();
    env.overwrite("a.txt", "shelved");
    env.opalro().arg("shelf").assert().success();
    // A planted project whose one shelf has a label no Opalro makes.
    let label = "1; touch pwned";
    {
        let mut repo = Repository::open(env.root()).unwrap();
        let lock = repo.lock_for_change("shelf", &mut |_| {}).unwrap();
        let (head, entry) = repo.current().unwrap();
        let mut state = entry.state.clone();
        state.shelves[0].0 = label.to_owned();
        let diary = DiaryEntry {
            parent: Some(head),
            action: Action::Command,
            command: "shelf".into(),
            who: entry.who,
            opal_version: "0.1.0".into(),
            state,
            folder_change: None,
        };
        let packs = repo.new_packs().unwrap();
        repo.commit(&lock, packs, &diary).unwrap();
    }
    // An unsaved change where the shelf would write.
    env.overwrite("a.txt", "in the way");
    let (code, _, said) = run(&env, &["unshelf", label]);
    assert_eq!(code, Some(1), "{said}");
    // Pasted into a shell, `opalro unshelf 1; touch pwned` runs `touch pwned`.
    assert!(
        !said.contains(&format!("opalro unshelf {label}")),
        "the hint is a shell command with the stored label unquoted: {said}"
    );
}
