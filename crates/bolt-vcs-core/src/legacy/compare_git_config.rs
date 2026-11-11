//! The rewritten Git config reader against the old one: the same inputs to both, the same answers.

use proptest::prelude::*;

use super::git_config as old;
use crate::git::{self as new, GitError};

fn config() -> ProptestConfig {
    let cases = std::env::var("BOLT_COMPARE_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(2_000);
    ProptestConfig {
        cases,
        failure_persistence: None,
        ..ProptestConfig::default()
    }
}

fn err(e: &GitError) -> String {
    format!("Err {e:?} | message: {e} | hint: {:?}", e.hint())
}

fn old_format(text: &[u8]) -> String {
    match old::read_format(text, ".git/config") {
        Ok(f) => format!(
            "Ok hash={:?} worktree_config={} sparse={:?} includes={} lfs={:?}",
            f.hash, f.worktree_config, f.sparse, f.includes, f.lfs_storage
        ),
        Err(e) => err(&e),
    }
}

fn new_format(text: &[u8]) -> String {
    match new::read_config_format(text, ".git/config") {
        Ok(f) => format!(
            "Ok hash={:?} worktree_config={} sparse={:?} includes={} lfs={:?}",
            f.hash, f.worktree_config, f.sparse, f.includes, f.lfs_storage
        ),
        Err(e) => err(&e),
    }
}

fn old_worktree(text: &[u8]) -> String {
    match old::read_worktree(text, ".git/config.worktree") {
        Ok(w) => format!(
            "Ok sparse={:?} includes={} lfs={:?}",
            w.sparse, w.includes, w.lfs_storage
        ),
        Err(e) => err(&e),
    }
}

fn new_worktree(text: &[u8]) -> String {
    match new::read_config_worktree(text, ".git/config.worktree") {
        Ok(w) => format!(
            "Ok sparse={:?} includes={} lfs={:?}",
            w.sparse, w.includes, w.lfs_storage
        ),
        Err(e) => err(&e),
    }
}

fn check_text(text: &[u8]) {
    assert_eq!(old_format(text), new_format(text), "read_format {text:?}");
    assert_eq!(
        old_worktree(text),
        new_worktree(text),
        "read_worktree {text:?}"
    );
    assert_eq!(
        format!("{:?}", old::read_user_settings(text)),
        format!("{:?}", new::read_user_settings(text)),
        "read_user_settings {text:?}"
    );
}

fn check_value(value: &[u8]) {
    assert_eq!(
        old::read_bool(value),
        new::read_bool(value),
        "read_bool {value:?}"
    );
    assert_eq!(
        old::read_count(value),
        new::read_count(value),
        "read_count {value:?}"
    );
}

fn check_variable(key: &[u8], value: &[u8]) {
    assert_eq!(
        format!("{:?}", old::read_user_variable(key, value)),
        format!("{:?}", new::read_user_variable(key, value)),
        "read_user_variable {key:?} {value:?}"
    );
}

const SECTIONS: &[&str] = &[
    "core",
    "Core",
    "CORE",
    "extensions",
    "Extensions",
    "lfs",
    "LFS",
    "include",
    "Include",
    "includeIf",
    "includeif",
    "user",
    "remote",
    "",
    "co re",
    "core.",
    ".core",
    "core.sub",
    "Core.Sub",
    "core..x",
    "include.path",
    "lfs.storage",
    "a-b",
    "a_b",
    "1core",
    "-core",
    "cöre",
];

const SUBSECTIONS: &[&str] = &[
    "",
    "sub",
    "gitdir:/x/",
    "onbranch:main",
    "a\\\"b",
    "a\\\\b",
    "a\\xb",
    "a\\nb",
    "a\\",
    "a\"b",
    "a b",
    "a]b",
    "a\tb",
    "a\x00b",
    "path",
    "Sub",
];

const HEADERS: &[&[u8]] = &[
    b"[core]",
    b"[ core ]",
    b"[\tcore]",
    b"[core ]",
    b"[core",
    b"[]",
    b"[\"x\"]",
    b"[ \"x\"]",
    b"[core \"sub\"",
    b"[core \"sub]",
    b"[core \"sub\" ]",
    b"[core \"sub\"x]",
    b"[core  \"sub\"]",
    b"[core\t\"sub\"]",
    b"[core\"sub\"]",
    b"[core \"\"]",
    b"[includeIf \"gitdir:x\"]",
    b"[includeif \"a\\\"b\"]",
    b"[includeIf]",
    b"[include \"x\"]",
    b"[extensions \"x\"]",
    b"[core.x]",
    b"[core] repositoryformatversion = 1",
    b"[extensions]objectformat=sha256",
    b"[core]# c",
    b"[core] ; c",
    b"[core]x",
    b"[core]]",
    b"[[core]",
    b"[core \"a\nb\"]",
    b"[core\n]",
];

const KEYS: &[&str] = &[
    "repositoryformatversion",
    "RepositoryFormatVersion",
    "objectformat",
    "objectFormat",
    "worktreeconfig",
    "worktreeConfig",
    "refstorage",
    "refStorage",
    "partialclone",
    "partialClone",
    "submodulepathconfig",
    "preciousobjects",
    "noop",
    "noop-v1",
    "compatobjectformat",
    "relativeworktrees",
    "unknownext",
    "sparsecheckout",
    "sparseCheckout",
    "sparsecheckoutcone",
    "storage",
    "path",
    "Path",
    "excludesfile",
    "excludesFile",
    "bare",
    "1key",
    "-key",
    "k-e-y",
    "k_y",
    "k.y",
    "ké",
    "k y",
    "",
    "a1",
];

const SEPS: &[&str] = &[
    "", " = ", "=", "  =", " =", "= ", "\t=\t", " ", "\t", " == ", "=\t",
];

const VALUES: &[&str] = &[
    "0",
    "1",
    "2",
    "-1",
    "+1",
    "01",
    "0x1",
    "0X1",
    "0x",
    "-0",
    "010",
    "1k",
    "1K",
    "1m",
    "1M",
    "1g",
    "1G",
    "1t",
    "0k",
    "2147483647",
    "2147483648",
    "4294967295",
    "4294967296",
    "9223372036854775807",
    "9223372036854775808",
    "99999999999999999999",
    "-9223372036854775808",
    "8388608k",
    "true",
    "false",
    "yes",
    "no",
    "on",
    "off",
    "TRUE",
    "Yes",
    "tru",
    "",
    "sha1",
    "sha256",
    "SHA256",
    "Sha1",
    "sha512",
    "files",
    "reftable",
    "files://",
    "files://x",
    "reftable://foo",
    "FILES",
    "origin",
    "~/x",
    "/tmp/x",
    "\"",
    "\"\"",
    "\"a\"",
    "\"a\\\"b\"",
    "\\\"",
    "\\\\",
    "\\n",
    "\\t",
    "\\b",
    "\\x",
    "\\0",
    "\\ ",
    "\\",
    " \\",
    "a\\\nb",
    "a \\\n b",
    "a\\\r\nb",
    "\"a\\\nb\"",
    " ",
    "\t",
    "  ",
    "#",
    ";",
    " # c",
    " ;c",
    "\"a;b\"",
    "\"a#b\"",
    "\"a ; b\" x",
    "a  b",
    "\x00",
    "\u{feff}",
    "é",
    "\u{0301}",
];

fn eol() -> impl Strategy<Value = Vec<u8>> {
    prop_oneof![
        6 => Just(b"\n".to_vec()),
        2 => Just(b"\r\n".to_vec()),
        1 => Just(b"\r".to_vec()),
        1 => Just(Vec::new()),
        1 => Just(b"\n\n".to_vec()),
    ]
}

fn pick(list: &'static [&'static str]) -> impl Strategy<Value = Vec<u8>> {
    prop::sample::select(list).prop_map(|s| s.as_bytes().to_vec())
}

