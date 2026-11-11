//! `bolt save --amend`, end to end.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;

use bolt_core::id::Id;
use bolt_core::repo::Repository;
use bolt_core::save::{GIT_FIELDS, Save};
use serde_json::{Value, json};

mod support;
use support::{Env, changed, files, head, holding, main_tip, saved_files, text, write};

/// A project with `entries` saved on `main` as "Files".
fn project(entries: &[(&str, &str)]) -> Env {
    let env = Env::empty();
    env.bolt()
        .args(["start", "--preset", "custom"])
        .assert()
        .success();
    for (path, bytes) in entries {
        write(&env, path, bytes);
    }
    ok(&env, &["save", "Files"]);
    env
}

/// `bolt <args>`, which must succeed; what it printed.
fn ok(env: &Env, args: &[&str]) -> String {
    let out = env.bolt().args(args).output().unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "{args:?}: {}",
        text(&out.stderr)
    );
    text(&out.stdout)
}

/// `bolt <args>`, which must fail and change nothing; what it said on stderr.
fn refused(env: &Env, args: &[&str]) -> String {
    let before = env.standing();
    let out = env.bolt().args(args).output().unwrap();
    assert_eq!(
        out.status.code(),
        Some(1),
        "{args:?}: {}",
        text(&out.stdout)
    );
    assert_eq!(env.standing(), before, "{args:?}");
    text(&out.stderr)
}

fn json(env: &Env, args: &[&str]) -> Value {
    let out = env.bolt().args(args).arg("--json").output().unwrap();
    serde_json::from_slice(&out.stdout).unwrap()
}

fn save_of(env: &Env, id: Id) -> Save {
    let repo = Repository::open(env.root()).unwrap();
    repo.store().save(&id, false).unwrap()
}

#[test]
fn amend_replaces_the_newest_save_keeping_change_id_parents_and_author() {
    let env = project(&[("a.txt", "a\n")]);
    let old_id = main_tip(&env);
    let old = save_of(&env, old_id);
    write(&env, "a.txt", "A\n");
    write(&env, "b.txt", "b\n");
    let out = env
        .bolt()
        .args(["save", "--amend", "Files, and b"])
        .env("BOLT_USER_NAME", "Grace")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    let new_id = main_tip(&env);
    assert_ne!(new_id, old_id);
    let new = save_of(&env, new_id);
    assert_eq!(new.change_id, old.change_id);
    assert_eq!(new.parents, old.parents);
    assert_eq!(new.author, old.author);
    assert_eq!(new.saver.name, "Grace");
    assert_eq!(new.message, "Files, and b");
    assert_eq!(new.line(), Some("main"));
    let both = holding(&[("a.txt", "A\n"), ("b.txt", "b\n")]);
    assert_eq!(saved_files(&env, new_id), both);
    // Its change ID, and every file it changes since the save before it.
    let short = json(&env, &["show"])["save"]["change_id_short"]
        .as_str()
        .unwrap()
        .to_owned();
    let said = text(&out.stdout);
    assert_eq!(
        said,
        format!(
            "Updated save {short} \"Files, and b\" on main  (2 files)\n\n    The ID stays the same.\n"
        )
    );
    // The line holds the new save in the old one's place, with nothing left unsaved.
    assert!(changed(&env).is_empty());
    let saves: Vec<Value> = json(&env, &["history"])["saves"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["id"].clone())
        .collect();
    assert_eq!(
        saves,
        [json!(new_id.to_hex()), json!(old.parents[0].to_hex())]
    );
    // A step of its own, recorded as a save, that changed no file; the old save stays stored.
    let (_, entry) = head(&env);
    assert_eq!(entry.command, "save");
    assert_eq!(entry.folder_change, None);
    assert_eq!(save_of(&env, old_id), old);
}

#[test]
fn amend_with_only_a_message_rewords() {
    let env = project(&[("a.txt", "a\n")]);
    let old = save_of(&env, main_tip(&env));
    let said = ok(&env, &["save", "--amend", "Better words"]);
    assert!(said.starts_with("Updated save "), "{said}");
    let new = save_of(&env, main_tip(&env));
    assert_eq!(new.root, old.root);
    assert_eq!(new.message, "Better words");
    assert_eq!(new.change_id, old.change_id);
    assert!(changed(&env).is_empty());
}

