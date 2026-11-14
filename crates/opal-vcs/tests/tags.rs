//! `opalro tag`: making, listing, moving and deleting tags.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use opal_core::diary::{Action, DiaryEntry, Position};
use opal_core::id::{Id, hash_content};
use opal_core::journal::Journal;
use opal_core::repo::Repository;

mod support;
use support::{Env, text};

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

/// `opalro args`, which must exit with `code`; its stdout and stderr.
fn run(env: &Env, args: &[&str], code: i32) -> (String, String) {
    let out = env.opalro().args(args).output().unwrap();
    let (shown, said) = (text(&out.stdout), text(&out.stderr));
    assert_eq!(out.status.code(), Some(code), "{args:?}: {shown}{said}");
    (shown, said)
}

/// `opalro args --json`, which must exit with `code`; its document.
fn json(env: &Env, args: &[&str], code: i32) -> serde_json::Value {
    let mut args = args.to_vec();
    args.push("--json");
    serde_json::from_str(&run(env, &args, code).0).unwrap()
}

/// The project's tags, each with its save and message.
fn tags(env: &Env) -> Vec<(String, Id, String)> {
    let (repo, state) = env.state();
    let list = repo.tag_list(&state).unwrap();
    (list.tags.into_iter())
        .map(|(name, tag)| (name, tag.save, tag.message))
        .collect()
}

fn tip(env: &Env) -> Id {
    env.state().1.lines["main"]
}

/// A project with `v1` at its save "A", then a save "B" on `main`: returns the two saves.
fn tagged_then_saved() -> (Env, Id, Id) {
    let env = project();
    let a = tip(&env);
    run(&env, &["tag", "v1"], 0);
    env.overwrite("a.txt", "b");
    run(&env, &["save", "B"], 0);
    let b = tip(&env);
    (env, a, b)
}

#[test]
fn tag_names_the_save_you_are_at_and_the_list_shows_it() {
    let env = project();
    let at = tip(&env);
    let (shown, said) = run(&env, &["tag", "v1.0"], 0);
    assert!(shown.starts_with("Tagged save "), "{shown}");
    assert!(shown.ends_with(" \"A\" as v1.0.\n"), "{shown}");
    assert_eq!(said, "");
    assert_eq!(tags(&env), [("v1.0".to_owned(), at, String::new())]);
    let (repo, _) = env.state();
    let (_, entry) = repo.current().unwrap();
    assert_eq!(entry.command, "tag");
    assert_eq!(entry.folder_change, None);
    drop(repo);
    let (shown, _) = run(&env, &["tag"], 0);
    assert_eq!(shown, format!("1 tag:\n\n  v1.0  {}\n", at.short_hex()));
}

#[test]
fn tag_takes_a_message() {
    let env = project();
    let at = tip(&env);
    run(&env, &["tag", "v1.0", "First playable"], 0);
    run(&env, &["tag", "release/2026-11"], 0);
    assert_eq!(
        tags(&env)[1],
        ("v1.0".to_owned(), at, "First playable".to_owned())
    );
    let (shown, _) = run(&env, &["tag"], 0);
    let hex = at.short_hex();
    assert_eq!(
        shown,
        format!(
            "2 tags:\n\n  release/2026-11  {hex}\n  v1.0             {hex}  \"First playable\"\n"
        )
    );
}

#[test]
fn tag_save_tags_another_save() {
    let env = project();
    let a = tip(&env);
    run(&env, &["line", "new", "side"], 0);
    env.overwrite("a.txt", "side");
    run(&env, &["save", "X"], 0);
    let x = env.state().1.lines["side"];
    run(&env, &["goto", "main"], 0);
    env.overwrite("a.txt", "b");
    run(&env, &["save", "B"], 0);
    let history = json(&env, &["history"], 0);
    let short = history["saves"][1]["change_id_short"].as_str().unwrap();
    let (shown, _) = run(&env, &["tag", "old", "--save", short], 0);
    assert!(shown.ends_with(" \"A\" as old.\n"), "{shown}");
    run(&env, &["tag", "on-side", "--save", "side"], 0);
    let at: Vec<(String, Id)> = tags(&env).into_iter().map(|(n, s, _)| (n, s)).collect();
    assert_eq!(at, [("old".to_owned(), a), ("on-side".to_owned(), x)]);
}

