//! Hostile diaries written out (security.md T26): each case plants diary records, and the saves
//! they name, as a hostile `.bolt` could hold them, makes them current through `state/head`, runs
//! `bolt undo`, `redo`, `unshelf` or `undo --to`, and checks what Bolt said, what was written, and
//! that nothing reached the folder beside the project or went into `.git` or `.bolt`.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::fmt::Debug;

use bolt_core::diary::{Action, DiaryEntry, FolderChange, Position, State};
use bolt_core::id::Id;
use bolt_core::listing::EntryKind;
use bolt_core::pack::PackSet;
use bolt_core::repo::Repository;
use support::corpus::{
    Case, PAYLOAD, Seen, file, ignores_case, kinds, message, no_link_right_here, project_entries,
    seen, sorted, strings,
};
use support::{add_save, holding_bolt_spelling, listings, text};

use EntryKind::{Executable, File, Symlink};

/// A save no real folder holds: spellings of `.git`, links whose targets climb out with `..`, a
/// link named like a folder with a file below it, names Windows can't hold; and one plain file,
/// with a link to it that must stay a link.
const HOSTILE: &[(&str, EntryKind, &[u8])] = &[
    (".git/hooks/post-checkout", Executable, PAYLOAD),
    (".GIT/hooks/pwned", File, PAYLOAD),
    ("git~1/pwned", File, PAYLOAD),
    ("sub/.Git/hooks/pwned", File, PAYLOAD),
    ("up", Symlink, b"../pwned"),
    ("deep/up", Symlink, b"../../pwned"),
    ("deep/through", Symlink, b"../ok.txt/../../pwned"),
    ("dot-dot", Symlink, b".."),
    // CVE-2021-21300: where letter case is ignored, `a/post-checkout` would go through `A`.
    ("A", Symlink, b".git/hooks"),
    ("a/post-checkout", Executable, PAYLOAD),
    ("CON", File, PAYLOAD),
    ("aux/pwned", File, PAYLOAD),
    ("C:pwned", File, PAYLOAD),
    ("a\\b/pwned", File, PAYLOAD),
    ("ok.txt", File, b"fine"),
    ("near", Symlink, b"ok.txt"),
];

fn hostile(pack: &mut PackSet) -> Id {
    listings(pack, HOSTILE)
}

/// A top folder holding `.BOLT/pwned`.
fn bolt_spelling(pack: &mut PackSet) -> Id {
    let inside = listings(pack, &[("pwned", File, PAYLOAD)]);
    holding_bolt_spelling(pack, inside)
}

/// A save the store doesn't hold.
const MISSING: Id = Id([0x77; 32]);

/// The save planted records lead a command to: one holding what the function stores, made on the
/// save the project is on, or one the store doesn't hold.
#[derive(Clone, Copy)]
enum Named {
    Save(fn(&mut PackSet) -> Id),
    Missing,
}

/// The commands a diary leads to a save.
const COMMANDS: [&str; 4] = ["undo", "redo", "unshelf", "undo --to"];

/// What [`lead_to`] planted.
struct Lead {
    /// The command's arguments.
    args: Vec<String>,
    /// The save it leads to.
    save: Id,
    /// The entry naming that save: the step to undo or redo, the shelf's entry, or the entry to
    /// go back to.
    entry: Id,
}

/// Plants the records that lead `command` to the save `named`, as each command finds the save it
/// writes: an undo writes back what its step's folder change came from, a redo what it went to,
/// an unshelf the shelf's save, and an `undo --to` the save of an entry neither undo nor redo can
/// reach. Only the folder change names it for an undo or redo, and only the state otherwise.
fn lead_to(case: &Case, command: &str, named: Named) -> Lead {
    let before = case.env.state().1;
    let on = before.lines["main"];
    let mut save = MISSING;
    let mut args = vec![command.to_owned()];
    let entry = match command {
        "undo" => plant(case, |repo, pack, head, now| {
            save = named_save(repo, pack, named, on);
            step(head, now, "goto", Some(FolderChange { from: save, to: on }))
        }),
        "redo" => {
            let done = plant(case, |repo, pack, head, now| {
                save = named_save(repo, pack, named, on);
                step(head, now, "goto", Some(FolderChange { from: on, to: save }))
            });
            plant(case, |_, _, head, now| undone(head, now, done, &before));
            done
        }
        "unshelf" => plant(case, |repo, pack, head, now| {
            save = named_save(repo, pack, named, on);
            let mut made = step(head, now, "shelf", None);
            made.state.shelves.insert(0, ("1".into(), save));
            made
        }),
        "undo --to" => {
            let target = plant(case, |repo, pack, head, now| {
                save = named_save(repo, pack, named, on);
                let mut made = step(head, now, "goto", None);
                made.state.position = Position::AtSave(save);
                made
            });
            args = left_behind(case, target, &before);
            target
        }
        other => panic!("{other}"),
    };
    Lead { args, save, entry }
}

