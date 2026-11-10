#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use super::*;
use crate::chunks::{Chunk, ChunkList, set_test_cut_sizes};
use crate::diary::{Action, DiaryEntry, FolderChange, Position, ServerLines, State, Tag, TagList};
use crate::history::tip_of;
use crate::id::{ChangeId, hash_content};
use crate::listing::{Entry, EntryKind};
use crate::pack::{ObjectKind, PackReader, PackSet};
use crate::save::{Person, Save};
use crate::snapshot::{SaveOutcome, SaveRequest, save_folder};
use crate::store::Store;
use crate::switch::{SwitchError, discard};
use crate::test_support::{
    add_link, add_listing, commit_records, lock_pack, noise, plant_damaged_copies, plant_root,
    plant_save,
};
use crate::undo::{self, StepRequest};
use crate::warning;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const DAY: i64 = 86_400_000;

fn now() -> i64 {
    let since = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
    i64::try_from(since.as_millis()).unwrap()
}

fn days_ago(days: i64) -> Person {
    Person {
        name: "Ada".into(),
        email: "ada@example.com".into(),
        time_ms: now() - days * DAY,
        tz_offset_min: 0,
    }
}

/// A project started `days` ago.
fn project(days: i64) -> (tempfile::TempDir, Repository) {
    let dir = tempfile::tempdir().unwrap();
    let repo = Repository::init(dir.path(), &days_ago(days)).unwrap();
    (dir, repo)
}

fn tip(repo: &Repository) -> Id {
    tip_of(&repo.current().unwrap().1).unwrap()
}

fn saved(repo: &mut Repository, days: i64, amend: bool) -> Id {
    let who = days_ago(days);
    let req = SaveRequest {
        message: (!amend).then_some("Change"),
        who: &who,
        command: "save",
        accept_crlf: false,
        git_excludes: None,
        new_line: None,
        only: None,
        amend,
    };
    match save_folder(repo, &req, &mut |_| {}, &mut |_| true).unwrap() {
        SaveOutcome::Saved { save_id, .. } => save_id,
        SaveOutcome::NothingChanged { .. } => panic!("nothing was saved"),
    }
}

/// Writes `text` into `a.txt` and saves it, `days` ago.
fn save_text(dir: &Path, repo: &mut Repository, text: &str, days: i64) -> Id {
    fs::write(dir.join("a.txt"), text).unwrap();
    saved(repo, days, false)
}

/// Writes `bytes` into `big.bin` and saves it, `days` ago, cutting files from 64 KiB on so that a
/// big file stays small here. Gives the file's ID.
fn save_big(dir: &Path, repo: &mut Repository, bytes: &[u8], days: i64) -> Id {
    set_test_cut_sizes(Some((64 << 10, 1 << 30)));
    fs::write(dir.join("big.bin"), bytes).unwrap();
    saved(repo, days, false);
    hash_content(bytes)
}

/// Two versions of a big file that share most chunks: the dropped one is saved first, so only its
/// pack holds the chunks they share, then undone long ago. Gives the kept file's ID and the
/// dropped one's, and the chunks only the dropped one names.
fn two_versions(dir: &Path, repo: &mut Repository) -> (Id, Id, Vec<Id>) {
    let kept = noise(1, 512 << 10);
    let mut dropped = kept.clone();
    dropped[256 << 10..(256 << 10) + 6].copy_from_slice(b"edited");
    let dropped = save_big(dir, repo, &dropped, 45);
    step(repo, 45, false).unwrap();
    let kept = save_big(dir, repo, &kept, 40);
    let keeps = chunks_of(repo, &kept);
    let drops = chunks_of(repo, &dropped);
    let own: Vec<Id> = drops.difference(&keeps).copied().collect();
    assert!(!own.is_empty() && own.len() < drops.len(), "{own:?}");
    let shared_in = holding(repo, &dropped);
    assert_eq!(shared_in.len(), 1);
    for id in keeps.intersection(&drops) {
        assert_eq!(holding(repo, id), shared_in, "{id}");
    }
    (kept, dropped, own)
}

/// The packs that hold `id`, by name.
fn holding(repo: &Repository, id: &Id) -> Vec<String> {
    let holds = |name: &String| {
        let pack = PackReader::open(&repo.packs_dir().join(name)).unwrap();
        pack.find(id).unwrap().is_some()
    };
    pack_names(repo).into_iter().filter(holds).collect()
}

/// Stores `list` again in a pack of its own, as a command a crash stopped can leave it.
fn store_again(repo: &mut Repository, list: &ChunkList) {
    let lock = repo.lock_for_change("test", &mut |_| {}).unwrap();
    let mut packs = repo.new_packs().unwrap();
    packs.add_chunk_list(list).unwrap();
    repo.store_packs(&lock, packs).unwrap();
}

/// Every chunk list stored under `file`, from every pack.
fn lists_of(repo: &Repository, file: &Id) -> Vec<ChunkList> {
    let mut lists = Vec::new();
    for name in pack_names(repo) {
        let pack = PackReader::open(&repo.packs_dir().join(name)).unwrap();
        if let Some(e) = pack.find(file).unwrap()
            && e.kind == ObjectKind::ChunkList
        {
            lists.push(pack.read_chunk_list(&e).unwrap());
        }
    }
    lists
}

/// The chunks the lists of `file` name.
fn chunks_of(repo: &Repository, file: &Id) -> BTreeSet<Id> {
    let lists = lists_of(repo, file).into_iter();
    lists.flat_map(|l| l.chunks).map(|c| c.id).collect()
}

/// Undoes one step, or redoes one, `days` ago.
fn step(repo: &mut Repository, days: i64, redo: bool) -> Result<(), SwitchError> {
    let who = days_ago(days);
    let req = StepRequest {
        n: 1,
        shelve: false,
        who: &who,
        git_excludes: None,
    };
    let done = if redo {
        undo::redo(repo, &req, &mut |_| {})
    } else {
        undo::undo(repo, &req, &mut |_| {})
    };
    let done = done.map_err(|e| *e.error)?;
    assert_eq!(done.taken.len(), 1, "no step was taken");
    Ok(())
}

/// Writes `text` into `a.txt` and throws it away, `days` ago; gives the save that kept it.
fn discard_text(dir: &Path, repo: &mut Repository, text: &str, days: i64) -> Id {
    fs::write(dir.join("a.txt"), text).unwrap();
    discard(repo, None, &days_ago(days), "discard", None, &mut |_| {}).unwrap();
    repo.current().unwrap().1.folder_change.unwrap().from
}

/// Sets the modified time of the pack, or folder, at `path` `hours` back.
fn set_age(path: &Path, hours: u64) {
    // A folder opens on Windows only with FILE_FLAG_BACKUP_SEMANTICS, and elsewhere only to read,
    // which lets its owner change its times.
    #[cfg(windows)]
    let file = {
        use std::os::windows::fs::OpenOptionsExt;
        let mut options = fs::File::options();
        options.write(true).custom_flags(0x0200_0000);
        options.open(path).unwrap()
    };
    #[cfg(not(windows))]
    let file = fs::File::open(path).unwrap();
    let then = SystemTime::now() - Duration::from_secs(hours * 3600);
    file.set_modified(then).unwrap();
}

/// Makes every pack `hours` old, so that `tidy` may change it.
fn age_packs(repo: &Repository, hours: u64) {
    repo.store().close_all();
    for name in pack_names(repo) {
        set_age(&repo.packs_dir().join(name), hours);
    }
}

