//! Crash recovery end to end: a lost `head`, picked up from the diary, and a failure after `head`
//! moved.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;
use std::path::PathBuf;

use bolt_core::diary::{Action, DiaryEntry, Position};
use bolt_core::id::Id;
use bolt_core::pack::PackReader;
use bolt_core::repo::Repository;

mod support;
use support::{Env, text};

fn write(env: &Env, path: &str, bytes: &str) {
    fs::write(env.root().join(path), bytes).unwrap();
}

fn read(env: &Env, path: &str) -> Vec<u8> {
    fs::read(env.root().join(path)).unwrap()
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

/// `bolt <args>`, which must fail; what it said, on stdout then stderr.
fn refused(env: &Env, args: &[&str]) -> (String, String) {
    let out = env.bolt().args(args).output().unwrap();
    assert_eq!(
        out.status.code(),
        Some(1),
        "{args:?}: {}",
        text(&out.stdout)
    );
    (text(&out.stdout), text(&out.stderr))
}

fn json(env: &Env, args: &[&str]) -> serde_json::Value {
    let out = env.bolt().args(args).arg("--json").output().unwrap();
    serde_json::from_slice(&out.stdout).unwrap()
}

fn head_file(env: &Env) -> PathBuf {
    env.root().join(".bolt/state/head")
}

/// The newest diary entry, with its ID.
fn head(env: &Env) -> (Id, DiaryEntry) {
    Repository::open(env.root()).unwrap().current().unwrap()
}

fn entry(env: &Env, id: Id) -> DiaryEntry {
    Repository::open(env.root()).unwrap().entry(&id).unwrap()
}

fn packs(env: &Env) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(env.root().join(".bolt/store/packs"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

/// A project whose diary is `start`, then saves `One` and `Two` of `a.txt`: the three entries.
fn project() -> (Env, [Id; 3]) {
    let env = Env::empty();
    ok(&env, &["start"]);
    let start = head(&env).0;
    write(&env, "a.txt", "one\n");
    ok(&env, &["save", "One"]);
    let one = head(&env).0;
    write(&env, "a.txt", "two\n");
    ok(&env, &["save", "Two"]);
    let two = head(&env).0;
    (env, [start, one, two])
}

/// Stores an entry after `parent`, holding its state as `change` leaves it, as a command that
/// crashed before its head moved leaves one.
fn plant_end(env: &Env, parent: Id, change: impl FnOnce(&mut DiaryEntry)) -> Id {
    let mut repo = Repository::open(env.root()).unwrap();
    let lock = repo.lock_for_change("test", &mut |_| {}).unwrap();
    let mut planted = repo.entry(&parent).unwrap();
    planted.parent = Some(parent);
    planted.action = Action::Command;
    planted.command = "save".into();
    change(&mut planted);
    let packs = repo.new_packs().unwrap();
    repo.store_entry(&lock, packs, &planted).unwrap()
}

/// Removes the pack that holds the diary entry `id`, as a failing disk or a sync tool can.
fn remove_pack_holding(env: &Env, id: Id) {
    let folder = env.root().join(".bolt/store/packs");
    for name in packs(env) {
        let pack = folder.join(name);
        if PackReader::open(&pack)
            .unwrap()
            .find(&id)
            .unwrap()
            .is_some()
        {
            fs::remove_file(pack).unwrap();
        }
    }
}

const LOST: &str = "error: Bolt can't tell which diary entry is the project's current one: \
                    .bolt/state/head";

#[test]
fn a_missing_head_stops_every_command_saying_the_saves_are_safe() {
    let (env, _) = project();
    write(&env, "a.txt", "edited\n");
    fs::remove_file(head_file(&env)).unwrap();
    let stored = packs(&env);
    for args in [
        &["status"][..],
        &["history"],
        &["show"],
        &["line"],
        &["changes"],
        &["save", "Three"],
        &["goto", "main"],
        &["discard"],
        &["shelf"],
        &["unshelf"],
        &["undo"],
        &["redo"],
    ] {
        let (_, said) = refused(&env, args);
        assert!(
            said.contains(&format!("{LOST} is missing.\n")),
            "{args:?}: {said}"
        );
        assert!(
            said.contains("hint: Your saves are safe"),
            "{args:?}: {said}"
        );
        assert!(
            said.contains("bolt undo --list") && said.contains("bolt undo --to <entry>"),
            "{args:?}: {said}"
        );
    }
    // Nothing was written: no head, no pack, and the edit is as it was.
    assert!(!head_file(&env).exists());
    assert_eq!(packs(&env), stored);
    assert_eq!(read(&env, "a.txt"), b"edited\n");
    let doc = json(&env, &["status"]);
    let message = doc["error"]["message"].as_str().unwrap();
    assert!(message.ends_with(".bolt/state/head is missing."), "{doc}");
    let hint = doc["error"]["hint"].as_str().unwrap();
    assert!(hint.starts_with("Your saves are safe"), "{doc}");
}

#[test]
fn a_garbled_head_and_a_head_naming_a_missing_entry_are_lost_too() {
    let (env, [.., two]) = project();
    let save = head(&env).1.state.lines["main"];
    let never = Id([0xab; 32]);
    let not_an_id = "isn't a diary entry's ID and a newline".to_owned();
    let not_held = |id: Id| {
        format!(
            "names diary entry {}, which the store doesn't hold",
            id.short_hex()
        )
    };
    for (written, why) in [
        ("not an ID\n".to_owned(), not_an_id.clone()),
        (two.to_hex().to_uppercase() + "\n", not_an_id.clone()),
        (two.to_hex(), not_an_id.clone()),
        ("0".repeat(100), not_an_id.clone()),
        (format!("{}\n", never.to_hex()), not_held(never)),
        // A save's ID: the store holds no diary entry by that name.
        (format!("{}\n", save.to_hex()), not_held(save)),
    ] {
        fs::write(head_file(&env), &written).unwrap();
        for args in [&["status"][..], &["history"], &["save", "Three"]] {
            let (_, said) = refused(&env, args);
            assert!(
                said.contains(&format!("{LOST} {why}.\n")),
                "{written:?} {args:?}: {said}"
            );
            assert!(said.contains("hint: Your saves are safe"), "{said}");
        }
        // The diary can still be read to pick it up from.
        let listed = ok(&env, &["undo", "--list"]);
        assert!(listed.contains(&two.short_hex()), "{written:?}: {listed}");
        assert_eq!(fs::read(head_file(&env)).unwrap(), written.as_bytes());
    }
    // And a garbled head is picked up as a missing one is.
    fs::write(head_file(&env), "not an ID\n").unwrap();
    ok(&env, &["undo", "--to", &two.to_hex()[..8]]);
    assert_eq!(head(&env).1.parent, Some(two));
}

#[test]
fn undo_list_with_a_lost_head_lists_the_entries_no_other_follows_newest_first() {
    let (env, [start, one, two]) = project();
    let made = head(&env).1.who.time_ms;
    // Two crashed commands: one after `One`, newer than `Two`, and one after the first entry,
    // long ago.
    let newer = plant_end(&env, one, |e| e.who.time_ms = made + 60_000);
    let older = plant_end(&env, start, |e| e.who.time_ms = 1_000);
    fs::remove_file(head_file(&env)).unwrap();
    for args in [&["undo", "--list"][..], &["undo", "--list", "--all"]] {
        let said = ok(&env, args);
        let lines: Vec<&str> = said.lines().collect();
        assert_eq!(lines.len(), 5, "{args:?}: {said}");
        assert!(lines[0].contains("current diary entry is lost"), "{said}");
        for (line, id) in lines[1..4].iter().zip([newer, two, older]) {
            assert!(line.starts_with(&id.short_hex()), "{said}");
            assert!(line.contains("  save"), "{said}");
        }
        assert!(lines[2].ends_with("\"Two\""), "{said}");
        assert!(lines[4].contains("bolt undo --to <entry>"), "{said}");
        // Listing writes nothing.
        assert!(!head_file(&env).exists());
    }
    let doc = json(&env, &["undo", "--list"]);
    assert_eq!(doc["bolt_json"], 1);
    assert_eq!(doc["head_lost"], true, "{doc}");
    assert_eq!(doc["warnings"], serde_json::json!([]));
    let ends: Vec<&str> = doc["ends"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["entry"].as_str().unwrap())
        .collect();
    let hex = [newer, two, older].map(|id| id.to_hex());
    assert_eq!(ends, hex.iter().map(String::as_str).collect::<Vec<_>>());
    let listed = &doc["ends"][1];
    assert_eq!(listed["action"], "command");
    assert_eq!(listed["command"], "save");
    assert_eq!(listed["first"], false);
    assert_eq!(listed["effects"][0]["kind"], "line", "{doc}");
    assert_eq!(doc["ends"][2]["time_ms"], 1_000);
    // With every pack gone too, there is nothing to go on from, and the list says so.
    for pack in packs(&env) {
        fs::remove_file(env.root().join(".bolt/store/packs").join(pack)).unwrap();
    }
    let said = ok(&env, &["undo", "--list"]);
    assert!(
        said.contains("no diary entry in the store can be read to go on from"),
        "{said}"
    );
    assert_eq!(
        json(&env, &["undo", "--list"])["ends"],
        serde_json::json!([])
    );
}

#[test]
fn undo_to_with_a_lost_head_makes_that_entry_current_without_touching_files() {
    let (env, [start, one, two]) = project();
    let crashed = plant_end(&env, two, |_| {});
    write(&env, "a.txt", "edited\n");
    write(&env, "n.txt", "new\n");
    fs::remove_file(head_file(&env)).unwrap();
    // Too few digits, none matching, as ever.
    let (_, said) = refused(&env, &["undo", "--to", &one.to_hex()[..7]]);
    assert!(said.contains("at least 8 hex digits"), "{said}");
    let ids = [start, one, two, crashed].map(|id| id.to_hex());
    let none = (0..16)
        .map(|d| format!("{d:x}{}", &ids[0][1..8]))
        .find(|p| !ids.iter().any(|id| id.starts_with(p.as_str())))
        .unwrap();
    let (_, said) = refused(&env, &["undo", "--to", &none]);
    assert!(
        said.contains("No diary entry in the store has an ID starting with"),
        "{said}"
    );
    assert!(said.contains("bolt undo --list"), "{said}");
    assert!(!head_file(&env).exists());

    let said = ok(&env, &["undo", "--to", &one.to_hex()[..8]]);
    let made = format!(
        "Made entry {} (save) the current one: on main at ",
        one.short_hex()
    );
    assert!(said.contains(&made), "{said}");
    assert!(said.contains("No file was changed"), "{said}");
    // One new step after that entry, holding its state, and no file changed.
    let (_, now) = head(&env);
    let picked = entry(&env, one);
    assert_eq!(now.parent, Some(one));
    assert_eq!(now.action, Action::Command);
    assert_eq!(now.command, "undo");
    assert_eq!(now.state, picked.state);
    assert_eq!(now.folder_change, None);
    assert_eq!(read(&env, "a.txt"), b"edited\n");
    assert_eq!(read(&env, "n.txt"), b"new\n");
    // Status shows how the folder differs from that state's save.
    let status = json(&env, &["status"]);
    assert_eq!(status["position"], serde_json::json!({"line": "main"}));
    assert_eq!(status["save"]["id"], picked.state.lines["main"].to_hex());
    let changed: Vec<&str> = status["changes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["path"].as_str().unwrap())
        .collect();
    assert_eq!(changed, ["a.txt", "n.txt"], "{status}");

    // Any entry the store holds will do, one a crash left among them.
    fs::remove_file(head_file(&env)).unwrap();
    let doc = json(&env, &["undo", "--to", &crashed.to_hex()[..10]]);
    assert_eq!(doc["head_lost"], true, "{doc}");
    assert_eq!(doc["finished"], false);
    assert_eq!(doc["shelved"], serde_json::json!([]));
    assert_eq!(doc["position"], serde_json::json!({"line": "main"}));
    let step = &doc["steps"][0];
    assert_eq!(step["entry"], crashed.to_hex(), "{doc}");
    assert_eq!(step["action"], "jump");
    assert_eq!(step["effects"], serde_json::json!([]));
    assert_eq!(step["written"], 0);
    assert_eq!(step["removed"], 0);
    let (_, now) = head(&env);
    assert_eq!(now.parent, Some(crashed));
    assert_eq!(now.state.position, Position::OnLine("main".into()));
    assert_eq!(read(&env, "a.txt"), b"edited\n");
}

#[cfg(feature = "fault-injection")]
#[test]
fn a_failed_flush_after_head_moves_is_a_warning_and_the_save_stands() {
    let (env, [_, one, two]) = project();
    write(&env, "a.txt", "three\n");
    let out = env
        .bolt()
        .args(["save", "Three"])
        .env("BOLT_FAULT_FAIL", "head:renamed")
        .output()
        .unwrap();
    let said = text(&out.stderr);
    assert_eq!(out.status.code(), Some(0), "{said}");
    assert!(
        said.contains(
            "warning: The command happened, but Bolt couldn't flush .bolt/state to the disk \
             after it (injected fault at head:renamed), so a power cut soon after could undo \
             it.\n"
        ),
        "{said}"
    );
    // The save stands.
    let (_, now) = head(&env);
    assert_eq!(now.parent, Some(two));
    assert_eq!(json(&env, &["status"])["changes"], serde_json::json!([]));
    // A switch too: done, its journal gone, and the warning has its kind.
    let save = entry(&env, one).state.lines["main"];
    let out = env
        .bolt()
        .args(["goto", &save.to_hex(), "--json"])
        .env("BOLT_FAULT_FAIL", "head:renamed")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    let doc: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let kinds: Vec<&str> = doc["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| w["kind"].as_str().unwrap())
        .collect();
    assert!(kinds.contains(&"head_not_flushed"), "{doc}");
    assert_eq!(read(&env, "a.txt"), b"one\n");
    assert!(!env.root().join(".bolt/state/journal").exists());
    assert_eq!(head(&env).1.state.position, Position::AtSave(save));
}

/// Runs `bolt <args>` on copies of `template` until a run stops at `label`; that copy.
#[cfg(feature = "fault-injection")]
fn stopped_at(template: &Env, args: &[&str], label: &str) -> Env {
    for n in 1..100 {
        let env = Env::empty();
        support::copy_dir(template.root(), env.root());
        let out = env
            .bolt()
            .args(args)
            .env("BOLT_FAULT_AT", n.to_string())
            .output()
            .unwrap();
        let said = text(&out.stderr);
        assert!(
            said.contains("injected crash"),
            "{args:?} never stopped at {label}"
        );
        if said.contains(&format!("injected crash at {label}\n")) {
            return env;
        }
    }
    panic!("{args:?} never stopped at {label}");
}

#[cfg(feature = "fault-injection")]
#[test]
fn status_and_history_say_when_a_stopped_switch_left_only_its_journal() {
    let (template, [_, one, _]) = project();
    let save = entry(&template, one).state.lines["main"];
    let env = stopped_at(&template, &["goto", &save.to_hex()], "head:renamed");
    // The switch took effect: its head moved and every file is written.
    assert_eq!(head(&env).1.state.position, Position::AtSave(save));
    assert_eq!(read(&env, "a.txt"), b"one\n");
    assert!(env.root().join(".bolt/state/journal").exists());
    let pending = "warning: A switch between saves was stopped just after it was done: its files \
                   are all written, and the next command that changes the project removes its \
                   journal, .bolt/state/journal, first.\n";
    for args in [&["status"][..], &["history"]] {
        let out = env.bolt().args(args).output().unwrap();
        assert_eq!(out.status.code(), Some(0), "{args:?}");
        let said = text(&out.stderr);
        assert!(said.contains(pending), "{args:?}: {said}");
    }
    let status = json(&env, &["status"]);
    let message = status["unfinished_switch"].as_str().unwrap();
    assert!(
        message.starts_with("A switch between saves was stopped just after"),
        "{status}"
    );
    assert_eq!(status["changes"], serde_json::json!([]));
    // The next command that changes the project removes it, and then there is nothing to say.
    ok(&env, &["discard"]);
    assert!(!env.root().join(".bolt/state/journal").exists());
    let out = env.bolt().arg("status").output().unwrap();
    assert!(
        !text(&out.stderr).contains("warning:"),
        "{}",
        text(&out.stderr)
    );
}

#[cfg(feature = "fault-injection")]
#[test]
fn a_journal_left_or_unflushed_after_a_switch_is_a_warning_and_the_switch_stands() {
    let (env, [_, one, _]) = project();
    let save = entry(&env, one).state.lines["main"];
    let journal = env.root().join(".bolt/state/journal");
    // Removing the journal fails: the switch is done all the same, and the journal is left.
    let out = env
        .bolt()
        .args(["goto", &save.to_hex()])
        .env("BOLT_FAULT_FAIL", "journal:removing")
        .output()
        .unwrap();
    let said = text(&out.stderr);
    assert_eq!(out.status.code(), Some(0), "{said}");
    assert!(
        said.contains(
            "warning: The switch between saves is done, but Bolt couldn't remove its journal, \
             .bolt/state/journal (injected fault at journal:removing): the next command that \
             changes the project removes it first.\n"
        ),
        "{said}"
    );
    assert!(
        text(&out.stdout).contains("You're at save"),
        "{}",
        text(&out.stdout)
    );
    assert_eq!(head(&env).1.state.position, Position::AtSave(save));
    assert_eq!(read(&env, "a.txt"), b"one\n");
    assert!(journal.exists());
    let out = env.bolt().arg("status").output().unwrap();
    let said = text(&out.stderr);
    assert!(said.contains("stopped just after it was done"), "{said}");
    // The next command removes it first, then does its own work.
    ok(&env, &["goto", "main"]);
    assert!(!journal.exists());
    assert_eq!(read(&env, "a.txt"), b"two\n");
    // The flush after the removal failing is told as the head's own flush is.
    let out = env
        .bolt()
        .args(["goto", &save.to_hex(), "--json"])
        .env("BOLT_FAULT_FAIL", "journal:removed")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    let doc: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let flushed = doc["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|w| w["kind"] == "head_not_flushed");
    let message = flushed.unwrap()["message"].as_str().unwrap();
    assert!(
        message.contains("(injected fault at journal:removed)"),
        "{doc}"
    );
    assert!(!journal.exists());
    assert_eq!(read(&env, "a.txt"), b"one\n");
}

#[test]
fn undo_to_with_a_lost_head_refuses_an_entry_whose_saves_cant_be_read() {
    let (env, [_, _, two]) = project();
    // A crashed command's entry whose state names a line at a save the store doesn't hold.
    let broken = plant_end(&env, two, |e| {
        e.state.lines.insert("gone".into(), Id([0x77; 32]));
    });
    fs::remove_file(head_file(&env)).unwrap();
    let stored = packs(&env);
    let (_, said) = refused(&env, &["undo", "--to", &broken.to_hex()[..8]]);
    let why = format!(
        "error: Diary entry {} can't be made current: one of its saves can't be read (",
        broken.short_hex()
    );
    assert!(said.contains(&why), "{said}");
    assert!(said.contains("so nothing was changed.\n"), "{said}");
    assert!(
        said.contains("hint: Pick another: bolt undo --list shows"),
        "{said}"
    );
    assert!(!head_file(&env).exists());
    assert_eq!(packs(&env), stored);
    // Another entry will do.
    ok(&env, &["undo", "--to", &two.to_hex()[..8]]);
    assert_eq!(head(&env).1.parent, Some(two));
}

#[test]
fn undo_list_with_a_lost_head_goes_on_past_an_entry_whose_pack_is_gone() {
    let (env, [start, one, two]) = project();
    // The middle entry is lost with its pack, and the head too.
    remove_pack_holding(&env, one);
    fs::remove_file(head_file(&env)).unwrap();
    let said = ok(&env, &["undo", "--list"]);
    let lines: Vec<&str> = said.lines().collect();
    assert_eq!(lines.len(), 4, "{said}");
    // `Two`, whose parent can't be read, comes without what it changed; then the first entry,
    // which no entry left follows.
    assert!(lines[1].starts_with(&two.short_hex()), "{said}");
    assert!(lines[1].ends_with("  save"), "{said}");
    assert!(lines[2].starts_with(&start.short_hex()), "{said}");
    let doc = json(&env, &["undo", "--list"]);
    assert_eq!(doc["ends"][0]["entry"], two.to_hex(), "{doc}");
    assert_eq!(doc["ends"][0]["effects"], serde_json::json!([]));
    // It can be picked up from there.
    ok(&env, &["undo", "--to", &two.to_hex()[..8]]);
    assert_eq!(head(&env).1.parent, Some(two));
}

#[cfg(feature = "fault-injection")]
#[test]
fn a_stopped_switch_from_a_lost_head_is_left_alone() {
    let (env, [_, one, two]) = project();
    let save = entry(&env, one).state.lines["main"];
    // A goto stopped after its first change; then the entry it was made from is lost.
    let out = env
        .bolt()
        .args(["goto", &save.to_hex()])
        .env("BOLT_FAULT_FAIL", "switch:changed")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1), "{}", text(&out.stdout));
    let journal = env.root().join(".bolt/state/journal");
    assert!(journal.exists());
    remove_pack_holding(&env, two);
    let folder = read(&env, "a.txt");
    // Nothing is judged against the lost head: the pick-up finishes no switch.
    ok(&env, &["undo", "--to", &one.to_hex()[..8]]);
    assert_eq!(head(&env).1.parent, Some(one));
    assert_eq!(read(&env, "a.txt"), folder);
    assert!(journal.exists());
    // The journal now belongs to no switch of this project, and status says so.
    let out = env.bolt().arg("status").output().unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    let said = text(&out.stderr);
    assert!(
        said.contains("was made for another save than the one the project is on"),
        "{said}"
    );
}

#[cfg(feature = "fault-injection")]
#[test]
fn an_undo_to_picking_up_a_lost_head_stopped_anywhere_leaves_it_lost_or_picked_up() {
    let (template, [_, one, _]) = project();
    write(&template, "a.txt", "edited\n");
    fs::remove_file(head_file(&template)).unwrap();
    let to = one.to_hex()[..8].to_owned();
    for n in 1.. {
        assert!(n < 50, "the pick-up never finished");
        let env = Env::empty();
        support::copy_dir(template.root(), env.root());
        let out = env
            .bolt()
            .args(["undo", "--to", &to])
            .env("BOLT_FAULT_AT", n.to_string())
            .output()
            .unwrap();
        let crashed = text(&out.stderr).contains("injected crash");
        assert_eq!(read(&env, "a.txt"), b"edited\n", "fault {n}");
        let report = bolt_core::verify::verify_store(&env.root().join(".bolt/store/packs"));
        assert!(report.is_ok(), "fault {n}: {:?}", report.problems);
        // Either the step is current, or the head is still lost and running it again picks it up.
        if !head_file(&env).exists() {
            assert!(crashed, "fault {n}");
            ok(&env, &["undo", "--to", &to]);
        }
        let (_, now) = head(&env);
        assert_eq!(
            (now.parent, now.command.as_str()),
            (Some(one), "undo"),
            "fault {n}"
        );
        if !crashed {
            assert!(n > 1, "no fault point was reached");
            assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
            break;
        }
    }
}