#[test]
fn tag_refuses_a_bad_name_and_a_lines_name() {
    let env = project();
    let before = env.standing();
    let (shown, said) = run(&env, &["tag", "has space"], 1);
    assert_eq!(shown, "");
    assert!(
        said.contains("error: \"has space\" can't be a tag's name: "),
        "{said}"
    );
    assert!(
        said.contains("hint: A tag's name takes letters, digits, - _ . and /."),
        "{said}"
    );
    let (_, said) = run(&env, &["tag", "main"], 1);
    assert!(
        said.contains("error: \"main\" is a line's name already, and a tag can't share it."),
        "{said}"
    );
    assert_eq!(env.standing(), before);
    assert!(tags(&env).is_empty());
}

#[test]
fn tagging_the_same_save_twice_says_already() {
    let env = project();
    run(&env, &["tag", "v1"], 0);
    let before = env.standing();
    let (shown, said) = run(&env, &["tag", "v1"], 0);
    assert!(shown.starts_with("Tag v1 is already at save "), "{shown}");
    assert!(shown.ends_with(" \"A\".\n"), "{shown}");
    assert_eq!(said, "");
    assert_eq!(env.standing(), before);
    let doc = json(&env, &["tag", "v1"], 0);
    assert_eq!(doc["changed"], false, "{doc}");
    assert_eq!(doc["moved_from"], serde_json::Value::Null, "{doc}");
}

#[test]
fn a_message_for_a_tag_already_there_is_left_as_it_is() {
    let env = project();
    run(&env, &["tag", "v1", "First"], 0);
    let before = env.standing();
    let (shown, _) = run(&env, &["tag", "v1", "Other"], 0);
    assert!(shown.starts_with("Tag v1 is already at save "), "{shown}");
    assert!(
        shown.ends_with(" \"A\".\n\n    Its message was left as it is.\n"),
        "{shown}"
    );
    assert_eq!(env.standing(), before);
    assert_eq!(tags(&env)[0].2, "First");
}

#[test]
fn moving_a_tag_without_a_terminal_stops_and_names_yes() {
    let (env, a, _) = tagged_then_saved();
    let before = env.standing();
    let (shown, said) = run(&env, &["tag", "v1"], 1);
    assert_eq!(shown, "");
    assert!(
        said.contains("error: Tag v1 is at another save, "),
        "{said}"
    );
    assert!(
        said.contains(", and Opalro can't ask whether to move it without a terminal."),
        "{said}"
    );
    assert!(
        said.contains("hint: To move it, run the same command with --yes."),
        "{said}"
    );
    let doc = json(&env, &["tag", "v1"], 1);
    let message = doc["error"]["message"].as_str().unwrap();
    assert!(
        message.ends_with("can't ask whether to move it with --json."),
        "{doc}"
    );
    assert_eq!(
        doc["error"]["hint"],
        "To move it, run the same command with --yes."
    );
    assert_eq!(env.standing(), before);
    assert_eq!(tags(&env)[0].1, a);
}

#[test]
fn tag_yes_moves_it() {
    let (env, a, b) = tagged_then_saved();
    let (shown, _) = run(&env, &["tag", "v1", "--yes"], 0);
    assert!(shown.starts_with("Moved tag v1 to save "), "{shown}");
    assert!(shown.contains(" \"B\". It was at save "), "{shown}");
    // The old save by its ID alone.
    assert!(!shown.contains("\"A\""), "{shown}");
    assert_eq!(tags(&env)[0].1, b);
    run(&env, &["undo"], 0);
    assert_eq!(tags(&env)[0].1, a);
    let doc = json(&env, &["tag", "v1", "--yes"], 0);
    assert_eq!(doc["moved_from"], a.to_hex(), "{doc}");
    assert_eq!(doc["changed"], true, "{doc}");
}

/// `opalro args` in a faked terminal, with `input` typed: its exit code, stdout and stderr.
#[cfg(feature = "fault-injection")]
fn asked(env: &Env, args: &[&str], input: &str) -> (Option<i32>, String, String) {
    let out = env
        .opalro()
        .args(args)
        .env("OPALRO_FAKE_TERMINAL", "80")
        .env("OPALRO_ASCII", "1")
        .env("NO_COLOR", "1")
        .write_stdin(input)
        .output()
        .unwrap();
    (out.status.code(), text(&out.stdout), text(&out.stderr))
}