fn header() -> impl Strategy<Value = Vec<u8>> {
    let built = (pick(SECTIONS), pick(SUBSECTIONS), any::<bool>()).prop_map(|(s, sub, quoted)| {
        let mut h = b"[".to_vec();
        h.extend(s);
        if quoted {
            h.extend(b" \"");
            h.extend(sub);
            h.push(b'"');
        }
        h.push(b']');
        h
    });
    prop_oneof![
        prop::sample::select(HEADERS).prop_map(|h| h.to_vec()),
        built,
    ]
}

fn value() -> impl Strategy<Value = Vec<u8>> {
    prop::collection::vec(pick(VALUES), 0..4).prop_map(|parts| parts.concat())
}

fn key_line() -> impl Strategy<Value = Vec<u8>> {
    (
        pick(&["", " ", "\t", "  \t"]),
        pick(KEYS),
        prop_oneof![4 => pick(SEPS).prop_map(Some), 1 => Just(None)],
        value(),
        pick(&["", " ", "\t", " # c", " ; c", "\\"]),
    )
        .prop_map(|(lead, key, sep, value, trail)| {
            let mut l = lead;
            l.extend(key);
            if let Some(sep) = sep {
                l.extend(sep);
                l.extend(value);
                l.extend(trail);
            }
            l
        })
}

