//! The rewritten look-alike names, Git path tidying, person lines and Git LFS pointers against
//! the old ones: the same inputs to both, the same answers.

use proptest::prelude::*;

use super::{git_path, lfs as old_lfs, lookalike, person};
use crate::{config, git, names};

fn cases() -> u32 {
    std::env::var("BOLT_COMPARE_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(2_000)
}

fn config_() -> ProptestConfig {
    ProptestConfig {
        cases: cases(),
        failure_persistence: None,
        ..ProptestConfig::default()
    }
}

const HASHED_PREFIXES: &[&[u8]] = &[
    b"gi7eba", b"gi250a", b"GI7EBA", b"gitmod", b"bo", b"g", b"", b"gi7eba~", b".git",
];
const PLAIN_PREFIXES: &[&[u8; 6]] = &[
    b"gitmod",
    b"gi7eba",
    b"GITMOD",
    b"bolt~1",
    b"gitatt",
    b"\xc3\xa9abcd",
];

// The answers of each side for one name, as text so a census can group them.
fn old_name(name: &str) -> String {
    let mut out = format!(
        "git={} bolt={}",
        lookalike::is_git_alias(name),
        lookalike::is_bolt_alias(name)
    );
    for p in HASHED_PREFIXES {
        out.push_str(&format!(
            " hashed({})={}",
            p.escape_ascii(),
            lookalike::is_hashed_short_name(name, p)
        ));
    }
    for p in PLAIN_PREFIXES {
        out.push_str(&format!(
            " plain({})={}",
            p.escape_ascii(),
            lookalike::is_plain_short_name(name, p)
        ));
    }
    out
}

fn new_name(name: &str) -> String {
    let mut out = format!(
        "git={} bolt={}",
        names::is_git_alias(name),
        names::is_bolt_alias(name)
    );
    for p in HASHED_PREFIXES {
        out.push_str(&format!(
            " hashed({})={}",
            p.escape_ascii(),
            names::compared_hashed_short_name(name, p)
        ));
    }
    for p in PLAIN_PREFIXES {
        out.push_str(&format!(
            " plain({})={}",
            p.escape_ascii(),
            names::compared_plain_short_name(name, p)
        ));
    }
    out
}

fn old_tidy(path: &[u8]) -> String {
    format!(
        "unix={:?} windows={:?}",
        git_path::tidy_path(path, false).map(|p| p.escape_ascii().to_string()),
        git_path::tidy_path(path, true).map(|p| p.escape_ascii().to_string())
    )
}

fn new_tidy(path: &[u8]) -> String {
    format!(
        "unix={:?} windows={:?}",
        config::compared_tidy_path(path, false).map(|p| p.escape_ascii().to_string()),
        config::compared_tidy_path(path, true).map(|p| p.escape_ascii().to_string())
    )
}

fn old_person(line: &[u8]) -> String {
    format!("{:?}", person::parse_person(line))
}

fn new_person(line: &[u8]) -> String {
    format!("{:?}", git::parse_person(line))
}

fn old_pointer(data: &[u8]) -> String {
    match old_lfs::lfs_pointer(data) {
        Some(p) => format!(
            "Some oid={} size={} extensions={}",
            p.oid_hex(),
            p.size,
            p.extensions
        ),
        None => "None".to_owned(),
    }
}

fn new_pointer(data: &[u8]) -> String {
    match git::lfs_pointer(data) {
        Some(p) => format!(
            "Some oid={} size={} extensions={}",
            p.oid_hex(),
            p.size,
            p.extensions
        ),
        None => "None".to_owned(),
    }
}

#[test]
fn constants_agree() {
    assert_eq!(old_lfs::MAX_LFS_POINTER_BYTES, git::MAX_LFS_POINTER_BYTES);
    assert_eq!(old_lfs::LFS_SPEC, git::LFS_SPEC);
    assert_eq!(old_lfs::LFS_SPECS, git::LFS_SPECS);
}

// Names near the ones Git and Bolt protect.
const NAME_BASES: &[&str] = &[
    ".git",
    "git",
    ".GIT",
    ".Git",
    ".gIt",
    "git~1",
    "GIT~1",
    "git~2",
    "git~10",
    "gi7eba~1",
    "GI7EBA~4",
    "gi7eba~9",
    "gi7eba~10",
    "gi7eb~1",
    ".bolt",
    "bolt",
    ".BOLT",
    "BOLT~1",
    "bolt~1",
    ".gitmodules",
    ".GITMODULES",
    "gitmodules",
    "GITMOD~1",
    "gitmod~1",
    "gitmod~12",
    "gitmod~0",
    ".gitmodul",
    ".gitattributes",
    ".gitignore",
    ".",
    "..",
    "",
    "g",
    ".g",
    "~1",
];

const NAME_PIECES: &[&str] = &[
    "\u{200c}", "\u{200d}", "\u{200e}", "\u{200f}", "\u{202a}", "\u{202b}", "\u{202c}", "\u{202d}",
    "\u{202e}", "\u{206a}", "\u{206b}", "\u{206c}", "\u{206d}", "\u{206e}", "\u{206f}", "\u{feff}",
    "\u{fffe}", "\u{ffff}", "\u{200b}", "\u{2060}", "\u{2028}", ".", " ", "  ", "..", ":",
    "::$DATA", ":$DATA", ":stream", "~", "~1", "1", "0", "9", "\\", "/", "é", "İ", "ı", "\u{212a}",
    "ſ", "\u{0}", "x", "T", "t", "\t", "G", "ⅰ",
];

fn name() -> impl Strategy<Value = String> {
    let near = (
        prop::sample::select(NAME_BASES),
        prop::collection::vec(
            (
                any::<prop::sample::Index>(),
                prop::sample::select(NAME_PIECES),
            ),
            0..4,
        ),
        prop::collection::vec(prop::sample::select(NAME_PIECES), 0..3),
        prop::collection::vec(prop::sample::select(NAME_PIECES), 0..2),
    )
        .prop_map(|(base, inserts, suffix, prefix)| {
            let mut s: Vec<char> = base.chars().collect();
            for (at, piece) in inserts {
                let at = at.index(s.len() + 1);
                for (i, c) in piece.chars().enumerate() {
                    s.insert(at + i, c);
                }
            }
            let mut out: String = prefix.concat();
            out.extend(s);
            out.push_str(&suffix.concat());
            out
        });
    prop_oneof![8 => near, 1 => any::<String>(), 1 => "[.gitGITboltBOLTmodue~0-9 :]{0,14}"]
}

const PATH_PIECES: &[&[u8]] = &[
    b"/",
    b"\\",
    b".",
    b"..",
    b"a",
    b"b",
    b"C:",
    b"c:",
    b":",
    b"//",
    b"\\\\",
    b"\xc3\xa9:",
    b"\xc3\xa9",
    b"1:",
    b" ",
    b"~",
    b"ab",
    b"\xff",
    b"\x00",
    b"?",
    b"Z:",
    b"..\\",
    b"./",
    b"/.",
    b"a:b",
];

fn path() -> impl Strategy<Value = Vec<u8>> {
    prop_oneof![
        8 => prop::collection::vec(prop::sample::select(PATH_PIECES), 0..10).prop_map(|p| p.concat()),
        1 => prop::collection::vec(any::<u8>(), 0..16),
    ]
}

const PERSON_PIECES: &[&[u8]] = &[
    b"A U Thor",
    b"Name",
    b" ",
    b"  ",
    b"\t",
    b"<",
    b">",
    b"<<",
    b">>",
    b"a@b",
    b"a@b.c",
    b"<a@b>",
    b"123",
    b"0",
    b"-1",
    b"+0100",
    b"-0130",
    b"+9999",
    b"+01",
    b"+0000000100",
    b"-0000",
    b"1700000000",
    b"99999999999999999999",
    b"18446744073709551615",
    b"9223372036854775807",
    b"9223372036854776",
    b"-9223372036854775808",
    b"0x10",
    b"+",
    b"-",
    b"\n",
    b"\r",
    b"\xff",
    b"\xc3\xa9",
    b"\xc3",
    b"\x00",
    b"1e3",
    b"+2400",
    b"-2401",
    b"+1260",
    b"0100",
    b"\xe2\x80\xae",
];

fn person_line() -> impl Strategy<Value = Vec<u8>> {
    let shaped = (
        prop::sample::select(vec![
            &b""[..],
            b"A U Thor",
            b"\xc3\xa9t\xc3\xa9",
            b" spaced ",
            b"a\xffb",
            b"x>y",
        ]),
        prop::sample::select(vec![&b" <"[..], b"<", b"  <", b" "]),
        prop::sample::select(vec![&b""[..], b"a@b", b"a<b@c", b" a@b ", b"\xff"]),
        prop::sample::select(vec![&b"> "[..], b">", b">  ", b"", b">> "]),
        prop::sample::select(vec![
            &b"1700000000"[..],
            b"0",
            b"",
            b"-5",
            b"99999999999999999999",
            b"9223372036854775",
            b"9223372036854776",
            b"12a",
        ]),
        prop::sample::select(vec![
            &b" +0100"[..],
            b" -0130",
            b"",
            b" +01",
            b" 0100",
            b" +9999",
            b" -0000",
            b"  +0100",
            b" +0100 ",
            b" +0100x",
            b" +12345",
        ]),
    )
        .prop_map(|(a, b, c, d, e, f)| [a, b, c, d, e, f].concat());
    prop_oneof![
        4 => shaped,
        4 => prop::collection::vec(prop::sample::select(PERSON_PIECES), 0..12).prop_map(|p| p.concat()),
        1 => prop::collection::vec(any::<u8>(), 0..24),
    ]
}

const HEX: &[u8] = b"0123456789abcdef";

fn hex(len: usize, seed: u64, upper: bool) -> Vec<u8> {
    let mut x = seed | 1;
    (0..len)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            let c = HEX[(x % 16) as usize];
            if upper { c.to_ascii_uppercase() } else { c }
        })
        .collect()
}