#[cfg(feature = "fault-injection")]
#[test]
fn moving_a_tag_asks_in_a_terminal() {
    let (env, a, b) = tagged_then_saved();
    let before = env.standing();
    // With --json, even at a terminal, nothing is asked.
    let (code, shown, said) = asked(&env, &["tag", "v1", "--json"], "y\n");
    assert_eq!(code, Some(1), "{said}");
    assert!(!said.contains("Move it"), "{said}");
    let doc: serde_json::Value = serde_json::from_str(&shown).unwrap();
    let message = doc["error"]["message"].as_str().unwrap();
    assert!(message.ends_with("move it with --json."), "{doc}");
    let (code, shown, said) = asked(&env, &["tag", "v1"], "n\n");
    assert_eq!(code, Some(0), "{said}");
    assert_eq!(shown, "Nothing was changed.\n");
    assert!(said.starts_with("Tag v1 is at save "), "{said}");
    assert!(said.contains(" \"A\".\nMove it to save "), "{said}");
    assert!(said.contains(" \"B\"? [y/N] "), "{said}");
    assert_eq!(env.standing(), before);
    assert_eq!(tags(&env)[0].1, a);
    let (code, shown, said) = asked(&env, &["tag", "v1"], "y\n");
    assert_eq!(code, Some(0), "{said}");
    assert!(shown.starts_with("Moved tag v1 to save "), "{shown}");
    assert_eq!(tags(&env)[0].1, b);
}

#[cfg(feature = "fault-injection")]
#[test]
fn deleting_a_tag_asks_in_a_terminal() {
    let env = project();
    run(&env, &["tag", "v1"], 0);
    let before = env.standing();
    // With --json, even at a terminal, nothing is asked.
    let (code, shown, said) = asked(&env, &["tag", "--delete", "v1", "--json"], "y\n");
    assert_eq!(code, Some(1), "{said}");
    assert!(!said.contains("Delete tag"), "{said}");
    let doc: serde_json::Value = serde_json::from_str(&shown).unwrap();
    assert_eq!(
        doc["error"]["message"],
        "Opalro asks before it deletes a tag, and can't with --json."
    );
    assert_eq!(env.standing(), before);
    let (code, shown, said) = asked(&env, &["tag", "--delete", "v1"], "n\n");
    assert_eq!(code, Some(0), "{said}");
    assert_eq!(shown, "Nothing was changed.\n");
    assert!(said.starts_with("Delete tag v1, at save "), "{said}");
    assert!(said.contains(" \"A\"? [y/N] "), "{said}");
    assert_eq!(env.standing(), before);
    let (code, shown, said) = asked(&env, &["tag", "--delete", "v1"], "y\n");
    assert_eq!(code, Some(0), "{said}");
    assert!(shown.starts_with("Deleted tag v1, "), "{shown}");
    assert!(tags(&env).is_empty());
}

#[test]
fn tag_delete_without_a_terminal_stops() {
    let env = project();
    run(&env, &["tag", "v1"], 0);
    let before = env.standing();
    let (shown, said) = run(&env, &["tag", "--delete", "v1"], 1);
    assert_eq!(shown, "");
    assert!(
        said.contains("error: Opalro asks before it deletes a tag, and can't without a terminal."),
        "{said}"
    );
    assert!(
        said.contains("\nhint: To delete it, run the same command with --yes.\n"),
        "{said}"
    );
    let doc = json(&env, &["tag", "--delete", "v1"], 1);
    assert_eq!(
        doc["error"]["message"],
        "Opalro asks before it deletes a tag, and can't with --json."
    );
    assert_eq!(
        doc["error"]["hint"],
        "To delete it, run the same command with --yes."
    );
    assert_eq!(env.standing(), before);
    assert_eq!(tags(&env).len(), 1);
    // A tag that isn't there is told before anything is asked.
    let (_, said) = run(&env, &["tag", "--delete", "v2"], 1);
    assert!(said.contains("There is no tag named \"v2\"."), "{said}");
}