fn pack_names(repo: &Repository) -> BTreeSet<String> {
    fs::read_dir(repo.packs_dir())
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .filter(|name| name.ends_with(".pack"))
        .collect()
}

fn tidied(repo: &mut Repository) -> TidyReport {
    tidy(repo, 30, now(), &mut |_| {}, &mut |_| {}).unwrap()
}

/// Whether a store opened afresh reads `id` back as a `kind`; a file stored as chunks is read
/// whole, each chunk checked.
fn reads(repo: &Repository, id: &Id, kind: ObjectKind) -> bool {
    let store = Store::open(repo.packs_dir()).unwrap();
    match kind {
        ObjectKind::Content | ObjectKind::ChunkList => {
            store.copy_content(id, &mut Vec::new()).is_ok()
        }
        _ => store.read_record(id, kind).is_ok(),
    }
}

fn text_reads(repo: &Repository, text: &str) -> bool {
    reads(repo, &hash_content(text.as_bytes()), ObjectKind::Content)
}

/// Stores a save of `root` on `parents`, made long ago.
fn a_save(packs: &mut PackSet, root: Id, parents: Vec<Id>, message: &str) -> Id {
    let who = days_ago(100);
    let save = Save {
        root,
        parents,
        author: who.clone(),
        saver: who,
        message: message.into(),
        change_id: ChangeId::random().unwrap(),
        move_notes: vec![],
        signature: vec![],
        extra: BTreeMap::new(),
    };
    packs.add_record(ObjectKind::Save, &save.encode()).unwrap()
}

/// Commits an entry after the current one, as `change` leaves a copy of it.
fn commit(repo: &mut Repository, change: impl FnOnce(&mut DiaryEntry)) -> Id {
    let lock = repo.lock_for_change("test", &mut |_| {}).unwrap();
    let (head, mut entry) = repo.current().unwrap();
    entry.parent = Some(head);
    entry.action = Action::Command;
    entry.command = "test".into();
    entry.folder_change = None;
    change(&mut entry);
    let packs = repo.new_packs().unwrap();
    repo.commit(&lock, packs, &entry).unwrap()
}

/// Commits an entry after the current one, made `days` ago.
fn append(
    repo: &mut Repository,
    days: i64,
    action: Action,
    state: State,
    folder_change: Option<FolderChange>,
) -> Id {
    commit(repo, |e| {
        e.who = days_ago(days);
        e.action = action;
        e.state = state;
        e.folder_change = folder_change;
    })
}

/// Stores an entry after the current one, holding a save of its own, without moving `head`, as a
/// command a crash stopped leaves them. Gives the entry and the save.
fn crash_leftover(repo: &mut Repository) -> (Id, Id) {
    let lock = repo.lock_for_change("test", &mut |_| {}).unwrap();
    let (head, mut entry) = repo.current().unwrap();
    let mut packs = repo.new_packs().unwrap();
    let text = b"left by a crash";
    let content = packs.add_content(text).unwrap();
    let file = Entry {
        name: "crash.txt".into(),
        kind: EntryKind::File,
        id: content,
        size: text.len() as u64,
    };
    let (root, _) = add_listing(&mut packs, vec![file]);
    let save = a_save(&mut packs, root, vec![tip_of(&entry).unwrap()], "Crashed");
    entry.parent = Some(head);
    entry.command = "save".into();
    entry.folder_change = None;
    entry.state.lines.insert("main".into(), save);
    let id = repo.store_entry(&lock, packs, &entry).unwrap();
    (id, save)
}

#[test]
fn tidy_keeps_every_line_tag_server_line_and_shelf_with_all_their_history() {
    let (_dir, mut repo) = project(100);
    let first = tip(&repo);
    plant_save(&mut repo, &[("a.txt", EntryKind::File, b"one")]);
    let one = tip(&repo);
    plant_save(&mut repo, &[("a.txt", EntryKind::File, b"two")]);
    let main = tip(&repo);
    // Two saves in a row, then `main` back where it was: only what names the second reaches them.
    let off_main = |repo: &mut Repository, what: &str| -> [Id; 2] {
        let mut made = [main; 2];
        for (i, at) in made.iter_mut().enumerate() {
            let text = format!("{what} {i}");
            plant_save(repo, &[("a.txt", EntryKind::File, text.as_bytes())]);
            *at = tip(repo);
        }
        commit(repo, |e| {
            e.state.lines.insert("main".into(), main);
        });
        made
    };
    let shelved = off_main(&mut repo, "shelved");
    let tagged = off_main(&mut repo, "tagged");
    let served = off_main(&mut repo, "served");
    let other = off_main(&mut repo, "other line");
    let undone = off_main(&mut repo, "undone");
    let tag = Tag {
        save: tagged[1],
        message: "First playable".into(),
        git_tags: vec![],
    };
    let tags = TagList {
        tags: BTreeMap::from([("v1".to_owned(), tag)]),
    };
    let servers = ServerLines {
        servers: BTreeMap::from([(
            "origin".to_owned(),
            BTreeMap::from([("main".to_owned(), served[1])]),
        )]),
    };
    let (tag_list, server_lines) = commit_records(&mut repo, &tags, &servers);
    commit(&mut repo, |e| {
        e.state.shelves = vec![("1".into(), shelved[1])];
        e.state.lines.insert("other".into(), other[1]);
    });
    // Every entry is older than the window, so only the current state counts.
    age_packs(&repo, 48);
    let report = tidied(&mut repo);
    assert!(report.packs_removed > 0, "{report:?}");
    let saves = [
        first, one, main, shelved[0], shelved[1], tagged[0], tagged[1],
    ]
    .into_iter()
    .chain([served[0], served[1], other[0], other[1]]);
    for id in saves {
        assert!(reads(&repo, &id, ObjectKind::Save), "save {id}");
    }
    assert!(reads(&repo, &tag_list, ObjectKind::TagList));
    assert!(reads(&repo, &server_lines, ObjectKind::ServerLines));
    for what in ["shelved", "tagged", "served", "other line"] {
        for i in 0..2 {
            assert!(text_reads(&repo, &format!("{what} {i}")), "{what} {i}");
        }
    }
    assert!(text_reads(&repo, "one") && text_reads(&repo, "two"));
    for id in undone {
        assert!(!reads(&repo, &id, ObjectKind::Save), "undone save {id}");
    }
    assert!(!text_reads(&repo, "undone 0") && !text_reads(&repo, "undone 1"));
}

#[test]
fn tidy_keeps_what_the_window_reaches_and_removes_older() {
    let (dir, mut repo) = project(50);
    let one = save_text(dir.path(), &mut repo, "one", 45);
    // Forty days ago: a save undone, and work thrown away.
    let undone = save_text(dir.path(), &mut repo, "two", 40);
    step(&mut repo, 40, false).unwrap();
    let thrown = discard_text(dir.path(), &mut repo, "thrown away", 40);
    // Two days ago, the same again.
    let lately = save_text(dir.path(), &mut repo, "three", 2);
    step(&mut repo, 2, false).unwrap();
    let thrown_lately = discard_text(dir.path(), &mut repo, "thrown away lately", 2);
    age_packs(&repo, 48);
    tidied(&mut repo);
    for id in [one, lately, thrown_lately] {
        assert!(reads(&repo, &id, ObjectKind::Save), "save {id}");
    }
    assert!(text_reads(&repo, "three") && text_reads(&repo, "thrown away lately"));
    for id in [undone, thrown] {
        assert!(!reads(&repo, &id, ObjectKind::Save), "save {id}");
    }
    assert!(!text_reads(&repo, "two") && !text_reads(&repo, "thrown away"));
    // The recent discard can still be undone: its work comes back.
    step(&mut repo, 0, false).unwrap();
    let a = fs::read_to_string(dir.path().join("a.txt")).unwrap();
    assert_eq!(a, "thrown away lately");
    // The old one can't: what it kept is gone, and undo says so.
    match step(&mut repo, 0, false) {
        Err(SwitchError::StepGone { .. }) => {}
        other => panic!("{other:?}"),
    }
}

