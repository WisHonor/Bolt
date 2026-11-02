//! Integrity check: reads every object of every pack. The core of the later `bolt verify`.
use crate::pack::{ObjectKind, PackReader};
use crate::text::escape;
use std::io;
use std::path::Path;

pub struct VerifyReport {
    pub objects_checked: u64,
    pub problems: Vec<String>,
}

impl VerifyReport {
    #[must_use]
    pub fn is_ok(&self) -> bool {
        self.problems.is_empty()
    }
}

/// Opens each pack on its own, so a broken pack becomes a listed problem instead of stopping the
/// check. Folder and pack names in the problems came from outside Bolt, so they are escaped.
#[must_use]
pub fn verify_store(packs_dir: &Path) -> VerifyReport {
    let mut report = VerifyReport {
        objects_checked: 0,
        problems: Vec::new(),
    };
    let folder = escape(&packs_dir.display().to_string(), false);
    let mut paths = Vec::new();
    match std::fs::read_dir(packs_dir) {
        Ok(rd) => {
            for entry in rd {
                match entry {
                    Ok(e) if e.path().extension().is_some_and(|x| x == "pack") => {
                        paths.push(e.path());
                    }
                    Ok(_) => {}
                    Err(err) => report.problems.push(format!("{folder}: {err}")),
                }
            }
        }
        Err(err) => report.problems.push(format!("{folder}: {err}")),
    }
    paths.sort();
    for path in paths {
        let name = path
            .file_name()
            .map_or_else(String::new, |n| escape(&n.to_string_lossy(), false));
        let reader = match PackReader::open(&path) {
            Ok(r) => r,
            Err(err) => {
                report.problems.push(format!("{name}: {err}"));
                continue;
            }
        };
        for entry in reader.entries() {
            report.objects_checked += 1;
            let result = if entry.kind == ObjectKind::Content {
                reader.copy_content(entry, &mut io::sink())
            } else {
                reader.read_record(entry).map(drop)
            };
            if let Err(err) = result {
                report
                    .problems
                    .push(format!("{name}: object {}: {err}", entry.id));
            }
        }
    }
    report
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;
    use crate::pack::{ENTRY_HEADER_LEN, PackReader, PackWriter};

    fn make(
        n: usize,
    ) -> (
        tempfile::TempDir,
        std::path::PathBuf,
        Vec<std::path::PathBuf>,
    ) {
        let root = tempfile::tempdir().unwrap();
        let tmp = root.path().join("tmp");
        let packs = root.path().join("packs");
        std::fs::create_dir(&tmp).unwrap();
        std::fs::create_dir(&packs).unwrap();
        let paths = (0..n)
            .map(|i| {
                let mut w = PackWriter::create(&tmp).unwrap();
                w.add_content(format!("content number {i}").as_bytes())
                    .unwrap();
                w.add_content(&vec![i as u8; 500]).unwrap();
                w.finish(&packs).unwrap().unwrap()
            })
            .collect();
        (root, packs, paths)
    }

    #[test]
    fn verify_catches_corruption() {
        let (_root, packs, paths) = make(3);
        let report = verify_store(&packs);
        assert!(report.is_ok(), "{:?}", report.problems);
        assert_eq!(report.objects_checked, 6);

        let entry = *PackReader::open(&paths[1])
            .unwrap()
            .entries()
            .first()
            .unwrap();
        let mut bytes = std::fs::read(&paths[1]).unwrap();
        bytes[(entry.offset + ENTRY_HEADER_LEN) as usize] ^= 1;
        std::fs::write(&paths[1], bytes).unwrap();

        let report = verify_store(&packs);
        assert!(!report.is_ok());
        let name = paths[1].file_name().unwrap().to_string_lossy().into_owned();
        assert_eq!(report.problems.len(), 1, "{:?}", report.problems);
        assert!(report.problems[0].contains(&name));
    }

    #[test]
    fn problems_show_pack_names_escaped() {
        let (_root, packs, _paths) = make(0);
        std::fs::write(packs.join("a\u{202e}b.pack"), b"short").unwrap();
        let report = verify_store(&packs);
        assert_eq!(report.problems.len(), 1, "{:?}", report.problems);
        let problem = &report.problems[0];
        assert!(!problem.contains('\u{202e}'), "{problem}");
        assert!(problem.starts_with("a\\u{202e}b.pack: "), "{problem}");
    }

    #[test]
    fn verify_reports_a_broken_pack_instead_of_failing() {
        let (_root, packs, paths) = make(3);
        let len = std::fs::metadata(&paths[0]).unwrap().len();
        std::fs::OpenOptions::new()
            .write(true)
            .open(&paths[0])
            .unwrap()
            .set_len(len - 10)
            .unwrap();
        let report = verify_store(&packs);
        assert_eq!(report.problems.len(), 1, "{:?}", report.problems);
        assert_eq!(report.objects_checked, 4);
    }
}