#[test]
fn tag_delete_yes_deletes_and_undo_brings_it_back() {
    let env = project();
    let at = tip(&env);
    run(&env, &["tag", "v1", "Mine"], 0);
    let (shown, _) = run(&env, &["tag", "--delete", "v1", "--yes"], 0);
    assert!(
        shown.starts_with("Deleted tag v1, which was at save "),
        "{shown}"
    );
    assert!(shown.ends_with(" \"A\".\n"), "{shown}");
    assert!(tags(&env).is_empty());
    let (_, entry) = Repository::open(env.root()).unwrap().current().unwrap();
    assert_eq!(entry.command, "tag");
    run(&env, &["undo"], 0);
    assert_eq!(tags(&env), [("v1".to_owned(), at, "Mine".to_owned())]);
}

#[test]
fn no_tags_says_how_to_make_one() {
    let env = project();
    let (shown, said) = run(&env, &["tag"], 0);
    assert_eq!(
        shown,
        "No tags: opalro tag <name> names the save you're at.\n"
    );
    assert_eq!(said, "");
    assert_eq!(json(&env, &["tag"], 0)["tags"], serde_json::json!([]));
}

#[test]
fn an_imported_tags_name_and_message_are_escaped() {
    let env = Env::new();
    env.commit("a.txt", "a\n", "First");
    // Not "…exe": Windows Defender quarantines Git's ref file for such a name.
    let name = "v1\u{202e}ateb";
    env.git(&["tag", "-a", name, "-m", "Red \u{1b}[31mhere\nSecond line"]);
    env.import();
    let listed = json(&env, &["tag"], 0);
    assert_eq!(listed["tags"][0]["name"], name, "{listed}");
    for args in [&["tag"][..], &["tag", name], &["tag", "--delete", name]] {
        let out = env.opalro().args(args).output().unwrap();
        let said = text(&out.stdout) + &text(&out.stderr);
        support::assert_escaped(&said, &format!("{args:?}"));
    }
    let (said, _) = run(&env, &["tag", name], 0);
    assert!(
        said.starts_with("Tag \"v1\\u{202e}ateb\" is already "),
        "{said}"
    );
    let (shown, _) = run(&env, &["tag"], 0);
    assert!(shown.contains("  \"v1\\u{202e}ateb\"  "), "{shown}");
    let (shown, _) = run(&env, &["tag"], 0);
    assert!(shown.contains("\"Red \\x1b[31mhere\"\n"), "{shown}");
    assert!(!shown.contains("Second line"), "{shown}");
}

#[cfg(feature = "fault-injection")]
#[test]
fn an_imported_tags_questions_are_escaped() {
    let env = Env::new();
    env.commit("a.txt", "a\n", "First");
    let name = "v1\u{202e}ateb";
    env.git(&["tag", "-a", name, "-m", "Red \u{1b}[31mhere"]);
    env.import();
    let (code, shown, said) = asked(&env, &["tag", "--delete", name], "n\n");
    assert_eq!(code, Some(0), "{said}");
    assert!(
        said.starts_with("Delete tag \"v1\\u{202e}ateb\", at save "),
        "{said}"
    );
    support::assert_escaped(&(shown + &said), "the delete question");
    env.overwrite("a.txt", "b");
    run(&env, &["save", "B"], 0);
    let (code, shown, said) = asked(&env, &["tag", name], "n\n");
    assert_eq!(code, Some(0), "{said}");
    assert!(
        said.starts_with("Tag \"v1\\u{202e}ateb\" is at save "),
        "{said}"
    );
    assert!(said.contains("Move it to save "), "{said}");
    support::assert_escaped(&(shown + &said), "the move question");
}

/// Names no line could have, each able to forge its own end unquoted, and how each is shown.
const ODD: [(&str, &str); 6] = [
    ("v1 b", "\"v1 b\""),
    ("v9]  [main", "\"v9]  [main\""),
    ("a, b", "\"a, b\""),
    ("say \"hi\"", "\"say \\\"hi\\\"\""),
    ("a\\b", "\"a\\\\b\""),
    ("bell\u{7}", "\"bell\\x07\""),
];

