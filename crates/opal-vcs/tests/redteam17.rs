//! The proofs of red-team session RT17 (milestone 7, lines and tags): one test per finding,
//! each asserting what should be true.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use opal_core::id::Id;

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

/// The save `show <text> --json` shows, as hex.
fn shown(env: &Env, text: &str) -> (Option<i32>, String, serde_json::Value) {
    let (code, out, said) = run(env, &["show", text, "--json"]);
    let doc: serde_json::Value = serde_json::from_str(&out).unwrap_or_default();
    (code, said, doc)
}

#[test]
fn a_server_lines_name_means_the_same_save_in_goto_and_show() {
    // Git allows a tag named `main@origin`; the import keeps it as it is.
    let env = Env::new();
    let one = env.commit("a.txt", "one", "one");
    let two = env.commit("a.txt", "two", "two");
    env.git(&["update-ref", "refs/remotes/origin/main", &one]);
    env.git(&["tag", "main@origin", &two]);
    env.import();
    let (repo, state) = env.state();
    let tag = repo.tag_list(&state).unwrap().tags["main@origin"].save;
    drop(repo);
    let (code, said, doc) = shown(&env, "main@origin");
    assert_ne!(
        doc["save"]["id"],
        tag.to_hex(),
        "show main@origin showed the tag's save: {said}"
    );
    // The server line and the tag are at two saves, so both commands refuse the name alike.
    let (shown_code, _, shown_said) = run(&env, &["show", "main@origin"]);
    let (goto_code, _, goto_said) = run(&env, &["goto", "main@origin"]);
    assert_eq!((code, shown_code, goto_code), (Some(1), Some(1), Some(1)));
    assert!(shown_said.contains("server line"), "{shown_said}");
    assert_eq!(shown_said, goto_said);
}

#[test]
fn a_name_that_is_both_a_line_and_a_tag_is_not_taken_silently_for_the_line() {
    // A branch and a tag of one name, at two commits: Git warns and takes the tag.
    let env = Env::new();
    let first = env.commit("a.txt", "one", "one");
    env.git(&["tag", "v1", &first]);
    env.commit("a.txt", "two", "two");
    env.git(&["branch", "v1"]);
    env.import();
    let (repo, state) = env.state();
    let tag = repo.tag_list(&state).unwrap().tags["v1"].save;
    let line = state.lines["v1"];
    assert_ne!(tag, line);
    drop(repo);
    let (code, said, doc) = shown(&env, "v1");
    let warned = doc["warnings"].as_array().is_some_and(|w| !w.is_empty());
    let refused = code != Some(0);
    let took_the_tag = doc["save"]["id"] == tag.to_hex();
    assert!(
        refused || warned || took_the_tag,
        "show v1 went to line v1's save without a word, though tag v1 (which `opalro tag` lists) \
         is at another save: {doc} {said}"
    );
}

#[test]
fn a_tag_at_a_missing_save_is_named_when_it_stops_line_delete() {
    let env = project();
    env.opalro()
        .args(["line", "new", "feat"])
        .assert()
        .success();
    env.overwrite("a.txt", "f");
    env.opalro().args(["save", "F"]).assert().success();
    env.opalro().args(["goto", "main"]).assert().success();
    let missing = Id::from_hex(&"7".repeat(64)).unwrap();
    env.plant_tags(["ghost".to_owned()].into_iter(), missing);
    let (code, _, said) = run(&env, &["line", "delete", "feat", "--yes"]);
    assert!(
        code == Some(0) || said.contains("ghost"),
        "line delete --yes stopped on the tag's missing save without naming the tag: {said}"
    );
}

#[test]
#[ignore = "RT17-4: the hint to delete a tag prints its stored name unquoted"]
fn the_hint_to_delete_a_tag_quotes_its_name() {
    let env = project();
    let tip = env.state().1.lines["main"];
    let name = "v1; touch pwned";
    env.plant_tags([name.to_owned()].into_iter(), tip);
    let (code, _, said) = run(&env, &["tag", "--delete", name]);
    assert_eq!(code, Some(1), "{said}");
    // Pasted into a shell, the hint as printed deletes nothing and runs `touch pwned --yes`.
    assert!(
        !said.contains("opalro tag --delete v1; touch pwned --yes"),
        "the hint is a shell command with the name unquoted: {said}"
    );
}

#[test]
#[ignore = "RT17-5: a tag's name can put a line's mark on another save in history"]
fn a_tags_name_cannot_put_a_lines_mark_on_another_save() {
    let env = project();
    let (repo, state) = env.state();
    let first = repo
        .store()
        .save(&state.lines["main"], false)
        .unwrap()
        .parents[0];
    drop(repo);
    env.plant_tags(["v9]  [main".to_owned()].into_iter(), first);
    let (code, shown, said) = run(&env, &["history"]);
    assert_eq!(code, Some(0), "{said}");
    let start = shown.lines().find(|l| l.contains("Start project")).unwrap();
    assert!(
        !start.contains("[main]"),
        "the first save's row reads as if line main were at it:\n{shown}"
    );
}