fn lfs_line() -> impl Strategy<Value = Vec<u8>> {
    let version = prop::sample::select(vec![
        &b"version https://git-lfs.github.com/spec/v1"[..],
        b"version https://hawser.github.com/spec/v1",
        b"version http://git-lfs.github.com/spec/v1",
        b"version https://git-lfs.github.com/spec/v2",
        b"version https://git-lfs.github.com/spec/v1/",
        b"version https://git-lfs.github.com/spec/v1 ",
        b"version HTTPS://GIT-LFS.GITHUB.COM/SPEC/V1",
        b"version  https://git-lfs.github.com/spec/v1",
        b"version https://hawser.github.com/spec/v2",
        b"version",
        b"version ",
        b"Version https://git-lfs.github.com/spec/v1",
    ])
    .prop_map(|v| v.to_vec());
    let oid = (
        prop::sample::select(vec![&b"oid sha256:"[..], b"oid sha256: ", b"oid  sha256:", b"oid sha1:", b"oid SHA256:", b"oid ", b"oid sha256"]),
        prop_oneof![6 => Just(64usize), 1 => Just(63), 1 => Just(65), 1 => Just(0), 1 => Just(40), 1 => Just(128)],
        any::<u64>(),
        prop::bool::weighted(0.1),
        prop::option::weighted(0.05, prop::sample::select(vec![&b"g"[..], b" ", b"\x00", b"\xc3\xa9"])),
    )
        .prop_map(|(key, len, seed, upper, junk)| {
            let mut l = key.to_vec();
            let mut h = hex(len, seed, upper);
            if let Some(j) = junk {
                let at = (seed as usize) % (h.len() + 1);
                h.splice(at..at, j.iter().copied());
            }
            l.extend(h);
            l
        });
    let size = (
        prop::sample::select(vec![&b"size "[..], b"size  ", b"size", b"Size ", b"size\t"]),
        prop::sample::select(vec![
            &b"0"[..],
            b"1",
            b"12345",
            b"007",
            b"00",
            b"+5",
            b"-5",
            b"-0",
            b"18446744073709551615",
            b"18446744073709551616",
            b"9223372036854775807",
            b"9223372036854775808",
            b"99999999999999999999999",
            b"",
            b"1 ",
            b"1x",
            b"0x10",
            b"1e3",
            b"\xd9\xa1",
        ]),
    )
        .prop_map(|(k, v)| [k, v].concat());
    let ext = (
        prop::sample::select(vec![
            &b"0"[..],
            b"1",
            b"2",
            b"9",
            b"10",
            b"00",
            b"01",
            b"a",
            b"",
            b"-1",
            b"99",
        ]),
        prop::sample::select(vec![
            &b"name"[..],
            b"a",
            b"",
            b"A",
            b"a-b",
            b"a b",
            b"\xc3\xa9",
        ]),
        prop::sample::select(vec![&b" sha256:"[..], b" sha1:", b" ", b""]),
        prop_oneof![4 => Just(64usize), 1 => Just(63), 1 => Just(0)],
        any::<u64>(),
    )
        .prop_map(|(n, name, sep, len, seed)| {
            [
                b"ext-".as_slice(),
                n,
                b"-",
                name,
                sep,
                &hex(len, seed, false),
            ]
            .concat()
        });
    let other = prop::sample::select(vec![
        &b""[..],
        b" ",
        b"x y",
        b"a",
        b"zzz value",
        b"aaa value",
        b"version",
        b"oid",
        b"size",
        b"ext",
        b"\r",
        b"key",
        b"key  value",
        b"\xff \xff",
    ])
    .prop_map(|v| v.to_vec());
    prop_oneof![2 => version, 3 => oid, 3 => size, 2 => ext, 1 => other]
}