#[test]
fn amend_with_paths_takes_only_those() {
    let env = project(&[("a.txt", "a\n"), ("b.txt", "b\n")]);
    write(&env, "a.txt", "A\n");
    write(&env, "b.txt", "B\n");
    ok(&env, &["save", "--amend", "Only a", "a.txt"]);
    let tip = main_tip(&env);
    let only_a = holding(&[("a.txt", "A\n"), ("b.txt", "b\n")]);
    assert_eq!(saved_files(&env, tip), only_a);
    assert_eq!(save_of(&env, tip).message, "Only a");
    assert_eq!(changed(&env), ["b.txt"]);
    // An empty message before the paths keeps the save's own.
    ok(&env, &["save", "--amend", "", "b.txt"]);
    let tip = main_tip(&env);
    let both = holding(&[("a.txt", "A\n"), ("b.txt", "B\n")]);
    assert_eq!(saved_files(&env, tip), both);
    assert_eq!(save_of(&env, tip).message, "Only a");
    assert!(changed(&env).is_empty());
}

#[test]
fn amend_with_nothing_new_says_so() {
    let env = project(&[("a.txt", "a\n")]);
    let before = env.standing();
    let said = ok(&env, &["save", "--amend"]);
    assert!(
        said.starts_with("Nothing to amend: your files match your last save, ")
            && said.ends_with(" \"Files\", and so does the message.\n"),
        "{said}"
    );
    let said = ok(&env, &["save", "--amend", "Files"]);
    assert!(said.starts_with("Nothing to amend: "), "{said}");
    let doc = json(&env, &["save", "--amend"]);
    let nothing = json!({"bolt_json": 1, "result": "nothing_changed", "paths": [], "warnings": []});
    assert_eq!(doc, nothing);
    // With paths, only what they name counts.
    write(&env, "b.txt", "b\n");
    let said = ok(&env, &["save", "--amend", "", "a.txt"]);
    assert!(
        said.starts_with("Nothing to amend: what you named matches your last save, "),
        "{said}"
    );
    assert_eq!(env.standing(), before);
}

#[test]
fn amend_at_a_save_is_refused() {
    let env = project(&[("a.txt", "a\n")]);
    let first = main_tip(&env);
    write(&env, "a.txt", "b\n");
    ok(&env, &["save", "B"]);
    ok(&env, &["goto", &first.to_hex()]);
    write(&env, "a.txt", "c\n");
    let err = refused(&env, &["save", "--amend", "C"]);
    assert!(err.contains("You're at a save, not on a line"), "{err}");
    assert!(err.contains("bolt goto <line>"), "{err}");
    assert_eq!(fs::read(env.root().join("a.txt")).unwrap(), b"c\n");
    // An amend never makes a line, so --line is a mistake on the command line.
    let out = env.save(&["--amend", "--line", "x", "C"]);
    assert_eq!(out.status.code(), Some(2), "{}", text(&out.stderr));
}

#[test]
fn amend_of_a_save_another_line_holds_is_refused() {
    let env = project(&[("a.txt", "a\n")]);
    let held = save_of(&env, main_tip(&env)).change_id.to_letters();
    ok(&env, &["goto", &held]);
    write(&env, "b.txt", "b\n");
    ok(&env, &["save", "--line", "b", "B"]);
    ok(&env, &["goto", "main"]);
    write(&env, "a.txt", "A\n");
    let err = refused(&env, &["save", "--amend", "A"]);
    assert!(err.contains("line b shows it too"), "{err}");
    assert!(
        err.contains("Save your changes as a new save instead"),
        "{err}"
    );
    assert_eq!(fs::read(env.root().join("a.txt")).unwrap(), b"A\n");
    // A save made on top is on no other line, so it can be amended.
    ok(&env, &["save", "A"]);
    write(&env, "a.txt", "AA\n");
    ok(&env, &["save", "--amend"]);
    assert!(changed(&env).is_empty());
}