#[test]
fn an_amend_made_after_a_month_away_can_still_be_undone_after_tidy() {
    let (dir, mut repo) = project(50);
    let replaced = save_text(dir.path(), &mut repo, "one", 40);
    fs::write(dir.path().join("a.txt"), "one, amended").unwrap();
    let amended = saved(&mut repo, 1, true);
    assert_ne!(replaced, amended);
    age_packs(&repo, 48);
    tidied(&mut repo);
    // Only the state before the amend reaches the save it replaced.
    assert!(reads(&repo, &replaced, ObjectKind::Save));
    step(&mut repo, 0, false).unwrap();
    assert_eq!(repo.current().unwrap().1.state.lines["main"], replaced);
}

#[test]
fn an_undo_whose_state_was_tidied_away_stops_and_changes_nothing() {
    let (dir, mut repo) = project(50);
    let replaced = save_text(dir.path(), &mut repo, "one", 45);
    fs::write(dir.path().join("a.txt"), "one, amended").unwrap();
    // Amended before the window: undoing it would need the save it replaced.
    saved(&mut repo, 40, true);
    age_packs(&repo, 48);
    tidied(&mut repo);
    assert!(!reads(&repo, &replaced, ObjectKind::Save));
    let head = repo.head().unwrap();
    match step(&mut repo, 0, false) {
        Err(SwitchError::StepGone { step, .. }) => assert_eq!(step, head),
        other => panic!("{other:?}"),
    }
    assert_eq!(repo.head().unwrap(), head);
}

#[test]
fn a_step_undone_within_the_window_can_be_redone_after_tidy_however_old() {
    let (dir, mut repo) = project(50);
    save_text(dir.path(), &mut repo, "one", 45);
    let old = save_text(dir.path(), &mut repo, "two", 40);
    step(&mut repo, 2, false).unwrap();
    age_packs(&repo, 48);
    tidied(&mut repo);
    assert!(reads(&repo, &old, ObjectKind::Save));
    step(&mut repo, 0, true).unwrap();
    assert_eq!(repo.current().unwrap().1.state.lines["main"], old);
}

#[test]
fn undo_to_an_entry_whose_saves_were_tidied_away_names_it() {
    let (dir, mut repo) = project(50);
    save_text(dir.path(), &mut repo, "one", 45);
    save_text(dir.path(), &mut repo, "two", 40);
    let entry = repo.head().unwrap();
    // Undone, then cleared from what redo can bring back by a new save.
    step(&mut repo, 40, false).unwrap();
    save_text(dir.path(), &mut repo, "three", 40);
    age_packs(&repo, 48);
    tidied(&mut repo);
    let head = repo.head().unwrap();
    let who = days_ago(0);
    let req = StepRequest {
        n: 1,
        shelve: false,
        who: &who,
        git_excludes: None,
    };
    let failed = undo::undo_to(&mut repo, &entry.to_hex()[..12], &req, &mut |_| {}).unwrap_err();
    assert!(failed.taken.is_empty());
    match &*failed.error {
        SwitchError::EntryGone { entry: named, .. } => assert_eq!(*named, entry),
        other => panic!("{other:?}"),
    }
    let said = failed.error.to_string();
    assert!(said.contains(&entry.short_hex()), "{said}");
    assert!(failed.error.hint().unwrap().contains("bolt tidy"));
    assert_eq!(repo.head().unwrap(), head);
    let a = fs::read_to_string(dir.path().join("a.txt")).unwrap();
    assert_eq!(a, "three");
}

#[test]
fn tidy_removes_entries_a_crash_left() {
    let (dir, mut repo) = project(50);
    save_text(dir.path(), &mut repo, "one", 40);
    let head = repo.head().unwrap();
    let (entry, save) = crash_leftover(&mut repo);
    assert!(reads(&repo, &entry, ObjectKind::DiaryEntry));
    age_packs(&repo, 48);
    tidied(&mut repo);
    assert!(!reads(&repo, &entry, ObjectKind::DiaryEntry));
    assert!(!reads(&repo, &save, ObjectKind::Save));
    assert!(!text_reads(&repo, "left by a crash"));
    assert_eq!(repo.current().unwrap().0, head);
    assert!(reads(&repo, &head, ObjectKind::DiaryEntry));
}

#[test]
fn tidy_keeps_a_diary_cut_off_from_head_while_its_end_is_in_the_window() {
    let (dir, mut repo) = project(50);
    save_text(dir.path(), &mut repo, "one", 45);
    let older = repo.head().unwrap();
    let old = save_text(dir.path(), &mut repo, "old work", 40);
    let recent = save_text(dir.path(), &mut repo, "recent work", 2);
    let end = repo.head().unwrap();
    // `head` put back, as a sync tool or a restore would.
    let head = dir.path().join(".bolt").join("state").join("head");
    fs::write(head, format!("{older}\n")).unwrap();
    assert_eq!(repo.head().unwrap(), older);
    age_packs(&repo, 48);
    tidied(&mut repo);
    // The whole diary back from the end is kept, with what its states reach.
    assert!(reads(&repo, &end, ObjectKind::DiaryEntry));
    assert!(reads(&repo, &recent, ObjectKind::Save) && reads(&repo, &old, ObjectKind::Save));
    assert!(text_reads(&repo, "recent work") && text_reads(&repo, "old work"));

    // Once the end is older than the window, it goes.
    let later = now() + 29 * DAY;
    tidy(&mut repo, 30, later, &mut |_| {}, &mut |_| {}).unwrap();
    assert!(!reads(&repo, &end, ObjectKind::DiaryEntry));
    assert!(!reads(&repo, &recent, ObjectKind::Save) && !reads(&repo, &old, ObjectKind::Save));
    assert!(!text_reads(&repo, "recent work"));
}

#[test]
fn tidy_leaves_packs_changed_in_the_last_day_untouched() {
    let (dir, mut repo) = project(50);
    save_text(dir.path(), &mut repo, "one", 40);
    age_packs(&repo, 48);
    let aged = pack_names(&repo);
    let (entry, save) = crash_leftover(&mut repo);
    let young: Vec<String> = pack_names(&repo).difference(&aged).cloned().collect();
    assert_eq!(young.len(), 1);
    let young = repo.packs_dir().join(&young[0]);
    repo.store().close_all();
    set_age(&young, 23);
    let bytes = fs::read(&young).unwrap();
    let report = tidied(&mut repo);
    assert_eq!(report.packs_young, 1, "{report:?}");
    assert_eq!(fs::read(&young).unwrap(), bytes);
    assert!(reads(&repo, &entry, ObjectKind::DiaryEntry));
    assert!(reads(&repo, &save, ObjectKind::Save));
    // A day later it goes.
    repo.store().close_all();
    set_age(&young, 25);
    tidied(&mut repo);
    assert!(!young.exists());
    assert!(!reads(&repo, &entry, ObjectKind::DiaryEntry));
}