// A section and key Bolt reads, often enough that their values are really tried.
fn interesting() -> impl Strategy<Value = Vec<u8>> {
    let pairs: &[(&str, &str)] = &[
        ("core", "repositoryformatversion"),
        ("extensions", "objectformat"),
        ("extensions", "worktreeconfig"),
        ("extensions", "refstorage"),
        ("extensions", "partialclone"),
        ("extensions", "submodulepathconfig"),
        ("extensions", "preciousobjects"),
        ("extensions", "foo"),
        ("extensions", "compatobjectformat"),
        ("core", "sparsecheckout"),
        ("lfs", "storage"),
        ("include", "path"),
        ("includeIf \"gitdir:x\"", "path"),
        ("includeif \"\"", "path"),
        ("core", "excludesfile"),
    ];
    (
        prop::sample::select(pairs),
        prop_oneof![3 => pick(&[" = ", "=", " =\t"]).prop_map(Some), 1 => Just(None)],
        value(),
    )
        .prop_map(|((section, key), sep, value)| {
            let mut l = format!("[{section}]\n\t{key}").into_bytes();
            if let Some(sep) = sep {
                l.extend(sep);
                l.extend(value);
            }
            l
        })
}

// A format file of mostly well-formed settings Bolt reads, repeated and mixed, so that answers
// other than "can't read" are common: hashes, compatibility hashes, refusals and their order.
fn format_file() -> impl Strategy<Value = Vec<u8>> {
    let settings: &[(&str, &str, &[&str])] = &[
        (
            "core",
            "repositoryformatversion",
            &["0", "1", "1", "2", "-1", "01", "1k", "\"1\"", "0x1"],
        ),
        (
            "extensions",
            "objectformat",
            &[
                "sha1",
                "sha256",
                "SHA256",
                "Sha1",
                "sha512",
                "",
                "\"sha256\"",
                "sha256 ",
                "sha1 # c",
            ],
        ),
        (
            "extensions",
            "compatobjectformat",
            &[
                "sha1",
                "sha256",
                "SHA1",
                "Sha256",
                "sha512",
                "",
                "\"sha1\"",
                "sha256 ; c",
                "sha1\\n",
                "md5",
            ],
        ),
        (
            "Extensions",
            "compatObjectFormat",
            &["sha1", "sha256", "SHA256", ""],
        ),
        (
            "extensions",
            "worktreeconfig",
            &["true", "false", "yes", "0", "1", "maybe", ""],
        ),
        (
            "extensions",
            "refstorage",
            &[
                "files",
                "reftable",
                "FILES",
                "files://x",
                "reftable://x",
                "",
                "x",
            ],
        ),
        ("extensions", "partialclone", &["origin", "", "\"\""]),
        ("extensions", "noop", &["x", ""]),
        ("extensions", "noop-v1", &["x", ""]),
        ("extensions", "preciousobjects", &["true", "false", ""]),
        ("extensions", "relativeworktrees", &["true", "false"]),
        ("extensions", "submodulepathconfig", &["true", "false"]),
        ("extensions", "unknownext", &["x"]),
        ("core", "sparsecheckout", &["true", "false", "maybe", ""]),
        ("lfs", "storage", &["/x", "", "\"a b\""]),
        ("include", "path", &["x", ""]),
        ("includeIf \"gitdir:x\"", "path", &["x"]),
    ];
    let line = (
        prop::sample::select(settings),
        any::<prop::sample::Index>(),
        prop_oneof![6 => pick(&[" = ", "=", "\t=\t"]).prop_map(Some), 1 => Just(None)],
        pick(&["", "", "", " ", " # c", "\r"]),
        any::<bool>(),
    )
        .prop_map(|((section, key, values), at, sep, trail, header)| {
            let mut l = Vec::new();
            if header {
                l.extend(format!("[{section}]\n").into_bytes());
            }
            l.extend(format!("\t{key}").into_bytes());
            if let Some(sep) = sep {
                l.extend(sep);
                l.extend(values[at.index(values.len())].as_bytes());
                l.extend(trail);
            }
            l.push(b'\n');
            l
        });
    prop::collection::vec(line, 1..8).prop_map(|lines| lines.concat())
}

