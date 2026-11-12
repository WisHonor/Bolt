#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
use super::*;

#[test]
fn pointers_parse_as_git_lfs_parses_them() {
    // The edges of Git LFS's reading of a pointer.
    let oid = "4d7a214614ab2935c943f9e0ff69d22eadbb8f32b1258daaa5e2ca24d17e2393";
    let v1 = "version https://git-lfs.github.com/spec/v1\n";
    let pointer = format!("{v1}oid sha256:{oid}\nsize 12345\n");
    let ext = |priority: &str, name: &str, digit: char| {
        format!(
            "ext-{priority}-{name} sha256:{}\n",
            digit.to_string().repeat(64)
        )
    };
    let padded = format!("{pointer}{}", "\n".repeat(1024 - pointer.len()));
    for (what, data, want) in [
        (
            "the alpha version",
            pointer.replace(
                "https://git-lfs.github.com/spec/v1",
                "http://git-media.io/v/2",
            ),
            true,
        ),
        (
            "an extension before the version",
            format!("{}{pointer}", ext("0", "foo", 'f')),
            true,
        ),
        ("no LF at the end", pointer.trim_end().to_owned(), true),
        ("CRLF", pointer.replace('\n', "\r\n"), true),
        (
            "whitespace around it, and blank lines",
            format!(" \t\n{v1}\noid sha256:{oid}\n\nsize 12345   \n\n"),
            true,
        ),
        (
            "a no-break space before it",
            format!("\u{a0}{pointer}"),
            true,
        ),
        (
            "a size with a plus sign",
            pointer.replace("12345", "+12345"),
            true,
        ),
        ("a size of minus zero", pointer.replace("12345", "-0"), true),
        (
            "a size of zero, zero-padded",
            pointer.replace("12345", "000"),
            true,
        ),
        ("1,024 bytes", padded.clone(), true),
        ("an empty file", String::new(), false),
        ("1,025 bytes", format!("{padded}\n"), false),
        (
            "an oid in capitals",
            pointer.replace(oid, &oid.to_uppercase()),
            false,
        ),
        (
            "two spaces after a key",
            pointer.replace("version ", "version  "),
            false,
        ),
        ("a negative size", pointer.replace("12345", "-1"), false),
        (
            "a size past 2^63",
            pointer.replace("12345", "9223372036854775808"),
            false,
        ),
        (
            "a version with a space after it",
            pointer.replace("v1\n", "v1 \n"),
            false,
        ),
        (
            "an extension after the size",
            format!("{pointer}{}", ext("0", "foo", 'f')),
            false,
        ),
    ] {
        assert_eq!(lfs_pointer(data.as_bytes()).is_some(), want, "{what}");
    }
    // What a pointer says.
    let parsed = |data: &str| lfs_pointer(data.as_bytes()).unwrap();
    let mut want = [0u8; 32];
    for (byte, pair) in want.iter_mut().zip(oid.as_bytes().chunks(2)) {
        *byte = u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap();
    }
    let plain = LfsPointer {
        oid: want,
        size: 12345,
        extensions: false,
    };
    assert_eq!(parsed(&pointer), plain);
    assert_eq!(parsed(&pointer).oid_hex(), oid);
    assert_eq!(parsed(&pointer.replace("12345", "+12345")), plain);
    assert_eq!(parsed(&pointer.replace("12345", "000")).size, 0);
    assert_eq!(parsed(&pointer.replace("12345", "-0")).size, 0);
    let with_extension = format!("{}{pointer}", ext("0", "foo", 'f'));
    assert_eq!(
        parsed(&with_extension),
        LfsPointer {
            extensions: true,
            ..plain
        }
    );
    // Bytes that aren't UTF-8 count as Git LFS counts them: as characters that match nothing.
    let named = [
        ext("0", "foo", 'f')
            .replacen("foo", "foo\u{fffd}", 1)
            .as_bytes(),
        pointer.as_bytes(),
    ]
    .concat();
    assert!(lfs_pointer(&named).is_some());
    let raw = [
        &ext("0", "foo", 'f').as_bytes()[..9],
        b"\xff",
        &ext("0", "foo", 'f').as_bytes()[9..],
        pointer.as_bytes(),
    ]
    .concat();
    assert!(lfs_pointer(&raw).is_some());
    assert!(lfs_pointer(&[b"\xff", pointer.as_bytes()].concat()).is_none());
}