/// Undoes the step `target`, then clears it with a later command, so that only a jump of
/// `undo --to` reaches it; that command's arguments.
fn left_behind(case: &Case, target: Id, before: &State) -> Vec<String> {
    plant(case, |_, _, head, now| undone(head, now, target, before));
    plant(case, |_, _, head, now| step(head, now, "save", None));
    let to = target.to_hex()[..12].to_owned();
    vec!["undo".into(), "--to".into(), to]
}

/// The save `named` stands for, stored in `pack` if it is one.
fn named_save(repo: &Repository, pack: &mut PackSet, named: Named, on: Id) -> Id {
    match named {
        Named::Save(folder) => {
            let top = folder(pack);
            add_save(repo, pack, top, on)
        }
        Named::Missing => MISSING,
    }
}

/// Stores what `make` puts in a pack and the diary entry it gives, made after the current one,
/// through Bolt's own library; then makes that entry current as a hostile `.bolt` could, by
/// writing `state/head`. The entry's ID.
fn plant(
    case: &Case,
    make: impl FnOnce(&Repository, &mut PackSet, Id, DiaryEntry) -> DiaryEntry,
) -> Id {
    let mut repo = Repository::open(case.root()).unwrap();
    let lock = repo.lock_for_change("test", &mut |_| {}).unwrap();
    let (head, now) = repo.current().unwrap();
    let mut pack = repo.new_packs().unwrap();
    let entry = make(&repo, &mut pack, head, now);
    let id = repo.store_entry(&lock, pack, &entry).unwrap();
    std::fs::write(case.root().join(".bolt/state/head"), format!("{id}\n")).unwrap();
    id
}

/// A command's entry after `head`, with the state of `now`, the entry `head` names.
fn step(
    head: Id,
    now: DiaryEntry,
    command: &str,
    folder_change: Option<FolderChange>,
) -> DiaryEntry {
    DiaryEntry {
        parent: Some(head),
        action: Action::Command,
        command: command.into(),
        folder_change,
        ..now
    }
}

/// The entry after `head` undoing the step `done`, which `now` is, putting back the state
/// `before` it.
fn undone(head: Id, now: DiaryEntry, done: Id, before: &State) -> DiaryEntry {
    let back = now.folder_change.map(|c| FolderChange {
        from: c.to,
        to: c.from,
    });
    DiaryEntry {
        parent: Some(head),
        action: Action::Undo(done),
        command: "undo".into(),
        state: before.clone(),
        folder_change: back,
        ..now
    }
}

/// `bolt <args> --json`: its exit code and its document.
fn run(case: &Case, args: &[impl AsRef<OsStr>]) -> (Option<i32>, serde_json::Value) {
    let out = case.env.bolt().args(args).arg("--json").output().unwrap();
    let doc = serde_json::from_slice(&out.stdout)
        .unwrap_or_else(|e| panic!("{e}: {}{}", text(&out.stdout), text(&out.stderr)));
    (out.status.code(), doc)
}

/// Runs `bolt <args> --json`, which must fail having changed nothing; its document.
fn refused(case: &Case, args: &[impl AsRef<OsStr> + Debug]) -> serde_json::Value {
    let before = bolt_folder(case);
    let (code, doc) = run(case, args);
    assert_eq!(code, Some(1), "{args:?}: {doc}");
    let changed = differing(&before, &bolt_folder(case));
    assert!(changed.is_empty(), "{args:?} changed .bolt: {changed:?}");
    assert!(project_entries(&case.env).is_empty(), "{args:?}");
    case.assert_nothing_escaped();
    doc
}