#[test]
fn an_odd_tags_name_is_shown_between_quotes() {
    let env = project();
    let at = tip(&env);
    for (name, shown) in ODD {
        env.plant_tags([name.to_owned(), "v1.0".to_owned()].into_iter(), at);
        let (list, _) = run(&env, &["tag"], 0);
        assert!(list.contains(&format!("\n  {shown}  ")), "{list}");
        // A name that fits the rules for line names is shown as it is.
        assert!(list.contains("\n  v1.0  "), "{list}");
        let (history, _) = run(&env, &["history"], 0);
        assert!(history.contains(&format!("  [tag {shown}]")), "{history}");
        assert!(history.contains("  [tag v1.0]"), "{history}");
        let (shown_save, _) = run(&env, &["show"], 0);
        let row = shown_save.lines().find(|l| l.contains("Tags ")).unwrap();
        assert!(row.contains(shown), "{row}");
        // `--json` gives the stored name.
        let listed = json(&env, &["tag"], 0);
        let names: Vec<&str> = (listed["tags"].as_array().unwrap().iter())
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        assert!(names.contains(&name), "{listed}");
        assert!(
            json(&env, &["show"], 0)["tags"]
                .as_array()
                .unwrap()
                .contains(&serde_json::json!(name))
        );
        let (deleted, _) = run(&env, &["tag", "--delete", name, "--yes"], 0);
        assert!(
            deleted.starts_with(&format!("Deleted tag {shown}, which was at save ")),
            "{deleted}"
        );
    }
}

#[test]
fn an_odd_tags_name_is_quoted_where_a_move_is_refused() {
    let (env, a, _) = tagged_then_saved();
    let (name, shown) = ODD[1];
    env.plant_tags([name.to_owned()].into_iter(), a);
    let (_, said) = run(&env, &["tag", name], 1);
    assert!(
        said.contains(&format!("error: Tag {shown} is at another save, ")),
        "{said}"
    );
    let (moved, _) = run(&env, &["tag", name, "--yes"], 0);
    assert!(
        moved.starts_with(&format!("Moved tag {shown} to save ")),
        "{moved}"
    );
    let (already, _) = run(&env, &["tag", name], 0);
    assert!(
        already.starts_with(&format!("Tag {shown} is already at save ")),
        "{already}"
    );
}

#[test]
fn the_hint_to_delete_a_tag_carries_no_name() {
    let env = project();
    let name = "v1; touch pwned";
    env.plant_tags([name.to_owned()].into_iter(), tip(&env));
    let (_, said) = run(&env, &["tag", "--delete", name], 1);
    let hint = said.lines().find(|l| l.starts_with("hint:")).unwrap();
    assert_eq!(hint, "hint: To delete it, run the same command with --yes.");
}

#[cfg(feature = "fault-injection")]
#[test]
fn an_odd_tags_name_is_quoted_in_the_questions() {
    let (env, a, _) = tagged_then_saved();
    for (name, shown) in ODD {
        env.plant_tags([name.to_owned()].into_iter(), a);
        let (code, _, said) = asked(&env, &["tag", "--delete", name], "n\n");
        assert_eq!(code, Some(0), "{said}");
        assert!(
            said.starts_with(&format!("Delete tag {shown}, at save ")),
            "{said}"
        );
        let (code, _, said) = asked(&env, &["tag", name], "n\n");
        assert_eq!(code, Some(0), "{said}");
        assert!(
            said.starts_with(&format!("Tag {shown} is at save ")),
            "{said}"
        );
    }
}

