//! Fingerprints (`Id`) and change IDs (`ChangeId`).

use std::fmt;

/// A 32-byte BLAKE3 fingerprint.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct Id(pub [u8; 32]);

impl Id {
    #[must_use]
    pub fn to_hex(&self) -> String {
        self.0.iter().map(|b| format!("{b:02x}")).collect()
    }

    /// The first 12 hex digits, as lists and messages show an ID.
    #[must_use]
    pub fn short_hex(&self) -> String {
        let mut hex = self.to_hex();
        hex.truncate(12);
        hex
    }

    /// Exactly 64 hex characters, either case.
    #[must_use]
    pub fn from_hex(s: &str) -> Option<Id> {
        let s = s.as_bytes();
        if s.len() != 64 {
            return None;
        }
        let mut out = [0u8; 32];
        for (o, pair) in out.iter_mut().zip(s.chunks(2)) {
            let hi = char::from(pair[0]).to_digit(16)?;
            let lo = char::from(pair[1]).to_digit(16)?;
            *o = u8::try_from(hi * 16 + lo).ok()?;
        }
        Some(Id(out))
    }
}

impl fmt::Display for Id {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

/// Domain-separation flavor for record fingerprints.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Flavor {
    FolderListing,
    Save,
    ChunkList,
    ProjectLink,
    Conflict,
    DiaryEntry,
    TagList,
    ServerLines,
}

impl Flavor {
    #[must_use]
    pub fn context(self) -> &'static str {
        match self {
            Flavor::FolderListing => "bolt-vcs 2026-10-31 folder-listing v1",
            Flavor::Save => "bolt-vcs 2026-10-31 save v1",
            Flavor::ChunkList => "bolt-vcs 2026-10-31 chunk-list v1",
            Flavor::ProjectLink => "bolt-vcs 2026-10-31 project-link v1",
            Flavor::Conflict => "bolt-vcs 2026-10-31 conflict v1",
            Flavor::DiaryEntry => "bolt-vcs 2026-10-31 diary-entry v1",
            Flavor::TagList => "bolt-vcs 2026-11-02 tag-list v1",
            Flavor::ServerLines => "bolt-vcs 2026-11-02 server-lines v1",
        }
    }
}

/// Plain BLAKE3 of raw content.
#[must_use]
pub fn hash_content(bytes: &[u8]) -> Id {
    Id(*blake3::hash(bytes).as_bytes())
}

/// Context-keyed BLAKE3 of an encoded record.
#[must_use]
pub fn hash_record(flavor: Flavor, encoded: &[u8]) -> Id {
    Id(blake3::derive_key(flavor.context(), encoded))
}

/// Streaming version of `hash_content`.
#[derive(Default)]
pub struct ContentHasher(blake3::Hasher);

impl ContentHasher {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn update(&mut self, bytes: &[u8]) {
        self.0.update(bytes);
    }

    #[must_use]
    pub fn finish(self) -> Id {
        Id(*self.0.finalize().as_bytes())
    }
}

/// A random 128-bit change ID, written as 32 letters `k`-`z`.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct ChangeId(pub [u8; 16]);

impl ChangeId {
    /// A new random ID; fails if the OS random source is unavailable.
    pub fn random() -> Result<ChangeId, getrandom::Error> {
        let mut b = [0u8; 16];
        getrandom::fill(&mut b)?;
        Ok(ChangeId(b))
    }

    #[must_use]
    pub fn to_letters(&self) -> String {
        self.0
            .iter()
            .flat_map(|b| [b >> 4, b & 0x0f])
            .map(|half| char::from(b'k' + half))
            .collect()
    }

    /// Exactly 32 letters `k`-`z`.
    #[must_use]
    pub fn from_letters(s: &str) -> Option<ChangeId> {
        from_halves(s.as_bytes(), |c| c.checked_sub(b'k'))
    }

    /// The change ID Jujutsu's rule gives a Git commit without a `change-id` header: the last 16
    /// bytes of its ID, last first, with each byte's bits reversed (storage-format.md section 5).
    /// Whether an imported commit's header is used instead is the converter's to decide.
    #[must_use]
    pub fn from_git_commit(commit_id: &[u8]) -> ChangeId {
        let mut out = [0u8; 16];
        for (o, b) in out.iter_mut().zip(commit_id.iter().rev()) {
            *o = b.reverse_bits();
        }
        ChangeId(out)
    }

