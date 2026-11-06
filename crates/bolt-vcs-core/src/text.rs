//! Text from outside Bolt, made safe to print.

use std::fmt::Write as _;

/// Makes text from outside Bolt safe to print. Control characters (an escape sequence could drive
/// the terminal) become `\xNN`; bidi controls, line and paragraph separators and other invisible
/// characters become `\u{NNNN}`. A newline stays when `keep_newlines`. Escaping again changes
/// nothing.
#[must_use]
pub fn escape(text: &str, keep_newlines: bool) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if c.is_control() && !(keep_newlines && c == '\n') {
            let _ = write!(out, "\\x{:02x}", u32::from(c));
        } else if hides_or_reorders(c) {
            let _ = write!(out, "\\u{{{:04x}}}", u32::from(c));
        } else {
            out.push(c);
        }
    }
    out
}

/// Non-control characters that spoof or hide: bidi controls (`invoice_\u{202e}fdp.exe` reads as
/// `invoice_exe.pdf`), separators, the Hangul fillers and every format character (category Cf,
/// Unicode 18.0) but the joiners, which emoji and many scripts need.
fn hides_or_reorders(c: char) -> bool {
    matches!(
        c,
        '\u{ad}' | '\u{600}'..='\u{605}' | '\u{61c}' | '\u{6dd}' | '\u{70f}'
            | '\u{890}'..='\u{891}' | '\u{8e2}' | '\u{180e}' | '\u{200b}'
            | '\u{200e}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2060}'..='\u{2064}'
            | '\u{2066}'..='\u{206f}' | '\u{feff}' | '\u{fff9}'..='\u{fffb}' | '\u{110bd}'
            | '\u{110cd}' | '\u{13430}'..='\u{1343f}' | '\u{1bca0}'..='\u{1bca3}'
            | '\u{1d173}'..='\u{1d17a}' | '\u{e0001}' | '\u{e0020}'..='\u{e007f}'
            // The fillers, and the line and paragraph separators.
            | '\u{115f}' | '\u{1160}' | '\u{3164}' | '\u{ffa0}' | '\u{2028}' | '\u{2029}'
    )
}

/// As [`escape`] without newlines, and the zero-width joiner and non-joiner too, which `escape`
/// keeps for emoji and scripts: every format character shows. For names that must be told apart
/// from others that look the same, such as spellings of `.git`.
#[must_use]
pub fn escape_all(text: &str) -> String {
    escape(text, false)
        .replace('\u{200c}', "\\u{200c}")
        .replace('\u{200d}', "\\u{200d}")
}

/// `n` with a comma between each group of three digits, as in 1,234,567: how Bolt writes counts
/// for people.
#[must_use]
pub fn thousands(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, digit) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