#[test]
fn tidy_stops_before_removing_anything_when_something_to_keep_is_missing() {
    let (dir, mut repo) = project(50);
    save_text(dir.path(), &mut repo, "one", 40);
    let (entry, _) = crash_leftover(&mut repo);
    // A save on `main` whose top folder's listing isn't in the store.
    let missing = Id([9; 32]);
    plant_root(&mut repo, |_| missing);
    age_packs(&repo, 48);
    let before = pack_names(&repo);
    let mut told = None;
    let failed = tidy(&mut repo, 30, now(), &mut |_| {}, &mut |n| told = Some(n));
    match failed {
        Err(e @ RepoError::Damaged { .. }) => {
            assert!(e.to_string().contains(&missing.to_hex()), "{e}");
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(told, None);
    assert_eq!(pack_names(&repo), before);
    assert!(reads(&repo, &entry, ObjectKind::DiaryEntry));
}

/// What `tidy` would keep now, with a 30-day window.
fn marked(repo: &Repository) -> HashSet<Id> {
    let now_ns = i128::from(now()) * 1_000_000;
    let listed = listed(repo.packs_folder(), repo.packs_dir(), now_ns).unwrap();
    kept(repo, &listed, 30, now()).unwrap()
}

/// Every object `keep` names, by kind, from the packs' indexes.
#[cfg(feature = "fault-injection")]
fn stored(repo: &Repository, keep: &HashSet<Id>) -> BTreeMap<Id, ObjectKind> {
    let mut found = BTreeMap::new();
    for name in pack_names(repo) {
        let pack = PackReader::open(&repo.packs_dir().join(name)).unwrap();
        for e in pack.entries() {
            let e = e.unwrap();
            if keep.contains(&e.id) {
                found.insert(e.id, e.kind);
            }
        }
    }
    found
}

#[cfg(feature = "fault-injection")]
#[test]
fn tidy_stopped_at_every_step_loses_nothing() {
    use crate::fsio::fault;
    let mut stopped = BTreeSet::new();
    for n in 1.. {
        assert!(n < 100, "tidy never finished");
        let (dir, mut repo) = project(50);
        save_text(dir.path(), &mut repo, "one", 45);
        save_text(dir.path(), &mut repo, "two", 40);
        step(&mut repo, 40, false).unwrap();
        discard_text(dir.path(), &mut repo, "thrown away", 40);
        crash_leftover(&mut repo);
        save_text(dir.path(), &mut repo, "three", 2);
        age_packs(&repo, 48);
        let keep = marked(&repo);
        let objects = stored(&repo, &keep);
        assert!(objects.len() > 10);
        fault::arm(n);
        let result = tidy(&mut repo, 30, now(), &mut |_| {}, &mut |_| {});
        fault::disarm();
        let why = format!("fault {n}");
        let all_read = |repo: &Repository| {
            for (id, kind) in &objects {
                assert!(reads(repo, id, *kind), "{why}: {kind:?} {id}");
            }
        };
        all_read(&repo);
        let done = ran_to_the_end(&repo, &result, &why, &mut stopped);
        // Tidying again finishes the job.
        tidied(&mut repo);
        all_read(&repo);
        assert!(!text_reads(&repo, "two") && !text_reads(&repo, "thrown away"));
        assert!(!text_reads(&repo, "left by a crash"), "{why}");
        if done {
            break;
        }
    }
    for at in ["tidy:marked", "tidy:written", "tidy:removing"] {
        assert!(stopped.contains(at), "never stopped at {at}: {stopped:?}");
    }
}

/// Whether an armed `tidy` that gave `result` ran to the end; if not, where it stopped goes into
/// `stopped`.
#[cfg(feature = "fault-injection")]
fn ran_to_the_end(
    repo: &Repository,
    result: &Result<TidyReport, RepoError>,
    why: &str,
    stopped: &mut BTreeSet<String>,
) -> bool {
    match result {
        Err(e) => {
            let said = e.to_string();
            // A fault replacing Bolt's own file comes after the file's name.
            let at = said.split_once("injected fault at ").map(|(_, at)| at);
            stopped.insert(at.unwrap_or_else(|| panic!("{why}: {said}")).to_owned());
            false
        }
        Ok(_) => {
            let warnings = repo.store().warnings();
            let hit = warnings
                .iter()
                .any(|w| w.message.contains("injected fault"));
            if hit {
                stopped.insert("tidy:removing".to_owned());
            }
            !hit
        }
    }
}

/// Stopped anywhere, `tidy` leaves every file of every kept save whole, with its chunks.
#[cfg(feature = "fault-injection")]
#[test]
fn tidy_stopped_at_every_step_loses_no_chunk() {
    use crate::fsio::fault;
    let mut stopped = BTreeSet::new();
    for n in 1.. {
        assert!(n < 100, "tidy never finished");
        let (dir, mut repo) = project(50);
        let (kept, dropped, own) = two_versions(dir.path(), &mut repo);
        // A file nothing keeps any more, but for its list stored again in a young pack.
        let young = save_big(dir.path(), &mut repo, &noise(6, 256 << 10), 40);
        step(&mut repo, 40, false).unwrap();
        crash_leftover(&mut repo);
        age_packs(&repo, 48);
        let list = lists_of(&repo, &young).remove(0);
        store_again(&mut repo, &list);
        let objects = stored(&repo, &marked(&repo));
        assert_eq!(objects.get(&kept), Some(&ObjectKind::ChunkList));
        fault::arm(n);
        let result = tidy(&mut repo, 30, now(), &mut |_| {}, &mut |_| {});
        fault::disarm();
        let why = format!("fault {n}");
        let all_read = |repo: &Repository| {
            for (id, kind) in &objects {
                assert!(reads(repo, id, *kind), "{why}: {kind:?} {id}");
            }
            assert!(reads(repo, &young, ObjectKind::ChunkList), "{why}: young");
        };
        all_read(&repo);
        let done = ran_to_the_end(&repo, &result, &why, &mut stopped);
        tidied(&mut repo);
        all_read(&repo);
        assert!(lists_of(&repo, &dropped).is_empty(), "{why}");
        for id in &own {
            assert!(!reads(&repo, id, ObjectKind::Content), "{why}: {id}");
        }
        if done {
            break;
        }
    }
    for at in ["tidy:marked", "tidy:written", "tidy:removing"] {
        assert!(stopped.contains(at), "never stopped at {at}: {stopped:?}");
    }
}

/// Writes a version 1 pack `name` into `packs` holding `contents`, stored as they are.
fn plant_version_1(packs: &Path, name: &str, contents: &[&[u8]]) {
    use crate::encoding::Writer;
    use crate::pack::{PACK_END_MAGIC, PACK_MAGIC};
    let mut objects: Vec<(Id, &[u8])> = contents.iter().map(|c| (hash_content(c), *c)).collect();
    objects.sort_by_key(|(id, _)| *id);
    let mut pack = PACK_MAGIC.to_vec();
    pack.extend_from_slice(&1u32.to_le_bytes());
    pack.extend_from_slice(&0u32.to_le_bytes());
    let mut index = Writer::new();
    index.varint(objects.len() as u64);
    for (id, bytes) in &objects {
        let (at, len) = (pack.len() as u64, bytes.len() as u64);
        let mut head = Writer::new();
        head.u8(ObjectKind::Content as u8);
        head.id(id);
        head.u64(len);
        head.u64(len);
        head.u8(0);
        pack.extend_from_slice(&head.into_bytes());
        pack.extend_from_slice(bytes);
        index.id(id);
        index.u8(ObjectKind::Content as u8);
        index.u64(at);
        index.varint(len);
        index.varint(len);
    }
    let index = index.into_bytes();
    let at = pack.len() as u64;
    pack.extend_from_slice(&index);
    pack.extend_from_slice(&at.to_le_bytes());
    pack.extend_from_slice(blake3::hash(&index).as_bytes());
    pack.extend_from_slice(&PACK_END_MAGIC);
    fs::write(packs.join(name), pack).unwrap();
}

#[test]
fn tidy_rewrites_version_1_and_small_packs_as_version_2() {
    let (dir, mut repo) = project(50);
    save_text(dir.path(), &mut repo, "one", 40);
    plant_version_1(repo.packs_dir(), "old.pack", &[b"one".as_slice()]);
    // 64 MiB that don't compress, alone in a pack: not small, and all of it kept.
    let mut big = vec![0u8; 64 << 20];
    blake3::Hasher::new()
        .update(b"big")
        .finalize_xof()
        .fill(&mut big);
    let big_id = {
        let lock = repo.lock_for_change("test", &mut |_| {}).unwrap();
        let mut packs = repo.new_packs().unwrap();
        let id = packs.add_content(&big).unwrap();
        repo.store_packs(&lock, packs).unwrap();
        id
    };
    let big_pack = pack_names(&repo)
        .into_iter()
        .find(|name| fs::metadata(repo.packs_dir().join(name)).unwrap().len() > 64 << 20)
        .unwrap();
    plant_root(&mut repo, |packs| {
        let file = Entry {
            name: "big.bin".into(),
            kind: EntryKind::File,
            id: big_id,
            size: big.len() as u64,
        };
        add_listing(packs, vec![file]).0
    });
    age_packs(&repo, 48);
    let before = pack_names(&repo);
    let report = tidied(&mut repo);
    let after = pack_names(&repo);
    assert_eq!(report.packs_written, 1, "{report:?}");
    // Only the big pack is as it was; the others, version 1 and small, were written again.
    let stayed: Vec<&String> = after.intersection(&before).collect();
    assert_eq!(stayed, [&big_pack]);
    for name in &after {
        let pack = PackReader::open(&repo.packs_dir().join(name)).unwrap();
        assert_eq!(pack.version(), 2, "{name}");
    }
    assert!(text_reads(&repo, "one"));
    assert!(reads(&repo, &big_id, ObjectKind::Content));
}

#[test]
fn tidy_leaves_a_pack_whose_copy_fails_its_check_with_a_warning() {
    let (dir, mut repo) = project(50);
    save_text(dir.path(), &mut repo, "one", 40);
    let (entry, _) = crash_leftover(&mut repo);
    // A bad copy of something kept, whose good copy is in another pack.
    plant_damaged_copies(repo.packs_dir(), &[(ObjectKind::Content, b"one")]);
    age_packs(&repo, 48);
    tidied(&mut repo);
    let warnings: Vec<_> = repo
        .store()
        .warnings()
        .into_iter()
        .filter(|w| w.kind == warning::PACK_NOT_TIDIED)
        .collect();
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(warnings[0].message.contains("00.pack"), "{warnings:?}");
    assert!(warnings[0].message.contains("Ask for help"), "{warnings:?}");
    assert!(repo.packs_dir().join("00.pack").exists());
    assert!(text_reads(&repo, "one"));
    assert!(!reads(&repo, &entry, ObjectKind::DiaryEntry));
}

/// Flips the last bit of `id` where the version 2 index of the pack at `path` lists it, as a
/// failing disk could.
fn flip_index_id(path: &Path, id: &Id) {
    let mut pack = fs::read(path).unwrap();
    let u64_at = |at: usize| u64::from_le_bytes(pack[at..at + 8].try_into().unwrap());
    let index = usize::try_from(u64_at(pack.len() - 48)).unwrap();
    let count = usize::try_from(u64_at(index)).unwrap();
    let ids = index + 8 + 1024;
    let i = (0..count)
        .find(|i| pack[ids + 32 * i..ids + 32 * (i + 1)] == id.0)
        .unwrap();
    pack[ids + 32 * i + 31] ^= 1;
    fs::write(path, pack).unwrap();
}

/// Gives the first line of the version 2 index of the pack at `path` a kind no version knows.
fn garble_index_line(path: &Path) {
    let mut pack = fs::read(path).unwrap();
    let u64_at = |at: usize| u64::from_le_bytes(pack[at..at + 8].try_into().unwrap());
    let index = usize::try_from(u64_at(pack.len() - 48)).unwrap();
    let count = usize::try_from(u64_at(index)).unwrap();
    pack[index + 8 + 1024 + 32 * count] = 0xee;
    fs::write(path, pack).unwrap();
}

/// A pack `tidy` can't open, or whose index it can't read or check, could hold the only chunk
/// list naming chunks in the others, however young it is: `tidy` stops, naming it, before it
/// removes anything. Here it is mostly the pack holding the chunks two versions of a file share.
#[test]
fn tidy_stops_at_a_pack_it_cannot_read() {
    let damages = [
        "locked",
        "not a pack",
        "a folder",
        "index unreadable",
        "checksum failing",
    ];
    for young in [false, true] {
        for damage in damages {
            let why = format!("{damage}, young: {young}");
            let (dir, mut repo) = project(50);
            let (_, dropped, _) = two_versions(dir.path(), &mut repo);
            // What tidy would remove, were it to go on.
            let (entry, _) = crash_leftover(&mut repo);
            age_packs(&repo, 48);
            let shared = repo.packs_dir().join(&holding(&repo, &dropped)[0]);
            let junk = repo.packs_dir().join("zz.pack");
            let path = match damage {
                "not a pack" => {
                    fs::write(&junk, vec![b'x'; 200]).unwrap();
                    junk
                }
                "a folder" => {
                    fs::create_dir(&junk).unwrap();
                    junk
                }
                "index unreadable" => {
                    garble_index_line(&shared);
                    shared
                }
                "checksum failing" => {
                    flip_index_id(&shared, &dropped);
                    shared
                }
                _ => shared,
            };
            set_age(&path, if young { 1 } else { 48 });
            let held = (damage == "locked").then(|| lock_pack(&path));
            if held.as_ref().is_some_and(Option::is_none) {
                // Root reads everything.
                continue;
            }
            let before = pack_names(&repo);
            let stopped = tidy(&mut repo, 30, now(), &mut |_| {}, &mut |_| {
                panic!("an estimate was given")
            });
            drop(held);
            match stopped {
                Err(e @ RepoError::CantTidy { .. }) => {
                    let name = path.file_name().unwrap().to_str().unwrap();
                    assert!(e.to_string().contains(name), "{why}: {e}");
                    let hint = e.hint().unwrap();
                    assert!(hint.starts_with("Nothing was removed"), "{why}: {hint}");
                    assert!(hint.contains(".bolt/store/packs"), "{why}: {hint}");
                    assert!(hint.contains("only its chunk lists name"), "{why}: {hint}");
                }
                other => panic!("{why}: {other:?}"),
            }
            assert_eq!(pack_names(&repo), before, "{why}");
            assert!(reads(&repo, &entry, ObjectKind::DiaryEntry), "{why}");
        }
    }
}

/// A chunk list that can't be read stops `tidy`, naming its pack: one in a young pack, and one of
/// a file it keeps.
#[test]
fn a_chunk_list_tidy_cannot_read_stops_it_naming_its_pack() {
    for young in [true, false] {
        let (_dir, mut repo) = project(50);
        let file = hash_content(b"a file");
        // Compressed, so it fails its check.
        let list = crate::pack::Prepared {
            id: file,
            size: 6,
            zstd: true,
            stored: zstd::bulk::compress(b"a list", 3).unwrap(),
        };
        let lock = repo.lock_for_change("test", &mut |_| {}).unwrap();
        let mut packs = repo.new_packs().unwrap();
        packs.add_prepared(ObjectKind::ChunkList, &list).unwrap();
        let path = packs.finish().unwrap().remove(0);
        drop(lock);
        if !young {
            plant_root(&mut repo, |packs| {
                let entry = Entry {
                    name: "a.bin".into(),
                    kind: EntryKind::File,
                    id: file,
                    size: 6,
                };
                add_listing(packs, vec![entry]).0
            });
        }
        age_packs(&repo, 48);
        if young {
            set_age(&path, 1);
        }
        let before = pack_names(&repo);
        let stopped = tidy(&mut repo, 30, now(), &mut |_| {}, &mut |_| {
            panic!("an estimate was given")
        });
        match stopped {
            Err(e @ RepoError::CantTidy { .. }) => {
                let said = e.to_string();
                let name = path.file_name().unwrap().to_str().unwrap();
                assert!(said.contains(name) && said.contains("compressed"), "{said}");
                assert!(!said.contains("must be kept"), "{said}");
            }
            other => panic!("young: {young}: {other:?}"),
        }
        assert_eq!(pack_names(&repo), before, "young: {young}");
    }
}

/// An index that reads otherwise the second time, as a failing disk could, must not drop a kept
/// object: the pack whose copy didn't take what its checked index said to keep stays.
#[cfg(feature = "fault-injection")]
#[test]
fn a_pack_whose_copy_takes_other_than_was_judged_is_left() {
    use crate::fsio::fault;
    let (dir, mut repo) = project(50);
    save_text(dir.path(), &mut repo, "one", 40);
    let content = hash_content(b"one");
    let holding = pack_names(&repo)
        .into_iter()
        .find(|name| {
            let pack = PackReader::open(&repo.packs_dir().join(name)).unwrap();
            pack.find(&content).unwrap().is_some()
        })
        .unwrap();
    let path = repo.packs_dir().join(&holding);
    age_packs(&repo, 48);
    let at = path.clone();
    fault::act_at("tidy:copying", move || flip_index_id(&at, &content));
    tidied(&mut repo);
    let warned: Vec<_> = repo
        .store()
        .warnings()
        .into_iter()
        .filter(|w| w.kind == warning::PACK_NOT_TIDIED)
        .collect();
    assert_eq!(warned.len(), 1, "{warned:?}");
    assert!(warned[0].message.contains(&holding), "{warned:?}");
    // Read as it was written, the pack still holds the kept file.
    let mut flipped = content;
    flipped.0[31] ^= 1;
    flip_index_id(&path, &flipped);
    assert!(text_reads(&repo, "one"));
}

/// Where letter case counts, an old pack named like a new one in capitals is another file, and
/// keeping it loses nothing; where it doesn't, it is the new pack.
#[test]
fn an_old_pack_named_like_a_new_one_in_other_letters_is_never_removed() {
    let (dir, mut repo) = project(50);
    save_text(dir.path(), &mut repo, "one", 40);
    // Tidied twice, what is kept is one pack in order of ID, which tidying writes again as it is.
    for _ in 0..2 {
        age_packs(&repo, 48);
        tidied(&mut repo);
    }
    let names = pack_names(&repo);
    assert_eq!(names.len(), 1, "{names:?}");
    let name = names.first().unwrap();
    let upper = format!(
        "{}.pack",
        name.trim_end_matches(".pack").to_ascii_uppercase()
    );
    fs::rename(repo.packs_dir().join(name), repo.packs_dir().join(&upper)).unwrap();
    age_packs(&repo, 48);
    let report = tidied(&mut repo);
    assert_eq!(report.packs_written, 1, "{report:?}");
    assert!(repo.packs_dir().join(&upper).exists(), "{report:?}");
    assert!(text_reads(&repo, "one"));
}

#[test]
fn tidy_still_stops_at_a_conflict_record() {
    for young in [false, true] {
        let (dir, mut repo) = project(50);
        save_text(dir.path(), &mut repo, "one", 40);
        let (entry, _) = crash_leftover(&mut repo);
        let before = pack_names(&repo);
        let lock = repo.lock_for_change("test", &mut |_| {}).unwrap();
        let mut packs = repo.new_packs().unwrap();
        packs
            .add_record(ObjectKind::Conflict, b"not followed here")
            .unwrap();
        repo.store_packs(&lock, packs).unwrap();
        drop(lock);
        let planted = pack_names(&repo).difference(&before).next().cloned();
        let planted = planted.unwrap();
        age_packs(&repo, 48);
        if young {
            set_age(&repo.packs_dir().join(&planted), 1);
        }
        let before = pack_names(&repo);
        let stopped = tidy(&mut repo, 30, now(), &mut |_| {}, &mut |_| {
            panic!("an estimate was given")
        });
        match stopped {
            Err(e @ RepoError::CantTidy { .. }) => {
                let said = e.to_string();
                assert!(said.contains(&planted), "{said}");
                assert!(said.contains("a conflict record"), "{said}");
                let hint = e.hint().unwrap();
                assert!(hint.starts_with("Nothing was removed"), "{hint}");
                assert!(hint.contains("follows them"), "{hint}");
            }
            other => panic!("young: {young}: {other:?}"),
        }
        assert_eq!(pack_names(&repo), before, "young: {young}");
        assert!(reads(&repo, &entry, ObjectKind::DiaryEntry));
    }
}

/// The chunks of an older version of a file, shared with the version kept, sit only in the pack
/// of the older one, beside its own chunks.
#[test]
fn tidy_keeps_every_chunk_a_kept_file_needs() {
    let (dir, mut repo) = project(50);
    let (kept, dropped, own) = two_versions(dir.path(), &mut repo);
    age_packs(&repo, 48);
    tidied(&mut repo);
    assert!(reads(&repo, &kept, ObjectKind::ChunkList));
    assert!(!reads(&repo, &dropped, ObjectKind::ChunkList));
    assert!(lists_of(&repo, &dropped).is_empty());
    for id in &own {
        assert!(!reads(&repo, id, ObjectKind::Content), "{id}");
    }
}

#[test]
fn tidy_removes_the_chunks_only_an_old_file_needed() {
    let (dir, mut repo) = project(50);
    save_text(dir.path(), &mut repo, "one", 45);
    let old = save_big(dir.path(), &mut repo, &noise(2, 512 << 10), 40);
    step(&mut repo, 40, false).unwrap();
    let recent = save_big(dir.path(), &mut repo, &noise(3, 512 << 10), 2);
    step(&mut repo, 2, false).unwrap();
    let old_chunks = chunks_of(&repo, &old);
    let recent_chunks = chunks_of(&repo, &recent);
    age_packs(&repo, 48);
    tidied(&mut repo);
    // Undone within the window, the recent one can still come back, every chunk of it.
    assert!(reads(&repo, &recent, ObjectKind::ChunkList));
    assert!(!reads(&repo, &old, ObjectKind::ChunkList));
    for id in &old_chunks {
        assert!(!reads(&repo, id, ObjectKind::Content), "{id}");
    }
    // A month on, the recent one goes too.
    tidy(&mut repo, 30, now() + 29 * DAY, &mut |_| {}, &mut |_| {}).unwrap();
    assert!(!reads(&repo, &recent, ObjectKind::ChunkList));
    for id in &recent_chunks {
        assert!(!reads(&repo, id, ObjectKind::Content), "{id}");
    }
    assert!(text_reads(&repo, "one"));
}

#[test]
fn tidy_keeps_the_chunks_a_young_packs_list_names() {
    let (dir, mut repo) = project(50);
    let file = save_big(dir.path(), &mut repo, &noise(4, 512 << 10), 40);
    step(&mut repo, 40, false).unwrap();
    let list = lists_of(&repo, &file).remove(0);
    age_packs(&repo, 48);
    // Not its chunks, which the store held.
    store_again(&mut repo, &list);
    let report = tidied(&mut repo);
    assert_eq!(report.packs_young, 1, "{report:?}");
    assert!(reads(&repo, &file, ObjectKind::ChunkList));
    // A day later, the list goes, and its chunks with it.
    age_packs(&repo, 25);
    tidied(&mut repo);
    assert!(!reads(&repo, &file, ObjectKind::ChunkList));
    for chunk in &list.chunks {
        assert!(
            !reads(&repo, &chunk.id, ObjectKind::Content),
            "{}",
            chunk.id
        );
    }
}

#[test]
fn tidy_follows_every_list_stored_under_a_files_id() {
    let (dir, mut repo) = project(50);
    let bytes = noise(5, 512 << 10);
    let file = save_big(dir.path(), &mut repo, &bytes, 40);
    // The same file cut otherwise, its chunks beside it in a pack of their own.
    let pieces: Vec<&[u8]> = bytes.chunks(100 << 10).collect();
    let other = ChunkList {
        file,
        size: bytes.len() as u64,
        chunks: pieces
            .iter()
            .map(|p| Chunk {
                id: hash_content(p),
                size: p.len() as u64,
            })
            .collect(),
    };
    let lock = repo.lock_for_change("test", &mut |_| {}).unwrap();
    let mut packs = repo.new_packs().unwrap();
    for piece in &pieces {
        packs.add_content(piece).unwrap();
    }
    packs.add_chunk_list(&other).unwrap();
    repo.store_packs(&lock, packs).unwrap();
    drop(lock);
    let mut lists = lists_of(&repo, &file);
    assert_eq!(lists.len(), 2);
    age_packs(&repo, 48);
    tidied(&mut repo);
    // Both lists stay, and so does every chunk either names.
    let mut after = lists_of(&repo, &file);
    lists.sort_by_key(ChunkList::encode);
    after.sort_by_key(ChunkList::encode);
    assert_eq!(after, lists);
    for chunk in lists.iter().flat_map(|l| &l.chunks) {
        assert!(reads(&repo, &chunk.id, ObjectKind::Content), "{}", chunk.id);
    }
}

/// A pack is small by its chunk lists' own bytes, not their files' sizes.
#[test]
fn a_pack_holding_a_big_files_list_is_written_again_as_small() {
    let (_dir, mut repo) = project(50);
    let big = ChunkList {
        file: Id([7; 32]),
        size: 10 << 30,
        chunks: vec![Chunk {
            id: Id([8; 32]),
            size: 10 << 30,
        }],
    };
    let lock = repo.lock_for_change("test", &mut |_| {}).unwrap();
    let mut packs = repo.new_packs().unwrap();
    packs.add_chunk_list(&big).unwrap();
    let path = packs.finish().unwrap().remove(0);
    drop(lock);
    plant_root(&mut repo, |packs| {
        let file = Entry {
            name: "big.bin".into(),
            kind: EntryKind::File,
            id: big.file,
            size: big.size,
        };
        add_listing(packs, vec![file]).0
    });
    age_packs(&repo, 48);
    tidied(&mut repo);
    // It held only what is kept, in version 2, so only being small had it written again.
    assert!(!path.exists());
    assert_eq!(lists_of(&repo, &big.file), [big]);
}

#[cfg(feature = "fault-injection")]
#[test]
fn packs_holding_nothing_to_keep_go_first_even_when_writing_fails() {
    use crate::fsio::fault;
    for n in 1.. {
        assert!(n < 50, "writing the new pack never failed");
        let (dir, mut repo) = project(50);
        save_text(dir.path(), &mut repo, "one", 40);
        let before = pack_names(&repo);
        crash_leftover(&mut repo);
        age_packs(&repo, 48);
        fault::arm(n);
        let result = tidy(&mut repo, 30, now(), &mut |_| {}, &mut |_| {});
        fault::disarm();
        let Err(e) = result else {
            continue;
        };
        if !e.to_string().starts_with("injected fault at pack:") {
            continue;
        }
        // As on a full disk: the pack holding nothing to keep is gone, the others as they were.
        assert_eq!(pack_names(&repo), before, "fault {n}");
        assert!(text_reads(&repo, "one"));
        break;
    }
}

/// Anything named like a pack is judged as one, a folder too, so one that isn't a pack stops
/// `tidy`; anything else is passed over.
#[test]
fn everything_named_like_a_pack_is_judged() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("a.pack"), b"x").unwrap();
    fs::create_dir(dir.path().join("b.pack")).unwrap();
    fs::write(dir.path().join("c.txt"), b"x").unwrap();
    let found = listed(&Dir::open_top(dir.path()).unwrap(), dir.path(), 0).unwrap();
    let names: Vec<&OsString> = found.iter().map(|pack| &pack.name).collect();
    assert_eq!(names, ["a.pack", "b.pack"]);
}