fn pointer() -> impl Strategy<Value = Vec<u8>> {
    let canonical = (
        any::<u64>(),
        prop::sample::select(vec![&b"12345"[..], b"0", b"1", b"18446744073709551615"]),
    )
        .prop_map(|(seed, size)| {
            let mut l = b"version https://git-lfs.github.com/spec/v1\noid sha256:".to_vec();
            l.extend(hex(64, seed, false));
            l.extend(b"\nsize ");
            l.extend(size);
            l.push(b'\n');
            l
        });
    let mutated = (
        canonical.clone(),
        prop::collection::vec((any::<prop::sample::Index>(), any::<u8>()), 1..3),
        prop::bool::ANY,
    )
        .prop_map(|(mut p, edits, insert)| {
            for (at, b) in edits {
                let at = at.index(p.len() + 1);
                if insert || at == p.len() {
                    p.insert(at, b);
                } else {
                    p[at] = b;
                }
            }
            p
        });
    let built = (
        prop::bool::weighted(0.85),
        prop::collection::vec(lfs_line(), 0..6),
        prop::sample::select(vec![&b"\n"[..], b"\r\n", b"\r", b"\n\n", b" \n"]),
        prop::sample::select(vec![&b"\n"[..], b"", b"\n\n", b"\r\n", b" "]),
        prop::option::weighted(0.1, 900usize..1100),
    )
        .prop_map(|(lead, lines, eol, end, pad)| {
            let mut all: Vec<Vec<u8>> = Vec::new();
            if lead {
                all.push(b"version https://git-lfs.github.com/spec/v1".to_vec());
            }
            all.extend(lines);
            if let Some(len) = pad {
                let mut l = b"ext-0-pad sha256:".to_vec();
                l.resize(len, b'a');
                all.push(l);
            }
            let mut out = all.join(eol);
            out.extend(end);
            out
        });
    prop_oneof![1 => canonical, 2 => mutated, 6 => built, 1 => prop::collection::vec(any::<u8>(), 0..40)]
}