#[test]
fn amend_of_a_save_a_server_line_or_tag_names_is_refused() {
    let env = Env::new();
    env.commit("a.txt", "a\n", "First");
    env.git(&["update-ref", "refs/remotes/origin/main", "HEAD"]);
    env.git(&["tag", "v1"]);
    env.git(&["branch", "develop"]);
    env.import();
    write(&env, "a.txt", "A\n");
    let err = refused(&env, &["save", "--amend", "A"]);
    let named = "line develop, server line main@origin and tag v1 show it too";
    assert!(err.contains(named), "{err}");
    ok(&env, &["save", "A"]);
    write(&env, "a.txt", "AA\n");
    ok(&env, &["save", "--amend"]);
    assert!(changed(&env).is_empty());
}

#[test]
fn amend_stops_at_the_line_ending_check_after_an_import() {
    let env = Env::new();
    env.commit("a.txt", "one\ntwo\n", "First");
    env.import();
    // As Git for Windows writes it out.
    env.overwrite("a.txt", "one\r\ntwo\r\n");
    let err = refused(&env, &["save", "--amend"]);
    assert!(err.contains("only in line endings"), "{err}");
    let again = "run the same command again with --accept-crlf.";
    assert!(err.contains(again), "{err}");
    assert_eq!(changed(&env), ["a.txt"]);
    let old = save_of(&env, main_tip(&env));
    ok(&env, &["save", "--amend", "--accept-crlf"]);
    let tip = main_tip(&env);
    assert_eq!(save_of(&env, tip).change_id, old.change_id);
    let crlf = holding(&[("a.txt", "one\r\ntwo\r\n")]);
    assert_eq!(saved_files(&env, tip), crlf);
}

#[test]
fn amend_drops_git_fields() {
    let env = Env::new();
    // A commit in Latin-1, as it says: the save keeps its Git header and message bytes too.
    let blob = env.git_input(&["hash-object", "-w", "--stdin"], b"a\n");
    let tree = env.git_input(
        &["mktree"],
        format!("100644 blob {blob}\ta.txt\n").as_bytes(),
    );
    let header = format!(
        "tree {tree}\nauthor Ida Lark <ida.lark@example.com> 1601234567 -0400\n\
         committer Omar Fenn <omar.fenn@example.com> 1601234627 +0545\n\
         encoding ISO-8859-1\n\n"
    );
    let raw = [header.as_bytes(), b"Caf\xe9\n"].concat();
    let args = [
        "hash-object",
        "-t",
        "commit",
        "--literally",
        "-w",
        "--stdin",
    ];
    let commit = env.git_input(&args, &raw);
    env.git(&["update-ref", "refs/heads/main", &commit]);
    env.git(&["reset", "-q", "--hard"]);
    env.import();
    let old = save_of(&env, main_tip(&env));
    for field in GIT_FIELDS {
        assert!(old.extra.contains_key(field), "{field}: {:?}", old.extra);
    }
    write(&env, "a.txt", "A\n");
    ok(&env, &["save", "--amend"]);
    let new = save_of(&env, main_tip(&env));
    let mut kept = old.extra.clone();
    kept.retain(|k, _| !GIT_FIELDS.contains(&k.as_str()));
    assert_eq!(new.extra, kept);
    assert_eq!(new.message, "Caf\u{e9}\n");
    assert_eq!(new.author, old.author);
    assert_eq!(new.change_id, old.change_id);
    assert!(new.parents.is_empty());
    assert!(changed(&env).is_empty());
}

