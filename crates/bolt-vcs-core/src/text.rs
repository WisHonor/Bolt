//! Text from outside Bolt, made safe to print.

use std::fmt::Write as _;

/// Makes text from outside Bolt safe to print. Control characters (an escape sequence could drive
/// the terminal) become `\xNN`; bidi controls and line or paragraph separators become `\u{NNNN}`.
/// A newline stays when `keep_newlines`. Escaping again changes nothing.
#[must_use]
pub fn escape(text: &str, keep_newlines: bool) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if c.is_control() && !(keep_newlines && c == '\n') {
            let _ = write!(out, "\\x{:02x}", u32::from(c));
        } else if reorders_or_splits(c) {
            let _ = write!(out, "\\u{{{:04x}}}", u32::from(c));
        } else {
            out.push(c);
        }
    }
    out
}

/// Non-control characters that still make a terminal show a name other than the one it has:
/// bidi controls reorder text (`invoice_\u{202e}fdp.exe` reads as `invoice_exe.pdf`) and separators
/// end a line. Joiners are not here: emoji and many scripts need them, and they only join letters.
fn reorders_or_splits(c: char) -> bool {
    matches!(
        c,
        // The Arabic letter mark, and the left-to-right and right-to-left marks.
        '\u{61c}' | '\u{200e}' | '\u{200f}'
            // Embeddings and overrides, then isolates.
            | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}'
            // The line and paragraph separators.
            | '\u{2028}' | '\u{2029}'
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

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
        ] {
            assert_eq!(escape(text, false), text);
            assert_eq!(escape(text, true), text);
        }
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
            let dangerous = |c: char| {
                (c.is_control() && !(keep && c == '\n'))
                    || matches!(
                        c,
                        '\u{61c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}'
                            | '\u{2066}'..='\u{2069}' | '\u{2028}' | '\u{2029}'
                    )
            };
            prop_assert!(!once.chars().any(dangerous), "{once:?}");
            prop_assert_eq!(escape(&once, keep), once);
        }
    }
}
