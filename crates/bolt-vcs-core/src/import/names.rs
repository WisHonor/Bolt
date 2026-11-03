//! The names Git's refs take in Bolt (decision 033): the limits on how many lines, servers and
//! tags an import brings in, and the fixed rule that renames the names Bolt's rules refuse.
#![deny(clippy::indexing_slicing, clippy::arithmetic_side_effects)]

use std::collections::{BTreeMap, HashMap, HashSet};

use super::{ImportError, Renaming, lossy};
use crate::diary::{MAX_LINES, MAX_SERVERS, MAX_TAGS, is_line_name_char};
use crate::git::{GitError, GitId, Refs, shown};
use crate::names::MAX_NAME_BYTES;

/// A ref an import brings in: its Git name, the name it takes in Bolt, and what it points at.
pub(super) struct Named {
    pub(super) git: Vec<u8>,
    pub(super) bolt: String,
    pub(super) target: GitId,
}

/// Which of Bolt's rules a name must pass.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Rules {
    /// Line and server names: decision 012's characters, and no two alike but for letter case.
    Line,
    /// Tag names: any UTF-8, and no two the same.
    Tag,
}

/// The refs an import brings in, by the names they take in Bolt.
pub(super) struct Names {
    pub(super) lines: Vec<Named>,
    /// Each server's lines, by the server's Bolt name.
    pub(super) servers: BTreeMap<String, Vec<Named>>,
    pub(super) tags: Vec<Named>,
    pub(super) renamed: Vec<Renaming>,
}

impl Names {
    /// Names every ref Bolt takes, once decision 033's limits are checked.
    pub(super) fn of(refs: &Refs) -> Result<Names, ImportError> {
        let too_many = |what: &str| -> ImportError {
            GitError::TooManyRefs {
                what: what.to_owned(),
            }
            .into()
        };
        if refs.branches.len() as u64 > MAX_LINES {
            return Err(too_many(
                "it has more than 100,000 branches, and Bolt holds at most 100,000 lines",
            ));
        }
        let mut by_server: BTreeMap<&[u8], Vec<(Vec<u8>, GitId)>> = BTreeMap::new();
        for (server, branch, id) in &refs.servers {
            let own = by_server.entry(server.as_slice()).or_default();
            own.push((branch.clone(), *id));
        }
        if by_server.len() as u64 > MAX_SERVERS {
            return Err(too_many(
                "it has branches from more than 64 servers, and Bolt holds at most 64",
            ));
        }
        if let Some((server, _)) = by_server
            .iter()
            .find(|(_, own)| own.len() as u64 > MAX_LINES)
        {
            return Err(too_many(&format!(
                "server `{}` has more than 100,000 branches, and Bolt holds at most 100,000 \
                 lines for one server",
                shown(server)
            )));
        }
        // Every tag ref counts, the ones Bolt can't hold too: each costs a look.
        if refs.tags.len() as u64 > MAX_TAGS {
            return Err(too_many(
                "it has more than 100,000 tags, and Bolt holds at most 100,000",
            ));
        }
        let mut renamed = Vec::new();
        let lines = named(&refs.branches, Rules::Line, "line", &mut renamed);
        let server_names: Vec<&[u8]> = by_server.keys().copied().collect();
        let mut servers = BTreeMap::new();
        for ((git, own), bolt) in by_server.iter().zip(rename(&server_names, Rules::Line)) {
            if bolt.as_bytes() != *git {
                renamed.push(Renaming {
                    kind: "server",
                    git: lossy(git),
                    bolt: bolt.clone(),
                });
            }
            let mut own_renamed = Vec::new();
            let own = named(own, Rules::Line, "server_line", &mut own_renamed);
            // Shown as each tool shows a server's branch.
            renamed.extend(own_renamed.into_iter().map(|line| Renaming {
                git: format!("{}/{}", lossy(git), line.git),
                bolt: format!("{}@{bolt}", line.bolt),
                ..line
            }));
            servers.insert(bolt, own);
        }
        let tags = named(&refs.tags, Rules::Tag, "tag", &mut renamed);
        Ok(Names {
            lines,
            servers,
            tags,
            renamed,
        })
    }
}

