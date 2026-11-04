//! The first save after an import stops at files that differ from it only in line endings: Git
//! on Windows writes LF as CRLF and stores LF, and Bolt keeps bytes as they are (decision 039).

use std::collections::HashMap;
use std::fs::File;
use std::io::{self, Read, Write};
use std::path::Path;

use super::{Folders, Kind, NAMED, SaveError, open_project_file, stamp};
use crate::listing::{Entry, EntryKind, Listing};
use crate::store::Store;

/// How many saved bytes are held up against the file at a time.
const PIECE: usize = 64 << 10;

/// Fails, naming the first few, if any file in `folders` is the base save's file with each LF
/// turned into CRLF. `old` is what the base save holds in each folder. Entries kept as they were
/// are not files in `folders`, so they are never looked at.
pub(super) fn check(
    store: &Store,
    folders: &Folders,
    old: &HashMap<String, Option<Listing>>,
) -> Result<(), SaveError> {
    let mut count = 0;
    let mut first = Vec::new();
    for (rel, children) in folders {
        let Some(before) = old.get(rel).and_then(Option::as_ref) else {
            continue;
        };
        for c in children.iter().filter(|c| c.kind == Kind::File) {
            let saved = before
                .get(&c.name)
                .filter(|e| matches!(e.kind, EntryKind::File | EntryKind::Executable));
            let Some(was) = saved else {
                continue;
            };
            if converted(store, was, &c.path).map_err(|e| e.at(&c.rel))? {
                count += 1;
                if first.len() < NAMED {
                    first.push(c.rel.clone());
                }
            }
        }
    }
    if count == 0 {
        Ok(())
    } else {
        Err(SaveError::LineEndingsChanged { count, first })
    }
}

/// Whether the file at `path` is exactly the saved bytes of `was` with every LF turned into CRLF.
fn converted(store: &Store, was: &Entry, path: &Path) -> Result<bool, SaveError> {
    let unreadable = |source| SaveError::Unreadable {
        path: path.display().to_string(),
        source,
    };
    let file = open_project_file(path)?;
    let before = stamp(&file).map_err(unreadable)?;
    // Each LF adds a byte, so a conversion is longer, and at most twice as long.
    if before.0 <= was.size || before.0 > was.size.saturating_mul(2) {
        return Ok(false);
    }
    let mut compare = Compare::new(&file);
    let copied = store.copy_content(&was.id, &mut compare);
    match compare.stop {
        Some(Stop::Differs) => return Ok(false),
        Some(Stop::Unreadable(source)) => return Err(unreadable(source)),
        // Not ended on purpose, so if the copy failed, the store did.
        None => copied?,
    }
    let ended = at_end(&file).map_err(unreadable)?;
    if stamp(&file).map_err(unreadable)? != before {
        return Err(SaveError::ChangedDuringSave {
            path: path.display().to_string(),
        });
    }
    Ok(ended)
}

/// Whether nothing is left to read in `file`.
fn at_end(mut file: &File) -> io::Result<bool> {
    match file.read_exact(&mut [0; 1]) {
        Ok(()) => Ok(false),
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => Ok(true),
        Err(e) => Err(e),
    }
}

/// Why a comparison ended before the saved bytes did.
enum Stop {
    /// The file is not the saved bytes with CRLF line endings, or those bytes hold a CR.
    Differs,
    /// The file could not be read.
    Unreadable(io::Error),
}

/// Takes the saved bytes as the store streams them and holds each up against the file, with every
/// LF read as CRLF. The first difference ends the copy.
struct Compare<'a> {
    file: &'a File,
    /// Why the copy was ended, if it was. The store sees only an I/O error; this says it was
    /// meant.
    stop: Option<Stop>,
    want: Vec<u8>,
    have: Vec<u8>,
}

