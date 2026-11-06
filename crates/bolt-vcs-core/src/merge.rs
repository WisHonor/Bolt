//! Merging the store's small packs into one, so their number stays near 32 however many commands
//! have run (decisions 057 and 062).

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

/// A merge runs once more packs than this are small.
const MERGE_ABOVE: usize = 32;
/// A small pack is under this size on disk...
const SMALL_ON_DISK: u64 = 64 << 20;
/// ...and its index records at most this much uncompressed, so checking it costs little.
const SMALL_UNCOMPRESSED: u64 = 256 << 20;

/// What a merge did.
#[derive(Debug, Default)]
pub struct MergeReport {
    /// The packs copied into the new ones.
    pub merged: usize,
    /// Packs left as they were because a copy in them failed its check.
    pub kept_damaged: Vec<PathBuf>,
    /// Packs left as they were because they couldn't be read, each with the reason.
    pub kept_unreadable: Vec<(PathBuf, String)>,
    /// Packs copied but not removed, each with the reason. Their objects are in the new pack too.
    pub not_removed: Vec<(PathBuf, String)>,
}

impl MergeReport {
    /// Keeps the pack at `path`, which failed to read with `e`.
    fn keep(&mut self, path: PathBuf, e: &PackError) {
        if is_damage(e) {
            self.kept_damaged.push(path);
        } else {
            self.kept_unreadable.push((path, e.to_string()));
        }
    }
}