/// Everything in `.bolt` but `cache`, which a command may write even when it changes nothing, and
/// `state/lock-info`.
fn bolt_folder(case: &Case) -> BTreeMap<String, Seen> {
    let mut all = seen(&case.root().join(".bolt"));
    // Only informational, and rewritten on taking the change lock, before a refusal is known.
    all.retain(|path, _| path.split('/').next() != Some("cache") && path != "state/lock-info");
    all
}

/// The paths where `a` and `b` differ.
fn differing(a: &BTreeMap<String, Seen>, b: &BTreeMap<String, Seen>) -> Vec<String> {
    let paths: BTreeSet<&String> = a.keys().chain(b.keys()).collect();
    let differ = paths.into_iter().filter(|p| a.get(*p) != b.get(*p));
    differ.cloned().collect()
}

/// What writing `HOSTILE` leaves on this system, as `goto` writes it: the project's entries, the
/// paths left unwritten, and the links written as files holding their targets.
fn hostile_written(case: &Case) -> (BTreeMap<String, Seen>, Vec<String>, Vec<String>) {
    let mut entries: BTreeMap<String, Seen> = [
        ("ok.txt", file(b"fine")),
        ("sub", Seen::Folder),
        ("deep", Seen::Folder),
        ("up", file(b"../pwned")),
        ("deep/up", file(b"../../pwned")),
        ("deep/through", file(b"../ok.txt/../../pwned")),
        ("dot-dot", file(b"..")),
    ]
    .into_iter()
    .map(|(path, seen)| (path.to_owned(), seen))
    .collect();
    let mut not_written = vec![".git", ".GIT", "git~1", "sub/.Git"];
    let mut placeholders = vec!["up", "deep/up", "deep/through", "dot-dot"];
    if ignores_case(case.root()) {
        not_written.extend(["A", "a"]);
    } else {
        // Its target goes through `.git`, a name never written.
        placeholders.push("A");
        entries.insert("A".into(), file(b".git/hooks"));
        entries.insert("a".into(), Seen::Folder);
        entries.insert("a/post-checkout".into(), file(PAYLOAD));
    }
    if cfg!(windows) {
        not_written.extend(["CON", "aux", "C:pwned", "a\\b"]);
    } else {
        // Elsewhere they are ordinary names.
        for path in ["CON", "aux/pwned", "C:pwned", "a\\b/pwned"] {
            entries.insert(path.into(), file(PAYLOAD));
        }
        entries.insert("aux".into(), Seen::Folder);
        entries.insert("a\\b".into(), Seen::Folder);
    }
    let near = std::fs::symlink_metadata(case.root().join("near")).unwrap();
    if near.file_type().is_symlink() {
        entries.insert("near".into(), Seen::Link("ok.txt".into()));
    } else {
        no_link_right_here("near");
        entries.insert("near".into(), file(b"ok.txt"));
        placeholders.push("near");
    }
    (entries, sorted(&not_written), sorted(&placeholders))
}

/// Fails unless the command left `HOSTILE` written as `goto` writes it, `written` files and links
/// in all, said so, and wrote nothing elsewhere. Gives the paths left unwritten and the links
/// written as files.
fn assert_written_as_goto_would(
    case: &Case,
    doc: &serde_json::Value,
    written: &serde_json::Value,
) -> (Vec<String>, Vec<String>) {
    let (entries, not_written, placeholders) = hostile_written(case);
    assert_eq!(project_entries(&case.env), entries);
    let files = entries.values().filter(|e| !matches!(e, Seen::Folder));
    assert_eq!(written.as_u64(), Some(files.count() as u64), "{doc}");
    assert_eq!(
        kinds(doc),
        ["not_written_here", "placeholders_written"],
        "{doc}"
    );
    let said = message(doc, "not_written_here");
    let left = format!("Left {} entries unwritten", not_written.len());
    assert!(
        said.starts_with(&left) && said.contains("a spelling of .git or .bolt"),
        "{said}"
    );
    let said = message(doc, "placeholders_written");
    let wrote = format!("Wrote {} links as plain files", placeholders.len());
    assert!(
        said.starts_with(&wrote) && said.contains("they lead out of the project"),
        "{said}"
    );
    case.assert_nothing_escaped();
    (not_written, placeholders)
}