fn version_prelude() -> impl Strategy<Value = Vec<u8>> {
    prop_oneof![
        2 => Just(Vec::new()),
        3 => Just(b"[core]\n\trepositoryformatversion = 1\n".to_vec()),
        1 => Just(b"[core]\n\trepositoryformatversion = 0\n".to_vec()),
        1 => Just(b"[CORE]RepositoryFormatVersion=1\n".to_vec()),
    ]
}

fn noise() -> impl Strategy<Value = Vec<u8>> {
    prop::collection::vec(
        prop::sample::select(b"[]\"\\=#; \t\r\n.-_aZ0\x00\xff\xc3\xa9".as_slice()),
        0..12,
    )
}

fn config_text() -> impl Strategy<Value = Vec<u8>> {
    prop_oneof![3 => mixed_text(), 2 => format_file()]
}

fn mixed_text() -> impl Strategy<Value = Vec<u8>> {
    let piece = (
        prop_oneof![
            3 => header(),
            4 => key_line(),
            4 => interesting(),
            1 => noise(),
            1 => pick(&["", "# comment", "; comment", "   ", "\t# c"]),
        ],
        eol(),
    )
        .prop_map(|(p, e)| [p, e].concat());
    (
        prop_oneof![8 => Just(Vec::new()), 1 => Just(b"\xef\xbb\xbf".to_vec()), 1 => Just(b"\xef\xbb".to_vec())],
        version_prelude(),
        prop::collection::vec(piece, 0..10),
        prop::option::weighted(0.1, (any::<prop::sample::Index>(), any::<u8>())),
    )
        .prop_map(|(bom, prelude, pieces, poke)| {
            let mut text = [bom, prelude, pieces.concat()].concat();
            if let Some((at, byte)) = poke {
                let at = at.index(text.len() + 1);
                text.insert(at, byte);
            }
            text
        })
}

fn count_text() -> impl Strategy<Value = Vec<u8>> {
    prop_oneof![
        2 => (
            pick(&["", " ", "\t", "\n", "\x0b", "\x0c", "\r", "  "]),
            pick(&["", "+", "-", "--", "+-"]),
            prop::collection::vec(prop::sample::select(b"0123456789".as_slice()), 0..25),
            pick(&["", " ", "k", "x", "\x00", "\n", "0x10"]),
        )
            .prop_map(|(a, b, c, d)| [a, b, c, d].concat()),
        1 => pick(VALUES),
        1 => noise(),
    ]
}

fn variable_key() -> impl Strategy<Value = Vec<u8>> {
    let built = (
        pick(SECTIONS),
        prop::option::of(pick(SUBSECTIONS)),
        pick(KEYS),
    )
        .prop_map(|(s, sub, k)| {
            let mut out = s;
            if let Some(sub) = sub {
                out.push(b'.');
                out.extend(sub);
            }
            out.push(b'.');
            out.extend(k);
            out
        });
    prop_oneof![
        2 => pick(&[
            "core.excludesfile",
            "CORE.ExcludesFile",
            "core.excludesFile",
            "include.path",
            "Include.Path",
            "includeIf.gitdir:x.path",
            "includeif..path",
            "includeIf.path",
            "lfs.storage",
            "LFS.Storage",
            "lfs.Storage.x",
            "core.sparsecheckout",
            "core.sparseCheckout",
            "core..excludesfile",
            "core",
            "core.",
            ".core.x",
            "core.x.",
            "a.b.c",
            "core.excludes file",
            "core.1x",
            "core.-x",
            "core.x_y",
            "core.x\x00",
            "",
            ".",
            "..",
        ]),
        3 => built,
        1 => noise(),
    ]
}