proptest! {
    #![proptest_config(config_())]

    #[test]
    fn names_agree(name in name()) {
        prop_assert_eq!(old_name(&name), new_name(&name), "{:?}", name);
    }

    #[test]
    fn paths_agree(path in path()) {
        prop_assert_eq!(old_tidy(&path), new_tidy(&path), "{:?}", path.escape_ascii().to_string());
    }

    #[test]
    fn persons_agree(line in person_line()) {
        prop_assert_eq!(old_person(&line), new_person(&line), "{:?}", line.escape_ascii().to_string());
    }

    #[test]
    fn pointers_agree(data in pointer()) {
        prop_assert_eq!(old_pointer(&data), new_pointer(&data), "{:?}", data.escape_ascii().to_string());
    }
}

fn hand_pointers() -> Vec<Vec<u8>> {
    let oid = hex(64, 7, false);
    let base = |v: &[u8], rest: &[u8]| {
        let mut p = v.to_vec();
        p.extend(b"\noid sha256:");
        p.extend(&oid);
        p.extend(rest);
        p
    };
    let mut out = vec![
        base(
            b"version https://git-lfs.github.com/spec/v1",
            b"\nsize 12345\n",
        ),
        base(
            b"version https://hawser.github.com/spec/v1",
            b"\nsize 12345\n",
        ),
        base(
            b"version https://git-lfs.github.com/spec/v1",
            b"\nsize 12345",
        ),
        base(
            b"version https://git-lfs.github.com/spec/v1",
            b"\r\nsize 12345\r\n",
        ),
        base(
            b"version https://git-lfs.github.com/spec/v1",
            b"\nsize 012345\n",
        ),
        base(
            b"version https://git-lfs.github.com/spec/v1",
            b"\nsize 18446744073709551616\n",
        ),
        base(
            b"version https://git-lfs.github.com/spec/v1",
            b"\nsize -1\n",
        ),
        base(
            b"version https://git-lfs.github.com/spec/v1",
            b"\nsize 1\n\n",
        ),
        base(
            b"version https://git-lfs.github.com/spec/v1",
            b"\nsize 1\next-0-a sha256:00\n",
        ),
        b"version https://git-lfs.github.com/spec/v1\nsize 1\n".to_vec(),
        b"version https://git-lfs.github.com/spec/v1\n".to_vec(),
        b"version https://git-lfs.github.com/spec/v1".to_vec(),
        Vec::new(),
    ];
    let mut ext = b"version https://git-lfs.github.com/spec/v1\next-0-a sha256:".to_vec();
    ext.extend(&oid);
    ext.extend(b"\noid sha256:");
    ext.extend(&oid);
    ext.extend(b"\nsize 5\n");
    out.push(ext);
    // Lengths around the 1024-byte limit.
    for len in [1020usize, 1023, 1024, 1025, 1030] {
        let mut p = b"version https://git-lfs.github.com/spec/v1\next-0-a sha256:".to_vec();
        let tail_len = b"\noid sha256:".len() + 64 + b"\nsize 5\n".len();
        while p.len() + tail_len < len {
            p.push(b'0');
        }
        p.extend(b"\noid sha256:");
        p.extend(&oid);
        p.extend(b"\nsize 5\n");
        out.push(p);
    }
    out
}