/// The save a step's `files` effect says it switched the folder to.
fn switched_to(step: &serde_json::Value) -> &serde_json::Value {
    let effects = step["effects"].as_array().unwrap();
    let files = effects.iter().find(|e| e["kind"] == "files");
    &files.unwrap_or_else(|| panic!("no files effect: {step}"))["to"]
}

/// The hint of every refusal for damage.
const DAMAGED: &str = "Nothing was changed. Ask for help before trying to repair it.";

/// Fails unless `doc` refuses the step of diary entry `step` because a save it names is missing.
fn assert_step_gone(doc: &serde_json::Value, step: Id) {
    assert_eq!(
        doc["error"]["message"],
        format!(
            "The step of diary entry {} can't be undone or redone: what it changed can't be read \
             (object {MISSING} is not in the store).",
            step.short_hex()
        ),
        "{doc}"
    );
    assert_eq!(
        doc["error"]["hint"],
        "bolt tidy removes what steps older than the undo window changed, and a damaged store \
         can lose it too, so undo and redo can't go past this step. To get back to an older save, \
         find it with bolt history and go to it with bolt goto.",
        "{doc}"
    );
}

// Saves a diary leads to, written out (T1 to T5, through T26).

#[test]
fn undo_writes_back_a_hostile_save_only_as_goto_would() {
    let case = Case::new();
    let lead = lead_to(&case, "undo", Named::Save(hostile));
    let (code, doc) = run(&case, &lead.args);
    assert_eq!(code, Some(0), "{doc}");
    let step = &doc["undone"][0];
    assert_eq!(step["entry"], lead.entry.to_hex(), "{doc}");
    assert_eq!(step["action"], "undo", "{doc}");
    assert_eq!(switched_to(step), &lead.save.to_hex());
    assert_written_as_goto_would(&case, &doc, &step["written"]);
}

#[test]
fn redo_writes_a_hostile_save_only_as_goto_would() {
    let case = Case::new();
    let lead = lead_to(&case, "redo", Named::Save(hostile));
    let (code, doc) = run(&case, &lead.args);
    assert_eq!(code, Some(0), "{doc}");
    let step = &doc["redone"][0];
    assert_eq!(step["entry"], lead.entry.to_hex(), "{doc}");
    assert_eq!(step["action"], "redo", "{doc}");
    assert_eq!(switched_to(step), &lead.save.to_hex());
    assert_written_as_goto_would(&case, &doc, &step["written"]);
}

#[test]
fn unshelf_writes_a_hostile_shelf_only_as_goto_would() {
    let case = Case::new();
    let lead = lead_to(&case, "unshelf", Named::Save(hostile));
    let (code, doc) = run(&case, &lead.args);
    assert_eq!(code, Some(0), "{doc}");
    assert_eq!(doc["shelf"]["label"], "1", "{doc}");
    assert_eq!(doc["shelf"]["save"], lead.save.to_hex(), "{doc}");
    let (not_written, placeholders) = assert_written_as_goto_would(&case, &doc, &doc["written"]);
    assert_eq!(strings(&doc["not_written"]), not_written, "{doc}");
    assert_eq!(strings(&doc["placeholders"]), placeholders, "{doc}");
}

#[test]
fn undo_to_an_entry_whose_state_names_a_hostile_save_writes_it_only_as_goto_would() {
    let case = Case::new();
    let lead = lead_to(&case, "undo --to", Named::Save(hostile));
    let (code, doc) = run(&case, &lead.args);
    assert_eq!(code, Some(0), "{doc}");
    let step = &doc["steps"][0];
    assert_eq!(step["entry"], lead.entry.to_hex(), "{doc}");
    assert_eq!(step["action"], "jump", "{doc}");
    assert_written_as_goto_would(&case, &doc, &step["written"]);
}

#[test]
fn a_save_holding_a_spelling_of_bolt_is_refused_whole_by_every_command() {
    for command in COMMANDS {
        let case = Case::new();
        let lead = lead_to(&case, command, Named::Save(bolt_spelling));
        let doc = refused(&case, &lead.args);
        assert_eq!(
            doc["error"]["message"],
            format!(
                "Save {} holds a name spelled like Bolt's own folder (.bolt), which Bolt never \
                 writes, so it can't be used.",
                lead.save.short_hex()
            ),
            "{command}: {doc}"
        );
        assert_eq!(
            doc["error"]["hint"],
            "Bolt never makes such a save, so another program put it there. Nothing was \
             changed: go on from another save, and ask for help before trying to repair this \
             one.",
            "{command}: {doc}"
        );
    }
}

