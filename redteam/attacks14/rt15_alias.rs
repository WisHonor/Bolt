//! RT15 exploration (decision 024), run only in the sandbox, copied into crates/opal-vcs/tests/
//! by `redteam/attacks14/run.sh explore rt15_alias`. Not a proof test: it reports what it finds.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use opal_core::id::{Flavor, hash_record};
use opal_core::listing::{Entry, EntryKind, Listing};
use opal_core::names::{is_git_alias, is_opal_alias, validate_name, writable_here};
use opal_core::pack::{ObjectKind, Prepared};
use opal_core::repo::Repository;
use support::{Env, add_pack};

fn hfs_ignores(c: char) -> bool {
    matches!(
        c,
        '\u{200C}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{206A}'..='\u{206F}' | '\u{FEFF}'
    )
}

/// HFS+ (TN1150): the ignorable code points dropped, ASCII letters in any case. Its NFD step adds
/// nothing here: the only canonical singletons onto ASCII are U+212A (K), U+037E (;) and U+1FEF
/// (`), none of them in `.opalro`.
fn hfs_takes(name: &str) -> bool {
    let kept: String = name.chars().filter(|&c| !hfs_ignores(c)).collect();
    kept.eq_ignore_ascii_case(".opalro")
}

/// NTFS as Git models it for a name of six letters or more (`is_ntfs_dot_generic`): `.opalro`, or
/// the plain short names `opalro~1` to `opalro~4`, then only spaces and dots, then the end or a
/// `:stream`. Checked on the part before the first `\` (Opalro's rule) and, separately, on every
/// part after a `\` (Git for Windows' rule for `.git`).
fn ntfs_takes(part: &str) -> bool {
    let b = part.as_bytes();
    let rest = if b.len() >= 7 && b[0] == b'.' && b[1..7].eq_ignore_ascii_case(b"opalro") {
        &b[7..]
    } else if b.len() >= 8
        && b[..6].eq_ignore_ascii_case(b"opalro")
        && b[6] == b'~'
        && (b'1'..=b'4').contains(&b[7])
    {
        &b[8..]
    } else {
        return false;
    };
    for &c in rest {
        if c == b':' {
            return true;
        }
        if c != b' ' && c != b'.' {
            return false;
        }
    }
    true
}

const PIECES: [&str; 34] = [
    ".", "o", "O", "p", "P", "a", "A", "l", "L", "r", "R", "~", "1", "4", "5", " ", ":", "\\",
    "/", "x", "\u{200C}", "\u{FEFF}", "\u{206F}", "\u{202E}", "\u{200B}", "\u{2060}", "opalro",
    ".opalro", "OPALRO~", "ignore", "$INDEX_ALLOCATION", "\u{0130}", "\u{212A}", "\u{FFFF}",
];

fn check(name: &str, missed: &mut Vec<String>, seen: &mut usize) {
    let first = name.split(['/', '\\']).next().unwrap_or(name);
    let drive = hfs_takes(name.split('/').next().unwrap_or(name)) || ntfs_takes(first);
    if !drive {
        return;
    }
    *seen += 1;
    let refused = is_opal_alias(name)
        && validate_name(name).is_err()
        && writable_here(name, &[]).is_err();
    if !refused {
        missed.push(format!("{name:?}"));
    }
}