/// A size for people, in powers of 1,000 as drives are sold: 950 bytes, 2.3 GB, 150 MB.
#[must_use]
pub fn bytes(n: u64) -> String {
    const UNITS: [(u64, &str); 4] = [
        (1_000_000_000_000, "TB"),
        (1_000_000_000, "GB"),
        (1_000_000, "MB"),
        (1_000, "KB"),
    ];
    let Some(&(unit, name)) = UNITS.iter().find(|(unit, _)| n >= *unit) else {
        return if n == 1 {
            "1 byte".to_owned()
        } else {
            format!("{n} bytes")
        };
    };
    // One decimal below 10 of a unit, rounded to the nearest.
    let tenths = (u128::from(n) * 10 + u128::from(unit / 2)) / u128::from(unit);
    if tenths < 100 {
        format!("{}.{} {name}", tenths / 10, tenths % 10)
    } else {
        format!("{} {name}", thousands(n.saturating_add(unit / 2) / unit))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn counts_have_a_comma_every_three_digits() {
        let counts = [0, 999, 1000, 1234, 123_456, 1_234_567, u64::MAX].map(thousands);
        assert_eq!(
            counts,
            [
                "0",
                "999",
                "1,000",
                "1,234",
                "123,456",
                "1,234,567",
                "18,446,744,073,709,551,615"
            ]
        );
    }

    #[test]
    fn sizes_are_rounded_to_a_unit_people_read() {
        let sizes = [
            0,
            1,
            999,
            1_000,
            2_349_999_999,
            9_960_000,
            150_000_000,
            100_000_001,
            u64::MAX,
        ]
        .map(bytes);
        assert_eq!(
            sizes,
            [
                "0 bytes",
                "1 byte",
                "999 bytes",
                "1.0 KB",
                "2.3 GB",
                "10 MB",
                "150 MB",
                "100 MB",
                "18,446,744 TB"
            ]
        );
    }

    #[test]
    fn control_characters_become_visible() {
        assert_eq!(escape("a\x1b[31m\nb", false), "a\\x1b[31m\\x0ab");
        assert_eq!(escape("\0\r\x7f\u{85}", true), "\\x00\\x0d\\x7f\\x85");
    }

    #[test]
    fn bidi_and_line_separators_are_escaped() {
        assert_eq!(
            escape("a\u{202e}b\u{2028}c", false),
            "a\\u{202e}b\\u{2028}c"
        );
        // Every one the list names, and the ends of each range.
        let all = "\u{61c}\u{200e}\u{200f}\u{202a}\u{202b}\u{202c}\u{202d}\u{202e}\
                   \u{2066}\u{2067}\u{2068}\u{2069}\u{2028}\u{2029}";
        let want = "\\u{061c}\\u{200e}\\u{200f}\\u{202a}\\u{202b}\\u{202c}\\u{202d}\\u{202e}\
                    \\u{2066}\\u{2067}\\u{2068}\\u{2069}\\u{2028}\\u{2029}";
        assert_eq!(escape(all, false), want);
        assert_eq!(escape(all, true), want);
    }

    #[test]
    fn newlines_kept_only_when_asked() {
        assert_eq!(escape("one\ntwo", true), "one\ntwo");
        assert_eq!(escape("one\ntwo", false), "one\\x0atwo");
        // Only the newline: a carriage return can overwrite the start of a line.
        assert_eq!(escape("one\r\ntwo", true), "one\\x0d\ntwo");
    }

    #[test]
    fn plain_text_unchanged() {
        for text in [
            "",
            "plain text, 100% fine: C:\\Users\\ada\\src",
            "café ✓ naïve 日本語 שלום مرحبا",
            // The joiners that emoji and many scripts need are not layout controls.
            "👨\u{200d}👩\u{200d}👧 क्‍ष \u{200c}",
            "می\u{200c}خواهم",
            // Variation selectors, keycaps and skin tones are not format characters.
            "❤\u{fe0f} 1\u{fe0f}\u{20e3} 👍🏽",
            // Real Hangul, next to the blank fillers that are escaped.
            "한국어 \u{1100}\u{1161}\u{11a8} \u{3163}\u{3165} \u{ffa1}",
        ] {
            assert_eq!(escape(text, false), text);
            assert_eq!(escape(text, true), text);
        }
    }

    #[test]
    fn invisible_characters_are_escaped_with_enough_digits() {
        // Invisible characters from many blocks; the tag is above U+FFFF, so it needs five digits.
        let hidden = "a\u{200b}b\u{2060}\u{2063}\u{206a}\u{206e}\u{feff}\u{e0041}\u{180e}\u{ad}c";
        let want =
            "a\\u{200b}b\\u{2060}\\u{2063}\\u{206a}\\u{206e}\\u{feff}\\u{e0041}\\u{180e}\\u{00ad}c";
        assert_eq!(escape(hidden, false), want);
        assert_eq!(escape(hidden, true), want);
        // The Hangul fillers draw nothing.
        assert_eq!(
            escape("\u{115f}\u{1160}\u{3164}\u{ffa0}", false),
            "\\u{115f}\\u{1160}\\u{3164}\\u{ffa0}"
        );
    }

    /// The format characters (category Cf) of Unicode 18.0, from its DerivedGeneralCategory.txt.
    const FORMAT: [(u32, u32); 21] = [
        (0x00AD, 0x00AD),
        (0x0600, 0x0605),
        (0x061C, 0x061C),
        (0x06DD, 0x06DD),
        (0x070F, 0x070F),
        (0x0890, 0x0891),
        (0x08E2, 0x08E2),
        (0x180E, 0x180E),
        (0x200B, 0x200F),
        (0x202A, 0x202E),
        (0x2060, 0x2064),
        (0x2066, 0x206F),
        (0xFEFF, 0xFEFF),
        (0xFFF9, 0xFFFB),
        (0x110BD, 0x110BD),
        (0x110CD, 0x110CD),
        (0x13430, 0x1343F),
        (0x1BCA0, 0x1BCA3),
        (0x1D173, 0x1D17A),
        (0xE0001, 0xE0001),
        (0xE0020, 0xE007F),
    ];

    /// The non-control characters `escape` must replace: the list above but for the joiners, plus
    /// the Hangul fillers and the line and paragraph separators.
    fn must_hide(c: char) -> bool {
        let n = u32::from(c);
        (FORMAT.iter().any(|&(lo, hi)| (lo..=hi).contains(&n))
            && !matches!(c, '\u{200c}' | '\u{200d}'))
            || matches!(
                c,
                '\u{115f}' | '\u{1160}' | '\u{3164}' | '\u{ffa0}' | '\u{2028}' | '\u{2029}'
            )
    }

    #[test]
    fn exactly_the_controls_and_invisible_characters_are_escaped() {
        for c in '\0'..=char::MAX {
            let text = c.to_string();
            assert_eq!(
                escape(&text, false) != text,
                c.is_control() || must_hide(c),
                "U+{:04X}",
                u32::from(c)
            );
            // `escape_all` shows the joiners too, so every format character.
            let joiner = matches!(c, '\u{200c}' | '\u{200d}');
            assert_eq!(
                escape_all(&text) != text,
                c.is_control() || must_hide(c) || joiner,
                "U+{:04X}",
                u32::from(c)
            );
        }
    }

    #[test]
    fn escape_all_shows_a_joiner_hidden_in_a_name() {
        assert_eq!(escape_all(".g\u{200c}it"), ".g\\u{200c}it");
        assert_eq!(
            escape_all("a\u{200d}b\nc\u{202e}"),
            "a\\u{200d}b\\x0ac\\u{202e}"
        );
    }

    /// Any text, with the characters that matter here far likelier than chance would make them.
    fn awkward_text() -> impl Strategy<Value = String> {
        let piece = prop_oneof![
            any::<char>(),
            Just('x'),
            Just('\\'),
            Just('\n'),
            Just('\r'),
            Just('\u{1b}'),
            Just('\u{85}'),
            Just('\u{61c}'),
            Just('\u{202e}'),
            Just('\u{2066}'),
            Just('\u{2028}'),
            Just('\u{2029}'),
            Just('\u{200b}'),
            Just('\u{200d}'),
            Just('\u{feff}'),
            Just('\u{3164}'),
            Just('\u{e0041}'),
        ];
        proptest::collection::vec(piece, 0..24).prop_map(|chars| chars.into_iter().collect())
    }

    proptest! {
        #[test]
        fn nothing_dangerous_is_left_and_escaping_twice_changes_nothing(
            text in awkward_text(),
            keep in any::<bool>(),
        ) {
            let once = escape(&text, keep);
            let dangerous = |c: char| (c.is_control() && !(keep && c == '\n')) || must_hide(c);
            prop_assert!(!once.chars().any(dangerous), "{once:?}");
            prop_assert_eq!(escape(&once, keep), once);
            let all = escape_all(&text);
            let shown = |c: char| !c.is_control() && !must_hide(c) && !matches!(c, '\u{200c}' | '\u{200d}');
            prop_assert!(all.chars().all(shown), "{all:?}");
            prop_assert_eq!(escape_all(&all), all);
        }
    }
}