// Records no command may follow (T26).

#[test]
fn an_undo_whose_target_is_no_command_entry_is_refused() {
    // An undo of an undo, which is no step, and an undo of a save, which is no entry at all.
    for of_a_save in [false, true] {
        let case = Case::new();
        let save = case.env.state().1.lines["main"];
        let first = plant(&case, |_, _, head, now| {
            let mut made = step(head, now, "undo", None);
            made.action = Action::Undo(head);
            made
        });
        let target = if of_a_save { save } else { first };
        let bad = plant(&case, |_, _, head, now| {
            let mut made = step(head, now, "undo", None);
            made.action = Action::Undo(target);
            made
        });
        let why = if of_a_save {
            format!("the diary has no entry {save}")
        } else {
            format!("diary entry {bad} undoes or redoes {first}, which is not a step")
        };
        let to = first.to_hex()[..12].to_owned();
        for args in [vec!["undo"], vec!["redo"], vec!["undo", "--to", &to]] {
            let doc = refused(&case, &args[..]);
            assert_eq!(
                doc["error"]["message"],
                format!("This project's Bolt data looks damaged: {why}."),
                "{args:?}: {doc}"
            );
            assert_eq!(doc["error"]["hint"], DAMAGED, "{args:?}: {doc}");
        }
    }
}

#[test]
fn a_state_naming_a_save_the_store_lacks_is_refused() {
    let gone = |mut entry: DiaryEntry| {
        entry.state.lines.insert("gone".into(), MISSING);
        entry
    };
    // Undo puts back the state before its step.
    let case = Case::new();
    plant(&case, |_, _, head, now| gone(step(head, now, "save", None)));
    let done = plant(&case, |_, _, head, now| {
        let mut made = step(head, now, "save", None);
        made.state.lines.remove("gone");
        made
    });
    let doc = refused(&case, &["undo"]);
    assert_step_gone(&doc, done);
    // Redo puts back the step's own.
    let case = Case::new();
    let before = case.env.state().1;
    let done = plant(&case, |_, _, head, now| gone(step(head, now, "save", None)));
    plant(&case, |_, _, head, now| undone(head, now, done, &before));
    let doc = refused(&case, &["redo"]);
    assert_step_gone(&doc, done);
    // A jump puts back the state of the entry it goes to.
    let case = Case::new();
    let before = case.env.state().1;
    let target = plant(&case, |_, _, head, now| gone(step(head, now, "save", None)));
    let doc = refused(&case, &left_behind(&case, target, &before));
    assert_eq!(
        doc["error"]["message"],
        format!(
            "Things can't be made as they were right after diary entry {}: its saves can't be \
             read (object {MISSING} is not in the store).",
            target.short_hex()
        ),
        "{doc}"
    );
    assert_eq!(
        doc["error"]["hint"],
        "bolt tidy removes the saves only entries older than the undo window needed, and a \
         damaged store can lose them too, so undo --to can't go back to this entry. To get back \
         to an older save, find it with bolt history and go to it with bolt goto.",
        "{doc}"
    );
    // Unshelf brings back what a shelf names.
    let case = Case::new();
    let shelf = lead_to(&case, "unshelf", Named::Missing);
    let doc = refused(&case, &shelf.args);
    assert_eq!(
        doc["error"]["message"],
        format!(
            "This project's Bolt data looks damaged: shelf 1 names save {MISSING}, which can't \
             be read."
        ),
        "{doc}"
    );
    assert_eq!(doc["error"]["hint"], DAMAGED, "{doc}");
}

#[test]
fn a_folder_change_naming_a_save_the_store_lacks_is_refused() {
    for command in ["undo", "redo"] {
        let case = Case::new();
        let lead = lead_to(&case, command, Named::Missing);
        let doc = refused(&case, &lead.args);
        assert_step_gone(&doc, lead.entry);
    }
}
