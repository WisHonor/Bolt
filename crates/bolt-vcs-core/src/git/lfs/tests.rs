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