#[test]
fn opal_alias_against_the_drives_rules() {
    let (mut missed, mut seen, mut tried) = (Vec::new(), 0usize, 0usize);
    let n = PIECES.len();
    for a in 0..n {
        for b in 0..=n {
            for c in 0..=n {
                let mut name = String::from(PIECES[a]);
                if b < n {
                    name.push_str(PIECES[b]);
                }
                if c < n {
                    name.push_str(PIECES[c]);
                }
                tried += 1;
                check(&name, &mut missed, &mut seen);
            }
        }
    }
    // Spellings built around the real name: each ignorable, case and NTFS ending at each place.
    let mut state = 0x9e37_79b9_7f4a_7c15u64;
    let mut next = |m: usize| {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        (state % m as u64) as usize
    };
    let base: Vec<char> = ".opalro".chars().collect();
    for _ in 0..400_000 {
        let mut name: Vec<char> = base
            .iter()
            .map(|c| if next(2) == 0 { c.to_ascii_uppercase() } else { *c })
            .collect();
        if next(3) == 0 {
            name = format!("opalro~{}", 1 + next(9)).chars().collect();
        }
        for _ in 0..next(4) {
            let at = next(name.len() + 1);
            let c = ['\u{200C}', '\u{200D}', '\u{200E}', '\u{200F}', '\u{202A}', '\u{202E}',
                '\u{206A}', '\u{206F}', '\u{FEFF}', '\u{200B}', '\u{00AD}', '\u{034F}'][next(12)];
            name.insert(at, c);
        }
        let mut name: String = name.into_iter().collect();
        for _ in 0..next(4) {
            name.push([' ', '.', ':', '\\', 'x'][next(5)]);
        }
        if next(4) == 0 {
            name.push_str("$INDEX_ALLOCATION");
        }
        tried += 1;
        check(&name, &mut missed, &mut seen);
    }
    println!("tried {tried} names, {seen} a drive takes for .opalro, missed {}", missed.len());
    // Informational: what Git's rule for `.git` (every part after a `\`) would add.
    let after_backslash = ["x\\.opalro", "x\\OPALRO~1", "a\\b\\.opalro."]
        .map(|n| (n, is_opal_alias(n), writable_here(n, &[]).is_ok()));
    println!("names with .opalro after a backslash (name, alias, writable here): {after_backslash:?}");
    assert!(missed.is_empty(), "{missed:#?}");
}

#[test]
fn context_strings_are_distinct() {
    let all = [
        Flavor::FolderListing,
        Flavor::Save,
        Flavor::ChunkList,
        Flavor::ProjectLink,
        Flavor::Conflict,
        Flavor::DiaryEntry,
        Flavor::TagList,
        Flavor::ServerLines,
    ];
    let contexts: Vec<&str> = all.iter().map(|f| f.context()).collect();
    for (i, a) in contexts.iter().enumerate() {
        for b in &contexts[i + 1..] {
            assert_ne!(a, b);
        }
        assert!(a.starts_with("opal-vcs 20"), "{a}");
    }
    println!("{contexts:#?}");
}

/// A record stored under the ID a pre-release build gave it (context `bolt-vcs …`) is never read
/// as that record.
#[test]
fn a_record_under_its_old_context_id_is_refused() {
    let env = Env::empty();
    std::fs::write(env.root().join("a.txt"), b"a").unwrap();
    env.opalro()
        .args(["start", "--preset", "custom"])
        .assert()
        .success();
    let listing = Listing::new(vec![Entry {
        name: "a.txt".into(),
        kind: EntryKind::File,
        id: opal_core::id::hash_content(b"a"),
        size: 1,
    }])
    .unwrap()
    .encode();
    let old = opal_core::id::Id(blake3::derive_key(
        "bolt-vcs 2026-10-31 folder-listing v1",
        &listing,
    ));
    let new = hash_record(Flavor::FolderListing, &listing);
    assert_ne!(old, new);
    add_pack(env.root(), |pack| {
        let p = Prepared {
            id: old,
            size: listing.len() as u64,
            zstd: false,
            stored: listing.clone(),
        };
        pack.add_prepared(ObjectKind::FolderListing, &p).unwrap();
    });
    let repo = Repository::open(env.root()).unwrap();
    let read = repo.store().read_record(&old, ObjectKind::FolderListing);
    println!("reading the record under its old-context ID: {read:?}");
    assert!(read.is_err());
    let out = env.opalro().arg("verify").output().unwrap();
    println!(
        "verify: {:?}\n{}{}",
        out.status.code(),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(is_git_alias(".git"));
}
