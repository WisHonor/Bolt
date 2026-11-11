use super::*;
use crate::save::Person;

#[test]
fn people_parse_best_effort() {
    let person = |name: &str, email: &str, time_ms: i64, tz_offset_min: i16| Person {
        name: name.into(),
        email: email.into(),
        time_ms,
        tz_offset_min,
    };
    for (line, want) in [
        (
            &b"A U Thor <author@example.com> 1112912053 -0700"[..],
            person("A U Thor", "author@example.com", 1_112_912_053_000, -420),
        ),
        (b"C <c@d> 0 +0530", person("C", "c@d", 0, 330)),
        (b"<a@b> 1 +0000", person("", "a@b", 1000, 0)),
        (b" A  <> 1 +0000", person(" A", "", 1000, 0)),
        // Git skips whitespace around the time.
        (b"A\t<a@b>  \t 1 \t +0100", person("A", "a@b", 1000, 60)),
        // The email ends at the first `>`, and the time follows the last.
        (b"A <a<b>c> 1 +0000", person("A", "a<b", 1000, 0)),
        // Without an email, Git can't split the line: it all becomes the name.
        (
            b"A U Thor 1112912053 -0700",
            person("A U Thor 1112912053 -0700", "", 0, 0),
        ),
        (b"A <a@b 1 +0000", person("A", "", 0, 0)),
        // Without a time and a zone, Git takes neither.
        (b"A <a@b>", person("A", "a@b", 0, 0)),
        (b"A <a@b> 1", person("A", "a@b", 0, 0)),
        (b"A <a@b> 1 0100", person("A", "a@b", 0, 0)),
        (b"A <a@b> +0100", person("A", "a@b", 0, 0)),
        // A time or zone out of range is zero by itself.
        (
            b"A <a@b> 99999999999999999999 +0100",
            person("A", "a@b", 0, 60),
        ),
        (b"A <a@b> 9223372036854776 +0100", person("A", "a@b", 0, 60)),
        (
            b"A <a@b> 9223372036854775 +0100",
            person("A", "a@b", 9_223_372_036_854_775_000, 60),
        ),
        (b"A <a@b> 0001 +0100", person("A", "a@b", 1000, 60)),
        (b"A <a@b> 1 +05301", person("A", "a@b", 1000, 0)),
        (b"A <a@b> 1 +5", person("A", "a@b", 1000, 0)),
        (b"A <a@b> 1 -0000", person("A", "a@b", 1000, 0)),
        (b"A <a@b> 1 -0130x", person("A", "a@b", 1000, -90)),
        // Minutes past 59 count as Git counts them.
        (b"A <a@b> 1 +9999", person("A", "a@b", 1000, 6039)),
        (
            b"Ren\xe9 <r\xe9@x> 1 +0000",
            person("Ren\u{fffd}", "r\u{fffd}@x", 1000, 0),
        ),
        (b"", person("", "", 0, 0)),
    ] {
        assert_eq!(
            parse_person(line),
            want,
            "{}",
            String::from_utf8_lossy(line)
        );
    }
    // A name or email past 4 KiB is cut at a character boundary.
    let long = "\u{20ac}".repeat(2000);
    let p = parse_person(format!("{long} <{long}> 1 +0000").as_bytes());
    assert_eq!((p.name.len(), p.email.len()), (4095, 4095));
    assert!(p.name.chars().all(|c| c == '\u{20ac}'));
    // Bytes that aren't UTF-8 grow as U+FFFD, and are cut after.
    let p = parse_person(&[&[0xff; 3000][..], b" <a> 1 +0000"].concat());
    assert_eq!(p.name, "\u{fffd}".repeat(1365));
    // A character that starts inside the limit and ends past it goes whole, not as U+FFFD.
    let line = format!("{}\u{1f600}bb <a> 1 +0000", "a".repeat(4093));
    assert_eq!(parse_person(line.as_bytes()).name, "a".repeat(4093));
}