const OID: &str = "4d7a214614ab2935c943f9e0ff69d22eadbb8f32b1258daaa5e2ca24d17e2393";

fn ext(digit: char, name: &str) -> String {
    format!("ext-{digit}-{name} sha256:{}\n", "f".repeat(64))
}

#[test]
fn the_lines_go_in_git_lfs_order() {
    // Each checked with `git lfs pointer --check`.
    let v = "version https://git-lfs.github.com/spec/v1\n";
    let o = format!("oid sha256:{OID}\n");
    let s = "size 12345\n";
    for (what, data, want) in [
        (
            "the pre-release version",
            format!("version https://hawser.github.com/spec/v1\n{o}{s}"),
            true,
        ),
        (
            "extensions between the lines",
            format!(
                "{}{v}{}{o}{}{s}",
                ext('0', "a"),
                ext('1', "b"),
                ext('2', "c")
            ),
            true,
        ),
        (
            "extensions out of order",
            format!("{}{}{v}{o}{s}", ext('1', "a"), ext('0', "b")),
            true,
        ),
        (
            "one extension twice",
            format!("{}{}{v}{o}{s}", ext('0', "a"), ext('0', "a")),
            true,
        ),
        (
            "one name at two digits",
            format!("{}{}{v}{o}{s}", ext('0', "a"), ext('1', "a")),
            true,
        ),
        (
            "two names at one digit",
            format!("{}{}{v}{o}{s}", ext('0', "a"), ext('0', "b")),
            false,
        ),
        (
            "names after their first letter",
            format!("{}{}{v}{o}{s}", ext('0', "_./"), ext('1', "9\u{e9}")),
            true,
        ),
        (
            "a name starting with `-`",
            format!("{}{v}{o}{s}", ext('0', "-a")),
            false,
        ),
        (
            "a name starting outside ASCII",
            format!("{}{v}{o}{s}", ext('0', "\u{e9}")),
            false,
        ),
        ("an empty name", format!("{}{v}{o}{s}", ext('0', "")), false),
        (
            "two digits",
            format!("ext-10-a sha256:{}\n{v}{o}{s}", "f".repeat(64)),
            false,
        ),
        ("the oid first", format!("{o}{v}{s}"), false),
        ("the size before the oid", format!("{v}{s}{o}"), false),
        ("the version twice", format!("{v}{v}{o}{s}"), false),
        ("the size twice", format!("{v}{o}{s}{s}"), false),
        ("another key", format!("{v}{o}{s}foo bar\n"), false),
        ("no size", format!("{v}{o}"), false),
        (
            "CR alone ends no line",
            format!("{v}{o}{s}").replace('\n', "\r"),
            false,
        ),
        (
            "two CRs",
            format!("{v}{o}{s}").replace('\n', "\r\r\n"),
            false,
        ),
        ("an empty CRLF line", format!("{v}\r\n{o}{s}"), true),
        ("a line of spaces", format!("{v}  \n{o}{s}"), false),
        ("a space starting a line", format!("{v} {o}{s}"), false),
        ("a tab after a key", format!("{v}{o}size\t12345\n"), false),
        (
            "Unicode spaces around it",
            format!("\u{2028}{v}{o}{s}\u{3000}"),
            true,
        ),
        ("a zero-width space", format!("\u{200b}{v}{o}{s}"), false),
        (
            "the largest size",
            format!("{v}{o}size 9223372036854775807\n"),
            true,
        ),
        ("a hex size", format!("{v}{o}size 0x10\n"), false),
    ] {
        assert_eq!(lfs_pointer(data.as_bytes()).is_some(), want, "{what}");
    }
    // A byte that isn't UTF-8 stops the trimming of whitespace.
    let pointer = format!("{v}{o}{s}");
    assert!(lfs_pointer(&[b"\xe2\x80 ", pointer.as_bytes()].concat()).is_none());
    assert!(lfs_pointer(&[pointer.as_bytes(), b"\xc2\xa0\xe2\x80"].concat()).is_none());
}