/// Merges the small packs (decision 062) into new ones once there are more than 32. Each object is
/// checked before its stored bytes are copied as they are, and the old packs are removed only once
/// the new ones are in the store, so a stop at any point leaves every object readable. A pack that
/// can't be read is kept; only a failure to write stops the merge. `None` when there weren't
/// enough small packs.
pub(crate) fn merge_small_packs(
    repo: &mut Repository,
    _lock: &ChangeLock,
) -> Result<Option<MergeReport>, RepoError> {
    let folder = Arc::clone(repo.packs_folder());
    let packs_dir = repo.packs_dir().to_owned();
    let mut on_disk = Vec::new();
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
            && meta.size < SMALL_ON_DISK
        {
            on_disk.push(entry.name);
        }
    }
    // Most commits stop here, without opening a pack.
    if on_disk.len() <= MERGE_ABOVE {
        return Ok(None);
    }
    on_disk.sort();
    // An open file can't always be replaced or removed on Windows.
    repo.store().close_all();
    let mut report = MergeReport::default();
    let mut small = Vec::new();
    for name in on_disk {
        let path = packs_dir.join(&name);
        match holds_little(&folder, &name, &path) {
            Ok(true) => small.push(name),
            Ok(false) => {}
            Err(e) => report.keep(path, &e),
        }
    }
    if small.len() <= MERGE_ABOVE {
        return Ok(None);
    }
    let mut taken = Vec::new();
    let mut out = repo.new_packs()?;
    for name in small {
        let path = packs_dir.join(&name);
        fsio::act_point("merge:copying");
        match copy_pack(&folder, &name, &path, &mut out) {
            Ok(()) => taken.push(name),
            Err(Failed::Read(e)) => report.keep(path, &e),
            Err(Failed::Write(e)) => return Err(e.into()),
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
    let reason = |r: &str| escape(r.trim_end_matches('.'), false);
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
            for (path, why) in &report.kept_unreadable {
                notes.push(format!(
                    "Pack {} wasn't merged with the other small packs because Bolt couldn't \
                     read it: {}. It is left as it is, and the next merge tries again.",
                    shown(path),
                    reason(why)
                ));
            }
            for (path, why) in &report.not_removed {
                notes.push(format!(
                    "Pack {} was merged into a new pack but couldn't be removed: {}. Its \
                     objects are in both, which does no harm; the next merge tries again.",
                    shown(path),
                    reason(why)
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

/// Whether the uncompressed sizes the index of pack `name` records total at most
/// `SMALL_UNCOMPRESSED`. Only the index is read.
fn holds_little(folder: &Dir, name: &OsStr, path: &Path) -> Result<bool, PackError> {
    let pack = PackReader::open_in(folder, name, path)?;
    let mut total = 0u64;
    for entry in pack.entries() {
        total = total.saturating_add(entry?.uncompressed_size);
        if total > SMALL_UNCOMPRESSED {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Why a pack wasn't copied: reading it, which keeps only that pack out, or writing the new ones,
/// which stops the merge.
enum Failed {
    Read(PackError),
    Write(PackError),
}

/// Checks each object of the pack `name` and copies it into `out`.
fn copy_pack(folder: &Dir, name: &OsStr, path: &Path, out: &mut PackSet) -> Result<(), Failed> {
    let pack = PackReader::open_in(folder, name, path).map_err(Failed::Read)?;
    for entry in pack.entries() {
        let entry = entry.map_err(Failed::Read)?;
        read_whole(entry.kind)(&pack, &entry).map_err(Failed::Read)?;
        match out.add_stored(&pack, &entry) {
            // The copy's header, read again.
            Err(e) if is_damage(&e) => return Err(Failed::Read(e)),
            added => added.map_err(Failed::Write)?,
        }
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

    /// Writes a version 1 pack `name` holding one zstd copy that claims `claimed` bytes
    /// uncompressed but whose few stored bytes are junk, so reading it would fail as damage.
    fn plant_claiming(packs: &Path, name: &str, claimed: u64) {
        use crate::encoding::Writer;
        use crate::pack::{PACK_END_MAGIC, PACK_MAGIC};
        let id = crate::id::hash_content(name.as_bytes());
        let data = [0x28, 0xb5, 0x2f, 0xfd, 0xff, 0xff, 0xff, 0xff];
        let stored = data.len() as u64;
        let mut index = Writer::new();
        index.varint(1);
        index.id(&id);
        index.u8(ObjectKind::Content as u8);
        index.u64(16);
        index.varint(claimed);
        index.varint(stored);
        let index = index.into_bytes();
        let mut head = Writer::new();
        head.u8(ObjectKind::Content as u8);
        head.id(&id);
        head.u64(claimed);
        head.u64(stored);
        head.u8(1);
        let mut pack = PACK_MAGIC.to_vec();
        pack.extend_from_slice(&1u32.to_le_bytes());
        pack.extend_from_slice(&0u32.to_le_bytes());
        pack.extend_from_slice(&head.into_bytes());
        pack.extend_from_slice(&data);
        let at = pack.len() as u64;
        pack.extend_from_slice(&index);
        pack.extend_from_slice(&at.to_le_bytes());
        pack.extend_from_slice(blake3::hash(&index).as_bytes());
        pack.extend_from_slice(&PACK_END_MAGIC);
        std::fs::write(packs.join(name), pack).unwrap();
    }

    /// Commits an entry like the current one, with `object(i)` in its pack.
    fn commit_one(repo: &mut Repository, lock: &ChangeLock, i: usize) {
        let (head, mut entry) = repo.current().unwrap();
        entry.parent = Some(head);
        let mut packs = repo.new_packs().unwrap();
        packs.add_content(&object(i)).unwrap();
        repo.commit(lock, packs, &entry).unwrap();
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
    fn big_packs_dont_make_every_commit_merge() {
        let (_dir, mut repo) = project();
        for i in 0..33 {
            plant_claiming(repo.packs_dir(), &format!("big-{i:02}.pack"), 300 << 20);
        }
        let lock = repo.lock_for_change("test", &mut |_| {}).unwrap();
        let small = |repo: &Repository| -> Vec<String> {
            let names = pack_names(repo.packs_dir());
            assert_eq!(names.iter().filter(|n| n.starts_with("big-")).count(), 33);
            names
                .into_iter()
                .filter(|n| !n.starts_with("big-"))
                .collect()
        };
        let mut merged = None;
        for i in 1..=40 {
            commit_one(&mut repo, &lock, i);
            let now = small(&repo);
            // The first save's pack and one a commit: merged only once there are 33.
            let expected = if i < 32 { 1 + i } else { i - 31 };
            assert_eq!(now.len(), expected, "commit {i}");
            if i == 32 {
                merged = now.first().cloned();
            }
            if i > 32 {
                assert!(
                    now.contains(merged.as_ref().unwrap()),
                    "commit {i} merged again"
                );
            }
        }
        assert!(
            repo.store().warnings().is_empty(),
            "{:?}",
            repo.store().warnings()
        );
    }

    #[test]
    fn a_pack_claiming_too_much_is_neither_merged_nor_read() {
        let (_dir, mut repo) = project();
        let lock = add_packs(&mut repo, 33);
        // A few bytes on disk claiming 1 TiB: read, it would be damage.
        plant_claiming(repo.packs_dir(), "claims.pack", 1 << 40);
        let report = merge_small_packs(&mut repo, &lock).unwrap().unwrap();
        assert_eq!(report.merged, 34);
        assert!(report.kept_damaged.is_empty(), "{report:?}");
        assert!(report.kept_unreadable.is_empty(), "{report:?}");
        assert!(repo.packs_dir().join("claims.pack").exists());
        assert_eq!(pack_names(repo.packs_dir()).len(), 2);
        assert!(repo.store().warnings().is_empty());
        assert_all_readable(&repo, 33, "after the merge");
    }

    #[cfg(feature = "fault-injection")]
    #[test]
    fn a_pack_that_cant_be_read_is_left_and_named_and_the_merge_goes_on() {
        use crate::fsio::fault;
        let (_dir, mut repo) = project();
        let lock = add_packs(&mut repo, 33);
        let packs = repo.packs_dir().to_owned();
        let first = pack_names(&packs).remove(0);
        let (from, away) = (packs.join(&first), packs.join("away"));
        // Gone when the merge comes to read it: an I/O error, not damage.
        fault::act_at("merge:copying", move || {
            std::fs::rename(from, away).unwrap()
        });
        merge_after_commit(&mut repo, &lock);
        let warnings = repo.store().warnings();
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert_eq!(warnings[0].kind, warning::PACK_NOT_MERGED);
        assert!(warnings[0].message.contains(&first), "{warnings:?}");
        assert!(
            warnings[0].message.contains("couldn't read"),
            "{warnings:?}"
        );
        // The rest were merged.
        assert_eq!(pack_names(&packs).len(), 1);
        std::fs::rename(packs.join("away"), packs.join(&first)).unwrap();
        assert_all_readable(&repo, 33, "after the merge");
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