proptest! {
    #![proptest_config(config())]

    #[test]
    fn config_texts_agree(text in config_text()) {
        check_text(&text);
    }

    #[test]
    fn values_agree(value in prop_oneof![value(), count_text(), noise()]) {
        check_value(&value);
    }

    #[test]
    fn variables_agree(key in variable_key(), value in prop_oneof![value(), noise()]) {
        check_variable(&key, &value);
    }
}

const HAND_TEXTS: &[&[u8]] = &[
    b"",
    b"\xef\xbb\xbf",
    b"\xef\xbb\xbf[core]\n\trepositoryformatversion = 1\n[extensions]\n\tobjectformat = sha256\n",
    b"[core]\r\n\trepositoryformatversion = 1\r\n[extensions]\r\n\tobjectformat = sha256\r\n",
    b"[core]\r\trepositoryformatversion = 1\r[extensions]\robjectformat = sha256\r",
    b"[core]\n\trepositoryformatversion = 0\n[extensions]\n\tobjectformat = sha256\n",
    b"[core]\n\trepositoryformatversion = 2\n",
    b"[core]\n\trepositoryformatversion = -1\n",
    b"[core]\n\trepositoryformatversion = 1k\n",
    b"[core]\n\trepositoryformatversion = 0x1\n",
    b"[core]\n\trepositoryformatversion = \"1\"\n",
    b"[core]\n\trepositoryformatversion\n",
    b"[core]\n\trepositoryformatversion =\n",
    b"[core]\n\trepositoryformatversion = 4294967297\n",
    b"[core]\n\trepositoryformatversion = 1\n[extensions]\n\tobjectformat = sha512\n",
    b"[core]\n\trepositoryformatversion = 1\n[extensions]\n\tobjectformat\n",
    b"[core]\n\trepositoryformatversion = 1\n[extensions]\n\tworktreeconfig = true\n",
    b"[core]\n\trepositoryformatversion = 1\n[extensions]\n\tworktreeconfig\n",
    b"[core]\n\trepositoryformatversion = 1\n[extensions]\n\tworktreeconfig = maybe\n",
    b"[core]\n\trepositoryformatversion = 1\n[extensions]\n\trefstorage = reftable\n",
    b"[core]\n\trepositoryformatversion = 1\n[extensions]\n\trefstorage = files\n",
    b"[core]\n\trepositoryformatversion = 1\n[extensions]\n\trefstorage = files://x\n",
    b"[core]\n\trepositoryformatversion = 1\n[extensions]\n\trefstorage = reftable://x\n",
    b"[core]\n\trepositoryformatversion = 1\n[extensions]\n\tpartialclone = origin\n",
    b"[core]\n\trepositoryformatversion = 1\n[extensions]\n\tpartialclone =\n",
    b"[core]\n\trepositoryformatversion = 1\n[extensions]\n\tpartialclone\n",
    b"[core]\n\trepositoryformatversion = 1\n[extensions]\n\tsubmodulepathconfig = true\n",
    b"[core]\n\trepositoryformatversion = 1\n[extensions]\n\tunknown = x\n",
    b"[core]\n\trepositoryformatversion = 0\n[extensions]\n\tunknown = x\n",
    b"[core]\n\tsparsecheckout = true\n",
    b"[core]\n\tsparsecheckout = maybe\n",
    b"[core]\n\tsparsecheckout\n",
    b"[lfs]\n\tstorage = /x\n",
    b"[lfs]\n\tstorage\n",
    b"[lfs]\n\tstorage =\n",
    b"[include]\n\tpath = x\n",
    b"[include]\n\tpath\n",
    b"[includeIf \"gitdir:x\"]\n\tpath = y\n",
    b"[includeIf \"gitdir:x\"]\n\tpath\n",
    b"[core]\n\texcludesfile = ~/x\n",
    b"[core]\n\texcludesfile\n",
    b"[core \"a\\\"b\\\\c\\x\"]\n\tx = 1\n",
    b"[core \"a",
    b"[core",
    b"[",
    b"]",
    b"=",
    b"x",
    b"[core]\n\t1x = 1\n",
    b"[core]\n\t-x = 1\n",
    b"[core]\n\tx = \"a\n",
    b"[core]\n\tx = a\\",
    b"[core]\n\tx = a \\\n\tb\n",
    b"[core]\n\tx = a\\\r\nb\n",
    b"[core]\n\tx = \\q\n",
    b"[core]\n\tx = \"a;b\" ; c\n",
    b"[core]\n\tx = a\x00b\n",
    b"\x00",
    b"[core]\x00\n",
    b"[core]\n\tx = \xff\xfe\n",
    b"[c\xc3\xb6re]\n",
    b"[core]\n\trepositoryformatversion = 1\n[extensions]\n\tcompatObjectFormat = sha1\n",
    b"[core]\n\trepositoryformatversion = 1\n[extensions]\n\tcompatObjectFormat = sha256\n",
    b"[core]\n\trepositoryformatversion = 1\n[extensions]\n\tobjectformat = sha256\n\tcompatObjectFormat = sha256\n",
    b"[core]\n\trepositoryformatversion = 1\n[extensions]\n\tcompatObjectFormat = sha256\n\tobjectformat = sha256\n",
    b"[core]\n\trepositoryformatversion = 1\n[extensions]\n\tobjectformat = sha256\n\tcompatObjectFormat = sha1\n",
    b"[core]\n\trepositoryformatversion = 1\n[extensions]\n\tobjectformat = sha256\n\tcompatObjectFormat = sha1\n\tcompatObjectFormat = sha1\n",
    b"[core]\n\trepositoryformatversion = 1\n[extensions]\n\tobjectformat = sha256\n\tcompatObjectFormat = sha1\n\tcompatObjectFormat = sha256\n",
    b"[core]\n\trepositoryformatversion = 1\n[extensions]\n\tcompatObjectFormat = \"sha256\"\n",
    b"[core]\n\trepositoryformatversion = 1\n[extensions]\n\tcompatObjectFormat = md5\n",
    b"[core]\n\trepositoryformatversion = 1\n[extensions]\n\tcompatObjectFormat\n",
    b"[core]\n\trepositoryformatversion = 1\n[extensions]\n\tcompatObjectFormat =\n",
    b"[core]\n\trepositoryformatversion = 0\n[extensions]\n\tcompatObjectFormat = sha1\n",
];