impl<'a> Compare<'a> {
    fn new(file: &'a File) -> Compare<'a> {
        Compare {
            file,
            stop: None,
            want: Vec::new(),
            have: Vec::new(),
        }
    }

    fn halt(&mut self, why: Stop) -> io::Result<usize> {
        self.stop = Some(why);
        Err(io::Error::other("the comparison is over"))
    }
}

impl Write for Compare<'_> {
    fn write(&mut self, saved: &[u8]) -> io::Result<usize> {
        for piece in saved.chunks(PIECE) {
            self.want.clear();
            for &byte in piece {
                match byte {
                    // Converting a CR as well would not give back what Git wrote.
                    b'\r' => return self.halt(Stop::Differs),
                    b'\n' => self.want.extend_from_slice(b"\r\n"),
                    _ => self.want.push(byte),
                }
            }
            self.have.resize(self.want.len(), 0);
            match self.file.read_exact(&mut self.have) {
                Ok(()) if self.have == self.want => {}
                Ok(()) => return self.halt(Stop::Differs),
                Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => {
                    return self.halt(Stop::Differs);
                }
                Err(e) => return self.halt(Stop::Unreadable(e)),
            }
        }
        Ok(saved.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;
    use crate::id::hash_content;
    use crate::listing::SaveLimits;
    use crate::snapshot::tests::{project, save_now, saved_root, walk_project, write};
    use crate::test_support::{add_listing, plant_root, plant_save};
    use proptest::prelude::*;

    /// `saved` with every LF turned into CRLF.
    fn expand(saved: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        for &byte in saved {
            if byte == b'\n' {
                out.push(b'\r');
            }
            out.push(byte);
        }
        out
    }

    /// What `check` says when the last save held `f.txt` as `saved` and the folder holds `disk`.
    fn checked(saved: &[u8], disk: &[u8]) -> Result<(), SaveError> {
        let (dir, mut repo) = project();
        write(dir.path(), "f.txt", saved);
        saved_root(save_now(&mut repo).unwrap());
        write(dir.path(), "f.txt", disk);
        let scan = walk_project(&repo, SaveLimits::default()).unwrap();
        let result = check(repo.store(), &scan.folders, &scan.old);
        // Ending a copy on purpose is no damage and no warning.
        assert!(repo.store().warnings().is_empty());
        result
    }

    fn found(result: Result<(), SaveError>) -> (u64, Vec<String>) {
        match result {
            Err(SaveError::LineEndingsChanged { count, first }) => (count, first),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_file_with_every_lf_turned_into_crlf_is_found() {
        let found = found(checked(b"one\ntwo\n", b"one\r\ntwo\r\n"));
        assert_eq!(found, (1, vec!["f.txt".to_owned()]));
    }

    #[test]
    fn anything_else_passes() {
        let saved: &[u8] = b"one\ntwo\nthree\n";
        for (what, before, now) in [
            ("unchanged", saved, saved),
            // Its own conversion, but unchanged: it has no line endings.
            ("unchanged without line endings", b"abc", b"abc"),
            ("some of the line endings", saved, b"one\r\ntwo\nthree\r\n"),
            ("a line more", saved, b"one\r\ntwo\r\nthree\r\nfour\r\n"),
            ("cut short", saved, b"one\r\ntwo\r\nthree"),
            ("edited", saved, b"ONE\r\ntwo\r\nthree\r\n"),
            ("a CR to begin with", b"a\r\nb\r\n", b"a\r\r\nb\r\r\n"),
            ("empty to begin with", b"", b"\r\n"),
            ("shorter", saved, b"one"),
        ] {
            checked(before, now).expect(what);
        }
    }

    /// Each LF adds one byte, so a file of nothing but line endings doubles, and no conversion
    /// grows more than that.
    #[test]
    fn a_conversion_is_at_most_twice_as_long_as_what_was_saved() {
        let found = found(checked(b"\n\n", b"\r\n\r\n"));
        assert_eq!(found, (1, vec!["f.txt".to_owned()]));
        // One byte more is no conversion, and it is decided from the sizes alone: the store lacks
        // the saved bytes, so any read of them would fail the check.
        let (dir, mut repo) = project();
        plant_root(&mut repo, |packs| {
            let lost = Entry {
                name: "f.txt".into(),
                kind: EntryKind::File,
                id: hash_content(b"\n\n"),
                size: 2,
            };
            add_listing(packs, vec![lost]).0
        });
        write(dir.path(), "f.txt", b"\r\n\r\nx");
        let scan = walk_project(&repo, SaveLimits::default()).unwrap();
        check(repo.store(), &scan.folders, &scan.old).unwrap();
    }

    #[test]
    fn executables_are_compared_and_other_kinds_are_not() {
        let (dir, mut repo) = project();
        plant_save(
            &mut repo,
            &[
                ("run.sh", EntryKind::Executable, b"a\nb\n"),
                ("link", EntryKind::Symlink, b"a\nb\n"),
            ],
        );
        write(dir.path(), "run.sh", b"a\r\nb\r\n");
        write(dir.path(), "link", b"a\r\nb\r\n");
        let scan = walk_project(&repo, SaveLimits::default()).unwrap();
        let found = found(check(repo.store(), &scan.folders, &scan.old));
        assert_eq!(found, (1, vec!["run.sh".to_owned()]));
    }

    #[test]
    fn the_error_names_five_files_and_counts_the_rest() {
        let (dir, mut repo) = project();
        for n in 1..=7 {
            write(dir.path(), &format!("sub/f{n}.txt"), b"a\nb\n");
        }
        saved_root(save_now(&mut repo).unwrap());
        for n in 1..=7 {
            write(dir.path(), &format!("sub/f{n}.txt"), b"a\r\nb\r\n");
        }
        let scan = walk_project(&repo, SaveLimits::default()).unwrap();
        let err = check(repo.store(), &scan.folders, &scan.old).unwrap_err();
        assert_eq!(
            err.to_string(),
            "7 files differ from what Git stored only in line endings: Git turned LF into CRLF \
             when it wrote them out (core.autocrlf).\n  sub/f1.txt\n  sub/f2.txt\n  \
             sub/f3.txt\n  sub/f4.txt\n  sub/f5.txt\n  and 2 more"
        );
    }

    #[test]
    fn one_file_is_said_in_the_singular() {
        let e = SaveError::LineEndingsChanged {
            count: 1,
            first: vec!["a.txt".into()],
        };
        assert_eq!(
            e.to_string(),
            "1 file differs from what Git stored only in line endings: Git turned LF into CRLF \
             when it wrote it out (core.autocrlf).\n  a.txt"
        );
    }

    /// Line endings fall just before, on and after the edge of a piece.
    #[test]
    fn long_files_are_compared_to_the_last_byte() {
        let mut saved = vec![b'x'; 3 * PIECE + 10];
        for at in (0..saved.len()).step_by(997) {
            saved[at] = b'\n';
        }
        for at in [
            PIECE - 1,
            PIECE,
            PIECE + 1,
            2 * PIECE - 1,
            2 * PIECE,
            saved.len() - 1,
        ] {
            saved[at] = b'\n';
        }
        let converted = expand(&saved);
        assert_eq!(found(checked(&saved, &converted)).0, 1);

        let mut last = converted.clone();
        *last.last_mut().unwrap() = b'y';
        checked(&saved, &last).unwrap();
        let mut second_piece = converted.clone();
        second_piece[PIECE + 100] = b'y';
        checked(&saved, &second_piece).unwrap();
        // A CR late in the saved bytes rules the file out, whatever came before it.
        let mut with_cr = saved.clone();
        with_cr[3 * PIECE] = b'\r';
        checked(&with_cr, &expand(&with_cr)).unwrap();
    }

    #[test]
    fn a_saved_file_the_store_lacks_stops_the_save() {
        let (dir, mut repo) = project();
        plant_root(&mut repo, |packs| {
            let lost = Entry {
                name: "f.txt".into(),
                kind: EntryKind::File,
                id: hash_content(b"a\nb\n"),
                size: 4,
            };
            add_listing(packs, vec![lost]).0
        });
        write(dir.path(), "f.txt", b"a\r\nb\r\n");
        let scan = walk_project(&repo, SaveLimits::default()).unwrap();
        // Not taken for a file that differs: the store failed.
        let err = check(repo.store(), &scan.folders, &scan.old).unwrap_err();
        assert!(matches!(err, SaveError::Repo(_)), "{err:?}");
    }

    /// Whether `Compare`, given `saved` in pieces of the lengths in `cuts` (round and round), and
    /// then the end of the file, finds `disk` to be it with CRLF line endings.
    fn agrees(saved: &[u8], disk: &[u8], cuts: &[usize]) -> bool {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("f");
        std::fs::write(&path, disk).unwrap();
        let file = File::open(&path).unwrap();
        let mut compare = Compare::new(&file);
        let mut rest = saved;
        for &len in cuts.iter().cycle() {
            if rest.is_empty() {
                break;
            }
            let (piece, tail) = rest.split_at(len.min(rest.len()));
            if compare.write_all(piece).is_err() {
                return false;
            }
            rest = tail;
        }
        at_end(&file).unwrap()
    }

    /// The store writes at most a piece at a time, so only a direct write of more reaches the
    /// split inside `write`.
    #[test]
    fn one_write_of_many_pieces_is_compared_piece_by_piece() {
        let mut saved = vec![b'x'; 2 * PIECE + 100];
        for at in [PIECE - 1, PIECE, 2 * PIECE - 1, 2 * PIECE, saved.len() - 1] {
            saved[at] = b'\n';
        }
        let disk = expand(&saved);
        let whole = [saved.len()];
        assert!(agrees(&saved, &disk, &whole));
        // A difference in any one of the three pieces is found.
        for at in [10, PIECE + 10, disk.len() - 10] {
            let mut changed = disk.clone();
            changed[at] ^= 1;
            assert!(!agrees(&saved, &changed, &whole), "byte {at}");
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]
        #[test]
        fn the_comparison_agrees_with_converting_the_saved_bytes(
            saved in prop::collection::vec(
                prop::sample::select(vec![b'a', b'b', b'\n', b'\n', b'\r']),
                0..200,
            ),
            change in 0..4usize,
            at in any::<prop::sample::Index>(),
            cuts in prop::collection::vec(1..40usize, 1..6),
        ) {
            let mut disk = expand(&saved);
            match change {
                1 if !disk.is_empty() => {
                    let i = at.index(disk.len());
                    disk[i] ^= 1;
                }
                2 => {
                    disk.pop();
                }
                3 => disk.push(b'a'),
                _ => {}
            }
            let want = disk == expand(&saved) && !saved.contains(&b'\r');
            prop_assert_eq!(agrees(&saved, &disk, &cuts), want);
        }
    }
}