#[test]
fn hand_made_inputs_agree() {
    for name in NAME_BASES {
        assert_eq!(old_name(name), new_name(name), "{name:?}");
    }
    for p in PATH_PIECES {
        assert_eq!(old_tidy(p), new_tidy(p), "{p:?}");
    }
    for p in PERSON_PIECES {
        assert_eq!(old_person(p), new_person(p), "{p:?}");
    }
    for len in [4095usize, 4096, 4097, 9000] {
        for fill in [&b"a"[..], b"\xc3\xa9", b"\xe2\x82\xac"] {
            let field: Vec<u8> = fill.iter().copied().cycle().take(len).collect();
            let mut line = field.clone();
            line.extend(b" <");
            line.extend(&field);
            line.extend(b"> 1700000000 +0100");
            assert_eq!(
                old_person(&line),
                new_person(&line),
                "long field {len} {fill:?}"
            );
        }
    }
    for p in hand_pointers() {
        assert_eq!(
            old_pointer(&p),
            new_pointer(&p),
            "{:?}",
            p.escape_ascii().to_string()
        );
    }
}

// The labels of the parts of two answers that differ: `a=1 b=2` against `a=1 b=3` gives `b`.
fn differing(old: &[(String, String)], new: &[(String, String)]) -> String {
    let mut out = Vec::new();
    for (o, n) in old.iter().zip(new) {
        if o != n {
            out.push(o.0.clone());
        }
    }
    out.join(",")
}

fn name_parts(answer: &str) -> Vec<(String, String)> {
    answer
        .split(' ')
        .map(|f| {
            let (k, v) = f.split_once('=').unwrap_or((f, ""));
            (k.to_owned(), v.to_owned())
        })
        .collect()
}

