//! Merging the store's small packs into one, so the number of packs stays near 32 however many
//! commands have run (decision 057).

use crate::fsio::{self, fault_point};
use crate::pack::{PackError, PackReader, PackSet};
use crate::repo::{ChangeLock, RepoError, Repository};
use crate::store::{is_damage, read_whole};
use crate::text::escape;
use crate::warning::{self, Warning};
use bolt_platform::dir::{Dir, EntryType};
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// A store with more packs than this is merged.
const MERGE_ABOVE: usize = 32;
/// Packs at least this big are left as they are.
const SMALL: u64 = 64 << 20;

/// What a merge did.
#[derive(Debug, Default)]
pub struct MergeReport {
    /// The packs copied into the new ones.
    pub merged: usize,
    /// Packs left as they were because a copy in them failed its check.
    pub kept_damaged: Vec<PathBuf>,
    /// Packs copied but not removed, each with the reason. Their objects are in the new pack too.
    pub not_removed: Vec<(PathBuf, String)>,
}

/// Merges every pack under 64 MiB into new ones once the store holds more than 32. Each object is
/// checked before its stored bytes are copied as they are, and the old packs are removed only once
/// the new ones are in the store, so a stop at any point leaves every object readable. `None` when
/// there weren't enough to merge.
pub(crate) fn merge_small_packs(
    repo: &mut Repository,
    _lock: &ChangeLock,
) -> Result<Option<MergeReport>, RepoError> {
    repo.store_mut().refresh()?;
    if repo.store().pack_count() <= MERGE_ABOVE {
        return Ok(None);
    }
    let folder = Arc::clone(repo.packs_folder());
    let packs_dir = repo.packs_dir().to_owned();
    let mut small = Vec::new();
    for entry in folder.entries()? {
        if Path::new(&entry.name)
            .extension()
            .is_none_or(|e| e != "pack")
        {
            continue;
        }
        // Anything but a file is left for reads to report.
        if let Ok(meta) = folder.metadata(&entry.name)
            && meta.kind == EntryType::File
            && meta.size < SMALL
        {
            small.push(entry.name);
        }
    }
    if small.len() < 2 {
        return Ok(None);
    }
    small.sort();
    // An open file can't always be replaced or removed on Windows.
    repo.store().close_all();
    let mut report = MergeReport::default();
    let mut taken = Vec::new();
    let mut out = repo.new_packs()?;
    for name in small {
        let path = packs_dir.join(&name);
        match copy_pack(&folder, &name, &path, &mut out) {
            Ok(()) => taken.push(name),
            Err(e) if is_damage(&e) => report.kept_damaged.push(path),
            Err(e) => return Err(e.into()),
        }
    }
    let made: Vec<OsString> = out
        .finish()?
        .iter()
        .filter_map(|p| p.file_name().map(OsStr::to_owned))
        .collect();
    fault_point("merge:written")?;
    report.merged = taken.len();
    for name in taken {
        // Merging what a stopped merge left can make the same pack again.
        if made.contains(&name) {
            continue;
        }
        let removed =
            fault_point("merge:removing").and_then(|()| fsio::remove_file(&folder, &name));
        if let Err(e) = removed {
            report
                .not_removed
                .push((packs_dir.join(&name), e.to_string()));
        }
    }
    repo.store_mut().refresh()?;
    Ok(Some(report))
}

/// Merges after a command has committed. The command has happened, so whatever goes wrong is
/// only a warning, kept with the store's.
pub(crate) fn merge_after_commit(repo: &mut Repository, lock: &ChangeLock) {
    let shown = |path: &Path| escape(&path.display().to_string(), false);
    let mut notes = Vec::new();
    match merge_small_packs(repo, lock) {
        Ok(None) => {}
        Ok(Some(report)) => {
            for path in &report.kept_damaged {
                notes.push(format!(
                    "Pack {} wasn't merged with the other small packs because a copy in it \
                     failed its check. Ask for help before trying to repair the project.",
                    shown(path)
                ));
            }
            for (path, reason) in &report.not_removed {
                notes.push(format!(
                    "Pack {} was merged into a new pack but couldn't be removed: {}. Its \
                     objects are in both, which does no harm; the next merge tries again.",
                    shown(path),
                    escape(reason.trim_end_matches('.'), false)
                ));
            }
        }
        Err(e) => notes.push(format!(
            "Bolt couldn't merge the store's small packs: {}. The command still happened, and \
             the next one that changes the project tries again.",
            e.to_string().trim_end_matches('.')
        )),
    }
    for note in notes {
        repo.store()
            .note(Warning::new(warning::PACK_NOT_MERGED, note));
    }
}