/// `refs`, in the byte order of their names, with the names they take in Bolt under `rules`;
/// each one renamed is noted as `kind`.
fn named(
    refs: &[(Vec<u8>, GitId)],
    rules: Rules,
    kind: &'static str,
    renamed: &mut Vec<Renaming>,
) -> Vec<Named> {
    let names: Vec<&[u8]> = refs.iter().map(|(name, _)| name.as_slice()).collect();
    let bolt = rename(&names, rules);
    refs.iter()
        .zip(bolt)
        .map(|((git, target), bolt)| {
            if bolt.as_bytes() != git.as_slice() {
                renamed.push(Renaming {
                    kind,
                    git: lossy(git),
                    bolt: bolt.clone(),
                });
            }
            Named {
                git: git.clone(),
                bolt,
                target: *target,
            }
        })
        .collect()
}

/// Decision 033's renaming rule over `names`, which come in byte order. Each character `rules`
/// refuse becomes `-`, as do bytes that aren't UTF-8 wherever U+FFFD would stand for them, and a
/// name over 255 bytes is cut at a character. Names that need neither keep them and are given
/// first; of names alike (for lines, alike but for letter case), the first in byte order. Then
/// every other name, in byte order, gets its new name unless a name given already is alike, and
/// else the first of `-2`, `-3`… that leaves it free. The same names always come out the same.
pub(super) fn rename(names: &[&[u8]], rules: Rules) -> Vec<String> {
    let fold = |name: &str| match rules {
        Rules::Line => name.to_lowercase(),
        Rules::Tag => name.to_owned(),
    };
    let bases: Vec<String> = names.iter().map(|name| base(name, rules)).collect();
    let mut taken = HashSet::with_capacity(names.len());
    let kept: Vec<bool> = names
        .iter()
        .zip(&bases)
        .map(|(name, base)| base.as_bytes() == *name && taken.insert(fold(base)))
        .collect();
    let mut tried = HashMap::new();
    bases
        .into_iter()
        .zip(kept)
        .map(|(base, kept)| {
            if kept || taken.insert(fold(&base)) {
                base
            } else {
                suffixed(&base, &fold, &mut taken, &mut tried)
            }
        })
        .collect()
}

/// `name` as `rules` take it, before any suffix: each character they refuse becomes `-`, and a
/// name over 255 bytes is cut.
fn base(name: &[u8], rules: Rules) -> String {
    let mut whole = String::with_capacity(name.len());
    for chunk in name.utf8_chunks() {
        for c in chunk.valid().chars() {
            let allowed = rules == Rules::Tag || is_line_name_char(c);
            whole.push(if allowed { c } else { '-' });
        }
        if !chunk.invalid().is_empty() {
            whole.push('-');
        }
    }
    cut(&whole, MAX_NAME_BYTES).to_owned()
}

/// `base` with the first suffix, `-2`, `-3` and so on, that leaves no name given alike, `base`
/// cut to make room for it; the name is then taken. A suffix of `digits` digits always follows
/// the same start of `base`, so `tried` keeps, for each start (as `fold` makes it) and width, the
/// next number to try: every number below it is taken, and each name is tried once however many
/// others share its start.
fn suffixed(
    base: &str,
    fold: &dyn Fn(&str) -> String,
    taken: &mut HashSet<String>,
    tried: &mut HashMap<(String, u32), u64>,
) -> String {
    let mut digits = 1u32;
    loop {
        let start = cut(
            base,
            MAX_NAME_BYTES
                .saturating_sub(digits as usize)
                .saturating_sub(1),
        );
        let past = 10u64.saturating_pow(digits);
        let n = tried
            .entry((fold(start), digits))
            .or_insert_with(|| 10u64.saturating_pow(digits.saturating_sub(1)).max(2));
        while *n < past {
            let name = format!("{start}-{n}");
            *n = n.saturating_add(1);
            if taken.insert(fold(&name)) {
                return name;
            }
        }
        digits = digits.saturating_add(1);
    }
}

