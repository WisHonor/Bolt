//! Links: a link's target as a save stores it (docs/storage-format.md section 3), and project
//! links (section 7).

use crate::encoding::{DecodeError, Reader, Writer};
use crate::id::{Flavor, Id, hash_record};

/// A link's target, read from disk, as a save stores it: with `/` between its parts, as Git
/// stores it (decision 045). Windows separates them with `\`; elsewhere `\` is an ordinary
/// character, and the target is stored as read.
pub(crate) fn stored_target(read: String) -> String {
    if cfg!(windows) {
        read.replace('\\', "/")
    } else {
        read
    }
}

/// A link to a commit of another project, which Opalro never fetches. So far the other project is
/// always a Git submodule, so the record's system is always `git`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectLink {
    pub url: String,
    /// The commit's raw ID: 20 bytes, or 32 in a SHA-256 repository.
    pub commit: Vec<u8>,
}

impl ProjectLink {
    /// A submodule's link as an import makes it, without an address: that stays in
    /// `.gitmodules` (decision 036).
    #[must_use]
    pub fn submodule(commit: &[u8]) -> ProjectLink {
        ProjectLink {
            url: String::new(),
            commit: commit.to_vec(),
        }
    }

    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        w.u8(1);
        w.string("git");
        w.string(&self.url);
        w.bytes(&self.commit);
        w.into_bytes()
    }

    pub fn decode(bytes: &[u8]) -> Result<ProjectLink, DecodeError> {
        let mut r = Reader::new(bytes);
        if r.u8()? != 1 {
            return Err(DecodeError::Invalid("unknown project-link version"));
        }
        // No field can be longer than the record, which the pack limits.
        if r.string(bytes.len(), "system")? != "git" {
            return Err(DecodeError::Invalid("unknown project-link system"));
        }
        let url = r.string(bytes.len(), "url")?.to_owned();
        let commit = r.bytes(32, "commit ID")?.to_vec();
        if commit.len() != 20 && commit.len() != 32 {
            return Err(DecodeError::Invalid("a Git commit ID is 20 or 32 bytes"));
        }
        r.finish()?;
        Ok(ProjectLink { url, commit })
    }

    #[must_use]
    pub fn id(&self) -> Id {
        hash_record(Flavor::ProjectLink, &self.encode())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{hex, unhex};

    #[test]
    fn a_submodule_link_bytes() {
        let link = ProjectLink::submodule(&[0x5a; 20]);
        assert_eq!(
            hex(&link.encode()),
            format!("0103676974{}14{}", "00", "5a".repeat(20))
        );
        assert_eq!(ProjectLink::decode(&link.encode()).unwrap(), link);
        let sha256 = ProjectLink::submodule(&[0x11; 32]);
        assert_eq!(ProjectLink::decode(&sha256.encode()).unwrap(), sha256);
    }

    #[test]
    fn decode_rejects_bad_links() {
        let link = |version: &str, system: &str, url: &str, commit: &str| {
            unhex(&format!("{version}{system}{url}{commit}"))
        };
        let git = "03676974";
        let commit = format!("14{}", "5a".repeat(20));
        let short = format!("13{}", "5a".repeat(19));
        let long = format!("21{}", "5a".repeat(33));
        let trailing = format!("{commit}00");
        for (what, bytes) in [
            ("version 2", link("02", git, "00", &commit)),
            ("another system", link("01", "03737666", "00", &commit)),
            ("a 19-byte commit", link("01", git, "00", &short)),
            ("a 33-byte commit", link("01", git, "00", &long)),
            ("a url that isn't UTF-8", link("01", git, "01ff", &commit)),
            ("a trailing byte", link("01", git, "00", &trailing)),
            ("cut short", link("01", git, "00", "14")),
        ] {
            assert!(ProjectLink::decode(&bytes).is_err(), "{what}");
        }
    }
}