#[test]
fn hand_made_texts_agree() {
    for text in HAND_TEXTS {
        check_text(text);
    }
    for len in [1023, 1024, 4095, 4096, 65535, 65536, 1 << 20] {
        let mut text = b"[lfs]\n\tstorage = ".to_vec();
        text.resize(text.len() + len, b'a');
        text.push(b'\n');
        check_text(&text);
        let mut key = b"[core]\n\t".to_vec();
        key.resize(key.len() + len, b'k');
        key.extend(b" = 1\n");
        check_text(&key);
        let mut header = b"[core \"".to_vec();
        header.resize(header.len() + len, b's');
        header.extend(b"\"]\n");
        check_text(&header);
    }
}

#[test]
fn hand_made_values_agree() {
    for value in VALUES {
        check_value(value.as_bytes());
    }
    for value in [
        &b"2147483647"[..],
        b"2147483648",
        b"18446744073709551615",
        b"18446744073709551616",
        b"-18446744073709551615",
        b"-2147483647",
        b" \t\n\x0b\x0c\r5",
        b"5 ",
        b"",
        b"+",
        b"-",
        b"0x10",
        b"\xff",
    ] {
        check_value(value);
    }
}

#[test]
fn hand_made_variables_agree() {
    for key in [
        &b"core.excludesfile"[..],
        b"include.path",
        b"includeIf.gitdir:x.path",
        b"lfs.storage",
        b"core.sparsecheckout",
        b"core",
        b"core.",
        b".x",
        b"core.1x",
        b"a.b c.d",
        b"a.b\nc.d",
        b"",
    ] {
        for value in [&b""[..], b"true", b"x", b"\x00", b"maybe"] {
            check_variable(key, value);
        }
    }
}

