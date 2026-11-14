#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use opal_core::id::Id;

mod support;
use support::{Env, text};

fn run(env: &Env, args: &[&str]) -> (Option<i32>, String, String) {
    let out = env.opalro().args(args).output().unwrap();
    let r = (out.status.code(), text(&out.stdout), text(&out.stderr));
    println!("$ opalro {args:?}\n  code {:?}\n  out: {}\n  err: {}", r.0, r.1, r.2);
    r
}

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

#[test]
fn e1_branch_and_tag_alike_from_git() {
    let env = Env::new();
    let first = env.commit("a.txt", "one", "one");
    env.git(&["tag", "v1", &first]);
    env.commit("a.txt", "two", "two");
    env.git(&["branch", "v1"]);
    println!("git rev-parse v1: {}", env.git(&["rev-parse", "v1"]));
    env.import();
    let (repo, state) = env.state();
    let tags = repo.tag_list(&state).unwrap();
    println!("lines {:?}", state.lines);
    println!("tags {:?}", tags.tags.keys().collect::<Vec<_>>());
    drop(repo);
    run(&env, &["show", "v1", "--json"]);
    run(&env, &["tag"]);
}

#[test]
fn e2_tag_at_missing_save_blocks_line_delete() {
    let env = project();
    run(&env, &["line", "new", "feat"]);
    run(&env, &["goto", "main"]);
    env.plant_tags(["ghost".to_owned()].into_iter(), Id::from_hex(&"7".repeat(64)).unwrap());
    run(&env, &["line", "delete", "feat", "--yes"]);
    run(&env, &["line", "delete", "feat"]);
    run(&env, &["history"]);
    run(&env, &["goto", "feat"]);
    run(&env, &["tag", "--delete", "ghost", "--yes"]);
}

#[test]
fn e3_hint_with_shell_text() {
    let env = project();
    let tip = env.state().1.lines["main"];
    env.plant_tags(["v1; touch pwned".to_owned()].into_iter(), tip);
    run(&env, &["tag", "--delete", "v1; touch pwned"]);
    run(&env, &["tag"]);
}

#[test]
fn e4_ghost_tag_with_own_save() {
    let env = project();
    run(&env, &["line", "new", "feat"]);
    env.overwrite("a.txt", "f");
    run(&env, &["save", "F"]);
    run(&env, &["goto", "main"]);
    env.plant_tags(["ghost".to_owned()].into_iter(), Id::from_hex(&"7".repeat(64)).unwrap());
    run(&env, &["line", "delete", "feat", "--yes"]);
    run(&env, &["line", "delete", "feat", "--json"]);
}

#[test]
fn e5_tag_at_a_tree() {
    let env = project();
    run(&env, &["line", "new", "feat"]);
    env.overwrite("a.txt", "f");
    run(&env, &["save", "F"]);
    run(&env, &["goto", "main"]);
    let (repo, state) = env.state();
    let root = repo.store().save(&state.lines["main"], false).unwrap().root;
    drop(repo);
    env.plant_tags(["tree".to_owned()].into_iter(), root);
    run(&env, &["line", "delete", "feat", "--yes"]);
    run(&env, &["history"]);
    run(&env, &["goto", "tree"]);
    run(&env, &["show", "tree"]);
}

#[test]
fn e6_server_line_and_tag_alike() {
    let env = Env::new();
    let one = env.commit("a.txt", "one", "one");
    let two = env.commit("a.txt", "two", "two");
    env.git(&["update-ref", "refs/remotes/origin/main", &one]);
    env.git(&["tag", "main@origin", &two]);
    env.import();
    let (repo, state) = env.state();
    println!("servers {:?}", repo.server_lines(&state).unwrap());
    drop(repo);
    run(&env, &["show", "main@origin", "--json"]);
    run(&env, &["changes", "--save", "main@origin"]);
    run(&env, &["goto", "main@origin"]);
    run(&env, &["history"]);
}

#[test]
fn e7_branch_and_tag_alike_goto() {
    let env = Env::new();
    let first = env.commit("a.txt", "one", "one");
    env.git(&["tag", "v1", &first]);
    env.commit("a.txt", "two", "two");
    env.git(&["branch", "v1"]);
    let out = git_out(&env, &["rev-parse", "v1"]);
    println!("git rev-parse v1: {out}");
    env.import();
    run(&env, &["goto", "v1"]);
    run(&env, &["tag", "copy", "--save", "v1"]);
    run(&env, &["tag"]);
}

fn git_out(env: &Env, args: &[&str]) -> String {
    let o = support::git_with_input(env.root(), args, b"");
    format!("{} / {}", text(&o.stdout), text(&o.stderr))
}

#[test]
fn e8_show_server_line_without_tag() {
    let env = Env::new();
    let one = env.commit("a.txt", "one", "one");
    env.commit("a.txt", "two", "two");
    env.git(&["update-ref", "refs/remotes/origin/main", &one]);
    env.import();
    run(&env, &["show", "main@origin"]);
    run(&env, &["tag", "x", "--save", "main@origin"]);
    run(&env, &["history"]);
}

#[test]
fn e9_forged_marks() {
    let env = project();
    let (repo, state) = env.state();
    let first = repo.store().save(&state.lines["main"], false).unwrap().parents[0];
    drop(repo);
    env.plant_tags(["v9]  [main".to_owned()].into_iter(), first);
    run(&env, &["history"]);
    run(&env, &["show", "v9]  [main"]);
}

#[test]
fn e10_amend_a_tagged_save() {
    let env = project();
    run(&env, &["tag", "v1"]);
    env.overwrite("a.txt", "changed");
    run(&env, &["save", "--amend", "A2"]);
    run(&env, &["tag"]);
    run(&env, &["history"]);
}