fn tidy_parts(answer: &str) -> Vec<(String, String)> {
    let (u, w) = answer.split_once(" windows=").unwrap_or((answer, ""));
    vec![
        ("unix".into(), u.to_owned()),
        ("windows".into(), w.to_owned()),
    ]
}

fn person_parts(p: &crate::save::Person) -> Vec<(String, String)> {
    vec![
        ("name".into(), p.name.clone()),
        ("email".into(), p.email.clone()),
        ("time".into(), p.time_ms.to_string()),
        ("zone".into(), p.tz_offset_min.to_string()),
    ]
}

type Seen = std::collections::BTreeMap<String, (usize, Vec<(String, String, String)>)>;

fn note(seen: &mut Seen, group: String, input: String, old: String, new: String) {
    let e = seen.entry(group).or_insert((0, Vec::new()));
    e.0 += 1;
    if e.1.iter().any(|x| x.0 == input) {
        return;
    }
    e.1.push((input, old, new));
    e.1.sort_by_key(|x| x.0.len());
    e.1.truncate(6);
}

/// Every disagreement the generators reach, grouped by unit and the parts that differ, with the
/// shortest inputs of each group.
#[test]
#[ignore]
fn census() {
    use proptest::strategy::ValueTree;
    use proptest::test_runner::TestRunner;
    let mut seen = Seen::new();
    let mut runner = TestRunner::default();
    let (names_s, paths_s, persons_s, pointers_s) = (name(), path(), person_line(), pointer());
    let started = std::time::Instant::now();
    for _ in 0..cases() {
        let n = names_s.new_tree(&mut runner).unwrap().current();
        let (o, w) = (old_name(&n), new_name(&n));
        if o != w {
            let tag = if n.contains(['\\', '/']) {
                "with a slash"
            } else {
                "no slash"
            };
            let g = format!(
                "names ({tag}): {}",
                differing(&name_parts(&o), &name_parts(&w))
            );
            note(&mut seen, g, format!("{n:?}"), o, w);
        }
        let p = paths_s.new_tree(&mut runner).unwrap().current();
        let (o, w) = (old_tidy(&p), new_tidy(&p));
        if o != w {
            let g = format!(
                "tidy_path ({}): {}",
                byte_tag(&p),
                differing(&tidy_parts(&o), &tidy_parts(&w))
            );
            note(&mut seen, g, format!("b\"{}\"", p.escape_ascii()), o, w);
        }
        let l = persons_s.new_tree(&mut runner).unwrap().current();
        let (op, np) = (person::parse_person(&l), git::parse_person(&l));
        if op != np {
            let g = format!(
                "parse_person ({}, {}): {}",
                byte_tag(&l),
                if np.name.trim() == op.name || np.email.trim() == op.email {
                    "trimmed"
                } else {
                    "other"
                },
                differing(&person_parts(&op), &person_parts(&np))
            );
            note(
                &mut seen,
                g,
                format!("b\"{}\"", l.escape_ascii()),
                format!("{op:?}"),
                format!("{np:?}"),
            );
        }
        let d = pointers_s.new_tree(&mut runner).unwrap().current();
        let (o, w) = (old_pointer(&d), new_pointer(&d));
        if o != w {
            let g = format!(
                "lfs_pointer: {} => {}",
                o.split(' ').next().unwrap_or(""),
                w.split(' ').next().unwrap_or("")
            );
            note(&mut seen, g, format!("b\"{}\"", d.escape_ascii()), o, w);
        }
    }
    println!(
        "{} cases a property in {:?}, {} groups",
        cases(),
        started.elapsed(),
        seen.len()
    );
    for (group, (count, examples)) in &seen {
        println!("\n## {group}\n  count: {count}");
        for (input, old, new) in examples {
            println!("  input: {input}\n    old: {old}\n    new: {new}");
        }
    }
}

fn byte_tag(bytes: &[u8]) -> &'static str {
    if bytes.contains(&0) {
        "NUL"
    } else if std::str::from_utf8(bytes).is_err() {
        "not UTF-8"
    } else {
        "UTF-8"
    }
}