/// Checks each object of the pack `name` and copies it into `out`.
fn copy_pack(folder: &Dir, name: &OsStr, path: &Path, out: &mut PackSet) -> Result<(), PackError> {
    let pack = PackReader::open_in(folder, name, path)?;
    for entry in pack.entries() {
        let entry = entry?;
        read_whole(entry.kind)(&pack, &entry)?;
        out.add_stored(&pack, &entry)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;
    use crate::diary::DiaryEntry;
    use crate::id::Id;
    use crate::pack::ObjectKind;
    use crate::save::Person;
    use crate::snapshot::{SaveOutcome, SaveRequest, save_folder};
    use crate::store::Store;
    use crate::test_support::plant_damaged_copies;
    use crate::warning;
    use std::path::Path;

    fn who() -> Person {
        Person {
            name: "Ada".into(),
            email: "ada@example.com".into(),
            time_ms: 0,
            tz_offset_min: 0,
        }
    }

    fn project() -> (tempfile::TempDir, Repository) {
        let dir = tempfile::tempdir().unwrap();
        let repo = Repository::init(dir.path(), &who()).unwrap();
        (dir, repo)
    }

    fn object(i: usize) -> Vec<u8> {
        format!("object {i}\n").into_bytes()
    }

    /// Adds `n` packs of one object each, `object(0)` to `object(n - 1)`, and returns the lock.
    fn add_packs(repo: &mut Repository, n: usize) -> ChangeLock {
        let lock = repo.lock_for_change("test", &mut |_| {}).unwrap();
        for i in 0..n {
            let mut packs = repo.new_packs().unwrap();
            packs.add_content(&object(i)).unwrap();
            packs.finish().unwrap();
        }
        drop(lock);
        repo.lock_for_change("test", &mut |_| {}).unwrap()
    }

    fn pack_names(packs: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(packs)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .filter(|name| name.ends_with(".pack"))
            .collect();
        names.sort();
        names
    }

    /// Reads `object(0)` to `object(n - 1)` and the head's entry through a store opened afresh.
    fn assert_all_readable(repo: &Repository, n: usize, why: &str) {
        let store = Store::open(repo.packs_dir()).unwrap();
        for i in 0..n {
            let mut out = Vec::new();
            store
                .copy_content(&crate::id::hash_content(&object(i)), &mut out)
                .unwrap_or_else(|e| panic!("{why}: object {i}: {e}"));
            assert_eq!(out, object(i), "{why}");
        }
        let head = repo.head().unwrap();
        let entry = store.read_record(&head, ObjectKind::DiaryEntry).unwrap();
        DiaryEntry::decode(&entry).unwrap();
    }

    #[test]
    fn thirty_three_saves_leave_at_most_a_few_packs() {
        let (dir, mut repo) = project();
        let who = who();
        let mut saves = Vec::new();
        for i in 0..33 {
            std::fs::write(dir.path().join("a.txt"), format!("version {i}\n")).unwrap();
            let req = SaveRequest {
                message: "Change a",
                who: &who,
                command: "save",
                accept_crlf: false,
                git_excludes: None,
            };
            match save_folder(&mut repo, &req, &mut |_| {}).unwrap() {
                SaveOutcome::Saved {
                    save_id, warnings, ..
                } => {
                    assert!(warnings.is_empty(), "{warnings:?}");
                    saves.push(save_id);
                }
                SaveOutcome::NothingChanged { .. } => panic!("nothing was saved"),
            }
        }
        let count = pack_names(repo.packs_dir()).len();
        assert!(count <= 3, "{count} packs");
        assert!(repo.store().warnings().is_empty());
        let store = Store::open(repo.packs_dir()).unwrap();
        for (i, id) in saves.iter().enumerate() {
            let save = store.save(id, false).unwrap();
            let listing = store.listing(&save.root).unwrap();
            let file = listing.entries().first().unwrap();
            let mut out = Vec::new();
            store.copy_content(&file.id, &mut out).unwrap();
            assert_eq!(out, format!("version {i}\n").into_bytes());
        }
    }

    #[test]
    fn thirty_two_packs_are_left_alone() {
        let (_dir, mut repo) = project();
        // The first save's pack makes 32.
        let lock = add_packs(&mut repo, 31);
        let before = pack_names(repo.packs_dir());
        assert_eq!(before.len(), 32);
        assert!(merge_small_packs(&mut repo, &lock).unwrap().is_none());
        assert_eq!(pack_names(repo.packs_dir()), before);
    }

    #[cfg(feature = "fault-injection")]
    #[test]
    fn a_merge_stopped_at_every_step_leaves_every_object_readable() {
        use crate::fsio::fault;
        let (mut stopped_written, mut stopped_removing) = (false, false);
        for n in 1.. {
            assert!(n < 200, "the merge never finished");
            let (_dir, mut repo) = project();
            let lock = add_packs(&mut repo, 33);
            fault::arm(n);
            let result = merge_small_packs(&mut repo, &lock);
            fault::disarm();
            let why = format!("fault {n}");
            assert_all_readable(&repo, 33, &why);
            let checked = crate::verify::verify_store(repo.packs_dir());
            assert!(checked.is_ok(), "{why}: {:?}", checked.problems);
            let done = match result {
                Ok(Some(report)) if report.not_removed.is_empty() => true,
                Ok(Some(report)) => {
                    stopped_removing = true;
                    assert!(report.not_removed[0].1.contains("merge:removing"), "{why}");
                    false
                }
                Ok(None) => panic!("{why}: nothing merged"),
                Err(e) => {
                    stopped_written |= e.to_string().contains("merge:written");
                    false
                }
            };
            // A merge run again finishes the job.
            merge_small_packs(&mut repo, &lock).unwrap();
            assert_all_readable(&repo, 33, &why);
            let left = pack_names(repo.packs_dir()).len();
            assert!(left <= 2, "{why}: {left} packs");
            if done {
                assert_eq!(left, 1, "{why}");
                break;
            }
        }
        assert!(stopped_written, "no stop after the new pack went in");
        assert!(stopped_removing, "no stop while removing");
    }

    #[test]
    fn a_damaged_pack_is_left_and_named() {
        let (_dir, mut repo) = project();
        let lock = add_packs(&mut repo, 33);
        // A bad copy of object 3, whose good copy is in another pack.
        plant_damaged_copies(repo.packs_dir(), &[(ObjectKind::Content, &object(3))]);
        let report = merge_small_packs(&mut repo, &lock).unwrap().unwrap();
        let damaged = repo.packs_dir().join("00.pack");
        assert_eq!(report.kept_damaged, vec![damaged.clone()]);
        assert_eq!(report.merged, 34);
        let names = pack_names(repo.packs_dir());
        assert_eq!(names.len(), 2, "{names:?}");
        assert!(names.contains(&"00.pack".to_owned()));
        assert_all_readable(&repo, 33, "after the merge");

        // After a commit the warning comes with the command's own.
        let (head, mut entry) = repo.current().unwrap();
        entry.parent = Some(head);
        for i in 0..32 {
            let mut packs = repo.new_packs().unwrap();
            packs.add_content(&object(100 + i)).unwrap();
            packs.finish().unwrap();
        }
        let packs = repo.new_packs().unwrap();
        repo.commit(&lock, packs, &entry).unwrap();
        let warnings = repo.store().warnings();
        let named: Vec<_> = warnings
            .iter()
            .filter(|w| w.kind == warning::PACK_NOT_MERGED)
            .collect();
        assert_eq!(named.len(), 1, "{warnings:?}");
        assert!(named[0].message.contains("00.pack"), "{warnings:?}");
    }

    #[test]
    fn big_packs_are_not_merged() {
        let (_dir, mut repo) = project();
        let lock = add_packs(&mut repo, 33);
        // 64 MiB that don't compress.
        let mut big = vec![0u8; 64 << 20];
        let mut reader = blake3::Hasher::new().update(b"big").finalize_xof();
        reader.fill(&mut big);
        let mut packs = repo.new_packs().unwrap();
        let big_id: Id = packs.add_content(&big).unwrap();
        let big_pack = packs.finish().unwrap().remove(0);
        let report = merge_small_packs(&mut repo, &lock).unwrap().unwrap();
        assert_eq!(report.merged, 34);
        assert!(big_pack.exists());
        assert_eq!(pack_names(repo.packs_dir()).len(), 2);
        assert_all_readable(&repo, 33, "after the merge");
        let store = Store::open(repo.packs_dir()).unwrap();
        let mut out = Vec::new();
        store.copy_content(&big_id, &mut out).unwrap();
        assert!(out == big);
    }
}