#[test]
fn tidy_says_what_it_will_free_and_what_it_freed() {
    let (dir, mut repo) = project(50);
    save_text(dir.path(), &mut repo, "one", 40);
    // 64 KiB that don't compress, which nothing reaches.
    let mut lost = vec![0u8; 64 << 10];
    blake3::Hasher::new()
        .update(b"lost")
        .finalize_xof()
        .fill(&mut lost);
    let lock = repo.lock_for_change("test", &mut |_| {}).unwrap();
    let mut packs = repo.new_packs().unwrap();
    packs.add_content(&lost).unwrap();
    repo.store_packs(&lock, packs).unwrap();
    drop(lock);
    age_packs(&repo, 48);
    let before = pack_names(&repo);
    let packs_dir = repo.packs_dir().to_owned();
    let mut told = Vec::new();
    let report = tidy(&mut repo, 30, now(), &mut |_| {}, &mut |n| {
        // Said before anything is removed.
        let there = before.iter().all(|name| packs_dir.join(name).exists());
        told.push((n, there));
    })
    .unwrap();
    assert_eq!(told, [(report.to_free, true)]);
    let freed = report.bytes_before - report.bytes_after;
    assert!(freed >= 64 << 10, "{report:?}");
    // Nothing kept is in two packs here, so the estimate is exact.
    assert_eq!(report.to_free, freed, "{report:?}");
    assert_eq!(report.packs_removed, before.len(), "{report:?}");
    assert_eq!(report.packs_written, 1, "{report:?}");
    // Nothing more to free.
    let again = tidy(&mut repo, 30, now(), &mut |_| {}, &mut |_| {}).unwrap();
    assert_eq!((again.to_free, again.packs_removed), (0, 0), "{again:?}");
}