// The shape of an answer, to group disagreements: digits, quoted values and setting names taken
// out, so one wording difference is one group.
fn shape(answer: &str) -> String {
    let answer = answer.split(" | message").next().unwrap_or_default();
    let mut out = String::new();
    let mut quoted = false;
    for c in answer.chars() {
        match c {
            '`' => {
                quoted = !quoted;
                if quoted {
                    out.push_str("`V`");
                }
            }
            _ if quoted => {}
            '0'..='9' => {
                if !out.ends_with('N') {
                    out.push('N');
                }
            }
            _ => out.push(c),
        }
    }
    let mut words: Vec<String> = Vec::new();
    for w in out.split(' ') {
        let after_of =
            words.len() >= 2 && words[words.len() - 2] == "value" && words[words.len() - 1] == "of";
        let lfs = w.starts_with("lfs=Some");
        words.push(if lfs {
            "lfs=Some".to_owned()
        } else if after_of
            || (w.contains('.') && !w.starts_with(".git") && !w.starts_with("\".git"))
        {
            "KEY".to_owned()
        } else {
            w.to_owned()
        });
    }
    let mut shaped = words.join(" ");
    while shaped.contains("N, N") {
        shaped = shaped.replace("N, N", "N");
    }
    shaped
}

#[derive(Default)]
struct Census(std::collections::BTreeMap<String, (usize, String, String, String)>);

impl Census {
    fn see(&mut self, what: &str, input: String, old: String, new: String) {
        if old == new {
            return;
        }
        let group = format!("{what}: {} => {}", shape(&old), shape(&new));
        let entry = self
            .0
            .entry(group)
            .or_insert((0, input.clone(), old.clone(), new.clone()));
        entry.0 += 1;
        if input.len() < entry.1.len() {
            *entry = (entry.0, input, old, new);
        }
    }
}

/// Every disagreement the generators reach, grouped, with the shortest input of each group.
#[test]
#[ignore]
fn census() {
    use proptest::strategy::ValueTree;
    use proptest::test_runner::TestRunner;
    let cases: usize = std::env::var("BOLT_COMPARE_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(2_000);
    let mut census = Census::default();
    let mut runner = TestRunner::default();
    let texts = config_text();
    let values = prop_oneof![value(), count_text(), noise()];
    let keys = variable_key();
    let var_values = prop_oneof![value(), noise()];
    let mut text_inputs: Vec<Vec<u8>> = HAND_TEXTS.iter().map(|t| t.to_vec()).collect();
    let mut value_inputs: Vec<Vec<u8>> = VALUES.iter().map(|v| v.as_bytes().to_vec()).collect();
    let mut var_inputs = Vec::new();
    let started = std::time::Instant::now();
    for _ in 0..cases {
        text_inputs.push(texts.new_tree(&mut runner).unwrap().current());
        value_inputs.push(values.new_tree(&mut runner).unwrap().current());
        var_inputs.push((
            keys.new_tree(&mut runner).unwrap().current(),
            var_values.new_tree(&mut runner).unwrap().current(),
        ));
        for t in text_inputs.drain(..) {
            let shown = format!("b\"{}\"", bstr(&t));
            census.see("read_format", shown.clone(), old_format(&t), new_format(&t));
            census.see(
                "read_worktree",
                shown.clone(),
                old_worktree(&t),
                new_worktree(&t),
            );
            census.see(
                "read_user_settings",
                shown,
                format!("{:?}", old::read_user_settings(&t)),
                format!("{:?}", new::read_user_settings(&t)),
            );
        }
        for v in value_inputs.drain(..) {
            let shown = format!("b\"{}\"", bstr(&v));
            census.see(
                "read_bool",
                shown.clone(),
                format!("{:?}", old::read_bool(&v)),
                format!("{:?}", new::read_bool(&v)),
            );
            census.see(
                "read_count",
                shown,
                format!("{:?}", old::read_count(&v)),
                format!("{:?}", new::read_count(&v)),
            );
        }
        for (k, v) in var_inputs.drain(..) {
            census.see(
                "read_user_variable",
                format!("b\"{}\", b\"{}\"", bstr(&k), bstr(&v)),
                format!("{:?}", old::read_user_variable(&k, &v)),
                format!("{:?}", new::read_user_variable(&k, &v)),
            );
        }
    }
    println!("{cases} cases a property in {:?}", started.elapsed());
    for (group, (count, input, old, new)) in &census.0 {
        println!("\n## {group}\n  count: {count}\n  input: {input}\n  old: {old}\n  new: {new}");
    }
}

// Bytes as a Rust byte-string literal body.
fn bstr(bytes: &[u8]) -> String {
    bytes.escape_ascii().to_string()
}