#[test]
fn undo_of_an_amend_puts_the_old_save_back_and_keeps_the_files() {
    let env = project(&[("a.txt", "a\n")]);
    let old_id = main_tip(&env);
    write(&env, "a.txt", "A\n");
    ok(&env, &["save", "--amend", "Changed a"]);
    let new_id = main_tip(&env);
    let edited = files(env.root());
    let said = ok(&env, &["undo"]);
    assert!(said.contains("its changes show as unsaved"), "{said}");
    assert_eq!(main_tip(&env), old_id);
    assert_eq!(files(env.root()), edited);
    assert_eq!(changed(&env), ["a.txt"]);
    ok(&env, &["redo"]);
    assert_eq!(main_tip(&env), new_id);
    assert_eq!(files(env.root()), edited);
    assert!(changed(&env).is_empty());
}

#[test]
fn an_undone_amend_names_the_amended_save_by_its_fingerprint() {
    let env = project(&[("a.txt", "a\n")]);
    write(&env, "a.txt", "A\n");
    ok(&env, &["save", "--amend", "Changed a"]);
    let amended = main_tip(&env);
    let change_id = save_of(&env, amended).change_id.to_letters();
    // Once the amend is undone, the save on the line has the amended save's change ID too.
    let said = ok(&env, &["undo"]);
    let fingerprint = &amended.to_hex()[..12];
    assert!(
        said.starts_with(&format!("Undid save {fingerprint} \"Changed a\": ")),
        "{said}"
    );
    let listed = ok(&env, &["undo", "--list"]);
    assert!(
        listed.contains(&format!("save: main moved to {fingerprint} \"Changed a\"")),
        "{listed}"
    );
    for shown in [said, listed] {
        assert!(!shown.contains(&change_id), "{shown}");
    }
}

#[test]
fn amend_json_says_what_it_replaced() {
    let env = project(&[("a.txt", "a\n")]);
    let old_id = main_tip(&env);
    let old = save_of(&env, old_id);
    write(&env, "b.txt", "b\n");
    let doc = json(&env, &["save", "--amend", "Files and b"]);
    assert_eq!(doc["result"], "saved", "{doc}");
    assert_eq!(doc["replaced"], old_id.to_hex());
    assert_eq!(doc["save"]["id"], main_tip(&env).to_hex());
    assert_eq!(doc["save"]["change_id"], old.change_id.to_letters());
    assert_eq!(doc["save"]["message"], "Files and b");
    assert_eq!(doc["changed_files"], 2);
    assert_eq!(doc["paths"], json!([]));
    // Any other save replaced nothing.
    write(&env, "c.txt", "c\n");
    let doc = json(&env, &["save", "Add c"]);
    assert_eq!(doc.get("replaced"), Some(&Value::Null), "{doc}");
}

#[cfg(feature = "fault-injection")]
#[test]
fn an_amend_stopped_at_every_write_leaves_the_old_save_on_the_line() {
    let template = project(&[("a.txt", "a\n")]);
    let old_id = main_tip(&template);
    let old = save_of(&template, old_id);
    write(&template, "a.txt", "A\n");
    let (at, _) = head(&template);
    let edited = files(template.root());
    for n in 1.. {
        assert!(n < 100, "the amend never finished");
        let env = Env::empty();
        support::copy_dir(template.root(), env.root());
        let out = env
            .bolt()
            .args(["save", "--amend", "New"])
            .env("BOLT_FAULT_AT", n.to_string())
            .output()
            .unwrap();
        let crashed = text(&out.stderr).contains("injected crash");
        // An amend never writes into the folder.
        assert_eq!(files(env.root()), edited, "fault {n}");
        let packs = env.root().join(".bolt/store/packs");
        let report = bolt_core::verify::verify_store(&packs);
        assert!(report.is_ok(), "fault {n}: {:?}", report.problems);
        let tip = main_tip(&env);
        if head(&env).0 == at {
            assert_eq!(tip, old_id, "fault {n}");
            assert_eq!(changed(&env), ["a.txt"], "fault {n}");
        } else {
            let new = save_of(&env, tip);
            assert_eq!(new.change_id, old.change_id, "fault {n}");
            assert_eq!(new.message, "New", "fault {n}");
            assert!(changed(&env).is_empty(), "fault {n}");
        }
        if !crashed {
            assert!(n > 1, "no fault point was reached");
            assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
            assert_ne!(tip, old_id);
            break;
        }
    }
}