/// Near-pointers made from the pieces Git LFS looks at, each judged by `git lfs pointer
/// --check` too.
#[test]
fn lfs_pointer_agrees_with_git_lfs() {
    use std::io::Write;
    use std::process::{Command, Stdio};

    let dir = tempfile::tempdir().unwrap();
    let git_lfs = || {
        let mut command = Command::new("git");
        command.args(["lfs"]).current_dir(dir.path());
        command
    };
    if !git_lfs()
        .arg("version")
        .output()
        .is_ok_and(|out| out.status.success())
    {
        eprintln!("git lfs isn't installed: skipped");
        return;
    }
    let oid = format!("sha256:{OID}");
    let pieces: [&[&str]; 7] = [
        &[
            "https://git-lfs.github.com/spec/v1",
            "http://git-media.io/v/2",
            "https://git-lfs.github.com/spec/v2",
            "https://git-lfs.github.com/spec/v1 ",
        ],
        &[&oid, &oid.to_uppercase(), &oid[..70], OID],
        &[
            "12345",
            "+1",
            "-0",
            "000",
            "-1",
            "9223372036854775808",
            "1 2",
            "",
        ],
        &[
            "",
            "ext-0-foo",
            "ext-0--a",
            "ext-0-foo ext-0-bar",
            "ext-3-a ext-1-b",
        ],
        &["\n", "\r\n", "\r", "\n\n"],
        &["", " ", "\u{a0}", "\u{200b}", "\u{b}"],
        &["vos", "ovs", "vso", "vosx"],
    ];
    let make = |pick: [usize; 7]| {
        let [v, o, s, e, end, around, order] = [0, 1, 2, 3, 4, 5, 6].map(|i| pieces[i][pick[i]]);
        let mut lines: Vec<String> = e
            .split(' ')
            .filter(|key| !key.is_empty())
            .map(|key| format!("{key} sha256:{}", "f".repeat(64)))
            .collect();
        for key in order.chars() {
            lines.push(match key {
                'v' => format!("version {v}"),
                'o' => format!("oid {o}"),
                's' => format!("size {s}"),
                _ => "x y".to_owned(),
            });
        }
        format!("{around}{}{end}{around}", lines.join(end))
    };
    // Each piece once from the canonical pointer, then mixtures.
    let mut picks = vec![[0; 7]];
    for (i, list) in pieces.iter().enumerate() {
        for choice in 1..list.len() {
            let mut pick = [0; 7];
            pick[i] = choice;
            picks.push(pick);
        }
    }
    let mut seed = 0x9e37_79b9_u32;
    for _ in 0..40 {
        picks.push(std::array::from_fn(|i| {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (seed >> 16) as usize % pieces[i].len()
        }));
    }
    let datas: Vec<String> = picks.into_iter().map(make).collect();
    let verdicts: Vec<bool> = std::thread::scope(|scope| {
        let handles: Vec<_> = datas
            .chunks(datas.len().div_ceil(8))
            .map(|chunk| {
                scope.spawn(|| {
                    chunk
                        .iter()
                        .map(|data| {
                            let mut child = git_lfs()
                                .args(["pointer", "--check", "--stdin"])
                                .stdin(Stdio::piped())
                                .stdout(Stdio::null())
                                .stderr(Stdio::null())
                                .spawn()
                                .unwrap();
                            child
                                .stdin
                                .take()
                                .unwrap()
                                .write_all(data.as_bytes())
                                .unwrap();
                            let code = child.wait().unwrap().code();
                            assert!(matches!(code, Some(0 | 1)), "{data:?}: {code:?}");
                            code == Some(0)
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|h| h.join().unwrap())
            .collect()
    });
    assert!(verdicts.iter().any(|&yes| yes) && verdicts.iter().any(|&yes| !yes));
    for (data, git) in datas.iter().zip(verdicts) {
        assert_eq!(lfs_pointer(data.as_bytes()).is_some(), git, "{data:?}");
    }
}