    /// The change ID a `change-id` header gives, if it spells 16 bytes in Jujutsu's letters.
    #[must_use]
    pub fn from_jujutsu(letters: &[u8]) -> Option<ChangeId> {
        // Jujutsu's letters run the other way, `z` for 0, and it reads them in either case.
        from_halves(letters, |c| b'z'.checked_sub(c.to_ascii_lowercase()))
    }
}

/// 32 letters as 16 bytes, two to a byte; `half` gives each letter's value, which must be under
/// 16.
fn from_halves(s: &[u8], half: impl Fn(u8) -> Option<u8>) -> Option<ChangeId> {
    if s.len() != 32 {
        return None;
    }
    let half = |c| half(c).filter(|h| *h < 16);
    let mut out = [0u8; 16];
    for (o, pair) in out.iter_mut().zip(s.chunks(2)) {
        *o = half(pair[0])? << 4 | half(pair[1])?;
    }
    Some(ChangeId(out))
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn plain_hash_matches_the_official_blake3_empty_vector() {
        assert_eq!(
            hash_content(b"").to_hex(),
            "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262"
        );
    }

    #[test]
    fn streaming_hash_equals_one_shot() {
        let mut h = ContentHasher::new();
        h.update(b"ab");
        h.update(b"c");
        assert_eq!(h.finish(), hash_content(b"abc"));
    }

    #[test]
    fn context_strings_are_exact() {
        assert_eq!(
            Flavor::FolderListing.context(),
            "bolt-vcs 2026-10-31 folder-listing v1"
        );
        assert_eq!(Flavor::Save.context(), "bolt-vcs 2026-10-31 save v1");
        assert_eq!(
            Flavor::ChunkList.context(),
            "bolt-vcs 2026-10-31 chunk-list v1"
        );
        assert_eq!(
            Flavor::ProjectLink.context(),
            "bolt-vcs 2026-10-31 project-link v1"
        );
        assert_eq!(
            Flavor::Conflict.context(),
            "bolt-vcs 2026-10-31 conflict v1"
        );
        assert_eq!(
            Flavor::DiaryEntry.context(),
            "bolt-vcs 2026-10-31 diary-entry v1"
        );
        assert_eq!(Flavor::TagList.context(), "bolt-vcs 2026-11-02 tag-list v1");
        assert_eq!(
            Flavor::ServerLines.context(),
            "bolt-vcs 2026-11-02 server-lines v1"
        );
    }

    #[test]
    fn flavors_never_collide_with_each_other_or_plain() {
        let b = b"same bytes";
        let all = [
            hash_content(b),
            hash_record(Flavor::FolderListing, b),
            hash_record(Flavor::Save, b),
            hash_record(Flavor::DiaryEntry, b),
            hash_record(Flavor::TagList, b),
            hash_record(Flavor::ServerLines, b),
        ];
        for i in 0..all.len() {
            for j in i + 1..all.len() {
                assert_ne!(all[i], all[j]);
            }
        }
    }

    #[test]
    fn change_id_letters() {
        assert_eq!(ChangeId([0x00; 16]).to_letters(), "k".repeat(32));
        assert_eq!(ChangeId([0xff; 16]).to_letters(), "z".repeat(32));
        let mut b = [0u8; 16];
        b[0] = 0x0f;
        b[1] = 0xf0;
        assert!(ChangeId(b).to_letters().starts_with("kzzk"));
        assert_eq!(ChangeId::from_letters("kka"), None);
    }

    proptest! {
        #[test]
        fn hex_round_trips(bytes in any::<[u8; 32]>()) {
            prop_assert_eq!(Id::from_hex(&Id(bytes).to_hex()), Some(Id(bytes)));
        }

        #[test]
        fn letters_round_trip(bytes in any::<[u8; 16]>()) {
            prop_assert_eq!(ChangeId::from_letters(&ChangeId(bytes).to_letters()), Some(ChangeId(bytes)));
        }
    }
}