/// `name`, cut to at most `max` bytes at a character, without a `/` at its end, which would make
/// an empty last part.
fn cut(name: &str, max: usize) -> &str {
    let end = name.floor_char_boundary(max);
    name.get(..end).unwrap_or(name).trim_end_matches('/')
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    #![allow(clippy::indexing_slicing, clippy::arithmetic_side_effects)]
    use super::*;
    use crate::git::{HashKind, Head};
    use proptest::prelude::*;
    use std::time::{Duration, Instant};

    /// `rename` over `names`, given in any order.
    fn renamed(names: &[&[u8]], rules: Rules) -> Vec<String> {
        let mut sorted = names.to_vec();
        sorted.sort();
        rename(&sorted, rules)
    }

    #[test]
    fn the_renaming_rule() {
        use Rules::{Line, Tag};
        // `Feature` is first in byte order, so it keeps its name, and `feature` takes a suffix.
        assert_eq!(
            renamed(&[b"main", b"feature+x", b"feature", b"Feature"], Line),
            ["Feature", "feature-2", "feature-x", "main"]
        );
        // A suffixed name must be free too.
        assert_eq!(renamed(&[b"X", b"X-2", b"x"], Line), ["X", "X-2", "x-3"]);
        // Bytes that aren't UTF-8 become `-` wherever U+FFFD would stand for them; letters of
        // any script stay.
        assert_eq!(
            renamed(
                &[
                    b"a\xff\xfeb",
                    b"a\xe2\x82b",
                    "caf\u{e9}/e\u{301}#1".as_bytes()
                ],
                Line
            ),
            ["a-b", "a--b", "caf\u{e9}/e--1"]
        );
        // Cut at a character, and never left ending in `/`.
        let long = "\u{e9}".repeat(200);
        let slashed = format!("{}/{}", "a".repeat(254), "b".repeat(10));
        let cut = renamed(&[long.as_bytes(), slashed.as_bytes()], Line);
        assert_eq!(cut, ["a".repeat(254), "\u{e9}".repeat(127)]);
        // A suffix still fits in 255 bytes.
        let (one, two) = ("a".repeat(300), format!("{}b", "a".repeat(299)));
        let alike = renamed(&[one.as_bytes(), two.as_bytes()], Line);
        assert_eq!(alike, ["a".repeat(255), format!("{}-2", "a".repeat(253))]);
        // Tags keep any UTF-8, and only names that are the same clash.
        assert_eq!(
            renamed(&[b"V1", b"v1", b"a#b@c", b"v\xff", b"v-"], Tag),
            ["V1", "a#b@c", "v-", "v1", "v--2"]
        );
        // Many names alike each get the next number.
        let alike: Vec<Vec<u8>> = (0x80..=0xffu8).map(|b| vec![b'x', b]).collect();
        let alike: Vec<&[u8]> = alike.iter().map(Vec::as_slice).collect();
        let numbered = renamed(&alike, Line);
        assert_eq!(numbered.first().map(String::as_str), Some("x-"));
        assert_eq!(numbered.last().map(String::as_str), Some("x--128"));
    }

    #[test]
    fn names_that_need_no_change_keep_them() {
        use Rules::{Line, Tag};
        // The valid name keeps it, so the renamed one takes the suffix.
        assert_eq!(
            renamed(&[b"feature+x", b"feature-x"], Line),
            ["feature-x-2", "feature-x"]
        );
        // Of names alike but for letter case, the first keeps its name; a later one takes the
        // first suffix free of every name given, `a-2` included.
        assert_eq!(renamed(&[b"A", b"a", b"a-2"], Line), ["A", "a-3", "a-2"]);
        // A renamed name that clashes with nothing keeps its new name.
        assert_eq!(renamed(&[b"feature+x"], Line), ["feature-x"]);
        assert_eq!(renamed(&[b"v\xff", b"v-"], Tag), ["v-", "v--2"]);
    }

    /// Pairs alike but for letter case, each a 251-byte start and a 4-byte letter: a suffix leaves
    /// room only for the start, which every pair shares, so each later name of a pair searches
    /// the same suffixes. Each must be tried once, not once for every pair before it.
    #[test]
    fn many_long_names_alike_are_renamed_in_linear_time() {
        let pairs = 4_000u32;
        let start = "b".repeat(250);
        let mut names: Vec<String> = Vec::new();
        for i in 0..pairs {
            let letter = char::from_u32(0x2_0000 + i).unwrap();
            names.push(format!("A{start}{letter}"));
            names.push(format!("a{start}{letter}"));
        }
        names.sort();
        let names: Vec<&[u8]> = names.iter().map(|name| name.as_bytes()).collect();
        let began = Instant::now();
        let bolt = rename(&names, Rules::Line);
        let took = began.elapsed();
        assert!(took < Duration::from_secs(1), "{took:?}");
        let (upper, lower) = bolt.split_at(pairs as usize);
        for (kept, name) in upper.iter().zip(&names) {
            assert_eq!(kept.as_bytes(), *name);
        }
        for (i, name) in lower.iter().enumerate() {
            let n = i + 2;
            let room = 255 - (n.to_string().len() + 1);
            assert_eq!(*name, format!("a{}-{n}", &start[..room.min(251) - 1]));
        }
    }

    /// A Git ref name's level: pieces Bolt allows, refuses, folds or can't read.
    fn level() -> impl Strategy<Value = Vec<u8>> {
        let piece = prop::sample::select(vec![
            &b"a"[..],
            b"A",
            b"0",
            b"2",
            b"+",
            b"#",
            b"@",
            b"-",
            "\u{e9}".as_bytes(),
            "\u{c9}".as_bytes(),
            b"\xff",
        ]);
        prop::collection::vec(piece, 1..90).prop_map(|pieces| pieces.concat())
    }

    proptest! {
        /// Whatever Git names come in, Bolt's come out valid, unlike but for letter case, and the
        /// same every time; and a name that needs no change keeps it unless one before it in
        /// byte order is alike but for letter case.
        #[test]
        fn renamed_lines_follow_bolts_rules(
            names in prop::collection::btree_set(
                prop::collection::vec(level(), 1..4).prop_map(|levels| levels.join(&b'/')),
                1..20,
            )
        ) {
            let names: Vec<&[u8]> = names.iter().map(Vec::as_slice).collect();
            let bolt = rename(&names, Rules::Line);
            let mut folded = HashSet::new();
            for name in &bolt {
                prop_assert!(crate::diary::validate_line_name(name).is_ok(), "{name}");
                prop_assert!(folded.insert(name.to_lowercase()), "{name}");
            }
            let mut first = HashSet::new();
            for (git, name) in names.iter().zip(&bolt) {
                let valid = std::str::from_utf8(git)
                    .is_ok_and(|git| crate::diary::validate_line_name(git).is_ok());
                if valid && first.insert(String::from_utf8_lossy(git).to_lowercase()) {
                    prop_assert_eq!(name.as_bytes(), *git);
                }
            }
            prop_assert_eq!(rename(&names, Rules::Line), bolt);
        }
    }

    #[test]
    fn refs_past_bolts_limits_stop_the_import() {
        let id = GitId::from_hex(HashKind::Sha1, &"1".repeat(40)).unwrap();
        let refs = |servers: usize, tags: usize| Refs {
            head: Head::Detached(id),
            branches: Vec::new(),
            servers: (0..servers)
                .map(|n| (format!("s{n:03}").into_bytes(), b"main".to_vec(), id))
                .collect(),
            tags: (0..tags)
                .map(|n| (format!("t{n:06}").into_bytes(), id))
                .collect(),
            other: Vec::new(),
        };
        assert!(Names::of(&refs(64, 100_000)).is_ok());
        for (refs, limit) in [(refs(65, 0), "64"), (refs(0, 100_001), "100,000 tags")] {
            match Names::of(&refs) {
                Err(ImportError::Git(e @ GitError::TooManyRefs { .. })) => {
                    assert!(e.to_string().contains(limit), "{e}");
                }
                other => panic!("{:?}", other.map(|_| ())),
            }
        }
    }
}