#[cfg(feature = "fault-injection")]
#[test]
fn history_fits_a_long_odd_tags_name_in_60_columns() {
    let env = project();
    env.overwrite("a.txt", "b");
    run(&env, &["save", &"A long message ".repeat(10)], 0);
    // Long, and longer still as shown, though the row has room for it whole.
    let name = "rc 7, \"final\"";
    env.plant_tags([name.to_owned()].into_iter(), tip(&env));
    let out = env
        .opalro()
        .arg("history")
        .env("OPALRO_FAKE_TERMINAL", "60")
        .env("OPALRO_ASCII", "1")
        .env("NO_COLOR", "1")
        .output()
        .unwrap();
    let shown = text(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    let row = shown.lines().find(|l| l.contains("[tag ")).unwrap();
    assert!(
        row.ends_with("  [main]  [tag \"rc 7, \\\"final\\\"\"]"),
        "{row}"
    );
    for row in shown.lines() {
        assert!(row.chars().count() <= 60, "{row:?}");
    }
}

#[test]
fn a_tag_step_after_a_stopped_goto_finishes_it_first() {
    let env = Env::empty();
    run(&env, &["start", "--preset", "custom"], 0);
    let first = Repository::open(env.root()).unwrap().head().unwrap();
    env.overwrite("a.txt", "a");
    run(&env, &["save", "A"], 0);
    let saved = tip(&env);
    // Back as a goto to the save "A" that stopped before `head` moved would leave it.
    let mut repo = Repository::open(env.root()).unwrap();
    let second = repo.head().unwrap();
    let lock = repo.lock_for_change("goto", &mut |_| {}).unwrap();
    repo.move_head_to(&lock, first).unwrap();
    let journal = Journal {
        target: second,
        from: first,
        actions: vec![],
    };
    journal.write(&repo, &lock).unwrap();
    drop(lock);
    drop(repo);
    let out = env.opalro().args(["tag", "v1", "--json"]).output().unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    let doc: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(doc["warnings"][0]["kind"], "switch_finished", "{doc}");
    assert_eq!(tags(&env)[0].1, saved);
    assert!(!env.root().join(".opalro/state/journal").exists());
}

#[test]
fn goto_takes_a_tags_name_and_lands_at_its_save_not_on_a_line() {
    let (env, a, _) = tagged_then_saved();
    let (shown, said) = run(&env, &["goto", "v1"], 0);
    assert!(shown.starts_with("Switched to save "), "{shown}");
    assert!(shown.contains(" \"A\". You are not on a line."), "{shown}");
    assert_eq!(said, "");
    assert_eq!(env.state().1.position, Position::AtSave(a));
    assert_eq!(
        std::fs::read_to_string(env.root().join("a.txt")).unwrap(),
        "a"
    );
}

#[test]
fn show_and_changes_save_take_a_tags_name() {
    let (env, a, _) = tagged_then_saved();
    let shown = json(&env, &["show", "v1"], 0);
    assert_eq!(shown["save"]["id"], a.to_hex(), "{shown}");
    let (changed, _) = run(&env, &["changes", "--save", "v1"], 0);
    assert!(changed.contains("a.txt"), "{changed}");
    let (_, said) = run(&env, &["show", "v2"], 1);
    assert!(
        said.contains("No save, line or ID matches \"v2\"."),
        "{said}"
    );
}

#[test]
fn history_and_show_name_a_saves_tags() {
    let env = project();
    let at = tip(&env).to_hex();
    run(&env, &["tag", "v1.0"], 0);
    run(&env, &["tag", "release/2026-11"], 0);
    let (shown, _) = run(&env, &["history"], 0);
    let first = shown.lines().nth(2).unwrap();
    assert!(
        first.ends_with("  [main]  [tag release/2026-11]  [tag v1.0]"),
        "{shown}"
    );
    let (shown, _) = run(&env, &["show"], 0);
    assert!(
        shown.contains("\n    Line         main\n    Tags         release/2026-11, v1.0\n"),
        "{shown}"
    );
    let listed = json(&env, &["history"], 0);
    assert_eq!(
        listed["tags"],
        serde_json::json!([
            {"name": "release/2026-11", "save": at},
            {"name": "v1.0", "save": at},
        ]),
        "{listed}"
    );
    let shown = json(&env, &["show"], 0);
    assert_eq!(
        shown["tags"],
        serde_json::json!(["release/2026-11", "v1.0"]),
        "{shown}"
    );
}

#[test]
fn a_save_without_tags_shows_no_tags_row() {
    let (env, a, _) = tagged_then_saved();
    let (shown, _) = run(&env, &["show"], 0);
    assert!(shown.contains("\n    Line "), "{shown}");
    assert!(!shown.contains("\n    Tags "), "{shown}");
    assert_eq!(json(&env, &["show"], 0)["tags"], serde_json::json!([]));
    let (shown, _) = run(&env, &["history"], 0);
    let rows: Vec<&str> = shown.lines().skip(2).collect();
    assert!(rows[0].ends_with("  [main]"), "{shown}");
    assert!(rows[1].ends_with("  A  [tag v1]"), "{shown}");
    let listed = json(&env, &["history"], 0);
    assert_eq!(
        listed["tags"],
        serde_json::json!([{"name": "v1", "save": a.to_hex()}]),
        "{listed}"
    );
}

#[test]
fn history_json_leaves_out_a_tag_off_the_listed_line() {
    let env = project();
    let on_main = tip(&env);
    run(&env, &["tag", "v1.0"], 0);
    run(&env, &["line", "new", "side"], 0);
    env.overwrite("a.txt", "side");
    run(&env, &["save", "On side"], 0);
    run(&env, &["tag", "side-1.0"], 0);
    run(&env, &["goto", "main"], 0);
    let listed = json(&env, &["history"], 0);
    assert_eq!(
        listed["tags"],
        serde_json::json!([{"name": "v1.0", "save": on_main.to_hex()}]),
        "{listed}"
    );
}

#[test]
fn history_and_show_stop_at_a_tag_list_that_cannot_be_read() {
    let env = project();
    let at = tip(&env);
    // The state names a tag list that no pack holds.
    let mut repo = Repository::open(env.root()).unwrap();
    let lock = repo.lock_for_change("tag", &mut |_| {}).unwrap();
    let (head, entry) = repo.current().unwrap();
    let mut state = entry.state.clone();
    state.tags = Some(hash_content(b"no such tag list"));
    let diary = DiaryEntry {
        parent: Some(head),
        action: Action::Command,
        command: "tag".into(),
        who: entry.who,
        opal_version: "0.1.0".into(),
        state,
        folder_change: None,
    };
    let packs = repo.new_packs().unwrap();
    repo.commit(&lock, packs, &diary).unwrap();
    drop(lock);
    drop(repo);
    let damaged = "This project's Opalro data looks damaged: the project's tag list can't be read: \
                   object ";
    let hint = "Nothing was changed. Ask for help before trying to repair it.";
    let id = at.to_hex();
    for args in [&["history"][..], &["show"], &["goto", &id[..12]], &["tag"]] {
        let (shown, said) = run(&env, args, 1);
        assert_eq!(shown, "", "{args:?}: {said}");
        assert!(
            said.starts_with(&format!("error: {damaged}")),
            "{args:?}: {said}"
        );
        assert!(said.contains(" is not in the store.\n"), "{args:?}: {said}");
        assert!(said.contains(&format!("hint: {hint}")), "{args:?}: {said}");
        let doc = json(&env, args, 1);
        let told = doc["error"]["message"].as_str().unwrap();
        assert!(told.starts_with(damaged), "{args:?}: {doc}");
        assert_eq!(doc["error"]["hint"], hint, "{args:?}: {doc}");
        assert_eq!(doc.get("saves"), None, "{args:?}: {doc}");
    }
}

#[test]
fn tag_save_takes_a_tags_name() {
    let (env, a, _) = tagged_then_saved();
    let (shown, _) = run(&env, &["tag", "v1-copy", "--save", "v1"], 0);
    assert!(shown.ends_with(" \"A\" as v1-copy.\n"), "{shown}");
    assert_eq!(tags(&env)[1].1, a);
}

/// An imported project with `main` at "two" and `main@origin` at "one", that save given.
/// `before` runs in Git just before the import.
fn with_a_server_line(before: impl FnOnce(&Env)) -> (Env, Id) {
    let env = Env::new();
    let one = env.commit("a.txt", "one", "one");
    env.commit("a.txt", "two", "two");
    env.git(&["update-ref", "refs/remotes/origin/main", &one]);
    before(&env);
    env.import();
    let (repo, state) = env.state();
    let server = repo.server_lines(&state).unwrap().servers["origin"]["main"];
    (env, server)
}

#[test]
fn show_changes_and_tag_save_take_a_server_line() {
    let (env, server) = with_a_server_line(|_| {});
    let shown = json(&env, &["show", "main@origin"], 0);
    assert_eq!(shown["save"]["id"], server.to_hex(), "{shown}");
    let (changed, _) = run(&env, &["changes", "--save", "main@origin"], 0);
    assert!(changed.contains("a.txt"), "{changed}");
    let (shown, _) = run(&env, &["tag", "v2", "--save", "main@origin"], 0);
    assert!(shown.ends_with(" \"one\" as v2.\n"), "{shown}");
    assert_eq!(tags(&env)[0].1, server);
}

/// Each of goto, show, changes --save and tag --save refuses `name` with `message` and `hint`,
/// in text and in JSON, and changes nothing.
fn refused_everywhere(env: &Env, name: &str, message: &str, hint: &str) {
    let before = env.standing();
    let commands = [
        &["goto", name][..],
        &["show", name],
        &["changes", "--save", name],
        &["tag", "rel-1", "--save", name],
    ];
    for args in commands {
        let (shown, said) = run(env, args, 1);
        assert_eq!(shown, "", "{args:?}");
        assert!(
            said.contains(&format!("error: {message}\n")),
            "{args:?}: {said}"
        );
        assert!(
            said.contains(&format!("hint: {hint}\n")),
            "{args:?}: {said}"
        );
        let doc = json(env, args, 1);
        assert_eq!(doc["error"]["message"], message, "{args:?}: {doc}");
        assert_eq!(doc["error"]["hint"], hint, "{args:?}: {doc}");
    }
    assert_eq!(env.standing(), before);
}

fn hex12(id: Id) -> String {
    id.to_hex()[..12].to_owned()
}

#[test]
fn a_line_and_a_tag_of_one_name_at_two_saves_are_refused() {
    let env = Env::new();
    let first = env.commit("a.txt", "one", "one");
    env.git(&["tag", "v1.0", &first]);
    env.commit("a.txt", "two", "two");
    env.git(&["branch", "v1.0"]);
    env.import();
    let (repo, state) = env.state();
    let tag = repo.tag_list(&state).unwrap().tags["v1.0"].save;
    let line = state.lines["v1.0"];
    drop(repo);
    let message = format!(
        "\"v1.0\" is the name of a line and also of a tag at another save (line {}, tag {}), so \
         Opalro didn't pick either.",
        hex12(line),
        hex12(tag)
    );
    let hint = "To reach one of them, type its ID as shown here. To reach the line by another \
                name: opalro line rename <old> <new>";
    refused_everywhere(&env, "v1.0", &message, hint);
    // The way out: the line's own commands take its name as a line's.
    run(&env, &["line", "rename", "v1.0", "v1.0-line"], 0);
    run(&env, &["goto", "v1.0"], 0);
    assert_eq!(env.state().1.position, Position::AtSave(tag));
    run(&env, &["goto", "v1.0-line"], 0);
    assert_eq!(env.state().1.position, Position::OnLine("v1.0-line".into()));
}

#[test]
fn a_server_line_and_a_tag_of_one_name_at_two_saves_are_refused() {
    let (env, server) = with_a_server_line(|env| drop(env.git(&["tag", "main@origin", "main"])));
    let tag = tags(&env)[0].1;
    assert_ne!(tag, server);
    let message = format!(
        "\"main@origin\" is the name of a server line and also of a tag at another save (server \
         line {}, tag {}), so Opalro didn't pick either.",
        hex12(server),
        hex12(tag)
    );
    refused_everywhere(
        &env,
        "main@origin",
        &message,
        "To reach one of them, type its ID as shown here.",
    );
}

#[test]
fn a_hundred_thousand_tags_are_listed_and_searched_in_time() {
    let (env, a, _) = tagged_then_saved();
    // One short of the most a project holds, so that one more can still be made.
    env.plant_tags((1..100_000).map(|i| format!("v{i:06}")), a);
    assert_eq!(tags(&env).len(), 99_999);
    for args in [
        &["tag"][..],
        &["goto", "v050000"],
        &["history"],
        &["tag", "one-more"],
    ] {
        let mut cmd = env.opal_process_in(env.root());
        cmd.args(args);
        let ended = support::run_limited(cmd, 20);
        eprintln!("{args:?}: {:.2} s", ended.secs);
        assert_eq!(ended.code, Some(0), "{args:?}: {ended:?}");
    }
    assert_eq!(tags(&env).len(), 100_000);
    assert_eq!(env.state().1.position, Position::AtSave(a));
}

/// `opalro args --json`, which must succeed; its document, masked.
fn json_of(env: &Env, args: &[&str]) -> serde_json::Value {
    support::masked(json(env, args, 0))
}

#[test]
fn tag_list_json_matches_its_snapshot() {
    let env = project();
    run(&env, &["tag", "v1.0", "First playable"], 0);
    run(&env, &["tag", "release/2026-11"], 0);
    insta::assert_json_snapshot!(json_of(&env, &["tag"]));
}

#[test]
fn tag_json_matches_its_snapshot() {
    let env = project();
    insta::assert_json_snapshot!(json_of(&env, &["tag", "v1.0", "First playable"]));
}

#[test]
fn tag_delete_json_matches_its_snapshot() {
    let env = project();
    run(&env, &["tag", "v1.0"], 0);
    insta::assert_json_snapshot!(json_of(&env, &["tag", "--delete", "v1.0", "--yes"]));
}