/// The references `kept` follows.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum Ref {
    Line,
    AtSave,
    Shelf,
    TagList,
    Tag,
    ServerLines,
    ServerLine,
    Parent,
    Root,
    File,
    Executable,
    Link,
    Folder,
    ProjectLink,
    Diary,
    Change,
    Before,
    Target,
    TargetBefore,
    TargetChange,
}

const REFS: [Ref; 20] = [
    Ref::Line,
    Ref::AtSave,
    Ref::Shelf,
    Ref::TagList,
    Ref::Tag,
    Ref::ServerLines,
    Ref::ServerLine,
    Ref::Parent,
    Ref::Root,
    Ref::File,
    Ref::Executable,
    Ref::Link,
    Ref::Folder,
    Ref::ProjectLink,
    Ref::Diary,
    Ref::Change,
    Ref::Before,
    Ref::Target,
    Ref::TargetBefore,
    Ref::TargetChange,
];

/// A store in which each reference names an object nothing else does, but `without`, which is
/// taken away: its object is stored all the same. Gives each object by the reference to it.
fn planted(without: Option<Ref>) -> (tempfile::TempDir, Repository, BTreeMap<Ref, Id>) {
    let has = |r: Ref| without != Some(r);
    let (dir, mut repo) = project(100);
    let base = tip(&repo);
    let mut named = BTreeMap::new();
    let lock = repo.lock_for_change("test", &mut |_| {}).unwrap();
    let mut packs = repo.new_packs().unwrap();
    let (empty, _) = add_listing(&mut packs, vec![]);
    let deep = packs.add_content(b"deep").unwrap();
    let below = Entry {
        name: "deep.txt".into(),
        kind: EntryKind::File,
        id: deep,
        size: 4,
    };
    let (folder, _) = add_listing(&mut packs, vec![below]);
    let link = add_link(&mut packs, &[7; 20]);
    let file = packs.add_content(b"file").unwrap();
    let executable = packs.add_content(b"executable").unwrap();
    let target = packs.add_content(b"link target").unwrap();
    let mut entries = Vec::new();
    for (r, name, kind, id) in [
        (Ref::File, "a.txt", EntryKind::File, file),
        (Ref::Executable, "b.sh", EntryKind::Executable, executable),
        (Ref::Link, "c", EntryKind::Symlink, target),
        (Ref::Folder, "d", EntryKind::Folder, folder),
        (Ref::ProjectLink, "e", EntryKind::ProjectLink, link),
    ] {
        named.insert(r, id);
        if has(r) {
            entries.push(Entry {
                name: name.into(),
                kind,
                id,
                size: 0,
            });
        }
    }
    let (root, _) = add_listing(&mut packs, entries);
    let mut unique = |what: &str| a_save(&mut packs, empty, vec![base], what);
    let saves: BTreeMap<&str, Id> = [
        "parent",
        "line",
        "at",
        "shelved",
        "tagged",
        "served",
        "before",
        "target",
        "target before",
        "from",
        "to",
        "target from",
        "target to",
    ]
    .into_iter()
    .map(|what| (what, unique(what)))
    .collect();
    // The parent only it reaches comes second: every parent is followed, not just the first.
    let parents = if has(Ref::Parent) {
        vec![base, saves["parent"]]
    } else {
        vec![base]
    };
    let main = a_save(
        &mut packs,
        if has(Ref::Root) { root } else { empty },
        parents,
        "main",
    );
    let tag = Tag {
        save: if has(Ref::Tag) { saves["tagged"] } else { base },
        message: String::new(),
        git_tags: vec![],
    };
    let tags = TagList {
        tags: BTreeMap::from([("v1".to_owned(), tag)]),
    };
    let tag_list = packs
        .add_record(ObjectKind::TagList, &tags.encode())
        .unwrap();
    let served = if has(Ref::ServerLine) {
        saves["served"]
    } else {
        base
    };
    let servers = ServerLines {
        servers: BTreeMap::from([(
            "origin".to_owned(),
            BTreeMap::from([("main".to_owned(), served)]),
        )]),
    };
    let server_lines = packs
        .add_record(ObjectKind::ServerLines, &servers.encode())
        .unwrap();
    repo.store_packs(&lock, packs).unwrap();
    drop(lock);
    for (r, what) in [
        (Ref::Parent, "parent"),
        (Ref::Line, "line"),
        (Ref::AtSave, "at"),
        (Ref::Shelf, "shelved"),
        (Ref::Tag, "tagged"),
        (Ref::ServerLine, "served"),
        (Ref::Before, "before"),
        (Ref::Target, "target"),
        (Ref::TargetBefore, "target before"),
        (Ref::Change, "from"),
        (Ref::TargetChange, "target from"),
    ] {
        named.insert(r, saves[what]);
    }
    named.extend([
        (Ref::Root, root),
        (Ref::TagList, tag_list),
        (Ref::ServerLines, server_lines),
    ]);

    // The diary: old entries, then the head, an undo within the window.
    let on_main = |save: Id| State {
        lines: BTreeMap::from([("main".to_owned(), save)]),
        tags: None,
        position: Position::OnLine("main".into()),
        shelves: vec![],
        server_lines: None,
    };
    let change = |from: &str, to: &str| FolderChange {
        from: saves[from],
        to: saves[to],
    };
    append(
        &mut repo,
        100,
        Action::Command,
        on_main(saves["target before"]),
        None,
    );
    let step_change = has(Ref::TargetChange).then(|| change("target from", "target to"));
    let step = append(
        &mut repo,
        100,
        Action::Command,
        on_main(saves["target"]),
        step_change,
    );
    // An entry only the diary reaches; without `Diary` a crash left it off the diary.
    let lock = repo.lock_for_change("test", &mut |_| {}).unwrap();
    let middle = DiaryEntry {
        parent: Some(step),
        action: Action::Command,
        command: "test".into(),
        who: days_ago(100),
        bolt_version: "0.1.0".into(),
        state: on_main(base),
        folder_change: None,
    };
    let packs = repo.new_packs().unwrap();
    let middle = if has(Ref::Diary) {
        repo.commit(&lock, packs, &middle).unwrap()
    } else {
        repo.store_entry(&lock, packs, &middle).unwrap()
    };
    drop(lock);
    named.insert(Ref::Diary, middle);
    append(
        &mut repo,
        100,
        Action::Command,
        on_main(saves["before"]),
        None,
    );
    let mut state = on_main(main);
    if has(Ref::Line) {
        state.lines.insert("other".into(), saves["line"]);
    }
    if has(Ref::AtSave) {
        state.position = Position::AtSave(saves["at"]);
    }
    if has(Ref::Shelf) {
        state.shelves = vec![("1".into(), saves["shelved"])];
    }
    state.tags = has(Ref::TagList).then_some(tag_list);
    state.server_lines = has(Ref::ServerLines).then_some(server_lines);
    let action = if has(Ref::Target) && has(Ref::TargetBefore) {
        Action::Undo(step)
    } else {
        Action::Command
    };
    let days = if has(Ref::Before) { 0 } else { 100 };
    let head_change = has(Ref::Change).then(|| change("from", "to"));
    append(&mut repo, days, action, state, head_change);
    (dir, repo, named)
}

#[test]
fn each_kind_of_reference_is_followed() {
    let (_dir, repo, named) = planted(None);
    let keep = marked(&repo);
    for (r, id) in &named {
        assert!(keep.contains(id), "{r:?} isn't followed");
    }
    // Each taken away in turn: what it named goes.
    for r in REFS {
        let (_dir, repo, named) = planted(Some(r));
        let keep = marked(&repo);
        assert!(
            !keep.contains(&named[&r]),
            "{r:?} taken away, but what it names is kept"
        );
    }
}
